//! Extension Host Authentication Manager (`vscode.authentication.*`).

use crate::rpc::RpcBridge;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtAuthProviderInfo {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub supports_multiple_accounts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtAuthAccountInfo {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtAuthSessionInfo {
    pub id: String,
    #[serde(default)]
    pub provider_id: String,
    pub access_token: String,
    pub account: ExtAuthAccountInfo,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtAuthEvent {
    ProviderRegistered {
        id: String,
        label: String,
        supports_multiple_accounts: bool,
    },
    ProviderUnregistered {
        id: String,
    },
    SessionsChanged {
        provider_id: String,
        added: usize,
        removed: usize,
        changed: usize,
    },
}

pub type AuthEventListener = Arc<dyn Fn(&ExtAuthEvent) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostAuthManager {
    providers: Arc<RwLock<HashMap<String, ExtAuthProviderInfo>>>,
    sessions: Arc<RwLock<HashMap<String, Vec<ExtAuthSessionInfo>>>>,
    listeners: Arc<RwLock<Vec<AuthEventListener>>>,
    pending_events: Arc<RwLock<Vec<ExtAuthEvent>>>,
    revision: Arc<AtomicU64>,
}

impl ExtHostAuthManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<ExtAuthEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_auth_event<F>(&self, listener: F)
    where
        F: Fn(&ExtAuthEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    fn emit(&self, event: ExtAuthEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.listeners.read().clone();
        for cb in listeners {
            cb(&event);
        }
    }

    pub fn list_providers(&self) -> Vec<ExtAuthProviderInfo> {
        self.providers.read().values().cloned().collect()
    }

    pub fn store_session(&self, mut session: ExtAuthSessionInfo) {
        if session.provider_id.is_empty() {
            session.provider_id = "default".to_string();
        }
        let provider_id = session.provider_id.clone();
        let mut guard = self.sessions.write();
        let list = guard.entry(provider_id).or_default();
        list.retain(|s| s.id != session.id);
        list.push(session);
    }

    pub fn list_sessions(&self, provider_id: &str) -> Vec<ExtAuthSessionInfo> {
        self.sessions
            .read()
            .get(provider_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "authentication.registerProvider" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing auth provider id".to_string())?
                    .to_string();
                let label = params
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string();
                let supports_multiple_accounts = params
                    .get("supportsMultipleAccounts")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let info = ExtAuthProviderInfo {
                    id: id.clone(),
                    label: label.clone(),
                    supports_multiple_accounts,
                };
                self.providers.write().insert(id.clone(), info);
                self.emit(ExtAuthEvent::ProviderRegistered {
                    id: id.clone(),
                    label,
                    supports_multiple_accounts,
                });
                Ok(json!({ "registered": true, "id": id }))
            }
            "authentication.unregisterProvider" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing auth provider id".to_string())?;
                let removed = self.providers.write().remove(id).is_some();
                self.sessions.write().remove(id);
                if removed {
                    self.emit(ExtAuthEvent::ProviderUnregistered { id: id.to_string() });
                }
                Ok(json!({ "unregistered": removed, "id": id }))
            }
            "authentication.didChangeSessions" => {
                let provider_id = params
                    .get("providerId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing providerId".to_string())?
                    .to_string();
                if !self.providers.read().contains_key(&provider_id) {
                    return Err(format!(
                        "Authentication provider '{}' is not registered",
                        provider_id
                    ));
                }
                let added: Vec<ExtAuthSessionInfo> = match params.get("added") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid added sessions: {}", e))?,
                    _ => Vec::new(),
                };
                let removed: Vec<ExtAuthSessionInfo> = match params.get("removed") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid removed sessions: {}", e))?,
                    _ => Vec::new(),
                };
                let changed: Vec<ExtAuthSessionInfo> = match params.get("changed") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid changed sessions: {}", e))?,
                    _ => Vec::new(),
                };

                {
                    let mut guard = self.sessions.write();
                    let list = guard.entry(provider_id.clone()).or_default();
                    for rem in &removed {
                        list.retain(|s| s.id != rem.id);
                    }
                    for mut s in added.iter().cloned().chain(changed.iter().cloned()) {
                        if s.provider_id.is_empty() {
                            s.provider_id = provider_id.clone();
                        }
                        list.retain(|existing| existing.id != s.id);
                        list.push(s);
                    }
                }

                self.emit(ExtAuthEvent::SessionsChanged {
                    provider_id,
                    added: added.len(),
                    removed: removed.len(),
                    changed: changed.len(),
                });
                Ok(json!({ "updated": true }))
            }
            "authentication.getSession" => {
                let provider_id = params
                    .get("providerId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing providerId".to_string())?;
                let requested_scopes: Vec<String> = match params.get("scopes") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid scopes: {}", e))?,
                    _ => Vec::new(),
                };
                let sessions = self.sessions.read();
                let matched = sessions.get(provider_id).and_then(|list| {
                    list.iter().find(|s| {
                        requested_scopes
                            .iter()
                            .all(|req| s.scopes.contains(req))
                    })
                });
                match matched {
                    Some(session) => Ok(serde_json::to_value(session).unwrap_or(Value::Null)),
                    None => Ok(Value::Null),
                }
            }
            "authentication.getAccounts" => {
                let provider_id = params
                    .get("providerId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing providerId".to_string())?;
                let sessions = self.sessions.read();
                let mut accounts: Vec<ExtAuthAccountInfo> = Vec::new();
                if let Some(list) = sessions.get(provider_id) {
                    for s in list {
                        if !accounts.iter().any(|a| a.id == s.account.id) {
                            accounts.push(s.account.clone());
                        }
                    }
                }
                Ok(serde_json::to_value(accounts).unwrap_or(json!([])))
            }
            _ => Err(format!("Unsupported authentication RPC method: {}", method)),
        }
    }

    pub async fn request_get_sessions(
        &self,
        rpc_bridge: &RpcBridge,
        provider_id: &str,
        scopes: &[String],
    ) -> Result<Vec<ExtAuthSessionInfo>, String> {
        let val = rpc_bridge
            .request(
                "authentication.getSessions",
                json!({
                    "providerId": provider_id,
                    "scopes": scopes,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'authentication.getSessions' timed out") {
                    "Timed out waiting for getSessions".to_string()
                } else {
                    e
                }
            })?;

        serde_json::from_value::<Vec<ExtAuthSessionInfo>>(val)
            .map_err(|e| format!("Invalid auth sessions response: {}", e))
    }

    pub async fn request_create_session(
        &self,
        rpc_bridge: &RpcBridge,
        provider_id: &str,
        scopes: &[String],
    ) -> Result<ExtAuthSessionInfo, String> {
        let val = rpc_bridge
            .request(
                "authentication.createSession",
                json!({
                    "providerId": provider_id,
                    "scopes": scopes,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'authentication.createSession' timed out") {
                    "Timed out waiting for createSession".to_string()
                } else {
                    e
                }
            })?;

        serde_json::from_value::<ExtAuthSessionInfo>(val)
            .map_err(|e| format!("Invalid auth session response: {}", e))
    }

    pub async fn request_remove_session(
        &self,
        rpc_bridge: &RpcBridge,
        provider_id: &str,
        session_id: &str,
    ) -> Result<bool, String> {
        let val = rpc_bridge
            .request(
                "authentication.removeSession",
                json!({
                    "providerId": provider_id,
                    "sessionId": session_id,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'authentication.removeSession' timed out") {
                    "Timed out waiting for removeSession".to_string()
                } else {
                    e
                }
            })?;

        val.as_bool()
            .ok_or_else(|| "Invalid removeSession response: expected boolean".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_auth_manager_lifecycle() {
        let mgr = ExtHostAuthManager::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        mgr.on_auth_event(move |ev| {
            events_clone.lock().unwrap().push(ev.clone());
        });

        mgr.handle_rpc_call(
            "authentication.registerProvider",
            &json!({ "id": "github", "label": "GitHub", "supportsMultipleAccounts": true }),
        )
        .unwrap();
        assert_eq!(mgr.list_providers().len(), 1);

        mgr.handle_rpc_call(
            "authentication.didChangeSessions",
            &json!({
                "providerId": "github",
                "added": [{
                    "id": "s1",
                    "accessToken": "tok_abc",
                    "account": { "id": "acc_1", "label": "octocat" },
                    "scopes": ["read:user", "repo"]
                }],
                "removed": [],
                "changed": []
            }),
        )
        .unwrap();

        let session = mgr
            .handle_rpc_call(
                "authentication.getSession",
                &json!({ "providerId": "github", "scopes": ["repo"] }),
            )
            .unwrap();
        assert_eq!(session["id"], "s1");

        let accounts = mgr
            .handle_rpc_call(
                "authentication.getAccounts",
                &json!({ "providerId": "github" }),
            )
            .unwrap();
        assert_eq!(accounts.as_array().unwrap().len(), 1);
        assert_eq!(accounts[0]["label"], "octocat");

        mgr.handle_rpc_call(
            "authentication.unregisterProvider",
            &json!({ "id": "github" }),
        )
        .unwrap();
        assert!(mgr.list_providers().is_empty());
        assert!(mgr.list_sessions("github").is_empty());
        assert_eq!(events.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn test_auth_request_roundtrip() {
        let mgr = ExtHostAuthManager::new();
        let rpc = Arc::new(RpcBridge::new());
        let rpc_clone = rpc.clone();

        tokio::spawn(async move {
            tokio::task::yield_now().await;
            rpc_clone.handle_incoming_response(
                1,
                Some(json!({
                    "id": "s2",
                    "providerId": "github",
                    "accessToken": "tok_new",
                    "account": { "id": "acc_1", "label": "octocat" },
                    "scopes": ["repo"]
                })),
                None,
            );
        });

        let created = mgr
            .request_create_session(&rpc, "github", &["repo".to_string()])
            .await
            .unwrap();
        assert_eq!(created.id, "s2");
        assert_eq!(created.access_token, "tok_new");
    }

    #[tokio::test]
    async fn test_auth_malformed_payloads_unregistered_provider_and_null_remove_session() {
        let mgr = ExtHostAuthManager::new();
        mgr.handle_rpc_call(
            "authentication.registerProvider",
            &json!({ "id": "gh", "label": "GitHub" }),
        )
        .unwrap();

        // Malformed added/removed/changed entries must return Err.
        assert!(mgr
            .handle_rpc_call(
                "authentication.didChangeSessions",
                &json!({
                    "providerId": "gh",
                    "added": [{ "id": "s_bad" }]
                }),
            )
            .is_err());
        assert!(mgr
            .handle_rpc_call(
                "authentication.didChangeSessions",
                &json!({
                    "providerId": "gh",
                    "removed": "not-an-array"
                }),
            )
            .is_err());
        assert!(mgr
            .handle_rpc_call(
                "authentication.didChangeSessions",
                &json!({
                    "providerId": "gh",
                    "changed": [123]
                }),
            )
            .is_err());

        mgr.handle_rpc_call(
            "authentication.didChangeSessions",
            &json!({
                "providerId": "gh",
                "added": [{
                    "id": "s1",
                    "accessToken": "tok",
                    "account": { "id": "a1", "label": "alice" },
                    "scopes": ["repo"]
                }]
            }),
        )
        .unwrap();

        // Non-array scopes must return Err rather than bypassing scope filtering.
        assert!(mgr
            .handle_rpc_call(
                "authentication.getSession",
                &json!({ "providerId": "gh", "scopes": "admin:org" }),
            )
            .is_err());

        // Unregister provider -> subsequent didChangeSessions must return Err.
        mgr.handle_rpc_call("authentication.unregisterProvider", &json!({ "id": "gh" }))
            .unwrap();
        assert!(mgr
            .handle_rpc_call(
                "authentication.didChangeSessions",
                &json!({
                    "providerId": "gh",
                    "added": [{
                        "id": "s2",
                        "accessToken": "tok2",
                        "account": { "id": "a1", "label": "alice" },
                        "scopes": ["repo"]
                    }]
                }),
            )
            .is_err());
        assert!(mgr.list_sessions("gh").is_empty());

        // request_remove_session must reject Value::Null instead of coercing to Ok(true).
        let rpc = Arc::new(RpcBridge::new());
        let rpc_clone = rpc.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            rpc_clone.handle_incoming_response(1, Some(Value::Null), None);
        });
        assert!(mgr.request_remove_session(&rpc, "gh", "s1").await.is_err());
    }
}
