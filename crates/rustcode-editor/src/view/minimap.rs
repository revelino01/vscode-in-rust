//! Minimap layout calculation and GPUI scene rendering for `EditorView`.

use crate::model::piece_table::PieceTable;
use crate::syntax::SyntaxHighlighter;
use crate::viewmodel::{DiagnosticSeverity, EditorDiagnostic};
use parking_lot::Mutex;
use rustcode_platform::render::{Edges, Quad, Rect, Scene, WorkbenchThemeStyles};

pub const DEFAULT_MINIMAP_WIDTH: f32 = 84.0;

#[derive(Debug, Clone, PartialEq)]
pub struct MinimapState {
    pub enabled: bool,
    pub width: f32,
    pub dragging: bool,
}

impl Default for MinimapState {
    fn default() -> Self {
        Self {
            enabled: true,
            width: DEFAULT_MINIMAP_WIDTH,
            dragging: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MinimapLayout {
    pub rect: Rect,
    pub line_count: usize,
    pub visible_lines: f32,
    pub line_step: f32,
    pub slider_y: f32,
    pub slider_h: f32,
    pub slider_travel: f32,
    pub is_compressed: bool,
}

impl MinimapLayout {
    pub fn compute(
        rect: Rect,
        line_count: usize,
        scroll_top: f32,
        viewport_height: f32,
        line_height: f32,
    ) -> Self {
        let line_count = line_count.max(1);
        let max_scroll = line_count.saturating_sub(1) as f32;
        let safe_line_height = if line_height.is_finite() {
            line_height.max(1.0)
        } else {
            1.0
        };
        let safe_viewport_height = if viewport_height.is_finite() {
            viewport_height.max(0.0)
        } else {
            0.0
        };
        let safe_scroll_top = if scroll_top.is_finite() {
            scroll_top.max(0.0)
        } else {
            0.0
        };
        let visible_lines = (safe_viewport_height / safe_line_height).max(1.0);
        let raw_min_x = if rect.min_x().is_finite() {
            rect.min_x()
        } else {
            0.0
        };
        let raw_w = if rect.width().is_finite() {
            rect.width().max(0.0)
        } else {
            0.0
        };
        let (safe_min_x, safe_w) = if (raw_min_x + raw_w).is_finite() {
            (raw_min_x, raw_w)
        } else {
            (0.0, 0.0)
        };
        let raw_rect_h = if rect.height().is_finite() {
            rect.height().max(1.0)
        } else {
            1.0
        };
        let raw_min_y = if rect.min_y().is_finite() {
            rect.min_y()
        } else {
            0.0
        };
        let (min_y, rect_h) = if (raw_min_y + raw_rect_h).is_finite() {
            (raw_min_y, raw_rect_h)
        } else {
            (0.0, 1.0)
        };
        let max_y = min_y + rect_h;
        let sanitized_rect = Rect::from_xywh(safe_min_x, min_y, safe_w, rect_h);
        let is_compressed = line_count as f32 * 3.0 > rect_h;

        let (line_step, mut slider_h, slider_travel, slider_y) = if !is_compressed {
            let line_step = 3.0;
            let slider_h = (visible_lines * line_step).clamp(12.0_f32.min(rect_h), rect_h);
            let slider_travel = (rect_h - slider_h).max(0.0);
            let raw_y = min_y + safe_scroll_top * line_step;
            let slider_y = if raw_y.is_finite() {
                raw_y.clamp(min_y, (max_y - slider_h).max(min_y))
            } else {
                min_y
            };
            (line_step, slider_h, slider_travel, slider_y)
        } else {
            let line_step = rect_h / line_count as f32;
            let slider_h = ((visible_lines / line_count as f32) * rect_h)
                .clamp(16.0_f32.min(rect_h), rect_h);
            let slider_travel = (rect_h - slider_h).max(0.0);
            let ratio = if max_scroll > 0.0 {
                (safe_scroll_top / max_scroll).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let raw_y = min_y + ratio * slider_travel;
            let slider_y = if raw_y.is_finite() { raw_y } else { min_y };
            (line_step, slider_h, slider_travel, slider_y)
        };
        let mut backoff = 0u8;
        while slider_h > 0.0 && !(slider_y + slider_h).is_finite() && backoff < 8 {
            slider_h = f32::from_bits(slider_h.to_bits() - 1);
            backoff += 1;
        }
        if !(slider_y + slider_h).is_finite() {
            slider_h = 0.0;
        }

        Self {
            rect: sanitized_rect,
            line_count,
            visible_lines,
            line_step,
            slider_y,
            slider_h,
            slider_travel,
            is_compressed,
        }
    }

    pub fn line_y(&self, line_idx: usize) -> f32 {
        let min_y = if self.rect.min_y().is_finite() {
            self.rect.min_y()
        } else {
            0.0
        };
        let step = if self.line_step.is_finite() && self.line_step > 0.0 {
            self.line_step
        } else {
            0.0
        };
        let y = min_y + line_idx as f32 * step;
        if y.is_finite() {
            y
        } else {
            min_y
        }
    }

    pub fn scroll_top_for_y(&self, y: f32) -> f32 {
        if !y.is_finite() {
            return 0.0;
        }
        let min_y = if self.rect.min_y().is_finite() {
            self.rect.min_y()
        } else {
            0.0
        };
        let max_scroll = self.line_count.saturating_sub(1) as f32;
        if !self.is_compressed {
            if !self.line_step.is_finite() || self.line_step <= 0.0 {
                return 0.0;
            }
            let raw = (y - min_y) / self.line_step;
            if raw.is_nan() {
                0.0
            } else {
                raw.clamp(0.0, max_scroll)
            }
        } else if self.slider_travel.is_finite() && self.slider_travel > 0.0 {
            let slider_h = if self.slider_h.is_finite() {
                self.slider_h
            } else {
                0.0
            };
            let ratio = (y - min_y - slider_h * 0.5) / self.slider_travel;
            if ratio.is_nan() {
                0.0
            } else {
                ratio.clamp(0.0, 1.0) * max_scroll
            }
        } else {
            0.0
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_minimap(
    layout: &MinimapLayout,
    buf: &PieceTable,
    syntax_highlighter: &Mutex<Option<SyntaxHighlighter>>,
    diagnostics: &[EditorDiagnostic],
    cursor_line: usize,
    dragging: bool,
    theme: &WorkbenchThemeStyles,
    scene: &mut Scene,
) {
    if !layout.rect.min_x().is_finite()
        || !layout.rect.min_y().is_finite()
        || !layout.rect.width().is_finite()
        || !layout.rect.height().is_finite()
        || !layout.rect.max_x().is_finite()
        || !layout.rect.max_y().is_finite()
        || layout.rect.width() <= 0.0
        || layout.rect.height() <= 0.0
        || !layout.line_step.is_finite()
        || layout.line_step <= 0.0
        || !layout.slider_y.is_finite()
        || !layout.slider_h.is_finite()
    {
        return;
    }

    scene.push_clip(layout.rect);

    // 1. Background quad with left separator border
    push_finite_minimap_quad(
        scene,
        Quad::new(layout.rect)
            .with_background(theme.minimap_background)
            .with_border_edges(theme.editor_gutter_border, Edges::new(0.0, 0.0, 0.0, 1.0)),
    );

    // 2. Proportional viewport slider quad
    let slider_rect = Rect::from_xywh(
        layout.rect.min_x() + 1.0,
        layout.slider_y,
        (layout.rect.width() - 1.0).max(0.0),
        layout.slider_h,
    );
    let slider_color = if dragging {
        theme.minimap_slider_hover_bg
    } else {
        theme.minimap_slider_bg
    };
    push_finite_minimap_quad(scene, Quad::new(slider_rect).with_background(slider_color));

    // 3. Syntax-highlighted micro-rects per sampled line
    let total_lines = buf.line_count();
    let max_rows = (layout.rect.height() / 2.0).ceil().max(1.0) as usize;
    let sample_step = if total_lines > max_rows {
        (total_lines as f32 / max_rows as f32).ceil().max(1.0) as usize
    } else {
        1
    };

    let content_x = layout.rect.min_x() + 4.0;
    let max_content_x = layout.rect.max_x() - 4.0;
    let max_w = (layout.rect.width() - 8.0).max(4.0);
    let col_scale = max_w / 80.0;
    let block_h = if layout.is_compressed {
        layout.line_step.clamp(1.0, 2.0)
    } else {
        2.0
    };

    if content_x.is_finite()
        && max_content_x.is_finite()
        && col_scale.is_finite()
        && block_h.is_finite()
    {
        let mut highlighter_guard = syntax_highlighter.lock();
        if let Some(highlighter) = highlighter_guard.as_mut() {
            let last_cached = highlighter.cache_len();
            if last_cached < total_lines {
                for l in last_cached..total_lines {
                    let content = buf.get_line_content(l).unwrap_or_default();
                    highlighter.get_line_tokens(l, &content);
                }
            }
        }

        for line_idx in (0..total_lines).step_by(sample_step) {
            let line_content = buf.get_line_content(line_idx).unwrap_or_default();
            if line_content.trim().is_empty() {
                continue;
            }
            let ry = layout.line_y(line_idx);
            if !ry.is_finite() || !(ry + block_h).is_finite() {
                continue;
            }
            let mut drew_tokens = false;

            if let Some(highlighter) = highlighter_guard.as_mut() {
                let tokens = highlighter.get_line_tokens(line_idx, &line_content);
                if !tokens.is_empty() {
                    for span in tokens {
                        let start = span.start.min(line_content.len());
                        let end = span.end.min(line_content.len());
                        if end <= start
                            || !line_content.is_char_boundary(start)
                            || !line_content.is_char_boundary(end)
                        {
                            continue;
                        }
                        let slice = &line_content[start..end];
                        if slice.trim().is_empty() {
                            continue;
                        }
                        let start_col = line_content[..start].chars().count() as f32;
                        let char_len = slice.chars().count() as f32;
                        let rx = content_x + start_col * col_scale;
                        if rx.is_finite() && rx < max_content_x {
                            let mut rw = (char_len * col_scale)
                                .max(2.0)
                                .min((max_content_x - rx).max(0.0));
                            if !rw.is_finite() {
                                rw = f32::MAX;
                            }
                            let mut backoff = 0u8;
                            while rw > 0.0 && !(rx + rw).is_finite() && backoff < 8 {
                                rw = f32::from_bits(rw.to_bits() - 1);
                                backoff += 1;
                            }
                            let quad_rect = Rect::from_xywh(rx, ry, rw, block_h);
                            if rw > 0.0 && is_finite_minimap_rect(quad_rect) {
                                let color =
                                    span.color.unwrap_or(theme.editor_foreground).with_alpha(0.65);
                                scene.push_quad(Quad::new(quad_rect).with_background(color));
                                drew_tokens = true;
                            }
                        }
                    }
                }
            }

            if !drew_tokens {
                render_plaintext_line_blocks(
                    &line_content,
                    content_x,
                    max_content_x,
                    ry,
                    block_h,
                    col_scale,
                    theme.editor_foreground.with_alpha(0.55),
                    scene,
                );
            }
        }
    }

    // 4. Diagnostic severity ticks on right margin
    for diag in diagnostics {
        let d_line = diag.range.start.line.min(layout.line_count.saturating_sub(1));
        let dy = layout.line_y(d_line);
        let color = match diag.severity {
            DiagnosticSeverity::Error => theme.editor_error_foreground,
            DiagnosticSeverity::Warning => theme.editor_warning_foreground,
            DiagnosticSeverity::Information => theme.editor_info_foreground,
            DiagnosticSeverity::Hint => theme.editor_hint_foreground,
        };
        push_finite_minimap_quad(
            scene,
            Quad::new(Rect::from_xywh(layout.rect.max_x() - 6.0, dy, 5.0, 2.5))
                .with_background(color),
        );
    }

    // 5. Cursor line indicator
    let cy = layout.line_y(cursor_line.min(layout.line_count.saturating_sub(1)));
    push_finite_minimap_quad(
        scene,
        Quad::new(Rect::from_xywh(
            layout.rect.min_x() + 1.0,
            cy,
            (layout.rect.width() - 2.0).max(0.0),
            2.0,
        ))
        .with_background(theme.editor_cursor.with_alpha(0.85)),
    );

    scene.pop_clip();
}

fn is_finite_minimap_rect(rect: Rect) -> bool {
    rect.min_x().is_finite()
        && rect.min_y().is_finite()
        && rect.width().is_finite()
        && rect.height().is_finite()
        && rect.max_x().is_finite()
        && rect.max_y().is_finite()
}

fn push_finite_minimap_quad(scene: &mut Scene, quad: Quad) {
    if is_finite_minimap_rect(quad.bounds) {
        scene.push_quad(quad);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_plaintext_line_blocks(
    line_content: &str,
    content_x: f32,
    max_content_x: f32,
    ry: f32,
    block_h: f32,
    col_scale: f32,
    color: rustcode_platform::render::Rgba,
    scene: &mut Scene,
) {
    let mut run_start: Option<usize> = None;
    let mut char_idx = 0usize;
    for ch in line_content.chars() {
        if ch.is_whitespace() {
            if let Some(start) = run_start.take() {
                push_block(
                    start,
                    char_idx - start,
                    content_x,
                    max_content_x,
                    ry,
                    block_h,
                    col_scale,
                    color,
                    scene,
                );
            }
        } else if run_start.is_none() {
            run_start = Some(char_idx);
        }
        char_idx += 1;
    }
    if let Some(start) = run_start {
        push_block(
            start,
            char_idx - start,
            content_x,
            max_content_x,
            ry,
            block_h,
            col_scale,
            color,
            scene,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_block(
    start_col: usize,
    char_len: usize,
    content_x: f32,
    max_content_x: f32,
    ry: f32,
    block_h: f32,
    col_scale: f32,
    color: rustcode_platform::render::Rgba,
    scene: &mut Scene,
) {
    let rx = content_x + start_col as f32 * col_scale;
    if rx.is_finite() && rx < max_content_x {
        let mut rw = (char_len as f32 * col_scale)
            .max(2.0)
            .min((max_content_x - rx).max(0.0));
        if !rw.is_finite() {
            rw = f32::MAX;
        }
        let mut backoff = 0u8;
        while rw > 0.0 && !(rx + rw).is_finite() && backoff < 8 {
            rw = f32::from_bits(rw.to_bits() - 1);
            backoff += 1;
        }
        let quad_rect = Rect::from_xywh(rx, ry, rw, block_h);
        if rw > 0.0 && is_finite_minimap_rect(quad_rect) {
            scene.push_quad(Quad::new(quad_rect).with_background(color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minimap_layout_compute_short_and_non_finite_heights_no_panic() {
        for height in [0.0_f32, 0.5, 5.0, 10.0, 12.0, 15.9, -4.0, f32::NAN, f32::INFINITY] {
            let rect = Rect::from_xywh(716.0, 0.0, 84.0, height);
            for line_count in [0, 1, 2, 300] {
                let layout = MinimapLayout::compute(rect, line_count, 0.0, height, 20.0);
                assert!(layout.slider_h.is_finite());
                assert!(layout.slider_y.is_finite());
                assert!(layout.slider_travel.is_finite());
                assert!(layout.slider_h >= 0.0);
                assert!(layout.slider_travel >= 0.0);
                if height.is_finite() && height >= 1.0 {
                    assert!(layout.slider_h <= height);
                }
            }
        }

        // Non-finite scroll/viewport/line_height/origin also must not panic
        let nan_rect = Rect::from_xywh(0.0, f32::NAN, 84.0, 10.0);
        let layout = MinimapLayout::compute(nan_rect, 100, f32::NAN, f32::NAN, f32::NAN);
        assert!(layout.slider_h.is_finite());
        assert!(layout.slider_y.is_finite());
        assert_eq!(layout.scroll_top_for_y(f32::NAN), 0.0);
    }

    #[test]
    fn test_minimap_layout_compute_sanitizes_non_finite_rect_and_outputs() {
        use rustcode_platform::render::{Point, Size};

        let non_finite_rects = [
            Rect::from_xywh(716.0, f32::NAN, 84.0, 200.0),
            Rect::from_xywh(716.0, f32::INFINITY, 84.0, 200.0),
            Rect::from_xywh(716.0, f32::NEG_INFINITY, 84.0, 200.0),
            Rect::from_xywh(f32::NAN, f32::INFINITY, 84.0, 200.0),
            Rect::from_xywh(f32::INFINITY, 0.0, f32::INFINITY, 200.0),
            Rect::new(Point::zero(), Size::new(84.0, f32::NAN)),
            Rect::new(Point::new(f32::MAX, f32::MAX), Size::new(100.0, 100.0)),
        ];

        for rect in non_finite_rects {
            for line_count in [0usize, 1, 300] {
                let layout = MinimapLayout::compute(rect, line_count, 5.0, 200.0, 20.0);
                assert!(layout.rect.min_x().is_finite());
                assert!(layout.rect.min_y().is_finite());
                assert!(layout.rect.width().is_finite());
                assert!(layout.rect.height().is_finite());
                assert!(layout.rect.max_x().is_finite());
                assert!(layout.rect.max_y().is_finite());
                assert!(layout.rect.width() >= 0.0);
                assert!(layout.rect.height() >= 1.0);

                let ly0 = layout.line_y(0);
                let ly10 = layout.line_y(10);
                assert!(ly0.is_finite() && ly0 >= 0.0);
                assert!(ly10.is_finite() && ly10 >= 0.0);

                for y in [0.0_f32, 50.0, -25.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                    let st = layout.scroll_top_for_y(y);
                    assert!(
                        st.is_finite() && st >= 0.0,
                        "expected finite >= 0.0 scroll_top_for_y({}), got {} for rect={:?}",
                        y,
                        st,
                        rect
                    );
                }
            }
        }

        // Direct render_minimap call with manually constructed non-finite layout must emit nothing and balance clips
        let bad_layout = MinimapLayout {
            rect: Rect::new(Point::zero(), Size::new(84.0, f32::NAN)),
            line_count: 10,
            visible_lines: 10.0,
            line_step: 3.0,
            slider_y: 0.0,
            slider_h: 20.0,
            slider_travel: 0.0,
            is_compressed: false,
        };
        let buf = PieceTable::from_str("fn main() {}");
        let sh = Mutex::new(None);
        let theme = WorkbenchThemeStyles::dark_default();
        let mut scene = Scene::new();
        render_minimap(&bad_layout, &buf, &sh, &[], 0, false, &theme, &mut scene);
        assert!(scene.is_empty());
    }

    #[test]
    fn test_render_minimap_f32_max_width_no_non_finite_quads() {
        use crate::syntax::GrammarRegistry;
        use rustcode_platform::render::{Point, SceneNode, Size};
        use rustcode_platform::theme::ThemeService;

        let lines: String = (0..300)
            .map(|i| format!("let variable_name_{} = \"value_{}\";\n", i, i))
            .collect();
        let buf = PieceTable::from_str(&lines);
        let registry = GrammarRegistry::default();
        let theme_svc = ThemeService::default_dark_theme();
        let sh_rust = Mutex::new(
            registry
                .get_by_id("rust")
                .map(|g| SyntaxHighlighter::new(g, theme_svc)),
        );
        let sh_plain = Mutex::new(None);
        let theme = WorkbenchThemeStyles::dark_default();
        let diags = vec![EditorDiagnostic::error(
            crate::cursor::Range::new(
                crate::cursor::Position::new(10, 0),
                crate::cursor::Position::new(10, 5),
            ),
            "err",
        )];

        for rect in [
            Rect::new(Point::new(716.0, 0.0), Size::new(f32::MAX, 200.0)),
            Rect::from_xywh(716.0, 0.0, f32::MAX, f32::MAX),
            Rect::new(Point::new(0.0, 0.0), Size::new(f32::MAX, 1e38)),
        ] {
            let layout = MinimapLayout::compute(rect, buf.line_count(), 0.0, rect.height(), 20.0);
            for sh in [&sh_plain, &sh_rust] {
                let mut scene = Scene::new();
                render_minimap(&layout, &buf, sh, &diags, 5, false, &theme, &mut scene);
                let mut clip_depth = 0isize;
                for node in scene.nodes() {
                    match node {
                        SceneNode::PushClip(r) => {
                            clip_depth += 1;
                            assert!(is_finite_minimap_rect(*r), "non-finite clip: {:?}", r);
                        }
                        SceneNode::PopClip => {
                            clip_depth -= 1;
                            assert!(clip_depth >= 0);
                        }
                        SceneNode::Quad(q) => {
                            assert!(
                                is_finite_minimap_rect(q.bounds),
                                "non-finite Quad bounds in minimap: {:?} (max_x={}, max_y={})",
                                q.bounds,
                                q.bounds.max_x(),
                                q.bounds.max_y()
                            );
                        }
                        _ => {}
                    }
                }
                assert_eq!(clip_depth, 0);
            }
        }
    }
}
