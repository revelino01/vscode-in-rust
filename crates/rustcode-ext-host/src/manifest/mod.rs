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
}
