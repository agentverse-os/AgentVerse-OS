//! Simple chat (docs/agents-ux.md §6.1–6.3): conversations directly over an OpenAI-compatible API through the core's routes
//! (`/api/llm/routes/{id}/chat` — the project gate, OpenRouter and other keys; the keys stay in the core), without a
//! container. A minimum of tools: images and text files as input, `web_search` (SearXNG / Brave / Tavily from the settings)
//! and `fetch_url` (a page as text). History — the `chats`/`chat_messages` tables in the service SQLite, attachments — as
//! files in the data directory; incognito chats live only in memory (TTL 2 h, up to 200 messages) and are never written to
//! disk, only a row with the money goes to the core's expense ledger. The answer is served as an SSE stream: delta · tool ·
//! done · error.
use crate::{App, ApiError};
use anyhow::{anyhow, bail, Result};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use base64::Engine;
use futures_util::{FutureExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const PROJECT: &str = "chat";
const INCOG_TTL: i64 = 2 * 3600;
const INCOG_MAX_MSGS: usize = 200;
const MAX_ROUNDS: usize = 5;
const FETCH_MAX_CHARS: usize = 20_000;
const FETCH_MAX_BYTES: usize = 2 * 1024 * 1024;
const ATTACH_MAX: usize = 8 * 1024 * 1024;
/// Body of a message request: the attachments in base64 (4/3 of their size) plus the text and the JSON around them. The
/// route needs this limit set explicitly — the framework default is 2 MiB, and a phone photo did not get through.
pub const SEND_BODY_MAX: usize = ATTACH_MAX / 3 * 4 + 512 * 1024;
const TEXT_ATTACH_MAX: usize = 200 * 1024;
/// Marks in `Message.error` that are states rather than provider errors: the owner stopped the answer; the answer was cut
/// by a service restart (the text written so far is kept). The interface shows them in its own words.
pub const STOPPED: &str = "stopped";
pub const INTERRUPTED: &str = "interrupted";
/// Text from a PDF (slice 37): up to 60 thousand
/// characters per attachment
const PDF_TEXT_MAX: usize = 60_000;

/// PDF text layer at upload (`pdf-extract`); scans without text give an empty string.
fn pdf_text(name: &str, mime: &str, bytes: &[u8]) -> String {
    if mime != "application/pdf" && !name.to_lowercase().ends_with(".pdf") { return String::new() }
    match pdf_extract::extract_text_from_mem(bytes) {
        Ok(t) => { let t = t.split_whitespace().collect::<Vec<_>>().join(" "); if t.chars().count() > PDF_TEXT_MAX { t.chars().take(PDF_TEXT_MAX).collect() } else { t } }
        Err(e) => { tracing::debug!("pdf {name}: {e}"); String::new() }
    }
}

/// `/img description` — an image instead of the model's
/// answer (slice 37).
pub fn img_prompt(text: &str) -> Option<String> {
    let t = text.trim();
    for p in ["/img ", "/картинка "] { if let Some(rest) = t.strip_prefix(p) { let r = rest.trim(); if !r.is_empty() { return Some(r.to_string()) } } }
    None
}
const MODELS_TTL: i64 = 600;
const TITLE_MAX: usize = 60;

fn now() -> i64 { chrono::Utc::now().timestamp() }
fn new_id(prefix: &str) -> String { format!("{prefix}_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]) }
fn short(s: &str, n: usize) -> String { let t: String = s.chars().take(n).collect(); if s.chars().count() > n { t + "…" } else { t } }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Tools { #[serde(default)] pub search: bool, #[serde(default)] pub fetch: bool }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Chat {
    #[serde(default)] pub id: String,
    #[serde(default)] pub title: String,
    #[serde(default)] pub route: String,
    #[serde(default)] pub model: String,
    #[serde(default)] pub system: String,
    #[serde(default)] pub tools: Tools,
    #[serde(default)] pub incognito: bool,
    #[serde(default)] pub created_at: i64,
    #[serde(default)] pub updated_at: i64,
    #[serde(default)] pub cost_usd: f64,
    #[serde(default)] pub tokens_in: u64,
    #[serde(default)] pub tokens_out: u64,
    #[serde(default)] pub messages: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Attachment { pub file: String, pub name: String, pub mime: String, pub size: u64, // / the PDF text layer (slice 37), empty for the others
    #[serde(default)] pub text: String }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ToolUse { pub name: String, pub args: Value, pub result: String, pub ok: bool }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Message {
    pub id: String,
    pub chat: String,
    pub seq: u64,
    /// user | assistant
    pub role: String,
    pub text: String,
    #[serde(default)] pub attachments: Vec<Attachment>,
    #[serde(default)] pub tools: Vec<ToolUse>,
    #[serde(default)] pub tokens_in: u64,
    #[serde(default)] pub tokens_out: u64,
    #[serde(default)] pub cost_usd: f64,
    pub created_at: i64,
    #[serde(default)] pub error: Option<String>,
}

/// Incognito chat: everything in memory,
/// attachments included.
pub struct Incog { pub chat: Chat, pub msgs: Vec<Message>, pub files: HashMap<String, (String, Vec<u8>)>, pub touched: i64 }

/// An answer in progress: the stop flag and what has been produced so far — a tab that opens the chat mid-answer shows
/// it, and the same text is saved to disk every few seconds so that a service restart does not lose it.
#[derive(Default)]
pub struct Live { pub stop: AtomicBool, /** the chat was deleted mid-answer: nothing of it is written */ pub gone: AtomicBool, pub msg: String, pub text: Mutex<String>, pub tools: Mutex<Vec<ToolUse>>, pub running: Mutex<Option<String>> }

#[derive(Default)]
pub struct ChatState {
    pub incog: Mutex<HashMap<String, Incog>>,
    pub busy: Mutex<HashMap<String, Arc<Live>>>,
    pub models: Mutex<HashMap<String, (i64, Value)>>,
}

/// The "answer in progress" mark of a chat. Dropping it removes the mark — on every way out of a turn, an early return
/// and a panic of the task included (a page that crashed the HTML parser used to leave the chat "thinking" until the
/// service restarted).
struct Busy { app: Arc<App>, id: String, live: Arc<Live> }
impl Busy {
    fn take(app: &Arc<App>, id: &str, msg: String) -> Option<Busy> {
        let mut busy = app.chat.busy.lock().unwrap_or_else(|e| e.into_inner());
        if busy.contains_key(id) { return None }
        let live = Arc::new(Live { msg, ..Default::default() });
        busy.insert(id.to_string(), live.clone());
        Some(Busy { app: app.clone(), id: id.to_string(), live })
    }
}
impl Drop for Busy { fn drop(&mut self) { self.app.chat.busy.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id); } }

/// Resolves when the turn is stopped: a wait for the provider's headers, for the next chunk or for a pause before a retry
/// ends at once instead of with the next byte from the provider.
async fn stopped(stop: &AtomicBool) { while !stop.load(Ordering::Relaxed) { tokio::time::sleep(std::time::Duration::from_millis(150)).await; } }

// ---------- storage: SQLite for regular chats, memory for incognito
// ----------

fn incog_get(app: &App, id: &str) -> Option<(Chat, Vec<Message>)> {
    let mut m = app.chat.incog.lock().unwrap();
    m.get_mut(id).map(|e| { e.touched = now(); (e.chat.clone(), e.msgs.clone()) })
}

pub fn load(app: &App, id: &str) -> Result<(Chat, Vec<Message>)> {
    if let Some(x) = incog_get(app, id) { return Ok(x); }
    let chat = app.store.chat_get(id)?.ok_or_else(|| anyhow!("чат не найден: {id}"))?;
    Ok((chat, app.store.chat_msgs(id)?))
}

fn save_chat(app: &App, chat: &Chat) -> Result<()> {
    app.hub.chats_changed();
    if chat.incognito {
        let mut m = app.chat.incog.lock().unwrap();
        match m.get_mut(&chat.id) { Some(e) => { e.chat = chat.clone(); e.touched = now(); }, None => { m.insert(chat.id.clone(), Incog { chat: chat.clone(), msgs: Vec::new(), files: HashMap::new(), touched: now() }); } }
        Ok(())
    } else { app.store.chat_put(chat) }
}

fn push_msg(app: &App, chat: &Chat, msg: &Message) -> Result<()> {
    app.hub.chats_changed();
    if chat.incognito {
        let mut m = app.chat.incog.lock().unwrap();
        let e = m.get_mut(&chat.id).ok_or_else(|| anyhow!("инкогнито-чат исчез: {}", chat.id))?;
        e.msgs.push(msg.clone()); e.touched = now();
        if e.msgs.len() > INCOG_MAX_MSGS { let cut = e.msgs.len() - INCOG_MAX_MSGS; e.msgs.drain(..cut); }
        Ok(())
    } else { app.store.chat_msg_put(msg) }
}

/// Cut the tail of a chat — the messages from `from_seq` on: a message being edited with everything after it, or an
/// answer being regenerated. Their files go too, except the ones in `keep` (attachments an edited message carries over).
fn cut_tail(app: &App, chat: &Chat, msgs: &mut Vec<Message>, from_seq: u64, keep: &[String]) -> Result<()> {
    let files: Vec<String> = msgs.iter().filter(|m| m.seq >= from_seq).flat_map(|m| m.attachments.iter().map(|a| a.file.clone())).filter(|f| !keep.contains(f)).collect();
    if chat.incognito {
        let mut m = app.chat.incog.lock().unwrap();
        let e = m.get_mut(&chat.id).ok_or_else(|| anyhow!("инкогнито-чат исчез: {}", chat.id))?;
        e.msgs.retain(|x| x.seq < from_seq);
        for f in &files { e.files.remove(f); }
        e.touched = now();
    } else {
        app.store.chat_msgs_cut(&chat.id, from_seq)?;
        for f in &files { let _ = std::fs::remove_file(files_dir(app, &chat.id).join(f)); }
    }
    msgs.retain(|m| m.seq < from_seq);
    app.hub.chats_changed();
    Ok(())
}

fn files_dir(app: &App, chat: &str) -> std::path::PathBuf { app.data.join("chat-files").join(chat) }

fn file_put(app: &App, chat: &Chat, file: &str, mime: &str, bytes: &[u8]) -> Result<()> {
    if chat.incognito {
        let mut m = app.chat.incog.lock().unwrap();
        let e = m.get_mut(&chat.id).ok_or_else(|| anyhow!("инкогнито-чат исчез"))?;
        e.files.insert(file.to_string(), (mime.to_string(), bytes.to_vec()));
        Ok(())
    } else {
        let dir = files_dir(app, &chat.id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(file), bytes)?;
        Ok(())
    }
}

pub fn file_get(app: &App, chat_id: &str, file: &str) -> Option<(String, Vec<u8>)> {
    if file.contains('/') || file.contains("..") { return None; }
    if let Some(e) = app.chat.incog.lock().unwrap().get(chat_id) { return e.files.get(file).cloned(); }
    let bytes = std::fs::read(files_dir(app, chat_id).join(file)).ok()?;
    Some((mime_guess::from_path(file).first_or_octet_stream().to_string(), bytes))
}

/// Remove incognito chats not accessed for longer than the TTL (scheduler tick).
pub fn sweep(app: &App) {
    let mut m = app.chat.incog.lock().unwrap();
    let cutoff = now() - INCOG_TTL;
    let dead: Vec<String> = m.iter().filter(|(_, e)| e.touched < cutoff).map(|(k, _)| k.clone()).collect();
    for k in dead { m.remove(&k); tracing::info!("инкогнито-чат {k} истёк"); }
}

// ---------- settings and routes ----------

#[derive(Deserialize, Clone, Debug, Default)]
pub struct SearchCfg { #[serde(default)] pub kind: String, #[serde(default)] pub url: String, #[serde(default)] pub key: String }

fn chat_settings(app: &App) -> Value { crate::api::settings_value(app).get("chat").cloned().unwrap_or(json!({})) }
fn search_cfg(app: &App) -> SearchCfg { serde_json::from_value(chat_settings(app).get("search").cloned().unwrap_or(json!({}))).unwrap_or_default() }
pub fn search_ready(app: &App) -> bool { let c = search_cfg(app); matches!(c.kind.as_str(), "searxng" | "brave" | "tavily") && (!c.url.trim().is_empty() || !c.key.trim().is_empty()) }

async fn route_models(app: &App, route: &str) -> Result<Value> {
    if let Some((ts, v)) = app.chat.models.lock().unwrap().get(route) { if now() - ts < MODELS_TTL { return Ok(v.clone()); } }
    let v = app.core_get(&format!("/llm/routes/{route}/models")).await?;
    app.chat.models.lock().unwrap().insert(route.to_string(), (now(), v.clone()));
    Ok(v)
}

/// Model price, $ per 1M tokens (input, output): the service's `prices` table, otherwise the route prices
/// (OpenRouter), otherwise 0.
async fn price_for(app: &App, route: &str, model: &str) -> (f64, f64, bool) {
    let prices = crate::api::settings_value(app).get("prices").cloned().unwrap_or(json!({}));
    if let Some(p) = prices.get(model).or_else(|| prices.get(model.split('/').last().unwrap_or(model))) {
        return (p.get("input").and_then(|x| x.as_f64()).unwrap_or(0.0), p.get("output").and_then(|x| x.as_f64()).unwrap_or(0.0), true);
    }
    if let Ok(v) = route_models(app, route).await {
        if let Some(m) = v.get("models").and_then(|m| m.as_array()).and_then(|a| a.iter().find(|m| m.get("id").and_then(|i| i.as_str()) == Some(model))) {
            if let (Some(i), Some(o)) = (m.get("price_in").and_then(|x| x.as_f64()), m.get("price_out").and_then(|x| x.as_f64())) { return (i, o, true); }
        }
    }
    (0.0, 0.0, false)
}

// ---------- tools ----------

fn tool_defs(tools: &Tools, search_ok: bool) -> Vec<Value> {
    let mut v = Vec::new();
    if tools.search && search_ok { v.push(json!({ "type": "function", "function": { "name": "web_search", "description": "Search the web. Use it for current events, facts you are not sure about, prices, documentation, news. Returns titles, URLs and snippets.", "parameters": { "type": "object", "properties": { "query": { "type": "string", "description": "search query" } }, "required": ["query"] } } })); }
    if tools.fetch { v.push(json!({ "type": "function", "function": { "name": "fetch_url", "description": "Fetch a web page or text file by URL and return its readable text (up to 20k characters). Use it to read a page the user gave or a search result.", "parameters": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] } } })); }
    v
}

/// An address the chat may fetch from: public unicast only. Loopback, private, link-local, CGNAT (the tailnet lives there),
/// multicast and reserved ranges are refused; an IPv4 address inside an IPv6 one (mapped, compatible, NAT64) is judged as
/// IPv4.
pub fn public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            !(a.is_loopback() || a.is_private() || a.is_link_local() || a.is_unspecified() || a.is_broadcast() || a.is_multicast() || a.is_documentation()
                || o[0] == 0 || (o[0] == 100 && (o[1] & 0xC0) == 64) || (o[0] == 192 && o[1] == 0 && o[2] == 0) || (o[0] == 198 && (o[1] & 0xFE) == 18) || o[0] >= 240)
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            if let Some(v4) = a.to_ipv4_mapped() { return public_ip(IpAddr::V4(v4)); }
            let embedded = |hi: u16, lo: u16| public_ip(IpAddr::V4(std::net::Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8)));
            if s[..6] == [0, 0, 0, 0, 0, 0] { return !(a.is_loopback() || a.is_unspecified()) && embedded(s[6], s[7]); }   // ::a.b.c.d
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] { return embedded(s[6], s[7]); }                    // NAT64 64:ff9b::/96
            !(a.is_multicast() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80 || (s[0] == 0x2001 && s[1] == 0x0db8))
        }
    }
}

