//! Diff engine and display lists.

pub mod myers;
pub mod view;

pub use myers::{
    myers_diff, DiffCell, DiffChange, DiffModel, DiffOpKind, DiffViewMode, EditOp, InlineDiffRow,
    SideBySideDiffRow,
};
pub use view::DiffEditorView;
