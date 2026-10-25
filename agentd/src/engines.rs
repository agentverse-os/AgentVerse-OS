//! Engine catalog (docs/agents-ux.md §4.3.8): which agents agentd can run, how to detect them in a workspace and how to
//! install them. Claude Code and Codex are native adapters (session.rs, codex.rs); the rest go through ACP (acp.rs):
//! Gemini CLI speaks ACP itself (`gemini --experimental-acp`), pi through the `pi-acp` adapter on top of `pi --mode
//! rpc`. A new agent from the ACP registry is one more entry here, no code.
use serde::Serialize;
use serde_json::{json, Value};
use std::process::Stdio;

#[derive(Clone, Debug, Serialize)]
pub struct Engine {
    pub id: &'static str,
    pub title: &'static str,
    /// Binary on the PATH of the login shell
    pub bin: &'static str,
    /// Other binaries the engine cannot work without (pi-acp for pi)
    pub extra_bins: &'static [&'static str],
    /// npm packages to install (`npm install -g …`)
    pub npm: &'static [&'static str],
    /// `claude` | `codex` | `acp`
    pub protocol: &'static str,
    /// ACP agent command (empty for native
    /// adapters)
    pub acp: &'static [&'static str],
    /// Native TUI command
    pub tui: &'static [&'static str],
    /// Model flag on the command line (ACP and
    /// TUI)
    pub model_flag: Option<&'static str>,
    pub icon: &'static str,
    /// How to sign in to the subscription without a browser (UI
    /// hint)
    pub login_hint: &'static str,
    /// Install not from npm: shell command before `npm install` (the Antigravity binary is installed by
    /// Google's script)
    pub install_sh: &'static str,
    /// What to do in the same tab after the install instead of "you can close this tab" (Antigravity: Google sign-in in agy
    /// right away)
    pub post_install: &'static str,
}

pub const ENGINES: &[Engine] = &[
    Engine { id: "claude", title: "Claude Code", bin: "claude", extra_bins: &[], npm: &["@anthropic-ai/claude-code"], protocol: "claude", acp: &[], tui: &["claude"], model_flag: Some("--model"), icon: "claude", login_hint: "вход по ссылке OAuth в «Настройках → Аккаунты»", install_sh: "", post_install: "" },
    Engine { id: "codex", title: "Codex", bin: "codex", extra_bins: &[], npm: &["@openai/codex"], protocol: "codex", acp: &[], tui: &["codex"], model_flag: Some("-m"), icon: "codex", login_hint: "вход по ссылке OAuth в «Настройках → Аккаунты»", install_sh: "", post_install: "" },
    Engine { id: "gemini", title: "Gemini CLI", bin: "gemini", extra_bins: &[], npm: &["@google/gemini-cli"], protocol: "acp", acp: &["gemini", "--experimental-acp"], tui: &["gemini"], model_flag: Some("--model"), icon: "gemini", login_hint: "через gate проекта работает сразу; вход в Google — во вкладке CLI (ссылка и код)", install_sh: "", post_install: "" },
    Engine { id: "pi", title: "pi", bin: "pi", extra_bins: &["pi-acp"], npm: &["@earendil-works/pi-coding-agent", "pi-acp"], protocol: "acp", acp: &["pi-acp"], tui: &["pi"], model_flag: Some("--model"), icon: "pi", login_hint: "через gate проекта работает сразу; подписки — /login во вкладке CLI", install_sh: "", post_install: "" },
    // Oh My Pi (`omp`, docs/agents-ux.md §4.3.18): a pi fork with its own ACP (`omp acp`); the npm package needs Bun, so the
    // prebuilt binary is installed by the omp.sh script (--binary) into ~/.local/bin; the gate provider goes into
    // ~/.omp/agent/models.json (acp.rs omp_ensure_gate)
    Engine { id: "omp", title: "Oh My Pi", bin: "omp", extra_bins: &[], npm: &[], protocol: "acp", acp: &["omp", "acp"], tui: &["omp"], model_flag: Some("--model"), icon: "omp",
        login_hint: "через gate проекта работает сразу (провайдер agentverse-gate в ~/.omp/agent/models.json); свои ключи и OAuth — `omp login` во вкладке CLI",
        install_sh: "curl -fsSL https://omp.sh/install | sh -s -- --binary && export PATH=\"$HOME/.local/bin:$PATH\" && omp --version", post_install: "" },
    // Antigravity CLI (`agy`, docs/agents-ux.md §4.3.17): Google's closed binary installed by its script into ~/.local/bin; it speaks ACP
    // through the google-antigravity-acp adapter on top of agy's headless mode (stream-json); sign-in is Google OAuth inside agy itself
    // (TUI in the CLI tab)
    Engine { id: "antigravity", title: "Antigravity", bin: "agy", extra_bins: &[], npm: &[], protocol: "agy", acp: &[], tui: &["agy"], model_flag: Some("-m"), icon: "antigravity",
        login_hint: "через gate проекта работает сразу (режим ключа Gemini API у agy); без gate — вход через Google в терминале agy (меню «+» → «Antigravity TUI», пункт Google OAuth, ссылка)",
        install_sh: "curl -fsSL https://antigravity.google/cli/install.sh | bash && export PATH=\"$HOME/.local/bin:$PATH\" && agy --version",
        post_install: "echo 'Вход в Google: сейчас запустится agy — выберите «Google OAuth», откройте ссылку в браузере и завершите вход. Потом закройте agy или эту вкладку: в меню «+» появится «Чат с Antigravity».'; sleep 4; exec agy" },
];

