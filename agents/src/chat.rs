//! The "Chat" section (ADR-006, stage 3; docs/agents-ux.md §6): quick conversations with tools without a project — in
//! the core's service workspace `scratch` (a regular project with reduced limits and `hidden: true`, created by the
//! service on first access). Every chat is an agentd session in its own directory `/home/coder/chats/<id8>`; "move to a
//! workspace" is a session move by the core (`POST /api/projects/scratch/sessions/{id}/move`).
use crate::{App, ApiError};
use anyhow::Result;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const SCRATCH: &str = "scratch";
pub const CHATS_DIR: &str = "/home/coder/chats";

fn spec() -> Value {
    json!({ "schema": 1, "project": SCRATCH, "workspace": { "runtime": "incus-nesting", "cpu": 2, "memory": "2GiB", "home": "10GiB" }, "agents": ["claude-code", "codex"], "capabilities": ["llm"],
        "description": "Служебный контейнер быстрых чатов (раздел «Чат» приложения «Агенты»): вопросы с инструментами без проекта; чат можно перенести в workspace проекта.", "hidden": true })
}

/// scratch state and its chats.
pub async fn status_of(app: &App) -> Result<Value> {
    let busy = app.scratch_busy.load(Ordering::Relaxed);
    let err = app.scratch_error.lock().unwrap().clone();
    let view = match app.core_get(&format!("/projects/{SCRATCH}")).await {
        Ok(v) => v,
        Err(e) if e.to_string().contains("404") => return Ok(json!({ "project": SCRATCH, "exists": false, "status": if busy { "creating" } else { "none" }, "reachable": false, "sessions": [], "busy": busy, "error": err })),
        Err(e) => return Err(e),
    };
    let ws_status = view.pointer("/workspace/status").and_then(|s| s.as_str()).unwrap_or("none").to_string();
    let (reachable, sessions) = if ws_status == "running" {
        match app.core_get(&format!("/projects/{SCRATCH}/sessions")).await { Ok(s) => (true, s), Err(_) => (false, json!([])) }
    } else { (false, json!([])) };
    let usage = app.core_get("/usage?days=1").await.unwrap_or(json!({}));
    let today = usage.get("by_project").and_then(|b| b.as_array()).and_then(|a| a.iter().find(|b| b.get("key").and_then(|k| k.as_str()) == Some(SCRATCH))).and_then(|b| b.get("cost_usd").cloned()).unwrap_or(json!(0.0));
    let status = if busy && ws_status != "running" { "starting".to_string() } else { ws_status };
    Ok(json!({ "project": SCRATCH, "exists": true, "status": status, "reachable": reachable, "sessions": sessions, "spec": view.get("spec"), "cost_today": today, "busy": busy, "error": err }))
}

pub async fn status(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> { Ok(Json(status_of(&app).await?)) }

/// scratch status from the hub snapshot (no core requests): the project, its state, reachability, sessions and today's
/// usage — all of it is already in the snapshot; `busy`/`error` are the service's own flags
pub fn status_from(app: &App, projects: &[Value]) -> Value {
    let busy = app.scratch_busy.load(Ordering::Relaxed);
    let err = app.scratch_error.lock().unwrap().clone();
    let Some(p) = projects.iter().find(|p| p.get("name").and_then(|n| n.as_str()) == Some(SCRATCH)) else {
        return json!({ "project": SCRATCH, "exists": false, "status": if busy { "creating" } else { "none" }, "reachable": false, "sessions": [], "busy": busy, "error": err });
    };
    let ws_status = p.get("status").and_then(|s| s.as_str()).unwrap_or("none").to_string();
    let reachable = ws_status == "running" && p.get("reachable").and_then(|r| r.as_bool()).unwrap_or(false);
    let status = if busy && ws_status != "running" { "starting".to_string() } else { ws_status };
    json!({ "project": SCRATCH, "exists": true, "status": status, "reachable": reachable, "sessions": if reachable { p.get("sessions").cloned().unwrap_or(json!([])) } else { json!([]) }, "cost_today": p.get("cost_today").cloned().unwrap_or(json!(0.0)), "busy": busy, "error": err })
}

/// Create scratch (in the background — minutes) or start a stopped one; a repeated call while busy — just the
/// state.
pub async fn ensure(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    let st = status_of(&app).await?;
    let exists = st.get("exists").and_then(|e| e.as_bool()).unwrap_or(false);
    let running = st.get("status").and_then(|s| s.as_str()) == Some("running");
    if !running && !app.scratch_busy.swap(true, Ordering::Relaxed) {
        *app.scratch_error.lock().unwrap() = String::new();
        let app2 = app.clone();
        tokio::spawn(async move {
            let res = if exists { app2.core_post(&format!("/projects/{SCRATCH}/workspace/start"), &json!({})).await } else { app2.core_post("/projects", &spec()).await };
            match res {
                Ok(_) => tracing::info!("scratch: {}", if exists { "запущен" } else { "создан" }),
                Err(e) => { tracing::warn!("scratch: {e:#}"); *app2.scratch_error.lock().unwrap() = format!("{e:#}"); }
            }
            app2.scratch_busy.store(false, Ordering::Relaxed);
        });
    }
    Ok(Json(status_of(&app).await?))
}

#[derive(Deserialize)]
pub struct NewChat { #[serde(default = "claude")] pub agent: String, pub model: Option<String>, pub permission_mode: Option<String>, pub cache_ttl: Option<String>, pub effort: Option<String> }
fn claude() -> String { "claude".into() }

/// New chat: a session in its own directory; the first message gives the title
/// (agentd).
pub async fn create(State(app): State<Arc<App>>, Json(b): Json<NewChat>) -> Result<Json<Value>, ApiError> {
    let id8 = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let mut body = json!({ "agent": b.agent, "cwd": format!("{CHATS_DIR}/{id8}") });
    for (k, v) in [("model", b.model), ("permission_mode", b.permission_mode), ("cache_ttl", b.cache_ttl), ("effort", b.effort)] { if let Some(v) = v.filter(|v| !v.is_empty()) { body[k] = json!(v); } }
    Ok(Json(app.core_post(&format!("/projects/{SCRATCH}/sessions"), &body).await?))
}

#[derive(Deserialize)]
pub struct MoveBody { pub to: String }

/// Move a chat into a project workspace ("promote to a
/// project").
pub async fn move_chat(State(app): State<Arc<App>>, Path(id): Path<String>, Json(b): Json<MoveBody>) -> Result<Json<Value>, ApiError> {
    if b.to.trim().is_empty() || b.to == SCRATCH { return Err(ApiError(axum::http::StatusCode::BAD_REQUEST, "укажите workspace проекта".into())); }
    Ok(Json(app.core_post(&format!("/projects/{SCRATCH}/sessions/{id}/move"), &json!({ "to": b.to })).await?))
}
