//! Workspace conversations from disk (ADR-006): Claude Code journals
//! (`<CLAUDE_CONFIG_DIR|~/.claude>/projects/<cwd>/<id>.jsonl`) and Codex rollouts
//! (`<CODEX_HOME|~/.codex>/sessions/YYYY/MM/DD/rollout-*-<id>.jsonl`) started outside agentd — in a terminal, code-server
//! or CloudCLI — become agentd records with the history in our journal: they are visible in the tree, can be read,
//! continued in the chat (`--resume` / `thread/resume`) and in the CLI. The same parsers pull in lines appended outside
//! our process (the native TUI from the CLI tab, a terminal). The journal format is internal to both CLIs, so parsing is
//! tolerant: unknown lines are skipped, only messages, tool calls and their results are needed.
use serde_json::{json, Value};
use std::collections::HashSet;

/// Parser version: when it is raised, previously picked-up conversations without agentd turns are rebuilt from the
/// provider journal.
pub const VERSION: u32 = 2;
use std::path::{Path, PathBuf};

/// A conversation found on disk that agentd does not
/// have yet.
#[derive(Debug, Clone)]
pub struct Found {
    pub agent: &'static str,
    pub id: String,
    pub path: PathBuf,
    pub cwd: String,
    /// seconds
    pub created_at: u64,
    pub mtime: u64,
}

/// What was pulled out of the provider
/// journal lines.
#[derive(Debug, Default)]
pub struct Parsed {
    pub events: Vec<Value>,
    /// Conversation title (Claude — `ai-title`), otherwise the first
    /// message
    pub title: Option<String>,
    pub first_text: Option<String>,
    /// Usage per the provider journal (Claude — `cost-state`, Codex — the last `token_count`, no
    /// price)
    pub usage: Option<crate::session::Usage>,
    pub model: Option<String>,
    /// How many complete lines were parsed (including
    /// skipped ones)
    pub lines: u64,
}

fn mtime_secs(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// Complete lines only: a tail without `\n` is still
/// being written.
pub fn complete_lines(text: &str) -> impl Iterator<Item = &str> {
    let end = if text.ends_with('\n') { text.len() } else { text.rfind('\n').map(|i| i + 1).unwrap_or(0) };
    text[..end].lines()
}

pub fn count_lines(path: &Path) -> u64 {
    std::fs::read(path).map(|b| b.iter().filter(|&&c| c == b'\n').count() as u64).unwrap_or(0)
}

/// `2026-09-27T18:28:57.445Z` → milliseconds (UTC only).
pub fn iso_ms(s: &str) -> Option<u64> {
    let num = |a: usize, l: usize| s.get(a..a + l)?.parse::<u64>().ok();
    let (y, mo, d, h, mi, se) = (num(0, 4)?, num(5, 2)?, num(8, 2)?, num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let ms = match s.as_bytes().get(19) {
        Some(b'.') => {
            let frac: String = s[20..].chars().take_while(|c| c.is_ascii_digit()).collect();
            let v = frac.parse::<u64>().unwrap_or(0);
            match frac.len() { 0 => 0, 1 => v * 100, 2 => v * 10, 3 => v, n => v / 10u64.pow((n - 3) as u32) }
        }
        _ => 0,
    };
    let days = days_from_civil(y as i64, mo as i64, d as i64);
    let secs = days * 86400 + (h * 3600 + mi * 60 + se) as i64;
    if secs < 0 { return None }
    Some(secs as u64 * 1000 + ms)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn ts_of(d: &Value) -> Option<u64> { d.get("timestamp").and_then(|t| t.as_str()).and_then(iso_ms) }

fn short_title(s: &str) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let t: String = line.chars().take(60).collect();
    if line.chars().count() > 60 { format!("{t}…") } else { t }
}

// ---------- Claude Code ----------

/// Claude Code journals in `<config>/projects/*/<uuid>.jsonl`, except already known ids (file name = session
/// id).
pub fn scan_claude(config_dir: &Path, known: &HashSet<String>) -> Vec<Found> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(config_dir.join("projects")) else { return out };
    for dir in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for f in files.flatten() {
            let path = f.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") || !is_uuid(stem) || known.contains(stem) { continue }
            if let Some(found) = claude_head(&path, stem) { out.push(found); }
        }
    }
    out
}

