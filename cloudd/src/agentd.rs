//! cloudos-agentd in the workspace (ADR-006): adapter deployment and the client to it. The binary is built from
//! `../agentd` and embedded into cloudd (build.rs), so a core release carries the adapter too; the unit comes
//! from `bootstrap/workspace/`. cloudd places them into the container the same way as `env.sh`, issues a token
//! from its secret store and talks to the adapter directly over the project network (`http://<workspace
//! ip>:7200`).
use crate::incus::Incus;
use crate::store::Store;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::time::Duration;

pub const PORT: u16 = 7200;
const BIN_PATH: &str = "/usr/local/bin/cloudos-agentd";
const UNIT_PATH: &str = "/etc/systemd/system/cloudos-agentd.service";
const TOKEN_PATH: &str = "/etc/cloudos/agentd.token";
pub const TOKEN_KEY: &str = "agentd_token";
static AGENTD_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cloudos-agentd"));
pub static AGENTD_UNIT: &str = include_str!("../../bootstrap/workspace/cloudos-agentd.service");

/// Adapter binary: `CLOUDD_AGENTD_BIN` (development) or the embedded one; `None` — agentd was not built when cloudd
/// was built.
pub fn binary() -> Option<std::borrow::Cow<'static, [u8]>> {
    if let Some(p) = std::env::var("CLOUDD_AGENTD_BIN").ok().filter(|p| !p.is_empty()) {
        return std::fs::read(&p).ok().map(Into::into);
    }
    (!AGENTD_BIN.is_empty()).then_some(AGENTD_BIN.into())
}

pub fn secret_scope(project: &str) -> String {
    format!("workspace:{project}")
}

fn sha256(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(b))
}

fn random_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 24];
    rand::rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[derive(Debug, Default, serde::Serialize)]
pub struct Deploy {
    pub binary: bool,
    pub unit: bool,
    pub token: bool,
    pub restarted: bool,
}

impl Deploy {
    pub fn changed(&self) -> bool {
        self.binary || self.unit || self.token
    }
}

/// Idempotent: the binary hash and the unit text are compared with what is in the container, whatever differs is placed (a
/// busy binary is replaced by renaming), the token is issued once per workspace and lives in the core secret store; the unit
/// is enabled and restarted on changes (v0: a restart takes its PTYs along — a separate scope will come with sessions).
/// Claude Code policy in workspaces (`/etc/claude-code/managed-settings.json`): by default the permission bypass mode is
/// forbidden (a container, not a VM — see templates/incus/main.tf and architecture-v3); the owner enables it in "Agents" →
/// Settings → Permissions (kv `agents.policy` `{bypass: true}`). The file is placed at every agentd deploy and on a policy
/// change.
pub const POLICY_PATH: &str = "/etc/claude-code/managed-settings.json";

pub fn policy_bypass(store: &Store) -> bool {
    store.get_kv("agents.policy").ok().flatten().and_then(|v| v.get("bypass").and_then(|b| b.as_bool())).unwrap_or(false)
}

/// Claude Code auto-update in workspaces is off (`DISABLE_AUTOUPDATER` via the policy `env`): CLI versions are updated by the
/// core and the owner themselves, and a background `npm install -g` from a live session, interrupted by stopping its process,
/// left the installation without the `claude` binary — every Claude session in the workspace stopped starting ("initialize: no
/// answer to the control request").
pub fn policy_content(bypass: bool) -> String {
    if bypass { "{\"env\":{\"DISABLE_AUTOUPDATER\":\"1\"}}\n".to_string() } else { "{\"env\":{\"DISABLE_AUTOUPDATER\":\"1\"},\"permissions\":{\"disableBypassPermissionsMode\":\"disable\"}}\n".to_string() }
}

/// Place the policy into the workspace if it differs; returns whether the file
/// changed.
pub async fn apply_policy(incus: &Incus, store: &Store, inst: &str) -> Result<bool> {
    let want = policy_content(policy_bypass(store));
    let have = incus.file_pull(inst, POLICY_PATH).await.ok().map(|b| String::from_utf8_lossy(&b).to_string());
    let same = have.as_deref().map(|h| h.trim() == want.trim()).unwrap_or(false);
    if same { return Ok(false) }
    incus.file_push(inst, POLICY_PATH, want.as_bytes(), 0o644).await.context("push managed-settings")?;
    Ok(true)
}

