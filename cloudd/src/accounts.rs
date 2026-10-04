//! Аккаунты агентов (ADR-006, решение 6): реестр в ядре, материализация в workspace при старте сессии, pinning.
//! Три пути к моделям: (A) прокси/gate — переменные `*_BASE_URL` и токен; (B) подписки самих CLI — файлы каталога
//! конфигурации (`CLAUDE_CONFIG_DIR/.credentials.json`, `CODEX_HOME/auth.json`), снятые с workspace, где владелец уже
//! вошёл; (C) ключи API — `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`. Метаданные — kv `accounts.list`, секреты — scope
//! `account:<id>` (переменные по имени, файлы как `file:<имя>`), аккаунт проекта по умолчанию — kv `accounts.default.<проект>`.
use crate::service::Service;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const LIST_KEY: &str = "accounts.list";
/// Куда кладутся файлы аккаунта в workspace (владелец coder).
pub const WS_DIR: &str = "/home/coder/.cloudos/accounts";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    /// `claude` | `codex` | `anthropic` | `openai` | `openrouter` | … — чей это доступ
    pub provider: String,
    /// `subscription` (файлы CLI) | `api_key` | `proxy` (адрес и токен, в т.ч. gate проекта)
    pub kind: String,
    pub label: String,
    pub created_at: String,
    #[serde(default)]
    pub env_keys: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub imported_from: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AccountCreate {
    pub provider: String,
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub files: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct AccountImport {
    /// `claude` | `codex` — снять файлы входа из workspace проекта; `gate` — скопировать переменные llm-гранта проекта
    pub provider: String,
    pub project: String,
    pub label: Option<String>,
    /// Каталог конфигурации в workspace, куда вошли через `claude auth login` / `codex login --device-auth`
    /// (терминал входа из «Агентов»); без него — штатные ~/.claude и ~/.codex
    #[serde(default)]
    pub dir: Option<String>,
}

/// Переменные, которые задаёт gate проекта через env.sh: аккаунт подписки или ключа должен их снять, иначе CLI пойдёт в gate.
fn gate_vars(provider: &str) -> &'static [&'static str] {
    match provider {
        "claude" | "anthropic" => &["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"],
        "codex" | "openai" => &["OPENAI_BASE_URL", "OPENAI_API_KEY"],
        "gemini" | "google" => &["GEMINI_BASE_URL", "GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL"],
        _ => &[],
    }
}

/// Что получит процесс агента: переменные аккаунта (+ каталог конфигурации для подписок) и список снимаемых.
pub fn session_env(a: &Account, env: &HashMap<String, String>) -> (HashMap<String, String>, Vec<String>) {
    let mut out = env.clone();
    if !a.files.is_empty() {
        let dir = format!("{WS_DIR}/{}", a.id);
        match a.provider.as_str() {
            "claude" => { out.insert("CLAUDE_CONFIG_DIR".into(), dir); }
            "codex" => { out.insert("CODEX_HOME".into(), dir); }
            _ => {}
        }
    }
    let unset = gate_vars(&a.provider).iter().filter(|k| !out.contains_key(**k)).map(|k| k.to_string()).collect();
    (out, unset)
}

