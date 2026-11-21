//! CSS (and SCSS/LESS) language tokenizer and grammar implementation.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct CssGrammar;

impl Grammar for CssGrammar {
    fn language_id(&self) -> &'static str {
        "css"
    }

    fn numeric_id(&self) -> u8 {
        11
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["css", "scss", "less"]
    }

    fn tokenize_line(
        &self,
        line: &str,
        state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>) {
        let lang_id = self.numeric_id();
        let line = line.trim_end_matches('\r');
        let preserved = match state {
            Some(GrammarState::Normal) | None => None,
            Some(s) => Some(s.clone()),
        };
        if line.bytes().all(|b| b.is_ascii_whitespace()) {
            return (Vec::new(), preserved);
        }

        let mut tokens = Vec::new();
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        // Resume non-nesting block comment /* ... */
        if let Some(GrammarState::BlockComment { .. }) = state {
            let start = i;
            let mut closed = false;
            while i < len {
                if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    closed = true;
                    break;
                }
                let ch = line[i..].chars().next().unwrap();
                i += ch.len_utf8();
            }
            tokens.push(TokenSpan::new(start, i, SyntaxScope::CommentBlock, lang_id));
            if !closed {
                return (tokens, Some(GrammarState::BlockComment { depth: 1 }));
            }
        }

        while i < len {
            let b = bytes[i];

            if b.is_ascii_whitespace() {
                i += 1;
                continue;
            }

            // Comments: /* ... */ and // (SCSS/LESS)
            if b == b'/' && i + 1 < len {
                if bytes[i + 1] == b'/' {
                    tokens.push(TokenSpan::new(i, len, SyntaxScope::CommentLine, lang_id));
                    return (tokens, None);
                } else if bytes[i + 1] == b'*' {
                    let start = i;
                    i += 2;
                    let mut closed = false;
                    while i < len {
                        if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            i += 2;
                            closed = true;
                            break;
                        }
                        let ch = line[i..].chars().next().unwrap();
                        i += ch.len_utf8();
                    }
                    tokens.push(TokenSpan::new(start, i, SyntaxScope::CommentBlock, lang_id));
                    if !closed {
                        return (tokens, Some(GrammarState::BlockComment { depth: 1 }));
                    }
                    continue;
                }
            }

            // Quoted strings
            if b == b'"' || b == b'\'' {
                let quote = b;
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 1;
                        if i < len {
                            let esc = line[i..].chars().next().unwrap();
                            i += esc.len_utf8();
                        }
                    } else if bytes[i] == quote {
                        i += 1;
                        break;
                    } else {
                        let ch = line[i..].chars().next().unwrap();
                        i += ch.len_utf8();
                    }
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
                continue;
            }

            // At-rules (@media, @import, @keyframes, ...)
            if b == b'@' {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::KeywordControl, lang_id));
                continue;
            }

            // !important
            if b == b'!'
                && len - i >= 10
                && bytes[i..i + 10].eq_ignore_ascii_case(b"!important")
                && (i + 10 == len
                    || !(bytes[i + 10].is_ascii_alphanumeric()
                        || bytes[i + 10] == b'-'
                        || bytes[i + 10] == b'_'))
            {
                tokens.push(TokenSpan::new(i, i + 10, SyntaxScope::Keyword, lang_id));
                i += 10;
                continue;
            }

            // Hex color (#fff, #ff00aa) or ID selector (#header)
            if b == b'#' {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                    i += 1;
                }
                let body = &line[start + 1..i];
                let is_hex_color = matches!(body.len(), 3 | 4 | 6 | 8)
                    && body.bytes().all(|c| c.is_ascii_hexdigit());
                let scope = if is_hex_color {
                    SyntaxScope::ConstantNumeric
                } else {
                    SyntaxScope::EntityNameType
                };
                tokens.push(TokenSpan::new(start, i, scope, lang_id));
                continue;
            }

            // SCSS variable ($var)
            if b == b'$'
                && i + 1 < len
                && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_' || bytes[i + 1] == b'-')
            {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Variable, lang_id));
                continue;
            }

            // Numeric literals with optional CSS units (12px, 1.5rem, 100%, -4px, .5em)
            let is_signed_num = (b == b'-' || b == b'+')
                && i + 1 < len
                && (bytes[i + 1].is_ascii_digit()
                    || (bytes[i + 1] == b'.' && i + 2 < len && bytes[i + 2].is_ascii_digit()));
            let is_dot_num = b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit();

            if b.is_ascii_digit() || is_signed_num || is_dot_num {
                let start = i;
                if b == b'-' || b == b'+' {
                    i += 1;
                }
                while i < len
                    && (bytes[i].is_ascii_digit()
                        || bytes[i] == b'.'
                        || bytes[i] == b'e'
                        || bytes[i] == b'E')
                {
                    if (bytes[i] == b'e' || bytes[i] == b'E')
                        && i + 1 < len
                        && (bytes[i + 1] == b'+' || bytes[i + 1] == b'-')
                    {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                // Unit suffix (% or letters like px, rem, vh, deg, ms)
                if i < len && bytes[i] == b'%' {
                    i += 1;
                } else {
                    while i < len && bytes[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Number, lang_id));
                continue;
            }

            // Class selector (.btn, .btn-old)
            if b == b'.'
                && i + 1 < len
                && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_' || bytes[i + 1] == b'-')
            {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::EntityNameType, lang_id));
                continue;
            }

            let ch = line[i..].chars().next().unwrap();
            let ch_len = ch.len_utf8();

            // Identifiers: properties, functions, tag selectors, custom properties, values
            if ch.is_alphabetic() || ch == '_' || ch == '-' {
                let start = i;
                while i < len {
                    let next_ch = line[i..].chars().next().unwrap();
                    if next_ch.is_alphanumeric() || next_ch == '_' || next_ch == '-' {
                        i += next_ch.len_utf8();
                    } else {
                        break;
                    }
                }
                let word = &line[start..i];

                let mut peek = i;
                while peek < len && bytes[peek].is_ascii_whitespace() {
                    peek += 1;
                }

                let scope = if peek < len && bytes[peek] == b'(' {
                    SyntaxScope::Function
                } else if peek < len
                    && bytes[peek] == b':'
                    && (peek + 1 >= len || bytes[peek + 1] != b':')
                    && (word.starts_with("--")
                        || peek + 1 >= len
                        || bytes[peek + 1].is_ascii_whitespace()
                        || !is_css_tag_selector(word))
                {
                    SyntaxScope::Property
                } else if is_css_keyword_value(word) {
                    SyntaxScope::ConstantLanguage
                } else if is_css_tag_selector(word) {
                    SyntaxScope::Keyword
                } else {
                    SyntaxScope::Variable
                };

                tokens.push(TokenSpan::new(start, i, scope, lang_id));
                continue;
            }

            // Punctuation and operators
            let start = i;
            if b == b':' && i + 1 < len && bytes[i + 1] == b':' {
                tokens.push(TokenSpan::new(start, start + 2, SyntaxScope::PunctuationDelimiter, lang_id));
                i += 2;
                continue;
            }

            let scope = match b {
                b'{' | b'}' | b'(' | b')' | b'[' | b']' => SyntaxScope::PunctuationBracket,
                b';' | b',' | b':' | b'.' => SyntaxScope::PunctuationDelimiter,
                b'+' | b'>' | b'~' | b'*' | b'/' | b'=' | b'!' => SyntaxScope::Operator,
                _ => SyntaxScope::Plain,
            };
            tokens.push(TokenSpan::new(start, start + ch_len, scope, lang_id));
            i += ch_len;
        }

        (tokens, None)
    }
}

