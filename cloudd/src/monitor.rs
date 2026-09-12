//! Монитор (2.9: только health и потребление, без time series на диске): сборщик метрик хоста, контейнеров Docker и
//! инстансов Incus раз в 5 с с кольцевой историей на 5 минут — для виджета «Монитор системы» и вкладки Система.

use crate::docker::DockerRt;
use crate::incus::Incus;
use bollard::query_parameters::StatsOptions;
use futures_util::StreamExt;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use sysinfo::{Disks, System};
use utoipa::ToSchema;

pub const HISTORY: usize = 60;

#[derive(Debug, Clone, Serialize, ToSchema, Default)]
pub struct HostSample {
    pub at: String,
    pub cpu_pct: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub load1: f32,
    pub load5: f32,
    pub load15: f32,
    pub uptime_s: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DiskInfo {
    pub mount: String,
    pub total: u64,
    pub avail: u64,
    pub fs: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ContainerStat {
    pub name: String,
    pub app: Option<String>,
    pub cpu_pct: f32,
    pub mem_used: u64,
    pub mem_limit: u64,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InstanceStat {
    pub name: String,
    pub status: String,
    pub cpu_pct: f32,
    pub mem_used: u64,
    pub disk_used: u64,
    pub processes: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema, Default)]
pub struct Snapshot {
    pub host: HostSample,
    pub disks: Vec<DiskInfo>,
    pub containers: Vec<ContainerStat>,
    pub instances: Vec<InstanceStat>,
    /// история CPU% и памяти хоста, старые → новые
    pub history_cpu: Vec<f32>,
    pub history_mem: Vec<u64>,
    pub cores: usize,
}

#[derive(Clone)]
pub struct Monitor {
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    sys: System,
    snap: Snapshot,
    hist_cpu: VecDeque<f32>,
    hist_mem: VecDeque<u64>,
    /// для CPU% инстансов Incus: предыдущее cpu usage (ns) и время
    incus_prev: HashMap<String, (u64, std::time::Instant)>,
}

impl Monitor {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        Self { inner: Arc::new(RwLock::new(Inner { sys, snap: Snapshot::default(), hist_cpu: VecDeque::new(), hist_mem: VecDeque::new(), incus_prev: HashMap::new() })) }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.read().map(|i| i.snap.clone()).unwrap_or_default()
    }

    /// Фоновый сборщик: раз в 5 с.
    pub fn spawn(self, docker: DockerRt, incus: Incus, apps: impl Fn() -> Vec<String> + Send + Sync + 'static) {
        tokio::spawn(async move {
            loop {
                self.tick(&docker, &incus, &apps).await;
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }

    async fn tick(&self, docker: &DockerRt, incus: &Incus, apps: &(impl Fn() -> Vec<String> + Send + Sync)) {
        // ---- хост ----
        let (host, cores) = {
            let mut g = match self.inner.write() { Ok(g) => g, Err(_) => return };
            g.sys.refresh_cpu_usage();
            g.sys.refresh_memory();
            let la = System::load_average();
            let host = HostSample {
                at: crate::store::now(),
                cpu_pct: g.sys.global_cpu_usage(),
                mem_used: g.sys.used_memory(),
                mem_total: g.sys.total_memory(),
                load1: la.one as f32,
                load5: la.five as f32,
                load15: la.fifteen as f32,
                uptime_s: System::uptime(),
            };
            let cores = g.sys.cpus().len();
            (host, cores)
        };
        let disks: Vec<DiskInfo> = Disks::new_with_refreshed_list()
            .iter()
            .filter(|d| { let m = d.mount_point().to_string_lossy(); m == "/" || m == "/var/lib/docker" || m == "/var/lib/cloudos" || (m.starts_with("/srv/") && m.matches('/').count() == 2) })
            .map(|d| DiskInfo { mount: d.mount_point().to_string_lossy().to_string(), total: d.total_space(), avail: d.available_space(), fs: d.file_system().to_string_lossy().to_string() })
            .collect();

        // ---- контейнеры (одноразовая статистика, параллельно) ----
        let app_names = apps();
        let mut containers = Vec::new();
        if let Ok(list) = docker.docker.list_containers(Some(bollard::query_parameters::ListContainersOptions { all: false, ..Default::default() })).await {
            let futs = list.into_iter().map(|c| {
                let d = docker.docker.clone();
                let app_names = app_names.clone();
                async move {
                    let id = c.id.clone().unwrap_or_default();
                    let name = c.names.unwrap_or_default().first().map(|n| n.trim_start_matches('/').to_string()).unwrap_or_default();
                    let mut s = d.stats(&id, Some(StatsOptions { stream: false, one_shot: false }));
                    let st = s.next().await.and_then(|r| r.ok());
                    let (cpu, mem, lim) = st.map(|s| {
                        let cs = s.cpu_stats.unwrap_or_default();
                        let ps = s.precpu_stats.unwrap_or_default();
                        let total = cs.cpu_usage.as_ref().and_then(|u| u.total_usage).unwrap_or(0);
                        let ptotal = ps.cpu_usage.as_ref().and_then(|u| u.total_usage).unwrap_or(0);
                        let cpu_delta = total.saturating_sub(ptotal) as f64;
                        let sys_delta = cs.system_cpu_usage.unwrap_or(0).saturating_sub(ps.system_cpu_usage.unwrap_or(0)) as f64;
                        let ncpu = cs.online_cpus.unwrap_or(1) as f64;
                        let pct = if sys_delta > 0.0 { (cpu_delta / sys_delta * ncpu * 100.0) as f32 } else { 0.0 };
                        let ms = s.memory_stats.unwrap_or_default();
                        let cache = ms.stats.as_ref().map(|m| m.get("inactive_file").or_else(|| m.get("cache")).copied().unwrap_or(0)).unwrap_or(0);
                        (pct, ms.usage.unwrap_or(0).saturating_sub(cache), ms.limit.unwrap_or(0))
                    }).unwrap_or((0.0, 0, 0));
                    let app = app_names.iter().find(|a| name.starts_with(&format!("{a}-"))).cloned();
                    ContainerStat { name, app, cpu_pct: cpu, mem_used: mem, mem_limit: lim, state: c.state.map(|s| s.to_string()).unwrap_or_default() }
                }
            });
            containers = futures_util::future::join_all(futs).await;
            containers.sort_by(|a, b| b.mem_used.cmp(&a.mem_used));
        }

        // ---- инстансы Incus ----
        let mut instances = Vec::new();
        if let Ok(list) = incus.instances_full().await {
            let now = std::time::Instant::now();
            let mut g = match self.inner.write() { Ok(g) => g, Err(_) => return };
            for i in list {
                let cpu_ns = i.cpu_usage_ns;
                let pct = match g.incus_prev.get(&i.name) {
                    Some((prev, t)) if cpu_ns >= *prev => { let dt = now.duration_since(*t).as_nanos() as f64; if dt > 0.0 { ((cpu_ns - prev) as f64 / dt * 100.0) as f32 } else { 0.0 } }
                    _ => 0.0,
                };
                g.incus_prev.insert(i.name.clone(), (cpu_ns, now));
                instances.push(InstanceStat { name: i.name, status: i.status, cpu_pct: pct, mem_used: i.memory_bytes, disk_used: i.disk_bytes, processes: i.processes });
            }
        }

        if let Ok(mut g) = self.inner.write() {
            g.hist_cpu.push_back(host.cpu_pct);
            g.hist_mem.push_back(host.mem_used);
            while g.hist_cpu.len() > HISTORY { g.hist_cpu.pop_front(); }
            while g.hist_mem.len() > HISTORY { g.hist_mem.pop_front(); }
            g.snap = Snapshot { host, disks, containers, instances, history_cpu: g.hist_cpu.iter().copied().collect(), history_mem: g.hist_mem.iter().copied().collect(), cores };
        }
    }
}
