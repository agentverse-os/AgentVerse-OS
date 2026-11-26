//! The agent's report box (docs/agents-ux.md §9.12.4). An agent gets neither a key nor an address of a notification
//! channel and goes nowhere itself. It has a box — a directory in its workspace, `~/.cloudos/outbox/<conversation>/` — and
//! a small script that puts a report there: `report.md` and the files next to it. This service looks into the boxes of
//! the conversations whose owner allowed reports, checks what was put (how many, how large, names and content that look
//! like secrets), takes it out and hands it to the channels bound to that conversation. The agent learns the outcome
//! from the answer of the script; the owner sees every report as a line in the conversation.
//!
//! Everything goes through the access to workspace files this service already has (the core's `fs` routes): nothing in
//! the workspaces has to change or restart.
//!
//! The boundary is the workspace, not the conversation: every program in a workspace runs as the same user, so any of
//! them can put a report into the box of a conversation that may report. What it cannot do is choose where the report
//! goes — that is fixed by the owner's bindings — or learn it: nothing written back into the workspace (the answer of
//! the script, the line in the conversation) names a channel or repeats the words of its service.
use crate::channels::{self, MsgFile};
use crate::{chanfmt, App};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub const MAX_FILES: usize = 5;
pub const MAX_FILE: u64 = 20 * 1024 * 1024;
pub const PER_TURN: u32 = 5;
pub const PER_DAY: u32 = 20;
/// The text of a report: longer ones are to be attached as a file.
const MAX_TEXT: usize = 200_000;
/// A box is looked into while its conversation is at work and this long after.
const LINGER: i64 = 90;
/// A report that has been lying in the box longer than this is nobody's news.
const REPORT_MAX_AGE: i64 = 15 * 60;
const ROOT: &str = "~/.cloudos/outbox";
const SCRIPT: &str = "~/.cloudos/bin/cloudos-report";
const KV_BOXES: &str = "notify.boxes";
/// The first line of the note that tells an agent about its box (the app folds the note away in the feed).
pub const INTRO_HEAD: &str = "# Отчёты владельцу";

/// The script in the workspace. It only moves files into the box and waits for the answer; it knows no address and no
/// key. Run with `sh`, so it needs no executable bit (the file API of the workspace sets none). A box is open while the
/// owner allows reports from its conversation (the `.open` mark, kept by the service): without it the script says so at
/// once instead of waiting for an answer that will not come.
const SCRIPT_BODY: &str = r#"#!/bin/sh
# cloudos-report — hand a report (Markdown) and files over to the owner's notification channels.
#   sh ~/.cloudos/bin/cloudos-report <box> <report.md> [file ...]
# The script only puts the files into the box; a service outside the workspace checks and sends them.
set -eu
if [ "$#" -lt 2 ]; then echo "usage: cloudos-report <box> <report.md> [file ...]" >&2; exit 2; fi
box="$1"; shift
report="$1"; shift
case "$box" in ""|*[!A-Za-z0-9_-]*) echo "cloudos-report: неверное имя ящика" >&2; exit 2;; esac
[ -f "$report" ] || { echo "cloudos-report: нет файла отчёта: $report" >&2; exit 2; }
root="$HOME/.cloudos/outbox/$box"
i=0
while [ ! -f "$root/.open" ]; do
  i=$((i + 1))
  if [ "$i" -gt 10 ]; then echo "refused: владелец не разрешал отчёты из этой беседы или уже выключил их"; exit 1; fi
  sleep 1
done
id="r-$(date +%s)-$$"
tmp="$root/.$id.tmp"
mkdir -p "$tmp/files"
cp -- "$report" "$tmp/report.md"
for f in "$@"; do
  [ -f "$f" ] || { echo "cloudos-report: нет файла: $f" >&2; rm -rf "${tmp:?}"; exit 2; }
  cp -- "$f" "$tmp/files/"
done
mv "$tmp" "$root/$id"
i=0
while [ "$i" -lt 120 ]; do
  if [ -f "$root/$id.status" ]; then
    line="$(cat "$root/$id.status")"; rm -f "${root:?}/${id:?}.status"
    echo "$line"
    case "$line" in sent*) exit 0;; *) exit 1;; esac
  fi
  sleep 1; i=$((i + 1))
done
echo "cloudos-report: сервис не ответил за 2 минуты; отчёт остался в ящике и может уйти позже" >&2
exit 3
"#;

#[derive(Default, Clone)]
struct Count { day: i64, in_day: u32, turn: u64, in_turn: u32, told_limit: bool }
struct Active { at: i64, boxed: bool, retry_at: i64 }

/// What the boxes keep in the process. The daily count starts again with the service: a restart is rare, and the count
/// is a guard against a loop, not a ledger.
#[derive(Default)]
pub struct State {
    /// By "project:session": when the conversation was last seen at work, whether its box is known to exist
    active: Mutex<HashMap<String, Active>>,
    counts: Mutex<HashMap<String, Count>>,
    /// Reports already taken ("project:session:name"): one that could not be removed from the box is not sent twice
    taken: Mutex<HashSet<String>>,
    /// Workspaces that have the script, and when it was put
    scripts: Mutex<HashMap<String, i64>>,
}

fn now() -> i64 { chrono::Utc::now().timestamp() }
fn short(s: &str, n: usize) -> String { let t: String = s.chars().take(n).collect(); if s.chars().count() > n { t + "…" } else { t } }

/// The note for the agent of a conversation: what the box is for and how to use it. `sid` is the name of the box.
pub fn intro(sid: &str) -> String {
    format!("{INTRO_HEAD}\nВладелец разрешил этой беседе присылать ему отчёты и файлы в его канал уведомлений. Чтобы прислать: запиши отчёт в Markdown-файл и выполни\n`sh {SCRIPT} {sid} <отчёт.md> [файлы…]`\nКоманда ждёт ответа и печатает, принят ли отчёт. До {MAX_FILES} файлов по {} МБ, до {PER_TURN} отчётов за ход. Ключи, токены и файлы с секретами (.env, ключи SSH) не прикладывай — такой отчёт не уйдёт. Присылай отчёт, когда закончена работа, о результате которой владелец хотел узнать, или когда он просил файл; по мелочам не пиши.", MAX_FILE / 1024 / 1024)
}

// ---------- workspace files through the core ----------

