//! Каталог движков (docs/agents-ux.md §4.3.8): какие агенты agentd умеет запускать, как их обнаружить в workspace и
//! поставить. Claude Code и Codex — родные адаптеры (session.rs, codex.rs); остальные — через ACP (acp.rs): Gemini CLI
//! говорит по ACP сам (`gemini --experimental-acp`), pi — через адаптер `pi-acp` поверх `pi --mode rpc`. Новый агент из
//! реестра ACP — ещё одна запись здесь, без кода.
use serde::Serialize;
use serde_json::{json, Value};
use std::process::Stdio;

#[derive(Clone, Debug, Serialize)]
pub struct Engine {
    pub id: &'static str,
    pub title: &'static str,
    /// Бинарь в PATH login-оболочки
    pub bin: &'static str,
    /// Остальные бинари, без которых движок не работает (pi-acp у pi)
    pub extra_bins: &'static [&'static str],
    /// npm-пакеты для установки (`npm install -g …`)
    pub npm: &'static [&'static str],
    /// `claude` | `codex` | `acp`
    pub protocol: &'static str,
    /// Команда ACP-агента (у родных пусто)
    pub acp: &'static [&'static str],
    /// Команда родного TUI
    pub tui: &'static [&'static str],
    /// Флаг модели в командной строке (ACP и TUI)
    pub model_flag: Option<&'static str>,
    pub icon: &'static str,
    /// Как войти в подписку без браузера (подсказка интерфейса)
    pub login_hint: &'static str,
}

pub const ENGINES: &[Engine] = &[
    Engine { id: "claude", title: "Claude Code", bin: "claude", extra_bins: &[], npm: &["@anthropic-ai/claude-code"], protocol: "claude", acp: &[], tui: &["claude"], model_flag: Some("--model"), icon: "claude", login_hint: "вход по ссылке OAuth в «Настройках → Аккаунты»" },
    Engine { id: "codex", title: "Codex", bin: "codex", extra_bins: &[], npm: &["@openai/codex"], protocol: "codex", acp: &[], tui: &["codex"], model_flag: Some("-m"), icon: "codex", login_hint: "вход по ссылке OAuth в «Настройках → Аккаунты»" },
    Engine { id: "gemini", title: "Gemini CLI", bin: "gemini", extra_bins: &[], npm: &["@google/gemini-cli"], protocol: "acp", acp: &["gemini", "--experimental-acp"], tui: &["gemini"], model_flag: Some("--model"), icon: "gemini", login_hint: "через gate проекта работает сразу; вход в Google — во вкладке CLI (ссылка и код)" },
    Engine { id: "pi", title: "pi", bin: "pi", extra_bins: &["pi-acp"], npm: &["@earendil-works/pi-coding-agent", "pi-acp"], protocol: "acp", acp: &["pi-acp"], tui: &["pi"], model_flag: Some("--model"), icon: "pi", login_hint: "через gate проекта работает сразу; подписки — /login во вкладке CLI" },
];

pub fn get(id: &str) -> Option<&'static Engine> { ENGINES.iter().find(|e| e.id == id) }
pub fn is_acp(id: &str) -> bool { get(id).map(|e| e.protocol == "acp").unwrap_or(false) }
pub fn title(id: &str) -> &'static str { get(id).map(|e| e.title).unwrap_or("Agent") }

