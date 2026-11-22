//! Theme service loading VS Code theme JSON files.

use crate::configuration::ConfigurationService;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const EMBEDDED_DARK_PLUS_JSON: &str =
    include_str!("../../../../resources/default-themes/dark_plus.json");
const EMBEDDED_LIGHT_PLUS_JSON: &str =
    include_str!("../../../../resources/default-themes/light_plus.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeKind {
    Dark,
    Light,
    HighContrast,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenColorRule {
    pub name: Option<String>,
    pub scope: Vec<String>,
    pub foreground: Option<String>,
    pub font_style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorTheme {
    pub name: String,
    pub kind: ThemeKind,
    pub colors: HashMap<String, String>,
    pub token_colors: Vec<TokenColorRule>,
}

#[derive(Debug, Deserialize)]
struct RawThemeJson {
    name: Option<String>,
    #[serde(rename = "type")]
    theme_type: Option<String>,
    colors: Option<HashMap<String, String>>,
    #[serde(rename = "tokenColors")]
    token_colors: Option<Vec<RawTokenColor>>,
}

#[derive(Debug, Deserialize)]
struct RawTokenColor {
    name: Option<String>,
    scope: Option<RawScope>,
    settings: RawTokenSettings,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawScope {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct RawTokenSettings {
    foreground: Option<String>,
    #[serde(rename = "fontStyle")]
    font_style: Option<String>,
}

#[derive(Clone)]
pub struct ThemeService {
    current_theme: ColorTheme,
    themes: HashMap<String, ColorTheme>,
    aliases: HashMap<String, String>,
}

impl Default for ThemeService {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeService {
    pub fn new() -> Self {
        let mut svc = Self {
            current_theme: Self::default_dark_theme(),
            themes: HashMap::new(),
            aliases: HashMap::new(),
        };
        let _ = svc.register_builtin_themes(None);
        svc
    }

    pub fn current(&self) -> &ColorTheme {
        &self.current_theme
    }

    pub fn set_theme(&mut self, theme: ColorTheme) {
        self.register_theme(theme.clone());
        self.current_theme = theme;
    }

    pub fn register_theme(&mut self, theme: ColorTheme) {
        let key = normalize_theme_key(&theme.name);
        self.themes.insert(key, theme);
    }

    pub fn register_alias(&mut self, alias: &str, target_name: &str) {
        self.aliases.insert(
            normalize_theme_key(alias),
            normalize_theme_key(target_name),
        );
    }

    pub fn get_theme(&self, name_or_id: &str) -> Option<&ColorTheme> {
        let norm = normalize_theme_key(name_or_id);
        if let Some(theme) = self.themes.get(&norm) {
            return Some(theme);
        }
        if let Some(target_key) = self.aliases.get(&norm) {
            return self.themes.get(target_key);
        }
        None
    }

    pub fn registered_theme_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.themes.values().map(|t| t.name.clone()).collect();
        names.sort();
        names
    }

    pub fn set_theme_by_name(&mut self, name_or_id: &str) -> Result<&ColorTheme, String> {
        let found = self
            .get_theme(name_or_id)
            .cloned()
            .ok_or_else(|| format!("Theme not found: {}", name_or_id))?;
        self.current_theme = found;
        Ok(&self.current_theme)
    }

    /// Synchronize active theme with `workbench.colorTheme` from `ConfigurationService`.
    pub fn sync_with_configuration(
        &mut self,
        config: &ConfigurationService,
    ) -> Result<bool, String> {
        let Some(theme_name) = config.get::<String>("workbench.colorTheme") else {
            return Ok(false);
        };
        self.set_theme_by_name(&theme_name)?;
        Ok(true)
    }

    /// Load a VS Code theme JSON/JSONC file from disk, register it, and return it.
    pub fn load_theme_from_disk(&mut self, path: &Path) -> Result<ColorTheme, String> {
        let content = std::fs::read_to_string(path).map_err(|err| {
            format!("Failed to read theme file '{}': {}", path.display(), err)
        })?;
        let theme = Self::parse_theme_json(&content)?;
        self.register_theme(theme.clone());
        Ok(theme)
    }

    /// Register built-in Dark+ and Light+ themes from `themes_dir` (if provided and files exist),
    /// falling back to `resources/default-themes` or embedded theme definitions.
    pub fn register_builtin_themes(&mut self, themes_dir: Option<&Path>) -> Result<(), String> {
        let fallback_dir = Path::new("resources").join("default-themes");
        let dir_opt = themes_dir.or_else(|| fallback_dir.is_dir().then_some(fallback_dir.as_path()));

        let dark_theme = match dir_opt.map(|d| d.join("dark_plus.json")) {
            Some(path) if path.is_file() => self.load_theme_from_disk(&path)?,
            _ => {
                let parsed = Self::parse_theme_json(EMBEDDED_DARK_PLUS_JSON)?;
                self.register_theme(parsed.clone());
                parsed
            }
        };

        let light_theme = match dir_opt.map(|d| d.join("light_plus.json")) {
            Some(path) if path.is_file() => self.load_theme_from_disk(&path)?,
            _ => {
                let parsed = Self::parse_theme_json(EMBEDDED_LIGHT_PLUS_JSON)?;
                self.register_theme(parsed.clone());
                parsed
            }
        };

        for alias in ["Default Dark+", "Dark+", "dark_plus"] {
            self.register_alias(alias, &dark_theme.name);
        }
        for alias in ["Default Light+", "Light+", "light_plus"] {
            self.register_alias(alias, &light_theme.name);
        }

        // Refresh active theme if it corresponds to one of the built-in dark/light themes
        let active_key = normalize_theme_key(&self.current_theme.name);
        if active_key == normalize_theme_key("Default Dark+")
            || active_key == normalize_theme_key(&dark_theme.name)
        {
            self.current_theme = dark_theme;
        } else if active_key == normalize_theme_key("Default Light+")
            || active_key == normalize_theme_key(&light_theme.name)
        {
            self.current_theme = light_theme;
        }

        Ok(())
    }

    /// Parse a VS Code theme JSON string (supports JSONC).
    pub fn parse_theme_json(json_str: &str) -> Result<ColorTheme, String> {
        let stripped = crate::configuration::strip_json_comments(json_str);
        let raw: RawThemeJson = serde_json::from_str(&stripped).map_err(|e| e.to_string())?;

        let name = raw.name.unwrap_or_else(|| "Custom Theme".to_string());
        let kind = match raw.theme_type.as_deref() {
            Some("light") => ThemeKind::Light,
            Some("hc") | Some("hcDark") | Some("hcLight") => ThemeKind::HighContrast,
            _ => ThemeKind::Dark,
        };

        let colors = raw.colors.unwrap_or_default();
        let mut token_colors = Vec::new();

        if let Some(raw_tokens) = raw.token_colors {
            for rt in raw_tokens {
                let scopes = match rt.scope {
                    Some(RawScope::Single(s)) => {
                        s.split(',').map(|p| p.trim().to_string()).collect()
                    }
                    Some(RawScope::Multiple(vec)) => vec,
                    None => Vec::new(),
                };

                token_colors.push(TokenColorRule {
                    name: rt.name,
                    scope: scopes,
                    foreground: rt.settings.foreground,
                    font_style: rt.settings.font_style,
                });
            }
        }

        Ok(ColorTheme {
            name,
            kind,
            colors,
            token_colors,
        })
    }

    pub fn default_dark_theme() -> ColorTheme {
        let mut colors = HashMap::new();
        colors.insert("editor.background".to_string(), "#1e1e1e".to_string());
        colors.insert("editor.foreground".to_string(), "#d4d4d4".to_string());
        colors.insert("activityBar.background".to_string(), "#333333".to_string());
        colors.insert("sideBar.background".to_string(), "#252526".to_string());
        colors.insert("statusBar.background".to_string(), "#007acc".to_string());

        ColorTheme {
            name: "Default Dark+".to_string(),
            kind: ThemeKind::Dark,
            colors,
            token_colors: vec![
                TokenColorRule {
                    name: Some("Comment".to_string()),
                    scope: vec!["comment".to_string()],
                    foreground: Some("#6A9955".to_string()),
                    font_style: None,
                },
                TokenColorRule {
                    name: Some("Keyword".to_string()),
                    scope: vec!["keyword".to_string(), "storage".to_string()],
                    foreground: Some("#C586C0".to_string()),
                    font_style: None,
                },
                TokenColorRule {
                    name: Some("String".to_string()),
                    scope: vec!["string".to_string()],
                    foreground: Some("#CE9178".to_string()),
                    font_style: None,
                },
            ],
        }
    }

    pub fn default_light_theme() -> ColorTheme {
        Self::parse_theme_json(EMBEDDED_LIGHT_PLUS_JSON).unwrap_or_else(|_| {
            let mut colors = HashMap::new();
            colors.insert("editor.background".to_string(), "#ffffff".to_string());
            colors.insert("editor.foreground".to_string(), "#000000".to_string());
            ColorTheme {
                name: "Light+ (default light)".to_string(),
                kind: ThemeKind::Light,
                colors,
                token_colors: Vec::new(),
            }
        })
    }
}

fn normalize_theme_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::ConfigurationTarget;
    use serde_json::json;

    #[test]
    fn test_theme_parsing() {
        let json = r##"{
            "name": "Monokai Classic",
            "type": "dark",
            "colors": {
                "editor.background": "#272822",
                "editor.foreground": "#f8f8f2"
            },
            "tokenColors": [
                {
                    "name": "Function declaration",
                    "scope": "entity.name.function, support.function",
                    "settings": {
                        "foreground": "#a6e22e"
                    }
                }
            ]
        }"##;

        let theme = ThemeService::parse_theme_json(json).unwrap();
        assert_eq!(theme.name, "Monokai Classic");
        assert_eq!(theme.kind, ThemeKind::Dark);
        assert_eq!(theme.colors.get("editor.background"), Some(&"#272822".to_string()));
        assert_eq!(theme.token_colors.len(), 1);
        assert_eq!(theme.token_colors[0].scope, vec!["entity.name.function", "support.function"]);
    }

    #[test]
    fn test_builtin_themes_disk_load_and_switch_by_configuration() {
        let mut themes = ThemeService::new();
        let names = themes.registered_theme_names();
        assert!(names.iter().any(|n| n.contains("Dark+")));
        assert!(names.iter().any(|n| n.contains("Light+")));
        assert_eq!(themes.current().kind, ThemeKind::Dark);
        assert_eq!(
            themes.current().colors.get("editor.background").map(String::as_str),
            Some("#1e1e1e")
        );

        // Switch via configuration `workbench.colorTheme`
        let cfg = ConfigurationService::new();
        cfg.update(
            "workbench.colorTheme",
            json!("Default Light+"),
            ConfigurationTarget::User,
        )
        .unwrap();
        assert!(themes.sync_with_configuration(&cfg).unwrap());
        assert_eq!(themes.current().kind, ThemeKind::Light);
        assert_eq!(
            themes.current().colors.get("editor.background").map(String::as_str),
            Some("#ffffff")
        );
        assert!(themes
            .current()
            .token_colors
            .iter()
            .any(|rule| rule.foreground.as_deref() == Some("#0000ff")));

        // Load a custom theme from disk and switch to it
        let dir = std::env::temp_dir().join(format!("rustcode_theme_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let custom_path = dir.join("nord_test.json");
        std::fs::write(
            &custom_path,
            r##"{
                // JSONC comment in theme file
                "name": "Nord Test",
                "type": "dark",
                "colors": {
                    "editor.background": "#2e3440",
                    "editor.foreground": "#d8dee9",
                },
                "tokenColors": [
                    {
                        "name": "Keyword",
                        "scope": ["keyword"],
                        "settings": { "foreground": "#81a1c1" },
                    },
                ],
            }"##,
        )
        .unwrap();

        let loaded = themes.load_theme_from_disk(&custom_path).unwrap();
        assert_eq!(loaded.name, "Nord Test");
        cfg.update(
            "workbench.colorTheme",
            json!("Nord Test"),
            ConfigurationTarget::Workspace,
        )
        .unwrap();
        assert!(themes.sync_with_configuration(&cfg).unwrap());
        assert_eq!(themes.current().name, "Nord Test");
        assert_eq!(
            themes.current().colors.get("editor.background").map(String::as_str),
            Some("#2e3440")
        );

        // Missing or unknown theme returns structured Err
        assert!(themes.set_theme_by_name("Missing Theme").is_err());
        assert!(themes.load_theme_from_disk(&dir.join("missing.json")).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
