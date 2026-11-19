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

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h1 = (bytes[i + 1] as char).to_digit(16);
            let h2 = (bytes[i + 2] as char).to_digit(16);
            if let (Some(d1), Some(d2)) = (h1, h2) {
                out.push(((d1 << 4) | d2) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Returns `true` if `uri` begins with the `file://` scheme (case-insensitive).
pub fn is_file_uri(uri: &str) -> bool {
    uri.len() >= 7 && uri[..7].eq_ignore_ascii_case("file://")
}

/// Strip Windows extended-length verbatim prefixes (`\\?\C:\...` -> `C:\...`,
/// `\\?\UNC\server\share\...` -> `\\server\share\...`). No-op on non-Windows.
#[cfg(windows)]
pub fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};
    let mut comps = path.components();
    let Some(Component::Prefix(prefix_comp)) = comps.next() else {
        return path;
    };
    let rest: PathBuf = comps.collect();
    match prefix_comp.kind() {
        Prefix::VerbatimDisk(drive) => {
            let mut out =
                std::ffi::OsString::from(format!("{}:", (drive as char).to_ascii_uppercase()));
            out.push(rest.as_os_str());
            PathBuf::from(out)
        }
        Prefix::VerbatimUNC(server, share) => {
            let mut out = std::ffi::OsString::from(r"\\");
            out.push(server);
            out.push(r"\");
            out.push(share);
            out.push(rest.as_os_str());
            PathBuf::from(out)
        }
        _ => path,
    }
}

/// No-op on non-Windows platforms.
#[cfg(not(windows))]
pub fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

#[cfg(windows)]
fn os_str_eq_ignore_ascii_case(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    match (a.to_str(), b.to_str()) {
        (Some(sa), Some(sb)) => sa.eq_ignore_ascii_case(sb),
        _ => a == b,
    }
}

#[cfg(windows)]
fn components_equal_windows(a: &std::path::Component<'_>, b: &std::path::Component<'_>) -> bool {
    use std::path::{Component, Prefix};
    match (a, b) {
        (Component::Prefix(pa), Component::Prefix(pb)) => match (pa.kind(), pb.kind()) {
            (Prefix::Disk(da), Prefix::Disk(db)) => da.eq_ignore_ascii_case(&db),
            (Prefix::UNC(sa, sha), Prefix::UNC(sb, shb)) => {
                os_str_eq_ignore_ascii_case(sa, sb) && os_str_eq_ignore_ascii_case(sha, shb)
            }
            _ => os_str_eq_ignore_ascii_case(pa.as_os_str(), pb.as_os_str()),
        },
        (Component::RootDir, Component::RootDir)
        | (Component::CurDir, Component::CurDir)
        | (Component::ParentDir, Component::ParentDir) => true,
        (Component::Normal(na), Component::Normal(nb)) => os_str_eq_ignore_ascii_case(na, nb),
        _ => false,
    }
}

/// Strip `base` prefix from `path`, matching components case-insensitively and ignoring
/// `\\?\` verbatim prefixes on Windows, while performing exact prefix stripping on Unix.
pub fn path_strip_prefix(path: &Path, base: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let norm_path = strip_windows_verbatim_prefix(path.to_path_buf());
        let norm_base = strip_windows_verbatim_prefix(base.to_path_buf());
        let p_comps: Vec<_> = norm_path.components().collect();
        let b_comps: Vec<_> = norm_base.components().collect();
        if b_comps.len() > p_comps.len() {
            return None;
        }
        if !b_comps
            .iter()
            .zip(p_comps.iter())
            .all(|(bc, pc)| components_equal_windows(bc, pc))
        {
            return None;
        }
        let mut rel = PathBuf::new();
        for c in &p_comps[b_comps.len()..] {
            rel.push(c.as_os_str());
        }
        Some(rel)
    }
    #[cfg(not(windows))]
    {
        path.strip_prefix(base).ok().map(Path::to_path_buf)
    }
}

/// Check whether `path` starts with `base` (case-insensitive & verbatim-normalized on Windows,
/// exact on Unix).
pub fn path_starts_with(path: &Path, base: &Path) -> bool {
    path_strip_prefix(path, base).is_some()
}

