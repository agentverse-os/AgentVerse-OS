//! Orchestration (architecture section 4): `cloudd` applies `project.yaml` and the manifests
//! idempotently, deletion rolls the artifacts back in reverse order.

use crate::caddy::{EdgeCaddy, EdgeInput};
use crate::coder::{Coder, RichParam};
use crate::config::Config;
use crate::docker::DockerRt;
use crate::gate;
use crate::incus::Incus;
use crate::komodo::Komodo;
use crate::model::*;
use crate::store::{self, Store};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct Service {
    pub cfg: Arc<Config>,
    pub store: Store,
    pub incus: Incus,
    pub docker: DockerRt,
    pub coder: Coder,
    pub komodo: Komodo,
    pub edge: EdgeCaddy,
    pub monitor: crate::monitor::Monitor,
    pub feeds: crate::feeds::Feeds,
    /// One reconcile at a time: operations are long and change shared
    /// state.
    pub(crate) lock: Arc<tokio::sync::Mutex<()>>,
    /// One backup cycle at a time plus an "in
    /// progress" flag for the UI.
    pub(crate) backup_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) backup_running: Arc<std::sync::atomic::AtomicBool>,
    /// Parsed catalog (942 manifests ≈ 1 s of YAML parsing) with a fingerprint of file mtimes: re-read only after changes.
    catalog_cache: Arc<std::sync::Mutex<Option<(u128, Vec<AppManifest>)>>>,
    /// A catalog re-import from upstream is running
    /// (updates.rs).
    pub(crate) importing: Arc<std::sync::atomic::AtomicBool>,
    /// Agent Runtime: session cache per workspace and Desktop presence (agents.rs).
    pub(crate) agents: Arc<crate::agents::AgentsState>,
    /// Spending limits (budgets.rs): when we warned last
    pub(crate) budgets_st: Arc<crate::budgets::BudgetsState>,
}

impl Service {
    pub fn new(cfg: Config) -> Result<Self> {
        let store = Store::open(&cfg.db_path(), &cfg.age_key_path())?;
        match store.fix_cumulative_costs() { Ok(0) => {} Ok(n) => tracing::info!("книга расходов: {n} строк пересчитаны из накопительных итогов в стоимость хода"), Err(e) => tracing::warn!("книга расходов: пересчёт не удался: {e:#}") }
        let incus = Incus::new(cfg.incus_socket.clone());
        let docker = DockerRt::new(&cfg.docker_socket)?;
        let coder = Coder::new(&cfg.coder_url, &cfg.coder_token);
        let komodo = Komodo::new(&cfg.komodo_url, &cfg.komodo_key, &cfg.komodo_secret, &cfg.komodo_server);
        let edge = EdgeCaddy::new(&cfg.caddy_admin);
        let agents = Arc::new(crate::agents::AgentsState::load(&store)); let budgets_st = Arc::new(crate::budgets::BudgetsState::load(&store));
        Ok(Self { cfg: Arc::new(cfg), store, incus, docker, coder, komodo, edge, monitor: crate::monitor::Monitor::new(), feeds: crate::feeds::Feeds::default(), lock: Arc::new(tokio::sync::Mutex::new(())), backup_lock: Arc::new(tokio::sync::Mutex::new(())), backup_running: Arc::new(std::sync::atomic::AtomicBool::new(false)), catalog_cache: Default::default(), importing: Default::default(), agents, budgets_st })
    }

    // =====================================================================
    // Store: reading manifests from the repository
    // =====================================================================

    pub fn catalog(&self) -> Result<Vec<AppManifest>> {
        let dir = self.cfg.store_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        // fingerprint: mtime and size of every manifest.yaml / login.yaml and recommended.yaml (≈1000 stats ≈
        // milliseconds)
        let mut fp: u128 = 0;
        let mut paths = Vec::new();
        for e in std::fs::read_dir(&dir)? {
            let p = e?.path();
            for f in ["manifest.yaml", "login.yaml"] {
                let fpth = p.join(f);
                if let Ok(md) = std::fs::metadata(&fpth) {
                    let t = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
                    fp = fp.wrapping_mul(1_000_003).wrapping_add(t ^ (md.len() as u128));
                    if f == "manifest.yaml" { paths.push(fpth); }
                }
            }
        }
        if let Ok(md) = std::fs::metadata(dir.join("recommended.yaml")) {
            let t = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
            fp = fp.wrapping_mul(1_000_003).wrapping_add(t ^ (md.len() as u128));
        }
        if let Some((cfp, cached)) = self.catalog_cache.lock().unwrap().as_ref() {
            if *cfp == fp { return Ok(cached.clone()); }
        }
        let mut out = Vec::new();
        for p in paths {
            out.push(load_manifest(&p)?);
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        let rec = self.recommended();
        for m in &mut out {
            m.recommended = rec.get(&m.name).cloned();
        }
        *self.catalog_cache.lock().unwrap() = Some((fp, out.clone()));
        Ok(out)
    }
    pub fn manifest(&self, name: &str) -> Result<AppManifest> {
        validate_name(name)?;
        let p = self.cfg.store_dir().join(name).join("manifest.yaml");
        if !p.exists() {
            bail!("нет приложения {name} в Store ({})", p.display());
        }
        let mut m = load_manifest(&p)?;
        m.recommended = self.recommended().get(name).cloned();
        if m.name != name {
            bail!("манифест {}: name={} не совпадает с каталогом", p.display(), m.name);
        }
        Ok(m)
    }
    /// App compose with the user override (`store/<app>/compose.override.yaml`, docker-compose.override semantics: deep merge
    /// by services). The override lives in a separate file, so a catalog re-import leaves it alone.
    pub(crate) fn compose_text(&self, m: &AppManifest) -> Result<String> {
        let dir = self.cfg.store_dir().join(&m.name);
        let base = std::fs::read_to_string(dir.join(&m.compose)).with_context(|| format!("compose {}", dir.join(&m.compose).display()))?;
        let ov = dir.join("compose.override.yaml");
        let ov_text = if ov.exists() { std::fs::read_to_string(&ov)? } else { String::new() };
        let has_links = self.is_shared(&m.name)? || self.store.list_links()?.iter().any(|l| l.consumer == m.name);
        if ov_text.trim().is_empty() && !has_links {
            return Ok(base); // unchanged — the original catalog text with comments
        }
        let mut a: serde_json::Value = serde_yaml_ng::from_str(&base).context("compose из каталога")?;
        if !ov_text.trim().is_empty() {
            let b: serde_json::Value = serde_yaml_ng::from_str(&ov_text).context("compose.override.yaml")?;
            merge_json(&mut a, &b);
        }
        self.inject_links(m, &mut a)?;
        Ok(serde_yaml_ng::to_string(&a)?)
    }

    /// Links and the shared network in the consumer's compose: provider networks as external, address variables as `${VAR}` (values are
    /// in the stack env).
    fn inject_links(&self, m: &AppManifest, compose: &mut serde_json::Value) -> Result<()> {
        let providers: Vec<(String, String)> = self.store.list_links()?.into_iter().filter(|l| l.consumer == m.name).map(|l| (l.provider.clone(), format!("{}-net", l.provider))).collect();
        let shared = self.is_shared(&m.name)?;
        if providers.is_empty() && !shared {
            return Ok(());
        }
        let keys: Vec<String> = self.link_env(m)?.into_iter().map(|(k, _)| k).collect();
        inject_links(compose, &m.name, &m.endpoint.service, &providers, shared, &keys);
        Ok(())
    }

    fn is_shared(&self, name: &str) -> Result<bool> {
        Ok(self.store.get_kv(&format!("app.shared.{name}"))?.and_then(|v| v.as_bool()).unwrap_or(false))
    }

    /// Variables from links: `<PROVIDER>_URL` (the address inside Cloud OS) plus the provider hook output saved at link time.
    fn link_env(&self, m: &AppManifest) -> Result<Vec<(String, String)>> {
        let mut out: Vec<(String, String)> = Vec::new();
        for l in self.store.list_links()?.into_iter().filter(|l| l.consumer == m.name) {
            if let Some(p) = self.store.get_app(&l.provider)? {
                out.push((format!("{}_URL", env_prefix(&l.provider)), internal_url(&p)));
            }
            let scope = format!("link:{}:{}", l.consumer, l.provider);
            for k in self.store.list_secret_keys(&scope)? {
                if let Some(v) = self.store.get_secret(&scope, &k)? {
                    if !out.iter().any(|(ek, _)| *ek == k) { out.push((k, v)); }
                }
            }
        }
        Ok(out)
    }

    /// User env overrides (`store/<app>/user.env`, KEY=VALUE) — on top of the manifest and settings.
    fn user_env(&self, name: &str) -> Vec<(String, String)> {
        std::fs::read_to_string(self.cfg.store_dir().join(name).join("user.env")).map(|t| parse_env(&t)).unwrap_or_default()
    }

    pub fn app_overrides(&self, name: &str) -> Result<AppOverrides> {
        self.manifest(name)?;
        let dir = self.cfg.store_dir().join(name);
        Ok(AppOverrides {
            env: std::fs::read_to_string(dir.join("user.env")).unwrap_or_default(),
            compose: std::fs::read_to_string(dir.join("compose.override.yaml")).unwrap_or_default(),
            effective_compose: self.manifest(name).and_then(|m| self.compose_text(&m)).unwrap_or_default(),
        })
    }

    /// Save the overrides and, if the app is installed, redeploy the stack.
    pub async fn app_set_overrides(&self, name: &str, env: &str, compose: &str) -> Result<AppView> {
        let _g = self.lock.lock().await;
        let m = self.manifest(name)?;
        if !compose.trim().is_empty() {
            let v: serde_json::Value = serde_yaml_ng::from_str(compose).map_err(|e| anyhow!("имя compose.override.yaml: не YAML: {e}"))?;
            if !v.is_object() { bail!("имя compose.override.yaml: ожидался объект с services"); }
        }
        for (k, _) in parse_env(env) { if k.starts_with("APP_") && k != "APP_PORT" { bail!("имя {k}: служебные APP_* переменные переопределять нельзя"); } }
        let dir = self.cfg.store_dir().join(name);
        std::fs::write(dir.join("user.env"), env)?;
        std::fs::write(dir.join("compose.override.yaml"), compose)?;
        if let Some(st) = self.store.get_app(name)? {
            let env_pairs = self.app_env(&m, st.port)?;
            let env_text = env_pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
            let compose_text = self.compose_text(&m)?;
            for n in self.prepare_data_dirs(name, &compose_text, &env_pairs).await? { self.store.event("app", name, &format!("каталог данных {n}"))?; }
            if let (Some(id), Ok(Some(_))) = (&st.komodo_stack_id, self.komodo.get_stack(name).await) {
                self.komodo.update_stack(id, &compose_text, &env_text).await?;
                self.komodo.deploy(name).await?;
                self.komodo.wait_running(name, 240).await?;
                self.store.event("app", name, "переопределения применены, стек перевыкачен")?;
            }
        } else {
            self.store.event("app", name, "переопределения сохранены (применятся при установке)")?;
        }
        self.app(name).await
    }

    pub fn project_spec_from_repo(&self, name: &str) -> Result<Option<ProjectSpec>> {
        validate_name(name)?;
        let p = self.cfg.projects_dir().join(name).join("project.yaml");
        if !p.exists() {
            return Ok(None);
        }
        let spec: ProjectSpec = serde_yaml_ng::from_str(&std::fs::read_to_string(&p)?).with_context(|| format!("{}", p.display()))?;
        Ok(Some(spec))
    }
    pub fn write_project_spec(&self, spec: &ProjectSpec) -> Result<()> {
        let dir = self.cfg.projects_dir().join(&spec.project);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("project.yaml"), serde_yaml_ng::to_string(spec)?)?;
        Ok(())
    }

