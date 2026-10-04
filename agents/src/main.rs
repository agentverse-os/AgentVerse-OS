//! cloudos-agents — сервис «Агенты» (ADR-006, этап 2). Отдельный бэкенд и веб-приложение: живёт на своём порту (7300) и на
//! маршруте `/agents/` за edge, рядом с Desktop, но не внутри него. Ядро cloudd остаётся платформой (проекты, workspace'ы,
//! agentd, аккаунты, книга расходов, секреты); этот сервис — приложение поверх него: проксирует API ядра
//! (`/agents/api/core/*`, включая WebSocket терминалов и журналов сессий), собирает свои представления (рабочие места, входящие,
//! модели) и хранит своё (цены, умолчания, лимиты) в собственной SQLite.
mod api;
mod assist;
mod chat;
mod proxy;
mod routines;
mod scheduled;
mod search;
mod simplechat;
mod skills;
mod store;

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get};
use axum::Router;
use clap::Parser;
use rust_embed::RustEmbed;
use std::path::PathBuf;
use std::sync::Arc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "cloudos-agents", version, about = "AgentVerse OS: сервис «Агенты» — бэкенд и веб-приложение поверх cloudd")]
struct Args {
    /// Адреса через запятую: loopback и шлюз docker (оттуда приходит edge)
    #[arg(long, env = "AGENTS_LISTEN", default_value = "127.0.0.1:7300,172.17.0.1:7300")]
    listen: String,
    /// Адрес ядра
    #[arg(long, env = "CLOUDD_URL", default_value = "http://127.0.0.1:7100")]
    cloudd: String,
    /// Каталог данных сервиса (своя SQLite)
    #[arg(long, env = "AGENTS_DATA", default_value = "/var/lib/cloudos/agents")]
    data: PathBuf,
}

pub struct App {
    pub http: reqwest::Client,
    pub cloudd: String,
    pub store: store::Store,
    /// Каталог данных сервиса (вложения простого чата в chat-files/)
    pub data: PathBuf,
    /// Простой чат: инкогнито в памяти, идущие ответы, кэш моделей маршрутов (simplechat.rs)
    pub chat: simplechat::ChatState,
    /// scratch создаётся/запускается в фоне (chat.rs)
    pub scratch_busy: std::sync::atomic::AtomicBool,
    pub scratch_error: std::sync::Mutex<String>,
}

#[derive(RustEmbed)]
#[folder = "ui/dist/"]
struct Ui;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "agents=info,tower_http=warn".into())).init();
    let args = Args::parse();
    let store = store::Store::open(&args.data).with_context(|| format!("хранилище в {}", args.data.display()))?;
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build()?;
    let app = Arc::new(App { http, cloudd: args.cloudd.trim_end_matches('/').to_string(), store, data: args.data.clone(), chat: Default::default(), scratch_busy: Default::default(), scratch_error: Default::default() });
    routines::start(app.clone());
    let router = Router::new()
        .route("/agents/api/health", get(api::health))
        .route("/agents/api/workspaces", get(api::workspaces))
        .route("/agents/api/inbox", get(api::inbox))
        .route("/agents/api/models", get(api::models))
        .route("/agents/api/settings", get(api::settings_get).put(api::settings_put))
        .route("/agents/api/agents", get(routines::agents_list).post(routines::agents_create))
        .route("/agents/api/agents/{id}", axum::routing::put(routines::agents_update).delete(routines::agents_delete))
        .route("/agents/api/agents/{id}/run", axum::routing::post(routines::agent_run))
        .route("/agents/api/agents/{id}/mail", axum::routing::post(routines::agent_mail))
        .route("/agents/api/agents/draft", axum::routing::post(assist::draft))
        .route("/agents/api/skills", get(skills::list).post(skills::create))
        .route("/agents/api/skills/materialize", axum::routing::post(skills::materialize_api))
        .route("/agents/api/skills/{name}", axum::routing::put(skills::update).delete(skills::delete))
        .route("/agents/api/routines", get(routines::routines_list).post(routines::routines_create))
        .route("/agents/api/routines/{id}", axum::routing::put(routines::routines_update).delete(routines::routines_delete))
        .route("/agents/api/routines/{id}/run", axum::routing::post(routines::routine_run))
        .route("/agents/api/scheduled", get(scheduled::list).post(scheduled::create))
        .route("/agents/api/scheduled/{id}", axum::routing::delete(scheduled::delete))
        .route("/agents/api/runs", get(routines::runs_list))
        .route("/agents/api/runs/{id}", axum::routing::delete(routines::run_delete))
        .route("/agents/api/hooks/{token}", axum::routing::post(routines::hook))
        .route("/agents/api/chat", get(chat::status).post(chat::create))
        .route("/agents/api/chat/ensure", axum::routing::post(chat::ensure))
        .route("/agents/api/chat/{id}/move", axum::routing::post(chat::move_chat))
        // простой чат (simplechat.rs): маршруты и модели, чаты, поток ответа, вложения, инкогнито → история, в продвинутый
        .route("/agents/api/chat/routes", get(simplechat::routes))
        .route("/agents/api/chat/presets", get(simplechat::presets_get).put(simplechat::presets_put))
        .route("/agents/api/chats", get(simplechat::list).post(simplechat::create))
        .route("/agents/api/chats/{id}", get(simplechat::get).put(simplechat::update).delete(simplechat::delete))
        .route("/agents/api/chats/{id}/messages", axum::routing::post(simplechat::send))
        .route("/agents/api/chats/{id}/stop", axum::routing::post(simplechat::stop))
        .route("/agents/api/chats/{id}/keep", axum::routing::post(simplechat::keep))
        .route("/agents/api/chats/{id}/handoff", axum::routing::post(simplechat::handoff))
        .route("/agents/api/chats/{id}/files/{file}", get(simplechat::file))
        .route("/agents/api/chats/{id}/export", get(search::chat_export))
        .route("/agents/api/search", get(search::search))
        .route("/agents/api/core/{*path}", any(proxy::core))
        .route("/", get(|| async { Redirect::permanent("/agents/") }))
        .route("/agents", get(|| async { Redirect::permanent("/agents/") }))
        .route("/agents/", get(ui_index))
        .route("/agents/{*path}", get(ui_file))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(app.clone());
    let mut handles = Vec::new();
    for addr in args.listen.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => { tracing::warn!("не слушаю {addr}: {e}"); continue; }
        };
        tracing::info!("cloudos-agents {VERSION}: {addr} → ядро {}", app.cloudd);
        let r = router.clone();
        handles.push(tokio::spawn(async move { axum::serve(listener, r).await }));
    }
    if handles.is_empty() { anyhow::bail!("ни один адрес из {} не удалось занять", args.listen); }
    for h in handles { h.await??; }
    Ok(())
}

