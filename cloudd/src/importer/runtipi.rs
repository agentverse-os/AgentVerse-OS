//! Адаптер каталога Runtipi: динамический compose (docker-compose.json, schemaVersion 2) и статический (x-runtipi),
//! form_fields → env/generate/settings, ${APP_DATA_DIR}, isMain/internalPort, addPorts. id — без суффикса (первичный источник).

use super::{existing_origin, fetch_tarball, normalize_service, scalar_to_string, write_app, ImportReport, Imported, DOCKER_RO_HOST, RAW_DOCKER_SOCKET_APPS};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};
use std::path::Path;

pub const TARBALL: &str = "https://github.com/runtipi/runtipi-appstore/archive/refs/heads/master.tar.gz";

pub async fn import(store_dir: &Path, only: &[String]) -> Result<ImportReport> {
    let (tmp, root) = fetch_tarball(TARBALL, "runtipi").await?;
    let apps = root.join("apps");
    let mut report = ImportReport { source: "runtipi".into(), ..Default::default() };
    let mut ids: Vec<String> = std::fs::read_dir(&apps)?.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    ids.sort();
    for id in ids {
        if id.starts_with("__") || (!only.is_empty() && !only.contains(&id)) {
            continue;
        }
        match import_one(&apps.join(&id), store_dir) {
            Ok(Outcome::Done) => report.imported.push(id),
            Ok(Outcome::Manual) => report.skipped_manual.push(id),
            Ok(Outcome::Skip(reason)) => report.skipped.push((id, reason)),
            Err(e) => report.skipped.push((id, format!("ошибка: {e:#}"))),
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(report)
}

enum Outcome {
    Done,
    Manual,
    Skip(String),
}

fn import_one(src: &Path, store_dir: &Path) -> Result<Outcome> {
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(src.join("config.json")).context("config.json")?)?;
    let id = cfg.get("id").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("нет id"))?.to_string();
    crate::service::validate_name(&id).map_err(|e| anyhow!("{e}"))?;
    if matches!(existing_origin(store_dir, &id), Some(o) if o != "runtipi") {
        return Ok(Outcome::Manual);
    }
    if cfg.get("available").and_then(|v| v.as_bool()) == Some(false) || cfg.get("deprecated").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(Outcome::Skip("недоступно/deprecated в каталоге".into()));
    }
    if let Some(arch) = cfg.get("supported_architectures").and_then(|v| v.as_array()) {
        if !arch.iter().any(|a| a.as_str() == Some("amd64")) {
            return Ok(Outcome::Skip("нет сборки amd64".into()));
        }
    }
    let dc_path = src.join("docker-compose.json");
    let (compose, main_name, main_port, notes, host_docker, uses_docker_ro) = if dc_path.exists() {
        let dc: Value = serde_json::from_str(&std::fs::read_to_string(&dc_path).context("docker-compose.json")?)?;
        translate_dynamic(&dc, &cfg)?
    } else if src.join("docker-compose.yml").exists() {
        let yml: Value = serde_yaml_ng::from_str(&std::fs::read_to_string(src.join("docker-compose.yml")).context("docker-compose.yml")?)?;
        translate_static(&yml, &cfg)?
    } else {
        return Ok(Outcome::Skip("нет compose".into()));
    };
    // env из form_fields
    let mut env = Map::new();
    let mut settings = Vec::new();
    if let Some(ff) = cfg.get("form_fields").and_then(|v| v.as_array()) {
        for f in ff {
            let Some(var) = f.get("env_variable").and_then(|v| v.as_str()) else { continue };
            let ty = f.get("type").and_then(|v| v.as_str()).unwrap_or("text");
            let default = f.get("default").map(scalar_to_string);
            let spec = match (ty, default) {
                ("random", _) => { let min = f.get("min").and_then(|v| v.as_u64()).unwrap_or(0); json!({ "generate": if min > 32 { "hex64" } else { "token" } }) }
                ("password", None) => json!({ "generate": "password" }),
                (_, Some(d)) => Value::String(d),
                (_, None) => Value::String(String::new()),
            };
            env.insert(var.into(), spec);
            settings.push(json!({ "env": var, "type": ty, "label": f.get("label").cloned().unwrap_or(Value::Null), "hint": f.get("hint").cloned().unwrap_or(Value::Null), "required": f.get("required").cloned().unwrap_or(json!(false)), "options": f.get("options").cloned().unwrap_or(Value::Null) }));
        }
    }
    let icon_file = std::fs::read(src.join("metadata/logo.jpg")).ok().map(|b| (b, "jpg"));
    let readme = if src.join("metadata/description.md").exists() {
        std::fs::copy(src.join("metadata/description.md"), store_dir.join(&id).join("description.md")).ok().or_else(|| { std::fs::create_dir_all(store_dir.join(&id)).ok(); std::fs::copy(src.join("metadata/description.md"), store_dir.join(&id).join("description.md")).ok() });
        Some("./description.md".to_string())
    } else { None };
    let cats: Vec<String> = cfg.get("categories").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|c| c.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    write_app(store_dir, Imported {
        login_hints: (None, None),
        id: id.clone(),
        title: cfg.get("name").and_then(|v| v.as_str()).unwrap_or(&id).to_string(),
        description: cfg.get("short_desc").and_then(|v| v.as_str()).map(|s| s.to_string()),
        compose, main_service: main_name, main_port, env, settings, notes, host_docker, uses_docker_ro,
        icon_file, icon_url: None, gallery: vec![], readme,
        website: cfg.get("source").and_then(|v| v.as_str()).map(|s| s.to_string()),
        tags: vec![], categories: cats.clone(),
        upstream: json!({ "source": cfg.get("source"), "author": cfg.get("author"), "version": cfg.get("version"), "categories": cats, "tipi_version": cfg.get("tipi_version") }),
        origin: "runtipi",
    })?;
    Ok(Outcome::Done)
}