/// The host of a URL as an address, when it is written as one. The URL parser has already brought every spelling to the
/// usual form: `2130706433`, `0x7f000001` and `0177.0.0.1` all come out as 127.0.0.1.
fn literal_ip(u: &reqwest::Url) -> Option<std::net::IpAddr> { u.host_str()?.trim_matches(['[', ']']).parse().ok() }

/// Check a URL for the page tool and resolve it: http(s) only, a public address only — for a name, every address it
/// resolves to. Returns the parsed URL and the addresses to connect to: the request is pinned to them, so that a second
/// DNS answer cannot lead elsewhere. (The old check compared strings and let through 127.0.0.1 written as a number, IPv6
/// literals, tailnet addresses and any name that resolves inside the network.)
pub(crate) async fn public_target(url: &str) -> Result<(reqwest::Url, Vec<std::net::SocketAddr>)> {
    let u = reqwest::Url::parse(url.trim()).map_err(|_| anyhow!("адрес не разобран"))?;
    if !matches!(u.scheme(), "http" | "https") { bail!("только http(s)-адреса"); }
    let port = u.port_or_known_default().unwrap_or(80);
    let inside = || anyhow!("адрес внутри сети недоступен из чата");
    if let Some(ip) = literal_ip(&u) {
        if !public_ip(ip) { return Err(inside()); }
        return Ok((u, vec![std::net::SocketAddr::new(ip, port)]));
    }
    let host = u.host_str().unwrap_or("").trim_end_matches('.').to_lowercase();
    if host.is_empty() || host == "localhost" || [".localhost", ".gate", ".internal", ".local"].iter().any(|z| host.ends_with(z)) { return Err(inside()); }
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), port)).await.map_err(|e| anyhow!("адрес не найден: {e}"))?.collect();
    if addrs.is_empty() { bail!("адрес не найден"); }
    if addrs.iter().any(|a| !public_ip(a.ip())) { return Err(inside()); }
    Ok((u, addrs))
}

/// Page text: without script/style/tags, collapsed whitespace, the title from
/// <title>.
pub fn html_to_text(html: &str) -> (String, String) {
    // ASCII lowercase only: it keeps every byte position of the source. `to_lowercase` changes the length of some
    // characters (İ, the kelvin sign), the positions drifted and a slice of the source panicked.
    let lower = html.to_ascii_lowercase();
    let title = lower.find("<title").and_then(|i| { let j = lower[i..].find('>')? + i + 1; let k = lower[j..].find("</title>")? + j; Some(html[j..k].trim().to_string()) }).unwrap_or_default();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &lower[i..];
            let skip_to = if rest.starts_with("<script") { rest.find("</script>").map(|e| i + e + 9) } else if rest.starts_with("<style") { rest.find("</style>").map(|e| i + e + 8) } else if rest.starts_with("<head") && !rest.starts_with("<header") { rest.find("</head>").map(|e| i + e + 7) } else if rest.starts_with("<!--") { rest.find("-->").map(|e| i + e + 3) } else { None };
            if let Some(e) = skip_to { i = e; out.push(' '); continue; }
            match html[i..].find('>') {
                Some(e) => { let tag = &lower[i + 1..i + e]; if tag.starts_with("br") || tag.starts_with("p") || tag.starts_with("/p") || tag.starts_with("div") || tag.starts_with("/div") || tag.starts_with("li") || tag.starts_with("h") || tag.starts_with("tr") { out.push('\n'); } else { out.push(' '); } i += e + 1; }
                None => break,
            }
        } else {
            let ch = html[i..].chars().next().unwrap();
            out.push(ch); i += ch.len_utf8();
        }
    }
    let unescape = |t: &str| t.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    let title = unescape(&title);
    let out = unescape(&out);
    let mut text = String::new();
    for line in out.lines() { let l: Vec<&str> = line.split_whitespace().collect(); if !l.is_empty() { text.push_str(&l.join(" ")); text.push('\n'); } }
    (title, text)
}

async fn fetch_url(url: &str) -> Result<String> { fetch_page(public_target(url).await?).await }

/// Fetch a page from a checked target. The request goes to the addresses that passed the check; a redirect is not followed
/// by the client — the next hop is checked the same way first (up to five).
async fn fetch_page(mut target: (reqwest::Url, Vec<std::net::SocketAddr>)) -> Result<String> {
    let mut hops = 0;
    let resp = loop {
        let (u, addrs) = &target;
        let mut b = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(15));
        if literal_ip(u).is_none() { b = b.resolve_to_addrs(u.host_str().unwrap_or(""), addrs); }
        let resp = b.build()?.get(u.clone()).header("User-Agent", "Mozilla/5.0 (compatible; AgentVerse chat)").header("Accept", "text/html, text/plain, application/json;q=0.9, */*;q=0.5").send().await.map_err(|e| anyhow!("не загрузилось: {e}"))?;
        if !resp.status().is_redirection() { break resp; }
        let next = resp.headers().get("location").and_then(|v| v.to_str().ok()).and_then(|l| u.join(l).ok()).ok_or_else(|| anyhow!("перенаправление без адреса"))?;
        hops += 1;
        if hops > 5 { bail!("слишком много перенаправлений"); }
        target = public_target(next.as_str()).await?;
    };
    let url = resp.url().to_string();
    let status = resp.status();
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await { let c = chunk?; buf.extend_from_slice(&c); if buf.len() >= FETCH_MAX_BYTES { break; } }
    if !status.is_success() { bail!("HTTP {status}"); }
    let body = String::from_utf8_lossy(&buf).to_string();
    let (title, text) = if ctype.contains("html") { html_to_text(&body) } else { (String::new(), body) };
    let text: String = text.chars().take(FETCH_MAX_CHARS).collect();
    Ok(format!("URL: {url}\n{}\n{text}", if title.is_empty() { String::new() } else { format!("Title: {title}") }))
}

