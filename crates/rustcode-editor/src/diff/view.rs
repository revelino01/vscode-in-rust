//! Diff Editor View presentation component.
//!
//! Renders inline and side-by-side diff views calculated via the Myers diff engine.

use super::myers::{DiffModel, DiffOpKind, DiffViewMode};
use parking_lot::RwLock;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Rgba, Scene, View,
    WorkbenchThemeStyles,
};

pub struct DiffEditorView {
    pub title: String,
    pub original_text: String,
    pub modified_text: String,
    pub model: DiffModel,
    pub mode: DiffViewMode,
    pub scroll_top: f32,
    pub scroll_left: f32,
    pub header_button_bounds: RwLock<Option<Rect>>,
}

impl DiffEditorView {
    pub fn new(title: &str, original: &str, modified: &str) -> Self {
        let model = DiffModel::compute(original, modified);
        Self {
            title: title.to_string(),
            original_text: original.to_string(),
            modified_text: modified.to_string(),
            model,
            mode: DiffViewMode::SideBySide,
            scroll_top: 0.0,
            scroll_left: 0.0,
            header_button_bounds: RwLock::new(None),
        }
    }

    pub fn set_mode(&mut self, mode: DiffViewMode) {
        self.mode = mode;
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            DiffViewMode::SideBySide => DiffViewMode::Inline,
            DiffViewMode::Inline => DiffViewMode::SideBySide,
        };
    }

    pub fn reload(&mut self, original: &str, modified: &str) {
        self.original_text = original.to_string();
        self.modified_text = modified.to_string();
        self.model = DiffModel::compute(original, modified);
    }
}

