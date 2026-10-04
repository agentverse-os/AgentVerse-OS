//! MCP-серверы workspace'а (срез 36, docs/agents-ux.md §4.3.10): один список в `~/.cloudos/mcp.json`; при сохранении
//! раскладывается всем движкам их же командами (`claude mcp add -s user`, `codex mcp add`, `gemini mcp add -s user`), снятые
//! серверы убираются `mcp remove` (помним только свои имена — чужие записи владельца не трогаем); ACP-сессии получают тот же
//! список в `session/new`.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct McpServer {
    pub name: String,
    /// stdio | http | sse
    #[serde(default = "d_stdio")] pub transport: String,
    #[serde(default)] pub command: String,
    #[serde(default)] pub args: Vec<String>,
    #[serde(default)] pub url: String,
    #[serde(default)] pub env: HashMap<String, String>,
    #[serde(default)] pub headers: HashMap<String, String>,
}
fn d_stdio() -> String { "stdio".into() }

#[derive(Serialize, Deserialize, Default)]
struct File { #[serde(default)] servers: Vec<McpServer>, #[serde(default)] managed: Vec<String> }

fn home() -> String { std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into()) }
fn path() -> PathBuf { PathBuf::from(format!("{}/.cloudos/mcp.json", home())) }
fn load() -> File { std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() }
pub fn servers() -> Vec<McpServer> { load().servers }
pub fn count() -> usize { load().servers.len() }

pub fn valid_name(n: &str) -> bool { !n.is_empty() && n.chars().count() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) }

/// Список в формате ACP `session/new.mcpServers`: stdio — {name, command, args, env:[{name,value}]}, http/sse — {type, name, url, headers:[…]}.
pub fn acp_servers() -> Vec<Value> {
    servers().iter().map(|s| {
        if s.transport == "stdio" {
            json!({ "name": s.name, "command": s.command, "args": s.args, "env": s.env.iter().map(|(k, v)| json!({ "name": k, "value": v })).collect::<Vec<_>>() })
        } else {
            json!({ "type": s.transport, "name": s.name, "url": s.url, "headers": s.headers.iter().map(|(k, v)| json!({ "name": k, "value": v })).collect::<Vec<_>>() })
        }
    }).collect()
}

/// Сколько серверов знает каждый CLI (их конфиги): проверка, что раскладка дошла.
pub fn engine_counts() -> Value {
    let h = home();
    let json_keys = |p: String, key: &str| -> Option<usize> { serde_json::from_slice::<Value>(&std::fs::read(p).ok()?).ok()?.get(key)?.as_object().map(|o| o.len()) };
    let codex = std::fs::read_to_string(format!("{h}/.codex/config.toml")).ok().map(|t| t.lines().filter(|l| l.trim_start().starts_with("[mcp_servers.")).filter(|l| l.matches('.').count() == 1 || !l.contains(".env")).count());
    json!({ "claude": json_keys(format!("{h}/.claude.json"), "mcpServers"), "codex": codex, "gemini": json_keys(format!("{h}/.gemini/settings.json"), "mcpServers") })
}

fn sh(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

/// Команды одного движка: снять устаревшие и свои прежние записи, добавить текущие. Возвращает None, если движку нечего делать.
fn script_for(engine: &str, stale: &[String], servers: &[McpServer]) -> Option<String> {
    let mut sc = String::from("set +e\n");
    let (bin, scope) = match engine { "claude" => ("claude", " -s user"), "codex" => ("codex", ""), "gemini" => ("gemini", " -s user"), _ => return None };
    sc.push_str(&format!("command -v {bin} >/dev/null 2>&1 || {{ echo '__missing__'; exit 0; }}\n"));
    for n in stale.iter().chain(servers.iter().map(|s| &s.name)) { sc.push_str(&format!("{bin} mcp remove{scope} {} >/dev/null 2>&1\n", sh(n))); }
    for s in servers {
        let name = sh(&s.name);
        let line = match (engine, s.transport.as_str()) {
            // claude: `-e` многозначный — после имени, перед `--` (иначе съедает имя); gemini (yargs): `-e` по одному значению до
            // имени, а `--` — после команды, перед аргументами (проверено на 0.62: `name cmd -- -y pkg`)
            ("claude", "stdio") => {
                let env: String = s.env.iter().map(|(k, v)| format!(" -e {}", sh(&format!("{k}={v}")))).collect();
                format!("{bin} mcp add{scope} {name}{env} -- {} {}", sh(&s.command), s.args.iter().map(|a| sh(a)).collect::<Vec<_>>().join(" "))
            }
            ("gemini", "stdio") => {
                let env: String = s.env.iter().map(|(k, v)| format!(" -e {}", sh(&format!("{k}={v}")))).collect();
                let args = if s.args.is_empty() { String::new() } else { format!(" -- {}", s.args.iter().map(|a| sh(a)).collect::<Vec<_>>().join(" ")) };
                format!("{bin} mcp add{scope} -t stdio{env} {name} {}{args}", sh(&s.command))
            }
            ("claude", t) | ("gemini", t) => {
                let hd: String = s.headers.iter().map(|(k, v)| format!(" -H {}", sh(&format!("{k}: {v}")))).collect();
                format!("{bin} mcp add{scope} -t {t} {name} {}{hd}", sh(&s.url))
            }
            ("codex", "stdio") => {
                let env: String = s.env.iter().map(|(k, v)| format!(" --env {}", sh(&format!("{k}={v}")))).collect();
                format!("{bin} mcp add {name}{env} -- {} {}", sh(&s.command), s.args.iter().map(|a| sh(a)).collect::<Vec<_>>().join(" "))
            }
            ("codex", "http") => format!("{bin} mcp add {name} --url {}", sh(&s.url)),
            ("codex", _) => format!("echo '__skip__ {}: codex не умеет sse'", s.name),
            _ => continue,
        };
        sc.push_str(&format!("out=$({line} 2>&1) || echo \"__err__ {}: $(echo \"$out\" | tail -1)\"\n", s.name.replace('"', "")));
    }
    Some(sc)
}

async fn run(engine: &str, script: String) -> String {
    let out = tokio::time::timeout(std::time::Duration::from_secs(90), tokio::process::Command::new("bash").args(["-lc", &script]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).output()).await;
    let text = match out { Ok(Ok(o)) => format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)), Ok(Err(e)) => format!("__err__ {engine}: {e}"), Err(_) => format!("__err__ {engine}: нет ответа 90 с") };
    if text.contains("__missing__") { return "не установлен".into() }
    let problems: Vec<&str> = text.lines().filter(|l| l.starts_with("__err__") || l.starts_with("__skip__")).map(|l| l.trim_start_matches("__err__ ").trim_start_matches("__skip__ ")).collect();
    if problems.is_empty() { "ok".into() } else { problems.join("; ").chars().take(300).collect() }
}

