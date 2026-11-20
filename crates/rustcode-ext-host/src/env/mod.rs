//! Extension Host Environment & Extensions Registry (`vscode.env`, `vscode.extensions`).

use crate::manifest::{ExtensionLoader, ExtensionSnapshot};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};
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

pub trait ClipboardBackend: Send + Sync {
    fn read_text(&self) -> Result<String, String>;
    fn write_text(&self, text: &str) -> Result<(), String>;
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryClipboardBackend {
    text: Arc<RwLock<String>>,
}

impl InMemoryClipboardBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ClipboardBackend for InMemoryClipboardBackend {
    fn read_text(&self) -> Result<String, String> {
        Ok(self.text.read().clone())
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        *self.text.write() = text.to_string();
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct OsClipboardBackend {
    fallback: InMemoryClipboardBackend,
    fallback_on_unavailable: bool,
}

impl Default for OsClipboardBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl OsClipboardBackend {
    pub fn new() -> Self {
        Self {
            fallback: InMemoryClipboardBackend::new(),
            fallback_on_unavailable: true,
        }
    }

    pub fn with_fallback(fallback_on_unavailable: bool) -> Self {
        Self {
            fallback: InMemoryClipboardBackend::new(),
            fallback_on_unavailable,
        }
    }

    fn run_read(&self) -> Result<String, String> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let script = "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $v = Get-Clipboard -Raw; if ($null -ne $v) { [Console]::Out.Write($v) }";
            for exe in ["powershell.exe", "pwsh.exe"] {
                if let Ok(out) = Command::new(exe)
                    .args(["-NoProfile", "-NonInteractive", "-Command", script])
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .output()
                {
                    if out.status.success() {
                        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
                    }
                }
            }
            Err("Windows clipboard command unavailable".to_string())
        }

        #[cfg(target_os = "macos")]
        {
            let out = Command::new("pbpaste")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .map_err(|e| format!("pbpaste failed: {}", e))?;
            if out.status.success() {
                return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
            }
            Err("pbpaste returned non-zero status".to_string())
        }

        #[cfg(all(not(windows), not(target_os = "macos")))]
        {
            let has_wayland = std::env::var_os("WAYLAND_DISPLAY")
                .is_some_and(|v| !v.is_empty());
            let has_x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
            if !has_wayland && !has_x11 {
                return Err("No display server (WAYLAND_DISPLAY/DISPLAY) available".to_string());
            }
            if has_wayland {
                if let Ok(out) = Command::new("wl-paste")
                    .arg("--no-newline")
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .output()
                {
                    if out.status.success() {
                        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
                    }
                }
            }
            if has_x11 {
                for (cmd, args) in [
                    ("xclip", &["-selection", "clipboard", "-o"][..]),
                    ("xsel", &["--clipboard", "--output"][..]),
                ] {
                    if let Ok(out) = Command::new(cmd)
                        .args(args)
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .output()
                    {
                        if out.status.success() {
                            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
                        }
                    }
                }
            }
            Err("No Linux clipboard utility (wl-paste/xclip/xsel) available".to_string())
        }
    }

    fn run_write(&self, text: &str) -> Result<(), String> {
        #[cfg(windows)]
        {
            use std::io::Write;
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let script = "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; $t = [Console]::In.ReadToEnd(); if ($t.Length -eq 0) { Set-Clipboard -Value $null } else { Set-Clipboard -Value $t }";
            for exe in ["powershell.exe", "pwsh.exe"] {
                if let Ok(mut child) = Command::new(exe)
                    .args(["-NoProfile", "-NonInteractive", "-Command", script])
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    if let Ok(status) = child.wait() {
                        if status.success() {
                            return Ok(());
                        }
                    }
                }
            }
            Err("Windows clipboard write command unavailable".to_string())
        }

