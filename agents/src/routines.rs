//! Agents (employees), routines and runs (ADR-006, stage 4; docs/agents-ux.md §7). The model follows BridgeMind One: an
//! agent is a named employee with a brief, persistent memory and access to its workspace; a routine is its task on a
//! schedule, a webhook or a core event; a run is a regular agentd session in the agent's workspace that the service
//! watches to the end (budget, timeout, result, memory, notification). Storage: kv `agents` and `routines` (JSON arrays),
//! the `runs` table (store.rs). The scheduler is a background task with a 15 s tick: cron is checked against the host time
//! minute and never fires twice, core events are read from the `/api/events` feed from the service start, "hanging" runs
//! after a restart are watched to the end.
use crate::{App, ApiError};
use anyhow::{anyhow, bail, Result};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{Datelike, Local, NaiveDateTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub const TICK_SECS: u64 = 15;
const WATCH_SECS: u64 = 10;

/// What the run engine remembers in the process, not in the store: the store may refuse a write (a full disk), and none
/// of these may then turn into a run started again and again.
#[derive(Default)]
pub struct RunState {
    /// Routines with a launch in flight — between the overlap check and the `running` row of the run. The check and
    /// this mark are one step with no await in between: two webhooks a second apart used to pass the overlap rule
    /// together.
    launching: std::sync::Mutex<std::collections::HashSet<String>>,
    /// When a webhook last asked for a routine (the rate rule, kept here for the same reason).
    hook_at: std::sync::Mutex<HashMap<String, i64>>,
    /// Retries this process has attempted: run → the retry time it was attempted for.
    retried: std::sync::Mutex<HashMap<String, i64>>,
    /// The last schedule minute this process has handled; not set until the first tick reads it from the store.
    minute: std::sync::Mutex<Option<Option<i64>>>,
}
/// Holds a routine in `RunState::launching` for the time of one launch.
struct Launching<'a>(&'a App, String);
impl Drop for Launching<'_> { fn drop(&mut self) { self.0.runs.launching.lock().unwrap().remove(&self.1); } }
/// Pauses before the retries of a run that failed for a transient reason (minutes): three attempts after the first one.
const RETRY_AFTER_MIN: [i64; 3] = [5, 15, 30];
/// Looks (every WATCH_SECS) a run that hit a limit is left alone before it is closed as failed: the core's account pool
/// (§9.7) may take the session over — stop it, resume it on a free account and repeat the request. That takes the core
/// from a few seconds to over a minute (its own poll, the limit lookups of the pool accounts, the stop, the resume). A
/// takeover later than this is caught when the retry comes due (`retry`).
const FAILOVER_LOOKS: u32 = 9;
/// A schedule minute missed while the service was down is caught up when no more than half the schedule period has
/// passed — within these bounds (seconds).
const CATCH_UP_MIN: i64 = 120;
const CATCH_UP_MAX: i64 = 2 * 3600;
/// How far back a firing minute missed while the service was down is still looked for, to be reported: a month covers
/// monthly schedules.
const MISSED_LOOKBACK_MIN: i64 = 31 * 24 * 60;
/// Finished runs older than this are forgotten (the newest RUNS_KEEP stay whatever their age).
const RUNS_KEEP_DAYS: i64 = 90;
const RUNS_KEEP: u32 = 300;
/// A webhook may not start its routine more often than this (seconds).
const HOOK_MIN_GAP: i64 = 5;
/// The task of a run is kept for the history up to this many characters; a longer one is kept whole for its retries.
const PROMPT_KEEP: usize = 20_000;
/// Title of the note about a skill an agent proposed (also what the event trigger must not react to).
const SKILL_NOTE: &str = "агент предложил навык";
/// The same skip of a routine is told (and recorded) again after this long.
const SKIP_TOLD_SECS: i64 = 20 * 3600;
/// Longer than this and the run is stopped
/// (`timeout`).
const MAX_RUN_SECS: i64 = 2 * 3600;
const MEMORY_DEFAULT: usize = 6000;
const SUMMARY_MAX: usize = 500;
const WEBHOOK_MAX: usize = 64 * 1024;
/// Skills in the task: no more than this in total
/// (characters)
const SKILLS_MAX: usize = 8000;
/// A mail chain between agents does not go longer than this: run → "ПОЧТА: …" (mail) →
/// run → …
pub const MAIL_MAX_HOPS: u32 = 3;
const MAIL_MAX: usize = 4000;
const SKILLS_PER_AGENT: usize = 20;

fn s_claude() -> String { "claude".into() }
fn s_ask() -> String { "ask".into() }
fn s_manual() -> String { "manual".into() }
fn s_ask_error() -> String { "ask_error".into() }
fn s_wait() -> String { "wait".into() }
fn yes() -> bool { true }
fn d_memory_budget() -> usize { MEMORY_DEFAULT }
fn now() -> i64 { chrono::Utc::now().timestamp() }
fn short(s: &str, n: usize) -> String { let t: String = s.chars().take(n).collect(); if s.chars().count() > n { t + "…" } else { t } }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentDef {
    #[serde(default)] pub id: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub brief: String,
    #[serde(default = "s_claude")] pub provider: String,
    #[serde(default)] pub model: Option<String>,
    #[serde(default)] pub account: Option<String>,
    #[serde(default)] pub workspace: String,
    /// ask — ask (questions go to "Inbox"); auto — on its own (auto for Claude, never for Codex); bypass — bypass (if the policy
    /// allows)
    #[serde(default = "s_ask")] pub mode: String,
    #[serde(default)] pub effort: Option<String>,
    /// Claude: tool rules of the run (`Bash(npm test:*)`, `Read`…) — `--allowedTools` / `--disallowedTools`; with
    /// them the "ask" mode works unattended: the allowed goes on its own, the rest waits in "Inbox"
    #[serde(default)] pub rules: Rules,
    /// Persistent memory: text the agent gets with every task and extends with "ПАМЯТЬ: …" (memory) lines
    #[serde(default)] pub memory: String,
    /// Append a short summary of every run to the
    /// memory
    #[serde(default = "yes")] pub remember: bool,
    /// Workplace: a subdirectory of the workspace home (empty — the whole home); runs go
    /// with this cwd
    #[serde(default)] pub place: String,
    /// Mail: can receive messages from the owner and other agents and send them with "ПОЧТА: <agent>: …"
    /// (mail) lines
    #[serde(default)] pub can_mail: bool,
    /// May be assigned to routines (schedule, webhook,
    /// event)
    #[serde(default = "yes")] pub can_routine: bool,
    /// Memory budget in characters (500–20000)
    #[serde(default = "d_memory_budget")] pub memory_budget: usize,
    /// Reflection: after a successful run the memory is rewritten by a cheap model — facts with dates, no duplicates,
    /// within the budget
    #[serde(default)] pub reflect: bool,
    /// Skills from the service catalog (skills.rs): go into the task, for Claude also into the workspace's
    /// ~/.claude/skills
    #[serde(default)] pub skills: Vec<String>,
    /// When reflection last rewrote the memory (assist.rs)
    #[serde(default)] pub reflected_at: Option<i64>,
    #[serde(default)] pub created_at: i64,
    #[serde(default)] pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Rules {
    #[serde(default)] pub allow: Vec<String>,
    #[serde(default)] pub deny: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Trigger {
    /// schedule | webhook | event | manual
    #[serde(default = "s_manual")] pub kind: String,
    #[serde(default)] pub cron: String,
    /// kind of the core feed event (agent, app, update, workspace, project, grant, system, host); empty —
    /// any
    #[serde(default)] pub event: String,
    /// a substring in the event subject or text; empty —
    /// any
    #[serde(default)] pub contains: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Budget {
    /// 0 — no limit
    #[serde(default)] pub run_usd: f64,
    #[serde(default)] pub day_usd: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Routine {
    #[serde(default)] pub id: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub agent: String,
    #[serde(default)] pub prompt: String,
    #[serde(default)] pub trigger: Trigger,
    #[serde(default)] pub budget: Budget,
    /// ask_error — on a question and an error; always;
    /// never
    #[serde(default = "s_ask_error")] pub notify: String,
    #[serde(default = "yes")] pub enabled: bool,
    /// Permission mode of this routine's runs: empty — as the agent's; ask | auto |
    /// bypass
    #[serde(default)] pub mode: String,
    /// A run that failed for a transient reason (provider limit, 429/503, network) is retried in 5, 15 and 30 minutes.
    #[serde(default = "yes")] pub retries: bool,
    /// The previous run of this routine is still going: skip — the new one is skipped; parallel — it starts next to it.
    /// Not set (a routine saved before this rule appeared) — by the trigger, see `overlap_of`.
    #[serde(default)] pub overlap: String,
    /// The agent asks for a permission and nobody is there to answer: wait — the question waits in "Inbox"; deny — the
    /// service refuses it and the agent goes on without that action.
    #[serde(default = "s_wait")] pub on_question: String,
    /// webhook secret: POST /agents/api/hooks/<token>
    #[serde(default)] pub token: String,
    #[serde(default)] pub created_at: i64,
    #[serde(default)] pub updated_at: i64,
    #[serde(default)] pub last_run_at: Option<i64>,
    #[serde(default)] pub last_status: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Run {
    pub id: String,
    pub routine: Option<String>,
    pub routine_name: Option<String>,
    pub agent: String,
    pub agent_name: String,
    pub project: String,
    pub session: Option<String>,
    /// schedule | webhook | event | manual
    pub trigger: String,
    /// running | done | failed | budget | timeout | skipped
    pub status: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub cost_usd: f64,
    pub summary: Option<String>,
    pub reason: Option<String>,
    pub prompt: Option<String>,
    /// Mail: the sending agent (None — the owner), the source run and the chain
    /// length
    #[serde(default)] pub from_agent: Option<String>,
    #[serde(default)] pub reply_to: Option<String>,
    #[serde(default)] pub hops: u32,
    /// Which attempt this is (0 — the first), when the retry of a failed run is due, the first run of the retry chain.
    #[serde(default)] pub attempt: u32,
    #[serde(default)] pub retry_at: Option<i64>,
    #[serde(default)] pub retry_of: Option<String>,
    /// The substitutions of the run (`{payload}`, `{event}`) as JSON: a retry needs them again.
    #[serde(default)] pub extra: Option<String>,
    /// Class of a failure or a skip: provider | limit | error | lost | start | overlap | workspace | budget | missed | …
    #[serde(default)] pub kind: Option<String>,
}

/// Mail between agents: who sent it, from which run, which step of the chain it
/// is.
#[derive(Clone, Debug, Default)]
pub struct Mail { pub from_agent: Option<String>, pub reply_to: Option<String>, pub hops: u32 }

// ---------- the owner's time ----------

/// The owner's time zone: `settings.timezone`, an IANA name set from the browser. Not set or not recognised — the
/// server's own zone (UTC on a typical installation), as it was before.
pub fn owner_tz(app: &App) -> Option<chrono_tz::Tz> { crate::api::settings_value(app).get("timezone").and_then(|v| v.as_str()).and_then(|z| z.trim().parse().ok()) }

/// The wall clock of the owner at a moment: schedules, `{date}`/`{time}`, memory dates and the midnight of the daily
/// budget are all read off it.
pub fn wall(tz: Option<chrono_tz::Tz>, at: chrono::DateTime<Utc>) -> NaiveDateTime { match tz { Some(z) => at.with_timezone(&z).naive_local(), None => at.with_timezone(&Local).naive_local() } }

/// The moment of the owner's last midnight (the start of the day the daily budget counts from).
fn day_start(tz: Option<chrono_tz::Tz>, at: chrono::DateTime<Utc>) -> i64 {
    let midnight = wall(tz, at).date().and_hms_opt(0, 0, 0).unwrap_or_else(|| wall(tz, at));
    let back = |t: NaiveDateTime| match tz { Some(z) => z.from_local_datetime(&t).earliest().map(|d| d.timestamp()), None => Local.from_local_datetime(&t).earliest().map(|d| d.timestamp()) };
    // a zone that moves its clocks at midnight has no 00:00 on that day: the day begins with its first existing hour
    back(midnight).or_else(|| back(midnight + chrono::Duration::hours(1))).unwrap_or(at.timestamp() - 86400)
}

// ---------- outcome of a run ----------

/// The first HTTP error status named in a failure text ("API Error: 529 …", "(429)", "unexpected status 503 …"): three
/// digits 400–599 standing alone — not a part of a longer number, a price, a fraction, and not a port or a position
/// after a colon ("connect ETIMEDOUT 142.250.1.95:443" is a network failure, not an error 443).
fn http_status(text: &str) -> Option<u16> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() { i += 1; continue }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() { i += 1; }
        let before = if start > 0 { b[start - 1] } else { b' ' };
        if i - start == 3 && !matches!(before, b'.' | b',' | b'$' | b':') {
            if let Ok(n) = text[start..i].parse::<u16>() { if (400..600).contains(&n) { return Some(n); } }
        }
    }
    None
}

/// A failure worth a retry: the provider's limit or overload, a gateway or network error — not a mistake of the task or
/// of the request. A named HTTP status decides first: 429 is a limit, 408/425/5xx are the provider's trouble, any other
/// 4xx (an unknown model, an expired sign-in, a refused request) will fail the same way in five minutes and is not
/// retried.
pub fn transient(text: &str) -> Option<&'static str> {
    let l = text.to_lowercase();
    if ["rate_limit", "rate limit", "usage limit", "hit your limit", "switch to another model", "cooling down", "too many requests", "quota", "resource_exhausted"].iter().any(|k| l.contains(k)) { return Some("limit"); }
    if let Some(code) = http_status(&l) {
        return match code { 429 => Some("limit"), 408 | 425 | 500..=599 => Some("provider"), _ => None };
    }
    if ["overloaded", "bad gateway", "service unavailable", "gateway timeout", "no capacity", "temporarily unavailable", "connection error", "connection reset", "connection refused", "econnreset", "econnrefused", "etimedout", "socket hang up", "network error", "fetch failed", "timed out", "upstream connect", "stream disconnected", "error sending request"].iter().any(|k| l.contains(k)) { return Some("provider"); }
    None
}

/// The last `result` of the session journal: (is_error, subtype, text, seq). A turn that ended with an error leaves the
/// session "idle" like any other turn — the outcome is read here and not from the status. A journal that could not be
/// read is an error of its own, not "no result": taken for the latter, one failed request would record an errored turn
/// as done.
async fn last_result(app: &App, project: &str, sid: &str, last_seq: u64) -> Result<Option<(bool, String, String, u64)>> {
    let since = last_seq.saturating_sub(200);
    let ev = app.core_get(&format!("/projects/{project}/sessions/{sid}/events?since={since}&limit=400")).await?;
    Ok(ev.as_array().and_then(|a| a.iter().rev().find(|e| e.get("kind").and_then(|k| k.as_str()) == Some("result"))).map(|e| (
        e.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false),
        e.get("subtype").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        e.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        e.get("seq").and_then(|n| n.as_u64()).unwrap_or(0),
    )))
}

/// Cost of a session for an engine that reports none (Codex, Gemini, pi): tokens by the price catalogue. Without it a
/// run budget never stopped such an agent.
pub fn estimate_cost(prices: &Value, model: Option<&str>, usage: &Value) -> f64 {
    let Some(p) = model.and_then(|m| prices.get(crate::prices::base_id(m)).or_else(|| prices.get(m))) else { return 0.0 };
    let n = |k: &str| usage.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    let price = |k: &str| p.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    (n("input") * price("input") + n("output") * price("output") + n("cache_read") * price("cache_read") + n("cache_create") * price("cache_write")) / 1e6
}

// ---------- cron ----------

/// Five cron fields (minute hour day month weekday): `*`, `*/n`, `a`, `a-b`, `a-b/n`, comma-separated lists; weekdays 0–7 (0
/// and 7 — Sunday). As in cron, when both the day of month and the weekday are set, either of the two fires.
#[derive(Debug, Clone)]
pub struct Cron { min: Vec<bool>, hour: Vec<bool>, dom: Vec<bool>, mon: Vec<bool>, dow: Vec<bool>, dom_any: bool, dow_any: bool }

fn field(spec: &str, lo: u32, hi: u32) -> Option<Vec<bool>> {
    let mut set = vec![false; (hi + 1) as usize];
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() { return None; }
        let (range, step) = match part.split_once('/') { Some((r, s)) => (r, s.parse::<u32>().ok().filter(|s| *s > 0)?), None => (part, 1) };
        let (a, b) = if range == "*" { (lo, hi) } else if let Some((a, b)) = range.split_once('-') { (a.parse().ok()?, b.parse().ok()?) } else { let a: u32 = range.parse().ok()?; (a, if part.contains('/') { hi } else { a }) };
        if a < lo || b > hi || a > b { return None; }
        let mut v = a;
        while v <= b { set[v as usize] = true; v += step; }
    }
    Some(set)
}

impl Cron {
    pub fn parse(expr: &str) -> Option<Cron> {
        let f: Vec<&str> = expr.split_whitespace().collect();
        if f.len() != 5 { return None; }
        let mut dow = field(f[4], 0, 7)?;
        if dow[7] { dow[0] = true; }
        Some(Cron { min: field(f[0], 0, 59)?, hour: field(f[1], 0, 23)?, dom: field(f[2], 1, 31)?, mon: field(f[3], 1, 12)?, dow, dom_any: f[2] == "*", dow_any: f[4] == "*" })
    }
    pub fn matches<T: Datelike + Timelike>(&self, t: &T) -> bool {
        if !self.min[t.minute() as usize] || !self.hour[t.hour() as usize] || !self.mon[t.month() as usize] { return false; }
        let dom = self.dom[t.day() as usize];
        let dow = self.dow[t.weekday().num_days_from_sunday() as usize];
        if self.dom_any && self.dow_any { true } else if self.dom_any { dow } else if self.dow_any { dom } else { dom || dow }
    }
    /// Next firing minute after `from` (up to 60 days ahead), on the owner's wall clock.
    pub fn next_after(&self, tz: Option<chrono_tz::Tz>, from: chrono::DateTime<Utc>) -> Option<chrono::DateTime<Utc>> {
        let mut t = from.with_second(0)?.with_nanosecond(0)? + chrono::Duration::minutes(1);
        for _ in 0..(60 * 24 * 60) { if self.matches(&wall(tz, t)) { return Some(t); } t += chrono::Duration::minutes(1); }
        None
    }
    /// How long after a missed firing minute a catch-up run still makes sense: half the schedule period, within bounds.
    pub fn catch_up_secs(&self, tz: Option<chrono_tz::Tz>, fired: chrono::DateTime<Utc>) -> i64 {
        let period = self.next_after(tz, fired).map(|n| (n - fired).num_seconds()).unwrap_or(CATCH_UP_MAX * 2);
        (period / 2).clamp(CATCH_UP_MIN, CATCH_UP_MAX)
    }
}

