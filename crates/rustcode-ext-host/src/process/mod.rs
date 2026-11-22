//! Process execution RPC handlers (`process.exec`, `process.spawn`, `process.spawnSync`).

use crate::rpc::RpcError;
use parking_lot::Mutex;
use rustcode_platform::{file_uri_to_path, is_file_uri, strip_windows_verbatim_prefix};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::Notify;

const DEFAULT_MAX_BUFFER_BYTES: usize = 16 * 1024 * 1024;
const STREAM_DRAIN_GRACE_MS: u64 = 100;

#[cfg(unix)]
mod unix_libc {
    extern "C" {
        pub(super) fn setsid() -> i32;
        pub(super) fn setpgid(pid: i32, pgid: i32) -> i32;
        pub(super) fn kill(pid: i32, sig: i32) -> i32;
    }
}

#[derive(Debug, Clone)]
enum ShellMode {
    None,
    DefaultShell,
    CustomShell(String),
}

#[derive(Debug, Clone)]
struct ProcessInvocation {
    command: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env_overrides: Vec<(String, Option<String>)>,
    timeout_ms: Option<u64>,
    max_buffer: usize,
    shell: ShellMode,
    stdin_input: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessRunOutput {
    pid: u32,
    exit_code: Option<i32>,
    signal: Option<String>,
    stdout: String,
    stderr: String,
    killed: bool,
    timed_out: bool,
}

/// Dispatch `process.{exec, spawn, spawnSync}` RPC calls.
pub async fn handle_process_rpc(
    method: &str,
    params: &Value,
    fs_root: Option<&Path>,
) -> Result<Value, RpcError> {
    match method {
        "process.exec" => handle_exec(params, fs_root).await,
        "process.spawn" => handle_spawn(params, fs_root).await,
        "process.spawnSync" => handle_spawn_sync(params, fs_root).await,
        _ => Err(RpcError::method_not_found(method)),
    }
}

async fn handle_exec(params: &Value, fs_root: Option<&Path>) -> Result<Value, RpcError> {
    let command = require_command(params)?;
    let cwd = resolve_cwd(params, fs_root)?;
    let env_overrides = parse_env_overrides(params)?;
    let timeout_ms = parse_timeout_ms(params)?;
    let max_buffer = parse_max_buffer(params)?;

    let inv = ProcessInvocation {
        command,
        args: Vec::new(),
        cwd,
        env_overrides,
        timeout_ms,
        max_buffer,
        shell: ShellMode::DefaultShell,
        stdin_input: None,
    };

    let out = execute_invocation(inv).await?;
    Ok(json!({
        "pid": out.pid,
        "exitCode": out.exit_code,
        "signal": out.signal,
        "stdout": out.stdout,
        "stderr": out.stderr,
        "killed": out.killed,
        "timedOut": out.timed_out,
    }))
}

async fn handle_spawn(params: &Value, fs_root: Option<&Path>) -> Result<Value, RpcError> {
    let inv = parse_spawn_invocation(params, fs_root)?;
    let out = execute_invocation(inv).await?;
    Ok(json!({
        "pid": out.pid,
        "exitCode": out.exit_code,
        "signal": out.signal,
        "stdout": out.stdout,
        "stderr": out.stderr,
        "killed": out.killed,
        "timedOut": out.timed_out,
    }))
}

async fn handle_spawn_sync(params: &Value, fs_root: Option<&Path>) -> Result<Value, RpcError> {
    let inv = parse_spawn_invocation(params, fs_root)?;
    let out = execute_invocation(inv).await?;
    Ok(json!({
        "pid": out.pid,
        "status": out.exit_code,
        "exitCode": out.exit_code,
        "signal": out.signal,
        "stdout": out.stdout,
        "stderr": out.stderr,
        "killed": out.killed,
        "timedOut": out.timed_out,
        "error": Value::Null,
    }))
}

fn parse_spawn_invocation(
    params: &Value,
    fs_root: Option<&Path>,
) -> Result<ProcessInvocation, RpcError> {
    let command = require_command(params)?;
    let args = parse_args(params)?;
    let cwd = resolve_cwd(params, fs_root)?;
    let env_overrides = parse_env_overrides(params)?;
    let timeout_ms = parse_timeout_ms(params)?;
    let max_buffer = parse_max_buffer(params)?;
    let shell = parse_shell_mode(params)?;
    let stdin_input = parse_stdin_input(params)?;

    Ok(ProcessInvocation {
        command,
        args,
        cwd,
        env_overrides,
        timeout_ms,
        max_buffer,
        shell,
        stdin_input,
    })
}

fn require_command(params: &Value) -> Result<String, RpcError> {
    let raw = params.get("command").and_then(|v| v.as_str());
    match raw {
        Some(s) if !s.trim().is_empty() && !s.contains('\0') => Ok(s.to_string()),
        _ => Err(RpcError::new(
            "EINVAL",
            "Missing or invalid non-empty string 'command' parameter",
        )),
    }
}

fn parse_args(params: &Value) -> Result<Vec<String>, RpcError> {
    match params.get("args") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                let s = match item {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => {
                        return Err(RpcError::new(
                            "EINVAL",
                            "Elements of 'args' must be strings",
                        ));
                    }
                };
                if s.contains('\0') {
                    return Err(RpcError::new("EINVAL", "'args' must not contain NUL bytes"));
                }
                out.push(s);
            }
            Ok(out)
        }
        Some(_) => Err(RpcError::new("EINVAL", "'args' must be an array")),
    }
}