/// Сохранить список и разложить движкам. Ошибка — только у валидации; проблемы движков возвращаются в `applied`.
pub async fn save(servers: Vec<McpServer>) -> Result<Value, String> {
    let mut seen = std::collections::HashSet::new();
    for s in &servers {
        if !valid_name(&s.name) { return Err(format!("имя «{}»: латиница, цифры, «-», «_», «.», до 64 знаков", s.name)) }
        if !seen.insert(s.name.clone()) { return Err(format!("имя «{}» повторяется", s.name)) }
        match s.transport.as_str() {
            "stdio" => if s.command.trim().is_empty() { return Err(format!("{}: нужна команда", s.name)) },
            "http" | "sse" => if !(s.url.starts_with("http://") || s.url.starts_with("https://")) { return Err(format!("{}: адрес должен начинаться с http:// или https://", s.name)) },
            t => return Err(format!("{}: неизвестный транспорт «{t}»", s.name)),
        }
    }
    let prev = load();
    let stale: Vec<String> = prev.managed.iter().filter(|n| !servers.iter().any(|s| &s.name == *n)).cloned().collect();
    let f = File { servers: servers.clone(), managed: servers.iter().map(|s| s.name.clone()).collect() };
    if let Some(d) = path().parent() { let _ = std::fs::create_dir_all(d); }
    std::fs::write(path(), serde_json::to_vec_pretty(&f).unwrap_or_default()).map_err(|e| format!("запись {}: {e}", path().display()))?;
    let mut applied = serde_json::Map::new();
    let jobs: Vec<(&'static str, String)> = ["claude", "codex", "gemini"].iter().filter_map(|e| script_for(e, &stale, &servers).map(|s| (*e, s))).collect();
    let handles: Vec<_> = jobs.into_iter().map(|(e, s)| tokio::spawn(async move { (e, run(e, s).await) })).collect();
    for h in handles { if let Ok((e, r)) = h.await { applied.insert(e.to_string(), json!(r)); } }
    Ok(json!({ "servers": servers, "applied": applied, "engines": engine_counts() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scripts_and_formats() {
        assert!(valid_name("context7") && valid_name("fs_tools.v2") && !valid_name("bad name") && !valid_name(""));
        let s = McpServer { name: "echo".into(), transport: "stdio".into(), command: "npx".into(), args: vec!["-y".into(), "@x/mcp".into()], env: HashMap::from([("K".to_string(), "v w".to_string())]), ..Default::default() };
        let h = McpServer { name: "ctx".into(), transport: "http".into(), url: "https://mcp.example.com/mcp".into(), headers: HashMap::from([("Authorization".to_string(), "Bearer t".to_string())]), ..Default::default() };
        let c = script_for("claude", &["old".into()], &[s.clone(), h.clone()]).unwrap();
        assert!(c.contains("claude mcp remove -s user 'old'") && c.contains("claude mcp add -s user 'echo' -e 'K=v w' -- 'npx' '-y' '@x/mcp'") && c.contains("claude mcp add -s user -t http 'ctx' 'https://mcp.example.com/mcp' -H 'Authorization: Bearer t'"));
        let x = script_for("codex", &[], &[s.clone(), h.clone()]).unwrap();
        assert!(x.contains("codex mcp add 'echo' --env 'K=v w' -- 'npx' '-y' '@x/mcp'") && x.contains("codex mcp add 'ctx' --url 'https://mcp.example.com/mcp'"));
        let g = script_for("gemini", &[], &[s]).unwrap();
        assert!(g.contains("gemini mcp add -s user -t stdio -e 'K=v w' 'echo' 'npx' -- '-y' '@x/mcp'"), "{g}");
        assert_eq!(sh("it's"), "'it'\\''s'");
    }
}
