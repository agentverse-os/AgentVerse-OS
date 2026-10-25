//! Agent accounts (ADR-006, decision 6): a registry in the core, materialization in the workspace at session start, pinning.
//! Three paths to models: (A) proxy/gate — `*_BASE_URL` variables and a token; (B) the CLIs' own subscriptions — configuration
//! directory files (`CLAUDE_CONFIG_DIR/.credentials.json`, `CODEX_HOME/auth.json`) taken from a workspace where the owner has
//! already signed in; (C) API keys — `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`. Metadata — kv `accounts.list`, secrets — scope
//! `account:<id>` (variables by name, files as `file:<name>`), the project default account — kv `accounts.default.<project>`.
use crate::service::Service;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const LIST_KEY: &str = "accounts.list";
/// Where account files are placed in the workspace (owner
/// coder).
pub const WS_DIR: &str = "/home/coder/.cloudos/accounts";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    /// `claude` | `codex` | `anthropic` | `openai` | `openrouter` | … — whose access
    /// this is
    pub provider: String,
    /// `subscription` (CLI files) | `api_key` | `proxy` (address and token, incl. the
    /// project gate)
    pub kind: String,
    pub label: String,
    pub created_at: String,
    #[serde(default)]
    pub env_keys: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub imported_from: Option<String>,
    /// Antigravity (§9.11): timestamp of the agy home directory files in the archive — to take the fresher copy from the workspace
    #[serde(default)]
    pub stamp: u64,
    /// Antigravity: native models (`agy models` under the account's home directory) — the list for the
    /// composer
    #[serde(default)]
    pub models: Vec<String>,
    /// In the auto-switch pool (docs/agents-ux.md §9.7): on a limit hit the session moves to the next pool account of the same provider
    #[serde(default)]
    pub pool: bool,
}

/// Account edit: name and the pool flag
#[derive(Debug, Deserialize, Default)]
pub struct AccountPatch {
    pub label: Option<String>,
    pub pool: Option<bool>,
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
    /// `claude` | `codex` — take the sign-in files from the project workspace; `gate` — copy the variables of the
    /// project's llm grant
    pub provider: String,
    pub project: String,
    pub label: Option<String>,
    /// Configuration directory in the workspace signed in via `claude auth login` / `codex login
    /// --device-auth` (the sign-in terminal from "Agents"); without it — the regular ~/.claude and ~/.codex
    #[serde(default)]
    pub dir: Option<String>,
}

/// What is left out of the agy home directory archive (§9.11): logs, caches, conversations, crashes, bundled plugins,
/// updates, databases.
pub const AGY_EXCLUDES: &[&str] = &[".gemini/antigravity-cli/log", ".gemini/antigravity-cli/cache", ".gemini/antigravity-cli/conversations", ".gemini/antigravity-cli/crashes", ".gemini/antigravity-cli/builtin", ".gemini/antigravity-cli/updater", ".gemini/antigravity-cli/brain", ".gemini/antigravity-cli/implicit", ".gemini/config", "*.db", "*.db-wal", "*.db-shm", "*.log"];

/// Google link from the agy terminal text (§9.11): the TUI draws it wrapped — join from `https://accounts.google.com/` up
/// to an empty line or a hint ("Copy and paste", "If you aren", "→"), removing all spaces and line breaks.
pub fn agy_link_from(text: &str) -> Option<String> {
    let i = text.rfind("https://accounts.google.com/")?;
    let tail = &text[i..];
    let mut end = tail.len();
    for marker in ["\n\n", "Copy and paste", "If you aren", "→", "Click here"] { if let Some(j) = tail.find(marker) { if j < end { end = j; } } }
    let raw: String = tail[..end].chars().filter(|c| !c.is_whitespace()).collect();
    if raw.contains("redirect_uri") && raw.contains("state=") { Some(raw) } else { None }
}

/// Parsing `agy models`: lines "id<TAB>name"; without a sign-in the CLI answers "Please sign in" — the
/// list is empty.
pub fn parse_agy_models(out: &str) -> Vec<String> {
    out.lines().filter_map(|l| { let mut it = l.split('\t'); let id = it.next()?.trim(); let name = it.next()?; if id.is_empty() || name.trim().is_empty() || id.contains(' ') { None } else { Some(id.to_string()) } }).collect()
}

/// Variables the project gate sets via env.sh: a subscription or key account must unset them, otherwise the CLI goes to the
/// gate.
fn gate_vars(provider: &str) -> &'static [&'static str] {
    match provider {
        "claude" | "anthropic" => &["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"],
        "codex" | "openai" => &["OPENAI_BASE_URL", "OPENAI_API_KEY"],
        "gemini" | "google" | "antigravity" => &["GEMINI_BASE_URL", "GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL"],
        _ => &[],
    }
}

