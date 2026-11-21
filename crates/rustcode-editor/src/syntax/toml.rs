//! TOML language tokenizer and grammar implementation.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct TomlGrammar;

impl Grammar for TomlGrammar {
    fn language_id(&self) -> &'static str {
        "toml"
    }

    fn numeric_id(&self) -> u8 {
        12
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["toml"]
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
        let mut failed_eq_until = 0usize;

        // Resume multi-line string """...""" or '''...'''
        if let Some(GrammarState::MultiLineString {
            quote: quote @ ('"' | '\''),
        }) = state
        {
            let q = *quote as u8;
            let has_escapes = *quote == '"';
            let start = i;
            let closed = scan_triple_quoted(line, &mut i, q, has_escapes);
            tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
            if !closed {
                return (tokens, Some(GrammarState::MultiLineString { quote: *quote }));
            }
        } else {
            // Check table header [section] or [[array]] at start of line
            let mut lead = 0;
            while lead < len && bytes[lead].is_ascii_whitespace() {
                lead += 1;
            }
            if lead < len && bytes[lead] == b'[' {
                i = lead;
                let bracket_start = i;
                i += 1;
                if i < len && bytes[i] == b'[' {
                    i += 1;
                }
                tokens.push(TokenSpan::new(
                    bracket_start,
                    i,
                    SyntaxScope::PunctuationBracket,
                    lang_id,
                ));

                while i < len {
                    let b = bytes[i];
                    if b.is_ascii_whitespace() {
                        i += 1;
                        continue;
                    }
                    if b == b'#' {
                        break;
                    }
                    if b == b']' {
                        let end_start = i;
                        i += 1;
                        if i < len && bytes[i] == b']' {
                            i += 1;
                        }
                        tokens.push(TokenSpan::new(
                            end_start,
                            i,
                            SyntaxScope::PunctuationBracket,
                            lang_id,
                        ));
                        break;
                    }
                    if b == b'.' {
                        tokens.push(TokenSpan::new(
                            i,
                            i + 1,
                            SyntaxScope::PunctuationDelimiter,
                            lang_id,
                        ));
                        i += 1;
                        continue;
                    }
                    if b == b'"' || b == b'\'' {
                        let start = i;
                        scan_single_line_string(line, &mut i, b, b == b'"');
                        tokens.push(TokenSpan::new(start, i, SyntaxScope::Property, lang_id));
                        continue;
                    }
                    if is_bare_key_byte(b) {
                        let start = i;
                        while i < len && is_bare_key_byte(bytes[i]) {
                            i += 1;
                        }
                        tokens.push(TokenSpan::new(start, i, SyntaxScope::Type, lang_id));
                        continue;
                    }
                    let ch = line[i..].chars().next().unwrap();
                    let ch_len = ch.len_utf8();
                    tokens.push(TokenSpan::new(i, i + ch_len, SyntaxScope::Plain, lang_id));
                    i += ch_len;
                }
            }
        }

        while i < len {
            let b = bytes[i];

            if b.is_ascii_whitespace() {
                i += 1;
                continue;
            }

            // Line comment #
            if b == b'#' {
                tokens.push(TokenSpan::new(i, len, SyntaxScope::CommentLine, lang_id));
                return (tokens, None);
            }

            // Triple-quoted strings """...""" or '''...'''
            if (b == b'"' || b == b'\'') && i + 2 < len && bytes[i + 1] == b && bytes[i + 2] == b {
                let quote = b as char;
                let start = i;
                i += 3;
                let closed = scan_triple_quoted(line, &mut i, b, b == b'"');
                tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
                if !closed {
                    return (tokens, Some(GrammarState::MultiLineString { quote }));
                }
                continue;
            }

            // Single-line basic "..." or literal '...' string (or quoted key)
            if b == b'"' || b == b'\'' {
                let start = i;
                scan_single_line_string(line, &mut i, b, b == b'"');
                let scope = if has_toml_equals_ahead(bytes, i, &mut failed_eq_until) {
                    SyntaxScope::Property
                } else {
                    SyntaxScope::String
                };
                tokens.push(TokenSpan::new(start, i, scope, lang_id));
                continue;
            }

            // Bare key followed by '=' (or dotted key segment before '=')
            if is_bare_key_byte(b) {
                let mut key_end = i;
                while key_end < len && is_bare_key_byte(bytes[key_end]) {
                    key_end += 1;
                }
                if has_toml_equals_ahead(bytes, key_end, &mut failed_eq_until) {
                    tokens.push(TokenSpan::new(i, key_end, SyntaxScope::Property, lang_id));
                    i = key_end;
                    continue;
                }
            }

            // Special float constants: +inf, -inf, +nan, -nan
            if (b == b'+' || b == b'-')
                && i + 3 < len
                && (bytes[i + 1..].starts_with(b"inf") || bytes[i + 1..].starts_with(b"nan"))
                && (i + 4 == len || !is_bare_key_byte(bytes[i + 4]))
            {
                tokens.push(TokenSpan::new(i, i + 4, SyntaxScope::Number, lang_id));
                i += 4;
                continue;
            }

            // Numbers and TOML dates/times
            if b.is_ascii_digit()
                || ((b == b'+' || b == b'-') && i + 1 < len && bytes[i + 1].is_ascii_digit())
            {
                let start = i;
                if b == b'+' || b == b'-' {
                    i += 1;
                }
                if i + 1 < len
                    && bytes[i] == b'0'
                    && matches!(bytes[i + 1], b'x' | b'o' | b'b')
                {
                    i += 2;
                    while i < len && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                        i += 1;
                    }
                } else {
                    while i < len
                        && (bytes[i].is_ascii_digit()
                            || matches!(
                                bytes[i],
                                b'_' | b'.' | b':' | b'-' | b'+' | b'T' | b't' | b'Z' | b'z' | b'e' | b'E'
                            ))
                    {
                        i += 1;
                    }
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Number, lang_id));
                continue;
            }

            // Identifiers / booleans / inf / nan
            if b.is_ascii_alphabetic() || b == b'_' {
                let start = i;
                while i < len && is_bare_key_byte(bytes[i]) {
                    i += 1;
                }
                let word = &line[start..i];
                let scope = match word {
                    "true" | "false" => SyntaxScope::ConstantLanguage,
                    "inf" | "nan" => SyntaxScope::Number,
                    _ => SyntaxScope::Variable,
                };
                tokens.push(TokenSpan::new(start, i, scope, lang_id));
                continue;
            }

            // Punctuation & unicode-safe fallback
            let ch = line[i..].chars().next().unwrap();
            let ch_len = ch.len_utf8();
            let scope = match b {
                b'[' | b']' | b'{' | b'}' => SyntaxScope::PunctuationBracket,
                b',' | b'.' => SyntaxScope::PunctuationDelimiter,
                b'=' => SyntaxScope::Operator,
                _ => SyntaxScope::Plain,
            };
            tokens.push(TokenSpan::new(i, i + ch_len, scope, lang_id));
            i += ch_len;
        }

        (tokens, None)
    }
}

