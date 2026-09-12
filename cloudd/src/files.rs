//! Файловый API Desktop («Файлы» на схеме раздела 2): три корня — данные приложений (tank/apps), репозиторий Store
//! (манифесты и overrides) и home workspace'ов через файловый API Incus. Ничего вне корней ядро не отдаёт:
//! путь нормализуется, `..` и абсолютные пути отклоняются, символические ссылки не разрешаются наружу.

use crate::service::Service;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use utoipa::ToSchema;

pub const MAX_TEXT: u64 = 2 * 1024 * 1024; // редактор — до 2 MB
pub const MAX_UPLOAD: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub modified: Option<String>,
    /// текстовый ли файл по расширению — можно открыть в редакторе
    pub text: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Listing {
    pub root: String,
    pub path: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Root {
    pub id: String,
    pub title: String,
    pub kind: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WriteBody {
    pub content: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RenameBody {
    pub to: String,
}

pub fn is_text(name: &str) -> bool {
    let lower = name.to_lowercase();
    const EXT: &[&str] = &["txt", "md", "yaml", "yml", "json", "toml", "ini", "cfg", "conf", "env", "sh", "bash", "py", "js", "ts", "rs", "go", "css", "html", "htm", "xml", "csv", "log", "sql", "properties", "service", "caddyfile", "dockerfile", "gitignore", "lock"];
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    if base.starts_with('.') && !base[1..].contains('.') {
        return true; // .env, .gitignore, .npmrc
    }
    match base.rsplit_once('.') {
        Some((_, ext)) => EXT.contains(&ext),
        None => ["caddyfile", "dockerfile", "makefile", "license", "readme", "torrc"].contains(&base),
    }
}

/// Безопасное соединение корня и относительного пути.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel = rel.trim_start_matches('/');
    let mut out = root.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            _ => bail!("недопустимый путь"),
        }
    }
    // защита от symlink наружу: канонизируем существующую часть
    let mut probe = out.clone();
    while !probe.exists() {
        if !probe.pop() {
            break;
        }
    }
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canon = probe.canonicalize().unwrap_or(probe);
    if !canon.starts_with(&canon_root) {
        bail!("путь выходит за пределы корня");
    }
    Ok(out)
}

pub enum RootKind {
    Host(PathBuf),
    /// Incus-инстанс и базовый путь внутри него
    Instance(String, String),
}

impl Service {
    /// Список корней: apps, store, по одному на проект с работающим workspace.
    pub async fn file_roots(&self) -> Result<Vec<Root>> {
        let mut roots = vec![
            Root { id: "apps".into(), title: "Данные приложений (tank/apps)".into(), kind: "host".into() },
            Root { id: "store".into(), title: "Store: манифесты и overrides".into(), kind: "host".into() },
        ];
        for p in self.store.list_projects()? {
            roots.push(Root { id: format!("ws:{}", p.name), title: format!("Workspace {} · /home/coder", p.name), kind: "incus".into() });
        }
        Ok(roots)
    }

    async fn resolve_root(&self, root: &str) -> Result<RootKind> {
        match root {
            "apps" => Ok(RootKind::Host(self.cfg.apps_data_dir.clone())),
            "store" => Ok(RootKind::Host(self.cfg.store_dir())),
            r if r.starts_with("ws:") => {
                let name = &r[3..];
                let st = self.store.get_project(name)?.ok_or_else(|| anyhow!("проект {name} не найден"))?;
                let owner = self.coder.me().await.map(|u| u.username.to_lowercase()).unwrap_or_else(|_| "admin".into());
                let inst = format!("ws-{owner}-{}", st.name);
                if !self.incus.instance_running(&inst).await? {
                    bail!("workspace {name} не запущен");
                }
                Ok(RootKind::Instance(inst, "/home/coder".into()))
            }
            _ => bail!("корень {root} не найден"),
        }
    }

