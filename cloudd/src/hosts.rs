//! Owner's servers (ADR-008): cloudos-agentd outside the workspaces, reached over the tailnet. The record lives in the
//! store (`hosts`), the token in the secrets (`host:<name>` / `agentd_token`), the one-time install code in kv
//! `hosts.code.<code>`. Everything the "Agents" service shows goes through the same agentd client as for a workspace:
//! `Service::agentd_client` looks the servers up first, `Service::projects` mixes them into the project list as
//! `ProjectView { kind: "host" }`.
use crate::agentd::{self, AgentdClient, TOKEN_KEY};
use crate::model::{AppKind, Grant, HostState, HostView, ProjectSpec, ProjectView, WorkspaceView};
use crate::service::{env_prefix, parse_env, Service};
use crate::store;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

/// A one-time install code lives this long.
const CODE_TTL_SECS: i64 = 3600;
/// Probe period of the servers.
pub const PROBE_SECS: u64 = 30;

pub fn secret_scope(name: &str) -> String { format!("host:{name}") }
/// Runs on the server by the owner after removal: both unit kinds, then the files.
pub const UNINSTALL_SH: &str = "sudo systemctl disable --now cloudos-agentd 2>/dev/null; systemctl --user disable --now cloudos-agentd 2>/dev/null; sudo rm -f /etc/systemd/system/cloudos-agentd.service 2>/dev/null; rm -rf ~/.cloudos ~/.config/systemd/user/cloudos-agentd.service";
fn code_key(code: &str) -> String { format!("hosts.code.{code}") }

fn random_hex(n: usize) -> String {
    use rand::RngCore;
    let mut b = vec![0u8; n];
    rand::rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// FNV-1a fingerprint — the same as agentd's `/health` `build`, so the core can tell a server that is behind.
pub fn fnv1a(b: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for x in b { h ^= *x as u64; h = h.wrapping_mul(0x100000001b3); }
    format!("{h:016x}")
}

/// Fingerprint of the agentd this core gives to servers (the static build; computed once).
pub fn embedded_fingerprint() -> &'static str {
    static FP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FP.get_or_init(|| agentd::host_binary().map(|b| fnv1a(&b)).unwrap_or_default())
}

fn valid_token(t: &str) -> bool { (32..=128).contains(&t.len()) && t.chars().all(|c| c.is_ascii_alphanumeric()) }

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !name.starts_with('-') && !name.ends_with('-')
}

#[derive(serde::Deserialize, Default)]
pub struct RegisterBody {
    pub code: String,
    pub addr: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub home: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub version: String,
    /// The agentd token the installer generated on the server (ADR-010): the core stores it instead of handing its own out
    /// with the script. Empty — a script issued by an older core, which carried the stored token and sends it as the bearer.
    #[serde(default)]
    pub token: String,
}

impl Service {
    fn host_token(&self, name: &str) -> Result<String> {
        self.store.get_secret(&secret_scope(name), TOKEN_KEY)?.ok_or_else(|| anyhow!("у сервера {name} нет токена"))
    }

    /// agentd client of a registered server (any status: the caller decides whether to try an offline one).
    pub fn host_client(&self, h: &HostState) -> Result<AgentdClient> {
        if h.addr.is_empty() { bail!("сервер {} ещё не зарегистрирован — выполните команду установки", h.name); }
        Ok(AgentdClient::with_addr(&h.dial(), self.host_token(&h.name)?))
    }

    pub fn host_view(&self, h: &HostState, install: Option<String>) -> HostView {
        let llm = self.store.list_grants(Some(&h.name)).unwrap_or_default().into_iter().find(|g| g.capability == "llm").map(|g| g.app);
        HostView { state: h.clone(), core_agentd_version: embedded_fingerprint().to_string(), llm, install, uninstall: UNINSTALL_SH.into() }
    }

