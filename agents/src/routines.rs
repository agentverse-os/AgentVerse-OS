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
use chrono::{Datelike, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub const TICK_SECS: u64 = 15;
const WATCH_SECS: u64 = 10;
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
}

/// Mail between agents: who sent it, from which run, which step of the chain it
/// is.
#[derive(Clone, Debug, Default)]
pub struct Mail { pub from_agent: Option<String>, pub reply_to: Option<String>, pub hops: u32 }

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
    pub fn matches(&self, t: &chrono::DateTime<Local>) -> bool {
        if !self.min[t.minute() as usize] || !self.hour[t.hour() as usize] || !self.mon[t.month() as usize] { return false; }
        let dom = self.dom[t.day() as usize];
        let dow = self.dow[t.weekday().num_days_from_sunday() as usize];
        if self.dom_any && self.dow_any { true } else if self.dom_any { dow } else if self.dow_any { dom } else { dom || dow }
    }
    /// Next firing minute after `from` (up to 60 days ahead).
    pub fn next_after(&self, from: chrono::DateTime<Local>) -> Option<chrono::DateTime<Local>> {
        let mut t = from.with_second(0)?.with_nanosecond(0)? + chrono::Duration::minutes(1);
        for _ in 0..(60 * 24 * 60) { if self.matches(&t) { return Some(t); } t += chrono::Duration::minutes(1); }
        None
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
    launch_with(app, routine, agent, prompt, trigger, extra, Mail::default()).await
}

