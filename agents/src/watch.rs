//! Workspace memory and long silent commands in "Inbox" (ADR-009, docs/agents-ux.md §4.13). The core's pressure watch
//! does the work (states, ending a process, the feed event); here its last action becomes a card, a conversation whose
//! turn has been silent for hours becomes another one, and both get a push. A card can be dismissed: the key is kept in
//! the service kv.
use crate::store::Store;
use crate::{ApiError, App};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

const KV_DISMISSED: &str = "inbox.dismissed";
/// "A process was ended" stays in "Inbox" for a day
const DONE_TTL: u64 = 24 * 3600;
/// "Stands at the limit" stays while it does and an hour after
const STALLED_TTL: u64 = 3600;
/// Dismissed keys are forgotten after a week (their cards are long gone)
const DISMISSED_TTL: u64 = 7 * 24 * 3600;
/// Default threshold of a long command when the core does not say
const LONG_HOURS: f64 = 2.0;

pub fn now_secs() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

pub fn dismissed(store: &Store) -> HashMap<String, u64> {
    store.get_kv(KV_DISMISSED).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

pub fn dismiss_key(store: &Store, key: &str, now: u64) -> anyhow::Result<()> {
    let mut m = dismissed(store);
    m.retain(|_, at| now.saturating_sub(*at) < DISMISSED_TTL);
    m.insert(key.to_string(), now);
    store.put_kv(KV_DISMISSED, &json!(m))
}

/// The "long command" threshold of a project in seconds, 0 — off: the core's stand-wide value or the project override
pub fn long_secs(cfg: &Value, project: &str) -> u64 {
    let h = cfg.get("projects").and_then(|p| p.get(project)).and_then(|o| o.get("long_turn_hours")).and_then(|v| v.as_f64())
        .or_else(|| cfg.get("long_turn_hours").and_then(|v| v.as_f64()))
        .unwrap_or(LONG_HOURS);
    if cfg.get("enabled").and_then(|e| e.as_bool()) == Some(false) || h <= 0.0 { 0 } else { (h * 3600.0).round().max(1.0) as u64 }
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str { v.get(key).and_then(|x| x.as_str()).unwrap_or("") }
fn n(v: &Value, key: &str) -> u64 { v.get(key).and_then(|x| x.as_u64()).unwrap_or(0) }

fn session_title(p: &Value, id: &str) -> Option<String> {
    p.get("sessions")?.as_array()?.iter().find(|x| s(x, "id") == id).map(|x| s(x, "title").to_string()).filter(|t| !t.is_empty())
}

/// Cards from the watch's last action per workspace: a process ended by the watch or by the system (a day), the workspace
/// standing at its limit (while it does and an hour after)
pub fn memory_cards(projects: &[Value], dismissed: &HashMap<String, u64>, now: u64) -> Vec<Value> {
    let mut out = Vec::new();
    for p in projects {
        let name = s(p, "name");
        let Some(m) = p.get("memory").filter(|m| m.is_object()) else { continue };
        let Some(last) = m.get("last").filter(|l| l.is_object()) else { continue };
        let (kind, at, state) = (s(last, "kind"), n(last, "at"), s(m, "state"));
        let live = kind == "stalled" && state == "stalled";
        let age = now.saturating_sub(at);
        let show = match kind { "killed" | "oom" => age < DONE_TTL, "stalled" => live || age < STALLED_TTL, _ => false };
        let key = format!("mem:{name}:{at}");
        if !show || dismissed.contains_key(&key) { continue }
        let session = last.get("session").and_then(|x| x.as_str()).filter(|x| !x.is_empty());
        out.push(json!({
            "key": key, "project": name, "kind": kind, "at": at, "cmd": s(last, "cmd"), "rss": n(last, "rss"), "age_secs": n(last, "age_secs"),
            "stalled_secs": n(last, "stalled_secs"), "full60": last.get("full60").cloned().unwrap_or(json!(0)), "limit": n(last, "limit"), "note": last.get("note").cloned().unwrap_or(Value::Null),
            "session": session, "session_title": session.and_then(|id| session_title(p, id)),
            "live": live,
            // the owner's "end the process" button: the workspace is still not calm and there is somebody to end
            "can_kill": kind == "stalled" && state != "ok" && m.get("top").map(|t| t.is_object()).unwrap_or(false),
        }));
    }
    out.sort_by(|a, b| n(b, "at").cmp(&n(a, "at")));
    out
}

/// Conversations whose turn has produced no journal event for longer than the project's threshold
pub fn long_candidates(projects: &[Value], dismissed: &HashMap<String, u64>, now: u64) -> Vec<Value> {
    let mut out = Vec::new();
    for p in projects {
        let name = s(p, "name");
        let limit = n(p, "long_turn_secs");
        if limit == 0 { continue }
        for sess in p.get("sessions").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
            if s(&sess, "status") != "working" { continue }
            let since = n(&sess, "updated_at");
            if since == 0 || now.saturating_sub(since) < limit { continue }
            let key = format!("long:{name}:{}:{since}", s(&sess, "id"));
            if dismissed.contains_key(&key) { continue }
            out.push(json!({ "key": key, "project": name, "session": sess, "since": since }));
        }
    }
    out.sort_by(|a, b| n(a, "since").cmp(&n(b, "since")));
    out
}

/// The tool call still waiting for its result at the tail of a journal: "Bash: node gen-maps.mjs"
pub fn running_tool(events: &[Value]) -> Option<String> {
    let mut open: Vec<(String, String)> = Vec::new();
    for e in events {
        for b in e.get("content").and_then(|c| c.as_array()).cloned().unwrap_or_default() {
            match s(&b, "type") {
                "tool_use" => open.push((s(&b, "id").to_string(), crate::push::request_line(s(&b, "name"), b.get("input").unwrap_or(&Value::Null)))),
                "tool_result" => { let id = s(&b, "tool_use_id").to_string(); open.retain(|(i, _)| *i != id); }
                _ => {}
            }
        }
    }
    open.pop().map(|(_, line)| line)
}

/// Long-command cards with the running command looked up at the core (once per silent stretch — cached in the hub)
pub async fn long_cards(app: &App, projects: &[Value], dismissed: &HashMap<String, u64>, now: u64) -> Vec<Value> {
    let mut out = Vec::new();
    for mut c in long_candidates(projects, dismissed, now) {
        let key = s(&c, "key").to_string();
        let tool = match app.hub.long_tool_cached(&key) {
            Some(t) => t,
            None => {
                let (project, id, last_seq) = (s(&c, "project").to_string(), s(&c["session"], "id").to_string(), n(&c["session"], "last_seq"));
                let t = app.core_get(&format!("/projects/{project}/sessions/{id}/events?since={}&lite=1", last_seq.saturating_sub(60))).await.ok()
                    .and_then(|v| v.as_array().map(|a| running_tool(a))).flatten();
                app.hub.long_tool_store(&key, t.clone());
                t
            }
        };
        c["tool"] = json!(tool);
        out.push(c);
    }
    out
}

fn gib(b: u64) -> String {
    let g = b as f64 / 1_073_741_824.0;
    if g >= 1.0 { format!("{g:.1} ГиБ") } else { format!("{:.0} МиБ", b as f64 / 1_048_576.0) }
}
fn dur(secs: u64) -> String {
    if secs >= 7200 { format!("{} ч", secs / 3600) } else if secs >= 120 { format!("{} мин", secs / 60) } else { format!("{secs} с") }
}

/// Push payloads for the cards that were not in the previous "Inbox"
pub fn push_payloads(inbox: &Value, prev: &std::collections::HashSet<String>, now: u64) -> Vec<Value> {
    let mut out = Vec::new();
    for c in inbox.get("memory").and_then(|m| m.as_array()).cloned().unwrap_or_default() {
        if prev.contains(s(&c, "key")) { continue }
        let (project, cmd) = (s(&c, "project"), s(&c, "cmd"));
        let (title, body) = match s(&c, "kind") {
            "killed" => (format!("{project}: завершён процесс — workspace стоял у лимита памяти"), format!("{cmd} — {}, работал {}", gib(n(&c, "rss")), dur(n(&c, "age_secs")))),
            "oom" => (format!("{project}: процесс завершён системой — не хватило памяти"), if cmd.is_empty() { format!("лимит workspace {}", gib(n(&c, "limit"))) } else { format!("{cmd} — {}", gib(n(&c, "rss"))) }),
            _ => (format!("{project}: workspace стоит у лимита памяти"), if cmd.is_empty() { format!("лимит {}", gib(n(&c, "limit"))) } else { format!("крупнейший процесс: {cmd} — {}", gib(n(&c, "rss"))) }),
        };
        let url = match c.get("session").and_then(|x| x.as_str()) { Some(id) => format!("/agents/?open={project}/{id}"), None => "/agents/?section=inbox".to_string() };
        out.push(json!({ "kind": "memory", "title": title, "body": body, "project": project, "id": c.get("session").cloned().unwrap_or(Value::Null), "tag": format!("mem-{project}"), "renotify": true, "url": url }));
    }
    for c in inbox.get("long").and_then(|m| m.as_array()).cloned().unwrap_or_default() {
        if prev.contains(s(&c, "key")) { continue }
        let (project, sess) = (s(&c, "project"), &c["session"]);
        let (id, title) = (s(sess, "id"), if s(sess, "title").is_empty() { "беседа" } else { s(sess, "title") });
        let body = match c.get("tool").and_then(|t| t.as_str()) { Some(t) => t.to_string(), None => format!("{project}: ход без событий") };
        out.push(json!({ "kind": "long", "title": format!("{title}: команда идёт {}", dur(now.saturating_sub(n(&c, "since")))), "body": body, "project": project, "id": id, "tag": format!("long-{id}"), "url": format!("/agents/?open={project}/{id}"),
            "actions": [{ "action": "open", "title": "Открыть" }, { "action": "interrupt", "title": "Прервать ход" }] }));
    }
    out
}

pub fn keys(inbox: &Value) -> std::collections::HashSet<String> {
    ["memory", "long"].iter().flat_map(|k| inbox.get(*k).and_then(|m| m.as_array()).cloned().unwrap_or_default()).map(|c| s(&c, "key").to_string()).collect()
}

#[derive(Deserialize)]
pub struct DismissBody { pub key: String }

/// `POST /agents/api/inbox/dismiss` — hide a memory card or stop reminding about a long command (until the turn moves on)
pub async fn dismiss(State(app): State<Arc<App>>, Json(b): Json<DismissBody>) -> Result<Json<Value>, ApiError> {
    if !(b.key.starts_with("mem:") || b.key.starts_with("long:")) || b.key.len() > 300 { return Err(ApiError(StatusCode::BAD_REQUEST, "неизвестная карточка".into())) }
    dismiss_key(&app.store, &b.key, now_secs())?;
    crate::hub::refresh_derived(app.clone());
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Store, std::path::PathBuf) { let dir = std::env::temp_dir().join(format!("agents-watch-{}-{}", std::process::id(), uuid::Uuid::new_v4())); (Store::open(&dir).unwrap(), dir) }

    fn project(name: &str, state: &str, last: Value, sessions: Value) -> Value {
        json!({ "name": name, "long_turn_secs": 7200, "sessions": sessions,
            "memory": { "used": 4_000_000_000u64, "limit": 4_294_967_296u64, "state": state, "action": "kill", "top": { "cmd": "node hog.js", "rss": 2_000_000_000u64 }, "last": last } })
    }

    #[test]
    fn memory_cards_follow_the_last_action() {
        let now = 1_800_000_000u64;
        let killed = json!({ "at": now - 60, "kind": "killed", "cmd": "node rust/tools/gen-maps.mjs", "rss": 2_791_728_742u64, "age_secs": 172_800, "stalled_secs": 120, "full60": 62.0, "limit": 4_294_967_296u64, "session": "s1" });
        let sessions = json!([{ "id": "s1", "title": "Рейсы из Вашингтона", "status": "working" }]);
        let ps = vec![project("alpha", "ok", killed.clone(), sessions.clone())];
        let cards = memory_cards(&ps, &HashMap::new(), now);
        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!((s(c, "key"), s(c, "kind"), s(c, "cmd"), s(c, "session"), s(c, "session_title")), (format!("mem:alpha:{}", now - 60).as_str(), "killed", "node rust/tools/gen-maps.mjs", "s1", "Рейсы из Вашингтона"));
        assert_eq!((c["live"].as_bool(), c["can_kill"].as_bool()), (Some(false), Some(false)));
        // a day later the card is gone; a dismissed one is gone at once
        assert!(memory_cards(&ps, &HashMap::new(), now + DONE_TTL).is_empty());
        let mut d = HashMap::new(); d.insert(format!("mem:alpha:{}", now - 60), now);
        assert!(memory_cards(&ps, &d, now).is_empty());

        // "stands at the limit": alive while the workspace is stalled (with the button), an hour after it calmed down, then gone
        let stalled = json!({ "at": now - 5000, "kind": "stalled", "cmd": "node hog.js", "rss": 2_000_000_000u64, "note": "notify", "limit": 4_294_967_296u64 });
        let live = memory_cards(&[project("beta", "stalled", stalled.clone(), json!([]))], &HashMap::new(), now);
        assert_eq!((live.len(), live[0]["live"].as_bool(), live[0]["can_kill"].as_bool(), s(&live[0], "note")), (1, Some(true), Some(true), "notify"));
        assert!(memory_cards(&[project("beta", "ok", stalled, json!([]))], &HashMap::new(), now).is_empty(), "успокоился больше часа назад");
        let recent = json!({ "at": now - 600, "kind": "stalled", "cmd": "node hog.js" });
        let calm = memory_cards(&[project("beta", "ok", recent, json!([]))], &HashMap::new(), now);
        assert_eq!((calm.len(), calm[0]["can_kill"].as_bool()), (1, Some(false)), "после пробуксовки карточка ещё час, но завершать уже нечего");
        // no watch data (a stopped workspace, an owner's server) — no cards
        assert!(memory_cards(&[json!({ "name": "host1", "memory": null }), json!({ "name": "x" })], &HashMap::new(), now).is_empty());
    }

    #[test]
    fn long_commands_and_thresholds() {
        let now = 1_800_000_000u64;
        let cfg = json!({ "enabled": true, "long_turn_hours": 2.0, "projects": { "e2e": { "long_turn_hours": 0.01 }, "quiet": { "long_turn_hours": 0 } } });
        assert_eq!((long_secs(&cfg, "alpha"), long_secs(&cfg, "e2e"), long_secs(&cfg, "quiet")), (7200, 36, 0));
        assert_eq!(long_secs(&json!({}), "alpha"), 7200, "ядро не ответило — порог по умолчанию");
        assert_eq!(long_secs(&json!({ "enabled": false, "long_turn_hours": 2.0 }), "alpha"), 0);

        let sessions = json!([
            { "id": "s1", "title": "Рейсы", "status": "working", "updated_at": now - 7500, "last_seq": 300 },
            { "id": "s2", "title": "Свежая", "status": "working", "updated_at": now - 600 },
            { "id": "s3", "title": "Ждёт", "status": "idle", "updated_at": now - 90_000 },
            { "id": "s4", "title": "Вопрос", "status": "waiting", "updated_at": now - 90_000 },
        ]);
        let ps = vec![json!({ "name": "alpha", "long_turn_secs": 7200, "sessions": sessions.clone() }), json!({ "name": "quiet", "long_turn_secs": 0, "sessions": sessions })];
        let c = long_candidates(&ps, &HashMap::new(), now);
        assert_eq!(c.len(), 1, "только ход без событий дольше порога; ожидание владельца и готовые беседы — не «долгая команда»");
        assert_eq!((s(&c[0], "key"), n(&c[0], "since")), (format!("long:alpha:s1:{}", now - 7500).as_str(), now - 7500));
        // "do not remind" holds until the turn moves on: a new event changes updated_at, hence the key
        let mut d = HashMap::new(); d.insert(s(&c[0], "key").to_string(), now);
        assert!(long_candidates(&ps, &d, now).is_empty());
        let moved = vec![json!({ "name": "alpha", "long_turn_secs": 7200, "sessions": [{ "id": "s1", "status": "working", "updated_at": now - 7300 }] })];
        assert_eq!(long_candidates(&moved, &d, now).len(), 1);
    }

    #[test]
    fn running_tool_is_the_last_call_without_a_result() {
        let ev = vec![
            json!({ "kind": "assistant", "content": [{ "type": "text", "text": "запускаю" }, { "type": "tool_use", "id": "t1", "name": "Read", "input": { "file_path": "/a" } }] }),
            json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "ok" }] }),
            json!({ "kind": "assistant", "content": [{ "type": "tool_use", "id": "t2", "name": "Bash", "input": { "command": "node rust/tools/gen-maps.mjs" } }] }),
        ];
        assert_eq!(running_tool(&ev).as_deref(), Some("Bash: node rust/tools/gen-maps.mjs"));
        assert_eq!(running_tool(&ev[..2]), None);
        assert_eq!(running_tool(&[]), None);
    }

    #[test]
    fn push_only_for_new_cards() {
        let now = 1_800_000_000u64;
        let inbox = json!({
            "memory": [{ "key": "mem:alpha:1", "project": "alpha", "kind": "killed", "cmd": "node gen-maps.mjs", "rss": 2_791_728_742u64, "age_secs": 172_800, "session": "s1" },
                       { "key": "mem:beta:2", "project": "beta", "kind": "stalled", "cmd": "python3 hog.py", "rss": 900_000_000u64, "limit": 4_294_967_296u64, "session": null },
                       { "key": "mem:gamma:3", "project": "gamma", "kind": "oom", "cmd": "", "limit": 12_884_901_888u64, "session": null }],
            "long": [{ "key": "long:alpha:s1:5", "project": "alpha", "since": now - 7500, "tool": "Bash: node gen-maps.mjs", "session": { "id": "s1", "title": "Рейсы" } }],
        });
        assert_eq!(keys(&inbox).len(), 4);
        let all = push_payloads(&inbox, &Default::default(), now);
        assert_eq!(all.len(), 4);
        assert_eq!((s(&all[0], "kind"), s(&all[0], "url"), s(&all[0], "body")), ("memory", "/agents/?open=alpha/s1", "node gen-maps.mjs — 2.6 ГиБ, работал 48 ч"));
        assert!(s(&all[0], "title").contains("alpha") && s(&all[0], "title").contains("завершён процесс"));
        assert_eq!((s(&all[1], "url"), s(&all[1], "body")), ("/agents/?section=inbox", "крупнейший процесс: python3 hog.py — 858 МиБ"));
        assert!(s(&all[2], "title").contains("завершён системой") && s(&all[2], "body").contains("12.0 ГиБ"));
        assert_eq!((s(&all[3], "kind"), s(&all[3], "title"), s(&all[3], "body"), s(&all[3]["actions"][1], "action")), ("long", "Рейсы: команда идёт 2 ч", "Bash: node gen-maps.mjs", "interrupt"));
        // what was already in the previous "Inbox" is not sent again
        let prev = keys(&inbox);
        assert!(push_payloads(&inbox, &prev, now).is_empty());
    }

    #[test]
    fn dismissed_keys_persist_and_expire() {
        let (st, dir) = store();
        assert!(dismissed(&st).is_empty());
        dismiss_key(&st, "mem:alpha:1", 1000).unwrap();
        dismiss_key(&st, "long:alpha:s1:5", 1000 + DISMISSED_TTL - 1).unwrap();
        assert_eq!(dismissed(&st).len(), 2);
        dismiss_key(&st, "mem:beta:2", 1000 + DISMISSED_TTL + 5).unwrap();
        let d = dismissed(&st);
        assert!(!d.contains_key("mem:alpha:1") && d.contains_key("long:alpha:s1:5") && d.contains_key("mem:beta:2"), "старше недели — забыт");
        let _ = std::fs::remove_dir_all(dir);
    }
}
