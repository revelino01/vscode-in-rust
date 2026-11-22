//! Jupyter `.ipynb` notebook model, nbformat v4 parser/serializer, pure-Rust kernel, and GPUI view.

use parking_lot::RwLock;
use rustcode_editor::{GrammarRegistry, GrammarState, SyntaxHighlighter};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, KeyEvent, MouseButton, Point,
    Quad, Rect, Scene, View, WorkbenchThemeStyles,
};
use rustcode_platform::theme::{ColorTheme, ThemeService};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers for nbformat v4 serialization/deserialization
// ---------------------------------------------------------------------------

fn default_nbformat() -> u32 {
    4
}

fn default_nbformat_minor() -> u32 {
    5
}

fn default_empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

fn generate_cell_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn join_ipynb_lines(lines: Vec<String>) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let has_newlines = lines.iter().any(|l| l.ends_with('\n'));
    if has_newlines {
        lines.concat()
    } else {
        lines.join("\n")
    }
}

fn split_ipynb_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split_inclusive('\n').map(|s| s.to_string()).collect()
    }
}

fn deserialize_string_or_lines<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrLines {
        Str(String),
        Lines(Vec<String>),
        Null,
    }

    match Option::<StringOrLines>::deserialize(deserializer)? {
        Some(StringOrLines::Str(s)) => Ok(s),
        Some(StringOrLines::Lines(lines)) => Ok(join_ipynb_lines(lines)),
        Some(StringOrLines::Null) | None => Ok(String::new()),
    }
}

fn serialize_lines<S>(text: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let lines = split_ipynb_lines(text);
    lines.serialize(serializer)
}

fn deserialize_cell_kind<'de, D>(deserializer: D) -> Result<CellKind, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    match s.to_ascii_lowercase().as_str() {
        "code" => Ok(CellKind::Code),
        "markdown" => Ok(CellKind::Markdown),
        "raw" => Ok(CellKind::Raw),
        other => Err(serde::de::Error::custom(format!(
            "unsupported cell_type: {}",
            other
        ))),
    }
}

fn value_to_plain_text(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Array(arr) => {
            let lines: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            join_ipynb_lines(lines)
        }
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Notebook Data Model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CellKind {
    Code,
    Markdown,
    #[serde(rename = "raw")]
    Raw,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "output_type")]
pub enum CellOutput {
    #[serde(rename = "stream")]
    Stream {
        name: String,
        #[serde(
            default,
            deserialize_with = "deserialize_string_or_lines",
            serialize_with = "serialize_lines"
        )]
        text: String,
    },
    #[serde(rename = "execute_result")]
    ExecuteResult {
        #[serde(default)]
        execution_count: Option<u32>,
        #[serde(default)]
        data: HashMap<String, Value>,
        #[serde(default = "default_empty_object")]
        metadata: Value,
    },
    #[serde(rename = "display_data")]
    DisplayData {
        #[serde(default)]
        data: HashMap<String, Value>,
        #[serde(default = "default_empty_object")]
        metadata: Value,
    },
    #[serde(rename = "error")]
    Error {
        ename: String,
        evalue: String,
        #[serde(default)]
        traceback: Vec<String>,
    },
}

impl CellOutput {
    pub fn stdout(text: impl Into<String>) -> Self {
        Self::Stream {
            name: "stdout".to_string(),
            text: text.into(),
        }
    }

    pub fn stderr(text: impl Into<String>) -> Self {
        Self::Stream {
            name: "stderr".to_string(),
            text: text.into(),
        }
    }

    pub fn execute_result_text(execution_count: u32, text: impl Into<String>) -> Self {
        let mut data = HashMap::new();
        data.insert("text/plain".to_string(), Value::String(text.into()));
        Self::ExecuteResult {
            execution_count: Some(execution_count),
            data,
            metadata: default_empty_object(),
        }
    }

    pub fn display_text(text: impl Into<String>) -> Self {
        let mut data = HashMap::new();
        data.insert("text/plain".to_string(), Value::String(text.into()));
        Self::DisplayData {
            data,
            metadata: default_empty_object(),
        }
    }

    pub fn error(
        ename: impl Into<String>,
        evalue: impl Into<String>,
        traceback: Vec<String>,
    ) -> Self {
        Self::Error {
            ename: ename.into(),
            evalue: evalue.into(),
            traceback,
        }
    }

