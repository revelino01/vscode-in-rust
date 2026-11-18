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
        if !is_finite_positive_rect(bounds) || !is_finite_font(font) {
            return;
        }

        // 1. Overall background
        push_finite_quad(scene, Quad::new(bounds).with_background(theme.editor_background));

        // 2. Toolbar Header (height 28.0)
        let header_h = 28.0;
        let header_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), header_h);
        push_finite_quad(
            scene,
            Quad::new(header_rect)
                .with_background(theme.panel_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        // Title
        let title_w = font.text_width(&self.title);
        push_finite_text(
            scene,
            GlyphRun::new(
                self.title.clone(),
                Point::new(bounds.min_x() + 10.0, bounds.min_y() + 5.0),
                Rect::from_xywh(bounds.min_x() + 10.0, bounds.min_y() + 5.0, title_w, font.line_height),
                theme.editor_foreground,
                font.font_size * 0.9,
            ),
        );

        // Stats badge: "+X -Y"
        let stats_text = format!("+{} -{}", self.model.additions, self.model.deletions);
        let stats_w = font.text_width(&stats_text);
        let stats_x = bounds.min_x() + 20.0 + title_w;
        push_finite_text(
            scene,
            GlyphRun::new(
                stats_text,
                Point::new(stats_x, bounds.min_y() + 5.0),
                Rect::from_xywh(stats_x, bounds.min_y() + 5.0, stats_w, font.line_height),
                theme.editor_info_foreground,
                font.font_size * 0.85,
            ),
        );

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
        if is_finite_rect(btn_rect) {
            *self.header_button_bounds.write() = Some(btn_rect);
            push_finite_quad(
                scene,
                Quad::new(btn_rect)
                    .with_background(theme.command_palette_selected_bg)
                    .with_border_edges(theme.panel_border, Edges::all(1.0))
                    .with_corner_radius(3.0),
            );
            push_finite_text(
                scene,
                GlyphRun::new(
                    mode_label.to_string(),
                    Point::new(btn_rect.min_x() + 8.0, btn_rect.min_y() + 3.0),
                    Rect::from_xywh(
                        btn_rect.min_x() + 8.0,
                        btn_rect.min_y() + 3.0,
                        (mode_w - 16.0).max(0.0),
                        font.line_height,
                    ),
                    theme.editor_foreground,
                    font.font_size * 0.8,
                ),
            );
        } else {
            *self.header_button_bounds.write() = None;
        }

        // 3. Diff Content Area
        let content_y = bounds.min_y() + header_h;
        let content_h = (bounds.height() - header_h).max(0.0);
        let content_rect = Rect::from_xywh(bounds.min_x(), content_y, bounds.width(), content_h);
        if !is_finite_positive_rect(content_rect) {
            return;
        }

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
        if !is_finite_positive_rect(bounds) {
            return EventResult::Ignored;
        }
        match event {
            InputEvent::MouseDown(mouse) => {
                if !mouse.position.x.is_finite()
                    || !mouse.position.y.is_finite()
                    || !bounds.contains(mouse.position)
                {
                    return EventResult::Ignored;
                }
                let clicked = self
                    .header_button_bounds
                    .read()
                    .is_some_and(|b| is_finite_positive_rect(b) && b.contains(mouse.position));
                if clicked {
                    self.toggle_mode();
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            InputEvent::ScrollWheel(scroll) => {
                if !scroll.position.x.is_finite()
                    || !scroll.position.y.is_finite()
                    || !bounds.contains(scroll.position)
                {
                    return EventResult::Ignored;
                }
                let total_rows = match self.mode {
                    DiffViewMode::SideBySide => self.model.side_by_side_rows.len(),
                    DiffViewMode::Inline => self.model.inline_rows.len(),
                };
                let max_scroll = (total_rows as f32 - 5.0).max(0.0);
                let safe_scroll = if self.scroll_top.is_finite() {
                    self.scroll_top.clamp(0.0, max_scroll)
                } else {
                    0.0
                };
                if !scroll.delta_y.is_finite() {
                    self.scroll_top = safe_scroll;
                    return EventResult::Ignored;
                }
                let next = safe_scroll + scroll.delta_y;
                self.scroll_top = if next.is_nan() {
                    0.0
                } else {
                    next.clamp(0.0, max_scroll)
                };
                EventResult::Consumed
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
        if !half_w.is_finite() || !gutter_w.is_finite() {
            return;
        }

        // Divider down the middle
        push_finite_quad(
            scene,
            Quad::new(Rect::from_xywh(bounds.min_x() + half_w, bounds.min_y(), 1.0, bounds.height()))
                .with_background(theme.panel_border),
        );

        let max_scroll = (self.model.side_by_side_rows.len() as f32 - 5.0).max(0.0);
        let scroll_top = if self.scroll_top.is_finite() {
            self.scroll_top.clamp(0.0, max_scroll)
        } else {
            0.0
        };

        let visible_lines = (bounds.height() / font.line_height)
            .ceil()
            .clamp(0.0, usize::MAX as f32) as usize;
        let visible_lines = visible_lines.saturating_add(1);
        let start_row = (scroll_top.floor() as usize)
            .min(self.model.side_by_side_rows.len().saturating_sub(1));
        let end_row = start_row
            .saturating_add(visible_lines)
            .min(self.model.side_by_side_rows.len());

        let added_bg = Rgba::new(0.18, 0.45, 0.25, 0.35);
        let deleted_bg = Rgba::new(0.65, 0.18, 0.18, 0.35);

        for row_idx in start_row..end_row {
            let row = &self.model.side_by_side_rows[row_idx];
            let row_y = bounds.min_y() + (row_idx as f32 - scroll_top) * font.line_height;
            if !row_y.is_finite() || !(row_y + font.line_height).is_finite() {
                if row_y.is_nan() || row_y > 0.0 {
                    break;
                }
                continue;
            }

            // --- Left Pane (Original) ---
            let left_rect = Rect::from_xywh(bounds.min_x(), row_y, half_w, font.line_height);
            let left_gutter_rect = Rect::from_xywh(bounds.min_x(), row_y, gutter_w, font.line_height);
            push_finite_quad(
                scene,
                Quad::new(left_gutter_rect).with_background(theme.editor_gutter_background),
            );

            if let Some(left_cell) = &row.left {
                if left_cell.kind == DiffOpKind::Delete || left_cell.kind == DiffOpKind::Modify {
                    push_finite_quad(scene, Quad::new(left_rect).with_background(deleted_bg));
                }

                if let Some(num) = left_cell.line_num {
                    let num_str = num.to_string();
                    let num_w = font.text_width(&num_str);
                    push_finite_text(
                        scene,
                        GlyphRun::new(
                            num_str,
                            Point::new(bounds.min_x() + gutter_w - num_w - 4.0, row_y),
                            Rect::from_xywh(
                                bounds.min_x() + gutter_w - num_w - 4.0,
                                row_y,
                                num_w,
                                font.line_height,
                            ),
                            theme.editor_line_number,
                            font.font_size * 0.9,
                        ),
                    );
                }

                let text_x = bounds.min_x() + gutter_w + 6.0;
                let text_w = half_w - gutter_w - 8.0;
                push_finite_text(
                    scene,
                    GlyphRun::new(
                        left_cell.text.clone(),
                        Point::new(text_x, row_y),
                        Rect::from_xywh(text_x, row_y, text_w, font.line_height),
                        theme.editor_foreground,
                        font.font_size,
                    ),
                );
            } else {
                // Empty filler slot on left
                push_finite_quad(
                    scene,
                    Quad::new(left_rect).with_background(Rgba::new(0.0, 0.0, 0.0, 0.15)),
                );
            }

            // --- Right Pane (Modified) ---
            let right_x = bounds.min_x() + half_w + 1.0;
            let right_w = bounds.width() - half_w - 1.0;
            let right_rect = Rect::from_xywh(right_x, row_y, right_w, font.line_height);
            let right_gutter_rect = Rect::from_xywh(right_x, row_y, gutter_w, font.line_height);
            push_finite_quad(
                scene,
                Quad::new(right_gutter_rect).with_background(theme.editor_gutter_background),
            );

            if let Some(right_cell) = &row.right {
                if right_cell.kind == DiffOpKind::Insert || right_cell.kind == DiffOpKind::Modify {
                    push_finite_quad(scene, Quad::new(right_rect).with_background(added_bg));
                }

                if let Some(num) = right_cell.line_num {
                    let num_str = num.to_string();
                    let num_w = font.text_width(&num_str);
                    push_finite_text(
                        scene,
                        GlyphRun::new(
                            num_str,
                            Point::new(right_x + gutter_w - num_w - 4.0, row_y),
                            Rect::from_xywh(
                                right_x + gutter_w - num_w - 4.0,
                                row_y,
                                num_w,
                                font.line_height,
                            ),
                            theme.editor_line_number,
                            font.font_size * 0.9,
                        ),
                    );
                }

                let text_x = right_x + gutter_w + 6.0;
                let text_w = right_w - gutter_w - 8.0;
                push_finite_text(
                    scene,
                    GlyphRun::new(
                        right_cell.text.clone(),
                        Point::new(text_x, row_y),
                        Rect::from_xywh(text_x, row_y, text_w, font.line_height),
                        theme.editor_foreground,
                        font.font_size,
                    ),
                );
            } else {
                // Empty filler slot on right
                push_finite_quad(
                    scene,
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
        if !single_gutter_w.is_finite() || !total_gutter_w.is_finite() {
            return;
        }

        let max_scroll = (self.model.inline_rows.len() as f32 - 5.0).max(0.0);
        let scroll_top = if self.scroll_top.is_finite() {
            self.scroll_top.clamp(0.0, max_scroll)
        } else {
            0.0
        };

        let visible_lines = (bounds.height() / font.line_height)
            .ceil()
            .clamp(0.0, usize::MAX as f32) as usize;
        let visible_lines = visible_lines.saturating_add(1);
        let start_row = (scroll_top.floor() as usize)
            .min(self.model.inline_rows.len().saturating_sub(1));
        let end_row = start_row
            .saturating_add(visible_lines)
            .min(self.model.inline_rows.len());

        let added_bg = Rgba::new(0.18, 0.45, 0.25, 0.35);
        let deleted_bg = Rgba::new(0.65, 0.18, 0.18, 0.35);

        for row_idx in start_row..end_row {
            let row = &self.model.inline_rows[row_idx];
            let row_y = bounds.min_y() + (row_idx as f32 - scroll_top) * font.line_height;
            if !row_y.is_finite() || !(row_y + font.line_height).is_finite() {
                if row_y.is_nan() || row_y > 0.0 {
                    break;
                }
                continue;
            }
            let line_rect = Rect::from_xywh(bounds.min_x(), row_y, bounds.width(), font.line_height);

            // Row background based on kind
            match row.kind {
                DiffOpKind::Insert => {
                    push_finite_quad(scene, Quad::new(line_rect).with_background(added_bg));
                }
                DiffOpKind::Delete => {
                    push_finite_quad(scene, Quad::new(line_rect).with_background(deleted_bg));
                }
                _ => {}
            }

            // Gutter background
            let gutter_rect = Rect::from_xywh(bounds.min_x(), row_y, total_gutter_w, font.line_height);
            push_finite_quad(
                scene,
                Quad::new(gutter_rect).with_background(theme.editor_gutter_background),
            );

            // Old line number
            if let Some(num) = row.old_line {
                let s = num.to_string();
                let w = font.text_width(&s);
                push_finite_text(
                    scene,
                    GlyphRun::new(
                        s,
                        Point::new(bounds.min_x() + single_gutter_w - w - 2.0, row_y),
                        Rect::from_xywh(
                            bounds.min_x() + single_gutter_w - w - 2.0,
                            row_y,
                            w,
                            font.line_height,
                        ),
                        theme.editor_line_number,
                        font.font_size * 0.9,
                    ),
                );
            }

            // New line number
            if let Some(num) = row.new_line {
                let s = num.to_string();
                let w = font.text_width(&s);
                let x = bounds.min_x() + single_gutter_w * 2.0 - w - 2.0;
                push_finite_text(
                    scene,
                    GlyphRun::new(
                        s,
                        Point::new(x, row_y),
                        Rect::from_xywh(x, row_y, w, font.line_height),
                        theme.editor_line_number,
                        font.font_size * 0.9,
                    ),
                );
            }

            // Prefix (+ / - / space)
            let prefix_str = row.prefix.to_string();
            let prefix_x = bounds.min_x() + single_gutter_w * 2.0 + 4.0;
            let prefix_color = match row.kind {
                DiffOpKind::Insert => theme.editor_info_foreground,
                DiffOpKind::Delete => theme.editor_error_foreground,
                _ => theme.editor_line_number,
            };
            push_finite_text(
                scene,
                GlyphRun::new(
                    prefix_str,
                    Point::new(prefix_x, row_y),
                    Rect::from_xywh(prefix_x, row_y, 12.0, font.line_height),
                    prefix_color,
                    font.font_size,
                ),
            );

            // Text
            let text_x = bounds.min_x() + total_gutter_w + 6.0;
            let text_w = bounds.width() - total_gutter_w - 12.0;
            push_finite_text(
                scene,
                GlyphRun::new(
                    row.text.clone(),
                    Point::new(text_x, row_y),
                    Rect::from_xywh(text_x, row_y, text_w, font.line_height),
                    theme.editor_foreground,
                    font.font_size,
                ),
            );
        }
    }
}

fn is_finite_rect(rect: Rect) -> bool {
    rect.min_x().is_finite()
        && rect.min_y().is_finite()
        && rect.width().is_finite()
        && rect.height().is_finite()
        && rect.max_x().is_finite()
        && rect.max_y().is_finite()
}

fn is_finite_positive_rect(rect: Rect) -> bool {
    is_finite_rect(rect) && rect.width() > 0.0 && rect.height() > 0.0
}

fn is_finite_font(font: &FontMetrics) -> bool {
    font.line_height.is_finite()
        && font.line_height > 0.0
        && font.char_width.is_finite()
        && font.char_width >= 0.0
        && font.font_size.is_finite()
        && font.font_size >= 0.0
}

fn push_finite_quad(scene: &mut Scene, quad: Quad) {
    if is_finite_rect(quad.bounds) {
        scene.push_quad(quad);
    }
}

fn push_finite_text(scene: &mut Scene, run: GlyphRun) {
    if run.origin.x.is_finite()
        && run.origin.y.is_finite()
        && is_finite_rect(run.bounds)
        && run.font_size.is_finite()
        && run.font_size >= 0.0
    {
        scene.push_text(run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::render::{SceneNode, ScrollEvent, Size};

    #[test]
    fn test_diff_editor_view_non_finite_bounds_scroll_and_events_safe() {
        let mut diff = DiffEditorView::new(
            "test.rs",
            "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7",
            "line 1\nline 2 modified\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8",
        );
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let assert_scene_finite = |scene: &Scene, label: &str| {
            let mut clip_depth = 0isize;
            for node in scene.nodes() {
                match node {
                    SceneNode::PushClip(r) => {
                        clip_depth += 1;
                        assert!(is_finite_rect(*r), "{}: non-finite PushClip {:?}", label, r);
                    }
                    SceneNode::PopClip => {
                        clip_depth -= 1;
                        assert!(clip_depth >= 0, "{}: clip stack underflow", label);
                    }
                    SceneNode::Quad(q) => {
                        assert!(is_finite_rect(q.bounds), "{}: non-finite Quad {:?}", label, q.bounds);
                    }
                    SceneNode::GlyphRun(g) => {
                        assert!(
                            g.origin.x.is_finite() && g.origin.y.is_finite(),
                            "{}: non-finite GlyphRun origin {:?}",
                            label,
                            g.origin
                        );
                        assert!(
                            is_finite_rect(g.bounds),
                            "{}: non-finite GlyphRun bounds {:?}",
                            label,
                            g.bounds
                        );
                    }
                    SceneNode::Shadow(s) => {
                        assert!(is_finite_rect(s.bounds), "{}: non-finite Shadow {:?}", label, s.bounds);
                    }
                }
            }
            assert_eq!(clip_depth, 0, "{}: unbalanced clip stack", label);
        };

        // 1. 800xInf and 800xNaN emit nothing and do not panic
        for bad_bounds in [
            Rect::from_xywh(0.0, 0.0, 800.0, f32::INFINITY),
            Rect::new(Point::zero(), Size::new(800.0, f32::NAN)),
            Rect::from_xywh(f32::NAN, 0.0, 800.0, 600.0),
            Rect::from_xywh(0.0, 0.0, 0.0, 600.0),
        ] {
            let mut scene = Scene::new();
            diff.render(bad_bounds, &theme, &font, &mut scene);
            assert!(scene.is_empty(), "expected empty scene for {:?}", bad_bounds);
        }

        // 2. Invalid FontMetrics emit nothing
        let valid_bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        for bad_font in [
            FontMetrics {
                line_height: 0.0,
                ..font.clone()
            },
            FontMetrics {
                line_height: f32::NAN,
                ..font.clone()
            },
            FontMetrics {
                char_width: -1.0,
                ..font.clone()
            },
            FontMetrics {
                font_size: f32::INFINITY,
                ..font.clone()
            },
        ] {
            let mut scene = Scene::new();
            diff.render(valid_bounds, &theme, &bad_font, &mut scene);
            assert!(scene.is_empty());
        }

        // 3. scroll_top = 1e38 (and NaN) on 800x600 emits zero non-finite nodes in both modes
        for mode in [DiffViewMode::SideBySide, DiffViewMode::Inline] {
            diff.set_mode(mode);
            for st in [1e38_f32, f32::NAN, f32::INFINITY, -1e38_f32] {
                diff.scroll_top = st;
                let mut scene = Scene::new();
                diff.render(valid_bounds, &theme, &font, &mut scene);
                assert_scene_finite(&scene, &format!("mode={:?} st={}", mode, st));
                assert!(!scene.is_empty());
            }
        }

        // 4. ScrollWheel with NaN and +/-Infinity never poisons scroll_top
        diff.scroll_top = 2.0;
        for bad_dy in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            diff.handle_event(
                &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(400.0, 300.0), 0.0, bad_dy)),
                valid_bounds,
            );
            assert!(diff.scroll_top.is_finite() && diff.scroll_top >= 0.0);
        }

        // MouseDown with non-finite position or bounds is safely ignored
        let res = diff.handle_event(
            &InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
                Point::new(f32::NAN, 10.0),
                rustcode_platform::render::MouseButton::Left,
            )),
            valid_bounds,
        );
        assert_eq!(res, EventResult::Ignored);
    }
}