impl Service {
    pub fn accounts(&self) -> Result<Vec<Account>> {
        Ok(self.store.get_kv(LIST_KEY)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
    }

    fn save_accounts(&self, list: &[Account]) -> Result<()> {
        self.store.put_kv(LIST_KEY, &json!(list))
    }

    pub fn account(&self, id: &str) -> Result<Account> {
        self.accounts()?.into_iter().find(|a| a.id == id).ok_or_else(|| anyhow!("аккаунт {id} не найден"))
    }

    pub fn account_create(&self, c: AccountCreate) -> Result<Account> {
        if c.label.trim().is_empty() { bail!("аккаунт требует название"); }
        if c.env.is_empty() && c.files.is_empty() { bail!("аккаунт требует переменные или файлы"); }
        let id = format!("{}-{}", c.provider, &uuid::Uuid::new_v4().to_string()[..8]);
        let scope = format!("account:{id}");
        for (k, v) in &c.env { self.store.put_secret(&scope, k, v)?; }
        for (name, content) in &c.files { self.store.put_secret(&scope, &format!("file:{name}"), content)?; }
        let a = Account { id, provider: c.provider, kind: c.kind, label: c.label.trim().to_string(), created_at: crate::store::now(), env_keys: c.env.keys().cloned().collect(), files: c.files.keys().cloned().collect(), imported_from: None };
        let mut list = self.accounts()?;
        list.push(a.clone());
        self.save_accounts(&list)?;
        self.store.event("account", &a.id, &format!("добавлен аккаунт «{}» ({} · {})", a.label, a.provider, a.kind))?;
        Ok(a)
    }

    /// Снять вход с workspace проекта (владелец уже вошёл там сам) или скопировать переменные gate'а проекта как аккаунт-прокси.
    pub async fn account_import(&self, i: AccountImport) -> Result<Account> {
        let inst = self.workspace_instance(&i.project).await;
        let dir = i.dir.as_deref().map(|d| d.trim_end_matches('/').to_string());
        if let Some(d) = &dir { if !d.starts_with("/home/coder/") || d.contains("..") { bail!("каталог входа должен быть внутри /home/coder"); } }
        let (provider, kind, files, env): (String, &str, HashMap<String, String>, HashMap<String, String>) = match i.provider.as_str() {
            "claude" => {
                let base = dir.clone().unwrap_or_else(|| "/home/coder/.claude".into());
                let b = self.incus.file_pull(&inst, &format!("{base}/.credentials.json")).await.map_err(|e| anyhow!("в {} нет входа Claude ({base}/.credentials.json): {e}", i.project))?;
                ("claude".into(), "subscription", HashMap::from([(".credentials.json".to_string(), String::from_utf8_lossy(&b).to_string())]), HashMap::new())
            }
            "codex" => {
                let base = dir.clone().unwrap_or_else(|| "/home/coder/.codex".into());
                let b = self.incus.file_pull(&inst, &format!("{base}/auth.json")).await.map_err(|e| anyhow!("в {} нет входа Codex ({base}/auth.json): {e}", i.project))?;
                let mut files = HashMap::from([("auth.json".to_string(), String::from_utf8_lossy(&b).to_string())]);
                if let Ok(c) = self.incus.file_pull(&inst, &format!("{base}/config.toml")).await { files.insert("config.toml".into(), String::from_utf8_lossy(&c).to_string()); }
                ("codex".into(), "subscription", files, HashMap::new())
            }
            "gate" => {
                let scope = format!("project:{}:llm", i.project);
                let mut env = HashMap::new();
                for k in self.store.list_secret_keys(&scope)? {
                    if let Some(v) = self.store.get_secret(&scope, &k)? { env.insert(k, v); }
                }
                if env.is_empty() { bail!("у проекта {} нет гранта llm", i.project); }
                ("gate".into(), "proxy", HashMap::new(), env)
            }
            other => bail!("импорт: claude | codex | gate, получено {other}"),
        };
        let label = i.label.unwrap_or_else(|| match (kind, dir.is_some()) { ("subscription", true) => format!("{} · вход OAuth", provider), ("subscription", false) => format!("{} · вход из {}", provider, i.project), _ => format!("gate проекта {}", i.project) });
        let mut a = self.account_create(AccountCreate { provider, kind: kind.into(), label, env, files })?;
        a.imported_from = Some(i.project.clone());
        let mut list = self.accounts()?;
        if let Some(x) = list.iter_mut().find(|x| x.id == a.id) { x.imported_from = a.imported_from.clone(); }
        self.save_accounts(&list)?;
        Ok(a)
    }

    pub fn account_delete(&self, id: &str) -> Result<()> {
        let mut list = self.accounts()?;
        let before = list.len();
        list.retain(|a| a.id != id);
        if list.len() == before { bail!("аккаунт {id} не найден"); }
        self.save_accounts(&list)?;
        self.store.delete_secrets(&format!("account:{id}"))?;
        // снять как умолчание у проектов
        for st in self.store.list_projects()? {
            if self.project_account(&st.name)?.as_deref() == Some(id) { self.set_project_account(&st.name, None)?; }
        }
        self.store.event("account", id, "аккаунт удалён")?;
        Ok(())
    }

    pub fn project_account(&self, project: &str) -> Result<Option<String>> {
        Ok(self.store.get_kv(&format!("accounts.default.{project}"))?.and_then(|v| v.as_str().map(str::to_string)))
    }

    pub fn set_project_account(&self, project: &str, id: Option<&str>) -> Result<()> {
        if let Some(id) = id { self.account(id)?; }
        self.store.put_kv(&format!("accounts.default.{project}"), &json!(id))
    }

    /// Материализовать аккаунт в workspace проекта и вернуть окружение для запуска сессии.
    pub async fn account_session_env(&self, project: &str, id: &str) -> Result<(HashMap<String, String>, Vec<String>)> {
        let a = self.account(id)?;
        let scope = format!("account:{id}");
        let mut env = HashMap::new();
        for k in &a.env_keys {
            if let Some(v) = self.store.get_secret(&scope, k)? { env.insert(k.clone(), v); }
        }
        if !a.files.is_empty() {
            let inst = self.workspace_instance(project).await;
            let dir = format!("{WS_DIR}/{}", a.id);
            for name in &a.files {
                let content = self.store.get_secret(&scope, &format!("file:{name}"))?.ok_or_else(|| anyhow!("у аккаунта {id} нет файла {name}"))?;
                self.incus.file_push(&inst, &format!("{dir}/{name}"), content.as_bytes(), 0o600).await?;
            }
            self.incus.exec(&inst, &["sh", "-c", &format!("chown -R 1000:1000 {WS_DIR} && chmod 700 {WS_DIR} {dir}")]).await?;
        }
        Ok(session_env(&a, &env))
    }

    /// Аккаунт для новой сессии: явный из тела запроса (`""` — без аккаунта), иначе умолчание проекта; для Codex без
    /// умолчания — свежайшая подписка ChatGPT, если она есть: gate для Codex нужен только ради OpenAI-совместимых моделей
    /// прокси, а владелец, войдя в ChatGPT, ждёт, что чат Codex пойдёт через подписку (Claude через gate не трогаем: там Fable).
    pub async fn resolve_session_account(&self, project: &str, body: &mut Value) -> Result<()> {
        let explicit = body.get("account").and_then(|v| v.as_str()).map(str::to_string);
        let mut chosen = match explicit { Some(s) if s.is_empty() => None, Some(s) => Some(s), None => self.project_account(project)? };
        if chosen.is_none() && body.get("account").is_none() {
            let mut agent = body.get("agent").and_then(|a| a.as_str()).map(str::to_string);
            let mut stored_account: Option<String> = None;
            if let Some(rid) = body.get("resume").and_then(|r| r.as_str()) {
                if let Ok(c) = self.agentd_client(project).await { if let Ok(v) = c.get(&format!("/v1/sessions/{rid}")).await {
                    if agent.is_none() { agent = v.pointer("/info/agent").and_then(|a| a.as_str()).map(str::to_string); }
                    stored_account = v.pointer("/info/account").and_then(|a| a.as_str()).map(str::to_string);
                } }
            }
            if agent.as_deref() == Some("codex") && stored_account.is_none() {
                chosen = self.accounts()?.into_iter().filter(|a| a.provider == "codex" && a.kind == "subscription").max_by(|a, b| a.created_at.cmp(&b.created_at)).map(|a| a.id);
                if let Some(id) = &chosen {
                    tracing::info!("проект {project}: Codex без умолчания аккаунта — подписка {id}");
                    // возобновление старой сессии без аккаунта: agentd применяет новое окружение только с reassign
                    if body.get("resume").is_some() { body["reassign"] = json!(true); }
                }
            }
        }
        let Some(id) = chosen else { return Ok(()) };
        let (env, unset) = self.account_session_env(project, &id).await?;
        body["account"] = json!(id);
        body["env"] = json!(env);
        body["unset"] = json!(unset);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(provider: &str, kind: &str, files: &[&str], env_keys: &[&str]) -> Account {
        Account { id: format!("{provider}-1"), provider: provider.into(), kind: kind.into(), label: "t".into(), created_at: String::new(), env_keys: env_keys.iter().map(|s| s.to_string()).collect(), files: files.iter().map(|s| s.to_string()).collect(), imported_from: None }
    }

    #[test]
    fn claude_subscription_pins_config_dir_and_drops_gate() {
        let (env, unset) = session_env(&acc("claude", "subscription", &[".credentials.json"], &[]), &HashMap::new());
        assert_eq!(env.get("CLAUDE_CONFIG_DIR").map(String::as_str), Some("/home/coder/.cloudos/accounts/claude-1"));
        assert_eq!(unset, vec!["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"]);
    }

    #[test]
    fn api_key_keeps_its_own_vars() {
        let env = HashMap::from([("ANTHROPIC_API_KEY".to_string(), "sk".to_string()), ("ANTHROPIC_BASE_URL".to_string(), "https://openrouter.ai/api".to_string())]);
        let (out, unset) = session_env(&acc("claude", "api_key", &[], &["ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL"]), &env);
        assert_eq!(out.len(), 2);
        assert_eq!(unset, vec!["ANTHROPIC_AUTH_TOKEN"], "снимается только то, что не задано аккаунтом");
    }

    #[test]
    fn codex_home_for_codex_files() {
        let (env, unset) = session_env(&acc("codex", "subscription", &["auth.json"], &[]), &HashMap::new());
        assert_eq!(env.get("CODEX_HOME").map(String::as_str), Some("/home/coder/.cloudos/accounts/codex-1"));
        assert_eq!(unset, vec!["OPENAI_BASE_URL", "OPENAI_API_KEY"]);
    }

    #[test]
    fn gate_proxy_overrides_nothing_else() {
        let env = HashMap::from([("ANTHROPIC_BASE_URL".to_string(), "http://llm.gate".to_string()), ("ANTHROPIC_AUTH_TOKEN".to_string(), "t".to_string())]);
        let (out, unset) = session_env(&acc("gate", "proxy", &[], &["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"]), &env);
        assert_eq!(out.len(), 2);
        assert!(unset.is_empty());
    }
}


// ---- вход Claude по ссылке и коду (OAuth с PKCE, как делает сам Claude Code) ----
// Ядро строит ссылку авторизации с redirect на страницу кода (`platform.claude.com/oauth/code/callback`): владелец
// открывает её на любом устройстве, входит, копирует показанный `code#state` и вставляет его в интерфейс; ядро меняет код
// на токены у `platform.claude.com/v1/oauth/token` и сохраняет `.credentials.json` в формате Claude Code как аккаунт-подписку.
pub const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const CLAUDE_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
pub const CLAUDE_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const CLAUDE_MANUAL_REDIRECT: &str = "https://platform.claude.com/oauth/code/callback";
pub const CLAUDE_SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload user:plugins";
const OAUTH_PENDING_KEY: &str = "accounts.oauth.pending";
// ---- вход ChatGPT (Codex) по ссылке: device-code, как `codex login --device-auth` ----
// Ядро просит одноразовый код у auth.openai.com, владелец открывает страницу устройства на любом устройстве и вводит код;
// ядро опрашивает выдачу, меняет authorization_code на токены (redirect устройства, PKCE даёт сервер) и сохраняет
// `auth.json` в формате Codex CLI (auth_mode chatgpt, tokens.id_token/access_token/refresh_token/account_id) как аккаунт-подписку.
pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const CODEX_DEVICE_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
pub const CODEX_DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
pub const CODEX_DEVICE_VERIFY_URL: &str = "https://auth.openai.com/codex/device";
pub const CODEX_DEVICE_REDIRECT: &str = "https://auth.openai.com/deviceauth/callback";
pub const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
// ---- OAuth-ссылки: вход в браузере, затем адрес из адресной строки (localhost:<порт>/…?code=…&state=…) ----
pub const CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const CODEX_OAUTH_REDIRECT: &str = "http://localhost:1455/auth/callback";
pub const CLAUDE_CB_REDIRECT: &str = "http://localhost:54545/callback";
/// Код устройства живёт 15 минут (auth.openai.com отдаёт expires_at; держим и свой предел).
const CODEX_DEVICE_TTL: u64 = 15 * 60;

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf: u32 = 0; let mut bits = 0;
    for c in s.bytes() {
        let v = match c { b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52, b'-' | b'+' => 62, b'_' | b'/' => 63, b'=' => continue, _ => return None } as u32;
        buf = (buf << 6) | v; bits += 6;
        if bits >= 8 { bits -= 8; out.push((buf >> bits) as u8 & 0xff); }
    }
    Some(out)
}

/// Полезные claims id_token OpenAI: email, chatgpt_account_id, chatgpt_plan_type (как Codex CLI `token_data.rs`).
pub fn codex_id_claims(jwt: &str) -> (Option<String>, Option<String>, Option<String>) {
    let payload = jwt.split('.').nth(1).and_then(b64url_decode).and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
    let email = payload.get("email").and_then(|v| v.as_str()).or_else(|| payload.pointer("/https:~1~1api.openai.com~1profile/email").and_then(|v| v.as_str())).map(str::to_string);
    let auth = payload.get("https://api.openai.com/auth");
    let account = auth.and_then(|a| a.get("chatgpt_account_id")).and_then(|v| v.as_str()).map(str::to_string);
    let plan = auth.and_then(|a| a.get("chatgpt_plan_type")).and_then(|v| v.as_str()).map(str::to_string);
    (email, account, plan)
}

/// RFC 3339 UTC из unix-секунд (chrono в ядре нет): `2026-09-30T10:20:30Z`.
pub fn rfc3339_utc(secs: u64) -> String {
    let days = secs / 86400; let rem = secs % 86400;
    // civil_from_days (Howard Hinnant)
    let z = days as i64 + 719468; let era = z.div_euclid(146097); let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1; let m = if mp < 10 { mp + 3 } else { mp - 9 }; let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `auth.json` Codex CLI из ответа token endpoint'а (формат `AuthDotJson` codex-rs/login: auth_mode, OPENAI_API_KEY, tokens, last_refresh).
pub fn codex_auth_json(tok: &Value, now_secs: u64) -> Result<(String, Option<String>, Option<String>)> {
    let id_token = tok.get("id_token").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("в ответе нет id_token"))?;
    let access = tok.get("access_token").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("в ответе нет access_token"))?;
    let refresh = tok.get("refresh_token").and_then(|v| v.as_str()).unwrap_or("");
    let (email, account_id, plan) = codex_id_claims(id_token);
    let o = json!({ "auth_mode": "chatgpt", "OPENAI_API_KEY": Value::Null, "tokens": { "id_token": id_token, "access_token": access, "refresh_token": refresh, "account_id": account_id }, "last_refresh": rfc3339_utc(now_secs) });
    Ok((serde_json::to_string(&o)?, email, plan))
}

