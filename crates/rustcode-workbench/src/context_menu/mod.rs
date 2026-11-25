//! VS Code `contributes.menus` registry and context menu overlay view.

use crate::explorer::evaluate_when_clause;
use rustcode_base::context_key::ContextKeyService;
use rustcode_ext_host::manifest::MenuContribution;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, MouseButton, Point, Quad, Rect,
    Rgba, Scene, Shadow, View, WorkbenchThemeStyles,
};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuLocation {
    EditorContext,
    ExplorerContext,
    EditorTitleContext,
    ScmTitle,
    ScmResourceState,
    ViewTitle,
    ViewItemContext,
    Custom(String),
}

impl MenuLocation {
    pub fn as_id(&self) -> &str {
        match self {
            Self::EditorContext => "editor/context",
            Self::ExplorerContext => "explorer/context",
            Self::EditorTitleContext => "editor/title/context",
            Self::ScmTitle => "scm/title",
            Self::ScmResourceState => "scm/resourceState/context",
            Self::ViewTitle => "view/title",
            Self::ViewItemContext => "view/item/context",
            Self::Custom(s) => s.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredMenuItem {
    pub command: String,
    pub label: String,
    pub shortcut: Option<String>,
    pub when: Option<String>,
    pub group: String,
    pub order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuDisplayEntry {
    Item {
        command: String,
        label: String,
        shortcut: Option<String>,
    },
    Separator,
}

#[derive(Debug, Clone)]
pub struct ContextMenuRegistry {
    items_by_location: HashMap<String, Vec<RegisteredMenuItem>>,
}

impl Default for ContextMenuRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_group_and_order(raw: Option<&str>) -> (String, i32) {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return ("z_extension".to_string(), i32::MAX);
    };
    if let Some((grp, ord_str)) = s.split_once('@') {
        let ord = ord_str.trim().parse::<i32>().unwrap_or(i32::MAX);
        (grp.trim().to_string(), ord)
    } else {
        (s.to_string(), i32::MAX)
    }
}

fn humanize_command_id(command_id: &str) -> String {
    let tail = command_id.rsplit('.').next().unwrap_or(command_id);
    let mut out = String::new();
    for (idx, ch) in tail.chars().enumerate() {
        if idx == 0 {
            for uc in ch.to_uppercase() {
                out.push(uc);
            }
        } else if ch.is_uppercase() && !out.ends_with(' ') {
            out.push(' ');
            out.push(ch);
        } else if ch == '_' || ch == '-' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

impl ContextMenuRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            items_by_location: HashMap::new(),
        };
        reg.seed_builtin_menus();
        reg
    }

    fn push_builtin(
        &mut self,
        location: &str,
        command: &str,
        label: &str,
        shortcut: Option<&str>,
        when: Option<&str>,
        group_spec: &str,
    ) {
        let (group, order) = parse_group_and_order(Some(group_spec));
        self.items_by_location
            .entry(location.to_string())
            .or_default()
            .push(RegisteredMenuItem {
                command: command.to_string(),
                label: label.to_string(),
                shortcut: shortcut.map(ToString::to_string),
                when: when.map(ToString::to_string),
                group,
                order,
            });
    }

    fn seed_builtin_menus(&mut self) {
        // Editor context menu
        self.push_builtin(
            "editor/context",
            "editor.action.revealDefinition",
            "Go to Definition",
            Some("F12"),
            None,
            "navigation@1",
        );
        self.push_builtin(
            "editor/context",
            "editor.action.goToReferences",
            "Find All References",
            Some("Shift+F12"),
            None,
            "navigation@2",
        );
        self.push_builtin(
            "editor/context",
            "editor.action.peekDefinition",
            "Peek Definition",
            Some("Alt+F12"),
            None,
            "navigation@3",
        );
        self.push_builtin(
            "editor/context",
            "editor.action.rename",
            "Rename Symbol",
            Some("F2"),
            Some("!editorReadonly"),
            "1_modification@1",
        );
        self.push_builtin(
            "editor/context",
            "editor.action.formatDocument",
            "Format Document",
            Some("Shift+Alt+F"),
            Some("!editorReadonly"),
            "1_modification@2",
        );
        self.push_builtin(
            "editor/context",
            "editor.action.quickFix",
            "Quick Fix...",
            Some("Ctrl+."),
            None,
            "1_modification@3",
        );
        self.push_builtin(
            "editor/context",
            "editor.action.commentLine",
            "Toggle Line Comment",
            Some("Ctrl+/"),
            None,
            "1_modification@4",
        );
        self.push_builtin(
            "editor/context",
            "actions.find",
            "Find",
            Some("Ctrl+F"),
            None,
            "9_cutcopypaste@1",
        );

        // Explorer context menu
        self.push_builtin(
            "explorer/context",
            "explorer.newFile",
            "New File...",
            None,
            None,
            "navigation@1",
        );
        self.push_builtin(
            "explorer/context",
            "explorer.newFolder",
            "New Folder...",
            None,
            None,
            "navigation@2",
        );
        self.push_builtin(
            "explorer/context",
            "explorer.rename",
            "Rename",
            Some("F2"),
            Some("explorerResourceSelected"),
            "2_workspace@1",
        );
        self.push_builtin(
            "explorer/context",
            "explorer.delete",
            "Delete",
            Some("Delete"),
            Some("explorerResourceSelected"),
            "2_workspace@2",
        );
        self.push_builtin(
            "explorer/context",
            "workbench.action.splitEditor",
            "Open to the Side",
            None,
            None,
            "3_compare@1",
        );

        // Editor tab title context menu
        self.push_builtin(
            "editor/title/context",
            "workbench.action.splitEditor",
            "Split Right",
            None,
            None,
            "1_split@1",
        );
        self.push_builtin(
            "editor/title/context",
            "workbench.action.splitEditorDown",
            "Split Down",
            None,
            None,
            "1_split@2",
        );
        self.push_builtin(
            "editor/title/context",
            "workbench.action.closeActiveEditor",
            "Close",
            Some("Ctrl+W"),
            None,
            "2_close@1",
        );
        self.push_builtin(
            "editor/title/context",
            "workbench.action.closeOtherEditors",
            "Close Others",
            None,
            None,
            "2_close@2",
        );
    }

    pub fn register_contributions(
        &mut self,
        menus: &HashMap<String, Vec<MenuContribution>>,
        command_titles: &HashMap<String, String>,
    ) {
        for (location, entries) in menus {
            let bucket = self.items_by_location.entry(location.clone()).or_default();
            for entry in entries {
                let (group, order) = parse_group_and_order(entry.group.as_deref());
                let label = command_titles
                    .get(&entry.command)
                    .cloned()
                    .unwrap_or_else(|| humanize_command_id(&entry.command));
                if let Some(existing) = bucket.iter_mut().find(|i| i.command == entry.command) {
                    existing.label = label;
                    existing.when = entry.when.clone();
                    existing.group = group;
                    existing.order = order;
                } else {
                    bucket.push(RegisteredMenuItem {
                        command: entry.command.clone(),
                        label,
                        shortcut: None,
                        when: entry.when.clone(),
                        group,
                        order,
                    });
                }
            }
        }
    }

    pub fn resolve(&self, location: &str, context_keys: &ContextKeyService) -> Vec<MenuDisplayEntry> {
        let Some(items) = self.items_by_location.get(location) else {
            return Vec::new();
        };

        let mut visible: Vec<&RegisteredMenuItem> = items
            .iter()
            .filter(|item| evaluate_when_clause(context_keys, item.when.as_deref()))
            .collect();

        fn group_rank(g: &str) -> (u8, &str) {
            if g == "navigation" {
                (0, "")
            } else if g.is_empty() {
                (2, "")
            } else {
                (1, g)
            }
        }
        visible.sort_by(|a, b| {
            group_rank(&a.group)
                .cmp(&group_rank(&b.group))
                .then_with(|| a.order.cmp(&b.order))
                .then_with(|| a.label.cmp(&b.label))
        });

        let mut out = Vec::new();
        let mut last_group: Option<&str> = None;
        for item in visible {
            if let Some(prev) = last_group {
                if prev != item.group {
                    out.push(MenuDisplayEntry::Separator);
                }
            }
            last_group = Some(item.group.as_str());
            out.push(MenuDisplayEntry::Item {
                command: item.command.clone(),
                label: item.label.clone(),
                shortcut: item.shortcut.clone(),
            });
        }
        out
    }
}

pub struct WorkbenchContextMenu {
    pub is_open: bool,
    pub location: String,
    pub anchor: Point,
    pub entries: Vec<MenuDisplayEntry>,
    pub selected_item: Option<usize>,
    pub last_selected_command: Option<String>,
    pub context_path: Option<PathBuf>,
    last_font: FontMetrics,
}

impl Default for WorkbenchContextMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkbenchContextMenu {
    const WIDTH: f32 = 250.0;
    const ITEM_HEIGHT: f32 = 24.0;
    const SEPARATOR_HEIGHT: f32 = 7.0;
    const PAD_V: f32 = 4.0;

    pub fn new() -> Self {
        Self {
            is_open: false,
            location: String::new(),
            anchor: Point::zero(),
            entries: Vec::new(),
            selected_item: None,
            last_selected_command: None,
            context_path: None,
            last_font: FontMetrics::default_monospace(),
        }
    }

    pub fn open(
        &mut self,
        location: impl Into<String>,
        anchor: Point,
        entries: Vec<MenuDisplayEntry>,
        context_path: Option<PathBuf>,
    ) {
        if entries.is_empty() {
            self.close();
            return;
        }
        self.is_open = true;
        self.location = location.into();
        self.anchor = anchor;
        self.selected_item = entries
            .iter()
            .position(|e| matches!(e, MenuDisplayEntry::Item { .. }));
        self.entries = entries;
        self.context_path = context_path;
        self.last_selected_command = None;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.entries.clear();
        self.selected_item = None;
    }

    pub fn take_selected_command(&mut self) -> Option<String> {
        self.last_selected_command.take()
    }

    pub fn compute_rect(&self, bounds: Rect, _font: &FontMetrics) -> Rect {
        if !self.is_open || self.entries.is_empty() || !bounds.is_finite_positive() {
            return Rect::zero();
        }
        let mut h = Self::PAD_V * 2.0;
        for e in &self.entries {
            h += match e {
                MenuDisplayEntry::Item { .. } => Self::ITEM_HEIGHT,
                MenuDisplayEntry::Separator => Self::SEPARATOR_HEIGHT,
            };
        }
        let w = Self::WIDTH.min(bounds.width().max(80.0));
        let h = h.min(bounds.height().max(Self::ITEM_HEIGHT));
        let max_x = (bounds.max_x() - w).max(bounds.min_x());
        let max_y = (bounds.max_y() - h).max(bounds.min_y());
        let x = self.anchor.x.clamp(bounds.min_x(), max_x);
        let y = self.anchor.y.clamp(bounds.min_y(), max_y);
        Rect::from_xywh(x, y, w, h)
    }

    fn entry_at_y(&self, rect: Rect, y: f32) -> Option<usize> {
        let mut cur_y = rect.min_y() + Self::PAD_V;
        for (idx, entry) in self.entries.iter().enumerate() {
            let eh = match entry {
                MenuDisplayEntry::Item { .. } => Self::ITEM_HEIGHT,
                MenuDisplayEntry::Separator => Self::SEPARATOR_HEIGHT,
            };
            if y >= cur_y && y < cur_y + eh {
                return matches!(entry, MenuDisplayEntry::Item { .. }).then_some(idx);
            }
            cur_y += eh;
        }
        None
    }

    fn move_selection(&mut self, forward: bool) {
        let item_indices: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| matches!(e, MenuDisplayEntry::Item { .. }).then_some(i))
            .collect();
        if item_indices.is_empty() {
            return;
        }
        let next_pos = match self
            .selected_item
            .and_then(|s| item_indices.iter().position(|&idx| idx == s))
        {
            Some(pos) => {
                if forward {
                    (pos + 1) % item_indices.len()
                } else if pos == 0 {
                    item_indices.len() - 1
                } else {
                    pos - 1
                }
            }
            None => 0,
        };
        self.selected_item = Some(item_indices[next_pos]);
    }
}

impl View for WorkbenchContextMenu {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !self.is_open {
            return;
        }
        let rect = self.compute_rect(bounds, font);
        if !rect.is_finite_positive() {
            return;
        }
        scene.push_shadow(Shadow {
            bounds: rect,
            offset: Point::new(0.0, 2.0),
            blur: 10.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.45),
        });
        scene.push_finite_quad(
            Quad::new(rect)
                .with_background(theme.command_palette_bg)
                .with_border_edges(theme.command_palette_border, Edges::all(1.0)),
        );

