//! Простой чат (docs/agents-ux.md §6.1–6.3): беседы напрямую по OpenAI-совместимому API через маршруты ядра
//! (`/api/llm/routes/{id}/chat` — gate проекта, OpenRouter и другие ключи; ключи остаются в ядре), без контейнера.
//! Минимум инструментов: картинки и текстовые файлы на вход, `web_search` (SearXNG / Brave / Tavily из настроек) и
//! `fetch_url` (страница текстом). История — таблицы `chats`/`chat_messages` в SQLite сервиса, вложения — файлами в
//! каталоге данных; инкогнито-чаты живут только в памяти (TTL 2 ч, до 200 сообщений) и не пишутся на диск, в книгу расходов
//! ядра уходит лишь строка с деньгами. Ответ отдаётся потоком SSE: delta · tool · done · error.
use crate::{App, ApiError};
use anyhow::{anyhow, bail, Result};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use base64::Engine;
use futures_util::StreamExt;
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
const TEXT_ATTACH_MAX: usize = 200 * 1024;
/// Текст из PDF (срез 37): до 60 тысяч знаков на вложение
const PDF_TEXT_MAX: usize = 60_000;

/// Текстовый слой PDF при загрузке (`pdf-extract`); сканы без текста дают пустую строку.
fn pdf_text(name: &str, mime: &str, bytes: &[u8]) -> String {
    if mime != "application/pdf" && !name.to_lowercase().ends_with(".pdf") { return String::new() }
    match pdf_extract::extract_text_from_mem(bytes) {
        Ok(t) => { let t = t.split_whitespace().collect::<Vec<_>>().join(" "); if t.chars().count() > PDF_TEXT_MAX { t.chars().take(PDF_TEXT_MAX).collect() } else { t } }
        Err(e) => { tracing::debug!("pdf {name}: {e}"); String::new() }
    }
}

/// `/img описание` — картинка вместо ответа модели (срез 37).
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
pub struct Attachment { pub file: String, pub name: String, pub mime: String, pub size: u64, /// текстовый слой PDF (срез 37), у остальных пусто
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

/// Инкогнито-чат: всё в памяти, включая вложения.
pub struct Incog { pub chat: Chat, pub msgs: Vec<Message>, pub files: HashMap<String, (String, Vec<u8>)>, pub touched: i64 }

#[derive(Default)]
pub struct ChatState {
    pub incog: Mutex<HashMap<String, Incog>>,
    pub busy: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub models: Mutex<HashMap<String, (i64, Value)>>,
}

// ---------- хранение: SQLite для обычных, память для инкогнито ----------

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
    if chat.incognito {
        let mut m = app.chat.incog.lock().unwrap();
        match m.get_mut(&chat.id) { Some(e) => { e.chat = chat.clone(); e.touched = now(); }, None => { m.insert(chat.id.clone(), Incog { chat: chat.clone(), msgs: Vec::new(), files: HashMap::new(), touched: now() }); } }
        Ok(())
    } else { app.store.chat_put(chat) }
}

fn push_msg(app: &App, chat: &Chat, msg: &Message) -> Result<()> {
    if chat.incognito {
        let mut m = app.chat.incog.lock().unwrap();
        let e = m.get_mut(&chat.id).ok_or_else(|| anyhow!("инкогнито-чат исчез: {}", chat.id))?;
        e.msgs.push(msg.clone()); e.touched = now();
        if e.msgs.len() > INCOG_MAX_MSGS { let cut = e.msgs.len() - INCOG_MAX_MSGS; e.msgs.drain(..cut); }
        Ok(())
    } else { app.store.chat_msg_put(msg) }
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

/// Убрать инкогнито-чаты, к которым не обращались дольше TTL (тик планировщика).
pub fn sweep(app: &App) {
    let mut m = app.chat.incog.lock().unwrap();
    let cutoff = now() - INCOG_TTL;
    let dead: Vec<String> = m.iter().filter(|(_, e)| e.touched < cutoff).map(|(k, _)| k.clone()).collect();
    for k in dead { m.remove(&k); tracing::info!("инкогнито-чат {k} истёк"); }
}

// ---------- настройки и маршруты ----------

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

/// Цена модели, $ за 1M токенов (вход, выход): таблица `prices` сервиса, иначе цены маршрута (OpenRouter), иначе 0.
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

// ---------- инструменты ----------

fn tool_defs(tools: &Tools, search_ok: bool) -> Vec<Value> {
    let mut v = Vec::new();
    if tools.search && search_ok { v.push(json!({ "type": "function", "function": { "name": "web_search", "description": "Search the web. Use it for current events, facts you are not sure about, prices, documentation, news. Returns titles, URLs and snippets.", "parameters": { "type": "object", "properties": { "query": { "type": "string", "description": "search query" } }, "required": ["query"] } } })); }
    if tools.fetch { v.push(json!({ "type": "function", "function": { "name": "fetch_url", "description": "Fetch a web page or text file by URL and return its readable text (up to 20k characters). Use it to read a page the user gave or a search result.", "parameters": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] } } })); }
    v
}