async fn web_search(app: &App, query: &str) -> Result<String> {
    let cfg = search_cfg(app);
    let q = query.trim();
    if q.is_empty() { bail!("пустой запрос"); }
    let items: Vec<(String, String, String)> = match cfg.kind.as_str() {
        "searxng" => {
            let base = cfg.url.trim().trim_end_matches('/');
            if base.is_empty() { bail!("адрес SearXNG не задан в настройках чата"); }
            let v: Value = app.http.get(format!("{base}/search")).query(&[("q", q), ("format", "json"), ("language", "auto")]).timeout(std::time::Duration::from_secs(20)).send().await.map_err(|e| anyhow!("SearXNG: {e}"))?.error_for_status().map_err(|e| anyhow!("SearXNG: {e}"))?.json().await?;
            v.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default().iter().take(8).map(|r| (s(r, "title"), s(r, "url"), s(r, "content"))).collect()
        }
        "brave" => {
            if cfg.key.trim().is_empty() { bail!("ключ Brave не задан"); }
            let v: Value = app.http.get("https://api.search.brave.com/res/v1/web/search").query(&[("q", q), ("count", "8")]).header("X-Subscription-Token", cfg.key.trim()).header("Accept", "application/json").timeout(std::time::Duration::from_secs(20)).send().await.map_err(|e| anyhow!("Brave: {e}"))?.error_for_status().map_err(|e| anyhow!("Brave: {e}"))?.json().await?;
            v.pointer("/web/results").and_then(|r| r.as_array()).cloned().unwrap_or_default().iter().take(8).map(|r| (s(r, "title"), s(r, "url"), s(r, "description"))).collect()
        }
        "tavily" => {
            if cfg.key.trim().is_empty() { bail!("ключ Tavily не задан"); }
            let v: Value = app.http.post("https://api.tavily.com/search").json(&json!({ "api_key": cfg.key.trim(), "query": q, "max_results": 8 })).timeout(std::time::Duration::from_secs(25)).send().await.map_err(|e| anyhow!("Tavily: {e}"))?.error_for_status().map_err(|e| anyhow!("Tavily: {e}"))?.json().await?;
            v.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default().iter().take(8).map(|r| (s(r, "title"), s(r, "url"), s(r, "content"))).collect()
        }
        _ => bail!("поиск не настроен: укажите SearXNG, Brave или Tavily в настройках чата"),
    };
    if items.is_empty() { return Ok("Ничего не найдено.".into()); }
    Ok(items.iter().enumerate().map(|(i, (t, u, c))| format!("{}. {} — {}\n   {}", i + 1, t, u, short(&c.replace('\n', " "), 300))).collect::<Vec<_>>().join("\n"))
}
fn s(v: &Value, k: &str) -> String { v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string() }

async fn run_tool(app: &App, name: &str, args: &Value) -> Result<String> {
    match name {
        "web_search" => web_search(app, args.get("query").and_then(|q| q.as_str()).unwrap_or("")).await,
        "fetch_url" => fetch_url(args.get("url").and_then(|q| q.as_str()).unwrap_or("")).await,
        other => bail!("неизвестный инструмент {other}"),
    }
}

// ---------- messages for the provider
// ----------

fn system_prompt(chat: &Chat) -> String {
    let base = if chat.system.trim().is_empty() { "Ты полезный ассистент. Отвечай на языке собеседника, по делу, с markdown там, где он помогает (списки, таблицы, код). Если пользовался поиском или страницами, в конце дай короткий список источников с адресами.".to_string() } else { chat.system.trim().to_string() };
    format!("{base}\nСегодня {}.", chrono::Local::now().format("%Y-%m-%d"))
}

fn provider_messages(app: &App, chat: &Chat, msgs: &[Message]) -> Vec<Value> {
    let mut out = vec![json!({ "role": "system", "content": system_prompt(chat) })];
    for m in msgs {
        if m.role == "assistant" { if !m.text.is_empty() { out.push(json!({ "role": "assistant", "content": m.text })); } continue; }
        let mut parts: Vec<Value> = Vec::new();
        let mut text = m.text.clone();
        for a in &m.attachments {
            if a.mime.starts_with("image/") {
                if let Some((mime, bytes)) = file_get(app, &chat.id, &a.file) { parts.push(json!({ "type": "image_url", "image_url": { "url": format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)) } })); }
            } else if !a.text.is_empty() {
                text.push_str(&format!("\n\nФайл «{}» (PDF, текстовый слой):\n```\n{}\n```", a.name, a.text));
            } else if a.mime == "application/pdf" || a.name.to_lowercase().ends_with(".pdf") {
                text.push_str(&format!("\n\nФайл «{}»: PDF без текстового слоя (скан) — текст не извлечён.", a.name));
            } else if let Some((_, bytes)) = file_get(app, &chat.id, &a.file) {
                let body: String = String::from_utf8_lossy(&bytes[..bytes.len().min(TEXT_ATTACH_MAX)]).to_string();
                text.push_str(&format!("\n\nФайл «{}»:\n```\n{}\n```", a.name, body));
            }
        }
        if parts.is_empty() { out.push(json!({ "role": "user", "content": text })); }
        else { parts.insert(0, json!({ "type": "text", "text": if text.is_empty() { "Смотри вложение.".to_string() } else { text } })); out.push(json!({ "role": "user", "content": parts })); }
    }
    out
}

// ---------- answer generation: a streaming call of the core route, tools, result
// ----------

#[derive(Default)]
struct Turn { text: String, tool_calls: Vec<(String, String, String)>, usage_in: u64, usage_out: u64, finish: String,
    /// The stream broke after the answer had started: the text received so far stays, the answer ends with this error.
    error: Option<String> }

/// One provider call: reading SSE (or JSON) → deltas into the channel and into the live text, tool calls and usage — into
/// Turn. `choice` is the `tool_choice` of the request ("auto"; "none" for the round that must give the answer itself).
async fn call_provider(app: &App, chat: &Chat, messages: &[Value], tools: &[Value], choice: &str, tx: &tokio::sync::mpsc::Sender<Value>, live: &Live) -> Result<Turn> {
    let stop = &live.stop;
    let halted = || Turn { finish: "stop".into(), ..Default::default() };
    let mut body = json!({ "model": chat.model, "messages": messages, "stream": true, "stream_options": { "include_usage": true } });
    if !tools.is_empty() { body["tools"] = json!(tools); body["tool_choice"] = json!(choice); }
    let url = format!("{}/api/llm/routes/{}/chat", app.cloudd, chat.route);
    // Transient failures — retry with a pause (3, 8, 15 s): the core restarts on an update (the connection drops), the provider
    // behind the gate temporarily has no capacity (503) or quota (429); the answer headers are awaited for at most 3 minutes so a
    // hung provider does not hold the turn for ten minutes. Retries — only while nothing has been sent into the stream and the turn
    // is not stopped. A stop ends any of these waits at once.
    const WAITS: [u64; 3] = [3, 8, 15];
    let mut attempt = 0usize;
    let resp = loop {
        let sent = tokio::select! {
            r = tokio::time::timeout(std::time::Duration::from_secs(180), app.http.post(&url).json(&body).timeout(std::time::Duration::from_secs(600)).send()) => r,
            _ = stopped(stop) => return Ok(halted()),
        };
        let (why, retry) = match sent {
            Err(_) => bail!("провайдер не ответил за 3 минуты — попробуйте ещё раз или выберите другую модель"),
            Ok(Err(e)) if e.is_connect() || e.is_request() => (format!("ядро недоступно: {e}"), true),
            Ok(Err(e)) => bail!("ядро: {e}"),
            Ok(Ok(r)) => {
                let st = r.status().as_u16();
                if matches!(st, 429 | 502 | 503 | 504) && attempt < WAITS.len() && !stop.load(Ordering::Relaxed) {
                    let text = r.text().await.unwrap_or_default();
                    (format!("{st} {}", short(&text, 200)), true)
                } else { break r; }
            }
        };
        if !retry || attempt >= WAITS.len() || stop.load(Ordering::Relaxed) { bail!("{why} — повторите через минуту (возможно, идёт обновление)"); }
        let wait = WAITS[attempt]; attempt += 1;
        tracing::warn!("чат {}: {why}; повтор {attempt} через {wait} с", chat.id);
        let _ = tx.send(json!({ "kind": "tool_start", "name": format!("повтор {attempt} через {wait} с: {}", short(&why, 80)) })).await;
        tokio::select! { _ = tokio::time::sleep(std::time::Duration::from_secs(wait)) => {}, _ = stopped(stop) => return Ok(halted()) }
    };
    let status = resp.status();
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.pointer("/error/message").or_else(|| v.get("error")).and_then(|e| e.as_str().map(str::to_string).or_else(|| Some(e.to_string())))).unwrap_or_else(|| short(&text, 300));
        let hint = match status.as_u16() { 503 => " — у провайдера нет мощностей для этой модели: попробуйте позже или выберите другую модель в меню", 429 => " — лимит или квота провайдера: подождите или выберите другой маршрут", _ => "" };
        bail!("{} {}{}", status.as_u16(), msg, hint);
    }
    let mut turn = Turn::default();
    let mut calls: Vec<(String, String, String)> = Vec::new(); // (id, name, arguments) by index
    let handle_chunk = |v: &Value, turn: &mut Turn, calls: &mut Vec<(String, String, String)>| -> Option<String> {
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) { turn.usage_in = u.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(turn.usage_in); turn.usage_out = u.get("completion_tokens").and_then(|x| x.as_u64()).unwrap_or(turn.usage_out); }
        let ch = v.get("choices").and_then(|c| c.as_array()).and_then(|c| c.first())?;
        let delta = ch.get("delta").or_else(|| ch.get("message"))?;
        let mut piece = None;
        if let Some(t) = delta.get("content").and_then(|c| c.as_str()) { if !t.is_empty() { turn.text.push_str(t); piece = Some(t.to_string()); } }
        if let Some(tc) = delta.get("tool_calls").and_then(|t| t.as_array()) {
            for c in tc {
                let idx = c.get("index").and_then(|i| i.as_u64()).unwrap_or(calls.len() as u64) as usize;
                while calls.len() <= idx { calls.push((String::new(), String::new(), String::new())); }
                if let Some(id) = c.get("id").and_then(|i| i.as_str()) { calls[idx].0 = id.to_string(); }
                if let Some(n) = c.pointer("/function/name").and_then(|i| i.as_str()) { calls[idx].1.push_str(n); }
                if let Some(a) = c.pointer("/function/arguments").and_then(|i| i.as_str()) { calls[idx].2.push_str(a); }
            }
        }
        if let Some(f) = ch.get("finish_reason").and_then(|f| f.as_str()) { turn.finish = f.to_string(); }
        piece
    };
    let emit = |p: String| { live.text.lock().unwrap().push_str(&p); json!({ "kind": "delta", "text": p }) };
    if ctype.contains("application/json") {
        let v: Value = resp.json().await?;
        if let Some(p) = handle_chunk(&v, &mut turn, &mut calls) { let _ = tx.send(emit(p)).await; }
    } else {
        let mut stream = resp.bytes_stream();
        // bytes, not text: a chunk may end in the middle of a character, and decoding chunk by chunk turned such a letter
        // into "�"; a line is decoded when it is complete
        let mut buf: Vec<u8> = Vec::new();
        let mut complete = false;
        'outer: loop {
            let chunk = tokio::select! { c = stream.next() => c, _ = stopped(stop) => { turn.finish = "stop".into(); break; } };
            let Some(chunk) = chunk else { break };
            let chunk = match chunk { Ok(c) => c, Err(e) => { turn.error = Some(format!("поток оборвался: {e}")); break; } };
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line = String::from_utf8_lossy(&buf[..pos]).trim_end_matches('\r').to_string();
                buf.drain(..=pos);
                let Some(data) = line.strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" { complete = true; break 'outer; }
                let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
                if let Some(e) = v.get("error") { turn.error = Some(e.get("message").and_then(|m| m.as_str()).map(str::to_string).unwrap_or_else(|| e.to_string())); break 'outer; }
                if let Some(p) = handle_chunk(&v, &mut turn, &mut calls) { let _ = tx.send(emit(p)).await; }
            }
        }
        // a connection closed without the end mark and without a finish reason is a cut answer, even when the transport
        // reports a clean end (a proxy on the way closes it that way)
        if !complete && turn.finish.is_empty() && turn.error.is_none() { turn.error = Some("поток оборвался: соединение закрылось до конца ответа".into()); }
    }
    turn.tool_calls = calls.into_iter().filter(|(_, n, _)| !n.is_empty()).collect();
    // a failure before anything arrived is an error of the whole call (the caller may retry it without tools); after the
    // answer has started the text received so far is returned together with the error
    if turn.text.is_empty() && turn.tool_calls.is_empty() { if let Some(e) = turn.error.take() { bail!("{e}"); } }
    Ok(turn)
}

