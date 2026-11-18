//! GPUI WorkbenchView coordinator composing all UI shell regions into a cohesive presentation tree.

use crate::activitybar::{ActivityBarAction, ActivityBarView};
use crate::debug::{
    DebugConsoleView, DebugSidebarView, DebugToolbarAction, DebugToolbarView, DebugViewAction,
};
use crate::editor_area::EditorAreaView;
use crate::explorer::{ExplorerAction, FileExplorerView};
use crate::extensions_view::{ExtensionsSidebarView, ExtensionsViewAction};
use crate::layout::WorkbenchLayout;
use crate::problems::{ProblemsAction, ProblemsPanel};
use crate::quickinput::{CommandPaletteView, PaletteAction};
use crate::scm::{ScmAction, ScmView};
use crate::search::{SearchAction, SearchView};
use crate::statusbar::{StatusBarAction, StatusBarAlignment, StatusBarItem, StatusBarView};
use crate::workspace::Workspace;
use parking_lot::RwLock;
use rustcode_platform::configuration::ConfigurationService;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Rgba, Scene, View,
    WorkbenchThemeStyles,
};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedRegion {
    Editor,
    Sidebar,
    Panel,
    CommandPalette,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchAction {
    ExecuteCommand(String),
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
    SidebarToggled(bool),
    PanelToggled(bool),
}

pub struct WorkbenchView {
    pub layout: WorkbenchLayout,
    pub activity_bar: ActivityBarView,
    pub explorer: FileExplorerView,
    pub scm_view: ScmView,
    pub search_view: SearchView,
    pub extensions_view: ExtensionsSidebarView,
    pub workspace: Workspace,
    pub debug_sidebar: DebugSidebarView,
    pub debug_toolbar: DebugToolbarView,
    pub debug_console: DebugConsoleView,
    pub editor_area: EditorAreaView,
    pub status_bar: StatusBarView,
    pub problems_panel: ProblemsPanel,
    pub panel_tab_bounds: RwLock<Vec<(String, Rect)>>,
    pub command_palette: Option<CommandPaletteView>,
    pub is_command_palette_open: bool,
    pub focused_region: FocusedRegion,
    pub last_action: Option<WorkbenchAction>,
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
            layout: WorkbenchLayout::new(),
            activity_bar: ActivityBarView::default(),
            explorer: FileExplorerView::default(),
            scm_view: ScmView::default(),
            search_view: SearchView::default(),
            extensions_view: ExtensionsSidebarView::default(),
            workspace: Workspace::default(),
            debug_sidebar: DebugSidebarView::default(),
            debug_toolbar: DebugToolbarView::default(),
            debug_console: DebugConsoleView::default(),
            editor_area: EditorAreaView::default(),
            status_bar,
            problems_panel: ProblemsPanel::new(),
            panel_tab_bounds: RwLock::new(Vec::new()),
            command_palette: None,
            is_command_palette_open: false,
            focused_region: FocusedRegion::Editor,
            last_action: None,
        }
    }

    pub fn update_problems(&mut self, uri: &str, diags: Vec<rustcode_editor::viewmodel::EditorDiagnostic>) {
        if let Some(editor) = self.editor_area.editors.get_mut(uri) {
            editor.set_diagnostics(diags.clone());
        }
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

    pub fn toggle_panel(&mut self) {
        self.layout.toggle_panel();
        self.last_action = Some(WorkbenchAction::PanelToggled(self.layout.panel_visible));
    }

    pub fn open_settings(&mut self, config: Option<ConfigurationService>) {
        self.editor_area.open_settings(config);
        self.focused_region = FocusedRegion::Editor;
    }

    pub fn open_notebook(&mut self, uri: &str, title: &str, content: &str, is_preview: bool) {
        self.editor_area.open_notebook(uri, title, content, is_preview);
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
        self.focused_region = FocusedRegion::Editor;
    }

    fn process_extensions_action(&mut self) {
        if let Some(act) = self.extensions_view.last_action.take() {
            self.last_action = Some(WorkbenchAction::ExtensionsAction(act));
        }
    }

    // --- Layout Region Calculations ---

    pub fn compute_regions(&self, bounds: Rect) -> WorkbenchRegions {
        let status_bar_h = if self.layout.status_bar_visible { 22.0 } else { 0.0 };
        let status_bar = Rect::from_xywh(
            bounds.min_x(),
            bounds.max_y() - status_bar_h,
            bounds.width(),
            status_bar_h,
        );

        let main_h = bounds.height() - status_bar_h;
        let act_bar_w = if self.layout.activity_bar_visible { 48.0 } else { 0.0 };
        let activity_bar = Rect::from_xywh(bounds.min_x(), bounds.min_y(), act_bar_w, main_h);

        let sidebar_w = if self.layout.sidebar_visible {
            self.layout
                .sidebar_width
                .min((bounds.width() - act_bar_w - 100.0).max(0.0))
        } else {
            0.0
        };
        let sidebar = Rect::from_xywh(
            bounds.min_x() + act_bar_w,
            bounds.min_y(),
            sidebar_w,
            main_h,
        );

        let rem_w = (bounds.width() - act_bar_w - sidebar_w).max(0.0);
        let panel_h = if self.layout.panel_visible {
            self.layout.panel_height.min((main_h - 100.0).max(0.0))
        } else {
            0.0
        };
        let panel = Rect::from_xywh(
            bounds.min_x() + act_bar_w + sidebar_w,
            bounds.min_y() + main_h - panel_h,
            rem_w,
            panel_h,
        );

        let editor_area = Rect::from_xywh(
            bounds.min_x() + act_bar_w + sidebar_w,
            bounds.min_y(),
            rem_w,
            main_h - panel_h,
        );

        let pal_w = (600.0f32).min(bounds.width() - 40.0);
        let pal_h = (320.0f32).min(bounds.height() - 80.0);
        let command_palette = Rect::from_xywh(
            bounds.min_x() + (bounds.width() - pal_w) / 2.0,
            bounds.min_y() + 40.0,
            pal_w,
            pal_h,
        );

        WorkbenchRegions {
            activity_bar,
            sidebar,
            editor_area,
            panel,
            status_bar,
            command_palette,
        }
    }
}

