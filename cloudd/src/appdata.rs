//! Каталоги данных приложений (`${APP_DATA_DIR}/…`): кто их владелец и починка прав.
//!
//! Первопричина «502 после установки»: образы из каталогов часто работают не от root (Umbrel: `user: 1000:1000`, linuxserver: PUID/PGID,
//! bitnami: 1001, grafana: 472…), а bind-каталог, созданный ядром от root, им недоступен → EACCES → контейнер перезапускается по кругу,
//! edge отдаёт 502. Docker решает это только для named volumes (копирует содержимое и владельца пути из образа); для bind-монтирований
//! платформы-источники полагаются на соглашение хоста (Umbrel и Runtipi: всё под 1000:1000). Здесь владелец выводится из compose и образа:
//!   1. `user:` сервиса — число, `uid:gid`, `${VAR:-default}`; имя — через /etc/passwd образа;
//!   2. PUID/PGID из environment сервиса (соглашение linuxserver);
//!   3. `USER` из конфигурации образа (число или имя через /etc/passwd);
//!   4. владелец пути монтирования внутри образа, если не root (то, что сделал бы Docker для named volume);
//!   5. иначе 1000:1000 — соглашение Umbrel/Runtipi; для образов, работающих от root, владелец безразличен.
//! Новым и ещё пустым каталогам владелец выставляется до `compose up` ([`Service::prepare_data_dirs`]); уже наполненным — [`Service::app_repair`]
//! (кнопка в карточке и автоматически из [`Service::spawn_watch`], если после выкатки контейнер перезапускается с ошибкой прав в логах).
use crate::service::{data_subdirs, ensure_data_dir, Service};
use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Соглашение Umbrel/Runtipi: данные приложений принадлежат первому пользователю хоста.
pub const DEFAULT_OWNER: (u32, u32) = (1000, 1000);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub uid: u32,
    pub gid: u32,
    /// Откуда взят (для событий и отладки).
    pub why: String,
    /// Выведен из compose или образа (true) — или взят по умолчанию (false): по умолчанию чужие данные не перекрашиваем.
    pub strong: bool,
}

fn default_owner(why: &str) -> Owner { Owner { uid: DEFAULT_OWNER.0, gid: DEFAULT_OWNER.1, why: format!("по умолчанию 1000:1000 — {why}"), strong: false } }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// Подкаталог внутри `${APP_DATA_DIR}` ("" — сам каталог данных).
    pub subdir: String,
    pub service: String,
    /// Путь внутри контейнера.
    pub target: String,
}

/// `${APP_DATA_DIR}/<subdir>` → subdir; сам `${APP_DATA_DIR}` → ""; файлы (точка в последнем сегменте) и выход наверх — None.
fn subdir_of(src: &str) -> Option<String> {
    let s = src.trim();
    let rest = s.strip_prefix("${APP_DATA_DIR}")?;
    if rest.is_empty() { return Some(String::new()); }
    let rest = rest.strip_prefix('/')?.trim_end_matches('/');
    if rest.is_empty() { return Some(String::new()); }
    if rest.contains("..") || rest.rsplit('/').next().unwrap_or("").contains('.') { return None; }
    if !rest.chars().all(|c| c.is_ascii_alphanumeric() || "_-./".contains(c)) { return None; }
    Some(rest.to_string())
}

