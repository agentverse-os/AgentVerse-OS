//! Model prices (docs/agents-ux.md §9.6): the `settings.prices` reference ($ per 1M tokens) and synchronization with the
//! public OpenRouter model list (`GET /api/v1/models`, no key) — input, output, cache read and cache write prices for
//! Anthropic and OpenAI. Our ids are what the CLIs call them (`claude-opus-5-5`, `gpt-6.1-sol`); mapping to the source ids —
//! `candidates()`.
use crate::{App, ApiError};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_SOURCE: &str = "https://openrouter.ai/api/v1/models";

/// Default prices for a fresh installation (2026-10-15, checked against platform.claude.com and OpenRouter): input, output, cache
/// read, cache write.
pub fn default_prices() -> Value {
    json!({
        "claude-fable-5-1": { "input": 10.0, "output": 50.0, "cache_read": 0.25, "cache_write": 12.5, "context": 200000 },
        "claude-opus-5-5": { "input": 4.0, "output": 20.0, "cache_read": 0.20, "cache_write": 5.0, "context": 200000 },
        "claude-opus-5": { "input": 5.0, "output": 25.0, "cache_read": 0.50, "cache_write": 6.25, "context": 200000 },
        "claude-sonnet-5-5": { "input": 2.0, "output": 10.0, "cache_read": 0.10, "cache_write": 2.5, "context": 200000 },
        "claude-sonnet-5": { "input": 2.0, "output": 10.0, "cache_read": 0.20, "cache_write": 2.5, "context": 200000 },
        "claude-haiku-5-5": { "input": 0.10, "output": 0.50, "cache_read": 0.01, "cache_write": 0.125, "context": 200000 },
        "claude-haiku-4-5-20251001": { "input": 1.0, "output": 5.0, "cache_read": 0.10, "cache_write": 1.25, "context": 200000 },
        "gpt-6-astra": { "input": 10.0, "output": 50.0, "cache_read": 1.0, "cache_write": 12.5, "context": 1050000 },
        "gpt-6.1-sol": { "input": 2.0, "output": 10.0, "cache_read": 0.10, "cache_write": 2.5, "context": 1050000 },
        "gpt-6-sol": { "input": 2.0, "output": 10.0, "cache_read": 0.20, "cache_write": 2.5, "context": 1050000 },
        "gpt-6-luna": { "input": 0.10, "output": 0.50, "cache_read": 0.01, "cache_write": 0.125, "context": 1050000 },
        "gpt-5.6-terra": { "input": 2.0, "output": 12.0, "cache_read": 0.20, "cache_write": 2.5, "context": 400000 },
        "gpt-oss-120b-medium": { "input": 0.037, "output": 0.17, "cache_read": 0.0, "cache_write": 0.0, "context": 131072 }
    })
}

/// Base id for the price lookup: without `[1m]`, `:batch` and the effort suffix of
/// open-weight models.
pub fn base_id(id: &str) -> String {
    let mut s = id.trim().to_string();
    for suf in ["[1m]", ":batch"] { if let Some(k) = s.strip_suffix(suf) { s = k.to_string(); } }
    s
}

/// Possible ids in the source space (OpenRouter: `vendor/model`), in decreasing precision.
pub fn candidates(id: &str) -> Vec<String> {
    let s = base_id(id);
    let mut out = Vec::new();
    if s.contains('/') { out.push(s.clone()); }
    if let Some(rest) = s.strip_prefix("claude-") {
        // claude-opus-5-5 → opus-5.5; claude-haiku-4-5-20251001 → haiku-4.5; claude-3-5-haiku-20241022 → 3.5-haiku; claude-opus-5 → opus-5
        let parts: Vec<&str> = rest.split('-').filter(|p| !(p.len() == 8 && p.chars().all(|c| c.is_ascii_digit()))).collect();
        let mut joined = String::new();
        for (i, p) in parts.iter().enumerate() {
            let prev_num = i > 0 && parts[i - 1].chars().all(|c| c.is_ascii_digit());
            let num = p.chars().all(|c| c.is_ascii_digit());
            if i > 0 { joined.push(if prev_num && num { '.' } else { '-' }); }
            joined.push_str(p);
        }
        out.push(format!("anthropic/claude-{joined}"));
        out.push(format!("anthropic/claude-{}", rest));
    } else if s.starts_with("gpt-") || s.starts_with("o1") || s.starts_with("o3") || s.starts_with("o4") || s.starts_with("codex") {
        out.push(format!("openai/{s}"));
        // gpt-oss-120b-medium → gpt-oss-120b (the effort is not
        // the model)
        for suf in ["-minimal", "-low", "-medium", "-high", "-xhigh"] { if let Some(k) = s.strip_suffix(suf) { out.push(format!("openai/{k}")); } }
    } else if s.starts_with("gemini-") {
        out.push(format!("google/{s}"));
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Price { pub input: f64, pub output: f64, pub cache_read: f64, pub cache_write: f64, pub context: Option<u64> }

/// Default context windows (tokens; 2026-10-16, OpenRouter): Codex 0.160 does not know them for every model and takes
/// 272k — on Astra that gave "284k of 258k". The value goes to Codex as `model_context_window` (see agentd codex.rs).
pub fn default_context(id: &str) -> Option<u64> {
    let b = base_id(id);
    if b.starts_with("gpt-6") { return Some(1_050_000) }
    if b.starts_with("gpt-5.6") { return Some(400_000) }
    if b.starts_with("gpt-oss") { return Some(131_072) }
    if b.starts_with("claude-") { return Some(200_000) }
    None
}

/// Parsing the source answer: id → price per 1M tokens (OpenRouter has per-token prices as
/// strings).
pub fn parse_source(v: &Value) -> BTreeMap<String, Price> {
    let per_m = |p: &Value, k: &str| -> f64 { p.get(k).and_then(|x| x.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| x.as_f64())).map(|x| (x * 1e6 * 10000.0).round() / 10000.0).unwrap_or(0.0) };
    let mut out = BTreeMap::new();
    for m in v.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default() {
        let (Some(id), Some(p)) = (m.get("id").and_then(|i| i.as_str()), m.get("pricing")) else { continue };
        let pr = Price { input: per_m(p, "prompt"), output: per_m(p, "completion"), cache_read: per_m(p, "input_cache_read"), cache_write: per_m(p, "input_cache_write"), context: m.get("context_length").and_then(|c| c.as_u64()).filter(|c| *c > 0) };
        if pr.input < 0.0 || pr.output < 0.0 { continue } // "-1" for dynamic prices
        out.insert(id.to_string(), pr);
    }
    out
}

pub fn lookup<'a>(src: &'a BTreeMap<String, Price>, id: &str) -> Option<(&'a str, &'a Price)> {
    for c in candidates(id) { if let Some((k, p)) = src.get_key_value(&c) { return Some((k.as_str(), p)); } }
    None
}

