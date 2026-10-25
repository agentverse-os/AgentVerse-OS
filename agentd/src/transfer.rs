//! Moving a session between workspaces (ADR-006, "Chat" section → "promote to a project"): export is a tar with a manifest
//! (Meta), our `events.jsonl` journal, the provider journal (Claude Code transcript or Codex rollout) and the conversation
//! directory files; import is the same record (same id and provider id) under a new cwd: the provider journal is rewritten for
//! the new directory (Claude looks up the transcript by directory), files land at the same absolute path, links in the history
//! stay valid. The conversation directory travels only if it is under home and is not home itself (`/home/coder/chats/<id8>`, not
//! `/home/coder`).
use crate::session::{claude_transcript, Manager, Meta};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What is left behind: dependencies and builds are restored in place.
const EXCLUDES: &[&str] = &["node_modules", ".venv", "target", ".cache", "__pycache__", ".pnpm-store"];
/// Archive limit (bytes): anything bigger is a project, not
/// a chat.
pub const MAX_ARCHIVE: usize = 512 * 1024 * 1024;

fn home() -> String { std::env::var("HOME").unwrap_or_else(|_| "/".into()) }

/// Conversation files directory if it must travel along: under home, not home
/// itself.
pub fn files_dir(cwd: &str, home: &str) -> Option<PathBuf> {
    let c = Path::new(cwd);
    let h = Path::new(home);
    if c != h && c.starts_with(h) { Some(c.to_path_buf()) } else { None }
}

/// Provider journal path relative to its root: for Codex `sessions/YYYY/MM/DD/rollout-…jsonl` (the date in the tree
/// matters for lookup), for Claude only the file name (the directory is derived from cwd).
fn transcript_rel(agent: &str, path: &Path) -> String {
    if agent == "codex" {
        let comps: Vec<&str> = path.components().filter_map(|c| c.as_os_str().to_str()).collect();
        if let Some(i) = comps.iter().position(|c| *c == "sessions") { return comps[i..].join("/"); }
    }
    path.file_name().and_then(|n| n.to_str()).unwrap_or("transcript.jsonl").to_string()
}

async fn run(cmd: &str, args: &[String]) -> Result<()> {
    let out = tokio::process::Command::new(cmd).args(args).output().await.with_context(|| format!("запуск {cmd}"))?;
    if !out.status.success() { bail!("{cmd}: {}", String::from_utf8_lossy(&out.stderr).trim()); }
    Ok(())
}

