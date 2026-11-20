//! MessagePack / JSON RPC bridge and live stdio transport between Rust Core and Extension Host.

use parking_lot::Mutex;
use rustcode_platform::strip_windows_verbatim_prefix;
use rustcode_protocols::codec::write_framed_message;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch, Notify};
use tokio::task::JoinHandle;

const MAX_RPC_FRAME_BYTES: usize = 64 * 1024 * 1024;
const MAX_RPC_HEADER_BYTES: usize = 64 * 1024;
const STDERR_TAIL_LIMIT: usize = 200;

/// Structured RPC error propagated across the Rust <-> JS extension host boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

impl RpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new("METHOD_NOT_FOUND", format!("RPC method not found: {}", method))
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new("INVALID_PARAMS", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("INTERNAL_ERROR", message)
    }

    /// Parse a `"CODE: message"` prefixed error string into an `RpcError`.
    pub fn from_prefixed_string(raw: impl Into<String>) -> Self {
        let s = raw.into();
        if let Some((prefix, rest)) = s.split_once(": ") {
            if !prefix.is_empty()
                && prefix
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                return Self::new(prefix, rest);
            }
        }
        Self::new("RPC_ERROR", s)
    }

    pub fn format_wire_string(&self) -> String {
        if self.code.is_empty() || self.message.starts_with(&format!("{}:", self.code)) {
            self.message.clone()
        } else {
            format!("{}: {}", self.code, self.message)
        }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RpcError {}

/// Wire-compatible RPC error payload supporting both structured `{ code, message }` objects
/// and legacy plain string error messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum RpcErrorPayload {
    Structured(RpcError),
    Message(String),
}

impl RpcErrorPayload {
    pub fn into_string(&self) -> String {
        match self {
            RpcErrorPayload::Structured(err) => err.format_wire_string(),
            RpcErrorPayload::Message(msg) => msg.clone(),
        }
    }

    pub fn to_rpc_error(&self) -> RpcError {
        match self {
            RpcErrorPayload::Structured(err) => err.clone(),
            RpcErrorPayload::Message(msg) => RpcError::from_prefixed_string(msg),
        }
    }
}

impl From<RpcError> for RpcErrorPayload {
    fn from(value: RpcError) -> Self {
        RpcErrorPayload::Structured(value)
    }
}

impl From<String> for RpcErrorPayload {
    fn from(value: String) -> Self {
        RpcErrorPayload::Message(value)
    }
}

impl From<&str> for RpcErrorPayload {
    fn from(value: &str) -> Self {
        RpcErrorPayload::Message(value.to_string())
    }
}

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
        error: Option<RpcErrorPayload>,
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

struct PendingRequestGuard<'a> {
    bridge: &'a RpcBridge,
    id: u64,
    armed: bool,
}

impl<'a> PendingRequestGuard<'a> {
    fn new(bridge: &'a RpcBridge, id: u64) -> Self {
        Self {
            bridge,
            id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingRequestGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.bridge.cancel_request(self.id);
        }
    }
}

pub struct RpcBridge {
    next_id: AtomicU64,
    pending: PendingResponseMap,
    outbound_requests: Mutex<VecDeque<RpcMessage>>,
    outbound_notifications: Mutex<VecDeque<RpcMessage>>,
    outbound_notify: Arc<Notify>,
    has_active_transport: AtomicBool,
    reader_closed: AtomicBool,
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
            outbound_requests: Mutex::new(VecDeque::new()),
            outbound_notifications: Mutex::new(VecDeque::new()),
            outbound_notify: Arc::new(Notify::new()),
            has_active_transport: AtomicBool::new(false),
            reader_closed: AtomicBool::new(false),
        }
    }

    pub fn has_active_transport(&self) -> bool {
        self.has_active_transport.load(Ordering::SeqCst)
    }

    pub fn set_transport_active(&self, active: bool) {
        self.has_active_transport.store(active, Ordering::SeqCst);
    }

    pub fn is_reader_closed(&self) -> bool {
        self.reader_closed.load(Ordering::SeqCst)
    }

    pub fn set_reader_closed(&self, closed: bool) {
        self.reader_closed.store(closed, Ordering::SeqCst);
    }

    /// Create an outbound RPC request message, enqueue it for transport delivery,
    /// and return an async receiver for the response.
    pub fn create_request(
        &self,
        method: &str,
        params: Value,
    ) -> (RpcMessage, oneshot::Receiver<Result<Value, String>>) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        let msg = RpcMessage::Request {
            id,
            method: method.to_string(),
            params,
        };
        if self.reader_closed.load(Ordering::SeqCst) {
            let _ = tx.send(Err("Extension host transport closed".to_string()));
            return (msg, rx);
        }
        {
            let mut pending = self.pending.lock();
            if pending.len() >= 64 {
                pending.retain(|_, sender| !sender.is_closed());
            }
            pending.insert(id, tx);
        }
        if self.reader_closed.load(Ordering::SeqCst) {
            if let Some(tx) = self.pending.lock().remove(&id) {
                let _ = tx.send(Err("Extension host transport closed".to_string()));
            }
            return (msg, rx);
        }

        self.outbound_requests.lock().push_back(msg.clone());
        self.outbound_notify.notify_one();
        (msg, rx)
    }

    /// Create an outbound fire-and-forget RPC notification without registering a pending response waiter.
    pub fn create_notification(&self, channel: &str, data: Value) -> RpcMessage {
        RpcMessage::Notification {
            channel: channel.to_string(),
            data,
        }
    }

    /// Queue an outbound notification message for transport delivery.
    pub fn push_notification(&self, msg: RpcMessage) {
        self.outbound_notifications.lock().push_back(msg);
        self.outbound_notify.notify_one();
    }

    /// Create and queue an outbound notification message for transport delivery.
    pub fn notify(&self, channel: &str, data: Value) {
        self.push_notification(self.create_notification(channel, data));
    }

    /// Drain all queued outbound notifications in FIFO order.
    pub fn drain_notifications(&self) -> Vec<RpcMessage> {
        self.outbound_notifications.lock().drain(..).collect()
    }

    /// Drain all queued outbound requests in FIFO order.
    pub fn take_outbound_requests(&self) -> Vec<RpcMessage> {
        self.outbound_requests.lock().drain(..).collect()
    }

    /// Return the notify handle used to wake transport writer loops when outbound items arrive.
    pub fn outbound_notify(&self) -> &Arc<Notify> {
        &self.outbound_notify
    }

    /// Prune closed pending request entries (e.g. timed-out or dropped receivers) and return the pruned count.
    pub fn prune_closed_pending(&self) -> usize {
        let mut pending = self.pending.lock();
        let before = pending.len();
        pending.retain(|_, tx| !tx.is_closed());
        before.saturating_sub(pending.len())
    }

    /// Return the number of currently active pending outbound RPC requests awaiting responses.
    pub fn pending_count(&self) -> usize {
        let mut pending = self.pending.lock();
        pending.retain(|_, tx| !tx.is_closed());
        pending.len()
    }

    /// Cancel a pending outbound request and remove any queued unsent request with the same id.
    pub fn cancel_request(&self, id: u64) -> bool {
        let removed_pending = self.pending.lock().remove(&id).is_some();
        self.outbound_requests
            .lock()
            .retain(|msg| !matches!(msg, RpcMessage::Request { id: req_id, .. } if *req_id == id));
        removed_pending
    }

    /// Fail all currently pending outbound requests (e.g. when the transport shuts down).
    pub fn fail_all_pending(&self, reason: &str) -> usize {
        let drained: Vec<(u64, oneshot::Sender<Result<Value, String>>)> =
            self.pending.lock().drain().collect();
        let mut failed = 0usize;
        for (_, tx) in drained {
            if !tx.is_closed() && tx.send(Err(reason.to_string())).is_ok() {
                failed += 1;
            }
        }
        failed
    }

    /// Dispatch an incoming response payload to fulfill a pending request.
    pub fn handle_incoming_response_payload(
        &self,
        id: u64,
        result: Option<Value>,
        error: Option<RpcErrorPayload>,
    ) -> bool {
        let sender = self.pending.lock().remove(&id);
        if let Some(tx) = sender {
            let res = match error {
                Some(err) => Err(err.into_string()),
                None => Ok(result.unwrap_or(Value::Null)),
            };
            tx.send(res).is_ok()
        } else {
            false
        }
    }

    /// Dispatch an incoming response to fulfill a pending request.
    pub fn handle_incoming_response(
        &self,
        id: u64,
        result: Option<Value>,
        error: Option<String>,
    ) -> bool {
        self.handle_incoming_response_payload(id, result, error.map(RpcErrorPayload::Message))
    }

    /// Send an outbound RPC request and await its response with leak-free timeout cleanup.
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        if self.reader_closed.load(Ordering::SeqCst) {
            return Err("Extension host transport closed".to_string());
        }
        let (msg, rx) = self.create_request(method, params);
        let id = match msg {
            RpcMessage::Request { id, .. } => id,
            _ => unreachable!("create_request always produces RpcMessage::Request"),
        };
        let mut guard = PendingRequestGuard::new(self, id);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(res)) => {
                guard.disarm();
                res
            }
            Ok(Err(_)) => {
                guard.disarm();
                Err("RPC response channel closed".to_string())
            }
            Err(_) => Err(format!(
                "RPC request '{}' timed out after {:?}",
                method, timeout
            )),
        }
    }
}

