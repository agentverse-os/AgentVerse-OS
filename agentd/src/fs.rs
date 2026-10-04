//! Файлы и git workspace'а для рендерингов карточки сессии (ADR-006: Chat | Shell | Files | Git): список каталога, чтение и
//! запись файла (загрузка с телефона), папка/переименование/удаление, статус, диф и лог git. Всё от пользователя coder
//! внутри контейнера — границы задаёт сам контейнер, здесь только запрет «..» и лимиты размера.
use crate::ApiError;
use anyhow::anyhow;
use axum::body::Bytes;
use axum::extract::Query;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;

/// Предел одного файла на чтение и запись.
pub const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_DIFF: usize = 1024 * 1024;

pub fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/home/coder"))
}

/// Абсолютный путь, `~/…` или путь относительно home; «..» не принимаем — путь должен быть явным.
pub fn resolve(p: &str) -> Result<PathBuf, ApiError> {
    let p = p.trim();
    let raw = if p.is_empty() || p == "~" { home() } else if let Some(rest) = p.strip_prefix("~/") { home().join(rest) } else if p.starts_with('/') { PathBuf::from(p) } else { home().join(p) };
    if raw.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(ApiError::BadRequest("путь с «..» не принимается".into()));
    }
    Ok(raw)
}

fn io_err(p: &Path, e: std::io::Error) -> ApiError {
    match e.kind() {
        std::io::ErrorKind::NotFound => ApiError::NotFound,
        std::io::ErrorKind::PermissionDenied => ApiError::BadRequest(format!("нет прав: {}", p.display())),
        _ => ApiError::Internal(anyhow!("{}: {e}", p.display())),
    }
}

/// Флаг из query: `1`, `true`, `yes`, `on` (serde из коробки принимает только `true`/`false`).
fn flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let s = String::deserialize(d)?;
    Ok(matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

#[derive(Deserialize)]
pub struct PathQ {
    #[serde(default)]
    pub path: String,
    #[serde(default, deserialize_with = "flag")]
    pub recursive: bool,
    #[serde(default, deserialize_with = "flag")]
    pub download: bool,
}

#[derive(Deserialize)]
pub struct FindQ {
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub q: String,
    #[serde(default = "find_limit")]
    pub limit: usize,
}
fn find_limit() -> usize { 20 }

/// Подсказки файлов для «@» в композере (у Codex; у Claude — `file_suggestions` самого CLI): обход каталога на 5 уровней,
/// без `.git`, `node_modules`, `target` и прочих тяжёлых каталогов; ранг — префикс имени > подстрока пути > подпоследовательность.
pub async fn find(Query(q): Query<FindQ>) -> Result<Json<Value>, ApiError> {
    let root = resolve(&q.cwd)?;
    let needle = q.q.trim().to_lowercase();
    let mut out: Vec<(u8, usize, String, bool)> = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > 20000 { break }
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let rel = e.path().strip_prefix(&root).map(|p| p.to_string_lossy().into_owned()).unwrap_or(name.clone());
            if is_dir {
                if depth < 5 && !matches!(name.as_str(), ".git" | "node_modules" | "target" | ".cache" | ".npm" | ".npm-global" | "dist" | "build" | ".venv" | "venv" | "__pycache__" | ".next" | ".claude" | ".codex") { stack.push((e.path(), depth + 1)); }
            }
            let lower = rel.to_lowercase();
            let rank = if needle.is_empty() { 3 } else if name.to_lowercase().starts_with(&needle) { 0 } else if lower.contains(&needle) { 1 } else if subseq(&lower, &needle) { 2 } else { continue };
            out.push((rank, rel.len(), rel, is_dir));
        }
    }
    out.sort();
    let items: Vec<Value> = out.into_iter().take(q.limit.clamp(1, 200)).map(|(_, _, rel, d)| json!({ "path": if d { format!("{rel}/") } else { rel } })).collect();
    Ok(Json(json!({ "suggestions": items, "cwd": root.to_string_lossy() })))
}

fn subseq(hay: &str, needle: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub kind: &'static str,
    pub link: bool,
    pub size: u64,
    pub mtime: u64,
    pub mode: u32,
}

pub async fn list(Query(q): Query<PathQ>) -> Result<Json<Value>, ApiError> {
    let dir = resolve(&q.path)?;
    let mut rd = tokio::fs::read_dir(&dir).await.map_err(|e| io_err(&dir, e))?;
    let mut entries = Vec::new();
    while let Some(e) = rd.next_entry().await.map_err(|er| io_err(&dir, er))? {
        let name = e.file_name().to_string_lossy().to_string();
        let link = e.file_type().await.map(|f| f.is_symlink()).unwrap_or(false);
        // метаданные цели (за симлинком); битая ссылка — обычный файл нулевого размера
        let md = tokio::fs::metadata(e.path()).await.ok();
        let kind = if md.as_ref().map(|m| m.is_dir()).unwrap_or(false) { "dir" } else { "file" };
        let (size, mtime, mode) = md.map(|m| (m.len(), m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0), m.permissions().mode() & 0o7777)).unwrap_or((0, 0, 0));
        entries.push(Entry { name, kind, link, size: if kind == "dir" { 0 } else { size }, mtime, mode });
    }
    entries.sort_by(|a, b| (a.kind != "dir").cmp(&(b.kind != "dir")).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(Json(json!({ "path": dir.to_string_lossy(), "home": home().to_string_lossy(), "entries": entries })))
}

