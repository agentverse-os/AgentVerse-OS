//! One-to-one proxy to the core API: `/agents/api/core/<path>` → `cloudd /api/<path>` with the same method, query, body and
//! main headers; WebSockets (session journals, terminals) are relayed in both directions. That way the "Agents" web app
//! works with the core through its own origin and port, while the core remains the sole owner of workspaces and agentd.
use crate::{App, ApiError};
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio_tungstenite::tungstenite::protocol::Message as TMsg;

const MAX_BODY: usize = 256 * 1024 * 1024;
const PASS_REQ: &[&str] = &["content-type", "accept", "x-desktop-presence", "content-disposition"];
const PASS_RESP: &[&str] = &["content-type", "content-disposition", "cache-control", "location"];

/// Where in the core a browser's request goes — or nothing, when the path is not for a browser.
///
/// The core has routes for the services of its own host (the secrets of the notification channels), and this service
/// reaches the core over loopback, where those routes live: without a check here the proxy would hand them to anyone
/// who can open the app. What is judged is the address after parsing — the very one that will be asked. A path that
/// reads one way as text and another way as an address must not pass by looking harmless: the parser drops tabs and line
/// breaks from inside a word (`in<tab>ternal` becomes `internal`), folds dot segments, and an encoded character is a
/// plain one to whoever decodes it again. Such a path is not passed on at all.
fn upstream(base: &str, path: &str, query: &str) -> Option<reqwest::Url> {
    let path = path.trim_start_matches('/');
    if path.chars().any(|c| c.is_control() || matches!(c, '\\' | '%' | '#' | '?')) || query.chars().any(|c| c.is_control() || c == '#') { return None }
    // no dot segments, and an empty segment only as a trailing slash
    let asked: Vec<&str> = path.split('/').collect();
    if path.is_empty() || asked.iter().enumerate().any(|(i, seg)| *seg == "." || *seg == ".." || (seg.is_empty() && i + 1 != asked.len())) { return None }
    let want = format!("/api/{path}");
    let (u, b) = (reqwest::Url::parse(&format!("{base}{want}{query}")).ok()?, reqwest::Url::parse(base).ok()?);
    if u.origin() != b.origin() { return None }
    // the parser has had its say: the path must still be the one that was asked for, segment by segment
    let got: Vec<&str> = u.path().split('/').collect();
    if got.len() != want.split('/').count() || got.get(1) != Some(&"api") { return None }
    if got.iter().skip(1).any(|seg| *seg == "." || *seg == ".." || seg.contains("%2e") || seg.contains("%2E") || seg.contains("%2f") || seg.contains("%2F") || seg.contains("%5c") || seg.contains("%5C")) { return None }
    if got.get(2).map(|seg| seg.eq_ignore_ascii_case("internal")).unwrap_or(true) { return None }
    Some(u)
}

pub async fn core(State(app): State<Arc<App>>, Path(path): Path<String>, req: Request) -> Result<Response, ApiError> {
    let (mut parts, body) = req.into_parts();
    let query = parts.uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let Some(url) = upstream(&app.cloudd, &path, &query) else { return Err(ApiError(StatusCode::NOT_FOUND, "нет такого адреса".into())) };
    let is_ws = parts.headers.get(header::UPGRADE).and_then(|v| v.to_str().ok()).map(|v| v.eq_ignore_ascii_case("websocket")).unwrap_or(false);
    if is_ws {
        let ws = <WebSocketUpgrade as FromRequestParts<()>>::from_request_parts(&mut parts, &()).await.map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("websocket: {e}")))?;
        let mut wurl = url.clone();
        let _ = wurl.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" });
        let (upstream, _) = tokio_tungstenite::connect_async(wurl.as_str()).await.map_err(|e| anyhow::anyhow!("ядро не отвечает: ws {e}"))?;
        return Ok(ws.on_upgrade(move |sock| relay(sock, upstream)));
    }
    let bytes = axum::body::to_bytes(body, MAX_BODY).await.map_err(|e| ApiError(StatusCode::PAYLOAD_TOO_LARGE, format!("тело запроса: {e}")))?;
    // a conversation that may hand in reports: the note about its box goes with the owner's next message (reports.rs)
    let told = if parts.method == axum::http::Method::POST { crate::reports::with_intro(&app, &path, &bytes) } else { None };
    let (bytes, told) = match told { Some((b, id)) => (axum::body::Bytes::from(b), Some(id)), None => (bytes, None) };
    // only the listed headers go on: what a browser sends besides them (and the key this service shows to the core's
    // internal routes) never reaches the core through here
    let mut r = app.http.request(parts.method.clone(), url);
    for (k, v) in parts.headers.iter() {
        if PASS_REQ.contains(&k.as_str()) { r = r.header(k, v); }
    }
    if !bytes.is_empty() { r = r.body(bytes); }
    let resp = r.send().await.map_err(|e| anyhow::anyhow!("ядро не отвечает: {e}"))?;
    let status = resp.status();
    if let (true, Some(id)) = (status.is_success(), told) { crate::reports::intro_delivered(&app, &id).await; }
    let mut headers = HeaderMap::new();
    for (k, v) in resp.headers().iter() {
        if PASS_RESP.contains(&k.as_str()) { headers.insert(k.clone(), v.clone()); }
    }
    let data = resp.bytes().await.map_err(|e| anyhow::anyhow!("ядро: {e}"))?;
    let mut out = Response::new(Body::from(data));
    *out.status_mut() = status;
    *out.headers_mut() = headers;
    out.headers_mut().entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static("no-store"));
    Ok(out)
}

