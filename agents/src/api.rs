//! Собственные представления сервиса поверх API ядра: рабочие места (проекты + сессии + терминалы + расход за сегодня),
//! входящие (всё, что ждёт человека), модели (маршруты, расход по моделям за период, цены, умолчания, лимиты), настройки.
use crate::{App, ApiError};
use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use futures_util::future::join_all;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

type R = Result<Json<Value>, ApiError>;

pub async fn health(State(app): State<Arc<App>>) -> Json<Value> {
    let core = app.core_get("/status").await;
    Json(json!({ "ok": true, "version": crate::VERSION, "cloudd": core.is_ok(), "cloudd_version": core.ok().and_then(|v| v.get("version").cloned()) }))
}

/// Проект с живыми сессиями и терминалами (из agentd через ядро) и расходом за сегодня.
struct Collected { projects: Vec<Value>, today: Value }

async fn collect(app: &App) -> Result<Collected> {
    // служебные проекты (scratch) — тоже: их сессии нужны «Входящим» и Thread, а дерево «Рабочих мест» их прячет по `hidden`
    let projects = app.core_get("/projects?all=1").await?;
    let projects = projects.as_array().cloned().unwrap_or_default();
    let usage = app.core_get("/usage?days=1").await.unwrap_or(json!({}));
    let by_project: HashMap<String, f64> = usage.get("by_project").and_then(|b| b.as_array()).map(|a| a.iter().filter_map(|b| Some((b.get("key")?.as_str()?.to_string(), b.get("cost_usd")?.as_f64()?))).collect()).unwrap_or_default();
    let running: Vec<String> = projects.iter().filter(|p| p.pointer("/workspace/status").and_then(|s| s.as_str()) == Some("running")).filter_map(|p| p.get("name")?.as_str().map(str::to_string)).collect();
    let fetched = join_all(running.iter().map(|name| async move {
        let s = app.core_get(&format!("/projects/{name}/sessions")).await;
        let p = app.core_get(&format!("/projects/{name}/ptys")).await;
        (name.clone(), s, p)
    })).await;
    let mut live: HashMap<String, (bool, Value, Value)> = HashMap::new();
    for (name, s, p) in fetched {
        let reachable = s.is_ok();
        live.insert(name, (reachable, s.unwrap_or(json!([])), p.unwrap_or(json!([]))));
    }
    let out = projects.iter().map(|p| {
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        let (reachable, sessions, ptys) = live.get(&name).cloned().unwrap_or((false, json!([]), json!([])));
        json!({
            "name": name, "description": p.pointer("/spec/description"), "status": p.pointer("/workspace/status").cloned().unwrap_or(json!("none")),
            "apps": p.pointer("/workspace/apps").cloned().unwrap_or(json!([])), "reachable": reachable, "cost_today": by_project.get(&name).copied().unwrap_or(0.0),
            "sessions": sessions, "ptys": ptys, "agents": p.get("agents").cloned(), "hidden": p.pointer("/spec/hidden").and_then(|h| h.as_bool()).unwrap_or(false),
        })
    }).collect();
    Ok(Collected { projects: out, today: usage.get("total").cloned().unwrap_or(json!({ "cost_usd": 0.0, "turns": 0 })) })
}

pub async fn workspaces(State(app): State<Arc<App>>) -> R {
    let c = collect(&app).await?;
    Ok(Json(json!({ "projects": c.projects, "today": c.today })))
}

