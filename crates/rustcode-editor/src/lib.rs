//! `rustcode-editor` — Layer 2: Text Editing Engine.
//!
//! Provides the core text buffer backed by a high-performance Piece Table,
//! transactional undo/redo history, and cursor/selection management.

pub mod completion;
pub mod cursor;
pub mod diff;
pub mod document;
pub mod find_replace;
pub mod folding;
pub mod model;
pub mod syntax;
pub mod view;
pub mod viewmodel;

pub use completion::{
    expand_snippet, fuzzy_match, AutocompleteWidget, CompletionItem, CompletionItemKind,
    CompletionList,
};
pub use cursor::{CursorState, Position, Range, RangeEdit, Selection};
pub use diff::{
    myers_diff, DiffCell, DiffChange, DiffEditorView, DiffModel, DiffOpKind, DiffViewMode, EditOp,
    InlineDiffRow, SideBySideDiffRow,
};
pub use document::{AutoSaveConfig, AutoSaveMode, AutoSaveService, TextDocument};
pub use find_replace::{
    FindMatch, FindReplaceClickTarget, FindReplaceFocus, FindReplaceLayout, FindReplaceWidget,
};
pub use folding::{FoldingModel, FoldingRange, FoldingRangeKind};
pub use model::{
    byte_offset_to_utf16_col, utf16_col_to_byte_offset, Piece, PieceSource, PieceTable, TextEdit,
    UndoElement, UndoRedoStack,
};
pub use syntax::{
    CGrammar, CppGrammar, CssGrammar, GoGrammar, Grammar, GrammarRegistry, GrammarState,
    HtmlGrammar, JavaScriptGrammar, JsonGrammar, MarkdownGrammar, PlainTextGrammar,
    PythonGrammar, RustGrammar, SyntaxHighlighter, SyntaxScope, TokenMetadata, TokenSpan,
    TomlGrammar, TypeScriptGrammar, YamlGrammar,
};
pub use view::{EditorGutterAction, EditorView, MinimapState, WordWrapMode};
pub use viewmodel::{
    DecorationKind, DecorationModel, DiagnosticSeverity, EditorDiagnostic, LineDecoration,
};

