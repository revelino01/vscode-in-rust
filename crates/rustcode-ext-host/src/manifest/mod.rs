//! VS Code extension manifest parser (`package.json`) and loader.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub name: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub version: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub main: Option<String>,
    #[serde(rename = "activationEvents", default)]
    pub activation_events: Vec<String>,
    pub contributes: Option<Contributions>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Contributions {
    pub commands: Option<Vec<ContributedCommand>>,
    pub languages: Option<Vec<LanguageContribution>>,
    pub grammars: Option<Vec<GrammarContribution>>,
    pub themes: Option<Vec<ThemeContribution>>,
    pub keybindings: Option<Vec<KeybindingContribution>>,
    pub configuration: Option<Value>,
    pub views: Option<std::collections::HashMap<String, Vec<ViewContribution>>>,
    #[serde(default)]
    pub debuggers: Option<Vec<DebuggerContribution>>,
    #[serde(rename = "taskDefinitions", default)]
    pub task_definitions: Option<Vec<TaskDefinitionContribution>>,
    #[serde(default)]
    pub authentication: Option<Vec<AuthenticationContribution>>,
    #[serde(rename = "customEditors", default)]
    pub custom_editors: Vec<CustomEditorContribution>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomEditorSelector {
    #[serde(default)]
    pub filename_pattern: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomEditorContribution {
    pub view_type: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub selector: Vec<CustomEditorSelector>,
    #[serde(default)]
    pub priority: Option<String>,
}

impl CustomEditorContribution {
    pub fn matches_path(&self, path: &str) -> bool {
        let trimmed_path = path.trim();
        if trimmed_path.is_empty() {
            return false;
        }
        let normalized_path = trimmed_path.replace('\\', "/");
        let file_name = normalized_path
            .rsplit('/')
            .next()
            .unwrap_or(&normalized_path);

        self.selector.iter().any(|sel| {
            let pat = sel.filename_pattern.trim();
            if pat.is_empty() {
                return false;
            }
            let normalized_pat = pat.replace('\\', "/");
            if let Some(rest) = normalized_pat.strip_prefix("**/") {
                if rest.contains('/') {
                    glob_match(rest, &normalized_path)
                        || normalized_path
                            .match_indices('/')
                            .any(|(idx, _)| glob_match(rest, &normalized_path[idx + 1..]))
                } else {
                    glob_match(rest, file_name)
                }
            } else if normalized_pat.contains('/') {
                glob_match(&normalized_pat, &normalized_path)
            } else {
                glob_match(&normalized_pat, file_name)
            }
        })
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebuggerContribution {
    #[serde(rename = "type")]
    pub debugger_type: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(rename = "configurationAttributes", default)]
    pub configuration_attributes: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDefinitionContribution {
    #[serde(rename = "type")]
    pub task_type: String,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub properties: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationContribution {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewContribution {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrammarContribution {
    pub language: String,
    #[serde(rename = "scopeName")]
    pub scope_name: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributedCommand {
    pub command: String,
    pub title: String,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageContribution {
    pub id: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeContribution {
    pub label: String,
    #[serde(rename = "uiTheme")]
    pub ui_theme: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingContribution {
    pub command: String,
    pub key: String,
    pub when: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoadedExtension {
    pub root_dir: PathBuf,
    pub manifest: ExtensionManifest,
    pub is_activated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSnapshot {
    pub id: String,
    pub extension_path: String,
    pub is_active: bool,
    #[serde(rename = "packageJSON")]
    pub package_json: Value,
    pub extension_kind: u8,
}

impl ExtensionSnapshot {
    pub fn from_loaded(ext: &LoadedExtension) -> Self {
        let id = crate::marketplace::ExtensionInstaller::extension_id(
            &ext.manifest.publisher,
            &ext.manifest.name,
        );
        let package_json = serde_json::to_value(&ext.manifest).unwrap_or_else(|_| {
            serde_json::json!({
                "name": ext.manifest.name,
                "publisher": ext.manifest.publisher,
                "version": ext.manifest.version,
            })
        });
        Self {
            id,
            extension_path: ext.root_dir.to_string_lossy().into_owned(),
            is_active: ext.is_activated,
            package_json,
            extension_kind: 1,
        }
    }
}

impl From<&LoadedExtension> for ExtensionSnapshot {
    fn from(ext: &LoadedExtension) -> Self {
        Self::from_loaded(ext)
    }
}

pub struct ExtensionLoader {
    search_paths: Vec<PathBuf>,
    extensions: Vec<LoadedExtension>,
}

impl Default for ExtensionLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtensionLoader {
    pub fn new() -> Self {
        Self {
            search_paths: Vec::new(),
            extensions: Vec::new(),
        }
    }

    pub fn add_search_path<P: AsRef<Path>>(&mut self, path: P) {
        self.search_paths.push(path.as_ref().to_path_buf());
    }

    /// Parse manifest from JSON string.
    pub fn parse_manifest(json_str: &str) -> Result<ExtensionManifest, String> {
        serde_json::from_str(json_str).map_err(|e| format!("Failed to parse extension manifest: {}", e))
    }

    /// Load extension from directory containing package.json.
    pub fn load_from_dir<P: AsRef<Path>>(&mut self, dir: P) -> Result<&LoadedExtension, String> {
        let manifest_path = dir.as_ref().join("package.json");
        let content = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("Could not read {}: {}", manifest_path.display(), e))?;

        let manifest = Self::parse_manifest(&content)?;
        self.extensions.push(LoadedExtension {
            root_dir: dir.as_ref().to_path_buf(),
            manifest,
            is_activated: false,
        });

        Ok(self.extensions.last().unwrap())
    }

    /// Find extensions that should activate for a given activation event.
    pub fn extensions_for_event(&self, event: &str) -> Vec<&LoadedExtension> {
        self.extensions
            .iter()
            .filter(|ext| {
                ext.manifest.activation_events.contains(&"*".to_string())
                    || ext.manifest.activation_events.contains(&event.to_string())
            })
            .collect()
    }

    pub fn loaded_extensions(&self) -> &[LoadedExtension] {
        &self.extensions
    }

    /// Mark an extension as activated by its manifest name. Returns true if found.
    pub fn activate(&mut self, name: &str) -> bool {
        if let Some(ext) = self.extensions.iter_mut().find(|e| e.manifest.name == name) {
            ext.is_activated = true;
            true
        } else {
            false
        }
    }

    /// Mark an extension as deactivated by its manifest name. Returns true if found.
    pub fn deactivate(&mut self, name: &str) -> bool {
        if let Some(ext) = self.extensions.iter_mut().find(|e| e.manifest.name == name) {
            ext.is_activated = false;
            true
        } else {
            false
        }
    }

    /// Check if an extension is activated.
    pub fn is_activated(&self, name: &str) -> bool {
        self.extensions
            .iter()
            .find(|e| e.manifest.name == name)
            .is_some_and(|e| e.is_activated)
    }

    /// Return serializable snapshots of all loaded extensions.
    pub fn snapshots(&self) -> Vec<ExtensionSnapshot> {
        self.extensions
            .iter()
            .map(ExtensionSnapshot::from_loaded)
            .collect()
    }

    /// Look up a loaded extension snapshot by canonical ID (case-insensitive).
    pub fn snapshot_for(&self, extension_id: &str) -> Option<ExtensionSnapshot> {
        self.extensions
            .iter()
            .map(ExtensionSnapshot::from_loaded)
            .find(|snap| snap.id.eq_ignore_ascii_case(extension_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_manifest() {
        let json = r#"{
            "name": "rustcode-sample",
            "publisher": "rustcode",
            "version": "1.0.0",
            "main": "./out/extension.js",
            "activationEvents": [
                "onLanguage:rust",
                "onCommand:sample.helloWorld"
            ],
            "contributes": {
                "commands": [
                    {
                        "command": "sample.helloWorld",
                        "title": "Hello World",
                        "category": "Sample"
                    }
                ],
                "languages": [
                    {
                        "id": "rust",
                        "extensions": [".rs"],
                        "aliases": ["Rust", "rust"]
                    }
                ]
            }
        }"#;

        let manifest = ExtensionLoader::parse_manifest(json).unwrap();
        assert_eq!(manifest.name, "rustcode-sample");
        assert_eq!(manifest.publisher, "rustcode");
        assert_eq!(manifest.activation_events.len(), 2);

        let contributes = manifest.contributes.unwrap();
        let commands = contributes.commands.unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].command, "sample.helloWorld");

        let languages = contributes.languages.unwrap();
        assert_eq!(languages[0].id, "rust");
        assert_eq!(languages[0].extensions, vec![".rs"]);
        assert!(contributes.debuggers.is_none());
        assert!(contributes.task_definitions.is_none());
        assert!(contributes.authentication.is_none());
    }

    #[test]
    fn test_parse_manifest_m2_contributions() {
        let json = r#"{
            "name": "rustcode-m2-ext",
            "publisher": "rustcode",
            "version": "1.0.0",
            "contributes": {
                "debuggers": [
                    {
                        "type": "lldb",
                        "label": "LLDB",
                        "languages": ["rust", "cpp"]
                    }
                ],
                "taskDefinitions": [
                    {
                        "type": "cargo",
                        "required": ["command"]
                    }
                ],
                "authentication": [
                    {
                        "id": "github",
                        "label": "GitHub"
                    }
                ]
            }
        }"#;

        let manifest = ExtensionLoader::parse_manifest(json).unwrap();
        let contributes = manifest.contributes.unwrap();
        let debuggers = contributes.debuggers.unwrap();
        assert_eq!(debuggers.len(), 1);
        assert_eq!(debuggers[0].debugger_type, "lldb");
        assert_eq!(debuggers[0].languages, vec!["rust", "cpp"]);

        let task_defs = contributes.task_definitions.unwrap();
        assert_eq!(task_defs.len(), 1);
        assert_eq!(task_defs[0].task_type, "cargo");
        assert_eq!(task_defs[0].required, vec!["command"]);

        let auth = contributes.authentication.unwrap();
        assert_eq!(auth.len(), 1);
        assert_eq!(auth[0].id, "github");
        assert_eq!(auth[0].label, "GitHub");
    }

    #[test]
    fn test_parse_manifest_custom_editors_and_selector_matching() {
        let json = r#"{
            "name": "cat-scratch",
            "publisher": "rustcode",
            "version": "1.0.0",
            "contributes": {
                "customEditors": [
                    {
                        "viewType": "catEdit.catScratch",
                        "displayName": "Cat Scratch",
                        "selector": [
                            { "filenamePattern": "*.cscratch" },
                            { "filenamePattern": "**/notes-*.txt" }
                        ],
                        "priority": "default"
                    }
                ]
            }
        }"#;

        let manifest = ExtensionLoader::parse_manifest(json).unwrap();
        let contributes = manifest.contributes.unwrap();
        assert_eq!(contributes.custom_editors.len(), 1);
        let ce = &contributes.custom_editors[0];
        assert_eq!(ce.view_type, "catEdit.catScratch");
        assert_eq!(ce.display_name, "Cat Scratch");
        assert_eq!(ce.priority.as_deref(), Some("default"));
        assert!(ce.matches_path("file:///workspace/demo.cscratch"));
        assert!(ce.matches_path("/workspace/sub/notes-alpha.txt"));
        assert!(!ce.matches_path("file:///workspace/main.rs"));
    }

    #[test]
    fn test_custom_editor_selector_nested_paths_and_separators() {
        let nested_ce = CustomEditorContribution {
            view_type: "catEdit.nested".to_string(),
            display_name: "Nested".to_string(),
            selector: vec![CustomEditorSelector {
                filename_pattern: "**/sub/*.cscratch".to_string(),
            }],
            priority: None,
        };
        assert!(nested_ce.matches_path("/a/sub/demo.cscratch"));
        assert!(nested_ce.matches_path("file:///a/sub/demo.cscratch"));
        assert!(nested_ce.matches_path("sub/demo.cscratch"));
        assert!(nested_ce.matches_path("C:\\a\\sub\\demo.cscratch"));
        assert!(!nested_ce.matches_path("/a/xsub/demo.cscratch"));
        assert!(!nested_ce.matches_path("/a/demo.cscratch"));

        let win_ce = CustomEditorContribution {
            view_type: "hex.win".to_string(),
            display_name: "Win".to_string(),
            selector: vec![CustomEditorSelector {
                filename_pattern: "sub\\*.bin".to_string(),
            }],
            priority: None,
        };
        assert!(win_ce.matches_path("sub\\demo.bin"));
        assert!(win_ce.matches_path("sub/demo.bin"));
        assert!(!win_ce.matches_path("demo.bin"));

        let ext_ce = CustomEditorContribution {
            view_type: "hex.ext".to_string(),
            display_name: "Ext".to_string(),
            selector: vec![CustomEditorSelector {
                filename_pattern: "*.bin".to_string(),
            }],
            priority: None,
        };
        assert!(!ext_ce.matches_path("foo.bin.txt"));
        assert!(ext_ce.matches_path("C:\\dir\\foo.bin"));
    }

    #[test]
    fn test_extension_snapshot_from_loaded_and_loader_snapshots() {
        let dir = std::env::temp_dir().join(format!(
            "rustcode_manifest_snap_{}",
            std::process::id()
        ));
        let ext_a = dir.join("ext-a");
        let ext_b = dir.join("ext-b");
        std::fs::create_dir_all(&ext_a).unwrap();
        std::fs::create_dir_all(&ext_b).unwrap();
        std::fs::write(
            ext_a.join("package.json"),
            r#"{"name": "hello-world", "publisher": "RustCode", "version": "0.1.0"}"#,
        )
        .unwrap();
        std::fs::write(
            ext_b.join("package.json"),
            r#"{"name": "local-only", "publisher": "", "version": "0.2.0"}"#,
        )
        .unwrap();

        let mut loader = ExtensionLoader::new();
        loader.load_from_dir(&ext_a).unwrap();
        loader.load_from_dir(&ext_b).unwrap();
        assert!(loader.activate("hello-world"));

        let snaps = loader.snapshots();
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].id, "RustCode.hello-world");
        assert!(snaps[0].is_active);
        assert_eq!(snaps[0].package_json["name"], "hello-world");
        assert_eq!(snaps[0].extension_kind, 1);
        assert_eq!(snaps[1].id, "local-only");
        assert!(!snaps[1].is_active);

        let found = loader.snapshot_for("rustcode.hello-world").unwrap();
        assert_eq!(found.id, "RustCode.hello-world");
        assert!(loader.snapshot_for("missing.ext").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
