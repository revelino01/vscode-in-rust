//! Autocomplete UI widget, snippet expansion, fuzzy matching, and popover rendering.

use crate::cursor::{CursorState, Position};
use crate::model::piece_table::PieceTable;
use crate::model::undo_redo::{TextEdit, UndoRedoStack};
use parking_lot::Mutex;
use rustcode_platform::render::{
    Edges, FontMetrics, GlyphRun, Point, Quad, Rect, Rgba, Scene, WorkbenchThemeStyles,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompletionItemKind {
    Text,
    Method,
    Function,
    Constructor,
    Field,
    Variable,
    Class,
    Interface,
    Module,
    Property,
    Keyword,
    Snippet,
    Struct,
    Enum,
    Constant,
    File,
    Reference,
}

impl CompletionItemKind {
    pub fn badge_symbol(&self) -> &'static str {
        match self {
            Self::Method => "m",
            Self::Function => "f",
            Self::Constructor => "c",
            Self::Field => "p",
            Self::Variable => "v",
            Self::Class => "C",
            Self::Interface => "I",
            Self::Module => "M",
            Self::Property => "p",
            Self::Keyword => "k",
            Self::Snippet => "s",
            Self::Struct => "S",
            Self::Enum => "E",
            Self::Constant => "K",
            Self::File => "F",
            Self::Reference => "r",
            Self::Text => "t",
        }
    }

    pub fn badge_color(&self) -> Rgba {
        match self {
            Self::Method | Self::Function => Rgba::new(0.86, 0.58, 0.35, 1.0), // Function yellow/orange
            Self::Keyword => Rgba::new(0.77, 0.53, 0.75, 1.0),                  // Keyword purple
            Self::Variable | Self::Field => Rgba::new(0.40, 0.75, 0.85, 1.0),  // Variable light blue
            Self::Struct | Self::Class => Rgba::new(0.30, 0.78, 0.69, 1.0),    // Type teal/green
            Self::Snippet => Rgba::new(0.90, 0.45, 0.45, 1.0),                  // Snippet reddish
            _ => Rgba::new(0.60, 0.60, 0.60, 1.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    pub kind: Option<CompletionItemKind>,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub insert_text: Option<String>,
    pub filter_text: Option<String>,
    pub sort_text: Option<String>,
}

impl CompletionItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            kind: None,
            detail: None,
            documentation: None,
            insert_text: None,
            filter_text: None,
            sort_text: None,
        }
    }

    pub fn with_kind(mut self, kind: CompletionItemKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_doc(mut self, doc: impl Into<String>) -> Self {
        self.documentation = Some(doc.into());
        self
    }

    pub fn with_insert_text(mut self, insert_text: impl Into<String>) -> Self {
        self.insert_text = Some(insert_text.into());
        self
    }

    pub fn effective_insert_text(&self) -> &str {
        self.insert_text.as_deref().unwrap_or(&self.label)
    }

    pub fn effective_filter_text(&self) -> &str {
        self.filter_text.as_deref().unwrap_or(&self.label)
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompletionList {
    pub is_incomplete: bool,
    pub items: Vec<CompletionItem>,
}

/// Parses snippet tabstops like `$0`, `$1`, `${1:default}` into expanded plain text
/// and the offset where the cursor should land.
pub fn expand_snippet(template: &str) -> (String, usize) {
    let mut expanded = String::new();
    let mut cursor_offset = None;
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' {
            if let Some(&next) = chars.peek() {
                if next == '$' {
                    chars.next();
                    expanded.push('$');
                    continue;
                } else if next.is_ascii_digit() {
                    let digit = chars.next().unwrap();
                    // Place cursor at tabstop
                    if cursor_offset.is_none() || digit == '0' {
                        cursor_offset = Some(expanded.len());
                    }
                    continue;
                } else if next == '{' {
                    chars.next(); // consume '{'
                    let mut placeholder = String::new();
                    for inner in chars.by_ref() {
                        if inner == '}' {
                            break;
                        }
                        placeholder.push(inner);
                    }
                    // Handle `${1:default_text}`, `${1|choice1,choice2|}`, or `${1}`
                    let text = if let Some((_, def)) = placeholder.split_once(':') {
                        def
                    } else if let Some((_, choices)) = placeholder.split_once('|') {
                        let choices_trimmed = choices.strip_suffix('|').unwrap_or(choices);
                        choices_trimmed.split(',').next().unwrap_or("")
                    } else if placeholder.chars().all(|ch| ch.is_ascii_digit()) {
                        ""
                    } else {
                        &placeholder
                    };
                    if cursor_offset.is_none() || placeholder.starts_with('0') {
                        cursor_offset = Some(expanded.len());
                    }
                    expanded.push_str(text);
                    continue;
                }
            }
        }
        expanded.push(c);
    }

    let final_cursor = cursor_offset.unwrap_or(expanded.len());
    (expanded, final_cursor)
}

/// Computes a fuzzy match score for `candidate` against `query`.
/// Returns `Some(score)` if matches, or `None` if it does not match.
pub fn fuzzy_match(candidate: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(100);
    }

    // 1. Exact prefix match
    if candidate.starts_with(query) {
        return Some(1000 + (100 - (candidate.len() as i32 - query.len() as i32)).max(0));
    }

    // 2. Case-insensitive prefix match (UTF-8 safe)
    let query_chars_count = query.chars().count();
    let cand_prefix: String = candidate.chars().take(query_chars_count).collect();
    if cand_prefix.eq_ignore_ascii_case(query) {
        return Some(800 + (100 - (candidate.len() as i32 - query.len() as i32)).max(0));
    }

    // 3. Subsequence fuzzy match (UTF-8 safe)
    let mut score = 500;
    let mut cand_chars = candidate.chars().enumerate();
    let mut prev_char: Option<char> = None;
    let mut prev_idx: Option<usize> = None;

    for q in query.chars() {
        let q_lower = q.to_ascii_lowercase();
        let mut matched = false;
        for (idx, c) in cand_chars.by_ref() {
            let is_boundary = idx == 0
                || prev_char == Some('_')
                || prev_char == Some(' ')
                || (prev_char.is_some_and(|p| p.is_lowercase()) && c.is_uppercase());
            prev_char = Some(c);

            if c.to_ascii_lowercase() == q_lower {
                // Bonus for exact case match
                if c == q {
                    score += 10;
                }
                // Bonus for consecutive matches
                if let Some(prev) = prev_idx {
                    if idx == prev + 1 {
                        score += 30;
                    }
                }
                // Bonus for word boundary
                if is_boundary {
                    score += 25;
                }
                prev_idx = Some(idx);
                matched = true;
                break;
            }
        }
        if !matched {
            return None;
        }
    }

    // Penalty for long candidate length compared to query
    score -= (candidate.len() as i32 - query.len() as i32).clamp(0, 100);
    Some(score)
}

