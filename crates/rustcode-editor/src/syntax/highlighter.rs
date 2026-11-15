//! Incremental syntax highlighter with TextMate/VS Code theme token color resolution and line caching.

use crate::syntax::registry::GrammarRegistry;
use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};
use rustcode_platform::fs::hash_content;
use rustcode_platform::render::Rgba;
use rustcode_platform::theme::ColorTheme;
use std::sync::Arc;

#[derive(Debug, Clone)]
struct LineCacheEntry {
    tokens: Vec<TokenSpan>,
    end_state: Option<GrammarState>,
    text_hash: u64,
}

pub struct SyntaxHighlighter {
    grammar: Arc<dyn Grammar>,
    theme: ColorTheme,
    cache: Vec<Option<LineCacheEntry>>,
}

impl SyntaxHighlighter {
    pub fn new(grammar: Arc<dyn Grammar>, theme: ColorTheme) -> Self {
        Self {
            grammar,
            theme,
            cache: Vec::new(),
        }
    }

    pub fn for_language(lang_id: &str, theme: ColorTheme, registry: &GrammarRegistry) -> Self {
        let grammar = registry
            .get_by_id(lang_id)
            .unwrap_or_else(|| registry.get_by_id("plaintext").unwrap());
        Self::new(grammar, theme)
    }

    pub fn grammar(&self) -> &Arc<dyn Grammar> {
        &self.grammar
    }

    pub fn set_grammar(&mut self, grammar: Arc<dyn Grammar>) {
        self.grammar = grammar;
        self.cache.clear();
    }

    pub fn theme(&self) -> &ColorTheme {
        &self.theme
    }

    /// Update theme and re-resolve foreground colors on already-cached tokens without re-parsing.
    pub fn set_theme(&mut self, theme: ColorTheme) {
        self.theme = theme;
        for entry in self.cache.iter_mut().flatten() {
            for span in &mut entry.tokens {
                span.color = Some(Self::resolve_color(&self.theme, span.scope));
            }
        }
    }

    /// Invalidate line cache from a specific line index onwards (e.g. after an edit).
    pub fn invalidate_from(&mut self, line_idx: usize) {
        if line_idx < self.cache.len() {
            self.cache.truncate(line_idx);
        }
    }

    /// Clear entire syntax cache.
    pub fn invalidate_all(&mut self) {
        self.cache.clear();
    }

    /// Number of cached lines.
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Retrieve token spans for a line, tokenizing incrementally if not cached.
    pub fn get_line_tokens(&mut self, line_idx: usize, line_text: &str) -> &[TokenSpan] {
        let text_hash = hash_content(line_text);

        if line_idx < self.cache.len() {
            if let Some(entry) = &self.cache[line_idx] {
                if entry.text_hash == text_hash {
                    return &self.cache[line_idx].as_ref().unwrap().tokens;
                }
            }
        }

        // Determine starting grammar state from previous line
        let prev_state = if line_idx > 0 && line_idx - 1 < self.cache.len() {
            self.cache[line_idx - 1]
                .as_ref()
                .and_then(|e| e.end_state.as_ref())
        } else {
            None
        };

        let (mut tokens, end_state) = self.grammar.tokenize_line(line_text, prev_state);

        // Resolve colors for each token span using active theme
        for span in &mut tokens {
            span.color = Some(Self::resolve_color(&self.theme, span.scope));
        }

        // Ensure cache capacity
        if line_idx >= self.cache.len() {
            self.cache.resize(line_idx + 1, None);
        }

        self.cache[line_idx] = Some(LineCacheEntry {
            tokens,
            end_state,
            text_hash,
        });

        &self.cache[line_idx].as_ref().unwrap().tokens
    }

