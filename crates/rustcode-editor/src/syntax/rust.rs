//! Rust language tokenizer and grammar implementation.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct RustGrammar;

impl Grammar for RustGrammar {
    fn language_id(&self) -> &'static str {
        "rust"
    }

    fn numeric_id(&self) -> u8 {
        1
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }

    fn tokenize_line(
        &self,
        line: &str,
        state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>) {
        let lang_id = self.numeric_id();
        let mut tokens = Vec::new();
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        let current_state = state.cloned();

        // 1. Resume from multi-line block comment
        if let Some(GrammarState::BlockComment { mut depth }) = current_state {
            let start = i;
            while i + 1 < len {
                if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            if i >= len && depth > 0 {
                tokens.push(TokenSpan::new(start, len, SyntaxScope::CommentBlock, lang_id));
                return (tokens, Some(GrammarState::BlockComment { depth }));
            } else {
                tokens.push(TokenSpan::new(start, i, SyntaxScope::CommentBlock, lang_id));
            }
        }


        while i < len {
            let b = bytes[i];

            // Whitespace: skip
            if b.is_ascii_whitespace() {
                i += 1;
                continue;
            }

            // Line comment or Block comment start
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
                    let mut depth = 1;
                    i += 2;
                    while i + 1 < len {
                        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                            depth += 1;
                            i += 2;
                        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            depth -= 1;
                            i += 2;
                            if depth == 0 {
                                break;
                            }
                        } else {
                            i += 1;
                        }
                    }
                    if depth > 0 {
                        tokens.push(TokenSpan::new(start, len, SyntaxScope::CommentBlock, lang_id));
                        return (tokens, Some(GrammarState::BlockComment { depth }));
                    } else {
                        tokens.push(TokenSpan::new(start, i, SyntaxScope::CommentBlock, lang_id));
                        continue;
                    }
                }
            }

            // Raw string r#"..."# or r"..."
            if b == b'r' && i + 1 < len && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
                let start = i;
                let mut hashes = 0;
                i += 1;
                while i < len && bytes[i] == b'#' {
                    hashes += 1;
                    i += 1;
                }
                if i < len && bytes[i] == b'"' {
                    i += 1;
                    let mut closed = false;
                    while i < len {
                        if bytes[i] == b'"' {
                            let mut end_hashes = 0;
                            let mut probe = i + 1;
                            while probe < len && bytes[probe] == b'#' && end_hashes < hashes {
                                end_hashes += 1;
                                probe += 1;
                            }
                            if end_hashes == hashes {
                                i = probe;
                                closed = true;
                                break;
                            }
                        }
                        i += 1;
                    }
                    tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
                    if !closed {
                        // Raw string across lines can be extended if needed
                    }
                    continue;
                }
            }

            // String literal "..."
            if b == b'"' {
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
                continue;
            }

            // Char literal or lifetime 'a'
            if b == b'\'' {
                let start = i;
                i += 1;
                if i < len && bytes[i] != b'\\' && i + 1 < len && bytes[i + 1] == b'\'' {
                    tokens.push(TokenSpan::new(start, i + 2, SyntaxScope::String, lang_id));
                    i += 2;
                    continue;
                } else if i < len && bytes[i] == b'\\' && i + 2 < len && bytes[i + 2] == b'\'' {
                    tokens.push(TokenSpan::new(start, i + 3, SyntaxScope::String, lang_id));
                    i += 3;
                    continue;
                } else if i < len && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
                    // Lifetime e.g. 'static
                    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                    tokens.push(TokenSpan::new(start, i, SyntaxScope::KeywordOther, lang_id));
                    continue;
                }
            }

            // Attributes: #[...] or #![...]
            if b == b'#' && i + 1 < len && (bytes[i + 1] == b'[' || (bytes[i + 1] == b'!' && i + 2 < len && bytes[i + 2] == b'[')) {
                let start = i;
                while i < len && bytes[i] != b']' {
                    i += 1;
                }
                if i < len && bytes[i] == b']' {
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Property, lang_id));
                continue;
            }

            // Number: 0x..., 0b..., digits
            if b.is_ascii_digit() {
                let start = i;
                if b == b'0' && i + 1 < len && (bytes[i + 1] == b'x' || bytes[i + 1] == b'b' || bytes[i + 1] == b'o') {
                    i += 2;
                    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                } else {
                    while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'_') {
                        // Avoid treating method call or range (e.g. 0..10 or 0.foo()) as float
                        if bytes[i] == b'.' && i + 1 < len && (bytes[i + 1] == b'.' || !bytes[i + 1].is_ascii_digit()) {
                            break;
                        }
                        i += 1;
                    }
                    // Type suffix e.g. 100u64, 1.0f32
                    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Number, lang_id));
                continue;
            }

            // Identifiers, keywords, types, functions, macros
            if b.is_ascii_alphabetic() || b == b'_' {
                let start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &line[start..i];

                // Check macro e.g. println!
                if i < len && bytes[i] == b'!' && (i + 1 == len || bytes[i + 1] != b'=') {
                    i += 1;
                    tokens.push(TokenSpan::new(start, i, SyntaxScope::Function, lang_id));
                    continue;
                }

                // Check function call: next non-whitespace char is '('
                let mut is_fn = false;
                let mut peek = i;
                while peek < len && bytes[peek].is_ascii_whitespace() {
                    peek += 1;
                }
                if peek < len && bytes[peek] == b'(' && !is_rust_keyword(word) {
                    is_fn = true;
                }

                let scope = if is_fn {
                    SyntaxScope::Function
                } else if is_rust_keyword(word) {
                    match word {
                        "true" | "false" => SyntaxScope::ConstantLanguage,
                        "let" | "mut" | "fn" | "struct" | "enum" | "trait" | "impl" | "type"
                        | "pub" | "const" | "static" | "mod" | "use" => SyntaxScope::Keyword,
                        "if" | "else" | "match" | "for" | "while" | "loop" | "return" | "break"
                        | "continue" | "yield" | "await" | "async" => SyntaxScope::KeywordControl,
                        _ => SyntaxScope::Keyword,
                    }
                } else if is_rust_type(word) {
                    SyntaxScope::Type
                } else {
                    SyntaxScope::Variable
                };

                tokens.push(TokenSpan::new(start, i, scope, lang_id));
                continue;
            }

            // Operators & Punctuation
            let start = i;
            if i + 1 < len {
                let op2 = &line[i..i + 2];
                if matches!(op2, "->" | "=>" | "::" | "==" | "!=" | "<=" | ">=" | "&&" | "||" | "+=" | "-=" | "*=" | "/=") {
                    tokens.push(TokenSpan::new(start, start + 2, SyntaxScope::Operator, lang_id));
                    i += 2;
                    continue;
                }
            }

            let scope = match b {
                b'(' | b')' | b'[' | b']' | b'{' => SyntaxScope::PunctuationBracket,
                b';' | b',' | b':' | b'.' => SyntaxScope::PunctuationDelimiter,
                b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'!' | b'=' | b'<' | b'>' => {
                    SyntaxScope::Operator
                }
                _ => SyntaxScope::Plain,
            };
            tokens.push(TokenSpan::new(start, start + 1, scope, lang_id));
            i += 1;
        }

        (tokens, None)
    }
}

fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else"
            | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let"
            | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "self"
            | "Self" | "static" | "struct" | "super" | "trait" | "true" | "type" | "unsafe"
            | "use" | "where" | "while" | "yield"
    )
}

fn is_rust_type(s: &str) -> bool {
    matches!(
        s,
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64"
            | "u128" | "usize" | "f32" | "f64" | "bool" | "char" | "str" | "Option"
            | "Result" | "Vec" | "String" | "Box" | "Arc" | "Rc" | "Cow" | "Path" | "PathBuf"
    ) || (s.starts_with(|c: char| c.is_ascii_uppercase()) && !s.contains('_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_tokenizer_keywords_and_comments() {
        let grammar = RustGrammar;
        let line = "pub fn calculate() -> i32 { // return value";
        let (tokens, state) = grammar.tokenize_line(line, None);
        assert!(state.is_none());

        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Keyword && &line[t.start..t.end] == "pub"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Keyword && &line[t.start..t.end] == "fn"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Function && &line[t.start..t.end] == "calculate"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Type && &line[t.start..t.end] == "i32"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::CommentLine));
    }

    #[test]
    fn test_rust_multiline_block_comments() {
        let grammar = RustGrammar;
        let line1 = "/* start block";
        let (tokens1, state1) = grammar.tokenize_line(line1, None);
        assert_eq!(state1, Some(GrammarState::BlockComment { depth: 1 }));
        assert_eq!(tokens1[0].scope, SyntaxScope::CommentBlock);

        let line2 = "middle of block */ let x = 42;";
        let (tokens2, state2) = grammar.tokenize_line(line2, state1.as_ref());
        assert_eq!(state2, None);
        assert_eq!(tokens2[0].scope, SyntaxScope::CommentBlock);
        assert!(tokens2.iter().any(|t| t.scope == SyntaxScope::Keyword && &line2[t.start..t.end] == "let"));
        assert!(tokens2.iter().any(|t| t.scope == SyntaxScope::Number && &line2[t.start..t.end] == "42"));
    }
}
