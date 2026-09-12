//! Этап 4 — бэкапы (docs/backups.md): снимки ZFS по расписанию и restic из снимка в tank/backups.
//!
//! По умолчанию ВЫКЛЮЧЕНО (kv `backup.config`, `enabled: false`): диск стенда небольшой, решение пользователя. Пока выключено, не работают
//! расписание, чистка по политике и снимки перед удалением/перевыкаткой; ручные «снимок сейчас», «копия сейчас» и восстановление
//! приложения доступны всегда. Имена снимков: `<dataset>@cloudos-<kind>-<ГГГГММДД-ЧЧММ>`, kind ∈ hourly | daily | weekly | manual |
//! pre-<операция>-<приложение>. Снимки — защита «здесь и сейчас» (откат приложения из `.zfs/snapshot`), restic — от потери диска.
use crate::service::Service;
use crate::store;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

/// Что снимаем по расписанию (tank/docker — одноразовое, не снимается).
pub const DATASETS: &[&str] = &["tank/core", "tank/postgres", "tank/apps", "tank/workspaces"];
/// Что уходит в restic: датасет → каталог в staging. Workspaces — только снимки ZFS: zvol'ы Incus как файлы не читаются.
pub const RESTIC_SETS: &[(&str, &str)] = &[("tank/core", "core"), ("tank/postgres", "postgres"), ("tank/apps", "apps")];
pub const KV_CONFIG: &str = "backup.config";
pub const KV_LAST: &str = "backup.last";
pub const STAGE: &str = "/srv/backups/.stage";
pub const RESTIC_REPO: &str = "/srv/backups/restic";
pub const PRE_KEEP: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BackupConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "d24")]
    pub hourly: u32,
    #[serde(default = "d7")]
    pub daily: u32,
    #[serde(default = "d4")]
    pub weekly: u32,
    /// Ночью после суточного снимка — restic в tank/backups.
    #[serde(default = "dtrue")]
    pub restic: bool,
    #[serde(default = "d7")]
    pub restic_keep_daily: u32,
    #[serde(default = "d4")]
    pub restic_keep_weekly: u32,
    #[serde(default = "d6")]
    pub restic_keep_monthly: u32,
    /// Внешний репозиторий restic (`s3:…`, `sftp:…`, `rclone:…`); пусто — только локальный. Переменные доступа — секрет system/RESTIC_REMOTE_ENV.
    #[serde(default)]
    pub remote: String,
}
fn d24() -> u32 { 24 }
fn d7() -> u32 { 7 }
fn d4() -> u32 { 4 }
fn d6() -> u32 { 6 }
fn dtrue() -> bool { true }
impl Default for BackupConfig {
    fn default() -> Self { Self { enabled: false, hourly: 24, daily: 7, weekly: 4, restic: true, restic_keep_daily: 7, restic_keep_weekly: 4, restic_keep_monthly: 6, remote: String::new() } }
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SnapshotInfo { pub dataset: String, pub name: String, pub kind: String, pub created: i64, pub used: u64 }

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DatasetInfo { pub name: String, pub mountpoint: String, pub used: u64, pub snapshots: usize, pub snapshots_used: u64, pub last: Option<i64> }

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct BackupStatus {
    pub config: BackupConfig,
    pub datasets: Vec<DatasetInfo>,
    pub restic_installed: bool,
    pub restic_repo: String,
    pub restic_snapshots: usize,
    pub restic_size: u64,
    /// {"snapshot": {...последний снимок...}, "restic": {...последняя копия...}}
    pub last: Value,
    pub running: bool,
}

/// Точка восстановления приложения: снимок tank/apps, в котором есть каталог приложения.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct RestorePoint { pub snapshot: String, pub kind: String, pub created: i64, pub path: String }

pub async fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let out = tokio::process::Command::new(cmd).args(args).output().await.with_context(|| format!("{cmd} {}", args.join(" ")))?;
    if !out.status.success() { bail!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()); }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

async fn run_env(cmd: &str, args: &[&str], env: &[(String, String)]) -> Result<String> {
    let mut c = tokio::process::Command::new(cmd);
    c.args(args);
    for (k, v) in env { c.env(k, v); }
    let out = c.output().await.with_context(|| format!("{cmd} {}", args.join(" ")))?;
    if !out.status.success() { bail!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim().chars().take(600).collect::<String>()); }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn stamp(t: OffsetDateTime) -> String { format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.year(), t.month() as u8, t.day(), t.hour(), t.minute(), t.second()) }
