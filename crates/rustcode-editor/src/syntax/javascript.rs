//! JavaScript & TypeScript language tokenizers.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct JavaScriptGrammar;

impl Grammar for JavaScriptGrammar {
    fn language_id(&self) -> &'static str {
        "javascript"
    }

    fn numeric_id(&self) -> u8 {
        2
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["js", "jsx", "mjs", "cjs"]
    }

    fn tokenize_line(
        &self,
        line: &str,
        state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>) {
        tokenize_js_ts(line, state, self.numeric_id(), false)
    }
}

pub struct TypeScriptGrammar;

impl Grammar for TypeScriptGrammar {
    fn language_id(&self) -> &'static str {
        "typescript"
    }

    fn numeric_id(&self) -> u8 {
        3
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["ts", "tsx", "mts", "cts"]
    }

    fn tokenize_line(
        &self,
        line: &str,
        state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>) {
        tokenize_js_ts(line, state, self.numeric_id(), true)
    }
}

fn tokenize_js_ts(
    line: &str,
    state: Option<&GrammarState>,
    lang_id: u8,
    is_ts: bool,
) -> (Vec<TokenSpan>, Option<GrammarState>) {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut current_state = state.cloned();

    // 1. Resume multi-line template literal
    if let Some(GrammarState::TemplateLiteral) = current_state {
        let start = i;
        let mut closed = false;
        while i < len {
            if bytes[i] == b'\\' {
                i += 2;
            } else if bytes[i] == b'`' {
                i += 1;
                closed = true;
                break;
            } else {
                i += 1;
            }
        }
        tokens.push(TokenSpan::new(start, i, SyntaxScope::StringTemplate, lang_id));
        if !closed {
            return (tokens, Some(GrammarState::TemplateLiteral));
        }
        current_state = None;
    }

    // 2. Resume multi-line block comment
    if let Some(GrammarState::BlockComment { mut depth }) = current_state {
        let start = i;
        while i + 1 < len {
            if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                depth = depth.saturating_sub(1);
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
                while i + 1 < len {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    tokens.push(TokenSpan::new(start, len, SyntaxScope::CommentBlock, lang_id));
                    return (tokens, Some(GrammarState::BlockComment { depth: 1 }));
                } else {
                    tokens.push(TokenSpan::new(start, i, SyntaxScope::CommentBlock, lang_id));
                    continue;
                }
            }
        }

        // Template literal `...`
        if b == b'`' {
            let start = i;
            i += 1;
            let mut closed = false;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == b'`' {
                    i += 1;
                    closed = true;
                    break;
                } else {
                    i += 1;
                }
            }
            tokens.push(TokenSpan::new(start, i, SyntaxScope::StringTemplate, lang_id));
            if !closed {
                return (tokens, Some(GrammarState::TemplateLiteral));
            }
            continue;
        }

        // Single or Double quoted strings
        if b == b'"' || b == b'\'' {
            let quote = b;
            let start = i;
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

        // Numbers
        if b.is_ascii_digit() {
            let start = i;
            if b == b'0' && i + 1 < len && (bytes[i + 1] == b'x' || bytes[i + 1] == b'b' || bytes[i + 1] == b'o') {
                i += 2;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
            } else {
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'_' || bytes[i] == b'n') {
                    if bytes[i] == b'.' && i + 1 < len && !bytes[i + 1].is_ascii_digit() {
                        break;
                    }
                    i += 1;
                }
            }
            tokens.push(TokenSpan::new(start, i, SyntaxScope::Number, lang_id));
            continue;
        }

        // Identifiers, keywords, functions
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$') {
                i += 1;
            }
            let word = &line[start..i];

            let mut is_fn = false;
            let mut peek = i;
            while peek < len && bytes[peek].is_ascii_whitespace() {
                peek += 1;
            }
            if peek < len && bytes[peek] == b'(' && !is_js_keyword(word) {
                is_fn = true;
            }

            let scope = if is_fn {
                SyntaxScope::Function
            } else if is_js_keyword(word) {
                match word {
                    "true" | "false" | "null" | "undefined" => SyntaxScope::ConstantLanguage,
                    "function" | "const" | "let" | "var" | "class" | "extends" | "new" => {
                        SyntaxScope::Keyword
                    }
                    "if" | "else" | "return" | "for" | "while" | "switch" | "case" | "break"
                    | "continue" | "try" | "catch" | "finally" | "async" | "await" | "import"
                    | "export" | "from" => SyntaxScope::KeywordControl,
                    _ => SyntaxScope::Keyword,
                }
            } else if is_ts && is_ts_keyword(word) {
                match word {
                    "type" | "interface" | "enum" => SyntaxScope::Keyword,
                    "string" | "number" | "boolean" | "any" | "void" | "unknown" | "never" => {
                        SyntaxScope::Type
                    }
                    _ => SyntaxScope::KeywordOther,
                }
            } else if word.starts_with(|c: char| c.is_ascii_uppercase()) {
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
            if matches!(op2, "=>" | "==" | "!=" | "<=" | ">=" | "&&" | "||" | "++" | "--" | "?." | "??" | "+=" | "-=" | "*=" | "/=") {
                tokens.push(TokenSpan::new(start, start + 2, SyntaxScope::Operator, lang_id));
                i += 2;
                continue;
            }
        }

        let scope = match b {
            b'(' | b')' | b'[' | b']' | b'{' => SyntaxScope::PunctuationBracket,
            b';' | b',' | b':' | b'.' => SyntaxScope::PunctuationDelimiter,
            b'+' | b'-' | b'*' | b'/' | b'%' | b'=' | b'<' | b'>' | b'!' | b'&' | b'|' | b'^' | b'?' => {
                SyntaxScope::Operator
            }
            _ => SyntaxScope::Plain,
        };
        tokens.push(TokenSpan::new(start, start + 1, scope, lang_id));
        i += 1;
    }

    (tokens, None)
}