fn resolve_cwd(params: &Value, fs_root: Option<&Path>) -> Result<Option<PathBuf>, RpcError> {
    match params.get("cwd") {
        None | Some(Value::Null) => Ok(fs_root.map(|r| strip_windows_verbatim_prefix(r.to_path_buf()))),
        Some(Value::String(raw)) => {
            if raw.trim().is_empty() || raw.contains('\0') {
                return Err(RpcError::new(
                    "EINVAL",
                    "Invalid empty or NUL 'cwd' parameter",
                ));
            }
            let parsed = if is_file_uri(raw) {
                file_uri_to_path(raw)
            } else {
                PathBuf::from(raw)
            };
            let cleaned = strip_windows_verbatim_prefix(parsed);
            let resolved = if cleaned.is_absolute() {
                cleaned
            } else if let Some(root) = fs_root {
                strip_windows_verbatim_prefix(root.join(&cleaned))
            } else {
                cleaned
            };

            let meta = std::fs::metadata(&resolved).map_err(map_process_io_error)?;
            if !meta.is_dir() {
                return Err(RpcError::new(
                    "ENOTDIR",
                    format!("ENOTDIR: not a directory, cwd '{}'", resolved.display()),
                ));
            }
            Ok(Some(resolved))
        }
        Some(_) => Err(RpcError::new("EINVAL", "'cwd' must be a string")),
    }
}

fn parse_env_overrides(params: &Value) -> Result<Vec<(String, Option<String>)>, RpcError> {
    match params.get("env") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) => {
            let mut overrides = Vec::with_capacity(map.len());
            for (k, v) in map {
                if k.is_empty() || k.contains('\0') || k.contains('=') {
                    return Err(RpcError::new(
                        "EINVAL",
                        format!("Invalid environment variable name '{}'", k),
                    ));
                }
                let val = match v {
                    Value::Null => None,
                    Value::String(s) => {
                        if s.contains('\0') {
                            return Err(RpcError::new(
                                "EINVAL",
                                format!("Environment variable '{}' contains NUL byte", k),
                            ));
                        }
                        Some(s.clone())
                    }
                    Value::Number(n) => Some(n.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                    _ => {
                        return Err(RpcError::new(
                            "EINVAL",
                            format!("Environment variable '{}' must be a string or null", k),
                        ));
                    }
                };
                overrides.push((k.clone(), val));
            }
            Ok(overrides)
        }
        Some(_) => Err(RpcError::new("EINVAL", "'env' must be an object")),
    }
}

