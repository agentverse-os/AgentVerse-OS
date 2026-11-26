//! Memory pressure watch for workspaces (ADR-009). A workspace container that sits at its memory limit can stall for hours:
//! the kernel keeps evicting file pages and reading them back, reclaim "succeeds", so the cgroup OOM killer never fires.
//! The core watches the PSI of every running workspace from the host (`/sys/fs/cgroup/lxc.payload.<instance>`), reports
//! the state (`ok` / `tight` / `stalled`) and, when a stall holds, ends the largest process of that container the way the
//! kernel OOM killer would — or only reports it (`action: notify`). The same pass protects the workspace infrastructure
//! (init, agentd, the Coder agent, dockerd, containerd) from the kernel OOM killer: inside an unprivileged container nobody
//! can lower `oom_score_adj`, the host can — each gets a shield just deep enough to weigh less than nothing. Nothing is
//! installed inside the container.
use crate::service::Service;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{Read, Seek, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use utoipa::ToSchema;

pub const KV_CONFIG: &str = "system.pressure";
pub const KV_LAST: &str = "system.pressure.last";
/// Poll period of the watch
pub const TICK: Duration = Duration::from_secs(5);
/// A calm workspace gets a full process scan (protection, the largest process) every Nth tick; a tight one — every tick
const SCAN_EVERY: u32 = 3;
/// After ending a process: no new action for this long, then the state is judged again by `avg10`
const COOLDOWN: Duration = Duration::from_secs(60);
const MAX_KILLS_PER_HOUR: usize = 3;
/// A process smaller than this share of the limit is not worth ending
const MIN_SHARE: f64 = 0.10;
/// The watch ends processes only in a workspace that stands at its own limit — at least this share of it is used and
/// the limit-hit counter kept growing during the stall. Otherwise the stall comes from the host running out of memory,
/// and there the choice of a process is not this workspace's alone.
const AT_LIMIT_SHARE: f64 = 0.90;
/// A failed attempt (the process changed under us) is retried after this pause, not on every tick
const RETRY: Duration = Duration::from_secs(15);
/// The shield of an infrastructure process (`oom_score_adj`, set from the host) is just deep enough to put its weight for
/// the kernel OOM killer below zero — its own memory as a share of the limit, in thousandths — plus this margin. A deep
/// fixed one (-900) would be inherited by everything the process starts until the next pass: a conversation's runaway
/// script would then outweigh nothing, and the kernel would end the older processes of the workspace one by one first.
const SHIELD_MARGIN: i32 = 50;
const SHIELD_MAX: i32 = 900;
/// A shield this much deeper than needed is raised back (the process shrank, or something wrote a deeper value)
const SHIELD_SLACK: i32 = 100;
/// A process this large gets its conversation looked up at scan time (so a later kernel OOM can still name it)
const SESSION_SHARE: f64 = 0.05;
/// How many of the largest processes get their full command line read
const CMDLINE_TOP: usize = 8;
/// How long the pass waits for one read of a command line or an environment
const SLOW_READ_WAIT: Duration = Duration::from_millis(150);
/// A pass slower than this is worth a line in the log
const SLOW_PASS: Duration = Duration::from_secs(2);
/// At most one "ended by the system" report per workspace in this period (a crash loop must not flood the feed)
const OOM_NOTICE_GAP: Duration = Duration::from_secs(600);
/// A sample older than this means the workspace is not running any more
const STALE: Duration = Duration::from_secs(30);

fn yes() -> bool { true }
fn d_action() -> String { "kill".into() }
fn d_full() -> f64 { 40.0 }
fn d_hold() -> u64 { 120 }
fn d_tight() -> f64 { 10.0 }
fn d_long() -> f64 { 2.0 }

/// Settings of the watch (kv `system.pressure`, `GET/PUT /api/system/pressure`)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct PressureConfig {
    /// Off: states are still shown, but nothing is ended, reported or protected
    #[serde(default = "yes")]
    pub enabled: bool,
    /// `kill` — end the largest process of a stalled workspace; `notify` — only report
    #[serde(default = "d_action")]
    pub action: String,
    /// Stall threshold: PSI `memory full avg60`, percent
    #[serde(default = "d_full")]
    pub full_pct: f64,
    /// How long the threshold must hold before the workspace counts as stalled
    #[serde(default = "d_hold")]
    pub hold_secs: u64,
    /// "Tight" label threshold, percent
    #[serde(default = "d_tight")]
    pub tight_pct: f64,
    /// A turn silent for this many hours is reported by the "Agents" service; 0 — never
    #[serde(default = "d_long")]
    pub long_turn_hours: f64,
    /// Per-project overrides: "do not end processes here" (`action: notify`) or other thresholds
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectOverride>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct ProjectOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_turn_hours: Option<f64>,
}

impl Default for PressureConfig {
    fn default() -> Self {
        Self { enabled: true, action: d_action(), full_pct: d_full(), hold_secs: d_hold(), tight_pct: d_tight(), long_turn_hours: d_long(), projects: BTreeMap::new() }
    }
}

/// What applies to one project after its override
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    pub enabled: bool,
    pub kill: bool,
    pub full_pct: f64,
    pub hold: Duration,
    pub tight_pct: f64,
}

fn check_action(a: &str) -> Result<()> {
    if a == "kill" || a == "notify" { Ok(()) } else { bail!("недопустимое значение action: {a} (kill или notify)") }
}

impl PressureConfig {
    /// Stand-wide values stay in a safe range; a project override may go lower (a test project with tiny thresholds)
    pub fn validate(&self) -> Result<()> {
        check_action(&self.action)?;
        if !(20.0..=100.0).contains(&self.full_pct) { bail!("недопустимое значение full_pct: {} (от 20 до 100)", self.full_pct) }
        if !(30..=3600).contains(&self.hold_secs) { bail!("недопустимое значение hold_secs: {} (от 30 до 3600)", self.hold_secs) }
        if !(1.0..=self.full_pct).contains(&self.tight_pct) { bail!("недопустимое значение tight_pct: {} (от 1 до full_pct)", self.tight_pct) }
        if !(self.long_turn_hours == 0.0 || (0.25..=168.0).contains(&self.long_turn_hours)) { bail!("недопустимое значение long_turn_hours: {} (0 — выключено, иначе от 0.25 до 168)", self.long_turn_hours) }
        for (name, o) in &self.projects {
            if name.is_empty() || name.len() > 64 { bail!("недопустимое значение: имя проекта в projects") }
            if let Some(a) = &o.action { check_action(a)?; }
            if let Some(v) = o.full_pct { if !(1.0..=100.0).contains(&v) { bail!("недопустимое значение full_pct у {name}: {v} (от 1 до 100)") } }
            if let Some(v) = o.hold_secs { if !(5..=3600).contains(&v) { bail!("недопустимое значение hold_secs у {name}: {v} (от 5 до 3600)") } }
            if let Some(v) = o.long_turn_hours { if !(v == 0.0 || (0.001..=168.0).contains(&v)) { bail!("недопустимое значение long_turn_hours у {name}: {v}") } }
        }
        Ok(())
    }

    pub fn effective(&self, project: &str) -> Effective {
        let o = self.projects.get(project);
        let action = o.and_then(|o| o.action.as_deref()).unwrap_or(&self.action);
        let full_pct = o.and_then(|o| o.full_pct).unwrap_or(self.full_pct);
        Effective {
            enabled: self.enabled,
            kill: action == "kill",
            full_pct,
            hold: Duration::from_secs(o.and_then(|o| o.hold_secs).unwrap_or(self.hold_secs)),
            tight_pct: self.tight_pct.min(full_pct),
        }
    }

    /// `PUT` body merged into the current settings: top-level keys replace, `projects` entries are upserted one by one,
    /// `null` removes an override. The result is validated.
    pub fn merged(&self, patch: &serde_json::Value) -> Result<Self> {
        let Some(patch) = patch.as_object() else { bail!("недопустимое значение: настройки — объект") };
        let mut doc = serde_json::to_value(self)?;
        let cur = doc.as_object_mut().ok_or_else(|| anyhow!("настройки не объект"))?;
        for (k, v) in patch {
            if k == "projects" {
                let Some(pp) = v.as_object() else { bail!("недопустимое значение: projects — объект") };
                let cur_p = cur.entry("projects").or_insert_with(|| serde_json::json!({}));
                let Some(cur_p) = cur_p.as_object_mut() else { bail!("projects не объект") };
                for (name, o) in pp {
                    if o.is_null() { cur_p.remove(name); } else { cur_p.insert(name.clone(), o.clone()); }
                }
            } else if matches!(k.as_str(), "enabled" | "action" | "full_pct" | "hold_secs" | "tight_pct" | "long_turn_hours") {
                cur.insert(k.clone(), v.clone());
            } else {
                bail!("недопустимое значение: неизвестное поле {k}");
            }
        }
        let next: Self = serde_json::from_value(doc).map_err(|e| anyhow!("недопустимое значение: {e}"))?;
        // an override that says nothing is no override
        let next = Self { projects: next.projects.into_iter().filter(|(_, o)| *o != ProjectOverride::default()).collect(), ..next };
        next.validate()?;
        Ok(next)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// cgroup files
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sample {
    pub used: u64,
    /// 0 — no limit (`max`)
    pub limit: u64,
    pub anon: u64,
    pub file: u64,
    pub full10: f64,
    pub full60: f64,
    /// Processes of the subtree ended by the kernel OOM killer (hierarchical counter)
    pub oom_kill: u64,
    /// Times this container's own limit ran into OOM (`memory.events.local`)
    pub oom_local: u64,
    /// Times the usage ran into this container's own limit (`memory.events.local`: `max`) — grows all the time while the
    /// container stalls at its limit and stands still when the stall comes from the host
    pub max_local: u64,
}

/// `memory.pressure`: `full avg10` and `full avg60`, percent
pub fn parse_pressure(text: &str) -> (f64, f64) {
    let mut out = (0.0, 0.0);
    for line in text.lines() {
        let mut it = line.split_whitespace();
        if it.next() != Some("full") { continue }
        for kv in it {
            match kv.split_once('=') {
                Some(("avg10", v)) => out.0 = v.parse().unwrap_or(0.0),
                Some(("avg60", v)) => out.1 = v.parse().unwrap_or(0.0),
                _ => {}
            }
        }
    }
    out
}

/// A `key value` line of a flat cgroup file (`memory.stat`, `memory.events`)
pub fn flat_value(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|l| { let mut it = l.split_whitespace(); (it.next() == Some(key)).then(|| it.next().and_then(|v| v.parse().ok())).flatten() })
}

pub fn read_sample(dir: &Path) -> Result<Sample> {
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).with_context(|| format!("{name}"));
    let used = read("memory.current")?.trim().parse::<u64>().context("memory.current")?;
    let limit = read("memory.max")?.trim().parse::<u64>().unwrap_or(0);
    let stat = read("memory.stat")?;
    let (full10, full60) = parse_pressure(&read("memory.pressure")?);
    let events = read("memory.events").unwrap_or_default();
    let local = read("memory.events.local").unwrap_or_default();
    Ok(Sample {
        used,
        limit,
        anon: flat_value(&stat, "anon").unwrap_or(0),
        file: flat_value(&stat, "file").unwrap_or(0),
        full10,
        full60,
        oom_kill: flat_value(&events, "oom_kill").unwrap_or(0),
        oom_local: flat_value(&local, "oom").unwrap_or(0),
        // an old kernel without the local file: the subtree counter is the best there is
        max_local: flat_value(&local, "max").or_else(|| flat_value(&events, "max")).unwrap_or(0),
    })
}

/// Host pids of the whole subtree of a container cgroup
pub fn cgroup_pids(dir: &Path) -> Vec<i32> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<i32>) {
        if let Ok(text) = std::fs::read_to_string(dir.join("cgroup.procs")) {
            out.extend(text.lines().filter_map(|l| l.trim().parse::<i32>().ok()));
        }
        if depth >= 16 { return }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { walk(&e.path(), depth + 1, out); }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out.sort_unstable();
    out.dedup();
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// processes
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    /// Host pid
    pub pid: i32,
    /// How many pid namespaces deep the process lives: 1 — the host itself, 2 — a workspace, 3 — a container inside it
    pub depth: usize,
    /// Pid as the workspace sees it
    pub ws_pid: i32,
    pub comm: String,
    pub cmd: String,
    /// Anonymous resident memory, bytes — what ending the process gives back
    pub anon: u64,
    /// All resident memory, bytes — what the kernel OOM killer weighs
    pub rss: u64,
    pub adj: i32,
    /// Start time in clock ticks since boot: together with the pid it names one process
    pub start: u64,
    pub protected: bool,
    pub session: Option<String>,
}

