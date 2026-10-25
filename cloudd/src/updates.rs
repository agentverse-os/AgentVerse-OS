//! Updates (docs/updates.md). Three levels:
//! 1. the system — cloudd with the embedded Desktop and the hand-written Store manifests in one package
//! `agentverse-os-<version>-<arch>.tar.gz`
//! (manifest.json + cloudd + store/); the source is the update channel (a JSON manifest by URL) or a file uploaded through
//! Desktop; install: sha256 check, `cloudd --version` of the new binary, current → `.prev`, atomic replacement, restart via a
//! systemd transient unit with a watchdog: if the core did not answer within 45 s — rollback to `.prev`;
//! 2. apps from the catalog — a new manifest version or a changed compose → a stack redeploy with image pull;
//! 3. components (edge Caddy, Coder, Komodo, docker proxy) — `docker compose pull && up -d` by their compose files from
//! the host.

use crate::model::{AppManifest, AppView, RouteMode};
use crate::service::Service;
use crate::store;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering as AtomicOrdering;
use std::time::Duration;
use utoipa::ToSchema;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Build identifier (`CLOUDD_BUILD=YYYYMMDD-<git>` at cargo build, see scripts/release.sh); in development — dev.
pub fn build_id() -> &'static str { option_env!("CLOUDD_BUILD").unwrap_or("dev") }
pub const ARCH: &str = std::env::consts::ARCH;
pub const PACKAGE_NAME: &str = "agentverse-os";