    // =====================================================================
    // Projects
    // =====================================================================

    /// Apply `project.yaml`: Incus network + macvlan + gate; workspace in Coder; grants by capabilities.
    pub async fn project_apply(&self, spec: ProjectSpec) -> Result<ProjectView> {
        let _g = self.lock.lock().await;
        validate_name(&spec.project)?;
        let spec = ProjectSpec { description: normalize_description(spec.description)?, ..spec };
        let base = &self.cfg.project_subnet_base;
        let now = store::now();
        let previous = self.store.get_project(&spec.project)?;
        // A change of the note alone does not touch the network, gate and workspace: write the spec and leave. A fully
        // identical spec takes the long path — that way `cloudd project apply` remains the way to repair a project.
        if let Some(p) = previous.as_ref().filter(|p| p.spec.description != spec.description && ProjectSpec { description: p.spec.description.clone(), ..spec.clone() } == p.spec) {
            let mut st = p.clone();
            st.spec = spec.clone();
            self.write_project_spec(&spec)?;
            self.store.upsert_project(&st)?;
            self.store.event("project", &st.name, if spec.description.is_some() { "описание обновлено" } else { "описание удалено" })?;
            return self.project_view(&st).await;
        }
        let workspace_changed = previous.as_ref().map(|p| p.spec.workspace.cpu != spec.workspace.cpu || p.spec.workspace.memory != spec.workspace.memory).unwrap_or(false);
        if let Some(p) = &previous {
            if p.spec.workspace.runtime != spec.workspace.runtime {
                bail!("runtime проекта {} нельзя сменить после создания ({} → {}): удалите проект и создайте заново", spec.project, p.spec.workspace.runtime.as_param(), spec.workspace.runtime.as_param());
            }
        }
        let mut st = match previous {
            Some(mut s) => {
                s.spec = spec.clone();
                s
            }
            None => ProjectState { name: spec.project.clone(), subnet_index: self.store.next_subnet_index()?, coder_workspace_id: None, spec: spec.clone(), created_at: now },
        };
        self.write_project_spec(&spec)?;
        self.store.upsert_project(&st)?;

        // 1. project network (Incus) with DNS *.gate
        // → gate
        let gw = st.gateway(base);
        let gate_ip = st.gate_ip(base);
        let dhcp = format!("{base}.{}.2-{base}.{}.239", st.subnet_index, st.subnet_index);
        self.incus.ensure_network(&st.incus_network(), &format!("{gw}/24"), &dhcp).await?;
        self.incus_set_gate_dns(&st.incus_network(), &gate_ip).await?;
        // 2. Docker macvlan on the project bridge +
        // gate
        self.docker.ensure_macvlan(&st.macvlan_network(), &st.incus_network(), &st.subnet(base), &gw, &st.gate_ip_range(base)).await?;
        let gate_dir = self.cfg.gates_dir().join(&st.name);
        self.render_gate(&st).await?;
        self.docker.ensure_gate(&st.gate_name(), &st.macvlan_network(), &gate_ip, &gate_dir).await?;
        if self.has_external_grants(&st.name)? {
            self.ensure_egress(&st).await?;
        }
        if let Err(e) = self.docker.reload_gate(&st.gate_name()).await {
            tracing::warn!("reload gate {}: {e}", st.gate_name());
        }
        self.store.event("project", &st.name, "сеть и gate готовы")?;

        // 3. workspace via Coder
        if self.cfg.coder_token.is_empty() {
            self.store.event("project", &st.name, "CLOUDD_CODER_TOKEN не задан — workspace пропущен")?;
        } else {
            let me = self.coder.me().await?;
            let existing = match st.coder_workspace_id {
                Some(id) => self.coder.workspace(id).await?,
                None => self.coder.workspace_by_name(&me.username, &st.name).await?,
            };
            match existing {
                Some(ws) if ws.simple_status() != "deleted" => {
                    st.coder_workspace_id = Some(ws.id);
                    if workspace_changed {
                        // new cpu/memory are applied by a start build with parameters (Incus updates the limits in
                        // place)
                        let params = self.rich_params(&st);
                        let b = self.coder.build_with_params(ws.id, "start", &params).await?;
                        self.store.event("workspace", &st.name, &format!("параметры изменены: {} CPU, {} GiB — build {}", st.spec.workspace.cpu, st.spec.workspace.memory, b.id))?;
                        self.push_env_when_running(st.clone());
                        // the instance is not rebooted, yet the new build has a different agent token: restart the
                        // agent unit, it re-reads user.coder_agent_token through the guest API
                        let svc = self.clone();
                        let (build_id, inst, name) = (b.id, format!("ws-{}-{}", ws.owner_name, ws.name).to_lowercase(), st.name.clone());
                        tokio::spawn(async move {
                            if svc.coder.wait_build(build_id, 300).await.is_ok() {
                                match svc.incus.exec(&inst, &["systemctl", "restart", "coder-agent"]).await {
                                    Ok(_) => { let _ = svc.store.event("workspace", &name, "агент перезапущен с новым токеном"); }
                                    Err(e) => tracing::warn!("restart coder-agent в {inst}: {e}"),
                                }
                            }
                        });
                    }
                }
                _ => {
                    let tpl = self.coder.template(&self.cfg.coder_org, &self.cfg.coder_template).await?;
                    let params = self.rich_params(&st);
                    let ws = self.coder.create_workspace(&me.username, &tpl, &st.name, &params).await?;
                    st.coder_workspace_id = Some(ws.id);
                    self.store.event("workspace", &st.name, &format!("создан workspace {} (build {})", ws.id, ws.latest_build.id))?;
                    // the instance is still being built: the capabilities env lands as soon as Incus reports running (the push at the end
                    // of apply will not catch it)
                    self.push_env_when_running(st.clone());
                }
            }
            self.store.upsert_project(&st)?;
        }

        // 4. grants by capabilities (what we can — we grant; what is missing —
        // into missing)
        let apps = self.store.list_apps()?;
        let current = self.store.list_grants(Some(&st.name))?;
        for cap in &spec.capabilities {
            if current.iter().any(|g| &g.capability == cap) {
                continue;
            }
            if let Some(app) = self.default_provider(cap, &apps)? {
                self.grant_inner(&st, cap, app).await?;
            }
        }
        for g in current.iter().filter(|g| !spec.capabilities.contains(&g.capability)) {
            self.revoke_inner(&st, &g.capability).await?;
        }
        self.render_gate(&st).await?;
        self.push_workspace_env(&st).await;
        self.project_view(&st).await
    }

    fn rich_params<'a>(&'a self, st: &'a ProjectState) -> Vec<RichParam<'a>> {
        let w = &st.spec.workspace;
        vec![
            RichParam { name: "runtime", value: w.runtime.as_param().to_string() },
            RichParam { name: "network", value: st.incus_network() },
            RichParam { name: "cpu", value: w.cpu.to_string() },
            RichParam { name: "memory_gb", value: w.memory.to_string() },
            RichParam { name: "home_gb", value: w.home.to_string() },
            RichParam { name: "image", value: w.image.clone().unwrap_or_else(|| self.cfg.workspace_image.clone()) },
        ]
    }

    async fn incus_set_gate_dns(&self, network: &str, gate_ip: &str) -> Result<()> {
        // the project network's dnsmasq answers *.gate with the gate address — so the workspace sees
        // http://s3.gate without /etc/hosts
        let v: serde_json::Value = serde_json::json!({ "raw.dnsmasq": format!("address=/.gate/{gate_ip}") });
        self.incus.patch_network(network, v).await
    }

    async fn render_gate(&self, st: &ProjectState) -> Result<()> {
        let grants = self.store.list_grants(Some(&st.name))?;
        let apps = self.store.list_apps()?;
        let mut externals = HashMap::new();
        for a in apps.iter().filter(|a| a.manifest.kind == AppKind::External) {
            match self.external_target(a) {
                Ok(t) => { externals.insert(a.name.clone(), t); }
                Err(e) => tracing::warn!("gate {}: внешний провайдер {}: {e:#}", st.name, a.name),
            }
        }
        let routes = gate::routes_for(&grants, &apps, &externals);
        let text = gate::render_caddyfile(&st.name, &routes);
        let dir = self.cfg.gates_dir().join(&st.name);
        if gate::write_caddyfile(&dir, &text)? && self.docker.container_exists(&st.gate_name()).await? {
            if let Err(e) = self.docker.reload_gate(&st.gate_name()).await {
                tracing::warn!("reload gate {}: {e}", st.gate_name());
            }
        }
        Ok(())
    }

    pub async fn project_delete(&self, name: &str) -> Result<()> {
        let _g = self.lock.lock().await;
        let Some(st) = self.store.get_project(name)? else { bail!("проект {name} не найден") };
        // reverse order: revoke → workspace → gate →
        // networks
        for g in self.store.list_grants(Some(name))? {
            self.revoke_inner(&st, &g.capability).await.ok();
        }
        if let Some(id) = st.coder_workspace_id {
            if let Some(ws) = self.coder.workspace(id).await? {
                if ws.simple_status() != "deleted" {
                    let b = self.coder.build(id, "delete").await?;
                    self.coder.wait_build(b.id, 300).await?;
                    self.store.event("workspace", name, "workspace удалён")?;
                }
            }
        }
        self.docker.remove_container(&st.gate_name()).await?;
        self.docker.remove_network(&st.macvlan_network()).await?;
        self.incus.delete_network(&st.incus_network()).await?;
        let _ = std::fs::remove_dir_all(self.cfg.gates_dir().join(name));
        let _ = std::fs::remove_dir_all(self.cfg.projects_dir().join(name));
        self.store.delete_project(name)?;
        self.store.event("project", name, "проект удалён")?;
        Ok(())
    }

    pub async fn workspace_transition(&self, name: &str, transition: &str) -> Result<ProjectView> {
        let Some(st) = self.store.get_project(name)? else { bail!("проект {name} не найден") };
        let id = st.coder_workspace_id.ok_or_else(|| anyhow!("у проекта {name} нет workspace"))?;
        // start of a lagging workspace — right away on the current version of its template (like `coder update`): new
        // extensions, the agent runner and cloud-init of new instances arrive without a separate step. Parameters — from
        // the spec, as at creation.
        let latest = match self.coder.workspace(id).await? {
            Some(ws) if transition == "start" && ws.outdated => ws.template_active_version_id,
            _ => None,
        };
        let b = match latest {
            Some(v) => self.coder.build_on_version(id, transition, v, &self.rich_params(&st)).await?,
            None => self.coder.build(id, transition).await?,
        };
        let note = if latest.is_some() { " (обновление до актуальной версии шаблона)" } else { "" };
        self.store.event("workspace", name, &format!("{transition}: build {}{note}", b.id))?;
        if transition == "start" {
            self.push_env_when_running(st.clone());
        }
        self.project_view(&st).await
    }

