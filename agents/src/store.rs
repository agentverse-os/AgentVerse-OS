//! The service's own storage: SQLite with a kv table (JSON by key). What the core does not have lives here: model
//! prices, per-agent defaults, spending limits, later — routines and scratch chats. Everything about workspaces and
//! sessions is at the core.
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("каталог {}", dir.display()))?;
        let conn = Connection::open(dir.join("agents.db"))?;
        // kv holds provider keys (search, voice): the database is readable only by the owner; SQLite creates -wal/-shm with the
        // same permissions
        #[cfg(unix)]
        for f in ["agents.db", "agents.db-wal", "agents.db-shm"] {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join(f);
            if p.exists() { let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)); }
        }
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=3000; CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS runs (id TEXT PRIMARY KEY, routine TEXT, routine_name TEXT, agent TEXT NOT NULL, agent_name TEXT NOT NULL, project TEXT NOT NULL, session TEXT, trigger_kind TEXT NOT NULL, status TEXT NOT NULL, started_at INTEGER NOT NULL, ended_at INTEGER, cost_usd REAL NOT NULL DEFAULT 0, summary TEXT, reason TEXT, prompt TEXT);
            CREATE INDEX IF NOT EXISTS runs_started ON runs(started_at DESC);
            CREATE TABLE IF NOT EXISTS scheduled (id TEXT PRIMARY KEY, project TEXT NOT NULL, session TEXT NOT NULL, content TEXT NOT NULL, at INTEGER NOT NULL, status TEXT NOT NULL, created_at INTEGER NOT NULL, sent_at INTEGER, error TEXT, note TEXT);
            CREATE INDEX IF NOT EXISTS scheduled_at ON scheduled(status, at);")?;
        // a table from the slice's first build — without note: add the column (a repeated ALTER fails with "duplicate
        // column", that is normal)
        let _ = conn.execute("ALTER TABLE scheduled ADD COLUMN note TEXT", []);
        // simple chat (simplechat.rs): chats and messages; incognito is not
        // written here
        conn.execute_batch("CREATE TABLE IF NOT EXISTS chats (id TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '', route TEXT NOT NULL, model TEXT NOT NULL, system TEXT NOT NULL DEFAULT '', tools TEXT NOT NULL DEFAULT '{}', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, cost_usd REAL NOT NULL DEFAULT 0, tokens_in INTEGER NOT NULL DEFAULT 0, tokens_out INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS chat_messages (id TEXT PRIMARY KEY, chat TEXT NOT NULL, seq INTEGER NOT NULL, role TEXT NOT NULL, text TEXT NOT NULL DEFAULT '', attachments TEXT NOT NULL DEFAULT '[]', tools TEXT NOT NULL DEFAULT '[]', tokens_in INTEGER NOT NULL DEFAULT 0, tokens_out INTEGER NOT NULL DEFAULT 0, cost_usd REAL NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, error TEXT);
            CREATE INDEX IF NOT EXISTS chat_messages_chat ON chat_messages(chat, seq);")?;
        // mail between agents (routines.rs): sender, source run, chain length
        for ddl in ["ALTER TABLE runs ADD COLUMN from_agent TEXT", "ALTER TABLE runs ADD COLUMN reply_to TEXT", "ALTER TABLE runs ADD COLUMN hops INTEGER NOT NULL DEFAULT 0"] { let _ = conn.execute(ddl, []); }
        Ok(Self { conn: Mutex::new(conn) })
    }
    /// Routine runs (routines.rs): insert/update of the whole row, selection by filter, "hanging" ones, usage over a
    /// period.
    pub fn put_run(&self, r: &crate::routines::Run) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO runs (id, routine, routine_name, agent, agent_name, project, session, trigger_kind, status, started_at, ended_at, cost_usd, summary, reason, prompt, from_agent, reply_to, hops) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
             ON CONFLICT(id) DO UPDATE SET session = excluded.session, status = excluded.status, ended_at = excluded.ended_at, cost_usd = excluded.cost_usd, summary = excluded.summary, reason = excluded.reason",
            params![r.id, r.routine, r.routine_name, r.agent, r.agent_name, r.project, r.session, r.trigger, r.status, r.started_at, r.ended_at, r.cost_usd, r.summary, r.reason, r.prompt, r.from_agent, r.reply_to, r.hops])?;
        Ok(())
    }
    pub fn runs(&self, routine: Option<&str>, agent: Option<&str>, limit: u32) -> Result<Vec<crate::routines::Run>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT id, routine, routine_name, agent, agent_name, project, session, trigger_kind, status, started_at, ended_at, cost_usd, summary, reason, prompt, from_agent, reply_to, hops FROM runs WHERE (?1 IS NULL OR routine = ?1) AND (?2 IS NULL OR agent = ?2) ORDER BY started_at DESC LIMIT ?3")?;
        let rows = st.query_map(params![routine, agent, limit], row_run)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn run(&self, id: &str) -> Result<Option<crate::routines::Run>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT id, routine, routine_name, agent, agent_name, project, session, trigger_kind, status, started_at, ended_at, cost_usd, summary, reason, prompt, from_agent, reply_to, hops FROM runs WHERE id = ?1")?;
        let mut rows = st.query_map(params![id], row_run)?;
        Ok(rows.next().transpose()?)
    }
    pub fn running_runs(&self) -> Result<Vec<crate::routines::Run>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT id, routine, routine_name, agent, agent_name, project, session, trigger_kind, status, started_at, ended_at, cost_usd, summary, reason, prompt, from_agent, reply_to, hops FROM runs WHERE status = 'running' ORDER BY started_at")?;
        let rows = st.query_map([], row_run)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    /// Scheduled messages (scheduled.rs): insert, list (by session or all pending), status update, deletion.
    pub fn put_scheduled(&self, m: &crate::scheduled::Scheduled) -> Result<()> {
        self.conn.lock().unwrap().execute("INSERT INTO scheduled (id, project, session, content, at, status, created_at, sent_at, error, note) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET status = excluded.status, sent_at = excluded.sent_at, error = excluded.error, note = excluded.note",
            params![m.id, m.project, m.session, m.content, m.at, m.status, m.created_at, m.sent_at, m.error, m.note])?;
        Ok(())
    }
    pub fn scheduled(&self, project: Option<&str>, session: Option<&str>, pending_only: bool) -> Result<Vec<crate::scheduled::Scheduled>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT id, project, session, content, at, status, created_at, sent_at, error, note FROM scheduled WHERE (?1 IS NULL OR project = ?1) AND (?2 IS NULL OR session = ?2) AND (?3 = 0 OR status = 'pending') ORDER BY at")?;
        let rows = st.query_map(params![project, session, pending_only as i64], |r| Ok(crate::scheduled::Scheduled { id: r.get(0)?, project: r.get(1)?, session: r.get(2)?, content: r.get(3)?, at: r.get(4)?, status: r.get(5)?, created_at: r.get(6)?, sent_at: r.get(7)?, error: r.get(8)?, note: r.get(9)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn delete_scheduled(&self, id: &str) -> Result<bool> { Ok(self.conn.lock().unwrap().execute("DELETE FROM scheduled WHERE id = ?1", params![id])? > 0) }
    /// Take all pending ones with at <= now for sending: the status changes atomically, the next tick
    /// will not see them.
    pub fn claim_due_scheduled(&self, now: i64) -> Result<Vec<crate::scheduled::Scheduled>> {
        let due = self.scheduled(None, None, true)?.into_iter().filter(|m| m.at <= now).collect::<Vec<_>>();
        let c = self.conn.lock().unwrap();
        let mut claimed = Vec::new();
        for m in due { if c.execute("UPDATE scheduled SET status = 'sending' WHERE id = ?1 AND status = 'pending'", params![m.id])? > 0 { claimed.push(m); } }
        Ok(claimed)
    }

    pub fn delete_run(&self, id: &str) -> Result<()> { self.conn.lock().unwrap().execute("DELETE FROM runs WHERE id = ?1", params![id])?; Ok(()) }
    pub fn runs_cost_since(&self, routine: &str, since: i64) -> Result<f64> {
        let c = self.conn.lock().unwrap();
        Ok(c.query_row("SELECT COALESCE(SUM(cost_usd), 0) FROM runs WHERE routine = ?1 AND started_at >= ?2", params![routine, since], |r| r.get::<_, f64>(0))?)
    }
    /// Simple chat (simplechat.rs): chats and
    /// messages.
    pub fn chat_put(&self, c: &crate::simplechat::Chat) -> Result<()> {
        self.conn.lock().unwrap().execute("INSERT INTO chats (id, title, route, model, system, tools, created_at, updated_at, cost_usd, tokens_in, tokens_out) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(id) DO UPDATE SET title = excluded.title, route = excluded.route, model = excluded.model, system = excluded.system, tools = excluded.tools, updated_at = excluded.updated_at, cost_usd = excluded.cost_usd, tokens_in = excluded.tokens_in, tokens_out = excluded.tokens_out",
            params![c.id, c.title, c.route, c.model, c.system, serde_json::to_string(&c.tools)?, c.created_at, c.updated_at, c.cost_usd, c.tokens_in as i64, c.tokens_out as i64])?;
        Ok(())
    }
    pub fn chats(&self) -> Result<Vec<crate::simplechat::Chat>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT c.id, c.title, c.route, c.model, c.system, c.tools, c.created_at, c.updated_at, c.cost_usd, c.tokens_in, c.tokens_out, (SELECT COUNT(*) FROM chat_messages m WHERE m.chat = c.id) FROM chats c ORDER BY updated_at DESC")?;
        let rows = st.query_map([], row_chat)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn chat_get(&self, id: &str) -> Result<Option<crate::simplechat::Chat>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT c.id, c.title, c.route, c.model, c.system, c.tools, c.created_at, c.updated_at, c.cost_usd, c.tokens_in, c.tokens_out, (SELECT COUNT(*) FROM chat_messages m WHERE m.chat = c.id) FROM chats c WHERE c.id = ?1")?;
        let mut rows = st.query_map(params![id], row_chat)?;
        Ok(rows.next().transpose()?)
    }
    pub fn chat_delete(&self, id: &str) -> Result<bool> {
        let c = self.conn.lock().unwrap();
        c.execute("DELETE FROM chat_messages WHERE chat = ?1", params![id])?;
        Ok(c.execute("DELETE FROM chats WHERE id = ?1", params![id])? > 0)
    }
    pub fn chat_msg_put(&self, m: &crate::simplechat::Message) -> Result<()> {
        self.conn.lock().unwrap().execute("INSERT INTO chat_messages (id, chat, seq, role, text, attachments, tools, tokens_in, tokens_out, cost_usd, created_at, error) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(id) DO UPDATE SET text = excluded.text, tools = excluded.tools, tokens_in = excluded.tokens_in, tokens_out = excluded.tokens_out, cost_usd = excluded.cost_usd, error = excluded.error",
            params![m.id, m.chat, m.seq as i64, m.role, m.text, serde_json::to_string(&m.attachments)?, serde_json::to_string(&m.tools)?, m.tokens_in as i64, m.tokens_out as i64, m.cost_usd, m.created_at, m.error])?;
        Ok(())
    }
    /// Search across simple chat messages (slice 34): a linear pass over the last `scan` messages, all words
    /// (case-insensitive and ё/е-insensitive — SQLite lower() does not know Cyrillic); one row per chat with the last match
    /// and their count.
    pub fn chat_search(&self, words: &[String], scan: usize) -> Result<Vec<serde_json::Value>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT m.chat, m.id, m.seq, m.role, m.text, m.created_at, c.title, c.model FROM chat_messages m JOIN chats c ON c.id = m.chat ORDER BY m.created_at DESC LIMIT ?1")?;
        let rows = st.query_map(params![scan as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?, r.get::<_, String>(6)?, r.get::<_, String>(7)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut by_chat: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for (chat, id, seq, role, text, created, title, model) in rows {
            let n = crate::search::norm(&text);
            if !words.iter().all(|w| n.contains(w.as_str())) { continue }
            let ts = if created > 10_000_000_000 { created } else { created * 1000 };
            match by_chat.get_mut(&chat) {
                Some(v) => { v["count"] = serde_json::json!(v["count"].as_u64().unwrap_or(1) + 1); }
                None => { order.push(chat.clone()); by_chat.insert(chat.clone(), serde_json::json!({ "kind": "chat", "id": chat, "title": if title.trim().is_empty() { text.chars().take(48).collect::<String>() } else { title }, "model": model, "ts": ts, "mid": id, "seq": seq, "role": if role == "user" { "user" } else { "agent" }, "snippet": crate::search::snippet(&text, &words[0]), "count": 1 })); }
            }
        }
        Ok(order.into_iter().filter_map(|c| by_chat.remove(&c)).collect())
    }

    pub fn chat_msgs(&self, chat: &str) -> Result<Vec<crate::simplechat::Message>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT id, chat, seq, role, text, attachments, tools, tokens_in, tokens_out, cost_usd, created_at, error FROM chat_messages WHERE chat = ?1 ORDER BY seq")?;
        let rows = st.query_map(params![chat], |r| Ok(crate::simplechat::Message {
            id: r.get(0)?, chat: r.get(1)?, seq: r.get::<_, i64>(2)? as u64, role: r.get(3)?, text: r.get(4)?,
            attachments: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(), tools: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
            tokens_in: r.get::<_, i64>(7)? as u64, tokens_out: r.get::<_, i64>(8)? as u64, cost_usd: r.get(9)?, created_at: r.get(10)?, error: r.get(11)?,
        }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn get_kv(&self, key: &str) -> Result<Option<Value>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT value FROM kv WHERE key = ?1")?;
        let mut rows = st.query(params![key])?;
        Ok(match rows.next()? { Some(r) => serde_json::from_str(&r.get::<_, String>(0)?).ok(), None => None })
    }
    pub fn put_kv(&self, key: &str, value: &Value) -> Result<()> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
        self.conn.lock().unwrap().execute("INSERT INTO kv (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at", params![key, value.to_string(), now])?;
        Ok(())
    }
}

fn row_chat(r: &rusqlite::Row) -> rusqlite::Result<crate::simplechat::Chat> {
    Ok(crate::simplechat::Chat {
        id: r.get(0)?, title: r.get(1)?, route: r.get(2)?, model: r.get(3)?, system: r.get(4)?, tools: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
        incognito: false, created_at: r.get(6)?, updated_at: r.get(7)?, cost_usd: r.get(8)?, tokens_in: r.get::<_, i64>(9)? as u64, tokens_out: r.get::<_, i64>(10)? as u64, messages: r.get::<_, i64>(11)? as u64,
    })
}

fn row_run(r: &rusqlite::Row) -> rusqlite::Result<crate::routines::Run> {
    Ok(crate::routines::Run {
        id: r.get(0)?, routine: r.get(1)?, routine_name: r.get(2)?, agent: r.get(3)?, agent_name: r.get(4)?, project: r.get(5)?, session: r.get(6)?, trigger: r.get(7)?, status: r.get(8)?,
        started_at: r.get(9)?, ended_at: r.get(10)?, cost_usd: r.get(11)?, summary: r.get(12)?, reason: r.get(13)?, prompt: r.get(14)?,
        from_agent: r.get(15)?, reply_to: r.get(16)?, hops: r.get::<_, i64>(17).unwrap_or(0).max(0) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kv_roundtrip() {
        let dir = std::env::temp_dir().join(format!("agents-store-{}", std::process::id()));
        let s = Store::open(&dir).unwrap();
        assert!(s.get_kv("x").unwrap().is_none());
        s.put_kv("x", &serde_json::json!({ "a": 1 })).unwrap();
        s.put_kv("x", &serde_json::json!({ "a": 2 })).unwrap();
        assert_eq!(s.get_kv("x").unwrap().unwrap()["a"], 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