fn parse_timeout_ms(params: &Value) -> Result<Option<u64>, RpcError> {
    match params.get("timeout") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_u64() {
            Some(0) => Ok(None),
            Some(ms) => Ok(Some(ms)),
            None => Err(RpcError::new(
                "EINVAL",
                "'timeout' must be a non-negative integer",
            )),
        },
        Some(_) => Err(RpcError::new(
            "EINVAL",
            "'timeout' must be a non-negative integer",
        )),
    }
}

fn parse_max_buffer(params: &Value) -> Result<usize, RpcError> {
    match params.get("maxBuffer") {
        None | Some(Value::Null) => Ok(DEFAULT_MAX_BUFFER_BYTES),
        Some(Value::Number(n)) => match n.as_u64() {
            Some(v) if v > 0 => Ok(usize::try_from(v).unwrap_or(usize::MAX)),
            _ => Err(RpcError::new(
                "EINVAL",
                "'maxBuffer' must be a positive integer",
            )),
        },
        Some(_) => Err(RpcError::new(
            "EINVAL",
            "'maxBuffer' must be a positive integer",
        )),
    }
}

fn parse_shell_mode(params: &Value) -> Result<ShellMode, RpcError> {
    match params.get("shell") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(ShellMode::None),
        Some(Value::Bool(true)) => Ok(ShellMode::DefaultShell),
        Some(Value::String(s)) => {
            if s.trim().is_empty() || s.contains('\0') {
                return Err(RpcError::new(
                    "EINVAL",
                    "Invalid empty or NUL 'shell' parameter",
                ));
            }
            Ok(ShellMode::CustomShell(s.clone()))
        }
        Some(_) => Err(RpcError::new(
            "EINVAL",
            "'shell' must be a boolean or string",
        )),
    }
}

fn parse_stdin_input(params: &Value) -> Result<Option<Vec<u8>>, RpcError> {
    match params.get("input") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_bytes().to_vec())),
        Some(Value::Array(arr)) => {
            let mut bytes = Vec::with_capacity(arr.len());
            for item in arr {
                let b = item
                    .as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| {
                        RpcError::new("EINVAL", "'input' array elements must be byte values")
                    })?;
                bytes.push(b);
            }
            Ok(Some(bytes))
        }
        Some(_) => Err(RpcError::new(
            "EINVAL",
            "'input' must be a string or byte array",
        )),
    }
}

fn join_shell_command(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        let mut parts = Vec::with_capacity(1 + args.len());
        parts.push(command.to_string());
        parts.extend(args.iter().cloned());
        parts.join(" ")
    }
}

fn build_shell_std_command(shell_prog: Option<&str>, cmd_line: &str) -> std::process::Command {
    match shell_prog {
        None => {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let mut cmd = std::process::Command::new("cmd.exe");
                cmd.arg("/D")
                    .arg("/S")
                    .arg("/C")
                    .raw_arg(format!("\"{}\"", cmd_line));
                cmd
            }
            #[cfg(not(windows))]
            {
                let mut cmd = std::process::Command::new("/bin/sh");
                cmd.arg("-c").arg(cmd_line);
                cmd
            }
        }
        Some(sh) => {
            let stem = Path::new(sh)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            let mut cmd = std::process::Command::new(sh);
            if stem == "cmd" {
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.arg("/D")
                        .arg("/S")
                        .arg("/C")
                        .raw_arg(format!("\"{}\"", cmd_line));
                }
                #[cfg(not(windows))]
                {
                    cmd.arg("/C").arg(cmd_line);
                }
            } else if stem == "powershell" || stem == "pwsh" {
                cmd.args(["-NoProfile", "-NonInteractive", "-Command", cmd_line]);
            } else if matches!(
                stem.as_str(),
                "sh" | "bash" | "zsh" | "dash" | "ash" | "ksh" | "fish"
            ) || !cfg!(windows)
            {
                cmd.args(["-c", cmd_line]);
            } else {
                cmd.args(["/C", cmd_line]);
            }
            cmd
        }
    }
}

