//! GPUI EditorView component rendering text lines, line numbers, cursor, selections, and routing input.

pub mod minimap;

pub use minimap::MinimapState;

use crate::completion::{AutocompleteWidget, CompletionItem, CompletionItemKind};
use crate::cursor::{CursorState, Position, RangeEdit, Selection};
use crate::document::TextDocument;
use crate::folding::FoldingModel;
use crate::model::piece_table::{utf16_col_to_byte_offset, PieceTable};
use crate::model::undo_redo::{TextEdit, UndoRedoStack};
use crate::syntax::{GrammarRegistry, SyntaxHighlighter};
use crate::viewmodel::{DiagnosticSeverity, EditorDiagnostic};
use parking_lot::{Mutex, RwLock};
use rustcode_platform::accessibility::{AccessibleNode, AccessibleRole, AccessibleState};
use rustcode_platform::fs::{FileMetadata, FileSystemError, FileSystemService};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Rgba,
    Scene, Shadow, View, WorkbenchThemeStyles,
};
use rustcode_platform::theme::ColorTheme;
use std::collections::BTreeSet;
use std::sync::Arc;

pub type CompletionProviderFn =
    Arc<dyn Fn(&str, Position, Option<char>) -> Vec<CompletionItem> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum WordWrapMode {
    #[default]
    Off,
    On,
    WordWrapColumn,
    Bounded,
}

impl WordWrapMode {
    pub fn from_config_str(s: &str) -> Self {
        match s.trim() {
            "on" => Self::On,
            "wordWrapColumn" | "wordwrapcolumn" => Self::WordWrapColumn,
            "bounded" => Self::Bounded,
            _ => Self::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorGutterAction {
    BreakpointToggled { line: usize, added: bool },
    /// Emitted when a gutter fold chevron is clicked. `line` is the 0-based buffer line index.
    FoldToggled { line: usize, collapsed: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WrapSegment {
    pub buffer_line: usize,
    pub visual_row: usize,
    pub is_first_segment: bool,
    pub is_last_segment: bool,
    pub byte_start: usize,
    pub byte_end: usize,
    pub char_start: usize,
    pub char_end: usize,
    pub utf16_start: usize,
    pub utf16_end: usize,
}

pub(crate) struct VisualLayout {
    identity: bool,
    line_count: usize,
    segments: Vec<WrapSegment>,
}

impl VisualLayout {
    pub fn build(buf: &PieceTable, folding: &FoldingModel, max_cols: Option<usize>) -> Self {
        let line_count = buf.line_count();
        if max_cols.is_none() && !folding.has_collapsed() {
            return Self {
                identity: true,
                line_count,
                segments: Vec::new(),
            };
        }

        let mut segments = Vec::with_capacity(line_count);
        let mut visual_row = 0usize;
        for line_idx in 0..line_count {
            if folding.is_line_hidden(line_idx) {
                continue;
            }
            let line_content = buf.get_line_content(line_idx).unwrap_or_default();
            let line_segs = wrap_line_segments(line_idx, &line_content, max_cols);
            for mut seg in line_segs {
                seg.visual_row = visual_row;
                visual_row += 1;
                segments.push(seg);
            }
        }
        Self {
            identity: false,
            line_count,
            segments,
        }
    }

    pub fn total_rows(&self) -> usize {
        if self.identity {
            self.line_count
        } else {
            self.segments.len()
        }
    }

    pub fn segment_at(&self, visual_row: usize, buf: &PieceTable) -> Option<WrapSegment> {
        if self.identity {
            if visual_row >= self.line_count {
                return None;
            }
            let content = buf.get_line_content(visual_row).unwrap_or_default();
            Some(identity_segment(visual_row, &content))
        } else {
            self.segments.get(visual_row).cloned()
        }
    }

    pub fn segments_in_range(
        &self,
        start_row: usize,
        end_row: usize,
        buf: &PieceTable,
    ) -> Vec<WrapSegment> {
        if self.identity {
            let end = end_row.min(self.line_count);
            let start = start_row.min(end);
            (start..end)
                .map(|line_idx| {
                    let content = buf.get_line_content(line_idx).unwrap_or_default();
                    identity_segment(line_idx, &content)
                })
                .collect()
        } else {
            let end = end_row.min(self.segments.len());
            let start = start_row.min(end);
            self.segments[start..end].to_vec()
        }
    }

    pub fn buffer_pos_to_visual(
        &self,
        pos: Position,
        buf: &PieceTable,
        folding: &FoldingModel,
    ) -> (usize, usize) {
        let (eff_line, eff_col) = if folding.is_line_hidden(pos.line) {
            let header = folding.enclosing_collapsed_start(pos.line).unwrap_or(pos.line);
            let h_len = buf
                .get_line_content(header)
                .map_or(0, |l| l.chars().map(|c| c.len_utf16()).sum());
            (header, h_len)
        } else {
            (pos.line, pos.column)
        };

        let line_content = buf.get_line_content(eff_line).unwrap_or_default();
        let char_in_line = utf16_col_to_char_count(&line_content, eff_col);

        if self.identity {
            return (eff_line.min(self.line_count.saturating_sub(1)), char_in_line);
        }

        for seg in &self.segments {
            if seg.buffer_line == eff_line && (seg.is_last_segment || eff_col < seg.utf16_end) {
                return (seg.visual_row, char_in_line.saturating_sub(seg.char_start));
            }
        }

        if let Some(last) = self.segments.last() {
            (last.visual_row, 0)
        } else {
            (0, 0)
        }
    }

    pub fn visual_to_buffer_pos(
        &self,
        visual_row: usize,
        visual_char_col: usize,
        buf: &PieceTable,
    ) -> Position {
        let total = self.total_rows();
        if total == 0 {
            return Position::new(0, 0);
        }
        let clamped_row = visual_row.min(total - 1);
        let Some(seg) = self.segment_at(clamped_row, buf) else {
            return Position::new(0, 0);
        };
        let line_content = buf.get_line_content(seg.buffer_line).unwrap_or_default();
        let seg_slice = &line_content[seg.byte_start..seg.byte_end];
        let seg_chars = seg.char_end - seg.char_start;
        let rel_char = visual_char_col.min(seg_chars);
        let rel_utf16: usize = seg_slice
            .chars()
            .take(rel_char)
            .map(|c| c.len_utf16())
            .sum();
        Position::new(seg.buffer_line, seg.utf16_start + rel_utf16)
    }

    pub fn visual_scroll_to_buffer_scroll(&self, scroll_top: f32) -> f32 {
        if self.identity || self.segments.is_empty() {
            return scroll_top;
        }
        let safe = if scroll_top.is_finite() {
            scroll_top.max(0.0)
        } else {
            0.0
        };
        let row = (safe.floor().clamp(0.0, usize::MAX as f32) as usize)
            .min(self.segments.len().saturating_sub(1));
        self.segments[row].buffer_line as f32
    }

    pub fn buffer_scroll_to_visual_scroll(&self, buf_scroll: f32) -> f32 {
        if self.identity || self.segments.is_empty() {
            return buf_scroll;
        }
        let safe = if buf_scroll.is_finite() {
            buf_scroll.max(0.0)
        } else {
            0.0
        };
        let target_line = safe.floor().clamp(0.0, usize::MAX as f32) as usize;
        if let Some(seg) = self.segments.iter().find(|s| s.buffer_line >= target_line) {
            seg.visual_row as f32
        } else {
            self.segments.last().map_or(0.0, |s| s.visual_row as f32)
        }
    }
}

fn identity_segment(line_idx: usize, line_content: &str) -> WrapSegment {
    let total_chars = line_content.chars().count();
    let total_utf16: usize = line_content.chars().map(|c| c.len_utf16()).sum();
    WrapSegment {
        buffer_line: line_idx,
        visual_row: line_idx,
        is_first_segment: true,
        is_last_segment: true,
        byte_start: 0,
        byte_end: line_content.len(),
        char_start: 0,
        char_end: total_chars,
        utf16_start: 0,
        utf16_end: total_utf16,
    }
}

fn wrap_line_segments(
    line_idx: usize,
    line_content: &str,
    max_cols: Option<usize>,
) -> Vec<WrapSegment> {
    let Some(max_c) = max_cols.map(|c| c.max(1)) else {
        return vec![identity_segment(line_idx, line_content)];
    };
    if line_content.is_empty() {
        return vec![identity_segment(line_idx, line_content)];
    }

    let mut segments = Vec::new();
    let mut seg_start_byte = 0usize;
    let mut seg_start_char = 0usize;
    let mut seg_start_utf16 = 0usize;
    let mut seg_chars = 0usize;
    let mut last_break: Option<(usize, usize, usize)> = None;

    let mut char_idx = 0usize;
    let mut utf16_idx = 0usize;

    for (byte_idx, ch) in line_content.char_indices() {
        if seg_chars >= max_c {
            let (split_byte, split_char, split_utf16) = match last_break {
                Some((lb_b, lb_c, lb_u)) if lb_c > seg_start_char => (lb_b, lb_c, lb_u),
                _ => (byte_idx, char_idx, utf16_idx),
            };
            segments.push(WrapSegment {
                buffer_line: line_idx,
                visual_row: 0,
                is_first_segment: segments.is_empty(),
                is_last_segment: false,
                byte_start: seg_start_byte,
                byte_end: split_byte,
                char_start: seg_start_char,
                char_end: split_char,
                utf16_start: seg_start_utf16,
                utf16_end: split_utf16,
            });
            seg_start_byte = split_byte;
            seg_start_char = split_char;
            seg_start_utf16 = split_utf16;
            seg_chars = char_idx - seg_start_char;
            if let Some((_, lb_c, _)) = last_break {
                if lb_c <= seg_start_char {
                    last_break = None;
                }
            }
        }

        let next_byte = byte_idx + ch.len_utf8();
        let next_char = char_idx + 1;
        let next_utf16 = utf16_idx + ch.len_utf16();

        if ch.is_whitespace() {
            last_break = Some((next_byte, next_char, next_utf16));
        }

        seg_chars += 1;
        char_idx = next_char;
        utf16_idx = next_utf16;
    }

    segments.push(WrapSegment {
        buffer_line: line_idx,
        visual_row: 0,
        is_first_segment: segments.is_empty(),
        is_last_segment: true,
        byte_start: seg_start_byte,
        byte_end: line_content.len(),
        char_start: seg_start_char,
        char_end: char_idx,
        utf16_start: seg_start_utf16,
        utf16_end: utf16_idx,
    });

    segments
}

#[derive(Debug, Clone)]
struct CursorUndoSnapshot {
    pre_sels: Vec<Selection>,
    post_sels: Vec<Selection>,
    fold_shifts: Vec<(usize, isize)>,
}

pub struct EditorView {
    pub buffer: Arc<RwLock<PieceTable>>,
    pub cursor_state: CursorState,
    pub undo_stack: UndoRedoStack,
    pub scroll_top: f32,
    pub scroll_left: f32,
    pub is_focused: bool,
    pub syntax_highlighter: Mutex<Option<SyntaxHighlighter>>,
    pub document: Option<Arc<RwLock<TextDocument>>>,
    pub autocomplete: AutocompleteWidget,
    pub completion_provider: Option<CompletionProviderFn>,
    pub diagnostics: Vec<EditorDiagnostic>,
    pub hovered_diagnostic: Option<EditorDiagnostic>,
    pub hover_position: Option<Point>,
    pub breakpoints: BTreeSet<usize>,
    pub execution_line: Option<usize>,
    pub last_gutter_action: Option<EditorGutterAction>,
    pub minimap_enabled: bool,
    pub minimap_width: f32,
    pub minimap_dragging: bool,
    pub folding_model: FoldingModel,
    pub word_wrap: WordWrapMode,
    pub word_wrap_column: usize,
    cursor_undo_snapshots: Vec<Option<CursorUndoSnapshot>>,
    cursor_redo_snapshots: Vec<Option<CursorUndoSnapshot>>,
    last_render_bounds: Mutex<Option<(Rect, FontMetrics)>>,
}

impl EditorView {
    pub fn new(buffer: Arc<RwLock<PieceTable>>) -> Self {
        let mut folding_model = FoldingModel::new();
        {
            let buf = buffer.read();
            folding_model.compute_from_buffer(&buf, None);
        }
        Self {
            buffer,
            cursor_state: CursorState::new(),
            undo_stack: UndoRedoStack::new(),
            scroll_top: 0.0,
            scroll_left: 0.0,
            is_focused: true,
            syntax_highlighter: Mutex::new(None),
            document: None,
            autocomplete: AutocompleteWidget::new(),
            completion_provider: None,
            diagnostics: Vec::new(),
            hovered_diagnostic: None,
            hover_position: None,
            breakpoints: BTreeSet::new(),
            execution_line: None,
            last_gutter_action: None,
            minimap_enabled: true,
            minimap_width: minimap::DEFAULT_MINIMAP_WIDTH,
            minimap_dragging: false,
            folding_model,
            word_wrap: WordWrapMode::Off,
            word_wrap_column: 80,
            cursor_undo_snapshots: Vec::new(),
            cursor_redo_snapshots: Vec::new(),
            last_render_bounds: Mutex::new(None),
        }
    }

    pub fn toggle_breakpoint(&mut self, line: usize) -> bool {
        if self.breakpoints.contains(&line) {
            self.breakpoints.remove(&line);
            false
        } else {
            self.breakpoints.insert(line);
            true
        }
    }

    pub fn add_breakpoint(&mut self, line: usize) {
        self.breakpoints.insert(line);
    }

    pub fn remove_breakpoint(&mut self, line: usize) {
        self.breakpoints.remove(&line);
    }

    pub fn has_breakpoint(&self, line: usize) -> bool {
        self.breakpoints.contains(&line)
    }

    pub fn set_execution_line(&mut self, line: Option<usize>) {
        self.execution_line = line;
    }

    pub fn set_diagnostics(&mut self, diagnostics: Vec<EditorDiagnostic>) {
        self.diagnostics = diagnostics;
        self.hovered_diagnostic = None;
        self.hover_position = None;
    }

    pub fn clear_diagnostics(&mut self) {
        self.diagnostics.clear();
        self.hovered_diagnostic = None;
        self.hover_position = None;
    }

    pub fn diagnostics_for_line(&self, line: usize) -> Vec<&EditorDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.range.start.line <= line && d.range.end.line >= line)
            .collect()
    }

    pub fn diagnostic_at_position(&self, pos: Position) -> Option<&EditorDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| {
                if pos.line < d.range.start.line || pos.line > d.range.end.line {
                    return false;
                }
                if d.range.start.line == d.range.end.line {
                    pos.column >= d.range.start.column && pos.column <= d.range.end.column
                } else if pos.line == d.range.start.line {
                    pos.column >= d.range.start.column
                } else if pos.line == d.range.end.line {
                    pos.column <= d.range.end.column
                } else {
                    true
                }
            })
            .min_by_key(|d| d.severity as u8)
    }

    pub fn hover_card_bounds(
        diag: &EditorDiagnostic,
        mouse_pt: Point,
        bounds: Rect,
        font: &FontMetrics,
    ) -> Rect {
        let header_text = diag.formatted_header();
        let message_text = &diag.message;

        let header_w = font.text_width(&header_text);
        let message_w = font.text_width(message_text);
        let card_w = (header_w.max(message_w) + 28.0).max(180.0).min(bounds.width() - 40.0);
        let card_h = font.line_height * 2.0 + 16.0;

        let mut card_x = mouse_pt.x + 8.0;
        if card_x + card_w > bounds.max_x() - 10.0 {
            card_x = (bounds.max_x() - card_w - 10.0).max(bounds.min_x() + 10.0);
        }
        let mut card_y = mouse_pt.y + 16.0;
        if card_y + card_h > bounds.max_y() - 10.0 {
            card_y = (mouse_pt.y - card_h - 6.0).max(bounds.min_y() + 10.0);
        }

        Rect::from_xywh(card_x, card_y, card_w, card_h)
    }

    fn detected_language_id(&self) -> Option<String> {
        if let Some(doc) = &self.document {
            return Some(doc.read().language_id.clone());
        }
        self.syntax_highlighter
            .lock()
            .as_ref()
            .map(|sh| sh.grammar().language_id().to_string())
    }

    pub fn recompute_folding(&mut self) {
        let lang = self.detected_language_id();
        let buf = self.buffer.read();
        self.folding_model.recompute(&buf, lang.as_deref());
    }

    pub fn with_syntax(mut self, highlighter: SyntaxHighlighter) -> Self {
        let lang = highlighter.grammar().language_id().to_string();
        *self.syntax_highlighter.lock() = Some(highlighter);
        {
            let buf = self.buffer.read();
            self.folding_model.compute_from_buffer(&buf, Some(&lang));
        }
        self
    }

    pub fn with_document(mut self, doc: Arc<RwLock<TextDocument>>) -> Self {
        self.set_document(doc);
        self
    }

    pub fn with_completion_provider(mut self, provider: CompletionProviderFn) -> Self {
        self.completion_provider = Some(provider);
        self
    }

    pub fn with_minimap(mut self, enabled: bool) -> Self {
        self.set_minimap_enabled(enabled);
        self
    }

    pub fn set_minimap_enabled(&mut self, enabled: bool) {
        self.minimap_enabled = enabled;
        if !enabled {
            self.minimap_dragging = false;
        }
    }

    pub fn with_word_wrap(mut self, mode: WordWrapMode) -> Self {
        self.set_word_wrap(mode);
        self
    }

    pub fn set_word_wrap(&mut self, mode: WordWrapMode) {
        if self.word_wrap == mode {
            return;
        }
        self.word_wrap = mode;
        self.clamp_scroll_top();
    }

    pub fn with_word_wrap_column(mut self, col: usize) -> Self {
        self.set_word_wrap_column(col);
        self
    }

    pub fn set_word_wrap_column(&mut self, col: usize) {
        let clamped = col.max(1);
        if self.word_wrap_column == clamped {
            return;
        }
        self.word_wrap_column = clamped;
        self.clamp_scroll_top();
    }

    fn clamp_scroll_top(&mut self) {
        if !self.scroll_top.is_finite() {
            self.scroll_top = 0.0;
            return;
        }
        let cached = self.last_render_bounds.lock().clone();
        let max_scroll_top = {
            let buf = self.buffer.read();
            if let Some((bounds, font)) = cached {
                let gutter_w = self.gutter_width(&font);
                let max_cols = self.effective_wrap_columns(bounds, gutter_w, &font);
                VisualLayout::build(&buf, &self.folding_model, max_cols)
                    .total_rows()
                    .saturating_sub(1) as f32
            } else {
                match self.word_wrap {
                    WordWrapMode::Off => self
                        .folding_model
                        .visible_line_count(buf.line_count())
                        .saturating_sub(1) as f32,
                    WordWrapMode::WordWrapColumn => VisualLayout::build(
                        &buf,
                        &self.folding_model,
                        Some(self.word_wrap_column.max(1)),
                    )
                    .total_rows()
                    .saturating_sub(1) as f32,
                    WordWrapMode::On | WordWrapMode::Bounded => {
                        // Without known viewport bounds, use the worst-case (1 char/row) visible
                        // row bound so empty/1-char buffers clamp to 0 without over-clamping
                        // wrapped lines for narrow viewports.
                        let worst_case_rows: usize = (0..buf.line_count())
                            .filter(|&i| !self.folding_model.is_line_hidden(i))
                            .map(|i| buf.get_line_content(i).map_or(1, |s| s.chars().count().max(1)))
                            .sum();
                        worst_case_rows.saturating_sub(1) as f32
                    }
                }
            }
        };
        self.scroll_top = self.scroll_top.clamp(0.0, max_scroll_top);
    }

    fn record_cursor_undo_snapshot(
        &mut self,
        prev_past_len: usize,
        pre_sels: Vec<Selection>,
        post_sels: Vec<Selection>,
        fold_shifts: Vec<(usize, isize)>,
    ) {
        let new_past_len = self.undo_stack.past_len();
        if new_past_len > prev_past_len {
            self.cursor_redo_snapshots.clear();
            let target_base = new_past_len - 1;
            if self.cursor_undo_snapshots.len() > target_base {
                self.cursor_undo_snapshots.truncate(target_base);
            }
            while self.cursor_undo_snapshots.len() < target_base {
                self.cursor_undo_snapshots.push(None);
            }
            self.cursor_undo_snapshots.push(Some(CursorUndoSnapshot {
                pre_sels,
                post_sels,
                fold_shifts,
            }));
        }
    }

    pub(crate) fn effective_wrap_columns(
        &self,
        bounds: Rect,
        gutter_w: f32,
        font: &FontMetrics,
    ) -> Option<usize> {
        if self.word_wrap == WordWrapMode::Off {
            return None;
        }
        let text_area_x = bounds.min_x() + gutter_w + 8.0;
        let mm_rect = self.minimap_rect_with_gutter(bounds, gutter_w);
        let code_right = mm_rect.map_or(bounds.max_x(), |mm| mm.min_x());
        let avail_w = code_right - text_area_x;
        let viewport_cols = if avail_w.is_finite()
            && avail_w > 0.0
            && font.char_width.is_finite()
            && font.char_width > 0.0
        {
            (avail_w / font.char_width).floor().max(1.0) as usize
        } else {
            1
        };
        let configured = self.word_wrap_column.max(1);
        Some(match self.word_wrap {
            WordWrapMode::Off => unreachable!(),
            WordWrapMode::On => viewport_cols,
            WordWrapMode::WordWrapColumn => configured,
            WordWrapMode::Bounded => viewport_cols.min(configured).max(1),
        })
    }

    pub fn minimap_rect_with_gutter(&self, bounds: Rect, gutter_w: f32) -> Option<Rect> {
        if !self.minimap_enabled
            || !self.minimap_width.is_finite()
            || self.minimap_width <= 0.0
            || !gutter_w.is_finite()
            || gutter_w < 0.0
            || !is_finite_positive_rect(bounds)
            || bounds.width() < gutter_w + self.minimap_width + 80.0
        {
            return None;
        }
        Some(Rect::from_xywh(
            bounds.max_x() - self.minimap_width,
            bounds.min_y(),
            self.minimap_width,
            bounds.height(),
        ))
    }

    pub fn minimap_rect(&self, bounds: Rect) -> Option<Rect> {
        self.minimap_rect_with_gutter(bounds, self.gutter_width(&FontMetrics::default_monospace()))
    }

    pub fn accessibility_node(&self, id: &str, bounds: Rect, font: &FontMetrics) -> AccessibleNode {
        let safe_bounds = if is_finite_rect(bounds) {
            Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), bounds.height())
        } else {
            Rect::zero()
        };
        let (line_count, text) = {
            let buf = self.buffer.read();
            let count = buf.line_count();
            let full = buf.to_string();
            let truncated = if full.len() > 4096 {
                let mut end = 4096;
                while !full.is_char_boundary(end) {
                    end -= 1;
                }
                full[..end].to_string()
            } else {
                full
            };
            (count, truncated)
        };

