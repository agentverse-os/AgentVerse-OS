//! HTTP API `cloudd` (`/api/*`) и статика Desktop (`/`). Слушает только на localhost и docker-мосте;
//! наружу — через edge-Caddy. Identity v1 — периметр overlay-сети (2.3), поэтому авторизации у API нет.

use crate::model::*;
use crate::service::Service;
use crate::store::Event;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::{OpenApi, ToSchema};

#[derive(rust_embed::RustEmbed)]
#[folder = "static/"]
struct Desktop;

pub struct ApiError(anyhow::Error);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let msg = format!("{:#}", self.0);
        let code = if msg.contains("не найден") || msg.contains("не установлено") { StatusCode::NOT_FOUND } else if msg.contains("имя ") || msg.contains("требует") || msg.contains("зависят") || msg.contains("не объявлена") || msg.contains("недопустимый путь") || msg.contains("за пределы корня") || msg.contains("описание проекта") || msg.contains("раскладка") || msg.contains("политика — объект") || msg.contains("каталог входа") || msg.contains("вход: claude") || msg.contains("вход по ссылке") || msg.contains("state не найден") || msg.contains("пустой код") || msg.contains("обмен кода") || msg.contains("agentd: 400") { StatusCode::BAD_REQUEST } else if msg.contains("agentd не отвечает") { StatusCode::BAD_GATEWAY } else if msg.contains("agentd: 404") { StatusCode::NOT_FOUND } else { StatusCode::INTERNAL_SERVER_ERROR };
        tracing::warn!("api error {}: {msg}", code.as_u16());
        (code, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self { Self(e) }
}
type R<T> = Result<Json<T>, ApiError>;

#[derive(OpenApi)]
#[openapi(
    info(title = "Cloud OS · cloudd API", version = "0.1.0"),
    paths(status, events, list_projects, get_project, apply_project, delete_project, workspace_start, workspace_stop,
          list_apps, install_app, remove_app, app_logs, app_settings, grant, revoke, ca_cert, desktop_settings_get, desktop_settings_put,
          app_overrides_get, app_overrides_put, app_open, monitor_snapshot, widget_feed, widget_weather, widget_geocode, files_roots, files_list, files_read, files_write, files_upload, files_mkdir, files_rename, files_delete),
    components(schemas(ProjectSpec, WorkspaceSpec, Runtime, ProjectView, WorkspaceView, WorkspaceApp, Grant, ComponentHealth, SystemStatus,
                       AppManifest, AppView, RouteSpec, RouteMode, OpenMode, AppKind, Endpoint, EnvSpec, GenerateKind, Hooks, Event, GrantBody, LogsQuery,
                       crate::service::AppOverrides, crate::files::Root, crate::files::Listing, crate::files::Entry, crate::files::WriteBody, crate::files::RenameBody, PathQuery, OpenBody, crate::monitor::Snapshot, crate::monitor::HostSample, crate::backup::BackupConfig, crate::backup::BackupStatus, crate::backup::SnapshotInfo, crate::backup::DatasetInfo, crate::backup::RestorePoint, crate::monitor::DiskInfo, crate::monitor::ContainerStat, crate::monitor::InstanceStat, crate::feeds::Feed, crate::feeds::FeedItem, FeedQuery, WeatherQuery, GeoQuery))
)]
struct ApiDoc;

pub fn router(svc: Service) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/events", get(events))
        .route("/projects", get(list_projects).post(apply_project))
        .route("/projects/{name}", get(get_project).delete(delete_project))
        .route("/projects/{name}/workspace/start", post(workspace_start))
        .route("/projects/{name}/workspace/stop", post(workspace_stop))
        .route("/projects/{name}/grants", post(grant))
        .route("/projects/{name}/grants/{capability}", delete(revoke))
        // Agent Runtime (ADR-006): терминалы workspace через cloudos-agentd; поток терминала — WebSocket-прокси
        .route("/projects/{name}/agentd", get(agentd_health))
        .route("/projects/{name}/agentd/restart", post(agentd_restart))
        .route("/projects/{name}/ptys", get(pty_list).post(pty_create))
        .route("/projects/{name}/ptys/{id}", get(pty_get).delete(pty_delete))
        .route("/projects/{name}/ptys/{id}/resize", post(pty_resize))
        .route("/projects/{name}/ptys/{id}/ws", get(pty_ws))
        .route("/projects/{name}/sessions", get(session_list).post(session_create))
        .route("/projects/{name}/sessions/{id}", get(session_get).delete(session_delete))
        .route("/projects/{name}/sessions/{id}/events", get(session_events))
        .route("/projects/{name}/sessions/{id}/download", get(session_download))
        .route("/projects/{name}/search", get(project_search))
        .route("/projects/{name}/mcp", get(project_mcp).post(project_mcp_set))
        .route("/projects/{name}/sessions/{id}/ws", get(session_ws))
        .route("/projects/{name}/sessions/{id}/prompt", post(session_prompt))
        .route("/projects/{name}/sessions/{id}/permission", post(session_permission))
        .route("/projects/{name}/sessions/{id}/interrupt", post(session_interrupt))
        .route("/projects/{name}/sessions/{id}/mode", post(session_mode))
        .route("/projects/{name}/sessions/{id}/model", post(session_model))
        .route("/projects/{name}/sessions/{id}/tui", post(session_tui).delete(session_tui_stop))
        .route("/projects/{name}/sessions/{id}/effort", post(session_effort))
        .route("/projects/{name}/sessions/{id}/control", post(session_control))
        .route("/projects/{name}/fs/find", get(fs_find))
        .route("/agents/policy", get(agents_policy_get).put(agents_policy_put))
        .route("/projects/{name}/fs", get(fs_list).delete(fs_remove))
        .route("/projects/{name}/fs/file", get(fs_read).put(fs_write))
        .route("/projects/{name}/fs/mkdir", post(fs_mkdir))
        .route("/projects/{name}/fs/rename", post(fs_rename))
        .route("/projects/{name}/git/{*op}", get(git_op).post(git_op_post))
        .route("/projects/{name}/llm/quick", post(llm_quick))
        // каталог движков workspace'а (agentd engines.rs): что установлено, установка в терминале
        .route("/projects/{name}/engines", get(engines_list))
        .route("/projects/{name}/engines/{id}/install", post(engine_install))
        // маршруты к моделям для простого чата (llm.rs): список, модели, потоковый прокси; расход не из agentd — в книгу
        .route("/llm/routes", get(llm_routes))
        .route("/llm/routes/{id}/models", get(llm_route_models))
        .route("/llm/routes/{id}/chat", post(llm_route_chat))
        .route("/llm/routes/{id}/images", post(llm_route_images))
        .route("/usage/record", post(usage_record))
        .route("/projects/{name}/usage", get(project_usage))
        .route("/usage", get(usage_summary))
        .route("/usage/export.csv", get(usage_csv))
        .route("/agents", get(agents_all))
        .route("/accounts", get(accounts_list).post(account_create))
        .route("/accounts/import", post(account_import))
        .route("/accounts/login", post(account_login))
        .route("/accounts/oauth/start", post(oauth_start))
        .route("/accounts/oauth/finish", post(oauth_finish))
        .route("/accounts/oauth/status", get(oauth_status))
        .route("/gate/auth", get(gate_auth))
        .route("/gate/auth/start", post(gate_auth_start))
        .route("/gate/auth/status", get(gate_auth_status))
        .route("/gate/auth/callback", post(gate_auth_callback))
        .route("/gate/auth/login", delete(gate_login_delete).patch(gate_login_status))
        .route("/gate/models", get(gate_models))
        .route("/gate/plugins/install", post(gate_plugin_install))
        .route("/gate/plugins/enabled", post(gate_plugin_enabled))
        .route("/projects/{name}/ptys/{id}/input", post(pty_input))
        .route("/projects/{name}/sessions/{id}/thinking", post(session_thinking))
        .route("/projects/{name}/sessions/{id}/tier", post(session_tier))
        .route("/projects/{name}/sessions/{id}/cache", post(session_cache))
        .route("/accounts/{id}", delete(account_delete))
        .route("/projects/{name}/account", axum::routing::put(project_account_set))
        .route("/projects/{name}/sessions/{id}/account", post(session_account))
        .route("/projects/{name}/sessions/{id}/move", post(session_move))
        .route("/projects/{name}/sessions/{id}/tasks/{task}/stop", post(session_task_stop))
        .route("/projects/{name}/sessions/{id}/fork", post(session_fork))
        .route("/projects/{name}/sessions/{id}/edit", post(session_edit))
        .route("/projects/{name}/sessions/{id}/queue", post(session_queue))
        .route("/projects/{name}/sessions/{id}/queue/{qid}", delete(session_unqueue))
        .route("/projects/{name}/permissions", get(permissions_get).put(permissions_put))
        .route("/agents/notify", get(agents_notify).post(agents_notify_push))
        .route("/agents/layout", get(agents_layout_get).put(agents_layout_put))
        .route("/apps", get(list_apps))
        .route("/apps/{name}/install", post(install_app))
        .route("/apps/{name}", get(get_app).delete(remove_app))
        .route("/apps/{name}/logs", get(app_logs))
        .route("/apps/{name}/icon", get(app_icon))
        .route("/apps/{name}/readme", get(app_readme))
        .route("/apps/{name}/settings", axum::routing::put(app_settings))
        .route("/apps/{name}/settings/{key}/reveal", get(app_setting_reveal))
        .route("/apps/{name}/settings/{key}/rotate", post(app_setting_rotate))
        .route("/apps/{name}/note", get(app_note_get).put(app_note_put))
        .route("/apps/{name}/open", axum::routing::put(app_open))
        .route("/apps/{name}/links", post(app_link))
        .route("/apps/{name}/links/{provider}", delete(app_unlink))
        .route("/apps/{name}/shared", axum::routing::put(app_shared))
        .route("/ca.crt", get(ca_cert))
        .route("/system/credentials/coder/reveal", get(coder_credentials))
        .route("/apps/{name}/repair", post(repair_app))
        .route("/apps/{name}/snapshots", get(app_snapshots))
        .route("/apps/{name}/restore/{snap}", post(app_restore))
        .route("/backups", get(backups_get).put(backups_put))
        .route("/backups/snapshot", post(backups_snapshot))
        .route("/backups/run", post(backups_run))
        .route("/apps/{name}/overrides", get(app_overrides_get).put(app_overrides_put))
        .route("/monitor", get(monitor_snapshot))
        .route("/widgets/feed", get(widget_feed))
        .route("/widgets/weather", get(widget_weather))
        .route("/widgets/geocode", get(widget_geocode))
        .route("/files/roots", get(files_roots))
        .route("/files/{root}/list", get(files_list))
        .route("/files/{root}/read", get(files_read))
        .route("/files/{root}/write", axum::routing::put(files_write))
        .route("/files/{root}/upload", post(files_upload))
        .route("/files/{root}/mkdir", post(files_mkdir))
        .route("/files/{root}/rename", post(files_rename))
        .route("/files/{root}/delete", delete(files_delete))
        .route("/settings/desktop", get(desktop_settings_get).put(desktop_settings_put))
        // мастер первого запуска (setup.rs) и обновления (updates.rs)
        .route("/setup", get(setup_get))
        .route("/setup/tailscale/login", post(setup_login))
        .route("/setup/edge-host", post(setup_host))
        .route("/setup/done", axum::routing::put(setup_done))
        .route("/updates", get(updates_get))
        .route("/updates/check", post(updates_check))
        .route("/updates/channel", axum::routing::put(updates_channel))
        .route("/updates/download", post(updates_download))
        .route("/updates/upload", post(updates_upload))
        .route("/updates/apply", post(updates_apply))
        .route("/updates/rollback", post(updates_rollback))
        .route("/updates/packages/{file}", delete(updates_package_delete))
        .route("/updates/catalog", post(updates_catalog))
        .route("/updates/components/{project}", post(update_component))
        .route("/apps/{name}/update", post(update_app))
        .route("/openapi.json", get(|| async { Json(ApiDoc::openapi()) }));
    Router::new()
        .nest("/api", api)
        .fallback(desktop)
        .layer(axum::extract::DefaultBodyLimit::max(crate::files::MAX_UPLOAD))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(svc)
}

