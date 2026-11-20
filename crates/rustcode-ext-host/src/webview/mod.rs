//! Extension Host Webview and Custom Editor management and bidirectional message bridge.

use crate::rpc::RpcBridge;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebviewOptions {
    #[serde(default)]
    pub enable_scripts: bool,
    #[serde(default)]
    pub local_resource_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebviewPanelState {
    pub id: String,
    pub view_type: String,
    pub title: String,
    pub html: String,
    pub options: WebviewOptions,
    pub is_disposed: bool,
}

pub type WebviewMessageListener = Arc<dyn Fn(Value) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WebviewEvent {
    PanelCreated {
        id: String,
        view_type: String,
        title: String,
    },
    HtmlUpdated {
        id: String,
        html: String,
    },
    PanelDisposed {
        id: String,
    },
    PanelRevealed {
        id: String,
        view_column: Option<i32>,
    },
    MessagePosted {
        id: String,
        message: Value,
    },
}

pub type WebviewEventListener = Arc<dyn Fn(&WebviewEvent) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostWebviewManager {
    panels: Arc<RwLock<HashMap<String, WebviewPanelState>>>,
    // Listeners for messages arriving from webview into ext-host
    inbound_listeners: Arc<RwLock<HashMap<String, Vec<WebviewMessageListener>>>>,
    // Listeners for messages posted from ext-host out to webview
    outbound_listeners: Arc<RwLock<HashMap<String, Vec<WebviewMessageListener>>>>,
    event_listeners: Arc<RwLock<Vec<WebviewEventListener>>>,
    pending_events: Arc<RwLock<Vec<WebviewEvent>>>,
    revision: Arc<AtomicU64>,
}

impl ExtHostWebviewManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<WebviewEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_webview_event<F>(&self, listener: F)
    where
        F: Fn(&WebviewEvent) + Send + Sync + 'static,
    {
        self.event_listeners.write().push(Arc::new(listener));
    }

    fn emit_event(&self, event: WebviewEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.event_listeners.read().clone();
        for listener in listeners {
            listener(&event);
        }
    }

    pub fn create_panel(
        &self,
        id: &str,
        view_type: &str,
        title: &str,
        options: WebviewOptions,
    ) -> WebviewPanelState {
        let state = WebviewPanelState {
            id: id.to_string(),
            view_type: view_type.to_string(),
            title: title.to_string(),
            html: String::new(),
            options,
            is_disposed: false,
        };
        self.panels.write().insert(id.to_string(), state.clone());
        self.emit_event(WebviewEvent::PanelCreated {
            id: id.to_string(),
            view_type: view_type.to_string(),
            title: title.to_string(),
        });
        state
    }

    pub fn dispose_panel(&self, id: &str) -> bool {
        let mut guard = self.panels.write();
        if let Some(panel) = guard.get_mut(id) {
            if panel.is_disposed {
                return false;
            }
            panel.is_disposed = true;
            drop(guard);
            self.emit_event(WebviewEvent::PanelDisposed { id: id.to_string() });
            true
        } else {
            false
        }
    }

    pub fn set_html(&self, id: &str, html: &str) -> bool {
        let mut guard = self.panels.write();
        if let Some(panel) = guard.get_mut(id) {
            if panel.is_disposed {
                return false;
            }
            panel.html = html.to_string();
            drop(guard);
            self.emit_event(WebviewEvent::HtmlUpdated {
                id: id.to_string(),
                html: html.to_string(),
            });
            true
        } else {
            false
        }
    }

    pub fn get_panel(&self, id: &str) -> Option<WebviewPanelState> {
        self.panels.read().get(id).cloned()
    }

    pub fn list_active_panels(&self) -> Vec<WebviewPanelState> {
        self.panels
            .read()
            .values()
            .filter(|p| !p.is_disposed)
            .cloned()
            .collect()
    }

    /// Ext-host posts a message to the Webview.
    pub fn post_message_to_webview(&self, id: &str, message: Value) -> Result<(), String> {
        let guard = self.panels.read();
        let panel = guard.get(id).ok_or_else(|| format!("Webview panel '{}' not found", id))?;
        if panel.is_disposed {
            return Err(format!("Webview panel '{}' is disposed", id));
        }
        drop(guard);

        let listeners = self.outbound_listeners.read();
        if let Some(cbs) = listeners.get(id) {
            for cb in cbs {
                cb(message.clone());
            }
        }
        drop(listeners);
        self.emit_event(WebviewEvent::MessagePosted {
            id: id.to_string(),
            message,
        });
        Ok(())
    }

    /// Webview receives a message and sends it to the Ext-host.
    pub fn receive_message_from_webview(&self, id: &str, message: Value) -> Result<(), String> {
        let guard = self.panels.read();
        let panel = guard.get(id).ok_or_else(|| format!("Webview panel '{}' not found", id))?;
        if panel.is_disposed {
            return Err(format!("Webview panel '{}' is disposed", id));
        }
        drop(guard);

        let listeners = self.inbound_listeners.read();
        if let Some(cbs) = listeners.get(id) {
            for cb in cbs {
                cb(message.clone());
            }
        }
        Ok(())
    }

    /// Subscribe to messages coming from webview to ext-host.
    pub fn on_did_receive_message<F>(&self, id: &str, listener: F)
    where
        F: Fn(Value) + Send + Sync + 'static,
    {
        self.inbound_listeners
            .write()
            .entry(id.to_string())
            .or_default()
            .push(Arc::new(listener));
    }

    /// Subscribe to messages going out from ext-host to webview.
    pub fn on_post_message<F>(&self, id: &str, listener: F)
    where
        F: Fn(Value) + Send + Sync + 'static,
    {
        self.outbound_listeners
            .write()
            .entry(id.to_string())
            .or_default()
            .push(Arc::new(listener));
    }

    /// Dispatch RPC calls from JavaScript ext-host bridge.
    pub fn handle_rpc_call(&self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "window.createWebviewPanel" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("Missing or empty id")?;
                let view_type = params
                    .get("viewType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("customWebview");
                let title = params
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Webview");
                let options: WebviewOptions = match params.get("options") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid webview options: {}", e))?,
                    _ => WebviewOptions::default(),
                };

                let state = self.create_panel(id, view_type, title, options);
                serde_json::to_value(state).map_err(|e| e.to_string())
            }
            "window.setWebviewHtml" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing id")?;
                let html = params
                    .get("html")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing html")?;
                {
                    let mut guard = self.panels.write();
                    let panel = guard
                        .get_mut(id)
                        .ok_or_else(|| format!("Webview panel '{}' not found", id))?;
                    if panel.is_disposed {
                        return Err(format!("Webview panel '{}' is disposed", id));
                    }
                    panel.html = html.to_string();
                }
                self.emit_event(WebviewEvent::HtmlUpdated {
                    id: id.to_string(),
                    html: html.to_string(),
                });
                Ok(Value::Bool(true))
            }
            "window.postMessageToWebview" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing id")?;
                let message = params
                    .get("message")
                    .cloned()
                    .unwrap_or(Value::Null);
                self.post_message_to_webview(id, message)?;
                Ok(Value::Bool(true))
            }
            "window.disposeWebviewPanel" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing id")?;
                {
                    let mut guard = self.panels.write();
                    let panel = guard
                        .get_mut(id)
                        .ok_or_else(|| format!("Webview panel '{}' not found", id))?;
                    if panel.is_disposed {
                        return Err(format!("Webview panel '{}' is disposed", id));
                    }
                    panel.is_disposed = true;
                }
                self.emit_event(WebviewEvent::PanelDisposed { id: id.to_string() });
                Ok(Value::Bool(true))
            }
            "window.revealWebviewPanel" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing id")?;
                let view_column = params
                    .get("viewColumn")
                    .and_then(|v| v.as_i64())
                    .map(|c| c as i32);
                let guard = self.panels.read();
                let panel = guard
                    .get(id)
                    .ok_or_else(|| format!("Webview panel '{}' not found", id))?;
                if panel.is_disposed {
                    return Err(format!("Webview panel '{}' is disposed", id));
                }
                drop(guard);
                self.emit_event(WebviewEvent::PanelRevealed {
                    id: id.to_string(),
                    view_column,
                });
                Ok(json!({ "revealed": true, "id": id }))
            }
            _ => Err(format!("Unknown webview RPC method '{}'", method)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomEditorProviderInfo {
    pub view_type: String,
    pub webview_options: WebviewOptions,
    pub supports_multiple_editors_per_document: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomEditorSessionState {
    pub id: String,
    pub view_type: String,
    pub uri: String,
    pub title: String,
    pub panel_id: String,
    pub is_closed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CustomEditorEvent {
    ProviderRegistered {
        view_type: String,
    },
    ProviderUnregistered {
        view_type: String,
    },
    SessionOpened {
        id: String,
        view_type: String,
        uri: String,
        panel_id: String,
    },
    SessionClosed {
        id: String,
    },
}

pub type CustomEditorEventListener = Arc<dyn Fn(&CustomEditorEvent) + Send + Sync>;

#[derive(Clone)]
pub struct ExtHostCustomEditorManager {
    providers: Arc<RwLock<HashMap<String, CustomEditorProviderInfo>>>,
    sessions: Arc<RwLock<HashMap<String, CustomEditorSessionState>>>,
    listeners: Arc<RwLock<Vec<CustomEditorEventListener>>>,
    pending_events: Arc<RwLock<Vec<CustomEditorEvent>>>,
    revision: Arc<AtomicU64>,
    next_session_id: Arc<AtomicU64>,
}

impl Default for ExtHostCustomEditorManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtHostCustomEditorManager {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            listeners: Arc::new(RwLock::new(Vec::new())),
            pending_events: Arc::new(RwLock::new(Vec::new())),
            revision: Arc::new(AtomicU64::new(0)),
            next_session_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<CustomEditorEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_custom_editor_event<F>(&self, listener: F)
    where
        F: Fn(&CustomEditorEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    fn emit(&self, event: CustomEditorEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.listeners.read().clone();
        for listener in listeners {
            listener(&event);
        }
    }

    pub fn register_provider(
        &self,
        view_type: &str,
        webview_options: WebviewOptions,
        supports_multiple_editors_per_document: bool,
    ) -> CustomEditorProviderInfo {
        let info = CustomEditorProviderInfo {
            view_type: view_type.to_string(),
            webview_options,
            supports_multiple_editors_per_document,
        };
        self.providers
            .write()
            .insert(view_type.to_string(), info.clone());
        self.emit(CustomEditorEvent::ProviderRegistered {
            view_type: view_type.to_string(),
        });
        info
    }

    pub fn unregister_provider(&self, view_type: &str) -> bool {
        let removed = self.providers.write().remove(view_type).is_some();
        if removed {
            self.emit(CustomEditorEvent::ProviderUnregistered {
                view_type: view_type.to_string(),
            });
        }
        removed
    }

    pub fn get_provider(&self, view_type: &str) -> Option<CustomEditorProviderInfo> {
        self.providers.read().get(view_type).cloned()
    }

    pub fn list_providers(&self) -> Vec<CustomEditorProviderInfo> {
        self.providers.read().values().cloned().collect()
    }

    pub fn open_session(
        &self,
        id: Option<&str>,
        view_type: &str,
        uri: &str,
        title: Option<&str>,
        webview_manager: Option<&ExtHostWebviewManager>,
    ) -> CustomEditorSessionState {
        let session_id = id
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                let seq = self.next_session_id.fetch_add(1, Ordering::SeqCst);
                format!("custom_editor_{}", seq)
            });
        let panel_id = session_id.clone();
        let resolved_title = title
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                uri.rsplit(['/', '\\'])
                    .next()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(view_type)
                    .to_string()
            });
        let options = self
            .get_provider(view_type)
            .map(|p| p.webview_options)
            .unwrap_or_default();

        if let Some(wv) = webview_manager {
            if wv.get_panel(&panel_id).is_none_or(|p| p.is_disposed) {
                wv.create_panel(&panel_id, view_type, &resolved_title, options);
            }
        }

        let session = CustomEditorSessionState {
            id: session_id.clone(),
            view_type: view_type.to_string(),
            uri: uri.to_string(),
            title: resolved_title,
            panel_id: panel_id.clone(),
            is_closed: false,
        };
        self.sessions.write().insert(session_id.clone(), session.clone());
        self.emit(CustomEditorEvent::SessionOpened {
            id: session_id,
            view_type: view_type.to_string(),
            uri: uri.to_string(),
            panel_id,
        });
        session
    }

    pub fn close_session(
        &self,
        id: &str,
        webview_manager: Option<&ExtHostWebviewManager>,
    ) -> bool {
        let mut sessions = self.sessions.write();
        if let Some(session) = sessions.get_mut(id) {
            if session.is_closed {
                return false;
            }
            session.is_closed = true;
            let panel_id = session.panel_id.clone();
            drop(sessions);
            if let Some(wv) = webview_manager {
                wv.dispose_panel(&panel_id);
            }
            self.emit(CustomEditorEvent::SessionClosed { id: id.to_string() });
            true
        } else {
            false
        }
    }

    pub fn get_session(&self, id: &str) -> Option<CustomEditorSessionState> {
        self.sessions.read().get(id).cloned()
    }

    pub fn find_session_by_uri(
        &self,
        uri: &str,
        view_type: Option<&str>,
    ) -> Option<CustomEditorSessionState> {
        self.sessions
            .read()
            .values()
            .find(|s| {
                !s.is_closed
                    && s.uri == uri
                    && view_type.is_none_or(|vt| s.view_type == vt)
            })
            .cloned()
    }

    pub fn find_session_by_panel_id(&self, panel_id: &str) -> Option<CustomEditorSessionState> {
        self.sessions
            .read()
            .values()
            .find(|s| !s.is_closed && s.panel_id == panel_id)
            .cloned()
    }

    pub fn list_active_sessions(&self) -> Vec<CustomEditorSessionState> {
        self.sessions
            .read()
            .values()
            .filter(|s| !s.is_closed)
            .cloned()
            .collect()
    }

    pub fn handle_rpc_call(
        &self,
        method: &str,
        params: &Value,
        webview_manager: Option<&ExtHostWebviewManager>,
    ) -> Result<Value, String> {
        match method {
            "window.registerCustomEditorProvider" => {
                let view_type = params
                    .get("viewType")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("Missing or empty viewType")?;
                let opts = match params.get("options") {
                    Some(v) if !v.is_null() => {
                        if !v.is_object() {
                            return Err("Invalid custom editor options: expected object".to_string());
                        }
                        v
                    }
                    _ => &Value::Null,
                };
                let webview_options: WebviewOptions = match opts.get("webviewOptions") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid webviewOptions: {}", e))?,
                    _ => WebviewOptions::default(),
                };
                let supports_multiple = opts
                    .get("supportsMultipleEditorsPerDocument")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let info = self.register_provider(view_type, webview_options, supports_multiple);
                serde_json::to_value(info).map_err(|e| e.to_string())
            }
            "window.unregisterCustomEditorProvider" => {
                let view_type = params
                    .get("viewType")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("Missing or empty viewType")?;
                let ok = self.unregister_provider(view_type);
                Ok(json!({ "unregistered": ok, "viewType": view_type }))
            }
            "window.openCustomEditor" => {
                let view_type = params
                    .get("viewType")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("Missing or empty viewType")?;
                let uri = params
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("Missing or empty uri")?;
                let id = params.get("id").and_then(|v| v.as_str());
                let title = params.get("title").and_then(|v| v.as_str());
                let session = self.open_session(id, view_type, uri, title, webview_manager);
                serde_json::to_value(session).map_err(|e| e.to_string())
            }
            _ => Err(format!("Unknown custom editor RPC method '{}'", method)),
        }
    }

    pub async fn request_resolve_custom_editor(
        &self,
        rpc_bridge: &RpcBridge,
        view_type: &str,
        uri: &str,
        panel_id: &str,
        title: &str,
        text: Option<&str>,
    ) -> Result<Value, String> {
        rpc_bridge
            .request(
                "window.resolveCustomEditor",
                json!({
                    "providerId": view_type,
                    "viewType": view_type,
                    "uri": uri,
                    "panelId": panel_id,
                    "title": title,
                    "text": text,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'window.resolveCustomEditor' timed out") {
                    "resolveCustomEditor timed out".to_string()
                } else if e == "RPC response channel closed" {
                    "resolveCustomEditor channel dropped".to_string()
                } else {
                    format!("resolveCustomEditor RPC error: {}", e)
                }
            })
    }

    pub async fn request_open_custom_document(
        &self,
        rpc_bridge: &RpcBridge,
        view_type: &str,
        uri: &str,
        backup_id: Option<&str>,
    ) -> Result<Value, String> {
        rpc_bridge
            .request(
                "CustomEditor.openCustomDocument",
                json!({
                    "providerId": view_type,
                    "viewType": view_type,
                    "uri": uri,
                    "openContext": {
                        "backupId": backup_id,
                    },
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'CustomEditor.openCustomDocument' timed out") {
                    "openCustomDocument timed out".to_string()
                } else if e == "RPC response channel closed" {
                    "openCustomDocument channel dropped".to_string()
                } else {
                    format!("openCustomDocument RPC error: {}", e)
                }
            })
    }

    pub async fn request_save_custom_document(
        &self,
        rpc_bridge: &RpcBridge,
        view_type: &str,
        uri: &str,
    ) -> Result<Value, String> {
        rpc_bridge
            .request(
                "CustomEditor.saveCustomDocument",
                json!({
                    "providerId": view_type,
                    "viewType": view_type,
                    "uri": uri,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'CustomEditor.saveCustomDocument' timed out") {
                    "saveCustomDocument timed out".to_string()
                } else if e == "RPC response channel closed" {
                    "saveCustomDocument channel dropped".to_string()
                } else {
                    format!("saveCustomDocument RPC error: {}", e)
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_ext_host_webview_lifecycle_and_messaging() {
        let manager = ExtHostWebviewManager::new();

        // 1. Create panel
        let panel = manager.create_panel(
            "webview_1",
            "catCoding",
            "Cat Coding",
            WebviewOptions {
                enable_scripts: true,
                local_resource_roots: vec![],
            },
        );
        assert_eq!(panel.id, "webview_1");
        assert_eq!(panel.title, "Cat Coding");

        // 2. Set HTML
        assert!(manager.set_html("webview_1", "<h1>Hello Webview</h1>"));
        let state = manager.get_panel("webview_1").unwrap();
        assert_eq!(state.html, "<h1>Hello Webview</h1>");

        // 3. Bidirectional Messaging
        let received_by_webview = Arc::new(AtomicUsize::new(0));
        let r_clone = received_by_webview.clone();
        manager.on_post_message("webview_1", move |msg| {
            if msg["command"] == "alert" {
                r_clone.fetch_add(1, Ordering::SeqCst);
            }
        });

        let received_by_ext_host = Arc::new(AtomicUsize::new(0));
        let e_clone = received_by_ext_host.clone();
        manager.on_did_receive_message("webview_1", move |msg| {
            if msg["text"] == "meow" {
                e_clone.fetch_add(1, Ordering::SeqCst);
            }
        });

        // Ext-host -> Webview
        manager
            .post_message_to_webview("webview_1", json!({ "command": "alert" }))
            .unwrap();
        assert_eq!(received_by_webview.load(Ordering::SeqCst), 1);

        // Webview -> Ext-host
        manager
            .receive_message_from_webview("webview_1", json!({ "text": "meow" }))
            .unwrap();
        assert_eq!(received_by_ext_host.load(Ordering::SeqCst), 1);

        // 4. Dispose panel
        assert!(manager.dispose_panel("webview_1"));
        let state = manager.get_panel("webview_1").unwrap();
        assert!(state.is_disposed);

        // Messaging to disposed panel fails
        assert!(manager
            .post_message_to_webview("webview_1", json!({ "command": "alert" }))
            .is_err());
    }

    #[test]
    fn test_ext_host_webview_events_and_custom_editor_manager() {
        let wv = ExtHostWebviewManager::new();
        let ce = ExtHostCustomEditorManager::new();

        let info = ce
            .handle_rpc_call(
                "window.registerCustomEditorProvider",
                &json!({
                    "viewType": "catEdit.catScratch",
                    "options": {
                        "webviewOptions": { "enableScripts": true },
                        "supportsMultipleEditorsPerDocument": true
                    }
                }),
                Some(&wv),
            )
            .unwrap();
        assert_eq!(info["viewType"], "catEdit.catScratch");
        assert_eq!(ce.list_providers().len(), 1);

        let session_val = ce
            .handle_rpc_call(
                "window.openCustomEditor",
                &json!({
                    "id": "ce_panel_1",
                    "viewType": "catEdit.catScratch",
                    "uri": "file:///demo.cscratch",
                    "title": "demo.cscratch"
                }),
                Some(&wv),
            )
            .unwrap();
        assert_eq!(session_val["panelId"], "ce_panel_1");
        assert!(wv.get_panel("ce_panel_1").is_some());
        assert_eq!(ce.find_session_by_uri("file:///demo.cscratch", None).unwrap().id, "ce_panel_1");

        let revealed = wv
            .handle_rpc_call(
                "window.revealWebviewPanel",
                json!({ "id": "ce_panel_1", "viewColumn": 2 }),
            )
            .unwrap();
        assert_eq!(revealed["revealed"], true);
        assert!(wv.revision() >= 2);
        assert!(ce.revision() >= 2);
        assert!(!wv.take_pending_events().is_empty());
        assert!(!ce.take_pending_events().is_empty());

        assert!(ce.close_session("ce_panel_1", Some(&wv)));
        assert!(wv.get_panel("ce_panel_1").unwrap().is_disposed);
        assert!(ce.list_active_sessions().is_empty());

        let unreg = ce
            .handle_rpc_call(
                "window.unregisterCustomEditorProvider",
                &json!({ "viewType": "catEdit.catScratch" }),
                Some(&wv),
            )
            .unwrap();
        assert_eq!(unreg["unregistered"], true);
    }

    #[test]
    fn test_disposed_panel_guards_and_empty_input_rejection() {
        let wv = ExtHostWebviewManager::new();
        let ce = ExtHostCustomEditorManager::new();

        // Empty and whitespace-only id / viewType rejected
        assert!(wv
            .handle_rpc_call(
                "window.createWebviewPanel",
                json!({ "id": "", "viewType": "v", "title": "t" })
            )
            .is_err());
        assert!(wv
            .handle_rpc_call(
                "window.createWebviewPanel",
                json!({ "id": "   ", "viewType": "v", "title": "t" })
            )
            .is_err());
        assert!(ce
            .handle_rpc_call(
                "window.registerCustomEditorProvider",
                &json!({ "viewType": "" }),
                Some(&wv),
            )
            .is_err());
        assert!(ce
            .handle_rpc_call(
                "window.registerCustomEditorProvider",
                &json!({ "viewType": "   " }),
                Some(&wv),
            )
            .is_err());

        // Disposed panel guards for set_html and double dispose_panel
        wv.create_panel("p_guard", "cat", "Cat", WebviewOptions::default());
        assert!(wv.set_html("p_guard", "<h1>Alive</h1>"));
        let _ = wv.take_pending_events();

        assert!(wv.dispose_panel("p_guard"));
        assert!(!wv.dispose_panel("p_guard"));
        assert!(!wv.set_html("p_guard", "<h1>Zombie</h1>"));
        assert!(wv
            .handle_rpc_call(
                "window.setWebviewHtml",
                json!({ "id": "p_guard", "html": "<h1>Zombie</h1>" })
            )
            .is_err());
        assert!(wv
            .handle_rpc_call(
                "window.disposeWebviewPanel",
                json!({ "id": "p_guard" })
            )
            .is_err());
        assert!(wv
            .handle_rpc_call(
                "window.setWebviewHtml",
                json!({ "id": "missing_panel", "html": "<h1>Missing</h1>" })
            )
            .is_err());
        assert!(wv
            .handle_rpc_call(
                "window.disposeWebviewPanel",
                json!({ "id": "missing_panel" })
            )
            .is_err());
        assert_eq!(wv.get_panel("p_guard").unwrap().html, "<h1>Alive</h1>");

        let events = wv.take_pending_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], WebviewEvent::PanelDisposed { id } if id == "p_guard"));
    }
}
