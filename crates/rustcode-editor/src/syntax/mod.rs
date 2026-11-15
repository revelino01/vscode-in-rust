//! Incremental syntax highlighting, grammar registry, tokenization pipeline, and scope resolution.

pub mod highlighter;
pub mod javascript;
pub mod json;
pub mod markdown;
pub mod python;
pub mod registry;
pub mod rust;
pub mod scope;

pub use highlighter::SyntaxHighlighter;
pub use javascript::{JavaScriptGrammar, TypeScriptGrammar};
pub use json::JsonGrammar;
pub use markdown::MarkdownGrammar;
pub use python::PythonGrammar;
pub use registry::{GrammarRegistry, PlainTextGrammar};
pub use rust::RustGrammar;
pub use scope::{Grammar, GrammarState, SyntaxScope, TokenMetadata, TokenSpan};
