//! `rustcode-editor` — Layer 2: Text Editing Engine.
//!
//! Provides the core text buffer backed by a high-performance Piece Table,
//! transactional undo/redo history, and cursor/selection management.

pub mod cursor;
pub mod document;
pub mod model;
pub mod syntax;
pub mod view;
pub mod viewmodel;

pub use cursor::{CursorState, Position, Range, Selection};
pub use document::{AutoSaveConfig, AutoSaveMode, AutoSaveService, TextDocument};
pub use model::{Piece, PieceSource, PieceTable, TextEdit, UndoElement, UndoRedoStack};
pub use syntax::{
    Grammar, GrammarRegistry, GrammarState, JavaScriptGrammar, JsonGrammar, MarkdownGrammar,
    PlainTextGrammar, PythonGrammar, RustGrammar, SyntaxHighlighter, SyntaxScope, TokenMetadata,
    TokenSpan, TypeScriptGrammar,
};
pub use view::EditorView;
pub use viewmodel::{DecorationKind, DecorationModel, LineDecoration};

