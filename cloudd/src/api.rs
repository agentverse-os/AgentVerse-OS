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
        let code = if msg.contains("не найден") || msg.contains("не установлено") { StatusCode::NOT_FOUND } else if msg.contains("имя ") || msg.contains("требует") || msg.contains("зависят") || msg.contains("не объявлена") || msg.contains("недопустимый путь") || msg.contains("за пределы корня") { StatusCode::BAD_REQUEST } else { StatusCode::INTERNAL_SERVER_ERROR };
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
async fn status(State(s): State<Service>) -> R<SystemStatus> { Ok(Json(s.status().await?)) }

#[utoipa::path(get, path = "/api/events", responses((status = 200, body = Vec<Event>)))]
async fn events(State(s): State<Service>) -> R<Vec<Event>> { Ok(Json(s.store.recent_events(100)?)) }

#[utoipa::path(get, path = "/api/projects", responses((status = 200, body = Vec<ProjectView>)))]
async fn list_projects(State(s): State<Service>) -> R<Vec<ProjectView>> { Ok(Json(s.projects().await?)) }

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
pub struct GrantBody { pub capability: String }

#[utoipa::path(post, path = "/api/projects/{name}/grants", request_body = GrantBody, responses((status = 200, body = ProjectView)))]
async fn grant(State(s): State<Service>, Path(name): Path<String>, Json(b): Json<GrantBody>) -> R<ProjectView> { Ok(Json(detached(async move { s.grant(&name, &b.capability).await }).await?)) }

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
    if serde_json::to_string(&v).map_err(anyhow::Error::from)?.len() > 64 * 1024 {
        return Err(anyhow::anyhow!("имя настроек: документ больше 64 KB").into());
    }
    s.store.put_kv("desktop.settings", &v)?;
    Ok(Json(v))
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
