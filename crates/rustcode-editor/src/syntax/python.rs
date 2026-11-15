//! Python language tokenizer.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct PythonGrammar;

impl Grammar for PythonGrammar {
    fn language_id(&self) -> &'static str {
        "python"
    }

    fn numeric_id(&self) -> u8 {
        4
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["py", "pyw", "pyi"]
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


        // Resume triple-quoted string
        if let Some(GrammarState::MultiLineString { quote }) = current_state {
            let start = i;
            let target = [quote as u8, quote as u8, quote as u8];
            let mut closed = false;
            while i + 2 < len {
                if bytes[i] == target[0] && bytes[i + 1] == target[1] && bytes[i + 2] == target[2] {
                    i += 3;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                tokens.push(TokenSpan::new(start, len, SyntaxScope::String, lang_id));
                return (tokens, Some(GrammarState::MultiLineString { quote }));
            } else {
                tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
            }
        }

        while i < len {
            let b = bytes[i];

            if b.is_ascii_whitespace() {
                i += 1;
                continue;
            }

            // Comment
            if b == b'#' {
                tokens.push(TokenSpan::new(i, len, SyntaxScope::CommentLine, lang_id));
                return (tokens, None);
            }

            // Triple quote """ or '''
            if (b == b'"' || b == b'\'') && i + 2 < len && bytes[i + 1] == b && bytes[i + 2] == b {
                let quote = b as char;
                let start = i;
                i += 3;
                let mut closed = false;
                while i + 2 < len {
                    if bytes[i] == b && bytes[i + 1] == b && bytes[i + 2] == b {
                        i += 3;
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    tokens.push(TokenSpan::new(start, len, SyntaxScope::String, lang_id));
                    return (tokens, Some(GrammarState::MultiLineString { quote }));
                } else {
                    tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
                    continue;
                }
            }

            // String: f"...", r"...", or standard "..." / '...'
            let is_prefix = matches!(b, b'f' | b'F' | b'r' | b'R' | b'b' | b'B' | b'u' | b'U')
                && i + 1 < len
                && (bytes[i + 1] == b'"' || bytes[i + 1] == b'\'');

            if is_prefix || b == b'"' || b == b'\'' {
                let start = i;
                if is_prefix {
                    i += 1;
                }
                let quote = bytes[i];
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == quote {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
                continue;
            }

            // Decorator @name
            if b == b'@' {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.') {
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Function, lang_id));
                continue;
            }

            // Number
            if b.is_ascii_digit() {
                let start = i;
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'_') {
                    if bytes[i] == b'.' && i + 1 < len && !bytes[i + 1].is_ascii_digit() {
                        break;
                    }
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Number, lang_id));
                continue;
            }

            // Identifier or keyword
            if b.is_ascii_alphabetic() || b == b'_' {
                let start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &line[start..i];

                let mut is_fn = false;
                let mut peek = i;
                while peek < len && bytes[peek].is_ascii_whitespace() {
                    peek += 1;
                }
                if peek < len && bytes[peek] == b'(' && !is_py_keyword(word) {
                    is_fn = true;
                }

                let scope = if is_fn {
                    SyntaxScope::Function
                } else if is_py_keyword(word) {
                    match word {
                        "True" | "False" | "None" => SyntaxScope::ConstantLanguage,
                        "def" | "class" | "async" => SyntaxScope::Keyword,
                        "if" | "elif" | "else" | "for" | "while" | "try" | "except" | "finally"
                        | "with" | "return" | "yield" | "break" | "continue" | "raise"
                        | "import" | "from" | "as" => SyntaxScope::KeywordControl,
                        _ => SyntaxScope::Keyword,
                    }
                } else if is_py_type(word) {
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
                if matches!(op2, "==" | "!=" | "<=" | ">=" | "//" | "**" | "->" | "+=" | "-=" | "*=" | "/=") {
                    tokens.push(TokenSpan::new(start, start + 2, SyntaxScope::Operator, lang_id));
                    i += 2;
                    continue;
                }
            }

            let scope = match b {
                b'(' | b')' | b'[' | b']' | b'{' => SyntaxScope::PunctuationBracket,
                b';' | b',' | b':' | b'.' => SyntaxScope::PunctuationDelimiter,
                b'+' | b'-' | b'*' | b'/' | b'%' | b'=' | b'<' | b'>' => SyntaxScope::Operator,
                _ => SyntaxScope::Plain,
            };
            tokens.push(TokenSpan::new(start, start + 1, scope, lang_id));
            i += 1;
        }

        (tokens, None)
    }
}

fn is_py_keyword(s: &str) -> bool {
    matches!(
        s,
        "and" | "as" | "assert" | "async" | "await" | "break" | "class" | "continue" | "def"
            | "del" | "elif" | "else" | "except" | "False" | "finally" | "for" | "from"
            | "global" | "if" | "import" | "in" | "is" | "lambda" | "None" | "nonlocal"
            | "not" | "or" | "pass" | "raise" | "return" | "True" | "try" | "while" | "with"
            | "yield"
    )
}

fn is_py_type(s: &str) -> bool {
    matches!(
        s,
        "int" | "float" | "str" | "bool" | "list" | "dict" | "set" | "tuple" | "bytes" | "object"
    ) || (s.starts_with(|c: char| c.is_ascii_uppercase()) && !s.contains('_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_python_tokenization() {
        let py = PythonGrammar;
        let line = "def calculate(total: int) -> bool: # calculate";
        let (tokens, _) = py.tokenize_line(line, None);

        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Keyword && &line[t.start..t.end] == "def"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Function && &line[t.start..t.end] == "calculate"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Type && &line[t.start..t.end] == "int"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Type && &line[t.start..t.end] == "bool"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::CommentLine));
    }
}

