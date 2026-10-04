//! cloudos-agentd — адаптер AgentVerse OS внутри workspace (ADR-006). Единственный клиент — cloudd на хосте: он
//! приходит по сети проекта с токеном, который сам же положил в контейнер. Протокол терминала (v1): по WebSocket
//! бинарные кадры — байты ввода/вывода, текстовые — управление JSON (`hello`, `resize`, `ping`/`pong`, `exit`).
mod acp;
mod codex;
mod engines;
mod fork;
mod fs;
mod import;
mod llm;
mod mcp;
mod perms;
mod pty;
mod session;
mod transfer;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Request, State};
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
    /// Адрес, на котором слушать: сеть проекта, куда приходит cloudd
    #[arg(long, env = "AGENTD_LISTEN", default_value = "0.0.0.0:7200")]
    listen: String,
    /// Файл с токеном, который кладёт cloudd
    #[arg(long, env = "AGENTD_TOKEN_FILE", default_value = "/etc/cloudos/agentd.token")]
    token_file: PathBuf,
    /// Токен явно (для разработки)
    #[arg(long, env = "AGENTD_TOKEN")]
    token: Option<String>,
    /// Работать без токена — только для отладки на loopback
    #[arg(long)]
    insecure: bool,
    /// Каталог данных (журналы сессий); по умолчанию ~/.cloudos/agentd
    #[arg(long, env = "AGENTD_DATA")]
    data: Option<PathBuf>,
}

struct AppState {
    ptys: RwLock<HashMap<String, Arc<PtySession>>>,
    sessions: Manager,
    token: Option<String>,
    started: Instant,
    /// Каталог движков с результатом обнаружения (engines.rs), кэш на минуту
    engines_cache: std::sync::Mutex<Option<(Instant, serde_json::Value)>>,
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
    let state = Arc::new(AppState { ptys: RwLock::new(HashMap::new()), sessions, token, started: Instant::now(), engines_cache: std::sync::Mutex::new(None) });
    // беседы, живые перед остановкой, возвращаются сами (срез 38); чуть позже старта, чтобы API уже отвечал
    { let st = state.clone(); tokio::spawn(async move { tokio::time::sleep(std::time::Duration::from_secs(2)).await; let n = st.sessions.resume_live().await; if n > 0 { tracing::info!("автовозобновление завершено: {n}"); } }); }
    let api = Router::new()
        .route("/v1/ptys", get(pty_list).post(pty_create))
        .route("/v1/ptys/{id}", get(pty_info).delete(pty_kill))
        .route("/v1/ptys/{id}/resize", axum::routing::post(pty_resize))
        .route("/v1/ptys/{id}/input", axum::routing::post(pty_input))
        .route("/v1/ptys/{id}/ws", get(pty_attach))
        .route("/v1/sessions", get(session_list).post(session_create))
        .route("/v1/sessions/{id}", get(session_info).delete(session_delete))
        .route("/v1/sessions/{id}/events", get(session_events))
        .route("/v1/sessions/{id}/download", get(session_download))
        .route("/v1/search", get(search))
        .route("/v1/mcp", get(mcp_get).post(mcp_set))
        .route("/v1/sessions/{id}/ws", get(session_attach))
        .route("/v1/sessions/{id}/prompt", axum::routing::post(session_prompt))
        .route("/v1/sessions/{id}/permission", axum::routing::post(session_permission))
        .route("/v1/sessions/{id}/interrupt", axum::routing::post(session_interrupt))
        .route("/v1/sessions/{id}/mode", axum::routing::post(session_mode))
        .route("/v1/sessions/{id}/model", axum::routing::post(session_model))
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

/// Bearer-токен в заголовке или `?token=` (WebSocket из браузера заголовков не ставит; cloudd ставит заголовок).
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
    /// Shell карточки сессии: PTY принадлежит сессии и убирается вместе с ней
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
    /// Движок, чей TUI запускается (меню «+»): перед стартом готовим его конфигурацию как перед сессией
    engine: Option<String>,
}
fn d_cols() -> u16 { 120 }
fn d_rows() -> u16 { 36 }

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