/// Тип содержимого по расширению, иначе по первым байтам (текст без NUL — text/plain).
pub fn content_type(p: &Path, data: &[u8]) -> &'static str {
    let ext = p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif", "webp" => "image/webp", "svg" => "image/svg+xml", "bmp" => "image/bmp", "avif" => "image/avif",
        "pdf" => "application/pdf", "json" => "application/json; charset=utf-8", "html" | "htm" => "text/html; charset=utf-8", "css" => "text/css; charset=utf-8",
        "md" | "txt" | "rs" | "py" | "js" | "ts" | "mjs" | "cjs" | "svelte" | "toml" | "yaml" | "yml" | "sh" | "bash" | "env" | "ini" | "cfg" | "conf" | "log" | "csv" | "tsv" | "xml" | "sql" | "go" | "java" | "kt" | "c" | "h" | "cpp" | "hpp" | "rb" | "php" | "lock" | "gitignore" | "dockerfile" | "makefile" => "text/plain; charset=utf-8",
        _ => if data.iter().take(8192).any(|&b| b == 0) { "application/octet-stream" } else { "text/plain; charset=utf-8" },
    }
}

pub async fn read(Query(q): Query<PathQ>) -> Result<Response, ApiError> {
    let p = resolve(&q.path)?;
    let md = tokio::fs::metadata(&p).await.map_err(|e| io_err(&p, e))?;
    if md.is_dir() { return Err(ApiError::BadRequest("это каталог".into())); }
    if md.len() as usize > MAX_FILE { return Err(ApiError::BadRequest("файл больше 64 МБ".into())); }
    let data = tokio::fs::read(&p).await.map_err(|e| io_err(&p, e))?;
    let ct = content_type(&p, &data);
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("file").replace('"', "");
    let disp = format!("{}; filename=\"{name}\"", if q.download { "attachment" } else { "inline" });
    Ok(([(header::CONTENT_TYPE, ct.to_string()), (header::CONTENT_DISPOSITION, disp), (header::CACHE_CONTROL, "no-store".into())], Bytes::from(data)).into_response())
}

pub async fn write(Query(q): Query<PathQ>, body: Bytes) -> Result<Json<Value>, ApiError> {
    let p = resolve(&q.path)?;
    if p.is_dir() { return Err(ApiError::BadRequest("это каталог".into())); }
    if body.len() > MAX_FILE { return Err(ApiError::BadRequest("файл больше 64 МБ".into())); }
    tokio::fs::write(&p, &body).await.map_err(|e| io_err(&p, e))?;
    Ok(Json(json!({ "path": p.to_string_lossy(), "size": body.len() })))
}

#[derive(Deserialize)]
pub struct MkdirBody { pub path: String }

pub async fn mkdir(Json(b): Json<MkdirBody>) -> Result<Json<Value>, ApiError> {
    let p = resolve(&b.path)?;
    tokio::fs::create_dir_all(&p).await.map_err(|e| io_err(&p, e))?;
    Ok(Json(json!({ "path": p.to_string_lossy() })))
}

#[derive(Deserialize)]
pub struct RenameBody { pub from: String, pub to: String }

pub async fn rename(Json(b): Json<RenameBody>) -> Result<Json<Value>, ApiError> {
    let (from, to) = (resolve(&b.from)?, resolve(&b.to)?);
    if to.exists() { return Err(ApiError::BadRequest(format!("{} уже существует", to.display()))); }
    tokio::fs::rename(&from, &to).await.map_err(|e| io_err(&from, e))?;
    Ok(Json(json!({ "path": to.to_string_lossy() })))
}

pub async fn remove(Query(q): Query<PathQ>) -> Result<StatusCode, ApiError> {
    let p = resolve(&q.path)?;
    if p == home() || p == Path::new("/") { return Err(ApiError::BadRequest("этот каталог не удаляем".into())); }
    let md = tokio::fs::symlink_metadata(&p).await.map_err(|e| io_err(&p, e))?;
    if md.is_dir() {
        if q.recursive { tokio::fs::remove_dir_all(&p).await.map_err(|e| io_err(&p, e))?; }
        else { tokio::fs::remove_dir(&p).await.map_err(|e| if e.kind() == std::io::ErrorKind::DirectoryNotEmpty || e.raw_os_error() == Some(39) { ApiError::BadRequest("каталог не пуст — удалите с recursive=1".into()) } else { io_err(&p, e) })?; }
    } else {
        tokio::fs::remove_file(&p).await.map_err(|e| io_err(&p, e))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---- git ----

#[derive(Deserialize)]
pub struct GitQ {
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default, deserialize_with = "flag")]
    pub staged: bool,
    #[serde(default, deserialize_with = "flag")]
    pub untracked: bool,
    #[serde(default = "default_n")]
    pub n: usize,
}
fn default_n() -> usize { 30 }

