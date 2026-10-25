//! The "Agents" service (cloudos-agents) on the host (ADR-006, stage 2): a separate backend and web app on its own port
//! (7300) and the `/agents/` route behind the edge. The binary is embedded into cloudd (build.rs), and the core itself
//! installs it into `/usr/local/bin`, places the systemd unit and restarts it on change — the same scheme as agentd in
//! workspaces: one update package updates both the core and the service. Without systemd (development) — a quiet skip.
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

/// Install or update the service on the host. Returns what changed.
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
