//! Language feature bridge coordinating LSP clients and JavaScript extension providers.

use super::{ProviderRegistry, ProviderType};
use crate::rpc::RpcBridge;
use rustcode_protocols::lsp::{
    CompletionContext, CompletionTriggerKind, Hover, LspClientManager, LspCompletionItem,
    LspLocation, LspPosition, MarkupContent,
};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

pub struct LanguageFeatureBridge {
    pub provider_registry: Arc<ProviderRegistry>,
    pub rpc_bridge: Arc<RpcBridge>,
    pub lsp_manager: Arc<LspClientManager>,
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
        }
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
}
