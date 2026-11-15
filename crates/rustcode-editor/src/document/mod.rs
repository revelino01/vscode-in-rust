//! Document management, dirty tracking, versioning, conflict detection, and file persistence.

pub mod auto_save;

pub use auto_save::{AutoSaveConfig, AutoSaveMode, AutoSaveService};

use crate::model::piece_table::PieceTable;
use parking_lot::RwLock;
use rustcode_base::event::Emitter;
use rustcode_platform::fs::{
    FileConflict, FileMetadata, FileSystemError, FileSystemService,
};

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A managed text document backed by a Piece Table, tracking dirty state,
/// version history, disk metadata, and file conflict status.
pub struct TextDocument {
    pub buffer: Arc<RwLock<PieceTable>>,
    pub path: Option<PathBuf>,
    pub metadata: Option<FileMetadata>,
    pub is_dirty: bool,
    pub version_id: u64,
    pub saved_version_id: u64,
    pub language_id: String,
    pub on_did_change_dirty: Emitter<bool>,
    pub on_did_save: Emitter<PathBuf>,
}

impl TextDocument {
    pub fn new(content: &str, path: Option<PathBuf>, language_id: Option<&str>) -> Self {
        let lang = language_id
            .map(|s| s.to_string())
            .or_else(|| {
                path.as_ref()
                    .and_then(|p| p.extension())
                    .and_then(|e| e.to_str())
                    .map(map_ext_to_lang_id)
            })
            .unwrap_or_else(|| "plaintext".to_string());


        Self {
            buffer: Arc::new(RwLock::new(PieceTable::from_str(content))),
            path,
            metadata: None,
            is_dirty: false,
            version_id: 1,
            saved_version_id: 1,
            language_id: lang,
            on_did_change_dirty: Emitter::new(),
            on_did_save: Emitter::new(),
        }
    }

    /// Open an existing file from disk synchronously.
    pub fn open_sync<P: AsRef<Path>>(
        _fs: &FileSystemService,
        path: P,
    ) -> Result<Self, FileSystemError> {

        let p = path.as_ref();
        let content = FileSystemService::read_to_string_sync(p)?;
        let meta = FileSystemService::compute_metadata(p, &content)?;

        let mut doc = Self::new(&content, Some(p.to_path_buf()), None);
        doc.metadata = Some(meta);
        doc.is_dirty = false;
        doc.saved_version_id = doc.version_id;
        Ok(doc)
    }

    /// Open an existing file from disk asynchronously.
    pub async fn open_async<P: AsRef<Path>>(path: P) -> Result<Self, FileSystemError> {
        let p = path.as_ref();
        let content = FileSystemService::read_to_string(p).await?;
        let meta = FileSystemService::compute_metadata(p, &content)?;

        let mut doc = Self::new(&content, Some(p.to_path_buf()), None);
        doc.metadata = Some(meta);
        doc.is_dirty = false;
        doc.saved_version_id = doc.version_id;
        Ok(doc)
    }

    pub fn text(&self) -> String {
        self.buffer.read().to_string()
    }

    /// Notify document that buffer content has changed.
    pub fn mark_edited(&mut self) {
        self.version_id = self.version_id.wrapping_add(1);
        let was_dirty = self.is_dirty;
        self.is_dirty = self.version_id != self.saved_version_id;

        if was_dirty != self.is_dirty {
            self.on_did_change_dirty.fire(self.is_dirty);
        }
    }

    /// Mark document as saved with fresh disk metadata.
    pub fn mark_saved(&mut self, metadata: FileMetadata) {
        self.metadata = Some(metadata.clone());
        self.saved_version_id = self.version_id;
        if self.is_dirty {
            self.is_dirty = false;
            self.on_did_change_dirty.fire(false);
        }
        self.on_did_save.fire(metadata.path);
    }

    /// Check if file on disk has diverged from our saved metadata.
    pub fn check_conflict(
        &self,
        fs: &FileSystemService,
    ) -> Result<Option<FileConflict>, FileSystemError> {
        if let (Some(path), Some(meta)) = (&self.path, &self.metadata) {
            fs.check_conflict(path, meta)
        } else {
            Ok(None)
        }
    }

    /// Save document to disk synchronously with atomic write.
    pub fn save_sync(&mut self, fs: &FileSystemService) -> Result<FileMetadata, FileSystemError> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "No file path set"))?;
        let content = self.text();
        let meta = fs.write_atomic_sync(path, &content)?;
        self.mark_saved(meta.clone());
        Ok(meta)
    }

    /// Save document to disk asynchronously with atomic write.
    pub async fn save_async(
        &mut self,
        fs: &FileSystemService,
    ) -> Result<FileMetadata, FileSystemError> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "No file path set"))?;
        let content = self.text();
        let meta = fs.write_atomic(path, &content).await?;
        self.mark_saved(meta.clone());
        Ok(meta)
    }

    /// Revert document content to disk version synchronously.
    pub fn revert_sync(&mut self, _fs: &FileSystemService) -> Result<(), FileSystemError> {

        let path = self
            .path
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "No file path set"))?;
        let content = FileSystemService::read_to_string_sync(path)?;
        let meta = FileSystemService::compute_metadata(path, &content)?;

        *self.buffer.write() = PieceTable::from_str(&content);
        self.metadata = Some(meta);
        self.saved_version_id = self.version_id;
        if self.is_dirty {
            self.is_dirty = false;
            self.on_did_change_dirty.fire(false);
        }
        Ok(())
    }
}

fn map_ext_to_lang_id(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "rs" => "rust".to_string(),
        "js" | "jsx" | "mjs" | "cjs" => "javascript".to_string(),
        "ts" | "tsx" | "mts" | "cts" => "typescript".to_string(),
        "py" | "pyw" | "pyi" => "python".to_string(),
        "json" | "jsonc" => "json".to_string(),
        "md" | "markdown" => "markdown".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]

mod tests {
    use super::*;

    #[test]
    fn test_document_dirty_tracking_and_atomic_save() {
        let tmp_dir = std::env::temp_dir().join(format!("rustcode_doc_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("test_doc.rs");

        let fs = FileSystemService::new();
        let mut doc = TextDocument::new("fn original() {}", Some(path.clone()), Some("rust"));
        assert!(!doc.is_dirty);

        // Edit
        doc.buffer.write().insert(16, "\n// added");
        doc.mark_edited();
        assert!(doc.is_dirty);

        // Save
        let meta = doc.save_sync(&fs).unwrap();
        assert!(!doc.is_dirty);
        assert_eq!(meta.path, path);
        assert_eq!(doc.text(), "fn original() {}\n// added");

        // External conflict test
        std::fs::write(&path, "external change").unwrap();
        let conflict = doc.check_conflict(&fs).unwrap();
        assert!(matches!(conflict, Some(FileConflict::ModifiedOnDisk { .. })));

        // Revert
        doc.revert_sync(&fs).unwrap();
        assert_eq!(doc.text(), "external change");
        assert!(!doc.is_dirty);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
