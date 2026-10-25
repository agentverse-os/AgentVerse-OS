//! Agent browser (docs/agents-ux.md §4.9, step 2): one headless Chromium per workspace with CDP on 127.0.0.1:9222. The agent works
//! with it through Playwright MCP (`--cdp-endpoint`), the human through `GET /v1/browser/ws`: screencast (`Page.startScreencast`)
//! and input (`Input.dispatch*`), address, tabs, window size. Off by default: `enable` starts the browser and adds the `browser`
//! MCP server to the shared workspace list (mcp.rs), `disable` removes both.
use crate::mcp;
use anyhow::{anyhow, bail, Context, Result};
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message as WsMsg;

pub const PORT: u16 = 9222;
pub const MCP_NAME: &str = "browser";

#[derive(Default)]
pub struct Browser { child: Mutex<Option<Child>> }

fn home() -> String { std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into()) }

/// Chromium executable: the Playwright cache (newest build), otherwise the system
/// one
pub fn chrome_bin() -> Option<PathBuf> {
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(format!("{}/.cache/ms-playwright", home())) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("chromium-") || name.contains("headless_shell") { continue }
            for sub in ["chrome-linux64/chrome", "chrome-linux/chrome"] {
                let p = e.path().join(sub);
                if p.is_file() { found.push((name.clone(), p)); break }
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    if let Some((_, p)) = found.into_iter().next() { return Some(p) }
    for c in ["chromium", "chromium-browser", "google-chrome", "google-chrome-stable"] {
        if let Ok(out) = Command::new("which").arg(c).output() { if out.status.success() { return Some(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())) } }
    }
    None
}

/// Chromium install command (terminal for the user): the browser into the Playwright cache plus system libraries
/// via sudo
pub fn install_cmd() -> Vec<String> {
    vec!["sh".into(), "-lc".into(), "npx -y playwright@1.55.0 install chromium && (sudo -n npx -y playwright@1.55.0 install-deps chromium || echo 'библиотеки: sudo недоступен, пропускаю')".into()]
}

/// Raw GET to CDP (http://127.0.0.1:9222/json…): no HTTP client in agentd — a short request over TCP
async fn cdp_http(path: &str, method: &str) -> Result<Value> {
    let mut s = tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(("127.0.0.1", PORT))).await.context("cdp connect")??;
    s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\nConnection: close\r\n\r\n").as_bytes()).await?;
    // Chrome's DevTools server does not close the connection on Connection: close — read the headers, then exactly
    // Content-Length bytes
    let mut buf = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let (head_end, content_len) = loop {
        let mut chunk = [0u8; 4096];
        let n = tokio::time::timeout_at(deadline, s.read(&mut chunk)).await.context("cdp read")??;
        if n == 0 { break (buf.len(), 0usize) }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..i]).to_string();
            let len = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
            break (i + 4, len)
        }
    };
    while buf.len() < head_end + content_len {
        let mut chunk = [0u8; 4096];
        let n = tokio::time::timeout_at(deadline, s.read(&mut chunk)).await.context("cdp read")??;
        if n == 0 { break }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = String::from_utf8_lossy(&buf[head_end.min(buf.len())..]).to_string();
    Ok(serde_json::from_str(body.trim()).unwrap_or(json!(null)))
}

pub async fn running() -> bool { cdp_http("/json/version", "GET").await.map(|v| v.get("Browser").is_some()).unwrap_or(false) }

/// Tabs (pages only): id, url, title
pub async fn pages() -> Vec<Value> {
    cdp_http("/json/list", "GET").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default().into_iter()
        .filter(|t| t.get("type").and_then(|x| x.as_str()) == Some("page"))
        .map(|t| json!({ "id": t.get("id"), "url": t.get("url"), "title": t.get("title") })).collect()
}