fn b64url(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((b.len() + 2) / 3 * 4);
    for chunk in b.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char); out.push(T[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 { out.push(T[(n >> 6) as usize & 63] as char); }
        if chunk.len() > 2 { out.push(T[n as usize & 63] as char); }
    }
    out
}

fn now_secs() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

fn rand_b64(n: usize) -> String {
    use rand::RngCore;
    let mut b = vec![0u8; n];
    rand::rng().fill_bytes(&mut b);
    b64url(&b)
}

/// Ссылка авторизации Claude с PKCE; verifier ждёт кода под своим `state` (не дольше часа).
pub fn oauth_authorize_url(verifier: &str, state: &str) -> String {
    use sha2::{Digest, Sha256};
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    format!("{CLAUDE_AUTHORIZE_URL}?code=true&client_id={CLAUDE_CLIENT_ID}&response_type=code&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&state={state}",
        urlencoding::encode(CLAUDE_MANUAL_REDIRECT), urlencoding::encode(CLAUDE_SCOPES).replace("%20", "+"))
}

/// OAuth-ссылка Codex с PKCE и возвратом на localhost:1455 (параметры как у Codex CLI).
pub fn codex_authorize_url(verifier: &str, state: &str) -> String {
    use sha2::{Digest, Sha256};
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    format!("{CODEX_AUTHORIZE_URL}?client_id={CODEX_CLIENT_ID}&response_type=code&redirect_uri={}&scope={}&state={state}&code_challenge={challenge}&code_challenge_method=S256&prompt=login&id_token_add_organizations=true&codex_cli_simplified_flow=true",
        urlencoding::encode(CODEX_OAUTH_REDIRECT), urlencoding::encode("openid email profile offline_access").replace("%20", "+"))
}

