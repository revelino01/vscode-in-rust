//! C language tokenizer and grammar implementation.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct CGrammar;

impl Grammar for CGrammar {
    fn language_id(&self) -> &'static str {
        "c"
    }

    fn numeric_id(&self) -> u8 {
        8
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["c", "h"]
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
        } else {
            // Preprocessor directive at start of line
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < len && bytes[i] == b'#' {
                let start = i;
                i += 1;
                while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                    i += 1;
                }
                let dir_start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let directive = &line[dir_start..i];
                tokens.push(TokenSpan::new(start, i, SyntaxScope::KeywordOther, lang_id));

                if directive == "include" {
                    while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                        i += 1;
                    }
                    if i < len && bytes[i] == b'<' {
                        let inc_start = i;
                        i += 1;
                        while i < len {
                            if bytes[i] == b'>' {
                                i += 1;
                                break;
                            }
                            let ch = line[i..].chars().next().unwrap();
                            i += ch.len_utf8();
                        }
                        tokens.push(TokenSpan::new(inc_start, i, SyntaxScope::String, lang_id));
                    }
                }
            }
        }

        while i < len {
            let b = bytes[i];

            if b.is_ascii_whitespace() {
                i += 1;
                continue;
            }

            // Comments
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

            // Prefixed or standard strings/char literals
            let prefix_len = if b == b'u'
                && i + 2 < len
                && bytes[i + 1] == b'8'
                && (bytes[i + 2] == b'"' || bytes[i + 2] == b'\'')
            {
                2
            } else if matches!(b, b'L' | b'u' | b'U')
                && i + 1 < len
                && (bytes[i + 1] == b'"' || bytes[i + 1] == b'\'')
            {
                1
            } else {
                0
            };

            if prefix_len > 0 || b == b'"' || b == b'\'' {
                let start = i;
                i += prefix_len;
                let quote = bytes[i];
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

            // Numbers
            if b.is_ascii_digit()
                || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit())
            {
                let start = i;
                if b == b'0' && i + 1 < len && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B') {
                    let prefix = bytes[i + 1].to_ascii_lowercase();
                    i += 2;
                    while i < len
                        && (bytes[i].is_ascii_hexdigit()
                            || bytes[i] == b'\''
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
                            || bytes[i] == b'.'
                            || bytes[i] == b'e'
                            || bytes[i] == b'E'
                            || (bytes[i] == b'\'' && i + 1 < len && bytes[i + 1].is_ascii_digit()))
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
                }
                while i < len && matches!(bytes[i], b'u' | b'U' | b'l' | b'L' | b'f' | b'F') {
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Number, lang_id));
                continue;
            }

            let ch = line[i..].chars().next().unwrap();
            let ch_len = ch.len_utf8();

            // Identifiers, keywords, types, functions
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

                let scope = if is_c_control_keyword(word) {
                    SyntaxScope::KeywordControl
                } else if is_c_keyword(word) {
                    SyntaxScope::Keyword
                } else if matches!(word, "NULL" | "true" | "false" | "nullptr") {
                    SyntaxScope::ConstantLanguage
                } else if is_c_type(word) {
                    SyntaxScope::StorageType
                } else if followed_by_paren {
                    SyntaxScope::Function
                } else if word.ends_with("_t") {
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
                if matches!(op3, b"<<=" | b">>=" | b"...") {
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
                    b"->" | b"++" | b"--" | b"<<" | b">>" | b"<=" | b">=" | b"==" | b"!="
                        | b"&&" | b"||" | b"+=" | b"-=" | b"*=" | b"/=" | b"%=" | b"&="
                        | b"|=" | b"^=" | b"##"
                ) {
                    tokens.push(TokenSpan::new(start, start + 2, SyntaxScope::Operator, lang_id));
                    i += 2;
                    continue;
                }
            }

            let scope = match b {
                b'(' | b')' | b'[' | b']' | b'{' | b'}' => SyntaxScope::PunctuationBracket,
                b';' | b',' | b':' | b'.' => SyntaxScope::PunctuationDelimiter,
                b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'~' | b'!' | b'='
                | b'<' | b'>' | b'?' => SyntaxScope::Operator,
                _ => SyntaxScope::Plain,
            };
            tokens.push(TokenSpan::new(start, start + ch_len, scope, lang_id));
            i += ch_len;
        }

        (tokens, None)
    }
}

fn is_c_control_keyword(s: &str) -> bool {
    matches!(
        s,
        "break" | "case" | "continue" | "default" | "do" | "else" | "for" | "goto" | "if"
            | "return" | "switch" | "while"
    )
}

fn is_c_keyword(s: &str) -> bool {
    matches!(
        s,
        "alignas" | "alignof" | "auto" | "const" | "constexpr" | "enum" | "extern" | "inline"
            | "register" | "restrict" | "sizeof" | "static" | "static_assert" | "struct"
            | "thread_local" | "typedef" | "typeof" | "union" | "volatile" | "_Alignas"
            | "_Alignof" | "_Atomic" | "_Generic" | "_Noreturn" | "_Static_assert"
            | "_Thread_local"
    )
}

fn is_c_type(s: &str) -> bool {
    matches!(
        s,
        "void" | "char" | "short" | "int" | "long" | "float" | "double" | "signed" | "unsigned"
            | "bool" | "_Bool" | "_Complex" | "_Imaginary" | "size_t" | "ssize_t" | "ptrdiff_t"
            | "intptr_t" | "uintptr_t" | "int8_t" | "int16_t" | "int32_t" | "int64_t"
            | "uint8_t" | "uint16_t" | "uint32_t" | "uint64_t" | "FILE" | "va_list"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_c_preprocessor_types_strings_comments() {
        let g = CGrammar;
        let (inc_tokens, _) = g.tokenize_line("#include <stdio.h>", None);
        assert!(inc_tokens.iter().any(|t| t.scope == SyntaxScope::KeywordOther));
        assert!(inc_tokens.iter().any(|t| t.scope == SyntaxScope::String));

        let line = "static int count = 0x2a; const char *s = \"hi\"; // ok";
        let (tokens, state) = g.tokenize_line(line, None);
        assert!(state.is_none());
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Keyword && &line[t.start..t.end] == "static"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::StorageType && &line[t.start..t.end] == "int"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Number && &line[t.start..t.end] == "0x2a"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::String && &line[t.start..t.end] == "\"hi\""));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::CommentLine));
    }

    #[test]
    fn test_c_multiline_block_comment_and_edges() {
        let g = CGrammar;
        assert!(g.tokenize_line("", None).0.is_empty());
        assert!(g.tokenize_line(" \t\r", None).0.is_empty());

        let (_, s1) = g.tokenize_line("/* start", None);
        assert_eq!(s1, Some(GrammarState::BlockComment { depth: 1 }));
        let (t2, s2) = g.tokenize_line("middle 🚀 */ return 0;\r", s1.as_ref());
        assert_eq!(s2, None);
        assert_eq!(t2[0].scope, SyntaxScope::CommentBlock);
        assert!(t2.iter().any(|t| t.scope == SyntaxScope::KeywordControl));
    }
}
