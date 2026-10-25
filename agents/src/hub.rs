//! Event hub (ADR-007): one subscriber per running workspace (the core's `/api/projects/{p}/events` → agentd's `/v1/events`),
//! shared by all tabs; the tree snapshot in memory; the base part (project list, today's usage) — from the core every 30 s. For
//! clients — `GET /agents/api/hub`: a full snapshot on connect, then per-project deltas and a full snapshot every 30 s.
use crate::App;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message as TMsg;

pub struct Hub {
    /// Last snapshot: projects (as in `/workspaces`) and today's usage
    projects: Mutex<Vec<Value>>,
    today: Mutex<Value>,
    /// Live workspace subscriptions: name → task
    subs: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    tx: broadcast::Sender<Value>,
    started: std::sync::atomic::AtomicBool,
    /// Derived from the snapshot (slice 60): "Inbox" and the scratch "Chat" status — computed after every snapshot
    /// change with a debounce and sent to clients only on change; a new client gets them right after the snapshot
    inbox: Mutex<Value>,
    chat: Mutex<Value>,
    derive: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Bodies of pending requests by "project:session" with a fingerprint (pending, updated_at, last_seq) — so we do not go to the
    /// core on every delta
    pending: Mutex<HashMap<String, ((u64, u64, u64), Value)>>,
}

impl Default for Hub {
    fn default() -> Self { Self { projects: Mutex::new(Vec::new()), today: Mutex::new(json!({ "cost_usd": 0.0, "turns": 0 })), subs: Mutex::new(HashMap::new()), tx: broadcast::channel(256).0, started: Default::default(), inbox: Mutex::new(Value::Null), chat: Mutex::new(Value::Null), derive: Mutex::new(None), pending: Mutex::new(HashMap::new()) } }
}

impl Hub {
    /// Engine catalog versions per project (engines_v from agentd snapshots) — for REST
    /// /workspaces
    pub fn engines_versions(&self) -> std::collections::HashMap<String, Value> {
        self.projects.lock().unwrap().iter().filter_map(|p| Some((p.get("name")?.as_str()?.to_string(), p.get("engines_v").cloned()?))).collect()
    }

    pub fn snapshot(&self) -> Value { json!({ "kind": "workspaces", "projects": *self.projects.lock().unwrap(), "today": *self.today.lock().unwrap() }) }
    /// How many workspaces are subscribed right now (for "About" and
    /// e2e)
    pub fn subscribed(&self) -> usize { self.subs.lock().unwrap().len() }
    /// Simple chats changed (created, renamed, a new message, deleted): clients with "Chat" open re-read the list
    pub fn chats_changed(&self) { let _ = self.tx.send(json!({ "kind": "chats" })); }
    /// Spending limits changed (slice 67): a frame with the new list — the composer bar is computed from it and
    /// the snapshot
    pub fn limits_changed(&self, limits: Value) { let _ = self.tx.send(json!({ "kind": "limits", "limits": limits })); }
    /// Derived values for e2e and "About"
    pub fn derived(&self) -> (Value, Value) { (self.inbox.lock().unwrap().clone(), self.chat.lock().unwrap().clone()) }
    pub fn pending_cached(&self, key: &str, stamp: (u64, u64, u64)) -> Option<Value> { self.pending.lock().unwrap().get(key).filter(|(st, _)| *st == stamp).map(|(_, v)| v.clone()) }
    pub fn pending_store(&self, key: &str, stamp: (u64, u64, u64), v: Value) {
        let mut m = self.pending.lock().unwrap();
        if m.len() > 256 { m.clear(); }
        m.insert(key.to_string(), (stamp, v));
    }
}

