//! PTY backend trait and implementations (`HeadlessPtyBackend` & `PortablePtyBackend`).

use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalSignal {
    Interrupt,
    Kill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtySpawnConfig {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub rows: u16,
    pub cols: u16,
}

impl PtySpawnConfig {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            rows: crate::DEFAULT_TERMINAL_ROWS,
            cols: crate::DEFAULT_TERMINAL_COLS,
        }
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }

    pub fn with_size(mut self, rows: u16, cols: u16) -> Self {
        self.rows = rows.max(1);
        self.cols = cols.max(1);
        self
    }
}

pub trait PtyBackend: Send + Sync {
    fn write_input(&mut self, data: &[u8]) -> Result<(), String>;
    fn read_output(&mut self) -> Vec<u8>;
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String>;
    fn send_signal(&mut self, sig: TerminalSignal) -> Result<(), String>;
    fn try_wait(&mut self) -> Option<i32>;
    fn is_alive(&self) -> bool;
}

#[derive(Debug)]
struct HeadlessPtyState {
    output_queue: VecDeque<u8>,
    written_input: Vec<u8>,
    signals: Vec<TerminalSignal>,
    rows: u16,
    cols: u16,
    echo: bool,
    alive: bool,
    exit_code: Option<i32>,
}

/// Deterministic in-memory PTY backend for unit/CI tests without OS PTY handles.
#[derive(Debug, Clone)]
pub struct HeadlessPtyBackend {
    state: Arc<Mutex<HeadlessPtyState>>,
}

impl Default for HeadlessPtyBackend {
    fn default() -> Self {
        Self::new(crate::DEFAULT_TERMINAL_ROWS, crate::DEFAULT_TERMINAL_COLS)
    }
}

impl HeadlessPtyBackend {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            state: Arc::new(Mutex::new(HeadlessPtyState {
                output_queue: VecDeque::new(),
                written_input: Vec::new(),
                signals: Vec::new(),
                rows: rows.max(1),
                cols: cols.max(1),
                echo: false,
                alive: true,
                exit_code: None,
            })),
        }
    }

    pub fn with_echo(self, echo: bool) -> Self {
        self.state.lock().echo = echo;
        self
    }

    pub fn push_output(&self, data: &[u8]) {
        let mut guard = self.state.lock();
        guard.output_queue.extend(data);
    }

    pub fn push_output_str(&self, text: &str) {
        self.push_output(text.as_bytes());
    }

    pub fn simulate_exit(&self, code: i32) {
        let mut guard = self.state.lock();
        guard.alive = false;
        guard.exit_code = Some(code);
    }

    pub fn written_input(&self) -> Vec<u8> {
        self.state.lock().written_input.clone()
    }

    pub fn take_written_input(&self) -> Vec<u8> {
        let mut guard = self.state.lock();
        std::mem::take(&mut guard.written_input)
    }

    pub fn signals(&self) -> Vec<TerminalSignal> {
        self.state.lock().signals.clone()
    }

    pub fn size(&self) -> (u16, u16) {
        let guard = self.state.lock();
        (guard.rows, guard.cols)
    }
}

impl PtyBackend for HeadlessPtyBackend {
    fn write_input(&mut self, data: &[u8]) -> Result<(), String> {
        let mut guard = self.state.lock();
        if !guard.alive {
            return Err("Headless PTY process has exited".to_string());
        }
        guard.written_input.extend_from_slice(data);
        if guard.echo {
            guard.output_queue.extend(data);
        }
        Ok(())
    }

    fn read_output(&mut self) -> Vec<u8> {
        let mut guard = self.state.lock();
        guard.output_queue.drain(..).collect()
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        let mut guard = self.state.lock();
        guard.rows = rows.max(1);
        guard.cols = cols.max(1);
        Ok(())
    }

    fn send_signal(&mut self, sig: TerminalSignal) -> Result<(), String> {
        let mut guard = self.state.lock();
        guard.signals.push(sig);
        match sig {
            TerminalSignal::Interrupt => {
                if !guard.alive {
                    return Err("Headless PTY process has exited".to_string());
                }
                guard.written_input.push(0x03);
                if guard.echo {
                    guard.output_queue.extend(b"^C\r\n");
                }
                Ok(())
            }
            TerminalSignal::Kill => {
                if guard.alive {
                    guard.alive = false;
                    if guard.exit_code.is_none() {
                        guard.exit_code = Some(-1);
                    }
                }
                Ok(())
            }
        }
    }

    fn try_wait(&mut self) -> Option<i32> {
        let guard = self.state.lock();
        if guard.alive {
            None
        } else {
            guard.exit_code
        }
    }

    fn is_alive(&self) -> bool {
        self.state.lock().alive
    }
}

/// Real OS PTY backend powered by `portable-pty` (ConPTY on Windows, openpty on Unix).
pub struct PortablePtyBackend {
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    child: Box<dyn Child + Send + Sync>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    output_buf: Arc<Mutex<Vec<u8>>>,
    reader_eof: Arc<AtomicBool>,
    exit_code: Option<i32>,
    alive: bool,
    rows: u16,
    cols: u16,
}

impl std::fmt::Debug for PortablePtyBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortablePtyBackend")
            .field("alive", &self.alive)
            .field("exit_code", &self.exit_code)
            .field("rows", &self.rows)
            .field("cols", &self.cols)
            .finish()
    }
}