    /// The container comes up asynchronously after start, and the grants may have changed while the workspace was
    /// stopped (a push on grant into a stopped instance is impossible): place the env contract as soon as Incus
    /// reports running.
    fn push_env_when_running(&self, st: ProjectState) {
        let s = self.clone();
        tokio::spawn(async move {
            let inst = s.workspace_instance(&st.name).await;
            for _ in 0..90 {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if s.incus.instance_running(&inst).await.unwrap_or(false) {
                    if s.push_workspace_env(&st).await {
                        let _ = s.store.event("workspace", &st.name, "env capabilities записан после старта");
                    }
                    return;
                }
            }
            tracing::warn!("{}: инстанс {inst} не поднялся за 3 минуты — env capabilities не записан", st.name);
        });
    }

    pub async fn projects(&self) -> Result<Vec<ProjectView>> {
        let mut out = Vec::new();
        for st in self.store.list_projects()? {
            out.push(self.project_view(&st).await?);
        }
        Ok(out)
    }
    pub async fn project(&self, name: &str) -> Result<ProjectView> {
        let st = self.store.get_project(name)?.ok_or_else(|| anyhow!("проект {name} не найден"))?;
        self.project_view(&st).await
    }

    async fn project_view(&self, st: &ProjectState) -> Result<ProjectView> {
        let base = &self.cfg.project_subnet_base;
        let gate_status = self.docker.container_state(&st.gate_name()).await.unwrap_or(None).unwrap_or_else(|| "missing".into());
        let grants = self.store.list_grants(Some(&st.name))?;
        let apps = self.store.list_apps()?;
        let missing = st.spec.capabilities.iter().filter(|c| !apps.iter().any(|a| a.manifest.provides.contains(c))).cloned().collect();
        let workspace = match (st.coder_workspace_id, self.cfg.coder_token.is_empty()) {
            (Some(id), false) => match self.coder.workspace(id).await {
                Ok(Some(ws)) => {
                    let inst = self.incus.instances().await.unwrap_or_default();
                    let mine = ws.instance_name().and_then(|n| inst.into_iter().find(|i| i.name == n));
                    let public = self.cfg.coder_public_url();
                    let agent = ws.agent();
                    Some(WorkspaceView {
                        id: ws.id,
                        name: ws.name.clone(),
                        status: ws.simple_status(),
                        agent_status: agent.map(|a| if a.lifecycle_state.is_empty() { a.status.clone() } else { format!("{}/{}", a.status, a.lifecycle_state) }),
                        instance: mine.as_ref().map(|i| i.name.clone()),
                        ip: mine.as_ref().and_then(|i| i.ip.clone()),
                        apps: agent
                            .map(|a| {
                                a.apps
                                    .iter()
                                    .map(|app| WorkspaceApp {
                                        slug: app.slug.clone(),
                                        display_name: if app.display_name.is_empty() { app.slug.clone() } else { app.display_name.clone() },
                                        url: format!("{public}/@{}/{}.{}/apps/{}/", ws.owner_name, ws.name, a.name, app.slug),
                                        health: app.health.clone(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        url: format!("{public}/@{}/{}", ws.owner_name, ws.name),
                    })
                }
                Ok(None) => None,
                Err(e) => Some(WorkspaceView { id, name: st.name.clone(), status: format!("unknown: {e}"), agent_status: None, instance: None, ip: None, apps: vec![], url: self.cfg.coder_public_url() }),
            },
            _ => None,
        };
        let mut grant_env = std::collections::BTreeMap::new();
        for g in &grants {
            grant_env.insert(g.capability.clone(), self.store.list_secret_keys(&format!("project:{}:{}", st.name, g.capability))?);
        }
        let mut available: Vec<String> = apps.iter().flat_map(|a| a.manifest.provides.iter().cloned()).collect();
        available.sort();
        available.dedup();
        let mut providers: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for a in &apps {
            for c in &a.manifest.provides { providers.entry(c.clone()).or_default().push(a.name.clone()); }
        }
        for v in providers.values_mut() { v.sort(); }
        Ok(ProjectView {
            name: st.name.clone(),
            spec: st.spec.clone(),
            network: st.incus_network(),
            subnet: st.subnet(base),
            gate: ComponentHealth { name: st.gate_name(), status: gate_status, detail: Some(st.gate_ip(base)) },
            workspace,
            grants,
            missing_capabilities: missing,
            grant_env,
            available_capabilities: available,
            providers,
            agents: self.agents.cache.read().unwrap().get(&st.name).cloned(),
        })
    }

    // =====================================================================
    // Apps (App Runtime = Komodo)
    // =====================================================================

    pub async fn apps(&self) -> Result<Vec<AppView>> {
        let installed = self.store.list_apps()?;
        let grants = self.store.list_grants(None)?;
        // stack states — one request to Komodo, not one per app
        let states = if installed.is_empty() { Default::default() } else { self.komodo.states().await.unwrap_or_default() };
        let mut out = Vec::new();
        for m in self.catalog()? {
            let inst = installed.iter().find(|a| a.name == m.name);
            let state = match inst {
                Some(a) if a.manifest.kind == AppKind::External => Some(self.external_state_cached(&m.name)),
                Some(_) => states.get(&m.name).cloned(),
                None => None,
            };
            out.push(self.app_view_with(&m, inst, &grants, state)?);
        }
        Ok(out)
    }

    /// One app (card, 🔑 in the window) — without walking the whole
    /// catalog.
    pub async fn app(&self, name: &str) -> Result<AppView> {
        let m = self.manifest(name)?;
        let inst = self.store.get_app(name)?;
        let grants = self.store.list_grants(None)?;
        self.app_view(&m, inst.as_ref(), &grants).await
    }

    async fn app_view(&self, m: &AppManifest, inst: Option<&AppState>, grants: &[Grant]) -> Result<AppView> {
        let state = match inst {
            Some(a) if a.manifest.kind == AppKind::External => Some(self.external_state_probe(a).await),
            Some(_) => self.komodo.status(&m.name).await.ok().flatten().map(|s| s.state),
            None => None,
        };
        self.app_view_with(m, inst, grants, state)
    }

    fn app_view_with(&self, m: &AppManifest, inst: Option<&AppState>, grants: &[Grant], state: Option<String>) -> Result<AppView> {
        let mut manifest = inst.map(|a| a.manifest.clone()).unwrap_or(m.clone());
        // open mode and login data — current ones from the catalog; the user's choice on top of the mode (kv app.open.<name>)
        manifest.route.open = m.route.open;
        manifest.login = m.login.clone();
        manifest.recommended = m.recommended.clone();
        if let Some(v) = self.store.get_kv(&format!("app.open.{}", m.name))?.and_then(|v| v.as_str().map(|s| s.to_string())) {
            manifest.route.open = if v == "newtab" { OpenMode::Newtab } else { OpenMode::Iframe };
        }
        if manifest.kind == AppKind::External { manifest.route.open = OpenMode::Newtab; } // a foreign site will not open in an iframe
        let mut settings_values = std::collections::BTreeMap::new();
        for (k, spec) in &manifest.env {
            let secret = matches!(spec, EnvSpec::Generate { .. }) || manifest.settings.iter().any(|f| f.get("env").and_then(|e| e.as_str()) == Some(k) && matches!(f.get("type").and_then(|t| t.as_str()), Some("password" | "random")));
            let v = self.store.get_secret(&format!("app:{}", manifest.name), k)?.unwrap_or_else(|| match spec { EnvSpec::Literal(s) => s.clone(), _ => String::new() });
            settings_values.insert(k.clone(), if secret && !v.is_empty() { "••••••".into() } else { v });
        }
        let links = self.store.list_links()?;
        let link_env: std::collections::BTreeMap<String, String> = if inst.is_some() {
            self.link_env(m)?.into_iter().map(|(k, v)| { let secret = ["KEY", "TOKEN", "PASSWORD", "SECRET"].iter().any(|w| k.contains(w)); (k, if secret && !v.is_empty() { "••••••".into() } else { v }) }).collect()
        } else { Default::default() };
        Ok(AppView {
            has_note: !self.store.list_secret_keys(&format!("note:{}", m.name))?.is_empty(),
            links_to: links.iter().filter(|l| l.consumer == m.name).map(|l| l.provider.clone()).collect(),
            links_from: links.iter().filter(|l| l.provider == m.name).map(|l| l.consumer.clone()).collect(),
            shared_net: self.is_shared(&m.name)?,
            internal_url: inst.filter(|a| a.manifest.kind != AppKind::External).map(internal_url),
            link_env,
            name: m.name.clone(),
            url: inst.map(|a| self.app_url(a)),
            port: inst.and_then(|a| a.port),
            installed: inst.is_some(),
            state,
            granted_to: grants.iter().filter(|g| g.app == m.name).map(|g| g.project.clone()).collect(),
            manifest,
            settings_values,
        })
    }

    pub(crate) fn app_url(&self, a: &AppState) -> String {
        if a.manifest.kind == AppKind::External {
            return self.external_target(a).map(|(u, _)| u).unwrap_or_default();
        }
        match a.manifest.route.mode {
            RouteMode::Path => format!("https://{}{}", self.cfg.edge_host, a.manifest.route.prefix.clone().unwrap_or_else(|| format!("/apps/{}/", a.name))),
            RouteMode::Port => format!("https://{}:{}/", self.cfg.edge_host, a.port.unwrap_or(0)),
            RouteMode::Host => format!("https://{}.{}/", a.name, self.cfg.edge_host),
        }
    }

    /// Address and key of an external provider from its settings: `https://host[:port]` without a path (Caddy does not accept
    /// a path in an upstream).
    pub(crate) fn external_target(&self, a: &AppState) -> Result<(String, Option<String>)> {
        let spec = a.manifest.external.as_ref().ok_or_else(|| anyhow!("{}: type external без раздела external", a.name))?;
        let scope = format!("app:{}", a.name);
        let raw = self
            .store
            .get_secret(&scope, &spec.url_env)?
            .filter(|v| !v.trim().is_empty())
            .or_else(|| match a.manifest.env.get(&spec.url_env) { Some(EnvSpec::Literal(s)) if !s.is_empty() => Some(s.clone()), _ => None })
            .ok_or_else(|| anyhow!("{}: не задан адрес ({})", a.name, spec.url_env))?;
        let url = normalize_external_url(&raw)?;
        let key = match &spec.key_env { Some(k) => self.store.get_secret(&scope, k)?.filter(|v| !v.trim().is_empty()), None => None };
        Ok((url, key))
    }

    /// Does the external provider answer? Any HTTP status means "available" (401 without a key too), a
    /// connection/TLS/DNS error — no.
    async fn external_reachable(&self, a: &AppState) -> Result<String> {
        let (url, key) = self.external_target(a)?;
        let path = a.manifest.external.as_ref().and_then(|e| e.check_path.clone()).unwrap_or_else(|| "/".into());
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build()?;
        let mut req = client.get(format!("{url}{path}"));
        if let Some(k) = &key { req = req.bearer_auth(k).header("x-api-key", k); }
        let r = req.send().await.map_err(|e| anyhow!("{url}{path}: {e}"))?;
        Ok(format!("HTTP {}", r.status().as_u16()))
    }

    /// External provider state for the card: a live check, the result is cached for the Store list.
    async fn external_state_probe(&self, a: &AppState) -> String {
        let state = match self.external_reachable(a).await {
            Ok(_) => "running",
            Err(e) => { tracing::warn!("внешний провайдер {}: {e:#}", a.name); "unreachable" }
        };
        let _ = self.store.put_kv(&format!("app.external.state.{}", a.name), &serde_json::Value::String(state.into()));
        state.to_string()
    }
    fn external_state_cached(&self, name: &str) -> String {
        self.store.get_kv(&format!("app.external.state.{name}")).ok().flatten().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "running".into())
    }

    /// External provider: nothing is started — check the address and reachability from the server, record the state, issue pending
    /// grants.
    async fn external_install(&self, name: &str, m: &AppManifest) -> Result<AppView> {
        let now = store::now();
        let mut st = self.store.get_app(name)?.unwrap_or(AppState { name: name.to_string(), manifest: m.clone(), port: None, komodo_stack_id: None, installed_at: now });
        st.manifest = m.clone();
        let (url, key) = self.external_target(&st)?;
        let probe = self.external_reachable(&st).await.with_context(|| format!("{name}: внешний адрес {url} недоступен с сервера"))?;
        self.store.upsert_app(&st)?;
        self.store.put_kv(&format!("app.external.state.{name}"), &serde_json::Value::String("running".into()))?;
        self.store.event("app", name, &format!("внешний провайдер {url} ({probe}{})", if key.is_some() { ", ключ задан" } else { ", без ключа" }))?;
        for p in self.store.list_projects()? {
            for cap in &p.spec.capabilities {
                if m.provides.contains(cap) && !self.store.list_grants(Some(&p.name))?.iter().any(|g| &g.capability == cap) {
                    self.grant_inner(&p, cap, &st).await?;
                    self.render_gate(&p).await?;
                }
            }
        }
        self.app(name).await
    }

    /// install(manifest, env): core network → env from the secret store → CreateStack/UpdateStack + DeployStack → edge
    /// route.
    pub async fn app_install(&self, name: &str) -> Result<AppView> {
        let _g = self.lock.lock().await;
        let m = self.manifest(name)?;
        for req in &m.requires {
            let apps = self.store.list_apps()?;
            if !apps.iter().any(|a| a.manifest.provides.contains(req)) {
                bail!("{name} требует capability {req}: установите приложение, которое её provides");
            }
        }
        if m.kind == AppKind::External {
            return self.external_install(name, &m).await;
        }
        let now = store::now();
        let mut st = self.store.get_app(name)?.unwrap_or(AppState { name: name.to_string(), manifest: m.clone(), port: None, komodo_stack_id: None, installed_at: now });
        st.manifest = m.clone();
        if m.route.mode == RouteMode::Port && st.port.is_none() {
            st.port = Some(self.alloc_port(m.route.port)?);
        }
        // 1. app network — created by the core (ADR-005
        // decision 3)
        let net = st.network();
        self.docker.ensure_bridge_network(&net, HashMap::from([("cloudos.app".into(), name.to_string())])).await?;
        // 1b. read-only docker proxy: we create the network (the proxy itself comes from
        // bootstrap/docker-proxy)
        if self.compose_text(&m)?.contains(crate::importer::DOCKER_RO_NET) {
            self.docker.ensure_bridge_network(crate::importer::DOCKER_RO_NET, HashMap::from([("cloudos.kind".into(), "docker-ro".into())])).await?;
            if !self.docker.container_exists("cloudos-docker-ro").await? {
                self.store.event("app", name, "ВНИМАНИЕ: прокси cloudos-docker-ro не запущен (bootstrap/docker-proxy) — доступ к Docker у приложения не будет")?;
            }
        }
        // 2. app data
        let data_dir = self.cfg.apps_data_dir.join(name);
        ensure_data_dir(&data_dir)?;
        // 3. env: generated → secret store (once), literal — as is
        let env_pairs = self.app_env(&m, st.port)?;
        let env_text = env_pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
        let compose = self.compose_text(&m)?;
        // directories under bind volumes (${APP_DATA_DIR}/…) must exist before `compose up` (Docker does not create the device for
        // driver_opts bind) and belong to the container user (appdata.rs: the image's user:/PUID/USER, otherwise 1000:1000 as in
        // Umbrel/Runtipi)
        for n in self.prepare_data_dirs(name, &compose, &env_pairs).await? { self.store.event("app", name, &format!("каталог данных {n}"))?; }
        // 4. Komodo
        match (&st.komodo_stack_id, self.komodo.get_stack(name).await?) {
            (Some(id), Some(_)) => self.komodo.update_stack(id, &compose, &env_text).await?,
            (_, Some(existing)) => {
                let id = existing.get("_id").and_then(|i| i.get("$oid")).and_then(|o| o.as_str()).unwrap_or_default().to_string();
                self.komodo.update_stack(&id, &compose, &env_text).await?;
                st.komodo_stack_id = Some(id);
            }
            _ => st.komodo_stack_id = Some(self.komodo.create_stack(name, &compose, &env_text).await?),
        }
        self.store.upsert_app(&st)?;
        self.komodo.deploy(name).await?;
        self.store.event("app", name, "deploy запущен")?;
        self.spawn_watch(name);
        let status = self.komodo.wait_running(name, 240).await?;
        self.store.event("app", name, &format!("стек running: {} сервисов", status.services.len()))?;
        // Komodo considers the stack running even if the container is in a crash loop — check health from the
        // manifest from the host
        if let Some(h) = &m.health {
            self.wait_health(&st, h, 90).await.with_context(|| format!("{name}: health {h} не прошёл; логи: cloudd app logs {name}"))?;
            self.store.event("app", name, "health OK")?;
        }
        // 5. edge route: edge Caddy joins the app network and reloads its config
        self.docker.connect(&net, &self.cfg.edge_container).await?;
        self.reload_edge().await?;
        self.store.event("app", name, &format!("маршрут {}", self.app_url(&st)))?;
        // 6. if projects were waiting for this
        // capability — grant it
        for p in self.store.list_projects()? {
            for cap in &p.spec.capabilities {
                if m.provides.contains(cap) && !self.store.list_grants(Some(&p.name))?.iter().any(|g| &g.capability == cap) {
                    self.grant_inner(&p, cap, &st).await?;
                    self.render_gate(&p).await?;
                }
            }
        }
        self.app(name).await
    }

    /// uninstall: revoke at the projects → edge route → DestroyStack/DeleteStack → network → record (the data in tank/apps
    /// stays). Remove the app. `purge` — together with the data in tank/apps, secrets, notes and overrides; otherwise all of
    /// it is kept and a reinstall brings the app back with the same passwords.
    pub async fn app_remove(&self, name: &str, purge: bool) -> Result<()> {
        let _g = self.lock.lock().await;
        let Some(st) = self.store.get_app(name)? else { bail!("приложение {name} не установлено") };
        let external = st.manifest.kind == AppKind::External;
        let dependents: Vec<String> = self
            .store
            .list_apps()?
            .into_iter()
            .filter(|a| a.name != name && a.manifest.requires.iter().any(|r| st.manifest.provides.contains(r)))
            .map(|a| a.name)
            .collect();
        if !dependents.is_empty() {
            bail!("от {name} зависят: {} — удалите их сначала", dependents.join(", "));
        }
        if !external && self.backup_config().enabled {
            match crate::backup::snapshot_of(&["tank/apps"], &format!("pre-remove-{name}")).await { Ok(s) => self.store.event("app", name, &format!("снимок перед удалением: {s}"))?, Err(e) => tracing::warn!("снимок перед удалением {name}: {e:#}") }
        }
        let links = self.store.list_links()?;
        let consumers: Vec<String> = links.iter().filter(|l| l.provider == name).map(|l| l.consumer.clone()).collect();
        if !consumers.is_empty() {
            bail!("к {name} подключены: {} — отключите связи сначала", consumers.join(", "));
        }
        for l in links.iter().filter(|l| l.consumer == name) {
            self.store.delete_secrets(&format!("link:{}:{}", l.consumer, l.provider))?;
            self.store.delete_link(&l.consumer, &l.provider)?;
        }
        for g in self.store.list_grants(None)?.into_iter().filter(|g| g.app == name) {
            if let Some(p) = self.store.get_project(&g.project)? {
                self.revoke_inner(&p, &g.capability).await.ok();
                self.render_gate(&p).await.ok();
            }
        }
        if !external {
            self.docker.disconnect(&st.network(), &self.cfg.edge_container).await.ok();
            if self.komodo.get_stack(name).await?.is_some() {
                self.komodo.destroy(name).await?;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                if let Some(id) = &st.komodo_stack_id {
                    self.komodo.delete_stack(id).await?;
                }
            }
            // the network may have vanished during an interrupted previous attempt — deletion must be
            // carried through
            if let Err(e) = self.docker.remove_network(&st.network()).await {
                tracing::warn!("сеть {}: {e:#}", st.network());
            }
        }
        self.store.put_kv(&format!("app.external.state.{name}"), &serde_json::Value::Null)?;
        self.store.delete_app(name)?;
        self.reload_edge().await?;
        if purge {
            validate_name(name)?;
            self.store.delete_secrets(&format!("app:{name}"))?;
            self.store.put_kv(&format!("app.open.{name}"), &serde_json::Value::Null)?;
            self.store.put_kv(&format!("app.shared.{name}"), &serde_json::Value::Null)?;
            self.store.delete_secrets(&format!("note:{name}"))?;
            let dir = self.cfg.apps_data_dir.join(name);
            if dir.exists() {
                let root = self.cfg.apps_data_dir.canonicalize()?;
                if !dir.canonicalize()?.starts_with(&root) {
                    bail!("каталог данных {} вне {}", dir.display(), root.display());
                }
                std::fs::remove_dir_all(&dir).with_context(|| format!("удаление {}", dir.display()))?;
            }
            for f in ["user.env", "compose.override.yaml"] {
                let _ = std::fs::remove_file(self.cfg.store_dir().join(name).join(f));
            }
            self.store.event("app", name, "удалено полностью: данные, секреты и переопределения")?;
        } else {
            self.store.event("app", name, "удалено (данные в tank/apps и пароли сохранены)")?;
        }
        Ok(())
    }

    /// How to open the app in Desktop: in a window (iframe) or a new tab — the user's choice over the manifest.
    pub fn app_set_open(&self, name: &str, open: &str) -> Result<()> {
        self.manifest(name)?;
        if !matches!(open, "iframe" | "newtab") { bail!("имя open: iframe | newtab"); }
        self.store.put_kv(&format!("app.open.{name}"), &serde_json::Value::String(open.into()))
    }

    /// App settings (the catalog's form_fields): save the values and, if installed, redeploy the stack.
    pub async fn app_configure(&self, name: &str, values: std::collections::BTreeMap<String, String>) -> Result<AppView> {
        let _g = self.lock.lock().await;
        let m = self.manifest(name)?;
        let scope = format!("app:{name}");
        for (k, v) in &values {
            if !m.env.contains_key(k) {
                bail!("{name}: переменная {k} не объявлена в манифесте");
            }
            if v == "••••••" {
                continue;
            }
            self.store.put_secret(&scope, k, v)?;
        }
        if let Some(st) = self.store.get_app(name)? {
            if st.manifest.kind == AppKind::External {
                let mut st = st;
                st.manifest = m.clone();
                let (url, _) = self.external_target(&st)?;
                let probe = self.external_reachable(&st).await.with_context(|| format!("{name}: внешний адрес {url} недоступен с сервера"))?;
                self.store.upsert_app(&st)?;
                self.store.put_kv(&format!("app.external.state.{name}"), &serde_json::Value::String("running".into()))?;
                for g in self.store.list_grants(None)?.into_iter().filter(|g| g.app == name) {
                    if let Some(p) = self.store.get_project(&g.project)? {
                        self.grant_inner(&p, &g.capability, &st).await?;
                        self.render_gate(&p).await?;
                    }
                }
                self.store.event("app", name, &format!("настройки применены: {url} ({probe}), гранты проектов обновлены"))?;
                return self.app(name).await;
            }
            let env_text = self.app_env(&m, st.port)?.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
            if let (Some(id), Ok(Some(_))) = (&st.komodo_stack_id, self.komodo.get_stack(name).await) {
                self.komodo.update_stack(id, &self.compose_text(&m)?, &env_text).await?;
                self.komodo.deploy(name).await?;
                self.komodo.wait_running(name, 240).await?;
                if let Some(h) = &m.health {
                    self.wait_health(&st, h, 90).await?;
                }
                self.store.event("app", name, "настройки применены, стек перевыкачен")?;
            }
        } else {
            self.store.event("app", name, "настройки сохранены (применятся при установке)")?;
        }
        self.app(name).await
    }

    pub async fn app_logs(&self, name: &str, tail: u32) -> Result<String> {
        let st = self.store.get_app(name)?.ok_or_else(|| anyhow!("приложение {name} не установлено"))?;
        if st.manifest.kind == AppKind::External { bail!("{name}: внешний сервис, логов у ядра нет"); }
        self.komodo.logs(name, &[st.manifest.endpoint.service.clone()], tail).await
    }

    /// health from the manifest: `http://<service>:<port>/path` → the same URL by the container IP in the app network (the host sees the
    /// bridge directly).
    async fn wait_health(&self, st: &AppState, health: &str, timeout_s: u64) -> Result<()> {
        let url = reqwest::Url::parse(health).context("health URL")?;
        let service = url.host_str().unwrap_or(&st.manifest.endpoint.service).to_string();
        let container = format!("{}-{service}-1", st.name);
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)).build()?;
        let start = std::time::Instant::now();
        let mut last = String::new();
        loop {
            if let Ok(ip) = self.docker.container_ip(&container, &st.network()).await {
                let mut u = url.clone();
                let _ = u.set_host(Some(&ip));
                match client.get(u.clone()).send().await {
                    Ok(r) if r.status().is_success() => return Ok(()),
                    Ok(r) => last = format!("HTTP {}", r.status()),
                    Err(e) => last = e.to_string(),
                }
            } else {
                last = format!("контейнер {container} без адреса в {}", st.network());
            }
            if start.elapsed().as_secs() > timeout_s {
                bail!("{last}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }

    pub(crate) fn alloc_port(&self, wanted: Option<u16>) -> Result<u16> {
        let mut used = self.store.used_ports()?;
        used.extend(self.store.previews()?.iter().map(|p| p.edge_port));
        let (lo, hi) = self.cfg.app_port_range;
        let reserved = |p: u16| p == self.cfg.coder_port || p == self.cfg.agents_port;
        if let Some(p) = wanted {
            if !used.contains(&p) && !reserved(p) {
                return Ok(p);
            }
        }
        (lo..=hi).find(|p| !used.contains(p) && !reserved(*p)).ok_or_else(|| anyhow!("нет свободных портов для route: port"))
    }

    // =====================================================================
    // Grants
    // =====================================================================

    /// `app` — which of the installed providers to use (default — see default_provider); for an already granted capability a
    /// provider change removes the old grant (on_revoke hook, network) and issues a new one.
    pub async fn grant(&self, project: &str, capability: &str, app: Option<&str>) -> Result<ProjectView> {
        let _g = self.lock.lock().await;
        let mut st = self.store.get_project(project)?.ok_or_else(|| anyhow!("проект {project} не найден"))?;
        let apps = self.store.list_apps()?;
        let providers: Vec<&AppState> = apps.iter().filter(|a| a.manifest.provides.contains(&capability.to_string())).collect();
        let app = match app {
            Some(n) => providers.iter().copied().find(|a| a.name == n).ok_or_else(|| anyhow!("{n} не установлено или не provides {capability}"))?,
            None => self.default_provider(capability, &apps)?.ok_or_else(|| anyhow!("нет установленного приложения, которое provides {capability}"))?,
        };
        let old = self.store.list_grants(Some(project))?.into_iter().find(|g| g.capability == capability && g.app != app.name);
        if old.is_some() {
            self.revoke_inner(&st, capability).await?;
        }
        if let Err(e) = self.grant_inner(&st, capability, app).await {
            // the provider change failed (hook, network): bring back the previous one so the project is not
            // left without the capability
            if let Some(old) = &old {
                if let Some(prev) = apps.iter().find(|a| a.name == old.app) {
                    match self.grant_inner(&st, capability, prev).await {
                        Ok(()) => { let _ = self.store.event("grant", project, &format!("{capability}: смена провайдера на {} не удалась, оставлен {}", app.name, prev.name)); }
                        Err(e2) => tracing::error!("{project}/{capability}: откат к {} тоже не удался: {e2:#}", prev.name),
                    }
                    self.render_gate(&st).await.ok();
                }
            }
            return Err(e);
        }
        if let Some(old) = &old {
            self.store.event("grant", project, &format!("{capability}: провайдер {} → {}", old.app, app.name))?;
        }
        if !st.spec.capabilities.iter().any(|c| c == capability) {
            st.spec.capabilities.push(capability.to_string());
            self.write_project_spec(&st.spec)?;
            self.store.upsert_project(&st)?;
        }
        self.render_gate(&st).await?;
        self.project_view(&st).await
    }
    pub async fn revoke(&self, project: &str, capability: &str) -> Result<ProjectView> {
        let _g = self.lock.lock().await;
        let mut st = self.store.get_project(project)?.ok_or_else(|| anyhow!("проект {project} не найден"))?;
        self.revoke_inner(&st, capability).await?;
        st.spec.capabilities.retain(|c| c != capability);
        self.write_project_spec(&st.spec)?;
        self.store.upsert_project(&st)?;
        self.render_gate(&st).await?;
        self.project_view(&st).await
    }

    /// grant = the gate joins `<app>-net` + the `<cap>.gate` route (no restarts — ADR-005) + the provider's `on_grant` hook,
    /// whose result (KEY=VALUE) becomes project secrets and the capability's env contract in the workspace (2.5). Default
    /// capability provider: with several installed — the one other projects already get this capability through (a new
    /// project behaves like the rest: "everyone through one external provider"), on a tie — the first by name.
    fn default_provider<'a>(&self, capability: &str, apps: &'a [AppState]) -> Result<Option<&'a AppState>> {
        let mut candidates: Vec<&AppState> = apps.iter().filter(|a| a.manifest.provides.iter().any(|c| c == capability)).collect();
        if candidates.len() > 1 {
            let grants = self.store.list_grants(None)?;
            candidates.sort_by_key(|a| std::cmp::Reverse(grants.iter().filter(|g| g.capability == capability && g.app == a.name).count()));
        }
        Ok(candidates.into_iter().next())
    }

    async fn grant_inner(&self, st: &ProjectState, capability: &str, app: &AppState) -> Result<()> {
        if app.manifest.kind != AppKind::External {
            self.docker.connect(&app.network(), &st.gate_name()).await?;
        } else {
            self.ensure_egress(st).await?;
        }
        let scope = format!("project:{}:{capability}", st.name);
        if let Some(hook) = &app.manifest.hooks.on_grant {
            let out = self.run_hook(app, hook, st, capability).await.with_context(|| format!("on_grant {} для {}", app.name, st.name))?;
            for (k, v) in parse_env(&out) {
                self.store.put_secret(&scope, &k, &v)?;
            }
        }
        // the capability address on the gate — always,
        // even without a hook
        self.store.put_secret(&scope, &format!("{}_URL", env_prefix(capability)), &format!("http://{capability}.gate"))?;
        self.store.put_grant(&Grant { project: st.name.clone(), capability: capability.to_string(), app: app.name.clone(), granted_at: store::now() })?;
        self.store.event("grant", &st.name, &format!("{capability} → {} (http://{capability}.gate)", app.name))?;
        self.push_workspace_env(st).await;
        Ok(())
    }
    async fn revoke_inner(&self, st: &ProjectState, capability: &str) -> Result<()> {
        let grants = self.store.list_grants(Some(&st.name))?;
        let Some(g) = grants.iter().find(|g| g.capability == capability) else { return Ok(()) };
        let app = self.store.get_app(&g.app)?;
        if let Some(app) = &app {
            if let Some(hook) = &app.manifest.hooks.on_revoke {
                if let Err(e) = self.run_hook(app, hook, st, capability).await {
                    tracing::warn!("on_revoke {} для {}: {e:#}", app.name, st.name);
                }
            }
            // the network may be disconnected only if no other grants of this project go through the same
            // app
            if app.manifest.kind != AppKind::External && !grants.iter().any(|o| o.capability != capability && o.app == g.app) {
                self.docker.disconnect(&app.network(), &st.gate_name()).await?;
            }
            // egress is needed while the project has at least one external
            // provider
            if app.manifest.kind == AppKind::External && !grants.iter().any(|o| o.capability != capability && self.is_external_app(&o.app)) {
                self.docker.disconnect(EGRESS_NET, &st.gate_name()).await?;
            }
        }
        self.store.delete_secrets(&format!("project:{}:{capability}", st.name))?;
        self.store.delete_grant(&st.name, capability)?;
        self.store.event("revoke", &st.name, &format!("{capability} снят"))?;
        self.push_workspace_env(st).await;
        Ok(())
    }

    /// An external provider lives outside the server, while the gate sits on the project macvlan, from where there is no
    /// path to the tailnet or the internet: for the duration of such grants the gate joins `cloudos-egress` (bridge, host
    /// NAT), which becomes its default gateway.
    async fn ensure_egress(&self, st: &ProjectState) -> Result<()> {
        self.docker.ensure_bridge_network(EGRESS_NET, HashMap::from([("cloudos.kind".into(), "egress".into())])).await?;
        self.docker.connect_gateway(EGRESS_NET, &st.gate_name(), 100).await.with_context(|| format!("{}: выход наружу для внешнего провайдера", st.gate_name()))
    }
    fn is_external_app(&self, name: &str) -> bool {
        matches!(self.store.get_app(name), Ok(Some(a)) if a.manifest.kind == AppKind::External)
    }
    fn has_external_grants(&self, project: &str) -> Result<bool> {
        Ok(self.store.list_grants(Some(project))?.iter().any(|g| self.is_external_app(&g.app)))
    }

    /// Provider hook: a script from the app's Store directory, executed on the host (cloudd sees app networks directly)
    /// with the app env (incl. generated secrets), PROJECT, CAPABILITY, APP_IP of the main service. stdout → KEY=VALUE.
    async fn run_hook(&self, app: &AppState, hook: &str, st: &ProjectState, capability: &str) -> Result<String> {
        self.run_hook_for(app, hook, &st.name, capability, &format!("http://{capability}.gate")).await
    }

    /// The same mechanics for app links: PROJECT=`app-<consumer>`, GATE_URL — the provider address inside Cloud OS.
    async fn run_hook_for(&self, app: &AppState, hook: &str, project: &str, capability: &str, gate_url: &str) -> Result<String> {
        let dir = self.cfg.store_dir().join(&app.name);
        let script = dir.join(hook);
        if !script.exists() {
            bail!("хук {} не найден", script.display());
        }
        let mut env = self.app_env(&app.manifest, app.port)?;
        env.push(("PROJECT".into(), project.to_string()));
        env.push(("CAPABILITY".into(), capability.to_string()));
        env.push(("GATE_URL".into(), gate_url.to_string()));
        if app.manifest.kind != AppKind::External {
            let ip = self.docker.container_ip(&format!("{}-{}-1", app.name, app.manifest.endpoint.service), &app.network()).await?;
            env.push(("APP_IP".into(), ip));
        }
        let out = tokio::process::Command::new("bash")
            .arg(&script)
            .current_dir(&dir)
            .env_clear()
            .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .output()
            .await?;
        if !out.status.success() {
            bail!("{} завершился с {}: {}", hook, out.status, String::from_utf8_lossy(&out.stderr).chars().take(500).collect::<String>());
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// App env for the stack and hooks: service variables + manifest (generated — from the secret
    /// store).
    pub(crate) fn app_env(&self, m: &AppManifest, port: Option<u16>) -> Result<Vec<(String, String)>> {
        let net = format!("{}-net", m.name);
        let data_dir = self.cfg.apps_data_dir.join(&m.name);
        let scope = format!("app:{}", m.name);
        let mut env = vec![
            ("APP_NAME".to_string(), m.name.clone()),
            ("APP_DATA_DIR".to_string(), data_dir.display().to_string()),
            ("APP_NET".to_string(), net),
            ("EDGE_HOST".to_string(), self.cfg.edge_host.clone()),
        ];
        if let Some(p) = port {
            env.push(("APP_PORT".into(), p.to_string()));
        }
        // Runtipi/Coolify directory conventions so their compose works without
        // edits
        env.push(("APP_ID".into(), m.name.clone()));
        env.push(("APP_HOST".into(), self.cfg.edge_host.clone()));
        env.push(("APP_PROTOCOL".into(), "https".into()));
        env.push(("APP_DOMAIN".into(), match port { Some(p) => format!("{}:{p}", self.cfg.edge_host), None => self.cfg.edge_host.clone() }));
        env.push(("APP_EXPOSED".into(), "true".into()));
        env.push(("ROOT_FOLDER_HOST".into(), data_dir.display().to_string()));
        env.push(("INTERNAL_IP".into(), self.cfg.edge_host.clone()));
        env.push(("TZ".into(), "Etc/UTC".into()));
        env.push(("TIPI_VERSION".into(), "cloudos".into()));
        for (k, spec) in &m.env {
            let v = match spec {
                // a literal can be overridden by a setting (stored in the app's secret store)
                EnvSpec::Literal(s) => self.store.get_secret(&scope, k)?.unwrap_or_else(|| s.clone()),
                EnvSpec::Generate { generate } => match self.store.get_secret(&scope, k)? {
                    Some(v) => v,
                    None => {
                        let v = store::generate(*generate);
                        self.store.put_secret(&scope, k, &v)?;
                        v
                    }
                },
            };
            env.push((k.clone(), v));
        }
        // links: the provider address always, keys from hooks — only if the app's value is empty (its own settings matter
        // more)
        for (k, v) in self.link_env(m)? {
            match env.iter_mut().find(|(ek, _)| *ek == k) {
                Some(e) if e.1.is_empty() || k.ends_with("_URL") => e.1 = v,
                Some(_) => {}
                None => env.push((k, v)),
            }
        }
        // user overrides (user.env) — on top of everything
        for (k, v) in self.user_env(&m.name) {
            if let Some(e) = env.iter_mut().find(|(ek, _)| *ek == k) { e.1 = v; } else { env.push((k, v)); }
        }
        // ${OTHER} references inside values (Umbrel/Coolify catalogs): compose does not expand values from .env — we do it
        // ourselves
        for _ in 0..3 {
            let snapshot = env.clone();
            let mut changed = false;
            for (_, v) in env.iter_mut() {
                if v.contains("${") {
                    let mut nv = v.clone();
                    for (ok, ov) in &snapshot {
                        let pat = format!("${{{ok}}}");
                        if nv.contains(&pat) && !ov.contains(&pat) { nv = nv.replace(&pat, ov); }
                    }
                    if nv != *v { *v = nv; changed = true; }
                }
            }
            if !changed { break; }
        }
        Ok(env)
    }

    /// Incus instance name of the project workspace:
    /// `ws-<owner>-<project>`.
    pub(crate) async fn workspace_instance(&self, project: &str) -> String {
        // TODO: owner from Coder once there is more than one user
        match self.coder_owner().await { Some(o) => format!("ws-{o}-{project}"), None => format!("ws-admin-{project}") }
    }

    const ENV_SH: &'static str = "/etc/cloudos/env.sh";
    /// Hooks that read env.sh: /etc/bash.bashrc for code-server terminals (an interactive non-login bash — ~/.profile is not read,
    /// hence the PATH to the agent CLIs here too), /etc/profile.d for login shells. Prints "profile.d created" if the stub was
    /// missing.
    const ENV_HOOKS_SH: &'static str = r#"chmod 0644 /etc/cloudos/env.sh 2>/dev/null; grep -q cloudos/env.sh /etc/bash.bashrc || echo '[ -r /etc/cloudos/env.sh ] && . /etc/cloudos/env.sh' >> /etc/bash.bashrc
grep -q npm-global/bin /etc/bash.bashrc || echo 'export PATH="$HOME/.npm-global/bin:$HOME/.local/bin:$PATH"' >> /etc/bash.bashrc
if [ ! -f /etc/profile.d/cloudos-env.sh ]; then
  printf '%s\n' '# Cloud OS: env capabilities проекта для login-оболочек (агент Coder -> code-server, расширения, терминалы); кладёт cloudd' 'if [ -r /etc/cloudos/env.sh ]; then . /etc/cloudos/env.sh; fi' > /etc/profile.d/cloudos-env.sh
  echo 'profile.d created'
fi"#;

    /// Capabilities env contract in the workspace: /etc/cloudos/env.sh — survives stop/start (rootfs). It is read by
    /// interactive terminals (/etc/bash.bashrc) and login shells (/etc/profile.d/cloudos-env.sh): the Coder agent starts via
    /// `su - coder`, so code-server, extensions and agent scripts inherit the variables. An already running agent will not see
    /// new values — when the file changed (or the profile.d stub has only just appeared) and the unit is active, it is
    /// restarted; on the first boot the unit is not active yet (cloud-init), and the agent simply starts with the env ready.
    /// Returns whether anything changed in the instance.
    async fn push_workspace_env(&self, st: &ProjectState) -> bool {
        let inst = self.workspace_instance(&st.name).await;
        if !self.incus.instance_running(&inst).await.unwrap_or(false) {
            return false;
        }
        let mut lines = vec!["# Cloud OS: env capabilities проекта — рендерит cloudd из grants, правки перезапишутся".to_string()];
        for g in self.store.list_grants(Some(&st.name)).unwrap_or_default() {
            let scope = format!("project:{}:{}", st.name, g.capability);
            lines.push(format!("# {} → {}", g.capability, g.app));
            for k in self.store.list_secret_keys(&scope).unwrap_or_default() {
                if let Ok(Some(v)) = self.store.get_secret(&scope, &k) {
                    lines.push(format!("export {k}={}", shell_quote(&v)));
                }
            }
        }
        let content = lines.join("\n") + "\n";
        let env_changed = self.incus.file_pull(&inst, Self::ENV_SH).await.ok().as_deref() != Some(content.as_bytes());
        if env_changed {
            if let Err(e) = self.incus.file_push(&inst, Self::ENV_SH, content.as_bytes(), 0o644).await {
                tracing::warn!("env в {inst}: {e}");
                return false;
            }
        }
        let hooks = self.incus.exec(&inst, &["sh", "-c", Self::ENV_HOOKS_SH]).await.unwrap_or_else(|e| {
            tracing::warn!("хуки env в {inst}: {e}");
            String::new()
        });
        let changed = env_changed || hooks.contains("profile.d created");
        self.push_workspace_agentd(st, &inst).await;
        if changed && self.incus.exec(&inst, &["systemctl", "is-active", "--quiet", "coder-agent"]).await.is_ok() {
            match self.incus.exec(&inst, &["systemctl", "try-restart", "coder-agent"]).await {
                Ok(_) => { let _ = self.store.event("workspace", &st.name, "env capabilities изменился — агент Coder перезапущен, code-server и терминалы переоткроются"); }
                Err(e) => tracing::warn!("try-restart coder-agent в {inst}: {e}"),
            }
        }
        changed
    }

    /// The agentd adapter (ADR-006) travels into the workspace together with env.sh: idempotent, an event only
    /// on changes.
    async fn push_workspace_agentd(&self, st: &ProjectState, inst: &str) {
        match crate::agentd::deploy(&self.incus, &self.store, inst, &st.name).await {
            Ok(d) if d.changed() => {
                let what = if d.binary { "agentd выложен" } else { "agentd настроен" };
                let _ = self.store.event("workspace", &st.name, &format!("{what}{}", if d.restarted { " и запущен" } else { "" }));
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("agentd в {inst}: {e:#}"),
        }
    }

    /// Claude Code permission bypass policy — into every running workspace; returns the names of those where the file
    /// changed.
    pub async fn apply_agents_policy(&self) -> Result<Vec<String>> {
        let running: std::collections::HashSet<String> = self.incus.instances().await?.into_iter().filter(|i| i.status == "Running").map(|i| i.name).collect();
        let mut changed = Vec::new();
        for st in self.store.list_projects()? {
            let inst = self.workspace_instance(&st.name).await;
            if !running.contains(&inst) { continue }
            match crate::agentd::apply_policy(&self.incus, &self.store, &inst).await {
                Ok(true) => { changed.push(st.name.clone()); let _ = self.store.event("workspace", &st.name, if crate::agentd::policy_bypass(&self.store) { "режим обхода разрешений Claude Code разрешён политикой" } else { "режим обхода разрешений Claude Code запрещён политикой" }); }
                Ok(false) => {}
                Err(e) => tracing::warn!("политика Claude Code в {inst}: {e:#}"),
            }
        }
        Ok(changed)
    }

    /// Client to the agentd of a running workspace: the address from Incus, the
    /// token from the secret store.
    pub async fn agentd_client(&self, project: &str) -> Result<crate::agentd::AgentdClient> {
        let inst = self.workspace_instance(project).await;
        let info = self.incus.instances().await?.into_iter().find(|i| i.name == inst).ok_or_else(|| anyhow!("workspace {project} не найден"))?;
        if info.status != "Running" {
            bail!("workspace {project} не запущен");
        }
        let ip = info.ip.ok_or_else(|| anyhow!("у workspace {project} ещё нет IP"))?;
        let token = self.store.get_secret(&crate::agentd::secret_scope(project), crate::agentd::TOKEN_KEY)?.ok_or_else(|| anyhow!("agentd в {project} ещё не выложен"))?;
        Ok(crate::agentd::AgentdClient::new(&ip, token))
    }

    async fn coder_owner(&self) -> Option<String> {
        if self.cfg.coder_token.is_empty() {
            return None;
        }
        self.coder.me().await.ok().map(|u| u.username.to_lowercase())
    }

    // =====================================================================
    // Edge, status
    // =====================================================================

    pub async fn reload_edge(&self) -> Result<()> {
        let apps = self.store.list_apps()?;
        let alt = self.edge_alt_hosts();
        let previews = self.preview_routes().await;
        let cfg = EdgeCaddy::render(&EdgeInput {
            host: &self.cfg.edge_host,
            alt_hosts: &alt,
            cloudd_upstream: &self.cfg.edge_cloudd_upstream,
            agents_upstream: &self.cfg.edge_agents_upstream,
            coder_upstream: &self.cfg.edge_coder_upstream,
            coder_port: self.cfg.coder_port,
            agents_port: self.cfg.agents_port,
            apps: &apps,
            previews: &previews,
            tls_internal: self.cfg.tls_internal,
        });
        self.edge.load(&cfg).await
    }

    /// Dev server previews (docs/agents-ux.md §4.9): routes for the edge render — upstream by the IP of the running
    /// workspace
    async fn preview_routes(&self) -> Vec<crate::caddy::PreviewRoute> {
        let list = self.store.previews().unwrap_or_default();
        if list.is_empty() { return Vec::new(); }
        let instances = self.incus.instances().await.unwrap_or_default();
        let mut out = Vec::new();
        for p in list {
            let inst = self.workspace_instance(&p.project).await;
            let ip = instances.iter().find(|i| i.name == inst && i.status == "Running").and_then(|i| i.ip.clone());
            out.push(crate::caddy::PreviewRoute { project: p.project.clone(), port: p.port, edge_port: p.edge_port, upstream: ip.map(|ip| format!("{ip}:{}", p.port)) });
        }
        out
    }
    pub fn preview_url(&self, edge_port: u16) -> String { format!("https://{}:{}/", self.cfg.edge_host, edge_port) }
    pub fn previews(&self, project: &str) -> Result<Vec<crate::model::Preview>> { Ok(self.store.previews()?.into_iter().filter(|p| p.project == project).collect()) }
    /// Enable a port preview: an existing route is returned as is, a new one takes a free edge port and re-renders the edge
    pub async fn preview_add(&self, project: &str, port: u16) -> Result<crate::model::Preview> {
        let _g = self.lock.lock().await;
        self.store.get_project(project)?.ok_or_else(|| anyhow!("проект {project} не найден"))?;
        if port == 0 { bail!("порт dev-сервера не задан"); }
        let mut list = self.store.previews()?;
        if let Some(p) = list.iter().find(|p| p.project == project && p.port == port) { return Ok(p.clone()); }
        let edge_port = self.alloc_port(None)?;
        let p = crate::model::Preview { project: project.to_string(), port, edge_port, created_at: crate::store::now() };
        list.push(p.clone());
        self.store.put_previews(&list)?;
        if let Err(e) = self.reload_edge().await { tracing::warn!("edge после превью {project}:{port}: {e:#}"); }
        let _ = self.store.event("workspace", project, &format!("превью dev-сервера :{port} опубликовано на порту {edge_port}"));
        Ok(p)
    }
    pub async fn preview_remove(&self, project: &str, port: u16) -> Result<bool> {
        let _g = self.lock.lock().await;
        let mut list = self.store.previews()?;
        let n = list.len();
        list.retain(|p| !(p.project == project && p.port == port));
        if list.len() == n { return Ok(false); }
        self.store.put_previews(&list)?;
        if let Err(e) = self.reload_edge().await { tracing::warn!("edge после снятия превью {project}:{port}: {e:#}"); }
        let _ = self.store.event("workspace", project, &format!("превью dev-сервера :{port} снято"));
        Ok(true)
    }

    pub async fn status(&self) -> Result<SystemStatus> {
        let mut c = Vec::new();
        let h = |name: &str, r: Result<String>| match r {
            Ok(v) => ComponentHealth { name: name.into(), status: "ok".into(), detail: Some(v) },
            Err(e) => ComponentHealth { name: name.into(), status: "error".into(), detail: Some(e.to_string()) },
        };
        c.push(h("docker", self.docker.ping().await));
        c.push(h("incus", self.incus.ping().await));
        c.push(h("coder", self.coder.health().await));
        c.push(h("komodo", self.komodo.ping().await));
        c.push(h("caddy", self.edge.ping().await.map(|_| "admin api".into())));
        let (update_available, app_updates) = self.update_summary();
        Ok(SystemStatus { edge_host: self.cfg.edge_host.clone(), edge_alt_hosts: self.edge_alt_hosts(), coder_url: self.cfg.coder_public_url(), agents_url: self.cfg.agents_public_url(), coder_user: self.cfg.admin_email.clone(), coder_password_set: self.cfg.admin_password.is_some(), components: c, version: crate::updates::VERSION.into(), build: crate::updates::build_id().into(), setup_done: self.setup_done(), update_available, app_updates })
    }

    /// Start the background metrics collector (serve mode only).
    pub fn start_monitor(&self) {
        let store = self.store.clone();
        self.monitor.clone().spawn(self.docker.clone(), self.incus.clone(), move || store.list_apps().map(|a| a.into_iter().map(|x| x.name).collect()).unwrap_or_default());
    }

    /// Join the edge to the app networks and upload its configuration: at start and when the edge container was
    /// recreated (it comes up with the bootstrap config without our servers — without this the stand stays silent
    /// until a core restart).
    pub async fn restore_edge(&self) -> Result<()> {
        for st in self.store.list_apps()? {
            if st.manifest.kind == AppKind::External { continue; }
            if let Err(e) = self.docker.connect(&st.network(), &self.cfg.edge_container).await {
                let m = format!("{e:#}");
                if !m.contains("already") { tracing::warn!("reconcile: edge → сеть {}: {m}", st.network()); }
            }
        }
        self.reload_edge().await
    }

    /// Edge watchdog: every 20 s checks whether the live Caddy configuration has the `desktop` server; if not (container
    /// recreated) — restores the networks and configuration itself.
    pub fn start_edge_watch(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                match me.edge.has_server("desktop").await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!("edge: в конфигурации Caddy нет сервера desktop (контейнер пересоздан?) — заливаю заново");
                        if let Err(e) = me.restore_edge().await { tracing::warn!("edge: восстановление не удалось: {e:#}"); } else { tracing::info!("edge: конфигурация восстановлена"); }
                    }
                    Err(_) => {}   // the admin API is unavailable — the container is still coming up; next check in 20 s
                }
            }
        });
    }

    /// Reconcile everything: re-read the repository and apply every project; bring up
    /// the edge.
    pub async fn reconcile_all(&self) -> Result<()> {
        // the edge may have been recreated (new image, new volumes): connections to app networks are lost then — restore them
        self.restore_edge().await?;
        let dir = self.cfg.projects_dir();
        if dir.exists() {
            for e in std::fs::read_dir(&dir)? {
                let name = e?.file_name().to_string_lossy().to_string();
                if let Some(spec) = self.project_spec_from_repo(&name)? {
                    if let Err(err) = self.project_apply(spec).await {
                        tracing::error!("reconcile {name}: {err:#}");
                    }
                }
            }
        }
        Ok(())
    }
}

/// Project note: surrounding whitespace is trimmed, an empty one becomes absence, longer than 2000 characters is
/// rejected.
pub fn normalize_description(d: Option<String>) -> Result<Option<String>> {
    let d = d.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(n) = d.as_ref().map(|s| s.chars().count()).filter(|n| *n > 2000) {
        bail!("описание проекта: до 2000 символов, получено {n}");
    }
    Ok(d)
}

pub fn validate_name(n: &str) -> Result<()> {
    if n.is_empty() || n.len() > 48 || !n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || n.starts_with('-') {
        bail!("имя {n:?}: только [a-z0-9-], до 48 символов");
    }
    Ok(())
}

/// KEY=VALUE per line; lines without `=` and comments are skipped.
pub fn parse_env(out: &str) -> Vec<(String, String)> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string())))
        .filter(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .collect()
}

