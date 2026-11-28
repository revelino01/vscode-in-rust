//! Language feature bridge coordinating LSP clients and JavaScript extension providers.

use super::{ProviderRegistry, ProviderType};
use crate::rpc::{RpcBoxFuture, RpcBridge, RpcError, RpcRequestHandler};
use parking_lot::{Mutex, RwLock};
use rustcode_base::disposable::Disposable;
use rustcode_platform::{
    CommandRegistry, ConfigurationService, ConfigurationTarget, FileChangeEvent, FileChangeType,
    FileSystemError, FileSystemService,
};
use rustcode_protocols::lsp::{
    decode_semantic_tokens, CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall,
    CodeAction, CodeActionContext, CodeActionKind, CodeLens, CompletionContext,
    CompletionTriggerKind, DecodedSemanticToken, DocumentLink, FormattingOptions, Hover, InlayHint,
    LspClientManager, LspCommand, LspCompletionItem, LspDiagnostic, LspLocation, LspPosition,
    LspRange, LspTextEdit, MarkupContent, PrepareRenameResult, SemanticTokens,
    SemanticTokensLegend, SignatureHelp, TypeHierarchyItem, WorkspaceEdit,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub type ExtDiagnosticsListener = Arc<dyn Fn(&str, &[LspDiagnostic]) + Send + Sync>;

#[derive(Default)]
pub struct DiagnosticCollectionManager {
    collections: RwLock<HashMap<String, HashMap<String, Vec<LspDiagnostic>>>>,
    listeners: Arc<Mutex<Vec<ExtDiagnosticsListener>>>,
}

impl DiagnosticCollectionManager {
    pub fn new() -> Self {
        Self {
            collections: RwLock::new(HashMap::new()),
            listeners: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn on_diagnostics<F>(&self, listener: F)
    where
        F: Fn(&str, &[LspDiagnostic]) + Send + Sync + 'static,
    {
        self.listeners.lock().push(Arc::new(listener));
    }

    fn notify_listeners(&self, uri: &str) {
        let diags = self.get_diagnostics_for_uri(uri);
        let listeners = self.listeners.lock().clone();
        for listener in listeners {
            listener(uri, &diags);
        }
    }

    pub fn set_diagnostics(&self, collection: &str, uri: &str, diagnostics: Vec<LspDiagnostic>) {
        {
            let mut guard = self.collections.write();
            let map = guard.entry(collection.to_string()).or_default();
            if diagnostics.is_empty() {
                map.remove(uri);
            } else {
                map.insert(uri.to_string(), diagnostics);
            }
        }
        self.notify_listeners(uri);
    }

    pub fn clear_diagnostics(&self, collection: &str, uri: &str) {
        {
            let mut guard = self.collections.write();
            if let Some(map) = guard.get_mut(collection) {
                map.remove(uri);
            }
        }
        self.notify_listeners(uri);
    }

    pub fn clear_all_in_collection(&self, collection: &str) {
        let uris: Vec<String> = {
            let mut guard = self.collections.write();
            if let Some(map) = guard.remove(collection) {
                map.into_keys().collect()
            } else {
                Vec::new()
            }
        };
        for uri in uris {
            self.notify_listeners(&uri);
        }
    }

    pub fn get_diagnostics_for_uri(&self, uri: &str) -> Vec<LspDiagnostic> {
        let mut result = Vec::new();
        let guard = self.collections.read();
        for map in guard.values() {
            if let Some(diags) = map.get(uri) {
                result.extend(diags.clone());
            }
        }
        result
    }

    pub fn get_all_diagnostics(&self) -> HashMap<String, Vec<LspDiagnostic>> {
        let mut result = HashMap::new();
        let guard = self.collections.read();
        for map in guard.values() {
            for (uri, diags) in map {
                result
                    .entry(uri.clone())
                    .or_insert_with(Vec::new)
                    .extend(diags.clone());
            }
        }
        result
    }

    pub fn handle_rpc_message(&self, method: &str, params: &serde_json::Value) -> bool {
        self.handle_rpc_call(method, params).is_ok()
    }

    pub fn handle_rpc_call(
        &self,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<Value, RpcError> {
        match method {
            "languages.setDiagnostics" => {
                let name = params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'name' parameter"))?;
                let uri = params
                    .get("uri")
                    .and_then(|u| u.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'uri' parameter"))?;
                let diags_val = params
                    .get("diagnostics")
                    .ok_or_else(|| RpcError::invalid_params("Missing 'diagnostics' parameter"))?;
                let diags: Vec<LspDiagnostic> = serde_json::from_value(diags_val.clone())
                    .map_err(|e| RpcError::invalid_params(format!("Invalid diagnostics: {}", e)))?;
                self.set_diagnostics(name, uri, diags);
                Ok(json!({ "success": true }))
            }
            "languages.clearDiagnostics" => {
                let name = params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'name' parameter"))?;
                let uri = params
                    .get("uri")
                    .and_then(|u| u.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'uri' parameter"))?;
                self.clear_diagnostics(name, uri);
                Ok(json!({ "success": true }))
            }
            "languages.clearAllDiagnostics" => {
                let name = params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'name' parameter"))?;
                self.clear_all_in_collection(name);
                Ok(json!({ "success": true }))
            }
            _ => Err(RpcError::method_not_found(method)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TreeItemCollapsibleState {
    #[default]
    None = 0,
    Collapsed = 1,
    Expanded = 2,
}

impl Serialize for TreeItemCollapsibleState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for TreeItemCollapsibleState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = TreeItemCollapsibleState;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an integer (0, 1, 2) or string (\"None\", \"Collapsed\", \"Expanded\")")
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                match v {
                    0 => Ok(TreeItemCollapsibleState::None),
                    1 => Ok(TreeItemCollapsibleState::Collapsed),
                    2 => Ok(TreeItemCollapsibleState::Expanded),
                    _ => Err(E::custom(format!("invalid TreeItemCollapsibleState: {}", v))),
                }
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                match v {
                    0 => Ok(TreeItemCollapsibleState::None),
                    1 => Ok(TreeItemCollapsibleState::Collapsed),
                    2 => Ok(TreeItemCollapsibleState::Expanded),
                    _ => Err(E::custom(format!("invalid TreeItemCollapsibleState: {}", v))),
                }
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "None" | "none" => Ok(TreeItemCollapsibleState::None),
                    "Collapsed" | "collapsed" => Ok(TreeItemCollapsibleState::Collapsed),
                    "Expanded" | "expanded" => Ok(TreeItemCollapsibleState::Expanded),
                    _ => Err(E::custom(format!("invalid TreeItemCollapsibleState: {}", v))),
                }
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeItemCheckbox {
    /// 0 = Unchecked, 1 = Checked (matches `vscode.TreeItemCheckboxState`).
    pub state: u8,
    #[serde(default)]
    pub tooltip: Option<String>,
}

impl<'de> Deserialize<'de> for TreeItemCheckbox {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            StateOnly(u8),
            Full {
                state: u8,
                #[serde(default)]
                tooltip: Option<String>,
            },
        }
        match Wire::deserialize(deserializer)? {
            Wire::StateOnly(state) => Ok(Self {
                state,
                tooltip: None,
            }),
            Wire::Full { state, tooltip } => Ok(Self { state, tooltip }),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TreeItem {
    pub id: Option<String>,
    #[serde(default)]
    pub label: String,
    pub description: Option<String>,
    pub tooltip: Option<String>,
    #[serde(rename = "collapsibleState", default)]
    pub collapsible_state: TreeItemCollapsibleState,
    #[serde(rename = "iconPath")]
    pub icon_path: Option<String>,
    #[serde(rename = "resourceUri", default)]
    pub resource_uri: Option<String>,
    #[serde(rename = "contextValue", alias = "context_value")]
    pub context_value: Option<String>,
    pub command: Option<LspCommand>,
    #[serde(rename = "checkboxState", default)]
    pub checkbox_state: Option<TreeItemCheckbox>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtStatusBarItem {
    pub id: String,
    pub text: String,
    pub tooltip: Option<String>,
    pub command: Option<String>,
    pub alignment: i32, // 1 = Left, 2 = Right
    pub priority: i32,
    pub visible: bool,
}

#[derive(Default)]
pub struct ExtStatusBarManager {
    items: RwLock<HashMap<String, ExtStatusBarItem>>,
    revision: AtomicU64,
}

impl ExtStatusBarManager {
    pub fn new() -> Self {
        Self {
            items: RwLock::new(HashMap::new()),
            revision: AtomicU64::new(0),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn set_item(&self, item: ExtStatusBarItem) {
        self.items.write().insert(item.id.clone(), item);
        self.revision.fetch_add(1, Ordering::SeqCst);
    }

    pub fn hide_item(&self, id: &str) {
        if let Some(item) = self.items.write().get_mut(id) {
            item.visible = false;
            self.revision.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn remove_item(&self, id: &str) {
        if self.items.write().remove(id).is_some() {
            self.revision.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn get_visible_items(&self) -> Vec<ExtStatusBarItem> {
        let guard = self.items.read();
        let mut list: Vec<ExtStatusBarItem> = guard.values().filter(|i| i.visible).cloned().collect();
        list.sort_by_key(|i| std::cmp::Reverse(i.priority));
        list
    }

    pub fn get_item(&self, id: &str) -> Option<ExtStatusBarItem> {
        self.items.read().get(id).cloned()
    }

    pub fn handle_rpc_message(&self, method: &str, params: &serde_json::Value) -> bool {
        self.handle_rpc_call(method, params).is_ok()
    }

    pub fn handle_rpc_call(
        &self,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<Value, RpcError> {
        match method {
            "window.setStatusBarItem" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'id' parameter"))?;
                let text = params
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tooltip = params
                    .get("tooltip")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let command = params
                    .get("command")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let alignment = params
                    .get("alignment")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1) as i32;
                let priority = params
                    .get("priority")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;
                self.set_item(ExtStatusBarItem {
                    id: id.to_string(),
                    text,
                    tooltip,
                    command,
                    alignment,
                    priority,
                    visible: true,
                });
                Ok(json!({ "success": true }))
            }
            "window.hideStatusBarItem" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'id' parameter"))?;
                self.hide_item(id);
                Ok(json!({ "success": true }))
            }
            _ => Err(RpcError::method_not_found(method)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenDocumentSnapshot {
    pub uri: String,
    pub file_name: String,
    pub language_id: String,
    pub version: i32,
    pub text: String,
    pub is_dirty: bool,
    pub is_untitled: bool,
    pub eol: i32,
    pub line_count: usize,
}

impl OpenDocumentSnapshot {
    pub fn from_parts(
        uri: String,
        file_name: String,
        language_id: String,
        version: i32,
        text: String,
        is_dirty: bool,
        is_untitled: bool,
    ) -> Self {
        let eol = if text.contains("\r\n") { 2 } else { 1 };
        let line_count = text.split('\n').count();
        Self {
            uri,
            file_name,
            language_id,
            version,
            text,
            is_dirty,
            is_untitled,
            eol,
            line_count,
        }
    }

    pub fn recompute_metrics(&mut self) {
        self.eol = if self.text.contains("\r\n") { 2 } else { 1 };
        self.line_count = self.text.split('\n').count();
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| {
            json!({
                "uri": self.uri,
                "fileName": self.file_name,
                "languageId": self.language_id,
                "version": self.version,
                "isDirty": self.is_dirty,
                "isUntitled": self.is_untitled,
                "eol": self.eol,
                "text": self.text,
                "lineCount": self.line_count,
            })
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtWorkspaceFolder {
    pub uri: String,
    pub name: String,
    pub path: String,
    pub index: usize,
}

impl ExtWorkspaceFolder {
    pub fn from_path(index: usize, path: impl AsRef<Path>) -> Self {
        let cleaned = rustcode_platform::strip_windows_verbatim_prefix(path.as_ref().to_path_buf());
        let name = cleaned
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| cleaned.to_string_lossy().into_owned());
        let uri = rustcode_platform::path_to_file_uri(&cleaned);
        Self {
            uri,
            name,
            path: cleaned.to_string_lossy().into_owned(),
            index,
        }
    }
}

pub struct LanguageFeatureBridge {
    pub provider_registry: Arc<ProviderRegistry>,
    pub rpc_bridge: Arc<RpcBridge>,
    pub lsp_manager: Arc<LspClientManager>,
    pub diagnostic_manager: Arc<DiagnosticCollectionManager>,
    pub status_bar_manager: Arc<ExtStatusBarManager>,
    pub notebook_manager: Arc<crate::notebook::ExtHostNotebookManager>,
    pub output_manager: Arc<crate::output::ExtHostOutputManager>,
    pub terminal_manager: Arc<crate::terminal::ExtHostTerminalManager>,
    pub scm_manager: Arc<crate::scm::ExtHostScmManager>,
    pub debug_manager: Arc<crate::debug::ExtHostDebugManager>,
    pub task_manager: Arc<crate::tasks::ExtHostTaskManager>,
    pub test_manager: Arc<crate::testing::ExtHostTestManager>,
    pub auth_manager: Arc<crate::auth::ExtHostAuthManager>,
    pub webview_manager: Arc<crate::webview::ExtHostWebviewManager>,
    pub custom_editor_manager: Arc<crate::webview::ExtHostCustomEditorManager>,
    pub webview_view_manager: Arc<crate::webview::ExtHostWebviewViewManager>,
    pub vfs_manager: Arc<crate::vfs::ExtHostVfsManager>,
    pub secret_manager: Arc<crate::secrets::ExtHostSecretManager>,
    pub profiler: Arc<crate::profiler::ExtHostProfiler>,
    pub window_manager: Arc<crate::window::ExtHostWindowManager>,
    pub env_manager: Arc<crate::env::ExtHostEnvManager>,
    pub configuration_service: ConfigurationService,
    pub command_registry: CommandRegistry,
    pub file_system_service: Arc<FileSystemService>,
    fs_root: Option<PathBuf>,
    workspace_folders: Arc<RwLock<Vec<ExtWorkspaceFolder>>>,
    open_documents: Arc<RwLock<HashMap<String, OpenDocumentSnapshot>>>,
    tree_view_options: Arc<RwLock<HashMap<String, super::TreeViewOptionsMeta>>>,
    pending_workspace_edits: Arc<Mutex<Vec<Value>>>,
    command_disposables: Arc<Mutex<HashMap<String, Box<dyn Disposable>>>>,
    js_commands: Arc<RwLock<HashSet<String>>>,
    command_mutation_lock: Arc<Mutex<()>>,
    active_js_commands: Arc<Mutex<HashMap<String, usize>>>,
    command_timeout: Duration,
    config_subscription: Arc<Mutex<Option<Box<dyn Disposable>>>>,
    next_untitled_id: AtomicU64,
    next_editor_id: AtomicU64,
}

impl Default for LanguageFeatureBridge {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl LanguageFeatureBridge {
    pub fn with_defaults() -> Self {
        Self::new(
            Arc::new(ProviderRegistry::new()),
            Arc::new(RpcBridge::new()),
            Arc::new(LspClientManager::new()),
        )
    }

    pub fn new(
        provider_registry: Arc<ProviderRegistry>,
        rpc_bridge: Arc<RpcBridge>,
        lsp_manager: Arc<LspClientManager>,
    ) -> Self {
        let bridge = Self {
            provider_registry,
            rpc_bridge,
            lsp_manager,
            diagnostic_manager: Arc::new(DiagnosticCollectionManager::new()),
            status_bar_manager: Arc::new(ExtStatusBarManager::new()),
            notebook_manager: Arc::new(crate::notebook::ExtHostNotebookManager::new()),
            output_manager: Arc::new(crate::output::ExtHostOutputManager::new()),
            terminal_manager: Arc::new(crate::terminal::ExtHostTerminalManager::new()),
            scm_manager: Arc::new(crate::scm::ExtHostScmManager::new()),
            debug_manager: Arc::new(crate::debug::ExtHostDebugManager::new()),
            task_manager: Arc::new(crate::tasks::ExtHostTaskManager::new()),
            test_manager: Arc::new(crate::testing::ExtHostTestManager::new()),
            auth_manager: Arc::new(crate::auth::ExtHostAuthManager::new()),
            webview_manager: Arc::new(crate::webview::ExtHostWebviewManager::new()),
            custom_editor_manager: Arc::new(crate::webview::ExtHostCustomEditorManager::new()),
            webview_view_manager: Arc::new(crate::webview::ExtHostWebviewViewManager::new()),
            vfs_manager: Arc::new(crate::vfs::ExtHostVfsManager::new()),
            secret_manager: Arc::new(crate::secrets::ExtHostSecretManager::new()),
            profiler: Arc::new(crate::profiler::ExtHostProfiler::new()),
            window_manager: Arc::new(crate::window::ExtHostWindowManager::new()),
            env_manager: Arc::new(crate::env::ExtHostEnvManager::new()),
            configuration_service: ConfigurationService::new(),
            command_registry: CommandRegistry::new(),
            file_system_service: Arc::new(FileSystemService::new()),
            fs_root: None,
            workspace_folders: Arc::new(RwLock::new(Vec::new())),
            open_documents: Arc::new(RwLock::new(HashMap::new())),
            tree_view_options: Arc::new(RwLock::new(HashMap::new())),
            pending_workspace_edits: Arc::new(Mutex::new(Vec::new())),
            command_disposables: Arc::new(Mutex::new(HashMap::new())),
            js_commands: Arc::new(RwLock::new(HashSet::new())),
            command_mutation_lock: Arc::new(Mutex::new(())),
            active_js_commands: Arc::new(Mutex::new(HashMap::new())),
            command_timeout: Duration::from_millis(500),
            config_subscription: Arc::new(Mutex::new(None)),
            next_untitled_id: AtomicU64::new(1),
            next_editor_id: AtomicU64::new(1),
        };
        bridge.attach_configuration_listener();
        bridge
    }

    fn attach_configuration_listener(&self) {
        if let Some(mut old) = self.config_subscription.lock().take() {
            old.dispose();
        }
        let rpc = Arc::clone(&self.rpc_bridge);
        let cs = self.configuration_service.clone();
        let disp = self.configuration_service.on_did_change.event().attach(move |ev| {
            let folder_configs: serde_json::Map<String, Value> = cs
                .folder_settings_snapshot()
                .into_iter()
                .map(|(p, v)| (rustcode_platform::path_to_file_uri(&p), v))
                .collect();
            let affected_folder_uris: Vec<String> = ev
                .affected_folders
                .iter()
                .map(|p| rustcode_platform::path_to_file_uri(p))
                .collect();
            rpc.notify(
                "workspace.onDidChangeConfiguration",
                json!({
                    "affectedKeys": ev.affected_keys,
                    "affectedFolderUris": affected_folder_uris,
                    "config": cs.get_all(),
                    "folderConfigs": folder_configs,
                }),
            );
        });
        *self.config_subscription.lock() = Some(disp);
    }

    pub fn with_diagnostic_manager(mut self, dm: Arc<DiagnosticCollectionManager>) -> Self {
        self.diagnostic_manager = dm;
        self
    }

    pub fn with_status_bar_manager(mut self, sbm: Arc<ExtStatusBarManager>) -> Self {
        self.status_bar_manager = sbm;
        self
    }

    pub fn with_notebook_manager(
        mut self,
        nm: Arc<crate::notebook::ExtHostNotebookManager>,
    ) -> Self {
        self.notebook_manager = nm;
        self
    }

    pub fn with_output_manager(
        mut self,
        om: Arc<crate::output::ExtHostOutputManager>,
    ) -> Self {
        self.output_manager = om;
        self
    }

    pub fn with_terminal_manager(
        mut self,
        tm: Arc<crate::terminal::ExtHostTerminalManager>,
    ) -> Self {
        self.terminal_manager = tm;
        self
    }

    pub fn with_scm_manager(mut self, sm: Arc<crate::scm::ExtHostScmManager>) -> Self {
        self.scm_manager = sm;
        self
    }

    pub fn with_debug_manager(mut self, dm: Arc<crate::debug::ExtHostDebugManager>) -> Self {
        self.debug_manager = dm;
        self
    }

    pub fn with_task_manager(mut self, tm: Arc<crate::tasks::ExtHostTaskManager>) -> Self {
        self.task_manager = tm;
        self
    }

    pub fn with_test_manager(mut self, tm: Arc<crate::testing::ExtHostTestManager>) -> Self {
        self.test_manager = tm;
        self
    }

    pub fn with_auth_manager(mut self, am: Arc<crate::auth::ExtHostAuthManager>) -> Self {
        self.auth_manager = am;
        self
    }

    pub fn with_webview_manager(
        mut self,
        wm: Arc<crate::webview::ExtHostWebviewManager>,
    ) -> Self {
        self.webview_manager = wm;
        self
    }

    pub fn with_custom_editor_manager(
        mut self,
        cem: Arc<crate::webview::ExtHostCustomEditorManager>,
    ) -> Self {
        self.custom_editor_manager = cem;
        self
    }

    pub fn with_webview_view_manager(
        mut self,
        wvm: Arc<crate::webview::ExtHostWebviewViewManager>,
    ) -> Self {
        self.webview_view_manager = wvm;
        self
    }

    pub fn with_vfs_manager(mut self, vfs: Arc<crate::vfs::ExtHostVfsManager>) -> Self {
        self.vfs_manager = vfs;
        self
    }

    pub fn with_secret_manager(mut self, sm: Arc<crate::secrets::ExtHostSecretManager>) -> Self {
        self.secret_manager = sm;
        self
    }

    pub fn with_profiler(mut self, profiler: Arc<crate::profiler::ExtHostProfiler>) -> Self {
        self.profiler = profiler;
        self
    }

    pub fn vfs_manager(&self) -> &Arc<crate::vfs::ExtHostVfsManager> {
        &self.vfs_manager
    }

    pub fn secret_manager(&self) -> &Arc<crate::secrets::ExtHostSecretManager> {
        &self.secret_manager
    }

    pub fn webview_view_manager(&self) -> &Arc<crate::webview::ExtHostWebviewViewManager> {
        &self.webview_view_manager
    }

    pub fn profiler(&self) -> &Arc<crate::profiler::ExtHostProfiler> {
        &self.profiler
    }

    pub fn tree_view_options(&self, view_id: &str) -> Option<super::TreeViewOptionsMeta> {
        self.tree_view_options.read().get(view_id).cloned()
    }

    pub async fn request_tree_handle_drag(
        &self,
        view_id: &str,
        source_element_ids: &[String],
    ) -> Result<Value, String> {
        self.rpc_bridge
            .request(
                "treeView.handleDrag",
                json!({
                    "viewId": view_id,
                    "sourceElementIds": source_element_ids,
                }),
                Duration::from_millis(800),
            )
            .await
    }

    pub async fn request_tree_handle_drop(
        &self,
        view_id: &str,
        target_element_id: Option<&str>,
        data_transfer: Value,
    ) -> Result<Value, String> {
        self.rpc_bridge
            .request(
                "treeView.handleDrop",
                json!({
                    "viewId": view_id,
                    "targetElementId": target_element_id,
                    "dataTransfer": data_transfer,
                }),
                Duration::from_millis(800),
            )
            .await
    }

    pub async fn ping_ext_host(&self) -> Result<u64, String> {
        let started = std::time::Instant::now();
        let res = self
            .rpc_bridge
            .request("extHost.ping", json!({}), Duration::from_secs(3))
            .await?;
        let latency_ms = started.elapsed().as_millis() as u64;
        if let Some(mem) = res.get("memory") {
            if let Ok(stats) = serde_json::from_value::<crate::profiler::ExtHostMemoryStats>(mem.clone()) {
                self.profiler.update_memory(stats);
            }
        }
        Ok(latency_ms)
    }

    pub fn scm_manager(&self) -> &Arc<crate::scm::ExtHostScmManager> {
        &self.scm_manager
    }

    pub fn debug_manager(&self) -> &Arc<crate::debug::ExtHostDebugManager> {
        &self.debug_manager
    }

    pub fn task_manager(&self) -> &Arc<crate::tasks::ExtHostTaskManager> {
        &self.task_manager
    }

    pub fn test_manager(&self) -> &Arc<crate::testing::ExtHostTestManager> {
        &self.test_manager
    }

    pub fn auth_manager(&self) -> &Arc<crate::auth::ExtHostAuthManager> {
        &self.auth_manager
    }

    pub fn webview_manager(&self) -> &Arc<crate::webview::ExtHostWebviewManager> {
        &self.webview_manager
    }

    pub fn custom_editor_manager(&self) -> &Arc<crate::webview::ExtHostCustomEditorManager> {
        &self.custom_editor_manager
    }

    pub fn with_window_manager(mut self, wm: Arc<crate::window::ExtHostWindowManager>) -> Self {
        self.window_manager = wm;
        self
    }

    pub fn with_env_manager(mut self, em: Arc<crate::env::ExtHostEnvManager>) -> Self {
        self.env_manager = em;
        self
    }

    pub fn with_extensions(self, extensions: Vec<crate::manifest::ExtensionSnapshot>) -> Self {
        self.env_manager.set_extensions(extensions);
        self
    }

    pub fn with_extension_loader(self, loader: &crate::manifest::ExtensionLoader) -> Self {
        self.env_manager.hydrate_from_loader(loader);
        self
    }

    pub fn with_shared_extension_loader(
        self,
        loader: Arc<RwLock<crate::manifest::ExtensionLoader>>,
    ) -> Self {
        self.env_manager.set_extension_loader(loader);
        self
    }

    pub fn set_extension_loader(&self, loader: Arc<RwLock<crate::manifest::ExtensionLoader>>) {
        self.env_manager.set_extension_loader(loader);
    }

    pub fn sync_extension_loader(&self, loader: &crate::manifest::ExtensionLoader) {
        self.env_manager.hydrate_from_loader(loader);
        self.rpc_bridge.notify(
            "extensions.onDidChange",
            json!(self.env_manager.list_extensions()),
        );
    }

    pub fn with_app_root(self, root: impl AsRef<Path>) -> Self {
        self.env_manager.set_app_root(root);
        self
    }

    pub fn with_os_env_backends(self) -> Self {
        self.env_manager
            .set_clipboard_backend(Arc::new(crate::env::OsClipboardBackend::new()));
        self.env_manager
            .set_external_uri_opener(Arc::new(crate::env::OsExternalUriOpener::new(true)));
        self
    }

    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }

    pub fn with_configuration_service(mut self, cs: ConfigurationService) -> Self {
        self.configuration_service = cs;
        self.attach_configuration_listener();
        self
    }

    pub fn with_command_registry(mut self, cr: CommandRegistry) -> Self {
        self.command_registry = cr;
        self
    }

    pub fn with_file_system_service(mut self, fs: Arc<FileSystemService>) -> Self {
        self.file_system_service = fs;
        self
    }

    pub fn with_fs_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.fs_root = Some(root.into());
        self
    }

    pub fn with_workspace_folders(self, folders: Vec<ExtWorkspaceFolder>) -> Self {
        *self.workspace_folders.write() = folders;
        self
    }

    pub fn workspace_folders(&self) -> Vec<ExtWorkspaceFolder> {
        self.workspace_folders.read().clone()
    }

    pub fn set_workspace_folders(&self, mut folders: Vec<ExtWorkspaceFolder>) {
        for (idx, f) in folders.iter_mut().enumerate() {
            f.index = idx;
        }
        let prev = {
            let mut guard = self.workspace_folders.write();
            let old = guard.clone();
            *guard = folders.clone();
            old
        };
        let added: Vec<ExtWorkspaceFolder> = folders
            .iter()
            .filter(|f| {
                !prev
                    .iter()
                    .any(|p| rustcode_platform::paths_equivalent(Path::new(&p.path), Path::new(&f.path)))
            })
            .cloned()
            .collect();
        let removed: Vec<ExtWorkspaceFolder> = prev
            .iter()
            .filter(|p| {
                !folders
                    .iter()
                    .any(|f| rustcode_platform::paths_equivalent(Path::new(&p.path), Path::new(&f.path)))
            })
            .cloned()
            .collect();
        self.rpc_bridge.notify(
            "workspace.onDidChangeWorkspaceFolders",
            json!({
                "folders": folders,
                "added": added,
                "removed": removed,
            }),
        );
    }

    pub fn add_workspace_folder(
        &self,
        path: impl AsRef<Path>,
        name: Option<&str>,
    ) -> ExtWorkspaceFolder {
        let mut folders = self.workspace_folders();
        let target = rustcode_platform::strip_windows_verbatim_prefix(path.as_ref().to_path_buf());
        if let Some(existing) = folders
            .iter()
            .find(|f| rustcode_platform::paths_equivalent(Path::new(&f.path), &target))
            .cloned()
        {
            return existing;
        }
        let mut folder = ExtWorkspaceFolder::from_path(folders.len(), &target);
        if let Some(custom_name) = name.filter(|s| !s.trim().is_empty()) {
            folder.name = custom_name.to_string();
        }
        folders.push(folder.clone());
        self.set_workspace_folders(folders);
        folder
    }

    pub fn remove_workspace_folder(&self, path: impl AsRef<Path>) -> bool {
        let target = rustcode_platform::strip_windows_verbatim_prefix(path.as_ref().to_path_buf());
        let mut folders = self.workspace_folders();
        let before = folders.len();
        folders.retain(|f| !rustcode_platform::paths_equivalent(Path::new(&f.path), &target));
        if folders.len() == before {
            return false;
        }
        self.set_workspace_folders(folders);
        true
    }

    pub fn provider_registry(&self) -> &Arc<ProviderRegistry> {
        &self.provider_registry
    }

    pub fn rpc_bridge(&self) -> &Arc<RpcBridge> {
        &self.rpc_bridge
    }

    pub fn lsp_manager(&self) -> &Arc<LspClientManager> {
        &self.lsp_manager
    }

    pub fn diagnostic_manager(&self) -> &Arc<DiagnosticCollectionManager> {
        &self.diagnostic_manager
    }

    pub fn status_bar_manager(&self) -> &Arc<ExtStatusBarManager> {
        &self.status_bar_manager
    }

    pub fn notebook_manager(&self) -> &Arc<crate::notebook::ExtHostNotebookManager> {
        &self.notebook_manager
    }

    pub fn output_manager(&self) -> &Arc<crate::output::ExtHostOutputManager> {
        &self.output_manager
    }

    pub fn terminal_manager(&self) -> &Arc<crate::terminal::ExtHostTerminalManager> {
        &self.terminal_manager
    }

    pub fn window_manager(&self) -> &Arc<crate::window::ExtHostWindowManager> {
        &self.window_manager
    }

    pub fn env_manager(&self) -> &Arc<crate::env::ExtHostEnvManager> {
        &self.env_manager
    }

    pub fn configuration_service(&self) -> &ConfigurationService {
        &self.configuration_service
    }

    pub fn command_registry(&self) -> &CommandRegistry {
        &self.command_registry
    }

    pub fn file_system_service(&self) -> &Arc<FileSystemService> {
        &self.file_system_service
    }

    pub fn fs_root(&self) -> Option<&Path> {
        self.fs_root.as_deref()
    }

    pub fn is_js_command(&self, command: &str) -> bool {
        let _mutation_guard = self.command_mutation_lock.lock();
        self.js_commands.read().contains(command)
    }

    pub fn list_js_commands(&self) -> Vec<String> {
        let _mutation_guard = self.command_mutation_lock.lock();
        let mut cmds: Vec<String> = self.js_commands.read().iter().cloned().collect();
        cmds.sort();
        cmds
    }

    pub fn active_editor(&self) -> Option<crate::window::ExtTextEditorState> {
        self.window_manager.active_editor()
    }

    pub fn set_active_editor(&self, editor: Option<crate::window::ExtTextEditorState>) {
        self.window_manager.set_active_editor(editor.clone());
        let payload = editor
            .as_ref()
            .and_then(|e| serde_json::to_value(e).ok())
            .unwrap_or(Value::Null);
        self.rpc_bridge
            .notify("window.onDidChangeActiveTextEditor", payload);
    }

    pub fn visible_editors(&self) -> Vec<crate::window::ExtTextEditorState> {
        self.window_manager.visible_editors()
    }

    pub fn set_visible_editors(&self, editors: Vec<crate::window::ExtTextEditorState>) {
        self.window_manager.set_visible_editors(editors.clone());
        let payload = serde_json::to_value(&editors).unwrap_or_else(|_| json!([]));
        self.rpc_bridge
            .notify("window.onDidChangeVisibleTextEditors", payload);
    }

    pub fn did_change_active_editor(&self, uri: Option<&str>) {
        let Some(target_uri) = uri else {
            self.set_active_editor(None);
            return;
        };
        let doc = {
            let docs = self.open_documents.read();
            find_open_document_key(&docs, target_uri)
                .and_then(|k| docs.get(&k).cloned())
                .unwrap_or_else(|| {
                    let path = decode_file_uri(target_uri);
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    OpenDocumentSnapshot::from_parts(
                        target_uri.to_string(),
                        path.to_string_lossy().into_owned(),
                        rustcode_base::map_ext_to_lang_id(ext),
                        1,
                        String::new(),
                        false,
                        target_uri.starts_with("untitled:"),
                    )
                })
        };
        let editor_id = format!(
            "editor_{}",
            self.next_editor_id.fetch_add(1, Ordering::SeqCst)
        );
        let zero = LspPosition::new(0, 0);
        let sel = crate::window::ExtSelection::from_range(LspRange::new(zero, zero));
        let editor = crate::window::ExtTextEditorState {
            id: editor_id,
            document: doc,
            selection: sel.clone(),
            selections: vec![sel],
            visible_ranges: vec![LspRange::new(zero, zero)],
            view_column: Some(1),
            options: json!({}),
        };
        let mut visible = self.visible_editors();
        if !visible
            .iter()
            .any(|e| e.document.uri == editor.document.uri)
        {
            visible.push(editor.clone());
            self.set_visible_editors(visible);
        }
        self.set_active_editor(Some(editor));
    }

    pub fn take_pending_show_document_requests(
        &self,
    ) -> Vec<crate::window::ShowTextDocumentRequest> {
        self.window_manager.take_pending_show_document_requests()
    }

    pub fn take_pending_workspace_edits(&self) -> Vec<Value> {
        std::mem::take(&mut *self.pending_workspace_edits.lock())
    }

    pub fn push_initial_state(&self) {
        let folders = self.workspace_folders();
        self.rpc_bridge.notify(
            "workspace.onDidChangeWorkspaceFolders",
            json!({ "folders": folders, "added": folders, "removed": [] }),
        );
        let folder_configs: serde_json::Map<String, Value> = self
            .configuration_service
            .folder_settings_snapshot()
            .into_iter()
            .map(|(p, v)| (rustcode_platform::path_to_file_uri(&p), v))
            .collect();
        self.rpc_bridge.notify(
            "workspace.onDidChangeConfiguration",
            json!({
                "affectedKeys": ["*"],
                "config": self.configuration_service.get_all(),
                "folderConfigs": folder_configs,
            }),
        );
        for doc in self.open_documents.read().values() {
            self.rpc_bridge
                .notify("workspace.onDidOpenTextDocument", doc.to_json());
        }
        let active = self
            .window_manager
            .active_editor()
            .and_then(|e| serde_json::to_value(e).ok())
            .unwrap_or(Value::Null);
        self.rpc_bridge
            .notify("window.onDidChangeActiveTextEditor", active);
        let visible = serde_json::to_value(self.window_manager.visible_editors())
            .unwrap_or_else(|_| json!([]));
        self.rpc_bridge
            .notify("window.onDidChangeVisibleTextEditors", visible);
        self.rpc_bridge.notify(
            "extensions.onDidChange",
            json!(self.env_manager.list_extensions()),
        );
    }

    pub async fn activate_extension(
        &self,
        id: &str,
    ) -> Result<crate::manifest::ExtensionSnapshot, RpcError> {
        let initial_snap = self
            .env_manager
            .get_extension(id)
            .ok_or_else(|| RpcError::new("NOT_FOUND", format!("Extension '{}' not found", id)))?;
        if self.rpc_bridge.has_active_transport() {
            self.rpc_bridge
                .request(
                    "extensions.activate",
                    json!({
                        "id": initial_snap.id,
                        "extensionPath": initial_snap.extension_path,
                        "packageJSON": initial_snap.package_json,
                    }),
                    Duration::from_secs(5),
                )
                .await
                .map_err(|e| RpcError::new("EXTENSION_ACTIVATION_FAILED", e))?;
            self.rpc_bridge.sync_incoming_notifications().await;
        }
        let snap = self
            .env_manager
            .activate_extension(id)
            .ok_or_else(|| RpcError::new("NOT_FOUND", format!("Extension '{}' not found", id)))?;
        self.rpc_bridge.notify(
            "extensions.onDidChange",
            json!(self.env_manager.list_extensions()),
        );
        Ok(snap)
    }

    pub fn get_diagnostics(&self, uri: &str) -> Vec<LspDiagnostic> {
        let mut list = self.lsp_manager.get_diagnostics(uri);
        list.extend(self.diagnostic_manager.get_diagnostics_for_uri(uri));
        list
    }

    pub fn get_all_diagnostics(&self) -> HashMap<String, Vec<LspDiagnostic>> {
        let mut all = self.lsp_manager.get_all_diagnostics();
        for (uri, ext_diags) in self.diagnostic_manager.get_all_diagnostics() {
            all.entry(uri).or_default().extend(ext_diags);
        }
        all
    }

    /// Request completions from LSP servers and registered JS extension providers,
    /// returning an aggregated, deduplicated list.
    pub async fn request_completions(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
        trigger_char: Option<char>,
    ) -> Vec<LspCompletionItem> {
        let mut results = Vec::new();
        let mut seen_labels = HashSet::new();

        // 1. Query LSP client if running
        let context = trigger_char.map(|ch| CompletionContext {
            trigger_kind: CompletionTriggerKind::TriggerCharacter,
            trigger_character: Some(ch.to_string()),
        });

        if let Ok(lsp_items) = self
            .lsp_manager
            .request_completions(uri, language_id, position, context)
            .await
        {
            for item in lsp_items {
                if seen_labels.insert(item.label.clone()) {
                    results.push(item);
                }
            }
        }

        // 2. Query registered JS extension providers
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::Completion, language_id);

        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideCompletionItems",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "position": position,
                        "triggerCharacter": trigger_char.map(|c| c.to_string()),
                    }),
                    std::time::Duration::from_millis(250),
                )
                .await
            {
                if let Ok(items) = serde_json::from_value::<Vec<LspCompletionItem>>(val) {
                    for item in items {
                        if seen_labels.insert(item.label.clone()) {
                            results.push(item);
                        }
                    }
                }
            }
        }

        results
    }

    /// Request hover documentation from LSP or JS extension providers.
    pub async fn request_hover(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Option<Hover> {
        // 1. Query LSP first
        if let Ok(Some(hover)) = self.lsp_manager.request_hover(uri, language_id, position).await {
            return Some(hover);
        }

        // 2. Query JS extension providers
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::Hover, language_id);

        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideHover",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "position": position,
                    }),
                    std::time::Duration::from_millis(250),
                )
                .await
            {
                if let Ok(hover) = serde_json::from_value::<Hover>(val.clone()) {
                    return Some(hover);
                } else if let Some(contents) = val.get("contents").and_then(|c| c.as_str()) {
                    return Some(Hover {
                        contents: MarkupContent::markdown(contents),
                        range: None,
                    });
                }
            }
        }

        None
    }

    /// Request definition locations from LSP or JS extension providers.
    pub async fn request_definition(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Vec<LspLocation> {
        // 1. Query LSP
        if let Ok(defs) = self.lsp_manager.request_definition(uri, language_id, position).await {
            if !defs.is_empty() {
                return defs;
            }
        }

        // 2. Query JS extension providers
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::Definition, language_id);

        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideDefinition",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "position": position,
                    }),
                    std::time::Duration::from_millis(250),
                )
                .await
            {
                if let Ok(locs) = serde_json::from_value::<Vec<LspLocation>>(val.clone()) {
                    return locs;
                } else if let Ok(loc) = serde_json::from_value::<LspLocation>(val) {
                    return vec![loc];
                }
            }
        }

        Vec::new()
    }

    /// Request document formatting edits from LSP or registered JS extension providers.
    pub async fn request_formatting(
        &self,
        uri: &str,
        language_id: &str,
        options: FormattingOptions,
    ) -> Vec<LspTextEdit> {
        // 1. Query LSP first
        if let Ok(edits) = self
            .lsp_manager
            .request_formatting(uri, language_id, options.clone())
            .await
        {
            if !edits.is_empty() {
                return edits;
            }
        }

        // 2. Query registered JS extension providers
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::DocumentFormatting, language_id);

        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideDocumentFormattingEdits",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "options": options,
                    }),
                    std::time::Duration::from_millis(500),
                )
                .await
            {
                if let Ok(edits) = serde_json::from_value::<Vec<LspTextEdit>>(val) {
                    if !edits.is_empty() {
                        return edits;
                    }
                }
            }
        }

        Vec::new()
    }

    /// Request code actions from LSP or registered JS extension providers.
    pub async fn request_code_actions(
        &self,
        uri: &str,
        language_id: &str,
        range: LspRange,
        context: CodeActionContext,
    ) -> Vec<CodeAction> {
        let mut results = Vec::new();

        // 1. Query LSP
        if let Ok(actions) = self
            .lsp_manager
            .request_code_actions(uri, language_id, range, context.clone())
            .await
        {
            results.extend(actions);
        }

        // 2. Query registered JS extension providers
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::CodeAction, language_id);

        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideCodeActions",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "range": range,
                        "context": context,
                    }),
                    std::time::Duration::from_millis(500),
                )
                .await
            {
                if let Ok(actions) = serde_json::from_value::<Vec<CodeAction>>(val.clone()) {
                    results.extend(actions);
                } else if let Ok(commands) = serde_json::from_value::<Vec<LspCommand>>(val) {
                    results.extend(commands.into_iter().map(|cmd| CodeAction {
                        title: cmd.title.clone(),
                        kind: Some(CodeActionKind::QUICKFIX.to_string()),
                        diagnostics: None,
                        is_preferred: None,
                        edit: None,
                        command: Some(cmd),
                    }));
                }
            }
        }

        results
    }

    /// Request signature help from LSP or registered JS extension providers.
    pub async fn request_signature_help(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Option<SignatureHelp> {
        if let Ok(Some(sig)) = self
            .lsp_manager
            .request_signature_help(uri, language_id, position)
            .await
        {
            if !sig.signatures.is_empty() {
                return Some(sig);
            }
        }

        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::SignatureHelp, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideSignatureHelp",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "position": position,
                    }),
                    std::time::Duration::from_millis(250),
                )
                .await
            {
                if let Ok(sig) = serde_json::from_value::<SignatureHelp>(val) {
                    if !sig.signatures.is_empty() {
                        return Some(sig);
                    }
                }
            }
        }
        None
    }

    /// Request symbol references from LSP and registered JS extension providers.
    pub async fn request_references(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
        include_declaration: bool,
    ) -> Vec<LspLocation> {
        let mut results = Vec::new();
        if let Ok(locs) = self
            .lsp_manager
            .request_references(uri, language_id, position, include_declaration)
            .await
        {
            results.extend(locs);
        }

        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::References, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideReferences",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "position": position,
                        "context": { "includeDeclaration": include_declaration },
                    }),
                    std::time::Duration::from_millis(350),
                )
                .await
            {
                if let Ok(locs) = serde_json::from_value::<Vec<LspLocation>>(val) {
                    results.extend(locs);
                }
            }
        }
        results
    }

    async fn request_js_locations(
        &self,
        provider_type: ProviderType,
        rpc_method: &str,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Vec<LspLocation> {
        let js_providers = self
            .provider_registry
            .get_providers_for_language(provider_type, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    rpc_method,
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "position": position,
                    }),
                    std::time::Duration::from_millis(350),
                )
                .await
            {
                if let Ok(locs) = serde_json::from_value::<Vec<LspLocation>>(val.clone()) {
                    if !locs.is_empty() {
                        return locs;
                    }
                } else if let Ok(single) = serde_json::from_value::<LspLocation>(val) {
                    return vec![single];
                }
            }
        }
        Vec::new()
    }

    /// Request implementation locations from LSP or JS extension providers.
    pub async fn request_implementation(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Vec<LspLocation> {
        if let Ok(locs) = self
            .lsp_manager
            .request_implementation(uri, language_id, position)
            .await
        {
            if !locs.is_empty() {
                return locs;
            }
        }
        self.request_js_locations(
            ProviderType::Implementation,
            "languages.provideImplementation",
            uri,
            language_id,
            position,
        )
        .await
    }

    /// Request type definition locations from LSP or JS extension providers.
    pub async fn request_type_definition(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Vec<LspLocation> {
        if let Ok(locs) = self
            .lsp_manager
            .request_type_definition(uri, language_id, position)
            .await
        {
            if !locs.is_empty() {
                return locs;
            }
        }
        self.request_js_locations(
            ProviderType::TypeDefinition,
            "languages.provideTypeDefinition",
            uri,
            language_id,
            position,
        )
        .await
    }

    /// Validate rename position and retrieve default placeholder if supported.
    pub async fn request_prepare_rename(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Option<PrepareRenameResult> {
        if let Ok(Some(res)) = self
            .lsp_manager
            .request_prepare_rename(uri, language_id, position)
            .await
        {
            return Some(res);
        }

        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::Rename, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.prepareRename",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "position": position,
                    }),
                    std::time::Duration::from_millis(250),
                )
                .await
            {
                if let Ok(res) = serde_json::from_value::<PrepareRenameResult>(val) {
                    return Some(res);
                }
            }
        }
        None
    }

    /// Request multi-file rename WorkspaceEdit from LSP or JS extension providers.
    pub async fn request_rename(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
        new_name: &str,
    ) -> Option<WorkspaceEdit> {
        if let Ok(Some(edit)) = self
            .lsp_manager
            .request_rename(uri, language_id, position, new_name)
            .await
        {
            return Some(edit);
        }

        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::Rename, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideRenameEdits",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "position": position,
                        "newName": new_name,
                    }),
                    std::time::Duration::from_millis(500),
                )
                .await
            {
                if let Ok(edit) = serde_json::from_value::<WorkspaceEdit>(val) {
                    return Some(edit);
                }
            }
        }
        None
    }

    /// Request decoded semantic tokens for a document from LSP or JS extension providers.
    pub async fn request_semantic_tokens(
        &self,
        uri: &str,
        language_id: &str,
    ) -> Vec<DecodedSemanticToken> {
        if let Ok(Some(tokens)) = self
            .lsp_manager
            .request_semantic_tokens_full(uri, language_id)
            .await
        {
            if !tokens.data.is_empty() {
                if let Some(legend) = self.lsp_manager.semantic_tokens_legend(language_id) {
                    return decode_semantic_tokens(&tokens.data, &legend);
                }
            }
        }

        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::SemanticTokens, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideDocumentSemanticTokens",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                    }),
                    std::time::Duration::from_millis(500),
                )
                .await
            {
                if let (Ok(tokens), Ok(legend)) = (
                    serde_json::from_value::<SemanticTokens>(val.clone()),
                    val.get("legend")
                        .cloned()
                        .ok_or(())
                        .and_then(|l| serde_json::from_value::<SemanticTokensLegend>(l).map_err(|_| ())),
                ) {
                    let decoded = decode_semantic_tokens(&tokens.data, &legend);
                    if !decoded.is_empty() {
                        return decoded;
                    }
                }
            }
        }
        Vec::new()
    }

    /// Request inlay hints for a range from LSP and registered JS extension providers.
    pub async fn request_inlay_hints(
        &self,
        uri: &str,
        language_id: &str,
        range: LspRange,
    ) -> Vec<InlayHint> {
        let mut results = Vec::new();
        if let Ok(hints) = self
            .lsp_manager
            .request_inlay_hints(uri, language_id, range)
            .await
        {
            results.extend(hints);
        }

        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::InlayHints, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideInlayHints",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "range": range,
                    }),
                    std::time::Duration::from_millis(250),
                )
                .await
            {
                if let Ok(hints) = serde_json::from_value::<Vec<InlayHint>>(val) {
                    results.extend(hints);
                }
            }
        }
        results
    }

    /// Request CodeLenses for a document from LSP and registered JS extension providers.
    pub async fn request_code_lenses(&self, uri: &str, language_id: &str) -> Vec<CodeLens> {
        let mut results = Vec::new();
        if let Ok(lenses) = self.lsp_manager.request_code_lens(uri, language_id).await {
            results.extend(lenses);
        }

        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::CodeLens, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideCodeLenses",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                    }),
                    std::time::Duration::from_millis(250),
                )
                .await
            {
                if let Ok(lenses) = serde_json::from_value::<Vec<CodeLens>>(val) {
                    results.extend(lenses);
                }
            }
        }
        results
    }

    /// Request DocumentLinks for a document from LSP and registered JS extension providers.
    pub async fn request_document_links(
        &self,
        uri: &str,
        language_id: &str,
    ) -> Vec<DocumentLink> {
        let mut results = Vec::new();
        if let Ok(links) = self
            .lsp_manager
            .request_document_links(uri, language_id)
            .await
        {
            results.extend(links);
        }

        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::DocumentLink, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideDocumentLinks",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                    }),
                    std::time::Duration::from_millis(250),
                )
                .await
            {
                if let Ok(links) = serde_json::from_value::<Vec<DocumentLink>>(val) {
                    results.extend(links);
                }
            }
        }
        results
    }

    pub async fn request_call_hierarchy_prepare(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Vec<CallHierarchyItem> {
        if let Ok(items) = self
            .lsp_manager
            .request_prepare_call_hierarchy(uri, language_id, position)
            .await
        {
            if !items.is_empty() {
                return items;
            }
        }
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::CallHierarchy, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.prepareCallHierarchy",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "position": position,
                    }),
                    std::time::Duration::from_millis(350),
                )
                .await
            {
                if let Ok(items) = serde_json::from_value::<Vec<CallHierarchyItem>>(val.clone()) {
                    return items;
                } else if let Ok(item) = serde_json::from_value::<CallHierarchyItem>(val) {
                    return vec![item];
                }
            }
        }
        Vec::new()
    }

    pub async fn request_call_hierarchy_incoming(
        &self,
        language_id: &str,
        item: &CallHierarchyItem,
    ) -> Vec<CallHierarchyIncomingCall> {
        if let Ok(calls) = self
            .lsp_manager
            .request_call_hierarchy_incoming(language_id, item)
            .await
        {
            if !calls.is_empty() {
                return calls;
            }
        }
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::CallHierarchy, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideCallHierarchyIncomingCalls",
                    json!({
                        "providerId": provider.id,
                        "item": item,
                    }),
                    std::time::Duration::from_millis(350),
                )
                .await
            {
                if let Ok(calls) = serde_json::from_value::<Vec<CallHierarchyIncomingCall>>(val) {
                    return calls;
                }
            }
        }
        Vec::new()
    }

    pub async fn request_call_hierarchy_outgoing(
        &self,
        language_id: &str,
        item: &CallHierarchyItem,
    ) -> Vec<CallHierarchyOutgoingCall> {
        if let Ok(calls) = self
            .lsp_manager
            .request_call_hierarchy_outgoing(language_id, item)
            .await
        {
            if !calls.is_empty() {
                return calls;
            }
        }
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::CallHierarchy, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideCallHierarchyOutgoingCalls",
                    json!({
                        "providerId": provider.id,
                        "item": item,
                    }),
                    std::time::Duration::from_millis(350),
                )
                .await
            {
                if let Ok(calls) = serde_json::from_value::<Vec<CallHierarchyOutgoingCall>>(val) {
                    return calls;
                }
            }
        }
        Vec::new()
    }

    pub async fn request_type_hierarchy_prepare(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Vec<TypeHierarchyItem> {
        if let Ok(items) = self
            .lsp_manager
            .request_prepare_type_hierarchy(uri, language_id, position)
            .await
        {
            if !items.is_empty() {
                return items;
            }
        }
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::TypeHierarchy, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.prepareTypeHierarchy",
                    json!({
                        "providerId": provider.id,
                        "uri": uri,
                        "position": position,
                    }),
                    std::time::Duration::from_millis(350),
                )
                .await
            {
                if let Ok(items) = serde_json::from_value::<Vec<TypeHierarchyItem>>(val.clone()) {
                    return items;
                } else if let Ok(item) = serde_json::from_value::<TypeHierarchyItem>(val) {
                    return vec![item];
                }
            }
        }
        Vec::new()
    }

    pub async fn request_type_hierarchy_supertypes(
        &self,
        language_id: &str,
        item: &TypeHierarchyItem,
    ) -> Vec<TypeHierarchyItem> {
        if let Ok(items) = self
            .lsp_manager
            .request_type_hierarchy_supertypes(language_id, item)
            .await
        {
            if !items.is_empty() {
                return items;
            }
        }
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::TypeHierarchy, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideTypeHierarchySupertypes",
                    json!({
                        "providerId": provider.id,
                        "item": item,
                    }),
                    std::time::Duration::from_millis(350),
                )
                .await
            {
                if let Ok(items) = serde_json::from_value::<Vec<TypeHierarchyItem>>(val) {
                    return items;
                }
            }
        }
        Vec::new()
    }

    pub async fn request_type_hierarchy_subtypes(
        &self,
        language_id: &str,
        item: &TypeHierarchyItem,
    ) -> Vec<TypeHierarchyItem> {
        if let Ok(items) = self
            .lsp_manager
            .request_type_hierarchy_subtypes(language_id, item)
            .await
        {
            if !items.is_empty() {
                return items;
            }
        }
        let js_providers = self
            .provider_registry
            .get_providers_for_language(ProviderType::TypeHierarchy, language_id);
        for provider in js_providers {
            if let Ok(val) = self
                .rpc_bridge
                .request(
                    "languages.provideTypeHierarchySubtypes",
                    json!({
                        "providerId": provider.id,
                        "item": item,
                    }),
                    std::time::Duration::from_millis(350),
                )
                .await
            {
                if let Ok(items) = serde_json::from_value::<Vec<TypeHierarchyItem>>(val) {
                    return items;
                }
            }
        }
        Vec::new()
    }
    pub async fn request_tree_children_result(
        &self,
        provider_id: &str,
        element_id: Option<&str>,
    ) -> Result<Vec<TreeItem>, String> {
        let val = self
            .rpc_bridge
            .request(
                "treeView.getChildren",
                json!({
                    "providerId": provider_id,
                    "element": element_id,
                }),
                std::time::Duration::from_millis(500),
            )
            .await?;
        serde_json::from_value::<Vec<TreeItem>>(val)
            .map_err(|e| format!("Failed to deserialize TreeItem list: {}", e))
    }

    /// Request hierarchical tree items for a tree data provider.
    pub async fn request_tree_children(
        &self,
        provider_id: &str,
        element_id: Option<&str>,
    ) -> Vec<TreeItem> {
        self.request_tree_children_result(provider_id, element_id)
            .await
            .unwrap_or_default()
    }

    /// Request a single resolved TreeItem from a TreeDataProvider.
    pub async fn request_tree_item(
        &self,
        provider_id: &str,
        element_id: &str,
    ) -> Option<TreeItem> {
        let val = self
            .rpc_bridge
            .request(
                "treeView.getTreeItem",
                json!({
                    "providerId": provider_id,
                    "element": element_id,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .ok()?;
        serde_json::from_value::<TreeItem>(val).ok()
    }

    /// Request live InputBox validation from the extension host JS callback.
    pub async fn request_validate_input(
        &self,
        client_id: &str,
        value: &str,
    ) -> Result<Option<Value>, String> {
        let val = self
            .rpc_bridge
            .request(
                "window.validateInput",
                json!({
                    "clientId": client_id,
                    "value": value,
                }),
                std::time::Duration::from_millis(500),
            )
            .await?;
        if val.is_null() {
            Ok(None)
        } else {
            Ok(Some(val))
        }
    }

    /// Notify extension host that a QuickPick item was highlighted/selected for `options.onDidSelectItem`.
    pub fn notify_quick_pick_item_selected(&self, client_id: &str, item: Value) {
        self.rpc_bridge.notify(
            "window.quickPickDidSelectItem",
            json!({
                "clientId": client_id,
                "item": item,
            }),
        );
    }

    pub async fn request_notebook_deserialize(
        &self,
        notebook_type: &str,
        content: &[u8],
    ) -> Result<crate::notebook::ExtNotebookData, String> {
        self.notebook_manager
            .request_deserialize(&self.rpc_bridge, notebook_type, content)
            .await
    }

    pub async fn request_notebook_serialize(
        &self,
        notebook_type: &str,
        data: &crate::notebook::ExtNotebookData,
    ) -> Result<Vec<u8>, String> {
        self.notebook_manager
            .request_serialize(&self.rpc_bridge, notebook_type, data)
            .await
    }

    pub async fn request_notebook_execute_cells(
        &self,
        controller_id: &str,
        notebook_uri: &str,
        cell_indices: &[usize],
        cells: &[crate::notebook::ExtNotebookCellData],
    ) -> Result<serde_json::Value, String> {
        self.notebook_manager
            .request_execute_cells(
                &self.rpc_bridge,
                controller_id,
                notebook_uri,
                cell_indices,
                cells,
            )
            .await
    }

    pub async fn request_scm_original_resource(
        &self,
        provider_id: &str,
        uri: &str,
    ) -> Result<Option<String>, String> {
        self.scm_manager
            .request_provide_original_resource(&self.rpc_bridge, provider_id, uri)
            .await
    }

    pub async fn request_scm_accept_input(
        &self,
        provider_id: &str,
        value: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.scm_manager
            .request_accept_input(&self.rpc_bridge, provider_id, value)
            .await
    }

    pub async fn request_debug_configurations(
        &self,
        provider_id: &str,
        folder: Option<serde_json::Value>,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.debug_manager
            .request_provide_debug_configurations(&self.rpc_bridge, provider_id, folder)
            .await
    }

    pub async fn request_resolve_debug_configuration(
        &self,
        provider_id: &str,
        folder: Option<serde_json::Value>,
        configuration: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.debug_manager
            .request_resolve_debug_configuration(
                &self.rpc_bridge,
                provider_id,
                folder,
                configuration,
            )
            .await
    }

    pub async fn request_debug_adapter_descriptor(
        &self,
        provider_id: &str,
        session: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.debug_manager
            .request_create_debug_adapter_descriptor(&self.rpc_bridge, provider_id, session)
            .await
    }

    pub async fn request_provide_tasks(
        &self,
        provider_id: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.task_manager
            .request_provide_tasks(&self.rpc_bridge, provider_id)
            .await
    }

    pub async fn request_resolve_task(
        &self,
        provider_id: &str,
        task: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.task_manager
            .request_resolve_task(&self.rpc_bridge, provider_id, task)
            .await
    }

    pub async fn request_resolve_test_children(
        &self,
        controller_id: &str,
        item_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.test_manager
            .request_resolve_children(&self.rpc_bridge, controller_id, item_id)
            .await
    }

    pub async fn request_refresh_tests(
        &self,
        controller_id: &str,
    ) -> Result<serde_json::Value, String> {
        self.test_manager
            .request_refresh_tests(&self.rpc_bridge, controller_id)
            .await
    }

    pub async fn request_run_tests(
        &self,
        controller_id: &str,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.test_manager
            .request_run_tests(&self.rpc_bridge, controller_id, request)
            .await
    }

    pub async fn request_auth_sessions(
        &self,
        provider_id: &str,
        scopes: &[String],
    ) -> Result<Vec<crate::auth::ExtAuthSessionInfo>, String> {
        self.auth_manager
            .request_get_sessions(&self.rpc_bridge, provider_id, scopes)
            .await
    }

    pub async fn request_create_auth_session(
        &self,
        provider_id: &str,
        scopes: &[String],
    ) -> Result<crate::auth::ExtAuthSessionInfo, String> {
        self.auth_manager
            .request_create_session(&self.rpc_bridge, provider_id, scopes)
            .await
    }

    pub async fn request_remove_auth_session(
        &self,
        provider_id: &str,
        session_id: &str,
    ) -> Result<bool, String> {
        self.auth_manager
            .request_remove_session(&self.rpc_bridge, provider_id, session_id)
            .await
    }

    pub async fn request_resolve_custom_editor(
        &self,
        view_type: &str,
        uri: &str,
        panel_id: &str,
        title: &str,
        text: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let val = self
            .custom_editor_manager
            .request_resolve_custom_editor(
                &self.rpc_bridge,
                view_type,
                uri,
                panel_id,
                title,
                text,
            )
            .await?;
        if let Some(html) = val.get("html").and_then(|v| v.as_str()) {
            if !html.is_empty() {
                self.webview_manager.set_html(panel_id, html);
            }
        }
        Ok(val)
    }

    pub async fn request_open_custom_document(
        &self,
        view_type: &str,
        uri: &str,
        backup_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.custom_editor_manager
            .request_open_custom_document(&self.rpc_bridge, view_type, uri, backup_id)
            .await
    }

    pub async fn request_save_custom_document(
        &self,
        view_type: &str,
        uri: &str,
    ) -> Result<serde_json::Value, String> {
        self.custom_editor_manager
            .request_save_custom_document(&self.rpc_bridge, view_type, uri)
            .await
    }

    /// Execute a command either in JS extension host (if registered by JS) or Rust CommandRegistry.
    pub async fn request_execute_command(
        &self,
        command: &str,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        self.request_execute_command_with_timeout(command, args, self.command_timeout)
            .await
    }

    /// Execute a command with an explicit JS RPC timeout and cross-IPC recursion depth guard.
    pub async fn request_execute_command_with_timeout(
        &self,
        command: &str,
        args: Vec<Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        if command.trim().is_empty() {
            return Err("INVALID_PARAMS: Command name must not be empty".to_string());
        }
        let is_js = {
            let _mutation_guard = self.command_mutation_lock.lock();
            self.js_commands.read().contains(command)
        };
        if is_js {
            const MAX_JS_COMMAND_DEPTH: usize = 8;
            struct ActiveCommandGuard<'a> {
                map: &'a Mutex<HashMap<String, usize>>,
                key: String,
            }
            impl Drop for ActiveCommandGuard<'_> {
                fn drop(&mut self) {
                    let mut guard = self.map.lock();
                    if let Some(count) = guard.get_mut(&self.key) {
                        if *count <= 1 {
                            guard.remove(&self.key);
                        } else {
                            *count -= 1;
                        }
                    }
                }
            }

            let _guard = {
                let mut map = self.active_js_commands.lock();
                let depth = map.get(command).copied().unwrap_or(0);
                if depth >= MAX_JS_COMMAND_DEPTH {
                    return Err(format!(
                        "COMMAND_RECURSION: Recursive execution of JS command '{}'",
                        command
                    ));
                }
                map.insert(command.to_string(), depth + 1);
                ActiveCommandGuard {
                    map: &self.active_js_commands,
                    key: command.to_string(),
                }
            };

            self.rpc_bridge
                .request(
                    "commands.executeCommand",
                    json!({ "command": command, "args": args }),
                    timeout,
                )
                .await
                .map_err(|e| {
                    if e.starts_with("RPC request 'commands.executeCommand' timed out") {
                        format!("Timed out executing JS command '{}'", command)
                    } else {
                        e
                    }
                })
        } else {
            let arg_val = if args.is_empty() {
                None
            } else {
                Some(Value::Array(args))
            };
            self.command_registry.execute(command, arg_val)
        }
    }

    /// Handle incoming RPC calls from JS runtime with structured error propagation.
    pub async fn handle_rpc_message_result(
        &self,
        method: &str,
        params: impl std::borrow::Borrow<serde_json::Value>,
    ) -> Result<serde_json::Value, RpcError> {
        let started = std::time::Instant::now();
        let res = self.dispatch_rpc_message_result(method, params.borrow()).await;
        self.profiler
            .record_rpc_call(method, started.elapsed(), res.is_ok());
        res
    }

    async fn dispatch_rpc_message_result(
        &self,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        if method == "extHost.ping" {
            if let Some(mem) = params.get("memory") {
                if let Ok(stats) =
                    serde_json::from_value::<crate::profiler::ExtHostMemoryStats>(mem.clone())
                {
                    self.profiler.update_memory(stats);
                }
            }
            return Ok(json!({ "pong": true }));
        }

        if method == "extHost.getProfile" {
            return serde_json::to_value(self.profiler.snapshot())
                .map_err(|e| RpcError::internal(e.to_string()));
        }

        if method == "extHost.recordActivation" {
            let ext_id = params
                .get("extensionId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| RpcError::invalid_params("Missing extensionId"))?;
            let event = params
                .get("activationEvent")
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            let duration_ms = params
                .get("durationMs")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let succeeded = params
                .get("succeeded")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let error = params
                .get("error")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            self.profiler.record_activation_duration(
                ext_id,
                event,
                Duration::from_millis(duration_ms),
                succeeded,
                error,
            );
            return Ok(json!({ "recorded": true, "extensionId": ext_id }));
        }

        if method.starts_with("secrets.") {
            return self
                .secret_manager
                .handle_rpc_call(method, params)
                .map_err(|e| map_manager_error(method, e));
        }

        if matches!(
            method,
            "languages.setDiagnostics"
                | "languages.clearDiagnostics"
                | "languages.clearAllDiagnostics"
        ) {
            return self.diagnostic_manager.handle_rpc_call(method, params);
        }

        if matches!(
            method,
            "window.setStatusBarItem" | "window.hideStatusBarItem"
        ) {
            return self.status_bar_manager.handle_rpc_call(method, params);
        }

        if method.starts_with("notebooks.") {
            return self
                .notebook_manager
                .handle_rpc_call(method, params)
                .map_err(|e| map_manager_error(method, e));
        }

        if method.starts_with("scm.") {
            return self
                .scm_manager
                .handle_rpc_call(method, params)
                .map_err(|e| map_manager_error(method, e));
        }

        if method.starts_with("debug.") {
            return self
                .debug_manager
                .handle_rpc_call(method, params)
                .map_err(|e| map_manager_error(method, e));
        }

        if method.starts_with("tasks.") {
            return self
                .task_manager
                .handle_rpc_call(method, params)
                .map_err(|e| map_manager_error(method, e));
        }

        if method.starts_with("tests.") {
            return self
                .test_manager
                .handle_rpc_call(method, params)
                .map_err(|e| map_manager_error(method, e));
        }

        if method.starts_with("authentication.") {
            return self
                .auth_manager
                .handle_rpc_call(method, params)
                .map_err(|e| map_manager_error(method, e));
        }

        if method.starts_with("env.") || method.starts_with("extensions.") {
            let res = self
                .env_manager
                .handle_rpc_call(method, params)
                .map_err(|e| map_manager_error(method, e))?;
            if method == "extensions.activate"
                && res.get("activated").and_then(|v| v.as_bool()) == Some(true)
            {
                self.rpc_bridge.notify(
                    "extensions.onDidChange",
                    json!(self.env_manager.list_extensions()),
                );
            }
            return Ok(res);
        }

        if method.starts_with("workspace.") {
            return self.handle_workspace_rpc(method, params).await;
        }

        if method.starts_with("commands.") {
            return self.handle_commands_rpc(method, params);
        }

        if method.starts_with("fs.") {
            return self
                .handle_fs_rpc(method, params)
                .await
                .map_err(RpcError::from_prefixed_string);
        }

        if method.starts_with("process.") {
            return crate::process::handle_process_rpc(method, params, self.fs_root.as_deref())
                .await;
        }

        match method {
            "window.showInformationMessage"
            | "window.showWarningMessage"
            | "window.showErrorMessage"
            | "window.showQuickPick"
            | "window.showInputBox"
            | "window.cancelPrompt" => {
                return self
                    .window_manager
                    .handle_rpc_call(method, params)
                    .await
                    .map_err(|e| map_manager_error(method, e));
            }
            "window.getActiveTextEditor" => {
                return Ok(serde_json::to_value(self.window_manager.active_editor())
                    .unwrap_or(Value::Null));
            }
            "window.getVisibleTextEditors" => {
                return Ok(serde_json::to_value(self.window_manager.visible_editors())
                    .unwrap_or_else(|_| json!([])));
            }
            "window.showTextDocument" => {
                return self.handle_show_text_document_rpc(params).await;
            }
            "treeView.onDidChangeTreeData" => {
                let provider_id = params
                    .get("providerId")
                    .or_else(|| params.get("viewId"))
                    .or_else(|| params.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        RpcError::invalid_params("Missing string 'providerId' parameter")
                    })?;
                let mut element_ids = Vec::new();
                if let Some(arr) = params.get("elementIds").and_then(|v| v.as_array()) {
                    for item in arr {
                        if let Some(s) = item.as_str().filter(|s| !s.is_empty()) {
                            element_ids.push(s.to_string());
                        }
                    }
                } else if let Some(single) = params.get("elementId").and_then(|v| v.as_str()) {
                    if !single.is_empty() {
                        element_ids.push(single.to_string());
                    }
                }
                self.provider_registry
                    .notify_tree_data_changed(provider_id, element_ids);
                return Ok(json!({ "notified": true, "providerId": provider_id }));
            }
            "window.createOutputChannel"
            | "window.appendOutputChannel"
            | "window.replaceOutputChannel"
            | "window.clearOutputChannel"
            | "window.showOutputChannel"
            | "window.hideOutputChannel"
            | "window.disposeOutputChannel"
            | "window.setOutputChannelLogLevel"
            | "window.logOutputChannelMessage" => {
                return self
                    .output_manager
                    .handle_rpc_call(method, params)
                    .map_err(|e| map_manager_error(method, e));
            }
            "window.createTerminal"
            | "window.showTerminal"
            | "window.sendTerminalText"
            | "window.resizeTerminal"
            | "window.exitTerminal"
            | "window.disposeTerminal"
            | "window.reportTerminalShellIntegration"
            | "window.executeTerminalCommand" => {
                return self
                    .terminal_manager
                    .handle_rpc_call(method, params)
                    .map_err(|e| map_manager_error(method, e));
            }
            "window.createWebviewPanel"
            | "window.setWebviewHtml"
            | "window.postMessageToWebview"
            | "window.disposeWebviewPanel"
            | "window.revealWebviewPanel" => {
                return self
                    .webview_manager
                    .handle_rpc_call(method, params.clone())
                    .map_err(|e| map_manager_error(method, e));
            }
            "window.registerWebviewViewProvider"
            | "window.unregisterWebviewViewProvider"
            | "window.setWebviewViewHtml"
            | "window.setWebviewViewMeta"
            | "window.postMessageToWebviewView"
            | "window.showWebviewView"
            | "window.revealWebviewView" => {
                let res = self
                    .webview_view_manager
                    .handle_rpc_call(method, params)
                    .map_err(|e| map_manager_error(method, e))?;
                if method == "window.registerWebviewViewProvider" {
                    if let Some(view_id) = params.get("viewId").and_then(|v| v.as_str()) {
                        self.provider_registry.register(super::ProviderRegistration {
                            id: view_id.to_string(),
                            provider_type: ProviderType::WebviewView,
                            language_selector: None,
                        });
                    }
                } else if method == "window.unregisterWebviewViewProvider" {
                    if let Some(view_id) = params.get("viewId").and_then(|v| v.as_str()) {
                        self.provider_registry.unregister(view_id);
                    }
                }
                return Ok(res);
            }
            "window.registerCustomEditorProvider"
            | "window.unregisterCustomEditorProvider"
            | "window.openCustomEditor"
            | "window.closeCustomEditor" => {
                return self
                    .custom_editor_manager
                    .handle_rpc_call(method, params, Some(&self.webview_manager))
                    .map_err(|e| map_manager_error(method, e));
            }
            "treeView.setOptions" => {
                let meta: super::TreeViewOptionsMeta = serde_json::from_value(params.clone())
                    .map_err(|e| RpcError::invalid_params(format!("Invalid treeView options: {}", e)))?;
                let view_id = meta.view_id.clone();
                if !meta.drag_mime_types.is_empty() || !meta.drop_mime_types.is_empty() {
                    self.provider_registry.register(super::ProviderRegistration {
                        id: format!("treeDnd:{}", view_id),
                        provider_type: ProviderType::TreeDragAndDrop,
                        language_selector: Some(view_id.clone()),
                    });
                }
                self.tree_view_options.write().insert(view_id.clone(), meta);
                return Ok(json!({ "recorded": true, "viewId": view_id }));
            }
            "treeView.onDidChangeCheckboxState" => {
                return Ok(json!({ "recorded": true }));
            }
            _ => {}
        }

        match method {
            "extHost.registerProvider" => {
                let type_str = params
                    .get("type")
                    .and_then(|t| t.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'type' parameter"))?;
                let id = params
                    .get("id")
                    .and_then(|i| i.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'id' parameter"))?;
                let selector = params
                    .get("selector")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string());

                if let Some(provider_type) = ProviderType::from_str_name(type_str) {
                    self.provider_registry.register(super::ProviderRegistration {
                        id: id.to_string(),
                        provider_type,
                        language_selector: selector,
                    });
                    Ok(json!({ "registered": true, "id": id }))
                } else {
                    Err(RpcError::invalid_params(format!(
                        "Unknown provider type '{}'",
                        type_str
                    )))
                }
            }
            "extHost.unregisterProvider" => {
                let id = params
                    .get("id")
                    .and_then(|i| i.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'id' parameter"))?;
                self.provider_registry.unregister(id);
                Ok(json!({ "unregistered": true, "id": id }))
            }
            "languages.format" => {
                let uri = params.get("uri").and_then(|u| u.as_str()).unwrap_or("");
                let language_id = params
                    .get("languageId")
                    .and_then(|l| l.as_str())
                    .unwrap_or("*");
                let options: FormattingOptions = match params.get("options") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| RpcError::invalid_params(format!("Invalid formatting options: {}", e)))?,
                    _ => FormattingOptions::new(4, true),
                };

                let edits = self.request_formatting(uri, language_id, options).await;
                serde_json::to_value(edits).map_err(|e| RpcError::internal(e.to_string()))
            }
            "languages.codeAction" => {
                let uri = params.get("uri").and_then(|u| u.as_str()).unwrap_or("");
                let language_id = params
                    .get("languageId")
                    .and_then(|l| l.as_str())
                    .unwrap_or("*");
                let range: LspRange = match params.get("range") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| RpcError::invalid_params(format!("Invalid range: {}", e)))?,
                    _ => LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 0)),
                };
                let context: CodeActionContext = match params.get("context") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| RpcError::invalid_params(format!("Invalid codeAction context: {}", e)))?,
                    _ => CodeActionContext::default(),
                };

                let actions = self
                    .request_code_actions(uri, language_id, range, context)
                    .await;
                serde_json::to_value(actions).map_err(|e| RpcError::internal(e.to_string()))
            }
            _ => Err(RpcError::method_not_found(method)),
        }
    }

    /// Handle incoming RPC calls from JS runtime (e.g. provider registration, format, codeAction).
    /// Returns `Some(result)` on success or `Some({ "error": { "code", "message" } })` on failure.
    pub async fn handle_rpc_message(
        &self,
        method: &str,
        params: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        match self.handle_rpc_message_result(method, params).await {
            Ok(val) => Some(val),
            Err(err) => Some(json!({ "error": err })),
        }
    }

    // --- Document Lifecycle Forwarding ---

    fn refresh_editor_snapshots(&self, snap: &OpenDocumentSnapshot) {
        if let Some(mut active) = self.window_manager.active_editor() {
            if active.document.uri == snap.uri
                || (!snap.is_untitled
                    && !active.document.is_untitled
                    && rustcode_platform::paths_equivalent(
                        Path::new(&active.document.file_name),
                        Path::new(&snap.file_name),
                    ))
            {
                active.document = snap.clone();
                self.window_manager.set_active_editor(Some(active));
            }
        }
        let mut visible = self.window_manager.visible_editors();
        let mut changed = false;
        for ed in &mut visible {
            if ed.document.uri == snap.uri
                || (!snap.is_untitled
                    && !ed.document.is_untitled
                    && rustcode_platform::paths_equivalent(
                        Path::new(&ed.document.file_name),
                        Path::new(&snap.file_name),
                    ))
            {
                ed.document = snap.clone();
                changed = true;
            }
        }
        if changed {
            self.window_manager.set_visible_editors(visible);
        }
    }

    fn did_open_document_local(&self, uri: &str, language_id: &str, version: i32, text: &str) {
        let is_untitled = uri.starts_with("untitled:");
        let file_name = if is_untitled {
            strip_untitled_prefix(uri)
                .unwrap_or("Untitled-1")
                .to_string()
        } else {
            decode_file_uri(uri).to_string_lossy().into_owned()
        };
        let snap = {
            let mut docs = self.open_documents.write();
            let key = find_open_document_key(&docs, uri).unwrap_or_else(|| uri.to_string());
            let snap = OpenDocumentSnapshot::from_parts(
                uri.to_string(),
                file_name,
                language_id.to_string(),
                version,
                text.to_string(),
                false,
                is_untitled,
            );
            docs.insert(key, snap.clone());
            snap
        };
        self.rpc_bridge
            .notify("workspace.onDidOpenTextDocument", snap.to_json());
    }

    fn did_change_document_local(&self, uri: &str, language_id: &str, version: i32, text: &str) {
        let snap = {
            let mut docs = self.open_documents.write();
            let existing_key = find_open_document_key(&docs, uri);
            if let Some(existing) = existing_key.as_deref().and_then(|k| docs.get_mut(k)) {
                existing.language_id = language_id.to_string();
                existing.version = version;
                existing.text = text.to_string();
                existing.is_dirty = true;
                existing.recompute_metrics();
                existing.clone()
            } else {
                let is_untitled = uri.starts_with("untitled:");
                let file_name = if is_untitled {
                    strip_untitled_prefix(uri)
                        .unwrap_or("Untitled-1")
                        .to_string()
                } else {
                    decode_file_uri(uri).to_string_lossy().into_owned()
                };
                let snap = OpenDocumentSnapshot::from_parts(
                    uri.to_string(),
                    file_name,
                    language_id.to_string(),
                    version,
                    text.to_string(),
                    true,
                    is_untitled,
                );
                docs.insert(uri.to_string(), snap.clone());
                snap
            }
        };
        self.refresh_editor_snapshots(&snap);
        self.rpc_bridge.notify(
            "workspace.onDidChangeTextDocument",
            json!({
                "document": snap.to_json(),
                "contentChanges": [{ "text": text }],
                "reason": Value::Null,
            }),
        );
    }

    fn did_save_document_local(&self, uri: &str) {
        let saved_snap = {
            let mut docs = self.open_documents.write();
            let key = find_open_document_key(&docs, uri);
            if let Some(existing) = key.as_deref().and_then(|k| docs.get_mut(k)) {
                existing.is_dirty = false;
                Some(existing.clone())
            } else {
                None
            }
        };
        if let Some(snap) = saved_snap {
            self.refresh_editor_snapshots(&snap);
            self.rpc_bridge
                .notify("workspace.onDidSaveTextDocument", snap.to_json());
        } else {
            let path = decode_file_uri(uri);
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let snap = OpenDocumentSnapshot::from_parts(
                uri.to_string(),
                path.to_string_lossy().into_owned(),
                rustcode_base::map_ext_to_lang_id(ext),
                1,
                String::new(),
                false,
                uri.starts_with("untitled:"),
            );
            self.rpc_bridge
                .notify("workspace.onDidSaveTextDocument", snap.to_json());
        }
    }

    fn did_close_document_local(&self, uri: &str, language_id: &str) {
        let removed = {
            let mut docs = self.open_documents.write();
            let key = find_open_document_key(&docs, uri);
            key.and_then(|k| docs.remove(&k))
        };
        let target_path = decode_file_uri(uri);
        let mut visible = self.window_manager.visible_editors();
        let prev_len = visible.len();
        visible.retain(|e| {
            e.document.uri != uri
                && (uri.starts_with("untitled:")
                    || e.document.is_untitled
                    || !rustcode_platform::paths_equivalent(
                        Path::new(&e.document.file_name),
                        &target_path,
                    ))
        });
        if visible.len() != prev_len {
            self.window_manager.set_visible_editors(visible);
        }
        if let Some(snap) = removed {
            let mut payload = snap.to_json();
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("isClosed".to_string(), json!(true));
            }
            self.rpc_bridge
                .notify("workspace.onDidCloseTextDocument", payload);
        } else {
            self.rpc_bridge.notify(
                "workspace.onDidCloseTextDocument",
                json!({
                    "uri": uri,
                    "languageId": language_id,
                    "isClosed": true,
                }),
            );
        }
    }

    pub fn did_open_document_sync(&self, uri: &str, language_id: &str, version: i32, text: &str) {
        self.did_open_document_local(uri, language_id, version, text);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let lsp = Arc::clone(&self.lsp_manager);
            let uri_owned = uri.to_string();
            let lang_owned = language_id.to_string();
            let text_owned = text.to_string();
            handle.spawn(async move {
                let _ = lsp
                    .did_open_document(&uri_owned, &lang_owned, version, &text_owned)
                    .await;
            });
        }
    }

    pub fn did_change_document_sync(&self, uri: &str, language_id: &str, version: i32, text: &str) {
        self.did_change_document_local(uri, language_id, version, text);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let lsp = Arc::clone(&self.lsp_manager);
            let uri_owned = uri.to_string();
            let lang_owned = language_id.to_string();
            let text_owned = text.to_string();
            handle.spawn(async move {
                let _ = lsp
                    .did_change_document(&uri_owned, &lang_owned, version, &text_owned)
                    .await;
            });
        }
    }

    pub fn did_save_document_sync(&self, uri: &str) {
        self.did_save_document_local(uri);
    }

    pub fn did_close_document_sync(&self, uri: &str, language_id: &str) {
        self.did_close_document_local(uri, language_id);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let lsp = Arc::clone(&self.lsp_manager);
            let uri_owned = uri.to_string();
            let lang_owned = language_id.to_string();
            handle.spawn(async move {
                let _ = lsp.did_close_document(&uri_owned, &lang_owned).await;
            });
        }
    }

    pub async fn did_open_document(&self, uri: &str, language_id: &str, version: i32, text: &str) {
        self.did_open_document_local(uri, language_id, version, text);
        let _ = self
            .lsp_manager
            .did_open_document(uri, language_id, version, text)
            .await;
    }

    pub async fn did_change_document(&self, uri: &str, language_id: &str, version: i32, text: &str) {
        self.did_change_document_local(uri, language_id, version, text);
        let _ = self
            .lsp_manager
            .did_change_document(uri, language_id, version, text)
            .await;
    }

    pub async fn did_save_document(&self, uri: &str) {
        self.did_save_document_local(uri);
    }

    pub async fn did_close_document(&self, uri: &str, language_id: &str) {
        self.did_close_document_local(uri, language_id);
        let _ = self.lsp_manager.did_close_document(uri, language_id).await;
    }

    async fn handle_show_text_document_rpc(&self, params: &Value) -> Result<Value, RpcError> {
        let doc_val = self.open_or_resolve_text_document(params).await?;
        let doc: OpenDocumentSnapshot = serde_json::from_value(doc_val)
            .map_err(|e| RpcError::internal(format!("Invalid document snapshot: {}", e)))?;

        let options = params.get("options").cloned().unwrap_or_else(|| json!({}));
        let view_column = params
            .get("viewColumn")
            .and_then(|v| v.as_i64())
            .or_else(|| options.get("viewColumn").and_then(|v| v.as_i64()))
            .map(|v| v as i32)
            .or(Some(1));
        let preserve_focus = params
            .get("preserveFocus")
            .and_then(|v| v.as_bool())
            .or_else(|| options.get("preserveFocus").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        let preview = options.get("preview").and_then(|v| v.as_bool());
        let selection_range: Option<LspRange> = options
            .get("selection")
            .and_then(|v| serde_json::from_value::<LspRange>(v.clone()).ok());
        let resolved_range = selection_range
            .unwrap_or_else(|| LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 0)));
        let sel = crate::window::ExtSelection::from_range(resolved_range);

        let editor_id = format!(
            "editor_{}",
            self.next_editor_id.fetch_add(1, Ordering::SeqCst)
        );
        let editor = crate::window::ExtTextEditorState {
            id: editor_id,
            document: doc.clone(),
            selection: sel.clone(),
            selections: vec![sel],
            visible_ranges: vec![LspRange::new(
                LspPosition::new(0, 0),
                LspPosition::new(doc.line_count.saturating_sub(1) as u32, 0),
            )],
            view_column,
            options: options.clone(),
        };

        self.window_manager.push_show_document_request(
            crate::window::ShowTextDocumentRequest {
                uri: doc.uri.clone(),
                view_column,
                preserve_focus,
                preview,
                selection: selection_range,
            },
        );

        let mut visible = self.window_manager.visible_editors();
        if let Some(slot) = visible.iter_mut().find(|e| e.document.uri == doc.uri) {
            *slot = editor.clone();
        } else {
            visible.push(editor.clone());
        }
        self.set_visible_editors(visible);
        if !preserve_focus || self.window_manager.active_editor().is_none() {
            self.set_active_editor(Some(editor.clone()));
        }

        serde_json::to_value(editor).map_err(|e| RpcError::internal(e.to_string()))
    }

    // --- Workspace, Commands, and Node-Compat FS RPC Handlers ---

    async fn handle_workspace_rpc(&self, method: &str, params: &Value) -> Result<Value, RpcError> {
        match method {
            "workspace.getConfiguration" => {
                let section = match params.get("section") {
                    None | Some(Value::Null) => "",
                    Some(Value::String(s)) => s.as_str(),
                    Some(_) => {
                        return Err(RpcError::invalid_params(
                            "Invalid 'section' parameter: expected string",
                        ))
                    }
                };
                let scope_path = extract_scope_path(params);
                let merged = if scope_path.is_some() {
                    self.configuration_service
                        .get_merged_for_resource(scope_path.as_deref())
                } else {
                    self.configuration_service.get_all()
                };
                if section.is_empty() {
                    Ok(merged)
                } else {
                    Ok(traverse_config_value(&merged, section).unwrap_or_else(|| json!({})))
                }
            }
            "workspace.inspectConfiguration" => {
                let key = params
                    .get("key")
                    .or_else(|| params.get("section"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        RpcError::invalid_params("Missing string 'key' or 'section' parameter")
                    })?;
                let scope_path = extract_scope_path(params);
                let inspection = self
                    .configuration_service
                    .inspect(key, scope_path.as_deref());
                serde_json::to_value(inspection).map_err(|e| RpcError::internal(e.to_string()))
            }
            "workspace.updateConfiguration" => {
                let key = params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'key' parameter"))?
                    .trim();
                if key.is_empty() {
                    return Err(RpcError::invalid_params("Configuration 'key' must not be empty"));
                }
                let scope_path = extract_scope_path(params);
                let target = match params.get("target") {
                    None | Some(Value::Null) => ConfigurationTarget::User,
                    Some(Value::Number(n)) => match n.as_i64() {
                        Some(1) => ConfigurationTarget::User,
                        Some(2) => ConfigurationTarget::Workspace,
                        Some(3) => scope_path
                            .map(ConfigurationTarget::WorkspaceFolder)
                            .unwrap_or(ConfigurationTarget::Workspace),
                        other => {
                            return Err(RpcError::invalid_params(format!(
                                "Invalid numeric configuration target: {:?}",
                                other
                            )))
                        }
                    },
                    Some(Value::Bool(b)) => {
                        if *b {
                            ConfigurationTarget::User
                        } else {
                            ConfigurationTarget::Workspace
                        }
                    }
                    Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
                        "user" | "global" => ConfigurationTarget::User,
                        "workspace" => ConfigurationTarget::Workspace,
                        "workspacefolder" | "folder" => scope_path
                            .map(ConfigurationTarget::WorkspaceFolder)
                            .unwrap_or(ConfigurationTarget::Workspace),
                        other => {
                            return Err(RpcError::invalid_params(format!(
                                "Invalid configuration target '{}'",
                                other
                            )))
                        }
                    },
                    Some(_) => {
                        return Err(RpcError::invalid_params(
                            "Invalid 'target' parameter type",
                        ))
                    }
                };

                match params.get("value") {
                    None => {
                        self.configuration_service.reset(key, target);
                        Ok(json!({ "updated": true, "reset": true, "key": key }))
                    }
                    Some(val) => {
                        self.configuration_service
                            .update(key, val.clone(), target)
                            .map_err(RpcError::invalid_params)?;
                        Ok(json!({ "updated": true, "key": key }))
                    }
                }
            }
            "workspace.getWorkspaceFolders" => {
                Ok(serde_json::to_value(&*self.workspace_folders.read())
                    .unwrap_or_else(|_| json!([])))
            }
            "workspace.getTextDocuments" => {
                let docs: Vec<Value> = self
                    .open_documents
                    .read()
                    .values()
                    .map(OpenDocumentSnapshot::to_json)
                    .collect();
                Ok(Value::Array(docs))
            }
            "workspace.openTextDocument" => self.open_or_resolve_text_document(params).await,
            "workspace.saveTextDocument" => {
                let raw_uri = params
                    .get("uri")
                    .or_else(|| params.get("fileName"))
                    .or_else(|| params.get("path"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        RpcError::invalid_params("Missing string 'uri' parameter")
                    })?;
                let saved_snap = {
                    let docs = self.open_documents.read();
                    find_open_document_key(&docs, raw_uri)
                        .and_then(|k| docs.get(&k).cloned().map(|s| (k, s)))
                };
                let Some((key, mut snap)) = saved_snap else {
                    return Ok(json!(false));
                };
                if snap.is_untitled {
                    return Ok(json!(false));
                }
                let target_path = if self.fs_root.is_some() {
                    self.resolve_fs_path(&snap.file_name)
                        .map_err(RpcError::from_prefixed_string)?
                } else {
                    PathBuf::from(&snap.file_name)
                };
                self.file_system_service
                    .write_atomic(&target_path, &snap.text)
                    .await
                    .map_err(|e| RpcError::from_prefixed_string(map_fs_error(e)))?;
                snap.is_dirty = false;
                {
                    let mut docs = self.open_documents.write();
                    if let Some(entry) = docs.get_mut(&key) {
                        entry.is_dirty = false;
                    }
                }
                self.rpc_bridge
                    .notify("workspace.onDidSaveTextDocument", snap.to_json());
                Ok(json!(true))
            }
            "workspace.saveAll" => {
                let targets: Vec<(String, OpenDocumentSnapshot)> = {
                    self.open_documents
                        .read()
                        .iter()
                        .filter(|(_, s)| !s.is_untitled && s.is_dirty)
                        .map(|(k, s)| (k.clone(), s.clone()))
                        .collect()
                };
                let mut all_ok = true;
                for (key, mut snap) in targets {
                    let target_path = if self.fs_root.is_some() {
                        match self.resolve_fs_path(&snap.file_name) {
                            Ok(p) => p,
                            Err(_) => {
                                all_ok = false;
                                continue;
                            }
                        }
                    } else {
                        PathBuf::from(&snap.file_name)
                    };
                    if self
                        .file_system_service
                        .write_atomic(&target_path, &snap.text)
                        .await
                        .is_ok()
                    {
                        snap.is_dirty = false;
                        if let Some(entry) = self.open_documents.write().get_mut(&key) {
                            entry.is_dirty = false;
                        }
                        self.rpc_bridge
                            .notify("workspace.onDidSaveTextDocument", snap.to_json());
                    } else {
                        all_ok = false;
                    }
                }
                Ok(json!(all_ok))
            }
            "workspace.applyEdit" => self.handle_apply_edit_rpc(params).await,
            "workspace.findFiles" => self.handle_find_files_rpc(params),
            "workspace.registerFileSystemProvider"
            | "workspace.unregisterFileSystemProvider"
            | "workspace.registerTextDocumentContentProvider"
            | "workspace.unregisterTextDocumentContentProvider"
            | "workspace.onDidChangeFileSystem"
            | "workspace.onDidChangeTextDocumentContent" => {
                let res = self
                    .vfs_manager
                    .handle_rpc_call(method, params)
                    .map_err(|e| map_manager_error(method, e))?;
                if method == "workspace.registerFileSystemProvider" {
                    if let Some(scheme) = res.get("scheme").and_then(|v| v.as_str()) {
                        self.provider_registry.register(super::ProviderRegistration {
                            id: format!("vfs:{}", scheme),
                            provider_type: ProviderType::FileSystem,
                            language_selector: Some(scheme.to_string()),
                        });
                    }
                } else if method == "workspace.unregisterFileSystemProvider" {
                    if let Some(scheme) = res.get("scheme").and_then(|v| v.as_str()) {
                        self.provider_registry.unregister(&format!("vfs:{}", scheme.to_ascii_lowercase()));
                    }
                } else if method == "workspace.registerTextDocumentContentProvider" {
                    if let Some(scheme) = res.get("scheme").and_then(|v| v.as_str()) {
                        self.provider_registry.register(super::ProviderRegistration {
                            id: format!("textContent:{}", scheme),
                            provider_type: ProviderType::TextDocumentContent,
                            language_selector: Some(scheme.to_string()),
                        });
                    }
                } else if method == "workspace.unregisterTextDocumentContentProvider" {
                    if let Some(scheme) = res.get("scheme").and_then(|v| v.as_str()) {
                        self.provider_registry.unregister(&format!(
                            "textContent:{}",
                            scheme.to_ascii_lowercase()
                        ));
                    }
                }
                Ok(res)
            }
            "workspace.fs.readFile" => {
                let uri = params
                    .get("uri")
                    .or_else(|| params.get("path"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'uri' parameter"))?;
                if self.vfs_manager.has_file_system_scheme(uri) {
                    let bytes = self
                        .vfs_manager
                        .request_read_file(&self.rpc_bridge, uri)
                        .await
                        .map_err(|e| RpcError::new("VFS_ERROR", e))?;
                    Ok(json!(bytes))
                } else {
                    let path_str = decode_file_uri(uri).to_string_lossy().into_owned();
                    self.handle_fs_rpc("fs.readFile", &json!({ "path": path_str, "encoding": Value::Null }))
                        .await
                        .map_err(RpcError::from_prefixed_string)
                }
            }
            "workspace.fs.stat" => {
                let uri = params
                    .get("uri")
                    .or_else(|| params.get("path"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'uri' parameter"))?;
                if self.vfs_manager.has_file_system_scheme(uri) {
                    self.vfs_manager
                        .request_stat(&self.rpc_bridge, uri)
                        .await
                        .map_err(|e| RpcError::new("VFS_ERROR", e))
                } else {
                    let path_str = decode_file_uri(uri).to_string_lossy().into_owned();
                    self.handle_fs_rpc("fs.stat", &json!({ "path": path_str }))
                        .await
                        .map_err(RpcError::from_prefixed_string)
                }
            }
            "workspace.fs.readDirectory" => {
                let uri = params
                    .get("uri")
                    .or_else(|| params.get("path"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'uri' parameter"))?;
                if self.vfs_manager.has_file_system_scheme(uri) {
                    self.vfs_manager
                        .request_read_directory(&self.rpc_bridge, uri)
                        .await
                        .map_err(|e| RpcError::new("VFS_ERROR", e))
                } else {
                    let path_str = decode_file_uri(uri).to_string_lossy().into_owned();
                    self.handle_fs_rpc(
                        "fs.readdir",
                        &json!({ "path": path_str, "withFileTypes": true }),
                    )
                    .await
                    .map_err(RpcError::from_prefixed_string)
                }
            }
            "workspace.fs.writeFile" => {
                let uri = params
                    .get("uri")
                    .or_else(|| params.get("path"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'uri' parameter"))?;
                if self.vfs_manager.has_file_system_scheme(uri) {
                    let bytes: Vec<u8> = params
                        .get("content")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    let create = params
                        .get("options")
                        .and_then(|o| o.get("create"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let overwrite = params
                        .get("options")
                        .and_then(|o| o.get("overwrite"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    self.vfs_manager
                        .request_write_file(&self.rpc_bridge, uri, &bytes, create, overwrite)
                        .await
                        .map_err(|e| RpcError::new("VFS_ERROR", e))
                } else {
                    let path_str = decode_file_uri(uri).to_string_lossy().into_owned();
                    let data = params.get("content").cloned().unwrap_or(json!([]));
                    self.handle_fs_rpc("fs.writeFile", &json!({ "path": path_str, "data": data }))
                        .await
                        .map_err(RpcError::from_prefixed_string)
                }
            }
            _ => Err(RpcError::method_not_found(method)),
        }
    }

    async fn open_or_resolve_text_document(&self, params: &Value) -> Result<Value, RpcError> {
        let raw_uri_val = params
            .get("uri")
            .or_else(|| params.get("fileName"))
            .or_else(|| params.get("path"));
        let raw_uri_opt = match raw_uri_val {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.as_str()),
            Some(_) => {
                return Err(RpcError::invalid_params(
                    "Invalid 'uri' parameter: expected string",
                ))
            }
        };

        if raw_uri_opt.is_none() || raw_uri_opt.is_some_and(|u| u.starts_with("untitled:")) {
            let open_snap = raw_uri_opt.and_then(|raw_u| {
                let docs = self.open_documents.read();
                find_open_document_key(&docs, raw_u).and_then(|k| docs.get(&k).cloned())
            });

            if let Some(snap) = open_snap {
                let mut out = snap.to_json();
                if let Some(obj) = out.as_object_mut() {
                    let clean_name = strip_untitled_prefix(&snap.uri)
                        .unwrap_or(&snap.file_name)
                        .to_string();
                    obj.insert("fileName".to_string(), json!(clean_name));
                    obj.insert("isUntitled".to_string(), json!(true));
                }
                return Ok(out);
            }

            let uri = match raw_uri_opt {
                Some(u) => u.to_string(),
                None => loop {
                    let n = self.next_untitled_id.fetch_add(1, Ordering::SeqCst);
                    let candidate = format!("untitled:Untitled-{}", n);
                    let docs = self.open_documents.read();
                    if find_open_document_key(&docs, &candidate).is_none() {
                        break candidate;
                    }
                },
            };
            let file_name = strip_untitled_prefix(&uri)
                .unwrap_or("Untitled-1")
                .to_string();
            let language_id = params
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("plaintext")
                .to_string();
            let text = params
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let snap = OpenDocumentSnapshot::from_parts(
                uri.clone(),
                file_name,
                language_id,
                1,
                text,
                false,
                true,
            );
            self.open_documents.write().insert(uri, snap.clone());
            self.rpc_bridge
                .notify("workspace.onDidOpenTextDocument", snap.to_json());
            return Ok(snap.to_json());
        }

        let raw = raw_uri_opt.ok_or_else(|| {
            RpcError::invalid_params("Missing 'uri', 'fileName', or 'path'")
        })?;
        if self.vfs_manager.has_text_content_scheme(raw) {
            let text = self
                .vfs_manager
                .request_provide_text_document_content(&self.rpc_bridge, raw)
                .await
                .map_err(|e| RpcError::new("VFS_CONTENT_ERROR", e))?;
            let snap = OpenDocumentSnapshot::from_parts(
                raw.to_string(),
                raw.to_string(),
                "plaintext".to_string(),
                1,
                text,
                false,
                false,
            );
            self.open_documents
                .write()
                .insert(raw.to_string(), snap.clone());
            self.rpc_bridge
                .notify("workspace.onDidOpenTextDocument", snap.to_json());
            return Ok(snap.to_json());
        }
        if self.vfs_manager.has_file_system_scheme(raw) {
            let bytes = self
                .vfs_manager
                .request_read_file(&self.rpc_bridge, raw)
                .await
                .map_err(|e| RpcError::new("VFS_READ_ERROR", e))?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let ext = Path::new(raw)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let snap = OpenDocumentSnapshot::from_parts(
                raw.to_string(),
                raw.to_string(),
                rustcode_base::map_ext_to_lang_id(ext),
                1,
                text,
                false,
                false,
            );
            self.open_documents
                .write()
                .insert(raw.to_string(), snap.clone());
            self.rpc_bridge
                .notify("workspace.onDidOpenTextDocument", snap.to_json());
            return Ok(snap.to_json());
        }
        let path = decode_file_uri(raw);
        let normalized_uri = rustcode_platform::path_to_file_uri(&path);
        let canonical_uri = if rustcode_platform::is_file_uri(raw) {
            raw.to_string()
        } else {
            normalized_uri.clone()
        };

        let open_snap = {
            let docs = self.open_documents.read();
            find_open_document_key(&docs, raw).and_then(|k| docs.get(&k).cloned())
        };

        if let Some(snap) = open_snap {
            return Ok(snap.to_json());
        }

        let text = FileSystemService::read_to_string(&path)
            .await
            .map_err(|e| RpcError::from_prefixed_string(map_fs_error(e)))?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let language_id = rustcode_base::map_ext_to_lang_id(ext);
        let snap = OpenDocumentSnapshot::from_parts(
            canonical_uri.clone(),
            path.to_string_lossy().into_owned(),
            language_id,
            1,
            text,
            false,
            false,
        );
        self.open_documents
            .write()
            .insert(canonical_uri, snap.clone());
        self.rpc_bridge
            .notify("workspace.onDidOpenTextDocument", snap.to_json());
        Ok(snap.to_json())
    }

    async fn handle_apply_edit_rpc(&self, params: &Value) -> Result<Value, RpcError> {
        let edit_val = params.get("edit").unwrap_or(params);
        let changes_obj = edit_val
            .get("changes")
            .and_then(|v| v.as_object())
            .ok_or_else(|| RpcError::invalid_params("Missing 'edit.changes' object"))?;

        for (uri_str, edits_val) in changes_obj {
            let edits: Vec<LspTextEdit> = serde_json::from_value(edits_val.clone())
                .map_err(|e| RpcError::invalid_params(format!("Invalid text edits: {}", e)))?;
            if edits.is_empty() {
                continue;
            }

            let existing_opt = {
                let docs = self.open_documents.read();
                find_open_document_key(&docs, uri_str)
                    .and_then(|k| docs.get(&k).cloned().map(|s| (k, s)))
            };

            let (doc_key, mut snap) = if let Some((k, s)) = existing_opt {
                (k, s)
            } else if uri_str.starts_with("untitled:") {
                let name = strip_untitled_prefix(uri_str)
                    .unwrap_or("Untitled-1")
                    .to_string();
                let s = OpenDocumentSnapshot::from_parts(
                    uri_str.clone(),
                    name,
                    "plaintext".to_string(),
                    1,
                    String::new(),
                    false,
                    true,
                );
                (uri_str.clone(), s)
            } else {
                let path = decode_file_uri(uri_str);
                let text = FileSystemService::read_to_string(&path)
                    .await
                    .map_err(|e| RpcError::from_prefixed_string(map_fs_error(e)))?;
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                let s = OpenDocumentSnapshot::from_parts(
                    uri_str.clone(),
                    path.to_string_lossy().into_owned(),
                    rustcode_base::map_ext_to_lang_id(ext),
                    1,
                    text,
                    false,
                    false,
                );
                (uri_str.clone(), s)
            };

            snap.text = apply_text_edits_to_str(&snap.text, &edits);
            snap.version += 1;
            snap.is_dirty = true;
            snap.recompute_metrics();
            self.open_documents
                .write()
                .insert(doc_key, snap.clone());

            self.rpc_bridge.notify(
                "workspace.onDidChangeTextDocument",
                json!({
                    "document": snap.to_json(),
                    "contentChanges": [{ "text": snap.text }],
                    "reason": Value::Null,
                }),
            );
        }

        self.pending_workspace_edits.lock().push(edit_val.clone());
        Ok(json!(true))
    }

    fn handle_find_files_rpc(&self, params: &Value) -> Result<Value, RpcError> {
        let include_val = params
            .get("include")
            .ok_or_else(|| RpcError::invalid_params("Missing 'include' parameter"))?;

        let (include_pattern, explicit_base): (String, Option<PathBuf>) = match include_val {
            Value::String(s) => {
                if s.trim().is_empty() {
                    return Err(RpcError::invalid_params(
                        "Invalid 'include' parameter: must not be empty",
                    ));
                }
                (s.clone(), None)
            }
            Value::Object(obj) => {
                let pat = obj
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        RpcError::invalid_params("Missing string 'include.pattern' parameter")
                    })?
                    .to_string();
                let base_str = obj
                    .get("base")
                    .and_then(|v| v.as_str())
                    .or_else(|| obj.get("baseUri").and_then(|v| v.get("fsPath")).and_then(|v| v.as_str()))
                    .or_else(|| obj.get("baseUri").and_then(|v| v.get("path")).and_then(|v| v.as_str()));
                let base = base_str
                    .filter(|s| !s.trim().is_empty())
                    .map(decode_file_uri);
                (pat, base)
            }
            _ => {
                return Err(RpcError::invalid_params(
                    "Invalid 'include' parameter: expected string or RelativePattern object",
                ))
            }
        };

        let (exclude_pattern, use_default_excludes): (Option<String>, bool) =
            match params.get("exclude") {
                None => (None, true),
                Some(Value::Null) => (None, false),
                Some(Value::String(s)) => (Some(s.clone()), false),
                Some(Value::Object(obj)) => {
                    let pat = obj
                        .get("pattern")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string);
                    (pat, false)
                }
                Some(_) => {
                    return Err(RpcError::invalid_params(
                        "Invalid 'exclude' parameter",
                    ))
                }
            };

        let max_results = params
            .get("maxResults")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        if max_results == Some(0) {
            return Ok(json!([]));
        }

        let roots: Vec<PathBuf> = if let Some(base) = explicit_base {
            vec![base]
        } else {
            let folders = self.workspace_folders.read();
            if !folders.is_empty() {
                folders.iter().map(|f| PathBuf::from(&f.path)).collect()
            } else if let Some(root) = &self.fs_root {
                vec![root.clone()]
            } else {
                Vec::new()
            }
        };

        let mut results: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for root in roots {
            if !root.exists() {
                continue;
            }
            let mut files = Vec::new();
            collect_workspace_files(&root, &root, use_default_excludes, &mut files);
            for (rel, abs) in files {
                if !glob_matches_relative_path(&include_pattern, &rel) {
                    continue;
                }
                if let Some(ref excl) = exclude_pattern {
                    if !excl.is_empty() && glob_matches_relative_path(excl, &rel) {
                        continue;
                    }
                }
                let uri = rustcode_platform::path_to_file_uri(&abs);
                if seen.insert(uri.clone()) {
                    results.push(uri);
                    if max_results.is_some_and(|limit| results.len() >= limit) {
                        return Ok(json!(results));
                    }
                }
            }
        }

        Ok(json!(results))
    }

    fn handle_commands_rpc(&self, method: &str, params: &Value) -> Result<Value, RpcError> {
        match method {
            "commands.registerCommand" => {
                let command = params
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'command' parameter"))?
                    .trim();
                if command.is_empty() {
                    return Err(RpcError::invalid_params("Command name must not be empty"));
                }
                let cmd_id = command.to_string();

                let _mutation_guard = self.command_mutation_lock.lock();

                // Drop any prior disposable BEFORE registering the new callback because
                // ActionDisposable::drop removes `cmd_id` from CommandRegistry.
                let old_disp = self.command_disposables.lock().remove(&cmd_id);
                if let Some(mut old) = old_disp {
                    old.dispose();
                }

                let rpc = Arc::clone(&self.rpc_bridge);
                let cb_cmd = cmd_id.clone();
                let new_disp = self.command_registry.register(&cmd_id, move |args| {
                    let args_arr = match args {
                        None | Some(Value::Null) => Vec::new(),
                        Some(Value::Array(arr)) => arr,
                        Some(other) => vec![other],
                    };
                    let notif = rpc.create_notification(
                        "commands.executeCommand",
                        json!({ "command": cb_cmd, "args": args_arr }),
                    );
                    rpc.push_notification(notif);
                    Ok(json!({
                        "dispatched": true,
                        "command": cb_cmd,
                        "args": args_arr,
                    }))
                });

                self.command_disposables
                    .lock()
                    .insert(cmd_id.clone(), new_disp);
                self.js_commands.write().insert(cmd_id.clone());

                Ok(json!({ "registered": true, "command": cmd_id }))
            }
            "commands.unregisterCommand" => {
                let command = params
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'command' parameter"))?
                    .trim();

                let _mutation_guard = self.command_mutation_lock.lock();
                let was_js = self.js_commands.write().remove(command);
                let old_disp = self.command_disposables.lock().remove(command);
                let had_disp = old_disp.is_some();
                if let Some(mut disp) = old_disp {
                    disp.dispose();
                }
                Ok(json!({
                    "unregistered": was_js || had_disp,
                    "command": command,
                }))
            }
            "commands.executeCommand" => {
                let command = params
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RpcError::invalid_params("Missing string 'command' parameter"))?
                    .trim();
                if command.is_empty() {
                    return Err(RpcError::invalid_params("Command name must not be empty"));
                }
                // Infinite recursion guard: never bounce JS-owned commands back to JS.
                {
                    let _mutation_guard = self.command_mutation_lock.lock();
                    if self.js_commands.read().contains(command) {
                        return Err(RpcError::new(
                            "COMMAND_RECURSION",
                            format!(
                                "Refusing to execute JS-owned command '{}' from extension host RPC",
                                command
                            ),
                        ));
                    }
                }
                let arg_val = match params.get("args") {
                    None | Some(Value::Null) => None,
                    Some(Value::Array(arr)) if arr.is_empty() => None,
                    Some(other) => Some(other.clone()),
                };
                self.command_registry.execute(command, arg_val).map_err(|e| {
                    if e.contains("not found") {
                        RpcError::new("COMMAND_NOT_FOUND", e)
                    } else {
                        RpcError::new("COMMAND_EXECUTION_FAILED", e)
                    }
                })
            }
            "commands.getCommands" => {
                let filter_internal = params
                    .get("filterInternal")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let mut cmds = {
                    let _mutation_guard = self.command_mutation_lock.lock();
                    self.command_registry.list_commands()
                };
                if filter_internal {
                    cmds.retain(|c| !c.starts_with('_'));
                }
                cmds.sort();
                Ok(json!(cmds))
            }
            _ => Err(RpcError::method_not_found(method)),
        }
    }

    fn require_fs_path_param(&self, params: &Value, key: &str) -> Result<PathBuf, String> {
        let raw_path = params
            .get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("EINVAL: Missing string '{}' parameter", key))?;
        self.resolve_fs_path(raw_path)
    }

    pub async fn handle_fs_rpc(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "fs.readFile" => {
                let path = self.require_fs_path_param(params, "path")?;
                if let Ok(meta) = std::fs::metadata(&path) {
                    if meta.is_dir() {
                        return Err(format!(
                            "EISDIR: illegal operation on a directory, read '{}'",
                            path.display()
                        ));
                    }
                }
                let enc_val = params.get("encoding");
                let is_utf8 = match enc_val {
                    None => true,
                    Some(Value::Null) => false,
                    Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
                        "utf8" | "utf-8" => true,
                        "binary" | "buffer" | "bytes" => false,
                        other => return Err(format!("EINVAL: Unsupported encoding '{}'", other)),
                    },
                    Some(_) => return Err("EINVAL: Invalid encoding parameter".to_string()),
                };

                if is_utf8 {
                    let text = FileSystemService::read_to_string(&path)
                        .await
                        .map_err(map_fs_error)?;
                    Ok(Value::String(text))
                } else {
                    let bytes = std::fs::read(&path).map_err(map_io_error)?;
                    Ok(json!(bytes))
                }
            }
            "fs.writeFile" => {
                let path = self.require_fs_path_param(params, "path")?;
                let data = params
                    .get("data")
                    .ok_or_else(|| "EINVAL: Missing 'data' parameter".to_string())?;
                let p_str = path.to_string_lossy().into_owned();
                match data {
                    Value::String(s) => {
                        let meta = self
                            .file_system_service
                            .write_atomic(&path, s)
                            .await
                            .map_err(map_fs_error)?;
                        Ok(json!({ "written": true, "path": p_str, "size": meta.size }))
                    }
                    Value::Array(arr) => {
                        let mut bytes = Vec::with_capacity(arr.len());
                        for item in arr {
                            let b = item
                                .as_u64()
                                .and_then(|n| u8::try_from(n).ok())
                                .ok_or_else(|| "EINVAL: Invalid byte in data array".to_string())?;
                            bytes.push(b);
                        }
                        if self.fs_root.is_some()
                            && std::fs::symlink_metadata(&path)
                                .is_ok_and(|m| is_symlink_or_reparse_point(&path, &m))
                        {
                            return Err(format!(
                                "EACCES: Refusing to write through symlink '{}'",
                                path.display()
                            ));
                        }
                        let parent = match path.parent() {
                            Some(p) if !p.as_os_str().is_empty() => p,
                            _ => Path::new("."),
                        };
                        if parent != Path::new(".") {
                            std::fs::create_dir_all(parent).map_err(map_io_error)?;
                        }
                        let file_stem = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("blob");
                        let seq = TEMP_FILE_COUNTER
                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let temp_path = parent.join(format!(
                            ".{}.tmp.{}.{}",
                            file_stem,
                            std::process::id(),
                            seq
                        ));
                        {
                            use std::io::Write;
                            let write_res = (|| -> Result<(), std::io::Error> {
                                let mut file = std::fs::OpenOptions::new()
                                    .write(true)
                                    .create_new(true)
                                    .open(&temp_path)?;
                                file.write_all(&bytes)?;
                                file.flush()?;
                                file.sync_all()?;
                                Ok(())
                            })();
                            if let Err(e) = write_res {
                                let _ = std::fs::remove_file(&temp_path);
                                return Err(map_io_error(e));
                            }
                        }
                        if let Err(e) = std::fs::rename(&temp_path, &path) {
                            let _ = std::fs::remove_file(&temp_path);
                            return Err(map_io_error(e));
                        }
                        self.file_system_service.on_did_change.fire(FileChangeEvent {
                            path: path.clone(),
                            change_type: FileChangeType::Changed,
                        });
                        Ok(json!({ "written": true, "path": p_str, "size": bytes.len() }))
                    }
                    _ => Err("EINVAL: 'data' must be a string or byte array".to_string()),
                }
            }
            "fs.readdir" => {
                let path = self.require_fs_path_param(params, "path")?;
                let with_file_types = params
                    .get("withFileTypes")
                    .and_then(|v| v.as_bool())
                    .or_else(|| {
                        params
                            .get("options")
                            .and_then(|o| o.get("withFileTypes"))
                            .and_then(|v| v.as_bool())
                    })
                    .unwrap_or(false);
                let entries = std::fs::read_dir(&path).map_err(map_io_error)?;
                if with_file_types {
                    let mut list = Vec::new();
                    for entry in entries {
                        let entry = entry.map_err(map_io_error)?;
                        let name = entry.file_name().to_string_lossy().into_owned();
                        let entry_path = entry.path();
                        let ft = entry.file_type().map_err(map_io_error)?;
                        let is_symlink = ft.is_symlink()
                            || std::fs::symlink_metadata(&entry_path)
                                .is_ok_and(|m| is_symlink_or_reparse_point(&entry_path, &m));
                        let is_file = !is_symlink && ft.is_file();
                        let is_dir = !is_symlink && ft.is_dir();
                        list.push((name, is_file, is_dir, is_symlink));
                    }
                    list.sort_by(|a, b| a.0.cmp(&b.0));
                    let json_entries: Vec<Value> = list
                        .into_iter()
                        .map(|(name, is_file, is_dir, is_symlink)| {
                            json!({
                                "name": name,
                                "isFile": is_file,
                                "isDirectory": is_dir,
                                "isSymbolicLink": is_symlink,
                            })
                        })
                        .collect();
                    Ok(Value::Array(json_entries))
                } else {
                    let mut names = Vec::new();
                    for entry in entries {
                        let entry = entry.map_err(map_io_error)?;
                        names.push(entry.file_name().to_string_lossy().into_owned());
                    }
                    names.sort();
                    Ok(json!(names))
                }
            }
            "fs.stat" => {
                let path = self.require_fs_path_param(params, "path")?;
                let meta = std::fs::metadata(&path).map_err(map_io_error)?;
                let sym_meta = std::fs::symlink_metadata(&path).ok();
                let is_symlink = sym_meta
                    .as_ref()
                    .is_some_and(|m| is_symlink_or_reparse_point(&path, m));
                let mtime_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                Ok(json!({
                    "size": meta.len(),
                    "isFile": meta.is_file(),
                    "isDirectory": meta.is_dir(),
                    "isSymbolicLink": is_symlink,
                    "mtimeMs": mtime_ms,
                }))
            }
            "fs.mkdir" => {
                let path = self.require_fs_path_param(params, "path")?;
                let recursive = params
                    .get("options")
                    .and_then(|o| {
                        o.as_bool()
                            .or_else(|| o.get("recursive").and_then(|r| r.as_bool()))
                    })
                    .or_else(|| params.get("recursive").and_then(|r| r.as_bool()))
                    .unwrap_or(false);
                let first_created = if recursive {
                    let mut first_missing: Option<PathBuf> = None;
                    let mut cur = path.as_path();
                    while std::fs::symlink_metadata(cur).is_err() {
                        first_missing = Some(cur.to_path_buf());
                        match cur.parent() {
                            Some(p) if !p.as_os_str().is_empty() && p != cur => cur = p,
                            _ => break,
                        }
                    }
                    std::fs::create_dir_all(&path).map_err(map_io_error)?;
                    first_missing.map(|p| p.to_string_lossy().into_owned())
                } else {
                    std::fs::create_dir(&path).map_err(map_io_error)?;
                    None
                };
                Ok(json!({
                    "created": true,
                    "path": path.to_string_lossy().into_owned(),
                    "firstCreated": first_created,
                }))
            }
            "fs.unlink" => {
                let raw_path = params
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "EINVAL: Missing string 'path' parameter".to_string())?;
                let path = self.resolve_fs_link_path(raw_path)?;
                if let Ok(meta) = std::fs::symlink_metadata(&path) {
                    if !is_symlink_or_reparse_point(&path, &meta) && meta.is_dir() {
                        return Err(format!(
                            "EISDIR: illegal operation on a directory, unlink '{}'",
                            path.display()
                        ));
                    }
                }
                std::fs::remove_file(&path).map_err(map_io_error)?;
                self.file_system_service.on_did_change.fire(FileChangeEvent {
                    path: path.clone(),
                    change_type: FileChangeType::Deleted,
                });
                Ok(json!({
                    "unlinked": true,
                    "path": path.to_string_lossy().into_owned(),
                }))
            }
            "fs.access" => {
                let path = self.require_fs_path_param(params, "path")?;
                let mode = params.get("mode").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let meta = std::fs::metadata(&path).map_err(map_io_error)?;
                if (mode & 2) != 0 && meta.permissions().readonly() {
                    return Err(format!(
                        "EACCES: permission denied, access '{}'",
                        path.display()
                    ));
                }
                #[cfg(unix)]
                if (mode & 1) != 0 && !meta.is_dir() {
                    use std::os::unix::fs::PermissionsExt;
                    if (meta.permissions().mode() & 0o111) == 0 {
                        return Err(format!(
                            "EACCES: permission denied, access '{}'",
                            path.display()
                        ));
                    }
                }
                Ok(json!({ "accessible": true }))
            }
            "fs.rm" => {
                let raw_path = params
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "EINVAL: Missing string 'path' parameter".to_string())?;
                let path = self.resolve_fs_link_path(raw_path)?;
                let recursive = params
                    .get("options")
                    .and_then(|o| o.get("recursive"))
                    .and_then(|v| v.as_bool())
                    .or_else(|| params.get("recursive").and_then(|v| v.as_bool()))
                    .unwrap_or(false);
                let force = params
                    .get("options")
                    .and_then(|o| o.get("force"))
                    .and_then(|v| v.as_bool())
                    .or_else(|| params.get("force").and_then(|v| v.as_bool()))
                    .unwrap_or(false);

                match std::fs::symlink_metadata(&path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        if force {
                            return Ok(json!({ "removed": false }));
                        }
                        return Err(map_io_error(e));
                    }
                    Err(e) => return Err(map_io_error(e)),
                    Ok(meta) => {
                        if is_symlink_or_reparse_point(&path, &meta) {
                            #[cfg(windows)]
                            if meta.is_dir() {
                                std::fs::remove_dir(&path).map_err(map_io_error)?;
                            } else {
                                std::fs::remove_file(&path).map_err(map_io_error)?;
                            }
                            #[cfg(not(windows))]
                            std::fs::remove_file(&path).map_err(map_io_error)?;
                        } else if meta.is_dir() {
                            if !recursive {
                                return Err(format!(
                                    "EISDIR: illegal operation on a directory, rm '{}'",
                                    path.display()
                                ));
                            }
                            std::fs::remove_dir_all(&path).map_err(map_io_error)?;
                        } else {
                            std::fs::remove_file(&path).map_err(map_io_error)?;
                        }
                    }
                }
                self.file_system_service.on_did_change.fire(FileChangeEvent {
                    path: path.clone(),
                    change_type: FileChangeType::Deleted,
                });
                Ok(json!({ "removed": true }))
            }
            "fs.rename" => {
                let old_raw = params
                    .get("oldPath")
                    .or_else(|| params.get("src"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "EINVAL: Missing string 'oldPath' parameter".to_string())?;
                let new_raw = params
                    .get("newPath")
                    .or_else(|| params.get("dest"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "EINVAL: Missing string 'newPath' parameter".to_string())?;
                let old_path = self.resolve_fs_link_path(old_raw)?;
                let new_path = self.resolve_fs_path(new_raw)?;
                std::fs::rename(&old_path, &new_path).map_err(map_io_error)?;
                self.file_system_service.on_did_change.fire(FileChangeEvent {
                    path: old_path,
                    change_type: FileChangeType::Deleted,
                });
                self.file_system_service.on_did_change.fire(FileChangeEvent {
                    path: new_path,
                    change_type: FileChangeType::Created,
                });
                Ok(json!({ "renamed": true }))
            }
            "fs.copyFile" => {
                let src_raw = params
                    .get("src")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "EINVAL: Missing string 'src' parameter".to_string())?;
                let dest_raw = params
                    .get("dest")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "EINVAL: Missing string 'dest' parameter".to_string())?;
                let mode = params.get("mode").and_then(|v| v.as_u64()).unwrap_or(0);
                let src = self.resolve_fs_path(src_raw)?;
                let dest = self.resolve_fs_path(dest_raw)?;

                let meta = std::fs::metadata(&src).map_err(map_io_error)?;
                if meta.is_dir() {
                    return Err(format!(
                        "EISDIR: illegal operation on a directory, copyFile '{}'",
                        src.display()
                    ));
                }
                if (mode & 1) != 0 {
                    let mut reader = std::fs::File::open(&src).map_err(map_io_error)?;
                    let mut writer = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&dest)
                        .map_err(map_io_error)?;
                    std::io::copy(&mut reader, &mut writer).map_err(map_io_error)?;
                } else {
                    std::fs::copy(&src, &dest).map_err(map_io_error)?;
                }
                self.file_system_service.on_did_change.fire(FileChangeEvent {
                    path: dest,
                    change_type: FileChangeType::Changed,
                });
                Ok(json!({ "copied": true }))
            }
            _ => Err(format!(
                "METHOD_NOT_FOUND: Unsupported fs RPC method: {}",
                method
            )),
        }
    }

    fn parse_fs_input_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        if raw_path.trim().is_empty() || raw_path.contains('\0') {
            return Err("EINVAL: Invalid empty or NUL path".to_string());
        }
        let path = if rustcode_platform::is_file_uri(raw_path) {
            decode_file_uri(raw_path)
        } else {
            PathBuf::from(raw_path)
        };
        let path = rustcode_platform::strip_windows_verbatim_prefix(path);
        if path.as_os_str().is_empty() || path.to_string_lossy().contains('\0') {
            return Err("EINVAL: Invalid empty or NUL path".to_string());
        }
        if self.fs_root.is_some() {
            #[cfg(windows)]
            {
                let has_prefix = path
                    .components()
                    .any(|c| matches!(c, Component::Prefix(_)));
                if has_prefix && !path.has_root() {
                    return Err(format!(
                        "EACCES: Drive-relative path '{}' is not permitted inside fs_root",
                        path.display()
                    ));
                }
            }
        }
        Ok(path)
    }

    fn resolve_sandbox_root(&self, root: &Path) -> PathBuf {
        let abs_root = if root.is_absolute() {
            root.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(root))
                .unwrap_or_else(|_| root.to_path_buf())
        };
        canonicalize_existing_prefix(&abs_root)
    }

    fn resolve_fs_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        let path = self.parse_fs_input_path(raw_path)?;
        if let Some(root) = &self.fs_root {
            let canon_root = self.resolve_sandbox_root(root);
            let candidate = if path.is_absolute() {
                path
            } else {
                canon_root.join(&path)
            };
            let mut symlinks_followed = 0usize;
            resolve_sandbox_steps(path_to_steps(&candidate), &canon_root, &mut symlinks_followed)
        } else {
            Ok(path)
        }
    }

    fn resolve_fs_link_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        let path = self.parse_fs_input_path(raw_path)?;
        if let Some(root) = &self.fs_root {
            let canon_root = self.resolve_sandbox_root(root);
            let candidate = if path.is_absolute() {
                path
            } else {
                canon_root.join(&path)
            };
            let normalized = lexical_normalize(&candidate);
            let (parent, final_name) = match (normalized.parent(), normalized.file_name()) {
                (Some(p), Some(name)) => (p, name),
                _ => {
                    return Err(format!(
                        "EACCES: Cannot target root or invalid link path '{}'",
                        normalized.display()
                    ));
                }
            };
            let mut symlinks_followed = 0usize;
            let resolved_parent =
                resolve_sandbox_steps(path_to_steps(parent), &canon_root, &mut symlinks_followed)?;
            if !rustcode_platform::path_starts_with(&resolved_parent, &canon_root) {
                return Err(format!(
                    "EACCES: Path '{}' escapes fs_root",
                    normalized.display()
                ));
            }
            Ok(resolved_parent.join(final_name))
        } else {
            Ok(path)
        }
    }
}