fn build_tokio_command(inv: &ProcessInvocation) -> Command {
    let mut std_cmd = match &inv.shell {
        ShellMode::None => {
            let mut cmd = std::process::Command::new(&inv.command);
            cmd.args(&inv.args);
            cmd
        }
        ShellMode::DefaultShell => {
            let line = join_shell_command(&inv.command, &inv.args);
            build_shell_std_command(None, &line)
        }
        ShellMode::CustomShell(sh) => {
            let line = join_shell_command(&inv.command, &inv.args);
            build_shell_std_command(Some(sh), &line)
        }
    };

    if let Some(ref cwd) = inv.cwd {
        std_cmd.current_dir(cwd);
    }

    if !inv.env_overrides.is_empty() {
        // Apply case-insensitive overrides on Windows so e.g. replacing `path` updates `PATH`.
        #[cfg(windows)]
        let existing_keys: std::collections::HashMap<String, std::ffi::OsString> =
            std::env::vars_os()
                .map(|(k, _)| (k.to_string_lossy().to_ascii_uppercase(), k))
                .collect();

        for (k, v) in &inv.env_overrides {
            #[cfg(windows)]
            let target_key = existing_keys
                .get(&k.to_ascii_uppercase())
                .map(|os| os.as_os_str())
                .unwrap_or_else(|| std::ffi::OsStr::new(k));
            #[cfg(not(windows))]
            let target_key = std::ffi::OsStr::new(k);

            match v {
                Some(val) => {
                    std_cmd.env(target_key, val);
                }
                None => {
                    std_cmd.env_remove(target_key);
                }
            }
        }
    }

    std_cmd.stdout(Stdio::piped());
    std_cmd.stderr(Stdio::piped());
    if inv.stdin_input.is_some() {
        std_cmd.stdin(Stdio::piped());
    } else {
        std_cmd.stdin(Stdio::null());
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std_cmd.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `pre_exec` runs in the forked child after `fork()` and before `exec()`.
        // `setsid` and `setpgid` are async-signal-safe POSIX functions that isolate the child
        // and any descendants into their own session/process group (PGID == child PID).
        unsafe {
            std_cmd.pre_exec(|| {
                let isolated = unix_libc::setsid() != -1 || unix_libc::setpgid(0, 0) != -1;
                if !isolated {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let mut cmd = Command::from(std_cmd);
    cmd.kill_on_drop(true);
    cmd
}

async fn execute_invocation(inv: ProcessInvocation) -> Result<ProcessRunOutput, RpcError> {
    let max_buffer = inv.max_buffer;
    let timeout_ms = inv.timeout_ms;
    let stdin_data = inv.stdin_input.clone();

    let mut cmd = build_tokio_command(&inv);
    let mut child = cmd.spawn().map_err(map_process_io_error)?;
    let pid = child.id().unwrap_or(0);

    if let (Some(bytes), Some(mut stdin)) = (stdin_data, child.stdin.take()) {
        tokio::spawn(async move {
            let _ = stdin.write_all(&bytes).await;
            let _ = stdin.shutdown().await;
        });
    }

    let stdout_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let max_buf_exceeded = Arc::new(AtomicBool::new(false));
    let max_buf_notify = Arc::new(Notify::new());

    let stdout_task = child.stdout.take().map(|reader| {
        let acc = Arc::clone(&stdout_buf);
        let exceeded = Arc::clone(&max_buf_exceeded);
        let notify = Arc::clone(&max_buf_notify);
        tokio::spawn(drain_stream_capped(reader, acc, max_buffer, exceeded, notify))
    });

    let stderr_task = child.stderr.take().map(|reader| {
        let acc = Arc::clone(&stderr_buf);
        let exceeded = Arc::clone(&max_buf_exceeded);
        let notify = Arc::clone(&max_buf_notify);
        tokio::spawn(drain_stream_capped(reader, acc, max_buffer, exceeded, notify))
    });

    enum WaitOutcome {
        Exited(Result<std::process::ExitStatus, std::io::Error>),
        TimedOut,
        MaxBufferExceeded,
    }

    let outcome = if let Some(ms) = timeout_ms {
        tokio::select! {
            res = tokio::time::timeout(Duration::from_millis(ms), child.wait()) => {
                match res {
                    Ok(status_res) => WaitOutcome::Exited(status_res),
                    Err(_) => WaitOutcome::TimedOut,
                }
            }
            _ = max_buf_notify.notified() => WaitOutcome::MaxBufferExceeded,
        }
    } else {
        tokio::select! {
            status_res = child.wait() => WaitOutcome::Exited(status_res),
            _ = max_buf_notify.notified() => WaitOutcome::MaxBufferExceeded,
        }
    };

    match outcome {
        WaitOutcome::MaxBufferExceeded => {
            terminate_child_tree(&mut child).await;
            if let Some(t) = stdout_task {
                t.abort();
            }
            if let Some(t) = stderr_task {
                t.abort();
            }
            Err(RpcError::new(
                "ERR_CHILD_PROCESS_STDIO_MAXBUFFER",
                format!("maxBuffer length ({} bytes) exceeded", max_buffer),
            ))
        }
        WaitOutcome::TimedOut => {
            terminate_child_tree(&mut child).await;
            finish_stream_task(stdout_task, true).await;
            finish_stream_task(stderr_task, true).await;
            let stdout_str = String::from_utf8_lossy(&stdout_buf.lock()).into_owned();
            let stderr_str = String::from_utf8_lossy(&stderr_buf.lock()).into_owned();
            Ok(ProcessRunOutput {
                pid,
                exit_code: None,
                signal: Some("SIGTERM".to_string()),
                stdout: stdout_str,
                stderr: stderr_str,
                killed: true,
                timed_out: true,
            })
        }
        WaitOutcome::Exited(status_res) => {
            let status = status_res.map_err(map_process_io_error)?;
            finish_stream_task(stdout_task, false).await;
            finish_stream_task(stderr_task, false).await;

            if max_buf_exceeded.load(Ordering::SeqCst) {
                return Err(RpcError::new(
                    "ERR_CHILD_PROCESS_STDIO_MAXBUFFER",
                    format!("maxBuffer length ({} bytes) exceeded", max_buffer),
                ));
            }

            let (exit_code, signal) = extract_exit_status(status);
            let stdout_str = String::from_utf8_lossy(&stdout_buf.lock()).into_owned();
            let stderr_str = String::from_utf8_lossy(&stderr_buf.lock()).into_owned();
            let killed = exit_code.is_none() && signal.is_some();

            Ok(ProcessRunOutput {
                pid,
                exit_code,
                signal,
                stdout: stdout_str,
                stderr: stderr_str,
                killed,
                timed_out: false,
            })
        }
    }
}

async fn drain_stream_capped<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    acc: Arc<Mutex<Vec<u8>>>,
    max_bytes: usize,
    exceeded: Arc<AtomicBool>,
    notify: Arc<Notify>,
) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut guard = acc.lock();
                if guard.len().saturating_add(n) > max_bytes {
                    let remaining = max_bytes.saturating_sub(guard.len());
                    if remaining > 0 {
                        guard.extend_from_slice(&buf[..remaining]);
                    }
                    drop(guard);
                    exceeded.store(true, Ordering::SeqCst);
                    notify.notify_waiters();
                    break;
                }
                guard.extend_from_slice(&buf[..n]);
            }
        }
    }
}

