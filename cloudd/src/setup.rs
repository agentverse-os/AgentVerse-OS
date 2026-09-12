//! Мастер первого запуска (docs/first-run.md): состояние Tailscale на хосте, адрес Entry Point и его сертификат,
//! переключение ядра на MagicDNS-имя узла, отметка «настройка завершена». Источник правды о Tailscale — CLI
//! `tailscale status --json` (cloudd работает от root, поэтому и вход `tailscale login` запускает сам).

use crate::service::Service;
use crate::store;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use utoipa::ToSchema;

pub const DONE_KEY: &str = "setup.done";
/// Админка tailnet: здесь включаются MagicDNS и HTTPS Certificates.
pub const TAILSCALE_ADMIN_DNS: &str = "https://login.tailscale.com/admin/dns";

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct TailscaleInfo {
    /// Есть ли бинарь tailscale на хосте.
    pub installed: bool,
    /// Running | NeedsLogin | NeedsMachineAuth | Stopped | Starting | NoState | Missing (не установлен) | Error
    pub state: String,
    /// Ссылка для входа, пока узел ждёт авторизации.
    pub auth_url: Option<String>,
    /// MagicDNS-имя узла без точки на конце: `aios.tail0fe52f.ts.net`.
    pub dns_name: Option<String>,
    pub ips: Vec<String>,
    /// Суффикс tailnet: `tail0fe52f.ts.net`.
    pub tailnet: Option<String>,
    pub magic_dns: bool,
    /// В tailnet включены HTTPS Certificates (у узла есть домены для сертификатов).
    pub https_certs: bool,
    pub cert_domains: Vec<String>,
    pub health: Vec<String>,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct EdgeInfo {
    pub host: String,
    pub alt_hosts: Vec<String>,
    /// Сертификат от внутреннего CA edge (нужно доверять cloudos-ca.crt) вместо Let's Encrypt через tailscaled.
    pub tls_internal: bool,
    /// Адрес ядра совпадает с MagicDNS-именем узла Tailscale.
    pub matches_tailscale: bool,
    /// Проверка с хоста: `https://<host>/api/status`.
    pub reachable: bool,
    /// null — неизвестно (до имени не дошли).
    pub dns_ok: Option<bool>,
    pub cert_ok: Option<bool>,
    pub error: Option<String>,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SetupState {
    pub done: bool,
    pub tailscale: TailscaleInfo,
    pub edge: EdgeInfo,
    pub projects: usize,
    pub apps_installed: usize,
    pub backups_enabled: bool,
    pub coder_ok: bool,
    /// Имя, на которое стоит переключить ядро (edge_host ≠ MagicDNS-имя узла).
    pub suggested_host: Option<String>,
    pub tailscale_admin_url: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SwitchResult {
    pub host: String,
    pub url: String,
    pub alt_hosts: Vec<String>,
    pub restart_in_s: u32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct HostBody { pub host: String }
#[derive(Debug, Deserialize, ToSchema)]
pub struct DoneBody { pub done: bool }

/// Разбор `tailscale status --json` (только нужные поля; неизвестное состояние → NoState).
pub fn parse_status(json: &str) -> TailscaleInfo {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => return TailscaleInfo { installed: true, state: "Error".into(), error: Some(format!("status --json: {e}")), ..Default::default() },
    };
    let s = |p: &str| v.get(p).and_then(|x| x.as_str()).map(|x| x.to_string()).filter(|x| !x.is_empty());
    let strings = |x: Option<&serde_json::Value>| x.and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|i| i.as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default();
    let me = v.get("Self");
    let dns_name = me.and_then(|x| x.get("DNSName")).and_then(|x| x.as_str()).map(|d| d.trim_end_matches('.').to_string()).filter(|d| !d.is_empty());
    let cert_domains = strings(v.get("CertDomains"));
    let tailnet = s("MagicDNSSuffix").or_else(|| v.get("CurrentTailnet").and_then(|t| t.get("MagicDNSSuffix")).and_then(|x| x.as_str()).map(String::from));
    TailscaleInfo {
        installed: true,
        state: s("BackendState").unwrap_or_else(|| "NoState".into()),
        auth_url: s("AuthURL"),
        dns_name,
        ips: strings(me.and_then(|x| x.get("TailscaleIPs"))),
        tailnet,
        magic_dns: v.get("CurrentTailnet").and_then(|t| t.get("MagicDNSEnabled")).and_then(|x| x.as_bool()).unwrap_or(false),
        https_certs: !cert_domains.is_empty(),
        cert_domains,
        health: strings(v.get("Health")),
        version: s("Version").map(|x| x.split('-').next().unwrap_or(&x).to_string()),
        error: None,
    }
}

/// Внешняя команда с ограничением по времени.
pub(crate) async fn run(cmd: &str, args: &[&str], timeout: Duration) -> Result<std::process::Output> {
    let fut = tokio::process::Command::new(cmd).args(args).stdin(std::process::Stdio::null()).kill_on_drop(true).output();
    tokio::time::timeout(timeout, fut).await.map_err(|_| anyhow::anyhow!("{cmd} {}: не ответил за {} с", args.join(" "), timeout.as_secs()))?.with_context(|| format!("{cmd} {}", args.join(" ")))
}

pub async fn tailscale_status() -> TailscaleInfo {
    match run("tailscale", &["status", "--json"], Duration::from_secs(8)).await {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.trim_start().starts_with('{') {
                parse_status(&stdout)
            } else {
                TailscaleInfo { installed: true, state: "Error".into(), error: Some(format!("{} {}", stdout.trim(), String::from_utf8_lossy(&out.stderr).trim()).trim().to_string()), ..Default::default() }
            }
        }
        Err(e) => {
            let msg = format!("{e:#}");
            if msg.contains("os error 2") {
                TailscaleInfo { installed: false, state: "Missing".into(), ..Default::default() }
            } else {
                TailscaleInfo { installed: true, state: "Error".into(), error: Some(msg), ..Default::default() }
            }
        }
    }
}

/// `KEY=value` в env-файле: заменить первую строку с ключом или дописать в конец; комментарии и порядок сохраняются.
pub fn env_file_set(text: &str, pairs: &[(&str, &str)]) -> String {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    for (k, v) in pairs {
        let line = format!("{k}={v}");
        match lines.iter().position(|l| l.trim_start().starts_with(&format!("{k}="))) {
            Some(i) => lines[i] = line,
            None => lines.push(line),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn write_env_file(path: &Path, text: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = path.with_extension("env.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("запись {}", tmp.display()))?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, path).with_context(|| format!("замена {}", path.display()))?;
    Ok(())
}

/// Текст ошибки вместе с цепочкой причин: у reqwest верхнее сообщение общее («error sending request»), суть — в source().
pub(crate) fn err_chain(e: &dyn std::error::Error) -> String {
    let mut parts = vec![e.to_string()];
    let mut cur = e.source();
    while let Some(c) = cur { parts.push(c.to_string()); cur = c.source(); }
    parts.dedup();
    parts.join(": ")
}

fn classify(err: &str) -> (Option<bool>, Option<bool>) {
    let m = err.to_lowercase();
    if m.contains("dns error") || m.contains("failed to lookup") || m.contains("name or service not known") || m.contains("no address") || m.contains("nodename nor servname") {
        return (Some(false), None);
    }
    if m.contains("certificate") || m.contains("unknownissuer") || m.contains("invalid peer") || m.contains("handshake") || m.contains("tls") {
        return (Some(true), Some(false));
    }
    if m.contains("connection refused") || m.contains("timed out") || m.contains("connect error") {
        return (Some(true), None);   // имя разрешилось, но edge не отвечает
    }
    (None, None)
}

impl Service {
    pub fn setup_done(&self) -> bool {
        self.store.get_kv(DONE_KEY).ok().flatten().and_then(|v| v.as_bool()).unwrap_or(false)
    }

    /// Проверка адреса ядра с хоста: имя резолвится, TLS проходит (для внутреннего CA — с корнем из state_dir/ca.crt).
    pub async fn edge_check(&self, ts: &TailscaleInfo) -> EdgeInfo {
        let host = self.cfg.edge_host.clone();
        let mut b = reqwest::Client::builder().timeout(Duration::from_secs(7)).connect_timeout(Duration::from_secs(4));
        if self.cfg.tls_internal {
            if let Ok(pem) = std::fs::read(self.cfg.state_dir.join("ca.crt")) {
                if let Ok(c) = reqwest::Certificate::from_pem(&pem) { b = b.add_root_certificate(c); }
            }
        }
        let mut info = EdgeInfo { host: host.clone(), alt_hosts: self.edge_alt_hosts(), tls_internal: self.cfg.tls_internal, matches_tailscale: ts.dns_name.as_deref() == Some(host.as_str()), checked_at: store::now(), ..Default::default() };
        match b.build() {
            Ok(c) => match c.get(format!("https://{host}/api/status")).send().await {
                Ok(_) => { info.reachable = true; info.dns_ok = Some(true); info.cert_ok = Some(true); }
                Err(e) => {
                    let msg = err_chain(&e);
                    let (dns, cert) = classify(&msg);
                    info.dns_ok = dns; info.cert_ok = cert; info.error = Some(msg);
                }
            },
            Err(e) => info.error = Some(e.to_string()),
        }
        info
    }

    pub async fn setup_state(&self) -> Result<SetupState> {
        let ts = tailscale_status().await;
        let edge = self.edge_check(&ts).await;
        let suggested_host = match (&ts.dns_name, ts.state.as_str()) {
            (Some(d), "Running") if d != &self.cfg.edge_host => Some(d.clone()),
            _ => None,
        };
        Ok(SetupState {
            done: self.setup_done(),
            projects: self.store.list_projects()?.len(),
            apps_installed: self.store.list_apps()?.len(),
            backups_enabled: self.backup_config().enabled,
            coder_ok: self.coder.health().await.is_ok(),
            suggested_host,
            tailscale: ts,
            edge,
            tailscale_admin_url: TAILSCALE_ADMIN_DNS.into(),
            version: crate::updates::VERSION.into(),
        })
    }

    pub fn setup_set_done(&self, done: bool) -> Result<()> {
        self.store.put_kv(DONE_KEY, &serde_json::json!(done))?;
        self.store.event("setup", "wizard", if done { "первый запуск завершён" } else { "мастер первого запуска открыт заново" })?;
        Ok(())
    }

    /// Войти в Tailscale: Stopped → `tailscale up`; NeedsLogin → `tailscale login` в фоне, ссылка — из status.
    pub async fn tailscale_login(&self) -> Result<TailscaleInfo> {
        let ts = tailscale_status().await;
        match ts.state.as_str() {
            "Missing" => bail!("tailscale не установлен на хосте: sudo bootstrap/install.sh tailscale"),
            "Running" => return Ok(ts),
            "Stopped" => {
                let out = run("tailscale", &["up", "--timeout", "30s"], Duration::from_secs(40)).await?;
                if !out.status.success() { bail!("tailscale up: {}", String::from_utf8_lossy(&out.stderr).trim()); }
                self.store.event("setup", "tailscale", "узел включён (tailscale up)")?;
                return Ok(tailscale_status().await);
            }
            _ => {}
        }
        if ts.auth_url.is_none() {
            // интерактивный вход ждёт подтверждения в браузере; сама команда живёт до 30 минут, ссылку читаем из status
            std::fs::create_dir_all(&self.cfg.state_dir)?;
            let log = std::fs::File::create(self.cfg.state_dir.join("tailscale-login.log"))?;
            let err = log.try_clone()?;
            tokio::process::Command::new("tailscale").args(["login", "--timeout", "30m"]).stdin(std::process::Stdio::null()).stdout(log).stderr(err).spawn().context("tailscale login")?;
            self.store.event("setup", "tailscale", "запрошена ссылка для входа в Tailscale")?;
        }
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let t = tailscale_status().await;
            if t.auth_url.is_some() || t.state == "Running" { return Ok(t); }
        }
        Ok(tailscale_status().await)
    }

    /// Переключить ядро на MagicDNS-имя узла: cloudd.env (EDGE_HOST, TLS_INTERNAL=false, прежний адрес и IP Tailscale — в ALT_HOSTS)
    /// и перезапуск cloudd; edge получит конфигурацию с новым именем и возьмёт сертификат у tailscaled.
    pub async fn setup_switch_host(&self, host: &str) -> Result<SwitchResult> {
        let ts = tailscale_status().await;
        let host = host.trim().trim_end_matches('.').to_lowercase();
        let Some(dns) = ts.dns_name.clone() else { bail!("Tailscale ещё не выдал имя узла: авторизуйте узел и повторите") };
        if host != dns { bail!("имя {host} не совпадает с MagicDNS-именем узла {dns}") }
        if !ts.https_certs { bail!("в tailnet выключены HTTPS Certificates: включите их в админке Tailscale (DNS → HTTPS Certificates) и повторите") }
        let path = crate::config::env_file_path();
        let text = std::fs::read_to_string(&path).with_context(|| format!("чтение {}", path.display()))?;
        let old = self.cfg.edge_host.clone();
        let mut alt: Vec<String> = self.cfg.edge_alt_hosts.clone();
        if old != host && !alt.contains(&old) { alt.push(old.clone()); }
        for ip in ts.ips.iter().filter(|i| i.contains('.')) { if !alt.contains(ip) { alt.push(ip.clone()); } }
        let alt_s = alt.join(",");
        let new_text = env_file_set(&text, &[("CLOUDD_EDGE_HOST", host.as_str()), ("CLOUDD_TLS_INTERNAL", "false"), ("CLOUDD_EDGE_ALT_HOSTS", alt_s.as_str())]);
        write_env_file(&path, &new_text)?;
        self.store.event("setup", "edge", &format!("адрес ядра {old} → {host}, запасные входы {alt_s}; перезапуск cloudd"))?;
        self.schedule_restart(false)?;
        Ok(SwitchResult { url: format!("https://{host}/"), host, alt_hosts: alt, restart_in_s: 3 })
    }
}
