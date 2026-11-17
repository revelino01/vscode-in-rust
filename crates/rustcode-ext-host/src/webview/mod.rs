//! Extension Host Webview management and bidirectional message bridge.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
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

#[derive(Clone, Default)]
pub struct ExtHostWebviewManager {
    panels: Arc<RwLock<HashMap<String, WebviewPanelState>>>,
    // Listeners for messages arriving from webview into ext-host
    inbound_listeners: Arc<RwLock<HashMap<String, Vec<WebviewMessageListener>>>>,
    // Listeners for messages posted from ext-host out to webview
    outbound_listeners: Arc<RwLock<HashMap<String, Vec<WebviewMessageListener>>>>,
}

impl ExtHostWebviewManager {
    pub fn new() -> Self {
        Self {
            panels: Arc::new(RwLock::new(HashMap::new())),
            inbound_listeners: Arc::new(RwLock::new(HashMap::new())),
            outbound_listeners: Arc::new(RwLock::new(HashMap::new())),
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
        state
    }

    pub fn dispose_panel(&self, id: &str) -> bool {
        let mut guard = self.panels.write();
        if let Some(panel) = guard.get_mut(id) {
            panel.is_disposed = true;
            true
        } else {
            false
        }
    }

    pub fn set_html(&self, id: &str, html: &str) -> bool {
        let mut guard = self.panels.write();
        if let Some(panel) = guard.get_mut(id) {
            panel.html = html.to_string();
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
                    .ok_or("Missing id")?;
                let view_type = params
                    .get("viewType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("customWebview");
                let title = params
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Webview");
                let options: WebviewOptions = params
                    .get("options")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

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
                let ok = self.set_html(id, html);
                Ok(Value::Bool(ok))
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
                let ok = self.dispose_panel(id);
                Ok(Value::Bool(ok))
            }
            _ => Err(format!("Unknown webview RPC method '{}'", method)),
        }
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
}
