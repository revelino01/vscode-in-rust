//! `rustcode-terminal` — Integrated Terminal Management.
//!
//! Manages terminal sessions, PTY backends (`HeadlessPtyBackend`, `PortablePtyBackend`),
//! `alacritty_terminal` VT100/xterm grid emulation, and split/tab views.

pub mod grid;
pub mod pty;

pub use grid::{TerminalCell, TerminalCellGrid, TerminalGridSnapshot};
pub use pty::{
    HeadlessPtyBackend, PortablePtyBackend, PtyBackend, PtySpawnConfig, TerminalSignal,
};

use parking_lot::Mutex as FastMutex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_MAX_SCROLLBACK: usize = 2000;
pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 5000;
pub const DEFAULT_TERMINAL_ROWS: u16 = 24;
pub const DEFAULT_TERMINAL_COLS: u16 = 80;

fn default_max_scrollback() -> usize {
    DEFAULT_MAX_SCROLLBACK
}

fn default_command_timeout_ms() -> u64 {
    DEFAULT_COMMAND_TIMEOUT_MS
}

fn default_rows() -> u16 {
    DEFAULT_TERMINAL_ROWS
}

fn default_cols() -> u16 {
    DEFAULT_TERMINAL_COLS
}

/// Returns the default interactive shell executable for the current operating system.
pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC")
            .ok()
            .filter(|s| !s.trim().is_empty() && Path::new(s).is_file())
            .unwrap_or_else(|| "cmd.exe".to_string())
    }
    #[cfg(not(windows))]
    {
        "/bin/sh".to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellKind {
    Cmd,
    PowerShell,
    Posix,
}

fn shell_kind(shell_prog: &str) -> ShellKind {
    let file_name = shell_prog
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell_prog)
        .to_ascii_lowercase();
    match file_name.as_str() {
        "cmd" | "cmd.exe" => ShellKind::Cmd,
        "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe" => ShellKind::PowerShell,
        _ => ShellKind::Posix,
    }
}

fn find_in_path(prog: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    #[cfg(windows)]
    {
        let has_ext = Path::new(prog).extension().is_some();
        let pathext_raw =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let exts: Vec<&str> = pathext_raw
            .split(';')
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .collect();
        for dir in std::env::split_paths(&path_var) {
            if dir.join(prog).is_file() {
                return true;
            }
            if !has_ext {
                for ext in &exts {
                    if dir.join(format!("{}{}", prog, ext)).is_file() {
                        return true;
                    }
                }
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        for dir in std::env::split_paths(&path_var) {
            if dir.join(prog).is_file() {
                return true;
            }
        }
        false
    }
}

pub(crate) fn resolve_shell_program(shell: &str) -> String {
    #[cfg(windows)]
    {
        if !shell.is_empty() {
            if shell.contains('/') || shell.contains('\\') {
                if Path::new(shell).is_file() {
                    return shell.to_string();
                }
            } else if find_in_path(shell) {
                return shell.to_string();
            }
        }
        default_shell()
    }
    #[cfg(not(windows))]
    {
        if !shell.is_empty() {
            if shell.contains('/') {
                if Path::new(shell).is_file() {
                    return shell.to_string();
                }
            } else if find_in_path(shell) {
                return shell.to_string();
            }
        } else if let Ok(env_shell) = std::env::var("SHELL") {
            let trimmed = env_shell.trim();
            if !trimmed.is_empty() && Path::new(trimmed).is_file() {
                return trimmed.to_string();
            }
        }
        if Path::new("/bin/sh").exists() {
            "/bin/sh".to_string()
        } else if Path::new("/system/bin/sh").exists() {
            "/system/bin/sh".to_string()
        } else {
            "sh".to_string()
        }
    }
}

#[cfg(windows)]
pub(crate) fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    use std::path::Prefix;
    let mut comps = path.components();
    let Some(Component::Prefix(prefix_comp)) = comps.next() else {
        return path;
    };
    let rest: PathBuf = comps.collect();
    match prefix_comp.kind() {
        Prefix::VerbatimDisk(drive) => {
            let mut out =
                std::ffi::OsString::from(format!("{}:", (drive as char).to_ascii_uppercase()));
            out.push(rest.as_os_str());
            PathBuf::from(out)
        }
        Prefix::VerbatimUNC(server, share) => {
            let mut out = std::ffi::OsString::from(r"\\");
            out.push(server);
            out.push(r"\");
            out.push(share);
            out.push(rest.as_os_str());
            PathBuf::from(out)
        }
        _ => path,
    }
}

#[cfg(not(windows))]
pub(crate) fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

fn build_shell_command(shell_prog: &str, trimmed: &str) -> Command {
    let mut command = Command::new(shell_prog);
    match shell_kind(shell_prog) {
        ShellKind::Cmd => {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                // cmd.exe stops parsing /C at the first CR/LF, so join multiline commands with ` & `.
                let cmd_line = if trimmed.contains('\n') || trimmed.contains('\r') {
                    trimmed
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .join(" & ")
                } else {
                    trimmed.to_string()
                };
                // /D disables AutoRun registry commands; /S strips exactly the single
                // outermost quote pair we wrap around `cmd_line`, preserving internal quotes.
                command
                    .arg("/D")
                    .arg("/S")
                    .arg("/C")
                    .raw_arg(format!("\"{}\"", cmd_line));
            }
            #[cfg(not(windows))]
            {
                command.arg("/C").arg(trimmed);
            }
        }
        ShellKind::PowerShell => {
            command
                .arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-Command")
                .arg(trimmed);
        }
        ShellKind::Posix => {
            command.arg("-c").arg(trimmed);
        }
    }
    command
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()))
        .map(PathBuf::from)
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE").filter(|v| !v.is_empty())?;
            let path = std::env::var_os("HOMEPATH").filter(|v| !v.is_empty())?;
            let mut combined = drive;
            combined.push(path);
            Some(PathBuf::from(combined))
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandExecutionResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub exited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalTab {
    pub id: String,
    pub pane_ids: Vec<String>,
    pub active_pane_index: usize,
}

impl TerminalTab {
    pub fn active_pane_id(&self) -> Option<&str> {
        self.pane_ids.get(self.active_pane_index).map(|s| s.as_str())
    }
}

#[derive(Serialize, Deserialize)]
pub struct TerminalInstance {
    pub id: String,
    pub title: String,
    pub shell: String,
    pub cwd: PathBuf,
    pub lines: Vec<String>,
    #[serde(default)]
    pub input_buffer: String,
    /// UTF-8 character offset within `input_buffer` (`0..=input_buffer.chars().count()`).
    #[serde(default)]
    pub cursor_pos: usize,
    #[serde(default)]
    pub command_history: Vec<String>,
    #[serde(default)]
    pub history_index: Option<usize>,
    #[serde(default)]
    draft_buffer: String,
    #[serde(default = "default_max_scrollback")]
    pub max_scrollback: usize,
    #[serde(default)]
    pub last_exit_code: Option<i32>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub exited: bool,
    #[serde(default = "default_command_timeout_ms")]
    pub command_timeout_ms: u64,
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default = "default_rows")]
    pub rows: u16,
    #[serde(default = "default_cols")]
    pub cols: u16,
    #[serde(skip)]
    grid: TerminalCellGrid,
    #[serde(skip)]
    pty: Option<Arc<FastMutex<Box<dyn PtyBackend>>>>,
}

impl std::fmt::Debug for TerminalInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalInstance")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("shell", &self.shell)
            .field("cwd", &self.cwd)
            .field("lines_len", &self.lines.len())
            .field("exited", &self.exited)
            .field("exit_code", &self.exit_code)
            .field("rows", &self.rows)
            .field("cols", &self.cols)
            .field("has_pty", &self.pty.is_some())
            .finish()
    }
}

