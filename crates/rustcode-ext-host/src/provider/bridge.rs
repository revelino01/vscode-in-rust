//! Language feature bridge coordinating LSP clients and JavaScript extension providers.

use super::{ProviderRegistry, ProviderType};
use crate::rpc::RpcBridge;
use parking_lot::{Mutex, RwLock};
use rustcode_protocols::lsp::{
    CodeAction, CodeActionContext, CodeActionKind, CompletionContext, CompletionTriggerKind,
    FormattingOptions, Hover, LspClientManager, LspCommand, LspCompletionItem, LspDiagnostic,
    LspLocation, LspPosition, LspRange, LspTextEdit, MarkupContent,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TreeItemCollapsibleState {
    #[default]
    None = 0,
    Collapsed = 1,
    Expanded = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeItem {
    pub id: Option<String>,
    pub label: String,
    pub description: Option<String>,
    pub tooltip: Option<String>,
    #[serde(rename = "collapsibleState", default)]
    pub collapsible_state: TreeItemCollapsibleState,
    #[serde(rename = "iconPath")]
    pub icon_path: Option<String>,
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

pub struct LanguageFeatureBridge {
    pub provider_registry: Arc<ProviderRegistry>,
    pub rpc_bridge: Arc<RpcBridge>,
    pub lsp_manager: Arc<LspClientManager>,
    pub diagnostic_manager: Arc<DiagnosticCollectionManager>,
    pub status_bar_manager: Arc<ExtStatusBarManager>,
    pub notebook_manager: Arc<crate::notebook::ExtHostNotebookManager>,
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
        let _ = self
            .lsp_manager
            .did_open_document(uri, language_id, version, text)
            .await;
    }

    pub async fn did_change_document(&self, uri: &str, language_id: &str, version: i32, text: &str) {
        let _ = self
            .lsp_manager
            .did_change_document(uri, language_id, version, text)
            .await;
    }

    pub async fn did_close_document(&self, uri: &str, language_id: &str) {
        let _ = self.lsp_manager.did_close_document(uri, language_id).await;
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
}