async fn generate(app: Arc<App>, mut chat: Chat, mut msgs: Vec<Message>, tx: tokio::sync::mpsc::Sender<Value>, busy: Busy) {
    let started = std::time::Instant::now();
    let live = busy.live.clone();
    let stop = &live.stop;
    let search_ok = search_ready(&app);
    let mut tools = tool_defs(&chat.tools, search_ok);
    let mut messages = provider_messages(&app, &chat, &msgs);
    let seq = msgs.last().map(|m| m.seq + 1).unwrap_or(1);
    let mut msg = Message { id: live.msg.clone(), chat: chat.id.clone(), seq, role: "assistant".into(), created_at: now(), ..Default::default() };
    // Every few seconds what has been written so far goes to disk with the "interrupted" mark: after a service restart the
    // chat shows that text instead of losing the answer; the final write replaces it. Incognito stays in memory only.
    let saver = (!chat.incognito).then(|| { let (app, live, base) = (app.clone(), live.clone(), msg.clone()); tokio::spawn(async move {
        let mut saved = usize::MAX;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if live.gone.load(Ordering::Relaxed) { return }
            let text = live.text.lock().unwrap().clone();
            if text.len() == saved { continue }
            saved = text.len();
            let part = Message { text, tools: live.tools.lock().unwrap().clone(), error: Some(INTERRUPTED.into()), ..base.clone() };
            if let Err(e) = app.store.chat_msg_put(&part) { tracing::debug!("чат {}: черновик ответа: {e:#}", part.chat); }
        }
    }) });
    let mut text = String::new();
    let mut uses: Vec<ToolUse> = Vec::new();
    let (mut tin, mut tout) = (0u64, 0u64);
    let mut error: Option<String> = None;
    'rounds: for round in 0..=MAX_ROUNDS {
        // The tool rounds are used up: one more call for the answer itself, with tools forbidden. Without it the loop simply
        // ended after the fifth round of tools and the owner got an empty answer.
        let last = round == MAX_ROUNDS;
        if last { messages.push(json!({ "role": "user", "content": "Лимит обращений к инструментам исчерпан. Дай итоговый ответ по уже полученным данным, без новых вызовов." })); }
        let mut res = call_provider(&app, &chat, &messages, &tools, if last { "none" } else { "auto" }, &tx, &live).await;
        // not every provider knows `tool_choice: none` — then the same request with the usual choice
        if last && res.is_err() && !stop.load(Ordering::Relaxed) { res = call_provider(&app, &chat, &messages, &tools, "auto", &tx, &live).await; }
        let turn = match res {
            Ok(t) => t,
            Err(e) => {
                let es = e.to_string();
                // a model or route without tool calling: retry without tools
                if !tools.is_empty() && round == 0 && (es.contains("tool") || es.contains("function") || es.starts_with("400")) { tracing::info!("чат {}: без инструментов после ошибки: {es}", chat.id); tools.clear(); match call_provider(&app, &chat, &messages, &tools, "auto", &tx, &live).await { Ok(t) => t, Err(e2) => { error = Some(e2.to_string()); break; } } }
                else { error = Some(es); break; }
            }
        };
        tin += turn.usage_in; tout += turn.usage_out;
        text.push_str(&turn.text);
        if let Some(e) = turn.error { error = Some(e); break; }
        if turn.tool_calls.is_empty() || stop.load(Ordering::Relaxed) { break; }
        if last { error = Some(format!("модель {MAX_ROUNDS} раз подряд обратилась к инструментам и не дала ответа — уточните вопрос или повторите")); break; }
        // tool calls: the provider answer with tool_calls → results → the next round
        let calls_json: Vec<Value> = turn.tool_calls.iter().map(|(id, n, a)| json!({ "id": id, "type": "function", "function": { "name": n, "arguments": a } })).collect();
        messages.push(json!({ "role": "assistant", "content": if turn.text.is_empty() { Value::Null } else { json!(turn.text) }, "tool_calls": calls_json }));
        for (id, name, args_s) in &turn.tool_calls {
            let args: Value = serde_json::from_str(args_s).unwrap_or(json!({}));
            *live.running.lock().unwrap() = Some(name.clone());
            let _ = tx.send(json!({ "kind": "tool_start", "name": name, "args": args })).await;
            // a tool that panics is an error of this one call, not the end of the answer; a stop does not wait for the tool
            let run = tokio::select! { r = std::panic::AssertUnwindSafe(run_tool(&app, name, &args)).catch_unwind() => r, _ = stopped(stop) => break 'rounds };
            let (result, ok) = match run { Ok(Ok(r)) => (r, true), Ok(Err(e)) => (format!("Ошибка: {e:#}"), false), Err(_) => { tracing::error!("чат {}: инструмент {name} упал", chat.id); ("Ошибка: внутренний сбой инструмента".to_string(), false) } };
            *live.running.lock().unwrap() = None;
            let _ = tx.send(json!({ "kind": "tool", "name": name, "args": args, "result": short(&result, 1500), "ok": ok })).await;
            messages.push(json!({ "role": "tool", "tool_call_id": id, "content": result }));
            let used = ToolUse { name: name.clone(), args, result: short(&result, 4000), ok };
            live.tools.lock().unwrap().push(used.clone());
            uses.push(used);
        }
        if !turn.text.is_empty() { text.push('\n'); }
    }
    // the draft writer must be gone before the final write, or its late copy would overwrite the finished message
    if let Some(s) = saver { s.abort(); let _ = s.await; }
    if live.gone.load(Ordering::Relaxed) { return }
    if error.is_none() && stop.load(Ordering::Relaxed) { error = Some(STOPPED.into()); }
    if tin == 0 && tout == 0 { tin = (messages.iter().map(|m| m.to_string().len()).sum::<usize>() / 4) as u64; tout = (text.len() / 4) as u64; }
    let (pi, po, priced) = price_for(&app, &chat.route, &chat.model).await;
    let cost = (tin as f64) * pi / 1e6 + (tout as f64) * po / 1e6;
    msg.text = text; msg.tools = uses; msg.tokens_in = tin; msg.tokens_out = tout; msg.cost_usd = cost; msg.error = error.clone();
    if let Err(e) = push_msg(&app, &chat, &msg) { tracing::warn!("чат {}: сообщение не записалось: {e:#}", chat.id); }
    msgs.push(msg.clone());
    chat.cost_usd += cost; chat.tokens_in += tin; chat.tokens_out += tout; chat.updated_at = now(); chat.messages = msgs.len() as u64;
    if chat.title.trim().is_empty() { if let Some(u) = msgs.iter().find(|m| m.role == "user") { let t: String = u.text.split_whitespace().collect::<Vec<_>>().join(" "); chat.title = short(if t.is_empty() { "Вложение" } else { &t }, TITLE_MAX); } }
    let _ = save_chat(&app, &chat);
    // the core's expense ledger: numbers only, no text there even for
    // incognito
    let rec = json!({ "project": PROJECT, "session": chat.id, "seq": seq, "agent": "api", "model": chat.model, "input": tin, "output": tout, "cost_usd": cost, "duration_ms": started.elapsed().as_millis() as u64, "source": if priced { "api" } else { "api-unpriced" } });
    if let Err(e) = app.core_post("/usage/record", &rec).await { tracing::debug!("usage/record: {e:#}"); }
    // the mark is taken off before the last event: a client that reloads the chat on "done" must not find it busy
    drop(busy);
    match error.as_deref() {
        Some(e) if e != STOPPED => { let _ = tx.send(json!({ "kind": "error", "error": e, "message": msg, "chat": chat })).await; }
        _ => { let _ = tx.send(json!({ "kind": "done", "message": msg, "chat": chat })).await; }
    }
}

// ---------- HTTP ----------

type R = Result<Json<Value>, ApiError>;

fn with_count(app: &App, c: &Chat) -> Value { let mut v = serde_json::to_value(c).unwrap_or(json!({})); v["kind"] = json!("simple"); if c.incognito { if let Some(e) = app.chat.incog.lock().unwrap().get(&c.id) { v["messages"] = json!(e.msgs.len()); } } v }

pub async fn list(State(app): State<Arc<App>>) -> R {
    let mut out: Vec<Value> = app.store.chats()?.iter().map(|c| with_count(&app, c)).collect();
    out.extend(app.chat.incog.lock().unwrap().values().map(|e| { let mut v = serde_json::to_value(&e.chat).unwrap_or(json!({})); v["kind"] = json!("simple"); v["messages"] = json!(e.msgs.len()); v["expires_at"] = json!(e.touched + INCOG_TTL); v }));
    out.sort_by(|a, b| b.get("updated_at").and_then(|x| x.as_i64()).unwrap_or(0).cmp(&a.get("updated_at").and_then(|x| x.as_i64()).unwrap_or(0)));
    Ok(Json(json!({ "chats": out })))
}

