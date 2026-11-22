//! DAP client process lifecycle and request/event routing over stdio.

use super::types::*;
use crate::codec::{read_framed_message, write_framed_message, CodecError};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Error)]
pub enum DapError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Codec error: {0}")]
    Codec(#[from] CodecError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("DAP adapter error: {message}")]
    AdapterError { message: String },
    #[error("DAP client not running")]
    NotRunning,
    #[error("Request cancelled or channel closed")]
    ChannelClosed,
    #[error("Timeout waiting for response")]
    Timeout,
}

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, DapError>>>>>;
pub type DapEventListener = Arc<dyn Fn(&DapMessage) + Send + Sync>;

#[derive(Clone)]
pub struct DapClient {
    next_seq: Arc<AtomicI64>,
    pending: PendingMap,
    tx_outbound: mpsc::Sender<Vec<u8>>,
    capabilities: Arc<Mutex<Option<Capabilities>>>,
    is_running: Arc<AtomicBool>,
    event_listeners: Arc<Mutex<Vec<DapEventListener>>>,
    child: Arc<Mutex<Option<tokio::process::Child>>>,
}

impl DapClient {
    /// Spawns a debug adapter process (e.g. `gdb`, `lldb-vscode`, `codelldb`, `node`, `debugpy`)
    /// with stdio communication.
    pub fn spawn_process(
        command: &str,
        args: &[&str],
        cwd: Option<PathBuf>,
    ) -> Result<Self, DapError> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = cmd.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("Failed to open debug adapter stdout"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("Failed to open debug adapter stdin"))?;