fn url_allowed(url: &str) -> Result<()> {
    let u = url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) { bail!("только http(s)-адреса"); }
    let host = u.split("://").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("").split('@').last().unwrap_or("").split(':').next().unwrap_or("").to_lowercase();
    let private = host.is_empty() || host == "localhost" || host.ends_with(".localhost") || host.ends_with(".gate") || host.ends_with(".internal") || host.ends_with(".local")
        || host.starts_with("127.") || host.starts_with("10.") || host.starts_with("192.168.") || host == "0.0.0.0" || host == "::1" || host.starts_with("169.254.")
        || (host.starts_with("172.") && host.split('.').nth(1).and_then(|s| s.parse::<u8>().ok()).map(|n| (16..=31).contains(&n)).unwrap_or(false));
    if private { bail!("адрес внутри сети недоступен из чата"); }
    Ok(())
}

/// Текст страницы: без script/style/тегов, сжатые пробелы, заголовок из <title>.
pub fn html_to_text(html: &str) -> (String, String) {
    let lower = html.to_lowercase();
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

async fn fetch_url(app: &App, url: &str) -> Result<String> {
    url_allowed(url)?;
    let resp = app.http.get(url.trim()).header("User-Agent", "Mozilla/5.0 (compatible; AgentVerse chat)").header("Accept", "text/html, text/plain, application/json;q=0.9, */*;q=0.5").timeout(std::time::Duration::from_secs(15)).send().await.map_err(|e| anyhow!("не загрузилось: {e}"))?;
    let status = resp.status();
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await { let c = chunk?; buf.extend_from_slice(&c); if buf.len() >= FETCH_MAX_BYTES { break; } }
    if !status.is_success() { bail!("HTTP {status}"); }
    let body = String::from_utf8_lossy(&buf).to_string();
    let (title, text) = if ctype.contains("html") { html_to_text(&body) } else { (String::new(), body) };
    let text: String = text.chars().take(FETCH_MAX_CHARS).collect();
    Ok(format!("URL: {}\n{}\n{text}", url.trim(), if title.is_empty() { String::new() } else { format!("Title: {title}") }))
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
        "fetch_url" => fetch_url(app, args.get("url").and_then(|q| q.as_str()).unwrap_or("")).await,
        other => bail!("неизвестный инструмент {other}"),
    }
}

// ---------- сообщения для провайдера ----------

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

// ---------- генерация ответа: потоковый вызов маршрута ядра, инструменты, итог ----------

#[derive(Default)]
struct Turn { text: String, tool_calls: Vec<(String, String, String)>, usage_in: u64, usage_out: u64, finish: String }