impl RpcRequestHandler for LanguageFeatureBridge {
    fn handle_request<'a>(
        &'a self,
        method: &'a str,
        params: &'a Value,
    ) -> RpcBoxFuture<'a, Result<Value, RpcError>> {
        Box::pin(async move { self.handle_rpc_message_result(method, params).await })
    }
}

static TEMP_FILE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn strip_untitled_prefix(uri: &str) -> Option<&str> {
    uri.strip_prefix("untitled://")
        .or_else(|| uri.strip_prefix("untitled:"))
}

fn find_open_document_key(
    docs: &HashMap<String, OpenDocumentSnapshot>,
    uri_or_path: &str,
) -> Option<String> {
    if docs.contains_key(uri_or_path) {
        return Some(uri_or_path.to_string());
    }
    if let Some(target_name) = strip_untitled_prefix(uri_or_path) {
        let canonical = format!("untitled:{}", target_name);
        if docs.contains_key(&canonical) {
            return Some(canonical);
        }
        let slash = format!("untitled://{}", target_name);
        if docs.contains_key(&slash) {
            return Some(slash);
        }
        return docs
            .iter()
            .find(|(_, snap)| strip_untitled_prefix(&snap.uri) == Some(target_name))
            .map(|(k, _)| k.clone());
    }
    let path = decode_file_uri(uri_or_path);
    let normalized_uri = rustcode_platform::path_to_file_uri(&path);
    if docs.contains_key(&normalized_uri) {
        return Some(normalized_uri);
    }
    let alt_uri = format!("file://{}", path.to_string_lossy());
    if docs.contains_key(&alt_uri) {
        return Some(alt_uri);
    }
    docs.iter()
        .find(|(_, snap)| {
            !snap.is_untitled
                && rustcode_platform::paths_equivalent(Path::new(&snap.file_name), &path)
        })
        .map(|(k, _)| k.clone())
}

