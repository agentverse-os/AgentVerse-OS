//! Служебные вызовы дешёвой модели через workspace (ядро → agentd `llm/quick`): рефлексия памяти агента после запуска и
//! черновик имени с брифом по описанию владельца. Оба — один ход без инструментов, через gate или аккаунт workspace'а.
use crate::{App, ApiError};
use anyhow::{anyhow, Result};
use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn quick(app: &App, project: &str, prompt: &str, timeout_secs: u64) -> Result<String> {
    let r = app.core_post(&format!("/projects/{project}/llm/quick"), &json!({ "prompt": prompt, "timeout_secs": timeout_secs })).await?;
    r.get("text").and_then(|t| t.as_str()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).ok_or_else(|| anyhow!("пустой ответ модели"))
}

/// Рефлексия: переписать память агента по итогу запуска — долгосрочные факты с датами, без дублей, в бюджет. Ошибка модели
/// оставляет память как есть (обычное дописывание уже сделано в routines::watch).
pub async fn reflect(app: Arc<App>, agent: crate::routines::AgentDef, run_label: String, summary: String) {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let memory = if agent.memory.trim().is_empty() { "(пусто)".to_string() } else { agent.memory.clone() };
    let prompt = format!("Ты ведёшь память агента-сотрудника «{}». Его бриф: {}\n\nТекущая память (строки «дата: факт»):\n{memory}\n\nИтог последнего запуска «{run_label}» ({date}):\n{}\n\nПерепиши память целиком: оставь долгосрочно важные факты (решения, ограничения, грабли, договорённости, состояние проекта), добавь новые из итога, убери устаревшее и повторы, похожее объединяй. Формат: каждая строка «ГГГГ-ММ-ДД: факт», всего не больше {} символов. Без заголовков, пояснений, пустых строк и markdown — только строки памяти.",
        agent.name.trim(), agent.brief.trim(), summary.trim(), agent.memory_budget);
    let text = match quick(&app, &agent.workspace, &prompt, 120).await { Ok(t) => t, Err(e) => { tracing::warn!("рефлексия «{}»: {e:#}", agent.name); return; } };
    // модель может обернуть список в ``` или добавить заголовок — берём только строки памяти
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("```") && !l.starts_with('#')).collect();
    let dated = lines.iter().filter(|l| l.len() > 11 && l.as_bytes()[4] == b'-' && l.as_bytes()[7] == b'-' && l[..4].chars().all(|c| c.is_ascii_digit())).count();
    if lines.is_empty() || dated * 2 < lines.len() { tracing::warn!("рефлексия «{}»: ответ не похож на память ({} строк, {} с датой)", agent.name, lines.len(), dated); return; }
    let kept: Vec<&str> = lines.iter().copied().filter(|l| l.len() > 11 && l.as_bytes()[4] == b'-' && l.as_bytes()[7] == b'-' && l[..4].chars().all(|c| c.is_ascii_digit())).collect();
    let next = crate::routines::trim_memory(&kept.join("\n"), agent.memory_budget);
    if let Err(e) = crate::routines::agent_set_memory(&app, &agent.id, &next) { tracing::warn!("рефлексия «{}»: {e:#}", agent.name); return; }
    tracing::info!("рефлексия «{}»: память переписана, {} строк, {} символов", agent.name, lines.len(), next.chars().count());
}

#[derive(Deserialize)]
pub struct DraftBody { #[serde(default)] pub description: String, #[serde(default)] pub workspace: Option<String> }

/// Черновик агента по описанию: {name, brief}. Считает в указанном workspace или в первом запущенном.
pub async fn draft(State(app): State<Arc<App>>, Json(b): Json<DraftBody>) -> Result<Json<Value>, ApiError> {
    let desc = b.description.trim();
    if desc.is_empty() { return Err(anyhow!("опишите, какой сотрудник нужен").into()); }
    let ws = match b.workspace.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
        Some(w) => w.to_string(),
        None => running_workspace(&app).await?,
    };
    let prompt = format!("Владелец описал нужного агента-сотрудника: «{}».\n\nПридумай короткое имя (два-три слова, на языке описания) и бриф из четырёх-шести предложений от второго лица: чем агент владеет, какой контекст важен, какие стандарты качества и проверки, когда остановиться и спросить владельца (публикация, деньги, удаление, изменения схемы). Ответь строго одним JSON-объектом без пояснений и без markdown: {{\"name\": \"…\", \"brief\": \"…\"}}", desc.chars().take(2000).collect::<String>());
    let text = quick(&app, &ws, &prompt, 90).await?;
    let (a, z) = (text.find('{'), text.rfind('}'));
    let v: Value = match (a, z) { (Some(a), Some(z)) if z > a => serde_json::from_str(&text[a..=z]).map_err(|e| anyhow!("ответ модели не JSON: {e}"))?, _ => return Err(anyhow!("ответ модели не JSON").into()) };
    let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
    let brief = v.get("brief").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
    if name.is_empty() || brief.is_empty() { return Err(anyhow!("модель не вернула имя и бриф").into()); }
    Ok(Json(json!({ "name": name, "brief": brief, "workspace": ws })))
}

async fn running_workspace(app: &App) -> Result<String> {
    let projects = app.core_get("/projects").await?;
    projects.as_array().into_iter().flatten()
        .filter(|p| p.pointer("/workspace/status").and_then(|s| s.as_str()) == Some("running") && p.pointer("/spec/hidden").and_then(|h| h.as_bool()) != Some(true))
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_string)).next()
        .ok_or_else(|| anyhow!("нет запущенного workspace для вызова модели — укажите workspace агента"))
}