/// Один вызов провайдера: чтение SSE (или JSON) → дельты в канал, вызовы инструментов и usage — в Turn.
async fn call_provider(app: &App, chat: &Chat, messages: &[Value], tools: &[Value], tx: &tokio::sync::mpsc::Sender<Value>, stop: &AtomicBool) -> Result<Turn> {
    let mut body = json!({ "model": chat.model, "messages": messages, "stream": true, "stream_options": { "include_usage": true } });
    if !tools.is_empty() { body["tools"] = json!(tools); body["tool_choice"] = json!("auto"); }
    let url = format!("{}/api/llm/routes/{}/chat", app.cloudd, chat.route);
    let resp = app.http.post(&url).json(&body).timeout(std::time::Duration::from_secs(600)).send().await.map_err(|e| anyhow!("ядро: {e}"))?;
    let status = resp.status();
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.pointer("/error/message").or_else(|| v.get("error")).and_then(|e| e.as_str().map(str::to_string).or_else(|| Some(e.to_string())))).unwrap_or_else(|| short(&text, 300));
        bail!("{} {}", status.as_u16(), msg);
    }
    let mut turn = Turn::default();
    let mut calls: Vec<(String, String, String)> = Vec::new(); // (id, name, arguments) по index
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
    if ctype.contains("application/json") {
        let v: Value = resp.json().await?;
        if let Some(p) = handle_chunk(&v, &mut turn, &mut calls) { let _ = tx.send(json!({ "kind": "delta", "text": p })).await; }
    } else {
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        'outer: while let Some(chunk) = stream.next().await {
            if stop.load(Ordering::Relaxed) { turn.finish = "stop".into(); break; }
            let chunk = chunk.map_err(|e| anyhow!("поток оборвался: {e}"))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim_end_matches('\r').to_string();
                buf.drain(..=pos);
                let Some(data) = line.strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" { break 'outer; }
                let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
                if let Some(e) = v.get("error") { bail!("{}", e.get("message").and_then(|m| m.as_str()).map(str::to_string).unwrap_or_else(|| e.to_string())); }
                if let Some(p) = handle_chunk(&v, &mut turn, &mut calls) { let _ = tx.send(json!({ "kind": "delta", "text": p })).await; }
            }
        }
    }
    turn.tool_calls = calls.into_iter().filter(|(_, n, _)| !n.is_empty()).collect();
    Ok(turn)
}

