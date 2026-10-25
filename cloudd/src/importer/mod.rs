//! Store catalog aggregator (2.6, 3.7): the compose and metadata are foreign, ours is only the manifest contract.
//! Sources: Runtipi, Coolify, Umbrel. Duplicates are allowed — every app has a source badge; ids from non-primary
//! sources get a `-<source>` suffix so they do not collide (id = the compose project and network name). Hand-written
//! manifests (`origin: manual`) are not overwritten. Common rules: ports are not published, the network is external and
//! managed by the core, data — bind in tank/apps, docker.sock — only through the read-only proxy.

pub mod coolify;
pub mod runtipi;
pub mod umbrel;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Apps that manage Docker (create/exec) — they need the full socket; for the rest docker.sock is replaced with the read-only
/// proxy.
pub const RAW_DOCKER_SOCKET_APPS: &[&str] = &["portainer", "dockge", "komodo", "watchtower", "yacht", "kasm-workspaces", "olivetin", "semaphore", "coolify", "dokploy", "cup"];
pub const DOCKER_RO_HOST: &str = "tcp://cloudos-docker-ro:2375";
pub const DOCKER_RO_NET: &str = "cloudos-docker-ro-net";

#[derive(Debug, Default)]
pub struct ImportReport {
    pub source: String,
    pub imported: Vec<String>,
    pub skipped_manual: Vec<String>,
    pub skipped: Vec<(String, String)>,
}

