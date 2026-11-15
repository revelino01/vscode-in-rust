//! Grammar registry for looking up language tokenizers by ID or file path.

use crate::syntax::javascript::{JavaScriptGrammar, TypeScriptGrammar};
use crate::syntax::json::JsonGrammar;
use crate::syntax::markdown::MarkdownGrammar;
use crate::syntax::python::PythonGrammar;
use crate::syntax::rust::RustGrammar;
use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub struct PlainTextGrammar;

impl Grammar for PlainTextGrammar {
    fn language_id(&self) -> &'static str {
        "plaintext"
    }

    fn numeric_id(&self) -> u8 {
        0
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["txt", "text", "log"]
    }

    fn tokenize_line(
        &self,
        line: &str,
        _state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>) {
        if line.is_empty() {
            (Vec::new(), None)
        } else {
            (
                vec![TokenSpan::new(0, line.len(), SyntaxScope::Plain, 0)],
                None,
            )
        }
    }
}

pub struct GrammarRegistry {
    grammars_by_id: HashMap<String, Arc<dyn Grammar>>,
    grammars_by_ext: HashMap<String, Arc<dyn Grammar>>,
    default_grammar: Arc<dyn Grammar>,
}

impl Default for GrammarRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl GrammarRegistry {
    pub fn new() -> Self {
        let default_grammar: Arc<dyn Grammar> = Arc::new(PlainTextGrammar);
        let mut registry = Self {
            grammars_by_id: HashMap::new(),
            grammars_by_ext: HashMap::new(),
            default_grammar,
        };

        registry.register(Arc::new(RustGrammar));
        registry.register(Arc::new(JavaScriptGrammar));
        registry.register(Arc::new(TypeScriptGrammar));
        registry.register(Arc::new(PythonGrammar));
        registry.register(Arc::new(JsonGrammar));
        registry.register(Arc::new(MarkdownGrammar));
        registry.register(Arc::new(PlainTextGrammar));

        registry
    }

    pub fn register(&mut self, grammar: Arc<dyn Grammar>) {
        let lang_id = grammar.language_id().to_string();
        self.grammars_by_id.insert(lang_id.clone(), grammar.clone());

        for ext in grammar.file_extensions() {
            self.grammars_by_ext
                .insert(ext.to_ascii_lowercase(), grammar.clone());
        }
    }

    pub fn get_by_id(&self, lang_id: &str) -> Option<Arc<dyn Grammar>> {
        self.grammars_by_id.get(lang_id).cloned()
    }

    pub fn get_for_path(&self, path: &Path) -> Arc<dyn Grammar> {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if let Some(g) = self.grammars_by_ext.get(&ext.to_ascii_lowercase()) {
                return g.clone();
            }
        }
        self.default_grammar.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grammar_registry_lookups() {
        let registry = GrammarRegistry::new();

        assert_eq!(registry.get_by_id("rust").unwrap().language_id(), "rust");
        assert_eq!(registry.get_by_id("javascript").unwrap().language_id(), "javascript");
        assert_eq!(registry.get_by_id("typescript").unwrap().language_id(), "typescript");
        assert_eq!(registry.get_by_id("python").unwrap().language_id(), "python");
        assert_eq!(registry.get_by_id("json").unwrap().language_id(), "json");
        assert_eq!(registry.get_by_id("markdown").unwrap().language_id(), "markdown");

        // Lookup by path
        assert_eq!(registry.get_for_path(Path::new("src/main.rs")).language_id(), "rust");
        assert_eq!(registry.get_for_path(Path::new("app.tsx")).language_id(), "typescript");
        assert_eq!(registry.get_for_path(Path::new("script.py")).language_id(), "python");
        assert_eq!(registry.get_for_path(Path::new("package.json")).language_id(), "json");
        assert_eq!(registry.get_for_path(Path::new("README.md")).language_id(), "markdown");
        assert_eq!(registry.get_for_path(Path::new("notes.unknown")).language_id(), "plaintext");
    }
}

