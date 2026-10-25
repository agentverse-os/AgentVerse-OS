//! Providers through the gate: OAuth sign-ins into the llm provider proxy (Antigravity, Kimi, xAI Grok, ChatGPT/Codex, Claude and plugins
//! like Gemini CLI / GitHub Copilot) from "Agents" → Settings → Accounts (docs/agents-ux.md §9.1). The core stores nothing
//! itself: it calls the Management API of the proxy that grants the `llm` capability to projects — a local app
//! (the panel key is known to the core) or an external one (`llm-external` with an optional management key).
//! Sign-in files live at the proxy, models appear in its `/v1/models` and are available to all projects through
//! `http://llm.gate`. The sign-in link opens on any device: device-code providers (Kimi, xAI) are confirmed with a code,
//! callback providers (Antigravity, Codex, Claude) redirect the browser to `http://localhost:<port>/…` — the page will not
//! open, the owner pastes that address back, the core hands it to the proxy (`POST /v0/management/oauth-callback`), as
//! the proxy's own panel does.
use crate::model::AppKind;
use crate::service::Service;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

/// Where to go for the Management API: the app, base, panel key and the API key for `/v1/models`. Proxies
/// without `/v0/management/quota` (404): address → when marked; retried after 10 minutes.
fn quota_flags() -> &'static std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>> {
    static F: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>> = std::sync::OnceLock::new();
    F.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}
fn quota_unsupported(base: &str) -> bool { quota_flags().lock().unwrap().get(base).map(|t| t.elapsed() < std::time::Duration::from_secs(600)).unwrap_or(false) }
fn mark_quota_unsupported(base: &str) { quota_flags().lock().unwrap().insert(base.to_string(), std::time::Instant::now()); }

#[derive(Debug, Clone)]
pub struct MgmtTarget {
    pub app: String,
    pub title: String,
    pub kind: &'static str,
    pub base: String,
    pub key: Option<String>,
    pub api_key: Option<String>,
}

/// Built-in proxy providers: the route `GET /v0/management/<id>-auth-url`, the flow type and the address
/// the provider redirects the browser to after sign-in (it is pasted back). Plugins are fetched from
/// `/v0/management/plugins`.
pub fn builtin_providers() -> Vec<Value> {
    vec![
        // the proxy's own OAuth links (sign in in the browser, then the localhost:<port>/… address from the address bar) — the
        // main way, as in the proxy panel (the owner's wish); device/page codes through the core's flows are the fallbacks
        json!({ "id": "codex", "title": "ChatGPT (Codex)", "flow": "callback", "callback": "http://localhost:1455/auth/callback", "models": "GPT-5.x Codex" }),
        json!({ "id": "anthropic", "title": "Claude", "flow": "callback", "callback": "http://localhost:54545/callback", "models": "Claude Opus / Sonnet / Fable" }),
        json!({ "id": "antigravity", "title": "Antigravity (Google)", "flow": "callback", "callback": "http://localhost:51121/oauth-callback", "models": "Gemini 3 Pro / Flash через Antigravity" }),
        json!({ "id": "kimi", "title": "Kimi (Moonshot)", "flow": "device", "models": "Kimi K2 / K3" }),
        json!({ "id": "xai", "title": "xAI Grok", "flow": "device", "models": "Grok 4.x" }),
        json!({ "id": "codex-device", "title": "ChatGPT (Codex) · код устройства", "flow": "device", "core": true, "models": "GPT-5.x Codex" }),
        json!({ "id": "anthropic-code", "title": "Claude · код со страницы", "flow": "code", "core": true, "models": "Claude Opus / Sonnet / Fable" }),
        json!({ "id": "kimi-ai", "title": "Kimi.ai", "flow": "device", "since": "v8", "models": "Kimi через kimi.ai" }),
        json!({ "id": "meta", "title": "Meta AI", "flow": "device", "since": "v8", "models": "Llama" }),
    ]
}

/// To clients of the Anthropic format (a `/v1/models` request with `anthropic-version`) the proxy returns non-Claude
/// models under the name `claude-fable-5-dd-<id reversed>` (internal/client/claude/models, EnsureClaudeModelIDPrefix) and
/// translates such a name back itself on request. These are not duplicates: behind them are GPT (Codex), Gemini, Kimi,
/// Grok, Devin. The real id — for the menu and for the OpenAI format (simple chat, Codex); Claude Code with
/// ANTHROPIC_BASE_URL accepts the real id too.
pub const CLOAK_PREFIX: &str = "claude-fable-5-dd-";
pub fn real_model_id(id: &str) -> String {
    match id.strip_prefix(CLOAK_PREFIX) { Some(enc) if !enc.is_empty() => enc.chars().rev().collect(), _ => id.to_string() }
}

