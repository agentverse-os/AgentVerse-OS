//! Провайдер Codex (ADR-006): `codex app-server` — JSON-RPC 2.0 построчно через stdio. Формы взяты из схемы, которую
//! печатает сам CLI (`codex app-server generate-json-schema`): `initialize` → `thread/start|resume` → `turn/start`;
//! уведомления `item/started|completed`, `turn/completed`, `thread/tokenUsage/updated`; запросы к клиенту
//! `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval`,
//! `item/tool/requestUserInput`. Всё это переводится в те же виды записей журнала, что у Claude, — карточка Chat одна.
use crate::session::{Pending, Session, Status};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Состояние треда Codex внутри сессии.
#[derive(Default)]
pub struct CodexState {
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub last_usage: Option<Value>,
    /// Окно контекста модели (из `thread/tokenUsage/updated` / `task_started`)
    pub context_window: Option<u64>,
    /// Каталог моделей app-server (`model/list`): displayName, уровни reasoning с описаниями, service tiers
    pub models: Vec<Value>,
    pub last_text: String,
    pub turn_started_ms: u64,
    /// Метка запуска процесса: id запросов app-server начинаются с нуля при каждом старте, а журнал сессии — один на все
    /// запуски, поэтому наружу уходит `cx<метка>-<id>`.
    pub epoch: u64,
    /// Имя инструмента по id элемента — чтобы `tool_result` знал, к какому вызову относится.
    pub tools: std::collections::HashMap<String, String>,
}

pub fn args() -> Vec<String> {
    vec!["codex".into(), "app-server".into()]
}

/// Режим разрешений сессии → политика одобрений и песочница Codex. Принимаются и имена Claude, и родные Codex.
pub fn approval(mode: Option<&str>) -> (&'static str, &'static str) {
    match mode.unwrap_or("default") {
        "bypassPermissions" | "never" | "danger-full-access" => ("never", "danger-full-access"),
        "dontAsk" => ("never", "workspace-write"),
        "untrusted" => ("untrusted", "workspace-write"),
        "plan" | "read-only" => ("untrusted", "read-only"),
        _ => ("on-request", "workspace-write"),
    }
}

/// `export K='v'` из env.sh gate'а (его читает login-оболочка процесса; здесь он нужен, чтобы знать маршрут заранее).
pub fn parse_env_sh(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("export ") else { continue };
        let Some((k, v)) = rest.split_once('=') else { continue };
        let v = v.trim();
        let v = if (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2) || (v.starts_with('"') && v.ends_with('"') && v.len() >= 2) { &v[1..v.len() - 1] } else { v };
        out.insert(k.trim().to_string(), v.replace("'\\''", "'"));
    }
    out
}

/// Итоговое окружение процесса: env.sh gate'а, поверх — переменные аккаунта, минус `unset`.
/// Окружение workspace'а без аккаунта: env.sh gate'а (login-оболочка читает его же).
pub fn workspace_env() -> HashMap<String, String> {
    std::fs::read_to_string("/etc/cloudos/env.sh").map(|t| parse_env_sh(&t)).unwrap_or_default()
}

pub fn effective_env(meta: &crate::session::Meta) -> HashMap<String, String> {
    let mut env = std::fs::read_to_string("/etc/cloudos/env.sh").map(|t| parse_env_sh(&t)).unwrap_or_default();
    for (k, v) in &meta.env { env.insert(k.clone(), v.clone()); }
    for k in &meta.unset { env.remove(k); }
    env
}

/// Маршрут к модели (ADR-006, решение 5): при `CODEX_HOME` (файлы входа аккаунта) Codex ходит своей подпиской; иначе при
/// `OPENAI_BASE_URL` — gate проекта или ключ аккаунта как провайдер `cloudos` через Responses API (Codex 0.156 сам эти
/// переменные не читает — переопределяем конфигурацию в `thread/start`).
pub fn gate_config(env: &HashMap<String, String>) -> Option<Value> {
    if env.contains_key("CODEX_HOME") { return None }
    let base = env.get("OPENAI_BASE_URL")?;
    Some(json!({ "model_provider": "cloudos", "model_providers": { "cloudos": { "name": "AgentVerse gate", "base_url": base, "env_key": "OPENAI_API_KEY", "wire_api": "responses" } } }))
}