        #[cfg(target_os = "macos")]
        {
            use std::io::Write;
            let mut child = Command::new("pbcopy")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("pbcopy failed: {}", e))?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(text.as_bytes())
                    .map_err(|e| format!("pbcopy write failed: {}", e))?;
            }
            let status = child.wait().map_err(|e| format!("pbcopy wait failed: {}", e))?;
            if status.success() {
                Ok(())
            } else {
                Err("pbcopy returned non-zero status".to_string())
            }
        }

        #[cfg(all(not(windows), not(target_os = "macos")))]
        {
            use std::io::Write;
            let has_wayland = std::env::var_os("WAYLAND_DISPLAY")
                .is_some_and(|v| !v.is_empty());
            let has_x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
            if !has_wayland && !has_x11 {
                return Err("No display server (WAYLAND_DISPLAY/DISPLAY) available".to_string());
            }
            let candidates: Vec<(&str, &[&str])> = {
                let mut v = Vec::new();
                if has_wayland {
                    v.push(("wl-copy", &[][..]));
                }
                if has_x11 {
                    v.push(("xclip", &["-selection", "clipboard", "-in"][..]));
                    v.push(("xsel", &["--clipboard", "--input"][..]));
                }
                v
            };
            for (cmd, args) in candidates {
                if let Ok(mut child) = Command::new(cmd)
                    .args(args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    if let Ok(status) = child.wait() {
                        if status.success() {
                            return Ok(());
                        }
                    }
                }
            }
            Err("No Linux clipboard write utility (wl-copy/xclip/xsel) available".to_string())
        }
    }
}

impl ClipboardBackend for OsClipboardBackend {
    fn read_text(&self) -> Result<String, String> {
        match self.run_read() {
            Ok(text) => {
                let _ = self.fallback.write_text(&text);
                Ok(text)
            }
            Err(err) => {
                if self.fallback_on_unavailable {
                    self.fallback.read_text()
                } else {
                    Err(err)
                }
            }
        }
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        match self.run_write(text) {
            Ok(()) => {
                let _ = self.fallback.write_text(text);
                Ok(())
            }
            Err(err) => {
                if self.fallback_on_unavailable {
                    self.fallback.write_text(text)
                } else {
                    Err(err)
                }
            }
        }
    }
}

pub trait ExternalUriOpener: Send + Sync {
    fn open_uri(&self, uri: &str) -> Result<bool, String>;
}

#[derive(Debug, Clone, Default)]
pub struct RecordingExternalUriOpener {
    uris: Arc<RwLock<Vec<String>>>,
}

impl RecordingExternalUriOpener {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn opened_uris(&self) -> Vec<String> {
        self.uris.read().clone()
    }

    pub fn clear(&self) {
        self.uris.write().clear();
    }
}

impl ExternalUriOpener for RecordingExternalUriOpener {
    fn open_uri(&self, uri: &str) -> Result<bool, String> {
        self.uris.write().push(uri.to_string());
        Ok(true)
    }
}

#[derive(Debug, Clone)]
pub struct OsExternalUriOpener {
    enabled: bool,
    fallback_on_unavailable: bool,
    fallback_recorder: RecordingExternalUriOpener,
}

impl Default for OsExternalUriOpener {
    fn default() -> Self {
        Self::new(true)
    }
}

impl OsExternalUriOpener {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            fallback_on_unavailable: true,
            fallback_recorder: RecordingExternalUriOpener::new(),
        }
    }

    pub fn with_fallback(enabled: bool, fallback_on_unavailable: bool) -> Self {
        Self {
            enabled,
            fallback_on_unavailable,
            fallback_recorder: RecordingExternalUriOpener::new(),
        }
    }

    pub fn fallback_recorded_uris(&self) -> Vec<String> {
        self.fallback_recorder.opened_uris()
    }

    #[cfg(windows)]
    fn build_windows_open_command(uri: &str) -> Command {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = Command::new("powershell.exe");
        // Pass URI via environment variable so neither cmd.exe (%VAR%) nor PowerShell
        // (-Command string parsing of &, $, #, etc.) can expand or split the URI.
        cmd.env("RUSTCODE_OPEN_URI", uri)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                "Start-Process -FilePath $env:RUSTCODE_OPEN_URI",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd
    }

    fn spawn_os_open(&self, uri: &str) -> Result<(), String> {
        if uri.contains('"') || uri.contains('`') {
            return Err("URI contains unsafe quote or backtick characters".to_string());
        }

        #[cfg(windows)]
        {
            Self::build_windows_open_command(uri)
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to spawn powershell.exe Start-Process: {}", e))
        }

        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .arg(uri)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to spawn open: {}", e))
        }

        #[cfg(all(not(windows), not(target_os = "macos")))]
        {
            let has_wayland = std::env::var_os("WAYLAND_DISPLAY")
                .is_some_and(|v| !v.is_empty());
            let has_x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
            if !has_wayland && !has_x11 {
                return Err("No display server (WAYLAND_DISPLAY/DISPLAY) available".to_string());
            }
            Command::new("xdg-open")
                .arg(uri)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to spawn xdg-open: {}", e))
        }
    }
}

