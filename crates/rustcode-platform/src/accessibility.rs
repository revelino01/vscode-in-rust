//! Pure-Rust semantic accessibility tree primitives for GPUI views.

use crate::render::geometry::{Point, Rect};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessibleRole {
    Window,
    Toolbar,
    Sidebar,
    Tree,
    TreeItem,
    TabList,
    Tab,
    Breadcrumb,
    BreadcrumbItem,
    TextEditor,
    ScrollBar,
    Panel,
    StatusBar,
    Button,
    Dialog,
    TextInput,
    List,
    ListItem,
    StaticText,
}

impl AccessibleRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Window => "Window",
            Self::Toolbar => "Toolbar",
            Self::Sidebar => "Sidebar",
            Self::Tree => "Tree",
            Self::TreeItem => "TreeItem",
            Self::TabList => "TabList",
            Self::Tab => "Tab",
            Self::Breadcrumb => "Breadcrumb",
            Self::BreadcrumbItem => "BreadcrumbItem",
            Self::TextEditor => "TextEditor",
            Self::ScrollBar => "ScrollBar",
            Self::Panel => "Panel",
            Self::StatusBar => "StatusBar",
            Self::Button => "Button",
            Self::Dialog => "Dialog",
            Self::TextInput => "TextInput",
            Self::List => "List",
            Self::ListItem => "ListItem",
            Self::StaticText => "StaticText",
        }
    }
}

impl fmt::Display for AccessibleRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AccessibleState {
    pub focused: bool,
    pub selected: bool,
    pub expanded: Option<bool>,
    pub disabled: bool,
    pub read_only: bool,
}

impl AccessibleState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibleNode {
    pub id: String,
    pub role: AccessibleRole,
    pub name: String,
    pub value: Option<String>,
    pub description: Option<String>,
    pub bounds: Rect,
    pub state: AccessibleState,
    pub actions: Vec<String>,
    pub children: Vec<AccessibleNode>,
}

