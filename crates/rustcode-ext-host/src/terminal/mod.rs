//! Extension Host Terminal Management (`vscode.window.createTerminal`).

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtTerminalState {
    pub id: String,
    pub name: String,
    pub shell_path: Option<String>,
    pub shell_args: Vec<String>,
    pub cwd: Option<String>,
    pub sent_texts: Vec<String>,
    pub is_visible: bool,
    pub preserve_focus: bool,
    pub is_disposed: bool,
    #[serde(default)]
    pub rows: Option<u16>,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub has_exited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtTerminalLifecycleEvent {
    Resized {
        id: String,
        rows: u16,
        cols: u16,
    },
    Exited {
        id: String,
        exit_code: Option<i32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtTerminalEvent {
    Created {
        id: String,
        name: String,
        shell_path: Option<String>,
        shell_args: Vec<String>,
        cwd: Option<String>,
    },
    Shown {
        id: String,
        preserve_focus: bool,
    },
    TextSent {
        id: String,
        text: String,
        add_new_line: bool,
    },
    Disposed {
        id: String,
    },
}

pub type TerminalEventListener = Arc<dyn Fn(&ExtTerminalEvent) + Send + Sync>;
pub type TerminalLifecycleListener = Arc<dyn Fn(&ExtTerminalLifecycleEvent) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostTerminalManager {
    terminals: Arc<RwLock<HashMap<String, ExtTerminalState>>>,
    order: Arc<RwLock<Vec<String>>>,
    listeners: Arc<RwLock<Vec<TerminalEventListener>>>,
    pending_events: Arc<RwLock<Vec<ExtTerminalEvent>>>,
    lifecycle_listeners: Arc<RwLock<Vec<TerminalLifecycleListener>>>,
    pending_lifecycle_events: Arc<RwLock<Vec<ExtTerminalLifecycleEvent>>>,
    revision: Arc<AtomicU64>,
}

impl ExtHostTerminalManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<ExtTerminalEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn take_pending_lifecycle_events(&self) -> Vec<ExtTerminalLifecycleEvent> {
        std::mem::take(&mut *self.pending_lifecycle_events.write())
    }

    pub fn on_terminal_event<F>(&self, listener: F)
    where
        F: Fn(&ExtTerminalEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    pub fn on_lifecycle_event<F>(&self, listener: F)
    where
        F: Fn(&ExtTerminalLifecycleEvent) + Send + Sync + 'static,
    {
        self.lifecycle_listeners.write().push(Arc::new(listener));
    }

    fn emit(&self, event: ExtTerminalEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.listeners.read().clone();
        for listener in listeners {
            listener(&event);
        }
    }

    fn emit_lifecycle(&self, event: ExtTerminalLifecycleEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_lifecycle_events.write().push(event.clone());
        let listeners = self.lifecycle_listeners.read().clone();
        for listener in listeners {
            listener(&event);
        }
    }

    pub fn create_terminal(
        &self,
        id: impl Into<String>,
        name: impl Into<String>,
        shell_path: Option<String>,
        shell_args: Vec<String>,
        cwd: Option<String>,
    ) -> ExtTerminalState {
        let id = id.into();
        let name = name.into();
        let state = ExtTerminalState {
            id: id.clone(),
            name: name.clone(),
            shell_path: shell_path.clone(),
            shell_args: shell_args.clone(),
            cwd: cwd.clone(),
            sent_texts: Vec::new(),
            is_visible: false,
            preserve_focus: false,
            is_disposed: false,
            rows: None,
            cols: None,
            exit_code: None,
            has_exited: false,
        };

        {
            let mut terminals = self.terminals.write();
            let mut order = self.order.write();
            if !terminals.contains_key(&id) {
                order.push(id.clone());
            }
            terminals.insert(id.clone(), state.clone());
        }

        self.emit(ExtTerminalEvent::Created {
            id,
            name,
            shell_path,
            shell_args,
            cwd,
        });

        state
    }

    pub fn show_terminal(&self, id: &str, preserve_focus: bool) -> Result<(), String> {
        {
            let mut terminals = self.terminals.write();
            let term = terminals
                .get_mut(id)
                .ok_or_else(|| format!("Terminal '{}' not found", id))?;
            if term.is_disposed {
                return Err(format!("Terminal '{}' is disposed", id));
            }
            term.is_visible = true;
            term.preserve_focus = preserve_focus;
        }

        self.emit(ExtTerminalEvent::Shown {
            id: id.to_string(),
            preserve_focus,
        });
        Ok(())
    }

    pub fn send_text(&self, id: &str, text: &str, add_new_line: bool) -> Result<(), String> {
        let recorded = if add_new_line && !text.ends_with('\n') {
            format!("{}\n", text)
        } else {
            text.to_string()
        };

        {
            let mut terminals = self.terminals.write();
            let term = terminals
                .get_mut(id)
                .ok_or_else(|| format!("Terminal '{}' not found", id))?;
            if term.is_disposed {
                return Err(format!("Terminal '{}' is disposed", id));
            }
            term.sent_texts.push(recorded);
        }

        self.emit(ExtTerminalEvent::TextSent {
            id: id.to_string(),
            text: text.to_string(),
            add_new_line,
        });
        Ok(())
    }

    pub fn send_stdin(&self, id: &str, data: &str) -> Result<(), String> {
        self.send_text(id, data, false)
    }

    pub fn resize_terminal(&self, id: &str, rows: u16, cols: u16) -> Result<(), String> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        {
            let mut terminals = self.terminals.write();
            let term = terminals
                .get_mut(id)
                .ok_or_else(|| format!("Terminal '{}' not found", id))?;
            if term.is_disposed {
                return Err(format!("Terminal '{}' is disposed", id));
            }
            term.rows = Some(rows);
            term.cols = Some(cols);
        }
        self.emit_lifecycle(ExtTerminalLifecycleEvent::Resized {
            id: id.to_string(),
            rows,
            cols,
        });
        Ok(())
    }

    pub fn notify_terminal_exit(&self, id: &str, exit_code: Option<i32>) -> Result<(), String> {
        {
            let mut terminals = self.terminals.write();
            let term = terminals
                .get_mut(id)
                .ok_or_else(|| format!("Terminal '{}' not found", id))?;
            term.has_exited = true;
            term.exit_code = exit_code;
        }
        self.emit_lifecycle(ExtTerminalLifecycleEvent::Exited {
            id: id.to_string(),
            exit_code,
        });
        Ok(())
    }

    pub fn dispose_terminal(&self, id: &str) -> bool {
        {
            let mut terminals = self.terminals.write();
            let Some(term) = terminals.get_mut(id) else {
                return false;
            };
            if term.is_disposed {
                return false;
            }
            term.is_disposed = true;
            term.is_visible = false;
        }

        self.emit(ExtTerminalEvent::Disposed { id: id.to_string() });
        true
    }

    pub fn get_terminal(&self, id: &str) -> Option<ExtTerminalState> {
        self.terminals.read().get(id).cloned()
    }

    pub fn list_terminals(&self) -> Vec<ExtTerminalState> {
        let terminals = self.terminals.read();
        let order = self.order.read();
        order
            .iter()
            .filter_map(|id| terminals.get(id))
            .filter(|t| !t.is_disposed)
            .cloned()
            .collect()
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "window.createTerminal" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing terminal id".to_string())?
                    .to_string();
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Terminal")
                    .to_string();
                let shell_path = params
                    .get("shellPath")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let shell_args = params
                    .get("shellArgs")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| item.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let cwd = params
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let state = self.create_terminal(id, name, shell_path, shell_args, cwd);
                Ok(json!({ "created": true, "id": state.id, "name": state.name }))
            }
            "window.showTerminal" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing terminal id".to_string())?;
                let preserve_focus = params
                    .get("preserveFocus")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.show_terminal(id, preserve_focus)?;
                Ok(json!({ "shown": true, "id": id, "preserveFocus": preserve_focus }))
            }
            "window.sendTerminalText" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing terminal id".to_string())?;
                let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let add_new_line = params
                    .get("addNewLine")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                self.send_text(id, text, add_new_line)?;
                Ok(json!({ "sent": true, "id": id }))
            }
            "window.resizeTerminal" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing terminal id".to_string())?;
                let rows = params
                    .get("rows")
                    .or_else(|| params.get("dimensions").and_then(|d| d.get("rows")))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(24)
                    .clamp(1, u16::MAX as u64) as u16;
                let cols = params
                    .get("cols")
                    .or_else(|| params.get("columns"))
                    .or_else(|| params.get("dimensions").and_then(|d| d.get("columns")))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(80)
                    .clamp(1, u16::MAX as u64) as u16;
                self.resize_terminal(id, rows, cols)?;
                Ok(json!({ "resized": true, "id": id, "rows": rows, "cols": cols }))
            }
            "window.exitTerminal" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing terminal id".to_string())?;
                let exit_code = params
                    .get("exitCode")
                    .and_then(|v| v.as_i64())
                    .map(|c| c as i32);
                self.notify_terminal_exit(id, exit_code)?;
                Ok(json!({ "exited": true, "id": id, "exitCode": exit_code }))
            }
            "window.disposeTerminal" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing terminal id".to_string())?;
                let disposed = self.dispose_terminal(id);
                Ok(json!({ "disposed": disposed, "id": id }))
            }
            _ => Err(format!("Unsupported terminal RPC method: {}", method)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_ext_host_terminal_manager_lifecycle() {
        let mgr = ExtHostTerminalManager::new();
        let events: Arc<Mutex<Vec<ExtTerminalEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        mgr.on_terminal_event(move |ev| {
            events_clone.lock().unwrap().push(ev.clone());
        });

        let rev0 = mgr.revision();
        mgr.handle_rpc_call(
            "window.createTerminal",
            &json!({ "id": "term_1", "name": "Build", "cwd": "/tmp" }),
        )
        .unwrap();
        assert!(mgr.revision() > rev0);
        assert_eq!(mgr.list_terminals().len(), 1);

        mgr.handle_rpc_call(
            "window.showTerminal",
            &json!({ "id": "term_1", "preserveFocus": true }),
        )
        .unwrap();
        assert!(mgr.get_terminal("term_1").unwrap().is_visible);
        assert!(mgr.get_terminal("term_1").unwrap().preserve_focus);

        mgr.handle_rpc_call(
            "window.sendTerminalText",
            &json!({ "id": "term_1", "text": "echo ok", "addNewLine": true }),
        )
        .unwrap();
        assert_eq!(mgr.get_terminal("term_1").unwrap().sent_texts, vec!["echo ok\n"]);

        mgr.handle_rpc_call("window.disposeTerminal", &json!({ "id": "term_1" }))
            .unwrap();
        assert!(mgr.get_terminal("term_1").unwrap().is_disposed);
        assert!(mgr.list_terminals().is_empty());
        assert!(mgr
            .handle_rpc_call(
                "window.sendTerminalText",
                &json!({ "id": "term_1", "text": "fail" }),
            )
            .is_err());
        assert_eq!(events.lock().unwrap().len(), 4);
        assert_eq!(mgr.take_pending_events().len(), 4);
    }

    #[test]
    fn test_ext_host_terminal_resize_exit_and_stdin() {
        let mgr = ExtHostTerminalManager::new();
        mgr.create_terminal("term_pty", "Shell", None, Vec::new(), None);
        mgr.send_stdin("term_pty", "ls -la").unwrap();
        mgr.handle_rpc_call(
            "window.resizeTerminal",
            &json!({ "id": "term_pty", "rows": 40, "cols": 120 }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "window.exitTerminal",
            &json!({ "id": "term_pty", "exitCode": 0 }),
        )
        .unwrap();

        let state = mgr.get_terminal("term_pty").unwrap();
        assert_eq!(state.sent_texts, vec!["ls -la"]);
        assert_eq!(state.rows, Some(40));
        assert_eq!(state.cols, Some(120));
        assert!(state.has_exited);
        assert_eq!(state.exit_code, Some(0));

        let lifecycle = mgr.take_pending_lifecycle_events();
        assert_eq!(
            lifecycle,
            vec![
                ExtTerminalLifecycleEvent::Resized {
                    id: "term_pty".to_string(),
                    rows: 40,
                    cols: 120,
                },
                ExtTerminalLifecycleEvent::Exited {
                    id: "term_pty".to_string(),
                    exit_code: Some(0),
                },
            ]
        );
    }
}