impl ExternalUriOpener for OsExternalUriOpener {
    fn open_uri(&self, uri: &str) -> Result<bool, String> {
        if !self.enabled {
            return self.fallback_recorder.open_uri(uri);
        }
        match self.spawn_os_open(uri) {
            Ok(()) => Ok(true),
            Err(err) => {
                if self.fallback_on_unavailable {
                    self.fallback_recorder.open_uri(uri)
                } else {
                    Err(err)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtEnvironmentInfo {
    pub app_name: String,
    pub app_root: String,
    pub app_host: String,
    pub language: String,
    pub machine_id: String,
    pub session_id: String,
    pub shell: String,
    pub uri_scheme: String,
    pub ui_kind: u8,
}

impl Default for ExtEnvironmentInfo {
    fn default() -> Self {
        let app_root = std::env::current_dir()
            .map(rustcode_platform::strip_windows_verbatim_prefix)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string());

        #[cfg(windows)]
        let shell = std::env::var("COMSPEC")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "cmd.exe".to_string());

        #[cfg(not(windows))]
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if Path::new("/bin/bash").exists() {
                    "/bin/bash".to_string()
                } else {
                    "/bin/sh".to_string()
                }
            });

        let pid = std::process::id();
        Self {
            app_name: "RustCode".to_string(),
            app_root,
            app_host: "desktop".to_string(),
            language: "en".to_string(),
            machine_id: format!("rustcode-machine-{}", pid),
            session_id: format!("rustcode-session-{}", pid),
            shell,
            uri_scheme: "rustcode".to_string(),
            ui_kind: 1,
        }
    }
}

#[derive(Clone)]
pub struct ExtHostEnvManager {
    clipboard_backend: Arc<RwLock<Arc<dyn ClipboardBackend>>>,
    external_uri_opener: Arc<RwLock<Arc<dyn ExternalUriOpener>>>,
    opened_external_uris: Arc<RwLock<Vec<String>>>,
    extensions: Arc<RwLock<Vec<ExtensionSnapshot>>>,
    extension_loader: Arc<RwLock<Option<Arc<RwLock<ExtensionLoader>>>>>,
    environment: Arc<RwLock<ExtEnvironmentInfo>>,
    pending_events: Arc<RwLock<Vec<ExtEnvEvent>>>,
    listeners: Arc<RwLock<Vec<EnvEventListener>>>,
    revision: Arc<AtomicU64>,
}

impl Default for ExtHostEnvManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtHostEnvManager {
    pub fn new() -> Self {
        Self {
            clipboard_backend: Arc::new(RwLock::new(Arc::new(InMemoryClipboardBackend::new()))),
            external_uri_opener: Arc::new(RwLock::new(Arc::new(
                RecordingExternalUriOpener::new(),
            ))),
            opened_external_uris: Arc::new(RwLock::new(Vec::new())),
            extensions: Arc::new(RwLock::new(Vec::new())),
            extension_loader: Arc::new(RwLock::new(None)),
            environment: Arc::new(RwLock::new(ExtEnvironmentInfo::default())),
            pending_events: Arc::new(RwLock::new(Vec::new())),
            listeners: Arc::new(RwLock::new(Vec::new())),
            revision: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_os_backends() -> Self {
        Self::new()
            .with_clipboard_backend(Arc::new(OsClipboardBackend::new()))
            .with_external_uri_opener(Arc::new(OsExternalUriOpener::new(true)))
    }

    pub fn with_clipboard_backend(self, backend: Arc<dyn ClipboardBackend>) -> Self {
        *self.clipboard_backend.write() = backend;
        self
    }

    pub fn set_clipboard_backend(&self, backend: Arc<dyn ClipboardBackend>) {
        *self.clipboard_backend.write() = backend;
    }

    pub fn with_external_uri_opener(self, opener: Arc<dyn ExternalUriOpener>) -> Self {
        *self.external_uri_opener.write() = opener;
        self
    }

    pub fn set_external_uri_opener(&self, opener: Arc<dyn ExternalUriOpener>) {
        *self.external_uri_opener.write() = opener;
    }

    pub fn with_environment(self, env: ExtEnvironmentInfo) -> Self {
        *self.environment.write() = env;
        self
    }

    pub fn set_environment(&self, env: ExtEnvironmentInfo) {
        *self.environment.write() = env;
    }

    pub fn environment(&self) -> ExtEnvironmentInfo {
        self.environment.read().clone()
    }

    pub fn with_app_root(self, root: impl AsRef<Path>) -> Self {
        self.set_app_root(root);
        self
    }

    pub fn set_app_root(&self, root: impl AsRef<Path>) {
        let cleaned = rustcode_platform::strip_windows_verbatim_prefix(root.as_ref().to_path_buf());
        self.environment.write().app_root = cleaned.to_string_lossy().into_owned();
    }

    pub fn with_shared_extension_loader(self, loader: Arc<RwLock<ExtensionLoader>>) -> Self {
        self.set_extension_loader(loader);
        self
    }

    pub fn set_extension_loader(&self, loader: Arc<RwLock<ExtensionLoader>>) {
        let snaps = loader.read().snapshots();
        *self.extension_loader.write() = Some(loader);
        self.set_extensions(snaps);
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

    pub fn try_clipboard_read(&self) -> Result<String, String> {
        let backend = self.clipboard_backend.read().clone();
        backend.read_text()
    }

    pub fn clipboard_read(&self) -> String {
        self.try_clipboard_read().unwrap_or_default()
    }

    pub fn try_clipboard_write(&self, value: impl Into<String>) -> Result<(), String> {
        let text = value.into();
        let backend = self.clipboard_backend.read().clone();
        backend.write_text(&text)?;
        self.emit(ExtEnvEvent::ClipboardWritten { text });
        Ok(())
    }

    pub fn clipboard_write(&self, value: impl Into<String>) {
        let _ = self.try_clipboard_write(value);
    }

    /// Validate and open an external URI via the configured `ExternalUriOpener`.
    pub fn open_external(&self, raw_uri: &str) -> bool {
        if raw_uri.is_empty()
            || raw_uri
                .chars()
                .any(|c| c.is_ascii_control() || c.is_whitespace())
            || raw_uri.contains('"')
            || raw_uri.contains('`')
        {
            return false;
        }
        if !is_valid_external_uri(raw_uri) {
            return false;
        }
        let opener = self.external_uri_opener.read().clone();
        match opener.open_uri(raw_uri) {
            Ok(true) => {
                let normalized = raw_uri.to_string();
                self.opened_external_uris.write().push(normalized.clone());
                self.emit(ExtEnvEvent::ExternalUriOpened { uri: normalized });
                true
            }
            _ => false,
        }
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
        if let Some(loader) = self.extension_loader.read().as_ref() {
            let snaps = loader.read().snapshots();
            *self.extensions.write() = snaps.clone();
            return snaps;
        }
        self.extensions.read().clone()
    }

    pub fn get_extension(&self, id: &str) -> Option<ExtensionSnapshot> {
        let target = id.trim();
        if target.is_empty() {
            return None;
        }
        if let Some(loader) = self.extension_loader.read().as_ref() {
            return loader.read().snapshot_for(target);
        }
        let guard = self.extensions.read();
        find_snapshot_by_id(&guard, target).cloned()
    }

    pub fn activate_extension(&self, id: &str) -> Option<ExtensionSnapshot> {
        let target = id.trim();
        if target.is_empty() {
            return None;
        }
        if let Some(loader_arc) = self.extension_loader.read().clone() {
            let (was_active, snap, snaps) = {
                let mut loader = loader_arc.write();
                let before = loader.snapshot_for(target).map(|s| s.is_active);
                if !loader.activate(target) {
                    return None;
                }
                let snap = loader.snapshot_for(target)?;
                let snaps = loader.snapshots();
                (before.unwrap_or(false), snap, snaps)
            };
            let count = snaps.len();
            *self.extensions.write() = snaps;
            if !was_active {
                self.emit(ExtEnvEvent::ExtensionsUpdated { count });
            }
            return Some(snap);
        }

        let (result, count, transitioned) = {
            let mut guard = self.extensions.write();
            let count = guard.len();
            let snap = find_snapshot_by_id_mut(&mut guard, target)?;
            let transitioned = !snap.is_active;
            snap.is_active = true;
            (snap.clone(), count, transitioned)
        };
        if transitioned {
            self.emit(ExtEnvEvent::ExtensionsUpdated { count });
        }
        Some(result)
    }

    pub fn deactivate_extension(&self, id: &str) -> Option<ExtensionSnapshot> {
        let target = id.trim();
        if target.is_empty() {
            return None;
        }
        if let Some(loader_arc) = self.extension_loader.read().clone() {
            let (was_active, snap, snaps) = {
                let mut loader = loader_arc.write();
                let before = loader.snapshot_for(target).map(|s| s.is_active);
                if !loader.deactivate(target) {
                    return None;
                }
                let snap = loader.snapshot_for(target)?;
                let snaps = loader.snapshots();
                (before.unwrap_or(false), snap, snaps)
            };
            let count = snaps.len();
            *self.extensions.write() = snaps;
            if was_active {
                self.emit(ExtEnvEvent::ExtensionsUpdated { count });
            }
            return Some(snap);
        }

        let (result, count, transitioned) = {
            let mut guard = self.extensions.write();
            let count = guard.len();
            let snap = find_snapshot_by_id_mut(&mut guard, target)?;
            let transitioned = snap.is_active;
            snap.is_active = false;
            (snap.clone(), count, transitioned)
        };
        if transitioned {
            self.emit(ExtEnvEvent::ExtensionsUpdated { count });
        }
        Some(result)
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "env.getEnvironment" => Ok(serde_json::to_value(self.environment()).unwrap_or_else(|_| json!({}))),
            "env.clipboardRead" => self
                .try_clipboard_read()
                .map(|v| json!(v))
                .map_err(|e| format!("CLIPBOARD_ERROR: {}", e)),
            "env.clipboardWrite" => {
                let value = params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'value' parameter".to_string())?;
                self.try_clipboard_write(value)
                    .map_err(|e| format!("CLIPBOARD_ERROR: {}", e))?;
                Ok(json!({ "written": true }))
            }
            "env.openExternal" => {
                let uri = params
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing string 'uri' parameter".to_string())?;
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
                let snap = self.activate_extension(id);
                let activated = snap.is_some();
                let resolved_id = snap
                    .as_ref()
                    .map(|s| s.id.clone())
                    .unwrap_or_else(|| id.to_string());
                Ok(json!({
                    "activated": activated,
                    "id": resolved_id,
                    "extension": snap,
                }))
            }
            _ => Err(format!("Unsupported env/extensions RPC method: {}", method)),
        }
    }
}

fn snapshot_matches_short_name(snap: &ExtensionSnapshot, target: &str) -> bool {
    snap.package_json
        .get("name")
        .and_then(|v| v.as_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(target))
}

fn find_snapshot_by_id<'a>(
    snapshots: &'a [ExtensionSnapshot],
    target: &str,
) -> Option<&'a ExtensionSnapshot> {
    snapshots
        .iter()
        .find(|snap| snap.id.eq_ignore_ascii_case(target))
        .or_else(|| {
            snapshots
                .iter()
                .find(|snap| snapshot_matches_short_name(snap, target))
        })
}

fn find_snapshot_by_id_mut<'a>(
    snapshots: &'a mut [ExtensionSnapshot],
    target: &str,
) -> Option<&'a mut ExtensionSnapshot> {
    if let Some(idx) = snapshots
        .iter()
        .position(|snap| snap.id.eq_ignore_ascii_case(target))
    {
        return snapshots.get_mut(idx);
    }
    if let Some(idx) = snapshots
        .iter()
        .position(|snap| snapshot_matches_short_name(snap, target))
    {
        return snapshots.get_mut(idx);
    }
    None
}