fn utf16_pos_to_byte_offset(text: &str, pos: LspPosition) -> usize {
    let target_line = pos.line as usize;
    let target_col = pos.character as usize;
    let mut byte_idx = 0usize;

    for (line_idx, segment) in text.split('\n').enumerate() {
        if line_idx == target_line {
            let clean = segment.strip_suffix('\r').unwrap_or(segment);
            let mut utf16_used = 0usize;
            for (rel_b, ch) in clean.char_indices() {
                let w = ch.len_utf16();
                if utf16_used + w > target_col {
                    return byte_idx + rel_b;
                }
                utf16_used += w;
                if utf16_used == target_col {
                    return byte_idx + rel_b + ch.len_utf8();
                }
            }
            return byte_idx + clean.len();
        }
        byte_idx += segment.len();
        if byte_idx < text.len() {
            byte_idx += 1; // '\n'
        }
    }
    text.len()
}

fn apply_text_edits_to_str(original: &str, edits: &[LspTextEdit]) -> String {
    let mut resolved: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|e| {
            let start = utf16_pos_to_byte_offset(original, e.range.start);
            let end = utf16_pos_to_byte_offset(original, e.range.end);
            let (lo, hi) = if start <= end { (start, end) } else { (end, start) };
            (lo, hi, e.new_text.as_str())
        })
        .collect();
    resolved.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));

    let mut out = original.to_string();
    for (lo, hi, new_text) in resolved {
        if lo <= hi && hi <= out.len() {
            out.replace_range(lo..hi, new_text);
        }
    }
    out
}