/// OAuth-ссылка Claude с возвратом на localhost:54545, вместо страницы с кодом.
pub fn claude_authorize_url_cb(verifier: &str, state: &str) -> String {
    use sha2::{Digest, Sha256};
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    format!("{CLAUDE_AUTHORIZE_URL}?code=true&client_id={CLAUDE_CLIENT_ID}&response_type=code&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&state={state}",
        urlencoding::encode(CLAUDE_CB_REDIRECT), urlencoding::encode(CLAUDE_SCOPES).replace("%20", "+"))
}

/// Разбор того, что вставил владелец: адрес после входа (`http://localhost:…/…?code=…&state=…`), `code#state` или голый код.
/// Возвращает (code, state); state из адреса обязан совпасть с ожидаемым.
pub fn parse_callback_input(raw: &str, expected_state: &str) -> Result<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() { bail!("пустой код"); }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        let q = raw.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut code = String::new(); let mut state = String::new(); let mut err = String::new();
        for kv in q.split('&') {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            let v = urlencoding::decode(v).map(|c| c.into_owned()).unwrap_or_else(|_| v.to_string());
            match k { "code" => code = v, "state" => state = v, "error" | "error_description" => { if err.is_empty() { err = v } }, _ => {} }
        }
        if !err.is_empty() { bail!("провайдер вернул ошибку: {err}"); }
        if code.is_empty() { bail!("в адресе нет code — скопируйте адрес целиком, как он показан после входа"); }
        if !state.is_empty() && state != expected_state { bail!("этот адрес от другого входа (state не совпадает) — откройте ссылку заново и вставьте адрес того же входа"); }
        return Ok((code, expected_state.to_string()));
    }
    match raw.split_once('#') { Some((c, st)) => Ok((c.trim().to_string(), st.trim().to_string())), None => Ok((raw.to_string(), expected_state.to_string())) }
}

