//! LSP Semantic Tokens overlay layered over TextMate, tree-sitter, and built-in grammars.

use crate::model::piece_table::utf16_col_to_byte_offset;
use crate::syntax::scope::{SyntaxScope, TokenSpan};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticTokenSpan {
    pub line: usize,
    pub start_utf16: usize,
    pub length_utf16: usize,
    pub token_type: String,
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticTokenOverlay {
    by_line: HashMap<usize, Vec<SemanticTokenSpan>>,
    total_tokens: usize,
}

impl SemanticTokenOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_spans(spans: Vec<SemanticTokenSpan>) -> Self {
        let mut by_line: HashMap<usize, Vec<SemanticTokenSpan>> = HashMap::new();
        let total_tokens = spans.len();
        for span in spans {
            by_line.entry(span.line).or_default().push(span);
        }
        for list in by_line.values_mut() {
            list.sort_by_key(|s| s.start_utf16);
        }
        Self {
            by_line,
            total_tokens,
        }
    }

    pub fn tokens_for_line(&self, line: usize) -> &[SemanticTokenSpan] {
        self.by_line.get(&line).map_or(&[], Vec::as_slice)
    }

    pub fn is_empty(&self) -> bool {
        self.total_tokens == 0
    }

    pub fn len(&self) -> usize {
        self.total_tokens
    }

    pub fn clear(&mut self) {
        self.by_line.clear();
        self.total_tokens = 0;
    }
}

/// Map an LSP semantic token type + modifiers into RustCode `SyntaxScope`, TextMate scope hint,
/// and font styles `(bold, italic, underline)`.
pub fn map_semantic_token_to_scope(
    token_type: &str,
    modifiers: &[String],
) -> (SyntaxScope, &'static str, bool, bool, bool) {
    let has_mod = |m: &str| modifiers.iter().any(|x| x == m);
    let is_doc = has_mod("documentation");
    let is_default_lib = has_mod("defaultLibrary");
    let bold = has_mod("declaration") || has_mod("static");
    let italic = has_mod("abstract") || has_mod("async");
    let underline = has_mod("deprecated");

    let (scope, tm_scope) = match token_type {
        "comment" => {
            if is_doc {
                (SyntaxScope::CommentDoc, "comment.block.documentation")
            } else {
                (SyntaxScope::Comment, "comment")
            }
        }
        "keyword" | "modifier" => (SyntaxScope::Keyword, "keyword.control"),
        "string" => (SyntaxScope::String, "string.quoted"),
        "number" => (SyntaxScope::Number, "constant.numeric"),
        "regexp" => (SyntaxScope::StringTemplate, "string.regexp"),
        "operator" => (SyntaxScope::Operator, "keyword.operator"),
        "namespace" => (SyntaxScope::Type, "entity.name.namespace"),
        "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" => {
            (SyntaxScope::EntityNameType, "entity.name.type")
        }
        "function" | "method" | "macro" | "decorator" => {
            if is_default_lib {
                (SyntaxScope::SupportFunction, "support.function")
            } else {
                (SyntaxScope::EntityNameFunction, "entity.name.function")
            }
        }
        "parameter" => (SyntaxScope::VariableParameter, "variable.parameter"),
        "property" => (SyntaxScope::Property, "variable.other.property"),
        "enumMember" => (SyntaxScope::ConstantLanguage, "constant.other.enum"),
        "variable" | "event" => {
            if has_mod("readonly") {
                (SyntaxScope::ConstantLanguage, "variable.other.constant")
            } else {
                (SyntaxScope::Variable, "variable.other")
            }
        }
        _ => (SyntaxScope::Plain, "source"),
    };

    (scope, tm_scope, bold, italic, underline)
}

/// Layer semantic token spans on top of underlying TextMate / tree-sitter / built-in `TokenSpan`s,
/// preserving non-overridden regions from `base_tokens`.
pub fn overlay_semantic_tokens(
    base_tokens: Vec<TokenSpan>,
    line_text: &str,
    sem_spans: &[SemanticTokenSpan],
    lang_id: u8,
) -> Vec<TokenSpan> {
    if sem_spans.is_empty() || line_text.is_empty() {
        return base_tokens;
    }

    let mut overrides: Vec<TokenSpan> = Vec::with_capacity(sem_spans.len());
    for sem in sem_spans {
        if sem.length_utf16 == 0 {
            continue;
        }
        let start_b = utf16_col_to_byte_offset(line_text, sem.start_utf16);
        let end_b = utf16_col_to_byte_offset(line_text, sem.start_utf16.saturating_add(sem.length_utf16));
        if end_b <= start_b || end_b > line_text.len() {
            continue;
        }
        let (scope, tm_scope, bold, italic, underline) =
            map_semantic_token_to_scope(&sem.token_type, &sem.modifiers);
        let mut span = TokenSpan::with_style(start_b, end_b, scope, lang_id, bold, italic, underline);
        span.tm_scope = Some(tm_scope.to_string());
        overrides.push(span);
    }

    if overrides.is_empty() {
        return base_tokens;
    }
    overrides.sort_by_key(|s| (s.start, s.end));

    // Merge non-overlapping overrides over base_tokens
    let mut merged: Vec<TokenSpan> = Vec::with_capacity(base_tokens.len() + overrides.len());
    for base in base_tokens {
        let mut cursor = base.start;
        for ov in &overrides {
            if ov.end <= cursor || ov.start >= base.end {
                continue;
            }
            if ov.start > cursor && line_text.is_char_boundary(cursor) && line_text.is_char_boundary(ov.start) {
                let mut left = base.clone();
                left.start = cursor;
                left.end = ov.start;
                merged.push(left);
            }
            cursor = cursor.max(ov.end.min(base.end));
        }
        if cursor < base.end && line_text.is_char_boundary(cursor) && line_text.is_char_boundary(base.end) {
            let mut right = base;
            right.start = cursor;
            merged.push(right);
        }
    }

    merged.extend(overrides);
    merged.sort_by_key(|s| (s.start, s.end));
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_token_overlay_layers_over_base_tokens() {
        let line = "let alpha = foo(bar);";
        let base = vec![
            TokenSpan::new(0, 3, SyntaxScope::Keyword, 1),
            TokenSpan::new(4, 9, SyntaxScope::Variable, 1),
            TokenSpan::new(12, 15, SyntaxScope::Plain, 1),
            TokenSpan::new(16, 19, SyntaxScope::Plain, 1),
        ];
        let sem = vec![
            SemanticTokenSpan {
                line: 0,
                start_utf16: 12,
                length_utf16: 3,
                token_type: "function".to_string(),
                modifiers: vec!["defaultLibrary".to_string(), "deprecated".to_string()],
            },
            SemanticTokenSpan {
                line: 0,
                start_utf16: 16,
                length_utf16: 3,
                token_type: "parameter".to_string(),
                modifiers: vec!["readonly".to_string()],
            },
        ];

        let layered = overlay_semantic_tokens(base, line, &sem, 1);
        assert_eq!(layered[0].scope, SyntaxScope::Keyword); // preserved from base
        assert_eq!(layered[1].scope, SyntaxScope::Variable); // preserved from base
        let foo_tok = layered.iter().find(|t| t.start == 12 && t.end == 15).unwrap();
        assert_eq!(foo_tok.scope, SyntaxScope::SupportFunction);
        assert!(foo_tok.metadata.is_underline());
        let bar_tok = layered.iter().find(|t| t.start == 16 && t.end == 19).unwrap();
        assert_eq!(bar_tok.scope, SyntaxScope::VariableParameter);
    }
}
