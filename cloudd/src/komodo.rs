//! App Runtime v1 — Komodo через API (ADR-005, решение 2). Контракт: install / upgrade / uninstall / status / logs.
//! Типы сверены на стенде (Komodo 2.x): CreateStack{name,config{server_id,project_name,file_contents,environment}},
//! DeployStack/DestroyStack{stack}, UpdateStack{id,config}, DeleteStack{id}, ListStacks, ListStackServices, GetStackLog.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct Komodo {
    base: String,
    key: String,
    secret: String,
    server: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StackStatus {
    pub state: String,
    pub services: Vec<ServiceStatus>,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServiceStatus {
    pub service: String,
    pub image: String,
    pub container: String,
    pub state: String,
}

impl Komodo {
    pub fn new(base: &str, key: &str, secret: &str, server: &str) -> Self {
        Self { base: base.trim_end_matches('/').to_string(), key: key.to_string(), secret: secret.to_string(), server: server.to_string(), http: reqwest::Client::new() }
    }

    async fn call(&self, kind: &str, ty: &str, params: Value) -> Result<Value> {
        let resp = self
            .http
            .post(format!("{}/{kind}", self.base))
            .header("X-Api-Key", &self.key)
            .header("X-Api-Secret", &self.secret)
            .json(&json!({ "type": ty, "params": params }))
            .send()
            .await
            .with_context(|| format!("komodo {kind}/{ty}"))?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(anyhow!("komodo {ty}: HTTP {status}: {}", text.chars().take(400).collect::<String>()));
        }
        serde_json::from_str(&text).with_context(|| format!("komodo {ty}: не JSON: {}", text.chars().take(200).collect::<String>()))
    }

    pub async fn ping(&self) -> Result<String> {
        let v = self.call("read", "GetVersion", json!({})).await?;
        Ok(v.get("version").and_then(|x| x.as_str()).unwrap_or("?").to_string())
    }

    fn oid(v: &Value) -> Option<String> {
        v.get("_id").and_then(|id| id.get("$oid").and_then(|o| o.as_str()).or_else(|| id.as_str())).map(|s| s.to_string())
    }

    pub async fn get_stack(&self, name: &str) -> Result<Option<Value>> {
        match self.call("read", "GetStack", json!({ "stack": name })).await {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.to_string().contains("HTTP 404") || e.to_string().to_lowercase().contains("not found") || e.to_string().contains("Did not find") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// install: CreateStack с содержимым compose (сеть уже создана ядром и помечена external) и env из секрет-стора.
    pub async fn create_stack(&self, name: &str, compose: &str, env: &str) -> Result<String> {
        let v = self
            .call(
                "write",
                "CreateStack",
                // стеки с `build:` (образ собирается на месте, как pipecat-voice): run_build — иначе Komodo не собирает; auto_pull=false —
                // иначе `compose pull` пытается скачать локальный образ из реестра и валит деплой. Остальным стекам pull оставляем.
                json!({ "name": name, "config": { "server_id": self.server, "project_name": name, "file_contents": compose, "environment": env, "run_build": has_build(compose), "auto_pull": !has_build(compose) } }),
            )
            .await?;
        Self::oid(&v).ok_or_else(|| anyhow!("CreateStack: нет _id в ответе"))
    }
    pub async fn update_stack(&self, id: &str, compose: &str, env: &str) -> Result<()> {
        self.call("write", "UpdateStack", json!({ "id": id, "config": { "file_contents": compose, "environment": env, "run_build": has_build(compose), "auto_pull": !has_build(compose) } })).await?;
        Ok(())
    }
    pub async fn deploy(&self, name: &str) -> Result<()> {
        self.call("execute", "DeployStack", json!({ "stack": name })).await?;
        Ok(())
    }
    pub async fn destroy(&self, name: &str) -> Result<()> {
        self.call("execute", "DestroyStack", json!({ "stack": name })).await?;
        Ok(())
    }
    /// Сразу после deploy Komodo ещё занят стеком («Stack busy») — повторяем несколько раз.
    pub async fn delete_stack(&self, id: &str) -> Result<()> {
        let mut last = None;
        for _ in 0..10 {
            match self.call("write", "DeleteStack", json!({ "id": id })).await {
                Ok(_) => return Ok(()),
                Err(e) if format!("{e:#}").contains("busy") => {
                    last = Some(e);
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("DeleteStack: стек занят")))
    }

    /// Состояния всех стеков одним ListStacks: для списка приложений (иначе по запросу на каждое установленное).
    pub async fn states(&self) -> Result<std::collections::HashMap<String, String>> {
        let list = self.call("read", "ListStacks", json!({})).await?;
        Ok(list.as_array().map(|a| a.iter().filter_map(|s| Some((s.get("name")?.as_str()?.to_string(), s.pointer("/info/state").and_then(|x| x.as_str()).unwrap_or("unknown").to_string()))).collect()).unwrap_or_default())
    }

    /// Состояние — из ListStacks/ListStackServices (в GetStack.info.state пусто — стенд).
    pub async fn status(&self, name: &str) -> Result<Option<StackStatus>> {
        let list = self.call("read", "ListStacks", json!({})).await?;
        let Some(st) = list.as_array().and_then(|a| a.iter().find(|s| s.get("name").and_then(|n| n.as_str()) == Some(name))) else {
            return Ok(None);
        };
        let state = st.pointer("/info/state").and_then(|s| s.as_str()).unwrap_or("unknown").to_string();
        let services = self.call("read", "ListStackServices", json!({ "stack": name })).await.unwrap_or(Value::Array(vec![]));
        let services = services
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| ServiceStatus {
                        service: s.get("service").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        image: s.get("image").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        container: s.pointer("/container/name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        state: s.pointer("/container/state").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(StackStatus { state, services }))
    }

    pub async fn logs(&self, name: &str, services: &[String], tail: u32) -> Result<String> {
        let v = self.call("read", "GetStackLog", json!({ "stack": name, "services": services, "tail": tail })).await?;
        Ok(v.get("stdout").and_then(|s| s.as_str()).unwrap_or("").to_string() + v.get("stderr").and_then(|s| s.as_str()).unwrap_or(""))
    }

    /// Ждать, пока стек станет `running` (deploy асинхронный, отвечает "InProgress").
    pub async fn wait_running(&self, name: &str, timeout_s: u64) -> Result<StackStatus> {
        let start = std::time::Instant::now();
        loop {
            if let Some(s) = self.status(name).await? {
                if s.state == "running" {
                    return Ok(s);
                }
                if s.state == "unhealthy" || s.state == "down" && start.elapsed().as_secs() > 20 {
                    return Err(anyhow!("стек {name}: состояние {}", s.state));
                }
            }
            if start.elapsed().as_secs() > timeout_s {
                return Err(anyhow!("стек {name}: не стал running за {timeout_s} с"));
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}

/// Есть ли в compose сервисы с `build:` (образ собирается на хосте).
fn has_build(compose: &str) -> bool {
    compose.lines().any(|l| l.trim_start().starts_with("build:"))
}