/// Channel (proxy account type) when there is no account list (a gate without a management key): the account
/// prefix in the id (`devin/…`) or the model owner.
pub fn guess_channel(real: &str, owned_by: &str) -> String {
    if let Some((pfx, _)) = real.split_once('/') { if !pfx.is_empty() { return pfx.to_ascii_lowercase(); } }
    match owned_by.trim() { "anthropic" => "claude", "openai" => "codex", "moonshot" => "kimi", "google" => "gemini", "" => "other", o => o }.to_string()
}

/// Models that per the proxy catalog are served by several channels. In the
/// gate list such a model has one `owned_by` (the last registered), so without an account list the channel is added from
/// the table.
const SHARED_MODELS: &[(&str, &str)] = &[("claude-sonnet-4-6", "antigravity"), ("claude-opus-4-6-thinking", "antigravity"), ("gpt-oss-120b-medium", "antigravity")];

/// Model channels in a gate without a management key: by id/owner, plus Claude Code for regular `claude-*` if the gate has
/// such a channel, plus the shared models from the proxy catalog (Sonnet 4.6 and Opus 4.6 Thinking at Antigravity).
pub fn guess_channels(real: &str, owned_by: &str, present: &std::collections::HashSet<String>) -> Vec<String> {
    let mut v = vec![guess_channel(real, owned_by)];
    let add = |v: &mut Vec<String>, ch: &str| if present.contains(ch) && !v.iter().any(|x| x == ch) { v.push(ch.to_string()); };
    if real.starts_with("claude-") && !real.contains('/') && !real.ends_with("-thinking") { add(&mut v, "claude"); }
    for (id, ch) in SHARED_MODELS { if real == *id { add(&mut v, ch); } }
    v
}

/// Channel label in the model menu.
pub fn channel_title(key: &str) -> String {
    match key {
        "claude" => "Claude Code".into(), "codex" => "Codex · ChatGPT".into(), "antigravity" => "Antigravity".into(), "gemini" | "gemini-cli" => "Gemini".into(),
        "aistudio" => "AI Studio".into(), "vertex" => "Vertex AI".into(), "kimi" => "Kimi".into(), "xai" | "grok" => "Grok · xAI".into(), "devin" => "Devin".into(),
        "qwen" => "Qwen".into(), "iflow" => "iFlow".into(), "other" => "Other".into(),
        k => { let mut c = k.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() }
    }
}

/// Channel order in the menu: the Claude Code and Codex
/// subscriptions first.
pub fn channel_rank(key: &str) -> usize { ["claude", "codex", "antigravity", "gemini", "gemini-cli", "kimi", "xai", "grok", "devin"].iter().position(|k| *k == key).unwrap_or(50) }

/// Model kind: images and video are not needed in the
/// chat menu.
pub fn model_kind(real: &str) -> &'static str {
    let l = real.to_ascii_lowercase();
    if l.contains("video") { "video" } else if l.contains("image") || l.contains("imagine") { "image" } else { "chat" }
}

impl Service {
    /// llm provider apps with Management API access. An external one without a management key is returned too (`managed: false`)
    /// so the UI can hint where to get the key.
    pub async fn gate_targets(&self) -> Result<Vec<(MgmtTarget, bool)>> {
        let mut out = Vec::new();
        for a in self.store.list_apps()? {
            if !a.manifest.provides.iter().any(|c| c == "llm") { continue; }
            let scope = format!("app:{}", a.name);
            if a.manifest.kind == AppKind::External {
                let Ok((url, api_key)) = self.external_target(&a) else { continue };
                let key = self.store.get_secret(&scope, "LLM_EXTERNAL_MANAGEMENT_KEY")?.filter(|k| !k.trim().is_empty());
                let managed = key.is_some();
                out.push((MgmtTarget { app: a.name.clone(), title: format!("{} · {}", a.manifest.title.clone().unwrap_or_else(|| a.name.clone()), url), kind: "external", base: url, key, api_key }, managed));
            } else {
                let Some(key) = self.store.get_secret(&scope, "GATE_MANAGEMENT_KEY")?.filter(|k| !k.trim().is_empty()) else { continue };
                let api_key = self.store.get_secret(&scope, "GATE_API_KEY")?.filter(|k| !k.trim().is_empty());
                let container = format!("{}-{}-1", a.name, a.manifest.endpoint.service);
                let base = match self.docker.container_ip(&container, &a.network()).await {
                    Ok(ip) => format!("http://{ip}:{}", a.manifest.endpoint.port),
                    Err(e) => { tracing::warn!("gate {}: адрес контейнера: {e:#}", a.name); continue }
                };
                out.push((MgmtTarget { app: a.name.clone(), title: format!("{} · локальный", a.manifest.title.clone().unwrap_or_else(|| a.name.clone())), kind: "local", base, key: Some(key), api_key }, true));
            }
        }
        Ok(out)
    }

