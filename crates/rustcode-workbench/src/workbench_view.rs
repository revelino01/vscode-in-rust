//! GPUI WorkbenchView coordinator composing all UI shell regions into a cohesive presentation tree.

use crate::activitybar::{ActivityBarAction, ActivityBarView};
use crate::context_menu::{ContextMenuRegistry, MenuLocation, WorkbenchContextMenu};
use crate::debug::{
    DebugConsoleView, DebugSidebarView, DebugToolbarAction, DebugToolbarView, DebugViewAction,
};
use crate::editor_area::EditorAreaView;
use crate::explorer::{ExplorerAction, FileExplorerView};
use crate::extensions_view::{ExtensionsSidebarView, ExtensionsViewAction};
use crate::hierarchy::{HierarchyAction, HierarchyView};
use crate::layout::{PanelPosition, SharedWorkbenchLayout};
use crate::notifications::{
    NotificationUiAction, NotificationsOverlay, PromptOverlay, PromptUiAction,
};
use crate::output_panel::{OutputAction, OutputPanelView};
use crate::problems::{ProblemsAction, ProblemsPanel};
use crate::quickinput::{CommandPaletteView, PaletteAction, QuickInputMode};
use crate::refactor_preview::{RefactorPreviewAction, RefactorPreviewOverlay};
use crate::scm::{ScmAction, ScmView};
use crate::search::symbols::search_document_symbols;
use crate::search::{SearchAction, SearchInputFocus, SearchView};
use crate::settings_ui::{SettingsFocus, SettingsUiView};
use crate::statusbar::{StatusBar, StatusBarAction, StatusBarAlignment, StatusBarItem, StatusBarView};
use crate::terminal_panel::{TerminalAction, TerminalPanelView};
use crate::testing::{decode_test_uri_to_path, TestingAction, TestingSidebarView};
use crate::titlebar::{TitleBarAction, TitleBarView};
use crate::webview::WebviewElementKind;
use crate::workspace::Workspace;
use parking_lot::RwLock;
use rustcode_base::context_key::ContextKeyService;
use rustcode_platform::accessibility::{AccessibilityTree, AccessibleNode, AccessibleRole};
use rustcode_platform::configuration::ConfigurationService;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, MouseButton, Point, Quad, Rect,
    Rgba, Scene, View, WorkbenchThemeStyles,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedRegion {
    Editor,
    Sidebar,
    Panel,
    CommandPalette,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchSash {
    Sidebar,
    SecondarySidebar,
    Panel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkbenchAction {
    ExecuteCommand(String),
    ContextMenuCommand {
        command: String,
        context_path: Option<PathBuf>,
    },
    TitleBarAction(TitleBarAction),
    OpenFile(PathBuf),
    OpenDiff {
        path: PathBuf,
        title: String,
        original: String,
        modified: String,
    },
    JumpToDiagnostic {
        uri: String,
        line: usize,
        column: usize,
    },
    DebugToolbar(DebugToolbarAction),
    DebugAction(DebugViewAction),
    ScmAction(ScmAction),
    ExtensionsAction(ExtensionsViewAction),
    OutputAction(OutputAction),
    TerminalAction(TerminalAction),
    NotificationAction(NotificationUiAction),
    PromptAction(PromptUiAction),
    TestingAction(TestingAction),
    ExplorerAction(ExplorerAction),
    HierarchyAction(HierarchyAction),
    RefactorPreviewAction(RefactorPreviewAction),
    SidebarToggled(bool),
    SecondarySidebarToggled(bool),
    PanelToggled(bool),
}

pub struct WorkbenchView {
    pub layout: SharedWorkbenchLayout,
    pub title_bar: TitleBarView,
    pub activity_bar: ActivityBarView,
    pub explorer: FileExplorerView,
    pub scm_view: ScmView,
    pub search_view: SearchView,
    pub extensions_view: ExtensionsSidebarView,
    pub testing_view: TestingSidebarView,
    pub hierarchy_view: HierarchyView,
    pub workspace: Workspace,
    pub debug_sidebar: DebugSidebarView,
    pub debug_toolbar: DebugToolbarView,
    pub debug_console: DebugConsoleView,
    pub editor_area: EditorAreaView,
    pub status_bar: StatusBarView,
    pub problems_panel: ProblemsPanel,
    pub terminal_panel: TerminalPanelView,
    pub output_panel: OutputPanelView,
    pub panel_tab_bounds: RwLock<Vec<(String, Rect)>>,
    pub command_palette: Option<CommandPaletteView>,
    pub is_command_palette_open: bool,
    pub notifications: NotificationsOverlay,
    pub prompt_overlay: PromptOverlay,
    pub refactor_preview: RefactorPreviewOverlay,
    pub context_menu_registry: ContextMenuRegistry,
    pub context_menu: WorkbenchContextMenu,
    pub active_sash: Option<WorkbenchSash>,
    pub cached_context_keys: Option<ContextKeyService>,
    pub focused_region: FocusedRegion,
    pub last_action: Option<WorkbenchAction>,
    pub pending_settings_open: Arc<AtomicBool>,
    pub(crate) cached_config: Option<ConfigurationService>,
    pub(crate) pending_ext_status_bar: RwLock<Option<Vec<StatusBarItem>>>,
}

impl Default for WorkbenchView {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkbenchView {
    pub fn new() -> Self {
        let mut status_bar = StatusBarView::default();
        status_bar.model.set_item(StatusBarItem {
            id: "workbench.status.problems".to_string(),
            text: "$(error) 0  $(warning) 0".to_string(),
            tooltip: Some("No problems".to_string()),
            command: Some("workbench.action.problems.focus".to_string()),
            alignment: StatusBarAlignment::Left,
            priority: 100,
        });

        Self {
            layout: SharedWorkbenchLayout::new(),
            title_bar: TitleBarView::new(),
            activity_bar: ActivityBarView::default(),
            explorer: FileExplorerView::default(),
            scm_view: ScmView::default(),
            search_view: SearchView::default(),
            extensions_view: ExtensionsSidebarView::default(),
            testing_view: TestingSidebarView::default(),
            hierarchy_view: HierarchyView::new(),
            workspace: Workspace::default(),
            debug_sidebar: DebugSidebarView::default(),
            debug_toolbar: DebugToolbarView::default(),
            debug_console: DebugConsoleView::default(),
            editor_area: EditorAreaView::default(),
            status_bar,
            problems_panel: ProblemsPanel::new(),
            terminal_panel: TerminalPanelView::new(),
            output_panel: OutputPanelView::new(),
            panel_tab_bounds: RwLock::new(Vec::new()),
            command_palette: None,
            is_command_palette_open: false,
            notifications: NotificationsOverlay::new(),
            prompt_overlay: PromptOverlay::new(),
            refactor_preview: RefactorPreviewOverlay::new(),
            context_menu_registry: ContextMenuRegistry::new(),
            context_menu: WorkbenchContextMenu::new(),
            active_sash: None,
            cached_context_keys: None,
            focused_region: FocusedRegion::Editor,
            last_action: None,
            pending_settings_open: Arc::new(AtomicBool::new(false)),
            cached_config: None,
            pending_ext_status_bar: RwLock::new(None),
        }
    }

    fn merged_status_bar(&self) -> Option<StatusBar> {
        let ext_guard = self.pending_ext_status_bar.read();
        let ext_items = ext_guard.as_ref()?;
        let mut bar = self.status_bar.model.clone();
        for item in ext_items {
            bar.set_item(item.clone());
        }
        Some(bar)
    }

    pub fn sync_scm_status_bar(&mut self) {
        let branch = self.scm_view.branch_name().map(str::to_string);
        let dirty = !self.scm_view.repo.staged.is_empty()
            || !self.scm_view.repo.changes.is_empty()
            || !self.scm_view.repo.untracked.is_empty();
        self.status_bar.model.set_git_branch(branch.as_deref(), dirty);
    }

    pub fn open_workspace_git(
        &mut self,
        workspace_root: &std::path::Path,
    ) -> Result<bool, rustcode_protocols::git::GitError> {
        let opened = self.scm_view.open_workspace_git(workspace_root)?;
        self.sync_scm_status_bar();
        Ok(opened)
    }

    pub fn set_git_backend(&mut self, backend: Arc<dyn rustcode_protocols::git::GitBackend>) {
        self.scm_view.set_git_backend(backend);
        self.sync_scm_status_bar();
    }

    pub fn refresh_scm(&mut self) -> Result<(), rustcode_protocols::git::GitError> {
        self.scm_view.refresh_from_git()?;
        self.sync_scm_status_bar();
        Ok(())
    }

    pub fn update_problems(&mut self, uri: &str, diags: Vec<rustcode_editor::viewmodel::EditorDiagnostic>) {
        self.editor_area
            .for_each_editor_for_uri_mut(uri, |editor| editor.set_diagnostics(diags.clone()));
        self.problems_panel.set_file_diagnostics(uri, diags);
        let badge_text = self.problems_panel.status_badge_text();
        let tooltip = format!(
            "{} errors, {} warnings",
            self.problems_panel.total_errors(),
            self.problems_panel.total_warnings()
        );
        self.status_bar.model.set_item(StatusBarItem {
            id: "workbench.status.problems".to_string(),
            text: badge_text,
            tooltip: Some(tooltip),
            command: Some("workbench.action.problems.focus".to_string()),
            alignment: StatusBarAlignment::Left,
            priority: 100,
        });
    }

    pub fn update_lsp_problems(&mut self, uri: &str, diags: &[rustcode_protocols::lsp::types::LspDiagnostic]) {
        let editor_diags: Vec<rustcode_editor::viewmodel::EditorDiagnostic> =
            diags.iter().map(crate::problems::lsp_diagnostic_to_editor).collect();
        self.update_problems(uri, editor_diags);
    }

    pub fn open_command_palette(&mut self, palette_view: CommandPaletteView) {
        self.editor_area.clear_all_drag_state();
        self.command_palette = Some(palette_view);
        self.is_command_palette_open = true;
        self.focused_region = FocusedRegion::CommandPalette;
    }

    pub fn close_command_palette(&mut self) {
        self.is_command_palette_open = false;
        self.focused_region = FocusedRegion::Editor;
    }

    pub fn toggle_sidebar(&mut self) {
        self.layout.toggle_sidebar();
        self.last_action = Some(WorkbenchAction::SidebarToggled(self.layout.sidebar_visible));
    }

    pub fn panel_content_rect(panel_rect: Rect) -> Rect {
        let tab_h = 26.0f32;
        Rect::from_xywh(
            panel_rect.min_x(),
            panel_rect.min_y() + tab_h,
            panel_rect.width(),
            (panel_rect.height() - tab_h).max(0.0),
        )
    }

    pub fn toggle_panel(&mut self) {
        self.layout.toggle_panel();
        if !self.layout.panel_visible && self.focused_region == FocusedRegion::Panel {
            self.focused_region = FocusedRegion::Editor;
        }
        self.last_action = Some(WorkbenchAction::PanelToggled(self.layout.panel_visible));
    }

    pub fn open_settings(&mut self, config: Option<ConfigurationService>) {
        self.pending_settings_open.store(false, Ordering::SeqCst);
        self.editor_area.open_settings(config);
        self.editor_area.sync_breadcrumbs(Some(&self.workspace));
        self.focused_region = FocusedRegion::Editor;
    }

    pub fn open_notebook(&mut self, uri: &str, title: &str, content: &str, is_preview: bool) {
        self.editor_area.open_notebook(uri, title, content, is_preview);
        self.editor_area.sync_breadcrumbs(Some(&self.workspace));
        self.focused_region = FocusedRegion::Editor;
    }

    pub fn new_untitled_notebook(&mut self) {
        let mut n = 1usize;
        let uri = loop {
            let candidate = format!("untitled://Untitled-{}.ipynb", n);
            if !self.editor_area.notebooks.contains_key(&candidate) {
                break candidate;
            }
            n += 1;
        };
        let title = format!("Untitled-{}.ipynb", n);
        self.editor_area.open_notebook(&uri, &title, "", false);
        self.editor_area.sync_breadcrumbs(Some(&self.workspace));
        self.focused_region = FocusedRegion::Editor;
    }

    fn process_extensions_action(&mut self) {
        if let Some(act) = self.extensions_view.last_action.take() {
            self.last_action = Some(WorkbenchAction::ExtensionsAction(act));
        }
    }

    pub fn is_prompt_open(&self) -> bool {
        self.prompt_overlay.is_open()
    }

    pub fn is_explorer_input_active(&self) -> bool {
        self.focused_region == FocusedRegion::Sidebar && self.explorer.is_input_active()
    }

    pub fn has_modal_overlay(&self) -> bool {
        self.is_command_palette_open
            || self.prompt_overlay.is_open()
            || self.notifications.has_modal()
            || self.refactor_preview.is_visible
    }

    fn process_explorer_action(&mut self) {
        if let Some(action) = self.explorer.last_action.take() {
            match action {
                ExplorerAction::OpenFile(path) => {
                    let target_uri = self
                        .editor_area
                        .find_open_tab_uri_for_path(&path)
                        .unwrap_or_else(|| rustcode_platform::path_to_file_uri(&path));
                    let title = path
                        .file_name()
                        .map_or("untitled", |n| n.to_str().unwrap_or(""));
                    let content = if self.editor_area.any_editor_for_uri(&target_uri).is_some() {
                        String::new()
                    } else {
                        std::fs::read_to_string(&path).unwrap_or_default()
                    };
                    self.editor_area.open_file(&target_uri, title, &content, true);
                    self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                    self.last_action = Some(WorkbenchAction::OpenFile(path));
                }
                ExplorerAction::CreateFile(path) => {
                    let target_uri = self
                        .editor_area
                        .find_open_tab_uri_for_path(&path)
                        .unwrap_or_else(|| rustcode_platform::path_to_file_uri(&path));
                    let title = path
                        .file_name()
                        .map_or("untitled", |n| n.to_str().unwrap_or(""));
                    let content = std::fs::read_to_string(&path).unwrap_or_default();
                    self.editor_area.open_file(&target_uri, title, &content, false);
                    self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                    self.last_action =
                        Some(WorkbenchAction::ExplorerAction(ExplorerAction::CreateFile(path)));
                }
                ExplorerAction::Renamed { old_path, new_path } => {
                    self.editor_area.rename_tab(&old_path, &new_path);
                    self.last_action =
                        Some(WorkbenchAction::ExplorerAction(ExplorerAction::Renamed {
                            old_path,
                            new_path,
                        }));
                }
                ExplorerAction::Deleted(path) => {
                    self.editor_area.close_tabs_for_path(&path);
                    self.last_action =
                        Some(WorkbenchAction::ExplorerAction(ExplorerAction::Deleted(path)));
                }
                ExplorerAction::ToggleDir(_) => {}
                other => {
                    self.last_action = Some(WorkbenchAction::ExplorerAction(other));
                }
            }
        }
    }

    fn process_testing_action(&mut self) {
        if let Some(act) = self.testing_view.last_action.take() {
            match act {
                TestingAction::OpenUri(uri) => {
                    if let Some(path) = decode_test_uri_to_path(&uri) {
                        let target_uri = self
                            .editor_area
                            .find_open_tab_uri_for_path(&path)
                            .unwrap_or_else(|| rustcode_platform::path_to_file_uri(&path));
                        let title = path
                            .file_name()
                            .map_or("untitled", |n| n.to_str().unwrap_or(""));
                        let content = if self.editor_area.any_editor_for_uri(&target_uri).is_some()
                        {
                            String::new()
                        } else {
                            std::fs::read_to_string(&path).unwrap_or_default()
                        };
                        self.editor_area.open_file(&target_uri, title, &content, false);
                        self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                        self.focused_region = FocusedRegion::Editor;
                        self.last_action = Some(WorkbenchAction::OpenFile(path));
                    }
                }
                other => {
                    self.last_action = Some(WorkbenchAction::TestingAction(other));
                }
            }
        }
    }

    // --- Layout Region Calculations ---

    pub fn effective_font_metrics(&self, base: &FontMetrics) -> FontMetrics {
        let factor = self.layout.zoom_factor();
        if (factor - 1.0).abs() < 1e-4 {
            return base.clone();
        }
        FontMetrics {
            font_family: base.font_family.clone(),
            font_size: (base.font_size * factor).max(6.0),
            line_height: (base.line_height * factor).max(8.0),
            char_width: (base.char_width * factor).max(4.0),
            ascent: (base.ascent * factor).max(5.0),
            descent: (base.descent * factor).max(1.0),
        }
    }

    fn compute_sashes(&self, regions: &WorkbenchRegions) -> Vec<(WorkbenchSash, Rect)> {
        let mut sashes = Vec::new();
        if self.layout.sidebar_visible && regions.sidebar.width() > 0.0 {
            sashes.push((
                WorkbenchSash::Sidebar,
                Rect::from_xywh(
                    regions.sidebar.max_x() - 2.0,
                    regions.sidebar.min_y(),
                    4.0,
                    regions.sidebar.height(),
                ),
            ));
        }
        if self.layout.secondary_sidebar_visible && regions.secondary_sidebar.width() > 0.0 {
            sashes.push((
                WorkbenchSash::SecondarySidebar,
                Rect::from_xywh(
                    regions.secondary_sidebar.min_x() - 2.0,
                    regions.secondary_sidebar.min_y(),
                    4.0,
                    regions.secondary_sidebar.height(),
                ),
            ));
        }
        if self.layout.panel_visible && regions.panel.width() > 0.0 && regions.panel.height() > 0.0 {
            let rect = match self.layout.panel_position {
                PanelPosition::Bottom => Rect::from_xywh(
                    regions.panel.min_x(),
                    regions.panel.min_y() - 2.0,
                    regions.panel.width(),
                    4.0,
                ),
                PanelPosition::Left => Rect::from_xywh(
                    regions.panel.max_x() - 2.0,
                    regions.panel.min_y(),
                    4.0,
                    regions.panel.height(),
                ),
                PanelPosition::Right => Rect::from_xywh(
                    regions.panel.min_x() - 2.0,
                    regions.panel.min_y(),
                    4.0,
                    regions.panel.height(),
                ),
            };
            sashes.push((WorkbenchSash::Panel, rect));
        }
        sashes
    }

    fn update_sash_drag(&mut self, sash: WorkbenchSash, pos: Point, bounds: Rect) {
        if !pos.x.is_finite() || !pos.y.is_finite() || !bounds.is_finite_positive() {
            return;
        }
        let regions = self.compute_regions(bounds);
        match sash {
            WorkbenchSash::Sidebar => {
                let act_w = regions.activity_bar.width();
                let sec_w = regions.secondary_sidebar.width();
                let max_w = (bounds.width() - act_w - sec_w - 160.0).max(120.0);
                let raw_w = pos.x - (bounds.min_x() + act_w);
                self.layout
                    .mutate(|l| l.sidebar_width = raw_w.clamp(120.0, max_w));
            }
            WorkbenchSash::SecondarySidebar => {
                let act_w = regions.activity_bar.width();
                let sb_w = regions.sidebar.width();
                let max_w = (bounds.width() - act_w - sb_w - 160.0).max(120.0);
                let raw_w = bounds.max_x() - pos.x;
                self.layout
                    .mutate(|l| l.secondary_sidebar_width = raw_w.clamp(120.0, max_w));
            }
            WorkbenchSash::Panel => {
                let top_y = regions.title_bar.max_y();
                let status_h = regions.status_bar.height();
                let main_h = (bounds.height() - regions.title_bar.height() - status_h).max(0.0);
                let center_x = regions.activity_bar.width() + regions.sidebar.width() + bounds.min_x();
                let center_w =
                    (bounds.width() - regions.activity_bar.width() - regions.sidebar.width() - regions.secondary_sidebar.width()).max(0.0);
                match self.layout.panel_position {
                    PanelPosition::Bottom => {
                        let max_h = (main_h - 80.0).max(80.0);
                        let raw_h = top_y + main_h - pos.y;
                        self.layout
                            .mutate(|l| l.panel_height = raw_h.clamp(80.0, max_h));
                    }
                    PanelPosition::Left => {
                        let max_w = (center_w - 160.0).max(120.0);
                        let raw_w = pos.x - center_x;
                        self.layout
                            .mutate(|l| l.panel_width = raw_w.clamp(120.0, max_w));
                    }
                    PanelPosition::Right => {
                        let max_w = (center_w - 160.0).max(120.0);
                        let raw_w = (center_x + center_w) - pos.x;
                        self.layout
                            .mutate(|l| l.panel_width = raw_w.clamp(120.0, max_w));
                    }
                }
            }
        }
    }

    fn secondary_sidebar_outline_items(&self) -> Vec<(String, u32)> {
        let Some(tab) = self.editor_area.model.current_group_ref().active_tab() else {
            return Vec::new();
        };
        let Some(ed) = self
            .editor_area
            .editor_for_group(self.editor_area.model.active_group_id(), &tab.uri)
        else {
            return Vec::new();
        };
        let text = ed.text();
        let path = if rustcode_platform::is_file_uri(&tab.uri) {
            rustcode_platform::file_uri_to_path(&tab.uri)
        } else {
            PathBuf::from(&tab.title)
        };
        search_document_symbols(&text, &path)
            .into_iter()
            .map(|s| (s.name, s.location.range.start.line))
            .collect()
    }

    fn secondary_sidebar_rects(sec_rect: Rect) -> (Rect, Rect, Rect, Rect) {
        let hdr_h = 26.0f32.min(sec_rect.height());
        let tab_outline = Rect::from_xywh(sec_rect.min_x() + 6.0, sec_rect.min_y(), 66.0, hdr_h);
        let tab_hierarchy = Rect::from_xywh(sec_rect.min_x() + 74.0, sec_rect.min_y(), 78.0, hdr_h);
        let close_rect = Rect::from_xywh(
            (sec_rect.max_x() - 24.0).max(sec_rect.min_x()),
            sec_rect.min_y(),
            24.0_f32.min(sec_rect.width()),
            hdr_h,
        );
        let content_rect = Rect::from_xywh(
            sec_rect.min_x(),
            sec_rect.min_y() + hdr_h,
            sec_rect.width(),
            (sec_rect.height() - hdr_h).max(0.0),
        );
        (tab_outline, tab_hierarchy, close_rect, content_rect)
    }

    fn open_context_menu_at(&mut self, location: MenuLocation, pos: Point, context_path: Option<PathBuf>) {
        let ctx = self.cached_context_keys.clone().unwrap_or_default();
        if let Some(p) = &context_path {
            ctx.set_bool("explorerResourceSelected", true);
            ctx.set_bool("explorerResourceIsFolder", p.is_dir());
        } else {
            ctx.set_bool("explorerResourceSelected", false);
        }
        let entries = self.context_menu_registry.resolve(location.as_id(), &ctx);
        self.context_menu.open(location.as_id(), pos, entries, context_path);
    }

    pub fn compute_regions(&self, bounds: Rect) -> WorkbenchRegions {
        if !bounds.is_finite_positive() {
            let zero = Rect::zero();
            return WorkbenchRegions {
                title_bar: zero,
                activity_bar: zero,
                sidebar: zero,
                secondary_sidebar: zero,
                editor_area: zero,
                panel: zero,
                status_bar: zero,
                command_palette: zero,
            };
        }
        let sanitize = |r: Rect| -> Rect {
            if r.is_finite() {
                r
            } else {
                Rect::zero()
            }
        };

        let title_bar_h = if self.layout.title_bar_visible {
            TitleBarView::HEIGHT.min(bounds.height())
        } else {
            0.0
        };
        let title_bar = sanitize(Rect::from_xywh(
            bounds.min_x(),
            bounds.min_y(),
            bounds.width(),
            title_bar_h,
        ));

        let status_bar_h = if self.layout.status_bar_visible { 22.0 } else { 0.0 };
        let status_bar = sanitize(Rect::from_xywh(
            bounds.min_x(),
            bounds.max_y() - status_bar_h,
            bounds.width(),
            status_bar_h,
        ));

        let top_y = bounds.min_y() + title_bar_h;
        let main_h = (bounds.height() - title_bar_h - status_bar_h).max(0.0);
        let act_bar_w = if self.layout.activity_bar_visible { 48.0 } else { 0.0 };
        let activity_bar = sanitize(Rect::from_xywh(
            bounds.min_x(),
            top_y,
            act_bar_w,
            main_h,
        ));

        let sidebar_w = if self.layout.sidebar_visible {
            self.layout
                .sidebar_width
                .min((bounds.width() - act_bar_w - 100.0).max(0.0))
        } else {
            0.0
        };
        let sidebar = sanitize(Rect::from_xywh(
            bounds.min_x() + act_bar_w,
            top_y,
            sidebar_w,
            main_h,
        ));

        let secondary_sidebar_w = if self.layout.secondary_sidebar_visible {
            self.layout
                .secondary_sidebar_width
                .min((bounds.width() - act_bar_w - sidebar_w - 160.0).max(0.0))
        } else {
            0.0
        };
        let secondary_sidebar = sanitize(Rect::from_xywh(
            (bounds.max_x() - secondary_sidebar_w).max(bounds.min_x()),
            top_y,
            secondary_sidebar_w,
            main_h,
        ));

        let center_x = bounds.min_x() + act_bar_w + sidebar_w;
        let rem_w = (bounds.width() - act_bar_w - sidebar_w - secondary_sidebar_w).max(0.0);
        let (panel, editor_area) = match self.layout.panel_position {
            PanelPosition::Bottom => {
                let panel_h = if self.layout.panel_visible {
                    self.layout.panel_height.min((main_h - 100.0).max(0.0))
                } else {
                    0.0
                };
                let p = sanitize(Rect::from_xywh(
                    center_x,
                    top_y + main_h - panel_h,
                    rem_w,
                    panel_h,
                ));
                let e = sanitize(Rect::from_xywh(
                    center_x,
                    top_y,
                    rem_w,
                    (main_h - panel_h).max(0.0),
                ));
                (p, e)
            }
            PanelPosition::Left => {
                let panel_w = if self.layout.panel_visible {
                    self.layout.panel_width.min((rem_w - 160.0).max(0.0))
                } else {
                    0.0
                };
                let p = sanitize(Rect::from_xywh(center_x, top_y, panel_w, main_h));
                let e = sanitize(Rect::from_xywh(
                    center_x + panel_w,
                    top_y,
                    (rem_w - panel_w).max(0.0),
                    main_h,
                ));
                (p, e)
            }
            PanelPosition::Right => {
                let panel_w = if self.layout.panel_visible {
                    self.layout.panel_width.min((rem_w - 160.0).max(0.0))
                } else {
                    0.0
                };
                let ed_w = (rem_w - panel_w).max(0.0);
                let e = sanitize(Rect::from_xywh(center_x, top_y, ed_w, main_h));
                let p = sanitize(Rect::from_xywh(center_x + ed_w, top_y, panel_w, main_h));
                (p, e)
            }
        };

        let pal_w = (600.0f32).min((bounds.width() - 40.0).max(0.0));
        let pal_h = (320.0f32).min((bounds.height() - 80.0).max(0.0));
        let command_palette = sanitize(Rect::from_xywh(
            bounds.min_x() + (bounds.width() - pal_w) / 2.0,
            bounds.min_y() + 40.0,
            pal_w,
            pal_h,
        ));

        WorkbenchRegions {
            title_bar,
            activity_bar,
            sidebar,
            secondary_sidebar,
            editor_area,
            panel,
            status_bar,
            command_palette,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn populate_accessible_editor_group(
        &self,
        parent: &mut AccessibleNode,
        group_id: usize,
        group_rect: Rect,
        font: &FontMetrics,
        is_active_group: bool,
        editor_focused: bool,
        legacy_ids: bool,
    ) {
        let Some(group) = self.editor_area.model.group_by_id(group_id) else {
            return;
        };
        let tab_bar_h = 35.0;
        let tab_bar_rect = Rect::from_xywh(
            group_rect.min_x(),
            group_rect.min_y(),
            group_rect.width(),
            tab_bar_h,
        );
        let tabs_id = if legacy_ids {
            "workbench.editorArea.tabs".to_string()
        } else {
            format!("workbench.editorArea.group.{}.tabs", group_id)
        };
        let mut tab_list =
            AccessibleNode::new(tabs_id, AccessibleRole::TabList, "Open Editors", tab_bar_rect);

        let mut cur_x = group_rect.min_x();
        for (idx, tab) in group.tabs.iter().enumerate() {
            let tab_w = (font.text_width(&tab.title) + 40.0).clamp(80.0, 220.0);
            let tab_rect = Rect::from_xywh(cur_x, group_rect.min_y(), tab_w, tab_bar_h);
            if !tab_rect.is_finite() {
                break;
            }
            let tab_id = if legacy_ids {
                format!("editor.tab.{}", idx)
            } else {
                format!("editor.group.{}.tab.{}", group_id, idx)
            };
            tab_list.add_child(
                AccessibleNode::new(tab_id, AccessibleRole::Tab, tab.title.clone(), tab_rect)
                    .with_selected(group.active_tab_index == Some(idx))
                    .with_action("select"),
            );
            cur_x += tab_w;
        }
        parent.add_child(tab_list);

        if let Some(bc_rect) = self
            .editor_area
            .breadcrumbs_rect_for_group(group_id, group_rect)
        {
            let bc_bar = self
                .editor_area
                .group_breadcrumbs
                .get(&group_id)
                .unwrap_or(&self.editor_area.breadcrumbs);
            let bc_id = if legacy_ids {
                "workbench.editorArea.breadcrumbs".to_string()
            } else {
                format!("workbench.editorArea.group.{}.breadcrumbs", group_id)
            };
            let mut bc_node =
                AccessibleNode::new(bc_id, AccessibleRole::Breadcrumb, "Breadcrumbs", bc_rect);
            let item_rects = bc_bar.compute_item_rects(bc_rect, font);
            for (idx, item_rect) in item_rects {
                if let Some(item) = bc_bar.items.get(idx) {
                    let item_id = if legacy_ids {
                        format!("breadcrumbs.item.{}", idx)
                    } else {
                        format!("breadcrumbs.group.{}.item.{}", group_id, idx)
                    };
                    bc_node.add_child(
                        AccessibleNode::new(
                            item_id,
                            AccessibleRole::BreadcrumbItem,
                            item.label.clone(),
                            item_rect,
                        )
                        .with_selected(bc_bar.selected_index == Some(idx))
                        .with_action("select"),
                    );
                }
            }
            parent.add_child(bc_node);
        }

        let content_rect = self
            .editor_area
            .content_rect_for_group(group_id, group_rect);
        let pane_focused = editor_focused && is_active_group;

        if let Some(tab) = group.active_tab() {
            if let Some(diff_view) = self.editor_area.diff_editors.get(&tab.uri) {
                parent.add_child(
                    AccessibleNode::new(
                        format!("workbench.editor.diff.{}", tab.uri),
                        AccessibleRole::TextEditor,
                        diff_view.title.clone(),
                        content_rect,
                    )
                    .with_read_only(true)
                    .with_focused(pane_focused),
                );
            } else if let Some(webview) = self.editor_area.webview_for_group(group_id, &tab.uri) {
                let mut wv_node = AccessibleNode::new(
                    format!("workbench.editor.webview.{}", tab.uri),
                    AccessibleRole::Panel,
                    webview.title.clone(),
                    content_rect,
                )
                .with_focused(pane_focused);
                for (idx, node) in webview.nodes.iter().enumerate() {
                    match &node.kind {
                        WebviewElementKind::Button { label, .. } => {
                            wv_node.add_child(AccessibleNode::new(
                                format!("webview.btn.{}", idx),
                                AccessibleRole::Button,
                                label.clone(),
                                content_rect,
                            ));
                        }
                        WebviewElementKind::Heading1(t)
                        | WebviewElementKind::Heading2(t)
                        | WebviewElementKind::Heading3(t)
                        | WebviewElementKind::Paragraph(t) => {
                            wv_node.add_child(AccessibleNode::new(
                                format!("webview.text.{}", idx),
                                AccessibleRole::StaticText,
                                t.clone(),
                                content_rect,
                            ));
                        }
                        _ => {}
                    }
                }
                parent.add_child(wv_node);
            } else if let Some(settings_view) = self.editor_area.settings_views.get(&tab.uri) {
                let search_focused = pane_focused && settings_view.focus == SettingsFocus::Search;
                let mut settings_node = AccessibleNode::new(
                    "workbench.editor.settings",
                    AccessibleRole::Panel,
                    "Settings",
                    content_rect,
                )
                .with_focused(pane_focused && !search_focused);

                settings_node.add_child(
                    AccessibleNode::new(
                        "settings.search",
                        AccessibleRole::TextInput,
                        "Search Settings",
                        Rect::from_xywh(
                            content_rect.min_x() + 16.0,
                            content_rect.min_y() + 12.0,
                            (content_rect.width() - 32.0).max(0.0),
                            28.0,
                        ),
                    )
                    .with_value(settings_view.state.search_query.clone())
                    .with_focused(search_focused)
                    .with_action("input"),
                );

                let mut list_node = AccessibleNode::new(
                    "settings.items",
                    AccessibleRole::List,
                    "Settings Items",
                    Rect::from_xywh(
                        content_rect.min_x(),
                        content_rect.min_y() + 48.0,
                        content_rect.width(),
                        (content_rect.height() - 48.0).max(0.0),
                    ),
                );
                for (idx, item) in settings_view.state.get_filtered().into_iter().enumerate() {
                    list_node.add_child(
                        AccessibleNode::new(
                            format!("settings.item.{}", item.key),
                            AccessibleRole::ListItem,
                            item.title.clone(),
                            Rect::from_xywh(
                                content_rect.min_x() + 160.0,
                                content_rect.min_y() + 48.0 + idx as f32 * 56.0,
                                (content_rect.width() - 176.0).max(0.0),
                                52.0,
                            ),
                        )
                        .with_value(item.current_value.to_string()),
                    );
                }
                settings_node.add_child(list_node);
                parent.add_child(settings_node);
            } else if let Some(notebook) = self.editor_area.notebooks.get(&tab.uri) {
                let mut nb_node = AccessibleNode::new(
                    format!("workbench.editor.notebook.{}", tab.uri),
                    AccessibleRole::Panel,
                    tab.title.clone(),
                    content_rect,
                )
                .with_focused(pane_focused);
                nb_node.add_child(AccessibleNode::new(
                    "notebook.toolbar",
                    AccessibleRole::Toolbar,
                    "Notebook Toolbar",
                    Rect::from_xywh(
                        content_rect.min_x(),
                        content_rect.min_y(),
                        content_rect.width(),
                        32.0,
                    ),
                ));
                let mut cells_list = AccessibleNode::new(
                    "notebook.cells",
                    AccessibleRole::List,
                    "Notebook Cells",
                    Rect::from_xywh(
                        content_rect.min_x(),
                        content_rect.min_y() + 32.0,
                        content_rect.width(),
                        (content_rect.height() - 32.0).max(0.0),
                    ),
                );
                for (idx, cell) in notebook.document.cells.iter().enumerate() {
                    cells_list.add_child(
                        AccessibleNode::new(
                            format!("notebook.cell.{}", idx),
                            AccessibleRole::ListItem,
                            format!("Cell {} ({:?})", idx + 1, cell.kind),
                            Rect::from_xywh(
                                content_rect.min_x() + 12.0,
                                content_rect.min_y() + 40.0 + idx as f32 * 64.0,
                                (content_rect.width() - 24.0).max(0.0),
                                60.0,
                            ),
                        )
                        .with_value(cell.source.clone())
                        .with_selected(notebook.active_cell_index == Some(idx)),
                    );
                }
                nb_node.add_child(cells_list);
                parent.add_child(nb_node);
            } else if let Some(editor) = self.editor_area.editor_for_group(group_id, &tab.uri) {
                let mut ed_node = editor.accessibility_node(
                    &format!("workbench.editor.text.{}", tab.uri),
                    content_rect,
                    font,
                );
                ed_node.state.focused = pane_focused && editor.is_focused;
                parent.add_child(ed_node);
            } else {
                let empty_id = if legacy_ids {
                    "workbench.editor.empty".to_string()
                } else {
                    format!("workbench.editor.group.{}.empty", group_id)
                };
                parent.add_child(
                    AccessibleNode::new(
                        empty_id,
                        AccessibleRole::StaticText,
                        "No Active Editor",
                        content_rect,
                    )
                    .with_focused(pane_focused),
                );
            }
        } else {
            let empty_id = if legacy_ids {
                "workbench.editor.empty".to_string()
            } else {
                format!("workbench.editor.group.{}.empty", group_id)
            };
            parent.add_child(
                AccessibleNode::new(
                    empty_id,
                    AccessibleRole::StaticText,
                    "No Active Editor",
                    content_rect,
                )
                .with_focused(pane_focused),
            );
        }
    }

    pub fn accessibility_tree(&self, bounds: Rect, font: &FontMetrics) -> AccessibilityTree {
        let safe_bounds = if bounds.is_finite() {
            Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), bounds.height())
        } else {
            Rect::zero()
        };
        let fallback_font = FontMetrics::default_monospace();
        let font = if font.is_finite() {
            font
        } else {
            &fallback_font
        };
        let regions = self.compute_regions(safe_bounds);
        let mut root = AccessibleNode::new(
            "workbench.window",
            AccessibleRole::Window,
            "RustCode Workbench",
            safe_bounds,
        );

        // 1. Activity bar
        if self.layout.activity_bar_visible {
            let mut act_node = AccessibleNode::new(
                "workbench.activityBar",
                AccessibleRole::Toolbar,
                "Activity Bar",
                regions.activity_bar,
            );
            let slot_h = 48.0;
            for (idx, item) in self.activity_bar.model.items.iter().enumerate() {
                let slot_rect = Rect::from_xywh(
                    regions.activity_bar.min_x(),
                    regions.activity_bar.min_y() + idx as f32 * slot_h,
                    regions.activity_bar.width(),
                    slot_h,
                );
                let mut btn = AccessibleNode::new(
                    format!("activityBar.{}", item.id),
                    AccessibleRole::Button,
                    item.title.clone(),
                    slot_rect,
                )
                .with_selected(idx == self.activity_bar.model.active_item_index)
                .with_action("activate");
                if let Some(badge) = &item.badge {
                    btn = btn.with_description(badge.to_string());
                }
                act_node.add_child(btn);
            }
            root.add_child(act_node);
        }

        // 2. Primary sidebar
        if self.layout.sidebar_visible {
            let sidebar_focused = self.focused_region == FocusedRegion::Sidebar;
            let view_id = self.layout.active_sidebar_view.as_str();
            let (sidebar_title, children, child_has_focus) =
                if view_id == "scm" || view_id == "workbench.view.scm" {
                    let input_focused = sidebar_focused && self.scm_view.commit_input_focused;
                    let mut list = Vec::new();
                    let input_rect = Rect::from_xywh(
                        regions.sidebar.min_x() + 8.0,
                        regions.sidebar.min_y() + 32.0,
                        (regions.sidebar.width() - 16.0).max(0.0),
                        24.0,
                    );
                    list.push(
                        AccessibleNode::new(
                            "workbench.sidebar.scm.commitInput",
                            AccessibleRole::TextInput,
                            "Commit Message",
                            input_rect,
                        )
                        .with_value(self.scm_view.commit_message.clone())
                        .with_focused(input_focused)
                        .with_action("input"),
                    );
                    let btn_rect = Rect::from_xywh(
                        regions.sidebar.min_x() + 8.0,
                        regions.sidebar.min_y() + 62.0,
                        (regions.sidebar.width() - 16.0).max(0.0),
                        24.0,
                    );
                    list.push(
                        AccessibleNode::new(
                            "workbench.sidebar.scm.commitButton",
                            AccessibleRole::Button,
                            "Commit",
                            btn_rect,
                        )
                        .with_action("press"),
                    );
                    let mut tree = AccessibleNode::new(
                        "workbench.sidebar.scm.changes",
                        AccessibleRole::Tree,
                        "Source Control Changes",
                        Rect::from_xywh(
                            regions.sidebar.min_x(),
                            regions.sidebar.min_y() + 92.0,
                            regions.sidebar.width(),
                            (regions.sidebar.height() - 92.0).max(0.0),
                        ),
                    );
                    for (idx, entry) in self.scm_view.repo.changes.iter().enumerate() {
                        let label = entry
                            .path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("change");
                        tree.add_child(AccessibleNode::new(
                            format!("scm.change.{}", idx),
                            AccessibleRole::TreeItem,
                            label,
                            Rect::from_xywh(
                                regions.sidebar.min_x(),
                                regions.sidebar.min_y() + 92.0 + idx as f32 * 22.0,
                                regions.sidebar.width(),
                                22.0,
                            ),
                        ));
                    }
                    list.push(tree);
                    ("Source Control", list, input_focused)
                } else if view_id == "debug" || view_id == "workbench.view.debug" {
                    let tree = AccessibleNode::new(
                        "workbench.sidebar.debug.tree",
                        AccessibleRole::Tree,
                        "Run and Debug",
                        regions.sidebar,
                    );
                    ("Run and Debug", vec![tree], false)
                } else if view_id == "search" || view_id == "workbench.view.search" {
                    let input_focused =
                        sidebar_focused && self.search_view.focused_input == SearchInputFocus::Query;
                    let input_rect = Rect::from_xywh(
                        regions.sidebar.min_x() + 8.0,
                        regions.sidebar.min_y() + 32.0,
                        (regions.sidebar.width() - 16.0).max(0.0),
                        26.0,
                    );
                    let input_node = AccessibleNode::new(
                        "workbench.sidebar.search.query",
                        AccessibleRole::TextInput,
                        "Search",
                        input_rect,
                    )
                    .with_value(self.search_view.query.query.clone())
                    .with_focused(input_focused)
                    .with_action("input");
                    let mut results_tree = AccessibleNode::new(
                        "workbench.sidebar.search.results",
                        AccessibleRole::Tree,
                        "Search Results",
                        Rect::from_xywh(
                            regions.sidebar.min_x(),
                            regions.sidebar.min_y() + 64.0,
                            regions.sidebar.width(),
                            (regions.sidebar.height() - 64.0).max(0.0),
                        ),
                    );
                    for (idx, row) in self.search_view.visible_rows().iter().enumerate() {
                        let label = match row {
                            crate::search::SearchRow::FileHeader { relative_path, .. } => {
                                relative_path.to_string_lossy().to_string()
                            }
                            crate::search::SearchRow::MatchLine { line_content, .. } => {
                                line_content.trim().to_string()
                            }
                        };
                        results_tree.add_child(AccessibleNode::new(
                            format!("search.row.{}", idx),
                            AccessibleRole::TreeItem,
                            label,
                            Rect::from_xywh(
                                regions.sidebar.min_x(),
                                regions.sidebar.min_y() + 64.0 + idx as f32 * 22.0,
                                regions.sidebar.width(),
                                22.0,
                            ),
                        ));
                    }
                    ("Search", vec![input_node, results_tree], input_focused)
                } else if view_id == "extensions" || view_id == "workbench.view.extensions" {
                    let input_focused = sidebar_focused && self.extensions_view.search_focused;
                    let ext_layout = self.extensions_view.compute_layout(regions.sidebar);
                    let search_node = AccessibleNode::new(
                        "workbench.sidebar.extensions.search",
                        AccessibleRole::TextInput,
                        "Search Extensions",
                        ext_layout.search_rect,
                    )
                    .with_value(self.extensions_view.search_query.clone())
                    .with_focused(input_focused)
                    .with_action("input");

                    let mut installed_list = AccessibleNode::new(
                        "workbench.sidebar.extensions.installed",
                        AccessibleRole::List,
                        "Installed Extensions",
                        ext_layout.installed_list_rect,
                    )
                    .with_expanded(!self.extensions_view.installed_collapsed);
                    for (ext, (_, row_rect)) in self
                        .extensions_view
                        .filtered_installed()
                        .into_iter()
                        .zip(ext_layout.installed_rows.iter())
                    {
                        if let Some(clipped) = row_rect.intersect(&ext_layout.body_rect) {
                            installed_list.add_child(
                                AccessibleNode::new(
                                    format!("extensions.installed.{}", ext.id),
                                    AccessibleRole::ListItem,
                                    ext.display_name.clone(),
                                    clipped,
                                )
                                .with_description(format!("v{}", ext.version))
                                .with_selected(
                                    self.extensions_view.selected_id.as_deref()
                                        == Some(ext.id.as_str()),
                                ),
                            );
                        }
                    }

                    let mut marketplace_list = AccessibleNode::new(
                        "workbench.sidebar.extensions.marketplace",
                        AccessibleRole::List,
                        "Marketplace Extensions",
                        ext_layout.marketplace_list_rect,
                    )
                    .with_expanded(!self.extensions_view.marketplace_collapsed);
                    for (ext, (_, row_rect)) in self
                        .extensions_view
                        .filtered_marketplace()
                        .into_iter()
                        .zip(ext_layout.marketplace_rows.iter())
                    {
                        if let Some(clipped) = row_rect.intersect(&ext_layout.body_rect) {
                            marketplace_list.add_child(
                                AccessibleNode::new(
                                    format!("extensions.marketplace.{}", ext.id),
                                    AccessibleRole::ListItem,
                                    ext.display_name.clone(),
                                    clipped,
                                )
                                .with_description(format!("v{}", ext.version))
                                .with_selected(
                                    self.extensions_view.selected_id.as_deref()
                                        == Some(ext.id.as_str()),
                                ),
                            );
                        }
                    }

                    (
                        "Extensions",
                        vec![search_node, installed_list, marketplace_list],
                        input_focused,
                    )
                } else if view_id == "testing" || view_id == "workbench.view.testing" {
                    let nodes = self.testing_view.accessibility_nodes(regions.sidebar, font);
                    ("Testing", nodes, false)
                } else if view_id == "hierarchy" || view_id == "workbench.view.hierarchy" {
                    ("Hierarchy", Vec::new(), false)
                } else {
                    let nodes = self.explorer.accessibility_nodes(regions.sidebar, font);
                    let input_focused = sidebar_focused && self.explorer.is_input_active();
                    ("Explorer", nodes, input_focused)
                };

            let sidebar_node = AccessibleNode::new(
                "workbench.sidebar",
                AccessibleRole::Sidebar,
                sidebar_title,
                regions.sidebar,
            )
            .with_focused(sidebar_focused && !child_has_focus)
            .with_children(children);
            root.add_child(sidebar_node);
        }

        // 3. Editor area
        let mut editor_area_node = AccessibleNode::new(
            "workbench.editorArea",
            AccessibleRole::Panel,
            "Editor Area",
            regions.editor_area,
        );
        let group_rects = self.editor_area.model.compute_group_rects(regions.editor_area);
        let active_gid = self.editor_area.model.active_group_id();
        let multi_group = self.editor_area.model.groups.len() > 1;
        let editor_focused = self.focused_region == FocusedRegion::Editor;

        if !multi_group {
            let group_id = self.editor_area.model.active_group_id();
            let group_rect = group_rects
                .first()
                .map(|(_, r)| *r)
                .unwrap_or(regions.editor_area);
            self.populate_accessible_editor_group(
                &mut editor_area_node,
                group_id,
                group_rect,
                font,
                true,
                editor_focused,
                true,
            );
        } else {
            for (group_id, group_rect) in group_rects {
                let is_active_group = group_id == active_gid;
                let mut group_node = AccessibleNode::new(
                    format!("workbench.editorGroup.{}", group_id),
                    AccessibleRole::Panel,
                    format!("Editor Group {}", group_id),
                    group_rect,
                );
                self.populate_accessible_editor_group(
                    &mut group_node,
                    group_id,
                    group_rect,
                    font,
                    is_active_group,
                    editor_focused,
                    false,
                );
                editor_area_node.add_child(group_node);
            }
        }
        root.add_child(editor_area_node);

        if self.pending_settings_open.load(Ordering::SeqCst)
            && !self.editor_area.settings_views.contains_key("rustcode://settings")
        {
            if let Some(last) = root.children.last_mut() {
                last.add_child(AccessibleNode::new(
                    "workbench.editor.settings",
                    AccessibleRole::Panel,
                    "Settings",
                    regions.editor_area,
                ));
            }
        }

        // 4. Bottom Panel
        if self.layout.panel_visible {
            let panel_focused = self.focused_region == FocusedRegion::Panel;
            let debug_input_focused = panel_focused
                && self.layout.active_panel_tab == "debug_console"
                && self.debug_console.input_focused;
            let terminal_input_focused = panel_focused
                && self.layout.active_panel_tab == "terminal"
                && self.terminal_panel.has_active_terminal()
                && self.terminal_panel.is_focused;
            let mut panel_node = AccessibleNode::new(
                "workbench.panel",
                AccessibleRole::Panel,
                "Panel",
                regions.panel,
            )
            .with_focused(panel_focused && !debug_input_focused && !terminal_input_focused);

            let tab_h = 26.0;
            let mut panel_tabs = AccessibleNode::new(
                "workbench.panel.tabs",
                AccessibleRole::TabList,
                "Panel Tabs",
                Rect::from_xywh(regions.panel.min_x(), regions.panel.min_y(), regions.panel.width(), tab_h),
            );
            for (idx, (id, label)) in [
                ("problems", "PROBLEMS"),
                ("output", "OUTPUT"),
                ("debug_console", "DEBUG CONSOLE"),
                ("terminal", "TERMINAL"),
            ]
            .iter()
            .enumerate()
            {
                panel_tabs.add_child(
                    AccessibleNode::new(
                        format!("panel.tab.{}", id),
                        AccessibleRole::Tab,
                        *label,
                        Rect::from_xywh(
                            regions.panel.min_x() + idx as f32 * 100.0,
                            regions.panel.min_y(),
                            100.0,
                            tab_h,
                        ),
                    )
                    .with_selected(self.layout.active_panel_tab == *id),
                );
            }
            panel_node.add_child(panel_tabs);

            let panel_content_rect = Self::panel_content_rect(regions.panel);
            if self.layout.active_panel_tab == "problems" {
                let mut tree = AccessibleNode::new(
                    "workbench.panel.problems",
                    AccessibleRole::Tree,
                    "Problems",
                    panel_content_rect,
                );
                for (uri, diags) in &self.problems_panel.files {
                    for (idx, diag) in diags.iter().enumerate() {
                        tree.add_child(AccessibleNode::new(
                            format!("problems.{}.{}", uri, idx),
                            AccessibleRole::TreeItem,
                            diag.message.clone(),
                            panel_content_rect,
                        ));
                    }
                }
                panel_node.add_child(tree);
            } else if self.layout.active_panel_tab == "debug_console" {
                panel_node.add_child(
                    AccessibleNode::new(
                        "workbench.panel.debugConsole.input",
                        AccessibleRole::TextInput,
                        "Debug Console Input",
                        Rect::from_xywh(
                            panel_content_rect.min_x(),
                            (panel_content_rect.max_y() - 28.0).max(panel_content_rect.min_y()),
                            panel_content_rect.width(),
                            28.0,
                        ),
                    )
                    .with_value(self.debug_console.input_text.clone())
                    .with_focused(debug_input_focused)
                    .with_action("input"),
                );
            } else if self.layout.active_panel_tab == "terminal" {
                panel_node.add_child(
                    self.terminal_panel
                        .accessibility_node(panel_content_rect, panel_focused),
                );
            } else if self.layout.active_panel_tab == "output" {
                panel_node.add_child(
                    self.output_panel
                        .accessibility_node(panel_content_rect, font),
                );
            } else {
                panel_node.add_child(AccessibleNode::new(
                    format!("workbench.panel.{}", self.layout.active_panel_tab),
                    AccessibleRole::StaticText,
                    self.layout.active_panel_tab.clone(),
                    panel_content_rect,
                ));
            }
            root.add_child(panel_node);
        }

        // 5. Status bar
        if self.layout.status_bar_visible {
            let mut sb_node = AccessibleNode::new(
                "workbench.statusBar",
                AccessibleRole::StatusBar,
                "Status Bar",
                regions.status_bar,
            );
            let item_rects = if let Some(merged) = self.merged_status_bar() {
                StatusBarView::new(merged).item_rects(regions.status_bar, font)
            } else {
                self.status_bar.item_rects(regions.status_bar, font)
            };
            for (item, rect) in item_rects {
                let role = if item.command.is_some() {
                    AccessibleRole::Button
                } else {
                    AccessibleRole::StaticText
                };
                let mut item_node = AccessibleNode::new(
                    format!("statusBar.{}", item.id),
                    role,
                    item.text.clone(),
                    rect,
                );
                if let Some(tooltip) = &item.tooltip {
                    item_node = item_node.with_description(tooltip.clone());
                }
                if item.command.is_some() {
                    item_node = item_node.with_action("press");
                }
                sb_node.add_child(item_node);
            }
            root.add_child(sb_node);
        }

        // 6. Command palette overlay (added last so hit_test reverse-order picks it first)
        if self.is_command_palette_open {
            if let Some(palette) = &self.command_palette {
                let pal_focused = self.focused_region == FocusedRegion::CommandPalette;
                let mut dialog = AccessibleNode::new(
                    "workbench.commandPalette",
                    AccessibleRole::Dialog,
                    "Command Palette",
                    regions.command_palette,
                );
                dialog.add_child(
                    AccessibleNode::new(
                        "commandPalette.input",
                        AccessibleRole::TextInput,
                        "Command Input",
                        Rect::from_xywh(
                            regions.command_palette.min_x() + 10.0,
                            regions.command_palette.min_y() + 8.0,
                            (regions.command_palette.width() - 20.0).max(0.0),
                            28.0,
                        ),
                    )
                    .with_value(palette.model.query.clone())
                    .with_focused(pal_focused)
                    .with_action("input"),
                );

                let mut results_list = AccessibleNode::new(
                    "commandPalette.results",
                    AccessibleRole::List,
                    "Command Palette Results",
                    Rect::from_xywh(
                        regions.command_palette.min_x(),
                        (regions.command_palette.min_y() + 42.0).min(regions.command_palette.max_y()),
                        regions.command_palette.width(),
                        (regions.command_palette.height() - 42.0).max(0.0),
                    ),
                );
                if palette.model.mode == QuickInputMode::Line {
                    let row_y = (regions.command_palette.min_y() + 42.0).min(regions.command_palette.max_y());
                    let row_h = 26.0f32.min((regions.command_palette.max_y() - row_y).max(0.0));
                    results_list.add_child(
                        AccessibleNode::new(
                            "commandPalette.item.0",
                            AccessibleRole::ListItem,
                            palette.model.line_prompt_text(),
                            Rect::from_xywh(
                                regions.command_palette.min_x() + 4.0f32.min(regions.command_palette.width() * 0.5),
                                row_y,
                                (regions.command_palette.width() - 8.0).max(0.0),
                                row_h,
                            ),
                        )
                        .with_selected(true),
                    );
                } else {
                    for (vis_idx, &item_idx) in palette.model.filtered.iter().enumerate() {
                        let row_y = regions.command_palette.min_y() + 42.0 + vis_idx as f32 * 26.0;
                        if row_y >= regions.command_palette.max_y() {
                            break;
                        }
                        let row_h = 26.0f32.min((regions.command_palette.max_y() - row_y).max(0.0));
                        if let Some(item) = palette.model.items.get(item_idx) {
                            results_list.add_child(
                                AccessibleNode::new(
                                    format!("commandPalette.item.{}", vis_idx),
                                    AccessibleRole::ListItem,
                                    item.label.clone(),
                                    Rect::from_xywh(
                                        regions.command_palette.min_x() + 4.0f32.min(regions.command_palette.width() * 0.5),
                                        row_y,
                                        (regions.command_palette.width() - 8.0).max(0.0),
                                        row_h,
                                    ),
                                )
                                .with_selected(vis_idx == palette.model.selected_index),
                            );
                        }
                    }
                }
                dialog.add_child(results_list);
                root.add_child(dialog);
            }
        }

        // 7. Notifications overlay (toasts & modal dialogs)
        for notif_node in self.notifications.accessibility_nodes(safe_bounds, font) {
            root.add_child(notif_node);
        }

        // 8. Interactive prompt overlay (showQuickPick / showInputBox)
        if let Some(prompt_node) = self.prompt_overlay.accessibility_node(safe_bounds, font) {
            root.add_child(prompt_node);
        }

        AccessibilityTree::new(root)
    }
}

pub struct WorkbenchRegions {
    pub title_bar: Rect,
    pub activity_bar: Rect,
    pub sidebar: Rect,
    pub secondary_sidebar: Rect,
    pub editor_area: Rect,
    pub panel: Rect,
    pub status_bar: Rect,
    pub command_palette: Rect,
}

impl View for WorkbenchView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !bounds.is_finite_positive() || !font.is_finite() {
            return;
        }
        let eff_font = self.effective_font_metrics(font);
        let font = &eff_font;
        let regions = self.compute_regions(bounds);

        // 1. Overall base background
        scene.push_quad(Quad::new(bounds).with_background(theme.editor_background));

        // 1b. Title bar
        if self.layout.title_bar_visible {
            self.title_bar.render_bar(regions.title_bar, theme, font, scene);
        }

        // 2. Activity bar
        if self.layout.activity_bar_visible {
            self.activity_bar.render(regions.activity_bar, theme, font, scene);
        }

        // 3. Primary sidebar (Explorer / SCM / Debug / Search / Extensions)
        if self.layout.sidebar_visible {
            let view_id = self.layout.active_sidebar_view.as_str();
            if view_id == "scm" || view_id == "workbench.view.scm" {
                self.scm_view.render(regions.sidebar, theme, font, scene);
            } else if view_id == "debug" || view_id == "workbench.view.debug" {
                self.debug_sidebar.render(regions.sidebar, theme, font, scene);
            } else if view_id == "search" || view_id == "workbench.view.search" {
                self.search_view.render(regions.sidebar, theme, font, scene);
            } else if view_id == "extensions" || view_id == "workbench.view.extensions" {
                self.extensions_view.render(regions.sidebar, theme, font, scene);
            } else if view_id == "testing" || view_id == "workbench.view.testing" {
                self.testing_view.render(regions.sidebar, theme, font, scene);
            } else if view_id == "hierarchy" || view_id == "workbench.view.hierarchy" {
                self.hierarchy_view.render(regions.sidebar, theme, font, scene);
            } else {
                self.explorer.render(regions.sidebar, theme, font, scene);
            }
        }

        // 4. Editor area & Floating Debug Toolbar
        self.editor_area.render(regions.editor_area, theme, font, scene);

        if self.pending_settings_open.load(Ordering::SeqCst)
            && !self.editor_area.settings_views.contains_key("rustcode://settings")
        {
            let tab_bar_h = 35.0;
            let tab_rect = Rect::from_xywh(
                regions.editor_area.min_x(),
                regions.editor_area.min_y(),
                100.0f32.min(regions.editor_area.width()),
                tab_bar_h,
            );
            if tab_rect.is_finite_positive() {
                scene.push_quad(Quad::new(tab_rect).with_background(theme.tab_active_bg));
                scene.push_text(GlyphRun::new(
                    "Settings".to_string(),
                    Point::new(
                        tab_rect.min_x() + 10.0,
                        tab_rect.min_y() + (tab_bar_h - font.line_height) / 2.0,
                    ),
                    tab_rect,
                    theme.tab_active_fg,
                    font.font_size,
                ));
            }
            let content_rect = Rect::from_xywh(
                regions.editor_area.min_x(),
                regions.editor_area.min_y() + tab_bar_h,
                regions.editor_area.width(),
                (regions.editor_area.height() - tab_bar_h).max(0.0),
            );
            if content_rect.is_finite_positive() {
                let mut settings_view = SettingsUiView::default();
                settings_view.state.open();
                if let Some(cfg) = &self.cached_config {
                    settings_view.set_config(cfg.clone());
                }
                settings_view.render(content_rect, theme, font, scene);
            }
        }

        if self.debug_toolbar.is_visible {
            let tb_bounds = self.debug_toolbar.toolbar_bounds(regions.editor_area);
            self.debug_toolbar.render(tb_bounds, theme, font, scene);
        }

        // 5. Panel (terminal / problems / debug_console / output)
        if self.layout.panel_visible {
            let tab_h = 26.0f32;
            let tab_bar_rect = Rect::from_xywh(regions.panel.min_x(), regions.panel.min_y(), regions.panel.width(), tab_h);
            scene.push_quad(
                Quad::new(tab_bar_rect)
                    .with_background(theme.panel_bg)
                    .with_border_edges(theme.panel_border, Edges::new(1.0, 0.0, 1.0, 0.0)),
            );

            let mut tab_bounds = Vec::new();
            let mut cur_x = regions.panel.min_x() + 8.0;

            let tabs = [
                ("problems", format!("PROBLEMS {}", self.problems_panel.total_count())),
                ("output", "OUTPUT".to_string()),
                ("debug_console", "DEBUG CONSOLE".to_string()),
                ("terminal", "TERMINAL".to_string()),
            ];

            for (tab_id, label) in tabs {
                let is_active = self.layout.active_panel_tab == tab_id;
                let text_w = font.text_width(&label);
                let tab_w = text_w + 16.0;
                let tab_rect = Rect::from_xywh(cur_x, regions.panel.min_y(), tab_w, tab_h);
                tab_bounds.push((tab_id.to_string(), tab_rect));

                if is_active {
                    scene.push_quad(
                        Quad::new(tab_rect)
                            .with_background(theme.tab_active_bg)
                            .with_border_edges(theme.activity_bar_active_border, Edges::new(0.0, 0.0, 2.0, 0.0)),
                    );
                }

                let text_color = if is_active {
                    theme.tab_active_fg
                } else {
                    theme.tab_inactive_fg
                };

                scene.push_text(GlyphRun::new(
                    label,
                    Point::new(cur_x + 8.0, regions.panel.min_y() + 5.0),
                    Rect::from_xywh(cur_x + 8.0, regions.panel.min_y() + 5.0, text_w, font.line_height),
                    text_color,
                    font.font_size * 0.85,
                ));

                cur_x += tab_w + 4.0;
            }

            // Close button on right edge
            let close_rect = Rect::from_xywh(regions.panel.max_x() - 28.0, regions.panel.min_y(), 24.0, tab_h);
            tab_bounds.push(("close".to_string(), close_rect));
            scene.push_text(GlyphRun::new(
                "✕".to_string(),
                Point::new(close_rect.min_x() + 6.0, regions.panel.min_y() + 5.0),
                Rect::from_xywh(close_rect.min_x() + 6.0, regions.panel.min_y() + 5.0, 16.0, font.line_height),
                theme.sidebar_header_fg,
                font.font_size * 0.85,
            ));

            *self.panel_tab_bounds.write() = tab_bounds;

            // Content below tab bar
            let content_rect = Self::panel_content_rect(regions.panel);

            match self.layout.active_panel_tab.as_str() {
                "problems" => self.problems_panel.render(content_rect, theme, font, scene),
                "debug_console" => self.debug_console.render(content_rect, theme, font, scene),
                "terminal" => self.terminal_panel.render(content_rect, theme, font, scene),
                "output" => self.output_panel.render(content_rect, theme, font, scene),
                _ => {
                    scene.push_finite_quad(
                        Quad::new(content_rect).with_background(theme.panel_bg),
                    );
                }
            }
        }

        // 6. Status bar
        if self.layout.status_bar_visible {
            if let Some(merged) = self.merged_status_bar() {
                StatusBarView::new(merged).render(regions.status_bar, theme, font, scene);
            } else {
                self.status_bar.render(regions.status_bar, theme, font, scene);
            }
        }

        // 6b. Secondary Sidebar (Outline & Hierarchy switcher)
        if self.layout.secondary_sidebar_visible && regions.secondary_sidebar.is_finite_positive() {
            let sr = regions.secondary_sidebar;
            scene.push_finite_quad(
                Quad::new(sr)
                    .with_background(theme.sidebar_bg)
                    .with_border_edges(theme.sidebar_border, Edges::new(0.0, 0.0, 0.0, 1.0)),
            );
            let (tab_outline, tab_hierarchy, close_rect, content_rect) =
                Self::secondary_sidebar_rects(sr);
            let is_hier = self.layout.active_secondary_sidebar_view == "hierarchy";
            scene.push_finite_text(GlyphRun::new(
                "OUTLINE".to_string(),
                Point::new(tab_outline.min_x() + 2.0, tab_outline.min_y() + 5.0),
                tab_outline,
                if !is_hier { theme.tab_active_fg } else { theme.tab_inactive_fg },
                font.font_size * 0.85,
            ));
            scene.push_finite_text(GlyphRun::new(
                "HIERARCHY".to_string(),
                Point::new(tab_hierarchy.min_x() + 2.0, tab_hierarchy.min_y() + 5.0),
                tab_hierarchy,
                if is_hier { theme.tab_active_fg } else { theme.tab_inactive_fg },
                font.font_size * 0.85,
            ));
            scene.push_finite_text(GlyphRun::new(
                "x".to_string(),
                Point::new(close_rect.min_x() + 6.0, close_rect.min_y() + 5.0),
                close_rect,
                theme.sidebar_header_fg,
                font.font_size * 0.85,
            ));
            if is_hier {
                self.hierarchy_view.render(content_rect, theme, font, scene);
            } else {
                let symbols = self.secondary_sidebar_outline_items();
                if symbols.is_empty() {
                    scene.push_finite_text(GlyphRun::new(
                        "No symbols in active editor".to_string(),
                        Point::new(content_rect.min_x() + 10.0, content_rect.min_y() + 10.0),
                        content_rect,
                        theme.breadcrumb_fg,
                        font.font_size * 0.9,
                    ));
                } else {
                    for (idx, (sym_name, sym_line)) in symbols.iter().enumerate() {
                        let ry = content_rect.min_y() + 4.0 + (idx as f32) * 22.0;
                        if ry + 20.0 > content_rect.max_y() {
                            break;
                        }
                        let row_rect = Rect::from_xywh(content_rect.min_x() + 4.0, ry, content_rect.width() - 8.0, 20.0);
                        scene.push_finite_text(GlyphRun::new(
                            format!("{} :{}", sym_name, sym_line + 1),
                            Point::new(row_rect.min_x() + 6.0, ry + 3.0),
                            row_rect,
                            theme.sidebar_fg,
                            font.font_size * 0.9,
                        ));
                    }
                }
            }
        }

        // 6c. Draggable Workbench Sashes highlight
        for (sash_kind, sash_rect) in self.compute_sashes(&regions) {
            let color = if self.active_sash == Some(sash_kind) {
                theme.sash_hover
            } else {
                theme.panel_border
            };
            scene.push_finite_quad(Quad::new(sash_rect).with_background(color));
        }

        // 7. Command Palette modal overlay
        if self.is_command_palette_open {
            if let Some(palette) = &self.command_palette {
                // Dim backdrop
                scene.push_quad(Quad::new(bounds).with_background(Rgba::new(0.0, 0.0, 0.0, 0.35)));
                palette.render(regions.command_palette, theme, font, scene);
            }
        }

        // 8. Notifications overlay (toasts & modal dialogs)
        self.notifications.render(bounds, theme, font, scene);

        // 9. Interactive prompt overlay (showQuickPick / showInputBox)
        if self.prompt_overlay.is_open() {
            self.prompt_overlay.render(bounds, theme, font, scene);
        }

        // 10. Multi-file Refactoring Preview overlay
        if self.refactor_preview.is_visible {
            scene.push_quad(Quad::new(bounds).with_background(Rgba::new(0.0, 0.0, 0.0, 0.35)));
            self.refactor_preview.render(bounds, theme, font, scene);
        }

        // 11. Custom TitleBar dropdown menu & Workbench Context Menu overlays (top z-order)
        if self.layout.title_bar_visible {
            self.title_bar
                .render_dropdown(regions.title_bar, bounds, theme, font, scene);
        }
        if self.context_menu.is_open {
            self.context_menu.render(bounds, theme, font, scene);
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let regions = self.compute_regions(bounds);

        // Modal prompt overlay intercept (showQuickPick / showInputBox)
        if self.prompt_overlay.is_open() {
            if matches!(event, InputEvent::MouseUp(_)) {
                self.editor_area.clear_all_drag_state();
            }
            let res = self.prompt_overlay.handle_event(event, bounds);
            if let Some(act) = self.prompt_overlay.last_action.take() {
                self.last_action = Some(WorkbenchAction::PromptAction(act));
            }
            return res;
        }

        // Modal refactor preview overlay intercept
        if self.refactor_preview.is_visible {
            if matches!(event, InputEvent::MouseUp(_)) {
                self.editor_area.clear_all_drag_state();
            }
            let res = self.refactor_preview.handle_event(event, bounds);
            if let Some(act) = self.refactor_preview.last_action.take() {
                self.last_action = Some(WorkbenchAction::RefactorPreviewAction(act));
            }
            return res;
        }

        // Notifications overlay (modal dialog intercepts all events; toasts intercept only button clicks)
        let notif_res = self.notifications.handle_event(event, bounds);
        if let Some(act) = self.notifications.last_action.take() {
            self.last_action = Some(WorkbenchAction::NotificationAction(act));
        }
        if notif_res == EventResult::Consumed {
            return EventResult::Consumed;
        }

        // Modal command palette intercept
        if self.is_command_palette_open {
            if matches!(event, InputEvent::MouseUp(_)) {
                self.editor_area.clear_all_drag_state();
            }
            if let Some(palette) = &mut self.command_palette {
                let res = palette.handle_event(event, regions.command_palette);
                if let Some(action) = &palette.last_action {
                    match action {
                        PaletteAction::Execute(cmd) => {
                            let cmd_str = cmd.clone();
                            self.close_command_palette();
                            self.last_action = Some(WorkbenchAction::ExecuteCommand(cmd_str));
                            return EventResult::Consumed;
                        }
                        PaletteAction::OpenFile { path, line, column } => {
                            let path_clone = path.clone();
                            let line_val = *line;
                            let col_val = *column;
                            self.close_command_palette();
                            let target_uri = self
                                .editor_area
                                .find_open_tab_uri_for_path(&path_clone)
                                .unwrap_or_else(|| rustcode_platform::path_to_file_uri(&path_clone));
                            let title = path_clone
                                .file_name()
                                .map_or("untitled", |n| n.to_str().unwrap_or(""));
                            let content = if self.editor_area.any_editor_for_uri(&target_uri).is_some() {
                                String::new()
                            } else {
                                std::fs::read_to_string(&path_clone).unwrap_or_default()
                            };
                            self.editor_area.open_file(&target_uri, title, &content, false);
                            if let Some(l) = line_val {
                                let target_pos = rustcode_editor::Position::new(
                                    l.saturating_sub(1),
                                    col_val.map(|c| c.saturating_sub(1)).unwrap_or(0),
                                );
                                if let Some(active_editor) = self.editor_area.active_editor_mut() {
                                    active_editor.jump_to_position_and_unfold(target_pos);
                                }
                            }
                            self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                            self.last_action = Some(WorkbenchAction::OpenFile(path_clone));
                            return EventResult::Consumed;
                        }
                        PaletteAction::JumpToLine { line, column } => {
                            let line_val = *line;
                            let col_val = *column;
                            self.close_command_palette();
                            let target_pos = rustcode_editor::Position::new(
                                line_val.saturating_sub(1),
                                col_val.map(|c| c.saturating_sub(1)).unwrap_or(0),
                            );
                            if let Some(active_editor) = self.editor_area.active_editor_mut() {
                                active_editor.jump_to_position_and_unfold(target_pos);
                            }
                            self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                            return EventResult::Consumed;
                        }
                        PaletteAction::Close => {
                            self.close_command_palette();
                            return EventResult::Consumed;
                        }
                    }
                }

                if let InputEvent::MouseDown(mouse) = event {
                    if !regions.command_palette.contains(mouse.position) {
                        self.close_command_palette();
                        return EventResult::Consumed;
                    }
                }

                return res;
            }
        }

        // Context Menu overlay (captures clicks/keys before region dispatch)
        if self.context_menu.is_open {
            let res = self.context_menu.handle_event(event, bounds);
            if let Some(cmd) = self.context_menu.take_selected_command() {
                let ctx_path = self.context_menu.context_path.take();
                self.last_action = Some(WorkbenchAction::ContextMenuCommand {
                    command: cmd,
                    context_path: ctx_path,
                });
            }
            if res == EventResult::Consumed {
                return EventResult::Consumed;
            }
        }

        // Custom TitleBar & Menu dropdown intercept
        if self.layout.title_bar_visible {
            let pointer_in_title = match event {
                InputEvent::MouseDown(m) => regions.title_bar.contains(m.position),
                InputEvent::MouseMove(mm) => regions.title_bar.contains(mm.position),
                _ => false,
            };
            if self.title_bar.is_dropdown_open() || pointer_in_title {
                let res = self.title_bar.dispatch_event_with_overlay(
                    event,
                    regions.title_bar,
                    bounds,
                );
                if let Some(act) = self.title_bar.take_action() {
                    match act {
                        TitleBarAction::TogglePrimarySidebar => self.toggle_sidebar(),
                        TitleBarAction::ToggleSecondarySidebar => {
                            self.layout.toggle_secondary_sidebar();
                            self.last_action = Some(WorkbenchAction::SecondarySidebarToggled(
                                self.layout.secondary_sidebar_visible,
                            ));
                        }
                        TitleBarAction::TogglePanel => self.toggle_panel(),
                        TitleBarAction::ExecuteCommand(cmd) => {
                            self.last_action = Some(WorkbenchAction::ExecuteCommand(cmd));
                        }
                        other => {
                            self.last_action = Some(WorkbenchAction::TitleBarAction(other));
                        }
                    }
                }
                if res == EventResult::Consumed {
                    return EventResult::Consumed;
                }
            }
        }

        match event {
            InputEvent::MouseDown(mouse) => {
                // Priority 0: Draggable Workbench sashes (±4px hit margin)
                if mouse.button == MouseButton::Left {
                    for (sash_kind, sash_rect) in self.compute_sashes(&regions) {
                        let hit = Rect::from_xywh(
                            sash_rect.min_x() - 4.0,
                            sash_rect.min_y() - 4.0,
                            sash_rect.width() + 8.0,
                            sash_rect.height() + 8.0,
                        );
                        if hit.contains(mouse.position) {
                            self.active_sash = Some(sash_kind);
                            return EventResult::Consumed;
                        }
                    }
                }

                // Priority 0b: Right-click context menu dispatch
                if mouse.button == MouseButton::Right {
                    if self.layout.sidebar_visible && regions.sidebar.contains(mouse.position) {
                        self.focused_region = FocusedRegion::Sidebar;
                        let is_explorer = matches!(
                            self.layout.active_sidebar_view.as_str(),
                            "workbench.view.explorer" | "explorer" | ""
                        );
                        if is_explorer {
                            let path = self.explorer.entry_path_at_point(regions.sidebar, mouse.position);
                            if let Some(p) = &path {
                                self.explorer.model.selected_path = Some(p.clone());
                            }
                            self.open_context_menu_at(
                                MenuLocation::ExplorerContext,
                                mouse.position,
                                path,
                            );
                            return EventResult::Consumed;
                        }
                    } else if regions.editor_area.contains(mouse.position) {
                        self.focused_region = FocusedRegion::Editor;
                        if self.editor_area.tab_bar_contains(regions.editor_area, mouse.position) {
                            let _ = self.editor_area.handle_event(event, regions.editor_area);
                            self.open_context_menu_at(
                                MenuLocation::EditorTitleContext,
                                mouse.position,
                                None,
                            );
                        } else {
                            let _ = self.editor_area.handle_event(event, regions.editor_area);
                            if let Some(ed) = self.editor_area.active_editor_mut() {
                                ed.context_menu.close();
                            }
                            self.open_context_menu_at(
                                MenuLocation::EditorContext,
                                mouse.position,
                                None,
                            );
                        }
                        return EventResult::Consumed;
                    }
                }

                // Priority 1: Floating Debug Toolbar overlay
                if self.debug_toolbar.is_visible {
                    let tb_bounds = self.debug_toolbar.toolbar_bounds(regions.editor_area);
                    if tb_bounds.contains(mouse.position) {
                        let res = self.debug_toolbar.handle_event(event, tb_bounds);
                        if let Some(act) = self.debug_toolbar.last_action.take() {
                            self.last_action = Some(WorkbenchAction::DebugToolbar(act));
                        }
                        return res;
                    }
                }

                if self.layout.activity_bar_visible && regions.activity_bar.contains(mouse.position) {
                    let res = self.activity_bar.handle_event(event, regions.activity_bar);
                    if let Some(action) = &self.activity_bar.last_action {
                        match action {
                            ActivityBarAction::ToggleSidebar => self.toggle_sidebar(),
                            ActivityBarAction::SelectView(v) => self.layout.set_sidebar_view(v),
                        }
                    }
                    res
                } else if self.layout.sidebar_visible && regions.sidebar.contains(mouse.position) {
                    self.focused_region = FocusedRegion::Sidebar;
                    let view_id = self.layout.active_sidebar_view.as_str();
                    if view_id == "scm" || view_id == "workbench.view.scm" {
                        let res = self.scm_view.handle_event(event, regions.sidebar);
                        self.sync_scm_status_bar();
                        if let Some(scm_act) = self.scm_view.last_action.take() {
                            if let ScmAction::OpenDiff { path, title, original, modified } = &scm_act {
                                let path_str = format!("diff://{}", path.to_string_lossy().replace('\\', "/"));
                                self.editor_area.open_diff(&path_str, title, original, modified);
                                self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                                self.last_action = Some(WorkbenchAction::OpenDiff {
                                    path: path.clone(),
                                    title: title.clone(),
                                    original: original.clone(),
                                    modified: modified.clone(),
                                });
                            } else {
                                self.last_action = Some(WorkbenchAction::ScmAction(scm_act));
                            }
                        }
                        res
                    } else if view_id == "debug" || view_id == "workbench.view.debug" {
                        let res = self.debug_sidebar.handle_event(event, regions.sidebar);
                        if let Some(act) = self.debug_sidebar.last_action.take() {
                            match &act {
                                DebugViewAction::SelectFrame {
                                    path: Some(p),
                                    line,
                                    ..
                                } => {
                                    let path_buf = PathBuf::from(p);
                                    let title = path_buf.file_name().map_or("untitled", |n| n.to_str().unwrap_or(""));
                                    let content = std::fs::read_to_string(&path_buf).unwrap_or_default();
                                    self.editor_area.open_file(p, title, &content, true);
                                    if let Some(active_editor) = self.editor_area.active_editor_mut() {
                                        active_editor.set_primary_position(rustcode_editor::Position::new((*line as usize).saturating_sub(1), 0));
                                        active_editor.set_execution_line(Some(*line as usize));
                                        if let Some(bps) = self.debug_sidebar.session.breakpoints.get(p) {
                                            for bp in bps {
                                                active_editor.add_breakpoint(bp.line as usize);
                                            }
                                        }
                                    }
                                    self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                                }
                                DebugViewAction::SelectFrame { path: None, .. } => {}
                                DebugViewAction::JumpToBreakpoint { path, line } => {
                                    let path_buf = PathBuf::from(path);
                                    let title = path_buf.file_name().map_or("untitled", |n| n.to_str().unwrap_or(""));
                                    let content = std::fs::read_to_string(&path_buf).unwrap_or_default();
                                    self.editor_area.open_file(path, title, &content, true);
                                    if let Some(active_editor) = self.editor_area.active_editor_mut() {
                                        active_editor.set_primary_position(rustcode_editor::Position::new((*line as usize).saturating_sub(1), 0));
                                        if let Some(bps) = self.debug_sidebar.session.breakpoints.get(path) {
                                            for bp in bps {
                                                active_editor.add_breakpoint(bp.line as usize);
                                            }
                                        }
                                    }
                                    self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                                }
                                _ => {}
                            }
                            self.last_action = Some(WorkbenchAction::DebugAction(act));
                        }
                        res
                    } else if view_id == "search" || view_id == "workbench.view.search" {
                        let res = self.search_view.handle_event(event, regions.sidebar);
                        if let Some(act) = self.search_view.last_action.take() {
                            match act {
                                SearchAction::OpenMatch { path, line, col } => {
                                    let path_str = path.to_string_lossy().to_string();
                                    let title = path.file_name().map_or("untitled", |n| n.to_str().unwrap_or(""));
                                    let content = std::fs::read_to_string(&path).unwrap_or_default();
                                    self.editor_area.open_file(&path_str, title, &content, true);
                                    if let Some(active_editor) = self.editor_area.active_editor_mut() {
                                        active_editor.set_primary_position(rustcode_editor::Position::new(line.saturating_sub(1), col));
                                    }
                                    self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                                    self.last_action = Some(WorkbenchAction::OpenFile(path));
                                }
                                SearchAction::ExecuteSearch => {
                                    self.search_view.execute_search(self.workspace.folders());
                                }
                                SearchAction::ClearSearch => {
                                    self.search_view.clear();
                                }
                                _ => {}
                            }
                        }
                        res
                    } else if view_id == "extensions" || view_id == "workbench.view.extensions" {
                        let res = self.extensions_view.handle_event(event, regions.sidebar);
                        self.process_extensions_action();
                        res
                    } else if view_id == "testing" || view_id == "workbench.view.testing" {
                        let res = self.testing_view.handle_event(event, regions.sidebar);
                        self.process_testing_action();
                        res
                    } else if view_id == "hierarchy" || view_id == "workbench.view.hierarchy" {
                        let res = self.hierarchy_view.handle_event(event, regions.sidebar);
                        if let Some(act) = self.hierarchy_view.last_action.take() {
                            self.last_action = Some(WorkbenchAction::HierarchyAction(act));
                        }
                        res
                    } else {
                        let res = self.explorer.handle_event(event, regions.sidebar);
                        self.process_explorer_action();
                        res
                    }
                } else if self.layout.status_bar_visible && regions.status_bar.contains(mouse.position) {
                    let res = self.status_bar.handle_event(event, regions.status_bar);
                    if let Some(action) = &self.status_bar.last_action {
                        match action {
                            StatusBarAction::ExecuteCommand(cmd) => {
                                if cmd == "workbench.action.problems.focus" {
                                    self.layout.mutate(|l| {
                                        l.panel_visible = true;
                                        l.set_panel_tab("problems");
                                    });
                                    self.focused_region = FocusedRegion::Panel;
                                } else if cmd == "workbench.view.scm" {
                                    self.layout.mutate(|l| {
                                        l.sidebar_visible = true;
                                        l.set_sidebar_view("scm");
                                    });
                                    self.focused_region = FocusedRegion::Sidebar;
                                }
                                self.last_action = Some(WorkbenchAction::ExecuteCommand(cmd.clone()));
                            }
                        }
                    }
                    res
                } else if self.layout.panel_visible && regions.panel.contains(mouse.position) {
                    self.focused_region = FocusedRegion::Panel;
                    let tab_h = 26.0f32;
                    let tab_bar_rect = Rect::from_xywh(regions.panel.min_x(), regions.panel.min_y(), regions.panel.width(), tab_h);

                    if tab_bar_rect.contains(mouse.position) {
                        self.output_panel.dropdown_open = false;
                        let tabs = self.panel_tab_bounds.read().clone();
                        for (tab_id, rect) in tabs {
                            if rect.contains(mouse.position) {
                                if tab_id == "close" {
                                    self.layout.mutate(|l| l.panel_visible = false);
                                    if self.focused_region == FocusedRegion::Panel {
                                        self.focused_region = FocusedRegion::Editor;
                                    }
                                    self.last_action = Some(WorkbenchAction::PanelToggled(false));
                                } else {
                                    self.layout.set_panel_tab(&tab_id);
                                    if tab_id == "terminal" {
                                        let default_cwd = self
                                            .workspace
                                            .folders()
                                            .first()
                                            .map(|f| f.path.clone());
                                        self.terminal_panel.ensure_default_terminal(default_cwd);
                                        self.terminal_panel.is_focused = true;
                                    }
                                }
                                return EventResult::Consumed;
                            }
                        }
                        EventResult::Consumed
                    } else {
                        let content_rect = Self::panel_content_rect(regions.panel);
                        match self.layout.active_panel_tab.as_str() {
                            "problems" => {
                                let res = self.problems_panel.handle_event(event, content_rect);
                                if let Some(ProblemsAction::JumpToLocation { uri, position }) =
                                    self.problems_panel.last_action.take()
                                {
                                    let path = rustcode_platform::file_uri_to_path(&uri);
                                    let title = path
                                        .file_name()
                                        .map_or("untitled", |n| n.to_str().unwrap_or(""));
                                    let target_uri = if !uri.contains("://") && !path.is_absolute() {
                                        uri.clone()
                                    } else {
                                        self.editor_area
                                            .find_open_tab_uri_for_path(&path)
                                            .unwrap_or_else(|| uri.clone())
                                    };
                                    let content = if self.editor_area.any_editor_for_uri(&target_uri).is_some() {
                                        String::new()
                                    } else {
                                        std::fs::read_to_string(&path).unwrap_or_default()
                                    };
                                    self.editor_area.open_file(&target_uri, title, &content, true);
                                    if let Some(active_editor) =
                                        self.editor_area.active_editor_mut()
                                    {
                                        active_editor.jump_to_position_and_unfold(position);
                                        if let Some(file_diags) =
                                            self.problems_panel.files.get(&uri)
                                        {
                                            active_editor.set_diagnostics(file_diags.clone());
                                        }
                                    }
                                    self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                                    self.focused_region = FocusedRegion::Editor;
                                    self.last_action = Some(WorkbenchAction::JumpToDiagnostic {
                                        uri,
                                        line: position.line,
                                        column: position.column,
                                    });
                                }
                                res
                            }
                            "debug_console" => {
                                let res = self.debug_console.handle_event(event, content_rect);
                                if let Some(act) = self.debug_console.last_action.take() {
                                    self.last_action = Some(WorkbenchAction::DebugAction(act));
                                }
                                res
                            }
                            "terminal" => {
                                let res = self.terminal_panel.handle_event(event, content_rect);
                                if let Some(act) = self.terminal_panel.last_action.take() {
                                    self.last_action = Some(WorkbenchAction::TerminalAction(act));
                                }
                                res
                            }
                            "output" => {
                                let res = self.output_panel.handle_event(event, content_rect);
                                if let Some(act) = self.output_panel.last_action.take() {
                                    self.last_action = Some(WorkbenchAction::OutputAction(act));
                                }
                                res
                            }
                            _ => EventResult::Consumed,
                        }
                    }
                } else if self.layout.secondary_sidebar_visible
                    && regions.secondary_sidebar.contains(mouse.position)
                {
                    let (tab_outline, tab_hierarchy, close_rect, content_rect) =
                        Self::secondary_sidebar_rects(regions.secondary_sidebar);
                    if close_rect.contains(mouse.position) {
                        self.layout.mutate(|l| l.secondary_sidebar_visible = false);
                        self.last_action = Some(WorkbenchAction::SecondarySidebarToggled(false));
                        return EventResult::Consumed;
                    }
                    if tab_outline.contains(mouse.position) {
                        self.layout.mutate(|l| {
                            l.active_secondary_sidebar_view = "outline".to_string();
                            l.secondary_sidebar_visible = true;
                        });
                        return EventResult::Consumed;
                    }
                    if tab_hierarchy.contains(mouse.position) {
                        self.layout.mutate(|l| {
                            l.active_secondary_sidebar_view = "hierarchy".to_string();
                            l.secondary_sidebar_visible = true;
                        });
                        return EventResult::Consumed;
                    }
                    if self.layout.active_secondary_sidebar_view == "hierarchy" {
                        let res = self.hierarchy_view.handle_event(event, content_rect);
                        if let Some(act) = self.hierarchy_view.last_action.take() {
                            self.last_action = Some(WorkbenchAction::HierarchyAction(act));
                        }
                        return res;
                    }
                    if content_rect.contains(mouse.position) {
                        let rel_y = mouse.position.y - (content_rect.min_y() + 4.0);
                        if rel_y >= 0.0 {
                            let idx = (rel_y / 22.0).floor() as usize;
                            let syms = self.secondary_sidebar_outline_items();
                            if let Some((_, line)) = syms.get(idx) {
                                if let Some(ed) = self.editor_area.active_editor_mut() {
                                    ed.jump_to_position_and_unfold(rustcode_editor::Position::new(
                                        *line as usize,
                                        0,
                                    ));
                                }
                                self.focused_region = FocusedRegion::Editor;
                            }
                        }
                        return EventResult::Consumed;
                    }
                    EventResult::Consumed
                } else if regions.editor_area.contains(mouse.position) {
                    self.focused_region = FocusedRegion::Editor;
                    let res = self.editor_area.handle_event(event, regions.editor_area);
                    self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                    // Sync editor gutter breakpoint clicks to debug session
                    if let Some(tab) = self.editor_area.model.current_group_ref().active_tab() {
                        let uri = tab.uri.clone();
                        if let Some(editor) = self.editor_area.active_editor_mut() {
                            if let Some(rustcode_editor::EditorGutterAction::BreakpointToggled { line, added }) = editor.last_gutter_action.take() {
                                if added {
                                    self.debug_sidebar.session.toggle_breakpoint(&uri, line as u32);
                                } else if let Some(bps) = self.debug_sidebar.session.breakpoints.get_mut(&uri) {
                                    bps.retain(|b| b.line != line as u32);
                                }
                                self.last_action = Some(WorkbenchAction::DebugAction(DebugViewAction::ToggleBreakpoint {
                                    path: uri,
                                    line: line as u32,
                                }));
                            }
                        }
                    }
                    res
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::KeyDown(key) => {
                // Global Debug shortcuts: F5, F10, F11, Shift+F11, Shift+F5, Ctrl+Shift+F5
                match key.key {
                    KeyCode::F5 | KeyCode::F10 | KeyCode::F11 => {
                        let res = self.debug_toolbar.handle_event(event, regions.editor_area);
                        if let Some(act) = self.debug_toolbar.last_action.take() {
                            self.last_action = Some(WorkbenchAction::DebugToolbar(act));
                            return EventResult::Consumed;
                        }
                        if res == EventResult::Consumed {
                            return res;
                        }
                    }
                    _ => {}
                }

                match self.focused_region {
                    FocusedRegion::Editor => {
                        let res = self.editor_area.handle_event(event, regions.editor_area);
                        self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                        res
                    }
                    FocusedRegion::Sidebar => {
                        let view_id = self.layout.active_sidebar_view.as_str();
                        if view_id == "scm" || view_id == "workbench.view.scm" {
                            let res = self.scm_view.handle_event(event, regions.sidebar);
                            self.sync_scm_status_bar();
                            if let Some(act) = self.scm_view.last_action.take() {
                                self.last_action = Some(WorkbenchAction::ScmAction(act));
                            }
                            res
                        } else if view_id == "debug" || view_id == "workbench.view.debug" {
                            self.debug_sidebar.handle_event(event, regions.sidebar)
                        } else if view_id == "search" || view_id == "workbench.view.search" {
                            let res = self.search_view.handle_event(event, regions.sidebar);
                            if let Some(act) = self.search_view.last_action.take() {
                                match act {
                                    SearchAction::OpenMatch { path, line, col } => {
                                        let path_str = path.to_string_lossy().to_string();
                                        let title = path.file_name().map_or("untitled", |n| n.to_str().unwrap_or(""));
                                        let content = std::fs::read_to_string(&path).unwrap_or_default();
                                        self.editor_area.open_file(&path_str, title, &content, true);
                                        if let Some(active_editor) = self.editor_area.active_editor_mut() {
                                            active_editor.set_primary_position(rustcode_editor::Position::new(line.saturating_sub(1), col));
                                        }
                                        self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                                        self.last_action = Some(WorkbenchAction::OpenFile(path));
                                    }
                                    SearchAction::ExecuteSearch => {
                                        self.search_view.execute_search(self.workspace.folders());
                                    }
                                    SearchAction::ClearSearch => {
                                        self.search_view.clear();
                                    }
                                    _ => {}
                                }
                            }
                            res
                        } else if view_id == "extensions" || view_id == "workbench.view.extensions" {
                            let res = self.extensions_view.handle_event(event, regions.sidebar);
                            self.process_extensions_action();
                            res
                        } else if view_id == "testing" || view_id == "workbench.view.testing" {
                            let res = self.testing_view.handle_event(event, regions.sidebar);
                            self.process_testing_action();
                            res
                        } else if view_id == "hierarchy" || view_id == "workbench.view.hierarchy" {
                            let res = self.hierarchy_view.handle_event(event, regions.sidebar);
                            if let Some(act) = self.hierarchy_view.last_action.take() {
                                self.last_action = Some(WorkbenchAction::HierarchyAction(act));
                            }
                            res
                        } else {
                            let res = self.explorer.handle_event(event, regions.sidebar);
                            self.process_explorer_action();
                            res
                        }
                    }
                    FocusedRegion::Panel => {
                        if !self.layout.panel_visible {
                            return EventResult::Ignored;
                        }
                        let content_rect = Self::panel_content_rect(regions.panel);
                        match self.layout.active_panel_tab.as_str() {
                            "debug_console" => {
                                let res = self.debug_console.handle_event(event, content_rect);
                                if let Some(act) = self.debug_console.last_action.take() {
                                    self.last_action = Some(WorkbenchAction::DebugAction(act));
                                }
                                res
                            }
                            "terminal" => {
                                let res = self.terminal_panel.handle_event(event, content_rect);
                                if let Some(act) = self.terminal_panel.last_action.take() {
                                    self.last_action = Some(WorkbenchAction::TerminalAction(act));
                                }
                                res
                            }
                            "output" => {
                                let res = self.output_panel.handle_event(event, content_rect);
                                if let Some(act) = self.output_panel.last_action.take() {
                                    self.last_action = Some(WorkbenchAction::OutputAction(act));
                                }
                                res
                            }
                            "problems" => self.problems_panel.handle_event(event, content_rect),
                            _ => EventResult::Ignored,
                        }
                    }
                    _ => EventResult::Ignored,
                }
            }
            InputEvent::KeyUp(_) => EventResult::Ignored,
            InputEvent::ScrollWheel(scroll) => {
                if self.layout.sidebar_visible && regions.sidebar.contains(scroll.position) {
                    let view_id = self.layout.active_sidebar_view.as_str();
                    if view_id == "scm" || view_id == "workbench.view.scm" {
                        self.scm_view.handle_event(event, regions.sidebar)
                    } else if view_id == "debug" || view_id == "workbench.view.debug" {
                        self.debug_sidebar.handle_event(event, regions.sidebar)
                    } else if view_id == "extensions" || view_id == "workbench.view.extensions" {
                        self.extensions_view.handle_event(event, regions.sidebar)
                    } else if view_id == "testing" || view_id == "workbench.view.testing" {
                        self.testing_view.handle_event(event, regions.sidebar)
                    } else if view_id == "hierarchy" || view_id == "workbench.view.hierarchy" {
                        self.hierarchy_view.handle_event(event, regions.sidebar)
                    } else {
                        self.explorer.handle_event(event, regions.sidebar)
                    }
                } else if self.layout.panel_visible && regions.panel.contains(scroll.position) {
                    let content_rect = Self::panel_content_rect(regions.panel);
                    if !content_rect.contains(scroll.position) {
                        return EventResult::Ignored;
                    }
                    match self.layout.active_panel_tab.as_str() {
                        "problems" => self.problems_panel.handle_event(event, content_rect),
                        "output" => self.output_panel.handle_event(event, content_rect),
                        "debug_console" => self.debug_console.handle_event(event, content_rect),
                        "terminal" => self.terminal_panel.handle_event(event, content_rect),
                        _ => EventResult::Ignored,
                    }
                } else if regions.editor_area.contains(scroll.position) {
                    self.editor_area.handle_event(event, regions.editor_area)
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::MouseMove(mouse) => {
                if let Some(sash) = self.active_sash {
                    self.update_sash_drag(sash, mouse.position, bounds);
                    return EventResult::Consumed;
                }
                if self.explorer.drag_state.is_some() {
                    let _ = self.explorer.handle_event(event, regions.sidebar);
                    return EventResult::Consumed;
                }
                let dragging = self.editor_area.is_minimap_or_sash_dragging();
                if dragging || regions.editor_area.contains(mouse.position) {
                    self.editor_area.handle_event(event, regions.editor_area)
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::MouseUp(mouse) => {
                if self.active_sash.take().is_some() {
                    return EventResult::Consumed;
                }
                if self.explorer.drag_state.is_some() {
                    if regions.editor_area.contains(mouse.position) {
                        if let Some(drag) = self.explorer.take_active_drag() {
                            if !drag.is_dir {
                                if let Some(gid) = self
                                    .editor_area
                                    .group_id_at_point(regions.editor_area, mouse.position)
                                {
                                    self.editor_area.focus_group(gid);
                                }
                                let uri = self
                                    .editor_area
                                    .find_open_tab_uri_for_path(&drag.source_path)
                                    .unwrap_or_else(|| {
                                        rustcode_platform::path_to_file_uri(&drag.source_path)
                                    });
                                let title = drag
                                    .source_path
                                    .file_name()
                                    .map_or("untitled", |n| n.to_str().unwrap_or(""));
                                let content =
                                    std::fs::read_to_string(&drag.source_path).unwrap_or_default();
                                self.editor_area.open_file(&uri, title, &content, false);
                                self.focused_region = FocusedRegion::Editor;
                                self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                                self.last_action = Some(WorkbenchAction::OpenFile(drag.source_path));
                            }
                            return EventResult::Consumed;
                        }
                    }
                    let res = self.explorer.handle_event(event, regions.sidebar);
                    self.process_explorer_action();
                    return res;
                }
                let any_dragging = self.editor_area.is_minimap_or_sash_dragging();
                let res = if any_dragging || self.focused_region == FocusedRegion::Editor {
                    self.editor_area.handle_event(event, regions.editor_area)
                } else {
                    EventResult::Ignored
                };
                self.editor_area.clear_all_minimap_dragging();
                if any_dragging {
                    EventResult::Consumed
                } else {
                    res
                }
            }
            InputEvent::ImePreedit { .. } => EventResult::Ignored,
            InputEvent::ImeCommit(text) => {
                if self.focused_region == FocusedRegion::Editor && !text.is_empty() {
                    let res = self.editor_area.handle_event(event, regions.editor_area);
                    if res == EventResult::Consumed {
                        self.editor_area.sync_breadcrumbs(Some(&self.workspace));
                    }
                    res
                } else {
                    EventResult::Ignored
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quickinput::{CommandPalette, PaletteItem};

    #[test]
    fn test_workbench_view_layout_and_regions() {
        let wb = WorkbenchView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let regions = wb.compute_regions(bounds);

        assert_eq!(regions.activity_bar.width(), 48.0);
        assert_eq!(regions.sidebar.width(), 280.0);
        assert_eq!(regions.status_bar.height(), 22.0);
        assert_eq!(regions.editor_area.width(), 1024.0 - 48.0 - 280.0);

        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let mut scene = Scene::new();
        wb.render(bounds, &theme, &font, &mut scene);

        assert!(!scene.quads().is_empty());
        assert!(scene.contains_text("EXPLORER"));
    }

    #[test]
    fn test_workbench_modal_command_palette_routing() {
        let mut wb = WorkbenchView::new();
        let mut palette = CommandPalette::new();
        palette.set_items(vec![PaletteItem::command(
            "workbench.action.toggleSidebar",
            "Toggle Sidebar",
            Some("View".to_string()),
            Some("Ctrl+B".to_string()),
        )]);

        wb.open_command_palette(CommandPaletteView::new(palette));
        assert!(wb.is_command_palette_open);

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let mut scene = Scene::new();
        wb.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("Toggle Sidebar"));

        // Press Enter to execute command
        let enter_ev = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            rustcode_platform::render::KeyCode::Enter,
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        let res = wb.handle_event(&enter_ev, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert!(!wb.is_command_palette_open);
        assert_eq!(
            wb.last_action,
            Some(WorkbenchAction::ExecuteCommand("workbench.action.toggleSidebar".to_string()))
        );
    }

    #[test]
    fn test_workbench_problems_panel_tabs_and_jump_to_location() {
        use rustcode_editor::cursor::{Position, Range};
        use rustcode_editor::viewmodel::EditorDiagnostic;

        let mut wb = WorkbenchView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        wb.update_problems(
            "src/main.rs",
            vec![EditorDiagnostic::error(
                Range::new(Position::new(12, 4), Position::new(12, 10)),
                "cannot find value `x` in this scope",
            )
            .with_source("rust-analyzer")
            .with_code("E0425")],
        );

        // Status bar item was updated with count badge
        let left_items = wb.status_bar.model.left_items();
        let prob_item = left_items.iter().find(|i| i.id == "workbench.status.problems").unwrap();
        assert_eq!(prob_item.text, "$(error) 1  $(warning) 0");

        // Toggle panel on and switch to problems
        wb.layout.panel_visible = true;
        wb.layout.set_panel_tab("problems");

        let mut scene = Scene::new();
        wb.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("PROBLEMS 1"));
        assert!(scene.contains_text("src/main.rs"));
        assert!(scene.contains_text("cannot find value `x` in this scope"));

        // Click diagnostic row in panel to jump to location
        let regions = wb.compute_regions(bounds);
        let diag_click_pt = Point::new(regions.panel.min_x() + 40.0, regions.panel.min_y() + 26.0 + 24.0 + 22.0 + 5.0);
        let click_ev = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            diag_click_pt,
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = wb.handle_event(&click_ev, bounds);
        assert_eq!(res, EventResult::Consumed);

        assert_eq!(
            wb.last_action,
            Some(WorkbenchAction::JumpToDiagnostic {
                uri: "src/main.rs".to_string(),
                line: 12,
                column: 4,
            })
        );
        assert_eq!(wb.focused_region, FocusedRegion::Editor);
    }

    #[test]
    fn test_workbench_gutter_breakpoint_sync_and_debug_console() {
        let mut wb = WorkbenchView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        // 1. Open file in editor area
        let file_path = "src/lib.rs";
        wb.editor_area.open_file(file_path, "lib.rs", "fn foo() {\n    let a = 1;\n    let b = 2;\n}\n", false);

        let regions = wb.compute_regions(bounds);
        let content_rect = wb.editor_area.content_rect(regions.editor_area);
        let gutter_click_x = content_rect.min_x() + 10.0;
        let gutter_click_y = content_rect.min_y() + font.line_height * 2.5; // Line 3
        let click_gutter = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(gutter_click_x, gutter_click_y),
            rustcode_platform::render::MouseButton::Left,
        ));

        let res = wb.handle_event(&click_gutter, bounds);
        assert_eq!(res, EventResult::Consumed);

        // Verify breakpoint was synced to Debug session state
        assert!(wb.debug_sidebar.session.breakpoints.contains_key(file_path));
        let bps = &wb.debug_sidebar.session.breakpoints[file_path];
        assert_eq!(bps.len(), 1);
        assert_eq!(bps[0].line, 3);
        assert!(matches!(
            wb.last_action,
            Some(WorkbenchAction::DebugAction(DebugViewAction::ToggleBreakpoint { line: 3, .. }))
        ));

        // 2. Switch panel to debug console and evaluate expression
        wb.layout.panel_visible = true;
        wb.layout.set_panel_tab("debug_console");
        wb.focused_region = FocusedRegion::Panel;
        wb.debug_console.input_focused = true;
        wb.debug_console.input_text = "10 * 20".to_string();

        let enter_key = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Enter,
            rustcode_platform::keybindings::Modifiers::default(),
        ));
        let res_eval = wb.handle_event(&enter_key, bounds);
        assert_eq!(res_eval, EventResult::Consumed);
        assert_eq!(
            wb.last_action,
            Some(WorkbenchAction::DebugAction(DebugViewAction::EvaluateExpression {
                expression: "10 * 20".to_string()
            }))
        );

        // 3. SCM Status Message rendering on failed commit
        wb.layout.set_sidebar_view("scm");
        wb.scm_view.commit_message.clear();
        let _ = wb.scm_view.execute_commit();
        assert!(wb.scm_view.status_message.is_some());

        let mut scm_scene = Scene::new();
        wb.scm_view.render(regions.sidebar, &theme, &font, &mut scm_scene);
        assert!(scm_scene.contains_text("Cannot commit empty message"));
    }

    #[test]
    fn test_workbench_view_extensions_sidebar_routing() {
        use crate::extensions_view::ExtensionsClickTarget;
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{KeyEvent, MouseButton, MouseEvent, ScrollEvent};

        let mut wb = WorkbenchView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        // 1. Click ActivityBar slot 4 (Extensions)
        let slot4_pt = Point::new(24.0, 4.0 * 48.0 + 24.0);
        let click_ext = InputEvent::MouseDown(MouseEvent::new(slot4_pt, MouseButton::Left));
        assert_eq!(wb.handle_event(&click_ext, bounds), EventResult::Consumed);
        assert_eq!(wb.layout.active_sidebar_view, "extensions");
        assert!(wb.layout.sidebar_visible);

        // 2. Render -> EXTENSIONS rendered, NOT EXPLORER
        let mut scene = Scene::new();
        wb.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("EXTENSIONS"));
        assert!(!scene.contains_text("EXPLORER"));

        // 3. Also test "workbench.view.extensions" alias
        wb.layout.active_sidebar_view = "workbench.view.extensions".to_string();
        let mut scene_alias = Scene::new();
        wb.render(bounds, &theme, &font, &mut scene_alias);
        assert!(scene_alias.contains_text("EXTENSIONS"));
        assert!(!scene_alias.contains_text("EXPLORER"));

        // 4. Click search box in sidebar -> focuses extensions_view search
        let search_rect = wb
            .extensions_view
            .click_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                ExtensionsClickTarget::SearchInput => Some(*r),
                _ => None,
            })
            .unwrap();
        let click_search = InputEvent::MouseDown(MouseEvent::new(
            Point::new(search_rect.min_x() + 6.0, search_rect.min_y() + 6.0),
            MouseButton::Left,
        ));
        assert_eq!(wb.handle_event(&click_search, bounds), EventResult::Consumed);
        assert_eq!(wb.focused_region, FocusedRegion::Sidebar);
        assert!(wb.extensions_view.search_focused);

        // 5. Type "rust" -> updates extensions_view.search_query & emits ExtensionsAction
        for ch in "rust".chars() {
            let key_ev = InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none()));
            assert_eq!(wb.handle_event(&key_ev, bounds), EventResult::Consumed);
        }
        assert_eq!(wb.extensions_view.search_query, "rust");
        assert_eq!(
            wb.last_action,
            Some(WorkbenchAction::ExtensionsAction(
                ExtensionsViewAction::Search("rust".to_string())
            ))
        );

        // 6. ScrollWheel inside sidebar -> clears filter first so list is scrollable, re-renders, scrolls extensions_view
        wb.extensions_view.apply_search_query("");
        wb.render(bounds, &theme, &font, &mut Scene::new());
        let regions = wb.compute_regions(bounds);
        // Use small bounds height to guarantee scrollable overflow
        let small_bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 260.0);
        wb.render(small_bounds, &theme, &font, &mut Scene::new());
        let small_regions = wb.compute_regions(small_bounds);
        let scroll_ev = InputEvent::ScrollWheel(ScrollEvent::new(
            Point::new(
                small_regions.sidebar.min_x() + 20.0,
                small_regions.sidebar.min_y() + 100.0,
            ),
            0.0,
            -80.0,
        ));
        assert_eq!(
            wb.handle_event(&scroll_ev, small_bounds),
            EventResult::Consumed
        );
        assert!(wb.extensions_view.scroll_top > 0.0);
        assert!(wb.explorer.last_action.is_none());
        let _ = regions;
    }

    #[test]
    fn test_workbench_minimap_drag_forwarding_and_release() {
        use rustcode_platform::render::{MouseButton, MouseEvent, MouseMoveEvent};

        let mut wb = WorkbenchView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let content: String = (0..200).map(|i| format!("let row_{} = {};\n", i, i)).collect();
        wb.editor_area
            .open_file("file:///workspace/big.rs", "big.rs", &content, false);

        let regions = wb.compute_regions(bounds);
        let content_rect = wb.editor_area.content_rect(regions.editor_area);
        let mm_rect = wb
            .editor_area
            .active_editor()
            .unwrap()
            .minimap_rect(content_rect)
            .unwrap();

        let down = InputEvent::MouseDown(MouseEvent::new(
            Point::new(mm_rect.min_x() + 20.0, mm_rect.max_y() - 30.0),
            MouseButton::Left,
        ));
        assert_eq!(wb.handle_event(&down, bounds), EventResult::Consumed);
        assert!(wb.editor_area.active_editor().unwrap().minimap_dragging);

        // Drag outside editor_area (into sidebar x=100) still updates scroll_top
        let drag_out = InputEvent::MouseMove(MouseMoveEvent {
            position: Point::new(100.0, mm_rect.min_y() + 20.0),
        });
        assert_eq!(wb.handle_event(&drag_out, bounds), EventResult::Consumed);

        // MouseUp outside editor_area ends minimap drag cleanly
        let up_out = InputEvent::MouseUp(MouseEvent::new(
            Point::new(100.0, mm_rect.min_y() + 20.0),
            MouseButton::Left,
        ));
        assert_eq!(wb.handle_event(&up_out, bounds), EventResult::Consumed);
        assert!(!wb.editor_area.active_editor().unwrap().minimap_dragging);
    }

    #[test]
    fn test_workbench_accessibility_tree_regions_focus_extensions_and_overlay() {
        let mut wb = WorkbenchView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let font = FontMetrics::default_monospace();

        wb.workspace = Workspace::from_folder(PathBuf::from("/workspace"));
        wb.editor_area.open_file(
            "file:///workspace/src/main.rs",
            "main.rs",
            "pub fn main() {\n    let x = 1;\n}\n",
            false,
        );
        wb.editor_area.sync_breadcrumbs(Some(&wb.workspace));

        let tree = wb.accessibility_tree(bounds, &font);
        assert_eq!(tree.root.role, AccessibleRole::Window);
        assert!(!tree.find_by_role(AccessibleRole::Toolbar).is_empty());
        assert!(!tree.find_by_role(AccessibleRole::Sidebar).is_empty());
        assert!(!tree.find_by_role(AccessibleRole::TabList).is_empty());
        assert!(!tree.find_by_role(AccessibleRole::Tab).is_empty());
        assert!(!tree.find_by_role(AccessibleRole::Breadcrumb).is_empty());
        assert!(!tree.find_by_role(AccessibleRole::BreadcrumbItem).is_empty());
        assert!(!tree.find_by_role(AccessibleRole::TextEditor).is_empty());
        assert!(!tree.find_by_role(AccessibleRole::ScrollBar).is_empty());
        assert!(!tree.find_by_role(AccessibleRole::StatusBar).is_empty());

        let focused = tree.focused_node().unwrap();
        assert_eq!(focused.role, AccessibleRole::TextEditor);

        // Switching focus to Sidebar -> Extensions view shows Extensions Search & Lists
        wb.layout.set_sidebar_view("workbench.view.extensions");
        wb.focused_region = FocusedRegion::Sidebar;
        wb.extensions_view.search_focused = true;
        let ext_tree = wb.accessibility_tree(bounds, &font);
        assert!(ext_tree.find_by_id("workbench.sidebar.extensions.search").is_some());
        assert!(ext_tree.find_by_id("workbench.sidebar.extensions.marketplace").is_some());
        assert_eq!(
            ext_tree.focused_node().unwrap().id,
            "workbench.sidebar.extensions.search"
        );

        // Opening CommandPalette -> Dialog & TextInput receive focus and win hit_test
        let mut palette = CommandPalette::new();
        palette.set_items(vec![PaletteItem::command("test.cmd", "Test Cmd", None, None)]);
        wb.open_command_palette(CommandPaletteView::new(palette));
        let pal_tree = wb.accessibility_tree(bounds, &font);
        assert!(!pal_tree.find_by_role(AccessibleRole::Dialog).is_empty());
        assert_eq!(pal_tree.focused_node().unwrap().id, "commandPalette.input");

        let regions = wb.compute_regions(bounds);
        let hit = pal_tree
            .hit_test(Point::new(
                regions.command_palette.min_x() + 20.0,
                regions.command_palette.min_y() + 16.0,
            ))
            .unwrap();
        assert_eq!(hit.id, "commandPalette.input");
    }

    #[test]
    fn test_minimap_drag_cleared_when_command_palette_opens_mid_drag() {
        use rustcode_platform::render::{MouseButton, MouseEvent, MouseMoveEvent};

        let mut wb = WorkbenchView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let content: String = (0..200).map(|i| format!("let row_{} = {};\n", i, i)).collect();
        wb.editor_area
            .open_file("file:///workspace/big.rs", "big.rs", &content, false);

        let regions = wb.compute_regions(bounds);
        let content_rect = wb.editor_area.content_rect(regions.editor_area);
        let mm_rect = wb
            .editor_area
            .active_editor()
            .unwrap()
            .minimap_rect(content_rect)
            .unwrap();

        let down = InputEvent::MouseDown(MouseEvent::new(
            Point::new(mm_rect.min_x() + 20.0, mm_rect.max_y() - 30.0),
            MouseButton::Left,
        ));
        assert_eq!(wb.handle_event(&down, bounds), EventResult::Consumed);
        assert!(wb.editor_area.active_editor().unwrap().minimap_dragging);

        // Open Command Palette mid-drag, send MouseUp, then close Command Palette
        let mut palette = CommandPalette::new();
        palette.set_items(vec![PaletteItem::command("test.cmd", "Test Cmd", None, None)]);
        wb.open_command_palette(CommandPaletteView::new(palette));
        assert!(!wb.editor_area.active_editor().unwrap().minimap_dragging);

        // Even if minimap_dragging was active when palette intercepted MouseUp, MouseUp clears it
        wb.editor_area.active_editor_mut().unwrap().minimap_dragging = true;
        let up = InputEvent::MouseUp(MouseEvent::new(
            Point::new(mm_rect.min_x() + 20.0, mm_rect.min_y() + 10.0),
            MouseButton::Left,
        ));
        let _ = wb.handle_event(&up, bounds);
        assert!(!wb.editor_area.active_editor().unwrap().minimap_dragging);

        wb.close_command_palette();
        let scroll_before = wb.editor_area.active_editor().unwrap().scroll_top;
        let move_after = InputEvent::MouseMove(MouseMoveEvent {
            position: Point::new(mm_rect.min_x() + 20.0, mm_rect.min_y() + 5.0),
        });
        let _ = wb.handle_event(&move_after, bounds);
        assert_eq!(wb.editor_area.active_editor().unwrap().scroll_top, scroll_before);
    }

    #[test]
    fn test_minimap_drag_cleared_on_mouseup_after_tab_switch() {
        use rustcode_platform::render::{MouseButton, MouseEvent, MouseMoveEvent};

        let mut wb = WorkbenchView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let content: String = (0..200).map(|i| format!("let row_{} = {};\n", i, i)).collect();
        wb.editor_area
            .open_file("file:///workspace/big.rs", "big.rs", &content, false);
        wb.editor_area
            .open_file("file:///workspace/other.rs", "other.rs", "fn other() {}\n", false);
        // Switch back to big.rs as active tab
        wb.editor_area
            .open_file("file:///workspace/big.rs", "big.rs", &content, false);

        let regions = wb.compute_regions(bounds);
        let content_rect = wb.editor_area.content_rect(regions.editor_area);
        let mm_rect = wb
            .editor_area
            .active_editor()
            .unwrap()
            .minimap_rect(content_rect)
            .unwrap();

        let down = InputEvent::MouseDown(MouseEvent::new(
            Point::new(mm_rect.min_x() + 20.0, mm_rect.max_y() - 30.0),
            MouseButton::Left,
        ));
        assert_eq!(wb.handle_event(&down, bounds), EventResult::Consumed);
        assert!(wb.editor_area.editors["file:///workspace/big.rs"].minimap_dragging);

        // Switch active tab mid-drag (e.g., via keybinding opening settings or another tab)
        wb.open_settings(None);
        assert_eq!(
            wb.editor_area.model.current_group_ref().active_tab().unwrap().uri,
            "rustcode://settings"
        );
        assert!(wb.editor_area.editors["file:///workspace/big.rs"].minimap_dragging);

        // Release MouseUp while Settings tab is active
        let up = InputEvent::MouseUp(MouseEvent::new(
            Point::new(mm_rect.min_x() + 20.0, mm_rect.min_y() + 10.0),
            MouseButton::Left,
        ));
        assert_eq!(wb.handle_event(&up, bounds), EventResult::Consumed);
        assert!(!wb.editor_area.editors["file:///workspace/big.rs"].minimap_dragging);

        // Switching back to big.rs and moving mouse must NOT scroll
        wb.editor_area.model.current_group().active_tab_index = Some(0);
        let scroll_before = wb.editor_area.active_editor().unwrap().scroll_top;
        let move_after = InputEvent::MouseMove(MouseMoveEvent {
            position: Point::new(mm_rect.min_x() + 20.0, mm_rect.min_y() + 5.0),
        });
        let _ = wb.handle_event(&move_after, bounds);
        assert_eq!(wb.editor_area.active_editor().unwrap().scroll_top, scroll_before);
    }

    #[test]
    fn test_extensions_accessibility_tree_matches_shared_layout_with_many_items_and_collapse() {
        use crate::extensions_view::InstalledExtensionItem;

        let mut wb = WorkbenchView::new();
        wb.layout.set_sidebar_view("workbench.view.extensions");
        for i in 0..4 {
            wb.extensions_view.installed.push(InstalledExtensionItem {
                id: format!("pub.ext{}", i),
                publisher: "pub".to_string(),
                name: format!("ext{}", i),
                version: "1.0.0".to_string(),
                display_name: format!("Ext {}", i),
                description: "Test extension".to_string(),
                enabled: true,
                root_dir: PathBuf::from(format!("/tmp/ext{}", i)),
            });
        }

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let regions = wb.compute_regions(bounds);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        wb.render(bounds, &theme, &font, &mut Scene::new());
        let layout = wb.extensions_view.compute_layout(regions.sidebar);
        let tree = wb.accessibility_tree(bounds, &font);

        // Installed items >2 shift marketplace list & items cleanly in both layout and accessibility tree
        let third_node = tree.find_by_id("extensions.installed.pub.ext3").unwrap();
        assert_eq!(
            third_node.bounds,
            layout.installed_rows[3].1.intersect(&layout.body_rect).unwrap()
        );
        let mkt_list = tree.find_by_id("workbench.sidebar.extensions.marketplace").unwrap();
        assert_eq!(mkt_list.bounds, layout.marketplace_list_rect);

        let first_mkt = &layout.marketplace_rows[0];
        let mkt_item_node = tree
            .find_by_id(&format!("extensions.marketplace.{}", first_mkt.0))
            .unwrap();
        assert_eq!(
            mkt_item_node.bounds,
            first_mkt.1.intersect(&layout.body_rect).unwrap()
        );

        // Collapsing installed section hides installed rows and shifts marketplace up identically
        wb.extensions_view.installed_collapsed = true;
        wb.render(bounds, &theme, &font, &mut Scene::new());
        let collapsed_layout = wb.extensions_view.compute_layout(regions.sidebar);
        let collapsed_tree = wb.accessibility_tree(bounds, &font);

        assert!(collapsed_tree.find_by_id("extensions.installed.pub.ext0").is_none());
        let collapsed_mkt_list = collapsed_tree
            .find_by_id("workbench.sidebar.extensions.marketplace")
            .unwrap();
        assert_eq!(collapsed_mkt_list.bounds, collapsed_layout.marketplace_list_rect);
        assert!(
            collapsed_mkt_list.bounds.min_y() < mkt_list.bounds.min_y(),
            "collapsing installed should shift marketplace list upward"
        );
    }

    #[test]
    fn test_workbench_and_window_non_finite_bounds_extensions_and_palette_no_panic_no_nan() {
        use crate::window::WorkbenchWindow;
        use rustcode_platform::render::{SceneNode, Size};

        let mut win = WorkbenchWindow::default();
        win.view.layout.sidebar_visible = true;
        win.view.layout.set_sidebar_view("extensions");
        win.open_command_palette();

        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for bad_bounds in [
            Rect::new(Point::new(f32::NAN, f32::NAN), Size::new(1024.0, 768.0)),
            Rect::new(Point::zero(), Size::new(f32::NAN, 768.0)),
            Rect::new(Point::zero(), Size::new(1024.0, f32::NAN)),
            Rect::from_xywh(0.0, 0.0, f32::INFINITY, 768.0),
            Rect::from_xywh(f32::NEG_INFINITY, 0.0, 1024.0, 768.0),
        ] {
            let mut scene = Scene::new();
            win.view.render(bad_bounds, &theme, &font, &mut scene);
            assert!(scene.nodes().is_empty());

            let tree = win.accessibility_tree(bad_bounds);
            for node in tree.to_flat_list() {
                assert!(
                    node.bounds.is_finite(),
                    "non-finite accessibility node bounds for {}: {:?}",
                    node.id,
                    node.bounds
                );
            }
        }

        // Verify valid render produces balanced clips and finite primitives when extensions + palette active
        let valid_bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let mut scene = Scene::new();
        win.view.render(valid_bounds, &theme, &font, &mut scene);
        let mut clip_depth = 0isize;
        for node in scene.nodes() {
            match node {
                SceneNode::PushClip(r) => {
                    clip_depth += 1;
                    assert!(r.is_finite());
                }
                SceneNode::PopClip => {
                    clip_depth -= 1;
                    assert!(clip_depth >= 0);
                }
                SceneNode::Quad(q) => assert!(q.bounds.is_finite()),
                SceneNode::Shadow(s) => assert!(s.bounds.is_finite()),
                SceneNode::GlyphRun(g) => {
                    assert!(g.origin.x.is_finite() && g.origin.y.is_finite());
                    assert!(g.bounds.is_finite());
                }
            }
        }
        assert_eq!(clip_depth, 0);
    }

    #[test]
    fn test_window_dispatch_webview_non_finite_scroll_and_f32_max_font_render_safe() {
        use crate::window::WorkbenchWindow;
        use rustcode_ext_host::webview::WebviewOptions;
        use rustcode_platform::render::{SceneNode, ScrollEvent};

        let assert_scene_finite = |scene: &Scene, label: &str| {
            let mut clip_depth = 0isize;
            for node in scene.nodes() {
                match node {
                    SceneNode::PushClip(r) => {
                        clip_depth += 1;
                        assert!(r.is_finite(), "{}: non-finite PushClip {:?}", label, r);
                    }
                    SceneNode::PopClip => {
                        clip_depth -= 1;
                        assert!(clip_depth >= 0, "{}: clip stack underflow", label);
                    }
                    SceneNode::Quad(q) => {
                        assert!(q.bounds.is_finite(), "{}: non-finite Quad {:?}", label, q.bounds);
                    }
                    SceneNode::Shadow(s) => {
                        assert!(s.bounds.is_finite(), "{}: non-finite Shadow {:?}", label, s.bounds);
                    }
                    SceneNode::GlyphRun(g) => {
                        assert!(
                            g.origin.x.is_finite() && g.origin.y.is_finite(),
                            "{}: non-finite GlyphRun origin {:?}",
                            label,
                            g.origin
                        );
                        assert!(
                            g.bounds.is_finite(),
                            "{}: non-finite GlyphRun bounds {:?}",
                            label,
                            g.bounds
                        );
                    }
                }
            }
            assert_eq!(clip_depth, 0, "{}: unbalanced clip stack", label);
        };

        // 1. WorkbenchWindow::dispatch_event with active webview tab + Inf/NaN ScrollWheel
        let mut win = WorkbenchWindow::default();
        win.view.editor_area.open_webview(
            "wv_1",
            "Preview",
            "testPreview",
            "<h1>Hello</h1><p>World</p><button>Action</button>",
            WebviewOptions::default(),
            None,
        );
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for bad_dy in [f32::INFINITY, f32::NAN, f32::NEG_INFINITY] {
            win.dispatch_event(
                &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(500.0, 300.0), 0.0, bad_dy)),
                bounds,
            );
            let wv = win.view.editor_area.webviews.get("wv_1").unwrap();
            assert!(wv.scroll_top.is_finite(), "scroll_top poisoned for dy={}", bad_dy);
        }

        let mut wv_scene = Scene::new();
        win.view.render(bounds, &theme, &font, &mut wv_scene);
        assert_scene_finite(&wv_scene, "webview_after_bad_scroll");
        assert!(wv_scene.contains_text("Hello"));

        // ProblemsPanel inside WorkbenchWindow with NEG_INFINITY ScrollWheel
        let mut panel_scene = Scene::new();
        win.view.layout.panel_visible = true;
        win.view.problems_panel.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(500.0, 650.0), 0.0, f32::NEG_INFINITY)),
            Rect::from_xywh(0.0, 550.0, 1024.0, 200.0),
        );
        assert!(win.view.problems_panel.scroll_top.is_finite());
        win.view.render(bounds, &theme, &font, &mut panel_scene);
        assert_scene_finite(&panel_scene, "problems_after_neg_inf_scroll");

        // 2. f32::MAX FontMetrics rejected cleanly by is_finite_font across Layer 3 views
        let max_font = FontMetrics {
            font_family: "monospace".to_string(),
            font_size: f32::MAX,
            line_height: f32::MAX,
            char_width: f32::MAX,
            ascent: f32::MAX,
            descent: 0.0,
        };
        assert!(!max_font.is_finite());

        let mut ext_view = crate::extensions_view::ExtensionsSidebarView::new();
        let mut ext_scene = Scene::new();
        ext_view.search_query = "rust".to_string();
        ext_view.render(bounds, &theme, &max_font, &mut ext_scene);
        assert_scene_finite(&ext_scene, "extensions_max_font");

        let settings_view = crate::settings_ui::SettingsUiView::default();
        let mut settings_scene = Scene::new();
        settings_view.render(bounds, &theme, &max_font, &mut settings_scene);
        assert_scene_finite(&settings_scene, "settings_max_font");

        let mut palette = crate::quickinput::CommandPalette::new();
        palette.set_items(vec![crate::quickinput::PaletteItem::command(
            "workbench.action.openSettings",
            "Open Settings",
            None,
            None,
        )]);
        let palette_view = crate::quickinput::CommandPaletteView::new(palette);
        let mut palette_scene = Scene::new();
        palette_view.render(bounds, &theme, &max_font, &mut palette_scene);
        assert_scene_finite(&palette_scene, "palette_max_font");

        win.view.layout.sidebar_visible = true;
        win.view.layout.set_sidebar_view("extensions");
        win.open_command_palette();
        let mut wb_scene = Scene::new();
        win.view.render(bounds, &theme, &max_font, &mut wb_scene);
        assert_scene_finite(&wb_scene, "workbench_max_font");
    }

    #[test]
    fn test_workbench_panel_terminal_and_output_routing_and_accessibility() {
        use rustcode_platform::accessibility::AccessibleRole;
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{KeyEvent, ScrollEvent};

        let mut view = WorkbenchView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        view.layout.set_panel_tab("terminal");
        view.focused_region = FocusedRegion::Panel;

        for ch in "echo wb_term".chars() {
            assert_eq!(
                view.handle_event(
                    &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                    bounds,
                ),
                EventResult::Consumed
            );
        }
        assert_eq!(
            view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
                bounds,
            ),
            EventResult::Consumed
        );

        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("wb_term"));

        let tree = view.accessibility_tree(bounds, &font);
        assert!(tree.find_by_id("workbench.panel.terminal").is_some());
        let focused = tree.focused_node().unwrap();
        assert!(focused.id.starts_with("terminal.pane."));
        assert_eq!(focused.role, AccessibleRole::TextInput);

        // Switch to Output tab and test ScrollWheel routing inside regions.panel
        view.layout.set_panel_tab("output");
        view.output_panel.ensure_channel("out_1", "Build");
        for i in 0..40 {
            view.output_panel
                .append_line("out_1", &format!("wb_out_{:02}", i));
        }
        let mut out_scene = Scene::new();
        view.render(bounds, &theme, &font, &mut out_scene);
        assert!(out_scene.contains_text("wb_out_39"));

        let regions = view.compute_regions(bounds);
        let panel_scroll_pt = Point::new(
            regions.panel.min_x() + 100.0,
            regions.panel.min_y() + 60.0,
        );
        assert_eq!(
            view.handle_event(
                &InputEvent::ScrollWheel(ScrollEvent::new(panel_scroll_pt, 0.0, 2000.0)),
                bounds,
            ),
            EventResult::Consumed
        );
        assert!(!view.output_panel.auto_scroll);

        let out_tree = view.accessibility_tree(bounds, &font);
        assert!(out_tree.find_by_id("workbench.panel.output").is_some());

        view.toggle_panel();
        assert!(!view.layout.panel_visible);
        assert_eq!(view.focused_region, FocusedRegion::Editor);
    }

    #[test]
    fn test_workbench_split_editor_accessibility_tree_and_gutter_breakpoint() {
        use crate::editor_area::SplitDirection;
        use rustcode_platform::render::{MouseButton, MouseEvent};

        let mut view = WorkbenchView::new();
        view.layout.sidebar_visible = false;
        view.layout.panel_visible = false;
        let bounds = Rect::from_xywh(0.0, 0.0, 1000.0, 600.0);
        let font = FontMetrics::default_monospace();

        view.editor_area
            .open_file("file:///demo/main.rs", "main.rs", "fn main() {\n    let x = 1;\n}\n", false);
        let gid1 = view.editor_area.split_active_group(SplitDirection::Horizontal);
        view.editor_area
            .open_file("file:///demo/other.rs", "other.rs", "fn other() {}\n", false);

        // Diagnostic updates propagate to all split editors for the URI
        view.update_problems(
            "file:///demo/main.rs",
            vec![rustcode_editor::viewmodel::EditorDiagnostic::error(
                rustcode_editor::Range::new(
                    rustcode_editor::Position::new(0, 0),
                    rustcode_editor::Position::new(0, 2),
                ),
                "err",
            )],
        );
        assert_eq!(
            view.editor_area
                .editor_for_group(0, "file:///demo/main.rs")
                .unwrap()
                .diagnostics
                .len(),
            1
        );

        let tree = view.accessibility_tree(bounds, &font);
        assert!(tree.find_by_id("workbench.editorGroup.0").is_some());
        assert!(tree
            .find_by_id(&format!("workbench.editorGroup.{}", gid1))
            .is_some());
        let focused = tree.focused_node().unwrap();
        assert_eq!(focused.id, "workbench.editor.text.file:///demo/other.rs");

        // Gutter breakpoint click inside split group 1
        let regions = view.compute_regions(bounds);
        let rects = view.editor_area.model.compute_group_rects(regions.editor_area);
        let r1 = rects.iter().find(|(id, _)| *id == gid1).unwrap().1;
        let content_r1 = view.editor_area.content_rect_for_group(gid1, r1);
        let gutter_click = InputEvent::MouseDown(MouseEvent::new(
            Point::new(content_r1.min_x() + 6.0, content_r1.min_y() + 6.0),
            MouseButton::Left,
        ));
        assert_eq!(view.handle_event(&gutter_click, bounds), EventResult::Consumed);
        assert!(view
            .debug_sidebar
            .session
            .breakpoints
            .get("file:///demo/other.rs")
            .is_some_and(|bps| !bps.is_empty()));
    }

    #[test]
    fn test_workbench_palette_open_file_dedups_uri_and_clamps_utf16_column() {
        use crate::quickinput::{CommandPalette, PaletteItem, PaletteItemKind};

        let tmp = std::env::temp_dir().join(format!("rc_wb_open_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp).unwrap();
        let file_path = tmp.join("utf16.txt");
        std::fs::write(&file_path, "line0\n🚀x\nline2\n").unwrap();
        let uri = rustcode_platform::path_to_file_uri(&file_path);

        let mut view = WorkbenchView::new();
        view.editor_area
            .open_file(&uri, "utf16.txt", "line0\n🚀x\nline2\n", false);

        // Open same path via PaletteAction::OpenFile with 1-based line=2, col=99999
        let mut palette = CommandPalette::new();
        palette.set_items(vec![PaletteItem {
            command_id: "open:u".to_string(),
            label: "utf16.txt".to_string(),
            category: None,
            keybinding: None,
            path: Some(file_path.clone()),
            line: Some(2),
            column: Some(99999),
            item_kind: PaletteItemKind::File,
        }]);
        palette.open_mode(QuickInputMode::File);
        view.open_command_palette(CommandPaletteView::new(palette));

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let enter_ev = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Enter,
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        assert_eq!(view.handle_event(&enter_ev, bounds), EventResult::Consumed);

        // Did NOT duplicate tab
        assert_eq!(view.editor_area.model.current_group_ref().tabs.len(), 1);
        // Clamped to line 1, UTF-16 col 3 ("🚀x")
        assert_eq!(
            view.editor_area.active_editor().unwrap().primary_position(),
            rustcode_editor::Position::new(1, 3)
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_workbench_view_testing_sidebar_routing_and_overlays() {
        use rustcode_ext_host::testing::ExtHostTestManager;
        use rustcode_ext_host::window::{
            ExtNotificationState, ExtPromptKind, ExtPromptState, NotificationActionItem,
            NotificationSeverity,
        };
        use rustcode_platform::render::{MouseButton, MouseEvent};
        use serde_json::json;

        let tmp = std::env::temp_dir().join(format!("rc_wb_m3_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp).unwrap();
        let spec_path = tmp.join("spec.rs");
        std::fs::write(&spec_path, "#[test]\nfn demo_test() {}\n").unwrap();
        let spec_uri = rustcode_platform::path_to_file_uri(&spec_path);

        let mut view = WorkbenchView::new();
        let mgr = ExtHostTestManager::new();
        mgr.handle_rpc_call(
            "tests.createController",
            &json!({ "id": "c1", "label": "Suite" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.updateItems",
            &json!({
                "controllerId": "c1",
                "items": [{ "id": "demo_test", "label": "demo_test", "uri": spec_uri }]
            }),
        )
        .unwrap();
        view.testing_view.sync_from_manager(&mgr);

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        // Click ActivityBar slot 5 -> activates testing sidebar
        let slot5 = Point::new(24.0, 5.0 * 48.0 + 24.0);
        assert_eq!(
            view.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(slot5, MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        assert_eq!(view.layout.active_sidebar_view, "testing");

        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("TESTING"));
        assert!(scene.contains_text("demo_test"));

        // Clicking test item opens file in editor_area and emits OpenFile
        let tree = view.accessibility_tree(bounds, &font);
        let item_bounds = tree
            .find_by_role(AccessibleRole::TreeItem)
            .into_iter()
            .find(|n| n.name == "demo_test")
            .unwrap()
            .bounds;
        assert_eq!(
            view.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(
                    Point::new(item_bounds.min_x() + 10.0, item_bounds.min_y() + 8.0),
                    MouseButton::Left
                )),
                bounds
            ),
            EventResult::Consumed
        );
        assert!(view.editor_area.active_editor().is_some());
        assert_eq!(view.last_action, Some(WorkbenchAction::OpenFile(spec_path)));

        // Non-blocking toast renders alongside modal prompt & prompt intercepts Escape
        view.notifications.sync_notifications(&[ExtNotificationState {
            id: "n1".to_string(),
            severity: NotificationSeverity::Info,
            message: "ToastMsg".to_string(),
            detail: None,
            modal: false,
            items: vec![NotificationActionItem {
                title: "OK".to_string(),
                is_close_affordance: false,
                raw: json!("OK"),
            }],
            selected_action: None,
            dismissed: false,
            resolved: false,
        }]);
        view.prompt_overlay.sync_prompts(&[ExtPromptState {
            id: "p1".to_string(),
            client_id: None,
            kind: ExtPromptKind::InputBox {
                prompt: Some("Enter val".to_string()),
                place_holder: None,
                value: None,
                password: false,
                title: None,
                has_validate_input: false,
                options: json!({}),
            },
            response: None,
            resolved: false,
        }]);

        let mut overlay_scene = Scene::new();
        view.render(bounds, &theme, &font, &mut overlay_scene);
        assert!(overlay_scene.contains_text("ToastMsg"));
        assert!(overlay_scene.contains_text("Enter val"));

        assert_eq!(
            view.handle_event(
                &InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
                    KeyCode::Escape,
                    rustcode_platform::keybindings::Modifiers::none()
                )),
                bounds
            ),
            EventResult::Consumed
        );
        assert!(!view.is_prompt_open());
        assert_eq!(
            view.last_action,
            Some(WorkbenchAction::PromptAction(PromptUiAction::Cancel {
                id: "p1".to_string()
            }))
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_workbench_view_no_sample_defaults_and_idle_debug_toolbar() {
        let wb = WorkbenchView::new();
        assert!(!wb.debug_toolbar.is_visible);
        assert!(!wb.debug_toolbar.is_running());
        assert!(!wb.debug_sidebar.session.is_active);
        assert!(wb.debug_sidebar.session.threads.is_empty());
        assert!(wb.debug_sidebar.session.call_stack.is_empty());
        assert!(wb.debug_sidebar.session.scopes.is_empty());
        assert!(wb.debug_sidebar.session.variables.is_empty());
        assert!(wb.debug_sidebar.session.breakpoints.is_empty());

        assert!(wb.scm_view.git_backend.is_none());
        assert!(wb.scm_view.branch_name().is_none());
        assert!(wb.scm_view.repo.staged.is_empty());
        assert!(wb.scm_view.repo.changes.is_empty());
        assert!(wb.scm_view.repo.untracked.is_empty());
        assert!(wb.scm_view.repo.commits.is_empty());
        assert!(wb
            .status_bar
            .model
            .item(crate::statusbar::STATUS_GIT_BRANCH_ID)
            .is_none());
    }

    #[test]
    fn test_workbench_scm_git_cli_and_statusbar_branch_readout() {
        let tmp =
            std::env::temp_dir().join(format!("rc_wb_git_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp).unwrap();

        let run_git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&tmp)
                .args(args)
                .output()
                .expect("git should run");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };

        run_git(&["init", "-b", "feature/ui-wire"]);
        run_git(&["config", "user.name", "RustCode Test"]);
        run_git(&["config", "user.email", "test@rustcode.local"]);

        let file_path = tmp.join("lib.rs");
        std::fs::write(&file_path, "pub fn one() -> i32 { 1 }\n").unwrap();
        run_git(&["add", "lib.rs"]);
        run_git(&["commit", "-m", "initial"]);

        std::fs::write(&file_path, "pub fn one() -> i32 { 2 }\n").unwrap();

        let mut wb = WorkbenchView::new();
        assert!(wb.open_workspace_git(&tmp).unwrap());
        let branch_item = wb
            .status_bar
            .model
            .item(crate::statusbar::STATUS_GIT_BRANCH_ID)
            .unwrap();
        assert_eq!(branch_item.text, "$(git-branch) feature/ui-wire*");

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let mut scene = Scene::new();
        wb.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("$(git-branch) feature/ui-wire*"));

        wb.scm_view.stage_all().unwrap();
        wb.scm_view.commit_with_message("update lib").unwrap();
        wb.sync_scm_status_bar();

        let clean_item = wb
            .status_bar
            .model
            .item(crate::statusbar::STATUS_GIT_BRANCH_ID)
            .unwrap();
        assert_eq!(clean_item.text, "$(git-branch) feature/ui-wire");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_workbench_ime_commit_forwards_to_active_editor() {
        let mut wb = WorkbenchView::new();
        wb.editor_area
            .open_file("file:///workspace/ime.txt", "ime.txt", "hello ", false);
        if let Some(ed) = wb.editor_area.active_editor_mut() {
            ed.set_primary_position(rustcode_editor::Position::new(0, 6));
        }

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let preedit = InputEvent::ImePreedit {
            text: "ni".to_string(),
            cursor: Some((0, 2)),
        };
        assert_eq!(wb.handle_event(&preedit, bounds), EventResult::Ignored);

        let commit = InputEvent::ImeCommit("你好".to_string());
        assert_eq!(wb.handle_event(&commit, bounds), EventResult::Consumed);
        assert_eq!(
            wb.editor_area.active_editor().unwrap().text(),
            "hello 你好"
        );
        assert!(wb
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .unwrap()
            .is_dirty);
    }

    #[test]
    fn test_workbench_view_phase9_m3_layout_sashes_zoom_zen_context_and_dnd() {
        use rustcode_platform::render::{MouseButton, MouseEvent, MouseMoveEvent};
        use crate::explorer::FileTreeNode;

        let mut wb = WorkbenchView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1280.0, 800.0);

        // 1. Secondary Sidebar + TitleBar + PanelPosition::{Left, Right, Bottom} + Zoom + Zen Mode
        wb.layout.mutate(|l| {
            l.toggle_title_bar();
            l.toggle_secondary_sidebar();
            l.panel_visible = true;
            l.set_panel_position(PanelPosition::Right);
        });
        let reg = wb.compute_regions(bounds);
        assert_eq!(reg.title_bar.height(), TitleBarView::HEIGHT);
        assert!(reg.secondary_sidebar.width() > 0.0);
        assert!(reg.panel.min_x() >= reg.editor_area.max_x() - 0.1);
        assert!(reg.secondary_sidebar.min_x() >= reg.panel.max_x() - 0.1);

        // Sash drag on Sidebar resizes sidebar_width
        let orig_sb_w = wb.layout.sidebar_width;
        let sb_sash_x = reg.sidebar.max_x();
        assert_eq!(
            wb.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(Point::new(sb_sash_x, 200.0), MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        assert_eq!(
            wb.handle_event(
                &InputEvent::MouseMove(MouseMoveEvent {
                    position: Point::new(sb_sash_x + 40.0, 200.0)
                }),
                bounds
            ),
            EventResult::Consumed
        );
        wb.handle_event(
            &InputEvent::MouseUp(MouseEvent::new(
                Point::new(sb_sash_x + 40.0, 200.0),
                MouseButton::Left
            )),
            bounds,
        );
        assert!((wb.layout.sidebar_width - (orig_sb_w + 40.0)).abs() < 1.0);

        // Zoom scales effective font metrics
        let base_font = FontMetrics::default_monospace();
        assert_eq!(wb.effective_font_metrics(&base_font), base_font);
        wb.layout.zoom_in();
        let zoomed_font = wb.effective_font_metrics(&base_font);
        assert!(zoomed_font.font_size > base_font.font_size);

        // Zen Mode hides chrome and restores on exit
        wb.layout.toggle_zen_mode();
        let zen_reg = wb.compute_regions(bounds);
        assert_eq!(zen_reg.editor_area, bounds);
        wb.layout.toggle_zen_mode();
        let restored_reg = wb.compute_regions(bounds);
        assert!(restored_reg.title_bar.height() > 0.0);
        assert!(restored_reg.sidebar.width() > 0.0);
        assert!(restored_reg.secondary_sidebar.width() > 0.0);

        // 2. Right-click opens WorkbenchContextMenu in editor area and selects command
        let ed_pt = Point::new(restored_reg.editor_area.min_x() + 80.0, restored_reg.editor_area.min_y() + 80.0);
        assert_eq!(
            wb.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(ed_pt, MouseButton::Right)),
                bounds
            ),
            EventResult::Consumed
        );
        assert!(wb.context_menu.is_open);

        // Render full M3 chrome + overlays
        let mut scene = Scene::new();
        wb.render(bounds, &WorkbenchThemeStyles::dark_default(), &base_font, &mut scene);
        assert!(scene.contains_text("File"));
        assert!(scene.contains_text("OUTLINE"));
        assert!(scene.contains_text("Go to Definition"));

        // 3. Explorer -> Editor D&D opens dragged file in target editor group
        let tmp = std::env::temp_dir().join(format!("rustcode_wb_dnd_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let drag_file = tmp.join("dragged.rs");
        std::fs::write(&drag_file, "fn dragged_fn() {}\n").unwrap();
        wb.explorer.model.set_roots(vec![FileTreeNode::from_path(tmp.clone())]);
        wb.explorer.model.expanded_paths.insert(tmp.clone());
        let reg2 = wb.compute_regions(bounds);
        wb.render(bounds, &WorkbenchThemeStyles::dark_default(), &base_font, &mut scene);

        // Close context menu first
        wb.context_menu.close();
        let file_y = reg2.sidebar.min_y() + 28.0 + 22.0 + 8.0; // header(28) + root(22) + first child
        let start = Point::new(reg2.sidebar.min_x() + 20.0, file_y);
        let drop_pt = Point::new(reg2.editor_area.min_x() + 100.0, reg2.editor_area.min_y() + 120.0);
        wb.handle_event(&InputEvent::MouseDown(MouseEvent::new(start, MouseButton::Left)), bounds);
        wb.handle_event(&InputEvent::MouseMove(MouseMoveEvent { position: drop_pt }), bounds);
        wb.handle_event(&InputEvent::MouseUp(MouseEvent::new(drop_pt, MouseButton::Left)), bounds);
        assert!(wb.editor_area.find_open_tab_uri_for_path(&drag_file).is_some());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
