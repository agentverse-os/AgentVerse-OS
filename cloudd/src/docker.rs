//! Docker через bollard: macvlan-сеть проекта на мосту Incus, gate-контейнер (Caddy), сети приложений,
//! connect/disconnect gate (grant/revoke), reload Caddy внутри gate. Реализация gate — по ADR-005 (docker-gate).

use anyhow::{anyhow, Context, Result};
use bollard::container::{Config as ContainerConfig, CreateContainerOptions, ListContainersOptions, RemoveContainerOptions, StartContainerOptions};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::image::CreateImageOptions;
use bollard::network::{ConnectNetworkOptions, CreateNetworkOptions, DisconnectNetworkOptions, InspectNetworkOptions};
use bollard::secret::{EndpointIpamConfig, EndpointSettings, HostConfig, Ipam, IpamConfig, RestartPolicy, RestartPolicyNameEnum};
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::path::Path;

pub const GATE_IMAGE: &str = "caddy:2";

#[derive(Clone)]
pub struct DockerRt {
    pub docker: Docker,
}

impl DockerRt {
    pub fn new(socket: &Path) -> Result<Self> {
        let docker = Docker::connect_with_unix(socket.to_str().unwrap_or("/var/run/docker.sock"), 60, bollard::API_DEFAULT_VERSION)?;
        Ok(Self { docker })
    }

    pub async fn ping(&self) -> Result<String> {
        let v = self.docker.version().await?;
        Ok(v.version.unwrap_or_default())
    }

