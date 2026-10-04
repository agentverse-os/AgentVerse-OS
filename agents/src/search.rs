//! Поиск по беседам и экспорт простого чата (срез 34, docs/agents-ux.md §4.6): слова запроса — все, по отдельности, без регистра
//! и ё/е; agentd запущенных workspace'ов опрашиваются параллельно (`GET /projects/{p}/search`), простые чаты — из SQLite
//! линейным проходом (объёмы позволяют, FTS не нужен). Ответ — одна строка на беседу с последним совпадением и их числом.
use crate::{App, ApiError};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

#[derive(Deserialize)]
pub struct SearchQuery { #[serde(default)] pub q: String, #[serde(default = "d_limit")] pub limit: usize }
fn d_limit() -> usize { 30 }

pub fn norm(s: &str) -> String { s.to_lowercase().replace('ё', "е") }

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b { b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char), _ => out.push_str(&format!("%{b:02X}")) }
    }
    out
}

/// Фрагмент ±80 знаков вокруг первого вхождения слова.
pub fn snippet(text: &str, word: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n: Vec<char> = norm(text).chars().collect();
    let w: Vec<char> = word.chars().collect();
    let pos = (0..n.len().saturating_sub(w.len()).saturating_add(1)).find(|&i| n[i..].starts_with(&w)).unwrap_or(0);
    let end_lim = chars.len().min(n.len());
    let start = pos.saturating_sub(80); let end = (pos + w.len() + 80).min(end_lim);
    let mut s: String = chars[start.min(end_lim)..end].iter().collect::<String>().replace('\n', " ");
    if start > 0 { s.insert(0, '…'); } if end < chars.len() { s.push('…'); }
    s
}

pub async fn search(State(app): State<Arc<App>>, Query(q): Query<SearchQuery>) -> Result<Json<Value>, ApiError> {
    let words: Vec<String> = norm(&q.q).split_whitespace().filter(|w| w.chars().count() >= 2).map(str::to_string).collect();
    if words.is_empty() { return Ok(Json(json!({ "q": q.q, "words": [], "hits": [], "stopped": [], "total": 0 }))) }
    let projects = app.core_get("/projects").await.unwrap_or(json!([]));
    let (mut running, mut stopped): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    for p in projects.as_array().cloned().unwrap_or_default() {
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        if name.is_empty() { continue }
        // статус workspace'а у ядра — внутри `workspace` ({status: running|stopped|…})
        if p.pointer("/workspace/status").and_then(|s| s.as_str()) == Some("running") { running.push(name) } else { stopped.push(name) }
    }
    let mut futs = Vec::new();
    for p in running {
        let app = app.clone(); let qs = q.q.clone();
        futs.push(tokio::spawn(async move { (p.clone(), tokio::time::timeout(Duration::from_secs(8), app.core_get(&format!("/projects/{p}/search?q={}&limit=50", encode(&qs)))).await) }));
    }
    let mut hits: Vec<Value> = Vec::new();
    for f in futs {
        let Ok((p, r)) = f.await else { continue };
        let Ok(Ok(v)) = r else { continue };
        for s in v.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default() {
            let hs = s.get("hits").and_then(|h| h.as_array()).cloned().unwrap_or_default();
            let Some(last) = hs.last() else { continue };
            let ts = last.get("ts").and_then(|t| t.as_u64()).or_else(|| s.get("updated_at").and_then(|u| u.as_u64()).map(|u| u * 1000)).unwrap_or(0);
            hits.push(json!({ "kind": "session", "project": p, "id": s.get("session"), "title": s.get("title"), "agent": s.get("agent"), "ts": ts, "seq": last.get("seq"), "role": last.get("role"), "snippet": last.get("snippet"), "count": hs.len() }));
        }
    }
    hits.extend(app.store.chat_search(&words, 500).unwrap_or_default());
    hits.sort_by(|a, b| b.get("ts").and_then(|t| t.as_u64()).cmp(&a.get("ts").and_then(|t| t.as_u64())));
    let total = hits.len(); hits.truncate(q.limit.clamp(1, 200));
    Ok(Json(json!({ "q": q.q, "words": words, "hits": hits, "stopped": stopped, "total": total })))
}

#[derive(Deserialize)]
pub struct ExportQuery { #[serde(default)] pub format: String }

fn fmt_ts(v: i64) -> String {
    let secs = if v > 10_000_000_000 { v / 1000 } else { v };
    chrono::DateTime::from_timestamp(secs, 0).map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

/// Экспорт простого чата: Markdown (по умолчанию) или JSON {chat, messages}.
pub async fn chat_export(State(app): State<Arc<App>>, Path(id): Path<String>, Query(q): Query<ExportQuery>) -> Result<Response, ApiError> {
    let (chat, msgs) = crate::simplechat::load(&app, &id).map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    let slug: String = chat.title.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect::<String>().split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let base = if slug.is_empty() { format!("chat-{}", id.trim_start_matches("ch_").chars().take(8).collect::<String>()) } else { slug.chars().take(60).collect() };
    if q.format == "json" {
        let body = serde_json::to_string_pretty(&json!({ "chat": chat, "messages": msgs })).unwrap_or_default();
        return Ok(([(header::CONTENT_TYPE, "application/json; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{base}.json\""))], body).into_response());
    }
    let mut out = format!("# {}\n\n- Простой чат · модель: {} · маршрут: {}\n- Создан: {} UTC · сообщений: {} · токены {}↑ {}↓ · ${:.2}\n\n---\n\n",
        if chat.title.trim().is_empty() { id.clone() } else { chat.title.clone() }, chat.model, chat.route, fmt_ts(chat.created_at), msgs.len(), chat.tokens_in, chat.tokens_out, chat.cost_usd);
    for m in &msgs {
        let who = if m.role == "user" { "Владелец" } else { "Модель" };
        out.push_str(&format!("**{who}** · {}\n\n", fmt_ts(m.created_at)));
        if !m.attachments.is_empty() { out.push_str(&format!("_Вложения: {}_\n\n", m.attachments.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", "))); }
        for tu in &m.tools {
            out.push_str(&format!("▸ **{}**: `{}`\n", tu.name, serde_json::to_string(&tu.args).unwrap_or_default().replace('`', "'").chars().take(300).collect::<String>()));
            let res: String = tu.result.chars().take(4000).collect();
            if !res.trim().is_empty() { out.push_str(&format!("<details><summary>{}</summary>\n\n```\n{}\n```\n\n</details>\n\n", if tu.ok { "результат" } else { "ошибка" }, res.trim_end())); }
        }
        if !m.text.trim().is_empty() { out.push_str(m.text.trim()); out.push_str("\n\n"); }
        if let Some(e) = &m.error { out.push_str(&format!("⚠ {e}\n\n")); }
    }
    Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{base}.md\""))], out).into_response())
}
