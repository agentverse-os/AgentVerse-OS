//! Состояние ядра в SQLite (`tank/core`) и секрет-стор: зашифрованные age-blob'ы под мастер-ключом хоста (2.9, 3.5).

use crate::model::{AppState, Grant, ProjectState, UsageBucket, UsageRow, UsageSummary, UsageTotals};
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
CREATE TABLE IF NOT EXISTS llm_usage (
  project TEXT NOT NULL, session TEXT NOT NULL, seq INTEGER NOT NULL, ts INTEGER NOT NULL, agent TEXT NOT NULL, model TEXT,
  input INTEGER NOT NULL, output INTEGER NOT NULL, cache_read INTEGER NOT NULL, cache_create INTEGER NOT NULL,
  cost_usd REAL NOT NULL, duration_ms INTEGER, num_turns INTEGER, source TEXT NOT NULL, PRIMARY KEY (project, session, seq));
CREATE INDEX IF NOT EXISTS llm_usage_ts ON llm_usage(ts);
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
    // ---- книга расходов LLM (ADR-006, решение 7) ----

    /// INSERT OR IGNORE: повторный опрос agentd не задваивает строки. true — строка новая.
    /// Одноразовый пересчёт книги расходов: до agentd с дельтами в `cost_usd` строк лежал накопительный `total_cost_usd`
    /// сессии Claude Code (он растёт от хода к ходу), и суммы раздувались; превращаем каждую строку в разницу с предыдущей
    /// строкой той же сессии. Отметка kv `usage.cost_deltas`, чтобы не пересчитывать новые, уже правильные строки.
    pub fn fix_cumulative_costs(&self) -> Result<usize> {
        if self.get_kv("usage.cost_deltas")?.is_some() { return Ok(0); }
        let fixed = self.with(|c| {
            let mut st = c.prepare("SELECT project, session, seq, cost_usd FROM llm_usage ORDER BY project, session, seq")?;
            let rows: Vec<(String, String, i64, f64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let mut prev: Option<(String, String, f64)> = None;
            let mut fixed = 0usize;
            let tx = c.unchecked_transaction()?;
            for (p, s, seq, cost) in rows {
                let last = match &prev { Some((pp, ss, l)) if pp == &p && ss == &s => *l, _ => 0.0 };
                let delta = if cost >= last { cost - last } else { cost };
                if (delta - cost).abs() > 1e-9 { tx.execute("UPDATE llm_usage SET cost_usd = ?1 WHERE project = ?2 AND session = ?3 AND seq = ?4", params![delta, p, s, seq])?; fixed += 1; }
                prev = Some((p, s, cost));
            }
            tx.commit()?;
            Ok(fixed)
        })?;
        self.put_kv("usage.cost_deltas", &serde_json::json!(crate::agents::now_ms()))?;
        Ok(fixed)
    }

    /// У беседы, подхваченной с диска до перехода на дельты, первая строка книги — накопительный итог CLI, включающий ходы
    /// до подхвата: вычитаем базу (`usage_base.cost_usd` из cost-state) один раз, только у строк старше пересчёта.
    pub fn deduct_usage_base(&self, project: &str, session: &str, base: f64) -> Result<bool> {
        // v2: первая версия ставила отметку и при неверном времени пересчёта (`1`), те отметки не считаются
        let key = format!("usage.base_fixed2.{session}");
        if self.get_kv(&key)?.is_some() { return Ok(false); }
        // время перехода на дельты: первая версия отметки хранила `1`, тогда берём/заводим отдельную отметку времени
        let mut migrated_at = self.get_kv("usage.cost_deltas")?.and_then(|v| v.as_u64()).unwrap_or(0) as i64;
        if migrated_at < 1_000_000 {
            migrated_at = match self.get_kv("usage.cost_deltas_at")?.and_then(|v| v.as_u64()) { Some(t) => t as i64, None => { let t = crate::agents::now_ms(); self.put_kv("usage.cost_deltas_at", &serde_json::json!(t))?; t as i64 } };
        }
        let changed = self.with(|c| {
            let first: Option<(i64, i64, f64)> = c.query_row("SELECT seq, ts, cost_usd FROM llm_usage WHERE project = ?1 AND session = ?2 ORDER BY seq LIMIT 1", params![project, session], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
            match first {
                Some((seq, ts, cost)) if ts < migrated_at && cost > base => {
                    c.execute("UPDATE llm_usage SET cost_usd = ?1 WHERE project = ?2 AND session = ?3 AND seq = ?4", params![cost - base, project, session, seq])?;
                    Ok(true)
                }
                _ => Ok(false),
            }
        })?;
        self.put_kv(&key, &serde_json::json!(1))?;
        Ok(changed)
    }

    pub fn insert_usage(&self, r: &UsageRow) -> Result<bool> {
        self.with(|c| {
            let n = c.execute(
                "INSERT OR IGNORE INTO llm_usage(project, session, seq, ts, agent, model, input, output, cache_read, cache_create, cost_usd, duration_ms, num_turns, source) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![r.project, r.session, r.seq as i64, r.ts as i64, r.agent, r.model, r.input as i64, r.output as i64, r.cache_read as i64, r.cache_create as i64, r.cost_usd, r.duration_ms.map(|d| d as i64), r.num_turns.map(|d| d as i64), r.source],
            )?;
            Ok(n == 1)
        })
    }

    /// Самая поздняя запись проекта — с неё продолжается опрос `/v1/usage?since=`.
    pub fn usage_last_ts(&self, project: &str) -> Result<u64> {
        self.with(|c| Ok(c.query_row("SELECT COALESCE(MAX(ts), 0) FROM llm_usage WHERE project = ?1", params![project], |r| r.get::<_, i64>(0))? as u64))
    }

    /// Строки книги расходов с `since_ts` (мс), по времени (срез 34: экспорт CSV).
    pub fn usage_rows(&self, since_ts: u64) -> Result<Vec<UsageRow>> {
        let c = self.conn.lock().map_err(|_| anyhow!("store mutex poisoned"))?;
        let mut st = c.prepare("SELECT project, session, seq, ts, agent, model, input, output, cache_read, cache_create, cost_usd, duration_ms, num_turns, source FROM llm_usage WHERE ts >= ?1 ORDER BY ts")?;
        let rows = st.query_map(params![since_ts as i64], |r| Ok(UsageRow {
            project: r.get(0)?, session: r.get(1)?, seq: r.get::<_, i64>(2)? as u64, ts: r.get::<_, i64>(3)? as u64, agent: r.get(4)?, model: r.get(5)?,
            input: r.get::<_, i64>(6)? as u64, output: r.get::<_, i64>(7)? as u64, cache_read: r.get::<_, i64>(8)? as u64, cache_create: r.get::<_, i64>(9)? as u64,
            cost_usd: r.get(10)?, duration_ms: r.get::<_, Option<i64>>(11)?.map(|d| d as u64), num_turns: r.get::<_, Option<i64>>(12)?.map(|n| n as u64), source: r.get(13)?,
        }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn usage_summary(&self, since_ts: u64) -> Result<UsageSummary> {
        fn totals(r: &rusqlite::Row, o: usize) -> rusqlite::Result<UsageTotals> {
            Ok(UsageTotals {
                cost_usd: r.get::<_, Option<f64>>(o)?.unwrap_or(0.0),
                input: r.get::<_, Option<i64>>(o + 1)?.unwrap_or(0) as u64,
                output: r.get::<_, Option<i64>>(o + 2)?.unwrap_or(0) as u64,
                cache_read: r.get::<_, Option<i64>>(o + 3)?.unwrap_or(0) as u64,
                cache_create: r.get::<_, Option<i64>>(o + 4)?.unwrap_or(0) as u64,
                turns: r.get::<_, Option<i64>>(o + 5)?.unwrap_or(0) as u64,
            })
        }
        fn bucket(r: &rusqlite::Row) -> rusqlite::Result<UsageBucket> {
            Ok(UsageBucket { key: r.get::<_, Option<String>>(0)?.unwrap_or_else(|| "?".into()), totals: totals(r, 1)? })
        }
        const AGG: &str = "SUM(cost_usd), SUM(input), SUM(output), SUM(cache_read), SUM(cache_create), COUNT(*)";
        self.with(|c| {
            let total = c.query_row(&format!("SELECT {AGG} FROM llm_usage WHERE ts > ?1"), params![since_ts as i64], |r| totals(r, 0))?;
            let group = |expr: &str, order: &str| -> Result<Vec<UsageBucket>> {
                let mut st = c.prepare(&format!("SELECT {expr} AS k, {AGG} FROM llm_usage WHERE ts > ?1 GROUP BY k ORDER BY {order}"))?;
                let rows = st.query_map(params![since_ts as i64], bucket)?.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            };
            let by_project = group("project", "SUM(cost_usd) DESC")?;
            let by_model = group("model", "SUM(cost_usd) DESC")?;
            let by_day = group("strftime('%Y-%m-%d', ts / 1000, 'unixepoch')", "k ASC")?;
            Ok(UsageSummary { since_ts, total, by_project, by_model, by_day })
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
