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
pub mod debug;
pub mod editor_area;
pub mod explorer;
pub mod extensions_view;
pub mod layout;
pub mod notebook;
pub mod output_panel;
pub mod problems;
pub mod quickinput;
pub mod scm;
pub mod search;
pub mod settings_ui;
pub mod statusbar;
pub mod terminal_panel;
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
pub use editor_area::{EditorArea, EditorAreaAction, EditorAreaView, EditorGroup, EditorTab};
pub use explorer::{ExplorerAction, ExplorerRow, FileExplorer, FileExplorerView, FileTreeNode};
pub use extensions_view::{
    curated_marketplace_extensions, ExtensionsClickTarget, ExtensionsSidebarLayout,
    ExtensionsSidebarView, ExtensionsViewAction, InstalledExtensionItem,
    MarketplaceExtensionItem,
};
pub use layout::WorkbenchLayout;
pub use notebook::{
    CellKind, CellOutput, KernelValue, NotebookAction, NotebookCell, NotebookClickTarget,
    NotebookDocument, NotebookEditorView, NotebookKernel,
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
pub use webview::{ParsedWebviewNode, WebviewEditorView, WebviewElementKind};
pub use window::WorkbenchWindow;
pub use workbench_view::{FocusedRegion, WorkbenchAction, WorkbenchRegions, WorkbenchView};
pub use workspace::{Workspace, WorkspaceFolder};

pub(crate) fn is_finite_rect(rect: rustcode_platform::render::Rect) -> bool {
    const MAX_RECT_COORD: f32 = 1.0e15;
    rect.min_x().is_finite()
        && rect.min_y().is_finite()
        && rect.width().is_finite()
        && rect.height().is_finite()
        && rect.max_x().is_finite()
        && rect.max_y().is_finite()
        && rect.origin.x.abs() <= MAX_RECT_COORD
        && rect.origin.y.abs() <= MAX_RECT_COORD
        && rect.size.width.abs() <= MAX_RECT_COORD
        && rect.size.height.abs() <= MAX_RECT_COORD
        && (rect.origin.x as f64 + rect.size.width as f64).abs() <= f32::MAX as f64
        && (rect.origin.y as f64 + rect.size.height as f64).abs() <= f32::MAX as f64
}

pub(crate) fn is_finite_positive_rect(rect: rustcode_platform::render::Rect) -> bool {
    is_finite_rect(rect) && rect.width() > 0.0 && rect.height() > 0.0
}

pub(crate) fn is_finite_font(font: &rustcode_platform::render::FontMetrics) -> bool {
    const MAX_FONT_METRIC: f32 = 10_000.0;
    font.line_height.is_finite()
        && font.line_height > 0.0
        && font.line_height <= MAX_FONT_METRIC
        && font.char_width.is_finite()
        && font.char_width >= 0.0
        && font.char_width <= MAX_FONT_METRIC
        && font.font_size.is_finite()
        && font.font_size >= 0.0
        && font.font_size <= MAX_FONT_METRIC
}

pub(crate) fn push_finite_quad(
    scene: &mut rustcode_platform::render::Scene,
    quad: rustcode_platform::render::Quad,
) {
    if is_finite_rect(quad.bounds) {
        scene.push_quad(quad);
    }
}

pub(crate) fn push_finite_text(
    scene: &mut rustcode_platform::render::Scene,
    run: rustcode_platform::render::GlyphRun,
) {
    if run.origin.x.is_finite()
        && run.origin.y.is_finite()
        && is_finite_rect(run.bounds)
        && run.font_size.is_finite()
        && run.font_size >= 0.0
    {
        scene.push_text(run);
    }
}