    pub async fn network_exists(&self, name: &str) -> Result<bool> {
        match self.docker.inspect_network(name, None::<InspectNetworkOptions<String>>).await {
            Ok(_) => Ok(true),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Сеть приложения `<app>-net` — создаёт ядро, compose помечает `external: true` (ADR-005, решение 3).
    pub async fn ensure_bridge_network(&self, name: &str, labels: HashMap<String, String>) -> Result<()> {
        if self.network_exists(name).await? {
            return Ok(());
        }
        self.docker
            .create_network(CreateNetworkOptions { name: name.to_string(), driver: "bridge".to_string(), labels, ..Default::default() })
            .await?;
        Ok(())
    }

    /// macvlan с parent = мост Incus проекта: gate получает интерфейс прямо в `net-<project>`.
    pub async fn ensure_macvlan(&self, name: &str, parent: &str, subnet: &str, gateway: &str, ip_range: &str) -> Result<()> {
        if self.network_exists(name).await? {
            return Ok(());
        }
        let mut opts = HashMap::new();
        opts.insert("parent".to_string(), parent.to_string());
        self.docker
            .create_network(CreateNetworkOptions {
                name: name.to_string(),
                driver: "macvlan".to_string(),
                options: opts,
                ipam: Ipam {
                    config: Some(vec![IpamConfig {
                        subnet: Some(subnet.to_string()),
                        gateway: Some(gateway.to_string()),
                        ip_range: Some(ip_range.to_string()),
                        ..Default::default()
                    }]),
                    ..Default::default()
                },
                labels: HashMap::from([("cloudos.kind".to_string(), "project-macvlan".to_string())]),
                ..Default::default()
            })
            .await?;
        Ok(())
    }

    pub async fn remove_network(&self, name: &str) -> Result<()> {
        if self.network_exists(name).await? {
            self.docker.remove_network(name).await?;
        }
        Ok(())
    }

    pub async fn container_exists(&self, name: &str) -> Result<bool> {
        match self.docker.inspect_container(name, None::<bollard::query_parameters::InspectContainerOptions>).await {
            Ok(_) => Ok(true),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn container_state(&self, name: &str) -> Result<Option<String>> {
        match self.docker.inspect_container(name, None::<bollard::query_parameters::InspectContainerOptions>).await {
            Ok(c) => Ok(c.state.and_then(|s| s.status).map(|s| s.to_string())),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn ensure_image(&self, image: &str) -> Result<()> {
        if self.docker.inspect_image(image).await.is_ok() {
            return Ok(());
        }
        let mut s = self.docker.create_image(Some(CreateImageOptions { from_image: image, ..Default::default() }), None, None);
        while let Some(r) = s.next().await {
            r.with_context(|| format!("pull {image}"))?;
        }
        Ok(())
    }

    /// `USER` из конфигурации образа: число, `uid:gid` или имя. None — root или не задан.
    pub async fn image_user(&self, image: &str) -> Result<Option<String>> {
        let i = self.docker.inspect_image(image).await?;
        Ok(i.config.and_then(|c| c.user).map(|u| u.trim().to_string()).filter(|u| !u.is_empty() && u != "0" && u != "root" && u != "0:0" && u != "root:root"))
    }

    /// Путь внутри образа как tar-архив без запуска контейнера (create → download → remove): владелец каталога, /etc/passwd.
    /// None — такого пути в образе нет. Читаем не больше 4 МБ: нужны заголовки, а не содержимое.
    pub async fn image_tar(&self, image: &str, path: &str) -> Result<Option<Vec<u8>>> {
        let name = format!("cloudos-peek-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let cfg = ContainerConfig {
            image: Some(image.to_string()),
            entrypoint: Some(vec!["/bin/sh".to_string()]),
            cmd: Some(vec!["-c".to_string(), "true".to_string()]),
            labels: Some(HashMap::from([("cloudos.kind".to_string(), "peek".to_string())])),
            ..Default::default()
        };
        self.docker.create_container(Some(CreateContainerOptions { name: name.as_str(), platform: None }), cfg).await.with_context(|| format!("create {image} для просмотра {path}"))?;
        let mut s = self.docker.download_from_container(&name, Some(bollard::container::DownloadFromContainerOptions { path }));
        let mut buf = Vec::new();
        let mut err = None;
        while let Some(chunk) = s.next().await {
            match chunk {
                Ok(b) => { buf.extend_from_slice(&b); if buf.len() > 4 << 20 { break; } }
                Err(e) => { err = Some(e); break; }
            }
        }
        drop(s);
        let _ = self.docker.remove_container(&name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
        match err {
            None => Ok(Some(buf)),
            Some(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(None),
            Some(e) => Err(e.into()),
        }
    }

    /// Gate проекта: Caddy на macvlan со статическим IP, Caddyfile смонтирован из state_dir, restart unless-stopped.
    pub async fn ensure_gate(&self, name: &str, macvlan: &str, ip: &str, caddyfile_dir: &Path) -> Result<()> {
        if self.container_exists(name).await? {
            self.docker.start_container(name, None::<StartContainerOptions<String>>).await.ok();
            return Ok(());
        }
        self.ensure_image(GATE_IMAGE).await?;
        let mut endpoints = HashMap::new();
        endpoints.insert(
            macvlan.to_string(),
            EndpointSettings { ipam_config: Some(EndpointIpamConfig { ipv4_address: Some(ip.to_string()), ..Default::default() }), ..Default::default() },
        );
        let cfg = ContainerConfig {
            image: Some(GATE_IMAGE.to_string()),
            labels: Some(HashMap::from([("cloudos.kind".to_string(), "gate".to_string())])),
            host_config: Some(HostConfig {
                binds: Some(vec![format!("{}:/etc/caddy:ro", caddyfile_dir.display())]),
                restart_policy: Some(RestartPolicy { name: Some(RestartPolicyNameEnum::UNLESS_STOPPED), ..Default::default() }),
                network_mode: Some(macvlan.to_string()),
                memory: Some(128 * 1024 * 1024),
                ..Default::default()
            }),
            networking_config: Some(bollard::container::NetworkingConfig { endpoints_config: endpoints }),
            ..Default::default()
        };
        self.docker.create_container(Some(CreateContainerOptions { name, platform: None }), cfg).await?;
        self.docker.start_container(name, None::<StartContainerOptions<String>>).await?;
        Ok(())
    }

    pub async fn remove_container(&self, name: &str) -> Result<()> {
        if self.container_exists(name).await? {
            self.docker.remove_container(name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await?;
        }
        Ok(())
    }

    /// grant: gate получает интерфейс в сети приложения — без рестарта (StartedAt не меняется, ADR-005).
    pub async fn connect(&self, network: &str, container: &str) -> Result<()> {
        match self.docker.connect_network(network, ConnectNetworkOptions { container, endpoint_config: EndpointSettings::default() }).await {
            Ok(()) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 403, message }) if message.contains("already exists") => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 409, .. }) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
    pub async fn disconnect(&self, network: &str, container: &str) -> Result<()> {
        match self.docker.disconnect_network(network, DisconnectNetworkOptions { container, force: true }).await {
            Ok(()) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError { message, .. }) if message.contains("is not connected") => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn exec(&self, container: &str, cmd: Vec<&str>) -> Result<String> {
        let exec = self
            .docker
            .create_exec(container, CreateExecOptions { cmd: Some(cmd), attach_stdout: Some(true), attach_stderr: Some(true), ..Default::default() })
            .await?;
        let mut out = String::new();
        if let StartExecResults::Attached { mut output, .. } = self.docker.start_exec(&exec.id, None).await? {
            while let Some(chunk) = output.next().await {
                out.push_str(&chunk?.to_string());
            }
        }
        let info = self.docker.inspect_exec(&exec.id).await?;
        if info.exit_code.unwrap_or(0) != 0 {
            return Err(anyhow!("exec в {container} завершился с {}: {out}", info.exit_code.unwrap_or(-1)));
        }
        Ok(out)
    }

    /// Перечитать Caddyfile gate без рестарта контейнера; если admin API внутри недоступен (старый процесс) — перезапуск.
    pub async fn reload_gate(&self, name: &str) -> Result<()> {
        match self.exec(name, vec!["caddy", "reload", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"]).await {
            Ok(_) => Ok(()),
            Err(e) => {
                tracing::warn!("gate {name}: reload не удался ({e}); перезапускаю контейнер");
                self.docker.restart_container(name, None::<bollard::query_parameters::RestartContainerOptions>).await?;
                Ok(())
            }
        }
    }

    /// Контейнеры стека приложения (по label compose) — для status.
    pub async fn stack_containers(&self, project: &str) -> Result<Vec<(String, String, String)>> {
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![format!("com.docker.compose.project={project}")]);
        let list = self.docker.list_containers(Some(ListContainersOptions { all: true, filters, ..Default::default() })).await?;
        Ok(list
            .into_iter()
            .map(|c| {
                (
                    c.names.unwrap_or_default().first().map(|n| n.trim_start_matches('/').to_string()).unwrap_or_default(),
                    c.image.unwrap_or_default(),
                    c.state.map(|s| s.to_string()).unwrap_or_default(),
                )
            })
            .collect())
    }

    /// IP контейнера в конкретной сети (для хуков с хоста: хост видит bridge-сети напрямую).
    pub async fn container_ip(&self, container: &str, network: &str) -> Result<String> {
        let c = self.docker.inspect_container(container, None::<bollard::query_parameters::InspectContainerOptions>).await?;
        c.network_settings
            .and_then(|n| n.networks)
            .and_then(|n| n.get(network).and_then(|e| e.ip_address.clone()))
            .filter(|ip| !ip.is_empty())
            .ok_or_else(|| anyhow!("у {container} нет адреса в сети {network}"))
    }

    /// Endpoint приложения доступен из gate? (для health в Desktop) — простая проверка через `wget` в gate не нужна; смотрим сеть.
    pub async fn gate_networks(&self, gate: &str) -> Result<Vec<String>> {
        let c = self.docker.inspect_container(gate, None::<bollard::query_parameters::InspectContainerOptions>).await?;
        Ok(c.network_settings.and_then(|n| n.networks).map(|n| n.keys().cloned().collect()).unwrap_or_default())
    }
}