/// Входящие: ждущие решения (с телом запроса из agentd), упавшие, завершившиеся с итогом.
pub async fn inbox(State(app): State<Arc<App>>) -> R {
    let c = collect(&app).await?;
    let mut waiting = Vec::new(); let mut failed = Vec::new(); let mut done = Vec::new();
    let mut pending_fetch = Vec::new();
    for p in &c.projects {
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        for s in p.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default() {
            match s.get("status").and_then(|x| x.as_str()).unwrap_or("") {
                "waiting" => pending_fetch.push((name.clone(), s)),
                "error" => failed.push(json!({ "project": name, "session": s })),
                // в терминальном режиме (`tui`) беседа продолжается в TUI — это не «завершилась»
                "exited" => { if s.get("tui").and_then(|t| t.as_str()).is_none() && s.get("last_text").and_then(|t| t.as_str()).map(|t| !t.is_empty()).unwrap_or(false) { done.push(json!({ "project": name, "session": s })); } }
                // «готов», но с фоновыми задачами — ещё работает, во «Входящие» не попадает
                _ => {}
            }
        }
    }
    let app_ref: &App = &app;
    let details = join_all(pending_fetch.iter().map(|(name, s)| async move {
        let id = s.get("id").and_then(|i| i.as_str()).unwrap_or("");
        app_ref.core_get(&format!("/projects/{name}/sessions/{id}")).await.ok().and_then(|v| v.get("pending").cloned())
    })).await;
    for ((name, s), pend) in pending_fetch.into_iter().zip(details) {
        waiting.push(json!({ "project": name, "session": s, "pending": pend.unwrap_or(json!([])) }));
    }
    let created = |v: &Value| v.pointer("/session/created_at").and_then(|c| c.as_u64()).unwrap_or(0);
    done.sort_by(|a, b| created(b).cmp(&created(a)));
    done.truncate(20);
    Ok(Json(json!({ "waiting": waiting, "failed": failed, "done": done, "today": c.today })))
}

#[derive(Deserialize)]
pub struct Days { #[serde(default = "d1")] pub days: u32 }
fn d1() -> u32 { 1 }

fn agent_of(model: &str) -> &'static str {
    let m = model.to_ascii_lowercase();
    if m.starts_with("claude") { "Claude" } else if m.starts_with("gpt") || m.starts_with("o1") || m.starts_with("o3") || m.contains("codex") { "Codex" } else { "—" }
}

/// Цены по умолчанию ($ за 1M токенов): вход, выход, чтение кэша, запись кэша. Правятся в настройках сервиса.
fn default_prices() -> Value {
    json!({
        "claude-fable-5-1": { "input": 10.0, "output": 50.0, "cache_read": 1.0, "cache_write": 12.5 },
        "claude-opus-5-5": { "input": 15.0, "output": 75.0, "cache_read": 1.50, "cache_write": 18.75 },
        "claude-sonnet-5": { "input": 3.0, "output": 15.0, "cache_read": 0.30, "cache_write": 3.75 },
        "claude-haiku-4-5-20251001": { "input": 1.0, "output": 5.0, "cache_read": 0.10, "cache_write": 1.25 },
        "gpt-6-astra": { "input": 0.0, "output": 0.0, "cache_read": 0.0, "cache_write": 0.0, "note": "подписка" },
        "gpt-oss-120b-medium": { "input": 0.15, "output": 0.60, "cache_read": 0.0, "cache_write": 0.0 }
    })
}

fn settings_of(app: &App) -> Result<Value> {
    let mut s = app.store.get_kv("settings")?.unwrap_or(json!({}));
    if !s.is_object() { s = json!({}); }
    if s.get("prices").map(|p| !p.is_object()).unwrap_or(true) { s["prices"] = default_prices(); }
    if s.get("defaults").map(|p| !p.is_object()).unwrap_or(true) { s["defaults"] = json!({ "claude": "", "codex": "" }); }
    if s.get("limits").map(|p| !p.is_array()).unwrap_or(true) { s["limits"] = json!([]); }
    if !s.get("mail_paused").map(|p| p.is_boolean()).unwrap_or(false) { s["mail_paused"] = json!(false); }
    // простой чат: маршрут и модель по умолчанию, поиск (searxng | brave | tavily | none)
    if !s.get("chat").map(|c| c.is_object()).unwrap_or(false) { s["chat"] = json!({ "route": "", "model": "", "search": { "kind": "none", "url": "", "key": "" } }); }
    Ok(s)
}
/// Настройки целиком (с умолчаниями) — для модулей, которым нужны несколько ключей.
pub fn settings_value(app: &App) -> Value { settings_of(app).unwrap_or(json!({})) }
/// Булев флаг настроек (например `mail_paused` — почта агентов на паузе).
pub fn settings_flag(app: &App, key: &str) -> bool { settings_of(app).ok().and_then(|s| s.get(key).and_then(|v| v.as_bool())).unwrap_or(false) }

