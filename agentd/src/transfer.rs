//! Moving a session between workspaces (ADR-006, "Chat" section → "promote to a project"): export is a tar with a manifest
//! (Meta), our `events.jsonl` journal, the provider journal (Claude Code transcript or Codex rollout) and the conversation
//! directory files; import is the same record (same id and provider id) under a new cwd: the provider journal is rewritten for
//! the new directory (Claude looks up the transcript by directory), files land at the same absolute path, links in the history
//! stay valid. The conversation directory travels only if it is under home and is not home itself (`/home/coder/chats/<id8>`, not
//! `/home/coder`).
use crate::session::{claude_transcript, Manager, Meta};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
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

// ---------------------------------------------------------------------------------------------------------------------
// What an archive may bring (ADR-010). It is built by the agentd of another workspace out of files that any code of that
// workspace can rewrite, and it lands here on the owner's click — so it carries the conversation, not the right to choose
// where files are written or what the agent process is started with.

/// A name used as a file or directory name: letters, digits, `.`, `_`, `-`; not a dot name.
fn safe_name(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.starts_with('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A relative path with ordinary components only — no `..`, no root, nothing empty.
fn safe_rel(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if rel.is_empty() || rel.contains('\0') || !p.components().all(|c| matches!(c, std::path::Component::Normal(_))) { return None }
    Some(p.to_path_buf())
}

/// The Codex rollout path under its home: `sessions/…/*.jsonl` as exported, otherwise a flat name.
fn codex_rel(rel: Option<&str>) -> PathBuf {
    rel.filter(|r| r.starts_with("sessions/") && r.ends_with(".jsonl")).and_then(safe_rel).unwrap_or_else(|| PathBuf::from("sessions/rollout.jsonl"))
}

/// The conversation's files directory may be recreated only inside this workspace's home, not as the home itself and not
/// under a dot directory of the home (`~/.ssh`, `~/.config`, `~/.claude` — places that programs read their settings from).
/// Returns the path relative to `/`, as it lies in the archive.
fn safe_files_dir(files: &str, home: &str) -> Option<PathBuf> {
    let rel_home = Path::new(files).strip_prefix(home).ok()?;
    let first = rel_home.components().next()?;
    if !rel_home.components().all(|c| matches!(c, std::path::Component::Normal(_))) || first.as_os_str().to_string_lossy().starts_with('.') { return None }
    safe_rel(files.trim_start_matches('/'))
}

/// Variables an imported session may keep: the provider keys and addresses of its account. A configuration directory is kept
/// only when it is this workspace's account directory (the core puts the sign-in files there); everything else — the
/// loader's, the shell's and the interpreters' variables, a CLI home pointing elsewhere — is dropped.
fn safe_env(env: &HashMap<String, String>, home: &str) -> HashMap<String, String> {
    const ROUTE: [&str; 10] = ["ANTHROPIC_", "OPENAI_", "GEMINI_", "GOOGLE_GEMINI_", "OPENROUTER_", "XAI_", "MOONSHOT_", "DEEPSEEK_", "MISTRAL_", "GROQ_"];
    const DIRS: [&str; 3] = ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "HOME"];
    let accounts = format!("{home}/.cloudos/accounts/");
    env.iter().filter(|(k, v)| {
        if v.contains('\n') || v.contains('\0') { return false }
        if DIRS.contains(&k.as_str()) { return v.strip_prefix(&accounts).map(|id| safe_name(id, 80)).unwrap_or(false) }
        ROUTE.iter().any(|p| k.starts_with(p)) && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    }).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// The record of an imported session with everything the archive must not decide removed. Errors — names that would become
/// paths.
fn sanitize_meta(mut meta: Meta, home: &str) -> Result<Meta> {
    if !safe_name(&meta.id, 64) || meta.id.len() < 8 { bail!("архив беседы: неверный идентификатор сессии"); }
    if let Some(p) = &meta.provider_session_id { if !safe_name(p, 128) { bail!("архив беседы: неверный идентификатор беседы у провайдера"); } }
    if !safe_name(&meta.agent, 32) { bail!("архив беседы: неверное имя движка"); }
    meta.env = safe_env(&meta.env, home);
    meta.unset.retain(|k| !k.is_empty() && k.len() <= 64 && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'));
    Ok(meta)
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
    let meta: Meta = serde_json::from_value(manifest.get("meta").cloned().unwrap_or(Value::Null)).context("манифест без meta")?;
    let mut meta = sanitize_meta(meta, home)?;
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
            PathBuf::from(root).join(codex_rel(manifest.pointer("/transcript/rel").and_then(|r| r.as_str())))
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
        let Some(rel) = safe_files_dir(files, home) else { bail!("архив беседы: каталог файлов {files} нельзя воссоздать здесь — он должен лежать в домашнем каталоге и не в скрытой папке"); };
        let src = stage.join(&rel);
        if src.is_dir() {
            let dst = PathBuf::from("/").join(&rel);
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
    fn archive_cannot_choose_where_files_go() {
        // the Codex journal: only under sessions/, only ordinary components
        assert_eq!(codex_rel(Some("sessions/2026/09/28/rollout-1-abc.jsonl")), PathBuf::from("sessions/2026/09/28/rollout-1-abc.jsonl"));
        for bad in ["sessions/../../.bashrc", "sessions/../.ssh/authorized_keys.jsonl", "/etc/passwd", "sessions//x.jsonl/../../y.jsonl", "notes/x.jsonl", "sessions/x.txt"] {
            assert_eq!(codex_rel(Some(bad)), PathBuf::from("sessions/rollout.jsonl"), "{bad}");
        }
        assert_eq!(codex_rel(None), PathBuf::from("sessions/rollout.jsonl"));
        // the files directory: inside the home, not the home, not a dot directory of it
        let h = "/home/coder";
        assert_eq!(safe_files_dir("/home/coder/chats/ab12", h), Some(PathBuf::from("home/coder/chats/ab12")));
        assert_eq!(safe_files_dir("/home/coder/proj/.worktrees/feat", h), Some(PathBuf::from("home/coder/proj/.worktrees/feat")), "скрытая папка глубже первого уровня — обычный каталог проекта");
        for bad in ["/home/coder", "/home/coder/.ssh", "/home/coder/.config/systemd/user", "/home/coder/.claude/x", "/home/coder/chats/../.ssh", "/etc/cron.d", "/home/other/x", "home/coder/x", ""] {
            assert_eq!(safe_files_dir(bad, h), None, "{bad}");
        }
    }

    #[test]
    fn archive_cannot_choose_what_the_agent_starts_with() {
        let env: HashMap<String, String> = [
            ("ANTHROPIC_API_KEY", "sk-1"), ("OPENAI_BASE_URL", "https://llm.example/v1"), ("GEMINI_API_KEY", "g"),
            ("CODEX_HOME", "/home/coder/.cloudos/accounts/acc-1"), ("CLAUDE_CONFIG_DIR", "/home/coder/chats/evil"), ("HOME", "/tmp/x"),
            ("LD_PRELOAD", "/tmp/x.so"), ("PATH", "/tmp/bin"), ("NODE_OPTIONS", "--require /tmp/x.js"), ("BASH_ENV", "/tmp/rc"), ("CLAUDE_CODE_SHELL", "/tmp/sh"),
            ("OPENAI_API_KEY", "a\nb"), ("anthropic_x", "lower"),
        ].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let mut kept: Vec<String> = safe_env(&env, "/home/coder").into_keys().collect();
        kept.sort();
        assert_eq!(kept, vec!["ANTHROPIC_API_KEY", "CODEX_HOME", "GEMINI_API_KEY", "OPENAI_BASE_URL"], "ключи и адреса аккаунта остаются, каталог настроек — только каталог аккаунта этого workspace'а");
        let meta = |id: &str, pid: Option<&str>| -> Meta { serde_json::from_value(json!({ "id": id, "agent": "claude", "cwd": "/x", "title": "t", "created_at": 0, "provider_session_id": pid, "env": env, "unset": ["ANTHROPIC_BASE_URL", "bad name"] })).unwrap() };
        let m = sanitize_meta(meta("c7dbb11a-ada7-4835-a694-d8bdb0634f82", Some("a9f05c23-1d02")), "/home/coder").unwrap();
        assert_eq!(m.env.len(), 4);
        assert_eq!(m.unset, vec!["ANTHROPIC_BASE_URL"]);
        // identifiers become directory and file names
        assert!(sanitize_meta(meta("../../.ssh", None), "/home/coder").is_err());
        assert!(sanitize_meta(meta("short", None), "/home/coder").is_err());
        assert!(sanitize_meta(meta("c7dbb11a-ada7-4835-a694-d8bdb0634f82", Some("../x")), "/home/coder").is_err());
    }

    #[test]
    fn rel_keeps_codex_date_tree() {
        assert_eq!(transcript_rel("codex", Path::new("/home/coder/.codex/sessions/2026/09/28/rollout-1-abc.jsonl")), "sessions/2026/09/28/rollout-1-abc.jsonl");
        assert_eq!(transcript_rel("claude", Path::new("/home/coder/.claude/projects/-home-coder/abc.jsonl")), "abc.jsonl");
    }
}
