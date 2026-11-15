//! File system abstraction and watcher event definitions.

use rustcode_base::event::Emitter;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChangeType {
    Created,
    Changed,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChangeEvent {
    pub path: PathBuf,
    pub change_type: FileChangeType,
}

pub struct FileSystemService {
    pub on_did_change: Emitter<FileChangeEvent>,
}

impl Default for FileSystemService {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystemService {
    pub fn new() -> Self {
        Self {
            on_did_change: Emitter::new(),
        }
    }
}