/// Two-way relay of frames between the browser and the core; closing either side ends both.
async fn relay(client: WebSocket, upstream: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) {
    let (mut up_tx, mut up_rx) = upstream.split();
    let (mut cl_tx, mut cl_rx) = client.split();
    let to_core = async {
        while let Some(Ok(m)) = cl_rx.next().await {
            let out = match m {
                Message::Text(t) => TMsg::Text(t.as_str().into()),
                Message::Binary(b) => TMsg::Binary(b),
                Message::Ping(p) => TMsg::Ping(p),
                Message::Pong(p) => TMsg::Pong(p),
                Message::Close(_) => break,
            };
            if up_tx.send(out).await.is_err() { break; }
        }
    };
    let to_client = async {
        while let Some(Ok(m)) = up_rx.next().await {
            let out = match m {
                TMsg::Text(t) => Message::Text(t.as_str().into()),
                TMsg::Binary(b) => Message::Binary(b),
                TMsg::Ping(p) => Message::Ping(p),
                TMsg::Pong(p) => Message::Pong(p),
                TMsg::Close(_) => break,
                TMsg::Frame(_) => continue,
            };
            if cl_tx.send(out).await.is_err() { break; }
        }
    };
    tokio::select! { _ = to_core => {}, _ = to_client => {} }
    let _ = cl_tx.close().await;
}

#[allow(dead_code)]
fn _unused(_: &dyn IntoResponse) {}

#[cfg(test)]
mod tests {
    use super::upstream;
    const BASE: &str = "http://127.0.0.1:7100";

    /// The core's routes for local services never pass through the proxy, however the path is spelled — judged by the
    /// address that would really be asked, not by how the text looks.
    #[test]
    fn internal_routes_are_hidden_from_the_browser() {
        for p in ["internal/secrets/notify:ch_1/token", "/internal/secrets/notify:ch_1", "INTERNAL/secrets/x/y", "internal", "Internal/",
            // dot segments and doubled slashes, plain and encoded
            "projects/../internal/secrets/a/b", "projects/%2e%2e/internal/secrets/a/b", "projects//internal/x", "x/..%2finternal/secrets/a/b", "a\\..\\internal", "../internal/secrets/a/b", "projects/ws/..", "./internal/secrets/a/b",
            // what the address parser silently drops from inside a word: a tab, a line feed, a carriage return
            "in\ternal/secrets/notify:ch_1/token", "in\nternal/secrets/notify:ch_1/token", "inter\rnal/secrets/a/b", "x/.\t./internal/secrets/a/b", "\tinternal/secrets/a/b", "internal\t/secrets/a/b",
            // an encoded letter is a plain one to whoever decodes it again; a fragment or a second query would cut the path
            "%69nternal/secrets/a/b", "int%65rnal/secrets/a/b", "projects#/../internal/secrets/a/b", "projects?x=/../internal",
            // nothing at all, and a NUL
            "", "/", "internal\0/secrets/a/b"] {
            assert!(upstream(BASE, p, "").is_none(), "{p:?} must not pass");
        }
        for (p, q, want) in [("projects/ws/sessions/s1/prompt", "", "/api/projects/ws/sessions/s1/prompt"), ("status", "", "/api/status"), ("projects/ws/fs/file", "?path=%7E%2F.cloudos%2Fx%20y.md", "/api/projects/ws/fs/file"),
            ("agents/budgets/state", "", "/api/agents/budgets/state"), ("projects/internal-tools/sessions", "?all=1", "/api/projects/internal-tools/sessions"), ("updates/packages/agentverse-os-0.2.0-x86_64.tar.gz", "", "/api/updates/packages/agentverse-os-0.2.0-x86_64.tar.gz"),
            ("projects/ws/sessions/3349b1e1-b3f7-4883-ad1c-bca20eb68e5b/ws", "?since=12", "/api/projects/ws/sessions/3349b1e1-b3f7-4883-ad1c-bca20eb68e5b/ws"), ("hosts/мой сервер/probe", "", "/api/hosts/%D0%BC%D0%BE%D0%B9%20%D1%81%D0%B5%D1%80%D0%B2%D0%B5%D1%80/probe")] {
            let u = upstream(BASE, p, q).unwrap_or_else(|| panic!("{p:?} must pass"));
            assert_eq!(u.path(), want); assert_eq!(u.origin(), reqwest::Url::parse(BASE).unwrap().origin());
            assert_eq!(u.query().map(|x| format!("?{x}")).unwrap_or_default(), q);
        }
        // the query is the caller's own and may carry anything printable — but no control character and no fragment
        assert!(upstream(BASE, "status", "?a=1\n").is_none() && upstream(BASE, "status", "?a=1#x").is_none());
    }
}