type Translated = (Value, String, u16, Vec<String>, bool, bool);

/// Статический compose Runtipi v2: обычный compose + `x-runtipi: {is_main, internal_port}` у сервисов.
fn translate_static(yml: &Value, cfg: &Value) -> Result<Translated> {
    let app_id = cfg.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let services = yml.get("services").and_then(|v| v.as_object()).ok_or_else(|| anyhow!("services не объект"))?;
    let mut out = Map::new();
    let mut notes = Vec::new();
    let mut host_docker = false;
    let mut uses_docker_ro = false;
    let mut main: Option<(String, u16)> = None;
    for (name, svc) in services {
        let mut m = svc.as_object().cloned().unwrap_or_default();
        let xr = m.remove("x-runtipi").unwrap_or(Value::Null);
        let is_main = xr.get("is_main").and_then(|v| v.as_bool()) == Some(true);
        let port = xr.get("internal_port").and_then(|v| v.as_u64()).map(|p| p as u16);
        if is_main || main.is_none() {
            main = Some((name.clone(), port.or_else(|| cfg.get("port").and_then(|v| v.as_u64()).map(|p| p as u16)).unwrap_or(80)));
        }
        normalize_service(app_id, name, &mut m, &mut notes, &mut host_docker, &mut uses_docker_ro);
        out.insert(name.clone(), Value::Object(m));
    }
    let (main_name, main_port) = main.ok_or_else(|| anyhow!("нет сервисов"))?;
    let mut compose = Map::new();
    compose.insert("services".into(), Value::Object(out));
    if let Some(v) = yml.get("volumes") { compose.insert("volumes".into(), v.clone()); }
    super::bind_named_volumes(&mut compose);
    compose.insert("networks".into(), json!({ "appnet": { "external": true, "name": "${APP_NET}" } }));
    Ok((Value::Object(compose), main_name, main_port, notes, host_docker, uses_docker_ro))
}