    pub fn hosts(&self) -> Result<Vec<HostView>> { Ok(self.store.list_hosts()?.iter().map(|h| self.host_view(h, None)).collect()) }

    pub fn host(&self, name: &str) -> Result<HostView> {
        let h = self.store.get_host(name)?.ok_or_else(|| anyhow!("сервер {name} не найден"))?;
        Ok(self.host_view(&h, None))
    }

    /// New server: the record, a token and a one-time install code; the answer carries the install command.
    pub fn host_add(&self, name: &str, note: &str) -> Result<HostView> {
        let name = name.trim().to_ascii_lowercase();
        if !valid_name(&name) { bail!("имя сервера: латиница в нижнем регистре, цифры и дефис, до 40 знаков"); }
        if self.store.get_host(&name)?.is_some() { bail!("сервер {name} уже есть"); }
        if self.store.get_project(&name)?.is_some() { bail!("имя {name} занято проектом"); }
        let h = HostState { name: name.clone(), note: note.trim().chars().take(500).collect(), status: "new".into(), created_at: store::now(), ..Default::default() };
        self.store.put_host(&h)?;
        self.store.put_secret(&secret_scope(&name), TOKEN_KEY, &random_hex(24))?;
        self.store.event("host", &name, "сервер добавлен — ждёт установки agentd")?;
        let install = self.host_issue_code(&name)?;
        Ok(self.host_view(&h, Some(install)))
    }

    /// (Re)issue the install code: the previous codes of the server stop working.
    pub fn host_issue_code(&self, name: &str) -> Result<String> {
        let h = self.store.get_host(name)?.ok_or_else(|| anyhow!("сервер {name} не найден"))?;
        for (k, v) in self.store.kv_prefix("hosts.code.")? {
            if v.get("name").and_then(|n| n.as_str()) == Some(&h.name) { let _ = self.store.delete_kv(&k); }
        }
        let code = random_hex(16);
        let exp = time::OffsetDateTime::now_utc().unix_timestamp() + CODE_TTL_SECS;
        self.store.put_kv(&code_key(&code), &json!({ "name": h.name, "exp": exp }))?;
        Ok(self.host_install_command(&code))
    }

    pub fn host_install_command(&self, code: &str) -> String { format!("curl -fsSL {}/api/hosts/install/{code} | bash", self.public_base()) }

    fn public_base(&self) -> String { format!("https://{}", self.cfg.edge_host) }

    /// The server behind a valid code, or why not.
    pub fn host_by_code(&self, code: &str) -> Result<HostState> {
        if code.len() != 32 || !code.chars().all(|c| c.is_ascii_hexdigit()) { bail!("код установки не распознан"); }
        let v = self.store.get_kv(&code_key(code))?.ok_or_else(|| anyhow!("код установки не найден или уже использован — выпустите новый в Desktop → Система → Серверы"))?;
        let exp = v.get("exp").and_then(|e| e.as_i64()).unwrap_or(0);
        if time::OffsetDateTime::now_utc().unix_timestamp() > exp { let _ = self.store.delete_kv(&code_key(code)); bail!("код установки истёк — выпустите новый"); }
        let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("");
        self.store.get_host(name)?.ok_or_else(|| anyhow!("сервер {name} уже удалён"))
    }

    /// The install script for a code: bash, runs on the server under an ordinary user. It carries no secret of the server:
    /// the agentd token is generated there and comes back with the registration (ADR-010) — whoever can read the script
    /// by its code learns nothing about a server that is already installed.
    pub fn host_install_script(&self, code: &str) -> Result<String> {
        let h = self.host_by_code(code)?;
        Ok(INSTALL_SH.replace("__CORE__", &self.public_base()).replace("__CODE__", code).replace("__NAME__", &h.name))
    }

