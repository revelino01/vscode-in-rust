//! File tree model, row projection, and GPUI FileExplorerView component.

use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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

    pub fn from_path(path: PathBuf) -> Self {
        Self::from_path_with_depth(path, 0, 15)
    }

    pub fn from_path_with_depth(path: PathBuf, depth: usize, max_depth: usize) -> Self {
        if path.is_dir() {
            let mut node = Self::new_dir(path.clone());
            if depth >= max_depth {
                return node;
            }
            if let Ok(entries) = std::fs::read_dir(&path) {
                let mut children = Vec::new();
                for entry in entries.flatten() {
                    let p = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    if crate::search::engine::is_ignored_directory(&name) {
                        continue;
                    }
                    if p.is_dir() {
                        children.push(Self::from_path_with_depth(p, depth + 1, max_depth));
                    } else {
                        children.push(Self::new_file(p));
                    }
                }
                children.sort_by(|a, b| {
                    if a.is_dir == b.is_dir {
                        a.name.cmp(&b.name)
                    } else if a.is_dir {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    }
                });
                node.children = Some(children);
            }
            node
        } else {
            Self::new_file(path)
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileExplorer {
    pub roots: Vec<FileTreeNode>,
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
            roots: Vec::new(),
            root: None,
            expanded_paths: HashSet::new(),
            selected_path: None,
        }
    }

    pub fn set_roots(&mut self, roots: Vec<FileTreeNode>) {
        self.root = roots.first().cloned();
        self.roots = roots;
    }

    pub fn add_root(&mut self, root: FileTreeNode) {
        if !self.roots.iter().any(|r| r.path == root.path) {
            self.roots.push(root);
            if self.root.is_none() {
                self.root = self.roots.first().cloned();
            }
        }
    }

    pub fn remove_root(&mut self, path: &Path) {
        self.roots.retain(|r| r.path != path);
        if self.root.as_ref().map(|r| &r.path) == Some(&path.to_path_buf()) {
            self.root = self.roots.first().cloned();
        }
    }

    pub fn sync_from_workspace(&mut self, ws: &crate::workspace::Workspace) {
        let mut new_roots = Vec::new();
        for folder in ws.folders() {
            new_roots.push(FileTreeNode::from_path(folder.path.clone()));
        }
        self.set_roots(new_roots);
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
        if !self.model.roots.is_empty() {
            for root in &self.model.roots {
                Self::flatten_node(
                    root,
                    0,
                    &self.model.expanded_paths,
                    &self.model.selected_path,
                    &mut rows,
                );
            }
        } else if let Some(root) = &self.model.root {
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

    #[test]
    fn test_multi_root_file_explorer() {
        let mut root1 = FileTreeNode::new_dir(PathBuf::from("/backend"));
        root1.children = Some(vec![FileTreeNode::new_file(PathBuf::from("/backend/main.rs"))]);

        let mut root2 = FileTreeNode::new_dir(PathBuf::from("/frontend"));
        root2.children = Some(vec![FileTreeNode::new_file(PathBuf::from("/frontend/index.ts"))]);

        let mut explorer = FileExplorer::new();
        explorer.set_roots(vec![root1, root2]);
        explorer.expanded_paths.insert(PathBuf::from("/backend"));
        explorer.expanded_paths.insert(PathBuf::from("/frontend"));

        let view = FileExplorerView::new(explorer);
        let rows = view.flattened_rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].name, "backend");
        assert_eq!(rows[1].name, "main.rs");
        assert_eq!(rows[2].name, "frontend");
        assert_eq!(rows[3].name, "index.ts");
    }

    #[test]
    fn test_file_explorer_recursive_subdirectory_expansion() {
        let temp_dir = std::env::temp_dir().join(format!("rustcode_explorer_test_{}", uuid::Uuid::new_v4().simple()));
        let sub_dir = temp_dir.join("src");
        let nested_dir = sub_dir.join("nested");
        std::fs::create_dir_all(&nested_dir).unwrap();
        let main_file = sub_dir.join("main.rs");
        let deep_file = nested_dir.join("deep.rs");
        let root_file = temp_dir.join("Cargo.toml");
        std::fs::write(&main_file, "fn main() {}\n").unwrap();
        std::fs::write(&deep_file, "// deep\n").unwrap();
        std::fs::write(&root_file, "[package]\n").unwrap();

        let root_node = FileTreeNode::from_path(temp_dir.clone());
        assert!(root_node.is_dir);

        let mut explorer = FileExplorer::new();
        explorer.set_roots(vec![root_node]);
        // Expand root and sub_dir, keep nested_dir collapsed
        explorer.expanded_paths.insert(temp_dir.clone());
        explorer.expanded_paths.insert(sub_dir.clone());

        let view = FileExplorerView::new(explorer);
        let rows = view.flattened_rows();

        // Subdirectory and its file must be visible in flattened rows
        assert!(rows.iter().any(|r| r.name == "src" && r.is_dir));
        assert!(rows.iter().any(|r| r.name == "main.rs" && !r.is_dir));
        assert!(rows.iter().any(|r| r.name == "Cargo.toml" && !r.is_dir));
        assert!(rows.iter().any(|r| r.name == "nested" && r.is_dir));
        // Deep file should not be visible yet while nested is collapsed
        assert!(!rows.iter().any(|r| r.name == "deep.rs"));

        // Now expand nested_dir
        let mut explorer2 = view.model.clone();
        explorer2.expanded_paths.insert(nested_dir.clone());
        let view2 = FileExplorerView::new(explorer2);
        let rows2 = view2.flattened_rows();
        assert!(rows2.iter().any(|r| r.name == "deep.rs" && !r.is_dir));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
