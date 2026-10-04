//! Провайдеры по ACP — Agent Client Protocol (docs/agents-ux.md §4.3.8): JSON-RPC 2.0 построчно через stdio, один протокол
//! для Gemini CLI (`gemini --experimental-acp`), pi (адаптер `pi-acp` поверх `pi --mode rpc`) и остальных агентов реестра
//! ACP. Мы — клиент: `initialize` → (`authenticate`) → `session/new` | `session/load` → `session/prompt` (ответ приходит,
//! когда ход закончен); агент шлёт уведомления `session/update` (куски текста и размышлений, вызовы инструментов и их
//! итоги, план, команды, режим, заполненность контекста) и запросы `session/request_permission`. Всё это ложится в тот же
//! журнал, что у Claude и Codex, — карточка Chat одна. Файловую систему и терминал по ACP не делегируем (агент работает
//! с диском контейнера сам), MCP-серверов в `session/new` не передаём.
use crate::session::{Pending, Session, Status};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub const PROTOCOL_VERSION: u64 = 1;

/// Состояние ACP-сессии внутри нашей сессии.
#[derive(Default)]
pub struct AcpState {
    pub session_id: Option<String>,
    /// Метка запуска процесса: id запросов начинаются с нуля при каждом старте, журнал — один на все запуски
    pub epoch: u64,
    /// Возобновление: история проигрывается агентом как `session/update`, в журнал её не кладём (она уже там)
    pub loading: bool,
    /// Копим куски текста и размышлений: в журнал — при вызове инструмента и в конце хода
    pub text: String,
    pub thought: String,
    /// Вызовы инструментов хода: id → (имя для карточки, заголовок агента, накопленный вывод)
    pub tools: HashMap<String, (String, String, String)>,
    pub tool_order: Vec<String>,
    /// `modes` из ответа session/new: { currentModeId, availableModes: [{id, name, description}] }
    pub modes: Value,
    /// `models` (нестабильное поле ACP): { currentModelId, availableModels: [{modelId, name, description}] }
    pub models: Value,
    pub config_options: Value,
    /// `available_commands_update`: [{name, description, input}]
    pub commands: Vec<Value>,
    /// `usage_update` (нестабильное): { used, size, cost? }
    pub usage: Option<Value>,
    pub turn_started_ms: u64,
    pub turn_text: String,
    pub auth_methods: Vec<Value>,
    pub load_session: bool,
    /// Текущая модель по данным агента (currentModelId или модель из init)
    pub model: Option<String>,
    /// pi: сколько записей toolResult из файла сессии уже показано (вывод bash приходит только туда)
    pub pi_tool_idx: usize,
    /// идёт ли ход (между session/prompt и его ответом): текст агента вне хода — служебная заметка, а не ответ
    pub in_turn: bool,
    /// после рестарта агент не смог поднять свою сессию (session/load) — первому промпту предпошлём пересказ журнала
    pub recap_pending: bool,
}

fn now_ms() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0) }

/// Имя инструмента для карточки по виду ACP (`kind`) и заголовку: карточка Chat знает Bash/Edit/Read/Grep/WebFetch/Agent.
pub fn tool_name(kind: Option<&str>, title: &str, raw_input: &Value) -> String {
    match kind.unwrap_or("") {
        "execute" => "Bash", "edit" => "Edit", "delete" | "move" => "Bash", "read" => "Read", "search" => "Grep", "fetch" => "WebFetch", "think" => "Agent",
        _ => {
            // pi-acp и Gemini кладут имя инструмента в rawInput или заголовок («run_shell_command», «bash», «edit»)
            let t = title.to_ascii_lowercase();
            if t.contains("shell") || t.contains("bash") || t.starts_with("run ") || raw_input.get("command").is_some() { "Bash" } else if t.contains("edit") || t.contains("write") || t.contains("replace") { "Edit" } else if t.contains("read") { "Read" } else if t.contains("search") || t.contains("grep") || t.contains("glob") || t.contains("find") { "Grep" } else if t.contains("fetch") || t.contains("http") { "WebFetch" } else { "Tool" }
        }
    }.to_string()
}

/// Текст из блоков содержимого ACP (ToolCallContent: `content` с ContentBlock, `diff`, `terminal`).
pub fn content_text(content: &Value) -> String {
    let mut out = String::new();
    for c in content.as_array().cloned().unwrap_or_default() {
        match c.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "content" => { if let Some(t) = c.pointer("/content/text").and_then(|t| t.as_str()) { out.push_str(t); out.push('\n'); } }
            "diff" => {
                let path = c.get("path").and_then(|p| p.as_str()).unwrap_or("");
                let old = c.get("oldText").and_then(|p| p.as_str()).unwrap_or("");
                let new = c.get("newText").and_then(|p| p.as_str()).unwrap_or("");
                out.push_str(&format!("--- {path}\n"));
                for l in old.lines() { out.push_str(&format!("- {l}\n")); }
                for l in new.lines() { out.push_str(&format!("+ {l}\n")); }
            }
            "text" => { if let Some(t) = c.get("text").and_then(|t| t.as_str()) { out.push_str(t); out.push('\n'); } }
            "terminal" => { out.push_str("[terminal]\n"); }
            _ => {}
        }
    }
    out
}

/// Режим сессии в терминах агента: наши имена (из Claude) и имена Codex переводятся в режимы Gemini; то, что агент
/// перечислил в availableModes, принимается как есть.
pub fn map_mode(engine: &str, mode: &str, available: &[String]) -> Option<String> {
    // ACP-движки именуют режимы по-своему (Gemini: `autoEdit`, флаг CLI — `auto_edit`): сравниваем без регистра и `_`/`-`
    let norm = |s: &str| s.chars().filter(|c| *c != '_' && *c != '-').collect::<String>().to_ascii_lowercase();
    if let Some(a) = available.iter().find(|a| norm(a) == norm(mode)) { return Some(a.clone()); }
    let m = if engine == "gemini" {
        match norm(mode).as_str() { "default" | "onrequest" | "untrusted" => "default", "acceptedits" | "autoedit" => "autoEdit", "bypasspermissions" | "auto" | "dontask" | "never" | "yolo" => "yolo", "plan" | "readonly" => "plan", _ => return None }
    } else { return None };
    if available.is_empty() { return Some(m.to_string()) }
    available.iter().find(|a| norm(a) == norm(m)).cloned()
}

/// Вариант разрешения под ответ владельца: allow (+ запомнить) → allow_always, иначе allow_once; deny → reject_once /
/// reject_always; нет подходящего — отмена.
pub fn pick_option(options: &Value, behavior: &str, remember: bool) -> Option<String> {
    let opts: Vec<(String, String)> = options.as_array().cloned().unwrap_or_default().iter().filter_map(|o| Some((o.get("optionId").and_then(|i| i.as_str())?.to_string(), o.get("kind").and_then(|k| k.as_str()).unwrap_or("").to_string()))).collect();
    let find = |kind: &str| opts.iter().find(|(_, k)| k == kind).map(|(id, _)| id.clone());
    match (behavior, remember) {
        ("allow", true) => find("allow_always").or_else(|| find("allow_once")),
        ("allow", false) => find("allow_once").or_else(|| find("allow_always")),
        (_, true) => find("reject_always").or_else(|| find("reject_once")),
        _ => find("reject_once").or_else(|| find("reject_always")),
    }
}