/// Supported framing formats for live Rust <-> JS extension host transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RpcWireFormat {
    #[default]
    ContentLengthJson,
    ContentLengthMsgPack,
    NewlineJson,
}

pub type RpcBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Handler invoked by the live transport when the JS extension host sends an RPC request or notification.
pub trait RpcRequestHandler: Send + Sync + 'static {
    fn handle_request<'a>(
        &'a self,
        method: &'a str,
        params: &'a Value,
    ) -> RpcBoxFuture<'a, Result<Value, RpcError>>;

    fn handle_notification<'a>(
        &'a self,
        channel: &'a str,
        data: &'a Value,
    ) -> RpcBoxFuture<'a, ()> {
        Box::pin(async move {
            let _ = self.handle_request(channel, data).await;
        })
    }
}

/// Configuration for spawning a live Node.js extension host subprocess.
#[derive(Debug, Clone, Default)]
pub struct NodeExtHostConfig {
    pub node_binary: Option<PathBuf>,
    pub entry_script: Option<PathBuf>,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub wire_format: RpcWireFormat,
}

fn first_existing_file(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    for path in candidates {
        if path.is_file() {
            return Some(strip_windows_verbatim_prefix(path));
        }
    }
    None
}

fn find_in_path(names: &[&str]) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(strip_windows_verbatim_prefix(candidate));
            }
        }
    }
    None
}

/// Locate the Node.js binary across Windows and Unix hosts.
pub fn resolve_node_binary(override_path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = override_path {
        if p.is_file() {
            return Ok(strip_windows_verbatim_prefix(p.to_path_buf()));
        }
        return Err(format!(
            "Configured Node.js binary does not exist: {}",
            p.display()
        ));
    }

    if let Some(env_bin) = std::env::var_os("RUSTCODE_NODE_BIN") {
        if !env_bin.is_empty() {
            let p = PathBuf::from(&env_bin);
            if p.is_file() {
                return Ok(strip_windows_verbatim_prefix(p));
            }
            if p.components().count() == 1 {
                if let Some(name) = p.to_str() {
                    if let Some(found) = find_in_path(&[name]) {
                        return Ok(found);
                    }
                }
            }
            return Err(format!(
                "RUSTCODE_NODE_BIN does not point to a valid executable: {}",
                p.display()
            ));
        }
    }

    #[cfg(windows)]
    let path_names: &[&str] = &["node.exe", "node.cmd", "node"];
    #[cfg(not(windows))]
    let path_names: &[&str] = &["node"];

    if let Some(found) = find_in_path(path_names) {
        return Ok(found);
    }

    #[cfg(windows)]
    {
        let mut fallbacks = vec![
            PathBuf::from(r"C:\Program Files\nodejs\node.exe"),
            PathBuf::from(r"C:\Program Files (x86)\nodejs\node.exe"),
        ];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            fallbacks.push(PathBuf::from(local).join("Programs/nodejs/node.exe"));
        }
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            fallbacks.push(PathBuf::from(pf).join("nodejs/node.exe"));
        }
        if let Some(found) = first_existing_file(fallbacks) {
            return Ok(found);
        }
    }

    #[cfg(not(windows))]
    {
        let fallbacks = [
            PathBuf::from("/usr/local/bin/node"),
            PathBuf::from("/usr/bin/node"),
            PathBuf::from("/opt/homebrew/bin/node"),
        ];
        if let Some(found) = first_existing_file(fallbacks) {
            return Ok(found);
        }
    }

    Err("Could not locate Node.js binary on PATH or standard install locations".to_string())
}

/// Locate the `ext-host-js/rpc-bridge.js` entrypoint across development and installed layouts.
pub fn resolve_ext_host_entry(override_path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = override_path {
        if p.is_file() {
            let resolved = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            return Ok(strip_windows_verbatim_prefix(resolved));
        }
        return Err(format!(
            "Configured ext-host entry script does not exist: {}",
            p.display()
        ));
    }

    if let Some(env_entry) = std::env::var_os("RUSTCODE_EXT_HOST_ENTRY") {
        if !env_entry.is_empty() {
            let p = PathBuf::from(env_entry);
            if p.is_file() {
                let resolved = std::fs::canonicalize(&p).unwrap_or(p);
                return Ok(strip_windows_verbatim_prefix(resolved));
            }
            return Err(format!(
                "RUSTCODE_EXT_HOST_ENTRY does not exist: {}",
                p.display()
            ));
        }
    }

    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("ext-host-js/rpc-bridge.js"));
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe.parent();
        for _ in 0..6 {
            if let Some(dir) = cur {
                candidates.push(dir.join("ext-host-js/rpc-bridge.js"));
                cur = dir.parent();
            } else {
                break;
            }
        }
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ext-host-js/rpc-bridge.js"));

    if let Some(found) = first_existing_file(candidates) {
        let resolved = std::fs::canonicalize(&found).unwrap_or(found);
        return Ok(strip_windows_verbatim_prefix(resolved));
    }

    Err("Could not locate ext-host-js/rpc-bridge.js".to_string())
}

async fn write_rpc_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &RpcMessage,
    format: RpcWireFormat,
) -> Result<(), String> {
    match format {
        RpcWireFormat::ContentLengthJson => {
            let payload =
                serde_json::to_vec(msg).map_err(|e| format!("JSON serialization error: {}", e))?;
            write_framed_message(writer, &payload)
                .await
                .map_err(|e| format!("Failed writing framed JSON RPC message: {}", e))
        }
        RpcWireFormat::ContentLengthMsgPack => {
            let payload = msg.to_msgpack()?;
            write_framed_message(writer, &payload)
                .await
                .map_err(|e| format!("Failed writing framed MsgPack RPC message: {}", e))
        }
        RpcWireFormat::NewlineJson => {
            let mut line = msg.to_json()?;
            line.push('\n');
            writer
                .write_all(line.as_bytes())
                .await
                .map_err(|e| format!("Failed writing newline-json RPC message: {}", e))?;
            writer
                .flush()
                .await
                .map_err(|e| format!("Failed flushing newline-json RPC message: {}", e))
        }
    }
}

fn find_header_terminator(buf: &[u8]) -> Option<(usize, usize)> {
    let pos_crlf = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| (p, p + 4));
    let pos_lf = buf
        .windows(2)
        .position(|w| w == b"\n\n")
        .map(|p| (p, p + 2));
    match (pos_crlf, pos_lf) {
        (Some((h1, b1)), Some((h2, b2))) => {
            if h1 <= h2 {
                Some((h1, b1))
            } else {
                Some((h2, b2))
            }
        }
        (Some(pair), None) | (None, Some(pair)) => Some(pair),
        (None, None) => None,
    }
}

fn trim_leading_line_breaks(buf: &mut Vec<u8>) {
    let trim_count = buf
        .iter()
        .take_while(|&&b| b == b'\r' || b == b'\n')
        .count();
    if trim_count > 0 {
        buf.drain(0..trim_count);
    }
}

