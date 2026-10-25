//! share_target (docs/agents-ux.md §9.9): the phone shares text, a link or files with the "Agents" app. The manifest
//! sends a multipart POST to `/agents/share`; the payload lives in memory for 10 minutes and is picked up by the app
//! once (`GET /agents/api/share/{id}`), after which it goes into the draft of the open conversation. Nothing is
//! written to disk.
use crate::{ApiError, App};
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Limit on everything shared (files + text)
pub const MAX_SHARE: usize = 20 * 1024 * 1024;
const TTL_SECS: u64 = 600;

#[derive(Default)]
pub struct ShareState { items: Mutex<HashMap<String, (u64, Value)>> }

fn now_secs() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

impl ShareState {
    pub fn put(&self, v: Value) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let now = now_secs();
        let mut m = self.items.lock().unwrap();
        m.retain(|_, (at, _)| now.saturating_sub(*at) < TTL_SECS);
        if m.len() >= 16 { if let Some(oldest) = m.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone()) { m.remove(&oldest); } }
        m.insert(id.clone(), (now, v));
        id
    }
    pub fn take(&self, id: &str) -> Option<Value> {
        let now = now_secs();
        let mut m = self.items.lock().unwrap();
        m.retain(|_, (at, _)| now.saturating_sub(*at) < TTL_SECS);
        m.remove(id).map(|(_, v)| v)
    }
}

/// Building the payload from the share_target form fields: title, text, url,
/// files[]
pub fn payload(title: &str, text: &str, url: &str, files: Vec<(String, String, Vec<u8>)>) -> Value {
    let files: Vec<Value> = files.into_iter().map(|(name, ct, data)| json!({ "name": name, "type": ct, "size": data.len(), "data": base64::engine::general_purpose::STANDARD.encode(&data) })).collect();
    json!({ "title": title.trim(), "text": text.trim(), "url": url.trim(), "files": files, "at": now_secs() })
}

pub async fn receive(State(app): State<Arc<App>>, mut mp: Multipart) -> Result<Response, ApiError> {
    let (mut title, mut text, mut url) = (String::new(), String::new(), String::new());
    let mut files: Vec<(String, String, Vec<u8>)> = vec![];
    let mut total = 0usize;
    while let Some(field) = mp.next_field().await.map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("форма share: {e}")))? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "title" => title = field.text().await.unwrap_or_default(),
            "text" => text = field.text().await.unwrap_or_default(),
            "url" => url = field.text().await.unwrap_or_default(),
            "files" => {
                let fname = field.file_name().unwrap_or("file").to_string();
                let ct = field.content_type().unwrap_or("application/octet-stream").to_string();
                let data = field.bytes().await.map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("файл share: {e}")))?;
                total += data.len();
                if total > MAX_SHARE { return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "больше 20 МБ за раз не принимаем".into())); }
                if !data.is_empty() { files.push((fname, ct, data.to_vec())); }
            }
            _ => {}
        }
    }
    if title.is_empty() && text.is_empty() && url.is_empty() && files.is_empty() { return Ok(Redirect::to("/agents/").into_response()); }
    let id = app.share.put(payload(&title, &text, &url, files));
    Ok(Redirect::to(&format!("/agents/?share={id}")).into_response())
}

/// One-time handout: a second request — 404
pub async fn take(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    app.share.take(&id).map(Json).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "нечем поделиться: запись забрана или устарела".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn share_is_taken_once_and_payload_shape() {
        let st = ShareState::default();
        let id = st.put(payload(" заголовок ", "текст\n", "https://x", vec![("a.png".into(), "image/png".into(), vec![1, 2, 3])]));
        let v = st.take(&id).unwrap();
        assert_eq!(v["title"], "заголовок"); assert_eq!(v["text"], "текст"); assert_eq!(v["url"], "https://x");
        assert_eq!(v["files"][0]["name"], "a.png"); assert_eq!(v["files"][0]["size"], 3); assert_eq!(v["files"][0]["data"], "AQID");
        assert!(st.take(&id).is_none(), "второй раз — пусто");
        assert!(st.take("nope").is_none());
    }
}
