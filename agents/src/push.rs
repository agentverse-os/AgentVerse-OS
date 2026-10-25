//! Push notifications of the installed app (docs/agents-ux.md §9.9): VAPID keys and device subscriptions in the
//! service kv, sending through the browser push service (aes128gcm encryption and VAPID signing — the web-push crate,
//! HTTP — our reqwest), events — by transitions between hub snapshots: a new pending permission request (with the
//! "Allow/Deny" actions), end of a turn, an error, an exhausted spending limit. The private key and subscription keys
//! are not given to the browser; only the title and a short line go into the push — request and answer bodies stay
//! with us.
use crate::store::Store;
use crate::{ApiError, App};
use anyhow::{anyhow, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use web_push::{ContentEncoding, SubscriptionInfo, Urgency, VapidSignatureBuilder, WebPushMessageBuilder};

const KV_VAPID: &str = "push.vapid";
const KV_SUBS: &str = "push.subs";
const KV_PREFS: &str = "push.prefs";
const B64URL: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
/// How many consecutive failures we tolerate before removing the subscription
/// (404/410 remove it at once)
const MAX_FAILS: u32 = 8;
/// Message lifetime in the push service: a permission request waits for the
/// owner for hours
const TTL_SECS: u32 = 6 * 3600;

#[derive(Clone, Serialize, Deserialize)]
pub struct Vapid { pub public: String, pub private: String }

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Sub {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    #[serde(default)] pub ua: String,
    #[serde(default)] pub created_at: u64,
    #[serde(default)] pub last_ok: u64,
    #[serde(default)] pub fails: u32,
}

/// Which events to send (all by default)
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Prefs {
    #[serde(default = "yes")] pub permission: bool,
    #[serde(default = "yes")] pub done: bool,
    #[serde(default = "yes")] pub error: bool,
    #[serde(default = "yes")] pub limit: bool,
}
fn yes() -> bool { true }
impl Default for Prefs { fn default() -> Self { Self { permission: true, done: true, error: true, limit: true } } }

/// Memory of transitions between hub snapshots: what we have already seen, to
/// send only the new
#[derive(Default)]
pub struct PushState {
    seen: Mutex<Option<Seen>>,
    limits_blocked: Mutex<Option<HashSet<String>>>,
}
#[derive(Default, Clone)]
struct Seen { requests: HashSet<String>, status: HashMap<String, String> }

fn now_secs() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

/// VAPID keys: from kv or a new P-256 pair (once per installation)
pub fn vapid(store: &Store) -> Result<Vapid> {
    if let Some(v) = store.get_kv(KV_VAPID)?.and_then(|v| serde_json::from_value::<Vapid>(v).ok()) { if !v.public.is_empty() && !v.private.is_empty() { return Ok(v) } }
    let sk = p256::SecretKey::random(&mut rand_core::OsRng);
    let v = Vapid { public: B64URL.encode(sk.public_key().to_encoded_point(false).as_bytes()), private: B64URL.encode(sk.to_bytes()) };
    store.put_kv(KV_VAPID, &serde_json::to_value(&v)?)?;
    Ok(v)
}
pub fn subs(store: &Store) -> Vec<Sub> { store.get_kv(KV_SUBS).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default() }
fn save_subs(store: &Store, list: &[Sub]) -> Result<()> { store.put_kv(KV_SUBS, &serde_json::to_value(list)?) }
pub fn prefs(store: &Store) -> Prefs { store.get_kv(KV_PREFS).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default() }

/// Subscription for the API answer: without encryption keys, the address — only the push
/// service host and the tail
fn public_sub(s: &Sub) -> Value {
    let host = s.endpoint.split('/').nth(2).unwrap_or("").to_string();
    let tail: String = s.endpoint.chars().rev().take(10).collect::<Vec<_>>().into_iter().rev().collect();
    json!({ "endpoint": s.endpoint, "host": host, "tail": tail, "ua": s.ua, "created_at": s.created_at, "last_ok": s.last_ok, "fails": s.fails })
}

pub fn public(store: &Store) -> Result<Value> {
    let v = vapid(store)?;
    let list = subs(store);
    Ok(json!({ "public_key": v.public, "subscriptions": list.iter().map(public_sub).collect::<Vec<_>>(), "count": list.len(), "prefs": prefs(store) }))
}

pub async fn get(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> { Ok(Json(public(&app.store)?)) }

#[derive(Deserialize)]
pub struct SubscribeBody { pub endpoint: String, pub keys: SubKeys, #[serde(default)] pub ua: String }
#[derive(Deserialize)]
pub struct SubKeys { pub p256dh: String, pub auth: String }

pub fn upsert(store: &Store, b: SubscribeBody) -> Result<usize> {
    if !b.endpoint.starts_with("https://") && !b.endpoint.starts_with("http://127.0.0.1") { anyhow::bail!("endpoint подписки должен быть https") }
    if b.keys.p256dh.trim().is_empty() || b.keys.auth.trim().is_empty() { anyhow::bail!("у подписки нет ключей") }
    let mut list = subs(store);
    list.retain(|s| s.endpoint != b.endpoint);
    list.push(Sub { endpoint: b.endpoint, p256dh: b.keys.p256dh.trim().to_string(), auth: b.keys.auth.trim().to_string(), ua: b.ua.chars().take(120).collect(), created_at: now_secs(), last_ok: 0, fails: 0 });
    if list.len() > 20 { list.remove(0); }
    save_subs(store, &list)?;
    Ok(list.len())
}
pub fn remove(store: &Store, endpoint: &str) -> Result<usize> {
    let mut list = subs(store); list.retain(|s| s.endpoint != endpoint); save_subs(store, &list)?; Ok(list.len())
}

pub async fn subscribe(State(app): State<Arc<App>>, Json(b): Json<SubscribeBody>) -> Result<Json<Value>, ApiError> {
    let n = upsert(&app.store, b).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "ok": true, "count": n })))
}
#[derive(Deserialize)]
pub struct EndpointBody { pub endpoint: String }
pub async fn unsubscribe(State(app): State<Arc<App>>, Json(b): Json<EndpointBody>) -> Result<Json<Value>, ApiError> {
    let n = remove(&app.store, &b.endpoint)?;
    Ok(Json(json!({ "ok": true, "count": n })))
}
#[derive(Deserialize)]
pub struct PrefsPatch { pub permission: Option<bool>, pub done: Option<bool>, pub error: Option<bool>, pub limit: Option<bool> }
pub async fn put_prefs(State(app): State<Arc<App>>, Json(b): Json<PrefsPatch>) -> Result<Json<Value>, ApiError> {
    let mut p = prefs(&app.store);
    if let Some(v) = b.permission { p.permission = v; } if let Some(v) = b.done { p.done = v; } if let Some(v) = b.error { p.error = v; } if let Some(v) = b.limit { p.limit = v; }
    app.store.put_kv(KV_PREFS, &serde_json::to_value(&p)?)?;
    Ok(Json(serde_json::to_value(&p)?))
}
#[derive(Deserialize, Default)]
pub struct TestBody { pub endpoint: Option<String> }
/// Test notification: to one device or to all
pub async fn test(State(app): State<Arc<App>>, Json(b): Json<TestBody>) -> Result<Json<Value>, ApiError> {
    let payload = json!({ "kind": "test", "title": "AgentVerse Agents", "body": "push работает — так придут запросы разрешений и ответы агентов", "tag": "test", "url": "/agents/?section=settings&tab=notify", "force": true });
    let r = send(&app.http, &app.store, &payload, b.endpoint.as_deref()).await;
    Ok(Json(json!({ "sent": r.sent, "failed": r.errors.len(), "errors": r.errors })))
}

