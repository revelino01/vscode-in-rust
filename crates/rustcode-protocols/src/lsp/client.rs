//! LSP client process lifecycle and request/notification routing.

use super::codec::{read_framed_message, write_framed_message};
use super::types::*;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Error)]
pub enum LspError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Codec error: {0}")]
    Codec(#[from] super::codec::LspCodecError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("LSP server error: code {code}: {message}")]
    ServerError { code: i64, message: String },
    #[error("LSP client not running")]
    NotRunning,
    #[error("Request cancelled or channel closed")]
    ChannelClosed,
    #[error("Timeout waiting for response")]
    Timeout,
}

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, LspError>>>>>;

pub type DiagnosticsListener = Arc<dyn Fn(&str, &[LspDiagnostic]) + Send + Sync>;

pub struct LspClient {
    next_id: AtomicU64,
    pending: PendingMap,
    tx_outbound: mpsc::Sender<Vec<u8>>,
    capabilities: Arc<Mutex<Option<ServerCapabilities>>>,
    is_running: Arc<AtomicBool>,
    diagnostics: Arc<Mutex<HashMap<String, Vec<LspDiagnostic>>>>,
    diagnostics_listeners: Arc<Mutex<Vec<DiagnosticsListener>>>,
}

impl LspClient {
    /// Spawns a language server process (e.g. `rust-analyzer`, `pyright`, etc.)
    /// with stdio communication.
    pub fn spawn_process(
        command: &str,
        args: &[&str],
        cwd: Option<PathBuf>,
    ) -> Result<Self, LspError> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = cmd.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("Failed to open child stdout"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("Failed to open child stdin"))?;

