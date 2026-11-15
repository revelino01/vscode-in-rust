//! `rustcode-editor` — Layer 2: Text Editing Engine.
//!
//! Provides the core text buffer backed by a high-performance Piece Table,
//! transactional undo/redo history, and cursor/selection management.

pub mod cursor;
pub mod model;
pub mod viewmodel;

pub use cursor::{CursorState, Position, Range, Selection};
pub use model::{Piece, PieceSource, PieceTable, TextEdit, UndoElement, UndoRedoStack};
pub use viewmodel::{DecorationKind, DecorationModel, LineDecoration};
