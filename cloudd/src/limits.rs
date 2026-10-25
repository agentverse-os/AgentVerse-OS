//! Subscription limits of accounts (docs/agents-ux.md §9.2): provider limit windows in one shape — as in the LLM proxy quota API
//! (subscription plan, windows with a usage share and reset time). The core asks the providers itself with the account
//! tokens: Claude — `api.anthropic.com/api/oauth/usage` (+ `/profile` for the plan), ChatGPT —
//! `chatgpt.com/backend-api/wham/usage`; an expired token is refreshed via refresh_token at the same token endpoints as at
//! sign-in, the new sign-in file is saved into the account (it reaches the workspace at the next session). 60 s cache per
//! account, `refresh=1` — forced.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::accounts::{codex_id_claims, rfc3339_utc, Account, CLAUDE_CLIENT_ID, CLAUDE_TOKEN_URL, CODEX_CLIENT_ID, CODEX_TOKEN_URL};
use crate::service::Service;

pub const ANTHROPIC_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
pub const ANTHROPIC_PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
pub const CHATGPT_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const TTL: Duration = Duration::from_secs(60);

/// Limit window: `id` — the provider's name (five_hour, seven_day, seven_day_opus, primary, secondary…), usage share in
/// percent, reset time (RFC 3339) and the window length in minutes if the provider reports it.
#[derive(Debug, Clone, Serialize)]
pub struct Window { pub id: String, pub used_pct: f64, pub resets_at: Option<String>, pub window_mins: Option<u64> }

#[derive(Debug, Clone, Serialize)]
pub struct Limits {
    pub account: String,
    pub provider: String,
    pub plan: Option<String>,
    pub windows: Vec<Window>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<Value>,
    pub fetched_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The subscription token was expired and refreshed via refresh_token (the new sign-in file is
    /// saved into the account)
    pub refreshed_token: bool,
}

impl Limits {
    fn err(account: &str, provider: &str, e: anyhow::Error) -> Self {
        Limits { account: account.into(), provider: provider.into(), plan: None, windows: vec![], credits: None, fetched_at: rfc3339_utc(now_secs()), error: Some(format!("{e:#}")), refreshed_token: false }
    }
}

fn now_secs() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }
fn cache() -> &'static Mutex<HashMap<String, (Instant, Limits)>> {
    static C: OnceLock<Mutex<HashMap<String, (Instant, Limits)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}
fn http(timeout_s: u64) -> Result<reqwest::Client> { Ok(reqwest::Client::builder().timeout(Duration::from_secs(timeout_s)).build()?) }

