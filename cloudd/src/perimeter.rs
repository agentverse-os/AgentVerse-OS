//! Perimeter, stage 1 (ADR-010): closing the ways around the front door.
//!
//! * The edge reaches the core and the "Agents" service through unix sockets in a directory shared with its container, and its
//!   own admin API listens on a socket there too — nothing is left on the docker bridge for app containers to call.
//! * The control plane behind the edge refuses containers that share a docker network with it (`guard_ranges`).
//! * The host firewall keeps workspaces and containers away from the control plane's published ports and workspaces away from
//!   the host's own services (`firewall_*`).
//!
//! An edge container created before this (no shared directory) keeps working the old way — the core listens on the bridge
//! only while such a container is there — until `cloudd edge-migrate` recreates it.

use crate::docker::ContainerFacts;
use crate::service::Service;
use anyhow::{anyhow, bail, Context, Result};
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

pub struct PerimeterState {
    /// Should the core listen on the docker bridge — is an edge container without the socket directory running? The listener
    /// lives in `serve` and follows this value.
    pub bridge: tokio::sync::watch::Sender<bool>,
    /// What the last look at the edge container found: Some(true) — it mounts the socket directory.
    sockets: std::sync::Mutex<Option<bool>>,
    /// The firewall was already reported as unavailable (no iptables, not root): say it once.
    fw_skipped: std::sync::atomic::AtomicBool,
    /// One look at a time: the first one installs the "Agents" unit, and the start asks from several places at once.
    sync_lock: tokio::sync::Mutex<()>,
    /// One firewall pass at a time (the minute watch and an edge reload can meet).
    fw_lock: tokio::sync::Mutex<()>,
    /// The guard the edge was last loaded with. Docker can move the container's default gateway to another of its networks
    /// (a network is attached or removed): visitors then arrive from an address the loaded guard refuses, so the edge watch
    /// compares and reloads.
    guard: std::sync::Mutex<Option<(Vec<String>, Vec<String>)>>,
}

impl Default for PerimeterState {
    fn default() -> Self { Self { bridge: tokio::sync::watch::channel(false).0, sockets: Default::default(), fw_skipped: Default::default(), sync_lock: Default::default(), fw_lock: Default::default(), guard: Default::default() } }
}

impl PerimeterState {
    /// What the last look at the edge container found (None — not looked yet).
    pub fn sockets(&self) -> Option<bool> { *self.sockets.lock().unwrap() }
    /// Remember the guard a configuration was loaded with.
    pub fn guard_loaded(&self, deny: &[String], allow: &[String]) { *self.guard.lock().unwrap() = Some((deny.to_vec(), allow.to_vec())); }
    /// Does the guard the edge runs with differ from the one it should have now? Before the first load — no.
    pub fn guard_stale(&self, deny: &[String], allow: &[String]) -> bool {
        self.guard.lock().unwrap().as_ref().map(|(d, a)| d.as_slice() != deny || a.as_slice() != allow).unwrap_or(false)
    }
}

/// Does the container mount the socket directory at the same path?
pub fn socket_mounted(f: &ContainerFacts, run_dir: &Path) -> bool {
    let d = run_dir.to_string_lossy();
    let d = d.trim_end_matches('/');
    f.mounts.iter().any(|(src, dst)| src.trim_end_matches('/') == d && dst.trim_end_matches('/') == d)
}

/// `10.200.9.4` with prefix 24 → `10.200.9.0/24`.
pub fn subnet_of(ip: &str, prefix: u8) -> Option<String> {
    let ip: Ipv4Addr = ip.parse().ok()?;
    if prefix > 32 { return None }
    let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
    Some(format!("{}/{prefix}", Ipv4Addr::from(u32::from(ip) & mask)))
}

/// The default gateway of a network namespace from its `/proc/<pid>/net/route`: the line with destination and mask zero; the
/// gateway is printed as the address bytes read as a little-endian number (`0101C80A` is 10.200.1.1).
pub fn default_gateway(route_table: &str) -> Option<Ipv4Addr> {
    route_table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 8 || f[1] != "00000000" || f[7] != "00000000" { return None }
        let gw = u32::from_str_radix(f[2], 16).ok().filter(|g| *g != 0)?;
        Some(Ipv4Addr::from(gw.to_le_bytes()))
    })
}

