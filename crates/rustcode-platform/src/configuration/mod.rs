//! Settings and configuration management mirroring VS Code's `IConfigurationService`.

use parking_lot::RwLock;
use rustcode_base::event::Emitter;
use serde::de::DeserializeOwned;
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
                "lineNumbers": "on"
            },
            "workbench": {
                "colorTheme": "Default Dark+",
                "iconTheme": "vs-seti"
            },
            "files": {
                "autoSave": "off",
                "exclude": {
                    "**/.git": true,
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
        *self.user.write() = val;
        self.recompute();
        Ok(())
    }

    /// Load workspace settings from JSON or JSONC string.
    pub fn load_workspace_settings(&self, content: &str) -> Result<(), String> {
        let stripped = strip_json_comments(content);
        let val: Value = serde_json::from_str(&stripped).map_err(|e| e.to_string())?;
        *self.workspace.write() = val;
        self.recompute();
        Ok(())
    }

    /// Load folder-specific settings from JSON or JSONC string for a workspace folder.
    pub fn load_folder_settings(&self, folder: &Path, content: &str) -> Result<(), String> {
        let stripped = strip_json_comments(content);
        let val: Value = serde_json::from_str(&stripped).map_err(|e| e.to_string())?;
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
                let mut cur = folder_val;
                let mut found = true;
                for part in path.split('.') {
                    if let Some(next) = cur.get(part) {
                        cur = next;
                    } else {
                        found = false;
                        break;
                    }
                }
                if found && !cur.is_null() {
                    return Some(cur.clone());
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

    /// Recompute merged view: defaults <- user <- workspace.
    fn recompute(&self) {
        let defs = self.defaults.read().clone();
        let user = self.user.read().clone();
        let ws = self.workspace.read().clone();

        let mut merged = defs;
        merge_json_values(&mut merged, &user);
        merge_json_values(&mut merged, &ws);

        *self.merged.write() = merged;
        self.on_did_change.fire(ConfigurationChangeEvent {
            affected_keys: vec!["*".to_string()],
        });
    }
}

/// Recursively merge `src` into `target`.
fn merge_json_values(target: &mut Value, src: &Value) {
    match (target, src) {
        (Value::Object(target_map), Value::Object(src_map)) => {
            for (k, v) in src_map {
                // Also support flattened keys like "editor.fontSize"
                if k.contains('.') {
                    set_nested_value(target_map, k, v.clone());
                } else {
                    merge_json_values(
                        target_map.entry(k.clone()).or_insert(Value::Null),
                        v,
                    );
                }
            }
        }
        (target_val, src_val) => {
            *target_val = src_val.clone();
        }
    }
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
}
