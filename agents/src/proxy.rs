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

pub async fn core(State(app): State<Arc<App>>, Path(path): Path<String>, req: Request) -> Result<Response, ApiError> {
    let (mut parts, body) = req.into_parts();
    let query = parts.uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let is_ws = parts.headers.get(header::UPGRADE).and_then(|v| v.to_str().ok()).map(|v| v.eq_ignore_ascii_case("websocket")).unwrap_or(false);
    if is_ws {
        let ws = <WebSocketUpgrade as FromRequestParts<()>>::from_request_parts(&mut parts, &()).await.map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("websocket: {e}")))?;
        let url = format!("{}/api/{path}{query}", app.cloudd.replacen("http", "ws", 1));
        let (upstream, _) = tokio_tungstenite::connect_async(&url).await.map_err(|e| anyhow::anyhow!("ядро не отвечает: ws {e}"))?;
        return Ok(ws.on_upgrade(move |sock| relay(sock, upstream)));
    }
    let url = format!("{}/api/{path}{query}", app.cloudd);
    let bytes = axum::body::to_bytes(body, MAX_BODY).await.map_err(|e| ApiError(StatusCode::PAYLOAD_TOO_LARGE, format!("тело запроса: {e}")))?;
    let mut r = app.http.request(parts.method.clone(), &url);
    for (k, v) in parts.headers.iter() {
        if PASS_REQ.contains(&k.as_str()) { r = r.header(k, v); }
    }
    if !bytes.is_empty() { r = r.body(bytes); }
    let resp = r.send().await.map_err(|e| anyhow::anyhow!("ядро не отвечает: {e}"))?;
    let status = resp.status();
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