/// The control-plane guard for the edge render: (subnets to refuse, addresses in them to let through).
///
/// Every docker network the edge is attached to is shared with an app's containers, and from there they reach the edge's
/// listeners directly, past the published ports. Connections to the published ports arrive from one address in those
/// subnets — the gateway of the edge's default route, after NAT — and that one is let through. If the gateway is unknown or
/// lies outside the subnets, no guard is built at all: refusing the subnets then would refuse every visitor.
pub fn guard_ranges(f: &ContainerFacts, gateway: Option<Ipv4Addr>) -> (Vec<String>, Vec<String>) {
    let Some(gw) = gateway else { return (vec![], vec![]) };
    let mut deny: Vec<String> = f.networks.iter().filter_map(|(_, ip, prefix)| subnet_of(ip, *prefix)).collect();
    deny.sort(); deny.dedup();
    let inside = f.networks.iter().any(|(_, ip, prefix)| subnet_of(ip, *prefix) == subnet_of(&gw.to_string(), *prefix));
    if deny.is_empty() || !inside { return (vec![], vec![]) }
    (deny, vec![format!("{gw}/32")])
}

// ---------------------------------------------------------------------------------------------------------------------
// Host firewall

/// Comment on every rule of ours: how they are told from everyone else's.
pub const FW_TAG: &str = "cloudos-perimeter";
/// Forwarded packets from a workspace or a container that were addressed to a published control-plane port.
pub const CH_FWD: &str = "CLOUDOS-CTL-FWD";
/// The same ports reached through the host's own listener (docker-proxy) from a docker bridge.
pub const CH_IN: &str = "CLOUDOS-CTL-IN";
/// Everything a workspace sends to the host itself.
pub const CH_WS: &str = "CLOUDOS-WS-IN";

fn strs(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

/// Content of our chains for the control-plane ports (Desktop with the API and "Agents"), in order. The first rule of each
/// chain carries the version of the whole set in its comment: a chain with another version is refilled.
pub fn firewall_chains(ports: &[u16]) -> Vec<(&'static str, Vec<Vec<String>>)> {
    let list = ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",");
    let mut fwd = Vec::new();
    for p in ports {
        // a connection that came through a published port is DNAT'ed; its original destination port is the published one —
        // connections to anywhere else on port 443 (the internet) are not touched. Only the direction it was opened in:
        // conntrack matches describe the connection, not the packet, and without `--ctdir ORIGINAL` the edge's own answers
        // to every visitor of port 443 (they leave through a docker bridge too) would be rejected as well.
        fwd.push(strs(&["-p", "tcp", "-m", "conntrack", "--ctstate", "DNAT", "--ctorigdstport", &p.to_string(), "--ctdir", "ORIGINAL", "-j", "REJECT", "--reject-with", "tcp-reset"]));
    }
    let ctl_in = vec![strs(&["-p", "tcp", "-m", "multiport", "--dports", &list, "-j", "REJECT", "--reject-with", "tcp-reset"])];
    let ws = vec![
        strs(&["-m", "conntrack", "--ctstate", "RELATED,ESTABLISHED", "-j", "ACCEPT"]),
        strs(&["-p", "udp", "-m", "multiport", "--dports", "53,67", "-j", "ACCEPT"]),   // the bridge's DNS and DHCP
        strs(&["-p", "tcp", "-m", "tcp", "--dport", "53", "-j", "ACCEPT"]),
        strs(&["-p", "icmp", "-j", "ACCEPT"]),
        strs(&["-j", "REJECT", "--reject-with", "icmp-port-unreachable"]),
    ];
    let version = format!("{FW_TAG}:v2:{list}");
    [(CH_FWD, fwd), (CH_IN, ctl_in), (CH_WS, ws)].into_iter().map(|(name, mut rules)| {
        if let Some(first) = rules.first_mut() { let mut r = strs(&["-m", "comment", "--comment", &version]); r.append(first); *first = r; }
        (name, rules)
    }).collect()
}

/// Jumps into our chains: (parent chain, input interface, our chain). Project bridges are `net-<project>` by our own naming;
/// docker bridges are listed by name — a wildcard like `br-+` would also catch a LAN bridge of the host.
pub fn firewall_jumps(docker_bridges: &[String]) -> Vec<(&'static str, String, &'static str)> {
    let mut out = vec![("DOCKER-USER", "net-+".to_string(), CH_FWD), ("INPUT", "net-+".to_string(), CH_WS)];
    for b in docker_bridges {
        out.push(("DOCKER-USER", b.clone(), CH_FWD));
        out.push(("INPUT", b.clone(), CH_IN));
    }
    out
}

fn jump_spec(iface: &str, target: &str) -> Vec<String> { strs(&["-i", iface, "-m", "comment", "--comment", FW_TAG, "-j", target]) }

