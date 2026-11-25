//! Multi-window orchestration (`WindowManager`) and cross-platform single-instance IPC.

use crate::cli::{CliOptions, GotoTarget};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

static TOKEN_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequest {
    pub paths: Vec<PathBuf>,
    pub goto: Vec<GotoTarget>,
    pub new_window: bool,
    pub reuse_window: bool,
}

impl OpenRequest {
    pub fn from_cli(opts: &CliOptions) -> Self {
        Self {
            paths: opts.paths.clone(),
            goto: opts.goto.clone(),
            new_window: opts.new_window,
            reuse_window: opts.reuse_window,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.goto.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowSummary {
    pub id: u64,
    pub workspace_roots: Vec<PathBuf>,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowRouteDecision {
    OpenInExisting {
        window_id: u64,
        request: OpenRequest,
    },
    OpenNewWindow(OpenRequest),
}

/// Manages logical workbench window identities, workspace roots, focus, and request routing.
#[derive(Debug, Default)]
pub struct WindowManager {
    windows: Vec<WindowSummary>,
    next_id: u64,
    active_window_id: Option<u64>,
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            next_id: 1,
            active_window_id: None,
        }
    }

    pub fn register_window(&mut self, workspace_roots: Vec<PathBuf>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        for w in &mut self.windows {
            w.focused = false;
        }
        self.windows.push(WindowSummary {
            id,
            workspace_roots,
            focused: true,
        });
        self.active_window_id = Some(id);
        id
    }

    pub fn unregister_window(&mut self, window_id: u64) -> bool {
        let before = self.windows.len();
        self.windows.retain(|w| w.id != window_id);
        if self.active_window_id == Some(window_id) {
            self.active_window_id = self.windows.last().map(|w| w.id);
            if let Some(new_active) = self.active_window_id {
                if let Some(w) = self.windows.iter_mut().find(|w| w.id == new_active) {
                    w.focused = true;
                }
            }
        }
        self.windows.len() != before
    }

    pub fn set_focused(&mut self, window_id: u64) {
        if self.windows.iter().any(|w| w.id == window_id) {
            for w in &mut self.windows {
                w.focused = w.id == window_id;
            }
            self.active_window_id = Some(window_id);
        }
    }

    pub fn update_workspace_roots(&mut self, window_id: u64, roots: Vec<PathBuf>) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == window_id) {
            w.workspace_roots = roots;
        }
    }

    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn active_window_id(&self) -> Option<u64> {
        self.active_window_id.or_else(|| self.windows.last().map(|w| w.id))
    }

    pub fn windows(&self) -> &[WindowSummary] {
        &self.windows
    }