fn collect_workspace_files(
    root: &Path,
    dir: &Path,
    use_default_excludes: bool,
    out: &mut Vec<(String, PathBuf)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut list: Vec<std::fs::DirEntry> = entries.flatten().collect();
    list.sort_by_key(|e| e.file_name());
    for entry in list {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if use_default_excludes
                && matches!(name_str.as_ref(), ".git" | "node_modules" | "target")
            {
                continue;
            }
            collect_workspace_files(root, &path, use_default_excludes, out);
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, path));
        }
    }
}

fn glob_matches_relative_path(pattern: &str, rel_path: &str) -> bool {
    let norm_pat = pattern.replace('\\', "/");
    let expanded = expand_brace_patterns(norm_pat.trim());
    let norm_rel = rel_path.replace('\\', "/");
    let file_name = norm_rel.rsplit('/').next().unwrap_or(&norm_rel);
    expanded.iter().any(|pat| {
        use std::borrow::Cow;
        #[cfg(windows)]
        let (p_cmp, r_cmp, f_cmp): (Cow<'_, str>, Cow<'_, str>, Cow<'_, str>) = (
            Cow::Owned(pat.to_ascii_lowercase()),
            Cow::Owned(norm_rel.to_ascii_lowercase()),
            Cow::Owned(file_name.to_ascii_lowercase()),
        );
        #[cfg(not(windows))]
        let (p_cmp, r_cmp, f_cmp): (Cow<'_, str>, Cow<'_, str>, Cow<'_, str>) = (
            Cow::Borrowed(pat.as_str()),
            Cow::Borrowed(norm_rel.as_str()),
            Cow::Borrowed(file_name),
        );

        if p_cmp.contains('/') {
            let p_segs: Vec<&str> = p_cmp.split('/').filter(|s| !s.is_empty()).collect();
            let r_segs: Vec<&str> = r_cmp.split('/').filter(|s| !s.is_empty()).collect();
            match_glob_segments(&p_segs, &r_segs)
        } else {
            wildcard_segment_match(p_cmp.as_ref(), f_cmp.as_ref())
        }
    })
}

