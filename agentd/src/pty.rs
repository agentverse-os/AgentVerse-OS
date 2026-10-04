//! PTY-демон: терминалы принадлежат agentd, а не вкладке и не IDE (ADR-006, урок Orca). Клиенты подключаются и
//! отключаются сколько угодно раз; процесс живёт, пока жив сам или пока его не убили явно. Вывод раздаётся всем
//! подключённым через broadcast, новому клиенту сначала проигрывается ограниченный хвост (scrollback).
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

/// Хвост вывода, который получает подключившийся клиент.
pub const SCROLLBACK_MAX: usize = 256 * 1024;
const BROADCAST_CAPACITY: usize = 4096;

pub struct PtyOpts {
    pub cmd: Vec<String>,
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
    pub title: String,
    pub env: HashMap<String, String>,
    /// Shell карточки сессии: PTY принадлежит сессии агента и убирается вместе с ней
    pub session: Option<String>,
    /// Терминальный режим (как в CloudCLI): родной TUI агента на той же беседе; один такой на сессию
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
    /// Scrollback и раздача под одним замком: клиент, подписавшийся под ним, не пропустит и не задвоит байты.
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
        // Блокирующее чтение мастера — в отдельном потоке; EOF означает, что процесс закрыл терминал.
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
            // send_replace, а не send: send без единого получателя (никто не подключён) не обновляет значение
            me.exit.send_replace(Some(code));
        })?;
        Ok(s)
    }

    /// Снимок хвоста и подписка на дальнейший вывод — атомарно относительно потока чтения.
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

    /// SIGHUP всей группе процессов терминала (оболочка — лидер сессии, дети — в её группе), через 5 с — SIGKILL тем,
    /// кто не вышел. Так уходит и агент, запущенный внутри оболочки, а не только сама оболочка.
    pub fn kill(&self) -> Result<()> {
        let pid = self.pid.context("нет pid")? as i32;
        // SAFETY: обычный kill(2); отрицательный pid — вся группа.
        if unsafe { libc::kill(-pid, libc::SIGHUP) } != 0 {
            return Err(std::io::Error::last_os_error()).context("kill(SIGHUP)");
        }
        let exit = self.exit.subscribe();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            if exit.borrow().is_none() {
                // SAFETY: см. выше.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
        });
        Ok(())
    }

    pub fn exit_code(&self) -> Option<i32> {
        *self.exit.borrow()
    }

    /// Дождаться выхода процесса (после kill) не дольше `timeout`; возвращает, вышел ли.
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