/// What the agent process gets: the account variables (+ the configuration directory for subscriptions) and
/// the list of ones to unset.
pub fn session_env(a: &Account, env: &HashMap<String, String>) -> (HashMap<String, String>, Vec<String>) {
    let mut out = env.clone();
    if !a.files.is_empty() {
        let dir = format!("{WS_DIR}/{}", a.id);
        match a.provider.as_str() {
            "claude" => { out.insert("CLAUDE_CONFIG_DIR".into(), dir); }
            "codex" => { out.insert("CODEX_HOME".into(), dir); }
            // Antigravity (§9.11): all agy state lives under the home directory — the account directory is the process
            // HOME
            "antigravity" => { out.insert("HOME".into(), dir); }
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

    pub(crate) fn save_accounts(&self, list: &[Account]) -> Result<()> {
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
        let a = Account { id, provider: c.provider, kind: c.kind, label: c.label.trim().to_string(), created_at: crate::store::now(), env_keys: c.env.keys().cloned().collect(), files: c.files.keys().cloned().collect(), imported_from: None, pool: false, stamp: 0, models: Vec::new() };
        let mut list = self.accounts()?;
        list.push(a.clone());
        self.save_accounts(&list)?;
        self.store.event("account", &a.id, &format!("добавлен аккаунт «{}» ({} · {})", a.label, a.provider, a.kind))?;
        Ok(a)
    }

    /// Take the sign-in from the project workspace (the owner already signed in there) or copy the project gate variables as a
    /// proxy account.
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
            "antigravity" => {
                let base = dir.clone().ok_or_else(|| anyhow!("импорт Antigravity: нужен каталог входа (HOME, в который вошли через agy)"))?;
                let (tgz, stamp, models) = self.agy_home_pack(&inst, &base).await?;
                let mut a = self.account_create(AccountCreate { provider: "antigravity".into(), kind: "subscription".into(), label: i.label.clone().unwrap_or_else(|| "antigravity · вход Google".into()), env: HashMap::new(), files: HashMap::from([("home.tgz".to_string(), tgz)]) })?;
                a.imported_from = Some(i.project.clone()); a.stamp = stamp; a.models = models;
                let mut list = self.accounts()?;
                if let Some(x) = list.iter_mut().find(|x| x.id == a.id) { x.imported_from = a.imported_from.clone(); x.stamp = stamp; x.models = a.models.clone(); }
                self.save_accounts(&list)?;
                return Ok(a);
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
            other => bail!("импорт: claude | codex | antigravity | gate, получено {other}"),
        };
        let label = i.label.unwrap_or_else(|| match (kind, dir.is_some()) { ("subscription", true) => format!("{} · вход OAuth", provider), ("subscription", false) => format!("{} · вход из {}", provider, i.project), _ => format!("gate проекта {}", i.project) });
        let mut a = self.account_create(AccountCreate { provider, kind: kind.into(), label, env, files })?;
        a.imported_from = Some(i.project.clone());
        let mut list = self.accounts()?;
        if let Some(x) = list.iter_mut().find(|x| x.id == a.id) { x.imported_from = a.imported_from.clone(); }
        self.save_accounts(&list)?;
        Ok(a)
    }

    pub fn account_update(&self, id: &str, p: AccountPatch) -> Result<Account> {
        let mut list = self.accounts()?;
        let a = list.iter_mut().find(|a| a.id == id).ok_or_else(|| anyhow!("аккаунт {id} не найден"))?;
        if let Some(l) = p.label { let l = l.trim().to_string(); if l.is_empty() { bail!("аккаунт требует название"); } a.label = l; }
        if let Some(pool) = p.pool { a.pool = pool; }
        let out = a.clone();
        self.save_accounts(&list)?;
        self.store.event("account", id, &format!("аккаунт «{}»: {}", out.label, if out.pool { "в пуле автопереключения" } else { "вне пула" }))?;
        Ok(out)
    }

    pub fn account_delete(&self, id: &str) -> Result<()> {
        let mut list = self.accounts()?;
        let before = list.len();
        list.retain(|a| a.id != id);
        if list.len() == before { bail!("аккаунт {id} не найден"); }
        self.save_accounts(&list)?;
        self.store.delete_secrets(&format!("account:{id}"))?;
        self.unblock_account(id);
        // unset as the default at the projects
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

    /// Materialize the account in the project workspace and return the environment for the
    /// session start.
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
            if a.provider == "antigravity" {
                // §9.11: the agy home directory archive — first the fresher copy from running workspaces, then the unpack and
                // the shared ~/.gemini/config as a symlink (rules, MCP, skills — one set per workspace)
                let a = self.freshest_agy_home(&a).await;
                let tgz = self.store.get_secret(&scope, "file:home.tgz")?.ok_or_else(|| anyhow!("у аккаунта {} нет архива home.tgz", a.id))?;
                let bytes = b64std_decode(tgz.trim()).ok_or_else(|| anyhow!("архив аккаунта {}: не base64", a.id))?;
                self.incus.file_push(&inst, &format!("{dir}/home.tgz"), &bytes, 0o600).await?;
                self.incus.exec(&inst, &["sh", "-c", &format!("mkdir -p {dir} && tar -xzf {dir}/home.tgz -C {dir} && rm -f {dir}/home.tgz && mkdir -p {dir}/.gemini /home/coder/.gemini/config && ln -sfn /home/coder/.gemini/config {dir}/.gemini/config && chown -R 1000:1000 {WS_DIR} && chmod 700 {WS_DIR} {dir}")]).await?;
                return Ok(session_env(&a, &env));
            }
            for name in &a.files {
                let content = self.freshest_login_file(&a, name).await?;
                self.incus.file_push(&inst, &format!("{dir}/{name}"), content.as_bytes(), 0o600).await?;
            }
            self.incus.exec(&inst, &["sh", "-c", &format!("chown -R 1000:1000 {WS_DIR} && chmod 700 {WS_DIR} {dir}")]).await?;
        }
        Ok(session_env(&a, &env))
    }

    /// Names of projects with a running workspace (for a sign-in terminal without an
    /// explicit project)
    pub async fn running_project_names(&self) -> Vec<String> {
        let Ok(instances) = self.incus.instances().await else { return Vec::new() };
        let running: std::collections::HashSet<String> = instances.into_iter().filter(|i| i.status == "Running").map(|i| i.name).collect();
        let mut out = Vec::new();
        for st in self.store.list_projects().unwrap_or_default() { if running.contains(&self.workspace_instance(&st.name).await) { out.push(st.name.clone()); } }
        out
    }

    /// Antigravity sign-in watcher (§9.11): every 6 s checks `agy models` under the sign-in directory; signed in — the
    /// account is imported, the sign-in terminal is killed, the sign-in directory removed; the terminal was closed by the
    /// user — cancel; 20 minutes — timeout.
    pub async fn agy_login_watch(&self, project: String, dir: String, pty: String) {
        let key = format!("accounts.agylogin.{pty}");
        let put = |st: Value| { let _ = self.store.put_kv(&key, &st); };
        put(json!({ "status": "waiting", "project": project, "dir": dir, "pty": pty, "started": now_secs() }));
        let inst = self.workspace_instance(&project).await;
        let client = self.agentd_client(&project).await.ok();
        for _ in 0..200 {
            tokio::time::sleep(std::time::Duration::from_secs(6)).await;
            if let Some(c) = &client {
                match c.get(&format!("/v1/ptys/{pty}")).await {
                    Ok(v) if v.get("alive").and_then(|a| a.as_bool()) == Some(false) => { put(json!({ "status": "cancelled", "project": project, "dir": dir, "pty": pty })); let _ = self.incus.exec(&inst, &["rm", "-rf", &dir]).await; return; }
                    Err(_) => { put(json!({ "status": "cancelled", "project": project, "dir": dir, "pty": pty })); let _ = self.incus.exec(&inst, &["rm", "-rf", &dir]).await; return; }
                    _ => {}
                }
            }
            if self.agy_signed_in(&inst, &dir).await.is_none() { continue }
            match self.account_import(AccountImport { provider: "antigravity".into(), project: project.clone(), label: None, dir: Some(dir.clone()) }).await {
                Ok(a) => {
                    put(json!({ "status": "done", "project": project, "dir": dir, "pty": pty, "account": a.id, "label": a.label, "models": a.models.len() }));
                    let _ = self.store.event("account", &a.id, &format!("вход Antigravity «{}» импортирован из {project}", a.label));
                    if let Some(c) = &client { let _ = c.delete(&format!("/v1/ptys/{pty}")).await; }
                    let _ = self.incus.exec(&inst, &["rm", "-rf", &dir]).await;
                }
                Err(e) => { tracing::warn!("вход Antigravity в {project}: {e:#}"); put(json!({ "status": "error", "project": project, "dir": dir, "pty": pty, "error": format!("{e:#}") })); }
            }
            return;
        }
        put(json!({ "status": "timeout", "project": project, "dir": dir, "pty": pty }));
        if let Some(c) = &client { let _ = c.delete(&format!("/v1/ptys/{pty}")).await; }
        let _ = self.incus.exec(&inst, &["rm", "-rf", &dir]).await;
    }

    /// Antigravity (§9.11): is agy signed in under home directory `home` in the workspace (`agy models` answers "Please sign in" without
    /// a sign-in). Returns the list of native models if signed in.
    pub async fn agy_signed_in(&self, inst: &str, home: &str) -> Option<Vec<String>> {
        let out = self.incus.exec(inst, &["su", "-", "coder", "-c", &format!("HOME={home} timeout 60 agy models </dev/null 2>&1")]).await.ok()?;
        let models = parse_agy_models(&out);
        if models.is_empty() { None } else { Some(models) }
    }

    /// Archive of the agy home directory without logs, caches, conversations and databases (base64 tgz), the timestamp of
    /// its files and the native models.
    pub async fn agy_home_pack(&self, inst: &str, home: &str) -> Result<(String, u64, Vec<String>)> {
        let models = self.agy_signed_in(inst, home).await.ok_or_else(|| anyhow!("в {home} нет входа Antigravity: agy models не отдаёт модели — войдите через Google в терминале"))?;
        let excl = AGY_EXCLUDES.iter().map(|e| format!("--exclude='{e}'")).collect::<Vec<_>>().join(" ");
        let tmp = format!("/tmp/agy-home-{}.tgz", now_secs());
        self.incus.exec(inst, &["sh", "-c", &format!("cd {home} && tar {excl} -czf {tmp} .gemini && chmod 600 {tmp}")]).await?;
        let bytes = self.incus.file_pull(inst, &tmp).await?;
        let _ = self.incus.exec(inst, &["rm", "-f", &tmp]).await;
        let stamp = self.agy_home_stamp(inst, home).await;
        Ok((b64std(&bytes), stamp, models))
    }

    /// Timestamp of the freshest file in the agy store (without logs and databases) — 0 if the
    /// directory is missing.
    pub async fn agy_home_stamp(&self, inst: &str, home: &str) -> u64 {
        self.incus.exec(inst, &["sh", "-c", &format!("find {home}/.gemini/antigravity-cli -maxdepth 1 -type f ! -name '*.db*' ! -name '*.log' ! -name '*.timestamp' -printf '%T@\n' 2>/dev/null | sort -n | tail -1")]).await.ok()
            .and_then(|o| o.trim().split('.').next().and_then(|t| t.parse::<u64>().ok())).unwrap_or(0)
    }

    /// A fresher copy of the agy home directory from a running workspace (tokens are refreshed inside the container) — into the
    /// store.
    pub async fn freshest_agy_home(&self, a: &Account) -> Account {
        let mut a = a.clone();
        let Ok(instances) = self.incus.instances().await else { return a };
        let dir = format!("{WS_DIR}/{}", a.id);
        for i in instances.into_iter().filter(|i| i.status == "Running" && i.name.starts_with("ws-")) {
            let stamp = self.agy_home_stamp(&i.name, &dir).await;
            if stamp <= a.stamp { continue }
            match self.agy_home_pack(&i.name, &dir).await {
                Ok((tgz, st, models)) => {
                    let scope = format!("account:{}", a.id);
                    if self.store.put_secret(&scope, "file:home.tgz", &tgz).is_ok() {
                        a.stamp = st; if !models.is_empty() { a.models = models; }
                        if let Ok(mut list) = self.accounts() { if let Some(x) = list.iter_mut().find(|x| x.id == a.id) { x.stamp = a.stamp; x.models = a.models.clone(); } let _ = self.save_accounts(&list); }
                        let _ = self.store.event("account", &a.id, &format!("вход Antigravity «{}» обновлён в {} — свежая копия забрана в хранилище", a.label, i.name));
                        tracing::info!("аккаунт {}: домашний каталог agy из {} новее копии в хранилище — принят", a.id, i.name);
                    }
                }
                Err(e) => tracing::warn!("аккаунт {}: свежая копия agy из {}: {e:#}", a.id, i.name),
            }
        }
        a
    }

    /// Account sign-in file — the freshest copy: from the store or from the account directory in a running workspace if the
    /// tokens there are newer (Codex and Claude Code refresh them themselves, inside the container, and get a new refresh_token;
    /// the store copy does not know about it, and unpacking the old copy at session start would wipe the new one). The core
    /// takes the fresh copy into the store.
    pub async fn freshest_login_file(&self, a: &Account, name: &str) -> Result<String> {
        let scope = format!("account:{}", a.id);
        let stored = self.store.get_secret(&scope, &format!("file:{name}"))?.ok_or_else(|| anyhow!("у аккаунта {} нет файла {name}", a.id))?;
        let mut best_stamp = serde_json::from_str::<Value>(&stored).map(|v| login_stamp(&a.provider, &v)).unwrap_or(0);
        let mut best: Option<(String, String)> = None;
        let Ok(instances) = self.incus.instances().await else { return Ok(stored) };
        for i in instances.into_iter().filter(|i| i.status == "Running" && i.name.starts_with("ws-")) {
            let Ok(b) = self.incus.file_pull(&i.name, &format!("{WS_DIR}/{}/{name}", a.id)).await else { continue };
            let Ok(text) = String::from_utf8(b) else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
            let stamp = login_stamp(&a.provider, &v);
            if stamp > best_stamp { best_stamp = stamp; best = Some((i.name.clone(), text)); }
        }
        let Some((inst, text)) = best else { return Ok(stored) };
        self.store.put_secret(&scope, &format!("file:{name}"), &text)?;
        let _ = self.store.event("account", &a.id, &format!("токены «{}» обновлены в {inst} — свежая копия {name} забрана в хранилище", a.label));
        tracing::info!("аккаунт {}: {name} из {inst} новее копии в хранилище — принят", a.id);
        Ok(text)
    }

    /// Account for a new session: an explicit one from the request body (`""` — no account), otherwise the project default; for
    /// Codex without a default — the freshest ChatGPT subscription if there is one: the gate for Codex is needed only for the
    /// proxy's OpenAI-compatible models, and the owner who signed in to ChatGPT expects the Codex chat to go through the
    /// subscription (Claude through the gate is left alone: Fable is there).
    pub async fn resolve_session_account(&self, project: &str, body: &mut Value) -> Result<()> {
        let explicit = body.get("account").and_then(|v| v.as_str()).map(str::to_string);
        let explicit_given = explicit.clone().filter(|s| !s.is_empty());
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
            if matches!(agent.as_deref(), Some("codex") | Some("antigravity")) && stored_account.is_none() {
                // Codex — a ChatGPT subscription, Antigravity (§9.11) — a Google sign-in: the newest account of the
                // provider, otherwise the gate
                let prov = agent.clone().unwrap_or_default();
                chosen = self.accounts()?.into_iter().filter(|a| a.provider == prov && a.kind == "subscription").max_by(|a, b| a.created_at.cmp(&b.created_at)).map(|a| a.id);
                if let Some(id) = &chosen {
                    tracing::info!("проект {project}: {prov} без умолчания аккаунта — подписка {id}");
                    // resuming an old session without an account: agentd applies the new environment only with
                    // reassign
                    if body.get("resume").is_some() { body["reassign"] = json!(true); }
                }
            }
        }
        let Some(mut id) = chosen else { return Ok(()) };
        // §9.7: the default account is blocked by a limit → a free one from the pool (an explicitly chosen account
        // is not substituted)
        if explicit_given.is_none() { if let Some((next, note)) = self.pool_substitute(&id).await {
            tracing::info!("проект {project}: {note}");
            id = next.id; body["note"] = json!(note);
            if body.get("resume").is_some() { body["reassign"] = json!(true); }
        } }
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
        Account { id: format!("{provider}-1"), provider: provider.into(), kind: kind.into(), label: "t".into(), created_at: String::new(), env_keys: env_keys.iter().map(|s| s.to_string()).collect(), files: files.iter().map(|s| s.to_string()).collect(), imported_from: None, pool: false, stamp: 0, models: Vec::new() }
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
    fn antigravity_home_and_models() {
        let (env, unset) = session_env(&acc("antigravity", "subscription", &["home.tgz"], &[]), &HashMap::from([("GEMINI_API_KEY".to_string(), "k".to_string())]));
        assert_eq!(env.get("HOME").map(String::as_str), Some("/home/coder/.cloudos/accounts/antigravity-1"));
        assert_eq!(unset, vec!["GEMINI_BASE_URL", "GOOGLE_GEMINI_BASE_URL"], "ключ задан окружением — снимаются остальные переменные gate");
        assert_eq!(parse_agy_models("Fetching available models...\ngemini-3.1-pro-high\tGemini 3.1 Pro (High)\ngemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n"), vec!["gemini-3.1-pro-high", "gemini-3.8-flash-low"]);
        assert!(parse_agy_models("Error: Please sign in to view available models. Launch the CLI without arguments to sign in.").is_empty());
        let txt = "Signing in...Your browser should open automatically. If not:\nhttps://accounts.google.com/o/oauth2/auth?access_type=offline&client_id=1\n071006060591-x.apps.googleusercontent.com&code_challenge=abc&redirect_uri=https%3A%2F%2Fantigravity.google%2Foauth-callback&state=HMnw\nCopy and paste the URL or click on the link below:\n→ Click here to authenticate\n";
        assert_eq!(agy_link_from(txt).unwrap(), "https://accounts.google.com/o/oauth2/auth?access_type=offline&client_id=1071006060591-x.apps.googleusercontent.com&code_challenge=abc&redirect_uri=https%3A%2F%2Fantigravity.google%2Foauth-callback&state=HMnw");
        assert!(agy_link_from("Select login method:\n> 1. Google OAuth\n").is_none());
        assert!(AGY_EXCLUDES.contains(&".gemini/config") && AGY_EXCLUDES.contains(&"*.db"));
        for sample in [&b""[..], b"a", b"ab", b"abc", b"\x00\xff\x10\x80tar"] { assert_eq!(b64std_decode(&b64std(sample)).unwrap(), sample); }
        assert_eq!(b64std(b"hello"), "aGVsbG8=");
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


// ---- Claude sign-in by link and code (OAuth with PKCE, as Claude Code itself does) ---- The core builds an
// authorization link with a redirect to the code page (`platform.claude.com/oauth/code/callback`): the owner opens it on any
// device, signs in, copies the shown `code#state` and pastes it into the UI; the core exchanges the code for tokens at
// `platform.claude.com/v1/oauth/token` and saves `.credentials.json` in the Claude Code format as a subscription account.
pub const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const CLAUDE_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
pub const CLAUDE_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const CLAUDE_MANUAL_REDIRECT: &str = "https://platform.claude.com/oauth/code/callback";
pub const CLAUDE_SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload user:plugins";
const OAUTH_PENDING_KEY: &str = "accounts.oauth.pending";
// ---- ChatGPT (Codex) sign-in by link: device-code, like `codex login --device-auth`
// ---- The core asks auth.openai.com for a one-time code, the owner opens the device page on any device and enters the code; the
// core polls the issuance, exchanges the authorization_code for tokens (the device redirect, PKCE is provided by the server) and
// saves `auth.json` in the Codex CLI format (auth_mode chatgpt, tokens.id_token/access_token/refresh_token/account_id) as a
// subscription account.
pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const CODEX_DEVICE_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
pub const CODEX_DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
pub const CODEX_DEVICE_VERIFY_URL: &str = "https://auth.openai.com/codex/device";
pub const CODEX_DEVICE_REDIRECT: &str = "https://auth.openai.com/deviceauth/callback";
pub const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
// ---- OAuth links: sign in in the browser, then the address from the address bar
// (localhost:<port>/…?code=…&state=…) ----
pub const CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const CODEX_OAUTH_REDIRECT: &str = "http://localhost:1455/auth/callback";
pub const CLAUDE_CB_REDIRECT: &str = "http://localhost:54545/callback";
/// The device code lives 15 minutes (auth.openai.com returns expires_at; we keep our own
/// limit too).
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

/// Useful OpenAI id_token claims: email, chatgpt_account_id, chatgpt_plan_type (as in Codex CLI `token_data.rs`).
pub fn codex_id_claims(jwt: &str) -> (Option<String>, Option<String>, Option<String>) {
    let payload = jwt.split('.').nth(1).and_then(b64url_decode).and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
    let email = payload.get("email").and_then(|v| v.as_str()).or_else(|| payload.pointer("/https:~1~1api.openai.com~1profile/email").and_then(|v| v.as_str())).map(str::to_string);
    let auth = payload.get("https://api.openai.com/auth");
    let account = auth.and_then(|a| a.get("chatgpt_account_id")).and_then(|v| v.as_str()).map(str::to_string);
    let plan = auth.and_then(|a| a.get("chatgpt_plan_type")).and_then(|v| v.as_str()).map(str::to_string);
    (email, account, plan)
}

/// RFC 3339 UTC from unix seconds (the core has no chrono):
/// `2026-09-30T10:20:30Z`.
pub fn rfc3339_utc(secs: u64) -> String {
    let days = secs / 86400; let rem = secs % 86400;
    // civil_from_days (Howard Hinnant)
    let z = days as i64 + 719468; let era = z.div_euclid(146097); let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1; let m = if mp < 10 { mp + 3 } else { mp - 9 }; let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Freshness mark of a sign-in file: Codex — `last_refresh` (RFC 3339) or the access token `exp`, Claude —
/// `claudeAiOauth.expiresAt` (ms); greater is newer. Without recognized fields — 0.
pub fn login_stamp(provider: &str, v: &Value) -> u64 {
    match provider {
        "codex" | "openai" => {
            let last = v.get("last_refresh").and_then(|s| s.as_str()).and_then(parse_rfc3339_utc).unwrap_or(0);
            let exp = v.pointer("/tokens/access_token").and_then(|s| s.as_str()).map(|t| jwt_claims(t).get("exp").and_then(|e| e.as_u64()).unwrap_or(0)).unwrap_or(0);
            last.max(exp)
        }
        "claude" | "anthropic" => v.pointer("/claudeAiOauth/expiresAt").and_then(|e| e.as_u64()).map(|ms| ms / 1000).unwrap_or(0),
        _ => 0,
    }
}

/// Unix seconds from RFC 3339 UTC (`2026-09-30T10:20:30Z`, fractional seconds and a `+00:00` offset allowed);
/// otherwise None.
pub fn parse_rfc3339_utc(s: &str) -> Option<u64> {
    let s = s.trim(); if s.len() < 19 { return None; }
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, m, d, hh, mm, ss) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut rest = &s[19..];
    if rest.starts_with('.') { let end = rest[1..].find(|c: char| !c.is_ascii_digit()).map(|i| i + 1).unwrap_or(rest.len()); rest = &rest[end..]; }
    let offset = match rest { "" | "Z" | "z" => 0, r => { let sign = if r.starts_with('-') { -1 } else { 1 }; let r = r.trim_start_matches(['+', '-']); let (oh, om) = r.split_once(':')?; sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60) } };
    // days_from_civil (Howard Hinnant)
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400); let yoe = y.rem_euclid(400); let doy = (153 * m + 2) / 5 + d - 1; let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + hh * 3600 + mm * 60 + ss - offset;
    u64::try_from(secs).ok()
}

