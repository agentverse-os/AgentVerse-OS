//! Agent sessions (ADR-006). The Claude provider is the native stream-json of the installed `claude`: the process lives
//! in agentd, user messages are written to stdin, events are read from stdout line by line, tool permissions arrive as
//! `control_request` (`can_use_tool`) and wait for an answer from any device. The session journal is append-only JSONL
//! on the home volume (`~/.cloudos/agentd/sessions/<id>/events.jsonl` + `meta.json`), served to clients in pages and as
//! a live stream. Every `result` yields a usage record (tokens, cost, model) — cloudd keeps the expense ledger from it.
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{broadcast, oneshot, watch};

const EVENTS_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Starting,
    Idle,
    Working,
    /// Waiting for a human: a tool permission or a question from the
    /// agent.
    Waiting,
    Exited,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub agent: String,
    pub cwd: String,
    pub title: String,
    /// Owner's own name (docs/agents-ux.md §4.8): shown instead of `title`, while the auto-title keeps living on
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Subscription limit hit (docs/agents-ux.md §9.7): `{window, resets_at, at, source, text}`; cleared by a successful turn or a
    /// reassignment
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_hit: Option<Value>,
    pub created_at: u64,
    /// Session id at the provider (for Claude it equals ours on creation; on resume — the one that was resumed).
    pub provider_session_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// Account (ADR-006, decision 6): the session is pinned to it at start; variables and `unset` are applied on top of the
    /// login shell, i.e. on top of the gate's env.sh — so the account's subscription or key beats the project route.
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub unset: Vec<String>,
    /// Conversation picked up from disk (a Claude Code journal / Codex rollout started outside agentd
    /// — import.rs)
    #[serde(default)]
    pub imported: bool,
    /// Provider journal path (for Codex — the rollout found; for Claude — computed from the config, cwd
    /// and id)
    #[serde(default)]
    pub transcript: Option<String>,
    /// How many provider journal lines are already reflected in our journal (None — never
    /// reconciled yet)
    #[serde(default)]
    pub synced_lines: Option<u64>,
    /// Conversation usage before pickup (from the provider journal: Claude `cost-state`, Codex tokens) — the base
    /// on top of which agentd turns are counted; not written to the expense ledger
    #[serde(default)]
    pub usage_base: Option<Usage>,
    /// Version of the import.rs parser that built the journal of a picked-up conversation (None — an
    /// old record)
    #[serde(default)]
    pub import_v: Option<u32>,
    /// Claude: own thinking budget (control `set_max_thinking_tokens`, MAX_THINKING_TOKENS at start)
    #[serde(default)]
    pub max_thinking_tokens: Option<u64>,
    /// Codex: service tier of the next turns (`fast` — fast mode, `priority`); `serviceTier` in turn/start
    #[serde(default)]
    pub service_tier: Option<String>,
    /// Claude: prompt cache TTL `5m` | `1h` (control `apply_flag_settings {promptCacheTtl}`,
    /// CLAUDE_CODE_PROMPT_CACHE_TTL at start); None — whatever the CLI decides (1h for subscribers, 5m
    /// for key/gate)
    #[serde(default)]
    pub cache_ttl: Option<String>,
    /// Model effort: Claude `low…max` (`--effort` at start, `apply_flag_settings` on a live process),
    /// Codex `low…xhigh` (`effort` in `turn/start`)
    #[serde(default)]
    pub effort: Option<String>,
    /// Claude: per-session tool rules — `--allowedTools` / `--disallowedTools` at start (for agents from the "Agents"
    /// section they make the "ask" mode usable for autonomous work)
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// Claude: continue from this answer uuid at the next start (`--resume-session-at`, message edit); cleared after start
    #[serde(default)]
    pub resume_at: Option<String>,
    /// Context window of the gate model (max_input_tokens from the proxy's `/v1/models`): for non-`claude-*` ids Claude Code
    /// takes it from CLAUDE_CODE_MAX_CONTEXT_TOKENS at start (otherwise treats it as "unknown model" = 200k); for claude-* only
    /// the `[1m]` tag works
    #[serde(default)]
    pub context_window: Option<u64>,
}

impl Meta {
    /// For clients: account variable values (keys, tokens) are not exposed, only names.
    pub fn public(&self) -> Meta {
        let mut m = self.clone();
        m.env = m.env.keys().map(|k| (k.clone(), "•••".to_string())).collect();
        m
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_create: u64,
    pub cost_usd: f64,
    pub turns: u64,
}

/// Claude families with a native 1M window (Claude Code 2.1.284 catalog: `context:{window:1e6,native_1m:!0}`): fable-5, opus-5,
/// sonnet-5. Claude Code believes in 1M only with a first-party ANTHROPIC_BASE_URL or with the `[1m]` tag in the id; through
/// the gate (`http://llm.gate`) a bare id is cut to 200k (autocompact at 167k) — "context 100%" with a live window. The tag is
/// purely client-side: the same id goes over the wire, the price is the same (verified on the stand), so we normalize the id at
/// start and in the catalog.
pub fn normalize_claude_model(id: &str) -> String {
    let lower = id.trim().to_ascii_lowercase();
    if lower.contains("[1m]") { return id.trim().to_string(); }
    const NATIVE_1M: [&str; 3] = ["claude-fable-5", "claude-opus-5", "claude-sonnet-5"];
    if NATIVE_1M.iter().any(|p| lower.starts_with(p)) { format!("{}[1m]", id.trim()) } else { id.trim().to_string() }
}

/// Does the CLI's own catalog (`initialize.models`) have a 1M-window variant for the family of model `id` (opus / sonnet /
/// fable): with a subscription Claude Code lists Opus and Sonnet as 200k without a "1M context" variant, through the API
/// (gate, key) — with one.
pub fn claude_1m_listed(models: &[Value], id: &str) -> bool {
    let lower = id.trim().to_ascii_lowercase();
    let Some(kw) = ["fable", "opus", "sonnet", "haiku"].iter().find(|k| lower.contains(*k)).copied() else { return false };
    models.iter().any(|m| {
        let f = |k: &str| m.get(k).and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase();
        let (v, r, d) = (f("value"), f("resolved"), f("description"));
        (v.contains("[1m]") || r.contains("[1m]") || d.contains("1m context")) && (v.contains(kw) || r.contains(kw) || d.contains(kw))
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Info {
    #[serde(flatten)]
    pub meta: Meta,
    pub status: Status,
    pub alive: bool,
    pub last_seq: u64,
    pub pending: usize,
    pub usage: Usage,
    pub last_text: Option<String>,
    /// Last activity (seconds): last journal event
    pub updated_at: u64,
    /// Terminal mode: id of the PTY with the agent's native TUI on this conversation (the headless process
    /// is stopped meanwhile)
    #[serde(default)]
    pub tui: Option<String>,
    /// Background tasks after a turn (background Bash, Agent, Workflow…): how many are
    /// running and which
    pub background: usize,
    pub tasks: Vec<Value>,
    /// Queued messages (they go out by themselves after the
    /// current turn)
    pub queued: Vec<Value>,
}

#[derive(Debug, Deserialize, Default)]
pub struct CreateOpts {
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub cache_ttl: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    /// Resume the provider session with this id (our record with the same id continues the journal if
    /// it exists).
    pub resume: Option<String>,
    pub model: Option<String>,
    /// Model context window (tokens) from the service catalog: Codex gets it as `model_context_window` — its own table
    /// does not know every model (GPT-6 Astra → 272k by default with a real ~1M)
    #[serde(default)]
    pub context_window: Option<u64>,
    pub permission_mode: Option<String>,
    /// Account and its environment (set by cloudd); on resume with a new account they replace the
    /// saved ones.
    pub account: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub unset: Vec<String>,
    /// On resume: replace the saved account and environment with the ones in the request (even empty — "no
    /// account").
    #[serde(default)]
    pub reassign: bool,
    /// On resume after a limit hit (§9.7): repeat the user's last request — the rejected turn was never executed
    #[serde(default)]
    pub retry_last: bool,
    /// Note into the conversation on reassignment ("account: A → B,
    /// limit …")
    #[serde(default)]
    pub note: Option<String>,
    /// Claude: session tool rules (`Bash(npm test:*)`, `Read`, …) — the --allowedTools / --disallowedTools flags
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
}

struct Journal {
    seq: u64,
    events: Vec<Value>,
    file: std::fs::File,
}

impl Journal {
    fn open(dir: &PathBuf) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("events.jsonl");
        let mut events = Vec::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            events.extend(text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()));
        }
        let seq = events.last().and_then(|e| e.get("seq").and_then(|s| s.as_u64())).unwrap_or(0);
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { seq, events, file })
    }

    /// Start the journal over (rebuilding a picked-up conversation with a new
    /// parser).
    fn reset(&mut self, dir: &PathBuf) -> Result<()> {
        self.file = std::fs::OpenOptions::new().create(true).write(true).truncate(true).append(false).open(dir.join("events.jsonl"))?;
        self.events.clear();
        self.seq = 0;
        Ok(())
    }

    /// Keep events up to and including `keep_seq` (message edit); numbering continues, the file is rewritten.
    fn truncate(&mut self, dir: &PathBuf, keep_seq: u64) -> Result<()> {
        self.events.retain(|e| e.get("seq").and_then(|s| s.as_u64()).unwrap_or(0) <= keep_seq);
        let path = dir.join("events.jsonl");
        let body: String = self.events.iter().map(|e| e.to_string() + "\n").collect();
        std::fs::write(&path, body)?;
        self.file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(())
    }

    fn append(&mut self, mut ev: Value) -> Value {
        self.seq += 1;
        ev["seq"] = json!(self.seq);
        // events picked up from the provider journal come with their own time
        if ev.get("ts").and_then(|t| t.as_u64()).is_none() { ev["ts"] = json!(now_ms()); }
        let _ = writeln!(self.file, "{ev}");
        self.events.push(ev.clone());
        ev
    }
}

/// Pending permission request (`can_use_tool`) — kept until a human answers.
#[derive(Debug, Clone, Serialize)]
pub struct Pending {
    pub request_id: String,
    pub tool_name: String,
    pub input: Value,
    pub seq: u64,
    /// `claude` — a Claude control_request; `codex_command` | `codex_file` | `codex_perms` | `codex_question` — app-server
    /// requests.
    #[serde(default)]
    pub kind: String,
    /// Claude Code rule suggestions (`permission_suggestions`): "remember" writes updatedPermissions from
    /// them
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestions: Option<Value>,
    #[serde(skip)]
    pub rpc_id: Option<Value>,
}

/// Live answer text buffer (slice 39): chunks accumulate for ~60 ms and go to connected clients as one `delta` message.
#[derive(Default)]
struct LiveBuf { buf: String, last_ms: u64, gen: u64 }

pub struct Session {
    pub meta: Mutex<Meta>,
    dir: PathBuf,
    status: watch::Sender<Status>,
    journal: Mutex<Journal>,
    events: broadcast::Sender<Value>,
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    pid: Mutex<Option<u32>>,
    pub(crate) pending: Mutex<HashMap<String, Pending>>,
    ctrl_seq: AtomicU64,
    ctrl_waiters: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    usage: Mutex<Usage>,
    last_text: Mutex<Option<String>>,
    /// Model of the last answer in the turn (assistant.message.model): in `result` for the expense ledger — model_usage is
    /// cumulative
    last_model: Mutex<Option<String>>,
    /// Workspace-wide usage journal (`../usage.jsonl`): survives session deletion — the core's expense ledger reads it.
    usage_log: PathBuf,
    /// Codex provider state (codex.rs); unused for Claude.
    pub(crate) codex: Mutex<crate::codex::CodexState>,
    /// ACP engine state (acp.rs: Gemini CLI, pi, …); unused for Claude and Codex.
    pub(crate) acp: Mutex<crate::acp::AcpState>,
    /// Antigravity CLI state (agy.rs: headless stream-json); unused for the others.
    pub(crate) agy: Mutex<crate::agy::AgyState>,
    /// Size and mtime of the provider journal at the last reconciliation — to avoid reading an
    /// unchanged file.
    transcript_seen: Mutex<(u64, u64)>,
    /// Claude Code background tasks (`task_started` … `task_notification`): background Bash, Agent, Workflow, Monitor
    /// etc. live on after the answer while stdin is open
    tasks: Mutex<HashMap<String, Value>>,
    /// Message queue: a send during a turn lands here, after the answer the next one goes out by
    /// itself
    queue: Mutex<Vec<Value>>,
    /// Last cumulative `total_cost_usd` of the session (Claude Code counts it for the whole conversation, including past
    /// processes and turns before pickup): the turn cost is the difference from the previous one, otherwise the sums
    /// inflate several times over
    last_total_cost: Mutex<f64>,
    /// Live answer text (slice 39): no journal and no seq, only over the session
    /// WebSocket
    live: Mutex<LiveBuf>,
}

impl Session {
    fn load(dir: PathBuf, mut meta: Meta) -> Result<Arc<Self>> {
        // the `<synthetic>` model got into records picked up before the filter in
        // import.rs
        if meta.model.as_deref().map(|m| m.starts_with('<')).unwrap_or(false) { meta.model = None; }
        let usage_log = dir.parent().map(|p| p.join("usage.jsonl")).unwrap_or_else(|| dir.join("usage.jsonl"));
        let journal = Journal::open(&dir)?;
        let (events, _) = broadcast::channel(EVENTS_CAPACITY);
        let mut usage = meta.usage_base.clone().unwrap_or_default();
        let mut last_total = meta.usage_base.as_ref().map(|b| b.cost_usd).unwrap_or(0.0);
        let mut last_text = None;
        for ev in &journal.events {
            if ev.get("kind").and_then(|k| k.as_str()) == Some("result") {
                let delta = cost_delta(&mut last_total, ev);
                add_usage(&mut usage, ev, delta);
                if let Some(t) = ev.get("text").and_then(|t| t.as_str()) {
                    last_text = Some(t.chars().take(200).collect());
                }
            }
        }
        Ok(Arc::new(Self {
            meta: Mutex::new(meta),
            dir,
            status: watch::channel(Status::Exited).0,
            journal: Mutex::new(journal),
            events,
            stdin: tokio::sync::Mutex::new(None),
            child: Mutex::new(None),
            pid: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            ctrl_seq: AtomicU64::new(1),
            ctrl_waiters: Mutex::new(HashMap::new()),
            usage: Mutex::new(usage),
            last_text: Mutex::new(last_text),
            last_model: Mutex::new(None),
            usage_log,
            codex: Mutex::new(Default::default()),
            acp: Mutex::new(Default::default()),
            agy: Mutex::new(Default::default()),
            transcript_seen: Mutex::new((0, 0)),
            tasks: Mutex::new(HashMap::new()),
            last_total_cost: Mutex::new(last_total),
            live: Mutex::new(LiveBuf::default()),
            queue: Mutex::new(Vec::new()),
        }))
    }

    pub fn id(&self) -> String {
        self.meta.lock().unwrap().id.clone()
    }

    pub(crate) fn dir(&self) -> &PathBuf {
        &self.dir
    }

    // ---- shared primitives for providers ----
    pub(crate) fn next_ctrl_id(&self) -> String {
        self.ctrl_seq.fetch_add(1, Ordering::Relaxed).to_string()
    }

