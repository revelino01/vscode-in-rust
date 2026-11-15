//! View Model and line decoration models for rendering.

use crate::cursor::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecorationKind {
    CurrentLine,
    SelectionHighlight,
    ErrorSquiggle,
    WarningSquiggle,
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

    pub fn for_line(&self, line: usize) -> Vec<&LineDecoration> {
        self.decorations
            .iter()
            .filter(|d| d.range.start.line <= line && d.range.end.line >= line)
            .collect()
    }
}