pub fn snap_name(kind: &str, t: OffsetDateTime) -> String { format!("cloudos-{kind}-{}", stamp(t)) }
/// kind из имени снимка: `cloudos-hourly-2026…` → hourly, `cloudos-pre-remove-x-2026…` → pre.
pub fn kind_of(snap: &str) -> Option<String> { snap.strip_prefix("cloudos-")?.split('-').next().map(|s| s.to_string()) }

/// Все снимки Cloud OS (по всем датасетам, включая дочерние tank/workspaces/*).
pub async fn list_snapshots() -> Result<Vec<SnapshotInfo>> {
    let out = run("zfs", &["list", "-t", "snapshot", "-H", "-p", "-o", "name,creation,used"]).await?;
    let mut v = Vec::new();
    for line in out.lines() {
        let mut it = line.split('\t');
        let (Some(full), Some(created), Some(used)) = (it.next(), it.next(), it.next()) else { continue };
        let Some((ds, snap)) = full.split_once('@') else { continue };
        let Some(kind) = kind_of(snap) else { continue };
        v.push(SnapshotInfo { dataset: ds.to_string(), name: snap.to_string(), kind, created: created.trim().parse().unwrap_or(0), used: used.trim().parse().unwrap_or(0) });
    }
    Ok(v)
}

pub async fn datasets() -> Result<Vec<DatasetInfo>> {
    let snaps = list_snapshots().await?;
    let mut v = Vec::new();
    for ds in DATASETS {
        let Ok(out) = run("zfs", &["list", "-H", "-p", "-o", "name,mountpoint,used,usedbysnapshots", ds]).await else { continue };
        let f: Vec<&str> = out.trim().split('\t').collect();
        if f.len() < 4 { continue; }
        let mine: Vec<&SnapshotInfo> = snaps.iter().filter(|s| s.dataset == *ds).collect();
        v.push(DatasetInfo { name: ds.to_string(), mountpoint: f[1].to_string(), used: f[2].parse().unwrap_or(0), snapshots: mine.len(), snapshots_used: f[3].parse().unwrap_or(0), last: mine.iter().map(|s| s.created).max() });
    }
    Ok(v)
}

/// Снимок указанных датасетов (рекурсивно — у tank/workspaces дочерние датасеты Incus). Возвращает короткое имя снимка.
pub async fn snapshot_of(datasets: &[&str], kind: &str) -> Result<String> {
    let mut name = snap_name(kind, OffsetDateTime::now_utc());
    let existing = list_snapshots().await.unwrap_or_default();
    if existing.iter().any(|s| s.name == name) { name.push_str("-2"); }
    for ds in datasets { run("zfs", &["snapshot", "-r", &format!("{ds}@{name}")]).await?; }
    Ok(name)
}

pub async fn destroy(dataset: &str, snap: &str) -> Result<()> { run("zfs", &["destroy", "-r", &format!("{dataset}@{snap}")]).await.map(|_| ()) }

/// Что удалить по политике: на каждый датасет верхнего уровня и kind оставляем N новейших (pre — 10, manual — все).
pub fn prune_plan(snaps: &[SnapshotInfo], cfg: &BackupConfig) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for ds in DATASETS {
        for kind in ["hourly", "daily", "weekly", "pre"] {
            let keep = match kind { "hourly" => cfg.hourly as usize, "daily" => cfg.daily as usize, "weekly" => cfg.weekly as usize, _ => PRE_KEEP };
            let mut mine: Vec<&SnapshotInfo> = snaps.iter().filter(|s| s.dataset == *ds && s.kind == kind).collect();
            mine.sort_by(|a, b| b.created.cmp(&a.created));
            for s in mine.into_iter().skip(keep) { out.push((s.dataset.clone(), s.name.clone())); }
        }
    }
    out
}

fn human(b: u64) -> String {
    if b >= 1 << 30 { format!("{:.1} GiB", b as f64 / (1u64 << 30) as f64) } else if b >= 1 << 20 { format!("{:.1} MiB", b as f64 / (1u64 << 20) as f64) } else { format!("{} KiB", b / 1024) }
}