/// Без команды — login-оболочка: она читает `/etc/profile.d/cloudos-env.sh`, и агенты получают gate и аккаунт.
async fn pty_create(State(st): State<Arc<AppState>>, Json(b): Json<CreateBody>) -> Result<Json<pty::PtyInfo>, ApiError> {
    let cmd = if b.cmd.is_empty() { vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into()), "-l".into()] } else { b.cmd };
    let title = b.title.clone().unwrap_or_else(|| cmd.join(" "));
    let cwd = b.cwd.unwrap_or_else(home);
    let mut env = b.env;
    // TUI ACP-движка из меню «+» (без сессии agentd): та же подготовка, что перед сессией — иначе pi стартует со своей
    // моделью без ключа (401), а Gemini без gate и доверенной папки
    if let Some(engine) = b.engine.as_deref().and_then(engines::get).filter(|e| e.protocol == "acp").or_else(|| engines::in_cmd(&cmd)) {
        let base = std::fs::read_to_string("/etc/cloudos/env.sh").map(|t| codex::parse_env_sh(&t)).unwrap_or_default();
        if engine.id == "pi" { acp::pi_ensure_gate(&base).await; }
        if engine.id == "gemini" {
            acp::gemini_ensure_trust(&cwd);
            if !base.contains_key("GOOGLE_GEMINI_BASE_URL") { if let Some(g) = base.get("GEMINI_BASE_URL") { env.entry("GOOGLE_GEMINI_BASE_URL".into()).or_insert_with(|| g.clone()); } }
            env.entry("NO_BROWSER".into()).or_insert_with(|| "true".into());
        }
    }
    let p = PtySession::spawn(PtyOpts { cmd, cwd, cols: b.cols.max(10), rows: b.rows.max(4), title, env, session: b.session, tui: false })?;
    st.ptys.write().unwrap().insert(p.id.clone(), p.clone());
    tracing::info!("pty {} создан: {:?} в {}", &p.id[..8], p.cmd, p.cwd);
    Ok(Json(p.info()))
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

