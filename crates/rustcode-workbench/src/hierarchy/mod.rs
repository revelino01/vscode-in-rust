//! Call Hierarchy and Type Hierarchy view for LSP M1 navigation.

use parking_lot::Mutex;
use rustcode_editor::{Position, Range};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, MouseButton, Point, Quad, Rect,
    Scene, View, WorkbenchThemeStyles,
};
use rustcode_protocols::lsp::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, LspRange, SymbolKind,
    TypeHierarchyItem,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HierarchyMode {
    CallIncoming,
    CallOutgoing,
    TypeSupertypes,
    TypeSubtypes,
}

impl HierarchyMode {
    pub fn title(&self) -> &'static str {
        match self {
            Self::CallIncoming => "CALL HIERARCHY: INCOMING",
            Self::CallOutgoing => "CALL HIERARCHY: OUTGOING",
            Self::TypeSupertypes => "TYPE HIERARCHY: SUPERTYPES",
            Self::TypeSubtypes => "TYPE HIERARCHY: SUBTYPES",
        }
    }

    pub fn is_call(&self) -> bool {
        matches!(self, Self::CallIncoming | Self::CallOutgoing)
    }

    pub fn opposite(&self) -> Self {
        match self {
            Self::CallIncoming => Self::CallOutgoing,
            Self::CallOutgoing => Self::CallIncoming,
            Self::TypeSupertypes => Self::TypeSubtypes,
            Self::TypeSubtypes => Self::TypeSupertypes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchyItemNode {
    pub name: String,
    pub detail: Option<String>,
    pub kind_label: String,
    pub uri: String,
    pub range: Range,
    pub selection_range: Range,
    pub children: Vec<HierarchyItemNode>,
    pub expanded: bool,
    pub loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatHierarchyRow {
    pub depth: usize,
    pub label: String,
    pub detail: String,
    pub uri: String,
    pub range: Range,
    pub has_children: bool,
    pub expanded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HierarchyAction {
    NavigateTo { uri: String, range: Range },
    SwitchMode(HierarchyMode),
}

fn lsp_range_to_editor(r: LspRange) -> Range {
    Range::new(
        Position::new(r.start.line as usize, r.start.character as usize),
        Position::new(r.end.line as usize, r.end.character as usize),
    )
}

fn symbol_kind_label(kind: &SymbolKind) -> String {
    match kind {
        SymbolKind::Function | SymbolKind::Method => "fn".to_string(),
        SymbolKind::Struct => "struct".to_string(),
        SymbolKind::Enum => "enum".to_string(),
        SymbolKind::Interface => "trait".to_string(),
        SymbolKind::Class => "class".to_string(),
        SymbolKind::Module => "mod".to_string(),
        _ => "sym".to_string(),
    }
}

fn call_item_to_node(item: &CallHierarchyItem, expanded: bool, loaded: bool) -> HierarchyItemNode {
    HierarchyItemNode {
        name: item.name.clone(),
        detail: item.detail.clone(),
        kind_label: symbol_kind_label(&item.kind),
        uri: item.uri.clone(),
        range: lsp_range_to_editor(item.range),
        selection_range: lsp_range_to_editor(item.selection_range),
        children: Vec::new(),
        expanded,
        loaded,
    }
}

fn type_item_to_node(item: &TypeHierarchyItem, expanded: bool, loaded: bool) -> HierarchyItemNode {
    HierarchyItemNode {
        name: item.name.clone(),
        detail: item.detail.clone(),
        kind_label: symbol_kind_label(&item.kind),
        uri: item.uri.clone(),
        range: lsp_range_to_editor(item.range),
        selection_range: lsp_range_to_editor(item.selection_range),
        children: Vec::new(),
        expanded,
        loaded,
    }
}

pub struct HierarchyView {
    pub is_open: bool,
    pub mode: HierarchyMode,
    pub root: Option<HierarchyItemNode>,
    pub raw_call_root: Option<CallHierarchyItem>,
    pub raw_type_root: Option<TypeHierarchyItem>,
    pub flat_rows: Vec<FlatHierarchyRow>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub last_action: Option<HierarchyAction>,
    pub last_bounds: Mutex<Option<Rect>>,
}

impl Default for HierarchyView {
    fn default() -> Self {
        Self::new()
    }
}

impl HierarchyView {
    pub const HEADER_HEIGHT: f32 = 28.0;
    pub const ROW_HEIGHT: f32 = 22.0;

    pub fn new() -> Self {
        Self {
            is_open: false,
            mode: HierarchyMode::CallIncoming,
            root: None,
            raw_call_root: None,
            raw_type_root: None,
            flat_rows: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            last_action: None,
            last_bounds: Mutex::new(None),
        }
    }

    pub fn clear(&mut self) {
        self.is_open = false;
        self.root = None;
        self.raw_call_root = None;
        self.raw_type_root = None;
        self.flat_rows.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    pub fn set_call_incoming(
        &mut self,
        root: CallHierarchyItem,
        calls: Vec<CallHierarchyIncomingCall>,
    ) {
        self.mode = HierarchyMode::CallIncoming;
        self.raw_call_root = Some(root.clone());
        let mut node = call_item_to_node(&root, true, true);
        node.children = calls
            .into_iter()
            .map(|c| {
                let mut child = call_item_to_node(&c.from, false, false);
                if let Some(first_range) = c.from_ranges.first() {
                    child.selection_range = lsp_range_to_editor(*first_range);
                }
                child
            })
            .collect();
        self.root = Some(node);
        self.is_open = true;
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.rebuild_flat_rows();
    }

    pub fn set_call_outgoing(
        &mut self,
        root: CallHierarchyItem,
        calls: Vec<CallHierarchyOutgoingCall>,
    ) {
        self.mode = HierarchyMode::CallOutgoing;
        self.raw_call_root = Some(root.clone());
        let mut node = call_item_to_node(&root, true, true);
        node.children = calls
            .into_iter()
            .map(|c| call_item_to_node(&c.to, false, false))
            .collect();
        self.root = Some(node);
        self.is_open = true;
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.rebuild_flat_rows();
    }

    pub fn set_type_hierarchy(
        &mut self,
        mode: HierarchyMode,
        root: TypeHierarchyItem,
        relatives: Vec<TypeHierarchyItem>,
    ) {
        self.mode = if matches!(mode, HierarchyMode::TypeSubtypes) {
            HierarchyMode::TypeSubtypes
        } else {
            HierarchyMode::TypeSupertypes
        };
        self.raw_type_root = Some(root.clone());
        let mut node = type_item_to_node(&root, true, true);
        node.children = relatives
            .into_iter()
            .map(|item| type_item_to_node(&item, false, false))
            .collect();
        self.root = Some(node);
        self.is_open = true;
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.rebuild_flat_rows();
    }

    pub fn rebuild_flat_rows(&mut self) {
        self.flat_rows.clear();
        if let Some(root) = &self.root {
            Self::flatten_node(root, 0, &mut self.flat_rows);
        }
        if self.selected_index >= self.flat_rows.len() {
            self.selected_index = self.flat_rows.len().saturating_sub(1);
        }
    }

    fn flatten_node(
        node: &HierarchyItemNode,
        depth: usize,
        out: &mut Vec<FlatHierarchyRow>,
    ) {
        out.push(FlatHierarchyRow {
            depth,
            label: format!("[{}] {}", node.kind_label, node.name),
            detail: node.detail.clone().unwrap_or_default(),
            uri: node.uri.clone(),
            range: node.selection_range,
            has_children: !node.children.is_empty(),
            expanded: node.expanded,
        });
        if node.expanded {
            for child in &node.children {
                Self::flatten_node(child, depth + 1, out);
            }
        }
    }

    pub fn selected_row(&self) -> Option<&FlatHierarchyRow> {
        self.flat_rows.get(self.selected_index)
    }
}

impl View for HierarchyView {
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
        *self.last_bounds.lock() = Some(bounds);

        scene.push_finite_quad(Quad::new(bounds).with_background(theme.sidebar_bg));
        let header = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), Self::HEADER_HEIGHT);
        scene.push_finite_quad(
            Quad::new(header)
                .with_background(theme.panel_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );
        scene.push_finite_text(GlyphRun::new(
            self.mode.title().to_string(),
            Point::new(header.min_x() + 8.0, header.min_y() + 6.0),
            Rect::from_xywh(header.min_x() + 8.0, header.min_y() + 4.0, (header.width() - 72.0).max(20.0), font.line_height),
            theme.editor_foreground,
            font.font_size * 0.85,
        ));

        // Switch direction button at top-right of header
        let toggle_label = "Flip";
        let btn_rect = Rect::from_xywh(header.max_x() - 48.0, header.min_y() + 4.0, 40.0, Self::HEADER_HEIGHT - 8.0);
        if btn_rect.is_finite_positive() {
            scene.push_finite_quad(
                Quad::new(btn_rect)
                    .with_background(theme.activity_bar_bg)
                    .with_border_edges(theme.panel_border, Edges::all(1.0))
                    .with_corner_radius(3.0),
            );
            scene.push_finite_text(GlyphRun::new(
                toggle_label.to_string(),
                Point::new(btn_rect.min_x() + 6.0, btn_rect.min_y() + 2.0),
                btn_rect,
                theme.editor_foreground,
                font.font_size * 0.8,
            ));
        }

        if self.flat_rows.is_empty() {
            scene.push_finite_text(GlyphRun::new(
                "No hierarchy results".to_string(),
                Point::new(bounds.min_x() + 12.0, bounds.min_y() + Self::HEADER_HEIGHT + 12.0),
                Rect::from_xywh(bounds.min_x() + 12.0, bounds.min_y() + Self::HEADER_HEIGHT + 12.0, (bounds.width() - 24.0).max(20.0), font.line_height),
                theme.editor_line_number,
                font.font_size,
            ));
            return;
        }

        let list_y = bounds.min_y() + Self::HEADER_HEIGHT;
        let max_rows = ((bounds.height() - Self::HEADER_HEIGHT).max(0.0) / Self::ROW_HEIGHT).floor() as usize;
        for (vis_idx, row) in self
            .flat_rows
            .iter()
            .skip(self.scroll_offset)
            .take(max_rows.max(1))
            .enumerate()
        {
            let actual_idx = self.scroll_offset + vis_idx;
            let ry = list_y + vis_idx as f32 * Self::ROW_HEIGHT;
            let rrect = Rect::from_xywh(bounds.min_x(), ry, bounds.width(), Self::ROW_HEIGHT);
            if actual_idx == self.selected_index {
                scene.push_finite_quad(Quad::new(rrect).with_background(theme.editor_selection));
            }
            let indent = 8.0 + row.depth as f32 * 14.0;
            let prefix = if row.has_children {
                if row.expanded { "▾ " } else { "▸ " }
            } else {
                "  "
            };
            let text = if row.detail.is_empty() {
                format!("{}{}", prefix, row.label)
            } else {
                format!("{}{} ({})", prefix, row.label, row.detail)
            };
            scene.push_finite_text(GlyphRun::new(
                text,
                Point::new(bounds.min_x() + indent, ry + 3.0),
                Rect::from_xywh(bounds.min_x() + indent, ry + 3.0, (bounds.width() - indent - 8.0).max(16.0), Self::ROW_HEIGHT),
                theme.editor_foreground,
                font.font_size * 0.9,
            ));
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if !bounds.is_finite_positive() {
            return EventResult::Ignored;
        }
        match event {
            InputEvent::MouseDown(mouse) if mouse.button == MouseButton::Left => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }
                let btn_rect = Rect::from_xywh(bounds.max_x() - 48.0, bounds.min_y() + 4.0, 40.0, Self::HEADER_HEIGHT - 8.0);
                if btn_rect.contains(mouse.position) {
                    let next_mode = self.mode.opposite();
                    self.last_action = Some(HierarchyAction::SwitchMode(next_mode));
                    return EventResult::Consumed;
                }
                if mouse.position.y >= bounds.min_y() + Self::HEADER_HEIGHT {
                    let idx = self.scroll_offset
                        + ((mouse.position.y - bounds.min_y() - Self::HEADER_HEIGHT) / Self::ROW_HEIGHT)
                            .floor()
                            .max(0.0) as usize;
                    if let Some(row) = self.flat_rows.get(idx) {
                        self.selected_index = idx;
                        self.last_action = Some(HierarchyAction::NavigateTo {
                            uri: row.uri.clone(),
                            range: row.range,
                        });
                    }
                }
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => match key.key {
                KeyCode::ArrowDown => {
                    if !self.flat_rows.is_empty() {
                        self.selected_index = (self.selected_index + 1).min(self.flat_rows.len() - 1);
                    }
                    EventResult::Consumed
                }
                KeyCode::ArrowUp => {
                    self.selected_index = self.selected_index.saturating_sub(1);
                    EventResult::Consumed
                }
                KeyCode::Enter => {
                    if let Some(row) = self.flat_rows.get(self.selected_index) {
                        self.last_action = Some(HierarchyAction::NavigateTo {
                            uri: row.uri.clone(),
                            range: row.range,
                        });
                    }
                    EventResult::Consumed
                }
                _ => EventResult::Ignored,
            },
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_protocols::lsp::LspPosition;

    #[test]
    fn test_hierarchy_view_call_and_type_modes() {
        let mut view = HierarchyView::new();
        let root = CallHierarchyItem {
            name: "compute".to_string(),
            kind: SymbolKind::Function,
            tags: None,
            detail: Some("src/lib.rs".to_string()),
            uri: "file:///src/lib.rs".to_string(),
            range: LspRange::new(LspPosition::new(1, 0), LspPosition::new(4, 1)),
            selection_range: LspRange::new(LspPosition::new(1, 3), LspPosition::new(1, 10)),
            data: None,
        };
        let caller = CallHierarchyIncomingCall {
            from: CallHierarchyItem {
                name: "main".to_string(),
                kind: SymbolKind::Function,
                tags: None,
                detail: Some("src/main.rs".to_string()),
                uri: "file:///src/main.rs".to_string(),
                range: LspRange::new(LspPosition::new(10, 0), LspPosition::new(14, 1)),
                selection_range: LspRange::new(LspPosition::new(10, 3), LspPosition::new(10, 7)),
                data: None,
            },
            from_ranges: vec![LspRange::new(LspPosition::new(12, 4), LspPosition::new(12, 11))],
        };
        view.set_call_incoming(root, vec![caller]);
        assert_eq!(view.flat_rows.len(), 2);
        assert_eq!(view.flat_rows[1].depth, 1);

        let bounds = Rect::from_xywh(0.0, 0.0, 300.0, 400.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("CALL HIERARCHY: INCOMING"));
        assert!(scene.contains_text("main"));
    }
}
