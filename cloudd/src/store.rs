//! Состояние ядра в SQLite (`tank/core`) и секрет-стор: зашифрованные age-blob'ы под мастер-ключом хоста (2.9, 3.5).

use crate::model::{AppState, Grant, ProjectState};
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
    identity: Arc<age::x25519::Identity>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
  name TEXT PRIMARY KEY, subnet_index INTEGER NOT NULL UNIQUE, coder_workspace_id TEXT, spec TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS apps (
  name TEXT PRIMARY KEY, manifest TEXT NOT NULL, port INTEGER, komodo_stack_id TEXT, installed_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS grants (
  project TEXT NOT NULL, capability TEXT NOT NULL, app TEXT NOT NULL, granted_at TEXT NOT NULL, PRIMARY KEY (project, capability));
CREATE TABLE IF NOT EXISTS secrets (
  scope TEXT NOT NULL, key TEXT NOT NULL, blob BLOB NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY (scope, key));
CREATE TABLE IF NOT EXISTS kv (
  key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS app_links (
  consumer TEXT NOT NULL, provider TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY (consumer, provider));
CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, kind TEXT NOT NULL, subject TEXT NOT NULL, message TEXT NOT NULL);
"#;

pub fn now() -> String {
    time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default()
}

impl Store {
    pub fn open(db_path: &Path, age_key_path: &Path) -> Result<Self> {
        if let Some(dir) = db_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(db_path).with_context(|| format!("открытие {}", db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        let identity = load_or_create_identity(age_key_path)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)), identity: Arc::new(identity) })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let c = self.conn.lock().map_err(|_| anyhow!("store mutex poisoned"))?;
        f(&c)
    }

    // ---- projects ----
    pub fn list_projects(&self) -> Result<Vec<ProjectState>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT name, subnet_index, coder_workspace_id, spec, created_at FROM projects ORDER BY name")?;
            let rows = st.query_map([], row_project)?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
        })
    }
    pub fn get_project(&self, name: &str) -> Result<Option<ProjectState>> {
        self.with(|c| {
            c.query_row("SELECT name, subnet_index, coder_workspace_id, spec, created_at FROM projects WHERE name=?1", [name], row_project)
                .optional()
                .map_err(Into::into)
        })
    }
    pub fn upsert_project(&self, p: &ProjectState) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO projects(name, subnet_index, coder_workspace_id, spec, created_at) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(name) DO UPDATE SET coder_workspace_id=excluded.coder_workspace_id, spec=excluded.spec",
                params![p.name, p.subnet_index, p.coder_workspace_id.map(|u| u.to_string()), serde_json::to_string(&p.spec)?, p.created_at],
            )?;
            Ok(())
        })
    }
    pub fn delete_project(&self, name: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM grants WHERE project=?1", [name])?;
            c.execute("DELETE FROM secrets WHERE scope=?1", [format!("project:{name}")])?;
            c.execute("DELETE FROM projects WHERE name=?1", [name])?;
            Ok(())
        })
    }
    /// Свободный третий октет подсети проекта (1..=254).
    pub fn next_subnet_index(&self) -> Result<u8> {
        self.with(|c| {
            let mut st = c.prepare("SELECT subnet_index FROM projects")?;
            let used: std::collections::HashSet<i64> = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            (1..=254i64).find(|i| !used.contains(i)).map(|i| i as u8).ok_or_else(|| anyhow!("нет свободных подсетей проектов"))
        })
    }

    // ---- apps ----
    pub fn list_apps(&self) -> Result<Vec<AppState>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT name, manifest, port, komodo_stack_id, installed_at FROM apps ORDER BY name")?;
            let rows = st.query_map([], row_app)?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
        })
    }
    pub fn get_app(&self, name: &str) -> Result<Option<AppState>> {
        self.with(|c| {
            c.query_row("SELECT name, manifest, port, komodo_stack_id, installed_at FROM apps WHERE name=?1", [name], row_app)
                .optional()
                .map_err(Into::into)
        })
    }
    pub fn upsert_app(&self, a: &AppState) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO apps(name, manifest, port, komodo_stack_id, installed_at) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(name) DO UPDATE SET manifest=excluded.manifest, port=excluded.port, komodo_stack_id=excluded.komodo_stack_id",
                params![a.name, serde_json::to_string(&a.manifest)?, a.port, a.komodo_stack_id, a.installed_at],
            )?;
            Ok(())
        })
    }
    /// Удаление записи об установке. Секреты приложения (`app:<name>`) сохраняются намеренно: переустановка возвращает
    /// те же пароли (docs/userflow-apps.md, шаг 4); полностью их стирает только purge в Service::app_remove.
    pub fn delete_app(&self, name: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM grants WHERE app=?1", [name])?;
            c.execute("DELETE FROM apps WHERE name=?1", [name])?;
            Ok(())
        })
    }
    pub fn used_ports(&self) -> Result<Vec<u16>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT port FROM apps WHERE port IS NOT NULL")?;
            let v = st.query_map([], |r| r.get::<_, i64>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(v.into_iter().map(|p| p as u16).collect())
        })
    }

    // ---- grants ----
    pub fn list_grants(&self, project: Option<&str>) -> Result<Vec<Grant>> {
        self.with(|c| {
            let mut out = Vec::new();
            let mut push = |r: &rusqlite::Row| -> rusqlite::Result<()> {
                out.push(Grant { project: r.get(0)?, capability: r.get(1)?, app: r.get(2)?, granted_at: r.get(3)? });
                Ok(())
            };
            match project {
                Some(p) => {
                    let mut st = c.prepare("SELECT project, capability, app, granted_at FROM grants WHERE project=?1 ORDER BY capability")?;
                    let mut rows = st.query([p])?;
                    while let Some(r) = rows.next()? { push(r)?; }
                }
                None => {
                    let mut st = c.prepare("SELECT project, capability, app, granted_at FROM grants ORDER BY project, capability")?;
                    let mut rows = st.query([])?;
                    while let Some(r) = rows.next()? { push(r)?; }
                }
            }
            Ok(out)
        })
    }
    pub fn put_grant(&self, g: &Grant) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT OR REPLACE INTO grants(project, capability, app, granted_at) VALUES(?1,?2,?3,?4)",
                params![g.project, g.capability, g.app, g.granted_at],
            )?;
            Ok(())
        })
    }
    pub fn delete_grant(&self, project: &str, capability: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM grants WHERE project=?1 AND capability=?2", params![project, capability])?;
            Ok(())
        })
    }

    // ---- связи приложений (потребитель → провайдер) ----
    pub fn list_links(&self) -> Result<Vec<crate::model::AppLink>> {
        self.with(|c| {
            let mut out = Vec::new();
            let mut st = c.prepare("SELECT consumer, provider, created_at FROM app_links ORDER BY consumer, provider")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                out.push(crate::model::AppLink { consumer: r.get(0)?, provider: r.get(1)?, created_at: r.get(2)? });
            }
            Ok(out)
        })
    }
    pub fn put_link(&self, l: &crate::model::AppLink) -> Result<()> {
        self.with(|c| {
            c.execute("INSERT OR REPLACE INTO app_links(consumer, provider, created_at) VALUES(?1,?2,?3)", params![l.consumer, l.provider, l.created_at])?;
            Ok(())
        })
    }
    pub fn delete_link(&self, consumer: &str, provider: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM app_links WHERE consumer=?1 AND provider=?2", params![consumer, provider])?;
            Ok(())
        })
    }

    // ---- secrets (age) ----
    pub fn put_secret(&self, scope: &str, key: &str, value: &str) -> Result<()> {
        let blob = encrypt(&self.identity.to_public(), value.as_bytes())?;
        self.with(|c| {
            c.execute(
                "INSERT OR REPLACE INTO secrets(scope, key, blob, updated_at) VALUES(?1,?2,?3,?4)",
                params![scope, key, blob, now()],
            )?;
            Ok(())
        })
    }
    pub fn get_secret(&self, scope: &str, key: &str) -> Result<Option<String>> {
        let blob: Option<Vec<u8>> = self.with(|c| {
            c.query_row("SELECT blob FROM secrets WHERE scope=?1 AND key=?2", params![scope, key], |r| r.get(0))
                .optional()
                .map_err(Into::into)
        })?;
        match blob {
            Some(b) => Ok(Some(String::from_utf8(decrypt(&self.identity, &b)?)?)),
            None => Ok(None),
        }
    }
    pub fn delete_secrets(&self, scope: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM secrets WHERE scope=?1", [scope])?;
            Ok(())
        })
    }
    pub fn list_secret_keys(&self, scope: &str) -> Result<Vec<String>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT key FROM secrets WHERE scope=?1 ORDER BY key")?;
            let v = st.query_map([scope], |r| r.get(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(v)
        })
    }

    // ---- kv: настройки Desktop и прочие небольшие JSON-документы ядра ----
    pub fn get_kv(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let v: Option<String> = self.with(|c| c.query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0)).optional().map_err(Into::into))?;
        Ok(v.and_then(|s| serde_json::from_str(&s).ok()))
    }
    pub fn put_kv(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        self.with(|c| {
            c.execute("INSERT OR REPLACE INTO kv(key, value, updated_at) VALUES(?1,?2,?3)", params![key, serde_json::to_string(value)?, now()])?;
            Ok(())
        })
    }

    // ---- events (лента для Desktop) ----
    pub fn event(&self, kind: &str, subject: &str, message: &str) -> Result<()> {
        tracing::info!(kind, subject, "{message}");
        self.with(|c| {
            c.execute("INSERT INTO events(at, kind, subject, message) VALUES(?1,?2,?3,?4)", params![now(), kind, subject, message])?;
            Ok(())
        })
    }
    pub fn recent_events(&self, limit: u32) -> Result<Vec<Event>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT at, kind, subject, message FROM events ORDER BY id DESC LIMIT ?1")?;
            let v = st
                .query_map([limit], |r| Ok(Event { at: r.get(0)?, kind: r.get(1)?, subject: r.get(2)?, message: r.get(3)? }))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(v)
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct Event {
    pub at: String,
    pub kind: String,
    pub subject: String,
    pub message: String,
}