pub fn get(id: &str) -> Option<&'static Engine> { ENGINES.iter().find(|e| e.id == id) }
pub fn is_acp(id: &str) -> bool { get(id).map(|e| e.protocol == "acp").unwrap_or(false) }
/// Antigravity: our own adapter on top of headless `agy` (agy.rs),
/// not ACP
pub fn is_agy(id: &str) -> bool { get(id).map(|e| e.protocol == "agy").unwrap_or(false) }
pub fn title(id: &str) -> &'static str { get(id).map(|e| e.title).unwrap_or("Agent") }

/// Binary directories the workspace login shell may not know about (`.profile` without `~/.local/bin`): the Antigravity
/// and Oh My Pi installers put their binaries into `~/.local/bin`, npm into `~/.npm-global/bin`. We add them ourselves
/// on detection and when launching engines, so the catalog and sessions do not depend on the owner's profile.
pub const PATH_PREFIX: &str = "export PATH=\"$HOME/.local/bin:$HOME/.npm-global/bin:$PATH\"; ";

/// What is installed in the workspace: path and version of every binary from the login shell (PATH to ~/.npm-global).
/// Versions are queried in parallel, 25 s overall limit.
pub async fn detect() -> Vec<Value> {
    let mut script = String::from("set +e\n"); script.push_str(PATH_PREFIX); script.push('\n');
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

/// Version from the first line of `--version` ("2.1.284 (Claude Code)", "codex-cli 0.158.0", "0.62.0"): the first
/// number with a dot.
fn version_of(line: &str) -> String {
    line.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',' || c == '/').map(|t| t.trim_start_matches('v')).find(|t| t.contains('.') && t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)).unwrap_or(line).to_string()
}

/// "Engine set changed" stamp (relative to $HOME): the install command touches it at the end, and the catalog (main.rs
/// engines_list) is re-read even if its cache was filled while the install was running; otherwise "Chat with …" showed up in
/// the "+" menu only five minutes after "Done".
pub const STAMP: &str = ".cache/cloudos/engines.stamp";