/// Russian-language schedule label for common forms; otherwise the
/// expression itself.
pub fn describe(expr: &str) -> String {
    let f: Vec<&str> = expr.split_whitespace().collect();
    if f.len() != 5 { return expr.to_string(); }
    let hm = |h: &str, m: &str| format!("{:0>2}:{:0>2}", h, m);
    match (f[0], f[1], f[2], f[3], f[4]) {
        (m, h, "*", "*", "*") if m.parse::<u32>().is_ok() && h.parse::<u32>().is_ok() => format!("каждый день в {}", hm(h, m)),
        (m, h, "*", "*", "1-5") if m.parse::<u32>().is_ok() && h.parse::<u32>().is_ok() => format!("пн–пт в {}", hm(h, m)),
        (m, h, "*", "*", "0,6") | (m, h, "*", "*", "6,0") if m.parse::<u32>().is_ok() && h.parse::<u32>().is_ok() => format!("сб и вс в {}", hm(h, m)),
        (m, "*", "*", "*", "*") if m.parse::<u32>().is_ok() => format!("каждый час в :{:0>2}", m),
        (m, "*", "*", "*", "*") if m.starts_with("*/") => format!("каждые {} мин", &m[2..]),
        ("0", h, "*", "*", "*") if h.starts_with("*/") => format!("каждые {} ч", &h[2..]),
        _ => expr.to_string(),
    }
}

// ---------- storage ----------

fn load_agents(app: &App) -> Result<Vec<AgentDef>> { Ok(serde_json::from_value(app.store.get_kv("agents")?.unwrap_or(json!([]))).unwrap_or_default()) }
fn save_agents(app: &App, v: &[AgentDef]) -> Result<()> { app.store.put_kv("agents", &serde_json::to_value(v)?) }
fn load_routines(app: &App) -> Result<Vec<Routine>> { Ok(serde_json::from_value(app.store.get_kv("routines")?.unwrap_or(json!([]))).unwrap_or_default()) }
fn save_routines(app: &App, v: &[Routine]) -> Result<()> { app.store.put_kv("routines", &serde_json::to_value(v)?) }
/// All agents (for the skills catalog).
pub fn agents_with(app: &App) -> Result<Vec<AgentDef>> { load_agents(app) }
/// Remove a skill from every agent's list (on deletion from the
/// catalog).
pub fn agents_drop_skill(app: &App, name: &str) -> Result<()> {
    let mut all = load_agents(app)?;
    let mut changed = false;
    for a in all.iter_mut() { let n = a.skills.len(); a.skills.retain(|s| s != name); if a.skills.len() != n { a.updated_at = now(); changed = true; } }
    if changed { save_agents(app, &all)?; }
    Ok(())
}
/// Replace the agent's memory (reflection,
/// assist.rs).
pub fn agent_set_memory(app: &App, id: &str, memory: &str) -> Result<()> {
    let mut all = load_agents(app)?;
    let a = all.iter_mut().find(|a| a.id == id).ok_or_else(|| anyhow!("агент не найден: {id}"))?;
    a.memory = memory.to_string(); a.updated_at = now(); a.reflected_at = Some(now());
    save_agents(app, &all)
}
fn agent_by(app: &App, id: &str) -> Result<AgentDef> { load_agents(app)?.into_iter().find(|a| a.id == id).ok_or_else(|| anyhow!("агент не найден: {id}")) }
/// Whether a question of this routine's run is told to the owner at all: not when the routine tells nothing, and not
/// when it refuses such requests by itself.
pub fn tells_questions(app: &App, routine: &str) -> bool { routine_by(app, routine).map(|r| r.notify != "never" && r.on_question != "deny").unwrap_or(false) }
fn routine_by(app: &App, id: &str) -> Result<Routine> { load_routines(app)?.into_iter().find(|r| r.id == id).ok_or_else(|| anyhow!("рутина не найдена: {id}")) }
fn new_id(prefix: &str) -> String { format!("{prefix}_{}", uuid::Uuid::new_v4().simple().to_string()[..12].to_string()) }

/// Agent workplace: a relative path inside the workspace home without "..", absolute paths and
/// "~".
pub fn clean_place(p: &str) -> Result<String> {
    let t = p.trim().trim_matches('/').trim();
    if t.is_empty() { return Ok(String::new()); }
    if t.starts_with('~') || t.split('/').any(|c| c == ".." || c.is_empty()) || t.chars().any(|c| c.is_control() || c == '\\') { bail!("место работы — относительный путь внутри workspace, например `apps/site`"); }
    Ok(t.to_string())
}
fn validate_agent(a: &mut AgentDef) -> Result<()> {
    if a.name.trim().is_empty() { bail!("у агента нет имени"); }
    if !matches!(a.provider.as_str(), "claude" | "codex" | "gemini" | "pi") { bail!("неверный провайдер: {}", a.provider); }
    if a.workspace.trim().is_empty() { bail!("у агента нет workspace"); }
    if !matches!(a.mode.as_str(), "ask" | "auto" | "bypass") { bail!("неверный режим: {}", a.mode); }
    a.place = clean_place(&a.place)?;
    a.memory_budget = a.memory_budget.clamp(500, 20_000);
    a.skills = a.skills.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if let Some(bad) = a.skills.iter().find(|s| !crate::skills::valid_name(s)) { bail!("неверное имя навыка: «{bad}» — латиница, цифры и дефис"); }
    a.skills.dedup();
    if a.skills.len() > SKILLS_PER_AGENT { bail!("у агента не больше {SKILLS_PER_AGENT} навыков"); }
    Ok(())
}
/// The overlap rule a routine starts with: a schedule asks for the same work again — the next turn is skipped while the
/// previous one is going; a webhook or an event brings its own data each time — skipping it would lose that data.
fn default_overlap(trigger: &str) -> &'static str { if trigger == "schedule" { "skip" } else { "parallel" } }
/// The overlap rule in force for a routine.
pub fn overlap_of(r: &Routine) -> &str { match r.overlap.as_str() { "skip" | "parallel" => r.overlap.as_str(), _ => default_overlap(&r.trigger.kind) } }

fn validate_routine(r: &Routine, agent: &AgentDef) -> Result<()> {
    if r.name.trim().is_empty() { bail!("у рутины нет названия"); }
    if r.agent.trim().is_empty() { bail!("у рутины нет агента"); }
    if r.prompt.trim().is_empty() { bail!("у рутины нет задания"); }
    if !agent.can_routine { bail!("агенту «{}» запрещены рутины — включите «может стоять на рутине» в его карточке", agent.name); }
    if !matches!(r.mode.as_str(), "" | "ask" | "auto" | "bypass") { bail!("неверный режим рутины: {}", r.mode); }
    match r.trigger.kind.as_str() {
        "schedule" => { if Cron::parse(&r.trigger.cron).is_none() { bail!("неверное расписание: «{}» — пять полей cron", r.trigger.cron); } }
        "webhook" | "event" | "manual" => {}
        k => bail!("неверный триггер: {k}"),
    }
    if !matches!(r.notify.as_str(), "ask_error" | "always" | "never") { bail!("неверное правило уведомлений: {}", r.notify); }
    if !matches!(r.overlap.as_str(), "skip" | "parallel") { bail!("неверное правило наложения запусков: {}", r.overlap); }
    if !matches!(r.on_question.as_str(), "wait" | "deny") { bail!("неверное правило для вопросов агента: {}", r.on_question); }
    Ok(())
}

/// Session permission mode from the agent mode and the
/// provider.
pub fn permission_mode(provider: &str, mode: &str) -> &'static str {
    match (provider, mode) {
        ("codex", "ask") => "on-request", ("codex", "auto") => "never", ("codex", _) => "bypassPermissions",
        // Gemini CLI: the CLI's own approval modes; pi asks no permissions — no mode needed
        ("gemini", "ask") => "default", ("gemini", _) => "yolo",
        ("pi", _) => "default",
        (_, "ask") => "default", (_, "auto") => "auto", (_, _) => "bypassPermissions",
    }
}

/// Task with a preamble: brief, memory, skills, the task itself with substitutions, the rules about "ПАМЯТЬ:
/// …" (memory), "НАВЫК: …" (skill) and (if the agent has mail) "ПОЧТА: …" (mail) lines.
pub fn compose(agent: &AgentDef, prompt: &str, vars: &HashMap<&str, String>, skills: &[crate::skills::Skill]) -> String {
    let mut task = prompt.to_string();
    for (k, v) in vars { task = task.replace(&format!("{{{k}}}"), v); }
    let memory = if agent.memory.trim().is_empty() { "(пока пусто)".to_string() } else { agent.memory.trim().to_string() };
    let mut out = format!("# Агент «{}»\n{}\n\n# Память (прошлые запуски)\n{}\n", agent.name.trim(), agent.brief.trim(), memory);
    if !skills.is_empty() {
        out.push_str("\n# Навыки (как делать повторяющуюся работу)\n");
        let mut used = 0usize;
        for sk in skills {
            let block = format!("## {} — {}\n{}\n", sk.name, sk.description.trim(), sk.body.trim());
            let n = block.chars().count();
            if used + n > SKILLS_MAX { out.push_str(&format!("## {} — {}\n(текст навыка не поместился в задание; у Claude он есть как /{})\n", sk.name, sk.description.trim(), sk.name)); continue; }
            used += n; out.push_str(&block);
        }
    }
    let place = if agent.place.is_empty() { String::new() } else { format!(" Место работы — каталог `{}` (текущий каталог); за его пределы без нужды не ходи.", agent.place) };
    let mail = if agent.can_mail { " Чтобы передать часть работы или вопрос другому агенту, добавь строку «ПОЧТА: <имя агента>: <текст>» — он получит её отдельным запуском, а его ответ ляжет в твою память." } else { "" };
    out.push_str(&format!("\n# Задание\n{}\n\n---\nПравила: это автономный запуск в workspace `{}`.{place} Когда закончишь, подведи итог одним-двумя абзацами. Если что-то стоит запомнить для следующих запусков, добавь в конце отдельные строки вида «ПАМЯТЬ: …». Если нашёл повторяемый приём, опиши его строкой «НАВЫК: <имя-латиницей-через-дефис>: <когда применять и шаги>».{mail}", task.trim(), agent.workspace));
    out
}

/// "ПАМЯТЬ: …" (memory) lines from the
/// final answer.
pub fn memory_lines(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| { let l = l.trim().trim_start_matches(['-', '*', ' ']); l.strip_prefix("ПАМЯТЬ:").or_else(|| l.strip_prefix("MEMORY:")).map(|s| s.trim().to_string()) }).filter(|s| !s.is_empty()).collect()
}

/// "НАВЫК: name: text" (skill) lines from the final answer →
/// (name, text).
pub fn skill_lines(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|l| {
        let l = l.trim().trim_start_matches(['-', '*', ' ']);
        let rest = l.strip_prefix("НАВЫК:").or_else(|| l.strip_prefix("SKILL:"))?;
        let (name, body) = rest.split_once(':')?;
        let name = name.trim().trim_matches('`').to_lowercase();
        let body = body.trim().to_string();
        (crate::skills::valid_name(&name) && !body.is_empty()).then_some((name, body))
    }).collect()
}

/// "ПОЧТА: agent: text" (mail) lines from the final answer →
/// (recipient, text).
pub fn mail_lines(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|l| {
        let l = l.trim().trim_start_matches(['-', '*', ' ']);
        let rest = l.strip_prefix("ПОЧТА:").or_else(|| l.strip_prefix("MAIL:"))?;
        let (to, body) = rest.split_once(':')?;
        let to = to.trim().trim_matches(|c| c == '«' || c == '»' || c == '"' || c == '@').trim().to_string();
        let body = body.trim().to_string();
        (!to.is_empty() && !body.is_empty()).then_some((to, body))
    }).collect()
}

/// Append to the memory with a date; the old part is trimmed from the
/// front down to the `max` budget.
pub fn remember(memory: &str, date: &str, lines: &[String], max: usize) -> String {
    if lines.is_empty() { return memory.to_string(); }
    let mut m = memory.trim_end().to_string();
    for l in lines { if !m.is_empty() { m.push('\n'); } m.push_str(&format!("{date}: {l}")); }
    trim_memory(&m, max)
}

/// Trim the memory from the front to `max` characters at a line
/// boundary.
pub fn trim_memory(m: &str, max: usize) -> String {
    if m.chars().count() <= max { return m.to_string(); }
    let cut: String = m.chars().rev().take(max).collect::<Vec<_>>().into_iter().rev().collect();
    match cut.find('\n') { Some(i) => cut[i + 1..].to_string(), None => cut }
}

// ---------- launch and watching ----------

/// Launch of an agent task: a session in its workspace through the core, a `runs` record, watching in the background.
/// Returns the run record (including `skipped` with a reason — the workspace is not running, the daily budget is
/// exhausted, routines are forbidden for the agent).
pub async fn launch(app: &Arc<App>, routine: Option<&Routine>, agent: &AgentDef, prompt: &str, trigger: &str, extra: HashMap<&str, String>) -> Result<Run> {
    launch_full(app, routine, agent, prompt, trigger, extra, Mail::default(), 0, None).await
}

/// The same with the mail fields (sender, source run, chain step).
pub async fn launch_with(app: &Arc<App>, routine: Option<&Routine>, agent: &AgentDef, prompt: &str, trigger: &str, extra: HashMap<&str, String>, mail: Mail) -> Result<Run> {
    launch_full(app, routine, agent, prompt, trigger, extra, mail, 0, None).await
}

/// A run that does not start. The same skip as the routine's previous run, within SKIP_TOLD_SECS of it, adds no record
/// and no notification — a stopped workspace under an every-half-hour schedule would otherwise fill the history and the
/// phone; the first one notifies, and a skip that goes on is told again once a day. A firing missed while the service was
/// down is never merged with an earlier one: each outage is an event of its own.
async fn skipped(app: &Arc<App>, routine: Option<&Routine>, mut run: Run, kind: &str, reason: String) -> Result<Run> {
    if let Some(r) = routine.filter(|_| kind != "missed") {
        if let Some(prev) = app.store.runs(Some(&r.id), None, 1)?.into_iter().next() { if prev.status == "skipped" && prev.kind.as_deref() == Some(kind) && now() - prev.started_at < SKIP_TOLD_SECS { return Ok(prev); } }
    }
    run.status = "skipped".into(); run.kind = Some(kind.into()); run.reason = Some(reason.clone()); run.ended_at = Some(now());
    app.store.put_run(&run)?;
    if let Some(r) = routine {
        // an overlap is the routine working as told, not a problem to wake the owner with
        if r.notify != "never" && kind != "overlap" {
            notify(app, &run.project, "рутина пропущена", &format!("{} · {} — {reason}", run.project, r.name), "x").await;
            crate::channels::routine_event(app, &r.notify, &run, None).await;
        }
        if let Ok(mut rs) = load_routines(app) { if let Some(x) = rs.iter_mut().find(|x| x.id == r.id) { x.last_run_at = Some(run.started_at); x.last_status = Some(run.status.clone()); let _ = save_routines(app, &rs); } }
    }
    Ok(run)
}