fn expand_brace_patterns(pat: &str) -> Vec<String> {
    if let (Some(open), Some(close)) = (pat.find('{'), pat.find('}')) {
        if open < close {
            let prefix = &pat[..open];
            let suffix = &pat[close + 1..];
            let inner = &pat[open + 1..close];
            let mut out = Vec::new();
            for alt in inner.split(',') {
                for sub in expand_brace_patterns(&format!("{}{}{}", prefix, alt, suffix)) {
                    out.push(sub);
                }
            }
            return out;
        }
    }
    vec![pat.to_string()]
}

fn match_glob_segments(pat: &[&str], path: &[&str]) -> bool {
    if pat.is_empty() {
        return path.is_empty();
    }
    if pat[0] == "**" {
        if pat.len() == 1 {
            return true;
        }
        for i in 0..=path.len() {
            if match_glob_segments(&pat[1..], &path[i..]) {
                return true;
            }
        }
        return false;
    }
    if path.is_empty() {
        return false;
    }
    wildcard_segment_match(pat[0], path[0]) && match_glob_segments(&pat[1..], &path[1..])
}

fn wildcard_segment_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(comp.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn normalize_canonical_path(path: PathBuf) -> PathBuf {
    rustcode_platform::strip_windows_verbatim_prefix(path)
}

fn is_symlink_or_reparse_point(path: &Path, meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
            && std::fs::read_link(path).is_ok()
        {
            return true;
        }
    }
    #[cfg(not(windows))]
    let _ = path;
    false
}

