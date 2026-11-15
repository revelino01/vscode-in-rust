//! GPUI WorkbenchView coordinator composing all UI shell regions into a cohesive presentation tree.

use crate::activitybar::{ActivityBarAction, ActivityBarView};
use crate::editor_area::EditorAreaView;
use crate::explorer::{ExplorerAction, FileExplorerView};
use crate::layout::WorkbenchLayout;
use crate::quickinput::{CommandPaletteView, PaletteAction};
use crate::statusbar::{StatusBarAction, StatusBarView};
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
    SidebarToggled(bool),
    PanelToggled(bool),
}

pub struct WorkbenchView {
    pub layout: WorkbenchLayout,
    pub activity_bar: ActivityBarView,
    pub explorer: FileExplorerView,
    pub editor_area: EditorAreaView,
    pub status_bar: StatusBarView,
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
        Self {
            layout: WorkbenchLayout::new(),
            activity_bar: ActivityBarView::default(),
            explorer: FileExplorerView::default(),
            editor_area: EditorAreaView::default(),
            status_bar: StatusBarView::default(),
            command_palette: None,
            is_command_palette_open: false,
            focused_region: FocusedRegion::Editor,
            last_action: None,
        }
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

        // 5. Panel (terminal / problems)
        if self.layout.panel_visible {
            scene.push_quad(
                Quad::new(regions.panel)
                    .with_background(theme.panel_bg)
                    .with_border_edges(theme.panel_border, Edges::new(1.0, 0.0, 0.0, 0.0)),
            );
            scene.push_text(GlyphRun::new(
                "TERMINAL".to_string(),
                Point::new(regions.panel.min_x() + 10.0, regions.panel.min_y() + 6.0),
                Rect::from_xywh(regions.panel.min_x() + 10.0, regions.panel.min_y() + 6.0, 100.0, font.line_height),
                theme.sidebar_header_fg,
                font.font_size * 0.85,
            ));
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
                                self.last_action = Some(WorkbenchAction::ExecuteCommand(cmd.clone()));
                            }
                        }
                    }
                    res
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
}
