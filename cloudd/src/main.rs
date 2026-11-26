//! `cloudd` — the Cloud OS core: the API/Desktop server and the CLI on top of one
//! service layer.

mod accounts;
mod agentd;
mod agents_svc;
mod agents;
mod api;
mod caddy;
mod coder;
mod config;
mod docker;
mod failover;
mod feeds;
mod files;
mod gate;
mod gateauth;
mod hosts;
mod importer;
mod incus;
mod komodo;
mod limits;
mod llm;
mod model;
mod monitor;
mod perimeter;
mod pressure;
mod appdata;
mod backup;
mod budgets;
mod service;
mod setup;
mod store;
mod updates;
#[cfg(test)]
mod tests;

use anyhow::Result;
use clap::{Parser, Subcommand};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Table};
use model::*;
use service::Service;

#[derive(Parser)]
#[command(name = "cloudd", version, about = "Cloud OS core: projects · workspaces · apps · grants")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the API and Desktop server
    /// (systemd)
    Serve {
        /// At start apply every project.yaml from the repository and reload the edge
        #[arg(long)]
        reconcile: bool,
    },
    /// Health of the core components
    Status,
    /// Projects
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// Project workspace (via Coder)
    Workspace {
        #[command(subcommand)]
        cmd: WsCmd,
    },
    /// Apps from the Store (via the App
    /// Runtime)
    App {
        #[command(subcommand)]
        cmd: AppCmd,
    },
    /// Grant a capability to a project (a route on
    /// the gate)
    Grant { project: String, capability: String },
    /// Revoke a capability
    Revoke { project: String, capability: String },
    /// Apply every project.yaml and the edge
    /// configuration
    Reconcile,
    /// Recreate an edge container made before ADR-010 so that it reaches the core through unix sockets (about ten
    /// seconds without HTTPS)
    EdgeMigrate,
    /// Put the perimeter rules of the host firewall in place now (the core also does it every minute)
    Firewall,
    /// Event feed
    Events,
    /// Store catalog: import from upstream
    /// catalogs
    Store {
        #[command(subcommand)]
        cmd: StoreCmd,
    },
}