async fn git(cwd: &Path, args: &[&str]) -> Result<(bool, String, String), ApiError> {
    let out = tokio::process::Command::new("git").arg("-C").arg(cwd).args(args).output().await.map_err(|e| ApiError::Internal(anyhow!("git: {e}")))?;
    Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string()))
}

#[derive(Serialize, Debug, PartialEq)]
pub struct GitEntry { pub path: String, pub x: String, pub y: String, pub staged: bool, pub unstaged: bool, pub kind: &'static str }

/// Разбор `git status --porcelain=v2 --branch`.
pub fn parse_status(out: &str) -> Value {
    let (mut head, mut upstream, mut ahead, mut behind) = (String::new(), None::<String>, 0i64, 0i64);
    let mut entries = Vec::new();
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            if let Some(v) = rest.strip_prefix("branch.head ") { head = v.to_string(); }
            else if let Some(v) = rest.strip_prefix("branch.upstream ") { upstream = Some(v.to_string()); }
            else if let Some(v) = rest.strip_prefix("branch.ab ") { let mut it = v.split(' '); ahead = it.next().and_then(|a| a.trim_start_matches('+').parse().ok()).unwrap_or(0); behind = it.next().and_then(|b| b.trim_start_matches('-').parse().ok()).unwrap_or(0); }
            continue;
        }
        let f: Vec<&str> = line.splitn(if line.starts_with('2') { 10 } else if line.starts_with('u') { 11 } else { 9 }, ' ').collect();
        let (x, y) = f.get(1).map(|xy| { let mut c = xy.chars(); (c.next().unwrap_or('.').to_string(), c.next().unwrap_or('.').to_string()) }).unwrap_or((".".into(), ".".into()));
        let mk = |path: &str, kind: &'static str, x: String, y: String| GitEntry { path: path.to_string(), staged: x != "." && kind != "untracked", unstaged: y != "." || kind == "untracked", x, y, kind };
        match line.chars().next() {
            Some('1') if f.len() >= 9 => entries.push(mk(f[8], "changed", x, y)),
            Some('2') if f.len() >= 10 => entries.push(mk(f[9].split('\t').next().unwrap_or(f[9]), "renamed", x, y)),
            Some('u') if f.len() >= 11 => entries.push(mk(f[10], "unmerged", x, y)),
            Some('?') => entries.push(mk(line[2..].trim(), "untracked", ".".into(), "?".into())),
            Some('!') => entries.push(mk(line[2..].trim(), "ignored", ".".into(), "!".into())),
            _ => {}
        }
    }
    json!({ "repo": true, "head": head, "upstream": upstream, "ahead": ahead, "behind": behind, "entries": entries })
}

pub async fn git_status(Query(q): Query<GitQ>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&q.cwd)?;
    if !cwd.is_dir() { return Err(ApiError::NotFound); }
    let (ok, out, err) = git(&cwd, &["status", "--porcelain=v2", "--branch"]).await?;
    if !ok { return Ok(Json(json!({ "repo": false, "cwd": cwd.to_string_lossy(), "reason": err.lines().next().unwrap_or("") }))); }
    let mut v = parse_status(&out);
    v["cwd"] = json!(cwd.to_string_lossy());
    Ok(Json(v))
}

pub async fn git_diff(Query(q): Query<GitQ>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&q.cwd)?;
    let mut args: Vec<&str> = vec!["diff", "--no-color"];
    if q.untracked {
        let Some(p) = q.path.as_deref() else { return Err(ApiError::BadRequest("нужен path".into())) };
        args.extend(["--no-index", "--", "/dev/null", p]);
    } else {
        if q.staged { args.push("--cached"); }
        if let Some(p) = q.path.as_deref() { args.push("--"); args.push(p); }
    }
    let (_, out, _) = git(&cwd, &args).await?;
    let truncated = out.len() > MAX_DIFF;
    let text = if truncated { out.chars().take(MAX_DIFF).collect() } else { out };
    Ok(Json(json!({ "diff": text, "truncated": truncated })))
}

pub async fn git_log(Query(q): Query<GitQ>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&q.cwd)?;
    let n = q.n.clamp(1, 200).to_string();
    let (ok, out, _) = git(&cwd, &["log", "--format=%H%x1f%h%x1f%an%x1f%at%x1f%s", "-n", &n]).await?;
    if !ok { return Ok(Json(json!({ "entries": [] }))); }
    let entries: Vec<Value> = out.lines().filter_map(|l| {
        let f: Vec<&str> = l.split('\u{1f}').collect();
        (f.len() >= 5).then(|| json!({ "hash": f[0], "short": f[1], "author": f[2], "ts": f[3].parse::<u64>().unwrap_or(0), "subject": f[4] }))
    }).collect();
    Ok(Json(json!({ "entries": entries })))
}