impl Service {
    pub fn backup_config(&self) -> BackupConfig {
        self.store.get_kv(KV_CONFIG).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    pub fn set_backup_config(&self, c: &BackupConfig) -> Result<BackupConfig> {
        self.store.put_kv(KV_CONFIG, &serde_json::to_value(c)?)?;
        self.store.event("backup", "config", &format!("бэкапы {}: снимки hourly {} / daily {} / weekly {}, restic {}{}", if c.enabled { "включены" } else { "выключены" }, c.hourly, c.daily, c.weekly, if c.restic { "включён" } else { "выключен" }, if c.remote.is_empty() { String::new() } else { format!(", внешний репозиторий {}", c.remote) }))?;
        Ok(c.clone())
    }

    /// Планировщик: раз в минуту смотрит на часы. Каждый час — hourly-снимок; в 03:00 UTC — daily (по воскресеньям weekly) и restic.
    pub fn start_backup_scheduler(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut last_hour: Option<i64> = None;
            let mut last_day: Option<i32> = None;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                let cfg = me.backup_config();
                if !cfg.enabled { continue; }
                let now = OffsetDateTime::now_utc();
                let hour_id = now.unix_timestamp() / 3600;
                let day_id = now.to_julian_day();
                if now.hour() == 3 && last_day != Some(day_id) {
                    last_day = Some(day_id);
                    last_hour = Some(hour_id);
                    let kind = if now.weekday() == time::Weekday::Sunday { "weekly" } else { "daily" };
                    if let Err(e) = me.backup_cycle(kind, cfg.restic).await { let _ = me.store.event("backup", kind, &format!("ошибка: {e:#}")); }
                } else if now.minute() < 5 && last_hour != Some(hour_id) && cfg.hourly > 0 {
                    last_hour = Some(hour_id);
                    if let Err(e) = me.backup_cycle("hourly", false).await { let _ = me.store.event("backup", "hourly", &format!("ошибка: {e:#}")); }
                }
            }
        });
    }

    /// Снимок всех датасетов + чистка по политике (+ restic из этого снимка). Один цикл за раз.
    pub async fn backup_cycle(&self, kind: &str, with_restic: bool) -> Result<Value> {
        let _g = self.backup_lock.lock().await;
        self.backup_running.store(true, std::sync::atomic::Ordering::Relaxed);
        let started = std::time::Instant::now();
        let res = async {
            let name = snapshot_of(DATASETS, kind).await?;
            let cfg = self.backup_config();
            let snaps = list_snapshots().await?;
            let mut pruned = 0;
            for (ds, s) in prune_plan(&snaps, &cfg) { destroy(&ds, &s).await?; pruned += 1; }
            let mut res = json!({ "at": store::now(), "kind": kind, "snapshot": name, "pruned": pruned, "ok": true });
            let mut last = self.store.get_kv(KV_LAST)?.unwrap_or_else(|| json!({}));
            last["snapshot"] = res.clone();
            self.store.put_kv(KV_LAST, &last)?;
            self.store.event("backup", kind, &format!("снимок {name} ({} датасетов), удалено по политике: {pruned}", DATASETS.len()))?;
            if with_restic && cfg.restic {
                match self.restic_backup(&name, kind, &cfg).await {
                    Ok(v) => { res["restic"] = v.clone(); last["restic"] = v; }
                    Err(e) => { let m = format!("{e:#}"); res["restic_error"] = json!(m); last["restic"] = json!({ "at": store::now(), "ok": false, "error": m }); self.store.event("backup", "restic", &format!("ошибка: {e:#}"))?; }
                }
                self.store.put_kv(KV_LAST, &last)?;
            }
            res["seconds"] = json!(started.elapsed().as_secs());
            Ok::<Value, anyhow::Error>(res)
        }
        .await;
        self.backup_running.store(false, std::sync::atomic::Ordering::Relaxed);
        res
    }

    fn restic_env(&self, repo: &str) -> Result<Vec<(String, String)>> {
        let pw = match self.store.get_secret("system", "RESTIC_PASSWORD")? {
            Some(p) => p,
            None => { let p = store::generate(crate::model::GenerateKind::Hex64); self.store.put_secret("system", "RESTIC_PASSWORD", &p)?; self.store.event("backup", "restic", "создан пароль репозитория restic (секрет system/RESTIC_PASSWORD — сохраните его вне сервера)")?; p }
        };
        let mut env = vec![("RESTIC_REPOSITORY".to_string(), repo.to_string()), ("RESTIC_PASSWORD".to_string(), pw), ("RESTIC_CACHE_DIR".to_string(), "/srv/backups/.cache".to_string())];
        if repo != RESTIC_REPO {
            if let Some(extra) = self.store.get_secret("system", "RESTIC_REMOTE_ENV")? {
                for (k, v) in crate::service::parse_env(&extra) { env.push((k, v)); }
            }
        }
        Ok(env)
    }

    /// restic из снимка: клоны датасетов в /srv/backups/.stage/<имя> (стабильные пути между запусками), дамп Postgres Coder,
    /// `restic backup`, `forget --prune` по политике. Затем то же во внешний репозиторий, если задан.
    async fn restic_backup(&self, snap: &str, kind: &str, cfg: &BackupConfig) -> Result<Value> {
        if run("which", &["restic"]).await.is_err() { bail!("restic не установлен на хосте (apt install restic)"); }
        let stage = Path::new(STAGE);
        std::fs::create_dir_all(stage.join("dumps"))?;
        // клоны снимков со стабильными путями
        for (ds, sub) in RESTIC_SETS {
            let clone = format!("tank/backups/stage-{sub}");
            let _ = run("zfs", &["destroy", "-f", &clone]).await;
            run("zfs", &["clone", "-o", "readonly=on", "-o", &format!("mountpoint={STAGE}/{sub}"), &format!("{ds}@{snap}"), &clone]).await?;
        }
        // дамп базы Coder (логическая копия рядом с файлами tank/postgres)
        match self.docker.exec(&self.cfg.coder_pg_container, vec!["pg_dumpall", "-U", "coder"]).await {
            Ok(sql) if sql.len() > 100 => { std::fs::write(stage.join("dumps/coder.sql"), sql)?; }
            Ok(_) => { self.store.event("backup", "restic", "дамп Postgres Coder пустой — пропущен")?; }
            Err(e) => { self.store.event("backup", "restic", &format!("дамп Postgres Coder не снят: {e:#}"))?; }
        }
        let result = async {
            let mut out = json!({ "at": store::now(), "kind": kind, "snapshot": snap, "ok": true });
            for repo in std::iter::once(RESTIC_REPO.to_string()).chain(if cfg.remote.trim().is_empty() { None } else { Some(cfg.remote.trim().to_string()) }) {
                let env = self.restic_env(&repo)?;
                if run_env("restic", &["cat", "config"], &env).await.is_err() {
                    run_env("restic", &["init"], &env).await.with_context(|| format!("restic init {repo}"))?;
                    self.store.event("backup", "restic", &format!("репозиторий инициализирован: {repo}"))?;
                }
                let bk = run_env("restic", &["backup", "--host", "cloudos", "--tag", kind, "--json", "--quiet", STAGE], &env).await?;
                let summary = bk.lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok().filter(|v| v["message_type"] == "summary")).unwrap_or_else(|| json!({}));
                let _ = run_env("restic", &["forget", "--keep-daily", &cfg.restic_keep_daily.to_string(), "--keep-weekly", &cfg.restic_keep_weekly.to_string(), "--keep-monthly", &cfg.restic_keep_monthly.to_string(), "--prune", "--quiet"], &env).await;
                let stats = run_env("restic", &["stats", "--json", "--mode", "raw-data"], &env).await.ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or_else(|| json!({}));
                let key = if repo == RESTIC_REPO { "local" } else { "remote" };
                out[key] = json!({ "repo": repo, "snapshot_id": summary["snapshot_id"], "files": summary["total_files_processed"], "bytes": summary["total_bytes_processed"], "added": summary["data_added"], "repo_size": stats["total_size"] });
                self.store.event("backup", "restic", &format!("копия в {repo}: {} файлов, {} данных, добавлено {}, репозиторий {}", summary["total_files_processed"], human(summary["total_bytes_processed"].as_u64().unwrap_or(0)), human(summary["data_added"].as_u64().unwrap_or(0)), human(stats["total_size"].as_u64().unwrap_or(0))))?;
            }
            Ok::<Value, anyhow::Error>(out)
        }
        .await;
        for (_, sub) in RESTIC_SETS { let _ = run("zfs", &["destroy", "-f", &format!("tank/backups/stage-{sub}")]).await; }
        result
    }

    pub async fn backup_status(&self) -> Result<BackupStatus> {
        let config = self.backup_config();
        let datasets = datasets().await.unwrap_or_default();
        let restic_installed = run("which", &["restic"]).await.is_ok();
        let (mut restic_snapshots, mut restic_size) = (0usize, 0u64);
        if restic_installed && Path::new(RESTIC_REPO).join("config").exists() {
            if let Ok(env) = self.restic_env(RESTIC_REPO) {
                if let Ok(s) = run_env("restic", &["snapshots", "--json"], &env).await { restic_snapshots = serde_json::from_str::<Vec<Value>>(&s).map(|v| v.len()).unwrap_or(0); }
                if let Ok(s) = run_env("restic", &["stats", "--json", "--mode", "raw-data"], &env).await { restic_size = serde_json::from_str::<Value>(&s).ok().and_then(|v| v["total_size"].as_u64()).unwrap_or(0); }
            }
        }
        Ok(BackupStatus { config, datasets, restic_installed, restic_repo: RESTIC_REPO.into(), restic_snapshots, restic_size, last: self.store.get_kv(KV_LAST)?.unwrap_or_else(|| json!({})), running: self.backup_running.load(std::sync::atomic::Ordering::Relaxed) })
    }

    fn apps_snapdir(&self) -> PathBuf { self.cfg.apps_data_dir.join(".zfs").join("snapshot") }

    /// Точки восстановления приложения: снимки tank/apps, где есть каталог приложения (новые сверху).
    pub async fn app_restore_points(&self, name: &str) -> Result<Vec<RestorePoint>> {
        self.manifest(name)?;
        let mut v: Vec<RestorePoint> = list_snapshots().await?.into_iter().filter(|s| s.dataset == "tank/apps").filter_map(|s| {
            let p = self.apps_snapdir().join(&s.name).join(name);
            p.is_dir().then(|| RestorePoint { snapshot: s.name.clone(), kind: s.kind.clone(), created: s.created, path: p.display().to_string() })
        }).collect();
        v.sort_by(|a, b| b.created.cmp(&a.created));
        Ok(v)
    }

    /// Откат данных приложения к снимку: снимок текущего состояния (pre-restore), стек останавливается, каталог приложения
    /// заменяется копией из снимка (rsync --delete), стек поднимается заново.
    pub async fn app_restore(&self, name: &str, snap: &str) -> Result<crate::model::AppView> {
        let _g = self.lock.lock().await;
        let Some(_st) = self.store.get_app(name)? else { bail!("приложение {name} не установлено") };
        if snap.contains('/') || snap.contains("..") || !snap.starts_with("cloudos-") { bail!("неверное имя снимка"); }
        let src = self.apps_snapdir().join(snap).join(name);
        if !src.is_dir() { bail!("в снимке {snap} нет данных приложения {name}"); }
        let live = self.cfg.apps_data_dir.join(name);
        let pre = snapshot_of(&["tank/apps"], &format!("pre-restore-{name}")).await?;
        self.store.event("app", name, &format!("восстановление из снимка {snap}; текущее состояние сохранено в {pre}"))?;
        // DestroyStack в Komodo асинхронный: ждём, пока контейнеры стека исчезнут, и только потом трогаем файлы
        self.komodo.destroy(name).await.context("остановка стека")?;
        for _ in 0..60 {
            if self.docker.stack_containers(name).await.map(|c| c.is_empty()).unwrap_or(false) { break; }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        run("rsync", &["-a", "--delete", &format!("{}/", src.display()), &format!("{}/", live.display())]).await?;
        // деплой сразу после destroy может упереться в «Stack busy» — повторяем
        let mut deployed = Ok(());
        for i in 0..10 {
            deployed = self.komodo.deploy(name).await;
            match &deployed { Ok(()) => break, Err(e) if format!("{e:#}").to_lowercase().contains("busy") && i < 9 => tokio::time::sleep(std::time::Duration::from_secs(3)).await, Err(_) => break }
        }
        deployed.context("запуск стека после восстановления")?;
        self.spawn_watch(name);
        self.komodo.wait_running(name, 240).await.context("стек не поднялся после восстановления")?;
        self.store.event("app", name, &format!("данные восстановлены из снимка {snap}, стек перезапущен"))?;
        drop(_g);
        self.app(name).await
    }
}
