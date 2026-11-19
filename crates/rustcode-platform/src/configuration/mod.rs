//! Settings and configuration management mirroring VS Code's `IConfigurationService`.

use parking_lot::RwLock;
use rustcode_base::event::Emitter;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ConfigurationChangeEvent {
    pub affected_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationTarget {
    User,
    Workspace,
    WorkspaceFolder(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigurationInspection {
    pub key: String,
    #[serde(rename = "defaultValue", skip_serializing_if = "Option::is_none")]
    pub default_value: Option<Value>,
    #[serde(rename = "globalValue", skip_serializing_if = "Option::is_none")]
    pub global_value: Option<Value>,
    #[serde(rename = "workspaceValue", skip_serializing_if = "Option::is_none")]
    pub workspace_value: Option<Value>,
    #[serde(rename = "workspaceFolderValue", skip_serializing_if = "Option::is_none")]
    pub workspace_folder_value: Option<Value>,
}

#[derive(Clone)]
pub struct ConfigurationService {
    defaults: Arc<RwLock<Value>>,
    user: Arc<RwLock<Value>>,
    workspace: Arc<RwLock<Value>>,
    folders: Arc<RwLock<HashMap<PathBuf, Value>>>,
    merged: Arc<RwLock<Value>>,
    pub on_did_change: Emitter<ConfigurationChangeEvent>,
}

impl Default for ConfigurationService {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigurationService {
    pub fn new() -> Self {
        let defaults = json!({
            "editor": {
                "fontSize": 14,
                "tabSize": 4,
                "insertSpaces": true,
                "wordWrap": "off",
                "wordWrapColumn": 80,
                "minimap": {
                    "enabled": true
                },
                "lineNumbers": "on",
                "formatOnSave": false,
                "formatOnPaste": false,
                "autoSave": "off",
                "cursorStyle": "line",
                "renderWhitespace": "selection"
            },
            "workbench": {
                "colorTheme": "Default Dark+",
                "iconTheme": "vs-seti",
                "sideBar": {
                    "location": "left"
                }
            },
            "files": {
                "autoSave": "off",
                "encoding": "utf8",
                "eol": "auto",
                "exclude": {
                    "**/.git": true,
                    "**/node_modules": true,
                    "**/target": true
                }
            },
            "terminal": {
                "integrated": {
                    "fontSize": 14,
                    "shell": {
                        "linux": "/bin/bash"
                    }
                }
            },
            "git": {
                "enableSmartCommit": false
            },
            "extensions": {
                "autoUpdate": "true"
            },
            "window": {
                "zoomLevel": 0
            },
            "breadcrumbs": {
                "enabled": true
            },
            "search": {
                "exclude": {
                    "**/node_modules": true,
                    "**/target": true
                }
            }
        });

        let s = Self {
            defaults: Arc::new(RwLock::new(defaults.clone())),
            user: Arc::new(RwLock::new(json!({}))),
            workspace: Arc::new(RwLock::new(json!({}))),
            folders: Arc::new(RwLock::new(HashMap::new())),
            merged: Arc::new(RwLock::new(defaults)),
            on_did_change: Emitter::new(),
        };
        s.recompute();
        s
    }

    /// Load user settings from JSON or JSONC string.
    pub fn load_user_settings(&self, content: &str) -> Result<(), String> {
        let stripped = strip_json_comments(content);
        let val: Value = serde_json::from_str(&stripped).map_err(|e| e.to_string())?;
        if !val.is_object() {
            return Err("User settings root must be a JSON object".to_string());
        }
        *self.user.write() = val;
        self.recompute();
        Ok(())
    }

    /// Load workspace settings from JSON or JSONC string.
    pub fn load_workspace_settings(&self, content: &str) -> Result<(), String> {
        let stripped = strip_json_comments(content);
        let val: Value = serde_json::from_str(&stripped).map_err(|e| e.to_string())?;
        if !val.is_object() {
            return Err("Workspace settings root must be a JSON object".to_string());
        }
        *self.workspace.write() = val;
        self.recompute();
        Ok(())
    }

    /// Load folder-specific settings from JSON or JSONC string for a workspace folder.
    pub fn load_folder_settings(&self, folder: &Path, content: &str) -> Result<(), String> {
        let stripped = strip_json_comments(content);
        let val: Value = serde_json::from_str(&stripped).map_err(|e| e.to_string())?;
        if !val.is_object() {
            return Err("Folder settings root must be a JSON object".to_string());
        }
        let mut normalized = json!({});
        merge_json_values(&mut normalized, &val);
        self.folders.write().insert(folder.to_path_buf(), normalized);
        self.on_did_change.fire(ConfigurationChangeEvent {
            affected_keys: vec!["*".to_string()],
        });
        Ok(())
    }

    /// Remove folder-specific settings for a workspace folder.
    pub fn remove_folder_settings(&self, folder: &Path) {
        if self.folders.write().remove(folder).is_some() {
            self.on_did_change.fire(ConfigurationChangeEvent {
                affected_keys: vec!["*".to_string()],
            });
        }
    }

    /// Query a setting by dot-separated path (e.g. "editor.fontSize"), scoped to a resource path.
    /// Follows hierarchical fallback: folder (longest prefix) <- workspace <- user <- defaults.
    pub fn get_for_resource<T: DeserializeOwned>(
        &self,
        path: &str,
        resource: Option<&Path>,
    ) -> Option<T> {
        let val = self.get_value_for_resource(path, resource)?;
        serde_json::from_value(val).ok()
    }

    /// Query raw Value at dot-separated path, scoped to a resource path.
    pub fn get_value_for_resource(&self, path: &str, resource: Option<&Path>) -> Option<Value> {
        if let Some(res) = resource {
            let folders_guard = self.folders.read();
            let mut matching: Vec<(&PathBuf, &Value)> = folders_guard
                .iter()
                .filter(|(folder_path, _)| res.starts_with(folder_path))
                .collect();
            // Sort in descending order of path length (deepest prefix first)
            matching.sort_by_key(|(folder_path, _)| std::cmp::Reverse(folder_path.as_os_str().len()));

            for (_, folder_val) in matching {
                if let Some(val) = lookup_in_layer(folder_val, path) {
                    return Some(val);
                }
            }
        }

        self.get_value(path)
    }

    /// Query a setting by dot-separated path (e.g. "editor.fontSize").
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Option<T> {
        self.get_for_resource(path, None)
    }

    /// Query raw Value at dot-separated path.
    pub fn get_value(&self, path: &str) -> Option<Value> {
        let guard = self.merged.read();
        let mut cur = &*guard;

        for part in path.split('.') {
            cur = cur.get(part)?;
        }

        Some(cur.clone())
    }

    /// Return the complete merged configuration tree (defaults <- user <- workspace).
    pub fn get_all(&self) -> Value {
        self.merged.read().clone()
    }

    /// Return the merged configuration tree with any matching workspace folder overrides applied.
    pub fn get_merged_for_resource(&self, resource: Option<&Path>) -> Value {
        let mut merged = self.merged.read().clone();
        if let Some(res) = resource {
            let folders_guard = self.folders.read();
            let mut matching: Vec<(&PathBuf, &Value)> = folders_guard
                .iter()
                .filter(|(folder_path, _)| res.starts_with(folder_path))
                .collect();
            // Sort ascending by folder path length so deeper folders override shallower ones.
            matching.sort_by_key(|(folder_path, _)| folder_path.as_os_str().len());
            for (_, folder_val) in matching {
                merge_json_values(&mut merged, folder_val);
            }
        }
        merged
    }

    /// Inspect a configuration key across defaults, user, workspace, and workspace folder layers.
    pub fn inspect(&self, path: &str, resource: Option<&Path>) -> ConfigurationInspection {
        let default_value = lookup_in_layer(&self.defaults.read(), path);
        let global_value = lookup_in_layer(&self.user.read(), path);
        let workspace_value = lookup_in_layer(&self.workspace.read(), path);
        let workspace_folder_value = resource.and_then(|res| {
            let folders_guard = self.folders.read();
            let mut matching: Vec<(&PathBuf, &Value)> = folders_guard
                .iter()
                .filter(|(folder_path, _)| res.starts_with(folder_path))
                .collect();
            matching.sort_by_key(|(folder_path, _)| std::cmp::Reverse(folder_path.as_os_str().len()));
            for (_, folder_val) in matching {
                if let Some(val) = lookup_in_layer(folder_val, path) {
                    return Some(val);
                }
            }
            None
        });

        ConfigurationInspection {
            key: path.to_string(),
            default_value,
            global_value,
            workspace_value,
            workspace_folder_value,
        }
    }

    /// Update a setting at `path` for the specified `ConfigurationTarget`.
    pub fn update(
        &self,
        path: &str,
        value: Value,
        target: ConfigurationTarget,
    ) -> Result<(), String> {
        if path.is_empty() || path.split('.').any(str::is_empty) {
            return Err(format!("Invalid configuration key path: '{}'", path));
        }

        match target {
            ConfigurationTarget::User => {
                {
                    let mut guard = self.user.write();
                    let map = guard
                        .as_object_mut()
                        .ok_or_else(|| "User configuration is not a JSON object".to_string())?;
                    remove_nested_value(map, path);
                    set_nested_value(map, path, value);
                }
                self.recompute_with_keys(vec![path.to_string()]);
            }
            ConfigurationTarget::Workspace => {
                {
                    let mut guard = self.workspace.write();
                    let map = guard.as_object_mut().ok_or_else(|| {
                        "Workspace configuration is not a JSON object".to_string()
                    })?;
                    remove_nested_value(map, path);
                    set_nested_value(map, path, value);
                }
                self.recompute_with_keys(vec![path.to_string()]);
            }
            ConfigurationTarget::WorkspaceFolder(folder) => {
                {
                    let mut folders_guard = self.folders.write();
                    let entry = folders_guard.entry(folder).or_insert_with(|| json!({}));
                    let map = entry
                        .as_object_mut()
                        .ok_or_else(|| "Folder configuration is not a JSON object".to_string())?;
                    remove_nested_value(map, path);
                    set_nested_value(map, path, value);
                }
                self.on_did_change.fire(ConfigurationChangeEvent {
                    affected_keys: vec![path.to_string()],
                });
            }
        }

        Ok(())
    }

    /// Reset (remove) a setting at `path` from the specified `ConfigurationTarget`.
    pub fn reset(&self, path: &str, target: ConfigurationTarget) {
        if path.is_empty() || path.split('.').any(str::is_empty) {
            return;
        }

        match target {
            ConfigurationTarget::User => {
                let removed = {
                    let mut guard = self.user.write();
                    guard
                        .as_object_mut()
                        .is_some_and(|map| remove_nested_value(map, path))
                };
                if removed {
                    self.recompute_with_keys(vec![path.to_string()]);
                }
            }
            ConfigurationTarget::Workspace => {
                let removed = {
                    let mut guard = self.workspace.write();
                    guard
                        .as_object_mut()
                        .is_some_and(|map| remove_nested_value(map, path))
                };
                if removed {
                    self.recompute_with_keys(vec![path.to_string()]);
                }
            }
            ConfigurationTarget::WorkspaceFolder(folder) => {
                let removed = {
                    let mut folders_guard = self.folders.write();
                    let mut was_removed = false;
                    let mut should_remove_folder = false;
                    if let Some(Value::Object(map)) = folders_guard.get_mut(&folder) {
                        was_removed = remove_nested_value(map, path);
                        should_remove_folder = map.is_empty();
                    }
                    if should_remove_folder {
                        folders_guard.remove(&folder);
                    }
                    was_removed
                };
                if removed {
                    self.on_did_change.fire(ConfigurationChangeEvent {
                        affected_keys: vec![path.to_string()],
                    });
                }
            }
        }
    }

    /// Recompute merged view: defaults <- user <- workspace.
    fn recompute(&self) {
        self.recompute_with_keys(vec!["*".to_string()]);
    }

    fn recompute_with_keys(&self, affected_keys: Vec<String>) {
        let defs = self.defaults.read().clone();
        let user = self.user.read().clone();
        let ws = self.workspace.read().clone();

        let mut merged = defs;
        merge_json_values(&mut merged, &user);
        merge_json_values(&mut merged, &ws);

        *self.merged.write() = merged;
        self.on_did_change
            .fire(ConfigurationChangeEvent { affected_keys });
    }
}

fn lookup_in_layer(root: &Value, path: &str) -> Option<Value> {
    if path.is_empty() {
        return None;
    }
    let parts: Vec<&str> = path.split('.').collect();
    lookup_in_layer_parts(root, &parts)
}

fn lookup_in_layer_parts(cur: &Value, parts: &[&str]) -> Option<Value> {
    if parts.is_empty() {
        return Some(cur.clone());
    }
    for i in (1..=parts.len()).rev() {
        let key = if i == 1 {
            parts[0].to_string()
        } else {
            parts[..i].join(".")
        };
        if let Some(next) = cur.get(&key) {
            if let Some(found) = lookup_in_layer_parts(next, &parts[i..]) {
                return Some(found);
            }
        }
    }
    None
}

fn is_leaf_object_setting(path: &str) -> bool {
    matches!(path, "files.exclude" | "search.exclude")
        || path.ends_with(".files.exclude")
        || path.ends_with(".search.exclude")
}

fn is_dotted_setting_key(k: &str) -> bool {
    k.contains('.') && !k.contains('*') && !k.contains('/') && !k.split('.').any(str::is_empty)
}

/// Recursively merge `src` into `target`.
fn merge_json_values(target: &mut Value, src: &Value) {
    merge_json_values_at(target, src, "");
}

fn merge_json_values_at(target: &mut Value, src: &Value, current_path: &str) {
    if is_leaf_object_setting(current_path) {
        *target = src.clone();
        return;
    }

    match src {
        Value::Object(src_map) => {
            if !target.is_object() {
                *target = Value::Object(serde_json::Map::new());
            }
            let Value::Object(target_map) = target else {
                unreachable!();
            };

            for (k, v) in src_map {
                let full_path = if current_path.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", current_path, k)
                };
                if is_dotted_setting_key(k) {
                    merge_nested_value(target_map, k, v, &full_path);
                } else {
                    let slot = target_map.entry(k.clone()).or_insert(Value::Null);
                    merge_json_values_at(slot, v, &full_path);
                }
            }
        }
        src_val => {
            *target = src_val.clone();
        }
    }
}

fn merge_nested_value(
    target_map: &mut serde_json::Map<String, Value>,
    rel_path: &str,
    value: &Value,
    full_path: &str,
) {
    let parts: Vec<&str> = rel_path.split('.').collect();
    if parts.is_empty() {
        return;
    }

    let mut cur: &mut serde_json::Map<String, Value> = target_map;
    for &key in parts.iter().take(parts.len().saturating_sub(1)) {
        if !cur.contains_key(key) || !cur[key].is_object() {
            cur.insert(key.to_string(), Value::Object(serde_json::Map::new()));
        }
        cur = match cur.get_mut(key) {
            Some(Value::Object(map)) => map,
            _ => unreachable!(),
        };
    }

    if let Some(&last_key) = parts.last() {
        let slot = cur.entry(last_key.to_string()).or_insert(Value::Null);
        merge_json_values_at(slot, value, full_path);
    }
}

fn remove_nested_value(target_map: &mut serde_json::Map<String, Value>, path: &str) -> bool {
    let mut removed = target_map.remove(path).is_some();

    let split_positions: Vec<usize> = path
        .char_indices()
        .filter_map(|(idx, ch)| if ch == '.' { Some(idx) } else { None })
        .collect();

    for idx in split_positions {
        let head = &path[..idx];
        let tail = &path[idx + 1..];
        let mut should_remove_head = false;
        if let Some(Value::Object(sub_map)) = target_map.get_mut(head) {
            if remove_nested_value(sub_map, tail) {
                removed = true;
            }
            if sub_map.is_empty() {
                should_remove_head = true;
            }
        }
        if should_remove_head {
            target_map.remove(head);
        }
    }

    removed
}

fn set_nested_value(target_map: &mut serde_json::Map<String, Value>, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }

    let mut cur: &mut serde_json::Map<String, Value> = target_map;
    for &key in parts.iter().take(parts.len().saturating_sub(1)) {
        if !cur.contains_key(key) || !cur[key].is_object() {
            cur.insert(key.to_string(), Value::Object(serde_json::Map::new()));
        }
        cur = match cur.get_mut(key) {
            Some(Value::Object(map)) => map,
            _ => unreachable!(),
        };
    }

    if let Some(&last_key) = parts.last() {
        cur.insert(last_key.to_string(), value);
    }
}

/// JSONC comment & trailing comma stripper (handles //, /* ... */, and trailing commas).
pub fn strip_json_comments(s: &str) -> String {
    let mut no_comments = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_string = false;
    let mut string_char = '"';
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            no_comments.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == string_char {
                in_string = false;
            }
        } else {
            if c == '"' || c == '\'' {
                in_string = true;
                string_char = c;
                no_comments.push(c);
            } else if c == '/' && chars.peek() == Some(&'/') {
                // Line comment
                chars.next();
                for next_c in chars.by_ref() {
                    if next_c == '\n' {
                        no_comments.push('\n');
                        break;
                    }
                }
            } else if c == '/' && chars.peek() == Some(&'*') {
                // Block comment
                chars.next();
                while let Some(bc) = chars.next() {
                    if bc == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        break;
                    }
                }
            } else {
                no_comments.push(c);
            }
        }
    }

    // Second pass: remove trailing commas before } or ]
    let mut result = String::with_capacity(no_comments.len());
    let chars_vec: Vec<char> = no_comments.chars().collect();
    in_string = false;
    escaped = false;
    let len = chars_vec.len();

    for i in 0..len {
        let c = chars_vec[i];
        if in_string {
            result.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == string_char {
                in_string = false;
            }
        } else if c == '"' || c == '\'' {
            in_string = true;
            string_char = c;
            result.push(c);
        } else if c == ',' {
            let mut is_trailing = false;
            for &nc in chars_vec.iter().take(len).skip(i + 1) {
                if nc.is_whitespace() {
                    continue;
                }
                if nc == '}' || nc == ']' {
                    is_trailing = true;
                }
                break;
            }
            if !is_trailing {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_configuration_service() {
        let config = ConfigurationService::new();

        // Default value
        let font_size: u32 = config.get("editor.fontSize").unwrap();
        assert_eq!(font_size, 14);

        // User settings override with JSON comments
        let user_settings = r#"
        {
            // Custom font size
            "editor.fontSize": 18,
            /* Auto save setting */
            "files": {
                "autoSave": "afterDelay"
            }
        }
        "#;
        config.load_user_settings(user_settings).unwrap();

        let updated_font_size: u32 = config.get("editor.fontSize").unwrap();
        assert_eq!(updated_font_size, 18);

        let auto_save: String = config.get("files.autoSave").unwrap();
        assert_eq!(auto_save, "afterDelay");

        // Workspace settings override user settings with trailing commas
        let ws_settings = r#"
        {
            "editor.fontSize": 20,
        }
        "#;
        config.load_workspace_settings(ws_settings).unwrap();
        let ws_font_size: u32 = config.get("editor.fontSize").unwrap();
        assert_eq!(ws_font_size, 20);
    }

    #[test]
    fn test_per_folder_configuration_hierarchical_fallback() {
        let config = ConfigurationService::new();

        // 1. Defaults: tabSize is 4
        assert_eq!(config.get::<u32>("editor.tabSize"), Some(4));

        // 2. User sets tabSize to 8
        config
            .load_user_settings(r#"{"editor.tabSize": 8}"#)
            .unwrap();
        assert_eq!(config.get::<u32>("editor.tabSize"), Some(8));

        // 3. Workspace sets tabSize to 2
        config
            .load_workspace_settings(r#"{"editor.tabSize": 2}"#)
            .unwrap();
        assert_eq!(config.get::<u32>("editor.tabSize"), Some(2));

        // 4. Folder A has no override -> inherits workspace (2)
        let _folder_a = Path::new("/workspace/folder_a");
        let file_in_a = Path::new("/workspace/folder_a/src/main.rs");
        assert_eq!(
            config.get_for_resource::<u32>("editor.tabSize", Some(file_in_a)),
            Some(2)
        );

        // 5. Folder B overrides tabSize to 6
        let folder_b = Path::new("/workspace/folder_b");
        let file_in_b = Path::new("/workspace/folder_b/src/lib.rs");
        config
            .load_folder_settings(folder_b, r#"{"editor.tabSize": 6}"#)
            .unwrap();

        // Resource in Folder B gets 6
        assert_eq!(
            config.get_for_resource::<u32>("editor.tabSize", Some(file_in_b)),
            Some(6)
        );

        // Resource in Folder A still gets workspace 2
        assert_eq!(
            config.get_for_resource::<u32>("editor.tabSize", Some(file_in_a)),
            Some(2)
        );

        // Nested folder C inside B overrides tabSize to 10
        let folder_c = Path::new("/workspace/folder_b/sub");
        let file_in_c = Path::new("/workspace/folder_b/sub/test.rs");
        config
            .load_folder_settings(folder_c, r#"{"editor.tabSize": 10}"#)
            .unwrap();
        assert_eq!(
            config.get_for_resource::<u32>("editor.tabSize", Some(file_in_c)),
            Some(10)
        );

        // Remove folder C override -> falls back to folder B (6)
        config.remove_folder_settings(folder_c);
        assert_eq!(
            config.get_for_resource::<u32>("editor.tabSize", Some(file_in_c)),
            Some(6)
        );
    }

    #[test]
    fn test_nested_folder_fallback_to_parent_folder() {
        let config = ConfigurationService::new();
        // Workspace sets editor.fontSize = 20, editor.tabSize = 4
        config
            .load_workspace_settings(r#"{"editor.fontSize": 20, "editor.tabSize": 4}"#)
            .unwrap();

        let parent_folder = Path::new("/workspace");
        let child_folder = Path::new("/workspace/sub");
        let file_in_child = Path::new("/workspace/sub/main.rs");

        // Parent folder overrides editor.fontSize = 18 and editor.tabSize = 8
        config
            .load_folder_settings(
                parent_folder,
                r#"{"editor.fontSize": 18, "editor.tabSize": 8}"#,
            )
            .unwrap();

        // Child folder only overrides editor.tabSize = 2, leaving fontSize undefined
        config
            .load_folder_settings(child_folder, r#"{"editor.tabSize": 2}"#)
            .unwrap();

        // 1. Child folder override applies for tabSize (2)
        assert_eq!(
            config.get_for_resource::<u32>("editor.tabSize", Some(file_in_child)),
            Some(2)
        );

        // 2. Child folder lacks fontSize -> falls back to parent folder (18), NOT workspace (20)
        assert_eq!(
            config.get_for_resource::<u32>("editor.fontSize", Some(file_in_child)),
            Some(18)
        );

        // 3. Setting missing in both child and parent folders falls back to workspace/defaults
        config
            .load_workspace_settings(r#"{"editor.wordWrap": "on"}"#)
            .unwrap();
        assert_eq!(
            config.get_for_resource::<String>("editor.wordWrap", Some(file_in_child)),
            Some("on".to_string())
        );
    }

    #[test]
    fn test_seeded_schema_defaults() {
        let config = ConfigurationService::new();

        assert_eq!(config.get::<u32>("editor.fontSize"), Some(14));
        assert_eq!(config.get::<u32>("editor.tabSize"), Some(4));
        assert_eq!(config.get::<bool>("editor.insertSpaces"), Some(true));
        assert_eq!(config.get::<String>("editor.wordWrap"), Some("off".into()));
        assert_eq!(config.get::<u32>("editor.wordWrapColumn"), Some(80));
        assert_eq!(config.get::<bool>("editor.minimap.enabled"), Some(true));
        assert_eq!(config.get::<String>("editor.lineNumbers"), Some("on".into()));
        assert_eq!(config.get::<bool>("editor.formatOnSave"), Some(false));
        assert_eq!(config.get::<bool>("editor.formatOnPaste"), Some(false));
        assert_eq!(config.get::<String>("editor.autoSave"), Some("off".into()));
        assert_eq!(config.get::<String>("editor.cursorStyle"), Some("line".into()));
        assert_eq!(
            config.get::<String>("editor.renderWhitespace"),
            Some("selection".into())
        );

        assert_eq!(
            config.get::<String>("workbench.colorTheme"),
            Some("Default Dark+".into())
        );
        assert_eq!(
            config.get::<String>("workbench.iconTheme"),
            Some("vs-seti".into())
        );
        assert_eq!(
            config.get::<String>("workbench.sideBar.location"),
            Some("left".into())
        );

        assert_eq!(config.get::<String>("files.autoSave"), Some("off".into()));
        assert_eq!(config.get::<String>("files.encoding"), Some("utf8".into()));
        assert_eq!(config.get::<String>("files.eol"), Some("auto".into()));
        assert_eq!(config.get::<bool>("files.exclude.**/.git"), None);
        let files_excl = config.get_value("files.exclude").unwrap();
        assert_eq!(files_excl["**/.git"], json!(true));

        assert_eq!(config.get::<u32>("terminal.integrated.fontSize"), Some(14));
        assert_eq!(
            config.get::<String>("terminal.integrated.shell.linux"),
            Some("/bin/bash".into())
        );
        assert_eq!(config.get::<bool>("git.enableSmartCommit"), Some(false));
        assert_eq!(
            config.get::<String>("extensions.autoUpdate"),
            Some("true".into())
        );
        assert_eq!(config.get::<i32>("window.zoomLevel"), Some(0));
        assert_eq!(config.get::<bool>("breadcrumbs.enabled"), Some(true));

        let search_excl = config.get_value("search.exclude").unwrap();
        assert_eq!(search_excl["**/node_modules"], json!(true));
        assert_eq!(search_excl["**/target"], json!(true));
    }

    #[test]
    fn test_clone_and_update_reset_across_targets() {
        use parking_lot::Mutex;

        let config = ConfigurationService::new();
        let cloned = config.clone();

        let recorded_events: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let rec_clone = Arc::clone(&recorded_events);
        let _sub = config.on_did_change.event().attach(move |ev| {
            rec_clone.lock().push(ev.affected_keys.clone());
        });

        // Invalid paths rejected
        assert!(config
            .update("", json!(1), ConfigurationTarget::User)
            .is_err());
        assert!(config
            .update("editor..fontSize", json!(1), ConfigurationTarget::User)
            .is_err());
        assert!(config
            .update(".editor", json!(1), ConfigurationTarget::User)
            .is_err());

        // Update User via clone -> original sees it
        cloned
            .update("editor.fontSize", json!(18), ConfigurationTarget::User)
            .unwrap();
        assert_eq!(config.get::<u32>("editor.fontSize"), Some(18));

        // Update Workspace overrides User
        config
            .update("editor.fontSize", json!(22), ConfigurationTarget::Workspace)
            .unwrap();
        assert_eq!(cloned.get::<u32>("editor.fontSize"), Some(22));

        // Update WorkspaceFolder overrides Workspace for resource
        let folder = PathBuf::from("/proj/app");
        let file_in_folder = PathBuf::from("/proj/app/src/main.rs");
        config
            .update(
                "editor.fontSize",
                json!(26),
                ConfigurationTarget::WorkspaceFolder(folder.clone()),
            )
            .unwrap();
        assert_eq!(
            cloned.get_for_resource::<u32>("editor.fontSize", Some(&file_in_folder)),
            Some(26)
        );

        // Reset WorkspaceFolder -> falls back to Workspace (22)
        cloned.reset("editor.fontSize", ConfigurationTarget::WorkspaceFolder(folder));
        assert_eq!(
            config.get_for_resource::<u32>("editor.fontSize", Some(&file_in_folder)),
            Some(22)
        );

        // Reset Workspace -> falls back to User (18)
        config.reset("editor.fontSize", ConfigurationTarget::Workspace);
        assert_eq!(cloned.get::<u32>("editor.fontSize"), Some(18));

        // Reset User -> falls back to default (14), NOT Value::Null
        cloned.reset("editor.fontSize", ConfigurationTarget::User);
        assert_eq!(config.get::<u32>("editor.fontSize"), Some(14));
        assert_eq!(config.get_value("editor.fontSize"), Some(json!(14)));

        // Reset non-existent key is a no-op (no extra event)
        let events_before = recorded_events.lock().len();
        config.reset("editor.fontSize", ConfigurationTarget::User);
        assert_eq!(recorded_events.lock().len(), events_before);

        let events = recorded_events.lock();
        assert!(events.iter().all(|keys| keys == &vec!["editor.fontSize".to_string()]));
        assert_eq!(events.len(), 6);
    }

    #[test]
    fn test_update_and_reset_flattened_dotted_keys() {
        let config = ConfigurationService::new();
        config
            .load_user_settings(
                r#"{
                    "editor.fontSize": 24,
                    "editor": {
                        "minimap.enabled": false,
                        "tabSize": 2
                    }
                }"#,
            )
            .unwrap();
        assert_eq!(config.get::<u32>("editor.fontSize"), Some(24));
        assert_eq!(config.get::<bool>("editor.minimap.enabled"), Some(false));
        assert_eq!(config.get::<u32>("editor.tabSize"), Some(2));

        // Updating a setting that had a flattened dotted key replaces it cleanly
        config
            .update("editor.fontSize", json!(16), ConfigurationTarget::User)
            .unwrap();
        assert_eq!(config.get::<u32>("editor.fontSize"), Some(16));

        // Resetting removes both nested and dotted forms, falling back to default
        config.reset("editor.fontSize", ConfigurationTarget::User);
        assert_eq!(config.get::<u32>("editor.fontSize"), Some(14));

        // Partial dotted key reset falls back to default without touching sibling tabSize
        config.reset("editor.minimap.enabled", ConfigurationTarget::User);
        assert_eq!(config.get::<bool>("editor.minimap.enabled"), Some(true));
        assert_eq!(config.get::<u32>("editor.tabSize"), Some(2));
    }

    #[test]
    fn test_exclude_dotfile_keys_flat_nested_consistency_and_replacement() {
        let val = json!({
            "**/*.log": true,
            ".DS_Store": true,
            "Thumbs.db": true,
            "Cargo.lock": false
        });

        // 1. Direct update() preserves dot-containing keys without splitting
        let cfg_update = ConfigurationService::new();
        cfg_update
            .update("files.exclude", val.clone(), ConfigurationTarget::User)
            .unwrap();
        cfg_update
            .update("search.exclude", val.clone(), ConfigurationTarget::User)
            .unwrap();
        // Trigger subsequent recompute on an unrelated key
        cfg_update
            .update("editor.fontSize", json!(18), ConfigurationTarget::User)
            .unwrap();

        assert_eq!(cfg_update.get_value("files.exclude"), Some(val.clone()));
        assert_eq!(cfg_update.get_value("search.exclude"), Some(val.clone()));

        // 2. Flattened dotted keys in load_user_settings match update() 100%
        let cfg_flat = ConfigurationService::new();
        cfg_flat
            .load_user_settings(
                r#"{
                    "files.exclude": {
                        "**/*.log": true,
                        ".DS_Store": true,
                        "Thumbs.db": true,
                        "Cargo.lock": false
                    },
                    "search.exclude": {
                        "**/*.log": true,
                        ".DS_Store": true,
                        "Thumbs.db": true,
                        "Cargo.lock": false
                    },
                    "editor.fontSize": 18
                }"#,
            )
            .unwrap();
        assert_eq!(cfg_flat.get_value("files.exclude"), Some(val.clone()));
        assert_eq!(cfg_flat.get_value("search.exclude"), Some(val.clone()));
        // Unrelated sibling defaults remain intact
        assert_eq!(cfg_flat.get::<String>("files.encoding"), Some("utf8".into()));

        // 3. Nested objects in load_user_settings match flat & update() 100%
        let cfg_nested = ConfigurationService::new();
        cfg_nested
            .load_user_settings(
                r#"{
                    "files": {
                        "exclude": {
                            "**/*.log": true,
                            ".DS_Store": true,
                            "Thumbs.db": true,
                            "Cargo.lock": false
                        }
                    },
                    "search": {
                        "exclude": {
                            "**/*.log": true,
                            ".DS_Store": true,
                            "Thumbs.db": true,
                            "Cargo.lock": false
                        }
                    }
                }"#,
            )
            .unwrap();
        assert_eq!(cfg_nested.get_value("files.exclude"), Some(val.clone()));
        assert_eq!(cfg_nested.get_value("search.exclude"), Some(val.clone()));
        assert_eq!(cfg_nested.get::<String>("files.encoding"), Some("utf8".into()));

        // 4. Workspace folder settings also preserve dotfile keys
        let folder = Path::new("/workspace/proj");
        let file_in_folder = Path::new("/workspace/proj/src/lib.rs");
        cfg_nested
            .load_folder_settings(
                folder,
                r#"{"files.exclude": {".DS_Store": true, "Thumbs.db": true}}"#,
            )
            .unwrap();
        assert_eq!(
            cfg_nested.get_value_for_resource("files.exclude", Some(file_in_folder)),
            Some(json!({ ".DS_Store": true, "Thumbs.db": true }))
        );

        // 5. Resetting user search.exclude restores default exclude map cleanly
        cfg_update.reset("search.exclude", ConfigurationTarget::User);
        assert_eq!(
            cfg_update.get_value("search.exclude"),
            Some(json!({ "**/node_modules": true, "**/target": true }))
        );
    }

    #[test]
    fn test_non_object_root_json_rejected_without_wiping_defaults() {
        let cfg = ConfigurationService::new();
        cfg.update("editor.fontSize", json!(18), ConfigurationTarget::User)
            .unwrap();

        let folder = Path::new("/workspace/app");
        for invalid_root in ["null", "123", "[]", "\"hello\"", "true"] {
            assert!(
                cfg.load_user_settings(invalid_root).is_err(),
                "user settings should reject {}",
                invalid_root
            );
            assert!(
                cfg.load_workspace_settings(invalid_root).is_err(),
                "workspace settings should reject {}",
                invalid_root
            );
            assert!(
                cfg.load_folder_settings(folder, invalid_root).is_err(),
                "folder settings should reject {}",
                invalid_root
            );
        }

        // Defaults and prior user overrides must remain intact and updatable
        assert_eq!(cfg.get::<u32>("editor.fontSize"), Some(18));
        assert_eq!(cfg.get::<bool>("editor.minimap.enabled"), Some(true));
        cfg.update("editor.tabSize", json!(2), ConfigurationTarget::User)
            .unwrap();
        assert_eq!(cfg.get::<u32>("editor.tabSize"), Some(2));
    }

    #[test]
    fn test_get_all_get_merged_for_resource_and_inspect() {
        let cfg = ConfigurationService::new();
        let all = cfg.get_all();
        assert_eq!(all["editor"]["fontSize"], json!(14));
        assert_eq!(all["workbench"]["colorTheme"], json!("Default Dark+"));

        cfg.load_user_settings(r#"{"editor.fontSize": 18}"#).unwrap();
        cfg.update("editor.tabSize", json!(2), ConfigurationTarget::Workspace)
            .unwrap();

        let folder = Path::new("/workspace/pkg");
        let file = Path::new("/workspace/pkg/src/lib.rs");
        cfg.update(
            "editor.tabSize",
            json!(6),
            ConfigurationTarget::WorkspaceFolder(folder.to_path_buf()),
        )
        .unwrap();

        let merged_for_file = cfg.get_merged_for_resource(Some(file));
        assert_eq!(merged_for_file["editor"]["tabSize"], json!(6));
        // Sibling defaults & user settings preserved when merging folder overrides
        assert_eq!(merged_for_file["editor"]["fontSize"], json!(18));
        assert_eq!(merged_for_file["editor"]["minimap"]["enabled"], json!(true));

        let inspected = cfg.inspect("editor.fontSize", Some(file));
        assert_eq!(inspected.key, "editor.fontSize");
        assert_eq!(inspected.default_value, Some(json!(14)));
        assert_eq!(inspected.global_value, Some(json!(18)));
        assert_eq!(inspected.workspace_value, None);
        assert_eq!(inspected.workspace_folder_value, None);

        let inspected_tab = cfg.inspect("editor.tabSize", Some(file));
        assert_eq!(inspected_tab.default_value, Some(json!(4)));
        assert_eq!(inspected_tab.global_value, None);
        assert_eq!(inspected_tab.workspace_value, Some(json!(2)));
        assert_eq!(inspected_tab.workspace_folder_value, Some(json!(6)));
    }

    #[test]
    fn test_inspect_mixed_nested_dotted_and_folder_null_consistency() {
        let cfg = ConfigurationService::new();
        cfg.load_user_settings(
            r#"{
                "myExt": {
                    "a": { "other": 2 },
                    "a.b": 1
                }
            }"#,
        )
        .unwrap();

        assert_eq!(cfg.get::<i64>("myExt.a.b"), Some(1));
        assert_eq!(cfg.get::<i64>("myExt.a.other"), Some(2));

        let inspected = cfg.inspect("myExt.a.b", None);
        assert_eq!(inspected.global_value, Some(json!(1)));
        let inspected_other = cfg.inspect("myExt.a.other", None);
        assert_eq!(inspected_other.global_value, Some(json!(2)));

        // WorkspaceFolder explicit Value::Null is consistent across get_value_for_resource,
        // get_merged_for_resource, and inspect.
        let folder = Path::new("/workspace/nullpkg");
        let file = Path::new("/workspace/nullpkg/src/main.rs");
        cfg.update("editor.tabSize", json!(8), ConfigurationTarget::Workspace)
            .unwrap();
        cfg.update(
            "editor.tabSize",
            Value::Null,
            ConfigurationTarget::WorkspaceFolder(folder.to_path_buf()),
        )
        .unwrap();

        assert_eq!(
            cfg.get_value_for_resource("editor.tabSize", Some(file)),
            Some(Value::Null)
        );
        assert_eq!(
            cfg.get_merged_for_resource(Some(file))["editor"]["tabSize"],
            Value::Null
        );
        assert_eq!(
            cfg.inspect("editor.tabSize", Some(file)).workspace_folder_value,
            Some(Value::Null)
        );
    }
}
