//! Editor group and tab model mirroring VS Code's editor grid, and GPUI EditorAreaView component.

use rustcode_editor::EditorView;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorTab {
    pub id: String,
    pub uri: String,
    pub title: String,
    pub is_dirty: bool,
    pub is_preview: bool,
}

impl EditorTab {
    pub fn new(uri: &str, title: &str, is_preview: bool) -> Self {
        Self {
            id: uri.to_string(),
            uri: uri.to_string(),
            title: title.to_string(),
            is_dirty: false,
            is_preview,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EditorGroup {
    pub tabs: Vec<EditorTab>,
    pub active_tab_index: Option<usize>,
}

impl EditorGroup {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab_index: None,
        }
    }

    /// Open or activate a tab. Replaces current preview tab if not dirty.
    pub fn open_tab(&mut self, uri: &str, title: &str, is_preview: bool) -> usize {
        // Check if already open
        if let Some(idx) = self.tabs.iter().position(|t| t.uri == uri) {
            self.active_tab_index = Some(idx);
            if !is_preview && self.tabs[idx].is_preview {
                self.tabs[idx].is_preview = false;
            }
            return idx;
        }

        // If current active tab is preview and not dirty, replace it
        if let Some(idx) = self.active_tab_index {
            if let Some(tab) = self.tabs.get_mut(idx) {
                if tab.is_preview && !tab.is_dirty {
                    *tab = EditorTab::new(uri, title, is_preview);
                    return idx;
                }
            }
        }

        let new_tab = EditorTab::new(uri, title, is_preview);
        self.tabs.push(new_tab);
        let new_idx = self.tabs.len() - 1;
        self.active_tab_index = Some(new_idx);
        new_idx
    }

    /// Close tab by index. Returns false if index was out of bounds.
    pub fn close_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }

        self.tabs.remove(index);

        if self.tabs.is_empty() {
            self.active_tab_index = None;
        } else if let Some(active) = self.active_tab_index {
            if active >= self.tabs.len() {
                self.active_tab_index = Some(self.tabs.len() - 1);
            } else if active > index {
                self.active_tab_index = Some(active - 1);
            }
        }
        true
    }

    pub fn set_dirty(&mut self, index: usize, dirty: bool) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.is_dirty = dirty;
            if dirty {
                tab.is_preview = false;
            }
        }
    }

    pub fn active_tab(&self) -> Option<&EditorTab> {
        self.active_tab_index.and_then(|idx| self.tabs.get(idx))
    }
}

#[derive(Default)]
pub struct EditorArea {
    pub groups: Vec<EditorGroup>,
    pub active_group: usize,
}

impl EditorArea {
    pub fn new() -> Self {
        Self {
            groups: vec![EditorGroup::new()],
            active_group: 0,
        }
    }

    pub fn current_group(&mut self) -> &mut EditorGroup {
        &mut self.groups[self.active_group]
    }