#[derive(Deserialize)]
pub struct NewChat { #[serde(default)] pub route: String, #[serde(default)] pub model: String, #[serde(default)] pub system: String, #[serde(default)] pub tools: Option<Tools>, #[serde(default)] pub incognito: bool, #[serde(default)] pub title: String }

pub async fn create(State(app): State<Arc<App>>, Json(b): Json<NewChat>) -> R {
    let cs = chat_settings(&app);
    let route = if b.route.trim().is_empty() { cs.get("route").and_then(|r| r.as_str()).unwrap_or("").to_string() } else { b.route.trim().to_string() };
    let model = if b.model.trim().is_empty() { cs.get("model").and_then(|r| r.as_str()).unwrap_or("").to_string() } else { b.model.trim().to_string() };
    let (route, model) = if route.is_empty() || model.is_empty() {
        // no defaults: the first route and its first model
        let routes = app.core_get("/llm/routes").await?;
        let first = routes.get("routes").and_then(|r| r.as_array()).and_then(|a| a.first()).and_then(|r| r.get("id").and_then(|i| i.as_str())).map(str::to_string).ok_or_else(|| anyhow!("нет ни одного маршрута к моделям: подключите gate или добавьте ключ в «Настройках → Аккаунты»"))?;
        let r = if route.is_empty() { first } else { route };
        let m = if model.is_empty() { route_models(&app, &r).await?.get("models").and_then(|m| m.as_array()).and_then(|a| a.first()).and_then(|m| m.get("id").and_then(|i| i.as_str())).map(str::to_string).ok_or_else(|| anyhow!("у маршрута {r} нет моделей"))? } else { model };
        (r, m)
    } else { (route, model) };
    let tools = b.tools.unwrap_or_else(|| Tools { search: search_ready(&app), fetch: true });
    let chat = Chat { id: new_id("ch"), title: b.title.trim().to_string(), route, model, system: b.system.trim().to_string(), tools, incognito: b.incognito, created_at: now(), updated_at: now(), ..Default::default() };
    save_chat(&app, &chat)?;
    Ok(Json(with_count(&app, &chat)))
}

pub async fn get(State(app): State<Arc<App>>, Path(id): Path<String>) -> R {
    let (chat, mut msgs) = load(&app, &id).map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    let live = app.chat.busy.lock().unwrap().get(&id).cloned();
    let mut out = json!({ "chat": with_count(&app, &chat), "busy": live.is_some() });
    if let Some(l) = &live {
        // the answer being written is not a message yet: its draft on disk stays out of the list, what is written so far
        // goes separately — a tab opened mid-answer shows the text instead of a bare "thinking"
        msgs.retain(|m| m.id != l.msg);
        out["partial"] = json!({ "text": l.text.lock().unwrap().clone(), "tools": l.tools.lock().unwrap().clone(), "running": l.running.lock().unwrap().clone() });
    }
    out["messages"] = json!(msgs);
    Ok(Json(out))
}

pub async fn update(State(app): State<Arc<App>>, Path(id): Path<String>, Json(patch): Json<Value>) -> R {
    let (mut chat, _) = load(&app, &id).map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    if let Some(t) = patch.get("title").and_then(|v| v.as_str()) { chat.title = short(t.trim(), 120); }
    if let Some(m) = patch.get("model").and_then(|v| v.as_str()) { if !m.trim().is_empty() { chat.model = m.trim().to_string(); } }
    if let Some(r) = patch.get("route").and_then(|v| v.as_str()) { if !r.trim().is_empty() { chat.route = r.trim().to_string(); } }
    if let Some(s) = patch.get("system").and_then(|v| v.as_str()) { chat.system = s.trim().to_string(); }
    if let Some(t) = patch.get("tools") { chat.tools = serde_json::from_value(t.clone())?; }
    chat.updated_at = now();
    save_chat(&app, &chat)?;
    Ok(Json(with_count(&app, &chat)))
}

pub async fn delete(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    // an answer still being written is stopped and told not to save anything: its final write used to bring the chat back
    if let Some(l) = app.chat.busy.lock().unwrap().get(&id) { l.gone.store(true, Ordering::Relaxed); l.stop.store(true, Ordering::Relaxed); }
    if app.chat.incog.lock().unwrap().remove(&id).is_some() { app.hub.chats_changed(); return Ok(StatusCode::NO_CONTENT); }
    if !app.store.chat_delete(&id)? { return Err(ApiError(StatusCode::NOT_FOUND, "чат не найден".into())); }
    let _ = std::fs::remove_dir_all(files_dir(&app, &id));
    app.hub.chats_changed();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AttachIn { pub name: String, #[serde(default)] pub mime: String, pub data: String }
#[derive(Deserialize)]
pub struct SendBody { #[serde(default)] pub text: String, #[serde(default)] pub attachments: Vec<AttachIn>, #[serde(default)] pub model: Option<String>, #[serde(default)] pub tools: Option<Tools>,
    /// Answer the owner's last question again: whatever came after it is dropped, no new message is added.
    #[serde(default)] pub regenerate: bool,
    /// The id of the owner's message being edited: it and everything after it is replaced by this message.
    #[serde(default)] pub replace_from: Option<String>,
    /// The attachments of the edited message that the new one keeps (their `file` names).
    #[serde(default)] pub keep_files: Vec<String> }

/// User message → an SSE stream with the answer (delta, tool_start, tool, done | error). The
/// refusal text if the "stop" limit for the simple chat (its project in the ledger) or the total
/// one is exhausted
async fn budget_block(app: &App) -> Option<String> {
    let st = app.core_get("/agents/budgets/state").await.ok()?;
    let b = st.as_array()?.iter().find(|b| b.get("blocked").and_then(|x| x.as_bool()) == Some(true) && matches!(b.get("scope").and_then(|x| x.as_str()), Some(sc) if sc == "total" || sc == PROJECT))?;
    let who = if b.get("scope").and_then(|x| x.as_str()) == Some("total") { "всего".to_string() } else { format!("для {PROJECT}") };
    Some(format!("дневной лимит ${:.2} {who} исчерпан (${:.2} за 24 ч): новые ходы остановлены, пока расход не снизится; лимит — в «Моделях»", b.get("per_day").and_then(|x| x.as_f64()).unwrap_or(0.0), b.get("spent").and_then(|x| x.as_f64()).unwrap_or(0.0)))
}
pub async fn send(State(app): State<Arc<App>>, Path(id): Path<String>, Json(b): Json<SendBody>) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let (mut chat, mut msgs) = load(&app, &id).map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    let regen = b.regenerate;
    let edited = if regen { None } else { b.replace_from.as_deref().filter(|m| !m.is_empty()) };
    if !regen && b.text.trim().is_empty() && b.attachments.is_empty() && (edited.is_none() || b.keep_files.is_empty()) { return Err(ApiError(StatusCode::BAD_REQUEST, "пустое сообщение".into())); }
    // where the tail of the chat is cut: after the last question (regenerate) or from the edited message on
    let cut_from: Option<u64> = if regen {
        Some(msgs.iter().rev().find(|m| m.role == "user").ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "нечего повторять: в чате нет вопроса".into()))?.seq + 1)
    } else if let Some(mid) = edited {
        Some(msgs.iter().find(|m| m.id == mid && m.role == "user").ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "сообщение для правки не найдено".into()))?.seq)
    } else { None };
    // core spending limits (§8.3): the simple chat goes to the provider itself, so it checks the limit state here
    if let Some(m) = budget_block(&app).await { return Err(ApiError(StatusCode::PAYMENT_REQUIRED, m)); }
    // from here on every way out — an error below, the end of the answer, a panic of its task — takes the mark off
    let busy = Busy::take(&app, &id, new_id("m")).ok_or_else(|| ApiError(StatusCode::CONFLICT, "ответ ещё пишется — дождитесь или остановите".into()))?;
    if let Some(m) = b.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) { chat.model = m.to_string(); }
    if let Some(t) = b.tools { chat.tools = t; }
    // an edited message keeps the attachments the owner left on it
    let mut attachments: Vec<Attachment> = edited.and_then(|mid| msgs.iter().find(|m| m.id == mid)).map(|m| m.attachments.iter().filter(|a| b.keep_files.contains(&a.file)).cloned().collect()).unwrap_or_default();
    let mut total: usize = attachments.iter().map(|a| a.size as usize).sum();
    // new attachments are decoded before anything is cut or written: a bad one leaves the chat as it was
    let mut fresh: Vec<(Attachment, Vec<u8>)> = Vec::new();
    if !regen {
        for a in &b.attachments {
            let bytes = base64::engine::general_purpose::STANDARD.decode(a.data.trim()).map_err(|_| ApiError(StatusCode::BAD_REQUEST, format!("вложение {} не в base64", a.name)))?;
            total += bytes.len();
            if total > ATTACH_MAX { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "вложения больше 8 МБ".into())); }
            let mime = if a.mime.trim().is_empty() { mime_guess::from_path(&a.name).first_or_octet_stream().to_string() } else { a.mime.trim().to_string() };
            let ext = std::path::Path::new(&a.name).extension().and_then(|e| e.to_str()).unwrap_or("bin").to_lowercase();
            let file = format!("{}.{}", new_id("f"), ext.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect::<String>());
            let text = pdf_text(&a.name, &mime, &bytes);
            fresh.push((Attachment { file, name: short(&a.name, 120), mime, size: bytes.len() as u64, text }, bytes));
        }
    }
    if let Some(from) = cut_from { let keep: Vec<String> = attachments.iter().map(|a| a.file.clone()).collect(); cut_tail(&app, &chat, &mut msgs, from, &keep)?; }
    for (a, bytes) in fresh { file_put(&app, &chat, &a.file, &a.mime, &bytes)?; attachments.push(a); }
    let (tx, rx) = tokio::sync::mpsc::channel::<Value>(64);
    let user = if regen {
        // the question stays as it is — the cut above left it the last message
        msgs.last().cloned().filter(|m| m.role == "user").ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "нечего повторять: в чате нет вопроса".into()))?
    } else {
        let seq = msgs.last().map(|m| m.seq + 1).unwrap_or(1);
        let user = Message { id: new_id("m"), chat: chat.id.clone(), seq, role: "user".into(), text: b.text.trim().to_string(), attachments, created_at: now(), ..Default::default() };
        push_msg(&app, &chat, &user)?;
        msgs.push(user.clone());
        let _ = tx.send(json!({ "kind": "user", "message": user.clone() })).await;
        user
    };
    chat.updated_at = now(); chat.messages = msgs.len() as u64;
    let _ = save_chat(&app, &chat);
    if let Some(prompt) = img_prompt(&user.text) { tokio::spawn(generate_image(app.clone(), chat, msgs, prompt, tx, busy)); }
    else { tokio::spawn(generate(app.clone(), chat, msgs, tx, busy)); }
    let stream = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|v| (Ok::<Event, std::convert::Infallible>(Event::default().data(v.to_string())), rx)) });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)).text("ping")))
}

