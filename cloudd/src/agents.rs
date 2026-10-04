//! Agent Runtime в ядре (ADR-006): ядро — единственная точка входа, адаптеры agentd в workspace'ах — источник фактов.
//! Раз в 10 с опрашиваются все запущенные workspace'ы: записи расхода (`/v1/usage`) складываются в книгу расходов
//! (`llm_usage`, дедупликация по проекту, сессии и seq), список сессий — в кэш для карточек проектов; переходы
//! «работал → ждёт решения» и «работал → закончил» становятся событиями ленты, а если Desktop «ушёл» (не опрашивал
//! статус видимой вкладкой полторы минуты) — push в ntfy (правило Orca: телефон только когда человека нет у экрана).
use crate::model::{AgentSessionBrief, AgentsSummary, UsageRow};
use crate::service::Service;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::Duration;

pub const POLL_SECS: u64 = 10;
/// Desktop считается «ушедшим», если видимая вкладка не опрашивала статус дольше этого.
pub const AWAY_MS: u64 = 90_000;
pub const NTFY_TOPIC_KEY: &str = "agents.ntfy_topic";
pub const DEFAULT_TOPIC: &str = "cloudos";
/// Системный пользователь ntfy ядра: секреты `app:ntfy` → `CLOUDOS_TOKEN` (push) и `CLOUDOS_PASSWORD` (подписка с телефона).
pub const NTFY_USER: &str = "cloudos";

#[derive(Default)]
pub struct AgentsState {
    pub cache: RwLock<HashMap<String, AgentsSummary>>,
    presence_seen_ms: AtomicU64,
}

impl AgentsState {
    pub fn touch_presence(&self) {
        self.presence_seen_ms.store(now_ms(), Ordering::Relaxed);
    }

    pub fn desktop_away(&self) -> bool {
        now_ms().saturating_sub(self.presence_seen_ms.load(Ordering::Relaxed)) > AWAY_MS
    }