fn is_bare_key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn scan_single_line_string(line: &str, i: &mut usize, quote: u8, has_escapes: bool) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    *i += 1;
    while *i < len {
        if has_escapes && bytes[*i] == b'\\' {
            *i += 1;
            if *i < len {
                let esc = line[*i..].chars().next().unwrap();
                *i += esc.len_utf8();
            }
        } else if bytes[*i] == quote {
            *i += 1;
            break;
        } else {
            let ch = line[*i..].chars().next().unwrap();
            *i += ch.len_utf8();
        }
    }
}

fn scan_triple_quoted(line: &str, i: &mut usize, quote: u8, has_escapes: bool) -> bool {
    let bytes = line.as_bytes();
    let len = bytes.len();
    while *i < len {
        if has_escapes && bytes[*i] == b'\\' {
            *i += 1;
            if *i < len {
                let esc = line[*i..].chars().next().unwrap();
                *i += esc.len_utf8();
            }
        } else if *i + 2 < len
            && bytes[*i] == quote
            && bytes[*i + 1] == quote
            && bytes[*i + 2] == quote
        {
            *i += 3;
            // TOML allows up to 2 extra quotes immediately before closing delimiter
            let mut extra = 0;
            while *i < len && bytes[*i] == quote && extra < 2 {
                *i += 1;
                extra += 1;
            }
            return true;
        } else {
            let ch = line[*i..].chars().next().unwrap();
            *i += ch.len_utf8();
        }
    }
    false
}

fn has_toml_equals_ahead(bytes: &[u8], mut p: usize, failed_until: &mut usize) -> bool {
    let len = bytes.len();
    if p < *failed_until {
        return false;
    }
    loop {
        while p < len && (bytes[p] == b' ' || bytes[p] == b'\t') {
            p += 1;
        }
        if p < len && bytes[p] == b'=' {
            return true;
        }
        if p < len && bytes[p] == b'.' {
            p += 1;
            while p < len && (bytes[p] == b' ' || bytes[p] == b'\t') {
                p += 1;
            }
            if p < len && (bytes[p] == b'"' || bytes[p] == b'\'') {
                let q = bytes[p];
                p += 1;
                while p < len {
                    if q == b'"' && bytes[p] == b'\\' {
                        p = (p + 2).min(len);
                    } else if bytes[p] == q {
                        p += 1;
                        break;
                    } else {
                        p += 1;
                    }
                }
            } else if p < len && is_bare_key_byte(bytes[p]) {
                while p < len && is_bare_key_byte(bytes[p]) {
                    p += 1;
                }
            } else {
                *failed_until = p;
                return false;
            }
        } else {
            *failed_until = p;
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toml_headers_keys_values_comments() {
        let g = TomlGrammar;
        let (hdr, _) = g.tokenize_line("[dependencies.serde]", None);
        assert!(hdr.iter().any(|t| t.scope == SyntaxScope::Type));
        assert!(hdr.iter().any(|t| t.scope == SyntaxScope::PunctuationBracket));

        let line = "name = \"rustcode\" # pkg";
        let (tokens, state) = g.tokenize_line(line, None);
        assert!(state.is_none());
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Property && &line[t.start..t.end] == "name"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::String && &line[t.start..t.end] == "\"rustcode\""));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::CommentLine));

        let (num_tokens, _) = g.tokenize_line("created = 2026-01-01T12:00:00Z", None);
        assert!(num_tokens.iter().any(|t| t.scope == SyntaxScope::Number));
    }

    #[test]
    fn test_toml_multiline_strings_and_edges() {
        let g = TomlGrammar;
        assert!(g.tokenize_line("", None).0.is_empty());
        assert!(g.tokenize_line(" \t\r", None).0.is_empty());

        let (_, s1) = g.tokenize_line("desc = \"\"\"first line", None);
        assert_eq!(s1, Some(GrammarState::MultiLineString { quote: '"' }));
        let (t2, s2) = g.tokenize_line("second 🚀 line", s1.as_ref());
        assert_eq!(s2, Some(GrammarState::MultiLineString { quote: '"' }));
        assert_eq!(t2[0].scope, SyntaxScope::String);
        let (t3, s3) = g.tokenize_line("end\"\"\" # done\r", s2.as_ref());
        assert_eq!(s3, None);
        assert_eq!(t3[0].scope, SyntaxScope::String);
        assert!(t3.iter().any(|t| t.scope == SyntaxScope::CommentLine));
    }
}