/// Model for images: from the chat settings (`image_model`), otherwise the route's first `gpt-image-*` (the gate
/// accepts only those on `/images/generations`), otherwise the first model of kind image.
async fn pick_image_model(app: &App, route: &str) -> Option<String> {
    let cfg = chat_settings(app).get("image_model").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    if cfg.is_some() { return cfg }
    let v = route_models(app, route).await.ok()?;
    let list: Vec<&Value> = v.get("models").and_then(|m| m.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
    let images: Vec<String> = list.iter().filter(|m| m.get("kind").and_then(|k| k.as_str()) == Some("image")).filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect();
    images.iter().find(|i| i.starts_with("gpt-image")).or_else(|| images.first()).cloned()
}

/// `/img`: an image through the core (`POST /llm/routes/{id}/images`), the file — into the model message's attachment; the cost
/// does not go to the ledger.
async fn generate_image(app: Arc<App>, mut chat: Chat, msgs: Vec<Message>, prompt: String, tx: tokio::sync::mpsc::Sender<Value>, busy: Busy) {
    let fail = |m: String| json!({ "kind": "error", "error": m, "message": Value::Null, "chat": Value::Null });
    let Some(model) = pick_image_model(&app, &chat.route).await else {
        let _ = tx.send(fail("на маршруте нет моделей для картинок (gpt-image-*) — задайте image_model в настройках чата".into())).await;
        return;
    };
    let _ = tx.send(json!({ "kind": "tool_start", "name": "image", "args": { "prompt": prompt, "model": model } })).await;
    let res = app.core_post(&format!("/llm/routes/{}/images", chat.route), &json!({ "model": model, "prompt": prompt, "n": 1, "size": "1024x1024" })).await;
    let v = match res { Ok(v) => v, Err(e) => { let _ = tx.send(fail(format!("картинка: {e:#}"))).await; return; } };
    let item = v.pointer("/data/0").cloned().unwrap_or(Value::Null);
    let bytes: Option<Vec<u8>> = if let Some(b) = item.get("b64_json").and_then(|x| x.as_str()) { base64::engine::general_purpose::STANDARD.decode(b.trim()).ok() }
        else if let Some(u) = item.get("url").and_then(|x| x.as_str()) {
            match app.http.get(u).timeout(std::time::Duration::from_secs(120)).send().await { Ok(r) if r.status().is_success() => r.bytes().await.ok().map(|b| b.to_vec()), _ => None }
        } else { None };
    let Some(bytes) = bytes else { let _ = tx.send(fail(format!("картинка: провайдер не вернул данных ({})", short(&v.to_string(), 200)))).await; return; };
    if busy.live.gone.load(Ordering::Relaxed) { return }
    let mime = if bytes.starts_with(b"\x89PNG") { "image/png" } else if bytes.starts_with(&[0xFF, 0xD8]) { "image/jpeg" } else if bytes.starts_with(b"RIFF") { "image/webp" } else { "image/png" };
    let file = format!("{}.{}", new_id("f"), if mime == "image/jpeg" { "jpg" } else if mime == "image/webp" { "webp" } else { "png" });
    if let Err(e) = file_put(&app, &chat, &file, mime, &bytes) { let _ = tx.send(fail(format!("картинка: {e:#}"))).await; return; }
    let _ = tx.send(json!({ "kind": "tool", "name": "image", "args": { "prompt": prompt, "model": model }, "result": format!("{} байт, {mime}", bytes.len()), "ok": true })).await;
    let seq = msgs.last().map(|m| m.seq + 1).unwrap_or(1);
    let msg = Message { id: busy.live.msg.clone(), chat: chat.id.clone(), seq, role: "assistant".into(), text: format!("Картинка: {prompt} · {model}"), attachments: vec![Attachment { file, name: "image.png".into(), mime: mime.into(), size: bytes.len() as u64, text: String::new() }], created_at: now(), ..Default::default() };
    if let Err(e) = push_msg(&app, &chat, &msg) { let _ = tx.send(fail(format!("картинка: {e:#}"))).await; return; }
    chat.updated_at = now(); chat.messages = (msgs.len() + 1) as u64;
    if chat.title.trim().is_empty() { chat.title = short(&format!("Картинка: {prompt}"), 60); }
    let _ = save_chat(&app, &chat);
    drop(busy);
    let _ = tx.send(json!({ "kind": "done", "message": msg, "chat": with_count(&app, &chat) })).await;
}

/// System prompt presets (slice 37): kv `chat.presets`, three templates by default.
fn presets(app: &App) -> Vec<Value> {
    if let Some(v) = app.store.get_kv("chat.presets").ok().flatten().and_then(|v| v.as_array().cloned()) { return v }
    vec![
        json!({ "id": "p-brief", "name": "Кратко и по делу", "text": "Отвечай коротко и по делу, без вступлений и повторов вопроса. Если нужен список — маркированный, не больше семи пунктов." }),
        json!({ "id": "p-review", "name": "Код-ревьюер", "text": "Ты строгий код-ревьюер. Ищи ошибки, риски безопасности и неочевидные последствия. Сначала критичное, затем мелочи; к каждому замечанию — как исправить." }),
        json!({ "id": "p-translate", "name": "Переводчик", "text": "Переводи присланный текст между русским и английским, сохраняя смысл, тон и форматирование. Ничего не добавляй и не комментируй." }),
    ]
}
pub async fn presets_get(State(app): State<Arc<App>>) -> R { Ok(Json(json!({ "presets": presets(&app) }))) }
pub async fn presets_put(State(app): State<Arc<App>>, Json(b): Json<Value>) -> R {
    let list = b.get("presets").and_then(|p| p.as_array()).cloned().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "нужен presets: [{name, text}]".into()))?;
    if list.len() > 30 { return Err(ApiError(StatusCode::BAD_REQUEST, "не больше 30 пресетов".into())) }
    let mut out = Vec::new();
    for p in list {
        let name = p.get("name").and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
        let text = p.get("text").and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
        if name.is_empty() || text.is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "у пресета нужны имя и текст".into())) }
        let id = p.get("id").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()).map(str::to_string).unwrap_or_else(|| new_id("p"));
        out.push(json!({ "id": id, "name": short(name, 60), "text": short(text, 4000) }));
    }
    app.store.put_kv("chat.presets", &json!(out))?;
    Ok(Json(json!({ "presets": out })))
}

pub async fn stop(State(app): State<Arc<App>>, Path(id): Path<String>) -> R {
    let stopped = app.chat.busy.lock().unwrap().get(&id).map(|l| { l.stop.store(true, Ordering::Relaxed); true }).unwrap_or(false);
    Ok(Json(json!({ "stopped": stopped })))
}

/// Incognito → history: the chat, messages and attachments move to
/// disk.
pub async fn keep(State(app): State<Arc<App>>, Path(id): Path<String>) -> R {
    let e = app.chat.incog.lock().unwrap().remove(&id).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "инкогнито-чат не найден".into()))?;
    let mut chat = e.chat.clone(); chat.incognito = false; chat.updated_at = now();
    app.store.chat_put(&chat)?;
    for m in &e.msgs { app.store.chat_msg_put(m)?; }
    for (file, (mime, bytes)) in &e.files { file_put(&app, &chat, file, mime, bytes)?; }
    Ok(Json(with_count(&app, &chat)))
}

pub async fn file(State(app): State<Arc<App>>, Path((id, file)): Path<(String, String)>) -> Result<axum::response::Response, ApiError> {
    let (mime, bytes) = file_get(&app, &id, &file).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "файл не найден".into()))?;
    Ok(axum::response::Response::builder().header("content-type", mime).header("cache-control", "private, max-age=86400").body(axum::body::Body::from(bytes)).unwrap())
}

