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
pub mod textmate;
pub mod toml;
#[cfg(feature = "desktop")]
pub mod treesitter;
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
pub use scope::{
    Grammar, GrammarState, SyntaxScope, TextMateStackFrame, TokenMetadata, TokenSpan,
};
pub use textmate::{
    load_textmate_contribution, map_tm_scope_to_syntax_scope, GrammarContribution,
    TextMateGrammar,
};
pub use toml::TomlGrammar;
#[cfg(feature = "desktop")]
pub use treesitter::{TreeSitterDocument, TreeSitterEngine};
pub use yaml::YamlGrammar;
