//! cloudos-agentd — the AgentVerse OS adapter inside a workspace (ADR-006). The only client is cloudd on the
//! host: it comes over the project network with a token it placed into the container itself. Terminal protocol
//! (v1): over WebSocket, binary frames are input/output bytes, text frames are JSON control (`hello`, `resize`,
//! `ping`/`pong`, `exit`).
mod acp;
mod agy;
mod browser;
mod codex;
mod engines;
mod fork;
mod fs;
mod hub;
mod import;
mod llm;
mod mcp;
mod perms;
mod pty;
mod session;
mod transfer;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use pty::{PtyOpts, PtySession};
use session::{CreateOpts, Manager, Session};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast::error::RecvError;

pub const PROTOCOL: u32 = 1;

#[derive(Parser)]
#[command(name = "cloudos-agentd", version, about = "AgentVerse OS: адаптер workspace — PTY-демон, сессии агентов, файлы")]
struct Args {
    /// Address to listen on: the project network cloudd comes from
    #[arg(long, env = "AGENTD_LISTEN", default_value = "0.0.0.0:7200")]
    listen: String,
    /// File with the token placed by cloudd
    #[arg(long, env = "AGENTD_TOKEN_FILE", default_value = "/etc/cloudos/agentd.token")]
    token_file: PathBuf,
    /// Token given explicitly (for development)
    #[arg(long, env = "AGENTD_TOKEN")]
    token: Option<String>,
    /// Work without a token — only for debugging on
    /// loopback
    #[arg(long)]
    insecure: bool,
    /// Data directory (session journals); ~/.cloudos/agentd by default
    #[arg(long, env = "AGENTD_DATA")]
    data: Option<PathBuf>,
}

struct AppState {
    ptys: RwLock<HashMap<String, Arc<PtySession>>>,
    sessions: Manager,
    token: Option<String>,
    started: Instant,
    /// Engine catalog with the detection result (engines.rs), cached for a
    /// minute
    engines_cache: std::sync::Mutex<Option<(Instant, std::time::SystemTime, serde_json::Value)>>,
    /// One version poll at a time: parallel requests wait for the running one instead of spawning their own `--version`
    /// (each one is a 100 MB node)
    engines_flight: tokio::sync::Mutex<()>,
    /// Agent browser (browser.rs): this workspace's headless Chromium process
    browser: browser::Browser,
}

