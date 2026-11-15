//! JSON language tokenizer.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct JsonGrammar;

impl Grammar for JsonGrammar {
    fn language_id(&self) -> &'static str {
        "json"
    }

    fn numeric_id(&self) -> u8 {
        5
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["json", "jsonc"]
    }

    fn tokenize_line(
        &self,
        line: &str,
        _state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>) {
        let lang_id = self.numeric_id();
        let mut tokens = Vec::new();
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            let b = bytes[i];

            if b.is_ascii_whitespace() {
                i += 1;
                continue;
            }

            // JSONC comments
            if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
                tokens.push(TokenSpan::new(i, len, SyntaxScope::CommentLine, lang_id));
                return (tokens, None);
            }

            // String or Object Key
            if b == b'"' {
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(len);
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }

                // Check if followed by colon ':' -> Property key
                let mut peek = i;
                while peek < len && bytes[peek].is_ascii_whitespace() {
                    peek += 1;
                }
                let scope = if peek < len && bytes[peek] == b':' {
                    SyntaxScope::Property
                } else {
                    SyntaxScope::String
                };

                tokens.push(TokenSpan::new(start, i.min(len), scope, lang_id));
                continue;
            }

            // Numbers
            if b.is_ascii_digit() || b == b'-' {
                let start = i;
                if b == b'-' {
                    i += 1;
                }
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'e' || bytes[i] == b'E' || bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Number, lang_id));
                continue;
            }

            // Constants: true, false, null
            if b.is_ascii_alphabetic() {
                let start = i;
                while i < len && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word = &line[start..i];
                let scope = if matches!(word, "true" | "false" | "null") {
                    SyntaxScope::ConstantLanguage
                } else {
                    SyntaxScope::Plain
                };
                tokens.push(TokenSpan::new(start, i, scope, lang_id));
                continue;
            }

            // Punctuation and unicode-safe fallback
            let ch = line[i..].chars().next().unwrap();
            let ch_len = ch.len_utf8();
            let scope = match ch {
                '{' | '}' | '[' | ']' => SyntaxScope::PunctuationBracket,
                ':' | ',' => SyntaxScope::PunctuationDelimiter,
                _ => SyntaxScope::Plain,
            };
            tokens.push(TokenSpan::new(i, i + ch_len, scope, lang_id));
            i += ch_len;
        }

        (tokens, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_tokenization() {
        let grammar = JsonGrammar;
        let line = r#"  "name": "RustCode", "count": 100, "active": true, "extra": null"#;
        let (tokens, _) = grammar.tokenize_line(line, None);

        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Property && &line[t.start..t.end] == "\"name\""));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::String && &line[t.start..t.end] == "\"RustCode\""));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Number && &line[t.start..t.end] == "100"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::ConstantLanguage && &line[t.start..t.end] == "true"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::ConstantLanguage && &line[t.start..t.end] == "null"));
    }

    #[test]
    fn test_json_unicode_and_escapes() {
        let grammar = JsonGrammar;
        let line = r#"  "café_key": "emoji 🚀", "val": "\"#;
        let (tokens, state) = grammar.tokenize_line(line, None);
        assert_eq!(state, None);

        for t in &tokens {
            assert!(line.is_char_boundary(t.start));
            assert!(line.is_char_boundary(t.end));
            assert!(t.end <= line.len());
        }
    }
}