/// O(N) linear bounded `Content-Length` frame reader with pre-read `MAX_RPC_FRAME_BYTES` guard.
async fn read_content_length_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    leftover: &mut Vec<u8>,
) -> Result<Option<Vec<u8>>, std::io::Error> {
    trim_leading_line_breaks(leftover);

    let mut scan_from = 0usize;
    let (header_len, body_start) = loop {
        let search_end = leftover.len().min(MAX_RPC_HEADER_BYTES);
        if search_end > scan_from {
            if let Some((rel_h, rel_b)) = find_header_terminator(&leftover[scan_from..search_end])
            {
                break (scan_from + rel_h, scan_from + rel_b);
            }
        }

        if leftover.len() >= MAX_RPC_HEADER_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("RPC frame header exceeds {} bytes", MAX_RPC_HEADER_BYTES),
            ));
        }

        let prev_len = leftover.len();
        let mut temp = [0u8; 8192];
        let n = reader.read(&mut temp).await?;
        if n == 0 {
            if leftover.is_empty() {
                return Ok(None);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Stream closed with incomplete framed message",
            ));
        }

        if prev_len == 0 {
            let lead = temp[..n]
                .iter()
                .take_while(|&&b| b == b'\r' || b == b'\n')
                .count();
            leftover.extend_from_slice(&temp[lead..n]);
            scan_from = 0;
        } else {
            leftover.extend_from_slice(&temp[..n]);
            scan_from = prev_len.saturating_sub(3);
        }
    };

    let header_str = std::str::from_utf8(&leftover[..header_len]).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Header is not valid UTF-8: {}", e),
        )
    })?;

    let mut content_length: Option<usize> = None;
    for line in header_str.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.trim().eq_ignore_ascii_case("Content-Length") {
                let val = v.trim();
                let len: usize = val.parse().map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Invalid Content-Length value: {}", val),
                    )
                })?;
                content_length = Some(len);
            }
        }
    }

    let length = content_length.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Missing Content-Length header",
        )
    })?;

    if length > MAX_RPC_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "RPC frame exceeds maximum size ({} > {})",
                length, MAX_RPC_FRAME_BYTES
            ),
        ));
    }

    let available = leftover.len().saturating_sub(body_start);
    if available >= length {
        let body = leftover[body_start..body_start + length].to_vec();
        leftover.drain(0..body_start + length);
        return Ok(Some(body));
    }

    let mut body = vec![0u8; length];
    if available > 0 {
        body[..available].copy_from_slice(&leftover[body_start..]);
    }
    leftover.clear();
    reader.read_exact(&mut body[available..]).await?;
    Ok(Some(body))
}