impl PortablePtyBackend {
    pub fn spawn_shell(
        shell: Option<&str>,
        cwd: Option<&Path>,
        env: &BTreeMap<String, String>,
        rows: u16,
        cols: u16,
    ) -> Result<Self, String> {
        let resolved_shell = crate::resolve_shell_program(shell.unwrap_or(""));
        let mut config = PtySpawnConfig::new(resolved_shell).with_size(rows, cols);
        if let Some(dir) = cwd {
            config.cwd = Some(dir.to_path_buf());
        }
        config.env = env.clone();
        Self::spawn_command(config)
    }

    pub fn spawn_command(config: PtySpawnConfig) -> Result<Self, String> {
        let rows = config.rows.max(1);
        let cols = config.cols.max(1);
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| format!("Failed to open PTY: {}", err))?;

        let mut cmd = CommandBuilder::new(&config.program);
        for arg in &config.args {
            cmd.arg(arg);
        }

        if let Some(ref cwd) = config.cwd {
            let clean_cwd = crate::strip_windows_verbatim_prefix(cwd.clone());
            if clean_cwd.is_dir() {
                cmd.cwd(clean_cwd);
            }
        }

        if !config.env.contains_key("TERM") && std::env::var_os("TERM").is_none() {
            cmd.env("TERM", "xterm-256color");
        }
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|err| format!("Failed to spawn '{}' in PTY: {}", config.program, err))?;

        // Drop slave immediately so master sees EOF when child exits.
        drop(pair.slave);

        let killer = child.clone_killer();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|err| format!("Failed to clone PTY reader: {}", err))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|err| format!("Failed to take PTY writer: {}", err))?;

        let output_buf = Arc::new(Mutex::new(Vec::new()));
        let reader_eof = Arc::new(AtomicBool::new(false));
        let output_buf_clone = Arc::clone(&output_buf);
        let reader_eof_clone = Arc::clone(&reader_eof);

        std::thread::Builder::new()
            .name("rustcode-pty-reader".to_string())
            .spawn(move || {
                let mut chunk = [0u8; 8192];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) => {
                            reader_eof_clone.store(true, Ordering::Release);
                            break;
                        }
                        Ok(n) => {
                            let mut guard = output_buf_clone.lock();
                            guard.extend_from_slice(&chunk[..n]);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => {
                            reader_eof_clone.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            })
            .map_err(|err| format!("Failed to spawn PTY reader thread: {}", err))?;

        Ok(Self {
            writer: Mutex::new(Some(writer)),
            master: Mutex::new(Some(pair.master)),
            child,
            killer,
            output_buf,
            reader_eof,
            exit_code: None,
            alive: true,
            rows,
            cols,
        })
    }

    pub fn size(&self) -> (u16, u16) {
        (self.rows, self.cols)
    }

    pub fn reader_at_eof(&self) -> bool {
        self.reader_eof.load(Ordering::Acquire)
    }
}

impl PtyBackend for PortablePtyBackend {
    fn write_input(&mut self, data: &[u8]) -> Result<(), String> {
        if self.try_wait().is_some() || !self.alive {
            return Err("PTY child process has exited".to_string());
        }
        let mut writer_guard = self.writer.lock();
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| "PTY writer is closed".to_string())?;
        writer
            .write_all(data)
            .map_err(|err| format!("Failed to write to PTY: {}", err))?;
        writer
            .flush()
            .map_err(|err| format!("Failed to flush PTY writer: {}", err))?;
        Ok(())
    }

    fn read_output(&mut self) -> Vec<u8> {
        let mut guard = self.output_buf.lock();
        std::mem::take(&mut *guard)
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let master_guard = self.master.lock();
        let master = master_guard
            .as_ref()
            .ok_or_else(|| "PTY master is closed".to_string())?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| format!("Failed to resize PTY: {}", err))?;
        drop(master_guard);
        self.rows = rows;
        self.cols = cols;
        Ok(())
    }

    fn send_signal(&mut self, sig: TerminalSignal) -> Result<(), String> {
        match sig {
            TerminalSignal::Interrupt => self.write_input(&[0x03]),
            TerminalSignal::Kill => {
                let _ = self.killer.kill();
                let _ = self.child.kill();
                self.alive = false;
                if let Ok(Some(status)) = self.child.try_wait() {
                    self.exit_code = Some(status.exit_code() as i32);
                } else if self.exit_code.is_none() {
                    self.exit_code = Some(-1);
                }
                Ok(())
            }
        }
    }

    fn try_wait(&mut self) -> Option<i32> {
        if let Some(code) = self.exit_code {
            return Some(code);
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                let code = status.exit_code() as i32;
                self.exit_code = Some(code);
                self.alive = false;
                Some(code)
            }
            Ok(None) => None,
            Err(_) => None,
        }
    }

    fn is_alive(&self) -> bool {
        self.alive
    }
}

impl Drop for PortablePtyBackend {
    fn drop(&mut self) {
        if self.alive {
            let _ = self.killer.kill();
            let _ = self.child.kill();
        }
        drop(self.writer.lock().take());
        drop(self.master.lock().take());
        for _ in 0..10 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
