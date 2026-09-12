//! Адаптер каталога Coolify (templates/compose/*.yaml, ветка v4.x): обычный compose + метаданные в комментариях
//! (`# documentation`, `# slogan`, `# category`, `# tags`, `# logo`, `# port`) и магические переменные
//! `SERVICE_{FQDN|URL|USER|PASSWORD|BASE64|REALBASE64|HEX_*|PASSWORD_64}_<SERVICE>[_PORT]`.
//! id — `<файл>-coolify` (дубликаты с Runtipi допускаются, различаются плашкой источника).

use super::{bind_named_volumes, existing_origin, fetch_tarball, normalize_service, write_app, ImportReport, Imported};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

pub const TARBALL: &str = "https://github.com/coollabsio/coolify/archive/refs/heads/v4.x.tar.gz";

pub async fn import(store_dir: &Path, only: &[String]) -> Result<ImportReport> {
    let (tmp, root) = fetch_tarball(TARBALL, "coolify").await?;
    let dir = root.join("templates/compose");
    let logos = root.join("public/svgs");
    let mut report = ImportReport { source: "coolify".into(), ..Default::default() };
    let mut files: Vec<_> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "yaml" || x == "yml").unwrap_or(false)).collect();
    files.sort();
    for f in files {
        let base = f.file_stem().unwrap().to_string_lossy().to_lowercase().replace('_', "-");
        let id = format!("{base}-coolify");
        if !only.is_empty() && !only.iter().any(|o| *o == id || *o == base) {
            continue;
        }
        match import_one(&f, &logos, &id, store_dir) {
            Ok(true) => report.imported.push(id),
            Ok(false) => report.skipped_manual.push(id),
            Err(e) => report.skipped.push((id, format!("{e:#}"))),
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(report)
}

fn meta(text: &str, key: &str) -> Option<String> {
    text.lines().take(40).find_map(|l| l.strip_prefix(&format!("# {key}:")).map(|v| v.trim().to_string())).filter(|v| !v.is_empty())
}

fn import_one(file: &Path, logos: &Path, id: &str, store_dir: &Path) -> Result<bool> {
    if matches!(existing_origin(store_dir, id), Some(o) if o != "coolify") {
        return Ok(false);
    }
    crate::service::validate_name(id).map_err(|e| anyhow!("{e}"))?;
    let text = std::fs::read_to_string(file)?;
    // `${VAR:?сообщение}` → `${VAR}`: значение придёт из настроек приложения
    let re_req = regex_lite::Regex::new(r"\$\{([A-Z][A-Z0-9_]+):\?[^}]*\}").unwrap();
    let text = re_req.replace_all(&text, "${$1}").to_string();
    let yml: Value = serde_yaml_ng::from_str(&text).map_err(|e| anyhow!("yaml: {e}"))?;
    let mut compose = yml.as_object().cloned().ok_or_else(|| anyhow!("не объект"))?;
    let services = compose.get("services").and_then(|v| v.as_object()).cloned().ok_or_else(|| anyhow!("нет services"))?;
    if services.is_empty() {
        return Err(anyhow!("пустой список сервисов"));
    }
    // магические переменные Coolify по всему тексту
    let re_magic = regex_lite::Regex::new(r"SERVICE_(FQDN|URL|USER|PASSWORD_64|PASSWORD|BASE64_128|BASE64_64|BASE64|REALBASE64_128|REALBASE64_64|REALBASE64|HEX_32|HEX_64|UUID)_([A-Z0-9]+)(?:_([0-9]{2,5}))?").unwrap();
    let mut env = Map::new();
    let mut settings = Vec::new();
    let mut main: Option<(String, u16)> = None;
    let mut fqdn_vars: BTreeSet<String> = BTreeSet::new();
    for cap in re_magic.captures_iter(&text) {
        let full = cap.get(0).unwrap().as_str().to_string();
        let kind = cap.get(1).unwrap().as_str();
        let svc = cap.get(2).unwrap().as_str().to_lowercase();
        let port = cap.get(3).and_then(|p| p.as_str().parse::<u16>().ok());
        if env.contains_key(&full) { continue; }
        let spec = match kind {
            "FQDN" => { fqdn_vars.insert(full.clone()); if main.is_none() || port.is_some() { main = Some((svc.clone(), port.unwrap_or(main.as_ref().map(|m| m.1).unwrap_or(80)))); } Value::String("${APP_DOMAIN}".into()) }
            "URL" => { fqdn_vars.insert(full.clone()); if main.is_none() || port.is_some() { main = Some((svc.clone(), port.unwrap_or(main.as_ref().map(|m| m.1).unwrap_or(80)))); } Value::String("https://${APP_DOMAIN}".into()) }
            "USER" => json!({ "generate": "password" }),
            "PASSWORD" => json!({ "generate": "password" }),
            "HEX_32" => json!({ "generate": "token" }),
            _ => json!({ "generate": "hex64" }),
        };
        env.insert(full, spec);
    }
    // главный сервис: из FQDN/URL или первый; порт — из `# port:` если задан
    let (mut main_service, mut main_port) = main.unwrap_or_else(|| (services.keys().next().unwrap().clone(), 80));
    if !services.contains_key(&main_service) {
        // имя в переменной — без дефисов; ищем сервис по нормализованному имени
        if let Some((k, _)) = services.iter().find(|(k, _)| k.to_lowercase().replace(['-', '_', '.'], "") == main_service) { main_service = k.clone(); } else { main_service = services.keys().next().unwrap().clone(); }
    }
    if let Some(p) = meta(&text, "port").and_then(|p| p.parse::<u16>().ok()) { main_port = p; }
    // сервисы: `- SERVICE_X` (без значения) → `SERVICE_X=${SERVICE_X}`; нормализация
    let mut notes = Vec::new();
    let mut host_docker = false;
    let mut uses_docker_ro = false;
    let mut out = Map::new();
    for (name, svc) in &services {
        let mut m = svc.as_object().cloned().unwrap_or_default();
        if let Some(Value::Array(items)) = m.get("environment").cloned() {
            let fixed: Vec<Value> = items.into_iter().map(|it| match it { Value::String(s) if !s.contains('=') && s.starts_with("SERVICE_") => Value::String(format!("{s}=${{{s}}}")), other => other }).collect();
            m.insert("environment".into(), Value::Array(fixed));
        }
        m.remove("exclude_from_hc");
        normalize_service(id, name, &mut m, &mut notes, &mut host_docker, &mut uses_docker_ro);
        out.insert(name.clone(), Value::Object(m));
    }
    compose.insert("services".into(), Value::Object(out));
    super::inline_volumes_to_configs(&mut compose);
    bind_named_volumes(&mut compose);
    compose.remove("networks");
    compose.insert("networks".into(), json!({ "appnet": { "external": true, "name": "${APP_NET}" } }));
    for k in ["x-coolify", "version", "name"] { compose.remove(k); }
    // настройки: ключи API и подобное, заданные как `${VAR}` без SERVICE_ — оставляем пользователю через settings с пустым значением
    let re_plain = regex_lite::Regex::new(r"\$\{([A-Z][A-Z0-9_]+)(?::?[-?]([^}]*))?\}").unwrap();
    for cap in re_plain.captures_iter(&text) {
        let var = cap.get(1).unwrap().as_str().to_string();
        if var.starts_with("SERVICE_") || var.starts_with("APP_") || env.contains_key(&var) || ["EDGE_HOST", "TZ"].contains(&var.as_str()) { continue; }
        let default = cap.get(2).map(|d| d.as_str().to_string()).unwrap_or_default();
        let secret = var.contains("KEY") || var.contains("SECRET") || var.contains("TOKEN") || var.contains("PASSWORD");
        env.insert(var.clone(), Value::String(default.clone()));
        settings.push(json!({ "env": var, "type": if secret { "password" } else { "text" }, "label": var.replace('_', " ").to_lowercase(), "hint": if default.is_empty() { Value::Null } else { json!(format!("по умолчанию: {default}")) }, "required": false, "options": Value::Null }));
    }
    let title = meta(&text, "name").unwrap_or_else(|| id.trim_end_matches("-coolify").split('-').map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() }).collect::<Vec<_>>().join(" "));
    let icon_file = meta(&text, "logo").and_then(|l| { let p = logos.join(l.trim_start_matches("svgs/")); std::fs::read(&p).ok().map(|b| (b, if p.extension().map(|e| e == "png").unwrap_or(false) { "png" } else { "svg" })) });
    let tags: Vec<String> = meta(&text, "tags").map(|t| t.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    let categories: Vec<String> = meta(&text, "category").map(|c| vec![c]).unwrap_or_default();
    write_app(store_dir, Imported {
        login_hints: (None, None),
        id: id.to_string(), title, description: meta(&text, "slogan"), compose: Value::Object(compose), main_service, main_port, env, settings, notes, host_docker, uses_docker_ro,
        icon_file, icon_url: None, gallery: vec![], readme: None, website: meta(&text, "documentation"), tags: tags.clone(), categories: categories.clone(),
        upstream: json!({ "source": meta(&text, "documentation"), "categories": categories, "tags": tags, "template": file.file_name().unwrap().to_string_lossy() }),
        origin: "coolify",
    })?;
    let _ = fqdn_vars;
    Ok(true)
}
