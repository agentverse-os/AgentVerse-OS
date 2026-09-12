//! Оркестрация (раздел 4 архитектуры): `cloudd` применяет `project.yaml` и манифесты идемпотентно,
//! удаление откатывает артефакты в обратном порядке.

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
    /// Один reconcile за раз: операции длинные и меняют общее состояние.
    pub(crate) lock: Arc<tokio::sync::Mutex<()>>,
    /// Один цикл бэкапа за раз и флаг «идёт» для UI.
    pub(crate) backup_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) backup_running: Arc<std::sync::atomic::AtomicBool>,
    /// Разобранный каталог (942 манифеста ≈ 1 с на разбор YAML) с отпечатком mtime файлов: перечитывается только после изменений.
    catalog_cache: Arc<std::sync::Mutex<Option<(u128, Vec<AppManifest>)>>>,
    /// Идёт переимпорт каталога из upstream (updates.rs).
    pub(crate) importing: Arc<std::sync::atomic::AtomicBool>,
}

impl Service {
    pub fn new(cfg: Config) -> Result<Self> {
        let store = Store::open(&cfg.db_path(), &cfg.age_key_path())?;
        let incus = Incus::new(cfg.incus_socket.clone());
        let docker = DockerRt::new(&cfg.docker_socket)?;
        let coder = Coder::new(&cfg.coder_url, &cfg.coder_token);
        let komodo = Komodo::new(&cfg.komodo_url, &cfg.komodo_key, &cfg.komodo_secret, &cfg.komodo_server);
        let edge = EdgeCaddy::new(&cfg.caddy_admin);
        Ok(Self { cfg: Arc::new(cfg), store, incus, docker, coder, komodo, edge, monitor: crate::monitor::Monitor::new(), feeds: crate::feeds::Feeds::default(), lock: Arc::new(tokio::sync::Mutex::new(())), backup_lock: Arc::new(tokio::sync::Mutex::new(())), backup_running: Arc::new(std::sync::atomic::AtomicBool::new(false)), catalog_cache: Default::default(), importing: Default::default() })
    }

    // =====================================================================
    // Store: чтение манифестов из репозитория
    // =====================================================================

