//! VS Code-compatible user profile management (`.code-profile` import/export,
//! per-profile settings, keybindings, extension sets, and global UI state).

use crate::configuration::{ConfigurationService, ConfigurationTarget};
use crate::keybindings::KeybindingService;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_PROFILE_ID: &str = "default";
pub const DEFAULT_PROFILE_NAME: &str = "Default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub use_for_workspaces: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileExtensionEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSnapshot {
    pub profile: UserProfile,
    #[serde(default = "empty_object")]
    pub settings: Value,
    #[serde(default = "empty_array")]
    pub keybindings: Value,
    #[serde(default)]
    pub extensions: Vec<ProfileExtensionEntry>,
    #[serde(default)]
    pub global_state: BTreeMap<String, Value>,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

fn empty_array() -> Value {
    Value::Array(Vec::new())
}

impl ProfileSnapshot {
    pub fn empty_for(profile: UserProfile) -> Self {
        Self {
            profile,
            settings: empty_object(),
            keybindings: empty_array(),
            extensions: Vec::new(),
            global_state: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfilesIndexFile {
    active_profile_id: String,
    profiles: Vec<UserProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeProfileWire {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(default)]
    settings: Option<Value>,
    #[serde(default)]
    keybindings: Option<Value>,
    #[serde(default)]
    extensions: Option<Value>,
    #[serde(default)]
    global_state: Option<Value>,
}

/// Manages named user profiles stored under `AppPaths::profiles_dir()`.
#[derive(Debug, Clone)]
pub struct ProfileManager {
    profiles_dir: PathBuf,
    profiles: BTreeMap<String, UserProfile>,
    snapshots: BTreeMap<String, ProfileSnapshot>,
    active_profile_id: String,
}

impl ProfileManager {
    pub fn new(profiles_dir: impl AsRef<Path>) -> Self {
        let dir = profiles_dir.as_ref().to_path_buf();
        let default_profile = UserProfile {
            id: DEFAULT_PROFILE_ID.to_string(),
            name: DEFAULT_PROFILE_NAME.to_string(),
            icon: None,
            use_for_workspaces: Vec::new(),
        };
        let mut profiles = BTreeMap::new();
        profiles.insert(default_profile.id.clone(), default_profile.clone());
        let mut snapshots = BTreeMap::new();
        snapshots.insert(
            default_profile.id.clone(),
            ProfileSnapshot::empty_for(default_profile),
        );
        Self {
            profiles_dir: dir,
            profiles,
            snapshots,
            active_profile_id: DEFAULT_PROFILE_ID.to_string(),
        }
    }

    pub fn profiles_dir(&self) -> &Path {
        &self.profiles_dir
    }

    pub fn active_profile_id(&self) -> &str {
        &self.active_profile_id
    }

    pub fn active_profile(&self) -> &UserProfile {
        self.profiles
            .get(&self.active_profile_id)
            .or_else(|| self.profiles.get(DEFAULT_PROFILE_ID))
            .expect("default profile always exists")
    }

    pub fn list_profiles(&self) -> Vec<&UserProfile> {
        self.profiles.values().collect()
    }

    pub fn get_profile(&self, id: &str) -> Option<&UserProfile> {
        self.profiles.get(id)
    }

    pub fn snapshot(&self, id: &str) -> Option<&ProfileSnapshot> {
        self.snapshots.get(id)
    }

    pub fn validate_profile_id(id: &str) -> Result<String, String> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err("Profile id cannot be empty".to_string());
        }
        if trimmed == "." || trimmed == ".." || trimmed.contains("..") {
            return Err(format!("Invalid profile id '{}': path traversal not allowed", id));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!(
                "Invalid profile id '{}': only [a-zA-Z0-9_-] allowed",
                id
            ));
        }
        Ok(trimmed.to_string())
    }

