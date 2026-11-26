//! Antigravity CLI (`agy`) — our own adapter on top of the headless stream-json mode (docs/agents-ux.md §4.3.17, path 2).
//! The `agy --input-format stream-json --output-format stream-json --print=` process lives between turns: every turn is one
//! `{"event":"user","message":{"content":"…"}}` line, the answer is `init` (conversation id, model, tools), `step_update`
//! (text as it streams, tools with parameters and output, usage) and `result` (turn outcome, `denied_actions`). The protocol
//! has no interrupt — only stopping the process; resume is `--conversation <id>` (the conversation is native, agy remembers
//! it itself). Changing the model, effort or mode on a live session restarts the process at the next message.
use crate::session::{now_ms, Session, Status};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// agy effort levels (`--effort`); they work the same with gate models
/// (gemini-api://…).
pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
/// Context window when the gate catalog does not know it (agy itself assumes 1M
/// for Gemini).
pub const DEFAULT_WINDOW: u64 = 1_000_000;

#[derive(Default)]
pub struct AgyState {
    pub conversation_id: Option<String>,
    pub in_turn: bool,
    /// Text of the current answer (flushed to the journal on a tool call and at the end of
    /// the answer)
    pub text: String,
    /// Whole text of the turn — for the result
    pub turn_text: String,
    pub turn_started_ms: u64,
    /// Active tools of the turn: step_index → (card id, name)
    pub tools: HashMap<u64, (String, String)>,
    /// Tools of the turn that finished without output (card ids): if the result lists `denied_actions`, these are the
    /// declined ones
    pub silent: Vec<String>,
    /// Prompt tokens of the last step — context usage
    pub last_input_tokens: u64,
    pub out_tokens: u64,
    /// Arguments the process was spawned with: if they differ from the current ones (model/effort/mode) — restart at the
    /// next message
    pub spawned_args: Vec<String>,
    pub init_tools: Vec<String>,
    pub model: Option<String>,
    /// Native mode (Google account, §9.11): no gate; models come from `agy models` under the
    /// process HOME
    pub native: bool,
    pub native_models: Vec<(String, String)>,
    /// Text of the current prompt and whether it was already re-sent once after a "no capacity" answer (§4.3.17)
    pub last_text: String,
    pub retried: bool,
    /// Retry generation: a pending retry is dropped when the turn was closed (stop, interrupt) meanwhile
    pub retry_gen: u64,
}

