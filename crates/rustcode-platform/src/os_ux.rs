//! Native OS UX abstractions: `AppPaths` (`dirs`), `ClipboardBackend` (`arboard` + in-memory),
//! and `FileDialogBackend` (`#[cfg(feature = "desktop")] rfd` + deterministic mock).

use crate::fs::strip_windows_verbatim_prefix;
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub user_data_dir: PathBuf,
    pub settings_file: PathBuf,
    pub keybindings_file: PathBuf,
    pub extensions_dir: PathBuf,
    pub state_db_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePaths {
    pub root: PathBuf,
    pub vscode_dir: PathBuf,
    pub settings_file: PathBuf,
    pub extensions_dir: PathBuf,
    pub launch_file: PathBuf,
}

impl AppPaths {
    /// Resolve platform user-data/config paths using `dirs` with optional CLI overrides
    /// (`--user-data-dir`, `--extensions-dir`).
    pub fn resolve(
        user_data_dir_override: Option<&Path>,
        extensions_dir_override: Option<&Path>,
    ) -> Self {
        let user_data_dir = match user_data_dir_override {
            Some(dir) if !dir.as_os_str().is_empty() => {
                strip_windows_verbatim_prefix(dir.to_path_buf())
            }
            _ => Self::default_user_data_dir(),
        };

        let extensions_dir = match extensions_dir_override {
            Some(dir) if !dir.as_os_str().is_empty() => {
                strip_windows_verbatim_prefix(dir.to_path_buf())
            }
            _ => user_data_dir.join("extensions"),
        };

        Self {
            settings_file: user_data_dir.join("settings.json"),
            keybindings_file: user_data_dir.join("keybindings.json"),
            state_db_file: user_data_dir.join("state.db"),
            user_data_dir,
            extensions_dir,
        }
    }

    pub fn for_user_data_dir(dir: impl AsRef<Path>) -> Self {
        Self::resolve(Some(dir.as_ref()), None)
    }

    pub fn with_extensions_dir(mut self, dir: impl AsRef<Path>) -> Self {
        if !dir.as_ref().as_os_str().is_empty() {
            self.extensions_dir = strip_windows_verbatim_prefix(dir.as_ref().to_path_buf());
        }
        self
    }

    pub fn default_user_data_dir() -> PathBuf {
        let base = dirs::config_dir()
            .map(|dir| dir.join("rustcode"))
            .or_else(|| dirs::home_dir().map(|home| home.join(".rustcode")))
            .unwrap_or_else(|| PathBuf::from(".rustcode"));
        strip_windows_verbatim_prefix(base)
    }

    pub fn workspace_paths(workspace_root: impl AsRef<Path>) -> WorkspacePaths {
        let root = strip_windows_verbatim_prefix(workspace_root.as_ref().to_path_buf());
        let vscode_dir = root.join(".vscode");
        WorkspacePaths {
            settings_file: vscode_dir.join("settings.json"),
            extensions_dir: vscode_dir.join("extensions"),
            launch_file: vscode_dir.join("launch.json"),
            vscode_dir,
            root,
        }
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.user_data_dir.join("profiles")
    }

    pub fn secrets_dir(&self) -> PathBuf {
        self.user_data_dir.join("secrets")
    }

    pub fn ensure_user_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.user_data_dir)?;
        std::fs::create_dir_all(&self.extensions_dir)?;
        Ok(())
    }
}

impl WorkspacePaths {
    pub fn extensions_recommendations_file(&self) -> PathBuf {
        self.vscode_dir.join("extensions.json")
    }
}

impl Default for AppPaths {
    fn default() -> Self {
        Self::resolve(None, None)
    }
}

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

    pub fn with_initial(text: impl Into<String>) -> Self {
        Self {
            text: Arc::new(RwLock::new(text.into())),
        }
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

#[derive(Default)]
pub struct ArboardClipboardBackend {
    clipboard: Mutex<Option<arboard::Clipboard>>,
}

impl std::fmt::Debug for ArboardClipboardBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArboardClipboardBackend").finish()
    }
}

impl ArboardClipboardBackend {
    pub fn new() -> Self {
        Self {
            clipboard: Mutex::new(None),
        }
    }

    fn with_clipboard<T, F>(&self, op: F) -> Result<T, String>
    where
        F: FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
    {
        let mut guard = self.clipboard.lock();
        if guard.is_none() {
            let created = arboard::Clipboard::new()
                .map_err(|err| format!("OS clipboard unavailable: {}", err))?;
            *guard = Some(created);
        }

        let clipboard = guard.as_mut().expect("clipboard initialized above");
        match op(clipboard) {
            Ok(val) => Ok(val),
            Err(err) => {
                // Drop cached handle so next attempt can recover if display/clipboard reset.
                *guard = None;
                Err(format!("OS clipboard error: {}", err))
            }
        }
    }
}