/// bind-монтирования `${APP_DATA_DIR}/…:<target>` по сервисам (короткая и длинная форма). Named volumes, даже с driver_opts bind
/// внутрь `${APP_DATA_DIR}`, не здесь: их Docker инициализирует сам.
pub fn data_mounts(compose: &Value) -> Vec<Mount> {
    let mut out = Vec::new();
    let Some(services) = compose.get("services").and_then(|s| s.as_object()) else { return out };
    for (sname, svc) in services {
        let Some(vols) = svc.get("volumes").and_then(|v| v.as_array()) else { continue };
        for v in vols {
            let (src, target) = match v {
                Value::String(s) => {
                    let mut it = s.splitn(3, ':');
                    let src = it.next().unwrap_or("").to_string();
                    let Some(t) = it.next() else { continue };
                    (src, t.to_string())
                }
                Value::Object(o) => {
                    let (Some(src), Some(t)) = (o.get("source").and_then(|x| x.as_str()), o.get("target").and_then(|x| x.as_str())) else { continue };
                    (src.to_string(), t.to_string())
                }
                _ => continue,
            };
            if let Some(subdir) = subdir_of(&src) { out.push(Mount { subdir, service: sname.clone(), target }); }
        }
    }
    out.sort_by(|a, b| a.subdir.len().cmp(&b.subdir.len()).then_with(|| a.subdir.cmp(&b.subdir)));
    out
}

/// Подстановка `${VAR}`, `${VAR:-def}`, `${VAR-def}`, `${VAR:?e}`, `$VAR` по окружению приложения. None — переменная не задана и нет default.
pub fn expand(expr: &str, env: &HashMap<String, String>) -> Option<String> {
    let mut out = String::new();
    let b = expr.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' && i + 1 < b.len() {
            if b[i + 1] == b'{' {
                let end = expr[i..].find('}')? + i;
                let inner = &expr[i + 2..end];
                let (name, def) = if let Some(p) = inner.find(":-") { (&inner[..p], Some(&inner[p + 2..])) }
                    else if let Some(p) = inner.find(":?") { (&inner[..p], None) }
                    else if let Some(p) = inner.find('-') { (&inner[..p], Some(&inner[p + 1..])) }
                    else { (inner, None) };
                match env.get(name).filter(|v| !v.is_empty()) {
                    Some(v) => out.push_str(v),
                    None => out.push_str(def?),
                }
                i = end + 1;
                continue;
            }
            let j = expr[i + 1..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map(|p| i + 1 + p).unwrap_or(b.len());
            if j > i + 1 {
                out.push_str(env.get(&expr[i + 1..j])?);
                i = j;
                continue;
            }
        }
        let ch = expr[i..].chars().next()?;
        out.push(ch);
        i += ch.len_utf8();
    }
    Some(out)
}

/// "1000" / "1000:1000" → (uid, gid?); имена — None.
pub fn parse_ids(s: &str) -> Option<(u32, Option<u32>)> {
    let s = s.trim().trim_matches(|c| c == '"' || c == '\'');
    let (u, g) = match s.split_once(':') { Some((u, g)) => (u, Some(g)), None => (s, None) };
    let uid = u.trim().parse::<u32>().ok()?;
    let gid = match g { Some(g) => Some(g.trim().parse::<u32>().ok()?), None => None };
    Some((uid, gid))
}