    pub fn current_group_ref(&self) -> &EditorGroup {
        &self.groups[self.active_group]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAreaAction {
    TabSelected(usize),
    TabClosed(usize),
}

pub struct EditorAreaView {
    pub model: EditorArea,
    pub editors: HashMap<String, EditorView>,
    pub last_action: Option<EditorAreaAction>,
}

impl Default for EditorAreaView {
    fn default() -> Self {
        Self::new(EditorArea::new())
    }
}

impl EditorAreaView {
    pub fn new(model: EditorArea) -> Self {
        Self {
            model,
            editors: HashMap::new(),
            last_action: None,
        }
    }

    pub fn open_file(&mut self, uri: &str, title: &str, content: &str, is_preview: bool) {
        let group = self.model.current_group();
        group.open_tab(uri, title, is_preview);

        if !self.editors.contains_key(uri) {
            self.editors
                .insert(uri.to_string(), EditorView::from_str(content));
        }
    }

    pub fn active_editor_mut(&mut self) -> Option<&mut EditorView> {
        let uri = self.model.current_group().active_tab()?.uri.clone();
        self.editors.get_mut(&uri)
    }

    pub fn active_editor(&self) -> Option<&EditorView> {
        let uri = self.model.current_group_ref().active_tab()?.uri.clone();
        self.editors.get(&uri)
    }
}

impl View for EditorAreaView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        // Tab bar height: 35.0
        let tab_bar_h = 35.0;
        let tab_bar_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), tab_bar_h);

        // Tab bar background
        scene.push_quad(
            Quad::new(tab_bar_rect)
                .with_background(theme.tab_inactive_bg)
                .with_border_edges(theme.tab_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        let group = self.model.current_group_ref();
        let mut cur_x = bounds.min_x();

        // Render each tab
        for (idx, tab) in group.tabs.iter().enumerate() {
            let is_active = group.active_tab_index == Some(idx);
            let tab_w = (font.text_width(&tab.title) + 40.0).clamp(80.0, 220.0);
            let tab_rect = Rect::from_xywh(cur_x, bounds.min_y(), tab_w, tab_bar_h);

            let bg_color = if is_active {
                theme.tab_active_bg
            } else {
                theme.tab_inactive_bg
            };

            let mut tab_quad = Quad::new(tab_rect).with_background(bg_color);
            if is_active {
                tab_quad = tab_quad.with_border_edges(
                    theme.activity_bar_active_border,
                    Edges::new(2.0, 0.0, 0.0, 0.0),
                );
            } else {
                tab_quad = tab_quad.with_border_edges(
                    theme.tab_border,
                    Edges::new(0.0, 1.0, 0.0, 0.0),
                );
            }
            scene.push_quad(tab_quad);

            // Title text (italic if preview)
            let fg_color = if is_active {
                theme.tab_active_fg
            } else {
                theme.tab_inactive_fg
            };

            let title_text = if tab.is_dirty {
                format!("{} \u{25cf}", tab.title)
            } else {
                tab.title.clone()
            };

            let title_y = bounds.min_y() + (tab_bar_h - font.line_height) / 2.0;
            let title_x = cur_x + 10.0;
            let mut run = GlyphRun::new(
                title_text,
                Point::new(title_x, title_y),
                Rect::from_xywh(title_x, title_y, tab_w - 24.0, font.line_height),
                fg_color,
                font.font_size,
            );
            if tab.is_preview {
                run = run.with_style(false, true, false);
            }
            scene.push_text(run);

            // Close button 'x' on right
            let close_x = cur_x + tab_w - 18.0;
            scene.push_text(GlyphRun::new(
                "x".to_string(),
                Point::new(close_x, title_y),
                Rect::from_xywh(close_x, title_y, 14.0, font.line_height),
                fg_color,
                font.font_size * 0.85,
            ));

            cur_x += tab_w;
        }

        // Editor content below tabs
        let content_rect = Rect::from_xywh(
            bounds.min_x(),
            bounds.min_y() + tab_bar_h,
            bounds.width(),
            (bounds.height() - tab_bar_h).max(0.0),
        );

        if let Some(active_editor) = self.active_editor() {
            active_editor.render(content_rect, theme, font, scene);
        } else {
            // Empty editor background
            scene.push_quad(Quad::new(content_rect).with_background(theme.editor_background));
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let tab_bar_h = 35.0;

        match event {
            InputEvent::MouseDown(mouse) => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }
                if mouse.position.y < bounds.min_y() + tab_bar_h {
                    // Click within tab bar
                    let font = FontMetrics::default_monospace();
                    let group = self.model.current_group_ref();
                    let mut cur_x = bounds.min_x();

                    for (idx, tab) in group.tabs.iter().enumerate() {
                        let tab_w = (font.text_width(&tab.title) + 40.0).clamp(80.0, 220.0);
                        let tab_rect = Rect::from_xywh(cur_x, bounds.min_y(), tab_w, tab_bar_h);

                        if tab_rect.contains(mouse.position) {
                            // Check if close button clicked
                            if mouse.position.x >= cur_x + tab_w - 22.0 {
                                self.model.current_group().close_tab(idx);
                                let open_uris: std::collections::HashSet<_> = self.model.groups.iter().flat_map(|g| g.tabs.iter().map(|t| &t.uri)).collect();
                                self.editors.retain(|u, _| open_uris.contains(u));
                                self.last_action = Some(EditorAreaAction::TabClosed(idx));
                            } else {
                                self.model.current_group().active_tab_index = Some(idx);
                                self.last_action = Some(EditorAreaAction::TabSelected(idx));
                            }
                            return EventResult::Consumed;
                        }
                        cur_x += tab_w;
                    }
                    EventResult::Ignored
                } else {
                    // Click within active editor
                    let content_rect = Rect::from_xywh(
                        bounds.min_x(),
                        bounds.min_y() + tab_bar_h,
                        bounds.width(),
                        (bounds.height() - tab_bar_h).max(0.0),
                    );
                    if let Some(editor) = self.active_editor_mut() {
                        editor.handle_event(event, content_rect)
                    } else {
                        EventResult::Ignored
                    }
                }
            }
            _ => {
                let content_rect = Rect::from_xywh(
                    bounds.min_x(),
                    bounds.min_y() + tab_bar_h,
                    bounds.width(),
                    (bounds.height() - tab_bar_h).max(0.0),
                );
                if let Some(editor) = self.active_editor_mut() {
                    editor.handle_event(event, content_rect)
                } else {
                    EventResult::Ignored
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::render::MouseEvent;

    #[test]
    fn test_editor_tabs_and_groups() {
        let mut group = EditorGroup::new();
        assert_eq!(group.tabs.len(), 0);

        // Open preview tab 1
        group.open_tab("file:///a.rs", "a.rs", true);
        assert_eq!(group.tabs.len(), 1);
        assert!(group.active_tab().unwrap().is_preview);

        // Open preview tab 2 replaces preview tab 1
        group.open_tab("file:///b.rs", "b.rs", true);
        assert_eq!(group.tabs.len(), 1);
        assert_eq!(group.active_tab().unwrap().title, "b.rs");

        // Make it dirty -> becomes permanent
        group.set_dirty(0, true);
        assert!(!group.active_tab().unwrap().is_preview);

        // Now opening c.rs adds a new tab instead of replacing
        group.open_tab("file:///c.rs", "c.rs", true);
        assert_eq!(group.tabs.len(), 2);

        // Close tab 0
        group.close_tab(0);
        assert_eq!(group.tabs.len(), 1);
        assert_eq!(group.active_tab().unwrap().title, "c.rs");
    }

    #[test]
    fn test_editor_area_view_tabs_and_rendering() {
        let mut area_view = EditorAreaView::default();
        area_view.open_file(
            "file:///src/main.rs",
            "main.rs",
            "fn main() {\n    println!(\"Rust\");\n}\n",
            false,
        );
        area_view.open_file(
            "file:///src/lib.rs",
            "lib.rs",
            "pub fn hello() {}\n",
            true,
        );

        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let mut scene = Scene::new();
        area_view.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("main.rs"));
        assert!(scene.contains_text("lib.rs"));
        assert!(scene.contains_text("pub fn hello()"));

        // Click first tab to switch back to main.rs
        let click_ev = InputEvent::MouseDown(MouseEvent::new(
            Point::new(30.0, 15.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = area_view.handle_event(&click_ev, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(
            area_view.model.current_group_ref().active_tab().unwrap().title,
            "main.rs"
        );
    }
}