pub struct SendResult { pub sent: usize, pub errors: Vec<String> }

/// Send the payload to all subscriptions (or one); 404/410 remove the subscription, a series of failures
/// too
pub async fn send(http: &reqwest::Client, store: &Store, payload: &Value, only: Option<&str>) -> SendResult {
    let mut out = SendResult { sent: 0, errors: vec![] };
    let Ok(v) = vapid(store) else { out.errors.push("ключи VAPID недоступны".into()); return out };
    let mut list = subs(store);
    if list.is_empty() { return out }
    let body = payload.to_string();
    let urgency = if payload.get("kind").and_then(|k| k.as_str()) == Some("permission") { Urgency::High } else { Urgency::Normal };
    let mut drop: Vec<String> = vec![];
    for s in list.iter_mut() {
        if let Some(e) = only { if s.endpoint != e { continue } }
        match send_one(http, &v, s, body.as_bytes(), urgency.clone()).await {
            Ok(()) => { out.sent += 1; s.last_ok = now_secs(); s.fails = 0; }
            Err(e) => {
                let m = e.to_string();
                out.errors.push(format!("{}: {m}", s.endpoint.split('/').nth(2).unwrap_or("")));
                if m.contains("410") || m.contains("404") { drop.push(s.endpoint.clone()); } else { s.fails += 1; if s.fails >= MAX_FAILS { drop.push(s.endpoint.clone()); } }
            }
        }
    }
    if !drop.is_empty() { list.retain(|s| !drop.contains(&s.endpoint)); }
    let _ = save_subs(store, &list);
    out
}