/// Merge: candidates = table keys ∪ used models ∪ route models; found ones are overwritten (including "subscription"), not found
/// stay, new not-found ones from the expense ledger are added with a "no price" mark. Returns (table, report).
pub fn merge(prices: &Value, used: &[String], src: &BTreeMap<String, Price>, now: u64) -> (Value, Value) {
    let mut table = prices.as_object().cloned().unwrap_or_default();
    let mut ids: Vec<String> = table.keys().cloned().collect();
    for u in used { let b = base_id(u); if !b.is_empty() && b != "?" && !ids.contains(&b) { ids.push(b); } }
    let (mut updated, mut added, mut missing) = (Vec::new(), Vec::new(), Vec::new());
    for id in ids {
        match lookup(src, &id) {
            Some((k, p)) => {
                let existed = table.contains_key(&id);
                // context window: from the source; none — the previous table value or the family
                // default
                let ctx = p.context.or_else(|| table.get(&id).and_then(|v| v.get("context")).and_then(|c| c.as_u64())).or_else(|| default_context(&id));
                let mut row = json!({ "input": p.input, "output": p.output, "cache_read": p.cache_read, "cache_write": p.cache_write, "source": "openrouter", "source_id": k, "synced_at": now });
                if let Some(c) = ctx { row["context"] = json!(c); }
                table.insert(id.clone(), row);
                if existed { updated.push(id); } else { added.push(id); }
            }
            None => {
                if !table.contains_key(&id) { table.insert(id.clone(), json!({ "note": "нет цены" })); }
                missing.push(id);
            }
        }
    }
    (Value::Object(table), json!({ "updated": updated, "added": added, "missing": missing }))
}

fn source_url(app: &App) -> String {
    crate::api::settings_value(app).get("prices_source").and_then(|s| s.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| DEFAULT_SOURCE.to_string())
}