    fn slugify_name(&self, name: &str) -> String {
        let mut slug = String::new();
        let mut last_dash = false;
        for ch in name.trim().chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
                last_dash = false;
            } else if (ch == '-' || ch == '_' || ch.is_whitespace()) && !last_dash && !slug.is_empty() {
                slug.push('-');
                last_dash = true;
            }
        }
        let base = slug.trim_matches(['-', '_']).to_string();
        let base = if base.is_empty() {
            "profile".to_string()
        } else {
            base
        };
        if !self.profiles.contains_key(&base) {
            return base;
        }
        let mut n = 2usize;
        loop {
            let candidate = format!("{}-{}", base, n);
            if !self.profiles.contains_key(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    pub fn create_profile(
        &mut self,
        name: &str,
        icon: Option<String>,
        copy_from_id: Option<&str>,
    ) -> Result<UserProfile, String> {
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            return Err("Profile name cannot be empty".to_string());
        }
        let id = self.slugify_name(trimmed_name);
        let profile = UserProfile {
            id: id.clone(),
            name: trimmed_name.to_string(),
            icon,
            use_for_workspaces: Vec::new(),
        };
        let snapshot = if let Some(source_id) = copy_from_id {
            let mut copied = self
                .snapshots
                .get(source_id)
                .ok_or_else(|| format!("Source profile '{}' not found", source_id))?
                .clone();
            copied.profile = profile.clone();
            copied
        } else {
            ProfileSnapshot::empty_for(profile.clone())
        };
        self.profiles.insert(id.clone(), profile.clone());
        self.snapshots.insert(id, snapshot);
        Ok(profile)
    }

    pub fn rename_profile(&mut self, id: &str, new_name: &str) -> Result<UserProfile, String> {
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return Err("Profile name cannot be empty".to_string());
        }
        let profile = self
            .profiles
            .get_mut(id)
            .ok_or_else(|| format!("Profile '{}' not found", id))?;
        profile.name = trimmed.to_string();
        if let Some(snap) = self.snapshots.get_mut(id) {
            snap.profile = profile.clone();
        }
        Ok(profile.clone())
    }

    pub fn bind_workspace_to_profile(
        &mut self,
        id: &str,
        workspace_root: &Path,
    ) -> Result<(), String> {
        let key = workspace_root.to_string_lossy().to_string();
        for profile in self.profiles.values_mut() {
            profile.use_for_workspaces.retain(|w| w != &key);
        }
        let profile = self
            .profiles
            .get_mut(id)
            .ok_or_else(|| format!("Profile '{}' not found", id))?;
        profile.use_for_workspaces.push(key);
        if let Some(snap) = self.snapshots.get_mut(id) {
            snap.profile = profile.clone();
        }
        Ok(())
    }

    pub fn profile_for_workspace(&self, workspace_root: &Path) -> Option<&UserProfile> {
        let needle = workspace_root.to_string_lossy();
        self.profiles
            .values()
            .find(|p| p.use_for_workspaces.iter().any(|w| w == needle.as_ref()))
    }

    pub fn delete_profile(&mut self, id: &str) -> Result<(), String> {
        Self::validate_profile_id(id)?;
        if id == DEFAULT_PROFILE_ID {
            return Err("Cannot delete the Default profile".to_string());
        }
        if self.profiles.remove(id).is_none() {
            return Err(format!("Profile '{}' not found", id));
        }
        self.snapshots.remove(id);
        if self.active_profile_id == id {
            self.active_profile_id = DEFAULT_PROFILE_ID.to_string();
        }
        let dir = self.profiles_dir.join(id);
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        Ok(())
    }

    pub fn update_profile_extensions(
        &mut self,
        id: &str,
        extensions: Vec<ProfileExtensionEntry>,
    ) -> Result<(), String> {
        let snap = self
            .snapshots
            .get_mut(id)
            .ok_or_else(|| format!("Profile '{}' not found", id))?;
        snap.extensions = extensions;
        Ok(())
    }

    /// Snapshot live user configuration, keybindings, extension state, and global UI state
    /// into the currently active profile.
    pub fn capture_current_into_active(
        &mut self,
        config: &ConfigurationService,
        keybindings: Option<&KeybindingService>,
        extensions: Vec<ProfileExtensionEntry>,
        global_state: BTreeMap<String, Value>,
    ) {
        let id = self.active_profile_id.clone();
        let profile = self.active_profile().clone();
        let settings = config.user_layer();
        let kb_val = keybindings
            .map(|kb| kb.export_json())
            .unwrap_or_else(empty_array);
        self.snapshots.insert(
            id,
            ProfileSnapshot {
                profile,
                settings,
                keybindings: kb_val,
                extensions,
                global_state,
            },
        );
    }

    /// Switch the active profile and apply its settings & keybindings to live platform services.
    pub fn switch_profile(
        &mut self,
        id: &str,
        config: &ConfigurationService,
        keybindings: Option<&KeybindingService>,
    ) -> Result<ProfileSnapshot, String> {
        Self::validate_profile_id(id)?;
        let snapshot = self
            .snapshots
            .get(id)
            .cloned()
            .ok_or_else(|| format!("Profile '{}' not found", id))?;

        let settings_str =
            serde_json::to_string_pretty(&snapshot.settings).map_err(|e| e.to_string())?;
        config.load_user_settings(&settings_str)?;

        if let Some(kb) = keybindings {
            let kb_str =
                serde_json::to_string_pretty(&snapshot.keybindings).map_err(|e| e.to_string())?;
            kb.replace_from_json(&kb_str)?;
        }

        self.active_profile_id = id.to_string();
        Ok(snapshot)
    }

    /// Export a profile in VS Code `.code-profile` JSON format.
    pub fn export_code_profile(&self, id: &str) -> Result<String, String> {
        Self::validate_profile_id(id)?;
        let snap = self
            .snapshots
            .get(id)
            .ok_or_else(|| format!("Profile '{}' not found", id))?;
        let wire = CodeProfileWire {
            name: snap.profile.name.clone(),
            icon: snap.profile.icon.clone(),
            settings: Some(Value::String(
                serde_json::to_string_pretty(&serde_json::json!({
                    "settings": serde_json::to_string_pretty(&snap.settings).map_err(|e| e.to_string())?
                }))
                .map_err(|e| e.to_string())?,
            )),
            keybindings: Some(Value::String(
                serde_json::to_string_pretty(&serde_json::json!({
                    "keybindings": serde_json::to_string_pretty(&snap.keybindings).map_err(|e| e.to_string())?
                }))
                .map_err(|e| e.to_string())?,
            )),
            extensions: Some(Value::String(
                serde_json::to_string_pretty(&snap.extensions).map_err(|e| e.to_string())?,
            )),
            global_state: Some(
                serde_json::to_value(&snap.global_state).map_err(|e| e.to_string())?,
            ),
        };
        serde_json::to_string_pretty(&wire).map_err(|e| format!("Failed to serialize profile: {}", e))
    }

    /// Import a VS Code `.code-profile` JSON payload (supports both stringified VS Code sections
    /// and plain JSON object/array sections).
    pub fn import_code_profile(
        &mut self,
        json_str: &str,
        activate: bool,
        config: Option<&ConfigurationService>,
        keybindings: Option<&KeybindingService>,
    ) -> Result<UserProfile, String> {
        let stripped = crate::configuration::strip_json_comments(json_str);
        let wire: CodeProfileWire = serde_json::from_str(&stripped)
            .map_err(|e| format!("Invalid .code-profile JSON: {}", e))?;

        let settings = decode_profile_settings(wire.settings)?;
        let kb_val = decode_profile_keybindings(wire.keybindings)?;
        let extensions = decode_profile_extensions(wire.extensions)?;
        let global_state = decode_global_state(wire.global_state)?;

        let profile = self.create_profile(&wire.name, wire.icon, None)?;
        let snap = ProfileSnapshot {
            profile: profile.clone(),
            settings,
            keybindings: kb_val,
            extensions,
            global_state,
        };
        self.snapshots.insert(profile.id.clone(), snap);

        if activate {
            if let Some(cfg) = config {
                let _ = self.switch_profile(&profile.id, cfg, keybindings)?;
            } else {
                self.active_profile_id = profile.id.clone();
            }
        }
        Ok(profile)
    }

    pub fn apply_settings_update_to_active(&mut self, key: &str, value: Value) {
        if let Some(snap) = self.snapshots.get_mut(&self.active_profile_id) {
            let mut obj = snap.settings.as_object().cloned().unwrap_or_default();
            obj.insert(key.to_string(), value);
            snap.settings = Value::Object(obj);
        }
    }

    pub fn load_from_disk(&mut self) -> Result<(), String> {
        let index_path = self.profiles_dir.join("profiles.json");
        if !index_path.exists() {
            return Ok(());
        }
        let raw = std::fs::read_to_string(&index_path).map_err(|e| e.to_string())?;
        let index: ProfilesIndexFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let mut profiles = BTreeMap::new();
        let mut snapshots = BTreeMap::new();

        for profile in index.profiles {
            if Self::validate_profile_id(&profile.id).is_err() {
                continue;
            }
            let dir = self.profiles_dir.join(&profile.id);
            let snap_path = dir.join("profile.json");
            let snap = if snap_path.exists() {
                let raw_snap = std::fs::read_to_string(&snap_path).map_err(|e| e.to_string())?;
                serde_json::from_str::<ProfileSnapshot>(&raw_snap).map_err(|e| e.to_string())?
            } else {
                ProfileSnapshot::empty_for(profile.clone())
            };
            profiles.insert(profile.id.clone(), profile.clone());
            snapshots.insert(profile.id.clone(), snap);
        }

        if !profiles.contains_key(DEFAULT_PROFILE_ID) {
            let def = UserProfile {
                id: DEFAULT_PROFILE_ID.to_string(),
                name: DEFAULT_PROFILE_NAME.to_string(),
                icon: None,
                use_for_workspaces: Vec::new(),
            };
            profiles.insert(def.id.clone(), def.clone());
            snapshots.insert(def.id.clone(), ProfileSnapshot::empty_for(def));
        }

        self.profiles = profiles;
        self.snapshots = snapshots;
        self.active_profile_id = if self.profiles.contains_key(&index.active_profile_id) {
            index.active_profile_id
        } else {
            DEFAULT_PROFILE_ID.to_string()
        };
        Ok(())
    }

    pub fn save_to_disk(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.profiles_dir).map_err(|e| e.to_string())?;
        let index = ProfilesIndexFile {
            active_profile_id: self.active_profile_id.clone(),
            profiles: self.profiles.values().cloned().collect(),
        };
        let index_json = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?;
        std::fs::write(self.profiles_dir.join("profiles.json"), index_json)
            .map_err(|e| e.to_string())?;

        for (id, snap) in &self.snapshots {
            Self::validate_profile_id(id)?;
            let dir = self.profiles_dir.join(id);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let snap_json = serde_json::to_string_pretty(snap).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("profile.json"), snap_json).map_err(|e| e.to_string())?;
            let settings_json =
                serde_json::to_string_pretty(&snap.settings).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("settings.json"), settings_json).map_err(|e| e.to_string())?;
            let kb_json =
                serde_json::to_string_pretty(&snap.keybindings).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("keybindings.json"), kb_json).map_err(|e| e.to_string())?;
            let ext_json =
                serde_json::to_string_pretty(&snap.extensions).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("extensions.json"), ext_json).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Seed or update a user setting directly into a profile snapshot (useful in tests / migration).
    pub fn set_profile_setting(&mut self, id: &str, key: &str, value: Value) -> Result<(), String> {
        Self::validate_profile_id(id)?;
        let snap = self
            .snapshots
            .get_mut(id)
            .ok_or_else(|| format!("Profile '{}' not found", id))?;
        let mut obj = snap.settings.as_object().cloned().unwrap_or_default();
        obj.insert(key.to_string(), value);
        snap.settings = Value::Object(obj);
        Ok(())
    }

    /// Apply a user config key into ConfigurationService::User target.
    pub fn set_active_user_setting(
        &mut self,
        config: &ConfigurationService,
        key: &str,
        value: Value,
    ) -> Result<(), String> {
        config.update(key, value.clone(), ConfigurationTarget::User)?;
        self.apply_settings_update_to_active(key, value);
        Ok(())
    }
}

