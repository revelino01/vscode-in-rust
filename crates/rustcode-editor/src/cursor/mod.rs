//! Cursor, Selection, and Movement state for the editor.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl Position {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub anchor: Position,
    pub active: Position,
}

impl Selection {
    pub fn new(anchor: Position, active: Position) -> Self {
        Self { anchor, active }
    }

    pub fn cursor(pos: Position) -> Self {
        Self {
            anchor: pos,
            active: pos,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.active
    }
}

#[derive(Debug, Clone)]
pub struct CursorState {
    pub selections: Vec<Selection>,
}

impl Default for CursorState {
    fn default() -> Self {
        Self::new()
    }
}

impl CursorState {
    pub fn new() -> Self {
        Self {
            selections: vec![Selection::cursor(Position::new(0, 0))],
        }
    }

    pub fn primary(&self) -> &Selection {
        &self.selections[0]
    }

    pub fn primary_mut(&mut self) -> &mut Selection {
        &mut self.selections[0]
    }

    pub fn set_primary(&mut self, pos: Position) {
        self.selections[0] = Selection::cursor(pos);
    }

    pub fn add_selection(&mut self, selection: Selection) {
        self.selections.push(selection);
    }

    pub fn clear_secondary_cursors(&mut self) {
        if self.selections.len() > 1 {
            self.selections.truncate(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_and_selection() {
        let mut state = CursorState::new();
        assert_eq!(state.primary().active, Position::new(0, 0));

        state.set_primary(Position::new(5, 12));
        assert_eq!(state.primary().active, Position::new(5, 12));

        state.add_selection(Selection::new(Position::new(10, 0), Position::new(10, 4)));
        assert_eq!(state.selections.len(), 2);

        state.clear_secondary_cursors();
        assert_eq!(state.selections.len(), 1);
    }
}