    pub fn presence_seen_ms(&self) -> u64 {
        self.presence_seen_ms.load(Ordering::Relaxed)
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Запись расхода из ленты agentd → строка книги; модель — первая из `model_usage`.
pub fn usage_row(project: &str, r: &Value) -> Option<UsageRow> {
    let u = |k: &str| r.pointer(&format!("/usage/{k}")).and_then(|x| x.as_u64()).unwrap_or(0);
    Some(UsageRow {
        project: project.to_string(),
        session: r.get("session")?.as_str()?.to_string(),
        seq: r.get("seq")?.as_u64()?,
        ts: r.get("ts")?.as_u64()?,
        agent: r.get("agent").and_then(|a| a.as_str()).unwrap_or("claude").to_string(),
        // ключ model_usage у сессий с меткой `[1m]` — «claude-fable-5-1[1m]»: в книге — канонический id (цены и группировка по модели)
        // модель хода — из `result.model` (последний ответ, agentd), иначе первая запись model_usage (накопительна, ключ с `[1m]`) по canonicalModel
        model: r.get("model").and_then(|m| m.as_str()).filter(|m| !m.is_empty() && !m.starts_with('<')).map(|m| m.replace("[1m]", "").replace("[1M]", ""))
            .or_else(|| r.get("model_usage").and_then(|m| m.as_object()).and_then(|m| m.iter().next().map(|(k, v)| v.get("canonicalModel").and_then(|c| c.as_str()).map(str::to_string).unwrap_or_else(|| k.replace("[1m]", "").replace("[1M]", ""))))),
        input: u("input_tokens"),
        output: u("output_tokens"),
        cache_read: u("cache_read_input_tokens"),
        cache_create: u("cache_creation_input_tokens"),
        // agentd с дельтами шлёт `cost_usd` (стоимость хода); `total_cost_usd` — накопительный итог сессии, фолбэк для старых adapter'ов
        cost_usd: r.get("cost_usd").and_then(|c| c.as_f64()).or_else(|| r.get("total_cost_usd").and_then(|c| c.as_f64())).unwrap_or(0.0),
        duration_ms: r.get("duration_ms").and_then(|d| d.as_u64()),
        num_turns: r.get("num_turns").and_then(|d| d.as_u64()),
        source: "agentd".into(),
    })
}

pub fn summarize(list: &[Value]) -> AgentsSummary {
    let sessions: Vec<AgentSessionBrief> = list
        .iter()
        .map(|s| AgentSessionBrief {
            id: s.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            title: s.get("title").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            agent: s.get("agent").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            status: s.get("status").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
            alive: s.get("alive").and_then(|v| v.as_bool()).unwrap_or(false),
            pending: s.get("pending").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            last_text: s.get("last_text").and_then(|v| v.as_str()).map(str::to_string),
            cost_usd: s.pointer("/usage/cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0),
        })
        .collect();
    AgentsSummary {
        total: sessions.len(),
        working: sessions.iter().filter(|s| s.status == "working" || s.status == "starting").count(),
        waiting: sessions.iter().filter(|s| s.status == "waiting").count(),
        sessions,
        updated_at: now_ms(),
    }
}

impl Service {
    pub fn start_agents_poller(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            loop {
                if let Err(e) = me.poll_agents().await {
                    tracing::warn!("опрос агентов: {e:#}");
                }
                tokio::time::sleep(Duration::from_secs(POLL_SECS)).await;
            }
        });
    }

    /// Один проход по всем проектам с запущенным workspace: расход → книга, сессии → кэш и переходы.
    pub async fn poll_agents(&self) -> Result<()> {
        let running: HashSet<String> = self.incus.instances().await?.into_iter().filter(|i| i.status == "Running").map(|i| i.name).collect();
        for st in self.store.list_projects()? {
            let inst = self.workspace_instance(&st.name).await;
            if !running.contains(&inst) {
                self.agents.cache.write().unwrap().remove(&st.name);
                continue;
            }
            let Ok(client) = self.agentd_client(&st.name).await else { continue };
            let since = self.store.usage_last_ts(&st.name)?;
            match client.get(&format!("/v1/usage?since={since}")).await {
                Ok(Value::Array(rows)) => {
                    let mut added = 0;
                    for r in &rows {
                        if let Some(row) = usage_row(&st.name, r) {
                            if self.store.insert_usage(&row)? { added += 1; }
                        }
                    }
                    if added > 0 { tracing::debug!("{}: {added} записей расхода", st.name); }
                }
                Ok(_) => {}
                Err(e) => tracing::debug!("{}: usage: {e:#}", st.name),
            }
            match client.get("/v1/sessions").await {
                Ok(Value::Array(list)) => {
                    // первая строка книги у подхваченных бесед (до перехода на дельты) включала ходы до подхвата — вычесть базу один раз
                    for s in &list {
                        if s.get("imported").and_then(|v| v.as_bool()) != Some(true) { continue }
                        let (Some(id), Some(base)) = (s.get("id").and_then(|v| v.as_str()), s.pointer("/usage_base/cost_usd").and_then(|v| v.as_f64()).filter(|b| *b > 0.0)) else { continue };
                        match self.store.deduct_usage_base(&st.name, id, base) { Ok(true) => tracing::info!("{}: книга расходов: у беседы {} из первой строки вычтена стоимость до подхвата (${base:.2})", st.name, &id[..8]), Ok(false) => {} Err(e) => tracing::debug!("книга: {e:#}") }
                    }
                    let cur = summarize(&list);
                    let prev = self.agents.cache.write().unwrap().insert(st.name.clone(), cur.clone());
                    self.agent_transitions(&st.name, prev.as_ref(), &cur).await;
                }
                Ok(_) => {}
                Err(e) => tracing::debug!("{}: sessions: {e:#}", st.name),
            }
        }
        Ok(())
    }

    async fn agent_transitions(&self, project: &str, prev: Option<&AgentsSummary>, cur: &AgentsSummary) {
        let Some(prev) = prev else { return };
        for s in &cur.sessions {
            // сессии, которой не было в прошлом снимке, приписываем статус `new`: «появилась уже ждущей» — тоже переход
            let before = prev.sessions.iter().find(|x| x.id == s.id).map(|p| p.status.as_str()).unwrap_or("new");
            if before == s.status { continue }
            match s.status.as_str() {
                "waiting" => self.notify_agent(project, "агент ждёт решения", &format!("{project} · {}: нужно подтверждение или ответ", s.title), "hourglass").await,
                "idle" if before == "working" => {
                    let tail = s.last_text.as_deref().unwrap_or("").chars().take(140).collect::<String>();
                    self.notify_agent(project, "агент закончил", &format!("{project} · {}{}", s.title, if tail.is_empty() { String::new() } else { format!(": {tail}") }), "white_check_mark").await
                }
                "error" => self.notify_agent(project, "агент упал", &format!("{project} · {}", s.title), "x").await,
                _ => {}
            }
        }
    }

    /// Событие в ленту всегда; на телефон — только если Desktop «ушёл».
    pub async fn notify_agent(&self, project: &str, title: &str, body: &str, tag: &str) {
        let _ = self.store.event("agent", project, &format!("{title}: {}", body.trim_start_matches(&format!("{project} · "))));
        if self.agents.desktop_away() {
            if let Err(e) = self.ntfy_push(title, body, tag).await {
                tracing::debug!("ntfy: {e:#}");
            }
        }
    }

    /// Токен системного пользователя ntfy: как хук `notify`, но для ядра — пользователь `cloudos`, права rw на системную тему,
    /// токен и пароль в секрет-сторе (пароль — чтобы владелец подписался на тему в приложении ntfy на телефоне).
    async fn ntfy_token(&self, container: &str, topic: &str) -> Result<String> {
        let scope = "app:ntfy";
        if let Some(t) = self.store.get_secret(scope, "CLOUDOS_TOKEN")? {
            return Ok(t);
        }
        let password = match self.store.get_secret(scope, "CLOUDOS_PASSWORD")? {
            Some(p) => p,
            None => {
                use rand::RngCore;
                let mut b = [0u8; 12];
                rand::rng().fill_bytes(&mut b);
                let p: String = b.iter().map(|x| format!("{x:02x}")).collect();
                self.store.put_secret(scope, "CLOUDOS_PASSWORD", &p)?;
                p
            }
        };
        let users = self.docker.exec(container, vec!["ntfy", "user", "list"]).await.unwrap_or_default();
        if !users.contains(&format!("user {NTFY_USER} ")) {
            self.docker.exec(container, vec!["sh", "-c", &format!("NTFY_PASSWORD={password} ntfy user add --role=user {NTFY_USER}")]).await?;
        }
        self.docker.exec(container, vec!["ntfy", "access", NTFY_USER, topic, "rw"]).await?;
        let out = self.docker.exec(container, vec!["ntfy", "token", "add", "--label", "cloudos system", NTFY_USER]).await?;
        let token = out.split(|c: char| c.is_whitespace() || c == ',' || c == ')').find(|w| w.starts_with("tk_")).ok_or_else(|| anyhow!("ntfy не выдал токен: {}", out.trim()))?.to_string();
        self.store.put_secret(scope, "CLOUDOS_TOKEN", &token)?;
        let _ = self.store.event("agent", "ntfy", &format!("создан пользователь {NTFY_USER} и тема {topic} для уведомлений ядра"));
        Ok(token)
    }

    /// Данные подписки на уведомления ядра с телефона: сервер ntfy, тема, пользователь и пароль (заводятся при первом обращении).
    pub async fn notify_settings(&self) -> Result<Value> {
        let topic = self.store.get_kv(NTFY_TOPIC_KEY)?.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| DEFAULT_TOPIC.into());
        let Some(app) = self.store.list_apps()?.into_iter().find(|a| a.name == "ntfy") else {
            return Ok(serde_json::json!({ "app": Value::Null, "server": Value::Null, "topic": topic, "user": NTFY_USER, "password": Value::Null, "token": Value::Null }));
        };
        let container = format!("{}-{}-1", app.stack_name(), app.manifest.endpoint.service);
        let token = self.ntfy_token(&container, &topic).await?;
        let password = self.store.get_secret("app:ntfy", "CLOUDOS_PASSWORD")?;
        Ok(serde_json::json!({ "app": app.name, "server": self.app_url(&app), "topic": topic, "user": NTFY_USER, "password": password, "token": token }))
    }

