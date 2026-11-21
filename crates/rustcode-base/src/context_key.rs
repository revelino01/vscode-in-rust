//! VS Code `when` clause context-key expression parser and evaluator.
//!
//! Evaluates expressions such as:
//! - `editorTextFocus && !editorReadonly`
//! - `resourceLangId == rust`
//! - `editorHasSelection || hasCodeActions`
//! - `resourceExt =~ /\\.rs|\\.toml/`
//! - `view in allowedViews`

use parking_lot::RwLock;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum ContextKeyExpr {
    True,
    False,
    Defined(String),
    Not(Box<ContextKeyExpr>),
    Equals(String, String),
    NotEquals(String, String),
    Regex(String, String),
    In(String, String),
    And(Vec<ContextKeyExpr>),
    Or(Vec<ContextKeyExpr>),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ContextKeyError {
    #[error("Empty expression")]
    Empty,
    #[error("Invalid syntax: {0}")]
    InvalidSyntax(String),
}

const MAX_EXPR_DEPTH: usize = 64;

impl ContextKeyExpr {
    /// Parse a VS Code `when` clause string into an expression tree.
    pub fn parse(expr: &str) -> Result<Self, ContextKeyError> {
        let trimmed = expr.trim();
        if trimmed.is_empty() {
            return Err(ContextKeyError::Empty);
        }
        let tokens = tokenize(trimmed)?;
        let mut parser = Parser {
            tokens: &tokens,
            pos: 0,
        };
        let ast = parser.parse_or(0)?;
        if parser.pos != parser.tokens.len() {
            return Err(ContextKeyError::InvalidSyntax(
                "Unexpected trailing tokens".to_string(),
            ));
        }
        Ok(ast)
    }

    /// Evaluate the expression against a context-key service or context provider.
    pub fn evaluate(&self, context: &ContextKeyService) -> bool {
        match self {
            ContextKeyExpr::True => true,
            ContextKeyExpr::False => false,
            ContextKeyExpr::Defined(key) => {
                match context.get(key) {
                    Some(Value::Bool(b)) => b,
                    Some(Value::Null) | None => false,
                    Some(Value::String(s)) => !s.is_empty(),
                    Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
                    Some(Value::Array(a)) => !a.is_empty(),
                    Some(Value::Object(o)) => !o.is_empty(),
                }
            }
            ContextKeyExpr::Not(inner) => !inner.evaluate(context),
            ContextKeyExpr::Equals(key, expected) => {
                match context.get(key) {
                    Some(Value::String(s)) => s == *expected,
                    Some(Value::Bool(b)) => b.to_string() == *expected,
                    Some(Value::Number(n)) => n.to_string() == *expected,
                    Some(Value::Null) | None => expected == "null" || expected.is_empty(),
                    _ => false,
                }
            }
            ContextKeyExpr::NotEquals(key, expected) => {
                !ContextKeyExpr::Equals(key.clone(), expected.clone()).evaluate(context)
            }
            ContextKeyExpr::Regex(key, pattern) => {
                if let Some(val_str) = context.get_string(key) {
                    if let Ok(re) = Regex::new(pattern) {
                        return re.is_match(&val_str);
                    }
                }
                false
            }
            ContextKeyExpr::In(key, set_key) => {
                if let (Some(val), Some(target_set)) = (context.get(key), context.get(set_key)) {
                    if let Some(arr) = target_set.as_array() {
                        return arr.contains(&val);
                    }
                }
                false
            }
            ContextKeyExpr::And(items) => items.iter().all(|e| e.evaluate(context)),
            ContextKeyExpr::Or(items) => items.iter().any(|e| e.evaluate(context)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    LParen,
    RParen,
    And,
    Or,
    Not,
    Eq,
    NotEq,
    RegexMatch,
    Quoted(String),
    RegexLit(String),
    Word(String),
}

fn is_word_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '(' | ')' | '&' | '|' | '!' | '=' | '\'' | '"' | '\\')
}

fn tokenize(expr: &str) -> Result<Vec<Token>, ContextKeyError> {
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let mut expect_regex = false;

    while i < expr.len() {
        let ch = expr[i..].chars().next().ok_or_else(|| {
            ContextKeyError::InvalidSyntax("Unexpected end of expression".to_string())
        })?;
        if ch.is_whitespace() {
            i += ch.len_utf8();
            continue;
        }

        if expect_regex {
            expect_regex = false;
            if ch == '/' {
                let mut pattern = String::new();
                let mut in_char_class = false;
                let mut closed = false;
                let mut j = i + 1;
                while j < expr.len() {
                    let rc = expr[j..].chars().next().ok_or_else(|| {
                        ContextKeyError::InvalidSyntax("Unclosed regex literal".to_string())
                    })?;
                    if rc == '\\' {
                        j += 1;
                        if j >= expr.len() {
                            return Err(ContextKeyError::InvalidSyntax(
                                "Unclosed regex literal".to_string(),
                            ));
                        }
                        let esc = expr[j..].chars().next().ok_or_else(|| {
                            ContextKeyError::InvalidSyntax("Unclosed regex literal".to_string())
                        })?;
                        j += esc.len_utf8();
                        if esc == '/' {
                            pattern.push('/');
                        } else {
                            pattern.push('\\');
                            pattern.push(esc);
                        }
                        continue;
                    }
                    if rc == '[' && !in_char_class {
                        in_char_class = true;
                        pattern.push(rc);
                        j += 1;
                        continue;
                    }
                    if rc == ']' && in_char_class {
                        in_char_class = false;
                        pattern.push(rc);
                        j += 1;
                        continue;
                    }
                    if rc == '/' && !in_char_class {
                        closed = true;
                        j += 1;
                        break;
                    }
                    pattern.push(rc);
                    j += rc.len_utf8();
                }
                if !closed {
                    return Err(ContextKeyError::InvalidSyntax(
                        "Unclosed regex literal".to_string(),
                    ));
                }

                let mut flag_i = false;
                let mut flag_m = false;
                let mut flag_s = false;
                while j < expr.len() {
                    let Some(fc) = expr[j..].chars().next() else {
                        break;
                    };
                    if fc.is_ascii_alphabetic() {
                        match fc {
                            'i' => flag_i = true,
                            'm' => flag_m = true,
                            's' => flag_s = true,
                            'g' | 'u' | 'y' | 'd' | 'v' => {}
                            _ => {
                                return Err(ContextKeyError::InvalidSyntax(format!(
                                    "Invalid regex flag '{}'",
                                    fc
                                )));
                            }
                        }
                        j += 1;
                    } else {
                        break;
                    }
                }
                if flag_i || flag_m || flag_s {
                    let mut prefix = String::from("(?");
                    if flag_i {
                        prefix.push('i');
                    }
                    if flag_m {
                        prefix.push('m');
                    }
                    if flag_s {
                        prefix.push('s');
                    }
                    prefix.push(')');
                    pattern = format!("{}{}", prefix, pattern);
                }

                tokens.push(Token::RegexLit(pattern));
                i = j;
                continue;
            }
        }

        match ch {
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '&' => {
                if expr[i..].starts_with("&&") {
                    tokens.push(Token::And);
                    i += 2;
                } else {
                    return Err(ContextKeyError::InvalidSyntax(
                        "Unexpected '&'".to_string(),
                    ));
                }
            }
            '|' => {
                if expr[i..].starts_with("||") {
                    tokens.push(Token::Or);
                    i += 2;
                } else {
                    return Err(ContextKeyError::InvalidSyntax(
                        "Unexpected '|'".to_string(),
                    ));
                }
            }
            '!' => {
                if expr[i..].starts_with("!=") {
                    tokens.push(Token::NotEq);
                    i += 2;
                } else {
                    tokens.push(Token::Not);
                    i += 1;
                }
            }
            '=' => {
                if expr[i..].starts_with("==") {
                    tokens.push(Token::Eq);
                    i += 2;
                } else if expr[i..].starts_with("=~") {
                    tokens.push(Token::RegexMatch);
                    expect_regex = true;
                    i += 2;
                } else {
                    return Err(ContextKeyError::InvalidSyntax(
                        "Unexpected '='".to_string(),
                    ));
                }
            }
            '\'' | '"' => {
                let quote = ch;
                let mut val = String::new();
                let mut j = i + 1;
                let mut closed = false;
                while j < expr.len() {
                    let sc = expr[j..].chars().next().ok_or_else(|| {
                        ContextKeyError::InvalidSyntax("Unclosed string literal".to_string())
                    })?;
                    if sc == '\\' {
                        j += 1;
                        if j >= expr.len() {
                            return Err(ContextKeyError::InvalidSyntax(
                                "Unclosed string literal".to_string(),
                            ));
                        }
                        let esc = expr[j..].chars().next().ok_or_else(|| {
                            ContextKeyError::InvalidSyntax("Unclosed string literal".to_string())
                        })?;
                        j += esc.len_utf8();
                        if esc == '\\' || esc == quote {
                            val.push(esc);
                        } else {
                            val.push('\\');
                            val.push(esc);
                        }
                        continue;
                    }
                    if sc == quote {
                        closed = true;
                        j += 1;
                        break;
                    }
                    val.push(sc);
                    j += sc.len_utf8();
                }
                if !closed {
                    return Err(ContextKeyError::InvalidSyntax(
                        "Unclosed string literal".to_string(),
                    ));
                }
                tokens.push(Token::Quoted(val));
                i = j;
            }
            '\\' => {
                return Err(ContextKeyError::InvalidSyntax(
                    "Unexpected escape character '\\'".to_string(),
                ));
            }
            _ => {
                let start = i;
                while i < expr.len() {
                    let Some(wc) = expr[i..].chars().next() else {
                        break;
                    };
                    if is_word_char(wc) {
                        i += wc.len_utf8();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Word(expr[start..i].to_string()));
            }
        }
    }

    Ok(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&Token> {
        let tok = self.tokens.get(self.pos);
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn parse_or(&mut self, depth: usize) -> Result<ContextKeyExpr, ContextKeyError> {
        let mut items = vec![self.parse_and(depth)?];
        while matches!(self.peek(), Some(Token::Or)) {
            self.advance();
            items.push(self.parse_and(depth)?);
        }
        if items.len() == 1 {
            Ok(items.pop().unwrap())
        } else {
            Ok(ContextKeyExpr::Or(items))
        }
    }

    fn parse_and(&mut self, depth: usize) -> Result<ContextKeyExpr, ContextKeyError> {
        let mut items = vec![self.parse_not(depth)?];
        while matches!(self.peek(), Some(Token::And)) {
            self.advance();
            items.push(self.parse_not(depth)?);
        }
        if items.len() == 1 {
            Ok(items.pop().unwrap())
        } else {
            Ok(ContextKeyExpr::And(items))
        }
    }

    fn parse_not(&mut self, depth: usize) -> Result<ContextKeyExpr, ContextKeyError> {
        let mut not_count = 0usize;
        while matches!(self.peek(), Some(Token::Not)) {
            self.advance();
            not_count += 1;
            if depth.saturating_add(not_count) > MAX_EXPR_DEPTH {
                return Err(ContextKeyError::InvalidSyntax(
                    "Maximum expression nesting depth exceeded".to_string(),
                ));
            }
        }
        let mut expr = self.parse_primary(depth.saturating_add(not_count))?;
        for _ in 0..not_count {
            expr = ContextKeyExpr::Not(Box::new(expr));
        }
        Ok(expr)
    }

    fn parse_primary(&mut self, depth: usize) -> Result<ContextKeyExpr, ContextKeyError> {
        if depth > MAX_EXPR_DEPTH {
            return Err(ContextKeyError::InvalidSyntax(
                "Maximum expression nesting depth exceeded".to_string(),
            ));
        }

        match self.peek().cloned() {
            Some(Token::LParen) => {
                self.advance();
                if matches!(self.peek(), Some(Token::RParen)) {
                    return Err(ContextKeyError::InvalidSyntax(
                        "Empty parenthesized expression".to_string(),
                    ));
                }
                let inner = self.parse_or(depth + 1)?;
                if !matches!(self.peek(), Some(Token::RParen)) {
                    return Err(ContextKeyError::InvalidSyntax(
                        "Unbalanced parentheses".to_string(),
                    ));
                }
                self.advance();
                Ok(inner)
            }
            Some(Token::Word(lhs)) => {
                self.advance();
                match self.peek().cloned() {
                    Some(Token::Eq) => {
                        self.advance();
                        match self.advance().cloned() {
                            Some(Token::Word(v)) | Some(Token::Quoted(v)) => {
                                Ok(ContextKeyExpr::Equals(lhs, v))
                            }
                            _ => Err(ContextKeyError::InvalidSyntax(
                                "Invalid equality expression".to_string(),
                            )),
                        }
                    }
                    Some(Token::NotEq) => {
                        self.advance();
                        match self.advance().cloned() {
                            Some(Token::Word(v)) | Some(Token::Quoted(v)) => {
                                Ok(ContextKeyExpr::NotEquals(lhs, v))
                            }
                            _ => Err(ContextKeyError::InvalidSyntax(
                                "Invalid inequality expression".to_string(),
                            )),
                        }
                    }
                    Some(Token::RegexMatch) => {
                        self.advance();
                        match self.advance().cloned() {
                            Some(Token::RegexLit(pat)) => Ok(ContextKeyExpr::Regex(lhs, pat)),
                            _ => Err(ContextKeyError::InvalidSyntax(
                                "Expected regex literal /.../ after '=~'".to_string(),
                            )),
                        }
                    }
                    Some(Token::Word(op)) if op == "in" => {
                        self.advance();
                        match self.advance().cloned() {
                            Some(Token::Word(set_key)) => Ok(ContextKeyExpr::In(lhs, set_key)),
                            _ => Err(ContextKeyError::InvalidSyntax(
                                "Invalid 'in' expression".to_string(),
                            )),
                        }
                    }
                    _ => match lhs.as_str() {
                        "true" => Ok(ContextKeyExpr::True),
                        "false" => Ok(ContextKeyExpr::False),
                        _ => Ok(ContextKeyExpr::Defined(lhs)),
                    },
                }
            }
            _ => Err(ContextKeyError::InvalidSyntax(
                "Unexpected token in expression".to_string(),
            )),
        }
    }
}

/// Central store for context keys, backing `when` clause resolution.
#[derive(Clone, Default)]
pub struct ContextKeyService {
    keys: Arc<RwLock<HashMap<String, Value>>>,
}

impl ContextKeyService {
    pub fn new() -> Self {
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn set<V: Into<Value>>(&self, key: &str, value: V) {
        self.keys.write().insert(key.to_string(), value.into());
    }

    pub fn set_bool(&self, key: &str, value: bool) {
        self.keys.write().insert(key.to_string(), Value::Bool(value));
    }

    pub fn set_string(&self, key: &str, value: &str) {
        self.keys.write().insert(key.to_string(), Value::String(value.to_string()));
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.keys.read().get(key).cloned()
    }

    pub fn get_string(&self, key: &str) -> Option<String> {
        match self.keys.read().get(key) {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn remove(&self, key: &str) {
        self.keys.write().remove(key);
    }

    pub fn evaluate_expr(&self, expr_str: &str) -> bool {
        match ContextKeyExpr::parse(expr_str) {
            Ok(expr) => expr.evaluate(self),
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_key_boolean_logic() {
        let ctx = ContextKeyService::new();
        ctx.set_bool("editorTextFocus", true);
        ctx.set_bool("editorReadonly", false);

        assert!(ctx.evaluate_expr("editorTextFocus"));
        assert!(!ctx.evaluate_expr("!editorTextFocus"));
        assert!(ctx.evaluate_expr("!editorReadonly"));
        assert!(ctx.evaluate_expr("editorTextFocus && !editorReadonly"));
        assert!(!ctx.evaluate_expr("editorTextFocus && editorReadonly"));
        assert!(ctx.evaluate_expr("editorReadonly || editorTextFocus"));
    }

    #[test]
    fn test_context_key_equality() {
        let ctx = ContextKeyService::new();
        ctx.set_string("resourceLangId", "rust");

        assert!(ctx.evaluate_expr("resourceLangId == rust"));
        assert!(ctx.evaluate_expr("resourceLangId == 'rust'"));
        assert!(ctx.evaluate_expr("resourceLangId != python"));
        assert!(!ctx.evaluate_expr("resourceLangId == typescript"));
    }

    #[test]
    fn test_context_key_regex() {
        let ctx = ContextKeyService::new();
        ctx.set_string("resourceExt", ".rs");

        assert!(ctx.evaluate_expr("resourceExt =~ /\\.rs$/"));
        assert!(!ctx.evaluate_expr("resourceExt =~ /\\.py$/"));
    }

    #[test]
    fn test_context_key_in() {
        let ctx = ContextKeyService::new();
        ctx.set("supportedExts", serde_json::json!([".rs", ".toml", ".md"]));
        ctx.set_string("currentExt", ".rs");
        ctx.set_string("unknownExt", ".cpp");

        assert!(ctx.evaluate_expr("currentExt in supportedExts"));
        assert!(!ctx.evaluate_expr("unknownExt in supportedExts"));
    }

    #[test]
    fn test_context_key_parentheses_and_precedence() {
        let ctx = ContextKeyService::new();
        ctx.set_bool("a", true);
        ctx.set_bool("b", false);
        ctx.set_bool("c", true);

        // a && (b || c) -> true && (false || true) -> true
        assert!(ctx.evaluate_expr("a && (b || c)"));
        // (a && b) || c -> (true && false) || true -> true
        assert!(ctx.evaluate_expr("(a && b) || c"));
        // (a && b) -> false
        assert!(!ctx.evaluate_expr("(a && b)"));
        // !(a && b) -> true
        assert!(ctx.evaluate_expr("!(a && b)"));
        // ! ( b ) -> true
        assert!(ctx.evaluate_expr("!(b)"));
    }

    #[test]
    fn test_context_key_rejects_deep_nesting_and_malformed_syntax() {
        let ctx = ContextKeyService::new();
        ctx.set_bool("a", true);
        ctx.set_bool("b", true);

        // Deeply nested groups (500 levels) must fail safely without stack overflow
        let deep_and = format!("{}a{}", "(a && ".repeat(500), ")".repeat(500));
        assert!(matches!(
            ContextKeyExpr::parse(&deep_and),
            Err(ContextKeyError::InvalidSyntax(_))
        ));
        assert!(!ctx.evaluate_expr(&deep_and));

        let deep_parens = format!("{}a{}", "(".repeat(500), ")".repeat(500));
        assert!(matches!(
            ContextKeyExpr::parse(&deep_parens),
            Err(ContextKeyError::InvalidSyntax(_))
        ));
        assert!(!ctx.evaluate_expr(&deep_parens));

        // Empty / malformed subexpressions and adjacent/dangling operators fail closed
        assert_eq!(ContextKeyExpr::parse(""), Err(ContextKeyError::Empty));
        assert_eq!(ContextKeyExpr::parse("   "), Err(ContextKeyError::Empty));
        for malformed in [
            "()",
            "(   )",
            "((()))",
            "() && a",
            "nonexistent || ()",
            "a &&|| b",
            "a ||&& b",
            "&&&&",
            "||||",
            "a &&",
            "&& a",
            "a ||",
            "|| a",
            "a && !",
            "!()",
            "(a && b",
            "a && b)",
            "a == 'unclosed",
        ] {
            assert!(
                ContextKeyExpr::parse(malformed).is_err(),
                "expected parse error for {:?}",
                malformed
            );
            assert!(
                !ctx.evaluate_expr(malformed),
                "expected false evaluation for {:?}",
                malformed
            );
        }

        // Quoted strings containing parens or operators remain valid
        ctx.set_string("sym", "()");
        ctx.set_string("op", "x && y");
        assert!(ctx.evaluate_expr("sym == '()'"));
        assert!(ctx.evaluate_expr("op == 'x && y'"));
    }

    #[test]
    fn test_context_key_lexer_slash_quotes_regex_and_depth_edge_cases() {
        let ctx = ContextKeyService::new();
        ctx.set_bool("a", true);
        ctx.set_bool("b", false);
        ctx.set_bool("d", false);
        ctx.set_string("slash", "b/c");
        ctx.set_string("op_str", "&& ||");
        ctx.set_string("op_str2", "a &&|| b");
        ctx.set_string("path", "src/foo/bar.rs");

        // Unquoted '/' in operand must NOT disable paren/quote validation
        assert!(matches!(
            ContextKeyExpr::parse("slash == b/c || (a == 'unclosed"),
            Err(ContextKeyError::InvalidSyntax(_))
        ));
        assert!(!ctx.evaluate_expr("slash == b/c || (a == 'unclosed"));
        assert!(ctx.evaluate_expr("slash == b/c && (a || !b)"));

        // Quoted operator strings remain valid
        assert!(ctx.evaluate_expr("op_str == '&& ||'"));
        assert!(ctx.evaluate_expr("op_str2 == 'a &&|| b'"));

        // Regex literals after =~ encapsulate ||, &&, (, ), ==, \/, and flags
        assert!(ctx.evaluate_expr("path =~ /foo||bar/ && a"));
        assert!(ctx.evaluate_expr("path =~ /a(b)c/ || a"));
        assert!(ctx.evaluate_expr("path =~ /foo\\/bar/"));
        assert!(ctx.evaluate_expr("path =~ /FOO/i"));
        assert!(ctx.evaluate_expr("a != 'x == y'"));
        ctx.set_string("eqval", "x==y");
        assert!(ctx.evaluate_expr("eqval =~ /x==y/"));

        // Invalid regex literals, dangling/single operators, bad 'in', trailing tokens
        for bad in [
            "a =~ /unclosed",
            "a =~ foo",
            "a =~ 'foo'",
            "!=b",
            "a = b",
            "a & b",
            "a | b",
            "a &&& b",
            "a ||| b",
            "a in ",
            " in b",
            "a ! b",
            "a == 'b' c",
        ] {
            assert!(
                ContextKeyExpr::parse(bad).is_err(),
                "expected parse error for {:?}",
                bad
            );
            assert!(!ctx.evaluate_expr(bad), "expected false for {:?}", bad);
        }

        // 5000 parens inside quotes succeed, while 5000 unquoted '(' or '!' fail safely
        let quoted_5000 = format!("q == '{}{}'", "(".repeat(5000), ")".repeat(5000));
        ctx.set_string("q", &format!("{}{}", "(".repeat(5000), ")".repeat(5000)));
        assert!(ctx.evaluate_expr(&quoted_5000));

        let unquoted_5000_parens = format!("{}a{}", "(".repeat(5000), ")".repeat(5000));
        assert!(matches!(
            ContextKeyExpr::parse(&unquoted_5000_parens),
            Err(ContextKeyError::InvalidSyntax(_))
        ));

        let unquoted_5000_nots = format!("{}a", "!".repeat(5000));
        assert!(matches!(
            ContextKeyExpr::parse(&unquoted_5000_nots),
            Err(ContextKeyError::InvalidSyntax(_))
        ));
    }
}
