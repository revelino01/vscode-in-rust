//! `rustcode-base` — Layer 0: Core Foundation Utilities.
//!
//! Provides the core architectural building blocks mirroring VS Code's runtime model:
//! - Disposables (`Disposable`, `DisposableStore`)
//! - Typed Event Emitter (`Emitter`, `Event`)
//! - Cancellation Tokens (`CancellationTokenSource`, `CancellationToken`)
//! - VS Code `when` Clause Context Key Evaluator (`ContextKeyExpr`, `ContextKeyService`)

pub mod cancellation;
pub mod context_key;
pub mod disposable;
pub mod event;

pub use cancellation::{CancellationToken, CancellationTokenSource};
pub use context_key::{ContextKeyError, ContextKeyExpr, ContextKeyService};
pub use disposable::{ActionDisposable, ArcDisposable, Disposable, DisposableStore, to_disposable};
pub use event::{Emitter, Event};

/// Maps a file extension (without leading dot, case-insensitive) to a canonical VS Code language ID.
pub fn map_ext_to_lang_id(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "rs" => "rust".to_string(),
        "js" | "jsx" | "mjs" | "cjs" => "javascript".to_string(),
        "ts" | "tsx" | "mts" | "cts" => "typescript".to_string(),
        "py" | "pyw" | "pyi" => "python".to_string(),
        "json" | "jsonc" => "json".to_string(),
        "md" | "markdown" => "markdown".to_string(),
        "go" => "go".to_string(),
        "c" | "h" => "c".to_string(),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "cpp".to_string(),
        "html" | "htm" => "html".to_string(),
        "css" | "scss" | "less" => "css".to_string(),
        "toml" => "toml".to_string(),
        "yaml" | "yml" => "yaml".to_string(),
        "sh" | "bash" => "shellscript".to_string(),
        "txt" | "text" | "log" | "" => "plaintext".to_string(),
        _ => "plaintext".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::map_ext_to_lang_id;

    #[test]
    fn test_map_ext_to_lang_id_canonical() {
        assert_eq!(map_ext_to_lang_id("rs"), "rust");
        assert_eq!(map_ext_to_lang_id("RS"), "rust");
        assert_eq!(map_ext_to_lang_id("hh"), "cpp");
        assert_eq!(map_ext_to_lang_id("hxx"), "cpp");
        assert_eq!(map_ext_to_lang_id("hpp"), "cpp");
        assert_eq!(map_ext_to_lang_id("cc"), "cpp");
        assert_eq!(map_ext_to_lang_id("CPP"), "cpp");
        assert_eq!(map_ext_to_lang_id("txt"), "plaintext");
        assert_eq!(map_ext_to_lang_id("TXT"), "plaintext");
        assert_eq!(map_ext_to_lang_id("text"), "plaintext");
        assert_eq!(map_ext_to_lang_id("log"), "plaintext");
        assert_eq!(map_ext_to_lang_id(""), "plaintext");
        assert_eq!(map_ext_to_lang_id("sh"), "shellscript");
        assert_eq!(map_ext_to_lang_id("bash"), "shellscript");
        assert_eq!(map_ext_to_lang_id("xyz"), "plaintext");
    }
}