/// The full form: `attempt` and `retry_of` mark a retry of a run that failed for a transient reason.
#[allow(clippy::too_many_arguments)]
pub async fn launch_full(app: &Arc<App>, routine: Option<&Routine>, agent: &AgentDef, prompt: &str, trigger: &str, extra: HashMap<&str, String>, mail: Mail, attempt: u32, retry_of: Option<String>) -> Result<Run> {
    let started = now();
    let tz = owner_tz(app);
    let at = Utc::now();
    let local = wall(tz, at);
    // what a retry needs again: the substitutions, and the task itself when the history keeps only its beginning
    let mut kept: HashMap<&str, String> = extra.clone();
    if prompt.chars().count() > PROMPT_KEEP { kept.insert("prompt", prompt.to_string()); }
    let mut run = Run {
        id: new_id("run"), routine: routine.map(|r| r.id.clone()), routine_name: routine.map(|r| r.name.clone()), agent: agent.id.clone(), agent_name: agent.name.clone(),
        project: agent.workspace.clone(), session: None, trigger: trigger.to_string(), status: "running".into(), started_at: started, ended_at: None, cost_usd: 0.0, summary: None, reason: None, prompt: Some(short(prompt, PROMPT_KEEP)),
        from_agent: mail.from_agent.clone(), reply_to: mail.reply_to.clone(), hops: mail.hops,
        attempt, retry_at: None, retry_of, extra: (!kept.is_empty()).then(|| json!(kept).to_string()), kind: None,
    };
    let mut _launching: Option<Launching> = None;
    if routine.is_some() && !agent.can_routine { return skipped(app, routine, run, "rights", format!("агенту «{}» запрещены рутины", agent.name)).await; }
    if mail.hops > MAIL_MAX_HOPS { return skipped(app, routine, run, "mail", format!("цепочка почты длиннее {MAIL_MAX_HOPS} шагов — остановлена")).await; }
    if let Some(r) = routine {
        // the previous run of the routine is still going: a frequent schedule with long runs piled sessions up in one
        // workspace. The owner's own "run now" is not held back
        if overlap_of(r) != "parallel" && trigger != "manual" {
            // a row older than the longest run plus a margin is not a run that is going: its last write was lost
            let busy = app.store.running_for_routine(&r.id, started - MAX_RUN_SECS - 600).unwrap_or(0) > 0 || !app.runs.launching.lock().unwrap().insert(r.id.clone());
            if busy { return skipped(app, routine, run, "overlap", "предыдущий запуск ещё идёт".into()).await; }
            _launching = Some(Launching(app.as_ref(), r.id.clone()));
        }
        if r.budget.day_usd > 0.0 {
            let spent = app.store.runs_cost_since(&r.id, day_start(tz, at)).unwrap_or(0.0);
            if spent >= r.budget.day_usd { return skipped(app, routine, run, "budget", format!("дневной бюджет исчерпан: ${spent:.2} из ${:.2}", r.budget.day_usd)).await; }
        }
    }
    // core spending limits (§8.3): with "stop" and an exhausted limit the run is skipped rather than failing on the first
    // turn
    if let Ok(st) = app.core_get("/agents/budgets/state").await {
        let blocked = st.as_array().and_then(|a| a.iter().find(|b| b.get("blocked").and_then(|x| x.as_bool()) == Some(true) && matches!(b.get("scope").and_then(|x| x.as_str()), Some(sc) if sc == "total" || sc == agent.workspace)));
        if let Some(b) = blocked {
            let who = if b.get("scope").and_then(|x| x.as_str()) == Some("total") { "всего".to_string() } else { agent.workspace.clone() };
            let reason = format!("дневной лимит расходов {who} исчерпан: ${:.2} из ${:.2} за 24 ч", b.get("spent").and_then(|x| x.as_f64()).unwrap_or(0.0), b.get("per_day").and_then(|x| x.as_f64()).unwrap_or(0.0));
            return skipped(app, routine, run, "limit", reason).await;
        }
    }
    if let Err(e) = app.core_get(&format!("/projects/{}/agentd", agent.workspace)).await {
        return skipped(app, routine, run, "workspace", format!("workspace {} не запущен или agentd недоступен: {e:#}", agent.workspace)).await;
    }
    let mut vars: HashMap<&str, String> = HashMap::from([("date", local.format("%Y-%m-%d").to_string()), ("time", local.format("%H:%M").to_string()), ("payload", String::new()), ("event", String::new())]);
    vars.extend(extra);
    // Agent skills go into the task for every engine. Into the workspace's ~/.claude/skills — where the owner's own
    // conversations would pick them up too — only the ones the owner wrote or accepted: a skill an agent proposed stays
    // inside that agent's tasks until then.
    let skills = crate::skills::for_agent(app, agent);
    let trusted: Vec<crate::skills::Skill> = skills.iter().filter(|s| s.source == "user").cloned().collect();
    if agent.provider == "claude" && !trusted.is_empty() { crate::skills::materialize(app, &agent.workspace, &trusted).await; }
    let text = compose(agent, prompt, &vars, &skills);
    let title = format!("{} · {}", routine.map(|r| r.name.as_str()).unwrap_or(agent.name.as_str()), local.format("%d.%m %H:%M"));
    // Permission mode: the routine's own (if set), otherwise the agent's; "bypass" goes only through the core policy
    // anyway. A message from another agent is carried out in "ask": a request cannot give more rights than its sender has.
    let mode = if mail.from_agent.is_some() { "ask" } else { routine.map(|r| r.mode.as_str()).filter(|m| !m.is_empty()).unwrap_or(agent.mode.as_str()) };
    let mut body = json!({ "agent": agent.provider, "title": title, "permission_mode": permission_mode(&agent.provider, mode) });
    if !agent.place.is_empty() { body["cwd"] = json!(format!("{HOME}/{}", agent.place)); }
    if let Some(m) = agent.model.as_deref().filter(|m| !m.is_empty()) {
        body["model"] = json!(m);
        // context window from the price catalog (Settings → Routes): Codex gets it as model_context_window
        if let Some(c) = crate::api::settings_value(app).get("prices").and_then(|p| p.get(crate::prices::base_id(m))).and_then(|r| r.get("context")).and_then(|c| c.as_u64()) { body["context_window"] = json!(c); }
    }
    if let Some(a) = agent.account.as_deref().filter(|a| !a.is_empty()) { body["account"] = json!(a); }
    if let Some(e) = agent.effort.as_deref().filter(|e| !e.is_empty()) { body["effort"] = json!(e); }
    if agent.provider == "claude" {
        let clean = |l: &Vec<String>| -> Vec<String> { l.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect() };
        let (allow, deny) = (clean(&agent.rules.allow), clean(&agent.rules.deny));
        if !allow.is_empty() { body["allowed_tools"] = json!(allow); }
        if !deny.is_empty() { body["disallowed_tools"] = json!(deny); }
    }
    // The run is written down before a session is asked for: from here the overlap rule sees it, and a store that cannot
    // be written stops the launch before anything is started — a session nobody recorded would work unwatched.
    app.store.put_run(&run)?;
    let created = match app.core_post(&format!("/projects/{}/sessions", agent.workspace), &body).await {
        Ok(v) => v,
        Err(e) => return fail_start(app, routine, run, format!("сессия не создалась: {e:#}")).await,
    };
    let sid = created.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
    run.session = Some(sid.clone());
    let session_path = format!("/projects/{}/sessions/{sid}", agent.workspace);
    // a routine bound to a channel with reports allowed: its agent is told about the box of this very session (§9.12.4)
    let text = match routine.filter(|r| crate::channels::routine_reports(app, &r.id)) {
        Some(_) => { crate::reports::open_box(app, &agent.workspace, &sid).await; format!("{text}\n\n{}", crate::reports::intro(&sid)) }
        None => text,
    };
    if let Err(e) = app.core_post(&format!("/projects/{}/sessions/{sid}/prompt", agent.workspace), &json!({ "content": text })).await {
        // the session got no task: it is stopped, not left idle in the workspace
        let _ = app.core_delete(&session_path).await;
        return fail_start(app, routine, run, format!("задание не отправилось: {e:#}")).await;
    }
    if let Err(e) = app.store.put_run(&run) { let _ = app.core_delete(&session_path).await; return Err(e); }
    tracing::info!("запуск {} ({}{}): агент «{}», workspace {}, сессия {}", run.id, trigger, if attempt > 0 { format!(", повтор {attempt}") } else { String::new() }, agent.name, agent.workspace, &sid[..8.min(sid.len())]);
    tokio::spawn(watch_boxed(app.clone(), run.id.clone()));
    Ok(run)
}

/// A run whose session did not start: the core refused it or the task could not be sent. Like a failed turn it is
/// retried when the cause is the provider's or the network's, and told to the owner when it is not — it used to be
/// written down and nothing more.
async fn fail_start(app: &Arc<App>, routine: Option<&Routine>, mut run: Run, reason: String) -> Result<Run> {
    run.status = "failed".into(); run.kind = Some("start".into()); run.ended_at = Some(now());
    if routine.map(|r| r.retries).unwrap_or(true) && transient(&reason).is_some() { if let Some(min) = RETRY_AFTER_MIN.get(run.attempt as usize) { run.retry_at = Some(now() + min * 60); } }
    run.reason = Some(short(&reason, 300));
    app.store.put_run(&run)?;
    if let Some(r) = routine {
        if let Ok(mut rs) = load_routines(app) { if let Some(x) = rs.iter_mut().find(|x| x.id == r.id) { x.last_run_at = Some(run.started_at); x.last_status = Some(run.status.clone()); let _ = save_routines(app, &rs); } }
    }
    if routine.map(|r| r.notify.as_str()).unwrap_or("ask_error") != "never" && run.retry_at.is_none() {
        let tried = if run.attempt > 0 { format!(" (после {} повт.)", run.attempt) } else { String::new() };
        notify(app, &run.project, "рутина: упала", &format!("{} · {} — {}{tried}", run.project, run.routine_name.clone().unwrap_or_else(|| run.agent_name.clone()), short(&reason, 200)), "x").await;
        crate::channels::routine_event(app, routine.map(|r| r.notify.as_str()).unwrap_or("ask_error"), &run, None).await;
    }
    Ok(run)
}