    pub fn catalog(&self) -> Result<Vec<AppManifest>> {
        let dir = self.cfg.store_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        // отпечаток: mtime и размер всех manifest.yaml / login.yaml и recommended.yaml (≈1000 stat ≈ миллисекунды)
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
    /// Compose приложения с пользовательским override (`store/<app>/compose.override.yaml`, семантика docker-compose.override:
    /// глубокое слияние по сервисам). Override живёт отдельным файлом, поэтому переимпорт каталога его не трогает.
    pub(crate) fn compose_text(&self, m: &AppManifest) -> Result<String> {
        let dir = self.cfg.store_dir().join(&m.name);
        let base = std::fs::read_to_string(dir.join(&m.compose)).with_context(|| format!("compose {}", dir.join(&m.compose).display()))?;
        let ov = dir.join("compose.override.yaml");
        let ov_text = if ov.exists() { std::fs::read_to_string(&ov)? } else { String::new() };
        let has_links = self.is_shared(&m.name)? || self.store.list_links()?.iter().any(|l| l.consumer == m.name);
        if ov_text.trim().is_empty() && !has_links {
            return Ok(base); // без изменений — исходный текст каталога с комментариями
        }
        let mut a: serde_json::Value = serde_yaml_ng::from_str(&base).context("compose из каталога")?;
        if !ov_text.trim().is_empty() {
            let b: serde_json::Value = serde_yaml_ng::from_str(&ov_text).context("compose.override.yaml")?;
            merge_json(&mut a, &b);
        }
        self.inject_links(m, &mut a)?;
        Ok(serde_yaml_ng::to_string(&a)?)
    }

    /// Связи и общая сеть в compose потребителя: сети провайдеров как external, переменные адресов как `${VAR}` (значения — в env стека).
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

    /// Переменные от связей: `<ПРОВАЙДЕР>_URL` (адрес внутри Cloud OS) плюс вывод хуков провайдера, сохранённый при подключении.
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

    /// Пользовательские переопределения env (`store/<app>/user.env`, KEY=VALUE) — поверх манифеста и настроек.
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

    /// Сохранить переопределения и, если приложение установлено, перевыкатить стек.
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

    /// Применить `project.yaml`: сеть Incus + macvlan + gate; workspace в Coder; grants по capabilities.
    pub async fn project_apply(&self, spec: ProjectSpec) -> Result<ProjectView> {
        let _g = self.lock.lock().await;
        validate_name(&spec.project)?;
        let base = &self.cfg.project_subnet_base;
        let now = store::now();
        let previous = self.store.get_project(&spec.project)?;
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

        // 1. сеть проекта (Incus) с DNS *.gate → gate
        let gw = st.gateway(base);
        let gate_ip = st.gate_ip(base);
        let dhcp = format!("{base}.{}.2-{base}.{}.239", st.subnet_index, st.subnet_index);
        self.incus.ensure_network(&st.incus_network(), &format!("{gw}/24"), &dhcp).await?;
        self.incus_set_gate_dns(&st.incus_network(), &gate_ip).await?;
        // 2. macvlan Docker на мосту проекта + gate
        self.docker.ensure_macvlan(&st.macvlan_network(), &st.incus_network(), &st.subnet(base), &gw, &st.gate_ip_range(base)).await?;
        let gate_dir = self.cfg.gates_dir().join(&st.name);
        self.render_gate(&st).await?;
        self.docker.ensure_gate(&st.gate_name(), &st.macvlan_network(), &gate_ip, &gate_dir).await?;
        if let Err(e) = self.docker.reload_gate(&st.gate_name()).await {
            tracing::warn!("reload gate {}: {e}", st.gate_name());
        }
        self.store.event("project", &st.name, "сеть и gate готовы")?;

        // 3. workspace через Coder
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
                        // новые cpu/memory применяются сборкой start с параметрами (Incus обновит лимиты in-place)
                        let params = self.rich_params(&st);
                        let b = self.coder.build_with_params(ws.id, "start", &params).await?;
                        self.store.event("workspace", &st.name, &format!("параметры изменены: {} CPU, {} GiB — build {}", st.spec.workspace.cpu, st.spec.workspace.memory, b.id))?;
                        // инстанс не перезагружается, а токен агента у новой сборки другой: перезапускаем юнит агента,
                        // он перечитает user.coder_agent_token через guest API
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
                }
            }
            self.store.upsert_project(&st)?;
        }

        // 4. grants по capabilities (что можем — выдаём; чего нет — в missing)
        let apps = self.store.list_apps()?;
        let current = self.store.list_grants(Some(&st.name))?;
        for cap in &spec.capabilities {
            if current.iter().any(|g| &g.capability == cap) {
                continue;
            }
            if let Some(app) = apps.iter().find(|a| a.manifest.provides.contains(cap)) {
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
        // dnsmasq сети проекта отвечает на *.gate адресом gate — так workspace видит http://s3.gate без /etc/hosts
        let v: serde_json::Value = serde_json::json!({ "raw.dnsmasq": format!("address=/.gate/{gate_ip}") });
        self.incus.patch_network(network, v).await
    }

    async fn render_gate(&self, st: &ProjectState) -> Result<()> {
        let grants = self.store.list_grants(Some(&st.name))?;
        let apps = self.store.list_apps()?;
        let routes = gate::routes_for(&grants, &apps);
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
        // обратный порядок: revoke → workspace → gate → сети
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
        let b = self.coder.build(id, transition).await?;
        self.store.event("workspace", name, &format!("{transition}: build {}", b.id))?;
        self.project_view(&st).await
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
        })
    }

    // =====================================================================
    // Apps (App Runtime = Komodo)
    // =====================================================================

    pub async fn apps(&self) -> Result<Vec<AppView>> {
        let installed = self.store.list_apps()?;
        let grants = self.store.list_grants(None)?;
        // состояния стеков — одним запросом к Komodo, а не по запросу на приложение
        let states = if installed.is_empty() { Default::default() } else { self.komodo.states().await.unwrap_or_default() };
        let mut out = Vec::new();
        for m in self.catalog()? {
            let inst = installed.iter().find(|a| a.name == m.name);
            let state = inst.and_then(|_| states.get(&m.name).cloned());
            out.push(self.app_view_with(&m, inst, &grants, state)?);
        }
        Ok(out)
    }

    /// Одно приложение (карточка, 🔑 в окне) — без обхода всего каталога.
    pub async fn app(&self, name: &str) -> Result<AppView> {
        let m = self.manifest(name)?;
        let inst = self.store.get_app(name)?;
        let grants = self.store.list_grants(None)?;
        self.app_view(&m, inst.as_ref(), &grants).await
    }

    async fn app_view(&self, m: &AppManifest, inst: Option<&AppState>, grants: &[Grant]) -> Result<AppView> {
        let state = match inst {
            Some(_) => self.komodo.status(&m.name).await.ok().flatten().map(|s| s.state),
            None => None,
        };
        self.app_view_with(m, inst, grants, state)
    }

    fn app_view_with(&self, m: &AppManifest, inst: Option<&AppState>, grants: &[Grant], state: Option<String>) -> Result<AppView> {
        let mut manifest = inst.map(|a| a.manifest.clone()).unwrap_or(m.clone());
        // режим открытия и данные для входа — актуальные из каталога; поверх режима — выбор пользователя (kv app.open.<name>)
        manifest.route.open = m.route.open;
        manifest.login = m.login.clone();
        manifest.recommended = m.recommended.clone();
        if let Some(v) = self.store.get_kv(&format!("app.open.{}", m.name))?.and_then(|v| v.as_str().map(|s| s.to_string())) {
            manifest.route.open = if v == "newtab" { OpenMode::Newtab } else { OpenMode::Iframe };
        }
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
            internal_url: inst.map(internal_url),
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

    fn app_url(&self, a: &AppState) -> String {
        match a.manifest.route.mode {
            RouteMode::Path => format!("https://{}{}", self.cfg.edge_host, a.manifest.route.prefix.clone().unwrap_or_else(|| format!("/apps/{}/", a.name))),
            RouteMode::Port => format!("https://{}:{}/", self.cfg.edge_host, a.port.unwrap_or(0)),
            RouteMode::Host => format!("https://{}.{}/", a.name, self.cfg.edge_host),
        }
    }

    /// install(manifest, env): сеть ядра → env из секрет-стора → CreateStack/UpdateStack + DeployStack → маршрут edge.
    pub async fn app_install(&self, name: &str) -> Result<AppView> {
        let _g = self.lock.lock().await;
        let m = self.manifest(name)?;
        for req in &m.requires {
            let apps = self.store.list_apps()?;
            if !apps.iter().any(|a| a.manifest.provides.contains(req)) {
                bail!("{name} требует capability {req}: установите приложение, которое её provides");
            }
        }
        let now = store::now();
        let mut st = self.store.get_app(name)?.unwrap_or(AppState { name: name.to_string(), manifest: m.clone(), port: None, komodo_stack_id: None, installed_at: now });
        st.manifest = m.clone();
        if m.route.mode == RouteMode::Port && st.port.is_none() {
            st.port = Some(self.alloc_port(m.route.port)?);
        }
        // 1. сеть приложения — создаёт ядро (ADR-005 решение 3)
        let net = st.network();
        self.docker.ensure_bridge_network(&net, HashMap::from([("cloudos.app".into(), name.to_string())])).await?;
        // 1b. read-only docker-прокси: сеть создаём (сам прокси — из bootstrap/docker-proxy)
        if self.compose_text(&m)?.contains(crate::importer::DOCKER_RO_NET) {
            self.docker.ensure_bridge_network(crate::importer::DOCKER_RO_NET, HashMap::from([("cloudos.kind".into(), "docker-ro".into())])).await?;
            if !self.docker.container_exists("cloudos-docker-ro").await? {
                self.store.event("app", name, "ВНИМАНИЕ: прокси cloudos-docker-ro не запущен (bootstrap/docker-proxy) — доступ к Docker у приложения не будет")?;
            }
        }
        // 2. данные приложения
        let data_dir = self.cfg.apps_data_dir.join(name);
        ensure_data_dir(&data_dir)?;
        // 3. env: generated → секрет-стор (один раз), literal — как есть
        let env_pairs = self.app_env(&m, st.port)?;
        let env_text = env_pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
        let compose = self.compose_text(&m)?;
        // каталоги под bind-тома (${APP_DATA_DIR}/…) должны существовать до `compose up` (Docker не создаёт device для driver_opts bind)
        // и принадлежать пользователю контейнера (appdata.rs: user:/PUID/USER образа, иначе 1000:1000 как в Umbrel/Runtipi)
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
        // Komodo считает стек running, даже если контейнер в crash-loop — проверяем health из манифеста с хоста
        if let Some(h) = &m.health {
            self.wait_health(&st, h, 90).await.with_context(|| format!("{name}: health {h} не прошёл; логи: cloudd app logs {name}"))?;
            self.store.event("app", name, "health OK")?;
        }
        // 5. маршрут edge: edge-Caddy подключается к сети приложения и перечитывает конфиг
        self.docker.connect(&net, &self.cfg.edge_container).await?;
        self.reload_edge().await?;
        self.store.event("app", name, &format!("маршрут {}", self.app_url(&st)))?;
        // 6. если проекты ждали эту capability — выдать
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

    /// uninstall: revoke у проектов → маршрут edge → DestroyStack/DeleteStack → сеть → запись (данные в tank/apps остаются).
    /// Удалить приложение. `purge` — вместе с данными в tank/apps, секретами, заметками и переопределениями;
    /// иначе всё это сохраняется и переустановка возвращает приложение с теми же паролями.
    pub async fn app_remove(&self, name: &str, purge: bool) -> Result<()> {
        let _g = self.lock.lock().await;
        let Some(st) = self.store.get_app(name)? else { bail!("приложение {name} не установлено") };
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
        if self.backup_config().enabled {
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
        self.docker.disconnect(&st.network(), &self.cfg.edge_container).await.ok();
        if self.komodo.get_stack(name).await?.is_some() {
            self.komodo.destroy(name).await?;
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if let Some(id) = &st.komodo_stack_id {
                self.komodo.delete_stack(id).await?;
            }
        }
        // сеть могла исчезнуть при прерванной прошлой попытке — удаление должно доводиться до конца
        if let Err(e) = self.docker.remove_network(&st.network()).await {
            tracing::warn!("сеть {}: {e:#}", st.network());
        }
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

    /// Как открывать приложение в Desktop: в окне (iframe) или в новой вкладке — выбор пользователя поверх манифеста.
    pub fn app_set_open(&self, name: &str, open: &str) -> Result<()> {
        self.manifest(name)?;
        if !matches!(open, "iframe" | "newtab") { bail!("имя open: iframe | newtab"); }
        self.store.put_kv(&format!("app.open.{name}"), &serde_json::Value::String(open.into()))
    }

    /// Настройки приложения (form_fields каталога): сохранить значения и, если установлено, перевыкатить стек.
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
        self.komodo.logs(name, &[st.manifest.endpoint.service.clone()], tail).await
    }

    /// health из манифеста: `http://<service>:<port>/path` → тот же URL по IP контейнера в сети приложения (хост видит bridge напрямую).
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
        let used = self.store.used_ports()?;
        let (lo, hi) = self.cfg.app_port_range;
        if let Some(p) = wanted {
            if !used.contains(&p) && p != self.cfg.coder_port {
                return Ok(p);
            }
        }
        (lo..=hi).find(|p| !used.contains(p) && *p != self.cfg.coder_port).ok_or_else(|| anyhow!("нет свободных портов для route: port"))
    }

    // =====================================================================
    // Grants
    // =====================================================================

    pub async fn grant(&self, project: &str, capability: &str) -> Result<ProjectView> {
        let _g = self.lock.lock().await;
        let mut st = self.store.get_project(project)?.ok_or_else(|| anyhow!("проект {project} не найден"))?;
        let apps = self.store.list_apps()?;
        let app = apps.iter().find(|a| a.manifest.provides.contains(&capability.to_string())).ok_or_else(|| anyhow!("нет установленного приложения, которое provides {capability}"))?;
        self.grant_inner(&st, capability, app).await?;
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

    /// grant = gate подключается к `<app>-net` + маршрут `<cap>.gate` (без рестартов — ADR-005) + хук provider'а `on_grant`,
    /// результат которого (KEY=VALUE) становится секретами проекта и env-контрактом capability в workspace (2.5).
    async fn grant_inner(&self, st: &ProjectState, capability: &str, app: &AppState) -> Result<()> {
        self.docker.connect(&app.network(), &st.gate_name()).await?;
        let scope = format!("project:{}:{capability}", st.name);
        if let Some(hook) = &app.manifest.hooks.on_grant {
            let out = self.run_hook(app, hook, st, capability).await.with_context(|| format!("on_grant {} для {}", app.name, st.name))?;
            for (k, v) in parse_env(&out) {
                self.store.put_secret(&scope, &k, &v)?;
            }
        }
        // адрес capability на gate — всегда, даже без хука
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
            // отключать сеть можно только если другие grants этого проекта не идут через то же приложение
            if !grants.iter().any(|o| o.capability != capability && o.app == g.app) {
                self.docker.disconnect(&app.network(), &st.gate_name()).await?;
            }
        }
        self.store.delete_secrets(&format!("project:{}:{capability}", st.name))?;
        self.store.delete_grant(&st.name, capability)?;
        self.store.event("revoke", &st.name, &format!("{capability} снят"))?;
        self.push_workspace_env(st).await;
        Ok(())
    }

    /// Хук provider'а: скрипт из каталога приложения в Store, выполняется на хосте (cloudd видит сети приложений напрямую)
    /// с env приложения (в т.ч. сгенерированные секреты), PROJECT, CAPABILITY, APP_IP главного сервиса. stdout → KEY=VALUE.
    async fn run_hook(&self, app: &AppState, hook: &str, st: &ProjectState, capability: &str) -> Result<String> {
        self.run_hook_for(app, hook, &st.name, capability, &format!("http://{capability}.gate")).await
    }

    /// Та же механика для связей приложений: PROJECT=`app-<потребитель>`, GATE_URL — адрес провайдера внутри Cloud OS.
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
        let ip = self.docker.container_ip(&format!("{}-{}-1", app.name, app.manifest.endpoint.service), &app.network()).await?;
        env.push(("APP_IP".into(), ip));
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

    /// Env приложения для стека и хуков: служебные переменные + манифест (generated — из секрет-стора).
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
        // соглашения каталогов Runtipi/Coolify, чтобы их compose работал без правок
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
                // literal можно переопределить настройкой (хранится в секрет-сторе приложения)
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
        // связи: адрес провайдера всегда, ключи из хуков — только если у приложения значение пустое (свои настройки важнее)
        for (k, v) in self.link_env(m)? {
            match env.iter_mut().find(|(ek, _)| *ek == k) {
                Some(e) if e.1.is_empty() || k.ends_with("_URL") => e.1 = v,
                Some(_) => {}
                None => env.push((k, v)),
            }
        }
        // пользовательские переопределения (user.env) — поверх всего
        for (k, v) in self.user_env(&m.name) {
            if let Some(e) = env.iter_mut().find(|(ek, _)| *ek == k) { e.1 = v; } else { env.push((k, v)); }
        }
        // ссылки вида ${OTHER} внутри значений (каталоги Umbrel/Coolify): compose значения из .env не раскрывает — делаем сами
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

    /// Env-контракт capabilities в workspace: /etc/cloudos/env.sh (source из /etc/bash.bashrc) — переживает stop/start (rootfs).
    async fn push_workspace_env(&self, st: &ProjectState) {
        let inst = format!("ws-admin-{}", st.name); // TODO: owner из Coder, когда пользователей станет больше одного
        let inst = match self.coder_owner().await { Some(o) => format!("ws-{o}-{}", st.name), None => inst };
        if !self.incus.instance_running(&inst).await.unwrap_or(false) {
            return;
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
        if let Err(e) = self.incus.file_push(&inst, "/etc/cloudos/env.sh", content.as_bytes(), 0o644).await {
            tracing::warn!("env в {inst}: {e}");
            return;
        }
        let _ = self.incus.exec(&inst, &["sh", "-c", "grep -q cloudos/env.sh /etc/bash.bashrc || echo '[ -r /etc/cloudos/env.sh ] && . /etc/cloudos/env.sh' >> /etc/bash.bashrc"]).await;
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
        let cfg = EdgeCaddy::render(&EdgeInput {
            host: &self.cfg.edge_host,
            alt_hosts: &alt,
            cloudd_upstream: &self.cfg.edge_cloudd_upstream,
            coder_upstream: &self.cfg.edge_coder_upstream,
            coder_port: self.cfg.coder_port,
            apps: &apps,
            tls_internal: self.cfg.tls_internal,
        });
        self.edge.load(&cfg).await
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
        Ok(SystemStatus { edge_host: self.cfg.edge_host.clone(), edge_alt_hosts: self.edge_alt_hosts(), coder_url: self.cfg.coder_public_url(), coder_user: self.cfg.admin_email.clone(), coder_password_set: self.cfg.admin_password.is_some(), components: c, version: crate::updates::VERSION.into(), build: crate::updates::build_id().into(), setup_done: self.setup_done(), update_available, app_updates })
    }

    /// Запустить фоновый сборщик метрик (только в режиме serve).
    pub fn start_monitor(&self) {
        let store = self.store.clone();
        self.monitor.clone().spawn(self.docker.clone(), self.incus.clone(), move || store.list_apps().map(|a| a.into_iter().map(|x| x.name).collect()).unwrap_or_default());
    }

    /// Reconcile всего: перечитать репозиторий и применить каждый проект; поднять edge.
    pub async fn reconcile_all(&self) -> Result<()> {
        // edge мог быть пересоздан (новый образ, новые тома): подключения к сетям приложений при этом теряются — восстанавливаем
        for st in self.store.list_apps()? {
            if let Err(e) = self.docker.connect(&st.network(), &self.cfg.edge_container).await {
                let m = format!("{e:#}");
                if !m.contains("already") { tracing::warn!("reconcile: edge → сеть {}: {m}", st.network()); }
            }
        }
        self.reload_edge().await?;
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

pub fn validate_name(n: &str) -> Result<()> {
    if n.is_empty() || n.len() > 48 || !n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || n.starts_with('-') {
        bail!("имя {n:?}: только [a-z0-9-], до 48 символов");
    }
    Ok(())
}

/// KEY=VALUE построчно; строки без `=` и комментарии пропускаются.
pub fn parse_env(out: &str) -> Vec<(String, String)> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string())))
        .filter(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .collect()
}

/// `storage.s3` → `STORAGE_S3` (префикс переменных capability).
pub fn env_prefix(capability: &str) -> String {
    capability.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect()
}

fn shell_quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}

