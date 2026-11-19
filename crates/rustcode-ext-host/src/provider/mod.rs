//! Provider callback registry for language intelligence and tree views.

pub mod bridge;

pub use bridge::{
    DiagnosticCollectionManager, ExtStatusBarItem, ExtStatusBarManager, LanguageFeatureBridge,
    TreeItem, TreeItemCollapsibleState,
};
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
    CodeAction,
    TreeData,
    NotebookSerializer,
    NotebookController,
    SourceControl,
    DebugConfiguration,
    DebugAdapterFactory,
    Task,
    TestController,
    Authentication,
    CustomEditor,
}

impl ProviderType {
    pub fn from_str_name(s: &str) -> Option<Self> {
        match s {
            "Completion" => Some(Self::Completion),
            "Hover" => Some(Self::Hover),
            "Definition" => Some(Self::Definition),
            "References" => Some(Self::References),
            "DocumentFormatting" => Some(Self::DocumentFormatting),
            "CodeAction" => Some(Self::CodeAction),
            "TreeData" | "TreeDataProvider" => Some(Self::TreeData),
            "NotebookSerializer" => Some(Self::NotebookSerializer),
            "NotebookController" => Some(Self::NotebookController),
            "SourceControl" => Some(Self::SourceControl),
            "DebugConfiguration" => Some(Self::DebugConfiguration),
            "DebugAdapterFactory" => Some(Self::DebugAdapterFactory),
            "Task" => Some(Self::Task),
            "TestController" => Some(Self::TestController),
            "Authentication" => Some(Self::Authentication),
            "CustomEditor" => Some(Self::CustomEditor),
            _ => None,
        }
    }
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
        let list = guard.entry(registration.provider_type).or_default();
        list.retain(|p| p.id != registration.id);
        list.push(registration);
    }

    pub fn get_providers(&self, provider_type: ProviderType) -> Vec<ProviderRegistration> {
        let guard = self.providers.read();
        guard.get(&provider_type).cloned().unwrap_or_default()
    }

    pub fn has_provider(&self, provider_type: ProviderType) -> bool {
        let guard = self.providers.read();
        guard
            .get(&provider_type)
            .is_some_and(|list| !list.is_empty())
    }

    pub fn has_provider_with_id(&self, provider_type: ProviderType, id: &str) -> bool {
        let guard = self.providers.read();
        guard
            .get(&provider_type)
            .is_some_and(|list| list.iter().any(|p| p.id == id))
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

        assert_eq!(
            ProviderType::from_str_name("NotebookSerializer"),
            Some(ProviderType::NotebookSerializer)
        );
        assert_eq!(
            ProviderType::from_str_name("NotebookController"),
            Some(ProviderType::NotebookController)
        );
        assert_eq!(
            ProviderType::from_str_name("SourceControl"),
            Some(ProviderType::SourceControl)
        );
        assert_eq!(
            ProviderType::from_str_name("DebugConfiguration"),
            Some(ProviderType::DebugConfiguration)
        );
        assert_eq!(
            ProviderType::from_str_name("DebugAdapterFactory"),
            Some(ProviderType::DebugAdapterFactory)
        );
        assert_eq!(
            ProviderType::from_str_name("Task"),
            Some(ProviderType::Task)
        );
        assert_eq!(
            ProviderType::from_str_name("TestController"),
            Some(ProviderType::TestController)
        );
        assert_eq!(
            ProviderType::from_str_name("Authentication"),
            Some(ProviderType::Authentication)
        );
        assert_eq!(
            ProviderType::from_str_name("CustomEditor"),
            Some(ProviderType::CustomEditor)
        );
        assert_eq!(ProviderType::from_str_name("UnknownProvider"), None);

        registry.register(ProviderRegistration {
            id: "git-scm".to_string(),
            provider_type: ProviderType::SourceControl,
            language_selector: Some("git".to_string()),
        });
        assert_eq!(
            registry.get_providers_for_language(ProviderType::SourceControl, "git").len(),
            1
        );
        registry.unregister("git-scm");
        assert!(registry
            .get_providers_for_language(ProviderType::SourceControl, "git")
            .is_empty());

        // TreeData & TreeDataProvider alias + deduplication + listing helpers
        assert_eq!(
            ProviderType::from_str_name("TreeData"),
            Some(ProviderType::TreeData)
        );
        assert_eq!(
            ProviderType::from_str_name("TreeDataProvider"),
            Some(ProviderType::TreeData)
        );
        assert!(!registry.has_provider(ProviderType::TreeData));

        registry.register(ProviderRegistration {
            id: "my.treeView".to_string(),
            provider_type: ProviderType::TreeData,
            language_selector: Some("*".to_string()),
        });
        registry.register(ProviderRegistration {
            id: "my.treeView".to_string(),
            provider_type: ProviderType::TreeData,
            language_selector: None,
        });

        assert!(registry.has_provider(ProviderType::TreeData));
        assert!(registry.has_provider_with_id(ProviderType::TreeData, "my.treeView"));
        assert!(!registry.has_provider_with_id(ProviderType::TreeData, "other.treeView"));
        let trees = registry.get_providers(ProviderType::TreeData);
        assert_eq!(trees.len(), 1);
        assert!(trees[0].language_selector.is_none());
    }
}