    /// The installer reports where agentd listens and with which token; the code is spent, the env is rendered, the first
    /// probe runs. The one-time code is what proves the right to set the token.
    pub async fn host_register(&self, bearer: &str, b: RegisterBody) -> Result<HostView> {
        let mut h = self.host_by_code(&b.code)?;
        let token = b.token.trim();
        if !token.is_empty() {
            if !valid_token(token) { bail!("токен сервера: от 32 до 128 латинских букв и цифр"); }
            self.store.put_secret(&secret_scope(&h.name), TOKEN_KEY, token)?;
        } else if bearer.is_empty() || self.host_token(&h.name)? != bearer {
            bail!("токен сервера не совпадает");
        }
        let addr = b.addr.trim().to_string();
        if addr.is_empty() || addr.contains(|c: char| c.is_whitespace() || c == '/') { bail!("адрес сервера не задан: нужен tailscale-адрес (tailscale ip -4) или ADDR="); }
        h.addr = addr;
        h.port = b.port.unwrap_or(7200);
        h.user = b.user.trim().chars().take(64).collect();
        h.home = b.home.trim().chars().take(256).collect();
        h.arch = b.arch.trim().chars().take(32).collect();
        h.os = b.os.trim().chars().take(120).collect();
        h.hostname = b.hostname.trim().chars().take(120).collect();
        h.agentd_version = b.version.trim().chars().take(64).collect();
        h.registered_at = Some(store::now());
        h.status = "registered".into();
        self.store.put_host(&h)?;
        let _ = self.store.delete_kv(&code_key(&b.code));
        self.store.event("host", &h.name, &format!("agentd установлен: {}@{} ({} {})", h.user, h.dial(), h.os, h.arch))?;
        // the llm capability through the external provider — the only one reachable from outside the stand
        if let Err(e) = self.host_grant_llm(&h.name).await { tracing::warn!("{}: llm для сервера: {e:#}", h.name); }
        let h = self.host_probe(&h).await;
        if h.online() { if let Err(e) = self.host_push_env(&h).await { tracing::warn!("{}: env.sh: {e:#}", h.name); } }
        Ok(self.host_view(&h, None))
    }

    /// One probe: `/health` → status, version and fingerprint; the record is written back.
    pub async fn host_probe(&self, h: &HostState) -> HostState {
        let mut h = h.clone();
        if h.addr.is_empty() || h.disabled { return h }
        let r = match self.host_client(&h) {
            Ok(c) => tokio::time::timeout(Duration::from_secs(8), c.health()).await.map_err(|_| anyhow!("нет ответа 8 с")).and_then(|r| r),
            Err(e) => Err(e),
        };
        match r {
            Ok(v) => {
                h.status = "online".into();
                h.last_seen = Some(store::now());
                h.last_error = None;
                if let Some(b) = v.get("build").and_then(|b| b.as_str()) { h.agentd_version = b.to_string(); }
                if let Some(x) = v.get("home").and_then(|x| x.as_str()) { if !x.is_empty() { h.home = x.to_string(); } }
                if let Some(x) = v.get("user").and_then(|x| x.as_str()) { if !x.is_empty() { h.user = x.to_string(); } }
                if let Some(x) = v.get("arch").and_then(|x| x.as_str()) { h.arch = x.to_string(); }
                if let Some(x) = v.get("os").and_then(|x| x.as_str()) { h.os = x.to_string(); }
                if let Some(x) = v.get("hostname").and_then(|x| x.as_str()) { h.hostname = x.to_string(); }
                if let Some(x) = v.get("env_file").and_then(|x| x.as_str()) { if !x.is_empty() { h.env_file = x.to_string(); } }
            }
            Err(e) => {
                if h.status == "online" { let _ = self.store.event("host", &h.name, &format!("сервер не отвечает: {e:#}")); }
                h.status = "offline".into();
                h.last_error = Some(format!("{e:#}").chars().take(200).collect());
            }
        }
        let _ = self.store.put_host(&h);
        h
    }