impl View for DiffEditorView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        // 1. Overall background
        scene.push_quad(Quad::new(bounds).with_background(theme.editor_background));

        // 2. Toolbar Header (height 28.0)
        let header_h = 28.0;
        let header_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), header_h);
        scene.push_quad(
            Quad::new(header_rect)
                .with_background(theme.panel_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        // Title
        let title_w = font.text_width(&self.title);
        scene.push_text(GlyphRun::new(
            self.title.clone(),
            Point::new(bounds.min_x() + 10.0, bounds.min_y() + 5.0),
            Rect::from_xywh(bounds.min_x() + 10.0, bounds.min_y() + 5.0, title_w, font.line_height),
            theme.editor_foreground,
            font.font_size * 0.9,
        ));

        // Stats badge: "+X -Y"
        let stats_text = format!("+{} -{}", self.model.additions, self.model.deletions);
        let stats_w = font.text_width(&stats_text);
        let stats_x = bounds.min_x() + 20.0 + title_w;
        scene.push_text(GlyphRun::new(
            stats_text,
            Point::new(stats_x, bounds.min_y() + 5.0),
            Rect::from_xywh(stats_x, bounds.min_y() + 5.0, stats_w, font.line_height),
            theme.editor_info_foreground,
            font.font_size * 0.85,
        ));

        // Mode switch button on right
        let mode_label = match self.mode {
            DiffViewMode::SideBySide => "Mode: Side-by-Side (click to switch)",
            DiffViewMode::Inline => "Mode: Inline (click to switch)",
        };
        let mode_w = font.text_width(mode_label) + 16.0;
        let btn_rect = Rect::from_xywh(
            bounds.max_x() - mode_w - 12.0,
            bounds.min_y() + 3.0,
            mode_w,
            header_h - 6.0,
        );
        *self.header_button_bounds.write() = Some(btn_rect);

        scene.push_quad(
            Quad::new(btn_rect)
                .with_background(theme.command_palette_selected_bg)
                .with_border_edges(theme.panel_border, Edges::all(1.0))
                .with_corner_radius(3.0),
        );
        scene.push_text(GlyphRun::new(
            mode_label.to_string(),
            Point::new(btn_rect.min_x() + 8.0, btn_rect.min_y() + 3.0),
            Rect::from_xywh(btn_rect.min_x() + 8.0, btn_rect.min_y() + 3.0, mode_w - 16.0, font.line_height),
            theme.editor_foreground,
            font.font_size * 0.8,
        ));

        // 3. Diff Content Area
        let content_y = bounds.min_y() + header_h;
        let content_h = (bounds.height() - header_h).max(0.0);
        let content_rect = Rect::from_xywh(bounds.min_x(), content_y, bounds.width(), content_h);

        match self.mode {
            DiffViewMode::SideBySide => {
                self.render_side_by_side(content_rect, theme, font, scene);
            }
            DiffViewMode::Inline => {
                self.render_inline(content_rect, theme, font, scene);
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::MouseDown(mouse) => {
                let clicked = self
                    .header_button_bounds
                    .read()
                    .is_some_and(|b| b.contains(mouse.position));
                if clicked {
                    self.toggle_mode();
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            InputEvent::ScrollWheel(scroll) => {
                if bounds.contains(scroll.position) {
                    let total_rows = match self.mode {
                        DiffViewMode::SideBySide => self.model.side_by_side_rows.len(),
                        DiffViewMode::Inline => self.model.inline_rows.len(),
                    };
                    let max_scroll = (total_rows as f32 - 5.0).max(0.0);
                    self.scroll_top = (self.scroll_top + scroll.delta_y).clamp(0.0, max_scroll);
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
        }
    }
}

impl DiffEditorView {
    fn render_side_by_side(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        let half_w = bounds.width() / 2.0;
        let gutter_w = (4.0 * font.char_width).max(36.0);

        // Divider down the middle
        scene.push_quad(
            Quad::new(Rect::from_xywh(bounds.min_x() + half_w, bounds.min_y(), 1.0, bounds.height()))
                .with_background(theme.panel_border),
        );

        let visible_lines = (bounds.height() / font.line_height).ceil() as usize + 1;
        let start_row = (self.scroll_top.floor() as usize).min(self.model.side_by_side_rows.len().saturating_sub(1));
        let end_row = (start_row + visible_lines).min(self.model.side_by_side_rows.len());

        let added_bg = Rgba::new(0.18, 0.45, 0.25, 0.35);
        let deleted_bg = Rgba::new(0.65, 0.18, 0.18, 0.35);

        for row_idx in start_row..end_row {
            let row = &self.model.side_by_side_rows[row_idx];
            let row_y = bounds.min_y() + (row_idx as f32 - self.scroll_top) * font.line_height;

            // --- Left Pane (Original) ---
            let left_rect = Rect::from_xywh(bounds.min_x(), row_y, half_w, font.line_height);
            let left_gutter_rect = Rect::from_xywh(bounds.min_x(), row_y, gutter_w, font.line_height);
            scene.push_quad(Quad::new(left_gutter_rect).with_background(theme.editor_gutter_background));

            if let Some(left_cell) = &row.left {
                if left_cell.kind == DiffOpKind::Delete || left_cell.kind == DiffOpKind::Modify {
                    scene.push_quad(Quad::new(left_rect).with_background(deleted_bg));
                }

                if let Some(num) = left_cell.line_num {
                    let num_str = num.to_string();
                    let num_w = font.text_width(&num_str);
                    scene.push_text(GlyphRun::new(
                        num_str,
                        Point::new(bounds.min_x() + gutter_w - num_w - 4.0, row_y),
                        Rect::from_xywh(bounds.min_x() + gutter_w - num_w - 4.0, row_y, num_w, font.line_height),
                        theme.editor_line_number,
                        font.font_size * 0.9,
                    ));
                }

                let text_x = bounds.min_x() + gutter_w + 6.0;
                let text_w = half_w - gutter_w - 8.0;
                scene.push_text(GlyphRun::new(
                    left_cell.text.clone(),
                    Point::new(text_x, row_y),
                    Rect::from_xywh(text_x, row_y, text_w, font.line_height),
                    theme.editor_foreground,
                    font.font_size,
                ));
            } else {
                // Empty filler slot on left
                scene.push_quad(
                    Quad::new(left_rect).with_background(Rgba::new(0.0, 0.0, 0.0, 0.15)),
                );
            }

            // --- Right Pane (Modified) ---
            let right_x = bounds.min_x() + half_w + 1.0;
            let right_w = bounds.width() - half_w - 1.0;
            let right_rect = Rect::from_xywh(right_x, row_y, right_w, font.line_height);
            let right_gutter_rect = Rect::from_xywh(right_x, row_y, gutter_w, font.line_height);
            scene.push_quad(Quad::new(right_gutter_rect).with_background(theme.editor_gutter_background));

            if let Some(right_cell) = &row.right {
                if right_cell.kind == DiffOpKind::Insert || right_cell.kind == DiffOpKind::Modify {
                    scene.push_quad(Quad::new(right_rect).with_background(added_bg));
                }

                if let Some(num) = right_cell.line_num {
                    let num_str = num.to_string();
                    let num_w = font.text_width(&num_str);
                    scene.push_text(GlyphRun::new(
                        num_str,
                        Point::new(right_x + gutter_w - num_w - 4.0, row_y),
                        Rect::from_xywh(right_x + gutter_w - num_w - 4.0, row_y, num_w, font.line_height),
                        theme.editor_line_number,
                        font.font_size * 0.9,
                    ));
                }

                let text_x = right_x + gutter_w + 6.0;
                let text_w = right_w - gutter_w - 8.0;
                scene.push_text(GlyphRun::new(
                    right_cell.text.clone(),
                    Point::new(text_x, row_y),
                    Rect::from_xywh(text_x, row_y, text_w, font.line_height),
                    theme.editor_foreground,
                    font.font_size,
                ));
            } else {
                // Empty filler slot on right
                scene.push_quad(
                    Quad::new(right_rect).with_background(Rgba::new(0.0, 0.0, 0.0, 0.15)),
                );
            }
        }
    }

    fn render_inline(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        let single_gutter_w = (4.0 * font.char_width).max(32.0);
        let total_gutter_w = single_gutter_w * 2.0 + 16.0;

        let visible_lines = (bounds.height() / font.line_height).ceil() as usize + 1;
        let start_row = (self.scroll_top.floor() as usize).min(self.model.inline_rows.len().saturating_sub(1));
        let end_row = (start_row + visible_lines).min(self.model.inline_rows.len());

        let added_bg = Rgba::new(0.18, 0.45, 0.25, 0.35);
        let deleted_bg = Rgba::new(0.65, 0.18, 0.18, 0.35);

        for row_idx in start_row..end_row {
            let row = &self.model.inline_rows[row_idx];
            let row_y = bounds.min_y() + (row_idx as f32 - self.scroll_top) * font.line_height;
            let line_rect = Rect::from_xywh(bounds.min_x(), row_y, bounds.width(), font.line_height);

            // Row background based on kind
            match row.kind {
                DiffOpKind::Insert => {
                    scene.push_quad(Quad::new(line_rect).with_background(added_bg));
                }
                DiffOpKind::Delete => {
                    scene.push_quad(Quad::new(line_rect).with_background(deleted_bg));
                }
                _ => {}
            }

            // Gutter background
            let gutter_rect = Rect::from_xywh(bounds.min_x(), row_y, total_gutter_w, font.line_height);
            scene.push_quad(Quad::new(gutter_rect).with_background(theme.editor_gutter_background));

            // Old line number
            if let Some(num) = row.old_line {
                let s = num.to_string();
                let w = font.text_width(&s);
                scene.push_text(GlyphRun::new(
                    s,
                    Point::new(bounds.min_x() + single_gutter_w - w - 2.0, row_y),
                    Rect::from_xywh(bounds.min_x() + single_gutter_w - w - 2.0, row_y, w, font.line_height),
                    theme.editor_line_number,
                    font.font_size * 0.9,
                ));
            }

            // New line number
            if let Some(num) = row.new_line {
                let s = num.to_string();
                let w = font.text_width(&s);
                let x = bounds.min_x() + single_gutter_w * 2.0 - w - 2.0;
                scene.push_text(GlyphRun::new(
                    s,
                    Point::new(x, row_y),
                    Rect::from_xywh(x, row_y, w, font.line_height),
                    theme.editor_line_number,
                    font.font_size * 0.9,
                ));
            }

            // Prefix (+ / - / space)
            let prefix_str = row.prefix.to_string();
            let prefix_x = bounds.min_x() + single_gutter_w * 2.0 + 4.0;
            let prefix_color = match row.kind {
                DiffOpKind::Insert => theme.editor_info_foreground,
                DiffOpKind::Delete => theme.editor_error_foreground,
                _ => theme.editor_line_number,
            };
            scene.push_text(GlyphRun::new(
                prefix_str,
                Point::new(prefix_x, row_y),
                Rect::from_xywh(prefix_x, row_y, 12.0, font.line_height),
                prefix_color,
                font.font_size,
            ));

            // Text
            let text_x = bounds.min_x() + total_gutter_w + 6.0;
            let text_w = bounds.width() - total_gutter_w - 12.0;
            scene.push_text(GlyphRun::new(
                row.text.clone(),
                Point::new(text_x, row_y),
                Rect::from_xywh(text_x, row_y, text_w, font.line_height),
                theme.editor_foreground,
                font.font_size,
            ));
        }
    }
}
