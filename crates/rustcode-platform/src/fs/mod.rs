//! File system abstraction, atomic file persistence, conflict detection, and file watching.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rustcode_base::disposable::Disposable;
use rustcode_base::event::Emitter;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;

use thiserror::Error;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub mtime: SystemTime,
    pub size: u64,
    pub hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileConflict {
    ModifiedOnDisk {
        expected_mtime: SystemTime,
        disk_mtime: SystemTime,
        expected_hash: u64,
        disk_hash: u64,
    },
    DeletedOnDisk,
}

#[derive(Debug, Error)]
pub enum FileSystemError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("File not found: {0}")]
    NotFound(PathBuf),

    #[error("Disk conflict: {0:?}")]
    Conflict(FileConflict),

    #[error("Watcher error: {0}")]
    WatcherError(String),
}

/// Compute a 64-bit content hash.
pub fn hash_content(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// File system service offering async and sync file operations,
/// atomic disk writes (temp file + atomic rename), and conflict checks.
#[derive(Clone)]
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

    /// Read file content asynchronously as a UTF-8 String.
    pub async fn read_to_string<P: AsRef<Path>>(path: P) -> Result<String, FileSystemError> {
        let p = path.as_ref();
        if !p.exists() {
            return Err(FileSystemError::NotFound(p.to_path_buf()));
        }
        let content = tokio::fs::read_to_string(p).await?;
        Ok(content)
    }

    /// Read file content synchronously as a UTF-8 String.
    pub fn read_to_string_sync<P: AsRef<Path>>(path: P) -> Result<String, FileSystemError> {
        let p = path.as_ref();
        if !p.exists() {
            return Err(FileSystemError::NotFound(p.to_path_buf()));
        }
        let content = std::fs::read_to_string(p)?;
        Ok(content)
    }

    /// Save content atomically using async `tokio::fs`.
    /// Writes to a temporary file in the same directory, flushes to disk,
    /// and atomically renames over the destination path.
    pub async fn write_atomic<P: AsRef<Path>>(
        &self,
        path: P,
        content: &str,
    ) -> Result<FileMetadata, FileSystemError> {
        let target = path.as_ref();
        let parent = match target.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        if parent != Path::new(".") {
            tokio::fs::create_dir_all(parent).await?;
        }

        let file_stem = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document");
        let temp_file_name = format!(".{}.tmp.{}", file_stem, uuid::Uuid::new_v4().simple());
        let temp_path = parent.join(temp_file_name);

        // Write and sync to temporary file
        let write_result: Result<(), std::io::Error> = async {
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .await?;
            file.write_all(content.as_bytes()).await?;
            file.flush().await?;
            file.sync_all().await?;
            Ok(())
        }
        .await;

        if let Err(e) = write_result {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(FileSystemError::Io(e));
        }

        // Atomic rename
        if let Err(e) = tokio::fs::rename(&temp_path, target).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(FileSystemError::Io(e));
        }

        let metadata = Self::compute_metadata(target, content)?;
        self.on_did_change.fire(FileChangeEvent {
            path: target.to_path_buf(),
            change_type: FileChangeType::Changed,
        });

        Ok(metadata)
    }

    /// Save content atomically using synchronous `std::fs`.
    pub fn write_atomic_sync<P: AsRef<Path>>(
        &self,
        path: P,
        content: &str,
    ) -> Result<FileMetadata, FileSystemError> {
        use std::io::Write;

        let target = path.as_ref();
        let parent = match target.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        if parent != Path::new(".") {
            std::fs::create_dir_all(parent)?;
        }

        let file_stem = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document");
        let temp_file_name = format!(".{}.tmp.{}", file_stem, uuid::Uuid::new_v4().simple());
        let temp_path = parent.join(temp_file_name);

        let write_res = (|| -> Result<(), std::io::Error> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
            file.sync_all()?;
            Ok(())
        })();

        if let Err(e) = write_res {
            let _ = std::fs::remove_file(&temp_path);
            return Err(FileSystemError::Io(e));
        }

        if let Err(e) = std::fs::rename(&temp_path, target) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(FileSystemError::Io(e));
        }

        let metadata = Self::compute_metadata(target, content)?;
        self.on_did_change.fire(FileChangeEvent {
            path: target.to_path_buf(),
            change_type: FileChangeType::Changed,
        });

        Ok(metadata)
    }

    /// Compute file metadata (path, mtime, size, content hash).
    pub fn compute_metadata<P: AsRef<Path>>(
        path: P,
        content: &str,
    ) -> Result<FileMetadata, FileSystemError> {
        let target = path.as_ref();
        let meta = std::fs::metadata(target)?;
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let size = meta.len();
        let hash = hash_content(content);

        Ok(FileMetadata {
            path: target.to_path_buf(),
            mtime,
            size,
            hash,
        })
    }

    /// Check if target file on disk differs from the expected metadata.
    pub fn check_conflict<P: AsRef<Path>>(
        &self,
        path: P,
        expected: &FileMetadata,
    ) -> Result<Option<FileConflict>, FileSystemError> {
        let target = path.as_ref();
        if !target.exists() {
            return Ok(Some(FileConflict::DeletedOnDisk));
        }

        let meta = std::fs::metadata(target)?;
        let disk_mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let disk_size = meta.len();

        // If size or mtime differs, read content and check hash
        if disk_mtime != expected.mtime || disk_size != expected.size {
            let disk_content = std::fs::read_to_string(target)?;
            let disk_hash = hash_content(&disk_content);
            if disk_hash != expected.hash {
                return Ok(Some(FileConflict::ModifiedOnDisk {
                    expected_mtime: expected.mtime,
                    disk_mtime,
                    expected_hash: expected.hash,
                    disk_hash,
                }));
            }
        }

        Ok(None)
    }
}