/// Watching in a box: watch → send_mail → launch_with → watch is a recursion of futures, the type is broken
/// with Box.
fn watch_boxed(app: Arc<App>, run_id: String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> { Box::pin(watch(app, run_id)) }

/// Workspace home (the cwd of runs without a "workplace"; chat.rs keeps the chat
/// directories there too).
pub const HOME: &str = "/home/coder";

/// Mail to an agent: from the owner (`from` = None) or from another agent. The recipient — by id or name (case-insensitive);
/// both must have mail enabled, mail not paused, the chain not longer than MAIL_MAX_HOPS. The message goes as a separate run
/// (trigger `mail`), the recipient's answer lands in the sending agent's memory.
pub async fn send_mail(app: &Arc<App>, from: Option<&AgentDef>, to: &str, text: &str, reply_to: Option<String>, hops: u32) -> Result<Run> {
    let text = text.trim();
    if text.is_empty() { bail!("пустое сообщение"); }
    if crate::api::settings_flag(app, "mail_paused") { bail!("почта агентов на паузе — снимите паузу в разделе «Агенты»"); }
    if let Some(f) = from { if !f.can_mail { bail!("у агента «{}» выключена почта", f.name); } }
    let key = to.trim().to_lowercase();
    let target = load_agents(app)?.into_iter().find(|a| a.id == to.trim() || a.name.trim().to_lowercase() == key).ok_or_else(|| anyhow!("адресат не найден: {to}"))?;
    if !target.can_mail { bail!("у агента «{}» выключена почта", target.name); }
    if from.map(|f| f.id == target.id).unwrap_or(false) { bail!("агент не может писать сам себе"); }
    let who = from.map(|f| format!("агента «{}»", f.name)).unwrap_or_else(|| "владельца".into());
    let prompt = format!("Сообщение от {who}:\n\n{}\n\nОтветь по существу и сделай то, о чём просят, в рамках своего брифа. Твой итоговый ответ уйдёт отправителю.", short(text, MAIL_MAX));
    launch_with(app, None, &target, &prompt, "mail", HashMap::new(), Mail { from_agent: from.map(|f| f.id.clone()), reply_to, hops }).await
}

/// Final session text: the last `result` from the journal tail (Info.last_text is cut to 200
/// characters).
async fn final_text(app: &App, project: &str, sid: &str, last_seq: u64) -> Option<String> {
    let since = last_seq.saturating_sub(60);
    let ev = app.core_get(&format!("/projects/{project}/sessions/{sid}/events?since={since}&limit=200")).await.ok()?;
    let arr = ev.as_array()?;
    arr.iter().rev().find_map(|e| if e.get("kind").and_then(|k| k.as_str()) == Some("result") { e.get("text").and_then(|t| t.as_str()).map(str::to_string) } else { None })
        .or_else(|| arr.iter().rev().find_map(|e| if e.get("kind").and_then(|k| k.as_str()) == Some("assistant") { e.get("content").and_then(|c| c.as_array()).and_then(|c| c.iter().rev().find_map(|b| b.get("text").and_then(|t| t.as_str()).map(str::to_string))) } else { None }))
}

async fn notify(app: &App, project: &str, title: &str, body: &str, tag: &str) {
    if let Err(e) = app.core_post("/agents/notify", &json!({ "project": project, "title": title, "body": body, "tag": tag })).await { tracing::debug!("уведомление: {e:#}"); }
}

/// Pause between the looks at a watched run (tests do not wait ten seconds).
fn watch_pause() -> std::time::Duration { std::time::Duration::from_millis(if cfg!(test) { 30 } else { WATCH_SECS * 1000 }) }

/// Watching the run to the end: waiting for a decision → a notification (per the rule) or a refusal, budget/timeout →
/// stop, the end of the turn → the outcome by its `result` (done or failed, a transient failure with a retry), agent
/// memory, notification, a mark in the routine.
async fn watch(app: Arc<App>, run_id: String) {
    let Some(mut run) = app.store.run(&run_id).ok().flatten() else { return };
    let Some(sid) = run.session.clone() else { return };
    let routine = run.routine.as_deref().and_then(|id| routine_by(&app, id).ok());
    let notify_rule = routine.as_ref().map(|r| r.notify.clone()).unwrap_or_else(|| "ask_error".into());
    let run_budget = routine.as_ref().map(|r| r.budget.run_usd).unwrap_or(0.0);
    let deny_questions = routine.as_ref().map(|r| r.on_question == "deny").unwrap_or(false);
    let label = format!("{} · {}", run.project, run.routine_name.clone().unwrap_or_else(|| run.agent_name.clone()));
    let prices = crate::api::settings_value(&app).get("prices").cloned().unwrap_or(json!({}));
    let settle = if cfg!(test) { 0 } else { WATCH_SECS as i64 };
    let mut told_waiting = false;
    let mut failures = 0u32;
    // the whole answer of a run that ended well, without the service lines: a channel bound with "the whole result"
    let mut answer: Option<String> = None;
    // the failed result (by its seq) the core's account pool is being given time to take over, and the looks left
    let mut limit_wait: Option<(u64, u32)> = None;
    loop {
        tokio::time::sleep(watch_pause()).await;
        let info = match app.core_get(&format!("/projects/{}/sessions/{sid}", run.project)).await {
            Ok(v) => v,
            Err(e) => {
                failures += 1;
                // "ядро 404: …" is the core's own answer; a 404 inside some other error text (a session id, a URL) is not
                if e.to_string().starts_with("ядро 404") || failures > 60 { run.status = "failed".into(); run.kind = Some("lost".into()); run.reason = Some(format!("сессия недоступна: {e:#}")); break; }
                continue;
            }
        };
        failures = 0;
        let i = info.get("info").cloned().unwrap_or(info.clone());
        let status = i.get("status").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let alive = i.get("alive").and_then(|a| a.as_bool()).unwrap_or(false);
        // an engine that reports no cost (Codex, Gemini, pi) is priced by its tokens: a run budget must stop it too
        let mut cost = i.pointer("/usage/cost_usd").and_then(|c| c.as_f64()).unwrap_or(0.0);
        if cost <= 0.0 { cost = estimate_cost(&prices, i.get("model").and_then(|m| m.as_str()), i.get("usage").unwrap_or(&Value::Null)); }
        run.cost_usd = cost;
        let elapsed = now() - run.started_at;
        if alive && status == "waiting" {
            if deny_questions {
                // nobody is there to answer: the request is refused and the agent goes on without that action
                for p in info.get("pending").and_then(|p| p.as_array()).cloned().unwrap_or_default() {
                    let Some(rid) = p.get("request_id").and_then(|r| r.as_str()) else { continue };
                    let body = json!({ "request_id": rid, "behavior": "deny", "message": "Запуск без присмотра: это действие не разрешено правилами, а ответить некому. Продолжай без него; если без него задачу не выполнить, скажи об этом в итоге." });
                    if let Err(e) = app.core_post(&format!("/projects/{}/sessions/{sid}/permission", run.project), &body).await { tracing::debug!("запуск {}: отказ на запрос {rid}: {e:#}", run.id); }
                }
            } else if !told_waiting {
                told_waiting = true;
                if notify_rule != "never" { notify(&app, &run.project, "рутина ждёт решения", &format!("{label} — ответьте во «Входящих»"), "question").await; }
            }
        }
        // the headless process does not exit by itself after a turn (the session is "ready" and waits for the next message):
        // a finished turn is the end of the run; the process is stopped, the session record stays (opens in Thread, can be
        // resumed)
        let turns = i.pointer("/usage/turns").and_then(|t| t.as_u64()).unwrap_or(0);
        let answered = i.get("last_text").and_then(|t| t.as_str()).map(|t| !t.trim().is_empty()).unwrap_or(false);
        // background tasks after the answer (background Bash, Agent, Workflow) — the
        // run is still going
        let background = i.get("background").and_then(|b| b.as_u64()).unwrap_or(0);
        let finished = alive && status == "idle" && background == 0 && (turns >= 1 || answered) && elapsed >= settle;
        // The budget and the time limit stop a run that is still going. A turn that has ended is judged by its result:
        // after a long pause of the service a run that finished well is not written down as a timeout.
        if alive && !finished && run_budget > 0.0 && cost >= run_budget {
            let _ = app.core_delete(&format!("/projects/{}/sessions/{sid}", run.project)).await;
            run.status = "budget".into(); run.kind = Some("budget".into()); run.reason = Some(format!("бюджет запуска исчерпан: ${cost:.2} из ${run_budget:.2}")); break;
        }
        if alive && !finished && elapsed > MAX_RUN_SECS {
            let _ = app.core_delete(&format!("/projects/{}/sessions/{sid}", run.project)).await;
            run.status = "timeout".into(); run.kind = Some("timeout".into()); run.reason = Some("дольше 2 часов — остановлен".into()); break;
        }
        if !alive || finished {
            let last_seq = i.get("last_seq").and_then(|s| s.as_u64()).unwrap_or(0);
            // The outcome is the turn's own `result`. A turn cut by a provider error leaves the session "idle" like any
            // other, and such a run used to be recorded as done, with the error text as its summary and in the memory.
            // The journal could not be read: nothing is decided at this look — the session is neither stopped nor judged.
            let result = match last_result(&app, &run.project, &sid, last_seq).await {
                Ok(r) => r,
                Err(e) => {
                    failures += 1;
                    if failures > 60 { run.status = "failed".into(); run.kind = Some("lost".into()); run.reason = Some(format!("журнал сессии недоступен: {e:#}")); break; }
                    continue;
                }
            };
            let errored = status == "error" || result.as_ref().map(|r| r.0).unwrap_or(false);
            // A limit: the core's account pool may take the session over (§9.7 — stop, resume on a free account, the
            // request repeated). The session is not stopped under it and the run is not closed for a few looks; if
            // the session went back to work, the watching goes on to that turn's own result.
            let limit = errored && (i.get("limit_hit").map(|h| !h.is_null()).unwrap_or(false) || result.as_ref().and_then(|r| transient(&r.2)) == Some("limit"));
            if limit {
                let seq = result.as_ref().map(|r| r.3).unwrap_or(0);
                let left = match limit_wait { Some((s, n)) if s == seq => n, _ => FAILOVER_LOOKS };
                if left > 0 { limit_wait = Some((seq, left - 1)); continue }
            }
            if finished { if let Err(e) = app.core_delete(&format!("/projects/{}/sessions/{sid}", run.project)).await { tracing::debug!("остановка сессии {sid}: {e:#}"); } }
            if errored || (result.is_none() && !finished && turns == 0) {
                let said = result.as_ref().map(|r| r.2.trim().to_string()).filter(|t| !t.is_empty()).or_else(|| i.get("last_text").and_then(|t| t.as_str()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()));
                let why = match (&said, errored) {
                    (Some(t), _) => short(&t.replace('\n', " "), 300),
                    (None, true) => result.as_ref().map(|r| r.1.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| "процесс завершился с ошибкой".into()),
                    (None, false) => "сессия остановлена до ответа".into(),
                };
                let class = if errored { transient(&why).or(limit.then_some("limit")) } else { None };
                run.status = "failed".into();
                run.kind = Some(class.unwrap_or(if errored { "error" } else { "stopped" }).into());
                // a transient failure is not the task's fault: the run is tried again after a pause
                let wanted = routine.as_ref().map(|r| r.retries).unwrap_or(true);
                if class.is_some() && wanted { if let Some(min) = RETRY_AFTER_MIN.get(run.attempt as usize) { run.retry_at = Some(now() + min * 60); } }
                run.reason = Some(why);
                break;
            }
            run.status = "done".into();
            let text = match result.map(|r| r.2).filter(|t| !t.trim().is_empty()) { Some(t) => t, None => final_text(&app, &run.project, &sid, last_seq).await.or_else(|| i.get("last_text").and_then(|t| t.as_str()).map(str::to_string)).unwrap_or_default() };
            let mem = memory_lines(&text);
            let summary: String = text.lines().filter(|l| !l.trim().is_empty() && memory_lines(l).is_empty()).collect::<Vec<_>>().join("\n");
            answer = Some(text.lines().filter(|l| memory_lines(l).is_empty()).collect::<Vec<_>>().join("\n"));
            run.summary = Some(short(summary.trim(), SUMMARY_MAX));
            // skills from the answer ("НАВЫК: name: text") — into the catalog and the agent's own list; a name that belongs to
            // another agent is left alone
            let mut new_skills: Vec<String> = Vec::new();
            for (name, body) in skill_lines(&text) {
                match crate::skills::upsert_from_agent(&app, &run.agent, &name, &body) {
                    Ok(crate::skills::Proposed::New) => { if notify_rule != "never" { notify(&app, &run.project, SKILL_NOTE, &format!("{} — «{name}»: посмотрите во вкладке «Навыки»", run.agent_name), "bulb").await; } new_skills.push(name); }
                    Ok(crate::skills::Proposed::Updated) | Ok(crate::skills::Proposed::Owners) => new_skills.push(name),
                    Ok(crate::skills::Proposed::Taken) => tracing::info!("навык «{name}» от агента {} пропущен: имя занято навыком другого агента", run.agent_name),
                    Err(e) => tracing::warn!("навык «{name}» от агента {}: {e:#}", run.agent_name),
                }
            }
            // agent memory: "ПАМЯТЬ: …" lines always, a short summary — if enabled; a mail answer — into the sender's
            // memory
            let date = wall(owner_tz(&app), Utc::now()).format("%Y-%m-%d").to_string();
            let mut reflect_agent: Option<AgentDef> = None;
            if let Ok(mut agents) = load_agents(&app) {
                if let Some(a) = agents.iter_mut().find(|a| a.id == run.agent) {
                    let mut lines = mem.clone();
                    if a.remember { if let Some(s) = run.summary.as_deref().filter(|s| !s.is_empty()) { lines.push(format!("итог «{}»: {}", run.routine_name.clone().unwrap_or_else(|| if run.trigger == "mail" { "почта".into() } else { "поручение".into() }), short(&s.replace('\n', " "), 240))); } }
                    for name in &new_skills { if !a.skills.iter().any(|s| s == name) && a.skills.len() < SKILLS_PER_AGENT { a.skills.push(name.clone()); } }
                    if !lines.is_empty() { a.memory = remember(&a.memory, &date, &lines, a.memory_budget); }
                    if !lines.is_empty() || !new_skills.is_empty() { a.updated_at = now(); }
                    if a.reflect { reflect_agent = Some(a.clone()); }
                }
                if let (Some(from), Some(s)) = (run.from_agent.as_deref(), run.summary.as_deref().filter(|s| !s.is_empty())) {
                    let who = run.agent_name.clone();
                    if let Some(sender) = agents.iter_mut().find(|a| a.id == from) {
                        let line = format!("ответ «{who}» на почту: {}", short(&s.replace('\n', " "), 300));
                        sender.memory = remember(&sender.memory, &date, &[line], sender.memory_budget); sender.updated_at = now();
                    }
                }
                let _ = save_agents(&app, &agents);
            }
            // mail from the answer ("ПОЧТА: agent: text") — separate runs for the recipients, chain
            // step +1
            if let Ok(sender) = agent_by(&app, &run.agent) {
                for (to, body) in mail_lines(&text) {
                    match send_mail(&app, Some(&sender), &to, &body, Some(run.id.clone()), run.hops + 1).await {
                        Ok(m) => tracing::info!("почта {} → «{to}»: запуск {} ({})", sender.name, m.id, m.status),
                        Err(e) => tracing::warn!("почта {} → «{to}»: {e:#}", sender.name),
                    }
                }
            }
            if let Some(a) = reflect_agent { let app2 = app.clone(); let label = run.routine_name.clone().unwrap_or_else(|| run.agent_name.clone()); let sum = run.summary.clone().unwrap_or_default(); tokio::spawn(async move { crate::assist::reflect(app2, a, label, sum).await; }); }
            break;
        }
    }
    run.ended_at = Some(now());
    for attempt in 1..=3 {
        match app.store.put_run(&run) {
            Ok(()) => break,
            Err(e) => { tracing::warn!("запуск {}: итог не записан (попытка {attempt}): {e:#}", run.id); if attempt < 3 { tokio::time::sleep(std::time::Duration::from_millis(if cfg!(test) { 10 } else { 2000 })).await; } }
        }
    }
    if let Ok(mut rs) = load_routines(&app) {
        if let Some(r) = run.routine.as_deref().and_then(|id| rs.iter_mut().find(|r| r.id == id)) { r.last_run_at = Some(run.started_at); r.last_status = Some(run.status.clone()); let _ = save_routines(&app, &rs); }
    }
    let dur = run.ended_at.unwrap_or(now()) - run.started_at;
    let body = format!("{label} · {} мин · ${:.2}{}", (dur / 60).max(0), run.cost_usd, run.summary.as_deref().filter(|s| !s.is_empty()).map(|s| format!(": {}", short(&s.replace('\n', " "), 160))).unwrap_or_default());
    let tried = if run.attempt > 0 { format!(" (после {} повт.)", run.attempt) } else { String::new() };
    match (run.status.as_str(), notify_rule.as_str()) {
        (_, "never") => {}
        ("done", "always") => notify(&app, &run.project, "рутина закончила", &body, "white_check_mark").await,
        ("done", _) => {}
        // a retry is on its way: the owner hears about the failure only when the retries are used up
        ("failed", _) if run.retry_at.is_some() => {}
        (st, _) => notify(&app, &run.project, &format!("рутина: {}", match st { "failed" => "упала", "budget" => "остановлена по бюджету", "timeout" => "остановлена по времени", s => s }), &format!("{body}{}{tried}", run.reason.as_deref().map(|r| format!(" — {r}")).unwrap_or_default()), "x").await,
    }
    crate::channels::routine_event(&app, &notify_rule, &run, answer.as_deref()).await;
    tracing::info!("запуск {}: {} (${:.2}, {} с{})", run.id, run.status, run.cost_usd, dur, run.retry_at.map(|_| ", будет повтор").unwrap_or(""));
    // tests wait for the watcher's last step (the notification above), not only for the saved status
    if cfg!(test) { let _ = app.store.put_kv(&format!("watched:{}", run.id), &json!(true)); }
}

// ---------- scheduler ----------

/// A schedule routine that is due at a tick: the minute that fires it and whether that minute is being caught up.
#[derive(Debug, PartialEq)]
pub enum Due { Fire { id: String, minute: i64, late: bool }, Missed { id: String, minute: i64 } }

/// Which schedules are due at `now`, given the last minute that was checked. Normally that is the current minute. After
/// the service was down (or a tick took long), the newest firing minute that was missed is taken instead: a run is
/// started if no more than half the schedule period has passed since (two hours at most), otherwise the miss is
/// reported — however long the service was down, within a month. Each routine comes out at most once.
pub fn due_schedules(routines: &[Routine], tz: Option<chrono_tz::Tz>, last_minute: Option<i64>, now: chrono::DateTime<Utc>) -> Vec<Due> {
    let minute = now.timestamp().div_euclid(60);
    let from = match last_minute { Some(l) if l < minute => (l + 1).max(minute - MISSED_LOOKBACK_MIN), Some(_) => return Vec::new(), None => minute };
    let mut out = Vec::new();
    for r in routines.iter().filter(|r| r.enabled && r.trigger.kind == "schedule") {
        let Some(c) = Cron::parse(&r.trigger.cron) else { continue };
        let at = |m: i64| Utc.timestamp_opt(m * 60, 0).single();
        let Some(hit) = (from..=minute).rev().find(|m| at(*m).map(|t| c.matches(&wall(tz, t))).unwrap_or(false)) else { continue };
        if hit == minute { out.push(Due::Fire { id: r.id.clone(), minute: hit, late: false }); continue }
        let fired = at(hit).unwrap_or(now);
        if (now - fired).num_seconds() <= c.catch_up_secs(tz, fired) { out.push(Due::Fire { id: r.id.clone(), minute: hit, late: true }); } else { out.push(Due::Missed { id: r.id.clone(), minute: hit }); }
    }
    out
}

/// Retries that are due: a failed run with a transient cause is started again with the same task and substitutions. The
/// retry time is cleared after the attempt, not before it: a service restart in between repeats the retry instead of
/// losing it without a word (the overlap rule of the routine keeps a repeated start from doubling the work). One process
/// attempts a given retry time once, even if the store could not forget it.
async fn retries(app: &Arc<App>) {
    let due = match app.store.due_retries(now()) { Ok(d) => d, Err(e) => { tracing::warn!("повторы: {e:#}"); return } };
    for r in due {
        let at = r.retry_at.unwrap_or(0);
        if app.runs.retried.lock().unwrap().insert(r.id.clone(), at) == Some(at) { let _ = app.store.clear_retry(&r.id); continue }
        if retry(app, &r).await { continue }
        if let Err(e) = app.store.clear_retry(&r.id) { tracing::warn!("повтор запуска {}: время повтора не снято: {e:#}", r.id); }
    }
}

/// The failure of a run was not told because a retry was planned, and the retry will not start: it is told now.
async fn withheld(app: &App, r: &Run, routine: Option<&Routine>, why: &str) {
    if routine.map(|x| x.notify.as_str()).unwrap_or("ask_error") == "never" { return }
    let label = format!("{} · {}", r.project, r.routine_name.clone().unwrap_or_else(|| r.agent_name.clone()));
    notify(app, &r.project, "рутина: упала", &format!("{label} — {}; повтора не будет: {why}", r.reason.clone().unwrap_or_default()), "x").await;
}

/// One retry. Returns true when the run itself was taken back to watching instead of a new one being started.
async fn retry(app: &Arc<App>, r: &Run) -> bool {
    // the agent is gone (its routines with it), or the routine was deleted: nobody is left to tell
    let Ok(agent) = agent_by(app, &r.agent) else { return false };
    let routine = r.routine.as_deref().and_then(|id| routine_by(app, id).ok());
    if r.routine.is_some() {
        match &routine {
            None => return false,
            Some(x) if !x.enabled => { withheld(app, r, Some(x), "рутина выключена").await; return false }
            _ => {}
        }
    }
    if r.trigger == "mail" && (crate::api::settings_flag(app, "mail_paused") || !agent.can_mail) { withheld(app, r, None, "почта агентов выключена").await; return false }
    // The core's account pool may have taken the failed session over after the watcher gave up on it (§9.7): the session
    // is alive again and the request was repeated there. Starting the task anew would do the work twice — the run is
    // taken back to watching, to that session's own end.
    if let Some(sid) = r.session.as_deref() {
        if let Ok(info) = app.core_get(&format!("/projects/{}/sessions/{sid}", r.project)).await {
            if info.pointer("/info/alive").and_then(|a| a.as_bool()).unwrap_or(false) {
                let again = Run { status: "running".into(), ended_at: None, reason: None, kind: None, retry_at: None, ..r.clone() };
                if app.store.put_run(&again).is_ok() {
                    tracing::info!("запуск {}: сессия снова работает — повтор не нужен, запуск досматривается", r.id);
                    tokio::spawn(watch_boxed(app.clone(), r.id.clone()));
                    return true;
                }
            }
        }
    }
    let saved: HashMap<String, String> = r.extra.as_deref().and_then(|e| serde_json::from_str(e).ok()).unwrap_or_default();
    // the task as it was given: whole from `extra` when the history keeps only its beginning
    let prompt = saved.get("prompt").cloned().or_else(|| r.prompt.clone()).or_else(|| routine.as_ref().map(|x| x.prompt.clone())).unwrap_or_default();
    let mut extra: HashMap<&str, String> = HashMap::new();
    for k in ["payload", "event"] { if let Some(v) = saved.get(k) { extra.insert(k, v.clone()); } }
    let root = r.retry_of.clone().unwrap_or_else(|| r.id.clone());
    let mail = Mail { from_agent: r.from_agent.clone(), reply_to: r.reply_to.clone(), hops: r.hops };
    match launch_full(app, routine.as_ref(), &agent, &prompt, &r.trigger, extra, mail, r.attempt + 1, Some(root)).await {
        Ok(n) => tracing::info!("повтор {} запуска {}: {} ({})", r.attempt + 1, r.id, n.id, n.status),
        Err(e) => { tracing::warn!("повтор запуска {}: {e:#}", r.id); withheld(app, r, routine.as_ref(), &format!("{e:#}")).await; }
    }
    false
}

/// One pass over the schedules: start what is due, report what was missed for good, remember the minute. The handled
/// minute is marked in the process before anything is started and only mirrored into the store afterwards: with a store
/// that refuses writes, a minute read from it anew on every tick would start the same routine every fifteen seconds.
/// The store's copy is what a restarted service continues from (a tick cut by the restart is done again; the overlap
/// rule keeps its runs from doubling).
async fn schedule_tick(app: &Arc<App>, routines: &[Routine], at: chrono::DateTime<Utc>) {
    let tz = owner_tz(app);
    let minute = at.timestamp().div_euclid(60);
    let last = {
        let mut seen = app.runs.minute.lock().unwrap();
        let last = match *seen { Some(l) => l, None => app.store.get_kv("scheduler").ok().flatten().and_then(|v| v.get("minute").and_then(|m| m.as_i64())) };
        // nothing new, or the clock was set back: wait until it passes the handled minute again
        if last.map(|l| l >= minute).unwrap_or(false) { *seen = Some(last); return }
        *seen = Some(Some(minute));
        last
    };
    for due in due_schedules(routines, tz, last, at) {
        let (id, m, late, missed) = match due { Due::Fire { id, minute, late } => (id, minute, late, false), Due::Missed { id, minute } => (id, minute, false, true) };
        let Some(r) = routines.iter().find(|r| r.id == id) else { continue };
        let Ok(a) = agent_by(app, &r.agent) else { continue };
        let planned = Utc.timestamp_opt(m * 60, 0).single().map(|t| wall(tz, t).format("%H:%M").to_string()).unwrap_or_default();
        if missed {
            let stub = Run { id: new_id("run"), routine: Some(r.id.clone()), routine_name: Some(r.name.clone()), agent: a.id.clone(), agent_name: a.name.clone(), project: a.workspace.clone(), trigger: "schedule".into(), started_at: now(), ..Default::default() };
            if let Err(e) = skipped(app, Some(r), stub, "missed", format!("сервис не работал в назначенное время ({planned})")).await { tracing::warn!("рутина «{}»: {e:#}", r.name); }
            continue;
        }
        if late { tracing::info!("рутина «{}»: догоняющий запуск за {planned}", r.name); }
        if let Err(e) = launch(app, Some(r), &a, &r.prompt, "schedule", HashMap::new()).await { tracing::warn!("рутина «{}»: {e:#}", r.name); }
    }
    if let Err(e) = app.store.put_kv("scheduler", &json!({ "minute": minute })) { tracing::warn!("планировщик: минута не записана: {e:#}"); }
}

/// Background task: watch the "hanging" runs to the end, then the tick — retries, cron by minutes and core
/// feed events.
/// The runs the service was watching when it stopped: watching goes on. A launch cut before its session existed has
/// nothing to watch — left "running" it would hold its routine back as an overlap for ever — and is closed as failed.
fn resume_runs(app: &Arc<App>) {
    let Ok(hanging) = app.store.running_runs() else { return };
    for mut r in hanging {
        if r.session.is_some() { tokio::spawn(watch_boxed(app.clone(), r.id.clone())); continue }
        r.status = "failed".into(); r.kind = Some("start".into()); r.reason = Some("запуск прерван перезапуском сервиса".into()); r.ended_at = Some(now());
        if let Err(e) = app.store.put_run(&r) { tracing::warn!("запуск {}: {e:#}", r.id); }
    }
}

pub fn start(app: Arc<App>) {
    tokio::spawn(async move {
        resume_runs(&app);
        let mut pruned_at = 0i64;
        // core events: only those newer than the start
        // moment
        let mut last_event_at = app.core_get("/events").await.ok().and_then(|v| v.as_array().and_then(|a| a.first()).and_then(|e| e.get("at").and_then(|t| t.as_str()).map(str::to_string))).unwrap_or_default();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(TICK_SECS)).await;
            crate::scheduled::dispatch(&app).await;
            crate::simplechat::sweep(&app);
            retries(&app).await;
            if now() - pruned_at > 3600 { pruned_at = now(); if let Ok(n) = app.store.prune_runs(now() - RUNS_KEEP_DAYS * 86400, RUNS_KEEP) { if n > 0 { tracing::info!("история запусков: убрано {n} старше {RUNS_KEEP_DAYS} дней"); } } }
            let routines = match load_routines(&app) { Ok(r) => r, Err(_) => continue };
            schedule_tick(&app, &routines, Utc::now()).await;
            if routines.iter().any(|r| r.enabled && r.trigger.kind == "event") {
                if let Ok(ev) = app.core_get("/events").await {
                    let mut fresh: Vec<Value> = ev.as_array().cloned().unwrap_or_default().into_iter().filter(|e| e.get("at").and_then(|t| t.as_str()).map(|t| t > last_event_at.as_str()).unwrap_or(false)).collect();
                    fresh.reverse();
                    for e in &fresh {
                        let (at, kind, subject, message) = (e.get("at").and_then(|t| t.as_str()).unwrap_or(""), e.get("kind").and_then(|t| t.as_str()).unwrap_or(""), e.get("subject").and_then(|t| t.as_str()).unwrap_or(""), e.get("message").and_then(|t| t.as_str()).unwrap_or(""));
                        if at > last_event_at.as_str() { last_event_at = at.to_string(); }
                        for r in routines.iter().filter(|r| r.enabled && r.trigger.kind == "event") {
                            let want = r.trigger.event.trim();
                            let sub = r.trigger.contains.trim().to_lowercase();
                            if (!want.is_empty() && want != kind) || (!sub.is_empty() && !subject.to_lowercase().contains(&sub) && !message.to_lowercase().contains(&sub)) { continue }
                            // the routines' own run events must not launch routines in a loop
                            if kind == "agent" && (message.starts_with("рутина") || message.starts_with("беседа") || message.starts_with(SKILL_NOTE)) { continue }
                            if let Ok(a) = agent_by(&app, &r.agent) {
                                let extra = HashMap::from([("event", format!("{kind} · {subject}: {message}"))]);
                                if let Err(e) = launch(&app, Some(r), &a, &r.prompt, "event", extra).await { tracing::warn!("рутина «{}»: {e:#}", r.name); }
                            }
                        }
                    }
                }
            }
        }
    });
}