fn canonicalize_existing_prefix(path: &Path) -> PathBuf {
    let normalized = lexical_normalize(path);
    let mut suffix = Vec::new();
    let mut cur = normalized.as_path();
    loop {
        if std::fs::symlink_metadata(cur).is_ok() {
            if let Ok(canon) = std::fs::canonicalize(cur) {
                let mut canon = normalize_canonical_path(canon);
                for seg in suffix.into_iter().rev() {
                    canon.push(seg);
                }
                return canon;
            }
        }
        match (cur.file_name(), cur.parent()) {
            (Some(name), Some(parent)) => {
                suffix.push(name.to_os_string());
                cur = parent;
            }
            _ => return normalized,
        }
    }
}

enum PathStep {
    Prefix(std::ffi::OsString),
    RootDir,
    ParentDir,
    Normal(std::ffi::OsString),
}

fn path_to_steps(path: &Path) -> Vec<PathStep> {
    let mut steps = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => steps.push(PathStep::Prefix(p.as_os_str().to_os_string())),
            Component::RootDir => steps.push(PathStep::RootDir),
            Component::CurDir => {}
            Component::ParentDir => steps.push(PathStep::ParentDir),
            Component::Normal(s) => steps.push(PathStep::Normal(s.to_os_string())),
        }
    }
    steps
}

fn resolve_sandbox_steps(
    steps: Vec<PathStep>,
    canon_root: &Path,
    symlinks_followed: &mut usize,
) -> Result<PathBuf, String> {
    use std::collections::VecDeque;
    const MAX_SYMLINKS: usize = 40;

    let mut queue: VecDeque<PathStep> = steps.into();
    let mut cur = PathBuf::new();

    while let Some(step) = queue.pop_front() {
        match step {
            PathStep::Prefix(p) => cur.push(p),
            PathStep::RootDir => {
                cur.push(Component::RootDir.as_os_str());
                #[cfg(windows)]
                if let Ok(canon) = std::fs::canonicalize(&cur) {
                    cur = normalize_canonical_path(canon);
                }
            }
            PathStep::ParentDir => {
                cur.pop();
            }
            PathStep::Normal(seg) => {
                let next = cur.join(&seg);
                match std::fs::symlink_metadata(&next) {
                    Ok(meta) if is_symlink_or_reparse_point(&next, &meta) => {
                        *symlinks_followed += 1;
                        if *symlinks_followed > MAX_SYMLINKS {
                            return Err("ELOOP: Too many levels of symbolic links".to_string());
                        }
                        let target = normalize_canonical_path(
                            std::fs::read_link(&next).map_err(map_io_error)?,
                        );
                        if rustcode_platform::path_starts_with(&cur, canon_root) {
                            let candidate_target = if target.is_absolute() {
                                target
                            } else {
                                cur.join(&target)
                            };
                            let resolved_target = resolve_sandbox_steps(
                                path_to_steps(&candidate_target),
                                canon_root,
                                symlinks_followed,
                            )?;
                            if !rustcode_platform::path_starts_with(&resolved_target, canon_root) {
                                return Err(format!(
                                    "EACCES: Path '{}' escapes fs_root via symlink",
                                    next.display()
                                ));
                            }
                            cur = resolved_target;
                        } else {
                            if target.is_absolute() {
                                cur = PathBuf::new();
                            }
                            for ts in path_to_steps(&target).into_iter().rev() {
                                queue.push_front(ts);
                            }
                        }
                    }
                    Ok(_) => {
                        #[cfg(windows)]
                        {
                            if let Ok(canon_next) = std::fs::canonicalize(&next) {
                                let canon_next = normalize_canonical_path(canon_next);
                                if rustcode_platform::path_starts_with(&cur, canon_root)
                                    && !rustcode_platform::path_starts_with(&canon_next, canon_root)
                                {
                                    return Err(format!(
                                        "EACCES: Path '{}' escapes fs_root",
                                        next.display()
                                    ));
                                }
                                cur = canon_next;
                            } else {
                                cur = next;
                            }
                        }
                        #[cfg(not(windows))]
                        {
                            cur = next;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        cur = next;
                    }
                    Err(_) => {
                        cur = next;
                    }
                }
            }
        }
    }

    if !rustcode_platform::path_starts_with(&cur, canon_root) {
        return Err(format!(
            "EACCES: Path '{}' escapes fs_root '{}'",
            cur.display(),
            canon_root.display()
        ));
    }
    Ok(cur)
}

fn decode_file_uri(uri_or_path: &str) -> PathBuf {
    rustcode_platform::file_uri_to_path(uri_or_path)
}

fn has_non_file_uri_scheme(raw: &str) -> bool {
    let trimmed = raw.trim();
    if let Some(colon_idx) = trimmed.find(':') {
        // Single-letter prefixes (e.g. `C:\...` or `C:/...`) are Windows drive paths, not URI schemes.
        if colon_idx <= 1 {
            return false;
        }
        let scheme = &trimmed[..colon_idx];
        let valid_scheme = scheme.bytes().enumerate().all(|(i, b)| {
            if i == 0 {
                b.is_ascii_alphabetic()
            } else {
                b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')
            }
        });
        if valid_scheme {
            return !scheme.eq_ignore_ascii_case("file");
        }
    }
    false
}

fn extract_path_from_scope_value(val: &Value) -> Option<PathBuf> {
    match val {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || has_non_file_uri_scheme(trimmed) {
                None
            } else {
                Some(decode_file_uri(trimmed))
            }
        }
        Value::Object(obj) => {
            if let Some(scheme) = obj.get("scheme").and_then(|v| v.as_str()) {
                let sch = scheme.trim();
                if !sch.is_empty() && !sch.eq_ignore_ascii_case("file") {
                    return None;
                }
            }
            if let Some(fs_path) = obj.get("fsPath").and_then(|v| v.as_str()) {
                let trimmed = fs_path.trim();
                if !trimmed.is_empty() && !has_non_file_uri_scheme(trimmed) {
                    return Some(decode_file_uri(trimmed));
                }
            }
            if let Some(path) = obj.get("path").and_then(|v| v.as_str()) {
                let trimmed = path.trim();
                if !trimmed.is_empty() {
                    let scheme = obj.get("scheme").and_then(|v| v.as_str()).map(str::trim);
                    let authority = obj
                        .get("authority")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .unwrap_or("");
                    if scheme.is_some_and(|s| s.eq_ignore_ascii_case("file"))
                        || trimmed.starts_with('/')
                        || !authority.is_empty()
                    {
                        let reconstructed = if trimmed.starts_with('/') {
                            format!("file://{}{}", authority, trimmed)
                        } else {
                            format!("file://{}/{}", authority, trimmed)
                        };
                        return Some(decode_file_uri(&reconstructed));
                    }
                    if !has_non_file_uri_scheme(trimmed) {
                        return Some(decode_file_uri(trimmed));
                    }
                    return None;
                }
            }
            if let Some(uri) = obj.get("uri") {
                return extract_path_from_scope_value(uri);
            }
            None
        }
        _ => None,
    }
}

