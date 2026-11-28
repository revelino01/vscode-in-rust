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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconDefinition {
    #[serde(default)]
    pub icon_path: Option<String>,
    #[serde(default)]
    pub font_character: Option<String>,
    #[serde(default)]
    pub font_color: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconTheme {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub base_dir: Option<String>,
    #[serde(default)]
    pub icon_definitions: HashMap<String, IconDefinition>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub folder_expanded: Option<String>,
    #[serde(default)]
    pub root_folder: Option<String>,
    #[serde(default)]
    pub root_folder_expanded: Option<String>,
    #[serde(default)]
    pub file_extensions: HashMap<String, String>,
    #[serde(default)]
    pub file_names: HashMap<String, String>,
    #[serde(default)]
    pub folder_names: HashMap<String, String>,
    #[serde(default)]
    pub folder_names_expanded: HashMap<String, String>,
    #[serde(default)]
    pub language_ids: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductIconDefinition {
    #[serde(default)]
    pub font_character: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductIconTheme {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub icon_definitions: HashMap<String, ProductIconDefinition>,
}

#[derive(Clone)]
pub struct ThemeService {
    current_theme: ColorTheme,
    themes: HashMap<String, ColorTheme>,
    aliases: HashMap<String, String>,
    icon_themes: HashMap<String, IconTheme>,
    active_icon_theme_id: Option<String>,
    product_icon_themes: HashMap<String, ProductIconTheme>,
    active_product_icon_theme_id: Option<String>,
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
            icon_themes: HashMap::new(),
            active_icon_theme_id: Some("vs-seti".to_string()),
            product_icon_themes: HashMap::new(),
            active_product_icon_theme_id: Some("Default".to_string()),
        };
        let _ = svc.register_builtin_themes(None);
        svc.register_builtin_icon_themes();
        svc
    }

    fn register_builtin_icon_themes(&mut self) {
        let mut seti = IconTheme {
            id: "vs-seti".to_string(),
            label: "Seti (Visual Studio Code)".to_string(),
            file: Some("_default".to_string()),
            folder: Some("_folder".to_string()),
            folder_expanded: Some("_folder_open".to_string()),
            ..Default::default()
        };
        for (id, ch) in [
            ("_default", "\u{E023}"),
            ("_folder", "\u{E033}"),
            ("_folder_open", "\u{E034}"),
            ("_rust", "\u{E07E}"),
            ("_typescript", "\u{E097}"),
            ("_javascript", "\u{E051}"),
            ("_json", "\u{E055}"),
            ("_python", "\u{E075}"),
            ("_markdown", "\u{E062}"),
        ] {
            seti.icon_definitions.insert(
                id.to_string(),
                IconDefinition {
                    icon_path: None,
                    font_character: Some(ch.to_string()),
                    font_color: None,
                },
            );
        }
        seti.file_extensions.insert("rs".to_string(), "_rust".to_string());
        seti.file_extensions.insert("ts".to_string(), "_typescript".to_string());
        seti.file_extensions.insert("js".to_string(), "_javascript".to_string());
        seti.file_extensions.insert("json".to_string(), "_json".to_string());
        seti.file_extensions.insert("py".to_string(), "_python".to_string());
        seti.file_extensions.insert("md".to_string(), "_markdown".to_string());
        self.register_icon_theme(seti);

        let mut default_product = ProductIconTheme {
            id: "Default".to_string(),
            label: "Default".to_string(),
            icon_definitions: HashMap::new(),
        };
        for (id, ch) in [
            ("explorer-view-icon", "\u{EA7B}"),
            ("search-view-icon", "\u{EA6D}"),
            ("source-control-view-icon", "\u{EA68}"),
            ("debug-view-icon", "\u{EA87}"),
            ("extensions-view-icon", "\u{EAEE}"),
        ] {
            default_product.icon_definitions.insert(
                id.to_string(),
                ProductIconDefinition {
                    font_character: Some(ch.to_string()),
                },
            );
        }
        self.register_product_icon_theme(default_product);
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

    /// Synchronize active themes with `workbench.colorTheme`, `workbench.iconTheme`,
    /// and `workbench.productIconTheme` from `ConfigurationService`.
    pub fn sync_with_configuration(
        &mut self,
        config: &ConfigurationService,
    ) -> Result<bool, String> {
        let Some(theme_name) = config.get::<String>("workbench.colorTheme") else {
            return Ok(false);
        };
        self.set_theme_by_name(&theme_name)?;
        if let Some(icon_theme) = config.get::<String>("workbench.iconTheme") {
            let norm = normalize_theme_key(&icon_theme);
            if self.icon_themes.contains_key(&norm) {
                self.active_icon_theme_id = Some(norm);
            }
        }
        if let Some(prod_theme) = config.get::<String>("workbench.productIconTheme") {
            let norm = normalize_theme_key(&prod_theme);
            if self.product_icon_themes.contains_key(&norm) {
                self.active_product_icon_theme_id = Some(norm);
            }
        }
        Ok(true)
    }

    pub fn register_icon_theme(&mut self, theme: IconTheme) {
        let key = normalize_theme_key(&theme.id);
        self.icon_themes.insert(key, theme);
    }

    pub fn active_icon_theme(&self) -> Option<&IconTheme> {
        self.active_icon_theme_id
            .as_deref()
            .and_then(|id| self.icon_themes.get(&normalize_theme_key(id)))
    }

    pub fn set_icon_theme(&mut self, id: &str) -> Result<&IconTheme, String> {
        let key = normalize_theme_key(id);
        if !self.icon_themes.contains_key(&key) {
            return Err(format!("Icon theme not found: {}", id));
        }
        self.active_icon_theme_id = Some(key.clone());
        Ok(self.icon_themes.get(&key).expect("verified key"))
    }

    pub fn registered_icon_theme_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.icon_themes.values().map(|t| t.id.clone()).collect();
        ids.sort();
        ids
    }

    pub fn parse_icon_theme_json(
        id: impl Into<String>,
        label: impl Into<String>,
        base_dir: Option<&Path>,
        json_str: &str,
    ) -> Result<IconTheme, String> {
        let stripped = crate::configuration::strip_json_comments(json_str);
        let mut theme: IconTheme = serde_json::from_str(&stripped).map_err(|e| e.to_string())?;
        theme.id = id.into();
        theme.label = label.into();
        theme.base_dir = base_dir.map(|p| p.to_string_lossy().to_string());
        Ok(theme)
    }

    pub fn load_icon_theme_from_disk(
        &mut self,
        id: impl Into<String>,
        label: impl Into<String>,
        path: &Path,
    ) -> Result<IconTheme, String> {
        let content = std::fs::read_to_string(path).map_err(|err| {
            format!(
                "Failed to read icon theme file '{}': {}",
                path.display(),
                err
            )
        })?;
        let theme = Self::parse_icon_theme_json(id, label, path.parent(), &content)?;
        self.register_icon_theme(theme.clone());
        Ok(theme)
    }

    pub fn resolve_file_icon(
        &self,
        file_name: &str,
        language_id: Option<&str>,
    ) -> Option<(String, Option<&IconDefinition>)> {
        let theme = self.active_icon_theme()?;
        let base_name = Path::new(file_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(file_name);
        let lower = base_name.to_ascii_lowercase();

        let icon_id = theme
            .file_names
            .get(base_name)
            .or_else(|| theme.file_names.get(&lower))
            .or_else(|| {
                // Match longest multi-part extension first (e.g. `d.ts` before `ts`).
                let mut rest = lower.as_str();
                while let Some((_, tail)) = rest.split_once('.') {
                    if let Some(id) = theme.file_extensions.get(tail) {
                        return Some(id);
                    }
                    rest = tail;
                }
                None
            })
            .or_else(|| language_id.and_then(|lang| theme.language_ids.get(lang)))
            .or(theme.file.as_ref())?
            .clone();

        let def = theme.icon_definitions.get(&icon_id);
        Some((icon_id, def))
    }

    pub fn resolve_folder_icon(
        &self,
        folder_name: &str,
        expanded: bool,
    ) -> Option<(String, Option<&IconDefinition>)> {
        let theme = self.active_icon_theme()?;
        let base_name = Path::new(folder_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(folder_name);
        let lower = base_name.to_ascii_lowercase();

        let icon_id = if expanded {
            theme
                .folder_names_expanded
                .get(base_name)
                .or_else(|| theme.folder_names_expanded.get(&lower))
                .or_else(|| theme.folder_names.get(base_name))
                .or_else(|| theme.folder_names.get(&lower))
                .or(theme.folder_expanded.as_ref())
                .or(theme.folder.as_ref())?
                .clone()
        } else {
            theme
                .folder_names
                .get(base_name)
                .or_else(|| theme.folder_names.get(&lower))
                .or(theme.folder.as_ref())?
                .clone()
        };

        let def = theme.icon_definitions.get(&icon_id);
        Some((icon_id, def))
    }

    pub fn register_product_icon_theme(&mut self, theme: ProductIconTheme) {
        let key = normalize_theme_key(&theme.id);
        self.product_icon_themes.insert(key, theme);
    }

    pub fn active_product_icon_theme(&self) -> Option<&ProductIconTheme> {
        self.active_product_icon_theme_id
            .as_deref()
            .and_then(|id| self.product_icon_themes.get(&normalize_theme_key(id)))
    }

    pub fn set_product_icon_theme(&mut self, id: &str) -> Result<&ProductIconTheme, String> {
        let key = normalize_theme_key(id);
        if !self.product_icon_themes.contains_key(&key) {
            return Err(format!("Product icon theme not found: {}", id));
        }
        self.active_product_icon_theme_id = Some(key.clone());
        Ok(self.product_icon_themes.get(&key).expect("verified key"))
    }

    pub fn parse_product_icon_theme_json(
        id: impl Into<String>,
        label: impl Into<String>,
        json_str: &str,
    ) -> Result<ProductIconTheme, String> {
        let stripped = crate::configuration::strip_json_comments(json_str);
        let mut theme: ProductIconTheme =
            serde_json::from_str(&stripped).map_err(|e| e.to_string())?;
        theme.id = id.into();
        theme.label = label.into();
        Ok(theme)
    }

    pub fn load_product_icon_theme_from_disk(
        &mut self,
        id: impl Into<String>,
        label: impl Into<String>,
        path: &Path,
    ) -> Result<ProductIconTheme, String> {
        let content = std::fs::read_to_string(path).map_err(|err| {
            format!(
                "Failed to read product icon theme file '{}': {}",
                path.display(),
                err
            )
        })?;
        let theme = Self::parse_product_icon_theme_json(id, label, &content)?;
        self.register_product_icon_theme(theme.clone());
        Ok(theme)
    }

    pub fn resolve_product_icon(&self, icon_id: &str) -> Option<&ProductIconDefinition> {
        self.active_product_icon_theme()?
            .icon_definitions
            .get(icon_id)
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

    #[test]
    fn test_icon_themes_and_product_icon_themes() {
        let mut themes = ThemeService::new();
        assert_eq!(
            themes.resolve_file_icon("main.rs", None).map(|(id, _)| id),
            Some("_rust".to_string())
        );

        let dir =
            std::env::temp_dir().join(format!("rustcode_icontheme_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let icon_json = dir.join("material-icons.json");
        std::fs::write(
            &icon_json,
            r##"{
                // Custom Material Icon Theme
                "iconDefinitions": {
                    "_file": { "iconPath": "./icons/file.svg" },
                    "_folder": { "iconPath": "./icons/folder.svg" },
                    "_folder_open": { "iconPath": "./icons/folder-open.svg" },
                    "_dts": { "iconPath": "./icons/dts.svg" },
                    "_ts": { "iconPath": "./icons/ts.svg" },
                    "_cargo": { "fontCharacter": "\\e901", "fontColor": "#dea584" },
                    "_src_folder": { "iconPath": "./icons/src.svg" },
                    "_src_folder_open": { "iconPath": "./icons/src-open.svg" }
                },
                "file": "_file",
                "folder": "_folder",
                "folderExpanded": "_folder_open",
                "fileNames": {
                    "cargo.toml": "_cargo"
                },
                "fileExtensions": {
                    "d.ts": "_dts",
                    "ts": "_ts"
                },
                "folderNames": {
                    "src": "_src_folder"
                },
                "folderNamesExpanded": {
                    "src": "_src_folder_open"
                }
            }"##,
        )
        .unwrap();
        themes
            .load_icon_theme_from_disk("material-icon-theme", "Material Icon Theme", &icon_json)
            .unwrap();
        themes.set_icon_theme("material-icon-theme").unwrap();

        assert_eq!(
            themes.resolve_file_icon("types.d.ts", None).map(|(id, _)| id),
            Some("_dts".to_string())
        );
        assert_eq!(
            themes.resolve_file_icon("index.ts", None).map(|(id, _)| id),
            Some("_ts".to_string())
        );
        let (cargo_id, cargo_def) = themes.resolve_file_icon("Cargo.toml", None).unwrap();
        assert_eq!(cargo_id, "_cargo");
        assert_eq!(cargo_def.unwrap().font_color.as_deref(), Some("#dea584"));
        assert_eq!(
            themes.resolve_folder_icon("src", false).map(|(id, _)| id),
            Some("_src_folder".to_string())
        );
        assert_eq!(
            themes.resolve_folder_icon("src", true).map(|(id, _)| id),
            Some("_src_folder_open".to_string())
        );

        let prod_json = dir.join("codicons-custom.json");
        std::fs::write(
            &prod_json,
            r#"{
                "iconDefinitions": {
                    "custom-run": { "fontCharacter": "\\eb01" }
                }
            }"#,
        )
        .unwrap();
        themes
            .load_product_icon_theme_from_disk("custom-codicons", "Custom Codicons", &prod_json)
            .unwrap();
        themes.set_product_icon_theme("custom-codicons").unwrap();
        assert_eq!(
            themes
                .resolve_product_icon("custom-run")
                .and_then(|d| d.font_character.as_deref()),
            Some("\\eb01")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