    pub fn route_request(&self, request: OpenRequest) -> WindowRouteDecision {
        if request.new_window || self.windows.is_empty() {
            return WindowRouteDecision::OpenNewWindow(request);
        }

        let fallback_id = self
            .active_window_id()
            .unwrap_or_else(|| self.windows[0].id);

        if request.reuse_window || request.is_empty() {
            return WindowRouteDecision::OpenInExisting {
                window_id: fallback_id,
                request,
            };
        }

        // Match any path whose ancestor/equality matches an already-open workspace root.
        let mut best_match: Option<(usize, u64)> = None;
        for w in &self.windows {
            for root in &w.workspace_roots {
                for p in request
                    .paths
                    .iter()
                    .chain(request.goto.iter().map(|g| &g.path))
                {
                    if rustcode_platform::paths_equivalent(p, root)
                        || rustcode_platform::path_starts_with(p, root)
                    {
                        let depth = root.components().count();
                        if best_match.is_none_or(|(best_d, _)| depth > best_d) {
                            best_match = Some((depth, w.id));
                        }
                    }
                }
            }
        }
        if let Some((_, window_id)) = best_match {
            return WindowRouteDecision::OpenInExisting { window_id, request };
        }

        // Opening an external directory without `--reuse-window` opens a new workspace window.
        let opens_folder = request.paths.iter().any(|p| p.is_dir());
        if opens_folder {
            WindowRouteDecision::OpenNewWindow(request)
        } else {
            WindowRouteDecision::OpenInExisting {
                window_id: fallback_id,
                request,
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IpcEnvelope {
    token: String,
    shutdown: bool,
    request: Option<OpenRequest>,
}

/// Active primary-instance IPC server handle; cleaned up automatically on `Drop`.
pub struct SingleInstanceIpcServer {
    rx: mpsc::Receiver<OpenRequest>,
    shutdown: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,
    cleanup_path: PathBuf,
    endpoint: String,
    wake_addr: String,
    token: String,
}

impl SingleInstanceIpcServer {
    pub fn try_recv(&self) -> Option<OpenRequest> {
        self.rx.try_recv().ok()
    }

    pub fn drain_requests(&self) -> Vec<OpenRequest> {
        let mut out = Vec::new();
        while let Ok(req) = self.rx.try_recv() {
            out.push(req);
        }
        out
    }

    pub fn endpoint_label(&self) -> &str {
        &self.endpoint
    }
}

impl Drop for SingleInstanceIpcServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        platform_ipc::wake_listener(&self.wake_addr, &self.token);
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.cleanup_path);
    }
}

pub enum SingleInstanceBind {
    Leader(SingleInstanceIpcServer),
    Forwarded,
}

pub struct SingleInstanceIpc;

impl SingleInstanceIpc {
    /// Attempt to bind the primary-instance IPC endpoint under `data_dir`.
    /// If another healthy instance is already listening, forwards `request` to it
    /// and returns `SingleInstanceBind::Forwarded`.
    pub fn try_bind_or_forward(
        data_dir: &Path,
        request: &OpenRequest,
        on_wake: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> std::io::Result<SingleInstanceBind> {
        std::fs::create_dir_all(data_dir)?;
        platform_ipc::try_bind_or_forward_impl(data_dir, request, on_wake)
    }
}

fn generate_ipc_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let seq = TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("rc-{}-{}-{}", std::process::id(), nanos, seq)
}

#[cfg(windows)]
mod platform_ipc {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::time::Duration;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct LockFileData {
        port: u16,
        token: String,
        pid: u32,
    }

    fn lock_path(data_dir: &Path) -> PathBuf {
        data_dir.join("rustcode-ipc.lock")
    }