impl AppState {
    fn pty(&self, id: &str) -> Option<Arc<PtySession>> {
        self.ptys.read().unwrap().get(id).cloned()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();
    let args = Args::parse();
    let token = match args.token.clone().or_else(|| std::fs::read_to_string(&args.token_file).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())) {
        Some(t) => Some(t),
        None if args.insecure => {
            tracing::warn!("токена нет, --insecure: любой, кто дотянется до {}, управляет терминалами", args.listen);
            None
        }
        None => anyhow::bail!("нет токена: положите его в {} или задайте --token / AGENTD_TOKEN (или --insecure для отладки)", args.token_file.display()),
    };
    let data = args.data.clone().unwrap_or_else(|| PathBuf::from(home()).join(".cloudos/agentd"));
    let sessions = Manager::open(data.join("sessions")).with_context(|| format!("каталог данных {}", data.display()))?;
    let state = Arc::new(AppState { ptys: RwLock::new(HashMap::new()), sessions, token, started: Instant::now(), engines_cache: std::sync::Mutex::new(None), engines_flight: tokio::sync::Mutex::new(()), browser: browser::Browser::default() });
    // agent browser (browser.rs): enabled — the `browser` MCP server is in the workspace list; after a restart we start Chromium ourselves so sessions with
    // MCP do not wait
    { let st2 = state.clone(); tokio::spawn(async move { if mcp::servers().iter().any(|m| m.name == browser::MCP_NAME) { if let Err(e) = browser::start(&st2.browser).await { tracing::warn!("автозапуск браузера агента: {e:#}"); } } }); }
    // conversations alive before the stop come back by themselves (slice 38); a little after start so the
    // API already answers
    { let st = state.clone(); tokio::spawn(async move { tokio::time::sleep(std::time::Duration::from_secs(2)).await; let n = st.sessions.resume_live().await; if n > 0 { tracing::info!("автовозобновление завершено: {n}"); } }); }
    let api = Router::new()
        .route("/v1/events", get(events_ws))
        .route("/v1/ptys", get(pty_list).post(pty_create))
        .route("/v1/ptys/{id}", get(pty_info).delete(pty_kill))
        .route("/v1/ptys/{id}/resize", axum::routing::post(pty_resize))
        .route("/v1/ptys/{id}/input", axum::routing::post(pty_input))
        .route("/v1/ptys/{id}/text", get(pty_text))
        .route("/v1/ptys/{id}/ws", get(pty_attach))
        .route("/v1/sessions", get(session_list).post(session_create))
        .route("/v1/sessions/{id}", get(session_info).delete(session_delete))
        .route("/v1/sessions/{id}/events", get(session_events))
        .route("/v1/sessions/{id}/events/{seq}", get(session_event))
        .route("/v1/sessions/{id}/download", get(session_download))
        .route("/v1/search", get(search))
        .route("/v1/mcp", get(mcp_get).post(mcp_set))
        .route("/v1/mcp/health", get(mcp_health))
        // agent browser (browser.rs, §4.9 step 2): status, enable/disable, Chromium install, screencast and input
        .route("/v1/browser", get(browser_status))
        .route("/v1/browser/enable", axum::routing::post(browser_enable))
        .route("/v1/browser/disable", axum::routing::post(browser_disable))
        .route("/v1/browser/install", axum::routing::post(browser_install))
        .route("/v1/browser/ws", get(browser_ws))
        .route("/v1/sessions/{id}/ws", get(session_attach))
        .route("/v1/sessions/{id}/prompt", axum::routing::post(session_prompt))
        .route("/v1/sessions/{id}/permission", axum::routing::post(session_permission))
        .route("/v1/sessions/{id}/interrupt", axum::routing::post(session_interrupt))
        .route("/v1/sessions/{id}/mode", axum::routing::post(session_mode))
        .route("/v1/sessions/{id}/model", axum::routing::post(session_model))
        .route("/v1/sessions/{id}/label", axum::routing::post(session_label))
        .route("/v1/sessions/{id}/limit_hit", axum::routing::post(session_limit_hit))
        .route("/v1/sessions/{id}/note", axum::routing::post(session_note))
        .route("/v1/sessions/{id}/tui", axum::routing::post(session_tui).delete(session_tui_stop))
        .route("/v1/sessions/{id}/effort", axum::routing::post(session_effort))
        .route("/v1/sessions/{id}/thinking", axum::routing::post(session_thinking))
        .route("/v1/sessions/{id}/tier", axum::routing::post(session_tier))
        .route("/v1/sessions/{id}/cache", axum::routing::post(session_cache))
        .route("/v1/sessions/{id}/control", axum::routing::post(session_control))
        .route("/v1/sessions/{id}/export", get(session_export))
        .route("/v1/sessions/{id}/tasks/{task}/stop", axum::routing::post(session_task_stop))
        .route("/v1/sessions/{id}/fork", axum::routing::post(session_fork))
        .route("/v1/sessions/{id}/edit", axum::routing::post(session_edit))
        .route("/v1/sessions/{id}/queue", axum::routing::post(session_queue))
        .route("/v1/sessions/{id}/queue/{qid}", axum::routing::delete(session_unqueue))
        .route("/v1/permissions", get(perms::get).put(perms::put))
        .route("/v1/sessions/import", axum::routing::post(session_import).layer(axum::extract::DefaultBodyLimit::max(transfer::MAX_ARCHIVE + 1024)))
        .route("/v1/fs/find", get(fs::find))
        .route("/v1/usage", get(usage_since))
        .route("/v1/fs", get(fs::list).delete(fs::remove))
        .route("/v1/fs/file", get(fs::read).put(fs::write))
        .route("/v1/fs/mkdir", axum::routing::post(fs::mkdir))
        .route("/v1/fs/rename", axum::routing::post(fs::rename))
        .route("/v1/git/status", get(fs::git_status))
        .route("/v1/git/diff", get(fs::git_diff))
        .route("/v1/git/log", get(fs::git_log))
        .route("/v1/git/branches", get(fs::git_branches))
        .route("/v1/git/worktrees", get(fs::git_worktrees))
        .route("/v1/git/init", axum::routing::post(fs::git_init))
        .route("/v1/git/remote", axum::routing::post(fs::git_remote))
        .route("/v1/git/stage", axum::routing::post(fs::git_stage))
        .route("/v1/git/unstage", axum::routing::post(fs::git_unstage))
        .route("/v1/git/discard", axum::routing::post(fs::git_discard))
        .route("/v1/git/commit", axum::routing::post(fs::git_commit))
        .route("/v1/git/checkout", axum::routing::post(fs::git_checkout))
        .route("/v1/git/fetch", axum::routing::post(fs::git_fetch))
        .route("/v1/git/pull", axum::routing::post(fs::git_pull))
        .route("/v1/git/push", axum::routing::post(fs::git_push))
        .route("/v1/git/worktrees/add", axum::routing::post(fs::git_worktree_add))
        .route("/v1/git/worktrees/remove", axum::routing::post(fs::git_worktree_remove))
        .route("/v1/git/worktrees/merge", axum::routing::post(fs::git_worktree_merge))
        .route("/v1/git/suggest", axum::routing::post(fs::git_suggest))
        .route("/v1/llm/quick", axum::routing::post(llm::quick))
        .route("/v1/engines", get(engines_list))
        .route("/v1/engines/{id}/install", axum::routing::post(engine_install))
        .layer(axum::extract::DefaultBodyLimit::max(fs::MAX_FILE + 1024))
        .layer(middleware::from_fn_with_state(state.clone(), auth));
    let app = Router::new().route("/health", get(health)).merge(api).with_state(state);
    let listener = tokio::net::TcpListener::bind(&args.listen).await.with_context(|| format!("bind {}", args.listen))?;
    tracing::info!("cloudos-agentd {} слушает {} (протокол {PROTOCOL})", env!("CARGO_PKG_VERSION"), args.listen);
    axum::serve(listener, app).await?;
    Ok(())
}

/// Bearer token in the header or `?token=` (a browser WebSocket sets no headers; cloudd sets the header).
async fn auth(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let Some(expected) = st.token.as_deref() else { return next.run(req).await };
    let header = req.headers().get(AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::to_owned);
    let query = req.uri().query().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token="))).map(str::to_owned);
    if [header, query].into_iter().flatten().any(|t| t.len() == expected.len() && t.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "нужен токен agentd").into_response()
    }
}

async fn health(State(st): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok().and_then(|s| s.lines().next().and_then(|l| l.rsplit(':').next().map(str::to_owned)));
    let ptys = st.ptys.read().unwrap();
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"), "protocol": PROTOCOL, "uptime_s": st.started.elapsed().as_secs(),
        "ptys": ptys.len(), "ptys_alive": ptys.values().filter(|p| p.exit_code().is_none()).count(),
        "sessions": st.sessions.list().len(), "sessions_alive": st.sessions.list().iter().filter(|s| s.alive).count(),
        "user": std::env::var("USER").ok(), "cgroup": cgroup, "auth": st.token.is_some(),
    }))
}

#[derive(Deserialize)]
struct CreateBody {
    /// Shell of the session card: the PTY belongs to the session and goes
    /// away with it
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    cmd: Vec<String>,
    cwd: Option<String>,
    #[serde(default = "d_cols")]
    cols: u16,
    #[serde(default = "d_rows")]
    rows: u16,
    title: Option<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    /// The engine whose TUI is being started (the "+" menu): prepare its configuration before start as
    /// before a session
    engine: Option<String>,
}
fn d_cols() -> u16 { 120 }
fn d_rows() -> u16 { 36 }

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

