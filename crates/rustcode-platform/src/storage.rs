//! Persistent storage service (`StorageBackend`, `InMemoryStorageBackend`, and
//! `#[cfg(feature = "desktop")] SqliteStorageService` backed by `rusqlite`).

use crate::fs::{paths_equivalent, strip_windows_verbatim_prefix};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

pub const DEFAULT_MAX_RECENT_WORKSPACES: usize = 20;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Invalid storage input: {0}")]
    InvalidInput(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MementoScope {
    Global,
    Workspace(String),
}

#[cfg(feature = "desktop")]
impl MementoScope {
    fn as_parts(&self) -> (&'static str, &str) {
        match self {
            Self::Global => ("global", ""),
            Self::Workspace(key) => ("workspace", key.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PersistedTabState {
    pub uri: String,
    #[serde(default)]
    pub cursor_line: usize,
    #[serde(default)]
    pub cursor_col: usize,
    #[serde(default)]
    pub scroll_top: f32,
    #[serde(default)]
    pub scroll_left: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PersistedWorkspaceState {
    #[serde(default)]
    pub open_files: Vec<String>,
    #[serde(default)]
    pub active_tab_index: Option<usize>,
    #[serde(default)]
    pub tab_states: Vec<PersistedTabState>,
    #[serde(default)]
    pub sidebar_visible: bool,
    #[serde(default)]
    pub sidebar_width: f32,
    #[serde(default)]
    pub panel_visible: bool,
    #[serde(default)]
    pub panel_height: f32,
    #[serde(default)]
    pub active_theme_id: Option<String>,
}

pub trait StorageBackend: Send + Sync {
    fn recent_workspaces(&self) -> Result<Vec<PathBuf>, StorageError>;
    fn add_recent_workspace(&self, path: &Path) -> Result<(), StorageError>;
    fn save_workspace_state(
        &self,
        workspace_key: &str,
        state: &PersistedWorkspaceState,
    ) -> Result<(), StorageError>;
    fn load_workspace_state(
        &self,
        workspace_key: &str,
    ) -> Result<Option<PersistedWorkspaceState>, StorageError>;
    fn memento_get(
        &self,
        scope: MementoScope,
        ext_id: &str,
        key: &str,
    ) -> Result<Option<Value>, StorageError>;
    fn memento_update(
        &self,
        scope: MementoScope,
        ext_id: &str,
        key: &str,
        value: Option<Value>,
    ) -> Result<(), StorageError>;
    fn memento_keys(&self, scope: MementoScope, ext_id: &str) -> Result<Vec<String>, StorageError>;
}

fn normalize_workspace_path(path: &Path) -> Result<PathBuf, StorageError> {
    if path.as_os_str().is_empty() {
        return Err(StorageError::InvalidInput(
            "Workspace path cannot be empty".to_string(),
        ));
    }
    Ok(strip_windows_verbatim_prefix(path.to_path_buf()))
}

fn validate_workspace_key(workspace_key: &str) -> Result<(), StorageError> {
    if workspace_key.trim().is_empty() {
        return Err(StorageError::InvalidInput(
            "Workspace key cannot be empty".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Default)]
struct InMemoryState {
    recent_workspaces: Vec<PathBuf>,
    workspace_states: HashMap<String, PersistedWorkspaceState>,
    memento: HashMap<(MementoScope, String), HashMap<String, Value>>,
}

#[derive(Debug, Clone)]
pub struct InMemoryStorageBackend {
    max_recent_workspaces: usize,
    inner: Arc<RwLock<InMemoryState>>,
}

impl Default for InMemoryStorageBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryStorageBackend {
    pub fn new() -> Self {
        Self::with_max_recent_workspaces(DEFAULT_MAX_RECENT_WORKSPACES)
    }

    pub fn with_max_recent_workspaces(max_recent_workspaces: usize) -> Self {
        Self {
            max_recent_workspaces,
            inner: Arc::new(RwLock::new(InMemoryState::default())),
        }
    }

    pub fn max_recent_workspaces(&self) -> usize {
        self.max_recent_workspaces
    }
}

impl StorageBackend for InMemoryStorageBackend {
    fn recent_workspaces(&self) -> Result<Vec<PathBuf>, StorageError> {
        Ok(self.inner.read().recent_workspaces.clone())
    }

    fn add_recent_workspace(&self, path: &Path) -> Result<(), StorageError> {
        let normalized = normalize_workspace_path(path)?;
        let mut guard = self.inner.write();
        guard
            .recent_workspaces
            .retain(|existing| !paths_equivalent(existing, &normalized));
        if self.max_recent_workspaces > 0 {
            guard.recent_workspaces.insert(0, normalized);
            guard
                .recent_workspaces
                .truncate(self.max_recent_workspaces);
        }
        Ok(())
    }

    fn save_workspace_state(
        &self,
        workspace_key: &str,
        state: &PersistedWorkspaceState,
    ) -> Result<(), StorageError> {
        validate_workspace_key(workspace_key)?;
        self.inner
            .write()
            .workspace_states
            .insert(workspace_key.to_string(), state.clone());
        Ok(())
    }

    fn load_workspace_state(
        &self,
        workspace_key: &str,
    ) -> Result<Option<PersistedWorkspaceState>, StorageError> {
        validate_workspace_key(workspace_key)?;
        Ok(self
            .inner
            .read()
            .workspace_states
            .get(workspace_key)
            .cloned())
    }

    fn memento_get(
        &self,
        scope: MementoScope,
        ext_id: &str,
        key: &str,
    ) -> Result<Option<Value>, StorageError> {
        let guard = self.inner.read();
        Ok(guard
            .memento
            .get(&(scope, ext_id.to_string()))
            .and_then(|map| map.get(key))
            .cloned())
    }

    fn memento_update(
        &self,
        scope: MementoScope,
        ext_id: &str,
        key: &str,
        value: Option<Value>,
    ) -> Result<(), StorageError> {
        let mut guard = self.inner.write();
        let entry = guard
            .memento
            .entry((scope, ext_id.to_string()))
            .or_default();
        match value {
            Some(v) => {
                entry.insert(key.to_string(), v);
            }
            None => {
                entry.remove(key);
            }
        }
        Ok(())
    }

    fn memento_keys(&self, scope: MementoScope, ext_id: &str) -> Result<Vec<String>, StorageError> {
        let guard = self.inner.read();
        let mut keys: Vec<String> = guard
            .memento
            .get(&(scope, ext_id.to_string()))
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        Ok(keys)
    }
}

#[cfg(feature = "desktop")]
mod sqlite_impl {
    use super::*;
    use parking_lot::Mutex;
    use rusqlite::{params, Connection, OptionalExtension};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub struct SqliteStorageService {
        conn: Mutex<Connection>,
        db_path: Option<PathBuf>,
        max_recent_workspaces: usize,
    }

    impl std::fmt::Debug for SqliteStorageService {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SqliteStorageService")
                .field("db_path", &self.db_path)
                .field("max_recent_workspaces", &self.max_recent_workspaces)
                .finish()
        }
    }

    fn map_sqlite_err(err: rusqlite::Error) -> StorageError {
        StorageError::Database(err.to_string())
    }

    fn current_unix_millis() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    impl SqliteStorageService {
        pub fn open(db_path: impl AsRef<Path>) -> Result<Self, StorageError> {
            Self::open_with_max_recent(db_path, DEFAULT_MAX_RECENT_WORKSPACES)
        }

        pub fn open_with_max_recent(
            db_path: impl AsRef<Path>,
            max_recent_workspaces: usize,
        ) -> Result<Self, StorageError> {
            let path = strip_windows_verbatim_prefix(db_path.as_ref().to_path_buf());
            if path.as_os_str().is_empty() {
                return Err(StorageError::InvalidInput(
                    "SQLite database path cannot be empty".to_string(),
                ));
            }
            if path.is_dir() {
                return Err(StorageError::InvalidInput(format!(
                    "SQLite database path is a directory: {}",
                    path.display()
                )));
            }
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }

            let conn = Connection::open(&path).map_err(map_sqlite_err)?;
            let service = Self {
                conn: Mutex::new(conn),
                db_path: Some(path),
                max_recent_workspaces,
            };
            service.init_schema()?;
            Ok(service)
        }

        pub fn open_in_memory() -> Result<Self, StorageError> {
            Self::open_in_memory_with_max_recent(DEFAULT_MAX_RECENT_WORKSPACES)
        }

        pub fn open_in_memory_with_max_recent(
            max_recent_workspaces: usize,
        ) -> Result<Self, StorageError> {
            let conn = Connection::open_in_memory().map_err(map_sqlite_err)?;
            let service = Self {
                conn: Mutex::new(conn),
                db_path: None,
                max_recent_workspaces,
            };
            service.init_schema()?;
            Ok(service)
        }

        pub fn db_path(&self) -> Option<&Path> {
            self.db_path.as_deref()
        }

        pub fn max_recent_workspaces(&self) -> usize {
            self.max_recent_workspaces
        }

        fn init_schema(&self) -> Result<(), StorageError> {
            let conn = self.conn.lock();
            conn.execute_batch(
                "
                PRAGMA foreign_keys = ON;
                CREATE TABLE IF NOT EXISTS recent_workspaces (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    path TEXT NOT NULL UNIQUE,
                    sort_order INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_recent_workspaces_order
                    ON recent_workspaces(sort_order DESC, id DESC);
                CREATE TABLE IF NOT EXISTS workspace_states (
                    workspace_key TEXT PRIMARY KEY,
                    state_json TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS memento_kv (
                    scope_kind TEXT NOT NULL,
                    scope_key TEXT NOT NULL,
                    ext_id TEXT NOT NULL,
                    key TEXT NOT NULL,
                    value_json TEXT NOT NULL,
                    PRIMARY KEY (scope_kind, scope_key, ext_id, key)
                );
                ",
            )
            .map_err(map_sqlite_err)?;
            Ok(())
        }
    }

    impl StorageBackend for SqliteStorageService {
        fn recent_workspaces(&self) -> Result<Vec<PathBuf>, StorageError> {
            let conn = self.conn.lock();
            let mut stmt = conn
                .prepare("SELECT path FROM recent_workspaces ORDER BY sort_order DESC, id DESC")
                .map_err(map_sqlite_err)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(map_sqlite_err)?;
            let mut out = Vec::new();
            for row in rows {
                let path_str = row.map_err(map_sqlite_err)?;
                out.push(PathBuf::from(path_str));
            }
            Ok(out)
        }

        fn add_recent_workspace(&self, path: &Path) -> Result<(), StorageError> {
            let normalized = normalize_workspace_path(path)?;
            let path_str = normalized.to_string_lossy().into_owned();

            let mut conn = self.conn.lock();
            let tx = conn.transaction().map_err(map_sqlite_err)?;

            let mut existing_ids: Vec<(i64, String)> = Vec::new();
            {
                let mut stmt = tx
                    .prepare("SELECT id, path FROM recent_workspaces")
                    .map_err(map_sqlite_err)?;
                let rows = stmt
                    .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
                    .map_err(map_sqlite_err)?;
                for row in rows {
                    existing_ids.push(row.map_err(map_sqlite_err)?);
                }
            }

            for (id, existing_path) in existing_ids {
                if paths_equivalent(Path::new(&existing_path), &normalized) {
                    tx.execute("DELETE FROM recent_workspaces WHERE id = ?1", params![id])
                        .map_err(map_sqlite_err)?;
                }
            }

            if self.max_recent_workspaces > 0 {
                let next_order: i64 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM recent_workspaces",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(map_sqlite_err)?;

                tx.execute(
                    "INSERT INTO recent_workspaces (path, sort_order) VALUES (?1, ?2)",
                    params![path_str, next_order],
                )
                .map_err(map_sqlite_err)?;

                let limit = self.max_recent_workspaces as i64;
                tx.execute(
                    "DELETE FROM recent_workspaces
                     WHERE id NOT IN (
                         SELECT id FROM recent_workspaces
                         ORDER BY sort_order DESC, id DESC
                         LIMIT ?1
                     )",
                    params![limit],
                )
                .map_err(map_sqlite_err)?;
            } else {
                tx.execute("DELETE FROM recent_workspaces", [])
                    .map_err(map_sqlite_err)?;
            }

            tx.commit().map_err(map_sqlite_err)?;
            Ok(())
        }

        fn save_workspace_state(
            &self,
            workspace_key: &str,
            state: &PersistedWorkspaceState,
        ) -> Result<(), StorageError> {
            validate_workspace_key(workspace_key)?;
            let state_json = serde_json::to_string(state)?;
            let updated_at = current_unix_millis();
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO workspace_states (workspace_key, state_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(workspace_key) DO UPDATE SET
                     state_json = excluded.state_json,
                     updated_at = excluded.updated_at",
                params![workspace_key, state_json, updated_at],
            )
            .map_err(map_sqlite_err)?;
            Ok(())
        }

        fn load_workspace_state(
            &self,
            workspace_key: &str,
        ) -> Result<Option<PersistedWorkspaceState>, StorageError> {
            validate_workspace_key(workspace_key)?;
            let conn = self.conn.lock();
            let raw: Option<String> = conn
                .query_row(
                    "SELECT state_json FROM workspace_states WHERE workspace_key = ?1",
                    params![workspace_key],
                    |row| row.get(0),
                )
                .optional()
                .map_err(map_sqlite_err)?;

            match raw {
                Some(json_str) => Ok(Some(serde_json::from_str(&json_str)?)),
                None => Ok(None),
            }
        }

        fn memento_get(
            &self,
            scope: MementoScope,
            ext_id: &str,
            key: &str,
        ) -> Result<Option<Value>, StorageError> {
            let (scope_kind, scope_key) = scope.as_parts();
            let conn = self.conn.lock();
            let raw: Option<String> = conn
                .query_row(
                    "SELECT value_json FROM memento_kv
                     WHERE scope_kind = ?1 AND scope_key = ?2 AND ext_id = ?3 AND key = ?4",
                    params![scope_kind, scope_key, ext_id, key],
                    |row| row.get(0),
                )
                .optional()
                .map_err(map_sqlite_err)?;

            match raw {
                Some(json_str) => Ok(Some(serde_json::from_str(&json_str)?)),
                None => Ok(None),
            }
        }

        fn memento_update(
            &self,
            scope: MementoScope,
            ext_id: &str,
            key: &str,
            value: Option<Value>,
        ) -> Result<(), StorageError> {
            let (scope_kind, scope_key) = scope.as_parts();
            let conn = self.conn.lock();
            match value {
                Some(v) => {
                    let value_json = serde_json::to_string(&v)?;
                    conn.execute(
                        "INSERT INTO memento_kv (scope_kind, scope_key, ext_id, key, value_json)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(scope_kind, scope_key, ext_id, key) DO UPDATE SET
                             value_json = excluded.value_json",
                        params![scope_kind, scope_key, ext_id, key, value_json],
                    )
                    .map_err(map_sqlite_err)?;
                }
                None => {
                    conn.execute(
                        "DELETE FROM memento_kv
                         WHERE scope_kind = ?1 AND scope_key = ?2 AND ext_id = ?3 AND key = ?4",
                        params![scope_kind, scope_key, ext_id, key],
                    )
                    .map_err(map_sqlite_err)?;
                }
            }
            Ok(())
        }

        fn memento_keys(
            &self,
            scope: MementoScope,
            ext_id: &str,
        ) -> Result<Vec<String>, StorageError> {
            let (scope_kind, scope_key) = scope.as_parts();
            let conn = self.conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT key FROM memento_kv
                     WHERE scope_kind = ?1 AND scope_key = ?2 AND ext_id = ?3
                     ORDER BY key ASC",
                )
                .map_err(map_sqlite_err)?;
            let rows = stmt
                .query_map(params![scope_kind, scope_key, ext_id], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(map_sqlite_err)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(map_sqlite_err)?);
            }
            Ok(out)
        }
    }
}

#[cfg(feature = "desktop")]
pub use sqlite_impl::SqliteStorageService;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(feature = "desktop")]
    struct TempTestDir {
        path: PathBuf,
    }

    #[cfg(feature = "desktop")]
    impl TempTestDir {
        fn new(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "rustcode_storage_{}_{}",
                prefix,
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    #[cfg(feature = "desktop")]
    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn exercise_backend_contract(backend: &dyn StorageBackend, max_recent: usize) {
        // 1. Recent workspaces: deduplicate, move to front, enforce limit
        assert!(backend.recent_workspaces().unwrap().is_empty());
        for i in 0..(max_recent + 2) {
            let p = PathBuf::from(format!("/ws/project_{}", i));
            backend.add_recent_workspace(&p).unwrap();
        }
        let recents = backend.recent_workspaces().unwrap();
        assert_eq!(recents.len(), max_recent);
        assert_eq!(
            recents[0],
            PathBuf::from(format!("/ws/project_{}", max_recent + 1))
        );

        // Re-adding an older workspace moves it to front without duplicating
        let target = PathBuf::from(format!("/ws/project_{}/", max_recent));
        backend.add_recent_workspace(&target).unwrap();
        let recents_after = backend.recent_workspaces().unwrap();
        assert_eq!(recents_after.len(), max_recent);
        assert!(paths_equivalent(
            &recents_after[0],
            &PathBuf::from(format!("/ws/project_{}", max_recent))
        ));

        // 2. Workspace state save & load
        assert_eq!(backend.load_workspace_state("ws-a").unwrap(), None);
        let state = PersistedWorkspaceState {
            open_files: vec![
                "file:///ws/a/src/main.rs".to_string(),
                "file:///ws/a/Cargo.toml".to_string(),
            ],
            active_tab_index: Some(1),
            tab_states: vec![
                PersistedTabState {
                    uri: "file:///ws/a/src/main.rs".to_string(),
                    cursor_line: 42,
                    cursor_col: 9,
                    scroll_top: 120.5,
                    scroll_left: 4.0,
                },
                PersistedTabState {
                    uri: "file:///ws/a/Cargo.toml".to_string(),
                    cursor_line: 3,
                    cursor_col: 0,
                    scroll_top: 0.0,
                    scroll_left: 0.0,
                },
            ],
            sidebar_visible: true,
            sidebar_width: 280.0,
            panel_visible: true,
            panel_height: 210.0,
            active_theme_id: Some("Dark+ (default dark)".to_string()),
        };
        backend.save_workspace_state("ws-a", &state).unwrap();
        assert_eq!(
            backend.load_workspace_state("ws-a").unwrap().as_ref(),
            Some(&state)
        );

        // 3. Memento global & workspace scopes
        let ext = "publisher.sample-ext";
        let ws_scope = MementoScope::Workspace("ws-a".to_string());
        backend
            .memento_update(MementoScope::Global, ext, "nullKey", Some(Value::Null))
            .unwrap();
        backend
            .memento_update(MementoScope::Global, ext, "flag", Some(json!(true)))
            .unwrap();
        backend
            .memento_update(MementoScope::Global, ext, "count", Some(json!(42)))
            .unwrap();
        backend
            .memento_update(
                MementoScope::Global,
                ext,
                "nested",
                Some(json!({"items": [1, "two", {"three": 3}]})),
            )
            .unwrap();
        backend
            .memento_update(ws_scope.clone(), ext, "wsOnly", Some(json!("hello")))
            .unwrap();

        assert_eq!(
            backend
                .memento_get(MementoScope::Global, ext, "nullKey")
                .unwrap(),
            Some(Value::Null)
        );
        assert_eq!(
            backend
                .memento_get(MementoScope::Global, ext, "nested")
                .unwrap(),
            Some(json!({"items": [1, "two", {"three": 3}]}))
        );
        assert_eq!(
            backend.memento_get(ws_scope.clone(), ext, "flag").unwrap(),
            None
        );
        assert_eq!(
            backend.memento_get(ws_scope.clone(), ext, "wsOnly").unwrap(),
            Some(json!("hello"))
        );

        let mut global_keys = backend.memento_keys(MementoScope::Global, ext).unwrap();
        global_keys.sort();
        assert_eq!(global_keys, vec!["count", "flag", "nested", "nullKey"]);

        // Delete with None
        backend
            .memento_update(MementoScope::Global, ext, "flag", None)
            .unwrap();
        assert_eq!(
            backend
                .memento_get(MementoScope::Global, ext, "flag")
                .unwrap(),
            None
        );
        let mut keys_after_del = backend.memento_keys(MementoScope::Global, ext).unwrap();
        keys_after_del.sort();
        assert_eq!(keys_after_del, vec!["count", "nested", "nullKey"]);
    }

    #[test]
    fn test_in_memory_storage_backend() {
        let backend = InMemoryStorageBackend::with_max_recent_workspaces(5);
        exercise_backend_contract(&backend, 5);
        assert!(backend.add_recent_workspace(Path::new("")).is_err());
        assert!(backend.load_workspace_state("   ").is_err());
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn test_sqlite_storage_service_roundtrip_and_reopen_persistence() {
        let tmp = TempTestDir::new("reopen");
        let db_path = tmp.path.join("nested").join("state.db");

        let state = PersistedWorkspaceState {
            open_files: vec!["file:///proj/lib.rs".to_string()],
            active_tab_index: Some(0),
            tab_states: vec![PersistedTabState {
                uri: "file:///proj/lib.rs".to_string(),
                cursor_line: 15,
                cursor_col: 7,
                scroll_top: 64.0,
                scroll_left: 0.0,
            }],
            sidebar_visible: false,
            sidebar_width: 240.0,
            panel_visible: true,
            panel_height: 180.0,
            active_theme_id: Some("Light+ (default light)".to_string()),
        };

        {
            let svc = SqliteStorageService::open_with_max_recent(&db_path, 4).unwrap();
            exercise_backend_contract(&svc, 4);
            svc.save_workspace_state("reopen-ws", &state).unwrap();
            svc.memento_update(
                MementoScope::Global,
                "ext.reopen",
                "persisted",
                Some(json!({"ok": [1, 2, null, true]})),
            )
            .unwrap();
        }

        // Reopen same database and confirm all records survive
        let reopened = SqliteStorageService::open_with_max_recent(&db_path, 4).unwrap();
        assert_eq!(
            reopened.load_workspace_state("reopen-ws").unwrap(),
            Some(state)
        );
        assert_eq!(
            reopened
                .memento_get(MementoScope::Global, "ext.reopen", "persisted")
                .unwrap(),
            Some(json!({"ok": [1, 2, null, true]}))
        );
        assert_eq!(reopened.recent_workspaces().unwrap().len(), 4);
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn test_sqlite_storage_service_corrupt_and_invalid_db_errors() {
        let tmp = TempTestDir::new("corrupt");
        let corrupt_db = tmp.path.join("corrupt.db");
        std::fs::write(&corrupt_db, b"NOT A VALID SQLITE DATABASE HEADER!!!").unwrap();

        let err = SqliteStorageService::open(&corrupt_db).unwrap_err();
        assert!(matches!(err, StorageError::Database(_)));

        // Directory path instead of file returns structured error without panicking
        let dir_err = SqliteStorageService::open(&tmp.path).unwrap_err();
        assert!(matches!(dir_err, StorageError::InvalidInput(_)));

        // Empty path returns structured error
        let empty_err = SqliteStorageService::open(Path::new("")).unwrap_err();
        assert!(matches!(empty_err, StorageError::InvalidInput(_)));
    }
}