const CHANNEL_KEY: &str = "update.channel";
const CHECK_KEY: &str = "update.check";
const APPLIED_KEY: &str = "update.applied";
const SUMMARY_KEY: &str = "update.summary";
const CATALOG_KEY: &str = "catalog.last_import";
const FLOATING: &[&str] = &["latest", "main", "master", "stable", "nightly", "edge", "dev", "develop", "rolling", "current"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ReleaseInfo {
    pub version: String,
    #[serde(default)] pub published: Option<String>,
    #[serde(default)] pub notes: Option<String>,
    #[serde(default)] pub url: Option<String>,
    #[serde(default)] pub sha256: Option<String>,
    #[serde(default)] pub size: Option<u64>,
}

/// `manifest.json` inside the package.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    #[serde(default)] pub build: Option<String>,
    #[serde(default)] pub arch: Option<String>,
    pub cloudd_sha256: String,
    #[serde(default)] pub min_version: Option<String>,
    #[serde(default)] pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PackageInfo {
    pub file: String,
    pub version: String,
    pub build: Option<String>,
    pub arch: Option<String>,
    pub size: u64,
    pub sha256: String,
    /// Hand-written Store manifests inside the
    /// package.
    pub store_apps: Vec<String>,
    pub notes: Option<String>,
    pub added_at: String,
    /// newer | same | older — relative to the running
    /// version.
    pub relation: String,
    pub arch_ok: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct SystemUpdateInfo {
    pub version: String,
    pub build: String,
    pub arch: String,
    pub binary: String,
    pub channel_url: Option<String>,
    pub latest: Option<ReleaseInfo>,
    pub available: bool,
    pub check_error: Option<String>,
    pub checked_at: Option<String>,
    pub packages: Vec<PackageInfo>,
    pub can_rollback: bool,
    pub prev_version: Option<String>,
    pub last_applied: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApplyResult { pub from: String, pub to: String, pub restart_in_s: u32, pub store_apps: usize }

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AppUpdateInfo {
    pub name: String,
    pub title: String,
    pub origin: String,
    pub installed_version: Option<String>,
    pub catalog_version: Option<String>,
    pub in_catalog: bool,
    pub version_changed: bool,
    pub compose_changed: bool,
    /// Images with a floating tag (latest etc.): "update" pulls a fresh image even if the catalog did not change.
    pub floating_tags: Vec<String>,
    pub has_update: bool,
    pub state: Option<String>,
    pub installed_at: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ComponentContainer { pub name: String, pub image: String, pub image_id: String, pub state: String, pub created: Option<String> }

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ComponentInfo {
    /// Compose project name (e.g. cloudos-edge, komodo).
    pub project: String,
    pub title: String,
    pub containers: Vec<ComponentContainer>,
    pub config_files: Vec<String>,
    pub working_dir: Option<String>,
    pub env_file: Option<String>,
    /// The compose files are on the host — `pull && up -d`
    /// is possible.
    pub updatable: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ComponentUpdateResult { pub project: String, pub changed: Vec<String>, pub log: String }

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct CatalogInfo { pub importing: bool, pub manifests: usize, pub last_import: Option<serde_json::Value> }

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdatesView { pub system: SystemUpdateInfo, pub apps: Vec<AppUpdateInfo>, pub components: Vec<ComponentInfo>, pub catalog: CatalogInfo }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChannelBody { pub url: String }
#[derive(Debug, Deserialize, ToSchema)]
pub struct ApplyBody { pub file: String }
#[derive(Debug, Deserialize, ToSchema)]
pub struct CatalogBody { #[serde(default)] pub source: Option<String> }

// ---------------------------------------------------------------------------------------------------------------------
// pure functions (tested in tests.rs)
// ---------------------------------------------------------------------------------------------------------------------

/// Version comparison `x.y.z[-pre]`: numeric parts in order, a version with a pre-release is older than the
/// same one without it.
pub fn version_cmp(a: &str, b: &str) -> Ordering {
    fn split(v: &str) -> (Vec<u64>, Option<String>) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once(['-', '+']) { Some((c, p)) => (c, Some(p.to_string())), None => (v, None) };
        (core.split('.').map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<u64>().unwrap_or(0)).collect(), pre)
    }
    let (an, ap) = split(a);
    let (bn, bp) = split(b);
    for i in 0..an.len().max(bn.len()) {
        let x = an.get(i).copied().unwrap_or(0);
        let y = bn.get(i).copied().unwrap_or(0);
        if x != y { return x.cmp(&y); }
    }
    match (ap, bp) { (None, None) => Ordering::Equal, (Some(_), None) => Ordering::Less, (None, Some(_)) => Ordering::Greater, (Some(x), Some(y)) => x.cmp(&y) }
}

/// Channel manifest (stable.json): `{ version, published?, notes?, assets: { "<arch>-unknown-linux-gnu" | "<arch>": { url, sha256?, size? } }
/// }`.
pub fn parse_channel(json: &str, arch: &str) -> Result<ReleaseInfo> {
    let v: serde_json::Value = serde_json::from_str(json).context("манифест канала: не JSON")?;
    let version = v.get("version").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).context("манифест канала: нет version")?.to_string();
    let assets = v.get("assets").and_then(|a| a.as_object());
    let asset = assets.and_then(|a| a.get(&format!("{arch}-unknown-linux-gnu")).or_else(|| a.get(arch)));
    let s = |o: Option<&serde_json::Value>, k: &str| o.and_then(|x| x.get(k)).and_then(|x| x.as_str()).map(String::from);
    Ok(ReleaseInfo {
        version,
        published: s(Some(&v), "published"),
        notes: s(Some(&v), "notes"),
        url: s(asset, "url"),
        sha256: s(asset, "sha256").map(|x| x.to_lowercase()),
        size: asset.and_then(|x| x.get("size")).and_then(|x| x.as_u64()),
    })
}

/// Does the deployed compose differ from the rendered one: compare the parsed YAML (formatting and comments do not count); if
/// either does not parse — the text without surrounding whitespace.
pub fn compose_differs(deployed: &str, rendered: &str) -> bool {
    match (serde_yaml_ng::from_str::<serde_json::Value>(deployed), serde_yaml_ng::from_str::<serde_json::Value>(rendered)) {
        (Ok(a), Ok(b)) => a != b,
        _ => deployed.trim() != rendered.trim(),
    }
}

/// Images with a floating tag in compose (services.*.image without a tag or with latest/main/…); `${VAR}`
/// is skipped.
pub fn floating_tags(compose: &str) -> Vec<String> {
    let Ok(v) = serde_yaml_ng::from_str::<serde_json::Value>(compose) else { return vec![] };
    let mut out = vec![];
    if let Some(services) = v.get("services").and_then(|s| s.as_object()) {
        for svc in services.values() {
            let Some(img) = svc.get("image").and_then(|i| i.as_str()) else { continue };
            if img.contains("${") { continue; }
            let no_digest = img.split('@').next().unwrap_or(img);
            let tag = match no_digest.rsplit_once(':') { Some((_, t)) if !t.contains('/') => Some(t), _ => None };
            if tag.map(|t| FLOATING.contains(&t.to_lowercase().as_str())).unwrap_or(true) && !out.contains(&img.to_string()) {
                out.push(img.to_string());
            }
        }
    }
    out
}

pub fn upstream_version(m: &AppManifest) -> Option<String> {
    m.upstream.as_ref()?.get("version")?.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn component_title(project: &str, container: &str) -> String {
    let p = project.to_lowercase();
    let c = container.to_lowercase();
    if c.contains("edge") || c.contains("caddy") || p.contains("edge") { "Edge (Caddy)".into() }
    else if p.contains("coder") || c.contains("coder") { "Coder + PostgreSQL".into() }
    else if p.contains("komodo") || c.contains("komodo") { "Komodo (App Runtime)".into() }
    else if p.contains("docker-proxy") || c.contains("docker-ro") { "Docker-прокси (read-only)".into() }
    else { project.to_string() }
}

/// Package file name without paths.
pub fn safe_file_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() || n.contains('/') || n.contains("..") || n.starts_with('.') { bail!("недопустимый путь пакета: {name}") }
    Ok(n.to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Read the package: manifest.json, the binary sha256 (checked against the manifest), the list of Store
/// apps inside.
pub fn inspect_package(path: &Path) -> Result<(PackageManifest, Vec<String>, u64, String)> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).with_context(|| format!("чтение {}", path.display()))?;
    let size = bytes.len() as u64;
    let file_sha = sha256_hex(&bytes);
    let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(&bytes));
    let mut ar = tar::Archive::new(gz);
    let mut manifest: Option<PackageManifest> = None;
    let mut bin_sha: Option<String> = None;
    let mut apps = vec![];
    for e in ar.entries().context("пакет не читается как tar.gz")? {
        let mut e = e.context("пакет повреждён или это не tar.gz")?;
        let p = e.path()?.to_path_buf();
        let rel = p.strip_prefix("./").unwrap_or(&p).to_path_buf();
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
        match parts.as_slice() {
            [m] if m == "manifest.json" => {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut e, &mut s)?;
                manifest = Some(serde_json::from_str(&s).context("manifest.json пакета")?);
            }
            [b] if b == "cloudd" => {
                let mut h = Sha256::new();
                std::io::copy(&mut e, &mut h)?;
                bin_sha = Some(format!("{:x}", h.finalize()));
            }
            [s, app, f] if s == "store" && f == "manifest.yaml" => apps.push(app.clone()),
            _ => {}
        }
    }
    let m = manifest.context("в пакете нет manifest.json")?;
    if m.name != PACKAGE_NAME { bail!("пакет {} — не {PACKAGE_NAME}", m.name) }
    let b = bin_sha.context("в пакете нет бинаря cloudd")?;
    if b != m.cloudd_sha256.to_lowercase() { bail!("sha256 бинаря не совпадает с manifest.json: пакет повреждён") }
    apps.sort();
    Ok((m, apps, size, file_sha))
}