    /// Resolve a `SyntaxScope` to an `Rgba` foreground color using the theme's `token_colors` rules.
    pub fn resolve_color(theme: &ColorTheme, scope: SyntaxScope) -> Rgba {
        let scope_str = scope.as_scope_str();

        let mut best_score = 0;
        let mut best_color_hex: Option<&str> = None;

        for rule in &theme.token_colors {
            if let Some(hex) = &rule.foreground {
                for rule_scope in &rule.scope {
                    let score = compute_scope_match_score(scope_str, rule_scope);
                    if score > best_score {
                        best_score = score;
                        best_color_hex = Some(hex);
                    }
                }
            }
        }

        if let Some(hex) = best_color_hex {
            if let Some(color) = Rgba::from_hex(hex) {
                return color;
            }
        }

        // VS Code default fallback palette
        match scope {
            SyntaxScope::Comment
            | SyntaxScope::CommentLine
            | SyntaxScope::CommentBlock
            | SyntaxScope::CommentDoc => Rgba::from_hex("#6A9955").unwrap(),

            SyntaxScope::Keyword
            | SyntaxScope::KeywordControl
            | SyntaxScope::KeywordOperator
            | SyntaxScope::KeywordOther => Rgba::from_hex("#C586C0").unwrap(),

            SyntaxScope::String | SyntaxScope::StringQuoted | SyntaxScope::StringTemplate => {
                Rgba::from_hex("#CE9178").unwrap()
            }

            SyntaxScope::Number | SyntaxScope::ConstantNumeric => {
                Rgba::from_hex("#B5CEA8").unwrap()
            }

            SyntaxScope::ConstantLanguage => Rgba::from_hex("#4FC1FF").unwrap(),

            SyntaxScope::Type | SyntaxScope::EntityNameType | SyntaxScope::StorageType => {
                Rgba::from_hex("#4EC9B0").unwrap()
            }

            SyntaxScope::Function
            | SyntaxScope::EntityNameFunction
            | SyntaxScope::SupportFunction => Rgba::from_hex("#DCDCAA").unwrap(),

            SyntaxScope::Variable
            | SyntaxScope::VariableParameter
            | SyntaxScope::Property => Rgba::from_hex("#9CDCFE").unwrap(),

            SyntaxScope::Operator => Rgba::from_hex("#D4D4D4").unwrap(),

            SyntaxScope::MarkdownHeading => Rgba::from_hex("#569CD6").unwrap(),
            SyntaxScope::MarkdownBold => Rgba::from_hex("#4FC1FF").unwrap(),
            SyntaxScope::MarkdownItalic => Rgba::from_hex("#9CDCFE").unwrap(),
            SyntaxScope::MarkdownCode => Rgba::from_hex("#CE9178").unwrap(),
            SyntaxScope::MarkdownLink => Rgba::from_hex("#3794FF").unwrap(),

            SyntaxScope::Punctuation
            | SyntaxScope::PunctuationBracket
            | SyntaxScope::PunctuationDelimiter
            | SyntaxScope::Plain => Rgba::from_hex("#D4D4D4").unwrap(),
        }
    }
}

/// Compute hierarchical matching score between token scope and rule scope.
/// Returns score > 0 on match, where higher means closer match.
fn compute_scope_match_score(token_scope: &str, rule_scope: &str) -> usize {
    if token_scope == rule_scope {
        return 1000 + token_scope.len();
    }

    if let Some(remainder) = token_scope.strip_prefix(rule_scope) {
        if remainder.starts_with('.') {
            return 500 + rule_scope.len();
        }
    }

    // Reverse prefix match (e.g. rule specifies "storage" and token is "storage.type")
    if let Some(remainder) = rule_scope.strip_prefix(token_scope) {
        if remainder.starts_with('.') {
            return 200 + token_scope.len();
        }
    }


    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::rust::RustGrammar;
    use rustcode_platform::theme::ThemeService;

    #[test]
    fn test_syntax_highlighter_tokenization_and_caching() {
        let grammar = Arc::new(RustGrammar);
        let theme = ThemeService::default_dark_theme();
        let mut highlighter = SyntaxHighlighter::new(grammar, theme);

        let line = "let mut count: i32 = 42; // counter";
        let tokens1 = highlighter.get_line_tokens(0, line);
        assert!(!tokens1.is_empty());

        let keyword_count = tokens1
            .iter()
            .filter(|t| t.scope == SyntaxScope::Keyword)
            .count();
        assert_eq!(keyword_count, 2); // "let", "mut"

        // Tokens have resolved colors
        assert!(tokens1[0].color.is_some());
        let tokens1_len = tokens1.len();

        // Cache hit
        let tokens2_len = highlighter.get_line_tokens(0, line).len();
        assert_eq!(tokens1_len, tokens2_len);

        // Invalidation on edit
        highlighter.invalidate_from(0);
        let tokens3 = highlighter.get_line_tokens(0, "let updated = 1;");
        assert_eq!(tokens3.len(), 5);
    }

    #[test]
    fn test_syntax_highlighter_multiline_block_comments() {
        let grammar = Arc::new(RustGrammar);
        let theme = ThemeService::default_dark_theme();
        let mut highlighter = SyntaxHighlighter::new(grammar, theme);

        let line0 = "/* multiline start";
        let line1 = "middle comment line";
        let line2 = "end of comment */ let x = 10;";

        let t0 = highlighter.get_line_tokens(0, line0);
        assert_eq!(t0[0].scope, SyntaxScope::CommentBlock);

        let t1 = highlighter.get_line_tokens(1, line1);
        assert_eq!(t1[0].scope, SyntaxScope::CommentBlock);

        let t2 = highlighter.get_line_tokens(2, line2);
        assert_eq!(t2[0].scope, SyntaxScope::CommentBlock);
        assert!(t2.iter().any(|t| t.scope == SyntaxScope::Keyword));
    }
}
