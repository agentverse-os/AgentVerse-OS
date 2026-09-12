//! Адаптер каталога Umbrel (umbrel-apps): `umbrel-app.yml` (метаданные, галерея, порт) + `docker-compose.yml`
//! с сервисом `app_proxy` (APP_HOST/APP_PORT → главный сервис и его порт), переменные ${APP_DATA_DIR}, ${APP_PASSWORD},
//! ${APP_SEED}, ${APP_DOMAIN}, экспорт из exports.sh. Приложения с зависимостями от других Umbrel-приложений пропускаются.
//! id — `<app>-umbrel`. Иконки и галерея — по URL из getumbrel.github.io/umbrel-apps-gallery.

use super::{bind_named_volumes, existing_origin, fetch_tarball, normalize_service, write_app, ImportReport, Imported};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::path::Path;

pub const TARBALL: &str = "https://github.com/getumbrel/umbrel-apps/archive/refs/heads/master.tar.gz";
const GALLERY: &str = "https://getumbrel.github.io/umbrel-apps-gallery";

pub async fn import(store_dir: &Path, only: &[String]) -> Result<ImportReport> {
    let (tmp, root) = fetch_tarball(TARBALL, "umbrel").await?;
    let mut report = ImportReport { source: "umbrel".into(), ..Default::default() };
    let mut dirs: Vec<_> = std::fs::read_dir(&root)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir() && p.join("umbrel-app.yml").exists()).collect();
    dirs.sort();
    for d in dirs {
        let base = d.file_name().unwrap().to_string_lossy().to_lowercase();
        let id = format!("{base}-umbrel");
        if !only.is_empty() && !only.iter().any(|o| *o == id || *o == base) {
            continue;
        }
        match import_one(&d, &base, &id, store_dir) {
            Ok(Some(reason)) => report.skipped.push((id, reason)),
            Ok(None) => report.imported.push(id),
            Err(e) => report.skipped.push((id, format!("{e:#}"))),
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(report)
}

fn s(v: &Value, k: &str) -> Option<String> { v.get(k).and_then(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()) }

fn import_one(dir: &Path, base: &str, id: &str, store_dir: &Path) -> Result<Option<String>> {
    if matches!(existing_origin(store_dir, id), Some(o) if o != "umbrel") {
        return Ok(Some("ручной манифест".into()));
    }
    crate::service::validate_name(id).map_err(|e| anyhow!("{e}"))?;
    let app: Value = serde_yaml_ng::from_str(&std::fs::read_to_string(dir.join("umbrel-app.yml"))?).map_err(|e| anyhow!("umbrel-app.yml: {e}"))?;
    if let Some(deps) = app.get("dependencies").and_then(|d| d.as_array()) {
        if !deps.is_empty() {
            return Ok(Some(format!("зависит от приложений Umbrel: {}", deps.iter().filter_map(|d| d.as_str()).collect::<Vec<_>>().join(", "))));
        }
    }
    let yml: Value = serde_yaml_ng::from_str(&std::fs::read_to_string(dir.join("docker-compose.yml"))?).map_err(|e| anyhow!("compose: {e}"))?;
    let mut compose = yml.as_object().cloned().ok_or_else(|| anyhow!("compose не объект"))?;
    let mut services = compose.get("services").and_then(|v| v.as_object()).cloned().ok_or_else(|| anyhow!("нет services"))?;
    // app_proxy → главный сервис и порт
    let proxy = services.remove("app_proxy");
    let (mut main_service, mut main_port) = (None::<String>, None::<u16>);
    if let Some(pr) = &proxy {
        let env = pr.get("environment").cloned().unwrap_or(Value::Null);
        let get = |k: &str| -> Option<String> { match &env { Value::Object(o) => o.get(k).map(super::scalar_to_string), Value::Array(a) => a.iter().filter_map(|x| x.as_str()).find_map(|x| x.strip_prefix(&format!("{k}=")).map(|v| v.to_string())), _ => None } };
        if let Some(host) = get("APP_HOST") {
            // `<app>_<service>_1` → service
            let svc = host.strip_prefix(&format!("{base}_")).and_then(|r| r.strip_suffix("_1")).map(|x| x.to_string()).unwrap_or(host.clone());
            if services.contains_key(&svc) { main_service = Some(svc); } else if let Some((k, _)) = services.iter().find(|(k, _)| host.contains(k.as_str())) { main_service = Some(k.clone()); }
        }
        main_port = get("APP_PORT").and_then(|p| p.parse().ok());
    }
    if services.is_empty() {
        return Err(anyhow!("только app_proxy, нет сервисов"));
    }
    let main_service = main_service.unwrap_or_else(|| services.keys().next().unwrap().clone());
    let main_port = main_port.or_else(|| app.get("port").and_then(|p| p.as_u64()).map(|p| p as u16)).unwrap_or(80);
    // exports.sh: `export APP_X="..."` → env литералы
    let mut env = Map::new();
    if let Ok(exp) = std::fs::read_to_string(dir.join("exports.sh")) {
        for line in exp.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("export ") {
                if let Some((k, v)) = rest.split_once('=') {
                    let v = v.trim().trim_matches('"').trim_matches('\'').replace("${EXPORTS_APP_DIR}", "${APP_DATA_DIR}").replace("${UMBREL_ROOT}", "${APP_DATA_DIR}");
                    // ссылки на другие переменные (${APP_DATA_DIR}, ${APP_X_PORT}) ядро раскроет при рендере .env; подстановки команд — нет
                    if !v.contains("$(") && !v.contains('`') { env.insert(k.trim().to_string(), Value::String(v)); }
                }
            }
        }
    }
    let text = serde_yaml_ng::to_string(&Value::Object(services.clone()))?;
    if text.contains("${APP_PASSWORD}") { env.insert("APP_PASSWORD".into(), json!({ "generate": "password" })); }
    if text.contains("${APP_SEED}") { env.insert("APP_SEED".into(), json!({ "generate": "hex64" })); }
    for (var, val) in [("APP_HIDDEN_SERVICE", ""), ("DEVICE_HOSTNAME", "${EDGE_HOST}"), ("DEVICE_DOMAIN_NAME", "${EDGE_HOST}"), ("APP_HOST", "${EDGE_HOST}"), ("UMBREL_ROOT", "${APP_DATA_DIR}"), ("APP_TOR_PROXY", ""), ("TOR_PROXY_IP", ""), ("TOR_PROXY_PORT", "9050")] {
        if text.contains(&format!("${{{var}}}")) { env.insert(var.into(), Value::String(val.into())); }
    }
    let mut notes = Vec::new();
    let mut host_docker = false;
    let mut uses_docker_ro = false;
    let mut out = Map::new();
    for (name, svc) in &services {
        let mut m = svc.as_object().cloned().unwrap_or_default();
        // статические IP сети Umbrel не нужны
        if let Some(Value::Object(nets)) = m.get("networks").cloned() { if nets.values().any(|n| n.get("ipv4_address").is_some()) { m.remove("networks"); } }
        // hostname с подстановкой ${APP_*_IP} и т.п. — убрать
        if m.get("hostname").map(|h| super::scalar_to_string(h).contains("${")).unwrap_or(false) { m.remove("hostname"); }
        normalize_service(base, name, &mut m, &mut notes, &mut host_docker, &mut uses_docker_ro);
        out.insert(name.clone(), Value::Object(m));
    }
    compose.insert("services".into(), Value::Object(out));
    bind_named_volumes(&mut compose);
    compose.remove("networks");
    compose.insert("networks".into(), json!({ "appnet": { "external": true, "name": "${APP_NET}" } }));
    compose.remove("version");
    // остаточные ${APP_*_IP}, ${APP_..._PORT} без значений → настройки/пустые литералы
    let re = regex_lite::Regex::new(r"\$\{([A-Z][A-Z0-9_]+)\}").unwrap();
    let mut settings = Vec::new();
    for cap in re.captures_iter(&serde_yaml_ng::to_string(&Value::Object(compose.clone()))?) {
        let var = cap.get(1).unwrap().as_str().to_string();
        if ["APP_DATA_DIR", "APP_NET", "APP_DOMAIN", "APP_PORT", "APP_NAME", "APP_ID", "EDGE_HOST", "TZ", "APP_PROTOCOL", "ROOT_FOLDER_HOST", "INTERNAL_IP"].contains(&var.as_str()) || env.contains_key(&var) { continue; }
        if var.ends_with("_IP") { env.insert(var.clone(), Value::String(String::new())); continue; }
        env.insert(var.clone(), Value::String(String::new()));
        settings.push(json!({ "env": var, "type": if var.contains("PASSWORD") || var.contains("KEY") || var.contains("SECRET") { "password" } else { "text" }, "label": var.replace('_', " ").to_lowercase(), "hint": "переменная каталога Umbrel без значения по умолчанию", "required": false, "options": Value::Null }));
    }
    let gallery: Vec<String> = app.get("gallery").and_then(|g| g.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).map(|f| format!("{GALLERY}/{base}/{f}")).collect()).unwrap_or_default();
    let cats = s(&app, "category").map(|c| vec![c.to_lowercase()]).unwrap_or_default();
    write_app(store_dir, Imported {
        // defaultUsername/defaultPassword каталога; при deterministicPassword пароль — наш сгенерированный APP_PASSWORD
        login_hints: (s(&app, "defaultUsername"), if app.get("deterministicPassword").and_then(|v| v.as_bool()).unwrap_or(false) { None } else { s(&app, "defaultPassword") }),
        id: id.to_string(),
        title: s(&app, "name").unwrap_or_else(|| base.to_string()),
        description: s(&app, "tagline"),
        compose: Value::Object(compose), main_service, main_port, env, settings, notes, host_docker, uses_docker_ro,
        icon_file: None, icon_url: Some(format!("{GALLERY}/{base}/icon.svg")), gallery,
        readme: s(&app, "description").map(|d| { let _ = std::fs::create_dir_all(store_dir.join(id)); let _ = std::fs::write(store_dir.join(id).join("description.md"), d); "./description.md".to_string() }),
        website: s(&app, "website").or_else(|| s(&app, "repo")),
        tags: vec![], categories: cats.clone(),
        upstream: json!({ "source": s(&app, "repo"), "author": s(&app, "developer"), "version": s(&app, "version"), "categories": cats, "website": s(&app, "website"), "release_notes": s(&app, "releaseNotes") }),
        origin: "umbrel",
    })?;
    Ok(None)
}