/// Check whether two paths refer to the same location (case-insensitive & verbatim-normalized
/// on Windows, exact component equality on Unix).
pub fn paths_equivalent(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        let norm_a = strip_windows_verbatim_prefix(a.to_path_buf());
        let norm_b = strip_windows_verbatim_prefix(b.to_path_buf());
        let a_comps: Vec<_> = norm_a.components().collect();
        let b_comps: Vec<_> = norm_b.components().collect();
        a_comps.len() == b_comps.len()
            && a_comps
                .iter()
                .zip(b_comps.iter())
                .all(|(ac, bc)| components_equal_windows(ac, bc))
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn percent_encode_uri_path(input: &str) -> String {
    if !input.contains('%') {
        return input.to_string();
    }
    input.replace('%', "%25")
}

#[cfg(windows)]
fn normalize_windows_drive_str(raw: &str) -> PathBuf {
    let converted = raw.replace('/', "\\");
    if converted.len() == 2 {
        PathBuf::from(format!("{}\\", converted))
    } else {
        PathBuf::from(converted)
    }
}

#[cfg(windows)]
fn is_windows_drive_prefix(bytes: &[u8]) -> bool {
    bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || bytes[2] == b'/' || bytes[2] == b'\\')
}

/// Convert a `file://` URI (including RFC 8089 `file:///C:/...`, `file://localhost/...`,
/// Windows UNC `file://server/share/...`, and percent-encoded paths) or a plain filesystem path
/// into a `PathBuf`.
pub fn file_uri_to_path(uri_or_path: &str) -> PathBuf {
    if !is_file_uri(uri_or_path) {
        return PathBuf::from(uri_or_path);
    }
    let after_scheme = &uri_or_path[7..];

    // Check for empty authority (`file:///...`) or explicit `localhost` authority
    // (`file://localhost/...`), taking care NOT to strip hostnames like `localhostname`.
    let local_path_raw = if after_scheme.starts_with('/') {
        Some(after_scheme)
    } else if after_scheme.len() >= 9
        && after_scheme[..9].eq_ignore_ascii_case("localhost")
        && (after_scheme.len() == 9 || after_scheme.as_bytes()[9] == b'/')
    {
        let tail = &after_scheme[9..];
        Some(if tail.is_empty() { "/" } else { tail })
    } else {
        None
    };

    if let Some(raw) = local_path_raw {
        let decoded = percent_decode(raw);
        #[cfg(windows)]
        {
            let bytes = decoded.as_bytes();
            if bytes.len() >= 3 && bytes[0] == b'/' && is_windows_drive_prefix(&bytes[1..]) {
                return normalize_windows_drive_str(&decoded[1..]);
            }
            if decoded.starts_with("//") {
                return PathBuf::from(decoded.replace('/', "\\"));
            }
        }
        return PathBuf::from(decoded);
    }

    // Non-empty authority (e.g. UNC `file://server/share/...`) or legacy `file://C:\...`
    let decoded = percent_decode(after_scheme);
    #[cfg(windows)]
    {
        if is_windows_drive_prefix(decoded.as_bytes()) {
            return normalize_windows_drive_str(&decoded);
        }
        if decoded.contains('/') {
            return PathBuf::from(format!(r"\\{}", decoded.replace('/', "\\")));
        }
    }
    PathBuf::from(decoded)
}

