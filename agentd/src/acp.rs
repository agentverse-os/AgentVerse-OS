//! Providers over ACP — the Agent Client Protocol (docs/agents-ux.md §4.3.8): line-delimited JSON-RPC 2.0 over stdio, one
//! protocol for Gemini CLI (`gemini --experimental-acp`), pi (the `pi-acp` adapter on top of `pi --mode rpc`) and the
//! other agents of the ACP registry. We are the client: `initialize` → (`authenticate`) → `session/new` | `session/load` →
//! `session/prompt` (the answer arrives when the turn is over); the agent sends `session/update` notifications (text and
//! thought chunks, tool calls and their results, plan, commands, mode, context usage) and `session/request_permission`
//! requests. All of it lands in the same journal as for Claude and Codex — there is one Chat card. We do not delegate the
//! file system and terminal over ACP (the agent works with the container disk itself) and pass no MCP servers in
//! `session/new`.
use crate::session::{Pending, Session, Status};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub const PROTOCOL_VERSION: u64 = 1;

/// ACP session state inside our session.
#[derive(Default)]
pub struct AcpState {
    pub session_id: Option<String>,
    /// Process start marker: request ids start from zero at every start, the journal is one for all starts
    pub epoch: u64,
    /// Resume: the history is replayed by the agent as `session/update`, we do not put it into the journal
    /// (it is already there)
    pub loading: bool,
    /// Accumulate text and thought chunks: into the journal on a tool call and at the end
    /// of the turn
    pub text: String,
    pub thought: String,
    /// Tool calls of the turn: id → (card name, agent title, accumulated output)
    pub tools: HashMap<String, (String, String, String)>,
    pub tool_order: Vec<String>,
    /// `modes` from the session/new answer: { currentModeId, availableModes: [{id, name,
    /// description}] }
    pub modes: Value,
    /// `models` (an unstable ACP field): { currentModelId, availableModels: [{modelId, name, description}] }
    pub models: Value,
    pub config_options: Value,
    /// `available_commands_update`: [{name, description, input}]
    pub commands: Vec<Value>,
    /// `usage_update` (unstable): { used, size, cost? }
    pub usage: Option<Value>,
    pub turn_started_ms: u64,
    pub turn_text: String,
    pub auth_methods: Vec<Value>,
    pub load_session: bool,
    /// Current model according to the agent (currentModelId or the model
    /// from init)
    pub model: Option<String>,
    /// pi: how many toolResult records from the session file have already been shown (bash output
    /// arrives only there)
    pub pi_tool_idx: usize,
    /// is a turn in progress (between session/prompt and its answer): agent text outside a turn is a service
    /// note, not an answer
    pub in_turn: bool,
    /// after a restart the agent could not bring up its session (session/load) — the first prompt gets a retelling
    /// of the journal prepended
    pub recap_pending: bool,
    /// The agent's modes live in configOptions (`mode`) rather than in `modes` (omp): switch via
    /// session/set_config_option
    pub mode_via_config: bool,
}

/// `modes` from the configOptions entry with id `mode`: { currentModeId, availableModes: [{id, name,
/// description}] }
pub fn modes_from_config(config: &Value) -> Option<Value> {
    let o = config.as_array()?.iter().find(|o| o.get("id").and_then(|i| i.as_str()) == Some("mode"))?;
    let opts: Vec<Value> = o.get("options")?.as_array()?.iter().filter_map(|v| Some(json!({ "id": v.get("value")?.as_str()?, "name": v.get("name").and_then(|n| n.as_str()).unwrap_or(v.get("value")?.as_str()?), "description": v.get("description") }))).collect();
    if opts.is_empty() { return None }
    Some(json!({ "currentModeId": o.get("currentValue"), "availableModes": opts }))
}

fn now_ms() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0) }

/// Tool name for the card from the ACP kind (`kind`) and title: the Chat card knows Bash/Edit/Read/Grep/WebFetch/Agent.
pub fn tool_name(kind: Option<&str>, title: &str, raw_input: &Value) -> String {
    match kind.unwrap_or("") {
        "execute" => "Bash", "edit" => "Edit", "delete" | "move" => "Bash", "read" => "Read", "search" => "Grep", "fetch" => "WebFetch", "think" => "Agent",
        _ => {
            // pi-acp and Gemini put the tool name into rawInput or the title ("run_shell_command", "bash", "edit")
            let t = title.to_ascii_lowercase();
            if t.contains("shell") || t.contains("bash") || t.starts_with("run ") || raw_input.get("command").is_some() { "Bash" } else if t.contains("edit") || t.contains("write") || t.contains("replace") { "Edit" } else if t.contains("read") { "Read" } else if t.contains("search") || t.contains("grep") || t.contains("glob") || t.contains("find") { "Grep" } else if t.contains("fetch") || t.contains("http") { "WebFetch" } else { "Tool" }
        }
    }.to_string()
}

/// Text from ACP content blocks (ToolCallContent: `content` with a ContentBlock, `diff`,
/// `terminal`).
pub fn content_text(content: &Value) -> String {
    let mut out = String::new();
    for c in content.as_array().cloned().unwrap_or_default() {
        match c.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "content" => { if let Some(t) = c.pointer("/content/text").and_then(|t| t.as_str()) { out.push_str(t); out.push('\n'); } }
            "diff" => {
                let path = c.get("path").and_then(|p| p.as_str()).unwrap_or("");
                let old = c.get("oldText").and_then(|p| p.as_str()).unwrap_or("");
                let new = c.get("newText").and_then(|p| p.as_str()).unwrap_or("");
                out.push_str(&format!("--- {path}\n"));
                for l in old.lines() { out.push_str(&format!("- {l}\n")); }
                for l in new.lines() { out.push_str(&format!("+ {l}\n")); }
            }
            "text" => { if let Some(t) = c.get("text").and_then(|t| t.as_str()) { out.push_str(t); out.push('\n'); } }
            "terminal" => { out.push_str("[terminal]\n"); }
            _ => {}
        }
    }
    out
}

/// Session mode in the agent's terms: our names (from Claude) and Codex names are translated into Gemini modes;
/// whatever the agent listed in availableModes is accepted as is.
pub fn map_mode(engine: &str, mode: &str, available: &[String]) -> Option<String> {
    // ACP engines name modes their own way (Gemini: `autoEdit`, the CLI flag — `auto_edit`): compare case-insensitively
    // and ignoring `_`/`-`
    let norm = |s: &str| s.chars().filter(|c| *c != '_' && *c != '-').collect::<String>().to_ascii_lowercase();
    if let Some(a) = available.iter().find(|a| norm(a) == norm(mode)) { return Some(a.clone()); }
    let m = if engine == "gemini" {
        match norm(mode).as_str() { "default" | "onrequest" | "untrusted" => "default", "acceptedits" | "autoedit" => "autoEdit", "bypasspermissions" | "auto" | "dontask" | "never" | "yolo" => "yolo", "plan" | "readonly" => "plan", _ => return None }
    } else { return None };
    if available.is_empty() { return Some(m.to_string()) }
    available.iter().find(|a| norm(a) == norm(m)).cloned()
}

/// Permission option for the owner's answer: allow (+ remember) → allow_always, otherwise allow_once; deny →
/// reject_once / reject_always; nothing suitable — cancel.
pub fn pick_option(options: &Value, behavior: &str, remember: bool) -> Option<String> {
    let opts: Vec<(String, String)> = options.as_array().cloned().unwrap_or_default().iter().filter_map(|o| Some((o.get("optionId").and_then(|i| i.as_str())?.to_string(), o.get("kind").and_then(|k| k.as_str()).unwrap_or("").to_string()))).collect();
    let find = |kind: &str| opts.iter().find(|(_, k)| k == kind).map(|(id, _)| id.clone());
    match (behavior, remember) {
        ("allow", true) => find("allow_always").or_else(|| find("allow_once")),
        ("allow", false) => find("allow_once").or_else(|| find("allow_always")),
        (_, true) => find("reject_always").or_else(|| find("reject_once")),
        _ => find("reject_once").or_else(|| find("reject_always")),
    }
}

/// Names that Gemini CLI replaces with its own "latest" models (resolveModel: gemini-3-flash → gemini-3.8-flash etc.); such
/// names do not pass through the gate — they are left out of the choice and the list.
const GEMINI_REMAPPED: &[&str] = &["gemini-3-flash", "gemini-3.5-flash", "gemini-3-flash-preview", "gemini-3.1-flash-lite", "gemini-3.5-flash-lite", "flash", "flash-lite", "pro", "auto"];

/// Rank of a Gemini model for the default: version number (3.8 > 3.1), then pro > flash, high > plain >
/// low.
fn gemini_rank(id: &str) -> f64 {
    let ver: f64 = id.trim_start_matches("gemini-").split('-').next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let tier = if id.contains("pro") { 0.3 } else if id.contains("flash") { 0.2 } else { 0.1 };
    let think = if id.ends_with("-high") { 0.03 } else if id.ends_with("-low") { 0.01 } else { 0.02 };
    ver * 10.0 + tier + think
}