/// `POST /agents/api/prices/sync` — source → price table; the report and timestamp are saved in the settings (`prices_sync`).
pub async fn sync(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    let url = source_url(&app);
    let r = app.http.get(&url).timeout(Duration::from_secs(40)).send().await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("источник цен: {e}")))?;
    if !r.status().is_success() { return Err(ApiError(StatusCode::BAD_GATEWAY, format!("источник цен: HTTP {}", r.status().as_u16()))); }
    let v: Value = r.json().await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("источник цен: неверный JSON: {e}")))?;
    let src = parse_source(&v);
    if src.is_empty() { return Err(ApiError(StatusCode::BAD_GATEWAY, "источник цен: пустой список моделей".into())); }
    // candidates: the built-in catalog of current models, those used over the year, the gate route
    // models
    let mut used: Vec<String> = default_prices().as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    if let Ok(u) = app.core_get("/usage?days=365").await {
        for b in u.get("by_model").and_then(|m| m.as_array()).cloned().unwrap_or_default() { if let Some(k) = b.get("key").and_then(|k| k.as_str()) { used.push(k.to_string()); } }
    }
    if let Ok(rt) = app.core_get("/llm/routes").await {
        for r in rt.get("routes").and_then(|r| r.as_array()).cloned().unwrap_or_default() {
            for m in r.get("models").and_then(|m| m.as_array()).cloned().unwrap_or_default() { if let Some(id) = m.get("id").and_then(|i| i.as_str()).or_else(|| m.as_str()) { used.push(id.to_string()); } }
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut settings = crate::api::settings_value(&app);
    let (table, report) = merge(settings.get("prices").unwrap_or(&json!({})), &used, &src, now);
    let n = table.as_object().map(|o| o.len()).unwrap_or(0);
    settings["prices"] = table;
    settings["prices_sync"] = json!({ "at": now, "source": url, "models": n, "updated": report["updated"].as_array().map(|a| a.len()).unwrap_or(0), "added": report["added"].as_array().map(|a| a.len()).unwrap_or(0), "missing": report["missing"].clone() });
    app.store.put_kv("settings", &settings).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "ok": true, "at": now, "source": settings["prices_sync"]["source"], "models": n, "updated": report["updated"], "added": report["added"], "missing": report["missing"] })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_map_to_source_namespace() {
        assert_eq!(candidates("claude-opus-5-5")[0], "anthropic/claude-opus-5.5");
        assert_eq!(candidates("claude-opus-5-5[1m]")[0], "anthropic/claude-opus-5.5");
        assert_eq!(candidates("claude-haiku-4-5-20251001")[0], "anthropic/claude-haiku-4.5");
        assert_eq!(candidates("claude-3-5-haiku-20241022")[0], "anthropic/claude-3.5-haiku");
        assert_eq!(candidates("claude-opus-5")[0], "anthropic/claude-opus-5");
        assert_eq!(candidates("claude-fable-5-1")[0], "anthropic/claude-fable-5.1");
        assert_eq!(candidates("gpt-6.1-sol"), vec!["openai/gpt-6.1-sol"]);
        assert_eq!(candidates("gpt-oss-120b-medium"), vec!["openai/gpt-oss-120b-medium", "openai/gpt-oss-120b"]);
        assert_eq!(candidates("gemini-3-pro")[0], "google/gemini-3-pro");
        assert_eq!(candidates("devin/swe-2"), vec!["devin/swe-2"]);
        assert!(candidates("?").is_empty());
    }

    #[test]
    fn source_parses_per_token_strings_and_merge_reports() {
        let v = json!({ "data": [
            { "id": "anthropic/claude-opus-5.5", "context_length": 200000, "pricing": { "prompt": "0.000004", "completion": "0.00002", "input_cache_read": "0.0000002", "input_cache_write": "0.000005" } },
            { "id": "openai/gpt-oss-120b", "pricing": { "prompt": "0.000000037", "completion": "0.00000017" } },
            { "id": "openrouter/auto", "pricing": { "prompt": "-1", "completion": "-1" } }
        ]});
        let src = parse_source(&v);
        assert_eq!(src["anthropic/claude-opus-5.5"], Price { input: 4.0, output: 20.0, cache_read: 0.2, cache_write: 5.0, context: Some(200000) });
        assert_eq!(src["openai/gpt-oss-120b"].input, 0.037); assert_eq!(src["openai/gpt-oss-120b"].context, None);
        assert!(!src.contains_key("openrouter/auto"));
        let prices = json!({ "claude-opus-5-5": { "input": 15.0, "output": 75.0, "cache_read": 1.5, "cache_write": 18.75, "context": 200000 }, "gpt-6-astra": { "input": 0.0, "output": 0.0, "note": "подписка", "context": 1050000 } });
        let (table, report) = merge(&prices, &["claude-opus-5-5[1m]".into(), "gpt-oss-120b-medium".into(), "devin/swe-2".into(), "?".into()], &src, 1700000000);
        assert_eq!(table["claude-opus-5-5"]["input"], 4.0); assert_eq!(table["claude-opus-5-5"]["source_id"], "anthropic/claude-opus-5.5");
        assert_eq!(table["gpt-oss-120b-medium"]["input"], 0.037);
        assert_eq!(table["claude-opus-5-5"]["context"], 200000, "окно из источника");
        assert_eq!(table["gpt-oss-120b-medium"]["context"], 131072, "источник без окна — умолчание семейства");
        assert_eq!(table["gpt-6-astra"]["note"], "подписка", "не найденная остаётся как была");
        assert_eq!(default_context("gpt-6-astra"), Some(1_050_000)); assert_eq!(default_context("gpt-6-astra[1m]"), Some(1_050_000)); assert_eq!(default_context("devin/swe-2"), None);
        assert_eq!(table["devin/swe-2"]["note"], "нет цены");
        assert!(table.get("?").is_none() && table.get("claude-opus-5-5[1m]").is_none());
        assert_eq!(report["updated"], json!(["claude-opus-5-5"])); assert_eq!(report["added"], json!(["gpt-oss-120b-medium"]));
        assert_eq!(report["missing"], json!(["gpt-6-astra", "devin/swe-2"]));
    }

    #[test]
    fn defaults_cover_current_models() {
        let d = default_prices();
        for id in ["claude-fable-5-1", "claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5", "gpt-6-astra", "gpt-6.1-sol"] { assert!(d.get(id).is_some(), "{id}"); }
        assert_eq!(d["claude-fable-5-1"]["cache_read"], 0.25); assert_eq!(d["claude-opus-5-5"]["output"], 20.0);
    }
}