async fn read_rpc_message<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    frame_buf: &mut Vec<u8>,
    line_buf: &mut String,
    format: RpcWireFormat,
) -> Result<Option<RpcMessage>, String> {
    match format {
        RpcWireFormat::ContentLengthJson => {
            match read_content_length_frame(reader, frame_buf).await {
                Ok(Some(bytes)) => {
                    let msg: RpcMessage = serde_json::from_slice(&bytes)
                        .map_err(|e| format!("JSON RPC decode error: {}", e))?;
                    Ok(Some(msg))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(format!("Framed JSON RPC read error: {}", e)),
            }
        }
        RpcWireFormat::ContentLengthMsgPack => {
            match read_content_length_frame(reader, frame_buf).await {
                Ok(Some(bytes)) => {
                    let msg = RpcMessage::from_msgpack(&bytes)?;
                    Ok(Some(msg))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(format!("Framed MsgPack RPC read error: {}", e)),
            }
        }
        RpcWireFormat::NewlineJson => loop {
            line_buf.clear();
            let n = reader
                .read_line(line_buf)
                .await
                .map_err(|e| format!("Newline JSON RPC read error: {}", e))?;
            if n == 0 {
                return Ok(None);
            }
            let trimmed = line_buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.len() > MAX_RPC_FRAME_BYTES {
                return Err(format!(
                    "RPC line exceeds maximum size ({} > {})",
                    trimmed.len(),
                    MAX_RPC_FRAME_BYTES
                ));
            }
            let msg = RpcMessage::from_json(trimmed)?;
            return Ok(Some(msg));
        },
    }
}

/// Live framed RPC transport running either against a spawned Node.js child process
/// or an arbitrary async `AsyncRead` / `AsyncWrite` stream pair.
pub struct NodeExtHostProcess {
    rpc_bridge: Arc<RpcBridge>,
    child: Option<Child>,
    is_running: Arc<AtomicBool>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    shutdown_tx: watch::Sender<bool>,
    reader_handle: Option<JoinHandle<()>>,
    writer_handle: Option<JoinHandle<()>>,
    stderr_handle: Option<JoinHandle<()>>,
}

impl NodeExtHostProcess {
    /// Spawn a live Node.js extension host subprocess wired to `LanguageFeatureBridge`.
    pub fn spawn(
        bridge: Arc<crate::provider::LanguageFeatureBridge>,
        config: NodeExtHostConfig,
    ) -> Result<Self, String> {
        let rpc_bridge = Arc::clone(&bridge.rpc_bridge);
        let handler: Arc<dyn RpcRequestHandler> = bridge;
        Self::spawn_with_handler(rpc_bridge, handler, config)
    }

    /// Spawn a live Node.js extension host subprocess with an explicit `RpcBridge` and `RpcRequestHandler`.
    pub fn spawn_with_handler(
        rpc_bridge: Arc<RpcBridge>,
        handler: Arc<dyn RpcRequestHandler>,
        config: NodeExtHostConfig,
    ) -> Result<Self, String> {
        let node_bin = resolve_node_binary(config.node_binary.as_deref())?;
        let entry_script = resolve_ext_host_entry(config.entry_script.as_deref())?;

        let mut cmd = Command::new(&node_bin);
        cmd.arg(&entry_script);
        for arg in &config.args {
            cmd.arg(arg);
        }
        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }
        let has_format_env = config.env.iter().any(|(k, _)| k == "RUSTCODE_RPC_FORMAT");
        if !has_format_env {
            let fmt_str = match config.wire_format {
                RpcWireFormat::ContentLengthJson | RpcWireFormat::ContentLengthMsgPack => {
                    "content-length"
                }
                RpcWireFormat::NewlineJson => "newline-json",
            };
            cmd.env("RUSTCODE_RPC_FORMAT", fmt_str);
        }
        for (k, v) in &config.env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to spawn Node.js extension host ({} {}): {}",
                node_bin.display(),
                entry_script.display(),
                e
            )
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Failed to capture Node.js stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to capture Node.js stdout".to_string())?;
        let stderr = child.stderr.take();

        let mut proc = Self::from_io_with_handler(
            rpc_bridge,
            handler,
            stdout,
            stdin,
            config.wire_format,
        );
        proc.child = Some(child);

        if let Some(stderr_pipe) = stderr {
            let tail = Arc::clone(&proc.stderr_tail);
            proc.stderr_handle = Some(tokio::spawn(async move {
                let mut reader = BufReader::new(stderr_pipe);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
                            let mut guard = tail.lock();
                            if guard.len() >= STDERR_TAIL_LIMIT {
                                guard.pop_front();
                            }
                            guard.push_back(trimmed);
                        }
                    }
                }
            }));
        }

        Ok(proc)
    }

    /// Construct a live RPC transport over arbitrary `AsyncRead`/`AsyncWrite` streams
    /// wired to `LanguageFeatureBridge` (ideal for `tokio::io::duplex` tests).
    pub fn from_io<R, W>(
        bridge: Arc<crate::provider::LanguageFeatureBridge>,
        reader: R,
        writer: W,
        format: RpcWireFormat,
    ) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let rpc_bridge = Arc::clone(&bridge.rpc_bridge);
        let handler: Arc<dyn RpcRequestHandler> = bridge;
        Self::from_io_with_handler(rpc_bridge, handler, reader, writer, format)
    }

    /// Construct a live RPC transport over arbitrary `AsyncRead`/`AsyncWrite` streams
    /// with an explicit `RpcBridge` and `RpcRequestHandler`.
    pub fn from_io_with_handler<R, W>(
        rpc_bridge: Arc<RpcBridge>,
        handler: Arc<dyn RpcRequestHandler>,
        reader: R,
        writer: W,
        format: RpcWireFormat,
    ) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        enum IncomingNotifWork {
            Notification { channel: String, data: Value },
            Barrier(oneshot::Sender<()>),
        }

        let is_running = Arc::new(AtomicBool::new(true));
        rpc_bridge.set_reader_closed(false);
        rpc_bridge.set_transport_active(true);
        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (response_tx, mut response_rx) = mpsc::unbounded_channel::<RpcMessage>();
        let (notif_tx, mut notif_rx) = mpsc::unbounded_channel::<IncomingNotifWork>();

        struct AbortHandleOnDrop(Option<JoinHandle<()>>);
        impl AbortHandleOnDrop {
            fn abort(&mut self) {
                if let Some(handle) = self.0.take() {
                    handle.abort();
                }
            }

            async fn join(&mut self) {
                if let Some(handle) = self.0.take() {
                    let _ = handle.await;
                }
            }
        }
        impl Drop for AbortHandleOnDrop {
            fn drop(&mut self) {
                self.abort();
            }
        }

        let notif_handler = Arc::clone(&handler);
        let notif_worker_handle = tokio::spawn(async move {
            while let Some(work) = notif_rx.recv().await {
                match work {
                    IncomingNotifWork::Notification { channel, data } => {
                        notif_handler.handle_notification(&channel, &data).await;
                    }
                    IncomingNotifWork::Barrier(tx) => {
                        let _ = tx.send(());
                    }
                }
            }
        });

        let writer_bridge = Arc::clone(&rpc_bridge);
        let writer_running = Arc::clone(&is_running);
        let mut writer_shutdown = shutdown_rx.clone();
        let writer_handle = tokio::spawn(async move {
            let mut writer = writer;
            let outbound_notify = Arc::clone(writer_bridge.outbound_notify());
            loop {
                let notified = outbound_notify.notified();
                let mut wrote_any = false;

                // Collect responses first, then drain outbound requests and notifications
                // BEFORE writing responses so notifications emitted during request handling
                // always precede the response frame on the wire.
                let mut pending_responses = Vec::new();
                while let Ok(resp) = response_rx.try_recv() {
                    pending_responses.push(resp);
                }

                for req in writer_bridge.take_outbound_requests() {
                    if write_rpc_message(&mut writer, &req, format).await.is_err() {
                        writer_running.store(false, Ordering::SeqCst);
                        return;
                    }
                    wrote_any = true;
                }

                for notif in writer_bridge.drain_notifications() {
                    if write_rpc_message(&mut writer, &notif, format).await.is_err() {
                        writer_running.store(false, Ordering::SeqCst);
                        return;
                    }
                    wrote_any = true;
                }

                for resp in pending_responses {
                    if write_rpc_message(&mut writer, &resp, format).await.is_err() {
                        writer_running.store(false, Ordering::SeqCst);
                        return;
                    }
                    wrote_any = true;
                }

                if *writer_shutdown.borrow() {
                    break;
                }
                if wrote_any {
                    continue;
                }

                tokio::select! {
                    maybe_resp = response_rx.recv() => {
                        match maybe_resp {
                            Some(resp) => {
                                for req in writer_bridge.take_outbound_requests() {
                                    if write_rpc_message(&mut writer, &req, format).await.is_err() {
                                        writer_running.store(false, Ordering::SeqCst);
                                        return;
                                    }
                                }
                                for notif in writer_bridge.drain_notifications() {
                                    if write_rpc_message(&mut writer, &notif, format).await.is_err() {
                                        writer_running.store(false, Ordering::SeqCst);
                                        return;
                                    }
                                }
                                if write_rpc_message(&mut writer, &resp, format).await.is_err() {
                                    writer_running.store(false, Ordering::SeqCst);
                                    return;
                                }
                            }
                            None => {
                                for req in writer_bridge.take_outbound_requests() {
                                    if write_rpc_message(&mut writer, &req, format).await.is_err() {
                                        writer_running.store(false, Ordering::SeqCst);
                                        writer_bridge.set_transport_active(false);
                                        return;
                                    }
                                }
                                for notif in writer_bridge.drain_notifications() {
                                    if write_rpc_message(&mut writer, &notif, format).await.is_err() {
                                        writer_running.store(false, Ordering::SeqCst);
                                        writer_bridge.set_transport_active(false);
                                        return;
                                    }
                                }
                                let _ = writer.shutdown().await;
                                break;
                            }
                        }
                    }
                    _ = notified => {}
                    changed = writer_shutdown.changed() => {
                        if changed.is_err() || *writer_shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        let reader_bridge = Arc::clone(&rpc_bridge);
        let reader_running = Arc::clone(&is_running);
        let reader_shutdown_tx = shutdown_tx.clone();
        let mut reader_shutdown_rx = shutdown_rx.clone();
        let reader_handle = tokio::spawn(async move {
            let mut buf_reader = BufReader::new(reader);
            let mut frame_buf = Vec::new();
            let mut line_buf = String::new();
            let mut in_flight = tokio::task::JoinSet::new();
            let mut notif_tx = Some(notif_tx);
            let mut notif_worker = AbortHandleOnDrop(Some(notif_worker_handle));
            loop {
                while in_flight.try_join_next().is_some() {}
                let read_res = tokio::select! {
                    res = read_rpc_message(&mut buf_reader, &mut frame_buf, &mut line_buf, format) => res,
                    changed = reader_shutdown_rx.changed() => {
                        if changed.is_err() || *reader_shutdown_rx.borrow() {
                            reader_bridge.set_reader_closed(true);
                            drop(notif_tx.take());
                            notif_worker.abort();
                            in_flight.abort_all();
                            drop(response_tx);
                            reader_running.store(false, Ordering::SeqCst);
                            reader_bridge.fail_all_pending("Extension host shut down");
                            break;
                        }
                        continue;
                    }
                };
                match read_res {
                    Ok(Some(RpcMessage::Response { id, result, error })) => {
                        reader_bridge.handle_incoming_response_payload(id, result, error);
                    }
                    Ok(Some(RpcMessage::Request { id, method, params })) => {
                        // Causal barrier: ensure all preceding incoming notifications have
                        // completed before this request runs, without blocking reader_handle
                        // from reading subsequent Response frames.
                        let fence_rx = notif_tx.as_ref().and_then(|tx| {
                            let (fence_tx, fence_rx) = oneshot::channel();
                            tx.send(IncomingNotifWork::Barrier(fence_tx))
                                .ok()
                                .map(|()| fence_rx)
                        });
                        let handler_clone = Arc::clone(&handler);
                        let resp_tx = response_tx.clone();
                        in_flight.spawn(async move {
                            if let Some(rx) = fence_rx {
                                let _ = rx.await;
                            }
                            let resp = match handler_clone.handle_request(&method, &params).await {
                                Ok(val) => RpcMessage::Response {
                                    id,
                                    result: Some(val),
                                    error: None,
                                },
                                Err(err) => RpcMessage::Response {
                                    id,
                                    result: None,
                                    error: Some(RpcErrorPayload::Structured(err)),
                                },
                            };
                            let _ = resp_tx.send(resp);
                        });
                    }
                    Ok(Some(RpcMessage::Notification { channel, data })) => {
                        if let Some(tx) = notif_tx.as_ref() {
                            let _ = tx.send(IncomingNotifWork::Notification { channel, data });
                        }
                    }
                    Ok(None) => {
                        reader_bridge.set_reader_closed(true);
                        reader_bridge.fail_all_pending("Extension host transport closed");
                        drop(notif_tx.take());
                        notif_worker.join().await;
                        while in_flight.join_next().await.is_some() {}
                        drop(response_tx);
                        reader_running.store(false, Ordering::SeqCst);
                        reader_bridge.set_transport_active(false);
                        reader_bridge.fail_all_pending("Extension host transport closed");
                        break;
                    }
                    Err(err) => {
                        reader_bridge.set_reader_closed(true);
                        drop(notif_tx.take());
                        notif_worker.abort();
                        in_flight.abort_all();
                        drop(response_tx);
                        reader_running.store(false, Ordering::SeqCst);
                        reader_bridge.set_transport_active(false);
                        let _ = reader_shutdown_tx.send(true);
                        reader_bridge.fail_all_pending(&err);
                        break;
                    }
                }
            }
        });

        Self {
            rpc_bridge,
            child: None,
            is_running,
            stderr_tail,
            shutdown_tx,
            reader_handle: Some(reader_handle),
            writer_handle: Some(writer_handle),
            stderr_handle: None,
        }
    }

    pub fn rpc_bridge(&self) -> &Arc<RpcBridge> {
        &self.rpc_bridge
    }

    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail.lock().iter().cloned().collect()
    }

    /// Gracefully shut down the transport and child process, closing stdin and reaping the child.
    pub async fn shutdown(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        let _ = self.shutdown_tx.send(true);
        self.is_running.store(false, Ordering::SeqCst);
        self.rpc_bridge.set_reader_closed(true);
        self.rpc_bridge.set_transport_active(false);

        if let Some(wh) = self.writer_handle.take() {
            let mut wh = wh;
            if tokio::time::timeout(Duration::from_millis(500), &mut wh)
                .await
                .is_err()
            {
                wh.abort();
            }
        }

        let mut exit_status = None;
        let mut wait_err: Option<String> = None;
        if let Some(mut child) = self.child.take() {
            match tokio::time::timeout(Duration::from_millis(500), child.wait()).await {
                Ok(Ok(status)) => exit_status = Some(status),
                Ok(Err(e)) => {
                    wait_err = Some(format!("Failed waiting for Node.js child exit: {}", e));
                }
                Err(_) => {
                    let _ = child.kill().await;
                    exit_status = child.wait().await.ok();
                }
            }
        }

        if let Some(rh) = self.reader_handle.take() {
            let mut rh = rh;
            if tokio::time::timeout(Duration::from_millis(250), &mut rh)
                .await
                .is_err()
            {
                rh.abort();
            }
        }

        if let Some(sh) = self.stderr_handle.take() {
            let mut sh = sh;
            if tokio::time::timeout(Duration::from_millis(250), &mut sh)
                .await
                .is_err()
            {
                sh.abort();
            }
        }

        self.rpc_bridge.fail_all_pending("Extension host shut down");
        if let Some(err) = wait_err {
            return Err(err);
        }
        Ok(exit_status)
    }
}

impl Drop for NodeExtHostProcess {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        self.is_running.store(false, Ordering::SeqCst);
        self.rpc_bridge.set_reader_closed(true);
        self.rpc_bridge.set_transport_active(false);
        if let Some(rh) = self.reader_handle.take() {
            rh.abort();
        }
        if let Some(wh) = self.writer_handle.take() {
            wh.abort();
        }
        if let Some(sh) = self.stderr_handle.take() {
            sh.abort();
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let _ = child.try_wait();
        }
        self.rpc_bridge.fail_all_pending("Extension host dropped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::duplex;

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
    fn test_rpc_response_error_payload_json_and_msgpack_roundtrip() {
        let cases = [
            RpcMessage::Response {
                id: 1,
                result: Some(json!({ "ok": true })),
                error: None,
            },
            RpcMessage::Response {
                id: 2,
                result: None,
                error: Some(RpcErrorPayload::Structured(RpcError::new(
                    "ENOENT",
                    "missing file",
                ))),
            },
            RpcMessage::Response {
                id: 3,
                result: None,
                error: Some(RpcErrorPayload::Message("legacy error".to_string())),
            },
        ];

        for msg in &cases {
            let json_str = msg.to_json().unwrap();
            let from_json = RpcMessage::from_json(&json_str).unwrap();
            assert_eq!(&from_json, msg, "JSON roundtrip mismatch for {}", json_str);

            let msgpack = msg.to_msgpack().unwrap();
            let from_msgpack = RpcMessage::from_msgpack(&msgpack).unwrap();
            assert_eq!(&from_msgpack, msg, "MsgPack roundtrip mismatch");
        }
    }

    #[test]
    fn test_rpc_bridge_request_response() {
        let bridge = RpcBridge::new();
        let (req, mut rx) = bridge.create_request(
            "workspace.getConfiguration",
            json!({ "section": "editor" }),
        );

        let queued = bridge.take_outbound_requests();
        assert_eq!(queued, vec![req.clone()]);

        if let RpcMessage::Request { id, .. } = req {
            let handled =
                bridge.handle_incoming_response(id, Some(json!({ "fontSize": 14 })), None);
            assert!(handled);

            let res = rx.try_recv().unwrap().unwrap();
            assert_eq!(res, json!({ "fontSize": 14 }));
            assert_eq!(bridge.pending_count(), 0);
        } else {
            panic!("Expected request message");
        }
    }

    #[test]
    fn test_rpc_bridge_queues_and_dropped_rx_cleanup() {
        let bridge = RpcBridge::new();
        bridge.notify("workspace.onDidChangeConfiguration", json!({ "affects": ["editor"] }));
        let notif = bridge.create_notification("custom.chan", json!(123));
        bridge.push_notification(notif.clone());
        let drained = bridge.drain_notifications();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[1], notif);
        assert!(bridge.drain_notifications().is_empty());

        let (_req, rx) = bridge.create_request("slow.op", json!({}));
        assert_eq!(bridge.pending_count(), 1);
        drop(rx);
        assert_eq!(bridge.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_rpc_bridge_request_timeout_cleans_up_pending_and_outbound() {
        let bridge = RpcBridge::new();
        let err = bridge
            .request("slow.method", json!({}), Duration::from_millis(15))
            .await
            .unwrap_err();
        assert!(err.contains("timed out"), "unexpected err: {}", err);
        assert_eq!(bridge.pending_count(), 0);
        assert!(bridge.take_outbound_requests().is_empty());
    }

    struct EchoReentrantHandler {
        rpc: Arc<RpcBridge>,
    }

    impl RpcRequestHandler for EchoReentrantHandler {
        fn handle_request<'a>(
            &'a self,
            method: &'a str,
            params: &'a Value,
        ) -> RpcBoxFuture<'a, Result<Value, RpcError>> {
            Box::pin(async move {
                match method {
                    "host.ping" => Ok(json!({ "pong": params })),
                    "host.fail" => Err(RpcError::new("EACCES", "denied")),
                    "host.reenter" => {
                        let peer = self
                            .rpc
                            .request("peer.subcall", json!({ "n": 7 }), Duration::from_secs(2))
                            .await
                            .map_err(RpcError::internal)?;
                        Ok(json!({ "nested": peer }))
                    }
                    _ => Err(RpcError::method_not_found(method)),
                }
            })
        }
    }

    #[tokio::test]
    async fn test_duplex_transport_json_msgpack_newline_and_reentrant_requests() {
        for format in [
            RpcWireFormat::ContentLengthJson,
            RpcWireFormat::ContentLengthMsgPack,
            RpcWireFormat::NewlineJson,
        ] {
            let rpc = Arc::new(RpcBridge::new());
            let handler = Arc::new(EchoReentrantHandler {
                rpc: Arc::clone(&rpc),
            });
            let (rust_read, mut js_write) = duplex(8192);
            let (mut js_read, rust_write) = duplex(8192);

            let mut proc = NodeExtHostProcess::from_io_with_handler(
                Arc::clone(&rpc),
                handler,
                rust_read,
                rust_write,
                format,
            );

            // Simulate JS peer in background handling reentrant request from Rust
            let peer_task = tokio::spawn(async move {
                let mut reader = BufReader::new(&mut js_read);
                let mut fbuf = Vec::new();
                let mut lbuf = String::new();

                // 1. JS sends host.reenter request
                let req = RpcMessage::Request {
                    id: 100,
                    method: "host.reenter".to_string(),
                    params: json!({}),
                };
                write_rpc_message(&mut js_write, &req, format).await.unwrap();

                // 2. JS receives peer.subcall request from Rust while host.reenter is awaiting!
                let sub = read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, format)
                    .await
                    .unwrap()
                    .unwrap();
                let sub_id = match sub {
                    RpcMessage::Request { id, method, params } => {
                        assert_eq!(method, "peer.subcall");
                        assert_eq!(params["n"], json!(7));
                        id
                    }
                    other => panic!("Expected peer.subcall request, got {:?}", other),
                };

                // 3. JS replies to peer.subcall
                let sub_resp = RpcMessage::Response {
                    id: sub_id,
                    result: Some(json!({ "ok": 42 })),
                    error: None,
                };
                write_rpc_message(&mut js_write, &sub_resp, format)
                    .await
                    .unwrap();

                // 4. JS receives final response to host.reenter
                let final_resp = read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, format)
                    .await
                    .unwrap()
                    .unwrap();
                match final_resp {
                    RpcMessage::Response { id, result, error } => {
                        assert_eq!(id, 100);
                        assert!(error.is_none());
                        assert_eq!(result, Some(json!({ "nested": { "ok": 42 } })));
                    }
                    other => panic!("Expected final response, got {:?}", other),
                }

                // 5. JS sends host.fail and checks structured error payload
                let fail_req = RpcMessage::Request {
                    id: 101,
                    method: "host.fail".to_string(),
                    params: json!({}),
                };
                write_rpc_message(&mut js_write, &fail_req, format)
                    .await
                    .unwrap();
                let fail_resp = read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, format)
                    .await
                    .unwrap()
                    .unwrap();
                match fail_resp {
                    RpcMessage::Response { id, result, error } => {
                        assert_eq!(id, 101);
                        assert!(result.is_none());
                        assert_eq!(
                            error,
                            Some(RpcErrorPayload::Structured(RpcError::new(
                                "EACCES", "denied"
                            )))
                        );
                    }
                    other => panic!("Expected error response, got {:?}", other),
                }
            });

            peer_task.await.unwrap();
            assert_eq!(rpc.pending_count(), 0);
            proc.shutdown().await.unwrap();
            assert!(!proc.is_running());
        }
    }

    #[tokio::test]
    async fn test_content_length_4mb_frame_roundtrips_linearly_under_1s() {
        let rpc = Arc::new(RpcBridge::new());
        let handler = Arc::new(EchoReentrantHandler {
            rpc: Arc::clone(&rpc),
        });
        let (rust_read, mut js_write) = duplex(65536);
        let (mut js_read, rust_write) = duplex(65536);

        let mut proc = NodeExtHostProcess::from_io_with_handler(
            Arc::clone(&rpc),
            handler,
            rust_read,
            rust_write,
            RpcWireFormat::ContentLengthJson,
        );

        let big_str = "x".repeat(4 * 1024 * 1024);
        let req = RpcMessage::Request {
            id: 777,
            method: "host.ping".to_string(),
            params: json!({ "blob": big_str }),
        };

        let start = std::time::Instant::now();
        let writer_task = tokio::spawn(async move {
            write_rpc_message(&mut js_write, &req, RpcWireFormat::ContentLengthJson)
                .await
                .unwrap();
            js_write
        });

        let mut reader = BufReader::new(&mut js_read);
        let mut fbuf = Vec::new();
        let mut lbuf = String::new();
        let resp = tokio::time::timeout(
            Duration::from_secs(2),
            read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, RpcWireFormat::ContentLengthJson),
        )
        .await
        .expect("4MB frame timed out")
        .unwrap()
        .unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(1),
            "4MB ContentLengthJson roundtrip took {:?} (expected < 1s)",
            elapsed
        );
        match resp {
            RpcMessage::Response { id, result, error } => {
                assert_eq!(id, 777);
                assert!(error.is_none());
                assert_eq!(
                    result.unwrap()["pong"]["blob"].as_str().unwrap().len(),
                    4 * 1024 * 1024
                );
            }
            other => panic!("Expected Response, got {:?}", other),
        }

        let _keep_open = writer_task.await.unwrap();
        proc.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_oversized_content_length_header_rejects_immediately_without_hanging() {
        let rpc = Arc::new(RpcBridge::new());
        let handler = Arc::new(EchoReentrantHandler {
            rpc: Arc::clone(&rpc),
        });
        let (rust_read, mut js_write) = duplex(8192);
        let (_js_read, rust_write) = duplex(8192);

        let mut proc = NodeExtHostProcess::from_io_with_handler(
            Arc::clone(&rpc),
            handler,
            rust_read,
            rust_write,
            RpcWireFormat::ContentLengthJson,
        );

        let (_req, pending_rx) = rpc.create_request("peer.op", json!({}));
        // Keep js_write open after sending an oversized Content-Length header
        js_write
            .write_all(b"Content-Length: 100000000\r\n\r\n{}")
            .await
            .unwrap();
        js_write.flush().await.unwrap();

        let err = tokio::time::timeout(Duration::from_secs(1), pending_rx)
            .await
            .expect("Pending request hung instead of failing immediately on oversized header")
            .unwrap()
            .unwrap_err();
        assert!(
            err.contains("exceeds maximum size"),
            "Expected frame size limit error, got: {}",
            err
        );

        // Give reader_handle a moment to finish updating atomic flag if not already false
        for _ in 0..20 {
            if !proc.is_running() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            !proc.is_running(),
            "NodeExtHostProcess should stop immediately on oversized Content-Length"
        );
        proc.shutdown().await.unwrap();
    }

    struct DelayedEchoHandler;

    impl RpcRequestHandler for DelayedEchoHandler {
        fn handle_request<'a>(
            &'a self,
            method: &'a str,
            params: &'a Value,
        ) -> RpcBoxFuture<'a, Result<Value, RpcError>> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(25)).await;
                Ok(json!({ "method": method, "params": params }))
            })
        }
    }

    #[tokio::test]
    async fn test_half_close_eof_delivers_inflight_response_before_writer_exits() {
        let rpc = Arc::new(RpcBridge::new());
        let handler = Arc::new(DelayedEchoHandler);
        let (rust_read, mut js_write) = duplex(8192);
        let (mut js_read, rust_write) = duplex(8192);

        let mut proc = NodeExtHostProcess::from_io_with_handler(
            Arc::clone(&rpc),
            handler,
            rust_read,
            rust_write,
            RpcWireFormat::ContentLengthJson,
        );

        let req = RpcMessage::Request {
            id: 55,
            method: "commands.getCommands".to_string(),
            params: json!({ "filterInternal": true }),
        };
        write_rpc_message(&mut js_write, &req, RpcWireFormat::ContentLengthJson)
            .await
            .unwrap();
        // Half-close write side immediately after sending valid request
        drop(js_write);

        let mut raw_out = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), js_read.read_to_end(&mut raw_out))
            .await
            .expect("Timed out waiting for response + EOF after half-close")
            .unwrap();

        let mut cursor = &raw_out[..];
        let mut leftover = Vec::new();
        let frame = read_content_length_frame(&mut cursor, &mut leftover)
            .await
            .unwrap()
            .expect("Expected complete Response frame before EOF");
        let resp: RpcMessage = serde_json::from_slice(&frame).unwrap();
        match resp {
            RpcMessage::Response { id, result, error } => {
                assert_eq!(id, 55);
                assert!(error.is_none());
                assert_eq!(result.unwrap()["method"], "commands.getCommands");
            }
            other => panic!("Expected Response, got {:?}", other),
        }

        proc.shutdown().await.unwrap();
    }

    struct NotifyDuringInflightHandler {
        rpc: Arc<RpcBridge>,
    }

    impl RpcRequestHandler for NotifyDuringInflightHandler {
        fn handle_request<'a>(
            &'a self,
            method: &'a str,
            params: &'a Value,
        ) -> RpcBoxFuture<'a, Result<Value, RpcError>> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                self.rpc.notify(
                    "test.inflightNotif",
                    json!({ "method": method, "params": params }),
                );
                Ok(json!({ "ok": true }))
            })
        }
    }

    #[tokio::test]
    async fn test_half_close_eof_drains_notifications_before_writer_exits() {
        let rpc = Arc::new(RpcBridge::new());
        assert!(!rpc.has_active_transport());
        let handler = Arc::new(NotifyDuringInflightHandler {
            rpc: Arc::clone(&rpc),
        });
        let (rust_read, mut js_write) = duplex(8192);
        let (mut js_read, rust_write) = duplex(8192);

        let mut proc = NodeExtHostProcess::from_io_with_handler(
            Arc::clone(&rpc),
            handler,
            rust_read,
            rust_write,
            RpcWireFormat::ContentLengthJson,
        );
        assert!(rpc.has_active_transport());

        let req = RpcMessage::Request {
            id: 91,
            method: "test.triggerNotif".to_string(),
            params: json!({ "k": 1 }),
        };
        write_rpc_message(&mut js_write, &req, RpcWireFormat::ContentLengthJson)
            .await
            .unwrap();
        drop(js_write);

        let mut raw_out = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), js_read.read_to_end(&mut raw_out))
            .await
            .expect("Timed out waiting for response+notification+EOF")
            .unwrap();

        let mut cursor = &raw_out[..];
        let mut leftover = Vec::new();
        let mut seen_response = false;
        let mut seen_notification = false;
        while let Some(frame) = read_content_length_frame(&mut cursor, &mut leftover)
            .await
            .unwrap()
        {
            let msg: RpcMessage = serde_json::from_slice(&frame).unwrap();
            match msg {
                RpcMessage::Response { id: 91, result, error } => {
                    assert!(error.is_none());
                    assert_eq!(result.unwrap()["ok"], true);
                    seen_response = true;
                }
                RpcMessage::Notification { channel, data } => {
                    assert_eq!(channel, "test.inflightNotif");
                    assert_eq!(data["method"], "test.triggerNotif");
                    seen_notification = true;
                }
                other => panic!("Unexpected RPC message: {:?}", other),
            }
        }
        assert!(seen_response, "Expected Response frame before EOF");
        assert!(seen_notification, "Expected Notification frame drained before EOF");

        proc.shutdown().await.unwrap();
        assert!(!rpc.has_active_transport());
    }

    struct ImmediateNotifyBeforeResponseHandler {
        rpc: Arc<RpcBridge>,
    }

    impl RpcRequestHandler for ImmediateNotifyBeforeResponseHandler {
        fn handle_request<'a>(
            &'a self,
            method: &'a str,
            params: &'a Value,
        ) -> RpcBoxFuture<'a, Result<Value, RpcError>> {
            Box::pin(async move {
                self.rpc.notify(
                    "workspace.onDidChangeTextDocument",
                    json!({ "method": method, "seq": params["seq"] }),
                );
                Ok(json!({ "applied": true, "seq": params["seq"] }))
            })
        }
    }

    #[tokio::test]
    async fn test_notification_written_before_response_when_both_ready() {
        let rpc = Arc::new(RpcBridge::new());
        let handler = Arc::new(ImmediateNotifyBeforeResponseHandler {
            rpc: Arc::clone(&rpc),
        });
        let (rust_read, mut js_write) = duplex(65536);
        let (mut js_read, rust_write) = duplex(65536);

        let mut proc = NodeExtHostProcess::from_io_with_handler(
            Arc::clone(&rpc),
            handler,
            rust_read,
            rust_write,
            RpcWireFormat::ContentLengthJson,
        );

        let iterations = 20u64;
        for seq in 1..=iterations {
            let req = RpcMessage::Request {
                id: seq,
                method: "workspace.applyEdit".to_string(),
                params: json!({ "seq": seq }),
            };
            write_rpc_message(&mut js_write, &req, RpcWireFormat::ContentLengthJson)
                .await
                .unwrap();

            let mut frame_buf = Vec::new();
            let mut line_buf = String::new();
            let first = tokio::time::timeout(
                Duration::from_secs(2),
                read_rpc_message(
                    &mut BufReader::new(&mut js_read),
                    &mut frame_buf,
                    &mut line_buf,
                    RpcWireFormat::ContentLengthJson,
                ),
            )
            .await
            .expect("timed out waiting for notification")
            .unwrap()
            .expect("unexpected EOF before notification");

            let second = tokio::time::timeout(
                Duration::from_secs(2),
                read_rpc_message(
                    &mut BufReader::new(&mut js_read),
                    &mut frame_buf,
                    &mut line_buf,
                    RpcWireFormat::ContentLengthJson,
                ),
            )
            .await
            .expect("timed out waiting for response")
            .unwrap()
            .expect("unexpected EOF before response");

            match first {
                RpcMessage::Notification { channel, data } => {
                    assert_eq!(channel, "workspace.onDidChangeTextDocument");
                    assert_eq!(data["seq"], json!(seq));
                }
                other => panic!(
                    "Notification must arrive before Response on iteration {}: got {:?}",
                    seq, other
                ),
            }
            match second {
                RpcMessage::Response { id, result, error } => {
                    assert_eq!(id, seq);
                    assert!(error.is_none());
                    assert_eq!(result.unwrap()["seq"], json!(seq));
                }
                other => panic!(
                    "Expected Response second on iteration {}: got {:?}",
                    seq, other
                ),
            }
        }

        drop(js_write);
        proc.shutdown().await.unwrap();
    }

    struct OrderedNotificationHandler {
        rpc: Arc<RpcBridge>,
        seen: Arc<Mutex<Vec<u64>>>,
    }

    impl RpcRequestHandler for OrderedNotificationHandler {
        fn handle_request<'a>(
            &'a self,
            method: &'a str,
            _params: &'a Value,
        ) -> RpcBoxFuture<'a, Result<Value, RpcError>> {
            Box::pin(async move { Err(RpcError::method_not_found(method)) })
        }

        fn handle_notification<'a>(
            &'a self,
            channel: &'a str,
            data: &'a Value,
        ) -> RpcBoxFuture<'a, ()> {
            Box::pin(async move {
                match channel {
                    "ordered.seq" => {
                        let seq = data["seq"].as_u64().unwrap();
                        // Earlier even notifications sleep longer than odd ones; without FIFO
                        // serialization on multi-threaded Tokio they would invert order.
                        if seq.is_multiple_of(2) {
                            tokio::time::sleep(Duration::from_millis(2)).await;
                        }
                        self.seen.lock().push(seq);
                        if data["emitReply"].as_bool() == Some(true) {
                            self.rpc.notify("ordered.ack", json!({ "seq": seq }));
                        }
                    }
                    "ordered.reenter" => {
                        let peer = self
                            .rpc
                            .request("peer.ping", json!({ "ok": true }), Duration::from_secs(2))
                            .await
                            .unwrap();
                        self.rpc.notify("ordered.reenterAck", peer);
                    }
                    _ => {}
                }
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_incoming_notifications_preserve_fifo_order_and_eof_drain() {
        let rpc = Arc::new(RpcBridge::new());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(OrderedNotificationHandler {
            rpc: Arc::clone(&rpc),
            seen: Arc::clone(&seen),
        });
        let (rust_read, mut js_write) = duplex(65536);
        let (mut js_read, rust_write) = duplex(65536);

        let mut proc = NodeExtHostProcess::from_io_with_handler(
            Arc::clone(&rpc),
            handler,
            rust_read,
            rust_write,
            RpcWireFormat::ContentLengthJson,
        );

        // 1. Reentrant request inside handle_notification must not deadlock reader_handle.
        write_rpc_message(
            &mut js_write,
            &RpcMessage::Notification {
                channel: "ordered.reenter".to_string(),
                data: json!({}),
            },
            RpcWireFormat::ContentLengthJson,
        )
        .await
        .unwrap();

        let mut reader = BufReader::new(&mut js_read);
        let mut fbuf = Vec::new();
        let mut lbuf = String::new();
        let sub_req = tokio::time::timeout(
            Duration::from_secs(2),
            read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, RpcWireFormat::ContentLengthJson),
        )
        .await
        .expect("timed out waiting for reentrant request from notification handler")
        .unwrap()
        .unwrap();
        let sub_id = match sub_req {
            RpcMessage::Request { id, method, .. } => {
                assert_eq!(method, "peer.ping");
                id
            }
            other => panic!("Expected peer.ping request, got {:?}", other),
        };
        write_rpc_message(
            &mut js_write,
            &RpcMessage::Response {
                id: sub_id,
                result: Some(json!({ "pong": 99 })),
                error: None,
            },
            RpcWireFormat::ContentLengthJson,
        )
        .await
        .unwrap();
        let ack_notif = tokio::time::timeout(
            Duration::from_secs(2),
            read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, RpcWireFormat::ContentLengthJson),
        )
        .await
        .expect("timed out waiting for reentrant ack notification")
        .unwrap()
        .unwrap();
        assert_eq!(
            ack_notif,
            RpcMessage::Notification {
                channel: "ordered.reenterAck".to_string(),
                data: json!({ "pong": 99 }),
            }
        );

        // 2. Blast 40 sequential notifications and immediately half-close js_write.
        let total = 40u64;
        for seq in 0..total {
            let msg = RpcMessage::Notification {
                channel: "ordered.seq".to_string(),
                data: json!({ "seq": seq, "emitReply": seq + 1 == total }),
            };
            write_rpc_message(&mut js_write, &msg, RpcWireFormat::ContentLengthJson)
                .await
                .unwrap();
        }
        drop(js_write);

        // Writer must drain the final outbound notification emitted by the last queued
        // incoming notification BEFORE closing on EOF.
        let mut drained = Vec::new();
        while let Some(msg) = tokio::time::timeout(
            Duration::from_secs(3),
            read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, RpcWireFormat::ContentLengthJson),
        )
        .await
        .expect("timed out waiting for EOF drain")
        .unwrap()
        {
            drained.push(msg);
        }

        assert_eq!(
            drained,
            vec![RpcMessage::Notification {
                channel: "ordered.ack".to_string(),
                data: json!({ "seq": total - 1 }),
            }]
        );
        let recorded = seen.lock().clone();
        let expected: Vec<u64> = (0..total).collect();
        assert_eq!(recorded, expected, "notifications executed out of FIFO order");

        proc.shutdown().await.unwrap();
    }

    struct CausalBarrierTestHandler {
        rpc: Arc<RpcBridge>,
        state: Arc<Mutex<u64>>,
        concurrent_peer_barrier: Arc<tokio::sync::Barrier>,
    }

    impl RpcRequestHandler for CausalBarrierTestHandler {
        fn handle_request<'a>(
            &'a self,
            method: &'a str,
            params: &'a Value,
        ) -> RpcBoxFuture<'a, Result<Value, RpcError>> {
            Box::pin(async move {
                match method {
                    "state.read" => {
                        let val = *self.state.lock();
                        Ok(json!({ "value": val, "seq": params["seq"] }))
                    }
                    "req.waitPeer" => {
                        // Succeeds only if "req.releasePeer" executes concurrently on in_flight.
                        tokio::time::timeout(
                            Duration::from_millis(500),
                            self.concurrent_peer_barrier.wait(),
                        )
                        .await
                        .map_err(|_| RpcError::internal("concurrent request stalled"))?;
                        Ok(json!({ "concurrent": true }))
                    }
                    "req.releasePeer" => {
                        tokio::time::timeout(
                            Duration::from_millis(500),
                            self.concurrent_peer_barrier.wait(),
                        )
                        .await
                        .map_err(|_| RpcError::internal("concurrent peer wait stalled"))?;
                        Ok(json!({ "released": true }))
                    }
                    _ => Err(RpcError::method_not_found(method)),
                }
            })
        }

        fn handle_notification<'a>(
            &'a self,
            channel: &'a str,
            data: &'a Value,
        ) -> RpcBoxFuture<'a, ()> {
            Box::pin(async move {
                match channel {
                    "state.setDelayed" => {
                        let next_val = data["value"].as_u64().unwrap();
                        tokio::time::sleep(Duration::from_millis(4)).await;
                        *self.state.lock() = next_val;
                    }
                    "state.reentrantSet" => {
                        let next_val = data["value"].as_u64().unwrap();
                        let peer = self
                            .rpc
                            .request("peer.fetch", json!({ "v": next_val }), Duration::from_secs(2))
                            .await
                            .unwrap();
                        *self.state.lock() = peer["v"].as_u64().unwrap();
                    }
                    _ => {}
                }
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_incoming_notification_completes_before_subsequent_request_on_multithread() {
        let rpc = Arc::new(RpcBridge::new());
        let state = Arc::new(Mutex::new(0u64));
        let handler = Arc::new(CausalBarrierTestHandler {
            rpc: Arc::clone(&rpc),
            state: Arc::clone(&state),
            concurrent_peer_barrier: Arc::new(tokio::sync::Barrier::new(2)),
        });
        let (rust_read, mut js_write) = duplex(65536);
        let (mut js_read, rust_write) = duplex(65536);

        let mut proc = NodeExtHostProcess::from_io_with_handler(
            Arc::clone(&rpc),
            handler,
            rust_read,
            rust_write,
            RpcWireFormat::ContentLengthJson,
        );

        let mut reader = BufReader::new(&mut js_read);
        let mut fbuf = Vec::new();
        let mut lbuf = String::new();

        // 1. 20 iterations of delayed Notification followed at 0ms by Request("state.read")
        for seq in 1..=20u64 {
            write_rpc_message(
                &mut js_write,
                &RpcMessage::Notification {
                    channel: "state.setDelayed".to_string(),
                    data: json!({ "value": seq }),
                },
                RpcWireFormat::ContentLengthJson,
            )
            .await
            .unwrap();
            write_rpc_message(
                &mut js_write,
                &RpcMessage::Request {
                    id: seq,
                    method: "state.read".to_string(),
                    params: json!({ "seq": seq }),
                },
                RpcWireFormat::ContentLengthJson,
            )
            .await
            .unwrap();

            let resp = tokio::time::timeout(
                Duration::from_secs(2),
                read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, RpcWireFormat::ContentLengthJson),
            )
            .await
            .expect("timed out waiting for state.read response")
            .unwrap()
            .unwrap();
            match resp {
                RpcMessage::Response { id, result, error } => {
                    assert_eq!(id, seq);
                    assert!(error.is_none());
                    let val = result.unwrap()["value"].as_u64().unwrap();
                    assert_eq!(
                        val, seq,
                        "Request {} overtook preceding Notification (saw state={} instead of {})",
                        seq, val, seq
                    );
                }
                other => panic!("Expected Response, got {:?}", other),
            }
        }

        // 2. Notification performing reentrant rpc.request followed at 0ms by Request("state.read")
        //    must NOT deadlock reader_handle and must still apply state before Request runs.
        write_rpc_message(
            &mut js_write,
            &RpcMessage::Notification {
                channel: "state.reentrantSet".to_string(),
                data: json!({ "value": 999 }),
            },
            RpcWireFormat::ContentLengthJson,
        )
        .await
        .unwrap();
        write_rpc_message(
            &mut js_write,
            &RpcMessage::Request {
                id: 999,
                method: "state.read".to_string(),
                params: json!({ "seq": 999 }),
            },
            RpcWireFormat::ContentLengthJson,
        )
        .await
        .unwrap();

        let sub_req = tokio::time::timeout(
            Duration::from_secs(2),
            read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, RpcWireFormat::ContentLengthJson),
        )
        .await
        .expect("timed out waiting for reentrant peer.fetch")
        .unwrap()
        .unwrap();
        let sub_id = match sub_req {
            RpcMessage::Request { id, method, .. } => {
                assert_eq!(method, "peer.fetch");
                id
            }
            other => panic!("Expected peer.fetch Request, got {:?}", other),
        };
        write_rpc_message(
            &mut js_write,
            &RpcMessage::Response {
                id: sub_id,
                result: Some(json!({ "v": 999 })),
                error: None,
            },
            RpcWireFormat::ContentLengthJson,
        )
        .await
        .unwrap();
        let reent_resp = tokio::time::timeout(
            Duration::from_secs(2),
            read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, RpcWireFormat::ContentLengthJson),
        )
        .await
        .expect("timed out waiting for state.read after reentrant notification")
        .unwrap()
        .unwrap();
        match reent_resp {
            RpcMessage::Response { id: 999, result, error } => {
                assert!(error.is_none());
                assert_eq!(result.unwrap()["value"], json!(999));
            }
            other => panic!("Expected Response 999, got {:?}", other),
        }

        // 3. Consecutive Requests still execute concurrently on in_flight.
        write_rpc_message(
            &mut js_write,
            &RpcMessage::Request {
                id: 2001,
                method: "req.waitPeer".to_string(),
                params: json!({}),
            },
            RpcWireFormat::ContentLengthJson,
        )
        .await
        .unwrap();
        write_rpc_message(
            &mut js_write,
            &RpcMessage::Request {
                id: 2002,
                method: "req.releasePeer".to_string(),
                params: json!({}),
            },
            RpcWireFormat::ContentLengthJson,
        )
        .await
        .unwrap();
        let mut got_ids = Vec::new();
        for _ in 0..2 {
            let msg = tokio::time::timeout(
                Duration::from_secs(2),
                read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, RpcWireFormat::ContentLengthJson),
            )
            .await
            .expect("concurrent requests deadlocked")
            .unwrap()
            .unwrap();
            match msg {
                RpcMessage::Response { id, error, .. } => {
                    assert!(error.is_none(), "concurrent request {} errored: {:?}", id, error);
                    got_ids.push(id);
                }
                other => panic!("Expected Response, got {:?}", other),
            }
        }
        got_ids.sort_unstable();
        assert_eq!(got_ids, vec![2001, 2002]);

        drop(js_write);
        proc.shutdown().await.unwrap();
    }

    struct EofFastFailNotifHandler {
        rpc: Arc<RpcBridge>,
        request_elapsed_ms: Arc<Mutex<Option<u128>>>,
        request_err: Arc<Mutex<Option<String>>>,
    }

    impl RpcRequestHandler for EofFastFailNotifHandler {
        fn handle_request<'a>(
            &'a self,
            method: &'a str,
            _params: &'a Value,
        ) -> RpcBoxFuture<'a, Result<Value, RpcError>> {
            Box::pin(async move { Err(RpcError::method_not_found(method)) })
        }

        fn handle_notification<'a>(
            &'a self,
            channel: &'a str,
            _data: &'a Value,
        ) -> RpcBoxFuture<'a, ()> {
            Box::pin(async move {
                if channel == "eof.triggerRequest" {
                    // Brief yield/sleep so reader_handle processes peer EOF first.
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    let start = std::time::Instant::now();
                    let err = self
                        .rpc
                        .request("peer.afterEof", json!({}), Duration::from_secs(5))
                        .await
                        .unwrap_err();
                    *self.request_elapsed_ms.lock() = Some(start.elapsed().as_millis());
                    *self.request_err.lock() = Some(err);
                    // Subsequent outbound notification during L4 drain must still flush cleanly.
                    self.rpc.notify("drain.afterFastFail", json!({ "ok": true }));
                }
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_half_close_eof_fast_fails_outbound_requests_without_blocking_drain() {
        let rpc = Arc::new(RpcBridge::new());
        let request_elapsed_ms = Arc::new(Mutex::new(None));
        let request_err = Arc::new(Mutex::new(None));
        let handler = Arc::new(EofFastFailNotifHandler {
            rpc: Arc::clone(&rpc),
            request_elapsed_ms: Arc::clone(&request_elapsed_ms),
            request_err: Arc::clone(&request_err),
        });
        let (rust_read, mut js_write) = duplex(8192);
        let (mut js_read, rust_write) = duplex(8192);

        let mut proc = NodeExtHostProcess::from_io_with_handler(
            Arc::clone(&rpc),
            handler,
            rust_read,
            rust_write,
            RpcWireFormat::ContentLengthJson,
        );

        write_rpc_message(
            &mut js_write,
            &RpcMessage::Notification {
                channel: "eof.triggerRequest".to_string(),
                data: json!({}),
            },
            RpcWireFormat::ContentLengthJson,
        )
        .await
        .unwrap();
        // Immediately half-close peer write side.
        drop(js_write);

        let start = std::time::Instant::now();
        let mut reader = BufReader::new(&mut js_read);
        let mut fbuf = Vec::new();
        let mut lbuf = String::new();
        let mut drained = Vec::new();
        while let Some(msg) = tokio::time::timeout(
            Duration::from_millis(500),
            read_rpc_message(&mut reader, &mut fbuf, &mut lbuf, RpcWireFormat::ContentLengthJson),
        )
        .await
        .expect("L4b EOF drain blocked instead of fast-failing outbound request")
        .unwrap()
        {
            drained.push(msg);
        }
        let total_elapsed = start.elapsed();

        assert!(
            total_elapsed < Duration::from_millis(500),
            "EOF drain took {:?} (expected < 500ms)",
            total_elapsed
        );
        let req_err = request_err.lock().clone().expect("expected request error");
        assert!(
            req_err.contains("transport closed"),
            "expected transport closed error, got: {}",
            req_err
        );
        let req_ms = request_elapsed_ms.lock().expect("expected elapsed measurement");
        assert!(
            req_ms < 500,
            "rpc.request after EOF took {}ms (expected fast-fail < 500ms)",
            req_ms
        );
        assert_eq!(
            drained,
            vec![RpcMessage::Notification {
                channel: "drain.afterFastFail".to_string(),
                data: json!({ "ok": true }),
            }]
        );

        proc.shutdown().await.unwrap();
        assert!(rpc.is_reader_closed());
    }
}