// ---------- HTTP ----------

type R = Result<Json<Value>, ApiError>;

fn run_brief(r: &Run) -> Value { json!({ "id": r.id, "status": r.status, "started_at": r.started_at, "ended_at": r.ended_at, "cost_usd": r.cost_usd, "summary": r.summary, "reason": r.reason, "session": r.session, "project": r.project }) }

pub async fn agents_list(State(app): State<Arc<App>>) -> R {
    let agents = load_agents(&app)?;
    let routines = load_routines(&app)?;
    let out: Vec<Value> = agents.iter().map(|a| {
        let mut v = serde_json::to_value(a).unwrap_or(json!({}));
        v["routines"] = json!(routines.iter().filter(|r| r.agent == a.id).count());
        v["last_run"] = app.store.runs(None, Some(&a.id), 1).ok().and_then(|r| r.first().map(run_brief)).unwrap_or(Value::Null);
        v
    }).collect();
    Ok(Json(json!({ "agents": out })))
}

pub async fn agents_create(State(app): State<Arc<App>>, Json(mut a): Json<AgentDef>) -> R {
    validate_agent(&mut a)?;
    a.id = new_id("ag"); a.created_at = now(); a.updated_at = now();
    let mut all = load_agents(&app)?; all.push(a.clone()); save_agents(&app, &all)?;
    Ok(Json(serde_json::to_value(a)?))
}

pub async fn agents_update(State(app): State<Arc<App>>, Path(id): Path<String>, Json(patch): Json<Value>) -> R {
    let mut all = load_agents(&app)?;
    let a = all.iter_mut().find(|a| a.id == id).ok_or_else(|| anyhow!("агент не найден: {id}"))?;
    let mut v = serde_json::to_value(&*a)?;
    for (k, val) in patch.as_object().cloned().unwrap_or_default() { if !matches!(k.as_str(), "id" | "created_at") { v[k] = val; } }
    let mut next: AgentDef = serde_json::from_value(v)?;
    validate_agent(&mut next)?;
    next.updated_at = now();
    *a = next.clone();
    save_agents(&app, &all)?;
    Ok(Json(serde_json::to_value(next)?))
}