pub async fn deploy(incus: &Incus, store: &Store, inst: &str, project: &str) -> Result<Deploy> {
    let Some(bin) = binary() else { bail!("cloudos-agentd не встроен в cloudd и CLOUDD_AGENTD_BIN не задан") };
    let mut d = Deploy::default();
    let want = sha256(&bin);
    let have = incus.exec(inst, &["sh", "-c", &format!("sha256sum {BIN_PATH} 2>/dev/null | cut -d' ' -f1")]).await.unwrap_or_default();
    if have.trim() != want {
        let tmp = format!("{BIN_PATH}.new");
        incus.file_push(inst, &tmp, &bin, 0o755).await.context("push agentd")?;
        incus.exec(inst, &["mv", "-f", &tmp, BIN_PATH]).await.context("замена agentd")?;
        d.binary = true;
    }
    if incus.file_pull(inst, UNIT_PATH).await.ok().as_deref() != Some(AGENTD_UNIT.as_bytes()) {
        incus.file_push(inst, UNIT_PATH, AGENTD_UNIT.as_bytes(), 0o644).await.context("push юнита agentd")?;
        d.unit = true;
    }
    let scope = secret_scope(project);
    let token = match store.get_secret(&scope, TOKEN_KEY)? {
        Some(t) => t,
        None => {
            let t = random_token();
            store.put_secret(&scope, TOKEN_KEY, &t)?;
            t
        }
    };
    if let Err(e) = apply_policy(incus, store, inst).await { tracing::warn!("политика Claude Code в {inst}: {e:#}"); }
    let have_token = incus.file_pull(inst, TOKEN_PATH).await.ok().map(|b| String::from_utf8_lossy(&b).trim().to_string());
    if have_token.as_deref() != Some(token.as_str()) {
        incus.file_push(inst, TOKEN_PATH, format!("{token}\n").as_bytes(), 0o640).await.context("push токена agentd")?;
        d.token = true;
    }
    // Permissions are set always, not only on write: that also repairs files placed with a wrong mode earlier.
    incus
        .exec(inst, &["sh", "-c", &format!("chmod 0755 {BIN_PATH} && chmod 0644 {UNIT_PATH} && chown 0:1000 {TOKEN_PATH} && chmod 0640 {TOKEN_PATH}")])
        .await
        .context("права файлов agentd")?;
    // Codex ≥ 0.160 runs commands in a bubblewrap sandbox; in a container bwrap without the setuid bit does not start (the host's
    // apparmor_restrict_unprivileged_userns forbids a user namespace to the coder user) — install the package if missing and set
    // setuid, idempotently; without this every Codex command in sandboxed modes fails and asks for escalation
    match incus.exec(inst, &["sh", "-c", "command -v bwrap >/dev/null 2>&1 || DEBIAN_FRONTEND=noninteractive apt-get install -y -q bubblewrap >/dev/null 2>&1; test -u /usr/bin/bwrap || chmod u+s /usr/bin/bwrap 2>/dev/null; test -u /usr/bin/bwrap && echo ok"]).await {
        Ok(o) if o.trim() == "ok" => {}
        _ => tracing::warn!("bubblewrap в {inst}: не установлен или без setuid — песочница Codex ≥ 0.160 работать не будет"),
    }
    let active = incus.exec(inst, &["systemctl", "is-active", "--quiet", "cloudos-agentd"]).await.is_ok();
    if d.changed() || !active {
        incus
            .exec(inst, &["sh", "-c", "systemctl daemon-reload && systemctl enable cloudos-agentd >/dev/null 2>&1; systemctl restart cloudos-agentd"])
            .await
            .context("запуск cloudos-agentd")?;
        d.restarted = true;
    }
    Ok(d)
}

/// HTTP client to the adapter of one workspace. Network errors — "agentd does not answer" (the API returns
/// 502), 4xx/5xx answers of the adapter — "agentd: <code> — <text>".
#[derive(Clone)]
pub struct AgentdClient {
    http: reqwest::Client,
    pub base: String,
    token: String,
}