/// Аргументы родного TUI на том же треде (терминальный режим): `codex resume <тред>`; провайдер gate'а — теми же ключами,
/// что в `thread/start`, но через `-c` (значения в TOML-кавычках); без треда (сессия ещё не стартовала) — новая беседа.
/// Режим: `bypass` → `--dangerously-bypass-approvals-and-sandbox`, иначе `-a`/`-s` из режима сессии (у TUI `-a` только
/// on-request | never — untrusted сводится к on-request).
pub fn tui_args(meta: &crate::session::Meta, gate: Option<&Value>, model: Option<&str>, bypass: bool) -> Vec<String> {
    let mut a = vec!["codex".to_string()];
    if let Some(cfg) = gate {
        a.push("-c".into()); a.push(format!("model_provider={:?}", cfg["model_provider"].as_str().unwrap_or("cloudos")));
        if let Some(p) = cfg.pointer("/model_providers/cloudos").and_then(|p| p.as_object()) {
            let mut keys: Vec<_> = p.keys().collect(); keys.sort();
            for k in keys { if let Some(v) = p[k].as_str() { a.push("-c".into()); a.push(format!("model_providers.cloudos.{k}={v:?}")); } }
        }
    }
    if let Some(tid) = &meta.provider_session_id { a.push("resume".into()); a.push(tid.clone()); }
    if let Some(m) = model { a.push("-m".into()); a.push(m.to_string()); }
    if bypass { a.push("--dangerously-bypass-approvals-and-sandbox".into()); } else {
        let (policy, sandbox) = approval(meta.permission_mode.as_deref());
        a.push("-a".into()); a.push(if policy == "never" { "never" } else { "on-request" }.into());
        a.push("-s".into()); a.push(sandbox.into());
    }
    a
}

/// Какую модель gate'а брать, когда ни сессия, ни `CODEX_MODEL` её не задали: сначала codex-модели, потом «голые» gpt-*
/// (без префикса вендора), потом любая gpt-*; иначе — умолчание CLI, и ошибка маршрута всплывёт в первом ходе.
pub fn rank_models(ids: &[String]) -> Option<String> {
    let pick = |f: &dyn Fn(&str) -> bool| { let mut v: Vec<&String> = ids.iter().filter(|s| f(s)).collect(); v.sort(); v.first().map(|s| s.to_string()) };
    pick(&|s| s.contains("codex") && !s.contains('/')).or_else(|| pick(&|s| s.starts_with("gpt-"))).or_else(|| pick(&|s| s.contains("codex"))).or_else(|| pick(&|s| s.contains("gpt-")))
}

pub(crate) async fn pick_gate_model(env: &HashMap<String, String>) -> Option<String> { rank_models(&gate_model_ids(env).await) }

/// Модели gate'а (`/v1/models` с ключом); пусто — недоступен или не gate.
pub(crate) async fn gate_model_ids(env: &HashMap<String, String>) -> Vec<String> {
    let Some(base) = env.get("OPENAI_BASE_URL") else { return Vec::new() };
    let key = env.get("OPENAI_API_KEY").cloned().unwrap_or_default();
    let out = tokio::time::timeout(std::time::Duration::from_secs(15), tokio::process::Command::new("curl").args(["-sS", "-m", "10", "-H", &format!("Authorization: Bearer {key}"), &format!("{}/models", base.trim_end_matches('/'))]).output()).await;
    let Ok(Ok(out)) = out else { return Vec::new() };
    let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) else { return Vec::new() };
    v.get("data").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect()).unwrap_or_default()
}

/// Каталог входа Codex сессии: CODEX_HOME аккаунта или штатный ~/.codex.
fn codex_home(env: &HashMap<String, String>) -> std::path::PathBuf {
    env.get("CODEX_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into())).join(".codex"))
}

/// Rollout треда (`sessions/ГГГГ/ММ/ДД/rollout-…-<tid>.jsonl`) в каталоге входа: (файл, путь относительно sessions/).
fn find_rollout_in(home: &std::path::Path, tid: &str) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let root = home.join("sessions");
    fn walk(dir: &std::path::Path, depth: u8, tid: &str, out: &mut Option<std::path::PathBuf>) {
        if out.is_some() || depth > 4 { return; }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { walk(&p, depth + 1, tid, out); }
            else if p.file_name().and_then(|n| n.to_str()).map(|n| n.contains(tid) && n.ends_with(".jsonl")).unwrap_or(false) { *out = Some(p); return; }
        }
    }
    let mut found = None; walk(&root, 0, tid, &mut found);
    found.map(|f| { let rel = f.strip_prefix(&root).map(|r| r.to_path_buf()).unwrap_or_else(|_| f.file_name().map(std::path::PathBuf::from).unwrap_or_default()); (f, rel) })
}