impl Clone for TerminalInstance {
    fn clone(&self) -> Self {
        let mut grid = TerminalCellGrid::new(self.rows, self.cols, self.max_scrollback);
        if !self.lines.is_empty() {
            let joined = self.lines.join("\r\n");
            grid.feed_bytes(joined.as_bytes());
        }
        Self {
            id: self.id.clone(),
            title: self.title.clone(),
            shell: self.shell.clone(),
            cwd: self.cwd.clone(),
            lines: self.lines.clone(),
            input_buffer: self.input_buffer.clone(),
            cursor_pos: self.cursor_pos,
            command_history: self.command_history.clone(),
            history_index: self.history_index,
            draft_buffer: self.draft_buffer.clone(),
            max_scrollback: self.max_scrollback,
            last_exit_code: self.last_exit_code,
            exit_code: self.exit_code,
            exited: self.exited,
            command_timeout_ms: self.command_timeout_ms,
            env_vars: self.env_vars.clone(),
            rows: self.rows,
            cols: self.cols,
            grid,
            pty: self.pty.clone(),
        }
    }
}

fn byte_index_at_char(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

fn lexical_normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && !out.has_root() {
                    out.push("..");
                }
            }
            Component::Normal(part) => out.push(part),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

fn strip_matching_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn has_shell_meta(cmd: &str) -> bool {
    cmd.contains('&')
        || cmd.contains("||")
        || cmd.contains(';')
        || cmd.contains('|')
        || cmd.contains('>')
        || cmd.contains('<')
        || cmd.contains('`')
        || cmd.contains('$')
        || cmd.contains('\n')
}

