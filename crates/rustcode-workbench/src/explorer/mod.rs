//! File tree model, row projection, and GPUI FileExplorerView component.

use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FileTreeNode {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub children: Option<Vec<FileTreeNode>>,
    pub is_expanded: bool,
}

impl FileTreeNode {
    pub fn new_file(path: PathBuf) -> Self {
        let name = path.file_name().map_or("".to_string(), |n| n.to_string_lossy().to_string());
        Self {
            path,
            name,
            is_dir: false,
            children: None,
            is_expanded: false,
        }
    }

    pub fn new_dir(path: PathBuf) -> Self {
        let name = path.file_name().map_or("".to_string(), |n| n.to_string_lossy().to_string());
        Self {
            path,
            name,
            is_dir: true,
            children: Some(Vec::new()),
            is_expanded: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileExplorer {
    pub root: Option<FileTreeNode>,
    pub expanded_paths: HashSet<PathBuf>,
    pub selected_path: Option<PathBuf>,
}

impl Default for FileExplorer {
    fn default() -> Self {
        Self::new()
    }
}

impl FileExplorer {
    pub fn new() -> Self {
        Self {
            root: None,
            expanded_paths: HashSet::new(),
            selected_path: None,
        }
    }

    pub fn toggle_expanded(&mut self, path: &PathBuf) {
        if self.expanded_paths.contains(path) {
            self.expanded_paths.remove(path);
        } else {
            self.expanded_paths.insert(path.clone());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplorerRow {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub is_expanded: bool,
    pub is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerAction {
    OpenFile(PathBuf),
    ToggleDir(PathBuf),
}

pub struct FileExplorerView {
    pub model: FileExplorer,
    pub last_action: Option<ExplorerAction>,
}

impl Default for FileExplorerView {
    fn default() -> Self {
        Self::new(FileExplorer::new())
    }
}

impl FileExplorerView {
    pub fn new(model: FileExplorer) -> Self {
        Self {
            model,
            last_action: None,
        }
    }

    pub fn flattened_rows(&self) -> Vec<ExplorerRow> {
        let mut rows = Vec::new();
        if let Some(root) = &self.model.root {
            Self::flatten_node(
                root,
                0,
                &self.model.expanded_paths,
                &self.model.selected_path,
                &mut rows,
            );
        }
        rows
    }

    fn flatten_node(
        node: &FileTreeNode,
        depth: usize,
        expanded_paths: &HashSet<PathBuf>,
        selected_path: &Option<PathBuf>,
        rows: &mut Vec<ExplorerRow>,
    ) {
        let is_expanded = expanded_paths.contains(&node.path);
        let is_selected = selected_path.as_ref() == Some(&node.path);

        rows.push(ExplorerRow {
            path: node.path.clone(),
            name: node.name.clone(),
            is_dir: node.is_dir,
            depth,
            is_expanded,
            is_selected,
        });

        if node.is_dir && is_expanded {
            if let Some(children) = &node.children {
                for child in children {
                    Self::flatten_node(child, depth + 1, expanded_paths, selected_path, rows);
                }
            }
        }
    }
}

impl View for FileExplorerView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        // Sidebar background
        scene.push_quad(
            Quad::new(bounds)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.sidebar_border, Edges::new(0.0, 1.0, 0.0, 0.0)),
        );

        // Header section (height 28.0)
        let header_h = 28.0;
        let header_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), header_h);
        scene.push_text(GlyphRun::new(
            "EXPLORER".to_string(),
            Point::new(bounds.min_x() + 12.0, bounds.min_y() + (header_h - font.line_height) / 2.0),
            header_rect,
            theme.sidebar_header_fg,
            font.font_size * 0.85,
        ));

        // Render rows
        let row_h = 22.0;
        let rows = self.flattened_rows();

        for (idx, row) in rows.iter().enumerate() {
            let row_y = bounds.min_y() + header_h + idx as f32 * row_h;
            if row_y + row_h > bounds.max_y() {
                break;
            }

            let row_rect = Rect::from_xywh(bounds.min_x(), row_y, bounds.width(), row_h);

            // Selection background
            if row.is_selected {
                scene.push_quad(Quad::new(row_rect).with_background(theme.list_active_selection_bg));
            }

            // Indentation + chevron + icon + label
            let indent_x = bounds.min_x() + 8.0 + (row.depth as f32 * 14.0);
            let prefix = if row.is_dir {
                if row.is_expanded { "v " } else { "> " }
            } else {
                "  "
            };

            let icon = if row.is_dir { "[D] " } else { "[F] " };
            let full_label = format!("{}{}{}", prefix, icon, row.name);
            let label_w = font.text_width(&full_label);

            scene.push_text(GlyphRun::new(
                full_label,
                Point::new(indent_x, row_y),
                Rect::from_xywh(indent_x, row_y, label_w, row_h),
                theme.sidebar_fg,
                font.font_size,
            ));
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if let InputEvent::MouseDown(mouse) = event {
            if !bounds.contains(mouse.position) {
                return EventResult::Ignored;
            }
            let header_h = 28.0;
            if mouse.position.y < bounds.min_y() + header_h {
                return EventResult::Ignored;
            }

            let row_h = 22.0;
            let row_idx = ((mouse.position.y - (bounds.min_y() + header_h)) / row_h).floor() as usize;
            let rows = self.flattened_rows();

            if let Some(row) = rows.get(row_idx) {
                self.model.selected_path = Some(row.path.clone());
                if row.is_dir {
                    self.model.toggle_expanded(&row.path);
                    self.last_action = Some(ExplorerAction::ToggleDir(row.path.clone()));
                } else {
                    self.last_action = Some(ExplorerAction::OpenFile(row.path.clone()));
                }
                EventResult::Consumed
            } else {
                EventResult::Ignored
            }
        } else {
            EventResult::Ignored
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::render::MouseEvent;

    #[test]
    fn test_explorer_view_hierarchy_and_click() {
        let mut root = FileTreeNode::new_dir(PathBuf::from("/workspace"));
        let mut src = FileTreeNode::new_dir(PathBuf::from("/workspace/src"));
        src.children = Some(vec![FileTreeNode::new_file(PathBuf::from("/workspace/src/main.rs"))]);
        root.children = Some(vec![
            src,
            FileTreeNode::new_file(PathBuf::from("/workspace/Cargo.toml")),
        ]);

        let mut explorer = FileExplorer::new();
        explorer.root = Some(root);
        explorer.expanded_paths.insert(PathBuf::from("/workspace"));
        explorer.expanded_paths.insert(PathBuf::from("/workspace/src"));

        let mut view = FileExplorerView::new(explorer);
        let rows = view.flattened_rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].name, "workspace");
        assert_eq!(rows[1].name, "src");
        assert_eq!(rows[2].name, "main.rs");
        assert_eq!(rows[3].name, "Cargo.toml");

        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let bounds = Rect::from_xywh(0.0, 0.0, 200.0, 600.0);
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("EXPLORER"));
        assert!(scene.contains_text("main.rs"));

        // Click row 2 (main.rs) -> OpenFile action
        let click_y = bounds.min_y() + 28.0 + 2.0 * 22.0 + 5.0;
        let click_ev = InputEvent::MouseDown(MouseEvent::new(
            Point::new(50.0, click_y),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = view.handle_event(&click_ev, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(
            view.last_action,
            Some(ExplorerAction::OpenFile(PathBuf::from("/workspace/src/main.rs")))
        );
    }
}