pub fn is_valid_external_uri(uri: &str) -> bool {
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

    struct FailingClipboardBackend;
    impl ClipboardBackend for FailingClipboardBackend {
        fn read_text(&self) -> Result<String, String> {
            Err("clipboard unavailable".to_string())
        }
        fn write_text(&self, _text: &str) -> Result<(), String> {
            Err("clipboard read-only".to_string())
        }
    }

    #[test]
    fn test_env_pluggable_backends_environment_and_shared_loader() {
        let recorder = Arc::new(RecordingExternalUriOpener::new());
        let mgr = ExtHostEnvManager::new()
            .with_external_uri_opener(recorder.clone())
            .with_app_root("/opt/rustcode-app");

        let env_val = mgr
            .handle_rpc_call("env.getEnvironment", &json!({}))
            .unwrap();
        assert_eq!(env_val["appName"], "RustCode");
        assert_eq!(env_val["appRoot"], "/opt/rustcode-app");
        assert_eq!(env_val["uriScheme"], "rustcode");
        assert_eq!(env_val["appHost"], "desktop");
        assert_eq!(env_val["uiKind"], 1);
        assert_eq!(env_val["language"], "en");
        assert!(!env_val["shell"].as_str().unwrap().is_empty());
        assert!(!env_val["machineId"].as_str().unwrap().is_empty());
        assert!(!env_val["sessionId"].as_str().unwrap().is_empty());

        // Quotes and backticks rejected even with valid scheme
        assert!(!mgr.open_external("https://example.com/\"quote\""));
        assert!(!mgr.open_external("https://example.com/`cmd`"));
        assert!(mgr.open_external("https://rustcode.dev/docs"));
        assert_eq!(
            recorder.opened_uris(),
            vec!["https://rustcode.dev/docs".to_string()]
        );

        // OsExternalUriOpener disabled mode falls back to recording safely
        let os_opener_disabled = OsExternalUriOpener::new(false);
        assert!(os_opener_disabled
            .open_uri("https://rustcode.dev/offline")
            .unwrap());
        assert_eq!(
            os_opener_disabled.fallback_recorded_uris(),
            vec!["https://rustcode.dev/offline".to_string()]
        );

        // Strict failing clipboard backend propagates CLIPBOARD_ERROR
        let strict_mgr = ExtHostEnvManager::new()
            .with_clipboard_backend(Arc::new(FailingClipboardBackend));
        assert!(strict_mgr
            .handle_rpc_call("env.clipboardRead", &json!({}))
            .unwrap_err()
            .starts_with("CLIPBOARD_ERROR:"));
        assert!(strict_mgr
            .handle_rpc_call("env.clipboardWrite", &json!({ "value": "x" }))
            .unwrap_err()
            .starts_with("CLIPBOARD_ERROR:"));

        // Shared ExtensionLoader synchronization + short-name lookup + ExtensionsUpdated event
        let dir = std::env::temp_dir().join(format!(
            "rustcode_env_shared_loader_{}",
            std::process::id()
        ));
        let ext_dir = dir.join("my-ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("package.json"),
            r#"{"name": "my-ext", "publisher": "RustCode", "version": "1.0.0"}"#,
        )
        .unwrap();

        let mut loader = ExtensionLoader::new();
        loader.load_from_dir(&ext_dir).unwrap();
        let shared = Arc::new(RwLock::new(loader));
        let loader_mgr = ExtHostEnvManager::new().with_shared_extension_loader(shared.clone());
        let _ = loader_mgr.take_pending_events();

        // Lookup & activate via short manifest name case-insensitively
        assert!(loader_mgr.get_extension("MY-EXT").is_some());
        let act = loader_mgr
            .handle_rpc_call("extensions.activate", &json!({ "id": "MY-EXT" }))
            .unwrap();
        assert_eq!(act["activated"], true);
        assert_eq!(act["id"], "RustCode.my-ext");
        assert!(shared.read().is_activated("rustcode.my-ext"));
        assert!(loader_mgr
            .take_pending_events()
            .iter()
            .any(|e| matches!(e, ExtEnvEvent::ExtensionsUpdated { count: 1 })));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_os_open_command_preserves_env_vars_and_special_chars() {
        let uri = "https://example.com/%TEMP%/foo?q=1&b=2#top%20bar";
        let cmd = OsExternalUriOpener::build_windows_open_command(uri);
        assert_eq!(cmd.get_program(), "powershell.exe");
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.iter().any(|a| *a == "-NoProfile"));
        assert!(args.iter().any(|a| *a == "-NonInteractive"));
        assert!(args
            .iter()
            .any(|a| *a == "Start-Process -FilePath $env:RUSTCODE_OPEN_URI"));
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(envs.iter().any(|(k, v)| {
            *k == "RUSTCODE_OPEN_URI" && v.as_deref() == Some(std::ffi::OsStr::new(uri))
        }));
    }
}