/// Our jumps found in `iptables -S <chain>` output, as (position among the chain's rules, interface, target), and the
/// position of the rule that lets everything from the project bridges through (`-i net-+ -j ACCEPT`, set by the host
/// script): a jump below it never sees a packet.
pub fn parse_jumps(listing: &str) -> (Vec<(usize, String, String)>, Option<usize>) {
    let mut ours = Vec::new();
    let mut accept_all = None;
    for (n, line) in listing.lines().filter(|l| l.starts_with("-A ")).enumerate() {
        let t: Vec<&str> = line.split_whitespace().collect();
        let val = |flag: &str| t.iter().position(|x| *x == flag).and_then(|i| t.get(i + 1)).map(|s| s.trim_matches('"').to_string());
        if val("--comment").as_deref() == Some(FW_TAG) {
            if let (Some(i), Some(j)) = (val("-i"), val("-j")) { ours.push((n, i, j)); }
        } else if t.len() == 6 && val("-i").as_deref() == Some("net-+") && val("-j").as_deref() == Some("ACCEPT") && accept_all.is_none() {
            accept_all = Some(n);
        }
    }
    (ours, accept_all)
}

/// Docker bridge interface names from the network list: `com.docker.network.bridge.name` or `br-` plus twelve characters
/// of the network id.
pub fn bridge_names(nets: &[(String, String, Option<String>)]) -> Vec<String> {
    let mut v: Vec<String> = nets.iter().filter(|(_, driver, _)| driver == "bridge")
        .filter_map(|(id, _, name)| name.clone().filter(|n| !n.is_empty()).or_else(|| (id.len() >= 12).then(|| format!("br-{}", &id[..12])))).collect();
    v.sort(); v.dedup();
    v
}

