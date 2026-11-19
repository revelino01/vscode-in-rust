//! Extension Host Environment & Extensions Registry (`vscode.env`, `vscode.extensions`).

use crate::manifest::{ExtensionLoader, ExtensionSnapshot};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtEnvEvent {
    ClipboardWritten { text: String },
    ExternalUriOpened { uri: String },
    ExtensionsUpdated { count: usize },
}

pub type EnvEventListener = Arc<dyn Fn(&ExtEnvEvent) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostEnvManager {
    clipboard: Arc<RwLock<String>>,
    opened_external_uris: Arc<RwLock<Vec<String>>>,
    extensions: Arc<RwLock<Vec<ExtensionSnapshot>>>,
    pending_events: Arc<RwLock<Vec<ExtEnvEvent>>>,
    listeners: Arc<RwLock<Vec<EnvEventListener>>>,
    revision: Arc<AtomicU64>,
}

impl ExtHostEnvManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<ExtEnvEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_env_event<F>(&self, listener: F)
    where
        F: Fn(&ExtEnvEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    fn emit(&self, event: ExtEnvEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.listeners.read().clone();
        for listener in listeners {
            listener(&event);
        }
    }

    pub fn clipboard_read(&self) -> String {
        self.clipboard.read().clone()
    }

    pub fn clipboard_write(&self, value: impl Into<String>) {
        let text = value.into();
        *self.clipboard.write() = text.clone();
        self.emit(ExtEnvEvent::ClipboardWritten { text });
    }

    /// Validate and record an external URI offline without spawning any OS process.
    pub fn open_external(&self, raw_uri: &str) -> bool {
        if raw_uri.is_empty()
            || raw_uri
                .chars()
                .any(|c| c.is_ascii_control() || c.is_whitespace())
        {
            return false;
        }
        if !is_valid_external_uri(raw_uri) {
            return false;
        }
        let normalized = raw_uri.to_string();
        self.opened_external_uris.write().push(normalized.clone());
        self.emit(ExtEnvEvent::ExternalUriOpened { uri: normalized });
        true
    }

    pub fn opened_external_uris(&self) -> Vec<String> {
        self.opened_external_uris.read().clone()
    }

    pub fn clear_opened_external_uris(&self) {
        self.opened_external_uris.write().clear();
    }

    pub fn set_extensions(&self, snapshots: Vec<ExtensionSnapshot>) {
        let count = snapshots.len();
        *self.extensions.write() = snapshots;
        self.emit(ExtEnvEvent::ExtensionsUpdated { count });
    }

    pub fn hydrate_from_loader(&self, loader: &ExtensionLoader) {
        self.set_extensions(loader.snapshots());
    }

    pub fn list_extensions(&self) -> Vec<ExtensionSnapshot> {
        self.extensions.read().clone()
    }

    pub fn get_extension(&self, id: &str) -> Option<ExtensionSnapshot> {
        self.extensions
            .read()
            .iter()
            .find(|snap| snap.id.eq_ignore_ascii_case(id))
            .cloned()
    }

    pub fn activate_extension(&self, id: &str) -> Option<ExtensionSnapshot> {
        let mut guard = self.extensions.write();
        if let Some(snap) = guard
            .iter_mut()
            .find(|snap| snap.id.eq_ignore_ascii_case(id))
        {
            snap.is_active = true;
            return Some(snap.clone());
        }
        None
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "env.clipboardRead" => Ok(json!(self.clipboard_read())),
            "env.clipboardWrite" => {
                let value = params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'value' parameter".to_string())?;
                self.clipboard_write(value);
                Ok(json!({ "written": true }))
            }
            "env.openExternal" => {
                let uri = params
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let ok = self.open_external(uri);
                Ok(json!(ok))
            }
            "extensions.getAll" => {
                Ok(serde_json::to_value(self.list_extensions()).unwrap_or_else(|_| json!([])))
            }
            "extensions.getExtension" => {
                let id = params
                    .get("id")
                    .or_else(|| params.get("extensionId"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'id' parameter".to_string())?;
                Ok(self
                    .get_extension(id)
                    .and_then(|snap| serde_json::to_value(snap).ok())
                    .unwrap_or(Value::Null))
            }
            "extensions.activate" => {
                let id = params
                    .get("id")
                    .or_else(|| params.get("extensionId"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'id' parameter".to_string())?;
                let activated = self.activate_extension(id).is_some();
                Ok(json!({ "activated": activated, "id": id }))
            }
            _ => Err(format!("Unsupported env/extensions RPC method: {}", method)),
        }
    }
}

fn is_valid_external_uri(uri: &str) -> bool {
    if uri.is_empty() {
        return false;
    }
    if uri.chars().any(|c| c.is_ascii_control() || c.is_whitespace()) {
        return false;
    }
    let Some(colon_idx) = uri.find(':') else {
        return false;
    };
    let scheme = &uri[..colon_idx];
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return false;
    }
    let scheme_lower = scheme.to_ascii_lowercase();
    let rest = &uri[colon_idx + 1..];
    match scheme_lower.as_str() {
        "http" | "https" => {
            let Some(after_slashes) = rest.strip_prefix("//") else {
                return false;
            };
            let host = after_slashes
                .split(['/', '?', '#'])
                .next()
                .unwrap_or("");
            !host.is_empty()
        }
        "file" | "vscode" | "rustcode" => {
            let Some(after_slashes) = rest.strip_prefix("//") else {
                return false;
            };
            !after_slashes.is_empty()
        }
        "mailto" => !rest.is_empty() && rest.contains('@') && !rest.starts_with('@') && !rest.ends_with('@'),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_clipboard_and_open_external_validation() {
        let mgr = ExtHostEnvManager::new();
        assert_eq!(
            mgr.handle_rpc_call("env.clipboardRead", &json!({})).unwrap(),
            json!("")
        );

        assert!(mgr
            .handle_rpc_call("env.clipboardWrite", &json!({}))
            .is_err());

        let wr = mgr
            .handle_rpc_call("env.clipboardWrite", &json!({ "value": "rustcode-clip" }))
            .unwrap();
        assert_eq!(wr, json!({ "written": true }));
        assert_eq!(
            mgr.handle_rpc_call("env.clipboardRead", &json!({})).unwrap(),
            json!("rustcode-clip")
        );

        // Allowlisted schemes succeed offline without spawning any process
        for valid in [
            "https://example.com/docs?q=1",
            "http://localhost:8080/health",
            "mailto:dev@rustcode.dev",
            "file:///workspace/README.md",
            "vscode://settings/editor.fontSize",
            "rustcode://extension/demo",
        ] {
            assert_eq!(
                mgr.handle_rpc_call("env.openExternal", &json!({ "uri": valid }))
                    .unwrap(),
                json!(true),
                "expected valid URI: {}",
                valid
            );
        }

        // Unsafe / malformed URIs return false and are NOT recorded
        for invalid in [
            "",
            "   ",
            "javascript:alert(1)",
            "data:text/html,<script>1</script>",
            "vbscript:msgbox",
            "not-a-uri",
            "https://example.com/has space",
            "https://example.com/\nnewline",
            "https://example.com\r\n",
            "https://example.com\n",
            "\thttps://example.com",
            "\u{000b}https://example.com",
            " https://example.com ",
            "https://",
            "mailto:invalid-email",
        ] {
            assert_eq!(
                mgr.handle_rpc_call("env.openExternal", &json!({ "uri": invalid }))
                    .unwrap(),
                json!(false),
                "expected invalid URI to be rejected: {:?}",
                invalid
            );
        }

        assert_eq!(mgr.opened_external_uris().len(), 6);
    }

    #[test]
    fn test_env_extensions_get_all_and_get_extension() {
        let mgr = ExtHostEnvManager::new();
        mgr.set_extensions(vec![ExtensionSnapshot {
            id: "RustCode.sample-ext".to_string(),
            extension_path: "/ext/sample".to_string(),
            is_active: false,
            package_json: json!({ "name": "sample-ext", "publisher": "RustCode" }),
            extension_kind: 1,
        }]);

        let all = mgr
            .handle_rpc_call("extensions.getAll", &json!({}))
            .unwrap();
        assert_eq!(all.as_array().unwrap().len(), 1);
        assert_eq!(all[0]["id"], "RustCode.sample-ext");

        // Case-insensitive lookup
        let found = mgr
            .handle_rpc_call(
                "extensions.getExtension",
                &json!({ "id": "rustcode.sample-ext" }),
            )
            .unwrap();
        assert_eq!(found["id"], "RustCode.sample-ext");
        assert_eq!(found["isActive"], false);

        // Activate extension
        let act = mgr
            .handle_rpc_call(
                "extensions.activate",
                &json!({ "extensionId": "rustcode.sample-ext" }),
            )
            .unwrap();
        assert_eq!(act["activated"], true);
        assert!(mgr.get_extension("RustCode.sample-ext").unwrap().is_active);

        // Missing extension returns Null
        let missing = mgr
            .handle_rpc_call("extensions.getExtension", &json!({ "id": "no.such.ext" }))
            .unwrap();
        assert_eq!(missing, Value::Null);

        // Missing param returns Err
        assert!(mgr
            .handle_rpc_call("extensions.getExtension", &json!({}))
            .is_err());
    }
}