    pub fn plain_text(&self) -> String {
        match self {
            Self::Stream { text, .. } => text.clone(),
            Self::ExecuteResult { data, .. } | Self::DisplayData { data, .. } => {
                if let Some(val) = data.get("text/plain") {
                    value_to_plain_text(val)
                } else if let Some(val) = data.get("text/markdown") {
                    value_to_plain_text(val)
                } else if let Some(val) = data.get("text/html") {
                    value_to_plain_text(val)
                } else if let Some((_, first)) = data.iter().next() {
                    value_to_plain_text(first)
                } else {
                    String::new()
                }
            }
            Self::Error {
                ename,
                evalue,
                traceback,
            } => {
                if traceback.is_empty() {
                    format!("{}: {}", ename, evalue)
                } else {
                    format!("{}: {}\n{}", ename, evalue, traceback.join("\n"))
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotebookCell {
    #[serde(default = "generate_cell_id")]
    pub id: String,
    #[serde(rename = "cell_type", deserialize_with = "deserialize_cell_kind")]
    pub kind: CellKind,
    #[serde(
        default,
        deserialize_with = "deserialize_string_or_lines",
        serialize_with = "serialize_lines"
    )]
    pub source: String,
    #[serde(default)]
    pub outputs: Vec<CellOutput>,
    #[serde(default)]
    pub execution_count: Option<u32>,
    #[serde(default = "default_empty_object")]
    pub metadata: Value,
    #[serde(skip)]
    pub is_executing: bool,
}

impl NotebookCell {
    pub fn code(source: impl Into<String>) -> Self {
        Self {
            id: generate_cell_id(),
            kind: CellKind::Code,
            source: source.into(),
            outputs: Vec::new(),
            execution_count: None,
            metadata: default_empty_object(),
            is_executing: false,
        }
    }

    pub fn markdown(source: impl Into<String>) -> Self {
        Self {
            id: generate_cell_id(),
            kind: CellKind::Markdown,
            source: source.into(),
            outputs: Vec::new(),
            execution_count: None,
            metadata: default_empty_object(),
            is_executing: false,
        }
    }

    pub fn raw(source: impl Into<String>) -> Self {
        Self {
            id: generate_cell_id(),
            kind: CellKind::Raw,
            source: source.into(),
            outputs: Vec::new(),
            execution_count: None,
            metadata: default_empty_object(),
            is_executing: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotebookDocument {
    #[serde(skip)]
    pub uri: String,
    #[serde(skip)]
    pub path: Option<PathBuf>,
    #[serde(default = "default_nbformat")]
    pub nbformat: u32,
    #[serde(default = "default_nbformat_minor")]
    pub nbformat_minor: u32,
    #[serde(default = "default_empty_object")]
    pub metadata: Value,
    #[serde(default)]
    pub cells: Vec<NotebookCell>,
    #[serde(skip)]
    pub is_dirty: bool,
}

fn resolve_path_from_uri(uri: &str) -> Option<PathBuf> {
    if uri.starts_with("untitled:") || uri.starts_with("rustcode:") {
        None
    } else {
        Some(rustcode_platform::file_uri_to_path(uri))
    }
}

impl NotebookDocument {
    pub fn new(uri: impl Into<String>) -> Self {
        let uri_str = uri.into();
        let path = resolve_path_from_uri(&uri_str);
        Self {
            uri: uri_str,
            path,
            nbformat: 4,
            nbformat_minor: 5,
            metadata: json!({
                "kernelspec": {
                    "display_name": "Pure-Rust Kernel",
                    "language": "python",
                    "name": "rustcode-pure"
                },
                "language_info": {
                    "name": "python"
                }
            }),
            cells: vec![NotebookCell::code("")],
            is_dirty: false,
        }
    }

    pub fn from_ipynb_json(uri: &str, json_str: &str) -> Result<Self, String> {
        if json_str.trim().is_empty() {
            return Ok(Self::new(uri));
        }
        let mut doc: Self =
            serde_json::from_str(json_str).map_err(|e| format!("Invalid .ipynb JSON: {}", e))?;
        doc.uri = uri.to_string();
        doc.path = resolve_path_from_uri(uri);
        doc.is_dirty = false;
        for cell in &mut doc.cells {
            if cell.id.trim().is_empty() {
                cell.id = generate_cell_id();
            }
            if !cell.metadata.is_object() {
                cell.metadata = default_empty_object();
            }
        }
        if !doc.metadata.is_object() {
            doc.metadata = default_empty_object();
        }
        Ok(doc)
    }

    pub fn to_ipynb_json(&self) -> Result<String, String> {
        let serialized_cells: Vec<Value> = self
            .cells
            .iter()
            .map(|cell| {
                let source_lines = split_ipynb_lines(&cell.source);
                match cell.kind {
                    CellKind::Code => json!({
                        "id": cell.id,
                        "cell_type": "code",
                        "execution_count": cell.execution_count,
                        "metadata": cell.metadata,
                        "outputs": cell.outputs,
                        "source": source_lines,
                    }),
                    CellKind::Markdown => json!({
                        "id": cell.id,
                        "cell_type": "markdown",
                        "metadata": cell.metadata,
                        "source": source_lines,
                    }),
                    CellKind::Raw => json!({
                        "id": cell.id,
                        "cell_type": "raw",
                        "metadata": cell.metadata,
                        "source": source_lines,
                    }),
                }
            })
            .collect();

        let root = json!({
            "cells": serialized_cells,
            "metadata": self.metadata,
            "nbformat": self.nbformat,
            "nbformat_minor": self.nbformat_minor,
        });

        serde_json::to_string_pretty(&root)
            .map_err(|e| format!("Failed to serialize .ipynb JSON: {}", e))
    }

    pub fn add_cell(
        &mut self,
        index: Option<usize>,
        kind: CellKind,
        source: impl Into<String>,
    ) -> usize {
        let cell = match kind {
            CellKind::Code => NotebookCell::code(source),
            CellKind::Markdown => NotebookCell::markdown(source),
            CellKind::Raw => NotebookCell::raw(source),
        };
        let insert_idx = index.unwrap_or(self.cells.len()).min(self.cells.len());
        self.cells.insert(insert_idx, cell);
        self.is_dirty = true;
        insert_idx
    }

    pub fn delete_cell(&mut self, index: usize) -> Option<NotebookCell> {
        if index < self.cells.len() {
            self.is_dirty = true;
            Some(self.cells.remove(index))
        } else {
            None
        }
    }

    pub fn move_cell_up(&mut self, index: usize) -> bool {
        if index > 0 && index < self.cells.len() {
            self.cells.swap(index - 1, index);
            self.is_dirty = true;
            true
        } else {
            false
        }
    }

    pub fn move_cell_down(&mut self, index: usize) -> bool {
        if index + 1 < self.cells.len() {
            self.cells.swap(index, index + 1);
            self.is_dirty = true;
            true
        } else {
            false
        }
    }

    pub fn update_cell_source(&mut self, index: usize, source: impl Into<String>) -> bool {
        if let Some(cell) = self.cells.get_mut(index) {
            let next = source.into();
            if cell.source != next {
                cell.source = next;
                self.is_dirty = true;
            }
            true
        } else {
            false
        }
    }

    pub fn clear_cell_outputs(&mut self, index: usize) -> bool {
        if let Some(cell) = self.cells.get_mut(index) {
            if !cell.outputs.is_empty() || cell.execution_count.is_some() {
                cell.outputs.clear();
                cell.execution_count = None;
                self.is_dirty = true;
            }
            true
        } else {
            false
        }
    }

    pub fn clear_outputs(&mut self, index: usize) -> bool {
        self.clear_cell_outputs(index)
    }

    pub fn clear_all_outputs(&mut self) {
        let mut changed = false;
        for cell in &mut self.cells {
            if !cell.outputs.is_empty() || cell.execution_count.is_some() {
                cell.outputs.clear();
                cell.execution_count = None;
                changed = true;
            }
        }
        if changed {
            self.is_dirty = true;
        }
    }

    pub fn language(&self) -> &str {
        self.metadata
            .get("language_info")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                self.metadata
                    .get("kernelspec")
                    .and_then(|v| v.get("language"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("python")
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust NotebookKernel
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum KernelValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl KernelValue {
    fn type_name(&self) -> &'static str {
        match self {
            Self::Int(_) => "int",
            Self::Float(_) => "float",
            Self::Bool(_) => "bool",
            Self::Str(_) => "str",
        }
    }

    fn is_truthy(&self) -> bool {
        match self {
            Self::Int(n) => *n != 0,
            Self::Float(f) => *f != 0.0,
            Self::Bool(b) => *b,
            Self::Str(s) => !s.is_empty(),
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(n) => Some(*n as f64),
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }
}

impl fmt::Display for KernelValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(n) => write!(f, "{}", n),
            Self::Float(v) => {
                if v.fract() == 0.0 && v.is_finite() {
                    write!(f, "{:.1}", v)
                } else {
                    write!(f, "{}", v)
                }
            }
            Self::Bool(b) => write!(f, "{}", b),
            Self::Str(s) => write!(f, "{}", s),
        }
    }
}

#[derive(Debug, Clone)]
struct KernelEvalError {
    ename: String,
    evalue: String,
}

impl KernelEvalError {
    fn new(ename: &str, evalue: impl Into<String>) -> Self {
        Self {
            ename: ename.to_string(),
            evalue: evalue.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    EqEq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    AndAnd,
    OrOr,
    LParen,
    RParen,
    Comma,
}

fn tokenize_expr(input: &str) -> Result<Vec<Token>, KernelEvalError> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }

        // String literal
        if ch == '"' || ch == '\'' {
            let quote = ch;
            i += 1;
            let mut s = String::new();
            let mut closed = false;
            while i < chars.len() {
                let c = chars[i];
                if c == '\\' && i + 1 < chars.len() {
                    i += 1;
                    match chars[i] {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        '\'' => s.push('\''),
                        other => s.push(other),
                    }
                    i += 1;
                } else if c == quote {
                    i += 1;
                    closed = true;
                    break;
                } else {
                    s.push(c);
                    i += 1;
                }
            }
            if !closed {
                return Err(KernelEvalError::new(
                    "SyntaxError",
                    "Unterminated string literal",
                ));
            }
            tokens.push(Token::Str(s));
            continue;
        }

        // Number literal
        if ch.is_ascii_digit() {
            let start = i;
            let mut is_float = false;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i < chars.len()
                && chars[i] == '.'
                && i + 1 < chars.len()
                && chars[i + 1].is_ascii_digit()
            {
                is_float = true;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let num_str: String = chars[start..i].iter().collect();
            if is_float {
                let val = num_str.parse::<f64>().map_err(|_| {
                    KernelEvalError::new("SyntaxError", format!("Invalid float: {}", num_str))
                })?;
                tokens.push(Token::Float(val));
            } else {
                let val = num_str.parse::<i64>().map_err(|_| {
                    KernelEvalError::new("SyntaxError", format!("Invalid integer: {}", num_str))
                })?;
                tokens.push(Token::Int(val));
            }
            continue;
        }

        // Identifier or keyword (including Rust macro style `println!`)
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let mut ident: String = chars[start..i].iter().collect();
            if ident == "println" && i < chars.len() && chars[i] == '!' {
                ident.push('!');
                i += 1;
            }
            match ident.as_str() {
                "true" | "True" => tokens.push(Token::Bool(true)),
                "false" | "False" => tokens.push(Token::Bool(false)),
                "and" => tokens.push(Token::AndAnd),
                "or" => tokens.push(Token::OrOr),
                "not" => tokens.push(Token::Bang),
                _ => tokens.push(Token::Ident(ident)),
            }
            continue;
        }

        // Two-char operators
        let next = chars.get(i + 1).copied();
        match (ch, next) {
            ('=', Some('=')) => {
                tokens.push(Token::EqEq);
                i += 2;
            }
            ('!', Some('=')) => {
                tokens.push(Token::NotEq);
                i += 2;
            }
            ('<', Some('=')) => {
                tokens.push(Token::LtEq);
                i += 2;
            }
            ('>', Some('=')) => {
                tokens.push(Token::GtEq);
                i += 2;
            }
            ('&', Some('&')) => {
                tokens.push(Token::AndAnd);
                i += 2;
            }
            ('|', Some('|')) => {
                tokens.push(Token::OrOr);
                i += 2;
            }
            ('+', _) => {
                tokens.push(Token::Plus);
                i += 1;
            }
            ('-', _) => {
                tokens.push(Token::Minus);
                i += 1;
            }
            ('*', _) => {
                tokens.push(Token::Star);
                i += 1;
            }
            ('/', _) => {
                tokens.push(Token::Slash);
                i += 1;
            }
            ('%', _) => {
                tokens.push(Token::Percent);
                i += 1;
            }
            ('!', _) => {
                tokens.push(Token::Bang);
                i += 1;
            }
            ('<', _) => {
                tokens.push(Token::Lt);
                i += 1;
            }
            ('>', _) => {
                tokens.push(Token::Gt);
                i += 1;
            }
            ('(', _) => {
                tokens.push(Token::LParen);
                i += 1;
            }
            (')', _) => {
                tokens.push(Token::RParen);
                i += 1;
            }
            (',', _) => {
                tokens.push(Token::Comma);
                i += 1;
            }
            _ => {
                return Err(KernelEvalError::new(
                    "SyntaxError",
                    format!("Unexpected character '{}'", ch),
                ));
            }
        }
    }

    Ok(tokens)
}

struct ExprParser<'a> {
    tokens: &'a [Token],
    pos: usize,
    variables: &'a HashMap<String, KernelValue>,
    stdout: &'a mut String,
    produced_output_only: bool,
}

impl<'a> ExprParser<'a> {
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

    fn parse_or(&mut self) -> Result<KernelValue, KernelEvalError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Token::OrOr)) {
            self.advance();
            let right = self.parse_and()?;
            left = KernelValue::Bool(left.is_truthy() || right.is_truthy());
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<KernelValue, KernelEvalError> {
        let mut left = self.parse_eq()?;
        while matches!(self.peek(), Some(Token::AndAnd)) {
            self.advance();
            let right = self.parse_eq()?;
            left = KernelValue::Bool(left.is_truthy() && right.is_truthy());
        }
        Ok(left)
    }

    fn parse_eq(&mut self) -> Result<KernelValue, KernelEvalError> {
        let mut left = self.parse_cmp()?;
        while let Some(op) = self.peek().cloned() {
            match op {
                Token::EqEq => {
                    self.advance();
                    let right = self.parse_cmp()?;
                    left = KernelValue::Bool(values_equal(&left, &right));
                }
                Token::NotEq => {
                    self.advance();
                    let right = self.parse_cmp()?;
                    left = KernelValue::Bool(!values_equal(&left, &right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_cmp(&mut self) -> Result<KernelValue, KernelEvalError> {
        let mut left = self.parse_add()?;
        while let Some(op) = self.peek().cloned() {
            match op {
                Token::Lt | Token::LtEq | Token::Gt | Token::GtEq => {
                    self.advance();
                    let right = self.parse_add()?;
                    let ord = compare_values(&left, &right)?;
                    let res = match op {
                        Token::Lt => ord < 0,
                        Token::LtEq => ord <= 0,
                        Token::Gt => ord > 0,
                        Token::GtEq => ord >= 0,
                        _ => unreachable!(),
                    };
                    left = KernelValue::Bool(res);
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<KernelValue, KernelEvalError> {
        let mut left = self.parse_mul()?;
        while let Some(op) = self.peek().cloned() {
            match op {
                Token::Plus => {
                    self.advance();
                    let right = self.parse_mul()?;
                    left = match (left, right) {
                        (KernelValue::Int(a), KernelValue::Int(b)) => {
                            let sum = a.checked_add(b).ok_or_else(|| {
                                KernelEvalError::new("OverflowError", "integer addition overflow")
                            })?;
                            KernelValue::Int(sum)
                        }
                        (KernelValue::Float(a), KernelValue::Float(b)) => KernelValue::Float(a + b),
                        (KernelValue::Int(a), KernelValue::Float(b)) => {
                            KernelValue::Float(a as f64 + b)
                        }
                        (KernelValue::Float(a), KernelValue::Int(b)) => {
                            KernelValue::Float(a + b as f64)
                        }
                        (KernelValue::Str(a), other) => {
                            KernelValue::Str(format!("{}{}", a, other))
                        }
                        (other, KernelValue::Str(b)) => {
                            KernelValue::Str(format!("{}{}", other, b))
                        }
                        (l, r) => {
                            return Err(KernelEvalError::new(
                                "TypeError",
                                format!("Cannot add {} and {}", l.type_name(), r.type_name()),
                            ));
                        }
                    };
                }
                Token::Minus => {
                    self.advance();
                    let right = self.parse_mul()?;
                    left = match (left, right) {
                        (KernelValue::Int(a), KernelValue::Int(b)) => {
                            let diff = a.checked_sub(b).ok_or_else(|| {
                                KernelEvalError::new(
                                    "OverflowError",
                                    "integer subtraction overflow",
                                )
                            })?;
                            KernelValue::Int(diff)
                        }
                        (KernelValue::Float(a), KernelValue::Float(b)) => KernelValue::Float(a - b),
                        (KernelValue::Int(a), KernelValue::Float(b)) => {
                            KernelValue::Float(a as f64 - b)
                        }
                        (KernelValue::Float(a), KernelValue::Int(b)) => {
                            KernelValue::Float(a - b as f64)
                        }
                        (l, r) => {
                            return Err(KernelEvalError::new(
                                "TypeError",
                                format!("Cannot subtract {} and {}", l.type_name(), r.type_name()),
                            ));
                        }
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<KernelValue, KernelEvalError> {
        let mut left = self.parse_unary()?;
        while let Some(op) = self.peek().cloned() {
            match op {
                Token::Star => {
                    self.advance();
                    let right = self.parse_unary()?;
                    left = match (left, right) {
                        (KernelValue::Int(a), KernelValue::Int(b)) => {
                            let prod = a.checked_mul(b).ok_or_else(|| {
                                KernelEvalError::new(
                                    "OverflowError",
                                    "integer multiplication overflow",
                                )
                            })?;
                            KernelValue::Int(prod)
                        }
                        (KernelValue::Float(a), KernelValue::Float(b)) => KernelValue::Float(a * b),
                        (KernelValue::Int(a), KernelValue::Float(b)) => {
                            KernelValue::Float(a as f64 * b)
                        }
                        (KernelValue::Float(a), KernelValue::Int(b)) => {
                            KernelValue::Float(a * b as f64)
                        }
                        (KernelValue::Str(s), KernelValue::Int(n))
                        | (KernelValue::Int(n), KernelValue::Str(s)) => {
                            if n <= 0 || s.is_empty() {
                                KernelValue::Str(String::new())
                            } else {
                                let count = usize::try_from(n).unwrap_or(usize::MAX);
                                if s.len().saturating_mul(count) > 1_000_000 {
                                    return Err(KernelEvalError::new(
                                        "OverflowError",
                                        "string repetition exceeds maximum length",
                                    ));
                                }
                                KernelValue::Str(s.repeat(count))
                            }
                        }
                        (l, r) => {
                            return Err(KernelEvalError::new(
                                "TypeError",
                                format!("Cannot multiply {} and {}", l.type_name(), r.type_name()),
                            ));
                        }
                    };
                }
                Token::Slash => {
                    self.advance();
                    let right = self.parse_unary()?;
                    left = match (left, right) {
                        (KernelValue::Int(_), KernelValue::Int(0))
                        | (KernelValue::Float(_), KernelValue::Int(0)) => {
                            return Err(KernelEvalError::new(
                                "ZeroDivisionError",
                                "division by zero",
                            ));
                        }
                        (KernelValue::Int(_), KernelValue::Float(0.0)) => {
                            return Err(KernelEvalError::new(
                                "ZeroDivisionError",
                                "division by zero",
                            ));
                        }
                        (KernelValue::Float(_), KernelValue::Float(0.0)) => {
                            return Err(KernelEvalError::new(
                                "ZeroDivisionError",
                                "division by zero",
                            ));
                        }
                        (KernelValue::Int(a), KernelValue::Int(b)) => {
                            let quot = a.checked_div(b).ok_or_else(|| {
                                KernelEvalError::new("OverflowError", "integer division overflow")
                            })?;
                            KernelValue::Int(quot)
                        }
                        (KernelValue::Float(a), KernelValue::Float(b)) => KernelValue::Float(a / b),
                        (KernelValue::Int(a), KernelValue::Float(b)) => {
                            KernelValue::Float(a as f64 / b)
                        }
                        (KernelValue::Float(a), KernelValue::Int(b)) => {
                            KernelValue::Float(a / b as f64)
                        }
                        (l, r) => {
                            return Err(KernelEvalError::new(
                                "TypeError",
                                format!("Cannot divide {} by {}", l.type_name(), r.type_name()),
                            ));
                        }
                    };
                }
                Token::Percent => {
                    self.advance();
                    let right = self.parse_unary()?;
                    left = match (left, right) {
                        (KernelValue::Int(_), KernelValue::Int(0))
                        | (KernelValue::Float(_), KernelValue::Int(0))
                        | (KernelValue::Int(_), KernelValue::Float(0.0))
                        | (KernelValue::Float(_), KernelValue::Float(0.0)) => {
                            return Err(KernelEvalError::new(
                                "ZeroDivisionError",
                                "modulo by zero",
                            ));
                        }
                        (KernelValue::Int(a), KernelValue::Int(b)) => {
                            let rem = a.checked_rem(b).ok_or_else(|| {
                                KernelEvalError::new("OverflowError", "integer modulo overflow")
                            })?;
                            KernelValue::Int(rem)
                        }
                        (KernelValue::Float(a), KernelValue::Float(b)) => KernelValue::Float(a % b),
                        (KernelValue::Int(a), KernelValue::Float(b)) => {
                            KernelValue::Float(a as f64 % b)
                        }
                        (KernelValue::Float(a), KernelValue::Int(b)) => {
                            KernelValue::Float(a % (b as f64))
                        }
                        (l, r) => {
                            return Err(KernelEvalError::new(
                                "TypeError",
                                format!("Cannot modulo {} by {}", l.type_name(), r.type_name()),
                            ));
                        }
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<KernelValue, KernelEvalError> {
        match self.peek().cloned() {
            Some(Token::Minus) => {
                self.advance();
                let val = self.parse_unary()?;
                match val {
                    KernelValue::Int(n) => n.checked_neg().map(KernelValue::Int).ok_or_else(|| {
                        KernelEvalError::new("OverflowError", "integer negation overflow")
                    }),
                    KernelValue::Float(f) => Ok(KernelValue::Float(-f)),
                    other => Err(KernelEvalError::new(
                        "TypeError",
                        format!("Cannot negate {}", other.type_name()),
                    )),
                }
            }
            Some(Token::Bang) => {
                self.advance();
                let val = self.parse_unary()?;
                Ok(KernelValue::Bool(!val.is_truthy()))
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<KernelValue, KernelEvalError> {
        match self.advance().cloned() {
            Some(Token::Int(n)) => Ok(KernelValue::Int(n)),
            Some(Token::Float(f)) => Ok(KernelValue::Float(f)),
            Some(Token::Bool(b)) => Ok(KernelValue::Bool(b)),
            Some(Token::Str(s)) => Ok(KernelValue::Str(s)),
            Some(Token::LParen) => {
                let val = self.parse_or()?;
                if !matches!(self.advance(), Some(Token::RParen)) {
                    return Err(KernelEvalError::new("SyntaxError", "Expected ')'"));
                }
                Ok(val)
            }
            Some(Token::Ident(name)) => {
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.advance(); // consume '('
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        loop {
                            args.push(self.parse_or()?);
                            if matches!(self.peek(), Some(Token::Comma)) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    if !matches!(self.advance(), Some(Token::RParen)) {
                        return Err(KernelEvalError::new(
                            "SyntaxError",
                            format!("Expected ')' in call to {}", name),
                        ));
                    }
                    self.eval_builtin_call(&name, &args)
                } else {
                    self.variables.get(&name).cloned().ok_or_else(|| {
                        KernelEvalError::new(
                            "NameError",
                            format!("name '{}' is not defined", name),
                        )
                    })
                }
            }
            Some(other) => Err(KernelEvalError::new(
                "SyntaxError",
                format!("Unexpected token {:?}", other),
            )),
            None => Err(KernelEvalError::new("SyntaxError", "Unexpected end of expression")),
        }
    }

    fn eval_builtin_call(
        &mut self,
        name: &str,
        args: &[KernelValue],
    ) -> Result<KernelValue, KernelEvalError> {
        match name {
            "print" | "println" | "println!" => {
                let formatted = args
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                self.stdout.push_str(&formatted);
                self.stdout.push('\n');
                self.produced_output_only = true;
                Ok(KernelValue::Str(String::new()))
            }
            "len" => {
                if args.len() != 1 {
                    return Err(KernelEvalError::new(
                        "TypeError",
                        "len() takes exactly 1 argument",
                    ));
                }
                match &args[0] {
                    KernelValue::Str(s) => Ok(KernelValue::Int(s.chars().count() as i64)),
                    other => Err(KernelEvalError::new(
                        "TypeError",
                        format!("object of type '{}' has no len()", other.type_name()),
                    )),
                }
            }
            "str" => {
                if args.len() != 1 {
                    return Err(KernelEvalError::new(
                        "TypeError",
                        "str() takes exactly 1 argument",
                    ));
                }
                Ok(KernelValue::Str(args[0].to_string()))
            }
            "int" => {
                if args.len() != 1 {
                    return Err(KernelEvalError::new(
                        "TypeError",
                        "int() takes exactly 1 argument",
                    ));
                }
                match &args[0] {
                    KernelValue::Int(n) => Ok(KernelValue::Int(*n)),
                    KernelValue::Float(f) => Ok(KernelValue::Int(*f as i64)),
                    KernelValue::Bool(b) => Ok(KernelValue::Int(if *b { 1 } else { 0 })),
                    KernelValue::Str(s) => s.trim().parse::<i64>().map(KernelValue::Int).map_err(|_| {
                        KernelEvalError::new("ValueError", format!("invalid literal for int(): '{}'", s))
                    }),
                }
            }
            "float" => {
                if args.len() != 1 {
                    return Err(KernelEvalError::new(
                        "TypeError",
                        "float() takes exactly 1 argument",
                    ));
                }
                match &args[0] {
                    KernelValue::Int(n) => Ok(KernelValue::Float(*n as f64)),
                    KernelValue::Float(f) => Ok(KernelValue::Float(*f)),
                    KernelValue::Bool(b) => Ok(KernelValue::Float(if *b { 1.0 } else { 0.0 })),
                    KernelValue::Str(s) => s.trim().parse::<f64>().map(KernelValue::Float).map_err(|_| {
                        KernelEvalError::new("ValueError", format!("could not convert string to float: '{}'", s))
                    }),
                }
            }
            "abs" => {
                if args.len() != 1 {
                    return Err(KernelEvalError::new(
                        "TypeError",
                        "abs() takes exactly 1 argument",
                    ));
                }
                match &args[0] {
                    KernelValue::Int(n) => n.checked_abs().map(KernelValue::Int).ok_or_else(|| {
                        KernelEvalError::new("OverflowError", "integer abs overflow")
                    }),
                    KernelValue::Float(f) => Ok(KernelValue::Float(f.abs())),
                    other => Err(KernelEvalError::new(
                        "TypeError",
                        format!("bad operand type for abs(): '{}'", other.type_name()),
                    )),
                }
            }
            "type" => {
                if args.len() != 1 {
                    return Err(KernelEvalError::new(
                        "TypeError",
                        "type() takes exactly 1 argument",
                    ));
                }
                Ok(KernelValue::Str(args[0].type_name().to_string()))
            }
            _ => Err(KernelEvalError::new(
                "NameError",
                format!("name '{}' is not defined", name),
            )),
        }
    }
}

fn values_equal(a: &KernelValue, b: &KernelValue) -> bool {
    match (a, b) {
        (KernelValue::Int(x), KernelValue::Int(y)) => x == y,
        (KernelValue::Float(x), KernelValue::Float(y)) => x == y,
        (KernelValue::Int(x), KernelValue::Float(y)) => (*x as f64) == *y,
        (KernelValue::Float(x), KernelValue::Int(y)) => *x == (*y as f64),
        (KernelValue::Bool(x), KernelValue::Bool(y)) => x == y,
        (KernelValue::Str(x), KernelValue::Str(y)) => x == y,
        _ => false,
    }
}

fn compare_values(a: &KernelValue, b: &KernelValue) -> Result<i8, KernelEvalError> {
    if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) {
        if x < y {
            Ok(-1)
        } else if x > y {
            Ok(1)
        } else {
            Ok(0)
        }
    } else if let (KernelValue::Str(x), KernelValue::Str(y)) = (a, b) {
        Ok(match x.cmp(y) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        })
    } else {
        Err(KernelEvalError::new(
            "TypeError",
            format!(
                "'<' not supported between instances of '{}' and '{}'",
                a.type_name(),
                b.type_name()
            ),
        ))
    }
}

fn strip_line_comment(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut in_quote: Option<char> = None;
    let mut escaped = false;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if in_quote.is_some() && c == '\\' {
            escaped = true;
            i += 1;
            continue;
        }
        if let Some(q) = in_quote {
            if c == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            in_quote = Some(c);
            i += 1;
            continue;
        }
        if c == '#' || (c == '/' && i + 1 < chars.len() && chars[i + 1] == '/') {
            return chars[..i].iter().collect();
        }
        i += 1;
    }
    line.to_string()
}

fn split_statements(source: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    for raw_line in source.lines() {
        let clean_line = strip_line_comment(raw_line);
        let chars: Vec<char> = clean_line.chars().collect();
        let mut in_quote: Option<char> = None;
        let mut escaped = false;
        let mut paren_depth: usize = 0;
        let mut start = 0;

        for (idx, &c) in chars.iter().enumerate() {
            if escaped {
                escaped = false;
                continue;
            }
            if in_quote.is_some() && c == '\\' {
                escaped = true;
                continue;
            }
            if let Some(q) = in_quote {
                if c == q {
                    in_quote = None;
                }
                continue;
            }
            if c == '"' || c == '\'' {
                in_quote = Some(c);
            } else if c == '(' {
                paren_depth += 1;
            } else if c == ')' {
                paren_depth = paren_depth.saturating_sub(1);
            } else if c == ';' && paren_depth == 0 {
                let piece: String = chars[start..idx].iter().collect();
                let trimmed = piece.trim();
                if !trimmed.is_empty() {
                    stmts.push(trimmed.to_string());
                }
                start = idx + 1;
            }
        }
        let tail: String = chars[start..].iter().collect();
        let trimmed = tail.trim();
        if !trimmed.is_empty() {
            stmts.push(trimmed.to_string());
        }
    }
    stmts
}

fn find_assignment_eq(stmt: &str) -> Option<usize> {
    let bytes = stmt.as_bytes();
    let mut in_quote: Option<u8> = None;
    let mut escaped = false;
    let mut paren_depth: usize = 0;

    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if in_quote.is_some() && b == b'\\' {
            escaped = true;
            i += 1;
            continue;
        }
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            in_quote = Some(b);
        } else if b == b'(' {
            paren_depth += 1;
        } else if b == b')' {
            paren_depth = paren_depth.saturating_sub(1);
        } else if b == b'=' && paren_depth == 0 {
            let prev = if i > 0 { Some(bytes[i - 1]) } else { None };
            let next = bytes.get(i + 1).copied();
            let is_compound = matches!(prev, Some(b'!' | b'<' | b'>' | b'='))
                || matches!(next, Some(b'='));
            if !is_compound {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn is_valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Debug, Clone)]
pub struct NotebookKernel {
    pub id: String,
    pub label: String,
    pub language: String,
    pub execution_counter: u32,
    pub variables: HashMap<String, KernelValue>,
}

impl Default for NotebookKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl NotebookKernel {
    pub fn new() -> Self {
        Self {
            id: "rustcode-pure".to_string(),
            label: "Pure-Rust Kernel".to_string(),
            language: "python".to_string(),
            execution_counter: 0,
            variables: HashMap::new(),
        }
    }

    pub fn restart(&mut self) {
        self.execution_counter = 0;
        self.variables.clear();
    }

    fn eval_expr(
        &self,
        expr_str: &str,
        stdout: &mut String,
    ) -> Result<(KernelValue, bool), KernelEvalError> {
        let tokens = tokenize_expr(expr_str)?;
        if tokens.is_empty() {
            return Err(KernelEvalError::new("SyntaxError", "Empty expression"));
        }
        let mut parser = ExprParser {
            tokens: &tokens,
            pos: 0,
            variables: &self.variables,
            stdout,
            produced_output_only: false,
        };
        let val = parser.parse_or()?;
        if parser.pos < parser.tokens.len() {
            return Err(KernelEvalError::new(
                "SyntaxError",
                format!("Unexpected token {:?}", parser.tokens[parser.pos]),
            ));
        }
        Ok((val, parser.produced_output_only))
    }

    pub fn execute_cell(&mut self, cell: &mut NotebookCell) -> Vec<CellOutput> {
        if cell.kind != CellKind::Code {
            return Vec::new();
        }

        self.execution_counter += 1;
        let exec_count = self.execution_counter;
        cell.execution_count = Some(exec_count);
        cell.is_executing = false;

        let statements = split_statements(&cell.source);
        let mut stdout = String::new();
        let mut last_result: Option<KernelValue> = None;
        let mut error_output: Option<CellOutput> = None;

        for stmt in statements {
            if let Some(eq_idx) = find_assignment_eq(&stmt) {
                let lhs_raw = stmt[..eq_idx].trim();
                let rhs_raw = stmt[eq_idx + 1..].trim();

                let var_name = if let Some(rest) = lhs_raw.strip_prefix("let ") {
                    let rest = rest.trim();
                    rest.strip_prefix("mut ").map(|s| s.trim()).unwrap_or(rest)
                } else {
                    lhs_raw
                };

                if !is_valid_ident(var_name) {
                    error_output = Some(CellOutput::error(
                        "SyntaxError",
                        format!("Invalid assignment target '{}'", lhs_raw),
                        vec![format!("  in statement: {}", stmt)],
                    ));
                    break;
                }
                if rhs_raw.is_empty() {
                    error_output = Some(CellOutput::error(
                        "SyntaxError",
                        "Missing right-hand side in assignment",
                        vec![format!("  in statement: {}", stmt)],
                    ));
                    break;
                }

                match self.eval_expr(rhs_raw, &mut stdout) {
                    Ok((val, _)) => {
                        self.variables.insert(var_name.to_string(), val);
                        last_result = None;
                    }
                    Err(err) => {
                        error_output = Some(CellOutput::error(
                            err.ename,
                            err.evalue,
                            vec![format!("  in statement: {}", stmt)],
                        ));
                        break;
                    }
                }
            } else {
                match self.eval_expr(&stmt, &mut stdout) {
                    Ok((val, output_only)) => {
                        if output_only {
                            last_result = None;
                        } else {
                            last_result = Some(val);
                        }
                    }
                    Err(err) => {
                        error_output = Some(CellOutput::error(
                            err.ename,
                            err.evalue,
                            vec![format!("  in statement: {}", stmt)],
                        ));
                        break;
                    }
                }
            }
        }

        let mut outputs = Vec::new();
        if !stdout.is_empty() {
            outputs.push(CellOutput::stdout(stdout));
        }
        if let Some(err) = error_output {
            outputs.push(err);
        } else if let Some(val) = last_result {
            outputs.push(CellOutput::execute_result_text(exec_count, val.to_string()));
        }

        cell.outputs = outputs.clone();
        outputs
    }

    pub fn execute_all(&mut self, doc: &mut NotebookDocument) {
        let mut executed_any = false;
        for cell in &mut doc.cells {
            if cell.kind == CellKind::Code {
                self.execute_cell(cell);
                executed_any = true;
            }
        }
        if executed_any {
            doc.is_dirty = true;
        }
    }
}

// ---------------------------------------------------------------------------
// GPUI NotebookEditorView
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotebookAction {
    CellExecuted(usize),
    AllExecuted,
    CellAdded(usize),
    CellDeleted(usize),
    CellMoved(usize, usize),
    OutputsCleared,
    KernelRestarted,
    DocumentChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookClickTarget {
    RunAll,
    AddCode,
    AddMarkdown,
    ClearOutputs,
    RestartKernel,
    RunCell(usize),
    MoveCellUp(usize),
    MoveCellDown(usize),
    DeleteCell(usize),
    CellBody(usize),
}

pub struct NotebookEditorView {
    pub document: NotebookDocument,
    pub kernel: NotebookKernel,
    pub active_cell_index: Option<usize>,
    pub editing_cell: bool,
    pub scroll_top: f32,
    pub last_action: Option<NotebookAction>,
    pub grammar_registry: Arc<GrammarRegistry>,
    pub theme: ColorTheme,
    pub hit_targets: RwLock<Vec<(Rect, NotebookClickTarget)>>,
    pub last_content_height: RwLock<f32>,
    pub last_viewport_height: RwLock<f32>,
    last_font: RwLock<FontMetrics>,
}

impl Default for NotebookEditorView {
    fn default() -> Self {
        Self::new(NotebookDocument::new("untitled://Untitled-1.ipynb"))
    }
}

impl NotebookEditorView {
    pub fn new(document: NotebookDocument) -> Self {
        let active_cell_index = if document.cells.is_empty() {
            None
        } else {
            Some(0)
        };
        let mut kernel = NotebookKernel::new();
        kernel.language = document.language().to_string();
        Self {
            document,
            kernel,
            active_cell_index,
            editing_cell: false,
            scroll_top: 0.0,
            last_action: None,
            grammar_registry: Arc::new(GrammarRegistry::new()),
            theme: ThemeService::default_dark_theme(),
            hit_targets: RwLock::new(Vec::new()),
            last_content_height: RwLock::new(0.0),
            last_viewport_height: RwLock::new(0.0),
            last_font: RwLock::new(FontMetrics::default_monospace()),
        }
    }

    pub fn with_grammar_registry(mut self, registry: Arc<GrammarRegistry>) -> Self {
        self.grammar_registry = registry;
        self
    }

    pub fn with_theme(mut self, theme: ColorTheme) -> Self {
        self.theme = theme;
        self
    }

    pub fn from_ipynb_str(uri: &str, content: &str) -> Self {
        let doc = match NotebookDocument::from_ipynb_json(uri, content) {
            Ok(d) => d,
            Err(_) => {
                let mut d = NotebookDocument::new(uri);
                if !content.trim().is_empty() {
                    d.cells = vec![NotebookCell::code(content)];
                }
                d
            }
        };
        Self::new(doc)
    }

    pub fn execute_cell(&mut self, index: usize) {
        if let Some(cell) = self.document.cells.get_mut(index) {
            if cell.kind == CellKind::Code {
                self.kernel.execute_cell(cell);
                self.document.is_dirty = true;
            }
            self.last_action = Some(NotebookAction::CellExecuted(index));
        }
    }

    pub fn execute_active_cell(&mut self) {
        if let Some(idx) = self.active_cell_index {
            self.execute_cell(idx);
        }
    }

    pub fn execute_all_cells(&mut self) {
        self.kernel.execute_all(&mut self.document);
        self.last_action = Some(NotebookAction::AllExecuted);
    }

    pub fn add_code_cell(&mut self) -> usize {
        let insert_at = self
            .active_cell_index
            .map(|i| (i + 1).min(self.document.cells.len()));
        let idx = self.document.add_cell(insert_at, CellKind::Code, "");
        self.active_cell_index = Some(idx);
        self.editing_cell = true;
        self.last_action = Some(NotebookAction::CellAdded(idx));
        idx
    }

    pub fn add_markdown_cell(&mut self) -> usize {
        let insert_at = self
            .active_cell_index
            .map(|i| (i + 1).min(self.document.cells.len()));
        let idx = self.document.add_cell(insert_at, CellKind::Markdown, "");
        self.active_cell_index = Some(idx);
        self.editing_cell = true;
        self.last_action = Some(NotebookAction::CellAdded(idx));
        idx
    }

    pub fn delete_cell(&mut self, index: usize) {
        if self.document.delete_cell(index).is_some() {
            if self.document.cells.is_empty() {
                self.active_cell_index = None;
                self.editing_cell = false;
            } else if let Some(active) = self.active_cell_index {
                if active >= self.document.cells.len() {
                    self.active_cell_index = Some(self.document.cells.len() - 1);
                } else if active > index {
                    self.active_cell_index = Some(active - 1);
                }
            }
            self.last_action = Some(NotebookAction::CellDeleted(index));
        }
    }

    pub fn move_cell_up(&mut self, index: usize) {
        if self.document.move_cell_up(index) {
            if self.active_cell_index == Some(index) {
                self.active_cell_index = Some(index - 1);
            } else if self.active_cell_index == Some(index - 1) {
                self.active_cell_index = Some(index);
            }
            self.last_action = Some(NotebookAction::CellMoved(index, index - 1));
        }
    }

    pub fn move_cell_down(&mut self, index: usize) {
        if self.document.move_cell_down(index) {
            if self.active_cell_index == Some(index) {
                self.active_cell_index = Some(index + 1);
            } else if self.active_cell_index == Some(index + 1) {
                self.active_cell_index = Some(index);
            }
            self.last_action = Some(NotebookAction::CellMoved(index, index + 1));
        }
    }

    pub fn clear_all_outputs(&mut self) {
        self.document.clear_all_outputs();
        self.last_action = Some(NotebookAction::OutputsCleared);
    }

    pub fn restart_kernel(&mut self) {
        self.kernel.restart();
        self.last_action = Some(NotebookAction::KernelRestarted);
    }

    fn key_event_to_char(key: &KeyEvent) -> Option<char> {
        if key.modifiers.ctrl || key.modifiers.alt || key.modifiers.meta {
            return None;
        }
        if let Some(ref txt) = key.text {
            let mut chars = txt.chars();
            if let (Some(ch), None) = (chars.next(), chars.next()) {
                if !ch.is_control() {
                    return Some(ch);
                }
            }
        }
        match key.key {
            KeyCode::Char(c) => Some(if key.modifiers.shift {
                c.to_ascii_uppercase()
            } else {
                c
            }),
            KeyCode::Space => Some(' '),
            _ => None,
        }
    }

    fn cell_source_lines(source: &str) -> Vec<&str> {
        if source.is_empty() {
            vec![""]
        } else {
            let mut lines: Vec<&str> = source.split('\n').collect();
            if lines.is_empty() {
                lines.push("");
            }
            lines
        }
    }

    fn cell_output_lines(cell: &NotebookCell) -> Vec<(String, bool)> {
        let mut lines = Vec::new();
        for out in &cell.outputs {
            let is_err = matches!(out, CellOutput::Error { .. });
            let text = out.plain_text();
            for line in text.lines() {
                lines.push((line.to_string(), is_err));
            }
            if text.is_empty() {
                lines.push((String::new(), is_err));
            }
        }
        lines
    }

    fn measure_cell_height(&self, cell: &NotebookCell, font: &FontMetrics) -> f32 {
        let header_h = 26.0;
        let line_h = font.line_height.max(16.0);
        let src_lines = Self::cell_source_lines(&cell.source).len().max(1) as f32;
        let src_box_h = src_lines * line_h + 12.0;
        let out_lines = Self::cell_output_lines(cell);
        let out_h = if out_lines.is_empty() {
            0.0
        } else {
            (out_lines.len() as f32 * line_h + 14.0) + 6.0
        };
        header_h + src_box_h + out_h + 12.0
    }
}

impl View for NotebookEditorView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !bounds.is_finite_positive()
            || !font.is_finite()
            || !(bounds.min_y() + 36.0).is_finite()
        {
            return;
        }
        *self.last_font.write() = font.clone();
        let mut targets = Vec::new();

        // Outer background
        scene.push_quad(Quad::new(bounds).with_background(theme.editor_background));

        // 1. Top Toolbar (36px)
        let toolbar_h = 36.0f32.min(bounds.height());
        let toolbar_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), toolbar_h);
        scene.push_quad(
            Quad::new(toolbar_rect)
                .with_background(theme.panel_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        let toolbar_buttons: &[(&str, NotebookClickTarget)] = &[
            ("Run All", NotebookClickTarget::RunAll),
            ("+ Code", NotebookClickTarget::AddCode),
            ("+ Markdown", NotebookClickTarget::AddMarkdown),
            ("Clear Outputs", NotebookClickTarget::ClearOutputs),
            ("Restart Kernel", NotebookClickTarget::RestartKernel),
        ];

        let mut btn_x = bounds.min_x() + 10.0;
        let btn_y = bounds.min_y() + 6.0;
        let btn_h = (toolbar_h - 12.0).max(18.0);

        for (label, target) in toolbar_buttons {
            let btn_w = (font.text_width(label) + 16.0).max(58.0);
            if btn_x + btn_w > bounds.max_x() - 10.0 {
                break;
            }
            let rect = Rect::from_xywh(btn_x, btn_y, btn_w, btn_h);
            scene.push_quad(
                Quad::new(rect)
                    .with_background(theme.tab_inactive_bg)
                    .with_border(theme.panel_border, 1.0)
                    .with_corner_radius(3.0),
            );
            let text_y = btn_y + (btn_h - font.line_height) / 2.0;
            scene.push_text(GlyphRun::new(
                label.to_string(),
                Point::new(btn_x + 8.0, text_y),
                rect,
                theme.editor_foreground,
                font.font_size * 0.9,
            ));
            if let Some(clipped) = rect.intersect(&toolbar_rect) {
                targets.push((clipped, *target));
            }
            btn_x += btn_w + 6.0;
        }

        // Right-aligned kernel label
        let kernel_label = format!("{} (#{})", self.kernel.label, self.kernel.execution_counter);
        let kernel_w = font.text_width(&kernel_label) + 12.0;
        if bounds.max_x() - kernel_w > btn_x + 8.0 {
            let kx = bounds.max_x() - kernel_w - 8.0;
            let ky = bounds.min_y() + (toolbar_h - font.line_height) / 2.0;
            scene.push_text(GlyphRun::new(
                kernel_label,
                Point::new(kx, ky),
                Rect::from_xywh(kx, ky, kernel_w, font.line_height),
                theme.editor_line_number,
                font.font_size * 0.85,
            ));
        }

        // 2. Scrollable Cell Body
        let body_rect = Rect::from_xywh(
            bounds.min_x(),
            bounds.min_y() + toolbar_h,
            bounds.width(),
            (bounds.height() - toolbar_h).max(0.0),
        );

        let line_h = font.line_height.max(16.0);
        let cell_gap = 10.0f32;
        let pad_x = 12.0f32;
        let pad_y = 10.0f32;

        let mut total_h = pad_y;
        let mut cell_heights = Vec::with_capacity(self.document.cells.len());
        for cell in &self.document.cells {
            let h = self.measure_cell_height(cell, font);
            cell_heights.push(h);
            total_h += h + cell_gap;
        }
        total_h += pad_y;

        *self.last_content_height.write() = total_h;
        *self.last_viewport_height.write() = body_rect.height();

        let max_scroll = (total_h - body_rect.height()).max(0.0);
        let max_scroll = if max_scroll.is_finite() { max_scroll } else { 0.0 };
        let safe_scroll = if self.scroll_top.is_finite() {
            self.scroll_top
        } else {
            0.0
        };
        let clamped_scroll = safe_scroll.clamp(0.0, max_scroll);

        scene.push_clip(body_rect);

        let lang_id = self.document.language();
        let raw_grammar = self.grammar_registry.get_by_id("plaintext").unwrap();
        let code_grammar = self
            .grammar_registry
            .get_by_id(lang_id)
            .unwrap_or_else(|| raw_grammar.clone());
        let md_grammar = self
            .grammar_registry
            .get_by_id("markdown")
            .unwrap_or_else(|| raw_grammar.clone());

        let mut cur_y = body_rect.min_y() + pad_y - clamped_scroll;
        let card_x = body_rect.min_x() + pad_x;
        let card_w = (body_rect.width() - pad_x * 2.0).max(60.0);

        for (idx, cell) in self.document.cells.iter().enumerate() {
            let card_h = cell_heights[idx];
            let card_rect = Rect::from_xywh(card_x, cur_y, card_w, card_h);

            // Skip cells completely outside viewport
            if card_rect.max_y() >= body_rect.min_y() && card_rect.min_y() <= body_rect.max_y() {
                let is_active = self.active_cell_index == Some(idx);
                let border_color = if is_active {
                    theme.activity_bar_active_border
                } else {
                    theme.panel_border
                };

                // Card background + border
                scene.push_quad(
                    Quad::new(card_rect)
                        .with_background(theme.tab_inactive_bg)
                        .with_border(border_color, if is_active { 1.5 } else { 1.0 })
                        .with_corner_radius(4.0),
                );

                // Register CellBody BEFORE per-cell buttons (and scan reverse on MouseDown)
                if let Some(clipped_card) = card_rect.intersect(&body_rect) {
                    targets.push((clipped_card, NotebookClickTarget::CellBody(idx)));
                }

                // Header row (26px)
                let header_h = 26.0f32;
                let header_rect = Rect::from_xywh(card_x, cur_y, card_w, header_h);
                scene.push_quad(
                    Quad::new(header_rect)
                        .with_background(theme.panel_bg)
                        .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
                );

                let badge_text = match cell.kind {
                    CellKind::Code => {
                        if cell.is_executing {
                            "[*]".to_string()
                        } else if let Some(n) = cell.execution_count {
                            format!("[{}]", n)
                        } else {
                            "[ ]".to_string()
                        }
                    }
                    CellKind::Markdown => "[MD]".to_string(),
                    CellKind::Raw => "[RAW]".to_string(),
                };

                let header_text_y = cur_y + (header_h - font.line_height) / 2.0;
                scene.push_text(GlyphRun::new(
                    badge_text,
                    Point::new(card_x + 8.0, header_text_y),
                    Rect::from_xywh(card_x + 8.0, header_text_y, 50.0, font.line_height),
                    if is_active {
                        theme.editor_line_number_active
                    } else {
                        theme.editor_line_number
                    },
                    font.font_size * 0.9,
                ));

                let kind_label = match cell.kind {
                    CellKind::Code => "Code",
                    CellKind::Markdown => "Markdown",
                    CellKind::Raw => "Raw",
                };
                scene.push_text(GlyphRun::new(
                    kind_label.to_string(),
                    Point::new(card_x + 56.0, header_text_y),
                    Rect::from_xywh(card_x + 56.0, header_text_y, 80.0, font.line_height),
                    theme.editor_line_number,
                    font.font_size * 0.85,
                ));

                // Per-cell action buttons on right side of header
                let cell_buttons: &[(&str, NotebookClickTarget)] = &[
                    ("Run", NotebookClickTarget::RunCell(idx)),
                    ("Up", NotebookClickTarget::MoveCellUp(idx)),
                    ("Down", NotebookClickTarget::MoveCellDown(idx)),
                    ("Delete", NotebookClickTarget::DeleteCell(idx)),
                ];
                let mut bx = card_rect.max_x() - 8.0;
                let by = cur_y + 3.0;
                let bh = (header_h - 6.0).max(16.0);
                for (lbl, tgt) in cell_buttons.iter().rev() {
                    let bw = (font.text_width(lbl) + 10.0).max(32.0);
                    bx -= bw;
                    if bx <= card_x + 140.0 {
                        break;
                    }
                    let brect = Rect::from_xywh(bx, by, bw, bh);
                    scene.push_quad(
                        Quad::new(brect)
                            .with_background(theme.tab_active_bg)
                            .with_border(theme.panel_border, 1.0)
                            .with_corner_radius(2.0),
                    );
                    scene.push_text(GlyphRun::new(
                        (*lbl).to_string(),
                        Point::new(bx + 5.0, by + (bh - font.line_height) / 2.0),
                        brect,
                        theme.editor_foreground,
                        font.font_size * 0.8,
                    ));
                    if let Some(clipped) = brect.intersect(&body_rect) {
                        targets.push((clipped, *tgt));
                    }
                    bx -= 4.0;
                }

                // Source area
                let src_lines = Self::cell_source_lines(&cell.source);
                let src_box_h = src_lines.len().max(1) as f32 * line_h + 12.0;
                let src_rect = Rect::from_xywh(
                    card_x + 8.0,
                    cur_y + header_h + 4.0,
                    (card_w - 16.0).max(20.0),
                    src_box_h,
                );
                scene.push_quad(
                    Quad::new(src_rect)
                        .with_background(theme.editor_background)
                        .with_border(theme.panel_border, 1.0)
                        .with_corner_radius(3.0),
                );

                let grammar = match cell.kind {
                    CellKind::Code => &code_grammar,
                    CellKind::Markdown => &md_grammar,
                    CellKind::Raw => &raw_grammar,
                };
                let mut grammar_state: Option<GrammarState> = None;

                for (line_idx, line) in src_lines.iter().enumerate() {
                    let ly = src_rect.min_y() + 6.0 + line_idx as f32 * line_h;
                    let lx = src_rect.min_x() + 8.0;
                    let (spans, next_state) =
                        grammar.tokenize_line(line, grammar_state.as_ref());
                    grammar_state = next_state;

                    if spans.is_empty() {
                        scene.push_text(GlyphRun::new(
                            (*line).to_string(),
                            Point::new(lx, ly),
                            Rect::from_xywh(lx, ly, (src_rect.width() - 16.0).max(10.0), line_h),
                            theme.editor_foreground,
                            font.font_size,
                        ));
                    } else {
                        let mut last_byte = 0usize;
                        let mut cur_tx = lx;
                        for span in &spans {
                            let start = span.start.min(line.len());
                            let end = span.end.min(line.len());
                            if !line.is_char_boundary(start) || !line.is_char_boundary(end) {
                                continue;
                            }
                            if start > last_byte && line.is_char_boundary(last_byte) {
                                let gap = &line[last_byte..start];
                                let w = font.text_width(gap);
                                scene.push_text(GlyphRun::new(
                                    gap.to_string(),
                                    Point::new(cur_tx, ly),
                                    Rect::from_xywh(cur_tx, ly, w.max(1.0), line_h),
                                    theme.editor_foreground,
                                    font.font_size,
                                ));
                                cur_tx += w;
                            }
                            if end > start {
                                let tok_str = &line[start..end];
                                let w = font.text_width(tok_str);
                                let color = SyntaxHighlighter::resolve_color(&self.theme, span.scope);
                                let mut run = GlyphRun::new(
                                    tok_str.to_string(),
                                    Point::new(cur_tx, ly),
                                    Rect::from_xywh(cur_tx, ly, w.max(1.0), line_h),
                                    color,
                                    font.font_size,
                                );
                                run = run.with_style(
                                    span.metadata.is_bold(),
                                    span.metadata.is_italic(),
                                    span.metadata.is_underline(),
                                );
                                scene.push_text(run);
                                cur_tx += w;
                            }
                            last_byte = end;
                        }
                        if last_byte < line.len() && line.is_char_boundary(last_byte) {
                            let tail = &line[last_byte..];
                            let w = font.text_width(tail);
                            scene.push_text(GlyphRun::new(
                                tail.to_string(),
                                Point::new(cur_tx, ly),
                                Rect::from_xywh(cur_tx, ly, w.max(1.0), line_h),
                                theme.editor_foreground,
                                font.font_size,
                            ));
                        }
                    }

                    // Cursor bar at end of last line when editing this cell
                    if is_active && self.editing_cell && line_idx + 1 == src_lines.len() {
                        let cursor_x = lx + font.text_width(line);
                        scene.push_quad(
                            Quad::new(Rect::from_xywh(cursor_x, ly + 1.0, 2.0, line_h - 2.0))
                                .with_background(theme.editor_cursor),
                        );
                    }
                }

                // Output area
                let out_lines = Self::cell_output_lines(cell);
                if !out_lines.is_empty() {
                    let out_y = src_rect.max_y() + 6.0;
                    let out_h = out_lines.len() as f32 * line_h + 12.0;
                    let out_rect = Rect::from_xywh(
                        card_x + 8.0,
                        out_y,
                        (card_w - 16.0).max(20.0),
                        out_h,
                    );
                    scene.push_quad(
                        Quad::new(out_rect)
                            .with_background(theme.panel_bg)
                            .with_border_edges(
                                theme.activity_bar_active_border,
                                Edges::new(0.0, 0.0, 0.0, 2.0),
                            ),
                    );
                    for (oidx, (otext, is_err)) in out_lines.iter().enumerate() {
                        let oy = out_rect.min_y() + 6.0 + oidx as f32 * line_h;
                        let color = if *is_err {
                            theme.editor_error_foreground
                        } else {
                            theme.editor_foreground
                        };
                        scene.push_text(GlyphRun::new(
                            otext.clone(),
                            Point::new(out_rect.min_x() + 10.0, oy),
                            Rect::from_xywh(
                                out_rect.min_x() + 10.0,
                                oy,
                                (out_rect.width() - 16.0).max(10.0),
                                line_h,
                            ),
                            color,
                            font.font_size * 0.95,
                        ));
                    }
                }
            }

            cur_y += card_h + cell_gap;
        }

        scene.pop_clip();
        *self.hit_targets.write() = targets;
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::MouseDown(mouse) => {
                if mouse.button != MouseButton::Left || !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }
                let hit = {
                    let targets = self.hit_targets.read();
                    targets
                        .iter()
                        .rev()
                        .find(|(rect, _)| rect.contains(mouse.position))
                        .map(|(_, target)| *target)
                };
                let Some(target) = hit else {
                    return EventResult::Ignored;
                };
                match target {
                    NotebookClickTarget::RunAll => self.execute_all_cells(),
                    NotebookClickTarget::AddCode => {
                        self.add_code_cell();
                    }
                    NotebookClickTarget::AddMarkdown => {
                        self.add_markdown_cell();
                    }
                    NotebookClickTarget::ClearOutputs => self.clear_all_outputs(),
                    NotebookClickTarget::RestartKernel => self.restart_kernel(),
                    NotebookClickTarget::RunCell(idx) => {
                        self.active_cell_index = Some(idx);
                        self.execute_cell(idx);
                    }
                    NotebookClickTarget::MoveCellUp(idx) => self.move_cell_up(idx),
                    NotebookClickTarget::MoveCellDown(idx) => self.move_cell_down(idx),
                    NotebookClickTarget::DeleteCell(idx) => self.delete_cell(idx),
                    NotebookClickTarget::CellBody(idx) => {
                        self.active_cell_index = Some(idx);
                        self.editing_cell = true;
                    }
                }
                EventResult::Consumed
            }
            InputEvent::ScrollWheel(scroll) => {
                if !bounds.is_finite_positive()
                    || !scroll.position.x.is_finite()
                    || !scroll.position.y.is_finite()
                    || !bounds.contains(scroll.position)
                {
                    return EventResult::Ignored;
                }
                let raw_total_h = *self.last_content_height.read();
                let total_h = if raw_total_h.is_finite() {
                    raw_total_h.max(0.0)
                } else {
                    0.0
                };
                let raw_vp = *self.last_viewport_height.read();
                let safe_vp = if raw_vp.is_finite() { raw_vp } else { 0.0 };
                let viewport_h = safe_vp.max((bounds.height() - 36.0).max(0.0));
                let max_scroll = (total_h - viewport_h).max(0.0);
                let max_scroll = if max_scroll.is_finite() { max_scroll } else { 0.0 };
                let safe_scroll = if self.scroll_top.is_finite() {
                    self.scroll_top
                } else {
                    0.0
                };
                if !scroll.delta_y.is_finite() {
                    self.scroll_top = safe_scroll.clamp(0.0, max_scroll);
                    return EventResult::Ignored;
                }
                let raw = safe_scroll - scroll.delta_y;
                self.scroll_top = if raw.is_nan() {
                    0.0
                } else {
                    raw.clamp(0.0, max_scroll)
                };
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => {
                // Shift+Enter: run active cell & advance
                if key.key == KeyCode::Enter && key.modifiers.shift {
                    if let Some(idx) = self.active_cell_index {
                        self.execute_cell(idx);
                        if idx + 1 < self.document.cells.len() {
                            self.active_cell_index = Some(idx + 1);
                        } else {
                            self.editing_cell = false;
                        }
                        return EventResult::Consumed;
                    }
                    return EventResult::Ignored;
                }

                // Ctrl+Enter: run active cell in place
                if key.key == KeyCode::Enter && key.modifiers.ctrl {
                    if let Some(idx) = self.active_cell_index {
                        self.execute_cell(idx);
                        return EventResult::Consumed;
                    }
                    return EventResult::Ignored;
                }

                if self.editing_cell {
                    let Some(idx) = self.active_cell_index else {
                        return EventResult::Ignored;
                    };
                    match key.key {
                        KeyCode::Escape => {
                            self.editing_cell = false;
                            return EventResult::Consumed;
                        }
                        KeyCode::Backspace => {
                            if let Some(cell) = self.document.cells.get_mut(idx) {
                                if cell.source.pop().is_some() {
                                    self.document.is_dirty = true;
                                    self.last_action = Some(NotebookAction::DocumentChanged);
                                }
                            }
                            return EventResult::Consumed;
                        }
                        KeyCode::Enter => {
                            if let Some(cell) = self.document.cells.get_mut(idx) {
                                cell.source.push('\n');
                                self.document.is_dirty = true;
                                self.last_action = Some(NotebookAction::DocumentChanged);
                            }
                            return EventResult::Consumed;
                        }
                        _ => {
                            if let Some(ch) = Self::key_event_to_char(key) {
                                if let Some(cell) = self.document.cells.get_mut(idx) {
                                    cell.source.push(ch);
                                    self.document.is_dirty = true;
                                    self.last_action = Some(NotebookAction::DocumentChanged);
                                }
                                return EventResult::Consumed;
                            }
                        }
                    }
                } else {
                    match key.key {
                        KeyCode::ArrowUp => {
                            if let Some(idx) = self.active_cell_index {
                                if idx > 0 {
                                    self.active_cell_index = Some(idx - 1);
                                }
                            } else if !self.document.cells.is_empty() {
                                self.active_cell_index = Some(0);
                            }
                            return EventResult::Consumed;
                        }
                        KeyCode::ArrowDown => {
                            if let Some(idx) = self.active_cell_index {
                                if idx + 1 < self.document.cells.len() {
                                    self.active_cell_index = Some(idx + 1);
                                }
                            } else if !self.document.cells.is_empty() {
                                self.active_cell_index = Some(0);
                            }
                            return EventResult::Consumed;
                        }
                        KeyCode::Enter if self.active_cell_index.is_some() => {
                            self.editing_cell = true;
                            return EventResult::Consumed;
                        }
                        _ => {}
                    }
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{MouseEvent, ScrollEvent};

    #[test]
    fn test_ipynb_parse_and_roundtrip_string_and_array_sources() {
        let raw_ipynb = r##"{
            "cells": [
                {
                    "cell_type": "markdown",
                    "metadata": {},
                    "source": ["# Title\n", "Description line"]
                },
                {
                    "id": "cell-code-1",
                    "cell_type": "code",
                    "execution_count": 2,
                    "metadata": {},
                    "outputs": [
                        {
                            "output_type": "stream",
                            "name": "stdout",
                            "text": ["hello\n", "world"]
                        },
                        {
                            "output_type": "execute_result",
                            "execution_count": 2,
                            "data": { "text/plain": ["42"] },
                            "metadata": {}
                        },
                        {
                            "output_type": "display_data",
                            "data": { "text/plain": "chart" },
                            "metadata": {}
                        },
                        {
                            "output_type": "error",
                            "ename": "ZeroDivisionError",
                            "evalue": "division by zero",
                            "traceback": ["line 1"]
                        }
                    ],
                    "source": "x = 40 + 2\nprint('hello')"
                }
            ],
            "metadata": {
                "kernelspec": { "name": "python3", "language": "python", "display_name": "Python 3" },
                "language_info": { "name": "python" }
            },
            "nbformat": 4,
            "nbformat_minor": 5
        }"##;

        let doc = NotebookDocument::from_ipynb_json("file:///tmp/demo.ipynb", raw_ipynb).unwrap();
        assert!(!doc.is_dirty);
        assert_eq!(doc.path, Some(PathBuf::from("/tmp/demo.ipynb")));
        assert_eq!(doc.language(), "python");
        assert_eq!(doc.cells.len(), 2);
        assert_eq!(doc.cells[0].kind, CellKind::Markdown);
        assert!(!doc.cells[0].id.is_empty());
        assert_eq!(doc.cells[0].source, "# Title\nDescription line");

        assert_eq!(doc.cells[1].kind, CellKind::Code);
        assert_eq!(doc.cells[1].id, "cell-code-1");
        assert_eq!(doc.cells[1].execution_count, Some(2));
        assert_eq!(doc.cells[1].source, "x = 40 + 2\nprint('hello')");
        assert_eq!(doc.cells[1].outputs.len(), 4);
        assert_eq!(doc.cells[1].outputs[0].plain_text(), "hello\nworld");
        assert_eq!(doc.cells[1].outputs[1].plain_text(), "42");
        assert_eq!(doc.cells[1].outputs[2].plain_text(), "chart");
        assert!(doc.cells[1].outputs[3].plain_text().contains("ZeroDivisionError"));

        let saved_json = doc.to_ipynb_json().unwrap();
        let reparsed = NotebookDocument::from_ipynb_json("file:///tmp/demo.ipynb", &saved_json).unwrap();
        assert_eq!(reparsed.cells.len(), 2);
        assert_eq!(reparsed.cells[0].source, doc.cells[0].source);
        assert_eq!(reparsed.cells[1].source, doc.cells[1].source);
        assert_eq!(reparsed.cells[1].outputs.len(), 4);
    }

    #[test]
    fn test_notebook_document_crud_and_dirty_flags() {
        let mut doc = NotebookDocument::new("untitled://Untitled-1.ipynb");
        assert_eq!(doc.path, None);
        assert!(!doc.is_dirty);
        assert_eq!(doc.cells.len(), 1);

        let md_idx = doc.add_cell(Some(0), CellKind::Markdown, "# Notes");
        assert_eq!(md_idx, 0);
        assert!(doc.is_dirty);
        assert_eq!(doc.cells.len(), 2);

        assert!(doc.update_cell_source(1, "a = 10"));
        assert_eq!(doc.cells[1].source, "a = 10");

        assert!(doc.move_cell_down(0));
        assert_eq!(doc.cells[0].kind, CellKind::Code);
        assert_eq!(doc.cells[1].kind, CellKind::Markdown);

        assert!(doc.move_cell_up(1));
        assert_eq!(doc.cells[0].kind, CellKind::Markdown);

        doc.cells[1].outputs.push(CellOutput::stdout("hi\n"));
        doc.cells[1].execution_count = Some(1);
        assert!(doc.clear_outputs(1));
        assert!(doc.cells[1].outputs.is_empty());
        assert_eq!(doc.cells[1].execution_count, None);

        assert!(doc.delete_cell(0).is_some());
        assert_eq!(doc.cells.len(), 1);
    }

    #[test]
    fn test_notebook_kernel_stateful_expressions_stdout_and_errors() {
        let mut kernel = NotebookKernel::new();

        // Cell 1: assignments, comments, arithmetic, stdout
        let mut c1 = NotebookCell::code(
            "# setup variables\nlet x = 10\nlet mut y = 2.5\nz = x * 2 + 1\nprint(\"sum =\", z)\n",
        );
        let out1 = kernel.execute_cell(&mut c1);
        assert_eq!(c1.execution_count, Some(1));
        assert_eq!(out1.len(), 1);
        assert_eq!(out1[0].plain_text(), "sum = 21\n");

        // Cell 2: stateful expression + string concat + println! + last expr result
        let mut c2 = NotebookCell::code(
            "// read prior variable\nprintln!(\"hi \" + str(x))\n(z + 4) / 5",
        );
        let out2 = kernel.execute_cell(&mut c2);
        assert_eq!(c2.execution_count, Some(2));
        assert_eq!(out2.len(), 2);
        assert_eq!(out2[0].plain_text(), "hi 10\n");
        assert_eq!(out2[1].plain_text(), "5");

        // Cell 3: boolean and logical operators
        let mut c3 = NotebookCell::code("true && !false and (x > 5 || false)");
        let out3 = kernel.execute_cell(&mut c3);
        assert_eq!(out3[0].plain_text(), "true");

        // Cell 4: division by zero error
        let mut c4 = NotebookCell::code("10 / 0");
        let out4 = kernel.execute_cell(&mut c4);
        assert!(matches!(out4[0], CellOutput::Error { ref ename, .. } if ename == "ZeroDivisionError"));

        // Cell 5: undefined variable error
        let mut c5 = NotebookCell::code("missing_var + 1");
        let out5 = kernel.execute_cell(&mut c5);
        assert!(matches!(out5[0], CellOutput::Error { ref ename, .. } if ename == "NameError"));

        // Markdown cell does not increment execution_counter
        let mut md = NotebookCell::markdown("# Header");
        let out_md = kernel.execute_cell(&mut md);
        assert!(out_md.is_empty());
        assert_eq!(kernel.execution_counter, 5);

        // Restart clears variables and resets counter
        kernel.restart();
        assert_eq!(kernel.execution_counter, 0);
        assert!(kernel.variables.is_empty());
        let mut c6 = NotebookCell::code("x");
        let out6 = kernel.execute_cell(&mut c6);
        assert!(matches!(out6[0], CellOutput::Error { ref ename, .. } if ename == "NameError"));
    }

    #[test]
    fn test_notebook_editor_view_render_syntax_buttons_keyboard_and_scroll_clipping() {
        let mut doc = NotebookDocument::new("file:///tmp/view_test.ipynb");
        doc.cells = vec![
            NotebookCell::code("let val = 41\nval + 1"),
            NotebookCell::markdown("# Markdown Section"),
        ];
        let mut view = NotebookEditorView::new(doc);
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("Run All"));
        assert!(scene.contains_text("+ Code"));
        assert!(scene.contains_text("[ ]"));
        assert!(scene.contains_text("let val = 41"));
        assert!(scene.contains_text("val + 1"));

        // Verify syntax highlighting produced distinct colors for keyword vs number
        let let_color = scene
            .glyph_runs()
            .iter()
            .find(|r| r.text == "let")
            .map(|r| r.color);
        let num_color = scene
            .glyph_runs()
            .iter()
            .find(|r| r.text == "41")
            .map(|r| r.color);
        assert!(let_color.is_some() && num_color.is_some());
        assert_ne!(let_color, num_color);

        // Click Run button on cell 0 (must win over CellBody(0))
        let run0_rect = view
            .hit_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                NotebookClickTarget::RunCell(0) => Some(*r),
                _ => None,
            })
            .unwrap();
        let click_res = view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(run0_rect.min_x() + 4.0, run0_rect.min_y() + 4.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(click_res, EventResult::Consumed);
        assert_eq!(view.last_action, Some(NotebookAction::CellExecuted(0)));
        assert_eq!(view.document.cells[0].execution_count, Some(1));
        assert_eq!(view.document.cells[0].outputs[0].plain_text(), "42");

        // Re-render and verify [1] badge & output text
        let mut scene2 = Scene::new();
        view.render(bounds, &theme, &font, &mut scene2);
        assert!(scene2.contains_text("[1]"));
        assert!(scene2.contains_text("42"));

        // Click CellBody(0) to enter edit mode, type Shift+Enter to run & advance
        let body0_rect = view
            .hit_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                NotebookClickTarget::CellBody(0) => Some(*r),
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(body0_rect.min_x() + 10.0, body0_rect.max_y() - 10.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert!(view.editing_cell);
        assert_eq!(view.active_cell_index, Some(0));

        // Append ` + 8` via KeyDown
        for (k, txt) in [
            (KeyCode::Space, None),
            (KeyCode::Char('+'), Some("+")),
            (KeyCode::Space, None),
            (KeyCode::Char('8'), Some("8")),
        ] {
            let mut ev = KeyEvent::new(k, Modifiers::none());
            ev.text = txt.map(|s| s.to_string());
            view.handle_event(&InputEvent::KeyDown(ev), bounds);
        }
        assert!(view.document.cells[0].source.ends_with(" + 8"));
        assert!(view.document.is_dirty);

        // Shift+Enter executes cell 0 and advances focus to cell 1
        let shift_enter = KeyEvent::new(
            KeyCode::Enter,
            Modifiers {
                ctrl: false,
                shift: true,
                alt: false,
                meta: false,
            },
        );
        view.handle_event(&InputEvent::KeyDown(shift_enter), bounds);
        assert_eq!(view.document.cells[0].outputs[0].plain_text(), "50");
        assert_eq!(view.active_cell_index, Some(1));

        // Escape exits edit mode, ArrowUp navigates back to 0
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bounds,
        );
        assert!(!view.editing_cell);
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::ArrowUp, Modifiers::none())),
            bounds,
        );
        assert_eq!(view.active_cell_index, Some(0));

        // Add many cells to test ScrollWheel clamping & clipped hit targets
        for _ in 0..8 {
            view.add_code_cell();
        }
        let small_bounds = Rect::from_xywh(0.0, 0.0, 800.0, 220.0);
        let mut small_scene = Scene::new();
        view.render(small_bounds, &theme, &font, &mut small_scene);
        assert!(*view.last_content_height.read() > 220.0);

        let scroll_res = view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(200.0, 120.0), 0.0, -120.0)),
            small_bounds,
        );
        assert_eq!(scroll_res, EventResult::Consumed);
        assert!(view.scroll_top > 0.0);

        let mut scrolled_scene = Scene::new();
        view.render(small_bounds, &theme, &font, &mut scrolled_scene);
        // Every cell hit target must be inside body_rect (y >= 36.0 and y <= 220.0)
        for (rect, target) in view.hit_targets.read().iter() {
            match target {
                NotebookClickTarget::RunAll
                | NotebookClickTarget::AddCode
                | NotebookClickTarget::AddMarkdown
                | NotebookClickTarget::ClearOutputs
                | NotebookClickTarget::RestartKernel => {
                    assert!(rect.max_y() <= 36.0);
                }
                _ => {
                    assert!(rect.min_y() >= 36.0 && rect.max_y() <= 220.0);
                }
            }
        }
    }

    #[test]
    fn test_notebook_kernel_checked_integer_overflow_and_string_repeat_bounds() {
        let mut kernel = NotebookKernel::new();
        let overflow_exprs = [
            "9223372036854775807 + 1",
            "(-9223372036854775807 - 1) - 1",
            "9223372036854775807 * 2",
            "(-9223372036854775807 - 1) / -1",
            "(-9223372036854775807 - 1) % -1",
            "-(-9223372036854775807 - 1)",
            "abs(-9223372036854775807 - 1)",
            "\"a\" * 9223372036854775807",
            "9223372036854775807 * \"a\"",
        ];

        for expr in overflow_exprs {
            let mut cell = NotebookCell::code(expr);
            let outputs = kernel.execute_cell(&mut cell);
            assert_eq!(outputs.len(), 1, "expected 1 error output for `{}`", expr);
            assert!(
                matches!(&outputs[0], CellOutput::Error { ename, .. } if ename == "OverflowError"),
                "expected OverflowError for `{}`, got {:?}",
                expr,
                outputs[0]
            );
        }

        let mut empty_neg = NotebookCell::code("\"a\" * -5");
        let out_neg = kernel.execute_cell(&mut empty_neg);
        assert_eq!(out_neg[0].plain_text(), "");

        let mut empty_str = NotebookCell::code("\"\" * 9223372036854775807");
        let out_empty = kernel.execute_cell(&mut empty_str);
        assert_eq!(out_empty[0].plain_text(), "");
    }

    #[test]
    fn test_notebook_kernel_mixed_int_float_modulo_and_zero_division() {
        let mut kernel = NotebookKernel::new();

        let mut c1 = NotebookCell::code("5 % 2.0");
        let out1 = kernel.execute_cell(&mut c1);
        assert_eq!(out1[0].plain_text(), "1.0");

        let mut c2 = NotebookCell::code("5.0 % 2");
        let out2 = kernel.execute_cell(&mut c2);
        assert_eq!(out2[0].plain_text(), "1.0");

        let mut c3 = NotebookCell::code("5 % 0.0");
        let out3 = kernel.execute_cell(&mut c3);
        assert!(matches!(
            &out3[0],
            CellOutput::Error { ename, .. } if ename == "ZeroDivisionError"
        ));

        let mut c4 = NotebookCell::code("5.0 % 0");
        let out4 = kernel.execute_cell(&mut c4);
        assert!(matches!(
            &out4[0],
            CellOutput::Error { ename, .. } if ename == "ZeroDivisionError"
        ));
    }

    #[test]
    fn test_ipynb_raw_cell_roundtrip_kernel_skip_and_view_render() {
        let raw_ipynb = r##"{
            "cells": [
                {
                    "id": "raw-1",
                    "cell_type": "raw",
                    "metadata": { "format": "text/latex" },
                    "source": ["\\section{Raw}"]
                },
                {
                    "id": "md-1",
                    "cell_type": "markdown",
                    "metadata": {},
                    "source": "# Heading"
                }
            ],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        }"##;

        let mut doc = NotebookDocument::from_ipynb_json("file:///tmp/raw.ipynb", raw_ipynb).unwrap();
        assert_eq!(doc.cells.len(), 2);
        assert_eq!(doc.cells[0].kind, CellKind::Raw);
        assert_eq!(doc.cells[0].source, "\\section{Raw}");

        let saved = doc.to_ipynb_json().unwrap();
        let saved_val: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved_val["cells"][0]["cell_type"], "raw");
        assert!(saved_val["cells"][0].get("outputs").is_none());
        assert!(saved_val["cells"][0].get("execution_count").is_none());

        let reparsed = NotebookDocument::from_ipynb_json("file:///tmp/raw.ipynb", &saved).unwrap();
        assert_eq!(reparsed.cells[0].kind, CellKind::Raw);
        assert_eq!(reparsed.cells[0].source, "\\section{Raw}");

        // Kernel skips raw cells and execute_all does not mark dirty if no code cells executed
        let mut kernel = NotebookKernel::new();
        assert!(kernel.execute_cell(&mut doc.cells[0]).is_empty());
        assert_eq!(kernel.execution_counter, 0);
        assert_eq!(doc.cells[0].execution_count, None);

        doc.is_dirty = false;
        kernel.execute_all(&mut doc);
        assert!(!doc.is_dirty);
        assert_eq!(kernel.execution_counter, 0);

        // NotebookEditorView renders [RAW] badge and Raw label
        let view = NotebookEditorView::new(doc);
        let mut scene = Scene::new();
        view.render(
            Rect::from_xywh(0.0, 0.0, 800.0, 400.0),
            &WorkbenchThemeStyles::dark_default(),
            &FontMetrics::default_monospace(),
            &mut scene,
        );
        assert!(scene.contains_text("[RAW]"));
        assert!(scene.contains_text("Raw"));
    }

    #[test]
    fn test_notebook_view_non_finite_bounds_and_nan_scrollwheel_no_poison() {
        use rustcode_platform::render::{ScrollEvent, Size};

        let mut doc = NotebookDocument::new("file:///tmp/test.ipynb");
        doc.add_cell(None, CellKind::Code, "let x = 1;");
        let mut view = NotebookEditorView::new(doc);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for bad_bounds in [
            Rect::new(Point::zero(), Size::new(800.0, f32::NAN)),
            Rect::new(Point::new(f32::NAN, f32::NAN), Size::new(800.0, 600.0)),
            Rect::from_xywh(0.0, 0.0, f32::INFINITY, 600.0),
        ] {
            let mut scene = Scene::new();
            view.render(bad_bounds, &theme, &font, &mut scene);
            assert!(scene.nodes().is_empty());
        }

        let valid_bounds = Rect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let mut init_scene = Scene::new();
        view.render(valid_bounds, &theme, &font, &mut init_scene);
        assert!(init_scene.contains_text("Code"));

        view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(200.0, 200.0), 0.0, f32::NAN)),
            valid_bounds,
        );
        assert!(view.scroll_top.is_finite());

        let mut after_scene = Scene::new();
        view.render(valid_bounds, &theme, &font, &mut after_scene);
        assert!(after_scene.contains_text("Code"));
    }
}