        let client = Self::from_io(stdout, stdin);
        *client.child.lock() = Some(child);
        Ok(client)
    }

    /// Creates a DAP client connected to custom async reader/writer streams.
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
        let event_listeners: Arc<Mutex<Vec<DapEventListener>>> = Arc::new(Mutex::new(Vec::new()));

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
        let listeners_clone = event_listeners.clone();
        let is_running_inbound = is_running.clone();

        tokio::spawn(async move {
            let mut reader = reader;
            let mut buffer = Vec::new();

            loop {
                match read_framed_message(&mut reader, &mut buffer).await {
                    Ok(Some(bytes)) => {
                        if let Ok(msg) = serde_json::from_slice::<DapMessage>(&bytes) {
                            match &msg {
                                DapMessage::Response {
                                    request_seq,
                                    success,
                                    message,
                                    body,
                                    ..
                                } => {
                                    let sender = pending_clone.lock().remove(request_seq);
                                    if let Some(tx) = sender {
                                        if *success {
                                            let _ = tx.send(Ok(body.clone().unwrap_or(Value::Null)));
                                        } else {
                                            let err_msg = message
                                                .clone()
                                                .unwrap_or_else(|| "Unknown DAP error".to_string());
                                            let _ = tx.send(Err(DapError::AdapterError {
                                                message: err_msg,
                                            }));
                                        }
                                    }
                                }
                                DapMessage::Event { .. } => {
                                    let listeners = listeners_clone.lock().clone();
                                    for listener in listeners {
                                        listener(&msg);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            is_running_inbound.store(false, Ordering::SeqCst);
            // Fail all pending requests
            let mut guard = pending_clone.lock();
            for (_, tx) in guard.drain() {
                let _ = tx.send(Err(DapError::ChannelClosed));
            }
        });

        Self {
            next_seq: Arc::new(AtomicI64::new(1)),
            pending,
            tx_outbound,
            capabilities,
            is_running,
            event_listeners,
            child: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    pub fn add_event_listener<F>(&self, listener: F)
    where
        F: Fn(&DapMessage) + Send + Sync + 'static,
    {
        self.event_listeners.lock().push(Arc::new(listener));
    }

    pub fn capabilities(&self) -> Option<Capabilities> {
        self.capabilities.lock().clone()
    }

    /// Generic request dispatch.
    pub async fn send_request(
        &self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<Value, DapError> {
        if !self.is_running() {
            return Err(DapError::NotRunning);
        }

        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(seq, tx);

        let req = DapMessage::Request {
            seq,
            command: command.to_string(),
            arguments,
        };

        let json_bytes = serde_json::to_vec(&req)?;
        if self.tx_outbound.send(json_bytes).await.is_err() {
            self.pending.lock().remove(&seq);
            return Err(DapError::NotRunning);
        }

        rx.await.map_err(|_| DapError::ChannelClosed)?
    }

    // --- Core DAP Specification API Methods ---

    pub async fn initialize(
        &self,
        args: InitializeRequestArguments,
    ) -> Result<Capabilities, DapError> {
        let args_val = serde_json::to_value(args)?;
        let resp = self.send_request("initialize", Some(args_val)).await?;
        let caps: Capabilities = serde_json::from_value(resp)?;
        *self.capabilities.lock() = Some(caps.clone());
        Ok(caps)
    }

    pub async fn launch(&self, config: Value) -> Result<(), DapError> {
        self.send_request("launch", Some(config)).await?;
        Ok(())
    }

    pub async fn attach(&self, config: Value) -> Result<(), DapError> {
        self.send_request("attach", Some(config)).await?;
        Ok(())
    }

    pub async fn configuration_done(&self) -> Result<(), DapError> {
        self.send_request("configurationDone", None).await?;
        Ok(())
    }

    pub async fn set_breakpoints(
        &self,
        source_path: &str,
        breakpoints: &[SourceBreakpoint],
    ) -> Result<Vec<Breakpoint>, DapError> {
        let args = SetBreakpointsArguments {
            source: Source {
                name: PathBuf::from(source_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string()),
                path: Some(source_path.to_string()),
                source_reference: None,
            },
            breakpoints: breakpoints.to_vec(),
        };
        let resp = self
            .send_request("setBreakpoints", Some(serde_json::to_value(args)?))
            .await?;
        let body: SetBreakpointsResponseBody = serde_json::from_value(resp)?;
        Ok(body.breakpoints)
    }

    pub async fn threads(&self) -> Result<Vec<Thread>, DapError> {
        let resp = self.send_request("threads", None).await?;
        let body: ThreadsResponseBody = serde_json::from_value(resp)?;
        Ok(body.threads)
    }

    pub async fn stack_trace(&self, thread_id: i64) -> Result<Vec<StackFrame>, DapError> {
        let resp = self
            .send_request("stackTrace", Some(json!({ "threadId": thread_id })))
            .await?;
        let body: StackTraceResponseBody = serde_json::from_value(resp)?;
        Ok(body.stack_frames)
    }

    pub async fn scopes(&self, frame_id: u32) -> Result<Vec<Scope>, DapError> {
        let resp = self
            .send_request("scopes", Some(json!({ "frameId": frame_id })))
            .await?;
        let body: ScopesResponseBody = serde_json::from_value(resp)?;
        Ok(body.scopes)
    }

    pub async fn variables(&self, variables_reference: u32) -> Result<Vec<Variable>, DapError> {
        let resp = self
            .send_request(
                "variables",
                Some(json!({ "variablesReference": variables_reference })),
            )
            .await?;
        let body: VariablesResponseBody = serde_json::from_value(resp)?;
        Ok(body.variables)
    }

    pub async fn continue_execution(&self, thread_id: i64) -> Result<(), DapError> {
        self.send_request("continue", Some(json!({ "threadId": thread_id })))
            .await?;
        Ok(())
    }

    pub async fn next(&self, thread_id: i64) -> Result<(), DapError> {
        self.send_request("next", Some(json!({ "threadId": thread_id })))
            .await?;
        Ok(())
    }

    pub async fn step_in(&self, thread_id: i64) -> Result<(), DapError> {
        self.send_request("stepIn", Some(json!({ "threadId": thread_id })))
            .await?;
        Ok(())
    }

    pub async fn step_out(&self, thread_id: i64) -> Result<(), DapError> {
        self.send_request("stepOut", Some(json!({ "threadId": thread_id })))
            .await?;
        Ok(())
    }

    pub async fn restart(&self) -> Result<(), DapError> {
        self.send_request("restart", None).await?;
        Ok(())
    }

    pub async fn disconnect(&self, restart: bool) -> Result<(), DapError> {
        let _ = self
            .send_request("disconnect", Some(json!({ "restart": restart })))
            .await;
        self.is_running.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub async fn evaluate(
        &self,
        expression: &str,
        frame_id: Option<u32>,
    ) -> Result<EvaluateResponseBody, DapError> {
        let mut args = json!({
            "expression": expression,
        });
        if let Some(fid) = frame_id {
            args["frameId"] = json!(fid);
        }
        let resp = self.send_request("evaluate", Some(args)).await?;
        let body: EvaluateResponseBody = serde_json::from_value(resp)?;
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn test_dap_client_lifecycle_and_mock_adapter() {
        let (client_io, server_io) = duplex(4096);
        let (c_reader, c_writer) = tokio::io::split(client_io);
        let client = DapClient::from_io(c_reader, c_writer);

        // Mock DAP adapter server task
        tokio::spawn(async move {
            let (mut s_reader, mut s_writer) = tokio::io::split(server_io);
            let mut buf = Vec::new();

            while let Ok(Some(bytes)) = read_framed_message(&mut s_reader, &mut buf).await {
                let msg: DapMessage = serde_json::from_slice(&bytes).unwrap();
                if let DapMessage::Request { seq, command, .. } = msg {
                    match command.as_str() {
                        "initialize" => {
                            let resp = DapMessage::Response {
                                seq: 100,
                                request_seq: seq,
                                success: true,
                                command,
                                message: None,
                                body: Some(json!({
                                    "supportsConfigurationDoneRequest": true
                                })),
                            };
                            let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                        }
                        "threads" => {
                            let resp = DapMessage::Response {
                                seq: 101,
                                request_seq: seq,
                                success: true,
                                command,
                                message: None,
                                body: Some(json!({
                                    "threads": [
                                        { "id": 1, "name": "main thread" }
                                    ]
                                })),
                            };
                            let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                        }
                        "stackTrace" => {
                            let resp = DapMessage::Response {
                                seq: 102,
                                request_seq: seq,
                                success: true,
                                command,
                                message: None,
                                body: Some(json!({
                                    "stackFrames": [
                                        { "id": 10, "name": "main()", "line": 42, "column": 5 }
                                    ]
                                })),
                            };
                            let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                        }
                        "next" => {
                            let resp = DapMessage::Response {
                                seq: 103,
                                request_seq: seq,
                                success: true,
                                command,
                                message: None,
                                body: None,
                            };
                            let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                        }
                        "disconnect" => {
                            let resp = DapMessage::Response {
                                seq: 104,
                                request_seq: seq,
                                success: true,
                                command,
                                message: None,
                                body: None,
                            };
                            let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        });

        // 1. Initialize
        let caps = client
            .initialize(InitializeRequestArguments {
                adapter_id: "mock_dap".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(caps.supports_configuration_done_request, Some(true));

        // 2. Query threads
        let threads = client.threads().await.unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].name, "main thread");

        // 3. Query stack trace
        let frames = client.stack_trace(1).await.unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].name, "main()");
        assert_eq!(frames[0].line, 42);

        // 4. Step over (next)
        assert!(client.next(1).await.is_ok());

        // 5. Disconnect
        assert!(client.disconnect(false).await.is_ok());
    }
}