/// Убить процесс; запись остаётся, пока её не удалят повторным DELETE (тогда и хвост вывода пропадает).
async fn pty_kill(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let p = st.pty(&id).ok_or(ApiError::NotFound)?;
    if p.exit_code().is_none() {
        p.kill()?;
        Ok(StatusCode::ACCEPTED)
    } else {
        st.ptys.write().unwrap().remove(&id);
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
                    // Клиент не успевает: пропущенное не восстанавливаем, экран догонит по следующим кадрам.
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                },
                _ = exit.changed() => {
                    let code = *exit.borrow();  // Ref не Send — не держать через await
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

// ---- сессии агентов (session.rs) ----
/// Каталог движков с тем, что установлено в workspace (engines.rs); кэш 60 с — проверка версий зовёт четыре CLI.
async fn engines_list(State(st): State<Arc<AppState>>) -> Json<serde_json::Value> {
    {
        let c = st.engines_cache.lock().unwrap();
        if let Some((at, v)) = c.as_ref() { if at.elapsed() < Duration::from_secs(60) { return Json(v.clone()); } }
    }
    let list = engines::detect().await;
    let v = serde_json::json!({ "engines": list });
    *st.engines_cache.lock().unwrap() = Some((Instant::now(), v.clone()));
    Json(v)
}

/// Установка движка — в терминале (владелец видит ход npm); после установки кэш каталога сбрасывается.
async fn engine_install(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<pty::PtyInfo>, ApiError> {
    let e = engines::get(&id).ok_or_else(|| ApiError::BadRequest(format!("неизвестный движок {id}")))?;
    let p = PtySession::spawn(PtyOpts { cmd: engines::install_cmd(e), cwd: home(), cols: 120, rows: 30, title: format!("Установка {}", e.title), env: Default::default(), session: None, tui: false })?;
    st.ptys.write().unwrap().insert(p.id.clone(), p.clone());
    *st.engines_cache.lock().unwrap() = None;
    tracing::info!("установка движка {}: pty {}", e.id, &p.id[..8]);
    Ok(Json(p.info()))
}

fn session_of(st: &AppState, id: &str) -> Result<Arc<Session>, ApiError> {
    st.sessions.get(id).ok_or(ApiError::NotFound)
}

/// Живой PTY терминального режима этой сессии, если есть.
fn tui_of(st: &AppState, id: &str) -> Option<String> {
    st.ptys.read().unwrap().values().find(|p| p.tui && p.session.as_deref() == Some(id) && p.exit_code().is_none()).map(|p| p.id.clone())
}

fn with_tui(st: &AppState, mut info: session::Info) -> session::Info {
    info.tui = tui_of(st, &info.meta.id);
    info
}

/// Закрыть TUI сессии (SIGHUP группе, ждём выхода) и убрать его PTY; возвращает, был ли он.
async fn kill_tui(st: &AppState, id: &str) -> bool {
    let linked: Vec<Arc<PtySession>> = st.ptys.read().unwrap().values().filter(|p| p.tui && p.session.as_deref() == Some(id)).cloned().collect();
    let mut any = false;
    for p in linked {
        if p.exit_code().is_none() { any = true; if p.kill().is_ok() { p.wait_exit(Duration::from_secs(7)).await; } }
        st.ptys.write().unwrap().remove(&p.id);
    }
    any
}

/// Список — со сверкой с диском workspace'а (беседы Claude Code и Codex, начатые вне agentd; не чаще раза в 15 с).
async fn session_list(State(st): State<Arc<AppState>>) -> Json<Vec<session::Info>> {
    let st2 = st.clone();
    if let Ok(n) = tokio::task::spawn_blocking(move || st2.sessions.sync_disk()).await { if n > 0 { tracing::info!("с диска подхвачено бесед: {n}"); } }
    Json(st.sessions.list().into_iter().map(|i| with_tui(&st, i)).collect())
}

async fn session_create(State(st): State<Arc<AppState>>, Json(opts): Json<CreateOpts>) -> Result<Json<session::Info>, ApiError> {
    // один живой процесс на беседу: возобновление из чата закрывает TUI терминального режима
    if let Some(rid) = opts.resume.as_deref() { if kill_tui(&st, rid).await { tracing::info!("сессия {}: TUI закрыт перед возобновлением", &rid[..rid.len().min(8)]); } }
    let s = st.sessions.create(opts).await?;
    // один захват meta: два lock() одного Mutex в одном выражении держат guard'ы до конца макроса и встают намертво
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

/// Запрос к провайдеру для интерфейса: Claude — control_request (`{subtype, …}`), Codex — `{method, params}` app-server.
async fn session_control(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<serde_json::Value>) -> Result<Json<serde_json::Value>, ApiError> {
    if !req.is_object() { return Err(ApiError::BadRequest("нужен объект запроса".into())); }
    Ok(Json(session_of(&st, &id)?.control_request(req).await?))
}

#[derive(Deserialize)]
struct TuiBody { #[serde(default = "d_cols")] cols: u16, #[serde(default = "d_rows")] rows: u16, #[serde(default)] bypass: bool }

/// Терминальный режим (как в CloudCLI): та же беседа в родном TUI агента в PTY сессии. Один живой процесс на беседу:
/// headless-процесс останавливается и ждётся, прежний TUI закрывается; обратно — отправка из чата (resume) или DELETE.
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
    st.ptys.write().unwrap().insert(p.id.clone(), p.clone());
    tracing::info!("сессия {}: терминальный режим, pty {} (обход: {}): {}", &id[..8], &p.id[..8], b.bypass, p.cmd.last().map(String::as_str).unwrap_or(""));
    Ok(Json(p.info()))
}

#[derive(Deserialize)]
struct ForkBody { #[serde(default)] at_seq: Option<u64>, #[serde(default)] title: Option<String> }
/// Форк беседы: новая сессия с копией истории до события `at_seq` (или целиком).
async fn session_fork(State(st): State<Arc<AppState>>, Path(id): Path<String>, body: Option<Json<ForkBody>>) -> Result<Json<session::Info>, ApiError> {
    let b = body.map(|b| b.0).unwrap_or(ForkBody { at_seq: None, title: None });
    let ns = st.sessions.fork(&id, b.at_seq, b.title).await?;
    tracing::info!("сессия {}: форк → {}{}", &id[..8], &ns.id()[..8], b.at_seq.map(|s| format!(" до seq {s}")).unwrap_or_default());
    Ok(Json(ns.info()))
}

#[derive(Deserialize)]
struct EditBody { seq: u64, content: serde_json::Value }
/// Правка отправленного сообщения: журнал режется, процесс перезапускается с точки перед ним, новый текст уходит ходом.
async fn session_edit(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<EditBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let s = session_of(&st, &id)?;
    kill_tui(&st, &id).await;
    Ok(Json(s.edit(b.seq, b.content).await?))
}

#[derive(Deserialize)]
struct QueueBody { content: serde_json::Value }
/// Очередь сообщений: уйдёт само после текущего хода.
async fn session_queue(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<QueueBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let s = session_of(&st, &id)?;
    let item = s.queue_push(b.content);
    // сессия уже свободна — отправляем сразу
    if matches!(s.status(), session::Status::Idle) && s.pending().is_empty() { s.drain_queue(); }
    Ok(Json(item))
}
async fn session_unqueue(State(st): State<Arc<AppState>>, Path((id, qid)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    if session_of(&st, &id)?.queue_remove(&qid) { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::NotFound) }
}

/// Остановить фоновую задачу Claude Code (Bash в фоне, Agent, Workflow…): control `stop_task`.
async fn session_task_stop(State(st): State<Arc<AppState>>, Path((id, task)): Path<(String, String)>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(session_of(&st, &id)?.stop_task(&task).await?))
}

/// Архив сессии для переноса в другой workspace (transfer.rs): живой процесс и TUI останавливаются.
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

/// Принять архив сессии из другого workspace: запись с тем же id под новым cwd (по умолчанию — дом).
async fn session_import(State(st): State<Arc<AppState>>, axum::extract::Query(q): axum::extract::Query<ImportQuery>, body: axum::body::Bytes) -> Result<Json<session::Info>, ApiError> {
    if body.is_empty() { return Err(ApiError::BadRequest("пустой архив".into())); }
    let id = transfer::import(&st.sessions, &body, q.cwd).await?;
    let s = session_of(&st, &id)?;
    tracing::info!("сессия {}: импортирована из другого workspace в {}", &id[..8], s.meta.lock().unwrap().cwd);
    Ok(Json(s.info()))
}

/// Отключить терминальный режим: TUI закрывается, сессия остаётся завершённой и возобновляется из чата.
async fn session_tui_stop(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    session_of(&st, &id)?;
    if kill_tui(&st, &id).await { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::NotFound) }
}

#[derive(Deserialize)]
struct SinceQuery { #[serde(default)] since: u64, #[serde(default = "d_limit")] limit: usize }
fn d_limit() -> usize { 500 }

/// MCP-серверы workspace'а (срез 36): список и сколько их знает каждый CLI.
async fn mcp_get() -> Json<serde_json::Value> { Json(serde_json::json!({ "servers": mcp::servers(), "engines": mcp::engine_counts() })) }
#[derive(Deserialize)]
struct McpBody { #[serde(default)] servers: Vec<mcp::McpServer> }
async fn mcp_set(Json(b): Json<McpBody>) -> Result<Json<serde_json::Value>, ApiError> {
    mcp::save(b.servers).await.map(Json).map_err(ApiError::BadRequest)
}

#[derive(Deserialize)]
struct SearchQuery { #[serde(default)] q: String, #[serde(default = "d_search_limit")] limit: usize }
fn d_search_limit() -> usize { 30 }
/// Поиск по беседам workspace'а (срез 34): слова запроса — все, без регистра и ё/е; ответ — беседы с совпадениями (до 3 на
/// беседу), новые сверху.
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
/// Выгрузка беседы для чтения (срез 34; `/export` занят архивом переноса): Markdown (по умолчанию) или JSON журнала; имя файла —
/// ASCII-слаг названия (браузер подставит своё).
async fn session_download(State(st): State<Arc<AppState>>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<ExportQuery>) -> Result<Response, ApiError> {
    let s = session_of(&st, &id)?;
    let title = s.info().meta.title;
    let slug: String = title.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect::<String>().split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let base = if slug.is_empty() { format!("session-{}", &id[..id.len().min(8)]) } else { slug.chars().take(60).collect() };
    let (body, ctype, ext) = if q.format == "json" { (serde_json::to_string_pretty(&s.export_json()).unwrap_or_default(), "application/json; charset=utf-8", "json") } else { (s.export_md(), "text/markdown; charset=utf-8", "md") };
    Ok(([(axum::http::header::CONTENT_TYPE, ctype.to_string()), (axum::http::header::CONTENT_DISPOSITION, format!("attachment; filename=\"{base}.{ext}\""))], body).into_response())
}

async fn session_events(State(st): State<Arc<AppState>>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<SinceQuery>) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    Ok(Json(session_of(&st, &id)?.events_since(q.since, q.limit.min(5000))))
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

async fn session_mode(State(st): State<Arc<AppState>>, Path(id): Path<String>, Json(b): Json<ModeBody>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(session_of(&st, &id)?.set_permission_mode(&b.mode).await?))
}

/// Первый DELETE останавливает процесс (журнал остаётся, сессию можно возобновить) — или закрывает TUI терминального
/// режима, если подключён он; второй — убирает запись с журналом.
async fn session_delete(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let s = session_of(&st, &id)?;
    if s.alive() {
        s.stop()?;
        Ok(StatusCode::ACCEPTED)
    } else if kill_tui(&st, &id).await {
        Ok(StatusCode::ACCEPTED)
    } else {
        st.sessions.remove(&id)?;
        // shell-PTY карточки этой сессии уходят вместе с ней
        let linked: Vec<Arc<PtySession>> = st.ptys.read().unwrap().values().filter(|p| p.session.as_deref() == Some(id.as_str())).cloned().collect();
        for p in linked { let _ = p.kill(); st.ptys.write().unwrap().remove(&p.id); }
        Ok(StatusCode::NO_CONTENT)
    }
}

async fn usage_since(State(st): State<Arc<AppState>>, axum::extract::Query(q): axum::extract::Query<SinceQuery>) -> Json<Vec<serde_json::Value>> {
    Json(st.sessions.usage_since(q.since))
}

/// Поток событий сессии: текстовые кадры JSON — сначала всё после `?since=`, затем живые; ввод — только через HTTP.
async fn session_attach(State(st): State<Arc<AppState>>, Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<SinceQuery>, ws: WebSocketUpgrade) -> Result<Response, ApiError> {
    let s = session_of(&st, &id)?;
    let info = with_tui(&st, s.info());
    Ok(ws.on_upgrade(move |sock| session_ws(sock, s, info, q.since)))
}

async fn session_ws(mut sock: WebSocket, s: Arc<Session>, info: session::Info, since: u64) {
    let (snapshot, mut rx) = s.attach(since);
    let hello = serde_json::json!({ "kind": "hello", "protocol": PROTOCOL, "info": info, "pending": s.pending(), "replay": snapshot.len() });
    if sock.send(Message::Text(hello.to_string().into())).await.is_err() { return }
    for ev in snapshot {
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
                Ok(v) => { if sock.send(Message::Text(v.to_string().into())).await.is_err() { break } }
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
