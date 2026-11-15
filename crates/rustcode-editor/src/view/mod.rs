//! GPUI EditorView component rendering text lines, line numbers, cursor, selections, and routing input.

use crate::cursor::{CursorState, Position};
use crate::model::piece_table::PieceTable;
use crate::model::undo_redo::{TextEdit, UndoRedoStack};
use parking_lot::RwLock;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use std::sync::Arc;

pub struct EditorView {
    pub buffer: Arc<RwLock<PieceTable>>,
    pub cursor_state: CursorState,
    pub undo_stack: UndoRedoStack,
    pub scroll_top: f32,
    pub scroll_left: f32,
    pub is_focused: bool,
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
        }
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

    // --- Editing & Movement Operations ---

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let pos = self.primary_position();
        let mut buf = self.buffer.write();
        let offset = buf.position_to_offset(pos.line, pos.column);

        buf.insert(offset, text);
        self.undo_stack.push_edit(TextEdit::insert(offset, text));

        let (new_line, new_col) = buf.offset_to_position(offset + text.len());
        self.cursor_state.set_primary(Position::new(new_line, new_col));
    }

    pub fn delete_backwards(&mut self) {
        let pos = self.primary_position();
        let mut buf = self.buffer.write();
        let offset = buf.position_to_offset(pos.line, pos.column);

        if offset == 0 {
            return;
        }

        let del_len = 1;
        let del_offset = offset - del_len;
        let deleted = buf.delete(del_offset, del_len);
        self.undo_stack
            .push_edit(TextEdit::delete(del_offset, &deleted));

        let (new_line, new_col) = buf.offset_to_position(del_offset);
        self.cursor_state.set_primary(Position::new(new_line, new_col));
    }

    pub fn delete_forwards(&mut self) {
        let pos = self.primary_position();
        let mut buf = self.buffer.write();
        let offset = buf.position_to_offset(pos.line, pos.column);

        if offset >= buf.len() {
            return;
        }

        let deleted = buf.delete(offset, 1);
        self.undo_stack.push_edit(TextEdit::delete(offset, &deleted));
    }

    pub fn move_cursor_left(&mut self) {
        let pos = self.primary_position();
        let buf = self.buffer.read();
        let offset = buf.position_to_offset(pos.line, pos.column);

        if offset > 0 {
            let (new_line, new_col) = buf.offset_to_position(offset - 1);
            self.cursor_state.set_primary(Position::new(new_line, new_col));
        }
    }

    pub fn move_cursor_right(&mut self) {
        let pos = self.primary_position();
        let buf = self.buffer.read();
        let offset = buf.position_to_offset(pos.line, pos.column);

        if offset < buf.len() {
            let (new_line, new_col) = buf.offset_to_position(offset + 1);
            self.cursor_state.set_primary(Position::new(new_line, new_col));
        }
    }

    pub fn move_cursor_up(&mut self) {
        let pos = self.primary_position();
        if pos.line > 0 {
            let target_line = pos.line - 1;
            let line_len = self
                .buffer
                .read()
                .get_line_content(target_line)
                .map_or(0, |l| l.len());
            let target_col = pos.column.min(line_len);
            self.cursor_state
                .set_primary(Position::new(target_line, target_col));
        }
    }

    pub fn move_cursor_down(&mut self) {
        let pos = self.primary_position();
        let line_count = self.buffer.read().line_count();
        if pos.line + 1 < line_count {
            let target_line = pos.line + 1;
            let line_len = self
                .buffer
                .read()
                .get_line_content(target_line)
                .map_or(0, |l| l.len());
            let target_col = pos.column.min(line_len);
            self.cursor_state
                .set_primary(Position::new(target_line, target_col));
        }
    }

    pub fn move_cursor_home(&mut self) {
        let pos = self.primary_position();
        self.cursor_state
            .set_primary(Position::new(pos.line, 0));
    }

    pub fn move_cursor_end(&mut self) {
        let pos = self.primary_position();
        let line_len = self
            .buffer
            .read()
            .get_line_content(pos.line)
            .map_or(0, |l| l.len());
        self.cursor_state
            .set_primary(Position::new(pos.line, line_len));
    }

    pub fn undo(&mut self) -> bool {
        let mut buf = self.buffer.write();
        let ok = self.undo_stack.undo(&mut buf);
        if ok {
            let (line, col) = buf.offset_to_position(buf.len());
            self.cursor_state.set_primary(Position::new(line, col));
        }
        ok
    }

    pub fn redo(&mut self) -> bool {
        let mut buf = self.buffer.write();
        let ok = self.undo_stack.redo(&mut buf);
        if ok {
            let (line, col) = buf.offset_to_position(buf.len());
            self.cursor_state.set_primary(Position::new(line, col));
        }
        ok
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

        for line_idx in start_line..end_line {
            let line_y = bounds.min_y() + (line_idx as f32 - self.scroll_top) * font.line_height;

            // 5a. Gutter line number
            let line_num_str = (line_idx + 1).to_string();
            let num_w = line_num_str.len() as f32 * font.char_width;
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

            // 5b. Selections on this line
            for sel in &self.cursor_state.selections {
                if !sel.is_empty() {
                    let (min_pos, max_pos) = if sel.anchor <= sel.active {
                        (sel.anchor, sel.active)
                    } else {
                        (sel.active, sel.anchor)
                    };

                    if line_idx >= min_pos.line && line_idx <= max_pos.line {
                        let line_len = buf.get_line_content(line_idx).map_or(0, |l| l.len());
                        let start_col = if line_idx == min_pos.line { min_pos.column } else { 0 };
                        let end_col = if line_idx == max_pos.line { max_pos.column } else { line_len };

                        if end_col > start_col {
                            let sel_x = text_area_x + (start_col as f32 - self.scroll_left) * font.char_width;
                            let sel_w = ((end_col - start_col) as f32 * font.char_width).max(4.0);
                            let sel_rect = Rect::from_xywh(sel_x, line_y, sel_w, font.line_height);
                            scene.push_quad(Quad::new(sel_rect).with_background(theme.editor_selection));
                        }
                    }
                }
            }

            // 5c. Text content
            if let Some(content) = buf.get_line_content(line_idx) {
                if !content.is_empty() {
                    let text_w = content.len() as f32 * font.char_width;
                    let text_x = text_area_x - self.scroll_left * font.char_width;
                    scene.push_text(GlyphRun::new(
                        content,
                        Point::new(text_x, line_y),
                        Rect::from_xywh(text_x, line_y, text_w, font.line_height),
                        theme.editor_foreground,
                        font.font_size,
                    ));
                }
            }

            // 5d. Cursors on this line
            if self.is_focused {
                for sel in &self.cursor_state.selections {
                    if sel.active.line == line_idx {
                        let cursor_x = text_area_x
                            + (sel.active.column as f32 - self.scroll_left) * font.char_width;
                        let cursor_rect =
                            Rect::from_xywh(cursor_x, line_y, 2.0, font.line_height);
                        scene.push_quad(Quad::new(cursor_rect).with_background(theme.editor_cursor));
                    }
                }
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let font = FontMetrics::default_monospace();
        let gutter_w = self.gutter_width(&font);

        match event {
            InputEvent::MouseDown(mouse) => {
                self.is_focused = true;
                if mouse.position.x >= bounds.min_x() + gutter_w {
                    let buf = self.buffer.read();
                    let line_count = buf.line_count();

                    let clicked_line = ((mouse.position.y - bounds.min_y()) / font.line_height
                        + self.scroll_top)
                        .floor()
                        .max(0.0) as usize;
                    let line_idx = clicked_line.min(line_count.saturating_sub(1));

                    let text_area_x = bounds.min_x() + gutter_w + 8.0;
                    let clicked_col = ((mouse.position.x - text_area_x
                        + self.scroll_left * font.char_width)
                        / font.char_width)
                        .round()
                        .max(0.0) as usize;

                    let line_len = buf.get_line_content(line_idx).map_or(0, |l| l.len());
                    let col = clicked_col.min(line_len);

                    let target = Position::new(line_idx, col);
                    if mouse.modifiers.shift {
                        self.cursor_state.primary_mut().active = target;
                    } else {
                        self.cursor_state.set_primary(target);
                    }
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::ScrollWheel(scroll) => {
                let line_count = self.buffer.read().line_count();
                self.scroll_top = (self.scroll_top - scroll.delta_y / font.line_height)
                    .clamp(0.0, line_count.saturating_sub(1) as f32);
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => {
                if !self.is_focused {
                    return EventResult::Ignored;
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
                    }
                }

                match &key.key {
                    KeyCode::ArrowLeft => {
                        self.move_cursor_left();
                        EventResult::Consumed
                    }
                    KeyCode::ArrowRight => {
                        self.move_cursor_right();
                        EventResult::Consumed
                    }
                    KeyCode::ArrowUp => {
                        self.move_cursor_up();
                        EventResult::Consumed
                    }
                    KeyCode::ArrowDown => {
                        self.move_cursor_down();
                        EventResult::Consumed
                    }
                    KeyCode::Home => {
                        self.move_cursor_home();
                        EventResult::Consumed
                    }
                    KeyCode::End => {
                        self.move_cursor_end();
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
                    KeyCode::Char(c) if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                        let s = c.to_string();
                        self.insert_text(&s);
                        EventResult::Consumed
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
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