/// Тред переехал в другой каталог входа (смена аккаунта: CODEX_HOME другой): найти rollout в прежних каталогах и скопировать
/// в текущий, чтобы `thread/resume` продолжил ту же историю. Возвращает true, если rollout теперь есть в текущем каталоге.
pub(crate) fn bring_rollout(env: &HashMap<String, String>, tid: &str) -> bool {
    let home = codex_home(env);
    if find_rollout_in(&home, tid).is_some() { return true; }
    let base = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into()));
    let mut candidates = vec![base.join(".codex")];
    if let Ok(rd) = std::fs::read_dir(base.join(".cloudos/accounts")) { for e in rd.flatten() { if e.path().is_dir() { candidates.push(e.path()); } } }
    for c in candidates {
        if c == home { continue; }
        if let Some((src, rel)) = find_rollout_in(&c, tid) {
            let dst = home.join("sessions").join(&rel);
            if let Some(parent) = dst.parent() { let _ = std::fs::create_dir_all(parent); }
            match std::fs::copy(&src, &dst) { Ok(_) => { tracing::info!("codex: rollout {} перенесён из {} в {}", tid, c.display(), home.display()); return true; } Err(e) => tracing::warn!("codex: копия rollout {}: {e}", src.display()) }
        }
    }
    false
}

/// Как называть элемент Codex в терминах карточки: команда — Bash, правка — Edit, MCP — сервер.инструмент.
fn tool_call(item: &Value) -> Option<(String, Value)> {
    let t = item.get("type").and_then(|t| t.as_str())?;
    match t {
        "commandExecution" => Some(("Bash".into(), json!({ "command": item.get("command"), "cwd": item.get("cwd") }))),
        "fileChange" => {
            let paths: Vec<Value> = item.get("changes").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|c| c.get("path").cloned()).collect()).unwrap_or_default();
            Some(("Edit".into(), json!({ "file_path": paths.first().and_then(|p| p.as_str()).unwrap_or(""), "changes": paths })))
        }
        "mcpToolCall" => Some((format!("{}.{}", item.get("server").and_then(|s| s.as_str()).unwrap_or("mcp"), item.get("tool").and_then(|s| s.as_str()).unwrap_or("tool")), item.get("arguments").cloned().unwrap_or(Value::Null))),
        "dynamicToolCall" => Some((item.get("tool").and_then(|s| s.as_str()).unwrap_or("tool").to_string(), item.get("arguments").cloned().unwrap_or(Value::Null))),
        _ => None,
    }
}

fn tool_output(item: &Value) -> (String, bool) {
    match item.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "commandExecution" => {
            let out = item.get("aggregatedOutput").and_then(|o| o.as_str()).unwrap_or("").to_string();
            let status = item.get("status").and_then(|s| s.as_str()).unwrap_or("");
            let code = item.get("exitCode").and_then(|c| c.as_i64());
            (if out.is_empty() { format!("(код выхода {})", code.unwrap_or(0)) } else { out }, status == "failed" || status == "declined" || code.map(|c| c != 0).unwrap_or(false))
        }
        "fileChange" => {
            let status = item.get("status").and_then(|s| s.as_str()).unwrap_or("");
            let diff = item.get("changes").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|c| c.get("diff").and_then(|d| d.as_str())).collect::<Vec<_>>().join("\n")).unwrap_or_default();
            (if diff.is_empty() { status.to_string() } else { diff }, status == "failed" || status == "declined")
        }
        "mcpToolCall" => (item.get("result").map(|r| r.to_string()).or_else(|| item.get("error").map(|e| e.to_string())).unwrap_or_default(), item.get("error").is_some()),
        "dynamicToolCall" => (item.get("contentItems").map(|c| c.to_string()).unwrap_or_default(), item.get("success").and_then(|s| s.as_bool()) == Some(false)),
        _ => (String::new(), false),
    }
}

