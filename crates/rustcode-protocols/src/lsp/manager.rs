//! LSP Client Manager orchestrating multiple language server processes.

use super::client::{LspClient, LspError};
use super::types::*;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
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

        Self {
            configs: RwLock::new(configs),
            clients: RwLock::new(HashMap::new()),
        }
    }

    pub fn register_config(&self, config: LspServerConfig) {
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
        self.clients.write().insert(language_id.to_string(), client);
    }

    pub fn get_client(&self, language_id: &str) -> Option<Arc<LspClient>> {
        self.clients.read().get(language_id).cloned()
    }

    pub fn remove_client(&self, language_id: &str) -> Option<Arc<LspClient>> {
        self.clients.write().remove(language_id)
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
        }

        let config = self
            .get_config(language_id)
            .ok_or_else(|| LspError::ServerError {
                code: -1,
                message: format!("No LSP configuration registered for language: {}", language_id),
            })?;

        let args_refs: Vec<&str> = config.args.iter().map(|s| s.as_str()).collect();
        let client = LspClient::spawn_process(&config.command, &args_refs, cwd)?;
        let arc_client = Arc::new(client);

        // Initialize handshake
        let init_params = InitializeParams {
            root_uri,
            capabilities: ClientCapabilities::default(),
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

    pub async fn shutdown_all(&self) {
        let clients: Vec<Arc<LspClient>> = self.clients.write().drain().map(|(_, c)| c).collect();
        for client in clients {
            let _ = client.shutdown().await;
            let _ = client.exit().await;
        }
    }

    pub fn get_diagnostics(&self, uri: &str) -> Vec<LspDiagnostic> {
        let clients = self.clients.read();
        for client in clients.values() {
            let diags = client.get_diagnostics(uri);
            if !diags.is_empty() {
                return diags;
            }
        }
        Vec::new()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_client_manager_configs() {
        let manager = LspClientManager::new();
        assert!(manager.get_config("rust").is_some());
        assert!(manager.get_config("typescript").is_some());
        assert!(manager.get_config("python").is_some());

        let rust_triggers = manager.get_trigger_characters("rust");
        assert!(rust_triggers.contains(&'.'));
        assert!(rust_triggers.contains(&':'));

        manager.register_config(LspServerConfig {
            language_id: "go".to_string(),
            command: "gopls".to_string(),
            args: Vec::new(),
            trigger_characters: vec!['.'],
        });
        assert_eq!(manager.get_config("go").unwrap().command, "gopls");
    }
}