fn decode_profile_settings(raw: Option<Value>) -> Result<Value, String> {
    let Some(val) = raw else {
        return Ok(empty_object());
    };
    let unwrapped = match val {
        Value::String(s) => serde_json::from_str::<Value>(&crate::configuration::strip_json_comments(&s))
            .map_err(|e| format!("Invalid settings payload in .code-profile: {}", e))?,
        other => other,
    };
    if let Some(inner) = unwrapped.get("settings") {
        match inner {
            Value::String(s) => serde_json::from_str::<Value>(&crate::configuration::strip_json_comments(s))
                .map_err(|e| format!("Invalid nested settings JSON: {}", e)),
            Value::Object(_) => Ok(inner.clone()),
            _ => Ok(empty_object()),
        }
    } else if unwrapped.is_object() {
        Ok(unwrapped)
    } else {
        Ok(empty_object())
    }
}

fn decode_profile_keybindings(raw: Option<Value>) -> Result<Value, String> {
    let Some(val) = raw else {
        return Ok(empty_array());
    };
    let unwrapped = match val {
        Value::String(s) => serde_json::from_str::<Value>(&crate::configuration::strip_json_comments(&s))
            .map_err(|e| format!("Invalid keybindings payload in .code-profile: {}", e))?,
        other => other,
    };
    if let Some(inner) = unwrapped.get("keybindings") {
        match inner {
            Value::String(s) => serde_json::from_str::<Value>(&crate::configuration::strip_json_comments(s))
                .map_err(|e| format!("Invalid nested keybindings JSON: {}", e)),
            Value::Array(_) => Ok(inner.clone()),
            _ => Ok(empty_array()),
        }
    } else if unwrapped.is_array() {
        Ok(unwrapped)
    } else {
        Ok(empty_array())
    }
}