/// Имена, которые Gemini CLI подменяет своими «последними» моделями (resolveModel: gemini-3-flash → gemini-3.8-flash и т.п.);
/// через gate такие не проходят — в выбор и список не попадают.
const GEMINI_REMAPPED: &[&str] = &["gemini-3-flash", "gemini-3.5-flash", "gemini-3-flash-preview", "gemini-3.1-flash-lite", "gemini-3.5-flash-lite", "flash", "flash-lite", "pro", "auto"];

/// Ранг модели Gemini для умолчания: номер версии (3.8 > 3.1), затем pro > flash, high > обычная > low.
fn gemini_rank(id: &str) -> f64 {
    let ver: f64 = id.trim_start_matches("gemini-").split('-').next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let tier = if id.contains("pro") { 0.3 } else if id.contains("flash") { 0.2 } else { 0.1 };
    let think = if id.ends_with("-high") { 0.03 } else if id.ends_with("-low") { 0.01 } else { 0.02 };
    ver * 10.0 + tier + think
}

/// Модели Gemini из каталога gate, которые CLI передаст как есть (без картинок, lite, чужих префиксов и подменяемых имён),
/// от лучшей к худшей.
pub fn gemini_gate_models(ids: &[String]) -> Vec<String> {
    let mut v: Vec<String> = ids.iter().filter(|s| s.starts_with("gemini-") && !s.contains("image") && !s.contains("lite") && !s.contains('/') && !GEMINI_REMAPPED.contains(&s.as_str())).cloned().collect();
    v.sort_by(|a, b| gemini_rank(b).partial_cmp(&gemini_rank(a)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.cmp(b)));
    v
}

/// Модель Gemini по умолчанию через gate — лучшая из `gemini_gate_models`.
pub fn pick_gemini_model(ids: &[String]) -> Option<String> { gemini_gate_models(ids).into_iter().next() }

/// Модель pi по умолчанию через gate: самый новый Claude Sonnet без датированного суффикса (такие gate обычно уже не
/// обслуживает), затем Opus, затем GPT, иначе первая из каталога.
pub fn pick_pi_model(ids: &[String]) -> Option<String> {
    let ver = |s: &str, prefix: &str| -> Option<(u32, u32)> {
        let mut nums: Vec<u32> = Vec::new();
        for seg in s.strip_prefix(prefix)?.split(|c| c == '-' || c == '.') {
            if let Ok(n) = seg.parse::<u32>() { if n >= 1000 { return None } nums.push(n); }
        }
        Some((*nums.first()?, nums.get(1).copied().unwrap_or(0)))
    };
    for prefix in ["claude-sonnet-", "claude-opus-", "gpt-"] {
        if let Some(best) = ids.iter().filter_map(|s| ver(s, prefix).map(|v| (v, s))).max_by_key(|(v, _)| *v).map(|(_, s)| s.clone()) { return Some(best) }
    }
    ids.first().cloned()
}

/// Провайдер gate для pi в `~/.pi/agent/models.json`: OpenAI-совместимый `/v1` gate'а с моделями из его каталога (без
/// картинок и видео). Остальное содержимое файла не трогаем; при отсутствии своих настроек модели — умолчание на gate.
pub async fn pi_ensure_gate(env: &HashMap<String, String>) -> Option<String> {
    let base = env.get("OPENAI_BASE_URL")?.trim_end_matches('/').to_string();
    if !env.contains_key("OPENAI_API_KEY") { return None }
    let ids = crate::codex::gate_model_ids(env).await;
    let models: Vec<Value> = ids.iter().filter(|s| { let l = s.to_ascii_lowercase(); !l.contains("image") && !l.contains("imagine") && !l.contains("video") && !l.contains("-dd-") }).map(|id| json!({ "id": id, "name": id })).collect();
    if models.is_empty() { return None }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    let dir = env.get("PI_CODING_AGENT_DIR").cloned().unwrap_or_else(|| format!("{home}/.pi/agent"));
    let _ = std::fs::create_dir_all(&dir);
    let path = format!("{dir}/models.json");
    let mut doc: Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    if !doc.is_object() { doc = json!({}); }
    if !doc.get("providers").map(|p| p.is_object()).unwrap_or(false) { doc["providers"] = json!({}); }
    doc["providers"]["agentverse-gate"] = json!({ "baseUrl": base, "api": "openai-completions", "apiKey": "$OPENAI_API_KEY", "models": models });
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap_or_default());
    // умолчание модели: только если владелец ничего не настроил сам (нет defaultModel в settings.json)
    let default = pick_pi_model(&ids);
    let spath = format!("{dir}/settings.json");
    let mut st: Value = std::fs::read(&spath).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    if !st.is_object() { st = json!({}); }
    if st.get("defaultModel").is_none() { if let Some(d) = &default { st["defaultProvider"] = json!("agentverse-gate"); st["defaultModel"] = json!(d); } }
    if st.get("quietStartup").is_none() { st["quietStartup"] = json!(true); }
    let _ = std::fs::write(&spath, serde_json::to_vec_pretty(&st).unwrap_or_default());
    default.map(|d| format!("agentverse-gate/{d}"))
}

/// Файл сессии pi по id ACP-сессии (pi-acp ведёт карту `~/.pi/pi-acp/session-map.json`) — для TUI `pi --session <файл>`.
pub fn pi_session_file(sid: &str) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    let v: Value = serde_json::from_slice(&std::fs::read(format!("{home}/.pi/pi-acp/session-map.json")).ok()?).ok()?;
    v.pointer(&format!("/sessions/{sid}/sessionFile")).and_then(|f| f.as_str()).map(str::to_string)
}

/// pi-acp глотает ошибки pi (ход заканчивается end_turn без текста): смотрим последнюю запись ассистента в файле сессии pi.
pub fn pi_last_error(sid: Option<&str>) -> Option<String> {
    let s = std::fs::read_to_string(pi_session_file(sid?)?).ok()?;
    let last = s.lines().rev().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v.get("type").and_then(|t| t.as_str()) == Some("message") && v.pointer("/message/role").and_then(|r| r.as_str()) == Some("assistant"))?;
    if last.pointer("/message/stopReason").and_then(|r| r.as_str()) != Some("error") { return None }
    Some(last.pointer("/message/errorMessage").and_then(|e| e.as_str()).unwrap_or("ошибка модели").to_string())
}

/// pi-acp отдаёт для bash только ссылку на терминал (`{type: terminal}`), а не вывод; вывод лежит в файле сессии pi
/// записью `toolResult`. Берём первую ещё не показанную запись после `skip` штук; возвращаем (текст, ошибка?, новый skip).
pub fn pi_tool_output(sid: Option<&str>, skip: usize) -> Option<(String, bool, usize)> {
    let s = std::fs::read_to_string(pi_session_file(sid?)?).ok()?;
    let mut seen = 0usize;
    for l in s.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        if v.get("type").and_then(|t| t.as_str()) != Some("message") || v.pointer("/message/role").and_then(|r| r.as_str()) != Some("toolResult") { continue }
        seen += 1;
        if seen <= skip { continue }
        let text = content_text(v.pointer("/message/content").unwrap_or(&Value::Null));
        let is_err = v.pointer("/message/isError").and_then(|e| e.as_bool()).unwrap_or(false);
        return Some((text, is_err, seen));
    }
    None
}