/// `.credentials.json` в формате Claude Code из ответа token endpoint'а.
pub fn credentials_from_token(tok: &Value) -> Result<String> {
    let access = tok.get("access_token").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("в ответе нет access_token"))?;
    let refresh = tok.get("refresh_token").and_then(|v| v.as_str()).unwrap_or("");
    let expires_in = tok.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(3600);
    let scopes: Vec<&str> = tok.get("scope").and_then(|v| v.as_str()).map(|s| s.split_whitespace().collect()).unwrap_or_default();
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let mut o = json!({ "claudeAiOauth": { "accessToken": access, "refreshToken": refresh, "expiresAt": now_ms + expires_in * 1000, "scopes": scopes } });
    if let Some(t) = tok.pointer("/account/subscription_type").or_else(|| tok.get("subscription_type")).and_then(|v| v.as_str()) { o["claudeAiOauth"]["subscriptionType"] = json!(t); }
    Ok(serde_json::to_string(&o)?)
}

pub enum DevicePoll { Wait, Done(Value), Error(String) }

#[derive(Debug, Deserialize)]
pub struct OauthFinish {
    pub state: String,
    /// код со страницы (`code#state` целиком тоже принимается)
    pub code: String,
    pub label: Option<String>,
}

impl Service {
    fn oauth_pending_put(&self, state: &str, entry: Value) -> Result<()> {
        let mut pending = self.store.get_kv(OAUTH_PENDING_KEY)?.unwrap_or(json!({}));
        let now = now_secs();
        if let Some(m) = pending.as_object_mut() { m.retain(|_, v| v.get("at").and_then(|a| a.as_u64()).map(|a| now - a < 3600).unwrap_or(false)); m.insert(state.to_string(), entry); }
        self.store.put_kv(OAUTH_PENDING_KEY, &pending)
    }
    fn oauth_pending_remove(&self, state: &str) -> Result<()> {
        let mut pending = self.store.get_kv(OAUTH_PENDING_KEY)?.unwrap_or(json!({}));
        if let Some(m) = pending.as_object_mut() { m.remove(state); }
        self.store.put_kv(OAUTH_PENDING_KEY, &pending)
    }

    /// Ссылка входа: Claude — авторизация с PKCE и кодом со страницы; Codex — одноразовый код устройства (device-code).
    pub async fn oauth_start(&self, provider: &str) -> Result<Value> { self.oauth_start_for(provider, None).await }