/// JWT payload without signature verification — for the `exp` of the ChatGPT
/// access token.
fn jwt_payload(jwt: &str) -> Value {
    let Some(part) = jwt.split('.').nth(1) else { return Value::Null };
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = Vec::with_capacity(part.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in part.bytes() {
        let Some(v) = T.iter().position(|t| *t == c) else { continue };
        buf = (buf << 6) | v as u32; bits += 6;
        if bits >= 8 { bits -= 8; out.push((buf >> bits) as u8 & 0xff); }
    }
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

/// Human-readable plan name from technical labels: `default_claude_max_5x` → "Max 5x", `claude_pro` → "Pro", `plus` → "Plus".
pub fn plan_label(raw: &str) -> String {
    let s = raw.trim().trim_start_matches("default_").trim_start_matches("claude_").replace(['_', '-'], " ");
    if s.is_empty() { return String::new() }
    let mut out = String::new();
    for (i, w) in s.split_whitespace().enumerate() {
        if i > 0 { out.push(' '); }
        let mut cs = w.chars();
        match cs.next() { Some(f) => { out.extend(f.to_uppercase()); out.push_str(cs.as_str()); } None => {} }
    }
    out
}

/// Windows from the Anthropic `/api/oauth/usage` answer: every object with `utilization` is a window; the name is the
/// key (five_hour, seven_day, seven_day_opus, seven_day_sonnet…), `resets_at` as is, the window length by name.
pub fn claude_windows(body: &Value) -> Vec<Window> {
    let mut out = Vec::new();
    let Some(obj) = body.as_object() else { return out };
    let order = ["five_hour", "seven_day", "seven_day_opus", "seven_day_sonnet"];
    // `extra_usage` (paid usage beyond the plan) is not a limit window: it
    // goes into credits
    let mut keys: Vec<&String> = obj.keys().filter(|k| !k.starts_with("extra") && obj[*k].get("utilization").is_some()).collect();
    keys.sort_by_key(|k| order.iter().position(|o| o == k).unwrap_or(99));
    for k in keys {
        let w = &obj[k];
        let used = w.get("utilization").and_then(|u| u.as_f64()).unwrap_or(0.0);
        let mins = if k.starts_with("five_hour") { Some(300) } else if k.starts_with("seven_day") { Some(7 * 24 * 60) } else { None };
        out.push(Window { id: k.clone(), used_pct: (used * 100.0).round() / 100.0, resets_at: w.get("resets_at").and_then(|r| r.as_str()).map(str::to_string), window_mins: mins });
    }
    out
}

/// Windows from the ChatGPT `backend-api/wham/usage` answer: `rate_limit.primary_window` / `secondary_window` with
/// used_percent, limit_window_seconds and reset_at (unix); windows with other names are picked up too.
pub fn codex_windows(body: &Value) -> Vec<Window> {
    let mut out = Vec::new();
    let Some(rl) = body.get("rate_limit").and_then(|r| r.as_object()) else { return out };
    let mut keys: Vec<&String> = rl.keys().filter(|k| rl[*k].is_object() && rl[*k].get("used_percent").is_some()).collect();
    keys.sort_by_key(|k| ["primary_window", "secondary_window"].iter().position(|o| o == k).unwrap_or(99));
    for k in keys {
        let w = &rl[k];
        let secs = w.get("limit_window_seconds").and_then(|s| s.as_u64());
        let reset = w.get("reset_at").and_then(|r| r.as_u64()).map(rfc3339_utc).or_else(|| w.get("reset_after_seconds").and_then(|s| s.as_u64()).map(|s| rfc3339_utc(now_secs() + s)));
        out.push(Window { id: k.trim_end_matches("_window").to_string(), used_pct: w.get("used_percent").and_then(|u| u.as_f64()).unwrap_or(0.0), resets_at: reset, window_mins: secs.map(|s| s / 60) });
    }
    out
}

/// Windows from the normalized proxy quota (`/v0/management/quota`): groups and buckets with the remaining share,
/// window and reset time (field names in both cases, as the Go struct and its JSON tags write them).
pub fn gate_windows(body: &Value) -> (Option<String>, Vec<Window>) {
    let get = |v: &Value, a: &str, b: &str| v.get(a).cloned().or_else(|| v.get(b).cloned()).unwrap_or(Value::Null);
    let plan = get(&get(body, "Subscription", "subscription"), "Plan", "plan").as_str().map(str::to_string).filter(|p| !p.is_empty());
    let mut out = Vec::new();
    if let Some(groups) = get(body, "Groups", "groups").as_array() {
        for g in groups {
            let gname = get(g, "Name", "name").as_str().unwrap_or("").to_string();
            if let Some(buckets) = get(g, "Buckets", "buckets").as_array() {
                for b in buckets {
                    let remaining = get(b, "RemainingFraction", "remaining_fraction").as_f64().unwrap_or(1.0);
                    let window = get(b, "Window", "window").as_str().unwrap_or("").to_string();
                    let reset = get(b, "ResetTime", "reset_time").as_str().map(str::to_string);
                    let id = if window.is_empty() { gname.clone() } else if gname.is_empty() { window.clone() } else { format!("{gname} · {window}") };
                    out.push(Window { id, used_pct: ((1.0 - remaining).clamp(0.0, 1.0) * 10000.0).round() / 100.0, resets_at: reset, window_mins: None });
                }
            }
        }
    }
    (plan, out)
}

impl Service {
    /// Limits of all subscriptions in parallel (60 s cache, `refresh` —
    /// anew).
    pub async fn all_limits(&self, refresh: bool) -> Vec<Limits> {
        let accs: Vec<Account> = self.accounts().unwrap_or_default().into_iter().filter(|a| a.kind == "subscription").collect();
        let mut set = tokio::task::JoinSet::new();
        for a in accs { let me = self.clone(); set.spawn(async move { me.account_limits(&a.id, refresh).await }); }
        let mut out = Vec::new();
        while let Some(r) = set.join_next().await { if let Ok(l) = r { out.push(l); } }
        out.sort_by(|a, b| a.account.cmp(&b.account));
        out
    }

    pub async fn account_limits(&self, id: &str, refresh: bool) -> Limits {
        if !refresh { if let Some((t, l)) = cache().lock().unwrap().get(id) { if t.elapsed() < TTL { return l.clone(); } } }
        let a = match self.account(id) { Ok(a) => a, Err(e) => return Limits::err(id, "", e) };
        let r = match a.provider.as_str() {
            "claude" | "anthropic" => self.claude_limits(&a).await,
            "codex" | "openai" => self.codex_limits(&a).await,
            "antigravity" => self.agy_limits(&a).await,
            p => Err(anyhow!("лимиты для провайдера {p} не поддерживаются")),
        };
        let l = match r { Ok(l) => l, Err(e) => Limits::err(&a.id, &a.provider, e) };
        cache().lock().unwrap().insert(id.to_string(), (Instant::now(), l.clone()));
        l
    }

    // ---- Antigravity (§9.11): agy has no limit windows — check that the sign-in is valid (`agy models` under the account HOME in
    // the workspace where the account is unpacked; if nowhere — unpack into the workspace it was imported from) and refresh the list
    // of native models ----
    async fn agy_limits(&self, a: &Account) -> Result<Limits> {
        let dir = format!("{}/{}", crate::accounts::WS_DIR, a.id);
        let running: Vec<String> = self.incus.instances().await?.into_iter().filter(|i| i.status == "Running" && i.name.starts_with("ws-")).map(|i| i.name).collect();
        let mut inst: Option<String> = None;
        for i in &running { if self.agy_home_stamp(i, &dir).await > 0 { inst = Some(i.clone()); break } }
        let inst = match inst {
            Some(i) => i,
            None => {
                let project = a.imported_from.clone().filter(|p| !p.is_empty()).ok_or_else(|| anyhow!("нет запущенного workspace'а с этим аккаунтом"))?;
                self.account_session_env(&project, &a.id).await?;
                self.workspace_instance(&project).await
            }
        };
        let models = self.agy_signed_in(&inst, &dir).await.ok_or_else(|| anyhow!("вход Antigravity не действует (agy не показывает модели) — войдите заново"))?;
        if models != a.models { if let Ok(mut list) = self.accounts() { if let Some(x) = list.iter_mut().find(|x| x.id == a.id) { x.models = models.clone(); } let _ = self.save_accounts(&list); } }
        Ok(Limits { account: a.id.clone(), provider: a.provider.clone(), plan: Some(format!("{} моделей", models.len())), windows: Vec::new(), credits: None, fetched_at: rfc3339_utc(now_secs()), error: None, refreshed_token: false })
    }

    // ---- Claude: .credentials.json (claudeAiOauth.accessToken / refreshToken / expiresAt in ms)
    // ----
    async fn claude_limits(&self, a: &Account) -> Result<Limits> {
        let raw = self.freshest_login_file(a, ".credentials.json").await?;
        let mut creds: Value = serde_json::from_str(&raw)?;
        let mut access = creds.pointer("/claudeAiOauth/accessToken").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let expires_ms = creds.pointer("/claudeAiOauth/expiresAt").and_then(|v| v.as_u64()).unwrap_or(0);
        let mut refreshed = false;
        if access.is_empty() || expires_ms <= (now_secs() + 60) * 1000 { access = self.claude_refresh(a, &mut creds).await?; refreshed = true; }
        let http = http(20)?;
        let call = |tok: String| { let http = http.clone(); async move {
            let r = http.get(ANTHROPIC_USAGE_URL).bearer_auth(&tok).header("anthropic-beta", "oauth-2025-04-20").header("User-Agent", "claude-code/2.1.284").send().await?;
            let st = r.status().as_u16(); let body: Value = r.json().await.unwrap_or(Value::Null); Ok::<_, anyhow::Error>((st, body)) } };
        let (mut st, mut body) = call(access.clone()).await?;
        if st == 401 && !refreshed { access = self.claude_refresh(a, &mut creds).await?; refreshed = true; (st, body) = call(access.clone()).await?; }
        if st != 200 { bail!("usage: HTTP {st} {}", body.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("")); }
        let windows = claude_windows(&body);
        // the plan comes from the organization profile; without it the limits
        // are shown anyway
        let plan = match http.get(ANTHROPIC_PROFILE_URL).bearer_auth(&access).header("anthropic-beta", "oauth-2025-04-20").header("User-Agent", "claude-code/2.1.284").send().await {
            Ok(r) if r.status().is_success() => {
                let p: Value = r.json().await.unwrap_or(Value::Null);
                let org = p.get("organization").cloned().unwrap_or(Value::Null);
                org.get("rate_limit_tier").and_then(|t| t.as_str()).or_else(|| org.get("organization_type").and_then(|t| t.as_str())).or_else(|| org.get("billing_type").and_then(|t| t.as_str())).map(plan_label).filter(|s| !s.is_empty())
            }
            _ => None,
        };
        let credits = body.get("extra_usage").cloned().filter(|e| e.is_object() && e.get("is_enabled").and_then(|b| b.as_bool()) == Some(true));
        Ok(Limits { account: a.id.clone(), provider: a.provider.clone(), plan, windows, credits, fetched_at: rfc3339_utc(now_secs()), error: None, refreshed_token: refreshed })
    }

    /// Claude token refresh via refresh_token (as Claude Code itself does): the new `.credentials.json` is saved into the
    /// account.
    async fn claude_refresh(&self, a: &Account, creds: &mut Value) -> Result<String> {
        let refresh = creds.pointer("/claudeAiOauth/refreshToken").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("вход Claude истёк, а refresh_token в файле нет — войдите заново"))?.to_string();
        let r = http(20)?.post(CLAUDE_TOKEN_URL).json(&json!({ "grant_type": "refresh_token", "refresh_token": refresh, "client_id": CLAUDE_CLIENT_ID })).header("Accept", "application/json").send().await?;
        let st = r.status().as_u16(); let tok: Value = r.json().await.unwrap_or(Value::Null);
        if st != 200 { bail!("обновление токена Claude: HTTP {st} {} — войдите в подписку заново", tok.get("error_description").or_else(|| tok.get("error")).and_then(|e| e.as_str()).unwrap_or("")); }
        let access = tok.get("access_token").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("в ответе нет access_token"))?.to_string();
        let expires_in = tok.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(3600);
        if creds.get("claudeAiOauth").map(|o| !o.is_object()).unwrap_or(true) { creds["claudeAiOauth"] = json!({}); }
        creds["claudeAiOauth"]["accessToken"] = json!(access);
        if let Some(rt) = tok.get("refresh_token").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { creds["claudeAiOauth"]["refreshToken"] = json!(rt); }
        creds["claudeAiOauth"]["expiresAt"] = json!((now_secs() + expires_in) * 1000);
        self.store.put_secret(&format!("account:{}", a.id), "file:.credentials.json", &serde_json::to_string(creds)?)?;
        let _ = self.store.event("account", &a.id, &format!("токен подписки Claude «{}» обновлён", a.label));
        Ok(access)
    }

    // ---- ChatGPT (Codex): auth.json (tokens.access_token / refresh_token / id_token / account_id) ----
    async fn codex_limits(&self, a: &Account) -> Result<Limits> {
        let raw = self.freshest_login_file(a, "auth.json").await?;
        let mut auth: Value = serde_json::from_str(&raw)?;
        let mut access = auth.pointer("/tokens/access_token").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let account_id = auth.pointer("/tokens/account_id").and_then(|s| s.as_str()).map(str::to_string)
            .or_else(|| auth.pointer("/tokens/id_token").and_then(|s| s.as_str()).and_then(|t| codex_id_claims(t).1));
        let exp = jwt_payload(&access).get("exp").and_then(|e| e.as_u64()).unwrap_or(0);
        let mut refreshed = false;
        if access.is_empty() || exp <= now_secs() + 60 { access = self.codex_refresh(a, &mut auth).await?; refreshed = true; }
        let http = http(20)?;
        let call = |tok: String, acc: Option<String>| { let http = http.clone(); async move {
            let mut req = http.get(CHATGPT_USAGE_URL).bearer_auth(&tok).header("User-Agent", "codex_cli_rs/0.156.1").header("originator", "codex_cli_rs").header("Accept", "application/json");
            if let Some(acc) = acc { req = req.header("ChatGPT-Account-Id", acc); }
            let r = req.send().await?; let st = r.status().as_u16(); let body: Value = r.json().await.unwrap_or(Value::Null); Ok::<_, anyhow::Error>((st, body)) } };
        let (mut st, mut body) = call(access.clone(), account_id.clone()).await?;
        if st == 401 && !refreshed { access = self.codex_refresh(a, &mut auth).await?; refreshed = true; (st, body) = call(access.clone(), account_id.clone()).await?; }
        if st != 200 { bail!("usage: HTTP {st} {}", body.pointer("/detail").and_then(|m| m.as_str()).or_else(|| body.pointer("/error/message").and_then(|m| m.as_str())).unwrap_or("")); }
        let windows = codex_windows(&body);
        let plan = body.get("plan_type").and_then(|p| p.as_str()).map(plan_label)
            .or_else(|| auth.pointer("/tokens/id_token").and_then(|s| s.as_str()).and_then(|t| codex_id_claims(t).2).map(|p| plan_label(&p)));
        let credits = body.get("credits").cloned().filter(|c| c.is_object());
        Ok(Limits { account: a.id.clone(), provider: a.provider.clone(), plan, windows, credits, fetched_at: rfc3339_utc(now_secs()), error: None, refreshed_token: refreshed })
    }

    /// ChatGPT token refresh via refresh_token (as Codex CLI does): the new `auth.json` is saved into the
    /// account.
    async fn codex_refresh(&self, a: &Account, auth: &mut Value) -> Result<String> {
        let refresh = auth.pointer("/tokens/refresh_token").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("вход ChatGPT истёк, а refresh_token в файле нет — войдите заново"))?.to_string();
        let r = http(20)?.post(CODEX_TOKEN_URL).json(&json!({ "client_id": CODEX_CLIENT_ID, "grant_type": "refresh_token", "refresh_token": refresh, "scope": "openid profile email" })).header("Accept", "application/json").send().await?;
        let st = r.status().as_u16(); let tok: Value = r.json().await.unwrap_or(Value::Null);
        if st != 200 { bail!("обновление токена ChatGPT: HTTP {st} {} — войдите в подписку заново", tok.get("error_description").or_else(|| tok.get("error")).and_then(|e| e.as_str()).unwrap_or("")); }
        let access = tok.get("access_token").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("в ответе нет access_token"))?.to_string();
        if auth.get("tokens").map(|t| !t.is_object()).unwrap_or(true) { auth["tokens"] = json!({}); }
        auth["tokens"]["access_token"] = json!(access);
        if let Some(rt) = tok.get("refresh_token").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { auth["tokens"]["refresh_token"] = json!(rt); }
        if let Some(idt) = tok.get("id_token").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { auth["tokens"]["id_token"] = json!(idt); if auth["tokens"].get("account_id").map(|v| v.is_null()).unwrap_or(true) { if let Some(acc) = codex_id_claims(idt).1 { auth["tokens"]["account_id"] = json!(acc); } } }
        auth["last_refresh"] = json!(rfc3339_utc(now_secs()));
        self.store.put_secret(&format!("account:{}", a.id), "file:auth.json", &serde_json::to_string(auth)?)?;
        let _ = self.store.event("account", &a.id, &format!("токены подписки ChatGPT «{}» обновлены", a.label));
        Ok(access)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_usage_windows() {
        let b = json!({ "five_hour": { "utilization": 42.5, "resets_at": "2026-10-06T15:00:00Z" }, "seven_day": { "utilization": 18, "resets_at": "2026-10-09T09:00:00Z" }, "seven_day_opus": null, "extra_usage": { "is_enabled": false, "utilization": 0.0 } });
        let w = claude_windows(&b);
        assert_eq!(w.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["five_hour", "seven_day"]);
        assert_eq!(w[0].used_pct, 42.5); assert_eq!(w[0].window_mins, Some(300)); assert_eq!(w[1].window_mins, Some(10080));
        assert_eq!(w[0].resets_at.as_deref(), Some("2026-10-06T15:00:00Z"));
    }

    #[test]
    fn codex_usage_windows_and_plan() {
        let b = json!({ "plan_type": "plus", "rate_limit": { "allowed": true, "primary_window": { "used_percent": 88, "limit_window_seconds": 18000, "reset_at": 1791400000 }, "secondary_window": { "used_percent": 31, "limit_window_seconds": 604800, "reset_after_seconds": 3600 } }, "credits": { "has_credits": false } });
        let w = codex_windows(&b);
        assert_eq!(w.iter().map(|x| (x.id.as_str(), x.used_pct, x.window_mins)).collect::<Vec<_>>(), vec![("primary", 88.0, Some(300)), ("secondary", 31.0, Some(10080))]);
        assert_eq!(w[0].resets_at.as_deref(), Some(rfc3339_utc(1791400000).as_str()));
        assert!(w[1].resets_at.is_some());
        assert_eq!(plan_label("plus"), "Plus"); assert_eq!(plan_label("default_claude_max_5x"), "Max 5x"); assert_eq!(plan_label("claude_pro"), "Pro");
    }

    #[test]
    fn gate_quota_windows() {
        let b = json!({ "Subscription": { "Plan": "ultra", "TierName": "Ultra" }, "Groups": [{ "Name": "gemini-3-pro", "Buckets": [{ "RemainingFraction": 0.25, "Window": "5h", "ResetTime": "2026-10-06T15:00:00Z" }] }] });
        let (plan, w) = gate_windows(&b);
        assert_eq!(plan.as_deref(), Some("ultra")); assert_eq!(w.len(), 1); assert_eq!(w[0].id, "gemini-3-pro · 5h"); assert_eq!(w[0].used_pct, 75.0);
        let (p2, w2) = gate_windows(&json!({ "subscription": { "plan": "pro" }, "groups": [{ "name": "g", "buckets": [{ "remaining_fraction": 1.0, "window": "day" }] }] }));
        assert_eq!(p2.as_deref(), Some("pro")); assert_eq!(w2[0].used_pct, 0.0);
    }

    #[test]
    fn jwt_payload_decodes() {
        // header.payload.sig with payload
        // {"exp":1791400000}
        let p = jwt_payload("eyJhbGciOiJSUzI1NiJ9.eyJleHAiOjE3OTE0MDAwMDB9.sig");
        assert_eq!(p.get("exp").and_then(|e| e.as_u64()), Some(1791400000));
    }
}