/// `/proc/<pid>/stat` → (comm, start time). The comm may hold spaces and brackets, so the fields are counted from the last `)`.
pub fn parse_stat(text: &str) -> Option<(String, u64)> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let comm = text.get(open + 1..close)?.to_string();
    let rest: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    // after the comm: state is field 3, start time is field 22
    let start = rest.get(19)?.parse().ok()?;
    Some((comm, start))
}

/// `/proc/<pid>/status` → (RssAnon in bytes, VmRSS in bytes, NSpid chain)
pub fn parse_status(text: &str) -> (u64, u64, Vec<i32>) {
    let kb = |v: &str| v.split_whitespace().next().and_then(|x| x.parse::<u64>().ok()).unwrap_or(0) * 1024;
    let (mut anon, mut rss) = (0u64, 0u64);
    let mut ns = Vec::new();
    for l in text.lines() {
        if let Some(v) = l.strip_prefix("RssAnon:") { anon = kb(v); }
        if let Some(v) = l.strip_prefix("VmRSS:") { rss = kb(v); }
        if let Some(v) = l.strip_prefix("NSpid:") { ns = v.split_whitespace().filter_map(|x| x.parse().ok()).collect(); }
    }
    (anon, rss.max(anon), ns)
}

/// The workspace infrastructure: never ended by the watch, shielded from the kernel OOM killer. The comm is at most 15
/// characters (`containerd-shim-runc-v2` reads `containerd-shim`). Only the workspace's own init counts as init — the
/// main process of a container nested in the workspace is an ordinary candidate, as it is for the kernel.
pub fn is_protected(comm: &str, nspid: &[i32]) -> bool {
    if nspid.len() == 2 && nspid[1] == 1 { return true }
    matches!(comm, "cloudos-agentd" | "coder" | "dockerd" | "containerd") || comm.starts_with("containerd-shim") || comm.starts_with("systemd")
}

fn cmdline_text(raw: &[u8], comm: &str) -> String {
    let s: String = String::from_utf8_lossy(raw).split('\0').filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
    let s = s.trim();
    if s.is_empty() { format!("[{comm}]") } else { s.chars().take(200).collect() }
}

fn session_of(environ: &[u8]) -> Option<String> {
    environ.split(|b| *b == 0).find_map(|kv| kv.strip_prefix(b"CLOUDOS_SESSION=")).map(|v| String::from_utf8_lossy(v).to_string()).filter(|s| !s.is_empty())
}

pub fn read_proc(proc_root: &Path, pid: i32) -> Option<Proc> {
    let dir = proc_root.join(pid.to_string());
    let (comm, start) = parse_stat(&std::fs::read_to_string(dir.join("stat")).ok()?)?;
    let (anon, rss, ns) = parse_status(&std::fs::read_to_string(dir.join("status")).ok()?);
    let adj = std::fs::read_to_string(dir.join("oom_score_adj")).ok()?.trim().parse().ok()?;
    let cmd = format!("[{comm}]");
    Some(Proc { pid, depth: ns.len().max(1), ws_pid: ns.get(1).copied().unwrap_or(pid), protected: is_protected(&comm, &ns), comm, cmd, anon, rss, adj, start, session: None })
}

/// All processes of a container cgroup, largest first. Only files that never wait for the process's memory are read here
/// (`stat`, `status`, `oom_score_adj`) — the command line and the environment come from `describe_procs`.
pub fn scan(paths: &Paths, dir: &Path) -> Vec<Proc> {
    let mut out: Vec<Proc> = cgroup_pids(dir).into_iter().filter_map(|pid| read_proc(&paths.proc_root, pid)).collect();
    out.sort_by(|a, b| b.anon.cmp(&a.anon));
    out
}

/// What a process is, kept by (pid, start time): the full command line and the conversation it was started from
#[derive(Debug, Clone, Default)]
pub struct ProcMeta {
    pub cmd: String,
    pub session: Option<String>,
    /// The environment has been read (a small process is asked later, once it has grown)
    pub asked_session: bool,
}

/// Reads of `/proc/<pid>/cmdline` and `environ` running right now: both take the memory map lock of the process, and a
/// process stalled at its memory limit holds that lock for seconds — exactly when the watch must stay quick
static SLOW_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Read a file that may hang, in a helper thread, waiting no longer than `wait`. A read that did not come back is left
/// behind (the thread ends when the kernel lets it) — at most 16 of them at once.
fn read_guarded(path: PathBuf, wait: Duration) -> Option<Vec<u8>> {
    use std::sync::atomic::Ordering::Relaxed;
    if SLOW_READS.load(Relaxed) >= 16 { return None }
    SLOW_READS.fetch_add(1, Relaxed);
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("proc-read".into()).stack_size(128 * 1024).spawn(move || {
        let r = std::fs::read(&path).ok();
        SLOW_READS.fetch_sub(1, Relaxed);
        let _ = tx.send(r);
    });
    if spawned.is_err() { SLOW_READS.fetch_sub(1, Relaxed); return None }
    rx.recv_timeout(wait).ok().flatten()
}

/// Name the largest processes: from the cache, or by a guarded read that gives up at the first one that hangs (the
/// workspace is stalled — the rest would hang too; next pass tries again). A process named while the workspace was still
/// calm keeps its name through the stall; one that could not be read shows its comm.
pub fn describe_procs(paths: &Paths, procs: &mut [Proc], meta: &mut HashMap<(i32, u64), ProcMeta>, limit: u64) {
    meta.retain(|k, _| procs.iter().any(|p| (p.pid, p.start) == *k));
    let big = (((limit as f64) * SESSION_SHARE) as u64).max(1);
    let mut stuck = false;
    for p in procs.iter_mut().take(CMDLINE_TOP) {
        let key = (p.pid, p.start);
        let dir = paths.proc_root.join(p.pid.to_string());
        if !meta.contains_key(&key) && !stuck {
            match read_guarded(dir.join("cmdline"), SLOW_READ_WAIT) {
                Some(raw) => { meta.insert(key, ProcMeta { cmd: cmdline_text(&raw, &p.comm), ..Default::default() }); }
                None => stuck = true,
            }
        }
        let want_session = !p.protected && p.anon >= big;
        if let Some(m) = meta.get_mut(&key) {
            if want_session && !m.asked_session && !stuck {
                match read_guarded(dir.join("environ"), SLOW_READ_WAIT) {
                    Some(env) => { m.session = session_of(&env); m.asked_session = true; }
                    None => stuck = true,
                }
            }
            p.cmd = m.cmd.clone();
            p.session = m.session.clone();
        }
    }
}

fn score(p: &Proc, limit: u64) -> f64 {
    // a negative value on an ordinary process is only our own protection inherited from its parent
    p.anon as f64 + p.adj.max(0) as f64 / 1000.0 * limit as f64
}

/// Whom the watch may touch at all: an ordinary process living inside a container pid namespace
fn eligible(p: &Proc, min_depth: usize) -> bool { !p.protected && p.depth >= min_depth && p.pid > 1 }

/// The process to end: the kernel's choice (memory plus `oom_score_adj` as a share of the limit) among the ordinary
/// processes that hold at least a tenth of the limit
pub fn choose(procs: &[Proc], limit: u64, min_depth: usize) -> Option<&Proc> {
    let min = (((limit as f64) * MIN_SHARE) as u64).max(1);
    procs.iter().filter(|p| eligible(p, min_depth) && p.anon >= min).max_by(|a, b| score(a, limit).partial_cmp(&score(b, limit)).unwrap_or(std::cmp::Ordering::Equal))
}

/// The largest ordinary process, whatever its size — for the label hint and the "only report" card
pub fn largest(procs: &[Proc], min_depth: usize) -> Option<&Proc> {
    procs.iter().filter(|p| eligible(p, min_depth) && p.anon > 0).max_by_key(|p| p.anon)
}

/// The shield an infrastructure process of this size needs: its memory as thousandths of the limit plus the margin
pub fn shield_adj(rss: u64, limit: u64) -> i32 {
    let share = if limit > 0 { (rss as f64 * 1000.0 / limit as f64).ceil() as i32 } else { 0 };
    -(share.saturating_add(SHIELD_MARGIN).clamp(SHIELD_MARGIN, SHIELD_MAX))
}

/// The value `oom_score_adj` should get, if it must change: the infrastructure gets its shield (deepened when the
/// process grew, raised back when it is far deeper than needed), an ordinary process that inherited a negative value goes
/// back to 0, positive values are the owner's own business
pub fn adj_fix(p: &Proc, limit: u64) -> Option<i32> {
    if !p.protected { return (p.adj < 0).then_some(0) }
    let want = shield_adj(p.rss, limit);
    (p.adj > want || p.adj < want - SHIELD_SLACK).then_some(want)
}

/// Where the watch reads the system from (temporary directories in tests)
#[derive(Clone)]
pub struct Paths {
    pub cgroup_root: PathBuf,
    pub proc_root: PathBuf,
    /// A process must live at least this deep in pid namespaces to be touched: 2 — inside a container, never the host
    pub min_depth: usize,
    /// Replaces the real signal in tests
    pub killer: Option<Arc<dyn Fn(&Proc) -> Result<()> + Send + Sync>>,
}

impl Paths {
    pub fn system() -> Self { Self { cgroup_root: "/sys/fs/cgroup".into(), proc_root: "/proc".into(), min_depth: 2, killer: None } }
}

/// One process pinned by its open `/proc/<pid>` directory: every read and write through it refers to the same process
/// even if the pid number is reused meanwhile (the files of an exited process answer ESRCH)
struct ProcDir(OwnedFd);

impl ProcDir {
    fn open(proc_root: &Path, pid: i32) -> std::io::Result<Self> {
        let path = std::ffi::CString::new(proc_root.join(pid.to_string()).to_string_lossy().as_bytes()).map_err(|_| std::io::Error::other("path"))?;
        // SAFETY: a plain open(2) of a NUL-terminated path; the returned descriptor is owned by OwnedFd
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if fd < 0 { return Err(std::io::Error::last_os_error()) }
        // SAFETY: fd is a fresh valid descriptor nobody else owns
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd) }))
    }
    fn file(&self, name: &str, flags: i32) -> std::io::Result<std::fs::File> {
        let c = std::ffi::CString::new(name).map_err(|_| std::io::Error::other("name"))?;
        // SAFETY: openat(2) relative to our own directory descriptor with a NUL-terminated name
        let fd = unsafe { libc::openat(self.0.as_raw_fd(), c.as_ptr(), flags | libc::O_CLOEXEC | libc::O_NOFOLLOW) };
        if fd < 0 { return Err(std::io::Error::last_os_error()) }
        // SAFETY: fd is a fresh valid descriptor nobody else owns
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }
    fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.file(name, libc::O_RDONLY)?.read_to_end(&mut buf)?;
        Ok(buf)
    }
    fn write(&self, name: &str, data: &str) -> std::io::Result<()> {
        self.file(name, libc::O_WRONLY | libc::O_TRUNC)?.write_all(data.as_bytes())
    }
}

/// `/lxc.payload.ws-admin-alpha/system.slice/x.service` lies within `/lxc.payload.ws-admin-alpha`; a sibling whose name
/// merely starts the same (`…-alpha2`) does not
pub fn cgroup_path_within(path: &str, cgroup_rel: &str) -> bool {
    path == cgroup_rel || path.strip_prefix(cgroup_rel).map(|r| r.starts_with('/')).unwrap_or(false)
}

/// `/proc/<pid>/cgroup` (`0::<path>`) names a cgroup within ours
pub fn cgroup_within(text: &str, cgroup_rel: &str) -> bool {
    text.lines().filter_map(|l| l.strip_prefix("0::")).any(|p| cgroup_path_within(p.trim(), cgroup_rel))
}

