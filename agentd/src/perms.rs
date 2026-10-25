//! Claude Code tool rules (`permissions.allow` / `deny` / `ask` in settings): the "user" scope is
//! `<CLAUDE_CONFIG_DIR|~/.claude>/settings.json`, the "project" scope is `<cwd>/.claude/settings.local.json`. Other
//! keys of the file are left alone; a live process reads the file at start (rules added via "remember" are applied by
//! the CLI itself).
use crate::ApiError;
use axum::extract::Query;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct PermQ { #[serde(default = "user")] pub scope: String, #[serde(default)] pub cwd: String }
fn user() -> String { "user".into() }

#[derive(Deserialize, Default)]
pub struct PermBody { #[serde(default)] pub allow: Vec<String>, #[serde(default)] pub deny: Vec<String>, #[serde(default)] pub ask: Vec<String> }

fn path_for(scope: &str, cwd: &str) -> Result<PathBuf, ApiError> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    match scope {
        "user" => {
            let env = crate::codex::workspace_env();
            let cfg = env.get("CLAUDE_CONFIG_DIR").cloned().unwrap_or_else(|| format!("{home}/.claude"));
            Ok(PathBuf::from(cfg).join("settings.json"))
        }
        "project" => {
            let cwd = if cwd.trim().is_empty() { home.clone() } else { cwd.trim().to_string() };
            if cwd.contains("..") { return Err(ApiError::BadRequest("путь с «..» не принимается".into())); }
            Ok(PathBuf::from(cwd).join(".claude").join("settings.local.json"))
        }
        other => Err(ApiError::BadRequest(format!("область: user | project, получено {other}"))),
    }
}

fn read(path: &PathBuf) -> Value {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).filter(|v| v.is_object()).unwrap_or(json!({}))
}

fn list(v: &Value, key: &str) -> Vec<String> {
    v.pointer(&format!("/permissions/{key}")).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn view(path: &PathBuf, scope: &str) -> Value {
    let v = read(path);
    json!({ "scope": scope, "path": path.to_string_lossy(), "exists": path.is_file(), "allow": list(&v, "allow"), "deny": list(&v, "deny"), "ask": list(&v, "ask"), "default_mode": v.pointer("/permissions/defaultMode") })
}

pub async fn get(Query(q): Query<PermQ>) -> Result<Json<Value>, ApiError> {
    let path = path_for(&q.scope, &q.cwd)?;
    Ok(Json(view(&path, &q.scope)))
}

/// Write the rule lists; an empty list removes the key. The file is created together with the
/// `.claude` directory.
pub async fn put(Query(q): Query<PermQ>, Json(b): Json<PermBody>) -> Result<Json<Value>, ApiError> {
    let path = path_for(&q.scope, &q.cwd)?;
    let mut v = read(&path);
    if !v.get("permissions").map(|p| p.is_object()).unwrap_or(false) { v["permissions"] = json!({}); }
    let clean = |l: &Vec<String>| -> Vec<String> { let mut out: Vec<String> = l.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); out.dedup(); out };
    for (key, val) in [("allow", clean(&b.allow)), ("deny", clean(&b.deny)), ("ask", clean(&b.ask))] {
        if val.is_empty() { v["permissions"].as_object_mut().unwrap().remove(key); } else { v["permissions"][key] = json!(val); }
    }
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| anyhow::anyhow!("каталог {}: {e}", parent.display()))?; }
    std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap_or_default()).map_err(|e| anyhow::anyhow!("запись {}: {e}", path.display()))?;
    Ok(Json(view(&path, &q.scope)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lists_are_read_from_permissions_block() {
        let v = json!({ "permissions": { "allow": ["Bash(npm test:*)", "Read"], "deny": ["Bash(rm:*)"] }, "other": 1 });
        assert_eq!(list(&v, "allow"), vec!["Bash(npm test:*)", "Read"]);
        assert_eq!(list(&v, "deny"), vec!["Bash(rm:*)"]);
        assert!(list(&v, "ask").is_empty());
    }
    #[test]
    fn project_scope_rejects_parent_dirs() {
        assert!(path_for("project", "/home/coder/../etc").is_err());
        assert!(path_for("nope", "").is_err());
        assert!(path_for("project", "/home/coder").unwrap().ends_with(".claude/settings.local.json"));
    }
}