fn extract_scope_path(params: &Value) -> Option<PathBuf> {
    let scope = params.get("scope").or_else(|| params.get("folder"))?;
    extract_path_from_scope_value(scope)
}

fn traverse_config_value(root: &Value, section: &str) -> Option<Value> {
    if section.is_empty() {
        return Some(root.clone());
    }
    let parts: Vec<&str> = section.split('.').collect();
    traverse_config_parts(root, &parts)
}

fn traverse_config_parts(cur: &Value, parts: &[&str]) -> Option<Value> {
    if parts.is_empty() {
        return Some(cur.clone());
    }
    for i in (1..=parts.len()).rev() {
        let key = if i == 1 {
            parts[0].to_string()
        } else {
            parts[..i].join(".")
        };
        if let Some(next) = cur.get(&key) {
            if let Some(found) = traverse_config_parts(next, &parts[i..]) {
                return Some(found);
            }
        }
    }
    None
}

fn map_manager_error(method: &str, err: String) -> RpcError {
    if err.starts_with("Unsupported ")
        || err.starts_with("Unknown ")
        || err.starts_with("METHOD_NOT_FOUND:")
    {
        return RpcError::method_not_found(method);
    }
    if let Some((prefix, rest)) = err.split_once(": ") {
        if !prefix.is_empty()
            && prefix
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            return RpcError::new(prefix, rest);
        }
    }
    if err.starts_with("Missing ") || err.starts_with("Invalid ") {
        return RpcError::invalid_params(err);
    }
    if err.contains("is disposed")
        || err.contains("already terminated")
        || err.contains("has already ")
        || err.contains("not registered")
    {
        return RpcError::new("INVALID_STATE", err);
    }
    if err.contains("not found") {
        return RpcError::new("NOT_FOUND", err);
    }
    RpcError::new("RPC_ERROR", err)
}

fn map_io_error(err: std::io::Error) -> String {
    #[cfg(windows)]
    if let Some(raw) = err.raw_os_error() {
        let win_code = match raw {
            2 | 3 | 15 => Some("ENOENT"),
            5 | 82 => Some("EACCES"),
            17 => Some("EXDEV"),
            19 => Some("EROFS"),
            32 | 33 => Some("EBUSY"),
            80 | 183 => Some("EEXIST"),
            87 | 123 | 161 => Some("EINVAL"),
            112 => Some("ENOSPC"),
            145 => Some("ENOTEMPTY"),
            206 => Some("ENAMETOOLONG"),
            267 => Some("ENOTDIR"),
            1314 => Some("EPERM"),
            _ => None,
        };
        if let Some(code) = win_code {
            return format!("{}: {}", code, err);
        }
    }

    let code = match err.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::AlreadyExists => "EEXIST",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::InvalidInput => "EINVAL",
        std::io::ErrorKind::NotADirectory => "ENOTDIR",
        std::io::ErrorKind::IsADirectory => "EISDIR",
        std::io::ErrorKind::DirectoryNotEmpty => "ENOTEMPTY",
        std::io::ErrorKind::ReadOnlyFilesystem => "EROFS",
        std::io::ErrorKind::ResourceBusy => "EBUSY",
        std::io::ErrorKind::StorageFull => "ENOSPC",
        std::io::ErrorKind::CrossesDevices => "EXDEV",
        _ => {
            #[cfg(unix)]
            {
                match err.raw_os_error() {
                    Some(21) => "EISDIR",
                    Some(20) => "ENOTDIR",
                    Some(1) => "EPERM",
                    Some(16) => "EBUSY",
                    Some(39) | Some(66) => "ENOTEMPTY",
                    Some(40) | Some(62) => "ELOOP",
                    _ => "EIO",
                }
            }
            #[cfg(not(unix))]
            {
                "EIO"
            }
        }
    };
    format!("{}: {}", code, err)
}