async fn desktop(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Desktop::get(path).or_else(|| if path.contains('.') { None } else { Desktop::get("index.html") }) {
        Some(f) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
            ([(header::CONTENT_TYPE, mime.as_ref()), (header::CACHE_CONTROL, cache)], f.data.into_owned()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

#[utoipa::path(get, path = "/api/status", responses((status = 200, body = SystemStatus)))]
async fn status(State(s): State<Service>, headers: axum::http::HeaderMap) -> R<SystemStatus> {
    // присутствие: видимая вкладка Desktop опрашивает статус с этим заголовком; без него полторы минуты — и человек «ушёл»
    if headers.get("x-desktop-presence").and_then(|v| v.to_str().ok()) == Some("visible") { s.agents.touch_presence(); }
    Ok(Json(s.status().await?))
}

#[derive(Deserialize)]
struct DaysQuery { #[serde(default = "d_days")] days: u64 }
fn d_days() -> u64 { 30 }
/// Книга расходов LLM за последние `days` дней: итог, по проектам, по моделям, по дням.
async fn usage_summary(State(s): State<Service>, Query(q): Query<DaysQuery>) -> R<UsageSummary> {
    let since = crate::agents::now_ms().saturating_sub(q.days.clamp(1, 3650) * 86_400_000);
    Ok(Json(s.store.usage_summary(since)?))
}
/// Книга расходов в CSV (срез 34): строка на ход — время ISO, workspace, сессия, движок, модель, токены, стоимость; UTF-8 с BOM.
async fn usage_csv(State(s): State<Service>, Query(q): Query<DaysQuery>) -> Result<axum::response::Response, ApiError> {
    let since = crate::agents::now_ms().saturating_sub(q.days.clamp(1, 3650) * 86_400_000);
    let rows = s.store.usage_rows(since)?;
    let esc = |v: &str| if v.contains([',', '"', '\n']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.to_string() };
    let mut out = String::from("\u{feff}time,workspace,session,agent,model,input,output,cache_read,cache_create,cost_usd,duration_ms,source\n");
    for r in rows {
        let ts = time::OffsetDateTime::from_unix_timestamp((r.ts / 1000) as i64).ok().and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok()).unwrap_or_default();
        out.push_str(&format!("{},{},{},{},{},{},{},{},{},{:.6},{},{}\n", ts, esc(&r.project), r.session, esc(&r.agent), esc(r.model.as_deref().unwrap_or("")), r.input, r.output, r.cache_read, r.cache_create, r.cost_usd, r.duration_ms.map(|d| d.to_string()).unwrap_or_default(), esc(&r.source)));
    }
    let name = format!("usage-{}d.csv", q.days.clamp(1, 3650));
    Ok(([(axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (axum::http::header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], out).into_response())
}
/// Подписка на уведомления ядра с телефона: сервер ntfy, тема, пользователь, пароль (секрет — только по этому запросу).
async fn agents_notify(State(s): State<Service>) -> R<serde_json::Value> { Ok(Json(s.notify_settings().await?)) }
/// Уведомление от сервиса «Агенты» (рутины): событие в ленту всегда, push в ntfy — по правилу присутствия Desktop
/// (`force` — всегда). Тело: {project, title, body, tag?, force?}.
async fn agents_notify_push(State(s): State<Service>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> {
    let f = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let (project, title, text) = (f("project"), f("title"), f("body"));
    if title.is_empty() { return Err(anyhow::anyhow!("уведомление — объект {{project, title, body, tag?, force?}}").into()); }
    let tag = { let t = f("tag"); if t.is_empty() { "bell".to_string() } else { t } };
    let project = if project.is_empty() { "agents".to_string() } else { project };
    let force = body.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
    let pushed = if force {
        match s.ntfy_push(&title, &text, &tag).await { Ok(()) => { let _ = s.store.event("agent", &project, &format!("{title}: {text}")); true } Err(e) => { let _ = s.store.event("agent", &project, &format!("{title}: {text} (push не ушёл: {e:#})")); false } }
    } else { s.notify_agent(&project, &title, &text, &tag).await; s.agents.desktop_away() };
    Ok(Json(serde_json::json!({ "ok": true, "pushed": pushed, "away": s.agents.desktop_away() })))
}
/// Кэш сводок по сессиям агентов всех запущенных workspace'ов и присутствие Desktop (push идёт, когда `away`).
async fn agents_all(State(s): State<Service>) -> R<serde_json::Value> {
    let projects = s.agents.cache.read().unwrap().clone();
    Ok(Json(serde_json::json!({ "away": s.agents.desktop_away(), "presence_seen_ms": s.agents.presence_seen_ms(), "projects": projects })))
}

#[utoipa::path(get, path = "/api/events", responses((status = 200, body = Vec<Event>)))]
async fn events(State(s): State<Service>) -> R<Vec<Event>> { Ok(Json(s.store.recent_events(100)?)) }

#[utoipa::path(get, path = "/api/projects", responses((status = 200, body = Vec<ProjectView>)))]
async fn list_projects(State(s): State<Service>, Query(q): Q) -> R<Vec<ProjectView>> {
    // служебные проекты (`hidden`, например scratch быстрых чатов) — только по ?all=1
    let all = matches!(q.get("all").map(String::as_str), Some("1") | Some("true"));
    Ok(Json(s.projects().await?.into_iter().filter(|p| all || !p.spec.hidden).collect()))
}

#[utoipa::path(get, path = "/api/projects/{name}", responses((status = 200, body = ProjectView)))]
async fn get_project(State(s): State<Service>, Path(name): Path<String>) -> R<ProjectView> { Ok(Json(s.project(&name).await?)) }

/// Создать или изменить проект = записать `project.yaml` и применить.
#[utoipa::path(post, path = "/api/projects", request_body = ProjectSpec, responses((status = 200, body = ProjectView)))]
async fn apply_project(State(s): State<Service>, Json(spec): Json<ProjectSpec>) -> R<ProjectView> { Ok(Json(detached(async move { s.project_apply(spec).await }).await?)) }

#[utoipa::path(delete, path = "/api/projects/{name}", responses((status = 204)))]
async fn delete_project(State(s): State<Service>, Path(name): Path<String>) -> Result<StatusCode, ApiError> {
    detached(async move { s.project_delete(&name).await }).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/projects/{name}/workspace/start", responses((status = 200, body = ProjectView)))]
async fn workspace_start(State(s): State<Service>, Path(name): Path<String>) -> R<ProjectView> { Ok(Json(detached(async move { s.workspace_transition(&name, "start").await }).await?)) }

#[utoipa::path(post, path = "/api/projects/{name}/workspace/stop", responses((status = 200, body = ProjectView)))]
async fn workspace_stop(State(s): State<Service>, Path(name): Path<String>) -> R<ProjectView> { Ok(Json(detached(async move { s.workspace_transition(&name, "stop").await }).await?)) }

#[derive(Deserialize, ToSchema)]
pub struct GrantBody { pub capability: String, /// какой провайдер использовать (по умолчанию — которым уже пользуются другие проекты, иначе первый по имени)
    #[serde(default)] pub app: Option<String> }

#[utoipa::path(post, path = "/api/projects/{name}/grants", request_body = GrantBody, responses((status = 200, body = ProjectView)))]
async fn grant(State(s): State<Service>, Path(name): Path<String>, Json(b): Json<GrantBody>) -> R<ProjectView> { Ok(Json(detached(async move { s.grant(&name, &b.capability, b.app.as_deref()).await }).await?)) }

#[utoipa::path(delete, path = "/api/projects/{name}/grants/{capability}", responses((status = 200, body = ProjectView)))]
async fn revoke(State(s): State<Service>, Path((name, cap)): Path<(String, String)>) -> R<ProjectView> { Ok(Json(detached(async move { s.revoke(&name, &cap).await }).await?)) }

#[utoipa::path(get, path = "/api/apps", responses((status = 200, body = Vec<AppView>)))]
async fn list_apps(State(s): State<Service>) -> R<Vec<AppView>> { Ok(Json(s.apps().await?)) }

#[utoipa::path(post, path = "/api/apps/{name}/install", responses((status = 200, body = AppView)))]
async fn install_app(State(s): State<Service>, Path(name): Path<String>) -> R<AppView> { Ok(Json(detached(async move { s.app_install(&name).await }).await?)) }

#[derive(Deserialize, ToSchema)]
pub struct RemoveQuery {
    /// Удалить вместе с данными в tank/apps, секретами и переопределениями.
    pub purge: Option<bool>,
}

#[utoipa::path(delete, path = "/api/apps/{name}", responses((status = 204)))]
async fn remove_app(State(s): State<Service>, Path(name): Path<String>, Query(q): Query<RemoveQuery>) -> Result<StatusCode, ApiError> {
    let purge = q.purge.unwrap_or(false);
    detached(async move { s.app_remove(&name, purge).await }).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Долгие изменяющие операции выполняются в отдельной задаче: если клиент оборвал соединение (закрыл вкладку, сменилась сеть),
/// axum отменяет future обработчика, и операция осталась бы на середине — стек снесён, запись и данные остались.
// ---- Agent Runtime: agentd и терминалы (ADR-006) ----
/// Перезапуск agentd в workspace'е (срез 38): живые беседы возвращаются сами (автовозобновление agentd).
async fn agentd_restart(State(s): State<Service>, Path(name): Path<String>) -> R<serde_json::Value> {
    let inst = s.workspace_instance(&name).await;
    s.incus.exec(&inst, &["systemctl", "restart", "cloudos-agentd"]).await.map_err(|e| anyhow::anyhow!("перезапуск agentd в {name}: {e:#}"))?;
    s.store.event("agentd", &name, "перезапуск agentd (запрос API)")?;
    Ok(Json(serde_json::json!({ "restarted": true })))
}
async fn agentd_health(State(s): State<Service>, Path(name): Path<String>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.health().await?)) }
async fn pty_list(State(s): State<Service>, Path(name): Path<String>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.ptys().await?)) }
async fn pty_create(State(s): State<Service>, Path(name): Path<String>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.pty_create(&body).await?)) }
async fn pty_get(State(s): State<Service>, Path((name, id)): Path<(String, String)>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.pty(&id).await?)) }
async fn pty_resize(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.pty_resize(&id, &body).await?)) }
async fn pty_delete(State(s): State<Service>, Path((name, id)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    Ok(StatusCode::from_u16(s.agentd_client(&name).await?.pty_delete(&id).await?).unwrap_or(StatusCode::NO_CONTENT))
}

/// Desktop ↔ cloudd ↔ agentd: кадры пересылаются как есть в обе стороны (протокол терминала v1 описан в agentd).
async fn pty_ws(State(s): State<Service>, Path((name, id)): Path<(String, String)>, ws: axum::extract::ws::WebSocketUpgrade) -> Result<Response, ApiError> {
    agentd_ws(&s, &name, &format!("/v1/ptys/{id}/ws"), ws).await
}

fn query_string(q: &std::collections::HashMap<String, String>) -> String {
    if q.is_empty() { return String::new() }
    let parts: Vec<String> = q.iter().filter(|(k, _)| k.as_str() != "token").map(|(k, v)| format!("{}={}", k, urlencoding::encode(v))).collect();
    format!("?{}", parts.join("&"))
}

/// Общий WebSocket-прокси к agentd: терминалы и потоки событий сессий.
async fn agentd_ws(s: &Service, name: &str, path: &str, ws: axum::extract::ws::WebSocketUpgrade) -> Result<Response, ApiError> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let c = s.agentd_client(name).await?;
    let mut req = c.ws_url_for(path).into_client_request().map_err(|e| anyhow::anyhow!("agentd ws: {e}"))?;
    req.headers_mut().insert("authorization", format!("Bearer {}", c.token()).parse().map_err(|_| anyhow::anyhow!("токен agentd"))?);
    let (upstream, _) = tokio_tungstenite::connect_async(req).await.map_err(|e| anyhow::anyhow!("agentd не отвечает: {e}"))?;
    Ok(ws.on_upgrade(move |sock| pty_relay(sock, upstream)))
}