impl Session {
    /// JSON-RPC-запрос к app-server с ожиданием ответа.
    pub(crate) async fn codex_call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_ctrl_id();
        let rx = self.ctrl_wait(&id);
        self.write(&json!({ "jsonrpc": "2.0", "id": id.parse::<u64>().map(Value::from).unwrap_or(Value::String(id.clone())), "method": method, "params": params })).await?;
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(v)) => {
                if let Some(err) = v.get("error") { bail!("codex {method}: {}", err.get("message").and_then(|m| m.as_str()).unwrap_or("ошибка")); }
                Ok(v.get("result").cloned().unwrap_or(Value::Null))
            }
            Ok(Err(_)) => bail!("процесс завершился до ответа на {method}"),
            Err(_) => { self.ctrl_forget(&id); bail!("нет ответа на {method} за 120 с") }
        }
    }

    /// Старт: initialize → initialized → thread/start (или resume). Тред — provider_session_id сессии.
    pub(crate) async fn codex_start(self: &Arc<Self>, resume: bool) -> Result<()> {
        let meta = self.meta.lock().unwrap().clone();
        self.codex_call("initialize", json!({ "clientInfo": { "name": "cloudos-agentd", "title": "AgentVerse OS", "version": env!("CARGO_PKG_VERSION") }, "capabilities": { "experimentalApi": true } })).await.context("initialize")?;
        let _ = self.write(&json!({ "jsonrpc": "2.0", "method": "initialized" })).await;
        let (policy, sandbox) = approval(meta.permission_mode.as_deref());
        let env = effective_env(&meta);
        let gate = gate_config(&env);
        let mut model = meta.model.clone().or_else(|| env.get("CODEX_MODEL").cloned()).or_else(|| env.get("OPENAI_MODEL").cloned());
        // каталог моделей Codex — как в его пикере (displayName, уровни reasoning, tiers); кэш на workspace для остановленных сессий
        let ml = self.codex_call("model/list", match &gate { Some(cfg) => json!({ "config": cfg }), None => json!({}) }).await;
        let catalog_ids: Vec<String> = ml.as_ref().ok().and_then(|v| v.get("data").and_then(|d| d.as_array())).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect()).unwrap_or_default();
        // модель сверяем с каталогом маршрута: после смены аккаунта (gate ↔ подписка) прежняя модель может быть чужой —
        // тогда у gate берём лучшую из его списка, у подписки — умолчание CLI (иначе первый ход упадёт «model not found»)
        if gate.is_some() {
            let ids = gate_model_ids(&env).await;
            if let Some(m) = &model { if !ids.is_empty() && !ids.contains(m) { tracing::warn!("session {}: модели {m} нет в gate — беру из списка gate", &meta.id[..8]); model = None; } }
            if model.is_none() { model = rank_models(&ids); }
        } else if let Some(m) = &model {
            if !catalog_ids.is_empty() && !catalog_ids.contains(m) { tracing::warn!("session {}: модели {m} нет в каталоге Codex этого входа — умолчание CLI", &meta.id[..8]); model = None; }
        }
        let mut common = json!({ "cwd": meta.cwd, "approvalPolicy": policy, "sandbox": sandbox });
        if let Some(m) = &model { common["model"] = json!(m); }
        if let Some(cfg) = &gate { common["config"] = cfg.clone(); }
        if let Ok(v) = ml {
            let list: Vec<Value> = v.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default().into_iter().map(|m| json!({
                "value": m.get("id"), "label": m.get("displayName"), "description": m.get("description"), "hidden": m.get("hidden"), "default": m.get("isDefault"),
                "efforts": m.get("supportedReasoningEfforts").and_then(|e| e.as_array()).map(|a| a.iter().filter_map(|x| x.get("reasoningEffort").cloned()).collect::<Vec<_>>()).unwrap_or_default(),
                "effort_hints": m.get("supportedReasoningEfforts").and_then(|e| e.as_array()).map(|a| a.iter().filter_map(|x| Some((x.get("reasoningEffort")?.as_str()?.to_string(), x.get("description").cloned().unwrap_or(Value::Null)))).collect::<serde_json::Map<_, _>>()).unwrap_or_default(),
                "default_effort": m.get("defaultReasoningEffort"), "tiers": m.get("serviceTiers").cloned().unwrap_or(json!([])),
            })).collect();
            if !list.is_empty() {
                if let Some(parent) = self.dir().parent() { let _ = std::fs::write(parent.join("codex-models.json"), serde_json::to_vec(&list).unwrap_or_default()); }
                self.codex.lock().unwrap().models = list;
            }
        }
        tracing::info!("session {}: codex через {} модель {}", &meta.id[..8], if gate.is_some() { "gate/ключ (OPENAI_BASE_URL)" } else if env.contains_key("CODEX_HOME") { "аккаунт (CODEX_HOME)" } else { "свой вход ~/.codex" }, model.as_deref().unwrap_or("по умолчанию"));
        let res = match (&meta.provider_session_id, resume) {
            (Some(tid), true) => {
                let mut p = common.clone(); p["threadId"] = json!(tid);
                match self.codex_call("thread/resume", p.clone()).await {
                    Ok(r) => r,
                    Err(e) => {
                        // rollout треда живёт в каталоге входа, где он начался (~/.codex или CODEX_HOME прежнего аккаунта): после
                        // смены аккаунта переносим его в новый каталог и повторяем; не нашли — новый тред, история беседы в журнале agentd
                        let moved = bring_rollout(&env, tid);
                        let retry = if moved { self.codex_call("thread/resume", p).await.ok() } else { None };
                        match retry {
                            Some(r) => r,
                            None => {
                                tracing::warn!("session {}: thread/resume не удался ({e:#}) — новый тред", &meta.id[..8]);
                                self.emit(json!({ "kind": "system", "subtype": "thread_restarted", "data": { "message": format!("прежний тред Codex недоступен в этом входе ({}) — начат новый тред, история беседы сохранена в журнале", e.to_string().chars().take(120).collect::<String>()) } }));
                                self.codex_call("thread/start", common).await.context("thread/start")?
                            }
                        }
                    }
                }
            }
            _ => self.codex_call("thread/start", common).await.context("thread/start")?,
        };
        let tid = res.pointer("/thread/id").or_else(|| res.get("threadId")).or_else(|| res.get("id")).and_then(|v| v.as_str()).map(str::to_string).ok_or_else(|| anyhow!("thread/start без id: {}", res.to_string().chars().take(200).collect::<String>()))?;
        { let mut c = self.codex.lock().unwrap(); c.thread_id = Some(tid.clone()); c.epoch = crate::session::now_ms() % 100_000_000; }
        {
            let mut m = self.meta.lock().unwrap();
            m.provider_session_id = Some(tid.clone());
            if let Some(model) = res.pointer("/thread/model").and_then(|v| v.as_str()) { m.model = Some(model.to_string()); }
        }
        self.save_meta();
        self.emit(json!({ "kind": "init", "model": res.pointer("/thread/model"), "session_id": tid, "cwd": meta.cwd, "permission_mode": meta.permission_mode, "provider": "codex", "route": if gate.is_some() { "gate" } else if env.contains_key("CODEX_HOME") { "account" } else { "codex-home" } }));
        self.set_status(Status::Idle);
        self.drain_queue();
        Ok(())
    }

    /// Сообщение пользователя → `turn/start`; картинки — файлами рядом с журналом (`localImage`).
    pub(crate) async fn codex_prompt(&self, content: &Value) -> Result<()> {
        let tid = self.codex.lock().unwrap().thread_id.clone().ok_or_else(|| anyhow!("тред Codex ещё не создан"))?;
        let mut input = Vec::new();
        for (i, b) in content.as_array().cloned().unwrap_or_default().iter().enumerate() {
            match b.get("type").and_then(|t| t.as_str()) {
                Some("text") => input.push(json!({ "type": "text", "text": b.get("text").and_then(|t| t.as_str()).unwrap_or("") })),
                Some("image") => {
                    let data = b.pointer("/source/data").and_then(|d| d.as_str()).unwrap_or("");
                    let ext = match b.pointer("/source/media_type").and_then(|m| m.as_str()).unwrap_or("image/png") { "image/jpeg" => "jpg", "image/webp" => "webp", "image/gif" => "gif", _ => "png" };
                    let bytes = base64_decode(data)?;
                    let path = self.dir().join(format!("img-{}-{i}.{ext}", crate::session::now_ms()));
                    std::fs::write(&path, bytes)?;
                    input.push(json!({ "type": "localImage", "path": path.to_string_lossy() }));
                }
                _ => {}
            }
        }
        if input.is_empty() { bail!("пустое сообщение"); }
        {
            let mut c = self.codex.lock().unwrap();
            c.last_text.clear();
            c.turn_started_ms = crate::session::now_ms();
        }
        let (model, policy, effort, tier) = { let m = self.meta.lock().unwrap(); (m.model.clone(), approval(m.permission_mode.as_deref()).0, m.effort.clone(), m.service_tier.clone()) };
        let mut params = json!({ "threadId": tid, "input": input, "approvalPolicy": policy });
        if let Some(m) = model { params["model"] = json!(m); }
        if let Some(e) = effort { params["effort"] = json!(e); }
        if let Some(t) = tier { params["serviceTier"] = json!(t); }
        let res = self.codex_call("turn/start", params).await?;
        if let Some(turn) = res.pointer("/turn/id").and_then(|v| v.as_str()) { self.codex.lock().unwrap().turn_id = Some(turn.to_string()); }
        Ok(())
    }

    pub(crate) async fn codex_interrupt(&self) -> Result<Value> {
        let (tid, turn) = { let c = self.codex.lock().unwrap(); (c.thread_id.clone(), c.turn_id.clone()) };
        let (Some(tid), Some(turn)) = (tid, turn) else { bail!("нет активного хода") };
        self.codex_call("turn/interrupt", json!({ "threadId": tid, "turnId": turn })).await
    }

    /// Ответ на запрос app-server по виду ожидающего запроса.
    pub(crate) async fn codex_answer(&self, p: &Pending, rid: &Value, behavior: &str, answers: Option<Value>, remember: bool) -> Result<()> {
        let result = match (p.kind.as_str(), behavior) {
            ("codex_question", _) => {
                // ответы по id вопроса; сопоставление «текст вопроса → id» лежит в input.question_ids
                let ids = p.input.get("question_ids").cloned().unwrap_or(json!({}));
                let mut out = serde_json::Map::new();
                if let Some(a) = answers.as_ref().and_then(|a| a.as_object()) {
                    for (q, v) in a {
                        let id = ids.get(q).and_then(|i| i.as_str()).unwrap_or(q.as_str());
                        let list: Vec<Value> = match v { Value::Array(x) => x.clone(), Value::String(s) => s.split(", ").map(|s| json!(s)).collect(), other => vec![other.clone()] };
                        out.insert(id.to_string(), json!({ "answers": list }));
                    }
                }
                json!({ "answers": out })
            }
            ("codex_perms", "allow") => json!({ "permissions": p.input.get("permissions").cloned().unwrap_or(json!({})) }),
            ("codex_perms", _) => json!({ "permissions": {} }),
            (_, "allow") => json!({ "decision": if remember { "acceptForSession" } else { "accept" } }),
            (_, _) => json!({ "decision": "decline" }),
        };
        self.write(&json!({ "jsonrpc": "2.0", "id": rid, "result": result })).await
    }

    /// Уведомления и запросы app-server → журнал сессии.
    pub(crate) async fn codex_on_message(self: &Arc<Self>, v: Value) {
        let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
        if method.is_empty() {
            // ответ на наш запрос
            if let Some(id) = v.get("id") {
                let key = match id { Value::Number(n) => n.to_string(), Value::String(s) => s.clone(), _ => String::new() };
                self.ctrl_resolve(&key, v.clone());
            }
            return;
        }
        let params = v.get("params").cloned().unwrap_or(Value::Null);
        let is_request = v.get("id").is_some();
        if is_request {
            self.codex_on_request(&method, v.get("id").cloned().unwrap_or(Value::Null), params).await;
            return;
        }
        match method.as_str() {
            "item/started" | "item/completed" => {
                let item = params.get("item").cloned().unwrap_or(Value::Null);
                let id = item.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
                let t = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let completed = method == "item/completed";
                match t {
                    "agentMessage" if completed => {
                        let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
                        if !text.trim().is_empty() {
                            self.codex.lock().unwrap().last_text = text.clone();
                            self.emit(json!({ "kind": "assistant", "content": [{ "type": "text", "text": text }], "model": self.meta.lock().unwrap().model, "uuid": id }));
                        }
                    }
                    "reasoning" if completed => {
                        let text = item.get("summary").or_else(|| item.get("content")).and_then(|s| s.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                        if !text.trim().is_empty() { self.emit(json!({ "kind": "assistant", "content": [{ "type": "thinking", "thinking": text }], "uuid": id })); }
                    }
                    "plan" if completed => {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) { self.emit(json!({ "kind": "assistant", "content": [{ "type": "text", "text": format!("📋 {text}") }], "uuid": id })); }
                    }
                    "commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall" => {
                        if !completed {
                            if let Some((name, input)) = tool_call(&item) {
                                self.codex.lock().unwrap().tools.insert(id.clone(), name.clone());
                                self.emit(json!({ "kind": "assistant", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }], "uuid": id }));
                            }
                        } else {
                            let known = self.codex.lock().unwrap().tools.contains_key(&id);
                            if !known {
                                if let Some((name, input)) = tool_call(&item) { self.emit(json!({ "kind": "assistant", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }], "uuid": id })); }
                            }
                            let (out, err) = tool_output(&item);
                            self.emit(json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": id, "content": out.chars().take(20000).collect::<String>(), "is_error": err }] }));
                        }
                    }
                    _ => {}
                }
                if self.status() == Status::Idle { self.set_status(Status::Working); }
            }
            "turn/started" => {
                if let Some(turn) = params.pointer("/turn/id").and_then(|v| v.as_str()) { self.codex.lock().unwrap().turn_id = Some(turn.to_string()); }
                self.set_status(Status::Working);
            }
            "thread/tokenUsage/updated" => {
                let mut c = self.codex.lock().unwrap();
                c.last_usage = params.get("tokenUsage").cloned();
                if let Some(w) = params.pointer("/tokenUsage/modelContextWindow").or_else(|| params.get("modelContextWindow")).and_then(|w| w.as_u64()) { c.context_window = Some(w); }
            }
            "turn/completed" => {
                let turn = params.get("turn").cloned().unwrap_or(Value::Null);
                let (usage, text, started) = { let c = self.codex.lock().unwrap(); (c.last_usage.clone(), c.last_text.clone(), c.turn_started_ms) };
                let last = usage.as_ref().and_then(|u| u.get("last")).cloned().unwrap_or(json!({}));
                let g = |k: &str| last.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
                let error = turn.get("error").filter(|e| !e.is_null()).cloned();
                let duration = turn.get("durationMs").and_then(|d| d.as_u64()).unwrap_or_else(|| crate::session::now_ms().saturating_sub(started));
                let ev = json!({
                    "kind": "result", "subtype": if error.is_some() { "error_during_execution" } else { "success" }, "is_error": error.is_some(), "duration_ms": duration, "num_turns": 1,
                    "total_cost_usd": 0.0, "cost_source": "none",
                    "usage": { "input_tokens": g("inputTokens").saturating_sub(g("cachedInputTokens")), "output_tokens": g("outputTokens"), "cache_read_input_tokens": g("cachedInputTokens"), "cache_creation_input_tokens": g("cacheWriteInputTokens") },
                    "model_usage": self.meta.lock().unwrap().model.clone().map(|m| json!({ m: { "inputTokens": g("inputTokens"), "outputTokens": g("outputTokens"), "cachedInputTokens": g("cachedInputTokens") } })),
                    "text": if let Some(e) = &error { e.get("message").and_then(|m| m.as_str()).unwrap_or("ошибка").to_string() } else { text },
                    "session_id": self.codex.lock().unwrap().thread_id,
                });
                self.record_result(ev);
                self.codex.lock().unwrap().turn_id = None;
                self.set_status(Status::Idle);
            }
            "error" => {
                // ошибки маршрута до модели: с повтором — только в лог, без повтора — в журнал (ход всё равно завершится turn/completed)
                let msg = params.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("").to_string();
                if params.get("willRetry").and_then(|w| w.as_bool()) == Some(true) { tracing::debug!("session {}: codex retry: {msg}", &self.id()[..8]); }
                else { self.emit(json!({ "kind": "system", "subtype": "provider_error", "data": { "message": msg } })); }
            }
            _ => {}
        }
    }

    async fn codex_on_request(self: &Arc<Self>, method: &str, rid: Value, params: Value) {
        let raw = match &rid { Value::Number(n) => n.to_string(), Value::String(s) => s.clone(), _ => "0".into() };
        let rid_key = format!("cx{}-{raw}", self.codex.lock().unwrap().epoch);
        let (kind, tool, input, question) = match method {
            "item/commandExecution/requestApproval" => ("codex_command", "Bash".to_string(), json!({ "command": params.get("command"), "cwd": params.get("cwd"), "reason": params.get("reason") }), false),
            "item/fileChange/requestApproval" => ("codex_file", "Edit".to_string(), json!({ "file_path": params.get("grantRoot"), "reason": params.get("reason") }), false),
            "item/permissions/requestApproval" => ("codex_perms", "Permissions".to_string(), json!({ "permissions": params.get("permissions"), "reason": params.get("reason"), "cwd": params.get("cwd") }), false),
            "item/tool/requestUserInput" => {
                // в форму Claude AskUserQuestion: карточка Chat уже умеет её рисовать
                let mut ids = serde_json::Map::new();
                let questions: Vec<Value> = params.get("questions").and_then(|q| q.as_array()).cloned().unwrap_or_default().iter().map(|q| {
                    let text = q.get("question").and_then(|s| s.as_str()).unwrap_or("").to_string();
                    ids.insert(text.clone(), q.get("id").cloned().unwrap_or(Value::Null));
                    json!({ "question": text, "header": q.get("header"), "multiSelect": false, "options": q.get("options").and_then(|o| o.as_array()).cloned().unwrap_or_default().iter().map(|o| match o { Value::String(s) => json!({ "label": s, "description": "" }), other => json!({ "label": other.get("label").or_else(|| other.get("value")).cloned().unwrap_or(json!("?")), "description": other.get("description").cloned().unwrap_or(json!("")) }) }).collect::<Vec<_>>() })
                }).collect();
                ("codex_question", "AskUserQuestion".to_string(), json!({ "questions": questions, "question_ids": ids }), true)
            }
            other => {
                let _ = self.write(&json!({ "jsonrpc": "2.0", "id": rid, "error": { "code": -32601, "message": format!("agentd: {other} не поддерживается") } })).await;
                return;
            }
        };
        let ev = self.emit(json!({ "kind": "permission_request", "request_id": rid_key, "tool_name": tool, "input": input, "question": question, "provider": "codex", "description": params.get("reason") }));
        let seq = ev.get("seq").and_then(|s| s.as_u64()).unwrap_or(0);
        self.pending.lock().unwrap().insert(rid_key.clone(), Pending { request_id: rid_key, tool_name: tool, input, seq, kind: kind.into(), suggestions: None, rpc_id: Some(rid) });
        self.set_status(Status::Waiting);
    }
}