async fn ipt(args: &[String]) -> Result<std::process::Output> {
    let mut a = vec!["-w".to_string(), "5".to_string()];
    a.extend_from_slice(args);
    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
    crate::setup::run("iptables", &refs, Duration::from_secs(20)).await
}
async fn ipt_ok(args: &[String]) -> Result<()> {
    let o = ipt(args).await?;
    if !o.status.success() { bail!("iptables {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()) }
    Ok(())
}
async fn ipt_list(chain: &str) -> Option<String> {
    let o = ipt(&strs(&["-S", chain])).await.ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

// ---------------------------------------------------------------------------------------------------------------------
// Moving an old edge container to the sockets

/// The starter Caddyfile of the edge: nothing but the admin API, on a socket in the shared directory.
pub fn bootstrap_caddyfile(run_dir: &Path) -> String {
    format!("# The starting configuration of edge: only the Admin API — on a socket in a directory shared with the host (ADR-010).\n# Everything else is loaded by cloudd (POST /load).\n{{\n  admin unix/{}/admin.sock\n  persist_config off\n}}\n", run_dir.display())
}

/// Our edge compose file with the socket directory mounted and without the published admin port. None — nothing to change;
/// an error — the file does not look like ours and is left alone.
pub fn patch_edge_compose(text: &str, run_dir: &Path) -> Result<Option<String>> {
    let d = run_dir.display().to_string();
    let mount = format!("{d}:{d}");
    let has_mount = text.lines().any(|l| l.trim_start().starts_with('-') && l.contains(&mount));
    let has_admin_port = text.lines().any(|l| l.trim_start().starts_with('-') && l.contains("2019:2019"));
    if has_mount && !has_admin_port { return Ok(None) }
    if !text.contains("container_name: cloudos-edge") && !text.contains("caddy-data:/data") { bail!("compose-файл edge не похож на наш (нет caddy-data:/data)") }
    let mut out = Vec::new();
    let mut mounted = has_mount;
    for l in text.lines() {
        if l.trim_start().starts_with('-') && l.contains("2019:2019") { continue }   // the admin API is no longer published
        out.push(l.to_string());
        // right after the last of our volumes: the Tailscale socket directory, or the Caddy config volume in older files
        if !mounted && l.trim_start().starts_with('-') && (l.contains("/var/run/tailscale:/var/run/tailscale") || (l.contains("caddy-config:/config") && !text.contains("/var/run/tailscale:/var/run/tailscale"))) {
            let indent: String = l.chars().take_while(|c| c.is_whitespace()).collect();
            out.push(format!("{indent}- {mount}   # sockets shared with the host: the Caddy admin API and the cloudd / \"Agents\" upstreams (ADR-010)"));
            mounted = true;
        }
    }
    if !mounted { bail!("в compose-файле edge не найден список томов сервиса — каталог сокетов не добавлен") }
    Ok(Some(out.join("\n") + "\n"))
}

impl Service {
    /// Follow the edge container: does it mount the socket directory? Updates the bridge listener and the "Agents" unit when
    /// the answer changes. No container counts as "sockets": a new one comes from the current compose file, and there is
    /// nobody to serve on the bridge meanwhile.
    pub async fn perimeter_sync(&self) -> bool {
        let _g = self.perimeter.sync_lock.lock().await;
        let facts = match self.docker.container_facts(&self.cfg.edge_container).await {
            Ok(f) => f,
            Err(e) => { tracing::debug!("периметр: docker inspect {}: {e:#}", self.cfg.edge_container); return self.perimeter.sockets.lock().unwrap().unwrap_or(true) }
        };
        let sockets = facts.as_ref().map(|f| socket_mounted(f, &self.cfg.edge_run_dir)).unwrap_or(true);
        let prev = self.perimeter.sockets.lock().unwrap().replace(sockets);
        self.perimeter.bridge.send_if_modified(|v| { let want = !sockets; let changed = *v != want; *v = want; changed });
        if prev != Some(sockets) {
            let bridge = (!sockets).then(|| self.cfg.edge_agents_bridge_listen.clone());
            if let Err(e) = crate::agents_svc::ensure_installed(&self.cfg.agents_sock(), bridge.as_deref()).await { tracing::warn!("сервис «Агенты» (cloudos-agents): {e:#}"); }
            if sockets {
                if prev == Some(false) { let _ = self.store.event("edge", "perimeter", "edge переведён на сокеты: ядро и «Агенты» больше не слушают мост Docker, админка Caddy закрыта от сетей приложений"); }
            } else {
                tracing::warn!("edge: контейнер {} создан без каталога {} — ядро слушает мост Docker, как раньше; переведите его: sudo cloudd edge-migrate", self.cfg.edge_container, self.cfg.edge_run_dir.display());
                let _ = self.store.event("edge", "perimeter", &format!("контейнер edge создан без каталога сокетов {}: ядро и админка Caddy доступны из контейнеров приложений. Выполните на хосте: sudo cloudd edge-migrate (около 10 секунд без HTTPS)", self.cfg.edge_run_dir.display()));
            }
        }
        sockets
    }

    /// For `/api/system/perimeter` and the status page: the edge mode as last seen, the guard and the firewall switch.
    pub async fn perimeter_view(&self) -> serde_json::Value {
        let (deny, allow) = self.edge_guard().await;
        serde_json::json!({
            "edge_sockets": self.perimeter.sockets(),
            "run_dir": self.cfg.edge_run_dir,
            "bridge_listen": (*self.perimeter.bridge.borrow()).then(|| self.cfg.edge_bridge_listen.clone()),
            "admin_listen": self.edge.admin_listen().await,
            "guard": { "deny": deny, "allow": allow },
            "firewall": if std::env::var("CLOUDD_FIREWALL").map(|v| v == "off").unwrap_or(false) { "off" } else { "on" },
            "control_ports": [443, self.cfg.agents_port],
        })
    }

    /// Inputs of the control-plane guard in the edge render (see `guard_ranges`).
    pub async fn edge_guard(&self) -> (Vec<String>, Vec<String>) {
        let Ok(Some(f)) = self.docker.container_facts(&self.cfg.edge_container).await else { return (vec![], vec![]) };
        let gw = f.pid.and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/net/route")).ok()).and_then(|t| default_gateway(&t));
        guard_ranges(&f, gw)
    }

    /// Keep the host firewall rules in place (see the module comment). Returns what was changed; `CLOUDD_FIREWALL=off`
    /// leaves the firewall alone. Idempotent and cheap: three listings when nothing changed.
    pub async fn ensure_firewall(&self) -> Result<Vec<String>> {
        let mut changed = Vec::new();
        if std::env::var("CLOUDD_FIREWALL").map(|v| v == "off").unwrap_or(false) { return Ok(changed) }
        let _g = self.perimeter.fw_lock.lock().await;
        // Docker creates DOCKER-USER; before that (or without iptables, or not as root) there is nothing to do yet
        let Some(user_chain) = ipt_list("DOCKER-USER").await else {
            if !self.perimeter.fw_skipped.swap(true, std::sync::atomic::Ordering::SeqCst) { tracing::warn!("периметр: цепочка DOCKER-USER недоступна (нет iptables или Docker ещё не поднялся) — правила firewall не ставятся"); }
            return Ok(changed);
        };
        self.perimeter.fw_skipped.store(false, std::sync::atomic::Ordering::SeqCst);
        let ports = [443u16, self.cfg.agents_port];
        for (chain, rules) in firewall_chains(&ports) {
            let version = rules.first().and_then(|r| r.get(3)).cloned().unwrap_or_default();
            let have = ipt_list(chain).await;
            let current = have.as_deref().map(|l| l.contains(&version) && l.lines().filter(|x| x.starts_with("-A ")).count() == rules.len()).unwrap_or(false);
            if current { continue }
            if have.is_none() { ipt_ok(&strs(&["-N", chain])).await?; }
            ipt_ok(&strs(&["-F", chain])).await?;
            for r in &rules { let mut a = strs(&["-A", chain]); a.extend(r.iter().cloned()); ipt_ok(&a).await?; }
            changed.push(format!("цепочка {chain}"));
        }
        let nets: Vec<(String, String, Option<String>)> = self.docker.docker.list_networks(None::<bollard::query_parameters::ListNetworksOptions>).await.context("docker networks")?.into_iter()
            .map(|n| (n.id.unwrap_or_default(), n.driver.unwrap_or_default(), n.options.and_then(|o| o.get("com.docker.network.bridge.name").cloned()))).collect();
        let want = firewall_jumps(&bridge_names(&nets));
        for parent in ["DOCKER-USER", "INPUT"] {
            let listing = if parent == "DOCKER-USER" { user_chain.clone() } else { ipt_list(parent).await.ok_or_else(|| anyhow!("iptables -S INPUT"))? };
            let (have, accept_all) = parse_jumps(&listing);
            // jumps for bridges that no longer exist
            for (_, iface, target) in have.iter().filter(|(_, i, t)| !want.iter().any(|(p, wi, wt)| *p == parent && wi == i && wt == t)) {
                let mut a = strs(&["-D", parent]); a.extend(jump_spec(iface, target)); ipt_ok(&a).await?;
                changed.push(format!("{parent}: снят переход для {iface}"));
            }
            for (_, iface, target) in want.iter().filter(|(p, _, _)| *p == parent) {
                let at = have.iter().find(|(_, i, t)| i == iface && t == target).map(|(n, _, _)| *n);
                let shadowed = matches!((at, accept_all), (Some(n), Some(a)) if n > a);
                if at.is_some() && !shadowed { continue }
                if shadowed { let mut a = strs(&["-D", parent]); a.extend(jump_spec(iface, target)); ipt_ok(&a).await?; }
                let mut a = strs(&["-I", parent, "1"]); a.extend(jump_spec(iface, target)); ipt_ok(&a).await?;
                changed.push(format!("{parent}: переход для {iface} → {target}"));
            }
        }
        if !changed.is_empty() { tracing::info!("периметр: firewall — {}", changed.join("; ")); }
        Ok(changed)
    }

    /// Firewall watch: at start and every minute (a new app brings a new docker bridge; a restarted Docker or a host script
    /// can put its own rules above ours).
    pub fn start_perimeter_watch(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = me.ensure_firewall().await { tracing::warn!("периметр: firewall: {e:#}"); }
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
    }

    /// Recreate an edge container made before ADR-010 with the socket directory: patch its compose file and the starter
    /// Caddyfile in place (both are ours; the originals are kept next to them as `.pre-sockets`), `docker compose up -d`,
    /// load the configuration. About ten seconds without HTTPS.
    pub async fn edge_migrate(&self) -> Result<String> {
        let name = &self.cfg.edge_container;
        let f = self.docker.container_facts(name).await?.ok_or_else(|| anyhow!("контейнера {name} нет — поднимите edge установщиком (bootstrap/install.sh edge)"))?;
        if socket_mounted(&f, &self.cfg.edge_run_dir) { self.restore_edge().await?; return Ok(format!("{name} уже работает через сокеты в {}", self.cfg.edge_run_dir.display())) }
        let project = f.labels.get("com.docker.compose.project").cloned().ok_or_else(|| anyhow!("{name} создан не через docker compose — пересоздайте его вручную с томом {0}:{0}", self.cfg.edge_run_dir.display()))?;
        let files: Vec<String> = f.labels.get("com.docker.compose.project.config_files").map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
        let [compose] = files.as_slice() else { bail!("у {name} не один compose-файл ({}) — добавьте том {1}:{1} и уберите порт 2019 вручную", files.join(", "), self.cfg.edge_run_dir.display()) };
        let caddyfile = f.mounts.iter().find(|(_, dst)| dst == "/etc/caddy/Caddyfile").map(|(src, _)| src.clone()).ok_or_else(|| anyhow!("у {name} не смонтирован стартовый Caddyfile"))?;
        let text = std::fs::read_to_string(compose).with_context(|| format!("чтение {compose}"))?;
        let keep = |p: &str| { let b = format!("{p}.pre-sockets"); if !Path::new(&b).exists() { let _ = std::fs::copy(p, &b); } };
        if let Some(patched) = patch_edge_compose(&text, &self.cfg.edge_run_dir)? {
            keep(compose);
            std::fs::write(compose, patched).with_context(|| format!("запись {compose}"))?;
        }
        keep(&caddyfile);
        std::fs::write(&caddyfile, bootstrap_caddyfile(&self.cfg.edge_run_dir)).with_context(|| format!("запись {caddyfile}"))?;
        std::fs::create_dir_all(&self.cfg.edge_run_dir).with_context(|| format!("каталог {}", self.cfg.edge_run_dir.display()))?;
        self.store.event("edge", "perimeter", "перевод edge на сокеты: docker compose up -d (HTTPS прервётся примерно на десять секунд)")?;
        let args: Vec<String> = vec!["compose".into(), "-p".into(), project, "-f".into(), compose.clone()];
        let wd = f.labels.get("com.docker.compose.project.working_dir").cloned().or_else(|| Path::new(compose).parent().map(|p| p.display().to_string()));
        crate::updates::run_docker(&args, &["up", "-d"], wd.as_deref(), &[], Duration::from_secs(300)).await?;
        // the new container starts with the starter Caddyfile and outside the app networks
        let mut last = Ok(());
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(700)).await;
            last = self.restore_edge().await;
            if last.is_ok() { break }
        }
        last.context("edge пересоздан, но конфигурация не загрузилась — сторож ядра повторит попытку")?;
        let _ = self.ensure_firewall().await;   // this process may exit before the pass started by the reload finishes
        Ok(format!("{name} пересоздан с каталогом {}: ядро, «Агенты» и админка Caddy теперь на сокетах", self.cfg.edge_run_dir.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(mounts: &[(&str, &str)], nets: &[(&str, &str, u8)]) -> ContainerFacts {
        ContainerFacts { pid: Some(1), running: true, mounts: mounts.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), networks: nets.iter().map(|(n, ip, p)| (n.to_string(), ip.to_string(), *p)).collect(), labels: Default::default() }
    }

    #[test]
    fn socket_directory_must_be_mounted_at_the_same_path() {
        let dir = Path::new("/run/cloudos-edge");
        assert!(socket_mounted(&facts(&[("/var/run/tailscale", "/var/run/tailscale"), ("/run/cloudos-edge", "/run/cloudos-edge")], &[]), dir));
        assert!(socket_mounted(&facts(&[("/run/cloudos-edge/", "/run/cloudos-edge")], &[]), dir));
        assert!(!socket_mounted(&facts(&[("/var/run/tailscale", "/var/run/tailscale")], &[]), dir), "контейнер до ADR-010");
        assert!(!socket_mounted(&facts(&[("/srv/other", "/run/cloudos-edge")], &[]), dir), "другой каталог хоста — сокеты ядра там не появятся");
    }

    #[test]
    fn default_gateway_is_read_from_the_route_table() {
        let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\neth3\t00000000\t0101C80A\t0003\t0\t0\t0\t00000000\t0\t0\t0\neth4\t0000C80A\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n";
        assert_eq!(default_gateway(table), Some(Ipv4Addr::new(10, 200, 1, 1)));
        assert_eq!(default_gateway("Iface\tDestination\tGateway\neth4\t0000C80A\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n"), None, "без маршрута по умолчанию");
        assert_eq!(default_gateway(""), None);
        assert_eq!(subnet_of("10.200.9.4", 24).as_deref(), Some("10.200.9.0/24"));
        assert_eq!(subnet_of("172.17.3.9", 16).as_deref(), Some("172.17.0.0/16"));
        assert_eq!(subnet_of("not-an-ip", 24), None);
    }

    #[test]
    fn guard_refuses_the_app_networks_and_lets_the_nat_gateway_through() {
        let f = facts(&[], &[("03-edge_default", "10.200.1.2", 24), ("gitea-net", "10.200.9.4", 24), ("coder-net", "10.200.0.4", 24)]);
        let (deny, allow) = guard_ranges(&f, Some(Ipv4Addr::new(10, 200, 1, 1)));
        assert_eq!(deny, vec!["10.200.0.0/24", "10.200.1.0/24", "10.200.9.0/24"]);
        assert_eq!(allow, vec!["10.200.1.1/32"], "с адреса шлюза приходят все, кто зашёл через опубликованный порт");
        // the gateway is unknown or not in any of the edge's networks: refusing the subnets would refuse every visitor
        assert_eq!(guard_ranges(&f, None), (vec![], vec![]));
        assert_eq!(guard_ranges(&f, Some(Ipv4Addr::new(192, 168, 1, 1))), (vec![], vec![]));
        assert_eq!(guard_ranges(&facts(&[], &[]), Some(Ipv4Addr::new(10, 200, 1, 1))), (vec![], vec![]));
    }

    #[test]
    fn firewall_rules_cover_published_control_ports_and_host_services() {
        let chains = firewall_chains(&[443, 8445]);
        let get = |name: &str| chains.iter().find(|(n, _)| *n == name).map(|(_, r)| r.iter().map(|x| x.join(" ")).collect::<Vec<_>>()).unwrap();
        let fwd = get(CH_FWD);
        // only connections that came through a published port (DNAT) with that original port — not port 443 of the internet
        assert_eq!(fwd, vec![
            "-m comment --comment cloudos-perimeter:v2:443,8445 -p tcp -m conntrack --ctstate DNAT --ctorigdstport 443 --ctdir ORIGINAL -j REJECT --reject-with tcp-reset",
            "-p tcp -m conntrack --ctstate DNAT --ctorigdstport 8445 --ctdir ORIGINAL -j REJECT --reject-with tcp-reset",
        ]);
        // the direction matters: the edge answers its visitors through a docker bridge, and those packets belong to a
        // DNAT'ed connection to port 443 too
        assert!(fwd.iter().all(|r| r.contains("--ctdir ORIGINAL")));
        assert_eq!(get(CH_IN), vec!["-m comment --comment cloudos-perimeter:v2:443,8445 -p tcp -m multiport --dports 443,8445 -j REJECT --reject-with tcp-reset"]);
        let ws = get(CH_WS);
        assert_eq!(ws.len(), 5);
        assert!(ws[0].ends_with("--ctstate RELATED,ESTABLISHED -j ACCEPT") && ws[1].contains("--dports 53,67") && ws[2].contains("--dport 53") && ws[3] == "-p icmp -j ACCEPT", "{ws:?}");
        assert_eq!(ws[4], "-j REJECT --reject-with icmp-port-unreachable", "всё остальное с моста проекта на сам хост закрыто");
        // another "Agents" port is another version: the chains are refilled
        assert!(firewall_chains(&[443, 9000]).iter().all(|(_, r)| r[0].join(" ").contains("cloudos-perimeter:v2:443,9000")));
    }

    #[test]
    fn firewall_jumps_follow_the_docker_bridges() {
        let nets = vec![
            ("330a36da0b25aaaaaaaaaaaa".to_string(), "bridge".to_string(), None),
            ("ffff".to_string(), "bridge".to_string(), Some("docker0".to_string())),
            ("c0ffee000000bbbb".to_string(), "macvlan".to_string(), None),
            ("deadbeef0000cccc".to_string(), "host".to_string(), None),
        ];
        let bridges = bridge_names(&nets);
        assert_eq!(bridges, vec!["br-330a36da0b25", "docker0"], "только мосты; macvlan проекта и сеть host — не они");
        let jumps = firewall_jumps(&bridges);
        assert!(jumps.contains(&("DOCKER-USER", "net-+".to_string(), CH_FWD)) && jumps.contains(&("INPUT", "net-+".to_string(), CH_WS)));
        assert!(jumps.contains(&("DOCKER-USER", "br-330a36da0b25".to_string(), CH_FWD)) && jumps.contains(&("INPUT", "docker0".to_string(), CH_IN)));
        assert_eq!(jumps.len(), 6);
        assert!(!jumps.iter().any(|(_, i, _)| i == "br-+"), "шаблон br-+ задел бы и мост локальной сети хоста");

        // what is already in a chain: ours by the comment, and the host script's «accept everything from project bridges»
        let listing = "-N DOCKER-USER\n-A DOCKER-USER -m physdev --physdev-is-bridged -j ACCEPT\n-A DOCKER-USER -i net-+ -m comment --comment cloudos-perimeter -j CLOUDOS-CTL-FWD\n-A DOCKER-USER -i net-+ -o net-+ -j DROP\n-A DOCKER-USER -o net-+ -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT\n-A DOCKER-USER -i net-+ -j ACCEPT\n-A DOCKER-USER -i br-old -m comment --comment \"cloudos-perimeter\" -j CLOUDOS-CTL-FWD\n";
        let (ours, accept_all) = parse_jumps(listing);
        assert_eq!(ours, vec![(1, "net-+".to_string(), CH_FWD.to_string()), (5, "br-old".to_string(), CH_FWD.to_string())]);
        assert_eq!(accept_all, Some(4), "переход ниже этого правила пакетов с мостов проектов не увидит");
        assert_eq!(parse_jumps("-P INPUT ACCEPT\n-A INPUT -j ts-input\n"), (vec![], None));
        assert_eq!(jump_spec("net-+", CH_WS).join(" "), "-i net-+ -m comment --comment cloudos-perimeter -j CLOUDOS-WS-IN");
    }

    #[test]
    fn old_edge_compose_gets_the_socket_directory_and_loses_the_admin_port() {
        let dir = Path::new("/run/cloudos-edge");
        // the compose file as shipped before ADR-010
        let old = "services:\n  caddy:\n    image: caddy:2\n    container_name: cloudos-edge\n    restart: unless-stopped\n    ports:\n      - \"80:80\"\n      - \"443:443\"\n      - \"8444:8444\"                 # Coder (route: port)\n      - \"8450-8499:8450-8499\"       # route: port apps\n      - \"127.0.0.1:2019:2019\"       # the Admin API — only for cloudd on the host\n    extra_hosts:\n      - \"host.docker.internal:host-gateway\"\n    networks:\n      - default\n      - coder-net\n    volumes:\n      - ./Caddyfile.bootstrap:/etc/caddy/Caddyfile:ro\n      - caddy-data:/data\n      - caddy-config:/config\n      - /var/run/tailscale:/var/run/tailscale   # certificates from tailscaled\nnetworks:\n  coder-net:\n    external: true\nvolumes:\n  caddy-data:\n  caddy-config:\n";
        let new = patch_edge_compose(old, dir).unwrap().expect("файл меняется");
        assert!(!new.contains("2019"), "админка Caddy больше не публикуется: {new}");
        let lines: Vec<&str> = new.lines().collect();
        let at = lines.iter().position(|l| l.contains("/var/run/tailscale:/var/run/tailscale")).unwrap();
        assert!(lines[at + 1].starts_with("      - /run/cloudos-edge:/run/cloudos-edge"), "том — следующей строкой, с тем же отступом: {:?}", lines[at + 1]);
        assert!(new.contains("\"443:443\"") && new.contains("caddy-data:/data") && new.ends_with("  caddy-config:\n"), "остальное на месте");
        let v: serde_json::Value = serde_yaml_ng::from_str(&new).expect("по-прежнему YAML");
        assert_eq!(v["services"]["caddy"]["volumes"].as_array().unwrap().len(), 5);
        assert_eq!(v["services"]["caddy"]["ports"].as_array().unwrap().len(), 4);
        // done once: the second pass finds nothing to change
        assert_eq!(patch_edge_compose(&new, dir).unwrap(), None);
        // a file without the Tailscale directory (older still): the mount goes after the Caddy config volume
        let older = old.replace("      - /var/run/tailscale:/var/run/tailscale   # certificates from tailscaled\n", "");
        let p = patch_edge_compose(&older, dir).unwrap().unwrap();
        let l: Vec<&str> = p.lines().collect();
        assert!(l[l.iter().position(|x| x.trim() == "- caddy-config:/config").unwrap() + 1].contains("/run/cloudos-edge:/run/cloudos-edge"));
        // somebody else's compose file is not touched
        assert!(patch_edge_compose("services:\n  web:\n    image: nginx\n    ports:\n      - \"2019:2019\"\n", dir).is_err());
        let boot = bootstrap_caddyfile(dir);
        assert!(boot.contains("admin unix//run/cloudos-edge/admin.sock") && boot.contains("persist_config off") && !boot.contains("2019"), "{boot}");
    }

    #[test]
    fn shipped_edge_files_are_already_on_sockets() {
        let dir = Path::new("/run/cloudos-edge");
        // a fresh install gets what the migration produces
        assert_eq!(include_str!("../../bootstrap/edge/Caddyfile.bootstrap"), bootstrap_caddyfile(dir));
        let compose = include_str!("../../bootstrap/edge/docker-compose.yml");
        assert_eq!(patch_edge_compose(compose, dir).unwrap(), None, "в поставляемом compose каталог сокетов смонтирован, порт админки не опубликован");
        let v: serde_json::Value = serde_yaml_ng::from_str(compose).unwrap();
        assert!(v["services"]["caddy"]["volumes"].as_array().unwrap().iter().any(|x| x == "/run/cloudos-edge:/run/cloudos-edge"));
        assert!(!compose.contains("2019"));
    }
}