    /// Push через установленное приложение ntfy: контейнер главного сервиса в сети приложения, тема из kv
    /// (`agents.ntfy_topic`, по умолчанию `cloudos`); клик открывает окно «Агенты» отдельным приложением.
    pub async fn ntfy_push(&self, title: &str, body: &str, tag: &str) -> Result<()> {
        let app = self.store.list_apps()?.into_iter().find(|a| a.name == "ntfy").ok_or_else(|| anyhow!("ntfy не установлен"))?;
        let container = format!("{}-{}-1", app.stack_name(), app.manifest.endpoint.service);
        let ip = self.docker.container_ip(&container, &app.network()).await?;
        let topic = self.store.get_kv(NTFY_TOPIC_KEY)?.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| DEFAULT_TOPIC.into());
        let token = self.ntfy_token(&container, &topic).await?;
        let click = format!("https://{}/?app=agents", self.cfg.edge_host);
        let http = reqwest::Client::builder().timeout(Duration::from_secs(8)).build()?;
        let resp = http
            .post(format!("http://{ip}:80/{topic}"))
            .bearer_auth(&token)
            .header("Title", title)
            .header("Tags", tag)
            .header("Click", click)
            .header("Priority", "high")
            .body(body.to_string())
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("ntfy ответил {}", resp.status());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_row_from_agentd_record() {
        let r = json!({ "session": "s1", "agent": "claude", "ts": 1790000000000u64, "seq": 7, "usage": { "input_tokens": 2, "output_tokens": 10, "cache_read_input_tokens": 31400, "cache_creation_input_tokens": 0 },
            "model_usage": { "claude-fable-5-1": { "costUSD": 0.05 } }, "total_cost_usd": 0.0526, "duration_ms": 2300, "num_turns": 1 });
        let row = usage_row("alpha", &r).unwrap();
        assert_eq!((row.project.as_str(), row.session.as_str(), row.seq, row.input, row.output, row.cache_read), ("alpha", "s1", 7, 2, 10, 31400));
        assert_eq!(row.model.as_deref(), Some("claude-fable-5-1"));
        assert!((row.cost_usd - 0.0526).abs() < 1e-9);
        assert!(usage_row("x", &json!({ "ts": 1 })).is_none(), "без session/seq запись не строится");
    }

    #[test]
    fn summary_counts_statuses() {
        let s = summarize(&[json!({ "id": "a", "title": "Claude", "status": "working", "alive": true, "pending": 0 }), json!({ "id": "b", "title": "Codex", "status": "waiting", "alive": true, "pending": 1 }), json!({ "id": "c", "status": "exited", "alive": false })]);
        assert_eq!((s.total, s.working, s.waiting), (3, 1, 1));
        assert_eq!(s.sessions[1].pending, 1);
    }

    #[test]
    fn presence() {
        let st = AgentsState::default();
        assert!(st.desktop_away(), "до первого опроса Desktop считается ушедшим");
        st.touch_presence();
        assert!(!st.desktop_away());
    }
}