        let mut cur_y = rect.min_y() + Self::PAD_V;
        for (idx, entry) in self.entries.iter().enumerate() {
            match entry {
                MenuDisplayEntry::Separator => {
                    if cur_y + Self::SEPARATOR_HEIGHT > rect.max_y() {
                        break;
                    }
                    let line_rect = Rect::from_xywh(
                        rect.min_x() + 8.0,
                        cur_y + 3.0,
                        (rect.width() - 16.0).max(0.0),
                        1.0,
                    );
                    scene.push_finite_quad(Quad::new(line_rect).with_background(theme.panel_border));
                    cur_y += Self::SEPARATOR_HEIGHT;
                }
                MenuDisplayEntry::Item {
                    label, shortcut, ..
                } => {
                    if cur_y + Self::ITEM_HEIGHT > rect.max_y() {
                        break;
                    }
                    let row_rect = Rect::from_xywh(
                        rect.min_x() + 2.0,
                        cur_y,
                        (rect.width() - 4.0).max(0.0),
                        Self::ITEM_HEIGHT,
                    );
                    if self.selected_item == Some(idx) {
                        scene.push_finite_quad(
                            Quad::new(row_rect).with_background(theme.command_palette_selected_bg),
                        );
                    }
                    let text_y = cur_y + ((Self::ITEM_HEIGHT - font.line_height) * 0.5).max(0.0);
                    scene.push_finite_text(GlyphRun::new(
                        label.clone(),
                        Point::new(row_rect.min_x() + 10.0, text_y),
                        row_rect,
                        theme.command_palette_fg,
                        font.font_size,
                    ));
                    if let Some(sc) = shortcut {
                        let sw = font.text_width(sc);
                        scene.push_finite_text(GlyphRun::new(
                            sc.clone(),
                            Point::new((row_rect.max_x() - sw - 10.0).max(row_rect.min_x()), text_y),
                            row_rect,
                            theme.breadcrumb_fg,
                            font.font_size * 0.9,
                        ));
                    }
                    cur_y += Self::ITEM_HEIGHT;
                }
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if !self.is_open {
            return EventResult::Ignored;
        }
        let rect = self.compute_rect(bounds, &self.last_font);
        match event {
            InputEvent::KeyDown(k) => match k.key {
                KeyCode::Escape => {
                    self.close();
                    EventResult::Consumed
                }
                KeyCode::ArrowDown => {
                    self.move_selection(true);
                    EventResult::Consumed
                }
                KeyCode::ArrowUp => {
                    self.move_selection(false);
                    EventResult::Consumed
                }
                KeyCode::Enter => {
                    if let Some(idx) = self.selected_item {
                        if let Some(MenuDisplayEntry::Item { command, .. }) = self.entries.get(idx) {
                            self.last_selected_command = Some(command.clone());
                        }
                    }
                    self.close();
                    EventResult::Consumed
                }
                _ => EventResult::Ignored,
            },
            InputEvent::MouseMove(mm) => {
                if rect.contains(mm.position) {
                    self.selected_item = self.entry_at_y(rect, mm.position.y);
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::MouseDown(mouse) => {
                if rect.contains(mouse.position) && mouse.button == MouseButton::Left {
                    if let Some(idx) = self.entry_at_y(rect, mouse.position.y) {
                        if let Some(MenuDisplayEntry::Item { command, .. }) = self.entries.get(idx) {
                            self.last_selected_command = Some(command.clone());
                        }
                    }
                    self.close();
                } else {
                    self.close();
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
    use rustcode_platform::render::{KeyEvent, MouseEvent};

    #[test]
    fn test_context_menu_registry_when_clause_group_sort_and_contributions() {
        let mut reg = ContextMenuRegistry::new();
        let mut menus = HashMap::new();
        menus.insert(
            "editor/context".to_string(),
            vec![
                MenuContribution {
                    command: "ext.firstNav".to_string(),
                    when: Some("resourceExtname == '.rs'".to_string()),
                    group: Some("navigation@0".to_string()),
                    alt: None,
                },
                MenuContribution {
                    command: "ext.hidden".to_string(),
                    when: Some("resourceExtname == '.py'".to_string()),
                    group: Some("z_custom@1".to_string()),
                    alt: None,
                },
            ],
        );
        let mut titles = HashMap::new();
        titles.insert("ext.firstNav".to_string(), "Rust Extra Nav".to_string());
        reg.register_contributions(&menus, &titles);

        let ctx = ContextKeyService::new();
        ctx.set_string("resourceExtname", ".rs");
        ctx.set_bool("editorReadonly", false);

        let resolved = reg.resolve(MenuLocation::EditorContext.as_id(), &ctx);
        // First item must be navigation@0 extension item
        assert!(matches!(
            resolved.first(),
            Some(MenuDisplayEntry::Item { command, label, .. })
                if command == "ext.firstNav" && label == "Rust Extra Nav"
        ));
        // Hidden .py item excluded
        assert!(!resolved.iter().any(|e| matches!(
            e,
            MenuDisplayEntry::Item { command, .. } if command == "ext.hidden"
        )));
        // Group separators exist
        assert!(resolved.iter().any(|e| matches!(e, MenuDisplayEntry::Separator)));

        // Setting editorReadonly=true hides !editorReadonly modification items
        ctx.set_bool("editorReadonly", true);
        let readonly_resolved = reg.resolve("editor/context", &ctx);
        assert!(!readonly_resolved.iter().any(|e| matches!(
            e,
            MenuDisplayEntry::Item { command, .. } if command == "editor.action.rename"
        )));
    }

    #[test]
    fn test_workbench_context_menu_clamp_render_and_selection() {
        let reg = ContextMenuRegistry::new();
        let ctx = ContextKeyService::new();
        let entries = reg.resolve("editor/context", &ctx);

        let mut menu = WorkbenchContextMenu::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        // Open near bottom-right corner to verify bounds clamping
        menu.open("editor/context", Point::new(780.0, 590.0), entries, None);
        assert!(menu.is_open);

        let font = FontMetrics::default_monospace();
        let rect = menu.compute_rect(bounds, &font);
        assert!(rect.max_x() <= bounds.max_x() + 0.1);
        assert!(rect.max_y() <= bounds.max_y() + 0.1);

        let mut scene = Scene::new();
        menu.render(bounds, &WorkbenchThemeStyles::dark_default(), &font, &mut scene);
        assert!(scene.contains_text("Go to Definition"));

        // ArrowDown + Enter selects second item
        menu.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::ArrowDown, Modifiers::none())),
            bounds,
        );
        menu.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert!(!menu.is_open);
        assert_eq!(
            menu.take_selected_command().as_deref(),
            Some("editor.action.goToReferences")
        );

        // Outside click dismisses cleanly
        let entries = reg.resolve("explorer/context", &ctx);
        menu.open("explorer/context", Point::new(50.0, 50.0), entries, None);
        menu.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(Point::new(700.0, 500.0), MouseButton::Left)),
            bounds,
        );
        assert!(!menu.is_open);
    }
}