async fn ui_index() -> Response { serve_ui("index.html") }
async fn ui_file(Path(path): Path<String>, uri: Uri) -> Response {
    let _ = uri;
    if Ui::get(&path).is_some() { return serve_ui(&path); }
    // SPA: пути без расширения ведут на оболочку, остальное — 404
    if path.rsplit('/').next().map(|n| n.contains('.')).unwrap_or(false) { return (StatusCode::NOT_FOUND, "нет такого файла").into_response(); }
    serve_ui("index.html")
}
fn serve_ui(path: &str) -> Response {
    match Ui::get(path) {
        Some(f) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
            ([(header::CONTENT_TYPE, mime.as_ref().to_string()), (header::CACHE_CONTROL, cache.to_string())], f.data.into_owned()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "нет такого файла").into_response(),
    }
}

/// Ошибка API: JSON с текстом; ядро недоступно — 502, остальное — 500 (или явный статус).
pub struct ApiError(pub StatusCode, pub String);
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        let msg = format!("{e:#}");
        let code = if msg.contains("ядро не отвечает") { StatusCode::BAD_GATEWAY } else if msg.contains("не найден") { StatusCode::NOT_FOUND } else if msg.contains("неверн") { StatusCode::BAD_REQUEST } else { StatusCode::INTERNAL_SERVER_ERROR };
        ApiError(code, msg)
    }
}
impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self { ApiError(StatusCode::BAD_REQUEST, format!("неверные данные: {e}")) }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response { (self.0, axum::Json(serde_json::json!({ "error": self.1 }))).into_response() }
}

impl App {
    /// GET к ядру с разбором JSON; недоступность ядра — отдельная ошибка (502 наружу).
    pub async fn core_get(&self, path: &str) -> Result<serde_json::Value> {
        let r = self.http.get(format!("{}/api{path}", self.cloudd)).timeout(std::time::Duration::from_secs(20)).send().await.map_err(|e| anyhow::anyhow!("ядро не отвечает: {e}"))?;
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        if !status.is_success() {
            let msg = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string)).unwrap_or(text);
            anyhow::bail!("ядро {}: {}", status.as_u16(), msg);
        }
        Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::Null))
    }
    /// POST/DELETE к ядру — тем же правилам (JSON тело, ошибка ядра — текстом).
    pub async fn core_post(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> { self.core_call(reqwest::Method::POST, path, Some(body)).await }
    pub async fn core_delete(&self, path: &str) -> Result<serde_json::Value> { self.core_call(reqwest::Method::DELETE, path, None).await }
    async fn core_call(&self, method: reqwest::Method, path: &str, body: Option<&serde_json::Value>) -> Result<serde_json::Value> {
        let mut req = self.http.request(method, format!("{}/api{path}", self.cloudd)).timeout(std::time::Duration::from_secs(900));
        if let Some(b) = body { req = req.json(b); }
        let r = req.send().await.map_err(|e| anyhow::anyhow!("ядро не отвечает: {e}"))?;
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        if !status.is_success() {
            let msg = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string)).unwrap_or(text);
            anyhow::bail!("ядро {}: {}", status.as_u16(), msg);
        }
        Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::Null))
    }
}

pub type AppRef = State<Arc<App>>;