/// `https://host[:port]/` → `https://host[:port]`: http(s) scheme, host required, path forbidden — the gate proxies the
/// root, paths (`/v1` etc.) are added by clients.
pub fn normalize_external_url(raw: &str) -> Result<String> {
    let u = reqwest::Url::parse(raw.trim()).map_err(|e| anyhow!("адрес {raw:?}: {e}"))?;
    if !matches!(u.scheme(), "http" | "https") { bail!("адрес {raw:?}: нужна схема http или https"); }
    let host = u.host_str().ok_or_else(|| anyhow!("адрес {raw:?}: нет хоста"))?;
    if !matches!(u.path(), "" | "/") || u.query().is_some() { bail!("адрес {raw:?}: укажите только схему и хост, без пути — /v1 и подобное добавляют клиенты"); }
    Ok(match u.port() { Some(p) => format!("{}://{host}:{p}", u.scheme()), None => format!("{}://{host}", u.scheme()) })
}

/// `storage.s3` → `STORAGE_S3` (the capability variable
/// prefix).
pub fn env_prefix(capability: &str) -> String {
    capability.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect()
}

fn shell_quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}

/// `${APP_DATA_DIR}/<path>` subdirectories from compose (without "file" paths with an extension in the last
/// segment).
pub fn data_subdirs(compose: &str) -> Vec<String> {
    let re = regex_lite::Regex::new(r"\$\{APP_DATA_DIR\}/([A-Za-z0-9_./-]+)").unwrap();
    let mut out: Vec<String> = re.captures_iter(compose).map(|c| c[1].trim_end_matches('/').to_string()).filter(|p| !p.contains("..") && !p.rsplit('/').next().unwrap_or("").contains('.')).collect();
    out.sort(); out.dedup(); out
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct AppOverrides {
    pub env: String,
    pub compose: String,
    #[serde(default)]
    pub effective_compose: String,
}

/// Deep merge of JSON objects (override over base); arrays are replaced whole.
pub fn merge_json(a: &mut serde_json::Value, b: &serde_json::Value) {
    match (a, b) {
        (serde_json::Value::Object(ao), serde_json::Value::Object(bo)) => {
            for (k, v) in bo {
                match ao.get_mut(k) {
                    Some(av) if av.is_object() && v.is_object() => merge_json(av, v),
                    _ => { ao.insert(k.clone(), v.clone()); }
                }
            }
        }
        (a, b) => *a = b.clone(),
    }
}

/// Manifest from the catalog plus a refinement of the `login` section by a `login.yaml` file next to it (a catalog
/// re-import leaves it alone).
pub fn load_manifest(p: &std::path::Path) -> Result<AppManifest> {
    let mut m: AppManifest = serde_yaml_ng::from_str(&std::fs::read_to_string(p)?).with_context(|| format!("манифест {}", p.display()))?;
    let lp = p.with_file_name("login.yaml");
    if lp.exists() {
        m.login = serde_yaml_ng::from_str(&std::fs::read_to_string(&lp)?).with_context(|| format!("{}", lp.display()))?;
    }
    Ok(m)
}

/// The "Recommended" list: `store/recommended.yaml` — the order sets the place on
/// the shelf.
#[derive(Debug, Default, serde::Deserialize)]
pub struct RecommendedList {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub items: Vec<RecommendedItem>,
}
#[derive(Debug, serde::Deserialize)]
pub struct RecommendedItem {
    pub app: String,
    #[serde(default)]
    pub reason: String,
    /// Reason translations: `i18n: { ru: { reason: … }
    /// }`.
    #[serde(default)]
    pub i18n: std::collections::BTreeMap<String, crate::model::I18nText>,
}

impl Service {
    /// App name → place and reason. A file parse error is a warning and an empty list; the catalog does not break because of
    /// it.
    pub fn recommended(&self) -> std::collections::BTreeMap<String, Recommended> {
        let p = self.cfg.store_dir().join("recommended.yaml");
        let Ok(text) = std::fs::read_to_string(&p) else { return Default::default() };
        match serde_yaml_ng::from_str::<RecommendedList>(&text) {
            Ok(l) => l.items.into_iter().enumerate().map(|(i, it)| (it.app, Recommended { rank: i as u32 + 1, reason: it.reason, i18n: it.i18n })).collect(),
            Err(e) => {
                tracing::warn!("{}: {e}", p.display());
                Default::default()
            }
        }
    }
}

impl Service {
    /// Alternative Entry Point addresses: from the configuration plus the IPv4 addresses edge_host resolves to on the server
    /// itself (the NetBird IP). Needed by devices where MagicDNS does not work: `https://<ip>/` opens the same Desktop with
    /// the same CA.
    pub fn edge_alt_hosts(&self) -> Vec<String> {
        use std::net::ToSocketAddrs;
        let mut out: Vec<String> = self.cfg.edge_alt_hosts.clone();
        if let Ok(addrs) = (self.cfg.edge_host.as_str(), 443u16).to_socket_addrs() {
            for a in addrs {
                if let std::net::IpAddr::V4(v4) = a.ip() {
                    if !v4.is_loopback() && !v4.is_unspecified() {
                        let s = v4.to_string();
                        if !out.contains(&s) { out.push(s); }
                    }
                }
            }
        }
        out
    }
}

pub const SHARED_NET: &str = "cloudos-shared";
/// Gate egress for external providers (a bridge with host NAT → host routes, incl. the tailnet); joined only for the
/// duration of grants with `type: external` and becomes the gate's default gateway.
pub const EGRESS_NET: &str = "cloudos-egress";

/// App address inside Cloud OS: the main service name resolves in its network (and in the shared one) for connected
/// containers.
pub fn internal_url(a: &AppState) -> String {
    format!("http://{}:{}", a.manifest.endpoint.service, a.manifest.endpoint.port)
}

/// Mix the provider networks and the shared network into compose, the link variables — as `${VAR}` in every service's
/// environment (values live in the stack env, the compose file holds no secrets). Services in `network_mode: host` and
/// without `networks` are left alone.
pub fn inject_links(compose: &mut serde_json::Value, app: &str, endpoint_service: &str, providers: &[(String, String)], shared: bool, env_keys: &[String]) {
    use serde_json::{json, Value};
    let Some(obj) = compose.as_object_mut() else { return };
    let mut nets = obj.get("networks").and_then(|n| n.as_object()).cloned().unwrap_or_default();
    let mut added: Vec<String> = Vec::new();
    for (p, net) in providers {
        let key = format!("link_{}", p.replace(['-', '.'], "_"));
        nets.insert(key.clone(), json!({ "external": true, "name": net }));
        added.push(key);
    }
    if shared {
        nets.insert("cloudos_shared".to_string(), json!({ "external": true, "name": SHARED_NET }));
    }
    obj.insert("networks".to_string(), Value::Object(nets));
    let Some(services) = obj.get_mut("services").and_then(|s| s.as_object_mut()) else { return };
    for (sname, svc) in services.iter_mut() {
        let Some(s) = svc.as_object_mut() else { continue };
        if s.get("network_mode").is_some() { continue; }
        let mut netmap: serde_json::Map<String, Value> = match s.get("networks") {
            Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).map(|n| (n.to_string(), json!({}))).collect(),
            Some(Value::Object(o)) => o.clone(),
            _ => continue,
        };
        for key in &added { netmap.entry(key.clone()).or_insert(json!({})); }
        if shared {
            let e = if sname == endpoint_service { json!({ "aliases": [format!("{app}.apps")] }) } else { json!({}) };
            netmap.entry("cloudos_shared".to_string()).or_insert(e);
        }
        s.insert("networks".to_string(), Value::Object(netmap));
        if env_keys.is_empty() { continue; }
        match s.get_mut("environment") {
            Some(Value::Array(a)) => {
                for k in env_keys {
                    if !a.iter().any(|x| x.as_str().map(|e| e.split('=').next() == Some(k.as_str())).unwrap_or(false)) {
                        a.push(Value::String(format!("{k}=${{{k}}}")));
                    }
                }
            }
            Some(Value::Object(o)) => { for k in env_keys { o.entry(k.clone()).or_insert(Value::String(format!("${{{k}}}"))); } }
            _ => { s.insert("environment".to_string(), json!(env_keys.iter().map(|k| format!("{k}=${{{k}}}")).collect::<Vec<_>>())); }
        }
    }
}