impl AccessibleNode {
    pub fn new(
        id: impl Into<String>,
        role: AccessibleRole,
        name: impl Into<String>,
        bounds: Rect,
    ) -> Self {
        Self {
            id: id.into(),
            role,
            name: name.into(),
            value: None,
            description: None,
            bounds,
            state: AccessibleState::default(),
            actions: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_state(mut self, state: AccessibleState) -> Self {
        self.state = state;
        self
    }

    pub fn with_focused(mut self, focused: bool) -> Self {
        self.state.focused = focused;
        self
    }

    pub fn with_selected(mut self, selected: bool) -> Self {
        self.state.selected = selected;
        self
    }

    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.state.expanded = Some(expanded);
        self
    }

    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.state.disabled = disabled;
        self
    }

    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.state.read_only = read_only;
        self
    }

    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.actions.push(action.into());
        self
    }

    pub fn with_actions<I, S>(mut self, actions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.actions.extend(actions.into_iter().map(Into::into));
        self
    }

    pub fn add_child(&mut self, child: AccessibleNode) {
        self.children.push(child);
    }

    pub fn with_child(mut self, child: AccessibleNode) -> Self {
        self.children.push(child);
        self
    }

    pub fn with_children(mut self, children: Vec<AccessibleNode>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn count_nodes(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(Self::count_nodes)
            .sum::<usize>()
    }

    pub fn find_by_id(&self, id: &str) -> Option<&AccessibleNode> {
        if self.id == id {
            return Some(self);
        }
        for child in &self.children {
            if let Some(found) = child.find_by_id(id) {
                return Some(found);
            }
        }
        None
    }

    pub fn collect_by_role<'a>(
        &'a self,
        role: AccessibleRole,
        out: &mut Vec<&'a AccessibleNode>,
    ) {
        if self.role == role {
            out.push(self);
        }
        for child in &self.children {
            child.collect_by_role(role, out);
        }
    }

    pub fn find_focused(&self) -> Option<&AccessibleNode> {
        for child in &self.children {
            if let Some(focused) = child.find_focused() {
                return Some(focused);
            }
        }
        if self.state.focused {
            Some(self)
        } else {
            None
        }
    }

    pub fn hit_test(&self, point: Point) -> Option<&AccessibleNode> {
        if self.bounds.width() <= 0.0
            || self.bounds.height() <= 0.0
            || !self.bounds.contains(point)
        {
            return None;
        }
        for child in self.children.iter().rev() {
            if let Some(hit) = child.hit_test(point) {
                return Some(hit);
            }
        }
        Some(self)
    }

    pub fn collect_flat<'a>(&'a self, out: &mut Vec<&'a AccessibleNode>) {
        out.push(self);
        for child in &self.children {
            child.collect_flat(out);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityTree {
    pub root: AccessibleNode,
}

impl AccessibilityTree {
    pub fn new(root: AccessibleNode) -> Self {
        Self { root }
    }

    pub fn node_count(&self) -> usize {
        self.root.count_nodes()
    }

    pub fn find_by_id(&self, id: &str) -> Option<&AccessibleNode> {
        self.root.find_by_id(id)
    }

    pub fn find_by_role(&self, role: AccessibleRole) -> Vec<&AccessibleNode> {
        let mut out = Vec::new();
        self.root.collect_by_role(role, &mut out);
        out
    }

    pub fn focused_node(&self) -> Option<&AccessibleNode> {
        self.root.find_focused()
    }

    pub fn hit_test(&self, point: Point) -> Option<&AccessibleNode> {
        self.root.hit_test(point)
    }

    pub fn to_flat_list(&self) -> Vec<&AccessibleNode> {
        let mut out = Vec::new();
        self.root.collect_flat(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> AccessibilityTree {
        let editor = AccessibleNode::new(
            "editor.main",
            AccessibleRole::TextEditor,
            "main.rs",
            Rect::from_xywh(100.0, 50.0, 600.0, 400.0),
        )
        .with_focused(true)
        .with_value("fn main() {}")
        .with_description("Ln 1, Col 1")
        .with_actions(["focus", "edit", "scroll"])
        .with_child(
            AccessibleNode::new(
                "editor.minimap",
                AccessibleRole::ScrollBar,
                "Minimap",
                Rect::from_xywh(616.0, 50.0, 84.0, 400.0),
            )
            .with_action("scroll"),
        );

        let editor_area = AccessibleNode::new(
            "workbench.editorArea",
            AccessibleRole::Panel,
            "Editor Area",
            Rect::from_xywh(100.0, 0.0, 600.0, 500.0),
        )
        .with_focused(true)
        .with_child(
            AccessibleNode::new(
                "workbench.tabs",
                AccessibleRole::TabList,
                "Open Editors",
                Rect::from_xywh(100.0, 0.0, 600.0, 35.0),
            )
            .with_child(
                AccessibleNode::new(
                    "tab.0",
                    AccessibleRole::Tab,
                    "main.rs",
                    Rect::from_xywh(100.0, 0.0, 120.0, 35.0),
                )
                .with_selected(true),
            ),
        )
        .with_child(editor);

        let overlay = AccessibleNode::new(
            "workbench.commandPalette",
            AccessibleRole::Dialog,
            "Command Palette",
            Rect::from_xywh(200.0, 40.0, 300.0, 200.0),
        )
        .with_child(
            AccessibleNode::new(
                "commandPalette.input",
                AccessibleRole::TextInput,
                "Type a command",
                Rect::from_xywh(210.0, 50.0, 280.0, 28.0),
            )
            .with_action("input"),
        );

        let root = AccessibleNode::new(
            "workbench.window",
            AccessibleRole::Window,
            "RustCode Workbench",
            Rect::from_xywh(0.0, 0.0, 800.0, 600.0),
        )
        .with_child(editor_area)
        .with_child(overlay);

        AccessibilityTree::new(root)
    }

    #[test]
    fn test_node_count_and_flat_list() {
        let tree = sample_tree();
        assert_eq!(tree.node_count(), 8);
        let flat = tree.to_flat_list();
        assert_eq!(flat.len(), 8);
        assert_eq!(flat[0].id, "workbench.window");
        assert_eq!(flat.last().unwrap().id, "commandPalette.input");
    }

    #[test]
    fn test_find_by_id_and_role() {
        let tree = sample_tree();
        let minimap = tree.find_by_id("editor.minimap").unwrap();
        assert_eq!(minimap.role, AccessibleRole::ScrollBar);
        assert_eq!(minimap.name, "Minimap");
        assert!(tree.find_by_id("missing.node").is_none());

        let tabs = tree.find_by_role(AccessibleRole::Tab);
        assert_eq!(tabs.len(), 1);
        assert!(tabs[0].state.selected);
    }

    #[test]
    fn test_focused_node_prefers_deepest_child_and_handles_none() {
        let tree = sample_tree();
        let focused = tree.focused_node().unwrap();
        assert_eq!(focused.id, "editor.main");

        let unfocused = AccessibilityTree::new(AccessibleNode::new(
            "root",
            AccessibleRole::Window,
            "Window",
            Rect::from_xywh(0.0, 0.0, 100.0, 100.0),
        ));
        assert!(unfocused.focused_node().is_none());
    }

    #[test]
    fn test_hit_test_reverse_paint_order_and_miss() {
        let tree = sample_tree();
        // Overlap between editor.main (100..700, 50..450) and commandPalette.input (210..490, 50..78):
        // later overlay wins.
        let hit_overlay = tree.hit_test(Point::new(250.0, 60.0)).unwrap();
        assert_eq!(hit_overlay.id, "commandPalette.input");

        // Inside minimap (616..700, 50..450), outside command palette (200..500)
        let hit_minimap = tree.hit_test(Point::new(650.0, 100.0)).unwrap();
        assert_eq!(hit_minimap.id, "editor.minimap");

        // Outside window bounds
        assert!(tree.hit_test(Point::new(900.0, 100.0)).is_none());
    }

    #[test]
    fn test_serde_roundtrip() {
        let tree = sample_tree();
        let json = serde_json::to_string(&tree).unwrap();
        let decoded: AccessibilityTree = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, tree);
        assert_eq!(AccessibleRole::TextEditor.to_string(), "TextEditor");
    }

    #[test]
    fn test_hit_test_ignores_zero_sized_nodes() {
        let visible_btn = AccessibleNode::new(
            "visible_btn",
            AccessibleRole::Button,
            "Visible Button",
            Rect::from_xywh(80.0, 80.0, 100.0, 40.0),
        );
        let zero_box = AccessibleNode::new(
            "zero_overlay",
            AccessibleRole::Panel,
            "Collapsed",
            Rect::from_xywh(100.0, 100.0, 0.0, 0.0),
        );
        let zero_w = AccessibleNode::new(
            "zero_w_overlay",
            AccessibleRole::Panel,
            "Zero Width",
            Rect::from_xywh(100.0, 90.0, 0.0, 30.0),
        );
        let zero_h = AccessibleNode::new(
            "zero_h_overlay",
            AccessibleRole::Panel,
            "Zero Height",
            Rect::from_xywh(90.0, 100.0, 40.0, 0.0),
        );

        let root = AccessibleNode::new(
            "root",
            AccessibleRole::Window,
            "Window",
            Rect::from_xywh(0.0, 0.0, 400.0, 400.0),
        )
        .with_child(visible_btn)
        .with_child(zero_box)
        .with_child(zero_w)
        .with_child(zero_h);

        let tree = AccessibilityTree::new(root);
        let hit = tree.hit_test(Point::new(100.0, 100.0)).unwrap();
        assert_eq!(hit.id, "visible_btn");
    }
}