/// Recompute "Inbox" and the "Chat" status from the snapshot — 300 ms debounce, to clients only on
/// change
fn schedule_derived(app: Arc<App>) {
    let mut g = app.hub.derive.lock().unwrap();
    if let Some(h) = g.take() { h.abort(); }
    let app2 = app.clone();
    *g = Some(tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let projects = app2.hub.projects.lock().unwrap().clone();
        let today = app2.hub.today.lock().unwrap().clone();
        let inbox = crate::api::inbox_from(&app2, &projects, &today).await;
        // push notifications (§9.9): new pending requests and status transitions — by the
        // snapshot difference
        crate::push::on_snapshot(&app2, &projects, &today, &inbox).await;
        let changed = { let mut cur = app2.hub.inbox.lock().unwrap(); if *cur != inbox { *cur = inbox.clone(); true } else { false } };
        if changed { let mut v = inbox; v["kind"] = json!("inbox"); let _ = app2.hub.tx.send(v); }
        let chat = crate::chat::status_from(&app2, &projects);
        let changed = { let mut cur = app2.hub.chat.lock().unwrap(); if *cur != chat { *cur = chat.clone(); true } else { false } };
        if changed { let mut v = chat; v["kind"] = json!("chat"); let _ = app2.hub.tx.send(v); }
    }));
}