fn is_js_keyword(s: &str) -> bool {
    matches!(
        s,
        "async" | "await" | "break" | "case" | "catch" | "class" | "const" | "continue"
            | "debugger" | "default" | "delete" | "do" | "else" | "export" | "extends"
            | "false" | "finally" | "for" | "from" | "function" | "get" | "if" | "import"
            | "in" | "instanceof" | "let" | "new" | "null" | "of" | "return" | "set"
            | "static" | "super" | "switch" | "this" | "throw" | "true" | "try" | "typeof"
            | "undefined" | "var" | "void" | "while" | "with" | "yield"
    )
}

fn is_ts_keyword(s: &str) -> bool {
    matches!(
        s,
        "type" | "interface" | "enum" | "declare" | "namespace" | "as" | "is" | "keyof"
            | "readonly" | "private" | "public" | "protected" | "abstract" | "implements"
            | "override" | "string" | "number" | "boolean" | "symbol" | "bigint" | "any"
            | "unknown" | "never" | "void"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_js_ts_tokenization() {
        let js = JavaScriptGrammar;
        let line = "const greeting = `Hello ${name}`; // comment";
        let (tokens, state) = js.tokenize_line(line, None);
        assert!(state.is_none());

        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Keyword && &line[t.start..t.end] == "const"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::StringTemplate));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::CommentLine));

        let ts = TypeScriptGrammar;
        let ts_line = "interface User { id: number; name: string; }";
        let (ts_tokens, _) = ts.tokenize_line(ts_line, None);
        assert!(ts_tokens.iter().any(|t| t.scope == SyntaxScope::Keyword && &ts_line[t.start..t.end] == "interface"));
        assert!(ts_tokens.iter().any(|t| t.scope == SyntaxScope::Type && &ts_line[t.start..t.end] == "number"));
        assert!(ts_tokens.iter().any(|t| t.scope == SyntaxScope::Type && &ts_line[t.start..t.end] == "string"));
    }
}

