//! Model routes for the simple chat (docs/agents-ux.md §6.1): llm provider gate apps (local and external) and accounts with a
//! key (OpenRouter, OpenAI, Anthropic via an OpenAI-compatible address, a custom address like Ollama). Keys stay in the
//! core: the "Agents" service sees only the route id, the model list and the streaming `chat/completions` proxy.
use crate::service::Service;
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Serialize, Clone, Debug)]
pub struct LlmRoute {
    /// `gate:<app>` | `account:<id>`
    pub id: String,
    /// gate | key
    pub kind: &'static str,
    pub title: String,
    pub provider: String,
    /// Host of the base address — for the label in the UI; the address itself and the key
    /// are not exposed
    pub host: String,
    #[serde(skip)] pub base: String,
    #[serde(skip)] pub key: Option<String>,
}

/// Base of the OpenAI-compatible API: `…/v1` (OpenRouter — `https://openrouter.ai/api/v1`, the gate —
/// `http://<ip>:8317/v1`).
pub fn normalize_base(b: &str) -> String {
    let b = b.trim().trim_end_matches('/');
    if b.ends_with("/v1") { b.to_string() } else { format!("{b}/v1") }
}

fn host_of(base: &str) -> String { base.split("://").nth(1).unwrap_or(base).split('/').next().unwrap_or("").to_string() }

impl Service {
    pub async fn llm_routes(&self) -> Result<Vec<LlmRoute>> {
        let mut out = Vec::new();
        for (t, _) in self.gate_targets().await? {
            let base = normalize_base(&t.base);
            out.push(LlmRoute { id: format!("gate:{}", t.app), kind: "gate", title: t.title, provider: "gate".into(), host: host_of(&base), base, key: t.api_key });
        }
        for a in self.accounts()? {
            if a.kind != "api_key" { continue; }
            let scope = format!("account:{}", a.id);
            let get = |k: &str| self.store.get_secret(&scope, k).ok().flatten().filter(|v| !v.trim().is_empty());
            let (base, key) = match (get("OPENAI_BASE_URL"), get("OPENAI_API_KEY"), get("ANTHROPIC_BASE_URL"), get("ANTHROPIC_API_KEY"), get("ANTHROPIC_AUTH_TOKEN")) {
                (Some(b), k, _, _, _) => (b, k),
                (None, Some(k), _, _, _) => ("https://api.openai.com/v1".to_string(), Some(k)),
                (None, None, b, Some(k), _) | (None, None, b, None, Some(k)) => (b.unwrap_or_else(|| "https://api.anthropic.com".into()), Some(k)),
                _ => continue,
            };
            // gate addresses (`http://llm.gate`) are reachable only from a workspace — from the host such a
            // route does not work
            if base.contains(".gate") || base.contains("localhost") || base.contains("127.0.0.1") { continue; }
            let base = normalize_base(&base);
            out.push(LlmRoute { id: format!("account:{}", a.id), kind: "key", title: a.label.clone(), provider: a.provider.clone(), host: host_of(&base), base, key });
        }
        Ok(out)
    }

    pub async fn llm_route(&self, id: &str) -> Result<LlmRoute> {
        self.llm_routes().await?.into_iter().find(|r| r.id == id).ok_or_else(|| anyhow!("маршрут не найден: {id}"))
    }

