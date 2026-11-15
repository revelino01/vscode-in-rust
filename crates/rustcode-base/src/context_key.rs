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

impl ContextKeyExpr {
    /// Parse a VS Code `when` clause string into an expression tree.
    pub fn parse(expr: &str) -> Result<Self, ContextKeyError> {
        let trimmed = unwrap_parens(expr.trim());
        if trimmed.is_empty() {
            return Ok(ContextKeyExpr::True);
        }

        // Handle OR expressions (lowest precedence)
        // Split by || if not inside parens/regex
        let or_parts = split_top_level(trimmed, "||");
        if or_parts.len() > 1 {
            let parsed_parts: Result<Vec<_>, _> = or_parts.into_iter().map(Self::parse).collect();
            return Ok(ContextKeyExpr::Or(parsed_parts?));
        }

        // Handle AND expressions (second lowest precedence)
        let and_parts = split_top_level(trimmed, "&&");
        if and_parts.len() > 1 {
            let parsed_parts: Result<Vec<_>, _> = and_parts.into_iter().map(Self::parse).collect();
            return Ok(ContextKeyExpr::And(parsed_parts?));
        }

        // Handle negation: !expr
        if let Some(rest) = trimmed.strip_prefix('!') {
            let inner = Self::parse(rest)?;
            return Ok(ContextKeyExpr::Not(Box::new(inner)));
        }

        // Single primary expression
        Self::parse_primary(trimmed)
    }

    fn parse_primary(expr: &str) -> Result<Self, ContextKeyError> {
        let trimmed = unwrap_parens(expr.trim());

        if trimmed == "true" {
            return Ok(ContextKeyExpr::True);
        }
        if trimmed == "false" {
            return Ok(ContextKeyExpr::False);
        }

        // Equality: key == val
        if let Some((left, right)) = split_binary(trimmed, "==") {
            let val = unquote(right.trim());
            return Ok(ContextKeyExpr::Equals(left.trim().to_string(), val));
        }

        // Inequality: key != val
        if let Some((left, right)) = split_binary(trimmed, "!=") {
            let val = unquote(right.trim());
            return Ok(ContextKeyExpr::NotEquals(left.trim().to_string(), val));
        }

        // Regex match: key =~ /pattern/
        if let Some((left, right)) = split_binary(trimmed, "=~") {
            let pattern = unwrap_regex(right.trim());
            return Ok(ContextKeyExpr::Regex(left.trim().to_string(), pattern));
        }

        // In operator: key in collection
        if let Some((left, right)) = split_binary(trimmed, " in ") {
            return Ok(ContextKeyExpr::In(left.trim().to_string(), right.trim().to_string()));
        }

        // Default: Key is defined and truthy
        Ok(ContextKeyExpr::Defined(trimmed.to_string()))
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
                    Some(Value::Number(n)) => n.as_f64().map_or(false, |v| v != 0.0),
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

fn unwrap_parens(mut s: &str) -> &str {
    loop {
        s = s.trim();
        if s.starts_with('(') && s.ends_with(')') && s.len() >= 2 {
            let bytes = s.as_bytes();
            let mut depth = 0;
            let mut all_enclosed = true;
            for (i, &b) in bytes.iter().enumerate() {
                if b == b'(' {
                    depth += 1;
                } else if b == b')' {
                    depth -= 1;
                    if depth == 0 && i < bytes.len() - 1 {
                        all_enclosed = false;
                        break;
                    }
                }
            }
            if all_enclosed && depth == 0 {
                s = &s[1..s.len() - 1];
                continue;
            }
        }
        break;
    }
    s.trim()
}

fn split_top_level<'a>(s: &'a str, delim: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let bytes = s.as_bytes();
    let delim_bytes = delim.as_bytes();
    let delim_len = delim_bytes.len();

    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            depth += 1;
        } else if bytes[i] == b')' && depth > 0 {
            depth -= 1;
        } else if depth == 0 && i + delim_len <= bytes.len() && &bytes[i..i + delim_len] == delim_bytes {
            parts.push(&s[start..i]);
            i += delim_len;
            start = i;
            continue;
        }
        i += 1;
    }

    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

fn split_binary<'a>(s: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    s.find(op).map(|idx| (&s[..idx], &s[idx + op.len()..]))
}

fn unquote(s: &str) -> String {
    if (s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')) {
        if s.len() >= 2 {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

fn unwrap_regex(s: &str) -> String {
    if s.starts_with('/') && s.ends_with('/') && s.len() >= 2 {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
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
}
