//! `cloudd` configuration: everything from environment variables or the file `/etc/cloudos/cloudd.env`.
//! Secrets (the Coder token, the Komodo key) — only via env/a file with mode 0600; they never get into
//! the repository.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    /// Manifest repository: `projects/<name>/project.yaml`, `store/<app>/manifest.yaml` (+ compose next to
    /// it).
    pub repo_dir: PathBuf,
    /// Core state (tank/core): SQLite, the age key, rendered gate Caddyfiles.
    pub state_dir: PathBuf,
    /// API/Desktop address (the edge Caddy proxies `/` and `/api/`
    /// here).
    pub listen: Vec<String>,
    /// Entry Point host name (`box.netbird.cloud` / `<box>.<tailnet>.ts.net`).
    pub edge_host: String,
    /// Admin API edge-Caddy (`http://127.0.0.1:2019`).
    pub caddy_admin: String,
    /// Coder port behind the edge (`route: port`) and the internal Coder
    /// URL for the API.
    pub coder_url: String,
    pub coder_port: u16,
    /// Port of "Agents" as a separate app behind the edge (`route: port`, docs/agents-ux.md §9.8): its own origin for the PWA
    /// install.
    pub agents_port: u16,
    /// Coder Postgres container for the backup dump.
    pub coder_pg_container: String,
    pub coder_token: String,
    pub coder_template: String,
    pub coder_org: String,
    /// Komodo (App Runtime v1).
    pub komodo_url: String,
    pub komodo_key: String,
    pub komodo_secret: String,
    pub komodo_server: String,
    /// Runtime sockets.
    pub incus_socket: PathBuf,
    pub docker_socket: PathBuf,
    /// Base of the project subnets: `10.77.<n>.0/24`, where n is
    /// assigned by cloudd.
    pub project_subnet_base: String,
    /// App data (tank/apps/<app>) and the Periphery root (where Komodo writes stacks).
    pub apps_data_dir: PathBuf,
    /// Incus pool and the default workspace
    /// image.
    pub incus_pool: String,
    pub workspace_image: String,
    /// First ports for `route: port` apps.
    pub app_port_range: (u16, u16),
    /// The edge Caddy container (cloudd joins it to app networks for path/port routes). Additional Entry Point names/IPs (e.g. the
    /// server's NetBird IP for devices without MagicDNS); the edge_host IPv4 is added automatically.
    pub edge_alt_hosts: Vec<String>,
    pub edge_container: String,
    /// Upstreams from inside the edge container: cloudd on the host and Coder
    /// in coder-net.
    pub edge_cloudd_upstream: String,
    /// The "Agents" service (cloudos-agents) on the host — the /agents/*
    /// route on the edge.
    pub edge_agents_upstream: String,
    pub edge_coder_upstream: String,
    /// Caddy's internal CA (NetBird) instead of a certificate from
    /// tailscaled/ACME.
    pub tls_internal: bool,
    /// Coder (and Komodo) administrator from bootstrap: shown in Desktop as the Coder sign-in data.
    pub admin_email: String,
    pub admin_password: Option<String>,
}

/// Core variables file (`CLOUDD_ENV_FILE`, default /etc/cloudos/cloudd.env) — edited by the first-run wizard.
pub fn env_file_path() -> PathBuf {
    env("CLOUDD_ENV_FILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/etc/cloudos/cloudd.env"))
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}
fn env_or(name: &str, default: &str) -> String {
    env(name).unwrap_or_else(|| default.to_string())
}

