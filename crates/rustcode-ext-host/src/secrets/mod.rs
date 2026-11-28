//! Extension-namespaced Secret Storage Manager (`ExtensionContext.secrets`).

use parking_lot::RwLock;
use rustcode_platform::{InMemorySecretStorageBackend, SecretStorageBackend};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretChangeEvent {
    pub extension_id: String,
    pub key: String,
}

pub type SecretChangeListener = Arc<dyn Fn(&SecretChangeEvent) + Send + Sync>;

#[derive(Clone)]
pub struct ExtHostSecretManager {
    backend: Arc<RwLock<Arc<dyn SecretStorageBackend>>>,
    tracked_keys: Arc<RwLock<BTreeMap<String, BTreeSet<String>>>>,
    listeners: Arc<RwLock<Vec<SecretChangeListener>>>,
    pending_events: Arc<RwLock<Vec<SecretChangeEvent>>>,
    revision: Arc<AtomicU64>,
}

impl Default for ExtHostSecretManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtHostSecretManager {
    pub fn new() -> Self {
        Self {
            backend: Arc::new(RwLock::new(Arc::new(InMemorySecretStorageBackend::new()))),
            tracked_keys: Arc::new(RwLock::new(BTreeMap::new())),
            listeners: Arc::new(RwLock::new(Vec::new())),
            pending_events: Arc::new(RwLock::new(Vec::new())),
            revision: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_backend(self, backend: Arc<dyn SecretStorageBackend>) -> Self {
        *self.backend.write() = backend;
        self
    }

    pub fn set_backend(&self, backend: Arc<dyn SecretStorageBackend>) {
        *self.backend.write() = backend;
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn on_did_change<F>(&self, listener: F)
    where
        F: Fn(&SecretChangeEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    pub fn take_pending_events(&self) -> Vec<SecretChangeEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    fn emit(&self, ev: SecretChangeEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(ev.clone());
        let listeners = self.listeners.read().clone();
        for cb in listeners {
            cb(&ev);
        }
    }

    /// Build a strictly namespaced storage account key so extension A cannot read
    /// or overwrite secrets stored by extension B.
    pub fn namespaced_account(extension_id: &str, key: &str) -> Result<String, String> {
        let ext = extension_id.trim();
        if ext.is_empty() || ext.chars().any(char::is_whitespace) {
            return Err(format!("Invalid extensionId '{}'", extension_id));
        }
        if key.is_empty() {
            return Err("Secret key cannot be empty".to_string());
        }
        Ok(format!(
            "rustcode.ext.{}:{}",
            ext.to_ascii_lowercase(),
            key
        ))
    }

    pub fn get(&self, extension_id: &str, key: &str) -> Result<Option<String>, String> {
        let account = Self::namespaced_account(extension_id, key)?;
        self.backend.read().get_secret(&account)
    }

    pub fn store(&self, extension_id: &str, key: &str, value: &str) -> Result<(), String> {
        let account = Self::namespaced_account(extension_id, key)?;
        self.backend.read().set_secret(&account, value)?;
        let norm_ext = extension_id.trim().to_ascii_lowercase();
        self.tracked_keys
            .write()
            .entry(norm_ext.clone())
            .or_default()
            .insert(key.to_string());
        self.emit(SecretChangeEvent {
            extension_id: norm_ext,
            key: key.to_string(),
        });
        Ok(())
    }

    pub fn delete(&self, extension_id: &str, key: &str) -> Result<(), String> {
        let account = Self::namespaced_account(extension_id, key)?;
        self.backend.read().delete_secret(&account)?;
        let norm_ext = extension_id.trim().to_ascii_lowercase();
        if let Some(set) = self.tracked_keys.write().get_mut(&norm_ext) {
            set.remove(key);
        }
        self.emit(SecretChangeEvent {
            extension_id: norm_ext,
            key: key.to_string(),
        });
        Ok(())
    }

    pub fn list_keys_for_extension(&self, extension_id: &str) -> Vec<String> {
        let norm = extension_id.trim().to_ascii_lowercase();
        self.tracked_keys
            .read()
            .get(&norm)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        let extension_id = params
            .get("extensionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing string 'extensionId' parameter".to_string())?;
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing string 'key' parameter".to_string())?;

        match method {
            "secrets.get" => {
                let value = self.get(extension_id, key)?;
                Ok(json!({ "value": value }))
            }
            "secrets.store" => {
                let value = params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'value' parameter".to_string())?;
                self.store(extension_id, key, value)?;
                Ok(json!({ "stored": true }))
            }
            "secrets.delete" => {
                self.delete(extension_id, key)?;
                Ok(json!({ "deleted": true }))
            }
            _ => Err(format!("Unsupported secret RPC method: {}", method)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_namespace_isolation_and_rpc() {
        let mgr = ExtHostSecretManager::new();
        mgr.handle_rpc_call(
            "secrets.store",
            &json!({ "extensionId": "GitHub.copilot", "key": "apiToken", "value": "tok_a" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "secrets.store",
            &json!({ "extensionId": "Other.ext", "key": "apiToken", "value": "tok_b" }),
        )
        .unwrap();

        let got_a = mgr
            .handle_rpc_call(
                "secrets.get",
                &json!({ "extensionId": "github.copilot", "key": "apiToken" }),
            )
            .unwrap();
        let got_b = mgr
            .handle_rpc_call(
                "secrets.get",
                &json!({ "extensionId": "other.ext", "key": "apiToken" }),
            )
            .unwrap();
        assert_eq!(got_a["value"], json!("tok_a"));
        assert_eq!(got_b["value"], json!("tok_b"));

        mgr.handle_rpc_call(
            "secrets.delete",
            &json!({ "extensionId": "GitHub.copilot", "key": "apiToken" }),
        )
        .unwrap();
        assert!(mgr.get("github.copilot", "apiToken").unwrap().is_none());
        assert_eq!(
            mgr.get("other.ext", "apiToken").unwrap().as_deref(),
            Some("tok_b")
        );
        assert_eq!(mgr.take_pending_events().len(), 3);
    }
}
