//! Конфигурация `cloudd`: всё из переменных окружения или файла `/etc/cloudos/cloudd.env`.
//! Секреты (токен Coder, ключ Komodo) — только через env/файл с правами 0600; в репозиторий не попадают.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    /// Репозиторий манифестов: `projects/<name>/project.yaml`, `store/<app>/manifest.yaml` (+ compose рядом).
    pub repo_dir: PathBuf,
    /// Состояние ядра (tank/core): SQLite, age-ключ, отрендеренные Caddyfile gate'ов.
    pub state_dir: PathBuf,
    /// Адрес API/Desktop (edge Caddy проксирует сюда `/` и `/api/`).
    pub listen: Vec<String>,
    /// Имя хоста Entry Point (`aios.netbird.cloud` / `<box>.<tailnet>.ts.net`).
    pub edge_host: String,
    /// Admin API edge-Caddy (`http://127.0.0.1:2019`).
    pub caddy_admin: String,
    /// Порт Coder за edge (`route: port`) и внутренний URL Coder для API.
    pub coder_url: String,
    pub coder_port: u16,
    /// Контейнер Postgres Coder для дампа при бэкапе.
    pub coder_pg_container: String,
    pub coder_token: String,
    pub coder_template: String,
    pub coder_org: String,
    /// Komodo (App Runtime v1).
    pub komodo_url: String,
    pub komodo_key: String,
    pub komodo_secret: String,
    pub komodo_server: String,
    /// Сокеты рантаймов.
    pub incus_socket: PathBuf,
    pub docker_socket: PathBuf,
    /// База подсетей проектов: `10.77.<n>.0/24`, где n выдаёт cloudd.
    pub project_subnet_base: String,
    /// Данные приложений (tank/apps/<app>) и корень Periphery (куда Komodo пишет стеки).
    pub apps_data_dir: PathBuf,
    /// Пул Incus и образ workspace по умолчанию.
    pub incus_pool: String,
    pub workspace_image: String,
    /// Первые порты для `route: port` приложений.
    pub app_port_range: (u16, u16),
    /// Контейнер edge-Caddy (его cloudd подключает к сетям приложений для path/port-маршрутов).
    /// Дополнительные имена/IP Entry Point (например, NetBird-IP сервера для устройств без MagicDNS); IPv4 edge_host добавляется сам.
    pub edge_alt_hosts: Vec<String>,
    pub edge_container: String,
    /// Upstream'ы изнутри контейнера edge: cloudd на хосте и Coder в coder-net.
    pub edge_cloudd_upstream: String,
    pub edge_coder_upstream: String,
    /// Внутренний CA Caddy (NetBird) вместо сертификата от tailscaled/ACME.
    pub tls_internal: bool,
    /// Администратор Coder (и Komodo) из bootstrap: показывается в Desktop как данные для входа в Coder.
    pub admin_email: String,
    pub admin_password: Option<String>,
}

/// Файл переменных ядра (`CLOUDD_ENV_FILE`, по умолчанию /etc/cloudos/cloudd.env) — его правит мастер первого запуска.
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
    /// Читает `/etc/cloudos/cloudd.env` (если есть, формат KEY=VALUE) в окружение, затем собирает Config.
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
    /// Публичный URL Coder за edge.
    pub fn coder_public_url(&self) -> String {
        format!("https://{}:{}", self.edge_host, self.coder_port)
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