    pub async fn file_list(&self, root: &str, rel: &str) -> Result<Listing> {
        let rel_norm = rel.trim_matches('/').to_string();
        let mut entries = Vec::new();
        match self.resolve_root(root).await? {
            RootKind::Host(base) => {
                let dir = safe_join(&base, &rel_norm)?;
                if !dir.exists() {
                    bail!("нет такого каталога");
                }
                for e in std::fs::read_dir(&dir)? {
                    let e = e?;
                    let md = e.metadata()?;
                    let name = e.file_name().to_string_lossy().to_string();
                    entries.push(Entry {
                        text: !md.is_dir() && is_text(&name),
                        name,
                        dir: md.is_dir(),
                        size: md.len(),
                        modified: md.modified().ok().and_then(|t| time::OffsetDateTime::from(t).format(&time::format_description::well_known::Rfc3339).ok()),
                    });
                }
            }
            RootKind::Instance(inst, base) => {
                let path = if rel_norm.is_empty() { base } else { format!("{base}/{rel_norm}") };
                // ls через exec: имя, тип, размер, mtime — одной строкой на запись
                let out = self.incus.exec(&inst, &["sh", "-c", &format!("cd '{}' && for f in .* *; do [ \"$f\" = . ] || [ \"$f\" = .. ] || [ ! -e \"$f\" ] && continue; if [ -d \"$f\" ]; then t=d; else t=f; fi; printf '%s\\t%s\\t%s\\t%s\\n' \"$t\" \"$(stat -c %s \"$f\")\" \"$(stat -c %Y \"$f\")\" \"$f\"; done", path.replace('\'', "'\\''"))]).await?;
                for line in out.lines() {
                    let parts: Vec<&str> = line.splitn(4, '\t').collect();
                    if parts.len() < 4 {
                        continue;
                    }
                    let name = parts[3].to_string();
                    let dir = parts[0] == "d";
                    let ts = parts[2].parse::<i64>().ok().and_then(|s| time::OffsetDateTime::from_unix_timestamp(s).ok()).and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok());
                    entries.push(Entry { text: !dir && is_text(&name), name, dir, size: parts[1].parse().unwrap_or(0), modified: ts });
                }
            }
        }
        entries.sort_by(|a, b| b.dir.cmp(&a.dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(Listing { root: root.into(), path: rel_norm, entries })
    }

    pub async fn file_read(&self, root: &str, rel: &str) -> Result<(Vec<u8>, String)> {
        let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
        match self.resolve_root(root).await? {
            RootKind::Host(base) => {
                let p = safe_join(&base, rel)?;
                let md = std::fs::metadata(&p)?;
                if md.is_dir() {
                    bail!("это каталог");
                }
                Ok((std::fs::read(&p)?, name))
            }
            RootKind::Instance(inst, base) => {
                let path = format!("{base}/{}", rel.trim_start_matches('/'));
                Ok((self.incus.file_pull(&inst, &path).await?, name))
            }
        }
    }

    pub async fn file_write(&self, root: &str, rel: &str, content: &[u8]) -> Result<()> {
        if content.len() > MAX_UPLOAD {
            bail!("файл больше {} MB", MAX_UPLOAD / 1024 / 1024);
        }
        match self.resolve_root(root).await? {
            RootKind::Host(base) => {
                let p = safe_join(&base, rel)?;
                if let Some(d) = p.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::write(&p, content)?;
            }
            RootKind::Instance(inst, base) => {
                let path = format!("{base}/{}", rel.trim_start_matches('/'));
                self.incus.file_push(&inst, &path, content, 0o644).await?;
                // владелец — coder (uid 1000), иначе файл будет root'а
                let _ = self.incus.exec(&inst, &["chown", "coder:coder", &path]).await;
            }
        }
        self.store.event("files", root, &format!("записан {rel}"))?;
        Ok(())
    }

    pub async fn file_mkdir(&self, root: &str, rel: &str) -> Result<()> {
        match self.resolve_root(root).await? {
            RootKind::Host(base) => std::fs::create_dir_all(safe_join(&base, rel)?)?,
            RootKind::Instance(inst, base) => {
                let path = format!("{base}/{}", rel.trim_start_matches('/'));
                self.incus.exec(&inst, &["sh", "-c", &format!("mkdir -p '{p}' && chown coder:coder '{p}'", p = path.replace('\'', "'\\''"))]).await?;
            }
        }
        Ok(())
    }

    pub async fn file_delete(&self, root: &str, rel: &str) -> Result<()> {
        if rel.trim_matches('/').is_empty() {
            bail!("корень удалять нельзя");
        }
        match self.resolve_root(root).await? {
            RootKind::Host(base) => {
                let p = safe_join(&base, rel)?;
                if p.is_dir() { std::fs::remove_dir_all(&p)?; } else { std::fs::remove_file(&p)?; }
            }
            RootKind::Instance(inst, base) => {
                let path = format!("{base}/{}", rel.trim_start_matches('/'));
                self.incus.exec(&inst, &["rm", "-rf", "--", &path]).await?;
            }
        }
        self.store.event("files", root, &format!("удалён {rel}"))?;
        Ok(())
    }

    pub async fn file_rename(&self, root: &str, rel: &str, to: &str) -> Result<()> {
        match self.resolve_root(root).await? {
            RootKind::Host(base) => std::fs::rename(safe_join(&base, rel)?, safe_join(&base, to)?)?,
            RootKind::Instance(inst, base) => {
                let (a, b) = (format!("{base}/{}", rel.trim_start_matches('/')), format!("{base}/{}", to.trim_start_matches('/')));
                self.incus.exec(&inst, &["mv", "--", &a, &b]).await?;
            }
        }
        Ok(())
    }
}