/// Gemini CLI: режимы autoEdit/yolo доступны только в «доверенной» папке (`setApprovalMode` иначе падает с Internal error);
/// доверие хранится в `~/.gemini/trustedFolders.json` — добавляем cwd сессии как TRUST_FOLDER, остальное не трогаем.
pub fn gemini_ensure_trust(cwd: &str) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    let dir = format!("{home}/.gemini");
    let _ = std::fs::create_dir_all(&dir);
    let path = format!("{dir}/trustedFolders.json");
    let mut doc: Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    if !doc.is_object() { doc = json!({}); }
    if doc.get(cwd).is_some() { return }
    doc[cwd] = json!("TRUST_FOLDER");
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap_or_default());
}

/// Gemini CLI по ACP не отдаёт вывод shell-команд (returnDisplay у него — объект терминала, `toToolCallContent` даёт
/// null), но пишет его в запись беседы `~/.gemini/tmp/<проект>/chats/session-<дата>-<8 знаков id>.jsonl`: строка
/// `{"type":"gemini","toolCalls":[{id, result:[{functionResponse:{response:{output}}}], resultDisplay:[[{text}]]}]}`.
pub fn gemini_tool_output(sid: Option<&str>, tid: &str) -> Option<(String, bool)> {
    let sid = sid?; let short: String = sid.chars().take(8).collect();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for proj in std::fs::read_dir(format!("{home}/.gemini/tmp")).ok()?.flatten() {
        if let Ok(rd) = std::fs::read_dir(proj.path().join("chats")) {
            files.extend(rd.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("session-") && n.ends_with(&format!("-{short}.jsonl"))).unwrap_or(false)));
        }
    }
    for f in files {
        let s = std::fs::read_to_string(&f).ok()?;
        for l in s.lines().rev() {
            let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
            let Some(calls) = v.get("toolCalls").and_then(|c| c.as_array()) else { continue };
            for c in calls {
                if c.get("id").and_then(|i| i.as_str()) != Some(tid) { continue }
                let is_err = c.get("status").and_then(|s| s.as_str()).map(|s| s != "success").unwrap_or(false);
                // resultDisplay — строки терминала (сегменты с пробелами до ширины окна); иначе — output функции без обёртки
                let mut lines: Vec<String> = c.get("resultDisplay").and_then(|d| d.as_array()).map(|rows| rows.iter().map(|row| row.as_array().map(|segs| segs.iter().filter_map(|sg| sg.get("text").and_then(|t| t.as_str())).collect::<String>()).unwrap_or_default().trim_end().to_string()).collect()).unwrap_or_default();
                while lines.last().map(|l| l.is_empty()).unwrap_or(false) { lines.pop(); }
                if lines.is_empty() {
                    if let Some(out) = c.pointer("/result/0/functionResponse/response/output").and_then(|o| o.as_str()) {
                        lines = out.lines().filter(|l| !l.starts_with("<untrusted_context>") && !l.starts_with("</untrusted_context>") && !l.starts_with("Process Group PGID:")).map(|l| l.strip_prefix("Output: ").unwrap_or(l).to_string()).collect();
                    }
                }
                return Some((lines.join("\n"), is_err));
            }
        }
    }
    None
}

/// Токены хода pi — из файла сессии: сумма `usage` записей ассистента после последнего сообщения пользователя
/// (pi-acp по ACP отдаёт только заполненность контекста).
pub fn pi_turn_usage(sid: Option<&str>) -> Option<Value> {
    let s = std::fs::read_to_string(pi_session_file(sid?)?).ok()?;
    let (mut inp, mut out, mut cr, mut cw, mut n) = (0u64, 0u64, 0u64, 0u64, 0u32);
    for l in s.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        if v.get("type").and_then(|t| t.as_str()) != Some("message") { continue }
        match v.pointer("/message/role").and_then(|r| r.as_str()) {
            Some("user") => break,
            Some("assistant") => {
                let g = |k: &str| v.pointer(&format!("/message/usage/{k}")).and_then(|x| x.as_u64()).unwrap_or(0);
                inp += g("input"); out += g("output"); cr += g("cacheRead"); cw += g("cacheWrite"); n += 1;
            }
            _ => {}
        }
    }
    if n == 0 { return None }
    Some(json!({ "input_tokens": inp, "output_tokens": out, "cache_read_input_tokens": cr, "cache_creation_input_tokens": cw }))
}

/// Значение модели для агента: у pi это `провайдер/модель` — голое имя дополняем по его списку (сначала наш провайдер gate).
pub fn acp_model_value(st: &AcpState, model: &str) -> String {
    if model.contains('/') { return model.to_string() }
    let mut candidates: Vec<String> = Vec::new();
    if let Some(opts) = st.config_options.as_array() {
        for o in opts.iter().filter(|o| o.get("id").and_then(|i| i.as_str()) == Some("model")) {
            if let Some(vs) = o.get("options").and_then(|v| v.as_array()) { candidates.extend(vs.iter().filter_map(|v| v.get("value").and_then(|x| x.as_str()).map(str::to_string))); }
        }
    }
    if let Some(ms) = st.models.get("availableModels").and_then(|a| a.as_array()) { candidates.extend(ms.iter().filter_map(|m| m.get("modelId").and_then(|x| x.as_str()).map(str::to_string))); }
    let suffix = format!("/{model}");
    candidates.iter().find(|c| c.as_str() == format!("agentverse-gate/{model}")).or_else(|| candidates.iter().find(|c| c.ends_with(&suffix))).cloned().unwrap_or_else(|| model.to_string())
}

/// Умеет ли агент менять модель на лету: через `configOptions` (id `model`, так у pi-acp) или `session/set_model` (Gemini).
pub fn acp_model_switchable(st: &AcpState) -> bool {
    st.config_options.as_array().map(|a| a.iter().any(|o| o.get("id").and_then(|i| i.as_str()) == Some("model"))).unwrap_or(false)
        || st.models.get("availableModels").and_then(|a| a.as_array()).map(|a| !a.is_empty()).unwrap_or(false)
}