#[derive(Subcommand)]
enum StoreCmd {
    /// Import catalogs into store/ (hand-written manifests with origin: manual are left
    /// alone)
    Import {
        /// Source: runtipi | coolify | umbrel | all
        #[arg(default_value = "all")]
        source: String,
        /// Only these apps (comma-separated)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Store directory (default CLOUDD_REPO_DIR/store)
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum ProjectCmd {
    List,
    Show { name: String },
    /// Create a project: write project.yaml and apply
    Create {
        name: String,
        #[arg(long, default_value = "incus-nesting")]
        runtime: String,
        #[arg(long, default_value_t = 2)]
        cpu: u32,
        #[arg(long, default_value_t = 4)]
        memory: u32,
        #[arg(long, default_value_t = 50)]
        home: u32,
        #[arg(long = "cap")]
        capabilities: Vec<String>,
        /// Project note: what it is and what for
        #[arg(long)]
        description: Option<String>,
    },
    /// Apply an existing projects/<name>/project.yaml
    Apply { name: String },
    Delete { name: String },
}

#[derive(Subcommand)]
enum WsCmd {
    Start { project: String },
    Stop { project: String },
}

#[derive(Subcommand)]
enum AppCmd {
    /// Store catalog and installed apps
    List,
    Install { name: String },
    Remove {
        name: String,
        /// Together with the data in tank/apps, secrets and overrides
        #[arg(long)]
        purge: bool,
    },
    Logs {
        name: String,
        #[arg(long, default_value_t = 100)]
        tail: u32,
    },
    /// Link an app to another: the provider network, <PROVIDER>_URL and the keys from its
    /// hooks
    Link { consumer: String, provider: String },
    Unlink { consumer: String, provider: String },
    /// Shared app network (name <app>.apps): on | off
    Shared { name: String, state: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logging goes to stderr: the CLI's stdout belongs to data (JSON/tables), otherwise `cloudd … | jq` breaks and kills the
    // process with SIGPIPE.
    use std::io::IsTerminal;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "cloudd=info,tower_http=info".into()))
        .init();
    let cli = Cli::parse();
    // a catalog import needs no runtimes and no DB — only the store
    // directory
    if let Cmd::Store { cmd: StoreCmd::Import { source, only, dir } } = &cli.cmd {
        let dir = dir.clone().unwrap_or_else(|| std::path::PathBuf::from(std::env::var("CLOUDD_REPO_DIR").unwrap_or_else(|_| "/var/lib/cloudos/repo".into())).join("store"));
        for r in importer::import(source, &dir, only).await? {
            println!("[{}] импортировано: {} · пропущено ручных: {} · пропущено: {}", r.source, r.imported.len(), r.skipped_manual.len(), r.skipped.len());
            for (id, why) in &r.skipped {
                println!("  - {id}: {why}");
            }
        }
        return Ok(());
    }
    let cfg = config::Config::load()?;
    let svc = Service::new(cfg)?;

    match cli.cmd {
        Cmd::Serve { reconcile } => serve(svc, reconcile).await,
        Cmd::Status => {
            let s = svc.status().await?;
            let mut t = table(vec!["компонент", "статус", "детали"]);
            for c in s.components {
                t.add_row(vec![c.name, c.status, c.detail.unwrap_or_default().chars().take(80).collect()]);
            }
            println!("Cloud OS cloudd {} · edge https://{} · Coder {}\n{t}", s.version, s.edge_host, s.coder_url);
            Ok(())
        }
        Cmd::Project { cmd } => match cmd {
            ProjectCmd::List => {
                let mut t = table(vec!["проект", "runtime", "workspace", "агент", "gate", "grants", "не хватает"]);
                for p in svc.projects().await? {
                    t.add_row(vec![
                        p.name,
                        p.spec.workspace.runtime.as_param().into(),
                        p.workspace.as_ref().map(|w| w.status.clone()).unwrap_or("—".into()),
                        p.workspace.as_ref().and_then(|w| w.agent_status.clone()).unwrap_or("—".into()),
                        p.gate.status,
                        p.grants.iter().map(|g| format!("{}→{}", g.capability, g.app)).collect::<Vec<_>>().join(", "),
                        p.missing_capabilities.join(", "),
                    ]);
                }
                println!("{t}");
                Ok(())
            }
            ProjectCmd::Show { name } => print_json(&svc.project(&name).await?),
            ProjectCmd::Create { name, runtime, cpu, memory, home, capabilities, description } => {
                let runtime = serde_yaml_ng::from_str::<Runtime>(&runtime).map_err(|_| anyhow::anyhow!("runtime: incus | incus-nesting | incus-vm"))?;
                let spec = ProjectSpec { schema: 1, project: name, workspace: WorkspaceSpec { runtime, cpu, memory, home, image: None }, agents: vec!["claude-code".into(), "codex".into()], capabilities, description, hidden: false };
                print_json(&svc.project_apply(spec).await?)
            }
            ProjectCmd::Apply { name } => {
                let spec = svc.project_spec_from_repo(&name)?.ok_or_else(|| anyhow::anyhow!("нет projects/{name}/project.yaml"))?;
                print_json(&svc.project_apply(spec).await?)
            }
            ProjectCmd::Delete { name } => {
                svc.project_delete(&name).await?;
                println!("проект {name} удалён");
                Ok(())
            }
        },
        Cmd::Workspace { cmd } => match cmd {
            WsCmd::Start { project } => print_json(&svc.workspace_transition(&project, "start").await?),
            WsCmd::Stop { project } => print_json(&svc.workspace_transition(&project, "stop").await?),
        },
        Cmd::App { cmd } => match cmd {
            AppCmd::List => {
                let mut t = table(vec!["приложение", "provides", "route", "установлено", "состояние", "url", "выдано"]);
                for a in svc.apps().await? {
                    t.add_row(vec![
                        a.name,
                        a.manifest.provides.join(", "),
                        format!("{:?}", a.manifest.route.mode).to_lowercase(),
                        if a.installed { "да".into() } else { "—".into() },
                        a.state.unwrap_or("—".into()),
                        a.url.unwrap_or("—".into()),
                        a.granted_to.join(", "),
                    ]);
                }
                println!("{t}");
                Ok(())
            }
            AppCmd::Install { name } => print_json(&svc.app_install(&name).await?),
            AppCmd::Remove { name, purge } => {
                svc.app_remove(&name, purge).await?;
                println!("приложение {name} удалено{}", if purge { " полностью" } else { " (данные и пароли сохранены)" });
                Ok(())
            }
            AppCmd::Logs { name, tail } => {
                print!("{}", svc.app_logs(&name, tail).await?);
                Ok(())
            }
            AppCmd::Link { consumer, provider } => print_json(&svc.app_link(&consumer, &provider).await?),
            AppCmd::Unlink { consumer, provider } => print_json(&svc.app_unlink(&consumer, &provider).await?),
            AppCmd::Shared { name, state } => print_json(&svc.app_set_shared(&name, matches!(state.as_str(), "on" | "true" | "1")).await?),
        },
        Cmd::Grant { project, capability } => print_json(&svc.grant(&project, &capability, None).await?),
        Cmd::Revoke { project, capability } => print_json(&svc.revoke(&project, &capability).await?),
        Cmd::Reconcile => {
            svc.reconcile_all().await?;
            println!("reconcile выполнен");
            Ok(())
        }
        Cmd::EdgeMigrate => {
            println!("{}", svc.edge_migrate().await?);
            Ok(())
        }
        Cmd::Firewall => {
            let changed = svc.ensure_firewall().await?;
            println!("{}", if changed.is_empty() { "правила на месте".to_string() } else { changed.join("\n") });
            Ok(())
        }
        Cmd::Store { .. } => unreachable!(),
        Cmd::Events => {
            for e in svc.store.recent_events(50)?.into_iter().rev() {
                println!("{} {:8} {:16} {}", e.at, e.kind, e.subject, e.message);
            }
            Ok(())
        }
    }
}

async fn serve(svc: Service, reconcile: bool) -> Result<()> {
    svc.start_monitor();
    svc.start_edge_watch();
    svc.start_gate_watch();
    svc.start_pressure_watch();
    svc.start_backup_scheduler();
    svc.start_update_checker();
    svc.start_agents_poller();
    svc.start_hosts_prober();
    svc.start_perimeter_watch();
    // the first look at the edge container: installs the "Agents" unit for it and says whether the docker bridge is needed
    { let s = svc.clone(); tokio::spawn(async move { s.perimeter_sync().await; }); }
    if reconcile {
        let s = svc.clone();
        tokio::spawn(async move {
            if let Err(e) = s.reconcile_all().await {
                tracing::error!("reconcile при старте: {e:#}");
            }
        });
    }
    let app = api::router(svc.clone());
    let mut handles = Vec::new();
    for addr in &svc.cfg.listen {
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("не слушаю {addr}: {e}");
                continue;
            }
        };
        tracing::info!("cloudd слушает http://{addr}");
        // the routes for the services of this host go only where nothing but this host can come from
        let local = listener.local_addr().map(|a| a.ip().is_loopback()).unwrap_or(false);
        let app = if local { app.clone().merge(api::internal_router(svc.clone())) } else { app.clone() };
        handles.push(tokio::spawn(async move { axum::serve(listener, app).await }));
    }
    if handles.is_empty() {
        anyhow::bail!("ни один адрес из CLOUDD_LISTEN не удалось занять");
    }
    // the edge comes through a unix socket in the directory shared with its container (ADR-010): nothing in the docker
    // networks can reach the core this way
    match bind_unix(&svc.cfg.cloudd_sock()) {
        Ok(listener) => {
            tracing::info!("cloudd слушает сокет {}", svc.cfg.cloudd_sock().display());
            let app = app.clone();
            handles.push(tokio::spawn(async move { axum::serve(listener, app).await }));
        }
        Err(e) => tracing::warn!("не слушаю сокет {}: {e:#}", svc.cfg.cloudd_sock().display()),
    }
    // …and the docker bridge only while an edge container without that directory is running
    if !svc.cfg.listen.contains(&svc.cfg.edge_bridge_listen) {
        tokio::spawn(bridge_listener(svc.perimeter.bridge.subscribe(), svc.cfg.edge_bridge_listen.clone(), app.clone()));
    }
    for h in handles {
        h.await??;
    }
    Ok(())
}