pub async fn status(b: &Browser) -> Value {
    let bin = chrome_bin();
    let running = running().await;
    let pages = if running { pages().await } else { Vec::new() };
    let own = b.child.lock().unwrap().is_some();
    json!({ "installed": bin.is_some(), "bin": bin.as_ref().map(|p| p.display().to_string()), "running": running, "own": own, "port": PORT,
            "mcp": mcp::servers().iter().any(|s| s.name == MCP_NAME), "pages": pages })
}

/// Start headless Chromium with CDP unless it is already running; wait for
/// readiness up to 10 s
pub async fn start(b: &Browser) -> Result<()> {
    if running().await { return Ok(()) }
    let bin = chrome_bin().ok_or_else(|| anyhow!("Chromium не установлен"))?;
    let profile = format!("{}/.cloudos/browser", home());
    let _ = std::fs::create_dir_all(&profile);
    let child = Command::new(&bin)
        .args(["--headless=new", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage", "--no-first-run", "--no-default-browser-check", "--disable-background-networking",
               &format!("--remote-debugging-port={PORT}"), "--remote-allow-origins=*", &format!("--user-data-dir={profile}"), "--window-size=1280,800", "about:blank"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().with_context(|| format!("запуск {}", bin.display()))?;
    *b.child.lock().unwrap() = Some(child);
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if running().await { tracing::info!("браузер агента запущен: {}", bin.display()); return Ok(()) }
    }
    bail!("Chromium не ответил на CDP за 10 с")
}

pub fn stop(b: &Browser) {
    if let Some(mut c) = b.child.lock().unwrap().take() { let _ = c.kill(); let _ = c.wait(); }
    // a foreign instance on our port (after an agentd restart) — found by pid from the
    // command line
    let _ = Command::new("pkill").args(["-f", &format!("--remote-debugging-port={PORT}")]).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn mcp_server() -> mcp::McpServer {
    mcp::McpServer { name: MCP_NAME.into(), transport: "stdio".into(), command: "npx".into(),
        args: vec!["-y".into(), "@playwright/mcp@latest".into(), "--cdp-endpoint".into(), format!("http://127.0.0.1:{PORT}"), "--caps".into(), "vision".into()], ..Default::default() }
}

/// Enable: the browser is running and the `browser` MCP server is in the workspace list (rolled out
/// to the CLI engines)
pub async fn enable(b: &Browser) -> Result<Value> {
    start(b).await?;
    let mut list = mcp::servers();
    if !list.iter().any(|s| s.name == MCP_NAME) {
        list.push(mcp_server());
        mcp::save(list).await.map_err(|e| anyhow!("MCP: {e}"))?;
    }
    crate::hub::touch();
    Ok(status(b).await)
}
pub async fn disable(b: &Browser) -> Result<Value> {
    let list: Vec<_> = mcp::servers().into_iter().filter(|s| s.name != MCP_NAME).collect();
    if list.len() != mcp::count() { mcp::save(list).await.map_err(|e| anyhow!("MCP: {e}"))?; }
    stop(b);
    crate::hub::touch();
    Ok(status(b).await)
}

/// CDP session of a tab: writes through a channel, answers by id into waiting oneshots, events into the
/// client channel
struct Cdp {
    tx: tokio::sync::mpsc::Sender<String>,
    pending: std::sync::Arc<Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Value>>>>,
    seq: std::sync::atomic::AtomicU64,
}
impl Cdp {
    async fn fire(&self, method: &str, params: Value) { let id = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1; let _ = self.tx.send(json!({ "id": id, "method": method, "params": params }).to_string()).await; }
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let (otx, orx) = tokio::sync::oneshot::channel();
        self.pending.lock().unwrap().insert(id, otx);
        self.tx.send(json!({ "id": id, "method": method, "params": params }).to_string()).await.map_err(|_| anyhow!("CDP закрыт"))?;
        let v = tokio::time::timeout(Duration::from_secs(15), orx).await.map_err(|_| anyhow!("CDP {method}: таймаут"))?.map_err(|_| anyhow!("CDP {method}: без ответа"))?;
        if let Some(e) = v.get("error") { bail!("CDP {method}: {}", e.get("message").and_then(|m| m.as_str()).unwrap_or("ошибка")) }
        Ok(v.get("result").cloned().unwrap_or(json!({})))
    }
}

/// Tab screencast and input: client ↔ agentd ↔ page CDP session. Client messages: nav/back/forward/reload/mouse/key/text/
/// viewport/newtab/close/tabs/design/hover/inspect; to the client: frame (JPEG base64), nav (url), tabs, element (Design Mode),
/// error.
pub async fn serve(mut client: WebSocket, target: Option<String>) {
    let id = match pick_target(target).await { Ok(id) => id, Err(e) => { let _ = client.send(Message::Text(json!({ "kind": "error", "error": e.to_string() }).to_string().into())).await; return } };
    let url = format!("ws://127.0.0.1:{PORT}/devtools/page/{id}");
    let (cdp_ws, _) = match tokio_tungstenite::connect_async(&url).await { Ok(c) => c, Err(e) => { let _ = client.send(Message::Text(json!({ "kind": "error", "error": format!("CDP: {e}") }).to_string().into())).await; return } };
    let (mut cdp_sink, mut cdp_stream) = cdp_ws.split();
    let (wtx, mut wrx) = tokio::sync::mpsc::channel::<String>(256);
    let (ctx, mut crx) = tokio::sync::mpsc::channel::<Value>(64);
    let cdp = std::sync::Arc::new(Cdp { tx: wtx, pending: Default::default(), seq: Default::default() });
    // CDP writer
    let writer = tokio::spawn(async move { while let Some(m) = wrx.recv().await { if cdp_sink.send(WsMsg::Text(m.into())).await.is_err() { break } } let _ = cdp_sink.send(WsMsg::Close(None)).await; });
    // CDP reader: answers → waiters, frames and navigation → client
    let reader = { let cdp = cdp.clone(); let ctx = ctx.clone(); tokio::spawn(async move {
        while let Some(Ok(WsMsg::Text(t))) = cdp_stream.next().await {
            let v: Value = match serde_json::from_str(&t) { Ok(v) => v, Err(_) => continue };
            if let Some(rid) = v.get("id").and_then(|i| i.as_u64()) { if let Some(tx) = cdp.pending.lock().unwrap().remove(&rid) { let _ = tx.send(v); } continue }
            match v.get("method").and_then(|m| m.as_str()) {
                Some("Page.screencastFrame") => {
                    let p = &v["params"]; let meta = &p["metadata"];
                    cdp.fire("Page.screencastFrameAck", json!({ "sessionId": p.get("sessionId").cloned().unwrap_or(json!(0)) })).await;
                    if ctx.send(json!({ "kind": "frame", "data": p.get("data"), "w": meta.get("deviceWidth"), "h": meta.get("deviceHeight"), "ts": meta.get("timestamp") })).await.is_err() { break }
                }
                Some("Page.frameNavigated") => { let f = &v["params"]["frame"]; if f.get("parentId").is_none() { let _ = ctx.send(json!({ "kind": "nav", "url": f.get("url") })).await; } }
                _ => {}
            }
        }
        let _ = ctx.send(json!({ "kind": "error", "error": "CDP закрыт" })).await;
    }) };
    let (mut vw, mut vh) = (1024u64, 700u64);
    let screencast = |cdp: std::sync::Arc<Cdp>, w: u64, h: u64| async move { cdp.fire("Page.startScreencast", json!({ "format": "jpeg", "quality": 60, "maxWidth": w, "maxHeight": h, "everyNthFrame": 1 })).await };
    cdp.fire("Page.enable", json!({})).await;
    screencast(cdp.clone(), vw, vh).await;
    let _ = client.send(Message::Text(json!({ "kind": "tabs", "active": id, "list": pages().await }).to_string().into())).await;
    let mut tick = tokio::time::interval(Duration::from_secs(3));
    let mut design = false;
    loop {
        tokio::select! {
            m = crx.recv() => {
                let Some(m) = m else { break };
                let is_err = m.get("kind").and_then(|k| k.as_str()) == Some("error");
                if client.send(Message::Text(m.to_string().into())).await.is_err() || is_err { break }
            }
            m = client.recv() => {
                let Some(Ok(Message::Text(t))) = m else { break };
                let v: Value = match serde_json::from_str(&t) { Ok(v) => v, Err(_) => continue };
                match v.get("kind").and_then(|k| k.as_str()) {
                    Some("nav") => {
                        let u = v.get("url").and_then(|u| u.as_str()).unwrap_or("about:blank").trim().to_string();
                        let has_scheme = u.split(':').next().map(|sch| !sch.is_empty() && sch.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) && u.len() > sch.len() + 1).unwrap_or(false) && !u.starts_with("localhost:") && !u.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
                        let u = if has_scheme { u } else { format!("http://{u}") };
                        cdp.fire("Page.navigate", json!({ "url": u })).await;
                    }
                    Some("reload") => { cdp.fire("Page.reload", json!({})).await; }
                    Some("back") | Some("forward") => { let js = if v["kind"] == "back" { "history.back()" } else { "history.forward()" }; cdp.fire("Runtime.evaluate", json!({ "expression": js })).await; }
                    Some("mouse") => {
                        let mut params = json!({ "type": v.get("type").cloned().unwrap_or(json!("mouseMoved")), "x": v.get("x").cloned().unwrap_or(json!(0)), "y": v.get("y").cloned().unwrap_or(json!(0)), "button": v.get("button").cloned().unwrap_or(json!("none")), "clickCount": v.get("clickCount").cloned().unwrap_or(json!(0)), "modifiers": v.get("modifiers").cloned().unwrap_or(json!(0)) });
                        if let Some(dx) = v.get("deltaX") { params["deltaX"] = dx.clone(); }
                        if let Some(dy) = v.get("deltaY") { params["deltaY"] = dy.clone(); }
                        cdp.fire("Input.dispatchMouseEvent", params).await;
                    }
                    Some("key") => {
                        let mut params = json!({ "type": v.get("type").cloned().unwrap_or(json!("keyDown")), "key": v.get("key").cloned().unwrap_or(json!("")), "code": v.get("code").cloned().unwrap_or(json!("")), "modifiers": v.get("modifiers").cloned().unwrap_or(json!(0)) });
                        if let Some(t) = v.get("text") { params["text"] = t.clone(); params["unmodifiedText"] = t.clone(); }
                        if let Some(k) = v.get("windowsVirtualKeyCode") { params["windowsVirtualKeyCode"] = k.clone(); params["nativeVirtualKeyCode"] = k.clone(); }
                        cdp.fire("Input.dispatchKeyEvent", params).await;
                    }
                    Some("text") => { cdp.fire("Input.insertText", json!({ "text": v.get("text").cloned().unwrap_or(json!("")) })).await; }
                    Some("viewport") => {
                        vw = v.get("w").and_then(|x| x.as_u64()).unwrap_or(1024).clamp(240, 2560); vh = v.get("h").and_then(|x| x.as_u64()).unwrap_or(700).clamp(200, 2000);
                        cdp.fire("Emulation.setDeviceMetricsOverride", json!({ "width": vw, "height": vh, "deviceScaleFactor": 1, "mobile": false })).await;
                        cdp.fire("Page.stopScreencast", json!({})).await;
                        screencast(cdp.clone(), vw, vh).await;
                    }
                    Some("newtab") => { let u = v.get("url").and_then(|u| u.as_str()).unwrap_or("about:blank"); let _ = cdp_http(&format!("/json/new?{u}"), "PUT").await; let _ = client.send(Message::Text(json!({ "kind": "tabs", "active": id, "list": pages().await }).to_string().into())).await; }
                    Some("close") => { if let Some(t) = v.get("id").and_then(|i| i.as_str()) { let _ = cdp_http(&format!("/json/close/{t}"), "GET").await; } let _ = client.send(Message::Text(json!({ "kind": "tabs", "active": id, "list": pages().await }).to_string().into())).await; }
                    Some("tabs") => { let _ = client.send(Message::Text(json!({ "kind": "tabs", "active": id, "list": pages().await }).to_string().into())).await; }
                    // Design Mode (§4.9 step 3): highlight under the cursor and element capture
                    // on click
                    Some("design") => {
                        design = v.get("on").and_then(|b| b.as_bool()).unwrap_or(false);
                        if design { for m in ["DOM.enable", "CSS.enable", "Overlay.enable"] { cdp.fire(m, json!({})).await; } let _ = cdp.call("DOM.getDocument", json!({ "depth": 0 })).await; }
                        else { cdp.fire("Overlay.hideHighlight", json!({})).await; }
                    }
                    Some("hover") => {
                        if design {
                            let (x, y) = (v.get("x").and_then(|x| x.as_i64()).unwrap_or(0), v.get("y").and_then(|y| y.as_i64()).unwrap_or(0));
                            if let Ok(n) = cdp.call("DOM.getNodeForLocation", json!({ "x": x, "y": y, "includeUserAgentShadowDOM": false })).await {
                                if let Some(b) = n.get("backendNodeId") { cdp.fire("Overlay.highlightNode", json!({ "backendNodeId": b, "highlightConfig": { "showInfo": true, "contentColor": { "r": 111, "g": 168, "b": 220, "a": 0.45 }, "paddingColor": { "r": 147, "g": 196, "b": 125, "a": 0.4 }, "borderColor": { "r": 255, "g": 229, "b": 153, "a": 0.6 }, "marginColor": { "r": 246, "g": 178, "b": 107, "a": 0.4 } } })).await; }
                            }
                        }
                    }
                    Some("inspect") => {
                        let (x, y) = (v.get("x").and_then(|x| x.as_i64()).unwrap_or(0), v.get("y").and_then(|y| y.as_i64()).unwrap_or(0));
                        match inspect(&cdp, x, y).await {
                            Ok(el) => { let mut el = el; el["kind"] = json!("element"); let _ = client.send(Message::Text(el.to_string().into())).await; }
                            Err(e) => { let _ = client.send(Message::Text(json!({ "kind": "element", "error": format!("{e:#}") }).to_string().into())).await; }
                        }
                    }
                    _ => {}
                }
            }
            _ = tick.tick() => {
                let list = pages().await;
                if !list.iter().any(|p| p.get("id").and_then(|i| i.as_str()) == Some(id.as_str())) { let _ = client.send(Message::Text(json!({ "kind": "error", "error": "вкладка закрыта" }).to_string().into())).await; break }
                let _ = client.send(Message::Text(json!({ "kind": "tabs", "active": id, "list": list }).to_string().into())).await;
            }
        }
    }
    reader.abort(); writer.abort();
}