async fn send_one(http: &reqwest::Client, v: &Vapid, s: &Sub, body: &[u8], urgency: Urgency) -> Result<()> {
    let info = SubscriptionInfo::new(s.endpoint.clone(), s.p256dh.clone(), s.auth.clone());
    let mut sig = VapidSignatureBuilder::from_base64(&v.private, &info).map_err(|e| anyhow!("vapid: {e}"))?;
    sig.add_claim("sub", "mailto:owner@agentverse.local");
    let sig = sig.build().map_err(|e| anyhow!("vapid: {e}"))?;
    let mut b = WebPushMessageBuilder::new(&info);
    b.set_payload(ContentEncoding::Aes128Gcm, body);
    b.set_vapid_signature(sig);
    b.set_ttl(TTL_SECS);
    b.set_urgency(urgency);
    let msg = b.build().map_err(|e| anyhow!("web-push: {e}"))?;
    let req = web_push::request_builder::build_request::<Vec<u8>>(msg);
    let url = req.uri().to_string();
    let mut r = http.post(&url).timeout(std::time::Duration::from_secs(15));
    for (k, val) in req.headers().iter() { if let Ok(vs) = val.to_str() { r = r.header(k.as_str(), vs); } }
    let resp = r.body(req.into_body()).send().await.map_err(|e| anyhow!("push-служба: {e}"))?;
    let st = resp.status();
    if st.is_success() { return Ok(()) }
    let text = resp.text().await.unwrap_or_default();
    Err(anyhow!("{}: {}", st.as_u16(), text.chars().take(120).collect::<String>()))
}

/// Short request line for the notification: the Bash command, the file path or the tool
/// name
pub fn request_line(tool: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(|v| v.as_str()).map(|x| x.trim().replace('\n', " ")).unwrap_or_default();
    let text = match tool {
        "Bash" => format!("Bash: {}", s("command")),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => format!("{tool}: {}", s("file_path")),
        "Read" | "Glob" | "Grep" => format!("{tool}: {}", if s("file_path").is_empty() { s("pattern") } else { s("file_path") }),
        "AskUserQuestion" => "вопрос владельцу".to_string(),
        _ => { let desc = s("description"); if desc.is_empty() { tool.to_string() } else { format!("{tool}: {desc}") } }
    };
    if text.chars().count() > 120 { format!("{}…", text.chars().take(119).collect::<String>()) } else { text }
}