/// Adapter result: everything needed to write the app into the
/// Store.
pub struct Imported {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub compose: Value,
    pub main_service: String,
    pub main_port: u16,
    pub env: Map<String, Value>,
    pub settings: Vec<Value>,
    pub notes: Vec<String>,
    pub host_docker: bool,
    pub uses_docker_ro: bool,
    /// Icon: a local file (bytes + extension) or a URL.
    pub icon_file: Option<(Vec<u8>, &'static str)>,
    pub icon_url: Option<String>,
    pub gallery: Vec<String>,
    pub readme: Option<String>,
    pub website: Option<String>,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
    pub upstream: Value,
    pub origin: &'static str,
    /// Catalog hints for the login section: default login and password literals (Umbrel defaultUsername/defaultPassword).
    pub login_hints: (Option<String>, Option<String>),
}

pub async fn import(source: &str, store_dir: &Path, only: &[String]) -> Result<Vec<ImportReport>> {
    let mut out = Vec::new();
    match source {
        "runtipi" => out.push(runtipi::import(store_dir, only).await?),
        "coolify" => out.push(coolify::import(store_dir, only).await?),
        "umbrel" => out.push(umbrel::import(store_dir, only).await?),
        "all" => {
            for s in ["runtipi", "coolify", "umbrel"] {
                match Box::pin(import(s, store_dir, only)).await {
                    Ok(mut r) => out.append(&mut r),
                    Err(e) => out.push(ImportReport { source: s.into(), skipped: vec![("*".into(), format!("источник недоступен: {e:#}"))], ..Default::default() }),
                }
            }
        }
        other => bail!("источник {other} не поддерживается: runtipi | coolify | umbrel | all"),
    }
    Ok(out)
}

/// Download a tar.gz and unpack it into a temporary directory; return the
/// archive root.
pub async fn fetch_tarball(url: &str, tag: &str) -> Result<(PathBuf, PathBuf)> {
    let tmp = std::env::temp_dir().join(format!("cloudd-import-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let tgz = tmp.join("src.tar.gz");
    let bytes = reqwest::Client::new().get(url).header("User-Agent", "cloudd").send().await?.error_for_status()?.bytes().await?;
    std::fs::write(&tgz, &bytes)?;
    let st = tokio::process::Command::new("tar").arg("-xzf").arg(&tgz).arg("-C").arg(&tmp).status().await?;
    if !st.success() {
        bail!("tar: {st}");
    }
    let root = std::fs::read_dir(&tmp)?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.is_dir()).ok_or_else(|| anyhow!("архив пуст"))?;
    Ok((tmp, root))
}

/// Is there already a manual manifest (or a manifest from another source) — then
/// leave it alone.
pub fn existing_origin(store_dir: &Path, id: &str) -> Option<String> {
    let text = std::fs::read_to_string(store_dir.join(id).join("manifest.yaml")).ok()?;
    Some(serde_yaml_ng::from_str::<Value>(&text).ok().and_then(|v| v.get("origin").and_then(|o| o.as_str()).map(|s| s.to_string())).unwrap_or_else(|| "manual".into()))
}

/// Write the app into the Store: manifest.yaml, compose.yaml, icon,
/// description.
pub fn write_app(store_dir: &Path, a: Imported) -> Result<()> {
    crate::service::validate_name(&a.id).map_err(|e| anyhow!("{e}"))?;
    let dst = store_dir.join(&a.id);
    std::fs::create_dir_all(&dst)?;
    let icon = match (&a.icon_file, &a.icon_url) {
        (Some((bytes, ext)), _) => {
            std::fs::write(dst.join(format!("logo.{ext}")), bytes)?;
            Some(format!("./logo.{ext}"))
        }
        (None, Some(u)) => Some(u.clone()),
        _ => None,
    };
    let mut notes = a.notes.clone();
    if a.host_docker {
        notes.push("монтирует /var/run/docker.sock хоста — приложение платформенного уровня доверия (в срезе 1 без socket-proxy)".into());
    }
    let mut compose = a.compose.clone();
    if a.uses_docker_ro {
        compose["networks"]["dockerro"] = json!({ "external": true, "name": DOCKER_RO_NET });
    }
    let login = derive_login(&a.env, a.login_hints.0.clone(), a.login_hints.1.clone(), &a.main_service, &serde_yaml_ng::to_string(&compose).unwrap_or_default());
    let manifest = json!({
        "schema": 1,
        "name": a.id,
        "title": a.title,
        "description": a.description,
        "icon": icon,
        "type": "app",
        "compose": "./compose.yaml",
        "provides": [],
        "requires": [],
        "route": { "mode": "port", "open": "iframe" }, // the edge rewrites frame-ancestors to the Desktop origin; ↗ in the window header is the fallback exit
        "auth": "own",
        "env": Value::Object(a.env),
        "data": [format!("tank/apps/{}", a.id)],
        "health": Value::Null,
        "endpoint": { "service": a.main_service, "port": a.main_port },
        "hooks": {},
        "origin": a.origin,
        "upstream": a.upstream,
        "settings": a.settings,
        "host_docker_socket": a.host_docker,
        "notes": notes,
        "gallery": a.gallery,
        "readme": a.readme,
        "website": a.website,
        "tags": a.tags,
        "login": login,
    });
    let mut manifest = manifest;
    manifest["upstream"]["categories"] = json!(a.categories);
    let header = format!("# Imported from the {src} catalog by `cloudd store import {src}`; manual edits will be overwritten —\n# for a hand-maintained version set `origin: manual`. Compose and metadata are upstream, the contract (route/endpoint/env/data) is ours.\n", src = a.origin);
    std::fs::write(dst.join("manifest.yaml"), header + &serde_yaml_ng::to_string(&manifest)?)?;
    std::fs::write(dst.join("compose.yaml"), format!("# {}: generated by the importer ({}); the network is external (created by cloudd), ports are not published.\n", a.id, a.origin) + &serde_yaml_ng::to_string(&compose)?)?;
    Ok(())
}

pub fn scalar_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Common finishing of a regular compose service (Coolify, Umbrel, static Runtipi): remove ports/container_name/traefik
/// labels, add restart and the appnet network, replace docker.sock with the proxy (unless a managing app).
pub fn normalize_service(app_id: &str, name: &str, svc: &mut Map<String, Value>, notes: &mut Vec<String>, host_docker: &mut bool, uses_docker_ro: &mut bool) {
    if let Some(ports) = svc.remove("ports") {
        for p in ports.as_array().cloned().unwrap_or_default() {
            notes.push(format!("порт {} ({name}) не публикуется: доступ только через edge/gate", scalar_to_string(&p)));
        }
    }
    svc.remove("container_name");
    // env_file pointing at files we do not have (Coolify settings.env) — remove, the values come via
    // environment/.env
    if svc.remove("env_file").is_some() {
        notes.push(format!("{name}: env_file из каталога проигнорирован (значения — через настройки приложения)"));
    }
    if let Some(labels) = svc.get("labels").cloned() {
        let keep: Value = match labels {
            Value::Array(a) => Value::Array(a.into_iter().filter(|l| { let t = l.as_str().unwrap_or(""); !t.starts_with("traefik.") && !t.starts_with("runtipi.") && !t.starts_with("coolify.") }).collect()),
            Value::Object(o) => Value::Object(o.into_iter().filter(|(k, _)| !k.starts_with("traefik.") && !k.starts_with("runtipi.") && !k.starts_with("coolify.")).collect()),
            _ => Value::Null,
        };
        let empty = match &keep { Value::Array(a) => a.is_empty(), Value::Object(o) => o.is_empty(), _ => true };
        if empty { svc.remove("labels"); } else { svc.insert("labels".into(), keep); }
    }
    // compose does not know the Coolify extensions in volumes (is_directory /
    // isDirectory)
    if let Some(Value::Array(vols)) = svc.get_mut("volumes") {
        for v in vols.iter_mut() {
            if let Some(o) = v.as_object_mut() { o.remove("is_directory"); o.remove("isDirectory"); }
        }
    }
    let mut docker_ro = false;
    if let Some(vols) = svc.get("volumes").and_then(|v| v.as_array()) {
        if vols.iter().any(|v| scalar_to_string(v).contains("docker.sock")) {
            if RAW_DOCKER_SOCKET_APPS.iter().any(|r| app_id.starts_with(r)) {
                *host_docker = true;
            } else {
                docker_ro = true;
                let kept: Vec<Value> = vols.iter().filter(|v| !scalar_to_string(v).contains("docker.sock")).cloned().collect();
                svc.insert("volumes".into(), Value::Array(kept));
            }
        }
    }
    if !svc.contains_key("restart") {
        svc.insert("restart".into(), json!("unless-stopped"));
    }
    if !svc.contains_key("network_mode") {
        let mut nets = json!({ "appnet": { "aliases": [format!("{app_id}_{name}_1")] } });
        if docker_ro { nets["dockerro"] = json!({}); }
        svc.insert("networks".into(), nets);
    }
    if docker_ro {
        match svc.get_mut("environment") {
            Some(Value::Array(a)) => a.push(json!(format!("DOCKER_HOST={DOCKER_RO_HOST}"))),
            Some(Value::Object(o)) => { o.insert("DOCKER_HOST".into(), json!(DOCKER_RO_HOST)); }
            _ => { svc.insert("environment".into(), json!({ "DOCKER_HOST": DOCKER_RO_HOST })); }
        }
        *uses_docker_ro = true;
        notes.push(format!("{name}: docker.sock заменён на read-only прокси (DOCKER_HOST={DOCKER_RO_HOST})"));
    }
}

/// Coolify: volumes with `content:` (an inline file) → compose `configs` with `content` +
/// `configs:` on the service.
pub fn inline_volumes_to_configs(compose: &mut Map<String, Value>) {
    let mut configs = compose.get("configs").and_then(|c| c.as_object()).cloned().unwrap_or_default();
    let mut n = 0;
    if let Some(services) = compose.get_mut("services").and_then(|v| v.as_object_mut()) {
        for (sname, svc) in services.iter_mut() {
            let Some(obj) = svc.as_object_mut() else { continue };
            let Some(Value::Array(vols)) = obj.get("volumes").cloned() else { continue };
            let mut keep = Vec::new();
            let mut cfgs: Vec<Value> = obj.get("configs").and_then(|c| c.as_array()).cloned().unwrap_or_default();
            for v in vols {
                match v.as_object() {
                    Some(o) if o.contains_key("content") => {
                        n += 1;
                        let key = format!("{}-inline-{n}", sname.replace(['_', '.'], "-"));
                        configs.insert(key.clone(), json!({ "content": o.get("content").cloned().unwrap_or(Value::String(String::new())) }));
                        let target = o.get("target").cloned().unwrap_or(Value::Null);
                        cfgs.push(json!({ "source": key, "target": target }));
                    }
                    _ => keep.push(v),
                }
            }
            obj.insert("volumes".into(), Value::Array(keep));
            if !cfgs.is_empty() { obj.insert("configs".into(), Value::Array(cfgs)); }
        }
    }
    if !configs.is_empty() { compose.insert("configs".into(), Value::Object(configs)); }
}

/// Named volumes: stay named (Docker fills an empty volume with the image content — bind cannot do that), but live
/// not in /var/lib/docker but in ${APP_DATA_DIR}/<name> via driver_opts bind (2.8: app data goes to tank/apps).
/// Coolify does not declare volumes at the top of the file, so any source that does not look like a path counts as
/// named.
pub fn bind_named_volumes(compose: &mut Map<String, Value>) {
    let mut declared: std::collections::BTreeSet<String> = compose.get("volumes").and_then(|v| v.as_object()).map(|o| o.keys().cloned().collect()).unwrap_or_default();
    let is_named = |src: &str| !src.is_empty() && !src.starts_with('/') && !src.starts_with('.') && !src.starts_with('~') && !src.starts_with('$');
    if let Some(services) = compose.get("services").and_then(|v| v.as_object()) {
        for (_, svc) in services {
            if let Some(vols) = svc.get("volumes").and_then(|v| v.as_array()) {
                for v in vols {
                    match v {
                        Value::String(s) => { if let Some((src, _)) = s.split_once(':') { if is_named(src) { declared.insert(src.to_string()); } } }
                        Value::Object(o) if o.get("type").and_then(|t| t.as_str()) == Some("volume") => { if let Some(src) = o.get("source").and_then(|x| x.as_str()) { declared.insert(src.to_string()); } }
                        _ => {}
                    }
                }
            }
        }
    }
    if declared.is_empty() { compose.remove("volumes"); return; }
    let mut vols = Map::new();
    for name in declared {
        let dir = name.replace([' '], "-");
        vols.insert(name.clone(), json!({ "driver": "local", "driver_opts": { "type": "none", "o": "bind", "device": format!("${{APP_DATA_DIR}}/{dir}") } }));
    }
    compose.insert("volumes".into(), Value::Object(vols));
}

/// The manifest `login` section: which env variable is the web UI sign-in password and which is the login. The main signal is how the
/// variable is used in compose (`GF_SECURITY_ADMIN_PASSWORD=${SERVICE_PASSWORD_GRAFANA}` — a sign-in password; `DB_PASSWORD=…`,
/// `*_AUTH_TOKEN=…` — infrastructure), the second — its own name. `user_hint`/`password_hint` — catalog literals (Umbrel
/// defaultUsername/defaultPassword). Without a confident candidate — `mode: app` (the sign-in is set up in the app). Refinements — a
/// `login.yaml` file next to the manifest.
pub fn derive_login(env: &Map<String, Value>, user_hint: Option<String>, password_hint: Option<String>, main_service: &str, compose_text: &str) -> Value {
    const INFRA: &[&str] = &["SECRET", "JWT", "SALT", "ENCRYPT", "ENCRYPTION", "MASTER", "DB", "DATABASE", "POSTGRES", "PG", "PGSQL", "MYSQL", "MARIADB", "REDIS", "MONGO", "RABBIT", "RABBITMQ", "MINIO", "SMTP", "MAIL", "S3", "RPC", "METRIC", "METRICS", "API", "KEY", "TOKEN", "SESSION", "COOKIE", "HASH", "SIGN", "SIG", "CLICKHOUSE", "ELASTIC", "KAFKA", "NATS", "MQTT", "LDAP", "OIDC", "OAUTH", "VAPID", "WEBHOOK", "AGENT", "PEPPER", "SEED", "ENC", "PROXY", "CACHE", "QUEUE", "BROKER", "STORAGE", "BUCKET", "SUDO", "VNC", "FTP", "SSH", "WIREGUARD", "VPN", "TURN", "STUN", "SENTRY", "MEILI", "TYPESENSE", "MEMCACHED", "INFLUX", "RUNNERS", "WORKER", "INTERNAL", "PRIVATE", "CERT", "TLS", "SSL", "GPG", "PGP", "VAULT", "KEYCLOAK_DB"];
    const UI: &[&str] = &["ADMIN", "WEB", "WEBUI", "UI", "ROOT", "LOGIN", "AUTH", "ACCESS", "USER", "PANEL", "DASHBOARD", "SUPERUSER", "INITIAL", "DEFAULT", "DEFAULTUSER", "BASIC", "OWNER", "PORTAL", "CONSOLE", "GUI", "FRONTEND"];
    fn hits(words: &[&str], name: &str) -> bool {
        let parts: Vec<&str> = name.split('_').collect();
        words.iter().any(|w| parts.contains(w) || (w.len() >= 4 && name.contains(w)) || (w.len() < 4 && name.ends_with(w)))
    }
    fn suffix_of(u: &str, prefixes: &[&str]) -> String {
        for p in prefixes {
            if let Some(r) = u.strip_prefix(p) {
                return r.to_string();
            }
        }
        u.to_string()
    }
    /// Names of compose variables that are assigned `${var}` / `$var` / `${var:-…}`.
    fn assigned_to(compose: &str, var: &str) -> Vec<String> {
        let re = regex_lite::Regex::new(&format!(r#"(?m)([A-Za-z][A-Za-z0-9_]*)\s*[:=]\s*["']?\$\{{?{}(?:[^A-Za-z0-9_]|$)"#, regex_lite::escape(var))).unwrap();
        re.captures_iter(compose).map(|c| c[1].to_uppercase()).collect()
    }
    /// Value of the neighbouring compose variable `BASE_(USER|USERNAME|NAME|EMAIL|LOGIN)`: `${VAR}`/`${VAR:-lit}`/`$VAR` → (Some(VAR), lit), a literal → (None,
    /// Some(lit)).
    fn sibling_user(compose: &str, base: &str) -> Option<(Option<String>, Option<String>)> {
        let re = regex_lite::Regex::new(&format!(r#"(?mi)^\s*-?\s*["']?{}_?(USER_?NAME|USERNAME|USER|NAME|EMAIL|LOGIN)["']?\s*[:=]\s*["']?([^"'\n#]*)"#, regex_lite::escape(base))).unwrap();
        let c = re.captures(compose)?;
        let raw = c[2].trim().trim_end_matches(&['"', '\''][..]).to_string();
        if raw.is_empty() { return None; }
        if let Some(inner) = raw.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
            let (var, def) = match inner.split_once(":-").or_else(|| inner.split_once(":?")).or_else(|| inner.split_once('-')) { Some((v, d)) => (v.to_string(), Some(d.to_string()).filter(|d| !d.is_empty())), None => (inner.to_string(), None) };
            return Some((Some(var), def));
        }
        if let Some(var) = raw.strip_prefix('$') { return Some((Some(var.to_string()), None)); }
        if raw.contains('$') { return None; }
        Some((None, Some(raw)))
    }
    let main = main_service.to_uppercase().replace(['-', '_', '.'], "");
    // if the app itself is the "infrastructure" (MinIO, Postgres, pgAdmin), its name in the variable is not a sign of a foreign
    // secret
    let infra: Vec<&str> = INFRA.iter().copied().filter(|w| !(main.len() >= 3 && (main.contains(*w) || w.contains(main.as_str())))).collect();
    let is_gen = |k: &str| env.get(k).map(|v| v.is_object()).unwrap_or(false);
    let empty_literal = |k: &str| env.get(k).and_then(|v| v.as_str()).map(|s| s.is_empty()).unwrap_or(true);
    let pw_prefixes: &[&str] = &["SERVICE_PASSWORD_64_", "SERVICE_PASSWORD_"];
    let mut pw: Vec<(String, i32)> = Vec::new();
    for k in env.keys() {
        let u = k.to_uppercase();
        if u.starts_with("SERVICE_URL") || u.starts_with("SERVICE_FQDN") || !(u.contains("PASSWORD") || u.contains("PASSWD") || u.ends_with("_PASS") || u.contains("_PASS_")) {
            continue;
        }
        // password policy flags (PASSWORDLOWERCASE=1, PASSWORD_MIN_LENGTH) and boolean/numeric literals are not
        // passwords
        const POLICY: &[&str] = &["LOWERCASE", "UPPERCASE", "LENGTH", "NUMERIC", "SYMBOL", "REQUIRE", "POLICY", "RESET", "CHANGE", "EXPIRE", "HISTORY", "COMPLEXITY", "ATTEMPT", "VALIDAT", "REGEX", "ENABLE", "DISABLE", "ALLOW", "HASH", "FILE", "PATH", "URL", "CHECK", "STRENGTH", "RULE", "TTL", "TIMEOUT", "LOGIN_PASSWORD_ENABLED"];
        if POLICY.iter().any(|w| u.contains(w)) { continue; }
        if let Some(lit) = env.get(k).and_then(|v| v.as_str()) {
            let l = lit.trim().to_lowercase();
            if !l.is_empty() && (l.len() < 4 || ["true", "false", "yes", "no", "null", "none", "auto"].contains(&l.as_str()) || l.chars().all(|c| c.is_ascii_digit())) { continue; }
        }
        let suffix = suffix_of(&u, pw_prefixes);
        let mut score = 0;
        if hits(&infra, &suffix) { score -= 100; }
        if hits(UI, &suffix) { score += 5; }
        let flat = suffix.replace('_', "");
        if main.len() >= 3 && (flat.contains(&main) || main.contains(&flat)) { score += 4; }
        if env.contains_key(&format!("SERVICE_USER_{suffix}")) { score += 6; }
        if u == "APP_PASSWORD" || suffix == "PASSWORD" || suffix == "PASS" { score += 3; }
        if is_gen(k) { score += 1; } else if empty_literal(k) { score -= 6; } // an empty literal: the user has not set it yet
        // how it is used in compose: the left side of the
        // assignment
        for lhs in assigned_to(compose_text, k) {
            if lhs == u { continue; }
            if hits(&infra, &lhs) { score -= 100; }
            if hits(UI, &lhs) { score += 8; }
            if lhs.contains("PASSWORD") || lhs.contains("PASSWD") || lhs.ends_with("_PASS") { score += 2; }
        }
        pw.push((k.clone(), score));
    }
    pw.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let password = pw.first().filter(|(_, s)| *s > -5).map(|(k, _)| k.clone());
    // login: SERVICE_USER_<the same suffix>; the neighbouring compose variable by name stem (GF_SECURITY_ADMIN_PASSWORD →
    // GF_SECURITY_ADMIN_USER); otherwise an env variable with USER/LOGIN/EMAIL without infrastructure words; otherwise the catalog literal
    let mut user: Option<String> = None;
    if let Some(p) = &password {
        let cand = format!("SERVICE_USER_{}", suffix_of(&p.to_uppercase(), pw_prefixes));
        if env.contains_key(&cand) { user = Some(cand); }
        if user.is_none() {
            let mut lhss = assigned_to(compose_text, p);
            lhss.push(p.to_uppercase());
            for lhs in lhss {
                let base = lhs.trim_end_matches("_PASSWORD").trim_end_matches("_PASSWD").trim_end_matches("_PASS").trim_end_matches("PASSWORD").trim_end_matches("PASSWD").trim_end_matches("PASS").trim_end_matches('_');
                if base.is_empty() { continue; }
                if let Some((var, lit)) = sibling_user(compose_text, base) {
                    user = match var { Some(v) if env.contains_key(&v) => Some(v), Some(_) => lit, None => lit };
                    if user.is_some() { break; }
                }
            }
        }
    }
    if user.is_none() {
        let mut us: Vec<(String, i32)> = Vec::new();
        for k in env.keys() {
            let u = k.to_uppercase();
            let looks = (u.contains("USER") || u.contains("LOGIN") || u.contains("EMAIL") || u.contains("ADMIN_NAME")) && !u.contains("PASSWORD") && !u.contains("USERS") && !u.contains("USERID") && !u.contains("USER_ID") && !u.contains("UID") && !u.starts_with("SERVICE_URL") && !u.starts_with("SERVICE_FQDN");
            if !looks { continue; }
            let suffix = suffix_of(&u, &["SERVICE_USER_"]);
            let mut score = 0;
            if hits(&infra, &suffix) { score -= 100; }
            if hits(UI, &suffix) { score += 3; }
            if is_gen(k) { score += 1; } else if empty_literal(k) { score -= 6; }
            for lhs in assigned_to(compose_text, k) { if hits(&infra, &lhs) { score -= 100; } if hits(UI, &lhs) { score += 5; } }
            us.push((k.clone(), score));
        }
        us.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        user = us.first().filter(|(_, s)| *s > -5).map(|(k, _)| k.clone());
    }
    match (password, password_hint) {
        (Some(p), _) => json!({ "mode": "generated", "user": user.or(user_hint), "password": p, "path": Value::Null, "note": Value::Null }),
        (None, Some(ph)) => json!({ "mode": "default", "user": user.or(user_hint), "password": ph, "path": Value::Null, "note": "пароль по умолчанию из образа приложения — смените после первого входа" }),
        (None, None) => json!({ "mode": "app", "user": user.or(user_hint), "password": Value::Null, "path": Value::Null, "note": Value::Null }),
    }
}
