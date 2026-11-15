//! File tree model for the primary sidebar Explorer.

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