/// The Autocomplete Popover Widget.
pub struct AutocompleteWidget {
    pub is_visible: bool,
    pub all_items: Vec<CompletionItem>,
    pub filtered_items: Vec<(CompletionItem, i32)>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub max_visible: usize,
    pub trigger_position: Position,
    pub trigger_char: Option<char>,
    pub filter_query: String,
    pub show_documentation: bool,
    pub last_bounds: Mutex<Option<Rect>>,
    pub last_doc_bounds: Mutex<Option<Rect>>,
}

impl Default for AutocompleteWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl AutocompleteWidget {
    pub fn new() -> Self {
        Self {
            is_visible: false,
            all_items: Vec::new(),
            filtered_items: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            max_visible: 8,
            trigger_position: Position::new(0, 0),
            trigger_char: None,
            filter_query: String::new(),
            show_documentation: true,
            last_bounds: Mutex::new(None),
            last_doc_bounds: Mutex::new(None),
        }
    }

    pub fn show(
        &mut self,
        items: Vec<CompletionItem>,
        pos: Position,
        trigger_char: Option<char>,
    ) {
        self.all_items = items;
        self.trigger_position = pos;
        self.trigger_char = trigger_char;
        self.filter_query.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.update_filter("");
        self.is_visible = !self.filtered_items.is_empty();
    }

