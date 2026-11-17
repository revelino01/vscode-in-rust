//! GPUI WorkbenchView coordinator composing all UI shell regions into a cohesive presentation tree.

use crate::activitybar::{ActivityBarAction, ActivityBarView};
use crate::editor_area::EditorAreaView;
use crate::explorer::{ExplorerAction, FileExplorerView};
use crate::layout::WorkbenchLayout;
use crate::problems::{ProblemsAction, ProblemsPanel};
use crate::quickinput::{CommandPaletteView, PaletteAction};
use crate::statusbar::{StatusBarAction, StatusBarAlignment, StatusBarItem, StatusBarView};
use parking_lot::RwLock;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Rgba, Scene, View,
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
    JumpToDiagnostic {
        uri: String,
        line: usize,
        column: usize,
    },
    SidebarToggled(bool),
    PanelToggled(bool),
}

pub struct WorkbenchView {
    pub layout: WorkbenchLayout,
    pub activity_bar: ActivityBarView,
    pub explorer: FileExplorerView,
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

        // 3. Primary sidebar (Explorer)
        if self.layout.sidebar_visible {
            self.explorer.render(regions.sidebar, theme, font, scene);
        }

        // 4. Editor area
        self.editor_area.render(regions.editor_area, theme, font, scene);

        // 5. Panel (terminal / problems / output)
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
                    } else {
                        EventResult::Consumed
                    }
                } else if regions.editor_area.contains(mouse.position) {
                    self.focused_region = FocusedRegion::Editor;
                    self.editor_area.handle_event(event, regions.editor_area)
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::KeyDown(_) | InputEvent::KeyUp(_) => {
                match self.focused_region {
                    FocusedRegion::Editor => self.editor_area.handle_event(event, regions.editor_area),
                    FocusedRegion::Sidebar => self.explorer.handle_event(event, regions.sidebar),
                    _ => EventResult::Ignored,
                }
            }
            InputEvent::ScrollWheel(scroll) => {
                if self.layout.sidebar_visible && regions.sidebar.contains(scroll.position) {
                    self.explorer.handle_event(event, regions.sidebar)
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
        palette.set_items(vec![PaletteItem {
            command_id: "workbench.action.toggleSidebar".to_string(),
            label: "Toggle Sidebar".to_string(),
            category: Some("View".to_string()),
            keybinding: Some("Ctrl+B".to_string()),
        }]);

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
}
