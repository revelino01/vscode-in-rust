//! Extension Host Debug Manager (`vscode.debug.*`) with `rustcode-protocols::dap` interop.

use crate::rpc::RpcBridge;
use parking_lot::RwLock;
use rustcode_protocols::dap::{
    Breakpoint, ContinuedEventBody, DapMessage, OutputEventBody, Source, SourceBreakpoint,
    StoppedEventBody,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtDebugProviderInfo {
    pub id: String,
    pub debug_type: String,
    pub kind: String,
    #[serde(default)]
    pub trigger_kind: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtDebugCustomEvent {
    pub session_id: String,
    pub event: String,
    #[serde(default)]
    pub body: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtDebugSessionState {
    pub id: String,
    pub session_type: String,
    pub name: String,
    #[serde(default)]
    pub workspace_folder: Option<String>,
    #[serde(default)]
    pub configuration: Value,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub is_terminated: bool,
    #[serde(default)]
    pub custom_events: Vec<ExtDebugCustomEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtDebugBreakpoint {
    pub id: String,
    #[serde(default = "default_source_kind")]
    pub kind: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub character: Option<u32>,
    #[serde(default)]
    pub function_name: Option<String>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub hit_condition: Option<String>,
    #[serde(default)]
    pub log_message: Option<String>,
}

fn default_source_kind() -> String {
    "source".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtDebugConsoleEntry {
    pub value: String,
    #[serde(default)]
    pub new_line: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtDebugEvent {
    ConfigurationProviderRegistered {
        id: String,
        debug_type: String,
        trigger_kind: u32,
    },
    ConfigurationProviderUnregistered {
        id: String,
    },
    AdapterDescriptorFactoryRegistered {
        id: String,
        debug_type: String,
    },
    AdapterDescriptorFactoryUnregistered {
        id: String,
    },
    AdapterTrackerFactoryRegistered {
        id: String,
        debug_type: String,
    },
    AdapterTrackerFactoryUnregistered {
        id: String,
    },
    SessionStarted {
        session_id: String,
        session_type: String,
        name: String,
    },
    SessionStopped {
        session_id: String,
    },
    BreakpointsAdded {
        ids: Vec<String>,
    },
    BreakpointsRemoved {
        ids: Vec<String>,
    },
    ConsoleAppended {
        value: String,
        new_line: bool,
    },
    CustomEventReceived {
        session_id: String,
        event: String,
        body: Value,
    },
}

pub type DebugEventListener = Arc<dyn Fn(&ExtDebugEvent) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostDebugManager {
    config_providers: Arc<RwLock<HashMap<String, ExtDebugProviderInfo>>>,
    adapter_factories: Arc<RwLock<HashMap<String, ExtDebugProviderInfo>>>,
    tracker_factories: Arc<RwLock<HashMap<String, ExtDebugProviderInfo>>>,
    sessions: Arc<RwLock<HashMap<String, ExtDebugSessionState>>>,
    active_session_id: Arc<RwLock<Option<String>>>,
    breakpoints: Arc<RwLock<HashMap<String, ExtDebugBreakpoint>>>,
    breakpoint_order: Arc<RwLock<Vec<String>>>,
    console_entries: Arc<RwLock<Vec<ExtDebugConsoleEntry>>>,
    listeners: Arc<RwLock<Vec<DebugEventListener>>>,
    pending_events: Arc<RwLock<Vec<ExtDebugEvent>>>,
    revision: Arc<AtomicU64>,
    next_session_seq: Arc<AtomicU64>,
}

pub fn to_dap_source(path_or_uri: &str) -> Source {
    let path = rustcode_platform::file_uri_to_path(path_or_uri);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());
    Source {
        name,
        path: Some(path.to_string_lossy().into_owned()),
        source_reference: None,
    }
}

pub fn format_dap_stopped_event(reason: &str, thread_id: Option<i64>) -> DapMessage {
    let body = StoppedEventBody {
        reason: reason.to_string(),
        description: None,
        thread_id,
        text: None,
        all_threads_stopped: Some(true),
    };
    DapMessage::Event {
        seq: 1,
        event: "stopped".to_string(),
        body: serde_json::to_value(body).ok(),
    }
}

pub fn format_dap_continued_event(thread_id: i64) -> DapMessage {
    let body = ContinuedEventBody {
        thread_id,
        all_threads_continued: Some(true),
    };
    DapMessage::Event {
        seq: 1,
        event: "continued".to_string(),
        body: serde_json::to_value(body).ok(),
    }
}

fn parse_breakpoint_from_value(v: &Value) -> Option<ExtDebugBreakpoint> {
    let id = v.get("id").and_then(|i| i.as_str())?.to_string();
    let enabled = v.get("enabled").and_then(|b| b.as_bool()).unwrap_or(true);
    let condition = v
        .get("condition")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    let hit_condition = v
        .get("hitCondition")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    let log_message = v
        .get("logMessage")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());

    if let Some(fn_name) = v.get("functionName").and_then(|f| f.as_str()) {
        return Some(ExtDebugBreakpoint {
            id,
            kind: "function".to_string(),
            enabled,
            uri: None,
            line: None,
            character: None,
            function_name: Some(fn_name.to_string()),
            condition,
            hit_condition,
            log_message,
        });
    }

    let loc = v.get("location");
    let uri = loc
        .and_then(|l| l.get("uri"))
        .and_then(|u| u.as_str())
        .or_else(|| v.get("uri").and_then(|u| u.as_str()))
        .map(|s| s.to_string());
    let line = loc
        .and_then(|l| l.get("range"))
        .and_then(|r| r.get("start"))
        .and_then(|s| s.get("line"))
        .and_then(|n| n.as_u64())
        .or_else(|| v.get("line").and_then(|n| n.as_u64()))
        .map(|n| n as u32);
    let character = loc
        .and_then(|l| l.get("range"))
        .and_then(|r| r.get("start"))
        .and_then(|s| s.get("character"))
        .and_then(|n| n.as_u64())
        .or_else(|| v.get("character").and_then(|n| n.as_u64()))
        .map(|n| n as u32);

    Some(ExtDebugBreakpoint {
        id,
        kind: v
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("source")
            .to_string(),
        enabled,
        uri,
        line,
        character,
        function_name: None,
        condition,
        hit_condition,
        log_message,
    })
}

impl ExtHostDebugManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<ExtDebugEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_debug_event<F>(&self, listener: F)
    where
        F: Fn(&ExtDebugEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    fn emit(&self, event: ExtDebugEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.listeners.read().clone();
        for cb in listeners {
            cb(&event);
        }
    }

    pub fn active_session(&self) -> Option<ExtDebugSessionState> {
        let active_id = self.active_session_id.read().clone()?;
        self.sessions.read().get(&active_id).cloned()
    }

    pub fn list_sessions(&self) -> Vec<ExtDebugSessionState> {
        self.sessions
            .read()
            .values()
            .filter(|s| !s.is_terminated)
            .cloned()
            .collect()
    }

    pub fn list_breakpoints(&self) -> Vec<ExtDebugBreakpoint> {
        let map = self.breakpoints.read();
        let order = self.breakpoint_order.read();
        order.iter().filter_map(|id| map.get(id)).cloned().collect()
    }

    pub fn console_entries(&self) -> Vec<ExtDebugConsoleEntry> {
        self.console_entries.read().clone()
    }

    pub fn list_configuration_providers(&self) -> Vec<ExtDebugProviderInfo> {
        self.config_providers.read().values().cloned().collect()
    }

    pub fn list_adapter_factories(&self) -> Vec<ExtDebugProviderInfo> {
        self.adapter_factories.read().values().cloned().collect()
    }

    pub fn to_dap_source_breakpoints(&self, path_or_uri: &str) -> Vec<SourceBreakpoint> {
        let target = rustcode_platform::file_uri_to_path(path_or_uri);
        self.list_breakpoints()
            .into_iter()
            .filter(|bp| bp.enabled && bp.kind == "source")
            .filter(|bp| {
                bp.uri
                    .as_deref()
                    .map(|u| {
                        rustcode_platform::paths_equivalent(
                            &rustcode_platform::file_uri_to_path(u),
                            &target,
                        )
                    })
                    .unwrap_or(false)
            })
            .filter_map(|bp| {
                let line = bp.line?.saturating_add(1);
                Some(SourceBreakpoint {
                    line,
                    column: bp.character.map(|c| c.saturating_add(1)),
                    condition: bp.condition,
                    hit_condition: bp.hit_condition,
                    log_message: bp.log_message,
                })
            })
            .collect()
    }

    pub fn to_dap_breakpoints(&self, path_or_uri: &str) -> Vec<Breakpoint> {
        let src = to_dap_source(path_or_uri);
        self.to_dap_source_breakpoints(path_or_uri)
            .into_iter()
            .enumerate()
            .map(|(idx, sbp)| Breakpoint {
                id: Some((idx + 1) as i64),
                verified: true,
                message: None,
                source: Some(src.clone()),
                line: Some(sbp.line),
                column: sbp.column,
            })
            .collect()
    }

    pub fn to_dap_output_events(&self) -> Vec<OutputEventBody> {
        self.console_entries
            .read()
            .iter()
            .map(|entry| OutputEventBody {
                category: Some("console".to_string()),
                output: if entry.new_line {
                    format!("{}\n", entry.value)
                } else {
                    entry.value.clone()
                },
                line: None,
                column: None,
                source: None,
            })
            .collect()
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "debug.registerConfigurationProvider" => {
                let debug_type = params
                    .get("debugType")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing debugType".to_string())?
                    .to_string();
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("debug_cfg_{}", debug_type));
                let trigger_kind = params
                    .get("triggerKind")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                let info = ExtDebugProviderInfo {
                    id: id.clone(),
                    debug_type: debug_type.clone(),
                    kind: "configuration".to_string(),
                    trigger_kind: Some(trigger_kind),
                };
                self.config_providers.write().insert(id.clone(), info);
                self.emit(ExtDebugEvent::ConfigurationProviderRegistered {
                    id: id.clone(),
                    debug_type,
                    trigger_kind,
                });
                Ok(json!({ "registered": true, "id": id }))
            }
            "debug.unregisterConfigurationProvider" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing provider id".to_string())?;
                let removed = self.config_providers.write().remove(id).is_some();
                if removed {
                    self.emit(ExtDebugEvent::ConfigurationProviderUnregistered {
                        id: id.to_string(),
                    });
                }
                Ok(json!({ "unregistered": removed, "id": id }))
            }
            "debug.registerAdapterDescriptorFactory" => {
                let debug_type = params
                    .get("debugType")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing debugType".to_string())?
                    .to_string();
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("debug_adapter_{}", debug_type));
                let info = ExtDebugProviderInfo {
                    id: id.clone(),
                    debug_type: debug_type.clone(),
                    kind: "adapter".to_string(),
                    trigger_kind: None,
                };
                self.adapter_factories.write().insert(id.clone(), info);
                self.emit(ExtDebugEvent::AdapterDescriptorFactoryRegistered {
                    id: id.clone(),
                    debug_type,
                });
                Ok(json!({ "registered": true, "id": id }))
            }
            "debug.unregisterAdapterDescriptorFactory" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing factory id".to_string())?;
                let removed = self.adapter_factories.write().remove(id).is_some();
                if removed {
                    self.emit(ExtDebugEvent::AdapterDescriptorFactoryUnregistered {
                        id: id.to_string(),
                    });
                }
                Ok(json!({ "unregistered": removed, "id": id }))
            }
            "debug.registerAdapterTrackerFactory" => {
                let debug_type = params
                    .get("debugType")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing debugType".to_string())?
                    .to_string();
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("debug_tracker_{}", debug_type));
                let info = ExtDebugProviderInfo {
                    id: id.clone(),
                    debug_type: debug_type.clone(),
                    kind: "tracker".to_string(),
                    trigger_kind: None,
                };
                self.tracker_factories.write().insert(id.clone(), info);
                self.emit(ExtDebugEvent::AdapterTrackerFactoryRegistered {
                    id: id.clone(),
                    debug_type,
                });
                Ok(json!({ "registered": true, "id": id }))
            }
            "debug.unregisterAdapterTrackerFactory" => {
                let id = params
                    .get("id")
                    .or_else(|| params.get("debugType"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing tracker id".to_string())?;
                let removed = {
                    let mut guard = self.tracker_factories.write();
                    if guard.remove(id).is_some() {
                        true
                    } else {
                        let matching: Vec<String> = guard
                            .iter()
                            .filter(|(_, v)| v.debug_type == id)
                            .map(|(k, _)| k.clone())
                            .collect();
                        let had = !matching.is_empty();
                        for k in matching {
                            guard.remove(&k);
                        }
                        had
                    }
                };
                if removed {
                    self.emit(ExtDebugEvent::AdapterTrackerFactoryUnregistered {
                        id: id.to_string(),
                    });
                }
                Ok(json!({ "unregistered": removed, "id": id }))
            }
            "debug.startDebugging" => {
                let config = params
                    .get("nameOrConfiguration")
                    .cloned()
                    .unwrap_or(Value::Null);
                let session_type = config
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("lldb")
                    .to_string();
                let name = config
                    .get("name")
                    .and_then(|v| v.as_str())
                    .or_else(|| config.as_str())
                    .unwrap_or("Debug Session")
                    .to_string();
                let session_id = params
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        let seq = self.next_session_seq.fetch_add(1, Ordering::SeqCst) + 1;
                        format!("dbg_session_{}", seq)
                    });
                let workspace_folder = params
                    .get("folder")
                    .and_then(|f| f.get("uri").or_else(|| f.as_str().map(|_| f)))
                    .and_then(|u| u.as_str())
                    .map(|s| s.to_string());

                let session = ExtDebugSessionState {
                    id: session_id.clone(),
                    session_type: session_type.clone(),
                    name: name.clone(),
                    workspace_folder,
                    configuration: config,
                    is_active: true,
                    is_terminated: false,
                    custom_events: Vec::new(),
                };

                {
                    let mut sessions = self.sessions.write();
                    for s in sessions.values_mut() {
                        s.is_active = false;
                    }
                    sessions.insert(session_id.clone(), session.clone());
                }
                *self.active_session_id.write() = Some(session_id.clone());

                self.emit(ExtDebugEvent::SessionStarted {
                    session_id,
                    session_type,
                    name,
                });
                Ok(json!({ "started": true, "session": session }))
            }
            "debug.stopDebugging" => {
                let target_id = params
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| self.active_session_id.read().clone());

                let Some(session_id) = target_id else {
                    return Ok(json!({ "stopped": false }));
                };

                let stopped = {
                    let mut sessions = self.sessions.write();
                    let mut active = self.active_session_id.write();
                    if let Some(s) = sessions.get_mut(&session_id) {
                        if s.is_terminated {
                            return Err(format!(
                                "Debug session '{}' is already terminated",
                                session_id
                            ));
                        }
                        s.is_active = false;
                        s.is_terminated = true;
                        if active.as_deref() == Some(&session_id) {
                            let fallback_id = sessions
                                .values()
                                .find(|other| !other.is_terminated)
                                .map(|other| other.id.clone());
                            if let Some(ref fb_id) = fallback_id {
                                if let Some(fb) = sessions.get_mut(fb_id) {
                                    fb.is_active = true;
                                }
                            }
                            *active = fallback_id;
                        }
                        true
                    } else {
                        false
                    }
                };

                if stopped {
                    self.emit(ExtDebugEvent::SessionStopped {
                        session_id: session_id.clone(),
                    });
                }
                Ok(json!({ "stopped": stopped, "sessionId": session_id }))
            }
            "debug.addBreakpoints" => {
                let raw_bps = match params.get("breakpoints") {
                    Some(v) if !v.is_null() => v
                        .as_array()
                        .ok_or_else(|| "Invalid breakpoints: expected an array".to_string())?
                        .clone(),
                    _ => Vec::new(),
                };
                let mut added_ids = Vec::new();
                {
                    let mut map = self.breakpoints.write();
                    let mut order = self.breakpoint_order.write();
                    for item in &raw_bps {
                        if let Some(bp) = parse_breakpoint_from_value(item) {
                            if !map.contains_key(&bp.id) {
                                order.push(bp.id.clone());
                            }
                            added_ids.push(bp.id.clone());
                            map.insert(bp.id.clone(), bp);
                        }
                    }
                }
                if !added_ids.is_empty() {
                    self.emit(ExtDebugEvent::BreakpointsAdded {
                        ids: added_ids.clone(),
                    });
                }
                Ok(json!({ "added": added_ids.len(), "ids": added_ids }))
            }
            "debug.removeBreakpoints" => {
                let ids: Vec<String> = match params.get("ids") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid breakpoint ids: {}", e))?,
                    _ => Vec::new(),
                };
                let mut removed = Vec::new();
                {
                    let mut map = self.breakpoints.write();
                    let mut order = self.breakpoint_order.write();
                    for id in &ids {
                        if map.remove(id).is_some() {
                            removed.push(id.clone());
                        }
                    }
                    order.retain(|id| !removed.contains(id));
                }
                if !removed.is_empty() {
                    self.emit(ExtDebugEvent::BreakpointsRemoved {
                        ids: removed.clone(),
                    });
                }
                Ok(json!({ "removed": removed.len(), "ids": removed }))
            }
            "debug.appendConsole" => {
                let value = params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let new_line = params
                    .get("newLine")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.console_entries.write().push(ExtDebugConsoleEntry {
                    value: value.clone(),
                    new_line,
                });
                self.emit(ExtDebugEvent::ConsoleAppended { value, new_line });
                Ok(json!({ "appended": true }))
            }
            "debug.sendCustomEvent" => {
                let session_id = params
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| self.active_session_id.read().clone())
                    .ok_or_else(|| "Missing sessionId for custom debug event".to_string())?;
                let event = params
                    .get("event")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing custom event name".to_string())?
                    .to_string();
                let body = params.get("body").cloned().unwrap_or(Value::Null);

                {
                    let mut sessions = self.sessions.write();
                    let session = sessions
                        .get_mut(&session_id)
                        .ok_or_else(|| format!("Debug session '{}' not found", session_id))?;
                    if session.is_terminated {
                        return Err(format!("Debug session '{}' is terminated", session_id));
                    }
                    session.custom_events.push(ExtDebugCustomEvent {
                        session_id: session_id.clone(),
                        event: event.clone(),
                        body: body.clone(),
                    });
                }

                self.emit(ExtDebugEvent::CustomEventReceived {
                    session_id,
                    event,
                    body,
                });
                Ok(json!({ "sent": true }))
            }
            _ => Err(format!("Unsupported debug RPC method: {}", method)),
        }
    }

    pub async fn request_provide_debug_configurations(
        &self,
        rpc_bridge: &RpcBridge,
        provider_id: &str,
        folder: Option<Value>,
    ) -> Result<Vec<Value>, String> {
        let (_req, rx) = rpc_bridge.create_request(
            "debug.provideDebugConfigurations",
            json!({
                "providerId": provider_id,
                "debugType": provider_id,
                "folder": folder,
            }),
        );

        let val = tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .map_err(|_| "Timed out waiting for provideDebugConfigurations".to_string())?
            .map_err(|e| format!("RPC channel closed: {}", e))??;

        Ok(val.as_array().cloned().unwrap_or_default())
    }

    pub async fn request_resolve_debug_configuration(
        &self,
        rpc_bridge: &RpcBridge,
        provider_id: &str,
        folder: Option<Value>,
        configuration: Value,
    ) -> Result<Value, String> {
        let (_req, rx) = rpc_bridge.create_request(
            "debug.resolveDebugConfiguration",
            json!({
                "providerId": provider_id,
                "debugType": provider_id,
                "folder": folder,
                "debugConfiguration": configuration,
            }),
        );

        tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .map_err(|_| "Timed out waiting for resolveDebugConfiguration".to_string())?
            .map_err(|e| format!("RPC channel closed: {}", e))?
    }

    pub async fn request_create_debug_adapter_descriptor(
        &self,
        rpc_bridge: &RpcBridge,
        provider_id: &str,
        session: Value,
    ) -> Result<Value, String> {
        let (_req, rx) = rpc_bridge.create_request(
            "debug.createDebugAdapterDescriptor",
            json!({
                "providerId": provider_id,
                "debugType": provider_id,
                "session": session,
            }),
        );

        tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .map_err(|_| "Timed out waiting for createDebugAdapterDescriptor".to_string())?
            .map_err(|e| format!("RPC channel closed: {}", e))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_debug_manager_lifecycle_and_dap_mapping() {
        let mgr = ExtHostDebugManager::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        mgr.on_debug_event(move |ev| {
            events_clone.lock().unwrap().push(ev.clone());
        });

        mgr.handle_rpc_call(
            "debug.registerConfigurationProvider",
            &json!({ "id": "cfg_lldb", "debugType": "lldb", "triggerKind": 1 }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "debug.registerAdapterDescriptorFactory",
            &json!({ "id": "adapter_lldb", "debugType": "lldb" }),
        )
        .unwrap();
        assert_eq!(mgr.list_configuration_providers().len(), 1);
        assert_eq!(mgr.list_adapter_factories().len(), 1);

        let started = mgr
            .handle_rpc_call(
                "debug.startDebugging",
                &json!({
                    "folder": { "uri": "file:///workspace" },
                    "nameOrConfiguration": { "type": "lldb", "name": "Launch Rust" }
                }),
            )
            .unwrap();
        let session_id = started["session"]["id"].as_str().unwrap().to_string();
        assert_eq!(mgr.active_session().unwrap().name, "Launch Rust");

        mgr.handle_rpc_call(
            "debug.addBreakpoints",
            &json!({
                "breakpoints": [{
                    "id": "bp_1",
                    "kind": "source",
                    "enabled": true,
                    "condition": "x > 5",
                    "location": {
                        "uri": "file:///workspace/src/main.rs",
                        "range": { "start": { "line": 9, "character": 4 } }
                    }
                }]
            }),
        )
        .unwrap();

        let dap_bps = mgr.to_dap_source_breakpoints("file:///workspace/src/main.rs");
        assert_eq!(dap_bps.len(), 1);
        assert_eq!(dap_bps[0].line, 10);
        assert_eq!(dap_bps[0].column, Some(5));
        assert_eq!(dap_bps[0].condition.as_deref(), Some("x > 5"));

        mgr.handle_rpc_call(
            "debug.appendConsole",
            &json!({ "value": "hit breakpoint", "newLine": true }),
        )
        .unwrap();
        assert_eq!(mgr.to_dap_output_events().len(), 1);

        mgr.handle_rpc_call(
            "debug.sendCustomEvent",
            &json!({ "sessionId": session_id, "event": "telemetry", "body": { "ok": true } }),
        )
        .unwrap();
        assert_eq!(mgr.active_session().unwrap().custom_events.len(), 1);

        mgr.handle_rpc_call(
            "debug.removeBreakpoints",
            &json!({ "ids": ["bp_1"] }),
        )
        .unwrap();
        assert!(mgr.list_breakpoints().is_empty());

        mgr.handle_rpc_call("debug.stopDebugging", &json!({ "sessionId": session_id }))
            .unwrap();
        assert!(mgr.active_session().is_none());

        mgr.handle_rpc_call(
            "debug.unregisterConfigurationProvider",
            &json!({ "id": "cfg_lldb" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "debug.unregisterAdapterDescriptorFactory",
            &json!({ "id": "adapter_lldb" }),
        )
        .unwrap();
        assert!(mgr.list_configuration_providers().is_empty());
        assert!(mgr.list_adapter_factories().is_empty());
        assert_eq!(events.lock().unwrap().len(), 10);
    }

    #[tokio::test]
    async fn test_debug_request_roundtrip() {
        let mgr = ExtHostDebugManager::new();
        let rpc = Arc::new(RpcBridge::new());
        let rpc_clone = rpc.clone();

        tokio::spawn(async move {
            tokio::task::yield_now().await;
            rpc_clone.handle_incoming_response(
                1,
                Some(json!({ "type": "lldb", "name": "Launch", "stopOnEntry": true })),
                None,
            );
        });

        let resolved = mgr
            .request_resolve_debug_configuration(
                &rpc,
                "lldb",
                None,
                json!({ "type": "lldb", "name": "Launch" }),
            )
            .await
            .unwrap();
        assert_eq!(resolved["stopOnEntry"], true);
    }

    #[test]
    fn test_debug_stop_debugging_listener_reentrancy_and_fallback_active() {
        let mgr = ExtHostDebugManager::new();
        let mgr_in_cb = mgr.clone();
        let seen_events = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = seen_events.clone();

        mgr.on_debug_event(move |ev| {
            let active = mgr_in_cb.active_session();
            let listed = mgr_in_cb.list_sessions();
            seen_clone
                .lock()
                .unwrap()
                .push((ev.clone(), active, listed.len()));
        });

        mgr.handle_rpc_call(
            "debug.startDebugging",
            &json!({
                "sessionId": "s1",
                "nameOrConfiguration": { "type": "lldb", "name": "First" }
            }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "debug.startDebugging",
            &json!({
                "sessionId": "s2",
                "nameOrConfiguration": { "type": "lldb", "name": "Second" }
            }),
        )
        .unwrap();

        let stop_res = mgr
            .handle_rpc_call("debug.stopDebugging", &json!({ "sessionId": "s2" }))
            .unwrap();
        assert_eq!(stop_res["stopped"], true);

        let fallback = mgr.active_session().expect("fallback session must be active");
        assert_eq!(fallback.id, "s1");
        assert!(fallback.is_active);
        assert!(!fallback.is_terminated);

        // Double-stopping an already terminated session must error and not emit duplicate event.
        assert!(mgr
            .handle_rpc_call("debug.stopDebugging", &json!({ "sessionId": "s2" }))
            .is_err());

        let log = seen_events.lock().unwrap();
        assert_eq!(log.len(), 3);
        assert!(matches!(log[2].0, ExtDebugEvent::SessionStopped { ref session_id } if session_id == "s2"));
        let cb_active = log[2].1.as_ref().expect("listener must observe fallback active session");
        assert_eq!(cb_active.id, "s1");
        assert!(cb_active.is_active);
        assert_eq!(log[2].2, 1);

        // Malformed breakpoints payload must return Err.
        assert!(mgr
            .handle_rpc_call("debug.addBreakpoints", &json!({ "breakpoints": "bad" }))
            .is_err());
        assert!(mgr
            .handle_rpc_call("debug.removeBreakpoints", &json!({ "ids": "bad" }))
            .is_err());
    }
}
