//! Theme service loading VS Code theme JSON files.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

pub struct ThemeService {
    current_theme: ColorTheme,
}

impl Default for ThemeService {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeService {
    pub fn new() -> Self {
        Self {
            current_theme: Self::default_dark_theme(),
        }
    }

    pub fn current(&self) -> &ColorTheme {
        &self.current_theme
    }

    pub fn set_theme(&mut self, theme: ColorTheme) {
        self.current_theme = theme;
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