/// `user:` сервиса с подстановкой окружения (число YAML тоже).
pub fn service_user(svc: &Value, env: &HashMap<String, String>) -> Option<String> {
    let u = svc.get("user")?;
    let s = match u { Value::String(s) => s.clone(), Value::Number(n) => n.to_string(), _ => return None };
    expand(&s, env).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// environment сервиса (список `K=V` или карта) с подстановкой окружения приложения.
pub fn service_env(svc: &Value, env: &HashMap<String, String>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    match svc.get("environment") {
        Some(Value::Array(a)) => for e in a {
            if let Some(s) = e.as_str() { if let Some((k, v)) = s.split_once('=') { if let Some(v) = expand(v, env) { out.insert(k.trim().to_string(), v); } } }
        },
        Some(Value::Object(o)) => for (k, v) in o {
            let s = match v { Value::String(s) => s.clone(), Value::Number(n) => n.to_string(), Value::Bool(b) => b.to_string(), _ => continue };
            if let Some(v) = expand(&s, env) { out.insert(k.clone(), v); }
        },
        _ => {}
    }
    out
}

/// Владелец только по compose (без Docker): числовой `user:` или PUID/PGID.
pub fn owner_from_compose(svc: &Value, env: &HashMap<String, String>) -> Option<Owner> {
    if let Some(u) = service_user(svc, env) {
        if let Some((uid, gid)) = parse_ids(&u) { return Some(Owner { uid, gid: gid.unwrap_or(uid), why: format!("user: {u} в compose"), strong: true }); }
    }
    let senv = service_env(svc, env);
    if let Some(uid) = senv.get("PUID").and_then(|v| v.trim().parse::<u32>().ok()) {
        let gid = senv.get("PGID").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(uid);
        return Some(Owner { uid, gid, why: format!("PUID/PGID {uid}:{gid} в environment"), strong: true });
    }
    None
}

/// `name`, `name:group`, `uid:group`, `name:gid` → (uid, gid) по /etc/passwd образа (группа по имени — первичная группа пользователя).
pub fn resolve_name(user: &str, passwd: &str) -> Option<(u32, u32)> {
    let (u, g) = match user.split_once(':') { Some((u, g)) => (u.trim(), Some(g.trim())), None => (user.trim(), None) };
    let mut uid = u.parse::<u32>().ok();
    let mut pgid = None;
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 4 { continue; }
        let matches = if let Some(id) = uid { f[2].parse::<u32>().ok() == Some(id) } else { f[0] == u };
        if matches { uid = f[2].parse().ok(); pgid = f[3].parse().ok(); break; }
    }
    let uid = uid?;
    let gid = match g { Some(g) => g.parse::<u32>().ok().or(pgid)?, None => pgid.unwrap_or(uid) };
    Some((uid, gid))
}

fn tar_first_owner(buf: &[u8]) -> Option<(u32, u32)> {
    let mut a = tar::Archive::new(buf);
    let mut es = a.entries().ok()?;
    let e = es.next()?.ok()?;
    let h = e.header();
    Some((h.uid().ok()? as u32, h.gid().ok()? as u32))
}

fn tar_first_file(buf: &[u8]) -> Option<String> {
    let mut a = tar::Archive::new(buf);
    for e in a.entries().ok()? {
        let mut e = e.ok()?;
        if e.header().entry_type().is_file() {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut e, &mut s).ok()?;
            return Some(s);
        }
    }
    None
}

/// Похоже ли на ошибку прав в логах контейнера.
pub fn perm_error(logs: &str) -> bool {
    let re = regex_lite::Regex::new(r"(?i)permission denied|EACCES|EPERM|operation not permitted|not writable|read-only file system|unable to (create|open|write)|cannot (create|open|write|access)|mkdir[^\n]*(denied|failed)|chown[^\n]*(denied|not permitted|failed)").unwrap();
    re.is_match(logs)
}

fn owner_of(p: &Path) -> Option<(u32, u32)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(p).ok()?;
    Some((m.uid(), m.gid()))
}

/// В дереве нет ни одного файла — только (возможно вложенные) пустые каталоги, созданные ядром под монтирования.
fn tree_has_no_files(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { return false };
        if ft.is_dir() { if !tree_has_no_files(&e.path()) { return false; } } else { return false; }
    }
    true
}

/// chown -R без прохода по символьным ссылкам (lchown).
pub fn chown_recursive(dir: &Path, uid: u32, gid: u32) -> std::io::Result<()> {
    std::os::unix::fs::lchown(dir, Some(uid), Some(gid))?;
    if std::fs::symlink_metadata(dir)?.is_dir() {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let p = e.path();
            if e.file_type()?.is_dir() { chown_recursive(&p, uid, gid)?; } else { std::os::unix::fs::lchown(&p, Some(uid), Some(gid))?; }
        }
    }
    Ok(())
}

/// Сервис по имени контейнера compose: `<project>-<service>-<n>` → service.
pub fn service_of(project: &str, container: &str) -> String {
    let rest = container.strip_prefix(project).and_then(|r| r.strip_prefix('-')).unwrap_or(container);
    match rest.rsplit_once('-') { Some((s, n)) if n.chars().all(|c| c.is_ascii_digit()) => s.to_string(), _ => rest.to_string() }
}