    pub async fn host_probe_all(&self) {
        for h in self.store.list_hosts().unwrap_or_default() {
            if h.addr.is_empty() || h.disabled { continue }
            let before = h.status.clone();
            let after = self.host_probe(&h).await;
            if before != "online" && after.online() {
                let _ = self.store.event("host", &h.name, &format!("сервер в сети ({})", h.dial()));
                if let Err(e) = self.host_push_env(&after).await { tracing::debug!("{}: env.sh: {e:#}", h.name); }
            }
        }
    }

    pub fn start_hosts_prober(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            loop {
                me.host_probe_all().await;
                tokio::time::sleep(Duration::from_secs(PROBE_SECS)).await;
            }
        });
    }

    /// The llm capability for a server: through `llm-external` only (the local providers sit on the stand's project
    /// networks), without a gate — the hook runs with GATE_URL = the provider's own address, its output becomes the
    /// server's secrets like a project's.
    pub async fn host_grant_llm(&self, name: &str) -> Result<Option<String>> {
        let apps = self.store.list_apps()?;
        let Some(app) = apps.iter().find(|a| a.manifest.kind == AppKind::External && a.manifest.provides.iter().any(|c| c == "llm")) else { return Ok(None) };
        let (url, _key) = self.external_target(app)?;
        // the owner's override: the same proxy reached from the server by another address
        let url = match self.store.get_host(name)?.and_then(|h| h.llm_url).filter(|u| !u.trim().is_empty()) { Some(u) => u.trim().trim_end_matches('/').to_string(), None => url };
        let scope = format!("project:{name}:llm");
        if let Some(hook) = &app.manifest.hooks.on_grant {
            let out = self.run_hook_for(app, hook, name, "llm", &url).await.with_context(|| format!("on_grant {} для сервера {name}", app.name))?;
            for (k, v) in parse_env(&out) { self.store.put_secret(&scope, &k, &v)?; }
        }
        self.store.put_secret(&scope, &format!("{}_URL", env_prefix("llm")), &url)?;
        self.store.put_grant(&Grant { project: name.to_string(), capability: "llm".into(), app: app.name.clone(), granted_at: store::now() })?;
        self.store.event("grant", name, &format!("llm → {} напрямую ({url}), без gate", app.name))?;
        Ok(Some(app.name.clone()))
    }

    /// env.sh of a server — the same rendering as for a workspace, delivered through agentd's file API into the file agentd
    /// reads (`--env-file`, reported by /health; `~/.cloudos/env.sh` by default).
    pub async fn host_push_env(&self, h: &HostState) -> Result<bool> {
        let client = self.host_client(h)?;
        let mut lines = vec!["# AgentVerse OS: env capabilities сервера — рендерит ядро из грантов, правки перезапишутся".to_string()];
        // a marker for scripts and agents on the server (and for the e2e proof that agentd injects this file into processes)
        lines.push(format!("export AGENTVERSE_HOST={}", crate::service::shell_quote(&h.name)));
        for g in self.store.list_grants(Some(&h.name)).unwrap_or_default() {
            let scope = format!("project:{}:{}", h.name, g.capability);
            lines.push(format!("# {} → {}", g.capability, g.app));
            for k in self.store.list_secret_keys(&scope).unwrap_or_default() {
                if let Ok(Some(v)) = self.store.get_secret(&scope, &k) { lines.push(format!("export {k}={}", crate::service::shell_quote(&v))); }
            }
        }
        let content = lines.join("\n") + "\n";
        let path = if h.env_file.is_empty() { "~/.cloudos/env.sh".to_string() } else { h.env_file.clone() };
        let path = path.as_str();
        let have = client.raw_get(&format!("/v1/fs/file?path={}", urlencoding::encode(path))).await.ok().map(|(_, _, b)| String::from_utf8_lossy(&b).to_string());
        if have.as_deref() == Some(content.as_str()) { return Ok(false) }
        client.raw_put(&format!("/v1/fs/file?path={}", urlencoding::encode(path)), content.into()).await.context("env.sh на сервере")?;
        Ok(true)
    }

    /// Push the agentd embedded in this core to the server (`POST /v1/update`); agentd replaces itself and restarts.
    pub async fn host_update_agentd(&self, name: &str) -> Result<Value> {
        let h = self.store.get_host(name)?.ok_or_else(|| anyhow!("сервер {name} не найден"))?;
        let Some(bin) = agentd::host_binary() else { bail!("cloudos-agentd не встроен в ядро") };
        if !h.arch.is_empty() && h.arch != std::env::consts::ARCH { bail!("сервер {name}: {} — у ядра есть сборка agentd только под {}", h.arch, std::env::consts::ARCH); }
        let client = self.host_client(&h)?;
        let r = client.raw_post_octets("/v1/update", bin.to_vec().into()).await.context("обновление agentd на сервере")?;
        self.store.event("host", name, &format!("agentd обновлён ({} байт)", bin.len()))?;
        Ok(r)
    }

    /// Owner's edits: the note and the LLM address override; the llm grant is re-rendered and env.sh re-pushed.
    pub async fn host_edit(&self, name: &str, note: Option<String>, llm_url: Option<String>, enabled: Option<bool>) -> Result<HostView> {
        if let Some(en) = enabled { let v = self.host_set_enabled(name, en).await?; if note.is_none() && llm_url.is_none() { return Ok(v) } }
        let mut h = self.store.get_host(name)?.ok_or_else(|| anyhow!("сервер {name} не найден"))?;
        if let Some(n) = note { h.note = n.trim().chars().take(500).collect(); }
        if let Some(u) = llm_url {
            let u = u.trim().to_string();
            if !u.is_empty() && !(u.starts_with("http://") || u.starts_with("https://")) { bail!("адрес LLM должен начинаться с http:// или https://"); }
            h.llm_url = (!u.is_empty()).then_some(u);
        }
        self.store.put_host(&h)?;
        if let Err(e) = self.host_grant_llm(&h.name).await { tracing::warn!("{}: llm для сервера: {e:#}", h.name); }
        if h.online() { if let Err(e) = self.host_push_env(&h).await { tracing::warn!("{}: env.sh: {e:#}", h.name); } }
        Ok(self.host_view(&h, None))
    }

    /// Ask agentd on the server to stop (`POST /v1/shutdown`, exit 75 — systemd does not restart it). Best effort: an
    /// unreachable server is simply left as is.
    async fn host_shutdown(&self, h: &HostState) -> Option<String> {
        if h.addr.is_empty() { return None }
        let client = self.host_client(h).ok()?;
        match tokio::time::timeout(Duration::from_secs(8), client.post("/v1/shutdown", &json!({}))).await {
            Ok(Ok(_)) => Some("agentd на сервере остановлен".into()),
            Ok(Err(e)) => Some(format!("agentd на сервере не остановлен: {e:#}")),
            Err(_) => Some("agentd на сервере не ответил за 8 с".into()),
        }
    }

    /// The switch in «Servers»: off — agentd is told to stop, the record stays, the core neither probes nor uses the server;
    /// on — the record is live again and probed at once (agentd must be started on the server by the owner).
    pub async fn host_set_enabled(&self, name: &str, enabled: bool) -> Result<HostView> {
        let mut h = self.store.get_host(name)?.ok_or_else(|| anyhow!("сервер {name} не найден"))?;
        if enabled == !h.disabled { return Ok(self.host_view(&h, None)) }
        if !enabled {
            let note = self.host_shutdown(&h).await.unwrap_or_else(|| "agentd не был установлен".into());
            h.disabled = true;
            h.status = "disabled".into();
            h.last_error = None;
            self.agents.cache.write().unwrap().remove(&h.name);
            self.store.put_host(&h)?;
            self.store.event("host", &h.name, &format!("сервер отключён: {note}"))?;
            return Ok(self.host_view(&h, None));
        }
        h.disabled = false;
        h.status = "offline".into();
        self.store.put_host(&h)?;
        let h = self.host_probe(&h).await;
        if h.online() { if let Err(e) = self.host_push_env(&h).await { tracing::warn!("{}: env.sh: {e:#}", h.name); } }
        self.store.event("host", &h.name, &format!("сервер включён: {}", if h.online() { "в сети" } else { "agentd на сервере не отвечает — запустите его или выполните команду установки заново" }))?;
        Ok(self.host_view(&h, None))
    }

    pub async fn host_remove(&self, name: &str) -> Result<()> {
        let h = self.store.get_host(name)?.ok_or_else(|| anyhow!("сервер {name} не найден"))?;
        // stop agentd on the server first (its token stays there, but the core forgets it)
        let note = if h.disabled { None } else { self.host_shutdown(&h).await };
        for (k, v) in self.store.kv_prefix("hosts.code.")? {
            if v.get("name").and_then(|n| n.as_str()) == Some(name) { let _ = self.store.delete_kv(&k); }
        }
        self.agents.cache.write().unwrap().remove(name);
        self.store.delete_host(name)?;
        self.store.event("host", name, &format!("сервер убран из ядра ({}; файлы на сервере остаются — см. команду очистки)", note.unwrap_or_else(|| "agentd уже был остановлен".into())))?;
        Ok(())
    }

    /// A server as an entry of the project list (ADR-008): the same ProjectView, `kind: host`, the workspace block carries the
    /// address and `running` | `stopped` by the last probe.
    pub fn host_project_view(&self, h: &HostState) -> ProjectView {
        let grants = self.store.list_grants(Some(&h.name)).unwrap_or_default();
        let mut grant_env = std::collections::BTreeMap::new();
        for g in &grants { grant_env.insert(g.capability.clone(), self.store.list_secret_keys(&format!("project:{}:{}", h.name, g.capability)).unwrap_or_default()); }
        let status = if h.online() { "running" } else if h.addr.is_empty() { "pending" } else { "stopped" };
        let agent_status = if h.disabled { "disabled".to_string() } else { h.status.clone() };
        ProjectView {
            name: h.name.clone(),
            kind: "host".into(),
            spec: serde_json::from_value::<ProjectSpec>(json!({ "project": h.name, "description": (!h.note.is_empty()).then(|| h.note.clone()) })).unwrap_or_else(|_| serde_json::from_value(json!({ "project": h.name })).expect("project spec")),
            network: String::new(),
            subnet: String::new(),
            gate: crate::model::ComponentHealth { name: String::new(), status: "none".into(), detail: None },
            workspace: Some(WorkspaceView { id: uuid::Uuid::nil(), name: h.name.clone(), status: status.into(), agent_status: Some(agent_status), instance: None, ip: (!h.addr.is_empty()).then(|| h.dial()), apps: vec![], url: String::new(), memory: None }),
            grants,
            missing_capabilities: vec![],
            grant_env,
            available_capabilities: vec![],
            providers: Default::default(),
            agents: self.agents.cache.read().unwrap().get(&h.name).cloned(),
        }
    }
}