/// Подкаталоги `${APP_DATA_DIR}/<путь>` из compose (без «файловых» путей с расширением в последнем сегменте).
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

/// Глубокое слияние JSON-объектов (override поверх base); массивы заменяются целиком.
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

/// Манифест из каталога плюс уточнение раздела `login` файлом `login.yaml` рядом (переимпорт каталога его не трогает).
pub fn load_manifest(p: &std::path::Path) -> Result<AppManifest> {
    let mut m: AppManifest = serde_yaml_ng::from_str(&std::fs::read_to_string(p)?).with_context(|| format!("манифест {}", p.display()))?;
    let lp = p.with_file_name("login.yaml");
    if lp.exists() {
        m.login = serde_yaml_ng::from_str(&std::fs::read_to_string(&lp)?).with_context(|| format!("{}", lp.display()))?;
    }
    Ok(m)
}

/// Список «Рекомендуем»: `store/recommended.yaml` — порядок задаёт место на полке.
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
    /// Переводы причины: `i18n: { ru: { reason: … } }`.
    #[serde(default)]
    pub i18n: std::collections::BTreeMap<String, crate::model::I18nText>,
}

impl Service {
    /// Имя приложения → место и причина. Ошибка разбора файла — предупреждение и пустой список, каталог от этого не ломается.
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
    /// Альтернативные адреса Entry Point: из конфигурации плюс IPv4-адреса, в которые резолвится edge_host на самом сервере
    /// (NetBird-IP). Нужны устройствам, у которых MagicDNS не работает: `https://<ip>/` открывает тот же Desktop с тем же CA.
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

/// Адрес приложения внутри Cloud OS: имя главного сервиса резолвится в его сети (и в общей) для подключённых контейнеров.
pub fn internal_url(a: &AppState) -> String {
    format!("http://{}:{}", a.manifest.endpoint.service, a.manifest.endpoint.port)
}

/// Подмешать в compose сети провайдеров и общую сеть, переменные связей — как `${VAR}` в environment каждого сервиса
/// (значения живут в env стека, в файле compose секретов нет). Сервисы в `network_mode: host` и без `networks` не трогаем.
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
    /// Перевыкатить установленное приложение с актуальными compose и env (связи, общая сеть, настройки).
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