fn row_project(r: &rusqlite::Row) -> rusqlite::Result<ProjectState> {
    let spec: String = r.get(3)?;
    let ws: Option<String> = r.get(2)?;
    Ok(ProjectState {
        name: r.get(0)?,
        subnet_index: r.get::<_, i64>(1)? as u8,
        coder_workspace_id: ws.and_then(|s| s.parse().ok()),
        spec: serde_json::from_str(&spec).map_err(|e| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e)))?,
        created_at: r.get(4)?,
    })
}
fn row_app(r: &rusqlite::Row) -> rusqlite::Result<AppState> {
    let m: String = r.get(1)?;
    Ok(AppState {
        name: r.get(0)?,
        manifest: serde_json::from_str(&m).map_err(|e| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e)))?,
        port: r.get::<_, Option<i64>>(2)?.map(|p| p as u16),
        komodo_stack_id: r.get(3)?,
        installed_at: r.get(4)?,
    })
}

fn load_or_create_identity(path: &Path) -> Result<age::x25519::Identity> {
    use std::str::FromStr;
    if path.exists() {
        let text = std::fs::read_to_string(path)?;
        let line = text.lines().find(|l| l.starts_with("AGE-SECRET-KEY-")).ok_or_else(|| anyhow!("в {} нет age-ключа", path.display()))?;
        return age::x25519::Identity::from_str(line).map_err(|e| anyhow!("age key: {e}"));
    }
    let id = age::x25519::Identity::generate();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new().create_new(true).write(true).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    use secrecy::ExposeSecret;
    writeln!(f, "# cloudd master key — сделайте офлайн-копию (3.5)\n# public: {}\n{}", id.to_public(), id.to_string().expose_secret())?;
    tracing::warn!("создан новый мастер-ключ {} — сделайте офлайн-копию", path.display());
    Ok(id)
}