/// A unix listener at `path`. The directory is only ever created, never removed or recreated: the edge container holds a
/// bind mount of it, and a recreated directory would leave the container with the old empty one. The socket file of the
/// previous process is replaced.
fn bind_unix(path: &std::path::Path) -> Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
    Ok(listener)
}

/// The core's listener on the docker bridge for an edge container created before ADR-010: bound while `want` says so,
/// closed when the container is replaced by one with the socket directory.
async fn bridge_listener(mut want: tokio::sync::watch::Receiver<bool>, addr: String, app: axum::Router) {
    let mut stop: Option<tokio::sync::oneshot::Sender<()>> = None;
    loop {
        let on = *want.borrow_and_update();
        if on && stop.is_none() {
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => {
                    tracing::warn!("cloudd слушает мост Docker http://{addr}: контейнер edge создан без каталога сокетов");
                    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
                    let app = app.clone();
                    tokio::spawn(async move { let _ = axum::serve(listener, app).with_graceful_shutdown(async { let _ = rx.await; }).await; });
                    stop = Some(tx);
                }
                Err(e) => tracing::warn!("не слушаю мост Docker {addr}: {e}"),
            }
        } else if !on {
            if let Some(tx) = stop.take() { let _ = tx.send(()); tracing::info!("cloudd больше не слушает мост Docker {addr}"); }
        }
        // a failed bind (the bridge is not up yet) is retried
        let retry = on && stop.is_none();
        tokio::select! {
            r = want.changed() => { if r.is_err() { break } }
            _ = tokio::time::sleep(std::time::Duration::from_secs(15)), if retry => {}
        }
    }
}

fn table(header: Vec<&str>) -> Table {
    let mut t = Table::new();
    t.load_preset(UTF8_FULL_CONDENSED).set_header(header);
    t
}
fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}
