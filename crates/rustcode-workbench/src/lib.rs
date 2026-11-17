//! `rustcode-workbench` — Layer 3: Workbench UI Shell & GPUI Presentation Layer.
//!
//! Provides the core shell state and GPUI component views mirroring VS Code's workbench:
//! - Layout regions (`WorkbenchLayout`, `WorkbenchView`, `WorkbenchRegions`)
//! - Activity bar (`ActivityBar`, `ActivityBarView`, `ActivityBarItem`, `ActivityBarAction`)
//! - File explorer model & view (`FileExplorer`, `FileExplorerView`, `FileTreeNode`, `ExplorerRow`, `ExplorerAction`)
//! - Editor tab, group & area view (`EditorArea`, `EditorAreaView`, `EditorGroup`, `EditorTab`, `EditorAreaAction`)
//! - Command palette with fuzzy search (`CommandPalette`, `CommandPaletteView`, `PaletteItem`, `PaletteAction`)
//! - Status bar management (`StatusBar`, `StatusBarView`, `StatusBarItem`, `StatusBarAlignment`, `StatusBarAction`)

pub mod activitybar;
pub mod debug;
pub mod editor_area;
pub mod explorer;
pub mod layout;
pub mod problems;
pub mod quickinput;
pub mod scm;
pub mod statusbar;
pub mod webview;
pub mod window;
pub mod workbench_view;

pub use activitybar::{ActivityBar, ActivityBarAction, ActivityBarItem, ActivityBarView};
pub use debug::{
    DebugConsoleView, DebugOutputEntry, DebugSessionState, DebugSidebarView, DebugToolbarAction,
    DebugToolbarView, DebugViewAction,
};
pub use editor_area::{EditorArea, EditorAreaAction, EditorAreaView, EditorGroup, EditorTab};
pub use explorer::{ExplorerAction, ExplorerRow, FileExplorer, FileExplorerView, FileTreeNode};
pub use layout::WorkbenchLayout;
pub use problems::{ProblemEntry, ProblemsAction, ProblemsPanel};
pub use quickinput::{CommandPalette, CommandPaletteView, PaletteAction, PaletteItem};
pub use scm::{ScmAction, ScmView};
pub use statusbar::{
    StatusBar, StatusBarAction, StatusBarAlignment, StatusBarItem, StatusBarView,
};
pub use webview::{ParsedWebviewNode, WebviewEditorView, WebviewElementKind};
pub use window::WorkbenchWindow;
pub use workbench_view::{FocusedRegion, WorkbenchAction, WorkbenchRegions, WorkbenchView};