const FS_SECS: u64 = 15;
fn fs_url(app: &App, project: &str, tail: &str) -> String { format!("{}/api/projects/{project}/fs{tail}", app.cloudd) }
async fn fs_fail(r: reqwest::Response) -> anyhow::Error {
    let code = r.status().as_u16();
    let text = r.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string)).unwrap_or(text);
    anyhow!("ядро {code}: {}", short(&msg, 200))
}
async fn fs_list(app: &App, project: &str, path: &str) -> Result<Vec<Value>> {
    let r = app.http.get(fs_url(app, project, "")).query(&[("path", path)]).timeout(std::time::Duration::from_secs(FS_SECS)).send().await.map_err(|e| anyhow!("ядро не отвечает: {e}"))?;
    if !r.status().is_success() { return Err(fs_fail(r).await); }
    let v: Value = r.json().await.unwrap_or(Value::Null);
    Ok(v.get("entries").and_then(|e| e.as_array()).cloned().unwrap_or_default())
}
/// A file of a workspace, no larger than `cap`: read as it comes and dropped as soon as it turns out larger — what a
/// listing said about its size a moment ago binds nobody.
async fn fs_bytes(app: &App, project: &str, path: &str, cap: u64) -> Result<Vec<u8>> {
    let mut r = app.http.get(fs_url(app, project, "/file")).query(&[("path", path)]).timeout(std::time::Duration::from_secs(60)).send().await.map_err(|e| anyhow!("ядро не отвечает: {e}"))?;
    if !r.status().is_success() { return Err(fs_fail(r).await); }
    let mut out: Vec<u8> = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(|e| anyhow!("ядро: {e}"))? {
        if (out.len() + chunk.len()) as u64 > cap { bail!("больше {} МБ", (cap / 1024 / 1024).max(1)); }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
async fn fs_put(app: &App, project: &str, path: &str, body: Vec<u8>) -> Result<()> {
    let r = app.http.put(fs_url(app, project, "/file")).query(&[("path", path)]).body(body).timeout(std::time::Duration::from_secs(FS_SECS)).send().await.map_err(|e| anyhow!("ядро не отвечает: {e}"))?;
    if !r.status().is_success() { return Err(fs_fail(r).await); }
    Ok(())
}
async fn fs_mkdir(app: &App, project: &str, path: &str) -> Result<()> {
    let r = app.http.post(fs_url(app, project, "/mkdir")).json(&json!({ "path": path })).timeout(std::time::Duration::from_secs(FS_SECS)).send().await.map_err(|e| anyhow!("ядро не отвечает: {e}"))?;
    if !r.status().is_success() { return Err(fs_fail(r).await); }
    Ok(())
}
async fn fs_rm(app: &App, project: &str, path: &str) -> Result<()> {
    let r = app.http.delete(fs_url(app, project, "")).query(&[("path", path), ("recursive", "1")]).timeout(std::time::Duration::from_secs(FS_SECS)).send().await.map_err(|e| anyhow!("ядро не отвечает: {e}"))?;
    if !r.status().is_success() { return Err(fs_fail(r).await); }
    Ok(())
}
/// A grey line in the conversation. It is kept in the conversation's journal — a file in the workspace — so nothing in
/// it names a channel or repeats what a channel's service answered.
async fn say(app: &App, project: &str, sid: &str, line: &str) {
    let r = app.http.post(format!("{}/api/projects/{project}/sessions/{sid}/note", app.cloudd)).json(&json!({ "message": short(line, 300) })).timeout(std::time::Duration::from_secs(FS_SECS)).send().await;
    if let Err(e) = r { tracing::debug!("строка об отчёте не записана: {}", e.without_url()); }
}

/// The script and the root of the boxes in a workspace. Written again once a day: a workspace rebuilt from its
/// template has neither.
pub async fn ensure_script(app: &App, project: &str) {
    if !channels::plain_id(project) { return }
    if app.channels.reports.scripts.lock().unwrap().get(project).map(|t| now() - *t < 86400).unwrap_or(false) { return }
    let done = async {
        fs_mkdir(app, project, "~/.cloudos/bin").await?;
        fs_mkdir(app, project, ROOT).await?;
        fs_put(app, project, SCRIPT, SCRIPT_BODY.as_bytes().to_vec()).await
    }.await;
    match done { Ok(()) => { app.channels.reports.scripts.lock().unwrap().insert(project.to_string(), now()); } Err(e) => tracing::debug!("ящик отчётов в {project}: {e:#}") }
}

// ---------- a box is open while the owner allows reports ----------

/// The boxes that are open: (workspace, conversation, when it was opened).
fn boxes(app: &App) -> Vec<(String, String, i64)> { app.store.get_kv(KV_BOXES).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default() }
fn save_boxes(app: &App, list: &[(String, String, i64)]) { let _ = app.store.put_kv(KV_BOXES, &json!(list)); }
fn remember_box(app: &App, project: &str, sid: &str) {
    let mut list = boxes(app);
    if !list.iter().any(|(p, s, _)| p == project && s == sid) { list.push((project.to_string(), sid.to_string(), now())); save_boxes(app, &list); }
}

/// Open the box of a conversation: the directory, the script, and the mark the script looks for. The box is remembered,
/// so that it is closed when the owner takes the permission back (or a "once" binding has done its work).
pub async fn open_box(app: &App, project: &str, sid: &str) {
    if !channels::plain_id(project) || !channels::plain_id(sid) { return }
    remember_box(app, project, sid);
    ensure_script(app, project).await;
    let dir = format!("{ROOT}/{sid}");
    let done = async { fs_mkdir(app, project, &dir).await?; fs_put(app, project, &format!("{dir}/.open"), now().to_string().into_bytes()).await }.await;
    if let Err(e) = done { tracing::debug!("ящик {project}/{sid} не открыт: {e:#} — откроется, когда беседа начнёт работать"); }
}
/// Close the boxes that have no use any more: the conversation may not report (the owner took the permission back, a
/// "once" binding has done its work) or is over (a routine's run that ended; a conversation that was stopped or is
/// gone). The mark goes away, and the script of an agent that still remembers the box answers at once that reports are
/// off. A conversation that comes back to work has its box opened again at the first look. A workspace that is not
/// running is tried again later.
async fn close_boxes(app: &App, binds: &[channels::Binding]) {
    let list = boxes(app);
    if list.is_empty() { return }
    let running: HashSet<String> = app.hub.running();
    let alive: HashSet<String> = app.hub.sessions().into_iter().filter(|s| !matches!(s.status.as_str(), "exited" | "error")).map(|s| format!("{}:{}", s.project, s.id)).collect();
    let t = now();
    let mut keep = Vec::new();
    for (project, sid, at) in &list {
        if !running.contains(project) { keep.push((project.clone(), sid.clone(), *at)); continue }
        let allowed = !channels::reports_to(app, binds, project, sid).is_empty();
        // a conversation just made may not be in the snapshot yet: its box is not closed in its first minute
        let over = !alive.contains(&format!("{project}:{sid}")) && t - *at > 60;
        // closed when the mark is gone: removed now, or not there any more
        let closed = (!allowed || over) && (fs_rm(app, project, &format!("{ROOT}/{sid}/.open")).await.is_ok()
            || fs_list(app, project, &format!("{ROOT}/{sid}")).await.map(|e| !e.iter().any(|x| x.get("name").and_then(|n| n.as_str()) == Some(".open"))).unwrap_or(false));
        if !closed { keep.push((project.clone(), sid.clone(), *at)); }
    }
    if keep.len() != list.len() { save_boxes(app, &keep); }
}

// ---------- the note for the agent goes with the owner's next message ----------

/// `projects/<p>/sessions/<id>/prompt` (or `/queue`, `/edit`) of the core API → (project, session, is it an edit).
fn prompt_target(path: &str) -> Option<(String, String, bool)> {
    let p: Vec<&str> = path.trim_matches('/').split('/').collect();
    if p.len() == 5 && p[0] == "projects" && p[2] == "sessions" && matches!(p[4], "prompt" | "queue" | "edit") { Some((p[1].to_string(), p[3].to_string(), p[4] == "edit")) } else { None }
}
/// The text of a message as the owner typed it: the string, or the text blocks of a list of blocks.
fn typed(content: &Value) -> String {
    match content { Value::String(s) => s.clone(), Value::Array(b) => b.iter().filter_map(|x| x.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join("\n"), _ => String::new() }
}
/// The owner's message on its way to a conversation whose agent has not been told about the box yet: the note is put
/// after the text (the first line of the first message becomes the name of the conversation — it must stay the
/// owner's). Returns the new body and the binding to mark once the core has taken the message. A body that is not
/// what we expect goes on untouched.
///
/// A command (`/model`, `/compact`…) is not a message for the agent: the note does not go with it. After `/clear` and
/// `/compact` the agent no longer remembers the note — it is to be told again with the next real message.
pub fn with_intro(app: &App, path: &str, body: &[u8]) -> Option<(Vec<u8>, String)> {
    let (project, session, edit) = prompt_target(path)?;
    let mine = |b: &channels::Binding| b.scope == "session" && b.reports && b.project == project && b.session == session;
    let all = channels::bindings(app);
    if !all.iter().any(|b| mine(b)) { return None }
    let mut v: Value = serde_json::from_slice(body).ok()?;
    let text = typed(v.get("content")?);
    let said = text.trim();
    if said.starts_with('/') && !said.contains('\n') {
        if said == "/clear" || said.starts_with("/clear ") || said == "/compact" || said.starts_with("/compact ") {
            let mut all = all;
            for b in all.iter_mut().filter(|b| mine(b)) { b.intro_sent = false; }
            let _ = channels::save_bindings(app, &all);
        }
        return None;
    }
    // a message rewritten by the owner takes the conversation back to that point — the note may have been after it, so
    // an edit always carries the note again
    let b = all.into_iter().find(|b| mine(b) && (edit || !b.intro_sent))?;
    let note = intro(&session);
    let joined = |old: &str| if old.trim().is_empty() { note.clone() } else { format!("{old}\n\n---\n{note}") };
    match v.get_mut("content")? {
        Value::String(s) => { *s = joined(s); }
        Value::Array(blocks) => {
            match blocks.iter_mut().rev().find(|x| x.get("type").and_then(|t| t.as_str()) == Some("text")) {
                Some(t) => { let old = t.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string(); t["text"] = json!(joined(&old)); }
                None => blocks.push(json!({ "type": "text", "text": note })),
            }
        }
        _ => return None,
    }
    Some((serde_json::to_vec(&v).ok()?, b.id))
}
/// The core took the message: the agent of that conversation knows about its box now.
pub async fn intro_delivered(app: &App, binding: &str) {
    let mut all = channels::bindings(app);
    let Some((project, session)) = all.iter().find(|b| b.id == binding).map(|b| (b.project.clone(), b.session.clone())) else { return };
    // one note serves every channel bound to the conversation
    for b in all.iter_mut().filter(|b| b.scope == "session" && b.project == project && b.session == session) { b.intro_sent = true; }
    let _ = channels::save_bindings(app, &all);
    open_box(app, &project, &session).await;
}

// ---------- looking into the boxes ----------

/// One look: the boxes of the conversations that may report and are at work (or were a moment ago). An idle
/// conversation costs nothing here — a report is put by the script, and the script waits for the answer inside a turn.
pub async fn sweep(app: &App) {
    let binds = channels::bindings(app);
    close_boxes(app, &binds).await;
    if !binds.iter().any(|b| b.reports) { app.channels.reports.active.lock().unwrap().clear(); return }
    let t = now();
    let mut todo: Vec<(String, String, String, u64, bool, i64, bool)> = Vec::new();
    let mut live: HashSet<String> = HashSet::new();
    for s in app.hub.sessions() {
        // the names come from the workspace and go into addresses of the core: plain ones only
        if !channels::plain_id(&s.project) || !channels::plain_id(&s.id) { continue }
        let key = format!("{}:{}", s.project, s.id);
        let busy = matches!(s.status.as_str(), "working" | "waiting" | "starting");
        let known = app.channels.reports.active.lock().unwrap().contains_key(&key);
        if !busy && !known { continue }
        let allowed = channels::reports_to(app, &binds, &s.project, &s.id);
        if allowed.is_empty() { continue }
        // a report counts from the moment reports were allowed: what lay in the box before is not one the owner asked for
        let since = allowed.iter().map(|b| if b.reports_at > 0 { b.reports_at } else { b.created_at }).min().unwrap_or(0);
        let mut g = app.channels.reports.active.lock().unwrap();
        let a = g.entry(key.clone()).or_insert(Active { at: t, boxed: false, retry_at: 0 });
        if busy { a.at = t; }
        if t - a.at > LINGER { continue }
        live.insert(key);
        if a.retry_at > t { continue }
        todo.push((s.project, s.id, s.title, s.turns, a.boxed, since, busy));
    }
    app.channels.reports.active.lock().unwrap().retain(|k, _| live.contains(k));
    for (project, sid, title, turns, boxed, since, busy) in todo { look(app, &project, &sid, &title, turns, boxed, since, busy).await; }
}

fn mark(app: &App, key: &str, boxed: bool, retry_in: i64) {
    if let Some(a) = app.channels.reports.active.lock().unwrap().get_mut(key) { a.boxed = boxed; a.retry_at = if retry_in > 0 { now() + retry_in } else { 0 }; }
}

#[allow(clippy::too_many_arguments)]
async fn look(app: &App, project: &str, sid: &str, title: &str, turns: u64, boxed: bool, since: i64, busy: bool) {
    let key = format!("{project}:{sid}");
    let dir = format!("{ROOT}/{sid}");
    if !boxed {
        ensure_script(app, project).await;
        if let Err(e) = fs_mkdir(app, project, &dir).await { tracing::debug!("ящик {key}: {e:#}"); mark(app, &key, false, 30); return }
        mark(app, &key, true, 0);
    }
    let entries = match fs_list(app, project, &dir).await { Ok(e) => e, Err(e) => { tracing::debug!("ящик {key}: {e:#}"); mark(app, &key, false, 30); return } };
    let t = now();
    let mut names: Vec<(String, i64)> = Vec::new();
    let mut open = false;
    for e in &entries {
        let name = e.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let dir_kind = e.get("kind").and_then(|k| k.as_str()) == Some("dir");
        let mtime = e.get("mtime").and_then(|m| m.as_i64()).unwrap_or(t);
        if name == ".open" { open = true; continue }
        let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !plain { continue }
        if dir_kind && name.starts_with("r-") && e.get("link").and_then(|l| l.as_bool()) != Some(true) { names.push((name.to_string(), mtime)); }
        // what a killed script left behind: an answer nobody read, a report half put
        else if (name.ends_with(".status") && t - mtime > 600) || (name.starts_with(".r-") && name.ends_with(".tmp") && t - mtime > 3600) { let _ = fs_rm(app, project, &format!("{dir}/{name}")).await; }
    }
    // the conversation may report and is at work: its box is open (the mark may be missing after the workspace was
    // rebuilt, or the box was closed when the conversation stopped). A conversation that has just finished gets its
    // last reports taken, but its box is not opened again.
    if !open && busy { remember_box(app, project, sid); let _ = fs_put(app, project, &format!("{dir}/.open"), t.to_string().into_bytes()).await; }
    names.sort();
    for (name, mtime) in names.into_iter().take(10) {
        let id = format!("{key}:{name}");
        if app.channels.reports.taken.lock().unwrap().contains(&id) { let _ = fs_rm(app, project, &format!("{dir}/{name}")).await; continue }
        // put before reports were allowed, or lying here for long: not a report the owner is waiting for
        let stale = if mtime + 5 < since { Some("отчёт положен до того, как владелец разрешил отчёты") } else if t - mtime > REPORT_MAX_AGE { Some("отчёт пролежал слишком долго — пришли его заново") } else { None };
        match stale {
            Some(why) => { let _ = fs_rm(app, project, &format!("{dir}/{name}")).await; let _ = fs_put(app, project, &format!("{dir}/{name}.status"), format!("refused: {why}").into_bytes()).await; }
            None => take(app, project, sid, title, turns, &name).await,
        }
        let mut g = app.channels.reports.taken.lock().unwrap();
        if g.len() > 2000 { g.clear(); }
        g.insert(id);
    }
}

struct Report { md: String, files: Vec<MsgFile>, local: std::path::PathBuf }

/// Whether the conversation may hand in one more report now; counts it when it may.
fn within_limits(app: &App, key: &str, turns: u64) -> Result<(), (String, bool)> {
    let day = now() / 86400;
    let mut g = app.channels.reports.counts.lock().unwrap();
    if g.len() > 500 { g.retain(|_, c| c.day == day); }
    let c = g.entry(key.to_string()).or_default();
    if c.day != day { *c = Count { day, turn: turns, ..Default::default() }; }
    if c.turn != turns { c.turn = turns; c.in_turn = 0; c.told_limit = false; }
    let over = if c.in_day >= PER_DAY { Some(format!("предел отчётов: {PER_DAY} в сутки на беседу")) } else if c.in_turn >= PER_TURN { Some(format!("предел отчётов: {PER_TURN} за ход")) } else { None };
    if let Some(why) = over { let first = !c.told_limit; c.told_limit = true; return Err((why, first)); }
    c.in_day += 1; c.in_turn += 1;
    Ok(())
}

/// A file name as it goes out: the last part of the path, nothing a chat would not show.
fn safe_name(name: &str) -> String {
    let n: String = name.rsplit('/').next().unwrap_or(name).chars().filter(|c| !c.is_control()).take(120).collect();
    if n.trim().is_empty() || n == "." || n == ".." { "file".into() } else { n }
}
fn looks_text(data: &[u8]) -> bool { !data.iter().take(8192).any(|&b| b == 0) }

/// Read a report out of the box and check it. The files land in the service's own directory; a refusal leaves nothing
/// behind. Only what the listing calls a plain file of some size is read at all — a pipe or a device looks like an empty
/// file there and is never opened — and never more than the size allowed. An archive or a binary file is not looked
/// into: that is the honest limit of this check.
async fn read_report(app: &App, project: &str, dir: &str) -> Result<Report> {
    let entries = fs_list(app, project, dir).await.map_err(|e| anyhow!("ящик не читается: {e:#}"))?;
    let entry = |name: &str| entries.iter().find(|e| e.get("name").and_then(|n| n.as_str()) == Some(name));
    let size_of = |e: &Value| e.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
    let regular = |e: &Value| e.get("kind").and_then(|k| k.as_str()) == Some("file") && e.get("link").and_then(|l| l.as_bool()) != Some(true);
    let rep = entry("report.md").filter(|e| regular(e)).ok_or_else(|| anyhow!("в отчёте нет файла report.md"))?;
    let too_long = || anyhow!("отчёт длиннее {MAX_TEXT} знаков — приложите его файлом");
    let text_cap = (MAX_TEXT * 4) as u64;
    if size_of(rep) > text_cap { return Err(too_long()); }
    let md = if size_of(rep) == 0 { String::new() } else { String::from_utf8_lossy(&fs_bytes(app, project, &format!("{dir}/report.md"), text_cap).await.map_err(|_| too_long())?).to_string() };
    if md.chars().count() > MAX_TEXT { return Err(too_long()); }
    let has_files = entry("files").map(|e| e.get("kind").and_then(|k| k.as_str()) == Some("dir") && e.get("link").and_then(|l| l.as_bool()) != Some(true)).unwrap_or(false);
    let listed = if has_files { fs_list(app, project, &format!("{dir}/files")).await? } else { Vec::new() };
    if listed.len() > MAX_FILES { bail!("файлов {}, а можно не больше {MAX_FILES}", listed.len()); }
    for e in &listed {
        let name = e.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if !regular(e) || name.contains('/') { bail!("«{}» — не обычный файл", short(name, 80)); }
        if size_of(e) > MAX_FILE { bail!("файл {} больше {} МБ", short(name, 80), MAX_FILE / 1024 / 1024); }
        if size_of(e) == 0 { bail!("файл {} пуст", short(name, 80)); }
        if chanfmt::secret_name(name) { bail!("файл {} похож на секреты — такие не отправляются", short(name, 80)); }
    }
    if md.trim().is_empty() && listed.is_empty() { bail!("отчёт пуст"); }
    let local = app.data.join("notify-files").join(format!("in_{}", uuid::Uuid::new_v4().simple()));
    let mut files: Vec<MsgFile> = Vec::new();
    let got = async {
        tokio::fs::create_dir_all(&local).await?;
        for (i, e) in listed.iter().enumerate() {
            let name = e.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let data = fs_bytes(app, project, &format!("{dir}/files/{name}"), MAX_FILE).await.map_err(|err| anyhow!("файл {}: {err:#}", short(name, 80)))?;
            let (data, secret) = if looks_text(&data) { tokio::task::spawn_blocking(move || { let s = chanfmt::has_secret(&String::from_utf8_lossy(&data)); (data, s) }).await? } else { (data, false) };
            if secret { bail!("в файле {} найден ключ или токен — отчёт не отправлен", short(name, 80)); }
            // every file in a directory of its own: two files of a report may not share a name on the way out
            let path = local.join(i.to_string()).join(safe_name(name));
            if let Some(d) = path.parent() { tokio::fs::create_dir_all(d).await?; }
            tokio::fs::write(&path, &data).await?;
            files.push(MsgFile { name: safe_name(name), path: path.to_string_lossy().to_string(), size: data.len() as u64 });
        }
        Ok(())
    }.await;
    if let Err(e) = got { let _ = tokio::fs::remove_dir_all(&local).await; return Err(e); }
    Ok(Report { md, files, local })
}

/// How the jobs of a report ended: (delivered, refused, still on their way). The queue is helped along here rather than
/// waited for — a job is taken by whoever comes first, so two hands on the queue never send the same thing twice.
async fn outcome(app: &App, jobs: &[String]) -> (usize, usize, usize) {
    let count = |app: &App| jobs.iter().fold((0usize, 0usize, 0usize), |(s, f, p), id| match app.store.outbox_job(id).ok().flatten().map(|j| j.status) { Some(st) if st == "sent" => (s + 1, f, p), Some(st) if st == "failed" => (s, f + 1, p), _ => (s, f, p + 1) });
    for _ in 0..40 {
        channels::pump(app).await;
        let c = count(app);
        if c.2 == 0 { return c }
        tokio::time::sleep(std::time::Duration::from_millis(if cfg!(test) { 5 } else { 500 })).await;
    }
    count(app)
}

/// One report: read and checked, taken out of the box, handed to the channels; the answer goes to the script (the
/// `.status` file next to the report) and into the conversation as a grey line.
async fn take(app: &App, project: &str, sid: &str, title: &str, turns: u64, name: &str) {
    let key = format!("{project}:{sid}");
    let dir = format!("{ROOT}/{sid}/{name}");
    let (status, note) = match within_limits(app, &key, turns) {
        Err((why, first)) => (format!("refused: {why}"), first.then(|| format!("📤 отчёт агента не отправлен: {why}"))),
        Ok(()) => match read_report(app, project, &dir).await {
            Err(e) => { let why = short(&format!("{e:#}"), 240); (format!("refused: {why}"), Some(format!("📤 отчёт агента не отправлен: {why}"))) }
            Ok(rep) => {
                let what = chanfmt::title_of(&rep.md);
                let n = rep.files.len();
                let attached = if n > 0 { format!(" · файлов: {n}") } else { String::new() };
                let (queued, failed) = channels::report(app, project, sid, title, &rep.md, rep.files, Vec::new()).await;
                let _ = tokio::fs::remove_dir_all(&rep.local).await;
                if queued.is_empty() {
                    let why = if failed.is_empty() { "нет включённого канала, которому разрешены отчёты этой беседы" } else { "очередь доставки недоступна" };
                    (format!("refused: {why}"), Some(format!("📤 отчёт «{what}» не отправлен: {why}")))
                } else {
                    let jobs: Vec<String> = queued.iter().map(|q| q.1.clone()).collect();
                    let (sent, refused, waiting) = outcome(app, &jobs).await;
                    // neither the answer nor the line names a channel or repeats its service: both are read inside the workspace
                    let all = jobs.len();
                    let how = if sent == all { "отправлен владельцу ✓".to_string() }
                        else if sent + waiting == 0 { "не доставлен — причина в журнале канала (Настройки → Уведомления)".to_string() }
                        else if refused == 0 { "принят, доставляется".to_string() }
                        else { format!("доставлен не во все каналы ({} из {all}) — причина в журнале канала", sent + waiting) };
                    let st = if sent + waiting > 0 { format!("sent: отчёт принят и отправлен владельцу (каналов: {})", sent + waiting) } else { "refused: канал владельца не принял отчёт — попробуй позже".to_string() };
                    (st, Some(format!("📤 отчёт «{what}»{attached} — {how}")))
                }
            }
        },
    };
    // the report leaves the box before the answer is written: the script reads the answer and the agent may go on
    if let Err(e) = fs_rm(app, project, &dir).await { tracing::debug!("ящик {key}: отчёт {name} не убран: {e:#}"); }
    if let Err(e) = fs_put(app, project, &format!("{ROOT}/{sid}/{name}.status"), status.clone().into_bytes()).await { tracing::debug!("ящик {key}: ответ не записан: {e:#}"); }
    tracing::info!("отчёт из {project}/{}: {}", sid.chars().take(8).collect::<String>(), status.split(':').next().unwrap_or(""));
    if let Some(n) = note { say(app, project, sid, &n).await; }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::rig::*;
    use crate::channels::Binding;

    fn boxed(w: &World, sid: &str, name: &str, md: &str, files: &[(&str, &[u8])]) {
        w.put(&format!("{ROOT}/{sid}/{name}/report.md"), md.as_bytes());
        for (n, d) in files { w.put(&format!("{ROOT}/{sid}/{name}/files/{n}"), d); }
    }
    fn answer(w: &World, sid: &str, name: &str) -> Option<String> { w.files.lock().unwrap().get(&format!("{ROOT}/{sid}/{name}.status")).map(|d| String::from_utf8_lossy(d).to_string()) }
    fn left(w: &World, sid: &str, name: &str) -> bool { w.dirs.lock().unwrap().contains(&format!("{ROOT}/{sid}/{name}")) }
    fn open(w: &World, sid: &str) -> bool { w.files.lock().unwrap().contains_key(&format!("{ROOT}/{sid}/.open")) }
    fn reporting(app: &App, channel: &str, sid: &str) -> Binding { bound(app, Binding { channel: channel.into(), scope: "session".into(), project: "ws".into(), session: sid.into(), when: "always".into(), content: "fact".into(), reports: true, reports_at: now(), ..Default::default() }) }
    /// Everything the workspace holds or was told: its files and the lines written into its conversations
    fn inside(w: &World) -> String { w.files.lock().unwrap().values().map(|d| String::from_utf8_lossy(d).to_string()).chain(w.notes.lock().unwrap().iter().map(|n| n.1.clone())).collect::<Vec<_>>().join("\n") }

    /// A report with files: formatted for the channel, the files next to it, the answer for the script, a line in the
    /// conversation — and the agent has seen neither a key nor an address nor the name of the channel.
    #[tokio::test]
    async fn a_report_with_files_reaches_the_bound_channel() {
        let (app, w, dir) = world().await;
        telegram(&app, &w, "ch_tg", false);
        reporting(&app, "ch_tg", "s1");
        app.hub.set_projects(snap("s1", "Исследование", "working", 0));
        boxed(&w, "s1", "r-100-1", "# Итоги исследования рынка\n\n- три поставщика подходят по **цене**\n- см. `data.csv`\n\n| имя | цена |\n|---|---|\n| A | 10 |\n", &[("chart.png", &[0x89, b'P', b'N', b'G', 0, 1, 2, 3]), ("data.csv", b"name,price\nA,10\n")]);
        sweep(&app).await;
        let msgs = w.at("/sendMessage");
        assert_eq!(msgs.len(), 1);
        let text = msgs[0]["text"].as_str().unwrap();
        assert!(text.starts_with("<b>📄 ws · Исследование</b> — отчёт агента\n\n<b>Итоги исследования рынка</b>\n\n• три поставщика подходят по <b>цене</b>\n• см. <code>data.csv</code>"), "{text}");
        assert!(text.contains("<pre>имя │ цена\nA   │ 10</pre>") && text.ends_with("<a href=\"https://stand.example/agents/?open=ws/s1\">Открыть</a>"), "{text}");
        let docs = w.texts("/sendDocument");
        assert_eq!(docs.len(), 2);
        assert!(docs[0].contains("filename=\"chart.png\"") && docs[1].contains("filename=\"data.csv\"") && docs[1].contains("name,price"));
        assert_eq!(answer(&w, "s1", "r-100-1").as_deref(), Some("sent: отчёт принят и отправлен владельцу (каналов: 1)"));
        assert!(!left(&w, "s1", "r-100-1"), "the report is taken out of the box");
        assert_eq!(*w.notes.lock().unwrap(), vec![("s1".to_string(), "📤 отчёт «Итоги исследования рынка» · файлов: 2 — отправлен владельцу ✓".to_string())]);
        // the workspace got the script and an open box; nothing in it — its files, the line in the conversation — names
        // a key, an address or the channel itself
        let script = String::from_utf8(w.files.lock().unwrap().get(SCRIPT).cloned().unwrap()).unwrap();
        assert!(script.starts_with("#!/bin/sh") && script.contains(".cloudos/outbox/$box") && script.contains(".open"));
        assert!(open(&w, "s1"));
        let seen = inside(&w);
        assert!(!seen.contains("AAHdq") && !seen.contains("api.telegram") && !seen.contains("Личный") && !seen.contains("stand_bot"), "the workspace holds no secret and no addressee");
        // nothing is left in the service's directory, and a second look sends nothing again
        assert_eq!(std::fs::read_dir(dir.join("notify-files")).unwrap().count(), 0);
        sweep(&app).await;
        assert_eq!(w.count("/sendMessage"), 1); assert_eq!(w.notes.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What does not leave: secrets by name and by content, too many or too large files, an empty report. The agent is
    /// told why in the answer of the script, the owner — by a line in the conversation.
    #[tokio::test]
    async fn refusals_by_name_content_and_size() {
        let (app, w, dir) = world().await;
        telegram(&app, &w, "ch_tg", false);
        reporting(&app, "ch_tg", "s1");
        app.hub.set_projects(snap("s1", "Исследование", "working", 0));
        let key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
        let cut = b"-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA7 (the end line was lost on the way)\n";
        let big = vec![b'x'; MAX_FILE as usize + 1];
        let cases: Vec<(&str, &str, Vec<(&str, &[u8])>, &str)> = vec![
            ("r-1", "Дамп окружения", vec![(".env", b"A=1\n")], "файл .env похож на секреты"),
            ("r-2", "Ключ", vec![("notes.txt", key)], "в файле notes.txt найден ключ или токен"),
            ("r-3", "Много", vec![("1.txt", b"1"), ("2.txt", b"2"), ("3.txt", b"3"), ("4.txt", b"4"), ("5.txt", b"5"), ("6.txt", b"6")], "файлов 6, а можно не больше 5"),
            ("r-4", "  \n", vec![], "отчёт пуст"),
            ("r-5", "Большой", vec![("dump.bin", &big)], "файл dump.bin больше 20 МБ"),
            ("r-6", "Пустой файл", vec![("empty.txt", b"")], "файл empty.txt пуст"),
            ("r-7", "Обрезанный ключ", vec![("part.txt", cut)], "в файле part.txt найден ключ или токен"),
        ];
        // five reports a turn: the rest go in the next turn
        for (name, md, files, _) in &cases[..5] { boxed(&w, "s1", name, md, files); }
        sweep(&app).await;
        app.hub.set_projects(snap("s1", "Исследование", "working", 1));
        for (name, md, files, _) in &cases[5..] { boxed(&w, "s1", name, md, files); }
        sweep(&app).await;
        for (name, _, _, why) in &cases {
            let a = answer(&w, "s1", name).unwrap_or_default();
            assert!(a.starts_with("refused: ") && a.contains(why), "{name}: {a}");
            assert!(!left(&w, "s1", name), "{name} is taken out all the same");
        }
        assert_eq!(w.count("/sendMessage") + w.count("/sendDocument"), 0, "nothing left the stand");
        let notes: Vec<String> = w.notes.lock().unwrap().iter().map(|n| n.1.clone()).collect();
        assert_eq!(notes.len(), 7); assert!(notes[0].starts_with("📤 отчёт агента не отправлен: файл .env похож на секреты"), "{notes:?}");
        assert_eq!(std::fs::read_dir(dir.join("notify-files")).map(|d| d.count()).unwrap_or(0), 0, "a refused report leaves nothing behind");
        // a report without report.md, a file that is a directory
        app.hub.set_projects(snap("s1", "Исследование", "working", 2));
        w.mkdir(&format!("{ROOT}/s1/r-8/files"));
        boxed(&w, "s1", "r-9", "Каталог", &[]); w.mkdir(&format!("{ROOT}/s1/r-9/files/inner"));
        sweep(&app).await;
        assert!(answer(&w, "s1", "r-8").unwrap().contains("нет файла report.md"));
        assert!(answer(&w, "s1", "r-9").unwrap().contains("не обычный файл"));
        // secrets in the text itself are closed, the report goes
        boxed(&w, "s1", "r-a", "Токен был ghp_abcdefghijklmnopqrstuvwxyz0123456789, дальше выводы.", &[]);
        sweep(&app).await;
        let text = w.at("/sendMessage")[0]["text"].as_str().unwrap().to_string();
        assert!(text.contains("Токен был «скрыто», дальше выводы.") && !text.contains("ghp_"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Only what a listing calls a plain file of some size is read at all, and never more than is allowed: a pipe looks
    /// like an empty file and is not opened; a file that turned out larger than it was listed is dropped while being read.
    #[tokio::test]
    async fn nothing_is_read_that_is_not_a_plain_file_of_the_allowed_size() {
        let (app, w, dir) = world().await;
        telegram(&app, &w, "ch_tg", false);
        reporting(&app, "ch_tg", "s1");
        app.hub.set_projects(snap("s1", "Беседа", "working", 0));
        // a pipe named report.md: the listing gives it as a file of no size — it is not read, the files go with the first line alone
        boxed(&w, "s1", "r-1", "этого никто не прочтёт", &[("data.csv", b"a,b\n")]);
        w.fake_size.lock().unwrap().insert(format!("{ROOT}/s1/r-1/report.md"), 0);
        sweep(&app).await;
        assert!(!w.reads.lock().unwrap().iter().any(|p| p.ends_with("r-1/report.md")), "a file of no size is not opened: {:?}", w.reads.lock().unwrap());
        assert!(answer(&w, "s1", "r-1").unwrap().starts_with("sent: "));
        assert!(!w.at("/sendMessage")[0]["text"].as_str().unwrap().contains("никто не прочтёт"));
        // a file listed as small that is in fact larger than the limit: dropped while being read, the report refused
        let big = vec![b'x'; MAX_FILE as usize + 4096];
        boxed(&w, "s1", "r-2", "Растущий файл", &[("grow.log", &big)]);
        w.fake_size.lock().unwrap().insert(format!("{ROOT}/s1/r-2/files/grow.log"), 1024);
        sweep(&app).await;
        let a = answer(&w, "s1", "r-2").unwrap();
        assert!(a.starts_with("refused: файл grow.log") && a.contains("больше 20 МБ"), "{a}");
        assert_eq!(w.count("/sendDocument"), 1, "only the file of the first report went");
        // the report text itself has its own bound
        let long = "ы".repeat(MAX_TEXT + 1);
        boxed(&w, "s1", "r-3", &long, &[]);
        sweep(&app).await;
        assert!(answer(&w, "s1", "r-3").unwrap().contains("отчёт длиннее 200000 знаков"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Five reports a turn; the rest are refused, and the conversation gets one line about it, not a line per attempt.
    #[tokio::test]
    async fn the_number_of_reports_is_bounded() {
        let (app, w, dir) = world().await;
        telegram(&app, &w, "ch_tg", false);
        reporting(&app, "ch_tg", "s1");
        app.hub.set_projects(snap("s1", "Цикл", "working", 3));
        for i in 0..8 { boxed(&w, "s1", &format!("r-{i}"), &format!("Отчёт {i}"), &[]); }
        sweep(&app).await;
        assert_eq!(w.count("/sendMessage"), PER_TURN as usize);
        for i in 0..5 { assert!(answer(&w, "s1", &format!("r-{i}")).unwrap().starts_with("sent: "), "{i}"); }
        for i in 5..8 { assert_eq!(answer(&w, "s1", &format!("r-{i}")).as_deref(), Some("refused: предел отчётов: 5 за ход"), "{i}"); }
        let limit_notes = w.notes.lock().unwrap().iter().filter(|n| n.1.contains("предел отчётов")).count();
        assert_eq!(limit_notes, 1);
        // the next turn may report again
        app.hub.set_projects(snap("s1", "Цикл", "working", 4));
        boxed(&w, "s1", "r-next", "Отчёт следующего хода", &[]);
        sweep(&app).await;
        assert!(answer(&w, "s1", "r-next").unwrap().starts_with("sent: "));
        // the day's bound
        app.channels.reports.counts.lock().unwrap().get_mut("ws:s1").unwrap().in_day = PER_DAY;
        app.hub.set_projects(snap("s1", "Цикл", "working", 5));
        boxed(&w, "s1", "r-day", "Ещё один", &[]);
        sweep(&app).await;
        assert_eq!(answer(&w, "s1", "r-day").as_deref(), Some("refused: предел отчётов: 20 в сутки на беседу"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A box is looked into only where the owner allowed reports, and only while the conversation is at work; what was
    /// put before the permission is not a report; a channel that refuses is said so — without its name and its words.
    #[tokio::test]
    async fn only_allowed_conversations_at_work_and_an_honest_outcome() {
        let (app, w, dir) = world().await;
        telegram(&app, &w, "ch_tg", false);
        // bound, but without "the agent may hand in reports"
        bound(&app, Binding { channel: "ch_tg".into(), scope: "session".into(), project: "ws".into(), session: "s1".into(), when: "always".into(), content: "full".into(), ..Default::default() });
        app.hub.set_projects(snap("s1", "Беседа", "working", 0));
        boxed(&w, "s1", "r-1", "Попытка до разрешения", &[]);
        sweep(&app).await;
        assert!(left(&w, "s1", "r-1") && answer(&w, "s1", "r-1").is_none() && w.got.lock().unwrap().is_empty(), "a box nobody opened for reports is not even looked into");
        assert!(!open(&w, "s1"), "and it carries no mark of an open box");
        // the owner allows reports a minute later; the conversation is idle: nothing is asked of the workspace
        w.age(&format!("{ROOT}/s1/r-1"), 60);
        let mut all = channels::bindings(&app); all[0].reports = true; all[0].reports_at = now(); channels::save_bindings(&app, &all).unwrap();
        app.hub.set_projects(snap("s1", "Беседа", "idle", 0));
        sweep(&app).await;
        assert!(left(&w, "s1", "r-1"));
        // at work: what lay in the box before the permission is not a report the owner asked for
        app.hub.set_projects(snap("s1", "Беседа", "working", 0));
        sweep(&app).await;
        assert_eq!(answer(&w, "s1", "r-1").as_deref(), Some("refused: отчёт положен до того, как владелец разрешил отчёты"));
        assert!(!left(&w, "s1", "r-1") && w.count("/sendMessage") == 0 && open(&w, "s1"));
        // a report lying for too long is not news either
        boxed(&w, "s1", "r-old", "Вчерашний", &[]); w.age(&format!("{ROOT}/s1/r-old"), REPORT_MAX_AGE + 10);
        let mut all = channels::bindings(&app); all[0].reports_at = now() - 86400; channels::save_bindings(&app, &all).unwrap();
        sweep(&app).await;
        assert!(answer(&w, "s1", "r-old").unwrap().contains("пролежал слишком долго"));
        // a fresh one goes; the channel refuses it — and that is what everybody is told, without the channel's name or words
        boxed(&w, "s1", "r-2", "Попытка", &[]);
        w.say("sendMessage", 403, json!({ "ok": false, "error_code": 403, "description": "Forbidden: bot was blocked by the user" }));
        sweep(&app).await;
        assert_eq!(answer(&w, "s1", "r-2").as_deref(), Some("refused: канал владельца не принял отчёт — попробуй позже"));
        assert_eq!(w.notes.lock().unwrap().last().unwrap().1, "📤 отчёт «Попытка» — не доставлен — причина в журнале канала (Настройки → Уведомления)");
        assert!(!inside(&w).contains("Личный") && !inside(&w).contains("blocked by the user"));
        // a moment after the turn the box is still looked into (the script may have been late), later it is not
        app.hub.set_projects(snap("s1", "Беседа", "idle", 1));
        boxed(&w, "s1", "r-3", "Вдогонку", &[]);
        sweep(&app).await;
        assert!(answer(&w, "s1", "r-3").unwrap().starts_with("sent: "));
        app.channels.reports.active.lock().unwrap().get_mut("ws:s1").unwrap().at = now() - LINGER - 1;
        boxed(&w, "s1", "r-4", "Слишком поздно", &[]);
        sweep(&app).await;
        assert!(left(&w, "s1", "r-4") && answer(&w, "s1", "r-4").is_none());
        // a channel switched off: the agent is told there is nowhere to send
        let mut c = channels::channels(&app); c[0].enabled = false; app.store.put_kv("notify.channels", &serde_json::to_value(&c).unwrap()).unwrap();
        app.hub.set_projects(snap("s1", "Беседа", "working", 1));
        sweep(&app).await;
        assert!(answer(&w, "s1", "r-4").unwrap().contains("нет включённого канала"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The box is open while reports are allowed and is closed when the owner takes the permission back (or a "once"
    /// binding has done its work): the script of an agent that still remembers the box then answers at once.
    #[tokio::test]
    async fn a_box_is_closed_when_reports_are_taken_back() {
        let (app, w, dir) = world().await;
        telegram(&app, &w, "ch_tg", false);
        app.hub.set_projects(snap("s1", "Беседа", "idle", 0));
        let body = |reports: bool| channels::BindBody { channel: "ch_tg".into(), scope: "session".into(), project: "ws".into(), session: "s1".into(), routine: String::new(), when: Some("once".into()), content: Some("fact".into()), reports: Some(reports), away_only: None, origin: None };
        let axum::Json(b) = channels::bind(axum::extract::State(app.clone()), no_headers(), axum::Json(body(true))).await.ok().unwrap();
        assert!(open(&w, "s1"), "allowing reports opens the box at once");
        assert!(b["reports_at"].as_i64().unwrap() >= now() - 2);
        sweep(&app).await; assert!(open(&w, "s1"));
        // the permission is taken back
        let axum::Json(_) = channels::bind(axum::extract::State(app.clone()), no_headers(), axum::Json(body(false))).await.ok().unwrap();
        sweep(&app).await;
        assert!(!open(&w, "s1"), "the mark is gone: the script refuses without waiting");
        assert!(boxes(&app).is_empty());
        // allowed again, then the binding itself goes away (a "once" binding after its turn, a removed channel)
        let axum::Json(b) = channels::bind(axum::extract::State(app.clone()), no_headers(), axum::Json(body(true))).await.ok().unwrap();
        assert!(open(&w, "s1"));
        channels::unbind(axum::extract::State(app.clone()), axum::extract::Path(b["id"].as_str().unwrap().to_string())).await.ok().unwrap();
        // the workspace is not running: the box stays remembered and is closed when the workspace is back
        app.hub.set_projects(vec![json!({ "name": "ws", "status": "stopped", "sessions": [] })]);
        sweep(&app).await;
        assert!(open(&w, "s1") && boxes(&app).len() == 1);
        app.hub.set_projects(snap("s1", "Беседа", "idle", 1));
        sweep(&app).await;
        assert!(!open(&w, "s1") && boxes(&app).is_empty());
        // a routine's run: its box is opened at the start and closed when the run's conversation is over — the binding of
        // the routine stays, the boxes of its past runs do not pile up
        bound(&app, Binding { channel: "ch_tg".into(), scope: "routine".into(), routine: "rt".into(), when: "always".into(), content: "fact".into(), reports: true, reports_at: now(), ..Default::default() });
        app.store.put_run(&crate::routines::Run { id: "run_1".into(), routine: Some("rt".into()), session: Some("s9".into()), project: "ws".into(), status: "running".into(), ..Default::default() }).unwrap();
        open_box(&app, "ws", "s9").await;
        app.hub.set_projects(snap("s9", "Ночной прогон", "working", 0));
        sweep(&app).await; assert!(open(&w, "s9"));
        app.hub.set_projects(snap("s9", "Ночной прогон", "exited", 1));
        sweep(&app).await; assert!(open(&w, "s9"), "not in its first minute: a conversation just made may be missing from the snapshot");
        let mut l = boxes(&app); l[0].2 -= 120; save_boxes(&app, &l);
        sweep(&app).await;
        assert!(!open(&w, "s9") && boxes(&app).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A long report to a channel that takes no files goes as far as it fits and says what did not — it is not cut to its
    /// first lines with a promise of a file that will never come.
    #[tokio::test]
    async fn a_long_report_to_a_channel_without_files_says_what_did_not_fit() {
        let (app, w, dir) = world().await;
        hook(&app, &w, "ch_sl", "slack");
        reporting(&app, "ch_sl", "s1");
        app.hub.set_projects(snap("s1", "Исследование", "working", 0));
        let long = format!("# Большой отчёт\n\n{}", (0..300).map(|i| format!("Абзац номер {i} с достаточно длинным текстом, чтобы набрать объём.")).collect::<Vec<_>>().join("\n\n"));
        boxed(&w, "s1", "r-1", &long, &[("data.csv", b"a,b\n")]);
        sweep(&app).await;
        let sent = w.at("/slack/hook");
        assert_eq!(sent.len(), 3, "as many messages as allowed, not one");
        let all: String = sent.iter().map(|m| m["text"].as_str().unwrap().to_string()).collect::<Vec<_>>().join("\n");
        assert!(all.contains("знаков не поместились: вебхук Slack не принимает файлы") && all.contains("файлы не отправлены (1)") && !all.contains("в приложении"), "{}", &all[all.len().saturating_sub(400)..]);
        assert!(answer(&w, "s1", "r-1").unwrap().starts_with("sent: "));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The note about the box goes with the owner's next message — once, whatever the shape of the message — and only to
    /// a conversation whose binding allows reports. A command is not a message; after `/clear` the note is due again.
    #[tokio::test]
    async fn the_note_about_the_box_goes_once_with_the_next_message() {
        let (app, w, dir) = world().await;
        telegram(&app, &w, "ch_tg", false); hook(&app, &w, "ch_dc", "discord");
        let b = reporting(&app, "ch_tg", "s1");
        reporting(&app, "ch_dc", "s1");
        bound(&app, Binding { channel: "ch_tg".into(), scope: "session".into(), project: "ws".into(), session: "s2".into(), when: "always".into(), content: "fact".into(), ..Default::default() });
        let plain = serde_json::to_vec(&json!({ "content": "Исследуй рынок" })).unwrap();
        assert!(with_intro(&app, "projects/ws/sessions/s2/prompt", &plain).is_none(), "no reports — no note");
        assert!(with_intro(&app, "projects/ws/sessions/s1/model", &plain).is_none() && with_intro(&app, "projects/other/sessions/s1/prompt", &plain).is_none());
        // a command is not a message for the agent
        for cmd in ["/model fable", "/compact", " /cost "] { assert!(with_intro(&app, "projects/ws/sessions/s1/prompt", &serde_json::to_vec(&json!({ "content": cmd })).unwrap()).is_none(), "{cmd}"); }
        let (body, id) = with_intro(&app, "projects/ws/sessions/s1/prompt", &plain).unwrap();
        assert_eq!(id, b.id);
        let text = serde_json::from_slice::<Value>(&body).unwrap()["content"].as_str().unwrap().to_string();
        assert!(text.starts_with(&format!("Исследуй рынок\n\n---\n{INTRO_HEAD}\n")) && text.contains("sh ~/.cloudos/bin/cloudos-report s1 <отчёт.md> [файлы…]"), "{text}");
        assert!(!text.contains("Личный") && !text.contains("telegram") && !text.contains("discord"), "the agent is not told where its reports go");
        // blocks with an image: the note joins the text block; an image alone gets a text block of its own
        let blocks = serde_json::to_vec(&json!({ "content": [{ "type": "image", "source": {} }, { "type": "text", "text": "что на картинке?" }] })).unwrap();
        let v: Value = serde_json::from_slice(&with_intro(&app, "projects/ws/sessions/s1/queue", &blocks).unwrap().0).unwrap();
        assert_eq!(v["content"][0]["type"], "image"); assert!(v["content"][1]["text"].as_str().unwrap().starts_with(&format!("что на картинке?\n\n---\n{INTRO_HEAD}")));
        let image = serde_json::to_vec(&json!({ "content": [{ "type": "image", "source": {} }] })).unwrap();
        let v: Value = serde_json::from_slice(&with_intro(&app, "projects/ws/sessions/s1/prompt", &image).unwrap().0).unwrap();
        assert_eq!(v["content"].as_array().unwrap().len(), 2); assert!(v["content"][1]["text"].as_str().unwrap().starts_with(INTRO_HEAD));
        // a body of another shape goes on untouched
        assert!(with_intro(&app, "projects/ws/sessions/s1/prompt", b"not json").is_none());
        // the core took the message: both channels of the conversation are served by the one note, and the box is open
        intro_delivered(&app, &id).await;
        assert!(channels::bindings(&app).iter().filter(|b| b.session == "s1").all(|b| b.intro_sent));
        assert!(with_intro(&app, "projects/ws/sessions/s1/prompt", &plain).is_none());
        assert!(w.files.lock().unwrap().contains_key(SCRIPT) && open(&w, "s1"), "the script and the open box are in the workspace by then");
        // a rewritten message carries the note again: the conversation is taken back, the note may be gone with it
        let edited = serde_json::to_vec(&json!({ "seq": 4, "content": [{ "type": "text", "text": "Исследуй рынок иначе" }] })).unwrap();
        let v: Value = serde_json::from_slice(&with_intro(&app, "projects/ws/sessions/s1/edit", &edited).unwrap().0).unwrap();
        assert_eq!(v["seq"], 4); assert!(v["content"][0]["text"].as_str().unwrap().starts_with(&format!("Исследуй рынок иначе\n\n---\n{INTRO_HEAD}")));
        assert!(with_intro(&app, "projects/ws/sessions/s2/edit", &edited).is_none());
        // the agent's memory is cleared: the note is due again with the next real message
        assert!(with_intro(&app, "projects/ws/sessions/s1/prompt", &serde_json::to_vec(&json!({ "content": "/clear" })).unwrap()).is_none());
        assert!(channels::bindings(&app).iter().filter(|b| b.session == "s1").all(|b| !b.intro_sent));
        assert!(with_intro(&app, "projects/ws/sessions/s1/prompt", &plain).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