// ---- git: индекс, коммит, ветки, remote, worktrees (docs/agents-ux.md §4.3.6) ----
#[derive(Deserialize, Default)]
pub struct GitBody {
    #[serde(default)] pub cwd: String,
    #[serde(default)] pub paths: Vec<String>,
    #[serde(default)] pub all: bool,
    #[serde(default)] pub message: Option<String>,
    #[serde(default)] pub branch: Option<String>,
    #[serde(default)] pub base: Option<String>,
    #[serde(default)] pub create: bool,
    #[serde(default)] pub name: Option<String>,
    #[serde(default)] pub url: Option<String>,
    #[serde(default)] pub remote: Option<String>,
    #[serde(default)] pub force: bool,
    #[serde(default)] pub delete_branch: bool,
    #[serde(default)] pub squash: Option<bool>,
    #[serde(default)] pub rebase: bool,
    #[serde(default)] pub bare: bool,
}

/// git с проверкой кода выхода: stdout на успех, иначе 400 с последними строками stderr.
async fn git_ok(cwd: &Path, args: &[&str]) -> Result<String, ApiError> {
    let (ok, out, err) = git(cwd, args).await?;
    if ok { Ok(out) } else {
        let tail: Vec<&str> = err.trim().lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect();
        Err(ApiError::BadRequest(format!("git {}: {}", args.first().copied().unwrap_or(""), tail.join(" · ").chars().take(400).collect::<String>())))
    }
}

/// Коммит; без user.name/email в workspace — от имени AgentVerse.
async fn git_commit_in(dir: &Path, msg: &str) -> Result<Value, ApiError> {
    let (ok, _, err) = git(dir, &["commit", "-q", "-m", msg]).await?;
    if !ok {
        if err.contains("Please tell me who you are") || err.contains("Author identity unknown") || err.contains("empty ident") {
            git_ok(dir, &["-c", "user.name=AgentVerse", "-c", "user.email=agent@cloudos.local", "commit", "-q", "-m", msg]).await?;
        } else {
            return Err(ApiError::BadRequest(format!("git commit: {}", err.trim().lines().last().unwrap_or("").chars().take(300).collect::<String>())));
        }
    }
    let (_, out, _) = git(dir, &["log", "-1", "--format=%H%x1f%h%x1f%s"]).await?;
    let f: Vec<&str> = out.trim().split('\u{1f}').collect();
    Ok(json!({ "hash": f.first().copied().unwrap_or(""), "short": f.get(1).copied().unwrap_or(""), "subject": f.get(2).copied().unwrap_or("") }))
}

pub async fn git_init(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    tokio::fs::create_dir_all(&cwd).await.map_err(|e| io_err(&cwd, e))?;
    let mut args = vec!["init", "-q", "-b", "main"];
    if b.bare { args.push("--bare"); }
    git_ok(&cwd, &args).await?;
    Ok(Json(json!({ "cwd": cwd.to_string_lossy(), "bare": b.bare })))
}

pub async fn git_remote(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    let name = b.name.as_deref().unwrap_or("origin");
    let url = b.url.as_deref().map(str::trim).filter(|u| !u.is_empty()).ok_or_else(|| ApiError::BadRequest("нужен url".into()))?;
    if git(&cwd, &["remote", "get-url", name]).await?.0 { git_ok(&cwd, &["remote", "set-url", name, url]).await?; } else { git_ok(&cwd, &["remote", "add", name, url]).await?; }
    Ok(Json(json!({ "name": name, "url": url })))
}

pub async fn git_stage(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    if b.all || b.paths.is_empty() { git_ok(&cwd, &["add", "-A"]).await?; }
    else { let mut a = vec!["add", "-A", "--"]; a.extend(b.paths.iter().map(String::as_str)); git_ok(&cwd, &a).await?; }
    Ok(Json(json!({ "ok": true })))
}

pub async fn git_unstage(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    // до первого коммита HEAD нет: из индекса — rm --cached
    let has_head = git(&cwd, &["rev-parse", "--verify", "-q", "HEAD"]).await?.0;
    let mut a: Vec<&str> = if has_head { vec!["restore", "--staged", "--"] } else { vec!["rm", "--cached", "-r", "-q", "--"] };
    if b.all || b.paths.is_empty() { a.push("."); } else { a.extend(b.paths.iter().map(String::as_str)); }
    git_ok(&cwd, &a).await?;
    Ok(Json(json!({ "ok": true })))
}

