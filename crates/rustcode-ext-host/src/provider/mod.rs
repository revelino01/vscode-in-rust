//! Provider callback registry for language intelligence and tree views.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderType {
    Completion,
    Hover,
    Definition,
    References,
    DocumentFormatting,
    TreeData,
}

#[derive(Debug, Clone)]
pub struct ProviderRegistration {
    pub id: String,
    pub provider_type: ProviderType,
    pub language_selector: Option<String>,
}

pub struct ProviderRegistry {
    providers: Arc<RwLock<HashMap<ProviderType, Vec<ProviderRegistration>>>>,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn register(&self, registration: ProviderRegistration) {
        let mut guard = self.providers.write();
        guard
            .entry(registration.provider_type)
            .or_default()
            .push(registration);
    }

    pub fn get_providers_for_language(&self, provider_type: ProviderType, lang: &str) -> Vec<ProviderRegistration> {
        let guard = self.providers.read();
        if let Some(list) = guard.get(&provider_type) {
            list.iter()
                .filter(|p| {
                    p.language_selector.as_deref().is_none_or(|sel| sel == lang || sel == "*")
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn unregister(&self, id: &str) {
        let mut guard = self.providers.write();
        for list in guard.values_mut() {
            list.retain(|p| p.id != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_registry() {
        let registry = ProviderRegistry::new();

        registry.register(ProviderRegistration {
            id: "rust-analyzer-completion".to_string(),
            provider_type: ProviderType::Completion,
            language_selector: Some("rust".to_string()),
        });

        registry.register(ProviderRegistration {
            id: "any-hover".to_string(),
            provider_type: ProviderType::Hover,
            language_selector: Some("*".to_string()),
        });

        let rust_completions = registry.get_providers_for_language(ProviderType::Completion, "rust");
        assert_eq!(rust_completions.len(), 1);
        assert_eq!(rust_completions[0].id, "rust-analyzer-completion");

        let py_completions = registry.get_providers_for_language(ProviderType::Completion, "python");
        assert!(py_completions.is_empty());

        let py_hover = registry.get_providers_for_language(ProviderType::Hover, "python");
        assert_eq!(py_hover.len(), 1);

        registry.unregister("rust-analyzer-completion");
        let remaining = registry.get_providers_for_language(ProviderType::Completion, "rust");
        assert!(remaining.is_empty());
    }
}