fn encrypt(recipient: &age::x25519::Recipient, plain: &[u8]) -> Result<Vec<u8>> {
    let enc = age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient)).map_err(|e| anyhow!("age: {e}"))?;
    let mut out = Vec::new();
    let mut w = enc.wrap_output(&mut out)?;
    w.write_all(plain)?;
    w.finish()?;
    Ok(out)
}
fn decrypt(identity: &age::x25519::Identity, blob: &[u8]) -> Result<Vec<u8>> {
    let dec = age::Decryptor::new(blob)?;
    let mut r = dec.decrypt(std::iter::once(identity as &dyn age::Identity))?;
    let mut out = Vec::new();
    r.read_to_end(&mut out)?;
    Ok(out)
}

/// Случайный секрет: token (32 hex) или password (24 символа [A-Za-z0-9]).
pub fn generate(kind: crate::model::GenerateKind) -> String {
    use rand::Rng as _;
    let mut rng = rand::rng();
    match kind {
        crate::model::GenerateKind::Token => (0..32).map(|_| format!("{:x}", rng.random_range(0..16u8))).collect(),
        crate::model::GenerateKind::Hex64 => (0..64).map(|_| format!("{:x}", rng.random_range(0..16u8))).collect(),
        crate::model::GenerateKind::Password => {
            const A: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789";
            (0..24).map(|_| A[rng.random_range(0..A.len())] as char).collect()
        }
    }
}

pub fn path_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}