/// Откат изменений файлов: отслеживаемые — к HEAD (индекс и рабочее дерево), новые — убрать из индекса и удалить.
pub async fn git_discard(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    if b.paths.is_empty() { return Err(ApiError::BadRequest("нужны paths".into())); }
    for p in &b.paths {
        if git(&cwd, &["cat-file", "-e", &format!("HEAD:{p}")]).await?.0 { git_ok(&cwd, &["restore", "--staged", "--worktree", "--", p]).await?; }
        else { let _ = git(&cwd, &["rm", "--cached", "-r", "-q", "--", p]).await; git_ok(&cwd, &["clean", "-fdq", "--", p]).await?; }
    }
    Ok(Json(json!({ "ok": true, "paths": b.paths })))
}

pub async fn git_commit(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    let msg = b.message.as_deref().map(str::trim).filter(|m| !m.is_empty()).ok_or_else(|| ApiError::BadRequest("пустое сообщение коммита".into()))?;
    if b.all { git_ok(&cwd, &["add", "-A"]).await?; }
    let (_, staged, _) = git(&cwd, &["diff", "--cached", "--name-only"]).await?;
    if staged.trim().is_empty() { return Err(ApiError::BadRequest("индекс пуст — нечего коммитить".into())); }
    Ok(Json(git_commit_in(&cwd, msg).await?))
}

pub async fn git_branches(Query(q): Query<GitQ>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&q.cwd)?;
    let fmt = "--format=%(refname:short)\u{1f}%(upstream:short)\u{1f}%(HEAD)\u{1f}%(objectname:short)\u{1f}%(committerdate:unix)\u{1f}%(subject)";
    let out = git_ok(&cwd, &["branch", "--list", fmt]).await?;
    let mut head = String::new();
    let branches: Vec<Value> = out.lines().filter_map(|l| {
        let f: Vec<&str> = l.split('\u{1f}').collect();
        if f.len() < 6 { return None; }
        let current = f[2] == "*";
        if current { head = f[0].to_string(); }
        Some(json!({ "name": f[0], "upstream": (!f[1].is_empty()).then(|| f[1]), "current": current, "short": f[3], "ts": f[4].parse::<u64>().unwrap_or(0), "subject": f[5] }))
    }).collect();
    let (_, remotes, _) = git(&cwd, &["remote"]).await?;
    Ok(Json(json!({ "head": head, "branches": branches, "remotes": remotes.lines().map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>() })))
}

pub async fn git_checkout(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    let br = b.branch.as_deref().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| ApiError::BadRequest("нужна ветка".into()))?;
    if b.create { let mut a = vec!["checkout", "-q", "-b", br]; if let Some(base) = b.base.as_deref().filter(|s| !s.trim().is_empty()) { a.push(base); } git_ok(&cwd, &a).await?; }
    else { git_ok(&cwd, &["checkout", "-q", br]).await?; }
    Ok(Json(json!({ "branch": br })))
}

pub async fn git_fetch(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    let (ok, out, err) = git(&cwd, &["fetch", "--prune", "--all"]).await?;
    if !ok { return Err(ApiError::BadRequest(format!("git fetch: {}", err.trim().lines().last().unwrap_or("")))); }
    Ok(Json(json!({ "ok": true, "out": format!("{out}{err}").trim() })))
}

pub async fn git_pull(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    let a: [&str; 2] = if b.rebase { ["pull", "--rebase"] } else { ["pull", "--ff-only"] };
    let (ok, out, err) = git(&cwd, &a).await?;
    if !ok { return Err(ApiError::BadRequest(format!("git pull: {}", err.trim().lines().last().unwrap_or("")))); }
    Ok(Json(json!({ "ok": true, "out": format!("{out}{err}").trim() })))
}

pub async fn git_push(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    let has_upstream = git(&cwd, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).await?.0;
    let remote = b.remote.clone().filter(|r| !r.trim().is_empty()).unwrap_or_else(|| "origin".into());
    let (ok, out, err) = if has_upstream { git(&cwd, &["push", "--porcelain"]).await? } else { git(&cwd, &["push", "--porcelain", "-u", &remote, "HEAD"]).await? };
    if !ok { return Err(ApiError::BadRequest(format!("git push: {}", err.trim().lines().last().unwrap_or("").chars().take(300).collect::<String>()))); }
    Ok(Json(json!({ "ok": true, "set_upstream": !has_upstream, "out": format!("{out}{err}").trim() })))
}

/// Разбор `git worktree list --porcelain`: path, name, head, branch, bare/detached/locked.
pub fn parse_worktrees(out: &str) -> Vec<Value> {
    let mut list = Vec::new(); let mut cur: Option<Value> = None;
    for line in out.lines().chain(std::iter::once("")) {
        if line.is_empty() { if let Some(c) = cur.take() { list.push(c); } continue; }
        if let Some(p) = line.strip_prefix("worktree ") {
            let name = Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            cur = Some(json!({ "path": p, "name": name, "head": null, "branch": null, "bare": false, "detached": false, "locked": false }));
        } else if let Some(c) = cur.as_mut() {
            if let Some(h) = line.strip_prefix("HEAD ") { c["head"] = json!(h.chars().take(7).collect::<String>()); }
            else if let Some(br) = line.strip_prefix("branch ") { c["branch"] = json!(br.strip_prefix("refs/heads/").unwrap_or(br)); }
            else if line == "bare" { c["bare"] = json!(true); }
            else if line == "detached" { c["detached"] = json!(true); }
            else if line.starts_with("locked") { c["locked"] = json!(true); }
        }
    }
    for (i, w) in list.iter_mut().enumerate() { w["main"] = json!(i == 0); }
    list
}