impl Service {
    /// Владелец каталогов сервиса: compose → образ → по умолчанию. Образ скачивается заранее (Komodo потом возьмёт из кеша).
    pub async fn resolve_owner(&self, svc: &Value, target: &str, env: &HashMap<String, String>) -> Owner {
        if let Some(o) = owner_from_compose(svc, env) { return o; }
        let user = service_user(svc, env);
        if svc.get("build").is_some() { return default_owner("образ собирается на месте"); }
        let Some(image) = svc.get("image").and_then(|i| i.as_str()).and_then(|i| expand(i, env)) else { return default_owner("у сервиса нет image") };
        if let Err(e) = self.docker.ensure_image(&image).await { return default_owner(&format!("образ {image} не скачан: {e}")); }
        let passwd = match self.docker.image_tar(&image, "/etc/passwd").await { Ok(Some(t)) => tar_first_file(&t).unwrap_or_default(), _ => String::new() };
        if let Some(u) = &user {
            if let Some((uid, gid)) = resolve_name(u, &passwd) { return Owner { uid, gid, why: format!("user: {u} в compose = {uid}:{gid} по /etc/passwd образа"), strong: true }; }
        }
        if let Ok(Some(u)) = self.docker.image_user(&image).await {
            let ids = parse_ids(&u).map(|(uid, gid)| (uid, gid.unwrap_or(uid))).or_else(|| resolve_name(&u, &passwd));
            if let Some((uid, gid)) = ids { if uid != 0 { return Owner { uid, gid, why: format!("USER {u} образа {image}"), strong: true }; } }
        }
        if let Ok(Some(t)) = self.docker.image_tar(&image, target).await {
            if let Some((uid, gid)) = tar_first_owner(&t) { if uid != 0 { return Owner { uid, gid, why: format!("владелец {target} в образе {image} (как named volume в Docker)"), strong: true }; } }
        }
        default_owner("образ работает от root, владелец безразличен")
    }

    /// Владельцы по монтированиям (один разбор compose и образов на сервис).
    async fn mount_owners(&self, compose_text: &str, env: &[(String, String)]) -> Vec<(Mount, Owner)> {
        let compose: Value = serde_yaml_ng::from_str(compose_text).unwrap_or(Value::Null);
        let envm: HashMap<String, String> = env.iter().cloned().collect();
        let mut cache: HashMap<String, Owner> = HashMap::new();
        let mut out = Vec::new();
        for m in data_mounts(&compose) {
            let o = match cache.get(&m.service) {
                Some(o) => o.clone(),
                None => {
                    let svc = compose.pointer(&format!("/services/{}", m.service)).cloned().unwrap_or(Value::Null);
                    let o = self.resolve_owner(&svc, &m.target, &envm).await;
                    cache.insert(m.service.clone(), o.clone());
                    o
                }
            };
            out.push((m, o));
        }
        out
    }

    /// Создать каталоги данных до `compose up` и выставить владельца новым и ещё пустым (без файлов) каталогам. Возвращает список изменений.
    pub async fn prepare_data_dirs(&self, name: &str, compose_text: &str, env: &[(String, String)]) -> Result<Vec<String>> {
        let root = self.cfg.apps_data_dir.join(name);
        ensure_data_dir(&root)?;
        for d in data_subdirs(compose_text) { let _ = ensure_data_dir(&root.join(&d)); }
        let mut notes = Vec::new();
        for (m, o) in self.mount_owners(compose_text, env).await {
            let dir = root.join(&m.subdir);
            let _ = ensure_data_dir(&dir);
            if tree_has_no_files(&dir) && owner_of(&dir) != Some((o.uid, o.gid)) {
                if let Err(e) = chown_recursive(&dir, o.uid, o.gid) { notes.push(format!("{}: не удалось выставить владельца {}:{}: {e}", display(&m.subdir), o.uid, o.gid)); continue; }
                notes.push(format!("{} → {}:{} ({})", display(&m.subdir), o.uid, o.gid, o.why));
            }
        }
        Ok(notes)
    }