/// Filesystem watcher backed by `notify::RecommendedWatcher`.
/// Emits granular VS Code-style `on_did_create`, `on_did_change`, `on_did_delete` events.
pub struct FileSystemWatcher {
    watcher: Option<RecommendedWatcher>,
    pub on_did_create: Emitter<PathBuf>,
    pub on_did_change: Emitter<PathBuf>,
    pub on_did_delete: Emitter<PathBuf>,
    pub on_any_event: Emitter<FileChangeEvent>,
    _worker_thread: Option<std::thread::JoinHandle<()>>,
}

impl FileSystemWatcher {
    pub fn new() -> Result<Self, FileSystemError> {
        let (tx, rx) = mpsc::channel();

        let watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            notify::Config::default(),
        )
        .map_err(|e| FileSystemError::WatcherError(e.to_string()))?;

        let on_did_create = Emitter::new();
        let on_did_change = Emitter::new();
        let on_did_delete = Emitter::new();
        let on_any_event = Emitter::new();

        let create_emitter = on_did_create.clone();
        let change_emitter = on_did_change.clone();
        let delete_emitter = on_did_delete.clone();
        let any_emitter = on_any_event.clone();

        let worker = std::thread::Builder::new()
            .name("rustcode-fs-watcher".to_string())
            .spawn(move || {
                while let Ok(event_res) = rx.recv() {
                    if let Ok(event) = event_res {
                        for path in event.paths {
                            let change_type = match event.kind {
                                EventKind::Create(_) => {
                                    create_emitter.fire(path.clone());
                                    FileChangeType::Created
                                }
                                EventKind::Modify(_) => {
                                    change_emitter.fire(path.clone());
                                    FileChangeType::Changed
                                }
                                EventKind::Remove(_) => {
                                    delete_emitter.fire(path.clone());
                                    FileChangeType::Deleted
                                }
                                _ => continue,
                            };
                            any_emitter.fire(FileChangeEvent {
                                path,
                                change_type,
                            });
                        }
                    }
                }
            })
            .map_err(|e| FileSystemError::WatcherError(e.to_string()))?;

        Ok(Self {
            watcher: Some(watcher),
            on_did_create,
            on_did_change,
            on_did_delete,
            on_any_event,
            _worker_thread: Some(worker),
        })
    }

    pub fn watch<P: AsRef<Path>>(&mut self, path: P, recursive: bool) -> Result<(), FileSystemError> {
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        if let Some(w) = self.watcher.as_mut() {
            w.watch(path.as_ref(), mode)
                .map_err(|e| FileSystemError::WatcherError(e.to_string()))?;
        }
        Ok(())
    }

    pub fn unwatch<P: AsRef<Path>>(&mut self, path: P) -> Result<(), FileSystemError> {
        if let Some(w) = self.watcher.as_mut() {
            w.unwatch(path.as_ref())
                .map_err(|e| FileSystemError::WatcherError(e.to_string()))?;
        }
        Ok(())
    }
}