fn is_css_keyword_value(s: &str) -> bool {
    matches!(
        s,
        "auto" | "none" | "inherit" | "initial" | "unset" | "revert" | "block" | "inline"
            | "inline-block" | "flex" | "inline-flex" | "grid" | "inline-grid" | "absolute"
            | "relative" | "fixed" | "sticky" | "static" | "hidden" | "visible" | "scroll"
            | "center" | "left" | "right" | "top" | "bottom" | "bold" | "normal" | "italic"
            | "solid" | "dashed" | "dotted" | "pointer" | "transparent" | "currentColor"
            | "red" | "blue" | "green" | "white" | "black" | "gray" | "grey" | "sans-serif"
            | "serif" | "monospace" | "nowrap" | "wrap" | "column" | "row" | "space-between"
            | "space-around" | "border-box" | "content-box"
    )
}

fn is_css_tag_selector(s: &str) -> bool {
    matches!(
        s,
        "html" | "body" | "div" | "span" | "p" | "a" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "ul" | "ol" | "li" | "table" | "tr" | "td" | "th" | "form" | "input" | "button"
            | "label" | "select" | "textarea" | "section" | "header" | "footer" | "nav"
            | "main" | "article" | "aside" | "img" | "svg" | "canvas" | "code" | "pre"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_css_selectors_properties_units_colors_comments() {
        let g = CssGrammar;
        let line = ".btn { color: #ff00aa; margin: 12px; width: 100%; } /* box */";
        let (tokens, state) = g.tokenize_line(line, None);
        assert!(state.is_none());
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::EntityNameType && &line[t.start..t.end] == ".btn"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Property && &line[t.start..t.end] == "color"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::ConstantNumeric && &line[t.start..t.end] == "#ff00aa"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Number && &line[t.start..t.end] == "12px"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Number && &line[t.start..t.end] == "100%"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::CommentBlock));
    }

    #[test]
    fn test_css_multiline_comment_and_edges() {
        let g = CssGrammar;
        assert!(g.tokenize_line("", None).0.is_empty());
        assert!(g.tokenize_line(" \t\r", None).0.is_empty());

        let (_, s1) = g.tokenize_line("/* 🚀 café", None);
        assert_eq!(s1, Some(GrammarState::BlockComment { depth: 1 }));
        let (t2, s2) = g.tokenize_line("still comment */ body { padding: .5rem; }\r", s1.as_ref());
        assert_eq!(s2, None);
        assert_eq!(t2[0].scope, SyntaxScope::CommentBlock);
        assert!(t2.iter().any(|t| {
            t.scope == SyntaxScope::Number
                && line_slice("still comment */ body { padding: .5rem; }\r", t) == ".5rem"
        }));
    }

    fn line_slice<'a>(s: &'a str, t: &TokenSpan) -> &'a str {
        &s[t.start..t.end]
    }
}