/// The install script (ADR-008 §3). Placeholders: __CORE__, __CODE__, __NAME__. Variables: PORT (7200),
/// ADDR (instead of the tailscale address), NO_SYSTEMD=1 (nohup, for checks), DIR (~/.cloudos). The agentd token is made on
/// the server and sent with the registration.
const INSTALL_SH: &str = r#"#!/usr/bin/env bash
# AgentVerse OS: install cloudos-agentd on this server and register it in the core (ADR-008).
set -euo pipefail
CORE="__CORE__"; CODE="__CODE__"; NAME="__NAME__"
# the token agentd will ask of the core: made here, sent with the registration, never served by the core
TOKEN="$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')"
[ "${#TOKEN}" -eq 48 ] || { echo "не удалось получить случайный токен из /dev/urandom" >&2; exit 1; }
PORT="${PORT:-7200}"; DIR="${DIR:-$HOME/.cloudos}"; USER="${USER:-$(id -un)}"; HOSTNAME_="$(hostname 2>/dev/null || cat /etc/hostname 2>/dev/null || echo unknown)"
if [ "$(id -u)" = "0" ]; then echo "не запускайте установку от root: нужен обычный пользователь (рекомендуется отдельный, без sudo)" >&2; exit 1; fi
command -v curl >/dev/null || { echo "нужен curl" >&2; exit 1; }
ADDR="${ADDR:-}"
if [ -z "$ADDR" ] && command -v tailscale >/dev/null 2>&1; then ADDR="$(tailscale ip -4 2>/dev/null | head -1 || true)"; fi
if [ -z "$ADDR" ]; then echo "tailscale-адрес не найден: подключите сервер к tailnet (tailscale up) или задайте ADDR=<ip> перед командой" >&2; exit 1; fi
ARCH="$(uname -m)"
mkdir -p "$DIR/bin" "$DIR/agentd"; chmod 700 "$DIR"
echo "скачиваю cloudos-agentd ($ARCH)…"
curl -fsSL "$CORE/api/hosts/install/$CODE/agentd?arch=$ARCH" -o "$DIR/bin/cloudos-agentd.new"
chmod 0755 "$DIR/bin/cloudos-agentd.new"; mv -f "$DIR/bin/cloudos-agentd.new" "$DIR/bin/cloudos-agentd"
umask 077; printf '%s\n' "$TOKEN" > "$DIR/agentd.token"; [ -f "$DIR/env.sh" ] || : > "$DIR/env.sh"
UNIT_BODY="[Unit]
Description=AgentVerse OS server agent (cloudos-agentd, ADR-008)
After=network-online.target
Wants=network-online.target