/// First lines of the journal: cwd and the time of the first message; a file without messages (service lines only)
/// is skipped.
fn claude_head(path: &Path, id: &str) -> Option<Found> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in complete_lines(&text).take(200) {
        let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
        if !matches!(d.get("type").and_then(|t| t.as_str()), Some("user") | Some("assistant")) { continue }
        if d.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) { continue }
        if d.get("sessionId").and_then(|s| s.as_str()).map(|s| s != id).unwrap_or(false) { continue }
        let cwd = d.get("cwd").and_then(|c| c.as_str()).unwrap_or("").to_string();
        if cwd.is_empty() { continue }
        return Some(Found { agent: "claude", id: id.to_string(), path: path.to_path_buf(), cwd, created_at: ts_of(&d).map(|ms| ms / 1000).unwrap_or_else(|| mtime_secs(path)), mtime: mtime_secs(path) });
    }
    None
}

/// Claude Code journal lines (from line `skip`) → events of our journal: `user`, `assistant` (blocks as is: text,
/// thinking, tool_use), `tool_result`; `ai-title` → title, `cost-state` → usage. Side chains (isSidechain) and
/// service messages (isMeta) are skipped, user images stay without bytes.
pub fn parse_claude(text: &str, skip: u64) -> Parsed {
    let mut p = Parsed::default();
    let mut cost: Option<Value> = None;
    for (i, line) in complete_lines(text).enumerate() {
        p.lines = i as u64 + 1;
        if (i as u64) < skip { continue }
        let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
        if d.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) { continue }
        let ts = ts_of(&d);
        match d.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "user" => {
                if d.get("isMeta").and_then(|b| b.as_bool()) == Some(true) { continue }
                let content = d.pointer("/message/content").cloned().unwrap_or(Value::Null);
                let blocks: Vec<Value> = match content {
                    Value::String(s) => vec![json!({ "type": "text", "text": s })],
                    Value::Array(a) => a,
                    _ => continue,
                };
                if blocks.iter().any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result")) {
                    let mut ev = json!({ "kind": "tool_result", "content": blocks, "parent_tool_use_id": null });
                    if let Some(t) = ts { ev["ts"] = json!(t); }
                    p.events.push(ev);
                    continue;
                }
                let blocks: Vec<Value> = blocks.into_iter().filter_map(|b| match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => Some(b),
                    Some("image") => Some(json!({ "type": "image", "media_type": b.pointer("/source/media_type"), "bytes": b.pointer("/source/data").and_then(|s| s.as_str()).map(|s| s.len() * 3 / 4) })),
                    _ => None,
                }).collect();
                if blocks.is_empty() { continue }
                if p.first_text.is_none() {
                    if let Some(t) = blocks.iter().find_map(|b| b.get("text").and_then(|t| t.as_str())).filter(|t| !t.trim_start().starts_with('<')) { p.first_text = Some(t.to_string()); }
                }
                // TUI slash command records: <command-name>/x</command-name><command-args>…</command-args> — the command
                // line; <local-command-stdout>…</local-command-stdout> — its output (a command card, no ANSI); caveat —
                // skipped
                if let Some(t) = blocks.iter().find_map(|b| b.get("text").and_then(|t| t.as_str())) {
                    if t.contains("<local-command-caveat>") { continue }
                    if let Some(out) = tag(t, "local-command-stdout") {
                        let mut ev = json!({ "kind": "assistant", "content": [{ "type": "text", "text": strip_ansi(&out) }], "model": "<synthetic>", "local": true, "parent_tool_use_id": null, "uuid": d.get("uuid") });
                        if let Some(t) = ts { ev["ts"] = json!(t); }
                        p.events.push(ev); continue;
                    }
                    if let Some(name) = tag(t, "command-name") {
                        let args = tag(t, "command-args").unwrap_or_default();
                        let line = if args.trim().is_empty() { name.trim().to_string() } else { format!("{} {}", name.trim(), args.trim()) };
                        let mut ev = json!({ "kind": "user", "content": [{ "type": "text", "text": line }], "uuid": d.get("uuid") });
                        if let Some(t) = ts { ev["ts"] = json!(t); }
                        p.events.push(ev); continue;
                    }
                }
                let mut ev = json!({ "kind": "user", "content": blocks, "uuid": d.get("uuid") });
                if let Some(t) = ts { ev["ts"] = json!(t); }
                p.events.push(ev);
            }
            "assistant" => {
                let content = d.pointer("/message/content").cloned().unwrap_or(json!([]));
                if !content.as_array().map(|a| !a.is_empty()).unwrap_or(false) { continue }
                // `<synthetic>` — service messages of the CLI itself (interruption, no answer), not
                // the model
                if let Some(m) = d.pointer("/message/model").and_then(|m| m.as_str()).filter(|m| !m.starts_with('<')) { p.model = Some(m.to_string()); }
                let mut ev = json!({ "kind": "assistant", "content": content, "model": d.pointer("/message/model"), "parent_tool_use_id": null, "uuid": d.get("uuid") });
                if let Some(t) = ts { ev["ts"] = json!(t); }
                p.events.push(ev);
            }
            "ai-title" => { if let Some(t) = d.get("aiTitle").and_then(|t| t.as_str()).filter(|t| !t.trim().is_empty()) { p.title = Some(t.trim().to_string()); } }
            "cost-state" => cost = Some(d),
            _ => {}
        }
    }
    if let Some(c) = cost {
        let mut u = crate::session::Usage::default();
        u.cost_usd = c.get("totalCostUSD").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let mut best: Option<(f64, String)> = None;
        if let Some(models) = c.get("modelUsage").and_then(|m| m.as_object()) {
            for (name, m) in models {
                let g = |k: &str| m.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
                u.input += g("inputTokens"); u.output += g("outputTokens"); u.cache_read += g("cacheReadInputTokens"); u.cache_create += g("cacheCreationInputTokens");
                let cost = m.get("costUSD").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if best.as_ref().map(|b| cost > b.0).unwrap_or(true) { best = Some((cost, name.clone())); }
            }
        }
        if p.model.is_none() { p.model = best.map(|b| b.1); }
        p.usage = Some(u);
    }
    p
}

