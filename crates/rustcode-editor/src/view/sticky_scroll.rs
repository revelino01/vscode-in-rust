//! Sticky Scroll pinned scope headers for `EditorView`.

use crate::model::piece_table::PieceTable;
use rustcode_platform::render::{
    Edges, FontMetrics, GlyphRun, Point, Quad, Rect, Rgba, Scene, Shadow, WorkbenchThemeStyles,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickyScrollConfig {
    pub enabled: bool,
    pub max_lines: usize,
}

impl Default for StickyScrollConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_lines: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickyScrollLine {
    pub buffer_line: usize,
    pub text: String,
    pub indent: usize,
}

fn is_scope_header(trimmed: &str, lang_id: Option<&str>) -> bool {
    if trimmed.is_empty() {
        return false;
    }
    let is_python = matches!(lang_id, Some("python"));
    if is_python {
        return trimmed.starts_with("def ")
            || trimmed.starts_with("async def ")
            || trimmed.starts_with("class ");
    }
    if trimmed.ends_with('{') || trimmed.contains(") {") || trimmed.ends_with("):") {
        return trimmed.starts_with("pub ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("async fn ")
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("enum ")
            || trimmed.starts_with("impl")
            || trimmed.starts_with("trait ")
            || trimmed.starts_with("mod ")
            || trimmed.starts_with("function ")
            || trimmed.starts_with("class ")
            || trimmed.starts_with("interface ")
            || trimmed.starts_with("namespace ")
            || trimmed.starts_with("func ")
            || trimmed.starts_with("export ");
    }
    false
}

/// Compute pinned enclosing scope headers above `top_line` using a bounded backward scan.
/// Returns an empty list at `top_line == 0` so top-of-file coordinates stay unchanged.
pub fn compute_sticky_scroll_lines(
    buf: &PieceTable,
    top_line: usize,
    max_lines: usize,
    lang_id: Option<&str>,
) -> Vec<StickyScrollLine> {
    if top_line == 0 || max_lines == 0 {
        return Vec::new();
    }
    let line_count = buf.line_count();
    let target_line = top_line.min(line_count.saturating_sub(1));
    let scan_start = target_line.saturating_sub(500);

    // Track stack of indentation levels & enclosing scope headers up to target_line
    let mut stack: Vec<StickyScrollLine> = Vec::new();
    for line_idx in scan_start..=target_line {
        let Some(raw) = buf.get_line_content(line_idx) else {
            continue;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }
        let indent = raw
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .map(|c| if c == '\t' { 4 } else { 1 })
            .sum::<usize>();

        // Pop scopes closed by equal or lower indentation before target_line
        if line_idx < target_line && (trimmed.starts_with('}') || !stack.is_empty()) {
            while let Some(top) = stack.last() {
                if indent <= top.indent && (trimmed.starts_with('}') || line_idx > top.buffer_line)
                {
                    stack.pop();
                } else {
                    break;
                }
            }
        }

        if line_idx < target_line && is_scope_header(trimmed, lang_id) {
            stack.push(StickyScrollLine {
                buffer_line: line_idx,
                text: raw.trim_end().to_string(),
                indent,
            });
        }
    }

    if stack.len() > max_lines {
        let drop_n = stack.len() - max_lines;
        stack.drain(0..drop_n);
    }
    stack
}

#[allow(clippy::too_many_arguments)]
pub fn render_sticky_scroll(
    lines: &[StickyScrollLine],
    bounds: Rect,
    gutter_w: f32,
    code_area_w: f32,
    scroll_left: f32,
    theme: &WorkbenchThemeStyles,
    font: &FontMetrics,
    scene: &mut Scene,
) -> f32 {
    if lines.is_empty() || !bounds.is_finite_positive() || !font.is_finite() {
        return 0.0;
    }
    let total_h = (lines.len() as f32 * font.line_height).min((bounds.height() * 0.4).max(font.line_height));
    let panel_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), total_h);
    if !panel_rect.is_finite_positive() {
        return 0.0;
    }

    scene.push_finite_shadow(Shadow {
        bounds: panel_rect,
        offset: Point::new(0.0, 2.0),
        blur: 4.0,
        spread: 0.0,
        color: Rgba::new(0.0, 0.0, 0.0, 0.28),
    });
    scene.push_finite_quad(
        Quad::new(panel_rect)
            .with_background(theme.editor_background)
            .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
    );

    let text_x = bounds.min_x() + gutter_w + 8.0 - scroll_left * font.char_width;
    for (idx, item) in lines.iter().enumerate() {
        let y = bounds.min_y() + idx as f32 * font.line_height;
        if y + font.line_height > panel_rect.max_y() + 0.5 {
            break;
        }
        let line_num = (item.buffer_line + 1).to_string();
        let num_w = font.text_width(&line_num);
        let num_x = (bounds.min_x() + gutter_w - 20.0 - num_w).max(bounds.min_x() + 4.0);
        scene.push_finite_text(GlyphRun::new(
            line_num,
            Point::new(num_x, y),
            Rect::from_xywh(num_x, y, num_w, font.line_height),
            theme.editor_line_number,
            font.font_size,
        ));
        scene.push_finite_text(GlyphRun::new(
            item.text.clone(),
            Point::new(text_x, y),
            Rect::from_xywh(
                bounds.min_x() + gutter_w,
                y,
                code_area_w.max(16.0),
                font.line_height,
            ),
            theme.editor_foreground,
            font.font_size,
        ));
    }

    total_h
}

pub fn hit_test_sticky_scroll(
    point: Point,
    lines: &[StickyScrollLine],
    bounds: Rect,
    font: &FontMetrics,
) -> Option<usize> {
    if lines.is_empty() || !bounds.is_finite_positive() || !font.is_finite() {
        return None;
    }
    let total_h = (lines.len() as f32 * font.line_height).min((bounds.height() * 0.4).max(font.line_height));
    let panel_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), total_h);
    if !panel_rect.contains(point) {
        return None;
    }
    let idx = ((point.y - bounds.min_y()) / font.line_height).floor().max(0.0) as usize;
    lines.get(idx).map(|l| l.buffer_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sticky_scroll_enclosing_scopes_and_hit_test() {
        let src = "mod outer {\n    struct Foo {\n        x: i32,\n    }\n    impl Foo {\n        pub fn bar(&self) {\n            let a = 1;\n            let b = 2;\n            let c = 3;\n        }\n    }\n}\n";
        let buf = PieceTable::from_str(src);
        assert!(compute_sticky_scroll_lines(&buf, 0, 5, Some("rust")).is_empty());

        let sticky = compute_sticky_scroll_lines(&buf, 7, 5, Some("rust"));
        assert!(sticky.iter().any(|l| l.buffer_line == 0)); // mod outer
        assert!(sticky.iter().any(|l| l.buffer_line == 4)); // impl Foo
        assert!(sticky.iter().any(|l| l.buffer_line == 5)); // pub fn bar

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let font = FontMetrics::default_monospace();
        let target = hit_test_sticky_scroll(Point::new(100.0, font.line_height * 0.5), &sticky, bounds, &font);
        assert_eq!(target, Some(0));
    }
}