/// `agy models` under the home directory `home` (Google account): (id, name) pairs; empty without a
/// sign-in.
pub async fn native_models(home: Option<&str>) -> Vec<(String, String)> {
    let mut cmd = tokio::process::Command::new("bash");
    let script = match home { Some(h) => format!("HOME={} exec timeout 60 agy models", shell_quote(h)), None => "exec timeout 60 agy models".to_string() };
    cmd.args(["-lc", &script]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let out = match tokio::time::timeout(std::time::Duration::from_secs(70), cmd.output()).await { Ok(Ok(o)) => String::from_utf8_lossy(&o.stdout).to_string(), _ => String::new() };
    parse_models(&out)
}
pub fn parse_models(out: &str) -> Vec<(String, String)> {
    out.lines().filter_map(|l| { let mut it = l.split('\t'); let id = it.next()?.trim(); let name = it.next()?.trim(); if id.is_empty() || name.is_empty() || id.contains(' ') { None } else { Some((id.to_string(), name.to_string())) } }).collect()
}
fn shell_quote(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

/// Permission mode flags (our modes → `--mode` / `--dangerously-skip-permissions`): "ask" (default) has no flags:
/// headless agy edits inside the workspace on its own and declines commands without a rule (it cannot ask).
pub fn mode_flags(mode: Option<&str>) -> Vec<String> {
    let norm = mode.unwrap_or("default").chars().filter(|c| *c != '_' && *c != '-').collect::<String>().to_ascii_lowercase();
    match norm.as_str() {
        "auto" | "yolo" | "bypasspermissions" | "dontask" | "never" => vec!["--dangerously-skip-permissions".into()],
        "acceptedits" | "autoedit" => vec!["--mode".into(), "accept-edits".into()],
        "plan" => vec!["--mode".into(), "plan".into()],
        _ => Vec::new(),
    }
}

/// Modes for the composer (caps.mode_options): ids are ours, descriptions say what agy
/// does with them.
pub fn mode_options() -> Value {
    json!([
        { "id": "default", "name": "Спрашивать", "description": "правки файлов в workspace — без вопросов; команды headless agy отклоняет (спросить не умеет) — разрешите правилом permissions.allow или выберите «авто»" },
        { "id": "acceptEdits", "name": "Принимать правки", "description": "режим accept-edits: правки без вопросов, команды — только по правилам" },
        { "id": "plan", "name": "План", "description": "только чтение и план без правок и команд" },
        { "id": "auto", "name": "Авто", "description": "все инструменты без вопросов (--dangerously-skip-permissions)" },
    ])
}

/// agy process arguments for a session: model (through the gate — gemini-api://<id>), effort, mode, conversation
/// resume.
pub fn args(model: Option<&str>, effort: Option<&str>, mode: Option<&str>, gate: bool, conversation: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = vec!["agy".into(), "--input-format".into(), "stream-json".into(), "--output-format".into(), "stream-json".into(), "--print=".into(), "--print-timeout".into(), "0".into()];
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) { a.push("--model".into()); a.push(crate::acp::antigravity_model(m, gate)); }
    if let Some(e) = effort.map(str::trim).filter(|e| EFFORTS.contains(e)) { a.push("--effort".into()); a.push(e.to_string()); }
    a.extend(mode_flags(mode));
    if let Some(c) = conversation.filter(|c| !c.is_empty()) { a.push("--conversation".into()); a.push(c.to_string()); }
    a
}

/// agy tool name → card name (as in Claude Code: Bash/Read/Write/Edit/…) and input parameters in the familiar fields.
pub fn tool_card(name: &str, params: &Value) -> (String, Value) {
    let p = |k: &str| params.get(k).cloned().unwrap_or(Value::Null);
    let mut input = if params.is_object() { params.clone() } else { json!({}) };
    let card = match name {
        "run_command" => { if let Some(c) = p("CommandLine").as_str() { input["command"] = json!(c); } if let Some(c) = p("Cwd").as_str() { input["cwd"] = json!(c); } "Bash" }
        "view_file" | "view_file_outline" | "view_code_item" | "view_content_chunk" => { if let Some(f) = p("AbsolutePath").as_str() { input["file_path"] = json!(f); } "Read" }
        "write_to_file" => { if let Some(f) = p("TargetFile").as_str() { input["file_path"] = json!(f); } if let Some(c) = p("CodeContent").as_str() { input["content"] = json!(c); } "Write" }
        "replace_file_content" | "multi_replace_file_content" => { if let Some(f) = p("TargetFile").as_str() { input["file_path"] = json!(f); } if let Some(o) = p("TargetContent").as_str() { input["old_string"] = json!(o); } if let Some(n) = p("ReplacementContent").as_str() { input["new_string"] = json!(n); } "Edit" }
        "list_dir" => { if let Some(d) = p("DirectoryPath").as_str() { input["path"] = json!(d); } "LS" }
        "grep_search" => { if let Some(q) = p("Query").as_str() { input["pattern"] = json!(q); } "Grep" }
        "find_by_name" => { if let Some(q) = p("Pattern").as_str() { input["pattern"] = json!(q); } "Glob" }
        "search_web" => "WebSearch",
        "read_url_content" => { if let Some(u) = p("Url").as_str() { input["url"] = json!(u); } "WebFetch" }
        "ask_question" | "ask_permission" | "ask_custom_permission" => "AskUserQuestion",
        n if n.starts_with("browser_") => "Browser",
        n if n.starts_with("subagent") || n == "run_subagent" || n == "spawn_subagent" => "Agent",
        _ => name,
    };
    (card.to_string(), input)
}

/// Prompt text from content blocks (headless agy accepts text only; images and files go as a path/note).
pub fn prompt_text(content: &Value) -> String {
    let mut out = String::new();
    for b in content.as_array().cloned().unwrap_or_default() {
        match b.get("type").and_then(|t| t.as_str()) {
            Some("text") => { if let Some(t) = b.get("text").and_then(|t| t.as_str()) { if !out.is_empty() { out.push('\n'); } out.push_str(t); } }
            Some("image") => { if !out.is_empty() { out.push('\n'); } out.push_str("[изображение приложено, но headless agy принимает только текст]"); }
            Some("file") | Some("document") => { if let Some(p) = b.get("path").or(b.get("name")).and_then(|t| t.as_str()) { if !out.is_empty() { out.push('\n'); } out.push_str(&format!("@{p}")); } }
            _ => {}
        }
    }
    out
}

/// Note about declined tools from `result.denied_actions`.
pub fn denied_note(denied: &Value) -> Option<String> {
    let list = denied.as_array()?;
    if list.is_empty() { return None }
    let what: Vec<String> = list.iter().map(|d| {
        let action = d.get("action").and_then(|a| a.as_str()).unwrap_or("");
        let name = d.get("display_name").and_then(|a| a.as_str()).unwrap_or("");
        if name.is_empty() { action.to_string() } else if action.is_empty() { name.to_string() } else { format!("{name} ({action})") }
    }).collect();
    Some(format!("Antigravity отклонил: {} — в headless-режиме agy не умеет спрашивать разрешение. Переключите режим беседы на «авто» (все инструменты) или добавьте правило permissions.allow в ~/.gemini/antigravity-cli/settings.json и повторите сообщение.", what.join(", ")))
}

/// Claude Code tool rule → agy permission rule (docs/agents-ux.md §4.3.17): `Bash(npm test:*)` → `command(npm test)`,
/// `Bash(git status)` → `command(regex:^git status$)`, `Bash` → `command(*)`, `Read`/`Read(p)` → `read_file(*|p)`,
/// `Edit`/`Write`/`MultiEdit`/`NotebookEdit` → `write_file(…)`, `WebFetch(domain:x)` → `read_url(x)`, `mcp__s__t` → `mcp(s/t)`,
/// `mcp__s` → `mcp(s/*)`; anything else (WebSearch, Agent, Glob…) has no agy counterpart.
pub fn claude_rule_to_agy(rule: &str) -> Option<String> {
    let rule = rule.trim();
    let (tool, arg) = match rule.find('(') { Some(i) if rule.ends_with(')') => (&rule[..i], Some(rule[i + 1..rule.len() - 1].trim())), _ => (rule, None) };
    let path_rule = |kind: &str| Some(match arg { Some(a) if !a.is_empty() => format!("{kind}({a})"), _ => format!("{kind}(*)") });
    match tool {
        "Bash" => Some(match arg {
            None | Some("") | Some("*") => "command(*)".to_string(),
            Some(a) if a.ends_with(":*") => format!("command({})", a[..a.len() - 2].trim()),
            Some(a) if a.ends_with('*') => format!("command({})", a[..a.len() - 1].trim()),
            Some(a) => format!("command(regex:^{}$)", regex_escape(a)),
        }),
        "Read" => path_rule("read_file"),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => path_rule("write_file"),
        "WebFetch" => Some(match arg { Some(a) if !a.is_empty() && a != "*" => format!("read_url({})", a.trim_start_matches("domain:").trim()), _ => "read_url(*)".to_string() }),
        t if t.starts_with("mcp__") => {
            let mut parts = t.trim_start_matches("mcp__").splitn(2, "__");
            let server = parts.next().unwrap_or("").trim();
            if server.is_empty() { return None }
            Some(match parts.next().filter(|x| !x.trim().is_empty()) { Some(tool) => format!("mcp({server}/{tool})"), None => format!("mcp({server}/*)") })
        }
        _ => None,
    }
}
fn regex_escape(s: &str) -> String { let mut o = String::new(); for c in s.chars() { if "\\.+*?()[]{}|^$".contains(c) { o.push('\\'); } o.push(c); } o }

/// Our rules into agy's `permissions.allow`/`deny` of `<home>/.gemini/antigravity-cli/settings.json`: the rules written last
/// time (remembered in `<home>/.cloudos/agy-rules.json`) are removed first, the owner's own rules stay. Returns whether the
/// file changed.
pub fn sync_rules(home: &str, allow: &[String], deny: &[String]) -> std::io::Result<bool> {
    let dir = format!("{home}/.gemini/antigravity-cli");
    std::fs::create_dir_all(&dir)?;
    let path = format!("{dir}/settings.json");
    let marker = format!("{home}/.cloudos/agy-rules.json");
    let mut st: Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    if !st.is_object() { st = json!({}); }
    let prev: Value = std::fs::read(&marker).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    let list = |v: &Value, k: &str| -> Vec<String> { v.get(k).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
    let mut changed = false;
    for (key, ours) in [("allow", allow), ("deny", deny)] {
        let before = list(st.get("permissions").unwrap_or(&Value::Null), key);
        let old_ours = list(&prev, key);
        let mut cur: Vec<String> = before.iter().filter(|r| !old_ours.contains(r)).cloned().collect();
        for r in ours { if !cur.contains(r) { cur.push(r.clone()); } }
        if cur != before {
            changed = true;
            if !st.get("permissions").map(|p| p.is_object()).unwrap_or(false) { st["permissions"] = json!({}); }
            if cur.is_empty() { st["permissions"].as_object_mut().unwrap().remove(key); } else { st["permissions"][key] = json!(cur); }
        }
    }
    if st.get("permissions").map(|p| p.as_object().map(|o| o.is_empty()).unwrap_or(false)).unwrap_or(false) { st.as_object_mut().unwrap().remove("permissions"); }
    if changed { std::fs::write(&path, serde_json::to_vec_pretty(&st).unwrap_or_default())?; }
    let _ = std::fs::create_dir_all(format!("{home}/.cloudos"));
    std::fs::write(&marker, serde_json::to_vec_pretty(&json!({ "allow": allow, "deny": deny })).unwrap_or_default())?;
    Ok(changed)
}

/// Google answered that there is no capacity for the model right now (503 / UNAVAILABLE / overloaded) — a temporary
/// condition worth one retry (§4.3.17).
pub fn is_capacity_error(e: &str) -> bool {
    let l = e.to_ascii_lowercase();
    l.contains("no capacity") || l.contains("503") || l.contains("unavailable") || l.contains("overloaded") || l.contains("try again later")
}

/// Claude Code rule for a declined agy tool call (card name and input): the "allow and retry" button. Commands get a
/// one- or two-word prefix (`git`, `npm`, `cargo`… take the subcommand too), files their path, MCP tools `mcp__s__t`.
pub fn rule_for_tool(card: &str, input: &Value) -> Option<String> {
    let s = |k: &str| input.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    match card {
        "Bash" => {
            let cmd = s("command")?;
            let words: Vec<&str> = cmd.split_whitespace().collect();
            let first = *words.first()?;
            let two = matches!(first, "git" | "npm" | "npx" | "pnpm" | "yarn" | "cargo" | "docker" | "kubectl" | "go" | "make" | "python" | "python3" | "pip" | "pip3" | "uv" | "gh" | "incus" | "systemctl");
            let prefix = if two && words.len() > 1 && !words[1].starts_with('-') { format!("{first} {}", words[1]) } else { first.to_string() };
            Some(format!("Bash({prefix}:*)"))
        }
        "Read" => Some(match s("file_path") { Some(p) => format!("Read({p})"), None => "Read".to_string() }),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => Some(match s("file_path") { Some(p) => format!("Edit({p})"), None => "Edit".to_string() }),
        "WebFetch" => Some(match s("url").and_then(|u| u.split("//").nth(1).map(|h| h.split('/').next().unwrap_or("").to_string())).filter(|h| !h.is_empty()) { Some(h) => format!("WebFetch(domain:{h})"), None => "WebFetch".to_string() }),
        t if t.starts_with("mcp__") => Some(t.to_string()),
        _ => None,
    }
}

impl Session {
    /// Does the process need a restart with new arguments (model/effort/mode changed on a live
    /// session).
    pub(crate) fn agy_restart_needed(&self, gate: bool) -> bool {
        let meta = self.meta.lock().unwrap().clone();
        let st = self.agy.lock().unwrap();
        if st.spawned_args.is_empty() { return false }
        let conv = st.conversation_id.clone().or(meta.provider_session_id.clone());
        let want = args(meta.model.as_deref(), meta.effort.as_deref(), meta.permission_mode.as_deref(), gate, conv.as_deref());
        strip_conversation(&want) != strip_conversation(&st.spawned_args)
    }

    pub(crate) async fn agy_prompt(self: &Arc<Self>, content: &Value) -> Result<()> {
        let text = prompt_text(content);
        if text.trim().is_empty() { bail!("пустое сообщение"); }
        { let mut st = self.agy.lock().unwrap(); if st.in_turn { bail!("ход ещё идёт — дождитесь ответа или остановите"); } st.in_turn = true; st.turn_started_ms = now_ms(); st.turn_text.clear(); st.text.clear(); st.tools.clear(); st.silent.clear(); st.last_text = text.clone(); st.retried = false; }
        self.write(&json!({ "event": "user", "message": { "content": text } })).await
    }

    /// agy stdout line → journal.
    pub(crate) async fn agy_on_message(self: &Arc<Self>, v: Value) {
        match v.get("event").and_then(|e| e.as_str()).unwrap_or("") {
            "init" => {
                let conv = v.get("conversation_id").and_then(|c| c.as_str()).map(str::to_string);
                let init = v.get("init").cloned().unwrap_or(Value::Null);
                let tools: Vec<String> = init.get("tools").and_then(|t| t.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let model = init.get("model").and_then(|m| m.as_str()).map(|m| m.trim_start_matches("gemini-api://").to_string());
                { let mut st = self.agy.lock().unwrap(); st.conversation_id = conv.clone(); st.init_tools = tools.clone(); st.model = model.clone(); }
                let resumed = { let mut m = self.meta.lock().unwrap(); let was = m.provider_session_id.clone(); m.provider_session_id = conv.clone(); was.is_some() && was == conv };
                self.save_meta();
                let (meta_model, mode) = { let m = self.meta.lock().unwrap(); (m.model.clone(), m.permission_mode.clone()) };
                self.emit(json!({ "kind": "init", "protocol": "agy", "provider": "antigravity", "model": model.or(meta_model), "cwd": init.get("cwd"), "tools": tools, "permission_mode": mode.unwrap_or_else(|| "default".into()),
                    "agy_permission_mode": init.get("permission_mode"), "session_id": conv, "loaded": resumed, "modes": mode_options() }));
                if self.status() == Status::Starting { self.set_status(Status::Idle); self.drain_queue(); }
            }
            "step_update" => {
                let s = v.get("step_update").cloned().unwrap_or(Value::Null);
                let idx = s.get("step_index").and_then(|i| i.as_u64()).unwrap_or(0);
                let state = s.get("state").and_then(|x| x.as_str()).unwrap_or("");
                let step = s.get("step_type").and_then(|x| x.as_str()).unwrap_or("");
                if let Some(u) = s.get("usage") { let mut st = self.agy.lock().unwrap(); if let Some(i) = u.get("input_tokens").and_then(|x| x.as_u64()) { st.last_input_tokens = i; } if let Some(o) = u.get("output_tokens").and_then(|x| x.as_u64()) { st.out_tokens += o; } }
                match step {
                    "agent_response" => {
                        if let Some(t) = s.get("text_delta").and_then(|t| t.as_str()).filter(|t| !t.is_empty()) {
                            { let mut st = self.agy.lock().unwrap(); st.text.push_str(t); st.turn_text.push_str(t); }
                            self.live_push(t);
                            if self.status() == Status::Idle { self.set_status(Status::Working); }
                        }
                        if state == "DONE" || state == "ERROR" { self.agy_flush(); }
                    }
                    "tool" | "tool_call" => {
                        let info = s.get("tool_info").cloned().unwrap_or(Value::Null);
                        let name = info.get("name").and_then(|n| n.as_str()).or(s.get("tool_name").and_then(|n| n.as_str())).unwrap_or("tool").to_string();
                        let params = info.get("parameters").cloned().unwrap_or(json!({}));
                        if state == "ACTIVE" {
                            self.agy_flush();
                            let conv = self.agy.lock().unwrap().conversation_id.clone().unwrap_or_default();
                            let tid = format!("agy-{}-{}", conv.chars().take(8).collect::<String>(), idx);
                            let (card, input) = tool_card(&name, &params);
                            { self.agy.lock().unwrap().tools.insert(idx, (tid.clone(), card.clone())); }
                            self.emit(json!({ "kind": "assistant", "content": [{ "type": "tool_use", "id": tid, "name": card, "input": input }], "uuid": tid }));
                            self.set_status(Status::Working);
                        } else {
                            let tid = { let mut st = self.agy.lock().unwrap(); st.tools.remove(&idx).map(|(t, _)| t) };
                            let Some(tid) = tid else { return };
                            let out = info.get("output").and_then(|o| o.as_str()).unwrap_or("").to_string();
                            let err = state == "ERROR";
                            // no output (DONE or ERROR — agy closes a declined step either way): if the turn result names declined
                            // actions, the card gets an explanation as a second tool_result (the UI takes the last one per id)
                            if out.trim().is_empty() { self.agy.lock().unwrap().silent.push(tid.clone()); }
                            self.emit(json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": tid, "content": out.chars().take(20000).collect::<String>(), "is_error": err }] }));
                        }
                    }
                    _ => {}
                }
            }
            "result" => {
                // a result outside a turn comes from a process we stopped ourselves (restart with new parameters,
                // interrupt): agy answers "stream input cancelled" — not journaled
                if !self.agy.lock().unwrap().in_turn { return }
                let r = v.get("result").cloned().unwrap_or(Value::Null);
                let status = r.get("status").and_then(|s| s.as_str()).unwrap_or("");
                let err = r.get("error").and_then(|e| e.as_str()).map(str::to_string);
                let stop = match status { "SUCCESS" => "end_turn", "CANCELED" | "INTERRUPTED" => "cancelled", _ => "error" };
                // Google has no capacity for the model (503 / UNAVAILABLE): one automatic retry after 20 s (§4.3.17)
                if stop == "error" && err.as_deref().map(is_capacity_error).unwrap_or(false) {
                    let first = { let mut st = self.agy.lock().unwrap(); if st.retried { false } else { st.retried = true; true } };
                    if first { self.agy_retry_later(err.unwrap_or_default()); return }
                }
                self.agy_finish_turn(stop, err, &r);
            }
            _ => {}
        }
    }

    /// Accumulated answer text → one journal message.
    pub(crate) fn agy_flush(&self) {
        let (text, model) = { let mut st = self.agy.lock().unwrap(); (std::mem::take(&mut st.text), st.model.clone()) };
        let model = model.or_else(|| self.meta.lock().unwrap().model.clone());
        if !text.trim().is_empty() { self.emit(json!({ "kind": "assistant", "content": [{ "type": "text", "text": text }], "model": model })); }
        self.live_reset();
    }

    pub(crate) fn agy_finish_turn(self: &Arc<Self>, stop: &str, err: Option<String>, r: &Value) {
        self.agy_close_turn(stop, err, r);
        self.drain_queue();
    }

    /// "No capacity" from Google: keep the turn open (status stays Working, new messages wait in the queue), explain in the
    /// journal and re-send the same text once after 20 s. Stop/interrupt meanwhile closes the turn and drops the retry.
    fn agy_retry_later(self: &Arc<Self>, err: String) {
        self.agy_flush();
        let (open, gen) = { let mut st = self.agy.lock().unwrap(); st.retry_gen += 1; (st.tools.drain().map(|(_, v)| v).collect::<Vec<_>>(), st.retry_gen) };
        for (tid, _) in open { self.emit(json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": tid, "content": "", "is_error": true }] })); }
        let short: String = err.chars().take(160).collect();
        self.note(&format!("Antigravity: у Google сейчас нет мощностей для модели ({short}) — повтор запроса через 20 с"));
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            let (text, live) = { let st = me.agy.lock().unwrap(); (st.last_text.clone(), st.in_turn && st.retry_gen == gen) };
            if !live || !me.alive() { return }
            { let mut st = me.agy.lock().unwrap(); st.turn_started_ms = now_ms(); st.text.clear(); st.silent.clear(); }
            if let Err(e) = me.write(&json!({ "event": "user", "message": { "content": text } })).await {
                me.agy_finish_turn("error", Some(format!("повтор не удался: {e}")), &json!({}));
            }
        });
    }

    /// Turn result into the journal (without the queue — agy_finish_turn releases it; on an interrupt the process is
    /// stopped anyway).
    fn agy_close_turn(&self, stop: &str, err: Option<String>, r: &Value) {
        self.agy_flush();
        // tools left without DONE (interrupted) — close them with an error so the cards stop
        // spinning
        let open: Vec<(String, String)> = { let mut st = self.agy.lock().unwrap(); st.in_turn = false; st.retry_gen += 1; st.tools.drain().map(|(_, v)| v).collect() };
        for (tid, _) in open { self.emit(json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": tid, "content": "", "is_error": true }] })); }
        let (started, text, model, conv, used) = { let st = self.agy.lock().unwrap(); (st.turn_started_ms, st.turn_text.clone(), st.model.clone(), st.conversation_id.clone(), st.last_input_tokens) };
        let (meta_model, window) = { let m = self.meta.lock().unwrap(); (m.model.clone(), m.context_window.unwrap_or(DEFAULT_WINDOW)) };
        // declined tools: agy closes their step as DONE without output — mark the cards as "declined" errors and explain with a note
        let silent: Vec<String> = std::mem::take(&mut self.agy.lock().unwrap().silent);
        if let Some(note) = r.get("denied_actions").and_then(denied_note) {
            for tid in silent { self.emit(json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": tid, "content": "отклонено: headless agy не умеет спрашивать разрешение", "is_error": true, "declined": true }] })); }
            self.note(&note);
        }
        if let Some(e) = &err { self.emit(json!({ "kind": "error", "message": e })); }
        let is_error = stop == "error";
        let subtype = match stop { "end_turn" => "success", "cancelled" => "interrupted", _ => "error_during_execution" };
        let usage = r.get("usage").map(|u| json!({ "input_tokens": u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0), "output_tokens": u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0), "thinking_tokens": u.get("thinking_tokens") })).unwrap_or(Value::Null);
        let duration = r.get("duration_seconds").and_then(|d| d.as_f64()).map(|d| (d * 1000.0) as u64).unwrap_or_else(|| now_ms().saturating_sub(started));
        let ev = json!({ "kind": "result", "subtype": subtype, "is_error": is_error, "duration_ms": duration, "num_turns": r.get("num_turns").and_then(|n| n.as_u64()).unwrap_or(1), "total_cost_usd": Value::Null, "usage": usage, "model_usage": Value::Null,
            "text": if text.trim().is_empty() { err.clone().unwrap_or_default() } else { text }, "session_id": conv, "model": model.or(meta_model), "stop_reason": stop,
            "context": { "sessionUpdate": "usage_update", "used": used, "size": window } });
        self.record_result(ev);
        self.pending.lock().unwrap().clear();
        self.set_status(Status::Idle);
    }

    /// Our rules → agy's `permissions.allow`/`deny` before the process starts (§4.3.17): the session rules
    /// (`allowed_tools`/`disallowed_tools`) plus the workspace rules of the user and project scopes (perms.rs), translated.
    pub(crate) fn agy_sync_rules(&self) {
        let meta = self.meta.lock().unwrap().clone();
        let (ua, ud) = crate::perms::rules_of("user", "");
        let (pa, pd) = crate::perms::rules_of("project", &meta.cwd);
        let tr = |src: &[&Vec<String>]| -> Vec<String> { let mut out = Vec::new(); for l in src { for r in l.iter() { if let Some(x) = claude_rule_to_agy(r) { if !out.contains(&x) { out.push(x); } } } } out };
        let allow = tr(&[&meta.allowed_tools, &ua, &pa]);
        let deny = tr(&[&meta.disallowed_tools, &ud, &pd]);
        let home = meta.env.get("HOME").cloned().unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into()));
        match sync_rules(&home, &allow, &deny) {
            Ok(true) => tracing::info!("session {}: agy permissions — allow {} deny {}", &self.id()[..8], allow.len(), deny.len()),
            Ok(false) => {}
            Err(e) => tracing::warn!("session {}: agy permissions не записаны: {e}", &self.id()[..8]),
        }
    }

    /// "Allow and retry" (§4.3.17): a Claude-style rule for the declined tool call goes into the project scope
    /// (`<cwd>/.claude/settings.local.json`, perms.rs), agy's settings get its translation, the process restarts on the
    /// same conversation (agy reads rules at start only — verified) and the conversation gets a short message to go on.
    pub(crate) async fn agy_allow_retry(self: &Arc<Self>, tool_use_id: &str) -> Result<Value> {
        let found = self.events_since(0, usize::MAX).iter().rev().find_map(|e| {
            if e.get("kind").and_then(|k| k.as_str()) != Some("assistant") { return None }
            e.get("content").and_then(|c| c.as_array()).and_then(|a| a.iter().find(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use") && b.get("id").and_then(|i| i.as_str()) == Some(tool_use_id)))
                .map(|b| (b.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(), b.get("input").cloned().unwrap_or(json!({}))))
        });
        let Some((card, input)) = found else { bail!("вызов инструмента {tool_use_id} не найден в журнале") };
        let Some(rule) = rule_for_tool(&card, &input) else { bail!("для инструмента {card} правило не выводится") };
        let cwd = self.meta.lock().unwrap().cwd.clone();
        let added = crate::perms::add_rule("project", &cwd, "allow", &rule)?;
        self.agy_sync_rules();
        self.note(&format!("{}: {rule} (область «проект», {cwd}/.claude/settings.local.json) — перезапускаю agy с этим правилом", if added { "правило добавлено" } else { "правило уже было" }));
        if self.alive() {
            let in_turn = self.agy.lock().unwrap().in_turn;
            if in_turn { self.agy_close_turn("cancelled", None, &json!({})); }
            let _ = self.stop();
            self.wait_exit(std::time::Duration::from_secs(6)).await;
        }
        self.spawn(true).await?;
        // the message goes through the usual prompt path (queued if a turn is still running) — the reply only says that it was accepted
        let ev = self.prompt(json!([{ "type": "text", "text": "Разрешение выдано — выполни команду и продолжи." }])).await?;
        Ok(json!({ "ok": true, "rule": rule, "added": added, "sent": true, "queued": ev.get("queued").is_some() }))
    }

    /// Interrupt the turn: agy has no cancel protocol — stop the process and close the turn as interrupted; the
    /// conversation resumes via `--conversation`.
    pub(crate) async fn agy_interrupt(&self) -> Result<Value> {
        let in_turn = self.agy.lock().unwrap().in_turn;
        if in_turn { self.agy_close_turn("cancelled", None, &json!({})); }
        self.stop()?;
        Ok(json!({ "ok": true, "restart": true }))
    }

    /// Capabilities for the composer: gate models, agy modes, effort levels, context
    /// usage.
    pub(crate) fn agy_caps(&self, meta: &crate::session::Meta, gate_models: &[String]) -> Value {
        let st = self.agy.lock().unwrap();
        let current = st.model.clone().or(meta.model.clone());
        // native mode (Google account): agy's own models
        if st.native && !st.native_models.is_empty() {
            let models: Vec<Value> = st.native_models.iter().map(|(id, name)| json!({ "value": id, "label": name, "description": id, "default": current.as_deref() == Some(id.as_str()) })).collect();
            let window = meta.context_window.unwrap_or(DEFAULT_WINDOW);
            let hints = json!({ "low": "быстро, без долгих размышлений", "medium": "сбалансированно", "high": "глубже думает", "xhigh": "ещё глубже", "max": "максимум размышлений" });
            return json!({ "models": models, "mode_options": mode_options(), "modes": ["default", "acceptEdits", "plan", "auto"], "current_mode": meta.permission_mode.clone().unwrap_or_else(|| "default".into()),
                "commands": [], "config_options": [], "acp_usage": { "used": st.last_input_tokens, "size": window }, "context_window": window, "model": current, "efforts": EFFORTS, "effort_hints": hints, "auth_methods": [], "tools": st.init_tools, "native": true });
        }
        let mut ids: Vec<String> = gate_models.iter().filter(|s| { let l = s.to_ascii_lowercase(); !l.contains("image") && !l.contains("imagine") && !l.contains("video") && !l.contains("-dd-") }).cloned().collect();
        let rank = |id: &str| if id.starts_with("devin/") { 4 } else if id.starts_with("claude-") { 0 } else if id.starts_with("gpt-") { 1 } else if id.starts_with("gemini-") { 2 } else { 3 };
        ids.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
        if let Some(c) = &current { if !ids.contains(c) { ids.insert(0, c.clone()); } }
        let models: Vec<Value> = ids.iter().map(|id| json!({ "value": id, "label": id, "default": current.as_deref() == Some(id.as_str()) })).collect();
        let window = meta.context_window.unwrap_or(DEFAULT_WINDOW);
        let hints = json!({ "low": "быстро, без долгих размышлений", "medium": "сбалансированно", "high": "глубже думает", "xhigh": "ещё глубже", "max": "максимум размышлений" });
        json!({
            "models": models, "mode_options": mode_options(), "modes": ["default", "acceptEdits", "plan", "auto"], "current_mode": meta.permission_mode.clone().unwrap_or_else(|| "default".into()),
            "commands": [], "config_options": [], "acp_usage": { "used": st.last_input_tokens, "size": window }, "context_window": window, "model": current,
            "efforts": EFFORTS, "effort_hints": hints, "auth_methods": [], "tools": st.init_tools,
        })
    }
}

fn strip_conversation(a: &[String]) -> Vec<String> {
    let mut out = Vec::new(); let mut skip = false;
    for x in a { if skip { skip = false; continue } if x == "--conversation" { skip = true; continue } out.push(x.clone()); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn args_and_modes() {
        let a = args(Some("gemini-3.8-flash-high"), Some("high"), Some("default"), true, None);
        assert_eq!(a, vec!["agy", "--input-format", "stream-json", "--output-format", "stream-json", "--print=", "--print-timeout", "0", "--model", "gemini-api://gemini-3.8-flash-high", "--effort", "high"]);
        assert_eq!(mode_flags(Some("auto")), vec!["--dangerously-skip-permissions"]); assert_eq!(mode_flags(Some("bypassPermissions")), vec!["--dangerously-skip-permissions"]);
        assert_eq!(mode_flags(Some("acceptEdits")), vec!["--mode", "accept-edits"]); assert_eq!(mode_flags(Some("plan")), vec!["--mode", "plan"]); assert!(mode_flags(None).is_empty());
        let b = args(None, Some("turbo"), Some("plan"), false, Some("abc")); assert_eq!(&b[8..], &["--mode", "plan", "--conversation", "abc"], "неизвестное усилие не передаём, беседа — в конце");
        assert_eq!(strip_conversation(&b), &b[..b.len() - 2]);
        assert_eq!(args(Some("gemini-api://x"), None, None, true, None)[9], "gemini-api://x");
    }
    #[test]
    fn tools_prompt_and_denied() {
        let (n, i) = tool_card("run_command", &json!({ "CommandLine": "id -un", "Cwd": "/tmp" })); assert_eq!(n, "Bash"); assert_eq!(i["command"], "id -un"); assert_eq!(i["cwd"], "/tmp"); assert_eq!(i["CommandLine"], "id -un");
        let (n, i) = tool_card("write_to_file", &json!({ "TargetFile": "/a/b.txt", "CodeContent": "hi" })); assert_eq!(n, "Write"); assert_eq!(i["file_path"], "/a/b.txt"); assert_eq!(i["content"], "hi");
        assert_eq!(tool_card("view_file", &json!({ "AbsolutePath": "/x" })).0, "Read"); assert_eq!(tool_card("browser_click_element", &json!({})).0, "Browser"); assert_eq!(tool_card("weird_tool", &json!({})).0, "weird_tool");
        assert_eq!(prompt_text(&json!([{ "type": "text", "text": "a" }, { "type": "image", "source": {} }, { "type": "text", "text": "b" }])), "a\n[изображение приложено, но headless agy принимает только текст]\nb");
        let n = denied_note(&json!([{ "action": "command", "display_name": "RunCommand" }])).unwrap(); assert!(n.contains("RunCommand (command)") && n.contains("«авто»") && n.contains("permissions.allow"));
        assert!(denied_note(&json!([])).is_none() && denied_note(&Value::Null).is_none());
        assert_eq!(mode_options().as_array().unwrap().len(), 4);
        assert_eq!(parse_models("Fetching available models...\ngemini-3.1-pro-high\tGemini 3.1 Pro (High)\nbad line\n"), vec![("gemini-3.1-pro-high".to_string(), "Gemini 3.1 Pro (High)".to_string())]);
        assert!(parse_models("Error: Please sign in").is_empty()); assert_eq!(shell_quote("/a b'c"), "'/a b'\\''c'");
    }
    #[test]
    fn rules_translate_and_sync() {
        assert_eq!(claude_rule_to_agy("Bash(npm test:*)").as_deref(), Some("command(npm test)"));
        assert_eq!(claude_rule_to_agy("Bash(git status)").as_deref(), Some("command(regex:^git status$)"));
        assert_eq!(claude_rule_to_agy("Bash(rm -rf *)").as_deref(), Some("command(rm -rf)"));
        assert_eq!(claude_rule_to_agy("Bash").as_deref(), Some("command(*)"));
        assert_eq!(claude_rule_to_agy("Read").as_deref(), Some("read_file(*)"));
        assert_eq!(claude_rule_to_agy("Edit(src/)").as_deref(), Some("write_file(src/)"));
        assert_eq!(claude_rule_to_agy("WebFetch(domain:example.com)").as_deref(), Some("read_url(example.com)"));
        assert_eq!(claude_rule_to_agy("mcp__linter__lint").as_deref(), Some("mcp(linter/lint)"));
        assert_eq!(claude_rule_to_agy("mcp__linter").as_deref(), Some("mcp(linter/*)"));
        assert!(claude_rule_to_agy("WebSearch").is_none() && claude_rule_to_agy("Agent").is_none() && claude_rule_to_agy("mcp__").is_none());
        // "allow and retry": a rule from the declined call
        assert_eq!(rule_for_tool("Bash", &json!({ "command": "npm run docs:build --silent" })).as_deref(), Some("Bash(npm run:*)"));
        assert_eq!(rule_for_tool("Bash", &json!({ "command": "ls -la" })).as_deref(), Some("Bash(ls:*)"));
        assert_eq!(rule_for_tool("Bash", &json!({ "command": "git -C /x status" })).as_deref(), Some("Bash(git:*)"));
        assert_eq!(rule_for_tool("Edit", &json!({ "file_path": "/home/coder/a.txt" })).as_deref(), Some("Edit(/home/coder/a.txt)"));
        assert_eq!(rule_for_tool("WebFetch", &json!({ "url": "https://docs.example.com/x/y" })).as_deref(), Some("WebFetch(domain:docs.example.com)"));
        assert_eq!(rule_for_tool("mcp__srv__tool", &json!({})).as_deref(), Some("mcp__srv__tool"));
        assert!(rule_for_tool("Bash", &json!({})).is_none() && rule_for_tool("Agent", &json!({})).is_none());
        assert!(is_capacity_error("Error 503, Message: No capacity available for model gemini-3-pro, Status: UNAVAILABLE") && is_capacity_error("The model is overloaded. Please try again later."));
        assert!(!is_capacity_error("API key not valid. Please pass a valid API key.") && !is_capacity_error("rate limit exceeded (429)"));
        // sync keeps the owner's rules and replaces only ours
        let dir = std::env::temp_dir().join(format!("agy-rules-{}", std::process::id())); let _ = std::fs::remove_dir_all(&dir);
        let home = dir.to_string_lossy().to_string();
        std::fs::create_dir_all(format!("{home}/.gemini/antigravity-cli")).unwrap();
        std::fs::write(format!("{home}/.gemini/antigravity-cli/settings.json"), r#"{"modelProvider":"gemini","permissions":{"allow":["command(make)"]}}"#).unwrap();
        assert!(sync_rules(&home, &["command(npm test)".into()], &["command(rm -rf)".into()]).unwrap());
        let st: Value = serde_json::from_slice(&std::fs::read(format!("{home}/.gemini/antigravity-cli/settings.json")).unwrap()).unwrap();
        assert_eq!(st["permissions"]["allow"], json!(["command(make)", "command(npm test)"])); assert_eq!(st["permissions"]["deny"], json!(["command(rm -rf)"])); assert_eq!(st["modelProvider"], "gemini");
        assert!(!sync_rules(&home, &["command(npm test)".into()], &["command(rm -rf)".into()]).unwrap());
        assert!(sync_rules(&home, &[], &[]).unwrap());
        let st: Value = serde_json::from_slice(&std::fs::read(format!("{home}/.gemini/antigravity-cli/settings.json")).unwrap()).unwrap();
        assert_eq!(st["permissions"]["allow"], json!(["command(make)"])); assert!(st["permissions"].get("deny").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
