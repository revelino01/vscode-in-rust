//! Custom Title Bar & Menu Bar view for RustCode.
//!
//! Renders top-level VS Code menus (`File`, `Edit`, `Selection`, `View`, `Go`, `Run`, `Terminal`, `Help`),
//! dropdown overlays, layout control toggles (Primary Sidebar / Panel / Secondary Sidebar),
//! active title readout, and window controls (`Minimize`, `Maximize`, `Close`).

use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, MouseButton, Point, Quad, Rect,
    Rgba, Scene, Shadow, View, WorkbenchThemeStyles,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuBarEntry {
    pub label: String,
    pub command_id: String,
    pub shortcut: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopLevelMenu {
    pub id: String,
    pub label: String,
    pub items: Vec<MenuBarEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleBarAction {
    ExecuteCommand(String),
    TogglePrimarySidebar,
    ToggleSecondarySidebar,
    TogglePanel,
    MinimizeWindow,
    MaximizeOrRestoreWindow,
    CloseWindow,
}

#[derive(Debug, Clone)]
struct TitleBarLayout {
    bar_rect: Rect,
    menu_rects: Vec<Rect>,
    toggle_primary_rect: Rect,
    toggle_panel_rect: Rect,
    toggle_secondary_rect: Rect,
    minimize_rect: Rect,
    maximize_rect: Rect,
    close_rect: Rect,
}

pub struct TitleBarView {
    pub menus: Vec<TopLevelMenu>,
    pub title: String,
    pub open_menu_index: Option<usize>,
    pub hovered_menu_index: Option<usize>,
    pub hovered_dropdown_index: Option<usize>,
    pub last_action: Option<TitleBarAction>,
    last_font: FontMetrics,
}

impl Default for TitleBarView {
    fn default() -> Self {
        Self::new()
    }
}

impl TitleBarView {
    pub const HEIGHT: f32 = 32.0;
    const DROPDOWN_ROW_H: f32 = 24.0;
    const DROPDOWN_W: f32 = 280.0;

    pub fn new() -> Self {
        Self {
            menus: Self::default_menus(),
            title: "RustCode".to_string(),
            open_menu_index: None,
            hovered_menu_index: None,
            hovered_dropdown_index: None,
            last_action: None,
            last_font: FontMetrics::default_monospace(),
        }
    }

    pub fn default_menus() -> Vec<TopLevelMenu> {
        let entry = |label: &str, command_id: &str, shortcut: &str| MenuBarEntry {
            label: label.to_string(),
            command_id: command_id.to_string(),
            shortcut: shortcut.to_string(),
        };
        vec![
            TopLevelMenu {
                id: "File".into(),
                label: "File".into(),
                items: vec![
                    entry("New Untitled File", "workbench.action.files.newUntitledFile", "Ctrl+N"),
                    entry("Open File...", "workbench.action.files.openFile", "Ctrl+O"),
                    entry("Open Folder...", "workbench.action.files.openFolder", ""),
                    entry("Save", "workbench.action.files.save", "Ctrl+S"),
                    entry("New Window", "workbench.action.newWindow", "Ctrl+Shift+N"),
                    entry("Close Editor", "workbench.action.closeActiveEditor", "Ctrl+W"),
                ],
            },
            TopLevelMenu {
                id: "Edit".into(),
                label: "Edit".into(),
                items: vec![
                    entry("Undo", "editor.action.undo", "Ctrl+Z"),
                    entry("Redo", "editor.action.redo", "Ctrl+Y"),
                    entry("Find", "actions.find", "Ctrl+F"),
                    entry("Replace", "editor.action.startFindReplaceAction", "Ctrl+H"),
                    entry("Toggle Line Comment", "editor.action.commentLine", "Ctrl+/"),
                ],
            },
            TopLevelMenu {
                id: "Selection".into(),
                label: "Selection".into(),
                items: vec![
                    entry("Add Next Occurrence", "editor.action.addSelectionToNextFindMatch", "Ctrl+D"),
                    entry("Copy Line Down", "editor.action.copyLinesDownAction", "Shift+Alt+Down"),
                    entry("Move Line Up", "editor.action.moveLinesUpAction", "Alt+Up"),
                    entry("Move Line Down", "editor.action.moveLinesDownAction", "Alt+Down"),
                ],
            },
            TopLevelMenu {
                id: "View".into(),
                label: "View".into(),
                items: vec![
                    entry("Command Palette...", "workbench.action.showCommands", "Ctrl+Shift+P"),
                    entry("Toggle Primary Side Bar", "workbench.action.toggleSidebar", "Ctrl+B"),
                    entry("Toggle Secondary Side Bar", "workbench.action.toggleAuxiliaryBar", "Ctrl+Alt+B"),
                    entry("Toggle Panel", "workbench.action.togglePanel", "Ctrl+J"),
                    entry("Zen Mode", "workbench.action.toggleZenMode", "Ctrl+K Z"),
                    entry("Zoom In", "workbench.action.zoomIn", "Ctrl+="),
                    entry("Zoom Out", "workbench.action.zoomOut", "Ctrl+-"),
                    entry("Reset Zoom", "workbench.action.zoomReset", "Ctrl+0"),
                    entry("Toggle Menu Bar", "workbench.action.toggleMenuBar", ""),
                ],
            },
            TopLevelMenu {
                id: "Go".into(),
                label: "Go".into(),
                items: vec![
                    entry("Go to File...", "workbench.action.quickOpen", "Ctrl+P"),
                    entry("Go to Symbol in Workspace...", "workbench.action.showAllSymbols", "Ctrl+T"),
                    entry("Go to Line/Column...", "workbench.action.gotoLine", "Ctrl+G"),
                    entry("Go to Definition", "editor.action.revealDefinition", "F12"),
                    entry("Go to References", "editor.action.goToReferences", "Shift+F12"),
                ],
            },
            TopLevelMenu {
                id: "Run".into(),
                label: "Run".into(),
                items: vec![
                    entry("Start Debugging", "workbench.action.debug.start", "F5"),
                    entry("Continue", "workbench.action.debug.continue", "F5"),
                    entry("Step Over", "workbench.action.debug.stepOver", "F10"),
                    entry("Stop Debugging", "workbench.action.debug.stop", "Shift+F5"),
                ],
            },
            TopLevelMenu {
                id: "Terminal".into(),
                label: "Terminal".into(),
                items: vec![
                    entry("New Terminal", "workbench.action.terminal.new", ""),
                    entry("Toggle Terminal", "workbench.action.terminal.toggleTerminal", "Ctrl+`"),
                ],
            },
            TopLevelMenu {
                id: "Help".into(),
                label: "Help".into(),
                items: vec![
                    entry("Show All Commands", "workbench.action.showCommands", "F1"),
                    entry("Keyboard Shortcuts", "workbench.action.openGlobalKeybindings", ""),
                ],
            },
        ]
    }

    pub fn close_dropdown(&mut self) {
        self.open_menu_index = None;
        self.hovered_dropdown_index = None;
    }

    pub fn is_dropdown_open(&self) -> bool {
        self.open_menu_index.is_some()
    }

    pub fn take_action(&mut self) -> Option<TitleBarAction> {
        self.last_action.take()
    }

    fn compute_layout(&self, bounds: Rect, font: &FontMetrics) -> TitleBarLayout {
        let bar_rect = if bounds.is_finite_positive() {
            Rect::from_xywh(
                bounds.min_x(),
                bounds.min_y(),
                bounds.width(),
                Self::HEIGHT.min(bounds.height()),
            )
        } else {
            Rect::zero()
        };

        let mut menu_rects = Vec::with_capacity(self.menus.len());
        let mut x = bar_rect.min_x() + 10.0;
        let btn_w = 28.0;
        let right_controls_w = btn_w * 6.0 + 14.0;
        let menu_limit_x = (bar_rect.max_x() - right_controls_w).max(bar_rect.min_x());

        for menu in &self.menus {
            let w = (font.text_width(&menu.label) + 14.0).max(32.0);
            if x + w <= menu_limit_x {
                menu_rects.push(Rect::from_xywh(x, bar_rect.min_y() + 2.0, w, bar_rect.height() - 4.0));
            } else {
                menu_rects.push(Rect::zero());
            }
            x += w + 2.0;
        }

        let mut rx = bar_rect.max_x();
        let make_right_btn = |rx: &mut f32| {
            *rx -= btn_w;
            Rect::from_xywh(
                rx.max(bar_rect.min_x()),
                bar_rect.min_y(),
                btn_w.min(bar_rect.width()),
                bar_rect.height(),
            )
        };
        let close_rect = make_right_btn(&mut rx);
        let maximize_rect = make_right_btn(&mut rx);
        let minimize_rect = make_right_btn(&mut rx);
        rx -= 8.0;
        let toggle_secondary_rect = make_right_btn(&mut rx);
        let toggle_panel_rect = make_right_btn(&mut rx);
        let toggle_primary_rect = make_right_btn(&mut rx);

        TitleBarLayout {
            bar_rect,
            menu_rects,
            toggle_primary_rect,
            toggle_panel_rect,
            toggle_secondary_rect,
            minimize_rect,
            maximize_rect,
            close_rect,
        }
    }

    pub fn dropdown_rect(
        &self,
        bar_bounds: Rect,
        window_bounds: Rect,
        menu_idx: usize,
        font: &FontMetrics,
    ) -> Option<Rect> {
        let menu = self.menus.get(menu_idx)?;
        if menu.items.is_empty() || !bar_bounds.is_finite_positive() {
            return None;
        }
        let layout = self.compute_layout(bar_bounds, font);
        let anchor = layout.menu_rects.get(menu_idx).copied().unwrap_or(bar_bounds);
        let max_w = window_bounds.width().max(Self::DROPDOWN_W);
        let w = Self::DROPDOWN_W.min(max_w);
        let h = (menu.items.len() as f32) * Self::DROPDOWN_ROW_H + 8.0;
        let x = anchor
            .min_x()
            .min((window_bounds.max_x() - w).max(window_bounds.min_x()));
        let y = bar_bounds.max_y();
        let clamped_h = h.min((window_bounds.max_y() - y).max(Self::DROPDOWN_ROW_H));
        Some(Rect::from_xywh(x, y, w, clamped_h))
    }

    pub fn render_bar(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !bounds.is_finite_positive() {
            return;
        }
        let layout = self.compute_layout(bounds, font);
        scene.push_finite_quad(
            Quad::new(layout.bar_rect)
                .with_background(theme.activity_bar_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        let text_y = layout.bar_rect.min_y() + ((layout.bar_rect.height() - font.line_height) * 0.5).max(0.0);
        let mut menus_end_x = layout.bar_rect.min_x() + 10.0;

        for (idx, (menu, rect)) in self.menus.iter().zip(layout.menu_rects.iter()).enumerate() {
            if !rect.is_finite_positive() {
                continue;
            }
            menus_end_x = rect.max_x();
            let is_active = self.open_menu_index == Some(idx) || self.hovered_menu_index == Some(idx);
            if is_active {
                scene.push_finite_quad(Quad::new(*rect).with_background(theme.list_hover_bg));
            }
            scene.push_finite_text(GlyphRun::new(
                menu.label.clone(),
                Point::new(rect.min_x() + 7.0, text_y),
                *rect,
                theme.sidebar_fg,
                font.font_size,
            ));
        }

        // Centered window title in remaining gap
        let gap_left = menus_end_x + 12.0;
        let gap_right = layout.toggle_primary_rect.min_x() - 12.0;
        if gap_right > gap_left + 40.0 {
            let title_w = font.text_width(&self.title);
            let title_x = (gap_left + (gap_right - gap_left - title_w) * 0.5).max(gap_left);
            let title_rect = Rect::from_xywh(gap_left, layout.bar_rect.min_y(), gap_right - gap_left, layout.bar_rect.height());
            scene.push_finite_text(GlyphRun::new(
                self.title.clone(),
                Point::new(title_x, text_y),
                title_rect,
                theme.breadcrumb_fg,
                font.font_size,
            ));
        }

        for (btn_rect, label, fg) in [
            (layout.toggle_primary_rect, "[L]", theme.sidebar_fg),
            (layout.toggle_panel_rect, "[P]", theme.sidebar_fg),
            (layout.toggle_secondary_rect, "[R]", theme.sidebar_fg),
            (layout.minimize_rect, "_", theme.sidebar_fg),
            (layout.maximize_rect, "[]", theme.sidebar_fg),
            (layout.close_rect, "X", theme.sidebar_fg),
        ] {
            if !btn_rect.is_finite_positive() {
                continue;
            }
            let lw = font.text_width(label);
            let lx = btn_rect.min_x() + ((btn_rect.width() - lw) * 0.5).max(2.0);
            scene.push_finite_text(GlyphRun::new(
                label.to_string(),
                Point::new(lx, text_y),
                btn_rect,
                fg,
                font.font_size,
            ));
        }
    }

    pub fn render_dropdown(
        &self,
        bar_bounds: Rect,
        window_bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        let Some(menu_idx) = self.open_menu_index else {
            return;
        };
        let Some(menu) = self.menus.get(menu_idx) else {
            return;
        };
        let Some(dd_rect) = self.dropdown_rect(bar_bounds, window_bounds, menu_idx, font) else {
            return;
        };

        scene.push_shadow(Shadow {
            bounds: dd_rect,
            offset: Point::new(0.0, 2.0),
            blur: 10.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.4),
        });
        scene.push_finite_quad(
            Quad::new(dd_rect)
                .with_background(theme.command_palette_bg)
                .with_border_edges(theme.command_palette_border, Edges::all(1.0)),
        );

        let mut ry = dd_rect.min_y() + 4.0;
        for (idx, item) in menu.items.iter().enumerate() {
            if ry + Self::DROPDOWN_ROW_H > dd_rect.max_y() {
                break;
            }
            let row_rect = Rect::from_xywh(
                dd_rect.min_x() + 2.0,
                ry,
                dd_rect.width() - 4.0,
                Self::DROPDOWN_ROW_H,
            );
            if self.hovered_dropdown_index == Some(idx) {
                scene.push_finite_quad(
                    Quad::new(row_rect).with_background(theme.command_palette_selected_bg),
                );
            }
            let text_y = ry + ((Self::DROPDOWN_ROW_H - font.line_height) * 0.5).max(0.0);
            scene.push_finite_text(GlyphRun::new(
                item.label.clone(),
                Point::new(row_rect.min_x() + 10.0, text_y),
                row_rect,
                theme.command_palette_fg,
                font.font_size,
            ));
            if !item.shortcut.is_empty() {
                let sw = font.text_width(&item.shortcut);
                scene.push_finite_text(GlyphRun::new(
                    item.shortcut.clone(),
                    Point::new((row_rect.max_x() - sw - 10.0).max(row_rect.min_x()), text_y),
                    row_rect,
                    theme.breadcrumb_fg,
                    font.font_size * 0.9,
                ));
            }
            ry += Self::DROPDOWN_ROW_H;
        }
    }

    pub fn dispatch_event_with_overlay(
        &mut self,
        event: &InputEvent,
        bar_bounds: Rect,
        window_bounds: Rect,
    ) -> EventResult {
        let font = self.last_font.clone();
        if let Some(menu_idx) = self.open_menu_index {
            match event {
                InputEvent::KeyDown(k) => match k.key {
                    KeyCode::Escape => {
                        self.close_dropdown();
                        return EventResult::Consumed;
                    }
                    KeyCode::ArrowLeft if !self.menus.is_empty() => {
                        let prev = if menu_idx == 0 {
                            self.menus.len() - 1
                        } else {
                            menu_idx - 1
                        };
                        self.open_menu_index = Some(prev);
                        self.hovered_dropdown_index = Some(0);
                        return EventResult::Consumed;
                    }
                    KeyCode::ArrowRight if !self.menus.is_empty() => {
                        let next = (menu_idx + 1) % self.menus.len();
                        self.open_menu_index = Some(next);
                        self.hovered_dropdown_index = Some(0);
                        return EventResult::Consumed;
                    }
                    KeyCode::ArrowDown => {
                        let len = self.menus[menu_idx].items.len();
                        if len > 0 {
                            let next = self
                                .hovered_dropdown_index
                                .map_or(0, |i| (i + 1) % len);
                            self.hovered_dropdown_index = Some(next);
                        }
                        return EventResult::Consumed;
                    }
                    KeyCode::ArrowUp => {
                        let len = self.menus[menu_idx].items.len();
                        if len > 0 {
                            let prev = match self.hovered_dropdown_index {
                                Some(i) if i > 0 => i - 1,
                                _ => len - 1,
                            };
                            self.hovered_dropdown_index = Some(prev);
                        }
                        return EventResult::Consumed;
                    }
                    KeyCode::Enter => {
                        if let Some(item_idx) = self.hovered_dropdown_index {
                            if let Some(item) = self.menus[menu_idx].items.get(item_idx) {
                                self.last_action =
                                    Some(TitleBarAction::ExecuteCommand(item.command_id.clone()));
                            }
                        }
                        self.close_dropdown();
                        return EventResult::Consumed;
                    }
                    _ => {}
                },
                InputEvent::MouseMove(mm) => {
                    if let Some(dd_rect) = self.dropdown_rect(bar_bounds, window_bounds, menu_idx, &font) {
                        if dd_rect.contains(mm.position) {
                            let rel_y = mm.position.y - (dd_rect.min_y() + 4.0);
                            if rel_y >= 0.0 {
                                let idx = (rel_y / Self::DROPDOWN_ROW_H).floor() as usize;
                                if idx < self.menus[menu_idx].items.len() {
                                    self.hovered_dropdown_index = Some(idx);
                                }
                            }
                            return EventResult::Consumed;
                        }
                    }
                    let layout = self.compute_layout(bar_bounds, &font);
                    for (idx, r) in layout.menu_rects.iter().enumerate() {
                        if r.contains(mm.position) {
                            self.open_menu_index = Some(idx);
                            self.hovered_menu_index = Some(idx);
                            self.hovered_dropdown_index = Some(0);
                            return EventResult::Consumed;
                        }
                    }
                }
                InputEvent::MouseDown(mouse) if mouse.button == MouseButton::Left => {
                    if let Some(dd_rect) = self.dropdown_rect(bar_bounds, window_bounds, menu_idx, &font) {
                        if dd_rect.contains(mouse.position) {
                            let rel_y = mouse.position.y - (dd_rect.min_y() + 4.0);
                            if rel_y >= 0.0 {
                                let idx = (rel_y / Self::DROPDOWN_ROW_H).floor() as usize;
                                if let Some(item) = self.menus[menu_idx].items.get(idx) {
                                    self.last_action = Some(TitleBarAction::ExecuteCommand(
                                        item.command_id.clone(),
                                    ));
                                }
                            }
                            self.close_dropdown();
                            return EventResult::Consumed;
                        }
                    }
                    // Clicking the same top-level menu header toggles closed
                    let layout = self.compute_layout(bar_bounds, &font);
                    if let Some(r) = layout.menu_rects.get(menu_idx) {
                        if r.contains(mouse.position) {
                            self.close_dropdown();
                            return EventResult::Consumed;
                        }
                    }
                    // Clicking outside closes the dropdown without swallowing top-bar clicks if outside bar
                    self.close_dropdown();
                    if !bar_bounds.contains(mouse.position) {
                        return EventResult::Consumed;
                    }
                }
                _ => {}
            }
        }

        if !bar_bounds.is_finite_positive() {
            return EventResult::Ignored;
        }
        let layout = self.compute_layout(bar_bounds, &font);

        match event {
            InputEvent::MouseMove(mm) => {
                if !bar_bounds.contains(mm.position) {
                    self.hovered_menu_index = None;
                    return EventResult::Ignored;
                }
                self.hovered_menu_index = layout
                    .menu_rects
                    .iter()
                    .position(|r| r.contains(mm.position));
                EventResult::Consumed
            }
            InputEvent::MouseDown(mouse) if mouse.button == MouseButton::Left => {
                if !bar_bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }
                for (idx, r) in layout.menu_rects.iter().enumerate() {
                    if r.contains(mouse.position) {
                        if self.open_menu_index == Some(idx) {
                            self.close_dropdown();
                        } else {
                            self.open_menu_index = Some(idx);
                            self.hovered_dropdown_index = Some(0);
                        }
                        return EventResult::Consumed;
                    }
                }
                if layout.toggle_primary_rect.contains(mouse.position) {
                    self.last_action = Some(TitleBarAction::TogglePrimarySidebar);
                    return EventResult::Consumed;
                }
                if layout.toggle_panel_rect.contains(mouse.position) {
                    self.last_action = Some(TitleBarAction::TogglePanel);
                    return EventResult::Consumed;
                }
                if layout.toggle_secondary_rect.contains(mouse.position) {
                    self.last_action = Some(TitleBarAction::ToggleSecondarySidebar);
                    return EventResult::Consumed;
                }
                if layout.minimize_rect.contains(mouse.position) {
                    self.last_action = Some(TitleBarAction::MinimizeWindow);
                    return EventResult::Consumed;
                }
                if layout.maximize_rect.contains(mouse.position) {
                    self.last_action = Some(TitleBarAction::MaximizeOrRestoreWindow);
                    return EventResult::Consumed;
                }
                if layout.close_rect.contains(mouse.position) {
                    self.last_action = Some(TitleBarAction::CloseWindow);
                    return EventResult::Consumed;
                }
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

impl View for TitleBarView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        self.render_bar(bounds, theme, font, scene);
        self.render_dropdown(bounds, bounds, theme, font, scene);
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        self.dispatch_event_with_overlay(event, bounds, bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{KeyEvent, MouseEvent};

    #[test]
    fn test_titlebar_menus_dropdown_layout_and_window_controls() {
        let mut tb = TitleBarView::new();
        let font = FontMetrics::default_monospace();
        let bar_bounds = Rect::from_xywh(0.0, 0.0, 1200.0, TitleBarView::HEIGHT);
        let win_bounds = Rect::from_xywh(0.0, 0.0, 1200.0, 800.0);

        let layout = tb.compute_layout(bar_bounds, &font);
        assert_eq!(layout.menu_rects.len(), 8);

        // Open "View" menu (index 3)
        let view_center = Point::new(
            (layout.menu_rects[3].min_x() + layout.menu_rects[3].max_x()) * 0.5,
            (layout.menu_rects[3].min_y() + layout.menu_rects[3].max_y()) * 0.5,
        );
        assert_eq!(
            tb.dispatch_event_with_overlay(
                &InputEvent::MouseDown(MouseEvent::new(view_center, MouseButton::Left)),
                bar_bounds,
                win_bounds
            ),
            EventResult::Consumed
        );
        assert_eq!(tb.open_menu_index, Some(3));

        // Render bar & dropdown into Scene
        let mut scene = Scene::new();
        let theme = WorkbenchThemeStyles::dark_default();
        tb.render_bar(bar_bounds, &theme, &font, &mut scene);
        tb.render_dropdown(bar_bounds, win_bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("File"));
        assert!(scene.contains_text("Zen Mode"));

        // Click first dropdown item ("Command Palette...")
        let dd = tb.dropdown_rect(bar_bounds, win_bounds, 3, &font).unwrap();
        let first_item_pt = Point::new(dd.min_x() + 20.0, dd.min_y() + 12.0);
        assert_eq!(
            tb.dispatch_event_with_overlay(
                &InputEvent::MouseDown(MouseEvent::new(first_item_pt, MouseButton::Left)),
                bar_bounds,
                win_bounds
            ),
            EventResult::Consumed
        );
        assert!(!tb.is_dropdown_open());
        assert_eq!(
            tb.take_action(),
            Some(TitleBarAction::ExecuteCommand(
                "workbench.action.showCommands".to_string()
            ))
        );

        // Keyboard reopen + Escape closes cleanly
        tb.open_menu_index = Some(0);
        tb.dispatch_event_with_overlay(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bar_bounds,
            win_bounds,
        );
        assert!(!tb.is_dropdown_open());

        // Layout toggle & window control clicks
        let sec_center = Point::new(
            (layout.toggle_secondary_rect.min_x() + layout.toggle_secondary_rect.max_x()) * 0.5,
            16.0,
        );
        tb.dispatch_event_with_overlay(
            &InputEvent::MouseDown(MouseEvent::new(sec_center, MouseButton::Left)),
            bar_bounds,
            win_bounds,
        );
        assert_eq!(tb.take_action(), Some(TitleBarAction::ToggleSecondarySidebar));

        let close_center = Point::new(
            (layout.close_rect.min_x() + layout.close_rect.max_x()) * 0.5,
            16.0,
        );
        tb.dispatch_event_with_overlay(
            &InputEvent::MouseDown(MouseEvent::new(close_center, MouseButton::Left)),
            bar_bounds,
            win_bounds,
        );
        assert_eq!(tb.take_action(), Some(TitleBarAction::CloseWindow));
    }
}