async fn worktree_list(cwd: &Path) -> Result<Vec<Value>, ApiError> { Ok(parse_worktrees(&git_ok(cwd, &["worktree", "list", "--porcelain"]).await?)) }

fn find_worktree<'a>(list: &'a [Value], name: &str) -> Option<&'a Value> {
    list.iter().find(|w| w.get("name").and_then(|n| n.as_str()) == Some(name) || w.get("path").and_then(|p| p.as_str()) == Some(name))
}

pub async fn git_worktrees(Query(q): Query<GitQ>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&q.cwd)?;
    let mut list = worktree_list(&cwd).await?;
    // текущий — самый длинный путь, который содержит cwd; изменения — по каждому каталогу
    let cur = list.iter().enumerate().filter(|(_, w)| w["path"].as_str().map(|p| cwd.starts_with(p)).unwrap_or(false)).max_by_key(|(_, w)| w["path"].as_str().map(str::len).unwrap_or(0)).map(|(i, _)| i);
    for (i, w) in list.iter_mut().enumerate() {
        w["current"] = json!(Some(i) == cur);
        let p = PathBuf::from(w["path"].as_str().unwrap_or(""));
        let n = if w["bare"].as_bool() == Some(true) || !p.is_dir() { 0 } else { git(&p, &["status", "--porcelain"]).await.map(|(_, o, _)| o.lines().count()).unwrap_or(0) };
        w["changes"] = json!(n);
    }
    Ok(Json(json!({ "cwd": cwd.to_string_lossy(), "worktrees": list })))
}

pub async fn git_worktree_add(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 64 && !n.starts_with('.') && n.chars().all(|c| c.is_alphanumeric() || "-_.".contains(c)))
        .ok_or_else(|| ApiError::BadRequest("имя worktree: буквы, цифры, «-», «_», «.»".into()))?;
    let list = worktree_list(&cwd).await?;
    let main = list.first().and_then(|w| w["path"].as_str()).map(PathBuf::from).ok_or_else(|| ApiError::BadRequest("нет основного worktree".into()))?;
    let dir = main.join(".worktrees");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| io_err(&dir, e))?;
    let path = dir.join(name);
    if path.exists() { return Err(ApiError::BadRequest(format!("каталог уже есть: {}", path.display()))); }
    // .worktrees/ — в локальный exclude репозитория (без правок в самом репозитории)
    if let Ok(common) = git_ok(&cwd, &["rev-parse", "--git-common-dir"]).await {
        let common = common.trim(); let common = if Path::new(common).is_absolute() { PathBuf::from(common) } else { main.join(common) };
        let ex = common.join("info").join("exclude");
        let cur = tokio::fs::read_to_string(&ex).await.unwrap_or_default();
        if !cur.lines().any(|l| matches!(l.trim(), ".worktrees/" | ".worktrees" | "/.worktrees/" | "/.worktrees")) {
            let _ = tokio::fs::create_dir_all(common.join("info")).await;
            let _ = tokio::fs::write(&ex, format!("{cur}{}.worktrees/\n", if cur.is_empty() || cur.ends_with('\n') { "" } else { "\n" })).await;
        }
    }
    let branch = b.branch.clone().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| name.to_string());
    let exists = git(&cwd, &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")]).await?.0;
    let ps = path.to_string_lossy().into_owned();
    if exists { git_ok(&cwd, &["worktree", "add", &ps, &branch]).await?; }
    else { let mut a = vec!["worktree", "add", "-b", &branch, &ps]; if let Some(base) = b.base.as_deref().filter(|s| !s.trim().is_empty()) { a.push(base); } git_ok(&cwd, &a).await?; }
    Ok(Json(json!({ "path": ps, "name": name, "branch": branch })))
}

pub async fn git_worktree_remove(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| ApiError::BadRequest("нужно имя worktree".into()))?;
    let list = worktree_list(&cwd).await?;
    let w = find_worktree(&list, name).ok_or(ApiError::NotFound)?;
    if w["main"].as_bool() == Some(true) { return Err(ApiError::BadRequest("основной worktree удалить нельзя".into())); }
    let path = w["path"].as_str().unwrap_or("").to_string(); let branch = w["branch"].as_str().map(str::to_string);
    let mut a = vec!["worktree", "remove"]; if b.force { a.push("--force"); } a.push(&path);
    git_ok(&cwd, &a).await?;
    if b.delete_branch { if let Some(br) = &branch { let _ = git(&cwd, &["branch", "-D", br]).await; } }
    Ok(Json(json!({ "removed": path, "branch": branch, "branch_deleted": b.delete_branch })))
}