impl AgentdClient {
    pub fn new(ip: &str, token: String) -> Self {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().expect("reqwest");
        Self { http, base: format!("http://{ip}:{PORT}"), token }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn ws_url(&self, pty: &str) -> String {
        format!("{}/v1/ptys/{pty}/ws", self.base.replacen("http", "ws", 1))
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<(u16, Value)> {
        let mut req = self.http.request(method, format!("{}{path}", self.base)).bearer_auth(&self.token);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| anyhow!("agentd не отвечает: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let v = if text.is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) };
        if status >= 400 {
            bail!("agentd: {status} — {}", v.get("error").and_then(|e| e.as_str()).unwrap_or("ошибка"));
        }
        Ok((status, v))
    }

    pub async fn health(&self) -> Result<Value> {
        Ok(self.call(reqwest::Method::GET, "/health", None).await?.1)
    }

    pub async fn ptys(&self) -> Result<Value> {
        Ok(self.call(reqwest::Method::GET, "/v1/ptys", None).await?.1)
    }

    pub async fn pty_create(&self, body: &Value) -> Result<Value> {
        Ok(self.call(reqwest::Method::POST, "/v1/ptys", Some(body)).await?.1)
    }

    pub async fn pty(&self, id: &str) -> Result<Value> {
        Ok(self.call(reqwest::Method::GET, &format!("/v1/ptys/{id}"), None).await?.1)
    }

    pub async fn pty_resize(&self, id: &str, body: &Value) -> Result<Value> {
        Ok(self.call(reqwest::Method::POST, &format!("/v1/ptys/{id}/resize"), Some(body)).await?.1)
    }

    /// 202 — the process was signalled, 204 — the record of a finished terminal
    /// was removed.
    pub async fn pty_delete(&self, id: &str) -> Result<u16> {
        Ok(self.call(reqwest::Method::DELETE, &format!("/v1/ptys/{id}"), None).await?.0)
    }

    // Generic calls for agent sessions and the expense ledger — the core API proxies them one to
    // one.
    pub async fn get(&self, path: &str) -> Result<Value> {
        Ok(self.call(reqwest::Method::GET, path, None).await?.1)
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        Ok(self.call(reqwest::Method::POST, path, Some(body)).await?.1)
    }

    /// GET with a long wait (engine detection calls four CLIs for versions).
    pub async fn post_slow_get(&self, path: &str, secs: u64) -> Result<Value> {
        let resp = self.http.get(format!("{}{path}", self.base)).bearer_auth(&self.token).timeout(Duration::from_secs(secs)).send().await.map_err(|e| anyhow!("agentd не отвечает: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let v = if text.is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) };
        if status >= 400 { bail!("agentd: {status} — {}", v.get("error").and_then(|e| e.as_str()).unwrap_or("ошибка")); }
        Ok(v)
    }

    /// POST with a long wait: calls backed by a model turn (llm/quick, git/suggest) — longer than the client's 10 s.
    pub async fn post_slow(&self, path: &str, body: &Value, secs: u64) -> Result<Value> {
        let resp = self.http.post(format!("{}{path}", self.base)).bearer_auth(&self.token).timeout(Duration::from_secs(secs)).json(body).send().await.map_err(|e| anyhow!("agentd не отвечает: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let v = if text.is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) };
        if status >= 400 { bail!("agentd: {status} — {}", v.get("error").and_then(|e| e.as_str()).unwrap_or("ошибка")); }
        Ok(v)
    }

    pub async fn put(&self, path: &str, body: &Value) -> Result<Value> {
        Ok(self.call(reqwest::Method::PUT, path, Some(body)).await?.1)
    }

    pub async fn delete(&self, path: &str) -> Result<u16> {
        Ok(self.call(reqwest::Method::DELETE, path, None).await?.0)
    }

    /// Raw GET (workspace files): status, headers and bytes; a longer timeout than usual — files up to 64
    /// MB.
    pub async fn raw_get(&self, path: &str) -> Result<(u16, reqwest::header::HeaderMap, axum::body::Bytes)> {
        let resp = self.http.get(format!("{}{path}", self.base)).bearer_auth(&self.token).timeout(Duration::from_secs(120)).send().await.map_err(|e| anyhow!("agentd не отвечает: {e}"))?;
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let body = resp.bytes().await.map_err(|e| anyhow!("agentd: {e}"))?;
        if status >= 400 {
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            bail!("agentd: {status} — {}", v.get("error").and_then(|e| e.as_str()).unwrap_or("ошибка"));
        }
        Ok((status, headers, body))
    }

    /// Raw PUT of bytes (file upload into the
    /// workspace).
    pub async fn raw_put(&self, path: &str, body: axum::body::Bytes) -> Result<Value> {
        let resp = self.http.put(format!("{}{path}", self.base)).bearer_auth(&self.token).timeout(Duration::from_secs(300)).body(body).send().await.map_err(|e| anyhow!("agentd не отвечает: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let v = if text.is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) };
        if status >= 400 { bail!("agentd: {status} — {}", v.get("error").and_then(|e| e.as_str()).unwrap_or("ошибка")); }
        Ok(v)
    }

    /// Raw POST of bytes (a session archive when moving between workspaces): the answer is
    /// agentd JSON.
    pub async fn raw_post(&self, path: &str, body: axum::body::Bytes) -> Result<Value> {
        let resp = self.http.post(format!("{}{path}", self.base)).bearer_auth(&self.token).timeout(Duration::from_secs(600)).header("content-type", "application/x-tar").body(body).send().await.map_err(|e| anyhow!("agentd не отвечает: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let v = if text.is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) };
        if status >= 400 { bail!("agentd: {status} — {}", v.get("error").and_then(|e| e.as_str()).unwrap_or("ошибка")); }
        Ok(v)
    }

    pub fn ws_url_for(&self, path: &str) -> String {
        format!("{}{path}", self.base.replacen("http", "ws", 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_and_token_shape() {
        assert!(AGENTD_UNIT.contains("ExecStart=/usr/local/bin/cloudos-agentd"), "юнит должен запускать бинарь из BIN_PATH");
        assert!(AGENTD_UNIT.contains("User=coder"));
        let t = random_token();
        assert_eq!(t.len(), 48);
        assert!(t.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(secret_scope("alpha"), "workspace:alpha");
    }

    #[test]
    fn client_urls() {
        let c = AgentdClient::new("10.77.7.146", "t".into());
        assert_eq!(c.base, "http://10.77.7.146:7200");
        assert_eq!(c.ws_url("abc"), "ws://10.77.7.146:7200/v1/ptys/abc/ws");
    }
}
