//! YAML language tokenizer and grammar implementation.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct YamlGrammar;

impl Grammar for YamlGrammar {
    fn language_id(&self) -> &'static str {
        "yaml"
    }

    fn numeric_id(&self) -> u8 {
        13
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["yaml", "yml"]
    }

    fn tokenize_line(
        &self,
        line: &str,
        _state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>) {
        let lang_id = self.numeric_id();
        let line = line.trim_end_matches('\r');
        if line.bytes().all(|b| b.is_ascii_whitespace()) {
            return (Vec::new(), None);
        }

        let mut tokens = Vec::new();
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        let mut expect_key = true;
        let mut flow_brace_depth: usize = 0;
        let mut flow_bracket_depth: usize = 0;

        while i < len {
            let b = bytes[i];

            if b.is_ascii_whitespace() {
                i += 1;
                continue;
            }

            // Comment: '#' at start of line or preceded by whitespace
            if b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
                tokens.push(TokenSpan::new(i, len, SyntaxScope::CommentLine, lang_id));
                return (tokens, None);
            }

            // Document markers --- or ...
            if (i == 0 || bytes[..i].iter().all(|c| c.is_ascii_whitespace()))
                && i + 3 <= len
                && (&bytes[i..i + 3] == b"---" || &bytes[i..i + 3] == b"...")
                && (i + 3 == len || bytes[i + 3].is_ascii_whitespace())
            {
                tokens.push(TokenSpan::new(i, i + 3, SyntaxScope::Keyword, lang_id));
                i += 3;
                expect_key = false;
                continue;
            }

            // List item marker '-'
            if b == b'-' && (i + 1 == len || bytes[i + 1].is_ascii_whitespace()) {
                tokens.push(TokenSpan::new(
                    i,
                    i + 1,
                    SyntaxScope::PunctuationDelimiter,
                    lang_id,
                ));
                i += 1;
                expect_key = true;
                continue;
            }

            // Flow brackets and delimiters
            if b == b'{' {
                tokens.push(TokenSpan::new(
                    i,
                    i + 1,
                    SyntaxScope::PunctuationBracket,
                    lang_id,
                ));
                i += 1;
                flow_brace_depth += 1;
                expect_key = true;
                continue;
            }
            if b == b'}' {
                tokens.push(TokenSpan::new(
                    i,
                    i + 1,
                    SyntaxScope::PunctuationBracket,
                    lang_id,
                ));
                i += 1;
                flow_brace_depth = flow_brace_depth.saturating_sub(1);
                expect_key = false;
                continue;
            }
            if b == b'[' {
                tokens.push(TokenSpan::new(
                    i,
                    i + 1,
                    SyntaxScope::PunctuationBracket,
                    lang_id,
                ));
                i += 1;
                flow_bracket_depth += 1;
                expect_key = false;
                continue;
            }
            if b == b']' {
                tokens.push(TokenSpan::new(
                    i,
                    i + 1,
                    SyntaxScope::PunctuationBracket,
                    lang_id,
                ));
                i += 1;
                flow_bracket_depth = flow_bracket_depth.saturating_sub(1);
                expect_key = false;
                continue;
            }
            if b == b',' && (flow_brace_depth > 0 || flow_bracket_depth > 0) {
                tokens.push(TokenSpan::new(
                    i,
                    i + 1,
                    SyntaxScope::PunctuationDelimiter,
                    lang_id,
                ));
                i += 1;
                expect_key = flow_brace_depth > flow_bracket_depth;
                continue;
            }

            // Quoted strings "..." or '...'
            if b == b'"' || b == b'\'' {
                let start = i;
                if b == b'"' {
                    scan_yaml_double_quoted(line, &mut i);
                } else {
                    scan_yaml_single_quoted(line, &mut i);
                }
                let mut peek = i;
                while peek < len && (bytes[peek] == b' ' || bytes[peek] == b'\t') {
                    peek += 1;
                }
                if expect_key
                    && peek < len
                    && bytes[peek] == b':'
                    && (peek + 1 == len
                        || bytes[peek + 1].is_ascii_whitespace()
                        || matches!(bytes[peek + 1], b',' | b'}' | b']'))
                {
                    tokens.push(TokenSpan::new(start, i, SyntaxScope::Property, lang_id));
                    tokens.push(TokenSpan::new(
                        peek,
                        peek + 1,
                        SyntaxScope::PunctuationDelimiter,
                        lang_id,
                    ));
                    i = peek + 1;
                    expect_key = false;
                } else {
                    tokens.push(TokenSpan::new(start, i, SyntaxScope::String, lang_id));
                    expect_key = false;
                }
                continue;
            }

            // Anchors (&name) and Aliases (*name)
            if (b == b'&' || b == b'*')
                && i + 1 < len
                && !bytes[i + 1].is_ascii_whitespace()
                && !matches!(bytes[i + 1], b',' | b'[' | b']' | b'{' | b'}')
            {
                let start = i;
                i += 1;
                while i < len
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b',' | b'[' | b']' | b'{' | b'}' | b':')
                {
                    let ch = line[i..].chars().next().unwrap();
                    i += ch.len_utf8();
                }
                tokens.push(TokenSpan::new(
                    start,
                    i,
                    SyntaxScope::VariableParameter,
                    lang_id,
                ));
                continue;
            }

            // Tags (!tag, !!str)
            if b == b'!' {
                let start = i;
                i += 1;
                while i < len
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b',' | b'[' | b']' | b'{' | b'}')
                {
                    let ch = line[i..].chars().next().unwrap();
                    i += ch.len_utf8();
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Type, lang_id));
                continue;
            }

            // Mapping key detection (unquoted key followed by ':')
            if expect_key {
                if let Some((key_end, colon_pos)) = find_yaml_mapping_colon(line, i) {
                    if key_end > i {
                        tokens.push(TokenSpan::new(i, key_end, SyntaxScope::Property, lang_id));
                    }
                    tokens.push(TokenSpan::new(
                        colon_pos,
                        colon_pos + 1,
                        SyntaxScope::PunctuationDelimiter,
                        lang_id,
                    ));
                    i = colon_pos + 1;
                    expect_key = false;
                    continue;
                }
                expect_key = false;
            }

            // Block scalar indicators: |, |-, |+, >, >-, >+
            if (b == b'|' || b == b'>')
                && (i + 1 == len
                    || bytes[i + 1].is_ascii_whitespace()
                    || ((bytes[i + 1] == b'-' || bytes[i + 1] == b'+')
                        && (i + 2 == len || bytes[i + 2].is_ascii_whitespace())))
            {
                let start = i;
                i += 1;
                if i < len && (bytes[i] == b'-' || bytes[i] == b'+') {
                    i += 1;
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Operator, lang_id));
                continue;
            }

            if b == b':' {
                tokens.push(TokenSpan::new(
                    i,
                    i + 1,
                    SyntaxScope::PunctuationDelimiter,
                    lang_id,
                ));
                i += 1;
                continue;
            }

            // Unquoted scalar word (booleans, null, numbers, unquoted string)
            let start = i;
            let in_flow = flow_brace_depth > 0 || flow_bracket_depth > 0;
            while i < len {
                let cur = bytes[i];
                if cur.is_ascii_whitespace() {
                    break;
                }
                if in_flow && matches!(cur, b',' | b'[' | b']' | b'{' | b'}') {
                    break;
                }
                let ch = line[i..].chars().next().unwrap();
                i += ch.len_utf8();
            }
            let word = &line[start..i];
            let scope = if is_yaml_constant(word) {
                SyntaxScope::ConstantLanguage
            } else if is_yaml_number(word) {
                SyntaxScope::Number
            } else {
                SyntaxScope::String
            };
            tokens.push(TokenSpan::new(start, i, scope, lang_id));
        }

        (tokens, None)
    }
}