async fn generate(app: Arc<App>, mut chat: Chat, mut msgs: Vec<Message>, tx: tokio::sync::mpsc::Sender<Value>, stop: Arc<AtomicBool>) {
    let started = std::time::Instant::now();
    let search_ok = search_ready(&app);
    let mut tools = tool_defs(&chat.tools, search_ok);
    let mut messages = provider_messages(&app, &chat, &msgs);
    let mut text = String::new();
    let mut uses: Vec<ToolUse> = Vec::new();
    let (mut tin, mut tout) = (0u64, 0u64);
    let mut error: Option<String> = None;
    for round in 0..MAX_ROUNDS {
        let turn = match call_provider(&app, &chat, &messages, &tools, &tx, &stop).await {
            Ok(t) => t,
            Err(e) => {
                let es = e.to_string();
                // модель или маршрут без tool calling: повторить без инструментов
                if !tools.is_empty() && round == 0 && (es.contains("tool") || es.contains("function") || es.starts_with("400")) { tracing::info!("чат {}: без инструментов после ошибки: {es}", chat.id); tools.clear(); match call_provider(&app, &chat, &messages, &tools, &tx, &stop).await { Ok(t) => t, Err(e2) => { error = Some(e2.to_string()); break; } } }
                else { error = Some(es); break; }
            }
        };
        tin += turn.usage_in; tout += turn.usage_out;
        text.push_str(&turn.text);
        if turn.tool_calls.is_empty() || stop.load(Ordering::Relaxed) { break; }
        // вызовы инструментов: ответ провайдера с tool_calls → результаты → следующий круг
        let calls_json: Vec<Value> = turn.tool_calls.iter().map(|(id, n, a)| json!({ "id": id, "type": "function", "function": { "name": n, "arguments": a } })).collect();
        messages.push(json!({ "role": "assistant", "content": if turn.text.is_empty() { Value::Null } else { json!(turn.text) }, "tool_calls": calls_json }));
        for (id, name, args_s) in &turn.tool_calls {
            let args: Value = serde_json::from_str(args_s).unwrap_or(json!({}));
            let _ = tx.send(json!({ "kind": "tool_start", "name": name, "args": args })).await;
            let (result, ok) = match run_tool(&app, name, &args).await { Ok(r) => (r, true), Err(e) => (format!("Ошибка: {e:#}"), false) };
            let _ = tx.send(json!({ "kind": "tool", "name": name, "args": args, "result": short(&result, 1500), "ok": ok })).await;
            messages.push(json!({ "role": "tool", "tool_call_id": id, "content": result }));
            uses.push(ToolUse { name: name.clone(), args, result: short(&result, 4000), ok });
        }
        if !turn.text.is_empty() { text.push('\n'); }
    }
    if tin == 0 && tout == 0 { tin = (messages.iter().map(|m| m.to_string().len()).sum::<usize>() / 4) as u64; tout = (text.len() / 4) as u64; }
    let (pi, po, priced) = price_for(&app, &chat.route, &chat.model).await;
    let cost = (tin as f64) * pi / 1e6 + (tout as f64) * po / 1e6;
    let seq = msgs.last().map(|m| m.seq + 1).unwrap_or(1);
    let msg = Message { id: new_id("m"), chat: chat.id.clone(), seq, role: "assistant".into(), text: text.clone(), attachments: Vec::new(), tools: uses, tokens_in: tin, tokens_out: tout, cost_usd: cost, created_at: now(), error: error.clone() };
    if let Err(e) = push_msg(&app, &chat, &msg) { tracing::warn!("чат {}: сообщение не записалось: {e:#}", chat.id); }
    msgs.push(msg.clone());
    chat.cost_usd += cost; chat.tokens_in += tin; chat.tokens_out += tout; chat.updated_at = now(); chat.messages = msgs.len() as u64;
    if chat.title.trim().is_empty() { if let Some(u) = msgs.iter().find(|m| m.role == "user") { let t: String = u.text.split_whitespace().collect::<Vec<_>>().join(" "); chat.title = short(if t.is_empty() { "Вложение" } else { &t }, TITLE_MAX); } }
    let _ = save_chat(&app, &chat);
    // книга расходов ядра: только цифры, текста там нет и у инкогнито
    let rec = json!({ "project": PROJECT, "session": chat.id, "seq": seq, "agent": "api", "model": chat.model, "input": tin, "output": tout, "cost_usd": cost, "duration_ms": started.elapsed().as_millis() as u64, "source": if priced { "api" } else { "api-unpriced" } });
    if let Err(e) = app.core_post("/usage/record", &rec).await { tracing::debug!("usage/record: {e:#}"); }
    match &error {
        Some(e) => { let _ = tx.send(json!({ "kind": "error", "error": e, "message": msg, "chat": chat })).await; }
        None => { let _ = tx.send(json!({ "kind": "done", "message": msg, "chat": chat })).await; }
    }
    app.chat.busy.lock().unwrap().remove(&chat.id);
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
        // умолчаний нет: первый маршрут и первая его модель
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
    let (chat, msgs) = load(&app, &id).map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    let busy = app.chat.busy.lock().unwrap().contains_key(&id);
    Ok(Json(json!({ "chat": with_count(&app, &chat), "messages": msgs, "busy": busy })))
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
    if app.chat.incog.lock().unwrap().remove(&id).is_some() { return Ok(StatusCode::NO_CONTENT); }
    if !app.store.chat_delete(&id)? { return Err(ApiError(StatusCode::NOT_FOUND, "чат не найден".into())); }
    let _ = std::fs::remove_dir_all(files_dir(&app, &id));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AttachIn { pub name: String, #[serde(default)] pub mime: String, pub data: String }
#[derive(Deserialize)]
pub struct SendBody { #[serde(default)] pub text: String, #[serde(default)] pub attachments: Vec<AttachIn>, #[serde(default)] pub model: Option<String>, #[serde(default)] pub tools: Option<Tools> }

/// Сообщение пользователя → поток SSE с ответом (delta, tool_start, tool, done | error).
pub async fn send(State(app): State<Arc<App>>, Path(id): Path<String>, Json(b): Json<SendBody>) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let (mut chat, mut msgs) = load(&app, &id).map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    if b.text.trim().is_empty() && b.attachments.is_empty() { return Err(ApiError(StatusCode::BAD_REQUEST, "пустое сообщение".into())); }
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut busy = app.chat.busy.lock().unwrap();
        if busy.contains_key(&id) { return Err(ApiError(StatusCode::CONFLICT, "ответ ещё пишется — дождитесь или остановите".into())); }
        busy.insert(id.clone(), stop.clone());
    }
    if let Some(m) = b.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) { chat.model = m.to_string(); }
    if let Some(t) = b.tools { chat.tools = t; }
    let mut attachments = Vec::new();
    let mut total = 0usize;
    for a in &b.attachments {
        let bytes = base64::engine::general_purpose::STANDARD.decode(a.data.trim()).map_err(|_| ApiError(StatusCode::BAD_REQUEST, format!("вложение {} не в base64", a.name)))?;
        total += bytes.len();
        if total > ATTACH_MAX { app.chat.busy.lock().unwrap().remove(&id); return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "вложения больше 8 МБ".into())); }
        let mime = if a.mime.trim().is_empty() { mime_guess::from_path(&a.name).first_or_octet_stream().to_string() } else { a.mime.trim().to_string() };
        let ext = std::path::Path::new(&a.name).extension().and_then(|e| e.to_str()).unwrap_or("bin").to_lowercase();
        let file = format!("{}.{}", new_id("f"), ext.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect::<String>());
        if let Err(e) = file_put(&app, &chat, &file, &mime, &bytes) { app.chat.busy.lock().unwrap().remove(&id); return Err(e.into()); }
        let text = pdf_text(&a.name, &mime, &bytes);
        attachments.push(Attachment { file, name: short(&a.name, 120), mime, size: bytes.len() as u64, text });
    }
    let seq = msgs.last().map(|m| m.seq + 1).unwrap_or(1);
    let user = Message { id: new_id("m"), chat: chat.id.clone(), seq, role: "user".into(), text: b.text.trim().to_string(), attachments, created_at: now(), ..Default::default() };
    if let Err(e) = push_msg(&app, &chat, &user) { app.chat.busy.lock().unwrap().remove(&id); return Err(e.into()); }
    msgs.push(user.clone());
    chat.updated_at = now(); chat.messages = msgs.len() as u64;
    let _ = save_chat(&app, &chat);
    let (tx, rx) = tokio::sync::mpsc::channel::<Value>(64);
    let _ = tx.send(json!({ "kind": "user", "message": user.clone() })).await;
    if let Some(prompt) = img_prompt(&user.text) { tokio::spawn(generate_image(app.clone(), chat, msgs, prompt, tx)); }
    else { tokio::spawn(generate(app.clone(), chat, msgs, tx, stop)); }
    let stream = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|v| (Ok::<Event, std::convert::Infallible>(Event::default().data(v.to_string())), rx)) });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)).text("ping")))
}

