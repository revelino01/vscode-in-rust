//! Language feature bridge coordinating LSP clients and JavaScript extension providers.

use super::{ProviderRegistry, ProviderType};
use crate::rpc::RpcBridge;
use parking_lot::{Mutex, RwLock};
use rustcode_base::disposable::Disposable;
use rustcode_platform::{
    CommandRegistry, ConfigurationService, ConfigurationTarget, FileChangeEvent, FileChangeType,
    FileSystemError, FileSystemService,
};
use rustcode_protocols::lsp::{
    CodeAction, CodeActionContext, CodeActionKind, CompletionContext, CompletionTriggerKind,
    FormattingOptions, Hover, LspClientManager, LspCommand, LspCompletionItem, LspDiagnostic,
    LspLocation, LspPosition, LspRange, LspTextEdit, MarkupContent,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

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
        match method {
            "languages.setDiagnostics" => {
                if let (Some(name), Some(uri), Some(diags_val)) = (
                    params.get("name").and_then(|n| n.as_str()),
                    params.get("uri").and_then(|u| u.as_str()),
                    params.get("diagnostics"),
                ) {
                    if let Ok(diags) = serde_json::from_value::<Vec<LspDiagnostic>>(diags_val.clone()) {
                        self.set_diagnostics(name, uri, diags);
                        return true;
                    }
                }
                false
            }
            "languages.clearDiagnostics" => {
                if let (Some(name), Some(uri)) = (
                    params.get("name").and_then(|n| n.as_str()),
                    params.get("uri").and_then(|u| u.as_str()),
                ) {
                    self.clear_diagnostics(name, uri);
                    return true;
                }
                false
            }
            "languages.clearAllDiagnostics" => {
                if let Some(name) = params.get("name").and_then(|n| n.as_str()) {
                    self.clear_all_in_collection(name);
                    return true;
                }
                false
            }
            _ => false,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(rename = "contextValue", alias = "context_value")]
    pub context_value: Option<String>,
    pub command: Option<LspCommand>,
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
}

impl ExtStatusBarManager {
    pub fn new() -> Self {
        Self {
            items: RwLock::new(HashMap::new()),
        }
    }

    pub fn set_item(&self, item: ExtStatusBarItem) {
        self.items.write().insert(item.id.clone(), item);
    }

    pub fn hide_item(&self, id: &str) {
        if let Some(item) = self.items.write().get_mut(id) {
            item.visible = false;
        }
    }

