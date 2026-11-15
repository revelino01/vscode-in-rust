//! `rustcode-terminal` — Integrated Terminal Management.
//!
//! Manages terminal sessions, output buffers, and split/tab views.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalInstance {
    pub id: String,
    pub title: String,
    pub shell: String,
    pub cwd: PathBuf,
    pub lines: Vec<String>,
}

impl TerminalInstance {
    pub fn new(id: &str, title: &str, shell: &str, cwd: PathBuf) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            shell: shell.to_string(),
            cwd,
            lines: Vec::new(),
        }
    }

    pub fn write_output(&mut self, text: &str) {
        for line in text.split('\n') {
            self.lines.push(line.to_string());
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

pub struct TerminalManager {
    terminals: Vec<TerminalInstance>,
    active_index: Option<usize>,
    next_id: u32,
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
        }
    }

    pub fn create_terminal(&mut self, title: Option<&str>, shell: Option<&str>, cwd: Option<PathBuf>) -> &TerminalInstance {
        let id = format!("term-{}", self.next_id);
        self.next_id += 1;

        let title = title.unwrap_or("Terminal");
        let shell = shell.unwrap_or("/bin/sh");
        let cwd = cwd.unwrap_or_else(|| PathBuf::from("."));

        let inst = TerminalInstance::new(&id, title, shell, cwd);
        self.terminals.push(inst);
        self.active_index = Some(self.terminals.len() - 1);
        self.terminals.last().unwrap()
    }

    pub fn close_terminal(&mut self, id: &str) -> bool {
        if let Some(pos) = self.terminals.iter().position(|t| t.id == id) {
            self.terminals.remove(pos);
            if self.terminals.is_empty() {
                self.active_index = None;
            } else if let Some(active) = self.active_index {
                if active >= self.terminals.len() {
                    self.active_index = Some(self.terminals.len() - 1);
                } else if active > pos {
                    self.active_index = Some(active - 1);
                }
            }
            true
        } else {
            false
        }
    }

    pub fn active_terminal(&self) -> Option<&TerminalInstance> {
        self.active_index.and_then(|idx| self.terminals.get(idx))
    }

    pub fn terminals(&self) -> &[TerminalInstance] {
        &self.terminals
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
}
