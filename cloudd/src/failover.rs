//! Account pool and auto-switching on limits (docs/agents-ux.md §9.7). The fact of a limit hit is set by agentd
//! (`limit_hit` in the session meta: a `rejected` rate_limit_event or a turn error with limit text); the core's
//! agent poller sees the new mark, blocks the account until `resets_at`, takes the next pool account of the same
//! provider, reassigns the session the same way as a manual reassignment (stop → resume with reassign), repeating
//! the owner's last request and leaving a note in the conversation. At session and routine start a blocked default
//! account is also replaced with a free one from the pool. Blocks live in the core's memory: after a restart the
//! limits set them anew.
use crate::accounts::Account;
use crate::model::AgentSessionBrief;
use crate::service::Service;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

/// Without `resets_at` the account is blocked for an hour: the five-hour window usually releases earlier, the weekly
/// one is checked by the limits
pub const DEFAULT_BLOCK_SECS: u64 = 3600;

pub fn now_secs() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

/// RFC3339 "2026-10-06T15:00:00Z" (also with fractional seconds / a ±hh:mm offset) →
/// epoch seconds
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok().map(|t| t.unix_timestamp().max(0) as u64)
}

/// Account provider for the session engine when the session has no
/// account
pub fn provider_of_agent(agent: &str) -> &str {
    match agent { "codex" => "codex", "gemini" => "gemini", _ => "claude" }
}

/// Human-readable ", reset in 1 h 20 min (15:00 UTC)" from RFC3339: the relative time does not depend on the
/// reader's time zone, the absolute one is as in the limits (server UTC)
fn until_text(resets_at: Option<&str>) -> String { until_text_at(resets_at, now_secs()) }
fn until_text_at(resets_at: Option<&str>, now: u64) -> String {
    let Some(t) = resets_at.and_then(parse_rfc3339) else { return String::new() };
    let r = crate::accounts::rfc3339_utc(t); let hm = r.get(11..16).unwrap_or(&r);
    if t <= now { return format!(", окно уже сброшено ({hm} UTC)") }
    let m = (t - now).div_ceil(60);
    let dur = if m < 60 { format!("{m} мин") } else if m.is_multiple_of(60) { format!("{} ч", m / 60) } else { format!("{} ч {} мин", m / 60, m % 60) };
    format!(", сброс через {dur} ({hm} UTC)")
}

/// Next pool account: the same provider, not the current one, not blocked, not exhausted per the limits; the least used
/// first
pub fn pick(accounts: &[Account], provider: &str, exclude: Option<&str>, blocked: &dyn Fn(&str) -> bool, used: &dyn Fn(&str) -> Option<f64>) -> Option<Account> {
    let mut cands: Vec<(f64, Account)> = accounts.iter()
        .filter(|a| a.pool && a.provider == provider && Some(a.id.as_str()) != exclude && !blocked(&a.id))
        .filter_map(|a| { let u = used(&a.id).unwrap_or(0.0); if u >= 100.0 { None } else { Some((u, a.clone())) } })
        .collect();
    cands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.created_at.cmp(&b.1.created_at)));
    cands.into_iter().next().map(|c| c.1)
}

impl Service {
    pub fn block_account(&self, id: &str, until: u64) {
        let snapshot = { let mut b = self.agents.blocked.write().unwrap(); b.insert(id.to_string(), until); b.clone() };
        crate::agents::save_map(&self.store, crate::agents::KV_BLOCKED, &snapshot);
    }
    pub fn unblock_account(&self, id: &str) {
        let snapshot = { let mut b = self.agents.blocked.write().unwrap(); if b.remove(id).is_none() { return } b.clone() };
        crate::agents::save_map(&self.store, crate::agents::KV_BLOCKED, &snapshot);
    }
    /// Until when the account is blocked by a limit hit (expired entries are removed; kv catches up on the next write)
    pub fn account_blocked_until(&self, id: &str) -> Option<u64> {
        let now = now_secs();
        let mut b = self.agents.blocked.write().unwrap();
        match b.get(id).copied() { Some(u) if u > now => Some(u), Some(_) => { b.remove(id); None } None => None }
    }
    /// Blocks for the API: `{id: rfc3339}`
    pub fn blocked_accounts(&self) -> Value {
        let now = now_secs();
        let b = self.agents.blocked.read().unwrap();
        Value::Object(b.iter().filter(|(_, u)| **u > now).map(|(k, u)| (k.clone(), json!(crate::accounts::rfc3339_utc(*u)))).collect())
    }

    /// Maximum usage of the account's windows per the limits cache (subscriptions; unknown for keys
    /// and proxies)
    async fn pool_used(&self, a: &Account) -> Option<f64> {
        if a.kind != "subscription" { return None }
        let l = self.account_limits(&a.id, false).await;
        if l.error.is_some() { return None }
        // a window exhausted per the limits but without a block — block until its resets_at so it is not chosen
        // again
        if let Some(w) = l.windows.iter().find(|w| w.used_pct >= 100.0) { self.block_account(&a.id, w.resets_at.as_deref().and_then(parse_rfc3339).unwrap_or(now_secs() + DEFAULT_BLOCK_SECS)); }
        l.windows.iter().map(|w| w.used_pct).fold(None, |m, u| Some(m.map_or(u, |x: f64| x.max(u))))
    }