/// Динамический compose Runtipi (docker-compose.json, schemaVersion 2).
fn translate_dynamic(dc: &Value, cfg: &Value) -> Result<Translated> {
    let app_id = cfg.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let services = dc.get("services").and_then(|v| v.as_array()).ok_or_else(|| anyhow!("services не массив"))?;
    let main = services.iter().find(|s| s.get("isMain").and_then(|v| v.as_bool()) == Some(true)).or_else(|| services.first()).ok_or_else(|| anyhow!("нет сервисов"))?;
    let main_name = main.get("name").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("у main нет name"))?.to_string();
    let main_port = main.get("internalPort").and_then(|v| v.as_u64()).or_else(|| cfg.get("port").and_then(|v| v.as_u64())).unwrap_or(80) as u16;

    // ---- compose ----
    let mut out_services = Map::new();
    let mut notes = Vec::new();
    let mut host_docker = false;
    let mut uses_docker_ro = false;
    for s in services {
        let name = s.get("name").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("сервис без name"))?;
        let mut svc = Map::new();
        let mut docker_ro = false;
        svc.insert("image".into(), s.get("image").cloned().unwrap_or(Value::Null));
        svc.insert("restart".into(), json!("unless-stopped"));
        if let Some(env) = s.get("environment").and_then(|v| v.as_array()) {
            let mut m = Map::new();
            for e in env {
                if let (Some(k), Some(v)) = (e.get("key").and_then(|v| v.as_str()), e.get("value")) {
                    m.insert(k.into(), Value::String(scalar_to_string(v)));
                }
            }
            if !m.is_empty() {
                svc.insert("environment".into(), Value::Object(m));
            }
        }
        if let Some(vols) = s.get("volumes").and_then(|v| v.as_array()) {
            let mut list = Vec::new();
            for v in vols {
                let hp = v.get("hostPath").and_then(|x| x.as_str()).unwrap_or("");
                let cp = v.get("containerPath").and_then(|x| x.as_str()).unwrap_or("");
                if hp.is_empty() || cp.is_empty() {
                    continue;
                }
                if hp.contains("docker.sock") {
                    if RAW_DOCKER_SOCKET_APPS.contains(&app_id) {
                        host_docker = true;
                    } else {
                        docker_ro = true; // вместо сокета — read-only прокси через DOCKER_HOST
                        continue;
                    }
                }
                let ro = v.get("readOnly").and_then(|x| x.as_bool()) == Some(true);
                list.push(Value::String(format!("{hp}:{cp}{}", if ro { ":ro" } else { "" })));
            }
            if !list.is_empty() {
                svc.insert("volumes".into(), Value::Array(list));
            }
        }
        for (from, to) in [("command", "command"), ("entrypoint", "entrypoint"), ("user", "user"), ("workingDir", "working_dir"), ("hostname", "hostname"), ("privileged", "privileged"), ("readOnly", "read_only"), ("shmSize", "shm_size"), ("capAdd", "cap_add"), ("capDrop", "cap_drop"), ("devices", "devices"), ("securityOpt", "security_opt"), ("sysctls", "sysctls"), ("ulimits", "ulimits"), ("stopGracePeriod", "stop_grace_period"), ("stopSignal", "stop_signal"), ("tty", "tty"), ("stdinOpen", "stdin_open"), ("logging", "logging"), ("dns", "dns"), ("pid", "pid"), ("deploy", "deploy"), ("extraLabels", "labels")] {
            if let Some(v) = s.get(from) {
                if !v.is_null() {
                    svc.insert(to.into(), v.clone());
                }
            }
        }
        if let Some(eh) = s.get("extraHosts").and_then(|v| v.as_array()) {
            svc.insert("extra_hosts".into(), Value::Array(eh.clone()));
        }
        if let Some(d) = s.get("dependsOn") {
            svc.insert("depends_on".into(), d.clone());
        }
        if let Some(h) = s.get("healthCheck").and_then(|v| v.as_object()) {
            let mut hc = Map::new();
            for (from, to) in [("test", "test"), ("interval", "interval"), ("timeout", "timeout"), ("retries", "retries"), ("startPeriod", "start_period"), ("startInterval", "start_interval")] {
                if let Some(v) = h.get(from) {
                    hc.insert(to.into(), v.clone());
                }
            }
            svc.insert("healthcheck".into(), Value::Object(hc));
        }
        if let Some(ports) = s.get("addPorts").and_then(|v| v.as_array()) {
            for p in ports {
                notes.push(format!("порт {}→{} ({}) не публикуется: доступ только через edge/gate", p.get("hostPort").and_then(|v| v.as_u64()).unwrap_or(0), p.get("containerPort").and_then(|v| v.as_u64()).unwrap_or(0), name));
            }
        }
        match s.get("networkMode").and_then(|v| v.as_str()) {
            Some(nm) => {
                svc.insert("network_mode".into(), json!(nm));
                notes.push(format!("{name}: network_mode={nm} из каталога"));
            }
            None => {
                let mut nets = json!({ "appnet": { "aliases": [format!("{app_id}_{name}_1")] } });
                if docker_ro { nets["dockerro"] = json!({}); }
                svc.insert("networks".into(), nets);
            }
        }
        if docker_ro {
            let env = svc.entry("environment").or_insert_with(|| Value::Object(Map::new()));
            if let Some(m) = env.as_object_mut() {
                m.insert("DOCKER_HOST".into(), json!(DOCKER_RO_HOST));
            }
            uses_docker_ro = true;
            notes.push(format!("{name}: docker.sock заменён на read-only прокси (DOCKER_HOST={DOCKER_RO_HOST})"));
        }
        out_services.insert(name.into(), Value::Object(svc));
    }
    let compose = json!({ "services": Value::Object(out_services), "networks": { "appnet": { "external": true, "name": "${APP_NET}" } } });
    Ok((compose, main_name, main_port, notes, host_docker, uses_docker_ro))
}