    pub fn hide(&mut self) {
        self.is_visible = false;
        self.all_items.clear();
        self.filtered_items.clear();
        self.filter_query.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
        *self.last_bounds.lock() = None;
        *self.last_doc_bounds.lock() = None;
    }

    pub fn update_filter(&mut self, query: &str) {
        self.filter_query = query.to_string();
        let mut matches = Vec::new();

        for item in &self.all_items {
            let filter_text = item.effective_filter_text();
            if let Some(score) = fuzzy_match(filter_text, query) {
                matches.push((item.clone(), score));
            }
        }

        // Sort descending by score, then alphabetically by sort_text/label
        matches.sort_by(|a, b| {
            b.1.cmp(&a.1).then_with(|| {
                let sort_a = a.0.sort_text.as_deref().unwrap_or(&a.0.label);
                let sort_b = b.0.sort_text.as_deref().unwrap_or(&b.0.label);
                sort_a.cmp(sort_b)
            })
        });

        self.filtered_items = matches;
        self.is_visible = !self.filtered_items.is_empty();
        if self.filtered_items.is_empty() {
            self.selected_index = 0;
            self.scroll_offset = 0;
        } else {
            self.selected_index = self.selected_index.min(self.filtered_items.len() - 1);
            self.adjust_scroll();
        }
    }

    pub fn select_next(&mut self) {
        if self.filtered_items.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.filtered_items.len();
        self.adjust_scroll();
    }

