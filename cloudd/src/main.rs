//! `cloudd` — ядро Cloud OS: сервер API/Desktop и CLI поверх одного сервисного слоя.

mod api;
mod caddy;
mod coder;
mod config;
mod docker;
mod feeds;
mod files;
mod gate;
mod importer;
mod incus;
mod komodo;
mod model;
mod monitor;
mod appdata;
mod backup;
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
    /// Запустить сервер API и Desktop (systemd)
    Serve {
        /// При старте применить все project.yaml из репозитория и перезагрузить edge
        #[arg(long)]
        reconcile: bool,
    },
    /// Здоровье компонентов ядра
    Status,
    /// Проекты
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// Workspace проекта (через Coder)
    Workspace {
        #[command(subcommand)]
        cmd: WsCmd,
    },
    /// Приложения из Store (через App Runtime)
    App {
        #[command(subcommand)]
        cmd: AppCmd,
    },
    /// Выдать проекту capability (маршрут на gate)
    Grant { project: String, capability: String },
    /// Снять capability
    Revoke { project: String, capability: String },
    /// Применить все project.yaml и конфигурацию edge
    Reconcile,
    /// Лента событий
    Events,
    /// Каталог Store: импорт из upstream-каталогов
    Store {
        #[command(subcommand)]
        cmd: StoreCmd,
    },
}

#[derive(Subcommand)]
enum StoreCmd {
    /// Импортировать каталоги в store/ (рукописные манифесты с origin: manual не трогаются)
    Import {
        /// Источник: runtipi | coolify | umbrel | all
        #[arg(default_value = "all")]
        source: String,
        /// Только эти приложения (через запятую)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Каталог store (по умолчанию CLOUDD_REPO_DIR/store)
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum ProjectCmd {
    List,
    Show { name: String },
    /// Создать проект: записать project.yaml и применить
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
    },
    /// Применить существующий projects/<name>/project.yaml
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
    /// Каталог Store и установленные
    List,
    Install { name: String },
    Remove {
        name: String,
        /// Вместе с данными в tank/apps, секретами и переопределениями
        #[arg(long)]
        purge: bool,
    },
    Logs {
        name: String,
        #[arg(long, default_value_t = 100)]
        tail: u32,
    },
    /// Подключить приложение к другому: сеть провайдера, <ПРОВАЙДЕР>_URL и ключи из его хуков
    Link { consumer: String, provider: String },
    Unlink { consumer: String, provider: String },
    /// Общая сеть приложений (имя <app>.apps): on | off
    Shared { name: String, state: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Журнал — в stderr: stdout CLI принадлежит данным (JSON/таблицы), иначе `cloudd … | jq` ломается и убивает процесс по SIGPIPE.
    use std::io::IsTerminal;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "cloudd=info,tower_http=info".into()))
        .init();
    let cli = Cli::parse();
    // импорт каталога не требует рантаймов и БД — только каталог store
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
            ProjectCmd::Create { name, runtime, cpu, memory, home, capabilities } => {
                let runtime = serde_yaml_ng::from_str::<Runtime>(&runtime).map_err(|_| anyhow::anyhow!("runtime: incus | incus-nesting | incus-vm"))?;
                let spec = ProjectSpec { schema: 1, project: name, workspace: WorkspaceSpec { runtime, cpu, memory, home, image: None }, agents: vec!["claude-code".into(), "codex".into()], capabilities };
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
        Cmd::Grant { project, capability } => print_json(&svc.grant(&project, &capability).await?),
        Cmd::Revoke { project, capability } => print_json(&svc.revoke(&project, &capability).await?),
        Cmd::Reconcile => {
            svc.reconcile_all().await?;
            println!("reconcile выполнен");
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
    svc.start_backup_scheduler();
    svc.start_update_checker();
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
        let app = app.clone();
        handles.push(tokio::spawn(async move { axum::serve(listener, app).await }));
    }
    if handles.is_empty() {
        anyhow::bail!("ни один адрес из CLOUDD_LISTEN не удалось занять");
    }
    for h in handles {
        h.await??;
    }
    Ok(())
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