    /// То же, с пометкой «для gate»: результат уйдёт не в аккаунт ядра, а файлом входа в прокси приложения `gate` (gateauth.rs).
    pub async fn oauth_start_for(&self, provider: &str, gate: Option<&str>) -> Result<Value> {
        match provider {
            "codex-oauth" | "claude-oauth" => {
                let verifier = rand_b64(32); let state = rand_b64(24);
                self.oauth_pending_put(&state, json!({ "provider": provider, "verifier": verifier, "at": now_secs(), "gate": gate }))?;
                let (url, cb) = if provider == "codex-oauth" { (codex_authorize_url(&verifier, &state), CODEX_OAUTH_REDIRECT) } else { (claude_authorize_url_cb(&verifier, &state), CLAUDE_CB_REDIRECT) };
                Ok(json!({ "provider": provider, "state": state, "url": url, "flow": "callback", "callback": cb, "hint": "redirect_url" }))
            }
            "claude" => {
                let verifier = rand_b64(32); let state = rand_b64(24);
                self.oauth_pending_put(&state, json!({ "provider": "claude", "verifier": verifier, "at": now_secs(), "gate": gate }))?;
                Ok(json!({ "provider": "claude", "state": state, "url": oauth_authorize_url(&verifier, &state), "hint": "code#state" }))
            }
            "codex" => {
                let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
                let resp = http.post(CODEX_DEVICE_USERCODE_URL).json(&json!({ "client_id": CODEX_CLIENT_ID })).header("Accept", "application/json").send().await.map_err(|e| anyhow!("auth.openai.com недоступен: {e}"))?;
                let status = resp.status().as_u16(); let text = resp.text().await.unwrap_or_default();
                if !(200..300).contains(&status) { bail!("код устройства: {status} — {}", text.chars().take(200).collect::<String>()); }
                let v: Value = serde_json::from_str(&text).map_err(|e| anyhow!("ответ device auth не JSON: {e}"))?;
                let device_auth_id = v.get("device_auth_id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let user_code = v.get("user_code").and_then(|x| x.as_str()).or_else(|| v.get("usercode").and_then(|x| x.as_str())).unwrap_or("").to_string();
                if device_auth_id.is_empty() || user_code.is_empty() { bail!("auth.openai.com не вернул код устройства"); }
                let interval = v.get("interval").and_then(|i| i.as_u64().or_else(|| i.as_str().and_then(|s| s.parse().ok()))).filter(|i| *i > 0).unwrap_or(5);
                let state = rand_b64(24); let at = now_secs();
                self.oauth_pending_put(&state, json!({ "provider": "codex", "device_auth_id": device_auth_id, "user_code": user_code, "interval": interval, "at": at, "gate": gate }))?;
                Ok(json!({ "provider": "codex", "state": state, "url": CODEX_DEVICE_VERIFY_URL, "user_code": user_code, "interval": interval, "expires_in": CODEX_DEVICE_TTL, "hint": "device" }))
            }
            other => bail!("вход по ссылке: claude | codex | claude-oauth | codex-oauth, получено {other}"),
        }
    }

    /// Ожидающий вход по state (provider, gate, …) — если ещё не истёк.
    pub fn oauth_pending(&self, state: &str) -> Result<Option<Value>> {
        Ok(self.store.get_kv(OAUTH_PENDING_KEY)?.unwrap_or(json!({})).get(state).cloned())
    }

    /// Один опрос device-code Codex: `Wait` — не подтверждено, `Done(tok)` — токены (pending снят), `Error` — истёк или отказ (pending снят).
    pub async fn codex_device_poll(&self, state: &str) -> Result<DevicePoll> {
        let Some(p) = self.oauth_pending(state)? else { return Ok(DevicePoll::Error("state не найден или устарел — получите ссылку заново".into())) };
        if p.get("provider").and_then(|v| v.as_str()) != Some("codex") { return Ok(DevicePoll::Wait); }
        let at = p.get("at").and_then(|v| v.as_u64()).unwrap_or(0);
        if now_secs().saturating_sub(at) > CODEX_DEVICE_TTL { self.oauth_pending_remove(state)?; return Ok(DevicePoll::Error("код устройства истёк — получите новый".into())); }
        let (device_auth_id, user_code) = (p.get("device_auth_id").and_then(|v| v.as_str()).unwrap_or("").to_string(), p.get("user_code").and_then(|v| v.as_str()).unwrap_or("").to_string());
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
        let resp = http.post(CODEX_DEVICE_TOKEN_URL).json(&json!({ "device_auth_id": device_auth_id, "user_code": user_code })).header("Accept", "application/json").send().await.map_err(|e| anyhow!("auth.openai.com недоступен: {e}"))?;
        let status = resp.status().as_u16(); let text = resp.text().await.unwrap_or_default();
        if status == 403 || status == 404 { return Ok(DevicePoll::Wait); }
        if !(200..300).contains(&status) { return Ok(DevicePoll::Error(format!("device auth: {status} — {}", text.chars().take(200).collect::<String>()))); }
        let v: Value = serde_json::from_str(&text).map_err(|e| anyhow!("ответ device auth не JSON: {e}"))?;
        let code = v.get("authorization_code").and_then(|x| x.as_str()).unwrap_or("");
        let verifier = v.get("code_verifier").and_then(|x| x.as_str()).unwrap_or("");
        if code.is_empty() || verifier.is_empty() { return Ok(DevicePoll::Error("auth.openai.com не вернул authorization_code".into())); }
        let form = [("grant_type", "authorization_code"), ("client_id", CODEX_CLIENT_ID), ("code", code), ("redirect_uri", CODEX_DEVICE_REDIRECT), ("code_verifier", verifier)];
        let resp = http.post(CODEX_TOKEN_URL).form(&form).header("Accept", "application/json").send().await.map_err(|e| anyhow!("auth.openai.com недоступен: {e}"))?;
        let status = resp.status().as_u16(); let text = resp.text().await.unwrap_or_default();
        if status != 200 { self.oauth_pending_remove(state)?; return Ok(DevicePoll::Error(format!("обмен кода на токены: {status} — {}", text.chars().take(300).collect::<String>()))); }
        let tok: Value = serde_json::from_str(&text).map_err(|e| anyhow!("ответ token endpoint'а не JSON: {e}"))?;
        self.oauth_pending_remove(state)?;
        Ok(DevicePoll::Done(tok))
    }

    /// Состояние входа по ссылке (Codex, аккаунт ядра): `wait` — ещё не подтверждено, `ok` — аккаунт создан, `error` — истёк или отказ.
    pub async fn oauth_status(&self, state: &str) -> Result<Value> {
        match self.codex_device_poll(state).await? {
            DevicePoll::Wait => Ok(json!({ "status": "wait" })),
            DevicePoll::Error(e) => Ok(json!({ "status": "error", "error": e })),
            DevicePoll::Done(tok) => {
                let (auth_json, email, plan) = codex_auth_json(&tok, now_secs())?;
                let label = match (&email, &plan) { (Some(e), Some(pl)) => format!("codex · {e} ({pl})"), (Some(e), None) => format!("codex · {e}"), _ => "codex · вход OAuth".into() };
                let a = self.account_create(AccountCreate { provider: "codex".into(), kind: "subscription".into(), label, env: HashMap::new(), files: HashMap::from([("auth.json".to_string(), auth_json)]) })?;
                Ok(json!({ "status": "ok", "account": a }))
            }
        }
    }

    /// Обмен кода на токены по ожидающему входу: Claude со страницы (`code#state`), Claude/Codex по OAuth-ссылке (адрес из
    /// браузера). Возвращает (провайдер токенов: claude | codex, ответ token endpoint'а); pending снимается.
    pub async fn oauth_exchange(&self, state: &str, raw: &str) -> Result<(String, Value)> {
        // `code#state` со страницы несёт свой state — он и есть ключ pending
        let state = match raw.trim().split_once('#') { Some((_, st)) if !raw.trim().starts_with("http") => st.trim().to_string(), _ => state.trim().to_string() };
        let p = self.oauth_pending(&state)?.ok_or_else(|| anyhow!("state не найден или устарел — получите ссылку заново"))?;
        let provider = p.get("provider").and_then(|v| v.as_str()).unwrap_or("claude").to_string();
        let verifier = p.get("verifier").and_then(|v| v.as_str()).map(str::to_string).ok_or_else(|| anyhow!("этот вход ждёт код устройства, а не адрес"))?;
        let (code, _) = parse_callback_input(raw, &state)?;
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
        let (kind, status, text) = match provider.as_str() {
            "codex-oauth" => {
                let form = [("grant_type", "authorization_code"), ("client_id", CODEX_CLIENT_ID), ("code", code.as_str()), ("redirect_uri", CODEX_OAUTH_REDIRECT), ("code_verifier", verifier.as_str())];
                let resp = http.post(CODEX_TOKEN_URL).form(&form).header("Accept", "application/json").send().await.map_err(|e| anyhow!("auth.openai.com недоступен: {e}"))?;
                ("codex", resp.status().as_u16(), resp.text().await.unwrap_or_default())
            }
            _ => {
                let redirect = if provider == "claude-oauth" { CLAUDE_CB_REDIRECT } else { CLAUDE_MANUAL_REDIRECT };
                let body = json!({ "grant_type": "authorization_code", "code": code, "state": state, "redirect_uri": redirect, "client_id": CLAUDE_CLIENT_ID, "code_verifier": verifier });
                let resp = http.post(CLAUDE_TOKEN_URL).json(&body).send().await.map_err(|e| anyhow!("platform.claude.com недоступен: {e}"))?;
                ("claude", resp.status().as_u16(), resp.text().await.unwrap_or_default())
            }
        };
        if status != 200 { bail!("обмен кода на токен: {status} — {}", text.chars().take(300).collect::<String>()); }
        let tok: Value = serde_json::from_str(&text).map_err(|e| anyhow!("ответ token endpoint'а не JSON: {e}"))?;
        self.oauth_pending_remove(&state)?;
        Ok((kind.to_string(), tok))
    }

    pub async fn oauth_finish(&self, f: OauthFinish) -> Result<Account> {
        let (kind, tok) = self.oauth_exchange(&f.state, &f.code).await?;
        if kind == "codex" {
            let (auth_json, email, plan) = codex_auth_json(&tok, now_secs())?;
            let label = f.label.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| match (&email, &plan) { (Some(e), Some(pl)) => format!("codex · {e} ({pl})"), (Some(e), None) => format!("codex · {e}"), _ => "codex · вход OAuth".into() });
            return self.account_create(AccountCreate { provider: "codex".into(), kind: "subscription".into(), label, env: HashMap::new(), files: HashMap::from([("auth.json".to_string(), auth_json)]) });
        }
        let creds = credentials_from_token(&tok)?;
        let label = f.label.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| match tok.pointer("/account/email_address").and_then(|v| v.as_str()) { Some(e) => format!("claude · {e}"), None => "claude · вход OAuth".into() });
        self.account_create(AccountCreate { provider: "claude".into(), kind: "subscription".into(), label, env: HashMap::new(), files: HashMap::from([(".credentials.json".to_string(), creds)]) })
    }
}

