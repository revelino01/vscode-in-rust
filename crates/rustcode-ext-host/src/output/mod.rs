//! Extension Host Output Channel Management (`vscode.window.createOutputChannel`).

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum ExtLogLevel {
    Off = 0,
    Trace = 1,
    Debug = 2,
    #[default]
    Info = 3,
    Warning = 4,
    Error = 5,
}

impl ExtLogLevel {
    pub fn from_wire(val: &Value) -> Self {
        if let Some(n) = val.as_u64() {
            return match n {
                0 => Self::Off,
                1 => Self::Trace,
                2 => Self::Debug,
                4 => Self::Warning,
                5 => Self::Error,
                _ => Self::Info,
            };
        }
        if let Some(s) = val.as_str() {
            return match s.trim().to_ascii_lowercase().as_str() {
                "off" => Self::Off,
                "trace" => Self::Trace,
                "debug" => Self::Debug,
                "warning" | "warn" => Self::Warning,
                "error" | "err" => Self::Error,
                _ => Self::Info,
            };
        }
        Self::Info
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warning => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutputChannelState {
    pub id: String,
    pub name: String,
    pub language_id: Option<String>,
    pub log: bool,
    pub content: String,
    pub is_visible: bool,
    pub preserve_focus: bool,
    pub is_disposed: bool,
    #[serde(default)]
    pub log_level: ExtLogLevel,
}

impl OutputChannelState {
    pub fn lines(&self) -> Vec<&str> {
        let trimmed = self.content.strip_suffix('\n').unwrap_or(&self.content);
        if trimmed.is_empty() && self.content.is_empty() {
            Vec::new()
        } else {
            trimmed.split('\n').collect()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum OutputChannelEvent {
    Created {
        id: String,
        name: String,
        language_id: Option<String>,
        log: bool,
    },
    Appended {
        id: String,
        name: String,
        text: String,
    },
    Replaced {
        id: String,
        name: String,
        text: String,
    },
    Cleared {
        id: String,
        name: String,
    },
    Shown {
        id: String,
        name: String,
        preserve_focus: bool,
    },
    Hidden {
        id: String,
        name: String,
    },
    Disposed {
        id: String,
        name: String,
    },
    LogLevelChanged {
        id: String,
        name: String,
        log_level: ExtLogLevel,
    },
}

pub type OutputEventListener = Arc<dyn Fn(&OutputChannelEvent) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostOutputManager {
    channels: Arc<RwLock<HashMap<String, OutputChannelState>>>,
    order: Arc<RwLock<Vec<String>>>,
    active_channel_id: Arc<RwLock<Option<String>>>,
    listeners: Arc<RwLock<Vec<OutputEventListener>>>,
    pending_events: Arc<RwLock<Vec<OutputChannelEvent>>>,
    revision: Arc<AtomicU64>,
}

impl ExtHostOutputManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<OutputChannelEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_output_event<F>(&self, listener: F)
    where
        F: Fn(&OutputChannelEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    fn emit(&self, event: OutputChannelEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.listeners.read().clone();
        for listener in listeners {
            listener(&event);
        }
    }

    pub fn create_channel(
        &self,
        id: impl Into<String>,
        name: impl Into<String>,
        language_id: Option<String>,
        log: bool,
    ) -> OutputChannelState {
        let id = id.into();
        let name = name.into();
        let state = OutputChannelState {
            id: id.clone(),
            name: name.clone(),
            language_id: language_id.clone(),
            log,
            content: String::new(),
            is_visible: false,
            preserve_focus: false,
            is_disposed: false,
            log_level: ExtLogLevel::Info,
        };

        {
            let mut channels = self.channels.write();
            let mut order = self.order.write();
            if !channels.contains_key(&id) {
                order.push(id.clone());
            }
            channels.insert(id.clone(), state.clone());
        }

        {
            let mut active = self.active_channel_id.write();
            if active.is_none() {
                *active = Some(id.clone());
            }
        }

        self.emit(OutputChannelEvent::Created {
            id,
            name,
            language_id,
            log,
        });

        state
    }

    pub fn append(&self, id: &str, text: &str) -> Result<(), String> {
        let name = {
            let mut channels = self.channels.write();
            let ch = channels
                .get_mut(id)
                .ok_or_else(|| format!("Output channel '{}' not found", id))?;
            if ch.is_disposed {
                return Err(format!("Output channel '{}' is disposed", id));
            }
            ch.content.push_str(text);
            ch.name.clone()
        };

        self.emit(OutputChannelEvent::Appended {
            id: id.to_string(),
            name,
            text: text.to_string(),
        });
        Ok(())
    }

    pub fn replace(&self, id: &str, text: &str) -> Result<(), String> {
        let name = {
            let mut channels = self.channels.write();
            let ch = channels
                .get_mut(id)
                .ok_or_else(|| format!("Output channel '{}' not found", id))?;
            if ch.is_disposed {
                return Err(format!("Output channel '{}' is disposed", id));
            }
            ch.content = text.to_string();
            ch.name.clone()
        };

        self.emit(OutputChannelEvent::Replaced {
            id: id.to_string(),
            name,
            text: text.to_string(),
        });
        Ok(())
    }

    pub fn clear(&self, id: &str) -> Result<(), String> {
        let name = {
            let mut channels = self.channels.write();
            let ch = channels
                .get_mut(id)
                .ok_or_else(|| format!("Output channel '{}' not found", id))?;
            if ch.is_disposed {
                return Err(format!("Output channel '{}' is disposed", id));
            }
            ch.content.clear();
            ch.name.clone()
        };

        self.emit(OutputChannelEvent::Cleared {
            id: id.to_string(),
            name,
        });
        Ok(())
    }

    pub fn show(&self, id: &str, preserve_focus: bool) -> Result<(), String> {
        let name = {
            let mut channels = self.channels.write();
            let ch = channels
                .get_mut(id)
                .ok_or_else(|| format!("Output channel '{}' not found", id))?;
            if ch.is_disposed {
                return Err(format!("Output channel '{}' is disposed", id));
            }
            ch.is_visible = true;
            ch.preserve_focus = preserve_focus;
            ch.name.clone()
        };

        *self.active_channel_id.write() = Some(id.to_string());
        self.emit(OutputChannelEvent::Shown {
            id: id.to_string(),
            name,
            preserve_focus,
        });
        Ok(())
    }

    pub fn hide(&self, id: &str) -> Result<(), String> {
        let name = {
            let mut channels = self.channels.write();
            let ch = channels
                .get_mut(id)
                .ok_or_else(|| format!("Output channel '{}' not found", id))?;
            if ch.is_disposed {
                return Err(format!("Output channel '{}' is disposed", id));
            }
            ch.is_visible = false;
            ch.name.clone()
        };

        self.emit(OutputChannelEvent::Hidden {
            id: id.to_string(),
            name,
        });
        Ok(())
    }

    pub fn dispose_channel(&self, id: &str) -> bool {
        let disposed_name = {
            let mut channels = self.channels.write();
            let Some(ch) = channels.get_mut(id) else {
                return false;
            };
            if ch.is_disposed {
                return false;
            }
            ch.is_disposed = true;
            ch.is_visible = false;
            ch.name.clone()
        };

        {
            let mut active = self.active_channel_id.write();
            if active.as_deref() == Some(id) {
                let channels = self.channels.read();
                let order = self.order.read();
                *active = order
                    .iter()
                    .find(|cid| channels.get(*cid).is_some_and(|c| !c.is_disposed))
                    .cloned();
            }
        }

        self.emit(OutputChannelEvent::Disposed {
            id: id.to_string(),
            name: disposed_name,
        });
        true
    }

    pub fn get_channel(&self, id: &str) -> Option<OutputChannelState> {
        self.channels.read().get(id).cloned()
    }

    pub fn get_channel_by_name(&self, name: &str) -> Option<OutputChannelState> {
        let channels = self.channels.read();
        let order = self.order.read();
        order
            .iter()
            .filter_map(|id| channels.get(id))
            .find(|ch| !ch.is_disposed && ch.name == name)
            .cloned()
    }

    pub fn list_channels(&self) -> Vec<OutputChannelState> {
        let channels = self.channels.read();
        let order = self.order.read();
        order
            .iter()
            .filter_map(|id| channels.get(id))
            .filter(|ch| !ch.is_disposed)
            .cloned()
            .collect()
    }

    pub fn active_channel_id(&self) -> Option<String> {
        self.active_channel_id.read().clone()
    }

    fn resolve_channel_id(&self, params: &Value) -> Result<String, String> {
        if let Some(id) = params.get("id").and_then(|v| v.as_str()) {
            return Ok(id.to_string());
        }
        if let Some(name) = params.get("name").and_then(|v| v.as_str()) {
            if let Some(ch) = self.get_channel_by_name(name) {
                return Ok(ch.id);
            }
        }
        Err("Missing output channel id".to_string())
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "window.createOutputChannel" => {
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Output")
                    .to_string();
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("output_{}", name));
                let language_id = params
                    .get("languageId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let log = params.get("log").and_then(|v| v.as_bool()).unwrap_or(false);
                let state = self.create_channel(id, name, language_id, log);
                Ok(json!({
                    "created": true,
                    "id": state.id,
                    "name": state.name,
                }))
            }
            "window.appendOutputChannel" => {
                let id = self.resolve_channel_id(params)?;
                let value = params
                    .get("value")
                    .or_else(|| params.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                self.append(&id, value)?;
                Ok(json!({ "appended": true, "id": id }))
            }
            "window.replaceOutputChannel" => {
                let id = self.resolve_channel_id(params)?;
                let value = params
                    .get("value")
                    .or_else(|| params.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                self.replace(&id, value)?;
                Ok(json!({ "replaced": true, "id": id }))
            }
            "window.clearOutputChannel" => {
                let id = self.resolve_channel_id(params)?;
                self.clear(&id)?;
                Ok(json!({ "cleared": true, "id": id }))
            }
            "window.showOutputChannel" => {
                let id = self.resolve_channel_id(params)?;
                let preserve_focus = params
                    .get("preserveFocus")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.show(&id, preserve_focus)?;
                Ok(json!({
                    "shown": true,
                    "id": id,
                    "preserveFocus": preserve_focus,
                }))
            }
            "window.hideOutputChannel" => {
                let id = self.resolve_channel_id(params)?;
                self.hide(&id)?;
                Ok(json!({ "hidden": true, "id": id }))
            }
            "window.disposeOutputChannel" => {
                let id = self.resolve_channel_id(params)?;
                let disposed = self.dispose_channel(&id);
                Ok(json!({ "disposed": disposed, "id": id }))
            }
            "window.setOutputChannelLogLevel" => {
                let id = self.resolve_channel_id(params)?;
                let level = params
                    .get("logLevel")
                    .map(ExtLogLevel::from_wire)
                    .unwrap_or(ExtLogLevel::Info);
                let name = {
                    let mut channels = self.channels.write();
                    let ch = channels
                        .get_mut(&id)
                        .ok_or_else(|| format!("Output channel '{}' not found", id))?;
                    ch.log_level = level;
                    ch.name.clone()
                };
                self.emit(OutputChannelEvent::LogLevelChanged {
                    id: id.clone(),
                    name,
                    log_level: level,
                });
                Ok(json!({ "id": id, "logLevel": level as u8 }))
            }
            "window.logOutputChannelMessage" => {
                let id = self.resolve_channel_id(params)?;
                let level = params
                    .get("level")
                    .map(ExtLogLevel::from_wire)
                    .unwrap_or(ExtLogLevel::Info);
                let message = params
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let current_level = self
                    .channels
                    .read()
                    .get(&id)
                    .map(|c| c.log_level)
                    .ok_or_else(|| format!("Output channel '{}' not found", id))?;
                let written = if current_level != ExtLogLevel::Off && level >= current_level {
                    self.append(&id, &format!("[{}] {}\n", level.label(), message))?;
                    true
                } else {
                    false
                };
                Ok(json!({ "written": written, "id": id, "level": level as u8 }))
            }
            _ => Err(format!("Unsupported output channel RPC method: {}", method)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_output_manager_lifecycle_and_events() {
        let mgr = ExtHostOutputManager::new();
        let events: Arc<Mutex<Vec<OutputChannelEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        mgr.on_output_event(move |ev| {
            events_clone.lock().unwrap().push(ev.clone());
        });

        let rev0 = mgr.revision();
        let res = mgr
            .handle_rpc_call(
                "window.createOutputChannel",
                &json!({ "id": "out_1", "name": "Build", "languageId": "log", "log": true }),
            )
            .unwrap();
        assert_eq!(res["created"], true);
        assert!(mgr.revision() > rev0);
        assert_eq!(mgr.active_channel_id().as_deref(), Some("out_1"));

        mgr.handle_rpc_call(
            "window.appendOutputChannel",
            &json!({ "id": "out_1", "value": "hello " }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "window.appendOutputChannel",
            &json!({ "id": "out_1", "value": "world\nline2\n" }),
        )
        .unwrap();
        let ch = mgr.get_channel("out_1").unwrap();
        assert_eq!(ch.lines(), vec!["hello world", "line2"]);

        mgr.handle_rpc_call(
            "window.replaceOutputChannel",
            &json!({ "id": "out_1", "value": "replaced\n" }),
        )
        .unwrap();
        assert_eq!(mgr.get_channel("out_1").unwrap().lines(), vec!["replaced"]);

        mgr.handle_rpc_call(
            "window.showOutputChannel",
            &json!({ "id": "out_1", "preserveFocus": true }),
        )
        .unwrap();
        let ch = mgr.get_channel("out_1").unwrap();
        assert!(ch.is_visible);
        assert!(ch.preserve_focus);

        mgr.handle_rpc_call("window.hideOutputChannel", &json!({ "id": "out_1" }))
            .unwrap();
        assert!(!mgr.get_channel("out_1").unwrap().is_visible);

        mgr.handle_rpc_call("window.clearOutputChannel", &json!({ "id": "out_1" }))
            .unwrap();
        assert!(mgr.get_channel("out_1").unwrap().lines().is_empty());

        mgr.handle_rpc_call("window.disposeOutputChannel", &json!({ "id": "out_1" }))
            .unwrap();
        assert!(mgr.get_channel("out_1").unwrap().is_disposed);
        assert!(mgr.list_channels().is_empty());
        assert!(mgr
            .handle_rpc_call(
                "window.appendOutputChannel",
                &json!({ "id": "out_1", "value": "after dispose" }),
            )
            .is_err());

        assert_eq!(events.lock().unwrap().len(), 8);
        assert_eq!(mgr.take_pending_events().len(), 8);
        assert!(mgr.take_pending_events().is_empty());
    }
}