[Service]
Environment=HOME=$HOME
Environment=USER=$USER
Environment=SHELL=${SHELL:-/bin/bash}
Environment=RUST_LOG=info
EnvironmentFile=-$DIR/agentd.env
WorkingDirectory=$HOME
ExecStart=$DIR/bin/cloudos-agentd --listen $ADDR:$PORT --token-file $DIR/agentd.token --env-file $DIR/env.sh --data $DIR/agentd
Restart=always
RestartSec=2
RestartPreventExitStatus=75
"
started=""
if [ "${NO_SYSTEMD:-}" = "1" ]; then
  pkill -f "$DIR/bin/[c]loudos-agentd --listen" 2>/dev/null || true; sleep 0.5
  (HOME="$HOME" USER="$USER" RUST_LOG=info nohup "$DIR/bin/cloudos-agentd" --listen "$ADDR:$PORT" --token-file "$DIR/agentd.token" --env-file "$DIR/env.sh" --data "$DIR/agentd" >"$DIR/agentd.log" 2>&1 &)
  started="nohup"
elif command -v systemctl >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
  printf '%s[Install]\nWantedBy=multi-user.target\n' "$UNIT_BODY" | sed "s|^\[Service\]|[Service]\nUser=$USER\nGroup=$(id -gn)|" | sudo tee /etc/systemd/system/cloudos-agentd.service >/dev/null
  sudo systemctl daemon-reload; sudo systemctl enable cloudos-agentd >/dev/null 2>&1 || true; sudo systemctl restart cloudos-agentd
  started="system unit"