fn map_fs_error(err: FileSystemError) -> String {
    match err {
        FileSystemError::NotFound(p) => format!("ENOENT: no such file or directory: {}", p.display()),
        FileSystemError::Io(io_err) => map_io_error(io_err),
        other => format!("EIO: {}", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_protocols::lsp::{
        read_framed_message, write_framed_message, LspClient, LspPosition,
    };
    use tokio::io::duplex;

    #[tokio::test]
    async fn test_language_feature_bridge_aggregates_lsp_and_js() {
        let reg = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp_mgr = Arc::new(LspClientManager::new());

        // 1. Set up mock LSP client for rust
        let (client_read, mut server_write) = duplex(4096);
        let (mut server_read, client_write) = duplex(4096);
        let lsp_client = Arc::new(LspClient::from_io(client_read, client_write));

        tokio::spawn(async move {
            let mut buf = Vec::new();
            // Handle completion request from LSP
            if let Ok(Some(bytes)) = read_framed_message(&mut server_read, &mut buf).await {
                let req: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let id = req["id"].as_u64().unwrap();
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": [
                        { "label": "to_string", "detail": "fn(&self) -> String", "insertText": "to_string()" }
                    ]
                });
                let _ = write_framed_message(&mut server_write, &serde_json::to_vec(&resp).unwrap()).await;
            }
        });

        lsp_mgr.attach_client("rust", lsp_client);

        // 2. Set up JS extension provider
        reg.register(super::super::ProviderRegistration {
            id: "js-rust-completions".to_string(),
            provider_type: ProviderType::Completion,
            language_selector: Some("rust".to_string()),
        });

        let bridge = LanguageFeatureBridge::new(reg, rpc.clone(), lsp_mgr);

        // Run bridge completion in background task
        let bridge_task = tokio::spawn(async move {
            bridge
                .request_completions("file:///main.rs", "rust", LspPosition::new(0, 5), Some('.'))
                .await
        });

        // Yield to allow bridge to create request
        tokio::task::yield_now().await;

        // Fulfill JS request with an additional item
        rpc.handle_incoming_response(
            1,
            Some(json!([
                { "label": "custom_macro", "detail": "macro_rules!", "insertText": "custom_macro!()" }
            ])),
            None,
        );

        let items = bridge_task.await.unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.label == "to_string"));
        assert!(items.iter().any(|i| i.label == "custom_macro"));
    }

    #[test]
    fn test_diagnostic_collection_manager_and_rpc() {
        let manager = DiagnosticCollectionManager::new();
        let notified = Arc::new(Mutex::new(Vec::new()));
        let notified_clone = notified.clone();
        manager.on_diagnostics(move |uri, diags| {
            notified_clone.lock().push((uri.to_string(), diags.len()));
        });

        // 1. Ingest via RPC notification: languages.setDiagnostics
        let params = json!({
            "name": "eslint",
            "uri": "file:///src/index.js",
            "diagnostics": [
                {
                    "range": {
                        "start": { "line": 5, "character": 2 },
                        "end": { "line": 5, "character": 10 }
                    },
                    "severity": 1,
                    "message": "Missing semicolon",
                    "source": "eslint",
                    "code": "semi"
                }
            ]
        });

        let handled = manager.handle_rpc_message("languages.setDiagnostics", &params);
        assert!(handled);

        let diags = manager.get_diagnostics_for_uri("file:///src/index.js");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Missing semicolon");
        assert_eq!(diags[0].source.as_deref(), Some("eslint"));

        // 2. Clear diagnostics for file
        let clear_params = json!({
            "name": "eslint",
            "uri": "file:///src/index.js"
        });
        let handled_clear = manager.handle_rpc_message("languages.clearDiagnostics", &clear_params);
        assert!(handled_clear);
        assert!(manager.get_diagnostics_for_uri("file:///src/index.js").is_empty());

        let events = notified.lock().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], ("file:///src/index.js".to_string(), 1));
        assert_eq!(events[1], ("file:///src/index.js".to_string(), 0));
    }

    #[tokio::test]
    async fn test_language_feature_bridge_formatting_aggregation() {
        let reg = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp_mgr = Arc::new(LspClientManager::new());

        reg.register(super::super::ProviderRegistration {
            id: "js-prettier".to_string(),
            provider_type: ProviderType::DocumentFormatting,
            language_selector: Some("javascript".to_string()),
        });

        let bridge = LanguageFeatureBridge::new(reg, rpc.clone(), lsp_mgr);

        let bridge_task = tokio::spawn(async move {
            bridge
                .request_formatting(
                    "file:///index.js",
                    "javascript",
                    FormattingOptions::new(2, true),
                )
                .await
        });

        tokio::task::yield_now().await;

        rpc.handle_incoming_response(
            1,
            Some(json!([
                {
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 10 }
                    },
                    "newText": "const x = 1;"
                }
            ])),
            None,
        );

        let edits = bridge_task.await.unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "const x = 1;");
    }

    #[tokio::test]
    async fn test_language_feature_bridge_code_action_aggregation() {
        let reg = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp_mgr = Arc::new(LspClientManager::new());

        reg.register(super::super::ProviderRegistration {
            id: "js-quickfix".to_string(),
            provider_type: ProviderType::CodeAction,
            language_selector: Some("rust".to_string()),
        });

        let bridge = LanguageFeatureBridge::new(reg, rpc.clone(), lsp_mgr);

        let bridge_task = tokio::spawn(async move {
            bridge
                .request_code_actions(
                    "file:///main.rs",
                    "rust",
                    LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 5)),
                    CodeActionContext::default(),
                )
                .await
        });

        tokio::task::yield_now().await;

        rpc.handle_incoming_response(
            1,
            Some(json!([
                {
                    "title": "Import std::sync::Arc",
                    "kind": "quickfix"
                }
            ])),
            None,
        );

        let actions = bridge_task.await.unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Import std::sync::Arc");
        assert_eq!(actions[0].kind.as_deref(), Some("quickfix"));
    }

    #[tokio::test]
    async fn test_rpc_register_and_unregister_provider_roundtrip() {
        let reg = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp_mgr = Arc::new(LspClientManager::new());
        let bridge = LanguageFeatureBridge::new(reg.clone(), rpc, lsp_mgr);

        // 1. Register provider via RPC
        let reg_params = json!({
            "type": "CodeAction",
            "id": "my-codeaction-provider",
            "selector": "rust"
        });
        let reg_res = bridge.handle_rpc_message("extHost.registerProvider", &reg_params).await;
        assert!(reg_res.is_some());
        assert_eq!(reg.get_providers_for_language(ProviderType::CodeAction, "rust").len(), 1);

        // 2. Unregister provider via RPC
        let unreg_params = json!({
            "id": "my-codeaction-provider"
        });
        let unreg_res = bridge.handle_rpc_message("extHost.unregisterProvider", &unreg_params).await;
        assert!(unreg_res.is_some());
        assert!(reg.get_providers_for_language(ProviderType::CodeAction, "rust").is_empty());
    }

    #[test]
    fn test_tree_item_collapsible_state_and_context_value_serde() {
        let from_int: TreeItem = serde_json::from_value(json!({
            "id": "n1",
            "label": "src",
            "collapsibleState": 1,
            "contextValue": "folder"
        }))
        .unwrap();
        assert_eq!(from_int.collapsible_state, TreeItemCollapsibleState::Collapsed);
        assert_eq!(from_int.context_value.as_deref(), Some("folder"));

        let from_str: TreeItem = serde_json::from_value(json!({
            "label": "lib.rs",
            "collapsibleState": "Expanded",
            "context_value": "rustFile"
        }))
        .unwrap();
        assert_eq!(from_str.collapsible_state, TreeItemCollapsibleState::Expanded);
        assert_eq!(from_str.context_value.as_deref(), Some("rustFile"));

        let serialized = serde_json::to_value(&from_int).unwrap();
        assert_eq!(serialized["collapsibleState"], json!(1));
        assert_eq!(serialized["contextValue"], json!("folder"));
        assert!(serialized.get("context_value").is_none());
    }

    #[tokio::test]
    async fn test_fs_rpc_handlers_and_validation() {
        let root = std::env::temp_dir().join(format!(
            "rustcode_fs_rpc_unit_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let reg = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let fs_svc = Arc::new(FileSystemService::new());

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let _sub = fs_svc.on_did_change.event().attach(move |ev| {
            events_clone.lock().push(ev.path.clone());
        });

        let bridge = LanguageFeatureBridge::new(reg, rpc, lsp)
            .with_file_system_service(fs_svc)
            .with_fs_root(&root);

        // Invalid paths -> EINVAL
        assert!(bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": "   " }))
            .await
            .unwrap_err()
            .starts_with("EINVAL:"));
        assert!(bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": "bad\0path" }))
            .await
            .unwrap_err()
            .starts_with("EINVAL:"));

        // Escape fs_root -> EACCES
        assert!(bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": "../../etc/passwd" }))
            .await
            .unwrap_err()
            .starts_with("EACCES:"));

        // Non-recursive mkdir with missing parent -> ENOENT
        let missing_parent = root.join("nested/sub");
        assert!(bridge
            .handle_fs_rpc("fs.mkdir", &json!({ "path": missing_parent.to_string_lossy() }))
            .await
            .unwrap_err()
            .starts_with("ENOENT:"));

        // Recursive mkdir succeeds
        bridge
            .handle_fs_rpc(
                "fs.mkdir",
                &json!({ "path": "nested/sub", "options": { "recursive": true } }),
            )
            .await
            .unwrap();

        // Non-recursive mkdir on existing dir -> EEXIST
        assert!(bridge
            .handle_fs_rpc("fs.mkdir", &json!({ "path": "nested/sub" }))
            .await
            .unwrap_err()
            .starts_with("EEXIST:"));

        // writeFile UTF-8 via file:// URI
        let target = root.join("nested/sub/hello.txt");
        let target_uri = format!("file://{}", target.to_string_lossy());
        let wr = bridge
            .handle_fs_rpc("fs.writeFile", &json!({ "path": target_uri, "data": "hi!" }))
            .await
            .unwrap();
        assert_eq!(wr["written"], json!(true));
        assert_eq!(events.lock().len(), 1);

        // readFile UTF-8 and binary (encoding: null)
        let txt = bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": "nested/sub/hello.txt", "encoding": "utf8" }))
            .await
            .unwrap();
        assert_eq!(txt, json!("hi!"));

        let raw_bytes = bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": "nested/sub/hello.txt", "encoding": null }))
            .await
            .unwrap();
        assert_eq!(raw_bytes, json!([104, 105, 33]));

        // readdir sorted + stat + unlink
        bridge
            .handle_fs_rpc("fs.writeFile", &json!({ "path": "nested/sub/alpha.bin", "data": [1, 2, 3] }))
            .await
            .unwrap();
        let entries = bridge
            .handle_fs_rpc("fs.readdir", &json!({ "path": "nested/sub" }))
            .await
            .unwrap();
        assert_eq!(entries, json!(["alpha.bin", "hello.txt"]));

        let st = bridge
            .handle_fs_rpc("fs.stat", &json!({ "path": "nested/sub/alpha.bin" }))
            .await
            .unwrap();
        assert_eq!(st["size"], json!(3));
        assert_eq!(st["isFile"], json!(true));
        assert_eq!(st["isDirectory"], json!(false));

        bridge
            .handle_fs_rpc("fs.unlink", &json!({ "path": "nested/sub/alpha.bin" }))
            .await
            .unwrap();
        assert!(!root.join("nested/sub/alpha.bin").exists());

        // %00 URI percent-decoding must be rejected with EINVAL
        let nul_uri = format!("file://{}/evil%00.txt", root.to_string_lossy());
        let nul_err = bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": nul_uri }))
            .await
            .unwrap_err();
        assert!(nul_err.starts_with("EINVAL:"), "expected EINVAL, got {}", nul_err);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_fs_rpc_rejects_dangling_symlink_and_allows_symlinked_root_alias() {
        let base = std::env::temp_dir().join(format!(
            "rustcode_fs_symlink_unit_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let real_root = base.join("real/sandbox");
        let outside = base.join("outside");
        std::fs::create_dir_all(&real_root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        // Symlinked alias parent to fs_root (like /tmp -> /private/tmp)
        let alias_parent = base.join("alias");
        std::os::unix::fs::symlink(base.join("real"), &alias_parent).unwrap();
        let aliased_root = alias_parent.join("sandbox");

        // Dangling symlink pointing to non-existent file outside fs_root
        let dangling_file = real_root.join("dangling_link");
        let outside_pwned = outside.join("pwned.txt");
        std::os::unix::fs::symlink(&outside_pwned, &dangling_file).unwrap();

        // Dangling directory symlink pointing outside fs_root
        let dangling_dir = real_root.join("dangling_dir");
        let outside_dir = outside.join("newdir");
        std::os::unix::fs::symlink(&outside_dir, &dangling_dir).unwrap();

        let reg = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let bridge = LanguageFeatureBridge::new(reg, rpc, lsp).with_fs_root(&aliased_root);

        // Byte-array writeFile through dangling symlink MUST fail with EACCES and NOT create outside file
        let err1 = bridge
            .handle_fs_rpc(
                "fs.writeFile",
                &json!({ "path": "dangling_link", "data": [112, 119, 110, 101, 100] }),
            )
            .await
            .unwrap_err();
        assert!(err1.starts_with("EACCES:"), "expected EACCES, got {}", err1);
        assert!(!outside_pwned.exists(), "outside file must not be created");

        // String writeFile through dangling directory symlink MUST fail with EACCES
        let err2 = bridge
            .handle_fs_rpc(
                "fs.writeFile",
                &json!({ "path": "dangling_dir/pwned.txt", "data": "pwned" }),
            )
            .await
            .unwrap_err();
        assert!(err2.starts_with("EACCES:"), "expected EACCES, got {}", err2);
        assert!(!outside_dir.exists(), "outside dir must not be created");

        // Absolute path via symlinked alias of fs_root MUST succeed
        let aliased_target = aliased_root.join("ok.txt");
        let ok = bridge
            .handle_fs_rpc(
                "fs.writeFile",
                &json!({ "path": aliased_target.to_string_lossy(), "data": [79, 75] }),
            )
            .await
            .unwrap();
        assert_eq!(ok["written"], json!(true));
        assert_eq!(std::fs::read(real_root.join("ok.txt")).unwrap(), b"OK");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(windows)]
    fn create_windows_junction(link: &Path, target: &Path) {
        use std::os::windows::process::CommandExt;
        let cmd_line = format!(
            "\"mklink /J \"{}\" \"{}\"\"",
            link.display(),
            target.display()
        );
        let status = std::process::Command::new("cmd.exe")
            .args(["/D", "/S", "/C"])
            .raw_arg(&cmd_line)
            .status()
            .expect("failed to execute cmd.exe mklink /J");
        assert!(
            status.success(),
            "mklink /J {} -> {} failed with {:?}",
            link.display(),
            target.display(),
            status
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_fs_rpc_windows_uri_verbatim_and_junction_sandbox() {
        let base = std::env::temp_dir().join(format!(
            "rustcode_fs_win_unit_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let real_parent = base.join("real");
        let root = real_parent.join("sandbox");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        // Junction-aliased parent to fs_root (Windows equivalent of symlinked root alias)
        let alias_parent = base.join("alias");
        create_windows_junction(&alias_parent, &real_parent);
        let aliased_root = alias_parent.join("sandbox");

        let reg = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let bridge = LanguageFeatureBridge::new(reg, rpc, lsp).with_fs_root(&aliased_root);

        // RFC 8089 file:///C:/..., percent-encoded drive colon file:///c%3A/..., and uppercase FILE:///
        let target = root.join("win_file.txt");
        let rfc_uri = rustcode_platform::path_to_file_uri(&target);
        assert!(rfc_uri.starts_with("file:///"), "expected RFC 8089 URI, got {}", rfc_uri);
        let wr = bridge
            .handle_fs_rpc("fs.writeFile", &json!({ "path": rfc_uri, "data": "win-ok" }))
            .await
            .unwrap();
        assert_eq!(wr["written"], json!(true));

        let slash_path = target.to_string_lossy().replace('\\', "/");
        let lower_drive_uri = format!(
            "file:///{}",
            slash_path.replacen(':', "%3A", 1).to_ascii_lowercase()
        );
        let read_back = bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": lower_drive_uri }))
            .await
            .unwrap();
        assert_eq!(read_back, json!("win-ok"));

        let upper_scheme_uri = format!("FILE:///{}", slash_path);
        let read_upper = bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": upper_scheme_uri }))
            .await
            .unwrap();
        assert_eq!(read_upper, json!("win-ok"));

        // Absolute path via junction-aliased fs_root MUST succeed
        let aliased_target = aliased_root.join("via_alias.txt");
        let alias_wr = bridge
            .handle_fs_rpc(
                "fs.writeFile",
                &json!({ "path": aliased_target.to_string_lossy(), "data": [79, 75] }),
            )
            .await
            .unwrap();
        assert_eq!(alias_wr["written"], json!(true));
        assert_eq!(std::fs::read(root.join("via_alias.txt")).unwrap(), b"OK");

        // Verbatim \\?\C:\... canonicalized path inside fs_root must also succeed
        let verbatim_target = std::fs::canonicalize(&target).unwrap();
        let read_verbatim = bridge
            .handle_fs_rpc(
                "fs.readFile",
                &json!({ "path": verbatim_target.to_string_lossy() }),
            )
            .await
            .unwrap();
        assert_eq!(read_verbatim, json!("win-ok"));

        // Drive-relative and root-relative paths MUST be rejected with EACCES
        for bad in ["C:escape.txt", r"\outside.txt", "/outside.txt"] {
            let err = bridge
                .handle_fs_rpc("fs.readFile", &json!({ "path": bad }))
                .await
                .unwrap_err();
            assert!(
                err.starts_with("EACCES:"),
                "expected EACCES for {}, got {}",
                bad,
                err
            );
        }

        // NTFS directory junction inside fs_root pointing outside MUST be rejected with EACCES
        let junction = root.join("escape_junction");
        create_windows_junction(&junction, &outside);
        let err = bridge
            .handle_fs_rpc(
                "fs.writeFile",
                &json!({ "path": "escape_junction/pwned.txt", "data": "pwned" }),
            )
            .await
            .unwrap_err();
        assert!(err.starts_with("EACCES:"), "expected EACCES, got {}", err);
        assert!(!outside.join("pwned.txt").exists());
        let _ = std::fs::remove_dir(&junction);

        // Dangling NTFS directory junction pointing to non-existent outside dir MUST fail with EACCES
        let dangling_target = outside.join("nonexistent_dir");
        let dangling_junction = root.join("dangling_junction");
        create_windows_junction(&dangling_junction, &dangling_target);
        let err_str = bridge
            .handle_fs_rpc(
                "fs.writeFile",
                &json!({ "path": "dangling_junction/pwned.txt", "data": "pwned" }),
            )
            .await
            .unwrap_err();
        assert!(err_str.starts_with("EACCES:"), "expected EACCES, got {}", err_str);
        let err_bytes = bridge
            .handle_fs_rpc(
                "fs.writeFile",
                &json!({ "path": "dangling_junction/pwned.bin", "data": [1, 2, 3] }),
            )
            .await
            .unwrap_err();
        assert!(err_bytes.starts_with("EACCES:"), "expected EACCES, got {}", err_bytes);
        assert!(!dangling_target.exists());
        let _ = std::fs::remove_dir(&dangling_junction);

        // Cyclic NTFS junctions inside fs_root MUST fail cleanly with ELOOP
        let cyc_a = root.join("cyc_a");
        let cyc_b = root.join("cyc_b");
        create_windows_junction(&cyc_a, &cyc_b);
        create_windows_junction(&cyc_b, &cyc_a);
        let loop_err = bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": "cyc_a/test.txt" }))
            .await
            .unwrap_err();
        assert!(loop_err.starts_with("ELOOP:"), "expected ELOOP, got {}", loop_err);
        let _ = std::fs::remove_dir(&cyc_a);
        let _ = std::fs::remove_dir(&cyc_b);

        // Non-existent fs_root with different ASCII casing in caller path MUST succeed on Windows
        let future_root = base.join("FutureMixedCaseRoot");
        let future_bridge = LanguageFeatureBridge::new(
            Arc::new(ProviderRegistry::new()),
            Arc::new(RpcBridge::new()),
            Arc::new(LspClientManager::new()),
        )
        .with_fs_root(&future_root);
        let lower_future_file = future_root
            .join("hello.txt")
            .to_string_lossy()
            .to_ascii_lowercase();
        let future_wr = future_bridge
            .handle_fs_rpc(
                "fs.writeFile",
                &json!({ "path": lower_future_file, "data": "case-ok" }),
            )
            .await
            .unwrap();
        assert_eq!(future_wr["written"], json!(true));

        // openTextDocument matches RFC 8089 did_open_document via native Windows backslash path & lowercase drive
        let spaced = root.join("hello win.rs");
        std::fs::write(&spaced, "disk_version").unwrap();
        let rfc_spaced = rustcode_platform::path_to_file_uri(&spaced).replace(' ', "%20");
        bridge
            .did_open_document(&rfc_spaced, "rust", 1, "disk_version")
            .await;
        bridge
            .did_change_document(&rfc_spaced, "rust", 2, "DIRTY_WIN_BUFFER")
            .await;
        let doc_by_win_path = bridge
            .handle_rpc_message(
                "workspace.openTextDocument",
                &json!({ "path": spaced.to_string_lossy().to_ascii_lowercase() }),
            )
            .await
            .unwrap();
        assert_eq!(doc_by_win_path["isDirty"], true);
        assert_eq!(doc_by_win_path["text"], "DIRTY_WIN_BUFFER");

        // Opening an uncached disk file by Windows path produces an RFC 8089 file:///C:/... URI
        let clean_file = root.join("clean.rs");
        std::fs::write(&clean_file, "fn clean() {}\n").unwrap();
        let clean_doc = bridge
            .handle_rpc_message(
                "workspace.openTextDocument",
                &json!({ "path": clean_file.to_string_lossy() }),
            )
            .await
            .unwrap();
        assert_eq!(
            clean_doc["uri"].as_str().unwrap(),
            rustcode_platform::path_to_file_uri(&clean_file)
        );

        let _ = std::fs::remove_dir(&alias_parent);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_map_io_error_platform_codes() {
        #[cfg(windows)]
        {
            assert!(map_io_error(std::io::Error::from_raw_os_error(5)).starts_with("EACCES:"));
            assert!(map_io_error(std::io::Error::from_raw_os_error(32)).starts_with("EBUSY:"));
            assert!(map_io_error(std::io::Error::from_raw_os_error(123)).starts_with("EINVAL:"));
            assert!(map_io_error(std::io::Error::from_raw_os_error(145)).starts_with("ENOTEMPTY:"));
            assert!(map_io_error(std::io::Error::from_raw_os_error(183)).starts_with("EEXIST:"));
            assert!(map_io_error(std::io::Error::from_raw_os_error(267)).starts_with("ENOTDIR:"));
            assert!(map_io_error(std::io::Error::from_raw_os_error(1314)).starts_with("EPERM:"));
        }
        #[cfg(unix)]
        {
            assert!(map_io_error(std::io::Error::from_raw_os_error(1)).starts_with("EPERM:"));
            assert!(map_io_error(std::io::Error::from_raw_os_error(20)).starts_with("ENOTDIR:"));
            assert!(map_io_error(std::io::Error::from_raw_os_error(21)).starts_with("EISDIR:"));
        }
        assert!(
            map_io_error(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"))
                .starts_with("ENOENT:")
        );
    }

    #[tokio::test]
    async fn test_fs_rpc_extended_ops_and_register_command_notification() {
        let root = std::env::temp_dir().join(format!("rustcode_fs_ext_unit_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let reg = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let cmd_reg = CommandRegistry::new();
        let bridge = LanguageFeatureBridge::new(reg, rpc.clone(), lsp)
            .with_command_registry(cmd_reg.clone())
            .with_fs_root(&root);

        // 1. commands.registerCommand callback pushes notification to RpcBridge
        bridge
            .handle_rpc_message_result("commands.registerCommand", json!({ "command": "ext.queued" }))
            .await
            .unwrap();
        let _ = cmd_reg.execute("ext.queued", Some(json!(["arg1"]))).unwrap();
        let notifs = rpc.drain_notifications();
        assert_eq!(notifs.len(), 1);
        match &notifs[0] {
            crate::rpc::RpcMessage::Notification { channel, data } => {
                assert_eq!(channel, "commands.executeCommand");
                assert_eq!(data["command"], json!("ext.queued"));
                assert_eq!(data["args"], json!(["arg1"]));
            }
            other => panic!("Expected notification, got {:?}", other),
        }

        // 2. fs.mkdir firstCreated + readdir withFileTypes + access + copyFile + rename + rm
        let mk1 = bridge
            .handle_fs_rpc(
                "fs.mkdir",
                &json!({ "path": "a/b/c", "options": { "recursive": true } }),
            )
            .await
            .unwrap();
        assert_eq!(mk1["created"], json!(true));
        let first = mk1["firstCreated"].as_str().unwrap();
        assert!(first.ends_with("a"), "expected firstCreated ending in 'a', got {}", first);

        let mk2 = bridge
            .handle_fs_rpc(
                "fs.mkdir",
                &json!({ "path": "a/b/c", "options": { "recursive": true } }),
            )
            .await
            .unwrap();
        assert!(mk2["firstCreated"].is_null());

        bridge
            .handle_fs_rpc("fs.writeFile", &json!({ "path": "a/file.txt", "data": "hello" }))
            .await
            .unwrap();
        let entries = bridge
            .handle_fs_rpc("fs.readdir", &json!({ "path": "a", "withFileTypes": true }))
            .await
            .unwrap();
        let arr = entries.as_array().unwrap();
        assert_eq!(arr[0]["name"], json!("b"));
        assert_eq!(arr[0]["isDirectory"], json!(true));
        assert_eq!(arr[1]["name"], json!("file.txt"));
        assert_eq!(arr[1]["isFile"], json!(true));

        bridge
            .handle_fs_rpc("fs.access", &json!({ "path": "a/file.txt", "mode": 4 }))
            .await
            .unwrap();
        bridge
            .handle_fs_rpc("fs.copyFile", &json!({ "src": "a/file.txt", "dest": "a/copy.txt", "mode": 1 }))
            .await
            .unwrap();
        let excl_err = bridge
            .handle_fs_rpc("fs.copyFile", &json!({ "src": "a/file.txt", "dest": "a/copy.txt", "mode": 1 }))
            .await
            .unwrap_err();
        assert!(excl_err.starts_with("EEXIST:"), "got {}", excl_err);

        let dir_copy_err = bridge
            .handle_fs_rpc("fs.copyFile", &json!({ "src": "a/b", "dest": "a/b2" }))
            .await
            .unwrap_err();
        assert!(dir_copy_err.starts_with("EISDIR:"), "got {}", dir_copy_err);

        bridge
            .handle_fs_rpc("fs.rename", &json!({ "oldPath": "a/copy.txt", "newPath": "a/moved.txt" }))
            .await
            .unwrap();
        assert!(!root.join("a/copy.txt").exists());
        assert_eq!(std::fs::read_to_string(root.join("a/moved.txt")).unwrap(), "hello");

        let rm_dir_err = bridge
            .handle_fs_rpc("fs.rm", &json!({ "path": "a/b" }))
            .await
            .unwrap_err();
        assert!(rm_dir_err.starts_with("EISDIR:"), "got {}", rm_dir_err);

        bridge
            .handle_fs_rpc("fs.rm", &json!({ "path": "a", "options": { "recursive": true } }))
            .await
            .unwrap();
        assert!(!root.join("a").exists());

        bridge
            .handle_fs_rpc("fs.rm", &json!({ "path": "missing", "options": { "force": true } }))
            .await
            .unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn test_m1_2_bridge_workspace_window_commands_and_config_events() {
        let root = std::env::temp_dir().join(format!(
            "rustcode_m1_2_bridge_unit_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let sub = root.join("src");
        let nm = root.join("node_modules");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(&nm).unwrap();
        let file_a = sub.join("main.rs");
        let file_b = sub.join("lib.rs");
        let file_nm = nm.join("ignored.rs");
        std::fs::write(&file_a, "fn main() {}\r\n").unwrap();
        std::fs::write(&file_b, "pub fn lib() {}\n").unwrap();
        std::fs::write(&file_nm, "// ignored\n").unwrap();

        let cs = ConfigurationService::new();
        let cmd_reg = CommandRegistry::new();
        let _rust_echo = cmd_reg.register("rust.echo", |args| Ok(json!({ "args": args })));

        let bridge = Arc::new(
            LanguageFeatureBridge::with_defaults()
                .with_configuration_service(cs.clone())
                .with_command_registry(cmd_reg.clone())
                .with_fs_root(&root),
        );
        bridge.add_workspace_folder(&root, Some("Proj"));
        let ws_notifs = bridge.rpc_bridge().drain_notifications();
        assert!(ws_notifs.iter().any(|m| matches!(
            m,
            crate::rpc::RpcMessage::Notification { channel, .. }
                if channel == "workspace.onDidChangeWorkspaceFolders"
        )));

        // 1. workspace.getWorkspaceFolders & workspace.findFiles
        let folders_res = bridge
            .handle_rpc_message_result("workspace.getWorkspaceFolders", json!({}))
            .await
            .unwrap();
        assert_eq!(folders_res.as_array().unwrap().len(), 1);
        assert_eq!(folders_res[0]["name"], "Proj");

        let found_default = bridge
            .handle_rpc_message_result("workspace.findFiles", json!({ "include": "**/*.rs" }))
            .await
            .unwrap();
        let arr_def = found_default.as_array().unwrap();
        assert_eq!(arr_def.len(), 2, "node_modules should be excluded by default");

        let found_all = bridge
            .handle_rpc_message_result(
                "workspace.findFiles",
                json!({ "include": "**/*.rs", "exclude": Value::Null, "maxResults": 10 }),
            )
            .await
            .unwrap();
        assert_eq!(found_all.as_array().unwrap().len(), 3);

        // 2. ConfigurationService change emits workspace.onDidChangeConfiguration
        let _ = bridge.rpc_bridge().drain_notifications();
        cs.update(
            "editor.tabSize",
            json!(2),
            rustcode_platform::ConfigurationTarget::Workspace,
        )
        .unwrap();
        let cfg_notifs = bridge.rpc_bridge().drain_notifications();
        assert!(cfg_notifs.iter().any(|m| matches!(
            m,
            crate::rpc::RpcMessage::Notification { channel, data }
                if channel == "workspace.onDidChangeConfiguration"
                && data["affectedKeys"] == json!(["editor.tabSize"])
                && data["config"]["editor"]["tabSize"] == json!(2)
        )));

        // 3. openTextDocument + applyEdit + saveTextDocument + saveAll + did_* notifications
        let doc_a = bridge
            .handle_rpc_message_result(
                "workspace.openTextDocument",
                json!({ "path": file_a.to_string_lossy() }),
            )
            .await
            .unwrap();
        assert_eq!(doc_a["eol"], 2);
        let uri_a = doc_a["uri"].as_str().unwrap().to_string();

        let _ = bridge.rpc_bridge().drain_notifications();
        let mut changes = serde_json::Map::new();
        changes.insert(
            uri_a.clone(),
            json!([
                {
                    "range": {
                        "start": { "line": 0, "character": 3 },
                        "end": { "line": 0, "character": 7 }
                    },
                    "newText": "entry"
                }
            ]),
        );
        let applied = bridge
            .handle_rpc_message_result("workspace.applyEdit", json!({ "edit": { "changes": changes } }))
            .await
            .unwrap();
        assert_eq!(applied, json!(true));
        assert_eq!(bridge.take_pending_workspace_edits().len(), 1);

        let docs_list = bridge
            .handle_rpc_message_result("workspace.getTextDocuments", json!({}))
            .await
            .unwrap();
        assert_eq!(docs_list.as_array().unwrap().len(), 1);
        assert_eq!(docs_list[0]["text"], "fn entry() {}\r\n");
        assert_eq!(docs_list[0]["isDirty"], true);

        let saved = bridge
            .handle_rpc_message_result("workspace.saveTextDocument", json!({ "uri": uri_a }))
            .await
            .unwrap();
        assert_eq!(saved, json!(true));
        assert_eq!(
            std::fs::read_to_string(&file_a).unwrap(),
            "fn entry() {}\r\n"
        );

        // 4. window.showTextDocument + getActiveTextEditor + getVisibleTextEditors
        let ed = bridge
            .handle_rpc_message_result(
                "window.showTextDocument",
                json!({ "uri": uri_a, "viewColumn": 2 }),
            )
            .await
            .unwrap();
        assert_eq!(ed["viewColumn"], 2);
        assert_eq!(bridge.take_pending_show_document_requests().len(), 1);
        let active = bridge
            .handle_rpc_message_result("window.getActiveTextEditor", json!({}))
            .await
            .unwrap();
        assert_eq!(active["document"]["uri"], uri_a);
        let visible = bridge
            .handle_rpc_message_result("window.getVisibleTextEditors", json!({}))
            .await
            .unwrap();
        assert_eq!(visible.as_array().unwrap().len(), 1);

        // 5. treeView.onDidChangeTreeData -> ProviderRegistry
        bridge
            .handle_rpc_message_result(
                "treeView.onDidChangeTreeData",
                json!({ "providerId": "outline", "elementIds": ["n1"] }),
            )
            .await
            .unwrap();
        assert_eq!(bridge.provider_registry().tree_refresh_revision(), 1);
        assert_eq!(
            bridge.provider_registry().take_pending_tree_refreshes()[0].element_ids,
            vec!["n1".to_string()]
        );

        // 6. Empty command rejection & recursion guard on active_js_commands
        let empty_err = bridge
            .handle_rpc_message_result("commands.executeCommand", json!({ "command": "   " }))
            .await
            .unwrap_err();
        assert_eq!(empty_err.code, "INVALID_PARAMS");

        bridge
            .handle_rpc_message_result("commands.registerCommand", json!({ "command": "js.loop" }))
            .await
            .unwrap();
        assert!(bridge.is_js_command("js.loop"));
        bridge.active_js_commands.lock().insert("js.loop".to_string(), 8);
        let rec_err = bridge
            .request_execute_command("js.loop", vec![])
            .await
            .unwrap_err();
        assert!(rec_err.starts_with("COMMAND_RECURSION:"), "got {}", rec_err);
        bridge.active_js_commands.lock().remove("js.loop");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn test_m1_2_extract_scope_path_uri_to_json_and_folder_config_update() {
        // 1. Direct unit checks on extract_scope_path for serialized Uri.toJSON() & WorkspaceFolder
        let win_uri_json = json!({
            "scope": {
                "$mid": 1,
                "scheme": "file",
                "authority": "",
                "path": "/C:/Users/revel/ws/RootA",
                "query": "",
                "fragment": ""
            }
        });
        let extracted_win = extract_scope_path(&win_uri_json).expect("expected Windows Uri path");
        #[cfg(windows)]
        assert_eq!(extracted_win, PathBuf::from(r"C:\Users\revel\ws\RootA"));
        #[cfg(not(windows))]
        assert_eq!(extracted_win, PathBuf::from("/C:/Users/revel/ws/RootA"));

        let unc_uri_json = json!({
            "scope": {
                "$mid": 1,
                "scheme": "file",
                "authority": "fileserver",
                "path": "/share/RootA",
                "query": "",
                "fragment": ""
            }
        });
        let extracted_unc = extract_scope_path(&unc_uri_json).expect("expected UNC Uri path");
        #[cfg(windows)]
        assert_eq!(extracted_unc, PathBuf::from(r"\\fileserver\share\RootA"));
        #[cfg(not(windows))]
        assert_eq!(extracted_unc, PathBuf::from("fileserver/share/RootA"));

        let wf_obj = json!({
            "scope": {
                "uri": {
                    "$mid": 1,
                    "scheme": "file",
                    "authority": "",
                    "path": "/C:/Users/revel/ws/RootB",
                    "query": "",
                    "fragment": ""
                },
                "name": "RootB",
                "index": 1
            }
        });
        let extracted_wf = extract_scope_path(&wf_obj).expect("expected WorkspaceFolder path");
        #[cfg(windows)]
        assert_eq!(extracted_wf, PathBuf::from(r"C:\Users\revel\ws\RootB"));
        #[cfg(not(windows))]
        assert_eq!(extracted_wf, PathBuf::from("/C:/Users/revel/ws/RootB"));

        // Non-file schemes must NOT be treated as filesystem folder paths
        for non_file in [
            json!({ "scope": "untitled:Untitled-1" }),
            json!({ "scope": "https://example.com/RootA" }),
            json!({ "scope": { "$mid": 1, "scheme": "untitled", "authority": "", "path": "Untitled-1" } }),
            json!({ "scope": { "uri": { "$mid": 1, "scheme": "untitled", "authority": "", "path": "Untitled-1" } } }),
        ] {
            assert_eq!(
                extract_scope_path(&non_file),
                None,
                "non-file scope should return None: {:?}",
                non_file
            );
        }

        // 2. End-to-end bridge workspace.updateConfiguration with serialized Uri.toJSON() scope
        let root = std::env::temp_dir().join(format!(
            "rustcode_scope_uri_json_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let root_a = root.join("RootA");
        let root_b = root.join("RootB");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        let file_a = root_a.join("main.rs");
        let file_b = root_b.join("lib.rs");
        std::fs::write(&file_a, "fn a() {}\n").unwrap();
        std::fs::write(&file_b, "fn b() {}\n").unwrap();

        let cs = ConfigurationService::new();
        let bridge = LanguageFeatureBridge::with_defaults().with_configuration_service(cs.clone());
        bridge.add_workspace_folder(&root_a, Some("RootA"));
        bridge.add_workspace_folder(&root_b, Some("RootB"));

        let root_a_uri_path = {
            let s = root_a.to_string_lossy().replace('\\', "/");
            if s.starts_with('/') {
                s
            } else {
                format!("/{}", s)
            }
        };
        bridge
            .handle_rpc_message_result(
                "workspace.updateConfiguration",
                &json!({
                    "key": "editor.tabSize",
                    "value": 6,
                    "target": 3,
                    "scope": {
                        "$mid": 1,
                        "scheme": "file",
                        "authority": "",
                        "path": root_a_uri_path,
                        "query": "",
                        "fragment": ""
                    }
                }),
            )
            .await
            .unwrap();

        assert_eq!(cs.get_for_resource::<i64>("editor.tabSize", Some(&file_a)), Some(6));
        assert_eq!(cs.get_for_resource::<i64>("editor.tabSize", Some(&file_b)), Some(4));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_m1_2_concurrent_command_register_unregister_stays_atomic() {
        let bridge = Arc::new(LanguageFeatureBridge::with_defaults());
        let cmd = "concurrent.atomic.cmd";

        let mut handles = Vec::new();
        for worker in 0..4 {
            let b = Arc::clone(&bridge);
            handles.push(tokio::spawn(async move {
                for i in 0..150 {
                    if (worker + i) % 2 == 0 {
                        b.handle_rpc_message_result(
                            "commands.registerCommand",
                            &json!({ "command": cmd }),
                        )
                        .await
                        .unwrap();
                    } else {
                        b.handle_rpc_message_result(
                            "commands.unregisterCommand",
                            &json!({ "command": cmd }),
                        )
                        .await
                        .unwrap();
                    }
                    // Under concurrent register/unregister, getCommands & is_js_command
                    // must remain valid and executeCommand must only return COMMAND_RECURSION
                    // or COMMAND_NOT_FOUND (never COMMAND_EXECUTION_FAILED or unexpected panic).
                    match b
                        .handle_rpc_message_result(
                            "commands.executeCommand",
                            &json!({ "command": cmd }),
                        )
                        .await
                    {
                        Err(e) => {
                            assert!(
                                e.code == "COMMAND_RECURSION" || e.code == "COMMAND_NOT_FOUND",
                                "unexpected error code during concurrent command mutation: {:?}",
                                e
                            );
                        }
                        Ok(_) => {
                            // Allowed only in the tiny window between js_commands check and
                            // command_registry.execute if a concurrent register just inserted.
                        }
                    }
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        // After quiescence, js_commands, command_disposables, and command_registry must agree.
        let is_js = bridge.is_js_command(cmd);
        let in_registry = bridge.command_registry().has_command(cmd);
        assert_eq!(
            is_js, in_registry,
            "split-brain between js_commands ({}) and command_registry ({})",
            is_js, in_registry
        );

        // Unregistering cleans up everything cleanly.
        bridge
            .handle_rpc_message_result("commands.unregisterCommand", &json!({ "command": cmd }))
            .await
            .unwrap();
        assert!(!bridge.is_js_command(cmd));
        assert!(!bridge.command_registry().has_command(cmd));
        assert!(bridge.list_js_commands().is_empty());
    }
}