/// Модель для картинок: из настроек чата (`image_model`), иначе первая `gpt-image-*` маршрута (gate принимает на
/// `/images/generations` только их), иначе первая модель вида image.
async fn pick_image_model(app: &App, route: &str) -> Option<String> {
    let cfg = chat_settings(app).get("image_model").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    if cfg.is_some() { return cfg }
    let v = route_models(app, route).await.ok()?;
    let list: Vec<&Value> = v.get("models").and_then(|m| m.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
    let images: Vec<String> = list.iter().filter(|m| m.get("kind").and_then(|k| k.as_str()) == Some("image")).filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect();
    images.iter().find(|i| i.starts_with("gpt-image")).or_else(|| images.first()).cloned()
}

/// `/img`: картинка через ядро (`POST /llm/routes/{id}/images`), файл — во вложение сообщения модели; стоимость в книгу не идёт.
async fn generate_image(app: Arc<App>, mut chat: Chat, msgs: Vec<Message>, prompt: String, tx: tokio::sync::mpsc::Sender<Value>) {
    let fail = |m: String| json!({ "kind": "error", "error": m, "message": Value::Null, "chat": Value::Null });
    let Some(model) = pick_image_model(&app, &chat.route).await else {
        let _ = tx.send(fail("на маршруте нет моделей для картинок (gpt-image-*) — задайте image_model в настройках чата".into())).await;
        app.chat.busy.lock().unwrap().remove(&chat.id); return;
    };
    let _ = tx.send(json!({ "kind": "tool_start", "name": "image", "args": { "prompt": prompt, "model": model } })).await;
    let res = app.core_post(&format!("/llm/routes/{}/images", chat.route), &json!({ "model": model, "prompt": prompt, "n": 1, "size": "1024x1024" })).await;
    let v = match res { Ok(v) => v, Err(e) => { let _ = tx.send(fail(format!("картинка: {e:#}"))).await; app.chat.busy.lock().unwrap().remove(&chat.id); return; } };
    let item = v.pointer("/data/0").cloned().unwrap_or(Value::Null);
    let bytes: Option<Vec<u8>> = if let Some(b) = item.get("b64_json").and_then(|x| x.as_str()) { base64::engine::general_purpose::STANDARD.decode(b.trim()).ok() }
        else if let Some(u) = item.get("url").and_then(|x| x.as_str()) {
            match app.http.get(u).timeout(std::time::Duration::from_secs(120)).send().await { Ok(r) if r.status().is_success() => r.bytes().await.ok().map(|b| b.to_vec()), _ => None }
        } else { None };
    let Some(bytes) = bytes else { let _ = tx.send(fail(format!("картинка: провайдер не вернул данных ({})", short(&v.to_string(), 200)))).await; app.chat.busy.lock().unwrap().remove(&chat.id); return; };
    let mime = if bytes.starts_with(b"\x89PNG") { "image/png" } else if bytes.starts_with(&[0xFF, 0xD8]) { "image/jpeg" } else if bytes.starts_with(b"RIFF") { "image/webp" } else { "image/png" };
    let file = format!("{}.{}", new_id("f"), if mime == "image/jpeg" { "jpg" } else if mime == "image/webp" { "webp" } else { "png" });
    if let Err(e) = file_put(&app, &chat, &file, mime, &bytes) { let _ = tx.send(fail(format!("картинка: {e:#}"))).await; app.chat.busy.lock().unwrap().remove(&chat.id); return; }
    let _ = tx.send(json!({ "kind": "tool", "name": "image", "args": { "prompt": prompt, "model": model }, "result": format!("{} байт, {mime}", bytes.len()), "ok": true })).await;
    let seq = msgs.last().map(|m| m.seq + 1).unwrap_or(1);
    let msg = Message { id: new_id("m"), chat: chat.id.clone(), seq, role: "assistant".into(), text: format!("Картинка: {prompt} · {model}"), attachments: vec![Attachment { file, name: "image.png".into(), mime: mime.into(), size: bytes.len() as u64, text: String::new() }], created_at: now(), ..Default::default() };
    if let Err(e) = push_msg(&app, &chat, &msg) { let _ = tx.send(fail(format!("картинка: {e:#}"))).await; app.chat.busy.lock().unwrap().remove(&chat.id); return; }
    chat.updated_at = now(); chat.messages = (msgs.len() + 1) as u64;
    if chat.title.trim().is_empty() { chat.title = short(&format!("Картинка: {prompt}"), 60); }
    let _ = save_chat(&app, &chat);
    app.chat.busy.lock().unwrap().remove(&chat.id);
    let _ = tx.send(json!({ "kind": "done", "message": msg, "chat": with_count(&app, &chat) })).await;
}

/// Пресеты системного промпта (срез 37): kv `chat.presets`, по умолчанию три заготовки.
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
    let stopped = app.chat.busy.lock().unwrap().get(&id).map(|s| { s.store(true, Ordering::Relaxed); true }).unwrap_or(false);
    Ok(Json(json!({ "stopped": stopped })))
}

