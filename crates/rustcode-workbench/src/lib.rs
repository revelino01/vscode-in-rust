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
pub mod breadcrumbs;
pub mod context_menu;
pub mod debug;
pub mod editor_area;
pub mod explorer;
pub mod extensions_view;
pub mod hierarchy;
pub mod layout;
pub mod notebook;
pub mod notifications;
pub mod output_panel;
pub mod problems;
pub mod quickinput;
pub mod refactor_preview;
pub mod scm;
pub mod search;
pub mod settings_ui;
pub mod statusbar;
pub mod terminal_panel;
pub mod testing;
pub mod titlebar;
pub mod webview;
pub mod window;
pub mod workbench_view;
pub mod workspace;

pub use activitybar::{ActivityBar, ActivityBarAction, ActivityBarItem, ActivityBarView};
pub use breadcrumbs::{BreadcrumbItem, BreadcrumbKind, BreadcrumbsAction, BreadcrumbsBar};
pub use debug::{
    DebugConsoleView, DebugOutputEntry, DebugSessionState, DebugSidebarView, DebugToolbarAction,
    DebugToolbarView, DebugViewAction,
};
pub use editor_area::{
    EditorArea, EditorAreaAction, EditorAreaView, EditorGridNode, EditorGroup, EditorSash,
    EditorTab, SplitDirection,
};
pub use explorer::{ExplorerAction, ExplorerRow, FileExplorer, FileExplorerView, FileTreeNode};
pub use extensions_view::{
    curated_marketplace_extensions, parse_extension_detail_message, render_extension_detail_html,
    ExtensionDetailContext, ExtensionDetailMessage, ExtensionsClickTarget, ExtensionsSidebarLayout,
    ExtensionsSidebarView, ExtensionsViewAction, InstalledExtensionItem,
    MarketplaceExtensionItem,
};
pub use hierarchy::{FlatHierarchyRow, HierarchyAction, HierarchyItemNode, HierarchyMode, HierarchyView};
pub use layout::WorkbenchLayout;
pub use notebook::{
    CellKind, CellOutput, KernelValue, NotebookAction, NotebookCell, NotebookClickTarget,
    NotebookDocument, NotebookEditorView, NotebookKernel,
};
pub use refactor_preview::{
    FileEditPreview, FileUndoSnapshot, RefactorPreviewAction, RefactorPreviewOverlay,
    WorkspaceEditUndoEntry, WorkspaceEditUndoStack,
};
pub use notifications::{
    ActivePromptKind, ActivePromptUi, NotificationUiAction, NotificationsOverlay, PromptOverlay,
    PromptUiAction,
};
pub use output_panel::{
    OutputAction, OutputChannelData, OutputClickTarget, OutputPanelView, DEFAULT_MAX_SCROLLBACK,
};
pub use problems::{ProblemEntry, ProblemsAction, ProblemsPanel};
pub use quickinput::{CommandPalette, CommandPaletteView, PaletteAction, PaletteItem};
pub use scm::{ScmAction, ScmView};
pub use search::{SearchAction, SearchQuery, SearchResults, SearchView};
pub use settings_ui::{
    SettingsCategory, SettingsFocus, SettingsHitTarget, SettingsItem, SettingsUiAction,
    SettingsUiState, SettingsUiView, SettingsValueType,
};
pub use statusbar::{
    StatusBar, StatusBarAction, StatusBarAlignment, StatusBarItem, StatusBarView,
};
pub use terminal_panel::{TerminalAction, TerminalClickTarget, TerminalPanelView};
pub use testing::{decode_test_uri_to_path, TestingAction, TestingSidebarView, TestingTreeRow};
pub use webview::{ParsedWebviewNode, WebviewEditorView, WebviewElementKind};
pub use window::WorkbenchWindow;
pub use workbench_view::{FocusedRegion, WorkbenchAction, WorkbenchRegions, WorkbenchView};
pub use workspace::{Workspace, WorkspaceFolder};

/// Case-insensitive subsequence fuzzy matcher shared across workbench search and quick input.
#[cfg(test)]
pub(crate) fn fuzzy_match(target: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    let target_lower = target.to_lowercase();
    let pattern_lower = pattern.to_lowercase();
    let mut pattern_chars = pattern_lower.chars().peekable();
    for c in target_lower.chars() {
        if let Some(&pc) = pattern_chars.peek() {
            if c == pc {
                pattern_chars.next();
            }
        } else {
            return true;
        }
    }
    pattern_chars.peek().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_fuzzy_match_case_insensitive_subsequence() {
        assert!(fuzzy_match("src/main.rs", ""));
        assert!(fuzzy_match("CommandPaletteView", "cpv"));
        assert!(fuzzy_match("src/search/symbols.rs", "sss"));
        assert!(!fuzzy_match("src/main.rs", "xyz"));
        assert!(!fuzzy_match("ab", "abc"));
    }
}