impl TerminalInstance {
    pub fn new(id: &str, title: &str, shell: &str, cwd: PathBuf) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            shell: shell.to_string(),
            cwd,
            lines: Vec::new(),
            input_buffer: String::new(),
            cursor_pos: 0,
            command_history: Vec::new(),
            history_index: None,
            draft_buffer: String::new(),
            max_scrollback: DEFAULT_MAX_SCROLLBACK,
            last_exit_code: None,
            exit_code: None,
            exited: false,
            command_timeout_ms: DEFAULT_COMMAND_TIMEOUT_MS,
            env_vars: BTreeMap::new(),
            rows: DEFAULT_TERMINAL_ROWS,
            cols: DEFAULT_TERMINAL_COLS,
            grid: TerminalCellGrid::new(
                DEFAULT_TERMINAL_ROWS,
                DEFAULT_TERMINAL_COLS,
                DEFAULT_MAX_SCROLLBACK,
            ),
            pty: None,
        }
    }

    pub fn new_pty(
        id: &str,
        title: &str,
        shell: &str,
        cwd: PathBuf,
        rows: u16,
        cols: u16,
    ) -> Result<Self, String> {
        let mut inst = Self::new(id, title, shell, cwd);
        inst.spawn_pty_shell(rows, cols)?;
        Ok(inst)
    }

    pub fn with_pty(mut self, backend: Box<dyn PtyBackend>) -> Self {
        self.attach_pty(backend);
        self
    }

    pub fn attach_pty(&mut self, mut backend: Box<dyn PtyBackend>) {
        let _ = backend.resize(self.rows.max(1), self.cols.max(1));
        self.pty = Some(Arc::new(FastMutex::new(backend)));
        self.exited = false;
        self.exit_code = None;
        self.last_exit_code = None;
    }

    pub fn spawn_pty_shell(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let shell_opt = if self.shell.trim().is_empty() {
            None
        } else {
            Some(self.shell.as_str())
        };
        let backend =
            PortablePtyBackend::spawn_shell(shell_opt, Some(&self.cwd), &self.env_vars, rows, cols)?;
        self.rows = rows;
        self.cols = cols;
        self.grid.resize(rows, cols);
        self.attach_pty(Box::new(backend));
        Ok(())
    }

    pub fn spawn_pty_command(&mut self, config: PtySpawnConfig) -> Result<(), String> {
        let rows = config.rows.max(1);
        let cols = config.cols.max(1);
        let backend = PortablePtyBackend::spawn_command(config)?;
        self.rows = rows;
        self.cols = cols;
        self.grid.resize(rows, cols);
        self.attach_pty(Box::new(backend));
        Ok(())
    }

    pub fn has_pty(&self) -> bool {
        self.pty.is_some()
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    pub fn grid(&self) -> &TerminalCellGrid {
        &self.grid
    }

    pub fn grid_mut(&mut self) -> &mut TerminalCellGrid {
        &mut self.grid
    }

    pub fn grid_snapshot(&self) -> TerminalGridSnapshot {
        self.grid.snapshot()
    }

    pub fn scroll_display(&mut self, delta: i32) {
        self.grid.scroll_display(delta);
    }

    pub fn feed_vt_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.grid.feed_bytes(bytes);
        self.sync_lines_from_grid();
    }

    pub fn send_pty_input(&mut self, data: &[u8]) -> Result<(), String> {
        if self.exited {
            return Err("Terminal process has exited".to_string());
        }
        let Some(pty) = &self.pty else {
            return Err("No PTY backend attached".to_string());
        };
        pty.lock().write_input(data)
    }

    pub fn send_pty_str(&mut self, text: &str) -> Result<(), String> {
        self.send_pty_input(text.as_bytes())
    }

    pub fn poll_pty(&mut self) -> bool {
        let Some(pty_arc) = self.pty.clone() else {
            return false;
        };

        let mut changed = false;
        let bytes = {
            let mut guard = pty_arc.lock();
            guard.read_output()
        };
        if !bytes.is_empty() {
            self.grid.feed_bytes(&bytes);
            let pty_writes = self.grid.drain_pty_writes();
            if !pty_writes.is_empty() {
                let mut guard = pty_arc.lock();
                for reply in pty_writes {
                    let _ = guard.write_input(reply.as_bytes());
                }
            }
            self.sync_lines_from_grid();
            changed = true;
        }

        let exit = {
            let mut guard = pty_arc.lock();
            guard.try_wait()
        };
        if let Some(code) = exit {
            let tail_bytes = {
                let mut guard = pty_arc.lock();
                guard.read_output()
            };
            if !tail_bytes.is_empty() {
                self.grid.feed_bytes(&tail_bytes);
                self.sync_lines_from_grid();
                changed = true;
            }
            if !self.exited || self.exit_code != Some(code) {
                self.exited = true;
                self.exit_code = Some(code);
                self.last_exit_code = Some(code);
                changed = true;
            }
        }

        changed
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        self.rows = rows;
        self.cols = cols;
        self.grid.resize(rows, cols);
        if let Some(pty) = &self.pty {
            pty.lock().resize(rows, cols)?;
        }
        Ok(())
    }

    pub fn send_ctrl_c(&mut self) -> Result<(), String> {
        if self.exited {
            return Err("Terminal process has exited".to_string());
        }
        if let Some(pty) = &self.pty {
            pty.lock().send_signal(TerminalSignal::Interrupt)
        } else {
            let line = format!("{}{}^C", self.prompt_prefix(), self.input_buffer);
            self.lines.push(line);
            self.trim_scrollback();
            self.clear_input();
            Ok(())
        }
    }

    pub fn send_signal(&mut self, sig: TerminalSignal) -> Result<(), String> {
        if let Some(pty) = &self.pty {
            let res = {
                let mut guard = pty.lock();
                guard.send_signal(sig)
            };
            if sig == TerminalSignal::Kill {
                let code = pty.lock().try_wait().unwrap_or(-1);
                self.exited = true;
                self.exit_code = Some(code);
                self.last_exit_code = Some(code);
            }
            res
        } else {
            match sig {
                TerminalSignal::Interrupt => self.send_ctrl_c(),
                TerminalSignal::Kill => {
                    self.exited = true;
                    self.exit_code = Some(-1);
                    self.last_exit_code = Some(-1);
                    Ok(())
                }
            }
        }
    }

    fn sync_lines_from_grid(&mut self) {
        self.lines = self.grid.all_non_empty_tail_trimmed_lines();
        self.trim_scrollback();
    }

    pub fn write_output(&mut self, text: &str) {
        for line in text.split('\n') {
            self.lines.push(line.to_string());
        }
        self.trim_scrollback();
        if self.pty.is_none() {
            let normalized = text.replace("\r\n", "\n").replace('\n', "\r\n");
            self.grid.feed_bytes(normalized.as_bytes());
            self.grid.feed_bytes(b"\r\n");
        }
    }

    pub fn append_output_block(&mut self, text: &str) {
        let trimmed = text.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return;
        }
        for line in trimmed.split('\n') {
            self.lines.push(line.trim_end_matches('\r').to_string());
        }
        self.trim_scrollback();
        if self.pty.is_none() {
            let normalized = trimmed.replace("\r\n", "\n").replace('\n', "\r\n");
            self.grid.feed_bytes(normalized.as_bytes());
            self.grid.feed_bytes(b"\r\n");
        }
    }

    pub fn trim_scrollback(&mut self) {
        if self.max_scrollback > 0 && self.lines.len() > self.max_scrollback {
            let excess = self.lines.len() - self.max_scrollback;
            self.lines.drain(0..excess);
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.grid.clear();
    }

    pub fn set_max_scrollback(&mut self, max_lines: usize) {
        self.max_scrollback = max_lines;
        self.grid.set_max_scrollback(max_lines);
        self.trim_scrollback();
    }

    pub fn set_command_timeout(&mut self, timeout: Duration) {
        self.command_timeout_ms = timeout.as_millis().max(1) as u64;
    }

    pub fn char_len(&self) -> usize {
        self.input_buffer.chars().count()
    }

    pub fn input_before_cursor(&self) -> &str {
        let byte_idx = byte_index_at_char(&self.input_buffer, self.cursor_pos);
        &self.input_buffer[..byte_idx]
    }

    pub fn insert_char(&mut self, ch: char) {
        if self.exited || ch == '\0' || ch == '\r' || ch == '\n' {
            return;
        }
        let byte_idx = byte_index_at_char(&self.input_buffer, self.cursor_pos);
        self.input_buffer.insert(byte_idx, ch);
        self.cursor_pos += 1;
        self.history_index = None;
    }

    pub fn insert_str(&mut self, s: &str) {
        if self.exited {
            return;
        }
        let filtered: String = s
            .chars()
            .filter(|&c| c != '\0' && c != '\r' && c != '\n')
            .collect();
        if filtered.is_empty() {
            return;
        }
        let count = filtered.chars().count();
        let byte_idx = byte_index_at_char(&self.input_buffer, self.cursor_pos);
        self.input_buffer.insert_str(byte_idx, &filtered);
        self.cursor_pos += count;
        self.history_index = None;
    }

    pub fn backspace(&mut self) -> bool {
        if self.exited || self.cursor_pos == 0 {
            return false;
        }
        let start = byte_index_at_char(&self.input_buffer, self.cursor_pos - 1);
        let end = byte_index_at_char(&self.input_buffer, self.cursor_pos);
        self.input_buffer.replace_range(start..end, "");
        self.cursor_pos -= 1;
        self.history_index = None;
        true
    }

    pub fn delete(&mut self) -> bool {
        let len = self.char_len();
        if self.exited || self.cursor_pos >= len {
            return false;
        }
        let start = byte_index_at_char(&self.input_buffer, self.cursor_pos);
        let end = byte_index_at_char(&self.input_buffer, self.cursor_pos + 1);
        self.input_buffer.replace_range(start..end, "");
        self.history_index = None;
        true
    }

    pub fn move_cursor_left(&mut self) -> bool {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            true
        } else {
            false
        }
    }

    pub fn move_cursor_right(&mut self) -> bool {
        if self.cursor_pos < self.char_len() {
            self.cursor_pos += 1;
            true
        } else {
            false
        }
    }

    pub fn move_cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn move_cursor_end(&mut self) {
        self.cursor_pos = self.char_len();
    }

    pub fn clear_input(&mut self) {
        self.input_buffer.clear();
        self.cursor_pos = 0;
        self.history_index = None;
        self.draft_buffer.clear();
    }

    pub fn history_up(&mut self) -> bool {
        if self.command_history.is_empty() {
            return false;
        }
        let new_idx = match self.history_index {
            None => {
                self.draft_buffer = self.input_buffer.clone();
                self.command_history.len() - 1
            }
            Some(0) => return false,
            Some(idx) => idx - 1,
        };
        self.history_index = Some(new_idx);
        self.input_buffer = self.command_history[new_idx].clone();
        self.cursor_pos = self.char_len();
        true
    }

    pub fn history_down(&mut self) -> bool {
        match self.history_index {
            None => false,
            Some(idx) => {
                if idx + 1 < self.command_history.len() {
                    let new_idx = idx + 1;
                    self.history_index = Some(new_idx);
                    self.input_buffer = self.command_history[new_idx].clone();
                } else {
                    self.history_index = None;
                    self.input_buffer = std::mem::take(&mut self.draft_buffer);
                }
                self.cursor_pos = self.char_len();
                true
            }
        }
    }

    pub fn prompt_prefix(&self) -> String {
        format!("{}$ ", self.cwd.display())
    }

    pub fn submit_input(&mut self) -> CommandExecutionResult {
        if self.exited {
            self.clear_input();
            return CommandExecutionResult {
                exit_code: self.last_exit_code,
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                exited: true,
            };
        }

        let cmd = std::mem::take(&mut self.input_buffer);
        self.cursor_pos = 0;
        self.history_index = None;
        self.draft_buffer.clear();

        let trimmed = cmd.trim();
        if !trimmed.is_empty()
            && self.command_history.last().map(|s| s.as_str()) != Some(trimmed)
        {
            self.command_history.push(trimmed.to_string());
        }

        if self.pty.is_some() {
            let line_to_send = format!("{}\r", cmd);
            let _ = self.send_pty_input(line_to_send.as_bytes());
            let _ = self.poll_pty();
            return CommandExecutionResult {
                exit_code: self.exit_code,
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                exited: self.exited,
            };
        }

        let prompt_line = format!("{}{}", self.prompt_prefix(), cmd);
        self.lines.push(prompt_line.clone());
        self.trim_scrollback();
        self.grid.feed_bytes(prompt_line.as_bytes());
        self.grid.feed_bytes(b"\r\n");

        self.execute_command(trimmed)
    }

    fn effective_cwd(&self) -> PathBuf {
        if self.cwd.is_dir() {
            self.cwd.clone()
        } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        }
    }

    fn resolve_shell(&self) -> String {
        resolve_shell_program(&self.shell)
    }

    fn set_exit_code_fields(&mut self, code: Option<i32>) {
        self.last_exit_code = code;
        self.exit_code = code;
    }

    pub fn execute_command(&mut self, cmd: &str) -> CommandExecutionResult {
        let trimmed = cmd.trim();
        if self.exited {
            return CommandExecutionResult {
                exit_code: self.last_exit_code,
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                exited: true,
            };
        }

        if self.pty.is_some() {
            let line_to_send = format!("{}\r", cmd);
            if let Err(err) = self.send_pty_input(line_to_send.as_bytes()) {
                return CommandExecutionResult {
                    exit_code: self.exit_code,
                    stdout: String::new(),
                    stderr: err,
                    timed_out: false,
                    exited: self.exited,
                };
            }
            let _ = self.poll_pty();
            return CommandExecutionResult {
                exit_code: self.exit_code,
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                exited: self.exited,
            };
        }

        if trimmed.is_empty() {
            self.set_exit_code_fields(Some(0));
            return CommandExecutionResult {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                exited: false,
            };
        }

        if !has_shell_meta(trimmed) {
            if let Some(res) = self.try_execute_builtin(trimmed) {
                return res;
            }
        }

        let shell_prog = self.resolve_shell();
        let cwd = strip_windows_verbatim_prefix(self.effective_cwd());
        let mut command = build_shell_command(&shell_prog, trimmed);
        command
            .current_dir(&cwd)
            .envs(&self.env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(err) => {
                let msg = format!("Failed to spawn '{}': {}", shell_prog, err);
                self.append_output_block(&msg);
                self.set_exit_code_fields(Some(127));
                return CommandExecutionResult {
                    exit_code: Some(127),
                    stdout: String::new(),
                    stderr: msg,
                    timed_out: false,
                    exited: false,
                };
            }
        };

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let stdout_buf = Arc::new(Mutex::new(Vec::new()));
        let (stdout_done_tx, stdout_done_rx) = mpsc::channel();
        let stdout_buf_clone = Arc::clone(&stdout_buf);
        std::thread::spawn(move || {
            if let Some(mut out) = stdout_pipe {
                let mut chunk = [0u8; 4096];
                loop {
                    match out.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            if let Ok(mut guard) = stdout_buf_clone.lock() {
                                guard.extend_from_slice(&chunk[..n]);
                            }
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            }
            let _ = stdout_done_tx.send(());
        });

        let stderr_buf = Arc::new(Mutex::new(Vec::new()));
        let (stderr_done_tx, stderr_done_rx) = mpsc::channel();
        let stderr_buf_clone = Arc::clone(&stderr_buf);
        std::thread::spawn(move || {
            if let Some(mut err) = stderr_pipe {
                let mut chunk = [0u8; 4096];
                loop {
                    match err.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            if let Ok(mut guard) = stderr_buf_clone.lock() {
                                guard.extend_from_slice(&chunk[..n]);
                            }
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            }
            let _ = stderr_done_tx.send(());
        });

        let timeout = Duration::from_millis(self.command_timeout_ms.max(1));
        let start = Instant::now();
        let mut exit_status = None;
        let mut timed_out = false;

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    exit_status = Some(status);
                    break;
                }
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        timed_out = true;
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }

        // Wait a bounded grace period for pipe EOF so normal output finishes completely,
        // while detaching immediately if a forked grandchild still holds the pipe open.
        let grace_ms = if timed_out { 25 } else { 200 };
        let grace_deadline = Instant::now() + Duration::from_millis(grace_ms);
        let _ = stdout_done_rx.recv_timeout(grace_deadline.saturating_duration_since(Instant::now()));
        let _ = stderr_done_rx.recv_timeout(grace_deadline.saturating_duration_since(Instant::now()));

        let stdout_bytes = stdout_buf.lock().map(|g| g.clone()).unwrap_or_default();
        let stderr_bytes = stderr_buf.lock().map(|g| g.clone()).unwrap_or_default();
        let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
        let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();

        self.append_output_block(&stdout);
        self.append_output_block(&stderr);

        if timed_out {
            let msg = format!("[Command timed out after {}ms]", self.command_timeout_ms);
            self.append_output_block(&msg);
            self.set_exit_code_fields(None);
            return CommandExecutionResult {
                exit_code: None,
                stdout,
                stderr,
                timed_out: true,
                exited: false,
            };
        }

        let code = exit_status.and_then(|s| s.code());
        self.set_exit_code_fields(code);
        CommandExecutionResult {
            exit_code: code,
            stdout,
            stderr,
            timed_out: false,
            exited: false,
        }
    }

    fn try_execute_builtin(&mut self, trimmed: &str) -> Option<CommandExecutionResult> {
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let verb = parts.next()?;
        let rest = parts.next().map(|s| s.trim()).unwrap_or("");

        match verb {
            "pwd" => {
                let resolved = if self.cwd == Path::new(".") {
                    std::env::current_dir().unwrap_or_else(|_| self.cwd.clone())
                } else {
                    self.cwd.clone()
                };
                let display = resolved.display().to_string();
                self.append_output_block(&display);
                self.set_exit_code_fields(Some(0));
                Some(CommandExecutionResult {
                    exit_code: Some(0),
                    stdout: format!("{}\n", display),
                    stderr: String::new(),
                    timed_out: false,
                    exited: false,
                })
            }
            "clear" => {
                self.clear();
                self.set_exit_code_fields(Some(0));
                Some(CommandExecutionResult {
                    exit_code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                    timed_out: false,
                    exited: false,
                })
            }
            "exit" => {
                let code = if rest.is_empty() {
                    0
                } else {
                    rest.parse::<i32>().unwrap_or(0)
                };
                self.exited = true;
                self.set_exit_code_fields(Some(code));
                self.write_output("[Process exited]");
                Some(CommandExecutionResult {
                    exit_code: Some(code),
                    stdout: String::new(),
                    stderr: String::new(),
                    timed_out: false,
                    exited: true,
                })
            }
            "cd" => {
                let target_raw = strip_matching_quotes(rest);
                let target_path = if target_raw.is_empty() || target_raw == "~" {
                    user_home_dir().unwrap_or_else(|| self.effective_cwd())
                } else if let Some(sub) = target_raw
                    .strip_prefix("~/")
                    .or_else(|| target_raw.strip_prefix("~\\"))
                {
                    user_home_dir()
                        .map(|h| h.join(sub))
                        .unwrap_or_else(|| PathBuf::from(target_raw))
                } else {
                    let candidate = Path::new(target_raw);
                    if candidate.is_absolute() {
                        candidate.to_path_buf()
                    } else {
                        self.effective_cwd().join(candidate)
                    }
                };

                let normalized = std::fs::canonicalize(&target_path)
                    .unwrap_or_else(|_| lexical_normalize_path(&target_path));
                if normalized.is_dir() {
                    self.cwd = normalized;
                    self.set_exit_code_fields(Some(0));
                    Some(CommandExecutionResult {
                        exit_code: Some(0),
                        stdout: String::new(),
                        stderr: String::new(),
                        timed_out: false,
                        exited: false,
                    })
                } else {
                    let msg = format!("cd: {}: No such file or directory", target_raw);
                    self.append_output_block(&msg);
                    self.set_exit_code_fields(Some(1));
                    Some(CommandExecutionResult {
                        exit_code: Some(1),
                        stdout: String::new(),
                        stderr: msg,
                        timed_out: false,
                        exited: false,
                    })
                }
            }
            "export" => {
                if let Some((k, v)) = rest.split_once('=') {
                    let key = k.trim();
                    let val = strip_matching_quotes(v.trim());
                    if !key.is_empty() {
                        self.env_vars.insert(key.to_string(), val.to_string());
                    }
                }
                self.set_exit_code_fields(Some(0));
                Some(CommandExecutionResult {
                    exit_code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                    timed_out: false,
                    exited: false,
                })
            }
            "unset" => {
                for key in rest.split_whitespace() {
                    self.env_vars.remove(key);
                }
                self.set_exit_code_fields(Some(0));
                Some(CommandExecutionResult {
                    exit_code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                    timed_out: false,
                    exited: false,
                })
            }
            _ => None,
        }
    }
}

