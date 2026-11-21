//! In-editor Find and Replace widget, match engine, and layout/rendering helpers.

use crate::cursor::{Position, Range};
use crate::model::piece_table::PieceTable;
use regex::{Regex, RegexBuilder};
use rustcode_platform::accessibility::{AccessibleNode, AccessibleRole};
use rustcode_platform::render::{
    FontMetrics, GlyphRun, Point, Quad, Rect, Rgba, Scene, Shadow, WorkbenchThemeStyles,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FindReplaceFocus {
    #[default]
    FindInput,
    ReplaceInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindMatch {
    /// UTF-8 byte offset of match start in `PieceTable`.
    pub start_offset: usize,
    /// UTF-8 byte offset of match end in `PieceTable`.
    pub end_offset: usize,
    /// Line/column range (UTF-16 columns).
    pub range: Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindReplaceClickTarget {
    Widget,
    FindInput,
    ReplaceInput,
    MatchCase,
    WholeWord,
    Regex,
    PrevMatch,
    NextMatch,
    Close,
    ReplaceOne,
    ReplaceAll,
}

#[derive(Debug, Clone)]
pub struct FindReplaceLayout {
    pub widget_rect: Rect,
    pub find_input_rect: Rect,
    pub case_rect: Rect,
    pub whole_word_rect: Rect,
    pub regex_rect: Rect,
    pub status_rect: Rect,
    pub prev_rect: Rect,
    pub next_rect: Rect,
    pub close_rect: Rect,
    pub replace_input_rect: Option<Rect>,
    pub replace_one_rect: Option<Rect>,
    pub replace_all_rect: Option<Rect>,
}

#[derive(Debug, Clone, Default)]
pub struct FindReplaceWidget {
    pub is_visible: bool,
    pub show_replace: bool,
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub is_regex: bool,
    pub focus: FindReplaceFocus,
    pub matches: Vec<FindMatch>,
    pub current_match: Option<usize>,
    pub error: Option<String>,
    pub search_origin_offset: usize,
}

impl FindReplaceWidget {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_find(&mut self, prefill: Option<String>, origin_offset: usize) {
        let was_visible = self.is_visible;
        self.is_visible = true;
        self.show_replace = false;
        self.focus = FindReplaceFocus::FindInput;
        self.search_origin_offset = origin_offset;
        if let Some(seed) = prefill {
            self.query = seed;
            self.error = None;
        } else if !was_visible {
            self.query.clear();
            self.error = None;
            self.matches.clear();
            self.current_match = None;
        }
    }

    pub fn open_replace(&mut self, prefill: Option<String>, origin_offset: usize) {
        let was_visible = self.is_visible;
        self.is_visible = true;
        self.show_replace = true;
        self.focus = FindReplaceFocus::FindInput;
        self.search_origin_offset = origin_offset;
        if let Some(seed) = prefill {
            self.query = seed;
            self.error = None;
        } else if !was_visible {
            self.query.clear();
            self.error = None;
            self.matches.clear();
            self.current_match = None;
        }
    }

    pub fn close(&mut self) {
        self.is_visible = false;
        self.query.clear();
        self.replacement.clear();
        self.matches.clear();
        self.current_match = None;
        self.error = None;
        self.focus = FindReplaceFocus::FindInput;
    }

    pub fn status_text(&self) -> String {
        if self.error.is_some() {
            return "Invalid regex".to_string();
        }
        if self.matches.is_empty() {
            return "No results".to_string();
        }
        let idx = self
            .current_match
            .unwrap_or(0)
            .min(self.matches.len() - 1);
        format!("{} of {}", idx + 1, self.matches.len())
    }

    pub fn build_regex(&self) -> Result<Regex, String> {
        let pattern = if self.is_regex {
            self.query.clone()
        } else {
            regex::escape(&self.query)
        };
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive)
            .multi_line(true)
            .crlf(true)
            .build()
            .map_err(|e| format!("Invalid regex: {}", e))
    }

    pub fn compute_matches(&mut self, buf: &PieceTable) {
        self.matches.clear();
        self.current_match = None;
        self.error = None;

        if self.query.is_empty() {
            return;
        }

        let re = match self.build_regex() {
            Ok(r) => r,
            Err(err) => {
                self.error = Some(err);
                return;
            }
        };

        let text = buf.to_string();
        let mut pos = 0usize;
        while pos <= text.len() {
            let Some(m) = re.find_at(&text, pos) else {
                break;
            };
            let start = m.start();
            let end = m.end();

            if !self.whole_word || passes_whole_word(&text, start, end) {
                let (sl, sc) = buf.offset_to_position(start);
                let (el, ec) = buf.offset_to_position(end);
                self.matches.push(FindMatch {
                    start_offset: start,
                    end_offset: end,
                    range: Range::new(Position::new(sl, sc), Position::new(el, ec)),
                });
            }

            if end > start {
                pos = end;
            } else {
                if start >= text.len() {
                    break;
                }
                pos = advance_past_zero_width(&text, start);
            }
        }

        if !self.matches.is_empty() {
            let idx = self
                .matches
                .iter()
                .position(|m| m.start_offset >= self.search_origin_offset)
                .unwrap_or(0);
            self.current_match = Some(idx);
        }
    }

    pub fn next_match(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            self.current_match = None;
            return None;
        }
        let next = match self.current_match {
            Some(i) => (i + 1) % self.matches.len(),
            None => 0,
        };
        self.current_match = Some(next);
        self.search_origin_offset = self.matches[next].start_offset;
        Some(next)
    }

    pub fn prev_match(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            self.current_match = None;
            return None;
        }
        let prev = match self.current_match {
            Some(0) | None => self.matches.len() - 1,
            Some(i) => i - 1,
        };
        self.current_match = Some(prev);
        self.search_origin_offset = self.matches[prev].start_offset;
        Some(prev)
    }

    pub fn current_match_item(&self) -> Option<&FindMatch> {
        self.current_match.and_then(|idx| self.matches.get(idx))
    }

    pub fn compute_replacement_for_match(&self, text: &str, m: &FindMatch) -> String {
        if !self.is_regex {
            return self.replacement.clone();
        }
        let Ok(re) = self.build_regex() else {
            return self.replacement.clone();
        };
        let slice = text.get(m.start_offset..m.end_offset).unwrap_or("");
        if let Some(caps) = re.captures(slice) {
            let mut out = String::new();
            caps.expand(&self.replacement, &mut out);
            out
        } else {
            self.replacement.clone()
        }
    }

    pub fn compute_layout(&self, editor_bounds: Rect, _font: &FontMetrics) -> FindReplaceLayout {
        if !is_finite_positive_rect(editor_bounds) {
            let z = Rect::zero();
            return FindReplaceLayout {
                widget_rect: z,
                find_input_rect: z,
                case_rect: z,
                whole_word_rect: z,
                regex_rect: z,
                status_rect: z,
                prev_rect: z,
                next_rect: z,
                close_rect: z,
                replace_input_rect: None,
                replace_one_rect: None,
                replace_all_rect: None,
            };
        }

        let pad = 6.0f32
            .min(editor_bounds.width() * 0.05)
            .min(editor_bounds.height() * 0.05);
        let avail_w = (editor_bounds.width() - pad * 2.0).max(0.0);
        let widget_w = 436.0f32.min(avail_w);
        let widget_x = (editor_bounds.max_x() - widget_w - pad).max(editor_bounds.min_x());

        let inner_pad = 6.0f32.min(widget_w * 0.05);
        let row_h = 26.0f32;
        let row_gap = 4.0f32;
        let desired_h = inner_pad * 2.0
            + if self.show_replace {
                row_h * 2.0 + row_gap
            } else {
                row_h
            };
        let widget_h = desired_h.min(editor_bounds.height()).max(0.0);
        let widget_y = (editor_bounds.min_y() + 4.0)
            .min((editor_bounds.max_y() - widget_h).max(editor_bounds.min_y()));
        let widget_rect = clamp_rect(
            Rect::from_xywh(widget_x, widget_y, widget_w, widget_h),
            editor_bounds,
        );

        let content_min_x = widget_rect.min_x() + inner_pad;
        let content_max_x = (widget_rect.max_x() - inner_pad).max(content_min_x);
        let row1_y = widget_rect.min_y() + inner_pad;
        let row1_h = row_h.min((widget_rect.max_y() - row1_y).max(0.0));

        // Allocate row 1 right-to-left: [FindInput] [Aa] [ab] [.*] [status] [↑] [↓] [✕]
        let btn_w = 22.0f32;
        let toggle_w = 24.0f32;
        let status_w = 74.0f32;
        let gap = 3.0f32;

        let mut cursor_right = content_max_x;
        let mut alloc_right = |w: f32| -> Rect {
            let r_max = cursor_right;
            let r_min = (r_max - w).max(content_min_x);
            cursor_right = (r_min - gap).max(content_min_x);
            clamp_rect(
                Rect::from_xywh(r_min, row1_y, (r_max - r_min).max(0.0), row1_h),
                widget_rect,
            )
        };

        let close_rect = alloc_right(btn_w);
        let next_rect = alloc_right(btn_w);
        let prev_rect = alloc_right(btn_w);
        let status_rect = alloc_right(status_w);
        let regex_rect = alloc_right(toggle_w);
        let whole_word_rect = alloc_right(toggle_w);
        let case_rect = alloc_right(toggle_w);
        let find_input_rect = clamp_rect(
            Rect::from_xywh(
                content_min_x,
                row1_y,
                (cursor_right - content_min_x).max(0.0),
                row1_h,
            ),
            widget_rect,
        );

        let (replace_input_rect, replace_one_rect, replace_all_rect) = if self.show_replace {
            let row2_y = (row1_y + row1_h + row_gap).min(widget_rect.max_y());
            let row2_h = row_h.min((widget_rect.max_y() - row2_y).max(0.0));
            let mut r2_right = content_max_x;
            let mut alloc_r2 = |w: f32| -> Rect {
                let r_max = r2_right;
                let r_min = (r_max - w).max(content_min_x);
                r2_right = (r_min - gap).max(content_min_x);
                clamp_rect(
                    Rect::from_xywh(r_min, row2_y, (r_max - r_min).max(0.0), row2_h),
                    widget_rect,
                )
            };
            let rep_all = alloc_r2(84.0);
            let rep_one = alloc_r2(64.0);
            let rep_in = clamp_rect(
                Rect::from_xywh(
                    content_min_x,
                    row2_y,
                    (r2_right - content_min_x).max(0.0),
                    row2_h,
                ),
                widget_rect,
            );
            (Some(rep_in), Some(rep_one), Some(rep_all))
        } else {
            (None, None, None)
        };

        FindReplaceLayout {
            widget_rect,
            find_input_rect,
            case_rect,
            whole_word_rect,
            regex_rect,
            status_rect,
            prev_rect,
            next_rect,
            close_rect,
            replace_input_rect,
            replace_one_rect,
            replace_all_rect,
        }
    }

    pub fn hit_test(
        &self,
        pos: Point,
        editor_bounds: Rect,
        font: &FontMetrics,
    ) -> Option<FindReplaceClickTarget> {
        if !self.is_visible || !pos.x.is_finite() || !pos.y.is_finite() {
            return None;
        }
        let layout = self.compute_layout(editor_bounds, font);
        if !layout.widget_rect.contains(pos) {
            return None;
        }
        if layout.close_rect.contains(pos) {
            return Some(FindReplaceClickTarget::Close);
        }
        if layout.next_rect.contains(pos) {
            return Some(FindReplaceClickTarget::NextMatch);
        }
        if layout.prev_rect.contains(pos) {
            return Some(FindReplaceClickTarget::PrevMatch);
        }
        if layout.regex_rect.contains(pos) {
            return Some(FindReplaceClickTarget::Regex);
        }
        if layout.whole_word_rect.contains(pos) {
            return Some(FindReplaceClickTarget::WholeWord);
        }
        if layout.case_rect.contains(pos) {
            return Some(FindReplaceClickTarget::MatchCase);
        }
        if layout.find_input_rect.contains(pos) {
            return Some(FindReplaceClickTarget::FindInput);
        }
        if layout.replace_all_rect.is_some_and(|r| r.contains(pos)) {
            return Some(FindReplaceClickTarget::ReplaceAll);
        }
        if layout.replace_one_rect.is_some_and(|r| r.contains(pos)) {
            return Some(FindReplaceClickTarget::ReplaceOne);
        }
        if layout.replace_input_rect.is_some_and(|r| r.contains(pos)) {
            return Some(FindReplaceClickTarget::ReplaceInput);
        }
        Some(FindReplaceClickTarget::Widget)
    }

    pub fn render(
        &self,
        editor_bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !self.is_visible || !is_finite_positive_rect(editor_bounds) || !is_finite_font(font) {
            return;
        }

        let layout = self.compute_layout(editor_bounds, font);
        if !is_finite_positive_rect(layout.widget_rect) {
            return;
        }

        scene.push_shadow(Shadow {
            bounds: layout.widget_rect,
            offset: Point::new(0.0, 4.0),
            blur: 10.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.35),
        });

        scene.push_quad(
            Quad::new(layout.widget_rect)
                .with_background(theme.command_palette_bg)
                .with_border(theme.command_palette_border, 1.0)
                .with_corner_radius(4.0),
        );

        let draw_input = |scene: &mut Scene, rect: Rect, text: &str, focused: bool| {
            if !is_finite_positive_rect(rect) {
                return;
            }
            let border = if focused {
                theme.activity_bar_active_border
            } else {
                theme.command_palette_border
            };
            scene.push_quad(
                Quad::new(rect)
                    .with_background(theme.editor_background)
                    .with_border(border, 1.0)
                    .with_corner_radius(2.0),
            );
            if rect.width() > 6.0 && rect.height() > 4.0 {
                let text_x = rect.min_x() + 4.0;
                let text_y = rect.min_y() + ((rect.height() - font.line_height) / 2.0).max(0.0);
                let avail_w = (rect.max_x() - text_x - 2.0).max(0.0);
                let text_h = font.line_height.min((rect.max_y() - text_y).max(0.0));
                if !text.is_empty() && avail_w > 0.0 && text_h > 0.0 {
                    let run_w = font.text_width(text).min(avail_w);
                    scene.push_text(GlyphRun::new(
                        text.to_string(),
                        Point::new(text_x, text_y),
                        Rect::from_xywh(text_x, text_y, run_w, text_h),
                        theme.command_palette_fg,
                        font.font_size,
                    ));
                }
            }
        };

        let draw_button = |scene: &mut Scene, rect: Rect, label: &str, active: bool| {
            if !is_finite_positive_rect(rect) {
                return;
            }
            let bg = if active {
                theme.command_palette_selected_bg
            } else {
                theme.tab_inactive_bg
            };
            scene.push_quad(
                Quad::new(rect)
                    .with_background(bg)
                    .with_border(theme.command_palette_border, 1.0)
                    .with_corner_radius(2.0),
            );
            if rect.width() > 4.0 && rect.height() > 4.0 {
                let label_w = font.text_width(label).min((rect.width() - 4.0).max(0.0));
                let lx = rect.min_x() + ((rect.width() - label_w) / 2.0).max(1.0);
                let ly = rect.min_y() + ((rect.height() - font.line_height) / 2.0).max(0.0);
                let lh = font.line_height.min((rect.max_y() - ly).max(0.0));
                if label_w > 0.0 && lh > 0.0 {
                    scene.push_text(GlyphRun::new(
                        label.to_string(),
                        Point::new(lx, ly),
                        Rect::from_xywh(lx, ly, label_w, lh),
                        theme.command_palette_fg,
                        font.font_size * 0.85,
                    ));
                }
            }
        };

        draw_input(
            scene,
            layout.find_input_rect,
            &self.query,
            self.focus == FindReplaceFocus::FindInput,
        );
        draw_button(scene, layout.case_rect, "Aa", self.case_sensitive);
        draw_button(scene, layout.whole_word_rect, "ab", self.whole_word);
        draw_button(scene, layout.regex_rect, ".*", self.is_regex);

        if is_finite_positive_rect(layout.status_rect) && layout.status_rect.width() > 4.0 {
            let status = self.status_text();
            let status_color = if self.error.is_some() {
                theme.editor_error_foreground
            } else {
                theme.command_palette_fg
            };
            let sx = layout.status_rect.min_x() + 2.0;
            let sy = layout.status_rect.min_y()
                + ((layout.status_rect.height() - font.line_height) / 2.0).max(0.0);
            let sw = font
                .text_width(&status)
                .min((layout.status_rect.max_x() - sx).max(0.0));
            let sh = font
                .line_height
                .min((layout.status_rect.max_y() - sy).max(0.0));
            if sw > 0.0 && sh > 0.0 {
                scene.push_text(GlyphRun::new(
                    status,
                    Point::new(sx, sy),
                    Rect::from_xywh(sx, sy, sw, sh),
                    status_color,
                    font.font_size * 0.85,
                ));
            }
        }

        draw_button(scene, layout.prev_rect, "↑", false);
        draw_button(scene, layout.next_rect, "↓", false);
        draw_button(scene, layout.close_rect, "✕", false);

        if self.show_replace {
            if let Some(rep_in) = layout.replace_input_rect {
                draw_input(
                    scene,
                    rep_in,
                    &self.replacement,
                    self.focus == FindReplaceFocus::ReplaceInput,
                );
            }
            // Render "Replace All" before "Replace" so reverse scene-text search for
            // "Replace" hits the single-Replace button rather than "Replace All".
            if let Some(rep_all) = layout.replace_all_rect {
                draw_button(scene, rep_all, "Replace All", false);
            }
            if let Some(rep_one) = layout.replace_one_rect {
                draw_button(scene, rep_one, "Replace", false);
            }
        }
    }

    pub fn accessibility_node(
        &self,
        editor_id: &str,
        editor_bounds: Rect,
        font: &FontMetrics,
    ) -> AccessibleNode {
        let layout = self.compute_layout(editor_bounds, font);
        let mut dialog = AccessibleNode::new(
            format!("{}-find", editor_id),
            AccessibleRole::Dialog,
            "Find and Replace",
            layout.widget_rect,
        );

        dialog.add_child(
            AccessibleNode::new(
                format!("{}-find-input", editor_id),
                AccessibleRole::TextInput,
                "Find",
                layout.find_input_rect,
            )
            .with_value(self.query.clone())
            .with_focused(self.focus == FindReplaceFocus::FindInput)
            .with_action("input"),
        );

        dialog.add_child(
            AccessibleNode::new(
                format!("{}-find-case", editor_id),
                AccessibleRole::Button,
                "Match Case",
                layout.case_rect,
            )
            .with_selected(self.case_sensitive)
            .with_action("press"),
        );
        dialog.add_child(
            AccessibleNode::new(
                format!("{}-find-word", editor_id),
                AccessibleRole::Button,
                "Whole Word",
                layout.whole_word_rect,
            )
            .with_selected(self.whole_word)
            .with_action("press"),
        );
        dialog.add_child(
            AccessibleNode::new(
                format!("{}-find-regex", editor_id),
                AccessibleRole::Button,
                "Use Regular Expression",
                layout.regex_rect,
            )
            .with_selected(self.is_regex)
            .with_action("press"),
        );
        dialog.add_child(AccessibleNode::new(
            format!("{}-find-status", editor_id),
            AccessibleRole::StaticText,
            self.status_text(),
            layout.status_rect,
        ));
        dialog.add_child(
            AccessibleNode::new(
                format!("{}-find-prev", editor_id),
                AccessibleRole::Button,
                "Previous Match",
                layout.prev_rect,
            )
            .with_action("press"),
        );
        dialog.add_child(
            AccessibleNode::new(
                format!("{}-find-next", editor_id),
                AccessibleRole::Button,
                "Next Match",
                layout.next_rect,
            )
            .with_action("press"),
        );
        dialog.add_child(
            AccessibleNode::new(
                format!("{}-find-close", editor_id),
                AccessibleRole::Button,
                "Close",
                layout.close_rect,
            )
            .with_action("press"),
        );

        if self.show_replace {
            if let Some(rep_in) = layout.replace_input_rect {
                dialog.add_child(
                    AccessibleNode::new(
                        format!("{}-replace-input", editor_id),
                        AccessibleRole::TextInput,
                        "Replace Input",
                        rep_in,
                    )
                    .with_value(self.replacement.clone())
                    .with_focused(self.focus == FindReplaceFocus::ReplaceInput)
                    .with_action("input"),
                );
            }
            if let Some(rep_one) = layout.replace_one_rect {
                dialog.add_child(
                    AccessibleNode::new(
                        format!("{}-replace-one", editor_id),
                        AccessibleRole::Button,
                        "Replace",
                        rep_one,
                    )
                    .with_action("press"),
                );
            }
            if let Some(rep_all) = layout.replace_all_rect {
                dialog.add_child(
                    AccessibleNode::new(
                        format!("{}-replace-all", editor_id),
                        AccessibleRole::Button,
                        "Replace All",
                        rep_all,
                    )
                    .with_action("press"),
                );
            }
        }

        dialog
    }
}

