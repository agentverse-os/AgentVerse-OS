//! Quick one-shot model call inside a workspace: `claude -p` with a cheap model and no session persistence (like the
//! commit message hint in fs.rs). Needed by the "Agents" service for housekeeping: the agent memory reflection and the
//! brief draft; runs through the workspace gate/account, knows nothing about the project and writes nothing into it.
use crate::ApiError;
use anyhow::anyhow;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::process::Stdio;

pub const DEFAULT_MODEL: &str = "claude-haiku-4-5-20251001";
const PROMPT_MAX: usize = 60_000;

#[derive(Deserialize)]
pub struct QuickBody {
    pub prompt: String,
    #[serde(default)] pub model: Option<String>,
    /// Launch directory (home by default): CLAUDE.md and the project settings the CLI picks up depend on it
    #[serde(default)] pub cwd: Option<String>,
    #[serde(default)] pub timeout_secs: Option<u64>,
}

/// POST /v1/llm/quick {prompt, model?, cwd?, timeout_secs?} → {text, model}. One turn, no tools and no session.
pub async fn quick(Json(b): Json<QuickBody>) -> Result<Json<Value>, ApiError> {
    let prompt = b.prompt.trim();
    if prompt.is_empty() { return Err(ApiError::BadRequest("пустой запрос".into())); }
    let prompt: String = if prompt.chars().count() > PROMPT_MAX { prompt.chars().take(PROMPT_MAX).collect() } else { prompt.to_string() };
    let model = b.model.as_deref().map(str::trim).filter(|m| !m.is_empty() && m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '[' | ']'))).unwrap_or(DEFAULT_MODEL).to_string();
    let cwd = crate::fs::resolve(b.cwd.as_deref().unwrap_or(""))?;
    let timeout = b.timeout_secs.unwrap_or(120).clamp(10, 300);
    let mut child = tokio::process::Command::new("bash").arg("-lc").arg(format!("exec claude -p --model {model} --output-format text --no-session-persistence --max-turns 1"))
        .current_dir(&cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| ApiError::Internal(anyhow!("claude: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() { use tokio::io::AsyncWriteExt; let _ = stdin.write_all(prompt.as_bytes()).await; let _ = stdin.shutdown().await; }
    let out = match tokio::time::timeout(std::time::Duration::from_secs(timeout), child.wait_with_output()).await {
        Ok(r) => r.map_err(|e| ApiError::Internal(anyhow!("claude: {e}")))?,
        Err(_) => return Err(ApiError::BadRequest(format!("claude не ответил за {timeout} с"))),
    };
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || text.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr); let last = err.trim().lines().last().unwrap_or("пустой ответ");
        return Err(ApiError::BadRequest(format!("claude: {}", last.chars().take(300).collect::<String>())));
    }
    Ok(Json(json!({ "text": text, "model": model })))
}