/// Gemini models from the gate catalog that the CLI passes as is (no image, lite, foreign prefixes or replaced names),
/// best to worst.
pub fn gemini_gate_models(ids: &[String]) -> Vec<String> {
    let mut v: Vec<String> = ids.iter().filter(|s| s.starts_with("gemini-") && !s.contains("image") && !s.contains("lite") && !s.contains('/') && !GEMINI_REMAPPED.contains(&s.as_str())).cloned().collect();
    v.sort_by(|a, b| gemini_rank(b).partial_cmp(&gemini_rank(a)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.cmp(b)));
    v
}

/// Default Gemini model through the gate — the best of
/// `gemini_gate_models`.
pub fn pick_gemini_model(ids: &[String]) -> Option<String> { gemini_gate_models(ids).into_iter().next() }

/// Default pi model through the gate: the newest Claude Sonnet without a dated suffix (the gate usually no longer
/// serves those), then Opus, then GPT, otherwise the first one from the catalog.
pub fn pick_pi_model(ids: &[String]) -> Option<String> {
    let ver = |s: &str, prefix: &str| -> Option<(u32, u32)> {
        let mut nums: Vec<u32> = Vec::new();
        for seg in s.strip_prefix(prefix)?.split(|c| c == '-' || c == '.') {
            if let Ok(n) = seg.parse::<u32>() { if n >= 1000 { return None } nums.push(n); }
        }
        Some((*nums.first()?, nums.get(1).copied().unwrap_or(0)))
    };
    for prefix in ["claude-sonnet-", "claude-opus-", "gpt-"] {
        if let Some(best) = ids.iter().filter_map(|s| ver(s, prefix).map(|v| (v, s))).max_by_key(|(v, _)| *v).map(|(_, s)| s.clone()) { return Some(best) }
    }
    ids.first().cloned()
}

/// Gate provider for pi in `~/.pi/agent/models.json`: the gate's OpenAI-compatible `/v1` with the models from its
/// catalog (no image and video ones). The rest of the file is left alone; without the owner's own model settings — the
/// default is on the gate.
pub async fn pi_ensure_gate(env: &HashMap<String, String>) -> Option<String> {
    let base = env.get("OPENAI_BASE_URL")?.trim_end_matches('/').to_string();
    if !env.contains_key("OPENAI_API_KEY") { return None }
    let ids = crate::codex::gate_model_ids(env).await;
    let models: Vec<Value> = ids.iter().filter(|s| { let l = s.to_ascii_lowercase(); !l.contains("image") && !l.contains("imagine") && !l.contains("video") && !l.contains("-dd-") }).map(|id| json!({ "id": id, "name": id })).collect();
    if models.is_empty() { return None }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    let dir = env.get("PI_CODING_AGENT_DIR").cloned().unwrap_or_else(|| format!("{home}/.pi/agent"));
    let _ = std::fs::create_dir_all(&dir);
    let path = format!("{dir}/models.json");
    let mut doc: Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    if !doc.is_object() { doc = json!({}); }
    if !doc.get("providers").map(|p| p.is_object()).unwrap_or(false) { doc["providers"] = json!({}); }
    doc["providers"]["agentverse-gate"] = json!({ "baseUrl": base, "api": "openai-completions", "apiKey": "$OPENAI_API_KEY", "models": models });
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap_or_default());
    // model default: only if the owner configured nothing themselves (no defaultModel in
    // settings.json)
    let default = pick_pi_model(&ids);
    let spath = format!("{dir}/settings.json");
    let mut st: Value = std::fs::read(&spath).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    if !st.is_object() { st = json!({}); }
    // …or if our provider's default disappeared from the gate catalog (otherwise every session without an explicit model
    // ends in a 404)
    let stale = st.get("defaultProvider").and_then(|p| p.as_str()) == Some("agentverse-gate") && st.get("defaultModel").and_then(|m| m.as_str()).map(|m| !ids.iter().any(|i| i == m)).unwrap_or(false);
    if st.get("defaultModel").is_none() || stale { if let Some(d) = &default { st["defaultProvider"] = json!("agentverse-gate"); st["defaultModel"] = json!(d); } }
    if st.get("quietStartup").is_none() { st["quietStartup"] = json!(true); }
    let _ = std::fs::write(&spath, serde_json::to_vec_pretty(&st).unwrap_or_default());
    default.map(|d| format!("agentverse-gate/{d}"))
}

/// Oh My Pi through the gate (docs/agents-ux.md §4.3.18): the `agentverse-gate` provider in `~/.omp/agent/models.yml` (on
/// first start omp migrates models.json to models.yml and from then on reads only YAML — we write YAML right away via
/// serde_yaml_ng, the owner's other providers are preserved), context windows per family. The default model as the `--model
/// agentverse-gate/<id>` flag — we return it (omp does not read config.json with modelRoles).
pub async fn omp_ensure_gate(env: &HashMap<String, String>) -> Option<String> {
    let base = env.get("OPENAI_BASE_URL")?.trim_end_matches('/').to_string();
    if !env.contains_key("OPENAI_API_KEY") { return None }
    let ids = crate::codex::gate_model_ids(env).await;
    let models: Vec<Value> = ids.iter().filter(|s| { let l = s.to_ascii_lowercase(); !l.contains("image") && !l.contains("imagine") && !l.contains("video") && !l.contains("-dd-") }).map(|id| {
        let l = id.to_ascii_lowercase();
        let (ctx, max) = if l.starts_with("gemini") || l.contains("/gemini") { (1_000_000u64, 65_536u64) } else if l.starts_with("gpt") || l.starts_with("o3") || l.starts_with("o4") { (400_000, 128_000) } else { (200_000, 32_000) };
        json!({ "id": id, "name": id, "contextWindow": ctx, "maxTokens": max })
    }).collect();
    if models.is_empty() { return None }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    let dir = format!("{home}/.omp/agent");
    let _ = std::fs::create_dir_all(&dir);
    let path = format!("{dir}/models.yml");
    let provider = json!({ "baseUrl": base, "api": "openai-completions", "apiKey": "$OPENAI_API_KEY", "models": models });
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if let Some(y) = omp_models_yaml(&existing, provider) { let _ = std::fs::write(&path, y); }
    // the first-run wizard (startup.setupWizard) is not needed in the TUI: the provider is already configured — turn it off in
    // config.yml, leave the rest alone
    let cpath = format!("{dir}/config.yml");
    let ctext = std::fs::read_to_string(&cpath).unwrap_or_default();
    let mut cfg: Value = if ctext.trim().is_empty() { json!({}) } else { serde_yaml_ng::from_str::<Value>(&ctext).unwrap_or(json!({})) };
    if !cfg.is_object() { cfg = json!({}); }
    if cfg.pointer("/startup/setupWizard") != Some(&json!(false)) {
        if !cfg.get("startup").map(|v| v.is_object()).unwrap_or(false) { cfg["startup"] = json!({}); }
        cfg["startup"]["setupWizard"] = json!(false);
        if let Ok(y) = serde_yaml_ng::to_string(&cfg) { let _ = std::fs::write(&cpath, y); }
    }
    // models.json from the first version of the slice (omp migrates it to YAML at start): if only our provider is there
    // — remove it
    let jpath = format!("{dir}/models.json");
    if let Some(j) = std::fs::read(&jpath).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
        if j.get("providers").and_then(|p| p.as_object()).map(|p| p.len() == 1 && p.contains_key("agentverse-gate")).unwrap_or(false) { let _ = std::fs::remove_file(&jpath); }
    }
    pick_pi_model(&ids).map(|d| format!("agentverse-gate/{d}"))
}

/// omp models.yml with our provider: the other providers and other keys are preserved; None — nothing to
/// change.
pub fn omp_models_yaml(existing: &str, provider: Value) -> Option<String> {
    let mut doc: Value = if existing.trim().is_empty() { json!({}) } else { serde_yaml_ng::from_str::<Value>(existing).unwrap_or(json!({})) };
    if !doc.is_object() { doc = json!({}); }
    if !doc.get("providers").map(|p| p.is_object()).unwrap_or(false) { doc["providers"] = json!({}); }
    if doc["providers"].get("agentverse-gate") == Some(&provider) { return None }
    doc["providers"]["agentverse-gate"] = provider;
    match serde_yaml_ng::to_string(&doc) { Ok(y) => Some(y), Err(e) => { tracing::warn!("omp models.yml: {e}"); None } }
}

/// pi session file by ACP session id (pi-acp keeps the map `~/.pi/pi-acp/session-map.json`) — for the TUI `pi --session
/// <file>`.
pub fn pi_session_file(sid: &str) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    let v: Value = serde_json::from_slice(&std::fs::read(format!("{home}/.pi/pi-acp/session-map.json")).ok()?).ok()?;
    v.pointer(&format!("/sessions/{sid}/sessionFile")).and_then(|f| f.as_str()).map(str::to_string)
}

/// pi-acp swallows pi errors (the turn ends with end_turn and no text): look at the last assistant record in the pi session
/// file.
pub fn pi_last_error(sid: Option<&str>) -> Option<String> {
    let s = std::fs::read_to_string(pi_session_file(sid?)?).ok()?;
    let last = s.lines().rev().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v.get("type").and_then(|t| t.as_str()) == Some("message") && v.pointer("/message/role").and_then(|r| r.as_str()) == Some("assistant"))?;
    if last.pointer("/message/stopReason").and_then(|r| r.as_str()) != Some("error") { return None }
    Some(last.pointer("/message/errorMessage").and_then(|e| e.as_str()).unwrap_or("ошибка модели").to_string())
}

