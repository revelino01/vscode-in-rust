//! Undo / Redo stack with composite edit grouping.

use crate::model::piece_table::PieceTable;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub offset: usize,
    pub deleted_text: String,
    pub inserted_text: String,
}

impl TextEdit {
    pub fn insert(offset: usize, inserted_text: &str) -> Self {
        Self {
            offset,
            deleted_text: String::new(),
            inserted_text: inserted_text.to_string(),
        }
    }

    pub fn delete(offset: usize, deleted_text: &str) -> Self {
        Self {
            offset,
            deleted_text: deleted_text.to_string(),
            inserted_text: String::new(),
        }
    }

    pub fn replace(offset: usize, deleted_text: &str, inserted_text: &str) -> Self {
        Self {
            offset,
            deleted_text: deleted_text.to_string(),
            inserted_text: inserted_text.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum UndoElement {
    Single(TextEdit),
    Group(Vec<TextEdit>),
}

#[derive(Default)]
pub struct UndoRedoStack {
    past: Vec<UndoElement>,
    future: Vec<UndoElement>,
    grouping: Option<Vec<TextEdit>>,
}

impl UndoRedoStack {
    pub fn new() -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            grouping: None,
        }
    }

    pub fn begin_group(&mut self) {
        self.grouping = Some(Vec::new());
    }

    pub fn end_group(&mut self) {
        if let Some(group) = self.grouping.take() {
            if !group.is_empty() {
                self.past.push(UndoElement::Group(group));
                self.future.clear();
            }
        }
    }

    pub fn push_edit(&mut self, edit: TextEdit) {
        if let Some(ref mut group) = self.grouping {
            group.push(edit);
        } else {
            self.past.push(UndoElement::Single(edit));
            self.future.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn undo(&mut self, table: &mut PieceTable) -> Option<usize> {
        let element = self.past.pop()?;
        let target_offset = match &element {
            UndoElement::Single(edit) => Self::revert_edit(table, edit),
            UndoElement::Group(edits) => {
                let mut last_off = None;
                for edit in edits.iter().rev() {
                    last_off = Some(Self::revert_edit(table, edit));
                }
                last_off.unwrap_or(0)
            }
        };

        self.future.push(element);
        Some(target_offset)
    }

    pub fn redo(&mut self, table: &mut PieceTable) -> Option<usize> {
        let element = self.future.pop()?;
        let target_offset = match &element {
            UndoElement::Single(edit) => Self::apply_edit(table, edit),
            UndoElement::Group(edits) => {
                let mut last_off = None;
                for edit in edits {
                    last_off = Some(Self::apply_edit(table, edit));
                }
                last_off.unwrap_or(0)
            }
        };

        self.past.push(element);
        Some(target_offset)
    }

    fn revert_edit(table: &mut PieceTable, edit: &TextEdit) -> usize {
        if !edit.inserted_text.is_empty() {
            table.delete(edit.offset, edit.inserted_text.len());
        }
        if !edit.deleted_text.is_empty() {
            table.insert(edit.offset, &edit.deleted_text);
            edit.offset + edit.deleted_text.len()
        } else {
            edit.offset
        }
    }

    fn apply_edit(table: &mut PieceTable, edit: &TextEdit) -> usize {
        if !edit.deleted_text.is_empty() {
            table.delete(edit.offset, edit.deleted_text.len());
        }
        if !edit.inserted_text.is_empty() {
            table.insert(edit.offset, &edit.inserted_text);
            edit.offset + edit.inserted_text.len()
        } else {
            edit.offset
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_undo_redo() {
        let mut table = PieceTable::from_str("Hello");
        let mut stack = UndoRedoStack::new();

        // Perform edit: insert " World" at offset 5
        table.insert(5, " World");
        stack.push_edit(TextEdit::insert(5, " World"));
        assert_eq!(table.to_string(), "Hello World");

        // Undo
        assert!(stack.can_undo());
        stack.undo(&mut table);
        assert_eq!(table.to_string(), "Hello");

        // Redo
        assert!(stack.can_redo());
        stack.redo(&mut table);
        assert_eq!(table.to_string(), "Hello World");
    }
}