    /// Починка прав уже наполненных каталогов по тем же правилам: рекурсивно, только при расхождении; владелец по умолчанию применяется,
    /// лишь когда каталог принадлежит root (наследие ядра, а не приложения). При изменениях — перевыкатка.
    pub async fn app_repair(&self, name: &str) -> Result<Vec<String>> {
        let _g = self.lock.lock().await;
        let m = self.manifest(name)?;
        let Some(st) = self.store.get_app(name)? else { bail!("приложение {name} не установлено") };
        let env = self.app_env(&m, st.port)?;
        let compose_text = self.compose_text(&m)?;
        let root = self.cfg.apps_data_dir.join(name);
        let mut notes = Vec::new();
        for (mnt, o) in self.mount_owners(&compose_text, &env).await {
            let dir = root.join(&mnt.subdir);
            let Some(cur) = owner_of(&dir) else { continue };
            if cur == (o.uid, o.gid) { continue; }
            if !o.strong && cur != (0, 0) { continue; }
            match chown_recursive(&dir, o.uid, o.gid) {
                Ok(()) => notes.push(format!("{}: {}:{} → {}:{} ({})", display(&mnt.subdir), cur.0, cur.1, o.uid, o.gid, o.why)),
                Err(e) => notes.push(format!("{}: не удалось сменить владельца: {e}", display(&mnt.subdir))),
            }
        }
        if !notes.is_empty() {
            self.store.event("app", name, &format!("права на каталоги данных исправлены: {}", notes.join("; ")))?;
            drop(_g);
            self.redeploy(name).await?;
        }
        Ok(notes)
    }

    /// После выкатки: до 5 минут ждём контейнеры, затем 90 с смотрим за ними. Если контейнер перезапускается и в логах ошибка прав —
    /// один раз чиним владельцев и перевыкатываем; иначе оставляем событие с последней строкой логов. Пользователь работает из браузера,
    /// и это единственный способ узнать, почему «502».
    pub fn spawn_watch(&self, name: &str) {
        let me = self.clone();
        let name = name.to_string();
        tokio::spawn(async move {
            let mut seen_at: Option<std::time::Instant> = None;
            let start = std::time::Instant::now();
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if start.elapsed() > std::time::Duration::from_secs(300) { return; }
                let Ok(cs) = me.docker.stack_containers(&name).await else { continue };
                if cs.is_empty() { continue; }
                let seen = *seen_at.get_or_insert_with(std::time::Instant::now);
                if seen.elapsed() > std::time::Duration::from_secs(90) { return; }
                let bad: Vec<&(String, String, String)> = cs.iter().filter(|(_, _, st)| matches!(st.as_str(), "restarting" | "dead")).collect();
                if bad.is_empty() { continue; }
                let services: Vec<String> = bad.iter().map(|(cn, _, _)| service_of(&name, cn)).collect();
                let logs = me.komodo.logs(&name, &services, 150).await.unwrap_or_default();
                if perm_error(&logs) {
                    match me.app_repair(&name).await {
                        Ok(fixed) if !fixed.is_empty() => { let _ = me.store.event("app", &name, &format!("контейнер {} падал с ошибкой прав — владелец каталогов данных исправлен автоматически, стек перевыкачен", services.join(", "))); }
                        Ok(_) => { let _ = me.store.event("app", &name, &format!("контейнер {} падает с ошибкой прав, но владельцы каталогов данных уже верные — смотрите логи", services.join(", "))); }
                        Err(e) => { let _ = me.store.event("app", &name, &format!("контейнер {} падает с ошибкой прав; починить не удалось: {e}", services.join(", "))); }
                    }
                } else {
                    let line: String = logs.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(200).collect();
                    let _ = me.store.event("app", &name, &format!("контейнер {} перезапускается; последняя строка логов: {line}", services.join(", ")));
                }
                return;
            }
        });
    }
}

fn display(subdir: &str) -> String { if subdir.is_empty() { "${APP_DATA_DIR}".into() } else { format!("${{APP_DATA_DIR}}/{subdir}") } }
