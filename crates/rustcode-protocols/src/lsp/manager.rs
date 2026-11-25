//! LSP Client Manager orchestrating multiple language server processes.

use super::client::{LspClient, LspError};
use super::types::*;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct LspServerConfig {
    pub language_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub trigger_characters: Vec<char>,
}

impl LspServerConfig {
    pub fn new(
        language_id: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
        trigger_characters: Vec<char>,
    ) -> Self {
        Self {
            language_id: language_id.into(),
            command: command.into(),
            args,
            trigger_characters,
        }
    }
}

pub struct LspClientManager {
    configs: RwLock<HashMap<String, LspServerConfig>>,
    clients: RwLock<HashMap<String, Arc<LspClient>>>,
    failed_languages: RwLock<HashMap<String, String>>,
    start_lock: tokio::sync::Mutex<()>,
}

impl Default for LspClientManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LspClientManager {
    pub fn new() -> Self {
        let mut configs = HashMap::new();

        // Standard default configurations
        configs.insert(
            "rust".to_string(),
            LspServerConfig {
                language_id: "rust".to_string(),
                command: "rust-analyzer".to_string(),
                args: Vec::new(),
                trigger_characters: vec!['.', ':'],
            },
        );

        configs.insert(
            "typescript".to_string(),
            LspServerConfig {
                language_id: "typescript".to_string(),
                command: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                trigger_characters: vec!['.', '/', '@', '<'],
            },
        );

        configs.insert(
            "javascript".to_string(),
            LspServerConfig {
                language_id: "javascript".to_string(),
                command: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                trigger_characters: vec!['.', '/', '@'],
            },
        );

        configs.insert(
            "python".to_string(),
            LspServerConfig {
                language_id: "python".to_string(),
                command: "pyright-langserver".to_string(),
                args: vec!["--stdio".to_string()],
                trigger_characters: vec!['.', '@'],
            },
        );

        configs.insert(
            "json".to_string(),
            LspServerConfig {
                language_id: "json".to_string(),
                command: "vscode-json-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                trigger_characters: vec!['"', ':'],
            },
        );

        configs.insert(
            "go".to_string(),
            LspServerConfig {
                language_id: "go".to_string(),
                command: "gopls".to_string(),
                args: Vec::new(),
                trigger_characters: vec!['.'],
            },
        );

        Self {
            configs: RwLock::new(configs),
            clients: RwLock::new(HashMap::new()),
            failed_languages: RwLock::new(HashMap::new()),
            start_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn register_config(&self, config: LspServerConfig) {
        self.failed_languages.write().remove(&config.language_id);
        self.configs.write().insert(config.language_id.clone(), config);
    }

    pub fn get_config(&self, language_id: &str) -> Option<LspServerConfig> {
        self.configs.read().get(language_id).cloned()
    }

    pub fn get_trigger_characters(&self, language_id: &str) -> Vec<char> {
        self.configs
            .read()
            .get(language_id)
            .map(|c| c.trigger_characters.clone())
            .unwrap_or_else(|| vec!['.', ':'])
    }

    pub fn attach_client(&self, language_id: &str, client: Arc<LspClient>) {
        self.failed_languages.write().remove(language_id);
        self.clients.write().insert(language_id.to_string(), client);
    }

    pub fn get_client(&self, language_id: &str) -> Option<Arc<LspClient>> {
        self.clients.read().get(language_id).cloned()
    }

    pub fn remove_client(&self, language_id: &str) -> Option<Arc<LspClient>> {
        self.clients.write().remove(language_id)
    }

    pub fn is_language_failed(&self, language_id: &str) -> bool {
        self.failed_languages.read().contains_key(language_id)
    }

    pub fn get_failure_reason(&self, language_id: &str) -> Option<String> {
        self.failed_languages.read().get(language_id).cloned()
    }

    pub fn clear_failure(&self, language_id: &str) {
        self.failed_languages.write().remove(language_id);
    }

    /// Ensures a language server is running for `lang_id` in `workspace_root`,
    /// spawning and handshaking on first open if configured.
    ///
    /// Missing binaries or startup failures are cached non-fatally (`Ok(None)`)
    /// so repeated document opens/edits never crash or spam-spawn processes.
    pub async fn ensure_server_for_language(
        &self,
        lang_id: &str,
        workspace_root: &Path,
    ) -> Result<Option<Arc<LspClient>>, LspError> {
        if let Some(existing) = self.get_client(lang_id) {
            if existing.is_running() {
                return Ok(Some(existing));
            }
            self.remove_client(lang_id);
        }

        if self.failed_languages.read().contains_key(lang_id) {
            return Ok(None);
        }

        if self.get_config(lang_id).is_none() {
            return Ok(None);
        }

        let _guard = self.start_lock.lock().await;

        if let Some(existing) = self.get_client(lang_id) {
            if existing.is_running() {
                return Ok(Some(existing));
            }
            self.remove_client(lang_id);
        }

        if self.failed_languages.read().contains_key(lang_id) {
            return Ok(None);
        }

        let root_uri = Some(path_to_file_uri(workspace_root));
        match self
            .start_server_inner(lang_id, root_uri, Some(workspace_root.to_path_buf()))
            .await
        {
            Ok(client) => Ok(Some(client)),
            Err(err) => {
                self.failed_languages
                    .write()
                    .insert(lang_id.to_string(), err.to_string());
                Ok(None)
            }
        }
    }

    pub async fn start_server(
        &self,
        language_id: &str,
        root_uri: Option<String>,
        cwd: Option<PathBuf>,
    ) -> Result<Arc<LspClient>, LspError> {
        if let Some(existing) = self.get_client(language_id) {
            if existing.is_running() {
                return Ok(existing);
            }
            self.remove_client(language_id);
        }

        let _guard = self.start_lock.lock().await;
        if let Some(existing) = self.get_client(language_id) {
            if existing.is_running() {
                return Ok(existing);
            }
            self.remove_client(language_id);
        }

        self.start_server_inner(language_id, root_uri, cwd).await
    }

    async fn start_server_inner(
        &self,
        language_id: &str,
        root_uri: Option<String>,
        cwd: Option<PathBuf>,
    ) -> Result<Arc<LspClient>, LspError> {
        let config = self
            .get_config(language_id)
            .ok_or_else(|| LspError::ServerError {
                code: -1,
                message: format!("No LSP configuration registered for language: {}", language_id),
            })?;

        let args_refs: Vec<&str> = config.args.iter().map(|s| s.as_str()).collect();
        let client = LspClient::spawn_process(&config.command, &args_refs, cwd)?;
        let arc_client = Arc::new(client);

        let init_params = InitializeParams {
            root_uri,
            capabilities: ClientCapabilities::default_phase9(),
            ..Default::default()
        };
        arc_client.initialize(init_params).await?;
        arc_client.initialized().await?;

        self.attach_client(language_id, arc_client.clone());
        Ok(arc_client)
    }

    // --- High-Level Document Synchronization Routing ---

    pub async fn did_open_document(
        &self,
        uri: &str,
        language_id: &str,
        version: i32,
        text: &str,
    ) -> Result<(), LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.did_open(uri, language_id, version, text).await?;
        }
        Ok(())
    }