fn extract_package(path: &Path, dest: &Path) -> Result<()> {
    let f = std::fs::File::open(path)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    ar.unpack(dest).with_context(|| format!("распаковка в {}", dest.display()))?;
    Ok(())
}

/// Hand-written manifests from the package → the Store catalog (only the files present in the package; compose.override.yaml and
/// login.yaml are left alone). App files that belong to the installation rather than the catalog: the package does not carry or
/// overwrite them.
pub(crate) const USER_FILES: [&str; 3] = ["compose.override.yaml", "login.yaml", "user.env"];

/// The whole manifest directory (compose, description, icon, hooks/ and other subdirectories), except user files.
fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for f in std::fs::read_dir(src)? {
        let f = f?;
        let name = f.file_name();
        if USER_FILES.iter().any(|u| *u == name.to_string_lossy()) { continue; }
        let p = f.path();
        if p.is_dir() { copy_tree(&p, &dst.join(&name))?; } else if p.is_file() { std::fs::copy(&p, dst.join(&name))?; }
    }
    Ok(())
}

pub(crate) fn copy_store(src: &Path, store_dir: &Path) -> Result<usize> {
    let mut n = 0;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let p = e.path();
        if p.is_file() && e.file_name() == "recommended.yaml" {
            std::fs::copy(&p, store_dir.join("recommended.yaml"))?;
            continue;
        }
        if !p.is_dir() || !p.join("manifest.yaml").exists() { continue; }
        copy_tree(&p, &store_dir.join(e.file_name()))?;
        n += 1;
    }
    Ok(n)
}