impl Config {
    /// Reads `/etc/cloudos/cloudd.env` (if present, KEY=VALUE format) into the environment, then builds
    /// the Config.
    pub fn load() -> Result<Self> {
        let env_file = env_file_path();
        if env_file.exists() {
            load_env_file(&env_file)?;
        }
        let coder_url = env_or("CLOUDD_CODER_URL", "http://127.0.0.1:7080");
        Ok(Self {
            repo_dir: PathBuf::from(env_or("CLOUDD_REPO_DIR", "/var/lib/cloudos/repo")),
            state_dir: PathBuf::from(env_or("CLOUDD_STATE_DIR", "/var/lib/cloudos")),
            listen: env_or("CLOUDD_LISTEN", "127.0.0.1:7100,172.17.0.1:7100").split(',').map(|s| s.trim().to_string()).collect(),
            edge_host: env("CLOUDD_EDGE_HOST").context("CLOUDD_EDGE_HOST не задан (имя хоста Entry Point)")?,
            caddy_admin: env_or("CLOUDD_CADDY_ADMIN", "http://127.0.0.1:2019"),
            coder_url,
            coder_port: env_or("CLOUDD_CODER_PORT", "8444").parse().context("CLOUDD_CODER_PORT")?,
            agents_port: env_or("CLOUDD_AGENTS_PORT", "8445").parse().context("CLOUDD_AGENTS_PORT")?,
            coder_pg_container: env_or("CLOUDD_CODER_PG_CONTAINER", "01-coder-incus-postgres-1"),
            coder_token: env_or("CLOUDD_CODER_TOKEN", ""),
            coder_template: env_or("CLOUDD_CODER_TEMPLATE", "cloudos-incus"),
            coder_org: env_or("CLOUDD_CODER_ORG", "coder"),
            komodo_url: env_or("CLOUDD_KOMODO_URL", "http://127.0.0.1:9120"),
            komodo_key: env_or("CLOUDD_KOMODO_KEY", ""),
            komodo_secret: env_or("CLOUDD_KOMODO_SECRET", ""),
            komodo_server: env_or("CLOUDD_KOMODO_SERVER", "Local"),
            incus_socket: PathBuf::from(env_or("CLOUDD_INCUS_SOCKET", "/var/lib/incus/unix.socket")),
            docker_socket: PathBuf::from(env_or("CLOUDD_DOCKER_SOCKET", "/var/run/docker.sock")),
            project_subnet_base: env_or("CLOUDD_PROJECT_SUBNET_BASE", "10.77"),
            apps_data_dir: PathBuf::from(env_or("CLOUDD_APPS_DATA_DIR", "/srv/apps")),
            incus_pool: env_or("CLOUDD_INCUS_POOL", "default"),
            workspace_image: env_or("CLOUDD_WORKSPACE_IMAGE", "images:ubuntu/24.04/cloud"),
            app_port_range: (
                env_or("CLOUDD_APP_PORT_MIN", "8450").parse().context("CLOUDD_APP_PORT_MIN")?,
                env_or("CLOUDD_APP_PORT_MAX", "8499").parse().context("CLOUDD_APP_PORT_MAX")?,
            ),
            edge_alt_hosts: env_or("CLOUDD_EDGE_ALT_HOSTS", "").split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            edge_container: env_or("CLOUDD_EDGE_CONTAINER", "cloudos-edge"),
            edge_cloudd_upstream: env_or("CLOUDD_EDGE_CLOUDD_UPSTREAM", "host.docker.internal:7100"),
            edge_agents_upstream: env_or("CLOUDD_EDGE_AGENTS_UPSTREAM", "host.docker.internal:7300"),
            edge_coder_upstream: env_or("CLOUDD_EDGE_CODER_UPSTREAM", "coder:7080"),
            tls_internal: env_or("CLOUDD_TLS_INTERNAL", "true") == "true",
            admin_email: env_or("CLOUDOS_ADMIN_EMAIL", "admin@cloudos.local"),
            admin_password: std::env::var("CLOUDOS_ADMIN_PASSWORD").ok().filter(|s| !s.is_empty()),
        })
    }

    pub fn db_path(&self) -> PathBuf {
        self.state_dir.join("cloudd.db")
    }
    pub fn age_key_path(&self) -> PathBuf {
        self.state_dir.join("age.key")
    }
    pub fn gates_dir(&self) -> PathBuf {
        self.state_dir.join("gates")
    }
    pub fn projects_dir(&self) -> PathBuf {
        self.repo_dir.join("projects")
    }
    pub fn store_dir(&self) -> PathBuf {
        self.repo_dir.join("store")
    }
    /// Public Coder URL behind the edge.
    pub fn coder_public_url(&self) -> String {
        format!("https://{}:{}", self.edge_host, self.coder_port)
    }
    /// Address of "Agents" as a separate app (its own origin, §9.8).
    pub fn agents_public_url(&self) -> String {
        format!("https://{}:{}/agents/", self.edge_host, self.agents_port)
    }
}

fn load_env_file(path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("чтение {}", path.display()))?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            if std::env::var_os(k.trim()).is_none() {
                std::env::set_var(k.trim(), v);
            }
        }
    }
    Ok(())
}