/// For bash pi-acp returns only a terminal reference (`{type: terminal}`), not the output; the output lies in the pi
/// session file as a `toolResult` record. Take the first not-yet-shown record after `skip` of them; return (text, error?,
/// new skip).
pub fn pi_tool_output(sid: Option<&str>, skip: usize) -> Option<(String, bool, usize)> {
    let s = std::fs::read_to_string(pi_session_file(sid?)?).ok()?;
    let mut seen = 0usize;
    for l in s.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        if v.get("type").and_then(|t| t.as_str()) != Some("message") || v.pointer("/message/role").and_then(|r| r.as_str()) != Some("toolResult") { continue }
        seen += 1;
        if seen <= skip { continue }
        let text = content_text(v.pointer("/message/content").unwrap_or(&Value::Null));
        let is_err = v.pointer("/message/isError").and_then(|e| e.as_bool()).unwrap_or(false);
        return Some((text, is_err, seen));
    }
    None
}

/// Gemini CLI: the autoEdit/yolo modes are available only in a "trusted" folder (`setApprovalMode` fails with Internal
/// error otherwise); trust is stored in `~/.gemini/trustedFolders.json` — add the session cwd as TRUST_FOLDER, leave the
/// rest alone.
pub fn gemini_ensure_trust(cwd: &str) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    let dir = format!("{home}/.gemini");
    let _ = std::fs::create_dir_all(&dir);
    let path = format!("{dir}/trustedFolders.json");
    let mut doc: Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    if !doc.is_object() { doc = json!({}); }
    if doc.get(cwd).is_some() { return }
    doc[cwd] = json!("TRUST_FOLDER");
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap_or_default());
}

/// Gemini CLI over ACP does not return shell command output (its returnDisplay is a terminal object,
/// `toToolCallContent` yields null), but writes it into the conversation record
/// `~/.gemini/tmp/<project>/chats/session-<date>-<8 chars of id>.jsonl`: the line `{"type":"gemini","toolCalls":[{id,
/// result:[{functionResponse:{response:{output}}}], resultDisplay:[[{text}]]}]}`.
pub fn gemini_tool_output(sid: Option<&str>, tid: &str) -> Option<(String, bool)> {
    let sid = sid?; let short: String = sid.chars().take(8).collect();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for proj in std::fs::read_dir(format!("{home}/.gemini/tmp")).ok()?.flatten() {
        if let Ok(rd) = std::fs::read_dir(proj.path().join("chats")) {
            files.extend(rd.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("session-") && n.ends_with(&format!("-{short}.jsonl"))).unwrap_or(false)));
        }
    }
    for f in files {
        let s = std::fs::read_to_string(&f).ok()?;
        for l in s.lines().rev() {
            let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
            let Some(calls) = v.get("toolCalls").and_then(|c| c.as_array()) else { continue };
            for c in calls {
                if c.get("id").and_then(|i| i.as_str()) != Some(tid) { continue }
                let is_err = c.get("status").and_then(|s| s.as_str()).map(|s| s != "success").unwrap_or(false);
                // resultDisplay — terminal lines (segments padded with spaces to the window width); otherwise — the function
                // output without a wrapper
                let mut lines: Vec<String> = c.get("resultDisplay").and_then(|d| d.as_array()).map(|rows| rows.iter().map(|row| row.as_array().map(|segs| segs.iter().filter_map(|sg| sg.get("text").and_then(|t| t.as_str())).collect::<String>()).unwrap_or_default().trim_end().to_string()).collect()).unwrap_or_default();
                while lines.last().map(|l| l.is_empty()).unwrap_or(false) { lines.pop(); }
                if lines.is_empty() {
                    if let Some(out) = c.pointer("/result/0/functionResponse/response/output").and_then(|o| o.as_str()) {
                        lines = out.lines().filter(|l| !l.starts_with("<untrusted_context>") && !l.starts_with("</untrusted_context>") && !l.starts_with("Process Group PGID:")).map(|l| l.strip_prefix("Output: ").unwrap_or(l).to_string()).collect();
                    }
                }
                return Some((lines.join("\n"), is_err));
            }
        }
    }
    None
}

/// pi turn tokens — from the session file: the sum of `usage` of the assistant records after the last user
/// message (over ACP pi-acp reports only the context usage).
pub fn pi_turn_usage(sid: Option<&str>) -> Option<Value> {
    let s = std::fs::read_to_string(pi_session_file(sid?)?).ok()?;
    let (mut inp, mut out, mut cr, mut cw, mut n) = (0u64, 0u64, 0u64, 0u64, 0u32);
    for l in s.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        if v.get("type").and_then(|t| t.as_str()) != Some("message") { continue }
        match v.pointer("/message/role").and_then(|r| r.as_str()) {
            Some("user") => break,
            Some("assistant") => {
                let g = |k: &str| v.pointer(&format!("/message/usage/{k}")).and_then(|x| x.as_u64()).unwrap_or(0);
                inp += g("input"); out += g("output"); cr += g("cacheRead"); cw += g("cacheWrite"); n += 1;
            }
            _ => {}
        }
    }
    if n == 0 { return None }
    Some(json!({ "input_tokens": inp, "output_tokens": out, "cache_read_input_tokens": cr, "cache_creation_input_tokens": cw }))
}

/// Model value for the agent: for pi it is `provider/model` — a bare name is completed from its list (our gate provider
/// first).
pub fn acp_model_value(st: &AcpState, model: &str) -> String {
    if model.contains('/') { return model.to_string() }
    let mut candidates: Vec<String> = Vec::new();
    if let Some(opts) = st.config_options.as_array() {
        for o in opts.iter().filter(|o| o.get("id").and_then(|i| i.as_str()) == Some("model")) {
            if let Some(vs) = o.get("options").and_then(|v| v.as_array()) { candidates.extend(vs.iter().filter_map(|v| v.get("value").and_then(|x| x.as_str()).map(str::to_string))); }
        }
    }
    if let Some(ms) = st.models.get("availableModels").and_then(|a| a.as_array()) { candidates.extend(ms.iter().filter_map(|m| m.get("modelId").and_then(|x| x.as_str()).map(str::to_string))); }
    let suffix = format!("/{model}");
    candidates.iter().find(|c| c.as_str() == format!("agentverse-gate/{model}")).or_else(|| candidates.iter().find(|c| c.ends_with(&suffix))).cloned().unwrap_or_else(|| model.to_string())
}

/// Antigravity through the gate (docs/agents-ux.md §4.3.17): `agy`'s own Gemini API key mode — `modelProvider: "gemini"` in
/// `~/.gemini/antigravity-cli/settings.json` plus GEMINI_API_KEY and GOOGLE_GEMINI_BASE_URL from the gate environment; gate
/// models are passed as `gemini-api://<id>` (that way agy sends the id as is; "known" names like gemini-3.8-flash-high it cuts
/// down itself to gemini-3.8-flash with a thinking level, and the gate has no such id). Returns whether the gate mode is on
/// (key and address in place).
pub fn antigravity_gate(env: &HashMap<String, String>) -> bool {
    env.contains_key("GEMINI_API_KEY") && (env.contains_key("GEMINI_BASE_URL") || env.contains_key("GOOGLE_GEMINI_BASE_URL"))
}
pub fn antigravity_ensure_gate(env: &HashMap<String, String>) -> bool {
    if !antigravity_gate(env) { return false }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    let dir = format!("{home}/.gemini/antigravity-cli");
    let _ = std::fs::create_dir_all(&dir);
    let path = format!("{dir}/settings.json");
    let mut st: Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    if !st.is_object() { st = json!({}); }
    if st.get("modelProvider").and_then(|v| v.as_str()) != Some("gemini") {
        st["modelProvider"] = json!("gemini");
        let _ = std::fs::write(&path, serde_json::to_vec_pretty(&st).unwrap_or_default());
    }
    true
}
/// Model for `agy` in gate mode: `gemini-api://<id>`; its own URL forms and the mode without a gate
/// — as is.
pub fn antigravity_model(model: &str, gate: bool) -> String {
    let m = model.trim();
    if gate && !m.contains("://") && !m.starts_with("blade:") && !m.starts_with("evergreen:") { format!("gemini-api://{m}") } else { m.to_string() }
}
/// Environment variables of the Antigravity adapter/TUI: the gate address in GOOGLE_GEMINI_BASE_URL (agy reads
/// only that one).
pub fn antigravity_env(env: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if !env.contains_key("GOOGLE_GEMINI_BASE_URL") { if let Some(b) = env.get("GEMINI_BASE_URL") { v.push(("GOOGLE_GEMINI_BASE_URL".to_string(), b.clone())); } }
    v
}