pub async fn agents_delete(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let mut all = load_agents(&app)?;
    let before = all.len();
    all.retain(|a| a.id != id);
    if all.len() == before { return Err(anyhow!("агент не найден: {id}").into()); }
    save_agents(&app, &all)?;
    // its routines go away with it
    let mut rs = load_routines(&app)?; rs.retain(|r| r.agent != id); save_routines(&app, &rs)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct RunBody { #[serde(default)] pub prompt: String }

/// One-off assignment to an agent (without
/// a routine).
pub async fn agent_run(State(app): State<Arc<App>>, Path(id): Path<String>, Json(b): Json<RunBody>) -> R {
    let a = agent_by(&app, &id)?;
    if b.prompt.trim().is_empty() { return Err(anyhow!("пустое поручение").into()); }
    let run = launch(&app, None, &a, &b.prompt, "manual", HashMap::new()).await?;
    Ok(Json(json!({ "run": run })))
}

#[derive(Deserialize)]
pub struct MailBody { #[serde(default)] pub text: String, #[serde(default)] pub from: Option<String> }

/// Mail to an agent: from the owner or (the `from` field — an agent id) from another agent; the recipient's answer lands in
/// the sender's memory.
pub async fn agent_mail(State(app): State<Arc<App>>, Path(id): Path<String>, Json(b): Json<MailBody>) -> R {
    let from = match b.from.as_deref().filter(|f| !f.trim().is_empty()) { Some(f) => Some(agent_by(&app, f.trim())?), None => None };
    let run = match send_mail(&app, from.as_ref(), &id, &b.text, None, from.as_ref().map(|_| 1).unwrap_or(0)).await {
        Ok(r) => r,
        Err(e) if e.to_string().contains("на паузе") => return Err(ApiError(StatusCode::CONFLICT, e.to_string())),
        Err(e) => return Err(e.into()),
    };
    Ok(Json(json!({ "run": run })))
}

pub async fn routines_list(State(app): State<Arc<App>>) -> R {
    let agents = load_agents(&app)?;
    let tz = owner_tz(&app);
    let at = Utc::now();
    let out: Vec<Value> = load_routines(&app)?.iter().map(|r| {
        let mut v = serde_json::to_value(r).unwrap_or(json!({}));
        v["agent_name"] = json!(agents.iter().find(|a| a.id == r.agent).map(|a| a.name.clone()).unwrap_or_else(|| "—".into()));
        v["workspace"] = json!(agents.iter().find(|a| a.id == r.agent).map(|a| a.workspace.clone()).unwrap_or_default());
        v["overlap"] = json!(overlap_of(r));
        v["schedule_text"] = json!(if r.trigger.kind == "schedule" { describe(&r.trigger.cron) } else { String::new() });
        v["next_at"] = json!(if r.enabled && r.trigger.kind == "schedule" { Cron::parse(&r.trigger.cron).and_then(|c| c.next_after(tz, at)).map(|t| t.timestamp()) } else { None });
        v["last"] = app.store.runs(Some(&r.id), None, 1).ok().and_then(|x| x.first().map(run_brief)).unwrap_or(Value::Null);
        v
    }).collect();
    // the zone the schedules run in: the owner's (settings.timezone) or, while it is not set, the server's
    let offset = wall(tz, at).signed_duration_since(at.naive_utc()).num_minutes();
    Ok(Json(json!({ "routines": out, "scheduler": true, "now": now(), "tick": TICK_SECS, "timezone": tz.map(|z| z.name().to_string()), "utc_offset_min": offset })))
}

pub async fn routines_create(State(app): State<Arc<App>>, Json(mut r): Json<Routine>) -> R {
    let a = agent_by(&app, &r.agent)?;
    if r.overlap.is_empty() { r.overlap = default_overlap(&r.trigger.kind).into(); }
    validate_routine(&r, &a)?;
    r.id = new_id("rt"); r.token = format!("rt_{}", uuid::Uuid::new_v4().simple()); r.created_at = now(); r.updated_at = now(); r.last_run_at = None; r.last_status = None;
    let mut all = load_routines(&app)?; all.push(r.clone()); save_routines(&app, &all)?;
    Ok(Json(serde_json::to_value(r)?))
}

pub async fn routines_update(State(app): State<Arc<App>>, Path(id): Path<String>, Json(patch): Json<Value>) -> R {
    let mut all = load_routines(&app)?;
    let r = all.iter_mut().find(|r| r.id == id).ok_or_else(|| anyhow!("рутина не найдена: {id}"))?;
    let mut v = serde_json::to_value(&*r)?;
    for (k, val) in patch.as_object().cloned().unwrap_or_default() { if !matches!(k.as_str(), "id" | "created_at" | "token" | "last_run_at" | "last_status") { v[k] = val; } }
    let mut next: Routine = serde_json::from_value(v)?;
    let a = agent_by(&app, &next.agent)?;
    if next.overlap.is_empty() { next.overlap = overlap_of(&next).to_string(); }
    validate_routine(&next, &a)?;
    next.updated_at = now();
    *r = next.clone();
    save_routines(&app, &all)?;
    Ok(Json(serde_json::to_value(next)?))
}

pub async fn routines_delete(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let mut all = load_routines(&app)?;
    let before = all.len();
    all.retain(|r| r.id != id);
    if all.len() == before { return Err(anyhow!("рутина не найдена: {id}").into()); }
    save_routines(&app, &all)?;
    Ok(StatusCode::NO_CONTENT)
}

/// A new webhook address for the routine: the old one stops working at once (the way to take back a leaked link).
pub async fn routine_token(State(app): State<Arc<App>>, Path(id): Path<String>) -> R {
    let mut all = load_routines(&app)?;
    let r = all.iter_mut().find(|r| r.id == id).ok_or_else(|| anyhow!("рутина не найдена: {id}"))?;
    r.token = format!("rt_{}", uuid::Uuid::new_v4().simple()); r.updated_at = now();
    let out = serde_json::to_value(&*r)?;
    save_routines(&app, &all)?;
    Ok(Json(out))
}

/// "Run now": the routine's task (or a custom one) by
/// hand.
pub async fn routine_run(State(app): State<Arc<App>>, Path(id): Path<String>, body: Option<Json<RunBody>>) -> R {
    let r = routine_by(&app, &id)?;
    let a = agent_by(&app, &r.agent)?;
    let prompt = body.map(|b| b.0.prompt).filter(|p| !p.trim().is_empty()).unwrap_or_else(|| r.prompt.clone());
    let run = launch(&app, Some(&r), &a, &prompt, "manual", HashMap::new()).await?;
    Ok(Json(json!({ "run": run })))
}

#[derive(Deserialize)]
pub struct RunsQuery { pub routine: Option<String>, pub agent: Option<String>, #[serde(default = "d_limit")] pub limit: u32 }
fn d_limit() -> u32 { 50 }

pub async fn runs_list(State(app): State<Arc<App>>, Query(q): Query<RunsQuery>) -> R {
    Ok(Json(json!({ "runs": app.store.runs(q.routine.as_deref(), q.agent.as_deref(), q.limit.clamp(1, 500))? })))
}

pub async fn run_delete(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let r = app.store.run(&id)?.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "запуск не найден".into()))?;
    if r.status == "running" { return Err(ApiError(StatusCode::CONFLICT, "запуск ещё идёт".into())); }
    app.store.delete_run(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Routine webhook: the body (JSON or text, up to 64 KB) goes into the task as
/// {payload}.
pub async fn hook(State(app): State<Arc<App>>, Path(token): Path<String>, body: axum::body::Bytes) -> Result<(StatusCode, Json<Value>), ApiError> {
    let r = load_routines(&app)?.into_iter().find(|r| r.token == token && !token.is_empty()).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "вебхук не найден".into()))?;
    if !r.enabled { return Err(ApiError(StatusCode::CONFLICT, "рутина выключена".into())); }
    if r.trigger.kind != "webhook" { return Err(ApiError(StatusCode::CONFLICT, "у рутины другой триггер".into())); }
    if body.len() > WEBHOOK_MAX { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "тело вебхука больше 64 КБ".into())); }
    // A caller in a loop must not start a run per request: one start in a few seconds, the rest are told to slow down.
    // The time of the last call is kept in the process and taken in one step — two deliveries a second apart both saw
    // "no recent run" in the store, where a run appears only once its session is being created.
    let recent = app.store.runs(Some(&r.id), None, 1)?.first().map(|last| now() - last.started_at < HOOK_MIN_GAP).unwrap_or(false);
    let again = { let mut at = app.runs.hook_at.lock().unwrap(); let t = now(); let again = at.get(&r.id).map(|l| t - l < HOOK_MIN_GAP).unwrap_or(false); if !again && !recent { at.insert(r.id.clone(), t); } again };
    if recent || again { return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, format!("не чаще одного запуска в {HOOK_MIN_GAP} с"))); }
    let text = String::from_utf8_lossy(&body).to_string();
    let payload = serde_json::from_str::<Value>(&text).ok().and_then(|v| serde_json::to_string_pretty(&v).ok()).unwrap_or(text);
    let a = agent_by(&app, &r.agent)?;
    let run = launch(&app, Some(&r), &a, &r.prompt, "webhook", HashMap::from([("payload", payload)])).await?;
    // a delivery that started nothing is not "accepted": the sender is told, with the reason, and may come again
    let code = match run.status.as_str() { "running" => StatusCode::ACCEPTED, "skipped" => StatusCode::CONFLICT, _ => StatusCode::BAD_GATEWAY };
    Ok((code, Json(json!({ "run": run.id, "status": run.status, "reason": run.reason }))))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<Local> { Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap() }
    #[test]
    fn cron_fields_and_matching() {
        let c = Cron::parse("0 3 * * *").unwrap();
        assert!(c.matches(&at(2026, 9, 28, 3, 0)));
        assert!(!c.matches(&at(2026, 9, 28, 3, 1)));
        let wd = Cron::parse("30 9 * * 1-5").unwrap();
        assert!(wd.matches(&at(2026, 9, 28, 9, 30))); // Monday
        assert!(!wd.matches(&at(2026, 9, 27, 9, 30))); // Sunday
        let every = Cron::parse("*/15 * * * *").unwrap();
        assert!(every.matches(&at(2026, 1, 1, 12, 45)) && !every.matches(&at(2026, 1, 1, 12, 50)));
        assert!(Cron::parse("0 7 * * 7").unwrap().matches(&at(2026, 9, 27, 7, 0)));
        assert!(Cron::parse("a b c d e").is_none() && Cron::parse("0 25 * * *").is_none() && Cron::parse("0 3 * *").is_none());
        // day of month OR weekday when both are set
        let both = Cron::parse("0 0 1 * 1").unwrap();
        assert!(both.matches(&at(2026, 10, 1, 0, 0)) && both.matches(&at(2026, 9, 28, 0, 0)) && !both.matches(&at(2026, 9, 29, 0, 0)));
    }
    #[test]
    fn next_and_describe() {
        let c = Cron::parse("0 3 * * *").unwrap();
        let utc = |y, mo, d, h, mi| Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap();
        // three at night for the owner in Dubai is 23:00 UTC of the day before; for one in Berlin it moves with the clocks
        let dubai: chrono_tz::Tz = "Asia/Dubai".parse().unwrap();
        assert_eq!(c.next_after(Some(dubai), utc(2026, 9, 28, 12, 0)), Some(utc(2026, 9, 28, 23, 0)));
        let berlin: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        assert_eq!(c.next_after(Some(berlin), utc(2026, 7, 10, 12, 0)), Some(utc(2026, 7, 11, 1, 0)));     // summer time, UTC+2
        assert_eq!(c.next_after(Some(berlin), utc(2026, 12, 10, 12, 0)), Some(utc(2026, 12, 11, 2, 0)));   // winter time, UTC+1
        // the night the clocks go back (at 01:00 UTC, from 03:00 to 02:00): three o'clock comes once, in winter time
        assert_eq!(c.next_after(Some(berlin), utc(2026, 10, 24, 12, 0)), Some(utc(2026, 10, 25, 2, 0)));
        assert_eq!(c.catch_up_secs(Some(dubai), utc(2026, 9, 28, 23, 0)), CATCH_UP_MAX, "a daily schedule is caught up within two hours");
        assert_eq!(Cron::parse("*/30 * * * *").unwrap().catch_up_secs(None, utc(2026, 9, 28, 23, 0)), 15 * 60, "half the period");
        assert_eq!(Cron::parse("* * * * *").unwrap().catch_up_secs(None, utc(2026, 9, 28, 23, 0)), CATCH_UP_MIN);
        assert_eq!(describe("0 3 * * *"), "каждый день в 03:00");
        assert_eq!(describe("0 9 * * 1-5"), "пн–пт в 09:00");
        assert_eq!(describe("*/10 * * * *"), "каждые 10 мин");
        assert_eq!(describe("15 * * * *"), "каждый час в :15");
        assert_eq!(describe("5 4 1 * *"), "5 4 1 * *");
    }
    #[test]
    fn compose_memory_and_modes() {
        let a = AgentDef { name: "Тестировщик".into(), brief: "Гоняешь тесты.".into(), workspace: "alpha".into(), memory: "2026-09-27: флак в auth".into(), ..Default::default() };
        let vars = HashMap::from([("date", "2026-09-28".to_string())]);
        let t = compose(&a, "Прогони тесты. Сегодня {date}.", &vars, &[]);
        assert!(t.contains("# Агент «Тестировщик»") && t.contains("Гоняешь тесты.") && t.contains("флак в auth") && t.contains("Сегодня 2026-09-28.") && t.contains("ПАМЯТЬ:") && t.contains("НАВЫК:") && !t.contains("ПОЧТА:") && !t.contains("# Навыки"));
        let sk = crate::skills::Skill { name: "lean-code".into(), description: "минимальная правка".into(), body: "Меняй только нужное.".into(), ..Default::default() };
        let a2 = AgentDef { can_mail: true, place: "apps/site".into(), ..a.clone() };
        let t2 = compose(&a2, "x", &vars, &[sk]);
        assert!(t2.contains("# Навыки") && t2.contains("## lean-code — минимальная правка") && t2.contains("ПОЧТА:") && t2.contains("`apps/site`"));
        assert_eq!(memory_lines("Готово.\nПАМЯТЬ: тесты зелёные\n- ПАМЯТЬ: второй факт\nMEMORY: third"), vec!["тесты зелёные", "второй факт", "third"]);
        assert_eq!(skill_lines("НАВЫК: run-tests: сначала npm test\nНАВЫК: Bad Name: x\nSKILL: `deploy`: push then wait"), vec![("run-tests".to_string(), "сначала npm test".to_string()), ("deploy".to_string(), "push then wait".to_string())]);
        assert_eq!(mail_lines("ПОЧТА: «Секретарь»: собери issues\n- MAIL: @Ночной тестировщик: прогони тесты"), vec![("Секретарь".to_string(), "собери issues".to_string()), ("Ночной тестировщик".to_string(), "прогони тесты".to_string())]);
        assert!(clean_place(" /apps/site/ ").unwrap() == "apps/site" && clean_place("").unwrap().is_empty() && clean_place("../x").is_err() && clean_place("~/x").is_err());
        let m = remember("старое", "2026-09-28", &["новое".into()], MEMORY_DEFAULT);
        assert_eq!(m, "старое\n2026-09-28: новое");
        let long = remember(&"x".repeat(MEMORY_DEFAULT + 100), "d", &["y".into()], MEMORY_DEFAULT);
        assert!(long.chars().count() <= MEMORY_DEFAULT && long.ends_with("d: y"));
        assert_eq!(trim_memory("a\nbb\nccc", 7), "bb\nccc");
        assert_eq!(permission_mode("claude", "auto"), "auto");
        assert_eq!(permission_mode("codex", "auto"), "never");
        assert_eq!(permission_mode("codex", "ask"), "on-request");
        assert_eq!(permission_mode("claude", "bypass"), "bypassPermissions");
        assert_eq!(permission_mode("gemini", "ask"), "default"); assert_eq!(permission_mode("gemini", "auto"), "yolo"); assert_eq!(permission_mode("pi", "bypass"), "default");
    }

    #[test]
    fn failures_worth_a_retry_and_cost_by_tokens() {
        assert_eq!(transient("API Error: Request rejected (429) · All credentials for model claude-fable-5-2 are cooling down via provider claude"), Some("limit"));
        assert_eq!(transient("You've hit your limit · resets 3am"), Some("limit"));
        assert_eq!(transient("API Error: 503 No capacity available for model gpt-oss"), Some("provider"));
        assert_eq!(transient("API Error: Overloaded"), Some("provider"));
        assert_eq!(transient("API Error: 529 {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}"), Some("provider"));
        assert_eq!(transient("unexpected status 503 Service Unavailable: upstream is restarting"), Some("provider"));
        assert_eq!(transient("exceeded retry limit, last status: 429 Too Many Requests"), Some("limit"));
        assert_eq!(transient("API Error: Connection error."), Some("provider"));
        assert_eq!(transient("Request timed out."), Some("provider"));
        // a mistake of the request fails the same way in five minutes: no retry
        assert_eq!(transient("API Error: 400 {\"error\":{\"message\":\"unknown provider for model e2e-no-such-model\",\"type\":\"invalid_request_error\",\"code\":\"model_not_found\"}}"), None);
        assert_eq!(transient("API Error: 401 OAuth token has expired. Please run /login"), None);
        assert_eq!(transient("API Error: 404 model: claude-old-1 not found"), None);
        assert_eq!(transient("error_max_turns"), None);
        assert_eq!(transient("Тесты красные: 3 падения в auth.test.ts"), None);
        // a network failure names an address with a port, not a status: it is the network's trouble and is retried
        assert_eq!(transient("connect ETIMEDOUT 142.250.1.95:443"), Some("provider"));
        assert_eq!(transient("dial tcp 160.79.104.10:443: connection refused"), Some("provider"));
        assert_eq!(transient("API Error: Connection error. (connect ECONNREFUSED 127.0.0.1:443)"), Some("provider"));
        assert_eq!(transient("stream disconnected before completion: error sending request for url (https://chatgpt.com/backend-api/codex/responses)"), Some("provider"));
        assert_eq!(http_status("at file.js:412:15"), None);
        // a status is three digits standing alone: a price, a longer number or a fraction is not one
        assert_eq!(http_status("потрачено $0.503, окно 400000 токенов, 1.429 с"), None);
        assert_eq!(http_status("API Error (502) bad gateway"), Some(502));
        assert_eq!(http_status("status 404, then 503"), Some(404));
        let prices = json!({ "gpt-6-luna": { "input": 0.10, "output": 0.50, "cache_read": 0.01, "cache_write": 0.125 } });
        let usage = json!({ "input": 2_000_000, "output": 1_000_000, "cache_read": 10_000_000, "cache_create": 0, "cost_usd": 0.0 });
        assert!((estimate_cost(&prices, Some("gpt-6-luna"), &usage) - 0.80).abs() < 1e-9);
        assert_eq!(estimate_cost(&prices, Some("unknown-model"), &usage), 0.0);
        assert_eq!(estimate_cost(&prices, None, &usage), 0.0);
    }

    fn sched(id: &str, cron: &str) -> Routine { Routine { id: id.into(), name: id.into(), agent: "ag".into(), prompt: "x".into(), enabled: true, trigger: Trigger { kind: "schedule".into(), cron: cron.into(), ..Default::default() }, ..Default::default() } }

    /// The current minute fires; a minute missed while the service was down is caught up within half the period and
    /// reported as missed after it; nothing fires twice.
    #[test]
    fn due_now_caught_up_or_missed() {
        let utc = |h, mi| Utc.with_ymd_and_hms(2026, 11, 23, h, mi, 30).unwrap();
        let min = |h, mi| utc(h, mi).timestamp() / 60;
        let dubai: Option<chrono_tz::Tz> = Some("Asia/Dubai".parse().unwrap());
        let rs = vec![sched("night", "0 3 * * *"), sched("half", "*/30 * * * *"), Routine { enabled: false, ..sched("off", "* * * * *") }, Routine { trigger: Trigger { kind: "manual".into(), ..Default::default() }, ..sched("hand", "* * * * *") }];
        // 03:00 in Dubai is 23:00 UTC: the nightly one fires together with the half-hourly one, in the owner's time
        assert_eq!(due_schedules(&rs, dubai, Some(min(22, 59)), utc(23, 0)), vec![Due::Fire { id: "night".into(), minute: min(23, 0), late: false }, Due::Fire { id: "half".into(), minute: min(23, 0), late: false }]);
        assert_eq!(due_schedules(&rs, Some(chrono_tz::UTC), Some(min(22, 59)), utc(23, 0)), vec![Due::Fire { id: "half".into(), minute: min(23, 0), late: false }], "on a UTC clock 23:00 is not three at night");
        // the same minute is not looked at again; an ordinary minute fires nothing
        assert!(due_schedules(&rs, dubai, Some(min(23, 0)), utc(23, 0)).is_empty());
        assert!(due_schedules(&rs, dubai, Some(min(23, 0)), utc(23, 1)).is_empty());
        // the service was down from 22:50 to 23:10: both are caught up (10 minutes late: within two hours and within 15 minutes)
        assert_eq!(due_schedules(&rs, dubai, Some(min(22, 50)), utc(23, 10)), vec![Due::Fire { id: "night".into(), minute: min(23, 0), late: true }, Due::Fire { id: "half".into(), minute: min(23, 0), late: true }]);
        // down until 23:20: the nightly one is still worth it, the half-hourly one is 20 minutes late — its next turn is close
        assert_eq!(due_schedules(&rs, dubai, Some(min(22, 50)), utc(23, 20)), vec![Due::Fire { id: "night".into(), minute: min(23, 0), late: true }, Due::Missed { id: "half".into(), minute: min(23, 0) }]);
        // down from 20:00 until 01:40 of the next day: the nightly run of 23:00 is 2 h 40 min late — too late to start, and
        // it is told as missed, not dropped without a word; the half-hourly one comes out once, with its newest minute
        // (01:30, ten minutes ago — caught up), not eleven times
        let next_day = |h, mi| utc(h, mi) + chrono::Duration::days(1);
        assert_eq!(due_schedules(&rs, dubai, Some(min(20, 0)), next_day(1, 40)), vec![Due::Missed { id: "night".into(), minute: min(23, 0) }, Due::Fire { id: "half".into(), minute: next_day(1, 30).timestamp() / 60, late: true }]);
        // down for five days: the newest nightly minute is still found and told as missed
        assert_eq!(due_schedules(&rs[..1], dubai, Some(min(20, 0)), next_day(9, 0) + chrono::Duration::days(4)), vec![Due::Missed { id: "night".into(), minute: (next_day(23, 0) + chrono::Duration::days(3)).timestamp() / 60 }]);
        // the very first tick of an installation: the current minute only
        assert_eq!(due_schedules(&rs, dubai, None, utc(23, 0)).len(), 2);
        assert!(due_schedules(&rs, dubai, None, utc(23, 7)).is_empty());
    }

    // ---------- a run against a scripted core: the service's calls to the core are answered by a local server ----------

    use axum::extract::Query as Q;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct Core { infos: Mutex<VecDeque<Value>>, last: Mutex<Value>, events: Mutex<Value>, agentd: AtomicBool, created: Mutex<Vec<Value>>, prompts: Mutex<Vec<Value>>, notes: Mutex<Vec<Value>>, answers: Mutex<Vec<Value>>, files: Mutex<Vec<String>>, stopped: AtomicUsize, gone: Mutex<std::collections::HashSet<String>>, refuse: Mutex<Option<(u16, String)>>, deaf: AtomicUsize }
    fn info(status: &str, alive: bool, turns: u64, text: &str, pending: Value) -> Value { json!({ "info": { "status": status, "alive": alive, "usage": { "turns": turns, "cost_usd": 0.4, "input": 1000, "output": 200 }, "last_text": text, "last_seq": 12, "background": 0, "model": "claude-haiku-5-5" }, "pending": pending, "caps": {} }) }
    fn result(is_error: bool, text: &str) -> Value { json!([{ "seq": 11, "kind": "assistant", "content": [{ "type": "text", "text": text }] }, { "seq": 12, "kind": "result", "is_error": is_error, "subtype": if is_error { "error_during_execution" } else { "success" }, "text": text }]) }

    /// The service state over a scripted core, with one agent (`ag`, Claude, workspace `ws`) and one schedule routine (`rt`).
    async fn rig() -> (Arc<App>, Arc<Core>, AgentDef, Routine, std::path::PathBuf) {
        let core = Arc::new(Core { infos: Default::default(), last: Mutex::new(info("working", true, 0, "", json!([]))), events: Mutex::new(json!([])), agentd: AtomicBool::new(true), created: Default::default(), prompts: Default::default(), notes: Default::default(), answers: Default::default(), files: Default::default(), stopped: Default::default(), gone: Default::default(), refuse: Default::default(), deaf: Default::default() });
        type S = State<Arc<Core>>;
        let router = axum::Router::new()
            .route("/api/agents/budgets/state", axum::routing::get(|| async { Json(json!([])) }))
            .route("/api/agents/notify", axum::routing::post(|State(c): S, Json(b): Json<Value>| async move { c.notes.lock().unwrap().push(b); Json(json!({})) }))
            .route("/api/projects/{p}/agentd", axum::routing::get(|State(c): S| async move { if c.agentd.load(Ordering::Relaxed) { (StatusCode::OK, Json(json!({}))) } else { (StatusCode::BAD_GATEWAY, Json(json!({ "error": "agentd: connection refused" }))) } }))
            // `refuse` — the core will not create a session (status and words); otherwise sessions are numbered as they come
            .route("/api/projects/{p}/sessions", axum::routing::post(|State(c): S, Json(b): Json<Value>| async move {
                if let Some((code, words)) = c.refuse.lock().unwrap().clone() { return (StatusCode::from_u16(code).unwrap(), Json(json!({ "error": words }))); }
                let mut l = c.created.lock().unwrap(); l.push(b); (StatusCode::OK, Json(json!({ "id": format!("session-{:04}", l.len()) }))) }))
            .route("/api/projects/{p}/sessions/{id}/prompt", axum::routing::post(|State(c): S, Json(b): Json<Value>| async move { c.prompts.lock().unwrap().push(b); Json(json!({})) }))
            .route("/api/projects/{p}/sessions/{id}/permission", axum::routing::post(|State(c): S, Json(b): Json<Value>| async move { c.answers.lock().unwrap().push(b); Json(json!({})) }))
            // the journal that goes with the session state served last (`_events` in a scripted info), or the common one
            // `deaf` — this many requests for the journal fail first
            .route("/api/projects/{p}/sessions/{id}/events", axum::routing::get(|State(c): S| async move {
                if c.deaf.load(Ordering::Relaxed) > 0 { c.deaf.fetch_sub(1, Ordering::Relaxed); return (StatusCode::BAD_GATEWAY, Json(json!({ "error": "agentd: timeout" }))); }
                let own = c.last.lock().unwrap().get("_events").cloned(); (StatusCode::OK, Json(own.unwrap_or_else(|| c.events.lock().unwrap().clone()))) }))
            // a session that was stopped stays stopped: asked about later, it is not alive (as in agentd)
            .route("/api/projects/{p}/sessions/{id}", axum::routing::get(|State(c): S, Path((_, id)): Path<(String, String)>| async move {
                    if c.gone.lock().unwrap().contains(&id) { let mut was = c.last.lock().unwrap().clone(); was["info"]["status"] = json!("exited"); was["info"]["alive"] = json!(false); return Json(was); }
                    let next = c.infos.lock().unwrap().pop_front(); if let Some(n) = next { *c.last.lock().unwrap() = n; } Json(c.last.lock().unwrap().clone()) })
                .delete(|State(c): S, Path((_, id)): Path<(String, String)>| async move { c.stopped.fetch_add(1, Ordering::Relaxed); c.gone.lock().unwrap().insert(id); Json(json!({})) }))
            .route("/api/projects/{p}/fs/mkdir", axum::routing::post(|| async { Json(json!({})) }))
            .route("/api/projects/{p}/fs/file", axum::routing::put(|State(c): S, Q(q): Q<HashMap<String, String>>| async move { c.files.lock().unwrap().push(q.get("path").cloned().unwrap_or_default()); Json(json!({})) }))
            .with_state(core.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, router).await.unwrap(); });
        let dir = std::env::temp_dir().join(format!("agents-runs-{}-{}", std::process::id(), uuid::Uuid::new_v4()));
        let app = Arc::new(App { http: reqwest::Client::new(), cloudd: format!("http://{addr}"), store: crate::store::Store::open(&dir).unwrap(), data: dir.clone(), chat: Default::default(), runs: Default::default(), scratch_busy: Default::default(), scratch_error: Default::default(),
            hub: Default::default(), voice_streams: Arc::new(tokio::sync::Semaphore::new(1)), push: Default::default(), share: Default::default(), channels: Default::default() });
        let agent = AgentDef { id: "ag".into(), name: "Тестировщик".into(), brief: "Гоняешь тесты.".into(), provider: "claude".into(), workspace: "ws".into(), mode: "auto".into(), remember: true, can_routine: true, memory_budget: MEMORY_DEFAULT, ..Default::default() };
        let routine = Routine { id: "rt".into(), name: "Ночной прогон".into(), agent: "ag".into(), prompt: "Прогони тесты.".into(), notify: "ask_error".into(), enabled: true, retries: true, overlap: "skip".into(), on_question: "wait".into(), trigger: Trigger { kind: "schedule".into(), cron: "0 3 * * *".into(), ..Default::default() }, ..Default::default() };
        save_agents(&app, &[agent.clone()]).unwrap(); save_routines(&app, &[routine.clone()]).unwrap();
        (app, core, agent, routine, dir)
    }
    /// Wait until the watcher has closed the run and done everything after it (memory, mail, the notification). A run
    /// that never got a watcher (skipped, or failed at the start) is ready as it is.
    async fn settled(app: &App, id: &str) -> Run {
        for _ in 0..300 {
            let r = app.store.run(id).unwrap().unwrap();
            let watched = r.session.is_none() || r.kind.as_deref() == Some("start") || app.store.get_kv(&format!("watched:{id}")).unwrap().is_some();
            if r.status != "running" && watched { return r; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the run {id} is still going");
    }
    fn script(core: &Core, infos: Vec<Value>, events: Value) { *core.infos.lock().unwrap() = infos.into(); *core.events.lock().unwrap() = events; }
    fn titles(core: &Core) -> Vec<String> { core.notes.lock().unwrap().iter().map(|n| n["title"].as_str().unwrap_or("").to_string()).collect() }

    /// A session that hit a limit and was taken over by the core's account pool (stopped, resumed on a free account, the
    /// request repeated) is watched to its real end: not stopped under the pool, not closed as failed, not retried.
    #[tokio::test]
    async fn limit_hit_session_taken_over_by_the_pool_is_watched_on() {
        let (app, core, agent, routine, dir) = rig().await;
        let limit = "API Error: Request rejected (429) · All credentials for model claude-fable-5-2 are cooling down";
        let with = |mut v: Value, events: Value, hit: bool| { v["_events"] = events; if hit { v["info"]["limit_hit"] = json!({ "source": "result", "at": 1 }); } v };
        let good = json!([{ "seq": 15, "kind": "assistant", "content": [{ "type": "text", "text": "Тесты зелёные." }] }, { "seq": 16, "kind": "result", "is_error": false, "subtype": "success", "text": "Тесты зелёные." }]);
        script(&core, vec![
            with(info("idle", true, 1, limit, json!([])), result(true, limit), true),      // the turn ended with the limit
            with(info("exited", false, 1, limit, json!([])), result(true, limit), true),   // the pool stops the session…
            with(info("working", true, 1, "", json!([])), result(true, limit), false),     // …and resumes it on another account
            with(info("idle", true, 2, "Тесты зелёные.", json!([])), good, false),
        ], json!([]));
        let run = launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap();
        let run = settled(&app, &run.id).await;
        assert_eq!(run.status, "done", "{:?}", run.reason);
        assert!(run.retry_at.is_none() && run.kind.is_none());
        assert_eq!(run.summary.as_deref(), Some("Тесты зелёные."));
        assert_eq!(core.stopped.load(Ordering::Relaxed), 1, "the session is stopped once, at its real end");
        assert!(titles(&core).is_empty());
        assert!(app.store.due_retries(now() + 86400).unwrap().is_empty(), "nothing is left to retry");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A routine bound to a notification channel with reports allowed: the task of every run carries the note about the
    /// box of that run's own session, and the workspace gets the script (docs/agents-ux.md §9.12.4).
    #[tokio::test]
    async fn a_reporting_routine_tells_its_agent_about_the_box() {
        let (app, core, agent, routine, dir) = rig().await;
        script(&core, vec![info("idle", true, 1, "Готово.", json!([]))], result(false, "Готово."));
        let run = launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap();
        settled(&app, &run.id).await;
        assert!(!core.prompts.lock().unwrap()[0]["content"].as_str().unwrap().contains("cloudos-report"), "no binding — no note");
        assert!(core.files.lock().unwrap().is_empty());
        app.store.put_kv("notify.bindings", &json!([{ "id": "nb_1", "channel": "ch_x", "scope": "routine", "routine": "rt", "reports": true }])).unwrap();
        script(&core, vec![info("idle", true, 1, "Готово.", json!([]))], result(false, "Готово."));
        let run = launch(&app, Some(&routine), &agent, &routine.prompt, "manual", HashMap::new()).await.unwrap();
        let run = settled(&app, &run.id).await;
        assert_eq!(run.status, "done");
        let sent = core.prompts.lock().unwrap()[1]["content"].as_str().unwrap().to_string();
        assert!(sent.contains("Прогони тесты.") && sent.contains("# Отчёты владельцу") && sent.contains("sh ~/.cloudos/bin/cloudos-report session-0002 <отчёт.md>"), "{sent}");
        assert!(core.files.lock().unwrap().iter().any(|f| f == "~/.cloudos/bin/cloudos-report"), "{:?}", core.files.lock().unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// One request for the journal that fails must not decide the outcome: read as "no result", an errored turn was
    /// written down as done, with the error as its summary and in the memory.
    #[tokio::test]
    async fn journal_that_cannot_be_read_decides_nothing() {
        let (app, core, agent, routine, dir) = rig().await;
        let overloaded = "API Error: 529 {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}";
        script(&core, vec![info("idle", true, 1, overloaded, json!([]))], result(true, overloaded));
        core.deaf.store(3, Ordering::Relaxed);
        let run = settled(&app, &launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        assert_eq!(core.deaf.load(Ordering::Relaxed), 0, "the journal was asked for again until it answered");
        assert_eq!((run.status.as_str(), run.kind.as_deref()), ("failed", Some("provider")));
        assert!(run.retry_at.is_some() && run.summary.is_none());
        assert!(agent_by(&app, "ag").unwrap().memory.is_empty());
        assert_eq!(core.stopped.load(Ordering::Relaxed), 1, "the session is stopped once the outcome is known, not before");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A limit the adapter marked on the session is a limit even when the provider words it in a new way; a task too
    /// long for the history is retried whole.
    #[tokio::test]
    async fn limit_known_by_the_mark_is_retried_and_a_long_task_goes_whole() {
        let (app, core, agent, routine, dir) = rig().await;
        let words = "Your plan's capacity is used up for now.";
        assert_eq!(transient(words), None, "the words alone say nothing");
        let mut marked = info("idle", true, 1, words, json!([])); marked["info"]["limit_hit"] = json!({ "source": "rate_limit_event", "at": 1 });
        script(&core, vec![marked], result(true, words));
        let long = format!("Сделай по списку.\n{}\nКОНЕЦ-СПИСКА", "пункт списка, который надо выполнить\n".repeat(700));
        assert!(long.chars().count() > PROMPT_KEEP);
        let run = settled(&app, &launch(&app, Some(&routine), &agent, &long, "manual", HashMap::new()).await.unwrap().id).await;
        assert_eq!((run.status.as_str(), run.kind.as_deref()), ("failed", Some("limit")));
        assert!(run.retry_at.is_some(), "a limit is retried whatever its wording");
        assert!(run.prompt.as_deref().unwrap().chars().count() <= PROMPT_KEEP + 1, "the history keeps the beginning");
        app.store.put_run(&Run { retry_at: Some(now() - 1), ..run.clone() }).unwrap();
        script(&core, vec![info("idle", true, 1, "готово", json!([]))], result(false, "готово"));
        retries(&app).await;
        let again = app.store.runs(Some("rt"), None, 10).unwrap().into_iter().find(|r| r.attempt == 1).expect("the retry");
        assert_eq!(settled(&app, &again.id).await.status, "done");
        let sent = core.prompts.lock().unwrap().last().unwrap()["content"].as_str().unwrap().to_string();
        assert!(sent.contains("КОНЕЦ-СПИСКА"), "the retry carries the task to its last line");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A session the core will not create: the provider's or the network's trouble is retried, a refusal is told at
    /// once — and nothing is left "running" or started.
    #[tokio::test]
    async fn run_that_cannot_start_is_retried_or_told() {
        let (app, core, agent, routine, dir) = rig().await;
        *core.refuse.lock().unwrap() = Some((502, "agentd: bad gateway".into()));
        let run = launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap();
        assert_eq!((run.status.as_str(), run.kind.as_deref()), ("failed", Some("start")));
        assert!(run.retry_at.is_some() && run.reason.as_deref().unwrap().contains("сессия не создалась"));
        assert!(titles(&core).is_empty(), "a retry is on its way");
        assert_eq!(app.store.runs(Some("rt"), None, 10).unwrap().len(), 1, "one record: written before the session was asked for, then closed");
        assert_eq!(app.store.running_for_routine("rt", 0).unwrap(), 0);
        assert_eq!(load_routines(&app).unwrap()[0].last_status.as_deref(), Some("failed"));
        *core.refuse.lock().unwrap() = Some((400, "аккаунт acc_x не найден".into()));
        let run = launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap();
        assert_eq!((run.status.as_str(), run.kind.as_deref(), run.retry_at), ("failed", Some("start"), None));
        assert_eq!(titles(&core), vec!["рутина: упала"]);
        assert!(core.notes.lock().unwrap()[0]["body"].as_str().unwrap().contains("аккаунт acc_x не найден"));
        assert!(core.created.lock().unwrap().is_empty() && core.prompts.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A store that refuses writes (a full disk): nothing is started that could not be written down, and neither the
    /// schedule nor a planned retry turns into a run started again on every tick.
    #[tokio::test]
    async fn store_that_refuses_writes_starts_nothing_and_no_storm() {
        let (app, core, agent, routine, dir) = rig().await;
        let limit = "You've hit your limit · resets 3am";
        script(&core, vec![info("idle", true, 1, limit, json!([]))], result(true, limit));
        let failed = settled(&app, &launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        app.store.put_run(&Run { retry_at: Some(now() - 1), ..failed.clone() }).unwrap();
        let every = Routine { overlap: "parallel".into(), trigger: Trigger { kind: "schedule".into(), cron: "* * * * *".into(), ..Default::default() }, ..routine.clone() };
        save_routines(&app, &[every.clone()]).unwrap();
        core.notes.lock().unwrap().clear();
        app.store.read_only(true);
        assert!(launch(&app, Some(&every), &agent, &every.prompt, "manual", HashMap::new()).await.is_err(), "the launch is refused");
        let at = Utc.with_ymd_and_hms(2026, 11, 23, 10, 0, 20).unwrap();
        for tick in 0..4 { schedule_tick(&app, std::slice::from_ref(&every), at + chrono::Duration::seconds(15 * tick)).await; retries(&app).await; }
        assert_eq!(core.created.lock().unwrap().len(), 1, "only the session of the first run, made while the store worked");
        assert_eq!(titles(&core), vec!["рутина: упала"], "the retry that could not start is told once, not on every tick");
        app.store.read_only(false);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A planned retry that does not start (the routine was switched off meanwhile) tells the failure that was held
    /// back for it.
    #[tokio::test]
    async fn retry_that_does_not_start_tells_the_failure_held_back() {
        let (app, core, agent, routine, dir) = rig().await;
        let limit = "API Error: Request rejected (429) · All credentials for model claude-fable-5-2 are cooling down";
        script(&core, vec![info("idle", true, 1, limit, json!([]))], result(true, limit));
        let run = settled(&app, &launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        assert!(run.retry_at.is_some() && titles(&core).is_empty());
        save_routines(&app, &[Routine { enabled: false, ..routine.clone() }]).unwrap();
        app.store.put_run(&Run { retry_at: Some(now() - 1), ..run.clone() }).unwrap();
        retries(&app).await;
        assert_eq!(titles(&core), vec!["рутина: упала"]);
        assert!(core.notes.lock().unwrap()[0]["body"].as_str().unwrap().contains("повтора не будет: рутина выключена"));
        assert_eq!(core.created.lock().unwrap().len(), 1, "no new session");
        assert!(app.store.run(&run.id).unwrap().unwrap().retry_at.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The core's account pool took the session over after the watcher had given up on it: when the retry comes due the
    /// session is working again. The run is taken back to watching — the task is not started a second time.
    #[tokio::test]
    async fn session_taken_over_late_is_watched_again_not_started_anew() {
        let (app, core, agent, routine, dir) = rig().await;
        let limit = "You've hit your limit · resets 3am";
        script(&core, vec![info("idle", true, 1, limit, json!([]))], result(true, limit));
        let run = settled(&app, &launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        assert_eq!((run.status.as_str(), run.kind.as_deref()), ("failed", Some("limit")));
        // the pool resumes the stopped session on another account and the request is answered there
        core.gone.lock().unwrap().clear();
        let good = json!([{ "seq": 15, "kind": "assistant", "content": [{ "type": "text", "text": "Тесты зелёные." }] }, { "seq": 16, "kind": "result", "is_error": false, "subtype": "success", "text": "Тесты зелёные." }]);
        let mut answered = info("idle", true, 2, "Тесты зелёные.", json!([])); answered["_events"] = good;
        script(&core, vec![info("working", true, 1, "", json!([])), answered], json!([]));
        app.store.put_run(&Run { retry_at: Some(now() - 1), ..run.clone() }).unwrap();
        retries(&app).await;
        let mut back = app.store.run(&run.id).unwrap().unwrap();
        for _ in 0..200 { if back.status != "running" { break } tokio::time::sleep(std::time::Duration::from_millis(20)).await; back = app.store.run(&run.id).unwrap().unwrap(); }
        assert_eq!((back.status.as_str(), back.kind.as_deref(), back.retry_at), ("done", None, None));
        assert_eq!(back.summary.as_deref(), Some("Тесты зелёные."));
        assert_eq!(core.created.lock().unwrap().len(), 1, "the task was not started anew");
        assert_eq!(app.store.runs(Some("rt"), None, 10).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The scheduler handles a minute once whatever the store remembers of it, and a restarted service continues from
    /// the store's copy.
    #[tokio::test]
    async fn schedule_minute_is_handled_once_even_if_the_store_forgets_it() {
        let (app, core, _agent, routine, dir) = rig().await;
        let every = Routine { overlap: "parallel".into(), trigger: Trigger { kind: "schedule".into(), cron: "* * * * *".into(), ..Default::default() }, ..routine.clone() };
        save_routines(&app, &[every.clone()]).unwrap();
        let rs = vec![every];
        let at = Utc.with_ymd_and_hms(2026, 11, 23, 10, 0, 20).unwrap();
        schedule_tick(&app, &rs, at).await;
        schedule_tick(&app, &rs, at + chrono::Duration::seconds(15)).await;
        assert_eq!(core.created.lock().unwrap().len(), 1, "one run for the minute");
        // the store lost the mark (its write failed): the process still knows the minute is done
        app.store.put_kv("scheduler", &json!({ "minute": at.timestamp() / 60 - 5 })).unwrap();
        schedule_tick(&app, &rs, at + chrono::Duration::seconds(30)).await;
        assert_eq!(core.created.lock().unwrap().len(), 1, "not started again from a stale mark");
        schedule_tick(&app, &rs, at + chrono::Duration::seconds(60)).await;
        assert_eq!(core.created.lock().unwrap().len(), 2, "the next minute fires");
        assert_eq!(app.store.get_kv("scheduler").unwrap().unwrap()["minute"], json!(at.timestamp() / 60 + 1));
        // the clock set back does not fire the handled minutes again
        schedule_tick(&app, &rs, at).await;
        assert_eq!(core.created.lock().unwrap().len(), 2);
        // a restarted service starts from the store's copy: the minute it missed is caught up once
        *app.runs.minute.lock().unwrap() = None;
        schedule_tick(&app, &rs, at + chrono::Duration::seconds(125)).await;
        assert_eq!(core.created.lock().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Two deliveries of a webhook at once start one run; a delivery that started nothing is answered with the reason,
    /// not "accepted". Two launches at once do not both pass the overlap rule.
    #[tokio::test]
    async fn webhooks_and_launches_at_the_same_moment_start_one_run() {
        let (app, core, agent, routine, dir) = rig().await;
        let hooked = Routine { token: "rt_secret".into(), overlap: "skip".into(), trigger: Trigger { kind: "webhook".into(), ..Default::default() }, ..routine.clone() };
        save_routines(&app, &[hooked.clone()]).unwrap();
        let call = || hook(State(app.clone()), Path("rt_secret".to_string()), axum::body::Bytes::from_static(b"{\"n\": 1}"));
        let (a, b) = tokio::join!(call(), call());
        let codes: Vec<u16> = [a, b].into_iter().map(|r| match r { Ok((c, _)) => c.as_u16(), Err(ApiError(c, _)) => c.as_u16() }).collect();
        assert!(codes.contains(&202) && codes.contains(&429), "{codes:?}");
        assert_eq!(core.created.lock().unwrap().len(), 1);
        // the run is going; five seconds later another delivery comes: the routine skips it, and the sender is told
        app.runs.hook_at.lock().unwrap().clear();
        let going = app.store.runs(Some("rt"), None, 1).unwrap().remove(0);
        app.store.delete_run(&going.id).unwrap(); app.store.put_run(&Run { started_at: now() - 60, ..going.clone() }).unwrap();
        match call().await { Ok((code, Json(body))) => { assert_eq!(code, StatusCode::CONFLICT); assert_eq!(body["status"], json!("skipped")); assert!(body["reason"].as_str().unwrap().contains("ещё идёт")); } Err(ApiError(c, m)) => panic!("{c} {m}") }
        assert_eq!(core.created.lock().unwrap().len(), 1);
        // two launches at the same moment (an event and a retry, say): one starts, the other is an overlap
        let (app, core, agent2, routine2, dir2) = { let _ = (agent, dir.clone()); rig().await };
        let (x, y) = tokio::join!(launch(&app, Some(&routine2), &agent2, "a", "event", HashMap::new()), launch(&app, Some(&routine2), &agent2, "b", "event", HashMap::new()));
        let mut st = vec![x.unwrap().status, y.unwrap().status]; st.sort();
        assert_eq!(st, vec!["running", "skipped"]);
        assert_eq!(core.created.lock().unwrap().len(), 1);
        assert!(app.runs.launching.lock().unwrap().is_empty(), "the mark of a launch in flight is dropped with it");
        let _ = std::fs::remove_dir_all(dir); let _ = std::fs::remove_dir_all(dir2);
    }

    /// The overlap rule a routine starts with follows its trigger; a turn that has ended is judged by its result even
    /// if the budget ran out with it; a launch cut by a restart does not stay "running".
    #[tokio::test]
    async fn defaults_by_trigger_finished_turn_over_budget_and_a_cut_launch() {
        let (app, core, agent, routine, dir) = rig().await;
        assert_eq!(overlap_of(&sched("s", "0 3 * * *")), "skip");
        assert_eq!(overlap_of(&Routine { trigger: Trigger { kind: "webhook".into(), ..Default::default() }, ..sched("w", "") }), "parallel");
        assert_eq!(overlap_of(&Routine { overlap: "skip".into(), trigger: Trigger { kind: "event".into(), ..Default::default() }, ..sched("e", "") }), "skip");
        let made = routines_create(State(app.clone()), Json(Routine { name: "по вебхуку".into(), agent: "ag".into(), prompt: "x".into(), notify: "ask_error".into(), on_question: "wait".into(), trigger: Trigger { kind: "webhook".into(), ..Default::default() }, ..Default::default() })).await.ok().unwrap().0;
        assert_eq!(made["overlap"], json!("parallel"), "a webhook brings its own data each time");
        save_routines(&app, &[routine.clone()]).unwrap();
        // the turn ended and cost more than the routine allows: it is done, with its result — not thrown away as "budget"
        let tight = Routine { budget: Budget { run_usd: 0.1, ..Default::default() }, ..routine.clone() };
        save_routines(&app, &[tight.clone()]).unwrap();
        script(&core, vec![info("idle", true, 1, "Готово.", json!([]))], result(false, "Готово."));
        let run = settled(&app, &launch(&app, Some(&tight), &agent, &tight.prompt, "manual", HashMap::new()).await.unwrap().id).await;
        assert_eq!((run.status.as_str(), run.summary.as_deref()), ("done", Some("Готово.")));
        // a run that is still going past its budget is stopped
        script(&core, vec![info("working", true, 0, "", json!([]))], json!([]));
        let run = settled(&app, &launch(&app, Some(&tight), &agent, &tight.prompt, "manual", HashMap::new()).await.unwrap().id).await;
        assert_eq!((run.status.as_str(), run.kind.as_deref()), ("budget", Some("budget")));
        // a launch cut by a restart: the row is "running" and has no session
        let cut = Run { id: "run_cut".into(), routine: Some("rt".into()), routine_name: Some(routine.name.clone()), agent: "ag".into(), agent_name: agent.name.clone(), project: "ws".into(), trigger: "schedule".into(), status: "running".into(), started_at: now(), ..Default::default() };
        app.store.put_run(&cut).unwrap();
        assert_eq!(app.store.running_for_routine("rt", 0).unwrap(), 1);
        resume_runs(&app);
        let closed = app.store.run("run_cut").unwrap().unwrap();
        assert_eq!((closed.status.as_str(), closed.kind.as_deref()), ("failed", Some("start")));
        assert!(closed.reason.as_deref().unwrap().contains("перезапуском"));
        // a row left "running" by a lost write, older than any run can be, does not hold the routine back
        app.store.put_run(&Run { id: "run_old".into(), started_at: now() - MAX_RUN_SECS - 3600, session: Some("session-x".into()), ..cut.clone() }).unwrap();
        script(&core, vec![info("working", true, 0, "", json!([]))], json!([]));
        assert_eq!(launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap().status, "running");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A scheduled message caught mid-send by a restart: a recent one is sent again, an old leftover is closed with the
    /// reason instead of landing in a conversation days later.
    #[test]
    fn scheduled_message_caught_by_a_restart() {
        let dir = std::env::temp_dir().join(format!("agents-sched-{}-{}", std::process::id(), uuid::Uuid::new_v4()));
        let msg = |id: &str, at: i64| crate::scheduled::Scheduled { id: id.into(), project: "ws".into(), session: "s".into(), content: "продолжай".into(), at, status: "sending".into(), created_at: at - 60, sent_at: None, note: None, error: None };
        {
            let store = crate::store::Store::open(&dir).unwrap();
            store.put_scheduled(&msg("recent", now() - 600)).unwrap();
            store.put_scheduled(&msg("old", now() - 3 * 86400)).unwrap();
        }
        let store = crate::store::Store::open(&dir).unwrap();
        let all = store.scheduled(None, None, false).unwrap();
        let by = |id: &str| all.iter().find(|m| m.id == id).unwrap();
        assert_eq!(by("recent").status, "pending");
        assert_eq!(by("old").status, "failed");
        assert!(by("old").error.as_deref().unwrap().contains("перезапуском"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A zone that moves its clocks at midnight has no 00:00 that day: the day of the daily budget begins with its
    /// first existing hour, not a whole day earlier.
    #[test]
    fn budget_day_where_midnight_does_not_exist() {
        let cairo: chrono_tz::Tz = "Africa/Cairo".parse().unwrap();
        // 2026-04-24, the last Friday of April: 00:00 becomes 01:00
        assert!(cairo.from_local_datetime(&chrono::NaiveDate::from_ymd_opt(2026, 4, 24).unwrap().and_hms_opt(0, 0, 0).unwrap()).earliest().is_none(), "the zone data has that gap");
        let noon = Utc.with_ymd_and_hms(2026, 4, 24, 10, 0, 0).unwrap();
        assert_eq!(day_start(Some(cairo), noon), Utc.with_ymd_and_hms(2026, 4, 23, 22, 0, 0).unwrap().timestamp());
        // an ordinary day: local midnight
        let tbilisi: chrono_tz::Tz = "Asia/Tbilisi".parse().unwrap();
        assert_eq!(day_start(Some(tbilisi), Utc.with_ymd_and_hms(2026, 11, 23, 10, 0, 0).unwrap()), Utc.with_ymd_and_hms(2026, 11, 22, 20, 0, 0).unwrap().timestamp());
    }

    /// The owner's zone goes through the settings: an unknown name is refused, a known one moves "next run" and the
    /// `{date}`/`{time}` of the task onto the owner's clock, an empty one gives the server clock back.
    #[tokio::test]
    async fn zone_of_the_owner_is_checked_kept_and_cleared() {
        let (app, core, agent, routine, dir) = rig().await;
        let put = |body: Value| crate::api::settings_put(State(app.clone()), Json(body));
        assert!(matches!(put(json!({ "timezone": "Mars/Olympus" })).await, Err(ApiError(StatusCode::BAD_REQUEST, _))), "an unknown zone must be refused");
        assert_eq!(owner_tz(&app), None);
        assert!(put(json!({ "timezone": "Asia/Tbilisi" })).await.is_ok());
        assert_eq!(owner_tz(&app), Some(chrono_tz::Asia::Tbilisi));
        let list = routines_list(State(app.clone())).await.ok().unwrap().0;
        assert_eq!(list["timezone"], json!("Asia/Tbilisi")); assert_eq!(list["utc_offset_min"], json!(240));
        // "every day at 03:00" is 03:00 in Tbilisi, i.e. 23:00 UTC
        let next = list["routines"][0]["next_at"].as_i64().unwrap();
        assert_eq!(Utc.timestamp_opt(next, 0).unwrap().format("%H:%M").to_string(), "23:00");
        // the task carries the owner's date and time, not the server's
        script(&core, vec![info("idle", true, 1, "ok", json!([]))], result(false, "ok"));
        let run = launch(&app, Some(&routine), &agent, "Сейчас {date} {time}.", "manual", HashMap::new()).await.unwrap();
        settled(&app, &run.id).await;
        let sent = core.prompts.lock().unwrap()[0]["content"].as_str().unwrap().to_string();
        let local = Utc::now().with_timezone(&chrono_tz::Asia::Tbilisi);
        assert!(sent.contains(&format!("Сейчас {} ", local.format("%Y-%m-%d"))), "{sent}");
        // another setting saved later does not drop the zone; an empty zone does
        assert!(put(json!({ "mail_paused": true })).await.is_ok());
        assert_eq!(owner_tz(&app), Some(chrono_tz::Asia::Tbilisi));
        assert!(put(json!({ "timezone": "" })).await.is_ok());
        assert_eq!(owner_tz(&app), None);
        assert!(crate::api::settings_value(&app).get("timezone").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A turn cut by the provider's limit is a failed run that is tried again — not a "done" one with the error as its
    /// summary and in the agent's memory. The owner hears of it when the retries are used up.
    #[tokio::test]
    async fn provider_error_fails_the_run_and_is_retried() {
        let (app, core, agent, routine, dir) = rig().await;
        let limit = "API Error: Request rejected (429) · All credentials for model claude-fable-5-2 are cooling down via provider claude";
        let task = "Прогони тесты. Данные: {payload}";
        let mut id = { script(&core, vec![info("idle", true, 1, limit, json!([]))], result(true, limit)); launch(&app, Some(&routine), &agent, task, "webhook", HashMap::from([("payload", "{\"n\": 7}".to_string())])).await.unwrap().id };
        let first = id.clone();
        let mut first_due = 0;
        for attempt in 0..=3u32 {
            let run = settled(&app, &id).await;
            assert_eq!((run.status.as_str(), run.kind.as_deref(), run.attempt), ("failed", Some("limit"), attempt));
            assert!(run.reason.as_deref().unwrap().contains("(429)") && run.summary.is_none(), "the error is the reason, not a summary");
            assert!(agent_by(&app, "ag").unwrap().memory.is_empty(), "nothing of a failed run goes into the memory");
            if attempt == 3 { assert!(run.retry_at.is_none(), "three retries, no more"); break; }
            let wait = run.retry_at.expect("a retry is planned") - now();
            assert!((wait - RETRY_AFTER_MIN[attempt as usize] * 60).abs() <= 2, "attempt {attempt}: the retry in {wait} s");
            assert!(titles(&core).is_empty(), "no notification while a retry is on its way");
            // the pause passes
            let due = now() - 1;
            if attempt == 0 { first_due = due; }
            app.store.put_run(&Run { retry_at: Some(due), ..run.clone() }).unwrap();
            script(&core, vec![info("idle", true, 1, limit, json!([]))], result(true, limit));
            retries(&app).await;
            assert!(app.store.run(&id).unwrap().unwrap().retry_at.is_none(), "the retry is taken once");
            let next = app.store.runs(Some("rt"), None, 10).unwrap().into_iter().find(|r| r.attempt == attempt + 1).expect("the retry is a run of its own");
            assert_eq!((next.retry_of.as_deref(), next.trigger.as_str()), (Some(first.as_str()), "webhook"));
            assert!(next.extra.as_deref().unwrap().contains("payload"), "the retry keeps what the webhook brought");
            id = next.id;
        }
        assert_eq!(titles(&core), vec!["рутина: упала"], "one notification, after the last attempt");
        assert!(core.notes.lock().unwrap()[0]["body"].as_str().unwrap().contains("после 3 повт."));
        assert!(core.prompts.lock().unwrap().iter().all(|p| p["content"].as_str().unwrap().contains("Прогони тесты. Данные: {\"n\": 7}")), "every attempt gets the task with what the webhook brought");
        assert_eq!(core.prompts.lock().unwrap().len(), 4);
        assert_eq!(load_routines(&app).unwrap()[0].last_status.as_deref(), Some("failed"));
        // a store that could not forget the retry time must not make the next tick start the same retry again
        app.store.put_run(&Run { retry_at: Some(first_due), ..app.store.run(&first).unwrap().unwrap() }).unwrap();
        retries(&app).await;
        assert_eq!(core.created.lock().unwrap().len(), 4, "a retry that was attempted is not started twice by one process");
        assert!(app.store.run(&first).unwrap().unwrap().retry_at.is_none());
        // retries switched off for the routine: the failure is told at once
        let quiet = Routine { retries: false, ..routine.clone() };
        save_routines(&app, &[quiet.clone()]).unwrap(); core.notes.lock().unwrap().clear();
        script(&core, vec![info("idle", true, 1, limit, json!([]))], result(true, limit));
        let run = settled(&app, &launch(&app, Some(&quiet), &agent, &quiet.prompt, "manual", HashMap::new()).await.unwrap().id).await;
        assert!(run.status == "failed" && run.retry_at.is_none() && titles(&core) == vec!["рутина: упала"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A good turn: done, the memory and the skill it proposed; the skill reaches the workspace only once the owner
    /// accepts it. A mistake of the task itself is a failure without a retry.
    #[tokio::test]
    async fn good_turn_feeds_memory_and_a_proposed_skill_waits_for_the_owner() {
        let (app, core, agent, routine, dir) = rig().await;
        let answer = "Тесты зелёные, правок нет.\nПАМЯТЬ: флак в auth.test.ts\nНАВЫК: run-tests: сначала npm test. Потом чинить.";
        script(&core, vec![info("working", true, 0, "", json!([])), info("idle", true, 1, answer, json!([]))], result(false, answer));
        let run = settled(&app, &launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        assert_eq!((run.status.as_str(), run.kind.as_deref(), run.retry_at), ("done", None, None));
        assert!(run.summary.as_deref().unwrap().starts_with("Тесты зелёные"));
        assert_eq!(core.stopped.load(Ordering::Relaxed), 1, "the session is stopped after its turn");
        let a = agent_by(&app, "ag").unwrap();
        assert!(a.memory.contains("флак в auth.test.ts") && a.memory.contains("итог «Ночной прогон»"), "{}", a.memory);
        assert_eq!(a.skills, vec!["run-tests"], "the proposed skill is the agent's own at once");
        assert_eq!(titles(&core), vec!["агент предложил навык"]);
        let skill = crate::skills::load(&app).unwrap().into_iter().find(|s| s.name == "run-tests").unwrap();
        assert_eq!(skill.source, "agent:ag");
        // the next run carries the skill in its task, but nothing is written into the workspace's skills yet
        script(&core, vec![info("idle", true, 1, "ок", json!([]))], result(false, "ок"));
        settled(&app, &launch(&app, Some(&routine), &a, &routine.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        assert!(core.prompts.lock().unwrap().last().unwrap()["content"].as_str().unwrap().contains("## run-tests"));
        assert!(core.files.lock().unwrap().is_empty(), "{:?}", core.files.lock().unwrap());
        // another agent proposing the same name changes nothing; the owner's acceptance makes the skill a workspace one
        assert_eq!(crate::skills::upsert_from_agent(&app, "other", "run-tests", "совсем другое").unwrap(), crate::skills::Proposed::Taken);
        assert_eq!(crate::skills::upsert_from_agent(&app, "ag", "run-tests", "сначала npm test, потом lint").unwrap(), crate::skills::Proposed::Updated);
        assert!(crate::skills::accept(State(app.clone()), Path("run-tests".to_string())).await.is_ok());
        assert_eq!(crate::skills::upsert_from_agent(&app, "ag", "run-tests", "x").unwrap(), crate::skills::Proposed::Owners);
        script(&core, vec![info("idle", true, 1, "ок", json!([]))], result(false, "ок"));
        settled(&app, &launch(&app, Some(&routine), &agent_by(&app, "ag").unwrap(), &routine.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        assert_eq!(*core.files.lock().unwrap(), vec![".claude/skills/run-tests/SKILL.md"]);
        // a routine told to keep quiet keeps quiet about a proposed skill too
        core.notes.lock().unwrap().clear();
        let quiet = Routine { notify: "never".into(), ..routine.clone() };
        save_routines(&app, &[quiet.clone()]).unwrap();
        let more = "Готово.\nНАВЫК: lint-first: сначала линтер.";
        script(&core, vec![info("idle", true, 1, more, json!([]))], result(false, more));
        settled(&app, &launch(&app, Some(&quiet), &agent_by(&app, "ag").unwrap(), &quiet.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        assert!(crate::skills::load(&app).unwrap().iter().any(|s| s.name == "lint-first") && titles(&core).is_empty());
        save_routines(&app, &[routine.clone()]).unwrap();
        // a mistake of the task: failed, told at once, not retried
        core.notes.lock().unwrap().clear();
        script(&core, vec![info("idle", true, 1, "Reached maximum number of turns", json!([]))], result(true, "Reached maximum number of turns"));
        let bad = settled(&app, &launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        assert_eq!((bad.status.as_str(), bad.kind.as_deref(), bad.retry_at), ("failed", Some("error"), None));
        assert_eq!(titles(&core), vec!["рутина: упала"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A run that does not start is told once, however often the schedule comes round; a run still going holds the next
    /// one back unless the owner starts it by hand or the routine allows runs side by side.
    #[tokio::test]
    async fn skips_are_told_once_and_runs_do_not_pile_up() {
        let (app, core, agent, routine, dir) = rig().await;
        core.agentd.store(false, Ordering::Relaxed);
        let a = launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap();
        assert_eq!((a.status.as_str(), a.kind.as_deref()), ("skipped", Some("workspace")));
        let b = launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap();
        assert_eq!(b.id, a.id, "the same skip again is the same record");
        assert_eq!(app.store.runs(Some("rt"), None, 10).unwrap().len(), 1);
        assert_eq!(titles(&core), vec!["рутина пропущена"]);
        assert!(core.notes.lock().unwrap()[0]["body"].as_str().unwrap().contains("не запущен"));
        assert_eq!(load_routines(&app).unwrap()[0].last_status.as_deref(), Some("skipped"));
        // the workspace stays stopped: a day later the same skip is told again, not kept quiet for ever
        app.store.delete_run(&a.id).unwrap(); app.store.put_run(&Run { started_at: now() - 21 * 3600, ..a.clone() }).unwrap();
        let c = launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap();
        assert!(c.id != a.id && c.kind.as_deref() == Some("workspace"));
        assert_eq!(titles(&core), vec!["рутина пропущена", "рутина пропущена"]);
        // a firing missed while the service was down is an event of its own every time
        let stub = || Run { id: new_id("run"), routine: Some("rt".into()), routine_name: Some(routine.name.clone()), agent: "ag".into(), agent_name: agent.name.clone(), project: "ws".into(), trigger: "schedule".into(), started_at: now(), ..Default::default() };
        let m1 = skipped(&app, Some(&routine), stub(), "missed", "сервис не работал в назначенное время (03:00)".into()).await.unwrap();
        let m2 = skipped(&app, Some(&routine), stub(), "missed", "сервис не работал в назначенное время (03:00)".into()).await.unwrap();
        assert!(m1.id != m2.id && m2.kind.as_deref() == Some("missed"));
        assert_eq!(titles(&core).len(), 4);
        // the workspace is back: a run starts and keeps working
        core.agentd.store(true, Ordering::Relaxed); core.notes.lock().unwrap().clear();
        let going = launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap();
        assert_eq!(going.status, "running");
        let held = launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap();
        assert_eq!((held.status.as_str(), held.kind.as_deref()), ("skipped", Some("overlap")));
        assert!(titles(&core).is_empty(), "an overlap is not a problem to notify about");
        assert_eq!(launch(&app, Some(&routine), &agent, &routine.prompt, "manual", HashMap::new()).await.unwrap().status, "running", "the owner's own start is not held back");
        let side = Routine { overlap: "parallel".into(), ..routine.clone() };
        assert_eq!(launch(&app, Some(&side), &agent, &side.prompt, "schedule", HashMap::new()).await.unwrap().status, "running");
        assert_eq!(core.created.lock().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A question nobody is there to answer: by default the owner is called to "Inbox"; a routine set to refuse answers
    /// "no" itself and the agent goes on.
    #[tokio::test]
    async fn unattended_question_waits_or_is_refused() {
        let (app, core, agent, routine, dir) = rig().await;
        let ask = || info("waiting", true, 0, "", json!([{ "request_id": "req-1", "tool_name": "Bash" }]));
        script(&core, vec![ask(), ask(), info("idle", true, 1, "готово", json!([]))], result(false, "готово"));
        settled(&app, &launch(&app, Some(&routine), &agent, &routine.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        assert_eq!(titles(&core), vec!["рутина ждёт решения"], "told once, however long it waits");
        assert!(core.answers.lock().unwrap().is_empty());
        core.notes.lock().unwrap().clear();
        let strict = Routine { on_question: "deny".into(), ..routine.clone() };
        save_routines(&app, &[strict.clone()]).unwrap();
        script(&core, vec![ask(), info("idle", true, 1, "готово без этой команды", json!([]))], result(false, "готово без этой команды"));
        let run = settled(&app, &launch(&app, Some(&strict), &agent, &strict.prompt, "schedule", HashMap::new()).await.unwrap().id).await;
        assert_eq!(run.status, "done");
        let answers = core.answers.lock().unwrap().clone();
        assert_eq!(answers.len(), 1);
        assert_eq!((answers[0]["request_id"].as_str(), answers[0]["behavior"].as_str()), (Some("req-1"), Some("deny")));
        assert!(titles(&core).is_empty(), "nobody is called: the routine decided itself");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A request from another agent is carried out in "ask", whatever the recipient's own mode: it cannot give more
    /// rights than its sender has. The owner's message keeps the recipient's mode.
    #[tokio::test]
    async fn mail_from_an_agent_runs_in_ask_mode() {
        let (app, core, agent, _routine, dir) = rig().await;
        let sender = AgentDef { id: "s".into(), name: "Секретарь".into(), can_mail: true, mode: "ask".into(), ..agent.clone() };
        let free = AgentDef { id: "f".into(), name: "Исполнитель".into(), can_mail: true, mode: "bypass".into(), ..agent.clone() };
        save_agents(&app, &[sender.clone(), free.clone()]).unwrap();
        assert_eq!(send_mail(&app, Some(&sender), "Исполнитель", "удали каталог build", None, 1).await.unwrap().status, "running");
        assert_eq!(send_mail(&app, None, "f", "удали каталог build", None, 0).await.unwrap().status, "running");
        let made = core.created.lock().unwrap().clone();
        assert_eq!((made[0]["permission_mode"].as_str(), made[1]["permission_mode"].as_str()), (Some("default"), Some("bypassPermissions")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
