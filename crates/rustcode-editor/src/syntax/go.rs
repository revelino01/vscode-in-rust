//! Go language tokenizer and grammar implementation.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct GoGrammar;

impl Grammar for GoGrammar {
    fn language_id(&self) -> &'static str {
        "go"
    }

    fn numeric_id(&self) -> u8 {
        7
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["go"]
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

        // Resume multi-line backtick raw string `...` (no backslash escapes in Go)
        if let Some(GrammarState::MultiLineString { quote: '`' }) = state {
            let start = i;
            let mut closed = false;
            while i < len {
                if bytes[i] == b'`' {
                    i += 1;
                    closed = true;
                    break;
                }
                let ch = line[i..].chars().next().unwrap();
                i += ch.len_utf8();
            }
            tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
            if !closed {
                return (tokens, Some(GrammarState::MultiLineString { quote: '`' }));
            }
        }

        // Resume non-nesting multi-line block comment /* ... */
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

            // Line comment // or Block comment /* ... */ (non-nesting)
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

            // Raw backtick string `...`
            if b == b'`' {
                let start = i;
                i += 1;
                let mut closed = false;
                while i < len {
                    if bytes[i] == b'`' {
                        i += 1;
                        closed = true;
                        break;
                    }
                    let ch = line[i..].chars().next().unwrap();
                    i += ch.len_utf8();
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
                if !closed {
                    return (tokens, Some(GrammarState::MultiLineString { quote: '`' }));
                }
                continue;
            }

            // Double-quoted string "..." or rune literal '...'
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

            // Numbers (hex, binary, octal, decimal, float, imaginary)
            if b.is_ascii_digit()
                || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit())
            {
                let start = i;
                if b == b'0' && i + 1 < len && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
                    let prefix = bytes[i + 1].to_ascii_lowercase();
                    i += 2;
                    while i < len
                        && (bytes[i].is_ascii_hexdigit()
                            || bytes[i] == b'_'
                            || (prefix == b'x' && (bytes[i] == b'.' || bytes[i] == b'p' || bytes[i] == b'P')))
                    {
                        if (bytes[i] == b'p' || bytes[i] == b'P')
                            && i + 1 < len
                            && (bytes[i + 1] == b'+' || bytes[i + 1] == b'-')
                        {
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                } else {
                    while i < len
                        && (bytes[i].is_ascii_digit()
                            || bytes[i] == b'_'
                            || bytes[i] == b'.'
                            || bytes[i] == b'e'
                            || bytes[i] == b'E')
                    {
                        if bytes[i] == b'.' && i + 1 < len && bytes[i + 1] == b'.' {
                            break;
                        }
                        if (bytes[i] == b'e' || bytes[i] == b'E')
                            && i + 1 < len
                            && (bytes[i + 1] == b'+' || bytes[i + 1] == b'-')
                        {
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                }
                if i < len && bytes[i] == b'i' {
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Number, lang_id));
                continue;
            }

            let ch = line[i..].chars().next().unwrap();
            let ch_len = ch.len_utf8();

            // Identifiers, keywords, types, builtins, functions
            if ch.is_alphabetic() || ch == '_' {
                let start = i;
                while i < len {
                    let next_ch = line[i..].chars().next().unwrap();
                    if next_ch.is_alphanumeric() || next_ch == '_' {
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
                let followed_by_paren = peek < len && bytes[peek] == b'(';

                let scope = if is_go_control_keyword(word) {
                    SyntaxScope::KeywordControl
                } else if is_go_keyword(word) {
                    SyntaxScope::Keyword
                } else if matches!(word, "true" | "false" | "iota" | "nil") {
                    SyntaxScope::ConstantLanguage
                } else if is_go_type(word) {
                    SyntaxScope::Type
                } else if followed_by_paren && is_go_builtin(word) {
                    SyntaxScope::SupportFunction
                } else if followed_by_paren {
                    SyntaxScope::Function
                } else if word.starts_with(|c: char| c.is_ascii_uppercase()) {
                    SyntaxScope::Type
                } else {
                    SyntaxScope::Variable
                };

                tokens.push(TokenSpan::new(start, i, scope, lang_id));
                continue;
            }

            // 3-char operators
            let start = i;
            if i + 2 < len {
                let op3 = &bytes[i..i + 3];
                if matches!(op3, b"<<=" | b">>=" | b"&^=" | b"...") {
                    tokens.push(TokenSpan::new(start, start + 3, SyntaxScope::Operator, lang_id));
                    i += 3;
                    continue;
                }
            }

            // 2-char operators
            if i + 1 < len {
                let op2 = &bytes[i..i + 2];
                if matches!(
                    op2,
                    b":=" | b"<-" | b"++" | b"--" | b"==" | b"!=" | b"<=" | b">=" | b"&&" | b"||"
                        | b"<<" | b">>" | b"&^" | b"+=" | b"-=" | b"*=" | b"/=" | b"%=" | b"&="
                        | b"|=" | b"^="
                ) {
                    tokens.push(TokenSpan::new(start, start + 2, SyntaxScope::Operator, lang_id));
                    i += 2;
                    continue;
                }
            }

            let scope = match b {
                b'(' | b')' | b'[' | b']' | b'{' | b'}' => SyntaxScope::PunctuationBracket,
                b';' | b',' | b':' | b'.' => SyntaxScope::PunctuationDelimiter,
                b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'!' | b'=' | b'<'
                | b'>' | b'~' => SyntaxScope::Operator,
                _ => SyntaxScope::Plain,
            };
            tokens.push(TokenSpan::new(start, start + ch_len, scope, lang_id));
            i += ch_len;
        }

        (tokens, None)
    }
}

fn is_go_control_keyword(s: &str) -> bool {
    matches!(
        s,
        "break"
            | "case"
            | "continue"
            | "default"
            | "defer"
            | "else"
            | "fallthrough"
            | "for"
            | "go"
            | "goto"
            | "if"
            | "range"
            | "return"
            | "select"
            | "switch"
    )
}

fn is_go_keyword(s: &str) -> bool {
    matches!(
        s,
        "chan" | "const" | "func" | "import" | "interface" | "map" | "package" | "struct"
            | "type" | "var"
    )
}

fn is_go_type(s: &str) -> bool {
    matches!(
        s,
        "any" | "bool" | "byte" | "comparable" | "complex64" | "complex128" | "error"
            | "float32" | "float64" | "int" | "int8" | "int16" | "int32" | "int64" | "rune"
            | "string" | "uint" | "uint8" | "uint16" | "uint32" | "uint64" | "uintptr"
    )
}

fn is_go_builtin(s: &str) -> bool {
    matches!(
        s,
        "append" | "cap" | "clear" | "close" | "complex" | "copy" | "delete" | "imag" | "len"
            | "make" | "max" | "min" | "new" | "panic" | "print" | "println" | "real" | "recover"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_go_keywords_types_numbers_comments() {
        let g = GoGrammar;
        let line = "func Compute(n int) string { return \"ok\" } // 0xff 42i";
        let (tokens, state) = g.tokenize_line(line, None);
        assert!(state.is_none());
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Keyword && &line[t.start..t.end] == "func"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Function && &line[t.start..t.end] == "Compute"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Type && &line[t.start..t.end] == "int"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::String && &line[t.start..t.end] == "\"ok\""));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::CommentLine));
    }

    #[test]
    fn test_go_multiline_backtick_and_non_nesting_block_comment() {
        let g = GoGrammar;
        let (t1, s1) = g.tokenize_line("raw := `line1 \\", None);
        assert_eq!(s1, Some(GrammarState::MultiLineString { quote: '`' }));
        assert_eq!(t1.last().unwrap().scope, SyntaxScope::String);

        let (t2, s2) = g.tokenize_line("line2` + 0x10", s1.as_ref());
        assert_eq!(s2, None);
        assert_eq!(t2[0].scope, SyntaxScope::String);
        assert!(t2.iter().any(|t| t.scope == SyntaxScope::Number));

        let (_, c1) = g.tokenize_line("/* nested /* ignored", None);
        assert_eq!(c1, Some(GrammarState::BlockComment { depth: 1 }));
        let (c_tokens, c2) = g.tokenize_line("closed */ var x = 1", c1.as_ref());
        assert_eq!(c2, None);
        assert_eq!(c_tokens[0].scope, SyntaxScope::CommentBlock);
        assert!(c_tokens.iter().any(|t| t.scope == SyntaxScope::Keyword));
    }

    #[test]
    fn test_go_whitespace_crlf_utf8() {
        let g = GoGrammar;
        assert!(g.tokenize_line("", None).0.is_empty());
        assert!(g.tokenize_line("  \t\r", None).0.is_empty());

        let line = "msg := \"🚀 café\" // 日本語\r";
        let (tokens, _) = g.tokenize_line(line, None);
        for t in &tokens {
            assert!(t.start < t.end && t.end <= line.len());
            assert!(line.is_char_boundary(t.start) && line.is_char_boundary(t.end));
            assert!(!line[t.start..t.end].ends_with('\r'));
        }
    }
}
