//! Incremental syntax highlighting, grammar registry, tokenization pipeline, and scope resolution.

pub mod c;
pub mod cpp;
pub mod css;
pub mod go;
pub mod highlighter;
pub mod html;
pub mod javascript;
pub mod json;
pub mod markdown;
pub mod python;
pub mod registry;
pub mod rust;
pub mod scope;
pub mod toml;
pub mod yaml;

pub use c::CGrammar;
pub use cpp::CppGrammar;
pub use css::CssGrammar;
pub use go::GoGrammar;
pub use highlighter::SyntaxHighlighter;
pub use html::HtmlGrammar;
pub use javascript::{JavaScriptGrammar, TypeScriptGrammar};
pub use json::JsonGrammar;
pub use markdown::MarkdownGrammar;
pub use python::PythonGrammar;
pub use registry::{GrammarRegistry, PlainTextGrammar};
pub use rust::RustGrammar;
pub use scope::{Grammar, GrammarState, SyntaxScope, TokenMetadata, TokenSpan};
pub use toml::TomlGrammar;
pub use yaml::YamlGrammar;
