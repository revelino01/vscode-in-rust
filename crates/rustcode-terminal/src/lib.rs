//! `rustcode-terminal` — Integrated Terminal Management.
//!
//! Manages terminal sessions, output buffers, and split/tab views.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_MAX_SCROLLBACK: usize = 2000;
pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 5000;

fn default_max_scrollback() -> usize {
    DEFAULT_MAX_SCROLLBACK
}

fn default_command_timeout_ms() -> u64 {
    DEFAULT_COMMAND_TIMEOUT_MS
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub exited: bool,
    #[serde(default = "default_command_timeout_ms")]
    pub command_timeout_ms: u64,
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
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
            exited: false,
            command_timeout_ms: DEFAULT_COMMAND_TIMEOUT_MS,
            env_vars: BTreeMap::new(),
        }
    }

    pub fn write_output(&mut self, text: &str) {
        for line in text.split('\n') {
            self.lines.push(line.to_string());
        }
        self.trim_scrollback();
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
    }

    pub fn trim_scrollback(&mut self) {
        if self.max_scrollback > 0 && self.lines.len() > self.max_scrollback {
            let excess = self.lines.len() - self.max_scrollback;
            self.lines.drain(0..excess);
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn set_max_scrollback(&mut self, max_lines: usize) {
        self.max_scrollback = max_lines;
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

        let prompt_line = format!("{}{}", self.prompt_prefix(), cmd);
        self.lines.push(prompt_line);
        self.trim_scrollback();

        let trimmed = cmd.trim();
        if !trimmed.is_empty()
            && self.command_history.last().map(|s| s.as_str()) != Some(trimmed)
        {
            self.command_history.push(trimmed.to_string());
        }

        self.execute_command(trimmed)
    }

    fn effective_cwd(&self) -> PathBuf {
        if self.cwd.is_dir() {
            self.cwd.clone()
        } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        }
    }

    fn resolve_shell(&self) -> &str {
        if !self.shell.is_empty() && Path::new(&self.shell).exists() {
            &self.shell
        } else if Path::new("/bin/sh").exists() {
            "/bin/sh"
        } else if Path::new("/system/bin/sh").exists() {
            "/system/bin/sh"
        } else {
            "sh"
        }
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
        if trimmed.is_empty() {
            self.last_exit_code = Some(0);
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

        let shell_prog = self.resolve_shell().to_string();
        let cwd = self.effective_cwd();
        let mut command = Command::new(&shell_prog);
        command
            .arg("-c")
            .arg(trimmed)
            .current_dir(&cwd)
            .envs(&self.env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(err) => {
                let msg = format!("Failed to spawn '{}': {}", shell_prog, err);
                self.append_output_block(&msg);
                self.last_exit_code = Some(127);
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
            self.last_exit_code = None;
            return CommandExecutionResult {
                exit_code: None,
                stdout,
                stderr,
                timed_out: true,
                exited: false,
            };
        }

        let code = exit_status.and_then(|s| s.code());
        self.last_exit_code = code;
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
                self.last_exit_code = Some(0);
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
                self.last_exit_code = Some(0);
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
                self.last_exit_code = Some(code);
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
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| self.effective_cwd())
                } else if let Some(sub) = target_raw.strip_prefix("~/") {
                    std::env::var_os("HOME")
                        .map(|h| PathBuf::from(h).join(sub))
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
                    self.last_exit_code = Some(0);
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
                    self.last_exit_code = Some(1);
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
                self.last_exit_code = Some(0);
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
                self.last_exit_code = Some(0);
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
        let shell = shell.unwrap_or("/bin/sh");
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
        self.terminals.retain(|t| !pane_ids.iter().any(|pid| pid == &t.id));

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
            .unwrap_or_else(|| "/bin/sh".to_string());
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
        assert!(term.lines.iter().any(|l| l.contains("[Process exited]")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_terminal_pure_rust_command_execution_and_timeout() {
        let mut term = TerminalInstance::new("t1", "sh", "/bin/sh", PathBuf::from("."));
        term.insert_str("echo hello_rustcode");
        let res = term.submit_input();
        assert_eq!(res.exit_code, Some(0));
        assert!(!res.timed_out);
        assert!(term.lines.iter().any(|l| l.contains("hello_rustcode")));

        term.set_command_timeout(Duration::from_millis(50));
        term.insert_str("sleep 2");
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
        let mut term = TerminalInstance::new("t1", "sh", "/bin/sh", PathBuf::from("."));
        term.set_command_timeout(Duration::from_millis(50));

        let start = Instant::now();
        let res = term.execute_command("sleep 2; echo after");
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
}