pub async fn settings_get(State(app): State<Arc<App>>) -> R { Ok(Json(settings_of(&app)?)) }
pub async fn settings_put(State(app): State<Arc<App>>, Json(body): Json<Value>) -> Result<StatusCode, ApiError> {
    if !body.is_object() { return Err(ApiError(StatusCode::BAD_REQUEST, "настройки — объект {prices, defaults, limits}".into())); }
    let mut cur = settings_of(&app)?;
    for (k, v) in body.as_object().unwrap() { cur[k] = v.clone(); }
    app.store.put_kv("settings", &cur)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Модели: маршруты (gate + аккаунты ядра), расход по моделям за период из книги расходов, цены, умолчания, лимиты.
pub async fn models(State(app): State<Arc<App>>, Query(q): Query<Days>) -> R {
    let days = q.days.clamp(1, 365);
    let usage = app.core_get(&format!("/usage?days={days}")).await.unwrap_or(json!({}));
    let accounts = app.core_get("/accounts").await.unwrap_or(json!({ "accounts": [], "defaults": {} }));
    let settings = settings_of(&app)?;
    let prices = settings.get("prices").cloned().unwrap_or(json!({}));
    let mut routes = vec![json!({ "id": "gate", "kind": "gate", "name": "gate проекта", "sub": "грант llm · прокси провайдера", "health": "ok" })];
    for a in accounts.get("accounts").and_then(|a| a.as_array()).cloned().unwrap_or_default() {
        let kind = match a.get("kind").and_then(|k| k.as_str()).unwrap_or("") { "subscription" => "подписка", "proxy" => "прокси", "api_key" => "ключ", other => other };
        routes.push(json!({ "id": a.get("id"), "kind": kind, "name": a.get("label"), "sub": a.get("imported_from").and_then(|i| i.as_str()).map(|s| format!("из workspace {s}")).unwrap_or_else(|| a.get("provider").and_then(|p| p.as_str()).unwrap_or("").to_string()), "provider": a.get("provider"), "health": "unknown" }));
    }
    let mut models: Vec<Value> = usage.get("by_model").and_then(|m| m.as_array()).cloned().unwrap_or_default().into_iter().map(|b| {
        let id = b.get("key").and_then(|k| k.as_str()).unwrap_or("").to_string();
        json!({ "id": id, "agent": agent_of(&id), "turns": b.get("turns"), "input": b.get("input"), "output": b.get("output"), "cache_read": b.get("cache_read"), "cache_create": b.get("cache_create"), "cost_usd": b.get("cost_usd"), "price": prices.get(&id) })
    }).collect();
    for (id, price) in prices.as_object().map(|o| o.iter().collect::<Vec<_>>()).unwrap_or_default() {
        if !models.iter().any(|m| m.get("id").and_then(|i| i.as_str()) == Some(id.as_str())) {
            models.push(json!({ "id": id, "agent": agent_of(id), "turns": 0, "input": 0, "output": 0, "cache_read": 0, "cache_create": 0, "cost_usd": 0.0, "price": price }));
        }
    }
    Ok(Json(json!({
        "period_days": days, "total": usage.get("total").cloned().unwrap_or(json!({})), "by_day": usage.get("by_day").cloned().unwrap_or(json!([])), "by_project": usage.get("by_project").cloned().unwrap_or(json!([])),
        "routes": routes, "models": models, "defaults": settings.get("defaults"), "limits": settings.get("limits"), "project_defaults": accounts.get("defaults"),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_by_model_name() {
        assert_eq!(agent_of("claude-fable-5-1"), "Claude");
        assert_eq!(agent_of("gpt-6-astra"), "Codex");
        assert_eq!(agent_of("gpt-oss-120b-medium"), "Codex");
        assert_eq!(agent_of("kimi-k3"), "—");
    }
}