    pub fn select_prev(&mut self) {
        if self.filtered_items.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = self.filtered_items.len() - 1;
        } else {
            self.selected_index -= 1;
        }
        self.adjust_scroll();
    }

    fn adjust_scroll(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + self.max_visible {
            self.scroll_offset = self.selected_index - self.max_visible + 1;
        }
    }

    pub fn selected_item(&self) -> Option<&CompletionItem> {
        self.filtered_items
            .get(self.selected_index)
            .map(|(item, _)| item)
    }

    /// Commits the currently selected item into the piece table buffer,
    /// updating undo history and moving the cursor.
    pub fn commit(
        &mut self,
        buffer: &mut PieceTable,
        cursor_state: &mut CursorState,
        undo_stack: &mut UndoRedoStack,
    ) -> Option<CompletionItem> {
        let item = self.selected_item()?.clone();
        let current_pos = cursor_state.primary().active;

        // Calculate replacement range: from word start / trigger to current cursor
        let replace_start_pos = if current_pos.line == self.trigger_position.line {
            Position::new(
                current_pos.line,
                self.trigger_position
                    .column
                    .min(current_pos.column),
            )
        } else {
            current_pos
        };

        let start_offset = buffer.position_to_offset(replace_start_pos.line, replace_start_pos.column);
        let end_offset = buffer.position_to_offset(current_pos.line, current_pos.column);
        let del_len = end_offset.saturating_sub(start_offset);

        let (inserted_text, cursor_rel_offset) = expand_snippet(item.effective_insert_text());

        if del_len > 0 {
            let deleted = buffer.delete(start_offset, del_len);
            buffer.insert(start_offset, &inserted_text);
            undo_stack.push_edit(TextEdit::replace(start_offset, &deleted, &inserted_text));
        } else {
            buffer.insert(start_offset, &inserted_text);
            undo_stack.push_edit(TextEdit::insert(start_offset, &inserted_text));
        }

        // Set cursor to the snippet tabstop or end of inserted text
        let final_offset = (start_offset + cursor_rel_offset).min(buffer.len());
        let (new_line, new_col) = buffer.offset_to_position(final_offset);
        cursor_state.set_primary(Position::new(new_line, new_col));

        self.hide();
        Some(item)
    }

    /// Renders the autocomplete popover overlay and documentation preview panel.
    pub fn render(
        &self,
        cursor_anchor: Point,
        editor_bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !self.is_visible
            || self.filtered_items.is_empty()
            || !cursor_anchor.x.is_finite()
            || !cursor_anchor.y.is_finite()
            || !is_finite_positive_rect(editor_bounds)
            || !font.line_height.is_finite()
            || font.line_height <= 0.0
            || !font.char_width.is_finite()
            || font.char_width < 0.0
            || !font.font_size.is_finite()
            || font.font_size < 0.0
        {
            return;
        }

        let item_h = 24.0;
        let list_w = 280.0f32;
        let visible_count = self.filtered_items.len().min(self.max_visible);
        let list_h = visible_count as f32 * item_h + 4.0;
        if !list_h.is_finite() || list_h <= 0.0 {
            return;
        }

        // Determine list position anchored at cursor
        let mut list_x = cursor_anchor.x;
        let mut list_y = cursor_anchor.y + font.line_height + 2.0;

        // Flip above if overflowing bottom (or if cursor_anchor.y + line_height overflows)
        if !list_y.is_finite()
            || !(list_y + list_h).is_finite()
            || list_y + list_h > editor_bounds.max_y() - 10.0
        {
            list_y = (cursor_anchor.y - list_h - 2.0).max(editor_bounds.min_y());
        }

        // Clamp horizontally
        if !(list_x + list_w).is_finite() || list_x + list_w > editor_bounds.max_x() - 10.0 {
            list_x = (editor_bounds.max_x() - list_w - 10.0).max(editor_bounds.min_x());
        }

        let list_rect = Rect::from_xywh(list_x, list_y, list_w, list_h);
        if !is_finite_positive_rect(list_rect) {
            return;
        }
        *self.last_bounds.lock() = Some(list_rect);

        // 1. Popover background & border
        let bg_color = theme.command_palette_bg;
        let border_color = theme.command_palette_border;
        push_finite_quad(
            scene,
            Quad::new(list_rect)
                .with_background(bg_color)
                .with_border_edges(border_color, Edges::all(1.0)),
        );

        // 2. Render visible items
        let start_idx = self.scroll_offset;
        let end_idx = (start_idx + self.max_visible).min(self.filtered_items.len());

        for (i, idx) in (start_idx..end_idx).enumerate() {
            let (item, _) = &self.filtered_items[idx];
            let is_selected = idx == self.selected_index;
            let item_y = list_y + 2.0 + i as f32 * item_h;
            let item_rect = Rect::from_xywh(list_x + 2.0, item_y, list_w - 4.0, item_h);
            let badge_y = item_y + (item_h - font.line_height) / 2.0;

            if !item_y.is_finite()
                || !is_finite_rect(item_rect)
                || !badge_y.is_finite()
                || !(badge_y + font.line_height).is_finite()
            {
                break;
            }

            // Item highlight
            if is_selected {
                push_finite_quad(
                    scene,
                    Quad::new(item_rect)
                        .with_background(theme.command_palette_selected_bg)
                        .with_border_edges(theme.activity_bar_active_border, Edges::new(0.0, 0.0, 0.0, 2.0)),
                );
            }

            // Kind badge icon (e.g. [m], [f], [k], [s])
            let kind = item.kind.unwrap_or(CompletionItemKind::Text);
            let badge_sym = kind.badge_symbol();
            let badge_color = kind.badge_color();
            let badge_x = list_x + 8.0;

            push_finite_text(
                scene,
                GlyphRun::new(
                    badge_sym.to_string(),
                    Point::new(badge_x, badge_y),
                    Rect::from_xywh(badge_x, badge_y, 14.0, font.line_height),
                    badge_color,
                    font.font_size * 0.9,
                )
                .with_style(true, false, false),
            );

            // Item label
            let label_x = badge_x + 18.0;
            let label_fg = if is_selected {
                theme.command_palette_fg
            } else {
                theme.editor_foreground
            };

            let label_text = item.label.clone();
            push_finite_text(
                scene,
                GlyphRun::new(
                    label_text,
                    Point::new(label_x, badge_y),
                    Rect::from_xywh(label_x, badge_y, 140.0, font.line_height),
                    label_fg,
                    font.font_size,
                ),
            );

            // Item detail (type/signature preview)
            if let Some(detail) = &item.detail {
                let detail_fg = Rgba::new(0.60, 0.60, 0.60, 0.85);
                let raw_detail_w = font.text_width(detail);
                let detail_w = if raw_detail_w.is_finite() {
                    raw_detail_w.clamp(0.0, 90.0)
                } else {
                    90.0
                };
                let detail_x = list_x + list_w - detail_w - 8.0;
                push_finite_text(
                    scene,
                    GlyphRun::new(
                        detail.clone(),
                        Point::new(detail_x, badge_y),
                        Rect::from_xywh(detail_x, badge_y, detail_w, font.line_height),
                        detail_fg,
                        font.font_size * 0.85,
                    )
                    .with_style(false, true, false),
                );
            }
        }

        // 3. Detail & Documentation Preview Panel
        if self.show_documentation {
            if let Some(selected) = self.selected_item() {
                if selected.detail.is_some() || selected.documentation.is_some() {
                    let doc_w = 280.0f32;
                    let doc_h = list_h.max(120.0);

                    // Position to right of list by default, or left if no space
                    let mut doc_x = list_x + list_w + 4.0;
                    if !(doc_x + doc_w).is_finite() || doc_x + doc_w > editor_bounds.max_x() - 10.0 {
                        if list_x - doc_w - 4.0 >= editor_bounds.min_x() {
                            doc_x = list_x - doc_w - 4.0;
                        } else {
                            doc_x = (editor_bounds.max_x() - doc_w - 10.0).max(editor_bounds.min_x());
                        }
                    }

                    // Clamp vertically so documentation does not overflow bottom
                    let mut doc_y = list_y;
                    if !(doc_y + doc_h).is_finite() || doc_y + doc_h > editor_bounds.max_y() - 10.0 {
                        doc_y = (editor_bounds.max_y() - doc_h - 10.0).max(editor_bounds.min_y());
                    }

                    let doc_rect = Rect::from_xywh(doc_x, doc_y, doc_w, doc_h);
                    if !is_finite_positive_rect(doc_rect) {
                        return;
                    }
                    *self.last_doc_bounds.lock() = Some(doc_rect);

                    // Background & border
                    push_finite_quad(
                        scene,
                        Quad::new(doc_rect)
                            .with_background(theme.sidebar_bg)
                            .with_border_edges(border_color, Edges::all(1.0)),
                    );

                    let mut cur_y = doc_y + 8.0;
                    if !cur_y.is_finite() {
                        return;
                    }

                    // Header: Detail (type signature)
                    if let Some(detail) = &selected.detail {
                        if cur_y.is_finite() && (cur_y + font.line_height).is_finite() {
                            push_finite_text(
                                scene,
                                GlyphRun::new(
                                    detail.clone(),
                                    Point::new(doc_x + 8.0, cur_y),
                                    Rect::from_xywh(doc_x + 8.0, cur_y, doc_w - 16.0, font.line_height),
                                    theme.editor_foreground,
                                    font.font_size * 0.9,
                                )
                                .with_style(true, false, false),
                            );
                            cur_y += font.line_height + 4.0;

                            // Separator line
                            if cur_y.is_finite() && (cur_y + 1.0).is_finite() {
                                push_finite_quad(
                                    scene,
                                    Quad::new(Rect::from_xywh(doc_x + 8.0, cur_y, doc_w - 16.0, 1.0))
                                        .with_background(border_color),
                                );
                                cur_y += 6.0;
                            }
                        }
                    }

                    // Body: Documentation text
                    if let Some(doc) = &selected.documentation {
                        let doc_fg = Rgba::new(0.80, 0.80, 0.80, 1.0);
                        for line in doc.lines() {
                            if !cur_y.is_finite()
                                || !(cur_y + font.line_height).is_finite()
                                || cur_y + font.line_height > doc_y + doc_h - 4.0
                            {
                                break;
                            }
                            push_finite_text(
                                scene,
                                GlyphRun::new(
                                    line.to_string(),
                                    Point::new(doc_x + 8.0, cur_y),
                                    Rect::from_xywh(doc_x + 8.0, cur_y, doc_w - 16.0, font.line_height),
                                    doc_fg,
                                    font.font_size * 0.85,
                                ),
                            );
                            cur_y += font.line_height;
                        }
                    }
                }
            }
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

    #[test]
    fn test_snippet_expansion() {
        let (text1, offset1) = expand_snippet("println!(\"{}\", $0);");
        assert_eq!(text1, "println!(\"{}\", );");
        assert_eq!(offset1, 15);

        let (text2, offset2) = expand_snippet("fn ${1:name}() {\n    $0\n}");
        assert_eq!(text2, "fn name() {\n    \n}");
        assert_eq!(offset2, 16);

        let (text2b, offset2b) = expand_snippet("fn ${1:name}();");
        assert_eq!(text2b, "fn name();");
        assert_eq!(offset2b, 3);

        let (text3, offset3) = expand_snippet("simple_ident");
        assert_eq!(text3, "simple_ident");
        assert_eq!(offset3, 12);

        let (text4, offset4) = expand_snippet("assert_eq!($1, $2);");
        assert_eq!(text4, "assert_eq!(, );");
        assert_eq!(offset4, 11);

        let (text5, offset5) = expand_snippet("let x = ${1};");
        assert_eq!(text5, "let x = ;");
        assert_eq!(offset5, 8);

        let (text6, _) = expand_snippet("choose(${1|alpha,beta|});");
        assert_eq!(text6, "choose(alpha);");
    }

    #[test]
    fn test_fuzzy_matching_and_ranking() {
        assert!(fuzzy_match("println!", "pri").unwrap() > 800);
        assert!(fuzzy_match("to_string", "ts").is_some());
        assert!(fuzzy_match("clone", "cl").is_some());
        assert!(fuzzy_match("clone", "xyz").is_none());

        let score_prefix = fuzzy_match("clone", "cl").unwrap();
        let score_subseq = fuzzy_match("collect", "cl").unwrap();
        assert!(score_prefix >= score_subseq);

        // UTF-8 multi-byte characters must not panic
        assert!(fuzzy_match("日本語_test", "a").is_none());
        assert!(fuzzy_match("✓_check", "✓").is_some());
        assert!(fuzzy_match("✓_check", "c").is_some());
    }

    #[test]
    fn test_autocomplete_widget_filter_and_commit() {
        let mut widget = AutocompleteWidget::new();
        let items = vec![
            CompletionItem::new("clone")
                .with_kind(CompletionItemKind::Method)
                .with_detail("fn(&self) -> Self")
                .with_doc("Returns a copy of the value.")
                .with_insert_text("clone()"),
            CompletionItem::new("collect")
                .with_kind(CompletionItemKind::Method)
                .with_insert_text("collect()"),
            CompletionItem::new("println!")
                .with_kind(CompletionItemKind::Snippet)
                .with_insert_text("println!(\"{}\", $0);"),
        ];

        widget.show(items, Position::new(0, 12), Some('.'));
        assert!(widget.is_visible);
        assert_eq!(widget.filtered_items.len(), 3);

        // Filter by "cl"
        widget.update_filter("cl");
        assert_eq!(widget.filtered_items.len(), 2);
        assert_eq!(widget.selected_item().unwrap().label, "clone");

        // Navigate down
        widget.select_next();
        assert_eq!(widget.selected_item().unwrap().label, "collect");

        // Navigate up
        widget.select_prev();
        assert_eq!(widget.selected_item().unwrap().label, "clone");

        // Commit into piece table
        let mut buffer = PieceTable::from_str("let x = foo.cl;");
        let mut cursor_state = CursorState::new();
        cursor_state.set_primary(Position::new(0, 14)); // cursor after "cl"
        let mut undo_stack = UndoRedoStack::new();

        let committed = widget.commit(&mut buffer, &mut cursor_state, &mut undo_stack);
        assert!(committed.is_some());
        assert_eq!(committed.unwrap().label, "clone");
        assert_eq!(buffer.to_string(), "let x = foo.clone();");
        assert!(!widget.is_visible);
    }

    #[test]
    fn test_autocomplete_widget_filter_empty_dismisses() {
        let mut widget = AutocompleteWidget::new();
        let items = vec![CompletionItem::new("clone")];
        widget.show(items, Position::new(0, 0), None);
        assert!(widget.is_visible);

        widget.update_filter("zzz");
        assert!(!widget.is_visible);
        assert!(widget.filtered_items.is_empty());
    }

    #[test]
    fn test_autocomplete_render_extreme_bounds_and_invalid_font_no_non_finite() {
        use rustcode_platform::render::SceneNode;

        let mut widget = AutocompleteWidget::new();
        widget.show_documentation = true;
        widget.show(
            vec![
                CompletionItem::new("println!")
                    .with_kind(CompletionItemKind::Snippet)
                    .with_detail("macro_rules! println")
                    .with_doc("Line 1 documentation.\nLine 2 documentation."),
                CompletionItem::new("clone")
                    .with_kind(CompletionItemKind::Method)
                    .with_detail("fn(&self) -> Self"),
            ],
            Position::new(0, 0),
            None,
        );

        let theme = WorkbenchThemeStyles::dark_default();
        let base_font = FontMetrics::default_monospace();

        // Invalid font metrics (negative/NaN/Inf char_width or font_size) must emit nothing
        for bad_font in [
            FontMetrics {
                char_width: f32::NAN,
                ..base_font.clone()
            },
            FontMetrics {
                char_width: -1.0,
                ..base_font.clone()
            },
            FontMetrics {
                char_width: f32::INFINITY,
                ..base_font.clone()
            },
            FontMetrics {
                font_size: -2.0,
                ..base_font.clone()
            },
            FontMetrics {
                line_height: 0.0,
                ..base_font.clone()
            },
        ] {
            let mut scene = Scene::new();
            widget.render(
                Point::new(100.0, 100.0),
                Rect::from_xywh(0.0, 0.0, 800.0, 600.0),
                &theme,
                &bad_font,
                &mut scene,
            );
            assert!(scene.is_empty());
        }

        // Extreme bounds & extreme line_height combinations must emit zero non-finite nodes
        for (anchor, bounds, lh) in [
            (
                Point::new(100.0, 2.5e38),
                Rect::from_xywh(0.0, 2.5e38, 800.0, 0.9e38),
                1e38_f32,
            ),
            (
                Point::new(100.0, 0.0),
                Rect::from_xywh(0.0, 0.0, f32::MAX, f32::MAX),
                2.5e38_f32,
            ),
            (
                Point::new(1e38, 1e38),
                Rect::from_xywh(1e38, 1e38, 1e38, 1e38),
                2.5e38_f32,
            ),
        ] {
            let extreme_font = FontMetrics {
                line_height: lh,
                ..base_font.clone()
            };
            let mut scene = Scene::new();
            widget.render(anchor, bounds, &theme, &extreme_font, &mut scene);
            for node in scene.nodes() {
                match node {
                    SceneNode::PushClip(r) => assert!(is_finite_rect(*r)),
                    SceneNode::PopClip => {}
                    SceneNode::Quad(q) => assert!(is_finite_rect(q.bounds)),
                    SceneNode::Shadow(s) => assert!(is_finite_rect(s.bounds)),
                    SceneNode::GlyphRun(g) => {
                        assert!(g.origin.x.is_finite() && g.origin.y.is_finite());
                        assert!(is_finite_rect(g.bounds));
                    }
                }
            }
        }
    }
}