elif command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then
  mkdir -p "$HOME/.config/systemd/user"
  printf '%s[Install]\nWantedBy=default.target\n' "$UNIT_BODY" > "$HOME/.config/systemd/user/cloudos-agentd.service"
  systemctl --user daemon-reload; systemctl --user enable cloudos-agentd >/dev/null 2>&1 || true; systemctl --user restart cloudos-agentd
  loginctl enable-linger "$USER" 2>/dev/null || echo "не удалось включить linger: после выхода из сессии agentd остановится — выполните: sudo loginctl enable-linger $USER" >&2
  started="user unit"
else
  echo "systemd недоступен — запускаю без него (после перезагрузки запустите установку снова)" >&2
  (HOME="$HOME" USER="$USER" nohup "$DIR/bin/cloudos-agentd" --listen "$ADDR:$PORT" --token-file "$DIR/agentd.token" --env-file "$DIR/env.sh" --data "$DIR/agentd" >"$DIR/agentd.log" 2>&1 &)
  started="nohup"
fi
for i in $(seq 1 30); do curl -fs -m 2 "http://$ADDR:$PORT/health" >/dev/null 2>&1 && break; sleep 1; done
VERSION="$("$DIR/bin/cloudos-agentd" --version 2>/dev/null | awk '{print $NF}' || true)"
OS="$(. /etc/os-release 2>/dev/null && printf '%s' "${PRETTY_NAME:-}" || uname -s)"
body=$(printf '{"code":"%s","token":"%s","addr":"%s","port":%s,"user":"%s","home":"%s","arch":"%s","os":"%s","hostname":"%s","version":"%s"}' "$CODE" "$TOKEN" "$ADDR" "$PORT" "$USER" "$HOME" "$ARCH" "$OS" "$HOSTNAME_" "$VERSION")
if out=$(curl -fsS -m 30 -X POST "$CORE/api/hosts/register" -H "Content-Type: application/json" -d "$body"); then
  echo "сервер $NAME зарегистрирован ($started, $ADDR:$PORT): $(printf '%s' "$out" | tr -d '\n' | cut -c1-160)"