/// Scopes stopped by a spending limit (§8.3): the action is "stop" and today's usage is not below the limit —
/// the same arithmetic as the composer bar (`data.budgetOf`)
pub fn blocked_scopes(limits: &[Value], projects: &[Value], today: &Value) -> Vec<(String, f64, f64)> {
    let mut out = vec![];
    for l in limits {
        let scope = l.get("scope").and_then(|s| s.as_str()).unwrap_or(""); let per_day = l.get("per_day").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if scope.is_empty() || per_day <= 0.0 || l.get("action").and_then(|a| a.as_str()) != Some("stop") { continue }
        let spent = if scope == "total" { today.get("cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0) } else { projects.iter().find(|p| p.get("name").and_then(|n| n.as_str()) == Some(scope)).and_then(|p| p.get("cost_today")).and_then(|v| v.as_f64()).unwrap_or(0.0) };
        if spent >= per_day { out.push((scope.to_string(), spent, per_day)); }
    }
    out
}

/// Events by transitions between hub snapshots (called after the "Inbox" recompute)
pub async fn on_snapshot(app: &App, projects: &[Value], today: &Value, inbox: &Value) {
    let prefs = prefs(&app.store);
    // limits: a scope's transition to "stopped" → one
    // notification
    let limits = crate::api::settings_of(app).ok().and_then(|s| s.get("limits").and_then(|l| l.as_array()).cloned()).unwrap_or_default();
    let blocked_now = blocked_scopes(&limits, projects, today);
    let blocked_set: HashSet<String> = blocked_now.iter().map(|b| b.0.clone()).collect();
    let prev_blocked = { let mut g = app.push.limits_blocked.lock().unwrap(); let prev = g.clone(); *g = Some(blocked_set.clone()); prev };
    let mut limit_payloads: Vec<Value> = vec![];
    if let Some(prev_blocked) = prev_blocked {
        for (scope, spent, per_day) in &blocked_now {
            if prev_blocked.contains(scope) || !prefs.limit { continue }
            let who = if scope == "total" { "всего".to_string() } else { format!("для {scope}") };
            limit_payloads.push(json!({ "kind": "limit", "title": "Дневной лимит исчерпан", "body": format!("{who}: ${spent:.2} за 24 ч из ${per_day:.2} — новые ходы остановлены"), "tag": format!("limit-{scope}"), "url": "/agents/?section=models" }));
        }
    }
    let mut requests: HashSet<String> = HashSet::new();
    let mut status: HashMap<String, String> = HashMap::new();
    let mut new_requests: Vec<Value> = vec![];
    let mut total_pending = 0usize;
    for w in inbox.get("waiting").and_then(|w| w.as_array()).cloned().unwrap_or_default() {
        let project = w.get("project").and_then(|p| p.as_str()).unwrap_or("").to_string();
        let sess = w.get("session").cloned().unwrap_or(Value::Null);
        let id = sess.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
        let title = sess.get("title").and_then(|t| t.as_str()).unwrap_or("беседа").to_string();
        for p in w.get("pending").and_then(|p| p.as_array()).cloned().unwrap_or_default() {
            let rid = p.get("request_id").and_then(|r| r.as_str()).unwrap_or("").to_string();
            if rid.is_empty() { continue }
            total_pending += 1;
            let key = format!("{project}:{id}:{rid}");
            requests.insert(key.clone());
            new_requests.push(json!({ "kind": "permission", "key": key, "title": format!("{title} ждёт решения"), "body": request_line(p.get("tool_name").and_then(|t| t.as_str()).unwrap_or(""), p.get("input").unwrap_or(&Value::Null)),
                "project": project, "id": id, "request_id": rid, "tag": format!("perm-{id}"), "renotify": true, "url": format!("/agents/?open={project}/{id}"),
                "actions": [{ "action": "allow", "title": "Разрешить" }, { "action": "deny", "title": "Отклонить" }] }));
        }
    }
    let mut transitions: Vec<Value> = vec![];
    for p in projects {
        let project = p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        for s in p.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default() {
            let id = s.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
            let st = s.get("status").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let title = s.get("title").and_then(|t| t.as_str()).unwrap_or("беседа").to_string();
            let last = s.get("last_text").and_then(|t| t.as_str()).unwrap_or("").trim().chars().take(140).collect::<String>();
            status.insert(format!("{project}:{id}"), st.clone());
            transitions.push(json!({ "project": project, "id": id, "status": st, "title": title, "last": last }));
        }
    }
    let prev = { let mut g = app.push.seen.lock().unwrap(); let prev = g.clone(); *g = Some(Seen { requests: requests.clone(), status: status.clone() }); prev };
    // the first snapshot after start — only remember: otherwise every service restart would send a batch of notifications about old
    // requests
    let Some(prev) = prev else { return };
    let mut payloads: Vec<Value> = limit_payloads.drain(..).collect();
    if prefs.permission {
        for r in new_requests { if !prev.requests.contains(r["key"].as_str().unwrap_or("")) { let mut r = r; r["badge"] = json!(total_pending); if let Some(o) = r.as_object_mut() { o.remove("key"); } payloads.push(r); } }
    }
    for t in transitions {
        let key = format!("{}:{}", t["project"].as_str().unwrap_or(""), t["id"].as_str().unwrap_or(""));
        let before = prev.status.get(&key).map(String::as_str).unwrap_or("");
        let now = t["status"].as_str().unwrap_or("");
        if before == now || before.is_empty() { continue }
        let title = t["title"].as_str().unwrap_or("беседа"); let id = t["id"].as_str().unwrap_or(""); let project = t["project"].as_str().unwrap_or("");
        let url = format!("/agents/?open={project}/{id}");
        if now == "idle" && before == "working" && prefs.done {
            let last = t["last"].as_str().unwrap_or("");
            payloads.push(json!({ "kind": "done", "title": format!("{title}: ответ готов"), "body": if last.is_empty() { "ход завершён".to_string() } else { last.to_string() }, "project": project, "id": id, "tag": format!("done-{id}"), "url": url, "badge": total_pending }));
        } else if now == "error" && prefs.error {
            payloads.push(json!({ "kind": "error", "title": format!("{title}: ошибка"), "body": if t["last"].as_str().unwrap_or("").is_empty() { "агент упал" } else { t["last"].as_str().unwrap_or("") }, "project": project, "id": id, "tag": format!("err-{id}"), "url": url, "badge": total_pending }));
        }
    }
    if payloads.is_empty() { return }
    if subs(&app.store).is_empty() { return }
    for p in payloads { let r = send(&app.http, &app.store, &p, None).await; if !r.errors.is_empty() { tracing::debug!("push: {}", r.errors.join("; ")); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;
    use axum::Router;
    use axum::http::HeaderMap;
    use bytes::Bytes;

    fn store() -> (Store, std::path::PathBuf) { let dir = std::env::temp_dir().join(format!("agents-push-{}-{}", std::process::id(), uuid::Uuid::new_v4())); (Store::open(&dir).unwrap(), dir) }

    /// Push service mock: remembers the headers and body size, answers with the given status
    /// per path
    async fn mock_push(seen: Arc<Mutex<Vec<(String, HashMap<String, String>, usize)>>>) -> String {
        let s2 = seen.clone();
        let handler = move |axum::extract::Path(code): axum::extract::Path<u16>, headers: HeaderMap, body: Bytes| { let seen = s2.clone(); async move {
            let h: HashMap<String, String> = headers.iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect();
            seen.lock().unwrap().push((format!("/{code}"), h, body.len()));
            StatusCode::from_u16(code).unwrap()
        } };
        let app = Router::new().route("/push/{code}", post(handler));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(); let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap(); });
        format!("http://{addr}/push")
    }
    /// A real subscription key pair (p256dh + auth), as the browser makes it
    fn fake_sub(endpoint: &str) -> SubscribeBody {
        let sk = p256::SecretKey::random(&mut rand_core::OsRng);
        let p256dh = B64URL.encode(sk.public_key().to_encoded_point(false).as_bytes());
        SubscribeBody { endpoint: endpoint.into(), keys: SubKeys { p256dh, auth: B64URL.encode([7u8; 16]) }, ua: "Chrome Android".into() }
    }

    #[test]
    fn vapid_keys_are_generated_once_and_persist() {
        let (s, dir) = store();
        let a = vapid(&s).unwrap(); let b = vapid(&s).unwrap();
        assert_eq!(a.public, b.public); assert_eq!(a.private, b.private);
        assert_eq!(B64URL.decode(&a.public).unwrap().len(), 65, "несжатая точка P-256"); assert_eq!(B64URL.decode(&a.private).unwrap().len(), 32);
        let pubv = public(&s).unwrap();
        assert_eq!(pubv["public_key"], a.public); assert_eq!(pubv["count"], 0); assert_eq!(pubv["prefs"]["permission"], true);
        assert!(pubv.get("private").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn subscriptions_upsert_and_remove() {
        let (s, dir) = store();
        assert_eq!(upsert(&s, fake_sub("https://push.example/a")).unwrap(), 1);
        assert_eq!(upsert(&s, fake_sub("https://push.example/a")).unwrap(), 1, "тот же endpoint — замена, не дубль");
        assert_eq!(upsert(&s, fake_sub("https://push.example/b")).unwrap(), 2);
        assert!(upsert(&s, SubscribeBody { endpoint: "http://plain.example/x".into(), keys: SubKeys { p256dh: "k".into(), auth: "a".into() }, ua: String::new() }).is_err());
        let pubv = public(&s).unwrap();
        assert_eq!(pubv["subscriptions"][0]["host"], "push.example"); assert!(pubv["subscriptions"][0].get("p256dh").is_none(), "ключи наружу не отдаются");
        assert_eq!(remove(&s, "https://push.example/a").unwrap(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn send_encrypts_signs_and_drops_gone_subscriptions() {
        let (s, dir) = store();
        let seen = Arc::new(Mutex::new(vec![])); let base = mock_push(seen.clone()).await;
        upsert(&s, fake_sub(&format!("{base}/201"))).unwrap();
        upsert(&s, fake_sub(&format!("{base}/410"))).unwrap();
        upsert(&s, fake_sub(&format!("{base}/500"))).unwrap();
        let r = send(&reqwest::Client::new(), &s, &json!({ "kind": "permission", "title": "t", "body": "b" }), None).await;
        assert_eq!(r.sent, 1); assert_eq!(r.errors.len(), 2, "{:?}", r.errors);
        let got = seen.lock().unwrap().clone();
        let ok = got.iter().find(|g| g.0 == "/201").unwrap();
        assert_eq!(ok.1.get("content-encoding").map(String::as_str), Some("aes128gcm"));
        assert!(ok.1.get("authorization").map(|a| a.starts_with("vapid t=")).unwrap_or(false), "{:?}", ok.1.get("authorization"));
        assert_eq!(ok.1.get("ttl").map(String::as_str), Some("21600")); assert_eq!(ok.1.get("urgency").map(String::as_str), Some("high"));
        assert!(ok.2 > 40, "зашифрованное тело с заголовком ece");
        let left = subs(&s);
        assert_eq!(left.len(), 2, "410 снимает подписку, 500 — только счётчик неудач");
        assert!(left.iter().all(|x| !x.endpoint.ends_with("/410")));
        assert_eq!(left.iter().find(|x| x.endpoint.ends_with("/500")).unwrap().fails, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn blocked_scopes_follow_the_composer_arithmetic() {
        let limits = vec![json!({ "scope": "total", "per_day": 10.0, "action": "stop" }), json!({ "scope": "alpha", "per_day": 1.0, "action": "stop" }), json!({ "scope": "beta", "per_day": 1.0, "action": "warn" }), json!({ "scope": "gamma", "per_day": 0, "action": "stop" })];
        let projects = vec![json!({ "name": "alpha", "cost_today": 1.5 }), json!({ "name": "beta", "cost_today": 5.0 }), json!({ "name": "gamma", "cost_today": 9.0 })];
        let b = blocked_scopes(&limits, &projects, &json!({ "cost_usd": 4.0 }));
        assert_eq!(b, vec![("alpha".to_string(), 1.5, 1.0)], "warn и выключенный лимит не блокируют, total ниже лимита");
    }

    #[test]
    fn request_line_shapes() {
        assert_eq!(request_line("Bash", &json!({ "command": "rm -rf build\n&& npm run build" })), "Bash: rm -rf build && npm run build");
        assert_eq!(request_line("Edit", &json!({ "file_path": "/w/src/a.rs", "old_string": "x" })), "Edit: /w/src/a.rs");
        assert_eq!(request_line("WebFetch", &json!({ "url": "https://x", "description": "fetch docs" })), "WebFetch: fetch docs");
        assert_eq!(request_line("AskUserQuestion", &json!({})), "вопрос владельцу");
        assert!(request_line("Bash", &json!({ "command": "a".repeat(300) })).chars().count() <= 120);
    }
}
