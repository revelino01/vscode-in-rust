//! C++ language tokenizer and grammar implementation.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct CppGrammar;

impl Grammar for CppGrammar {
    fn language_id(&self) -> &'static str {
        "cpp"
    }

    fn numeric_id(&self) -> u8 {
        9
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["cpp", "cc", "cxx", "hpp", "hh", "hxx"]
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

        // Resume multi-line C++ raw string R"delim(...)delim"
        if let Some(GrammarState::RawString { delimiter }) = state {
            let start = i;
            let mut closed = false;
            while i < len {
                if bytes[i] == b')' {
                    let after_paren = i + 1;
                    if line[after_paren..].starts_with(delimiter.as_str()) {
                        let after_delim = after_paren + delimiter.len();
                        if after_delim < len && bytes[after_delim] == b'"' {
                            i = after_delim + 1;
                            closed = true;
                            break;
                        }
                    }
                }
                let ch = line[i..].chars().next().unwrap();
                i += ch.len_utf8();
            }
            tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
            if !closed {
                return (
                    tokens,
                    Some(GrammarState::RawString {
                        delimiter: delimiter.clone(),
                    }),
                );
            }
        } else if let Some(GrammarState::BlockComment { .. }) = state {
            // Resume non-nesting block comment /* ... */
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
                    let scope = if i + 2 < len && bytes[i + 2] == b'/' {
                        SyntaxScope::CommentDoc
                    } else {
                        SyntaxScope::CommentLine
                    };
                    tokens.push(TokenSpan::new(i, len, scope, lang_id));
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

            // Raw string literal: [u8|u|U|L]R"delim(...)delim"
            let raw_r_pos = if b == b'R' && i + 1 < len && bytes[i + 1] == b'"' {
                Some(i)
            } else if matches!(b, b'u' | b'U' | b'L')
                && i + 2 < len
                && bytes[i + 1] == b'R'
                && bytes[i + 2] == b'"'
            {
                Some(i + 1)
            } else if b == b'u'
                && i + 3 < len
                && bytes[i + 1] == b'8'
                && bytes[i + 2] == b'R'
                && bytes[i + 3] == b'"'
            {
                Some(i + 2)
            } else {
                None
            };

            if let Some(r_pos) = raw_r_pos {
                let delim_start = r_pos + 2;
                let mut d = delim_start;
                let mut valid_delim = false;
                while d < len && d - delim_start <= 16 {
                    if bytes[d] == b'(' {
                        valid_delim = true;
                        break;
                    }
                    if bytes[d].is_ascii_whitespace() || bytes[d] == b')' || bytes[d] == b'\\' {
                        break;
                    }
                    let ch = line[d..].chars().next().unwrap();
                    d += ch.len_utf8();
                }

                if valid_delim {
                    let start = i;
                    let delim = &line[delim_start..d];
                    i = d + 1;
                    let mut closed = false;
                    while i < len {
                        if bytes[i] == b')' {
                            let after_paren = i + 1;
                            if line[after_paren..].starts_with(delim) {
                                let after_delim = after_paren + delim.len();
                                if after_delim < len && bytes[after_delim] == b'"' {
                                    i = after_delim + 1;
                                    closed = true;
                                    break;
                                }
                            }
                        }
                        let ch = line[i..].chars().next().unwrap();
                        i += ch.len_utf8();
                    }
                    tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
                    if !closed {
                        return (
                            tokens,
                            Some(GrammarState::RawString {
                                delimiter: delim.to_string(),
                            }),
                        );
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

            // Numbers (with C++14 digit separators `'`)
            if b.is_ascii_digit()
                || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit())
            {
                let start = i;
                if b == b'0' && i + 1 < len && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B') {
                    let prefix = bytes[i + 1].to_ascii_lowercase();
                    i += 2;
                    while i < len
                        && (bytes[i].is_ascii_hexdigit()
                            || (bytes[i] == b'\''
                                && i + 1 < len
                                && bytes[i + 1].is_ascii_hexdigit())
                            || (prefix == b'x'
                                && (bytes[i] == b'.' || bytes[i] == b'p' || bytes[i] == b'P')))
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
                while i < len && matches!(bytes[i], b'u' | b'U' | b'l' | b'L' | b'f' | b'F' | b'z' | b'Z') {
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

                let scope = if is_cpp_control_keyword(word) {
                    SyntaxScope::KeywordControl
                } else if is_cpp_keyword(word) {
                    SyntaxScope::Keyword
                } else if matches!(word, "true" | "false" | "nullptr" | "NULL") {
                    SyntaxScope::ConstantLanguage
                } else if is_cpp_type(word) {
                    SyntaxScope::StorageType
                } else if followed_by_paren {
                    SyntaxScope::Function
                } else if word.starts_with(|c: char| c.is_ascii_uppercase()) || word.ends_with("_t") {
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
                if matches!(op3, b"<=>" | b"<<=" | b">>=" | b"->*" | b"...") {
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
                    b"::" | b"->" | b".*" | b"++" | b"--" | b"<<" | b">>" | b"<=" | b">="
                        | b"==" | b"!=" | b"&&" | b"||" | b"+=" | b"-=" | b"*=" | b"/="
                        | b"%=" | b"&=" | b"|=" | b"^="
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

fn is_cpp_control_keyword(s: &str) -> bool {
    matches!(
        s,
        "break" | "case" | "catch" | "co_await" | "co_return" | "co_yield" | "continue"
            | "default" | "do" | "else" | "for" | "goto" | "if" | "return" | "switch"
            | "throw" | "try" | "while"
    )
}

fn is_cpp_keyword(s: &str) -> bool {
    matches!(
        s,
        "alignas" | "alignof" | "and" | "and_eq" | "asm" | "auto" | "bitand" | "bitor"
            | "class" | "compl" | "concept" | "const" | "consteval" | "constexpr" | "constinit"
            | "const_cast" | "decltype" | "delete" | "dynamic_cast" | "enum" | "explicit"
            | "export" | "extern" | "final" | "friend" | "import" | "inline" | "module"
            | "mutable" | "namespace" | "new" | "noexcept" | "not" | "not_eq" | "operator"
            | "or" | "or_eq" | "override" | "private" | "protected" | "public" | "register"
            | "reinterpret_cast" | "requires" | "sizeof" | "static" | "static_assert"
            | "static_cast" | "struct" | "template" | "this" | "thread_local" | "typedef"
            | "typeid" | "typename" | "union" | "using" | "virtual" | "volatile" | "xor"
            | "xor_eq"
    )
}

fn is_cpp_type(s: &str) -> bool {
    matches!(
        s,
        "void" | "bool" | "char" | "char8_t" | "char16_t" | "char32_t" | "wchar_t" | "short"
            | "int" | "long" | "float" | "double" | "signed" | "unsigned" | "size_t"
            | "ssize_t" | "ptrdiff_t" | "intptr_t" | "uintptr_t" | "int8_t" | "int16_t"
            | "int32_t" | "int64_t" | "uint8_t" | "uint16_t" | "uint32_t" | "uint64_t"
            | "string" | "string_view" | "vector" | "map" | "unordered_map" | "set"
            | "unordered_set" | "pair" | "tuple" | "optional" | "variant" | "unique_ptr"
            | "shared_ptr" | "weak_ptr" | "span" | "array"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpp_keywords_preprocessor_and_raw_strings() {
        let g = CppGrammar;
        let (inc, _) = g.tokenize_line("#include <vector>", None);
        assert!(inc.iter().any(|t| t.scope == SyntaxScope::KeywordOther));
        assert!(inc.iter().any(|t| t.scope == SyntaxScope::String));

        let (t1, s1) = g.tokenize_line("const char* s = R\"EOF(hello )", None);
        assert_eq!(s1, Some(GrammarState::RawString { delimiter: "EOF".to_string() }));
        assert!(t1.iter().any(|t| t.scope == SyntaxScope::String));

        let (t2, s2) = g.tokenize_line("still raw", s1.as_ref());
        assert_eq!(s2, Some(GrammarState::RawString { delimiter: "EOF".to_string() }));
        assert_eq!(t2[0].scope, SyntaxScope::String);

        let (t3, s3) = g.tokenize_line("done)EOF\"; constexpr int n = 1'000;", s2.as_ref());
        assert_eq!(s3, None);
        assert_eq!(t3[0].scope, SyntaxScope::String);
        assert!(t3.iter().any(|t| t.scope == SyntaxScope::Keyword));
        assert!(t3.iter().any(|t| t.scope == SyntaxScope::Number));
    }

    #[test]
    fn test_cpp_whitespace_crlf_utf8() {
        let g = CppGrammar;
        assert!(g.tokenize_line("", None).0.is_empty());
        assert!(g.tokenize_line("  \t\r", None).0.is_empty());

        let line = "std::string s = \"🚀 café\"; // 日本語\r";
        let (tokens, _) = g.tokenize_line(line, None);
        for t in &tokens {
            assert!(t.start < t.end && t.end <= line.len());
            assert!(line.is_char_boundary(t.start) && line.is_char_boundary(t.end));
            assert!(!line[t.start..t.end].ends_with('\r'));
        }
    }
}