/// Слияние ветки worktree в основной worktree (squash по умолчанию); основной должен быть чистым, конфликт — откат и ошибка.
pub async fn git_worktree_merge(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| ApiError::BadRequest("нужно имя worktree".into()))?;
    let list = worktree_list(&cwd).await?;
    let w = find_worktree(&list, name).ok_or(ApiError::NotFound)?;
    if w["main"].as_bool() == Some(true) { return Err(ApiError::BadRequest("это основной worktree".into())); }
    let path = PathBuf::from(w["path"].as_str().unwrap_or(""));
    let br = w["branch"].as_str().map(str::to_string).ok_or_else(|| ApiError::BadRequest("у worktree нет ветки (detached HEAD)".into()))?;
    let main = PathBuf::from(list[0]["path"].as_str().unwrap_or(""));
    let (_, st, _) = git(&main, &["status", "--porcelain"]).await?;
    if !st.trim().is_empty() { return Err(ApiError::BadRequest("в основном worktree есть незакоммиченные изменения — сначала закоммитьте или откатите их".into())); }
    let (_, wst, _) = git(&path, &["status", "--porcelain"]).await?;
    if !wst.trim().is_empty() && !b.force { return Err(ApiError::BadRequest("в worktree есть незакоммиченные изменения — они не попадут в слияние; закоммитьте их или подтвердите слияние без них".into())); }
    let squash = b.squash.unwrap_or(true);
    let msg = b.message.clone().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| format!("Merge worktree {name} ({br})"));
    let commit = if squash {
        if let Err(e) = git_ok(&main, &["merge", "--squash", "-q", &br]).await { let _ = git(&main, &["merge", "--abort"]).await; let _ = git(&main, &["reset", "-q", "--hard"]).await; return Err(e); }
        let (_, staged, _) = git(&main, &["diff", "--cached", "--name-only"]).await?;
        if staged.trim().is_empty() { return Err(ApiError::BadRequest("нечего сливать: ветка уже в основной".into())); }
        git_commit_in(&main, &msg).await?
    } else {
        if let Err(e) = git_ok(&main, &["merge", "--no-ff", "-q", "-m", &msg, &br]).await { let _ = git(&main, &["merge", "--abort"]).await; return Err(e); }
        let (_, out, _) = git(&main, &["log", "-1", "--format=%H%x1f%h%x1f%s"]).await?;
        let f: Vec<&str> = out.trim().split('\u{1f}').collect();
        json!({ "hash": f.first().copied().unwrap_or(""), "short": f.get(1).copied().unwrap_or(""), "subject": f.get(2).copied().unwrap_or("") })
    };
    let mut removed = false;
    if b.delete_branch {
        let ps = path.to_string_lossy().into_owned();
        git_ok(&main, &["worktree", "remove", "--force", &ps]).await?;
        let _ = git(&main, &["branch", "-D", &br]).await;
        removed = true;
    }
    Ok(Json(json!({ "commit": commit, "branch": br, "squash": squash, "removed": removed })))
}

