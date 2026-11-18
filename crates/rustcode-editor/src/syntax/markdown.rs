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
        let mut no_more_links = false;
        let mut failed_bracket_until: usize = 0;

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
                let mut closed = false;
                while i + 1 < len {
                    if bytes[i] == delim && bytes[i + 1] == delim {
                        i += 2;
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if closed {
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
                } else {
                    tokens.push(TokenSpan::new(start, start + 2, SyntaxScope::Punctuation, lang_id));
                    i = start + 2;
                    continue;
                }
            }

            // Italic *...* or _..._
            if (b == b'*' || b == b'_') && i + 1 < len && bytes[i + 1] != b && bytes[i + 1] != b' ' {
                let delim = b;
                let start = i;
                i += 1;
                let mut closed = false;
                while i < len {
                    if bytes[i] == delim {
                        i += 1;
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if closed {
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
                } else {
                    tokens.push(TokenSpan::new(start, start + 1, SyntaxScope::Punctuation, lang_id));
                    i = start + 1;
                    continue;
                }
            }

            // Link [text](url)
            if b == b'[' {
                let start = i;
                if !no_more_links && start >= failed_bracket_until {
                    let mut scan = start + 1;
                    let mut close_bracket = None;
                    while scan < len {
                        if bytes[scan] == b']' {
                            close_bracket = Some(scan);
                            break;
                        }
                        scan += 1;
                    }
                    match close_bracket {
                        None => {
                            no_more_links = true;
                        }
                        Some(cb) => {
                            if cb + 1 < len && bytes[cb + 1] == b'(' {
                                let mut p = cb + 2;
                                let mut close_paren = None;
                                while p < len {
                                    if bytes[p] == b')' {
                                        close_paren = Some(p);
                                        break;
                                    }
                                    p += 1;
                                }
                                if let Some(cp) = close_paren {
                                    i = cp + 1;
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
                                } else {
                                    no_more_links = true;
                                }
                            } else {
                                failed_bracket_until = cb + 1;
                            }
                        }
                    }
                }
                tokens.push(TokenSpan::new(start, start + 1, SyntaxScope::Punctuation, lang_id));
                i = start + 1;
                continue;
            }

            // Standalone '*' or '_' not forming bold/italic
            if b == b'*' || b == b'_' {
                tokens.push(TokenSpan::new(i, i + 1, SyntaxScope::Plain, lang_id));
                i += 1;
                continue;
            }

            // Regular word / text with unicode support
            let start = i;
            while i < len && !matches!(bytes[i], b'`' | b'*' | b'_' | b'[') {
                let ch = line[i..].chars().next().unwrap();
                i += ch.len_utf8();
            }
            if i > start {
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Plain, lang_id));
            } else {
                let ch = line[i..].chars().next().unwrap();
                i += ch.len_utf8();
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Plain, lang_id));
            }
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

    #[test]
    fn test_markdown_unclosed_and_unicode() {
        let grammar = MarkdownGrammar;
        let line = "Here is **unclosed bold with 🦀 emoji and [unclosed link";
        let (tokens, state) = grammar.tokenize_line(line, None);
        assert_eq!(state, None);

        for t in &tokens {
            assert!(line.is_char_boundary(t.start));
            assert!(line.is_char_boundary(t.end));
            assert!(t.start < t.end);
            assert!(t.end <= line.len());
        }
    }

    #[test]
    fn test_markdown_standalone_star_and_underscore_no_hang() {
        let grammar = MarkdownGrammar;
        let cases = [
            "a * b",
            "hello *",
            "a _ b",
            "hello _",
            "/* 块注释 🦀 */",
            "* ** _ __",
        ];

        for line in cases {
            let (tokens, state) = grammar.tokenize_line(line, None);
            assert_eq!(state, None, "unexpected state for {:?}", line);
            assert!(!tokens.is_empty(), "expected tokens for {:?}", line);
            for t in &tokens {
                assert!(line.is_char_boundary(t.start));
                assert!(line.is_char_boundary(t.end));
                assert!(t.start < t.end);
                assert!(t.end <= line.len());
            }
        }
    }

    #[test]
    fn test_markdown_link_linear_time_and_edge_cases() {
        use std::time::Instant;

        let grammar = MarkdownGrammar;

        // Adversarial O(N^2) candidates must finish in O(N) time (< 200ms)
        let unclosed_50k = "[".repeat(50_000);
        let repeated_pair_20k = "[a]".repeat(20_000);
        let empty_pair_25k = "[]".repeat(25_000);
        let prefix_then_close_25k = format!("{}]", "[".repeat(25_000));
        let prefix_then_unclosed_paren_25k = format!("{}](unclosed", "[".repeat(25_000));
        let prefix_then_valid_link_50k = format!("{}[ok](https://example.com)", "[".repeat(50_000));

        for (name, line, expect_link) in [
            ("unclosed_50k", unclosed_50k.as_str(), false),
            ("repeated_pair_20k", repeated_pair_20k.as_str(), false),
            ("empty_pair_25k", empty_pair_25k.as_str(), false),
            ("prefix_then_close_25k", prefix_then_close_25k.as_str(), false),
            (
                "prefix_then_unclosed_paren_25k",
                prefix_then_unclosed_paren_25k.as_str(),
                false,
            ),
            ("prefix_then_valid_link_50k", prefix_then_valid_link_50k.as_str(), true),
        ] {
            let t0 = Instant::now();
            let (tokens, state) = grammar.tokenize_line(line, None);
            let elapsed = t0.elapsed();
            assert_eq!(state, None, "case {}", name);
            assert!(
                elapsed.as_millis() < 200,
                "case {} took {:?} (expected O(N) < 200ms)",
                name,
                elapsed
            );
            let has_link = tokens.iter().any(|t| t.scope == SyntaxScope::MarkdownLink);
            assert_eq!(has_link, expect_link, "case {}", name);
            for t in &tokens {
                assert!(line.is_char_boundary(t.start));
                assert!(line.is_char_boundary(t.end));
                assert!(t.start < t.end);
                assert!(t.end <= line.len());
            }
        }

        // Mixed valid & broken bracket cases
        let mixed_cases = [
            ("[text](url)", vec![(0, 11, SyntaxScope::MarkdownLink)]),
            ("[unclosed [ok](https://example.com)", vec![(0, 35, SyntaxScope::MarkdownLink)]),
            ("[a [b](url)", vec![(0, 11, SyntaxScope::MarkdownLink)]),
            (
                "[foo][bar](baz)",
                vec![
                    (0, 1, SyntaxScope::Punctuation),
                    (1, 5, SyntaxScope::Plain),
                    (5, 15, SyntaxScope::MarkdownLink),
                ],
            ),
            (
                "[a [b][c](d)",
                vec![
                    (0, 1, SyntaxScope::Punctuation),
                    (1, 3, SyntaxScope::Plain),
                    (3, 4, SyntaxScope::Punctuation),
                    (4, 6, SyntaxScope::Plain),
                    (6, 12, SyntaxScope::MarkdownLink),
                ],
            ),
            (
                "[foo](bar",
                vec![
                    (0, 1, SyntaxScope::Punctuation),
                    (1, 9, SyntaxScope::Plain),
                ],
            ),
            ("[🦀](https://example.com)", vec![(0, 27, SyntaxScope::MarkdownLink)]),
        ];

        for (line, expected) in mixed_cases {
            let (tokens, state) = grammar.tokenize_line(line, None);
            assert_eq!(state, None, "line {:?}", line);
            let actual: Vec<(usize, usize, SyntaxScope)> =
                tokens.iter().map(|t| (t.start, t.end, t.scope)).collect();
            assert_eq!(actual, expected, "line {:?}", line);
        }
    }
}