    pub async fn did_change_document(
        &self,
        uri: &str,
        language_id: &str,
        version: i32,
        text: &str,
    ) -> Result<(), LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.did_change_full(uri, version, text).await?;
        }
        Ok(())
    }

    pub async fn did_close_document(
        &self,
        uri: &str,
        language_id: &str,
    ) -> Result<(), LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.did_close(uri).await?;
        }
        Ok(())
    }

    // --- High-Level Request Routing ---

    pub async fn request_completions(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
        context: Option<CompletionContext>,
    ) -> Result<Vec<LspCompletionItem>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.completion(uri, position, context).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_hover(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Result<Option<Hover>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.hover(uri, position).await
        } else {
            Ok(None)
        }
    }

    pub async fn request_definition(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Result<Vec<LspLocation>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.definition(uri, position).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_formatting(
        &self,
        uri: &str,
        language_id: &str,
        options: FormattingOptions,
    ) -> Result<Vec<LspTextEdit>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.formatting(uri, options).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_code_actions(
        &self,
        uri: &str,
        language_id: &str,
        range: LspRange,
        context: CodeActionContext,
    ) -> Result<Vec<CodeAction>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.code_action(uri, range, context).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_workspace_symbols(
        &self,
        query: &str,
    ) -> Result<Vec<SymbolInformation>, LspError> {
        let clients: Vec<Arc<LspClient>> = self.clients.read().values().cloned().collect();
        let mut all_symbols = Vec::new();
        for client in clients {
            if let Ok(symbols) = client.workspace_symbol(query).await {
                all_symbols.extend(symbols);
            }
        }
        Ok(all_symbols)
    }

    pub async fn request_document_symbols(
        &self,
        uri: &str,
        language_id: &str,
    ) -> Result<Vec<SymbolInformation>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.document_symbol(uri).await
        } else {
            Ok(Vec::new())
        }
    }

    pub fn semantic_tokens_legend(&self, language_id: &str) -> Option<SemanticTokensLegend> {
        self.get_client(language_id)?
            .server_capabilities()?
            .semantic_tokens_legend()
    }

    pub async fn request_signature_help(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Result<Option<SignatureHelp>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.signature_help(uri, position).await
        } else {
            Ok(None)
        }
    }

    pub async fn request_references(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
        include_declaration: bool,
    ) -> Result<Vec<LspLocation>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.references(uri, position, include_declaration).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_implementation(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Result<Vec<LspLocation>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.implementation(uri, position).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_type_definition(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Result<Vec<LspLocation>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.type_definition(uri, position).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_prepare_rename(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Result<Option<PrepareRenameResult>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.prepare_rename(uri, position).await
        } else {
            Ok(None)
        }
    }

    pub async fn request_rename(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.rename(uri, position, new_name).await
        } else {
            Ok(None)
        }
    }

    pub async fn request_semantic_tokens_full(
        &self,
        uri: &str,
        language_id: &str,
    ) -> Result<Option<SemanticTokens>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.semantic_tokens_full(uri).await
        } else {
            Ok(None)
        }
    }

    pub async fn request_semantic_tokens_range(
        &self,
        uri: &str,
        language_id: &str,
        range: LspRange,
    ) -> Result<Option<SemanticTokens>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.semantic_tokens_range(uri, range).await
        } else {
            Ok(None)
        }
    }

    pub async fn request_inlay_hints(
        &self,
        uri: &str,
        language_id: &str,
        range: LspRange,
    ) -> Result<Vec<InlayHint>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.inlay_hints(uri, range).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_code_lens(
        &self,
        uri: &str,
        language_id: &str,
    ) -> Result<Vec<CodeLens>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.code_lens(uri).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_code_lens_resolve(
        &self,
        language_id: &str,
        lens: &CodeLens,
    ) -> Result<Option<CodeLens>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.code_lens_resolve(lens).await.map(Some)
        } else {
            Ok(None)
        }
    }

    pub async fn request_document_links(
        &self,
        uri: &str,
        language_id: &str,
    ) -> Result<Vec<DocumentLink>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.document_links(uri).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_prepare_call_hierarchy(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Result<Vec<CallHierarchyItem>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.prepare_call_hierarchy(uri, position).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_call_hierarchy_incoming(
        &self,
        language_id: &str,
        item: &CallHierarchyItem,
    ) -> Result<Vec<CallHierarchyIncomingCall>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.call_hierarchy_incoming(item).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_call_hierarchy_outgoing(
        &self,
        language_id: &str,
        item: &CallHierarchyItem,
    ) -> Result<Vec<CallHierarchyOutgoingCall>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.call_hierarchy_outgoing(item).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_prepare_type_hierarchy(
        &self,
        uri: &str,
        language_id: &str,
        position: LspPosition,
    ) -> Result<Vec<TypeHierarchyItem>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.prepare_type_hierarchy(uri, position).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_type_hierarchy_supertypes(
        &self,
        language_id: &str,
        item: &TypeHierarchyItem,
    ) -> Result<Vec<TypeHierarchyItem>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.type_hierarchy_supertypes(item).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn request_type_hierarchy_subtypes(
        &self,
        language_id: &str,
        item: &TypeHierarchyItem,
    ) -> Result<Vec<TypeHierarchyItem>, LspError> {
        if let Some(client) = self.get_client(language_id) {
            client.type_hierarchy_subtypes(item).await
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn notify_workspace_folders_changed(
        &self,
        added: Vec<WorkspaceFolder>,
        removed: Vec<WorkspaceFolder>,
    ) -> Result<(), LspError> {
        let clients: Vec<Arc<LspClient>> = self.clients.read().values().cloned().collect();
        for client in clients {
            let _ = client
                .did_change_workspace_folders(added.clone(), removed.clone())
                .await;
        }
        Ok(())
    }

    pub fn register_client(&self, language_id: &str, client: Arc<LspClient>) {
        self.failed_languages.write().remove(language_id);
        self.clients.write().insert(language_id.to_string(), client);
    }

    pub async fn shutdown_all(&self) {
        let clients: Vec<Arc<LspClient>> = self.clients.write().drain().map(|(_, c)| c).collect();
        for client in clients {
            let _ = client.shutdown().await;
            let _ = client.exit().await;
        }
    }

    pub fn get_diagnostics(&self, uri: &str) -> Vec<LspDiagnostic> {
        let mut all = Vec::new();
        let clients = self.clients.read();
        for client in clients.values() {
            all.extend(client.get_diagnostics(uri));
        }
        all
    }

    pub fn get_all_diagnostics(&self) -> HashMap<String, Vec<LspDiagnostic>> {
        let mut map = HashMap::new();
        let clients = self.clients.read();
        for client in clients.values() {
            for (uri, diags) in client.get_all_diagnostics() {
                map.entry(uri).or_insert_with(Vec::new).extend(diags);
            }
        }
        map
    }
}

/// Converts a filesystem path into a normalized `file:///` URI.
pub fn path_to_file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let without_verbatim = raw
        .strip_prefix(r"\\?\")
        .or_else(|| raw.strip_prefix("//?/"))
        .unwrap_or(&raw);
    let forward = without_verbatim.replace('\\', "/");
    format!("file:///{}", forward.trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_client_manager_configs() {
        let manager = LspClientManager::new();
        assert_eq!(manager.get_config("rust").unwrap().command, "rust-analyzer");
        assert_eq!(
            manager.get_config("typescript").unwrap().command,
            "typescript-language-server"
        );
        assert_eq!(
            manager.get_config("javascript").unwrap().command,
            "typescript-language-server"
        );
        assert_eq!(
            manager.get_config("python").unwrap().command,
            "pyright-langserver"
        );
        assert_eq!(
            manager.get_config("json").unwrap().command,
            "vscode-json-language-server"
        );
        assert_eq!(manager.get_config("go").unwrap().command, "gopls");

        let rust_triggers = manager.get_trigger_characters("rust");
        assert!(rust_triggers.contains(&'.'));
        assert!(rust_triggers.contains(&':'));

        assert!(manager.get_trigger_characters("go").contains(&'.'));
    }

    #[test]
    fn test_path_to_file_uri_normalization() {
        assert_eq!(
            path_to_file_uri(Path::new("/home/dev/project")),
            "file:///home/dev/project"
        );
        assert_eq!(
            path_to_file_uri(Path::new(r"C:\Users\dev\project")),
            "file:///C:/Users/dev/project"
        );
        assert_eq!(
            path_to_file_uri(Path::new(r"\\?\C:\Users\dev\project")),
            "file:///C:/Users/dev/project"
        );
    }

    #[tokio::test]
    async fn test_ensure_server_for_language_caches_missing_binary_and_allows_attach() {
        use tokio::io::duplex;

        let manager = LspClientManager::new();
        manager.register_config(LspServerConfig::new(
            "go",
            "nonexistent-gopls-binary-xyz-99999",
            Vec::new(),
            vec!['.'],
        ));

        let workspace = std::env::temp_dir();

        // Unconfigured language returns Ok(None) without failing
        let unconfigured = manager
            .ensure_server_for_language("markdown", &workspace)
            .await
            .unwrap();
        assert!(unconfigured.is_none());
        assert!(!manager.is_language_failed("markdown"));

        // First open with missing binary fails non-fatally and caches the failure
        let first = manager
            .ensure_server_for_language("go", &workspace)
            .await
            .unwrap();
        assert!(first.is_none());
        assert!(manager.is_language_failed("go"));
        assert!(manager.get_failure_reason("go").is_some());

        // Second open returns cached Ok(None) without error/panic
        let second = manager
            .ensure_server_for_language("go", &workspace)
            .await
            .unwrap();
        assert!(second.is_none());

        // Attaching an existing/mock LspClient clears cached failure and succeeds
        let (_server_w, client_r) = duplex(1024);
        let (client_w, _server_r) = duplex(1024);
        let mock_client = Arc::new(LspClient::from_io(client_r, client_w));
        manager.attach_client("go", mock_client.clone());
        assert!(!manager.is_language_failed("go"));

        let attached = manager
            .ensure_server_for_language("go", &workspace)
            .await
            .unwrap();
        assert!(attached.is_some());
        assert!(Arc::ptr_eq(&attached.unwrap(), &mock_client));
    }

    #[tokio::test]
    async fn test_lsp_client_manager_multi_client_diagnostics_aggregation() {
        use crate::lsp::types::{DiagnosticSeverity, LspDiagnostic, LspPosition, LspRange};
        use tokio::io::duplex;

        let manager = LspClientManager::new();
        let (server_w1, client_r1) = duplex(1024);
        let (client_w1, _server_r1) = duplex(1024);
        let client1 = Arc::new(LspClient::from_io(client_r1, client_w1));

        let (server_w2, client_r2) = duplex(1024);
        let (client_w2, _server_r2) = duplex(1024);
        let client2 = Arc::new(LspClient::from_io(client_r2, client_w2));

        let diag1 = LspDiagnostic {
            range: LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 5)),
            severity: Some(DiagnosticSeverity::Error),
            message: "syntax error".to_string(),
            source: Some("rust-analyzer".to_string()),
            code: None,
        };
        let diag2 = LspDiagnostic {
            range: LspRange::new(LspPosition::new(0, 6), LspPosition::new(0, 10)),
            severity: Some(DiagnosticSeverity::Warning),
            message: "variable not snake_case".to_string(),
            source: Some("clippy".to_string()),
            code: None,
        };

        client1.set_diagnostics("file:///main.rs", vec![diag1]);
        client2.set_diagnostics("file:///main.rs", vec![diag2]);

        manager.register_client("rust", client1);
        manager.register_client("clippy", client2);

        // Verify get_diagnostics aggregates both clients
        let aggregated = manager.get_diagnostics("file:///main.rs");
        assert_eq!(aggregated.len(), 2);
        assert!(aggregated.iter().any(|d| d.source.as_deref() == Some("rust-analyzer")));
        assert!(aggregated.iter().any(|d| d.source.as_deref() == Some("clippy")));

        let all_map = manager.get_all_diagnostics();
        assert_eq!(all_map.get("file:///main.rs").unwrap().len(), 2);

        let _ = server_w1;
        let _ = server_w2;
    }

    #[tokio::test]
    async fn test_lsp_client_manager_formatting_and_code_actions_routing() {
        let manager = LspClientManager::new();
        // If no client registered, should return empty Vec
        let edits = manager
            .request_formatting("file:///test.rs", "rust", FormattingOptions::new(4, true))
            .await
            .unwrap();
        assert!(edits.is_empty());

        let actions = manager
            .request_code_actions(
                "file:///test.rs",
                "rust",
                LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 1)),
                CodeActionContext::default(),
            )
            .await
            .unwrap();
        assert!(actions.is_empty());
    }
}
