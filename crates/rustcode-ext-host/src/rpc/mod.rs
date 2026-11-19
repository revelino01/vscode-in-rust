//! MessagePack / JSON RPC bridge between Rust Core and Extension Host.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum RpcMessage {
    #[serde(rename = "request")]
    Request {
        id: u64,
        method: String,
        params: Value,
    },
    #[serde(rename = "response")]
    Response {
        id: u64,
        result: Option<Value>,
        error: Option<String>,
    },
    #[serde(rename = "notification")]
    Notification {
        channel: String,
        data: Value,
    },
}

impl RpcMessage {
    /// Serialize message to MessagePack binary format.
    pub fn to_msgpack(&self) -> Result<Vec<u8>, String> {
        rmp_serde::to_vec_named(self).map_err(|e| format!("MessagePack serialization error: {}", e))
    }

    /// Deserialize message from MessagePack binary format.
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, String> {
        rmp_serde::from_slice(bytes).map_err(|e| format!("MessagePack deserialization error: {}", e))
    }

    /// Serialize message to JSON string.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("JSON serialization error: {}", e))
    }

    /// Deserialize message from JSON string.
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("JSON deserialization error: {}", e))
    }
}

type PendingResponseMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

pub struct RpcBridge {
    next_id: AtomicU64,
    pending: PendingResponseMap,
}

impl Default for RpcBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl RpcBridge {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create an outbound RPC request message and get an async receiver for the response.
    pub fn create_request(&self, method: &str, params: Value) -> (RpcMessage, oneshot::Receiver<Result<Value, String>>) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id, tx);

        let msg = RpcMessage::Request {
            id,
            method: method.to_string(),
            params,
        };
        (msg, rx)
    }

    /// Create an outbound fire-and-forget RPC notification without registering a pending response waiter.
    pub fn create_notification(&self, channel: &str, data: Value) -> RpcMessage {
        RpcMessage::Notification {
            channel: channel.to_string(),
            data,
        }
    }

    /// Return the number of currently pending outbound RPC requests awaiting responses.
    pub fn pending_count(&self) -> usize {
        self.pending.lock().len()
    }

    /// Dispatch an incoming response to fulfill a pending request.
    pub fn handle_incoming_response(&self, id: u64, result: Option<Value>, error: Option<String>) -> bool {
        let sender = self.pending.lock().remove(&id);
        if let Some(tx) = sender {
            let res = match error {
                Some(err) => Err(err),
                None => Ok(result.unwrap_or(Value::Null)),
            };
            let _ = tx.send(res);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_rpc_msgpack_roundtrip() {
        let msg = RpcMessage::Request {
            id: 42,
            method: "window.showInformationMessage".to_string(),
            params: json!({ "message": "Hello from RustCode!" }),
        };

        let bytes = msg.to_msgpack().unwrap();
        let decoded = RpcMessage::from_msgpack(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_rpc_bridge_request_response() {
        let bridge = RpcBridge::new();
        let (req, mut rx) = bridge.create_request("workspace.getConfiguration", json!({ "section": "editor" }));

        if let RpcMessage::Request { id, .. } = req {
            let handled = bridge.handle_incoming_response(id, Some(json!({ "fontSize": 14 })), None);
            assert!(handled);

            let res = rx.try_recv().unwrap().unwrap();
            assert_eq!(res, json!({ "fontSize": 14 }));
        } else {
            panic!("Expected request message");
        }
    }
}
