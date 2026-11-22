//! Editor breadcrumbs bar showing workspace-relative path and enclosing symbol hierarchy.

use crate::search::search_document_symbols;
use crate::workspace::Workspace;
use parking_lot::RwLock;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use rustcode_protocols::lsp::types::SymbolKind;
use std::path::{Component, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreadcrumbKind {
    Workspace,
    Folder,
    File,
    Symbol(SymbolKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreadcrumbItem {
    pub label: String,
    pub kind: BreadcrumbKind,
    pub path: Option<PathBuf>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreadcrumbsAction {
    SelectItem { index: usize, item: BreadcrumbItem },
}

pub struct BreadcrumbsBar {
    pub items: Vec<BreadcrumbItem>,
    pub selected_index: Option<usize>,
    pub item_bounds: RwLock<Vec<(usize, Rect)>>,
    pub last_action: Option<BreadcrumbsAction>,
    last_font: RwLock<FontMetrics>,
}

impl Default for BreadcrumbsBar {
    fn default() -> Self {
        Self::new()
    }
}

impl BreadcrumbsBar {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selected_index: None,
            item_bounds: RwLock::new(Vec::new()),
            last_action: None,
            last_font: RwLock::new(FontMetrics::default_monospace()),
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.selected_index = None;
        self.item_bounds.write().clear();
    }

    pub fn update(
        &mut self,
        uri: &str,
        title: &str,
        workspace: Option<&Workspace>,
        content_and_cursor: Option<(&str, usize)>,
    ) {
        self.items.clear();
        self.selected_index = None;
        self.item_bounds.write().clear();

        // Special non-file URIs such as rustcode://settings, untitled://..., or untitled:Untitled-1
        if (uri.contains("://") && !rustcode_platform::is_file_uri(uri))
            || uri.starts_with("untitled:")
        {
            let label = if title.is_empty() { uri } else { title };
            self.items.push(BreadcrumbItem {
                label: label.to_string(),
                kind: BreadcrumbKind::File,
                path: None,
                line: None,
                column: None,
            });
            return;
        }

        let path_buf = rustcode_platform::file_uri_to_path(uri);
        let path = path_buf.as_path();
        let clean = path_buf.to_string_lossy();

        if let Some(folder) = workspace.and_then(|ws| ws.get_folder_for_path(path)) {
            self.items.push(BreadcrumbItem {
                label: folder.name.clone(),
                kind: BreadcrumbKind::Workspace,
                path: Some(folder.path.clone()),
                line: None,
                column: None,
            });

            if let Some(rel) = folder.relative_path(path) {
                let comps: Vec<_> = rel
                    .components()
                    .filter_map(|c| match c {
                        Component::Normal(s) => Some(s),
                        _ => None,
                    })
                    .collect();

                let mut accum = folder.path.clone();
                for (idx, comp) in comps.iter().enumerate() {
                    accum.push(comp);
                    let kind = if idx + 1 < comps.len() {
                        BreadcrumbKind::Folder
                    } else {
                        BreadcrumbKind::File
                    };
                    self.items.push(BreadcrumbItem {
                        label: comp.to_string_lossy().to_string(),
                        kind,
                        path: Some(accum.clone()),
                        line: None,
                        column: None,
                    });
                }
            } else {
                let label = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(title)
                    .to_string();
                self.items.push(BreadcrumbItem {
                    label,
                    kind: BreadcrumbKind::File,
                    path: Some(path.to_path_buf()),
                    line: None,
                    column: None,
                });
            }
        } else {
            let comps: Vec<_> = path
                .components()
                .filter_map(|c| match c {
                    Component::Normal(s) => Some(s),
                    _ => None,
                })
                .collect();

            if comps.is_empty() {
                let label = if title.is_empty() {
                    clean.as_ref()
                } else {
                    title
                };
                self.items.push(BreadcrumbItem {
                    label: label.to_string(),
                    kind: BreadcrumbKind::File,
                    path: Some(path.to_path_buf()),
                    line: None,
                    column: None,
                });
            } else {
                let mut accum = PathBuf::new();
                for comp in path.components() {
                    match comp {
                        Component::Prefix(p) => accum.push(p.as_os_str()),
                        Component::RootDir => accum.push(comp.as_os_str()),
                        _ => break,
                    }
                }
                for (idx, comp) in comps.iter().enumerate() {
                    accum.push(comp);
                    let kind = if idx + 1 < comps.len() {
                        BreadcrumbKind::Folder
                    } else {
                        BreadcrumbKind::File
                    };
                    self.items.push(BreadcrumbItem {
                        label: comp.to_string_lossy().to_string(),
                        kind,
                        path: Some(accum.clone()),
                        line: None,
                        column: None,
                    });
                }
            }
        }

        if let Some((content, cursor_line)) = content_and_cursor {
            let symbols = search_document_symbols(content, path);
            if let Some(sym) = symbols
                .iter()
                .rfind(|s| s.location.range.start.line as usize <= cursor_line)
            {
                if let Some(container) = &sym.container_name {
                    if container != &sym.name {
                        if let Some(c_sym) = symbols.iter().rfind(|s| {
                            s.name == *container
                                && s.location.range.start.line <= sym.location.range.start.line
                        }) {
                            self.items.push(BreadcrumbItem {
                                label: c_sym.name.clone(),
                                kind: BreadcrumbKind::Symbol(c_sym.kind),
                                path: Some(path.to_path_buf()),
                                line: Some(c_sym.location.range.start.line as usize),
                                column: Some(c_sym.location.range.start.character as usize),
                            });
                        } else {
                            self.items.push(BreadcrumbItem {
                                label: container.clone(),
                                kind: BreadcrumbKind::Symbol(SymbolKind::Struct),
                                path: Some(path.to_path_buf()),
                                line: Some(sym.location.range.start.line as usize),
                                column: Some(0),
                            });
                        }
                    }
                }
                self.items.push(BreadcrumbItem {
                    label: sym.name.clone(),
                    kind: BreadcrumbKind::Symbol(sym.kind),
                    path: Some(path.to_path_buf()),
                    line: Some(sym.location.range.start.line as usize),
                    column: Some(sym.location.range.start.character as usize),
                });
            }
        }
    }

    pub fn compute_item_rects(&self, bounds: Rect, font: &FontMetrics) -> Vec<(usize, Rect)> {
        let mut rects = Vec::new();
        if !bounds.is_finite_positive() || !font.is_finite() {
            return rects;
        }
        let mut cur_x = bounds.min_x() + 10.0;
        if !cur_x.is_finite() {
            return rects;
        }
        let sep_w = font.text_width(" > ");
        for (idx, item) in self.items.iter().enumerate() {
            if !cur_x.is_finite() || cur_x >= bounds.max_x() - 4.0 {
                break;
            }
            let label_w = font.text_width(&item.label).max(8.0);
            let clamped_w = label_w.min((bounds.max_x() - cur_x).max(0.0));
            let rect = Rect::from_xywh(cur_x, bounds.min_y(), clamped_w, bounds.height());
            if !rect.is_finite() {
                break;
            }
            rects.push((idx, rect));
            cur_x += label_w + sep_w;
        }
        rects
    }
}

impl View for BreadcrumbsBar {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !bounds.is_finite_positive() || !font.is_finite() {
            return;
        }

        *self.last_font.write() = font.clone();

        scene.push_quad(
            Quad::new(bounds)
                .with_background(theme.breadcrumb_bg)
                .with_border_edges(theme.tab_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        let rects = self.compute_item_rects(bounds, font);
        *self.item_bounds.write() = rects.clone();

        let text_y = bounds.min_y() + ((bounds.height() - font.line_height) / 2.0).max(0.0);
        let sep_w = font.text_width(" > ");
        let total = rects.len();

        scene.push_clip(bounds);
        for (pos_idx, (item_idx, rect)) in rects.iter().enumerate() {
            if let Some(item) = self.items.get(*item_idx) {
                let is_last = *item_idx + 1 == self.items.len();
                let is_selected = self.selected_index == Some(*item_idx);
                let fg = if is_last || is_selected {
                    theme.breadcrumb_focus_fg
                } else {
                    theme.breadcrumb_fg
                };

                scene.push_text(GlyphRun::new(
                    item.label.clone(),
                    Point::new(rect.min_x(), text_y),
                    Rect::from_xywh(rect.min_x(), text_y, rect.width(), font.line_height),
                    fg,
                    font.font_size * 0.9,
                ));

                if pos_idx + 1 < total && rect.max_x() + sep_w < bounds.max_x() {
                    scene.push_text(GlyphRun::new(
                        " > ".to_string(),
                        Point::new(rect.max_x(), text_y),
                        Rect::from_xywh(rect.max_x(), text_y, sep_w, font.line_height),
                        theme.breadcrumb_fg,
                        font.font_size * 0.9,
                    ));
                }
            }
        }
        scene.pop_clip();
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::MouseDown(mouse) => {
                if !bounds.is_finite_positive()
                    || !mouse.position.x.is_finite()
                    || !mouse.position.y.is_finite()
                    || !bounds.contains(mouse.position)
                {
                    return EventResult::Ignored;
                }
                let font = self.last_font.read().clone();
                let rects = self.compute_item_rects(bounds, &font);
                *self.item_bounds.write() = rects.clone();
                for (idx, rect) in rects {
                    if rect.contains(mouse.position) {
                        if let Some(item) = self.items.get(idx).cloned() {
                            self.selected_index = Some(idx);
                            self.last_action =
                                Some(BreadcrumbsAction::SelectItem { index: idx, item });
                            return EventResult::Consumed;
                        }
                    }
                }
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{MouseButton, MouseEvent};

    #[test]
    fn test_breadcrumbs_workspace_relative_and_rust_symbols() {
        let ws = Workspace::from_folder(PathBuf::from("/my-proj"));
        let mut bar = BreadcrumbsBar::new();
        let src = "pub struct Engine {\n    pub count: usize,\n}\n\nimpl Engine {\n    pub fn tick(&mut self) {\n        let x = 1;\n    }\n}\n";
        bar.update(
            "file:///my-proj/src/engine.rs",
            "engine.rs",
            Some(&ws),
            Some((src, 6)),
        );

        let labels: Vec<&str> = bar.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["my-proj", "src", "engine.rs", "Engine", "tick"]);
        assert_eq!(bar.items[0].kind, BreadcrumbKind::Workspace);
        assert_eq!(bar.items[1].kind, BreadcrumbKind::Folder);
        assert_eq!(bar.items[2].kind, BreadcrumbKind::File);
        assert_eq!(bar.items[3].kind, BreadcrumbKind::Symbol(SymbolKind::Struct));
        assert_eq!(bar.items[3].line, Some(0));
        assert_eq!(bar.items[4].kind, BreadcrumbKind::Symbol(SymbolKind::Method));
        assert_eq!(bar.items[4].line, Some(5));
    }

    #[test]
    fn test_breadcrumbs_special_uri_and_non_workspace_fallback() {
        let mut bar = BreadcrumbsBar::new();
        bar.update("rustcode://settings", "Settings", None, None);
        assert_eq!(bar.items.len(), 1);
        assert_eq!(bar.items[0].label, "Settings");
        assert!(bar.items[0].path.is_none());

        bar.update("file:///tmp/demo/app.py", "app.py", None, Some(("class Runner:\n    def run(self):\n        pass\n", 2)));
        let labels: Vec<&str> = bar.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["tmp", "demo", "app.py", "Runner", "run"]);

        #[cfg(windows)]
        {
            bar.update("file:///C:/Users/demo/app.py", "app.py", None, None);
            let win_labels: Vec<&str> = bar.items.iter().map(|i| i.label.as_str()).collect();
            assert_eq!(win_labels, vec!["Users", "demo", "app.py"]);
            assert_eq!(
                bar.items[2].path.as_deref(),
                Some(std::path::Path::new(r"C:\Users\demo\app.py"))
            );
        }
    }

    #[test]
    fn test_breadcrumbs_render_and_click_emits_action() {
        let ws = Workspace::from_folder(PathBuf::from("/proj"));
        let mut bar = BreadcrumbsBar::new();
        bar.update(
            "file:///proj/src/lib.rs",
            "lib.rs",
            Some(&ws),
            Some(("pub fn alpha() {}\n", 0)),
        );

        let bounds = Rect::from_xywh(0.0, 35.0, 600.0, 22.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        bar.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("proj"));
        assert!(scene.contains_text("lib.rs"));
        assert!(scene.contains_text("alpha"));

        let rects = bar.compute_item_rects(bounds, &font);
        let (alpha_idx, alpha_rect) = *rects.last().unwrap();
        let click = InputEvent::MouseDown(MouseEvent {
            position: Point::new(alpha_rect.min_x() + 4.0, alpha_rect.min_y() + 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        assert_eq!(bar.handle_event(&click, bounds), EventResult::Consumed);
        assert_eq!(bar.selected_index, Some(alpha_idx));
        match bar.last_action.as_ref().unwrap() {
            BreadcrumbsAction::SelectItem { index, item } => {
                assert_eq!(*index, alpha_idx);
                assert_eq!(item.label, "alpha");
                assert_eq!(item.line, Some(0));
            }
        }
    }

    #[test]
    fn test_breadcrumbs_untitled_without_slashes_has_no_path() {
        let mut bar = BreadcrumbsBar::new();
        bar.update("untitled:Untitled-1", "Untitled-1", None, None);
        assert_eq!(bar.items.len(), 1);
        assert_eq!(bar.items[0].label, "Untitled-1");
        assert!(bar.items[0].path.is_none());

        bar.update("untitled:nested/doc", "", None, None);
        assert_eq!(bar.items.len(), 1);
        assert_eq!(bar.items[0].label, "untitled:nested/doc");
        assert!(bar.items[0].path.is_none());
    }

    #[test]
    fn test_breadcrumbs_click_after_update_and_bounds_shift_without_render() {
        let ws = Workspace::from_folder(PathBuf::from("/proj"));
        let mut bar = BreadcrumbsBar::new();
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        // 1. Render initial file at bounds_a (sidebar open: x = 298.0)
        bar.update("file:///proj/a.rs", "a.rs", Some(&ws), None);
        let bounds_a = Rect::from_xywh(298.0, 35.0, 600.0, 22.0);
        bar.render(bounds_a, &theme, &font, &mut Scene::new());
        assert!(!bar.item_bounds.read().is_empty());

        // 2. Update to file with symbol WITHOUT calling render() -> item_bounds cleared and click still hits symbol
        bar.update(
            "file:///proj/longer_subdir/mod_file.rs",
            "mod_file.rs",
            Some(&ws),
            Some(("pub fn target_fn() {}\n", 0)),
        );
        assert!(bar.item_bounds.read().is_empty());

        let rects_b = bar.compute_item_rects(bounds_a, &font);
        let (sym_idx, sym_rect) = *rects_b.last().unwrap();
        assert_eq!(bar.items[sym_idx].label, "target_fn");
        let click_b = InputEvent::MouseDown(MouseEvent {
            position: Point::new(sym_rect.min_x() + 4.0, sym_rect.min_y() + 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        assert_eq!(bar.handle_event(&click_b, bounds_a), EventResult::Consumed);
        match bar.last_action.as_ref().unwrap() {
            BreadcrumbsAction::SelectItem { index, item } => {
                assert_eq!(*index, sym_idx);
                assert_eq!(item.label, "target_fn");
            }
        }

        // 3. Shift bounds (e.g. sidebar toggled closed: x = 48.0) WITHOUT calling render()
        bar.last_action = None;
        let bounds_shifted = Rect::from_xywh(48.0, 35.0, 850.0, 22.0);
        let rects_shifted = bar.compute_item_rects(bounds_shifted, &font);
        let (shifted_idx, shifted_rect) = *rects_shifted.last().unwrap();
        let click_shifted = InputEvent::MouseDown(MouseEvent {
            position: Point::new(shifted_rect.min_x() + 4.0, shifted_rect.min_y() + 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        assert_eq!(bar.handle_event(&click_shifted, bounds_shifted), EventResult::Consumed);
        match bar.last_action.as_ref().unwrap() {
            BreadcrumbsAction::SelectItem { index, item } => {
                assert_eq!(*index, shifted_idx);
                assert_eq!(item.label, "target_fn");
            }
        }
    }

    #[test]
    fn test_breadcrumbs_render_clips_to_bounds() {
        use rustcode_platform::render::SceneNode;

        let ws = Workspace::from_folder(PathBuf::from("/very_long_workspace_name"));
        let mut bar = BreadcrumbsBar::new();
        bar.update(
            "file:///very_long_workspace_name/src/main.rs",
            "main.rs",
            Some(&ws),
            None,
        );
        let bounds = Rect::from_xywh(20.0, 35.0, 120.0, 22.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        bar.render(bounds, &theme, &font, &mut scene);

        let push_clips: Vec<Rect> = scene
            .nodes()
            .iter()
            .filter_map(|n| match n {
                SceneNode::PushClip(r) => Some(*r),
                _ => None,
            })
            .collect();
        let pop_count = scene
            .nodes()
            .iter()
            .filter(|n| matches!(n, SceneNode::PopClip))
            .count();
        assert_eq!(push_clips, vec![bounds]);
        assert_eq!(pop_count, 1);
    }

    #[test]
    fn test_breadcrumbs_render_non_finite_bounds_emits_nothing() {
        use rustcode_platform::render::Size;

        let ws = Workspace::from_folder(PathBuf::from("/demo"));
        let mut bar = BreadcrumbsBar::new();
        bar.update("file:///demo/a.rs", "a.rs", Some(&ws), None);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for bad_bounds in [
            Rect::new(Point::zero(), Size::new(400.0, f32::NAN)),
            Rect::new(Point::new(f32::NAN, 0.0), Size::new(400.0, 22.0)),
            Rect::from_xywh(0.0, 0.0, f32::INFINITY, 22.0),
        ] {
            let mut scene = Scene::new();
            bar.render(bad_bounds, &theme, &font, &mut scene);
            assert!(scene.is_empty());
        }
    }
}
