//! Quick Input, Command Palette with fuzzy search, and GPUI CommandPaletteView component.

use rustcode_platform::render::{
    EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Scene, Shadow,
    View, WorkbenchThemeStyles,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaletteItemKind {
    Command,
    File,
    WorkspaceSymbol,
    DocumentSymbol,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickInputMode {
    Command,
    File,
    WorkspaceSymbol,
    DocumentSymbol,
    Line,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteItem {
    pub command_id: String,
    pub label: String,
    pub category: Option<String>,
    pub keybinding: Option<String>,
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub line: Option<usize>,
    #[serde(default)]
    pub column: Option<usize>,
    #[serde(default = "default_item_kind")]
    pub item_kind: PaletteItemKind,
}

fn default_item_kind() -> PaletteItemKind {
    PaletteItemKind::Command
}

impl Default for PaletteItem {
    fn default() -> Self {
        Self {
            command_id: String::new(),
            label: String::new(),
            category: None,
            keybinding: None,
            path: None,
            line: None,
            column: None,
            item_kind: PaletteItemKind::Command,
        }
    }
}

impl PaletteItem {
    pub fn command(
        command_id: impl Into<String>,
        label: impl Into<String>,
        category: Option<String>,
        keybinding: Option<String>,
    ) -> Self {
        Self {
            command_id: command_id.into(),
            label: label.into(),
            category,
            keybinding,
            path: None,
            line: None,
            column: None,
            item_kind: PaletteItemKind::Command,
        }
    }

    pub fn file(path: PathBuf, relative_label: impl Into<String>) -> Self {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        Self {
            command_id: format!("file:{}", path.to_string_lossy()),
            label: file_name,
            category: Some(relative_label.into()),
            keybinding: None,
            path: Some(path),
            line: None,
            column: None,
            item_kind: PaletteItemKind::File,
        }
    }

    pub fn symbol(
        name: impl Into<String>,
        container: Option<String>,
        path: PathBuf,
        line: usize,
        column: usize,
    ) -> Self {
        let name_str = name.into();
        let cat = match container {
            Some(c) => format!("{} ({}:{})", c, path.to_string_lossy(), line),
            None => format!("{}:{}", path.to_string_lossy(), line),
        };
        Self {
            command_id: format!("symbol:{}:{}:{}", path.to_string_lossy(), line, column),
            label: name_str,
            category: Some(cat),
            keybinding: None,
            path: Some(path),
            line: Some(line),
            column: Some(column),
            item_kind: PaletteItemKind::WorkspaceSymbol,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandPalette {
    pub items: Vec<PaletteItem>,
    pub query: String,
    pub filtered: Vec<usize>,
    pub selected_index: usize,
    pub mode: QuickInputMode,
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
            mode: QuickInputMode::Command,
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

    pub fn open_mode(&mut self, mode: QuickInputMode) {
        self.mode = mode;
        self.query = match mode {
            QuickInputMode::Command => ">".to_string(),
            QuickInputMode::WorkspaceSymbol => "#".to_string(),
            QuickInputMode::DocumentSymbol => "@".to_string(),
            QuickInputMode::Line => ":".to_string(),
            QuickInputMode::File => String::new(),
        };
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
        let (detected_mode, target_query) = if let Some(stripped) = self.query.strip_prefix('>') {
            (QuickInputMode::Command, stripped.trim().to_lowercase())
        } else if let Some(stripped) = self.query.strip_prefix('#') {
            (QuickInputMode::WorkspaceSymbol, stripped.trim().to_lowercase())
        } else if let Some(stripped) = self.query.strip_prefix('@') {
            (QuickInputMode::DocumentSymbol, stripped.trim().to_lowercase())
        } else if let Some(stripped) = self.query.strip_prefix(':') {
            (QuickInputMode::Line, stripped.trim().to_lowercase())
        } else {
            (QuickInputMode::File, self.query.trim().to_lowercase())
        };

        self.mode = detected_mode;
        self.filtered.clear();
        self.selected_index = 0;

        let has_files = self.items.iter().any(|i| i.item_kind == PaletteItemKind::File);

        for (idx, item) in self.items.iter().enumerate() {
            // Mode filter
            let kind_matches = match detected_mode {
                QuickInputMode::Command => item.item_kind == PaletteItemKind::Command,
                QuickInputMode::WorkspaceSymbol => item.item_kind == PaletteItemKind::WorkspaceSymbol,
                QuickInputMode::DocumentSymbol => item.item_kind == PaletteItemKind::DocumentSymbol,
                QuickInputMode::Line => item.item_kind == PaletteItemKind::Line,
                QuickInputMode::File => {
                    if has_files {
                        item.item_kind == PaletteItemKind::File
                    } else {
                        // Fallback: match any item (e.g. in command-only tests)
                        true
                    }
                }
            };

            if !kind_matches {
                continue;
            }

            if target_query.is_empty() {
                self.filtered.push(idx);
                continue;
            }

            // Fuzzy match against category and label
            let full_text = match &item.category {
                Some(cat) => format!("{}: {}", cat, item.label).to_lowercase(),
                None => item.label.to_lowercase(),
            };

            if fuzzy_match(&full_text, &target_query) || fuzzy_match(&item.label.to_lowercase(), &target_query) {
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
    OpenFile {
        path: PathBuf,
        line: Option<usize>,
        column: Option<usize>,
    },
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

        // Prompt string & query
        let (prompt_str, display_query) = if let Some(stripped) = self.model.query.strip_prefix('>') {
            ("> ", stripped)
        } else if let Some(stripped) = self.model.query.strip_prefix('#') {
            ("# ", stripped)
        } else if let Some(stripped) = self.model.query.strip_prefix('@') {
            ("@ ", stripped)
        } else if let Some(stripped) = self.model.query.strip_prefix(':') {
            (": ", stripped)
        } else {
            ("> ", self.model.query.as_str())
        };

        let prompt_w = font.text_width(prompt_str);

        // Prompt
        scene.push_text(GlyphRun::new(
            prompt_str.to_string(),
            Point::new(prompt_x, text_y),
            Rect::from_xywh(prompt_x, text_y, prompt_w, font.line_height),
            theme.command_palette_fg,
            font.font_size,
        ));

        // Query text
        let query_x = prompt_x + prompt_w;
        let query_w = font.text_width(display_query);
        if !display_query.is_empty() {
            scene.push_text(GlyphRun::new(
                display_query.to_string(),
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
            let label_w = font.text_width(&display_str);

            scene.push_text(GlyphRun::new(
                display_str,
                Point::new(label_x, item_y),
                Rect::from_xywh(label_x, item_y, label_w, font.line_height),
                theme.command_palette_fg,
                font.font_size,
            ));

            // Keybinding badge if present
            if let Some(kb) = &item.keybinding {
                let kb_w = font.text_width(kb);
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
                        if let Some(path) = &item.path {
                            self.last_action = Some(PaletteAction::OpenFile {
                                path: path.clone(),
                                line: item.line,
                                column: item.column,
                            });
                        } else {
                            self.last_action = Some(PaletteAction::Execute(item.command_id.clone()));
                        }
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
                KeyCode::Space if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                    self.model.query.push(' ');
                    self.model.refilter();
                    EventResult::Consumed
                }
                KeyCode::Char(c) if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                    let ch = if key.modifiers.shift { c.to_ascii_uppercase() } else { *c };
                    self.model.query.push(ch);
                    self.model.refilter();
                    EventResult::Consumed
                }
                _ => EventResult::Ignored,
            },
            InputEvent::MouseDown(mouse) => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }
                let input_h = 36.0;
                let row_h = 28.0;
                if mouse.position.y > bounds.min_y() + input_h {
                    let idx = ((mouse.position.y - (bounds.min_y() + input_h + 1.0)) / row_h)
                        .floor() as usize;
                    let maybe_action = if let Some(item) = self.model.results().get(idx) {
                        if let Some(path) = &item.path {
                            Some(PaletteAction::OpenFile {
                                path: path.clone(),
                                line: item.line,
                                column: item.column,
                            })
                        } else {
                            Some(PaletteAction::Execute(item.command_id.clone()))
                        }
                    } else {
                        None
                    };

                    if let Some(act) = maybe_action {
                        self.model.selected_index = idx;
                        self.last_action = Some(act);
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
            PaletteItem::command(
                "workbench.action.reloadWindow",
                "Reload Window",
                Some("Developer".to_string()),
                None,
            ),
            PaletteItem::command(
                "editor.action.formatDocument",
                "Format Document",
                None,
                Some("Shift+Alt+F".to_string()),
            ),
            PaletteItem::command(
                "git.commit",
                "Commit",
                Some("Git".to_string()),
                None,
            ),
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
    fn test_command_palette_unicode_case_folding() {
        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem::command(
                "workbench.action.openFile",
                "Öffnen",
                Some("Datei".to_string()),
                None,
            ),
            PaletteItem::command(
                "workbench.action.closeFile",
                "Schließen",
                Some("Datei".to_string()),
                None,
            ),
        ]);

        // Query with lowercase umlaut matching uppercase umlaut
        palette.set_query(">öffnen");
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].label, "Öffnen");

        // Query category with lowercase
        palette.set_query(">datei");
        assert_eq!(palette.results().len(), 2);

        // Query lowercase matching German eszett
        palette.set_query(">schließen");
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].label, "Schließen");
    }

    #[test]
    fn test_command_palette_view_interaction() {
        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem::command(
                "workbench.action.toggleSidebar",
                "Toggle Sidebar",
                Some("View".to_string()),
                Some("Ctrl+B".to_string()),
            ),
            PaletteItem::command(
                "workbench.action.openSettings",
                "Open Settings",
                Some("Preferences".to_string()),
                Some("Ctrl+,".to_string()),
            ),
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

        // Test space typing in command palette
        let mut space_palette = CommandPalette::new();
        space_palette.set_items(vec![
            PaletteItem::command(
                "workbench.action.toggleSidebar",
                "Toggle Sidebar",
                Some("View".to_string()),
                None,
            ),
        ]);
        let mut space_view = CommandPaletteView::new(space_palette);
        for key in [KeyCode::Char('v'), KeyCode::Space, KeyCode::Char('t')] {
            let res = space_view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(key, Modifiers::none())),
                bounds,
            );
            assert_eq!(res, EventResult::Consumed);
        }
        assert_eq!(space_view.model.query, "v t");
        assert_eq!(space_view.model.results().len(), 1);

        // Click outside bounds should be ignored
        let outside_ev = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(50.0, 20.0), // outside bounds
            rustcode_platform::render::MouseButton::Left,
        ));
        assert_eq!(space_view.handle_event(&outside_ev, bounds), EventResult::Ignored);
    }

    #[test]
    fn test_quick_open_multi_mode_and_open_file() {
        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem::command(
                "workbench.action.reloadWindow",
                "Reload Window",
                Some("Developer".to_string()),
                None,
            ),
            PaletteItem::file(
                PathBuf::from("/project/backend/src/main.rs"),
                "backend/src/main.rs",
            ),
            PaletteItem::file(
                PathBuf::from("/project/frontend/src/app.tsx"),
                "frontend/src/app.tsx",
            ),
            PaletteItem::symbol(
                "process_order",
                Some("OrderService".to_string()),
                PathBuf::from("/project/backend/src/order.rs"),
                42,
                8,
            ),
        ]);

        // 1. Default / prefix-less: file search
        palette.set_query("main");
        assert_eq!(palette.mode, QuickInputMode::File);
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].label, "main.rs");

        // 2. Command mode with '>'
        palette.set_query(">reload");
        assert_eq!(palette.mode, QuickInputMode::Command);
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].command_id, "workbench.action.reloadWindow");

        // 3. Workspace symbol mode with '#'
        palette.set_query("#order");
        assert_eq!(palette.mode, QuickInputMode::WorkspaceSymbol);
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].label, "process_order");

        // 4. Test Enter emits OpenFile action
        let mut view = CommandPaletteView::new(palette);
        view.model.set_query("app");
        let bounds = Rect::from_xywh(100.0, 50.0, 600.0, 300.0);
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            view.last_action,
            Some(PaletteAction::OpenFile {
                path: PathBuf::from("/project/frontend/src/app.tsx"),
                line: None,
                column: None,
            })
        );
    }
}
