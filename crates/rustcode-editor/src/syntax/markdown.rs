//! Markdown document tokenizer.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct MarkdownGrammar;

impl Grammar for MarkdownGrammar {
    fn language_id(&self) -> &'static str {
        "markdown"
    }

    fn numeric_id(&self) -> u8 {
        6
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["md", "markdown"]
    }

    fn tokenize_line(
        &self,
        line: &str,
        state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>) {
        let lang_id = self.numeric_id();
        let mut tokens = Vec::new();
        let trimmed = line.trim_start();
        let leading_spaces = line.len() - trimmed.len();

        // 1. Resume fenced code block
        if let Some(GrammarState::CodeBlock { lang }) = state {
            if trimmed.starts_with("```") {
                tokens.push(TokenSpan::new(0, line.len(), SyntaxScope::MarkdownCode, lang_id));
                return (tokens, None);
            } else {
                tokens.push(TokenSpan::new(0, line.len(), SyntaxScope::MarkdownCode, lang_id));
                return (tokens, Some(GrammarState::CodeBlock { lang: lang.clone() }));
            }
        }

        // 2. Fenced code block start
        if trimmed.starts_with("```") {
            let lang = trimmed.trim_start_matches('`').trim().to_string();
            tokens.push(TokenSpan::new(0, line.len(), SyntaxScope::MarkdownCode, lang_id));
            return (tokens, Some(GrammarState::CodeBlock { lang }));
        }

        // 3. Headings #
        if trimmed.starts_with('#') {
            let heading_level = trimmed.chars().take_while(|&c| c == '#').count();
            if heading_level <= 6 && trimmed[heading_level..].starts_with(' ') {
                tokens.push(TokenSpan::with_style(
                    0,
                    line.len(),
                    SyntaxScope::MarkdownHeading,
                    lang_id,
                    true,
                    false,
                    false,
                ));
                return (tokens, None);
            }
        }

        // 4. Blockquotes >
        if trimmed.starts_with('>') {
            tokens.push(TokenSpan::new(0, line.len(), SyntaxScope::Comment, lang_id));
            return (tokens, None);
        }

        // 5. In-line markdown constructs: bold, italic, inline code, links
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = leading_spaces;

        // Check bullet or number list
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            tokens.push(TokenSpan::new(i, i + 2, SyntaxScope::Punctuation, lang_id));
            i += 2;
        }

        while i < len {
            let b = bytes[i];

            // Inline code `...`
            if b == b'`' {
                let start = i;
                i += 1;
                while i < len && bytes[i] != b'`' {
                    i += 1;
                }
                if i < len && bytes[i] == b'`' {
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::MarkdownCode, lang_id));
                continue;
            }

            // Bold **...** or __...__
            if (b == b'*' || b == b'_') && i + 1 < len && bytes[i + 1] == b {
                let delim = b;
                let start = i;
                i += 2;
                while i + 1 < len && !(bytes[i] == delim && bytes[i + 1] == delim) {
                    i += 1;
                }
                if i + 1 < len && bytes[i] == delim && bytes[i + 1] == delim {
                    i += 2;
                }
                tokens.push(TokenSpan::with_style(
                    start,
                    i,
                    SyntaxScope::MarkdownBold,
                    lang_id,
                    true,
                    false,
                    false,
                ));
                continue;
            }

            // Italic *...* or _..._
            if (b == b'*' || b == b'_') && i + 1 < len && bytes[i + 1] != b && bytes[i + 1] != b' ' {
                let delim = b;
                let start = i;
                i += 1;
                while i < len && bytes[i] != delim {
                    i += 1;
                }
                if i < len && bytes[i] == delim {
                    i += 1;
                }
                tokens.push(TokenSpan::with_style(
                    start,
                    i,
                    SyntaxScope::MarkdownItalic,
                    lang_id,
                    false,
                    true,
                    false,
                ));
                continue;
            }

            // Link [text](url)
            if b == b'[' {
                let start = i;
                i += 1;
                while i < len && bytes[i] != b']' {
                    i += 1;
                }
                if i < len && bytes[i] == b']' && i + 1 < len && bytes[i + 1] == b'(' {
                    i += 2;
                    while i < len && bytes[i] != b')' {
                        i += 1;
                    }
                    if i < len && bytes[i] == b')' {
                        i += 1;
                    }
                    tokens.push(TokenSpan::with_style(
                        start,
                        i,
                        SyntaxScope::MarkdownLink,
                        lang_id,
                        false,
                        false,
                        true,
                    ));
                    continue;
                }
            }

            // Regular word / text
            let start = i;
            while i < len && !matches!(bytes[i], b'`' | b'*' | b'_' | b'[') {
                i += 1;
            }
            tokens.push(TokenSpan::new(start, i, SyntaxScope::Plain, lang_id));
        }

        (tokens, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markdown_tokenization() {
        let grammar = MarkdownGrammar;
        let line = "# Title with **bold** and `code`";
        let (tokens, _) = grammar.tokenize_line(line, None);

        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::MarkdownHeading));
        assert!(tokens.iter().any(|t| t.metadata.is_bold()));

        // Fenced code blocks
        let fence = "```rust";
        let (fence_tokens, state) = grammar.tokenize_line(fence, None);
        assert!(fence_tokens.iter().any(|t| t.scope == SyntaxScope::MarkdownCode));
        assert!(matches!(state, Some(GrammarState::CodeBlock { .. })));
    }
}