    async fn gate_target(&self, app: &str) -> Result<MgmtTarget> {
        let (t, managed) = self.gate_targets().await?.into_iter().find(|(t, _)| t.app == app).ok_or_else(|| anyhow!("провайдер llm {app} не найден"))?;
        if !managed { bail!("у {app} нет ключа управления (Management API): задайте его в настройках приложения"); }
        Ok(t)
    }

    /// Management API call: status and body (JSON or text).
    async fn mgmt(&self, t: &MgmtTarget, method: reqwest::Method, path: &str, body: Option<Value>, timeout_s: u64) -> Result<(u16, Value)> {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout_s)).build()?;
        let mut req = http.request(method, format!("{}{path}", t.base.trim_end_matches('/'))).header("Accept", "application/json");
        if let Some(k) = &t.key { req = req.bearer_auth(k); }
        if let Some(b) = body { req = req.json(&b); }
        let resp = req.send().await.map_err(|e| anyhow!("{}: {e}", t.app))?;
        let status = resp.status().as_u16(); let text = resp.text().await.unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or(Value::String(text))))
    }

    fn mgmt_err(status: u16, v: &Value) -> String {
        let msg = v.get("error").and_then(|e| e.as_str()).or_else(|| v.get("message").and_then(|m| m.as_str())).map(str::to_string).unwrap_or_else(|| v.as_str().unwrap_or("").chars().take(200).collect());
        match status { 401 | 403 => format!("ключ управления не принят ({status}) {msg}"), 404 => format!("нет такого маршрута в этой версии прокси ({status}) {msg}"), _ => format!("{status} {msg}") }.trim().to_string()
    }

    /// Overview: targets with their sign-ins (auth files), plugins with OAuth and providers that can be
    /// added.
    pub async fn gate_auth_overview(&self) -> Result<Value> {
        let mut targets = Vec::new();
        for (t, managed) in self.gate_targets().await? {
            let mut v = json!({ "app": t.app, "title": t.title, "kind": t.kind, "managed": managed, "reachable": false, "logins": [], "plugins": [], "store": [], "providers": builtin_providers() });
            if !managed { targets.push(v); continue; }
            match self.mgmt(&t, reqwest::Method::GET, "/v0/management/auth-files", None, 15).await {
                Ok((200, files)) => {
                    v["reachable"] = json!(true);
                    let list: Vec<Value> = files.get("files").and_then(|f| f.as_array()).cloned().unwrap_or_default().into_iter().map(|f| {
                        let provider = f.get("provider").and_then(|p| p.as_str()).or_else(|| f.get("type").and_then(|p| p.as_str())).unwrap_or("").to_string();
                        json!({ "name": f.get("name"), "auth_index": f.get("auth_index"), "provider": provider, "label": f.get("label").and_then(|l| l.as_str()).filter(|l| !l.is_empty()).or_else(|| f.get("email").and_then(|e| e.as_str())), "email": f.get("email"), "status": f.get("status"), "status_message": f.get("status_message"), "disabled": f.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false), "unavailable": f.get("unavailable").and_then(|d| d.as_bool()).unwrap_or(false), "plan": f.get("plan_type") })
                    }).collect();
                    v["logins"] = json!(list);
                    // account quotas (GET /v0/management/quota?auth_index=…) → windows as for core accounts (docs/agents-ux.md
                    // §9.2); a proxy without support answers 404 — such a one is not asked for another 10 minutes
                    if !quota_unsupported(&t.base) {
                        let mut unsupported = false;
                        if let Some(arr) = v["logins"].as_array_mut() {
                            for l in arr.iter_mut() {
                                let Some(idx) = l.get("auth_index").and_then(|i| i.as_str()).map(str::to_string) else { continue };
                                match self.mgmt(&t, reqwest::Method::GET, &format!("/v0/management/quota?auth_index={}", urlencoding::encode(&idx)), None, 15).await {
                                    Ok((200, q)) => { let (plan, windows) = crate::limits::gate_windows(&q); if !windows.is_empty() || plan.is_some() { l["limits"] = json!({ "plan": plan, "windows": windows }); } }
                                    Ok((404, _)) => { unsupported = true; break }
                                    _ => {}
                                }
                            }
                        }
                        if unsupported { mark_quota_unsupported(&t.base); }
                    }
                }
                Ok((status, body)) => { v["error"] = json!(Self::mgmt_err(status, &body)); }
                Err(e) => { v["error"] = json!(format!("{e:#}")); }
            }
            if v["reachable"].as_bool() == Some(true) {
                if let Ok((200, pl)) = self.mgmt(&t, reqwest::Method::GET, "/v0/management/plugins", None, 15).await {
                    let plugins: Vec<Value> = pl.get("plugins").and_then(|p| p.as_array()).cloned().unwrap_or_default().into_iter().map(|p| json!({
                        "id": p.get("id"), "name": p.pointer("/metadata/name").and_then(|n| n.as_str()).filter(|n| !n.is_empty()).or_else(|| p.get("id").and_then(|i| i.as_str())), "version": p.pointer("/metadata/version"),
                        "enabled": p.get("effective_enabled").and_then(|e| e.as_bool()).or_else(|| p.get("enabled").and_then(|e| e.as_bool())).unwrap_or(false),
                        "oauth": p.get("supports_oauth").and_then(|e| e.as_bool()).unwrap_or(false), "provider": p.get("oauth_provider"),
                    })).collect();
                    v["plugins"] = json!(plugins);
                    let ids: Vec<String> = pl.get("plugins").and_then(|p| p.as_array()).cloned().unwrap_or_default().iter().filter_map(|p| p.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect();
                    // plugin store (GitHub): only those that give a provider sign-in
                    if let Ok((200, st)) = self.mgmt(&t, reqwest::Method::GET, "/v0/management/plugin-store", None, 25).await {
                        let store: Vec<Value> = st.get("plugins").and_then(|p| p.as_array()).cloned().unwrap_or_default().into_iter()
                            .filter(|p| p.get("auth_required").and_then(|a| a.as_bool()).unwrap_or(false) || p.get("id").and_then(|i| i.as_str()).map(|i| i.contains("gemini") || i.contains("copilot") || i.contains("qwen") || i.contains("iflow")).unwrap_or(false))
                            .map(|p| { let id = p.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(); json!({ "id": id, "name": p.get("name"), "description": p.get("description"), "version": p.get("version"), "installed": ids.contains(&id) }) }).collect();
                        v["store"] = json!(store);
                    }
                }
            }
            targets.push(v);
        }
        Ok(json!({ "targets": targets }))
    }

    /// Sign-in file in the proxy format from OpenAI/Anthropic tokens: (file name,
    /// JSON).
    fn gate_auth_file(provider: &str, tok: &Value) -> Result<(String, Value)> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let expires_in = tok.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(3600);
        let access = tok.get("access_token").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("в ответе нет access_token"))?;
        let refresh = tok.get("refresh_token").and_then(|v| v.as_str()).unwrap_or("");
        let stamp = crate::accounts::rfc3339_utc(now); let expired = crate::accounts::rfc3339_utc(now + expires_in);
        let safe = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || "@.-_".contains(c) { c } else { '_' }).collect::<String>();
        match provider {
            "codex" => {
                let id_token = tok.get("id_token").and_then(|v| v.as_str()).unwrap_or("");
                let (email, account_id, plan) = crate::accounts::codex_id_claims(id_token);
                let email = email.unwrap_or_default(); let plan = plan.unwrap_or_else(|| "unknown".into());
                let name = if email.is_empty() { format!("codex-{}.json", &stamp[..10]) } else { format!("codex-{}-{}.json", safe(&email), safe(&plan)) };
                Ok((name, json!({ "id_token": id_token, "access_token": access, "refresh_token": refresh, "account_id": account_id.unwrap_or_default(), "last_refresh": stamp, "email": email, "type": "codex", "expired": expired, "plan_type": plan })))
            }
            "claude" => {
                let email = tok.pointer("/account/email_address").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = if email.is_empty() { format!("claude-{}.json", &stamp[..10]) } else { format!("claude-{}.json", safe(&email)) };
                let mut v = json!({ "id_token": "", "access_token": access, "refresh_token": refresh, "last_refresh": stamp, "email": email, "type": "claude", "expired": expired });
                if let Some(u) = tok.pointer("/account/uuid").and_then(|x| x.as_str()) { v["account_uuid"] = json!(u); }
                if let Some(u) = tok.pointer("/organization/uuid").and_then(|x| x.as_str()) { v["organization_uuid"] = json!(u); }
                if let Some(n) = tok.pointer("/organization/name").and_then(|x| x.as_str()) { v["organization_name"] = json!(n); }
                Ok((name, v))
            }
            other => bail!("файл входа: неизвестный провайдер {other}"),
        }
    }

    /// Uploading a sign-in file to the proxy: `POST /v0/management/auth-files?name=<file>.json` with a JSON body (no
    /// multipart).
    async fn upload_auth_file(&self, t: &MgmtTarget, name: &str, body: &Value) -> Result<()> {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build()?;
        let mut req = http.post(format!("{}/v0/management/auth-files?name={}", t.base.trim_end_matches('/'), urlencoding::encode(name))).header("Content-Type", "application/json").body(serde_json::to_vec(body)?);
        if let Some(k) = &t.key { req = req.bearer_auth(k); }
        let resp = req.send().await.map_err(|e| anyhow!("{}: {e}", t.app))?;
        let status = resp.status().as_u16(); let text = resp.text().await.unwrap_or_default();
        if !(200..300).contains(&status) { bail!("загрузка файла входа в прокси: {status} — {}", text.chars().take(200).collect::<String>()); }
        let _ = self.store.event("account", &t.app, &format!("gate {}: добавлен вход {name}", t.app));
        Ok(())
    }

    /// Start a sign-in: ChatGPT and Claude — through the core's flows (the state in the core's pending marked gate), the rest — a link from the
    /// proxy (the `state` lives at the proxy).
    pub async fn gate_auth_start(&self, app: &str, provider: &str) -> Result<Value> {
        let provider = provider.trim().to_ascii_lowercase();
        if provider.is_empty() || !provider.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') { bail!("провайдер: буквы, цифры, «-»"); }
        let t = self.gate_target(app).await?;
        if provider == "codex-device" || provider == "anthropic-code" {
            let core_provider = if provider == "codex-device" { "codex" } else { "claude" };
            let mut r = self.oauth_start_for(core_provider, Some(&t.app)).await?;
            let flow = if core_provider == "codex" { "device" } else { "code" };
            r["app"] = json!(t.app); r["provider"] = json!(provider); r["flow"] = json!(flow); r["title"] = json!(if core_provider == "codex" { "ChatGPT (Codex) · код устройства" } else { "Claude · код со страницы" }); r["core"] = json!(true);
            return Ok(r);
        }
        let (status, v) = self.mgmt(&t, reqwest::Method::GET, &format!("/v0/management/{provider}-auth-url"), None, 45).await?;
        if status != 200 { bail!("вход {provider}: {}", Self::mgmt_err(status, &v)); }
        let url = v.get("url").and_then(|u| u.as_str()).ok_or_else(|| anyhow!("прокси не вернул ссылку входа"))?.to_string();
        let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let cat = builtin_providers().into_iter().find(|p| p.get("id").and_then(|i| i.as_str()) == Some(provider.as_str()));
        let user_code = v.get("user_code").and_then(|c| c.as_str()).map(str::to_string);
        let flow = v.get("flow").and_then(|f| f.as_str()).map(str::to_string).or_else(|| if user_code.is_some() { Some("device".into()) } else { None }).or_else(|| cat.as_ref().and_then(|c| c.get("flow").and_then(|f| f.as_str()).map(str::to_string))).unwrap_or_else(|| "callback".into());
        Ok(json!({ "app": t.app, "provider": provider, "url": url, "state": state, "flow": flow, "user_code": user_code, "expires_in": v.get("expires_in").and_then(|e| e.as_u64()).unwrap_or(900), "callback": cat.as_ref().and_then(|c| c.get("callback").cloned()).unwrap_or(Value::Null), "title": cat.as_ref().and_then(|c| c.get("title").cloned()).unwrap_or(json!(provider)) }))
    }

    pub async fn gate_auth_status(&self, app: &str, state: &str) -> Result<Value> {
        let t = self.gate_target(app).await?;
        // sign-in through the core's flow: we poll the Codex device-code ourselves and put the file into the proxy; Claude
        // waits for the code from the page (callback)
        if let Some(p) = self.oauth_pending(state)? {
            if p.get("gate").and_then(|g| g.as_str()) == Some(app) {
                if p.get("provider").and_then(|v| v.as_str()) != Some("codex") { return Ok(json!({ "status": "wait" })); }
                return match self.codex_device_poll(state).await? {
                    crate::accounts::DevicePoll::Wait => Ok(json!({ "status": "wait" })),
                    crate::accounts::DevicePoll::Error(e) => Ok(json!({ "status": "error", "error": e })),
                    crate::accounts::DevicePoll::Done(tok) => { let (name, body) = Self::gate_auth_file("codex", &tok)?; self.upload_auth_file(&t, &name, &body).await?; Ok(json!({ "status": "ok", "file": name })) }
                };
            }
        }
        let (status, v) = self.mgmt(&t, reqwest::Method::GET, &format!("/v0/management/get-auth-status?state={}", urlencoding::encode(state)), None, 15).await?;
        if status != 200 { bail!("состояние входа: {}", Self::mgmt_err(status, &v)); }
        let st = v.get("status").and_then(|s| s.as_str()).unwrap_or("wait");
        if st == "ok" { let _ = self.store.event("account", app, &format!("gate {app}: вход провайдера завершён")); }
        Ok(json!({ "status": st, "error": v.get("error") }))
    }

    /// Callback providers: the address the browser was redirected to (`http://localhost:…/…?code=…&state=…`) — to the
    /// proxy.
    pub async fn gate_auth_callback(&self, app: &str, state: &str, redirect_url: &str) -> Result<Value> {
        let t = self.gate_target(app).await?;
        let raw = redirect_url.trim();
        if raw.is_empty() { bail!("вставьте адрес из адресной строки браузера"); }
        // Claude through the core's flow: `code#state` from the page → tokens → a
        // sign-in file into the proxy
        if let Some(p) = self.oauth_pending(state)? {
            if p.get("gate").and_then(|g| g.as_str()) == Some(app) && p.get("provider").and_then(|v| v.as_str()) == Some("claude") {
                let (_, tok) = self.oauth_exchange(state, raw).await?;
                let (name, body) = Self::gate_auth_file("claude", &tok)?; self.upload_auth_file(&t, &name, &body).await?;
                return Ok(json!({ "status": "ok", "done": true, "file": name }));
            }
        }
        // the proxy would accept an address from another sign-in (a different state) and try to exchange a
        // foreign code — cut it off right away
        if let Some(q) = raw.split_once('?').map(|(_, q)| q) {
            let other = q.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == "state").map(|(_, v)| urlencoding::decode(v).map(|c| c.into_owned()).unwrap_or_else(|_| v.to_string()));
            if let Some(o) = other { if o != state { bail!("этот адрес от другого входа (state не совпадает) — откройте ссылку заново и вставьте адрес того же входа"); } }
            if !q.split('&').any(|kv| kv.starts_with("code=") || kv.starts_with("error=")) { bail!("в адресе нет code — скопируйте адрес целиком, как он показан после входа"); }
        }
        let body = if raw.starts_with("http://") || raw.starts_with("https://") { json!({ "state": state, "redirect_url": raw }) } else { json!({ "state": state, "code": raw }) };
        let (status, v) = self.mgmt(&t, reqwest::Method::POST, "/v0/management/oauth-callback", Some(body), 15).await?;
        if status != 200 { bail!("callback: {}", Self::mgmt_err(status, &v)); }
        Ok(json!({ "status": v.get("status").and_then(|s| s.as_str()).unwrap_or("ok"), "done": false }))
    }

    pub async fn gate_login_delete(&self, app: &str, name: &str) -> Result<()> {
        let t = self.gate_target(app).await?;
        if name.trim().is_empty() || name.contains('/') || name.contains("..") { bail!("имя файла входа"); }
        let (status, v) = self.mgmt(&t, reqwest::Method::DELETE, &format!("/v0/management/auth-files?name={}", urlencoding::encode(name)), None, 15).await?;
        if !(200..300).contains(&status) { bail!("удаление входа: {}", Self::mgmt_err(status, &v)); }
        let _ = self.store.event("account", app, &format!("gate {app}: удалён вход {name}"));
        Ok(())
    }

    pub async fn gate_login_disabled(&self, app: &str, name: &str, disabled: bool) -> Result<Value> {
        let t = self.gate_target(app).await?;
        let (status, v) = self.mgmt(&t, reqwest::Method::PATCH, "/v0/management/auth-files/status", Some(json!({ "name": name, "disabled": disabled })), 15).await?;
        if !(200..300).contains(&status) { bail!("статус входа: {}", Self::mgmt_err(status, &v)); }
        Ok(json!({ "name": name, "disabled": disabled }))
    }

    /// Proxy models (`/v1/models` with the API key): for a project — only its llm provider, otherwise all targets.
    /// Models by gate accounts (Management API, `auth-files` → `auth-files/models`): real id → the channels that serve
    /// it. None — the gate has no management key or it is unavailable.
    async fn models_by_channel(&self, t: &MgmtTarget) -> Option<std::collections::HashMap<String, Vec<String>>> {
        t.key.as_ref()?;
        let (st, files) = self.mgmt(t, reqwest::Method::GET, "/v0/management/auth-files", None, 15).await.ok()?;
        if st >= 400 { return None; }
        let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for f in files.get("files").and_then(|f| f.as_array()).cloned().unwrap_or_default() {
            if f.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false) { continue; }
            let Some(name) = f.get("name").and_then(|n| n.as_str()) else { continue };
            let ch = f.get("provider").and_then(|p| p.as_str()).or_else(|| f.get("type").and_then(|p| p.as_str())).unwrap_or("other").trim().to_ascii_lowercase();
            let Ok((200, ms)) = self.mgmt(t, reqwest::Method::GET, &format!("/v0/management/auth-files/models?name={}", urlencoding::encode(name)), None, 15).await else { continue };
            for m in ms.get("models").and_then(|m| m.as_array()).cloned().unwrap_or_default() {
                if let Some(id) = m.get("id").and_then(|i| i.as_str()) { let e = map.entry(id.to_string()).or_default(); if !e.contains(&ch) { e.push(ch.clone()); } }
            }
        }
        Some(map)
    }

    /// Models of a gate app for the menu: the `/v1/models` list in the Anthropic format (context window and name), the id
    /// translated to the real one (`listed` — as the proxy returned it), the channel by accounts or by id/owner, the kind.
    /// A model served by several channels (Sonnet at Claude Code and at Antigravity) gets an entry per channel; the order
    /// is by channels.
    pub async fn gate_model_list(&self, t: &MgmtTarget) -> Vec<Value> {
        let http = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build() { Ok(h) => h, Err(_) => return Vec::new() };
        let mut req = http.get(format!("{}/v1/models", t.base.trim_end_matches('/'))).header("Accept", "application/json").header("anthropic-version", "2023-06-01");
        if let Some(k) = &t.api_key { req = req.bearer_auth(k).header("x-api-key", k); }
        let models: Vec<Value> = match req.send().await {
            Ok(r) if r.status().is_success() => r.json::<Value>().await.ok().and_then(|v| v.get("data").and_then(|d| d.as_array()).cloned()).unwrap_or_default(),
            Ok(r) => { tracing::warn!("gate {}: /v1/models {}", t.app, r.status()); Vec::new() }
            Err(e) => { tracing::warn!("gate {}: /v1/models {e}", t.app); Vec::new() }
        };
        let by_ch = self.models_by_channel(t).await;
        // channels that exist in this gate at all (by owners) — for the shared models table
        let present: std::collections::HashSet<String> = models.iter().filter_map(|m| { let id = m.get("id").and_then(|i| i.as_str())?; Some(guess_channel(&real_model_id(id), m.get("owned_by").and_then(|o| o.as_str()).unwrap_or(""))) }).collect();
        let mut out: Vec<Value> = Vec::new();
        for m in &models {
            let Some(listed) = m.get("id").and_then(|i| i.as_str()) else { continue };
            let real = real_model_id(listed);
            let owned = m.get("owned_by").and_then(|o| o.as_str()).unwrap_or("");
            let chans = by_ch.as_ref().and_then(|mp| mp.get(&real).or_else(|| mp.get(listed)).cloned()).filter(|v| !v.is_empty()).unwrap_or_else(|| guess_channels(&real, owned, &present));
            for ch in chans {
                out.push(json!({ "id": real, "listed": listed, "owned_by": owned, "type": m.get("type"), "name": m.get("display_name").or_else(|| m.get("name")),
                    "context": m.get("max_input_tokens").and_then(|c| c.as_u64()), "max_output": m.get("max_tokens").and_then(|c| c.as_u64()),
                    "provider": ch, "provider_title": channel_title(&ch), "kind": model_kind(&real) }));
            }
        }
        out.sort_by_key(|v| { let p = v.get("provider").and_then(|p| p.as_str()).unwrap_or("").to_string(); (channel_rank(&p), p) });
        out
    }

    pub async fn gate_models(&self, app: Option<&str>, project: Option<&str>) -> Result<Value> {
        let want: Option<String> = match (app, project) {
            (Some(a), _) => Some(a.to_string()),
            (None, Some(p)) => self.store.list_grants(Some(p))?.into_iter().find(|g| g.capability == "llm").map(|g| g.app),
            _ => None,
        };
        let mut out = Vec::new();
        for (t, _) in self.gate_targets().await? {
            if let Some(w) = &want { if &t.app != w { continue; } }
            let list = self.gate_model_list(&t).await;
            out.push(json!({ "app": t.app, "title": t.title, "models": list }));
        }
        Ok(json!({ "targets": out }))
    }

    pub async fn gate_plugin_install(&self, app: &str, id: &str) -> Result<Value> {
        let t = self.gate_target(app).await?;
        if id.trim().is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') { bail!("идентификатор плагина"); }
        let (status, v) = self.mgmt(&t, reqwest::Method::POST, &format!("/v0/management/plugin-store/{id}/install"), Some(json!({})), 180).await?;
        if !(200..300).contains(&status) { bail!("установка плагина {id}: {}", Self::mgmt_err(status, &v)); }
        let _ = self.mgmt(&t, reqwest::Method::PATCH, &format!("/v0/management/plugins/{id}/enabled"), Some(json!({ "enabled": true })), 15).await;
        let _ = self.store.event("account", app, &format!("gate {app}: установлен плагин {id}"));
        Ok(v)
    }

    pub async fn gate_plugin_enabled(&self, app: &str, id: &str, enabled: bool) -> Result<Value> {
        let t = self.gate_target(app).await?;
        let (status, v) = self.mgmt(&t, reqwest::Method::PATCH, &format!("/v0/management/plugins/{id}/enabled"), Some(json!({ "enabled": enabled })), 15).await?;
        if !(200..300).contains(&status) { bail!("плагин {id}: {}", Self::mgmt_err(status, &v)); }
        Ok(json!({ "id": id, "enabled": enabled }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cloaked_ids_and_channels() {
        use super::*;
        assert_eq!(real_model_id("claude-fable-5-dd-artsa-6-tpg"), "gpt-6-astra");
        assert_eq!(real_model_id("claude-fable-5-dd-elbaf-5-edualc/nived"), "devin/claude-5-fable");
        assert_eq!(real_model_id("claude-fable-5-1"), "claude-fable-5-1");
        assert_eq!(real_model_id("claude-fable-5-dd-"), "claude-fable-5-dd-");
        assert_eq!(guess_channel("devin/gpt-6-sol", "openai"), "devin");
        assert_eq!(guess_channel("gpt-6-astra", "openai"), "codex");
        assert_eq!(guess_channel("claude-sonnet-4-6", "anthropic"), "claude");
        assert_eq!(guess_channel("claude-opus-4-6-thinking", "antigravity"), "antigravity");
        assert_eq!(guess_channel("kimi-k2.6", "moonshot"), "kimi");
        assert_eq!(guess_channel("grok-4.7", "xai"), "xai");
        let present: std::collections::HashSet<String> = ["claude", "antigravity", "codex"].iter().map(|s| s.to_string()).collect();
        assert_eq!(guess_channels("claude-sonnet-4-6", "antigravity", &present), vec!["antigravity", "claude"]);
        assert_eq!(guess_channels("claude-sonnet-4-6", "anthropic", &present), vec!["claude", "antigravity"]);
        assert_eq!(guess_channels("claude-opus-4-6-thinking", "antigravity", &present), vec!["antigravity"]);
        assert_eq!(guess_channels("devin/claude-opus-4-6", "anthropic", &present), vec!["devin"]);
        assert_eq!(guess_channels("claude-sonnet-4-6", "anthropic", &["claude".to_string()].into_iter().collect()), vec!["claude"]);
        assert_eq!(channel_title("codex"), "Codex · ChatGPT"); assert_eq!(channel_title("zhipu"), "Zhipu");
        assert!(channel_rank("claude") < channel_rank("codex") && channel_rank("codex") < channel_rank("devin") && channel_rank("devin") < channel_rank("zhipu"));
        assert_eq!(model_kind("gpt-image-2"), "image"); assert_eq!(model_kind("grok-imagine-video-1.5"), "video"); assert_eq!(model_kind("gpt-6-sol"), "chat");
    }
    use super::*;
    #[test]
    fn gate_auth_files() {
        let payload = serde_json::to_vec(&json!({ "email": "u@example.com", "https://api.openai.com/auth": { "chatgpt_account_id": "acc-1", "chatgpt_plan_type": "plus" } })).unwrap();
        let b64 = |b: &[u8]| { const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"; let mut o = String::new(); for ch in b.chunks(3) { let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32; o.push(T[(n >> 18) as usize & 63] as char); o.push(T[(n >> 12) as usize & 63] as char); if ch.len() > 1 { o.push(T[(n >> 6) as usize & 63] as char); } if ch.len() > 2 { o.push(T[n as usize & 63] as char); } } o };
        let jwt = format!("h.{}.s", b64(&payload));
        let (name, v) = Service::gate_auth_file("codex", &json!({ "id_token": jwt, "access_token": "at", "refresh_token": "rt", "expires_in": 3600 })).unwrap();
        assert_eq!(name, "codex-u@example.com-plus.json"); assert_eq!(v["type"], "codex"); assert_eq!(v["account_id"], "acc-1"); assert_eq!(v["email"], "u@example.com"); assert!(v["expired"].as_str().unwrap().ends_with('Z'));
        let (name, v) = Service::gate_auth_file("claude", &json!({ "access_token": "sk-ant-oat01-x", "refresh_token": "r", "expires_in": 28800, "account": { "email_address": "c@example.com", "uuid": "u1" }, "organization": { "uuid": "o1", "name": "Org" } })).unwrap();
        assert_eq!(name, "claude-c@example.com.json"); assert_eq!(v["type"], "claude"); assert_eq!(v["organization_name"], "Org"); assert_eq!(v["account_uuid"], "u1");
    }

    #[test]
    fn providers_catalog() {
        let p = builtin_providers();
        assert!(p.iter().any(|x| x["id"] == "antigravity" && x["flow"] == "callback" && x["callback"].as_str().unwrap().contains("51121")));
        assert!(p.iter().any(|x| x["id"] == "kimi" && x["flow"] == "device"));
        assert!(p.iter().any(|x| x["id"] == "codex" && x["callback"].as_str().unwrap().contains("1455")));
        assert_eq!(p[0]["id"], "codex"); assert_eq!(p[1]["id"], "anthropic"); assert!(p.iter().any(|x| x["id"] == "codex-device" && x["core"] == true));
    }
}