/// Selector, HTML, styles and screenshot crop of the element under a point
/// (Design Mode)
async fn inspect(cdp: &Cdp, x: i64, y: i64) -> Result<Value> {
    let n = cdp.call("DOM.getNodeForLocation", json!({ "x": x, "y": y, "includeUserAgentShadowDOM": false })).await?;
    let backend = n.get("backendNodeId").cloned().ok_or_else(|| anyhow!("под курсором нет элемента"))?;
    let node_id = match n.get("nodeId").and_then(|i| i.as_i64()).filter(|i| *i > 0) { Some(i) => i, None => cdp.call("DOM.pushNodesByBackendIdsToFrontend", json!({ "backendNodeIds": [backend] })).await?.get("nodeIds").and_then(|a| a.as_array()).and_then(|a| a.first()).and_then(|i| i.as_i64()).ok_or_else(|| anyhow!("узел не найден"))? };
    let html = cdp.call("DOM.getOuterHTML", json!({ "nodeId": node_id })).await?.get("outerHTML").and_then(|h| h.as_str()).unwrap_or("").to_string();
    let obj = cdp.call("DOM.resolveNode", json!({ "nodeId": node_id })).await?;
    let object_id = obj.pointer("/object/objectId").and_then(|o| o.as_str()).ok_or_else(|| anyhow!("нет объекта элемента"))?.to_string();
    let js = r#"function() {
      const el = this; const r = el.getBoundingClientRect(); const cs = getComputedStyle(el);
      const props = ['display','position','width','height','margin','padding','font-family','font-size','font-weight','line-height','color','background-color','border','border-radius','gap','flex-direction','justify-content','align-items','grid-template-columns','opacity','z-index','box-shadow'];
      const styles = {}; for (const p of props) { const v = cs.getPropertyValue(p); if (v && v !== 'none' && v !== 'normal' && v !== 'auto' && v !== '0px' && v !== 'static' && v !== 'rgba(0, 0, 0, 0)') styles[p] = v; }
      const sel = (e) => { if (e.id) return '#' + e.id; const parts = []; while (e && e.nodeType === 1 && parts.length < 5) { let s = e.tagName.toLowerCase(); if (e.id) { parts.unshift('#' + e.id); break; } const cls = [...e.classList].filter(c => !/^svelte-|^[a-z]+-[0-9a-z]{5,}$/.test(c)).slice(0, 2); if (cls.length) s += '.' + cls.join('.'); const p = e.parentElement; if (p) { const same = [...p.children].filter(c => c.tagName === e.tagName); if (same.length > 1) s += `:nth-of-type(${same.indexOf(e) + 1})`; } parts.unshift(s); e = p; } return parts.join(' > '); };
      return { selector: sel(el), tag: el.tagName.toLowerCase(), text: (el.innerText || '').trim().slice(0, 200), rect: { x: r.left, y: r.top, w: r.width, h: r.height }, styles, url: location.href, title: document.title };
    }"#;
    let info = cdp.call("Runtime.callFunctionOn", json!({ "objectId": object_id, "functionDeclaration": js, "returnByValue": true })).await?;
    let mut info = info.pointer("/result/value").cloned().unwrap_or(json!({}));
    // screenshot crop: by the element rectangle within the window, at most 1200×900
    let r = info.get("rect").cloned().unwrap_or(json!({}));
    let (rx, ry, rw, rh) = (r.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0), r.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0), r.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0).min(1200.0), r.get("h").and_then(|v| v.as_f64()).unwrap_or(0.0).min(900.0));
    if rw >= 2.0 && rh >= 2.0 {
        if let Ok(shot) = cdp.call("Page.captureScreenshot", json!({ "format": "png", "clip": { "x": rx, "y": ry, "width": rw, "height": rh, "scale": 1 }, "captureBeyondViewport": false })).await { info["png"] = shot.get("data").cloned().unwrap_or(Value::Null); }
    }
    let html: String = html.chars().take(4000).collect();
    info["html"] = json!(html);
    info["backendNodeId"] = backend;
    Ok(info)
}

/// Screencast target: the given tab, otherwise the first one; with no tabs — a new
/// about:blank
async fn pick_target(target: Option<String>) -> Result<String> {
    if !running().await { bail!("браузер не запущен") }
    let list = pages().await;
    if let Some(t) = target { if list.iter().any(|p| p.get("id").and_then(|i| i.as_str()) == Some(t.as_str())) { return Ok(t) } }
    if let Some(p) = list.first() { return Ok(p.get("id").and_then(|i| i.as_str()).unwrap_or_default().to_string()) }
    let v = cdp_http("/json/new?about:blank", "PUT").await?;
    v.get("id").and_then(|i| i.as_str()).map(str::to_string).ok_or_else(|| anyhow!("не удалось открыть вкладку"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_server_attaches_to_our_cdp() {
        let s = mcp_server();
        assert_eq!(s.name, "browser"); assert_eq!(s.transport, "stdio"); assert_eq!(s.command, "npx");
        assert!(s.args.iter().any(|a| a == "--cdp-endpoint") && s.args.iter().any(|a| a == "http://127.0.0.1:9222"));
        assert!(install_cmd()[2].contains("playwright") && install_cmd()[2].contains("install chromium"));
    }
}
