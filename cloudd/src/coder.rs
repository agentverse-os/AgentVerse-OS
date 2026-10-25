//! Coder REST API client (v2): a workspace per `project.yaml` with explicit rich parameters
//! (ADR-005), builds start/stop/delete, agent and app state (code-server).

use anyhow::{anyhow, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Clone)]
pub struct Coder {
    base: String,
    token: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub owner_name: String,
    pub latest_build: Build,
    /// The template has a new active version since the last build (Coder `outdated`).
    #[serde(default)]
    pub outdated: bool,
    /// Current version of its own template — we update to it at start, with no risk of slipping in another
    /// template's version.
    #[serde(default)]
    pub template_active_version_id: Option<Uuid>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Build {
    pub id: Uuid,
    pub transition: String,
    /// Workspace status after this build (running | stopped | deleted | starting | …).
    pub status: String,
    /// Status of the provisioner job itself (pending | running | succeeded | failed |
    /// canceled).
    #[serde(default)]
    pub job: Job,
    #[serde(default)]
    pub resources: Vec<Resource>,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Job {
    #[serde(default)]
    pub status: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Resource {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub agents: Vec<Agent>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub lifecycle_state: String,
    #[serde(default)]
    pub apps: Vec<App>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct App {
    pub slug: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub health: String,
    #[serde(default)]
    pub subdomain: bool,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Template {
    pub id: Uuid,
    pub name: String,
    pub active_version_id: Uuid,
    pub organization_id: Uuid,
}
#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RichParam<'a> {
    pub name: &'a str,
    pub value: String,
}

impl Coder {
    pub fn new(base: &str, token: &str) -> Self {
        Self { base: base.trim_end_matches('/').to_string(), token: token.to_string(), http: reqwest::Client::new() }
    }

    async fn req<T: DeserializeOwned>(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<T> {
        let mut r = self.http.request(method.clone(), format!("{}{}", self.base, path)).header("Coder-Session-Token", &self.token);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.with_context(|| format!("coder {method} {path}"))?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(anyhow!("coder {method} {path}: HTTP {status}: {}", text.chars().take(400).collect::<String>()));
        }
        if text.is_empty() {
            return serde_json::from_str("null").map_err(Into::into);
        }
        serde_json::from_str(&text).with_context(|| format!("coder {path}: разбор ответа: {}", text.chars().take(300).collect::<String>()))
    }

    pub async fn me(&self) -> Result<User> {
        self.req(reqwest::Method::GET, "/api/v2/users/me", None).await
    }
    pub async fn health(&self) -> Result<String> {
        let r = self.http.get(format!("{}/healthz", self.base)).send().await?;
        Ok(r.text().await?)
    }

    pub async fn template(&self, org: &str, name: &str) -> Result<Template> {
        self.req(reqwest::Method::GET, &format!("/api/v2/organizations/{org}/templates/{name}"), None).await
    }

    pub async fn workspace_by_name(&self, owner: &str, name: &str) -> Result<Option<Workspace>> {
        match self.req::<Workspace>(reqwest::Method::GET, &format!("/api/v2/users/{owner}/workspace/{name}"), None).await {
            Ok(w) => Ok(Some(w)),
            Err(e) if e.to_string().contains("HTTP 404") => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub async fn workspace(&self, id: Uuid) -> Result<Option<Workspace>> {
        match self.req::<Workspace>(reqwest::Method::GET, &format!("/api/v2/workspaces/{id}"), None).await {
            Ok(w) => Ok(Some(w)),
            Err(e) if e.to_string().contains("HTTP 404") || e.to_string().contains("HTTP 410") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `POST /api/v2/users/{user}/workspaces` — all rich parameters
    /// explicitly.
    pub async fn create_workspace(&self, owner: &str, template: &Template, name: &str, params: &[RichParam<'_>]) -> Result<Workspace> {
        let body = json!({
            "name": name,
            "template_version_id": template.active_version_id,
            "rich_parameter_values": params,
            "automatic_updates": "always",
        });
        self.req(reqwest::Method::POST, &format!("/api/v2/users/{owner}/workspaces"), Some(body)).await
    }

    /// `POST /api/v2/workspaces/{id}/builds` with transition
    /// start|stop|delete.
    pub async fn build(&self, id: Uuid, transition: &str) -> Result<Build> {
        self.req(reqwest::Method::POST, &format!("/api/v2/workspaces/{id}/builds"), Some(json!({ "transition": transition }))).await
    }
    /// A `start` build with new rich parameters (cpu, memory): the Incus provider applies the limits in
    /// place.
    pub async fn build_with_params(&self, id: Uuid, transition: &str, params: &[RichParam<'_>]) -> Result<Build> {
        self.req(reqwest::Method::POST, &format!("/api/v2/workspaces/{id}/builds"), Some(json!({ "transition": transition, "rich_parameter_values": params }))).await
    }
    /// A build on the given template version (updating the workspace to the current version, like `coder update`); rich
    /// parameters — explicitly, as at creation: Coder does not carry them over itself on a version change.
    pub async fn build_on_version(&self, id: Uuid, transition: &str, version: Uuid, params: &[RichParam<'_>]) -> Result<Build> {
        let body = json!({ "transition": transition, "template_version_id": version, "rich_parameter_values": params });
        self.req(reqwest::Method::POST, &format!("/api/v2/workspaces/{id}/builds"), Some(body)).await
    }

    pub async fn build_status(&self, build_id: Uuid) -> Result<Build> {
        self.req(reqwest::Method::GET, &format!("/api/v2/workspacebuilds/{build_id}"), None).await
    }

    /// Wait for the build to finish (succeeded|failed|canceled), up to
    /// `timeout_s`.
    pub async fn wait_build(&self, build_id: Uuid, timeout_s: u64) -> Result<Build> {
        let start = std::time::Instant::now();
        loop {
            let b = self.build_status(build_id).await?;
            match b.job.status.as_str() {
                "succeeded" => return Ok(b),
                "failed" | "canceled" => return Err(anyhow!("build {build_id}: job {}", b.job.status)),
                _ => {}
            }
            if start.elapsed().as_secs() > timeout_s {
                return Err(anyhow!("build {build_id}: таймаут (job {}, workspace {})", b.job.status, b.status));
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}

impl Workspace {
    /// Status for Desktop. Coder's `latest_build.status` is already the workspace status: pending | starting |
    /// running | stopping | stopped | failed | canceling | canceled | deleting | deleted.
    pub fn simple_status(&self) -> String {
        match self.latest_build.status.as_str() {
            "running" | "stopped" | "starting" | "stopping" | "pending" | "deleting" | "deleted" | "failed" => self.latest_build.status.clone(),
            "canceling" | "canceled" => "failed".to_string(),
            other => other.to_string(),
        }
    }
    pub fn agent(&self) -> Option<&Agent> {
        self.latest_build.resources.iter().flat_map(|r| r.agents.iter()).next()
    }
    /// Incus instance name by the template formula (`lower("ws-${owner}-${workspace}")`): the Coder resources
    /// hold the Terraform resource name ("workspace"), not the instance name.
    pub fn instance_name(&self) -> Option<String> {
        self.latest_build.resources.iter().any(|r| r.kind == "incus_instance").then(|| format!("ws-{}-{}", self.owner_name, self.name).to_lowercase())
    }
}