        Ok(Self::from_io(stdout, stdin))
    }

    /// Creates an LSP client connected to custom async reader/writer streams.
    /// Ideal for unit tests using `tokio::io::duplex`.
    pub fn from_io<R, W>(reader: R, mut writer: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (tx_outbound, mut rx_outbound) = mpsc::channel::<Vec<u8>>(100);
        let capabilities = Arc::new(Mutex::new(None));
        let is_running = Arc::new(AtomicBool::new(true));
        let diagnostics = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics_listeners: Arc<Mutex<Vec<DiagnosticsListener>>> = Arc::new(Mutex::new(Vec::new()));

        // Outbound writer loop
        let is_running_clone = is_running.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx_outbound.recv().await {
                if let Err(_e) = write_framed_message(&mut writer, &msg).await {
                    break;
                }
            }
            is_running_clone.store(false, Ordering::SeqCst);
        });

        // Inbound reader loop
        let pending_clone = pending.clone();
        let is_running_reader = is_running.clone();
        let diags_clone = diagnostics.clone();
        let listeners_clone = diagnostics_listeners.clone();
        tokio::spawn(async move {
            let mut reader = reader;
            let mut buf = Vec::new();
            loop {
                match read_framed_message(&mut reader, &mut buf).await {
                    Ok(Some(bytes)) => {
                        if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                            // Check if response
                            if let Some(id) = val.get("id").and_then(|i| i.as_u64()) {
                                let mut map = pending_clone.lock();
                                if let Some(tx) = map.remove(&id) {
                                    if let Some(err) = val.get("error") {
                                        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
                                        let msg = err
                                            .get("message")
                                            .and_then(|m| m.as_str())
                                            .unwrap_or("unknown LSP error")
                                            .to_string();
                                        let _ = tx.send(Err(LspError::ServerError { code, message: msg }));
                                    } else {
                                        let res = val.get("result").cloned().unwrap_or(Value::Null);
                                        let _ = tx.send(Ok(res));
                                    }
                                }
                            } else if let Some(method) = val.get("method").and_then(|m| m.as_str()) {
                                // Notification from server
                                if method == "textDocument/publishDiagnostics" {
                                    if let Some(params) = val.get("params") {
                                        if let (Some(uri), Some(diags)) = (
                                            params.get("uri").and_then(|u| u.as_str()),
                                            params.get("diagnostics"),
                                        ) {
                                            if let Ok(parsed) = serde_json::from_value::<Vec<LspDiagnostic>>(diags.clone()) {
                                                diags_clone.lock().insert(uri.to_string(), parsed.clone());
                                                let listeners = listeners_clone.lock().clone();
                                                for listener in listeners {
                                                    listener(uri, &parsed);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_e) => {
                        break;
                    }
                }
            }
            is_running_reader.store(false, Ordering::SeqCst);
            let mut map = pending_clone.lock();
            for (_id, tx) in map.drain() {
                let _ = tx.send(Err(LspError::NotRunning));
            }
        });

        Self {
            next_id: AtomicU64::new(1),
            pending,
            tx_outbound,
            capabilities,
            is_running,
            diagnostics,
            diagnostics_listeners,
        }
    }

    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    pub fn server_capabilities(&self) -> Option<ServerCapabilities> {
        self.capabilities.lock().clone()
    }

    pub fn get_diagnostics(&self, uri: &str) -> Vec<LspDiagnostic> {
        self.diagnostics.lock().get(uri).cloned().unwrap_or_default()
    }

    pub fn get_all_diagnostics(&self) -> HashMap<String, Vec<LspDiagnostic>> {
        self.diagnostics.lock().clone()
    }

    pub fn on_diagnostics<F>(&self, listener: F)
    where
        F: Fn(&str, &[LspDiagnostic]) + Send + Sync + 'static,
    {
        self.diagnostics_listeners.lock().push(Arc::new(listener));
    }

    pub fn set_diagnostics(&self, uri: &str, diags: Vec<LspDiagnostic>) {
        self.diagnostics.lock().insert(uri.to_string(), diags.clone());
        let listeners = self.diagnostics_listeners.lock().clone();
        for listener in listeners {
            listener(uri, &diags);
        }
    }

    pub fn clear_diagnostics(&self, uri: &str) {
        self.diagnostics.lock().remove(uri);
        let listeners = self.diagnostics_listeners.lock().clone();
        for listener in listeners {
            listener(uri, &[]);
        }
    }

    pub fn clear_all_diagnostics(&self) {
        let keys: Vec<String> = self.diagnostics.lock().keys().cloned().collect();
        self.diagnostics.lock().clear();
        let listeners = self.diagnostics_listeners.lock().clone();
        for key in keys {
            for listener in &listeners {
                listener(&key, &[]);
            }
        }
    }

    /// Send a low-level JSON-RPC request and wait for the response.
    pub async fn send_request(&self, method: &str, params: Value) -> Result<Value, LspError> {
        if !self.is_running() {
            return Err(LspError::NotRunning);
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id, tx);

        let bytes = serde_json::to_vec(&req)?;
        if self.tx_outbound.send(bytes).await.is_err() {
            self.pending.lock().remove(&id);
            return Err(LspError::ChannelClosed);
        }

        rx.await.map_err(|_| LspError::ChannelClosed)?
    }

    /// Send a low-level JSON-RPC notification.
    pub async fn send_notification(&self, method: &str, params: Value) -> Result<(), LspError> {
        if !self.is_running() {
            return Err(LspError::NotRunning);
        }

        let notif = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let bytes = serde_json::to_vec(&notif)?;
        self.tx_outbound
            .send(bytes)
            .await
            .map_err(|_| LspError::ChannelClosed)?;
        Ok(())
    }

    // --- Lifecycle Operations ---

    pub async fn initialize(&self, params: InitializeParams) -> Result<ServerCapabilities, LspError> {
        let val = self
            .send_request("initialize", serde_json::to_value(params)?)
            .await?;
        let result: InitializeResult = serde_json::from_value(val)?;
        *self.capabilities.lock() = Some(result.capabilities.clone());
        Ok(result.capabilities)
    }

    pub async fn initialized(&self) -> Result<(), LspError> {
        self.send_notification("initialized", json!({})).await
    }

    pub async fn shutdown(&self) -> Result<(), LspError> {
        self.send_request("shutdown", Value::Null).await?;
        Ok(())
    }

    pub async fn exit(&self) -> Result<(), LspError> {
        self.send_notification("exit", Value::Null).await
    }

    // --- Document Synchronization ---

    pub async fn did_open(
        &self,
        uri: &str,
        language_id: &str,
        version: i32,
        text: &str,
    ) -> Result<(), LspError> {
        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.to_string(),
                language_id: language_id.to_string(),
                version,
                text: text.to_string(),
            },
        };
        self.send_notification("textDocument/didOpen", serde_json::to_value(params)?)
            .await
    }

    pub async fn did_change_full(
        &self,
        uri: &str,
        version: i32,
        text: &str,
    ) -> Result<(), LspError> {
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.to_string(),
                version,
            },
            content_changes: vec![TextDocumentContentChangeEvent::full(text)],
        };
        self.send_notification("textDocument/didChange", serde_json::to_value(params)?)
            .await
    }

    pub async fn did_change_incremental(
        &self,
        uri: &str,
        version: i32,
        range: LspRange,
        text: &str,
    ) -> Result<(), LspError> {
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.to_string(),
                version,
            },
            content_changes: vec![TextDocumentContentChangeEvent::incremental(range, text)],
        };
        self.send_notification("textDocument/didChange", serde_json::to_value(params)?)
            .await
    }

    pub async fn did_close(&self, uri: &str) -> Result<(), LspError> {
        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: uri.to_string(),
            },
        };
        self.send_notification("textDocument/didClose", serde_json::to_value(params)?)
            .await
    }

    // --- Language Features Routing ---

    pub async fn completion(
        &self,
        uri: &str,
        position: LspPosition,
        context: Option<CompletionContext>,
    ) -> Result<Vec<LspCompletionItem>, LspError> {
        let mut params = json!({
            "textDocument": { "uri": uri },
            "position": position,
        });

        if let Some(ctx) = context {
            params["context"] = serde_json::to_value(ctx)?;
        }

        let res = self.send_request("textDocument/completion", params).await?;

        if res.is_null() {
            return Ok(Vec::new());
        }

        // Response can be CompletionList or Vec<LspCompletionItem>
        if let Ok(list) = serde_json::from_value::<CompletionList>(res.clone()) {
            Ok(list.items)
        } else if let Ok(items) = serde_json::from_value::<Vec<LspCompletionItem>>(res) {
            Ok(items)
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn hover(
        &self,
        uri: &str,
        position: LspPosition,
    ) -> Result<Option<Hover>, LspError> {
        let params = json!({
            "textDocument": { "uri": uri },
            "position": position,
        });

        let res = self.send_request("textDocument/hover", params).await?;
        if res.is_null() {
            return Ok(None);
        }

        let hover: Option<Hover> = serde_json::from_value(res)?;
        Ok(hover)
    }

    pub async fn definition(
        &self,
        uri: &str,
        position: LspPosition,
    ) -> Result<Vec<LspLocation>, LspError> {
        let params = json!({
            "textDocument": { "uri": uri },
            "position": position,
        });

        let res = self.send_request("textDocument/definition", params).await?;
        if res.is_null() {
            return Ok(Vec::new());
        }

        if let Ok(locs) = serde_json::from_value::<Vec<LspLocation>>(res.clone()) {
            Ok(locs)
        } else if let Ok(single) = serde_json::from_value::<LspLocation>(res) {
            Ok(vec![single])
        } else {
            Ok(Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn test_lsp_client_lifecycle_and_features_mock() {
        let (client_read, mut server_write) = duplex(4096);
        let (mut server_read, client_write) = duplex(4096);

        let client = LspClient::from_io(client_read, client_write);

        // Server mock task
        tokio::spawn(async move {
            let mut buf = Vec::new();

            // 1. Expect initialize request
            let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
            let req: Value = serde_json::from_slice(&msg).unwrap();
            assert_eq!(req["method"], "initialize");
            let id = req["id"].as_u64().unwrap();

            let resp = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "capabilities": {
                        "textDocumentSync": 1,
                        "completionProvider": { "triggerCharacters": [".", ":"] },
                        "hoverProvider": true,
                        "definitionProvider": true
                    }
                }
            });
            write_framed_message(&mut server_write, &serde_json::to_vec(&resp).unwrap()).await.unwrap();

            // 2. Expect initialized notification
            let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
            let notif: Value = serde_json::from_slice(&msg).unwrap();
            assert_eq!(notif["method"], "initialized");

            // 3. Expect didOpen notification
            let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
            let open: Value = serde_json::from_slice(&msg).unwrap();
            assert_eq!(open["method"], "textDocument/didOpen");
            assert_eq!(open["params"]["textDocument"]["languageId"], "rust");

            // 4. Expect completion request
            let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
            let comp_req: Value = serde_json::from_slice(&msg).unwrap();
            assert_eq!(comp_req["method"], "textDocument/completion");
            let comp_id = comp_req["id"].as_u64().unwrap();

            let comp_resp = json!({
                "jsonrpc": "2.0",
                "id": comp_id,
                "result": [
                    {
                        "label": "clone",
                        "kind": 2,
                        "detail": "fn(&self) -> Self",
                        "documentation": "Returns a copy of the value.",
                        "insertText": "clone()"
                    }
                ]
            });
            write_framed_message(&mut server_write, &serde_json::to_vec(&comp_resp).unwrap()).await.unwrap();

            // 5. Expect hover request
            let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
            let hover_req: Value = serde_json::from_slice(&msg).unwrap();
            assert_eq!(hover_req["method"], "textDocument/hover");
            let hover_id = hover_req["id"].as_u64().unwrap();

            let hover_resp = json!({
                "jsonrpc": "2.0",
                "id": hover_id,
                "result": {
                    "contents": {
                        "kind": "markdown",
                        "value": "```rust\npub fn clone(&self) -> Self\n```"
                    }
                }
            });
            write_framed_message(&mut server_write, &serde_json::to_vec(&hover_resp).unwrap()).await.unwrap();

            // 6. Expect definition request
            let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
            let def_req: Value = serde_json::from_slice(&msg).unwrap();
            assert_eq!(def_req["method"], "textDocument/definition");
            let def_id = def_req["id"].as_u64().unwrap();

            let def_resp = json!({
                "jsonrpc": "2.0",
                "id": def_id,
                "result": [{
                    "uri": "file:///workspace/main.rs",
                    "range": {
                        "start": { "line": 0, "character": 4 },
                        "end": { "line": 0, "character": 8 }
                    }
                }]
            });
            write_framed_message(&mut server_write, &serde_json::to_vec(&def_resp).unwrap()).await.unwrap();
        });

        // Client execution
        let caps = client.initialize(InitializeParams::default()).await.unwrap();
        assert!(caps.completion_provider.is_some());
        client.initialized().await.unwrap();

        client.did_open("file:///workspace/main.rs", "rust", 1, "fn main() {}").await.unwrap();

        let items = client.completion("file:///workspace/main.rs", LspPosition::new(0, 5), None).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "clone");
        assert_eq!(items[0].insert_text, Some("clone()".to_string()));

        let hover = client.hover("file:///workspace/main.rs", LspPosition::new(0, 5)).await.unwrap().unwrap();
        assert_eq!(hover.contents.kind, MarkupKind::Markdown);
        assert!(hover.contents.value.contains("pub fn clone"));

        let defs = client.definition("file:///workspace/main.rs", LspPosition::new(0, 5)).await.unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].uri, "file:///workspace/main.rs");
        assert_eq!(defs[0].range.start.line, 0);
    }

    #[tokio::test]
    async fn test_lsp_client_incremental_sync_and_server_error() {
        let (client_read, mut server_write) = duplex(4096);
        let (mut server_read, client_write) = duplex(4096);

        let client = LspClient::from_io(client_read, client_write);

        tokio::spawn(async move {
            let mut buf = Vec::new();

            // 1. Receive incremental didChange
            let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
            let change: Value = serde_json::from_slice(&msg).unwrap();
            assert_eq!(change["method"], "textDocument/didChange");
            assert_eq!(change["params"]["textDocument"]["version"], 2);
            assert_eq!(change["params"]["contentChanges"][0]["text"], "x");

            // 2. Receive hover request and return error
            let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
            let req: Value = serde_json::from_slice(&msg).unwrap();
            assert_eq!(req["method"], "textDocument/hover");
            let id = req["id"].as_u64().unwrap();

            let err_resp = json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32603,
                    "message": "Internal error occurred"
                }
            });
            write_framed_message(&mut server_write, &serde_json::to_vec(&err_resp).unwrap()).await.unwrap();
        });

        let change_range = LspRange::new(LspPosition::new(1, 4), LspPosition::new(1, 5));
        client.did_change_incremental("file:///a.rs", 2, change_range, "x").await.unwrap();

        let hover_res = client.hover("file:///a.rs", LspPosition::new(1, 4)).await;
        assert!(matches!(hover_res, Err(LspError::ServerError { code: -32603, .. })));
    }

    #[tokio::test]
    async fn test_lsp_client_disconnect_fails_pending_requests() {
        let (client_read, server_write) = duplex(4096);
        let (server_read, client_write) = duplex(4096);

        let client = LspClient::from_io(client_read, client_write);

        // Abruptly drop server read and write streams to simulate server crash / EOF
        drop(server_write);
        drop(server_read);

        // Give reader task a moment to detect EOF
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let res = client.send_request("test/ping", Value::Null).await;
        assert!(res.is_err());
    }
}
