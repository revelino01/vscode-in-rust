//! Auto-save service supporting `afterDelay`, `onFocusChange`, and `onWindowChange`.

use crate::document::TextDocument;
use parking_lot::RwLock;
use rustcode_base::event::Emitter;
use rustcode_platform::fs::{FileMetadata, FileSystemError, FileSystemService};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoSaveMode {
    Off,
    AfterDelay,
    OnFocusChange,
    OnWindowChange,
}

#[derive(Debug, Clone)]
pub struct AutoSaveConfig {
    pub mode: AutoSaveMode,
    pub delay: Duration,
}

impl Default for AutoSaveConfig {
    fn default() -> Self {
        Self {
            mode: AutoSaveMode::AfterDelay,
            delay: Duration::from_millis(1000), // Default 1000ms delay like VS Code
        }
    }
}

pub struct AutoSaveService {
    config: AutoSaveConfig,
    documents: HashMap<PathBuf, Arc<RwLock<TextDocument>>>,
    pending_delay_saves: HashMap<PathBuf, Instant>,
    pub on_did_auto_save: Emitter<PathBuf>,
}

impl Default for AutoSaveService {
    fn default() -> Self {
        Self::new(AutoSaveConfig::default())
    }
}

impl AutoSaveService {
    pub fn new(config: AutoSaveConfig) -> Self {
        Self {
            config,
            documents: HashMap::new(),
            pending_delay_saves: HashMap::new(),
            on_did_auto_save: Emitter::new(),
        }
    }

    pub fn config(&self) -> &AutoSaveConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: AutoSaveConfig) {
        self.config = config;
    }

    pub fn register(&mut self, doc: Arc<RwLock<TextDocument>>) {
        let path = doc.read().path.clone();
        if let Some(p) = path {
            self.documents.insert(p, doc);
        }
    }

    pub fn unregister(&mut self, path: &Path) {
        self.documents.remove(path);
        self.pending_delay_saves.remove(path);
    }

    /// Notify that document content changed.
    /// If mode is `AfterDelay`, schedules a save deadline.
    pub fn on_document_content_changed(&mut self, path: &Path) {
        if self.config.mode == AutoSaveMode::AfterDelay && self.documents.contains_key(path) {
            let deadline = Instant::now() + self.config.delay;
            self.pending_delay_saves.insert(path.to_path_buf(), deadline);
        }
    }

    /// Periodic tick for `AfterDelay` mode to save expired documents.
    pub fn tick(&mut self, fs: &FileSystemService) -> Vec<Result<FileMetadata, FileSystemError>> {
        if self.config.mode != AutoSaveMode::AfterDelay {
            return Vec::new();
        }

        let now = Instant::now();
        let ready_paths: Vec<PathBuf> = self
            .pending_delay_saves
            .iter()
            .filter(|(_, &deadline)| now >= deadline)
            .map(|(p, _)| p.clone())
            .collect();

        let mut results = Vec::new();
        for path in ready_paths {
            self.pending_delay_saves.remove(&path);
            if let Some(doc_lock) = self.documents.get(&path) {
                let mut doc = doc_lock.write();
                if doc.is_dirty {
                    let res = doc.save_sync(fs);
                    if res.is_ok() {
                        self.on_did_auto_save.fire(path);
                    }
                    results.push(res);
                }
            }
        }
        results
    }

    /// Focus lost on a specific editor / document.
    pub fn on_focus_lost(
        &mut self,
        path: &Path,
        fs: &FileSystemService,
    ) -> Option<Result<FileMetadata, FileSystemError>> {
        if matches!(
            self.config.mode,
            AutoSaveMode::OnFocusChange | AutoSaveMode::AfterDelay
        ) {
            self.pending_delay_saves.remove(path);
            if let Some(doc_lock) = self.documents.get(path) {
                let mut doc = doc_lock.write();
                if doc.is_dirty {
                    let res = doc.save_sync(fs);
                    if res.is_ok() {
                        self.on_did_auto_save.fire(path.to_path_buf());
                    }
                    return Some(res);
                }
            }
        }
        None
    }

    /// Window blur / focus lost across the entire application.
    pub fn on_window_blur(
        &mut self,
        fs: &FileSystemService,
    ) -> Vec<Result<FileMetadata, FileSystemError>> {
        if matches!(
            self.config.mode,
            AutoSaveMode::OnWindowChange | AutoSaveMode::OnFocusChange | AutoSaveMode::AfterDelay
        ) {
            self.pending_delay_saves.clear();
            return self.save_all_dirty(fs);
        }
        Vec::new()
    }

    /// Force immediate save of all dirty managed documents.
    pub fn save_all_dirty(
        &mut self,
        fs: &FileSystemService,
    ) -> Vec<Result<FileMetadata, FileSystemError>> {
        let mut results = Vec::new();
        for (path, doc_lock) in &self.documents {
            let mut doc = doc_lock.write();
            if doc.is_dirty {
                let res = doc.save_sync(fs);
                if res.is_ok() {
                    self.on_did_auto_save.fire(path.clone());
                }
                results.push(res);
            }
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_save_after_delay_and_focus_change() {
        let tmp_dir = std::env::temp_dir().join(format!("rustcode_autosave_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path1 = tmp_dir.join("file1.rs");
        let path2 = tmp_dir.join("file2.rs");

        let fs = FileSystemService::new();
        let doc1 = Arc::new(RwLock::new(TextDocument::new(
            "initial 1",
            Some(path1.clone()),
            Some("rust"),
        )));
        let doc2 = Arc::new(RwLock::new(TextDocument::new(
            "initial 2",
            Some(path2.clone()),
            Some("rust"),
        )));

        let mut auto_save = AutoSaveService::new(AutoSaveConfig {
            mode: AutoSaveMode::AfterDelay,
            delay: Duration::from_millis(50),
        });

        auto_save.register(doc1.clone());
        auto_save.register(doc2.clone());

        // Edit doc1
        doc1.write().buffer.write().insert(9, " edited");
        doc1.write().mark_edited();
        assert!(doc1.read().is_dirty);
        auto_save.on_document_content_changed(&path1);

        // Before delay: tick should not save
        let early = auto_save.tick(&fs);
        assert!(early.is_empty());
        assert!(doc1.read().is_dirty);

        // Sleep past delay
        std::thread::sleep(Duration::from_millis(60));
        let saved = auto_save.tick(&fs);
        assert_eq!(saved.len(), 1);
        assert!(!doc1.read().is_dirty);
        assert_eq!(std::fs::read_to_string(&path1).unwrap(), "initial 1 edited");

        // Test OnFocusChange mode
        auto_save.set_config(AutoSaveConfig {
            mode: AutoSaveMode::OnFocusChange,
            delay: Duration::from_millis(1000),
        });

        doc2.write().buffer.write().insert(9, " edited2");
        doc2.write().mark_edited();
        assert!(doc2.read().is_dirty);

        // Losing focus triggers save
        let res = auto_save.on_focus_lost(&path2, &fs);
        assert!(res.is_some());
        assert!(!doc2.read().is_dirty);
        assert_eq!(std::fs::read_to_string(&path2).unwrap(), "initial 2 edited2");

        // Test OnWindowChange mode
        auto_save.set_config(AutoSaveConfig {
            mode: AutoSaveMode::OnWindowChange,
            delay: Duration::from_millis(1000),
        });

        doc1.write().buffer.write().insert(16, " more");
        doc1.write().mark_edited();
        assert!(doc1.read().is_dirty);

        let blur_saved = auto_save.on_window_blur(&fs);
        assert_eq!(blur_saved.len(), 1);
        assert!(!doc1.read().is_dirty);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