/// Сообщение коммита по дифу индекса (или рабочего дерева, если индекс пуст): headless Claude (haiku через gate) без сохранения сессии.
pub async fn git_suggest(Json(b): Json<GitBody>) -> Result<Json<Value>, ApiError> {
    let cwd = resolve(&b.cwd)?;
    let (_, mut diff, _) = git(&cwd, &["diff", "--cached", "--no-color", "--stat=100", "--patch"]).await?;
    let mut scope = "staged";
    if diff.trim().is_empty() {
        let (_, d2, _) = git(&cwd, &["diff", "--no-color", "--stat=100", "--patch"]).await?;
        let (_, un, _) = git(&cwd, &["ls-files", "--others", "--exclude-standard"]).await?;
        diff = if un.trim().is_empty() { d2 } else { format!("{d2}\n# untracked files:\n{un}") }; scope = "worktree";
    }
    if diff.trim().is_empty() { return Err(ApiError::BadRequest("нет изменений — нечего описывать".into())); }
    const MAX: usize = 60_000;
    let diff: String = if diff.len() > MAX { let mut s: String = diff.chars().take(MAX).collect(); s.push_str("\n… (diff truncated)"); s } else { diff };
    let prompt = format!("Write a git commit message for the diff below. First line: an imperative summary of at most 72 characters, no trailing period. If useful, add a blank line and up to 5 short bullet lines explaining what and why. Output only the message: no quotes, no markdown fences, no preamble, no trailers such as Co-Authored-By.\n\n{diff}");
    let mut child = tokio::process::Command::new("bash").arg("-lc").arg("exec claude -p --model claude-haiku-4-5-20251001 --output-format text --no-session-persistence --max-turns 1")
        .current_dir(&cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| ApiError::Internal(anyhow!("claude: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() { use tokio::io::AsyncWriteExt; let _ = stdin.write_all(prompt.as_bytes()).await; let _ = stdin.shutdown().await; }
    let out = match tokio::time::timeout(std::time::Duration::from_secs(120), child.wait_with_output()).await {
        Ok(r) => r.map_err(|e| ApiError::Internal(anyhow!("claude: {e}")))?,
        Err(_) => return Err(ApiError::BadRequest("claude не ответил за 120 с".into())),
    };
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || text.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr); let last = err.trim().lines().last().unwrap_or("пустой ответ");
        return Err(ApiError::BadRequest(format!("claude: {}", last.chars().take(300).collect::<String>())));
    }
    // трейлеры CLI (Co-Authored-By, «Generated with») — не наши: правило репозитория — без атрибуции в коммитах
    let message = text.trim().trim_start_matches("```").trim_end_matches("```").lines()
        .filter(|l| { let t = l.trim_start().to_ascii_lowercase(); !(t.starts_with("co-authored-by:") || t.starts_with("🤖 generated with") || t.starts_with("generated with [claude")) })
        .collect::<Vec<_>>().join("\n").trim().to_string();
    Ok(Json(json!({ "message": message, "scope": scope, "model": "claude-haiku-4-5-20251001" })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_paths() {
        std::env::set_var("HOME", "/home/coder");
        assert_eq!(resolve("").unwrap(), PathBuf::from("/home/coder"));
        assert_eq!(resolve("~/x").unwrap(), PathBuf::from("/home/coder/x"));
        assert_eq!(resolve("rel/a").unwrap(), PathBuf::from("/home/coder/rel/a"));
        assert_eq!(resolve("/tmp/a").unwrap(), PathBuf::from("/tmp/a"));
        assert!(resolve("/tmp/../etc").is_err());
    }

    #[test]
    fn worktrees_porcelain() {
        let out = "worktree /home/coder/repo\nHEAD abcdef1234567890\nbranch refs/heads/main\n\nworktree /home/coder/repo/.worktrees/fix\nHEAD 1234567abcdef\nbranch refs/heads/fix\nlocked\n\nworktree /home/coder/repo/.worktrees/x\nHEAD 999999999\ndetached\n";
        let l = parse_worktrees(out);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0]["name"], "repo"); assert_eq!(l[0]["main"], true); assert_eq!(l[0]["branch"], "main"); assert_eq!(l[0]["head"], "abcdef1");
        assert_eq!(l[1]["name"], "fix"); assert_eq!(l[1]["locked"], true); assert_eq!(l[1]["main"], false);
        assert_eq!(l[2]["detached"], true); assert!(l[2]["branch"].is_null());
    }

    #[test]
    fn query_flags() {
        let q: PathQ = serde_urlencoded::from_str("path=/x&download=1&recursive=true").unwrap();
        assert!(q.download && q.recursive);
        let q: PathQ = serde_urlencoded::from_str("path=/x&download=0").unwrap();
        assert!(!q.download && !q.recursive);
        let g: GitQ = serde_urlencoded::from_str("cwd=/x&staged=yes&untracked=false").unwrap();
        assert!(g.staged && !g.untracked && g.n == 30);
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type(Path::new("a.png"), b""), "image/png");
        assert_eq!(content_type(Path::new("a.rs"), b""), "text/plain; charset=utf-8");
        assert_eq!(content_type(Path::new("bin"), b"\x00\x01"), "application/octet-stream");
        assert_eq!(content_type(Path::new("noext"), b"hello"), "text/plain; charset=utf-8");
    }

    #[test]
    fn parses_porcelain_v2() {
        let out = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 h1 h2 a.txt\n1 A. N... 000000 100644 100644 h1 h2 src/new file.rs\n2 R. N... 100644 100644 100644 h1 h2 R100 new.txt\told.txt\n? b.txt\n";
        let v = parse_status(out);
        assert_eq!(v["head"], "main"); assert_eq!(v["upstream"], "origin/main"); assert_eq!(v["ahead"], 2); assert_eq!(v["behind"], 1);
        let e = v["entries"].as_array().unwrap();
        assert_eq!(e.len(), 4);
        assert_eq!(e[0]["path"], "a.txt"); assert_eq!(e[0]["staged"], false); assert_eq!(e[0]["unstaged"], true); assert_eq!(e[0]["y"], "M");
        assert_eq!(e[1]["path"], "src/new file.rs"); assert_eq!(e[1]["staged"], true); assert_eq!(e[1]["unstaged"], false);
        assert_eq!(e[2]["path"], "new.txt"); assert_eq!(e[2]["kind"], "renamed");
        assert_eq!(e[3]["path"], "b.txt"); assert_eq!(e[3]["kind"], "untracked"); assert_eq!(e[3]["unstaged"], true);
    }
}