        let (name, lang_id, is_dirty) = if let Some(doc) = &self.document {
            let d = doc.read();
            let file_name = d
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| "Text Editor".to_string());
            (file_name, d.language_id.clone(), d.is_dirty)
        } else {
            ("Text Editor".to_string(), "plaintext".to_string(), false)
        };

        let pos = self.primary_position();
        let dirty_suffix = if is_dirty { ", modified" } else { "" };
        let description = format!(
            "Ln {}, Col {}, {} lines, {}{}",
            pos.line + 1,
            pos.column + 1,
            line_count,
            lang_id,
            dirty_suffix
        );

        let mut node = AccessibleNode::new(id, AccessibleRole::TextEditor, name, safe_bounds)
            .with_value(text)
            .with_description(description)
            .with_state(AccessibleState::default().with_focused(self.is_focused))
            .with_actions(["focus", "edit", "scroll"]);

        if let Some(mm_rect) = self.minimap_rect_with_gutter(safe_bounds, self.gutter_width(font)) {
            let max_line = line_count.max(1);
            let safe_scroll_top = if self.scroll_top.is_finite() {
                self.scroll_top.max(0.0)
            } else {
                0.0
            };
            let buf_scroll_top = if self.word_wrap == WordWrapMode::Off && !self.folding_model.has_collapsed() {
                safe_scroll_top
            } else {
                let gutter_w = self.gutter_width(font);
                let max_cols = self.effective_wrap_columns(safe_bounds, gutter_w, font);
                let buf = self.buffer.read();
                let visual = VisualLayout::build(&buf, &self.folding_model, max_cols);
                visual.visual_scroll_to_buffer_scroll(safe_scroll_top)
            };
            let cur_top = (buf_scroll_top.floor() as usize)
                .saturating_add(1)
                .min(max_line);
            node.add_child(
                AccessibleNode::new(
                    format!("{}-minimap", id),
                    AccessibleRole::ScrollBar,
                    "Minimap",
                    mm_rect,
                )
                .with_value(format!("Line {} of {}", cur_top, max_line))
                .with_description("Editor minimap scrollbar")
                .with_action("scroll"),
            );
        }

        if self.autocomplete.is_visible {
            if let Some(pop_bounds) = *self.autocomplete.last_bounds.lock() {
                if is_finite_rect(pop_bounds) {
                    node.add_child(
                        AccessibleNode::new(
                            format!("{}-autocomplete", id),
                            AccessibleRole::List,
                            "Autocomplete Suggestions",
                            pop_bounds,
                        )
                        .with_action("select"),
                    );
                }
            }
        }

        node
    }

    pub fn set_document(&mut self, doc: Arc<RwLock<TextDocument>>) {
        let (buf_arc, lang_id) = {
            let d = doc.read();
            (d.buffer.clone(), d.language_id.clone())
        };
        self.buffer = buf_arc;
        self.document = Some(doc);
        let buf = self.buffer.read();
        self.folding_model.compute_from_buffer(&buf, Some(&lang_id));
    }

    pub fn set_language(&mut self, lang_id: &str, theme: ColorTheme, registry: &GrammarRegistry) {
        let sh = SyntaxHighlighter::for_language(lang_id, theme, registry);
        *self.syntax_highlighter.lock() = Some(sh);
        let buf = self.buffer.read();
        self.folding_model.compute_from_buffer(&buf, Some(lang_id));
    }

    pub fn set_theme(&mut self, theme: ColorTheme) {
        if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
            sh.set_theme(theme);
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|d| d.read().is_dirty)
    }

    pub fn save_sync(&mut self, fs: &FileSystemService) -> Result<FileMetadata, FileSystemError> {
        if let Some(doc) = &self.document {
            doc.write().save_sync(fs)
        } else {
            Err(FileSystemError::NotFound(std::path::PathBuf::from("unnamed")))
        }
    }

    pub async fn save_async(&mut self, fs: &FileSystemService) -> Result<FileMetadata, FileSystemError> {
        let (path, content) = if let Some(doc) = &self.document {
            let d = doc.read();
            let p = d.path.clone().ok_or_else(|| {
                FileSystemError::NotFound(std::path::PathBuf::from("unnamed"))
            })?;
            (p, d.text())
        } else {
            return Err(FileSystemError::NotFound(std::path::PathBuf::from("unnamed")));
        };

        let meta = fs.write_atomic(&path, &content).await?;
        if let Some(doc) = &self.document {
            doc.write().mark_saved(meta.clone());
        }
        Ok(meta)
    }



    #[allow(clippy::should_implement_trait)]
    pub fn from_str(content: &str) -> Self {
        Self::new(Arc::new(RwLock::new(PieceTable::from_str(content))))
    }

    pub fn primary_position(&self) -> Position {
        self.cursor_state.primary().active
    }

    pub fn set_primary_position(&mut self, pos: Position) {
        self.cursor_state.set_primary(pos);
    }

    pub fn text(&self) -> String {
        self.buffer.read().to_string()
    }

    pub fn gutter_width(&self, font: &FontMetrics) -> f32 {
        let line_count = self.buffer.read().line_count();
        let digits = line_count.to_string().len();
        let char_w = if font.char_width.is_finite() && font.char_width > 0.0 {
            font.char_width
        } else {
            0.0
        };
        let raw = (digits + 4) as f32 * char_w + 14.0;
        if raw.is_finite() {
            raw.max(56.0)
        } else {
            56.0
        }
    }

    pub fn trigger_suggest(&mut self, trigger_char: Option<char>) {
        let pos = self.primary_position();
        let lang = self
            .document
            .as_ref()
            .map(|d| d.read().language_id.clone())
            .unwrap_or_else(|| "rust".to_string());

        // Calculate trigger anchor position and prefix
        let (trigger_pos, prefix) = {
            let buf = self.buffer.read();
            if let Some(line) = buf.get_line_content(pos.line) {
                let col = pos.column.min(line.len());
                if trigger_char.is_none() {
                    let mut word_start = col;
                    while word_start > 0 {
                        let prev_idx = line[..word_start]
                            .char_indices()
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        let ch = line[prev_idx..word_start].chars().next().unwrap();
                        if ch.is_alphanumeric() || ch == '_' {
                            word_start = prev_idx;
                        } else {
                            break;
                        }
                    }
                    let prefix = if col > word_start {
                        line[word_start..col].to_string()
                    } else {
                        String::new()
                    };
                    (Position::new(pos.line, word_start), prefix)
                } else {
                    (pos, String::new())
                }
            } else {
                (pos, String::new())
            }
        };

        let mut items = Vec::new();

        // 1. Query external completion provider if attached
        if let Some(provider) = &self.completion_provider {
            let prov_items = (provider)(&lang, pos, trigger_char);
            items.extend(prov_items);
        }

        // 2. Built-in keywords, snippets, and document identifiers
        let text = self.buffer.read().to_string();
        let current_pref = if prefix.is_empty() { None } else { Some(prefix.as_str()) };
        let builtin = get_builtin_completions(&lang, trigger_char, &text, current_pref);
        for b in builtin {
            if !items.iter().any(|i| i.label == b.label) {
                items.push(b);
            }
        }

        if !items.is_empty() {
            self.autocomplete.show(items, trigger_pos, trigger_char);
            if !prefix.is_empty() {
                self.autocomplete.update_filter(&prefix);
            }
        }
    }

    pub fn commit_autocomplete(&mut self) -> bool {
        if !self.autocomplete.is_visible {
            return false;
        }

        let primary_before = self.primary_position();
        let pre_sels = self.cursor_state.selections.clone();
        let prev_past_len = self.undo_stack.past_len();
        let old_line_count;
        let committed;
        {
            let mut buf = self.buffer.write();
            old_line_count = buf.line_count();
            committed = self.autocomplete.commit(
                &mut buf,
                &mut self.cursor_state,
                &mut self.undo_stack,
            );
        }

        if committed.is_some() {
            let post_sels = self.cursor_state.selections.clone();
            let lang = self.detected_language_id();
            let fold_shifts = {
                let buf = self.buffer.read();
                let new_line_count = buf.line_count();
                let delta = new_line_count as isize - old_line_count as isize;
                self.folding_model
                    .adjust_for_edit(primary_before.line, delta, &buf, lang.as_deref());
                if delta != 0 {
                    vec![(primary_before.line, delta)]
                } else {
                    Vec::new()
                }
            };
            self.record_cursor_undo_snapshot(prev_past_len, pre_sels, post_sels, fold_shifts);
            if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
                sh.invalidate_from(primary_before.line);
            }
            if let Some(doc) = &self.document {
                doc.write().mark_edited();
            }
            true
        } else {
            false
        }
    }

    // --- Editing & Movement Operations ---

    /// Applies per-selection edits atomically across all cursors:
    /// computes shifted post-edit byte offsets in ascending order, mutates `PieceTable`
    /// in descending order inside an `UndoRedoStack` group (when > 1 mutation), then
    /// converts final offsets to UTF-16 `Position`s and normalizes/merges cursors.
    fn apply_cursor_edits(&mut self, entries: Vec<(usize, usize, usize, String, bool)>) -> bool {
        let num_sels = self.cursor_state.selections.len();
        if num_sels == 0 || entries.is_empty() {
            return false;
        }

        let pre_sels = self.cursor_state.selections.clone();
        let prev_past_len = self.undo_stack.past_len();

        let mut sorted = entries;
        sorted.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)).then_with(|| a.0.cmp(&b.0)));

        // Compute post-edit byte offsets in ascending order using cumulative byte shift.
        let mut shift: isize = 0;
        let mut final_offsets = vec![0usize; num_sels];
        let mut mutations: Vec<(usize, usize, String)> = Vec::new();
        let mut prev_mut: Option<(usize, usize)> = None;

        for (sel_idx, start_off, end_off, new_text, is_mut) in sorted {
            let overlaps_prev_mut = prev_mut.is_some_and(|(ps, pe)| {
                start_off < pe || (ps == pe && start_off == ps && end_off == pe)
            });
            if is_mut && !overlaps_prev_mut {
                let inserted_len = new_text.len() as isize;
                let deleted_len = end_off.saturating_sub(start_off) as isize;
                let new_off = (start_off as isize + shift + inserted_len).max(0) as usize;
                final_offsets[sel_idx] = new_off;
                shift += inserted_len - deleted_len;
                prev_mut = Some((start_off, end_off));
                mutations.push((start_off, end_off, new_text));
            } else {
                final_offsets[sel_idx] = (start_off as isize + shift).max(0) as usize;
            }
        }

        if mutations.is_empty() {
            return false;
        }

        let min_start_offset = mutations.iter().map(|m| m.0).min().unwrap_or(0);
        let min_affected_line;
        let mut fold_shifts: Vec<(usize, isize)> = Vec::new();

        {
            let mut buf = self.buffer.write();
            min_affected_line = buf.offset_to_position(min_start_offset).0;

            let use_group = mutations.len() > 1;
            if use_group {
                self.undo_stack.begin_group();
            }

            for (start_off, end_off, new_text) in mutations.into_iter().rev() {
                let edit_start_line = buf.offset_to_position(start_off).0;
                let del_len = end_off.saturating_sub(start_off);
                let deleted = if del_len > 0 {
                    buf.delete(start_off, del_len)
                } else {
                    String::new()
                };
                if !new_text.is_empty() {
                    buf.insert(start_off, &new_text);
                }
                let deleted_lines = deleted.bytes().filter(|&b| b == b'\n').count() as isize;
                let inserted_lines = new_text.bytes().filter(|&b| b == b'\n').count() as isize;
                let mut_lines_delta = inserted_lines - deleted_lines;
                if mut_lines_delta != 0 {
                    fold_shifts.push((edit_start_line, mut_lines_delta));
                }
                if !deleted.is_empty() && !new_text.is_empty() {
                    self.undo_stack
                        .push_edit(TextEdit::replace(start_off, &deleted, &new_text));
                } else if !deleted.is_empty() {
                    self.undo_stack
                        .push_edit(TextEdit::delete(start_off, &deleted));
                } else if !new_text.is_empty() {
                    self.undo_stack
                        .push_edit(TextEdit::insert(start_off, &new_text));
                }
            }

            if use_group {
                self.undo_stack.end_group();
            }

            let buf_len = buf.len();
            let mut updated_sels = Vec::with_capacity(num_sels);
            for off in final_offsets {
                let (line, col) = buf.offset_to_position(off.min(buf_len));
                updated_sels.push(Selection::cursor(Position::new(line, col)));
            }
            self.cursor_state.selections = updated_sels;
        }

        self.cursor_state.normalize_and_merge();
        let post_sels = self.cursor_state.selections.clone();
        self.record_cursor_undo_snapshot(prev_past_len, pre_sels, post_sels, fold_shifts.clone());

        let lang = self.detected_language_id();
        {
            let buf = self.buffer.read();
            for (edit_line, delta) in fold_shifts {
                self.folding_model.shift_lines_for_edit(edit_line, delta);
            }
            self.folding_model.recompute(&buf, lang.as_deref());
        }
        let cursor_lines: Vec<usize> = self
            .cursor_state
            .selections
            .iter()
            .map(|s| s.active.line)
            .collect();
        for l in cursor_lines {
            self.folding_model.unfold_containing(l);
        }

        if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
            sh.invalidate_from(min_affected_line);
        }
        if let Some(doc) = &self.document {
            doc.write().mark_edited();
        }
        true
    }

    pub fn delete_selection(&mut self) -> bool {
        self.cursor_state.normalize_and_merge();
        let has_non_empty = self.cursor_state.selections.iter().any(|s| !s.is_empty());
        if !has_non_empty {
            return false;
        }

        let entries: Vec<(usize, usize, usize, String, bool)> = {
            let buf = self.buffer.read();
            self.cursor_state
                .selections
                .iter()
                .enumerate()
                .map(|(idx, sel)| {
                    let s = sel.start();
                    let e = sel.end();
                    let s_off = buf.position_to_offset(s.line, s.column);
                    let e_off = buf.position_to_offset(e.line, e.column);
                    let (so, eo) = if s_off <= e_off {
                        (s_off, e_off)
                    } else {
                        (e_off, s_off)
                    };
                    (idx, so, eo, String::new(), eo > so)
                })
                .collect()
        };

        if !self.apply_cursor_edits(entries) {
            for sel in &mut self.cursor_state.selections {
                let s = sel.start();
                *sel = Selection::cursor(s);
            }
            self.cursor_state.normalize_and_merge();
            false
        } else {
            true
        }
    }

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.cursor_state.normalize_and_merge();

        let entries: Vec<(usize, usize, usize, String, bool)> = {
            let buf = self.buffer.read();
            self.cursor_state
                .selections
                .iter()
                .enumerate()
                .map(|(idx, sel)| {
                    let s = sel.start();
                    let e = sel.end();
                    let s_off = buf.position_to_offset(s.line, s.column);
                    let e_off = buf.position_to_offset(e.line, e.column);
                    let (so, eo) = if s_off <= e_off {
                        (s_off, e_off)
                    } else {
                        (e_off, s_off)
                    };
                    (idx, so, eo, text.to_string(), true)
                })
                .collect()
        };

        self.apply_cursor_edits(entries);
    }

    pub fn delete_backwards(&mut self) {
        if self.delete_selection() {
            return;
        }
        self.cursor_state.normalize_and_merge();

        let entries: Vec<(usize, usize, usize, String, bool)> = {
            let buf = self.buffer.read();
            self.cursor_state
                .selections
                .iter()
                .enumerate()
                .map(|(idx, sel)| {
                    let off = buf.position_to_offset(sel.active.line, sel.active.column);
                    if off > 0 {
                        let prev = buf.prev_char_boundary(off);
                        (idx, prev, off, String::new(), true)
                    } else {
                        (idx, 0, 0, String::new(), false)
                    }
                })
                .collect()
        };

        self.apply_cursor_edits(entries);
    }

    pub fn delete_forwards(&mut self) {
        if self.delete_selection() {
            return;
        }
        self.cursor_state.normalize_and_merge();

        let entries: Vec<(usize, usize, usize, String, bool)> = {
            let buf = self.buffer.read();
            let buf_len = buf.len();
            self.cursor_state
                .selections
                .iter()
                .enumerate()
                .map(|(idx, sel)| {
                    let off = buf.position_to_offset(sel.active.line, sel.active.column);
                    if off < buf_len {
                        let next = buf.next_char_boundary(off);
                        (idx, off, next, String::new(), true)
                    } else {
                        (idx, off, off, String::new(), false)
                    }
                })
                .collect()
        };

        self.apply_cursor_edits(entries);
    }

    pub fn move_cursor_left(&mut self, select: bool) {
        {
            let buf = self.buffer.read();
            for sel in &mut self.cursor_state.selections {
                let pos = sel.active;
                let offset = buf.position_to_offset(pos.line, pos.column);
                let target = if offset > 0 {
                    let prev_offset = buf.prev_char_boundary(offset);
                    let (new_line, new_col) = buf.offset_to_position(prev_offset);
                    Position::new(new_line, new_col)
                } else {
                    pos
                };
                if select {
                    sel.active = target;
                } else {
                    *sel = Selection::cursor(target);
                }
            }
        }
        self.cursor_state.normalize_and_merge();
    }

    pub fn move_cursor_right(&mut self, select: bool) {
        {
            let buf = self.buffer.read();
            let buf_len = buf.len();
            for sel in &mut self.cursor_state.selections {
                let pos = sel.active;
                let offset = buf.position_to_offset(pos.line, pos.column);
                let target = if offset < buf_len {
                    let next_offset = buf.next_char_boundary(offset);
                    let (new_line, new_col) = buf.offset_to_position(next_offset);
                    Position::new(new_line, new_col)
                } else {
                    pos
                };
                if select {
                    sel.active = target;
                } else {
                    *sel = Selection::cursor(target);
                }
            }
        }
        self.cursor_state.normalize_and_merge();
    }

    pub fn move_cursor_up(&mut self, select: bool) {
        {
            let buf = self.buffer.read();
            for sel in &mut self.cursor_state.selections {
                let pos = sel.active;
                if let Some(target_line) = self.folding_model.prev_visible_line(pos.line) {
                    let content = buf.get_line_content(target_line).unwrap_or_default();
                    let target_col = clamp_utf16_col(&content, pos.column);
                    let target = Position::new(target_line, target_col);
                    if select {
                        sel.active = target;
                    } else {
                        *sel = Selection::cursor(target);
                    }
                } else if !select {
                    *sel = Selection::cursor(pos);
                }
            }
        }
        self.cursor_state.normalize_and_merge();
    }

    pub fn move_cursor_down(&mut self, select: bool) {
        {
            let buf = self.buffer.read();
            let line_count = buf.line_count();
            for sel in &mut self.cursor_state.selections {
                let pos = sel.active;
                if let Some(target_line) = self.folding_model.next_visible_line(pos.line, line_count) {
                    let content = buf.get_line_content(target_line).unwrap_or_default();
                    let target_col = clamp_utf16_col(&content, pos.column);
                    let target = Position::new(target_line, target_col);
                    if select {
                        sel.active = target;
                    } else {
                        *sel = Selection::cursor(target);
                    }
                } else if !select {
                    *sel = Selection::cursor(pos);
                }
            }
        }
        self.cursor_state.normalize_and_merge();
    }

    pub fn move_cursor_home(&mut self, select: bool) {
        for sel in &mut self.cursor_state.selections {
            let target = Position::new(sel.active.line, 0);
            if select {
                sel.active = target;
            } else {
                *sel = Selection::cursor(target);
            }
        }
        self.cursor_state.normalize_and_merge();
    }

    pub fn move_cursor_end(&mut self, select: bool) {
        {
            let buf = self.buffer.read();
            for sel in &mut self.cursor_state.selections {
                let line_len = buf
                    .get_line_content(sel.active.line)
                    .map_or(0, |l| l.chars().map(|c| c.len_utf16()).sum());
                let target = Position::new(sel.active.line, line_len);
                if select {
                    sel.active = target;
                } else {
                    *sel = Selection::cursor(target);
                }
            }
        }
        self.cursor_state.normalize_and_merge();
    }

    // --- Multi-Cursor & Folding Commands ---

    pub fn insert_cursor_above(&mut self) -> bool {
        let before = self.cursor_state.selections.len();
        let mut additions = Vec::new();
        {
            let buf = self.buffer.read();
            for sel in &self.cursor_state.selections {
                let pos = sel.active;
                if let Some(target_line) = self.folding_model.prev_visible_line(pos.line) {
                    let content = buf.get_line_content(target_line).unwrap_or_default();
                    let target_col = clamp_utf16_col(&content, pos.column);
                    additions.push(Selection::cursor(Position::new(target_line, target_col)));
                }
            }
        }
        for sel in additions {
            self.cursor_state.add_selection(sel);
        }
        self.cursor_state.normalize_and_merge();
        self.cursor_state.selections.len() > before
    }

    pub fn insert_cursor_below(&mut self) -> bool {
        let before = self.cursor_state.selections.len();
        let mut additions = Vec::new();
        {
            let buf = self.buffer.read();
            let line_count = buf.line_count();
            for sel in &self.cursor_state.selections {
                let pos = sel.active;
                if let Some(target_line) = self.folding_model.next_visible_line(pos.line, line_count)
                {
                    let content = buf.get_line_content(target_line).unwrap_or_default();
                    let target_col = clamp_utf16_col(&content, pos.column);
                    additions.push(Selection::cursor(Position::new(target_line, target_col)));
                }
            }
        }
        for sel in additions {
            self.cursor_state.add_selection(sel);
        }
        self.cursor_state.normalize_and_merge();
        self.cursor_state.selections.len() > before
    }

    pub fn add_selection_to_next_find_match(&mut self) -> bool {
        // Case 1: Single empty cursor -> select word under cursor (UTF-16 column safe)
        if self.cursor_state.selections.len() == 1 && self.cursor_state.primary().is_empty() {
            let pos = self.primary_position();
            let buf = self.buffer.read();
            let Some(line_content) = buf.get_line_content(pos.line) else {
                return false;
            };
            let byte_col = utf16_col_to_byte_offset(&line_content, pos.column);
            let is_word = |c: char| c.is_alphanumeric() || c == '_';

            let on_word = line_content[byte_col..]
                .chars()
                .next()
                .is_some_and(is_word);
            let before_word = byte_col > 0
                && line_content[..byte_col]
                    .chars()
                    .next_back()
                    .is_some_and(is_word);

            if !on_word && !before_word {
                return false;
            }

            let mut word_start = byte_col;
            while word_start > 0 {
                if let Some((prev_idx, ch)) = line_content[..word_start].char_indices().next_back()
                {
                    if is_word(ch) {
                        word_start = prev_idx;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            let mut word_end = byte_col;
            while word_end < line_content.len() {
                if let Some(ch) = line_content[word_end..].chars().next() {
                    if is_word(ch) {
                        word_end += ch.len_utf8();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            if word_end <= word_start {
                return false;
            }

            let start_col = crate::model::byte_offset_to_utf16_col(&line_content, word_start);
            let end_col = crate::model::byte_offset_to_utf16_col(&line_content, word_end);
            *self.cursor_state.primary_mut() = Selection::new(
                Position::new(pos.line, start_col),
                Position::new(pos.line, end_col),
            );
            return true;
        }

        // Case 2: Find next occurrence of primary selection starting after the newest selection
        let primary = self.cursor_state.primary().clone();
        if primary.is_empty() {
            return false;
        }

        let buf = self.buffer.read();
        let p_s = primary.start();
        let p_e = primary.end();
        let p_start_off = buf.position_to_offset(p_s.line, p_s.column);
        let p_end_off = buf.position_to_offset(p_e.line, p_e.column);
        let needle = buf.get_range(p_start_off, p_end_off);
        if needle.is_empty() {
            return false;
        }

        let existing_ranges: Vec<(usize, usize)> = self
            .cursor_state
            .selections
            .iter()
            .map(|sel| {
                let s = sel.start();
                let e = sel.end();
                let so = buf.position_to_offset(s.line, s.column);
                let eo = buf.position_to_offset(e.line, e.column);
                if so <= eo { (so, eo) } else { (eo, so) }
            })
            .collect();

        let search_from = existing_ranges
            .last()
            .map_or(p_end_off, |&(_, eo)| eo);
        let full_text = buf.to_string();

        let find_non_overlapping = |start_at: usize, limit_end: usize| -> Option<(usize, usize)> {
            let mut cursor = start_at.min(full_text.len());
            while !full_text.is_char_boundary(cursor) {
                cursor += 1;
            }
            while cursor < limit_end.min(full_text.len()) {
                let rel = full_text[cursor..].find(&needle)?;
                let m_start = cursor + rel;
                if m_start >= limit_end {
                    return None;
                }
                let m_end = m_start + needle.len();
                let overlaps = existing_ranges
                    .iter()
                    .any(|&(es, ee)| m_start < ee && es < m_end);
                if !overlaps {
                    return Some((m_start, m_end));
                }
                let step = full_text[m_start..]
                    .chars()
                    .next()
                    .map_or(1, |c| c.len_utf8());
                cursor = m_start + step;
            }
            None
        };

        let found = find_non_overlapping(search_from, full_text.len())
            .or_else(|| find_non_overlapping(0, search_from));

        if let Some((m_start, m_end)) = found {
            let (s_line, s_col) = buf.offset_to_position(m_start);
            let (e_line, e_col) = buf.offset_to_position(m_end);
            drop(buf);
            self.cursor_state.add_selection(Selection::new(
                Position::new(s_line, s_col),
                Position::new(e_line, e_col),
            ));
            self.folding_model.unfold_containing(s_line);
            self.folding_model.unfold_containing(e_line);
            self.cursor_state.normalize_and_merge();
            true
        } else {
            false
        }
    }

    fn clamp_cursors_to_visible(&mut self) {
        let buf = self.buffer.read();
        for sel in &mut self.cursor_state.selections {
            let mut changed = false;
            if self.folding_model.is_line_hidden(sel.active.line) {
                if let Some(header) = self.folding_model.enclosing_collapsed_start(sel.active.line) {
                    let header_content = buf.get_line_content(header).unwrap_or_default();
                    let clamped_col = clamp_utf16_col(&header_content, sel.active.column);
                    sel.active = Position::new(header, clamped_col);
                    changed = true;
                }
            }
            if self.folding_model.is_line_hidden(sel.anchor.line) {
                if let Some(header) = self.folding_model.enclosing_collapsed_start(sel.anchor.line) {
                    let header_content = buf.get_line_content(header).unwrap_or_default();
                    let clamped_col = clamp_utf16_col(&header_content, sel.anchor.column);
                    sel.anchor = Position::new(header, clamped_col);
                    changed = true;
                }
            }
            if changed && sel.is_empty() {
                sel.anchor = sel.active;
            }
        }
        drop(buf);
        self.cursor_state.normalize_and_merge();
    }

    pub fn fold_at_cursor(&mut self) -> bool {
        let line = self.primary_position().line;
        let target = if self.folding_model.is_foldable(line) && !self.folding_model.is_collapsed(line) {
            Some(line)
        } else {
            self.folding_model.find_enclosing_fold(line)
        };
        if let Some(start) = target {
            if self.folding_model.fold(start) {
                self.clamp_cursors_to_visible();
                self.clamp_scroll_top();
                return true;
            }
        }
        false
    }

    pub fn unfold_at_cursor(&mut self) -> bool {
        let line = self.primary_position().line;
        if self.folding_model.is_collapsed(line) {
            return self.folding_model.unfold(line);
        }
        if let Some(start) = self.folding_model.find_enclosing_collapsed_fold(line) {
            return self.folding_model.unfold(start);
        }
        false
    }

    pub fn fold_all(&mut self) {
        self.folding_model.fold_all();
        self.clamp_cursors_to_visible();
        self.clamp_scroll_top();
    }

    pub fn unfold_all(&mut self) {
        self.folding_model.unfold_all();
    }

    pub fn undo(&mut self) -> bool {
        let past_before = self.undo_stack.past_len();
        let mut buf = self.buffer.write();
        if let Some((target_offset, fallback_shifts)) = self.undo_stack.undo_with_line_deltas(&mut buf) {
            let snapshot = if self.cursor_undo_snapshots.len() == past_before {
                self.cursor_undo_snapshots.pop().flatten()
            } else if self.cursor_undo_snapshots.len() > past_before {
                self.cursor_undo_snapshots.truncate(self.undo_stack.past_len());
                None
            } else {
                None
            };

            if let Some(snap) = snapshot {
                let mut restored: Vec<Selection> = snap
                    .pre_sels
                    .iter()
                    .map(|sel| clamp_selection_to_buffer(&buf, sel))
                    .collect();
                if restored.is_empty() {
                    let (line, col) = buf.offset_to_position(target_offset);
                    restored.push(Selection::cursor(Position::new(line, col)));
                }
                self.cursor_state.selections = restored;
                self.cursor_state.normalize_and_merge();

                for &(line, delta) in snap.fold_shifts.iter().rev() {
                    self.folding_model.shift_lines_for_edit(line, -delta);
                }

                let future_len = self.undo_stack.future_len();
                if future_len > 0 {
                    let target_base = future_len - 1;
                    if self.cursor_redo_snapshots.len() > target_base {
                        self.cursor_redo_snapshots.truncate(target_base);
                    }
                    while self.cursor_redo_snapshots.len() < target_base {
                        self.cursor_redo_snapshots.push(None);
                    }
                    self.cursor_redo_snapshots.push(Some(snap));
                }
            } else {
                let (line, col) = buf.offset_to_position(target_offset);
                self.cursor_state.clear_secondary_cursors();
                self.cursor_state.set_primary(Position::new(line, col));

                for (edit_line, delta) in fallback_shifts {
                    self.folding_model.shift_lines_for_edit(edit_line, delta);
                }

                let future_len = self.undo_stack.future_len();
                if future_len > 0 {
                    let target_base = future_len - 1;
                    if self.cursor_redo_snapshots.len() > target_base {
                        self.cursor_redo_snapshots.truncate(target_base);
                    }
                    while self.cursor_redo_snapshots.len() < target_base {
                        self.cursor_redo_snapshots.push(None);
                    }
                    self.cursor_redo_snapshots.push(None);
                }
            }

            let lang = self.detected_language_id();
            self.folding_model.recompute(&buf, lang.as_deref());
            drop(buf);

            let cursor_lines: Vec<usize> = self
                .cursor_state
                .selections
                .iter()
                .flat_map(|s| [s.anchor.line, s.active.line])
                .collect();
            for l in cursor_lines {
                self.folding_model.unfold_containing(l);
            }
            self.clamp_scroll_top();

            if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
                sh.invalidate_all();
            }
            if let Some(doc) = &self.document {
                doc.write().mark_edited();
            }
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        let future_before = self.undo_stack.future_len();
        let mut buf = self.buffer.write();
        if let Some((target_offset, fallback_shifts)) = self.undo_stack.redo_with_line_deltas(&mut buf) {
            let snapshot = if self.cursor_redo_snapshots.len() == future_before {
                self.cursor_redo_snapshots.pop().flatten()
            } else if self.cursor_redo_snapshots.len() > future_before {
                self.cursor_redo_snapshots.truncate(self.undo_stack.future_len());
                None
            } else {
                None
            };

            if let Some(snap) = snapshot {
                let mut restored: Vec<Selection> = snap
                    .post_sels
                    .iter()
                    .map(|sel| clamp_selection_to_buffer(&buf, sel))
                    .collect();
                if restored.is_empty() {
                    let (line, col) = buf.offset_to_position(target_offset);
                    restored.push(Selection::cursor(Position::new(line, col)));
                }
                self.cursor_state.selections = restored;
                self.cursor_state.normalize_and_merge();

                for &(line, delta) in &snap.fold_shifts {
                    self.folding_model.shift_lines_for_edit(line, delta);
                }

                let past_len = self.undo_stack.past_len();
                if past_len > 0 {
                    let target_base = past_len - 1;
                    if self.cursor_undo_snapshots.len() > target_base {
                        self.cursor_undo_snapshots.truncate(target_base);
                    }
                    while self.cursor_undo_snapshots.len() < target_base {
                        self.cursor_undo_snapshots.push(None);
                    }
                    self.cursor_undo_snapshots.push(Some(snap));
                }
            } else {
                let (line, col) = buf.offset_to_position(target_offset);
                self.cursor_state.clear_secondary_cursors();
                self.cursor_state.set_primary(Position::new(line, col));

                for (edit_line, delta) in fallback_shifts {
                    self.folding_model.shift_lines_for_edit(edit_line, delta);
                }

                let past_len = self.undo_stack.past_len();
                if past_len > 0 {
                    let target_base = past_len - 1;
                    if self.cursor_undo_snapshots.len() > target_base {
                        self.cursor_undo_snapshots.truncate(target_base);
                    }
                    while self.cursor_undo_snapshots.len() < target_base {
                        self.cursor_undo_snapshots.push(None);
                    }
                    self.cursor_undo_snapshots.push(None);
                }
            }

            let lang = self.detected_language_id();
            self.folding_model.recompute(&buf, lang.as_deref());
            drop(buf);

            let cursor_lines: Vec<usize> = self
                .cursor_state
                .selections
                .iter()
                .flat_map(|s| [s.anchor.line, s.active.line])
                .collect();
            for l in cursor_lines {
                self.folding_model.unfold_containing(l);
            }
            self.clamp_scroll_top();

            if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
                sh.invalidate_all();
            }
            if let Some(doc) = &self.document {
                doc.write().mark_edited();
            }
            true
        } else {
            false
        }
    }

    pub fn apply_text_edits(&mut self, edits: &[RangeEdit]) -> bool {
        if edits.is_empty() {
            return false;
        }

        let pre_sels = self.cursor_state.selections.clone();
        let prev_past_len = self.undo_stack.past_len();

        let mut offset_edits = Vec::with_capacity(edits.len());
        {
            let buf = self.buffer.read();
            for edit in edits {
                let s = buf.position_to_offset(edit.range.start.line, edit.range.start.column);
                let e = buf.position_to_offset(edit.range.end.line, edit.range.end.column);
                let (start_offset, end_offset) = if s <= e { (s, e) } else { (e, s) };
                offset_edits.push((start_offset, end_offset, edit.new_text.clone()));
            }
        }

        // Sort descending by start offset, then descending by end offset
        offset_edits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));

        let mut fold_shifts: Vec<(usize, isize)> = Vec::new();
        self.undo_stack.begin_group();
        {
            let mut buf = self.buffer.write();
            for (start_offset, end_offset, new_text) in offset_edits {
                let edit_start_line = buf.offset_to_position(start_offset).0;
                let del_len = end_offset.saturating_sub(start_offset);
                let deleted_text = if del_len > 0 {
                    buf.delete(start_offset, del_len)
                } else {
                    String::new()
                };

                if !new_text.is_empty() {
                    buf.insert(start_offset, &new_text);
                }

                let del_lines = deleted_text.bytes().filter(|&b| b == b'\n').count() as isize;
                let ins_lines = new_text.bytes().filter(|&b| b == b'\n').count() as isize;
                let delta = ins_lines - del_lines;
                if delta != 0 {
                    fold_shifts.push((edit_start_line, delta));
                }

                self.undo_stack.push_edit(TextEdit::replace(
                    start_offset,
                    &deleted_text,
                    &new_text,
                ));
            }
        }
        self.undo_stack.end_group();

        let (new_line, new_col) = {
            let buf = self.buffer.read();
            let primary = self.primary_position();
            let offset = buf.position_to_offset(primary.line, primary.column).min(buf.len());
            let pos = buf.offset_to_position(offset);
            let lang = self.detected_language_id();
            for &(edit_line, delta) in &fold_shifts {
                self.folding_model.shift_lines_for_edit(edit_line, delta);
            }
            self.folding_model.recompute(&buf, lang.as_deref());
            pos
        };
        self.cursor_state.clear_secondary_cursors();
        self.cursor_state.set_primary(Position::new(new_line, new_col));
        let post_sels = self.cursor_state.selections.clone();
        self.record_cursor_undo_snapshot(prev_past_len, pre_sels, post_sels, fold_shifts);
        self.folding_model.unfold_containing(new_line);

        if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
            sh.invalidate_all();
        }
        if let Some(doc) = &self.document {
            doc.write().mark_edited();
        }

        true
    }
}

impl View for EditorView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !is_finite_positive_rect(bounds)
            || !font.line_height.is_finite()
            || font.line_height <= 0.0
            || !font.char_width.is_finite()
            || font.char_width < 0.0
            || !font.font_size.is_finite()
            || font.font_size < 0.0
        {
            return;
        }
        *self.last_render_bounds.lock() = Some((bounds, font.clone()));

        let buf = self.buffer.read();
        let line_count = buf.line_count();
        let gutter_w = self.gutter_width(font);
        let max_wrap_cols = self.effective_wrap_columns(bounds, gutter_w, font);
        let visual = VisualLayout::build(&buf, &self.folding_model, max_wrap_cols);
        let total_rows = visual.total_rows().max(1);
        let max_scroll_top = total_rows.saturating_sub(1) as f32;

        let scroll_top = if self.scroll_top.is_finite() {
            self.scroll_top.clamp(0.0, max_scroll_top)
        } else {
            0.0
        };
        let raw_scroll_left = if self.scroll_left.is_finite() {
            self.scroll_left.clamp(0.0, 1_000_000.0)
        } else {
            0.0
        };
        let scroll_left = if matches!(self.word_wrap, WordWrapMode::On | WordWrapMode::Bounded) {
            0.0
        } else {
            raw_scroll_left
        };

        // 1. Editor background
        push_finite_quad(
            scene,
            Quad::new(bounds).with_background(theme.editor_background),
        );

        // 2. Gutter background and border
        let gutter_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), gutter_w, bounds.height());
        push_finite_quad(
            scene,
            Quad::new(gutter_rect)
                .with_background(theme.editor_gutter_background)
                .with_border_edges(
                    theme.editor_gutter_border,
                    Edges::new(0.0, 1.0, 0.0, 0.0),
                ),
        );

        // 3. Compute visible visual row range and minimap / clip geometry
        let start_row = (scroll_top.floor().clamp(0.0, usize::MAX as f32) as usize)
            .min(total_rows.saturating_sub(1));
        let visible_rows = ((bounds.height() / font.line_height)
            .ceil()
            .clamp(0.0, (usize::MAX - 1) as f32) as usize)
            .saturating_add(1);
        let end_row = start_row.saturating_add(visible_rows).min(total_rows);
        let segments = visual.segments_in_range(start_row, end_row, &buf);

        let mm_rect = self.minimap_rect_with_gutter(bounds, gutter_w);
        let code_min_x = bounds.min_x() + gutter_w;
        if !code_min_x.is_finite() {
            return;
        }
        let mut code_area_w = mm_rect.map_or(
            (bounds.width() - gutter_w).max(0.0),
            |mm| (mm.min_x() - code_min_x).max(0.0),
        );
        if !code_area_w.is_finite() {
            return;
        }
        if let Some(mm) = mm_rect {
            while code_min_x + code_area_w > mm.min_x() && code_area_w > 0.0 {
                code_area_w = f32::from_bits(code_area_w.to_bits() - 1);
            }
        }
        let code_clip_rect = Rect::from_xywh(
            code_min_x,
            bounds.min_y(),
            code_area_w,
            bounds.height(),
        );
        if !is_finite_rect(code_clip_rect) {
            return;
        }

        let primary_pos = self.primary_position();
        let text_area_x = bounds.min_x() + gutter_w + 8.0;

        // Warm up tokenizer cache sequentially up to the last visible buffer line so multiline state
        // is preserved across blank lines and folded ranges.
        let max_warmup_line = segments
            .last()
            .map_or(0, |s| (s.buffer_line + 1).min(line_count));
        {
            let mut highlighter_guard = self.syntax_highlighter.lock();
            if let Some(highlighter) = highlighter_guard.as_mut() {
                let last_cached = highlighter.cache_len();
                if last_cached < max_warmup_line {
                    for l in last_cached..max_warmup_line {
                        if let Some(content) = buf.get_line_content(l) {
                            highlighter.get_line_tokens(l, &content);
                        }
                    }
                }
            }
        }

        // 4. Gutter pass (breakpoints, execution arrow, line numbers, fold chevrons)
        for seg in &segments {
            if !seg.is_first_segment {
                continue;
            }
            let line_idx = seg.buffer_line;
            let line_y = bounds.min_y() + (seg.visual_row as f32 - scroll_top) * font.line_height;
            if !line_y.is_finite() || !(line_y + font.line_height).is_finite() {
                continue;
            }
            let line_1based = line_idx + 1;

            // Breakpoint dot in gutter margin
            if self.breakpoints.contains(&line_1based) {
                let dot_size = 10.0;
                let dot_x = bounds.min_x() + 4.0;
                let dot_y = line_y + (font.line_height - dot_size) / 2.0;
                push_finite_quad(
                    scene,
                    Quad::new(Rect::from_xywh(dot_x, dot_y, dot_size, dot_size))
                        .with_background(Rgba::new(0.9, 0.15, 0.15, 1.0))
                        .with_corner_radius(dot_size / 2.0),
                );
            }

            // Execution arrow in gutter
            if self.execution_line == Some(line_1based) {
                let arrow_x = bounds.min_x() + 4.0;
                push_finite_text(
                    scene,
                    GlyphRun::new(
                        "▶".to_string(),
                        Point::new(arrow_x, line_y),
                        Rect::from_xywh(arrow_x, line_y, 12.0, font.line_height),
                        Rgba::new(1.0, 0.85, 0.2, 1.0),
                        font.font_size * 0.85,
                    ),
                );
            }

            // Gutter line number (positioned left of the 16px fold indicator column)
            let line_num_str = line_1based.to_string();
            let num_w = font.text_width(&line_num_str);
            let num_x = bounds.min_x() + gutter_w - 16.0 - num_w - 2.0;
            let num_color = if line_idx == primary_pos.line {
                theme.editor_line_number_active
            } else {
                theme.editor_line_number
            };

            push_finite_text(
                scene,
                GlyphRun::new(
                    line_num_str,
                    Point::new(num_x, line_y),
                    Rect::from_xywh(num_x, line_y, num_w, font.line_height),
                    num_color,
                    font.font_size,
                ),
            );

            // Gutter fold chevron indicator in [min_x + gutter_w - 16.0, min_x + gutter_w)
            if self.folding_model.is_foldable(line_idx) {
                let fold_glyph = if self.folding_model.is_collapsed(line_idx) {
                    "▸"
                } else {
                    "▾"
                };
                let fold_x = bounds.min_x() + gutter_w - 14.0;
                push_finite_text(
                    scene,
                    GlyphRun::new(
                        fold_glyph.to_string(),
                        Point::new(fold_x, line_y),
                        Rect::from_xywh(fold_x, line_y, 12.0, font.line_height),
                        theme.editor_line_number,
                        font.font_size,
                    ),
                );
            }
        }

        // 5. Clipped code area pass (current line highlight, execution highlight, selections, text, squigglies, cursors)
        scene.push_clip(code_clip_rect);

        // 5a. Current line highlight
        if self.is_focused {
            for seg in &segments {
                if seg.buffer_line == primary_pos.line {
                    let line_y =
                        bounds.min_y() + (seg.visual_row as f32 - scroll_top) * font.line_height;
                    if line_y.is_finite() && (line_y + font.line_height).is_finite() {
                        let hl_rect = Rect::from_xywh(
                            bounds.min_x() + gutter_w,
                            line_y,
                            code_area_w,
                            font.line_height,
                        );
                        push_finite_quad(
                            scene,
                            Quad::new(hl_rect).with_background(theme.editor_current_line),
                        );
                    }
                }
            }
        }

        for seg in &segments {
            let line_idx = seg.buffer_line;
            let line_y = bounds.min_y() + (seg.visual_row as f32 - scroll_top) * font.line_height;
            if !line_y.is_finite() || !(line_y + font.line_height).is_finite() {
                continue;
            }
            let line_1based = line_idx + 1;

            // Current execution paused line highlight
            if self.execution_line == Some(line_1based) {
                let hl_rect = Rect::from_xywh(
                    bounds.min_x() + gutter_w,
                    line_y,
                    code_area_w,
                    font.line_height,
                );
                push_finite_quad(
                    scene,
                    Quad::new(hl_rect).with_background(Rgba::new(0.4, 0.4, 0.1, 0.3)),
                );
            }

            let line_content = buf.get_line_content(line_idx).unwrap_or_default();
            let seg_text = &line_content[seg.byte_start..seg.byte_end];
            let line_utf16_len: usize = line_content.chars().map(|c| c.len_utf16()).sum();

            // 5b. Selections intersecting this visual segment
            for sel in &self.cursor_state.selections {
                if !sel.is_empty() {
                    let min_pos = sel.start();
                    let max_pos = sel.end();

                    if line_idx >= min_pos.line && line_idx <= max_pos.line {
                        let line_start_col = if line_idx == min_pos.line { min_pos.column } else { 0 };
                        let line_end_col = if line_idx == max_pos.line {
                            max_pos.column
                        } else {
                            line_utf16_len
                        };

                        let seg_sel_start = line_start_col.clamp(seg.utf16_start, seg.utf16_end);
                        let seg_sel_end = line_end_col.clamp(seg.utf16_start, seg.utf16_end);

                        if seg_sel_end > seg_sel_start {
                            let start_visual = utf16_col_to_char_count(
                                seg_text,
                                seg_sel_start - seg.utf16_start,
                            );
                            let end_visual = utf16_col_to_char_count(
                                seg_text,
                                seg_sel_end - seg.utf16_start,
                            );
                            let sel_x =
                                text_area_x + (start_visual as f32 - scroll_left) * font.char_width;
                            let sel_w =
                                ((end_visual.saturating_sub(start_visual)) as f32 * font.char_width)
                                    .max(4.0);
                            let sel_rect = Rect::from_xywh(sel_x, line_y, sel_w, font.line_height);
                            push_finite_quad(
                                scene,
                                Quad::new(sel_rect).with_background(theme.editor_selection),
                            );
                        }
                    }
                }
            }

            // 5c. Text content for this visual segment
            if !seg_text.is_empty() {
                let mut rendered_tokens = false;
                let mut highlighter_guard = self.syntax_highlighter.lock();
                if let Some(highlighter) = highlighter_guard.as_mut() {
                    let tokens = highlighter.get_line_tokens(line_idx, &line_content);
                    if !tokens.is_empty() {
                        rendered_tokens = true;
                        let mut last_byte = seg.byte_start;
                        for span in tokens {
                            let span_start = span
                                .start
                                .min(line_content.len())
                                .clamp(seg.byte_start, seg.byte_end);
                            let span_end = span
                                .end
                                .min(line_content.len())
                                .clamp(seg.byte_start, seg.byte_end);

                            if span_start > last_byte
                                && line_content.is_char_boundary(last_byte)
                                && line_content.is_char_boundary(span_start)
                            {
                                let gap_text = &line_content[last_byte..span_start];
                                let gap_start_vis =
                                    line_content[seg.byte_start..last_byte].chars().count();
                                let gap_x = text_area_x
                                    + (gap_start_vis as f32 - scroll_left) * font.char_width;
                                push_clipped_code_glyph_run(
                                    scene,
                                    gap_text,
                                    gap_x,
                                    line_y,
                                    theme.editor_foreground,
                                    font,
                                    false,
                                    false,
                                    false,
                                    code_clip_rect,
                                );
                                last_byte = span_start;
                            }

                            if span_end > last_byte
                                && span_start >= last_byte
                                && line_content.is_char_boundary(span_start)
                                && line_content.is_char_boundary(span_end)
                            {
                                let span_text = &line_content[span_start..span_end];
                                let span_start_vis =
                                    line_content[seg.byte_start..span_start].chars().count();
                                let span_x = text_area_x
                                    + (span_start_vis as f32 - scroll_left) * font.char_width;
                                let span_color = span.color.unwrap_or(theme.editor_foreground);
                                push_clipped_code_glyph_run(
                                    scene,
                                    span_text,
                                    span_x,
                                    line_y,
                                    span_color,
                                    font,
                                    span.metadata.is_bold(),
                                    span.metadata.is_italic(),
                                    span.metadata.is_underline(),
                                    code_clip_rect,
                                );
                                last_byte = span_end;
                            } else {
                                last_byte = last_byte.max(span_end);
                            }
                        }

                        if last_byte < seg.byte_end && line_content.is_char_boundary(last_byte) {
                            let tail_text = &line_content[last_byte..seg.byte_end];
                            let tail_start_vis =
                                line_content[seg.byte_start..last_byte].chars().count();
                            let tail_x = text_area_x
                                + (tail_start_vis as f32 - scroll_left) * font.char_width;
                            push_clipped_code_glyph_run(
                                scene,
                                tail_text,
                                tail_x,
                                line_y,
                                theme.editor_foreground,
                                font,
                                false,
                                false,
                                false,
                                code_clip_rect,
                            );
                        }
                    }
                }
                drop(highlighter_guard);

                if !rendered_tokens {
                    let text_x = text_area_x - scroll_left * font.char_width;
                    push_clipped_code_glyph_run(
                        scene,
                        seg_text,
                        text_x,
                        line_y,
                        theme.editor_foreground,
                        font,
                        false,
                        false,
                        false,
                        code_clip_rect,
                    );
                }
            }

            // Collapsed fold summary badge (" ...") on folded header lines
            if seg.is_last_segment && self.folding_model.is_collapsed(line_idx) {
                let seg_chars = seg.char_end - seg.char_start;
                let badge_x = text_area_x + (seg_chars as f32 - scroll_left) * font.char_width;
                push_clipped_code_glyph_run(
                    scene,
                    " ...",
                    badge_x,
                    line_y,
                    theme.editor_line_number,
                    font,
                    false,
                    false,
                    false,
                    code_clip_rect,
                );
            }

            // 5d. Diagnostics squiggly underlines on this segment
            for diag in self.diagnostics_for_line(line_idx) {
                let line_start_col = if line_idx == diag.range.start.line {
                    diag.range.start.column
                } else {
                    0
                };
                let line_end_col = if line_idx == diag.range.end.line {
                    diag.range.end.column
                } else {
                    line_utf16_len
                };

                if line_end_col <= seg.utf16_start && !seg.is_first_segment {
                    continue;
                }
                if line_start_col >= seg.utf16_end && !seg.is_last_segment && seg.utf16_end > seg.utf16_start {
                    continue;
                }

                let clamped_start = line_start_col.clamp(seg.utf16_start, seg.utf16_end);
                let clamped_end = line_end_col.clamp(seg.utf16_start, seg.utf16_end);
                let start_vis =
                    utf16_col_to_char_count(seg_text, clamped_start - seg.utf16_start);
                let end_vis = utf16_col_to_char_count(seg_text, clamped_end - seg.utf16_start)
                    .max(start_vis + 1);

                let sq_x = text_area_x + (start_vis as f32 - scroll_left) * font.char_width;
                let sq_w = ((end_vis - start_vis) as f32 * font.char_width).max(4.0);
                let sq_y = line_y + font.line_height - 2.5;

                let color = match diag.severity {
                    DiagnosticSeverity::Error => theme.editor_error_foreground,
                    DiagnosticSeverity::Warning => theme.editor_warning_foreground,
                    DiagnosticSeverity::Information => theme.editor_info_foreground,
                    DiagnosticSeverity::Hint => theme.editor_hint_foreground,
                };

                if sq_x.is_finite() && sq_y.is_finite() && !sq_w.is_nan() && sq_w > 0.0 {
                    let raw_end = sq_x + sq_w;
                    let clipped_start =
                        sq_x.clamp(code_clip_rect.min_x(), code_clip_rect.max_x());
                    let clipped_end =
                        raw_end.clamp(code_clip_rect.min_x(), code_clip_rect.max_x());
                    render_squiggly_wave(scene, clipped_start, clipped_end, sq_y, color);
                }
            }

            // 5e. Cursors on this visual segment
            if self.is_focused {
                for sel in &self.cursor_state.selections {
                    if sel.active.line == line_idx {
                        let on_seg = if seg.is_last_segment {
                            sel.active.column >= seg.utf16_start
                        } else {
                            sel.active.column >= seg.utf16_start
                                && sel.active.column < seg.utf16_end
                        };
                        if on_seg {
                            let visual_col = utf16_col_to_char_count(
                                seg_text,
                                sel.active.column.saturating_sub(seg.utf16_start),
                            );
                            let cursor_x = text_area_x
                                + (visual_col as f32 - scroll_left) * font.char_width;
                            let cursor_rect =
                                Rect::from_xywh(cursor_x, line_y, 2.0, font.line_height);
                            push_finite_quad(
                                scene,
                                Quad::new(cursor_rect).with_background(theme.editor_cursor),
                            );
                        }
                    }
                }
            }
        }

        scene.pop_clip();

        // 5f. Right-edge minimap (operates in buffer-line coordinates)
        if let Some(mm_rect) = mm_rect {
            let buf_scroll_top = visual.visual_scroll_to_buffer_scroll(scroll_top);
            let layout = minimap::MinimapLayout::compute(
                mm_rect,
                line_count,
                buf_scroll_top,
                bounds.height(),
                font.line_height,
            );
            minimap::render_minimap(
                &layout,
                &buf,
                &self.syntax_highlighter,
                &self.diagnostics,
                primary_pos.line,
                self.minimap_dragging,
                theme,
                scene,
            );
        }

        // 6. Autocomplete Popover Overlay
        if self.autocomplete.is_visible {
            let buf = self.buffer.read();
            let (ac_row, ac_col) = visual.buffer_pos_to_visual(primary_pos, &buf, &self.folding_model);
            let cursor_x = text_area_x + (ac_col as f32 - scroll_left) * font.char_width;
            let cursor_y = bounds.min_y() + (ac_row as f32 - scroll_top) * font.line_height;
            drop(buf);

            if cursor_x.is_finite() && cursor_y.is_finite() {
                self.autocomplete.render(
                    Point::new(cursor_x, cursor_y),
                    bounds,
                    theme,
                    font,
                    scene,
                );
            }
        }

        // 7. Error hover tooltip overlay
        if let (Some(diag), Some(mouse_pt)) = (&self.hovered_diagnostic, self.hover_position) {
            let header_text = diag.formatted_header();
            let message_text = &diag.message;

            let header_w = font.text_width(&header_text);
            let message_w = font.text_width(message_text);
            let card_rect = Self::hover_card_bounds(diag, mouse_pt, bounds, font);
            if is_finite_rect(card_rect) {
                let card_x = card_rect.min_x();
                let card_y = card_rect.min_y();
                let card_h = card_rect.height();
                push_finite_shadow(
                    scene,
                    Shadow {
                        bounds: card_rect,
                        offset: Point::new(0.0, 4.0),
                        blur: 10.0,
                        spread: 0.0,
                        color: Rgba::new(0.0, 0.0, 0.0, 0.35),
                    },
                );

                let sev_color = match diag.severity {
                    DiagnosticSeverity::Error => theme.editor_error_foreground,
                    DiagnosticSeverity::Warning => theme.editor_warning_foreground,
                    DiagnosticSeverity::Information => theme.editor_info_foreground,
                    DiagnosticSeverity::Hint => theme.editor_hint_foreground,
                };

                push_finite_quad(
                    scene,
                    Quad::new(card_rect)
                        .with_background(theme.command_palette_bg)
                        .with_border_edges(theme.panel_border, Edges::all(1.0))
                        .with_corner_radius(4.0),
                );

                let bar_rect = Rect::from_xywh(card_x, card_y, 4.0, card_h);
                push_finite_quad(
                    scene,
                    Quad::new(bar_rect)
                        .with_background(sev_color)
                        .with_corner_radius(2.0),
                );

                let header_pt = Point::new(card_x + 10.0, card_y + 6.0);
                push_finite_text(
                    scene,
                    GlyphRun::new(
                        header_text,
                        header_pt,
                        Rect::from_xywh(header_pt.x, header_pt.y, header_w, font.line_height),
                        sev_color,
                        font.font_size * 0.9,
                    )
                    .with_style(true, false, false),
                );

                let msg_pt = Point::new(card_x + 10.0, card_y + 6.0 + font.line_height);
                push_finite_text(
                    scene,
                    GlyphRun::new(
                        message_text.clone(),
                        msg_pt,
                        Rect::from_xywh(msg_pt.x, msg_pt.y, message_w, font.line_height),
                        theme.editor_foreground,
                        font.font_size,
                    ),
                );
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let font = {
            let mut guard = self.last_render_bounds.lock();
            if let Some((_, f)) = guard.as_ref() {
                f.clone()
            } else {
                let default_font = FontMetrics::default_monospace();
                if is_finite_positive_rect(bounds)
                    && matches!(
                        event,
                        InputEvent::ScrollWheel(_)
                            | InputEvent::MouseDown(_)
                            | InputEvent::MouseMove(_)
                    )
                {
                    *guard = Some((bounds, default_font.clone()));
                }
                default_font
            }
        };
        let gutter_w = self.gutter_width(&font);

        match event {
            InputEvent::MouseDown(mouse) => {
                self.hovered_diagnostic = None;
                self.hover_position = None;
                if !is_finite_positive_rect(bounds)
                    || !mouse.position.x.is_finite()
                    || !mouse.position.y.is_finite()
                    || !bounds.contains(mouse.position)
                {
                    return EventResult::Ignored;
                }
                self.is_focused = true;
                let scroll_top = if self.scroll_top.is_finite() {
                    self.scroll_top.max(0.0)
                } else {
                    0.0
                };
                let scroll_left = if self.scroll_left.is_finite() {
                    self.scroll_left.max(0.0)
                } else {
                    0.0
                };

                // Check click in autocomplete popover or documentation panel
                if self.autocomplete.is_visible {
                    let maybe_pop = *self.autocomplete.last_bounds.lock();
                    if let Some(pop_bounds) = maybe_pop {
                        if pop_bounds.contains(mouse.position) {
                            let item_idx = self.autocomplete.scroll_offset
                                + ((mouse.position.y - pop_bounds.min_y() - 2.0) / 24.0).floor().max(0.0) as usize;
                            if item_idx < self.autocomplete.filtered_items.len() {
                                self.autocomplete.selected_index = item_idx;
                                self.commit_autocomplete();
                            }
                            return EventResult::Consumed;
                        }
                    }
                    let maybe_doc = *self.autocomplete.last_doc_bounds.lock();
                    if let Some(doc_bounds) = maybe_doc {
                        if doc_bounds.contains(mouse.position) {
                            return EventResult::Consumed;
                        }
                    }
                    self.autocomplete.hide();
                }

                if let Some(mm_rect) = self.minimap_rect_with_gutter(bounds, gutter_w) {
                    if mm_rect.contains(mouse.position) {
                        self.minimap_dragging = true;
                        let max_wrap_cols = self.effective_wrap_columns(bounds, gutter_w, &font);
                        let buf = self.buffer.read();
                        let line_count = buf.line_count();
                        let visual = VisualLayout::build(&buf, &self.folding_model, max_wrap_cols);
                        let max_st = visual.total_rows().saturating_sub(1) as f32;
                        let clamped_scroll_top = scroll_top.clamp(0.0, max_st);
                        self.scroll_top = clamped_scroll_top;
                        let buf_scroll = visual.visual_scroll_to_buffer_scroll(clamped_scroll_top);
                        let layout = minimap::MinimapLayout::compute(
                            mm_rect,
                            line_count,
                            buf_scroll,
                            bounds.height(),
                            font.line_height,
                        );
                        self.scroll_top = visual
                            .buffer_scroll_to_visual_scroll(layout.scroll_top_for_y(mouse.position.y))
                            .clamp(0.0, max_st);
                        return EventResult::Consumed;
                    }
                }

                if mouse.position.x >= bounds.min_x() + gutter_w {
                    let max_wrap_cols = self.effective_wrap_columns(bounds, gutter_w, &font);
                    let eff_scroll_left =
                        if matches!(self.word_wrap, WordWrapMode::On | WordWrapMode::Bounded) {
                            0.0
                        } else {
                            scroll_left
                        };
                    let (target, clamped_scroll_top) = {
                        let buf = self.buffer.read();
                        let visual = VisualLayout::build(&buf, &self.folding_model, max_wrap_cols);
                        let max_st = visual.total_rows().saturating_sub(1) as f32;
                        let clamped_scroll_top = scroll_top.clamp(0.0, max_st);
                        let clicked_row = ((mouse.position.y - bounds.min_y()) / font.line_height
                            + clamped_scroll_top)
                            .floor()
                            .max(0.0) as usize;
                        let text_area_x = bounds.min_x() + gutter_w + 8.0;
                        let clicked_char_col = ((mouse.position.x - text_area_x
                            + eff_scroll_left * font.char_width)
                            / font.char_width)
                            .round()
                            .max(0.0) as usize;
                        (
                            visual.visual_to_buffer_pos(clicked_row, clicked_char_col, &buf),
                            clamped_scroll_top,
                        )
                    };
                    self.scroll_top = clamped_scroll_top;

                    if mouse.modifiers.shift {
                        self.cursor_state.primary_mut().active = target;
                    } else {
                        self.cursor_state.clear_secondary_cursors();
                        self.cursor_state.set_primary(target);
                    }
                    EventResult::Consumed
                } else {
                    // Click in gutter: fold chevron on right edge, or breakpoint/line select on left margin
                    let max_wrap_cols = self.effective_wrap_columns(bounds, gutter_w, &font);
                    let (line_idx, line_len, clamped_scroll_top) = {
                        let buf = self.buffer.read();
                        let visual = VisualLayout::build(&buf, &self.folding_model, max_wrap_cols);
                        let max_st = visual.total_rows().saturating_sub(1) as f32;
                        let clamped_scroll_top = scroll_top.clamp(0.0, max_st);
                        let clicked_row = ((mouse.position.y - bounds.min_y()) / font.line_height
                            + clamped_scroll_top)
                            .floor()
                            .max(0.0) as usize;
                        let clamped_row = clicked_row.min(visual.total_rows().saturating_sub(1));
                        let idx = visual
                            .segment_at(clamped_row, &buf)
                            .map_or(0, |s| s.buffer_line);
                        let len = buf
                            .get_line_content(idx)
                            .map_or(0, |l| l.chars().map(|c| c.len_utf16()).sum());
                        (idx, len, clamped_scroll_top)
                    };
                    self.scroll_top = clamped_scroll_top;

                    if mouse.position.x >= bounds.min_x() + gutter_w - 16.0
                        && self.folding_model.is_foldable(line_idx)
                    {
                        if let Some(collapsed) = self.folding_model.toggle_fold(line_idx) {
                            self.last_gutter_action = Some(EditorGutterAction::FoldToggled {
                                line: line_idx,
                                collapsed,
                            });
                            if collapsed {
                                self.clamp_cursors_to_visible();
                            }
                            let buf = self.buffer.read();
                            let visual =
                                VisualLayout::build(&buf, &self.folding_model, max_wrap_cols);
                            let max_st = visual.total_rows().saturating_sub(1) as f32;
                            self.scroll_top = if self.scroll_top.is_finite() {
                                self.scroll_top.clamp(0.0, max_st)
                            } else {
                                0.0
                            };
                        }
                        return EventResult::Consumed;
                    }

                    let line_1based = line_idx + 1;
                    let added = self.toggle_breakpoint(line_1based);
                    self.last_gutter_action = Some(EditorGutterAction::BreakpointToggled {
                        line: line_1based,
                        added,
                    });
                    self.cursor_state.clear_secondary_cursors();
                    self.cursor_state.primary_mut().anchor = Position::new(line_idx, 0);
                    self.cursor_state.primary_mut().active = Position::new(line_idx, line_len);
                    EventResult::Consumed
                }
            }
            InputEvent::ScrollWheel(scroll) => {
                let max_wrap_cols = self.effective_wrap_columns(bounds, gutter_w, &font);
                let total_rows = {
                    let buf = self.buffer.read();
                    VisualLayout::build(&buf, &self.folding_model, max_wrap_cols).total_rows()
                };
                let max_st = total_rows.saturating_sub(1) as f32;
                let safe_scroll_top = if self.scroll_top.is_finite() {
                    self.scroll_top.clamp(0.0, max_st)
                } else {
                    0.0
                };
                self.scroll_top = safe_scroll_top;
                let delta = if scroll.delta_y.is_finite() {
                    scroll.delta_y
                } else {
                    0.0
                };
                let raw = safe_scroll_top - delta / font.line_height;
                self.scroll_top = if raw.is_nan() {
                    0.0
                } else {
                    raw.clamp(0.0, max_st)
                };
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => {
                self.hovered_diagnostic = None;
                self.hover_position = None;
                if !self.is_focused {
                    return EventResult::Ignored;
                }

                // Multi-cursor & folding direct keybindings
                if key.modifiers.ctrl
                    && !key.modifiers.alt
                    && !key.modifiers.meta
                    && !key.modifiers.shift
                    && key.key == KeyCode::Char('d')
                {
                    self.add_selection_to_next_find_match();
                    return EventResult::Consumed;
                }
                if key.modifiers.ctrl && key.modifiers.alt && !key.modifiers.meta && !key.modifiers.shift {
                    if key.key == KeyCode::ArrowUp {
                        self.insert_cursor_above();
                        return EventResult::Consumed;
                    } else if key.key == KeyCode::ArrowDown {
                        self.insert_cursor_below();
                        return EventResult::Consumed;
                    }
                }
                if key.modifiers.ctrl && key.modifiers.shift && !key.modifiers.alt && !key.modifiers.meta {
                    if matches!(key.key, KeyCode::Char('[') | KeyCode::Char('{')) {
                        self.fold_at_cursor();
                        return EventResult::Consumed;
                    } else if matches!(key.key, KeyCode::Char(']') | KeyCode::Char('}')) {
                        self.unfold_at_cursor();
                        return EventResult::Consumed;
                    }
                }

                // Check manual trigger suggest keybinding: Ctrl+Space
                if key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta && key.key == KeyCode::Space {
                    self.trigger_suggest(None);
                    return EventResult::Consumed;
                }

                // Check undo / redo / select-all key combinations
                if key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta {
                    if key.key == KeyCode::Char('z') && !key.modifiers.shift {
                        self.undo();
                        return EventResult::Consumed;
                    } else if key.key == KeyCode::Char('y')
                        || (key.key == KeyCode::Char('z') && key.modifiers.shift)
                    {
                        self.redo();
                        return EventResult::Consumed;
                    } else if key.key == KeyCode::Char('a') && !key.modifiers.shift {
                        // Select all
                        let buf = self.buffer.read();
                        let line_count = buf.line_count();
                        let last_line = line_count.saturating_sub(1);
                        let last_col = buf.get_line_content(last_line).map_or(0, |l| l.chars().map(|c| c.len_utf16()).sum());
                        self.cursor_state.clear_secondary_cursors();
                        self.cursor_state.primary_mut().anchor = Position::new(0, 0);
                        self.cursor_state.primary_mut().active = Position::new(last_line, last_col);
                        return EventResult::Consumed;
                    }
                }

                // Intercept autocomplete controls when suggestion popup is active
                if self.autocomplete.is_visible {
                    match &key.key {
                        KeyCode::ArrowUp => {
                            self.autocomplete.select_prev();
                            return EventResult::Consumed;
                        }
                        KeyCode::ArrowDown => {
                            self.autocomplete.select_next();
                            return EventResult::Consumed;
                        }
                        KeyCode::Tab | KeyCode::Enter => {
                            if self.commit_autocomplete() {
                                return EventResult::Consumed;
                            }
                        }
                        KeyCode::Escape => {
                            self.autocomplete.hide();
                            return EventResult::Consumed;
                        }
                        KeyCode::Backspace => {
                            self.delete_backwards();
                            let new_pos = self.primary_position();
                            let trigger_pos = self.autocomplete.trigger_position;
                            if new_pos.line != trigger_pos.line || new_pos.column < trigger_pos.column {
                                self.autocomplete.hide();
                            } else {
                                let buf = self.buffer.read();
                                if let Some(content) = buf.get_line_content(new_pos.line) {
                                    let mut prefix = String::new();
                                    let mut utf16_count = 0;
                                    for ch in content.chars() {
                                        let ch_utf16 = ch.len_utf16();
                                        if utf16_count >= trigger_pos.column && utf16_count < new_pos.column {
                                            prefix.push(ch);
                                        }
                                        utf16_count += ch_utf16;
                                    }
                                    self.autocomplete.update_filter(&prefix);
                                }
                            }
                            return EventResult::Consumed;
                        }
                        KeyCode::Char(c)
                            if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta =>
                        {
                            let ch = if key.modifiers.shift {
                                c.to_ascii_uppercase()
                            } else {
                                *c
                            };
                            let s = ch.to_string();
                            self.insert_text(&s);

                            if ch == '.' || ch == ':' || ch == '/' {
                                self.trigger_suggest(Some(ch));
                            } else {
                                let new_pos = self.primary_position();
                                let trigger_pos = self.autocomplete.trigger_position;
                                let buf = self.buffer.read();
                                if let Some(content) = buf.get_line_content(new_pos.line) {
                                    let mut prefix = String::new();
                                    let mut utf16_count = 0;
                                    for ch in content.chars() {
                                        let ch_utf16 = ch.len_utf16();
                                        if utf16_count >= trigger_pos.column && utf16_count < new_pos.column {
                                            prefix.push(ch);
                                        }
                                        utf16_count += ch_utf16;
                                    }
                                    self.autocomplete.update_filter(&prefix);
                                }
                            }
                            return EventResult::Consumed;
                        }
                        _ => {}
                    }
                }

                match &key.key {
                    KeyCode::ArrowLeft => {
                        self.move_cursor_left(key.modifiers.shift);
                        EventResult::Consumed
                    }
                    KeyCode::ArrowRight => {
                        self.move_cursor_right(key.modifiers.shift);
                        EventResult::Consumed
                    }
                    KeyCode::ArrowUp => {
                        self.move_cursor_up(key.modifiers.shift);
                        EventResult::Consumed
                    }
                    KeyCode::ArrowDown => {
                        self.move_cursor_down(key.modifiers.shift);
                        EventResult::Consumed
                    }
                    KeyCode::Home => {
                        self.move_cursor_home(key.modifiers.shift);
                        EventResult::Consumed
                    }
                    KeyCode::End => {
                        self.move_cursor_end(key.modifiers.shift);
                        EventResult::Consumed
                    }
                    KeyCode::Backspace => {
                        self.delete_backwards();
                        EventResult::Consumed
                    }
                    KeyCode::Delete => {
                        self.delete_forwards();
                        EventResult::Consumed
                    }
                    KeyCode::Enter => {
                        self.insert_text("\n");
                        EventResult::Consumed
                    }
                    KeyCode::Tab => {
                        self.insert_text("    ");
                        EventResult::Consumed
                    }
                    KeyCode::Space if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                        self.insert_text(" ");
                        EventResult::Consumed
                    }
                    KeyCode::Char(c) if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                        let ch = if key.modifiers.shift { c.to_ascii_uppercase() } else { *c };
                        let s = ch.to_string();
                        self.insert_text(&s);
                        if ch == '.' || ch == ':' || ch == '/' {
                            self.trigger_suggest(Some(ch));
                        }
                        EventResult::Consumed
                    }
                    KeyCode::Escape => {
                        if self.autocomplete.is_visible {
                            self.autocomplete.hide();
                            EventResult::Consumed
                        } else if self.cursor_state.has_multiple_cursors() {
                            self.cursor_state.clear_secondary_cursors();
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                    _ => {
                        if let Some(txt) = &key.text {
                            self.insert_text(txt);
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                }
            }
            InputEvent::MouseMove(mouse) => {
                if !is_finite_positive_rect(bounds)
                    || !mouse.position.x.is_finite()
                    || !mouse.position.y.is_finite()
                {
                    self.minimap_dragging = false;
                    if self.hovered_diagnostic.is_some() {
                        self.hovered_diagnostic = None;
                        self.hover_position = None;
                        return EventResult::Consumed;
                    }
                    return EventResult::Ignored;
                }
                if self.minimap_dragging {
                    if let Some(mm_rect) = self.minimap_rect_with_gutter(bounds, gutter_w) {
                        let max_wrap_cols = self.effective_wrap_columns(bounds, gutter_w, &font);
                        let buf = self.buffer.read();
                        let line_count = buf.line_count();
                        let visual = VisualLayout::build(&buf, &self.folding_model, max_wrap_cols);
                        let max_st = visual.total_rows().saturating_sub(1) as f32;
                        let clamped_st = if self.scroll_top.is_finite() {
                            self.scroll_top.clamp(0.0, max_st)
                        } else {
                            0.0
                        };
                        self.scroll_top = clamped_st;
                        let buf_scroll = visual.visual_scroll_to_buffer_scroll(clamped_st);
                        let layout = minimap::MinimapLayout::compute(
                            mm_rect,
                            line_count,
                            buf_scroll,
                            bounds.height(),
                            font.line_height,
                        );
                        self.scroll_top = visual
                            .buffer_scroll_to_visual_scroll(layout.scroll_top_for_y(mouse.position.y))
                            .clamp(0.0, max_st);
                        return EventResult::Consumed;
                    } else {
                        self.minimap_dragging = false;
                    }
                }

                if !bounds.contains(mouse.position) {
                    if self.hovered_diagnostic.is_some() {
                        self.hovered_diagnostic = None;
                        self.hover_position = None;
                        return EventResult::Consumed;
                    }
                    return EventResult::Ignored;
                }

                // If mouse rests inside the active hover card, keep tooltip visible
                if let (Some(diag), Some(hover_pt)) = (&self.hovered_diagnostic, self.hover_position) {
                    let card_rect = Self::hover_card_bounds(diag, hover_pt, bounds, &font);
                    if card_rect.contains(mouse.position) {
                        return EventResult::Consumed;
                    }
                }

                let over_minimap = self
                    .minimap_rect_with_gutter(bounds, gutter_w)
                    .is_some_and(|r| r.contains(mouse.position));
                if over_minimap {
                    if self.hovered_diagnostic.is_some() {
                        self.hovered_diagnostic = None;
                        self.hover_position = None;
                        return EventResult::Consumed;
                    }
                    return EventResult::Ignored;
                }

                if mouse.position.x >= bounds.min_x() + gutter_w {
                    let max_wrap_cols = self.effective_wrap_columns(bounds, gutter_w, &font);
                    let eff_scroll_left =
                        if matches!(self.word_wrap, WordWrapMode::On | WordWrapMode::Bounded) {
                            0.0
                        } else if self.scroll_left.is_finite() {
                            self.scroll_left.max(0.0)
                        } else {
                            0.0
                        };
                    let safe_scroll_top = if self.scroll_top.is_finite() {
                        self.scroll_top.max(0.0)
                    } else {
                        0.0
                    };
                    let buf = self.buffer.read();
                    let visual = VisualLayout::build(&buf, &self.folding_model, max_wrap_cols);
                    let max_st = visual.total_rows().saturating_sub(1) as f32;
                    let clamped_scroll_top = safe_scroll_top.clamp(0.0, max_st);
                    let row = ((mouse.position.y - bounds.min_y()) / font.line_height
                        + clamped_scroll_top)
                        .floor()
                        .max(0.0) as usize;
                    if row < visual.total_rows() {
                        let text_area_x = bounds.min_x() + gutter_w + 8.0;
                        let char_col = ((mouse.position.x - text_area_x + eff_scroll_left * font.char_width)
                            / font.char_width)
                            .floor()
                            .max(0.0) as usize;
                        let pos = visual.visual_to_buffer_pos(row, char_col, &buf);
                        drop(buf);
                        self.scroll_top = clamped_scroll_top;

                        if let Some(diag) = self.diagnostic_at_position(pos) {
                            let diag_cloned = diag.clone();
                            self.hovered_diagnostic = Some(diag_cloned);
                            self.hover_position = Some(mouse.position);
                            return EventResult::Consumed;
                        }
                    } else {
                        drop(buf);
                        self.scroll_top = clamped_scroll_top;
                    }
                }

                if self.hovered_diagnostic.is_some() {
                    self.hovered_diagnostic = None;
                    self.hover_position = None;
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::MouseUp(_) => {
                if self.minimap_dragging {
                    self.minimap_dragging = false;
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
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
    {
        scene.push_text(run);
    }
}

fn push_finite_shadow(scene: &mut Scene, shadow: Shadow) {
    if is_finite_rect(shadow.bounds)
        && shadow.offset.x.is_finite()
        && shadow.offset.y.is_finite()
    {
        scene.push_shadow(shadow);
    }
}

fn utf16_col_to_char_count(s: &str, col: usize) -> usize {
    let mut count = 0;
    let mut utf16_count = 0;
    for ch in s.chars() {
        let ch_utf16 = ch.len_utf16();
        if utf16_count + ch_utf16 > col {
            break;
        }
        utf16_count += ch_utf16;
        count += 1;
    }
    count
}

fn clamp_utf16_col(s: &str, desired_col: usize) -> usize {
    let mut target_col = 0;
    let mut utf16_count = 0;
    for ch in s.chars() {
        let ch_utf16 = ch.len_utf16();
        if utf16_count + ch_utf16 > desired_col {
            break;
        }
        utf16_count += ch_utf16;
        target_col = utf16_count;
    }
    target_col
}

fn clamp_position_to_buffer(buf: &PieceTable, pos: Position) -> Position {
    let line_count = buf.line_count().max(1);
    let line = pos.line.min(line_count - 1);
    let content = buf.get_line_content(line).unwrap_or_default();
    let col = clamp_utf16_col(&content, pos.column);
    Position::new(line, col)
}

fn clamp_selection_to_buffer(buf: &PieceTable, sel: &Selection) -> Selection {
    Selection::new(
        clamp_position_to_buffer(buf, sel.anchor),
        clamp_position_to_buffer(buf, sel.active),
    )
}


#[allow(clippy::too_many_arguments)]
fn push_clipped_code_glyph_run(
    scene: &mut Scene,
    text: &str,
    x: f32,
    y: f32,
    color: Rgba,
    font: &FontMetrics,
    bold: bool,
    italic: bool,
    underline: bool,
    clip_rect: Rect,
) {
    if text.is_empty()
        || !x.is_finite()
        || !y.is_finite()
        || !is_finite_positive_rect(clip_rect)
        || !font.line_height.is_finite()
        || font.line_height <= 0.0
        || !font.font_size.is_finite()
        || x >= clip_rect.max_x()
    {
        return;
    }
    let char_w = if font.char_width.is_finite() {
        font.char_width.max(1.0)
    } else {
        1.0
    };
    let skip_chars = if x < clip_rect.min_x() {
        ((clip_rect.min_x() - x) / char_w).floor().max(0.0) as usize
    } else {
        0
    };
    let clipped_x = x + skip_chars as f32 * char_w;
    if !clipped_x.is_finite() || !clip_rect.max_x().is_finite() || clipped_x >= clip_rect.max_x() {
        return;
    }
    let avail_w = (clip_rect.max_x() - clipped_x).max(0.0);
    if !avail_w.is_finite() || avail_w <= 0.0 {
        return;
    }
    let max_chars = ((avail_w / char_w).ceil() as usize).max(1);
    let clipped_text: String = text.chars().skip(skip_chars).take(max_chars).collect();
    if clipped_text.is_empty() {
        return;
    }
    let mut clipped_w = (clipped_text.chars().count() as f32 * char_w).min(avail_w);
    while clipped_x + clipped_w > clip_rect.max_x() && clipped_w > 0.0 {
        clipped_w = f32::from_bits(clipped_w.to_bits() - 1);
    }
    let bounds = Rect::from_xywh(clipped_x, y, clipped_w, font.line_height);
    if !is_finite_rect(bounds) {
        return;
    }
    let mut run = GlyphRun::new(
        clipped_text,
        Point::new(clipped_x, y),
        bounds,
        color,
        font.font_size,
    );
    if bold || italic || underline {
        run = run.with_style(bold, italic, underline);
    }
    scene.push_text(run);
}

fn render_squiggly_wave(scene: &mut Scene, start_x: f32, end_x: f32, y: f32, color: Rgba) {
    if !start_x.is_finite()
        || !end_x.is_finite()
        || !y.is_finite()
        || end_x <= start_x
        || !(y - 1.2).is_finite()
        || !(y + 2.0).is_finite()
    {
        return;
    }
    const MAX_SQUIGGLY_SEGMENTS: usize = 4096;
    let wave_step: f32 = 3.0;
    let mut cur_x = start_x;
    let mut up = false;
    let mut segments = 0usize;
    while cur_x < end_x && segments < MAX_SQUIGGLY_SEGMENTS {
        let seg_w = wave_step.min(end_x - cur_x);
        if !seg_w.is_finite() || seg_w <= 0.0 {
            break;
        }
        let seg_y = if up { y - 1.2 } else { y + 0.8 };
        let quad_rect = Rect::from_xywh(cur_x, seg_y, seg_w, 1.2);
        if !is_finite_rect(quad_rect) {
            break;
        }
        scene.push_quad(Quad::new(quad_rect).with_background(color));
        let next_x = cur_x + seg_w;
        if !next_x.is_finite() || next_x <= cur_x {
            break;
        }
        cur_x = next_x;
        up = !up;
        segments += 1;
    }
}

pub fn get_builtin_completions(
    lang: &str,
    trigger_char: Option<char>,
    text: &str,
    current_prefix: Option<&str>,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();

    if trigger_char == Some('.') {
        match lang {
            "rust" => {
                items.push(
                    CompletionItem::new("clone")
                        .with_kind(CompletionItemKind::Method)
                        .with_detail("fn(&self) -> Self")
                        .with_doc("Returns a copy of the value.")
                        .with_insert_text("clone()"),
                );
                items.push(
                    CompletionItem::new("to_string")
                        .with_kind(CompletionItemKind::Method)
                        .with_detail("fn(&self) -> String")
                        .with_doc("Converts the given value to a String.")
                        .with_insert_text("to_string()"),
                );
                items.push(
                    CompletionItem::new("len")
                        .with_kind(CompletionItemKind::Method)
                        .with_detail("fn(&self) -> usize")
                        .with_doc("Returns the number of elements in the container.")
                        .with_insert_text("len()"),
                );
                items.push(
                    CompletionItem::new("is_empty")
                        .with_kind(CompletionItemKind::Method)
                        .with_detail("fn(&self) -> bool")
                        .with_doc("Returns true if the container has a length of 0.")
                        .with_insert_text("is_empty()"),
                );
                items.push(
                    CompletionItem::new("iter")
                        .with_kind(CompletionItemKind::Method)
                        .with_detail("fn(&self) -> Iter<'_, T>")
                        .with_doc("Returns an iterator over the slice.")
                        .with_insert_text("iter()"),
                );
                items.push(
                    CompletionItem::new("map")
                        .with_kind(CompletionItemKind::Method)
                        .with_detail("fn<B, F>(self, f: F) -> Map<Self, F>")
                        .with_doc("Takes a closure and creates an iterator which calls that closure on each element.")
                        .with_insert_text("map(|$1| $0)"),
                );
                items.push(
                    CompletionItem::new("unwrap")
                        .with_kind(CompletionItemKind::Method)
                        .with_detail("fn(self) -> T")
                        .with_doc("Returns the contained Some or Ok value, or panics.")
                        .with_insert_text("unwrap()"),
                );
                items.push(
                    CompletionItem::new("push")
                        .with_kind(CompletionItemKind::Method)
                        .with_detail("fn(&mut self, value: T)")
                        .with_doc("Appends an element to the back of a collection.")
                        .with_insert_text("push($0)"),
                );
                items.push(
                    CompletionItem::new("contains")
                        .with_kind(CompletionItemKind::Method)
                        .with_detail("fn(&self, x: &T) -> bool")
                        .with_insert_text("contains(&$0)"),
                );
            }
            "javascript" | "typescript" => {
                for (name, detail, doc) in [
                    ("length", "number", "The length of the array or string."),
                    (
                        "map",
                        "(callback: (value: T) => U) => U[]",
                        "Calls a defined callback function on each element of an array.",
                    ),
                    (
                        "filter",
                        "(predicate: (value: T) => boolean) => T[]",
                        "Returns the elements of an array that meet the condition.",
                    ),
                    (
                        "forEach",
                        "(callback: (value: T) => void) => void",
                        "Performs the specified action for each element in an array.",
                    ),
                    (
                        "push",
                        "(...items: T[]) => number",
                        "Appends new elements to the end of an array.",
                    ),
                    (
                        "pop",
                        "() => T | undefined",
                        "Removes the last element from an array and returns it.",
                    ),
                    (
                        "indexOf",
                        "(searchElement: T) => number",
                        "Returns the index of the first occurrence of a value in an array.",
                    ),
                    (
                        "includes",
                        "(searchElement: T) => boolean",
                        "Determines whether an array includes a certain element.",
                    ),
                    (
                        "toString",
                        "() => string",
                        "Returns a string representation of an object.",
                    ),
                ] {
                    items.push(
                        CompletionItem::new(name)
                            .with_kind(CompletionItemKind::Method)
                            .with_detail(detail)
                            .with_doc(doc)
                            .with_insert_text(if name == "length" {
                                "length".to_string()
                            } else {
                                format!("{}()", name)
                            }),
                    );
                }
            }
            "python" => {
                for (name, detail, doc) in [
                    (
                        "append",
                        "def append(self, object: _T) -> None",
                        "Append object to the end of the list.",
                    ),
                    (
                        "extend",
                        "def extend(self, iterable: Iterable[_T]) -> None",
                        "Extend list by appending elements from the iterable.",
                    ),
                    (
                        "get",
                        "def get(self, key: _KT, default: _VT = ...) -> _VT",
                        "Return the value for key if key is in the dictionary.",
                    ),
                    (
                        "keys",
                        "def keys(self) -> dict_keys",
                        "Return a new view of the dictionary's keys.",
                    ),
                    (
                        "values",
                        "def values(self) -> dict_values",
                        "Return a new view of the dictionary's values.",
                    ),
                    (
                        "items",
                        "def items(self) -> dict_items",
                        "Return a new view of the dictionary's items.",
                    ),
                    (
                        "split",
                        "def split(self, sep: str | None = None) -> list[str]",
                        "Return a list of the words in the string.",
                    ),
                    (
                        "strip",
                        "def strip(self, chars: str | None = None) -> str",
                        "Return a copy of the string with whitespace removed.",
                    ),
                ] {
                    items.push(
                        CompletionItem::new(name)
                            .with_kind(CompletionItemKind::Method)
                            .with_detail(detail)
                            .with_doc(doc)
                            .with_insert_text(format!("{}()", name)),
                    );
                }
            }
            _ => {}
        }
    } else if trigger_char == Some(':') {
        if lang == "rust" {
            items.push(
                CompletionItem::new("new")
                    .with_kind(CompletionItemKind::Function)
                    .with_detail("fn() -> Self")
                    .with_doc("Constructs a new instance.")
                    .with_insert_text("new()"),
            );
            items.push(
                CompletionItem::new("default")
                    .with_kind(CompletionItemKind::Function)
                    .with_detail("fn() -> Self")
                    .with_doc("Returns the default value for the type.")
                    .with_insert_text("default()"),
            );
            items.push(
                CompletionItem::new("from")
                    .with_kind(CompletionItemKind::Function)
                    .with_detail("fn(t: T) -> Self")
                    .with_doc("Converts to this type from the input type.")
                    .with_insert_text("from($0)"),
            );
        }
    } else {
        match lang {
            "rust" => {
                let keywords = [
                    "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "match", "if",
                    "else", "return", "use", "mod", "crate", "self", "Self", "true", "false",
                    "async", "await", "where", "for", "while", "loop", "break", "continue", "type",
                ];
                for kw in keywords {
                    items.push(CompletionItem::new(kw).with_kind(CompletionItemKind::Keyword));
                }

                let types = [
                    "String", "Vec", "Option", "Result", "Some", "None", "Ok", "Err", "HashMap",
                    "Arc", "Mutex", "RwLock", "Box", "u8", "u32", "u64", "i32", "i64", "f32",
                    "f64", "bool", "usize",
                ];
                for t in types {
                    items.push(CompletionItem::new(t).with_kind(CompletionItemKind::Struct));
                }

                items.push(
                    CompletionItem::new("println!")
                        .with_kind(CompletionItemKind::Snippet)
                        .with_detail("macro")
                        .with_doc("Prints to the standard output, with a newline.")
                        .with_insert_text("println!(\"{}\", $0);"),
                );
                items.push(
                    CompletionItem::new("eprintln!")
                        .with_kind(CompletionItemKind::Snippet)
                        .with_detail("macro")
                        .with_doc("Prints to the standard error, with a newline.")
                        .with_insert_text("eprintln!(\"{}\", $0);"),
                );
                items.push(
                    CompletionItem::new("fn")
                        .with_kind(CompletionItemKind::Snippet)
                        .with_detail("Function template")
                        .with_doc("Define a new function.")
                        .with_insert_text("fn ${1:name}() {\n    $0\n}"),
                );
                items.push(
                    CompletionItem::new("struct")
                        .with_kind(CompletionItemKind::Snippet)
                        .with_detail("Struct template")
                        .with_doc("Define a new struct.")
                        .with_insert_text("struct ${1:Name} {\n    $0\n}"),
                );
            }
            "javascript" | "typescript" => {
                let keywords = [
                    "function", "const", "let", "var", "if", "else", "return", "import", "export",
                    "from", "class", "async", "await", "try", "catch", "throw", "new", "this",
                    "typeof", "instanceof", "null", "undefined", "true", "false",
                ];
                for kw in keywords {
                    items.push(CompletionItem::new(kw).with_kind(CompletionItemKind::Keyword));
                }
                items.push(
                    CompletionItem::new("console.log")
                        .with_kind(CompletionItemKind::Snippet)
                        .with_detail("console.log(data)")
                        .with_doc("Prints output to the console.")
                        .with_insert_text("console.log($0);"),
                );
            }
            "python" => {
                let keywords = [
                    "def", "class", "import", "from", "as", "if", "elif", "else", "for", "while",
                    "return", "yield", "try", "except", "finally", "with", "raise", "pass", "True",
                    "False", "None", "lambda", "async", "await",
                ];
                for kw in keywords {
                    items.push(CompletionItem::new(kw).with_kind(CompletionItemKind::Keyword));
                }
                items.push(
                    CompletionItem::new("print")
                        .with_kind(CompletionItemKind::Snippet)
                        .with_detail("print(*values)")
                        .with_doc("Prints values to a stream, or to sys.stdout by default.")
                        .with_insert_text("print($0)"),
                );
            }
            _ => {}
        }

        // Scan identifiers from document text
        let mut seen = std::collections::HashSet::new();
        for word in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if word.len() >= 3
                && !word.chars().next().unwrap().is_ascii_digit()
                && Some(word) != current_prefix
                && seen.insert(word)
                && !items.iter().any(|i| i.label == word)
            {
                items.push(CompletionItem::new(word).with_kind(CompletionItemKind::Variable));
            }
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::Range;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::MouseEvent;

    #[test]
    fn test_editor_view_render_and_input() {
        let content = "fn main() {\n    println!(\"RustCode\");\n}\n";
        let mut view = EditorView::from_str(content);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        // Verify scene contains quads (background, gutter, highlight, cursor) and text runs
        assert!(!scene.quads().is_empty());
        assert!(!scene.glyph_runs().is_empty());
        assert!(scene.contains_text("println!(\"RustCode\");"));

        // Type a character
        let key_ev = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Char('x'),
            Modifiers::none(),
        ));
        view.handle_event(&key_ev, bounds);
        assert_eq!(view.primary_position(), Position::new(0, 1));
        assert!(view.text().starts_with("xfn main()"));

        // Backspace
        let bs_ev = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Backspace,
            Modifiers::none(),
        ));
        view.handle_event(&bs_ev, bounds);
        assert_eq!(view.primary_position(), Position::new(0, 0));
        assert!(view.text().starts_with("fn main()"));

        // Mouse click on line 1
        let click_ev = InputEvent::MouseDown(
            MouseEvent::new(
                Point::new(bounds.min_x() + view.gutter_width(&font) + 20.0, bounds.min_y() + font.line_height + 5.0),
                rustcode_platform::render::MouseButton::Left,
            ),
        );
        view.handle_event(&click_ev, bounds);
        assert_eq!(view.primary_position().line, 1);
    }

    #[test]
    fn test_editor_view_multibyte_utf8_editing_and_undo() {
        let content = "🦀hello";
        let mut view = EditorView::from_str(content);
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);

        // Move cursor right past '🦀' (2 UTF-16 code units, 4 UTF-8 bytes)
        view.move_cursor_right(false);
        assert_eq!(view.primary_position(), Position::new(0, 2));

        // Backspace deletes '🦀' cleanly
        let bs_ev = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Backspace,
            Modifiers::none(),
        ));
        view.handle_event(&bs_ev, bounds);
        assert_eq!(view.text(), "hello");
        assert_eq!(view.primary_position(), Position::new(0, 0));

        // Undo restores '🦀hello' with cursor at (0, 2)
        assert!(view.undo());
        assert_eq!(view.text(), "🦀hello");
        assert_eq!(view.primary_position(), Position::new(0, 2));

        // Move to start, then delete forwards
        view.move_cursor_home(false);
        assert_eq!(view.primary_position(), Position::new(0, 0));
        let del_ev = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Delete,
            Modifiers::none(),
        ));
        view.handle_event(&del_ev, bounds);
        assert_eq!(view.text(), "hello");

        // Undo delete forwards restores '🦀hello'
        assert!(view.undo());
        assert_eq!(view.text(), "🦀hello");
    }

    #[test]
    fn test_editor_view_selection_replacement_and_shift_navigation() {
        let content = "hello world";
        let mut view = EditorView::from_str(content);
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);

        // Shift+Right 5 times to select "hello"
        for _ in 0..5 {
            view.handle_event(
                &InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
                    KeyCode::ArrowRight,
                    Modifiers { ctrl: false, shift: true, alt: false, meta: false },
                )),
                bounds,
            );
        }

        assert_eq!(view.cursor_state.primary().anchor, Position::new(0, 0));
        assert_eq!(view.cursor_state.primary().active, Position::new(0, 5));
        assert!(!view.cursor_state.primary().is_empty());

        // Type 'H' replaces "hello" with "H"
        view.handle_event(
            &InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
                KeyCode::Char('h'),
                Modifiers { ctrl: false, shift: true, alt: false, meta: false },
            )),
            bounds,
        );
        assert_eq!(view.text(), "H world");
        assert_eq!(view.primary_position(), Position::new(0, 1));

        // Ctrl+A selects all
        view.handle_event(
            &InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
                KeyCode::Char('a'),
                Modifiers { ctrl: true, shift: false, alt: false, meta: false },
            )),
            bounds,
        );
        assert_eq!(view.cursor_state.primary().anchor, Position::new(0, 0));
        assert_eq!(view.cursor_state.primary().active, Position::new(0, 7));

        // Backspace deletes entire selection
        view.handle_event(
            &InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
                KeyCode::Backspace,
                Modifiers::none(),
            )),
            bounds,
        );
        assert_eq!(view.text(), "");
        assert_eq!(view.primary_position(), Position::new(0, 0));

        // Undo restores text
        assert!(view.undo());
        assert_eq!(view.text(), "H world");
    }

    #[test]
    fn test_editor_view_gutter_click_line_selection() {
        let content = "first line\nsecond line\nthird line\n";
        let mut view = EditorView::from_str(content);
        let font = FontMetrics::default_monospace();
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);

        // Click in gutter at line 1 (second line)
        let click_y = bounds.min_y() + font.line_height + 5.0;
        let click_ev = InputEvent::MouseDown(MouseEvent::new(
            Point::new(bounds.min_x() + 10.0, click_y),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = view.handle_event(&click_ev, bounds);
        assert_eq!(res, EventResult::Consumed);

        // Entire line 1 selected
        assert_eq!(view.cursor_state.primary().anchor, Position::new(1, 0));
        assert_eq!(view.cursor_state.primary().active, Position::new(1, 11)); // "second line".len() == 11
    }

    #[test]
    fn test_editor_view_syntax_highlighted_rendering() {
        use rustcode_platform::render::Rgba;
        use rustcode_platform::theme::ThemeService;

        let content = "pub fn main() {\n    let msg = \"RustCode\";\n}\n";
        let mut view = EditorView::from_str(content);
        let registry = GrammarRegistry::new();
        let theme = ThemeService::default_dark_theme();

        view.set_language("rust", theme.clone(), &registry);

        let font = FontMetrics::default_monospace();
        let theme_styles = WorkbenchThemeStyles::from_theme(&theme);
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let mut scene = Scene::new();
        view.render(bounds, &theme_styles, &font, &mut scene);

        // Verify keyword color (#C586C0) and string color (#CE9178) exist in the scene glyph runs
        let keyword_color = Rgba::from_hex("#C586C0").unwrap();
        let string_color = Rgba::from_hex("#CE9178").unwrap();

        let has_keyword = scene.glyph_runs().iter().any(|r| r.color == keyword_color);
        let has_string = scene.glyph_runs().iter().any(|r| r.color == string_color);

        assert!(has_keyword, "Scene should contain keyword color #C586C0");
        assert!(has_string, "Scene should contain string color #CE9178");
    }

    #[test]
    fn test_editor_view_document_dirty_and_save() {
        let tmp_dir = std::env::temp_dir().join(format!("rustcode_view_doc_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let file_path = tmp_dir.join("main.rs");

        let fs = FileSystemService::new();
        let doc = Arc::new(RwLock::new(TextDocument::new(
            "fn hello() {}",
            Some(file_path.clone()),
            Some("rust"),
        )));

        let mut view = EditorView::new(doc.read().buffer.clone()).with_document(doc.clone());
        assert!(!view.is_dirty());

        // Type text
        view.insert_text(" ");
        assert!(view.is_dirty());

        // Save
        let meta = view.save_sync(&fs).unwrap();
        assert!(!view.is_dirty());
        assert_eq!(meta.path, file_path);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_editor_view_unicode_rendering_and_scrolled_multiline_comment() {
        use rustcode_platform::render::Rgba;
        use rustcode_platform::theme::ThemeService;

        let content = "/* start block\nmiddle line without end\nend block */\nlet café_π = \"☕\";\n";
        let mut view = EditorView::from_str(content);
        let registry = GrammarRegistry::new();
        let theme = ThemeService::default_dark_theme();
        view.set_language("rust", theme.clone(), &registry);

        let font = FontMetrics::default_monospace();
        let theme_styles = WorkbenchThemeStyles::from_theme(&theme);
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);

        // Test normal rendering
        let mut scene = Scene::new();
        view.render(bounds, &theme_styles, &font, &mut scene);

        let comment_color = Rgba::from_hex("#6A9955").unwrap();
        let keyword_color = Rgba::from_hex("#C586C0").unwrap();

        assert!(scene.glyph_runs().iter().any(|r| r.color == comment_color));
        assert!(scene.glyph_runs().iter().any(|r| r.color == keyword_color));

        // Test scrolling down: starting from line 1 (middle line of comment)
        view.scroll_top = 1.0;
        let mut scene2 = Scene::new();
        view.render(bounds, &theme_styles, &font, &mut scene2);
        assert!(scene2.glyph_runs().iter().any(|r| r.color == comment_color));
    }

    #[test]
    fn test_editor_view_autocomplete_trigger_characters_and_commit() {
        let content = "let x = foo;";
        let mut view = EditorView::from_str(content);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);

        // Move cursor right after "foo" (column 11)
        view.set_primary_position(Position::new(0, 11));

        // Type trigger character '.'
        let dot_key = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Char('.'),
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        let res = view.handle_event(&dot_key, bounds);
        assert_eq!(res, EventResult::Consumed);

        // Autocomplete popover should now be visible with method suggestions
        assert!(view.autocomplete.is_visible);
        assert!(view.autocomplete.filtered_items.iter().any(|(i, _)| i.label == "clone"));

        // Render scene with autocomplete popup active
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("clone"));
        assert!(scene.contains_text("fn(&self) -> Self"));

        // Type 'c' to filter candidates
        let c_key = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Char('c'),
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        view.handle_event(&c_key, bounds);
        assert_eq!(view.autocomplete.selected_item().unwrap().label, "clone");

        // Commit with Tab
        let tab_key = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Tab,
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        let res = view.handle_event(&tab_key, bounds);
        assert_eq!(res, EventResult::Consumed);

        // Popover should be dismissed and buffer text updated
        assert!(!view.autocomplete.is_visible);
        assert_eq!(view.text(), "let x = foo.clone();");
        assert_eq!(view.primary_position(), Position::new(0, 19));
    }

    #[test]
    fn test_editor_view_autocomplete_ctrl_space_and_snippet() {
        let mut view = EditorView::from_str("");
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);

        // Press Ctrl+Space to trigger suggest
        let ctrl_space = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Space,
            rustcode_platform::keybindings::Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        let res = view.handle_event(&ctrl_space, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert!(view.autocomplete.is_visible);

        // Arrow navigation
        let arrow_down = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::ArrowDown,
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        view.handle_event(&arrow_down, bounds);

        let arrow_up = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::ArrowUp,
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        view.handle_event(&arrow_up, bounds);

        // Type "pri" to filter to println! snippet
        for ch in "pri".chars() {
            view.handle_event(
                &InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
                    KeyCode::Char(ch),
                    rustcode_platform::keybindings::Modifiers::none(),
                )),
                bounds,
            );
        }

        assert_eq!(view.autocomplete.selected_item().unwrap().label, "println!");

        // Commit with Enter
        let enter_key = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Enter,
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        view.handle_event(&enter_key, bounds);

        assert!(!view.autocomplete.is_visible);
        assert_eq!(view.text(), "println!(\"{}\", );");
        // Cursor placed at tabstop $0 inside quotes
        assert_eq!(view.primary_position(), Position::new(0, 15));
    }

    #[test]
    fn test_editor_view_autocomplete_custom_provider_and_dismiss() {
        let mut view = EditorView::from_str("");
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);

        // Attach custom completion provider
        view = view.with_completion_provider(Arc::new(|_lang, _pos, _trigger| {
            vec![
                CompletionItem::new("custom_feature")
                    .with_kind(CompletionItemKind::Function)
                    .with_detail("fn custom_feature() -> bool")
                    .with_doc("A custom extension-provided item.")
                    .with_insert_text("custom_feature()"),
            ]
        }));

        // Trigger with Ctrl+Space
        view.trigger_suggest(None);
        assert!(view.autocomplete.is_visible);
        assert!(view
            .autocomplete
            .filtered_items
            .iter()
            .any(|(i, _)| i.label == "custom_feature"));

        // Type "custom" to filter
        for ch in "custom".chars() {
            view.handle_event(
                &InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
                    KeyCode::Char(ch),
                    rustcode_platform::keybindings::Modifiers::none(),
                )),
                bounds,
            );
        }
        assert_eq!(view.autocomplete.selected_item().unwrap().label, "custom_feature");

        // Escape dismisses
        let esc_key = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Escape,
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        let res = view.handle_event(&esc_key, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert!(!view.autocomplete.is_visible);
        assert_eq!(view.text(), "custom");
    }

    #[test]
    fn test_editor_view_autocomplete_ctrl_space_with_existing_prefix_no_duplication() {
        let mut view = EditorView::from_str("let x = pri");
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);

        // Move cursor to end of "pri" (column 11)
        view.set_primary_position(Position::new(0, 11));

        // Press Ctrl+Space
        let ctrl_space = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Space,
            rustcode_platform::keybindings::Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        let res = view.handle_event(&ctrl_space, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert!(view.autocomplete.is_visible);

        // Verify println! is the top match
        assert_eq!(view.autocomplete.selected_item().unwrap().label, "println!");

        // Press Tab to commit
        let tab_key = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Tab,
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        let res = view.handle_event(&tab_key, bounds);
        assert_eq!(res, EventResult::Consumed);

        assert!(!view.autocomplete.is_visible);
        // Prefix "pri" must have been replaced, NOT duplicated as "priprintln!"
        assert_eq!(view.text(), "let x = println!(\"{}\", );");
        assert_eq!(view.primary_position(), Position::new(0, 23));
    }

    #[test]
    fn test_editor_view_autocomplete_no_match_dismisses_and_restores_navigation() {
        let mut view = EditorView::from_str("line 1\nline 2\n");
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        view.set_primary_position(Position::new(0, 0));

        // Trigger autocomplete
        view.trigger_suggest(None);
        assert!(view.autocomplete.is_visible);

        // Type characters that match nothing
        for ch in "xyz123".chars() {
            view.handle_event(
                &InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
                    KeyCode::Char(ch),
                    rustcode_platform::keybindings::Modifiers::none(),
                )),
                bounds,
            );
        }

        // Popup should now be dismissed
        assert!(!view.autocomplete.is_visible);

        // Arrow Down should now move cursor normally instead of being swallowed
        let arrow_down = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::ArrowDown,
            rustcode_platform::keybindings::Modifiers::none(),
        ));
        view.handle_event(&arrow_down, bounds);
        assert_eq!(view.primary_position().line, 1);
    }

    #[test]
    fn test_editor_view_diagnostics_squiggly_underlines_rendering() {
        let mut view = EditorView::from_str("let x: i32 = \"bad\";\nlet warning_val = 123;\nlet info_note = 456;");
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        // Add 3 diagnostics: Error, Warning, Info
        view.set_diagnostics(vec![
            EditorDiagnostic::error(
                Range::new(Position::new(0, 13), Position::new(0, 18)),
                "mismatched types: expected `i32`, found `&str`",
            )
            .with_source("rust-analyzer")
            .with_code("E0308"),
            EditorDiagnostic::warning(
                Range::new(Position::new(1, 4), Position::new(1, 15)),
                "unused variable: `warning_val`",
            )
            .with_source("rustc")
            .with_code("unused_variables"),
            EditorDiagnostic::info(
                Range::new(Position::new(2, 4), Position::new(2, 13)),
                "consider making this constant",
            ),
        ]);

        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        // Verify quads exist with the diagnostic colors
        let quads = scene.quads();
        let has_error_wave = quads.iter().any(|q| q.background == Some(theme.editor_error_foreground));
        let has_warning_wave = quads.iter().any(|q| q.background == Some(theme.editor_warning_foreground));
        let has_info_wave = quads.iter().any(|q| q.background == Some(theme.editor_info_foreground));

        assert!(has_error_wave, "Should have rendered red squiggly wave for error");
        assert!(has_warning_wave, "Should have rendered yellow squiggly wave for warning");
        assert!(has_info_wave, "Should have rendered blue squiggly wave for info");
    }

    #[test]
    fn test_editor_view_diagnostics_hover_tooltip_and_dismiss() {
        let mut view = EditorView::from_str("fn main() {\n    let result = unknown_fn();\n}");
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        view.set_diagnostics(vec![
            EditorDiagnostic::error(
                Range::new(Position::new(1, 17), Position::new(1, 27)),
                "cannot find function `unknown_fn` in this scope",
            )
            .with_source("rust-analyzer")
            .with_code("E0425"),
        ]);

        // Calculate coordinate of `unknown_fn` on line 1, col 20
        let gutter_w = view.gutter_width(&font);
        let text_area_x = bounds.min_x() + gutter_w + 8.0;
        let target_x = text_area_x + 20.0 * font.char_width;
        let target_y = bounds.min_y() + 1.5 * font.line_height;

        // Hover over the diagnostic span
        let move_over = InputEvent::MouseMove(rustcode_platform::render::MouseMoveEvent {
            position: Point::new(target_x, target_y),
        });
        let res = view.handle_event(&move_over, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert!(view.hovered_diagnostic.is_some());
        let diag = view.hovered_diagnostic.as_ref().unwrap();
        assert_eq!(diag.message, "cannot find function `unknown_fn` in this scope");
        assert_eq!(diag.source.as_deref(), Some("rust-analyzer"));
        assert_eq!(diag.code.as_deref(), Some("E0425"));

        // Render scene with hover active
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("rust-analyzer (E0425)"));
        assert!(scene.contains_text("cannot find function `unknown_fn` in this scope"));

        // Move mouse away to empty area
        let move_away = InputEvent::MouseMove(rustcode_platform::render::MouseMoveEvent {
            position: Point::new(bounds.min_x() + 10.0, bounds.min_y() + 500.0),
        });
        view.handle_event(&move_away, bounds);
        assert!(view.hovered_diagnostic.is_none());

        // Hover again and then dismiss via MouseDown
        view.handle_event(&move_over, bounds);
        assert!(view.hovered_diagnostic.is_some());

        let mouse_down = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(target_x, target_y),
            rustcode_platform::render::MouseButton::Left,
        ));
        view.handle_event(&mouse_down, bounds);
        assert!(view.hovered_diagnostic.is_none());
    }

    #[test]
    fn test_editor_view_diagnostics_utf8_char_boundaries_no_panic() {
        // Line with 4-byte UTF-8 emoji at byte indices 6..10
        let mut view = EditorView::from_str("hello 🦀 world;\nlet val = 100;");
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        // Intentionally place diagnostic start/end in the middle of the 4-byte 🦀 character (byte 7)
        view.set_diagnostics(vec![
            EditorDiagnostic::error(
                Range::new(Position::new(0, 7), Position::new(0, 9)),
                "unsupported emoji identifier",
            ),
        ]);

        let mut scene = Scene::new();
        // Rendering must not panic with slice indexing error
        view.render(bounds, &theme, &font, &mut scene);
        let quads = scene.quads();
        let has_error_wave = quads.iter().any(|q| q.background == Some(theme.editor_error_foreground));
        assert!(has_error_wave);
    }

    #[test]
    fn test_editor_view_diagnostic_severity_precedence() {
        let mut view = EditorView::from_str("fn test() { let x = 1; }");
        // Warning first, then Error on same span
        view.set_diagnostics(vec![
            EditorDiagnostic::warning(
                Range::new(Position::new(0, 16), Position::new(0, 17)),
                "unused variable",
            ),
            EditorDiagnostic::error(
                Range::new(Position::new(0, 16), Position::new(0, 17)),
                "type mismatch",
            ),
        ]);

        let diag = view.diagnostic_at_position(Position::new(0, 16)).unwrap();
        assert_eq!(diag.severity, DiagnosticSeverity::Error);
        assert_eq!(diag.message, "type mismatch");
    }

    #[test]
    fn test_editor_view_hover_card_mouse_navigation() {
        let mut view = EditorView::from_str("fn test() { let x = 1; }");
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let font = FontMetrics::default_monospace();

        view.set_diagnostics(vec![
            EditorDiagnostic::error(
                Range::new(Position::new(0, 16), Position::new(0, 20)),
                "severe error",
            ),
        ]);

        let gutter_w = view.gutter_width(&font);
        let text_area_x = bounds.min_x() + gutter_w + 8.0;
        let target_x = text_area_x + 18.0 * font.char_width;
        let target_y = bounds.min_y() + 0.5 * font.line_height;

        let move_over = InputEvent::MouseMove(rustcode_platform::render::MouseMoveEvent {
            position: Point::new(target_x, target_y),
        });
        view.handle_event(&move_over, bounds);
        assert!(view.hovered_diagnostic.is_some());

        // Calculate card bounds
        let card = EditorView::hover_card_bounds(
            view.hovered_diagnostic.as_ref().unwrap(),
            Point::new(target_x, target_y),
            bounds,
            &font,
        );

        // Move mouse into the card itself (e.g. card center)
        let inside_card = Point::new(card.min_x() + card.width() * 0.5, card.min_y() + card.height() * 0.5);
        let move_inside = InputEvent::MouseMove(rustcode_platform::render::MouseMoveEvent {
            position: inside_card,
        });
        let res = view.handle_event(&move_inside, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert!(view.hovered_diagnostic.is_some(), "Hover card must remain open when mouse moves inside it");
    }

    #[test]
    fn test_editor_view_gutter_breakpoint_toggle_and_render() {
        let mut view = EditorView::from_str("line 1\nline 2\nline 3");
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        assert!(!view.has_breakpoint(2));

        // Click in gutter on line 2 (y around line_height * 1.5)
        let gutter_click_x = bounds.min_x() + 10.0;
        let gutter_click_y = bounds.min_y() + font.line_height * 1.5;
        let click = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(gutter_click_x, gutter_click_y),
            rustcode_platform::render::MouseButton::Left,
        ));

        let res = view.handle_event(&click, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert!(view.has_breakpoint(2));
        assert_eq!(
            view.last_gutter_action,
            Some(EditorGutterAction::BreakpointToggled {
                line: 2,
                added: true
            })
        );

        // Render and verify red dot quad is pushed
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        let red_quad = scene.quads().iter().any(|q| {
            if let Some(bg) = q.background {
                (bg.r - 0.9).abs() < 0.01 && (bg.g - 0.15).abs() < 0.01
            } else {
                false
            }
        });
        assert!(red_quad, "Breakpoint red dot must be rendered in the scene");

        // Second click in gutter on line 2 toggles it off
        let res2 = view.handle_event(&click, bounds);
        assert_eq!(res2, EventResult::Consumed);
        assert!(!view.has_breakpoint(2));
        assert_eq!(
            view.last_gutter_action,
            Some(EditorGutterAction::BreakpointToggled {
                line: 2,
                added: false
            })
        );
    }

    #[test]
    fn test_diff_editor_view_render_and_mode_toggle() {
        use crate::diff::view::DiffEditorView;
        use crate::diff::myers::DiffViewMode;

        let original = "fn test() {\n    let a = 1;\n}";
        let modified = "fn test() {\n    let a = 2;\n    let b = 3;\n}";

        let mut diff_view = DiffEditorView::new("test.rs", original, modified);
        assert_eq!(diff_view.mode, DiffViewMode::SideBySide);
        assert!(diff_view.model.additions > 0);

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        let mut scene = Scene::new();
        diff_view.render(bounds, &theme, &font, &mut scene);
        assert!(!scene.quads().is_empty());

        // Click toggle mode button
        let btn_bounds = diff_view.header_button_bounds.read().unwrap();
        let click = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(btn_bounds.min_x() + 5.0, btn_bounds.min_y() + 5.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = diff_view.handle_event(&click, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(diff_view.mode, DiffViewMode::Inline);

        // Render inline
        let mut inline_scene = Scene::new();
        diff_view.render(bounds, &theme, &font, &mut inline_scene);
        assert!(!inline_scene.quads().is_empty());
    }

    #[test]
    fn test_piece_table_apply_text_edits_multiple_disjoint() {
        let initial = "fn foo() {\n    let x = 1;\n    let y = 2;\n    x + y\n}\n";
        let mut view = EditorView::from_str(initial);

        // Multiple edits in arbitrary (ascending) order
        let edits = vec![
            RangeEdit::new(Range::new(Position::new(1, 4), Position::new(1, 14)), "let x = 10;"),
            RangeEdit::new(Range::new(Position::new(2, 4), Position::new(2, 14)), "let y = 20;"),
            RangeEdit::new(Range::new(Position::new(3, 4), Position::new(3, 9)), "x * y"),
        ];

        let ok = view.apply_text_edits(&edits);
        assert!(ok);

        let expected = "fn foo() {\n    let x = 10;\n    let y = 20;\n    x * y\n}\n";
        assert_eq!(view.text(), expected);
    }

    #[test]
    fn test_piece_table_apply_text_edits_whole_document() {
        let initial = "line1\nline2\nline3\n";
        let mut view = EditorView::from_str(initial);

        let replacement = "new file content\nformatted perfectly\n";
        let edits = vec![RangeEdit::new(
            Range::new(Position::new(0, 0), Position::new(3, 0)),
            replacement,
        )];

        let ok = view.apply_text_edits(&edits);
        assert!(ok);
        assert_eq!(view.text(), replacement);
    }

    #[test]
    fn test_piece_table_formatting_undo_redo_atomic() {
        let initial = "fn bar(){\nlet a=1;\nlet b=2;\n}\n";
        let mut view = EditorView::from_str(initial);

        let edits = vec![
            RangeEdit::new(Range::new(Position::new(0, 8), Position::new(0, 9)), " {"),
            RangeEdit::new(Range::new(Position::new(1, 0), Position::new(1, 8)), "    let a = 1;"),
            RangeEdit::new(Range::new(Position::new(2, 0), Position::new(2, 8)), "    let b = 2;"),
        ];

        assert!(view.apply_text_edits(&edits));
        let formatted = "fn bar() {\n    let a = 1;\n    let b = 2;\n}\n";
        assert_eq!(view.text(), formatted);

        // Single undo reverts entire format transaction atomically
        assert!(view.undo());
        assert_eq!(view.text(), initial);

        // Single redo restores entire format transaction atomically
        assert!(view.redo());
        assert_eq!(view.text(), formatted);
    }

    #[test]
    fn test_piece_table_apply_text_edits_multibyte_emoji_lsp_offset() {
        let initial = "let s = \"🦀abc\";\n";
        let mut view = EditorView::from_str(initial);

        // In LSP UTF-16 code units:
        // "let s = \"" is 9 code units.
        // '🦀' is 2 code units (indices 9..11, 4 UTF-8 bytes 9..13).
        // "abc" starts at column 11 and ends at column 14 (3 UTF-8 bytes 13..16).
        let edits = vec![RangeEdit::new(
            Range::new(Position::new(0, 11), Position::new(0, 14)),
            "xyz",
        )];

        assert!(view.apply_text_edits(&edits));
        let expected = "let s = \"🦀xyz\";\n";
        assert_eq!(view.text(), expected);

        // Undo atomically restores
        assert!(view.undo());
        assert_eq!(view.text(), initial);

        // Redo restores
        assert!(view.redo());
        assert_eq!(view.text(), expected);
    }

    #[test]
    fn test_piece_table_apply_text_edits_multibyte_no_trailing_newline() {
        let initial = "Hello 🦀";
        let mut view = EditorView::from_str(initial);

        // Replace '🦀' at col 6..8 with "World 🚀"
        let edits = vec![RangeEdit::new(
            Range::new(Position::new(0, 6), Position::new(0, 8)),
            "World 🚀",
        )];

        assert!(view.apply_text_edits(&edits));
        assert_eq!(view.text(), "Hello World 🚀");

        assert!(view.undo());
        assert_eq!(view.text(), initial);
    }

    #[test]
    fn test_piece_table_apply_text_edits_cjk_lsp_offset() {
        let initial = "let msg = \"你好世界\";\n";
        let mut view = EditorView::from_str(initial);

        // "let msg = \"" is 11 ASCII chars.
        // "你好" is 2 CJK characters = 2 UTF-16 code units (cols 11..13, 6 UTF-8 bytes 11..17).
        // "世界" is 2 CJK characters = 2 UTF-16 code units (cols 13..15, 6 UTF-8 bytes 17..23).
        let edits = vec![RangeEdit::new(
            Range::new(Position::new(0, 13), Position::new(0, 15)),
            "RustCode 🦀",
        )];

        assert!(view.apply_text_edits(&edits));
        let expected = "let msg = \"你好RustCode 🦀\";\n";
        assert_eq!(view.text(), expected);

        assert!(view.undo());
        assert_eq!(view.text(), initial);
    }

    #[test]
    fn test_minimap_rect_and_toggle() {
        let mut view = EditorView::from_str("fn main() {}\nlet x = 1;");
        let wide = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let narrow = Rect::from_xywh(0.0, 0.0, 150.0, 600.0);

        assert!(view.minimap_enabled);
        let mm = view.minimap_rect(wide).unwrap();
        assert_eq!(mm.min_x(), 716.0);
        assert_eq!(mm.max_x(), 800.0);
        assert_eq!(mm.width(), 84.0);
        assert!(view.minimap_rect(narrow).is_none());

        view.minimap_dragging = true;
        view.set_minimap_enabled(false);
        assert!(!view.minimap_enabled);
        assert!(!view.minimap_dragging);
        assert!(view.minimap_rect(wide).is_none());
    }

    #[test]
    fn test_minimap_render_clip_and_diagnostics() {
        use rustcode_platform::render::SceneNode;

        let content = "pub fn hello_world() -> usize { 42 }\n".repeat(120);
        let mut view = EditorView::from_str(&content);
        let registry = GrammarRegistry::default();
        let theme_svc = rustcode_platform::theme::ThemeService::default_dark_theme();
        view.set_language("rust", theme_svc, &registry);
        view.set_diagnostics(vec![EditorDiagnostic::new(
            Range::new(Position::new(10, 0), Position::new(10, 5)),
            DiagnosticSeverity::Error,
            "Sample error",
        )]);

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 300.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        let mm_rect = view.minimap_rect_with_gutter(bounds, view.gutter_width(&font)).unwrap();
        let has_code_clip = scene.nodes().iter().any(|n| match n {
            SceneNode::PushClip(r) => (r.max_x() - mm_rect.min_x()).abs() < 0.01 && r.min_x() > 0.0,
            _ => false,
        });
        assert!(has_code_clip, "Expected PushClip excluding minimap_rect");

        let minimap_quads: Vec<_> = scene
            .quads()
            .into_iter()
            .filter(|q| q.bounds.min_x() >= mm_rect.min_x() - 0.1)
            .collect();
        assert!(
            minimap_quads.len() > 5,
            "Expected background, slider, code blocks, diagnostic tick, and cursor indicator in minimap"
        );

        let has_error_tick = minimap_quads
            .iter()
            .any(|q| q.background == Some(theme.editor_error_foreground));
        assert!(has_error_tick, "Expected diagnostic error tick in minimap");

        // Disabling minimap removes minimap quads and code clip expands to full width
        view.set_minimap_enabled(false);
        let mut disabled_scene = Scene::new();
        view.render(bounds, &theme, &font, &mut disabled_scene);
        let disabled_mm_quads = disabled_scene
            .quads()
            .into_iter()
            .filter(|q| q.bounds.min_x() >= mm_rect.min_x() - 0.1)
            .count();
        assert_eq!(disabled_mm_quads, 0);
    }

    #[test]
    fn test_minimap_mouse_click_drag_wheel_do_not_move_cursor() {
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{MouseButton, MouseEvent, MouseMoveEvent, ScrollEvent};

        let content: String = (0..200).map(|i| format!("let line_{} = {};\n", i, i)).collect();
        let mut view = EditorView::from_str(&content);
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 300.0);
        let mm = view.minimap_rect(bounds).unwrap();
        let initial_pos = view.primary_position();
        assert_eq!(initial_pos, Position::new(0, 0));

        // MouseDown near bottom of minimap
        let down = InputEvent::MouseDown(MouseEvent {
            position: Point::new(mm.min_x() + 20.0, mm.max_y() - 20.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        assert_eq!(view.handle_event(&down, bounds), EventResult::Consumed);
        assert!(view.minimap_dragging);
        assert!(view.scroll_top > 50.0);
        assert_eq!(view.primary_position(), initial_pos);

        // Drag upward
        let prev_scroll = view.scroll_top;
        let drag = InputEvent::MouseMove(MouseMoveEvent {
            position: Point::new(mm.min_x() - 10.0, mm.min_y() + 20.0),
        });
        assert_eq!(view.handle_event(&drag, bounds), EventResult::Consumed);
        assert!(view.scroll_top < prev_scroll);
        assert_eq!(view.primary_position(), initial_pos);

        // MouseUp ends drag
        let up = InputEvent::MouseUp(MouseEvent {
            position: Point::new(mm.min_x() - 10.0, mm.min_y() + 20.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        assert_eq!(view.handle_event(&up, bounds), EventResult::Consumed);
        assert!(!view.minimap_dragging);

        // ScrollWheel inside minimap scrolls without moving cursor
        let scroll_before = view.scroll_top;
        let wheel = InputEvent::ScrollWheel(ScrollEvent {
            position: Point::new(mm.min_x() + 10.0, mm.min_y() + 50.0),
            delta_x: 0.0,
            delta_y: -100.0,
            modifiers: Modifiers::default(),
        });
        assert_eq!(view.handle_event(&wheel, bounds), EventResult::Consumed);
        assert!(view.scroll_top > scroll_before);
        assert_eq!(view.primary_position(), initial_pos);
    }

    #[test]
    fn test_editor_accessibility_node_with_and_without_minimap() {
        let mut doc = TextDocument::new(
            "fn main() {}\nlet a = 1;",
            Some(std::path::PathBuf::from("/workspace/src/main.rs")),
            Some("rust"),
        );
        doc.is_dirty = true;

        let mut view = EditorView::from_str("").with_document(Arc::new(RwLock::new(doc)));
        view.set_primary_position(Position::new(1, 4));
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let font = FontMetrics::default_monospace();

        let node = view.accessibility_node("ed.1", bounds, &font);
        assert_eq!(node.role, AccessibleRole::TextEditor);
        assert_eq!(node.name, "main.rs");
        assert!(node.state.focused);
        let desc = node.description.as_deref().unwrap();
        assert!(desc.contains("Ln 2, Col 5"));
        assert!(desc.contains("rust"));
        assert!(desc.contains("modified"));
        assert_eq!(node.children.len(), 1);
        assert_eq!(node.children[0].role, AccessibleRole::ScrollBar);
        assert_eq!(node.children[0].name, "Minimap");

        view.set_minimap_enabled(false);
        let node_no_mm = view.accessibility_node("ed.1", bounds, &font);
        assert!(node_no_mm.children.is_empty());
    }

    #[test]
    fn test_minimap_render_does_not_poison_multiline_comment_cache_on_scroll() {
        let mut lines = Vec::new();
        for i in 0..5 {
            lines.push(format!("let before_{} = {};", i, i));
        }
        lines.push("/* start multiline block comment".to_string());
        for i in 6..220 {
            if i % 5 == 0 {
                lines.push(String::new());
            } else if i % 7 == 0 {
                lines.push("   ".to_string());
            } else {
                lines.push(format!("comment body line {}", i));
            }
        }
        lines.push("end multiline block comment */".to_string());
        lines.push("let after_comment = 42;".to_string());
        let content = lines.join("\n");

        let mut view = EditorView::from_str(&content);
        let registry = GrammarRegistry::default();
        let theme_svc = rustcode_platform::theme::ThemeService::default_dark_theme();
        view.set_language("rust", theme_svc.clone(), &registry);

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 300.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        // Render at scroll_top = 0.0 with minimap enabled (sample_step > 1 and blank lines present)
        let mut scene1 = Scene::new();
        view.render(bounds, &theme, &font, &mut scene1);

        // Scroll down into the middle of the block comment (past initial viewport and blank lines)
        view.scroll_top = 150.0;
        let mut scene2 = Scene::new();
        view.render(bounds, &theme, &font, &mut scene2);

        let comment_color =
            SyntaxHighlighter::resolve_color(&theme_svc, crate::SyntaxScope::CommentBlock);
        let gutter_w = view.gutter_width(&font);
        let target_y = bounds.min_y() + (151.0 - view.scroll_top) * font.line_height;
        let line_151_runs: Vec<_> = scene2
            .glyph_runs()
            .into_iter()
            .filter(|g| (g.origin.y - target_y).abs() < 0.5 && g.origin.x >= bounds.min_x() + gutter_w)
            .collect();
        assert!(!line_151_runs.is_empty(), "expected rendered glyph runs for line 151");
        assert!(
            line_151_runs.iter().all(|g| g.color == comment_color),
            "expected CommentBlock color on scrolled multiline comment, got {:?}",
            line_151_runs.iter().map(|g| (&g.text, g.color)).collect::<Vec<_>>()
        );

        let guard = view.syntax_highlighter.lock();
        let hl = guard.as_ref().unwrap();
        assert_eq!(hl.cache_len(), lines.len());
    }

    #[test]
    fn test_code_glyph_runs_horizontally_culled_before_minimap_rect() {
        use rustcode_platform::render::SceneNode;

        let long_rust = format!("let very_long_variable_name = \"{}\"; // trailing comment", "x".repeat(180));
        let long_plain = "a".repeat(220);
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for (lang, text) in [("rust", long_rust), ("plaintext", long_plain)] {
            let mut view = EditorView::from_str(&text);
            let registry = GrammarRegistry::default();
            let theme_svc = rustcode_platform::theme::ThemeService::default_dark_theme();
            view.set_language(lang, theme_svc, &registry);

            let mut scene = Scene::new();
            view.render(bounds, &theme, &font, &mut scene);

            let mm_rect = view
                .minimap_rect_with_gutter(bounds, view.gutter_width(&font))
                .unwrap();

            let mut push_clips = 0usize;
            let mut pop_clips = 0usize;
            for node in scene.nodes() {
                match node {
                    SceneNode::PushClip(_) => push_clips += 1,
                    SceneNode::PopClip => pop_clips += 1,
                    SceneNode::GlyphRun(run) => {
                        assert!(
                            run.origin.x < mm_rect.min_x(),
                            "lang={} emitted GlyphRun origin inside minimap: {:?}",
                            lang,
                            run
                        );
                        assert!(
                            run.bounds.max_x() <= mm_rect.min_x(),
                            "lang={} emitted GlyphRun overlapping minimap: {:?} (mm_rect.min_x={})",
                            lang,
                            run,
                            mm_rect.min_x()
                        );
                    }
                    _ => {}
                }
            }
            assert_eq!(push_clips, pop_clips, "unbalanced clip stack for {}", lang);

            let mm_hits = scene.hit_test(Point::new(mm_rect.min_x() + 20.0, 5.0));
            assert!(
                !mm_hits.iter().any(|n| matches!(n, SceneNode::GlyphRun(_))),
                "scene.hit_test inside minimap_rect returned GlyphRun for {}",
                lang
            );
        }

        // Strict <= mm_rect.min_x() (0 ULP slack) under fractional origins, widths, and scroll_left
        let mut view = EditorView::from_str(&"let x = \"🦀αβγ\".repeat(50);".repeat(2));
        let registry = GrammarRegistry::default();
        let theme_svc = rustcode_platform::theme::ThemeService::default_dark_theme();
        view.set_language("rust", theme_svc, &registry);
        for origin_x in [0.0_f32, 10.3, 217.7] {
            for width in [300.0_f32, 513.7, 799.3] {
                for scroll_left in [0.0_f32, 0.5, 23.3] {
                    view.scroll_left = scroll_left;
                    let frac_bounds = Rect::from_xywh(origin_x, 0.0, width, 200.0);
                    let mut scene = Scene::new();
                    view.render(frac_bounds, &theme, &font, &mut scene);
                    let mm_rect = view
                        .minimap_rect_with_gutter(frac_bounds, view.gutter_width(&font))
                        .unwrap();
                    for node in scene.nodes() {
                        if let SceneNode::GlyphRun(run) = node {
                            assert!(
                                run.origin.x < mm_rect.min_x(),
                                "origin.x >= mm_rect.min_x() at origin_x={}, width={}, scroll_left={}",
                                origin_x,
                                width,
                                scroll_left
                            );
                            assert!(
                                run.bounds.max_x() <= mm_rect.min_x(),
                                "bounds.max_x()={} > mm_rect.min_x()={} at origin_x={}, width={}, scroll_left={}",
                                run.bounds.max_x(),
                                mm_rect.min_x(),
                                origin_x,
                                width,
                                scroll_left
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_editor_render_short_height_minimap_no_panic() {
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let single_line = "fn main() { println!(\"short height\"); }";
        let compressed_lines: String = (0..300).map(|i| format!("let line_{} = {};\n", i, i)).collect();

        for (label, content) in [("single_line", single_line), ("compressed_300", compressed_lines.as_str())] {
            let view = EditorView::from_str(content);
            for height in [0.0_f32, 0.5, 10.0, 14.0, 15.9] {
                let bounds = Rect::from_xywh(0.0, 0.0, 800.0, height);
                let mut scene = Scene::new();
                view.render(bounds, &theme, &font, &mut scene);
                assert!(
                    !scene.quads().is_empty() || height <= 0.0,
                    "expected rendered quads for {} at height={}",
                    label,
                    height
                );
            }
        }
    }

    #[test]
    fn test_editor_render_and_helpers_non_finite_bounds_no_panic_no_nan() {
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{MouseButton, MouseEvent, SceneNode, Size};

        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let compressed_lines: String = (0..300).map(|i| format!("let line_{} = {};\n", i, i)).collect();

        let bad_bounds = [
            Rect::from_xywh(0.0, 0.0, 800.0, f32::INFINITY),
            Rect::new(Point::zero(), Size::new(800.0, f32::NAN)),
            Rect::from_xywh(0.0, 0.0, f32::INFINITY, 600.0),
            Rect::new(Point::zero(), Size::new(f32::NAN, 600.0)),
            Rect::from_xywh(f32::NAN, 0.0, 800.0, 600.0),
            Rect::from_xywh(0.0, f32::NEG_INFINITY, 800.0, 600.0),
            Rect::new(Point::new(f32::MAX, f32::MAX), Size::new(f32::MAX, f32::MAX)),
        ];

        for content in ["", "fn main() {}", compressed_lines.as_str()] {
            for scroll_top in [0.0_f32, 5.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut view = EditorView::from_str(content);
                view.scroll_top = scroll_top;
                view.scroll_left = f32::NAN;

                for &bounds in &bad_bounds {
                    let mut scene = Scene::new();
                    view.render(bounds, &theme, &font, &mut scene);

                    let mut clip_depth = 0isize;
                    for node in scene.nodes() {
                        match node {
                            SceneNode::PushClip(r) => {
                                clip_depth += 1;
                                assert!(is_finite_rect(*r), "non-finite clip: {:?}", r);
                            }
                            SceneNode::PopClip => {
                                clip_depth -= 1;
                                assert!(clip_depth >= 0, "clip underflow");
                            }
                            SceneNode::Quad(q) => {
                                assert!(is_finite_rect(q.bounds), "non-finite quad: {:?}", q.bounds);
                            }
                            SceneNode::GlyphRun(g) => {
                                assert!(g.origin.x.is_finite() && g.origin.y.is_finite());
                                assert!(is_finite_rect(g.bounds), "non-finite glyph: {:?}", g.bounds);
                            }
                            SceneNode::Shadow(s) => {
                                assert!(is_finite_rect(s.bounds));
                            }
                        }
                    }
                    assert_eq!(clip_depth, 0, "unbalanced clip stack");
                    assert!(scene.quads().is_empty(), "expected 0 quads for non-finite bounds {:?}", bounds);
                    assert!(scene.glyph_runs().is_empty(), "expected 0 glyphs for non-finite bounds {:?}", bounds);

                    assert!(view.minimap_rect_with_gutter(bounds, view.gutter_width(&font)).is_none());

                    let acc = view.accessibility_node("ed.bad", bounds, &font);
                    assert!(is_finite_rect(acc.bounds));

                    let down = InputEvent::MouseDown(MouseEvent {
                        position: Point::new(10.0, 10.0),
                        button: MouseButton::Left,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    });
                    assert_eq!(view.handle_event(&down, bounds), EventResult::Ignored);
                }
            }
        }

        // Non-finite FontMetrics also must not emit NaN/Inf or panic
        let view = EditorView::from_str("fn main() {}");
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
                char_width: f32::INFINITY,
                ..font.clone()
            },
        ] {
            let gw = view.gutter_width(&bad_font);
            assert!(gw.is_finite() && gw >= 48.0);
            let mut scene = Scene::new();
            view.render(valid_bounds, &theme, &bad_font, &mut scene);
            for q in scene.quads() {
                assert!(is_finite_rect(q.bounds));
            }
        }
    }

    #[test]
    fn test_editor_render_extreme_scroll_and_font_metrics_no_inf_and_squiggly_bounded() {
        use rustcode_platform::render::SceneNode;

        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let compressed_lines: String = (0..300).map(|i| format!("let line_{} = {};\n", i, i)).collect();

        let assert_scene_valid = |scene: &Scene, label: &str| {
            let mut clip_depth = 0isize;
            for node in scene.nodes() {
                match node {
                    SceneNode::PushClip(r) => {
                        clip_depth += 1;
                        assert!(is_finite_rect(*r), "{}: non-finite PushClip: {:?}", label, r);
                    }
                    SceneNode::PopClip => {
                        clip_depth -= 1;
                        assert!(clip_depth >= 0, "{}: clip stack underflow", label);
                    }
                    SceneNode::Quad(q) => {
                        assert!(is_finite_rect(q.bounds), "{}: non-finite Quad: {:?}", label, q.bounds);
                    }
                    SceneNode::GlyphRun(g) => {
                        assert!(
                            g.origin.x.is_finite() && g.origin.y.is_finite(),
                            "{}: non-finite GlyphRun origin: {:?}",
                            label,
                            g.origin
                        );
                        assert!(is_finite_rect(g.bounds), "{}: non-finite GlyphRun bounds: {:?}", label, g.bounds);
                    }
                    SceneNode::Shadow(s) => {
                        assert!(is_finite_rect(s.bounds), "{}: non-finite Shadow: {:?}", label, s.bounds);
                    }
                }
            }
            assert_eq!(clip_depth, 0, "{}: unbalanced clip stack", label);
        };

        // 1. Extreme finite scroll_top = 1e38, scroll_left = 1e38, font.line_height = 1e38, font.char_width = 1e38
        for content in ["", "let x = 1;", compressed_lines.as_str()] {
            for (scroll_top, scroll_left, test_font) in [
                (1e38_f32, 0.0_f32, font.clone()),
                (0.0_f32, 1e38_f32, font.clone()),
                (1e38_f32, 1e38_f32, font.clone()),
                (
                    5.0_f32,
                    0.0_f32,
                    FontMetrics {
                        line_height: 1e38,
                        ..font.clone()
                    },
                ),
                (
                    0.0_f32,
                    10.0_f32,
                    FontMetrics {
                        char_width: 1e38,
                        ..font.clone()
                    },
                ),
                (
                    5.0_f32,
                    10.0_f32,
                    FontMetrics {
                        line_height: 2e38,
                        char_width: 2e38,
                        ..font.clone()
                    },
                ),
            ] {
                let mut view = EditorView::from_str(content);
                view.scroll_top = scroll_top;
                view.scroll_left = scroll_left;
                view.toggle_breakpoint(1);
                view.set_execution_line(Some(1));
                view.cursor_state.primary_mut().anchor = Position::new(0, 0);
                view.cursor_state.primary_mut().active = Position::new(0, 4);
                let diag = EditorDiagnostic::error(
                    crate::cursor::Range::new(Position::new(0, 0), Position::new(0, 4)),
                    "err",
                );
                view.set_diagnostics(vec![diag.clone()]);
                view.hovered_diagnostic = Some(diag);
                view.hover_position = Some(Point::new(100.0, 100.0));
                view.autocomplete.show(
                    vec![CompletionItem::new("println!")],
                    Position::new(0, 0),
                    None,
                );

                let mut scene = Scene::new();
                view.render(bounds, &theme, &test_font, &mut scene);
                assert_scene_valid(
                    &scene,
                    &format!(
                        "len={} st={} sl={} lh={} cw={}",
                        content.len(),
                        scroll_top,
                        scroll_left,
                        test_font.line_height,
                        test_font.char_width
                    ),
                );
            }
        }

        // 2. render_squiggly_wave precision-loss hang & clipping regression
        let mut view = EditorView::from_str("let x = 1;");
        view.set_diagnostics(vec![EditorDiagnostic::error(
            crate::cursor::Range::new(Position::new(0, 0), Position::new(0, 4)),
            "squiggly precision stress",
        )]);
        for cw in [1e7_f32, 1e38_f32] {
            let big_cw_font = FontMetrics {
                char_width: cw,
                ..font.clone()
            };
            let mut scene = Scene::new();
            view.render(bounds, &theme, &big_cw_font, &mut scene);
            assert_scene_valid(&scene, &format!("squiggly cw={}", cw));
            assert!(scene.quads().len() < 10_000, "unbounded quads emitted for cw={}", cw);
        }

        // Direct precision-loss check:
        // - at 3.4e7 (between 2^25 and 2^26, ULP=4), MAX_SQUIGGLY_SEGMENTS bounds output
        // - at 7.0e7 (>= 2^26, ULP=8), cur_x + 3.0 == cur_x breaks immediately
        let mut mid_scene = Scene::new();
        render_squiggly_wave(
            &mut mid_scene,
            3.4e7,
            3.5e7,
            100.0,
            theme.editor_error_foreground,
        );
        assert!(mid_scene.quads().len() <= 4096);
        for q in mid_scene.quads() {
            assert!(is_finite_rect(q.bounds));
        }

        let mut direct_scene = Scene::new();
        render_squiggly_wave(
            &mut direct_scene,
            7.0e7,
            7.1e7,
            100.0,
            theme.editor_error_foreground,
        );
        assert!(direct_scene.quads().len() <= 1);
        for q in direct_scene.quads() {
            assert!(is_finite_rect(q.bounds));
        }
    }

    #[test]
    fn test_editor_render_with_autocomplete_doc_extreme_bounds_and_line_height_finite() {
        use rustcode_platform::render::SceneNode;

        let mut view = EditorView::from_str("let x = foo.cl;");
        view.cursor_state.set_primary(Position::new(0, 14));
        view.autocomplete.show_documentation = true;
        view.autocomplete.show(
            vec![
                CompletionItem::new("clone")
                    .with_kind(CompletionItemKind::Method)
                    .with_detail("fn(&self) -> Self")
                    .with_doc("Returns a duplicate of the value.\nSecond doc line."),
            ],
            Position::new(0, 12),
            Some('.'),
        );

        let theme = WorkbenchThemeStyles::dark_default();
        let base_font = FontMetrics::default_monospace();

        for (bounds, line_height) in [
            (Rect::from_xywh(0.0, 2.5e38, 800.0, 0.9e38), 1e38_f32),
            (Rect::from_xywh(0.0, 0.0, f32::MAX, f32::MAX), 2.5e38_f32),
            (Rect::from_xywh(1e38, 1e38, 1e38, 1e38), 2.5e38_f32),
        ] {
            let font = FontMetrics {
                line_height,
                ..base_font.clone()
            };
            let mut scene = Scene::new();
            view.render(bounds, &theme, &font, &mut scene);

            let mut clip_depth = 0isize;
            for node in scene.nodes() {
                match node {
                    SceneNode::PushClip(r) => {
                        clip_depth += 1;
                        assert!(is_finite_rect(*r), "non-finite PushClip: {:?}", r);
                    }
                    SceneNode::PopClip => {
                        clip_depth -= 1;
                        assert!(clip_depth >= 0, "clip stack underflow");
                    }
                    SceneNode::Quad(q) => {
                        assert!(is_finite_rect(q.bounds), "non-finite Quad: {:?}", q.bounds);
                    }
                    SceneNode::Shadow(s) => {
                        assert!(is_finite_rect(s.bounds), "non-finite Shadow: {:?}", s.bounds);
                    }
                    SceneNode::GlyphRun(g) => {
                        assert!(
                            g.origin.x.is_finite() && g.origin.y.is_finite(),
                            "non-finite GlyphRun origin: {:?}",
                            g.origin
                        );
                        assert!(
                            is_finite_rect(g.bounds),
                            "non-finite GlyphRun bounds: {:?}",
                            g.bounds
                        );
                    }
                }
            }
            assert_eq!(clip_depth, 0, "unbalanced clip stack");
        }
    }

    #[test]
    fn test_editor_view_code_folding_gutter_render_and_navigation() {
        let src = "fn alpha() {\n    let a = 1;\n    let b = 2;\n}\nfn beta() {\n    let c = 3;\n}";
        let mut view = EditorView::from_str(src);
        let registry = GrammarRegistry::default();
        let theme_svc = rustcode_platform::theme::ThemeService::default_dark_theme();
        view.set_language("rust", theme_svc, &registry);

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        // Render unfolded -> shows "▾" and hidden lines
        let mut scene1 = Scene::new();
        view.render(bounds, &theme, &font, &mut scene1);
        assert!(scene1.contains_text("▾"));
        assert!(scene1.contains_text("let a = 1;"));
        assert!(scene1.contains_text("let b = 2;"));

        // Click fold chevron on line 0 (right edge of gutter)
        let gutter_w = view.gutter_width(&font);
        let fold_click = InputEvent::MouseDown(MouseEvent::new(
            Point::new(bounds.min_x() + gutter_w - 8.0, bounds.min_y() + font.line_height * 0.5),
            rustcode_platform::render::MouseButton::Left,
        ));
        assert_eq!(view.handle_event(&fold_click, bounds), EventResult::Consumed);
        assert_eq!(
            view.last_gutter_action,
            Some(EditorGutterAction::FoldToggled {
                line: 0,
                collapsed: true,
            })
        );
        assert!(view.folding_model.is_collapsed(0));

        // Render folded -> shows "▸", " ...", hides lines 1..=3, keeps line 4 ("fn beta() {") visible
        let mut scene2 = Scene::new();
        view.render(bounds, &theme, &font, &mut scene2);
        assert!(scene2.contains_text("▸"));
        assert!(scene2.contains_text(" ..."));
        assert!(!scene2.contains_text("let a = 1;"));
        assert!(!scene2.contains_text("let b = 2;"));
        assert!(scene2.contains_text("fn beta() {"));

        // Left-margin click on line 4 still toggles breakpoint without touching fold
        let line4_row_y = bounds.min_y() + font.line_height * 1.5; // visual row 1 is line 4
        let bp_click = InputEvent::MouseDown(MouseEvent::new(
            Point::new(bounds.min_x() + 10.0, line4_row_y),
            rustcode_platform::render::MouseButton::Left,
        ));
        assert_eq!(view.handle_event(&bp_click, bounds), EventResult::Consumed);
        assert_eq!(
            view.last_gutter_action,
            Some(EditorGutterAction::BreakpointToggled {
                line: 5,
                added: true,
            })
        );

        // Cursor down from line 0 skips folded lines 1..=3 and lands on line 4
        view.set_primary_position(Position::new(0, 0));
        view.move_cursor_down(false);
        assert_eq!(view.primary_position().line, 4);
        view.move_cursor_up(false);
        assert_eq!(view.primary_position().line, 0);

        // Second fold chevron click unfolds line 0
        assert_eq!(view.handle_event(&fold_click, bounds), EventResult::Consumed);
        assert_eq!(
            view.last_gutter_action,
            Some(EditorGutterAction::FoldToggled {
                line: 0,
                collapsed: false,
            })
        );
        assert!(!view.folding_model.is_collapsed(0));

        // fold_at_cursor while inside inner line clamps cursor to header line
        view.set_primary_position(Position::new(2, 6));
        assert!(view.fold_at_cursor());
        assert!(view.folding_model.is_collapsed(0));
        assert_eq!(view.primary_position().line, 0);
        assert!(view.unfold_at_cursor());
        assert!(!view.folding_model.is_collapsed(0));
    }

    #[test]
    fn test_editor_view_soft_word_wrap_layout_render_and_hit_testing() {
        let long_line = "alpha bravo charlie delta echo foxtrot";
        let mut view = EditorView::from_str(&format!("{}\nsecond", long_line))
            .with_minimap(false)
            .with_word_wrap(WordWrapMode::WordWrapColumn)
            .with_word_wrap_column(16);

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        // Add diagnostic on wrapped portion ("delta" starts at col 20)
        view.set_diagnostics(vec![EditorDiagnostic::new(
            Range::new(Position::new(0, 20), Position::new(0, 25)),
            DiagnosticSeverity::Warning,
            "Wrapped diag",
        )]);

        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        // Gutter line number "1" appears only once even though line 0 wrapped into multiple rows
        let gutter_w = view.gutter_width(&font);
        let line1_nums: Vec<_> = scene
            .glyph_runs()
            .into_iter()
            .filter(|g| g.text == "1" && g.origin.x < bounds.min_x() + gutter_w)
            .collect();
        assert_eq!(line1_nums.len(), 1);

        // Line 2 ("second") is pushed down past the wrapped segments of line 0
        let line2_num = scene
            .glyph_runs()
            .into_iter()
            .find(|g| g.text == "2" && g.origin.x < bounds.min_x() + gutter_w)
            .unwrap();
        assert!(line2_num.origin.y >= bounds.min_y() + font.line_height * 2.0);

        // Clicking on visual row 1 ("charlie delta ") places cursor on buffer line 0 at column > 10
        let click_row1 = InputEvent::MouseDown(MouseEvent::new(
            Point::new(
                bounds.min_x() + gutter_w + 8.0 + 2.0 * font.char_width,
                bounds.min_y() + font.line_height * 1.5,
            ),
            rustcode_platform::render::MouseButton::Left,
        ));
        assert_eq!(view.handle_event(&click_row1, bounds), EventResult::Consumed);
        assert_eq!(view.primary_position().line, 0);
        assert!(view.primary_position().column >= 12);

        // Test WordWrapMode::On wraps to narrow viewport & preserves state on idempotent set
        view.set_word_wrap(WordWrapMode::On);
        view.scroll_top = 1.0;
        let saved_pos = view.primary_position();
        view.set_word_wrap(WordWrapMode::On);
        view.set_word_wrap_column(16);
        assert_eq!(view.scroll_top, 1.0);
        assert_eq!(view.primary_position(), saved_pos);

        // Bounded mode uses min(viewport_cols, word_wrap_column)
        let narrow_bounds = Rect::from_xywh(0.0, 0.0, 160.0, 300.0);
        view.set_word_wrap(WordWrapMode::Bounded);
        view.set_word_wrap_column(80);
        let eff = view
            .effective_wrap_columns(narrow_bounds, gutter_w, &font)
            .unwrap();
        assert!((1..80).contains(&eff));
    }

    #[test]
    fn test_editor_view_multi_cursor_editing_ctrl_d_alt_up_down_and_escape() {
        let mut view = EditorView::from_str("🦀 foo bar\nfoo baz\nfoofoo");
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);

        // Place cursor at "foo" on line 0 (after "🦀 " -> UTF-16 col 3)
        view.set_primary_position(Position::new(0, 4));

        // 1. First Ctrl+D selects "foo" on line 0 (UTF-16 cols 3..6)
        let ctrl_d = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Char('d'),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        assert_eq!(view.handle_event(&ctrl_d, bounds), EventResult::Consumed);
        assert_eq!(view.cursor_state.selections.len(), 1);
        assert_eq!(
            view.cursor_state.primary(),
            &Selection::new(Position::new(0, 3), Position::new(0, 6))
        );

        // 2. Ctrl+D three more times selects line 1 "foo" and BOTH adjacent "foo"s in "foofoo"
        assert_eq!(view.handle_event(&ctrl_d, bounds), EventResult::Consumed);
        assert_eq!(view.handle_event(&ctrl_d, bounds), EventResult::Consumed);
        assert_eq!(view.handle_event(&ctrl_d, bounds), EventResult::Consumed);
        assert_eq!(view.cursor_state.selections.len(), 4);

        // 3. Replace all 4 occurrences atomically with "qux"
        view.insert_text("qux");
        assert_eq!(view.text(), "🦀 qux bar\nqux baz\nquxqux");
        assert_eq!(view.cursor_state.selections.len(), 4);

        // 4. Single atomic undo restores all 4 occurrences and their selections
        assert!(view.undo());
        assert_eq!(view.text(), "🦀 foo bar\nfoo baz\nfoofoo");
        assert_eq!(view.cursor_state.selections.len(), 4);

        // 5. Escape collapses secondary cursors when multiple cursors exist
        let esc_collapse = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Escape,
            Modifiers::none(),
        ));
        assert_eq!(view.handle_event(&esc_collapse, bounds), EventResult::Consumed);
        assert_eq!(view.cursor_state.selections.len(), 1);
        view.set_primary_position(Position::new(0, 3));
        let ctrl_alt_down = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::ArrowDown,
            Modifiers {
                ctrl: true,
                shift: false,
                alt: true,
                meta: false,
            },
        ));
        let ctrl_alt_up = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::ArrowUp,
            Modifiers {
                ctrl: true,
                shift: false,
                alt: true,
                meta: false,
            },
        ));
        assert_eq!(view.handle_event(&ctrl_alt_down, bounds), EventResult::Consumed);
        assert_eq!(view.cursor_state.selections.len(), 2);
        // Ctrl+Alt+Up from {(0,3), (1,3)} inserts (0,3) which deduplicates with existing (0,3)
        assert_eq!(view.handle_event(&ctrl_alt_up, bounds), EventResult::Consumed);
        assert_eq!(view.cursor_state.selections.len(), 2);

        // Ctrl+Alt+Down from {(0,3), (1,3)} adds line 2 -> 3 cursors across lines 0, 1, 2
        assert_eq!(view.handle_event(&ctrl_alt_down, bounds), EventResult::Consumed);
        assert_eq!(view.cursor_state.selections.len(), 3);

        // Escape clears secondary cursors, then Ctrl+Alt+Up from line 2 adds upward
        let esc_check = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Escape,
            Modifiers::none(),
        ));
        assert_eq!(view.handle_event(&esc_check, bounds), EventResult::Consumed);
        assert_eq!(view.cursor_state.selections.len(), 1);
        view.set_primary_position(Position::new(2, 3));
        assert_eq!(view.handle_event(&ctrl_alt_up, bounds), EventResult::Consumed);
        assert_eq!(view.handle_event(&ctrl_alt_up, bounds), EventResult::Consumed);
        assert_eq!(view.cursor_state.selections.len(), 3);

        // Multi-cursor backspace at mixed positions (including offset 0 collision)
        view.move_cursor_home(false);
        view.insert_text("// ");
        assert_eq!(view.text(), "// 🦀 foo bar\n// foo baz\n// foofoo");
        view.delete_backwards();
        assert_eq!(view.text(), "//🦀 foo bar\n//foo baz\n//foofoo");

        // Escape clears secondary cursors
        let esc = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Escape,
            Modifiers::none(),
        ));
        assert_eq!(view.handle_event(&esc, bounds), EventResult::Consumed);
        assert_eq!(view.cursor_state.selections.len(), 1);
    }

    #[test]
    fn test_editor_view_regression_multicursor_undo_redo_and_external_fallback() {
        // probe_1b: Multi-cursor undo/redo restores primary & secondary selections
        let mut view = EditorView::from_str("alpha 🦀\nbeta 世\ngamma 🚀");
        view.set_primary_position(Position::new(0, 8));
        view.cursor_state
            .add_selection(Selection::cursor(Position::new(2, 8)));
        view.insert_text("!");
        assert_eq!(view.text(), "alpha 🦀!\nbeta 世\ngamma 🚀!");
        assert_eq!(view.primary_position(), Position::new(0, 9));
        assert_eq!(
            view.cursor_state.selections,
            vec![
                Selection::cursor(Position::new(0, 9)),
                Selection::cursor(Position::new(2, 9)),
            ]
        );

        assert!(view.undo());
        assert_eq!(view.text(), "alpha 🦀\nbeta 世\ngamma 🚀");
        assert_eq!(view.primary_position(), Position::new(0, 8));
        assert_eq!(
            view.cursor_state.selections,
            vec![
                Selection::cursor(Position::new(0, 8)),
                Selection::cursor(Position::new(2, 8)),
            ]
        );

        assert!(view.redo());
        assert_eq!(view.text(), "alpha 🦀!\nbeta 世\ngamma 🚀!");
        assert_eq!(view.primary_position(), Position::new(0, 9));
        assert_eq!(
            view.cursor_state.selections,
            vec![
                Selection::cursor(Position::new(0, 9)),
                Selection::cursor(Position::new(2, 9)),
            ]
        );

        // External push onto undo_stack falls back cleanly to offset_to_position without corrupting earlier snapshots
        {
            let mut buf = view.buffer.write();
            buf.insert(0, "// ");
            view.undo_stack.push_edit(TextEdit::insert(0, "// "));
        }
        assert!(view.undo());
        assert_eq!(view.text(), "alpha 🦀!\nbeta 世\ngamma 🚀!");
        assert_eq!(view.primary_position(), Position::new(0, 0));

        // Next undo still recovers the multi-cursor snapshot from before insert_text("!")
        assert!(view.undo());
        assert_eq!(view.text(), "alpha 🦀\nbeta 世\ngamma 🚀");
        assert_eq!(
            view.cursor_state.selections,
            vec![
                Selection::cursor(Position::new(0, 8)),
                Selection::cursor(Position::new(2, 8)),
            ]
        );
    }

    #[test]
    fn test_editor_view_regression_fold_clamp_utf16_surrogate_and_multicursor_newline_fold_shift() {
        use crate::folding::{FoldingRange, FoldingRangeKind};

        // probe_4: folding onto a header containing '🦀' clamps anchor & active without landing mid-surrogate
        let mut view = EditorView::from_str("// 🦀 header {\n    let x = 1;\n}");
        view.folding_model
            .add_range(FoldingRange::new(0, 2, FoldingRangeKind::Brace));
        view.cursor_state.selections = vec![Selection::new(
            Position::new(1, 4),
            Position::new(2, 0),
        )];
        // Line 0: "// " (3) + '🦀' (2 -> cols 3..5) + " header {"
        // Col 4 is mid-surrogate of '🦀', so clamp_utf16_col must land on 3, never 4!
        assert!(view.fold_at_cursor());
        assert_eq!(view.cursor_state.primary().anchor, Position::new(0, 3));
        assert_eq!(view.cursor_state.primary().active, Position::new(0, 0));

        // probe_5: multi-cursor newline insertion across a collapsed fold shifts it accurately
        let src = "fn f1() {\n    a\n}\nmid\nfn f2() {\n    b\n}\nend";
        let mut fold_view = EditorView::from_str(src);
        let registry = GrammarRegistry::default();
        let theme_svc = rustcode_platform::theme::ThemeService::default_dark_theme();
        fold_view.set_language("rust", theme_svc, &registry);
        assert!(fold_view.folding_model.fold(0));
        assert!(fold_view.folding_model.fold(4));

        fold_view.set_primary_position(Position::new(3, 0));
        fold_view
            .cursor_state
            .add_selection(Selection::cursor(Position::new(7, 0)));
        fold_view.insert_text("\n");
        assert!(fold_view.folding_model.is_collapsed(0));
        assert!(fold_view.folding_model.is_collapsed(5));
        assert_eq!(fold_view.folding_model.collapsed.len(), 2);
    }

    #[test]
    fn test_editor_view_regression_duplicate_cursors_normalized_before_editing() {
        // probe_1d: duplicate empty cursors added via add_selection do not double-insert
        let mut view = EditorView::from_str("abcdef");
        view.set_primary_position(Position::new(0, 3));
        view.cursor_state
            .add_selection(Selection::cursor(Position::new(0, 3)));
        view.insert_text("X");
        assert_eq!(view.text(), "abcXdef");
        assert_eq!(
            view.cursor_state.selections,
            vec![Selection::cursor(Position::new(0, 4))]
        );

        // probe_1e: duplicate empty cursors do not drift on delete_backwards or delete_forwards
        let mut del_view = EditorView::from_str("abcdef");
        del_view.set_primary_position(Position::new(0, 4));
        del_view
            .cursor_state
            .add_selection(Selection::cursor(Position::new(0, 4)));
        del_view.delete_backwards();
        assert_eq!(del_view.text(), "abcef");
        assert_eq!(
            del_view.cursor_state.selections,
            vec![Selection::cursor(Position::new(0, 3))]
        );

        del_view
            .cursor_state
            .add_selection(Selection::cursor(Position::new(0, 3)));
        del_view.delete_forwards();
        assert_eq!(del_view.text(), "abcf");
        assert_eq!(
            del_view.cursor_state.selections,
            vec![Selection::cursor(Position::new(0, 3))]
        );
    }

    #[test]
    fn test_editor_view_regression_stale_scroll_top_after_fold_and_word_wrap_off() {
        // 1. Fold contraction clamps scroll_top & keeps MouseMove diagnostic hover, MouseDown, and ScrollWheel accurate
        let mut lines = vec!["fn outer() {".to_string()];
        for i in 1..=20 {
            lines.push(format!("    let x{} = {};", i, i));
        }
        lines.push("}".to_string());
        lines.push("let err = 1;".to_string());
        let src = lines.join("\n");

        let mut view = EditorView::from_str(&src).with_minimap(false);
        let registry = GrammarRegistry::default();
        let theme_svc = rustcode_platform::theme::ThemeService::default_dark_theme();
        view.set_language("rust", theme_svc, &registry);
        view.set_diagnostics(vec![EditorDiagnostic::new(
            Range::new(Position::new(22, 0), Position::new(22, 12)),
            DiagnosticSeverity::Error,
            "Tail error",
        )]);

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 300.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let gutter_w = view.gutter_width(&font);

        view.scroll_top = 18.0;
        view.set_primary_position(Position::new(5, 4));
        assert!(view.fold_at_cursor());
        // Visible lines now: line 0, line 22 -> max_scroll_top = 1.0
        assert_eq!(view.scroll_top, 1.0);

        // Render shows line 22 at row 1 (or row 0 if scroll_top == 1.0)
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("let err = 1;"));

        // Even if scroll_top becomes stale externally before an event, MouseMove clamps & triggers hover
        view.scroll_top = 18.0;
        let hover_ev = InputEvent::MouseMove(rustcode_platform::render::MouseMoveEvent {
            position: Point::new(bounds.min_x() + gutter_w + 16.0, bounds.min_y() + font.line_height * 0.5),
        });
        assert_eq!(view.handle_event(&hover_ev, bounds), EventResult::Consumed);
        assert!(view.hovered_diagnostic.is_some());
        assert_eq!(view.scroll_top, 1.0);

        // MouseDown with stale scroll_top clamps and lands on visibly rendered line 22
        view.scroll_top = 18.0;
        let click_ev = InputEvent::MouseDown(MouseEvent::new(
            Point::new(bounds.min_x() + gutter_w + 16.0, bounds.min_y() + font.line_height * 0.5),
            rustcode_platform::render::MouseButton::Left,
        ));
        assert_eq!(view.handle_event(&click_ev, bounds), EventResult::Consumed);
        assert_eq!(view.primary_position().line, 22);
        assert_eq!(view.scroll_top, 1.0);

        // ScrollWheel up from stale scroll_top moves immediately instead of remaining pinned at max
        view.scroll_top = 18.0;
        let scroll_up = InputEvent::ScrollWheel(rustcode_platform::render::ScrollEvent {
            position: Point::new(100.0, 100.0),
            delta_x: 0.0,
            delta_y: font.line_height,
            modifiers: Modifiers::none(),
        });
        assert_eq!(view.handle_event(&scroll_up, bounds), EventResult::Consumed);
        assert_eq!(view.scroll_top, 0.0);

        // 2. Switching wordWrap from WordWrapColumn to Off clamps scroll_top and keeps MouseMove hover working
        let long_line = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike";
        let mut wrap_view = EditorView::from_str(&format!("{}\nlet tail_err = 1;", long_line))
            .with_minimap(false)
            .with_word_wrap(WordWrapMode::WordWrapColumn)
            .with_word_wrap_column(10);
        wrap_view.set_diagnostics(vec![EditorDiagnostic::new(
            Range::new(Position::new(1, 0), Position::new(1, 12)),
            DiagnosticSeverity::Error,
            "Wrap tail error",
        )]);
        let mut wrap_scene = Scene::new();
        wrap_view.render(bounds, &theme, &font, &mut wrap_scene);
        wrap_view.scroll_top = 7.0;
        wrap_view.set_word_wrap(WordWrapMode::Off);
        assert_eq!(wrap_view.scroll_top, 1.0);
        let wrap_hover = InputEvent::MouseMove(rustcode_platform::render::MouseMoveEvent {
            position: Point::new(
                bounds.min_x() + wrap_view.gutter_width(&font) + 16.0,
                bounds.min_y() + font.line_height * 0.5,
            ),
        });
        assert_eq!(
            wrap_view.handle_event(&wrap_hover, bounds),
            EventResult::Consumed
        );
        assert!(wrap_view.hovered_diagnostic.is_some());
    }

    #[test]
    fn test_editor_view_regression_undo_redo_preserves_shifted_collapsed_fold() {
        let registry = GrammarRegistry::default();

        // 1. 3-function case: folding fn b(), inserting multiple lines in fn a(), undo & redo
        let src3 = "fn a() {\n    x\n}\nfn b() {\n    y\n}\nfn c() {\n    z\n}";
        let mut view = EditorView::from_str(src3);
        let theme_svc = rustcode_platform::theme::ThemeService::default_dark_theme();
        view.set_language("rust", theme_svc, &registry);
        assert!(view.folding_model.fold(3));
        assert_eq!(view.folding_model.collapsed.iter().copied().collect::<Vec<_>>(), vec![3]);

        view.set_primary_position(Position::new(1, 5));
        view.insert_text("\n    a\n    b\n    c");
        assert_eq!(view.folding_model.collapsed.iter().copied().collect::<Vec<_>>(), vec![6]);

        // Set scroll_top to the bottom of the expanded buffer to verify undo clamps scroll_top
        view.scroll_top = 8.0;
        assert!(view.undo());
        assert_eq!(view.folding_model.collapsed.iter().copied().collect::<Vec<_>>(), vec![3]);
        assert!(view.scroll_top <= 6.0);

        assert!(view.redo());
        assert_eq!(view.folding_model.collapsed.iter().copied().collect::<Vec<_>>(), vec![6]);

        // 2. 2-function case without fn c(): fn b() must stay collapsed at line 3 on undo (never silently unfolded)
        let src2 = "fn a() {\n    x\n}\nfn b() {\n    y\n}";
        let mut view2 = EditorView::from_str(src2);
        let theme_svc2 = rustcode_platform::theme::ThemeService::default_dark_theme();
        view2.set_language("rust", theme_svc2, &registry);
        assert!(view2.folding_model.fold(3));
        view2.set_primary_position(Position::new(1, 5));
        view2.insert_text("\n    a\n    b\n    c");
        assert_eq!(view2.folding_model.collapsed.iter().copied().collect::<Vec<_>>(), vec![6]);
        assert!(view2.undo());
        assert_eq!(view2.folding_model.collapsed.iter().copied().collect::<Vec<_>>(), vec![3]);
        assert!(view2.redo());
        assert_eq!(view2.folding_model.collapsed.iter().copied().collect::<Vec<_>>(), vec![6]);

        // 3. Multi-cursor newline edits above & between two collapsed folds
        let src_mc = "fn a() {\n    x\n}\nfn b() {\n    y\n}\nlet mid = 1;\nfn c() {\n    z\n}";
        let mut view_mc = EditorView::from_str(src_mc);
        let theme_svc_mc = rustcode_platform::theme::ThemeService::default_dark_theme();
        view_mc.set_language("rust", theme_svc_mc, &registry);
        assert!(view_mc.folding_model.fold(3));
        assert!(view_mc.folding_model.fold(7));
        view_mc.set_primary_position(Position::new(1, 5));
        view_mc
            .cursor_state
            .add_selection(Selection::cursor(Position::new(6, 12)));
        view_mc.insert_text("\n");
        assert_eq!(
            view_mc.folding_model.collapsed.iter().copied().collect::<Vec<_>>(),
            vec![4, 9]
        );
        assert!(view_mc.undo());
        assert_eq!(
            view_mc.folding_model.collapsed.iter().copied().collect::<Vec<_>>(),
            vec![3, 7]
        );
        assert!(view_mc.redo());
        assert_eq!(
            view_mc.folding_model.collapsed.iter().copied().collect::<Vec<_>>(),
            vec![4, 9]
        );
    }

    #[test]
    fn test_editor_view_regression_external_push_edit_after_undo_no_stale_redo_snapshot() {
        let mut view = EditorView::from_str("hello\nworld");

        // Edit 1: single cursor insert "A" at (0, 0)
        view.set_primary_position(Position::new(0, 0));
        view.insert_text("A");
        assert_eq!(view.text(), "Ahello\nworld");

        // Edit 2: multi-cursor insert "!!" on lines 0 and 1
        view.set_primary_position(Position::new(0, 6));
        view.cursor_state
            .add_selection(Selection::cursor(Position::new(1, 5)));
        view.insert_text("!!");
        assert_eq!(view.text(), "Ahello!!\nworld!!");
        assert_eq!(view.cursor_state.selections.len(), 2);

        // Undo Edit 2 -> puts Edit 2 snapshot on cursor_redo_snapshots
        assert!(view.undo());
        assert_eq!(view.text(), "Ahello\nworld");

        // External push_edit clears undo_stack.future; stale redo snapshot must NOT leak onto this edit
        {
            let mut buf = view.buffer.write();
            buf.insert(0, "EXT_");
            view.undo_stack.push_edit(TextEdit::insert(0, "EXT_"));
        }
        assert_eq!(view.text(), "EXT_Ahello\nworld");

        assert!(view.undo());
        assert_eq!(view.text(), "Ahello\nworld");
        assert_eq!(view.cursor_state.selections.len(), 1);

        assert!(view.redo());
        assert_eq!(view.text(), "EXT_Ahello\nworld");
        assert_eq!(
            view.cursor_state.selections.len(),
            1,
            "Stale redo snapshot from discarded Edit 2 leaked onto external Edit 3"
        );
        assert_eq!(view.primary_position(), Position::new(0, 4));
    }

    #[test]
    fn test_editor_view_regression_undo_redo_unfolds_anchor_and_active_lines() {
        let registry = GrammarRegistry::default();
        let src = "fn a() {\n    x\n}\nfn b() {\n    y\n}";
        let mut view = EditorView::from_str(src);
        let theme_svc = rustcode_platform::theme::ThemeService::default_dark_theme();
        view.set_language("rust", theme_svc, &registry);

        // Multi-line selection with anchor inside fn a() (line 1) and active inside fn b() (line 4)
        view.cursor_state.selections = vec![Selection::new(
            Position::new(1, 4),
            Position::new(4, 5),
        )];
        assert!(view.apply_text_edits(&[RangeEdit::new(
            Range::new(Position::new(4, 5), Position::new(4, 5)),
            "_edited",
        )]));
        assert_eq!(view.text(), "fn a() {\n    x\n}\nfn b() {\n    y_edited\n}");

        // Collapse fn a() (0..=2) before undoing
        assert!(view.folding_model.fold(0));
        assert!(view.folding_model.is_line_hidden(1));

        // Undo restores pre_sels with anchor at line 1 and must unfold fn a() so anchor is visible
        assert!(view.undo());
        assert_eq!(view.text(), src);
        assert_eq!(
            view.cursor_state.primary(),
            &Selection::new(Position::new(1, 4), Position::new(4, 5))
        );
        assert!(
            !view.folding_model.is_line_hidden(1),
            "Selection anchor line 1 remained hidden after undo"
        );
        assert!(!view.folding_model.is_collapsed(0));
        assert!(!view.folding_model.is_line_hidden(4));

        // Collapse fn b() (3..=5) and redo: active line 4 must also be unfolded
        assert!(view.folding_model.fold(3));
        assert!(view.folding_model.is_line_hidden(4));
        assert!(view.redo());
        assert!(!view.folding_model.is_line_hidden(4));
        assert!(!view.folding_model.is_collapsed(3));
    }

    #[test]
    fn test_editor_view_regression_non_default_font_metrics_hittest_gutter_and_hover() {
        let registry = GrammarRegistry::default();
        let src = "fn alpha() {\n    let a = 1;\n    let b = a + 2;\n    let diagnostic_here = b;\n}\n";
        let mut view = EditorView::from_str(src);
        let theme_svc = rustcode_platform::theme::ThemeService::default_dark_theme();
        view.set_language("rust", theme_svc, &registry);
        view.set_diagnostics(vec![EditorDiagnostic {
            range: Range::new(Position::new(3, 8), Position::new(3, 23)),
            severity: DiagnosticSeverity::Error,
            message: "bad variable".to_string(),
            source: Some("rustc".to_string()),
            code: Some("E0425".to_string()),
        }]);

        // Non-default 24px font: line_height=36.0, char_width=14.0, gutter_w=84.0
        let font = FontMetrics::new("monospace", 24.0);
        assert!((font.line_height - 36.0).abs() < f32::EPSILON);
        assert!((font.char_width - 14.0).abs() < f32::EPSILON);
        let gutter_w = view.gutter_width(&font);
        assert!((gutter_w - 84.0).abs() < f32::EPSILON);

        let theme = WorkbenchThemeStyles::dark_default();
        let bounds = Rect::from_xywh(40.0, 20.0, 700.0, 400.0);
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        // 1. MouseDown text hit-test on line 3, col 8
        let click_pos = Point::new(
            bounds.min_x() + gutter_w + 8.0 + 8.0 * font.char_width,
            bounds.min_y() + 3.5 * font.line_height,
        );
        let click_ev = InputEvent::MouseDown(MouseEvent {
            position: click_pos,
            button: rustcode_platform::render::MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        assert_eq!(view.handle_event(&click_ev, bounds), EventResult::Consumed);
        assert_eq!(view.primary_position(), Position::new(3, 8));

        // 2. MouseMove diagnostic hover on line 3, col 15
        let hover_pos = Point::new(
            bounds.min_x() + gutter_w + 8.0 + 15.0 * font.char_width,
            bounds.min_y() + 3.5 * font.line_height,
        );
        let hover_ev = InputEvent::MouseMove(rustcode_platform::render::MouseMoveEvent {
            position: hover_pos,
        });
        assert_eq!(view.handle_event(&hover_ev, bounds), EventResult::Consumed);
        assert!(view.hovered_diagnostic.is_some());

        // 3. MouseDown on fold chevron on line 0 (min_x + gutter_w - 8.0)
        let chevron_ev = InputEvent::MouseDown(MouseEvent {
            position: Point::new(
                bounds.min_x() + gutter_w - 8.0,
                bounds.min_y() + 0.5 * font.line_height,
            ),
            button: rustcode_platform::render::MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        assert_eq!(view.handle_event(&chevron_ev, bounds), EventResult::Consumed);
        assert_eq!(
            view.last_gutter_action,
            Some(EditorGutterAction::FoldToggled {
                line: 0,
                collapsed: true,
            })
        );
        assert!(view.folding_model.is_collapsed(0));
        assert!(view.breakpoints.is_empty());

        // Unfold and verify left-gutter breakpoint click still works
        assert!(view.folding_model.unfold(0));
        let bp_ev = InputEvent::MouseDown(MouseEvent {
            position: Point::new(bounds.min_x() + 10.0, bounds.min_y() + 0.5 * font.line_height),
            button: rustcode_platform::render::MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        assert_eq!(view.handle_event(&bp_ev, bounds), EventResult::Consumed);
        assert_eq!(
            view.last_gutter_action,
            Some(EditorGutterAction::BreakpointToggled {
                line: 1,
                added: true,
            })
        );

        // 4. ScrollWheel uses rendered font.line_height (36.0)
        let scroll_ev = InputEvent::ScrollWheel(rustcode_platform::render::ScrollEvent {
            position: click_pos,
            delta_x: 0.0,
            delta_y: -36.0,
            modifiers: Modifiers::default(),
        });
        assert_eq!(view.handle_event(&scroll_ev, bounds), EventResult::Consumed);
        assert!((view.scroll_top - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_editor_view_regression_unrendered_scroll_wheel_undo_clamps_all_wrap_modes() {
        let bounds = Rect::from_xywh(0.0, 0.0, 500.0, 300.0);
        for mode in [
            WordWrapMode::Off,
            WordWrapMode::On,
            WordWrapMode::WordWrapColumn,
            WordWrapMode::Bounded,
        ] {
            let mut view = EditorView::from_str("initial")
                .with_word_wrap(mode)
                .with_word_wrap_column(20);
            view.set_primary_position(Position::new(0, 7));

            let mut many_lines = String::new();
            for i in 1..=25 {
                many_lines.push_str(&format!("\nline {}", i));
            }
            view.insert_text(&many_lines);
            assert_eq!(view.buffer.read().line_count(), 26);

            // Dispatch ScrollWheel without any prior render()
            let scroll_ev = InputEvent::ScrollWheel(rustcode_platform::render::ScrollEvent {
                position: Point::new(100.0, 100.0),
                delta_x: 0.0,
                delta_y: -500.0,
                modifiers: Modifiers::default(),
            });
            assert_eq!(view.handle_event(&scroll_ev, bounds), EventResult::Consumed);
            assert!(
                view.scroll_top > 5.0,
                "mode={:?} should have scrolled down before undo",
                mode
            );

            // Undo contracts back to 1 line ("initial"); scroll_top must clamp to 0.0
            assert!(view.undo());
            assert_eq!(view.buffer.read().line_count(), 1);
            assert_eq!(
                view.scroll_top, 0.0,
                "mode={:?} left stale scroll_top={} after undo without prior render",
                mode, view.scroll_top
            );
        }

        // Also verify KeyDown with zero-sized Rect::default() does NOT seed last_render_bounds,
        // while WordWrapColumn and 1-char On/Bounded still clamp safely when never rendered.
        let mut col_view = EditorView::from_str("123456789012345678901234567890")
            .with_word_wrap(WordWrapMode::WordWrapColumn)
            .with_word_wrap_column(20);
        let key_ev = InputEvent::KeyDown(rustcode_platform::render::KeyEvent {
            key: KeyCode::ArrowRight,
            modifiers: Modifiers::default(),
            text: None,
        });
        assert_eq!(
            col_view.handle_event(&key_ev, Rect::default()),
            EventResult::Consumed
        );
        assert!(col_view.last_render_bounds.lock().is_none());
        col_view.scroll_top = 10.0;
        col_view.set_word_wrap_column(40);
        assert_eq!(col_view.scroll_top, 0.0);

        for mode in [WordWrapMode::On, WordWrapMode::Bounded] {
            let mut unrendered = EditorView::from_str("a")
                .with_word_wrap(mode)
                .with_word_wrap_column(20);
            unrendered.set_primary_position(Position::new(0, 1));
            unrendered.insert_text("\nb\nc\nd\ne");
            unrendered.scroll_top = 4.0;
            assert!(unrendered.undo());
            assert!(unrendered.last_render_bounds.lock().is_none());
            assert_eq!(unrendered.scroll_top, 0.0);
        }
    }
}