#[cfg(test)]
mod oauth_tests {
    use super::*;
    #[test]
    fn oauth_links_and_callback_parsing() {
        let u = codex_authorize_url("v", "st");
        assert!(u.starts_with("https://auth.openai.com/oauth/authorize?client_id=app_EMoamEEZ73f0CkXaXp7hrann&response_type=code&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback&scope=openid+email+profile+offline_access&state=st&code_challenge="), "{u}");
        assert!(u.ends_with("&code_challenge_method=S256&prompt=login&id_token_add_organizations=true&codex_cli_simplified_flow=true"));
        let c = claude_authorize_url_cb("v", "st");
        assert!(c.contains("redirect_uri=http%3A%2F%2Flocalhost%3A54545%2Fcallback") && c.starts_with("https://claude.com/cai/oauth/authorize?code=true&client_id="), "{c}");
        assert_eq!(parse_callback_input("http://localhost:1455/auth/callback?code=abc&state=st", "st").unwrap(), ("abc".into(), "st".into()));
        assert!(parse_callback_input("http://localhost:1455/auth/callback?code=abc&state=other", "st").is_err());
        assert!(parse_callback_input("http://localhost:1455/auth/callback?error=access_denied", "st").is_err());
        assert_eq!(parse_callback_input("code123#st2", "st").unwrap(), ("code123".into(), "st2".into()));
        assert_eq!(parse_callback_input("bare", "st").unwrap(), ("bare".into(), "st".into()));
    }