impl ClipboardBackend for ArboardClipboardBackend {
    fn read_text(&self) -> Result<String, String> {
        self.with_clipboard(|cb| cb.get_text())
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        self.with_clipboard(|cb| cb.set_text(text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileDialogOptions {
    pub title: Option<String>,
    pub default_directory: Option<PathBuf>,
    pub default_file_name: Option<String>,
    pub filters: Vec<(String, Vec<String>)>,
}

pub trait FileDialogBackend: Send + Sync {
    fn pick_open_file(&self, options: &FileDialogOptions) -> Option<PathBuf>;
    fn pick_open_folder(&self, options: &FileDialogOptions) -> Option<PathBuf>;
    fn pick_save_file(&self, options: &FileDialogOptions) -> Option<PathBuf>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialogRequestKind {
    OpenFile,
    OpenFolder,
    SaveFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedDialogRequest {
    pub kind: FileDialogRequestKind,
    pub options: FileDialogOptions,
}

pub type FileDialogCallback = Arc<dyn Fn(&FileDialogOptions) -> Option<PathBuf> + Send + Sync>;

#[derive(Default)]
struct MockFileDialogState {
    open_file_queue: VecDeque<Option<PathBuf>>,
    open_folder_queue: VecDeque<Option<PathBuf>>,
    save_file_queue: VecDeque<Option<PathBuf>>,
    open_file_callback: Option<FileDialogCallback>,
    open_folder_callback: Option<FileDialogCallback>,
    save_file_callback: Option<FileDialogCallback>,
    recorded: Vec<RecordedDialogRequest>,
}

#[derive(Clone, Default)]
pub struct MockFileDialogBackend {
    state: Arc<Mutex<MockFileDialogState>>,
}

impl std::fmt::Debug for MockFileDialogBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockFileDialogBackend").finish()
    }
}

impl MockFileDialogBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_open_file(&self, result: Option<PathBuf>) {
        self.state.lock().open_file_queue.push_back(result);
    }

    pub fn push_open_folder(&self, result: Option<PathBuf>) {
        self.state.lock().open_folder_queue.push_back(result);
    }

    pub fn push_save_file(&self, result: Option<PathBuf>) {
        self.state.lock().save_file_queue.push_back(result);
    }

    pub fn set_open_file_callback<F>(&self, cb: F)
    where
        F: Fn(&FileDialogOptions) -> Option<PathBuf> + Send + Sync + 'static,
    {
        self.state.lock().open_file_callback = Some(Arc::new(cb));
    }

    pub fn set_open_folder_callback<F>(&self, cb: F)
    where
        F: Fn(&FileDialogOptions) -> Option<PathBuf> + Send + Sync + 'static,
    {
        self.state.lock().open_folder_callback = Some(Arc::new(cb));
    }

    pub fn set_save_file_callback<F>(&self, cb: F)
    where
        F: Fn(&FileDialogOptions) -> Option<PathBuf> + Send + Sync + 'static,
    {
        self.state.lock().save_file_callback = Some(Arc::new(cb));
    }

    pub fn recorded_requests(&self) -> Vec<RecordedDialogRequest> {
        self.state.lock().recorded.clone()
    }

    pub fn clear_recorded_requests(&self) {
        self.state.lock().recorded.clear();
    }
}

impl FileDialogBackend for MockFileDialogBackend {
    fn pick_open_file(&self, options: &FileDialogOptions) -> Option<PathBuf> {
        let cb = {
            let mut guard = self.state.lock();
            guard.recorded.push(RecordedDialogRequest {
                kind: FileDialogRequestKind::OpenFile,
                options: options.clone(),
            });
            if let Some(queued) = guard.open_file_queue.pop_front() {
                return queued;
            }
            guard.open_file_callback.clone()
        };
        cb.and_then(|f| f(options))
    }

    fn pick_open_folder(&self, options: &FileDialogOptions) -> Option<PathBuf> {
        let cb = {
            let mut guard = self.state.lock();
            guard.recorded.push(RecordedDialogRequest {
                kind: FileDialogRequestKind::OpenFolder,
                options: options.clone(),
            });
            if let Some(queued) = guard.open_folder_queue.pop_front() {
                return queued;
            }
            guard.open_folder_callback.clone()
        };
        cb.and_then(|f| f(options))
    }

    fn pick_save_file(&self, options: &FileDialogOptions) -> Option<PathBuf> {
        let cb = {
            let mut guard = self.state.lock();
            guard.recorded.push(RecordedDialogRequest {
                kind: FileDialogRequestKind::SaveFile,
                options: options.clone(),
            });
            if let Some(queued) = guard.save_file_queue.pop_front() {
                return queued;
            }
            guard.save_file_callback.clone()
        };
        cb.and_then(|f| f(options))
    }
}

#[cfg(feature = "desktop")]
#[derive(Debug, Default, Clone, Copy)]
pub struct RfdFileDialogBackend;

#[cfg(feature = "desktop")]
impl RfdFileDialogBackend {
    pub fn new() -> Self {
        Self
    }

    fn build_dialog(options: &FileDialogOptions) -> rfd::FileDialog {
        let mut dialog = rfd::FileDialog::new();
        if let Some(title) = &options.title {
            dialog = dialog.set_title(title);
        }
        if let Some(dir) = &options.default_directory {
            dialog = dialog.set_directory(dir);
        }
        if let Some(file_name) = &options.default_file_name {
            dialog = dialog.set_file_name(file_name);
        }
        for (name, exts) in &options.filters {
            let ext_refs: Vec<&str> = exts.iter().map(String::as_str).collect();
            dialog = dialog.add_filter(name, &ext_refs);
        }
        dialog
    }
}

#[cfg(feature = "desktop")]
impl FileDialogBackend for RfdFileDialogBackend {
    fn pick_open_file(&self, options: &FileDialogOptions) -> Option<PathBuf> {
        Self::build_dialog(options).pick_file()
    }

    fn pick_open_folder(&self, options: &FileDialogOptions) -> Option<PathBuf> {
        Self::build_dialog(options).pick_folder()
    }

    fn pick_save_file(&self, options: &FileDialogOptions) -> Option<PathBuf> {
        Self::build_dialog(options).save_file()
    }
}

/// Abstract secret storage backend for VS Code `ExtensionContext.secrets`.
pub trait SecretStorageBackend: Send + Sync {
    fn get_secret(&self, account: &str) -> Result<Option<String>, String>;
    fn set_secret(&self, account: &str, value: &str) -> Result<(), String>;
    fn delete_secret(&self, account: &str) -> Result<(), String>;
}

/// Deterministic in-memory secret storage backend for headless execution and tests.
#[derive(Debug, Clone, Default)]
pub struct InMemorySecretStorageBackend {
    entries: Arc<RwLock<std::collections::BTreeMap<String, String>>>,
}

impl InMemorySecretStorageBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list_accounts(&self) -> Vec<String> {
        self.entries.read().keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.read().is_empty()
    }
}

impl SecretStorageBackend for InMemorySecretStorageBackend {
    fn get_secret(&self, account: &str) -> Result<Option<String>, String> {
        validate_secret_account(account)?;
        Ok(self.entries.read().get(account).cloned())
    }

    fn set_secret(&self, account: &str, value: &str) -> Result<(), String> {
        validate_secret_account(account)?;
        self.entries
            .write()
            .insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn delete_secret(&self, account: &str) -> Result<(), String> {
        validate_secret_account(account)?;
        self.entries.write().remove(account);
        Ok(())
    }
}

fn validate_secret_account(account: &str) -> Result<(), String> {
    if account.trim().is_empty() {
        return Err("Secret account identifier cannot be empty".to_string());
    }
    Ok(())
}

fn encode_secret_filename(account: &str) -> String {
    let mut out = String::with_capacity(account.len() * 2 + 4);
    for b in account.as_bytes() {
        out.push_str(&format!("{:02x}", b));
    }
    out.push_str(".sec");
    out
}

/// Desktop secret storage backend backed by OS user secret directories (with Windows DPAPI
/// when enabled via `use_native_os_keyring`, falling back deterministically to mode-0600 files
/// inside `AppPaths::secrets_dir()` when a headless environment lacks a keyring daemon).
#[derive(Debug, Clone)]
pub struct OsKeyringSecretStorageBackend {
    service_name: String,
    secrets_dir: PathBuf,
    use_native_keyring: bool,
    cache: Arc<RwLock<std::collections::BTreeMap<String, String>>>,
}

impl OsKeyringSecretStorageBackend {
    pub fn new(service_name: impl Into<String>, secrets_dir: impl AsRef<Path>) -> Self {
        Self {
            service_name: service_name.into(),
            secrets_dir: strip_windows_verbatim_prefix(secrets_dir.as_ref().to_path_buf()),
            use_native_keyring: cfg!(feature = "desktop"),
            cache: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
        }
    }

    /// Create a strictly file-vault-backed secret store without shelling out to OS helpers.
    pub fn file_backed_only(service_name: impl Into<String>, secrets_dir: impl AsRef<Path>) -> Self {
        Self {
            service_name: service_name.into(),
            secrets_dir: strip_windows_verbatim_prefix(secrets_dir.as_ref().to_path_buf()),
            use_native_keyring: false,
            cache: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
        }
    }

    pub fn service_name(&self) -> &str {
        &self.service_name
    }

    pub fn secrets_dir(&self) -> &Path {
        &self.secrets_dir
    }

    fn secret_file_path(&self, account: &str) -> PathBuf {
        self.secrets_dir.join(encode_secret_filename(account))
    }

    #[cfg(target_os = "linux")]
    fn try_native_get(&self, account: &str) -> Option<String> {
        if !self.use_native_keyring {
            return None;
        }
        let out = std::process::Command::new("secret-tool")
            .args(["lookup", "service", &self.service_name, "account", account])
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).to_string())
        } else {
            None
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn try_native_get(&self, _account: &str) -> Option<String> {
        None
    }

    #[cfg(target_os = "linux")]
    fn try_native_set(&self, account: &str, value: &str) -> bool {
        use std::io::Write;
        if !self.use_native_keyring {
            return false;
        }
        let Ok(mut child) = std::process::Command::new("secret-tool")
            .args([
                "store",
                "--label",
                &format!("{}: {}", self.service_name, account),
                "service",
                &self.service_name,
                "account",
                account,
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return false;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(value.as_bytes());
        }
        child.wait().map(|s| s.success()).unwrap_or(false)
    }

    #[cfg(not(target_os = "linux"))]
    fn try_native_set(&self, _account: &str, _value: &str) -> bool {
        let _ = self.use_native_keyring;
        false
    }

    #[cfg(target_os = "linux")]
    fn try_native_delete(&self, account: &str) {
        if !self.use_native_keyring {
            return;
        }
        let _ = std::process::Command::new("secret-tool")
            .args(["clear", "service", &self.service_name, "account", account])
            .output();
    }

    #[cfg(not(target_os = "linux"))]
    fn try_native_delete(&self, _account: &str) {}
}

impl SecretStorageBackend for OsKeyringSecretStorageBackend {
    fn get_secret(&self, account: &str) -> Result<Option<String>, String> {
        validate_secret_account(account)?;
        if let Some(val) = self.cache.read().get(account) {
            return Ok(Some(val.clone()));
        }
        if let Some(val) = self.try_native_get(account) {
            self.cache.write().insert(account.to_string(), val.clone());
            return Ok(Some(val));
        }
        let path = self.secret_file_path(account);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                self.cache
                    .write()
                    .insert(account.to_string(), content.clone());
                Ok(Some(content))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(format!("Failed to read secret file '{}': {}", path.display(), err)),
        }
    }

    fn set_secret(&self, account: &str, value: &str) -> Result<(), String> {
        validate_secret_account(account)?;
        self.cache
            .write()
            .insert(account.to_string(), value.to_string());
        let _ = self.try_native_set(account, value);

        std::fs::create_dir_all(&self.secrets_dir).map_err(|e| e.to_string())?;
        let path = self.secret_file_path(account);
        std::fs::write(&path, value).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    fn delete_secret(&self, account: &str) -> Result<(), String> {
        validate_secret_account(account)?;
        self.cache.write().remove(account);
        self.try_native_delete(account);
        let path = self.secret_file_path(account);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_paths_resolution_and_overrides() {
        let defaults = AppPaths::resolve(None, None);
        assert!(defaults.settings_file.ends_with("settings.json"));
        assert!(defaults.keybindings_file.ends_with("keybindings.json"));
        assert!(defaults.extensions_dir.ends_with("extensions"));
        assert!(defaults.state_db_file.ends_with("state.db"));
        assert_eq!(
            defaults.settings_file.parent(),
            Some(defaults.user_data_dir.as_path())
        );

        let custom_data = PathBuf::from("/tmp/custom-rustcode-data");
        let custom_ext = PathBuf::from("/tmp/custom-rustcode-exts");
        let overridden = AppPaths::resolve(Some(&custom_data), Some(&custom_ext));
        assert_eq!(overridden.user_data_dir, custom_data);
        assert_eq!(overridden.settings_file, custom_data.join("settings.json"));
        assert_eq!(
            overridden.keybindings_file,
            custom_data.join("keybindings.json")
        );
        assert_eq!(overridden.state_db_file, custom_data.join("state.db"));
        assert_eq!(overridden.extensions_dir, custom_ext);

        let ws = AppPaths::workspace_paths(Path::new("/projects/demo"));
        assert_eq!(ws.vscode_dir, PathBuf::from("/projects/demo/.vscode"));
        assert_eq!(
            ws.settings_file,
            PathBuf::from("/projects/demo/.vscode/settings.json")
        );
        assert_eq!(
            ws.extensions_dir,
            PathBuf::from("/projects/demo/.vscode/extensions")
        );
        assert_eq!(
            ws.launch_file,
            PathBuf::from("/projects/demo/.vscode/launch.json")
        );
    }

    #[test]
    fn test_in_memory_clipboard_backend() {
        let cb = InMemoryClipboardBackend::new();
        assert_eq!(cb.read_text().unwrap(), "");
        cb.write_text("hello rustcode").unwrap();
        assert_eq!(cb.read_text().unwrap(), "hello rustcode");

        let seeded = InMemoryClipboardBackend::with_initial("seed");
        assert_eq!(seeded.read_text().unwrap(), "seed");
    }

    #[test]
    fn test_mock_file_dialog_backend_queue_and_callbacks() {
        let dialog = MockFileDialogBackend::new();
        dialog.push_open_file(Some(PathBuf::from("/ws/main.rs")));
        dialog.push_open_file(None);
        dialog.push_open_folder(Some(PathBuf::from("/ws")));
        dialog.set_save_file_callback(|opts| {
            opts.default_file_name
                .as_ref()
                .map(|name| PathBuf::from("/saved").join(name))
        });

        let open_opts = FileDialogOptions {
            title: Some("Open File".to_string()),
            ..Default::default()
        };
        assert_eq!(
            dialog.pick_open_file(&open_opts),
            Some(PathBuf::from("/ws/main.rs"))
        );
        assert_eq!(dialog.pick_open_file(&open_opts), None);
        assert_eq!(
            dialog.pick_open_folder(&FileDialogOptions::default()),
            Some(PathBuf::from("/ws"))
        );
        assert_eq!(
            dialog.pick_save_file(&FileDialogOptions {
                default_file_name: Some("notes.md".to_string()),
                ..Default::default()
            }),
            Some(PathBuf::from("/saved/notes.md"))
        );

        let reqs = dialog.recorded_requests();
        assert_eq!(reqs.len(), 4);
        assert_eq!(reqs[0].kind, FileDialogRequestKind::OpenFile);
        assert_eq!(reqs[2].kind, FileDialogRequestKind::OpenFolder);
        assert_eq!(reqs[3].kind, FileDialogRequestKind::SaveFile);
    }

    #[test]
    fn test_paths_and_secret_storage_backends() {
        let dir =
            std::env::temp_dir().join(format!("rustcode_secrets_{}", uuid::Uuid::new_v4()));
        let app_paths = AppPaths::for_user_data_dir(&dir);
        assert!(app_paths.profiles_dir().ends_with("profiles"));
        assert!(app_paths.secrets_dir().ends_with("secrets"));
        let ws = AppPaths::workspace_paths(Path::new("/demo"));
        assert!(ws.extensions_recommendations_file().ends_with("extensions.json"));

        let mem = InMemorySecretStorageBackend::new();
        assert!(mem.get_secret("ext:token").unwrap().is_none());
        mem.set_secret("ext:token", "secret-123").unwrap();
        assert_eq!(mem.get_secret("ext:token").unwrap().as_deref(), Some("secret-123"));
        assert_eq!(mem.len(), 1);
        mem.delete_secret("ext:token").unwrap();
        assert!(mem.get_secret("ext:token").unwrap().is_none());
        assert!(mem.set_secret("", "bad").is_err());

        let os_vault =
            OsKeyringSecretStorageBackend::file_backed_only("rustcode", app_paths.secrets_dir());
        os_vault
            .set_secret("github.copilot/token:user/../../../escape", "tok_xyz")
            .unwrap();
        let reloaded =
            OsKeyringSecretStorageBackend::file_backed_only("rustcode", app_paths.secrets_dir());
        assert_eq!(
            reloaded
                .get_secret("github.copilot/token:user/../../../escape")
                .unwrap()
                .as_deref(),
            Some("tok_xyz")
        );
        reloaded
            .delete_secret("github.copilot/token:user/../../../escape")
            .unwrap();
        assert!(reloaded
            .get_secret("github.copilot/token:user/../../../escape")
            .unwrap()
            .is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
