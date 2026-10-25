//! Scheduled messages (docs/agents-ux.md §4.3.5, modelled on CloudCLI): "send in 15 min / 1 h / tomorrow" — the service
//! holds the timer (the `scheduled` table), the scheduler tick every 15 s. When due, the message goes to the session
//! through the core: a stopped one is resumed, a busy one — through the agentd queue (goes out after the turn). Anything
//! missed while the service was down goes out at the first tick.
use crate::{App, ApiError};
use anyhow::Result;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Scheduled {
    pub id: String,
    pub project: String,
    pub session: String,
    pub content: String,
    /// unix seconds
    pub at: i64,
    /// pending | sending | sent | failed
    pub status: String,
    pub created_at: i64,
    pub sent_at: Option<i64>,
    /// how it went: "sent" / "queued (session busy)" / "session resumed, sent"
    #[serde(default)]
    pub note: Option<String>,
    pub error: Option<String>,
}

fn now() -> i64 { chrono::Utc::now().timestamp() }

#[derive(Deserialize)]
pub struct ListQ { pub project: Option<String>, pub session: Option<String>, // / `all=1|true` — including sent and failed ones (like the core's `?all=1`)
    #[serde(default)] pub all: Option<String> }

pub async fn list(State(app): State<Arc<App>>, Query(q): Query<ListQ>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "scheduled": app.store.scheduled(q.project.as_deref(), q.session.as_deref(), !matches!(q.all.as_deref(), Some("1") | Some("true")))?, "now": now() })))
}

#[derive(Deserialize)]
pub struct NewScheduled { pub project: String, pub session: String, pub content: String, pub at: i64 }

pub async fn create(State(app): State<Arc<App>>, Json(b): Json<NewScheduled>) -> Result<Json<Value>, ApiError> {
    if b.content.trim().is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "пустое сообщение".into())); }
    if b.at < now() - 60 { return Err(ApiError(StatusCode::BAD_REQUEST, "время уже прошло".into())); }
    if b.at > now() + 366 * 86400 { return Err(ApiError(StatusCode::BAD_REQUEST, "не дальше года".into())); }
    if b.content.chars().count() > 100_000 { return Err(ApiError(StatusCode::BAD_REQUEST, "сообщение длиннее 100 000 символов".into())); }
    let m = Scheduled { id: format!("sm_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]), project: b.project, session: b.session, content: b.content, at: b.at, status: "pending".into(), created_at: now(), sent_at: None, note: None, error: None };
    app.store.put_scheduled(&m)?;
    Ok(Json(serde_json::to_value(m)?))
}

pub async fn delete(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if app.store.delete_scheduled(&id)? { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError(StatusCode::NOT_FOUND, "нет такого сообщения".into())) }
}

/// One tick: take the due ones and send them. A live busy session → the agentd queue; a free one → prompt; a stopped
/// one → resume, then prompt.
pub async fn dispatch(app: &Arc<App>) {
    let due = match app.store.claim_due_scheduled(now()) { Ok(d) => d, Err(e) => { tracing::warn!("отложенные: {e:#}"); return } };
    for mut m in due {
        let res = send(app, &m).await;
        match res {
            Ok(how) => { m.status = "sent".into(); m.sent_at = Some(now()); m.note = Some(how); m.error = None; tracing::info!("отложенное {} → {}/{}: {}", m.id, m.project, &m.session[..8.min(m.session.len())], m.note.as_deref().unwrap_or("")); }
            Err(e) => { m.status = "failed".into(); m.error = Some(format!("{e:#}")); tracing::warn!("отложенное {}: {e:#}", m.id); }
        }
        let _ = app.store.put_scheduled(&m);
    }
}

async fn send(app: &Arc<App>, m: &Scheduled) -> Result<String> {
    let info = app.core_get(&format!("/projects/{}/sessions/{}", m.project, m.session)).await?;
    let alive = info.pointer("/info/alive").and_then(|v| v.as_bool()).unwrap_or(false);
    let status = info.pointer("/info/status").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let content = json!(m.content);
    if alive && status != "idle" {
        app.core_post(&format!("/projects/{}/sessions/{}/queue", m.project, m.session), &json!({ "content": content })).await?;
        return Ok("в очередь (сессия занята)".into());
    }
    if !alive {
        app.core_post(&format!("/projects/{}/sessions", m.project), &json!({ "resume": m.session })).await?;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
    app.core_post(&format!("/projects/{}/sessions/{}/prompt", m.project, m.session), &json!({ "content": content })).await?;
    Ok(if alive { "отправлено".into() } else { "сессия возобновлена, отправлено".into() })
}
