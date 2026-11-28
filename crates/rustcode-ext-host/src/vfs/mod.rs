//! Virtual File System & Read-Only Text Document Content Provider manager
//! (`vscode.workspace.registerFileSystemProvider` / `registerTextDocumentContentProvider`).

use crate::rpc::RpcBridge;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredFileSystemProvider {
    pub scheme: String,
    pub is_case_sensitive: bool,
    pub is_readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VfsFileChange {
    /// 1 = Changed, 2 = Created, 3 = Deleted
    #[serde(rename = "type")]
    pub change_type: u8,
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum VfsEvent {
    FileSystemRegistered { scheme: String, is_readonly: bool },
    FileSystemUnregistered { scheme: String },
    TextContentRegistered { scheme: String },
    TextContentUnregistered { scheme: String },
    FilesChanged { scheme: String, changes: Vec<VfsFileChange> },
    TextDocumentContentChanged { uri: String },
}

pub type VfsEventListener = Arc<dyn Fn(&VfsEvent) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostVfsManager {
    fs_providers: Arc<RwLock<HashMap<String, RegisteredFileSystemProvider>>>,
    content_providers: Arc<RwLock<HashMap<String, String>>>,
    listeners: Arc<RwLock<Vec<VfsEventListener>>>,
    pending_events: Arc<RwLock<Vec<VfsEvent>>>,
    revision: Arc<AtomicU64>,
}

/// Extract URI scheme (`git`, `untitled`, `vscode-userdata`, etc.) while ignoring Windows
/// single-letter drive paths (`C:\foo` or `C:/foo`).
pub fn extract_uri_scheme(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let colon_idx = trimmed.find(':')?;
    if colon_idx == 1 && trimmed.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    let scheme = &trimmed[..colon_idx];
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
    {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

impl ExtHostVfsManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<VfsEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_vfs_event<F>(&self, listener: F)
    where
        F: Fn(&VfsEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    fn emit(&self, ev: VfsEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(ev.clone());
        let listeners = self.listeners.read().clone();
        for cb in listeners {
            cb(&ev);
        }
    }

    pub fn register_file_system_provider(
        &self,
        scheme: impl Into<String>,
        is_case_sensitive: bool,
        is_readonly: bool,
    ) -> Result<RegisteredFileSystemProvider, String> {
        let raw = scheme.into();
        let norm = raw.trim().to_ascii_lowercase();
        if norm.is_empty() || norm == "file" {
            return Err(format!("Cannot register FileSystemProvider for scheme '{}'", raw));
        }
        let info = RegisteredFileSystemProvider {
            scheme: norm.clone(),
            is_case_sensitive,
            is_readonly,
        };
        self.fs_providers.write().insert(norm.clone(), info.clone());
        self.emit(VfsEvent::FileSystemRegistered {
            scheme: norm,
            is_readonly,
        });
        Ok(info)
    }

    pub fn unregister_file_system_provider(&self, scheme: &str) -> bool {
        let norm = scheme.trim().to_ascii_lowercase();
        let removed = self.fs_providers.write().remove(&norm).is_some();
        if removed {
            self.emit(VfsEvent::FileSystemUnregistered { scheme: norm });
        }
        removed
    }

    pub fn has_file_system_scheme(&self, uri_or_scheme: &str) -> bool {
        let key = extract_uri_scheme(uri_or_scheme)
            .unwrap_or_else(|| uri_or_scheme.trim().to_ascii_lowercase());
        self.fs_providers.read().contains_key(&key)
    }

    pub fn get_file_system_provider(&self, scheme: &str) -> Option<RegisteredFileSystemProvider> {
        self.fs_providers
            .read()
            .get(&scheme.trim().to_ascii_lowercase())
            .cloned()
    }

    pub fn list_file_system_schemes(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.fs_providers.read().keys().cloned().collect();
        keys.sort();
        keys
    }

    pub fn register_text_document_content_provider(
        &self,
        scheme: impl Into<String>,
    ) -> Result<String, String> {
        let raw = scheme.into();
        let norm = raw.trim().to_ascii_lowercase();
        if norm.is_empty() || norm == "file" || norm == "untitled" {
            return Err(format!(
                "Cannot register TextDocumentContentProvider for scheme '{}'",
                raw
            ));
        }
        self.content_providers
            .write()
            .insert(norm.clone(), norm.clone());
        self.emit(VfsEvent::TextContentRegistered {
            scheme: norm.clone(),
        });
        Ok(norm)
    }

    pub fn unregister_text_document_content_provider(&self, scheme: &str) -> bool {
        let norm = scheme.trim().to_ascii_lowercase();
        let removed = self.content_providers.write().remove(&norm).is_some();
        if removed {
            self.emit(VfsEvent::TextContentUnregistered { scheme: norm });
        }
        removed
    }

    pub fn has_text_content_scheme(&self, uri_or_scheme: &str) -> bool {
        let key = extract_uri_scheme(uri_or_scheme)
            .unwrap_or_else(|| uri_or_scheme.trim().to_ascii_lowercase());
        self.content_providers.read().contains_key(&key)
    }

    pub fn list_text_content_schemes(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.content_providers.read().keys().cloned().collect();
        keys.sort();
        keys
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "workspace.registerFileSystemProvider" => {
                let scheme = params
                    .get("scheme")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'scheme' parameter".to_string())?;
                let opts = params.get("options").unwrap_or(&Value::Null);
                let is_case_sensitive = opts
                    .get("isCaseSensitive")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let is_readonly = opts
                    .get("isReadonly")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let info = self.register_file_system_provider(
                    scheme,
                    is_case_sensitive,
                    is_readonly,
                )?;
                Ok(json!({ "registered": true, "scheme": info.scheme }))
            }
            "workspace.unregisterFileSystemProvider" => {
                let scheme = params
                    .get("scheme")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'scheme' parameter".to_string())?;
                let removed = self.unregister_file_system_provider(scheme);
                Ok(json!({ "unregistered": removed, "scheme": scheme }))
            }
            "workspace.registerTextDocumentContentProvider" => {
                let scheme = params
                    .get("scheme")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'scheme' parameter".to_string())?;
                let norm = self.register_text_document_content_provider(scheme)?;
                Ok(json!({ "registered": true, "scheme": norm }))
            }
            "workspace.unregisterTextDocumentContentProvider" => {
                let scheme = params
                    .get("scheme")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'scheme' parameter".to_string())?;
                let removed = self.unregister_text_document_content_provider(scheme);
                Ok(json!({ "unregistered": removed, "scheme": scheme }))
            }
            "workspace.onDidChangeFileSystem" => {
                let scheme = params
                    .get("scheme")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let changes: Vec<VfsFileChange> = params
                    .get("changes")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let count = changes.len();
                self.emit(VfsEvent::FilesChanged { scheme, changes });
                Ok(json!({ "recorded": true, "count": count }))
            }
            "workspace.onDidChangeTextDocumentContent" => {
                let uri = params
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'uri' parameter".to_string())?;
                self.emit(VfsEvent::TextDocumentContentChanged {
                    uri: uri.to_string(),
                });
                Ok(json!({ "recorded": true, "uri": uri }))
            }
            _ => Err(format!("Unsupported VFS RPC method: {}", method)),
        }
    }

    pub async fn request_read_file(
        &self,
        rpc: &RpcBridge,
        uri: &str,
    ) -> Result<Vec<u8>, String> {
        let val = rpc
            .request(
                "vfs.readFile",
                json!({ "uri": uri }),
                Duration::from_millis(800),
            )
            .await?;
        decode_bytes_value(&val)
    }

    pub async fn request_stat(&self, rpc: &RpcBridge, uri: &str) -> Result<Value, String> {
        rpc.request("vfs.stat", json!({ "uri": uri }), Duration::from_millis(800))
            .await
    }

    pub async fn request_read_directory(
        &self,
        rpc: &RpcBridge,
        uri: &str,
    ) -> Result<Value, String> {
        rpc.request(
            "vfs.readDirectory",
            json!({ "uri": uri }),
            Duration::from_millis(800),
        )
        .await
    }

    pub async fn request_write_file(
        &self,
        rpc: &RpcBridge,
        uri: &str,
        bytes: &[u8],
        create: bool,
        overwrite: bool,
    ) -> Result<Value, String> {
        rpc.request(
            "vfs.writeFile",
            json!({
                "uri": uri,
                "content": bytes,
                "options": { "create": create, "overwrite": overwrite },
            }),
            Duration::from_millis(800),
        )
        .await
    }

    pub async fn request_delete(
        &self,
        rpc: &RpcBridge,
        uri: &str,
        recursive: bool,
    ) -> Result<Value, String> {
        rpc.request(
            "vfs.delete",
            json!({
                "uri": uri,
                "options": { "recursive": recursive },
            }),
            Duration::from_millis(800),
        )
        .await
    }

    pub async fn request_create_directory(
        &self,
        rpc: &RpcBridge,
        uri: &str,
    ) -> Result<Value, String> {
        rpc.request(
            "vfs.createDirectory",
            json!({ "uri": uri }),
            Duration::from_millis(800),
        )
        .await
    }

    pub async fn request_provide_text_document_content(
        &self,
        rpc: &RpcBridge,
        uri: &str,
    ) -> Result<String, String> {
        let val = rpc
            .request(
                "vfs.provideTextDocumentContent",
                json!({ "uri": uri }),
                Duration::from_millis(800),
            )
            .await?;
        Ok(match val {
            Value::String(s) => s,
            other => other
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })
    }
}

fn decode_bytes_value(val: &Value) -> Result<Vec<u8>, String> {
    if let Some(arr) = val.as_array() {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            let b = item
                .as_u64()
                .ok_or_else(|| "Invalid byte value in vfs.readFile array".to_string())?;
            out.push(b as u8);
        }
        return Ok(out);
    }
    if let Some(s) = val.as_str() {
        return Ok(s.as_bytes().to_vec());
    }
    if let Some(content) = val.get("content") {
        return decode_bytes_value(content);
    }
    Err("Invalid vfs.readFile payload from provider".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uri_scheme_extraction_and_provider_registration() {
        assert_eq!(extract_uri_scheme("git:/src/lib.rs"), Some("git".into()));
        assert_eq!(
            extract_uri_scheme("vscode-userdata:/settings.json"),
            Some("vscode-userdata".into())
        );
        assert_eq!(extract_uri_scheme("C:\\Users\\dev\\a.rs"), None);
        assert_eq!(extract_uri_scheme("D:/repo/a.rs"), None);

        let vfs = ExtHostVfsManager::new();
        assert!(vfs
            .handle_rpc_call(
                "workspace.registerFileSystemProvider",
                &json!({ "scheme": "memfs", "options": { "isReadonly": false } })
            )
            .is_ok());
        assert!(vfs.has_file_system_scheme("memfs:/hello.txt"));
        assert!(!vfs.has_file_system_scheme("file:///hello.txt"));
        assert!(vfs
            .register_file_system_provider("file", true, false)
            .is_err());

        assert!(vfs
            .handle_rpc_call(
                "workspace.registerTextDocumentContentProvider",
                &json!({ "scheme": "git-show" })
            )
            .is_ok());
        assert!(vfs.has_text_content_scheme("git-show://HEAD/main.rs"));
    }
}