    fn send_to_existing(addr: SocketAddr, token: &str, request: &OpenRequest) -> std::io::Result<()> {
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(400))?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let env = IpcEnvelope {
            token: token.to_string(),
            shutdown: false,
            request: Some(request.clone()),
        };
        let payload = serde_json::to_string(&env)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(stream, "{payload}")?;
        stream.flush()?;
        let mut ack = String::new();
        let mut reader = BufReader::new(stream);
        reader.read_line(&mut ack)?;
        if ack.trim() == "OK" {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "IPC peer rejected or failed ack",
            ))
        }
    }

    pub(super) fn wake_listener(wake_addr: &str, token: &str) {
        if let Ok(mut stream) = TcpStream::connect(wake_addr) {
            let env = IpcEnvelope {
                token: token.to_string(),
                shutdown: true,
                request: None,
            };
            if let Ok(line) = serde_json::to_string(&env) {
                let _ = writeln!(stream, "{line}");
                let _ = stream.flush();
            }
        }
    }

    pub(super) fn try_bind_or_forward_impl(
        data_dir: &Path,
        request: &OpenRequest,
        on_wake: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> std::io::Result<SingleInstanceBind> {
        let lp = lock_path(data_dir);
        if let Ok(raw) = std::fs::read_to_string(&lp) {
            if let Ok(info) = serde_json::from_str::<LockFileData>(&raw) {
                let addr = SocketAddr::from(([127, 0, 0, 1], info.port));
                if send_to_existing(addr, &info.token, request).is_ok() {
                    return Ok(SingleInstanceBind::Forwarded);
                }
            }
            // Stale lockfile from an unclean shutdown; clean up and rebind
            let _ = std::fs::remove_file(&lp);
        }

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let local_addr = listener.local_addr()?;
        let token = generate_ipc_token();
        let lock_data = LockFileData {
            port: local_addr.port(),
            token: token.clone(),
            pid: std::process::id(),
        };
        let lock_json = serde_json::to_string(&lock_data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&lp, lock_json)?;

        let (tx, rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_bg = Arc::clone(&shutdown);
        let expected_token = token.clone();
        let join_handle = std::thread::Builder::new()
            .name("rustcode-ipc".to_string())
            .spawn(move || {
                for stream in listener.incoming() {
                    if shutdown_bg.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(mut stream) = stream else {
                        continue;
                    };
                    let mut line = String::new();
                    let read_ok = {
                        let mut reader = BufReader::new(&stream);
                        reader.read_line(&mut line).is_ok()
                    };
                    if !read_ok {
                        continue;
                    }
                    if let Ok(env) = serde_json::from_str::<IpcEnvelope>(line.trim()) {
                        if env.token != expected_token {
                            continue;
                        }
                        if env.shutdown || shutdown_bg.load(Ordering::SeqCst) {
                            let _ = writeln!(stream, "OK");
                            break;
                        }
                        if let Some(req) = env.request {
                            let _ = tx.send(req);
                            if let Some(cb) = &on_wake {
                                cb();
                            }
                            let _ = writeln!(stream, "OK");
                            let _ = stream.flush();
                        }
                    }
                }
            })?;

        Ok(SingleInstanceBind::Leader(SingleInstanceIpcServer {
            rx,
            shutdown,
            join_handle: Some(join_handle),
            cleanup_path: lp,
            endpoint: local_addr.to_string(),
            wake_addr: local_addr.to_string(),
            token,
        }))
    }
}

#[cfg(unix)]
mod platform_ipc {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};

    fn socket_path(data_dir: &Path) -> PathBuf {
        data_dir.join("rustcode-ipc.sock")
    }

    fn send_to_existing(sock: &Path, token: &str, request: &OpenRequest) -> std::io::Result<()> {
        let mut stream = UnixStream::connect(sock)?;
        let env = IpcEnvelope {
            token: token.to_string(),
            shutdown: false,
            request: Some(request.clone()),
        };
        let payload = serde_json::to_string(&env)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(stream, "{payload}")?;
        stream.flush()?;
        let mut ack = String::new();
        let mut reader = BufReader::new(stream);
        reader.read_line(&mut ack)?;
        if ack.trim() == "OK" {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "IPC peer rejected or failed ack",
            ))
        }
    }

    pub(super) fn wake_listener(wake_addr: &str, token: &str) {
        if let Ok(mut stream) = UnixStream::connect(wake_addr) {
            let env = IpcEnvelope {
                token: token.to_string(),
                shutdown: true,
                request: None,
            };
            if let Ok(line) = serde_json::to_string(&env) {
                let _ = writeln!(stream, "{line}");
                let _ = stream.flush();
            }
        }
    }

    pub(super) fn try_bind_or_forward_impl(
        data_dir: &Path,
        request: &OpenRequest,
        on_wake: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> std::io::Result<SingleInstanceBind> {
        let sp = socket_path(data_dir);
        if sp.exists() {
            if send_to_existing(&sp, "", request).is_ok() {
                return Ok(SingleInstanceBind::Forwarded);
            }
            let _ = std::fs::remove_file(&sp);
        }

        let listener = UnixListener::bind(&sp)?;
        let token = generate_ipc_token();
        let (tx, rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_bg = Arc::clone(&shutdown);
        let expected_token = token.clone();
        let join_handle = std::thread::Builder::new()
            .name("rustcode-ipc".to_string())
            .spawn(move || {
                for stream in listener.incoming() {
                    if shutdown_bg.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(mut stream) = stream else {
                        continue;
                    };
                    let mut line = String::new();
                    let read_ok = {
                        let mut reader = BufReader::new(&stream);
                        reader.read_line(&mut line).is_ok()
                    };
                    if !read_ok {
                        continue;
                    }
                    if let Ok(env) = serde_json::from_str::<IpcEnvelope>(line.trim()) {
                        if env.shutdown && env.token == expected_token {
                            let _ = writeln!(stream, "OK");
                            break;
                        }
                        if !env.shutdown {
                            if let Some(req) = env.request {
                                let _ = tx.send(req);
                                if let Some(cb) = &on_wake {
                                    cb();
                                }
                                let _ = writeln!(stream, "OK");
                                let _ = stream.flush();
                            }
                        }
                    }
                }
            })?;

        let sp_str = sp.to_string_lossy().into_owned();
        Ok(SingleInstanceBind::Leader(SingleInstanceIpcServer {
            rx,
            shutdown,
            join_handle: Some(join_handle),
            cleanup_path: sp,
            endpoint: sp_str.clone(),
            wake_addr: sp_str,
            token,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn test_window_manager_routing_new_reuse_and_workspace_match() {
        let mut mgr = WindowManager::new();
        let ws_a = PathBuf::from("/tmp/proj_a");
        let ws_b = PathBuf::from("/tmp/proj_b");
        let id1 = mgr.register_window(vec![ws_a.clone()]);
        assert_eq!(mgr.window_count(), 1);

        // File inside ws_a routes to id1
        let dec = mgr.route_request(OpenRequest {
            paths: vec![ws_a.join("src/main.rs")],
            goto: Vec::new(),
            new_window: false,
            reuse_window: false,
        });
        assert!(matches!(
            dec,
            WindowRouteDecision::OpenInExisting { window_id, .. } if window_id == id1
        ));

        // --new-window always opens a new window
        let dec_new = mgr.route_request(OpenRequest {
            paths: vec![ws_a.join("src/lib.rs")],
            goto: Vec::new(),
            new_window: true,
            reuse_window: false,
        });
        assert!(matches!(dec_new, WindowRouteDecision::OpenNewWindow(_)));

        // Second window with ws_b
        let id2 = mgr.register_window(vec![ws_b.clone()]);
        assert_eq!(mgr.window_count(), 2);
        let dec_a = mgr.route_request(OpenRequest {
            paths: vec![ws_a.join("Cargo.toml")],
            goto: Vec::new(),
            new_window: false,
            reuse_window: false,
        });
        assert!(matches!(
            dec_a,
            WindowRouteDecision::OpenInExisting { window_id, .. } if window_id == id1
        ));

        // --reuse-window targets active window (id2)
        let dec_reuse = mgr.route_request(OpenRequest {
            paths: vec![PathBuf::from("/elsewhere/file.rs")],
            goto: Vec::new(),
            new_window: false,
            reuse_window: true,
        });
        assert!(matches!(
            dec_reuse,
            WindowRouteDecision::OpenInExisting { window_id, .. } if window_id == id2
        ));

        assert!(mgr.unregister_window(id2));
        assert_eq!(mgr.window_count(), 1);
        assert_eq!(mgr.active_window_id(), Some(id1));
    }

    #[test]
    fn test_single_instance_ipc_leader_forward_and_stale_recovery() {
        let dir = std::env::temp_dir().join(format!(
            "rustcode_ipc_test_{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_cb = Arc::clone(&wakes);
        let initial = OpenRequest::from_cli(&CliOptions::default());
        let leader = match SingleInstanceIpc::try_bind_or_forward(
            &dir,
            &initial,
            Some(Arc::new(move || {
                wakes_cb.fetch_add(1, Ordering::SeqCst);
            })),
        )
        .unwrap()
        {
            SingleInstanceBind::Leader(srv) => srv,
            SingleInstanceBind::Forwarded => panic!("expected first instance to bind leader"),
        };

        // Second bind forwards request to leader synchronously
        let req2 = OpenRequest {
            paths: vec![PathBuf::from("/repo/src/lib.rs")],
            goto: vec![GotoTarget {
                path: PathBuf::from("/repo/src/main.rs"),
                line: 12,
                column: 4,
            }],
            new_window: false,
            reuse_window: true,
        };
        let second = SingleInstanceIpc::try_bind_or_forward(&dir, &req2, None).unwrap();
        assert!(matches!(second, SingleInstanceBind::Forwarded));

        let received = leader.drain_requests();
        assert_eq!(received, vec![req2]);
        assert_eq!(wakes.load(Ordering::SeqCst), 1);

        // Drop cleans up IPC lock/socket file; subsequent bind becomes leader again
        drop(leader);
        #[cfg(windows)]
        {
            // Also verify stale lock recovery if corrupt lockfile exists
            std::fs::write(dir.join("rustcode-ipc.lock"), r#"{"port":1,"token":"dead","pid":999999}"#)
                .unwrap();
        }
        let rebound = SingleInstanceIpc::try_bind_or_forward(&dir, &initial, None).unwrap();
        assert!(matches!(rebound, SingleInstanceBind::Leader(_)));
        drop(rebound);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