/// Is the pinned process still the one we scanned — the same start time, still inside our container cgroup and inside a
/// container pid namespace?
fn verify(d: &ProcDir, p: &Proc, cgroup_rel: &str, min_depth: usize) -> Result<()> {
    let stat = String::from_utf8_lossy(&d.read("stat")?).to_string();
    let (comm, start) = parse_stat(&stat).ok_or_else(|| anyhow!("stat не разобран"))?;
    if start != p.start { bail!("pid {} уже другой процесс", p.pid) }
    let (_, _, ns) = parse_status(&String::from_utf8_lossy(&d.read("status")?));
    if ns.len().max(1) < min_depth { bail!("pid {} не в пространстве контейнера", p.pid) }
    if is_protected(&comm, &ns) != p.protected { bail!("pid {} сменил роль", p.pid) }
    if !cgroup_within(&String::from_utf8_lossy(&d.read("cgroup")?), cgroup_rel) { bail!("pid {} не в {cgroup_rel}", p.pid) }
    Ok(())
}

/// End one process without a pid race: `pidfd_open` pins whatever process holds the number now, then the same number is
/// checked again through `/proc` (start time, cgroup, namespace depth, not infrastructure) and only then the signal goes
/// to the pinned process. If the number was reused in between, either the check fails or the pinned process is already
/// gone and the signal answers ESRCH.
pub fn kill_checked(paths: &Paths, cgroup_rel: &str, p: &Proc) -> Result<()> {
    if p.protected || p.pid <= 1 || p.pid == std::process::id() as i32 { bail!("pid {} не завершается", p.pid) }
    // SAFETY: pidfd_open(2) takes a pid and flags and returns a new descriptor or -1
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, p.pid as libc::pid_t, 0 as libc::c_uint) };
    if raw < 0 { return Err(std::io::Error::last_os_error()).with_context(|| format!("pidfd_open {}", p.pid)) }
    // SAFETY: raw is a fresh valid descriptor nobody else owns
    let pidfd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
    let d = ProcDir::open(&paths.proc_root, p.pid).with_context(|| format!("/proc/{}", p.pid))?;
    verify(&d, p, cgroup_rel, paths.min_depth)?;
    // SAFETY: pidfd_send_signal(2) on our own pidfd, no siginfo, no flags
    let r = unsafe { libc::syscall(libc::SYS_pidfd_send_signal, pidfd.as_raw_fd(), libc::SIGKILL, std::ptr::null::<libc::siginfo_t>(), 0 as libc::c_uint) };
    if r < 0 { return Err(std::io::Error::last_os_error()).with_context(|| format!("pidfd_send_signal {}", p.pid)) }
    Ok(())
}

/// Set `oom_score_adj` of one process through its pinned directory after the same checks
pub fn set_adj(paths: &Paths, cgroup_rel: &str, p: &Proc, value: i32) -> Result<()> {
    let d = ProcDir::open(&paths.proc_root, p.pid)?;
    verify(&d, p, cgroup_rel, paths.min_depth)?;
    d.write("oom_score_adj", &value.to_string())?;
    Ok(())
}

/// Protection pass over a scanned workspace; returns how many values were changed
pub fn protect(paths: &Paths, cgroup_rel: &str, procs: &mut [Proc], limit: u64) -> usize {
    let mut n = 0;
    for p in procs.iter_mut() {
        if p.depth < paths.min_depth { continue }
        let Some(v) = adj_fix(p, limit) else { continue };
        match set_adj(paths, cgroup_rel, p, v) {
            Ok(()) => { p.adj = v; n += 1; }
            Err(e) => tracing::debug!("oom_score_adj {} ({}): {e:#}", p.pid, p.comm),
        }
    }
    n
}

// ---------------------------------------------------------------------------------------------------------------------
// kernel log: whom the kernel OOM killer ended
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct OomKill {
    pub pid: i32,
    pub comm: String,
    /// The cgroup whose limit was hit
    pub oom_memcg: String,
    pub task_memcg: String,
    pub anon: Option<u64>,
}

/// Kernel OOM reports among log lines: the `oom-kill:` line names the cgroups and the task, the `Killed process` line
/// adds the size
pub fn parse_oom(lines: &[String]) -> Vec<OomKill> {
    let mut out: Vec<OomKill> = Vec::new();
    for l in lines {
        if let Some(i) = l.find("oom-kill:") {
            let body = &l[i..];
            let field = |key: &str, end: &str| -> Option<String> {
                let s = body.find(key)? + key.len();
                let rest = &body[s..];
                Some(rest[..rest.find(end).unwrap_or(rest.len())].to_string())
            };
            let (Some(task), Some(pid)) = (field(",task=", ",pid="), field(",pid=", ",").and_then(|p| p.trim().parse::<i32>().ok())) else { continue };
            out.push(OomKill { pid, comm: task, oom_memcg: field(",oom_memcg=", ",").unwrap_or_default(), task_memcg: field(",task_memcg=", ",").unwrap_or_default(), anon: None });
        } else if let Some(i) = l.find("Killed process ") {
            let rest = &l[i + "Killed process ".len()..];
            let Some(pid) = rest.split_whitespace().next().and_then(|p| p.parse::<i32>().ok()) else { continue };
            let anon = rest.find("anon-rss:").and_then(|j| rest[j + 9..].split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<u64>().ok())).map(|kb| kb * 1024);
            if let Some(k) = out.iter_mut().rev().find(|k| k.pid == pid) { k.anon = anon; }
        }
    }
    out
}

/// Tail of `/dev/kmsg`: opened once at the end of the buffer, every drain returns the messages that arrived since
pub struct Kmsg { file: std::fs::File }