    /// Подключить потребителя к провайдеру: сеть провайдера в compose потребителя, `<ПРОВАЙДЕР>_URL`, хуки провайдера (ключи) — и перевыкатка.
    pub async fn app_link(&self, consumer: &str, provider: &str) -> Result<AppView> {
        let _g = self.lock.lock().await;
        if consumer == provider { bail!("приложение нельзя подключить к самому себе"); }
        let Some(_c) = self.store.get_app(consumer)? else { bail!("приложение {consumer} не установлено") };
        let Some(p) = self.store.get_app(provider)? else { bail!("провайдер {provider} не установлен") };
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

    /// Общая сеть приложений: вторая сеть `cloudos-shared` и имя `<app>.apps` для всех, кто в ней.
    pub async fn app_set_shared(&self, name: &str, shared: bool) -> Result<AppView> {
        let _g = self.lock.lock().await;
        self.manifest(name)?;
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
    /// Сменить сгенерированный секрет приложения (пароль, токен): новое значение той же формы, перевыкатка стека.
    /// Работает для приложений, читающих env при старте; сохранившие пароль при инициализации нужно менять внутри них.
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

    /// Заметка пользователя к приложению (учётка из мастера первого запуска и т.п.) — хранится зашифрованной.
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

/// Каталог данных приложения: создать недостающие уровни с владельцем по умолчанию (1000:1000, см. appdata.rs). Точный владелец
/// по compose и образу выставляет `Service::prepare_data_dirs`; существующие каталоги здесь не трогаем.
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
