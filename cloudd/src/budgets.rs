//! Spending limits in action (docs/agents-ux.md §8.3, slice 67): daily limits per workspace and a total — the owner sets
//! the numbers in "Models" (the service mirrors `settings.limits` here, kv `agents.budgets`), and the core applies them
//! as the single entry point of turns: before `prompt`/`queue` the usage of the last 24 hours per the ledger is checked
//! against the limits. "Stop" — a 402 refusal with a clear text; "warn" — the turn goes through, a feed event and a
//! notification once on crossing (repeated at most once per 6 h). Routines ask for the state in advance and mark the run
//! `skipped`.
use crate::service::Service;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;

pub const KEY: &str = "agents.budgets";
/// The "day" window is a sliding 24 hours, like "today" in the
/// "Models" section
pub const WINDOW_MS: u64 = 86_400_000;
/// A repeated warning on the same limit — at most once per 6 hours
const REWARN_MS: u64 = 6 * 3_600_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    /// `total` or a workspace name
    pub scope: String,
    #[serde(default)]
    pub per_day: f64,
    /// `warn` | `stop`
    #[serde(default = "warn")]
    pub action: String,
}
fn warn() -> String { "warn".into() }

/// Limit state for the UI/routines: 24 h usage, share, whether new turns are
/// blocked
#[derive(Debug, Clone, Serialize)]
pub struct BudgetState {
    pub scope: String,
    pub per_day: f64,
    pub action: String,
    pub spent: f64,
    pub pct: f64,
    pub blocked: bool,
}

#[derive(Default)]
pub struct BudgetsState {
    /// When the limit was last warned about: scope → ms; kv `agents.budget_warned`, so a core restart does not send the warning again
    warned: Mutex<HashMap<String, u64>>,
}

impl BudgetsState {
    pub fn load(store: &crate::store::Store) -> Self {
        Self { warned: Mutex::new(crate::agents::load_map(store, crate::agents::KV_BUDGET_WARNED)) }
    }
}

pub fn money(v: f64) -> String { format!("${v:.2}") }

/// Limits that apply to the project: its own and the total; `per_day` ≤ 0 — off
pub fn applicable<'a>(budgets: &'a [Budget], project: &str) -> Vec<&'a Budget> {
    budgets.iter().filter(|b| b.per_day > 0.0 && (b.scope == "total" || b.scope == project)).collect()
}

impl Service {
    pub fn budgets(&self) -> Result<Vec<Budget>> {
        Ok(self.store.get_kv(KEY)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
    }
    pub fn put_budgets(&self, list: &[Budget]) -> Result<()> {
        for b in list { if b.scope.trim().is_empty() { bail!("лимит требует scope (total или workspace)"); } if b.action != "warn" && b.action != "stop" { bail!("действие лимита: warn или stop"); } if !b.per_day.is_finite() || b.per_day < 0.0 { bail!("лимит требует неотрицательную сумму"); } }
        // a changed or new limit — its warning must come anew rather than wait 6 h from the previous one
        let old = self.budgets().unwrap_or_default();
        let changed: Vec<String> = list.iter().filter(|b| !old.iter().any(|o| o.scope == b.scope && o.per_day == b.per_day && o.action == b.action)).map(|b| b.scope.clone()).collect();
        if !changed.is_empty() {
            let snapshot = { let mut w = self.budgets_st.warned.lock().unwrap(); let mut touched = false; for sc in &changed { touched |= w.remove(sc).is_some(); } if touched { Some(w.clone()) } else { None } };
            if let Some(snapshot) = snapshot { crate::agents::save_map(&self.store, crate::agents::KV_BUDGET_WARNED, &snapshot); }
        }
        self.store.put_kv(KEY, &json!(list))
    }

    /// 24-hour usage per scope from the ledger: `total` and per
    /// project
    fn spent_24h(&self) -> Result<HashMap<String, f64>> {
        let since = crate::agents::now_ms().saturating_sub(WINDOW_MS);
        let s = self.store.usage_summary(since)?;
        let mut m: HashMap<String, f64> = s.by_project.into_iter().map(|b| (b.key, b.totals.cost_usd)).collect();
        m.insert("total".into(), s.total.cost_usd);
        Ok(m)
    }

    /// State of all limits (for "Models", the composer and
    /// routines)
    pub fn budgets_state(&self) -> Result<Vec<BudgetState>> {
        let spent = self.spent_24h()?;
        Ok(self.budgets()?.into_iter().map(|b| {
            let sp = *spent.get(&b.scope).unwrap_or(&0.0);
            let pct = if b.per_day > 0.0 { sp / b.per_day * 100.0 } else { 0.0 };
            BudgetState { blocked: b.per_day > 0.0 && b.action == "stop" && sp >= b.per_day, scope: b.scope, per_day: b.per_day, action: b.action, spent: sp, pct }
        }).collect())
    }

    /// Before a turn: "stop" with an exhausted limit — an error; "warn" — an event and a notification on crossing
    pub async fn budget_check(&self, project: &str) -> Result<()> {
        let budgets = self.budgets()?;
        let appl = applicable(&budgets, project);
        if appl.is_empty() { return Ok(()) }
        let spent = self.spent_24h()?;
        let now = crate::agents::now_ms();
        for b in appl {
            let sp = *spent.get(&b.scope).unwrap_or(&0.0);
            if sp < b.per_day {
                let snapshot = { let mut w = self.budgets_st.warned.lock().unwrap(); if w.remove(&b.scope).is_none() { continue } w.clone() };
                crate::agents::save_map(&self.store, crate::agents::KV_BUDGET_WARNED, &snapshot); continue;
            }
            let who = if b.scope == "total" { "всего".to_string() } else { format!("для {}", b.scope) };
            if b.action == "stop" {
                bail!("дневной лимит {} {who} исчерпан ({} за 24 ч): новые ходы остановлены, пока расход не снизится; лимит — в «Моделях»", money(b.per_day), money(sp));
            }
            let due = { let mut w = self.budgets_st.warned.lock().unwrap(); let last = w.get(&b.scope).copied().unwrap_or(0); if now.saturating_sub(last) >= REWARN_MS { w.insert(b.scope.clone(), now); Some(w.clone()) } else { None } };
            if let Some(snapshot) = due {
                crate::agents::save_map(&self.store, crate::agents::KV_BUDGET_WARNED, &snapshot);
                self.notify_agent(project, "дневной лимит превышен", &format!("{project} · лимит {} {who}: {} за 24 ч — ходы идут, действие «предупредить»", money(b.per_day), money(sp)), "warning").await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn applicable_filters_scope_and_zero() {
        let list = vec![Budget { scope: "total".into(), per_day: 20.0, action: "warn".into() }, Budget { scope: "gamma".into(), per_day: 5.0, action: "stop".into() }, Budget { scope: "alpha".into(), per_day: 0.0, action: "stop".into() }, Budget { scope: "beta".into(), per_day: 2.0, action: "stop".into() }];
        let a: Vec<&str> = applicable(&list, "gamma").iter().map(|b| b.scope.as_str()).collect();
        assert_eq!(a, vec!["total", "gamma"]);
        assert_eq!(applicable(&list, "alpha").len(), 1);
        assert_eq!(money(5.1), "$5.10");
    }
}
