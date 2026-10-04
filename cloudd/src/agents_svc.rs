//! Сервис «Агенты» (cloudos-agents) на хосте (ADR-006, этап 2): отдельный бэкенд и веб-приложение на своём порту (7300) и
//! маршруте `/agents/` за edge. Бинарь встроен в cloudd (build.rs), и ядро само ставит его в `/usr/local/bin`, кладёт юнит
//! systemd и перезапускает при изменении — та же схема, что с agentd в workspace'ах: один пакет обновления обновляет и ядро,
//! и сервис. Без systemd (разработка) — тихий пропуск.
use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

static BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cloudos-agents"));
pub static UNIT: &str = include_str!("../../bootstrap/systemd/cloudos-agents.service");
pub const BIN_PATH: &str = "/usr/local/bin/cloudos-agents";
pub const UNIT_PATH: &str = "/etc/systemd/system/cloudos-agents.service";
pub const SERVICE: &str = "cloudos-agents";

async fn sc(args: &[&str]) -> Result<std::process::Output> { crate::setup::run("systemctl", args, Duration::from_secs(60)).await }

/// Поставить или обновить сервис на хосте. Возвращает, что изменилось.
pub async fn ensure_installed() -> Result<Vec<&'static str>> {
    let mut changed = Vec::new();
    if BIN.is_empty() {
        tracing::warn!("cloudos-agents не встроен в cloudd — сервис «Агенты» не ставится");
        return Ok(changed);
    }
    if !Path::new("/run/systemd/system").exists() {
        tracing::debug!("systemd нет — установка cloudos-agents пропущена");
        return Ok(changed);
    }
    if std::fs::read(BIN_PATH).ok().as_deref() != Some(BIN) {
        let tmp = format!("{BIN_PATH}.new");
        std::fs::write(&tmp, BIN).with_context(|| format!("запись {tmp}"))?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, BIN_PATH).with_context(|| format!("замена {BIN_PATH}"))?;
        changed.push("binary");
    }
    if std::fs::read_to_string(UNIT_PATH).ok().as_deref() != Some(UNIT) {
        std::fs::write(UNIT_PATH, UNIT).with_context(|| format!("запись {UNIT_PATH}"))?;
        changed.push("unit");
    }
    if changed.contains(&"unit") { sc(&["daemon-reload"]).await.context("daemon-reload")?; }
    let active = sc(&["is-active", "--quiet", SERVICE]).await.map(|o| o.status.success()).unwrap_or(false);
    if !active {
        let out = sc(&["enable", "--now", SERVICE]).await.context("enable --now")?;
        if !out.status.success() { anyhow::bail!("systemctl enable --now {SERVICE}: {}", String::from_utf8_lossy(&out.stderr).trim()); }
        changed.push("started");
    } else if !changed.is_empty() {
        let out = sc(&["restart", SERVICE]).await.context("restart")?;
        if !out.status.success() { anyhow::bail!("systemctl restart {SERVICE}: {}", String::from_utf8_lossy(&out.stderr).trim()); }
        changed.push("restarted");
    }
    if !changed.is_empty() { tracing::info!("cloudos-agents: {}", changed.join(", ")); }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unit_runs_the_installed_binary() {
        assert!(UNIT.contains(&format!("ExecStart={BIN_PATH}")), "юнит должен запускать бинарь из BIN_PATH");
        assert!(UNIT.contains("After=network-online.target cloudd.service"), "сервис стартует после ядра");
    }
}