impl Session {
    pub(crate) async fn acp_call(&self, method: &str, params: Value, timeout_s: u64) -> Result<Value> {
        let id = self.next_ctrl_id();
        let rx = self.ctrl_wait(&id);
        self.write(&json!({ "jsonrpc": "2.0", "id": id.parse::<u64>().map(Value::from).unwrap_or(Value::String(id.clone())), "method": method, "params": params })).await?;
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), rx).await {
            Ok(Ok(v)) => {
                if let Some(err) = v.get("error") { bail!("{method}: {} (код {})", err.get("message").and_then(|m| m.as_str()).unwrap_or("ошибка"), err.get("code").and_then(|c| c.as_i64()).unwrap_or(0)); }
                Ok(v.get("result").cloned().unwrap_or(Value::Null))
            }
            Ok(Err(_)) => bail!("процесс завершился до ответа на {method}"),
            Err(_) => { self.ctrl_forget(&id); bail!("нет ответа на {method} за {timeout_s} с") }
        }
    }

    async fn acp_notify(&self, method: &str, params: Value) -> Result<()> {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params })).await
    }

    fn acp_persist(&self) {
        let st = self.acp.lock().unwrap();
        let v = json!({ "modes": st.modes, "models": st.models, "commands": st.commands, "config_options": st.config_options, "model": st.model });
        let _ = std::fs::write(self.dir().join("acp.json"), serde_json::to_vec(&v).unwrap_or_default());
    }

    /// Старт: initialize → (authenticate) → session/load при возобновлении или session/new. Модель — у Gemini флагом при
    /// запуске, у остальных `session/set_model`, если агент отдал список моделей.
    pub(crate) async fn acp_start(self: &Arc<Self>, resume: bool) -> Result<()> {
        let meta = self.meta.lock().unwrap().clone();
        let engine = crate::engines::get(&meta.agent).ok_or_else(|| anyhow!("неизвестный движок {}", meta.agent))?;
        { let mut st = self.acp.lock().unwrap(); st.epoch += 1; st.session_id = None; st.loading = false; st.text.clear(); st.thought.clear(); st.tools.clear(); st.tool_order.clear(); }
        let init = self.acp_call("initialize", json!({ "protocolVersion": PROTOCOL_VERSION, "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false }, "clientInfo": { "name": "cloudos-agentd", "title": "AgentVerse OS", "version": env!("CARGO_PKG_VERSION") } }), 60).await?;
        {
            let mut st = self.acp.lock().unwrap();
            st.auth_methods = init.get("authMethods").and_then(|a| a.as_array()).cloned().unwrap_or_default();
            st.load_session = init.pointer("/agentCapabilities/loadSession").and_then(|b| b.as_bool()).unwrap_or(false);
        }
        let env = crate::codex::effective_env(&meta);
        let gate = env.contains_key("OPENAI_BASE_URL") && !env.contains_key("CODEX_HOME");
        // каталог моделей gate — на весь workspace (меню модели Gemini у остановленных сессий тоже)
        if engine.id == "gemini" && gate { let ids = crate::codex::gate_model_ids(&env).await; if !ids.is_empty() { if let Some(parent) = self.dir().parent() { let _ = std::fs::write(parent.join("gate-models.json"), serde_json::to_vec(&ids).unwrap_or_default()); } } }
        let can_load = self.acp.lock().unwrap().load_session;
        let mut restarted = false;
        let mut res = match (&meta.provider_session_id, resume && can_load) {
            (Some(sid), true) => {
                self.acp.lock().unwrap().loading = true;
                let r = self.acp_call("session/load", json!({ "sessionId": sid, "cwd": meta.cwd, "mcpServers": crate::mcp::acp_servers() }), 300).await;
                self.acp.lock().unwrap().loading = false;
                match r {
                    Ok(mut v) => { if v.get("sessionId").is_none() { v["sessionId"] = json!(sid); } Ok(v) }
                    Err(e) => { tracing::warn!("session {}: session/load: {e:#} — новая ACP-сессия на том же журнале", &meta.id[..8]); restarted = true; self.acp_call("session/new", json!({ "cwd": meta.cwd, "mcpServers": crate::mcp::acp_servers() }), 120).await }
                }
            }
            _ => self.acp_call("session/new", json!({ "cwd": meta.cwd, "mcpServers": crate::mcp::acp_servers() }), 120).await,
        };
        // нет входа: Gemini через gate — метод gateway (адрес из GOOGLE_GEMINI_BASE_URL), по ключу — gemini-api-key; затем ещё раз
        if let Err(e) = &res {
            let msg = e.to_string();
            if msg.contains("-32000") || msg.to_ascii_lowercase().contains("auth") {
                let method = if meta.agent == "gemini" {
                    if env.contains_key("GOOGLE_GEMINI_BASE_URL") || env.contains_key("GEMINI_BASE_URL") { Some("gateway") } else if env.contains_key("GEMINI_API_KEY") { Some("gemini-api-key") } else { None }
                } else { None };
                match method {
                    Some(m) => {
                        self.acp_call("authenticate", json!({ "methodId": m }), 120).await.map_err(|e2| anyhow!("вход {m}: {e2:#}"))?;
                        res = self.acp_call("session/new", json!({ "cwd": meta.cwd, "mcpServers": crate::mcp::acp_servers() }), 120).await;
                    }
                    None => bail!("{}: нужен вход — {} ({msg})", engine.title, engine.login_hint),
                }
            }
        }
        let r = res?;
        let sid = r.get("sessionId").and_then(|s| s.as_str()).ok_or_else(|| anyhow!("session/new без sessionId"))?.to_string();
        {
            let mut st = self.acp.lock().unwrap();
            st.session_id = Some(sid.clone());
            if let Some(m) = r.get("modes").filter(|m| m.is_object()) { st.modes = m.clone(); }
            if let Some(m) = r.get("models").filter(|m| m.is_object()) { st.models = m.clone(); st.model = m.get("currentModelId").and_then(|c| c.as_str()).map(str::to_string); }
            if let Some(c) = r.get("configOptions").filter(|c| c.is_array()) { st.config_options = c.clone(); }
        }
        {
            let mut m = self.meta.lock().unwrap();
            m.provider_session_id = Some(sid.clone());
            // модель из ответа агента (pi: currentModelId), если сессия своей не задавала
            if m.model.is_none() { if let Some(cur) = self.acp.lock().unwrap().model.clone() { m.model = Some(cur); } }
        }
        self.save_meta();
        // модель сессии: у Gemini уже флагом при запуске; у остальных — session/set_model, если агент отдал список
        let switchable = acp_model_switchable(&self.acp.lock().unwrap());
        if let Some(model) = meta.model.clone() {
            let current = self.acp.lock().unwrap().model.clone();
            if switchable && engine.id != "gemini" && current.as_deref().map(|c| c != model && !c.ends_with(&format!("/{model}"))).unwrap_or(true) {
                if let Err(e) = self.acp_apply_model(&sid, &model).await { tracing::warn!("session {}: set_model {model}: {e:#}", &meta.id[..8]); self.emit(json!({ "kind": "system", "subtype": "provider_error", "data": { "message": format!("модель {model}: {e:#}") } })); }
            }
        }
        // режим сессии (наш или Codex-овый) → режим агента, если он такие умеет
        let available: Vec<String> = self.acp.lock().unwrap().modes.get("availableModes").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect()).unwrap_or_default();
        if let Some(mode) = meta.permission_mode.as_deref() {
            if let Some(target) = map_mode(engine.id, mode, &available) {
                let cur = self.acp.lock().unwrap().modes.get("currentModeId").and_then(|c| c.as_str()).map(str::to_string);
                if cur.as_deref() != Some(target.as_str()) {
                    if let Err(e) = self.acp_call("session/set_mode", json!({ "sessionId": sid, "modeId": target }), 60).await { tracing::warn!("session {}: set_mode {target}: {e:#}", &meta.id[..8]); }
                    else { self.acp.lock().unwrap().modes["currentModeId"] = json!(target); }
                }
            }
        }
        self.acp_persist();
        let (modes, models, model) = { let st = self.acp.lock().unwrap(); (st.modes.clone(), st.models.clone(), st.model.clone().or(meta.model.clone())) };
        if resume && restarted { self.acp.lock().unwrap().recap_pending = true; }
        self.emit(json!({ "kind": "init", "model": model, "session_id": sid, "cwd": meta.cwd, "permission_mode": modes.get("currentModeId").cloned().unwrap_or(json!(meta.permission_mode)), "provider": engine.id, "protocol": "acp", "route": if gate { "gate" } else { "own" }, "modes": modes.get("availableModes"), "models": models.get("availableModels"), "loaded": resume && !restarted }));
        if restarted { self.emit(json!({ "kind": "system", "subtype": "thread_restarted", "data": { "message": "прежняя сессия агента не загрузилась — беседа продолжается в новой, история агента начата заново" } })); }
        if self.status() == Status::Starting { self.set_status(Status::Idle); self.drain_queue(); }
        Ok(())
    }

    /// Сообщение пользователя → session/prompt; ответ (stopReason) означает конец хода.
    /// Пересказ беседы из журнала (срез 38): движок не поднял свою сессию после рестарта — отдаём ему историю первым сообщением.
    fn journal_recap(&self, max_chars: usize) -> String {
        let events = self.events_since(0, 100_000);
        let mut lines: Vec<String> = Vec::new();
        for e in events.iter() {
            let kind = e.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            if e.get("parent_tool_use_id").map(|p| !p.is_null()).unwrap_or(false) { continue }
            let text = crate::session::event_text(e);
            if text.trim().is_empty() { continue }
            match kind { "user" => lines.push(format!("Владелец: {}", text.trim())), "assistant" => lines.push(format!("Агент: {}", text.trim())), _ => {} }
        }
        let mut out: Vec<String> = Vec::new(); let mut used = 0usize;
        for l in lines.iter().rev() { let n = l.chars().count() + 1; if used + n > max_chars { break } used += n; out.push(l.clone()); }
        out.reverse();
        out.join("\n")
    }

    pub(crate) async fn acp_prompt(self: &Arc<Self>, content: &Value) -> Result<()> {
        let sid = self.acp.lock().unwrap().session_id.clone().ok_or_else(|| anyhow!("ACP-сессия ещё не создана"))?;
        // после рестарта без session/load: первый промпт получает пересказ истории (до 8 тысяч знаков), дальше — как обычно
        let recap = if std::mem::replace(&mut self.acp.lock().unwrap().recap_pending, false) { let r = self.journal_recap(8000); if r.is_empty() { None } else { Some(r) } } else { None };
        let content: Value = match recap {
            Some(r) => { let mut arr = content.as_array().cloned().unwrap_or_default(); let prefix = format!("Это продолжение беседы после перезапуска; предыдущие сообщения:\n{r}\n---\n");
                self.emit(json!({ "kind": "system", "subtype": "recap_sent", "data": { "message": format!("после перезапуска агенту передан пересказ беседы ({} строк, {} знаков)", r.lines().count(), r.chars().count()) } }));
                match arr.iter_mut().find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")) { Some(b) => { let t = b.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(); b["text"] = json!(format!("{prefix}{t}")); } None => arr.insert(0, json!({ "type": "text", "text": prefix })) }
                Value::Array(arr) }
            None => content.clone(),
        };
        let content = &content;
        let blocks: Vec<Value> = content.as_array().cloned().unwrap_or_default().into_iter().filter_map(|b| match b.get("type").and_then(|t| t.as_str()) {
            Some("text") => Some(json!({ "type": "text", "text": b.get("text").and_then(|t| t.as_str()).unwrap_or("") })),
            Some("image") => Some(json!({ "type": "image", "data": b.pointer("/source/data").and_then(|d| d.as_str()).unwrap_or(""), "mimeType": b.pointer("/source/media_type").and_then(|d| d.as_str()).unwrap_or("image/png") })),
            _ => None,
        }).collect();
        { let mut st = self.acp.lock().unwrap(); st.turn_started_ms = now_ms(); st.turn_text.clear(); st.text.clear(); st.thought.clear(); st.in_turn = true; }
        let id = self.next_ctrl_id();
        let rx = self.ctrl_wait(&id);
        self.write(&json!({ "jsonrpc": "2.0", "id": id.parse::<u64>().map(Value::from).unwrap_or(Value::String(id.clone())), "method": "session/prompt", "params": { "sessionId": sid, "prompt": blocks } })).await?;
        let me = self.clone();
        tokio::spawn(async move {
            let r = tokio::time::timeout(std::time::Duration::from_secs(4 * 3600), rx).await;
            let (stop, err, meta): (String, Option<String>, Option<Value>) = match r {
                Ok(Ok(v)) => match v.get("error") {
                    Some(e) => ("error".into(), Some(e.get("message").and_then(|m| m.as_str()).unwrap_or("ошибка").to_string()), None),
                    None => (v.pointer("/result/stopReason").and_then(|s| s.as_str()).unwrap_or("end_turn").to_string(), None, v.pointer("/result/_meta").cloned()),
                },
                Ok(Err(_)) => return, // процесс вышел — exit уже в журнале
                Err(_) => { me.ctrl_forget(&id); ("error".into(), Some("нет ответа на ход за 4 часа".into()), None) }
            };
            me.acp_finish_turn(&stop, err, meta);
        });
        Ok(())
    }

    /// Конец хода: докладываем накопленное, пишем result (стоимости ACP не отдаёт — токены нулевые, ход учитывается).
    fn acp_finish_turn(self: &Arc<Self>, stop: &str, err: Option<String>, meta: Option<Value>) {
        self.acp_flush();
        self.acp.lock().unwrap().in_turn = false;
        let (started, text, model, usage, sid) = { let st = self.acp.lock().unwrap(); (st.turn_started_ms, st.turn_text.clone(), st.model.clone(), st.usage.clone(), st.session_id.clone()) };
        let (meta_model, agent) = { let m = self.meta.lock().unwrap(); (m.model.clone(), m.agent.clone()) };
        // pi-acp отдаёт end_turn и при ошибке модели (404/401 от gate) — без текста; правду берём из файла сессии pi
        let mut stop = stop.to_string(); let mut err = err;
        if agent == "pi" && stop == "end_turn" && text.trim().is_empty() && err.is_none() {
            if let Some(e) = pi_last_error(sid.as_deref()) { stop = "error".into(); err = Some(e); }
        }
        let stop = stop.as_str();
        let is_error = stop == "error" || stop == "refusal";
        let subtype = match stop { "end_turn" => "success", "cancelled" => "interrupted", "max_tokens" => "max_tokens", "max_turn_requests" => "max_turns", "refusal" => "refusal", _ => "error_during_execution" };
        if let Some(e) = &err { self.emit(json!({ "kind": "error", "message": e })); }
        // токены: Gemini кладёт их в `_meta.quota.token_count` ответа на prompt, pi — только в файл сессии
        let tokens: Value = if agent == "pi" { pi_turn_usage(sid.as_deref()).unwrap_or(Value::Null) } else {
            match meta.as_ref().and_then(|m| m.pointer("/quota/token_count")) {
                Some(tc) => json!({ "input_tokens": tc.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0), "output_tokens": tc.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0) }),
                None => Value::Null,
            }
        };
        let ev = json!({ "kind": "result", "subtype": subtype, "is_error": is_error, "duration_ms": now_ms().saturating_sub(started), "num_turns": 1, "total_cost_usd": Value::Null, "usage": tokens, "model_usage": Value::Null,
            "text": if text.trim().is_empty() { err.clone().unwrap_or_default() } else { text }, "session_id": self.acp.lock().unwrap().session_id, "model": model.or(meta_model), "stop_reason": stop, "context": usage });
        self.record_result(ev);
        self.pending.lock().unwrap().clear();
        self.set_status(Status::Idle);
        self.drain_queue();
    }

    /// Накопленные размышления и текст — в журнал.
    fn acp_flush(&self) {
        let (thought, text, model) = { let mut st = self.acp.lock().unwrap(); (std::mem::take(&mut st.thought), std::mem::take(&mut st.text), st.model.clone()) };
        if !thought.trim().is_empty() { self.emit(json!({ "kind": "assistant", "content": [{ "type": "thinking", "thinking": thought }] })); }
        if !text.trim().is_empty() { self.emit(json!({ "kind": "assistant", "content": [{ "type": "text", "text": text }], "model": model })); }
    }

    pub(crate) async fn acp_on_message(self: &Arc<Self>, v: Value) {
        let method = v.get("method").and_then(|m| m.as_str()).map(str::to_string);
        let id = v.get("id").cloned();
        match (method, id) {
            (None, Some(id)) => {
                let key = match &id { Value::Number(n) => n.to_string(), Value::String(s) => s.clone(), _ => String::new() };
                self.ctrl_resolve(&key, v);
            }
            (Some(m), Some(id)) => self.acp_on_request(&m, id, v.get("params").cloned().unwrap_or(Value::Null)).await,
            (Some(m), None) => self.acp_on_notification(&m, v.get("params").cloned().unwrap_or(Value::Null)),
            (None, None) => {}
        }
    }

    fn acp_on_notification(self: &Arc<Self>, method: &str, params: Value) {
        if method != "session/update" { return }
        if self.acp.lock().unwrap().loading { return }
        let u = params.get("update").cloned().unwrap_or(Value::Null);
        let kind = u.get("sessionUpdate").and_then(|k| k.as_str()).unwrap_or("");
        match kind {
            "agent_message_chunk" => {
                // вне хода (Gemini после session/set_mode шлёт уведомление текстом) — служебная строка, статус не трогаем
                let in_turn = self.acp.lock().unwrap().in_turn;
                if !in_turn { if let Some(t) = u.pointer("/content/text").and_then(|t| t.as_str()).filter(|t| !t.trim().is_empty()) { self.emit(json!({ "kind": "system", "subtype": "agent_note", "data": { "message": t.trim() } })); } return }
                if let Some(t) = u.pointer("/content/text").and_then(|t| t.as_str()) { let mut st = self.acp.lock().unwrap(); st.text.push_str(t); st.turn_text.push_str(t); }
                if self.status() == Status::Idle { self.set_status(Status::Working); }
            }
            "agent_thought_chunk" => { if let Some(t) = u.pointer("/content/text").and_then(|t| t.as_str()) { self.acp.lock().unwrap().thought.push_str(t); } }
            "user_message_chunk" => {}
            "tool_call" => {
                self.acp_flush();
                let tid = u.get("toolCallId").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let title = u.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let raw = u.get("rawInput").cloned().unwrap_or(Value::Null);
                let name = tool_name(u.get("kind").and_then(|k| k.as_str()), &title, &raw);
                let mut input = if raw.is_object() { raw.clone() } else { json!({}) };
                if input.get("title").is_none() && !title.is_empty() { input["title"] = json!(title); }
                // содержимое при старте вызова — описание (у Gemini: «[cwd] (зачем)»), а не вывод: в input.detail
                if let Some(c) = u.get("content").filter(|c| c.is_array()) { let txt = content_text(c); if !txt.trim().is_empty() && !txt.trim().starts_with("[terminal]") && input.get("detail").is_none() { input["detail"] = json!(txt.trim_end()); } }
                if let Some(l) = u.get("locations").filter(|l| l.as_array().map(|a| !a.is_empty()).unwrap_or(false)) { input["locations"] = l.clone(); }
                if name == "Bash" && input.get("command").is_none() && !title.is_empty() { input["command"] = json!(title); }
                { let mut st = self.acp.lock().unwrap(); st.tools.insert(tid.clone(), (name.clone(), title.clone(), String::new())); st.tool_order.push(tid.clone()); }
                self.emit(json!({ "kind": "assistant", "content": [{ "type": "tool_use", "id": tid, "name": name, "input": input }], "uuid": tid }));
                // уже завершён (агент прислал итог сразу)
                let status = u.get("status").and_then(|s| s.as_str()).unwrap_or("pending");
                if matches!(status, "completed" | "failed") { self.acp_tool_done(&tid, status, &u); }
                self.set_status(Status::Working);
            }
            "tool_call_update" => {
                let tid = u.get("toolCallId").and_then(|t| t.as_str()).unwrap_or("").to_string();
                // content в обновлении — полное состояние (не дельта): заменяем накопленный вывод
                if let Some(c) = u.get("content").filter(|c| c.is_array()) { let txt = content_text(c); if let Some(t) = self.acp.lock().unwrap().tools.get_mut(&tid) { if !txt.trim().is_empty() { t.2 = txt; } } }
                if let Some(status) = u.get("status").and_then(|s| s.as_str()) { if matches!(status, "completed" | "failed") { self.acp_tool_done(&tid, status, &u); } }
            }
            "plan" => { self.emit(json!({ "kind": "system", "subtype": "plan", "data": { "entries": u.get("entries") } })); }
            "available_commands_update" => { self.acp.lock().unwrap().commands = u.get("availableCommands").and_then(|a| a.as_array()).cloned().unwrap_or_default(); self.acp_persist(); }
            "current_mode_update" => {
                if let Some(m) = u.get("currentModeId").and_then(|m| m.as_str()) {
                    self.acp.lock().unwrap().modes["currentModeId"] = json!(m);
                    self.meta.lock().unwrap().permission_mode = Some(m.to_string()); self.save_meta(); self.acp_persist();
                    self.emit(json!({ "kind": "permission_mode", "mode": m }));
                }
            }
            "config_option_update" => { if let Some(c) = u.get("configOptions") { self.acp.lock().unwrap().config_options = c.clone(); self.acp_persist(); } }
            "usage_update" => { self.acp.lock().unwrap().usage = Some(u.clone()); }
            _ => {}
        }
    }

    fn acp_tool_done(self: &Arc<Self>, tid: &str, status: &str, u: &Value) {
        let (name, out) = { let mut st = self.acp.lock().unwrap(); match st.tools.remove(tid) { Some((n, _, o)) => (n, o), None => ("Tool".to_string(), String::new()) } };
        let mut text = out;
        if text.trim().is_empty() { if let Some(r) = u.get("rawOutput") { text = if let Some(s) = r.as_str() { s.to_string() } else { serde_json::to_string_pretty(r).unwrap_or_default() }; } }
        let _ = name;
        let agent = self.meta.lock().unwrap().agent.clone();
        if (agent == "pi" || agent == "gemini") && (text.trim().is_empty() || text.trim() == "[terminal]") {
            // вывод инструмента — из записи беседы самого движка (pi: toolResult, Gemini: toolCalls[].resultDisplay);
            // запись появляется чуть позже статуса completed, ждём до 3 с
            let (me, tid, status) = (Arc::clone(self), tid.to_string(), status.to_string());
            tokio::spawn(async move {
                let mut got: Option<(String, bool)> = None;
                for _ in 0..15 {
                    let (sid, skip) = { let st = me.acp.lock().unwrap(); (st.session_id.clone(), st.pi_tool_idx) };
                    if agent == "gemini" {
                        if let Some(r) = gemini_tool_output(sid.as_deref(), &tid) { got = Some(r); break; }
                    } else if let Some((t, e, n)) = pi_tool_output(sid.as_deref(), skip) { me.acp.lock().unwrap().pi_tool_idx = n; got = Some((t, e)); break; }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                let (t, e) = got.unwrap_or_default();
                me.emit(json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": tid, "content": t.chars().take(20000).collect::<String>(), "is_error": e || status == "failed" }] }));
            });
            return;
        }
        self.emit(json!({ "kind": "tool_result", "content": [{ "type": "tool_result", "tool_use_id": tid, "content": text.chars().take(20000).collect::<String>(), "is_error": status == "failed" }] }));
    }

    async fn acp_on_request(self: &Arc<Self>, method: &str, rid: Value, params: Value) {
        match method {
            "session/request_permission" => {
                let raw = match &rid { Value::Number(n) => n.to_string(), Value::String(s) => s.clone(), _ => "0".into() };
                let key = format!("acp{}-{raw}", self.acp.lock().unwrap().epoch);
                let tc = params.get("toolCall").cloned().unwrap_or(Value::Null);
                let tid = tc.get("toolCallId").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let title = tc.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let rawi = tc.get("rawInput").cloned().unwrap_or(Value::Null);
                let name = self.acp.lock().unwrap().tools.get(&tid).map(|t| t.0.clone()).unwrap_or_else(|| tool_name(tc.get("kind").and_then(|k| k.as_str()), &title, &rawi));
                let mut input = if rawi.is_object() { rawi.clone() } else { json!({}) };
                if input.get("title").is_none() && !title.is_empty() { input["title"] = json!(title); }
                if name == "Bash" && input.get("command").is_none() && !title.is_empty() { input["command"] = json!(title); }
                let detail = content_text(&tc.get("content").cloned().unwrap_or(json!([])));
                if !detail.trim().is_empty() { input["detail"] = json!(detail.chars().take(4000).collect::<String>()); }
                let options = params.get("options").cloned().unwrap_or(json!([]));
                let ev = self.emit(json!({ "kind": "permission_request", "request_id": key, "tool_name": name, "input": input, "question": false, "provider": "acp", "options": options, "tool_use_id": tid, "title": title }));
                let seq = ev.get("seq").and_then(|s| s.as_u64()).unwrap_or(0);
                self.pending.lock().unwrap().insert(key.clone(), Pending { request_id: key, tool_name: name, input: json!({ "options": options }), seq, kind: "acp".into(), suggestions: None, rpc_id: Some(rid) });
                self.set_status(Status::Waiting);
            }
            other => {
                // файлы и терминал мы не делегируем (сказано в initialize) — агент работает с диском сам
                let _ = self.write(&json!({ "jsonrpc": "2.0", "id": rid, "error": { "code": -32601, "message": format!("agentd: {other} не поддерживается") } })).await;
            }
        }
    }

    /// Ответ на запрос разрешения: выбранный вариант агента или отмена.
    pub(crate) async fn acp_answer(&self, p: &Pending, rid: &Value, behavior: &str, remember: bool) -> Result<()> {
        let options = p.input.get("options").cloned().unwrap_or(json!([]));
        let result = match pick_option(&options, behavior, remember) {
            Some(opt) => json!({ "outcome": { "outcome": "selected", "optionId": opt } }),
            None => json!({ "outcome": { "outcome": "cancelled" } }),
        };
        self.write(&json!({ "jsonrpc": "2.0", "id": rid, "result": result })).await
    }

    pub(crate) async fn acp_interrupt(&self) -> Result<Value> {
        let sid = self.acp.lock().unwrap().session_id.clone().ok_or_else(|| anyhow!("ACP-сессия ещё не создана"))?;
        self.acp_notify("session/cancel", json!({ "sessionId": sid })).await?;
        Ok(json!({ "ok": true }))
    }

    /// Смена модели у агента: `session/set_config_option` (configId `model`), если он отдал такую опцию, иначе `session/set_model`.
    async fn acp_apply_model(&self, sid: &str, model: &str) -> Result<String> {
        let (value, via_config) = { let st = self.acp.lock().unwrap(); (acp_model_value(&st, model), st.config_options.as_array().map(|a| a.iter().any(|o| o.get("id").and_then(|i| i.as_str()) == Some("model"))).unwrap_or(false)) };
        if via_config {
            let r = self.acp_call("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": value }), 60).await?;
            if let Some(c) = r.get("configOptions").filter(|c| c.is_array()) { self.acp.lock().unwrap().config_options = c.clone(); }
        } else {
            self.acp_call("session/set_model", json!({ "sessionId": sid, "modelId": value }), 60).await?;
        }
        let mut st = self.acp.lock().unwrap(); st.model = Some(value.clone()); st.models["currentModelId"] = json!(value); drop(st); self.acp_persist();
        Ok(value)
    }

    /// Режим: session/set_mode, если агент его знает (с переводом наших имён); иначе — к следующему запуску.
    pub(crate) async fn acp_set_mode(&self, mode: &str) -> Result<Value> {
        let (sid, available, engine) = { let st = self.acp.lock().unwrap(); (st.session_id.clone(), st.modes.get("availableModes").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect::<Vec<_>>()).unwrap_or_default(), self.meta.lock().unwrap().agent.clone()) };
        let Some(sid) = sid else { return Ok(json!({ "ok": true, "applied": "next_start" })) };
        match map_mode(&engine, mode, &available) {
            Some(target) => { self.acp_call("session/set_mode", json!({ "sessionId": sid, "modeId": target }), 60).await?; self.acp.lock().unwrap().modes["currentModeId"] = json!(target); self.acp_persist(); Ok(json!({ "ok": true, "mode": target })) }
            None => Ok(json!({ "ok": true, "applied": "next_start", "note": "агент не знает такого режима" })),
        }
    }

    /// Модель: session/set_model, если агент отдал список; иначе — при следующем запуске (Gemini берёт её флагом).
    pub(crate) async fn acp_set_model(&self, model: Option<&str>) -> Result<Value> {
        let (sid, has) = { let st = self.acp.lock().unwrap(); (st.session_id.clone(), acp_model_switchable(&st)) };
        match (sid, has, model) {
            (Some(sid), true, Some(m)) => { let v = self.acp_apply_model(&sid, m).await?; Ok(json!({ "ok": true, "applied": "live", "model": v })) }
            _ => Ok(json!({ "ok": true, "applied": "next_start" })),
        }
    }

    /// Часть caps для ACP-движка: модели (агента или gate), режимы с подписями, команды, контекст.
    pub(crate) fn acp_caps(&self, meta: &crate::session::Meta, gate_models: &[String]) -> Value {
        let mut st = self.acp.lock().unwrap();
        if st.models.is_null() && st.modes.is_null() {
            if let Some(v) = std::fs::read(self.dir().join("acp.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                st.modes = v.get("modes").cloned().unwrap_or(Value::Null); st.models = v.get("models").cloned().unwrap_or(Value::Null);
                st.commands = v.get("commands").and_then(|c| c.as_array()).cloned().unwrap_or_default(); st.config_options = v.get("config_options").cloned().unwrap_or(Value::Null);
                if st.model.is_none() { st.model = v.get("model").and_then(|m| m.as_str()).map(str::to_string); }
            }
        }
        let current = st.model.clone().or(meta.model.clone());
        let mut models: Vec<Value> = st.models.get("availableModels").and_then(|a| a.as_array()).cloned().unwrap_or_default().iter().filter_map(|m| {
            let id = m.get("modelId").and_then(|i| i.as_str())?;
            Some(json!({ "value": id, "label": m.get("name").and_then(|n| n.as_str()).unwrap_or(id), "description": m.get("description"), "default": current.as_deref() == Some(id) }))
        }).collect();
        // pi через gate: из всех провайдеров pi показываем только наш (`agentverse-gate/…`), подпись — без префикса
        if models.iter().any(|m| m.get("value").and_then(|v| v.as_str()).map(|v| v.starts_with("agentverse-gate/")).unwrap_or(false)) {
            models.retain(|m| m.get("value").and_then(|v| v.as_str()).map(|v| v.starts_with("agentverse-gate/")).unwrap_or(false));
            for m in models.iter_mut() { if let Some(v) = m.get("value").and_then(|v| v.as_str()).map(str::to_string) { m["label"] = json!(v.trim_start_matches("agentverse-gate/")); } }
            // порядок каталога gate случайный — семейства по рангу (Claude, GPT, Gemini, остальные, Devin), внутри по имени
            let rank = |m: &Value| { let id = m.get("label").and_then(|l| l.as_str()).unwrap_or(""); (if id.starts_with("devin/") { 4 } else if id.starts_with("claude-") { 0 } else if id.starts_with("gpt-") { 1 } else if id.starts_with("gemini-") { 2 } else { 3 }, id.to_string()) };
            models.sort_by_key(rank);
        }
        // Gemini через gate: собственный список CLI (auto, 2.5-pro, 3.8-flash…) gate не обслуживает — показываем Gemini из каталога gate
        if meta.agent == "gemini" && !gate_models.is_empty() {
            let mut ids = gemini_gate_models(gate_models);
            if let Some(c) = &current { if !ids.contains(c) { ids.insert(0, c.clone()); } }
            models = ids.into_iter().map(|id| json!({ "value": id, "label": id, "default": current.as_deref() == Some(id.as_str()) })).collect();
        }
        let modes: Vec<Value> = st.modes.get("availableModes").and_then(|a| a.as_array()).cloned().unwrap_or_default();
        let commands: Vec<Value> = st.commands.iter().filter_map(|c| { let name = c.get("name").and_then(|n| n.as_str())?; Some(json!({ "name": name, "description": c.get("description"), "argumentHint": c.pointer("/input/hint") })) }).collect();
        let usage = st.usage.clone();
        json!({
            "models": models, "mode_options": modes, "modes": modes.iter().filter_map(|m| m.get("id").cloned()).collect::<Vec<_>>(), "current_mode": st.modes.get("currentModeId"),
            "commands": commands, "config_options": st.config_options, "acp_usage": usage,
            "context_window": usage.as_ref().and_then(|u| u.get("size")).and_then(|s| s.as_u64()),
            "auth_methods": st.auth_methods, "model": current,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapping() {
        assert_eq!(tool_name(Some("execute"), "ls", &json!({})), "Bash");
        assert_eq!(tool_name(None, "run_shell_command", &json!({})), "Bash");
        assert_eq!(tool_name(None, "Edit file", &json!({})), "Edit");
        assert_eq!(tool_name(Some("fetch"), "", &json!({})), "WebFetch");
        assert_eq!(tool_name(None, "Mystery", &json!({ "command": "x" })), "Bash");
        assert_eq!(content_text(&json!([{ "type": "content", "content": { "type": "text", "text": "hi" } }, { "type": "diff", "path": "/a", "oldText": "x", "newText": "y" }])), "hi\n--- /a\n- x\n+ y\n");
        let g = ["default".to_string(), "autoEdit".into(), "yolo".into()];
        assert_eq!(map_mode("gemini", "acceptEdits", &g).as_deref(), Some("autoEdit"));
        assert_eq!(map_mode("gemini", "auto_edit", &g).as_deref(), Some("autoEdit"));
        assert_eq!(map_mode("gemini", "auto-edit", &[]).as_deref(), Some("autoEdit"));
        assert_eq!(map_mode("gemini", "bypassPermissions", &g).as_deref(), Some("yolo"));
        assert_eq!(map_mode("gemini", "plan", &g), None);
        assert_eq!(map_mode("gemini", "yolo", &g).as_deref(), Some("yolo"));
        assert_eq!(map_mode("pi", "default", &["low".to_string()]), None);
        let mut st = AcpState::default();
        st.config_options = json!([{ "id": "model", "options": [{ "value": "anthropic/claude-sonnet-4-6" }, { "value": "agentverse-gate/claude-sonnet-4-6" }, { "value": "agentverse-gate/gpt-5.5" }] }]);
        assert_eq!(acp_model_value(&st, "claude-sonnet-4-6"), "agentverse-gate/claude-sonnet-4-6");
        assert_eq!(acp_model_value(&st, "openai/gpt-5.5"), "openai/gpt-5.5");
        assert_eq!(acp_model_value(&st, "kimi-k2"), "kimi-k2");
        assert!(acp_model_switchable(&st));
        assert!(!acp_model_switchable(&AcpState::default()));
        assert_eq!(map_mode("pi", "low", &["low".to_string()]).as_deref(), Some("low"));
        let opts = json!([{ "optionId": "proceed_once", "kind": "allow_once" }, { "optionId": "proceed_always", "kind": "allow_always" }, { "optionId": "cancel", "kind": "reject_once" }]);
        assert_eq!(pick_option(&opts, "allow", false).as_deref(), Some("proceed_once"));
        assert_eq!(pick_option(&opts, "allow", true).as_deref(), Some("proceed_always"));
        assert_eq!(pick_option(&opts, "deny", true).as_deref(), Some("cancel"));
        assert_eq!(pick_option(&json!([]), "deny", false), None);
        let ids = ["gemini-3-flash".to_string(), "gemini-3.1-pro-low".into(), "gemini-3.1-flash-image".into(), "devin/gemini-3-1-pro".into(), "gemini-pro-agent".into(), "gemini-3.8-flash-high".into(), "gemini-3.5-flash-lite".into()];
        assert_eq!(pick_gemini_model(&ids).as_deref(), Some("gemini-3.8-flash-high"));
        assert_eq!(gemini_gate_models(&ids), vec!["gemini-3.8-flash-high".to_string(), "gemini-3.1-pro-low".into(), "gemini-pro-agent".into()]);
        assert_eq!(pick_gemini_model(&["gemini-3-flash".to_string(), "gemini-3.5-flash-lite".into()]), None);
        let pi: Vec<String> = ["claude-sonnet-4-20250514", "claude-opus-5-5", "claude-sonnet-4-6", "claude-sonnet-5-5", "gpt-5.5"].iter().map(|s| s.to_string()).collect();
        assert_eq!(pick_pi_model(&pi).as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(pick_pi_model(&["gpt-5.5".to_string(), "gpt-5.6-terra".into(), "kimi-k2".into()]).as_deref(), Some("gpt-5.6-terra"));
        assert_eq!(pick_pi_model(&["kimi-k2".to_string()]).as_deref(), Some("kimi-k2"));
    }
}