/// The same with the mail fields (sender, source run, chain step).
pub async fn launch_with(app: &Arc<App>, routine: Option<&Routine>, agent: &AgentDef, prompt: &str, trigger: &str, extra: HashMap<&str, String>, mail: Mail) -> Result<Run> {
    let started = now();
    let local = Local::now();
    let mut run = Run {
        id: new_id("run"), routine: routine.map(|r| r.id.clone()), routine_name: routine.map(|r| r.name.clone()), agent: agent.id.clone(), agent_name: agent.name.clone(),
        project: agent.workspace.clone(), session: None, trigger: trigger.to_string(), status: "running".into(), started_at: started, ended_at: None, cost_usd: 0.0, summary: None, reason: None, prompt: Some(short(prompt, 2000)),
        from_agent: mail.from_agent.clone(), reply_to: mail.reply_to.clone(), hops: mail.hops,
    };
    let skip = |run: &mut Run, reason: String| { run.status = "skipped".into(); run.reason = Some(reason); run.ended_at = Some(now()); };
    if routine.is_some() && !agent.can_routine { skip(&mut run, format!("агенту «{}» запрещены рутины", agent.name)); app.store.put_run(&run)?; return Ok(run); }
    if mail.hops > MAIL_MAX_HOPS { skip(&mut run, format!("цепочка почты длиннее {MAIL_MAX_HOPS} шагов — остановлена")); app.store.put_run(&run)?; return Ok(run); }
    if let Some(r) = routine {
        if r.budget.day_usd > 0.0 {
            let midnight = local.date_naive().and_hms_opt(0, 0, 0).and_then(|d| Local.from_local_datetime(&d).single()).map(|d| d.timestamp()).unwrap_or(started - 86400);
            let spent = app.store.runs_cost_since(&r.id, midnight).unwrap_or(0.0);
            if spent >= r.budget.day_usd { skip(&mut run, format!("дневной бюджет исчерпан: ${spent:.2} из ${:.2}", r.budget.day_usd)); app.store.put_run(&run)?; return Ok(run); }
        }
    }
    // core spending limits (§8.3): with "stop" and an exhausted limit the run is skipped rather than failing on the first
    // turn
    if let Ok(st) = app.core_get("/agents/budgets/state").await {
        let blocked = st.as_array().and_then(|a| a.iter().find(|b| b.get("blocked").and_then(|x| x.as_bool()) == Some(true) && matches!(b.get("scope").and_then(|x| x.as_str()), Some(sc) if sc == "total" || sc == agent.workspace)));
        if let Some(b) = blocked {
            let who = if b.get("scope").and_then(|x| x.as_str()) == Some("total") { "всего".to_string() } else { agent.workspace.clone() };
            skip(&mut run, format!("дневной лимит расходов {who} исчерпан: ${:.2} из ${:.2} за 24 ч", b.get("spent").and_then(|x| x.as_f64()).unwrap_or(0.0), b.get("per_day").and_then(|x| x.as_f64()).unwrap_or(0.0)));
            app.store.put_run(&run)?; return Ok(run);
        }
    }
    if let Err(e) = app.core_get(&format!("/projects/{}/agentd", agent.workspace)).await {
        skip(&mut run, format!("workspace {} не запущен или agentd недоступен: {e:#}", agent.workspace)); app.store.put_run(&run)?; return Ok(run);
    }
    let mut vars: HashMap<&str, String> = HashMap::from([("date", local.format("%Y-%m-%d").to_string()), ("time", local.format("%H:%M").to_string()), ("payload", String::new()), ("event", String::new())]);
    vars.extend(extra);
    // agent skills: into the task for everyone, for Claude also as files ~/.claude/skills/<name>/SKILL.md (available as /name and in
    // interactive sessions)
    let skills = crate::skills::for_agent(app, agent);
    if agent.provider == "claude" && !skills.is_empty() { crate::skills::materialize(app, &agent.workspace, &skills).await; }
    let text = compose(agent, prompt, &vars, &skills);
    let title = format!("{} · {}", routine.map(|r| r.name.as_str()).unwrap_or(agent.name.as_str()), local.format("%d.%m %H:%M"));
    // permission mode: the routine's own (if set), otherwise the agent's; "bypass" goes only through the core policy
    // anyway
    let mode = routine.map(|r| r.mode.as_str()).filter(|m| !m.is_empty()).unwrap_or(agent.mode.as_str());
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
    let created = match app.core_post(&format!("/projects/{}/sessions", agent.workspace), &body).await {
        Ok(v) => v,
        Err(e) => { run.status = "failed".into(); run.reason = Some(format!("сессия не создалась: {e:#}")); run.ended_at = Some(now()); app.store.put_run(&run)?; return Ok(run); }
    };
    let sid = created.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
    run.session = Some(sid.clone());
    if let Err(e) = app.core_post(&format!("/projects/{}/sessions/{sid}/prompt", agent.workspace), &json!({ "content": text })).await {
        run.status = "failed".into(); run.reason = Some(format!("задание не отправилось: {e:#}")); run.ended_at = Some(now()); app.store.put_run(&run)?; return Ok(run);
    }
    app.store.put_run(&run)?;
    tracing::info!("запуск {} ({}): агент «{}», workspace {}, сессия {}", run.id, trigger, agent.name, agent.workspace, &sid[..8.min(sid.len())]);
    tokio::spawn(watch_boxed(app.clone(), run.id.clone()));
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

/// Watching the run to the end: waiting for a decision → a notification (per the rule), budget/timeout → stop, exit →
/// result, agent memory, notification, a mark in the routine.
async fn watch(app: Arc<App>, run_id: String) {
    let Some(mut run) = app.store.run(&run_id).ok().flatten() else { return };
    let Some(sid) = run.session.clone() else { return };
    let routine = run.routine.as_deref().and_then(|id| routine_by(&app, id).ok());
    let notify_rule = routine.as_ref().map(|r| r.notify.clone()).unwrap_or_else(|| "ask_error".into());
    let run_budget = routine.as_ref().map(|r| r.budget.run_usd).unwrap_or(0.0);
    let label = format!("{} · {}", run.project, run.routine_name.clone().unwrap_or_else(|| run.agent_name.clone()));
    let mut told_waiting = false;
    let mut failures = 0u32;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(WATCH_SECS)).await;
        let info = match app.core_get(&format!("/projects/{}/sessions/{sid}", run.project)).await {
            Ok(v) => v,
            Err(e) => {
                failures += 1;
                if e.to_string().contains("404") || failures > 60 { run.status = "failed".into(); run.reason = Some(format!("сессия недоступна: {e:#}")); break; }
                continue;
            }
        };
        failures = 0;
        let i = info.get("info").cloned().unwrap_or(info.clone());
        let status = i.get("status").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let alive = i.get("alive").and_then(|a| a.as_bool()).unwrap_or(false);
        let cost = i.pointer("/usage/cost_usd").and_then(|c| c.as_f64()).unwrap_or(0.0);
        run.cost_usd = cost;
        let elapsed = now() - run.started_at;
        if alive && status == "waiting" && !told_waiting {
            told_waiting = true;
            if notify_rule != "never" { notify(&app, &run.project, "рутина ждёт решения", &format!("{label} — ответьте во «Входящих»"), "question").await; }
        }
        if alive && run_budget > 0.0 && cost >= run_budget {
            let _ = app.core_delete(&format!("/projects/{}/sessions/{sid}", run.project)).await;
            run.status = "budget".into(); run.reason = Some(format!("бюджет запуска исчерпан: ${cost:.2} из ${run_budget:.2}")); break;
        }
        if alive && elapsed > MAX_RUN_SECS {
            let _ = app.core_delete(&format!("/projects/{}/sessions/{sid}", run.project)).await;
            run.status = "timeout".into(); run.reason = Some("дольше 2 часов — остановлен".into()); break;
        }
        // the headless process does not exit by itself after a turn (the session is "ready" and waits for the next message):
        // a finished turn is the end of the run; the process is stopped, the session record stays (opens in Thread, can be
        // resumed)
        let turns = i.pointer("/usage/turns").and_then(|t| t.as_u64()).unwrap_or(0);
        let answered = i.get("last_text").and_then(|t| t.as_str()).map(|t| !t.trim().is_empty()).unwrap_or(false);
        // background tasks after the answer (background Bash, Agent, Workflow) — the
        // run is still going
        let background = i.get("background").and_then(|b| b.as_u64()).unwrap_or(0);
        let finished = alive && status == "idle" && background == 0 && (turns >= 1 || answered) && elapsed >= WATCH_SECS as i64;
        if finished { if let Err(e) = app.core_delete(&format!("/projects/{}/sessions/{sid}", run.project)).await { tracing::debug!("остановка сессии {sid}: {e:#}"); } }
        if !alive || finished {
            run.status = if status == "error" { "failed".into() } else { "done".into() };
            let last_seq = i.get("last_seq").and_then(|s| s.as_u64()).unwrap_or(0);
            let text = final_text(&app, &run.project, &sid, last_seq).await.or_else(|| i.get("last_text").and_then(|t| t.as_str()).map(str::to_string)).unwrap_or_default();
            let mem = memory_lines(&text);
            let summary: String = text.lines().filter(|l| !l.trim().is_empty() && memory_lines(l).is_empty()).collect::<Vec<_>>().join("\n");
            run.summary = Some(short(summary.trim(), SUMMARY_MAX));
            if status == "error" { run.reason = Some(i.get("last_text").and_then(|t| t.as_str()).unwrap_or("процесс завершился с ошибкой").to_string()); }
            // skills from the answer ("НАВЫК: name: text") — into the catalog and
            // the agent's list
            let new_skills = skill_lines(&text);
            for (name, body) in &new_skills { if let Err(e) = crate::skills::upsert_from_agent(&app, &run.agent, name, body) { tracing::warn!("навык «{name}» от агента {}: {e:#}", run.agent_name); } }
            // agent memory: "ПАМЯТЬ: …" lines always, a short summary — if enabled; a mail answer — into the sender's
            // memory
            let date = Local::now().format("%Y-%m-%d").to_string();
            let mut reflect_agent: Option<AgentDef> = None;
            if let Ok(mut agents) = load_agents(&app) {
                if let Some(a) = agents.iter_mut().find(|a| a.id == run.agent) {
                    let mut lines = mem.clone();
                    if a.remember && run.status == "done" { if let Some(s) = run.summary.as_deref().filter(|s| !s.is_empty()) { lines.push(format!("итог «{}»: {}", run.routine_name.clone().unwrap_or_else(|| if run.trigger == "mail" { "почта".into() } else { "поручение".into() }), short(&s.replace('\n', " "), 240))); } }
                    for (name, _) in &new_skills { if !a.skills.iter().any(|s| s == name) && a.skills.len() < SKILLS_PER_AGENT { a.skills.push(name.clone()); } }
                    if !lines.is_empty() { a.memory = remember(&a.memory, &date, &lines, a.memory_budget); }
                    if !lines.is_empty() || !new_skills.is_empty() { a.updated_at = now(); }
                    if a.reflect && run.status == "done" { reflect_agent = Some(a.clone()); }
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
    let _ = app.store.put_run(&run);
    if let Ok(mut rs) = load_routines(&app) {
        if let Some(r) = run.routine.as_deref().and_then(|id| rs.iter_mut().find(|r| r.id == id)) { r.last_run_at = Some(run.started_at); r.last_status = Some(run.status.clone()); let _ = save_routines(&app, &rs); }
    }
    let dur = run.ended_at.unwrap_or(now()) - run.started_at;
    let body = format!("{label} · {} мин · ${:.2}{}", (dur / 60).max(0), run.cost_usd, run.summary.as_deref().filter(|s| !s.is_empty()).map(|s| format!(": {}", short(&s.replace('\n', " "), 160))).unwrap_or_default());
    match (run.status.as_str(), notify_rule.as_str()) {
        (_, "never") => {}
        ("done", "always") => notify(&app, &run.project, "рутина закончила", &body, "white_check_mark").await,
        ("done", _) => {}
        (st, _) => notify(&app, &run.project, &format!("рутина: {}", match st { "failed" => "упала", "budget" => "остановлена по бюджету", "timeout" => "остановлена по времени", s => s }), &format!("{body}{}", run.reason.as_deref().map(|r| format!(" — {r}")).unwrap_or_default()), "x").await,
    }
    tracing::info!("запуск {}: {} (${:.2}, {} с)", run.id, run.status, run.cost_usd, dur);
}

// ---------- scheduler ----------

/// Background task: watch the "hanging" runs to the end, then the tick — cron by minutes and core
/// feed events.
pub fn start(app: Arc<App>) {
    tokio::spawn(async move {
        if let Ok(hanging) = app.store.running_runs() {
            for r in hanging { tokio::spawn(watch_boxed(app.clone(), r.id.clone())); }
        }
        let mut fired: HashMap<String, i64> = HashMap::new();
        // core events: only those newer than the start
        // moment
        let mut last_event_at = app.core_get("/events").await.ok().and_then(|v| v.as_array().and_then(|a| a.first()).and_then(|e| e.get("at").and_then(|t| t.as_str()).map(str::to_string))).unwrap_or_default();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(TICK_SECS)).await;
            crate::scheduled::dispatch(&app).await;
            crate::simplechat::sweep(&app);
            let routines = match load_routines(&app) { Ok(r) => r, Err(_) => continue };
            let local = Local::now();
            let minute = local.timestamp() / 60;
            for r in routines.iter().filter(|r| r.enabled && r.trigger.kind == "schedule") {
                let Some(c) = Cron::parse(&r.trigger.cron) else { continue };
                if fired.get(&r.id) == Some(&minute) || !c.matches(&local) { continue }
                fired.insert(r.id.clone(), minute);
                if let Ok(a) = agent_by(&app, &r.agent) { if let Err(e) = launch(&app, Some(r), &a, &r.prompt, "schedule", HashMap::new()).await { tracing::warn!("рутина «{}»: {e:#}", r.name); } }
            }
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
                            if kind == "agent" && (message.starts_with("рутина") || message.starts_with("беседа")) { continue }
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
    let local = Local::now();
    let out: Vec<Value> = load_routines(&app)?.iter().map(|r| {
        let mut v = serde_json::to_value(r).unwrap_or(json!({}));
        v["agent_name"] = json!(agents.iter().find(|a| a.id == r.agent).map(|a| a.name.clone()).unwrap_or_else(|| "—".into()));
        v["workspace"] = json!(agents.iter().find(|a| a.id == r.agent).map(|a| a.workspace.clone()).unwrap_or_default());
        v["schedule_text"] = json!(if r.trigger.kind == "schedule" { describe(&r.trigger.cron) } else { String::new() });
        v["next_at"] = json!(if r.enabled && r.trigger.kind == "schedule" { Cron::parse(&r.trigger.cron).and_then(|c| c.next_after(local)).map(|t| t.timestamp()) } else { None });
        v["last"] = app.store.runs(Some(&r.id), None, 1).ok().and_then(|x| x.first().map(run_brief)).unwrap_or(Value::Null);
        v
    }).collect();
    Ok(Json(json!({ "routines": out, "scheduler": true, "now": now(), "tick": TICK_SECS })))
}

pub async fn routines_create(State(app): State<Arc<App>>, Json(mut r): Json<Routine>) -> R {
    let a = agent_by(&app, &r.agent)?;
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
    let text = String::from_utf8_lossy(&body).to_string();
    let payload = serde_json::from_str::<Value>(&text).ok().and_then(|v| serde_json::to_string_pretty(&v).ok()).unwrap_or(text);
    let a = agent_by(&app, &r.agent)?;
    let run = launch(&app, Some(&r), &a, &r.prompt, "webhook", HashMap::from([("payload", payload)])).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "run": run.id, "status": run.status }))))
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
        assert_eq!(c.next_after(at(2026, 9, 28, 3, 0)), Some(at(2026, 9, 29, 3, 0)));
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
}