/// Archive of a finished session. A live session is stopped by the
/// caller.
pub async fn export(m: &Manager, id: &str) -> Result<Vec<u8>> {
    let s = m.get(id).ok_or_else(|| anyhow!("нет сессии {id}"))?;
    if s.alive() { bail!("сессия ещё запущена"); }
    let meta = s.meta.lock().unwrap().clone();
    let home = home();
    let stage = std::env::temp_dir().join(format!("agentd-export-{id}"));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let mut manifest = json!({ "version": 1, "meta": meta, "home": home });
    let mut names = vec!["manifest.json".to_string(), "events.jsonl".to_string()];
    std::fs::copy(s.dir().join("events.jsonl"), stage.join("events.jsonl")).context("журнал сессии")?;
    if let Some(t) = s.transcript_path().filter(|p| p.is_file()) {
        std::fs::copy(&t, stage.join("transcript.jsonl")).context("журнал провайдера")?;
        names.push("transcript.jsonl".into());
        manifest["transcript"] = json!({ "rel": transcript_rel(&meta.agent, &t) });
        // Claude may keep an <id>/ directory with large tool results next to the transcript
        let side = t.with_extension("");
        if side.is_dir() {
            run("cp", &["-a".into(), side.to_string_lossy().into_owned(), stage.join("transcript-side").to_string_lossy().into_owned()]).await?;
            names.push("transcript-side".into());
        }
    }
    let files = files_dir(&meta.cwd, &home).filter(|d| d.is_dir());
    if let Some(d) = &files { manifest["files"] = json!(d.to_string_lossy()); }
    std::fs::write(stage.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    let out = std::env::temp_dir().join(format!("agentd-export-{id}.tar"));
    let mut args: Vec<String> = vec!["-cf".into(), out.to_string_lossy().into_owned()];
    for e in EXCLUDES { args.push(format!("--exclude={e}")); }
    args.push("-C".into()); args.push(stage.to_string_lossy().into_owned());
    args.extend(names);
    if let Some(d) = &files {
        args.push("-C".into()); args.push("/".into());
        args.push(d.strip_prefix("/").unwrap_or(d).to_string_lossy().into_owned());
    }
    let res = run("tar", &args).await;
    let _ = std::fs::remove_dir_all(&stage);
    res?;
    let data = std::fs::read(&out).context("чтение архива")?;
    let _ = std::fs::remove_file(&out);
    if data.len() > MAX_ARCHIVE { bail!("архив беседы больше {} МБ — это уже проект, а не чат", MAX_ARCHIVE / 1024 / 1024); }
    Ok(data)
}

/// Record from an archive under a new cwd (home by default). Returns the
/// session id.
pub async fn import(m: &Manager, data: &[u8], cwd: Option<String>) -> Result<String> {
    let home = home();
    let new_cwd = cwd.filter(|c| !c.trim().is_empty()).unwrap_or_else(|| home.clone());
    if !Path::new(&new_cwd).is_dir() {
        if Path::new(&new_cwd).starts_with(&home) { std::fs::create_dir_all(&new_cwd).with_context(|| format!("каталог {new_cwd}"))?; } else { bail!("каталог {new_cwd} не существует"); }
    }
    let tag = uuid::Uuid::new_v4().to_string();
    let tar = std::env::temp_dir().join(format!("agentd-import-{tag}.tar"));
    let stage = std::env::temp_dir().join(format!("agentd-import-{tag}"));
    std::fs::write(&tar, data)?;
    std::fs::create_dir_all(&stage)?;
    let res = import_inner(m, &tar, &stage, &new_cwd, &home).await;
    let _ = std::fs::remove_file(&tar);
    let _ = std::fs::remove_dir_all(&stage);
    res
}

async fn import_inner(m: &Manager, tar: &Path, stage: &Path, new_cwd: &str, home: &str) -> Result<String> {
    run("tar", &["-xf".into(), tar.to_string_lossy().into_owned(), "-C".into(), stage.to_string_lossy().into_owned()]).await?;
    let manifest: Value = serde_json::from_slice(&std::fs::read(stage.join("manifest.json")).context("в архиве нет manifest.json")?)?;
    let mut meta: Meta = serde_json::from_value(manifest.get("meta").cloned().unwrap_or(Value::Null)).context("манифест без meta")?;
    if m.get(&meta.id).is_some() { bail!("сессия {} уже есть в этом workspace", &meta.id[..8]); }
    let old_cwd = meta.cwd.clone();
    let env = crate::codex::effective_env(&meta);
    let pid = meta.provider_session_id.clone().unwrap_or_else(|| meta.id.clone());
    // provider journal — under the new
    // directory
    let src_t = stage.join("transcript.jsonl");
    meta.transcript = None;
    meta.synced_lines = Some(0);
    if src_t.is_file() {
        let text = std::fs::read_to_string(&src_t)?;
        let text = text.replace(&format!("\"{old_cwd}\""), &format!("\"{new_cwd}\""));
        let dst = if meta.agent == "codex" {
            let root = env.get("CODEX_HOME").cloned().unwrap_or_else(|| format!("{home}/.codex"));
            let rel = manifest.pointer("/transcript/rel").and_then(|r| r.as_str()).filter(|r| r.starts_with("sessions/")).map(str::to_string)
                .unwrap_or_else(|| format!("sessions/{}", src_t.file_name().and_then(|n| n.to_str()).unwrap_or("rollout.jsonl")));
            PathBuf::from(root).join(rel)
        } else {
            let cfg = env.get("CLAUDE_CONFIG_DIR").cloned().unwrap_or_else(|| format!("{home}/.claude"));
            claude_transcript(&cfg, new_cwd, &pid)
        };
        if let Some(p) = dst.parent() { std::fs::create_dir_all(p)?; }
        std::fs::write(&dst, text).with_context(|| format!("запись {}", dst.display()))?;
        let side = stage.join("transcript-side");
        if side.is_dir() {
            let side_dst = dst.with_extension("");
            let _ = std::fs::remove_dir_all(&side_dst);
            run("mv", &[side.to_string_lossy().into_owned(), side_dst.to_string_lossy().into_owned()]).await?;
        }
        meta.transcript = Some(dst.to_string_lossy().into_owned());
        meta.synced_lines = Some(crate::import::count_lines(&dst));
    }
    // conversation files — at the previous
    // path
    if let Some(files) = manifest.get("files").and_then(|f| f.as_str()) {
        let rel = files.trim_start_matches('/');
        let src = stage.join(rel);
        if src.is_dir() {
            let dst = PathBuf::from("/").join(rel);
            if dst.exists() { bail!("каталог {} уже существует здесь", dst.display()); }
            if let Some(p) = dst.parent() { std::fs::create_dir_all(p)?; }
            run("mv", &[src.to_string_lossy().into_owned(), dst.to_string_lossy().into_owned()]).await?;
        }
    }
    meta.cwd = new_cwd.to_string();
    let events = std::fs::read(stage.join("events.jsonl")).unwrap_or_default();
    let s = m.import(meta, &events)?;
    Ok(s.id())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn files_dir_only_under_home() {
        assert_eq!(files_dir("/home/coder/chats/ab12", "/home/coder"), Some(PathBuf::from("/home/coder/chats/ab12")));
        assert_eq!(files_dir("/home/coder", "/home/coder"), None);
        assert_eq!(files_dir("/tmp/x", "/home/coder"), None);
    }
    #[test]
    fn rel_keeps_codex_date_tree() {
        assert_eq!(transcript_rel("codex", Path::new("/home/coder/.codex/sessions/2026/09/28/rollout-1-abc.jsonl")), "sessions/2026/09/28/rollout-1-abc.jsonl");
        assert_eq!(transcript_rel("claude", Path::new("/home/coder/.claude/projects/-home-coder/abc.jsonl")), "abc.jsonl");
    }
}
