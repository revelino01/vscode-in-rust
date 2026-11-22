//! Right-click context menu state, layout, and scene rendering for `EditorView`.

use parking_lot::Mutex;
use rustcode_platform::render::{
    Edges, FontMetrics, GlyphRun, Point, Quad, Rect, Rgba, Scene, Shadow, WorkbenchThemeStyles,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    Cut,
    Copy,
    Paste,
    GoToDefinition,
    FormatDocument,
    ToggleComment,
    SelectAll,
    CommandPalette,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenuItem {
    pub label: String,
    pub shortcut: Option<String>,
    pub action: ContextMenuAction,
}

impl ContextMenuItem {
    pub fn new(
        label: impl Into<String>,
        shortcut: Option<&str>,
        action: ContextMenuAction,
    ) -> Self {
        Self {
            label: label.into(),
            shortcut: shortcut.map(ToString::to_string),
            action,
        }
    }
}

pub struct EditorContextMenu {
    pub is_visible: bool,
    pub anchor: Point,
    pub items: Vec<ContextMenuItem>,
    pub selected_index: Option<usize>,
    pub last_bounds: Mutex<Option<Rect>>,
}

impl Default for EditorContextMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorContextMenu {
    pub const ITEM_HEIGHT: f32 = 24.0;
    pub const MENU_WIDTH: f32 = 220.0;
    pub const PADDING_Y: f32 = 4.0;

    pub fn new() -> Self {
        Self {
            is_visible: false,
            anchor: Point::zero(),
            items: Self::default_items(),
            selected_index: None,
            last_bounds: Mutex::new(None),
        }
    }

    pub fn default_items() -> Vec<ContextMenuItem> {
        vec![
            ContextMenuItem::new("Cut", Some("Ctrl+X"), ContextMenuAction::Cut),
            ContextMenuItem::new("Copy", Some("Ctrl+C"), ContextMenuAction::Copy),
            ContextMenuItem::new("Paste", Some("Ctrl+V"), ContextMenuAction::Paste),
            ContextMenuItem::new(
                "Go to Definition",
                Some("F12"),
                ContextMenuAction::GoToDefinition,
            ),
            ContextMenuItem::new(
                "Format Document",
                Some("Shift+Alt+F"),
                ContextMenuAction::FormatDocument,
            ),
            ContextMenuItem::new(
                "Toggle Line Comment",
                Some("Ctrl+/"),
                ContextMenuAction::ToggleComment,
            ),
            ContextMenuItem::new(
                "Select All",
                Some("Ctrl+A"),
                ContextMenuAction::SelectAll,
            ),
            ContextMenuItem::new(
                "Command Palette...",
                Some("Ctrl+Shift+P"),
                ContextMenuAction::CommandPalette,
            ),
        ]
    }

    pub fn open(&mut self, position: Point) {
        self.is_visible = true;
        self.anchor = position;
        self.selected_index = Some(0);
    }

    pub fn close(&mut self) {
        self.is_visible = false;
        self.selected_index = None;
        *self.last_bounds.lock() = None;
    }

    pub fn select_next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected_index = Some(match self.selected_index {
            Some(i) => (i + 1) % self.items.len(),
            None => 0,
        });
    }

    pub fn select_prev(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected_index = Some(match self.selected_index {
            Some(0) | None => self.items.len() - 1,
            Some(i) => i - 1,
        });
    }

    pub fn compute_bounds(&self, editor_bounds: Rect) -> Option<Rect> {
        if !self.is_visible || self.items.is_empty() || !editor_bounds.is_finite_positive() {
            return None;
        }
        let menu_w = Self::MENU_WIDTH.min((editor_bounds.width() - 8.0).max(80.0));
        let menu_h = self.items.len() as f32 * Self::ITEM_HEIGHT + Self::PADDING_Y * 2.0;

        let mut x = if self.anchor.x.is_finite() {
            self.anchor.x
        } else {
            editor_bounds.min_x()
        };
        let mut y = if self.anchor.y.is_finite() {
            self.anchor.y
        } else {
            editor_bounds.min_y()
        };

        if x + menu_w > editor_bounds.max_x() - 4.0 {
            x = (editor_bounds.max_x() - menu_w - 4.0).max(editor_bounds.min_x());
        }
        if y + menu_h > editor_bounds.max_y() - 4.0 {
            y = (editor_bounds.max_y() - menu_h - 4.0).max(editor_bounds.min_y());
        }
        let rect = Rect::from_xywh(x, y, menu_w, menu_h);
        rect.is_finite_positive().then_some(rect)
    }

    pub fn hit_test_item(&self, point: Point, editor_bounds: Rect) -> Option<(usize, ContextMenuAction)> {
        let menu_rect = self.compute_bounds(editor_bounds)?;
        if !menu_rect.contains(point) {
            return None;
        }
        let rel_y = point.y - menu_rect.min_y() - Self::PADDING_Y;
        if rel_y < 0.0 {
            return None;
        }
        let idx = (rel_y / Self::ITEM_HEIGHT).floor() as usize;
        self.items.get(idx).map(|item| (idx, item.action))
    }

    pub fn render(
        &self,
        editor_bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        let Some(menu_rect) = self.compute_bounds(editor_bounds) else {
            return;
        };
        *self.last_bounds.lock() = Some(menu_rect);

        scene.push_finite_shadow(Shadow {
            bounds: menu_rect,
            offset: Point::new(0.0, 4.0),
            blur: 10.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.35),
        });

        scene.push_finite_quad(
            Quad::new(menu_rect)
                .with_background(theme.command_palette_bg)
                .with_border_edges(theme.command_palette_border, Edges::all(1.0))
                .with_corner_radius(4.0),
        );

        for (idx, item) in self.items.iter().enumerate() {
            let item_y = menu_rect.min_y() + Self::PADDING_Y + idx as f32 * Self::ITEM_HEIGHT;
            let item_rect = Rect::from_xywh(
                menu_rect.min_x() + 4.0,
                item_y,
                (menu_rect.width() - 8.0).max(0.0),
                Self::ITEM_HEIGHT,
            );
            let is_selected = self.selected_index == Some(idx);
            if is_selected {
                scene.push_finite_quad(
                    Quad::new(item_rect)
                        .with_background(theme.command_palette_selected_bg)
                        .with_corner_radius(2.0),
                );
            }

            let text_y = item_y + (Self::ITEM_HEIGHT - font.line_height) * 0.5;
            let fg = if is_selected {
                theme.command_palette_fg
            } else {
                theme.editor_foreground
            };
            let label_x = menu_rect.min_x() + 10.0;
            scene.push_finite_text(GlyphRun::new(
                item.label.clone(),
                Point::new(label_x, text_y),
                Rect::from_xywh(label_x, text_y, (menu_rect.width() - 80.0).max(20.0), font.line_height),
                fg,
                font.font_size * 0.92,
            ));

            if let Some(sc) = &item.shortcut {
                let sc_w = font.text_width(sc);
                let sc_x = (menu_rect.max_x() - sc_w - 10.0).max(label_x + 40.0);
                scene.push_finite_text(GlyphRun::new(
                    sc.clone(),
                    Point::new(sc_x, text_y),
                    Rect::from_xywh(sc_x, text_y, sc_w, font.line_height),
                    Rgba::new(0.6, 0.6, 0.6, 0.9),
                    font.font_size * 0.85,
                ));
            }
        }
    }
}
