//! GPUI EditorView component rendering text lines, line numbers, cursor, selections, and routing input.

use crate::completion::{AutocompleteWidget, CompletionItem, CompletionItemKind};
use crate::cursor::{CursorState, Position};
use crate::document::TextDocument;
use crate::model::piece_table::PieceTable;
use crate::model::undo_redo::{TextEdit, UndoRedoStack};
use crate::syntax::{GrammarRegistry, SyntaxHighlighter};
use crate::viewmodel::{DiagnosticSeverity, EditorDiagnostic};
use parking_lot::{Mutex, RwLock};
use rustcode_platform::fs::{FileMetadata, FileSystemError, FileSystemService};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Rgba,
    Scene, Shadow, View, WorkbenchThemeStyles,
};
use rustcode_platform::theme::ColorTheme;
use std::sync::Arc;

pub type CompletionProviderFn =
    Arc<dyn Fn(&str, Position, Option<char>) -> Vec<CompletionItem> + Send + Sync>;

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
}

impl EditorView {
    pub fn new(buffer: Arc<RwLock<PieceTable>>) -> Self {
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
        }
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

    pub fn with_syntax(self, highlighter: SyntaxHighlighter) -> Self {
        *self.syntax_highlighter.lock() = Some(highlighter);
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

    pub fn set_document(&mut self, doc: Arc<RwLock<TextDocument>>) {
        self.buffer = doc.read().buffer.clone();
        self.document = Some(doc);
    }

    pub fn set_language(&mut self, lang_id: &str, theme: ColorTheme, registry: &GrammarRegistry) {
        let sh = SyntaxHighlighter::for_language(lang_id, theme, registry);
        *self.syntax_highlighter.lock() = Some(sh);
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
        ((digits + 2) as f32 * font.char_width).max(40.0)
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
        let mut buf = self.buffer.write();
        let committed = self.autocomplete.commit(
            &mut buf,
            &mut self.cursor_state,
            &mut self.undo_stack,
        );
        drop(buf);

        if committed.is_some() {
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

    pub fn delete_selection(&mut self) -> bool {
        let sel = self.cursor_state.primary().clone();
        if sel.is_empty() {
            return false;
        }

        let (start_pos, end_pos) = if sel.anchor <= sel.active {
            (sel.anchor, sel.active)
        } else {
            (sel.active, sel.anchor)
        };

        let mut buf = self.buffer.write();
        let start_offset = buf.position_to_offset(start_pos.line, start_pos.column);
        let end_offset = buf.position_to_offset(end_pos.line, end_pos.column);
        let del_len = end_offset.saturating_sub(start_offset);
        if del_len > 0 {
            let deleted = buf.delete(start_offset, del_len);
            self.undo_stack
                .push_edit(TextEdit::delete(start_offset, &deleted));
            let (new_line, new_col) = buf.offset_to_position(start_offset);
            self.cursor_state.set_primary(Position::new(new_line, new_col));
            drop(buf);

            if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
                sh.invalidate_from(start_pos.line);
            }
            if let Some(doc) = &self.document {
                doc.write().mark_edited();
            }
            true
        } else {
            self.cursor_state.set_primary(start_pos);
            false
        }
    }

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let sel = self.cursor_state.primary().clone();
        let mut buf = self.buffer.write();

        if !sel.is_empty() {
            let (start_pos, end_pos) = if sel.anchor <= sel.active {
                (sel.anchor, sel.active)
            } else {
                (sel.active, sel.anchor)
            };
            let start_offset = buf.position_to_offset(start_pos.line, start_pos.column);
            let end_offset = buf.position_to_offset(end_pos.line, end_pos.column);
            let del_len = end_offset.saturating_sub(start_offset);
            let deleted = if del_len > 0 {
                buf.delete(start_offset, del_len)
            } else {
                String::new()
            };
            buf.insert(start_offset, text);
            self.undo_stack
                .push_edit(TextEdit::replace(start_offset, &deleted, text));
            let (new_line, new_col) = buf.offset_to_position(start_offset + text.len());
            self.cursor_state.set_primary(Position::new(new_line, new_col));
            drop(buf);

            if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
                sh.invalidate_from(start_pos.line);
            }
            if let Some(doc) = &self.document {
                doc.write().mark_edited();
            }
            return;
        }

        let pos = self.primary_position();
        let offset = buf.position_to_offset(pos.line, pos.column);

        buf.insert(offset, text);
        self.undo_stack.push_edit(TextEdit::insert(offset, text));

        let (new_line, new_col) = buf.offset_to_position(offset + text.len());
        self.cursor_state.set_primary(Position::new(new_line, new_col));
        drop(buf);

        if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
            sh.invalidate_from(pos.line);
        }
        if let Some(doc) = &self.document {
            doc.write().mark_edited();
        }
    }

    pub fn delete_backwards(&mut self) {
        if self.delete_selection() {
            return;
        }

        let pos = self.primary_position();
        let mut buf = self.buffer.write();
        let offset = buf.position_to_offset(pos.line, pos.column);

        if offset == 0 {
            return;
        }

        let prev_offset = buf.prev_char_boundary(offset);
        let del_len = offset - prev_offset;
        let deleted = buf.delete(prev_offset, del_len);
        self.undo_stack
            .push_edit(TextEdit::delete(prev_offset, &deleted));

        let (new_line, new_col) = buf.offset_to_position(prev_offset);
        self.cursor_state.set_primary(Position::new(new_line, new_col));
        drop(buf);

        if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
            sh.invalidate_from(new_line);
        }
        if let Some(doc) = &self.document {
            doc.write().mark_edited();
        }
    }

    pub fn delete_forwards(&mut self) {
        if self.delete_selection() {
            return;
        }

        let pos = self.primary_position();
        let mut buf = self.buffer.write();
        let offset = buf.position_to_offset(pos.line, pos.column);

        if offset >= buf.len() {
            return;
        }

        let next_offset = buf.next_char_boundary(offset);
        let del_len = next_offset - offset;
        let deleted = buf.delete(offset, del_len);
        self.undo_stack.push_edit(TextEdit::delete(offset, &deleted));
        drop(buf);

        if let Some(sh) = self.syntax_highlighter.lock().as_mut() {
            sh.invalidate_from(pos.line);
        }
        if let Some(doc) = &self.document {
            doc.write().mark_edited();
        }
    }


    pub fn move_cursor_left(&mut self, select: bool) {
        let pos = self.primary_position();
        let buf = self.buffer.read();
        let offset = buf.position_to_offset(pos.line, pos.column);

        if offset > 0 {
            let prev_offset = buf.prev_char_boundary(offset);
            let (new_line, new_col) = buf.offset_to_position(prev_offset);
            let target = Position::new(new_line, new_col);
            if select {
                self.cursor_state.primary_mut().active = target;
            } else {
                self.cursor_state.set_primary(target);
            }
        } else if !select {
            self.cursor_state.set_primary(pos);
        }
    }

    pub fn move_cursor_right(&mut self, select: bool) {
        let pos = self.primary_position();
        let buf = self.buffer.read();
        let offset = buf.position_to_offset(pos.line, pos.column);

        if offset < buf.len() {
            let next_offset = buf.next_char_boundary(offset);
            let (new_line, new_col) = buf.offset_to_position(next_offset);
            let target = Position::new(new_line, new_col);
            if select {
                self.cursor_state.primary_mut().active = target;
            } else {
                self.cursor_state.set_primary(target);
            }
        } else if !select {
            self.cursor_state.set_primary(pos);
        }
    }

    pub fn move_cursor_up(&mut self, select: bool) {
        let pos = self.primary_position();
        if pos.line > 0 {
            let target_line = pos.line - 1;
            let buf = self.buffer.read();
            let content = buf.get_line_content(target_line).unwrap_or_default();
            let mut target_col = pos.column.min(content.len());
            while target_col > 0 && !content.is_char_boundary(target_col) {
                target_col -= 1;
            }
            let target = Position::new(target_line, target_col);
            if select {
                self.cursor_state.primary_mut().active = target;
            } else {
                self.cursor_state.set_primary(target);
            }
        }
    }

    pub fn move_cursor_down(&mut self, select: bool) {
        let pos = self.primary_position();
        let buf = self.buffer.read();
        let line_count = buf.line_count();
        if pos.line + 1 < line_count {
            let target_line = pos.line + 1;
            let content = buf.get_line_content(target_line).unwrap_or_default();
            let mut target_col = pos.column.min(content.len());
            while target_col > 0 && !content.is_char_boundary(target_col) {
                target_col -= 1;
            }
            let target = Position::new(target_line, target_col);
            if select {
                self.cursor_state.primary_mut().active = target;
            } else {
                self.cursor_state.set_primary(target);
            }
        }
    }

    pub fn move_cursor_home(&mut self, select: bool) {
        let pos = self.primary_position();
        let target = Position::new(pos.line, 0);
        if select {
            self.cursor_state.primary_mut().active = target;
        } else {
            self.cursor_state.set_primary(target);
        }
    }

    pub fn move_cursor_end(&mut self, select: bool) {
        let pos = self.primary_position();
        let line_len = self
            .buffer
            .read()
            .get_line_content(pos.line)
            .map_or(0, |l| l.len());
        let target = Position::new(pos.line, line_len);
        if select {
            self.cursor_state.primary_mut().active = target;
        } else {
            self.cursor_state.set_primary(target);
        }
    }

    pub fn undo(&mut self) -> bool {
        let mut buf = self.buffer.write();
        if let Some(target_offset) = self.undo_stack.undo(&mut buf) {
            let (line, col) = buf.offset_to_position(target_offset);
            self.cursor_state.set_primary(Position::new(line, col));
            drop(buf);

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
        let mut buf = self.buffer.write();
        if let Some(target_offset) = self.undo_stack.redo(&mut buf) {
            let (line, col) = buf.offset_to_position(target_offset);
            self.cursor_state.set_primary(Position::new(line, col));
            drop(buf);

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

}

impl View for EditorView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        let buf = self.buffer.read();
        let line_count = buf.line_count();

        // 1. Editor background
        scene.push_quad(Quad::new(bounds).with_background(theme.editor_background));

        // 2. Gutter background and border
        let gutter_w = self.gutter_width(font);
        let gutter_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), gutter_w, bounds.height());
        scene.push_quad(
            Quad::new(gutter_rect)
                .with_background(theme.editor_gutter_background)
                .with_border_edges(
                    theme.editor_gutter_border,
                    Edges::new(0.0, 1.0, 0.0, 0.0),
                ),
        );

        // 3. Compute visible line range
        let start_line = (self.scroll_top.floor() as usize).min(line_count.saturating_sub(1));
        let visible_lines = (bounds.height() / font.line_height).ceil() as usize + 1;
        let end_line = (start_line + visible_lines).min(line_count);

        let primary_pos = self.primary_position();

        // 4. Current line highlight
        if self.is_focused && primary_pos.line >= start_line && primary_pos.line < end_line {
            let line_y = bounds.min_y() + (primary_pos.line as f32 - self.scroll_top) * font.line_height;
            let hl_rect = Rect::from_xywh(
                bounds.min_x() + gutter_w,
                line_y,
                bounds.width() - gutter_w,
                font.line_height,
            );
            scene.push_quad(Quad::new(hl_rect).with_background(theme.editor_current_line));
        }

        // 5. Render visible lines
        let text_area_x = bounds.min_x() + gutter_w + 8.0;

        // Warm up tokenizer cache sequentially up to start_line so multiline state is preserved
        {
            let mut highlighter_guard = self.syntax_highlighter.lock();
            if let Some(highlighter) = highlighter_guard.as_mut() {
                let last_cached = highlighter.cache_len();
                if last_cached < start_line {
                    for l in last_cached..start_line {
                        if let Some(content) = buf.get_line_content(l) {
                            highlighter.get_line_tokens(l, &content);
                        }
                    }
                }
            }
        }

        for line_idx in start_line..end_line {
            let line_y = bounds.min_y() + (line_idx as f32 - self.scroll_top) * font.line_height;

            // 5a. Gutter line number
            let line_num_str = (line_idx + 1).to_string();
            let num_w = font.text_width(&line_num_str);
            let num_x = bounds.min_x() + gutter_w - num_w - 6.0;
            let num_color = if line_idx == primary_pos.line {
                theme.editor_line_number_active
            } else {
                theme.editor_line_number
            };

            scene.push_text(GlyphRun::new(
                line_num_str,
                Point::new(num_x, line_y),
                Rect::from_xywh(num_x, line_y, num_w, font.line_height),
                num_color,
                font.font_size,
            ));

            let line_content = buf.get_line_content(line_idx).unwrap_or_default();

            // 5b. Selections on this line
            for sel in &self.cursor_state.selections {
                if !sel.is_empty() {
                    let (min_pos, max_pos) = if sel.anchor <= sel.active {
                        (sel.anchor, sel.active)
                    } else {
                        (sel.active, sel.anchor)
                    };

                    if line_idx >= min_pos.line && line_idx <= max_pos.line {
                        let start_col = if line_idx == min_pos.line { min_pos.column } else { 0 };
                        let end_col = if line_idx == max_pos.line { max_pos.column } else { line_content.len() };

                        if end_col > start_col {
                            let start_visual = line_content[..start_col.min(line_content.len())].chars().count();
                            let end_visual = line_content[..end_col.min(line_content.len())].chars().count();
                            let sel_x = text_area_x + (start_visual as f32 - self.scroll_left) * font.char_width;
                            let sel_w = ((end_visual - start_visual) as f32 * font.char_width).max(4.0);
                            let sel_rect = Rect::from_xywh(sel_x, line_y, sel_w, font.line_height);
                            scene.push_quad(Quad::new(sel_rect).with_background(theme.editor_selection));
                        }
                    }
                }
            }

            // 5c. Text content
            if !line_content.is_empty() {
                let mut rendered_tokens = false;
                let mut highlighter_guard = self.syntax_highlighter.lock();
                if let Some(highlighter) = highlighter_guard.as_mut() {
                    let tokens = highlighter.get_line_tokens(line_idx, &line_content);
                    if !tokens.is_empty() {
                        rendered_tokens = true;
                        let mut last_byte = 0;
                        for span in tokens {
                            let span_start = span.start.min(line_content.len());
                            let span_end = span.end.min(line_content.len());

                            // Emit gap if valid char boundaries
                            if span_start > last_byte
                                && line_content.is_char_boundary(last_byte)
                                && line_content.is_char_boundary(span_start)
                            {
                                let gap_text = &line_content[last_byte..span_start];
                                let gap_start_vis = line_content[..last_byte].chars().count();
                                let gap_vis_w = gap_text.chars().count() as f32 * font.char_width;
                                let gap_x = text_area_x + (gap_start_vis as f32 - self.scroll_left) * font.char_width;
                                scene.push_text(GlyphRun::new(
                                    gap_text.to_string(),
                                    Point::new(gap_x, line_y),
                                    Rect::from_xywh(gap_x, line_y, gap_vis_w, font.line_height),
                                    theme.editor_foreground,
                                    font.font_size,
                                ));
                                last_byte = span_start;
                            }

                            // Emit token span if valid
                            if span_end > last_byte
                                && span_start >= last_byte
                                && line_content.is_char_boundary(span_start)
                                && line_content.is_char_boundary(span_end)
                            {
                                let span_text = &line_content[span_start..span_end];
                                let span_start_vis = line_content[..span_start].chars().count();
                                let span_vis_w = span_text.chars().count() as f32 * font.char_width;
                                let span_x = text_area_x + (span_start_vis as f32 - self.scroll_left) * font.char_width;
                                let span_color = span.color.unwrap_or(theme.editor_foreground);
                                scene.push_text(
                                    GlyphRun::new(
                                        span_text.to_string(),
                                        Point::new(span_x, line_y),
                                        Rect::from_xywh(span_x, line_y, span_vis_w, font.line_height),
                                        span_color,
                                        font.font_size,
                                    )
                                    .with_style(
                                        span.metadata.is_bold(),
                                        span.metadata.is_italic(),
                                        span.metadata.is_underline(),
                                    ),
                                );
                                last_byte = span_end;
                            } else {
                                last_byte = last_byte.max(span_end);
                            }
                        }

                        // Emit remaining tail text
                        if last_byte < line_content.len()
                            && line_content.is_char_boundary(last_byte)
                        {
                            let tail_text = &line_content[last_byte..];
                            let tail_start_vis = line_content[..last_byte].chars().count();
                            let tail_vis_w = tail_text.chars().count() as f32 * font.char_width;
                            let tail_x = text_area_x + (tail_start_vis as f32 - self.scroll_left) * font.char_width;
                            scene.push_text(GlyphRun::new(
                                tail_text.to_string(),
                                Point::new(tail_x, line_y),
                                Rect::from_xywh(tail_x, line_y, tail_vis_w, font.line_height),
                                theme.editor_foreground,
                                font.font_size,
                            ));
                        }
                    }
                }
                drop(highlighter_guard);

                if !rendered_tokens {
                    let text_w = font.text_width(&line_content);
                    let text_x = text_area_x - self.scroll_left * font.char_width;
                    scene.push_text(GlyphRun::new(
                        line_content.clone(),
                        Point::new(text_x, line_y),
                        Rect::from_xywh(text_x, line_y, text_w, font.line_height),
                        theme.editor_foreground,
                        font.font_size,
                    ));
                }
            }


            // 5d. Diagnostics squiggly underlines on this line
            for diag in self.diagnostics_for_line(line_idx) {
                let start_col = if line_idx == diag.range.start.line {
                    diag.range.start.column
                } else {
                    0
                };
                let end_col = if line_idx == diag.range.end.line {
                    diag.range.end.column
                } else {
                    line_content.len()
                };
                let safe_start = safe_char_boundary(&line_content, start_col);
                let safe_end = {
                    let end_aligned = safe_char_boundary(&line_content, end_col);
                    if end_aligned <= safe_start {
                        next_char_boundary(&line_content, safe_start)
                    } else {
                        end_aligned
                    }
                };

                let start_vis = line_content[..safe_start].chars().count();
                let end_vis = line_content[..safe_end.min(line_content.len())].chars().count().max(start_vis + 1);

                let sq_x = text_area_x + (start_vis as f32 - self.scroll_left) * font.char_width;
                let sq_w = ((end_vis - start_vis) as f32 * font.char_width).max(4.0);
                let sq_y = line_y + font.line_height - 2.5;

                let color = match diag.severity {
                    DiagnosticSeverity::Error => theme.editor_error_foreground,
                    DiagnosticSeverity::Warning => theme.editor_warning_foreground,
                    DiagnosticSeverity::Information => theme.editor_info_foreground,
                    DiagnosticSeverity::Hint => theme.editor_hint_foreground,
                };

                render_squiggly_wave(scene, sq_x, sq_x + sq_w, sq_y, color);
            }

            // 5e. Cursors on this line
            if self.is_focused {
                for sel in &self.cursor_state.selections {
                    if sel.active.line == line_idx {
                        let safe_col = safe_char_boundary(&line_content, sel.active.column);
                        let visual_col = line_content[..safe_col].chars().count();
                        let cursor_x = text_area_x
                            + (visual_col as f32 - self.scroll_left) * font.char_width;
                        let cursor_rect =
                            Rect::from_xywh(cursor_x, line_y, 2.0, font.line_height);
                        scene.push_quad(Quad::new(cursor_rect).with_background(theme.editor_cursor));
                    }
                }
            }
        }

        // 6. Autocomplete Popover Overlay
        if self.autocomplete.is_visible {
            let buf = self.buffer.read();
            let line_content = buf.get_line_content(primary_pos.line).unwrap_or_default();
            let safe_col = safe_char_boundary(&line_content, primary_pos.column);
            let visual_col = line_content[..safe_col].chars().count();
            let cursor_x = text_area_x + (visual_col as f32 - self.scroll_left) * font.char_width;
            let cursor_y = bounds.min_y() + (primary_pos.line as f32 - self.scroll_top) * font.line_height;
            drop(buf);

            self.autocomplete.render(
                Point::new(cursor_x, cursor_y),
                bounds,
                theme,
                font,
                scene,
            );
        }

        // 7. Error hover tooltip overlay
        if let (Some(diag), Some(mouse_pt)) = (&self.hovered_diagnostic, self.hover_position) {
            let header_text = diag.formatted_header();
            let message_text = &diag.message;

            let header_w = font.text_width(&header_text);
            let message_w = font.text_width(message_text);
            let card_rect = Self::hover_card_bounds(diag, mouse_pt, bounds, font);
            let card_x = card_rect.min_x();
            let card_y = card_rect.min_y();
            let card_h = card_rect.height();
            scene.push_shadow(Shadow {
                bounds: card_rect,
                offset: Point::new(0.0, 4.0),
                blur: 10.0,
                spread: 0.0,
                color: Rgba::new(0.0, 0.0, 0.0, 0.35),
            });

            let sev_color = match diag.severity {
                DiagnosticSeverity::Error => theme.editor_error_foreground,
                DiagnosticSeverity::Warning => theme.editor_warning_foreground,
                DiagnosticSeverity::Information => theme.editor_info_foreground,
                DiagnosticSeverity::Hint => theme.editor_hint_foreground,
            };

            scene.push_quad(
                Quad::new(card_rect)
                    .with_background(theme.command_palette_bg)
                    .with_border_edges(theme.panel_border, Edges::all(1.0))
                    .with_corner_radius(4.0),
            );

            let bar_rect = Rect::from_xywh(card_x, card_y, 4.0, card_h);
            scene.push_quad(
                Quad::new(bar_rect)
                    .with_background(sev_color)
                    .with_corner_radius(2.0),
            );

            let header_pt = Point::new(card_x + 10.0, card_y + 6.0);
            scene.push_text(
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
            scene.push_text(GlyphRun::new(
                message_text.clone(),
                msg_pt,
                Rect::from_xywh(msg_pt.x, msg_pt.y, message_w, font.line_height),
                theme.editor_foreground,
                font.font_size,
            ));
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let font = FontMetrics::default_monospace();
        let gutter_w = self.gutter_width(&font);

        match event {
            InputEvent::MouseDown(mouse) => {
                self.hovered_diagnostic = None;
                self.hover_position = None;
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }
                self.is_focused = true;

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

                if mouse.position.x >= bounds.min_x() + gutter_w {
                    let buf = self.buffer.read();
                    let line_count = buf.line_count();

                    let clicked_line = ((mouse.position.y - bounds.min_y()) / font.line_height
                        + self.scroll_top)
                        .floor()
                        .max(0.0) as usize;
                    let line_idx = clicked_line.min(line_count.saturating_sub(1));

                    let text_area_x = bounds.min_x() + gutter_w + 8.0;
                    let clicked_char_col = ((mouse.position.x - text_area_x
                        + self.scroll_left * font.char_width)
                        / font.char_width)
                        .round()
                        .max(0.0) as usize;

                    let line_content = buf.get_line_content(line_idx).unwrap_or_default();
                    let total_chars = line_content.chars().count();
                    let char_idx = clicked_char_col.min(total_chars);
                    let col = if char_idx >= total_chars {
                        line_content.len()
                    } else {
                        line_content.char_indices().nth(char_idx).map_or(line_content.len(), |(b, _)| b)
                    };

                    let target = Position::new(line_idx, col);
                    if mouse.modifiers.shift {
                        self.cursor_state.primary_mut().active = target;
                    } else {
                        self.cursor_state.set_primary(target);
                    }
                    EventResult::Consumed
                } else {
                    // Click in gutter: select entire line
                    let buf = self.buffer.read();
                    let line_count = buf.line_count();
                    let clicked_line = ((mouse.position.y - bounds.min_y()) / font.line_height
                        + self.scroll_top)
                        .floor()
                        .max(0.0) as usize;
                    let line_idx = clicked_line.min(line_count.saturating_sub(1));
                    let line_len = buf.get_line_content(line_idx).map_or(0, |l| l.len());
                    self.cursor_state.primary_mut().anchor = Position::new(line_idx, 0);
                    self.cursor_state.primary_mut().active = Position::new(line_idx, line_len);
                    EventResult::Consumed
                }
            }
            InputEvent::ScrollWheel(scroll) => {
                let line_count = self.buffer.read().line_count();
                self.scroll_top = (self.scroll_top - scroll.delta_y / font.line_height)
                    .clamp(0.0, line_count.saturating_sub(1) as f32);
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => {
                self.hovered_diagnostic = None;
                self.hover_position = None;
                if !self.is_focused {
                    return EventResult::Ignored;
                }

                // Check manual trigger suggest keybinding: Ctrl+Space
                if key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta && key.key == KeyCode::Space {
                    self.trigger_suggest(None);
                    return EventResult::Consumed;
                }

                // Check undo / redo key combinations
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
                        let last_col = buf.get_line_content(last_line).map_or(0, |l| l.len());
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
                                    let start = trigger_pos.column.min(content.len());
                                    let end = new_pos.column.min(content.len());
                                    let prefix = if end >= start
                                        && content.is_char_boundary(start)
                                        && content.is_char_boundary(end)
                                    {
                                        &content[start..end]
                                    } else {
                                        ""
                                    };
                                    self.autocomplete.update_filter(prefix);
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
                                    let start = trigger_pos.column.min(content.len());
                                    let end = new_pos.column.min(content.len());
                                    let prefix = if end >= start
                                        && content.is_char_boundary(start)
                                        && content.is_char_boundary(end)
                                    {
                                        &content[start..end]
                                    } else {
                                        ""
                                    };
                                    self.autocomplete.update_filter(prefix);
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

                if mouse.position.x >= bounds.min_x() + gutter_w {
                    let buf = self.buffer.read();
                    let line_count = buf.line_count();
                    let line = ((mouse.position.y - bounds.min_y()) / font.line_height + self.scroll_top)
                        .floor()
                        .max(0.0) as usize;
                    if line < line_count {
                        let text_area_x = bounds.min_x() + gutter_w + 8.0;
                        let char_col = ((mouse.position.x - text_area_x + self.scroll_left * font.char_width)
                            / font.char_width)
                            .floor()
                            .max(0.0) as usize;
                        let line_content = buf.get_line_content(line).unwrap_or_default();
                        let total_chars = line_content.chars().count();
                        let char_idx = char_col.min(total_chars);
                        let col = if char_idx >= total_chars {
                            line_content.len()
                        } else {
                            line_content.char_indices().nth(char_idx).map_or(line_content.len(), |(b, _)| b)
                        };
                        drop(buf);

                        let pos = Position::new(line, col);
                        if let Some(diag) = self.diagnostic_at_position(pos) {
                            let diag_cloned = diag.clone();
                            self.hovered_diagnostic = Some(diag_cloned);
                            self.hover_position = Some(mouse.position);
                            return EventResult::Consumed;
                        }
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
            _ => EventResult::Ignored,
        }
    }
}

fn safe_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn next_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    idx += 1;
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

fn render_squiggly_wave(scene: &mut Scene, start_x: f32, end_x: f32, y: f32, color: Rgba) {
    if !start_x.is_finite() || !end_x.is_finite() || start_x >= end_x {
        return;
    }
    let wave_step: f32 = 3.0;
    let mut cur_x = start_x;
    let mut up = false;
    while cur_x < end_x {
        let seg_w = wave_step.min(end_x - cur_x);
        if seg_w <= 0.0 {
            break;
        }
        let seg_y = if up { y - 1.2 } else { y + 0.8 };
        scene.push_quad(Quad::new(Rect::from_xywh(cur_x, seg_y, seg_w, 1.2)).with_background(color));
        cur_x += seg_w;
        up = !up;
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

        // Move cursor right past '🦀' (4 bytes)
        view.move_cursor_right(false);
        assert_eq!(view.primary_position(), Position::new(0, 4));

        // Backspace deletes '🦀' cleanly
        let bs_ev = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Backspace,
            Modifiers::none(),
        ));
        view.handle_event(&bs_ev, bounds);
        assert_eq!(view.text(), "hello");
        assert_eq!(view.primary_position(), Position::new(0, 0));

        // Undo restores '🦀hello' with cursor at (0, 4)
        assert!(view.undo());
        assert_eq!(view.text(), "🦀hello");
        assert_eq!(view.primary_position(), Position::new(0, 4));

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
}