fn base64_decode(s: &str) -> Result<Vec<u8>> {
    // без зависимости: стандартный алфавит, паддинг допускается
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        if c == b'=' || c == b'\n' || c == b'\r' { continue }
        let v = T.iter().position(|&t| t == c).ok_or_else(|| anyhow!("не base64"))? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((buf >> bits) as u8); buf &= (1 << bits) - 1; }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rollout_lookup_by_thread_id() {
        let d = std::env::temp_dir().join(format!("codex-home-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sessions/2026/09/29")).unwrap();
        std::fs::write(d.join("sessions/2026/09/29/rollout-2026-09-29T10-00-00-abc123.jsonl"), b"{}\n").unwrap();
        let (f, rel) = find_rollout_in(&d, "abc123").unwrap();
        assert!(f.ends_with("rollout-2026-09-29T10-00-00-abc123.jsonl")); assert_eq!(rel, std::path::PathBuf::from("2026/09/29/rollout-2026-09-29T10-00-00-abc123.jsonl"));
        assert!(find_rollout_in(&d, "nope").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn maps_items_to_tool_calls_and_outputs() {
        let cmd = json!({ "type": "commandExecution", "id": "i1", "command": "ls -la", "cwd": "/home/coder", "status": "completed", "exitCode": 0, "aggregatedOutput": "total 0" });
        let (name, input) = tool_call(&cmd).unwrap();
        assert_eq!(name, "Bash");
        assert_eq!(input["command"], "ls -la");
        assert_eq!(tool_output(&cmd), ("total 0".to_string(), false));
        let failed = json!({ "type": "commandExecution", "command": "x", "status": "failed", "exitCode": 1, "aggregatedOutput": "" });
        assert_eq!(tool_output(&failed), ("(код выхода 1)".to_string(), true));
        let fc = json!({ "type": "fileChange", "id": "i2", "status": "completed", "changes": [{ "path": "/a.txt", "diff": "+hello" }] });
        assert_eq!(tool_call(&fc).unwrap().0, "Edit");
        assert_eq!(tool_output(&fc).0, "+hello");
        assert!(tool_call(&json!({ "type": "agentMessage" })).is_none());
    }

    #[test]
    fn tui_args_resume_gate_and_bypass() {
        let mut m = crate::session::Meta { id: "s1".into(), agent: "codex".into(), cwd: "/w".into(), title: "t".into(), created_at: 0, provider_session_id: Some("thread-1".into()), model: None, permission_mode: Some("plan".into()), account: None, env: Default::default(), unset: vec![], imported: false, transcript: None, synced_lines: None, usage_base: None, effort: None, import_v: None, max_thinking_tokens: None, service_tier: None, cache_ttl: None, allowed_tools: Vec::new(), disallowed_tools: Vec::new(), resume_at: None, context_window: None };
        let env = parse_env_sh("export OPENAI_BASE_URL='http://llm.gate/v1'\n");
        let a = tui_args(&m, gate_config(&env).as_ref(), Some("gpt-5-codex"), false).join(" ");
        assert!(a.starts_with("codex -c model_provider=\"cloudos\" -c model_providers.cloudos.base_url=\"http://llm.gate/v1\""), "{a}");
        assert!(a.ends_with("resume thread-1 -m gpt-5-codex -a on-request -s read-only"), "{a}");
        m.provider_session_id = None;
        let b = tui_args(&m, None, None, true);
        assert_eq!(b, vec!["codex", "--dangerously-bypass-approvals-and-sandbox"]);
        let t = crate::session::claude_transcript("/home/coder/.claude", "/home/coder/my.app", "abc");
        assert_eq!(t.to_string_lossy(), "/home/coder/.claude/projects/-home-coder-my-app/abc.jsonl");
    }

    #[test]
    fn approval_mapping_and_env() {
        assert_eq!(approval(None), ("on-request", "workspace-write"));
        assert_eq!(approval(Some("bypassPermissions")), ("never", "danger-full-access"));
        assert_eq!(approval(Some("plan")), ("untrusted", "read-only"));
        assert_eq!(approval(Some("untrusted")), ("untrusted", "workspace-write"));
        let env = parse_env_sh("# comment\nexport OPENAI_BASE_URL='http://llm.gate/v1'\nexport OPENAI_API_KEY=\"k1\"\nexport X=plain\n");
        assert_eq!(env.get("OPENAI_BASE_URL").map(String::as_str), Some("http://llm.gate/v1"));
        assert_eq!(env.get("OPENAI_API_KEY").map(String::as_str), Some("k1"));
        assert_eq!(env.get("X").map(String::as_str), Some("plain"));
        let cfg = gate_config(&env).unwrap();
        assert_eq!(cfg["model_provider"], "cloudos");
        assert_eq!(cfg["model_providers"]["cloudos"]["base_url"], "http://llm.gate/v1");
        let mut with_home = env.clone(); with_home.insert("CODEX_HOME".into(), "/x".into());
        assert!(gate_config(&with_home).is_none());
        assert!(gate_config(&HashMap::new()).is_none());
        let ids: Vec<String> = ["devin/gpt-5-3-codex", "kimi-k3", "gpt-oss-120b-medium", "devin/gpt-6-astra"].iter().map(|s| s.to_string()).collect();
        assert_eq!(rank_models(&ids).as_deref(), Some("gpt-oss-120b-medium"));
        let ids2: Vec<String> = ["gpt-5.1", "gpt-5-codex", "claude-x"].iter().map(|s| s.to_string()).collect();
        assert_eq!(rank_models(&ids2).as_deref(), Some("gpt-5-codex"));
        assert_eq!(rank_models(&["claude-x".to_string()]), None);
    }

    #[test]
    fn base64_roundtrip() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGk").unwrap(), b"hi");
        assert!(base64_decode("!!").is_err());
    }
}