    /// Next free pool account for the provider (except `exclude`)
    pub async fn next_pool_account(&self, provider: &str, exclude: Option<&str>) -> Option<Account> {
        let accounts = self.accounts().unwrap_or_default();
        let pool: Vec<&Account> = accounts.iter().filter(|a| a.pool && a.provider == provider && Some(a.id.as_str()) != exclude).collect();
        if pool.is_empty() { return None }
        let mut used = std::collections::HashMap::new();
        for a in &pool { if self.account_blocked_until(&a.id).is_none() { if let Some(u) = self.pool_used(a).await { used.insert(a.id.clone(), u); } } }
        pick(&accounts, provider, exclude, &|id| self.account_blocked_until(id).is_some(), &|id| used.get(id).copied())
    }

    /// Reassign the session account: stop, wait for exit, bring up with resume and the new environment (same journal).
    /// `retry_last` — repeat the owner's last request (the turn rejected by the limit was never executed); `note` — a note into
    /// the conversation.
    pub async fn reassign_session_account(&self, project: &str, id: &str, account: Option<&str>, retry_last: bool, note: Option<&str>) -> Result<Value> {
        let c = self.agentd_client(project).await?;
        let alive = c.get(&format!("/v1/sessions/{id}")).await?.pointer("/info/alive").and_then(|v| v.as_bool()).unwrap_or(false);
        if alive {
            c.delete(&format!("/v1/sessions/{id}")).await?;
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if c.get(&format!("/v1/sessions/{id}")).await?.pointer("/info/alive").and_then(|v| v.as_bool()) == Some(false) { break }
            }
        }
        let mut body = json!({ "resume": id, "reassign": true, "account": account.unwrap_or_default(), "env": {}, "unset": [], "retry_last": retry_last, "note": note });
        self.resolve_session_account(project, &mut body).await?;
        if body.get("account").and_then(|v| v.as_str()) == Some("") { body["account"] = Value::Null; }
        c.post("/v1/sessions", &body).await
    }

    /// Limit hits in a fresh session snapshot: for every new one (by `at`) — a switch in the
    /// background
    pub fn limit_failovers(&self, project: &str, sessions: &[AgentSessionBrief]) {
        for s in sessions {
            let Some(hit) = &s.limit_hit else { continue };
            let at = hit.get("at").and_then(|a| a.as_u64()).unwrap_or(0);
            let key = format!("{project}:{}", s.id);
            let snapshot = {
                let mut seen = self.agents.limit_seen.write().unwrap();
                if seen.get(&key) == Some(&at) { continue }
                let now = now_secs(); seen.retain(|_, t| now.saturating_sub(*t) < crate::agents::LIMIT_SEEN_TTL_SECS);
                seen.insert(key, at); seen.clone()
            };
            crate::agents::save_map(&self.store, crate::agents::KV_LIMIT_SEEN, &snapshot);
            let me = self.clone(); let project = project.to_string(); let s = s.clone();
            tokio::spawn(async move { if let Err(e) = me.failover_session(&project, &s).await { tracing::warn!("{project}: переключение аккаунта сессии {}: {e:#}", &s.id[..8.min(s.id.len())]); } });
        }
    }

    /// Switching one session after a limit hit (§9.7)
    pub async fn failover_session(&self, project: &str, s: &AgentSessionBrief) -> Result<()> {
        let hit = s.limit_hit.clone().unwrap_or(Value::Null);
        let resets_at = hit.get("resets_at").and_then(|r| r.as_str()).map(str::to_string);
        let window = hit.get("window").and_then(|w| w.as_str()).map(str::to_string);
        let accounts = self.accounts()?;
        let cur = s.account.as_deref().and_then(|id| accounts.iter().find(|a| a.id == id));
        let provider = cur.map(|a| a.provider.clone()).unwrap_or_else(|| provider_of_agent(&s.agent).to_string());
        if let Some(a) = cur { self.block_account(&a.id, resets_at.as_deref().and_then(parse_rfc3339).unwrap_or(now_secs() + DEFAULT_BLOCK_SECS)); }
        let short = &s.id[..8.min(s.id.len())];
        let Some(next) = self.next_pool_account(&provider, s.account.as_deref()).await else {
            let until = until_text(resets_at.as_deref());
            let msg = match cur { Some(a) if a.pool => format!("лимит исчерпан, свободных аккаунтов в пуле нет — сессия ждёт{until}"), Some(_) => format!("лимит исчерпан{until}; аккаунт не в пуле, автопереключение не применяется"), None => format!("лимит исчерпан{until}; сессия идёт через gate проекта, свободных аккаунтов в пуле нет") };
            tracing::info!("{project}: сессия {short}: {msg}");
            if let Ok(c) = self.agentd_client(project).await { let _ = c.post(&format!("/v1/sessions/{}/note", s.id), &json!({ "message": msg })).await; }
            self.notify_agent(project, "агент упёрся в лимит", &format!("{project} · {}: {msg}", s.title), "no_entry").await;
            return Ok(());
        };
        // only sessions of pool accounts are switched (a manual account choice outside the pool is
        // the owner's will)
        if let Some(a) = cur { if !a.pool { let _ = self.agentd_client(project).await?.post(&format!("/v1/sessions/{}/note", s.id), &json!({ "message": "лимит исчерпан; аккаунт не в пуле, автопереключение не применяется" })).await; return Ok(()); } }
        let from = cur.map(|a| a.label.clone()).unwrap_or_else(|| "gate проекта".into());
        let note = format!("аккаунт: {from} → {}, лимит{} исчерпан{}", next.label, window.as_deref().map(|w| format!(" {w}")).unwrap_or_default(), until_text(resets_at.as_deref()));
        tracing::info!("{project}: сессия {short}: {note}");
        self.reassign_session_account(project, &s.id, Some(&next.id), true, Some(&note)).await.map_err(|e| anyhow!("переназначение на «{}»: {e:#}", next.label))?;
        let _ = self.store.event("account", &next.id, &format!("{project} · {}: {note}", s.title));
        self.notify_agent(project, "аккаунт переключён", &format!("{project} · {}: {note}", s.title), "arrows_counterclockwise").await;
        Ok(())
    }

    /// Proactive substitution at start: the default account is blocked by a limit hit or exhausted per the limits → a free one
    /// from the pool. Returns a note for the conversation if a substitution happened.
    pub async fn pool_substitute(&self, id: &str) -> Option<(Account, String)> {
        let a = self.account(id).ok()?;
        if !a.pool { return None }
        let others = self.accounts().ok()?.into_iter().any(|x| x.pool && x.provider == a.provider && x.id != a.id);
        if !others { return None }
        let blocked = match self.account_blocked_until(&a.id) { Some(u) => Some(u), None => { self.pool_used(&a).await; self.account_blocked_until(&a.id) } };
        let until = blocked?;
        let next = self.next_pool_account(&a.provider, Some(&a.id)).await?;
        Some((next.clone(), format!("аккаунт: {} → {}, лимит исчерпан{}", a.label, next.label, until_text(Some(&crate::accounts::rfc3339_utc(until))))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(id: &str, provider: &str, pool: bool, created: &str) -> Account {
        Account { id: id.into(), provider: provider.into(), kind: "subscription".into(), label: id.into(), created_at: created.into(), env_keys: vec![], files: vec![], imported_from: None, pool, stamp: 0, models: Vec::new() }
    }

    #[test]
    fn pick_prefers_least_used_pool_account_of_same_provider() {
        let list = vec![acc("a", "claude", true, "1"), acc("b", "claude", true, "2"), acc("c", "claude", false, "3"), acc("d", "codex", true, "4")];
        let used = |id: &str| match id { "a" => Some(80.0), "b" => Some(20.0), _ => None };
        assert_eq!(pick(&list, "claude", Some("a"), &|_| false, &used).map(|a| a.id).as_deref(), Some("b"));
        // the current one is excluded, b is blocked, c is not in
        // the pool → nobody
        assert!(pick(&list, "claude", Some("a"), &|id| id == "b", &used).is_none());
        // an exhausted window (≥100%) is not taken
        assert!(pick(&list, "claude", Some("a"), &|_| false, &|_| Some(100.0)).is_none());
        // without usage data — by creation date
        assert_eq!(pick(&list, "claude", None, &|_| false, &|_| None).map(|a| a.id).as_deref(), Some("a"));
    }

    #[test]
    fn rfc3339_roundtrip_and_until_text() {
        assert_eq!(parse_rfc3339("2026-10-06T15:00:00Z"), Some(1791298800));
        assert_eq!(parse_rfc3339("2026-10-06T15:00:00.5+02:00"), Some(1791291600));
        assert_eq!(parse_rfc3339("вчера"), None);
        let now = 1791298800 - 80 * 60;
        assert_eq!(until_text_at(Some("2026-10-06T15:00:00Z"), now), ", сброс через 1 ч 20 мин (15:00 UTC)");
        assert_eq!(until_text_at(Some("2026-10-06T15:00:00Z"), now + 75 * 60), ", сброс через 5 мин (15:00 UTC)");
        assert_eq!(until_text_at(Some("2026-10-06T15:00:00Z"), now + 20 * 60), ", сброс через 1 ч (15:00 UTC)");
        assert_eq!(until_text_at(Some("2026-10-06T15:00:00Z"), now + 100 * 60), ", окно уже сброшено (15:00 UTC)");
        assert_eq!(until_text_at(None, now), "");
    }
}
