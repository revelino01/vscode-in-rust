//! VS Code `language-configuration.json` loader and language registry.

use crate::snippets::strip_jsonc_comments_and_trailing_commas;
use crate::view::comments::CommentConfig;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentRule {
    #[serde(rename = "lineComment", default)]
    pub line_comment: Option<String>,
    #[serde(rename = "blockComment", default)]
    pub block_comment: Option<[String; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AutoClosingPair {
    pub open: String,
    pub close: String,
    #[serde(rename = "notIn", default)]
    pub not_in: Vec<String>,
}

impl<'de> Deserialize<'de> for AutoClosingPair {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = Value::deserialize(deserializer)?;
        match val {
            Value::Array(arr) if arr.len() >= 2 => Ok(Self {
                open: arr[0].as_str().unwrap_or("").to_string(),
                close: arr[1].as_str().unwrap_or("").to_string(),
                not_in: Vec::new(),
            }),
            Value::Object(map) => {
                let open = map
                    .get("open")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let close = map
                    .get("close")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let not_in = map
                    .get("notIn")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(Self {
                    open,
                    close,
                    not_in,
                })
            }
            _ => Err(serde::de::Error::custom(
                "Expected [open, close] pair or { open, close } object",
            )),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndentationRule {
    #[serde(rename = "increaseIndentPattern", default)]
    pub increase_indent_pattern: Option<String>,
    #[serde(rename = "decreaseIndentPattern", default)]
    pub decrease_indent_pattern: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum EnterIndentKind {
    #[default]
    None,
    Indent,
    IndentOutdent,
    Outdent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterActionSpec {
    #[serde(default)]
    pub indent: EnterIndentKind,
    #[serde(rename = "appendText", default)]
    pub append_text: Option<String>,
    #[serde(rename = "removeText", default)]
    pub remove_text: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnEnterRule {
    #[serde(rename = "beforeText")]
    pub before_text: String,
    #[serde(rename = "afterText", default)]
    pub after_text: Option<String>,
    pub action: EnterActionSpec,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageConfiguration {
    #[serde(default)]
    pub comments: CommentRule,
    #[serde(default)]
    pub brackets: Vec<[String; 2]>,
    #[serde(rename = "autoClosingPairs", default)]
    pub auto_closing_pairs: Vec<AutoClosingPair>,
    #[serde(rename = "surroundingPairs", default)]
    pub surrounding_pairs: Vec<AutoClosingPair>,
    #[serde(rename = "indentationRules", default)]
    pub indentation_rules: Option<IndentationRule>,
    #[serde(rename = "onEnterRules", default)]
    pub on_enter_rules: Vec<OnEnterRule>,
    #[serde(rename = "wordPattern", default)]
    pub word_pattern: Option<String>,
}

impl LanguageConfiguration {
    pub fn from_json_str(raw: &str) -> Result<Self, String> {
        let cleaned = strip_jsonc_comments_and_trailing_commas(raw);
        serde_json::from_str::<Self>(&cleaned)
            .map_err(|e| format!("Failed parsing language-configuration.json: {}", e))
    }

    pub fn to_comment_config(&self) -> Option<CommentConfig> {
        let block = self
            .comments
            .block_comment
            .as_ref()
            .map(|b| (b[0].clone(), b[1].clone()));
        if self.comments.line_comment.is_none() && block.is_none() {
            None
        } else {
            Some(CommentConfig {
                line_comment: self.comments.line_comment.clone(),
                block_comment: block,
            })
        }
    }

    pub fn matching_auto_closing_pair(&self, ch: char) -> Option<&str> {
        let mut buf = [0u8; 4];
        let s: &str = ch.encode_utf8(&mut buf);
        self.auto_closing_pairs
            .iter()
            .find(|p| p.open == s && !p.close.is_empty())
            .map(|p| p.close.as_str())
    }

    pub fn matching_surrounding_pair(&self, ch: char) -> Option<&str> {
        let mut buf = [0u8; 4];
        let s: &str = ch.encode_utf8(&mut buf);
        self.surrounding_pairs
            .iter()
            .find(|p| p.open == s && !p.close.is_empty())
            .map(|p| p.close.as_str())
            .or_else(|| self.matching_auto_closing_pair(ch))
    }

    /// Compute newline insertion text and how many bytes back from end the cursor should sit.
    pub fn compute_enter_action(
        &self,
        line_before_cursor: &str,
        line_after_cursor: &str,
        indent_unit: &str,
    ) -> (String, usize) {
        let leading_ws: String = line_before_cursor
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();

        // 1. Check `onEnterRules` in priority order
        for rule in &self.on_enter_rules {
            let Ok(before_re) = Regex::new(&rule.before_text) else {
                continue;
            };
            if !before_re.is_match(line_before_cursor) {
                continue;
            }
            if let Some(after_pat) = &rule.after_text {
                let Ok(after_re) = Regex::new(after_pat) else {
                    continue;
                };
                if !after_re.is_match(line_after_cursor) {
                    continue;
                }
            }
            let append = rule.action.append_text.as_deref().unwrap_or("");
            match rule.action.indent {
                EnterIndentKind::IndentOutdent => {
                    let inner = format!("\n{}{}{}", leading_ws, indent_unit, append);
                    let tail = format!("\n{}", leading_ws);
                    let back = tail.len();
                    return (format!("{}{}", inner, tail), back);
                }
                EnterIndentKind::Indent => {
                    return (format!("\n{}{}{}", leading_ws, indent_unit, append), 0);
                }
                EnterIndentKind::Outdent | EnterIndentKind::None => {
                    return (format!("\n{}{}", leading_ws, append), 0);
                }
            }
        }

        // 2. Fallback: brace/paren split if `after` starts with matching closer
        let trimmed_before = line_before_cursor.trim_end();
        let trimmed_after = line_after_cursor.trim_start();
        if let (Some(open), Some(close)) = (trimmed_before.chars().last(), trimmed_after.chars().next())
        {
            let is_pair = self.brackets.iter().any(|b| {
                b[0].starts_with(open) && b[1].starts_with(close)
            });
            if is_pair {
                let inner = format!("\n{}{}", leading_ws, indent_unit);
                let tail = format!("\n{}", leading_ws);
                let back = tail.len();
                return (format!("{}{}", inner, tail), back);
            }
        }

        // 3. Check `indentationRules.increaseIndentPattern`
        if let Some(rules) = &self.indentation_rules {
            if let Some(pat) = &rules.increase_indent_pattern {
                if let Ok(re) = Regex::new(pat) {
                    if re.is_match(line_before_cursor) {
                        return (format!("\n{}{}", leading_ws, indent_unit), 0);
                    }
                }
            }
        }

        (format!("\n{}", leading_ws), 0)
    }
}

fn brace_lang_config(
    line_comment: Option<&str>,
    extra_enter: Vec<OnEnterRule>,
) -> LanguageConfiguration {
    let pairs = vec![
        AutoClosingPair { open: "{".into(), close: "}".into(), not_in: vec!["string".into()] },
        AutoClosingPair { open: "[".into(), close: "]".into(), not_in: vec!["string".into()] },
        AutoClosingPair { open: "(".into(), close: ")".into(), not_in: vec!["string".into()] },
        AutoClosingPair { open: "\"".into(), close: "\"".into(), not_in: vec!["string".into()] },
        AutoClosingPair { open: "'".into(), close: "'".into(), not_in: vec!["string".into()] },
    ];
    LanguageConfiguration {
        comments: CommentRule {
            line_comment: line_comment.map(ToString::to_string),
            block_comment: Some(["/*".to_string(), "*/".to_string()]),
        },
        brackets: vec![
            ["{".into(), "}".into()],
            ["[".into(), "]".into()],
            ["(".into(), ")".into()],
        ],
        auto_closing_pairs: pairs.clone(),
        surrounding_pairs: pairs,
        indentation_rules: Some(IndentationRule {
            increase_indent_pattern: Some(r"^.*\{[^}]*$".to_string()),
            decrease_indent_pattern: Some(r"^\s*\}.*$".to_string()),
        }),
        on_enter_rules: extra_enter,
        word_pattern: None,
    }
}

#[derive(Debug, Clone)]
pub struct LanguageConfigurationRegistry {
    configs: HashMap<String, LanguageConfiguration>,
}

impl Default for LanguageConfigurationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageConfigurationRegistry {
    pub fn new() -> Self {
        let mut configs = HashMap::new();
        let doc_enter_rules = vec![
            OnEnterRule {
                before_text: r"^\s*///.*$".to_string(),
                after_text: None,
                action: EnterActionSpec {
                    indent: EnterIndentKind::None,
                    append_text: Some("/// ".to_string()),
                    remove_text: None,
                },
            },
            OnEnterRule {
                before_text: r"^\s*/\*\*(?!/)([^\*]|\*(?!/))*$".to_string(),
                after_text: Some(r"^\s*\*/$".to_string()),
                action: EnterActionSpec {
                    indent: EnterIndentKind::IndentOutdent,
                    append_text: Some("* ".to_string()),
                    remove_text: None,
                },
            },
        ];

        for lang in ["rust", "typescript", "javascript", "go", "c", "cpp"] {
            configs.insert(
                lang.to_string(),
                brace_lang_config(Some("//"), doc_enter_rules.clone()),
            );
        }
        configs.insert(
            "python".to_string(),
            LanguageConfiguration {
                comments: CommentRule {
                    line_comment: Some("#".to_string()),
                    block_comment: None,
                },
                brackets: vec![["[".into(), "]".into()], ["(".into(), ")".into()], ["{".into(), "}".into()]],
                auto_closing_pairs: vec![
                    AutoClosingPair { open: "(".into(), close: ")".into(), not_in: vec![] },
                    AutoClosingPair { open: "[".into(), close: "]".into(), not_in: vec![] },
                    AutoClosingPair { open: "{".into(), close: "}".into(), not_in: vec![] },
                    AutoClosingPair { open: "\"".into(), close: "\"".into(), not_in: vec![] },
                    AutoClosingPair { open: "'".into(), close: "'".into(), not_in: vec![] },
                ],
                surrounding_pairs: vec![
                    AutoClosingPair { open: "(".into(), close: ")".into(), not_in: vec![] },
                    AutoClosingPair { open: "[".into(), close: "]".into(), not_in: vec![] },
                    AutoClosingPair { open: "{".into(), close: "}".into(), not_in: vec![] },
                    AutoClosingPair { open: "\"".into(), close: "\"".into(), not_in: vec![] },
                    AutoClosingPair { open: "'".into(), close: "'".into(), not_in: vec![] },
                ],
                indentation_rules: Some(IndentationRule {
                    increase_indent_pattern: Some(r"^.*:\s*(#.*)?$".to_string()),
                    decrease_indent_pattern: Some(r"^\s*(return|break|continue|pass)\b".to_string()),
                }),
                on_enter_rules: Vec::new(),
                word_pattern: None,
            },
        );
        configs.insert("json".to_string(), brace_lang_config(Some("//"), Vec::new()));
        configs.insert("css".to_string(), brace_lang_config(None, Vec::new()));
        configs.insert(
            "html".to_string(),
            LanguageConfiguration {
                comments: CommentRule {
                    line_comment: None,
                    block_comment: Some(["<!--".to_string(), "-->".to_string()]),
                },
                brackets: vec![["<".into(), ">".into()]],
                auto_closing_pairs: vec![
                    AutoClosingPair { open: "\"".into(), close: "\"".into(), not_in: vec![] },
                    AutoClosingPair { open: "'".into(), close: "'".into(), not_in: vec![] },
                ],
                surrounding_pairs: vec![
                    AutoClosingPair { open: "\"".into(), close: "\"".into(), not_in: vec![] },
                    AutoClosingPair { open: "'".into(), close: "'".into(), not_in: vec![] },
                ],
                indentation_rules: None,
                on_enter_rules: Vec::new(),
                word_pattern: None,
            },
        );
        Self { configs }
    }

    pub fn register(&mut self, language_id: impl Into<String>, cfg: LanguageConfiguration) {
        self.configs.insert(language_id.into(), cfg);
    }

    pub fn load_from_json_str(
        &mut self,
        language_id: impl Into<String>,
        json: &str,
    ) -> Result<(), String> {
        let cfg = LanguageConfiguration::from_json_str(json)?;
        self.configs.insert(language_id.into(), cfg);
        Ok(())
    }

    pub fn get(&self, language_id: &str) -> Option<&LanguageConfiguration> {
        self.configs.get(language_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_configuration_json_loader_and_enter_rules() {
        let raw = r#"{
            // Language comments
            "comments": { "lineComment": "//", "blockComment": ["/*", "*/"] },
            "brackets": [["{", "}"], ["(", ")"]],
            "autoClosingPairs": [
                { "open": "{", "close": "}" },
                ["\"", "\""]
            ],
            "surroundingPairs": [["(", ")"], ["\"", "\""]],
            "indentationRules": {
                "increaseIndentPattern": "^.*\\{[^}]*$"
            },
            "onEnterRules": [
                {
                    "beforeText": "^\\s*///.*$",
                    "action": { "indent": "none", "appendText": "/// " }
                }
            ]
        }"#;

        let cfg = LanguageConfiguration::from_json_str(raw).unwrap();
        assert_eq!(cfg.comments.line_comment.as_deref(), Some("//"));
        assert_eq!(cfg.matching_surrounding_pair('('), Some(")"));

        // Enter after `fn main() {` with `}` after cursor -> IndentOutdent
        let (ins, back) = cfg.compute_enter_action("fn main() {", "}", "    ");
        assert_eq!(ins, "\n    \n");
        assert_eq!(back, 1);

        // Enter on `    /// doc line` -> continues `/// `
        let (doc_ins, doc_back) = cfg.compute_enter_action("    /// doc line", "", "    ");
        assert_eq!(doc_ins, "\n    /// ");
        assert_eq!(doc_back, 0);
    }
}