/// Catalog version for the hub: STAMP mtime in seconds (0 means no installs yet)
pub fn stamp_secs() -> u64 {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    std::fs::metadata(std::path::Path::new(&home).join(STAMP)).and_then(|m| m.modified()).ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Is the stamp newer than `since` (the catalog cache was filled before
/// the install finished)
pub fn stamp_after(since: std::time::SystemTime) -> bool {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/coder".into());
    stamp_after_in(std::path::Path::new(&home), since)
}
fn stamp_after_in(home: &std::path::Path, since: std::time::SystemTime) -> bool {
    std::fs::metadata(home.join(STAMP)).and_then(|m| m.modified()).map(|m| m > since).unwrap_or(false)
}

/// Engine install command (in a terminal, so the owner sees the progress): `npm install -g <packages>`; at the end the STAMP
/// mark and either "you can close this tab" or the engine's post_install (Antigravity without the gate: Google sign-in in
/// agy). `gate` means the engine will run through the project gate (Antigravity in Gemini API-key mode): no sign-in needed,
/// the tab can be closed.
pub fn install_cmd(e: &Engine, gate: bool) -> Vec<String> {
    let pkgs = e.npm.iter().map(|p| format!("{p}@latest")).collect::<Vec<_>>().join(" ");
    let pre = if e.install_sh.is_empty() { String::new() } else { format!("echo 'Установка {}: скрипт поставщика'; {}; echo; ", e.title, e.install_sh) };
    let stamp = format!("( mkdir -p \"$HOME/.cache/cloudos\" && touch \"$HOME/{STAMP}\" ) 2>/dev/null || true; ");
    let tail = if e.post_install.is_empty() { "echo 'Эту вкладку можно закрыть.'; exec bash -l".to_string() }
        else if gate { format!("echo '{} будет работать через gate проекта — вход в Google не нужен. Эту вкладку можно закрыть.'; exec bash -l", e.title) }
        else { e.post_install.to_string() };
    // no npm packages (Antigravity: Google's script only) — skip the npm
    // step
    let main = if pkgs.is_empty() { format!("echo 'Готово: {} установлен.' && {} --version", e.title, e.bin) } else { format!("echo 'Установка {}: npm install -g {pkgs}'; npm install -g {pkgs} && echo && echo 'Готово: {} установлен.' && {} --version", e.title, e.title, e.bin) };
    vec!["bash".into(), "-lc".into(), format!("set -e; {pre}{main}; echo; {stamp}{tail}")]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog() {
        assert!(is_acp("gemini") && is_acp("pi") && !is_acp("claude") && !is_acp("codex") && !is_acp("nope"));
        assert_eq!(title("pi"), "pi"); assert_eq!(title("x"), "Agent");
        assert_eq!(version_of("2.1.284 (Claude Code)"), "2.1.284"); assert_eq!(version_of("codex-cli 0.158.0"), "0.158.0"); assert_eq!(version_of("0.62.0"), "0.62.0"); assert_eq!(version_of("pi v1.0.0"), "1.0.0"); assert_eq!(version_of("omp/18.12.6"), "18.12.6");
        let c = install_cmd(get("pi").unwrap(), false); assert!(c[2].contains("npm install -g @earendil-works/pi-coding-agent@latest pi-acp@latest"));
        let g = install_cmd(get("antigravity").unwrap(), true); assert!(g[2].contains("через gate") && g[2].ends_with("exec bash -l") && !g[2].contains("exec agy"), "{}", g[2]);
        let a = install_cmd(get("antigravity").unwrap(), false); assert!(a[2].contains("antigravity.google/cli/install.sh") && !a[2].contains("npm install") && a[2].contains("Готово: Antigravity установлен."), "{}", a[2]);
        assert!(a[2].contains(STAMP) && a[2].ends_with("exec agy") && !a[2].contains("exec bash -l") && a[2].find(STAMP).unwrap() < a[2].find("exec agy").unwrap(), "{}", a[2]);
        assert!(c[2].contains(STAMP) && c[2].ends_with("exec bash -l") && c[2].find(STAMP).unwrap() < c[2].find("Эту вкладку").unwrap(), "{}", c[2]);
        let dir = std::env::temp_dir().join(format!("agentd-stamp-{}", std::process::id())); let _ = std::fs::remove_dir_all(&dir);
        let before = std::time::SystemTime::now() - std::time::Duration::from_secs(5);
        assert!(!stamp_after_in(&dir, before), "без метки — кэш в силе");
        std::fs::create_dir_all(dir.join(".cache/cloudos")).unwrap(); std::fs::write(dir.join(STAMP), b"").unwrap();
        assert!(stamp_after_in(&dir, before) && !stamp_after_in(&dir, std::time::SystemTime::now() + std::time::Duration::from_secs(5)), "метка моложе кэша → перечитать, старше → нет");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!is_acp("antigravity") && is_agy("antigravity") && !is_agy("pi"));
        assert!(is_acp("omp") && get("omp").unwrap().acp == &["omp", "acp"] && get("omp").unwrap().npm.is_empty() && get("omp").unwrap().install_sh.contains("omp.sh/install"));
        let o = install_cmd(get("omp").unwrap(), true); assert!(o[2].contains("--binary") && !o[2].contains("npm install") && o[2].ends_with("exec bash -l"), "{}", o[2]);
        assert_eq!(in_cmd(&["bash".into(), "-lic".into(), "exec omp --model x".into()]).map(|e| e.id), Some("omp")); assert!(get("antigravity").unwrap().acp.is_empty() && get("antigravity").unwrap().npm.is_empty());
        assert!(login_help("antigravity", "Internal error: agy process exited before emitting init event (code: 1)").unwrap().contains("Google OAuth"));
        assert!(login_help("antigravity", "spawn ENOENT").is_none() && login_help("pi", "authentication required").is_none());
    }
}

/// Sign-in hint from the start-up error text: Antigravity without a Google sign-in — the adapter fails with "agy process exited
/// before emitting init event" / "authentication required"; the other engines sign in through the gate or accounts, no hint
/// needed
pub fn login_help(engine: &str, error: &str) -> Option<String> {
    let e = get(engine)?;
    if e.id != "antigravity" { return None }
    let t = error.to_ascii_lowercase();
    if t.contains("before emitting init") || t.contains("authentication") || t.contains("sign in") || t.contains("not signed") {
        let _ = e;
        Some("Antigravity не вошёл в Google — откройте терминал agy (меню «+» → «Antigravity TUI»), выберите Google OAuth и пройдите по ссылке; через gate проекта вход не нужен — проверьте ключ и адрес gate в окружении workspace'а. После входа отправьте сообщение ещё раз.".to_string())
    } else { None }
}

/// Engine by a PTY command ("bash -lic \"exec pi\"", "gemini --model …"): the first word of any argument that matches an
/// ACP engine binary. For a TUI from the "+" menu, so the engine configuration is prepared the same way as before a
/// session.
pub fn in_cmd(cmd: &[String]) -> Option<&'static Engine> {
    for a in cmd {
        for w in a.split_whitespace() {
            let w = w.rsplit('/').next().unwrap_or(w);
            if let Some(e) = ENGINES.iter().find(|e| (e.protocol == "acp" || e.protocol == "agy") && e.bin == w) { return Some(e) }
        }
    }
    None
}