/// Content of `<tag>…</tag>` in the text of a TUI journal
/// record.
fn tag(text: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>"); let close = format!("</{name}>");
    let a = text.find(&open)? + open.len(); let b = text[a..].find(&close)? + a;
    Some(text[a..b].to_string())
}

/// Strip ANSI sequences (`ESC[2m`, in the journal sometimes without ESC —
/// `[2m`).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len()); let b: Vec<char> = s.chars().collect(); let mut i = 0;
    while i < b.len() {
        if b[i] == '\u{1b}' && i + 1 < b.len() && b[i + 1] == '[' || (b[i] == '[' && i + 1 < b.len() && b[i + 1].is_ascii_digit() && b[i..].iter().take(6).any(|c| *c == 'm')) {
            let mut j = if b[i] == '\u{1b}' { i + 2 } else { i + 1 };
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == ';') { j += 1; }
            if j < b.len() && b[j] == 'm' { i = j + 1; continue }
        }
        out.push(b[i]); i += 1;
    }
    out
}

// ---------- Codex ----------

/// Codex rollouts `<home>/sessions/YYYY/MM/DD/rollout-<time>-<uuid>.jsonl`, except known ids and subagent branches.
pub fn scan_codex(codex_home: &Path, known: &HashSet<String>) -> Vec<Found> {
    let mut out = Vec::new();
    for path in rollouts(codex_home) {
        let Some(id) = rollout_id(&path) else { continue };
        if known.contains(&id) { continue }
        if let Some(f) = codex_head(&path, &id) { out.push(f); }
    }
    out
}