pub struct TerminalManager {
    terminals: Vec<TerminalInstance>,
    active_index: Option<usize>,
    next_id: u32,
    tabs: Vec<TerminalTab>,
    active_tab_index: Option<usize>,
    next_tab_id: u32,
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            terminals: Vec::new(),
            active_index: None,
            next_id: 1,
            tabs: Vec::new(),
            active_tab_index: None,
            next_tab_id: 1,
        }
    }

    pub fn create_terminal(
        &mut self,
        title: Option<&str>,
        shell: Option<&str>,
        cwd: Option<PathBuf>,
    ) -> &TerminalInstance {
        let id = format!("term-{}", self.next_id);
        self.next_id += 1;

        let title = title.unwrap_or("Terminal");
        let default_sh = default_shell();
        let shell = shell.unwrap_or(&default_sh);
        let cwd = cwd.unwrap_or_else(|| PathBuf::from("."));

        let inst = TerminalInstance::new(&id, title, shell, cwd);
        self.terminals.push(inst);
        self.active_index = Some(self.terminals.len() - 1);

        let tab_id = format!("tab-{}", self.next_tab_id);
        self.next_tab_id += 1;
        self.tabs.push(TerminalTab {
            id: tab_id,
            pane_ids: vec![id],
            active_pane_index: 0,
        });
        self.active_tab_index = Some(self.tabs.len() - 1);

        self.terminals.last().unwrap()
    }

    pub fn create_pty_terminal(
        &mut self,
        title: Option<&str>,
        shell: Option<&str>,
        cwd: Option<PathBuf>,
        rows: u16,
        cols: u16,
    ) -> Result<&TerminalInstance, String> {
        let resolved_cwd = cwd.unwrap_or_else(|| PathBuf::from("."));
        let env = BTreeMap::new();
        let backend =
            PortablePtyBackend::spawn_shell(shell, Some(&resolved_cwd), &env, rows, cols)?;
        let rows = rows.max(1);
        let cols = cols.max(1);

        let id = format!("term-{}", self.next_id);
        self.next_id += 1;

        let title = title.unwrap_or("Terminal");
        let default_sh = default_shell();
        let shell_str = shell.unwrap_or(&default_sh);

        let mut inst = TerminalInstance::new(&id, title, shell_str, resolved_cwd);
        let _ = inst.resize(rows, cols);
        inst.attach_pty(Box::new(backend));

        self.terminals.push(inst);
        self.active_index = Some(self.terminals.len() - 1);

        let tab_id = format!("tab-{}", self.next_tab_id);
        self.next_tab_id += 1;
        self.tabs.push(TerminalTab {
            id: tab_id,
            pane_ids: vec![id],
            active_pane_index: 0,
        });
        self.active_tab_index = Some(self.tabs.len() - 1);

        Ok(self.terminals.last().unwrap())
    }

    pub fn poll_all_ptys(&mut self) -> Vec<String> {
        let mut updated = Vec::new();
        for term in &mut self.terminals {
            if term.poll_pty() {
                updated.push(term.id.clone());
            }
        }
        updated
    }

    fn sync_active_index_from_active_tab(&mut self) {
        self.active_index = self
            .active_tab_index
            .and_then(|ti| self.tabs.get(ti))
            .and_then(|tab| tab.active_pane_id())
            .and_then(|pid| self.terminals.iter().position(|t| t.id == pid));
    }

    pub fn close_terminal(&mut self, id: &str) -> bool {
        if let Some(pos) = self.terminals.iter().position(|t| t.id == id) {
            self.terminals.remove(pos);

            let mut removed_tab_idx = None;
            for (ti, tab) in self.tabs.iter_mut().enumerate() {
                if let Some(pane_pos) = tab.pane_ids.iter().position(|pid| pid == id) {
                    tab.pane_ids.remove(pane_pos);
                    if tab.pane_ids.is_empty() {
                        removed_tab_idx = Some(ti);
                    } else if tab.active_pane_index >= tab.pane_ids.len() {
                        tab.active_pane_index = tab.pane_ids.len() - 1;
                    } else if tab.active_pane_index > pane_pos {
                        tab.active_pane_index -= 1;
                    }
                    break;
                }
            }

            if let Some(ti) = removed_tab_idx {
                self.tabs.remove(ti);
                if self.tabs.is_empty() {
                    self.active_tab_index = None;
                } else if let Some(active_ti) = self.active_tab_index {
                    if active_ti >= self.tabs.len() {
                        self.active_tab_index = Some(self.tabs.len() - 1);
                    } else if active_ti > ti {
                        self.active_tab_index = Some(active_ti - 1);
                    }
                }
            }

            self.sync_active_index_from_active_tab();
            if self.active_index.is_none() && !self.terminals.is_empty() {
                self.active_index = Some(self.terminals.len() - 1);
            }
            true
        } else {
            false
        }
    }

    pub fn active_terminal(&self) -> Option<&TerminalInstance> {
        self.active_index.and_then(|idx| self.terminals.get(idx))
    }

    pub fn active_terminal_mut(&mut self) -> Option<&mut TerminalInstance> {
        self.active_index.and_then(|idx| self.terminals.get_mut(idx))
    }

    pub fn terminal(&self, id: &str) -> Option<&TerminalInstance> {
        self.terminals.iter().find(|t| t.id == id)
    }

    pub fn terminal_mut(&mut self, id: &str) -> Option<&mut TerminalInstance> {
        self.terminals.iter_mut().find(|t| t.id == id)
    }

    pub fn set_active_terminal(&mut self, id: &str) -> bool {
        let Some(pos) = self.terminals.iter().position(|t| t.id == id) else {
            return false;
        };
        self.active_index = Some(pos);
        for (ti, tab) in self.tabs.iter_mut().enumerate() {
            if let Some(pane_pos) = tab.pane_ids.iter().position(|pid| pid == id) {
                tab.active_pane_index = pane_pos;
                self.active_tab_index = Some(ti);
                break;
            }
        }
        true
    }

    pub fn terminals(&self) -> &[TerminalInstance] {
        &self.terminals
    }

    pub fn tabs(&self) -> &[TerminalTab] {
        &self.tabs
    }

    pub fn active_tab(&self) -> Option<&TerminalTab> {
        self.active_tab_index.and_then(|idx| self.tabs.get(idx))
    }

    pub fn active_tab_index(&self) -> Option<usize> {
        self.active_tab_index
    }

    pub fn set_active_tab(&mut self, tab_index: usize) -> bool {
        if tab_index >= self.tabs.len() {
            return false;
        }
        self.active_tab_index = Some(tab_index);
        self.sync_active_index_from_active_tab();
        true
    }

    pub fn close_tab(&mut self, tab_index: usize) -> bool {
        if tab_index >= self.tabs.len() {
            return false;
        }
        let pane_ids = self.tabs.remove(tab_index).pane_ids;
        self.terminals
            .retain(|t| !pane_ids.iter().any(|pid| pid == &t.id));

        if self.tabs.is_empty() {
            self.active_tab_index = None;
        } else if let Some(active_ti) = self.active_tab_index {
            if active_ti >= self.tabs.len() {
                self.active_tab_index = Some(self.tabs.len() - 1);
            } else if active_ti > tab_index {
                self.active_tab_index = Some(active_ti - 1);
            }
        }
        self.sync_active_index_from_active_tab();
        true
    }

    pub fn split_active_terminal(
        &mut self,
        title: Option<&str>,
        shell: Option<&str>,
        cwd: Option<PathBuf>,
    ) -> &TerminalInstance {
        let Some(tab_idx) = self.active_tab_index.filter(|&i| i < self.tabs.len()) else {
            return self.create_terminal(title, shell, cwd);
        };

        let inherited_shell = self
            .active_terminal()
            .map(|t| t.shell.clone())
            .unwrap_or_else(default_shell);
        let inherited_cwd = self
            .active_terminal()
            .map(|t| t.cwd.clone())
            .unwrap_or_else(|| PathBuf::from("."));

        let id = format!("term-{}", self.next_id);
        self.next_id += 1;

        let final_title = title.unwrap_or("Terminal");
        let final_shell = shell.unwrap_or(&inherited_shell);
        let final_cwd = cwd.unwrap_or(inherited_cwd);

        let inst = TerminalInstance::new(&id, final_title, final_shell, final_cwd);
        self.terminals.push(inst);
        self.active_index = Some(self.terminals.len() - 1);

        let tab = &mut self.tabs[tab_idx];
        let insert_idx = (tab.active_pane_index + 1).min(tab.pane_ids.len());
        tab.pane_ids.insert(insert_idx, id);
        tab.active_pane_index = insert_idx;

        self.terminals.last().unwrap()
    }

    pub fn set_active_pane(&mut self, pane_index: usize) -> bool {
        let Some(tab_idx) = self.active_tab_index else {
            return false;
        };
        let Some(tab) = self.tabs.get_mut(tab_idx) else {
            return false;
        };
        if pane_index >= tab.pane_ids.len() {
            return false;
        }
        tab.active_pane_index = pane_index;
        self.sync_active_index_from_active_tab();
        true
    }

    pub fn focus_next_pane(&mut self) -> bool {
        let Some(tab) = self.active_tab() else {
            return false;
        };
        if tab.pane_ids.is_empty() {
            return false;
        }
        let next = (tab.active_pane_index + 1) % tab.pane_ids.len();
        self.set_active_pane(next)
    }

    pub fn focus_prev_pane(&mut self) -> bool {
        let Some(tab) = self.active_tab() else {
            return false;
        };
        if tab.pane_ids.is_empty() {
            return false;
        }
        let prev = if tab.active_pane_index == 0 {
            tab.pane_ids.len() - 1
        } else {
            tab.active_pane_index - 1
        };
        self.set_active_pane(prev)
    }

    pub fn active_tab_terminals(&self) -> Vec<&TerminalInstance> {
        let Some(tab) = self.active_tab() else {
            return Vec::new();
        };
        tab.pane_ids
            .iter()
            .filter_map(|pid| self.terminal(pid))
            .collect()
    }

    pub fn active_pane_index(&self) -> Option<usize> {
        self.active_tab().map(|t| t.active_pane_index)
    }

    pub fn close_active_terminal(&mut self) -> Option<String> {
        let id = self.active_terminal()?.id.clone();
        if self.close_terminal(&id) {
            Some(id)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_manager() {
        let mut mgr = TerminalManager::new();
        let _t1 = mgr.create_terminal(Some("bash"), Some("/bin/bash"), None);
        assert_eq!(mgr.terminals().len(), 1);

        let t2_id = {
            let t2 = mgr.create_terminal(Some("zsh"), Some("/bin/zsh"), None);
            assert_eq!(t2.title, "zsh");
            t2.id.clone()
        };
        assert_eq!(mgr.terminals().len(), 2);

        mgr.close_terminal(&t2_id);
        assert_eq!(mgr.terminals().len(), 1);
        assert_eq!(mgr.active_terminal().unwrap().title, "bash");

        // Test closing preceding terminal preserves active terminal
        let t1_id = mgr.terminals()[0].id.clone();
        let _t3 = mgr.create_terminal(Some("fish"), Some("/bin/fish"), None);
        assert_eq!(mgr.active_terminal().unwrap().title, "fish");
        assert_eq!(mgr.terminals().len(), 2);

        // Close preceding terminal (t1)
        mgr.close_terminal(&t1_id);
        assert_eq!(mgr.terminals().len(), 1);
        assert_eq!(mgr.active_terminal().unwrap().title, "fish");
    }

    #[test]
    fn test_terminal_input_editing_and_utf8() {
        let mut term = TerminalInstance::new("t1", "sh", "/bin/sh", PathBuf::from("."));
        term.insert_str("echo 🚀");
        assert_eq!(term.input_buffer, "echo 🚀");
        assert_eq!(term.cursor_pos, 6);
        assert_eq!(term.input_before_cursor(), "echo 🚀");

        assert!(term.move_cursor_left());
        assert_eq!(term.cursor_pos, 5);
        term.insert_char('!');
        assert_eq!(term.input_buffer, "echo !🚀");
        assert_eq!(term.cursor_pos, 6);

        assert!(term.delete());
        assert_eq!(term.input_buffer, "echo !");
        assert!(term.backspace());
        assert_eq!(term.input_buffer, "echo ");

        term.move_cursor_home();
        assert_eq!(term.cursor_pos, 0);
        term.insert_char('#');
        assert_eq!(term.input_buffer, "#echo ");

        term.move_cursor_end();
        assert_eq!(term.cursor_pos, 6);
        term.clear_input();
        assert!(term.input_buffer.is_empty());
        assert_eq!(term.cursor_pos, 0);
    }

    #[test]
    fn test_terminal_command_history_and_draft_preservation() {
        let mut term = TerminalInstance::new("t1", "sh", "/bin/sh", PathBuf::from("."));
        term.insert_str("pwd");
        let _ = term.submit_input();
        term.insert_str("pwd");
        let _ = term.submit_input();
        // Consecutive duplicate should be deduplicated
        assert_eq!(term.command_history, vec!["pwd"]);

        term.insert_str("echo hi");
        let _ = term.submit_input();
        assert_eq!(term.command_history, vec!["pwd", "echo hi"]);

        term.insert_str("draft");
        assert!(term.history_up());
        assert_eq!(term.input_buffer, "echo hi");
        assert!(term.history_up());
        assert_eq!(term.input_buffer, "pwd");
        assert!(!term.history_up());

        assert!(term.history_down());
        assert_eq!(term.input_buffer, "echo hi");
        assert!(term.history_down());
        assert_eq!(term.input_buffer, "draft");
        assert!(!term.history_down());
    }

    #[test]
    fn test_terminal_builtins_cd_pwd_clear_exit() {
        let tmp = std::env::temp_dir().join(format!("rustcode_term_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let canonical_tmp = std::fs::canonicalize(&tmp).unwrap();

        let mut term = TerminalInstance::new("t1", "sh", "/bin/sh", std::env::current_dir().unwrap());
        let pwd1 = term.execute_command("pwd");
        assert_eq!(pwd1.exit_code, Some(0));

        let cd_ok = term.execute_command(&format!("cd \"{}\"", tmp.display()));
        assert_eq!(cd_ok.exit_code, Some(0));
        assert_eq!(term.cwd, canonical_tmp);

        let pwd2 = term.execute_command("pwd");
        assert!(pwd2.stdout.contains(&canonical_tmp.display().to_string()));

        let cd_bad = term.execute_command("cd /nonexistent_rustcode_dir_xyz");
        assert_eq!(cd_bad.exit_code, Some(1));
        assert_eq!(term.cwd, canonical_tmp);

        let clear_res = term.execute_command("clear");
        assert_eq!(clear_res.exit_code, Some(0));
        assert!(term.lines.is_empty());

        let exit_res = term.execute_command("exit 42");
        assert!(exit_res.exited);
        assert_eq!(exit_res.exit_code, Some(42));
        assert!(term.exited);
        assert_eq!(term.last_exit_code, Some(42));
        assert_eq!(term.exit_code, Some(42));
        assert!(term.lines.iter().any(|l| l.contains("[Process exited]")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_terminal_pure_rust_command_execution_and_timeout() {
        #[cfg(windows)]
        let shell = default_shell();
        #[cfg(not(windows))]
        let shell = "/bin/sh".to_string();
        let mut term = TerminalInstance::new("t1", "sh", &shell, PathBuf::from("."));
        term.insert_str("echo hello_rustcode");
        let res = term.submit_input();
        assert_eq!(res.exit_code, Some(0));
        assert!(!res.timed_out);
        assert!(term.lines.iter().any(|l| l.contains("hello_rustcode")));

        term.set_command_timeout(Duration::from_millis(50));
        #[cfg(windows)]
        let sleep_cmd = "ping -n 4 127.0.0.1 >NUL";
        #[cfg(not(windows))]
        let sleep_cmd = "sleep 2";
        term.insert_str(sleep_cmd);
        let timeout_res = term.submit_input();
        assert!(timeout_res.timed_out);
        assert_eq!(timeout_res.exit_code, None);
        assert!(term.lines.iter().any(|l| l.contains("timed out")));
    }

    #[test]
    fn test_terminal_manager_tabs_splits_and_mut() {
        let mut mgr = TerminalManager::new();
        let t1_id = mgr
            .create_terminal(Some("sh1"), Some("/bin/sh"), Some(PathBuf::from("/tmp")))
            .id
            .clone();
        let t2_id = mgr
            .create_terminal(Some("sh2"), Some("/bin/sh"), Some(PathBuf::from("/")))
            .id
            .clone();
        assert_eq!(mgr.tabs().len(), 2);
        assert_eq!(mgr.active_tab_index(), Some(1));

        assert!(mgr.set_active_tab(0));
        assert_eq!(mgr.active_terminal().unwrap().id, t1_id);

        let split_id = mgr.split_active_terminal(Some("split"), None, None).id.clone();
        assert_eq!(mgr.tabs().len(), 2);
        assert_eq!(mgr.active_tab_terminals().len(), 2);
        assert_eq!(mgr.terminal(&split_id).unwrap().cwd, PathBuf::from("/tmp"));
        assert_eq!(mgr.active_terminal().unwrap().id, split_id);

        assert!(mgr.focus_prev_pane());
        assert_eq!(mgr.active_terminal().unwrap().id, t1_id);
        assert!(mgr.focus_next_pane());
        assert_eq!(mgr.active_terminal().unwrap().id, split_id);

        mgr.active_terminal_mut().unwrap().insert_str("echo split");
        assert_eq!(mgr.terminal_mut(&split_id).unwrap().input_buffer, "echo split");

        assert!(mgr.set_active_terminal(&t2_id));
        assert_eq!(mgr.active_tab_index(), Some(1));

        assert!(mgr.set_active_tab(0));
        assert!(mgr.close_terminal(&split_id));
        assert_eq!(mgr.tabs().len(), 2);
        assert_eq!(mgr.active_tab_terminals().len(), 1);
        assert_eq!(mgr.active_terminal().unwrap().id, t1_id);

        assert_eq!(mgr.close_active_terminal(), Some(t1_id));
        assert_eq!(mgr.tabs().len(), 1);
        assert_eq!(mgr.active_terminal().unwrap().id, t2_id);
    }

    #[test]
    fn test_terminal_timeout_forked_grandchild_no_pipe_hang() {
        #[cfg(windows)]
        let shell = default_shell();
        #[cfg(not(windows))]
        let shell = "/bin/sh".to_string();
        let mut term = TerminalInstance::new("t1", "sh", &shell, PathBuf::from("."));
        term.set_command_timeout(Duration::from_millis(50));

        // Do NOT redirect stdout away so the grandchild inherits both stdout & stderr pipes.
        #[cfg(windows)]
        let cmd = "ping -n 4 127.0.0.1 & echo after";
        #[cfg(not(windows))]
        let cmd = "sleep 2; echo after";

        let start = Instant::now();
        let res = term.execute_command(cmd);
        let elapsed = start.elapsed();

        assert!(res.timed_out);
        assert!(res.exit_code.is_none());
        assert!(
            elapsed < Duration::from_millis(500),
            "expected forked-grandchild timeout to return in <500ms, took {:?}",
            elapsed
        );
    }

    #[test]
    fn test_has_shell_meta_detects_dollar_ampersand_newline() {
        assert!(has_shell_meta("cd $HOME"));
        assert!(has_shell_meta("export A=$B"));
        assert!(has_shell_meta("sleep 1 &"));
        assert!(has_shell_meta("echo a\necho b"));
        assert!(!has_shell_meta("cd ~"));
        assert!(!has_shell_meta("pwd"));
    }

    #[test]
    fn test_shell_kind_detection_and_cd_then_exec_with_env() {
        assert_eq!(shell_kind("cmd"), ShellKind::Cmd);
        assert_eq!(shell_kind("cmd.exe"), ShellKind::Cmd);
        assert_eq!(shell_kind(r"C:\Windows\System32\cmd.exe"), ShellKind::Cmd);
        assert_eq!(shell_kind("powershell.exe"), ShellKind::PowerShell);
        assert_eq!(shell_kind("pwsh"), ShellKind::PowerShell);
        assert_eq!(shell_kind("/bin/sh"), ShellKind::Posix);
        assert_eq!(shell_kind(r"C:\Program Files\Git\bin\bash.exe"), ShellKind::Posix);

        let tmp = std::env::temp_dir().join(format!(
            "rustcode_term_cwd_exec_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let marker = tmp.join("marker_ok.txt");
        std::fs::write(&marker, "present").unwrap();

        let mut term = TerminalInstance::new("t1", "shell", &default_shell(), PathBuf::from("."));
        let cd_res = term.execute_command(&format!("cd \"{}\"", tmp.display()));
        assert_eq!(cd_res.exit_code, Some(0));

        // Verify spawned shell runs in the canonicalized cwd (even on Windows where canonicalize adds \\?\)
        #[cfg(windows)]
        let check_cmd = "if exist marker_ok.txt echo FOUND_MARKER";
        #[cfg(not(windows))]
        let check_cmd = "test -f marker_ok.txt && echo FOUND_MARKER";
        let exec_res = term.execute_command(check_cmd);
        assert_eq!(exec_res.exit_code, Some(0));
        assert!(
            exec_res.stdout.contains("FOUND_MARKER"),
            "expected FOUND_MARKER in stdout, got stdout={:?} stderr={:?}",
            exec_res.stdout,
            exec_res.stderr
        );

        // Verify export env var propagates to child shell
        assert_eq!(
            term.execute_command("export RUSTCODE_XPLAT_TEST=hello_env").exit_code,
            Some(0)
        );
        #[cfg(windows)]
        let env_cmd = "echo %RUSTCODE_XPLAT_TEST%";
        #[cfg(not(windows))]
        let env_cmd = "echo $RUSTCODE_XPLAT_TEST";
        let env_res = term.execute_command(env_cmd);
        assert_eq!(env_res.exit_code, Some(0));
        assert!(
            env_res.stdout.contains("hello_env"),
            "expected hello_env in stdout, got {:?}",
            env_res.stdout
        );

        // Verify multiline command execution (especially cmd.exe where raw /C truncates at \n)
        let multi = term.execute_command("echo line_one\necho line_two");
        assert_eq!(multi.exit_code, Some(0));
        assert!(
            multi.stdout.contains("line_one") && multi.stdout.contains("line_two"),
            "expected both lines in stdout, got {:?}",
            multi.stdout
        );

        // Verify missing configured shell (including pwsh when absent) falls back cleanly
        let mut missing_shell_term =
            TerminalInstance::new("t2", "missing", "nonexistent_rustcode_shell_xyz", PathBuf::from("."));
        let fallback_res = missing_shell_term.execute_command("echo fallback_ok");
        assert_eq!(fallback_res.exit_code, Some(0));
        assert!(fallback_res.stdout.contains("fallback_ok"));

        let mut pwsh_term = TerminalInstance::new("t3", "pwsh", "pwsh", PathBuf::from("."));
        let pwsh_fallback = pwsh_term.execute_command("echo pwsh_or_fallback_ok");
        assert_eq!(
            pwsh_fallback.exit_code,
            Some(0),
            "pwsh should execute if installed or fall back cleanly; stderr={:?}",
            pwsh_fallback.stderr
        );
        assert!(pwsh_fallback.stdout.contains("pwsh_or_fallback_ok"));

        // Verify non-ASCII directory cd + subprocess execution
        let unicode_dir = tmp.join("café_λ_dir");
        std::fs::create_dir_all(&unicode_dir).unwrap();
        std::fs::write(unicode_dir.join("u_marker.txt"), "ok").unwrap();
        assert_eq!(
            term.execute_command(&format!("cd \"{}\"", unicode_dir.display()))
                .exit_code,
            Some(0)
        );
        #[cfg(windows)]
        let u_cmd = "if exist u_marker.txt echo UNICODE_CWD_OK";
        #[cfg(not(windows))]
        let u_cmd = "test -f u_marker.txt && echo UNICODE_CWD_OK";
        let u_res = term.execute_command(u_cmd);
        assert_eq!(u_res.exit_code, Some(0));
        assert!(u_res.stdout.contains("UNICODE_CWD_OK"));

        #[cfg(windows)]
        {
            // Embedded quotes + pipe under cmd.exe /D /S /C
            let mut cmd_term = TerminalInstance::new("t_cmd", "cmd", "cmd.exe", PathBuf::from("."));
            let pipe_res = cmd_term.execute_command(r#"echo "a & b" | findstr a"#);
            assert_eq!(pipe_res.exit_code, Some(0));
            assert!(
                pipe_res.stdout.contains("a & b"),
                "expected quoted ampersand preserved across pipe, got {:?}",
                pipe_res.stdout
            );

            // Live PowerShell end-to-end execution (env propagation, cwd, multiline)
            if find_in_path("powershell.exe") {
                let mut ps_term =
                    TerminalInstance::new("t_ps", "powershell", "powershell.exe", PathBuf::from("."));
                ps_term.set_command_timeout(Duration::from_millis(10_000));
                assert_eq!(
                    ps_term
                        .execute_command(&format!("cd \"{}\"", tmp.display()))
                        .exit_code,
                    Some(0)
                );
                assert_eq!(
                    ps_term
                        .execute_command("export RUSTCODE_PS_TEST=ps_env_ok")
                        .exit_code,
                    Some(0)
                );
                let ps_res = ps_term.execute_command(
                    "if (Test-Path marker_ok.txt) { Write-Output \"PS_FOUND:$env:RUSTCODE_PS_TEST\" }\nWrite-Output 'PS_LINE_2'",
                );
                assert_eq!(
                    ps_res.exit_code,
                    Some(0),
                    "powershell failed: stdout={:?} stderr={:?}",
                    ps_res.stdout,
                    ps_res.stderr
                );
                assert!(ps_res.stdout.contains("PS_FOUND:ps_env_ok"), "stdout={:?}", ps_res.stdout);
                assert!(ps_res.stdout.contains("PS_LINE_2"), "stdout={:?}", ps_res.stdout);
            }
        }

        // Verify `cd ~` resolves user home directory on both Unix (HOME) and Windows (USERPROFILE)
        if let Some(expected_home) = user_home_dir().filter(|h| h.is_dir()) {
            let home_cd = term.execute_command("cd ~");
            assert_eq!(home_cd.exit_code, Some(0));
            let canon_home = std::fs::canonicalize(&expected_home)
                .unwrap_or_else(|_| lexical_normalize_path(&expected_home));
            assert_eq!(term.cwd, canon_home);
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -------------------------------------------------------------------------
    // Phase 8 M4 Unit Tests: HeadlessPtyBackend, alacritty_terminal VT100/xterm,
    // resize clamping, scrollback, and PortablePtyBackend interactive shell
    // -------------------------------------------------------------------------

    #[test]
    fn test_headless_pty_backend_echo_resize_signals_and_exit() {
        let pty = HeadlessPtyBackend::new(24, 80).with_echo(true);
        let pty_handle = pty.clone();

        let mut term = TerminalInstance::new("pty-h", "headless", "sh", PathBuf::from("."))
            .with_pty(Box::new(pty));
        assert!(term.has_pty());
        assert!(pty_handle.is_alive());

        term.send_pty_input(b"echo HEADLESS_OK\r\n").unwrap();
        assert_eq!(pty_handle.written_input(), b"echo HEADLESS_OK\r\n");
        assert!(term.poll_pty());

        let snap = term.grid_snapshot();
        assert_eq!(snap.rows, 24);
        assert_eq!(snap.cols, 80);
        assert!(snap
            .visible_lines()
            .iter()
            .any(|l| l.contains("HEADLESS_OK")));

        term.send_ctrl_c().unwrap();
        assert_eq!(pty_handle.signals(), vec![TerminalSignal::Interrupt]);
        assert!(term.poll_pty());

        term.resize(12, 40).unwrap();
        assert_eq!(pty_handle.size(), (12, 40));
        assert_eq!(term.grid_snapshot().rows, 12);
        assert_eq!(term.grid_snapshot().cols, 40);

        pty_handle.simulate_exit(9);
        assert!(term.poll_pty());
        assert!(term.exited);
        assert_eq!(term.exit_code, Some(9));
        assert_eq!(term.exit_code(), Some(9));
        assert_eq!(term.last_exit_code, Some(9));
        assert!(term.send_pty_input(b"more").is_err());
    }

    #[test]
    fn test_alacritty_ansi_sgr_16_256_truecolor_and_styles() {
        let mut term = TerminalInstance::new("vt-colors", "vt", "sh", PathBuf::from("."));

        // SGR 16 red fg + green bg + bold/italic/underline
        term.feed_vt_bytes(b"\x1b[1;3;4;31;42mA\x1b[0m");
        // SGR 256 color (fg index 196 -> [255,0,0], bg index 21 -> [0,0,255])
        term.feed_vt_bytes(b"\x1b[38;5;196;48;5;21mB\x1b[0m");
        // Truecolor RGB (fg [10,20,30], bg [40,50,60])
        term.feed_vt_bytes(b"\x1b[38;2;10;20;30;48;2;40;50;60mC\x1b[0m");

        let snap = term.grid_snapshot();
        let a = snap.cell(0, 0).unwrap();
        assert_eq!(a.ch, 'A');
        assert_eq!(a.fg, [205, 49, 49, 255]);
        assert_eq!(a.bg, [13, 188, 121, 255]);
        assert!(a.bold);
        assert!(a.italic);
        assert!(a.underline);

        let b = snap.cell(0, 1).unwrap();
        assert_eq!(b.ch, 'B');
        assert_eq!(b.fg, [255, 0, 0, 255]);
        assert_eq!(b.bg, [0, 0, 255, 255]);
        assert!(!b.bold);
        assert!(!b.italic);
        assert!(!b.underline);

        let c = snap.cell(0, 2).unwrap();
        assert_eq!(c.ch, 'C');
        assert_eq!(c.fg, [10, 20, 30, 255]);
        assert_eq!(c.bg, [40, 50, 60, 255]);
    }

    #[test]
    fn test_alacritty_cursor_move_clear_screen_and_alt_screen() {
        let mut term = TerminalInstance::new("vt-nav", "vt", "sh", PathBuf::from("."));
        term.feed_vt_bytes(b"PRIMARY_TEXT");
        // Cursor positioning: 1-based row 2, col 5 -> 0-based row 1, col 4
        term.feed_vt_bytes(b"\x1b[2;5HX");
        let snap1 = term.grid_snapshot();
        assert_eq!(snap1.cell(1, 4).unwrap().ch, 'X');
        assert_eq!(snap1.cursor_row, 1);
        assert_eq!(snap1.cursor_col, 5);
        assert!(snap1.cursor_visible);
        assert!(!snap1.alt_screen);

        // Switch to alternate screen, write, then return to primary screen
        term.feed_vt_bytes(b"\x1b[?1049h\x1b[1;1HALT_ONLY");
        let snap_alt = term.grid_snapshot();
        assert!(snap_alt.alt_screen);
        assert_eq!(snap_alt.row_text(0).unwrap(), "ALT_ONLY");

        term.feed_vt_bytes(b"\x1b[?1049l");
        let snap_restored = term.grid_snapshot();
        assert!(!snap_restored.alt_screen);
        assert_eq!(snap_restored.row_text(0).unwrap(), "PRIMARY_TEXT");
        assert_eq!(snap_restored.cell(1, 4).unwrap().ch, 'X');

        // Clear screen (\x1b[2J) and home (\x1b[H)
        term.feed_vt_bytes(b"\x1b[2J\x1b[H");
        let snap_cleared = term.grid_snapshot();
        assert!(snap_cleared.row_text(0).unwrap().is_empty());
        assert!(snap_cleared.row_text(1).unwrap().is_empty());
        assert_eq!(snap_cleared.cursor_row, 0);
        assert_eq!(snap_cleared.cursor_col, 0);

        // Hide cursor (\x1b[?25l) and show cursor (\x1b[?25h)
        term.feed_vt_bytes(b"\x1b[?25l");
        assert!(!term.grid_snapshot().cursor_visible);
        term.feed_vt_bytes(b"\x1b[?25h");
        assert!(term.grid_snapshot().cursor_visible);
    }

    #[test]
    fn test_vt_resize_clamping_and_scrollback_limit() {
        let pty = HeadlessPtyBackend::new(24, 80);
        let pty_handle = pty.clone();
        let mut term = TerminalInstance::new("vt-scroll", "vt", "sh", PathBuf::from("."))
            .with_pty(Box::new(pty));

        // Zero dimensions clamp safely >= 1 without panicking
        term.resize(0, 0).unwrap();
        let clamped = term.grid_snapshot();
        assert!(clamped.rows >= 1);
        assert!(clamped.cols >= 1);
        assert!(pty_handle.size().0 >= 1 && pty_handle.size().1 >= 1);

        // Resize to 5x40 and emit > DEFAULT_MAX_SCROLLBACK lines
        term.resize(5, 40).unwrap();
        let mut batch = String::new();
        for i in 0..(DEFAULT_MAX_SCROLLBACK + 150) {
            batch.push_str(&format!("hist_{:04}\r\n", i));
        }
        term.feed_vt_bytes(batch.as_bytes());

        let snap = term.grid_snapshot();
        assert!(snap.history_size <= DEFAULT_MAX_SCROLLBACK);
        assert_eq!(snap.history_size, DEFAULT_MAX_SCROLLBACK);

        // Scroll up into history and back to bottom
        term.scroll_display(10);
        let scrolled = term.grid_snapshot();
        assert_eq!(scrolled.display_offset, 10);
        assert!(!scrolled.cursor_visible);

        term.grid_mut().scroll_to_bottom();
        let bottom = term.grid_snapshot();
        assert_eq!(bottom.display_offset, 0);
    }

    #[test]
    fn test_portable_pty_interactive_shell_echo_and_exit() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut term =
            TerminalInstance::new_pty("real-pty", "shell", &default_shell(), cwd, 24, 80)
                .expect("PortablePtyBackend should spawn OS shell");
        assert!(term.has_pty());

        // Resize real PTY safely
        term.resize(20, 90).unwrap();
        assert_eq!(term.grid_snapshot().rows, 20);
        assert_eq!(term.grid_snapshot().cols, 90);

        // Write interactive command and verify grid updates with output
        term.send_pty_input(b"echo PHASE8_PTY_OK\r").unwrap();

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut found_echo = false;
        while Instant::now() < deadline {
            let _ = term.poll_pty();
            if term
                .grid_snapshot()
                .visible_lines()
                .iter()
                .any(|line| line.trim() == "PHASE8_PTY_OK" || line.contains("PHASE8_PTY_OK"))
            {
                found_echo = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            found_echo,
            "expected PHASE8_PTY_OK in PTY grid, got: {:?}",
            term.grid_snapshot().visible_lines()
        );

        // Exit shell with explicit exit code and verify propagation
        term.send_pty_input(b"exit 7\r").unwrap();
        let exit_deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < exit_deadline && !term.exited {
            let _ = term.poll_pty();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(term.exited, "expected PTY shell to exit");
        assert_eq!(term.exit_code, Some(7));
        assert_eq!(term.last_exit_code, Some(7));
    }
}
