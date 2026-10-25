//! The service's own views on top of the core API: workplaces (projects + sessions + terminals + today's usage), inbox
//! (everything waiting for a human), models (routes, usage by model over a period, prices, defaults, limits), settings.
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
    let derived = { let (i, c) = app.hub.derived(); json!({ "inbox": !i.is_null(), "chat": !c.is_null() }) };
    Json(json!({ "ok": true, "version": crate::VERSION, "cloudd": core.is_ok(), "cloudd_version": core.ok().and_then(|v| v.get("version").cloned()), "hub_subscriptions": app.hub.subscribed(), "hub_derived": derived }))
}

/// Project with live sessions and terminals (from agentd through the core) and today's
/// usage.
pub struct Collected { pub projects: Vec<Value>, pub today: Value }

pub async fn collect(app: &App) -> Result<Collected> {
    // service projects (scratch) too: their sessions are needed by "Inbox" and Thread, while the "Workplaces" tree hides
    // them by `hidden`
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
    // engine catalog version — from hub snapshots (agentd puts it into the registry snapshot; the core REST
    // does not return it)
    let hub_v: HashMap<String, Value> = app.hub.engines_versions();
    let out = projects.iter().map(|p| {
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        let (reachable, sessions, ptys) = live.get(&name).cloned().unwrap_or((false, json!([]), json!([])));
        json!({
            "name": name, "description": p.pointer("/spec/description"), "status": p.pointer("/workspace/status").cloned().unwrap_or(json!("none")),
            "apps": p.pointer("/workspace/apps").cloned().unwrap_or(json!([])), "reachable": reachable, "cost_today": by_project.get(&name).copied().unwrap_or(0.0),
            "sessions": sessions, "ptys": ptys, "agents": p.get("agents").cloned(), "hidden": p.pointer("/spec/hidden").and_then(|h| h.as_bool()).unwrap_or(false),
            "engines_v": hub_v.get(&name).cloned().unwrap_or(json!(0)),
        })
    }).collect();
    Ok(Collected { projects: out, today: usage.get("total").cloned().unwrap_or(json!({ "cost_usd": 0.0, "turns": 0 })) })
}

pub async fn workspaces(State(app): State<Arc<App>>) -> R {
    let c = collect(&app).await?;
    Ok(Json(json!({ "projects": c.projects, "today": c.today })))
}

/// Inbox: pending decisions (with the request body from agentd), failed, finished with a
/// result.
pub async fn inbox(State(app): State<Arc<App>>) -> R {
    let c = collect(&app).await?;
    Ok(Json(inbox_from(&app, &c.projects, &c.today).await))
}

/// "Inbox" from the project list (a hub snapshot or a fresh `collect`): the bodies of pending
/// requests are at the core
pub async fn inbox_from(app: &App, projects: &[Value], today: &Value) -> Value {
    let mut waiting = Vec::new(); let mut failed = Vec::new(); let mut done = Vec::new();
    let mut pending_fetch = Vec::new();
    for p in projects {
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        for s in p.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default() {
            match s.get("status").and_then(|x| x.as_str()).unwrap_or("") {
                "waiting" => pending_fetch.push((name.clone(), s)),
                "error" => failed.push(json!({ "project": name, "session": s })),
                // in terminal mode (`tui`) the conversation continues in the TUI — that is not
                // "finished"
                "exited" => { if s.get("tui").and_then(|t| t.as_str()).is_none() && s.get("last_text").and_then(|t| t.as_str()).map(|t| !t.is_empty()).unwrap_or(false) { done.push(json!({ "project": name, "session": s })); } }
                // "ready" but with background tasks — still working, does not go to "Inbox"
                _ => {}
            }
        }
    }
    // the bodies of pending requests are at the core, but only when the session's pending count or last activity
    // changed (the recompute runs on every hub delta; the cache is in the hub, key "project:session")
    let app_ref: &App = app;
    let details = join_all(pending_fetch.iter().map(|(name, s)| async move {
        let id = s.get("id").and_then(|i| i.as_str()).unwrap_or("");
        let stamp = (s.get("pending").and_then(|v| v.as_u64()).unwrap_or(0), s.get("updated_at").and_then(|v| v.as_u64()).unwrap_or(0), s.get("last_seq").and_then(|v| v.as_u64()).unwrap_or(0));
        let key = format!("{name}:{id}");
        if let Some(v) = app_ref.hub.pending_cached(&key, stamp) { return Some(v) }
        let v = app_ref.core_get(&format!("/projects/{name}/sessions/{id}")).await.ok().and_then(|v| v.get("pending").cloned());
        if let Some(v) = &v { app_ref.hub.pending_store(&key, stamp, v.clone()); }
        v
    })).await;
    for ((name, s), pend) in pending_fetch.into_iter().zip(details) {
        waiting.push(json!({ "project": name, "session": s, "pending": pend.unwrap_or(json!([])) }));
    }
    let created = |v: &Value| v.pointer("/session/created_at").and_then(|c| c.as_u64()).unwrap_or(0);
    done.sort_by(|a, b| created(b).cmp(&created(a)));
    done.truncate(20);
    json!({ "waiting": waiting, "failed": failed, "done": done, "today": today })
}