impl Service {
    /// Redeploy an installed app with the current compose and env (links, shared network, settings).
    pub(crate) async fn redeploy(&self, name: &str) -> Result<()> {
        let m = self.manifest(name)?;
        let Some(st) = self.store.get_app(name)? else { return Ok(()) };
        let env_pairs = self.app_env(&m, st.port)?;
        let env_text = env_pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
        let compose = self.compose_text(&m)?;
        for n in self.prepare_data_dirs(name, &compose, &env_pairs).await? { self.store.event("app", name, &format!("каталог данных {n}"))?; }
        if let (Some(id), Ok(Some(_))) = (&st.komodo_stack_id, self.komodo.get_stack(name).await) {
            self.komodo.update_stack(id, &compose, &env_text).await?;
            self.komodo.deploy(name).await?;
            self.spawn_watch(name);
            self.komodo.wait_running(name, 240).await?;
            if let Some(h) = &m.health { self.wait_health(&st, h, 90).await?; }
        }
        Ok(())
    }

    /// Link a consumer to a provider: the provider network in the consumer's compose, `<PROVIDER>_URL`, provider hooks (keys) — and a
    /// redeploy.
    pub async fn app_link(&self, consumer: &str, provider: &str) -> Result<AppView> {
        let _g = self.lock.lock().await;
        if consumer == provider { bail!("приложение нельзя подключить к самому себе"); }
        let Some(c) = self.store.get_app(consumer)? else { bail!("приложение {consumer} не установлено") };
        let Some(p) = self.store.get_app(provider)? else { bail!("провайдер {provider} не установлен") };
        if p.manifest.kind == AppKind::External || c.manifest.kind == AppKind::External {
            bail!("внешние провайдеры пока не участвуют в связях приложений: укажите их адрес и ключ в настройках потребителя");
        }
        let scope = format!("link:{consumer}:{provider}");
        let gate_url = internal_url(&p);
        if let Some(hook) = &p.manifest.hooks.on_grant {
            for cap in &p.manifest.provides {
                let out = self.run_hook_for(&p, hook, &format!("app-{consumer}"), cap, &gate_url).await.with_context(|| format!("on_grant {provider} для {consumer}"))?;
                for (k, v) in parse_env(&out) { self.store.put_secret(&scope, &k, &v)?; }
            }
        }
        self.store.put_link(&AppLink { consumer: consumer.into(), provider: provider.into(), created_at: store::now() })?;
        self.redeploy(consumer).await?;
        self.store.event("app", consumer, &format!("подключено к {provider}: сеть {provider}-net, {}_URL={gate_url}", env_prefix(provider)))?;
        self.store.event("app", provider, &format!("к нему подключено {consumer}"))?;
        self.app(consumer).await
    }