/// JWT payload without signature verification (like codex_id_claims, but any fields).
fn jwt_claims(jwt: &str) -> Value {
    let Some(p) = jwt.split('.').nth(1) else { return Value::Null };
    b64url_decode(p).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

/// Codex CLI `auth.json` from the token endpoint answer (the `AuthDotJson` format of codex-rs/login: auth_mode, OPENAI_API_KEY, tokens,
/// last_refresh).
pub fn codex_auth_json(tok: &Value, now_secs: u64) -> Result<(String, Option<String>, Option<String>)> {
    let id_token = tok.get("id_token").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("в ответе нет id_token"))?;
    let access = tok.get("access_token").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("в ответе нет access_token"))?;
    let refresh = tok.get("refresh_token").and_then(|v| v.as_str()).unwrap_or("");
    let (email, account_id, plan) = codex_id_claims(id_token);
    let o = json!({ "auth_mode": "chatgpt", "OPENAI_API_KEY": Value::Null, "tokens": { "id_token": id_token, "access_token": access, "refresh_token": refresh, "account_id": account_id }, "last_refresh": rfc3339_utc(now_secs) });
    Ok((serde_json::to_string(&o)?, email, plan))
}

/// Standard base64 (the agy home directory archive in the store)
pub fn b64std(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((b.len() + 2) / 3 * 4);
    for chunk in b.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char); out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}