/// Arguments of the google-antigravity-acp adapter (docs/agents-ux.md §4.3.17): model `-m`, effort `-e` (the adapter knows only
/// low|medium|high — anything else is not passed, the model takes its default), permissions — the adapter approves everything
/// itself by default, so in the "ask" modes we set `--no-skip-permissions`, and in auto/bypass we leave the auto-approval.
pub fn antigravity_args(model: Option<&str>, effort: Option<&str>, mode: Option<&str>, gate: bool) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(m) = model.filter(|m| !m.trim().is_empty()) { a.push("-m".into()); a.push(antigravity_model(m, gate)); }
    if let Some(e) = effort.map(str::trim).filter(|e| ["low", "medium", "high"].contains(e)) { a.push("-e".into()); a.push(e.to_string()); }
    let norm = mode.unwrap_or("default").chars().filter(|c| *c != '_' && *c != '-').collect::<String>().to_ascii_lowercase();
    if !matches!(norm.as_str(), "auto" | "yolo" | "bypasspermissions" | "dontask" | "never" | "acceptedits" | "autoedit") { a.push("--no-skip-permissions".into()); }
    a
}

/// Can the agent change the model on the fly: via `configOptions` (id `model`, as pi-acp does) or `session/set_model`
/// (Gemini).
pub fn acp_model_switchable(st: &AcpState) -> bool {
    st.config_options.as_array().map(|a| a.iter().any(|o| o.get("id").and_then(|i| i.as_str()) == Some("model"))).unwrap_or(false)
        || st.models.get("availableModels").and_then(|a| a.as_array()).map(|a| !a.is_empty()).unwrap_or(false)
}

