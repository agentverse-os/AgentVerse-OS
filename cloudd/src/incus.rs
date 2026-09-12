//! Минимальный клиент Incus REST API через unix-сокет: сети проектов и чтение инстансов.
//! Workspace'ы создаёт Coder (через провайдер lxc/incus), ядро трогает только сети и читает состояние.

use anyhow::{anyhow, Context, Result};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::client::legacy::Client;
use hyperlocal::{UnixClientExt, UnixConnector, Uri};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct Incus {
    socket: PathBuf,
    client: Client<UnixConnector, Full<Bytes>>,
}

#[derive(Debug, serde::Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    status_code: u16,
    #[serde(default)]
    error: String,
    #[serde(default)]
    metadata: Value,
}

impl Incus {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket, client: Client::unix() }
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, path: &str, body: Option<Value>) -> Result<T> {
        let uri: hyper::Uri = Uri::new(&self.socket, path).into();
        let req = hyper::Request::builder()
            .method(method)
            .uri(uri)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body.map(|b| b.to_string()).unwrap_or_default())))?;
        let resp = self.client.request(req).await.with_context(|| format!("incus {method} {path}"))?;
        let status = resp.status();
        let bytes = resp.into_body().collect().await?.to_bytes();
        let env: Envelope = serde_json::from_slice(&bytes).with_context(|| format!("incus {path}: не JSON: {}", String::from_utf8_lossy(&bytes)))?;
        if env.kind == "error" || status.is_client_error() || status.is_server_error() {
            return Err(anyhow!("incus {method} {path}: {} ({})", env.error, if env.status_code > 0 { env.status_code } else { status.as_u16() }));
        }
        if env.kind == "async" {
            // дождаться операции
            let op_id = env.metadata.get("id").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("incus: async без id"))?.to_string();
            let done: Envelope = self.call_raw("GET", &format!("/1.0/operations/{op_id}/wait?timeout=120")).await?;
            let st = done.metadata.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if st != "Success" {
                return Err(anyhow!("incus operation {op_id}: {} — {}", st, done.metadata.get("err").and_then(|v| v.as_str()).unwrap_or("")));
            }
            return serde_json::from_value(done.metadata).map_err(Into::into);
        }
        serde_json::from_value(env.metadata).map_err(Into::into)
    }
    async fn call_raw(&self, method: &str, path: &str) -> Result<Envelope> {
        let uri: hyper::Uri = Uri::new(&self.socket, path).into();
        let req = hyper::Request::builder().method(method).uri(uri).body(Full::new(Bytes::new()))?;
        let resp = self.client.request(req).await?;
        let bytes = resp.into_body().collect().await?.to_bytes();
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub async fn ping(&self) -> Result<String> {
        let v: Value = self.call("GET", "/1.0", None).await?;
        Ok(v.get("environment").and_then(|e| e.get("server_version")).and_then(|v| v.as_str()).unwrap_or("?").to_string())
    }

    pub async fn network_exists(&self, name: &str) -> Result<bool> {
        match self.call::<Value>("GET", &format!("/1.0/networks/{name}"), None).await {
            Ok(_) => Ok(true),
            Err(e) if e.to_string().contains("(404)") || e.to_string().contains("not found") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Управляемый мост проекта: `ipv4.address=<gw>/24`, NAT, без IPv6 (как в спайке).
    pub async fn ensure_network(&self, name: &str, gateway_cidr: &str, dhcp_range: &str) -> Result<()> {
        if self.network_exists(name).await? {
            return Ok(());
        }
        let body = json!({
            "name": name, "type": "bridge",
            "description": "Cloud OS project network (managed by cloudd)",
            "config": {
                "ipv4.address": gateway_cidr, "ipv4.nat": "true", "ipv6.address": "none",
                // адреса .240–.254 зарезервированы под gate (macvlan Docker) — DHCP их не выдаёт
                "ipv4.dhcp.ranges": dhcp_range,
            }
        });
        self.call::<Value>("POST", "/1.0/networks", Some(body)).await?;
        Ok(())
    }

    /// PATCH конфигурации сети (например, `raw.dnsmasq` для `*.gate`).
    pub async fn patch_network(&self, name: &str, config: Value) -> Result<()> {
        let current: Value = self.call("GET", &format!("/1.0/networks/{name}"), None).await?;
        let mut cfg = current.get("config").cloned().unwrap_or_else(|| json!({}));
        let mut changed = false;
        if let (Some(dst), Some(src)) = (cfg.as_object_mut(), config.as_object()) {
            for (k, v) in src {
                if dst.get(k) != Some(v) {
                    dst.insert(k.clone(), v.clone());
                    changed = true;
                }
            }
        }
        if changed {
            self.call::<Value>("PATCH", &format!("/1.0/networks/{name}"), Some(json!({ "config": cfg }))).await?;
        }
        Ok(())
    }

    pub async fn delete_network(&self, name: &str) -> Result<()> {
        if !self.network_exists(name).await? {
            return Ok(());
        }
        self.call::<Value>("DELETE", &format!("/1.0/networks/{name}"), None).await?;
        Ok(())
    }

    pub async fn instance_running(&self, name: &str) -> Result<bool> {
        match self.call::<Value>("GET", &format!("/1.0/instances/{name}"), None).await {
            Ok(v) => Ok(v.get("status").and_then(|s| s.as_str()) == Some("Running")),
            Err(e) if e.to_string().contains("(404)") || e.to_string().contains("not found") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Записать файл в инстанс (`POST /1.0/instances/{name}/files?path=`), создавая каталоги.
    pub async fn file_push(&self, instance: &str, path: &str, content: &[u8], mode: u32) -> Result<()> {
        if let Some(dir) = std::path::Path::new(path).parent() {
            self.exec(instance, &["mkdir", "-p", &dir.to_string_lossy()]).await?;
        }
        let uri: hyper::Uri = Uri::new(&self.socket, &format!("/1.0/instances/{instance}/files?path={}", urlenc(path))).into();
        let req = hyper::Request::builder()
            .method("POST")
            .uri(uri)
            .header("Content-Type", "application/octet-stream")
            .header("X-Incus-type", "file")
            .header("X-Incus-mode", format!("{mode:o}"))
            .header("X-Incus-uid", "0")
            .header("X-Incus-gid", "0")
            .header("X-Incus-write", "overwrite")
            .body(Full::new(Bytes::copy_from_slice(content)))?;
        let resp = self.client.request(req).await.with_context(|| format!("incus file push {instance}:{path}"))?;
        let status = resp.status();
        let bytes = resp.into_body().collect().await?.to_bytes();
        if !status.is_success() {
            return Err(anyhow!("incus file push {instance}:{path}: {} {}", status, String::from_utf8_lossy(&bytes)));
        }
        Ok(())
    }

    /// Прочитать файл из инстанса (`GET /1.0/instances/{name}/files?path=`).
    pub async fn file_pull(&self, instance: &str, path: &str) -> Result<Vec<u8>> {
        let uri: hyper::Uri = Uri::new(&self.socket, &format!("/1.0/instances/{instance}/files?path={}", urlenc(path))).into();
        let req = hyper::Request::builder().method("GET").uri(uri).body(Full::new(Bytes::new()))?;
        let resp = self.client.request(req).await.with_context(|| format!("incus file pull {instance}:{path}"))?;
        let status = resp.status();
        let is_dir = resp.headers().get("X-Incus-type").and_then(|v| v.to_str().ok()) == Some("directory");
        let bytes = resp.into_body().collect().await?.to_bytes();
        if !status.is_success() {
            return Err(anyhow!("incus file pull {instance}:{path}: {} {}", status, String::from_utf8_lossy(&bytes)));
        }
        if is_dir {
            return Err(anyhow!("{path}: это каталог"));
        }
        Ok(bytes.to_vec())
    }

    /// Выполнить команду в инстансе без websocket; возвращает stdout (record-output).
    pub async fn exec(&self, instance: &str, cmd: &[&str]) -> Result<String> {
        let body = json!({ "command": cmd, "wait-for-websocket": false, "record-output": true, "interactive": false });
        let meta: Value = self.call("POST", &format!("/1.0/instances/{instance}/exec"), Some(body)).await?;
        let code = meta.pointer("/metadata/return").and_then(|v| v.as_i64()).unwrap_or(0);
        let out = match meta.pointer("/metadata/output/1").and_then(|v| v.as_str()) {
            Some(p) => {
                let uri: hyper::Uri = Uri::new(&self.socket, p).into();
                let req = hyper::Request::builder().method("GET").uri(uri).body(Full::new(Bytes::new()))?;
                let resp = self.client.request(req).await?;
                String::from_utf8_lossy(&resp.into_body().collect().await?.to_bytes()).to_string()
            }
            None => String::new(),
        };
        if code != 0 {
            return Err(anyhow!("incus exec {instance} {:?}: код {code}: {out}", cmd));
        }
        Ok(out)
    }

    /// Инстансы с состоянием (recursion=2) — для health workspace'ов.
    pub async fn instances(&self) -> Result<Vec<InstanceInfo>> {
        let v: Vec<Value> = self.call("GET", "/1.0/instances?recursion=2", None).await?;
        Ok(v.into_iter()
            .map(|i| {
                let name = i.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let status = i.get("status").and_then(|s| s.as_str()).unwrap_or("").to_string();
                let ip = i
                    .pointer("/state/network/eth0/addresses")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.iter().find(|x| x.get("family").and_then(|f| f.as_str()) == Some("inet")))
                    .and_then(|x| x.get("address"))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
                let mem = i.pointer("/state/memory/usage").and_then(|m| m.as_u64()).unwrap_or(0);
                let project = i.pointer("/config/user.cloudos.project").and_then(|p| p.as_str()).map(|s| s.to_string());
                InstanceInfo { name, status, ip, memory_bytes: mem, project }
            })
            .collect())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InstanceFull {
    pub name: String,
    pub status: String,
    pub memory_bytes: u64,
    pub cpu_usage_ns: u64,
    pub disk_bytes: u64,
    pub processes: i64,
}

impl Incus {
    /// Инстансы с метриками для монитора: память, cpu usage (ns), диск root, процессы.
    pub async fn instances_full(&self) -> Result<Vec<InstanceFull>> {
        let v: Vec<Value> = self.call("GET", "/1.0/instances?recursion=2", None).await?;
        Ok(v.into_iter()
            .map(|i| InstanceFull {
                name: i.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                status: i.get("status").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                memory_bytes: i.pointer("/state/memory/usage").and_then(|m| m.as_u64()).unwrap_or(0),
                cpu_usage_ns: i.pointer("/state/cpu/usage").and_then(|m| m.as_u64()).unwrap_or(0),
                disk_bytes: i.pointer("/state/disk/root/usage").and_then(|m| m.as_u64()).unwrap_or(0),
                processes: i.pointer("/state/processes").and_then(|m| m.as_i64()).unwrap_or(0),
            })
            .collect())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InstanceInfo {
    pub name: String,
    pub status: String,
    pub ip: Option<String>,
    pub memory_bytes: u64,
    pub project: Option<String>,
}

fn urlenc(s: &str) -> String {
    s.bytes().map(|b| match b { b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => (b as char).to_string(), _ => format!("%{b:02X}") }).collect()
}
