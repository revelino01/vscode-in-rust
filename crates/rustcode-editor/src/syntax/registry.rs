//! Grammar registry for looking up language tokenizers by ID or file path.

use crate::syntax::c::CGrammar;
use crate::syntax::cpp::CppGrammar;
use crate::syntax::css::CssGrammar;
use crate::syntax::go::GoGrammar;
use crate::syntax::html::HtmlGrammar;
use crate::syntax::javascript::{JavaScriptGrammar, TypeScriptGrammar};
use crate::syntax::json::JsonGrammar;
use crate::syntax::markdown::MarkdownGrammar;
use crate::syntax::python::PythonGrammar;
use crate::syntax::rust::RustGrammar;
use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};
use crate::syntax::toml::TomlGrammar;
use crate::syntax::yaml::YamlGrammar;
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
        registry.register(Arc::new(GoGrammar));
        registry.register(Arc::new(CGrammar));
        registry.register(Arc::new(CppGrammar));
        registry.register(Arc::new(HtmlGrammar));
        registry.register(Arc::new(CssGrammar));
        registry.register(Arc::new(TomlGrammar));
        registry.register(Arc::new(YamlGrammar));
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
        for ext in grammar.dynamic_file_extensions() {
            self.grammars_by_ext
                .insert(ext.to_ascii_lowercase(), grammar.clone());
        }
    }

    pub fn register_textmate(
        &mut self,
        grammar: Arc<crate::syntax::textmate::TextMateGrammar>,
    ) {
        let lang_id = grammar.language_id_str().to_string();
        let dyn_grammar: Arc<dyn Grammar> = grammar.clone();
        self.grammars_by_id.insert(lang_id, dyn_grammar.clone());
        for ext in grammar.extensions() {
            self.grammars_by_ext
                .insert(ext.to_ascii_lowercase(), dyn_grammar.clone());
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
        assert_eq!(registry.grammars_by_id.len(), 14);

        assert_eq!(registry.get_by_id("rust").unwrap().language_id(), "rust");
        assert_eq!(registry.get_by_id("javascript").unwrap().language_id(), "javascript");
        assert_eq!(registry.get_by_id("typescript").unwrap().language_id(), "typescript");
        assert_eq!(registry.get_by_id("python").unwrap().language_id(), "python");
        assert_eq!(registry.get_by_id("json").unwrap().language_id(), "json");
        assert_eq!(registry.get_by_id("markdown").unwrap().language_id(), "markdown");
        assert_eq!(registry.get_by_id("go").unwrap().numeric_id(), 7);
        assert_eq!(registry.get_by_id("c").unwrap().numeric_id(), 8);
        assert_eq!(registry.get_by_id("cpp").unwrap().numeric_id(), 9);
        assert_eq!(registry.get_by_id("html").unwrap().numeric_id(), 10);
        assert_eq!(registry.get_by_id("css").unwrap().numeric_id(), 11);
        assert_eq!(registry.get_by_id("toml").unwrap().numeric_id(), 12);
        assert_eq!(registry.get_by_id("yaml").unwrap().numeric_id(), 13);

        // Lookup by path
        assert_eq!(registry.get_for_path(Path::new("src/main.rs")).language_id(), "rust");
        assert_eq!(registry.get_for_path(Path::new("app.tsx")).language_id(), "typescript");
        assert_eq!(registry.get_for_path(Path::new("script.py")).language_id(), "python");
        assert_eq!(registry.get_for_path(Path::new("package.json")).language_id(), "json");
        assert_eq!(registry.get_for_path(Path::new("README.md")).language_id(), "markdown");
        assert_eq!(registry.get_for_path(Path::new("main.go")).language_id(), "go");
        assert_eq!(registry.get_for_path(Path::new("driver.c")).language_id(), "c");
        assert_eq!(registry.get_for_path(Path::new("driver.h")).language_id(), "c");
        assert_eq!(registry.get_for_path(Path::new("engine.cpp")).language_id(), "cpp");
        assert_eq!(registry.get_for_path(Path::new("vec.cxx")).language_id(), "cpp");
        assert_eq!(registry.get_for_path(Path::new("api.HPP")).language_id(), "cpp");
        assert_eq!(registry.get_for_path(Path::new("index.html")).language_id(), "html");
        assert_eq!(registry.get_for_path(Path::new("about.htm")).language_id(), "html");
        assert_eq!(registry.get_for_path(Path::new("styles.css")).language_id(), "css");
        assert_eq!(registry.get_for_path(Path::new("theme.scss")).language_id(), "css");
        assert_eq!(registry.get_for_path(Path::new("base.less")).language_id(), "css");
        assert_eq!(registry.get_for_path(Path::new("Cargo.toml")).language_id(), "toml");
        assert_eq!(registry.get_for_path(Path::new("ci.yaml")).language_id(), "yaml");
        assert_eq!(registry.get_for_path(Path::new("config.YML")).language_id(), "yaml");
        assert_eq!(registry.get_for_path(Path::new("notes.unknown")).language_id(), "plaintext");
    }

    #[test]
    fn test_seven_new_grammars_safety_invariants() {
        let registry = GrammarRegistry::new();
        for lang in ["go", "c", "cpp", "html", "css", "toml", "yaml"] {
            let g = registry.get_by_id(lang).unwrap();
            for ws in ["", "\r", "   \t  ", "  \t\r"] {
                let (tokens, state) = g.tokenize_line(ws, None);
                assert!(tokens.is_empty(), "{lang} emitted tokens on whitespace {ws:?}");
                assert!(state.is_none(), "{lang} emitted state on whitespace {ws:?}");
            }

            let malformed = [
                "/* unclosed 🚀 café \\",
                "\"unclosed 🚀 \\",
                "'unclosed 🚀 \\",
                "<!-- 🚀 unclosed",
                "R\"(🚀 unclosed",
                "\"\"\"🚀 unclosed",
                "[[[ 🚀 <<< >>> ### --- :::",
            ];
            for line in malformed {
                let (tokens, _) = g.tokenize_line(line, None);
                for t in &tokens {
                    assert!(t.start < t.end, "{lang} zero/invalid span on {line:?}");
                    assert!(t.end <= line.len(), "{lang} out-of-bounds span on {line:?}");
                    assert!(line.is_char_boundary(t.start), "{lang} non-char-boundary start");
                    assert!(line.is_char_boundary(t.end), "{lang} non-char-boundary end");
                }
            }
        }
    }
}