impl Session {
    pub(crate) async fn acp_call(&self, method: &str, params: Value, timeout_s: u64) -> Result<Value> {
        let id = self.next_ctrl_id();
        let rx = self.ctrl_wait(&id);
        self.write(&json!({ "jsonrpc": "2.0", "id": id.parse::<u64>().map(Value::from).unwrap_or(Value::String(id.clone())), "method": method, "params": params })).await?;
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), rx).await {
            Ok(Ok(v)) => {
                if let Some(err) = v.get("error") {
                    // error details (`data.details` of the adapters, e.g. "agy process exited before emitting init event") — into the text, otherwise only "Internal error"
                    // is visible
                    let details = err.get("data").map(|d| d.get("details").and_then(|x| x.as_str()).map(str::to_string).unwrap_or_else(|| if d.is_string() { d.as_str().unwrap_or("").to_string() } else { d.to_string() })).filter(|d| !d.is_empty() && d != "null").map(|d| format!(" — {}", d.chars().take(300).collect::<String>())).unwrap_or_default();
                    bail!("{method}: {} (код {}){details}", err.get("message").and_then(|m| m.as_str()).unwrap_or("ошибка"), err.get("code").and_then(|c| c.as_i64()).unwrap_or(0));
                }
                Ok(v.get("result").cloned().unwrap_or(Value::Null))
            }
            Ok(Err(_)) => bail!("процесс завершился до ответа на {method}"),
            Err(_) => { self.ctrl_forget(&id); bail!("нет ответа на {method} за {timeout_s} с") }
        }
    }

    async fn acp_notify(&self, method: &str, params: Value) -> Result<()> {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params })).await
    }

    fn acp_persist(&self) {
        let st = self.acp.lock().unwrap();
        let v = json!({ "modes": st.modes, "models": st.models, "commands": st.commands, "config_options": st.config_options, "model": st.model });
        let _ = std::fs::write(self.dir().join("acp.json"), serde_json::to_vec(&v).unwrap_or_default());
    }

    /// Start: initialize → (authenticate) → session/load on resume or session/new. The model — for Gemini as a flag at
    /// start, for the others `session/set_model` if the agent returned a model list.
    pub(crate) async fn acp_start(self: &Arc<Self>, resume: bool) -> Result<()> {
        let meta = self.meta.lock().unwrap().clone();
        let engine = crate::engines::get(&meta.agent).ok_or_else(|| anyhow!("неизвестный движок {}", meta.agent))?;
        { let mut st = self.acp.lock().unwrap(); st.epoch += 1; st.session_id = None; st.loading = false; st.text.clear(); st.thought.clear(); st.tools.clear(); st.tool_order.clear(); }
        let init = self.acp_call("initialize", json!({ "protocolVersion": PROTOCOL_VERSION, "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false }, "clientInfo": { "name": "cloudos-agentd", "title": "AgentVerse OS", "version": env!("CARGO_PKG_VERSION") } }), 60).await?;
        {
            let mut st = self.acp.lock().unwrap();
            st.auth_methods = init.get("authMethods").and_then(|a| a.as_array()).cloned().unwrap_or_default();
            st.load_session = init.pointer("/agentCapabilities/loadSession").and_then(|b| b.as_bool()).unwrap_or(false);
        }
        let env = crate::codex::effective_env(&meta);
        let gate = env.contains_key("OPENAI_BASE_URL") && !env.contains_key("CODEX_HOME");
        // the gate model catalog — for the whole workspace (the Gemini model menu of stopped
        // sessions too)
        if engine.id == "gemini" && gate { let ids = crate::codex::gate_model_ids(&env).await; if !ids.is_empty() { if let Some(parent) = self.dir().parent() { let _ = std::fs::write(parent.join("gate-models.json"), serde_json::to_vec(&ids).unwrap_or_default()); } } }
        let can_load = self.acp.lock().unwrap().load_session;
        let mut restarted = false;
        let mut res = match (&meta.provider_session_id, resume && can_load) {
            (Some(sid), true) => {
                self.acp.lock().unwrap().loading = true;
                let r = self.acp_call("session/load", json!({ "sessionId": sid, "cwd": meta.cwd, "mcpServers": crate::mcp::acp_servers() }), 300).await;
                self.acp.lock().unwrap().loading = false;
                match r {
                    Ok(mut v) => { if v.get("sessionId").is_none() { v["sessionId"] = json!(sid); } Ok(v) }
                    Err(e) => { tracing::warn!("session {}: session/load: {e:#} — новая ACP-сессия на том же журнале", &meta.id[..8]); restarted = true; self.acp_call("session/new", json!({ "cwd": meta.cwd, "mcpServers": crate::mcp::acp_servers() }), 120).await }
                }
            }
            _ => self.acp_call("session/new", json!({ "cwd": meta.cwd, "mcpServers": crate::mcp::acp_servers() }), 120).await,
        };
        // not signed in: Gemini through the gate — the gateway method (address from GOOGLE_GEMINI_BASE_URL), with a key —
        // gemini-api-key; then once more
        if let Err(e) = &res {
            let msg = e.to_string();
            if msg.contains("-32000") || msg.to_ascii_lowercase().contains("auth") {
                let method = if meta.agent == "gemini" {
                    if env.contains_key("GOOGLE_GEMINI_BASE_URL") || env.contains_key("GEMINI_BASE_URL") { Some("gateway") } else if env.contains_key("GEMINI_API_KEY") { Some("gemini-api-key") } else { None }
                } else { None };
                match method {
                    Some(m) => {
                        self.acp_call("authenticate", json!({ "methodId": m }), 120).await.map_err(|e2| anyhow!("вход {m}: {e2:#}"))?;
                        res = self.acp_call("session/new", json!({ "cwd": meta.cwd, "mcpServers": crate::mcp::acp_servers() }), 120).await;
                    }
                    None => bail!("{}: нужен вход — {} ({msg})", engine.title, engine.login_hint),
                }
            }
        }
        let r = res?;
        let sid = r.get("sessionId").and_then(|s| s.as_str()).ok_or_else(|| anyhow!("session/new без sessionId"))?.to_string();
        {
            let mut st = self.acp.lock().unwrap();
            st.session_id = Some(sid.clone());
            if let Some(m) = r.get("modes").filter(|m| m.is_object()) { st.modes = m.clone(); }
            if let Some(m) = r.get("models").filter(|m| m.is_object()) { st.models = m.clone(); st.model = m.get("currentModelId").and_then(|c| c.as_str()).map(str::to_string); }
            if let Some(c) = r.get("configOptions").filter(|c| c.is_array()) { st.config_options = c.clone(); }
            // modes only in configOptions (omp: `mode` default | plan) — bring them to the modes shape, switching —
            // set_config_option
            if st.modes.is_null() { if let Some(m) = modes_from_config(&st.config_options) { st.modes = m; st.mode_via_config = true; } }
        }
        {
            let mut m = self.meta.lock().unwrap();
            m.provider_session_id = Some(sid.clone());
            // the model from the agent's answer (pi: currentModelId) if the session did
            // not set its own
            if m.model.is_none() { if let Some(cur) = self.acp.lock().unwrap().model.clone() { m.model = Some(cur); } }
        }
        self.save_meta();
        // pi through the gate without an explicit model: the agent's current model may have disappeared from the gate catalog (404 on
        // every turn) — check and change
        if engine.id == "pi" && gate && meta.model.is_none() {
            let cur = { let st = self.acp.lock().unwrap(); st.config_options.as_array().and_then(|a| a.iter().find(|o| o.get("id").and_then(|i| i.as_str()) == Some("model"))).and_then(|o| o.get("currentValue").and_then(|v| v.as_str()).map(str::to_string)) };
            if let Some(cur) = cur { if let Some(bare) = cur.strip_prefix("agentverse-gate/") {
                let ids = crate::codex::gate_model_ids(&env).await;
                if !ids.is_empty() && !ids.iter().any(|i| i == bare) {
                    if let Some(pick) = pick_pi_model(&ids) {
                        match self.acp_apply_model(&sid, &pick).await {
                            Ok(v) => { self.meta.lock().unwrap().model = Some(v.clone()); self.save_meta(); self.emit(json!({ "kind": "system", "subtype": "agent_note", "data": { "message": format!("модели {cur} в каталоге gate больше нет — сессия переключена на {v}") } })); }
                            Err(e) => tracing::warn!("session {}: pi: смена устаревшей модели {cur}: {e:#}", &meta.id[..8]),
                        }
                    }
                }
            } }
        }
        // the session model: for Gemini already as a flag at start; for the others — session/set_model if the
        // agent returned a list
        let switchable = acp_model_switchable(&self.acp.lock().unwrap());
        if let Some(model) = meta.model.clone() {
            let current = self.acp.lock().unwrap().model.clone();
            if switchable && engine.id != "gemini" && current.as_deref().map(|c| c != model && !c.ends_with(&format!("/{model}"))).unwrap_or(true) {
                if let Err(e) = self.acp_apply_model(&sid, &model).await { tracing::warn!("session {}: set_model {model}: {e:#}", &meta.id[..8]); self.emit(json!({ "kind": "system", "subtype": "provider_error", "data": { "message": format!("модель {model}: {e:#}") } })); }
            }
        }
        // the session mode (ours or Codex's) → the agent's mode, if it knows
        // such
        let available: Vec<String> = self.acp.lock().unwrap().modes.get("availableModes").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect()).unwrap_or_default();
        if let Some(mode) = meta.permission_mode.as_deref() {
            if let Some(target) = map_mode(engine.id, mode, &available) {
                let cur = self.acp.lock().unwrap().modes.get("currentModeId").and_then(|c| c.as_str()).map(str::to_string);
                if cur.as_deref() != Some(target.as_str()) {
                    let via_config = self.acp.lock().unwrap().mode_via_config;
                    let r = if via_config { self.acp_call("session/set_config_option", json!({ "sessionId": sid, "configId": "mode", "value": target }), 60).await } else { self.acp_call("session/set_mode", json!({ "sessionId": sid, "modeId": target }), 60).await };
                    if let Err(e) = r { tracing::warn!("session {}: set_mode {target}: {e:#}", &meta.id[..8]); }
                    else { self.acp.lock().unwrap().modes["currentModeId"] = json!(target); }
                }
            }
        }
        self.acp_persist();
        let (modes, models, model) = { let st = self.acp.lock().unwrap(); (st.modes.clone(), st.models.clone(), st.model.clone().or(meta.model.clone())) };
        if resume && restarted { self.acp.lock().unwrap().recap_pending = true; }
        self.emit(json!({ "kind": "init", "model": model, "session_id": sid, "cwd": meta.cwd, "permission_mode": modes.get("currentModeId").cloned().unwrap_or(json!(meta.permission_mode)), "provider": engine.id, "protocol": "acp", "route": if gate { "gate" } else { "own" }, "modes": modes.get("availableModes"), "models": models.get("availableModels"), "loaded": resume && !restarted }));
        if restarted { self.emit(json!({ "kind": "system", "subtype": "thread_restarted", "data": { "message": "прежняя сессия агента не загрузилась — беседа продолжается в новой, история агента начата заново" } })); }
        if self.status() == Status::Starting { self.set_status(Status::Idle); self.drain_queue(); }
        Ok(())
    }

    /// User message → session/prompt; the answer (stopReason) means the end of the turn. Retelling the conversation from the
    /// journal (slice 38): the engine did not bring up its session after a restart — we hand it the history as the first
    /// message.
    fn journal_recap(&self, max_chars: usize) -> String {
        let events = self.events_since(0, 100_000);
        let mut lines: Vec<String> = Vec::new();
        for e in events.iter() {
            let kind = e.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            if e.get("parent_tool_use_id").map(|p| !p.is_null()).unwrap_or(false) { continue }
            let text = crate::session::event_text(e);
            if text.trim().is_empty() { continue }
            match kind { "user" => lines.push(format!("Владелец: {}", text.trim())), "assistant" => lines.push(format!("Агент: {}", text.trim())), _ => {} }
        }
        let mut out: Vec<String> = Vec::new(); let mut used = 0usize;
        for l in lines.iter().rev() { let n = l.chars().count() + 1; if used + n > max_chars { break } used += n; out.push(l.clone()); }
        out.reverse();
        out.join("\n")
    }

    pub(crate) async fn acp_prompt(self: &Arc<Self>, content: &Value) -> Result<()> {
        let sid = self.acp.lock().unwrap().session_id.clone().ok_or_else(|| anyhow!("ACP-сессия ещё не создана"))?;
        // after a restart without session/load: the first prompt gets a retelling of the history (up to 8 thousand
        // characters), then as usual
        let recap = if std::mem::replace(&mut self.acp.lock().unwrap().recap_pending, false) { let r = self.journal_recap(8000); if r.is_empty() { None } else { Some(r) } } else { None };
        let content: Value = match recap {
            Some(r) => { let mut arr = content.as_array().cloned().unwrap_or_default(); let prefix = format!("Это продолжение беседы после перезапуска; предыдущие сообщения:\n{r}\n---\n");
                self.emit(json!({ "kind": "system", "subtype": "recap_sent", "data": { "message": format!("после перезапуска агенту передан пересказ беседы ({} строк, {} знаков)", r.lines().count(), r.chars().count()) } }));
                match arr.iter_mut().find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")) { Some(b) => { let t = b.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(); b["text"] = json!(format!("{prefix}{t}")); } None => arr.insert(0, json!({ "type": "text", "text": prefix })) }
                Value::Array(arr) }
            None => content.clone(),
        };
        let content = &content;
        let blocks: Vec<Value> = content.as_array().cloned().unwrap_or_default().into_iter().filter_map(|b| match b.get("type").and_then(|t| t.as_str()) {
            Some("text") => Some(json!({ "type": "text", "text": b.get("text").and_then(|t| t.as_str()).unwrap_or("") })),
            Some("image") => Some(json!({ "type": "image", "data": b.pointer("/source/data").and_then(|d| d.as_str()).unwrap_or(""), "mimeType": b.pointer("/source/media_type").and_then(|d| d.as_str()).unwrap_or("image/png") })),
            _ => None,
        }).collect();
        { let mut st = self.acp.lock().unwrap(); st.turn_started_ms = now_ms(); st.turn_text.clear(); st.text.clear(); st.thought.clear(); st.in_turn = true; }
        let id = self.next_ctrl_id();
        let rx = self.ctrl_wait(&id);
        self.write(&json!({ "jsonrpc": "2.0", "id": id.parse::<u64>().map(Value::from).unwrap_or(Value::String(id.clone())), "method": "session/prompt", "params": { "sessionId": sid, "prompt": blocks } })).await?;
        let me = self.clone();
        tokio::spawn(async move {
            let r = tokio::time::timeout(std::time::Duration::from_secs(4 * 3600), rx).await;
            let (stop, err, meta): (String, Option<String>, Option<Value>) = match r {
                Ok(Ok(v)) => match v.get("error") {
                    Some(e) => ("error".into(), Some(e.get("message").and_then(|m| m.as_str()).unwrap_or("ошибка").to_string()), None),
                    None => (v.pointer("/result/stopReason").and_then(|s| s.as_str()).unwrap_or("end_turn").to_string(), None, v.pointer("/result/_meta").cloned()),
                },
                Ok(Err(_)) => return, // the process exited — exit is already in the journal
                Err(_) => { me.ctrl_forget(&id); ("error".into(), Some("нет ответа на ход за 4 часа".into()), None) }
            };
            me.acp_finish_turn(&stop, err, meta);
        });
        Ok(())
    }

    /// End of turn: report what was accumulated, write result (ACP gives no costs — tokens are zero, the turn is
    /// counted).
    fn acp_finish_turn(self: &Arc<Self>, stop: &str, err: Option<String>, meta: Option<Value>) {
        self.acp_flush();
        self.acp.lock().unwrap().in_turn = false;
        let (started, text, model, usage, sid) = { let st = self.acp.lock().unwrap(); (st.turn_started_ms, st.turn_text.clone(), st.model.clone(), st.usage.clone(), st.session_id.clone()) };
        let (meta_model, agent) = { let m = self.meta.lock().unwrap(); (m.model.clone(), m.agent.clone()) };
        // pi-acp returns end_turn on a model error too (404/401 from the gate) — without text; the truth comes from
        // the pi session file
        let mut stop = stop.to_string(); let mut err = err;
        if agent == "pi" && stop == "end_turn" && text.trim().is_empty() && err.is_none() {
            if let Some(e) = pi_last_error(sid.as_deref()) { stop = "error".into(); err = Some(e); }
        }
        let stop = stop.as_str();
        let is_error = stop == "error" || stop == "refusal";
        let subtype = match stop { "end_turn" => "success", "cancelled" => "interrupted", "max_tokens" => "max_tokens", "max_turn_requests" => "max_turns", "refusal" => "refusal", _ => "error_during_execution" };
        if let Some(e) = &err { self.emit(json!({ "kind": "error", "message": e })); }
        // tokens: Gemini puts them into `_meta.quota.token_count` of the prompt answer, pi — only into the
        // session file
        let tokens: Value = if agent == "pi" { pi_turn_usage(sid.as_deref()).unwrap_or(Value::Null) } else {
            match meta.as_ref().and_then(|m| m.pointer("/quota/token_count")) {
                Some(tc) => json!({ "input_tokens": tc.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0), "output_tokens": tc.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0) }),
                None => Value::Null,
            }
        };
        let ev = json!({ "kind": "result", "subtype": subtype, "is_error": is_error, "duration_ms": now_ms().saturating_sub(started), "num_turns": 1, "total_cost_usd": Value::Null, "usage": tokens, "model_usage": Value::Null,
            "text": if text.trim().is_empty() { err.clone().unwrap_or_default() } else { text }, "session_id": self.acp.lock().unwrap().session_id, "model": model.or(meta_model), "stop_reason": stop, "context": usage });
        self.record_result(ev);
        self.pending.lock().unwrap().clear();
        self.set_status(Status::Idle);
        self.drain_queue();
    }

    /// Accumulated thoughts and text — into the
    /// journal.
    fn acp_flush(&self) {
        let (thought, text, model) = { let mut st = self.acp.lock().unwrap(); (std::mem::take(&mut st.thought), std::mem::take(&mut st.text), st.model.clone()) };
        if !thought.trim().is_empty() { self.emit(json!({ "kind": "assistant", "content": [{ "type": "thinking", "thinking": thought }] })); }
        if !text.trim().is_empty() { self.emit(json!({ "kind": "assistant", "content": [{ "type": "text", "text": text }], "model": model })); }
        self.live_reset();
    }

    pub(crate) async fn acp_on_message(self: &Arc<Self>, v: Value) {
        let method = v.get("method").and_then(|m| m.as_str()).map(str::to_string);
        let id = v.get("id").cloned();
        match (method, id) {
            (None, Some(id)) => {
                let key = match &id { Value::Number(n) => n.to_string(), Value::String(s) => s.clone(), _ => String::new() };
                self.ctrl_resolve(&key, v);
            }
            (Some(m), Some(id)) => self.acp_on_request(&m, id, v.get("params").cloned().unwrap_or(Value::Null)).await,
            (Some(m), None) => self.acp_on_notification(&m, v.get("params").cloned().unwrap_or(Value::Null)),
            (None, None) => {}
        }
    }

    fn acp_on_notification(self: &Arc<Self>, method: &str, params: Value) {
        if method != "session/update" { return }
        if self.acp.lock().unwrap().loading { return }
        let u = params.get("update").cloned().unwrap_or(Value::Null);
        let kind = u.get("sessionUpdate").and_then(|k| k.as_str()).unwrap_or("");
        match kind {
            "agent_message_chunk" => {
                // outside a turn (Gemini sends a text notification after session/set_mode) — a service line, the status
                // is untouched
                let in_turn = self.acp.lock().unwrap().in_turn;
                if !in_turn { if let Some(t) = u.pointer("/content/text").and_then(|t| t.as_str()).filter(|t| !t.trim().is_empty()) { self.emit(json!({ "kind": "system", "subtype": "agent_note", "data": { "message": t.trim() } })); } return }
                if let Some(t) = u.pointer("/content/text").and_then(|t| t.as_str()) { { let mut st = self.acp.lock().unwrap(); st.text.push_str(t); st.turn_text.push_str(t); } self.live_push(t); }
                if self.status() == Status::Idle { self.set_status(Status::Working); }
            }
            "agent_thought_chunk" => { if let Some(t) = u.pointer("/content/text").and_then(|t| t.as_str()) { self.acp.lock().unwrap().thought.push_str(t); } }
            "user_message_chunk" => {}
            "tool_call" => {
                self.acp_flush();
                let tid = u.get("toolCallId").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let title = u.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let raw = u.get("rawInput").cloned().unwrap_or(Value::Null);
                let name = tool_name(u.get("kind").and_then(|k| k.as_str()), &title, &raw);
                let mut input = if raw.is_object() { raw.clone() } else { json!({}) };
                if input.get("title").is_none() && !title.is_empty() { input["title"] = json!(title); }
                // the content at call start is a description (Gemini: "[cwd] (why)"), not output: into
                // input.detail
                if let Some(c) = u.get("content").filter(|c| c.is_array()) { let txt = content_text(c); if !txt.trim().is_empty() && !txt.trim().starts_with("[terminal]") && input.get("detail").is_none() { input["detail"] = json!(txt.trim_end()); } }
                if let Some(l) = u.get("locations").filter(|l| l.as_array().map(|a| !a.is_empty()).unwrap_or(false)) { input["locations"] = l.clone(); }
                if name == "Bash" && input.get("command").is_none() && !title.is_empty() { input["command"] = json!(title); }
                { let mut st = self.acp.lock().unwrap(); st.tools.insert(tid.clone(), (name.clone(), title.clone(), String::new())); st.tool_order.push(tid.clone()); }
                self.emit(json!({ "kind": "assistant", "content": [{ "type": "tool_use", "id": tid, "name": name, "input": input }], "uuid": tid }));
                // already finished (the agent sent the
                // result right away)
                let status = u.get("status").and_then(|s| s.as_str()).unwrap_or("pending");
                if matches!(status, "completed" | "failed") { self.acp_tool_done(&tid, status, &u); }
                self.set_status(Status::Working);
            }
            "tool_call_update" => {
                let tid = u.get("toolCallId").and_then(|t| t.as_str()).unwrap_or("").to_string();
                // content in an update is the full state (not a delta): replace the accumulated
                // output
                if let Some(c) = u.get("content").filter(|c| c.is_array()) { let txt = content_text(c); if let Some(t) = self.acp.lock().unwrap().tools.get_mut(&tid) { if !txt.trim().is_empty() { t.2 = txt; } } }
                if let Some(status) = u.get("status").and_then(|s| s.as_str()) { if matches!(status, "completed" | "failed") { self.acp_tool_done(&tid, status, &u); } }
            }
            "plan" => { self.emit(json!({ "kind": "system", "subtype": "plan", "data": { "entries": u.get("entries") } })); }
            "available_commands_update" => { self.acp.lock().unwrap().commands = u.get("availableCommands").and_then(|a| a.as_array()).cloned().unwrap_or_default(); self.acp_persist(); }
            "current_mode_update" => {
                if let Some(m) = u.get("currentModeId").and_then(|m| m.as_str()) {
                    self.acp.lock().unwrap().modes["currentModeId"] = json!(m);
                    self.meta.lock().unwrap().permission_mode = Some(m.to_string()); self.save_meta(); self.acp_persist();
                    self.emit(json!({ "kind": "permission_mode", "mode": m }));
                }
            }
            "config_option_update" => { if let Some(c) = u.get("configOptions") { self.acp.lock().unwrap().config_options = c.clone(); self.acp_persist(); } }
            "usage_update" => { self.acp.lock().unwrap().usage = Some(u.clone()); }
            _ => {}
        }
    }

    fn acp_tool_done(self: &Arc<Self>, tid: &str, status: &str, u: &Value) {
        let (name, out) = { let mut st = self.acp.lock().unwrap(); match st.tools.remove(tid) { Some((n, _, o)) => (n, o), None => ("Tool".to_string(), String::new()) } };
        let mut text = out;
        if text.trim().is_empty() { if let Some(r) = u.get("rawOutput") { text = if let Some(s) = r.as_str() { s.to_string() } else { serde_json::to_string_pretty(r).unwrap_or_default() }; } }
        let _ = name;
        let agent = self.meta.lock().unwrap().agent.clone();
        if (agent == "pi" || agent == "gemini") && (text.trim().is_empty() || text.trim() == "[terminal]") {
            // tool output — from the engine's own conversation record (pi: toolResult, Gemini:
            // toolCalls[].resultDisplay); the record appears a little after the completed status, wait up to 3 s
            let (me, tid, status) = (Arc::clone(self), tid.to_string(), status.to_string());
            tokio::spawn(async move {
                let mut got: Option<(String, bool)> = None;
                for _ in 0..15 {
                    let (sid, skip) = { let st = me.acp.lock().unwrap(); (st.session_id.clone(), st.pi_tool_idx) };
                    if agent == "gemini" {
                        if let Some(r) = gemini_tool_output(sid.as_deref(), &tid) { got = Some(r); break; }
                    } else if let Some((t, e, n)) = pi_tool_output(sid.as_deref(), skip) { me.acp.lock().unwrap().pi_tool_idx = n; got = Some((t, e)); break; }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                let (t, e) = got.unwrap_or_default();
                me.emit(json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": tid, "content": t.chars().take(20000).collect::<String>(), "is_error": e || status == "failed" }] }));
            });
            return;
        }
        self.emit(json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": tid, "content": text.chars().take(20000).collect::<String>(), "is_error": status == "failed" }] }));
    }

    async fn acp_on_request(self: &Arc<Self>, method: &str, rid: Value, params: Value) {
        match method {
            "session/request_permission" => {
                let raw = match &rid { Value::Number(n) => n.to_string(), Value::String(s) => s.clone(), _ => "0".into() };
                let key = format!("acp{}-{raw}", self.acp.lock().unwrap().epoch);
                let tc = params.get("toolCall").cloned().unwrap_or(Value::Null);
                let tid = tc.get("toolCallId").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let title = tc.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let rawi = tc.get("rawInput").cloned().unwrap_or(Value::Null);
                let name = self.acp.lock().unwrap().tools.get(&tid).map(|t| t.0.clone()).unwrap_or_else(|| tool_name(tc.get("kind").and_then(|k| k.as_str()), &title, &rawi));
                let mut input = if rawi.is_object() { rawi.clone() } else { json!({}) };
                if input.get("title").is_none() && !title.is_empty() { input["title"] = json!(title); }
                if name == "Bash" && input.get("command").is_none() && !title.is_empty() { input["command"] = json!(title); }
                let detail = content_text(&tc.get("content").cloned().unwrap_or(json!([])));
                if !detail.trim().is_empty() { input["detail"] = json!(detail.chars().take(4000).collect::<String>()); }
                let options = params.get("options").cloned().unwrap_or(json!([]));
                let ev = self.emit(json!({ "kind": "permission_request", "request_id": key, "tool_name": name, "input": input, "question": false, "provider": "acp", "options": options, "tool_use_id": tid, "title": title }));
                let seq = ev.get("seq").and_then(|s| s.as_u64()).unwrap_or(0);
                self.pending.lock().unwrap().insert(key.clone(), Pending { request_id: key, tool_name: name, input: json!({ "options": options }), seq, kind: "acp".into(), suggestions: None, rpc_id: Some(rid) });
                self.set_status(Status::Waiting);
            }
            other => {
                // we do not delegate files and terminal (declared in initialize) — the agent works with
                // the disk itself
                let _ = self.write(&json!({ "jsonrpc": "2.0", "id": rid, "error": { "code": -32601, "message": format!("agentd: {other} не поддерживается") } })).await;
            }
        }
    }

    /// Answer to a permission request: the agent's chosen option or
    /// cancel.
    pub(crate) async fn acp_answer(&self, p: &Pending, rid: &Value, behavior: &str, remember: bool) -> Result<()> {
        let options = p.input.get("options").cloned().unwrap_or(json!([]));
        let result = match pick_option(&options, behavior, remember) {
            Some(opt) => json!({ "outcome": { "outcome": "selected", "optionId": opt } }),
            None => json!({ "outcome": { "outcome": "cancelled" } }),
        };
        self.write(&json!({ "jsonrpc": "2.0", "id": rid, "result": result })).await
    }

    pub(crate) async fn acp_interrupt(&self) -> Result<Value> {
        let sid = self.acp.lock().unwrap().session_id.clone().ok_or_else(|| anyhow!("ACP-сессия ещё не создана"))?;
        self.acp_notify("session/cancel", json!({ "sessionId": sid })).await?;
        Ok(json!({ "ok": true }))
    }

    /// Changing the agent's model: `session/set_config_option` (configId `model`) if it offered such an option, otherwise
    /// `session/set_model`.
    async fn acp_apply_model(&self, sid: &str, model: &str) -> Result<String> {
        let (value, via_config) = { let st = self.acp.lock().unwrap(); (acp_model_value(&st, model), st.config_options.as_array().map(|a| a.iter().any(|o| o.get("id").and_then(|i| i.as_str()) == Some("model"))).unwrap_or(false)) };
        if via_config {
            let r = self.acp_call("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": value }), 60).await?;
            if let Some(c) = r.get("configOptions").filter(|c| c.is_array()) { self.acp.lock().unwrap().config_options = c.clone(); }
        } else {
            self.acp_call("session/set_model", json!({ "sessionId": sid, "modelId": value }), 60).await?;
        }
        let mut st = self.acp.lock().unwrap(); st.model = Some(value.clone()); st.models["currentModelId"] = json!(value); drop(st); self.acp_persist();
        Ok(value)
    }

    /// Mode: session/set_mode if the agent knows it (translating our names); otherwise — at the next start.
    pub(crate) async fn acp_set_mode(&self, mode: &str) -> Result<Value> {
        let via_config = self.acp.lock().unwrap().mode_via_config;
        let (sid, available, engine) = { let st = self.acp.lock().unwrap(); (st.session_id.clone(), st.modes.get("availableModes").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect::<Vec<_>>()).unwrap_or_default(), self.meta.lock().unwrap().agent.clone()) };
        let Some(sid) = sid else { return Ok(json!({ "ok": true, "applied": "next_start" })) };
        match map_mode(&engine, mode, &available) {
            Some(target) => {
                if via_config { let r = self.acp_call("session/set_config_option", json!({ "sessionId": sid, "configId": "mode", "value": target }), 60).await?; if let Some(c) = r.get("configOptions").filter(|c| c.is_array()) { self.acp.lock().unwrap().config_options = c.clone(); } }
                else { self.acp_call("session/set_mode", json!({ "sessionId": sid, "modeId": target }), 60).await?; }
                self.acp.lock().unwrap().modes["currentModeId"] = json!(target); self.acp_persist(); Ok(json!({ "ok": true, "mode": target }))
            }
            None => Ok(json!({ "ok": true, "applied": "next_start", "note": "агент не знает такого режима" })),
        }
    }

    /// Model: session/set_model if the agent returned a list; otherwise — at the next start (Gemini takes it as a
    /// flag).
    pub(crate) async fn acp_set_model(&self, model: Option<&str>) -> Result<Value> {
        let (sid, has) = { let st = self.acp.lock().unwrap(); (st.session_id.clone(), acp_model_switchable(&st)) };
        match (sid, has, model) {
            (Some(sid), true, Some(m)) => { let v = self.acp_apply_model(&sid, m).await?; Ok(json!({ "ok": true, "applied": "live", "model": v })) }
            _ => Ok(json!({ "ok": true, "applied": "next_start" })),
        }
    }

    /// The ACP engine part of caps: models (the agent's or the gate's), modes with labels,
    /// commands, context.
    pub(crate) fn acp_caps(&self, meta: &crate::session::Meta, gate_models: &[String]) -> Value {
        let mut st = self.acp.lock().unwrap();
        if st.models.is_null() && st.modes.is_null() {
            if let Some(v) = std::fs::read(self.dir().join("acp.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                st.modes = v.get("modes").cloned().unwrap_or(Value::Null); st.models = v.get("models").cloned().unwrap_or(Value::Null);
                st.commands = v.get("commands").and_then(|c| c.as_array()).cloned().unwrap_or_default(); st.config_options = v.get("config_options").cloned().unwrap_or(Value::Null);
                if st.model.is_none() { st.model = v.get("model").and_then(|m| m.as_str()).map(str::to_string); }
                if st.modes.is_null() { if let Some(m) = modes_from_config(&st.config_options) { st.modes = m; st.mode_via_config = true; } }
            }
        }
        // the model only in configOptions (omp): current — currentValue, list — options
        let cfg_model = st.config_options.as_array().and_then(|a| a.iter().find(|o| o.get("id").and_then(|i| i.as_str()) == Some("model")).cloned());
        if st.model.is_none() { if let Some(cur) = cfg_model.as_ref().and_then(|o| o.get("currentValue")).and_then(|v| v.as_str()) { st.model = Some(cur.to_string()); } }
        let current = st.model.clone().or(meta.model.clone());
        let mut models: Vec<Value> = st.models.get("availableModels").and_then(|a| a.as_array()).cloned().unwrap_or_default().iter().filter_map(|m| {
            let id = m.get("modelId").and_then(|i| i.as_str())?;
            Some(json!({ "value": id, "label": m.get("name").and_then(|n| n.as_str()).unwrap_or(id), "description": m.get("description"), "default": current.as_deref() == Some(id) }))
        }).collect();
        if models.is_empty() { if let Some(opts) = cfg_model.as_ref().and_then(|o| o.get("options")).and_then(|o| o.as_array()) {
            models = opts.iter().filter_map(|m| { let id = m.get("value").and_then(|i| i.as_str())?; Some(json!({ "value": id, "label": m.get("name").and_then(|n| n.as_str()).unwrap_or(id), "description": m.get("description"), "default": current.as_deref() == Some(id) })) }).collect();
        } }
        // pi through the gate: of all pi providers show only ours (`agentverse-gate/…`), the label without the
        // prefix
        if models.iter().any(|m| m.get("value").and_then(|v| v.as_str()).map(|v| v.starts_with("agentverse-gate/")).unwrap_or(false)) {
            models.retain(|m| m.get("value").and_then(|v| v.as_str()).map(|v| v.starts_with("agentverse-gate/")).unwrap_or(false));
            for m in models.iter_mut() { if let Some(v) = m.get("value").and_then(|v| v.as_str()).map(str::to_string) { m["label"] = json!(v.trim_start_matches("agentverse-gate/")); } }
            // the gate catalog order is random — families by rank (Claude, GPT, Gemini, the rest, Devin), by name within
            let rank = |m: &Value| { let id = m.get("label").and_then(|l| l.as_str()).unwrap_or(""); (if id.starts_with("devin/") { 4 } else if id.starts_with("claude-") { 0 } else if id.starts_with("gpt-") { 1 } else if id.starts_with("gemini-") { 2 } else { 3 }, id.to_string()) };
            models.sort_by_key(rank);
        }
        // Gemini through the gate: the CLI's own list (auto, 2.5-pro, 3.8-flash…) is not served by the gate — show the Gemini models
        // from the gate catalog
        if meta.agent == "gemini" && !gate_models.is_empty() {
            let mut ids = gemini_gate_models(gate_models);
            if let Some(c) = &current { if !ids.contains(c) { ids.insert(0, c.clone()); } }
            models = ids.into_iter().map(|id| json!({ "value": id, "label": id, "default": current.as_deref() == Some(id.as_str()) })).collect();
        }
        let modes: Vec<Value> = st.modes.get("availableModes").and_then(|a| a.as_array()).cloned().unwrap_or_default();
        let commands: Vec<Value> = st.commands.iter().filter_map(|c| { let name = c.get("name").and_then(|n| n.as_str())?; Some(json!({ "name": name, "description": c.get("description"), "argumentHint": c.pointer("/input/hint") })) }).collect();
        let usage = st.usage.clone();
        json!({
            "models": models, "mode_options": modes, "modes": modes.iter().filter_map(|m| m.get("id").cloned()).collect::<Vec<_>>(), "current_mode": st.modes.get("currentModeId"),
            "commands": commands, "config_options": st.config_options, "acp_usage": usage,
            "context_window": usage.as_ref().and_then(|u| u.get("size")).and_then(|s| s.as_u64()),
            "auth_methods": st.auth_methods, "model": current,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn omp_models_yaml_keeps_other_providers() {
        let existing = "providers:\n  mine:\n    baseUrl: http://x/v1\n    api: openai-completions\n    apiKey: k\n    models:\n      - id: m1\n        name: m1\nother: 7\n";
        let prov = json!({ "baseUrl": "http://llm.gate/v1", "api": "openai-completions", "apiKey": "$OPENAI_API_KEY", "models": [{ "id": "a", "name": "a", "contextWindow": 200000, "maxTokens": 32000 }] });
        let y = omp_models_yaml(existing, prov.clone()).unwrap();
        let back: Value = serde_yaml_ng::from_str(&y).unwrap();
        assert_eq!(back["providers"]["mine"]["baseUrl"], "http://x/v1"); assert_eq!(back["other"], 7);
        assert_eq!(back["providers"]["agentverse-gate"]["apiKey"], "$OPENAI_API_KEY"); assert_eq!(back["providers"]["agentverse-gate"]["models"][0]["id"], "a");
        assert!(omp_models_yaml(&y, prov).is_none(), "без изменений — не переписываем");
        assert!(omp_models_yaml("", json!({ "baseUrl": "b" })).unwrap().contains("agentverse-gate"));
    }
    #[test]
    fn modes_from_config_options() {
        let cfg = json!([{ "id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "default", "options": [{ "value": "default", "name": "Default", "description": "Standard" }, { "value": "plan", "name": "Plan", "description": "Read-only" }] }, { "id": "model", "options": [] }]);
        let m = modes_from_config(&cfg).unwrap();
        assert_eq!(m["currentModeId"], "default"); assert_eq!(m["availableModes"][1]["id"], "plan"); assert_eq!(m["availableModes"][1]["name"], "Plan");
        assert!(modes_from_config(&json!([{ "id": "model" }])).is_none() && modes_from_config(&Value::Null).is_none());
    }
    #[test]
    fn mapping() {
        assert_eq!(tool_name(Some("execute"), "ls", &json!({})), "Bash");
        assert_eq!(tool_name(None, "run_shell_command", &json!({})), "Bash");
        assert_eq!(tool_name(None, "Edit file", &json!({})), "Edit");
        assert_eq!(tool_name(Some("fetch"), "", &json!({})), "WebFetch");
        assert_eq!(tool_name(None, "Mystery", &json!({ "command": "x" })), "Bash");
        assert_eq!(content_text(&json!([{ "type": "content", "content": { "type": "text", "text": "hi" } }, { "type": "diff", "path": "/a", "oldText": "x", "newText": "y" }])), "hi\n--- /a\n- x\n+ y\n");
        assert_eq!(antigravity_args(Some("gemini-3.5-pro"), Some("high"), Some("default"), false), vec!["-m", "gemini-3.5-pro", "-e", "high", "--no-skip-permissions"]);
        assert_eq!(antigravity_args(Some("gemini-3.8-flash-high"), Some("low"), Some("auto"), true), vec!["-m", "gemini-api://gemini-3.8-flash-high", "-e", "low"], "через gate — id модели как есть под gemini-api://");
        assert_eq!(antigravity_model("gemini-api://x", true), "gemini-api://x"); assert_eq!(antigravity_model("gemini-3-flash", false), "gemini-3-flash");
        let mut genv = HashMap::new(); genv.insert("GEMINI_API_KEY".to_string(), "k".to_string());
        assert!(!antigravity_gate(&genv), "без адреса gate режим ключа не включаем");
        genv.insert("GEMINI_BASE_URL".to_string(), "http://llm.gate".to_string());
        assert!(antigravity_gate(&genv) && antigravity_env(&genv) == vec![("GOOGLE_GEMINI_BASE_URL".to_string(), "http://llm.gate".to_string())]);
        assert_eq!(antigravity_args(None, Some("xhigh"), Some("bypassPermissions"), true), Vec::<String>::new(), "xhigh адаптер не знает, в auto разрешения не спрашиваем");
        assert_eq!(antigravity_args(None, None, Some("plan"), false), vec!["--no-skip-permissions"]);
        let g = ["default".to_string(), "autoEdit".into(), "yolo".into()];
        assert_eq!(map_mode("gemini", "acceptEdits", &g).as_deref(), Some("autoEdit"));
        assert_eq!(map_mode("gemini", "auto_edit", &g).as_deref(), Some("autoEdit"));
        assert_eq!(map_mode("gemini", "auto-edit", &[]).as_deref(), Some("autoEdit"));
        assert_eq!(map_mode("gemini", "bypassPermissions", &g).as_deref(), Some("yolo"));
        assert_eq!(map_mode("gemini", "plan", &g), None);
        assert_eq!(map_mode("gemini", "yolo", &g).as_deref(), Some("yolo"));
        assert_eq!(map_mode("pi", "default", &["low".to_string()]), None);
        let mut st = AcpState::default();
        st.config_options = json!([{ "id": "model", "options": [{ "value": "anthropic/claude-sonnet-4-6" }, { "value": "agentverse-gate/claude-sonnet-4-6" }, { "value": "agentverse-gate/gpt-5.5" }] }]);
        assert_eq!(acp_model_value(&st, "claude-sonnet-4-6"), "agentverse-gate/claude-sonnet-4-6");
        assert_eq!(acp_model_value(&st, "openai/gpt-5.5"), "openai/gpt-5.5");
        assert_eq!(acp_model_value(&st, "kimi-k2"), "kimi-k2");
        assert!(acp_model_switchable(&st));
        assert!(!acp_model_switchable(&AcpState::default()));
        assert_eq!(map_mode("pi", "low", &["low".to_string()]).as_deref(), Some("low"));
        let opts = json!([{ "optionId": "proceed_once", "kind": "allow_once" }, { "optionId": "proceed_always", "kind": "allow_always" }, { "optionId": "cancel", "kind": "reject_once" }]);
        assert_eq!(pick_option(&opts, "allow", false).as_deref(), Some("proceed_once"));
        assert_eq!(pick_option(&opts, "allow", true).as_deref(), Some("proceed_always"));
        assert_eq!(pick_option(&opts, "deny", true).as_deref(), Some("cancel"));
        assert_eq!(pick_option(&json!([]), "deny", false), None);
        let ids = ["gemini-3-flash".to_string(), "gemini-3.1-pro-low".into(), "gemini-3.1-flash-image".into(), "devin/gemini-3-1-pro".into(), "gemini-pro-agent".into(), "gemini-3.8-flash-high".into(), "gemini-3.5-flash-lite".into()];
        assert_eq!(pick_gemini_model(&ids).as_deref(), Some("gemini-3.8-flash-high"));
        assert_eq!(gemini_gate_models(&ids), vec!["gemini-3.8-flash-high".to_string(), "gemini-3.1-pro-low".into(), "gemini-pro-agent".into()]);
        assert_eq!(pick_gemini_model(&["gemini-3-flash".to_string(), "gemini-3.5-flash-lite".into()]), None);
        let pi: Vec<String> = ["claude-sonnet-4-20250514", "claude-opus-5-5", "claude-sonnet-4-6", "claude-sonnet-5-5", "gpt-5.5"].iter().map(|s| s.to_string()).collect();
        assert_eq!(pick_pi_model(&pi).as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(pick_pi_model(&["gpt-5.5".to_string(), "gpt-5.6-terra".into(), "kimi-k2".into()]).as_deref(), Some("gpt-5.6-terra"));
        assert_eq!(pick_pi_model(&["kimi-k2".to_string()]).as_deref(), Some("kimi-k2"));
    }
}