/// Что установлено в workspace: путь и версия каждого бинаря из login-оболочки (PATH к ~/.npm-global). Версии
/// спрашиваются параллельно, общий предел 25 с.
pub async fn detect() -> Vec<Value> {
    let mut script = String::from("set +e\n");
    for e in ENGINES {
        for b in std::iter::once(e.bin).chain(e.extra_bins.iter().copied()) {
            script.push_str(&format!("( p=$(command -v {b} 2>/dev/null); v=''; if [ -n \"$p\" ]; then v=$(timeout 15 {b} --version 2>/dev/null | head -1); fi; printf '%s|%s|%s\\n' '{b}' \"$p\" \"$v\" ) &\n"));
        }
    }
    script.push_str("wait\n");
    let out = tokio::time::timeout(std::time::Duration::from_secs(25), tokio::process::Command::new("bash").args(["-lc", &script]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).output()).await;
    let text = match out { Ok(Ok(o)) => String::from_utf8_lossy(&o.stdout).to_string(), _ => String::new() };
    let mut found: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
    for line in text.lines() {
        let mut it = line.splitn(3, '|');
        if let (Some(b), Some(p), v) = (it.next(), it.next(), it.next().unwrap_or("")) { found.insert(b.to_string(), (p.to_string(), v.trim().to_string())); }
    }
    ENGINES.iter().map(|e| {
        let main = found.get(e.bin).filter(|(p, _)| !p.is_empty());
        let extras_ok = e.extra_bins.iter().all(|b| found.get(*b).map(|(p, _)| !p.is_empty()).unwrap_or(false));
        let missing: Vec<&str> = std::iter::once(e.bin).chain(e.extra_bins.iter().copied()).filter(|b| !found.get(*b).map(|(p, _)| !p.is_empty()).unwrap_or(false)).collect();
        json!({ "id": e.id, "title": e.title, "protocol": e.protocol, "icon": e.icon, "bin": e.bin, "npm": e.npm, "installed": main.is_some() && extras_ok,
            "path": main.map(|(p, _)| p.clone()), "version": main.map(|(_, v)| version_of(v)), "missing": missing, "login_hint": e.login_hint, "model_flag": e.model_flag })
    }).collect()
}

/// Версия из первой строки `--version` («2.1.284 (Claude Code)», «codex-cli 0.158.0», «0.62.0»): первое число с точкой.
fn version_of(line: &str) -> String {
    line.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',').map(|t| t.trim_start_matches('v')).find(|t| t.contains('.') && t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)).unwrap_or(line).to_string()
}

/// Команда установки движка (в терминале, чтобы владелец видел ход): `npm install -g <пакеты>`.
pub fn install_cmd(e: &Engine) -> Vec<String> {
    let pkgs = e.npm.iter().map(|p| format!("{p}@latest")).collect::<Vec<_>>().join(" ");
    vec!["bash".into(), "-lc".into(), format!("set -e; echo 'Установка {}: npm install -g {pkgs}'; npm install -g {pkgs} && echo && echo 'Готово: {} установлен.' && {} --version; echo; echo 'Эту вкладку можно закрыть.'; exec bash -l", e.title, e.title, e.bin)]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog() {
        assert!(is_acp("gemini") && is_acp("pi") && !is_acp("claude") && !is_acp("codex") && !is_acp("nope"));
        assert_eq!(title("pi"), "pi"); assert_eq!(title("x"), "Agent");
        assert_eq!(version_of("2.1.284 (Claude Code)"), "2.1.284"); assert_eq!(version_of("codex-cli 0.158.0"), "0.158.0"); assert_eq!(version_of("0.62.0"), "0.62.0"); assert_eq!(version_of("pi v1.0.0"), "1.0.0");
        let c = install_cmd(get("pi").unwrap()); assert!(c[2].contains("npm install -g @earendil-works/pi-coding-agent@latest pi-acp@latest"));
    }
}

/// Движок по команде PTY («bash -lic "exec pi"», «gemini --model …»): первое слово любого аргумента, совпадающее с бинарём
/// ACP-движка. Для TUI из меню «+» — чтобы подготовить конфигурацию движка так же, как перед сессией.
pub fn in_cmd(cmd: &[String]) -> Option<&'static Engine> {
    for a in cmd {
        for w in a.split_whitespace() {
            let w = w.rsplit('/').next().unwrap_or(w);
            if let Some(e) = ENGINES.iter().find(|e| e.protocol == "acp" && e.bin == w) { return Some(e) }
        }
    }
    None
}