pub struct WorkbenchRegions {
    pub activity_bar: Rect,
    pub sidebar: Rect,
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
        let regions = self.compute_regions(bounds);

        // 1. Overall base background
        scene.push_quad(Quad::new(bounds).with_background(theme.editor_background));

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
            } else {
                self.explorer.render(regions.sidebar, theme, font, scene);
            }
        }

        // 4. Editor area & Floating Debug Toolbar
        self.editor_area.render(regions.editor_area, theme, font, scene);

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
            let content_rect = Rect::from_xywh(
                regions.panel.min_x(),
                regions.panel.min_y() + tab_h,
                regions.panel.width(),
                (regions.panel.height() - tab_h).max(0.0),
            );

            if self.layout.active_panel_tab == "problems" {
                self.problems_panel.render(content_rect, theme, font, scene);
            } else if self.layout.active_panel_tab == "debug_console" {
                self.debug_console.render(content_rect, theme, font, scene);
            } else {
                scene.push_quad(Quad::new(content_rect).with_background(theme.panel_bg));
                scene.push_text(GlyphRun::new(
                    if self.layout.active_panel_tab == "terminal" {
                        "Terminal session active."
                    } else {
                        "Output console empty."
                    }.to_string(),
                    Point::new(content_rect.min_x() + 16.0, content_rect.min_y() + 12.0),
                    Rect::from_xywh(content_rect.min_x() + 16.0, content_rect.min_y() + 12.0, 300.0, font.line_height),
                    theme.sidebar_header_fg,
                    font.font_size * 0.9,
                ));
            }
        }

        // 6. Status bar
        if self.layout.status_bar_visible {
            self.status_bar.render(regions.status_bar, theme, font, scene);
        }

        // 7. Command Palette modal overlay
        if self.is_command_palette_open {
            if let Some(palette) = &self.command_palette {
                // Dim backdrop
                scene.push_quad(Quad::new(bounds).with_background(Rgba::new(0.0, 0.0, 0.0, 0.35)));
                palette.render(regions.command_palette, theme, font, scene);
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let regions = self.compute_regions(bounds);

        // Modal command palette intercept
        if self.is_command_palette_open {
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
                            let path_str = path_clone.to_string_lossy().to_string();
                            let title = path_clone.file_name().map_or("untitled", |n| n.to_str().unwrap_or(""));
                            let content = std::fs::read_to_string(&path_clone).unwrap_or_default();
                            self.editor_area.open_file(&path_str, title, &content, false);
                            if let Some(active_editor) = self.editor_area.active_editor_mut() {
                                if let Some(l) = line_val {
                                    active_editor.set_primary_position(rustcode_editor::Position::new(l.saturating_sub(1), col_val.unwrap_or(0)));
                                }
                            }
                            self.last_action = Some(WorkbenchAction::OpenFile(path_clone));
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

        match event {
            InputEvent::MouseDown(mouse) => {
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
                        if let Some(scm_act) = self.scm_view.last_action.take() {
                            if let ScmAction::OpenDiff { path, title, original, modified } = &scm_act {
                                let path_str = path.to_string_lossy().to_string();
                                self.editor_area.open_diff(&path_str, title, original, modified);
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
                    } else {
                        let res = self.explorer.handle_event(event, regions.sidebar);
                        if let Some(action) = &self.explorer.last_action {
                            match action {
                                ExplorerAction::OpenFile(path) => {
                                    let path_str = path.to_string_lossy().to_string();
                                    let title = path.file_name().map_or("untitled", |n| n.to_str().unwrap_or(""));
                                    let content = std::fs::read_to_string(path).unwrap_or_default();
                                    self.editor_area.open_file(&path_str, title, &content, true);
                                    self.last_action = Some(WorkbenchAction::OpenFile(path.clone()));
                                }
                                ExplorerAction::ToggleDir(_) => {}
                            }
                        }
                        res
                    }
                } else if self.layout.status_bar_visible && regions.status_bar.contains(mouse.position) {
                    let res = self.status_bar.handle_event(event, regions.status_bar);
                    if let Some(action) = &self.status_bar.last_action {
                        match action {
                            StatusBarAction::ExecuteCommand(cmd) => {
                                if cmd == "workbench.action.problems.focus" {
                                    self.layout.panel_visible = true;
                                    self.layout.set_panel_tab("problems");
                                    self.focused_region = FocusedRegion::Panel;
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
                        let tabs = self.panel_tab_bounds.read().clone();
                        for (tab_id, rect) in tabs {
                            if rect.contains(mouse.position) {
                                if tab_id == "close" {
                                    self.layout.panel_visible = false;
                                    self.last_action = Some(WorkbenchAction::PanelToggled(false));
                                } else {
                                    self.layout.set_panel_tab(&tab_id);
                                }
                                return EventResult::Consumed;
                            }
                        }
                        EventResult::Consumed
                    } else if self.layout.active_panel_tab == "problems" {
                        let content_rect = Rect::from_xywh(
                            regions.panel.min_x(),
                            regions.panel.min_y() + tab_h,
                            regions.panel.width(),
                            (regions.panel.height() - tab_h).max(0.0),
                        );
                        let res = self.problems_panel.handle_event(event, content_rect);
                        if let Some(ProblemsAction::JumpToLocation { uri, position }) = self.problems_panel.last_action.take() {
                            let clean_path = uri.strip_prefix("file://").unwrap_or(&uri);
                            let path = PathBuf::from(clean_path);
                            let title = path.file_name().map_or("untitled", |n| n.to_str().unwrap_or(""));
                            let content = std::fs::read_to_string(&path).unwrap_or_default();
                            self.editor_area.open_file(&uri, title, &content, true);
                            if let Some(active_editor) = self.editor_area.active_editor_mut() {
                                active_editor.set_primary_position(position);
                                if let Some(file_diags) = self.problems_panel.files.get(&uri) {
                                    active_editor.set_diagnostics(file_diags.clone());
                                }
                            }
                            self.focused_region = FocusedRegion::Editor;
                            self.last_action = Some(WorkbenchAction::JumpToDiagnostic {
                                uri,
                                line: position.line,
                                column: position.column,
                            });
                        }
                        res
                    } else if self.layout.active_panel_tab == "debug_console" {
                        let content_rect = Rect::from_xywh(
                            regions.panel.min_x(),
                            regions.panel.min_y() + tab_h,
                            regions.panel.width(),
                            (regions.panel.height() - tab_h).max(0.0),
                        );
                        let res = self.debug_console.handle_event(event, content_rect);
                        if let Some(act) = self.debug_console.last_action.take() {
                            self.last_action = Some(WorkbenchAction::DebugAction(act));
                        }
                        res
                    } else {
                        EventResult::Consumed
                    }
                } else if regions.editor_area.contains(mouse.position) {
                    self.focused_region = FocusedRegion::Editor;
                    let res = self.editor_area.handle_event(event, regions.editor_area);
                    // Sync editor gutter breakpoint clicks to debug session
                    if let Some(tab) = self.editor_area.model.current_group_ref().active_tab() {
                        let uri = tab.uri.clone();
                        if let Some(editor) = self.editor_area.editors.get_mut(&uri) {
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
                    FocusedRegion::Editor => self.editor_area.handle_event(event, regions.editor_area),
                    FocusedRegion::Sidebar => {
                        let view_id = self.layout.active_sidebar_view.as_str();
                        if view_id == "scm" || view_id == "workbench.view.scm" {
                            let res = self.scm_view.handle_event(event, regions.sidebar);
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
                        } else {
                            self.explorer.handle_event(event, regions.sidebar)
                        }
                    }
                    FocusedRegion::Panel => {
                        if self.layout.active_panel_tab == "debug_console" {
                            let res = self.debug_console.handle_event(event, regions.panel);
                            if let Some(act) = self.debug_console.last_action.take() {
                                self.last_action = Some(WorkbenchAction::DebugAction(act));
                            }
                            res
                        } else {
                            EventResult::Ignored
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
                    } else {
                        self.explorer.handle_event(event, regions.sidebar)
                    }
                } else if regions.editor_area.contains(scroll.position) {
                    self.editor_area.handle_event(event, regions.editor_area)
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::MouseMove(mouse) => {
                if regions.editor_area.contains(mouse.position) {
                    self.editor_area.handle_event(event, regions.editor_area)
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
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
        let gutter_click_x = regions.editor_area.min_x() + 10.0;
        let gutter_click_y = regions.editor_area.min_y() + 35.0 + font.line_height * 2.5; // Line 3
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
}