else
  echo "agentd запущен ($started), но регистрация в ядре не удалась — проверьте, что $CORE достижим с сервера, и повторите команду" >&2; exit 1
fi
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_dial_and_fingerprint() {
        assert!(valid_name("build-01") && valid_name("gpu") && !valid_name("Bad") && !valid_name("-x") && !valid_name("a b") && !valid_name(""));
        let h = HostState { name: "x".into(), addr: "100.64.1.2".into(), port: 7201, ..Default::default() };
        assert_eq!(h.dial(), "100.64.1.2:7201");
        let h6 = HostState { addr: "fd7a:115c::1".into(), port: 7200, ..Default::default() };
        assert_eq!(h6.dial(), "[fd7a:115c::1]:7200");
        assert_eq!(fnv1a(b""), "cbf29ce484222325");
        assert_eq!(fnv1a(b"a"), "af63dc4c8601ec8c");
        assert!(!HostState::default().online());
    }

    #[test]
    fn install_script_is_complete() {
        let s = INSTALL_SH.replace("__CORE__", "https://core.example").replace("__CODE__", "c").replace("__NAME__", "n");
        for needle in ["set -euo pipefail", "id -u", "tailscale ip -4", "/api/hosts/install/$CODE/agentd?arch=", "--env-file", "--token-file", "NO_SYSTEMD", "loginctl enable-linger", "RestartPreventExitStatus=75", "/api/hosts/register"] {
            assert!(s.contains(needle), "{needle}");
        }
        assert!(!s.contains("__"));
        // the token is made on the server and travels to the core in the registration body, under the one-time code; the
        // script the core serves carries no secret
        assert!(s.contains("TOKEN=\"$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \\n')\"") && s.contains("\"token\":\"%s\""), "{s}");
        assert!(!s.contains("Authorization: Bearer"));
        assert!(!INSTALL_SH.contains("__TOKEN__"));
    }

    #[test]
    fn server_token_shape() {
        assert!(valid_token(&"a1".repeat(24)));
        assert!(!valid_token("short") && !valid_token(&"x".repeat(129)) && !valid_token(&format!("{}\";rm -rf", "a".repeat(40))));
    }
}
