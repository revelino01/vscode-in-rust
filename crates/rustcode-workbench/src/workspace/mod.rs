//! Multi-Root Workspace model representing workspace folders and configurations.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Represents a folder in a multi-root workspace (mirroring VS Code's `WorkspaceFolder`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFolder {
    pub uri: String,
    pub name: String,
    pub path: PathBuf,
    pub index: usize,
}

impl WorkspaceFolder {
    pub fn new(path: PathBuf, name: Option<String>, index: usize) -> Self {
        let folder_name = name.unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("folder_{}", index))
        });

        let uri = if path.to_string_lossy().starts_with("file://") {
            path.to_string_lossy().to_string()
        } else {
            format!("file://{}", path.to_string_lossy())
        };

        Self {
            uri,
            name: folder_name,
            path,
            index,
        }
    }

    /// Check if this workspace folder contains the given file path.
    pub fn contains_path(&self, file_path: &Path) -> bool {
        file_path.starts_with(&self.path)
    }

    /// Returns the relative path of `file_path` with respect to this workspace folder.
    pub fn relative_path(&self, file_path: &Path) -> Option<PathBuf> {
        file_path.strip_prefix(&self.path).ok().map(|p| p.to_path_buf())
    }
}

/// Represents the active workspace, which can contain one or more workspace folders.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub folders: Vec<WorkspaceFolder>,
    pub configuration_file: Option<PathBuf>,
}

impl Workspace {
    pub fn new(name: impl Into<String>) -> Self {
        let name_str = name.into();
        let id = uuid::Uuid::new_v4().to_string();
        Self {
            id,
            name: name_str,
            folders: Vec::new(),
            configuration_file: None,
        }
    }

    /// Creates a single-root workspace from a folder path.
    pub fn from_folder(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Workspace".to_string());
        let mut ws = Self::new(name);
        ws.add_folder(path, None);
        ws
    }

    /// Creates a multi-root workspace from a collection of folder paths.
    pub fn from_folders(paths: Vec<PathBuf>) -> Self {
        let mut ws = Self::new("Multi-Root Workspace");
        for p in paths {
            ws.add_folder(p, None);
        }
        ws
    }

    /// Adds a folder to the workspace. If the path already exists, returns the existing folder reference.
    pub fn add_folder(&mut self, path: PathBuf, name: Option<String>) -> &WorkspaceFolder {
        if let Some(idx) = self.folders.iter().position(|f| f.path == path) {
            return &self.folders[idx];
        }

        let index = self.folders.len();
        let folder = WorkspaceFolder::new(path, name, index);
        self.folders.push(folder);
        self.folders.last().expect("folder just pushed")
    }

    /// Removes a folder by path, reindexing the remaining folders.
    pub fn remove_folder(&mut self, path: &Path) -> Option<WorkspaceFolder> {
        let idx = self.folders.iter().position(|f| f.path == path)?;
        let removed = self.folders.remove(idx);
        self.reindex_folders();
        Some(removed)
    }

    /// Removes a folder by URI, reindexing the remaining folders.
    pub fn remove_folder_by_uri(&mut self, uri: &str) -> Option<WorkspaceFolder> {
        let idx = self.folders.iter().position(|f| f.uri == uri)?;
        let removed = self.folders.remove(idx);
        self.reindex_folders();
        Some(removed)
    }

    /// Finds the workspace folder that contains the given path.
    /// If nested folders exist, chooses the longest (deepest) matching prefix.
    pub fn get_folder_for_path(&self, path: &Path) -> Option<&WorkspaceFolder> {
        self.folders
            .iter()
            .filter(|f| f.contains_path(path))
            .max_by_key(|f| f.path.as_os_str().len())
    }

    /// Finds a workspace folder by its exact URI.
    pub fn get_folder_by_uri(&self, uri: &str) -> Option<&WorkspaceFolder> {
        self.folders.iter().find(|f| f.uri == uri)
    }

    /// Returns whether this workspace has multiple roots.
    pub fn is_multi_root(&self) -> bool {
        self.folders.len() > 1
    }

    /// Returns a slice of all workspace folders.
    pub fn folders(&self) -> &[WorkspaceFolder] {
        &self.folders
    }

    /// Returns an iterator over all workspace folders.
    pub fn iter(&self) -> impl Iterator<Item = &WorkspaceFolder> {
        self.folders.iter()
    }

    /// Checks if any workspace folder contains the given path.
    pub fn contains_path(&self, path: &Path) -> bool {
        self.folders.iter().any(|f| f.contains_path(path))
    }

    fn reindex_folders(&mut self) {
        for (i, folder) in self.folders.iter_mut().enumerate() {
            folder.index = i;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_and_multi_root_workspace() {
        let mut ws = Workspace::new("TestWorkspace");
        assert!(!ws.is_multi_root());
        assert_eq!(ws.folders().len(), 0);

        // Add first folder
        ws.add_folder(PathBuf::from("/project/backend"), Some("Backend".to_string()));
        assert_eq!(ws.folders().len(), 1);
        assert!(!ws.is_multi_root());

        // Add second folder
        ws.add_folder(PathBuf::from("/project/frontend"), Some("Frontend".to_string()));
        assert_eq!(ws.folders().len(), 2);
        assert!(ws.is_multi_root());

        // Test path resolution
        let file1 = PathBuf::from("/project/backend/src/main.rs");
        let folder1 = ws.get_folder_for_path(&file1).expect("found backend folder");
        assert_eq!(folder1.name, "Backend");
        assert_eq!(folder1.index, 0);
        assert_eq!(
            folder1.relative_path(&file1),
            Some(PathBuf::from("src/main.rs"))
        );

        let file2 = PathBuf::from("/project/frontend/package.json");
        let folder2 = ws.get_folder_for_path(&file2).expect("found frontend folder");
        assert_eq!(folder2.name, "Frontend");
        assert_eq!(folder2.index, 1);

        // Outside path
        assert!(ws.get_folder_for_path(Path::new("/var/log")).is_none());
        assert!(ws.contains_path(&file1));
        assert!(!ws.contains_path(Path::new("/var/log")));

        // Remove folder and reindex
        let removed = ws.remove_folder(Path::new("/project/backend")).unwrap();
        assert_eq!(removed.name, "Backend");
        assert_eq!(ws.folders().len(), 1);
        assert!(!ws.is_multi_root());
        assert_eq!(ws.folders()[0].index, 0);
        assert_eq!(ws.folders()[0].name, "Frontend");
    }

    #[test]
    fn test_nested_folders_longest_prefix_match() {
        let mut ws = Workspace::new("NestedWorkspace");
        ws.add_folder(PathBuf::from("/monorepo"), Some("Root".to_string()));
        ws.add_folder(PathBuf::from("/monorepo/packages/core"), Some("Core".to_string()));

        let file = PathBuf::from("/monorepo/packages/core/src/index.ts");
        let matched = ws.get_folder_for_path(&file).expect("matched folder");
        assert_eq!(matched.name, "Core");
    }
}