fn decode_profile_extensions(raw: Option<Value>) -> Result<Vec<ProfileExtensionEntry>, String> {
    let Some(val) = raw else {
        return Ok(Vec::new());
    };
    let unwrapped = match val {
        Value::String(s) => serde_json::from_str::<Value>(&s)
            .map_err(|e| format!("Invalid extensions payload in .code-profile: {}", e))?,
        other => other,
    };
    let mut entries = Vec::new();
    if let Some(arr) = unwrapped.as_array() {
        for item in arr {
            if let Ok(entry) = serde_json::from_value::<ProfileExtensionEntry>(item.clone()) {
                entries.push(entry);
            } else if let Some(id) = item
                .get("identifier")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
                .or_else(|| item.get("id").and_then(Value::as_str))
            {
                entries.push(ProfileExtensionEntry {
                    id: id.to_string(),
                    version: item
                        .get("version")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    enabled: item.get("disabled").and_then(Value::as_bool) != Some(true),
                });
            }
        }
    }
    Ok(entries)
}

fn decode_global_state(raw: Option<Value>) -> Result<BTreeMap<String, Value>, String> {
    let Some(val) = raw else {
        return Ok(BTreeMap::new());
    };
    let unwrapped = match val {
        Value::String(s) => serde_json::from_str::<Value>(&s)
            .map_err(|e| format!("Invalid globalState payload in .code-profile: {}", e))?,
        other => other,
    };
    let mut map = BTreeMap::new();
    if let Some(obj) = unwrapped.as_object() {
        for (k, v) in obj {
            map.insert(k.clone(), v.clone());
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_profile_lifecycle_switch_persistence_and_code_profile_roundtrip() {
        let dir = std::env::temp_dir().join(format!("rustcode_profiles_{}", uuid::Uuid::new_v4()));
        let cfg = ConfigurationService::new();
        let kb = KeybindingService::new();

        cfg.update("editor.tabSize", json!(4), ConfigurationTarget::User)
            .unwrap();
        kb.load_keybindings_json(r#"[{"key":"ctrl+k ctrl+d","command":"default.cmd"}]"#)
            .unwrap();

        let mut mgr = ProfileManager::new(&dir);
        mgr.capture_current_into_active(
            &cfg,
            Some(&kb),
            vec![ProfileExtensionEntry {
                id: "esbenp.prettier-vscode".into(),
                version: Some("10.0.0".into()),
                enabled: true,
            }],
            BTreeMap::from([("workbench.sidebar".into(), json!("left"))]),
        );

        let rust_profile = mgr.create_profile("Rust Dev", Some("rust".into()), None).unwrap();
        assert_eq!(rust_profile.id, "rust-dev");
        mgr.set_profile_setting(&rust_profile.id, "editor.tabSize", json!(2))
            .unwrap();
        mgr.update_profile_extensions(
            &rust_profile.id,
            vec![ProfileExtensionEntry {
                id: "rust-lang.rust-analyzer".into(),
                version: Some("0.3.2000".into()),
                enabled: true,
            }],
        )
        .unwrap();

        // Switch to Rust Dev -> updates ConfigurationService & KeybindingService
        let snap = mgr.switch_profile(&rust_profile.id, &cfg, Some(&kb)).unwrap();
        assert_eq!(mgr.active_profile_id(), "rust-dev");
        assert_eq!(cfg.get::<u64>("editor.tabSize"), Some(2));
        assert_eq!(snap.extensions.len(), 1);
        assert!(kb.lookup_keybinding("default.cmd").is_none());

        // Export & import `.code-profile`
        let exported = mgr.export_code_profile("rust-dev").unwrap();
        assert!(exported.contains("Rust Dev"));
        let imported = mgr
            .import_code_profile(&exported, false, Some(&cfg), Some(&kb))
            .unwrap();
        assert_eq!(imported.id, "rust-dev-2");

        // Persist to disk & reload
        mgr.save_to_disk().unwrap();
        let mut reloaded = ProfileManager::new(&dir);
        reloaded.load_from_disk().unwrap();
        assert_eq!(reloaded.active_profile_id(), "rust-dev");
        assert!(reloaded.get_profile("rust-dev-2").is_some());

        // Security checks: default deletion & traversal ids rejected
        assert!(reloaded.delete_profile(DEFAULT_PROFILE_ID).is_err());
        assert!(reloaded.delete_profile("../escape").is_err());
        assert!(ProfileManager::validate_profile_id("bad/id").is_err());

        // Delete imported profile cleanly
        reloaded.delete_profile("rust-dev-2").unwrap();
        assert!(reloaded.get_profile("rust-dev-2").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