    pub async fn app_unlink(&self, consumer: &str, provider: &str) -> Result<AppView> {
        let _g = self.lock.lock().await;
        if let Some(p) = self.store.get_app(provider)? {
            if let Some(hook) = &p.manifest.hooks.on_revoke {
                for cap in &p.manifest.provides {
                    if let Err(e) = self.run_hook_for(&p, hook, &format!("app-{consumer}"), cap, &internal_url(&p)).await { tracing::warn!("on_revoke {provider} для {consumer}: {e:#}"); }
                }
            }
        }
        self.store.delete_secrets(&format!("link:{consumer}:{provider}"))?;
        self.store.delete_link(consumer, provider)?;
        self.redeploy(consumer).await?;
        self.store.event("app", consumer, &format!("отключено от {provider}"))?;
        self.app(consumer).await
    }

    /// Shared app network: a second network `cloudos-shared` and the name `<app>.apps` for
    /// everyone in it.
    pub async fn app_set_shared(&self, name: &str, shared: bool) -> Result<AppView> {
        let _g = self.lock.lock().await;
        self.manifest(name)?;
        if self.store.get_app(name)?.is_some_and(|a| a.manifest.kind == AppKind::External) { bail!("{name}: внешний сервис, в общую сеть приложений не входит"); }
        if shared {
            self.docker.ensure_bridge_network(SHARED_NET, HashMap::from([("cloudos.kind".into(), "shared".into())])).await?;
        }
        self.store.put_kv(&format!("app.shared.{name}"), &serde_json::Value::Bool(shared))?;
        self.redeploy(name).await?;
        self.store.event("app", name, if shared { "вошло в общую сеть приложений" } else { "вышло из общей сети приложений" })?;
        self.app(name).await
    }
}