/// Advances past a zero-width match at `offset` in `text` without splitting `\r\n`
/// or multi-byte UTF-8 characters. Returns `text.len() + 1` when `offset >= text.len()`
/// so `search_origin_offset` wraps around to match `0`.
pub(crate) fn advance_past_zero_width(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len().saturating_add(1);
    }
    let mut safe = offset;
    while safe > 0 && !text.is_char_boundary(safe) {
        safe -= 1;
    }
    if text[safe..].starts_with("\r\n") {
        safe + 2
    } else {
        let step = text[safe..].chars().next().map_or(1, |c| c.len_utf8());
        safe + step
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn passes_whole_word(text: &str, start: usize, end: usize) -> bool {
    let Some(matched) = text.get(start..end) else {
        return false;
    };
    if matched.is_empty() {
        return false;
    }
    let starts_word = matched.chars().next().is_some_and(is_word_char);
    let ends_word = matched.chars().next_back().is_some_and(is_word_char);
    if starts_word && text[..start].chars().next_back().is_some_and(is_word_char) {
        return false;
    }
    if ends_word && text[end..].chars().next().is_some_and(is_word_char) {
        return false;
    }
    true
}

fn clamp_rect(r: Rect, container: Rect) -> Rect {
    let x = r.min_x().clamp(container.min_x(), container.max_x());
    let y = r.min_y().clamp(container.min_y(), container.max_y());
    let max_x = r.max_x().clamp(x, container.max_x());
    let max_y = r.max_y().clamp(y, container.max_y());
    Rect::from_xywh(x, y, (max_x - x).max(0.0), (max_y - y).max(0.0))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_replace_case_whole_word_and_regex() {
        let buf = PieceTable::from_str("Foo foo foobar f123 f99\n");
        let mut widget = FindReplaceWidget::new();
        widget.open_find(Some("foo".to_string()), 0);
        widget.compute_matches(&buf);
        assert_eq!(widget.matches.len(), 3);
        assert_eq!(widget.status_text(), "1 of 3");

        widget.case_sensitive = true;
        widget.compute_matches(&buf);
        assert_eq!(widget.matches.len(), 2);
        assert_eq!(widget.status_text(), "1 of 2");

        widget.whole_word = true;
        widget.compute_matches(&buf);
        assert_eq!(widget.matches.len(), 1);
        assert_eq!(widget.status_text(), "1 of 1");

        widget.case_sensitive = false;
        widget.whole_word = false;
        widget.is_regex = true;
        widget.query = "f[0-9]+".to_string();
        widget.compute_matches(&buf);
        assert_eq!(widget.matches.len(), 2);
    }

    #[test]
    fn test_find_replace_zero_width_regex_and_invalid_regex() {
        let buf = PieceTable::from_str("🚀 café\r\nabc\r\n");
        let mut widget = FindReplaceWidget::new();
        widget.open_find(None, 0);
        widget.is_regex = true;

        for pat in ["^", "$", ".*", "a*"] {
            widget.query = pat.to_string();
            widget.compute_matches(&buf);
            assert!(widget.error.is_none(), "pat={pat}");
            assert!(!widget.matches.is_empty(), "pat={pat}");
            // Zero-width advances must never land between \r and \n or split UTF-8
            let raw = buf.to_string();
            for m in &widget.matches {
                assert!(raw.is_char_boundary(m.start_offset));
                assert!(raw.is_char_boundary(m.end_offset));
                if m.start_offset > 0 && m.start_offset < raw.len() {
                    assert!(
                        !(raw.as_bytes()[m.start_offset - 1] == b'\r'
                            && raw.as_bytes()[m.start_offset] == b'\n'),
                        "match split CRLF for pat={pat} at {}",
                        m.start_offset
                    );
                }
            }
        }

        for bad in ["[unclosed(", "(?=a)", "(?invalid"] {
            widget.query = bad.to_string();
            widget.compute_matches(&buf);
            assert!(widget.error.is_some(), "expected error for {bad}");
            assert!(widget.matches.is_empty(), "expected no matches for {bad}");
            assert_eq!(widget.status_text(), "Invalid regex");
        }
    }

    #[test]
    fn test_find_replace_utf16_and_crlf_positions() {
        let buf = PieceTable::from_str("🚀 café\r\nhello 🚀 world\r\n");
        let mut widget = FindReplaceWidget::new();
        widget.open_find(Some("café".to_string()), 0);
        widget.compute_matches(&buf);
        assert_eq!(widget.matches.len(), 1);
        let m = &widget.matches[0];
        assert_eq!(m.range.start, Position::new(0, 3));
        assert_eq!(m.range.end, Position::new(0, 7));
    }

    #[test]
    fn test_find_replace_tiny_viewport_finite_non_negative_rects() {
        let mut widget = FindReplaceWidget::new();
        widget.open_replace(Some("x".to_string()), 0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let tiny_bounds = Rect::from_xywh(0.0, 0.0, 100.0, 90.0);
        let mut scene = Scene::new();
        widget.render(tiny_bounds, &theme, &font, &mut scene);

        for quad in scene.quads() {
            assert!(
                quad.bounds.origin.x.is_finite()
                    && quad.bounds.origin.y.is_finite()
                    && quad.bounds.width().is_finite()
                    && quad.bounds.height().is_finite()
                    && quad.bounds.width() >= 0.0
                    && quad.bounds.height() >= 0.0
            );
        }
        for run in scene.glyph_runs() {
            assert!(
                run.origin.x.is_finite()
                    && run.origin.y.is_finite()
                    && run.bounds.width().is_finite()
                    && run.bounds.height().is_finite()
                    && run.bounds.width() >= 0.0
                    && run.bounds.height() >= 0.0
            );
        }
    }
}
