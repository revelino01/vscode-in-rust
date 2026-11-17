//! View Model and line decoration models for rendering.

use crate::cursor::Range;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Information = 3,
    Hint = 4,
}

impl DiagnosticSeverity {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warning => "Warning",
            Self::Information => "Info",
            Self::Hint => "Hint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorDiagnostic {
    pub range: Range,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

impl EditorDiagnostic {
    pub fn new(range: Range, severity: DiagnosticSeverity, message: impl Into<String>) -> Self {
        Self {
            range,
            severity,
            message: message.into(),
            source: None,
            code: None,
        }
    }

    pub fn error(range: Range, message: impl Into<String>) -> Self {
        Self::new(range, DiagnosticSeverity::Error, message)
    }

    pub fn warning(range: Range, message: impl Into<String>) -> Self {
        Self::new(range, DiagnosticSeverity::Warning, message)
    }

    pub fn info(range: Range, message: impl Into<String>) -> Self {
        Self::new(range, DiagnosticSeverity::Information, message)
    }

    pub fn hint(range: Range, message: impl Into<String>) -> Self {
        Self::new(range, DiagnosticSeverity::Hint, message)
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn formatted_header(&self) -> String {
        match (&self.source, &self.code) {
            (Some(src), Some(code)) => format!("{} ({})", src, code),
            (Some(src), None) => src.clone(),
            (None, Some(code)) => format!("({})", code),
            (None, None) => self.severity.label().to_string(),
        }
    }

    pub fn to_decoration(&self) -> LineDecoration {
        let kind = match self.severity {
            DiagnosticSeverity::Error => DecorationKind::ErrorSquiggle,
            DiagnosticSeverity::Warning => DecorationKind::WarningSquiggle,
            DiagnosticSeverity::Information => DecorationKind::InfoSquiggle,
            DiagnosticSeverity::Hint => DecorationKind::HintSquiggle,
        };
        LineDecoration {
            range: self.range,
            kind,
            message: Some(self.message.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecorationKind {
    CurrentLine,
    SelectionHighlight,
    ErrorSquiggle,
    WarningSquiggle,
    InfoSquiggle,
    HintSquiggle,
    Breakpoint,
}

#[derive(Debug, Clone)]
pub struct LineDecoration {
    pub range: Range,
    pub kind: DecorationKind,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DecorationModel {
    decorations: Vec<LineDecoration>,
}

impl DecorationModel {
    pub fn new() -> Self {
        Self {
            decorations: Vec::new(),
        }
    }

    pub fn add(&mut self, decoration: LineDecoration) {
        self.decorations.push(decoration);
    }

    pub fn clear(&mut self) {
        self.decorations.clear();
    }

    pub fn len(&self) -> usize {
        self.decorations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.decorations.is_empty()
    }

    pub fn all(&self) -> &[LineDecoration] {
        &self.decorations
    }

    pub fn for_line(&self, line: usize) -> Vec<&LineDecoration> {
        self.decorations
            .iter()
            .filter(|d| d.range.start.line <= line && d.range.end.line >= line)
            .collect()
    }
}
