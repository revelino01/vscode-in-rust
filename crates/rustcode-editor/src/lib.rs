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
pub mod language_config;
pub mod model;
pub mod snippets;
pub mod syntax;
pub mod view;
pub mod viewmodel;

pub const LARGE_FILE_BYTE_THRESHOLD: usize = 4 * 1024 * 1024;
pub const LARGE_FILE_LINE_THRESHOLD: usize = 50_000;

pub use completion::{
    expand_snippet, fuzzy_match, AutocompleteWidget, CompletionItem, CompletionItemKind,
    CompletionList,
};
pub use cursor::{CursorState, Position, Range, RangeEdit, Selection};
pub use diff::{
    myers_diff, DiffCell, DiffChange, DiffEditorView, DiffModel, DiffOpKind, DiffViewMode, EditOp,
    InlineDiffRow, SideBySideDiffRow,
};
pub use document::{AutoSaveConfig, AutoSaveMode, AutoSaveService, TextDocument, TextEncoding};
pub use find_replace::{
    FindMatch, FindReplaceClickTarget, FindReplaceFocus, FindReplaceLayout, FindReplaceWidget,
};
pub use folding::{FoldingModel, FoldingRange, FoldingRangeKind};
pub use language_config::{
    AutoClosingPair, CommentRule, EnterActionSpec, EnterIndentKind, IndentationRule,
    LanguageConfiguration, LanguageConfigurationRegistry, OnEnterRule,
};
pub use model::{
    byte_offset_to_utf16_col, display_col_to_utf16_col, display_x_col_to_utf16_col,
    grapheme_display_width, str_display_width, utf16_col_to_byte_offset, utf16_col_to_display_col,
    Piece, PieceSource, PieceTable, TextEdit, UndoElement, UndoRedoStack,
};
pub use snippets::{
    expand_snippet_with_context, SnippetContext, SnippetDefinition, SnippetNode, SnippetRegistry,
    SnippetSession, SnippetTransform, TabstopGroup, TabstopSite,
};
pub use syntax::{
    load_textmate_contribution, map_semantic_token_to_scope, map_tm_scope_to_syntax_scope,
    overlay_semantic_tokens, CGrammar, CppGrammar, CssGrammar, GoGrammar, Grammar,
    GrammarContribution, GrammarRegistry, GrammarState, HtmlGrammar, JavaScriptGrammar,
    JsonGrammar, MarkdownGrammar, PlainTextGrammar, PythonGrammar, RustGrammar,
    SemanticTokenOverlay, SemanticTokenSpan, SyntaxHighlighter, SyntaxScope, TextMateGrammar,
    TextMateStackFrame, TokenMetadata, TokenSpan, TomlGrammar, TypeScriptGrammar, YamlGrammar,
};
#[cfg(feature = "desktop")]
pub use syntax::{TreeSitterDocument, TreeSitterEngine};
pub use view::{
    hit_test_codelens, parse_markdown_hover, render_inline_annotations,
    render_markdown_hover_popover, ClipboardReadFn, ClipboardWriteFn, CommentConfig,
    ContextMenuAction, ContextMenuItem, EditorAction, EditorCodeLens, EditorContextMenu,
    EditorDocumentLink, EditorGutterAction, EditorInlayHint, EditorView, LspHoverInfo,
    MarkdownBlock, MarkdownSpan, MinimapState, PeekKind, PeekLocationItem, PeekViewWidget,
    RenameInputWidget, ScrollbarAxis, SignatureEntry, SignatureHelpWidget, SignatureParamInfo,
    WordWrapMode,
};
pub use viewmodel::{
    DecorationKind, DecorationModel, DiagnosticSeverity, EditorDiagnostic, LineDecoration,
};