/// Start of the hub background loop (once, at service start): a base snapshot every 30 s + agentd
/// subscriptions.
pub fn start(app: Arc<App>) {
    if app.hub.started.swap(true, std::sync::atomic::Ordering::SeqCst) { return; }
    tokio::spawn(async move {
        loop {
            match crate::api::collect(&app).await {
                Ok(c) => {
                    { *app.hub.today.lock().unwrap() = c.today.clone(); }
                    // the base project list; sessions/terminals of subscribed workspaces come from their latest
                    // snapshots
                    let mut merged = c.projects.clone();
                    {
                        let cur = app.hub.projects.lock().unwrap();
                        for p in merged.iter_mut() {
                            let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                            if let Some(old) = cur.iter().find(|o| o.get("name").and_then(|n| n.as_str()) == Some(name.as_str())) {
                                if old.get("hub").and_then(|h| h.as_bool()) == Some(true) {
                                    // a safeguard: if the fresh core list knows sessions or terminals missing from the latest
                                    // subscription snapshot (a missed frame), take the fresh one — otherwise the lag would live
                                    // until the next event
                                    let ids = |v: &Value| -> std::collections::BTreeSet<String> { v.as_array().map(|a| a.iter().filter_map(|s| s.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect()).unwrap_or_default() };
                                    let stale = ids(&old["sessions"]) != ids(&p["sessions"]) || ids(&old["ptys"]) != ids(&p["ptys"]);
                                    if stale { tracing::info!("хаб: снимок {name} отстал от ядра — обновлён базовым"); }
                                    else { p["sessions"] = old["sessions"].clone(); p["ptys"] = old["ptys"].clone(); p["reachable"] = old["reachable"].clone(); }
                                    p["hub"] = json!(true);
                                }
                            }
                        }
                    }
                    *app.hub.projects.lock().unwrap() = merged;
                    let _ = app.hub.tx.send(app.hub.snapshot());
                    schedule_derived(app.clone());
                    // subscriptions: add newly started ones, drop stopped ones
                    let running: Vec<String> = c.projects.iter().filter(|p| p.get("status").and_then(|s| s.as_str()) == Some("running")).filter_map(|p| p.get("name")?.as_str().map(str::to_string)).collect();
                    let mut subs = app.hub.subs.lock().unwrap();
                    subs.retain(|name, h| { let keep = running.contains(name) && !h.is_finished(); if !keep { h.abort(); } keep });
                    for name in running {
                        if !subs.contains_key(&name) {
                            let app2 = app.clone(); let n2 = name.clone();
                            subs.insert(name, tokio::spawn(async move { subscribe_loop(app2, n2).await }));
                        }
                    }
                }
                Err(e) => tracing::warn!("хаб: базовый снимок: {e:#}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

/// Subscription to one workspace: snapshots from agentd through the core → the project snapshot in memory → a delta to clients;
/// reconnect 3 → 30 s.
async fn subscribe_loop(app: Arc<App>, name: String) {
    let mut wait = 3u64;
    loop {
        let url = format!("{}/api/projects/{}/events", app.cloudd.replacen("http", "ws", 1), name);
        match tokio_tungstenite::connect_async(&url).await {
            Ok((mut ws, _)) => {
                wait = 3;
                while let Some(m) = ws.next().await {
                    let text = match m { Ok(TMsg::Text(t)) => t.to_string(), Ok(TMsg::Close(_)) | Err(_) => break, _ => continue };
                    let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                    if v.get("kind").and_then(|k| k.as_str()) != Some("snapshot") { continue; }
                    let sessions = v.get("sessions").cloned().unwrap_or(json!([])); let ptys = v.get("ptys").cloned().unwrap_or(json!([]));
                    let engines_v = v.get("engines_v").cloned().unwrap_or(json!(0));
                    {
                        let mut cur = app.hub.projects.lock().unwrap();
                        if let Some(p) = cur.iter_mut().find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name.as_str())) {
                            p["sessions"] = sessions.clone(); p["ptys"] = ptys.clone(); p["reachable"] = json!(true); p["hub"] = json!(true); p["engines_v"] = engines_v.clone();
                        }
                    }
                    let _ = app.hub.tx.send(json!({ "kind": "project", "name": name, "sessions": sessions, "ptys": ptys, "reachable": true, "engines_v": engines_v }));
                    schedule_derived(app.clone());
                }
                tracing::info!("хаб: подписка на {name} оборвалась — переподключение через {wait} с");
            }
            Err(e) => tracing::warn!("хаб: {name}: {e} — повтор через {wait} с"),
        }
        {
            let mut cur = app.hub.projects.lock().unwrap();
            if let Some(p) = cur.iter_mut().find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name.as_str())) { p["hub"] = json!(false); }
        }
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        wait = (wait * 2).min(30);
    }
}

/// `GET /agents/api/hub` — the client WebSocket: a full snapshot on connect, then hub events; ping every 25 s.
pub async fn ws(State(app): State<Arc<App>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |sock| client(app, sock))
}

async fn client(app: Arc<App>, mut sock: WebSocket) {
    let mut rx = app.hub.tx.subscribe();
    if sock.send(Message::Text(app.hub.snapshot().to_string().into())).await.is_err() { return; }
    // spending limits (slice 67) — right away, like the
    // derived values
    if let Ok(st) = crate::api::settings_of(&app) { let limits = st.get("limits").cloned().unwrap_or(json!([])); if sock.send(Message::Text(json!({ "kind": "limits", "limits": limits }).to_string().into())).await.is_err() { return; } }
    // derived values right away: the client has "Inbox" and the "Chat" status before
    // the first change
    let (inbox, chat) = app.hub.derived();
    for (mut v, kind) in [(inbox, "inbox"), (chat, "chat")] {
        if v.is_null() { continue }
        v["kind"] = json!(kind);
        if sock.send(Message::Text(v.to_string().into())).await.is_err() { return; }
    }
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                Ok(v) => { if sock.send(Message::Text(v.to_string().into())).await.is_err() { return; } }
                Err(broadcast::error::RecvError::Lagged(_)) => { if sock.send(Message::Text(app.hub.snapshot().to_string().into())).await.is_err() { return; } }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(25)) => { if sock.send(Message::Text("{\"kind\":\"ping\"}".into())).await.is_err() { return; } }
            // a client ping — answer with pong: after the network returns the browser checks this way whether
            // the socket is alive (§9.10)
            m = sock.recv() => match m { Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return, Some(Ok(Message::Text(t))) => { if t.contains("\"ping\"") && sock.send(Message::Text("{\"kind\":\"pong\"}".into())).await.is_err() { return; } } _ => {} },
        }
    }
}