    #[test]
    fn codex_device_auth_json() {
        // id_token с claims как у OpenAI: email + https://api.openai.com/auth
        let payload = serde_json::to_vec(&json!({ "email": "u@example.com", "https://api.openai.com/auth": { "chatgpt_account_id": "acc-1", "chatgpt_plan_type": "plus" } })).unwrap();
        let jwt = format!("eyJhbGciOiJSUzI1NiJ9.{}.sig", b64url(&payload));
        let (email, acc, plan) = codex_id_claims(&jwt);
        assert_eq!((email.as_deref(), acc.as_deref(), plan.as_deref()), (Some("u@example.com"), Some("acc-1"), Some("plus")));
        let (auth, email, plan) = codex_auth_json(&json!({ "id_token": jwt, "access_token": "at", "refresh_token": "rt" }), 1_790_000_000).unwrap();
        let v: Value = serde_json::from_str(&auth).unwrap();
        assert_eq!(v["auth_mode"], "chatgpt"); assert!(v["OPENAI_API_KEY"].is_null());
        assert_eq!(v["tokens"]["access_token"], "at"); assert_eq!(v["tokens"]["account_id"], "acc-1"); assert!(v["tokens"]["id_token"].as_str().unwrap().starts_with("eyJ"));
        assert_eq!(v["last_refresh"], "2026-09-21T14:13:20Z"); assert_eq!(email.as_deref(), Some("u@example.com")); assert_eq!(plan.as_deref(), Some("plus"));
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z"); assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(b64url_decode("aGVsbG8").unwrap(), b"hello");
    }

    #[test]
    fn pkce_url_and_credentials() {
        assert_eq!(b64url(b"hello"), "aGVsbG8");
        let url = oauth_authorize_url("verifier-xyz", "st1");
        assert!(url.starts_with("https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=org%3Acreate_api_key+user%3Aprofile"), "{url}");
        assert!(url.contains("&code_challenge_method=S256&state=st1"));
        let c = credentials_from_token(&json!({ "access_token": "sk-ant-oat01-x", "refresh_token": "sk-ant-ort01-y", "expires_in": 3600, "scope": "user:inference user:profile" })).unwrap();
        let v: Value = serde_json::from_str(&c).unwrap();
        assert_eq!(v["claudeAiOauth"]["accessToken"], "sk-ant-oat01-x");
        assert_eq!(v["claudeAiOauth"]["scopes"][1], "user:profile");
        assert!(v["claudeAiOauth"]["expiresAt"].as_u64().unwrap() > 1_700_000_000_000);
    }
}