fn scan_yaml_double_quoted(line: &str, i: &mut usize) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    *i += 1;
    while *i < len {
        if bytes[*i] == b'\\' {
            *i += 1;
            if *i < len {
                let esc = line[*i..].chars().next().unwrap();
                *i += esc.len_utf8();
            }
        } else if bytes[*i] == b'"' {
            *i += 1;
            break;
        } else {
            let ch = line[*i..].chars().next().unwrap();
            *i += ch.len_utf8();
        }
    }
}

fn scan_yaml_single_quoted(line: &str, i: &mut usize) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    *i += 1;
    while *i < len {
        if bytes[*i] == b'\'' {
            if *i + 1 < len && bytes[*i + 1] == b'\'' {
                *i += 2;
            } else {
                *i += 1;
                break;
            }
        } else {
            let ch = line[*i..].chars().next().unwrap();
            *i += ch.len_utf8();
        }
    }
}

fn find_yaml_mapping_colon(line: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut p = start;
    let mut trimmed_end = start;

    while p < len {
        let b = bytes[p];
        if b == b':'
            && (p + 1 == len
                || bytes[p + 1].is_ascii_whitespace()
                || matches!(bytes[p + 1], b',' | b'}' | b']'))
        {
            return Some((trimmed_end, p));
        }
        if b == b'#' && p > start && bytes[p - 1].is_ascii_whitespace() {
            return None;
        }
        if matches!(b, b'{' | b'}' | b'[' | b']' | b',') {
            return None;
        }
        let ch = line[p..].chars().next().unwrap();
        p += ch.len_utf8();
        if !b.is_ascii_whitespace() {
            trimmed_end = p;
        }
    }
    None
}