fn rollouts(codex_home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(years) = std::fs::read_dir(codex_home.join("sessions")) else { return out };
    for y in years.flatten() {
        let Ok(months) = std::fs::read_dir(y.path()) else { continue };
        for m in months.flatten() {
            let Ok(days) = std::fs::read_dir(m.path()) else { continue };
            for d in days.flatten() {
                let Ok(files) = std::fs::read_dir(d.path()) else { continue };
                for f in files.flatten() {
                    let p = f.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("jsonl") && p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("rollout-")).unwrap_or(false) { out.push(p); }
                }
            }
        }
    }
    out
}

fn rollout_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    is_uuid(id).then(|| id.to_string())
}

/// Thread rollout by id (for agentd sessions: the path becomes known after
/// the first turn).
pub fn find_rollout(codex_home: &Path, id: &str) -> Option<PathBuf> {
    rollouts(codex_home).into_iter().find(|p| rollout_id(p).as_deref() == Some(id))
}

fn codex_head(path: &Path, id: &str) -> Option<Found> {
    let text = std::fs::read_to_string(path).ok()?;
    let first = complete_lines(&text).next()?;
    let d = serde_json::from_str::<Value>(first).ok()?;
    if d.get("type").and_then(|t| t.as_str()) != Some("session_meta") { return None }
    let m = d.get("payload")?;
    // subagent branches (`source: {subagent: …}`) are not human
    // conversations
    if m.get("source").map(|s| s.is_object()).unwrap_or(false) { return None }
    let cwd = m.get("cwd").and_then(|c| c.as_str()).unwrap_or("").to_string();
    if cwd.is_empty() { return None }
    let created = m.get("timestamp").and_then(|t| t.as_str()).and_then(iso_ms).or_else(|| ts_of(&d)).map(|ms| ms / 1000).unwrap_or_else(|| mtime_secs(path));
    Some(Found { agent: "codex", id: m.get("id").and_then(|i| i.as_str()).unwrap_or(id).to_string(), path: path.to_path_buf(), cwd, created_at: created, mtime: mtime_secs(path) })
}

fn texts(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(|c| c.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(""),
        _ => String::new(),
    }
}