impl Service {
    /// Rotate a generated app secret (password, token): a new value of the same shape, a stack redeploy. Works for
    /// apps that read env at start; those that stored the password at initialization must be changed inside them.
    pub async fn app_rotate_secret(&self, name: &str, key: &str) -> Result<AppView> {
        let _g = self.lock.lock().await;
        let m = self.manifest(name)?;
        let Some(EnvSpec::Generate { generate }) = m.env.get(key) else { bail!("{key}: сменить можно только сгенерированный секрет (generate в манифесте)") };
        let v = store::generate(*generate);
        self.store.put_secret(&format!("app:{name}"), key, &v)?;
        self.redeploy(name).await?;
        self.store.event("app", name, &format!("секрет {key} сменён, стек перевыкачен"))?;
        self.app(name).await
    }

    /// User note for the app (the account from the first-run wizard etc.) — stored encrypted.
    pub fn app_note(&self, name: &str) -> Result<String> {
        self.manifest(name)?;
        let t = self.store.get_secret(&format!("note:{name}"), "text")?.unwrap_or_default();
        if !t.is_empty() { self.store.event("app", name, "показана заметка")?; }
        Ok(t)
    }
    pub fn app_set_note(&self, name: &str, text: &str) -> Result<()> {
        self.manifest(name)?;
        let scope = format!("note:{name}");
        if text.trim().is_empty() { self.store.delete_secrets(&scope)?; } else { self.store.put_secret(&scope, "text", text)?; }
        Ok(())
    }
}

/// App data directory: create the missing levels with the default owner (1000:1000, see appdata.rs). The exact owner per
/// compose and image is set by `Service::prepare_data_dirs`; existing directories are left alone here.
pub fn ensure_data_dir(dir: &std::path::Path) -> Result<()> {
    let mut missing: Vec<std::path::PathBuf> = Vec::new();
    let mut p = dir.to_path_buf();
    while !p.exists() {
        missing.push(p.clone());
        match p.parent() { Some(par) => p = par.to_path_buf(), None => break }
    }
    std::fs::create_dir_all(dir)?;
    for d in missing {
        let _ = std::os::unix::fs::chown(&d, Some(crate::appdata::DEFAULT_OWNER.0), Some(crate::appdata::DEFAULT_OWNER.1));
    }
    Ok(())
}