/// Convert a filesystem `Path` into an RFC 8089 `file://` URI string.
pub fn path_to_file_uri(path: &Path) -> String {
    if let Some(s) = path.to_str() {
        if is_file_uri(s) {
            return s.to_string();
        }
    }
    let cleaned = strip_windows_verbatim_prefix(path.to_path_buf());
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let raw = cleaned.to_string_lossy();
        if let Some(Component::Prefix(p)) = cleaned.components().next() {
            match p.kind() {
                Prefix::Disk(_) | Prefix::VerbatimDisk(_) => {
                    let slash = raw.replace('\\', "/");
                    return format!("file:///{}", percent_encode_uri_path(&slash));
                }
                Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _) => {
                    let trimmed = raw.trim_start_matches('\\').replace('\\', "/");
                    return format!("file://{}", percent_encode_uri_path(&trimmed));
                }
                _ => {}
            }
        }
        let slash = raw.replace('\\', "/");
        format!("file://{}", percent_encode_uri_path(&slash))
    }
    #[cfg(not(windows))]
    {
        format!(
            "file://{}",
            percent_encode_uri_path(&cleaned.to_string_lossy())
        )
    }
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

    #[test]
    fn test_file_uri_to_path_cross_platform() {
        assert_eq!(
            file_uri_to_path("file:///workspace/my%20file.rs"),
            PathBuf::from("/workspace/my file.rs")
        );
        assert_eq!(
            file_uri_to_path("file://localhost/tmp/demo.txt"),
            PathBuf::from("/tmp/demo.txt")
        );
        assert_eq!(
            file_uri_to_path("FILE://LOCALHOST/tmp/demo.txt"),
            PathBuf::from("/tmp/demo.txt")
        );
        // Raw non-URI paths preserve literal percent sequences
        assert_eq!(
            file_uri_to_path("relative/100%20literal.txt"),
            PathBuf::from("relative/100%20literal.txt")
        );
        // Round-trip Unix path including literal '%'
        let literal_pct = Path::new("/workspace/100%20literal.rs");
        assert_eq!(path_to_file_uri(literal_pct), "file:///workspace/100%2520literal.rs");
        assert_eq!(file_uri_to_path(&path_to_file_uri(literal_pct)), literal_pct);
        assert_eq!(path_to_file_uri(Path::new("/workspace/main.rs")), "file:///workspace/main.rs");

        // Hostname starting with "localhost" must NOT be stripped as "localhost"
        let lh_path = file_uri_to_path("file://localhostname/share/file.txt");
        assert!(!lh_path.to_string_lossy().starts_with("name/"));

        #[cfg(windows)]
        {
            assert_eq!(
                file_uri_to_path("file:///C:/Users/dev/main.rs"),
                PathBuf::from(r"C:\Users\dev\main.rs")
            );
            assert_eq!(
                file_uri_to_path("file:///C:/Users/dev/main.rs").to_string_lossy(),
                r"C:\Users\dev\main.rs"
            );
            assert_eq!(
                file_uri_to_path("file:///c%3A/Users/dev/my%20app.rs"),
                PathBuf::from(r"c:\Users\dev\my app.rs")
            );
            assert_eq!(
                file_uri_to_path("FILE:///C:/Users/dev/main.rs"),
                PathBuf::from(r"C:\Users\dev\main.rs")
            );
            assert_eq!(
                file_uri_to_path("file://localhost/D:/work/lib.rs"),
                PathBuf::from(r"D:\work\lib.rs")
            );
            assert_eq!(
                file_uri_to_path(r"file://C:\Users\dev\main.rs"),
                PathBuf::from(r"C:\Users\dev\main.rs")
            );
            // Bare drive root URI becomes absolute C:\ (not drive-relative C:)
            let bare_drive = file_uri_to_path("file:///C:");
            assert!(bare_drive.is_absolute(), "expected absolute drive root, got {:?}", bare_drive);
            assert_eq!(bare_drive, PathBuf::from(r"C:\"));

            // UNC file:// URIs (2-slash RFC 8089, 4-slash, and localhostname host)
            assert_eq!(
                file_uri_to_path("file://server/share/dir/my%20file.txt"),
                PathBuf::from(r"\\server\share\dir\my file.txt")
            );
            assert_eq!(
                file_uri_to_path("file:////server/share/dir/file.txt"),
                PathBuf::from(r"\\server\share\dir\file.txt")
            );
            assert_eq!(
                file_uri_to_path("file://localhostname/share/file.txt"),
                PathBuf::from(r"\\localhostname\share\file.txt")
            );

            // path_to_file_uri on Windows drive, verbatim, and UNC paths
            assert_eq!(
                path_to_file_uri(Path::new(r"C:\Users\dev\main.rs")),
                "file:///C:/Users/dev/main.rs"
            );
            assert_eq!(
                path_to_file_uri(Path::new(r"\\?\C:\Users\dev\main.rs")),
                "file:///C:/Users/dev/main.rs"
            );
            assert_eq!(
                path_to_file_uri(Path::new(r"\\server\share\dir\main.rs")),
                "file://server/share/dir/main.rs"
            );

            // Case-insensitive & verbatim-normalized prefix/equality helpers
            assert!(path_starts_with(
                Path::new(r"\\?\C:\Users\Dev\Proj\src\lib.rs"),
                Path::new(r"c:\users\dev\proj")
            ));
            assert_eq!(
                path_strip_prefix(
                    Path::new("c:/users/dev/proj/src/lib.rs"),
                    Path::new(r"C:\Users\Dev\Proj")
                ),
                Some(PathBuf::from(r"src\lib.rs"))
            );
            assert!(paths_equivalent(
                Path::new(r"\\?\C:\Users\Dev\App.rs"),
                Path::new("c:/users/dev/app.rs")
            ));
        }
    }
}