fn self_binary_path() -> PathBuf {
    let p = std::env::var("CLOUDD_SELF_PATH").map(PathBuf::from).or_else(|_| std::env::current_exe()).unwrap_or_else(|_| PathBuf::from("/usr/local/bin/cloudd"));
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn tail(s: &str, n: usize) -> String {
    if s.len() <= n { s.to_string() } else { format!("…{}", &s[s.len() - n..]) }
}

/// A background script outside the cloudd cgroup (a systemd transient unit): survives the service restart; the fallback is
/// plain sh.
fn detached_shell(script: &str) -> Result<()> {
    use std::process::{Command, Stdio};
    let unit = format!("cloudd-restart-{}", uuid::Uuid::new_v4().simple());
    let ok = Command::new("systemd-run").args(["--quiet", "--collect", "--unit", &unit, "/bin/sh", "-c", script]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    if !ok {
        Command::new("/bin/sh").args(["-c", script]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().context("sh -c")?;
    }
    Ok(())
}

async fn run_docker(args: &[String], step: &[&str], cwd: Option<&str>, env: &[(String, String)], timeout: Duration) -> Result<String> {
    let mut cmd = tokio::process::Command::new("docker");
    cmd.args(args).args(step).stdin(std::process::Stdio::null()).kill_on_drop(true);
    if let Some(c) = cwd { cmd.current_dir(c); }
    for (k, v) in env { cmd.env(k, v); }
    let out = tokio::time::timeout(timeout, cmd.output()).await.map_err(|_| anyhow::anyhow!("docker compose {}: не завершился за {} с", step.join(" "), timeout.as_secs()))?.context("docker compose")?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() { bail!("docker compose {}: {}", step.join(" "), tail(text.trim(), 1500)) }
    Ok(text)
}

// ---------------------------------------------------------------------------------------------------------------------

impl Service {
    pub fn updates_dir(&self) -> PathBuf { self.cfg.state_dir.join("updates") }

    pub fn channel_url(&self) -> Option<String> {
        self.store.get_kv(CHANNEL_KEY).ok().flatten().and_then(|v| v.get("url").and_then(|u| u.as_str()).map(String::from)).filter(|u| !u.is_empty())
            .or_else(|| std::env::var("CLOUDD_UPDATE_URL").ok().filter(|u| !u.is_empty()))
    }

    pub async fn set_channel(&self, url: &str) -> Result<SystemUpdateInfo> {
        let url = url.trim().to_string();
        if !url.is_empty() && !(url.starts_with("https://") || url.starts_with("http://")) { bail!("адрес канала должен начинаться с https:// или http://") }
        self.store.put_kv(CHANNEL_KEY, &json!({ "url": url }))?;
        if url.is_empty() {
            self.store.put_kv(CHECK_KEY, &json!(null))?;
            self.store.event("update", "channel", "канал обновлений отключён")?;
            return self.system_update_info();
        }
        self.store.event("update", "channel", &format!("канал обновлений: {url}"))?;
        self.check_system_update().await
    }

    fn list_packages(&self) -> Result<Vec<PackageInfo>> {
        let dir = self.updates_dir();
        let mut out = vec![];
        if !dir.exists() { return Ok(out); }
        let mut entries: Vec<_> = std::fs::read_dir(&dir)?.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tar.gz")).collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().and_then(|m| m.modified()).ok()));
        for e in entries {
            let file = e.file_name().to_string_lossy().to_string();
            let added_at = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| time::OffsetDateTime::from(t).format(&time::format_description::well_known::Rfc3339).ok()).unwrap_or_default();
            match inspect_package(&e.path()) {
                Ok((m, apps, size, sha)) => out.push(PackageInfo {
                    relation: match version_cmp(&m.version, VERSION) { Ordering::Greater => "newer", Ordering::Equal => "same", Ordering::Less => "older" }.into(),
                    arch_ok: m.arch.as_deref().map(|a| a.starts_with(ARCH)).unwrap_or(true),
                    file, version: m.version, build: m.build, arch: m.arch, size, sha256: sha, store_apps: apps, notes: m.notes, added_at,
                }),
                Err(err) => tracing::warn!("пакет {file}: {err:#}"),
            }
        }
        Ok(out)
    }

    pub fn system_update_info(&self) -> Result<SystemUpdateInfo> {
        let check = self.store.get_kv(CHECK_KEY)?.filter(|v| !v.is_null());
        let latest: Option<ReleaseInfo> = check.as_ref().and_then(|c| c.get("latest")).cloned().and_then(|v| serde_json::from_value(v).ok());
        let available = latest.as_ref().map(|l| version_cmp(&l.version, VERSION) == Ordering::Greater).unwrap_or(false);
        let bin = self_binary_path();
        let prev = PathBuf::from(format!("{}.prev", bin.display()));
        let applied = self.store.get_kv(APPLIED_KEY)?.filter(|v| !v.is_null());
        Ok(SystemUpdateInfo {
            version: VERSION.into(), build: build_id().into(), arch: ARCH.into(), binary: bin.display().to_string(),
            channel_url: self.channel_url(), latest, available,
            check_error: check.as_ref().and_then(|c| c.get("error")).and_then(|e| e.as_str()).map(String::from),
            checked_at: check.as_ref().and_then(|c| c.get("at")).and_then(|e| e.as_str()).map(String::from),
            packages: self.list_packages()?,
            can_rollback: prev.exists(),
            prev_version: applied.as_ref().and_then(|a| a.get("from")).and_then(|f| f.as_str()).map(String::from).filter(|_| prev.exists()),
            last_applied: applied,
        })
    }

    /// Poll the channel: the manifest by URL → kv update.check (the last result or error).
    pub async fn check_system_update(&self) -> Result<SystemUpdateInfo> {
        let Some(url) = self.channel_url() else { bail!("канал обновлений не настроен: укажите адрес манифеста (stable.json) или загрузите пакет обновления") };
        let res: Result<ReleaseInfo> = async {
            let c = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
            let txt = c.get(&url).header("User-Agent", format!("cloudd/{VERSION}")).send().await.context("запрос к каналу")?.error_for_status().context("канал ответил ошибкой")?.text().await?;
            parse_channel(&txt, ARCH)
        }.await;
        let rec = match &res {
            Ok(l) => json!({ "at": store::now(), "latest": l }),
            Err(e) => json!({ "at": store::now(), "error": e.chain().map(|c| c.to_string()).collect::<Vec<_>>().join(": ") }),
        };
        self.store.put_kv(CHECK_KEY, &rec)?;
        if let Ok(l) = &res {
            if version_cmp(&l.version, VERSION) == Ordering::Greater {
                self.store.event("update", "system", &format!("доступно обновление {} (сейчас {VERSION})", l.version))?;
            }
        }
        self.system_update_info()
    }

    /// Download the package from the channel into
    /// updates/ and verify it.
    pub async fn download_update(&self) -> Result<PackageInfo> {
        let info = self.check_system_update().await?;
        let latest = info.latest.context("канал не сообщил версию")?;
        let url = latest.url.context("в манифесте канала нет ссылки на пакет для этой архитектуры")?;
        let dir = self.updates_dir();
        std::fs::create_dir_all(&dir)?;
        let file = format!("{PACKAGE_NAME}-{}-{ARCH}.tar.gz", latest.version);
        let tmp = dir.join(format!("{file}.part"));
        let c = reqwest::Client::builder().timeout(Duration::from_secs(600)).build()?;
        let bytes = c.get(&url).header("User-Agent", format!("cloudd/{VERSION}")).send().await.context("скачивание пакета")?.error_for_status()?.bytes().await?;
        if let Some(want) = &latest.sha256 {
            let got = sha256_hex(&bytes);
            if &got != want { bail!("sha256 скачанного пакета не совпадает с манифестом канала") }
        }
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, dir.join(&file))?;
        self.store.event("update", "system", &format!("скачан пакет {file} ({} МБ)", bytes.len() / 1_048_576))?;
        self.package_info(&file)
    }

    fn package_info(&self, file: &str) -> Result<PackageInfo> {
        self.list_packages()?.into_iter().find(|p| p.file == file).with_context(|| format!("пакет {file} не прошёл проверку"))
    }

    /// A package uploaded through Desktop: save and verify; a broken one is
    /// deleted.
    pub fn upload_update(&self, name: &str, bytes: &[u8]) -> Result<PackageInfo> {
        let dir = self.updates_dir();
        std::fs::create_dir_all(&dir)?;
        let base: String = Path::new(name).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
        let base = if base.ends_with(".tar.gz") { base } else { format!("{}.tar.gz", base.trim_end_matches(".tgz")) };
        let file = format!("upload-{}-{base}", time::OffsetDateTime::now_utc().unix_timestamp());
        let path = dir.join(&file);
        std::fs::write(&path, bytes)?;
        match inspect_package(&path) {
            Ok(_) => {
                self.store.event("update", "system", &format!("загружен пакет {file} ({} МБ)", bytes.len() / 1_048_576))?;
                self.package_info(&file)
            }
            Err(e) => { let _ = std::fs::remove_file(&path); Err(e) }
        }
    }

    pub fn delete_package(&self, file: &str) -> Result<()> {
        let p = self.updates_dir().join(safe_file_name(file)?);
        if p.exists() { std::fs::remove_file(&p)?; }
        Ok(())
    }

    /// Install the package: verify → the binary runs → store/ from the package → the current binary to .prev → replacement → restart
    /// with a watchdog.
    pub async fn apply_update(&self, file: &str) -> Result<ApplyResult> {
        use std::os::unix::fs::PermissionsExt;
        let _g = self.lock.lock().await;
        let dir = self.updates_dir();
        let path = dir.join(safe_file_name(file)?);
        if !path.exists() { bail!("пакет {file} не найден") }
        let (pm, _apps, _size, _sha) = inspect_package(&path)?;
        if let Some(a) = &pm.arch { if !a.starts_with(ARCH) { bail!("пакет собран для {a}, а хост — {ARCH}") } }
        if let Some(minv) = &pm.min_version { if version_cmp(VERSION, minv) == Ordering::Less { bail!("пакет требует версию не ниже {minv} (сейчас {VERSION}): сначала обновитесь до неё") } }
        let stage = dir.join(format!("stage-{}", pm.version));
        let _ = std::fs::remove_dir_all(&stage);
        std::fs::create_dir_all(&stage)?;
        extract_package(&path, &stage)?;
        let new_bin = stage.join("cloudd");
        std::fs::set_permissions(&new_bin, std::fs::Permissions::from_mode(0o755))?;
        let out = crate::setup::run(&new_bin.display().to_string(), &["--version"], Duration::from_secs(20)).await.context("новый cloudd не запускается")?;
        if !out.status.success() { bail!("новый cloudd --version завершился с ошибкой: {}", String::from_utf8_lossy(&out.stderr).trim()) }
        let reported = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let store_apps = if stage.join("store").is_dir() { copy_store(&stage.join("store"), &self.cfg.store_dir())? } else { 0 };
        let bin = self_binary_path();
        let prev = PathBuf::from(format!("{}.prev", bin.display()));
        let tmp = PathBuf::from(format!("{}.new", bin.display()));
        if bin.exists() { std::fs::copy(&bin, &prev).with_context(|| format!("копия текущего бинаря в {}", prev.display()))?; }
        std::fs::copy(&new_bin, &tmp)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, &bin).with_context(|| format!("замена {}", bin.display()))?;
        let _ = std::fs::remove_dir_all(&stage);
        self.store.put_kv(APPLIED_KEY, &json!({ "from": VERSION, "to": pm.version, "build": pm.build, "at": store::now(), "package": file, "pending": true, "store_apps": store_apps }))?;
        self.store.event("update", "system", &format!("установлен пакет {} ({reported}), манифестов Store обновлено: {store_apps}; перезапуск cloudd, автооткат, если ядро не ответит за 45 с", pm.version))?;
        self.schedule_restart(true)?;
        Ok(ApplyResult { from: VERSION.into(), to: pm.version, restart_in_s: 3, store_apps })
    }

    /// Bring back the previous binary (`.prev`) and
    /// restart.
    pub async fn rollback_update(&self) -> Result<()> {
        let _g = self.lock.lock().await;
        let bin = self_binary_path();
        let prev = PathBuf::from(format!("{}.prev", bin.display()));
        if !prev.exists() { bail!("предыдущей версии нет ({})", prev.display()) }
        let failed = PathBuf::from(format!("{}.failed", bin.display()));
        std::fs::copy(&bin, &failed)?;
        let tmp = PathBuf::from(format!("{}.new", bin.display()));
        std::fs::copy(&prev, &tmp)?;
        std::fs::rename(&tmp, &bin)?;
        std::fs::remove_file(&prev)?;
        let applied = self.store.get_kv(APPLIED_KEY)?.unwrap_or(json!({}));
        self.store.put_kv(APPLIED_KEY, &json!({ "rolled_back": true, "from": VERSION, "to": applied.get("from"), "at": store::now() }))?;
        self.store.event("update", "system", &format!("откат на предыдущую версию {}; перезапуск cloudd", applied.get("from").and_then(|f| f.as_str()).unwrap_or("?")))?;
        self.schedule_restart(false)?;
        Ok(())
    }

    /// Restart cloudd in the background; with a watchdog — rollback to `.prev` if the core did
    /// not answer within 45 s.
    pub(crate) fn schedule_restart(&self, watchdog: bool) -> Result<()> {
        let dir = self.updates_dir();
        std::fs::create_dir_all(&dir)?;
        let log = dir.join("restart.log").display().to_string();
        let bin = self_binary_path().display().to_string();
        let probe = format!("http://{}/api/status", self.cfg.listen.first().cloned().unwrap_or_else(|| "127.0.0.1:7100".into()));
        let mut script = format!("echo \"$(date -Is) перезапуск cloudd\" >> {log}\nsleep 2\nsystemctl restart cloudd\n");
        if watchdog {
            script.push_str(&format!(
                "for i in $(seq 1 45); do sleep 1; curl -fs -m 3 {probe} >/dev/null 2>&1 && {{ echo \"$(date -Is) ядро отвечает\" >> {log}; exit 0; }}; done\n\
                 echo \"$(date -Is) ядро не ответило за 45 с — откат на {bin}.prev\" >> {log}\n\
                 cp -f {bin} {bin}.failed && cp -f {bin}.prev {bin} && systemctl restart cloudd\n"
            ));
        }
        detached_shell(&script)
    }

    /// At start: if the previous run installed an update — record the result.
    pub fn finish_pending_update(&self) {
        if let Ok(Some(mut a)) = self.store.get_kv(APPLIED_KEY) {
            if a.get("pending").and_then(|p| p.as_bool()) == Some(true) {
                let to = a.get("to").and_then(|t| t.as_str()).unwrap_or("").to_string();
                a["pending"] = json!(false);
                a["result"] = json!(if to == VERSION { "ok" } else { "version_mismatch" });
                let _ = self.store.put_kv(APPLIED_KEY, &a);
                let _ = self.store.event("update", "system", &if to == VERSION { format!("обновление до {VERSION} применено") } else { format!("после обновления работает {VERSION}, ожидалась {to}") });
            }
        }
    }

    // ------------------------------------------------------------------------------------------------ apps

    pub async fn app_updates(&self) -> Result<Vec<AppUpdateInfo>> {
        let states = self.komodo.states().await.unwrap_or_default();
        let mut out = vec![];
        for st in self.store.list_apps()? {
            if st.manifest.kind == crate::model::AppKind::External { continue; } // no stack — nothing to compare
            let cat = self.manifest(&st.name).ok();
            let installed_version = upstream_version(&st.manifest);
            let catalog_version = cat.as_ref().and_then(upstream_version);
            let (compose_changed, floating) = match &cat {
                Some(m) => {
                    let rendered = self.compose_text(m).unwrap_or_default();
                    let deployed = self.komodo.get_stack(&st.name).await.ok().flatten().and_then(|s| s.get("config").and_then(|c| c.get("file_contents")).and_then(|f| f.as_str()).map(String::from));
                    (deployed.map(|d| compose_differs(&d, &rendered)).unwrap_or(false), floating_tags(&rendered))
                }
                None => (false, vec![]),
            };
            let version_changed = matches!((&installed_version, &catalog_version), (Some(a), Some(b)) if a != b);
            let updated_at = self.store.get_kv(&format!("app.{}.updated_at", st.name))?.and_then(|v| v.as_str().map(String::from));
            out.push(AppUpdateInfo {
                title: st.manifest.title.clone().unwrap_or_else(|| st.name.clone()),
                origin: st.manifest.origin.clone(),
                in_catalog: cat.is_some(),
                has_update: version_changed || compose_changed,
                state: states.get(&st.name).cloned(),
                installed_at: st.installed_at.clone(),
                name: st.name, installed_version, catalog_version, version_changed, compose_changed, floating_tags: floating, updated_at,
            });
        }
        out.sort_by(|a, b| b.has_update.cmp(&a.has_update).then(a.name.cmp(&b.name)));
        Ok(out)
    }

    /// Update an app from the catalog: a manifest snapshot → the current compose and env → redeploy (Komodo pulls the images).
    pub async fn app_update(&self, name: &str) -> Result<AppView> {
        let _g = self.lock.lock().await;
        let m = self.manifest(name)?;
        let Some(mut st) = self.store.get_app(name)? else { bail!("{name} не установлено") };
        let from = upstream_version(&st.manifest);
        let to = upstream_version(&m);
        st.manifest = m.clone();
        if m.route.mode == RouteMode::Port && st.port.is_none() { st.port = Some(self.alloc_port(m.route.port)?); }
        self.store.upsert_app(&st)?;
        let what = match (&from, &to) { (Some(a), Some(b)) if a != b => format!(": {a} → {b}"), (_, Some(b)) => format!(" ({b}, свежие образы)"), _ => " (свежие образы)".into() };
        self.store.event("update", name, &format!("обновление{what}"))?;
        self.redeploy(name).await.with_context(|| format!("{name}: перевыкатка"))?;
        self.reload_edge().await?;
        self.store.put_kv(&format!("app.{name}.updated_at"), &json!(store::now()))?;
        self.store.event("update", name, "обновлено")?;
        self.app(name).await
    }

    /// Catalog re-import from upstream in the background (runtipi | coolify | umbrel | all); hand-written manifests
    /// are left alone.
    pub fn catalog_refresh(&self, source: &str) -> Result<bool> {
        let source = match source { "" | "all" => "all", s @ ("runtipi" | "coolify" | "umbrel") => s, other => bail!("источник {other}: runtipi | coolify | umbrel | all") }.to_string();
        if self.importing.swap(true, AtomicOrdering::SeqCst) { return Ok(false); }
        let me = self.clone();
        let _ = me.store.event("update", "catalog", &format!("обновление каталога ({source}) запущено"));
        tokio::spawn(async move {
            let r = crate::importer::import(&source, &me.cfg.store_dir(), &[]).await;
            let report = match r {
                Ok(reps) => json!({
                    "at": store::now(), "source": source, "ok": true,
                    "imported": reps.iter().map(|r| r.imported.len()).sum::<usize>(),
                    "details": reps.iter().map(|r| json!({ "source": r.source, "imported": r.imported.len(), "skipped_manual": r.skipped_manual.len(),
                        "errors": r.skipped.iter().filter(|(n, _)| n == "*").map(|(_, e)| e.clone()).collect::<Vec<_>>() })).collect::<Vec<_>>(),
                }),
                Err(e) => json!({ "at": store::now(), "source": source, "ok": false, "error": format!("{e:#}") }),
            };
            let _ = me.store.put_kv(CATALOG_KEY, &report);
            let msg = if report["ok"].as_bool() == Some(true) { format!("каталог обновлён: {} манифестов", report["imported"]) } else { format!("каталог не обновился: {}", report["error"]) };
            let _ = me.store.event("update", "catalog", &msg);
            me.importing.store(false, AtomicOrdering::SeqCst);
        });
        Ok(true)
    }

    // ------------------------------------------------------------------------------------------------ components

    /// Host compose projects other than Store apps: edge, Coder, Komodo, docker proxy.
    pub async fn components(&self) -> Result<Vec<ComponentInfo>> {
        let apps: HashSet<String> = self.store.list_apps()?.into_iter().map(|a| a.stack_name()).collect();
        let list = self.docker.docker.list_containers(Some(bollard::query_parameters::ListContainersOptions { all: true, ..Default::default() })).await?;
        let mut by: BTreeMap<String, ComponentInfo> = BTreeMap::new();
        for c in list {
            let labels = c.labels.unwrap_or_default();
            let Some(project) = labels.get("com.docker.compose.project").cloned() else { continue };
            if apps.contains(&project) { continue; }
            let wd = labels.get("com.docker.compose.project.working_dir").cloned();
            if wd.as_deref().map(|w| w.contains("/periphery/stacks/")).unwrap_or(false) { continue; }
            let files: Vec<String> = labels.get("com.docker.compose.project.config_files").map(|f| f.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
            let env_file = labels.get("com.docker.compose.project.environment_file").cloned().filter(|s| !s.is_empty());
            let name = c.names.unwrap_or_default().first().map(|n| n.trim_start_matches('/').to_string()).unwrap_or_default();
            let created = c.created.and_then(|t| time::OffsetDateTime::from_unix_timestamp(t).ok()).and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok());
            let e = by.entry(project.clone()).or_insert_with(|| ComponentInfo {
                title: component_title(&project, &name),
                updatable: !files.is_empty() && files.iter().all(|f| Path::new(f).exists()),
                note: if files.is_empty() { Some("контейнеры не из docker compose".into()) } else if !files.iter().all(|f| Path::new(f).exists()) { Some(format!("compose-файлы не найдены на хосте: {}", files.join(", "))) } else { None },
                project: project.clone(), containers: vec![], config_files: files, working_dir: wd, env_file,
            });
            if e.title == e.project { e.title = component_title(&project, &name); }
            e.containers.push(ComponentContainer { name, image: c.image.unwrap_or_default(), image_id: c.image_id.unwrap_or_default(), state: c.state.map(|s| s.to_string()).unwrap_or_default(), created });
        }
        for c in by.values_mut() { c.containers.sort_by(|a, b| a.name.cmp(&b.name)); }
        Ok(by.into_values().collect())
    }

    fn compose_env(&self) -> Vec<(String, String)> {
        // the Coder compose expects EDGE_HOST and INCUS_ADMIN_GID from the environment (bootstrap/install.sh) — otherwise recreation
        // breaks the access URL
        let gid = std::fs::read_to_string("/etc/group").ok().and_then(|g| g.lines().find_map(|l| { let mut p = l.split(':'); (p.next()? == "incus-admin").then(|| p.nth(1).map(String::from)).flatten() })).unwrap_or_else(|| "0".into());
        vec![("EDGE_HOST".into(), self.cfg.edge_host.clone()), ("INCUS_ADMIN_GID".into(), gid), ("COMPOSE_STATUS_STDOUT".into(), "1".into())]
    }

    /// `docker compose pull && up -d` of a component; after the edge is recreated — reconcile (app networks and the Caddy
    /// configuration).
    pub async fn component_update(&self, project: &str) -> Result<ComponentUpdateResult> {
        let comps = self.components().await?;
        let Some(c) = comps.into_iter().find(|c| c.project == project) else { bail!("компонент {project} не найден") };
        if !c.updatable { bail!("{}", c.note.clone().unwrap_or_else(|| "компонент нельзя обновить автоматически".into())) }
        let before: BTreeMap<String, String> = c.containers.iter().map(|x| (x.name.clone(), x.image_id.clone())).collect();
        let mut args: Vec<String> = vec!["compose".into(), "-p".into(), project.to_string()];
        for f in &c.config_files { args.push("-f".into()); args.push(f.clone()); }
        let wd = c.working_dir.clone().or_else(|| c.config_files.first().and_then(|f| Path::new(f).parent().map(|p| p.display().to_string())));
        if let Some(ef) = c.env_file.as_ref().filter(|f| Path::new(f).exists()) {
            args.push("--env-file".into()); args.push(ef.clone());
        } else if let Some(w) = &wd {
            let ce = Path::new(w).join("compose.env");
            if ce.exists() { args.push("--env-file".into()); args.push(ce.display().to_string()); }
        }
        let env = self.compose_env();
        self.store.event("update", project, "docker compose pull")?;
        let mut log = run_docker(&args, &["pull"], wd.as_deref(), &env, Duration::from_secs(900)).await?;
        self.store.event("update", project, "docker compose up -d")?;
        log.push_str(&run_docker(&args, &["up", "-d"], wd.as_deref(), &env, Duration::from_secs(600)).await?);
        if c.title.starts_with("Edge") {
            // a new edge container starts with an empty Caddyfile and without app
            // networks
            tokio::time::sleep(Duration::from_secs(2)).await;
            self.reconcile_all().await.context("reconcile после обновления edge")?;
        }
        let after = self.components().await?.into_iter().find(|x| x.project == project).map(|x| x.containers).unwrap_or_default();
        let changed: Vec<String> = after.iter().filter(|x| before.get(&x.name).map(|id| id != &x.image_id).unwrap_or(true)).map(|x| x.name.clone()).collect();
        self.store.event("update", project, &if changed.is_empty() { "компонент уже актуален".to_string() } else { format!("обновлены контейнеры: {}", changed.join(", ")) })?;
        Ok(ComponentUpdateResult { project: project.to_string(), changed, log: tail(log.trim(), 4000) })
    }

    // ------------------------------------------------------------------------------------------------
    // summary

    pub async fn updates_view(&self) -> Result<UpdatesView> {
        let system = self.system_update_info()?;
        let apps = self.app_updates().await?;
        let components = self.components().await.unwrap_or_default();
        let n = apps.iter().filter(|a| a.has_update).count();
        self.store.put_kv(SUMMARY_KEY, &json!({ "apps": n, "at": store::now() }))?;
        Ok(UpdatesView { system, apps, components, catalog: CatalogInfo { importing: self.importing.load(AtomicOrdering::SeqCst), manifests: self.catalog().map(|c| c.len()).unwrap_or(0), last_import: self.store.get_kv(CATALOG_KEY)? } })
    }

    /// For /api/status: whether a system update is available and how many apps can be updated (per the last check).
    pub fn update_summary(&self) -> (bool, u32) {
        let sys = self.store.get_kv(CHECK_KEY).ok().flatten().and_then(|c| c.get("latest").and_then(|l| l.get("version")).and_then(|v| v.as_str()).map(|v| version_cmp(v, VERSION) == Ordering::Greater)).unwrap_or(false);
        let apps = self.store.get_kv(SUMMARY_KEY).ok().flatten().and_then(|s| s.get("apps").and_then(|a| a.as_u64())).unwrap_or(0) as u32;
        (sys, apps)
    }

    /// Background: 2 minutes after start and then every 6 hours — poll the channel (if set) and the app summary.
    pub fn start_update_checker(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            me.finish_pending_update();
            tokio::time::sleep(Duration::from_secs(120)).await;
            loop {
                if me.channel_url().is_some() {
                    if let Err(e) = me.check_system_update().await { tracing::warn!("проверка обновлений: {e:#}"); }
                }
                if let Ok(apps) = me.app_updates().await {
                    let n = apps.iter().filter(|a| a.has_update).count();
                    let _ = me.store.put_kv(SUMMARY_KEY, &json!({ "apps": n, "at": store::now() }));
                }
                tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
            }
        });
    }
}
