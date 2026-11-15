//! Quick Input, Command Palette with fuzzy search, and GPUI CommandPaletteView component.

use rustcode_platform::render::{
    EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Scene, Shadow,
    View, WorkbenchThemeStyles,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteItem {
    pub command_id: String,
    pub label: String,
    pub category: Option<String>,
    pub keybinding: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CommandPalette {
    pub items: Vec<PaletteItem>,
    pub query: String,
    pub filtered: Vec<usize>,
    pub selected_index: usize,
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandPalette {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            query: String::new(),
            filtered: Vec::new(),
            selected_index: 0,
        }
    }

    pub fn set_items(&mut self, items: Vec<PaletteItem>) {
        self.items = items;
        self.refilter();
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.refilter();
    }

    pub fn results(&self) -> Vec<&PaletteItem> {
        self.filtered.iter().map(|&idx| &self.items[idx]).collect()
    }

    pub fn selected_item(&self) -> Option<&PaletteItem> {
        self.filtered.get(self.selected_index).map(|&idx| &self.items[idx])
    }

    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.filtered.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.filtered.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    pub fn refilter(&mut self) {
        let q = self.query.trim().to_ascii_lowercase();
        self.filtered.clear();
        self.selected_index = 0;

        if q.is_empty() {
            self.filtered.extend(0..self.items.len());
            return;
        }

        // Fuzzy match against label and category
        for (idx, item) in self.items.iter().enumerate() {
            let full_text = match &item.category {
                Some(cat) => format!("{}: {}", cat, item.label).to_ascii_lowercase(),
                None => item.label.to_ascii_lowercase(),
            };

            if fuzzy_match(&full_text, &q) {
                self.filtered.push(idx);
            }
        }
    }
}

/// Simple subsequence fuzzy matcher.
fn fuzzy_match(target: &str, pattern: &str) -> bool {
    let mut pattern_chars = pattern.chars().peekable();
    for c in target.chars() {
        if let Some(&pc) = pattern_chars.peek() {
            if c == pc {
                pattern_chars.next();
            }
        } else {
            return true;
        }
    }
    pattern_chars.peek().is_none()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    Execute(String),
    Close,
}

pub struct CommandPaletteView {
    pub model: CommandPalette,
    pub last_action: Option<PaletteAction>,
}

impl Default for CommandPaletteView {
    fn default() -> Self {
        Self::new(CommandPalette::new())
    }
}

impl CommandPaletteView {
    pub fn new(model: CommandPalette) -> Self {
        Self {
            model,
            last_action: None,
        }
    }
}

impl View for CommandPaletteView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        // Shadow behind popup
        scene.push_shadow(Shadow {
            bounds,
            offset: Point::new(0.0, 6.0),
            blur: 16.0,
            spread: 0.0,
            color: rustcode_platform::render::Rgba::new(0.0, 0.0, 0.0, 0.4),
        });

        // Popup background with rounded border
        scene.push_quad(
            Quad::new(bounds)
                .with_background(theme.command_palette_bg)
                .with_border(theme.command_palette_border, 1.0)
                .with_corner_radius(6.0),
        );

        // Input container (height 36.0)
        let input_h = 36.0;
        let prompt_x = bounds.min_x() + 12.0;
        let text_y = bounds.min_y() + (input_h - font.line_height) / 2.0;

        // Prompt "> "
        scene.push_text(GlyphRun::new(
            "> ".to_string(),
            Point::new(prompt_x, text_y),
            Rect::from_xywh(prompt_x, text_y, 2.0 * font.char_width, font.line_height),
            theme.command_palette_fg,
            font.font_size,
        ));

        // Query text
        let query_x = prompt_x + 2.0 * font.char_width;
        let query_w = self.model.query.len() as f32 * font.char_width;
        if !self.model.query.is_empty() {
            scene.push_text(GlyphRun::new(
                self.model.query.clone(),
                Point::new(query_x, text_y),
                Rect::from_xywh(query_x, text_y, query_w, font.line_height),
                theme.command_palette_fg,
                font.font_size,
            ));
        }

        // Blinking cursor after query
        let cursor_x = query_x + query_w;
        let cursor_rect = Rect::from_xywh(cursor_x, text_y, 2.0, font.line_height);
        scene.push_quad(Quad::new(cursor_rect).with_background(theme.editor_cursor));

        // Divider line below input
        let div_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y() + input_h, bounds.width(), 1.0);
        scene.push_quad(Quad::new(div_rect).with_background(theme.command_palette_border));

        // Result rows
        let row_h = 28.0;
        let results = self.model.results();
        let max_visible = ((bounds.height() - input_h - 1.0) / row_h).floor() as usize;

        for (idx, item) in results.iter().take(max_visible).enumerate() {
            let row_y = bounds.min_y() + input_h + 1.0 + idx as f32 * row_h;
            let row_rect = Rect::from_xywh(bounds.min_x() + 4.0, row_y, bounds.width() - 8.0, row_h);

            let is_selected = idx == self.model.selected_index;
            if is_selected {
                scene.push_quad(
                    Quad::new(row_rect)
                        .with_background(theme.command_palette_selected_bg)
                        .with_corner_radius(4.0),
                );
            }

            // Category + label
            let display_str = match &item.category {
                Some(cat) => format!("{}: {}", cat, item.label),
                None => item.label.clone(),
            };
            let label_x = bounds.min_x() + 14.0;
            let item_y = row_y + (row_h - font.line_height) / 2.0;
            let label_w = display_str.len() as f32 * font.char_width;

            scene.push_text(GlyphRun::new(
                display_str,
                Point::new(label_x, item_y),
                Rect::from_xywh(label_x, item_y, label_w, font.line_height),
                theme.command_palette_fg,
                font.font_size,
            ));

            // Keybinding badge if present
            if let Some(kb) = &item.keybinding {
                let kb_w = kb.len() as f32 * font.char_width;
                let kb_x = bounds.max_x() - 14.0 - kb_w;
                scene.push_text(GlyphRun::new(
                    kb.clone(),
                    Point::new(kb_x, item_y),
                    Rect::from_xywh(kb_x, item_y, kb_w, font.line_height),
                    theme.tab_inactive_fg,
                    font.font_size * 0.9,
                ));
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::KeyDown(key) => match &key.key {
                KeyCode::ArrowDown => {
                    self.model.select_next();
                    EventResult::Consumed
                }
                KeyCode::ArrowUp => {
                    self.model.select_prev();
                    EventResult::Consumed
                }
                KeyCode::Enter => {
                    if let Some(item) = self.model.selected_item() {
                        self.last_action = Some(PaletteAction::Execute(item.command_id.clone()));
                        EventResult::Consumed
                    } else {
                        EventResult::Ignored
                    }
                }
                KeyCode::Escape => {
                    self.last_action = Some(PaletteAction::Close);
                    EventResult::Consumed
                }
                KeyCode::Backspace => {
                    if !self.model.query.is_empty() {
                        self.model.query.pop();
                        self.model.refilter();
                        EventResult::Consumed
                    } else {
                        EventResult::Ignored
                    }
                }
                KeyCode::Char(c) if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                    self.model.query.push(*c);
                    self.model.refilter();
                    EventResult::Consumed
                }
                _ => EventResult::Ignored,
            },
            InputEvent::MouseDown(mouse) => {
                let input_h = 36.0;
                let row_h = 28.0;
                if mouse.position.y > bounds.min_y() + input_h {
                    let idx = ((mouse.position.y - (bounds.min_y() + input_h + 1.0)) / row_h)
                        .floor() as usize;
                    if let Some(cmd_id) = self.model.results().get(idx).map(|item| item.command_id.clone()) {
                        self.model.selected_index = idx;
                        self.last_action = Some(PaletteAction::Execute(cmd_id));
                        return EventResult::Consumed;
                    }
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::KeyEvent;

    #[test]
    fn test_command_palette_fuzzy_search() {
        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem {
                command_id: "workbench.action.reloadWindow".to_string(),
                label: "Reload Window".to_string(),
                category: Some("Developer".to_string()),
                keybinding: None,
            },
            PaletteItem {
                command_id: "editor.action.formatDocument".to_string(),
                label: "Format Document".to_string(),
                category: None,
                keybinding: Some("Shift+Alt+F".to_string()),
            },
            PaletteItem {
                command_id: "git.commit".to_string(),
                label: "Commit".to_string(),
                category: Some("Git".to_string()),
                keybinding: None,
            },
        ]);

        assert_eq!(palette.results().len(), 3);

        palette.set_query("format");
        assert_eq!(palette.results().len(), 1);
        assert_eq!(
            palette.selected_item().unwrap().command_id,
            "editor.action.formatDocument"
        );

        palette.set_query("dev rel");
        assert_eq!(palette.results().len(), 1);
        assert_eq!(
            palette.selected_item().unwrap().command_id,
            "workbench.action.reloadWindow"
        );
    }

    #[test]
    fn test_command_palette_view_interaction() {
        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem {
                command_id: "workbench.action.toggleSidebar".to_string(),
                label: "Toggle Sidebar".to_string(),
                category: Some("View".to_string()),
                keybinding: Some("Ctrl+B".to_string()),
            },
            PaletteItem {
                command_id: "workbench.action.openSettings".to_string(),
                label: "Open Settings".to_string(),
                category: Some("Preferences".to_string()),
                keybinding: Some("Ctrl+,".to_string()),
            },
        ]);

        let mut view = CommandPaletteView::new(palette);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let bounds = Rect::from_xywh(100.0, 50.0, 600.0, 300.0);
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("Toggle Sidebar"));
        assert!(scene.contains_text("Open Settings"));

        // Type "set"
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('s'), Modifiers::none())),
            bounds,
        );
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('e'), Modifiers::none())),
            bounds,
        );
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('t'), Modifiers::none())),
            bounds,
        );

        assert_eq!(view.model.results().len(), 1);
        assert_eq!(
            view.model.selected_item().unwrap().command_id,
            "workbench.action.openSettings"
        );

        // Press Enter to execute
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            view.last_action,
            Some(PaletteAction::Execute("workbench.action.openSettings".to_string()))
        );
    }
}