    pub(crate) fn ctrl_wait(&self, id: &str) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        self.ctrl_waiters.lock().unwrap().insert(id.to_string(), tx);
        rx
    }

    pub(crate) fn ctrl_forget(&self, id: &str) {
        self.ctrl_waiters.lock().unwrap().remove(id);
    }

    pub(crate) fn ctrl_resolve(&self, id: &str, v: Value) {
        if let Some(tx) = self.ctrl_waiters.lock().unwrap().remove(id) { let _ = tx.send(v); }
    }

    /// Writing `result` to the journal, usage totals and the shared usage.jsonl — the same for every
    /// provider.
    pub(crate) fn record_result(&self, mut ev: Value) {
        let delta = cost_delta(&mut self.last_total_cost.lock().unwrap(), &ev);
        ev["cost_usd"] = json!(delta);
        add_usage(&mut self.usage.lock().unwrap(), &ev, delta);
        if let Some(t) = ev.get("text").and_then(|t| t.as_str()) { *self.last_text.lock().unwrap() = Some(t.chars().take(200).collect()); }
        // limit hit (§9.7): a turn error whose text is about the limit sets the mark, a successful
        // turn clears it
        let is_err = ev.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
        let text = ev.get("text").and_then(|t| t.as_str()).unwrap_or("");
        if is_err && is_limit_text(text) {
            if self.meta.lock().unwrap().limit_hit.is_none() { self.set_limit_hit(json!({ "source": "result", "text": text.chars().take(200).collect::<String>() })); }
        } else if !is_err && self.meta.lock().unwrap().limit_hit.is_some() {
            { self.meta.lock().unwrap().limit_hit = None; }
            self.save_meta(); crate::hub::touch();
        }
        let ev = self.emit(ev);
        self.append_usage(&ev);
    }

    /// Limit-hit mark: in meta (visible to the core via Info), a note into the
    /// conversation, the hub
    pub fn set_limit_hit(&self, mut v: Value) {
        v["at"] = json!(now_ms() / 1000);
        { let mut m = self.meta.lock().unwrap(); if m.limit_hit.is_some() { return } m.limit_hit = Some(v.clone()); }
        self.save_meta();
        let when = v.get("resets_at").and_then(|r| r.as_str()).and_then(epoch_of_rfc3339).map(|t| { let now = now_ms() / 1000; let hm = rfc3339(t); let hm = hm.get(11..16).unwrap_or(&hm).to_string(); if t > now { let m = (t - now).div_ceil(60); format!(", сброс через {} ({hm} UTC)", if m < 60 { format!("{m} мин") } else { format!("{} ч {} мин", m / 60, m % 60) }) } else { format!(" ({hm} UTC)") } }).unwrap_or_default();
        let win = v.get("window").and_then(|w| w.as_str()).map(|w| format!(" ({w})")).unwrap_or_default();
        self.emit(json!({ "kind": "system", "subtype": "agent_note", "data": { "message": format!("лимит подписки исчерпан{win}{when} — ядро переключит аккаунт, если в пуле есть свободный") } }));
        crate::hub::touch();
    }
    /// The user's last request from the journal (to repeat after a reassignment)
    pub fn last_user_content(&self) -> Option<Value> {
        let j = self.journal.lock().unwrap();
        let content = j.events.iter().rev().find(|e| e.get("kind").and_then(|k| k.as_str()) == Some("user")).and_then(|e| e.get("content").cloned())?;
        // images are stored in the journal without data — only text blocks are
        // repeated
        let blocks: Vec<Value> = content.as_array().cloned().unwrap_or_default().into_iter().filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")).collect();
        if blocks.is_empty() { None } else { Some(Value::Array(blocks)) }
    }
    /// Core note into the conversation (a grey line, like an agent_note from
    /// agentd itself)
    pub fn note(&self, message: &str) { self.emit(json!({ "kind": "system", "subtype": "agent_note", "data": { "message": message } })); }

    pub fn status(&self) -> Status {
        *self.status.borrow()
    }

    pub fn alive(&self) -> bool {
        !matches!(self.status(), Status::Exited | Status::Error)
    }

    pub fn info(&self) -> Info {
        // updated_at — as separate expressions: a second lock() of the same Mutex inside the Info literal would
        // deadlock
        let created = self.meta.lock().unwrap().created_at;
        let last_ts = self.journal.lock().unwrap().events.last().and_then(|e| e.get("ts").and_then(|t| t.as_u64())).map(|ms| ms / 1000).unwrap_or(0);
        let tasks: Vec<Value> = if self.alive() { self.tasks.lock().unwrap().values().cloned().collect() } else { Vec::new() };
        Info {
            meta: self.meta.lock().unwrap().public(),
            status: self.status(),
            alive: self.alive(),
            last_seq: self.journal.lock().unwrap().seq,
            pending: self.pending.lock().unwrap().len(),
            usage: self.usage.lock().unwrap().clone(),
            last_text: self.last_text.lock().unwrap().clone(),
            updated_at: last_ts.max(created),
            tui: None,
            background: tasks.len(),
            tasks,
            queued: self.queue.lock().unwrap().clone(),
        }
    }

    /// Journal search (slice 34): the query words — all of them, separately, case-insensitive and ё/е-insensitive — in the
    /// owner's and agent's text and in the title; up to `max` matches with a ±80-character snippet around the first word.
    pub fn search(&self, words: &[String], max: usize) -> Vec<Value> {
        let mut hits = Vec::new();
        if words.is_empty() { return hits }
        let title = self.meta.lock().unwrap().title.clone();
        if words.iter().all(|w| norm_text(&title).contains(w.as_str())) { hits.push(json!({ "seq": 0, "ts": self.meta.lock().unwrap().created_at * 1000, "role": "title", "snippet": title })); }
        let j = self.journal.lock().unwrap();
        for e in j.events.iter() {
            if hits.len() >= max { break }
            let kind = e.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            if kind != "user" && kind != "assistant" { continue }
            if e.get("parent_tool_use_id").map(|p| !p.is_null()).unwrap_or(false) { continue }
            let text = event_text(e);
            if text.trim().is_empty() { continue }
            let n = norm_text(&text);
            if !words.iter().all(|w| n.contains(w.as_str())) { continue }
            hits.push(json!({ "seq": e.get("seq"), "ts": e.get("ts"), "role": if kind == "user" { "user" } else { "agent" }, "snippet": snippet(&text, &n, &words[0]) }));
        }
        hits
    }

    /// Export the conversation as Markdown: header, then messages in order; tool calls as a line with the result in
    /// <details>.
    pub fn export_md(&self) -> String {
        let meta = self.meta.lock().unwrap().clone();
        let usage = self.usage.lock().unwrap().clone();
        let j = self.journal.lock().unwrap();
        let mut out = format!("# {}\n\n", if meta.title.trim().is_empty() { meta.id.clone() } else { meta.title.clone() });
        out.push_str(&format!("- Движок: {}{}\n- Каталог: {}\n- Создана: {} UTC\n- Ходов: {} · токены {}↑ {}↓ (кэш {} / {}) · ${:.2}\n\n---\n\n",
            crate::engines::title(&meta.agent), meta.model.as_deref().map(|m| format!(" · {m}")).unwrap_or_default(), meta.cwd, fmt_ts(meta.created_at * 1000), usage.turns, usage.input, usage.output, usage.cache_read, usage.cache_create, usage.cost_usd));
        let mut results: HashMap<String, (String, bool)> = HashMap::new();
        for e in j.events.iter() {
            if let Some(arr) = e.get("content").and_then(|c| c.as_array()) { if e.get("kind").and_then(|k| k.as_str()) == Some("tool_result") {
                for b in arr { if let (Some(id), Some(c)) = (b.get("tool_use_id").and_then(|i| i.as_str()), b.get("content")) { let txt = match c { Value::String(s) => s.clone(), Value::Array(a) => a.iter().filter_map(|x| x.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join("\n"), _ => String::new() }; results.insert(id.to_string(), (txt, b.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false))); } }
            } }
        }
        for e in j.events.iter() {
            let kind = e.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            if e.get("parent_tool_use_id").map(|p| !p.is_null()).unwrap_or(false) { continue }
            let ts = e.get("ts").and_then(|t| t.as_u64()).map(fmt_ts).unwrap_or_default();
            match kind {
                "user" => { let t = event_text(e); if !t.trim().is_empty() { out.push_str(&format!("**Владелец** · {ts}\n\n{}\n\n", t.trim())); } }
                "assistant" => {
                    for b in e.get("content").and_then(|c| c.as_array()).cloned().unwrap_or_default() {
                        match b.get("type").and_then(|t| t.as_str()) {
                            Some("text") => { let t = b.get("text").and_then(|t| t.as_str()).unwrap_or("").trim(); if !t.is_empty() { out.push_str(&format!("**Агент** · {ts}\n\n{t}\n\n")); } }
                            Some("tool_use") => {
                                let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("Tool");
                                let input = b.get("input").cloned().unwrap_or(Value::Null);
                                let what = input.get("command").or(input.get("file_path")).or(input.get("path")).or(input.get("pattern")).or(input.get("url")).or(input.get("title")).and_then(|x| x.as_str()).map(str::to_string).unwrap_or_else(|| serde_json::to_string(&input).unwrap_or_default());
                                out.push_str(&format!("▸ **{name}**: `{}`\n", what.replace('`', "'").chars().take(300).collect::<String>()));
                                if let Some((res, err)) = b.get("id").and_then(|i| i.as_str()).and_then(|i| results.get(i)) {
                                    let res_t: String = res.chars().take(4000).collect();
                                    if !res_t.trim().is_empty() { out.push_str(&format!("<details><summary>{}</summary>\n\n```\n{}\n```\n\n</details>\n\n", if *err { "ошибка" } else { "результат" }, res_t.trim_end())); } else { out.push('\n'); }
                                } else { out.push('\n'); }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Export the journal as is.
    pub fn export_json(&self) -> Value {
        let meta = self.meta.lock().unwrap().public();
        let events: Vec<Value> = self.journal.lock().unwrap().events.clone();
        json!({ "session": meta, "events": events })
    }

    /// Was the conversation alive before agentd stopped (slice 38): the last `status` in the journal is not exited/error,
    /// activity not older than a day. In terminal mode the headless process is already stopped (status exited) — those are
    /// not resumed.
    pub fn was_live(&self) -> bool {
        let j = self.journal.lock().unwrap();
        let last_status = j.events.iter().rev().find(|e| e.get("kind").and_then(|k| k.as_str()) == Some("status")).and_then(|e| e.get("status").and_then(|s| s.as_str()).map(str::to_string));
        let last_ts = j.events.last().and_then(|e| e.get("ts").and_then(|t| t.as_u64())).unwrap_or(0);
        matches!(last_status.as_deref(), Some("idle") | Some("working") | Some("waiting") | Some("starting")) && now_ms().saturating_sub(last_ts) < 24 * 3600 * 1000
    }

    pub fn events_since(&self, since: u64, limit: usize) -> Vec<Value> {
        let j = self.journal.lock().unwrap();
        j.events.iter().filter(|e| e.get("seq").and_then(|s| s.as_u64()).unwrap_or(0) > since).take(limit).cloned().collect()
    }
    /// One event by number (full, for "show whole")
    pub fn event_at(&self, seq: u64) -> Option<Value> {
        let j = self.journal.lock().unwrap();
        j.events.iter().find(|e| e.get("seq").and_then(|s| s.as_u64()) == Some(seq)).cloned()
    }

    /// Snapshot of events after `since` plus a subscription to new ones — under one lock, no
    /// gaps and no duplicates.
    pub fn attach(&self, since: u64) -> (Vec<Value>, broadcast::Receiver<Value>) {
        let j = self.journal.lock().unwrap();
        let snapshot = j.events.iter().filter(|e| e.get("seq").and_then(|s| s.as_u64()).unwrap_or(0) > since).cloned().collect();
        (snapshot, self.events.subscribe())
    }

    pub fn pending(&self) -> Vec<Pending> {
        self.pending.lock().unwrap().values().cloned().collect()
    }

    pub(crate) fn emit(&self, ev: Value) -> Value {
        let ev = self.journal.lock().unwrap().append(ev);
        let _ = self.events.send(ev.clone());
        crate::hub::touch();
        ev
    }

    /// Live text (slice 39): reset at the start of a turn and after a finished message — clients clear the live
    /// bubble and the buffer.
    pub(crate) fn live_reset(&self) {
        { let mut l = self.live.lock().unwrap(); l.buf.clear(); l.gen += 1; }
        let _ = self.events.send(json!({ "kind": "delta", "reset": true, "live": true }));
    }
    /// A chunk of answer text: send at once if ≥ 60 ms passed since the last send, otherwise accumulate and flush the
    /// tail after 80 ms.
    pub(crate) fn live_push(self: &Arc<Self>, text: &str) {
        if text.is_empty() { return }
        let (flush_now, gen) = { let mut l = self.live.lock().unwrap(); l.buf.push_str(text); let now = now_ms(); if now.saturating_sub(l.last_ms) >= 60 { l.last_ms = now; (true, l.gen) } else { (false, l.gen) } };
        if flush_now { self.live_flush(); return }
        let me = self.clone();
        tokio::spawn(async move { tokio::time::sleep(std::time::Duration::from_millis(80)).await; if me.live.lock().unwrap().gen == gen { me.live_flush(); } });
    }
    fn live_flush(&self) {
        let text = { let mut l = self.live.lock().unwrap(); l.last_ms = now_ms(); std::mem::take(&mut l.buf) };
        if !text.is_empty() { let _ = self.events.send(json!({ "kind": "delta", "text": text, "live": true })); }
    }

    /// The `[1m]` tag for families with a native 1M — only where the 1M window really exists: through the API (gate:
    /// ANTHROPIC_BASE_URL, or a key) — always (verified on the stand); with a subscription (first-party OAuth) — only if the
    /// CLI's own catalog (the workspace's `claude-init.json`) contains a 1M variant for this family (Fable — yes, Opus and
    /// Sonnet — no).
    pub(crate) fn claude_model_for(&self, id: &str) -> String {
        let n = normalize_claude_model(id);
        if n == id.trim() { return n }
        let env = crate::codex::effective_env(&self.meta.lock().unwrap());
        if env.contains_key("ANTHROPIC_BASE_URL") || env.contains_key("ANTHROPIC_API_KEY") || env.contains_key("ANTHROPIC_AUTH_TOKEN") { return n }
        let listed: Vec<Value> = self.dir.parent().and_then(|p| std::fs::read(p.join("claude-init.json")).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|r| r.get("models").and_then(|m| m.as_array()).cloned()).unwrap_or_default();
        if claude_1m_listed(&listed, id) { n } else { id.trim().to_string() }
    }

    pub(crate) fn set_status(&self, s: Status) {
        if self.status.send_replace(s) != s {
            self.emit(json!({ "kind": "status", "status": s }));
        }
    }

    pub(crate) fn save_meta(&self) {
        let m = self.meta.lock().unwrap().clone();
        let _ = std::fs::write(self.dir.join("meta.json"), serde_json::to_vec_pretty(&m).unwrap_or_default());
    }

    pub(crate) async fn write(&self, v: &Value) -> Result<()> {
        let mut g = self.stdin.lock().await;
        let stdin = g.as_mut().ok_or_else(|| anyhow!("процесс агента не запущен"))?;
        let mut line = serde_json::to_vec(v)?;
        line.push(b'\n');
        stdin.write_all(&line).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Start the provider process. Claude: an interactive login shell (PATH to the CLI and the gate env from profile.d and
    /// bash.bashrc), inside it `claude -p` with stream-json both ways and permissions over stdio.
    async fn spawn(self: &Arc<Self>, resume: bool) -> Result<()> {
        // a turn from the TUI or terminal between our processes — into the journal before start, otherwise
        // the mark would cover it
        if resume { let n = self.sync_transcript(); if n > 0 { tracing::info!("session {}: из журнала провайдера подхвачено {n} событий", &self.id()[..8]); } }
        let meta = self.meta.lock().unwrap().clone();
        let is_codex = meta.agent == "codex";
        let is_acp = crate::engines::is_acp(&meta.agent);
        let is_agy = crate::engines::is_agy(&meta.agent);
        if meta.agent != "claude" && !is_codex && !is_acp && !is_agy {
            bail!("агент {} пока не поддерживается (есть: {})", meta.agent, crate::engines::ENGINES.iter().map(|e| e.id).collect::<Vec<_>>().join(", "));
        }
        // ACP engine: command from the catalog; Gemini takes the model as a flag (through the gate — the model from the proxy catalog unless the session
        // set its own), into the environment — GOOGLE_GEMINI_BASE_URL from the gate's GEMINI_BASE_URL (the CLI's own gateway mode) and NO_BROWSER; pi gets
        // the gate provider in models.json
        let mut acp_env: Vec<(String, String)> = Vec::new();
        let mut acp_args: Vec<String> = Vec::new();
        if is_acp {
            let engine = crate::engines::get(&meta.agent).unwrap();
            acp_args = engine.acp.iter().map(|s| s.to_string()).collect();
            let env = crate::codex::effective_env(&meta);
            if engine.id == "gemini" {
                if !env.contains_key("GOOGLE_GEMINI_BASE_URL") { if let Some(b) = env.get("GEMINI_BASE_URL") { acp_env.push(("GOOGLE_GEMINI_BASE_URL".into(), b.clone())); } }
                acp_env.push(("NO_BROWSER".into(), "true".into()));
                crate::acp::gemini_ensure_trust(&meta.cwd);
                let mut model = meta.model.clone();
                if model.is_none() && env.contains_key("GEMINI_BASE_URL") && !env.contains_key("GOOGLE_GEMINI_BASE_URL") || model.is_none() && env.contains_key("GOOGLE_GEMINI_BASE_URL") {
                    model = crate::acp::pick_gemini_model(&crate::codex::gate_model_ids(&env).await);
                    if let Some(m) = &model { self.meta.lock().unwrap().model = Some(m.clone()); self.save_meta(); }
                }
                if let (Some(flag), Some(m)) = (engine.model_flag, &model) { acp_args.push(flag.into()); acp_args.push(m.clone()); }
            } else if engine.id == "pi" {
                let default = crate::acp::pi_ensure_gate(&env).await;
                if meta.model.is_none() { if let Some(d) = default { self.meta.lock().unwrap().model = Some(d); self.save_meta(); } }
            } else if engine.id == "omp" {
                // Oh My Pi (§4.3.18): the gate provider in models.json, the model as a flag before the acp subcommand (omp does not
                // read config.json)
                let default = crate::acp::omp_ensure_gate(&env).await;
                let mut model = meta.model.clone();
                if model.is_none() { if let Some(d) = default { model = Some(d.clone()); self.meta.lock().unwrap().model = Some(d); self.save_meta(); } }
                if let Some(m) = &model { let v = if m.contains('/') { m.clone() } else { format!("agentverse-gate/{m}") }; acp_args = vec!["omp".into(), "--model".into(), v, "acp".into()]; }
            } else if engine.id == "antigravity" {
                // through the gate (agy's Gemini API key mode, §4.3.17): the modelProvider setting, the gate address
                // in the environment, the default model — the best Gemini from the gate catalog, model ids under
                // gemini-api://
                let gate = crate::acp::antigravity_ensure_gate(&env);
                acp_env.extend(crate::acp::antigravity_env(&env));
                let mut model = meta.model.clone();
                if model.is_none() && gate {
                    model = crate::acp::pick_gemini_model(&crate::codex::gate_model_ids(&env).await);
                    if let Some(m) = &model { self.meta.lock().unwrap().model = Some(m.clone()); self.save_meta(); }
                }
                acp_args.extend(crate::acp::antigravity_args(model.as_deref(), meta.effort.as_deref(), meta.permission_mode.as_deref(), gate));
            }
        }
        // Antigravity (agy.rs): its own stream-json process; through the gate — Gemini API key mode, model
        // gemini-api://<id>, the default model — the best Gemini from the gate catalog; resume — --conversation
        if is_agy {
            let env = crate::codex::effective_env(&meta);
            let gate = crate::acp::antigravity_ensure_gate(&env);
            acp_env.extend(crate::acp::antigravity_env(&env));
            let mut model = meta.model.clone();
            if model.is_none() && gate {
                model = crate::acp::pick_gemini_model(&crate::codex::gate_model_ids(&env).await);
                if let Some(m) = &model { self.meta.lock().unwrap().model = Some(m.clone()); self.save_meta(); }
            }
            let conv = if resume { meta.provider_session_id.clone() } else { None };
            acp_args = crate::agy::args(model.as_deref(), meta.effort.as_deref(), meta.permission_mode.as_deref(), gate, conv.as_deref());
            // native mode (Google account, §9.11): the account's HOME in the session environment, the models are its own (`agy
            // models` under that HOME)
            let native_models = if gate { Vec::new() } else { crate::agy::native_models(meta.env.get("HOME").map(String::as_str)).await };
            { let mut st = self.agy.lock().unwrap(); st.spawned_args = acp_args.clone(); st.in_turn = false; st.text.clear(); st.tools.clear(); st.native = !gate; if !native_models.is_empty() { st.native_models = native_models; } }
        }
        let mut args = if is_acp || is_agy { acp_args.clone() } else if is_codex { crate::codex::args() } else { vec![
            "claude".to_string(), "-p".into(), "--output-format".into(), "stream-json".into(), "--input-format".into(), "stream-json".into(),
            "--verbose".into(), "--permission-prompt-tool".into(), "stdio".into(),
            // subagent text comes with parent_tool_use_id and folds under the Agent card (without the flag the CLI does not
            // send it)
            "--forward-subagent-text".into(),
            // text chunks as the turn goes (stream_event / content_block_delta) — the live answer bubble in
            // the chat (slice 39)
            "--include-partial-messages".into(),
        ] };
        if !is_codex && !is_acp && !is_agy {
            match (&meta.provider_session_id, resume) {
                (Some(pid), true) => {
                    args.push("--resume".into()); args.push(pid.clone());
                    // message edit: continue from the last kept answer (a hidden CLI flag, the Agent SDK uses the same one)
                    if let Some(at) = &meta.resume_at { args.push("--resume-session-at".into()); args.push(at.clone()); }
                }
                _ => { args.push("--session-id".into()); args.push(meta.provider_session_id.clone().unwrap_or_else(|| meta.id.clone())); }
            }
            if let Some(m) = &meta.model { args.push("--model".into()); args.push(if meta.agent == "codex" { m.clone() } else { self.claude_model_for(m) }); }
            if let Some(p) = &meta.permission_mode { args.push("--permission-mode".into()); args.push(p.clone()); }
            if let Some(e) = &meta.effort { args.push("--effort".into()); args.push(e.clone()); }
            // SDK option: makes the bypassPermissions mode available to set_permission_mode; whether it is on is
            // decided by the session's permission_mode, and whether it is allowed at all — by the workspace
            // managed-settings (owner policy)
            args.push("--allow-dangerously-skip-permissions".into());
            // tool rules are variadic flags, hence at the very end
            if !meta.allowed_tools.is_empty() { args.push("--allowedTools".into()); args.extend(meta.allowed_tools.iter().cloned()); }
            if !meta.disallowed_tools.is_empty() { args.push("--disallowedTools".into()); args.extend(meta.disallowed_tools.iter().cloned()); }
        }
        // account environment — via `env` after the login shell: `-u` removes the gate variables from env.sh, K=V overrides
        // them
        let mut prefix: Vec<String> = Vec::new();
        let thinking = if is_codex || is_acp || is_agy { None } else { meta.max_thinking_tokens };
        let cache_ttl = if is_codex || is_acp || is_agy { None } else { meta.cache_ttl.clone() };
        // a non-Anthropic model through the gate (gemini-*, gpt-* …): the window from the proxy catalog, otherwise the CLI treats
        // it as "unknown model" = 200k
        let ctx_env = if is_codex || is_acp || is_agy { None } else { meta.context_window.filter(|_| meta.model.as_deref().map(|m| !m.to_ascii_lowercase().starts_with("claude")).unwrap_or(false)) };
        if !meta.unset.is_empty() || !meta.env.is_empty() || thinking.is_some() || cache_ttl.is_some() || ctx_env.is_some() || !acp_env.is_empty() {
            prefix.push("env".into());
            for u in &meta.unset { prefix.push("-u".into()); prefix.push(u.clone()); }
            let mut kv: Vec<_> = meta.env.iter().collect();
            kv.sort();
            for (k, v) in kv { prefix.push(format!("{k}={v}")); }
            for (k, v) in &acp_env { if !meta.env.contains_key(k) { prefix.push(format!("{k}={v}")); } }
            if let Some(n) = thinking { prefix.push(format!("MAX_THINKING_TOKENS={n}")); }
            if let Some(t) = &cache_ttl { prefix.push(format!("CLAUDE_CODE_PROMPT_CACHE_TTL={t}")); }
            if let Some(n) = ctx_env { prefix.push(format!("CLAUDE_CODE_MAX_CONTEXT_TOKENS={n}")); }
        }
        let cmdline = format!("{}exec {}", crate::engines::PATH_PREFIX, prefix.iter().chain(args.iter()).map(|a| shell_quote(a)).collect::<Vec<_>>().join(" "));
        let mut cmd = Command::new("bash");
        cmd.args(["-lic", &cmdline]).current_dir(&meta.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0).kill_on_drop(false);
        cmd.env("CLOUDOS_SESSION", &meta.id);
        let mut child = cmd.spawn().with_context(|| format!("запуск {cmdline}"))?;
        let stdin = child.stdin.take().context("stdin")?;
        let stdout = child.stdout.take().context("stdout")?;
        let stderr = child.stderr.take().context("stderr")?;
        *self.pid.lock().unwrap() = child.id();
        *self.stdin.lock().await = Some(stdin);
        *self.child.lock().unwrap() = Some(child);
        self.set_status(Status::Starting);
        self.mark_synced();
        // the resume point after an edit is one-shot
        if self.meta.lock().unwrap().resume_at.take().is_some() { self.save_meta(); }
        self.emit(json!({ "kind": "spawn", "cmd": args, "resume": resume, "account": meta.account, "env": meta.env.keys().collect::<Vec<_>>(), "unset": meta.unset }));
        // stderr — into the agentd log, not the session journal (shell noise and CLI warnings
        // live there)
        let id = meta.id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                tracing::debug!("session {} stderr: {}", &id[..8], l.chars().take(300).collect::<String>());
            }
        });
        let me = self.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let Ok(v) = serde_json::from_str::<Value>(&l) else { tracing::debug!("session {}: не JSON: {}", &me.id()[..8], l.chars().take(120).collect::<String>()); continue };
                me.on_message(v).await;
            }
            let code = me.child.lock().unwrap().take().map(|mut c| async move { c.wait().await.ok().and_then(|s| s.code()) });
            let code = match code { Some(f) => f.await, None => None };
            *me.stdin.lock().await = None;
            me.pending.lock().unwrap().clear();
            me.emit(json!({ "kind": "exit", "code": code }));
            me.tasks.lock().unwrap().clear();
            me.set_status(Status::Exited);
            me.mark_synced();
        });
        if is_acp {
            let me = self.clone();
            tokio::spawn(async move {
                if let Err(e) = me.acp_start(resume).await {
                    tracing::warn!("session {}: acp: {e:#}", &me.id()[..8]);
                    let text = format!("{e:#}");
                    // Antigravity without a Google sign-in: the adapter reports "agy process exited before emitting init event" — turn it into a hint on
                    // how to sign in
                    let agent = me.meta.lock().unwrap().agent.clone();
                    if let Some(hint) = crate::engines::login_help(&agent, &text) { me.emit(json!({ "kind": "system", "subtype": "agent_note", "data": { "message": hint } })); }
                    me.emit(json!({ "kind": "error", "message": text }));
                    me.set_status(Status::Error);
                    let _ = me.stop();
                }
            });
            return Ok(());
        }
        // agy: no handshake — the first init event moves the session to Idle and releases the
        // queue
        if is_agy { return Ok(()); }
        if is_codex {
            let me = self.clone();
            tokio::spawn(async move {
                if let Err(e) = me.codex_start(resume).await {
                    tracing::warn!("session {}: codex: {e:#}", &me.id()[..8]);
                    me.emit(json!({ "kind": "error", "message": format!("{e:#}") }));
                    me.set_status(Status::Error);
                }
            });
            return Ok(());
        }
        // control_request initialize: as the SDK does; the CLI's answer (commands, models) goes
        // into the journal
        let me = self.clone();
        tokio::spawn(async move {
            match me.control(json!({ "subtype": "initialize" })).await {
                Ok(r) => {
                    // the list of models and commands of the installed CLI — for the whole workspace: sessions that have not yet
                    // started under agentd need it too
                    if let Some(parent) = me.dir.parent() { let _ = std::fs::write(parent.join("claude-init.json"), serde_json::to_vec(&r).unwrap_or_default()); }
                    me.emit(json!({ "kind": "initialized", "response": r }));
                    // learn the CLI's default model before the first turn (get_context_usage.model) and set the `[1m]` tag
                    // right away: a model change takes effect from the next turn, while system/init with its model arrives
                    // already inside the first one
                    if me.meta.lock().unwrap().model.is_none() {
                        if let Ok(Ok(cu)) = tokio::time::timeout(std::time::Duration::from_secs(10), me.control(json!({ "subtype": "get_context_usage", "detail": "summary" }))).await {
                            if let Some(model) = cu.get("model").and_then(|m| m.as_str()).map(str::to_string) {
                                let n = me.claude_model_for(&model);
                                if n != model {
                                    match tokio::time::timeout(std::time::Duration::from_secs(15), me.control(json!({ "subtype": "set_model", "model": n.clone() }))).await {
                                        Ok(Ok(_)) => { me.meta.lock().unwrap().model = Some(n); me.save_meta(); }
                                        Ok(Err(e)) => tracing::warn!("set_model [1m] после initialize: {e:#}"),
                                        Err(_) => tracing::warn!("set_model [1m] после initialize: нет ответа 15 с"),
                                    }
                                }
                            }
                        }
                    }
                    if me.status() == Status::Starting { me.set_status(Status::Idle); me.drain_queue(); }
                }
                Err(e) => tracing::warn!("session {}: initialize: {e:#}", &me.id()[..8]),
            }
        });
        Ok(())
    }

    /// Our control request to the CLI (initialize, interrupt, set_permission_mode) waiting for
    /// the answer.
    async fn control(&self, request: Value) -> Result<Value> {
        let id = format!("agentd-{}", self.next_ctrl_id());
        let rx = self.ctrl_wait(&id);
        self.write(&json!({ "type": "control_request", "request_id": id, "request": request })).await?;
        match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => bail!("процесс завершился до ответа"),
            Err(_) => { self.ctrl_forget(&id); bail!("нет ответа на control-запрос за 60 с") }
        }
    }

    async fn on_message(self: &Arc<Self>, v: Value) {
        let agent = self.meta.lock().unwrap().agent.clone();
        if agent == "codex" {
            self.codex_on_message(v).await;
            return;
        }
        if crate::engines::is_acp(&agent) {
            self.acp_on_message(v).await;
            return;
        }
        if crate::engines::is_agy(&agent) {
            self.agy_on_message(v).await;
            return;
        }
        let t = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match t {
            "control_request" => self.on_control_request(v).await,
            "control_response" => {
                let r = v.get("response").cloned().unwrap_or(Value::Null);
                let rid = r.get("request_id").and_then(|s| s.as_str()).unwrap_or("").to_string();
                self.ctrl_resolve(&rid, r.get("response").cloned().unwrap_or(r.clone()));
            }
            "system" => {
                let sub = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
                if sub == "init" {
                    let mut m = self.meta.lock().unwrap();
                    if let Some(sid) = v.get("session_id").and_then(|s| s.as_str()) { m.provider_session_id = Some(sid.to_string()); }
                    // the model from init is a bare id (the CLI/settings.json default): native 1M families get the `[1m]`
                    // tag and the live process is switched with a set_model control, otherwise it believes in 200k (see
                    // normalize_claude_model)
                    let mut switch: Option<String> = None;
                    if let Some(model) = v.get("model").and_then(|s| s.as_str()) { drop(m); let n = self.claude_model_for(model); let mut m = self.meta.lock().unwrap(); if n != model { switch = Some(n.clone()); } m.model = Some(n); drop(m); } else { drop(m); }
                    self.save_meta();
                    self.emit(json!({ "kind": "init", "model": v.get("model"), "tools": v.get("tools"), "permission_mode": v.get("permissionMode"), "session_id": v.get("session_id"), "cwd": v.get("cwd") }));
                    match switch {
                        // a model change takes effect from the next turn: the first turn is held in the queue (status Starting)
                        // until the CLI confirms set_model — otherwise the first turn of a new session goes at 200k and
                        // get_context_usage lies
                        Some(n) => { let me = self.clone(); tokio::spawn(async move {
                            match tokio::time::timeout(std::time::Duration::from_secs(15), me.control(json!({ "subtype": "set_model", "model": n }))).await {
                                Ok(Ok(_)) => {}, Ok(Err(e)) => tracing::warn!("set_model [1m] после init: {e:#}"), Err(_) => tracing::warn!("set_model [1m] после init: нет ответа 15 с"),
                            }
                            if me.status() == Status::Starting { me.set_status(Status::Idle); me.drain_queue(); }
                        }); }
                        None => { if self.status() == Status::Starting { self.set_status(Status::Idle); self.drain_queue(); } }
                    }
                } else if matches!(sub, "task_started" | "task_updated" | "task_notification") {
                    // background work after a turn: a chip in the task strip, "N in background" on the session, a
                    // journal line on completion
                    let tid = v.get("task_id").and_then(|t| t.as_str()).unwrap_or("").to_string();
                    let status = v.get("status").and_then(|s| s.as_str()).or_else(|| v.pointer("/patch/status").and_then(|s| s.as_str())).unwrap_or(if sub == "task_started" { "running" } else { "" }).to_string();
                    {
                        let mut tasks = self.tasks.lock().unwrap();
                        if sub == "task_started" {
                            tasks.insert(tid.clone(), json!({ "task_id": tid, "tool_use_id": v.get("tool_use_id"), "task_type": v.get("task_type"), "description": v.get("description"), "workflow_name": v.get("workflow_name"), "started_at": now_ms(), "status": "running" }));
                        } else if sub == "task_notification" || matches!(status.as_str(), "completed" | "failed" | "killed" | "stopped" | "cancelled" | "error") {
                            tasks.remove(&tid);
                        } else if let Some(t) = tasks.get_mut(&tid) { if !status.is_empty() { t["status"] = json!(status); } }
                    }
                    self.emit(json!({ "kind": "task", "subtype": sub, "task_id": tid, "tool_use_id": v.get("tool_use_id"), "task_type": v.get("task_type"), "description": v.get("description"), "workflow_name": v.get("workflow_name"), "status": status, "summary": v.get("summary"), "output_file": v.get("output_file"), "patch": v.get("patch") }));
                } else if matches!(sub, "status" | "compact_boundary" | "permission_denied" | "notification" | "api_retry") {
                    self.emit(json!({ "kind": "system", "subtype": sub, "data": v }));
                }
            }
            "assistant" => {
                let content = v.pointer("/message/content").cloned().unwrap_or(json!([]));
                let model = v.pointer("/message/model").cloned();
                // `<synthetic>` — output of a local CLI slash command (/context, /cost, /effort…), not a model
                // answer; the API call usage is needed by the context card (input + cache = the context size
                // right now)
                let local = model.as_ref().and_then(|m| m.as_str()).map(|m| m.starts_with('<')).unwrap_or(false);
                if !local { if let Some(m) = model.as_ref().and_then(|m| m.as_str()) { *self.last_model.lock().unwrap() = Some(m.to_string()); } }
                self.emit(json!({ "kind": "assistant", "content": content, "model": model, "parent_tool_use_id": v.get("parent_tool_use_id"), "uuid": v.get("uuid"), "usage": v.pointer("/message/usage"), "local": local }));
                self.live_reset();
                self.set_status(Status::Working);
            }
            "user" => {
                // tool echo: tool_result for calls from the previous assistant
                let content = v.pointer("/message/content").cloned().unwrap_or(json!([]));
                let has_results = content.as_array().map(|a| a.iter().any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))).unwrap_or(false);
                if has_results {
                    self.emit(json!({ "kind": "tool_result", "content": content, "parent_tool_use_id": v.get("parent_tool_use_id") }));
                }
            }
            "result" => {
                let text: Option<String> = v.get("result").and_then(|r| r.as_str()).map(str::to_string);
                let ev = json!({
                    "kind": "result", "subtype": v.get("subtype"), "is_error": v.get("is_error"), "duration_ms": v.get("duration_ms"), "num_turns": v.get("num_turns"),
                    "total_cost_usd": v.get("total_cost_usd"), "usage": v.get("usage"), "model_usage": v.get("modelUsage"), "text": text, "session_id": v.get("session_id"),
                    "model": self.last_model.lock().unwrap().clone(),
                });
                self.record_result(ev);
                self.set_status(Status::Idle);
                self.drain_queue();
            }
            "stream_event" => {
                // live text: only the main agent (subagents are folded under the card), only text deltas
                if v.get("parent_tool_use_id").map(|p| !p.is_null()).unwrap_or(false) { return }
                let ev = v.get("event").cloned().unwrap_or(Value::Null);
                match ev.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "message_start" => self.live_reset(),
                    "content_block_delta" => { if ev.pointer("/delta/type").and_then(|t| t.as_str()) == Some("text_delta") { if let Some(t) = ev.pointer("/delta/text").and_then(|t| t.as_str()) { self.live_push(t); } } }
                    _ => {}
                }
            }
            "rate_limit_event" | "auth_status" => {
                if t == "rate_limit_event" {
                    let info = v.get("rate_limit_info").cloned().unwrap_or(Value::Null);
                    if info.get("status").and_then(|st| st.as_str()) == Some("rejected") {
                        let resets = info.get("resetsAt").and_then(|r| r.as_u64()).map(|secs| crate::session::rfc3339(if secs > 10_000_000_000 { secs / 1000 } else { secs }));
                        self.set_limit_hit(json!({ "window": info.get("rateLimitType"), "resets_at": resets, "utilization": info.get("utilization"), "source": "rate_limit_event" }));
                    }
                }
                self.emit(json!({ "kind": t, "data": v }));
            }
            _ => {}
        }
    }

    async fn on_control_request(self: &Arc<Self>, v: Value) {
        let rid = v.get("request_id").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let req = v.get("request").cloned().unwrap_or(Value::Null);
        let sub = req.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
        match sub {
            "can_use_tool" => {
                let tool = req.get("tool_name").and_then(|s| s.as_str()).unwrap_or("?").to_string();
                let input = req.get("input").cloned().unwrap_or(Value::Null);
                let ev = self.emit(json!({
                    "kind": "permission_request", "request_id": rid, "tool_name": tool, "input": input,
                    "suggestions": req.get("permission_suggestions"), "blocked_path": req.get("blocked_path"), "decision_reason": req.get("decision_reason"),
                    "title": req.get("title"), "description": req.get("description"), "display_name": req.get("display_name"), "tool_use_id": req.get("tool_use_id"),
                    "question": tool == "AskUserQuestion",
                }));
                let seq = ev.get("seq").and_then(|s| s.as_u64()).unwrap_or(0);
                self.pending.lock().unwrap().insert(rid.clone(), Pending { request_id: rid, tool_name: tool, input, seq, kind: "claude".into(), suggestions: req.get("permission_suggestions").cloned(), rpc_id: None });
                self.set_status(Status::Waiting);
            }
            "request_user_dialog" => {
                // CLI dialogs (updates, surveys) are not shown in headless
                let _ = self.write(&json!({ "type": "control_response", "response": { "subtype": "success", "request_id": rid, "response": { "behavior": "cancelled" } } })).await;
            }
            other => {
                let _ = self.write(&json!({ "type": "control_response", "response": { "subtype": "error", "request_id": rid, "error": format!("agentd: {other} не поддерживается") } })).await;
            }
        }
    }

    /// User message: text/image blocks as in the Messages API; status — working.
    pub async fn prompt(self: &Arc<Self>, content: Value) -> Result<Value> {
        if !self.alive() { bail!("сессия не запущена — возобновите её"); }
        let content = match content { Value::String(s) => json!([{ "type": "text", "text": s }]), c => c };
        // agy: the turn is still running — queue it (the protocol does not accept a second message before result);
        // model/effort/mode changed on a live session — restart the process with --conversation, the message leaves the
        // queue after init
        if crate::engines::is_agy(&self.meta.lock().unwrap().agent) && self.status() != Status::Starting {
            if self.agy.lock().unwrap().in_turn { let q = self.queue_push(content); return Ok(json!({ "queued": q })); }
            let gate = crate::acp::antigravity_gate(&crate::codex::effective_env(&self.meta.lock().unwrap().clone()));
            if self.agy_restart_needed(gate) {
                tracing::info!("session {}: agy — перезапуск под новые параметры", &self.id()[..8]);
                let _ = self.stop();
                self.wait_exit(std::time::Duration::from_secs(6)).await;
                self.spawn(true).await?;
            }
        }
        // the process is still starting (initialize / thread/start after a resume): the message goes to the queue and leaves
        // by itself after init (otherwise Codex turn/start hits the previous thread — "thread not found", Claude loses the
        // input)
        if self.status() == Status::Starting { let q = self.queue_push(content); return Ok(json!({ "queued": q })); }
        // the first message gives the session a name instead of "Claude"/"Codex" — as in CloudCLI; the lock is
        // released before save_meta
        let auto_title = {
            let m = self.meta.lock().unwrap();
            if matches!(m.title.as_str(), "Claude" | "Codex" | "Claude Code") || crate::engines::ENGINES.iter().any(|e| e.title == m.title) {
                content.as_array().and_then(|a| a.iter().find_map(|b| b.get("text").and_then(|t| t.as_str())))
                    .map(|t| t.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(48).collect::<String>()).filter(|t| !t.is_empty())
            } else { None }
        };
        if let Some(title) = auto_title {
            self.meta.lock().unwrap().title = title.clone();
            self.save_meta();
            self.emit(json!({ "kind": "title", "title": title }));
        }
        let agent = self.meta.lock().unwrap().agent.clone();
        if agent == "codex" {
            self.codex_prompt(&content).await?;
        } else if crate::engines::is_acp(&agent) {
            self.acp_prompt(&content).await?;
        } else if crate::engines::is_agy(&agent) {
            self.agy_prompt(&content).await?;
        } else {
            self.write(&json!({ "type": "user", "message": { "role": "user", "content": content }, "parent_tool_use_id": Value::Null })).await?;
        }
        // into the journal — without image base64 (only the size), otherwise it grows
        // by megabytes
        let stored: Vec<Value> = content.as_array().cloned().unwrap_or_default().into_iter().map(|b| match b.get("type").and_then(|t| t.as_str()) {
            Some("image") => json!({ "type": "image", "media_type": b.pointer("/source/media_type"), "bytes": b.pointer("/source/data").and_then(|d| d.as_str()).map(|d| d.len() * 3 / 4) }),
            _ => b,
        }).collect();
        let ev = self.emit(json!({ "kind": "user", "content": stored }));
        self.live_reset();
        self.set_status(Status::Working);
        Ok(ev)
    }

    /// Queue: the message goes out by itself when the session frees up (even with no browser
    /// open).
    pub fn queue_push(&self, content: Value) -> Value {
        let item = json!({ "id": uuid::Uuid::new_v4().to_string(), "content": content, "ts": now_ms() });
        self.queue.lock().unwrap().push(item.clone());
        self.emit(json!({ "kind": "queued", "item": item }));
        item
    }
    pub fn queue_remove(&self, id: &str) -> bool {
        let mut q = self.queue.lock().unwrap();
        let before = q.len(); q.retain(|i| i.get("id").and_then(|x| x.as_str()) != Some(id));
        let removed = q.len() != before; drop(q);
        if removed { self.emit(json!({ "kind": "unqueued", "id": id })); }
        removed
    }
    /// After the answer — the next message from the queue (if there are no pending
    /// permissions).
    pub(crate) fn drain_queue(self: &Arc<Self>) {
        let next = { let mut q = self.queue.lock().unwrap(); if q.is_empty() || !self.pending.lock().unwrap().is_empty() { None } else { Some(q.remove(0)) } };
        if let Some(item) = next {
            let me = self.clone();
            tokio::spawn(async move {
                let content = item.get("content").cloned().unwrap_or(Value::Null);
                me.emit(json!({ "kind": "unqueued", "id": item.get("id"), "sent": true }));
                if let Err(e) = me.prompt(content).await { tracing::warn!("session {}: сообщение из очереди не ушло: {e:#}", &me.id()[..8]); }
            });
        }
    }

    /// Editing a sent message (Claude): the journal is cut to the previous answer, the process is restarted with
    /// `--resume-session-at <answer uuid>`, the new text goes as a regular turn. Editing the first message starts the
    /// conversation over under a new provider id. Returns the event of the sent message.
    pub async fn edit(self: &Arc<Self>, seq: u64, content: Value) -> Result<Value> {
        if self.meta.lock().unwrap().agent != "claude" { bail!("правка сообщений поддерживается только у Claude Code — используйте форк"); }
        let (ok_user, anchor) = {
            let j = self.journal.lock().unwrap();
            let is_user = j.events.iter().any(|e| e.get("seq").and_then(|s| s.as_u64()) == Some(seq) && e.get("kind").and_then(|k| k.as_str()) == Some("user") && e.get("parent_tool_use_id").map(|p| p.is_null()).unwrap_or(true));
            (is_user, if seq > 1 { crate::fork::anchor_uuid(&j.events, seq - 1) } else { None })
        };
        if !ok_user { bail!("событие {seq} — не сообщение пользователя"); }
        if self.alive() {
            self.stop()?;
            if !self.wait_exit(std::time::Duration::from_secs(10)).await { bail!("процесс сессии не остановился за 10 с"); }
        }
        {
            let mut j = self.journal.lock().unwrap();
            j.truncate(&self.dir, seq - 1)?;
        }
        self.recount_usage();
        self.emit(json!({ "kind": "history_truncated", "at": seq - 1, "anchor": anchor }));
        {
            let mut m = self.meta.lock().unwrap();
            match &anchor {
                Some(a) => { m.resume_at = Some(a.clone()); }
                None => { m.provider_session_id = Some(uuid::Uuid::new_v4().to_string()); m.transcript = None; m.synced_lines = Some(0); m.usage_base = None; }
            }
        }
        self.save_meta();
        self.spawn(anchor.is_some()).await?;
        self.prompt(content).await
    }

    /// Recompute usage and the last text from the journal (after
    /// truncation).
    fn recount_usage(&self) {
        let base = self.meta.lock().unwrap().usage_base.clone();
        let mut usage = base.clone().unwrap_or_default();
        let mut last_total = base.map(|b| b.cost_usd).unwrap_or(0.0);
        let mut last_text = None;
        for ev in &self.journal.lock().unwrap().events {
            if ev.get("kind").and_then(|k| k.as_str()) == Some("result") {
                let delta = cost_delta(&mut last_total, ev); add_usage(&mut usage, ev, delta);
                if let Some(t) = ev.get("text").and_then(|t| t.as_str()) { last_text = Some(t.chars().take(200).collect()); }
            }
        }
        *self.usage.lock().unwrap() = usage;
        *self.last_total_cost.lock().unwrap() = last_total;
        *self.last_text.lock().unwrap() = last_text;
    }

    /// Stop a Claude Code background task (control `stop_task`): the CLI answers with
    /// `task_notification`/`task_updated`.
    pub async fn stop_task(&self, task_id: &str) -> Result<Value> {
        if !self.alive() { bail!("сессия не запущена"); }
        if self.meta.lock().unwrap().agent != "claude" { bail!("фоновые задачи есть только у Claude Code"); }
        self.control(json!({ "subtype": "stop_task", "task_id": task_id })).await
    }

    /// Permission answer: allow (with modified input or answers to questions) or deny with a reason.
    pub async fn answer(&self, request_id: &str, behavior: &str, updated_input: Option<Value>, message: Option<String>, answers: Option<Value>, remember: bool, scope: Option<String>, permissions: Option<Value>) -> Result<Value> {
        let p = self.pending.lock().unwrap().remove(request_id).ok_or_else(|| anyhow!("нет ожидающего запроса {request_id}"))?;
        if p.kind == "acp" {
            let rid = p.rpc_id.clone().unwrap_or(Value::Null);
            self.acp_answer(&p, &rid, behavior, remember).await?;
            let ev = self.emit(json!({ "kind": "permission_result", "request_id": request_id, "tool_name": p.tool_name, "behavior": behavior, "remember": remember }));
            if self.pending.lock().unwrap().is_empty() { self.set_status(Status::Working); }
            return Ok(ev);
        }
        if p.kind.starts_with("codex") {
            let rid = p.rpc_id.clone().unwrap_or(Value::Null);
            self.codex_answer(&p, &rid, behavior, answers, remember).await?;
            let ev = self.emit(json!({ "kind": "permission_result", "request_id": request_id, "tool_name": p.tool_name, "behavior": behavior, "remember": remember }));
            if self.pending.lock().unwrap().is_empty() { self.set_status(Status::Working); }
            return Ok(ev);
        }
        let response = match behavior {
            "allow" => {
                let mut input = updated_input.unwrap_or_else(|| p.input.clone());
                if let Some(a) = answers { input["answers"] = a; if input.get("questions").is_none() { input["questions"] = p.input.get("questions").cloned().unwrap_or(json!([])); } }
                let mut r = json!({ "behavior": "allow", "updatedInput": input });
                // "remember": CLI rule suggestions (`permission_suggestions`) go as updatedPermissions with the chosen
                // scope — session (this process only), localSettings (the project's .claude/settings.local.json),
                // userSettings (~/.claude/settings.json); explicit `permissions` beat the suggestions
                let mut perms: Vec<Value> = permissions.as_ref().and_then(|p| p.as_array()).cloned().unwrap_or_default();
                if perms.is_empty() && remember {
                    let dest = match scope.as_deref() { Some("session") => "session", Some("user") => "userSettings", _ => "localSettings" };
                    let suggestions = p.suggestions.clone().unwrap_or(json!([]));
                    for mut sug in suggestions.as_array().cloned().unwrap_or_default() { sug["destination"] = json!(dest); perms.push(sug); }
                }
                if !perms.is_empty() { r["updatedPermissions"] = json!(perms); }
                r
            }
            "deny" => json!({ "behavior": "deny", "message": message.unwrap_or_else(|| "Отклонено пользователем".into()) }),
            other => bail!("behavior: allow | deny, получено {other}"),
        };
        let rules = permission_rules(response.get("updatedPermissions"));
        self.write(&json!({ "type": "control_response", "response": { "subtype": "success", "request_id": request_id, "response": response } })).await?;
        let ev = self.emit(json!({ "kind": "permission_result", "request_id": request_id, "tool_name": p.tool_name, "behavior": behavior, "remember": remember, "scope": scope, "rules": rules.iter().map(|(t, c)| if c.is_empty() { t.clone() } else { format!("{t}({c})") }).collect::<Vec<_>>() }));
        // other pending requests of the same tool that fall under the remembered rule — approve ourselves (like "Allow & remember" in
        // CloudCLI)
        if behavior == "allow" && !rules.is_empty() {
            let others: Vec<Pending> = self.pending.lock().unwrap().values().filter(|q| q.kind == "claude" && rule_matches(&rules, &q.tool_name, &q.input)).cloned().collect();
            for q in others {
                self.pending.lock().unwrap().remove(&q.request_id);
                if self.write(&json!({ "type": "control_response", "response": { "subtype": "success", "request_id": q.request_id, "response": { "behavior": "allow", "updatedInput": q.input } } })).await.is_ok() {
                    self.emit(json!({ "kind": "permission_result", "request_id": q.request_id, "tool_name": q.tool_name, "behavior": "allow", "auto": true }));
                }
            }
        }
        if self.pending.lock().unwrap().is_empty() { self.set_status(Status::Working); }
        Ok(ev)
    }

    /// A line into the workspace-wide usage journal: one per `result`, with the same seq as in the session
    /// journal.
    fn append_usage(&self, ev: &Value) {
        let meta = self.meta.lock().unwrap().clone();
        let rec = json!({
            "session": meta.id, "agent": meta.agent, "provider_session_id": meta.provider_session_id, "ts": ev.get("ts"), "seq": ev.get("seq"),
            "usage": ev.get("usage"), "model_usage": ev.get("model_usage"), "total_cost_usd": ev.get("total_cost_usd"), "cost_usd": ev.get("cost_usd"), "duration_ms": ev.get("duration_ms"), "num_turns": ev.get("num_turns"),
        });
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.usage_log) {
            let _ = writeln!(f, "{rec}");
        }
    }

    pub async fn interrupt(&self) -> Result<Value> {
        let agent = self.meta.lock().unwrap().agent.clone();
        let r = if agent == "codex" { self.codex_interrupt().await? } else if crate::engines::is_acp(&agent) { self.acp_interrupt().await? } else if crate::engines::is_agy(&agent) { self.agy_interrupt().await? } else { self.control(json!({ "subtype": "interrupt" })).await? };
        self.emit(json!({ "kind": "interrupt" }));
        Ok(r)
    }

    /// Model change: Claude — control `set_model`; Codex — remembered, goes with the next `turn/start`.
    /// Own name (§4.8): empty — back to the auto-title; the journal is untouched, the registry — through
    /// the hub
    pub fn set_label(&self, label: Option<String>) -> Value {
        let label = label.map(|l| l.trim().chars().take(80).collect::<String>()).filter(|l| !l.is_empty());
        { let mut m = self.meta.lock().unwrap(); m.label = label.clone(); }
        self.save_meta();
        crate::hub::touch();
        json!({ "ok": true, "label": label, "title": self.meta.lock().unwrap().title })
    }
    pub async fn set_model(&self, model: Option<String>, context_window: Option<u64>) -> Result<Value> {
        let agent = self.meta.lock().unwrap().agent.clone();
        let is_codex = agent == "codex";
        let is_acp = crate::engines::is_acp(&agent) || crate::engines::is_agy(&agent);
        let model = if is_codex || is_acp { model } else { model.map(|m| self.claude_model_for(&m)) };
        // a stopped session: remember it, on resume it goes as the --model flag (as in CloudCLI — flags at start)
        let r = if is_acp && !crate::engines::is_agy(&agent) && self.alive() { self.acp_set_model(model.as_deref()).await? } else if is_codex || is_acp || !self.alive() { json!({ "ok": true, "applied": "next_start" }) } else { self.control(json!({ "subtype": "set_model", "model": model })).await? };
        { let mut m = self.meta.lock().unwrap(); m.model = model.clone(); m.context_window = context_window; }
        self.save_meta();
        self.emit(json!({ "kind": "model", "model": model }));
        Ok(r)
    }

    /// Model effort: Claude — control `apply_flag_settings {effortLevel}` on a live process and `--effort` at the next
    /// start; Codex — `effort` in the next `turn/start`. An empty value — the model default.
    pub async fn set_effort(&self, effort: Option<String>) -> Result<Value> {
        let (is_claude, alive) = (self.meta.lock().unwrap().agent == "claude", self.alive());
        let r = if is_claude && alive { self.control(json!({ "subtype": "apply_flag_settings", "settings": { "effortLevel": effort } })).await? } else { json!({ "ok": true }) };
        self.meta.lock().unwrap().effort = effort.clone();
        self.save_meta();
        self.emit(json!({ "kind": "effort", "effort": effort }));
        Ok(r)
    }

    /// Own thinking budget (Claude): control `set_max_thinking_tokens` on a live process, MAX_THINKING_TOKENS at start.
    pub async fn set_thinking(&self, tokens: Option<u64>) -> Result<Value> {
        if self.meta.lock().unwrap().agent != "claude" { bail!("бюджет думания задаётся только у Claude Code"); }
        let r = if self.alive() { self.control(json!({ "subtype": "set_max_thinking_tokens", "max_thinking_tokens": tokens })).await? } else { json!({ "ok": true, "applied": "next_start" }) };
        self.meta.lock().unwrap().max_thinking_tokens = tokens;
        self.save_meta();
        self.emit(json!({ "kind": "thinking", "max_thinking_tokens": tokens }));
        Ok(r)
    }

    /// Prompt cache TTL (Claude): live — control `apply_flag_settings {promptCacheTtl}` (the CLI's flag-settings key),
    /// at start — CLAUDE_CODE_PROMPT_CACHE_TTL. The CLI itself gives 1h to subscribers and 5m to key/gate; here the
    /// owner chooses.
    pub async fn set_cache_ttl(&self, ttl: Option<String>) -> Result<Value> {
        if self.meta.lock().unwrap().agent != "claude" { bail!("TTL кэша промпта задаётся только у Claude Code"); }
        let r = if self.alive() { self.control(json!({ "subtype": "apply_flag_settings", "settings": { "promptCacheTtl": ttl } })).await? } else { json!({ "ok": true, "applied": "next_start" }) };
        self.meta.lock().unwrap().cache_ttl = ttl.clone();
        self.save_meta();
        self.emit(json!({ "kind": "cache_ttl", "ttl": ttl }));
        Ok(r)
    }

    /// Service tier (Codex): `fast` — fast mode, goes into the next `turn/start` as serviceTier. For Claude, fast mode is
    /// unavailable in the SDK's headless mode (fast_mode_disabled_reason = sdk_opt_in_required) — only in the TUI via /fast.
    pub async fn set_tier(&self, tier: Option<String>) -> Result<Value> {
        if self.meta.lock().unwrap().agent != "codex" { bail!("быстрый режим Claude недоступен в headless-режиме SDK — включается в CLI командой /fast"); }
        self.meta.lock().unwrap().service_tier = tier.clone();
        self.save_meta();
        self.emit(json!({ "kind": "tier", "tier": tier }));
        Ok(json!({ "ok": true, "applied": "next_turn" }))
    }

    /// Arbitrary provider request for the UI: Claude — control_request (`get_context_usage`, `file_suggestions`,
    /// `get_usage`…), Codex — app-server JSON-RPC (`{method, params}`). The provider's answer as is.
    pub async fn control_request(&self, req: Value) -> Result<Value> {
        if !self.alive() { bail!("сессия не запущена — возобновите её"); }
        if self.meta.lock().unwrap().agent == "codex" {
            let method = req.get("method").and_then(|m| m.as_str()).ok_or_else(|| anyhow!("для Codex нужен {{method, params}}"))?.to_string();
            let mut params = req.get("params").cloned().unwrap_or(json!({}));
            if params.get("threadId").is_none() { if let Some(tid) = self.codex.lock().unwrap().thread_id.clone() { params["threadId"] = json!(tid); } }
            self.codex_call(&method, params).await
        } else {
            self.control(req).await
        }
    }

    /// Session capabilities for the UI (provider adapters): models and slash commands from the installed Claude CLI's
    /// `initialize` (or the Codex list), effort levels, modes, current Codex context.
    pub fn caps(&self) -> Value {
        let mut v = self.caps_inner();
        if let Some(o) = v.as_object_mut() { o.insert("mcp".into(), json!(crate::mcp::count())); }
        v
    }
    fn caps_inner(&self) -> Value {
        let meta = self.meta.lock().unwrap().clone();
        if crate::engines::is_agy(&meta.agent) {
            // Antigravity (agy.rs): models — the gate catalog, modes and efforts — its own, context — from the usage
            // of the last step
            let gate_ids: Vec<String> = self.dir.parent().and_then(|p| std::fs::read(p.join("gate-models.json")).ok()).and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok()).unwrap_or_default();
            let a = self.agy_caps(&meta, &gate_ids);
            return json!({
                "provider": meta.agent, "protocol": "agy", "models": a["models"], "commands": [], "efforts": a["efforts"], "effort_hints": a["effort_hints"], "default_effort": Value::Null, "tiers": [],
                "effort": meta.effort, "max_thinking_tokens": Value::Null, "service_tier": Value::Null, "cache_ttl": Value::Null,
                "modes": a["modes"], "mode_options": a["mode_options"], "current_mode": a["current_mode"], "model": a["model"], "token_usage": Value::Null, "context_window": a["context_window"], "acp_usage": a["acp_usage"],
                "account": Value::Null, "output_style": Value::Null, "fast_mode": Value::Null, "bypass_allowed": true, "auth_methods": [], "config_options": [],
                "features": { "commands": false, "files": false, "effort": true, "fast": false, "fast_reason": Value::Null, "edit": false, "fork": false, "cache": false, "thinking": false },
            });
        }
        if crate::engines::is_acp(&meta.agent) {
            // ACP engine: models and modes — from the agent (or the gate for Gemini), no efforts or tiers, commands — its
            // slash commands
            let gate_ids: Vec<String> = self.dir.parent().and_then(|p| std::fs::read(p.join("gate-models.json")).ok()).and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok()).unwrap_or_default();
            let a = self.acp_caps(&meta, &gate_ids);
            return json!({
                "provider": meta.agent, "protocol": "acp", "models": a["models"], "commands": a["commands"], "efforts": [], "effort_hints": {}, "default_effort": Value::Null, "tiers": [],
                "effort": meta.effort, "max_thinking_tokens": Value::Null, "service_tier": Value::Null, "cache_ttl": Value::Null,
                "modes": a["modes"], "mode_options": a["mode_options"], "current_mode": a["current_mode"], "model": a["model"], "token_usage": Value::Null, "context_window": a["context_window"], "acp_usage": a["acp_usage"],
                "account": Value::Null, "output_style": Value::Null, "fast_mode": Value::Null, "bypass_allowed": true, "auth_methods": a["auth_methods"], "config_options": a["config_options"],
                "features": { "commands": true, "files": false, "effort": false, "fast": false, "fast_reason": Value::Null, "edit": false, "fork": false, "cache": false, "thinking": false },
            });
        }
        let is_codex = meta.agent == "codex";
        let mut init = self.journal.lock().unwrap().events.iter().rev().find(|e| e.get("kind").and_then(|k| k.as_str()) == Some("initialized")).and_then(|e| e.get("response").cloned()).unwrap_or(Value::Null);
        if init.is_null() && !is_codex { if let Some(parent) = self.dir.parent() { init = std::fs::read(parent.join("claude-init.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null); } }
        let codex_models: Vec<Value> = if is_codex {
            let mut l = self.codex.lock().unwrap().models.clone();
            if l.is_empty() { if let Some(parent) = self.dir.parent() { l = std::fs::read(parent.join("codex-models.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default(); } }
            l
        } else { Vec::new() };
        let models = if is_codex {
            if codex_models.is_empty() { json!([{ "value": "gpt-6-astra", "label": "GPT-6-Astra", "efforts": ["low", "medium", "high", "xhigh", "max", "ultra"], "default": true }, { "value": "gpt-5.5", "label": "GPT-5.5", "efforts": ["low", "medium", "high", "xhigh"] }, { "value": "gpt-oss-120b-medium", "label": "gpt-oss-120b", "efforts": ["low", "medium", "high"] }]) } else { json!(codex_models.iter().filter(|m| m.get("hidden").and_then(|h| h.as_bool()) != Some(true)).cloned().collect::<Vec<_>>()) }
        } else {
            // context window: `[1m]` in the id or "1M context" in the description — 1 000 000; native 1M families (fable-5, opus-5,
            // sonnet-5) — also 1 000 000: with a subscription CLI 2.1.291 reports get_context_usage.maxTokens = 1e6 for them even
            // without the tag (catalog descriptions say nothing about the window), through API/gate agentd sets the `[1m]` tag
            // itself on selection (claude_model_for); otherwise 200 000
            let window = |m: &Value| -> u64 {
                let resolved = m.get("resolvedModel").and_then(|v| v.as_str()).unwrap_or("");
                let text = format!("{} {} {}", m.get("value").and_then(|v| v.as_str()).unwrap_or(""), resolved, m.get("description").and_then(|v| v.as_str()).unwrap_or(""));
                if text.contains("[1m]") || text.to_ascii_lowercase().contains("1m context") { 1_000_000 }
                else if !resolved.is_empty() && normalize_claude_model(resolved) != resolved { 1_000_000 }
                else { 200_000 }
            };
            let mut list: Vec<Value> = init.get("models").and_then(|m| m.as_array()).cloned().unwrap_or_default().into_iter().map(|m| json!({
                "value": m.get("value"), "label": m.get("displayName"), "description": m.get("description"), "resolved": m.get("resolvedModel"),
                "efforts": m.get("supportedEffortLevels"), "fast": m.get("supportsFastMode"), "context": window(&m),
            })).collect();
            // the CLI catalog gives "model / model (1M context)" pairs only for Opus and Sonnet; for Fable the `[1m]` alias is
            // accepted too and gives a 1 000 000 window (verified through the gate) — we add such an entry ourselves
            let fable_1m: Vec<Value> = list.iter().filter(|m| {
                let v = m.get("value").and_then(|v| v.as_str()).unwrap_or("");
                v.starts_with("claude-fable") && !v.contains("[1m]") && !list.iter().any(|o| o.get("value").and_then(|x| x.as_str()) == Some(&format!("{v}[1m]")))
            }).map(|m| {
                let v = m.get("value").and_then(|v| v.as_str()).unwrap_or("");
                let label = m.get("label").and_then(|l| l.as_str()).unwrap_or("Fable");
                json!({ "value": format!("{v}[1m]"), "label": format!("{label} (1M context)"), "description": format!("{label} with 1M context"), "resolved": Value::Null, "efforts": m.get("efforts"), "fast": m.get("fast"), "context": 1_000_000 })
            }).collect();
            if let Some(pos) = list.iter().position(|m| m.get("value").and_then(|v| v.as_str()).map(|v| v.starts_with("claude-fable")).unwrap_or(false)) { for (i, f) in fable_1m.into_iter().enumerate() { list.insert(pos + 1 + i, f); } }
            // native 1M: one entry with the `[1m]` tag (1 000 000 window), no duplicate "Fable ·
            // 200k"
            let mut seen = std::collections::HashSet::new();
            list = list.into_iter().map(|mut m| {
                let v = m.get("value").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let nv = normalize_claude_model(&v);
                if nv != v { m["value"] = json!(nv); m["context"] = json!(1_000_000); if m.get("resolved").map(|r| r.is_null()).unwrap_or(true) { m["resolved"] = json!(v); } }
                m
            }).filter(|m| seen.insert(m.get("value").and_then(|x| x.as_str()).unwrap_or("").to_string())).collect();
            if list.is_empty() { json!([{ "value": "opus", "label": "Opus", "context": 200000 }, { "value": "opus[1m]", "label": "Opus (1M context)", "context": 1000000 }, { "value": "claude-fable-5-1", "label": "Fable", "context": 200000 }, { "value": "claude-fable-5-1[1m]", "label": "Fable (1M context)", "context": 1000000 }, { "value": "sonnet", "label": "Sonnet", "context": 200000 }, { "value": "sonnet[1m]", "label": "Sonnet (1M context)", "context": 1000000 }, { "value": "haiku", "label": "Haiku", "context": 200000 }]) } else { json!(list) }
        };
        let commands = if is_codex {
            json!([{ "name": "model", "argumentHint": "<model>", "description": "Модель следующих ходов" }, { "name": "effort", "argumentHint": "<low|medium|high|xhigh>", "description": "Усилие reasoning следующих ходов" }, { "name": "mode", "argumentHint": "<on-request|untrusted|never>", "description": "Политика одобрений" }])
        } else {
            init.get("commands").cloned().unwrap_or(json!([]))
        };
        let codex = self.codex.lock().unwrap();
        // reasoning levels and tiers — those of the selected Codex model (or of the default model from the catalog) when the session
        // model is in the catalog; the Codex catalog does not know a gate model (gpt-oss-…) — then take the levels and tiers of the
        // default model
        let by_default = || models.as_array().and_then(|a| a.iter().find(|m| m.get("default").and_then(|d| d.as_bool()) == Some(true)).cloned());
        let cur_model = models.as_array().and_then(|a| a.iter().find(|m| meta.model.as_deref().map(|id| m.get("value").and_then(|v| v.as_str()) == Some(id)).unwrap_or(false)).cloned()).or_else(by_default);
        let codex_efforts = cur_model.as_ref().and_then(|m| m.get("efforts").cloned()).filter(|e| e.as_array().map(|a| !a.is_empty()).unwrap_or(false)).unwrap_or(json!(["low", "medium", "high", "xhigh"]));
        let effort_hints = cur_model.as_ref().and_then(|m| m.get("effort_hints").cloned()).unwrap_or(json!({}));
        let tiers = cur_model.as_ref().and_then(|m| m.get("tiers").cloned()).unwrap_or(json!([]));
        json!({
            "provider": meta.agent, "models": models, "commands": commands,
            "efforts": if is_codex { codex_efforts } else { json!(["low", "medium", "high", "xhigh", "max", "ultracode"]) },
            "effort_hints": if is_codex { effort_hints } else { json!({ "low": "быстро, без долгих размышлений", "medium": "сбалансированно", "high": "глубже думает", "xhigh": "очень глубоко", "max": "максимум: много токенов, долгие ответы", "ultracode": "xhigh + оркестрация workflow" }) },
            "default_effort": cur_model.as_ref().and_then(|m| m.get("default_effort").cloned()).unwrap_or(Value::Null),
            "tiers": if is_codex { tiers } else { json!([]) },
            "effort": meta.effort, "max_thinking_tokens": meta.max_thinking_tokens, "service_tier": meta.service_tier, "cache_ttl": meta.cache_ttl,
            "modes": if is_codex { json!(["on-request", "untrusted", "never"]) } else { json!(["default", "acceptEdits", "plan", "auto", "bypassPermissions", "dontAsk"]) },
            "token_usage": if is_codex { codex.last_usage.clone().unwrap_or(Value::Null) } else { Value::Null },
            "context_window": if is_codex { json!(codex.context_window) } else { json!(meta.context_window) },
            "account": init.get("account"), "output_style": init.get("output_style"), "fast_mode": init.get("fast_mode_state"),
            "bypass_allowed": is_codex || bypass_allowed(),
            "features": { "commands": true, "files": true, "effort": true, "fast": is_codex && cur_model.as_ref().and_then(|m| m.get("tiers")).and_then(|t| t.as_array()).map(|a| !a.is_empty()).unwrap_or(true), "fast_reason": if is_codex { Value::Null } else { json!(init.get("fast_mode_disabled_reason").and_then(|r| r.as_str()).unwrap_or("sdk_opt_in_required")) }, "thinking": !is_codex, "compact": !is_codex },
        })
    }

    /// Permission mode: Claude — control `set_permission_mode`; Codex — approvalPolicy and sandboxPolicy of the next turn (codex::approval,
    /// codex::sandbox_policy).
    pub async fn set_permission_mode(&self, mode: &str) -> Result<Value> {
        let agent = self.meta.lock().unwrap().agent.clone();
        let is_codex = agent == "codex";
        if crate::engines::is_agy(&agent) {
            // agy: the mode is a start flag; on a live session it applies through a restart at the next
            // message
            if !["default", "acceptEdits", "plan", "auto", "bypassPermissions", "dontAsk"].contains(&mode) { bail!("у Antigravity нет режима {mode}"); }
            self.meta.lock().unwrap().permission_mode = Some(mode.to_string());
            self.save_meta();
            self.emit(json!({ "kind": "permission_mode", "mode": mode }));
            return Ok(json!({ "ok": true, "mode": mode, "applied": if self.alive() { "next_message" } else { "next_start" } }));
        }
        if crate::engines::is_acp(&agent) {
            let r = if self.alive() { self.acp_set_mode(mode).await? } else { json!({ "ok": true, "applied": "next_start" }) };
            let stored = r.get("mode").and_then(|m| m.as_str()).unwrap_or(mode).to_string();
            self.meta.lock().unwrap().permission_mode = Some(stored.clone());
            self.save_meta();
            self.emit(json!({ "kind": "permission_mode", "mode": stored }));
            return Ok(r);
        }
        if !is_codex && mode == "bypassPermissions" && !bypass_allowed() { bail!("режим обхода отключён политикой workspace'а (managed-settings) — включается владельцем в Настройках → Разрешения"); }
        let r = if is_codex || !self.alive() { json!({ "ok": true, "applied": "next_start" }) } else {
            self.control(json!({ "subtype": "set_permission_mode", "mode": mode })).await.map_err(|e| if e.to_string().contains("bypass") { anyhow!("режим обхода отклонён CLI: {e}") } else { e })?
        };
        self.meta.lock().unwrap().permission_mode = Some(mode.to_string());
        self.save_meta();
        self.emit(json!({ "kind": "permission_mode", "mode": mode }));
        Ok(r)
    }

    /// Wait for the process to stop (after stop) for at most `timeout`; returns whether it
    /// stopped.
    pub async fn wait_exit(&self, timeout: std::time::Duration) -> bool {
        let mut rx = self.status.subscribe();
        let _ = tokio::time::timeout(timeout, async { while !matches!(*rx.borrow_and_update(), Status::Exited | Status::Error) { if rx.changed().await.is_err() { break } } }).await;
        !self.alive()
    }

    /// Terminal mode command (as in CloudCLI): the agent's native TUI on the same conversation, in the same login shell and with
    /// the same account environment as the headless process. Claude: `--resume` if the provider journal already exists on disk,
    /// otherwise `--session-id` (the first turn creates it under our id); Codex: `codex resume <thread>` with the same gate
    /// provider via `-c`. `bypass` — permission bypass (bypassPermissions / --dangerously-bypass-approvals-and-sandbox),
    /// otherwise the session mode.
    pub(crate) async fn tui_command(&self, bypass: bool) -> Result<(Vec<String>, HashMap<String, String>)> {
        let meta = self.meta.lock().unwrap().clone();
        let env = crate::codex::effective_env(&meta);
        let args: Vec<String> = if meta.agent == "codex" {
            let gate = crate::codex::gate_config(&env);
            let mut model = meta.model.clone().or_else(|| env.get("CODEX_MODEL").cloned()).or_else(|| env.get("OPENAI_MODEL").cloned());
            // the model — from the route catalog (as in codex_start): a foreign one after an account change gives "model
            // not found" in the TUI
            if gate.is_some() {
                let ids = crate::codex::gate_model_ids(&env).await;
                if let Some(m) = &model { if !ids.is_empty() && !ids.contains(m) { model = None; } }
                if model.is_none() { model = crate::codex::rank_models(&ids); }
            } else if let Some(m) = &model {
                let cached: Vec<String> = self.dir.parent().and_then(|p| std::fs::read(p.join("codex-models.json")).ok()).and_then(|b| serde_json::from_slice::<Vec<Value>>(&b).ok()).map(|l| l.iter().filter_map(|x| x.get("value").and_then(|v| v.as_str()).map(str::to_string)).collect()).unwrap_or_default();
                if !cached.is_empty() && !cached.contains(m) { model = None; }
            }
            // the thread rollout lives in the login directory where the thread started: on an account change we move it; nowhere at all (a
            // thread without turns) — TUI without resume
            let mut m2 = meta.clone();
            if let Some(tid) = &meta.provider_session_id {
                if !crate::codex::bring_rollout(&env, tid) { tracing::warn!("session {}: rollout треда {tid} не найден — TUI без resume", &meta.id[..8]); m2.provider_session_id = None; }
            }
            crate::codex::tui_args(&m2, gate.as_ref(), model.as_deref(), bypass)
        } else if meta.agent == "claude" {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            let cfg = env.get("CLAUDE_CONFIG_DIR").cloned().unwrap_or_else(|| format!("{home}/.claude"));
            let pid = meta.provider_session_id.clone().unwrap_or_else(|| meta.id.clone());
            let mut a = vec!["claude".to_string()];
            if claude_transcript(&cfg, &meta.cwd, &pid).exists() { a.push("--resume".into()); a.push(pid); } else { a.push("--session-id".into()); a.push(meta.id.clone()); }
            if let Some(m) = &meta.model { a.push("--model".into()); a.push(if meta.agent == "codex" { m.clone() } else { self.claude_model_for(m) }); }
            match (bypass, &meta.permission_mode) {
                (true, _) => { a.push("--permission-mode".into()); a.push("bypassPermissions".into()); }
                (false, Some(p)) => { a.push("--permission-mode".into()); a.push(p.clone()); }
                _ => {}
            }
            a
        } else if let Some(engine) = crate::engines::get(&meta.agent).filter(|e| e.protocol == "acp" || e.protocol == "agy") {
            // the engine's native TUI in the same environment; pi — on the same session file (the pi-acp map), Gemini — a new
            // conversation (its --resume accepts only an index), the approval mode — from the session mode
            let mut a: Vec<String> = engine.tui.iter().map(|s| s.to_string()).collect();
            if engine.id == "pi" {
                if let Some(f) = meta.provider_session_id.as_deref().and_then(crate::acp::pi_session_file) { a.push("--session".into()); a.push(f); }
                if let Some(m) = &meta.model { a.push("--model".into()); a.push(m.clone()); }
            } else if engine.id == "gemini" {
                if let Some(m) = &meta.model { a.push("--model".into()); a.push(m.clone()); }
                let mode = if bypass { Some("yolo".to_string()) } else { meta.permission_mode.as_deref().and_then(|m| crate::acp::map_mode("gemini", m, &[])) };
                if let Some(m) = mode { a.push("--approval-mode".into()); a.push(if m == "autoEdit" { "auto_edit".to_string() } else { m }); }
            } else if engine.id == "omp" {
                // the same model and the same session (omp --resume by the ACP session id; omp sessions live in
                // ~/.omp/agent/sessions)
                crate::acp::omp_ensure_gate(&env).await;
                if let Some(m) = &meta.model { a.push("--model".into()); a.push(if m.contains('/') { m.clone() } else { format!("agentverse-gate/{m}") }); }
                if let Some(sid) = &meta.provider_session_id { a.push("--resume".into()); a.push(sid.clone()); }
                if bypass { a.push("--auto-approve".into()); }
            } else if engine.id == "antigravity" {
                // the same model (through the gate — under gemini-api://) and the same conversation: agy remembers
                // it by id (--conversation)
                let gate = crate::acp::antigravity_ensure_gate(&env);
                if let Some(m) = &meta.model { a.push("--model".into()); a.push(crate::acp::antigravity_model(m, gate)); }
                if let Some(e) = meta.effort.as_deref().filter(|e| crate::agy::EFFORTS.contains(e)) { a.push("--effort".into()); a.push(e.to_string()); }
                if bypass { a.push("--dangerously-skip-permissions".into()); } else { a.extend(crate::agy::mode_flags(meta.permission_mode.as_deref())); }
                if let Some(c) = &meta.provider_session_id { a.push("--conversation".into()); a.push(c.clone()); }
            }
            a
        } else { bail!("агент {} пока не поддерживается (есть: {})", meta.agent, crate::engines::ENGINES.iter().map(|e| e.id).collect::<Vec<_>>().join(", ")) };
        let mut prefix: Vec<String> = Vec::new();
        // Gemini: the CLI's own gateway mode — the gate address from GEMINI_BASE_URL; no
        // browser
        let mut tui_env: Vec<(String, String)> = Vec::new();
        if meta.agent == "gemini" {
            if !env.contains_key("GOOGLE_GEMINI_BASE_URL") { if let Some(b) = env.get("GEMINI_BASE_URL") { tui_env.push(("GOOGLE_GEMINI_BASE_URL".into(), b.clone())); } }
            tui_env.push(("NO_BROWSER".into(), "true".into()));
            crate::acp::gemini_ensure_trust(&meta.cwd);
        } else if meta.agent == "antigravity" {
            tui_env.extend(crate::acp::antigravity_env(&env));
        }
        if !meta.unset.is_empty() || !meta.env.is_empty() || !tui_env.is_empty() {
            prefix.push("env".into());
            for u in &meta.unset { prefix.push("-u".into()); prefix.push(u.clone()); }
            let mut kv: Vec<_> = meta.env.iter().collect();
            kv.sort();
            for (k, v) in kv { prefix.push(format!("{k}={v}")); }
            for (k, v) in &tui_env { if !meta.env.contains_key(k) { prefix.push(format!("{k}={v}")); } }
        }
        let cmdline = format!("{}exec {}", crate::engines::PATH_PREFIX, prefix.iter().chain(args.iter()).map(|a| shell_quote(a)).collect::<Vec<_>>().join(" "));
        let mut penv = HashMap::new();
        penv.insert("CLOUDOS_SESSION".to_string(), meta.id.clone());
        Ok((vec!["bash".into(), "-lic".into(), cmdline], penv))
    }

    /// Events from the provider journal — into our journal (with their own time), not counted in the
    /// expense ledger.
    pub(crate) fn import_events(&self, events: Vec<Value>) {
        for ev in events { self.emit(ev); }
    }

    /// Provider journal path: Claude — from the config (the account's CLAUDE_CONFIG_DIR or ~/.claude), cwd and id; Codex —
    /// the rollout found by thread id in CODEX_HOME/sessions (remembered in meta).
    pub(crate) fn transcript_path(&self) -> Option<PathBuf> {
        let meta = self.meta.lock().unwrap().clone();
        if let Some(p) = &meta.transcript { return Some(PathBuf::from(p)); }
        let env = crate::codex::effective_env(&meta);
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let pid = meta.provider_session_id.clone().unwrap_or_else(|| meta.id.clone());
        // for ACP engines the provider journal is not read (their history lives inside the agent, we
        // keep our own journal)
        if crate::engines::is_acp(&meta.agent) || crate::engines::is_agy(&meta.agent) { return None; }
        if meta.agent == "codex" {
            let root = env.get("CODEX_HOME").cloned().unwrap_or_else(|| format!("{home}/.codex"));
            let path = crate::import::find_rollout(std::path::Path::new(&root), &pid)?;
            self.meta.lock().unwrap().transcript = Some(path.to_string_lossy().into_owned());
            self.save_meta();
            Some(path)
        } else {
            let cfg = env.get("CLAUDE_CONFIG_DIR").cloned().unwrap_or_else(|| format!("{home}/.claude"));
            Some(claude_transcript(&cfg, &meta.cwd, &pid))
        }
    }

    /// Mark: everything now in the provider journal is already reflected in ours (at start and exit of our own
    /// process — its turn gets into the journal from stream-json / app-server, not from the file).
    pub(crate) fn mark_synced(&self) {
        let n = self.transcript_path().map(|p| crate::import::count_lines(&p)).unwrap_or(0);
        self.meta.lock().unwrap().synced_lines = Some(n);
        self.save_meta();
    }

    /// Pull in lines appended to the provider journal outside our process (the native TUI from the CLI tab, a
    /// terminal, code-server). A record without a mark (old sessions) simply gets a mark at the current size —
    /// without re-importing what is already in the journal. Returns the number of events added.
    pub(crate) fn sync_transcript(&self) -> usize {
        if self.alive() { return 0 }
        let Some(path) = self.transcript_path() else { return 0 };
        // picked up by the old parser and never had turns under agentd — rebuild the journal
        // entirely
        let stale = { let m = self.meta.lock().unwrap(); m.imported && m.import_v.unwrap_or(0) < crate::import::VERSION && self.usage.lock().unwrap().turns == 0 };
        if stale {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let agent = self.meta.lock().unwrap().agent.clone();
                let parsed = if agent == "codex" { crate::import::parse_codex(&text, 0) } else { crate::import::parse_claude(&text, 0) };
                if self.journal.lock().unwrap().reset(&self.dir).is_ok() {
                    let n = parsed.events.len();
                    self.import_events(parsed.events);
                    { let mut m = self.meta.lock().unwrap(); m.synced_lines = Some(parsed.lines); m.import_v = Some(crate::import::VERSION); if let Some(t) = parsed.title { m.title = t; } }
                    self.save_meta();
                    tracing::info!("session {}: журнал пересобран разборщиком v{} — {n} событий", &self.id()[..8], crate::import::VERSION);
                    return n;
                }
            }
        }
        // records picked up before usage_base existed: usage from the provider journal — once
        let needs_base = { let m = self.meta.lock().unwrap(); m.imported && m.usage_base.is_none() };
        if needs_base {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let agent = self.meta.lock().unwrap().agent.clone();
                let parsed = if agent == "codex" { crate::import::parse_codex(&text, 0) } else { crate::import::parse_claude(&text, 0) };
                let base = parsed.usage.unwrap_or_default();
                { let mut u = self.usage.lock().unwrap(); u.input += base.input; u.output += base.output; u.cache_read += base.cache_read; u.cache_create += base.cache_create; u.cost_usd += base.cost_usd; }
                self.meta.lock().unwrap().usage_base = Some(base);
                self.save_meta();
            }
        }
        let Ok(md) = std::fs::metadata(&path) else { return 0 };
        let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        { let mut seen = self.transcript_seen.lock().unwrap(); if *seen == (md.len(), mtime) { return 0 } *seen = (md.len(), mtime); }
        let (agent, synced) = { let m = self.meta.lock().unwrap(); (m.agent.clone(), m.synced_lines) };
        let Ok(text) = std::fs::read_to_string(&path) else { return 0 };
        let total = crate::import::complete_lines(&text).count() as u64;
        let Some(skip) = synced else { self.meta.lock().unwrap().synced_lines = Some(total); self.save_meta(); return 0 };
        if total <= skip { return 0 }
        let parsed = if agent == "codex" { crate::import::parse_codex(&text, skip) } else { crate::import::parse_claude(&text, skip) };
        let known: std::collections::HashSet<String> = self.journal.lock().unwrap().events.iter().filter_map(|e| e.get("uuid").and_then(|u| u.as_str()).map(str::to_string)).collect();
        let events: Vec<Value> = parsed.events.into_iter().filter(|e| e.get("uuid").and_then(|u| u.as_str()).map(|u| !known.contains(u)).unwrap_or(true)).collect();
        let n = events.len();
        self.import_events(events);
        {
            let mut m = self.meta.lock().unwrap();
            m.synced_lines = Some(total);
            if let Some(t) = parsed.title { m.title = t; }
            if m.model.is_none() { m.model = parsed.model; }
        }
        self.save_meta();
        n
    }

    /// Stop the process: SIGTERM to the group, SIGKILL after 5 s; the journal stays, the session can be
    /// resumed.
    pub fn stop(&self) -> Result<()> {
        let Some(pid) = *self.pid.lock().unwrap() else { bail!("процесс не запущен") };
        let pid = pid as i32;
        // SAFETY: kill(2); a negative pid is the process group we created via process_group(0).
        unsafe { libc::kill(-pid, libc::SIGTERM) };
        let mut st = self.status.subscribe();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            if !matches!(*st.borrow_and_update(), Status::Exited | Status::Error) {
                // SAFETY: see above.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
        });
        Ok(())
    }
}

/// Workspace policy for Claude Code: `/etc/claude-code/managed-settings.json` with `permissions.disableBypassPermissionsMode`
/// = "disable" (placed by the core at the owner's decision, see templates/incus/main.tf) forbids the bypass mode — both in the
/// chat and in the TUI.
pub fn bypass_allowed() -> bool {
    std::fs::read_to_string("/etc/claude-code/managed-settings.json").ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.pointer("/permissions/disableBypassPermissionsMode").and_then(|d| d.as_str()).map(|d| d != "disable"))
        .unwrap_or(true)
}

/// Claude Code conversation journal path: `<config>/projects/<cwd with non-alphanumerics → '-'>/<id>.jsonl`. Rules
/// from `updatedPermissions` ({type: addRules, rules: [{toolName, ruleContent}]}) → (tool, content).
pub fn permission_rules(perms: Option<&Value>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for p in perms.and_then(|p| p.as_array()).cloned().unwrap_or_default() {
        if p.get("type").and_then(|t| t.as_str()) != Some("addRules") || p.get("behavior").and_then(|b| b.as_str()).unwrap_or("allow") != "allow" { continue }
        for r in p.get("rules").and_then(|r| r.as_array()).cloned().unwrap_or_default() {
            let tool = r.get("toolName").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let content = r.get("ruleContent").and_then(|c| c.as_str()).unwrap_or("").to_string();
            if !tool.is_empty() { out.push((tool, content)); }
        }
    }
    out
}

/// Does the request match the rule: the whole tool, exact content, or a `cmd:*` prefix for Bash.
pub fn rule_matches(rules: &[(String, String)], tool: &str, input: &Value) -> bool {
    let command = input.get("command").and_then(|c| c.as_str()).unwrap_or("").trim();
    rules.iter().any(|(t, c)| t == tool && (c.is_empty() || c == command || (tool == "Bash" && c.strip_suffix(":*").map(|prefix| command.starts_with(prefix.trim())).unwrap_or(false))))
}

pub fn claude_transcript(config_dir: &str, cwd: &str, id: &str) -> PathBuf {
    let enc: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    PathBuf::from(config_dir).join("projects").join(enc).join(format!("{id}.jsonl"))
}

pub struct Manager {
    dir: PathBuf,
    sessions: RwLock<HashMap<String, Arc<Session>>>,
    usage_log: Mutex<Vec<Value>>,
    /// Disk reconciliation (import.rs) — at most once per 15 s; conversations without messages are remembered by mtime so they
    /// are not read again
    last_sync: Mutex<Option<std::time::Instant>>,
    skipped: Mutex<HashMap<String, u64>>,
}

impl Manager {
    /// Loads session records from disk (all as finished — processes do not survive an agentd
    /// restart).
    pub fn open(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let mut sessions = HashMap::new();
        for e in std::fs::read_dir(&dir)?.flatten() {
            let p = e.path();
            let Ok(meta) = std::fs::read_to_string(p.join("meta.json")).map_err(anyhow::Error::from).and_then(|s| Ok(serde_json::from_str::<Meta>(&s)?)) else { continue };
            match Session::load(p, meta.clone()) {
                Ok(s) => { sessions.insert(meta.id.clone(), s); }
                Err(err) => tracing::warn!("сессия {}: {err:#}", meta.id),
            }
        }
        tracing::info!("сессий на диске: {}", sessions.len());
        Ok(Self { dir, sessions: RwLock::new(sessions), usage_log: Mutex::new(Vec::new()), last_sync: Mutex::new(None), skipped: Mutex::new(HashMap::new()) })
    }

    /// Auto-resume after a restart (slice 38, docs/agents-ux.md §4.3.11): conversations alive before the stop are brought up
    /// the same way as "Resume", at most three at a time; a system line about the restart goes into the journal.
    pub async fn resume_live(&self) -> usize {
        let mut cand: Vec<Arc<Session>> = self.sessions.read().unwrap().values().filter(|s| !s.alive() && s.was_live()).cloned().collect();
        cand.sort_by_key(|s| std::cmp::Reverse(s.info().updated_at));
        let n = cand.len();
        if n == 0 { return 0 }
        tracing::info!("автовозобновление после рестарта: {n} бесед");
        let sem = Arc::new(tokio::sync::Semaphore::new(3));
        let mut handles = Vec::new();
        for s in cand {
            let sem = sem.clone();
            handles.push(tokio::spawn(async move {
                let _p = sem.acquire().await;
                s.emit(json!({ "kind": "system", "subtype": "agentd_restart", "data": { "message": "agentd перезапущен (обновление) — беседа возобновлена" } }));
                if let Err(e) = s.spawn(true).await { tracing::warn!("сессия {}: автовозобновление: {e:#}", &s.id()[..8]); s.emit(json!({ "kind": "error", "message": format!("автовозобновление после рестарта не удалось: {e:#}") })); }
            }));
        }
        for h in handles { let _ = h.await; }
        n
    }

    /// Reconciliation with the workspace disk: Claude Code and Codex conversations started outside agentd become
    /// records with history; known sessions pull in lines appended outside our process. Returns the number of
    /// conversations picked up.
    pub fn sync_disk(&self) -> usize {
        {
            let mut last = self.last_sync.lock().unwrap();
            if last.map(|t| t.elapsed() < std::time::Duration::from_secs(15)).unwrap_or(false) { return 0 }
            *last = Some(std::time::Instant::now());
        }
        let env = crate::codex::workspace_env();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let claude_dir = PathBuf::from(env.get("CLAUDE_CONFIG_DIR").cloned().unwrap_or_else(|| format!("{home}/.claude")));
        let codex_dir = PathBuf::from(env.get("CODEX_HOME").cloned().unwrap_or_else(|| format!("{home}/.codex")));
        let (known, existing): (std::collections::HashSet<String>, Vec<Arc<Session>>) = {
            let g = self.sessions.read().unwrap();
            let mut known = std::collections::HashSet::new();
            for s in g.values() { let m = s.meta.lock().unwrap(); known.insert(m.id.clone()); if let Some(p) = &m.provider_session_id { known.insert(p.clone()); } }
            (known, g.values().cloned().collect())
        };
        let mut found = crate::import::scan_claude(&claude_dir, &known);
        found.extend(crate::import::scan_codex(&codex_dir, &known));
        let mut added = 0;
        for f in found {
            if self.skipped.lock().unwrap().get(&f.id) == Some(&f.mtime) { continue }
            match self.adopt(&f) {
                Ok(s) => { added += 1; tracing::info!("сессия {} ({}): подхвачена с диска — {}", &s.id()[..8], f.agent, f.path.display()); }
                Err(e) => { tracing::debug!("{}: {e:#}", f.path.display()); self.skipped.lock().unwrap().insert(f.id.clone(), f.mtime); }
            }
        }
        for s in existing {
            let n = s.sync_transcript();
            if n > 0 { tracing::info!("session {}: из журнала провайдера подхвачено {n} событий", &s.id()[..8]); }
        }
        added
    }

    /// An agentd record for a conversation from disk: history — into our journal, usage and model — from
    /// the provider journal.
    fn adopt(&self, f: &crate::import::Found) -> Result<Arc<Session>> {
        let text = std::fs::read_to_string(&f.path)?;
        let parsed = if f.agent == "codex" { crate::import::parse_codex(&text, 0) } else { crate::import::parse_claude(&text, 0) };
        if !parsed.events.iter().any(|e| e.get("kind").and_then(|k| k.as_str()) == Some("user")) { bail!("нет сообщений пользователя"); }
        let id = uuid::Uuid::new_v4().to_string();
        let meta = Meta {
            id: id.clone(), title: crate::import::title_of(&parsed, f.agent), label: None, limit_hit: None, agent: f.agent.to_string(), cwd: f.cwd.clone(), created_at: f.created_at,
            provider_session_id: Some(f.id.clone()), model: parsed.model.clone(), permission_mode: None, account: None, env: HashMap::new(), unset: Vec::new(),
            imported: true, transcript: Some(f.path.to_string_lossy().into_owned()), synced_lines: Some(parsed.lines), usage_base: parsed.usage, effort: None, import_v: Some(crate::import::VERSION), max_thinking_tokens: None, service_tier: None, cache_ttl: None, allowed_tools: Vec::new(), disallowed_tools: Vec::new(), resume_at: None, context_window: None
        };
        let s = Session::load(self.dir.join(&id), meta)?;
        s.import_events(parsed.events);
        s.save_meta();
        self.sessions.write().unwrap().insert(id, s.clone());
        crate::hub::touch();
        Ok(s)
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.read().unwrap().get(id).cloned()
    }

    pub fn all(&self) -> Vec<Arc<Session>> { self.sessions.read().unwrap().values().cloned().collect() }

    pub fn list(&self) -> Vec<Info> {
        let mut v: Vec<Info> = self.sessions.read().unwrap().values().map(|s| s.info()).collect();
        v.sort_by_key(|i| i.meta.created_at);
        v
    }

    pub async fn create(&self, opts: CreateOpts) -> Result<Arc<Session>> {
        let agent = opts.agent.unwrap_or_else(|| "claude".into());
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let cwd = opts.cwd.unwrap_or_else(|| home.clone());
        // a directory under home is created automatically (scratch chats live in /home/coder/chats/<id8>), a foreign
        // one must exist
        if !std::path::Path::new(&cwd).is_dir() {
            if std::path::Path::new(&cwd).starts_with(&home) { std::fs::create_dir_all(&cwd).map_err(|e| anyhow!("каталог {cwd}: {e}"))?; } else { bail!("каталог {cwd} не существует"); }
        }
        // resuming an existing record — the same journal, a new process
        if let Some(rid) = &opts.resume {
            if let Some(s) = self.get(rid) {
                if s.alive() { bail!("сессия уже запущена"); }
                if opts.reassign {
                    // account reassignment: a new process, the same journal; the limit hit is cleared — it belonged to the previous
                    // account
                    let mut m = s.meta.lock().unwrap();
                    m.account = opts.account.clone();
                    m.env = opts.env.clone();
                    m.unset = opts.unset.clone();
                    m.limit_hit = None;
                    drop(m);
                    s.save_meta();
                }
                if let Some(n) = opts.note.as_deref().filter(|n| !n.trim().is_empty()) { s.emit(json!({ "kind": "system", "subtype": "agent_note", "data": { "message": n } })); }
                s.spawn(true).await?;
                if opts.retry_last {
                    // the turn rejected by the limit was never executed: the same request goes out again
                    // on the new account
                    if let Some(content) = s.last_user_content() { if let Err(e) = s.prompt(content).await { tracing::warn!("session {}: повтор запроса после переназначения: {e:#}", &s.id()[..8]); } }
                }
                return Ok(s);
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let ctx_win = opts.context_window.or_else(|| opts.model.as_deref().and_then(crate::codex::default_context_window));
        let meta = Meta {
            id: id.clone(), title: opts.title.unwrap_or_else(|| crate::engines::title(&agent).to_string()), label: None, limit_hit: None, agent, cwd, created_at: now_ms() / 1000,
            provider_session_id: opts.resume.clone(), model: opts.model, permission_mode: opts.permission_mode,
            account: opts.account, env: opts.env, unset: opts.unset, imported: false, transcript: None, synced_lines: None, usage_base: None, effort: opts.effort, import_v: None, max_thinking_tokens: None, service_tier: None, cache_ttl: opts.cache_ttl, allowed_tools: opts.allowed_tools, disallowed_tools: opts.disallowed_tools, resume_at: None, context_window: ctx_win
        };
        let s = Session::load(self.dir.join(&id), meta)?;
        s.save_meta();
        if let Some(n) = opts.note.as_deref().filter(|n| !n.trim().is_empty()) { s.emit(json!({ "kind": "system", "subtype": "agent_note", "data": { "message": n } })); }
        s.spawn(opts.resume.is_some()).await?;
        self.sessions.write().unwrap().insert(id, s.clone());
        crate::hub::touch();
        Ok(s)
    }

    /// Conversation fork (fork.rs): a new record with a copy of the history up to `at_seq` (or whole). Claude — a
    /// transcript copy under a new provider id; Codex — app-server `thread/fork` (needs a live process, started if
    /// necessary; `at_seq` is unsupported — the whole thread is copied).
    pub async fn fork(&self, id: &str, at_seq: Option<u64>, title: Option<String>) -> Result<Arc<Session>> {
        let s = self.get(id).ok_or_else(|| anyhow!("нет сессии {id}"))?;
        let meta = s.meta.lock().unwrap().clone();
        if crate::engines::is_acp(&meta.agent) || crate::engines::is_agy(&meta.agent) { bail!("форк беседы у {} пока не поддерживается", crate::engines::title(&meta.agent)); }
        let is_codex = meta.agent == "codex";
        let new_id = uuid::Uuid::new_v4().to_string();
        let events: Vec<Value> = s.journal.lock().unwrap().events.clone();
        let (provider_id, transcript, lines) = if is_codex {
            if !s.alive() { s.spawn(true).await?; tokio::time::sleep(std::time::Duration::from_secs(2)).await; }
            let tid = s.codex.lock().unwrap().thread_id.clone().or(meta.provider_session_id.clone()).ok_or_else(|| anyhow!("у сессии нет треда Codex"))?;
            let r = s.codex_call("thread/fork", json!({ "threadId": tid, "cwd": meta.cwd })).await?;
            let new_tid = r.pointer("/thread/id").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("Codex не вернул id нового треда"))?.to_string();
            let path = r.pointer("/thread/path").and_then(|v| v.as_str()).map(str::to_string);
            let n = path.as_deref().map(|p| crate::import::count_lines(std::path::Path::new(p))).unwrap_or(0);
            (new_tid, path, n)
        } else {
            let src = s.transcript_path().filter(|p| p.is_file()).ok_or_else(|| anyhow!("у беседы ещё нет транскрипта — форкать нечего"))?;
            let anchor = match at_seq { Some(seq) => Some(crate::fork::anchor_uuid(&events, seq).ok_or_else(|| anyhow!("до события {seq} нет ответа ассистента"))?), None => None };
            let (dst, n) = crate::fork::copy_claude_transcript(&src, &new_id, anchor.as_deref())?;
            (new_id.clone(), Some(dst.to_string_lossy().into_owned()), n)
        };
        // a fork "from here" at a user message: the new conversation's journal ends before it (the anchor itself is the assistant
        // answer before it)
        let keep = if is_codex { None } else { at_seq.map(|s| s.saturating_sub(1)) };
        let dir = self.dir.join(&new_id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("events.jsonl"), crate::fork::ensure_json_lines(&events, keep))?;
        let mut m = meta.clone();
        m.id = new_id.clone(); m.provider_session_id = Some(provider_id); m.transcript = transcript; m.synced_lines = Some(lines); m.imported = false; m.created_at = now_ms() / 1000;
        m.title = title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| format!("⑂ {}", meta.title)); m.resume_at = None;
        let ns = Session::load(dir, m)?;
        ns.emit(json!({ "kind": "forked", "from": id, "at": at_seq }));
        ns.save_meta();
        self.sessions.write().unwrap().insert(new_id, ns.clone());
        Ok(ns)
    }

    /// A record brought from another workspace (transfer.rs): the journal as is, the same id, no process started.
    pub(crate) fn import(&self, meta: Meta, events: &[u8]) -> Result<Arc<Session>> {
        if self.get(&meta.id).is_some() { bail!("сессия {} уже есть", &meta.id[..8]); }
        let dir = self.dir.join(&meta.id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("events.jsonl"), events)?;
        let s = Session::load(dir, meta)?;
        s.save_meta();
        self.sessions.write().unwrap().insert(s.id(), s.clone());
        Ok(s)
    }

    /// Remove the record of a finished session along with the journal — and with the provider journal (the Claude
    /// Code file with its tool-results directory, or the Codex rollout), otherwise the conversation comes back at the
    /// next disk reconciliation.
    pub fn remove(&self, id: &str) -> Result<()> {
        let s = self.get(id).ok_or_else(|| anyhow!("нет сессии {id}"))?;
        if s.alive() { bail!("сессия ещё запущена"); }
        let transcript = s.transcript_path();
        self.sessions.write().unwrap().remove(id);
        crate::hub::touch();
        let _ = std::fs::remove_dir_all(&s.dir);
        if let Some(p) = transcript {
            let inside = p.components().any(|c| matches!(c.as_os_str().to_str(), Some("projects") | Some("sessions")));
            if inside && p.is_file() {
                let _ = std::fs::remove_file(&p);
                if p.extension().and_then(|e| e.to_str()) == Some("jsonl") { let side = p.with_extension(""); if side.is_dir() { let _ = std::fs::remove_dir_all(side); } }
            }
        }
        Ok(())
    }

    /// Workspace usage records after `since` (ms) — from the shared `usage.jsonl`, which survives session
    /// deletion.
    pub fn usage_since(&self, since_ms: u64) -> Vec<Value> {
        let _ = &self.usage_log;
        let path = self.dir.join("usage.jsonl");
        let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
        let mut out: Vec<Value> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|r| r.get("ts").and_then(|t| t.as_u64()).unwrap_or(0) > since_ms)
            .collect();
        out.sort_by_key(|r| r.get("ts").and_then(|t| t.as_u64()).unwrap_or(0));
        out
    }
}

/// Turn cost from the cumulative `total_cost_usd` of the result: the difference from the session's previous value (verified
/// on the stand: for Claude Code it is the sum for the whole conversation, grows monotonically and survives resume); a drop
/// means a new baseline.
fn cost_delta(last: &mut f64, ev: &Value) -> f64 {
    if ev.get("num_turns").and_then(|n| n.as_u64()) == Some(0) { return 0.0 }
    let total = ev.get("total_cost_usd").and_then(|c| c.as_f64()).unwrap_or(*last);
    let delta = if total >= *last { total - *last } else { total };
    *last = total;
    delta
}

fn add_usage(u: &mut Usage, ev: &Value, cost: f64) {
    // output of a local slash command (/context, /cost…) comes as a result with num_turns 0 — not a model
    // turn
    if ev.get("num_turns").and_then(|n| n.as_u64()) == Some(0) { return }
    let g = |k: &str| ev.pointer(&format!("/usage/{k}")).and_then(|x| x.as_u64()).unwrap_or(0);
    u.input += g("input_tokens");
    u.output += g("output_tokens");
    u.cache_read += g("cache_read_input_tokens");
    u.cache_create += g("cache_creation_input_tokens");
    u.cost_usd += cost;
    u.turns += 1;
}

/// Normalization for search: lower case, ё → е.
pub fn norm_text(s: &str) -> String { s.to_lowercase().replace('ё', "е") }

/// Text of an owner's or agent's message: a string or text blocks.
pub fn event_text(e: &Value) -> String {
    match e.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter(|b| b.get("type").and_then(|t| t.as_str()).map(|t| t == "text").unwrap_or(true)).filter_map(|b| b.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// A ±80-character snippet around the first occurrence of the word (search is done on a normalized copy of the
/// same length in characters).
pub fn snippet(text: &str, normed: &str, word: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let nchars: Vec<char> = normed.chars().collect();
    let wchars: Vec<char> = word.chars().collect();
    let pos = (0..nchars.len().saturating_sub(wchars.len()).saturating_add(1)).find(|&i| nchars[i..].starts_with(&wchars)).unwrap_or(0);
    let start = pos.saturating_sub(80); let end = (pos + wchars.len() + 80).min(chars.len().min(nchars.len()));
    let mut s: String = chars[start.min(chars.len())..end].iter().collect::<String>().replace('\n', " ");
    if start > 0 { s.insert(0, '…'); } if end < chars.len() { s.push('…'); }
    s
}

/// UTC time "YYYY-MM-DD HH:MM" without external
/// crates.
pub fn fmt_ts(ms: u64) -> String {
    let secs = ms / 1000; let days = (secs / 86400) as i64; let (h, m) = ((secs % 86400) / 3600, (secs % 3600) / 60);
    // civil-from-days (Howard Hinnant)
    let z = days + 719468; let era = z.div_euclid(146097); let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; let y = yoe + era * 400; let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153; let d = doy - (153 * mp + 2) / 5 + 1; let mo = if mp < 10 { mp + 3 } else { mp - 9 }; let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}")
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@".contains(c)) { s.to_string() } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

/// Lightweight event for the feed (docs/agents-ux.md §4.3.16): tool result text and string fields of the tool input longer
/// than LITE_MAX are cut to LITE_KEEP with a `truncated` flag and the full length; answer text is untouched. The full event —
/// `GET /v1/sessions/{id}/events/{seq}`.
pub const LITE_MAX: usize = 8 * 1024;
pub const LITE_KEEP: usize = 6 * 1024;
fn cut_str(s: &str) -> Option<String> {
    if s.len() <= LITE_MAX { return None }
    let mut end = LITE_KEEP; while !s.is_char_boundary(end) { end -= 1; }
    Some(s[..end].to_string())
}
pub fn lite(ev: &Value) -> Value {
    let kind = ev.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    if kind != "tool_result" && kind != "assistant" { return ev.clone() }
    let mut out = ev.clone(); let mut touched = false;
    if let Some(blocks) = out.get_mut("content").and_then(|c| c.as_array_mut()) {
        for b in blocks.iter_mut() {
            match b.get("type").and_then(|t| t.as_str()) {
                Some("tool_result") => {
                    let full = b.get("content").cloned().unwrap_or(Value::Null);
                    match full {
                        Value::String(t) => if let Some(c) = cut_str(&t) { let n = t.len(); b["content"] = json!(c); b["truncated"] = json!(true); b["full_len"] = json!(n); touched = true; },
                        Value::Array(items) => {
                            let mut items = items; let mut n = 0usize; let mut cut = false;
                            for it in items.iter_mut() {
                                if let Some(t) = it.get("text").and_then(|t| t.as_str()).map(str::to_string) { n += t.len(); if let Some(c) = cut_str(&t) { it["text"] = json!(c); cut = true; } }
                                // images (a Read of a screenshot, base64 PNG of 100–200 KB): not drawn in the feed — the data goes, the
                                // size stays
                                if it.get("type").and_then(|t| t.as_str()) == Some("image") {
                                    if let Some(d) = it.pointer("/source/data").and_then(|d| d.as_str()).map(|d| d.len()) { if d > 0 { n += d; it["source"]["data"] = json!(""); it["omitted"] = json!(true); it["bytes"] = json!(d * 3 / 4); cut = true; } }
                                }
                            }
                            if cut { b["content"] = Value::Array(items); b["truncated"] = json!(true); b["full_len"] = json!(n); touched = true; }
                        }
                        _ => {}
                    }
                }
                Some("tool_use") => {
                    let mut cutf: Vec<String> = Vec::new(); let mut total = 0usize;
                    if let Some(inp) = b.get_mut("input").and_then(|i| i.as_object_mut()) {
                        for (k, v) in inp.iter_mut() { if let Some(t) = v.as_str().map(str::to_string) { total += t.len(); if let Some(c) = cut_str(&t) { *v = json!(c); cutf.push(k.clone()); } } }
                    }
                    if !cutf.is_empty() { b["truncated"] = json!(true); b["full_len"] = json!(total); b["truncated_fields"] = json!(cutf); touched = true; }
                }
                _ => {}
            }
        }
    }
    if touched { out["lite"] = json!(true); }
    out
}

/// Does the turn error text speak of an exhausted limit (Claude: "Switch to another model … to continue", rate_limit_error, 429; gate: cooling
/// down)
pub fn is_limit_text(t: &str) -> bool {
    let l = t.to_ascii_lowercase();
    ["rate_limit", "rate limit", "usage limit", "hit your limit", "switch to another model", "cooling down", "(429)", "too many requests", "quota exceeded", "resource_exhausted"].iter().any(|k| l.contains(k))
}
/// The reverse: "2026-10-15T10:54:27Z" (or with fractions/offset +hh:mm) → epoch
/// seconds
pub fn epoch_of_rfc3339(s: &str) -> Option<u64> {
    let s = s.trim(); if s.len() < 19 { return None }
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d, hh, mm, ss) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) { return None }
    // days from civil (Howard Hinnant)
    let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y2.div_euclid(400); let yoe = y2 - era * 400; let doy = (153 * m2 + 2) / 5 + d - 1; let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let mut t = days * 86400 + hh * 3600 + mm * 60 + ss;
    // a ±hh:mm offset in the tail (after fractional
    // seconds)
    let tail = &s[19..];
    if let Some(pos) = tail.rfind(['+', '-']) { if tail.len() >= pos + 6 { if let (Some(oh), Some(om)) = (tail.get(pos + 1..pos + 3).and_then(|x| x.parse::<i64>().ok()), tail.get(pos + 4..pos + 6).and_then(|x| x.parse::<i64>().ok())) { let off = oh * 3600 + om * 60; t = if &tail[pos..pos + 1] == "+" { t - off } else { t + off }; } } }
    if t < 0 { None } else { Some(t as u64) }
}
pub fn rfc3339(secs: u64) -> String {
    // without chrono: UTC from seconds (as in cloudd::limits), enough for "until
    // 15:00"
    let days = secs / 86400; let rem = secs % 86400; let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (mut y, mut mo, mut d) = (1970i64, 1u64, 1u64);
    let mut left = days as i64;
    loop { let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0; let yd = if leap { 366 } else { 365 }; if left < yd { break } left -= yd; y += 1; }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let ml = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for (i, l) in ml.iter().enumerate() { if left < *l { mo = i as u64 + 1; d = left as u64 + 1; break } left -= *l; }
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    #[test]
    fn lite_trims_tool_results_and_inputs_only() {
        let big = "x".repeat(20_000);
        let ev = serde_json::json!({ "kind": "tool_result", "seq": 5, "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": big }] });
        let l = super::lite(&ev);
        let b = &l["content"][0];
        assert_eq!(b["truncated"], true); assert_eq!(b["full_len"], 20_000); assert_eq!(b["content"].as_str().unwrap().len(), super::LITE_KEEP); assert_eq!(l["lite"], true);
        let ev2 = serde_json::json!({ "kind": "assistant", "seq": 6, "content": [{ "type": "text", "text": big }, { "type": "tool_use", "id": "t2", "name": "Write", "input": { "file_path": "/a", "content": big } }] });
        let l2 = super::lite(&ev2);
        assert_eq!(l2["content"][0]["text"].as_str().unwrap().len(), 20_000);
        assert_eq!(l2["content"][1]["truncated_fields"], serde_json::json!(["content"])); assert_eq!(l2["content"][1]["input"]["file_path"], "/a");
        let small = serde_json::json!({ "kind": "tool_result", "seq": 7, "content": [{ "type": "tool_result", "content": "ok" }] });
        assert_eq!(super::lite(&small), small);
        let img = serde_json::json!({ "kind": "tool_result", "seq": 8, "content": [{ "type": "tool_result", "content": [{ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "A".repeat(120_000) } }] }] });
        let li = super::lite(&img); let it = &li["content"][0]["content"][0];
        assert_eq!(it["source"]["data"], ""); assert_eq!(it["omitted"], true); assert_eq!(li["content"][0]["truncated"], true); assert!(li.to_string().len() < 1000);
    }
    #[test]
    fn limit_text_and_rfc3339() {
        assert!(super::is_limit_text("API Error: Request rejected (429) · All credentials for model claude-fable-5-1 are cooling down"));
        assert!(super::is_limit_text("You've hit your limit · resets 3pm. Switch to another model to continue."));
        assert!(!super::is_limit_text("Готово: 3 файла, тесты зелёные"));
        assert_eq!(super::rfc3339(1790578200), "2026-09-28T06:50:00Z");
        assert_eq!(super::rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::epoch_of_rfc3339("2026-09-28T06:50:00Z"), Some(1790578200));
        assert_eq!(super::epoch_of_rfc3339("2026-09-28T06:50:00.123+02:00"), Some(1790578200 - 7200));
        assert_eq!(super::epoch_of_rfc3339("2026-10-15T10:54:27Z").map(super::rfc3339).as_deref(), Some("2026-10-15T10:54:27Z"));
        assert_eq!(super::epoch_of_rfc3339("вчера"), None);
    }
    use super::*;
    #[test]
    fn claude_model_1m_normalization() {
        assert_eq!(normalize_claude_model("claude-fable-5-1"), "claude-fable-5-1[1m]");
        assert_eq!(normalize_claude_model("claude-fable-5-1[1m]"), "claude-fable-5-1[1m]");
        assert_eq!(normalize_claude_model("claude-opus-5-5"), "claude-opus-5-5[1m]");
        assert_eq!(normalize_claude_model("claude-sonnet-5"), "claude-sonnet-5[1m]");
        assert_eq!(normalize_claude_model("claude-haiku-4-5-20251001"), "claude-haiku-4-5-20251001");
        assert_eq!(normalize_claude_model("claude-opus-4-6"), "claude-opus-4-6");
        assert_eq!(normalize_claude_model("opus"), "opus");
        // subscription catalog: Opus/Sonnet without a 1M variant, Fable with the tag → [1m] only for Fable; API catalog: "1M context"
        // variants exist
        let sub = vec![json!({ "value": "opus", "description": "Opus 5.5 · Best for everyday, complex tasks", "resolved": "claude-opus-5-5" }), json!({ "value": "claude-fable-5-1[1m]", "description": "Fable 5.1 · Most capable", "resolved": "claude-fable-5-1" })];
        assert!(!claude_1m_listed(&sub, "claude-opus-5-5")); assert!(!claude_1m_listed(&sub, "claude-sonnet-5-5")); assert!(claude_1m_listed(&sub, "claude-fable-5-1"));
        let api = vec![json!({ "value": "opus[1m]", "description": "Opus 5.5 with 1M context · Best for everyday, complex tasks" })];
        assert!(claude_1m_listed(&api, "claude-opus-5-5")); assert!(!claude_1m_listed(&api, "claude-sonnet-5"));
        assert_eq!(fmt_ts(1_790_944_070_000), "2026-10-02 12:27");
        assert_eq!(fmt_ts(0), "1970-01-01 00:00");
        let t = "Перенести миграцию очереди на NATS, затем проверить воркеры"; let n = norm_text(t);
        assert!(snippet(t, &n, "очереди").contains("миграцию очереди"));
        assert_eq!(norm_text("Ёлка"), "елка");
    }

    #[test]
    fn permission_rules_and_matching() {
        let perms = json!([{ "type": "addRules", "behavior": "allow", "destination": "session", "rules": [{ "toolName": "Bash", "ruleContent": "git diff:*" }, { "toolName": "Read", "ruleContent": "" }] }]);
        let rules = permission_rules(Some(&perms));
        assert_eq!(rules, vec![("Bash".to_string(), "git diff:*".to_string()), ("Read".to_string(), String::new())]);
        assert!(rule_matches(&rules, "Bash", &json!({ "command": "git diff --stat" })));
        assert!(!rule_matches(&rules, "Bash", &json!({ "command": "git push" })));
        assert!(rule_matches(&rules, "Read", &json!({ "file_path": "/x" })));
        assert!(!rule_matches(&rules, "Edit", &json!({})));
        assert!(permission_rules(Some(&json!([{ "type": "addRules", "behavior": "deny", "rules": [{ "toolName": "Bash", "ruleContent": "rm:*" }] }]))).is_empty());
    }

    #[test]
    fn usage_accumulates_from_result_events() {
        // total_cost_usd is cumulative: 0.01 → 0.03 means turns of 0.01 and 0.02
        let mut u = Usage::default(); let mut last = 0.0;
        let e1 = json!({ "usage": { "input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 100, "cache_creation_input_tokens": 7 }, "total_cost_usd": 0.01 });
        let e2 = json!({ "usage": { "input_tokens": 1 }, "total_cost_usd": 0.03 });
        let d1 = cost_delta(&mut last, &e1); add_usage(&mut u, &e1, d1);
        let d2 = cost_delta(&mut last, &e2); add_usage(&mut u, &e2, d2);
        assert_eq!((u.input, u.output, u.cache_read, u.cache_create, u.turns), (11, 5, 100, 7, 2));
        assert!((u.cost_usd - 0.03).abs() < 1e-9 && (d1 - 0.01).abs() < 1e-9 && (d2 - 0.02).abs() < 1e-9);
        // a local command (num_turns 0) is not a turn and does not shift the baseline; a drop of
        // the total is a new baseline
        assert_eq!(cost_delta(&mut last, &json!({ "num_turns": 0, "total_cost_usd": 99.0 })), 0.0);
        assert!((cost_delta(&mut last, &json!({ "total_cost_usd": 0.005 })) - 0.005).abs() < 1e-9);
        // a picked-up conversation: the baseline starts from the cost-state base, the first result
        // yields only its own turn
        let mut base = 952.5; assert!((cost_delta(&mut base, &json!({ "total_cost_usd": 959.45 })) - 6.95).abs() < 1e-6);
    }

    #[test]
    fn journal_persists_and_resumes_seq() {
        let d = tempfile_dir();
        let mut j = Journal::open(&d).unwrap();
        j.append(json!({ "kind": "user" }));
        let second = j.append(json!({ "kind": "assistant" }));
        assert_eq!(second["seq"], 2);
        let j2 = Journal::open(&d).unwrap();
        assert_eq!(j2.seq, 2);
        assert_eq!(j2.events.len(), 2);
    }

    #[test]
    fn meta_public_hides_env_values() {
        let m = Meta { id: "x".into(), agent: "claude".into(), cwd: "/".into(), title: "t".into(), label: None, limit_hit: None, created_at: 0, provider_session_id: None, model: None, permission_mode: None, account: Some("acc".into()), env: HashMap::from([("ANTHROPIC_API_KEY".to_string(), "sk-secret".to_string())]), unset: vec!["ANTHROPIC_BASE_URL".into()], imported: false, transcript: None, synced_lines: None, usage_base: None, effort: None, import_v: None, max_thinking_tokens: None, service_tier: None, cache_ttl: None, allowed_tools: Vec::new(), disallowed_tools: Vec::new(), resume_at: None, context_window: None };
        let p = m.public();
        assert_eq!(p.env.get("ANTHROPIC_API_KEY").map(String::as_str), Some("•••"));
        assert_eq!(p.unset, vec!["ANTHROPIC_BASE_URL"]);
        assert!(!serde_json::to_string(&p).unwrap().contains("sk-secret"));
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("--session-id"), "--session-id");
        assert_eq!(shell_quote("a b'c"), "'a b'\\''c'");
    }

    fn tempfile_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("agentd-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