    pub fn remove_item(&self, id: &str) {
        self.items.write().remove(id);
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
        match method {
            "window.setStatusBarItem" => {
                if let Some(id) = params.get("id").and_then(|v| v.as_str()) {
                    let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let tooltip = params.get("tooltip").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let command = params.get("command").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let alignment = params.get("alignment").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
                    let priority = params.get("priority").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    self.set_item(ExtStatusBarItem {
                        id: id.to_string(),
                        text,
                        tooltip,
                        command,
                        alignment,
                        priority,
                        visible: true,
                    });
                    true
                } else {
                    false
                }
            }
            "window.hideStatusBarItem" => {
                if let Some(id) = params.get("id").and_then(|v| v.as_str()) {
                    self.hide_item(id);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
struct OpenDocumentSnapshot {
    uri: String,
    file_name: String,
    language_id: String,
    version: i32,
    text: String,
    is_dirty: bool,
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
    pub window_manager: Arc<crate::window::ExtHostWindowManager>,
    pub env_manager: Arc<crate::env::ExtHostEnvManager>,
    pub configuration_service: ConfigurationService,
    pub command_registry: CommandRegistry,
    pub file_system_service: Arc<FileSystemService>,
    fs_root: Option<PathBuf>,
    open_documents: Arc<RwLock<HashMap<String, OpenDocumentSnapshot>>>,
    command_disposables: Arc<Mutex<HashMap<String, Box<dyn Disposable>>>>,
    js_commands: Arc<RwLock<HashSet<String>>>,
}

impl LanguageFeatureBridge {
    pub fn new(
        provider_registry: Arc<ProviderRegistry>,
        rpc_bridge: Arc<RpcBridge>,
        lsp_manager: Arc<LspClientManager>,
    ) -> Self {
        Self {
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
            window_manager: Arc::new(crate::window::ExtHostWindowManager::new()),
            env_manager: Arc::new(crate::env::ExtHostEnvManager::new()),
            configuration_service: ConfigurationService::new(),
            command_registry: CommandRegistry::new(),
            file_system_service: Arc::new(FileSystemService::new()),
            fs_root: None,
            open_documents: Arc::new(RwLock::new(HashMap::new())),
            command_disposables: Arc::new(Mutex::new(HashMap::new())),
            js_commands: Arc::new(RwLock::new(HashSet::new())),
        }
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

    pub fn with_configuration_service(mut self, cs: ConfigurationService) -> Self {
        self.configuration_service = cs;
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
            let (_req, rx) = self.rpc_bridge.create_request(
                "languages.provideCompletionItems",
                json!({
                    "providerId": provider.id,
                    "uri": uri,
                    "position": position,
                    "triggerCharacter": trigger_char.map(|c| c.to_string()),
                }),
            );

            // If a response arrives within 250ms, incorporate items
            if let Ok(Ok(Ok(val))) = tokio::time::timeout(std::time::Duration::from_millis(250), rx).await {
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
            let (_req, rx) = self.rpc_bridge.create_request(
                "languages.provideHover",
                json!({
                    "providerId": provider.id,
                    "uri": uri,
                    "position": position,
                }),
            );

            if let Ok(Ok(Ok(val))) = tokio::time::timeout(std::time::Duration::from_millis(250), rx).await {
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
            let (_req, rx) = self.rpc_bridge.create_request(
                "languages.provideDefinition",
                json!({
                    "providerId": provider.id,
                    "uri": uri,
                    "position": position,
                }),
            );

            if let Ok(Ok(Ok(val))) = tokio::time::timeout(std::time::Duration::from_millis(250), rx).await {
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
            let (_req, rx) = self.rpc_bridge.create_request(
                "languages.provideDocumentFormattingEdits",
                json!({
                    "providerId": provider.id,
                    "uri": uri,
                    "options": options,
                }),
            );

            if let Ok(Ok(Ok(val))) = tokio::time::timeout(std::time::Duration::from_millis(500), rx).await {
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
            let (_req, rx) = self.rpc_bridge.create_request(
                "languages.provideCodeActions",
                json!({
                    "providerId": provider.id,
                    "uri": uri,
                    "range": range,
                    "context": context,
                }),
            );

            if let Ok(Ok(Ok(val))) = tokio::time::timeout(std::time::Duration::from_millis(500), rx).await {
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

    /// Request hierarchical tree items for a tree data provider.
    pub async fn request_tree_children(
        &self,
        provider_id: &str,
        element_id: Option<&str>,
    ) -> Vec<TreeItem> {
        let (_req, rx) = self.rpc_bridge.create_request(
            "treeView.getChildren",
            json!({
                "providerId": provider_id,
                "element": element_id,
            }),
        );

        if let Ok(Ok(Ok(val))) = tokio::time::timeout(std::time::Duration::from_millis(500), rx).await {
            if let Ok(items) = serde_json::from_value::<Vec<TreeItem>>(val) {
                return items;
            }
        }

        Vec::new()
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
        if self.js_commands.read().contains(command) {
            let (_req, rx) = self.rpc_bridge.create_request(
                "commands.executeCommand",
                json!({ "command": command, "args": args }),
            );
            match tokio::time::timeout(std::time::Duration::from_millis(500), rx).await {
                Ok(Ok(Ok(val))) => Ok(val),
                Ok(Ok(Err(err))) => Err(err),
                Ok(Err(_)) => Err("RPC response channel closed".to_string()),
                Err(_) => Err(format!("Timed out executing JS command '{}'", command)),
            }
        } else {
            let arg_val = if args.is_empty() {
                None
            } else {
                Some(Value::Array(args))
            };
            self.command_registry.execute(command, arg_val)
        }
    }

    /// Handle incoming RPC calls from JS runtime (e.g. provider registration, format, codeAction).
    pub async fn handle_rpc_message(
        &self,
        method: &str,
        params: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        if self.diagnostic_manager.handle_rpc_message(method, params) {
            return Some(json!({ "success": true }));
        }

        if self.status_bar_manager.handle_rpc_message(method, params) {
            return Some(json!({ "success": true }));
        }

        if method.starts_with("notebooks.") {
            return self.notebook_manager.handle_rpc_call(method, params).ok();
        }

        if method.starts_with("scm.") {
            return self.scm_manager.handle_rpc_call(method, params).ok();
        }

        if method.starts_with("debug.") {
            return self.debug_manager.handle_rpc_call(method, params).ok();
        }

        if method.starts_with("tasks.") {
            return self.task_manager.handle_rpc_call(method, params).ok();
        }

        if method.starts_with("tests.") {
            return self.test_manager.handle_rpc_call(method, params).ok();
        }

        if method.starts_with("authentication.") {
            return self.auth_manager.handle_rpc_call(method, params).ok();
        }

        if method.starts_with("env.") || method.starts_with("extensions.") {
            return self.env_manager.handle_rpc_call(method, params).ok();
        }

        if method.starts_with("workspace.") {
            return self.handle_workspace_rpc(method, params).await;
        }

        if method.starts_with("commands.") {
            return self.handle_commands_rpc(method, params);
        }

        if method.starts_with("fs.") {
            return self.handle_fs_rpc(method, params).await.ok();
        }

        match method {
            "window.showInformationMessage"
            | "window.showWarningMessage"
            | "window.showErrorMessage"
            | "window.showQuickPick"
            | "window.showInputBox" => {
                return self.window_manager.handle_rpc_call(method, params).await.ok();
            }
            "window.createOutputChannel"
            | "window.appendOutputChannel"
            | "window.replaceOutputChannel"
            | "window.clearOutputChannel"
            | "window.showOutputChannel"
            | "window.hideOutputChannel"
            | "window.disposeOutputChannel" => {
                return self.output_manager.handle_rpc_call(method, params).ok();
            }
            "window.createTerminal"
            | "window.showTerminal"
            | "window.sendTerminalText"
            | "window.disposeTerminal" => {
                return self.terminal_manager.handle_rpc_call(method, params).ok();
            }
            "window.createWebviewPanel"
            | "window.setWebviewHtml"
            | "window.postMessageToWebview"
            | "window.disposeWebviewPanel"
            | "window.revealWebviewPanel" => {
                return self
                    .webview_manager
                    .handle_rpc_call(method, params.clone())
                    .ok();
            }
            "window.registerCustomEditorProvider"
            | "window.unregisterCustomEditorProvider"
            | "window.openCustomEditor" => {
                return self
                    .custom_editor_manager
                    .handle_rpc_call(method, params, Some(&self.webview_manager))
                    .ok();
            }
            _ => {}
        }

        match method {
            "extHost.registerProvider" => {
                let type_str = params.get("type").and_then(|t| t.as_str())?;
                let id = params.get("id").and_then(|i| i.as_str())?;
                let selector = params.get("selector").and_then(|s| s.as_str()).map(|s| s.to_string());

                if let Some(provider_type) = ProviderType::from_str_name(type_str) {
                    self.provider_registry.register(super::ProviderRegistration {
                        id: id.to_string(),
                        provider_type,
                        language_selector: selector,
                    });
                    Some(json!({ "registered": true, "id": id }))
                } else {
                    None
                }
            }
            "extHost.unregisterProvider" => {
                let id = params.get("id").and_then(|i| i.as_str())?;
                self.provider_registry.unregister(id);
                Some(json!({ "unregistered": true, "id": id }))
            }
            "languages.format" => {
                let uri = params.get("uri").and_then(|u| u.as_str()).unwrap_or("");
                let language_id = params.get("languageId").and_then(|l| l.as_str()).unwrap_or("*");
                let options: FormattingOptions = params
                    .get("options")
                    .and_then(|o| serde_json::from_value(o.clone()).ok())
                    .unwrap_or_else(|| FormattingOptions::new(4, true));

                let edits = self.request_formatting(uri, language_id, options).await;
                Some(serde_json::to_value(edits).unwrap_or(json!([])))
            }
            "languages.codeAction" => {
                let uri = params.get("uri").and_then(|u| u.as_str()).unwrap_or("");
                let language_id = params.get("languageId").and_then(|l| l.as_str()).unwrap_or("*");
                let range: LspRange = params
                    .get("range")
                    .and_then(|r| serde_json::from_value(r.clone()).ok())
                    .unwrap_or_else(|| LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 0)));
                let context: CodeActionContext = params
                    .get("context")
                    .and_then(|c| serde_json::from_value(c.clone()).ok())
                    .unwrap_or_default();

                let actions = self.request_code_actions(uri, language_id, range, context).await;
                Some(serde_json::to_value(actions).unwrap_or(json!([])))
            }
            _ => None,
        }
    }

    // --- Document Lifecycle Forwarding ---

    pub async fn did_open_document(&self, uri: &str, language_id: &str, version: i32, text: &str) {
        let file_name = decode_file_uri(uri).to_string_lossy().into_owned();
        let snap = OpenDocumentSnapshot {
            uri: uri.to_string(),
            file_name,
            language_id: language_id.to_string(),
            version,
            text: text.to_string(),
            is_dirty: false,
        };
        self.open_documents.write().insert(uri.to_string(), snap);
        let _ = self
            .lsp_manager
            .did_open_document(uri, language_id, version, text)
            .await;
    }

    pub async fn did_change_document(&self, uri: &str, language_id: &str, version: i32, text: &str) {
        {
            let mut docs = self.open_documents.write();
            if let Some(existing) = docs.get_mut(uri) {
                existing.language_id = language_id.to_string();
                existing.version = version;
                existing.text = text.to_string();
                existing.is_dirty = true;
            } else {
                let file_name = decode_file_uri(uri).to_string_lossy().into_owned();
                docs.insert(
                    uri.to_string(),
                    OpenDocumentSnapshot {
                        uri: uri.to_string(),
                        file_name,
                        language_id: language_id.to_string(),
                        version,
                        text: text.to_string(),
                        is_dirty: true,
                    },
                );
            }
        }
        let _ = self
            .lsp_manager
            .did_change_document(uri, language_id, version, text)
            .await;
    }

    pub async fn did_close_document(&self, uri: &str, language_id: &str) {
        self.open_documents.write().remove(uri);
        let _ = self.lsp_manager.did_close_document(uri, language_id).await;
    }

    // --- Workspace, Commands, and Node-Compat FS RPC Handlers ---

    async fn handle_workspace_rpc(&self, method: &str, params: &Value) -> Option<Value> {
        match method {
            "workspace.getConfiguration" => {
                let section = params.get("section").and_then(|v| v.as_str()).unwrap_or("");
                let scope_path = extract_scope_path(params);
                let merged = if scope_path.is_some() {
                    self.configuration_service
                        .get_merged_for_resource(scope_path.as_deref())
                } else {
                    self.configuration_service.get_all()
                };
                if section.is_empty() {
                    Some(merged)
                } else {
                    Some(traverse_config_value(&merged, section).unwrap_or_else(|| json!({})))
                }
            }
            "workspace.inspectConfiguration" => {
                let key = params
                    .get("key")
                    .or_else(|| params.get("section"))
                    .and_then(|v| v.as_str())?;
                let scope_path = extract_scope_path(params);
                let inspection = self
                    .configuration_service
                    .inspect(key, scope_path.as_deref());
                serde_json::to_value(inspection).ok()
            }
            "workspace.updateConfiguration" => {
                let key = params.get("key").and_then(|v| v.as_str())?.trim();
                if key.is_empty() {
                    return None;
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
                        _ => return None,
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
                        _ => return None,
                    },
                    _ => return None,
                };

                match params.get("value") {
                    None => {
                        self.configuration_service.reset(key, target);
                        Some(json!({ "updated": true, "reset": true, "key": key }))
                    }
                    Some(val) => {
                        self.configuration_service
                            .update(key, val.clone(), target)
                            .ok()?;
                        Some(json!({ "updated": true, "key": key }))
                    }
                }
            }
            "workspace.openTextDocument" => {
                let raw_uri_opt = params
                    .get("uri")
                    .or_else(|| params.get("fileName"))
                    .or_else(|| params.get("path"))
                    .and_then(|v| v.as_str());

                let (uri, file_name, language_id, version, is_dirty, is_untitled, text) =
                    if raw_uri_opt.is_none()
                        || raw_uri_opt.is_some_and(|u| u.starts_with("untitled:"))
                    {
                        let open_snap = raw_uri_opt.and_then(|raw_u| {
                            let target_name = strip_untitled_prefix(raw_u).unwrap_or("Untitled-1");
                            let canonical_untitled = format!("untitled:{}", target_name);
                            let slash_untitled = format!("untitled://{}", target_name);
                            let docs = self.open_documents.read();
                            docs.get(raw_u)
                                .or_else(|| docs.get(&canonical_untitled))
                                .or_else(|| docs.get(&slash_untitled))
                                .or_else(|| {
                                    docs.values().find(|snap| {
                                        strip_untitled_prefix(&snap.uri) == Some(target_name)
                                    })
                                })
                                .cloned()
                        });

                        if let Some(snap) = open_snap {
                            let clean_name = strip_untitled_prefix(&snap.uri)
                                .unwrap_or(&snap.file_name)
                                .to_string();
                            (
                                snap.uri,
                                clean_name,
                                snap.language_id,
                                snap.version,
                                snap.is_dirty,
                                true,
                                snap.text,
                            )
                        } else {
                            let uri = raw_uri_opt
                                .map(ToString::to_string)
                                .unwrap_or_else(|| "untitled:Untitled-1".to_string());
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
                            (uri, file_name, language_id, 1, false, true, text)
                        }
                    } else {
                        let raw = raw_uri_opt?;
                        let path = decode_file_uri(raw);
                        let path_str = path.to_string_lossy();
                        let canonical_uri = if raw.starts_with("file://") {
                            raw.to_string()
                        } else {
                            format!("file://{}", path_str)
                        };
                        let alt_uri = format!("file://{}", path_str);

                        let open_snap = {
                            let docs = self.open_documents.read();
                            docs.get(&canonical_uri)
                                .or_else(|| docs.get(&alt_uri))
                                .or_else(|| docs.get(raw))
                                .or_else(|| {
                                    docs.values()
                                        .find(|snap| snap.file_name == path_str.as_ref())
                                })
                                .cloned()
                        };

                        if let Some(snap) = open_snap {
                            (
                                snap.uri,
                                snap.file_name,
                                snap.language_id,
                                snap.version,
                                snap.is_dirty,
                                false,
                                snap.text,
                            )
                        } else {
                            let text = FileSystemService::read_to_string(&path).await.ok()?;
                            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                            let language_id = map_ext_to_lang_id(ext);
                            (
                                canonical_uri,
                                path.to_string_lossy().into_owned(),
                                language_id,
                                1,
                                false,
                                false,
                                text,
                            )
                        }
                    };

                let line_count = text.split('\n').count();
                let eol = if text.contains("\r\n") { 2 } else { 1 };
                Some(json!({
                    "uri": uri,
                    "fileName": file_name,
                    "languageId": language_id,
                    "version": version,
                    "isDirty": is_dirty,
                    "isUntitled": is_untitled,
                    "eol": eol,
                    "text": text,
                    "lineCount": line_count,
                }))
            }
            _ => None,
        }
    }

    fn handle_commands_rpc(&self, method: &str, params: &Value) -> Option<Value> {
        match method {
            "commands.registerCommand" => {
                let command = params.get("command").and_then(|v| v.as_str())?.trim();
                if command.is_empty() {
                    return None;
                }
                let cmd_id = command.to_string();

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
                    let _notif = rpc.create_notification(
                        "commands.executeCommand",
                        json!({ "command": cb_cmd, "args": args_arr }),
                    );
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

                Some(json!({ "registered": true, "command": cmd_id }))
            }
            "commands.unregisterCommand" => {
                let command = params.get("command").and_then(|v| v.as_str())?;
                let was_js = self.js_commands.write().remove(command);
                let old_disp = self.command_disposables.lock().remove(command);
                let had_disp = old_disp.is_some();
                if let Some(mut disp) = old_disp {
                    disp.dispose();
                }
                Some(json!({
                    "unregistered": was_js || had_disp,
                    "command": command,
                }))
            }
            "commands.executeCommand" => {
                let command = params.get("command").and_then(|v| v.as_str())?;
                // Infinite recursion guard: never bounce JS-owned commands back to JS.
                if self.js_commands.read().contains(command) {
                    return None;
                }
                let arg_val = match params.get("args") {
                    None | Some(Value::Null) => None,
                    Some(Value::Array(arr)) if arr.is_empty() => None,
                    Some(other) => Some(other.clone()),
                };
                self.command_registry.execute(command, arg_val).ok()
            }
            "commands.getCommands" => {
                let filter_internal = params
                    .get("filterInternal")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let mut cmds = self.command_registry.list_commands();
                if filter_internal {
                    cmds.retain(|c| !c.starts_with('_'));
                }
                cmds.sort();
                Some(json!(cmds))
            }
            _ => None,
        }
    }

    pub async fn handle_fs_rpc(&self, method: &str, params: &Value) -> Result<Value, String> {
        let raw_path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "EINVAL: Missing string 'path' parameter".to_string())?;
        let path = self.resolve_fs_path(raw_path)?;

        match method {
            "fs.readFile" => {
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
                                .is_ok_and(|m| m.file_type().is_symlink())
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
                let entries = std::fs::read_dir(&path).map_err(map_io_error)?;
                let mut names = Vec::new();
                for entry in entries {
                    let entry = entry.map_err(map_io_error)?;
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
                names.sort();
                Ok(json!(names))
            }
            "fs.stat" => {
                let meta = std::fs::metadata(&path).map_err(map_io_error)?;
                let sym_meta = std::fs::symlink_metadata(&path).ok();
                let is_symlink = sym_meta
                    .as_ref()
                    .is_some_and(|m| m.file_type().is_symlink());
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
                let recursive = params
                    .get("options")
                    .and_then(|o| {
                        o.as_bool()
                            .or_else(|| o.get("recursive").and_then(|r| r.as_bool()))
                    })
                    .or_else(|| params.get("recursive").and_then(|r| r.as_bool()))
                    .unwrap_or(false);
                if recursive {
                    std::fs::create_dir_all(&path).map_err(map_io_error)?;
                } else {
                    std::fs::create_dir(&path).map_err(map_io_error)?;
                }
                Ok(json!({
                    "created": true,
                    "path": path.to_string_lossy().into_owned(),
                }))
            }
            "fs.unlink" => {
                std::fs::remove_file(&path).map_err(map_io_error)?;
                Ok(json!({
                    "unlinked": true,
                    "path": path.to_string_lossy().into_owned(),
                }))
            }
            _ => Err(format!("EINVAL: Unsupported fs RPC method: {}", method)),
        }
    }

    fn resolve_fs_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        if raw_path.trim().is_empty() || raw_path.contains('\0') {
            return Err("EINVAL: Invalid empty or NUL path".to_string());
        }
        let path = if raw_path.starts_with("file://") {
            decode_file_uri(raw_path)
        } else {
            PathBuf::from(raw_path)
        };
        if path.as_os_str().is_empty() || path.to_string_lossy().contains('\0') {
            return Err("EINVAL: Invalid empty or NUL path".to_string());
        }

        if let Some(root) = &self.fs_root {
            let abs_root = if root.is_absolute() {
                root.to_path_buf()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(root))
                    .unwrap_or_else(|_| root.to_path_buf())
            };
            let canon_root = canonicalize_existing_prefix(&abs_root);
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
}

static TEMP_FILE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn strip_untitled_prefix(uri: &str) -> Option<&str> {
    uri.strip_prefix("untitled://")
        .or_else(|| uri.strip_prefix("untitled:"))
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

fn canonicalize_existing_prefix(path: &Path) -> PathBuf {
    let normalized = lexical_normalize(path);
    let mut suffix = Vec::new();
    let mut cur = normalized.as_path();
    loop {
        if std::fs::symlink_metadata(cur).is_ok() {
            if let Ok(mut canon) = std::fs::canonicalize(cur) {
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
            PathStep::RootDir => cur.push(Component::RootDir.as_os_str()),
            PathStep::ParentDir => {
                cur.pop();
            }
            PathStep::Normal(seg) => {
                let next = cur.join(&seg);
                match std::fs::symlink_metadata(&next) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        *symlinks_followed += 1;
                        if *symlinks_followed > MAX_SYMLINKS {
                            return Err("ELOOP: Too many levels of symbolic links".to_string());
                        }
                        let target = std::fs::read_link(&next).map_err(map_io_error)?;
                        if cur.starts_with(canon_root) {
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
                            if !resolved_target.starts_with(canon_root) {
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
                        cur = next;
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

    if !cur.starts_with(canon_root) {
        return Err(format!(
            "EACCES: Path '{}' escapes fs_root '{}'",
            cur.display(),
            canon_root.display()
        ));
    }
    Ok(cur)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h1 = (bytes[i + 1] as char).to_digit(16);
            let h2 = (bytes[i + 2] as char).to_digit(16);
            if let (Some(d1), Some(d2)) = (h1, h2) {
                out.push(((d1 << 4) | d2) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn decode_file_uri(uri_or_path: &str) -> PathBuf {
    if let Some(rest) = uri_or_path.strip_prefix("file://localhost") {
        PathBuf::from(percent_decode(rest))
    } else if let Some(rest) = uri_or_path.strip_prefix("file://") {
        PathBuf::from(percent_decode(rest))
    } else {
        PathBuf::from(uri_or_path)
    }
}

fn extract_scope_path(params: &Value) -> Option<PathBuf> {
    let scope = params.get("scope").or_else(|| params.get("folder"))?;
    match scope {
        Value::String(s) if !s.trim().is_empty() => Some(decode_file_uri(s)),
        Value::Object(obj) => {
            if let Some(fs_path) = obj.get("fsPath").and_then(|v| v.as_str()) {
                if !fs_path.trim().is_empty() {
                    return Some(decode_file_uri(fs_path));
                }
            }
            if let Some(path) = obj.get("path").and_then(|v| v.as_str()) {
                if !path.trim().is_empty() {
                    return Some(decode_file_uri(path));
                }
            }
            if let Some(uri) = obj.get("uri") {
                if let Some(s) = uri.as_str() {
                    if !s.trim().is_empty() {
                        return Some(decode_file_uri(s));
                    }
                } else if let Some(fs_path) = uri
                    .get("fsPath")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| {
                        uri.get("path")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                    })
                {
                    return Some(decode_file_uri(fs_path));
                }
            }
            None
        }
        _ => None,
    }
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

fn map_ext_to_lang_id(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "rs" => "rust".to_string(),
        "js" | "jsx" | "mjs" | "cjs" => "javascript".to_string(),
        "ts" | "tsx" | "mts" | "cts" => "typescript".to_string(),
        "py" | "pyw" | "pyi" => "python".to_string(),
        "json" | "jsonc" => "json".to_string(),
        "md" | "markdown" => "markdown".to_string(),
        "go" => "go".to_string(),
        "c" | "h" => "c".to_string(),
        "cpp" | "cc" | "cxx" | "hpp" => "cpp".to_string(),
        "html" | "htm" => "html".to_string(),
        "css" | "scss" | "less" => "css".to_string(),
        "toml" => "toml".to_string(),
        "yaml" | "yml" => "yaml".to_string(),
        "sh" | "bash" => "shellscript".to_string(),
        "" => "plaintext".to_string(),
        other => other.to_string(),
    }
}

fn map_io_error(err: std::io::Error) -> String {
    let code = match err.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::AlreadyExists => "EEXIST",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::InvalidInput => "EINVAL",
        _ => match err.raw_os_error() {
            Some(21) => "EISDIR",
            Some(20) => "ENOTDIR",
            Some(1) => "EPERM",
            _ => "EIO",
        },
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
}