impl Disposable for FileSystemWatcher {
    fn dispose(&mut self) {
        self.watcher.take();
        if let Some(handle) = self._worker_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;


    #[test]
    fn test_sync_atomic_write_and_read() {
        let tmp_dir = std::env::temp_dir().join(format!("rustcode_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let file_path = tmp_dir.join("sample.txt");

        let fs = FileSystemService::new();
        let text = "Hello Atomic World!\nSecond Line";

        let meta = fs.write_atomic_sync(&file_path, text).unwrap();
        assert_eq!(meta.path, file_path);
        assert_eq!(meta.size, text.len() as u64);
        assert_eq!(meta.hash, hash_content(text));

        // Read back
        let read = FileSystemService::read_to_string_sync(&file_path).unwrap();
        assert_eq!(read, text);

        // Check conflict (none when clean)
        let conflict = fs.check_conflict(&file_path, &meta).unwrap();
        assert_eq!(conflict, None);

        // External modification causes conflict
        std::fs::write(&file_path, "Modified outside editor!").unwrap();
        let conflict2 = fs.check_conflict(&file_path, &meta).unwrap();
        assert!(matches!(conflict2, Some(FileConflict::ModifiedOnDisk { .. })));

        // Deletion causes conflict
        std::fs::remove_file(&file_path).unwrap();
        let conflict3 = fs.check_conflict(&file_path, &meta).unwrap();
        assert_eq!(conflict3, Some(FileConflict::DeletedOnDisk));

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_async_atomic_write_and_read() {
        let tmp_dir = std::env::temp_dir().join(format!("rustcode_async_test_{}", uuid::Uuid::new_v4().simple()));
        tokio::fs::create_dir_all(&tmp_dir).await.unwrap();
        let file_path = tmp_dir.join("async_sample.rs");

        let fs = FileSystemService::new();
        let text = "fn main() {\n    println!(\"Async rustcode\");\n}\n";

        let meta = fs.write_atomic(&file_path, text).await.unwrap();
        assert_eq!(meta.size, text.len() as u64);

        let read = FileSystemService::read_to_string(&file_path).await.unwrap();
        assert_eq!(read, text);

        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
    }

    #[test]
    fn test_filesystem_watcher_events() {
        let tmp_dir = std::env::temp_dir().join(format!("rustcode_watch_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let test_file = tmp_dir.join("watched.txt");

        let mut watcher = FileSystemWatcher::new().unwrap();
        watcher.watch(&tmp_dir, true).unwrap();

        let event_count = Arc::new(AtomicUsize::new(0));
        let count_clone = event_count.clone();

        let _sub = watcher.on_any_event.event().attach(move |_ev| {
            count_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Trigger filesystem writes
        std::fs::write(&test_file, "init").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));

        std::fs::write(&test_file, "updated content").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));

        std::fs::remove_file(&test_file).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));

        assert!(event_count.load(Ordering::SeqCst) > 0);

        watcher.dispose();
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_atomic_write_relative_paths() {
        let fs = FileSystemService::new();
        let filename = format!(".test_rel_{}.txt", uuid::Uuid::new_v4().simple());
        let content = "Relative file atomic persistence";

        // Sync write
        let meta_sync = fs.write_atomic_sync(&filename, content).unwrap();
        assert_eq!(meta_sync.size, content.len() as u64);
        assert_eq!(FileSystemService::read_to_string_sync(&filename).unwrap(), content);
        let _ = std::fs::remove_file(&filename);

        // Async write
        let meta_async = fs.write_atomic(&filename, content).await.unwrap();
        assert_eq!(meta_async.size, content.len() as u64);
        assert_eq!(FileSystemService::read_to_string(&filename).await.unwrap(), content);
        let _ = tokio::fs::remove_file(&filename).await;
    }
}