async fn finish_stream_task(task: Option<tokio::task::JoinHandle<()>>, bounded: bool) {
    let Some(handle) = task else {
        return;
    };
    if bounded {
        let abort_handle = handle.abort_handle();
        if tokio::time::timeout(Duration::from_millis(STREAM_DRAIN_GRACE_MS), handle)
            .await
            .is_err()
        {
            abort_handle.abort();
        }
    } else {
        let _ = handle.await;
    }
}

async fn terminate_child_tree(child: &mut Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut tk = Command::new("taskkill");
        tk.args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        if let Ok(mut tk_child) = tk.spawn() {
            let _ = tokio::time::timeout(Duration::from_millis(1000), tk_child.wait()).await;
        }
    }
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        if let Ok(pgid) = i32::try_from(pid) {
            if pgid > 0 {
                const SIGTERM: i32 = 15;
                const SIGKILL: i32 = 9;
                // SAFETY: `pgid` is the positive PID of our un-reaped child isolated into its own
                // session/process group in `pre_exec`, so signaling `-pgid` targets only that tree.
                unsafe {
                    unix_libc::kill(-pgid, SIGTERM);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
                // SAFETY: `child` has not been waited on yet, so `pgid` has not been recycled;
                // escalate to `SIGKILL` to terminate any surviving descendants in the group.
                unsafe {
                    unix_libc::kill(-pgid, SIGKILL);
                }
            }
        }
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_millis(500), child.wait()).await;
}