#[derive(Deserialize)]
pub struct HandoffBody { #[serde(default = "d_claude")] pub agent: String }
fn d_claude() -> String { "claude".into() }

/// Open in the advanced chat: a Claude Code / Codex session in scratch with the history as the
/// first message.
pub async fn handoff(State(app): State<Arc<App>>, Path(id): Path<String>, Json(b): Json<HandoffBody>) -> R {
    let (chat, msgs) = load(&app, &id).map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    if !matches!(b.agent.as_str(), "claude" | "codex" | "gemini" | "pi") { return Err(ApiError(StatusCode::BAD_REQUEST, "agent: claude | codex | gemini | pi".into())); }
    let id8 = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let body = json!({ "agent": b.agent, "cwd": format!("{}/{id8}", crate::chat::CHATS_DIR), "title": if chat.title.is_empty() { "из простого чата".to_string() } else { chat.title.clone() } });
    let s = app.core_post(&format!("/projects/{}/sessions", crate::chat::SCRATCH), &body).await.map_err(|e| anyhow!("scratch: {e:#} — запустите служебный workspace в разделе «Чат»"))?;
    let sid = s.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
    let mut transcript = String::from("Это продолжение беседы из простого чата (без инструментов контейнера). История:\n\n");
    for m in &msgs { transcript.push_str(&format!("**{}:** {}\n\n", if m.role == "user" { "Пользователь" } else { "Ассистент" }, short(&m.text, 6000))); }
    transcript.push_str("Продолжай с этого места; теперь тебе доступны файлы и команды. Если следующего вопроса нет, кратко подтверди, что контекст получен.");
    app.core_post(&format!("/projects/{}/sessions/{sid}/prompt", crate::chat::SCRATCH), &json!({ "content": transcript })).await?;
    Ok(Json(json!({ "session": s, "project": crate::chat::SCRATCH })))
}

/// Routes and their models (through the core, 10 min cache) + chat defaults and search
/// readiness.
pub async fn routes(State(app): State<Arc<App>>) -> R {
    let rs = app.core_get("/llm/routes").await?;
    let mut out = Vec::new();
    for r in rs.get("routes").and_then(|r| r.as_array()).cloned().unwrap_or_default() {
        let id = r.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
        let mut v = r.clone();
        match route_models(&app, &id).await { Ok(m) => { v["models"] = m.get("models").cloned().unwrap_or(json!([])); } Err(e) => { v["models"] = json!([]); v["error"] = json!(short(&e.to_string(), 200)); } }
        out.push(v);
    }
    let cs = chat_settings(&app);
    Ok(Json(json!({ "routes": out, "defaults": { "route": cs.get("route"), "model": cs.get("model") }, "search": { "ready": search_ready(&app), "kind": search_cfg(&app).kind } })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_and_tools() {
        let (t, x) = html_to_text("<html><head><title>Hi &amp; bye</title><style>p{}</style></head><body><script>var a=1;</script><p>One <b>two</b></p><div>three</div><!-- c --></body></html>");
        assert_eq!(t, "Hi & bye"); assert_eq!(x, "One two\nthree\n");
        let d = tool_defs(&Tools { search: true, fetch: true }, false);
        assert_eq!(d.len(), 1); assert_eq!(d[0].pointer("/function/name").and_then(|n| n.as_str()), Some("fetch_url"));
        let c = Chat { system: String::new(), ..Default::default() };
        assert!(system_prompt(&c).contains("Сегодня"));
    }
    /// Characters whose lowercase form has another byte length used to shift the positions and crash the parser (and the
    /// answer with it): the Turkish İ, the kelvin and ohm signs.
    #[test]
    fn html_with_length_changing_lowercase() {
        let (t, x) = html_to_text("<html><head><meta content=\"İİİİİİİİ\"><TITLE>300 \u{212A} в Стамбуле</TITLE></head><body><P>İstanbul \u{212A}\u{2126}</P><SCRIPT>var a=1;</SCRIPT><p>текст</p></body></html>");
        assert_eq!(t, "300 \u{212A} в Стамбуле");
        assert_eq!(x, "İstanbul \u{212A}\u{2126}\nтекст\n");
    }
    #[test]
    fn only_public_addresses() {
        let ip = |s: &str| public_ip(s.parse().unwrap());
        for a in ["8.8.8.8", "1.1.1.1", "172.40.0.1", "100.63.255.255", "100.128.0.1", "2606:4700:4700::1111", "64:ff9b::808:808"] { assert!(ip(a), "{a} is public"); }
        for a in ["127.0.0.1", "127.8.9.1", "10.77.1.2", "172.20.0.1", "192.168.1.1", "169.254.169.254", "0.0.0.0", "0.1.2.3", "100.64.0.1", "100.90.1.2", "100.127.255.255", "224.0.0.1", "255.255.255.255", "240.0.0.1", "198.18.0.1", "192.0.0.8",
                  "::1", "::", "::ffff:127.0.0.1", "::ffff:10.0.0.1", "::7f00:1", "64:ff9b::7f00:1", "64:ff9b::a00:1", "fc00::1", "fd7a:115c:a1e0::1", "fe80::1", "ff02::1", "2001:db8::1"] { assert!(!ip(a), "{a} is not public"); }
    }
    /// Every spelling of an inside address is refused, whatever it looks like as a string; nothing is resolved for these.
    #[tokio::test]
    async fn page_tool_targets() {
        for u in ["http://127.0.0.1:7100/api/status", "http://2130706433:7100/api/status", "http://0x7f000001:7100/", "http://0177.0.0.1:7100/", "http://127.1:7100/", "http://0/", "http://[::1]:7100/", "http://[::ffff:127.0.0.1]:7100/",
                  "http://100.90.1.2:9120/", "http://10.77.1.2/", "http://172.20.0.1/", "http://user@10.0.0.1/", "http://169.254.169.254/latest/meta-data/", "http://localhost:7100/", "http://LOCALHOST./", "http://llm.gate/v1", "http://db.internal/", "http://printer.local/"] {
            let r = public_target(u).await;
            assert!(r.is_err(), "{u} must be refused");
            assert!(r.unwrap_err().to_string().contains("внутри сети"), "{u}: the refusal names the reason");
        }
        for u in ["ftp://example.com/x", "file:///etc/passwd", "javascript:alert(1)", "not a url"] { assert!(public_target(u).await.is_err(), "{u} must be refused"); }
        let (u, addrs) = public_target(" https://8.8.8.8/x ").await.unwrap();
        assert_eq!((u.as_str(), addrs[0].to_string().as_str(), addrs.len()), ("https://8.8.8.8/x", "8.8.8.8:443", 1));
        assert_eq!(public_target("http://172.40.0.1:8080/").await.unwrap().1[0].to_string(), "172.40.0.1:8080");
    }
    /// The page tool against the real network (run by hand: `cargo test page_tool_on_the_internet -- --ignored`): a name is
    /// resolved and pinned, a redirect to a public address is followed.
    #[tokio::test]
    #[ignore = "needs the internet"]
    async fn page_tool_on_the_internet() {
        let page = fetch_url("https://example.com/").await.unwrap();
        assert!(page.contains("Example Domain"), "{page}");
        let redirected = fetch_url("http://github.com/").await.unwrap();
        assert!(redirected.starts_with("URL: https://github.com/"), "{}", &redirected[..redirected.len().min(120)]);
    }
    /// A redirect is checked like the first address: a public page that points inside the network is not followed. The
    /// first hop here is a local server handed over as already checked — the test is about the second one.
    #[tokio::test]
    async fn redirect_to_an_inside_address_is_not_followed() {
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h2 = hits.clone();
        let router = axum::Router::new()
            .route("/start", axum::routing::get(|| async { (StatusCode::FOUND, [("location", "/api/status")]) }))
            .route("/api/status", axum::routing::get(move || { let h = h2.clone(); async move { h.fetch_add(1, Ordering::Relaxed); "inside data" } }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, router).await.unwrap(); });
        let start = reqwest::Url::parse(&format!("http://127.0.0.1:{}/start", addr.port())).unwrap();
        let r = fetch_page((start, vec![addr])).await;
        assert!(r.as_ref().is_err_and(|e| e.to_string().contains("внутри сети")), "{r:?}");
        assert_eq!(hits.load(Ordering::Relaxed), 0, "the address the redirect points to was never requested");
    }
    #[test]
    fn body_limit_covers_the_attachments() {
        // 8 MB of attachments in base64 with the JSON around them must fit into the limit of the route
        assert!(SEND_BODY_MAX > ATTACH_MAX / 3 * 4 + 64 * 1024 && SEND_BODY_MAX < 16 * 1024 * 1024);
    }

    // ---------- a turn against a scripted provider: the core's chat route is replaced by a local server ----------

    use axum::response::IntoResponse;
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    enum Act { Bytes(Vec<u8>), Sleep(u64), Break }
    /// One answer of the provider: the status, a pause before the headers, then what the body does.
    struct Script { status: u16, wait: u64, acts: Vec<Act> }
    #[derive(Default)]
    struct Mock { scripts: Mutex<VecDeque<Script>>, seen: Mutex<Vec<Value>> }

    fn sse(v: Value) -> Act { Act::Bytes(format!("data: {v}\n\n").into_bytes()) }
    fn delta(t: &str) -> Act { sse(json!({ "choices": [{ "delta": { "content": t } }] })) }
    fn call(id: &str, url: &str) -> Act { sse(json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": id, "function": { "name": "fetch_url", "arguments": json!({ "url": url }).to_string() } }] }, "finish_reason": "tool_calls" }] })) }
    fn done() -> Act { Act::Bytes(b"data: [DONE]\n\n".to_vec()) }
    fn ok(acts: Vec<Act>) -> Script { Script { status: 200, wait: 0, acts } }

    async fn mock_chat(State(m): State<Arc<Mock>>, Json(body): Json<Value>) -> axum::response::Response {
        m.seen.lock().unwrap().push(body);
        let sc = m.scripts.lock().unwrap().pop_front().unwrap_or_else(|| ok(vec![delta("(no script)"), done()]));
        if sc.wait > 0 { tokio::time::sleep(Duration::from_millis(sc.wait)).await; }
        if sc.status != 200 { return (StatusCode::from_u16(sc.status).unwrap(), Json(json!({ "error": { "message": "scripted failure" } }))).into_response(); }
        let stream = futures_util::stream::unfold(sc.acts.into_iter(), |mut it| async move {
            loop { match it.next()? { Act::Bytes(b) => return Some((Ok::<_, std::io::Error>(bytes::Bytes::from(b)), it)), Act::Sleep(ms) => tokio::time::sleep(Duration::from_millis(ms)).await, Act::Break => return Some((Err(std::io::Error::other("cut")), it)) } }
        });
        axum::response::Response::builder().header("content-type", "text/event-stream").body(axum::body::Body::from_stream(stream)).unwrap()
    }

    /// The service state over a scripted core; the chat `ch_t` is created with the given messages (role, text).
    async fn rig(scripts: Vec<Script>, history: &[(&str, &str)]) -> (Arc<App>, Arc<Mock>, Chat, Vec<Message>, std::path::PathBuf) {
        let mock = Arc::new(Mock { scripts: Mutex::new(scripts.into()), ..Default::default() });
        let router = axum::Router::new()
            .route("/api/llm/routes/{id}/chat", axum::routing::post(mock_chat))
            .route("/api/llm/routes/{id}/models", axum::routing::get(|| async { Json(json!({ "models": [] })) }))
            .route("/api/usage/record", axum::routing::post(|| async { Json(json!({})) }))
            .route("/api/agents/budgets/state", axum::routing::get(|| async { Json(json!([])) }))
            .with_state(mock.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, router).await.unwrap(); });
        let dir = std::env::temp_dir().join(format!("agents-chat-{}-{}", std::process::id(), uuid::Uuid::new_v4()));
        let app = Arc::new(App { http: reqwest::Client::new(), cloudd: format!("http://{addr}"), store: crate::store::Store::open(&dir).unwrap(), data: dir.clone(), chat: Default::default(), runs: Default::default(), scratch_busy: Default::default(), scratch_error: Default::default(),
            hub: Default::default(), voice_streams: Arc::new(tokio::sync::Semaphore::new(1)), push: Default::default(), share: Default::default(), channels: Default::default() });
        let chat = Chat { id: "ch_t".into(), route: "r".into(), model: "m".into(), tools: Tools { search: false, fetch: true }, created_at: now(), updated_at: now(), ..Default::default() };
        app.store.chat_put(&chat).unwrap();
        let mut msgs = Vec::new();
        for (i, (role, text)) in history.iter().enumerate() {
            let m = Message { id: format!("m{}", i + 1), chat: chat.id.clone(), seq: i as u64 + 1, role: role.to_string(), text: text.to_string(), created_at: now(), ..Default::default() };
            app.store.chat_msg_put(&m).unwrap(); msgs.push(m);
        }
        (app, mock, chat, msgs, dir)
    }
    /// Run `generate` to the end and return the events it sent.
    async fn turn(app: &Arc<App>, chat: &Chat, msgs: &[Message]) -> Vec<Value> {
        let busy = Busy::take(app, &chat.id, new_id("m")).expect("the chat is free");
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Value>(256);
        generate(app.clone(), chat.clone(), msgs.to_vec(), tx, busy).await;
        let mut ev = Vec::new(); while let Ok(v) = rx.try_recv() { ev.push(v); } ev
    }
    fn last_kind(ev: &[Value]) -> &str { ev.last().and_then(|e| e.get("kind")).and_then(|k| k.as_str()).unwrap_or("") }
    fn stored(app: &App) -> Vec<Message> { app.store.chat_msgs("ch_t").unwrap() }

    /// A stream that breaks after the answer has started keeps the text the owner has already seen.
    #[tokio::test]
    async fn broken_stream_keeps_the_text_received() {
        let (app, _m, chat, msgs, dir) = rig(vec![ok(vec![delta("Привет, "), delta("это начало"), Act::Sleep(120), Act::Break])], &[("user", "Вопрос")]).await;   // the pause lets the headers and the first words leave before the cut
        let ev = turn(&app, &chat, &msgs).await;
        assert_eq!(last_kind(&ev), "error");
        let a = stored(&app).pop().unwrap();
        assert_eq!((a.role.as_str(), a.text.as_str()), ("assistant", "Привет, это начало"));
        assert!(a.error.as_deref().unwrap_or("").contains("поток оборвался"), "{:?}", a.error);
        assert!(app.chat.busy.lock().unwrap().is_empty(), "the chat is free after the turn");
        // an error object in the stream after some text behaves the same way
        let (app, _m, chat, msgs, dir2) = rig(vec![ok(vec![delta("Часть"), sse(json!({ "error": { "message": "overloaded" } })), delta("не должно попасть")])], &[("user", "Вопрос")]).await;
        turn(&app, &chat, &msgs).await;
        let a = stored(&app).pop().unwrap();
        assert_eq!((a.text.as_str(), a.error.as_deref()), ("Часть", Some("overloaded")));
        let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(dir2));
    }

    /// A character cut in two by a chunk boundary arrives whole.
    #[tokio::test]
    async fn characters_split_between_chunks() {
        let line = format!("data: {}\n\n", json!({ "choices": [{ "delta": { "content": "Привет, мир" } }] })).into_bytes();
        let cut = line.iter().position(|b| *b == 0xD0).unwrap() + 1;   // inside the first Cyrillic letter
        let (app, _m, chat, msgs, dir) = rig(vec![ok(vec![Act::Bytes(line[..cut].to_vec()), Act::Sleep(40), Act::Bytes(line[cut..].to_vec()), done()])], &[("user", "Вопрос")]).await;
        let ev = turn(&app, &chat, &msgs).await;
        assert_eq!(last_kind(&ev), "done");
        assert_eq!(stored(&app).pop().unwrap().text, "Привет, мир");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A stop ends the turn at once — while the provider is silent before the headers and in the middle of the stream;
    /// meanwhile the draft of the answer is on disk and out of the message list.
    #[tokio::test]
    async fn stop_is_immediate_and_the_draft_is_saved() {
        // silent provider: no headers for a minute
        let (app, _m, chat, msgs, dir) = rig(vec![Script { status: 200, wait: 60_000, acts: vec![] }], &[("user", "Вопрос")]).await;
        let (app2, chat2, msgs2) = (app.clone(), chat.clone(), msgs.clone());
        let h = tokio::spawn(async move { turn(&app2, &chat2, &msgs2).await });
        tokio::time::sleep(Duration::from_millis(300)).await;
        let t0 = Instant::now();
        app.chat.busy.lock().unwrap().get("ch_t").unwrap().stop.store(true, Ordering::Relaxed);
        let ev = h.await.unwrap();
        assert!(t0.elapsed() < Duration::from_secs(2), "stopped in {:?}", t0.elapsed());
        assert_eq!(last_kind(&ev), "done");
        let a = stored(&app).pop().unwrap();
        assert_eq!((a.text.as_str(), a.error.as_deref()), ("", Some(STOPPED)));
        let _ = std::fs::remove_dir_all(dir);

        // a stream that goes quiet after the first words
        let (app, _m, chat, msgs, dir) = rig(vec![ok(vec![delta("Начало ответа"), Act::Sleep(60_000), delta("конец")])], &[("user", "Вопрос")]).await;
        let (app2, chat2, msgs2) = (app.clone(), chat.clone(), msgs.clone());
        let h = tokio::spawn(async move { turn(&app2, &chat2, &msgs2).await });
        tokio::time::sleep(Duration::from_millis(3400)).await;
        // the draft: on disk with the "interrupted" mark, hidden from the chat while the answer is being written
        let draft = stored(&app).pop().unwrap();
        assert_eq!((draft.role.as_str(), draft.text.as_str(), draft.error.as_deref()), ("assistant", "Начало ответа", Some(INTERRUPTED)));
        let Ok(Json(view)) = get(State(app.clone()), Path("ch_t".to_string())).await else { panic!("the chat opens") };
        assert_eq!(view["busy"], json!(true));
        assert_eq!(view["partial"]["text"], json!("Начало ответа"));
        assert_eq!(view["messages"].as_array().unwrap().len(), 1, "only the question is a message so far");
        let t0 = Instant::now();
        assert!(matches!(stop(State(app.clone()), Path("ch_t".to_string())).await, Ok(Json(v)) if v["stopped"] == json!(true)));
        h.await.unwrap();
        assert!(t0.elapsed() < Duration::from_secs(2), "stopped in {:?}", t0.elapsed());
        let all = stored(&app);
        assert_eq!(all.len(), 2, "the draft became the message, not a second row");
        assert_eq!((all[1].id.as_str(), all[1].text.as_str(), all[1].error.as_deref()), (draft.id.as_str(), "Начало ответа", Some(STOPPED)));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Five rounds of tools in a row: the sixth call forbids tools and asks for the answer itself.
    #[tokio::test]
    async fn tool_rounds_end_with_an_answer() {
        let rounds = || (0..MAX_ROUNDS).map(|i| ok(vec![call(&format!("c{i}"), "http://127.0.0.1/inside"), done()])).collect::<Vec<_>>();
        let mut scripts = rounds(); scripts.push(ok(vec![delta("Итог по собранному"), done()]));
        let (app, mock, chat, msgs, dir) = rig(scripts, &[("user", "Исследуй")]).await;
        let ev = turn(&app, &chat, &msgs).await;
        assert_eq!(last_kind(&ev), "done");
        let a = stored(&app).pop().unwrap();
        assert_eq!((a.text.as_str(), a.error.as_deref(), a.tools.len()), ("Итог по собранному", None, MAX_ROUNDS));
        assert!(a.tools.iter().all(|t| !t.ok && t.result.contains("внутри сети")), "an inside address is refused by the page tool");
        let seen = mock.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), MAX_ROUNDS + 1);
        assert_eq!(seen[0]["tool_choice"], json!("auto"));
        assert_eq!(seen[MAX_ROUNDS]["tool_choice"], json!("none"));
        assert!(seen[MAX_ROUNDS]["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().contains("Лимит обращений"));
        let _ = std::fs::remove_dir_all(dir);
        // a model that asks for tools even then: the owner gets a clear error, not an empty answer
        let mut scripts = rounds(); scripts.push(ok(vec![call("c9", "http://127.0.0.1/again"), done()]));
        let (app, _m, chat, msgs, dir) = rig(scripts, &[("user", "Исследуй")]).await;
        let ev = turn(&app, &chat, &msgs).await;
        assert_eq!(last_kind(&ev), "error");
        assert!(stored(&app).pop().unwrap().error.unwrap().contains("не дала ответа"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Read the answer stream of the `send` handler to the end; an error of the handler — its status and text.
    async fn post(app: &Arc<App>, body: Value) -> Result<Vec<Value>, (StatusCode, String)> {
        match send(State(app.clone()), Path("ch_t".to_string()), Json(serde_json::from_value(body).unwrap())).await {
            Err(e) => Err((e.0, e.1)),
            Ok(sse) => {
                let bytes = axum::body::to_bytes(sse.into_response().into_body(), usize::MAX).await.unwrap();
                Ok(String::from_utf8_lossy(&bytes).lines().filter_map(|l| l.strip_prefix("data:")).filter_map(|d| serde_json::from_str(d.trim()).ok()).collect())
            }
        }
    }
    fn texts(app: &App) -> Vec<(String, String)> { stored(app).into_iter().map(|m| (m.role, m.text)).collect() }
    fn pair(r: &str, t: &str) -> (String, String) { (r.to_string(), t.to_string()) }

    /// "Retry" answers the last question again without repeating it; an edit replaces the message and everything after it.
    #[tokio::test]
    async fn regenerate_and_edit_cut_the_tail() {
        let (app, mock, _chat, _msgs, dir) = rig(vec![ok(vec![delta("Новый ответ"), done()]), ok(vec![delta("Ответ на правку"), done()]), ok(vec![delta("Ещё раз"), done()])], &[("user", "Вопрос один"), ("assistant", "Ответ один"), ("user", "Вопрос два"), ("assistant", "")]).await;
        // regenerate: the failed answer goes, the question stays where it was
        let ev = post(&app, json!({ "regenerate": true, "model": "m2" })).await.unwrap();
        assert!(ev.iter().all(|e| e["kind"] != json!("user")), "no new message of the owner");
        assert_eq!(last_kind(&ev), "done");
        assert_eq!(texts(&app), vec![pair("user", "Вопрос один"), pair("assistant", "Ответ один"), pair("user", "Вопрос два"), pair("assistant", "Новый ответ")]);
        assert_eq!(stored(&app)[2].id, "m3", "the question is the same message");
        assert_eq!(app.store.chat_get("ch_t").unwrap().unwrap().model, "m2", "retry with another model keeps that model");
        assert_eq!(mock.seen.lock().unwrap()[0]["messages"].as_array().unwrap().len(), 4, "system + three messages, the dropped answer is not sent");
        // edit of the first question: it and the whole tail are replaced
        let ev = post(&app, json!({ "text": "Вопрос один, точнее", "replace_from": "m1" })).await.unwrap();
        assert_eq!(ev[0]["kind"], json!("user"));
        assert_eq!(texts(&app), vec![pair("user", "Вопрос один, точнее"), pair("assistant", "Ответ на правку")]);
        // nothing to regenerate / nothing to edit
        assert_eq!(post(&app, json!({ "text": "x", "replace_from": "nope" })).await.unwrap_err().0, StatusCode::NOT_FOUND);
        assert_eq!(post(&app, json!({ "text": "" })).await.unwrap_err().0, StatusCode::BAD_REQUEST);
        // a bad attachment leaves the chat as it was and free for the next message
        assert_eq!(post(&app, json!({ "text": "с файлом", "replace_from": stored(&app)[0].id, "attachments": [{ "name": "a.txt", "data": "%%% not base64 %%%" }] })).await.unwrap_err().0, StatusCode::BAD_REQUEST);
        assert_eq!(texts(&app).len(), 2, "nothing was cut");
        assert!(app.chat.busy.lock().unwrap().is_empty(), "the chat is not left busy");
        assert_eq!(last_kind(&post(&app, json!({ "regenerate": true })).await.unwrap()), "done");
        assert_eq!(texts(&app), vec![pair("user", "Вопрос один, точнее"), pair("assistant", "Ещё раз")]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An edited message keeps the attachments left on it; the files of everything that was cut are removed.
    #[tokio::test]
    async fn edit_carries_attachments_over() {
        let (app, _mock, chat, _msgs, dir) = rig(vec![ok(vec![delta("Первый"), done()]), ok(vec![delta("Второй"), done()])], &[]).await;
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
        post(&app, json!({ "text": "смотри", "attachments": [{ "name": "a.txt", "data": b64("alpha") }, { "name": "b.txt", "data": b64("beta") }] })).await.unwrap();
        let first = stored(&app)[0].clone();
        let (keep, gone) = (first.attachments[0].file.clone(), first.attachments[1].file.clone());
        assert!(files_dir(&app, &chat.id).join(&keep).exists() && files_dir(&app, &chat.id).join(&gone).exists());
        post(&app, json!({ "text": "смотри только первый", "replace_from": first.id, "keep_files": [keep], "attachments": [{ "name": "c.txt", "data": b64("gamma") }] })).await.unwrap();
        let now_first = stored(&app)[0].clone();
        assert_eq!(now_first.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["a.txt", "c.txt"]);
        assert!(files_dir(&app, &chat.id).join(&keep).exists(), "the kept file stays");
        assert!(!files_dir(&app, &chat.id).join(&gone).exists(), "the dropped file is removed");
        assert_eq!(texts(&app), vec![pair("user", "смотри только первый"), pair("assistant", "Второй")]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A chat deleted while its answer is being written stays deleted.
    #[tokio::test]
    async fn deleted_chat_does_not_come_back() {
        let (app, _m, chat, msgs, dir) = rig(vec![ok(vec![delta("Пишу"), Act::Sleep(60_000)])], &[("user", "Вопрос")]).await;
        let (app2, chat2, msgs2) = (app.clone(), chat.clone(), msgs.clone());
        let h = tokio::spawn(async move { turn(&app2, &chat2, &msgs2).await });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(delete(State(app.clone()), Path("ch_t".to_string())).await.is_ok());
        let ev = tokio::time::timeout(Duration::from_secs(3), h).await.expect("the turn ends with the chat").unwrap();
        assert!(ev.iter().all(|e| e["kind"] != json!("done")), "nothing is reported as done");
        assert!(app.store.chat_get("ch_t").unwrap().is_none() && stored(&app).is_empty(), "the chat and its messages are gone");
        assert!(app.chat.busy.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
