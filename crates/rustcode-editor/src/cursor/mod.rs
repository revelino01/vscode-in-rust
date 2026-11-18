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
pub struct RangeEdit {
    pub range: Range,
    pub new_text: String,
}

impl RangeEdit {
    pub fn new(range: Range, new_text: impl Into<String>) -> Self {
        Self {
            range,
            new_text: new_text.into(),
        }
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

    pub fn start(&self) -> Position {
        self.anchor.min(self.active)
    }

    pub fn end(&self) -> Position {
        self.anchor.max(self.active)
    }

    pub fn is_reversed(&self) -> bool {
        self.active < self.anchor
    }

    pub fn to_range(&self) -> Range {
        Range::new(self.start(), self.end())
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

    pub fn has_multiple_cursors(&self) -> bool {
        self.selections.len() > 1
    }

    /// Normalizes and merges overlapping or duplicate selections while preserving
    /// `selections[0]` as the primary cursor and maintaining insertion order for
    /// non-overlapping selections. Adjacent non-empty selections (`s2 == e1`) are
    /// kept distinct (e.g. `"foofoo"` matched via `Ctrl+D`).
    pub fn normalize_and_merge(&mut self) {
        if self.selections.is_empty() {
            self.selections
                .push(Selection::cursor(Position::new(0, 0)));
            return;
        }
        if self.selections.len() == 1 {
            return;
        }

        let mut tagged: Vec<(usize, Selection)> =
            self.selections.iter().cloned().enumerate().collect();
        tagged.sort_by(|(idx_a, a), (idx_b, b)| {
            a.start()
                .cmp(&b.start())
                .then_with(|| a.end().cmp(&b.end()))
                .then_with(|| idx_a.cmp(idx_b))
        });

        let mut merged: Vec<(usize, Selection)> = Vec::with_capacity(tagged.len());
        for (cur_idx, cur_sel) in tagged {
            if let Some((last_idx, last_sel)) = merged.last_mut() {
                let s1 = last_sel.start();
                let e1 = last_sel.end();
                let s2 = cur_sel.start();
                let e2 = cur_sel.end();

                let overlaps = if last_sel.is_empty() && cur_sel.is_empty() {
                    s1 == s2
                } else if last_sel.is_empty() {
                    s2 <= s1 && s1 <= e2
                } else if cur_sel.is_empty() {
                    s1 <= s2 && s2 <= e1
                } else {
                    // Strict overlap: adjacent non-empty selections (s2 == e1) remain distinct
                    s2 < e1
                };

                if overlaps {
                    let new_start = s1.min(s2);
                    let new_end = e1.max(e2);
                    let reversed = if *last_idx == 0 {
                        last_sel.is_reversed()
                    } else if cur_idx == 0 {
                        cur_sel.is_reversed()
                    } else if !last_sel.is_empty() {
                        last_sel.is_reversed()
                    } else {
                        cur_sel.is_reversed()
                    };
                    *last_idx = (*last_idx).min(cur_idx);
                    *last_sel = if new_start == new_end {
                        Selection::cursor(new_start)
                    } else if reversed {
                        Selection::new(new_end, new_start)
                    } else {
                        Selection::new(new_start, new_end)
                    };
                    continue;
                }
            }
            merged.push((cur_idx, cur_sel));
        }

        merged.sort_by_key(|(idx, _)| *idx);
        self.selections = merged.into_iter().map(|(_, s)| s).collect();
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

    #[test]
    fn test_selection_helpers_and_normalize_merge() {
        let rev = Selection::new(Position::new(2, 8), Position::new(2, 3));
        assert!(rev.is_reversed());
        assert_eq!(rev.start(), Position::new(2, 3));
        assert_eq!(rev.end(), Position::new(2, 8));
        assert_eq!(rev.to_range(), Range::new(Position::new(2, 3), Position::new(2, 8)));

        // Adjacent non-empty selections (e.g. "foofoo") must remain distinct
        let mut state = CursorState::new();
        state.selections = vec![
            Selection::new(Position::new(0, 0), Position::new(0, 3)),
            Selection::new(Position::new(0, 3), Position::new(0, 6)),
        ];
        state.normalize_and_merge();
        assert_eq!(state.selections.len(), 2);

        // Overlapping selections & duplicate empty cursors merge while preserving primary at index 0
        state.selections = vec![
            Selection::new(Position::new(5, 10), Position::new(5, 2)), // primary (reversed)
            Selection::cursor(Position::new(5, 4)),                    // inside primary -> merges
            Selection::cursor(Position::new(1, 0)),                    // distinct earlier cursor
            Selection::cursor(Position::new(1, 0)),                    // duplicate -> merges
            Selection::new(Position::new(5, 8), Position::new(5, 12)), // overlaps primary -> merges
        ];
        state.normalize_and_merge();
        assert_eq!(state.selections.len(), 2);
        assert_eq!(
            state.primary(),
            &Selection::new(Position::new(5, 12), Position::new(5, 2))
        );
        assert_eq!(state.selections[1], Selection::cursor(Position::new(1, 0)));
    }
}