// ---- сессии агентов (ADR-006): прокси к agentd один в один ----
type Q = Query<std::collections::HashMap<String, String>>;
async fn session_list(State(s): State<Service>, Path(name): Path<String>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.get("/v1/sessions").await?)) }
/// Создание сессии: аккаунт из тела (`account`, `""` — без аккаунта) или умолчание проекта материализуется в workspace,
/// его переменные и `unset` уезжают в agentd вместе с запросом.
async fn session_create(State(s): State<Service>, Path(name): Path<String>, Json(mut body): Json<serde_json::Value>) -> R<serde_json::Value> {
    s.resolve_session_account(&name, &mut body).await?;
    Ok(Json(s.agentd_client(&name).await?.post("/v1/sessions", &body).await?))
}

// ---- аккаунты агентов (ADR-006, решение 6) ----
/// Раскладка холста «Агентов» (ADR-006, этап 2): kv `agents.layout`, общая для всех устройств владельца.
async fn agents_layout_get(State(s): State<Service>) -> R<serde_json::Value> {
    Ok(Json(s.store.get_kv("agents.layout")?.unwrap_or(serde_json::json!({}))))
}
async fn agents_layout_put(State(s): State<Service>, Json(body): Json<serde_json::Value>) -> Result<StatusCode, ApiError> {
    if !body.is_object() { return Err(anyhow::anyhow!("раскладка — объект {{cards, cols}}").into()); }
    if body.to_string().len() > 64 * 1024 { return Err(anyhow::anyhow!("раскладка слишком большая (64 КБ)").into()); }
    s.store.put_kv("agents.layout", &body)?;
    Ok(StatusCode::NO_CONTENT)
}
/// Политика workspace'ов для Claude Code: разрешён ли режим обхода разрешений (managed-settings). Смена применяется ко всем
/// запущенным workspace'ам сразу и к новым при деплое agentd; живые процессы Claude читают файл при старте — им нужен перезапуск.
async fn agents_policy_get(State(s): State<Service>) -> R<serde_json::Value> {
    Ok(Json(serde_json::json!({ "bypass": crate::agentd::policy_bypass(&s.store) })))
}
async fn agents_policy_put(State(s): State<Service>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> {
    let bypass = body.get("bypass").and_then(|b| b.as_bool()).ok_or_else(|| anyhow::anyhow!("политика — объект {{bypass: bool}}"))?;
    s.store.put_kv("agents.policy", &serde_json::json!({ "bypass": bypass }))?;
    let changed = s.apply_agents_policy().await?;
    Ok(Json(serde_json::json!({ "bypass": bypass, "applied": changed })))
}
async fn accounts_list(State(s): State<Service>) -> R<serde_json::Value> {
    let mut defaults = serde_json::Map::new();
    for st in s.store.list_projects()? {
        if let Some(id) = s.project_account(&st.name)? { defaults.insert(st.name, serde_json::Value::String(id)); }
    }
    Ok(Json(serde_json::json!({ "accounts": s.accounts()?, "defaults": defaults })))
}
async fn account_create(State(s): State<Service>, Json(body): Json<crate::accounts::AccountCreate>) -> R<crate::accounts::Account> { Ok(Json(s.account_create(body)?)) }
async fn account_import(State(s): State<Service>, Json(body): Json<crate::accounts::AccountImport>) -> R<crate::accounts::Account> { Ok(Json(s.account_import(body).await?)) }
async fn account_delete(State(s): State<Service>, Path(id): Path<String>) -> Result<StatusCode, ApiError> { s.account_delete(&id)?; Ok(StatusCode::NO_CONTENT) }
#[derive(Deserialize)]
struct OauthStartBody { provider: String }
/// Вход Claude по ссылке и коду: ссылка с PKCE → владелец входит на своём устройстве → `code#state` → токены → аккаунт.
async fn oauth_start(State(s): State<Service>, Json(b): Json<OauthStartBody>) -> R<serde_json::Value> { Ok(Json(s.oauth_start(&b.provider).await?)) }
#[derive(Deserialize)]
struct StateQuery { state: String }
async fn oauth_status(State(s): State<Service>, Query(q): Query<StateQuery>) -> R<serde_json::Value> { Ok(Json(s.oauth_status(&q.state).await?)) }
// ---- провайдеры через gate: OAuth-входы в прокси провайдера llm (Antigravity, Kimi, xAI, Codex, Claude, плагины) ----
async fn gate_auth(State(s): State<Service>) -> R<serde_json::Value> { Ok(Json(s.gate_auth_overview().await?)) }
#[derive(Deserialize)]
struct GateStartBody { app: String, provider: String }
async fn gate_auth_start(State(s): State<Service>, Json(b): Json<GateStartBody>) -> R<serde_json::Value> { Ok(Json(s.gate_auth_start(&b.app, &b.provider).await?)) }
#[derive(Deserialize)]
struct GateStateQuery { app: String, state: String }
async fn gate_auth_status(State(s): State<Service>, Query(q): Query<GateStateQuery>) -> R<serde_json::Value> { Ok(Json(s.gate_auth_status(&q.app, &q.state).await?)) }
#[derive(Deserialize)]
struct GateCallbackBody { app: String, state: String, redirect_url: String }
async fn gate_auth_callback(State(s): State<Service>, Json(b): Json<GateCallbackBody>) -> R<serde_json::Value> { Ok(Json(s.gate_auth_callback(&b.app, &b.state, &b.redirect_url).await?)) }
#[derive(Deserialize)]
struct GateLoginQuery { app: String, name: String }
async fn gate_login_delete(State(s): State<Service>, Query(q): Query<GateLoginQuery>) -> Result<StatusCode, ApiError> { s.gate_login_delete(&q.app, &q.name).await?; Ok(StatusCode::NO_CONTENT) }
#[derive(Deserialize)]
struct GateLoginStatusBody { app: String, name: String, disabled: bool }
async fn gate_login_status(State(s): State<Service>, Json(b): Json<GateLoginStatusBody>) -> R<serde_json::Value> { Ok(Json(s.gate_login_disabled(&b.app, &b.name, b.disabled).await?)) }
#[derive(Deserialize)]
struct GateModelsQuery { #[serde(default)] app: Option<String>, #[serde(default)] project: Option<String> }
async fn gate_models(State(s): State<Service>, Query(q): Query<GateModelsQuery>) -> R<serde_json::Value> { Ok(Json(s.gate_models(q.app.as_deref(), q.project.as_deref()).await?)) }
#[derive(Deserialize)]
struct GatePluginBody { app: String, id: String, #[serde(default)] enabled: Option<bool> }
async fn gate_plugin_install(State(s): State<Service>, Json(b): Json<GatePluginBody>) -> R<serde_json::Value> { Ok(Json(s.gate_plugin_install(&b.app, &b.id).await?)) }
async fn gate_plugin_enabled(State(s): State<Service>, Json(b): Json<GatePluginBody>) -> R<serde_json::Value> { Ok(Json(s.gate_plugin_enabled(&b.app, &b.id, b.enabled.unwrap_or(true)).await?)) }
async fn oauth_finish(State(s): State<Service>, Json(b): Json<crate::accounts::OauthFinish>) -> R<crate::accounts::Account> { Ok(Json(s.oauth_finish(b).await?)) }
async fn pty_input(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> Result<StatusCode, ApiError> {
    s.agentd_client(&name).await?.post(&format!("/v1/ptys/{id}/input"), &body).await?; Ok(StatusCode::NO_CONTENT)
}
async fn session_thinking(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/thinking"), &body).await?)) }
async fn session_tier(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/tier"), &body).await?)) }
async fn session_cache(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/cache"), &body).await?)) }
#[derive(Deserialize)]
struct LoginBody { provider: String, project: String }
/// Вход в подписку из «Агентов» без браузера на стенде: терминал в workspace с `claude auth login` / `codex login --device-auth`
/// и отдельным каталогом конфигурации; CLI печатает ссылку, владелец открывает её на своём устройстве и вставляет код обратно.
/// После выхода терминала интерфейс импортирует вход из этого каталога (`/accounts/import {dir}`).
async fn account_login(State(s): State<Service>, Json(b): Json<LoginBody>) -> R<serde_json::Value> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (dir, cmd, title) = match b.provider.as_str() {
        "claude" => { let d = format!("/home/coder/.cloudos/logins/claude-{stamp}"); (d.clone(), format!("mkdir -p {d} && export CLAUDE_CONFIG_DIR={d} && echo 'Вход в Claude: откройте ссылку ниже на любом устройстве, войдите и вставьте код сюда.' && claude auth login; echo; echo \"[выход: $?] закройте окно — вход будет импортирован\"; sleep 1"), "вход Claude") }
        "codex" => { let d = format!("/home/coder/.cloudos/logins/codex-{stamp}"); (d.clone(), format!("mkdir -p {d} && export CODEX_HOME={d} && echo 'Вход в Codex: откройте ссылку ниже на любом устройстве и введите код.' && codex login --device-auth; echo; echo \"[выход: $?] закройте окно — вход будет импортирован\"; sleep 1"), "вход Codex") }
        other => return Err(anyhow::anyhow!("вход: claude | codex, получено {other}").into()),
    };
    let body = serde_json::json!({ "cmd": ["bash", "-lic", cmd], "cwd": "/home/coder", "title": title });
    let pty = s.agentd_client(&b.project).await?.pty_create(&body).await?;
    Ok(Json(serde_json::json!({ "pty": pty.get("id"), "dir": dir, "project": b.project, "provider": b.provider })))
}
#[derive(Deserialize)]
struct AccountBody { account: Option<String> }
async fn project_account_set(State(s): State<Service>, Path(name): Path<String>, Json(b): Json<AccountBody>) -> Result<StatusCode, ApiError> {
    s.set_project_account(&name, b.account.as_deref().filter(|a| !a.is_empty()))?;
    Ok(StatusCode::NO_CONTENT)
}
/// Переназначить аккаунт работающей сессии: остановить, дождаться выхода, поднять resume'ом с новым окружением (журнал тот же).
async fn session_account(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(b): Json<AccountBody>) -> R<serde_json::Value> {
    let c = s.agentd_client(&name).await?;
    let alive = c.get(&format!("/v1/sessions/{id}")).await?.pointer("/info/alive").and_then(|v| v.as_bool()).unwrap_or(false);
    if alive {
        c.delete(&format!("/v1/sessions/{id}")).await?;
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if c.get(&format!("/v1/sessions/{id}")).await?.pointer("/info/alive").and_then(|v| v.as_bool()) == Some(false) { break }
        }
    }
    let mut body = serde_json::json!({ "resume": id, "reassign": true, "account": b.account.clone().unwrap_or_default(), "env": {}, "unset": [] });
    s.resolve_session_account(&name, &mut body).await?;
    if body.get("account").and_then(|v| v.as_str()) == Some("") { body["account"] = serde_json::Value::Null; }
    Ok(Json(c.post("/v1/sessions", &body).await?))
}
async fn session_get(State(s): State<Service>, Path((name, id)): Path<(String, String)>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.get(&format!("/v1/sessions/{id}")).await?)) }
/// MCP-серверы workspace'а (срез 36) — список и раскладка движкам у agentd.
async fn project_mcp(State(s): State<Service>, Path(name): Path<String>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.get("/v1/mcp").await?)) }
async fn project_mcp_set(State(s): State<Service>, Path(name): Path<String>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post_slow("/v1/mcp", &body, 120).await?)) }
/// Поиск по беседам workspace'а (срез 34) — у agentd, query как есть.
async fn project_search(State(s): State<Service>, Path(name): Path<String>, Query(q): Q) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.get(&format!("/v1/search{}", query_string(&q))).await?)) }
/// Выгрузка беседы для чтения (Markdown/JSON) — тело и заголовки agentd как есть.
async fn session_download(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Query(q): Q) -> Result<axum::response::Response, ApiError> {
    let (status, headers, body) = s.agentd_client(&name).await?.raw_get(&format!("/v1/sessions/{id}/download{}", query_string(&q))).await?;
    let mut out = axum::response::Response::new(axum::body::Body::from(body));
    *out.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    for k in ["content-type", "content-disposition"] { if let Some(v) = headers.get(k) { out.headers_mut().insert(axum::http::header::HeaderName::from_static(if k == "content-type" { "content-type" } else { "content-disposition" }), v.clone()); } }
    Ok(out)
}
async fn session_events(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Query(q): Q) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.get(&format!("/v1/sessions/{id}/events{}", query_string(&q))).await?)) }
async fn session_prompt(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/prompt"), &body).await?)) }
async fn session_permission(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/permission"), &body).await?)) }
async fn session_interrupt(State(s): State<Service>, Path((name, id)): Path<(String, String)>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/interrupt"), &serde_json::json!({})).await?)) }
async fn session_mode(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/mode"), &body).await?)) }
async fn session_model(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/model"), &body).await?)) }
// Терминальный режим (как в CloudCLI): родной TUI агента на той же беседе в PTY сессии; POST подключает (headless-процесс
// останавливается), DELETE отключает. Поток терминала — обычный /ptys/{id}/ws.
async fn session_tui(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/tui"), &body).await?)) }
async fn session_effort(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/effort"), &body).await?)) }
// Запрос к провайдеру для композера (контекст, подсказки файлов, usage): Claude control_request / Codex JSON-RPC
async fn session_control(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/control"), &body).await?)) }
async fn fs_find(State(s): State<Service>, Path(name): Path<String>, axum::extract::RawQuery(q): axum::extract::RawQuery) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.get(&format!("/v1/fs/find?{}", q.unwrap_or_default())).await?)) }
async fn session_tui_stop(State(s): State<Service>, Path((name, id)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    Ok(StatusCode::from_u16(s.agentd_client(&name).await?.delete(&format!("/v1/sessions/{id}/tui")).await?).unwrap_or(StatusCode::NO_CONTENT))
}

// Файлы и git workspace'а (ADR-006, рендеринги Files и Git карточки): прокси к agentd с тем же query.
async fn fs_list(State(s): State<Service>, Path(name): Path<String>, axum::extract::RawQuery(q): axum::extract::RawQuery) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.get(&format!("/v1/fs?{}", q.unwrap_or_default())).await?)) }
async fn fs_remove(State(s): State<Service>, Path(name): Path<String>, axum::extract::RawQuery(q): axum::extract::RawQuery) -> Result<StatusCode, ApiError> {
    Ok(StatusCode::from_u16(s.agentd_client(&name).await?.delete(&format!("/v1/fs?{}", q.unwrap_or_default())).await?).unwrap_or(StatusCode::NO_CONTENT))
}
async fn fs_read(State(s): State<Service>, Path(name): Path<String>, axum::extract::RawQuery(q): axum::extract::RawQuery) -> Result<Response, ApiError> {
    let (_, h, body) = s.agentd_client(&name).await?.raw_get(&format!("/v1/fs/file?{}", q.unwrap_or_default())).await?;
    let mut resp = body.into_response();
    for k in [header::CONTENT_TYPE, header::CONTENT_DISPOSITION, header::CACHE_CONTROL] {
        if let Some(v) = h.get(&k) { resp.headers_mut().insert(k, v.clone()); }
    }
    Ok(resp)
}
async fn fs_write(State(s): State<Service>, Path(name): Path<String>, axum::extract::RawQuery(q): axum::extract::RawQuery, body: axum::body::Bytes) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.raw_put(&format!("/v1/fs/file?{}", q.unwrap_or_default()), body).await?)) }
async fn fs_mkdir(State(s): State<Service>, Path(name): Path<String>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post("/v1/fs/mkdir", &body).await?)) }
async fn fs_rename(State(s): State<Service>, Path(name): Path<String>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post("/v1/fs/rename", &body).await?)) }
async fn git_op(State(s): State<Service>, Path((name, op)): Path<(String, String)>, axum::extract::RawQuery(q): axum::extract::RawQuery) -> R<serde_json::Value> {
    if !matches!(op.as_str(), "status" | "diff" | "log" | "branches" | "worktrees") { return Err(anyhow::anyhow!("git: не найдено — {op}").into()); }
    Ok(Json(s.agentd_client(&name).await?.get(&format!("/v1/git/{op}?{}", q.unwrap_or_default())).await?))
}
/// Действия git-панели (agentd fs.rs): индекс, коммит, ветки, remote, worktrees, подсказка сообщения; коммит, push и слияние — в ленту событий.
async fn git_op_post(State(s): State<Service>, Path((name, op)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> {
    if !matches!(op.as_str(), "init" | "remote" | "stage" | "unstage" | "discard" | "commit" | "checkout" | "fetch" | "pull" | "push" | "worktrees/add" | "worktrees/remove" | "worktrees/merge" | "suggest") { return Err(anyhow::anyhow!("git: не найдено — {op}").into()); }
    let r = if op == "suggest" { s.agentd_client(&name).await?.post_slow(&format!("/v1/git/{op}"), &body, 150).await? } else { s.agentd_client(&name).await?.post(&format!("/v1/git/{op}"), &body).await? };
    let short = |v: &serde_json::Value| v.get("short").and_then(|x| x.as_str()).unwrap_or("").to_string();
    match op.as_str() {
        "commit" => { let _ = s.store.event("agent", &name, &format!("git: коммит {} — {}", short(&r), r.get("subject").and_then(|x| x.as_str()).unwrap_or(""))); }
        "push" => { let _ = s.store.event("agent", &name, "git: push"); }
        "worktrees/merge" => { let _ = s.store.event("agent", &name, &format!("git: слита ветка {} ({})", r.get("branch").and_then(|x| x.as_str()).unwrap_or("?"), r.get("commit").map(short).unwrap_or_default())); }
        _ => {}
    }
    Ok(Json(r))
}
/// Маршруты к моделям (llm.rs): без адресов и ключей.
async fn llm_routes(State(s): State<Service>) -> R<serde_json::Value> { Ok(Json(serde_json::json!({ "routes": s.llm_routes().await? }))) }
async fn llm_route_models(State(s): State<Service>, Path(id): Path<String>) -> R<serde_json::Value> { Ok(Json(s.llm_models(&id).await?)) }
/// Потоковый прокси chat/completions: статус и тип ответа upstream — как есть, тело — потоком.
/// Картинка на маршруте (срез 37): тело как у OpenAI `/images/generations`, ответ провайдера как есть.
async fn llm_route_images(State(s): State<Service>, Path(id): Path<String>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.llm_images(&id, &body).await?)) }
async fn llm_route_chat(State(s): State<Service>, Path(id): Path<String>, Json(body): Json<serde_json::Value>) -> Result<Response, ApiError> {
    let up = s.llm_chat(&id, &body).await?;
    let status = up.status();
    let ctype = up.headers().get(header::CONTENT_TYPE).cloned();
    let mut resp = Response::new(axum::body::Body::from_stream(up.bytes_stream()));
    *resp.status_mut() = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    if let Some(c) = ctype { resp.headers_mut().insert(header::CONTENT_TYPE, c); }
    resp.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    Ok(resp)
}
/// Расход не из agentd (простой чат сервиса «Агенты»): строка книги `llm_usage` с источником `api`; ключ project+session+seq.
async fn usage_record(State(s): State<Service>, Json(b): Json<serde_json::Value>) -> R<serde_json::Value> {
    let str_of = |k: &str| b.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let num = |k: &str| b.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let (project, session) = (str_of("project").filter(|p| !p.is_empty()).ok_or_else(|| anyhow::anyhow!("usage: нет project"))?, str_of("session").filter(|p| !p.is_empty()).ok_or_else(|| anyhow::anyhow!("usage: нет session"))?);
    // ts книги — миллисекунды (как у agentd); секунды из тела переводятся
    let ts = b.get("ts").and_then(|v| v.as_u64()).map(|t| if t < 1_000_000_000_000 { t * 1000 } else { t }).unwrap_or_else(crate::agents::now_ms);
    let row = UsageRow { project, session, seq: num("seq"), ts,
        agent: str_of("agent").unwrap_or_else(|| "api".into()), model: str_of("model"), input: num("input"), output: num("output"), cache_read: num("cache_read"), cache_create: num("cache_create"),
        cost_usd: b.get("cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0), duration_ms: b.get("duration_ms").and_then(|v| v.as_u64()), num_turns: None, source: str_of("source").unwrap_or_else(|| "api".into()) };
    let inserted = s.store.insert_usage(&row)?;
    Ok(Json(serde_json::json!({ "inserted": inserted })))
}
/// Движки workspace'а (Claude Code, Codex, Gemini CLI, pi…): что установлено и версии; установка — PTY с npm, как терминал.
async fn engines_list(State(s): State<Service>, Path(name): Path<String>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post_slow_get("/v1/engines", 40).await?)) }
async fn engine_install(State(s): State<Service>, Path((name, id)): Path<(String, String)>) -> R<serde_json::Value> {
    let r = s.agentd_client(&name).await?.post(&format!("/v1/engines/{id}/install"), &serde_json::json!({})).await?;
    let _ = s.store.event("agent", &name, &format!("установка движка {id}"));
    Ok(Json(r))
}
/// Быстрый одноразовый вызов модели внутри workspace (agentd llm.rs): служебные задачи сервиса «Агенты» — рефлексия памяти, черновик брифа.
async fn llm_quick(State(s): State<Service>, Path(name): Path<String>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> {
    let secs = body.get("timeout_secs").and_then(|t| t.as_u64()).unwrap_or(120).clamp(10, 300) + 15;
    Ok(Json(s.agentd_client(&name).await?.post_slow("/v1/llm/quick", &body, secs).await?))
}
async fn session_delete(State(s): State<Service>, Path((name, id)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    Ok(StatusCode::from_u16(s.agentd_client(&name).await?.delete(&format!("/v1/sessions/{id}")).await?).unwrap_or(StatusCode::NO_CONTENT))
}
/// Форк беседы, правка отправленного сообщения и очередь сообщений — прокси в agentd (см. agentd fork.rs / session.rs).
async fn session_fork(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> {
    let r = s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/fork"), &body).await?;
    let _ = s.store.event("agent", &name, &format!("форк беседы {} → {}", &id[..8.min(id.len())], r.get("id").and_then(|i| i.as_str()).map(|i| &i[..8.min(i.len())]).unwrap_or("?")));
    Ok(Json(r))
}
async fn session_edit(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/edit"), &body).await?)) }
async fn session_queue(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/queue"), &body).await?)) }
async fn session_unqueue(State(s): State<Service>, Path((name, id, qid)): Path<(String, String, String)>) -> Result<StatusCode, ApiError> {
    Ok(StatusCode::from_u16(s.agentd_client(&name).await?.delete(&format!("/v1/sessions/{id}/queue/{qid}")).await?).unwrap_or(StatusCode::NO_CONTENT))
}
/// Фоновая задача Claude Code (Bash в фоне, Agent, Workflow…): остановить через agentd (control `stop_task`).
async fn session_task_stop(State(s): State<Service>, Path((name, id, task)): Path<(String, String, String)>) -> R<serde_json::Value> {
    Ok(Json(s.agentd_client(&name).await?.post(&format!("/v1/sessions/{id}/tasks/{task}/stop"), &serde_json::json!({})).await?))
}
/// Правила инструментов Claude Code в settings workspace'а (области user | project, см. agentd perms.rs): чтение и запись.
async fn permissions_get(State(s): State<Service>, Path(name): Path<String>, axum::extract::RawQuery(q): axum::extract::RawQuery) -> R<serde_json::Value> {
    Ok(Json(s.agentd_client(&name).await?.get(&format!("/v1/permissions?{}", q.unwrap_or_default())).await?))
}
async fn permissions_put(State(s): State<Service>, Path(name): Path<String>, axum::extract::RawQuery(q): axum::extract::RawQuery, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> {
    Ok(Json(s.agentd_client(&name).await?.put(&format!("/v1/permissions?{}", q.unwrap_or_default()), &body).await?))
}
/// Перенос сессии в другой workspace (ADR-006, «поднять в проект»): архив от agentd-источника → agentd-приёмник (тот же id,
/// cwd — дом проекта, файлы чата по прежнему пути), затем исходная запись удаляется. Тело: {to, cwd?}.
async fn session_move(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Json(body): Json<serde_json::Value>) -> R<serde_json::Value> {
    let to = body.get("to").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
    if to.is_empty() { return Err(anyhow::anyhow!("перенос — объект {{to: workspace}}").into()); }
    if to == name { return Err(anyhow::anyhow!("сессия уже в {to}").into()); }
    let cwd = body.get("cwd").and_then(|c| c.as_str()).map(|c| format!("?cwd={}", urlencoding::encode(c))).unwrap_or_default();
    let src = s.agentd_client(&name).await?;
    let dst = s.agentd_client(&to).await?;
    // каталог беседы (под домом, не сам дом) уезжает с ней — у источника его потом убираем
    let src_cwd = src.get(&format!("/v1/sessions/{id}")).await.ok().and_then(|v| v.pointer("/info/cwd").and_then(|c| c.as_str()).map(str::to_string)).unwrap_or_default();
    let (_, _, tar) = src.raw_get(&format!("/v1/sessions/{id}/export")).await?;
    let size = tar.len();
    let info = dst.raw_post(&format!("/v1/sessions/import{cwd}"), tar).await?;
    let removed = src.delete(&format!("/v1/sessions/{id}")).await.map(|c| c < 400).unwrap_or(false);
    let files_dir = src_cwd.strip_prefix("/home/").map(|rest| rest.contains('/')).unwrap_or(false) && src_cwd.matches('/').count() >= 3;
    if removed && files_dir {
        if let Err(e) = src.delete(&format!("/v1/fs?path={}&recursive=1", urlencoding::encode(&src_cwd))).await { tracing::warn!("перенос {id}: каталог {src_cwd} у источника не удалился: {e:#}"); }
    }
    let title = info.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let _ = s.store.event("agent", &to, &format!("беседа «{title}» перенесена из {name} ({} КБ){}", size / 1024, if removed { "" } else { "; исходная запись не удалилась" }));
    Ok(Json(serde_json::json!({ "project": to, "session": info, "bytes": size, "source_removed": removed })))
}
async fn session_ws(State(s): State<Service>, Path((name, id)): Path<(String, String)>, Query(q): Q, ws: axum::extract::ws::WebSocketUpgrade) -> Result<Response, ApiError> {
    agentd_ws(&s, &name, &format!("/v1/sessions/{id}/ws{}", query_string(&q)), ws).await
}
async fn project_usage(State(s): State<Service>, Path(name): Path<String>, Query(q): Q) -> R<serde_json::Value> { Ok(Json(s.agentd_client(&name).await?.get(&format!("/v1/usage{}", query_string(&q))).await?)) }

async fn pty_relay(client: axum::extract::ws::WebSocket, upstream: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) {
    use axum::extract::ws::Message as A;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as U;
    let (mut cs, mut cr) = client.split();
    let (mut us, mut ur) = upstream.split();
    let to_upstream = async {
        while let Some(Ok(m)) = cr.next().await {
            let u = match m { A::Text(t) => U::Text(t.as_str().into()), A::Binary(b) => U::Binary(b), A::Ping(p) => U::Ping(p), A::Pong(p) => U::Pong(p), A::Close(_) => break };
            if us.send(u).await.is_err() { break }
        }
        let _ = us.close().await;
    };
    let to_client = async {
        while let Some(Ok(m)) = ur.next().await {
            let a = match m { U::Text(t) => A::Text(t.as_str().into()), U::Binary(b) => A::Binary(b), U::Ping(p) => A::Ping(p), U::Pong(p) => A::Pong(p), U::Close(_) => break, U::Frame(_) => continue };
            if cs.send(a).await.is_err() { break }
        }
        let _ = cs.close().await;
    };
    tokio::select! { _ = to_upstream => {}, _ = to_client => {} }
}

async fn detached<T: Send + 'static>(fut: impl std::future::Future<Output = anyhow::Result<T>> + Send + 'static) -> Result<T, ApiError> {
    Ok(tokio::spawn(fut).await.map_err(|e| anyhow::anyhow!("задача прервана: {e}"))??)
}

#[utoipa::path(get, path = "/api/apps/{name}", responses((status = 200, body = AppView)))]
async fn get_app(State(s): State<Service>, Path(name): Path<String>) -> R<AppView> { Ok(Json(s.app(&name).await?)) }

#[derive(Deserialize, ToSchema)]
pub struct LogsQuery { pub tail: Option<u32> }

#[utoipa::path(get, path = "/api/apps/{name}/logs", params(("tail" = Option<u32>, Query)), responses((status = 200, body = String)))]
async fn app_logs(State(s): State<Service>, Path(name): Path<String>, Query(q): Query<LogsQuery>) -> Result<String, ApiError> {
    Ok(s.app_logs(&name, q.tail.unwrap_or(100)).await?)
}

/// Настройки приложения: значения env из манифеста; секреты, присланные как маска, не меняются.
#[utoipa::path(put, path = "/api/apps/{name}/settings", request_body = std::collections::BTreeMap<String, String>, responses((status = 200, body = AppView)))]
async fn app_settings(State(s): State<Service>, Path(name): Path<String>, Json(values): Json<std::collections::BTreeMap<String, String>>) -> R<AppView> {
    Ok(Json(detached(async move { s.app_configure(&name, values).await }).await?))
}

// ---------- переопределения приложения ----------

#[utoipa::path(get, path = "/api/apps/{name}/overrides", responses((status = 200, body = crate::service::AppOverrides)))]
async fn app_overrides_get(State(s): State<Service>, Path(name): Path<String>) -> R<crate::service::AppOverrides> { Ok(Json(s.app_overrides(&name)?)) }

#[utoipa::path(put, path = "/api/apps/{name}/overrides", request_body = crate::service::AppOverrides, responses((status = 200, body = AppView)))]
async fn app_overrides_put(State(s): State<Service>, Path(name): Path<String>, Json(o): Json<crate::service::AppOverrides>) -> R<AppView> {
    Ok(Json(detached(async move { s.app_set_overrides(&name, &o.env, &o.compose).await }).await?))
}

// ---------- монитор и виджеты ----------

#[utoipa::path(get, path = "/api/monitor", responses((status = 200, body = crate::monitor::Snapshot)))]
async fn monitor_snapshot(State(s): State<Service>) -> Json<crate::monitor::Snapshot> { Json(s.monitor.snapshot()) }

#[derive(Deserialize, ToSchema)]
pub struct FeedQuery { pub url: String, pub max_age: Option<u64> }
#[utoipa::path(get, path = "/api/widgets/feed", params(("url" = String, Query), ("max_age" = Option<u64>, Query)), responses((status = 200, body = crate::feeds::Feed)))]
async fn widget_feed(State(s): State<Service>, Query(q): Query<FeedQuery>) -> R<crate::feeds::Feed> { Ok(Json(s.feeds.fetch(&q.url, q.max_age.unwrap_or(600).clamp(60, 86400)).await?)) }

#[derive(Deserialize, ToSchema)]
pub struct WeatherQuery { pub lat: f64, pub lon: f64 }
#[utoipa::path(get, path = "/api/widgets/weather", params(("lat" = f64, Query), ("lon" = f64, Query)), responses((status = 200, body = serde_json::Value)))]
async fn widget_weather(State(s): State<Service>, Query(q): Query<WeatherQuery>) -> R<serde_json::Value> { Ok(Json(s.feeds.weather(q.lat, q.lon).await?)) }

#[derive(Deserialize, ToSchema)]
pub struct GeoQuery { pub q: String }
#[utoipa::path(get, path = "/api/widgets/geocode", params(("q" = String, Query)), responses((status = 200, body = serde_json::Value)))]
async fn widget_geocode(State(s): State<Service>, Query(q): Query<GeoQuery>) -> R<serde_json::Value> { Ok(Json(s.feeds.geocode(&q.q).await?)) }

// ---------- файлы ----------

#[derive(Deserialize, ToSchema)]
pub struct PathQuery { pub path: Option<String> }

#[utoipa::path(get, path = "/api/files/roots", responses((status = 200, body = Vec<crate::files::Root>)))]
async fn files_roots(State(s): State<Service>) -> R<Vec<crate::files::Root>> { Ok(Json(s.file_roots().await?)) }

#[utoipa::path(get, path = "/api/files/{root}/list", params(("path" = Option<String>, Query)), responses((status = 200, body = crate::files::Listing)))]
async fn files_list(State(s): State<Service>, Path(root): Path<String>, Query(q): Query<PathQuery>) -> R<crate::files::Listing> {
    Ok(Json(s.file_list(&root, q.path.as_deref().unwrap_or("")).await?))
}

/// Содержимое файла: текст — inline, остальное — attachment.
#[utoipa::path(get, path = "/api/files/{root}/read", params(("path" = String, Query)), responses((status = 200, body = String)))]
async fn files_read(State(s): State<Service>, Path(root): Path<String>, Query(q): Query<PathQuery>) -> Result<Response, ApiError> {
    let rel = q.path.ok_or_else(|| anyhow::anyhow!("имя: path обязателен"))?;
    let (bytes, name) = s.file_read(&root, &rel).await?;
    let mime = mime_guess::from_path(&name).first_or_octet_stream();
    let disp = if crate::files::is_text(&name) || mime.type_() == "image" { "inline" } else { "attachment" };
    Ok(([(header::CONTENT_TYPE, mime.as_ref().to_string()), (header::CONTENT_DISPOSITION, format!("{disp}; filename=\"{}\"", name.replace('"', "")))], bytes).into_response())
}

#[utoipa::path(put, path = "/api/files/{root}/write", params(("path" = String, Query)), request_body = crate::files::WriteBody, responses((status = 204)))]
async fn files_write(State(s): State<Service>, Path(root): Path<String>, Query(q): Query<PathQuery>, Json(b): Json<crate::files::WriteBody>) -> Result<StatusCode, ApiError> {
    let rel = q.path.ok_or_else(|| anyhow::anyhow!("имя: path обязателен"))?;
    s.file_write(&root, &rel, b.content.as_bytes()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Загрузка файла как есть (тело запроса — содержимое).
#[utoipa::path(post, path = "/api/files/{root}/upload", params(("path" = String, Query)), request_body(content = Vec<u8>, content_type = "application/octet-stream"), responses((status = 204)))]
async fn files_upload(State(s): State<Service>, Path(root): Path<String>, Query(q): Query<PathQuery>, body: axum::body::Bytes) -> Result<StatusCode, ApiError> {
    let rel = q.path.ok_or_else(|| anyhow::anyhow!("имя: path обязателен"))?;
    s.file_write(&root, &rel, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/files/{root}/mkdir", params(("path" = String, Query)), responses((status = 204)))]
async fn files_mkdir(State(s): State<Service>, Path(root): Path<String>, Query(q): Query<PathQuery>) -> Result<StatusCode, ApiError> {
    s.file_mkdir(&root, q.path.as_deref().unwrap_or("")).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/files/{root}/rename", params(("path" = String, Query)), request_body = crate::files::RenameBody, responses((status = 204)))]
async fn files_rename(State(s): State<Service>, Path(root): Path<String>, Query(q): Query<PathQuery>, Json(b): Json<crate::files::RenameBody>) -> Result<StatusCode, ApiError> {
    s.file_rename(&root, q.path.as_deref().unwrap_or(""), &b.to).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/files/{root}/delete", params(("path" = String, Query)), responses((status = 204)))]
async fn files_delete(State(s): State<Service>, Path(root): Path<String>, Query(q): Query<PathQuery>) -> Result<StatusCode, ApiError> {
    s.file_delete(&root, q.path.as_deref().unwrap_or("")).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct OpenBody { pub open: String }
/// Режим открытия приложения в Desktop: iframe (окно) | newtab.
#[utoipa::path(put, path = "/api/apps/{name}/open", request_body = OpenBody, responses((status = 200, body = AppView)))]
async fn app_open(State(s): State<Service>, Path(name): Path<String>, Json(b): Json<OpenBody>) -> R<AppView> {
    s.app_set_open(&name, &b.open)?;
    Ok(Json(s.app(&name).await?))
}

/// Показать значение секрета приложения (пароль веб-интерфейса и т.п.); один пользователь, периметр — overlay-сеть.
async fn app_setting_reveal(State(s): State<Service>, Path((name, key)): Path<(String, String)>) -> Result<Json<serde_json::Value>, ApiError> {
    let m = s.manifest(&name)?;
    if !m.env.contains_key(&key) {
        return Err(anyhow::anyhow!("имя {key}: переменная не объявлена").into());
    }
    let v = s.store.get_secret(&format!("app:{name}"), &key)?.unwrap_or_else(|| match m.env.get(&key) { Some(EnvSpec::Literal(l)) => l.clone(), _ => String::new() });
    s.store.event("app", &name, &format!("показан секрет {key}"))?;
    Ok(Json(serde_json::json!({ "key": key, "value": v })))
}

/// Длинное описание приложения (markdown) из каталога.
async fn app_readme(State(s): State<Service>, Path(name): Path<String>) -> Response {
    let Ok(m) = s.manifest(&name) else { return StatusCode::NOT_FOUND.into_response() };
    let Some(r) = m.readme.as_deref().filter(|i| i.starts_with("./") && !i.contains("..")) else { return (StatusCode::NOT_FOUND, "").into_response() };
    match std::fs::read_to_string(s.cfg.store_dir().join(&name).join(r)) {
        Ok(t) => ([(header::CONTENT_TYPE, "text/markdown; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=3600")], t).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Иконка приложения из каталога Store (`icon: ./logo.jpg` в манифесте).
async fn app_icon(State(s): State<Service>, Path(name): Path<String>) -> Response {
    let Ok(m) = s.manifest(&name) else { return StatusCode::NOT_FOUND.into_response() };
    let Some(icon) = m.icon.as_deref().filter(|i| i.starts_with("./") && !i.contains("..")) else { return StatusCode::NOT_FOUND.into_response() };
    match std::fs::read(s.cfg.store_dir().join(&name).join(icon)) {
        Ok(b) => ([(header::CONTENT_TYPE, mime_guess::from_path(icon).first_or_octet_stream().as_ref()), (header::CACHE_CONTROL, "public, max-age=86400")], b).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Настройки Desktop (оформление, раскладки) — один пользователь, поэтому один документ; хранится в ядре, чтобы
/// быть общим для всех устройств; браузер держит копию в localStorage для мгновенного применения.
#[utoipa::path(get, path = "/api/settings/desktop", responses((status = 200, body = serde_json::Value)))]
async fn desktop_settings_get(State(s): State<Service>) -> R<serde_json::Value> {
    Ok(Json(s.store.get_kv("desktop.settings")?.unwrap_or(serde_json::json!({}))))
}
#[utoipa::path(put, path = "/api/settings/desktop", request_body = serde_json::Value, responses((status = 200, body = serde_json::Value)))]
async fn desktop_settings_put(State(s): State<Service>, Json(v): Json<serde_json::Value>) -> R<serde_json::Value> {
    // разделы документа (appearance, widgets, session…) присылают разные части Desktop независимо друг от друга:
    // сливаем по ключам верхнего уровня, иначе сохранение оформления стирало бы виджеты и наоборот
    let mut doc = s.store.get_kv("desktop.settings")?.unwrap_or(serde_json::json!({}));
    match (doc.as_object_mut(), v.as_object()) {
        (Some(cur), Some(new)) => { for (k, val) in new { cur.insert(k.clone(), val.clone()); } }
        _ => doc = v,
    }
    if serde_json::to_string(&doc).map_err(anyhow::Error::from)?.len() > 64 * 1024 {
        return Err(anyhow::anyhow!("настройки Desktop: документ больше 64 KB").into());
    }
    s.store.put_kv("desktop.settings", &doc)?;
    Ok(Json(doc))
}

/// Корневой сертификат внутреннего CA (страница «Устройства» в Desktop).
#[utoipa::path(get, path = "/api/ca.crt", responses((status = 200, body = String)))]
async fn ca_cert(State(s): State<Service>) -> Response {
    // копия в state_dir (bootstrap) либо напрямую корень внутреннего CA edge из тома Caddy
    let candidates = [s.cfg.state_dir.join("ca.crt"), std::path::PathBuf::from(std::env::var("CLOUDD_EDGE_CA_PATH").unwrap_or_else(|_| "/var/lib/docker/volumes/03-edge_caddy-data/_data/caddy/pki/authorities/local/root.crt".into()))];
    match candidates.iter().find_map(|p| std::fs::read(p).ok()).ok_or(()) {
        Ok(b) => ([(header::CONTENT_TYPE, "application/x-pem-file"), (header::CONTENT_DISPOSITION, "attachment; filename=\"cloudos-ca.crt\"")], b).into_response(),
        Err(()) => (StatusCode::NOT_FOUND, "CA не настроен (tls_internal=false или ca.crt отсутствует)").into_response(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct LinkBody { pub provider: String }
#[derive(Deserialize, ToSchema)]
pub struct SharedBody { pub shared: bool }

/// Связь приложений: потребитель получает сеть провайдера, `<ПРОВАЙДЕР>_URL` и ключи из хуков провайдера.
#[utoipa::path(post, path = "/api/apps/{name}/links", request_body = LinkBody, responses((status = 200, body = AppView)))]
async fn app_link(State(s): State<Service>, Path(name): Path<String>, Json(b): Json<LinkBody>) -> R<AppView> { Ok(Json(detached(async move { s.app_link(&name, &b.provider).await }).await?)) }

#[utoipa::path(delete, path = "/api/apps/{name}/links/{provider}", responses((status = 200, body = AppView)))]
async fn app_unlink(State(s): State<Service>, Path((name, provider)): Path<(String, String)>) -> R<AppView> { Ok(Json(detached(async move { s.app_unlink(&name, &provider).await }).await?)) }

#[utoipa::path(put, path = "/api/apps/{name}/shared", request_body = SharedBody, responses((status = 200, body = AppView)))]
async fn app_shared(State(s): State<Service>, Path(name): Path<String>, Json(b): Json<SharedBody>) -> R<AppView> { Ok(Json(detached(async move { s.app_set_shared(&name, b.shared).await }).await?)) }

#[derive(Deserialize, ToSchema)]
pub struct NoteBody { pub text: String }

/// Сменить сгенерированный секрет (новый пароль/токен) и перевыкатить стек.
#[utoipa::path(post, path = "/api/apps/{name}/settings/{key}/rotate", responses((status = 200, body = AppView)))]
async fn app_setting_rotate(State(s): State<Service>, Path((name, key)): Path<(String, String)>) -> R<AppView> { Ok(Json(detached(async move { s.app_rotate_secret(&name, &key).await }).await?)) }

#[utoipa::path(get, path = "/api/apps/{name}/note", responses((status = 200, body = String)))]
async fn app_note_get(State(s): State<Service>, Path(name): Path<String>) -> Result<Json<serde_json::Value>, ApiError> { Ok(Json(serde_json::json!({ "text": s.app_note(&name)? }))) }

#[utoipa::path(put, path = "/api/apps/{name}/note", request_body = NoteBody, responses((status = 204)))]
async fn app_note_put(State(s): State<Service>, Path(name): Path<String>, Json(b): Json<NoteBody>) -> Result<StatusCode, ApiError> { s.app_set_note(&name, &b.text)?; Ok(StatusCode::NO_CONTENT) }

/// Учётные данные администратора Coder (из bootstrap): один пользователь, периметр — overlay-сеть; показ пишется в события.
#[utoipa::path(get, path = "/api/system/credentials/coder/reveal", responses((status = 200, body = String)))]
async fn coder_credentials(State(s): State<Service>) -> Result<Json<serde_json::Value>, ApiError> {
    let Some(pw) = s.cfg.admin_password.clone() else { return Err(anyhow::anyhow!("пароль администратора Coder ядру не известен: задайте CLOUDOS_ADMIN_PASSWORD в /etc/cloudos/cloudd.env").into()) };
    s.store.event("system", "coder", "показан пароль администратора Coder")?;
    Ok(Json(serde_json::json!({ "url": s.cfg.coder_public_url(), "user": s.cfg.admin_email, "password": pw })))
}

/// Починить права на каталоги данных приложения (владелец по compose/образу, см. appdata.rs) и перевыкатить; список изменений.
#[utoipa::path(post, path = "/api/apps/{name}/repair", responses((status = 200, body = Vec<String>)))]
async fn repair_app(State(s): State<Service>, Path(name): Path<String>) -> R<Vec<String>> { Ok(Json(detached(async move { s.app_repair(&name).await }).await?)) }

#[utoipa::path(get, path = "/api/apps/{name}/snapshots", responses((status = 200, body = Vec<crate::backup::RestorePoint>)))]
async fn app_snapshots(State(s): State<Service>, Path(name): Path<String>) -> R<Vec<crate::backup::RestorePoint>> { Ok(Json(s.app_restore_points(&name).await?)) }

/// Откат данных приложения к снимку tank/apps (текущее состояние сохраняется снимком pre-restore).
#[utoipa::path(post, path = "/api/apps/{name}/restore/{snap}", responses((status = 200, body = AppView)))]
async fn app_restore(State(s): State<Service>, Path((name, snap)): Path<(String, String)>) -> R<AppView> { Ok(Json(detached(async move { s.app_restore(&name, &snap).await }).await?)) }

#[utoipa::path(get, path = "/api/backups", responses((status = 200, body = crate::backup::BackupStatus)))]
async fn backups_get(State(s): State<Service>) -> R<crate::backup::BackupStatus> { Ok(Json(s.backup_status().await?)) }

#[utoipa::path(put, path = "/api/backups", request_body = crate::backup::BackupConfig, responses((status = 200, body = crate::backup::BackupConfig)))]
async fn backups_put(State(s): State<Service>, Json(c): Json<crate::backup::BackupConfig>) -> R<crate::backup::BackupConfig> { Ok(Json(s.set_backup_config(&c)?)) }

/// Снимок всех датасетов сейчас (kind manual, политикой не удаляется).
#[utoipa::path(post, path = "/api/backups/snapshot", responses((status = 200, body = serde_json::Value)))]
async fn backups_snapshot(State(s): State<Service>) -> R<serde_json::Value> { Ok(Json(detached(async move { s.backup_cycle("manual", false).await }).await?)) }

/// Снимок + restic сейчас.
#[utoipa::path(post, path = "/api/backups/run", responses((status = 200, body = serde_json::Value)))]
async fn backups_run(State(s): State<Service>) -> R<serde_json::Value> { Ok(Json(detached(async move { s.backup_cycle("manual", true).await }).await?)) }

// ---------------------------------------------------------------------------------------------------------------------
// первый запуск и обновления
// ---------------------------------------------------------------------------------------------------------------------

async fn setup_get(State(s): State<Service>) -> R<crate::setup::SetupState> { Ok(Json(s.setup_state().await?)) }
async fn setup_login(State(s): State<Service>) -> R<crate::setup::TailscaleInfo> { Ok(Json(s.tailscale_login().await?)) }
async fn setup_host(State(s): State<Service>, Json(b): Json<crate::setup::HostBody>) -> R<crate::setup::SwitchResult> { Ok(Json(s.setup_switch_host(&b.host).await?)) }
async fn setup_done(State(s): State<Service>, Json(b): Json<crate::setup::DoneBody>) -> R<crate::setup::SetupState> { s.setup_set_done(b.done)?; Ok(Json(s.setup_state().await?)) }

#[derive(Deserialize)]
struct NameQuery { name: Option<String> }

async fn updates_get(State(s): State<Service>) -> R<crate::updates::UpdatesView> { Ok(Json(s.updates_view().await?)) }
async fn updates_check(State(s): State<Service>) -> R<crate::updates::SystemUpdateInfo> { Ok(Json(s.check_system_update().await?)) }
async fn updates_channel(State(s): State<Service>, Json(b): Json<crate::updates::ChannelBody>) -> R<crate::updates::SystemUpdateInfo> { Ok(Json(s.set_channel(&b.url).await?)) }
async fn updates_download(State(s): State<Service>) -> R<crate::updates::PackageInfo> { Ok(Json(s.download_update().await?)) }
async fn updates_upload(State(s): State<Service>, Query(q): Query<NameQuery>, body: axum::body::Bytes) -> R<crate::updates::PackageInfo> {
    Ok(Json(s.upload_update(q.name.as_deref().unwrap_or("package.tar.gz"), &body)?))
}
async fn updates_apply(State(s): State<Service>, Json(b): Json<crate::updates::ApplyBody>) -> R<crate::updates::ApplyResult> { Ok(Json(s.apply_update(&b.file).await?)) }
async fn updates_rollback(State(s): State<Service>) -> R<serde_json::Value> { s.rollback_update().await?; Ok(Json(serde_json::json!({ "ok": true }))) }
async fn updates_package_delete(State(s): State<Service>, Path(file): Path<String>) -> R<serde_json::Value> { s.delete_package(&file)?; Ok(Json(serde_json::json!({ "ok": true }))) }
async fn updates_catalog(State(s): State<Service>, Json(b): Json<crate::updates::CatalogBody>) -> R<serde_json::Value> {
    let started = s.catalog_refresh(b.source.as_deref().unwrap_or("all"))?;
    Ok(Json(serde_json::json!({ "started": started })))
}
async fn update_app(State(s): State<Service>, Path(name): Path<String>) -> R<AppView> { Ok(Json(s.app_update(&name).await?)) }
async fn update_component(State(s): State<Service>, Path(project): Path<String>) -> R<crate::updates::ComponentUpdateResult> { Ok(Json(s.component_update(&project).await?)) }