pub fn b64std_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> { Some(match c { b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52, b'+' => 62, b'/' => 63, _ => return None } as u32) };
    let bytes: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32; for (i, c) in chunk.iter().enumerate() { n |= val(*c)? << (18 - 6 * i); }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 { out.push((n >> 8) as u8); }
        if chunk.len() > 3 { out.push(n as u8); }
    }
    Some(out)
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

/// Claude authorization link with PKCE; the verifier waits for the code under its `state`
/// (for at most an hour).
pub fn oauth_authorize_url(verifier: &str, state: &str) -> String {
    use sha2::{Digest, Sha256};
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    format!("{CLAUDE_AUTHORIZE_URL}?code=true&client_id={CLAUDE_CLIENT_ID}&response_type=code&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&state={state}",
        urlencoding::encode(CLAUDE_MANUAL_REDIRECT), urlencoding::encode(CLAUDE_SCOPES).replace("%20", "+"))
}

/// Codex OAuth link with PKCE and a return to localhost:1455 (parameters as in Codex CLI).
pub fn codex_authorize_url(verifier: &str, state: &str) -> String {
    use sha2::{Digest, Sha256};
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    format!("{CODEX_AUTHORIZE_URL}?client_id={CODEX_CLIENT_ID}&response_type=code&redirect_uri={}&scope={}&state={state}&code_challenge={challenge}&code_challenge_method=S256&prompt=login&id_token_add_organizations=true&codex_cli_simplified_flow=true",
        urlencoding::encode(CODEX_OAUTH_REDIRECT), urlencoding::encode("openid email profile offline_access").replace("%20", "+"))
}