fn extract_exit_status(status: std::process::ExitStatus) -> (Option<i32>, Option<String>) {
    let code = status.code();
    #[cfg(unix)]
    let signal = if code.is_none() {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map(unix_signal_name)
    } else {
        None
    };
    #[cfg(not(unix))]
    let signal = None;
    (code, signal)
}

#[cfg(unix)]
fn unix_signal_name(sig: i32) -> String {
    match sig {
        1 => "SIGHUP".to_string(),
        2 => "SIGINT".to_string(),
        3 => "SIGQUIT".to_string(),
        6 => "SIGABRT".to_string(),
        9 => "SIGKILL".to_string(),
        11 => "SIGSEGV".to_string(),
        13 => "SIGPIPE".to_string(),
        14 => "SIGALRM".to_string(),
        15 => "SIGTERM".to_string(),
        other => format!("SIG{}", other),
    }
}

/// Map `std::io::Error` from process spawn/wait/cwd validation into a structured `RpcError`.
pub fn map_process_io_error(err: std::io::Error) -> RpcError {
    #[cfg(windows)]
    if let Some(raw) = err.raw_os_error() {
        let win_code = match raw {
            2 | 3 | 15 => Some("ENOENT"),
            5 | 82 => Some("EACCES"),
            87 | 123 | 161 => Some("EINVAL"),
            267 => Some("ENOTDIR"),
            1314 => Some("EPERM"),
            _ => None,
        };
        if let Some(code) = win_code {
            return RpcError::new(code, err.to_string());
        }
    }

    let code = match err.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::InvalidInput => "EINVAL",
        std::io::ErrorKind::NotADirectory => "ENOTDIR",
        std::io::ErrorKind::IsADirectory => "EISDIR",
        std::io::ErrorKind::AlreadyExists => "EEXIST",
        _ => {
            #[cfg(unix)]
            {
                match err.raw_os_error() {
                    Some(2) => "ENOENT",
                    Some(13) => "EACCES",
                    Some(20) => "ENOTDIR",
                    Some(21) => "EISDIR",
                    Some(22) => "EINVAL",
                    Some(1) => "EPERM",
                    _ => "SPAWN_ERROR",
                }
            }
            #[cfg(not(unix))]
            {
                "SPAWN_ERROR"
            }
        }
    };
    RpcError::new(code, err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_bin_and_args() -> (&'static str, Vec<&'static str>) {
        #[cfg(windows)]
        {
            ("cmd.exe", vec!["/C"])
        }
        #[cfg(not(windows))]
        {
            ("/bin/sh", vec!["-c"])
        }
    }

    #[tokio::test]
    async fn test_process_exec_success_and_non_zero_exit() {
        let ok_res = handle_process_rpc(
            "process.exec",
            &json!({ "command": "echo rustcode_proc_ok" }),
            None,
        )
        .await
        .expect("exec should succeed");
        assert_eq!(ok_res["exitCode"], json!(0));
        assert_eq!(ok_res["killed"], json!(false));
        assert_eq!(ok_res["timedOut"], json!(false));
        assert!(ok_res["stdout"]
            .as_str()
            .is_some_and(|s| s.contains("rustcode_proc_ok")));

        let fail_cmd = if cfg!(windows) {
            "echo err_msg 1>&2 && exit 3"
        } else {
            "echo err_msg >&2; exit 3"
        };
        let fail_res = handle_process_rpc("process.exec", &json!({ "command": fail_cmd }), None)
            .await
            .expect("non-zero exit should succeed at RPC level");
        assert_eq!(fail_res["exitCode"], json!(3));
        assert_eq!(fail_res["killed"], json!(false));
        assert_eq!(fail_res["timedOut"], json!(false));
        assert!(fail_res["stderr"]
            .as_str()
            .is_some_and(|s| s.contains("err_msg")));
    }

    #[tokio::test]
    async fn test_process_exec_timeout_kills_child() {
        let slow_cmd = if cfg!(windows) {
            "ping -n 6 127.0.0.1 >nul"
        } else {
            "sleep 5"
        };
        let res = handle_process_rpc(
            "process.exec",
            &json!({ "command": slow_cmd, "timeout": 100 }),
            None,
        )
        .await
        .expect("timeout returns structured killed/timedOut payload");
        assert_eq!(res["killed"], json!(true));
        assert_eq!(res["timedOut"], json!(true));
        assert_eq!(res["exitCode"], Value::Null);
        assert_eq!(res["signal"], json!("SIGTERM"));
    }

    #[tokio::test]
    async fn test_process_exec_max_buffer_enforced() {
        let noisy_cmd = if cfg!(windows) {
            "for /L %i in (1,1,60) do @echo 12345678901234567890"
        } else {
            "i=0; while [ $i -lt 60 ]; do echo 12345678901234567890; i=$((i+1)); done"
        };
        let err = handle_process_rpc(
            "process.exec",
            &json!({ "command": noisy_cmd, "maxBuffer": 128 }),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "ERR_CHILD_PROCESS_STDIO_MAXBUFFER");
    }

    #[tokio::test]
    async fn test_process_spawn_and_spawn_sync_with_env_and_stdin() {
        let (bin, base_args) = shell_bin_and_args();
        let mut env_args = base_args.clone();
        if cfg!(windows) {
            env_args.push("echo %RUSTCODE_TEST_VAR%");
        } else {
            env_args.push("printf '%s' \"$RUSTCODE_TEST_VAR\"");
        }

        let spawn_res = handle_process_rpc(
            "process.spawn",
            &json!({
                "command": bin,
                "args": env_args,
                "env": { "RUSTCODE_TEST_VAR": "hello-env-99" }
            }),
            None,
        )
        .await
        .expect("spawn should succeed");
        assert_eq!(spawn_res["exitCode"], json!(0));
        assert!(spawn_res["stdout"]
            .as_str()
            .is_some_and(|s| s.trim() == "hello-env-99"));

        let mut stdin_args = base_args;
        if cfg!(windows) {
            stdin_args.push("findstr .*");
        } else {
            stdin_args.push("cat");
        }
        let sync_res = handle_process_rpc(
            "process.spawnSync",
            &json!({
                "command": bin,
                "args": stdin_args,
                "input": "sync-stdin-payload"
            }),
            None,
        )
        .await
        .expect("spawnSync should succeed");
        assert_eq!(sync_res["status"], json!(0));
        assert_eq!(sync_res["exitCode"], json!(0));
        assert_eq!(sync_res["error"], Value::Null);
        assert!(sync_res["stdout"]
            .as_str()
            .is_some_and(|s| s.contains("sync-stdin-payload")));
    }

    #[tokio::test]
    async fn test_process_spawn_validation_missing_exec_and_invalid_cwd() {
        for method in ["process.exec", "process.spawn", "process.spawnSync"] {
            for bad in [json!({}), json!({ "command": "" }), json!({ "command": "   " }), json!({ "command": 42 })] {
                let err = handle_process_rpc(method, &bad, None).await.unwrap_err();
                assert_eq!(err.code, "EINVAL", "method={} payload={}", method, bad);
            }
        }

        let missing_bin = handle_process_rpc(
            "process.spawn",
            &json!({ "command": "rustcode_definitely_missing_binary_98765", "args": [] }),
            None,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(missing_bin.code.as_str(), "ENOENT" | "EACCES" | "SPAWN_ERROR"),
            "unexpected missing binary error: {:?}",
            missing_bin
        );

        let tmp = std::env::temp_dir().join(format!("rustcode_proc_cwd_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let file_path = tmp.join("not_a_dir.txt");
        std::fs::write(&file_path, "hi").unwrap();

        let missing_cwd = handle_process_rpc(
            "process.exec",
            &json!({ "command": "echo ok", "cwd": tmp.join("does_not_exist") }),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(missing_cwd.code, "ENOENT");

        let file_cwd = handle_process_rpc(
            "process.exec",
            &json!({ "command": "echo ok", "cwd": file_path }),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(file_cwd.code, "ENOTDIR");

        // Omitted cwd defaults to fs_root when provided
        let default_cwd_res = handle_process_rpc(
            "process.exec",
            &json!({ "command": if cfg!(windows) { "cd" } else { "pwd" } }),
            Some(&tmp),
        )
        .await
        .expect("default cwd from fs_root should succeed");
        assert_eq!(default_cwd_res["exitCode"], json!(0));
        assert!(default_cwd_res["stdout"]
            .as_str()
            .is_some_and(|s| s.contains(&format!("rustcode_proc_cwd_{}", std::process::id()))));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_process_spawn_with_shell_true() {
        let res = handle_process_rpc(
            "process.spawn",
            &json!({
                "command": "echo",
                "args": ["shell-spawn-ok"],
                "shell": true
            }),
            None,
        )
        .await
        .expect("spawn with shell=true should succeed");
        assert_eq!(res["exitCode"], json!(0));
        assert!(res["stdout"]
            .as_str()
            .is_some_and(|s| s.contains("shell-spawn-ok")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_process_group_isolation_and_kill() {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let inv = ProcessInvocation {
            command: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                "sleep 30 & child=$!; echo $child; wait".to_string(),
            ],
            cwd: None,
            env_overrides: Vec::new(),
            timeout_ms: None,
            max_buffer: DEFAULT_MAX_BUFFER_BYTES,
            shell: ShellMode::None,
            stdin_input: None,
        };

        let mut cmd = build_tokio_command(&inv);
        let mut child = cmd.spawn().expect("spawn should succeed on unix");
        let stdout = child.stdout.take().expect("stdout should be piped");
        let mut line = String::new();
        let mut reader = BufReader::new(stdout);
        tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
            .await
            .expect("timed out reading grandchild pid")
            .expect("failed reading grandchild pid");
        let grandchild_pid: i32 = line.trim().parse().expect("grandchild pid should be numeric");
        assert!(grandchild_pid > 0);

        terminate_child_tree(&mut child).await;

        let mut reaped = false;
        for _ in 0..40 {
            // SAFETY: `kill(pid, 0)` checks process existence without sending a signal.
            let alive = unsafe { unix_libc::kill(grandchild_pid, 0) == 0 };
            if !alive {
                reaped = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            reaped,
            "grandchild process {} should be terminated with the process group",
            grandchild_pid
        );
    }
}
