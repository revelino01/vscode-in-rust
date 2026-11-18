//! Editor group and tab model mirroring VS Code's editor grid, and GPUI EditorAreaView component.

use parking_lot::RwLock;
use rustcode_editor::{
    DiffEditorView, EditorView, GrammarRegistry, Position, Range, RangeEdit, TextDocument,
};
use rustcode_ext_host::webview::{ExtHostWebviewManager, WebviewOptions};
use rustcode_platform::fs::{FileMetadata, FileSystemError, FileSystemService};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use rustcode_platform::theme::ThemeService;
use rustcode_protocols::lsp::WorkspaceEdit;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use crate::settings_ui::SettingsUiView;
use crate::webview::WebviewEditorView;
use rustcode_platform::configuration::ConfigurationService;


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
    pub diff_editors: HashMap<String, DiffEditorView>,
    pub webviews: HashMap<String, WebviewEditorView>,
    pub settings_views: HashMap<String, SettingsUiView>,
    pub grammar_registry: Arc<GrammarRegistry>,
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
            diff_editors: HashMap::new(),
            webviews: HashMap::new(),
            settings_views: HashMap::new(),
            grammar_registry: Arc::new(GrammarRegistry::new()),
            last_action: None,
        }
    }

    pub fn open_settings(&mut self, config: Option<ConfigurationService>) {
        let uri = "rustcode://settings";
        let group = self.model.current_group();
        group.open_tab(uri, "Settings", false);
        let view = self
            .settings_views
            .entry(uri.to_string())
            .or_default();
        view.state.open();
        if let Some(cfg) = config {
            view.set_config(cfg);
        }
    }

    pub fn close_tab(&mut self, index: usize) -> bool {
        let closed = self.model.current_group().close_tab(index);
        if closed {
            let open_uris: std::collections::HashSet<_> = self
                .model
                .groups
                .iter()
                .flat_map(|g| g.tabs.iter().map(|t| t.uri.clone()))
                .collect();
            self.editors.retain(|u, _| open_uris.contains(u));
            self.diff_editors.retain(|u, _| open_uris.contains(u));
            self.webviews.retain(|u, _| open_uris.contains(u));
            self.settings_views.retain(|u, _| open_uris.contains(u));
            self.last_action = Some(EditorAreaAction::TabClosed(index));
        }
        closed
    }

    pub fn open_diff(&mut self, uri: &str, title: &str, original: &str, modified: &str) {
        let group = self.model.current_group();
        group.open_tab(uri, title, false);
        self.diff_editors
            .insert(uri.to_string(), DiffEditorView::new(title, original, modified));
    }

    pub fn open_webview(
        &mut self,
        id: &str,
        title: &str,
        view_type: &str,
        html: &str,
        options: WebviewOptions,
        ext_host_manager: Option<ExtHostWebviewManager>,
    ) {
        let group = self.model.current_group();
        group.open_tab(id, title, false);
        self.webviews.insert(
            id.to_string(),
            WebviewEditorView::new(id, view_type, title, html, options, ext_host_manager),
        );
    }

    pub fn open_file(&mut self, uri: &str, title: &str, content: &str, is_preview: bool) {
        let group = self.model.current_group();
        group.open_tab(uri, title, is_preview);

        if !self.editors.contains_key(uri) {
            let path_buf = uri
                .strip_prefix("file://")
                .map(PathBuf::from)
                .or_else(|| Some(PathBuf::from(uri)));


            let grammar = if let Some(p) = &path_buf {
                self.grammar_registry.get_for_path(p)
            } else {
                self.grammar_registry.get_by_id("plaintext").unwrap()
            };

            let doc = Arc::new(RwLock::new(TextDocument::new(
                content,
                path_buf,
                Some(grammar.language_id()),
            )));

            let mut editor = EditorView::from_str(content).with_document(doc);
            let theme = ThemeService::default_dark_theme();
            editor.set_language(grammar.language_id(), theme, &self.grammar_registry);

            self.editors.insert(uri.to_string(), editor);
        }
    }

    pub fn save_active_file(
        &mut self,
        fs: &FileSystemService,
    ) -> Result<FileMetadata, FileSystemError> {
        let uri = self
            .model
            .current_group()
            .active_tab()
            .map(|t| t.uri.clone())
            .ok_or_else(|| {
                FileSystemError::NotFound(PathBuf::from("no_active_tab"))
            })?;

        if let Some(editor) = self.editors.get_mut(&uri) {
            let meta = editor.save_sync(fs)?;
            if let Some(idx) = self.model.current_group().active_tab_index {
                self.model.current_group().set_dirty(idx, false);
            }
            Ok(meta)
        } else {
            Err(FileSystemError::NotFound(PathBuf::from(&uri)))
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

    pub fn apply_text_edits(&mut self, uri: &str, edits: &[RangeEdit]) -> bool {
        if let Some(editor) = self.editors.get_mut(uri) {
            let changed = editor.apply_text_edits(edits);
            if changed {
                for group in &mut self.model.groups {
                    for tab in &mut group.tabs {
                        if tab.uri == uri {
                            tab.is_dirty = true;
                            tab.is_preview = false;
                        }
                    }
                }
            }
            changed
        } else {
            false
        }
    }

    pub fn apply_workspace_edit(&mut self, edit: &WorkspaceEdit) -> usize {
        let mut total_applied = 0;
        if let Some(changes) = &edit.changes {
            for (uri, lsp_edits) in changes {
                let range_edits: Vec<RangeEdit> = lsp_edits
                    .iter()
                    .map(|e| RangeEdit {
                        range: Range::new(
                            Position::new(e.range.start.line as usize, e.range.start.character as usize),
                            Position::new(e.range.end.line as usize, e.range.end.character as usize),
                        ),
                        new_text: e.new_text.clone(),
                    })
                    .collect();

                if self.apply_text_edits(uri, &range_edits) {
                    total_applied += range_edits.len();
                } else {
                    let path = uri.strip_prefix("file://").unwrap_or(uri);
                    if let Ok(content) = std::fs::read_to_string(path) {
                        let title = std::path::Path::new(path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(path);
                        self.open_file(uri, title, &content, false);
                        if self.apply_text_edits(uri, &range_edits) {
                            total_applied += range_edits.len();
                        }
                    }
                }
            }
        }
        total_applied
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

            let is_dirty = tab.is_dirty
                || self.editors.get(&tab.uri).is_some_and(|e| e.is_dirty());

            let title_text = if is_dirty {
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

        if let Some(tab) = self.model.current_group_ref().active_tab() {
            if let Some(diff_view) = self.diff_editors.get(&tab.uri) {
                diff_view.render(content_rect, theme, font, scene);
            } else if let Some(webview) = self.webviews.get(&tab.uri) {
                webview.render(content_rect, theme, font, scene);
            } else if let Some(settings_view) = self.settings_views.get(&tab.uri) {
                settings_view.render(content_rect, theme, font, scene);
            } else if let Some(editor) = self.editors.get(&tab.uri) {
                editor.render(content_rect, theme, font, scene);
            } else {
                scene.push_quad(Quad::new(content_rect).with_background(theme.editor_background));
            }
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
                                self.close_tab(idx);
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
                    // Click within active editor / diff / webview / settings
                    let content_rect = Rect::from_xywh(
                        bounds.min_x(),
                        bounds.min_y() + tab_bar_h,
                        bounds.width(),
                        (bounds.height() - tab_bar_h).max(0.0),
                    );
                    if let Some(tab) = self.model.current_group_ref().active_tab() {
                        let uri = tab.uri.clone();
                        if let Some(diff_view) = self.diff_editors.get_mut(&uri) {
                            diff_view.handle_event(event, content_rect)
                        } else if let Some(webview) = self.webviews.get_mut(&uri) {
                            webview.handle_event(event, content_rect)
                        } else if let Some(settings_view) = self.settings_views.get_mut(&uri) {
                            settings_view.handle_event(event, content_rect)
                        } else if let Some(editor) = self.editors.get_mut(&uri) {
                            editor.handle_event(event, content_rect)
                        } else {
                            EventResult::Ignored
                        }
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
                if let Some(tab) = self.model.current_group_ref().active_tab() {
                    let uri = tab.uri.clone();
                    if let Some(diff_view) = self.diff_editors.get_mut(&uri) {
                        diff_view.handle_event(event, content_rect)
                    } else if let Some(webview) = self.webviews.get_mut(&uri) {
                        webview.handle_event(event, content_rect)
                    } else if let Some(settings_view) = self.settings_views.get_mut(&uri) {
                        settings_view.handle_event(event, content_rect)
                    } else if let Some(editor) = self.editors.get_mut(&uri) {
                        editor.handle_event(event, content_rect)
                    } else {
                        EventResult::Ignored
                    }
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

    #[test]
    fn test_editor_area_save_active_file() {
        let tmp_dir = std::env::temp_dir().join(format!("rustcode_area_save_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("saved_app.rs");
        let uri = format!("file://{}", path.display());

        let fs = FileSystemService::new();
        let mut area_view = EditorAreaView::default();
        area_view.open_file(&uri, "saved_app.rs", "fn initial() {}", false);

        // Edit active editor
        area_view.active_editor_mut().unwrap().insert_text(" ");
        assert!(area_view.active_editor().unwrap().is_dirty());

        // Save active file
        let meta = area_view.save_active_file(&fs).unwrap();
        assert_eq!(meta.path, path);
        assert!(!area_view.active_editor().unwrap().is_dirty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), " fn initial() {}");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_editor_area_apply_text_edits() {
        let mut area_view = EditorAreaView::default();
        area_view.open_file(
            "file:///src/calc.rs",
            "calc.rs",
            "fn add(a: i32, b: i32) -> i32 {\na + b\n}\n",
            false,
        );

        let edits = vec![RangeEdit::new(
            Range::new(Position::new(1, 0), Position::new(1, 5)),
            "    a + b",
        )];

        let ok = area_view.apply_text_edits("file:///src/calc.rs", &edits);
        assert!(ok);
        assert_eq!(
            area_view.editors.get("file:///src/calc.rs").unwrap().text(),
            "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n"
        );
        assert!(area_view.model.current_group_ref().tabs[0].is_dirty);
    }

    #[test]
    fn test_editor_area_apply_workspace_edit() {
        use rustcode_protocols::lsp::{LspPosition, LspRange, LspTextEdit};
        use std::collections::HashMap;

        let mut area_view = EditorAreaView::default();
        area_view.open_file(
            "file:///src/file1.rs",
            "file1.rs",
            "let x = 1;\n",
            false,
        );

        let mut changes = HashMap::new();
        changes.insert(
            "file:///src/file1.rs".to_string(),
            vec![LspTextEdit::new(
                LspRange::new(LspPosition::new(0, 8), LspPosition::new(0, 9)),
                "100",
            )],
        );

        let ws_edit = WorkspaceEdit::with_changes(changes);
        let applied = area_view.apply_workspace_edit(&ws_edit);
        assert_eq!(applied, 1);
        assert_eq!(
            area_view.editors.get("file:///src/file1.rs").unwrap().text(),
            "let x = 100;\n"
        );
        assert!(area_view.model.current_group_ref().tabs[0].is_dirty);
    }

    #[test]
    fn test_editor_area_apply_workspace_edit_multibyte_unicode() {
        use rustcode_protocols::lsp::{LspPosition, LspRange, LspTextEdit};
        use std::collections::HashMap;

        let mut area_view = EditorAreaView::default();
        area_view.open_file(
            "file:///src/unicode.rs",
            "unicode.rs",
            "let s = \"🦀abc\";\n",
            false,
        );

        let mut changes = HashMap::new();
        // Replace "abc" at UTF-16 col 11..14 with "xyz"
        changes.insert(
            "file:///src/unicode.rs".to_string(),
            vec![LspTextEdit::new(
                LspRange::new(LspPosition::new(0, 11), LspPosition::new(0, 14)),
                "xyz",
            )],
        );

        let ws_edit = WorkspaceEdit::with_changes(changes);
        let applied = area_view.apply_workspace_edit(&ws_edit);
        assert_eq!(applied, 1);
        assert_eq!(
            area_view.editors.get("file:///src/unicode.rs").unwrap().text(),
            "let s = \"🦀xyz\";\n"
        );
        assert!(area_view.model.current_group_ref().tabs[0].is_dirty);
    }

    #[test]
    fn test_editor_area_open_settings_render_events_and_close() {
        use crate::settings_ui::{SettingsFocus, SettingsHitTarget};
        use rustcode_platform::configuration::ConfigurationService;
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{KeyCode, KeyEvent, MouseButton, ScrollEvent};

        let config = ConfigurationService::new();
        let mut area_view = EditorAreaView::default();

        area_view.open_settings(Some(config.clone()));
        assert_eq!(area_view.model.current_group_ref().tabs.len(), 1);
        let tab = area_view.model.current_group_ref().active_tab().unwrap();
        assert_eq!(tab.uri, "rustcode://settings");
        assert_eq!(tab.title, "Settings");
        assert!(!tab.is_preview);
        assert!(area_view.settings_views.contains_key("rustcode://settings"));

        // Re-opening does not create a duplicate tab
        area_view.open_settings(Some(config.clone()));
        assert_eq!(area_view.model.current_group_ref().tabs.len(), 1);

        let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        area_view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("Settings"));
        assert!(scene.contains_text("Search settings..."));
        assert!(scene.contains_text("Font Size"));

        // Click SearchBar inside settings view
        let search_rect = area_view
            .settings_views
            .get("rustcode://settings")
            .unwrap()
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::SearchBar(r) => Some(*r),
                _ => None,
            })
            .unwrap();

        let res = area_view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(search_rect.min_x() + 5.0, search_rect.min_y() + 5.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(
            area_view
                .settings_views
                .get("rustcode://settings")
                .unwrap()
                .focus,
            SettingsFocus::Search
        );

        // Type into SearchBar via EditorAreaView::handle_event
        for ch in "minimap".chars() {
            let res = area_view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
            assert_eq!(res, EventResult::Consumed);
        }
        assert_eq!(
            area_view
                .settings_views
                .get("rustcode://settings")
                .unwrap()
                .state
                .search_query,
            "minimap"
        );

        // ScrollWheel routes to settings view
        let scroll_res = area_view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(400.0, 200.0), 0.0, -40.0)),
            bounds,
        );
        assert_eq!(scroll_res, EventResult::Consumed);

        // Closing tab cleans up settings_views
        assert!(area_view.close_tab(0));
        assert!(area_view.model.current_group_ref().tabs.is_empty());
        assert!(!area_view.settings_views.contains_key("rustcode://settings"));
    }
}