/// Инкогнито → в историю: чат, сообщения и вложения переезжают на диск.
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

/// Открыть в продвинутом чате: сессия Claude Code / Codex в scratch с историей первым сообщением.
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

/// Маршруты и их модели (через ядро, кэш 10 мин) + умолчания чата и готовность поиска.
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
    fn html_and_urls() {
        let (t, x) = html_to_text("<html><head><title>Hi &amp; bye</title><style>p{}</style></head><body><script>var a=1;</script><p>One <b>two</b></p><div>three</div><!-- c --></body></html>");
        assert_eq!(t, "Hi & bye"); assert_eq!(x, "One two\nthree\n");
        assert!(url_allowed("https://example.com/x").is_ok() && url_allowed("http://10.77.1.2/").is_err() && url_allowed("http://llm.gate/v1").is_err() && url_allowed("ftp://x").is_err() && url_allowed("http://172.20.0.1/").is_err() && url_allowed("http://172.40.0.1/").is_ok());
        let d = tool_defs(&Tools { search: true, fetch: true }, false);
        assert_eq!(d.len(), 1); assert_eq!(d[0].pointer("/function/name").and_then(|n| n.as_str()), Some("fetch_url"));
        let c = Chat { system: String::new(), ..Default::default() };
        assert!(system_prompt(&c).contains("Сегодня"));
    }
}
