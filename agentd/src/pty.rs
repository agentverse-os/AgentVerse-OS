//! PTY daemon: terminals belong to agentd, not to a tab or the IDE (ADR-006, the Orca lesson). Clients attach
//! and detach any number of times; the process lives until it exits on its own or is killed explicitly. Output
//! is broadcast to every attached client; a new client first gets a bounded tail (scrollback) replayed.
use anyhow::{Context, Result};
use bytes::Bytes;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, watch};

/// Output tail an attaching client receives.
pub const SCROLLBACK_MAX: usize = 256 * 1024;
const BROADCAST_CAPACITY: usize = 4096;

pub struct PtyOpts {
    pub cmd: Vec<String>,
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
    pub title: String,
    pub env: HashMap<String, String>,
    /// Session card shell: the PTY belongs to the agent session and goes away with
    /// it
    pub session: Option<String>,
    /// Terminal mode (as in CloudCLI): the agent's native TUI on the same conversation; one per
    /// session
    pub tui: bool,
}

#[derive(Clone, Serialize)]
pub struct PtyInfo {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub cmd: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    pub pid: Option<u32>,
    pub alive: bool,
    pub exit_code: Option<i32>,
    pub created_at: u64,
    pub clients: usize,
    pub session: Option<String>,
    pub tui: bool,
}

pub struct PtySession {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub cmd: Vec<String>,
    pub created_at: u64,
    pub session: Option<String>,
    pub tui: bool,
    pid: Option<u32>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    size: Mutex<(u16, u16)>,
    /// Scrollback and broadcast under one lock: a client that subscribes under it neither misses nor
    /// duplicates bytes.
    scrollback: Mutex<(VecDeque<u8>, broadcast::Sender<Bytes>)>,
    exit: watch::Sender<Option<i32>>,
    pub clients: AtomicUsize,
}

impl PtySession {
    pub fn spawn(opts: PtyOpts) -> Result<Arc<Self>> {
        let pty = native_pty_system();
        let pair = pty.openpty(PtySize { rows: opts.rows, cols: opts.cols, pixel_width: 0, pixel_height: 0 }).context("openpty")?;
        let (argv0, rest) = opts.cmd.split_first().context("пустая команда")?;
        let mut cmd = CommandBuilder::new(argv0);
        cmd.args(rest);
        cmd.cwd(&opts.cwd);
        cmd.env("TERM", "xterm-256color");
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).with_context(|| format!("запуск {:?}", opts.cmd))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().context("reader")?;
        let writer = pair.master.take_writer().context("writer")?;
        let (out_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (exit_tx, _) = watch::channel(None);
        let s = Arc::new(Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: opts.title,
            session: opts.session,
            tui: opts.tui,
            cwd: opts.cwd,
            cmd: opts.cmd,
            created_at: now(),
            pid: child.process_id(),
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            size: Mutex::new((opts.cols, opts.rows)),
            scrollback: Mutex::new((VecDeque::with_capacity(SCROLLBACK_MAX), out_tx)),
            exit: exit_tx,
            clients: AtomicUsize::new(0),
        });
        // Blocking read of the master in a separate thread; EOF means the process closed the
        // terminal.
        let me = s.clone();
        std::thread::Builder::new().name(format!("pty-{}", &me.id[..8])).spawn(move || {
            let mut buf = [0u8; 16384];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut g = me.scrollback.lock().unwrap();
                        let (sb, tx) = &mut *g;
                        sb.extend(&buf[..n]);
                        if sb.len() > SCROLLBACK_MAX {
                            let over = sb.len() - SCROLLBACK_MAX;
                            sb.drain(..over);
                        }
                        let _ = tx.send(Bytes::copy_from_slice(&buf[..n]));
                    }
                }
            }
            let code = me.child.lock().unwrap().wait().map(|st| st.exit_code() as i32).unwrap_or(-1);
            crate::hub::touch();
            // send_replace rather than send: send without a single receiver (nobody attached) does not update
            // the value
            me.exit.send_replace(Some(code));
        })?;
        Ok(s)
    }

    /// Snapshot of the tail plus a subscription to further output — atomic with respect to
    /// the reader thread.
    pub fn attach(&self) -> (Vec<u8>, broadcast::Receiver<Bytes>, watch::Receiver<Option<i32>>) {
        let g = self.scrollback.lock().unwrap();
        let snapshot: Vec<u8> = g.0.iter().copied().collect();
        (snapshot, g.1.subscribe(), self.exit.subscribe())
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.lock().unwrap().resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        *self.size.lock().unwrap() = (cols, rows);
        Ok(())
    }

    /// SIGHUP to the whole process group of the terminal (the shell is the session leader, children are in its
    /// group), SIGKILL after 5 s to whoever has not exited. This way the agent started inside the shell goes away
    /// too, not only the shell itself.
    pub fn kill(&self) -> Result<()> {
        let pid = self.pid.context("нет pid")? as i32;
        // SAFETY: plain kill(2); a negative pid means the whole
        // group.
        if unsafe { libc::kill(-pid, libc::SIGHUP) } != 0 {
            return Err(std::io::Error::last_os_error()).context("kill(SIGHUP)");
        }
        let exit = self.exit.subscribe();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            if exit.borrow().is_none() {
                // SAFETY: see above.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
        });
        Ok(())
    }

    pub fn exit_code(&self) -> Option<i32> {
        *self.exit.borrow()
    }

    /// Wait for the process to exit (after kill) no longer than `timeout`; returns
    /// whether it exited.
    pub async fn wait_exit(&self, timeout: std::time::Duration) -> bool {
        let mut rx = self.exit.subscribe();
        let _ = tokio::time::timeout(timeout, async { while rx.borrow_and_update().is_none() { if rx.changed().await.is_err() { break } } }).await;
        self.exit_code().is_some()
    }

    pub fn info(&self) -> PtyInfo {
        let (cols, rows) = *self.size.lock().unwrap();
        let exit_code = self.exit_code();
        PtyInfo {
            id: self.id.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            cmd: self.cmd.clone(),
            cols,
            rows,
            pid: self.pid,
            alive: exit_code.is_none(),
            exit_code,
            created_at: self.created_at,
            session: self.session.clone(),
            tui: self.tui,
            clients: self.clients.load(Ordering::Relaxed),
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Text from terminal bytes: without CSI/OSC/ESC sequences and carriage returns, line breaks are kept.
pub fn strip_ansi(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            match it.next() {
                Some('[') => { while let Some(&n) = it.peek() { it.next(); if ('@'..='~').contains(&n) { break } } }
                Some(']') => { while let Some(n) = it.next() { if n == '\x07' { break } if n == '\x1b' { if it.peek() == Some(&'\\') { it.next(); } break } } }
                Some(_) | None => {}
            }
            continue;
        }
        if c == '\r' || (c.is_control() && c != '\n' && c != '\t') { continue }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod strip_tests {
    #[test]
    fn strips_sequences() {
        assert_eq!(super::strip_ansi(b"a\x1b[31mb\x1b[0m\r\n\x1b]8;;https://x\x07link\x1b]8;;\x07 c"), "ab\nlink c");
        assert_eq!(super::strip_ansi(b"\x1b[?1049h\x1b[H\x1b[2Jhttps://accounts.google.com/o/oauth2/auth?x=1\n"), "https://accounts.google.com/o/oauth2/auth?x=1\n");
    }
}