/// Codex rollout lines (from line `skip`) → events: `response_item` message (user/assistant), reasoning (summary →
/// thinking), function_call / custom_tool_call (→ tool_use), *_output (→ tool_result); `turn_context` → model,
/// `token_count` → tokens. Service user messages (in `<tags>`, instructions, developer) are skipped.
pub fn parse_codex(text: &str, skip: u64) -> Parsed {
    let mut p = Parsed::default();
    let mut tokens: Option<Value> = None;
    for (i, line) in complete_lines(text).enumerate() {
        p.lines = i as u64 + 1;
        if (i as u64) < skip { continue }
        let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
        let ts = ts_of(&d);
        let Some(pl) = d.get("payload") else { continue };
        let with_ts = |mut ev: Value| { if let Some(t) = ts { ev["ts"] = json!(t); } ev };
        match d.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "turn_context" => { if let Some(m) = pl.get("model").and_then(|m| m.as_str()) { p.model = Some(m.to_string()); } }
            "event_msg" => { if pl.get("type").and_then(|t| t.as_str()) == Some("token_count") { if let Some(t) = pl.pointer("/info/total_token_usage") { tokens = Some(t.clone()); } } }
            "response_item" => match pl.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "message" => {
                    let text = texts(pl.get("content"));
                    match pl.get("role").and_then(|r| r.as_str()) {
                        Some("user") => {
                            if text.trim().is_empty() || text.trim_start().starts_with('<') { continue }
                            if p.first_text.is_none() { p.first_text = Some(text.clone()); }
                            p.events.push(with_ts(json!({ "kind": "user", "content": [{ "type": "text", "text": text }] })));
                        }
                        Some("assistant") => { if !text.trim().is_empty() { p.events.push(with_ts(json!({ "kind": "assistant", "content": [{ "type": "text", "text": text }], "model": p.model, "parent_tool_use_id": null }))); } }
                        _ => {}
                    }
                }
                "reasoning" => {
                    let s = texts(pl.get("summary"));
                    if !s.trim().is_empty() { p.events.push(with_ts(json!({ "kind": "assistant", "content": [{ "type": "thinking", "thinking": s }], "model": p.model, "parent_tool_use_id": null }))); }
                }
                "function_call" | "custom_tool_call" | "local_shell_call" => {
                    let name = pl.get("name").and_then(|n| n.as_str()).unwrap_or("shell");
                    let raw = pl.get("arguments").or_else(|| pl.get("input")).or_else(|| pl.get("action")).cloned().unwrap_or(Value::Null);
                    let input = match &raw { Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or_else(|_| json!({ "input": s })), v => v.clone() };
                    let id = pl.get("call_id").or_else(|| pl.get("id")).cloned().unwrap_or(json!(""));
                    p.events.push(with_ts(json!({ "kind": "assistant", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }], "model": p.model, "parent_tool_use_id": null })));
                }
                "function_call_output" | "custom_tool_call_output" => {
                    let out = texts(pl.get("output"));
                    let id = pl.get("call_id").cloned().unwrap_or(json!(""));
                    p.events.push(with_ts(json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": id, "content": out.chars().take(20000).collect::<String>() }], "parent_tool_use_id": null })));
                }
                _ => {}
            },
            _ => {}
        }
    }
    if let Some(t) = tokens {
        let g = |k: &str| t.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
        p.usage = Some(crate::session::Usage { input: g("input_tokens"), output: g("output_tokens"), cache_read: g("cached_input_tokens"), cache_create: g("cache_write_input_tokens"), cost_usd: 0.0, turns: 0 });
    }
    p
}