/// Without a command — a login shell: it reads `/etc/profile.d/cloudos-env.sh`, and the agents get the gate
/// and the account.
async fn pty_create(State(st): State<Arc<AppState>>, Json(b): Json<CreateBody>) -> Result<Json<pty::PtyInfo>, ApiError> {
    let mut cmd = if b.cmd.is_empty() { vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into()), "-l".into()] } else { b.cmd };
    let title = b.title.clone().unwrap_or_else(|| cmd.join(" "));
    let cwd = b.cwd.unwrap_or_else(home);
    let mut env = b.env;
    // TUI of an ACP engine from the "+" menu (no agentd session): the same preparation as before a session — otherwise
    // pi starts with its own model without a key (401), and Gemini without the gate and a trusted folder
    if let Some(engine) = b.engine.as_deref().and_then(engines::get).filter(|e| e.protocol == "acp" || e.protocol == "agy").or_else(|| engines::in_cmd(&cmd)) {
        let base = std::fs::read_to_string("/etc/cloudos/env.sh").map(|t| codex::parse_env_sh(&t)).unwrap_or_default();
        if engine.id == "pi" { acp::pi_ensure_gate(&base).await; }
        // Oh My Pi: the gate provider and the model as a flag (without it omp takes its own default google/…,
        // which we do not have)
        if engine.id == "omp" {
            if let Some(d) = acp::omp_ensure_gate(&base).await {
                if let Some(last) = cmd.last_mut() { if last.trim() == "exec omp" || last.trim() == "omp" { last.push_str(&format!(" --model {d}")); } }
            }
        }
        if engine.id == "gemini" {
            acp::gemini_ensure_trust(&cwd);
            if !base.contains_key("GOOGLE_GEMINI_BASE_URL") { if let Some(g) = base.get("GEMINI_BASE_URL") { env.entry("GOOGLE_GEMINI_BASE_URL".into()).or_insert_with(|| g.clone()); } }
            env.entry("NO_BROWSER".into()).or_insert_with(|| "true".into());
        }
        // Antigravity through the gate: Gemini API key mode in agy's settings.json, the gate address in the
        // environment, and the TUI from the "+" menu starts on the best gate Gemini (gemini-api://…) — agy's built-in
        // catalog sends the gate names it does not have
        if engine.id == "antigravity" && acp::antigravity_ensure_gate(&base) {
            for (k, v) in acp::antigravity_env(&base) { env.entry(k).or_insert(v); }
            if let Some(last) = cmd.last_mut() {
                if last.trim() == "exec agy" || last.trim() == "agy" {
                    if let Some(m) = acp::pick_gemini_model(&codex::gate_model_ids(&base).await) { last.push_str(&format!(" --model {}", acp::antigravity_model(&m, true))); }
                }
            }
        }
    }
    // Engine TUI from the "+" menu (`bash -lic "exec agy …"`): binaries from ~/.local/bin and ~/.npm-global/bin —
    // regardless of the profile
    let mut cmd = cmd;
    if cmd.len() == 3 && cmd[0] == "bash" && cmd[1] == "-lic" && cmd[2].starts_with("exec ") { cmd[2] = format!("{}{}", engines::PATH_PREFIX, cmd[2]); }
    let p = PtySession::spawn(PtyOpts { cmd, cwd, cols: b.cols.max(10), rows: b.rows.max(4), title, env, session: b.session, tui: false })?;
    st.ptys.write().unwrap().insert(p.id.clone(), p.clone()); hub::touch();
    tracing::info!("pty {} создан: {:?} в {}", &p.id[..8], p.cmd, p.cwd);
    Ok(Json(p.info()))
}

/// Registry snapshot for the hub (ADR-007): sessions (with TUI) and terminals — the same as `/v1/sessions` and
/// `/v1/ptys` return.
fn registry_snapshot(st: &AppState) -> serde_json::Value {
    let sessions: Vec<session::Info> = st.sessions.list().into_iter().map(|i| with_tui(st, i)).collect();
    let mut ptys: Vec<pty::PtyInfo> = st.ptys.read().unwrap().values().map(|p| p.info()).collect();
    ptys.sort_by_key(|p| p.created_at);
    // engine catalog version (the install stamp, engines.rs STAMP): when it changes the client re-reads the catalog —
    // the "+" menu switches "Install…" to "Chat with …" without a reload, in any tab and on the phone
    serde_json::json!({ "kind": "snapshot", "sessions": sessions, "ptys": ptys, "engines_v": engines::stamp_secs() })
}

/// `GET /v1/events` — the hub WebSocket: a snapshot on connect, then a new snapshot on every registry change (250 ms debounce,
/// only if it differs), keepalive `ping` every 20 s. Disk reconciliation — as for the list, at most once per 15 s.
async fn events_ws(State(st): State<Arc<AppState>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |mut sock| async move {
        let mut rx = hub::subscribe();
        let st2 = st.clone(); let _ = tokio::task::spawn_blocking(move || st2.sessions.sync_disk()).await;
        let mut last = registry_snapshot(&st);
        if sock.send(Message::Text(last.to_string().into())).await.is_err() { return; }
        loop {
            let changed = tokio::select! {
                r = rx.recv() => match r { Ok(()) => true, Err(RecvError::Lagged(_)) => true, Err(RecvError::Closed) => return },
                _ = tokio::time::sleep(Duration::from_secs(20)) => false,
                m = sock.recv() => match m { Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return, _ => continue },
            };
            if !changed {
                // the engine install stamp changed without registry events (an install in a terminal) — a new snapshot
                // instead of ping
                if last.get("engines_v").and_then(|v| v.as_u64()).unwrap_or(0) != engines::stamp_secs() {
                    last = registry_snapshot(&st);
                    if sock.send(Message::Text(last.to_string().into())).await.is_err() { return; }
                    continue;
                }
                if sock.send(Message::Text("{\"kind\":\"ping\"}".into())).await.is_err() { return; }
                continue;
            }
            // debounce: a model turn yields dozens of events per second — gather them into one
            // snapshot
            tokio::time::sleep(Duration::from_millis(250)).await;
            while rx.try_recv().is_ok() {}
            let st2 = st.clone(); let _ = tokio::task::spawn_blocking(move || st2.sessions.sync_disk()).await;
            let snap = registry_snapshot(&st);
            if snap != last {
                last = snap;
                if sock.send(Message::Text(last.to_string().into())).await.is_err() { return; }
            }
        }
    })
}

