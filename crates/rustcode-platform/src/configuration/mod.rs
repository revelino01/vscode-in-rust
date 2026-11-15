//! Settings and configuration management mirroring VS Code's `IConfigurationService`.

use parking_lot::RwLock;
use rustcode_base::event::Emitter;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ConfigurationChangeEvent {
    pub affected_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationTarget {
    User,
    Workspace,
}

pub struct ConfigurationService {
    defaults: Arc<RwLock<Value>>,
    user: Arc<RwLock<Value>>,
    workspace: Arc<RwLock<Value>>,
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

    /// Query a setting by dot-separated path (e.g. "editor.fontSize").
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Option<T> {
        let guard = self.merged.read();
        let mut cur = &*guard;

        for part in path.split('.') {
            cur = cur.get(part)?;
        }

        serde_json::from_value(cur.clone()).ok()
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
    for i in 0..parts.len() - 1 {
        let key = parts[i];
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
            for j in (i + 1)..len {
                let nc = chars_vec[j];
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
}