impl Kmsg {
    pub fn open() -> Option<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open("/dev/kmsg").ok()?;
        file.seek(std::io::SeekFrom::End(0)).ok()?;
        Some(Self { file })
    }
    /// One read returns one record `prio,seq,time,flags;message\n` plus indented dictionary lines
    pub fn drain(&mut self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        for _ in 0..20_000 {
            match self.file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let rec = String::from_utf8_lossy(&buf[..n]);
                    let Some((head, body)) = rec.split_once(';') else { continue };
                    let msg = body.lines().next().unwrap_or("").to_string();
                    // a continuation fragment (flag `+`) belongs to the previous line
                    let cont = head.split(',').nth(3).map(|f| f.contains('+')).unwrap_or(false);
                    match (cont, out.last_mut()) { (true, Some(prev)) => prev.push_str(&msg), _ => out.push(msg) }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                // EPIPE: the oldest records were overwritten while we read — go on from what is left
                Err(e) if e.raw_os_error() == Some(libc::EPIPE) => continue,
                Err(_) => break,
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// state machine
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level { Ok, Tight, Stalled }

impl Level {
    pub fn as_str(&self) -> &'static str { match self { Level::Ok => "ok", Level::Tight => "tight", Level::Stalled => "stalled" } }
}

#[derive(Debug, Default)]
pub struct Track {
    /// Since when `full avg60` has been at or over the threshold without a break
    pub over_since: Option<Instant>,
    /// The limit-hit counter at that moment
    pub max_at_over: Option<u64>,
    pub cooldown_until: Option<Instant>,
    /// Processes ended by the watch during the last hour
    pub kills: VecDeque<Instant>,
    /// This stall has already been reported without ending anything
    pub told: bool,
    pub oom_kill: Option<u64>,
    pub oom_local: Option<u64>,
    pub last_oom_notice: Option<Instant>,
    /// The OOM counters grew, but the kernel log line had not arrived yet — look again on the next tick (the value: did
    /// this container's own limit counter grow)
    pub oom_pending: Option<bool>,
}

/// One sample through the automaton: the level and whether it is time to act. `stalled` needs `avg60` over the threshold
/// for the whole hold time *and* `avg10` over it right now — `avg60` remembers a stall for a minute after it is over.
pub fn step(t: &mut Track, s: &Sample, e: &Effective, now: Instant) -> (Level, bool) {
    if s.full60 < e.full_pct {
        t.over_since = None;
        t.max_at_over = None;
        t.told = false;
        return (if s.full60 >= e.tight_pct { Level::Tight } else { Level::Ok }, false);
    }
    let since = *t.over_since.get_or_insert(now);
    t.max_at_over.get_or_insert(s.max_local);
    if now.duration_since(since) < e.hold || s.full10 < e.full_pct { return (Level::Tight, false) }
    let cooling = t.cooldown_until.map(|c| now < c).unwrap_or(false);
    (Level::Stalled, !cooling)
}

// ---------------------------------------------------------------------------------------------------------------------
// what the owner sees
// ---------------------------------------------------------------------------------------------------------------------

/// The last thing the watch did or reported for a workspace
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct MemoryAction {
    /// Epoch seconds
    pub at: u64,
    /// `killed` — the watch ended a process; `stalled` — the workspace stands at the limit, nothing was ended; `oom` —
    /// the kernel ended a process
    pub kind: String,
    #[serde(default)]
    pub cmd: String,
    /// Anonymous memory of the process, bytes
    #[serde(default)]
    pub rss: u64,
    #[serde(default)]
    pub age_secs: u64,
    /// The conversation the process was started from
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub stalled_secs: u64,
    #[serde(default)]
    pub full60: f64,
    #[serde(default)]
    pub limit: u64,
    /// Why nothing was ended: `notify` (the mode), `small` (no process holds a tenth of the limit), `rate` (three per
    /// hour already), `host` (the workspace is not at its own limit — the host is short of memory); `manual` — ended by
    /// the owner's button
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct MemoryProc {
    pub cmd: String,
    pub rss: u64,
    pub age_secs: u64,
    #[serde(default)]
    pub session: Option<String>,
}

/// `ProjectView.workspace.memory`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct MemoryView {
    pub used: u64,
    pub limit: u64,
    pub anon: u64,
    pub full10: f64,
    pub full60: f64,
    /// `ok | tight | stalled`
    pub state: String,
    /// What the watch does with this workspace when it stalls: `kill | notify | off`
    pub action: String,
    /// The largest ordinary process — present while the workspace is not `ok`
    #[serde(default)]
    pub top: Option<MemoryProc>,
    #[serde(default)]
    pub last: Option<MemoryAction>,
}

/// A row of `GET /api/projects/{name}/memory`
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemoryProcRow {
    /// Pid inside the workspace
    pub pid: i32,
    pub comm: String,
    pub cmd: String,
    pub rss: u64,
    pub adj: i32,
    pub protected: bool,
    /// Lives in a container nested in the workspace
    pub nested: bool,
}

struct Ws {
    track: Track,
    sample: Sample,
    level: Level,
    at: Instant,
    procs: Vec<Proc>,
    meta: HashMap<(i32, u64), ProcMeta>,
    action: &'static str,
}

#[derive(Default)]
pub struct PressureState {
    /// Working state of the pass — locked for the whole pass (file reads of every workspace)
    ws: Mutex<HashMap<String, Ws>>,
    /// What the API shows: refreshed at the end of every pass, so a project card never waits for the pass
    views: Mutex<HashMap<String, (Instant, MemoryView, Vec<MemoryProcRow>)>>,
    last: Mutex<HashMap<String, MemoryAction>>,
}

pub struct Notice {
    pub project: String,
    pub action: MemoryAction,
}

fn epoch() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

fn uptime_secs(proc_root: &Path) -> f64 {
    std::fs::read_to_string(proc_root.join("uptime")).ok().and_then(|t| t.split_whitespace().next().and_then(|v| v.parse().ok())).unwrap_or(0.0)
}

fn clk_tck() -> f64 {
    // SAFETY: sysconf(3) with a constant name
    let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if v > 0 { v as f64 } else { 100.0 }
}

fn age_of(p: &Proc, uptime: f64) -> u64 { (uptime - p.start as f64 / clk_tck()).max(0.0) as u64 }

fn brief(p: &Proc, uptime: f64) -> MemoryProc { MemoryProc { cmd: p.cmd.clone(), rss: p.anon, age_secs: age_of(p, uptime), session: p.session.clone() } }

impl PressureState {
    pub fn load(store: &crate::store::Store) -> Self {
        let last = store.get_kv(KV_LAST).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        Self { ws: Mutex::new(HashMap::new()), views: Mutex::new(HashMap::new()), last: Mutex::new(last) }
    }

    pub fn last_map(&self) -> HashMap<String, MemoryAction> { self.last.lock().unwrap().clone() }

    /// The view for a project card; None — the workspace is not running (no fresh sample)
    pub fn view(&self, project: &str) -> Option<MemoryView> {
        let (at, mut v) = self.views.lock().unwrap().get(project).map(|(at, v, _)| (*at, v.clone()))?;
        if at.elapsed() > STALE { return None }
        v.last = self.last.lock().unwrap().get(project).cloned();
        Some(v)
    }

    /// State of every running workspace in one cheap answer — what the "Agents" service polls to notice a change at once
    pub fn states(&self) -> BTreeMap<String, serde_json::Value> {
        let last = self.last.lock().unwrap();
        self.views.lock().unwrap().iter().filter(|(_, (at, _, _))| at.elapsed() <= STALE)
            .map(|(name, (_, v, _))| (name.clone(), serde_json::json!({ "state": v.state, "action": v.action, "last_at": last.get(name).map(|l| l.at) }))).collect()
    }

    pub fn rows(&self, project: &str) -> Vec<MemoryProcRow> {
        self.views.lock().unwrap().get(project).filter(|(at, _, _)| at.elapsed() <= STALE).map(|(_, _, rows)| rows.clone()).unwrap_or_default()
    }

    fn set_last(&self, project: &str, a: &MemoryAction) { self.last.lock().unwrap().insert(project.to_string(), a.clone()); }

    /// One pass over the running workspaces: `(project, instance)` pairs. Blocking file I/O — call from a blocking
    /// thread. Returns what must be reported to the owner.
    pub fn pass(&self, paths: &Paths, cfg: &PressureConfig, workspaces: &[(String, String)], tick: u32, kmsg_lines: Option<&[String]>, now: Instant) -> Vec<Notice> {
        let mut notices = Vec::new();
        let uptime = uptime_secs(&paths.proc_root);
        let ooms = parse_oom(kmsg_lines.unwrap_or(&[]));
        let mut g = self.ws.lock().unwrap();
        g.retain(|name, _| workspaces.iter().any(|(p, _)| p == name));
        for (project, instance) in workspaces {
            let rel = format!("/lxc.payload.{instance}");
            let dir = paths.cgroup_root.join(format!("lxc.payload.{instance}"));
            let Ok(sample) = read_sample(&dir) else { g.remove(project); continue };
            let eff = cfg.effective(project);
            let w = g.entry(project.clone()).or_insert_with(|| Ws { track: Track::default(), sample: Sample::default(), level: Level::Ok, at: now, procs: Vec::new(), meta: HashMap::new(), action: "off" });
            let (level, act) = step(&mut w.track, &sample, &eff, now);
            // base for shares: the limit, or what is used when there is none
            let limit = if sample.limit > 0 { sample.limit } else { sample.used };

            // the kernel ended somebody: only a run into this container's own limit is the owner's news — a container
            // nested in the workspace hitting its own smaller limit is its own business
            let prev = (w.track.oom_kill.replace(sample.oom_kill), w.track.oom_local.replace(sample.oom_local));
            let grew = |prev: Option<u64>, cur: u64| prev.map(|p| cur > p).unwrap_or(false);
            let (grew_kill, grew_local) = (grew(prev.0, sample.oom_kill), grew(prev.1, sample.oom_local));
            let deferred = w.track.oom_pending.take();
            if eff.enabled && (grew_kill || grew_local || deferred.is_some()) {
                let local = grew_local || deferred.unwrap_or(false);
                let mine: Vec<&OomKill> = ooms.iter().filter(|k| cgroup_path_within(&k.task_memcg, &rel)).collect();
                let victim: Option<OomKill> = mine.iter().find(|k| k.oom_memcg == rel).map(|k| (*k).clone());
                if mine.is_empty() && deferred.is_none() && kmsg_lines.is_some() {
                    // the log line may land a moment after the counter
                    w.track.oom_pending = Some(local);
                } else {
                    let report = if mine.is_empty() { local } else { victim.is_some() };
                    let quiet = w.track.last_oom_notice.map(|t| now.duration_since(t) < OOM_NOTICE_GAP).unwrap_or(false);
                    if report && !quiet {
                        w.track.last_oom_notice = Some(now);
                        let known = victim.as_ref().and_then(|k| w.procs.iter().find(|p| p.pid == k.pid));
                        let a = MemoryAction {
                            at: epoch(),
                            kind: "oom".into(),
                            cmd: known.map(|p| p.cmd.clone()).or_else(|| victim.as_ref().map(|k| k.comm.clone())).unwrap_or_default(),
                            rss: victim.as_ref().and_then(|k| k.anon).or_else(|| known.map(|p| p.anon)).unwrap_or(0),
                            age_secs: known.map(|p| age_of(p, uptime)).unwrap_or(0),
                            session: known.and_then(|p| p.session.clone()),
                            limit: sample.limit,
                            full60: sample.full60,
                            ..Default::default()
                        };
                        self.set_last(project, &a);
                        notices.push(Notice { project: project.clone(), action: a });
                    }
                }
            }

            if level != Level::Ok || act || tick % SCAN_EVERY == 0 || w.procs.is_empty() {
                w.procs = scan(paths, &dir);
                describe_procs(paths, &mut w.procs, &mut w.meta, limit);
                if eff.enabled { protect(paths, &rel, &mut w.procs, limit); }
            }

            if eff.enabled && act {
                let stalled_secs = w.track.over_since.map(|s| now.duration_since(s).as_secs()).unwrap_or(0);
                while w.track.kills.front().map(|t| now.duration_since(*t) > Duration::from_secs(3600)).unwrap_or(false) { w.track.kills.pop_front(); }
                let top = largest(&w.procs, paths.min_depth).cloned();
                let base = |kind: &str, p: Option<&Proc>, note: Option<&str>| MemoryAction {
                    at: epoch(),
                    kind: kind.into(),
                    cmd: p.map(|p| p.cmd.clone()).unwrap_or_default(),
                    rss: p.map(|p| p.anon).unwrap_or(0),
                    age_secs: p.map(|p| age_of(p, uptime)).unwrap_or(0),
                    session: p.and_then(|p| p.session.clone()),
                    stalled_secs,
                    full60: sample.full60,
                    limit: sample.limit,
                    note: note.map(str::to_string),
                };
                let victim = choose(&w.procs, limit, paths.min_depth).cloned();
                let at_limit = sample.limit > 0 && sample.used as f64 >= sample.limit as f64 * AT_LIMIT_SHARE && w.track.max_at_over.map(|b| sample.max_local > b).unwrap_or(false);
                let why_not = if !eff.kill { Some("notify") } else if !at_limit { Some("host") } else if w.track.kills.len() >= MAX_KILLS_PER_HOUR { Some("rate") } else if victim.is_none() { Some("small") } else { None };
                match (why_not, victim) {
                    (None, Some(v)) => {
                        let r = match &paths.killer { Some(k) => k(&v), None => kill_checked(paths, &rel, &v) };
                        match r {
                            Ok(()) => {
                                w.track.kills.push_back(now);
                                w.track.cooldown_until = Some(now + COOLDOWN);
                                w.track.told = false;
                                w.procs.retain(|p| p.pid != v.pid);
                                let a = base("killed", Some(&v), None);
                                self.set_last(project, &a);
                                notices.push(Notice { project: project.clone(), action: a });
                            }
                            // the process is gone or changed under us — look again after a short pause
                            Err(e) => {
                                w.track.cooldown_until = Some(now + RETRY);
                                tracing::warn!("память {project}: процесс {} не завершён: {e:#}", v.pid);
                            }
                        }
                    }
                    (why, _) => {
                        if !w.track.told {
                            w.track.told = true;
                            let a = base("stalled", top.as_ref(), why);
                            self.set_last(project, &a);
                            notices.push(Notice { project: project.clone(), action: a });
                        }
                    }
                }
            }
            w.sample = sample;
            w.level = level;
            w.at = now;
            w.action = if !eff.enabled { "off" } else if eff.kill { "kill" } else { "notify" };
        }
        let views: HashMap<String, (Instant, MemoryView, Vec<MemoryProcRow>)> = g.iter().map(|(name, w)| {
            let top = (w.level != Level::Ok).then(|| largest(&w.procs, paths.min_depth).map(|p| brief(p, uptime))).flatten();
            let rows = w.procs.iter().map(|p| MemoryProcRow { pid: p.ws_pid, comm: p.comm.clone(), cmd: p.cmd.clone(), rss: p.anon, adj: p.adj, protected: p.protected, nested: p.depth > 2 }).collect();
            (name.clone(), (w.at, MemoryView { used: w.sample.used, limit: w.sample.limit, anon: w.sample.anon, full10: w.sample.full10, full60: w.sample.full60, state: w.level.as_str().into(), action: w.action.into(), top, last: None }, rows))
        }).collect();
        drop(g);
        *self.views.lock().unwrap() = views;
        notices
    }

    /// The owner's "end the process" button: the largest ordinary process of a workspace that is not calm right now
    pub fn kill_now(&self, paths: &Paths, project: &str, instance: &str) -> Result<MemoryAction> {
        let rel = format!("/lxc.payload.{instance}");
        let dir = paths.cgroup_root.join(format!("lxc.payload.{instance}"));
        let sample = read_sample(&dir).map_err(|_| anyhow!("workspace {project} не запущен"))?;
        let calm = self.views.lock().unwrap().get(project).map(|(_, v, _)| v.state == "ok").unwrap_or(true);
        if calm { bail!("workspace {project} уже не у лимита памяти — завершать нечего") }
        let limit = if sample.limit > 0 { sample.limit } else { sample.used };
        let mut procs = scan(paths, &dir);
        {
            // names: what the pass already knows; the rest — by the same guarded read
            let mut g = self.ws.lock().unwrap();
            let mut own = HashMap::new();
            describe_procs(paths, &mut procs, g.get_mut(project).map(|w| &mut w.meta).unwrap_or(&mut own), limit);
        }
        let v = largest(&procs, paths.min_depth).cloned().ok_or_else(|| anyhow!("в workspace {project} нет процесса, который можно завершить"))?;
        match &paths.killer { Some(k) => k(&v)?, None => kill_checked(paths, &rel, &v)? }
        let a = MemoryAction { at: epoch(), kind: "killed".into(), cmd: v.cmd.clone(), rss: v.anon, age_secs: age_of(&v, uptime_secs(&paths.proc_root)), session: v.session.clone(), full60: sample.full60, limit: sample.limit, note: Some("manual".into()), ..Default::default() };
        if let Some(w) = self.ws.lock().unwrap().get_mut(project) { w.track.cooldown_until = Some(Instant::now() + COOLDOWN); w.procs.retain(|p| p.pid != v.pid); }
        self.set_last(project, &a);
        Ok(a)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// texts
// ---------------------------------------------------------------------------------------------------------------------

pub fn fmt_bytes(b: u64) -> String {
    let g = b as f64 / 1_073_741_824.0;
    if g >= 1.0 { format!("{g:.1} ГиБ") } else { format!("{:.0} МиБ", b as f64 / 1_048_576.0) }
}

pub fn fmt_dur(secs: u64) -> String {
    if secs >= 7200 { format!("{} ч", secs / 3600) } else if secs >= 120 { format!("{} мин", secs / 60) } else { format!("{secs} с") }
}

/// Title and body of the report (the feed event, the push)
pub fn describe(project: &str, a: &MemoryAction) -> (String, String) {
    let cmd: String = a.cmd.chars().take(120).collect();
    match a.kind.as_str() {
        "killed" => (
            "завершён процесс: workspace стоял у лимита памяти".into(),
            if a.note.as_deref() == Some("manual") { format!("{project}: {cmd} — {}, работал {}; завершён по кнопке владельца", fmt_bytes(a.rss), fmt_dur(a.age_secs)) }
            else { format!("{project}: {cmd} — {}, работал {}; workspace {} стоял у лимита {}, простой {:.0} %", fmt_bytes(a.rss), fmt_dur(a.age_secs), fmt_dur(a.stalled_secs), fmt_bytes(a.limit), a.full60) },
        ),
        "oom" => (
            "процесс завершён системой: не хватило памяти".into(),
            if cmd.is_empty() { format!("{project}: workspace упёрся в лимит {}", fmt_bytes(a.limit)) } else { format!("{project}: {cmd} — {}; лимит workspace {}", fmt_bytes(a.rss), fmt_bytes(a.limit)) },
        ),
        _ => {
            let why = match a.note.as_deref() {
                Some("small") => "; завершать нечего: ни один процесс не занимает десятой части лимита",
                Some("rate") => "; три процесса за час уже завершены — дальше только сообщения",
                Some("host") => "; workspace не у своего лимита — памяти не хватает самому хосту, процессы не завершались",
                _ => "",
            };
            let top = if cmd.is_empty() { String::new() } else { format!("; крупнейший процесс: {cmd} — {}", fmt_bytes(a.rss)) };
            ("workspace стоит у лимита памяти".into(), format!("{project}: простой {:.0} % уже {}, лимит {}{top}{why}", a.full60, fmt_dur(a.stalled_secs), fmt_bytes(a.limit)))
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the core
// ---------------------------------------------------------------------------------------------------------------------

impl Service {
    pub fn pressure_config(&self) -> PressureConfig {
        self.store.get_kv(KV_CONFIG).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }

    pub fn pressure_config_put(&self, patch: &serde_json::Value) -> Result<PressureConfig> {
        let cur = self.pressure_config();
        let next = cur.merged(patch)?;
        self.store.put_kv(KV_CONFIG, &serde_json::to_value(&next)?)?;
        let act = |a: &str| if a == "kill" { "завершать процесс" } else { "только сообщать" };
        let stand = |c: &PressureConfig| (c.enabled, c.action.clone(), c.full_pct, c.hold_secs, c.tight_pct, c.long_turn_hours);
        if stand(&cur) != stand(&next) {
            let _ = self.store.event("memory", "system", &format!("защита памяти: {}, при пробуксовке — {}, порог {:.0} % дольше {} с, долгая команда — от {} ч", if next.enabled { "включена" } else { "выключена" }, act(&next.action), next.full_pct, next.hold_secs, next.long_turn_hours));
        }
        for name in cur.projects.keys().chain(next.projects.keys()).collect::<std::collections::BTreeSet<_>>() {
            match (cur.projects.get(name), next.projects.get(name)) {
                (a, b) if a == b => {}
                (_, None) => { let _ = self.store.event("memory", name, "защита памяти: свои настройки проекта убраны — действуют общие"); }
                (_, Some(_)) => {
                    let e = next.effective(name);
                    let _ = self.store.event("memory", name, &format!("защита памяти: свои настройки проекта — при пробуксовке {}, порог {:.0} % дольше {} с", act(if e.kill { "kill" } else { "notify" }), e.full_pct, e.hold.as_secs()));
                }
            }
        }
        Ok(next)
    }

    pub fn memory_view(&self, project: &str) -> Option<MemoryView> { self.pressure.view(project) }

    /// Feed event, push when Desktop is away, a grey line in the conversation that started the process
    async fn pressure_report(&self, n: &Notice) {
        let (title, body) = describe(&n.project, &n.action);
        let _ = self.store.event("memory", &n.project, &format!("{title}: {}", body.trim_start_matches(&format!("{}: ", n.project))));
        if let Err(e) = self.store.put_kv(KV_LAST, &serde_json::json!(self.pressure.last_map())) { tracing::warn!("kv {KV_LAST}: {e:#}"); }
        if self.agents.desktop_away() {
            if let Err(e) = self.ntfy_push(&title, &body, "warning").await { tracing::debug!("ntfy: {e:#}"); }
        }
        if let (Some(id), true) = (n.action.session.clone(), n.action.kind != "stalled") {
            let me = self.clone();
            let project = n.project.clone();
            let msg = if n.action.kind == "killed" { format!("процесс завершён сторожем памяти: {} — {}; workspace стоял у лимита памяти", n.action.cmd.chars().take(120).collect::<String>(), fmt_bytes(n.action.rss)) } else { format!("процесс завершён системой — не хватило памяти: {}", n.action.cmd.chars().take(120).collect::<String>()) };
            // agentd of a workspace that has just been stalled may answer late — a few patient tries in the background
            tokio::spawn(async move {
                for wait in [2u64, 15, 45] {
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    let Ok(c) = me.agentd_client(&project).await else { continue };
                    if tokio::time::timeout(Duration::from_secs(10), c.post(&format!("/v1/sessions/{id}/note"), &serde_json::json!({ "message": msg }))).await.map(|r| r.is_ok()).unwrap_or(false) { return }
                }
            });
        }
    }

    pub fn start_pressure_watch(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let paths = Paths::system();
            let mut kmsg = Kmsg::open();
            if kmsg.is_none() { tracing::warn!("сторож памяти: /dev/kmsg недоступен — завершённые системой процессы будут без имени"); }
            let mut prefix = String::new();
            let mut prefix_at: Option<Instant> = None;
            let mut tick: u32 = 0;
            loop {
                // `ws-<owner>-` — the owner comes from Coder, asked once in ten minutes
                if prefix_at.map(|t| t.elapsed() > Duration::from_secs(600)).unwrap_or(true) {
                    prefix = me.workspace_instance("").await;
                    prefix_at = Some(Instant::now());
                }
                let cfg = me.pressure_config();
                let workspaces: Vec<(String, String)> = me.store.list_projects().unwrap_or_default().into_iter().map(|p| (p.name.clone(), format!("{prefix}{}", p.name))).collect();
                let lines: Option<Vec<String>> = kmsg.as_mut().map(|k| k.drain().into_iter().filter(|l| l.contains("oom-kill:") || l.contains("Killed process ")).collect());
                let (state, p2) = (me.pressure.clone(), paths.clone());
                let started = Instant::now();
                let notices = tokio::task::spawn_blocking(move || state.pass(&p2, &cfg, &workspaces, tick, lines.as_deref(), Instant::now())).await.unwrap_or_default();
                if started.elapsed() > SLOW_PASS { tracing::warn!("сторож памяти: проход занял {:.1} с", started.elapsed().as_secs_f64()); }
                for n in &notices { me.pressure_report(n).await; }
                tick = tick.wrapping_add(1);
                tokio::time::sleep(TICK).await;
            }
        });
    }

    /// The "end the process" button of the "only report" card
    pub async fn pressure_kill_now(&self, project: &str) -> Result<MemoryAction> {
        self.store.get_project(project)?.ok_or_else(|| anyhow!("проект {project} не найден"))?;
        let instance = self.workspace_instance(project).await;
        let (state, name) = (self.pressure.clone(), project.to_string());
        let a = tokio::task::spawn_blocking(move || state.kill_now(&Paths::system(), &name, &instance)).await??;
        self.pressure_report(&Notice { project: project.to_string(), action: a.clone() }).await;
        Ok(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const GIB: u64 = 1 << 30;
    const MIB_KB: u64 = 1024;

    /// A fake host: cgroup files of workspaces and `/proc` entries of their processes in a temporary directory
    struct Fake { dir: tempfile::TempDir }

    impl Fake {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("proc")).unwrap();
            fs::write(dir.path().join("proc/uptime"), "100000.00 5000.00\n").unwrap();
            Self { dir }
        }
        fn paths(&self) -> Paths { Paths { cgroup_root: self.dir.path().join("cg"), proc_root: self.dir.path().join("proc"), min_depth: 2, killer: None } }
        fn cg(&self, inst: &str) -> PathBuf { self.dir.path().join("cg").join(format!("lxc.payload.{inst}")) }
        fn memory(&self, inst: &str, used: u64, limit: u64, full10: f64, full60: f64, oom_kill: u64, oom_local: u64) {
            let d = self.cg(inst);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("memory.current"), format!("{used}\n")).unwrap();
            fs::write(d.join("memory.max"), if limit == 0 { "max\n".to_string() } else { format!("{limit}\n") }).unwrap();
            fs::write(d.join("memory.stat"), format!("anon {}\nfile {}\nshmem 0\n", used / 2, used / 4)).unwrap();
            fs::write(d.join("memory.pressure"), format!("some avg10={full10:.2} avg60={full60:.2} avg300=0.00 total=1\nfull avg10={full10:.2} avg60={full60:.2} avg300=0.00 total=1\n")).unwrap();
            let max = fs::read_to_string(d.join("memory.events.local")).ok().and_then(|t| flat_value(&t, "max")).unwrap_or(3);
            fs::write(d.join("memory.events"), format!("low 0\nhigh 0\nmax {max}\noom {oom_local}\noom_kill {oom_kill}\noom_group_kill 0\n")).unwrap();
            fs::write(d.join("memory.events.local"), format!("low 0\nhigh 0\nmax {max}\noom {oom_local}\noom_kill 0\n")).unwrap();
        }
        /// The usage ran into the container's own limit again (what a container stalled at its limit does all the time)
        fn hit_limit(&self, inst: &str) {
            for f in ["memory.events", "memory.events.local"] {
                let path = self.cg(inst).join(f);
                let Ok(text) = fs::read_to_string(&path) else { continue };
                let max = flat_value(&text, "max").unwrap_or(0);
                fs::write(&path, text.replace(&format!("max {max}\n"), &format!("max {}\n", max + 50))).unwrap();
            }
        }
        #[allow(clippy::too_many_arguments)]
        fn proc(&self, inst: &str, sub: &str, pid: i32, nspid: &[i32], comm: &str, cmd: &[&str], anon_mib: u64, adj: i32, start: u64) {
            let cgd = self.cg(inst).join(sub);
            fs::create_dir_all(&cgd).unwrap();
            let mut procs = fs::read_to_string(cgd.join("cgroup.procs")).unwrap_or_default();
            procs.push_str(&format!("{pid}\n"));
            fs::write(cgd.join("cgroup.procs"), procs).unwrap();
            self.proc_files(pid, nspid, comm, cmd, anon_mib, adj, start, &format!("/lxc.payload.{inst}{}{sub}", if sub.is_empty() { "" } else { "/" }));
        }
        #[allow(clippy::too_many_arguments)]
        fn proc_files(&self, pid: i32, nspid: &[i32], comm: &str, cmd: &[&str], anon_mib: u64, adj: i32, start: u64, cgroup: &str) {
            let d = self.dir.path().join("proc").join(pid.to_string());
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("stat"), format!("{pid} ({comm}) S 1 {pid} {pid} 0 -1 4194560 10 0 0 0 5 5 0 0 20 0 1 0 {start} 1000000 100 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0\n")).unwrap();
            fs::write(d.join("status"), format!("Name:\t{comm}\nVmRSS:\t{} kB\nRssAnon:\t{} kB\nRssFile:\t100 kB\nNSpid:\t{}\n", anon_mib * MIB_KB + 100, anon_mib * MIB_KB, nspid.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("\t"))).unwrap();
            fs::write(d.join("oom_score_adj"), format!("{adj}\n")).unwrap();
            fs::write(d.join("cmdline"), cmd.join("\0") + "\0").unwrap();
            fs::write(d.join("cgroup"), format!("0::{cgroup}\n")).unwrap();
            fs::write(d.join("environ"), "PATH=/usr/bin\0CLOUDOS_SESSION=sess-1\0HOME=/home/coder\0").unwrap();
        }
        fn adj(&self, pid: i32) -> String { fs::read_to_string(self.dir.path().join("proc").join(pid.to_string()).join("oom_score_adj")).unwrap().trim().to_string() }
        /// A typical workspace: init, agentd (the largest of all), dockerd, a conversation with a runaway script, a small shell
        fn workspace(&self, inst: &str) {
            self.proc(inst, "init.scope", 1001, &[1001, 1], "systemd", &["/sbin/init"], 3, 0, 100);
            self.proc(inst, "system.slice/cloudos-agentd.service", 1002, &[1002, 210], "cloudos-agentd", &["/usr/local/bin/cloudos-agentd", "--listen", "0.0.0.0:7200"], 900, 0, 200);
            self.proc(inst, "system.slice/docker.service", 1003, &[1003, 300], "dockerd", &["/usr/bin/dockerd"], 30, 0, 300);
            self.proc(inst, "system.slice/cloudos-agentd.service", 1004, &[1004, 400], "claude", &["claude", "-p"], 200, 0, 400);
            self.proc(inst, "system.slice/cloudos-agentd.service", 1005, &[1005, 500], "node", &["node", "rust/tools/gen-maps.mjs"], 2600, 0, 500);
            self.proc(inst, "user.slice", 1006, &[1006, 600], "bash", &["bash", "-l"], 4, 0, 600);
        }
    }

    fn cfg(action: &str) -> PressureConfig { PressureConfig { action: action.into(), ..Default::default() } }

    #[test]
    fn parsers() {
        assert_eq!(parse_pressure("some avg10=1.50 avg60=2.00 avg300=0.10 total=95981\nfull avg10=61.90 avg60=62.03 avg300=40.00 total=80118\n"), (61.9, 62.03));
        assert_eq!(parse_pressure(""), (0.0, 0.0));
        let ev = "low 0\nhigh 0\nmax 12\noom 2\noom_kill 3\noom_group_kill 0\nsock_throttled 835\n";
        assert_eq!((flat_value(ev, "oom_kill"), flat_value(ev, "oom"), flat_value(ev, "nope")), (Some(3), Some(2), None));
        // a comm with spaces and brackets must not shift the fields
        let stat = "4242 (my (odd) name) R 1 4242 4242 0 -1 4194560 10 0 0 0 5 5 0 0 20 0 1 0 987654 1000000 100 1 1 1 0\n";
        assert_eq!(parse_stat(stat), Some(("my (odd) name".to_string(), 987654)));
        assert_eq!(parse_stat("garbage"), None);
        assert_eq!(parse_status("Name:\tnode\nVmRSS:\t  3000 kB\nRssAnon:\t  2048 kB\nNSpid:\t5005\t77\n"), (2048 * 1024, 3000 * 1024, vec![5005, 77]));
        // the shield: the process's memory as thousandths of the limit plus the margin, between -50 and -900
        assert_eq!((shield_adj(0, 4 * GIB), shield_adj(3 << 20, 4 * GIB), shield_adj(470 << 20, 4 * GIB), shield_adj(470 << 20, 2 * GIB), shield_adj(8 * GIB, 4 * GIB), shield_adj(1 << 20, 0)), (-50, -51, -165, -280, -900, -50));
        assert!(cgroup_within("0::/lxc.payload.ws-admin-alpha/system.slice/a.service\n", "/lxc.payload.ws-admin-alpha"));
        assert!(cgroup_within("0::/lxc.payload.ws-admin-alpha\n", "/lxc.payload.ws-admin-alpha"));
        assert!(!cgroup_within("0::/lxc.payload.ws-admin-alpha2/init.scope\n", "/lxc.payload.ws-admin-alpha"), "сосед с похожим именем — чужой");
        assert!(!cgroup_within("0::/system.slice/ssh.service\n", "/lxc.payload.ws-admin-alpha"));
        assert_eq!(session_of(b"A=1\0CLOUDOS_SESSION=abc-1\0B=2\0").as_deref(), Some("abc-1"));
        assert_eq!(session_of(b"A=1\0"), None);
        assert_eq!(cmdline_text(b"node\0a b.mjs\0", "node"), "node a b.mjs");
        assert_eq!(cmdline_text(b"", "kworker"), "[kworker]");
    }

    #[test]
    fn protected_set() {
        assert!(is_protected("systemd", &[900, 1]), "init workspace'а");
        assert!(is_protected("anything", &[900, 1]));
        assert!(!is_protected("node", &[900, 120, 1]), "главный процесс вложенного контейнера — обычный кандидат");
        for c in ["cloudos-agentd", "coder", "dockerd", "containerd", "containerd-shim", "systemd-journal", "systemd-logind"] { assert!(is_protected(c, &[900, 50]), "{c}"); }
        for c in ["node", "claude", "python3", "postgres", "code-server", "coder2", "bash"] { assert!(!is_protected(c, &[900, 50]), "{c}"); }
    }

    #[test]
    fn config_defaults_merge_and_ranges() {
        let c = PressureConfig::default();
        assert_eq!((c.enabled, c.action.as_str(), c.full_pct, c.hold_secs, c.tight_pct, c.long_turn_hours), (true, "kill", 40.0, 120, 10.0, 2.0));
        c.validate().unwrap();
        let e = c.effective("alpha");
        assert_eq!(e, Effective { enabled: true, kill: true, full_pct: 40.0, hold: Duration::from_secs(120), tight_pct: 10.0 });
        // an empty stored document means the defaults
        assert_eq!(serde_json::from_value::<PressureConfig>(serde_json::json!({})).unwrap(), c);
        // a partial update keeps the rest; a project override is upserted and may go below the stand-wide range
        let c2 = c.merged(&serde_json::json!({ "action": "notify", "projects": { "e2e": { "full_pct": 2, "hold_secs": 5, "action": "kill", "long_turn_hours": 0.01 }, "gamma": { "action": "notify" } } })).unwrap();
        assert_eq!((c2.action.as_str(), c2.full_pct, c2.projects.len()), ("notify", 40.0, 2));
        assert_eq!(c2.effective("e2e"), Effective { enabled: true, kill: true, full_pct: 2.0, hold: Duration::from_secs(5), tight_pct: 2.0 });
        assert!(!c2.effective("alpha").kill && !c2.effective("gamma").kill);
        // null removes one override, the others stay; an empty override is dropped
        let c3 = c2.merged(&serde_json::json!({ "projects": { "e2e": null, "beta": {} } })).unwrap();
        assert_eq!(c3.projects.keys().cloned().collect::<Vec<_>>(), vec!["gamma".to_string()]);
        // stand-wide thresholds cannot be lowered into the dangerous range, unknown keys and actions are refused
        for bad in [serde_json::json!({ "full_pct": 5 }), serde_json::json!({ "hold_secs": 5 }), serde_json::json!({ "action": "reboot" }), serde_json::json!({ "tight_pct": 90 }), serde_json::json!({ "long_turn_hours": 0.01 }),
                    serde_json::json!({ "nope": 1 }), serde_json::json!({ "projects": { "x": { "action": "burn" } } }), serde_json::json!({ "projects": { "x": { "full_pct": 0 } } }), serde_json::json!([1])] {
            let e = c.merged(&bad).unwrap_err().to_string();
            assert!(e.contains("недопустимое значение"), "{bad}: {e}");
        }
        assert!(c.merged(&serde_json::json!({ "long_turn_hours": 0 })).is_ok(), "0 выключает сообщение о долгой команде");
    }

    #[test]
    fn automaton() {
        let e = PressureConfig::default().effective("x");
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let smp = |full10: f64, full60: f64| Sample { full10, full60, limit: 4 * GIB, used: 4 * GIB, ..Default::default() };
        let mut t = Track::default();
        assert_eq!(step(&mut t, &smp(0.0, 3.0), &e, at(0)), (Level::Ok, false));
        assert_eq!(step(&mut t, &smp(30.0, 12.0), &e, at(5)), (Level::Tight, false));
        // over the threshold: tight until the hold time has passed
        assert_eq!(step(&mut t, &smp(70.0, 45.0), &e, at(10)), (Level::Tight, false));
        assert_eq!(step(&mut t, &smp(70.0, 60.0), &e, at(100)), (Level::Tight, false));
        assert_eq!(step(&mut t, &smp(70.0, 62.0), &e, at(130)), (Level::Stalled, true));
        // after an action — a pause, the level stays
        t.cooldown_until = Some(at(190));
        assert_eq!(step(&mut t, &smp(70.0, 62.0), &e, at(135)), (Level::Stalled, false));
        // avg60 still remembers the stall, avg10 says it is over: no action, the label goes back to "tight"
        assert_eq!(step(&mut t, &smp(5.0, 50.0), &e, at(195)), (Level::Tight, false));
        // the stall goes on after the pause — act again without a new hold time
        assert_eq!(step(&mut t, &smp(80.0, 55.0), &e, at(200)), (Level::Stalled, true));
        // a dip under the threshold ends the episode: the hold time starts over, "told" is cleared
        t.told = true;
        assert_eq!(step(&mut t, &smp(1.0, 20.0), &e, at(300)), (Level::Tight, false));
        assert!(!t.told && t.over_since.is_none());
        assert_eq!(step(&mut t, &smp(90.0, 41.0), &e, at(305)), (Level::Tight, false));
        assert_eq!(step(&mut t, &smp(90.0, 41.0), &e, at(424)), (Level::Tight, false));
        assert_eq!(step(&mut t, &smp(90.0, 41.0), &e, at(425)), (Level::Stalled, true));
    }

    #[test]
    fn choose_takes_the_kernels_victim_and_never_the_infrastructure() {
        let f = Fake::new();
        f.memory("ws-admin-a", 4 * GIB, 4 * GIB, 60.0, 60.0, 0, 0);
        f.workspace("ws-admin-a");
        let p = f.paths();
        let mut procs = scan(&p, &f.cg("ws-admin-a")); describe_procs(&p, &mut procs, &mut HashMap::new(), 4 * GIB);
        assert_eq!(procs.len(), 6);
        // largest first; the full command line and the conversation — for the big ordinary ones
        assert_eq!(procs[0].comm, "node");
        assert_eq!((procs[0].cmd.as_str(), procs[0].session.as_deref(), procs[0].ws_pid, procs[0].depth), ("node rust/tools/gen-maps.mjs", Some("sess-1"), 500, 2));
        let agentd = procs.iter().find(|x| x.comm == "cloudos-agentd").unwrap();
        assert!(agentd.protected && agentd.session.is_none());
        assert!(procs.iter().find(|x| x.pid == 1001).unwrap().protected, "init workspace'а");
        assert_eq!(choose(&procs, 4 * GIB, 2).map(|x| x.pid), Some(1005));
        assert_eq!(largest(&procs, 2).map(|x| x.pid), Some(1005));

        // without the runaway script nothing holds a tenth of the limit: agentd (900 МиБ) is the largest, but it is not a candidate
        let rest: Vec<Proc> = procs.iter().filter(|x| x.pid != 1005).cloned().collect();
        assert_eq!(choose(&rest, 4 * GIB, 2), None);
        assert_eq!(largest(&rest, 2).map(|x| x.comm.as_str()), Some("claude"), "для подсказки — крупнейший обычный процесс");

        // a positive oom_score_adj moves a smaller process ahead, as in the kernel; a negative one on an ordinary process
        // is our own inherited protection and counts as 0
        let mut two = vec![procs[0].clone(), procs[0].clone()];
        two[1].pid = 2000; two[1].anon = 600 * 1024 * 1024; two[1].adj = 800;
        assert_eq!(choose(&two, 4 * GIB, 2).map(|x| x.pid), Some(2000));
        two[1].adj = -900;
        assert_eq!(choose(&two, 4 * GIB, 2).map(|x| x.pid), Some(1005));
        two[0].adj = -900;
        assert_eq!(choose(&two, 4 * GIB, 2).map(|x| x.pid), Some(1005), "оба унаследовали защиту — решает память");

        // a process of the host's own pid namespace is never touched, whatever cgroup it sits in
        let mut host = procs[0].clone();
        host.depth = 1;
        assert_eq!(choose(&[host.clone()], 4 * GIB, 2), None);
        assert_eq!(largest(&[host], 2), None);
        // the main process of a container nested in the workspace is an ordinary candidate
        f.proc("ws-admin-a", "system.slice/docker-abc.scope", 1010, &[1010, 700, 1], "postgres", &["postgres", "-D", "/data"], 3000, 0, 700);
        let mut procs = scan(&p, &f.cg("ws-admin-a")); describe_procs(&p, &mut procs, &mut HashMap::new(), 4 * GIB);
        assert_eq!(choose(&procs, 4 * GIB, 2).map(|x| x.pid), Some(1010));
    }

    #[test]
    fn protection_pass_sets_the_infrastructure_and_resets_inherited_values() {
        let f = Fake::new();
        f.memory("ws-admin-a", GIB, 4 * GIB, 0.0, 0.0, 0, 0);
        f.workspace("ws-admin-a");
        // a conversation that inherited agentd's protection, a process that asked to go first, dockerd half-protected by its unit
        f.proc("ws-admin-a", "system.slice/cloudos-agentd.service", 1007, &[1007, 710], "claude", &["claude"], 150, -900, 710);
        f.proc("ws-admin-a", "user.slice", 1008, &[1008, 720], "chrome", &["chrome", "--type=renderer"], 100, 300, 720);
        f.proc("ws-admin-a", "system.slice/containerd.service", 1009, &[1009, 730], "containerd", &["/usr/bin/containerd"], 12, -500, 730);
        let p = f.paths();
        let rel = "/lxc.payload.ws-admin-a";
        let mut procs = scan(&p, &f.cg("ws-admin-a")); describe_procs(&p, &mut procs, &mut HashMap::new(), 4 * GIB);
        assert_eq!(protect(&p, rel, &mut procs, 4 * GIB), 5, "init, agentd, dockerd, containerd получают щит; унаследовавшая беседа → 0");
        // the shield is as deep as the process is large: agentd (900 МиБ of 4 ГиБ) — 220 ‰ + 50, the small ones — just the margin;
        // containerd's far deeper -500 is raised back
        assert_eq!((f.adj(1001), f.adj(1002), f.adj(1003), f.adj(1009)), ("-51".into(), "-270".into(), "-58".into(), "-53".into()));
        assert_eq!((f.adj(1007), f.adj(1004), f.adj(1005), f.adj(1008)), ("0".into(), "0".into(), "0".into(), "300".into()), "положительное значение не трогаем");
        // the second pass has nothing to do
        let mut procs = scan(&p, &f.cg("ws-admin-a")); describe_procs(&p, &mut procs, &mut HashMap::new(), 4 * GIB);
        assert_eq!(protect(&p, rel, &mut procs, 4 * GIB), 0);
        // agentd grew: the shield deepens; it shrank a little: nothing is rewritten
        let mut agentd = procs.iter().find(|x| x.pid == 1002).unwrap().clone();
        agentd.rss = 2 * GIB;
        assert_eq!(adj_fix(&agentd, 4 * GIB), Some(-550));
        agentd.rss = 800 << 20;
        assert_eq!(adj_fix(&agentd, 4 * GIB), None);

        // the pid now belongs to another process (another start time) or to another cgroup — nothing is written
        let agentd = procs.iter().find(|x| x.pid == 1002).unwrap().clone();
        f.proc_files(1002, &[1002, 210], "cloudos-agentd", &["x"], 10, 0, 999_999, "/lxc.payload.ws-admin-a/system.slice/x.service");
        assert!(set_adj(&p, rel, &agentd, -900).unwrap_err().to_string().contains("другой процесс"));
        assert_eq!(f.adj(1002), "0");
        f.proc_files(1002, &[1002, 210], "cloudos-agentd", &["x"], 10, 0, agentd.start, "/system.slice/ssh.service");
        assert!(set_adj(&p, rel, &agentd, -900).unwrap_err().to_string().contains("не в /lxc.payload.ws-admin-a"));
        f.proc_files(1002, &[1002], "cloudos-agentd", &["x"], 10, 0, agentd.start, "/lxc.payload.ws-admin-a/system.slice/x.service");
        assert!(set_adj(&p, rel, &agentd, -900).unwrap_err().to_string().contains("не в пространстве контейнера"));
        assert_eq!(f.adj(1002), "0");
    }

    /// A process stalled at its memory limit holds its memory map lock: its command line and environment cannot be read for
    /// seconds. The pass must not wait: a name learned while the workspace was calm stays, an unread one shows the comm
    #[test]
    fn a_hanging_command_line_does_not_hold_the_pass() {
        let f = Fake::new();
        f.memory("ws-admin-a", 4 * GIB, 4 * GIB, 62.0, 62.0, 0, 0);
        f.workspace("ws-admin-a");
        let p = f.paths();
        let mut meta = HashMap::new();
        let mut procs = scan(&p, &f.cg("ws-admin-a"));
        assert_eq!(procs[0].cmd, "[node]", "сам обход читает только то, что не ждёт память процесса");
        describe_procs(&p, &mut procs, &mut meta, 4 * GIB);
        assert_eq!((procs[0].cmd.as_str(), procs[0].session.as_deref()), ("node rust/tools/gen-maps.mjs", Some("sess-1")));
        // now every command line and environment of the workspace hangs (a FIFO nobody writes to)
        let fifo = |pid: i32, name: &str| {
            let path = f.dir.path().join("proc").join(pid.to_string()).join(name);
            fs::remove_file(&path).unwrap();
            let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
            path
        };
        let mut fifos = Vec::new();
        for pid in [1001, 1002, 1003, 1004, 1005, 1006] { fifos.push(fifo(pid, "cmdline")); fifos.push(fifo(pid, "environ")); }
        // a new large process appears during the stall
        f.proc("ws-admin-a", "user.slice", 1050, &[1050, 900], "python3", &["python3", "late.py"], 1200, 0, 900);
        fifos.push(fifo(1050, "cmdline"));
        let started = Instant::now();
        let mut procs = scan(&p, &f.cg("ws-admin-a"));
        describe_procs(&p, &mut procs, &mut meta, 4 * GIB);
        assert!(started.elapsed() < Duration::from_millis(900), "один невернувшийся ответ — и проход идёт дальше: {:?}", started.elapsed());
        let by = |pid: i32| procs.iter().find(|x| x.pid == pid).unwrap();
        assert_eq!((by(1005).cmd.as_str(), by(1005).session.as_deref()), ("node rust/tools/gen-maps.mjs", Some("sess-1")), "названный до пробуксовки процесс сохранил имя и беседу");
        assert_eq!((by(1050).cmd.as_str(), by(1050).session.as_deref()), ("[python3]", None), "новый — только comm");
        // the choice and the checks before the signal never touch those files
        assert_eq!(choose(&procs, 4 * GIB, 2).map(|x| x.pid), Some(1005));
        let d = ProcDir::open(&p.proc_root, 1005).unwrap();
        verify(&d, by(1005), "/lxc.payload.ws-admin-a", 2).unwrap();
        // let the helper threads go
        for path in fifos {
            let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
            let fd = unsafe { libc::open(c.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
            if fd >= 0 { unsafe { libc::close(fd) }; }
        }
    }

    /// Runs `pass` on the fake host with a recording "killer"
    struct Rig { f: Fake, state: PressureState, killed: Arc<Mutex<Vec<i32>>>, paths: Paths, t0: Instant, own_limit: std::cell::Cell<bool> }
    impl Rig {
        fn new() -> Self {
            let f = Fake::new();
            let killed = Arc::new(Mutex::new(Vec::new()));
            let k2 = killed.clone();
            let mut paths = f.paths();
            paths.killer = Some(Arc::new(move |p: &Proc| { k2.lock().unwrap().push(p.pid); Ok(()) }));
            Self { f, state: PressureState::default(), killed, paths, t0: Instant::now(), own_limit: std::cell::Cell::new(true) }
        }
        fn pass(&self, cfg: &PressureConfig, secs: u64, kmsg: Option<&[String]>) -> Vec<Notice> {
            if self.own_limit.get() { self.f.hit_limit("ws-admin-a"); self.f.hit_limit("ws-admin-b"); }
            self.state.pass(&self.paths, cfg, &[("a".to_string(), "ws-admin-a".to_string()), ("b".to_string(), "ws-admin-b".to_string())], (secs / 5) as u32, kmsg, self.t0 + Duration::from_secs(secs))
        }
        fn level(&self, project: &str) -> String { self.state.view(project).map(|v| v.state).unwrap_or_else(|| "none".into()) }
    }

    #[test]
    fn stalled_workspace_loses_its_largest_process_and_only_that() {
        let r = Rig::new();
        r.f.memory("ws-admin-a", 4 * GIB - 1000, 4 * GIB, 62.0, 62.0, 0, 0);
        r.f.workspace("ws-admin-a");
        r.f.memory("ws-admin-b", GIB, 4 * GIB, 0.0, 1.0, 0, 0);
        r.f.proc("ws-admin-b", "user.slice", 3001, &[3001, 50], "node", &["node", "big.js"], 3000, 0, 50);
        let c = cfg("kill");
        assert!(r.pass(&c, 0, None).is_empty());
        assert_eq!((r.level("a"), r.level("b")), ("tight".to_string(), "ok".to_string()));
        assert!(r.pass(&c, 60, None).is_empty());
        assert!(r.killed.lock().unwrap().is_empty(), "до выдержки никого не трогаем");
        let n = r.pass(&c, 120, None);
        assert_eq!(*r.killed.lock().unwrap(), vec![1005], "завершён именно крупнейший процесс стоящего workspace'а");
        assert_eq!(n.len(), 1);
        let a = &n[0].action;
        assert_eq!((n[0].project.as_str(), a.kind.as_str(), a.cmd.as_str(), a.rss, a.session.as_deref(), a.stalled_secs, a.limit, a.note.clone()), ("a", "killed", "node rust/tools/gen-maps.mjs", 2600 * 1024 * 1024, Some("sess-1"), 120, 4 * GIB, None));
        assert_eq!(a.age_secs, 100_000 - 5, "возраст — от времени старта процесса");
        let v = r.state.view("a").unwrap();
        assert_eq!((v.state.as_str(), v.action.as_str(), v.used, v.limit, v.full60), ("stalled", "kill", 4 * GIB - 1000, 4 * GIB, 62.0));
        assert_eq!(v.last.as_ref().map(|l| l.kind.as_str()), Some("killed"));
        assert_eq!(r.state.view("b").unwrap().last, None);
        // the pause after an action: the next pass does nothing even though the pressure is still there
        assert!(r.pass(&c, 125, None).is_empty());
        assert_eq!(r.killed.lock().unwrap().len(), 1);
        // the report text names the process, its size and the stall
        let (title, body) = describe("a", a);
        assert!(title.contains("завершён процесс") && body.contains("node rust/tools/gen-maps.mjs") && body.contains("2.5 ГиБ") && body.contains("2 мин") && body.contains("62 %"), "{title} / {body}");
        // the protection ran on the stalled workspace too
        assert_eq!((r.f.adj(1002), r.f.adj(1001), r.f.adj(1005)), ("-270".into(), "-51".into(), "0".into()));
        // a stopped workspace leaves the state
        fs::remove_dir_all(r.f.cg("ws-admin-b")).unwrap();
        r.pass(&c, 130, None);
        assert_eq!(r.level("b"), "none");
    }

    #[test]
    fn notify_mode_small_processes_and_the_hourly_limit_only_report() {
        // notify: one report per stall, nobody ended, the card names the largest process
        let r = Rig::new();
        r.f.memory("ws-admin-a", 4 * GIB, 4 * GIB, 62.0, 62.0, 0, 0);
        r.f.workspace("ws-admin-a");
        let c = cfg("notify");
        r.pass(&c, 0, None);
        let n = r.pass(&c, 120, None);
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].action.kind.as_str(), n[0].action.note.as_deref(), n[0].action.cmd.as_str()), ("stalled", Some("notify"), "node rust/tools/gen-maps.mjs"));
        assert!(r.pass(&c, 125, None).is_empty() && r.pass(&c, 600, None).is_empty(), "одно сообщение на эпизод");
        assert!(r.killed.lock().unwrap().is_empty());
        assert_eq!(r.state.view("a").unwrap().action, "notify");
        let (title, body) = describe("a", &n[0].action);
        assert!(title.contains("стоит у лимита") && body.contains("крупнейший процесс: node rust/tools/gen-maps.mjs"), "{title} / {body}");
        // a project override beats the stand-wide mode
        let mut c2 = cfg("kill");
        c2.projects.insert("a".into(), ProjectOverride { action: Some("notify".into()), ..Default::default() });
        let r2 = Rig::new();
        r2.f.memory("ws-admin-a", 4 * GIB, 4 * GIB, 62.0, 62.0, 0, 0);
        r2.f.workspace("ws-admin-a");
        r2.pass(&c2, 0, None);
        assert_eq!(r2.pass(&c2, 120, None)[0].action.note.as_deref(), Some("notify"));
        assert!(r2.killed.lock().unwrap().is_empty());

        // kill mode, but nothing holds a tenth of the limit: a report, not a kill
        let r3 = Rig::new();
        r3.f.memory("ws-admin-a", 4 * GIB, 4 * GIB, 62.0, 62.0, 0, 0);
        r3.f.proc("ws-admin-a", "init.scope", 1001, &[1001, 1], "systemd", &["/sbin/init"], 3, 0, 100);
        r3.f.proc("ws-admin-a", "system.slice/cloudos-agentd.service", 1002, &[1002, 210], "cloudos-agentd", &["cloudos-agentd"], 900, 0, 200);
        for i in 0..6 { r3.f.proc("ws-admin-a", "system.slice/cloudos-agentd.service", 1100 + i, &[1100 + i, 800 + i], "claude", &["claude", "-p"], 300, 0, 800); }
        let c = cfg("kill");
        r3.pass(&c, 0, None);
        let n = r3.pass(&c, 120, None);
        assert_eq!((n.len(), n[0].action.kind.as_str(), n[0].action.note.as_deref()), (1, "stalled", Some("small")));
        assert!(r3.killed.lock().unwrap().is_empty(), "agentd — крупнейший, но он под защитой; остальные меньше десятой части лимита");

        // the workspace stalls far below its own limit: the host is short of memory — report, do not end anything here
        let r5 = Rig::new();
        r5.f.memory("ws-admin-a", 2 * GIB, 4 * GIB, 62.0, 62.0, 0, 0);
        r5.f.workspace("ws-admin-a");
        r5.pass(&c, 0, None);
        let n = r5.pass(&c, 120, None);
        assert_eq!((n.len(), n[0].action.kind.as_str(), n[0].action.note.as_deref()), (1, "stalled", Some("host")));
        assert!(r5.killed.lock().unwrap().is_empty());
        assert!(describe("a", &n[0].action).1.contains("не хватает самому хосту"));
        // …the same when the workspace is full (of page cache), but its own limit was never hit during the stall
        let r6 = Rig::new();
        r6.own_limit.set(false);
        r6.f.memory("ws-admin-a", 4 * GIB, 4 * GIB, 62.0, 62.0, 0, 0);
        r6.f.workspace("ws-admin-a");
        r6.pass(&c, 0, None);
        assert_eq!(r6.pass(&c, 120, None)[0].action.note.as_deref(), Some("host"));
        assert!(r6.killed.lock().unwrap().is_empty());

        // three kills within an hour, then only a report; an hour later the watch may act again
        let r4 = Rig::new();
        r4.f.memory("ws-admin-a", 4 * GIB, 4 * GIB, 62.0, 62.0, 0, 0);
        for i in 0..5 { r4.f.proc("ws-admin-a", "user.slice", 1200 + i, &[1200 + i, 900 + i], "python3", &["python3", "hog.py"], 700, 0, 900); }
        r4.pass(&c, 0, None);
        let mut kinds = Vec::new();
        for t in [120u64, 185, 250, 315, 380] { for n in r4.pass(&c, t, None) { kinds.push((n.action.kind.clone(), n.action.note.clone())); } }
        assert_eq!(kinds, vec![("killed".to_string(), None), ("killed".to_string(), None), ("killed".to_string(), None), ("stalled".to_string(), Some("rate".to_string()))]);
        assert_eq!(r4.killed.lock().unwrap().len(), 3);
        let n = r4.pass(&c, 120 + 3700, None);
        assert_eq!(n.iter().map(|n| n.action.kind.as_str()).collect::<Vec<_>>(), vec!["killed"], "через час лимит завершений сброшен");
    }

    #[test]
    fn disabled_watch_shows_the_state_and_touches_nothing() {
        let r = Rig::new();
        r.f.memory("ws-admin-a", 4 * GIB, 4 * GIB, 62.0, 62.0, 0, 0);
        r.f.workspace("ws-admin-a");
        let c = PressureConfig { enabled: false, ..Default::default() };
        r.pass(&c, 0, None);
        assert!(r.pass(&c, 120, None).is_empty());
        assert!(r.killed.lock().unwrap().is_empty());
        assert_eq!((r.f.adj(1002), r.f.adj(1001)), ("0".into(), "0".into()), "защита инфраструктуры тоже выключена");
        let v = r.state.view("a").unwrap();
        assert_eq!((v.state.as_str(), v.action.as_str()), ("stalled", "off"));
        assert_eq!(v.top.as_ref().map(|t| t.cmd.as_str()), Some("node rust/tools/gen-maps.mjs"));
    }

    #[test]
    fn kernel_oom_is_reported_for_the_workspace_limit_only() {
        let line = |memcg: &str, task_memcg: &str, comm: &str, pid: i32| format!("oom-kill:constraint=CONSTRAINT_MEMCG,nodemask=(null),cpuset=lxc.payload.ws-admin-a,mems_allowed=0,oom_memcg={memcg},task_memcg={task_memcg},task={comm},pid={pid},uid=1001000");
        let killed = |pid: i32, comm: &str, anon_kb: u64| format!("Memory cgroup out of memory: Killed process {pid} ({comm}) total-vm:9000000kB, anon-rss:{anon_kb}kB, file-rss:0kB, shmem-rss:0kB, UID:1001000 pgtables:5000kB oom_score_adj:0");
        let lines = vec![line("/lxc.payload.ws-admin-a", "/lxc.payload.ws-admin-a/system.slice/cloudos-agentd.service", "node", 1005), killed(1005, "node", 2_700_000)];
        let k = parse_oom(&lines);
        assert_eq!(k, vec![OomKill { pid: 1005, comm: "node".into(), oom_memcg: "/lxc.payload.ws-admin-a".into(), task_memcg: "/lxc.payload.ws-admin-a/system.slice/cloudos-agentd.service".into(), anon: Some(2_700_000 * 1024) }]);
        assert!(parse_oom(&["audit: type=1400 apparmor=\"DENIED\"".to_string()]).is_empty());

        let c = cfg("kill");
        let r = Rig::new();
        r.f.memory("ws-admin-a", 3 * GIB, 4 * GIB, 0.0, 0.0, 0, 0);
        r.f.workspace("ws-admin-a");
        assert!(r.pass(&c, 0, Some(&[])).is_empty(), "первый проход только запоминает счётчики");
        // the kernel ended the script at the workspace limit: the report names it from the last scan
        r.f.memory("ws-admin-a", GIB, 4 * GIB, 0.0, 0.0, 1, 1);
        let n = r.pass(&c, 5, Some(&lines));
        assert_eq!(n.len(), 1);
        let a = &n[0].action;
        assert_eq!((a.kind.as_str(), a.cmd.as_str(), a.rss, a.session.as_deref(), a.limit), ("oom", "node rust/tools/gen-maps.mjs", 2_700_000 * 1024, Some("sess-1"), 4 * GIB));
        let (title, body) = describe("a", a);
        assert!(title.contains("завершён системой") && body.contains("node rust/tools/gen-maps.mjs"), "{title} / {body}");
        assert!(r.killed.lock().unwrap().is_empty());
        // a second one within ten minutes is not reported again
        r.f.memory("ws-admin-a", GIB, 4 * GIB, 0.0, 0.0, 2, 2);
        assert!(r.pass(&c, 10, Some(&lines)).is_empty());

        // a container nested in the workspace hit its own limit: the subtree counter grows, the workspace's own does not
        let r2 = Rig::new();
        r2.f.memory("ws-admin-a", GIB, 4 * GIB, 0.0, 0.0, 0, 0);
        r2.f.workspace("ws-admin-a");
        r2.pass(&c, 0, Some(&[]));
        r2.f.memory("ws-admin-a", GIB, 4 * GIB, 0.0, 0.0, 1, 0);
        let nested = vec![line("/lxc.payload.ws-admin-a/system.slice/docker-abc.scope", "/lxc.payload.ws-admin-a/system.slice/docker-abc.scope", "java", 4444), killed(4444, "java", 500_000)];
        assert!(r2.pass(&c, 5, Some(&nested)).is_empty());
        // …and a neighbour workspace's report is not ours
        r2.f.memory("ws-admin-a", GIB, 4 * GIB, 0.0, 0.0, 2, 0);
        let other = vec![line("/lxc.payload.ws-admin-ab", "/lxc.payload.ws-admin-ab/init.scope", "node", 7)];
        assert!(r2.pass(&c, 700, Some(&other)).is_empty());
        assert!(r2.pass(&c, 705, Some(&[])).is_empty(), "отложенная проверка без записи и без роста своего счётчика — тишина");

        // the log line lands one tick after the counter: the report waits for it
        let r3 = Rig::new();
        r3.f.memory("ws-admin-a", 3 * GIB, 4 * GIB, 0.0, 0.0, 0, 0);
        r3.f.workspace("ws-admin-a");
        r3.pass(&c, 0, Some(&[]));
        r3.f.memory("ws-admin-a", GIB, 4 * GIB, 0.0, 0.0, 1, 1);
        assert!(r3.pass(&c, 5, Some(&[])).is_empty());
        let n = r3.pass(&c, 10, Some(&lines));
        assert_eq!(n.iter().map(|n| (n.action.kind.as_str(), n.action.cmd.as_str())).collect::<Vec<_>>(), vec![("oom", "node rust/tools/gen-maps.mjs")]);
        // no kernel log at all: the workspace's own counter is enough for a nameless report
        let r4 = Rig::new();
        r4.f.memory("ws-admin-a", 3 * GIB, 4 * GIB, 0.0, 0.0, 0, 0);
        r4.pass(&c, 0, None);
        r4.f.memory("ws-admin-a", GIB, 4 * GIB, 0.0, 0.0, 1, 1);
        let n = r4.pass(&c, 5, None);
        assert_eq!((n.len(), n[0].action.kind.as_str(), n[0].action.cmd.as_str()), (1, "oom", ""));
        assert!(describe("a", &n[0].action).1.contains("упёрся в лимит"));
    }

    #[test]
    fn manual_button_ends_the_largest_process_only_when_not_calm() {
        let r = Rig::new();
        r.f.memory("ws-admin-a", GIB, 4 * GIB, 0.0, 0.0, 0, 0);
        r.f.workspace("ws-admin-a");
        let c = cfg("notify");
        r.pass(&c, 0, None);
        assert!(r.state.kill_now(&r.paths, "a", "ws-admin-a").unwrap_err().to_string().contains("завершать нечего"));
        assert!(r.state.kill_now(&r.paths, "zzz", "ws-admin-zzz").unwrap_err().to_string().contains("не запущен"));
        r.f.memory("ws-admin-a", 4 * GIB, 4 * GIB, 62.0, 62.0, 0, 0);
        r.pass(&c, 5, None);
        let a = r.state.kill_now(&r.paths, "a", "ws-admin-a").unwrap();
        assert_eq!((a.kind.as_str(), a.note.as_deref(), a.cmd.as_str()), ("killed", Some("manual"), "node rust/tools/gen-maps.mjs"));
        assert_eq!(*r.killed.lock().unwrap(), vec![1005]);
        assert!(describe("a", &a).1.contains("по кнопке владельца"));
    }

    /// The real signal path on a real child: refused for a wrong cgroup, a changed start time or a protected name; delivered
    /// to exactly the process that was scanned
    #[test]
    fn kill_checked_refuses_anything_but_the_scanned_process() {
        let mut child = std::process::Command::new("sleep").arg("300").spawn().unwrap();
        let pid = child.id() as i32;
        let paths = Paths { cgroup_root: "/sys/fs/cgroup".into(), proc_root: "/proc".into(), min_depth: 1, killer: None };
        // right after spawn the child still carries the name of this test binary: wait until exec has made it `sleep`
        let p = (0..400).find_map(|_| match read_proc(Path::new("/proc"), pid) { Some(p) if p.comm == "sleep" => Some(p), _ => { std::thread::sleep(Duration::from_millis(5)); None } }).expect("дочерний процесс стал sleep");
        let own = fs::read_to_string("/proc/self/cgroup").unwrap().lines().find_map(|l| l.strip_prefix("0::").map(str::to_string)).unwrap_or_else(|| "/".into());
        let own = if own == "/" { String::new() } else { own };
        let alive = |c: &mut std::process::Child| c.try_wait().unwrap().is_none();

        assert!(kill_checked(&paths, "/lxc.payload.ws-admin-nope", &p).unwrap_err().to_string().contains("не в /lxc.payload"));
        let mut other = p.clone(); other.start += 1;
        assert!(kill_checked(&paths, &own, &other).unwrap_err().to_string().contains("другой процесс"));
        let mut prot = p.clone(); prot.protected = true;
        assert!(kill_checked(&paths, &own, &prot).is_err());
        let deep = Paths { min_depth: 9, ..paths.clone() };
        assert!(kill_checked(&deep, &own, &p).unwrap_err().to_string().contains("не в пространстве контейнера"));
        assert!(alive(&mut child), "ни один отказ не задел процесс");

        // an empty prefix would match every cgroup — the test host's own cgroup is the tightest check available here
        if !own.is_empty() {
            kill_checked(&paths, &own, &p).unwrap();
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
            // the pid is gone: a second attempt fails instead of hitting somebody else
            assert!(kill_checked(&paths, &own, &p).is_err());
        } else {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