async fn pty_list(State(st): State<Arc<AppState>>) -> Json<Vec<pty::PtyInfo>> {
    let mut v: Vec<_> = st.ptys.read().unwrap().values().map(|p| p.info()).collect();
    v.sort_by_key(|p| p.created_at);
    Json(v)
}

async fn pty_info(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<pty::PtyInfo>, ApiError> {
    Ok(Json(st.pty(&id).ok_or(ApiError::NotFound)?.info()))
}

#[derive(Deserialize)]
struct ResizeBody { cols: u16, rows: u16 }

async fn pty_resize(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<ResizeBody>) -> Result<Json<pty::PtyInfo>, ApiError> {
    let p = st.pty(&id).ok_or(ApiError::NotFound)?;
    p.resize(b.cols.max(10), b.rows.max(4))?;
    Ok(Json(p.info()))
}

#[derive(Deserialize)]
struct InputBody { data: String }

async fn pty_input(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<InputBody>) -> Result<StatusCode, ApiError> {
    st.pty(&id).ok_or(ApiError::NotFound)?.write(b.data.as_bytes())?;
    Ok(StatusCode::NO_CONTENT)
}

/// Kill the process; the record stays until removed by a second DELETE (then the output tail goes too).
async fn pty_kill(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let p = st.pty(&id).ok_or(ApiError::NotFound)?;
    if p.exit_code().is_none() {
        p.kill()?;
        Ok(StatusCode::ACCEPTED)
    } else {
        st.ptys.write().unwrap().remove(&id); hub::touch();
        Ok(StatusCode::NO_CONTENT)
    }
}

async fn pty_attach(State(st): State<Arc<AppState>>, Path(id): Path<String>, ws: WebSocketUpgrade) -> Result<Response, ApiError> {
    let p = st.pty(&id).ok_or(ApiError::NotFound)?;
    Ok(ws.on_upgrade(move |sock| pty_ws(sock, p)))
}

#[derive(Deserialize)]
#[serde(tag = "t")]
enum Control {
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
    #[serde(rename = "ping")]
    Ping,
}

/// Terminal output tail as text without control sequences (§9.11: the UI looks for the Google sign-in link in it)
async fn pty_text(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    let p = st.ptys.read().unwrap().get(&id).cloned().ok_or(ApiError::NotFound)?;
    let (snapshot, _, _) = p.attach();
    Ok(Json(serde_json::json!({ "id": id, "alive": p.exit_code().is_none(), "text": pty::strip_ansi(&snapshot) })))
}

async fn pty_ws(mut sock: WebSocket, p: Arc<PtySession>) {
    p.clients.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (snapshot, mut out, mut exit) = p.attach();
    let info = p.info();
    let hello = serde_json::json!({ "t": "hello", "protocol": PROTOCOL, "id": p.id, "cols": info.cols, "rows": info.rows, "replay": snapshot.len(), "alive": info.alive });
    let _ = sock.send(Message::Text(hello.to_string().into())).await;
    if !snapshot.is_empty() {
        let _ = sock.send(Message::Binary(snapshot.into())).await;
    }
    if let Some(code) = p.exit_code() {
        let _ = sock.send(Message::Text(serde_json::json!({ "t": "exit", "code": code }).to_string().into())).await;
    } else {
        loop {
            tokio::select! {
                msg = sock.recv() => match msg {
                    Some(Ok(Message::Binary(b))) => { if p.write(&b).is_err() { break } }
                    Some(Ok(Message::Text(t))) => match serde_json::from_str::<Control>(&t) {
                        Ok(Control::Resize { cols, rows }) => { let _ = p.resize(cols.max(10), rows.max(4)); }
                        Ok(Control::Ping) => { let _ = sock.send(Message::Text(r#"{"t":"pong"}"#.into())).await; }
                        Err(_) => {}
                    },
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    _ => {}
                },
                chunk = out.recv() => match chunk {
                    Ok(b) => { if sock.send(Message::Binary(b)).await.is_err() { break } }
                    // The client cannot keep up: what was missed is not restored, the screen catches up with
                    // the next frames.
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                },
                _ = exit.changed() => {
                    let code = *exit.borrow();  // Ref is not Send — do not hold it across an await
                    if let Some(code) = code {
                        let _ = sock.send(Message::Text(serde_json::json!({ "t": "exit", "code": code }).to_string().into())).await;
                        break;
                    }
                }
            }
        }
    }
    let _ = sock.send(Message::Close(None)).await;
    p.clients.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

// ---- agent sessions (session.rs) ----
/// Engine catalog with what is installed in the workspace (engines.rs); a 5-minute cache and one poll at a time: the version
/// check calls seven CLIs, and without single-flight simultaneous app opens (several tabs, e2e) launched dozens of `gemini
/// --version` at once — the workspace hit its memory limit and dragged the host into swap. The cache is invalidated by an
/// install (at its start), the engines::STAMP mark (the install command touches it at the end — a cache filled while the
/// install was running goes stale at once) and `?refresh=1`.
async fn engines_list(State(st): State<Arc<AppState>>, Query(q): Query<HashMap<String, String>>) -> Json<serde_json::Value> {
    const TTL: Duration = Duration::from_secs(300);
    let refresh = matches!(q.get("refresh").map(|s| s.as_str()), Some("1" | "true"));
    let cached = |st: &AppState| st.engines_cache.lock().unwrap().as_ref().filter(|(at, sys, _)| at.elapsed() < TTL && !engines::stamp_after(*sys)).map(|(_, _, v)| v.clone());
    if !refresh { if let Some(v) = cached(&st) { return Json(v); } }
    let _flight = st.engines_flight.lock().await;
    if !refresh { if let Some(v) = cached(&st) { return Json(v); } }
    let list = engines::detect().await;
    let v = serde_json::json!({ "engines": list });
    *st.engines_cache.lock().unwrap() = Some((Instant::now(), std::time::SystemTime::now(), v.clone()));
    hub::touch();   // the catalog was re-read — the hub sends out a snapshot with the new catalog version
    Json(v)
}

/// Engine install — in a terminal (the owner sees npm progress); after the install the catalog cache
/// is invalidated.
async fn engine_install(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<pty::PtyInfo>, ApiError> {
    let e = engines::get(&id).ok_or_else(|| ApiError::BadRequest(format!("неизвестный движок {id}")))?;
    let base = std::fs::read_to_string("/etc/cloudos/env.sh").map(|t| codex::parse_env_sh(&t)).unwrap_or_default();
    let gate = e.id == "antigravity" && acp::antigravity_gate(&base);
    let p = PtySession::spawn(PtyOpts { cmd: engines::install_cmd(e, gate), cwd: home(), cols: 120, rows: 30, title: format!("Установка {}", e.title), env: Default::default(), session: None, tui: false })?;
    st.ptys.write().unwrap().insert(p.id.clone(), p.clone()); hub::touch();
    *st.engines_cache.lock().unwrap() = None;
    tracing::info!("установка движка {}: pty {}", e.id, &p.id[..8]);
    Ok(Json(p.info()))
}

fn session_of(st: &AppState, id: &str) -> Result<Arc<Session>, ApiError> {
    st.sessions.get(id).ok_or(ApiError::NotFound)
}

/// Live terminal-mode PTY of this session, if any.
fn tui_of(st: &AppState, id: &str) -> Option<String> {
    st.ptys.read().unwrap().values().find(|p| p.tui && p.session.as_deref() == Some(id) && p.exit_code().is_none()).map(|p| p.id.clone())
}

fn with_tui(st: &AppState, mut info: session::Info) -> session::Info {
    info.tui = tui_of(st, &info.meta.id);
    info
}

/// Close the session's TUI (SIGHUP to the group, wait for exit) and remove its PTY; returns
/// whether there was one.
async fn kill_tui(st: &AppState, id: &str) -> bool {
    let linked: Vec<Arc<PtySession>> = st.ptys.read().unwrap().values().filter(|p| p.tui && p.session.as_deref() == Some(id)).cloned().collect();
    let mut any = false;
    for p in linked {
        if p.exit_code().is_none() { any = true; if p.kill().is_ok() { p.wait_exit(Duration::from_secs(7)).await; } }
        st.ptys.write().unwrap().remove(&p.id); hub::touch();
    }
    any
}

/// List — with reconciliation against the workspace disk (Claude Code and Codex conversations started outside
/// agentd; at most once per 15 s).
async fn session_list(State(st): State<Arc<AppState>>) -> Json<Vec<session::Info>> {
    let st2 = st.clone();
    if let Ok(n) = tokio::task::spawn_blocking(move || st2.sessions.sync_disk()).await { if n > 0 { tracing::info!("с диска подхвачено бесед: {n}"); } }
    Json(st.sessions.list().into_iter().map(|i| with_tui(&st, i)).collect())
}

async fn session_create(State(st): State<Arc<AppState>>, Json(opts): Json<CreateOpts>) -> Result<Json<session::Info>, ApiError> {
    // one live process per conversation: resuming from the chat closes the terminal-mode TUI
    if let Some(rid) = opts.resume.as_deref() { if kill_tui(&st, rid).await { tracing::info!("сессия {}: TUI закрыт перед возобновлением", &rid[..rid.len().min(8)]); } }
    let s = st.sessions.create(opts).await?;
    // one meta lock: two lock() of the same Mutex in one expression hold the guards until the end of the macro and
    // deadlock
    let m = s.meta.lock().unwrap().clone();
    tracing::info!("сессия {} ({}) в {}", &m.id[..8], m.agent, m.cwd);
    Ok(Json(s.info()))
}

async fn session_info(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    let s = session_of(&st, &id)?;
    Ok(Json(serde_json::json!({ "info": with_tui(&st, s.info()), "pending": s.pending(), "caps": s.caps() })))
}

#[derive(Deserialize)]
struct EffortBody { effort: Option<String> }

async fn session_effort(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<EffortBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let e = b.effort.map(|e| e.trim().to_ascii_lowercase()).filter(|e| !e.is_empty());
    if let Some(e) = &e { if !matches!(e.as_str(), "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra" | "ultracode" | "auto") { return Err(ApiError::BadRequest(format!("усилие {e}: ожидается low | medium | high | xhigh | max | ultra | ultracode | auto"))); } }
    Ok(Json(session_of(&st, &id)?.set_effort(e).await?))
}

#[derive(Deserialize)]
struct ThinkingBody { tokens: Option<u64> }
async fn session_thinking(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<ThinkingBody>) -> Result<Json<serde_json::Value>, ApiError> {
    if let Some(n) = b.tokens { if !(1024..=200_000).contains(&n) { return Err(ApiError::BadRequest("бюджет думания: от 1024 до 200000 токенов".into())); } }
    Ok(Json(session_of(&st, &id)?.set_thinking(b.tokens).await?))
}

#[derive(Deserialize)]
struct CacheBody { ttl: Option<String> }
async fn session_cache(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<CacheBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let t = b.ttl.map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty() && t != "auto");
    if let Some(t) = &t { if !matches!(t.as_str(), "5m" | "1h") { return Err(ApiError::BadRequest(format!("ttl {t}: ожидается 5m | 1h | auto"))); } }
    Ok(Json(session_of(&st, &id)?.set_cache_ttl(t).await?))
}
#[derive(Deserialize)]
struct TierBody { tier: Option<String> }
async fn session_tier(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<TierBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let t = b.tier.map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty());
    if let Some(t) = &t { if !matches!(t.as_str(), "fast" | "priority" | "default") { return Err(ApiError::BadRequest(format!("tier {t}: ожидается fast | priority | default"))); } }
    Ok(Json(session_of(&st, &id)?.set_tier(t).await?))
}

/// Provider request for the UI: Claude — control_request (`{subtype, …}`), Codex — app-server `{method, params}`.
async fn session_control(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<serde_json::Value>) -> Result<Json<serde_json::Value>, ApiError> {
    if !req.is_object() { return Err(ApiError::BadRequest("нужен объект запроса".into())); }
    Ok(Json(session_of(&st, &id)?.control_request(req).await?))
}

#[derive(Deserialize)]
struct TuiBody { #[serde(default = "d_cols")] cols: u16, #[serde(default = "d_rows")] rows: u16, #[serde(default)] bypass: bool }

/// Terminal mode (as in CloudCLI): the same conversation in the agent's native TUI in the session PTY. One live
/// process per conversation: the headless process is stopped and waited for, the previous TUI is closed; the way back
/// — a send from the chat (resume) or DELETE.
async fn session_tui(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<TuiBody>) -> Result<Json<pty::PtyInfo>, ApiError> {
    let s = session_of(&st, &id)?;
    if s.alive() {
        s.stop()?;
        if !s.wait_exit(Duration::from_secs(10)).await { return Err(anyhow::anyhow!("процесс сессии не остановился за 10 с").into()); }
    }
    kill_tui(&st, &id).await;
    let (cmd, env) = s.tui_command(b.bypass).await?;
    let m = s.meta.lock().unwrap().clone();
    let title = format!("{} · CLI · {}", engines::title(&m.agent), &m.id[..8]);
    let p = PtySession::spawn(PtyOpts { cmd, cwd: m.cwd.clone(), cols: b.cols.max(10), rows: b.rows.max(4), title, env, session: Some(id.clone()), tui: true })?;
    st.ptys.write().unwrap().insert(p.id.clone(), p.clone()); hub::touch();
    tracing::info!("сессия {}: терминальный режим, pty {} (обход: {}): {}", &id[..8], &p.id[..8], b.bypass, p.cmd.last().map(String::as_str).unwrap_or(""));
    Ok(Json(p.info()))
}

#[derive(Deserialize)]
struct ForkBody { #[serde(default)] at_seq: Option<u64>, #[serde(default)] title: Option<String> }
/// Conversation fork: a new session with a copy of the history up to event
/// `at_seq` (or whole).
async fn session_fork(State(st): State<Arc<AppState>>, Path(id): Path<String>, body: Option<Json<ForkBody>>) -> Result<Json<session::Info>, ApiError> {
    let b = body.map(|b| b.0).unwrap_or(ForkBody { at_seq: None, title: None });
    let ns = st.sessions.fork(&id, b.at_seq, b.title).await?;
    tracing::info!("сессия {}: форк → {}{}", &id[..8], &ns.id()[..8], b.at_seq.map(|s| format!(" до seq {s}")).unwrap_or_default());
    Ok(Json(ns.info()))
}

#[derive(Deserialize)]
struct EditBody { seq: u64, content: serde_json::Value }
/// Editing a sent message: the journal is cut, the process is restarted from the point before it, the new text goes as
/// a turn.
async fn session_edit(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<EditBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let s = session_of(&st, &id)?;
    kill_tui(&st, &id).await;
    Ok(Json(s.edit(b.seq, b.content).await?))
}

#[derive(Deserialize)]
struct QueueBody { content: serde_json::Value }
/// Message queue: goes out by itself after the
/// current turn.
async fn session_queue(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<QueueBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let s = session_of(&st, &id)?;
    let item = s.queue_push(b.content);
    // the session is already free — send right
    // away
    if matches!(s.status(), session::Status::Idle) && s.pending().is_empty() { s.drain_queue(); }
    Ok(Json(item))
}
async fn session_unqueue(State(st): State<Arc<AppState>>, Path((id, qid)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    if session_of(&st, &id)?.queue_remove(&qid) { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::NotFound) }
}

/// Stop a Claude Code background task (background Bash, Agent, Workflow…): control
/// `stop_task`.
async fn session_task_stop(State(st): State<Arc<AppState>>, Path((id, task)): Path<(String, String)>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(session_of(&st, &id)?.stop_task(&task).await?))
}

/// Session archive for moving to another workspace (transfer.rs): the live process and the TUI are
/// stopped.
async fn session_export(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let s = session_of(&st, &id)?;
    kill_tui(&st, &id).await;
    if s.alive() {
        s.stop()?;
        if !s.wait_exit(Duration::from_secs(10)).await { return Err(anyhow::anyhow!("процесс сессии не остановился за 10 с").into()); }
    }
    let data = transfer::export(&st.sessions, &id).await?;
    tracing::info!("сессия {}: экспорт для переноса, {} байт", &id[..8], data.len());
    Ok(([(axum::http::header::CONTENT_TYPE, "application/x-tar"), (axum::http::header::CACHE_CONTROL, "no-store")], data).into_response())
}

#[derive(Deserialize)]
struct ImportQuery { #[serde(default)] cwd: Option<String> }

/// Accept a session archive from another workspace: a record with the same id under a new cwd (home
/// by default).
async fn session_import(State(st): State<Arc<AppState>>, axum::extract::Query(q): axum::extract::Query<ImportQuery>, body: axum::body::Bytes) -> Result<Json<session::Info>, ApiError> {
    if body.is_empty() { return Err(ApiError::BadRequest("пустой архив".into())); }
    let id = transfer::import(&st.sessions, &body, q.cwd).await?;
    let s = session_of(&st, &id)?;
    tracing::info!("сессия {}: импортирована из другого workspace в {}", &id[..8], s.meta.lock().unwrap().cwd);
    Ok(Json(s.info()))
}

/// Turn off terminal mode: the TUI is closed, the session stays finished and is resumed from the chat.
async fn session_tui_stop(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    session_of(&st, &id)?;
    if kill_tui(&st, &id).await { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::NotFound) }
}

#[derive(Deserialize)]
struct SinceQuery { #[serde(default)] since: u64, #[serde(default = "d_limit")] limit: usize, #[serde(default)] lite: Option<String> }
fn is_lite(q: &SinceQuery) -> bool { matches!(q.lite.as_deref(), Some("1") | Some("true")) }
fn d_limit() -> usize { 500 }

/// Workspace MCP servers (slice 36): the list and how many of them each CLI
/// knows.
async fn browser_status(State(st): State<Arc<AppState>>) -> Json<serde_json::Value> { Json(browser::status(&st.browser).await) }
async fn browser_enable(State(st): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, ApiError> { browser::enable(&st.browser).await.map(Json).map_err(|e| ApiError::BadRequest(format!("{e:#}"))) }
async fn browser_disable(State(st): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, ApiError> { browser::disable(&st.browser).await.map(Json).map_err(|e| ApiError::BadRequest(format!("{e:#}"))) }
/// Chromium install — a terminal, as for the engines: the user sees the install
/// progress
async fn browser_install(State(st): State<Arc<AppState>>) -> Result<Json<pty::PtyInfo>, ApiError> {
    let p = PtySession::spawn(PtyOpts { cmd: browser::install_cmd(), cwd: home(), cols: 120, rows: 30, title: "Установка Chromium".into(), env: Default::default(), session: None, tui: false })?;
    st.ptys.write().unwrap().insert(p.id.clone(), p.clone()); hub::touch();
    Ok(Json(p.info()))
}
#[derive(Deserialize)]
struct BrowserWsQuery { target: Option<String> }
async fn browser_ws(ws: WebSocketUpgrade, axum::extract::Query(q): axum::extract::Query<BrowserWsQuery>) -> Response {
    ws.on_upgrade(move |sock| browser::serve(sock, q.target))
}

#[derive(Deserialize)]
struct HealthQuery { #[serde(default)] refresh: Option<String> }
/// MCP server health via `claude mcp list` (30 s cache, `?refresh=1` — anew)
async fn mcp_health(axum::extract::Query(q): axum::extract::Query<HealthQuery>) -> Json<serde_json::Value> {
    let refresh = matches!(q.refresh.as_deref(), Some("1") | Some("true"));
    Json(serde_json::json!({ "servers": mcp::health(refresh).await }))
}
async fn mcp_get() -> Json<serde_json::Value> { Json(serde_json::json!({ "servers": mcp::servers(), "engines": mcp::engine_counts() })) }
#[derive(Deserialize)]
struct McpBody { #[serde(default)] servers: Vec<mcp::McpServer> }
async fn mcp_set(Json(b): Json<McpBody>) -> Result<Json<serde_json::Value>, ApiError> {
    mcp::save(b.servers).await.map(Json).map_err(ApiError::BadRequest)
}

#[derive(Deserialize)]
struct SearchQuery { #[serde(default)] q: String, #[serde(default = "d_search_limit")] limit: usize }
fn d_search_limit() -> usize { 30 }
/// Search across workspace conversations (slice 34): the query words — all of them, case-insensitive and ё/е-insensitive;
/// the answer — conversations with matches (up to 3 per conversation), newest first.
async fn search(State(st): State<Arc<AppState>>, axum::extract::Query(q): axum::extract::Query<SearchQuery>) -> Json<serde_json::Value> {
    let words: Vec<String> = session::norm_text(&q.q).split_whitespace().filter(|w| w.chars().count() >= 2).map(str::to_string).collect();
    let mut out: Vec<serde_json::Value> = Vec::new();
    if !words.is_empty() {
        let sessions: Vec<Arc<Session>> = st.sessions.all();
        for s in sessions {
            let hits = s.search(&words, 3);
            if hits.is_empty() { continue }
            let info = s.info();
            out.push(serde_json::json!({ "session": info.meta.id, "title": info.meta.title, "agent": info.meta.agent, "cwd": info.meta.cwd, "created_at": info.meta.created_at, "updated_at": info.updated_at, "status": info.status, "hits": hits }));
            if out.len() >= q.limit.clamp(1, 200) { break }
        }
        out.sort_by(|a, b| b.get("updated_at").and_then(|u| u.as_u64()).cmp(&a.get("updated_at").and_then(|u| u.as_u64())));
    }
    Json(serde_json::json!({ "sessions": out, "words": words }))
}

#[derive(Deserialize)]
struct ExportQuery { #[serde(default)] format: String }
/// Conversation download for reading (slice 34; `/export` is taken by the transfer archive): Markdown (default) or the journal
/// JSON; the file name is an ASCII slug of the title (the browser substitutes its own).
async fn session_download(State(st): State<Arc<AppState>>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<ExportQuery>) -> Result<Response, ApiError> {
    let s = session_of(&st, &id)?;
    let title = s.info().meta.title;
    let slug: String = title.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect::<String>().split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let base = if slug.is_empty() { format!("session-{}", &id[..id.len().min(8)]) } else { slug.chars().take(60).collect() };
    let (body, ctype, ext) = if q.format == "json" { (serde_json::to_string_pretty(&s.export_json()).unwrap_or_default(), "application/json; charset=utf-8", "json") } else { (s.export_md(), "text/markdown; charset=utf-8", "md") };
    Ok(([(axum::http::header::CONTENT_TYPE, ctype.to_string()), (axum::http::header::CONTENT_DISPOSITION, format!("attachment; filename=\"{base}.{ext}\""))], body).into_response())
}

async fn session_events(State(st): State<Arc<AppState>>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<SinceQuery>) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let list = session_of(&st, &id)?.events_since(q.since, q.limit.min(5000));
    Ok(Json(if is_lite(&q) { list.iter().map(session::lite).collect() } else { list }))
}
/// Full event by number (§4.3.16, "show whole" after a lightweight replay)
async fn session_event(State(st): State<Arc<AppState>>, Path((id, seq)): Path<(String, u64)>) -> Result<Json<serde_json::Value>, ApiError> {
    session_of(&st, &id)?.event_at(seq).map(Json).ok_or(ApiError::NotFound)
}

#[derive(Deserialize)]
struct PromptBody { content: serde_json::Value }

async fn session_prompt(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<PromptBody>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(session_of(&st, &id)?.prompt(b.content).await?))
}

#[derive(Deserialize)]
struct PermissionBody { request_id: String, behavior: String, updated_input: Option<serde_json::Value>, message: Option<String>, answers: Option<serde_json::Value>, #[serde(default)] remember: bool, #[serde(default)] scope: Option<String>, #[serde(default)] permissions: Option<serde_json::Value> }

async fn session_permission(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<PermissionBody>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(session_of(&st, &id)?.answer(&b.request_id, &b.behavior, b.updated_input, b.message, b.answers, b.remember, b.scope, b.permissions).await?))
}

async fn session_interrupt(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(session_of(&st, &id)?.interrupt().await?))
}

#[derive(Deserialize)]
struct ModeBody { mode: String }

#[derive(Deserialize)]
struct ModelBody { model: Option<String>, #[serde(default)] context_window: Option<u64> }

async fn session_model(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<ModelBody>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(session_of(&st, &id)?.set_model(b.model.filter(|m| !m.trim().is_empty()), b.context_window).await?))
}

/// Core note into the conversation (§9.7: "no free accounts in the pool"
/// and the like)
async fn session_note(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<serde_json::Value>) -> Result<Json<serde_json::Value>, ApiError> {
    let s = session_of(&st, &id)?;
    let m = b.get("message").and_then(|m| m.as_str()).map(str::trim).filter(|m| !m.is_empty()).ok_or_else(|| ApiError::BadRequest("нужно поле message".into()))?;
    s.note(m);
    Ok(Json(serde_json::json!({ "ok": true })))
}
/// Simulated limit hit (e2e and a manual check of auto-switching, §9.7): the same mark rate_limit_event sets
async fn session_limit_hit(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<serde_json::Value>) -> Result<Json<serde_json::Value>, ApiError> {
    let s = session_of(&st, &id)?;
    s.set_limit_hit(serde_json::json!({ "window": b.get("window").cloned().unwrap_or(serde_json::json!("five_hour")), "resets_at": b.get("resets_at").cloned().unwrap_or(serde_json::Value::Null), "source": "manual" }));
    Ok(Json(serde_json::json!({ "ok": true, "limit_hit": s.info().meta.limit_hit })))
}
#[derive(Deserialize)]
struct LabelBody { label: Option<String> }
/// Own session name (docs/agents-ux.md §4.8): `{label: ""}` brings back the auto-title
async fn session_label(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<LabelBody>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(session_of(&st, &id)?.set_label(b.label)))
}

async fn session_mode(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<ModeBody>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(session_of(&st, &id)?.set_permission_mode(&b.mode).await?))
}

/// The first DELETE stops the process (the journal stays, the session can be resumed) — or closes the terminal-mode
/// TUI if that is attached; the second removes the record with the journal.
async fn session_delete(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let s = session_of(&st, &id)?;
    if s.alive() {
        s.stop()?;
        Ok(StatusCode::ACCEPTED)
    } else if kill_tui(&st, &id).await {
        Ok(StatusCode::ACCEPTED)
    } else {
        st.sessions.remove(&id)?;
        // the shell PTYs of this session's card go away with
        // it
        let linked: Vec<Arc<PtySession>> = st.ptys.read().unwrap().values().filter(|p| p.session.as_deref() == Some(id.as_str())).cloned().collect();
        for p in linked { let _ = p.kill(); st.ptys.write().unwrap().remove(&p.id); hub::touch(); }
        Ok(StatusCode::NO_CONTENT)
    }
}

async fn usage_since(State(st): State<Arc<AppState>>, axum::extract::Query(q): axum::extract::Query<SinceQuery>) -> Json<Vec<serde_json::Value>> {
    Json(st.sessions.usage_since(q.since))
}

/// Session event stream: JSON text frames — first everything after `?since=`, then live ones; input — only over
/// HTTP.
async fn session_attach(State(st): State<Arc<AppState>>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<SinceQuery>, ws: WebSocketUpgrade) -> Result<Response, ApiError> {
    let s = session_of(&st, &id)?;
    let info = with_tui(&st, s.info());
    let lite = is_lite(&q);
    Ok(ws.on_upgrade(move |sock| session_ws(sock, s, info, q.since, lite)))
}

async fn session_ws(mut sock: WebSocket, s: Arc<Session>, info: session::Info, since: u64, lite: bool) {
    let (snapshot, mut rx) = s.attach(since);
    let hello = serde_json::json!({ "kind": "hello", "protocol": PROTOCOL, "info": info, "pending": s.pending(), "replay": snapshot.len(), "lite": lite });
    if sock.send(Message::Text(hello.to_string().into())).await.is_err() { return }
    for ev in snapshot {
        let ev = if lite { session::lite(&ev) } else { ev };
        if sock.send(Message::Text(ev.to_string().into())).await.is_err() { return }
    }
    loop {
        tokio::select! {
            msg = sock.recv() => match msg {
                Some(Ok(Message::Text(t))) => { if t.as_str().contains("\"ping\"") { let _ = sock.send(Message::Text(r#"{"kind":"pong"}"#.into())).await; } }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                _ => {}
            },
            ev = rx.recv() => match ev {
                Ok(v) => { let v = if lite { session::lite(&v) } else { v }; if sock.send(Message::Text(v.to_string().into())).await.is_err() { break } }
                Err(RecvError::Lagged(n)) => { let _ = sock.send(Message::Text(serde_json::json!({ "kind": "lagged", "missed": n }).to_string().into())).await; }
                Err(RecvError::Closed) => break,
            },
        }
    }
    let _ = sock.send(Message::Close(None)).await;
}

#[derive(Debug)]
enum ApiError {
    BadRequest(String),
    NotFound,
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self { Self::Internal(e) }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "не найдено" }))).into_response(),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": m }))).into_response(),
            Self::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("{e:#}") }))).into_response(),
        }
    }
}