fn is_yaml_constant(s: &str) -> bool {
    matches!(
        s,
        "true" | "True" | "TRUE" | "false" | "False" | "FALSE" | "null" | "Null" | "NULL" | "~"
    )
}

fn is_yaml_number(s: &str) -> bool {
    if matches!(
        s,
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" | "-.inf" | "-.Inf" | "-.INF"
            | ".nan" | ".NaN" | ".NAN"
    ) {
        return true;
    }
    if let Some(hex) = s.strip_prefix("0x") {
        return !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if let Some(oct) = s.strip_prefix("0o") {
        return !oct.is_empty() && oct.bytes().all(|b| matches!(b, b'0'..=b'7'));
    }
    let rest = s.strip_prefix('+').or_else(|| s.strip_prefix('-')).unwrap_or(s);
    if rest.is_empty() {
        return false;
    }
    let bytes = rest.as_bytes();
    if !bytes[0].is_ascii_digit() {
        return false;
    }
    let mut saw_dot = false;
    let mut saw_exp = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            i += 1;
        } else if b == b'.' && !saw_dot && !saw_exp {
            saw_dot = true;
            i += 1;
        } else if (b == b'e' || b == b'E') && !saw_exp {
            saw_exp = true;
            i += 1;
            if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                i += 1;
            }
            if i == bytes.len() {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yaml_keys_values_lists_anchors_comments() {
        let g = YamlGrammar;
        let line = "image: \"alpine:3.19\" # base";
        let (tokens, state) = g.tokenize_line(line, None);
        assert!(state.is_none());
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Property && &line[t.start..t.end] == "image"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::String && &line[t.start..t.end] == "\"alpine:3.19\""));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::CommentLine));

        let (list_tokens, _) = g.tokenize_line("  - replicas: 3", None);
        assert!(list_tokens.iter().any(|t| t.scope == SyntaxScope::Property));
        assert!(list_tokens.iter().any(|t| t.scope == SyntaxScope::Number));

        let (const_tokens, _) = g.tokenize_line("enabled: true", None);
        assert!(const_tokens.iter().any(|t| t.scope == SyntaxScope::ConstantLanguage));

        let (doc_tokens, _) = g.tokenize_line("---", None);
        assert!(doc_tokens.iter().any(|t| t.scope == SyntaxScope::Keyword));
    }

    #[test]
    fn test_yaml_whitespace_crlf_utf8() {
        let g = YamlGrammar;
        assert!(g.tokenize_line("", None).0.is_empty());
        assert!(g.tokenize_line("  \t\r", None).0.is_empty());

        let line = "greeting: \"🚀 café 日本語\" # 🚀\r";
        let (tokens, _) = g.tokenize_line(line, None);
        for t in &tokens {
            assert!(t.start < t.end && t.end <= line.len());
            assert!(line.is_char_boundary(t.start) && line.is_char_boundary(t.end));
            assert!(!line[t.start..t.end].ends_with('\r'));
        }
    }
}