#[derive(Deserialize)]
pub struct Days { #[serde(default = "d1")] pub days: u32 }
fn d1() -> u32 { 1 }

fn agent_of(model: &str) -> &'static str {
    let m = model.to_ascii_lowercase();
    if m.starts_with("claude") { "Claude" } else if m.starts_with("gpt") || m.starts_with("o1") || m.starts_with("o3") || m.contains("codex") { "Codex" } else { "—" }
}

fn default_prices() -> Value { crate::prices::default_prices() }

pub fn settings_of(app: &App) -> Result<Value> {
    let mut s = app.store.get_kv("settings")?.unwrap_or(json!({}));
    if !s.is_object() { s = json!({}); }
    if s.get("prices").map(|p| !p.is_object()).unwrap_or(true) { s["prices"] = default_prices(); }
    if s.get("defaults").map(|p| !p.is_object()).unwrap_or(true) { s["defaults"] = json!({ "claude": "", "codex": "" }); }
    if s.get("limits").map(|p| !p.is_array()).unwrap_or(true) { s["limits"] = json!([]); }
    if !s.get("mail_paused").map(|p| p.is_boolean()).unwrap_or(false) { s["mail_paused"] = json!(false); }
    // simple chat: the default route and model, search (searxng | brave | tavily | none)
    if !s.get("chat").map(|c| c.is_object()).unwrap_or(false) { s["chat"] = json!({ "route": "", "model": "", "search": { "kind": "none", "url": "", "key": "" } }); }
    Ok(s)
}
/// Whole settings (with defaults) — for modules that need several keys.
pub fn settings_value(app: &App) -> Value { settings_of(app).unwrap_or(json!({})) }
/// Boolean settings flag (e.g. `mail_paused` — agent mail is paused).
pub fn settings_flag(app: &App, key: &str) -> bool { settings_of(app).ok().and_then(|s| s.get(key).and_then(|v| v.as_bool())).unwrap_or(false) }

pub async fn settings_get(State(app): State<Arc<App>>) -> R { Ok(Json(settings_of(&app)?)) }
pub async fn settings_put(State(app): State<Arc<App>>, Json(body): Json<Value>) -> Result<StatusCode, ApiError> {
    if !body.is_object() { return Err(ApiError(StatusCode::BAD_REQUEST, "настройки — объект {prices, defaults, limits}".into())); }
    let mut cur = settings_of(&app)?;
    for (k, v) in body.as_object().unwrap() { cur[k] = v.clone(); }
    app.store.put_kv("settings", &cur)?;
    // spending limits are applied by the core (§8.3, slice 67): mirror on every save
    if body.get("limits").is_some() {
        mirror_budgets(&app).await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("лимиты сохранены, но ядро их не приняло: {e:#}")))?;
        app.hub.limits_changed(cur.get("limits").cloned().unwrap_or(json!([])));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `settings.limits` → the core's `PUT /api/agents/budgets` (on save and at service start)
pub async fn mirror_budgets(app: &App) -> anyhow::Result<()> {
    let limits = settings_of(app)?.get("limits").cloned().unwrap_or(json!([]));
    let list: Vec<Value> = limits.as_array().cloned().unwrap_or_default().into_iter().filter_map(|l| {
        let scope = l.get("scope")?.as_str()?.trim().to_string(); if scope.is_empty() { return None }
        Some(json!({ "scope": scope, "per_day": l.get("per_day").and_then(|v| v.as_f64()).unwrap_or(0.0), "action": if l.get("action").and_then(|a| a.as_str()) == Some("stop") { "stop" } else { "warn" } }))
    }).collect();
    app.core_call(reqwest::Method::PUT, "/agents/budgets", Some(&json!(list))).await.map(|_| ())
}

/// Models: routes (gate + core accounts), usage by model over a period from the expense ledger, prices, defaults,
/// limits.
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
        json!({ "id": id, "agent": agent_of(&id), "turns": b.get("turns"), "input": b.get("input"), "output": b.get("output"), "cache_read": b.get("cache_read"), "cache_create": b.get("cache_create"), "cost_usd": b.get("cost_usd"), "price": prices.get(&id).or_else(|| prices.get(&crate::prices::base_id(&id))) })
    }).collect();
    for (id, price) in prices.as_object().map(|o| o.iter().collect::<Vec<_>>()).unwrap_or_default() {
        if !models.iter().any(|m| m.get("id").and_then(|i| i.as_str()) == Some(id.as_str())) {
            models.push(json!({ "id": id, "agent": agent_of(id), "turns": 0, "input": 0, "output": 0, "cache_read": 0, "cache_create": 0, "cost_usd": 0.0, "price": price }));
        }
    }
    Ok(Json(json!({
        "period_days": days, "total": usage.get("total").cloned().unwrap_or(json!({})), "by_day": usage.get("by_day").cloned().unwrap_or(json!([])), "by_project": usage.get("by_project").cloned().unwrap_or(json!([])),
        "routes": routes, "models": models, "defaults": settings.get("defaults"), "limits": settings.get("limits"), "project_defaults": accounts.get("defaults"), "prices_sync": settings.get("prices_sync"),
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