/// Claude OAuth link with a return to localhost:54545, instead of the code page.
pub fn claude_authorize_url_cb(verifier: &str, state: &str) -> String {
    use sha2::{Digest, Sha256};
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    format!("{CLAUDE_AUTHORIZE_URL}?code=true&client_id={CLAUDE_CLIENT_ID}&response_type=code&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&state={state}",
        urlencoding::encode(CLAUDE_CB_REDIRECT), urlencoding::encode(CLAUDE_SCOPES).replace("%20", "+"))
}

/// Parsing what the owner pasted: the address after sign-in (`http://localhost:…/…?code=…&state=…`), `code#state` or a bare
/// code. Returns (code, state); the state from the address must match the expected one.
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

/// `.credentials.json` in the Claude Code format from the token endpoint
/// answer.
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
    /// the code from the page (a whole `code#state` is
    /// accepted too)
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

    /// Sign-in link: Claude — authorization with PKCE and a code from the page; Codex — a one-time device code
    /// (device-code).
    pub async fn oauth_start(&self, provider: &str) -> Result<Value> { self.oauth_start_for(provider, None).await }

    /// Antigravity sign-in by link (§9.11): the same Google OAuth of agy itself, but the terminal is hidden — the core starts
    /// agy under the sign-in directory, picks "Google OAuth", pulls the link out of the terminal text; the owner pastes the
    /// code from the page into the UI, the core hands it to agy and waits until the sign-in watcher imports the account.
    /// Pending holds the terminal and the directory.
    pub async fn oauth_start_agy(&self, project: &str) -> Result<Value> {
        let stamp = now_secs();
        let dir = format!("/home/coder/.cloudos/logins/antigravity-{stamp}");
        let cmd = format!("mkdir -p {dir} && export HOME={dir} && agy; echo; echo \"[выход: $?]\"; sleep 1");
        let client = self.agentd_client(project).await?;
        let pty = client.pty_create(&json!({ "cmd": ["bash", "-lic", cmd], "cwd": "/home/coder", "title": "вход Antigravity (по ссылке)", "cols": 200, "rows": 50 })).await?;
        let pty_id = pty.get("id").and_then(|i| i.as_str()).ok_or_else(|| anyhow!("терминал входа не создан"))?.to_string();
        { let (s2, pr, d, p) = (self.clone(), project.to_string(), dir.clone(), pty_id.clone()); tokio::spawn(async move { s2.agy_login_watch(pr, d, p).await; }); }
        // the "Select login method" menu → Enter (the first item is Google OAuth) → the link in the
        // terminal text
        let mut url: Option<String> = None; let mut pressed = false;
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            let text = client.get(&format!("/v1/ptys/{pty_id}/text")).await.ok().and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string)).unwrap_or_default();
            if !pressed && (text.contains("Select login method") || text.contains("Google OAuth")) { let _ = client.post(&format!("/v1/ptys/{pty_id}/input"), &json!({ "data": "\r" })).await; pressed = true; continue }
            if let Some(u) = agy_link_from(&text) { url = Some(u); break }
            if text.contains("[выход:") { break }
        }
        let Some(url) = url else { let _ = client.delete(&format!("/v1/ptys/{pty_id}")).await; bail!("agy не напечатал ссылку входа — проверьте, что Antigravity установлен в {project}"); };
        let state = rand_b64(24);
        self.oauth_pending_put(&state, json!({ "provider": "antigravity", "project": project, "pty": pty_id, "dir": dir, "at": stamp }))?;
        Ok(json!({ "provider": "antigravity", "state": state, "url": url, "flow": "code", "hint": "agy-code", "pty": pty_id, "project": project }))
    }

    /// Authorization code from the Google page → into the hidden agy terminal; wait for the watcher to import the
    /// account (up to 2 minutes).
    pub async fn oauth_finish_agy(&self, p: &Value, state: &str, code: &str) -> Result<Account> {
        let project = p.get("project").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let pty = p.get("pty").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let code = code.trim();
        // the callback address can be pasted from the page too — then take the
        // code from it
        let code = if code.starts_with("http") { parse_callback_input(code, state).map(|(c, _)| c).unwrap_or_else(|_| code.to_string()) } else { code.to_string() };
        if code.is_empty() { bail!("вставьте код авторизации со страницы Google"); }
        let client = self.agentd_client(&project).await?;
        let st = client.get(&format!("/v1/ptys/{pty}")).await.map_err(|_| anyhow!("терминал входа уже закрыт — получите ссылку заново"))?;
        if st.get("alive").and_then(|a| a.as_bool()) == Some(false) { bail!("терминал входа уже закрыт — получите ссылку заново"); }
        client.post(&format!("/v1/ptys/{pty}/input"), &json!({ "data": format!("{code}\r") })).await?;
        let key = format!("accounts.agylogin.{pty}");
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let v = self.store.get_kv(&key)?.unwrap_or(json!({}));
            match v.get("status").and_then(|s| s.as_str()).unwrap_or("waiting") {
                "done" => { self.oauth_pending_remove(state)?; let id = v.get("account").and_then(|a| a.as_str()).unwrap_or("").to_string(); return self.account(&id); }
                "error" => { self.oauth_pending_remove(state)?; bail!("импорт входа: {}", v.get("error").and_then(|e| e.as_str()).unwrap_or("ошибка")); }
                "cancelled" | "timeout" => { self.oauth_pending_remove(state)?; bail!("вход не состоялся (терминал закрыт или время вышло) — получите ссылку заново"); }
                _ => {}
            }
        }
        bail!("вход не подтвердился за 2 минуты — проверьте код и попробуйте ещё раз")
    }

    /// The same, marked "for the gate": the result goes not into a core account but as a sign-in file into the proxy of the
    /// `gate` app (gateauth.rs).
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

    /// Pending sign-in by state (provider, gate, …) — if not expired
    /// yet.
    pub fn oauth_pending(&self, state: &str) -> Result<Option<Value>> {
        Ok(self.store.get_kv(OAUTH_PENDING_KEY)?.unwrap_or(json!({})).get(state).cloned())
    }

    /// One Codex device-code poll: `Wait` — not confirmed, `Done(tok)` — tokens (pending removed), `Error` — expired or refused (pending
    /// removed).
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

    /// Sign-in by link state (Codex, a core account): `wait` — not confirmed yet, `ok` — the account is created, `error` — expired or
    /// refused.
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

    /// Exchange of the code for tokens on a pending sign-in: Claude from the page (`code#state`), Claude/Codex by OAuth
    /// link (the address from the browser). Returns (token provider: claude | codex, the token endpoint answer); pending
    /// is removed.
    pub async fn oauth_exchange(&self, state: &str, raw: &str) -> Result<(String, Value)> {
        // `code#state` from the page carries its own state — that is the
        // pending key
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
        if let Some(p) = self.oauth_pending(&f.state)? { if p.get("provider").and_then(|v| v.as_str()) == Some("antigravity") { return self.oauth_finish_agy(&p, &f.state, &f.code).await; } }
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
    fn login_stamp_prefers_newer_copy() {
        assert_eq!(parse_rfc3339_utc("2026-09-30T10:20:30Z"), Some(1_790_763_630));
        assert_eq!(parse_rfc3339_utc("2026-09-30T10:20:30.123456+00:00"), Some(1_790_763_630));
        assert_eq!(parse_rfc3339_utc("2026-09-30T12:20:30+02:00"), Some(1_790_763_630));
        assert_eq!(parse_rfc3339_utc("вчера"), None);
        assert_eq!(rfc3339_utc(1_790_763_630), "2026-09-30T10:20:30Z");
        let old = json!({ "tokens": { "access_token": "x" }, "last_refresh": "2026-09-29T22:07:37Z" });
        let newer = json!({ "tokens": { "access_token": "x" }, "last_refresh": "2026-10-09T21:22:00Z" });
        assert!(login_stamp("codex", &newer) > login_stamp("codex", &old));
        // without last_refresh the access token exp
        // decides
        let jwt = format!("h.{}.s", b64url(br#"{"exp":1791583657}"#));
        assert_eq!(login_stamp("codex", &json!({ "tokens": { "access_token": jwt } })), 1_791_583_657);
        assert_eq!(login_stamp("claude", &json!({ "claudeAiOauth": { "expiresAt": 1_791_583_657_000u64 } })), 1_791_583_657);
        assert_eq!(login_stamp("gate", &json!({})), 0);
    }

    fn codex_device_auth_json() {
        // an id_token with claims as at OpenAI: email +
        // https://api.openai.com/auth
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