    /// Route models: `GET <base>/models` → {id, name, context, price_in, price_out} (prices — OpenRouter, $ per 1M tokens;
    /// context — OpenRouter `context_length` or the Anthropic-format `max_input_tokens`).
    pub async fn llm_models(&self, id: &str) -> Result<Value> {
        let r = self.llm_route(id).await?;
        // gate: real ids (in the Anthropic list the proxy masks non-Claude models) and groups by account
        // channels
        if let Some(app) = r.id.strip_prefix("gate:") {
            let t = self.gate_targets().await?.into_iter().map(|(t, _)| t).find(|t| t.app == app).ok_or_else(|| anyhow!("gate {app} не найден"))?;
            let list: Vec<Value> = self.gate_model_list(&t).await.into_iter().map(|m| json!({ "id": m.get("id"), "name": m.get("name").and_then(|n| n.as_str()).or_else(|| m.get("id").and_then(|i| i.as_str())),
                "context": m.get("context"), "price_in": Value::Null, "price_out": Value::Null, "vision": Value::Null, "tools": Value::Null,
                "provider": m.get("provider"), "provider_title": m.get("provider_title"), "vendor": m.get("owned_by"), "kind": m.get("kind") })).collect();
            return Ok(json!({ "route": r.id, "title": r.title, "models": list }));
        }
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build()?;
        let mut req = http.get(format!("{}/models", r.base)).header("Accept", "application/json").header("anthropic-version", "2023-06-01");
        if let Some(k) = &r.key { req = req.bearer_auth(k).header("x-api-key", k); }
        let resp = req.send().await.map_err(|e| anyhow!("маршрут {}: {e}", r.title))?;
        if !resp.status().is_success() { bail!("маршрут {}: /models {}", r.title, resp.status()); }
        let v: Value = resp.json().await?;
        let per_m = |x: Option<&Value>| x.and_then(|p| p.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| p.as_f64())).map(|p| p * 1_000_000.0);
        let mut list: Vec<Value> = v.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default().iter().filter_map(|m| {
            let mid = m.get("id").and_then(|i| i.as_str())?;
            let context = m.get("context_length").and_then(|c| c.as_u64()).or_else(|| m.get("max_input_tokens").and_then(|c| c.as_u64()));
            let pricing = m.get("pricing");
            // OpenRouter and the like: the vendor is the id prefix ("anthropic/…"), otherwise the owner or the
            // route provider
            let vendor = mid.split_once('/').map(|(p, _)| p.to_string()).or_else(|| m.get("owned_by").and_then(|o| o.as_str()).map(str::to_string)).unwrap_or_else(|| r.provider.clone());
            Some(json!({ "id": mid, "name": m.get("name").or_else(|| m.get("display_name")).and_then(|n| n.as_str()).unwrap_or(mid), "context": context,
                "price_in": per_m(pricing.and_then(|p| p.get("prompt"))), "price_out": per_m(pricing.and_then(|p| p.get("completion"))),
                "vision": m.pointer("/architecture/input_modalities").and_then(|a| a.as_array()).map(|a| a.iter().any(|x| x.as_str() == Some("image"))),
                "tools": m.get("supported_parameters").and_then(|a| a.as_array()).map(|a| a.iter().any(|x| x.as_str() == Some("tools"))),
                "provider": vendor.to_ascii_lowercase(), "provider_title": crate::gateauth::channel_title(&vendor.to_ascii_lowercase()), "vendor": vendor, "kind": crate::gateauth::model_kind(mid) }))
        }).collect();
        list.sort_by_key(|m| m.get("provider").and_then(|p| p.as_str()).unwrap_or("").to_string());
        Ok(json!({ "route": r.id, "title": r.title, "models": list }))
    }

    /// Streaming proxy `POST <base>/chat/completions`: the body as is, the answer (SSE or JSON) — as bytes
    /// outwards.
    pub async fn llm_chat(&self, id: &str, body: &Value) -> Result<reqwest::Response> {
        let r = self.llm_route(id).await?;
        let http = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(15)).timeout(std::time::Duration::from_secs(600)).build()?;
        let mut req = http.post(format!("{}/chat/completions", r.base)).header("Accept", "text/event-stream, application/json").header("HTTP-Referer", "https://agentverse.local").header("X-Title", "AgentVerse OS").json(body);
        if let Some(k) = &r.key { req = req.bearer_auth(k).header("x-api-key", k); }
        req.send().await.map_err(|e| anyhow!("маршрут {}: {e}", r.title))
    }

    /// Image generation on a route (slice 37): the route's `/images/generations`, the key stays in the core; the provider answer
    /// as is.
    pub async fn llm_images(&self, id: &str, body: &Value) -> Result<Value> {
        let r = self.llm_route(id).await?;
        let http = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(15)).timeout(std::time::Duration::from_secs(300)).build()?;
        let mut req = http.post(format!("{}/images/generations", r.base)).header("HTTP-Referer", "https://agentverse.local").header("X-Title", "AgentVerse OS").json(body);
        if let Some(k) = &r.key { req = req.bearer_auth(k).header("x-api-key", k); }
        let resp = req.send().await.map_err(|e| anyhow!("маршрут {}: {e}", r.title))?;
        let status = resp.status(); let text = resp.text().await.unwrap_or_default();
        if !status.is_success() { anyhow::bail!("маршрут {}: {} {}", r.title, status.as_u16(), text.chars().take(300).collect::<String>()) }
        Ok(serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "raw": text })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base_normalization() {
        assert_eq!(normalize_base("https://openrouter.ai/api/v1/"), "https://openrouter.ai/api/v1");
        assert_eq!(normalize_base("https://openrouter.ai/api"), "https://openrouter.ai/api/v1");
        assert_eq!(normalize_base("http://10.0.0.5:8317"), "http://10.0.0.5:8317/v1");
        assert_eq!(host_of("https://openrouter.ai/api/v1"), "openrouter.ai");
    }
}