/// Conversation title for the agentd
/// record.
pub fn title_of(p: &Parsed, agent: &str) -> String {
    p.title.clone().or_else(|| p.first_text.as_deref().map(short_title)).filter(|t| !t.is_empty()).unwrap_or_else(|| if agent == "codex" { "Codex".into() } else { "Claude Code".into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_and_lines() {
        assert_eq!(iso_ms("2026-09-27T18:28:57.445Z"), Some(1790533737445));
        assert_eq!(iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(complete_lines("a\nb\nc").collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(complete_lines("a\nb\n").collect::<Vec<_>>(), vec!["a", "b"]);
        assert!(is_uuid("bd68b2b3-65df-4f0b-9f2f-353fa75005e1") && !is_uuid("agent-bd68b2b3"));
        assert_eq!(rollout_id(Path::new("/x/rollout-2026-09-19T23-14-48-01a0bbf3-714e-78d2-b476-670f990e42f8.jsonl")).as_deref(), Some("01a0bbf3-714e-78d2-b476-670f990e42f8"));
    }

    #[test]
    fn claude_transcript_to_events() {
        let text = concat!(
            r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-09-27T18:28:57.429Z","sessionId":"s"}"#, "\n",
            r#"{"type":"user","isSidechain":false,"message":{"role":"user","content":[{"text":"Дай ключ","type":"text"}]},"uuid":"u1","timestamp":"2026-09-27T18:28:57.445Z","cwd":"/home/coder","sessionId":"s"}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-fable-5-1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]},"uuid":"a1","timestamp":"2026-09-27T18:29:00.000Z"}"#, "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]},"uuid":"u2","timestamp":"2026-09-27T18:29:01.000Z"}"#, "\n",
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>x"},"uuid":"u3"}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-fable-5-1","content":[{"type":"text","text":"Готово"}]},"uuid":"a2","timestamp":"2026-09-27T18:29:02.000Z"}"#, "\n",
            r#"{"type":"ai-title","aiTitle":"SSH-ключ для GitLab","sessionId":"s"}"#, "\n",
            r#"{"type":"cost-state","totalCostUSD":1.5,"modelUsage":{"claude-fable-5-1":{"inputTokens":10,"outputTokens":20,"cacheReadInputTokens":30,"cacheCreationInputTokens":40,"costUSD":1.5}}}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","model":"<synthetic>","content":[{"type":"text","text":"Request interrupted"}]},"uuid":"a3","timestamp":"2026-09-27T18:29:03.000Z"}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"хвост без перевода строки"}]}}"#,
        );
        let p = parse_claude(text, 0);
        assert_eq!(p.lines, 9, "хвост без \\n не считается");
        let kinds: Vec<&str> = p.events.iter().map(|e| e["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["user", "assistant", "tool_result", "assistant", "assistant"]);
        assert_eq!(p.events[0]["ts"], 1790533737445u64);
        assert_eq!(p.events[1]["content"][0]["name"], "Bash");
        assert_eq!(p.title.as_deref(), Some("SSH-ключ для GitLab"));
        assert_eq!(p.first_text.as_deref(), Some("Дай ключ"));
        assert_eq!(title_of(&p, "claude"), "SSH-ключ для GitLab");
        let u = p.usage.unwrap();
        assert_eq!((u.input, u.output, u.cache_read, u.cache_create, u.turns), (10, 20, 30, 40, 0));
        assert!((u.cost_usd - 1.5).abs() < 1e-9);
        assert_eq!(p.model.as_deref(), Some("claude-fable-5-1"));
        // TUI slash command records → the command line and an output card
        // without ANSI
        let cmd = concat!(
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/model</command-name>\n<command-message>model</command-message>\n<command-args>sonnet</command-args>"},"uuid":"c1","timestamp":"2026-09-27T18:30:00.000Z"}"#, "\n",
            r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>Set model to \u001b[1msonnet\u001b[22m [2mok[22m</local-command-stdout>"},"uuid":"c2","timestamp":"2026-09-27T18:30:01.000Z"}"#, "\n",
            r#"{"type":"user","message":{"role":"user","content":"<local-command-caveat>Caveat: x</local-command-caveat>"},"uuid":"c3"}"#, "\n",
        );
        let c = parse_claude(cmd, 0);
        assert_eq!(c.events.len(), 2);
        assert_eq!(c.events[0]["content"][0]["text"], "/model sonnet");
        assert_eq!(c.events[1]["local"], true);
        assert_eq!(c.events[1]["content"][0]["text"], "Set model to sonnet ok");
        assert!(c.first_text.is_none(), "служебные записи не дают заголовка");
        // pulling in: from line 6 — only the last two events
        let tail = parse_claude(text, 5);
        assert_eq!(tail.events.len(), 2);
        assert_eq!(tail.events[0]["content"][0]["text"], "Готово");
    }

    #[test]
    fn codex_rollout_to_events() {
        let text = concat!(
            r#"{"timestamp":"2026-09-19T23:13:44.034Z","type":"session_meta","payload":{"id":"01a0bbf2-7462-7242-b286-fd9576385034","timestamp":"2026-09-19T23:13:44.000Z","cwd":"/home/coder","originator":"codex_vscode","source":"vscode"}}"#, "\n",
            r#"{"timestamp":"2026-09-19T23:13:44.100Z","type":"turn_context","payload":{"model":"gpt-6-astra","cwd":"/home/coder"}}"#, "\n",
            r#"{"timestamp":"2026-09-19T23:13:44.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}"#, "\n",
            r#"{"timestamp":"2026-09-19T23:13:44.300Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Изучи проект"}]}}"#, "\n",
            r#"{"timestamp":"2026-09-19T23:13:45.000Z","type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"Смотрю файлы"}]}}"#, "\n",
            r#"{"timestamp":"2026-09-19T23:13:46.000Z","type":"response_item","payload":{"type":"function_call","name":"shell","call_id":"c1","arguments":"{\"command\":[\"ls\"]}"}}"#, "\n",
            r#"{"timestamp":"2026-09-19T23:13:47.000Z","type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"a\nb"}}"#, "\n",
            r#"{"timestamp":"2026-09-19T23:13:48.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":50,"output_tokens":7}}}}"#, "\n",
            r#"{"timestamp":"2026-09-19T23:13:49.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Проверил"}]}}"#, "\n",
        );
        let p = parse_codex(text, 0);
        let kinds: Vec<(&str, &str)> = p.events.iter().map(|e| (e["kind"].as_str().unwrap(), e["content"][0]["type"].as_str().unwrap())).collect();
        assert_eq!(kinds, vec![("user", "text"), ("assistant", "thinking"), ("assistant", "tool_use"), ("tool_result", "tool_result"), ("assistant", "text")]);
        assert_eq!(p.events[2]["content"][0]["input"]["command"][0], "ls");
        assert_eq!(p.events[3]["content"][0]["tool_use_id"], "c1");
        assert_eq!(title_of(&p, "codex"), "Изучи проект");
        assert_eq!(p.model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(p.usage.as_ref().map(|u| (u.input, u.cache_read, u.output)), Some((100, 50, 7)));
        let sub = r#"{"timestamp":"2026-09-19T23:14:48.810Z","type":"session_meta","payload":{"id":"01a0bbf3-714e-78d2-b476-670f990e42f8","cwd":"/home/coder","source":{"subagent":{}}}}"#;
        let dir = std::env::temp_dir().join(format!("agentd-import-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sessions/2026/09/19")).unwrap();
        let f = dir.join("sessions/2026/09/19/rollout-2026-09-19T23-14-48-01a0bbf3-714e-78d2-b476-670f990e42f8.jsonl");
        std::fs::write(&f, format!("{sub}\n")).unwrap();
        let g = dir.join("sessions/2026/09/19/rollout-2026-09-19T23-13-44-01a0bbf2-7462-7242-b286-fd9576385034.jsonl");
        std::fs::write(&g, text).unwrap();
        let found = scan_codex(&dir, &HashSet::new());
        assert_eq!(found.len(), 1, "ветка субагента пропущена");
        assert_eq!(found[0].id, "01a0bbf2-7462-7242-b286-fd9576385034");
        assert_eq!(found[0].cwd, "/home/coder");
        assert_eq!(find_rollout(&dir, "01a0bbf2-7462-7242-b286-fd9576385034").as_deref(), Some(g.as_path()));
        assert!(scan_codex(&dir, &HashSet::from(["01a0bbf2-7462-7242-b286-fd9576385034".to_string()])).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn claude_scan_skips_known_and_non_sessions() {
        let dir = std::env::temp_dir().join(format!("agentd-scan-{}", std::process::id()));
        let proj = dir.join("projects/-home-coder");
        std::fs::create_dir_all(proj.join("memory")).unwrap();
        let line = r#"{"type":"user","message":{"role":"user","content":"hi"},"uuid":"u","timestamp":"2026-09-27T18:28:57.445Z","cwd":"/home/coder","sessionId":"bd68b2b3-65df-4f0b-9f2f-353fa75005e1"}"#;
        std::fs::write(proj.join("bd68b2b3-65df-4f0b-9f2f-353fa75005e1.jsonl"), format!("{line}\n")).unwrap();
        std::fs::write(proj.join("agent-123.jsonl"), format!("{line}\n")).unwrap();
        std::fs::write(proj.join("00000000-0000-0000-0000-000000000000.jsonl"), "{\"type\":\"queue-operation\"}\n").unwrap();
        let found = scan_claude(&dir, &HashSet::new());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].id.as_str(), found[0].cwd.as_str(), found[0].created_at), ("bd68b2b3-65df-4f0b-9f2f-353fa75005e1", "/home/coder", 1790533737));
        assert!(scan_claude(&dir, &HashSet::from(["bd68b2b3-65df-4f0b-9f2f-353fa75005e1".to_string()])).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
