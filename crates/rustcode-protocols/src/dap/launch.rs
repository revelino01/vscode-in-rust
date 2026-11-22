//! VS Code `.vscode/launch.json` JSONC parser and launch variable expander.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors produced when loading, stripping JSONC, or validating `.vscode/launch.json`.
#[derive(Debug, Error)]
pub enum LaunchConfigError {
    #[error("I/O error reading launch.json: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSONC syntax: {0}")]
    InvalidJsonc(String),
    #[error("invalid launch.json JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid launch configuration: {0}")]
    InvalidConfiguration(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaunchRequestKind {
    Launch,
    Attach,
}

/// A single entry inside `.vscode/launch.json`'s `"configurations"` array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchConfiguration {
    pub name: String,
    #[serde(rename = "type")]
    pub adapter_type: String,
    pub request: String,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default, rename = "stopOnEntry")]
    pub stop_on_entry: Option<bool>,
    #[serde(default, rename = "preLaunchTask")]
    pub pre_launch_task: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl LaunchConfiguration {
    pub fn request_kind(&self) -> Option<LaunchRequestKind> {
        match self.request.as_str() {
            "launch" => Some(LaunchRequestKind::Launch),
            "attach" => Some(LaunchRequestKind::Attach),
            _ => None,
        }
    }

    /// Returns a copy of this configuration with VS Code variables (`${workspaceFolder}`,
    /// `${file}`, `${fileBasename}`, `${env:VAR}`, etc.) expanded.
    pub fn resolve(&self, ctx: &LaunchVariableContext) -> Self {
        let mut resolved = self.clone();
        resolved.program = self
            .program
            .as_deref()
            .map(|s| expand_launch_variables(s, ctx));
        resolved.args = self
            .args
            .iter()
            .map(|a| expand_launch_variables(a, ctx))
            .collect();
        resolved.cwd = self
            .cwd
            .as_deref()
            .map(|s| expand_launch_variables(s, ctx));
        resolved.env = self
            .env
            .iter()
            .map(|(k, v)| (k.clone(), expand_launch_variables(v, ctx)))
            .collect();
        resolved.extra = self
            .extra
            .iter()
            .map(|(k, v)| (k.clone(), expand_value_variables(v, ctx)))
            .collect();
        resolved
    }
}

/// Top-level `.vscode/launch.json` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LaunchConfigFile {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub configurations: Vec<LaunchConfiguration>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl LaunchConfigFile {
    /// Expands variables across all configurations in the file.
    pub fn resolve(&self, ctx: &LaunchVariableContext) -> Self {
        Self {
            version: self.version.clone(),
            configurations: self
                .configurations
                .iter()
                .map(|c| c.resolve(ctx))
                .collect(),
            extra: self.extra.clone(),
        }
    }
}

/// Context used for expanding VS Code launch configuration variables.
#[derive(Debug, Clone, Default)]
pub struct LaunchVariableContext {
    pub workspace_folder: PathBuf,
    pub active_file: Option<PathBuf>,
    pub env_overrides: HashMap<String, String>,
}

impl LaunchVariableContext {
    pub fn new(workspace_folder: impl Into<PathBuf>) -> Self {
        Self {
            workspace_folder: workspace_folder.into(),
            active_file: None,
            env_overrides: HashMap::new(),
        }
    }

    pub fn with_active_file(mut self, file: impl Into<PathBuf>) -> Self {
        self.active_file = Some(file.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_overrides.insert(key.into(), value.into());
        self
    }
}

/// Strips JSONC `//` line comments, `/* ... */` block comments, and trailing commas
/// without altering string literals (including URLs like `"http://..."`).
pub fn strip_jsonc(input: &str) -> Result<String, LaunchConfigError> {
    let input = input.strip_prefix('\u{FEFF}').unwrap_or(input);
    let without_comments = strip_jsonc_comments(input)?;
    Ok(strip_trailing_commas(&without_comments))
}

fn strip_jsonc_comments(input: &str) -> Result<String, LaunchConfigError> {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut in_string = false;
    let mut escaped = false;

    while i < bytes.len() {
        let ch = input[i..].chars().next().ok_or_else(|| {
            LaunchConfigError::InvalidJsonc("unexpected end of UTF-8 stream".to_string())
        })?;
        let ch_len = ch.len_utf8();

        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += ch_len;
            continue;
        }

        if ch == '"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }

        if ch == '/' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'/' {
                // Line comment: skip until newline (preserve newline for error line numbers)
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if next == b'*' {
                // Block comment: skip until */ (preserve newlines)
                i += 2;
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                        i += 2;
                        closed = true;
                        break;
                    }
                    if bytes[i] == b'\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                if !closed {
                    return Err(LaunchConfigError::InvalidJsonc(
                        "unterminated /* ... */ block comment".to_string(),
                    ));
                }
                continue;
            }
        }

        out.push(ch);
        i += ch_len;
    }

    Ok(out)
}

fn strip_trailing_commas(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut in_string = false;
    let mut escaped = false;

    while i < bytes.len() {
        let ch = match input[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        let ch_len = ch.len_utf8();

        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += ch_len;
            continue;
        }

        if ch == '"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }

        if ch == ',' {
            // Look ahead past ASCII whitespace; if next token is `}` or `]`, drop comma
            let mut j = i + 1;
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\r' | b'\n') {
                j += 1;
            }
            if j < bytes.len() && matches!(bytes[j], b'}' | b']') {
                i += 1;
                continue;
            }
        }

        out.push(ch);
        i += ch_len;
    }

    out
}

/// Parses JSONC `launch.json` text into a validated `LaunchConfigFile`.
pub fn parse_launch_json(content: &str) -> Result<LaunchConfigFile, LaunchConfigError> {
    let cleaned = strip_jsonc(content)?;
    let parsed: LaunchConfigFile = serde_json::from_str(&cleaned)?;

    for cfg in &parsed.configurations {
        if cfg.name.trim().is_empty() {
            return Err(LaunchConfigError::InvalidConfiguration(
                "configuration `name` cannot be empty".to_string(),
            ));
        }
        if cfg.adapter_type.trim().is_empty() {
            return Err(LaunchConfigError::InvalidConfiguration(format!(
                "configuration `{}` has empty `type`",
                cfg.name
            )));
        }
        if cfg.request != "launch" && cfg.request != "attach" {
            return Err(LaunchConfigError::InvalidConfiguration(format!(
                "configuration `{}` has invalid `request`: `{}` (expected `launch` or `attach`)",
                cfg.name, cfg.request
            )));
        }
    }

    Ok(parsed)
}

/// Loads and parses `<workspace_root>/.vscode/launch.json`.
pub fn load_launch_json(workspace_root: &Path) -> Result<LaunchConfigFile, LaunchConfigError> {
    let path = workspace_root.join(".vscode").join("launch.json");
    let content = std::fs::read_to_string(path)?;
    parse_launch_json(&content)
}

/// Expands VS Code launch variables (`${workspaceFolder}`, `${file}`, `${fileBasename}`, `${env:VAR}`, etc.).
pub fn expand_launch_variables(input: &str, ctx: &LaunchVariableContext) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        if let Some(end) = after_open.find('}') {
            let var_name = &after_open[..end];
            if let Some(replacement) = resolve_variable(var_name, ctx) {
                out.push_str(&replacement);
            } else {
                out.push_str("${");
                out.push_str(var_name);
                out.push('}');
            }
            rest = &after_open[end + 1..];
        } else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        }
    }

    out.push_str(rest);
    out
}

fn resolve_variable(name: &str, ctx: &LaunchVariableContext) -> Option<String> {
    match name {
        "workspaceFolder" => Some(ctx.workspace_folder.to_string_lossy().into_owned()),
        "workspaceFolderBasename" => Some(
            ctx.workspace_folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        "file" => Some(
            ctx.active_file
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        "fileBasename" => Some(
            ctx.active_file
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        "fileBasenameNoExtension" => Some(
            ctx.active_file
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        "fileDirname" => Some(
            ctx.active_file
                .as_ref()
                .and_then(|p| p.parent())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        "fileExtname" => Some(
            ctx.active_file
                .as_ref()
                .and_then(|p| p.extension())
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default(),
        ),
        _ if name.starts_with("env:") => {
            let env_key = &name[4..];
            let val = ctx
                .env_overrides
                .get(env_key)
                .cloned()
                .or_else(|| std::env::var(env_key).ok())
                .unwrap_or_default();
            Some(val)
        }
        _ => None,
    }
}

fn expand_value_variables(val: &Value, ctx: &LaunchVariableContext) -> Value {
    match val {
        Value::String(s) => Value::String(expand_launch_variables(s, ctx)),
        Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| expand_value_variables(v, ctx)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), expand_value_variables(v, ctx)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_jsonc_preserves_urls_and_strings() {
        let jsonc = r#"{
            // Top-level comment
            "url": "http://localhost:8080/api/*not-a-comment*/",
            /* Block comment
               across lines */
            "text": "escaped \" // not comment, ",
            "arr": [1, 2, /* trailing */ ],
        }"#;
        let stripped = strip_jsonc(jsonc).unwrap();
        let val: Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(val["url"], "http://localhost:8080/api/*not-a-comment*/");
        assert_eq!(val["text"], "escaped \" // not comment, ");
        assert_eq!(val["arr"], serde_json::json!([1, 2]));
    }

    #[test]
    fn test_parse_launch_json_comments_trailing_commas_and_extra_fields() {
        let jsonc = "\u{FEFF}{
            // Use IntelliSense to learn about possible attributes.
            \"version\": \"0.2.0\",
            \"configurations\": [
                {
                    \"name\": \"Debug Rust Binary\",
                    \"type\": \"lldb\",
                    \"request\": \"launch\",
                    \"program\": \"${workspaceFolder}/target/debug/app\",
                    \"args\": [\"--input\", \"${file}\", \"--base\", \"${fileBasename}\",],
                    \"cwd\": \"${workspaceFolder}\",
                    \"env\": {
                        \"RUST_LOG\": \"debug\",
                        \"TOKEN\": \"${env:TEST_LAUNCH_TOKEN}\",
                        \"ENDPOINT\": \"https://example.com/v1\", // inline comment
                    },
                    \"stopOnEntry\": true,
                    \"preLaunchTask\": \"cargo build\",
                    \"sourceLanguages\": [\"rust\"],
                },
                {
                    \"name\": \"Attach Node\",
                    \"type\": \"node\",
                    \"request\": \"attach\",
                    \"port\": 9229,
                },
            ],
        }";

        let parsed = parse_launch_json(jsonc).unwrap();
        assert_eq!(parsed.version.as_deref(), Some("0.2.0"));
        assert_eq!(parsed.configurations.len(), 2);

        let cfg0 = &parsed.configurations[0];
        assert_eq!(cfg0.name, "Debug Rust Binary");
        assert_eq!(cfg0.adapter_type, "lldb");
        assert_eq!(cfg0.request_kind(), Some(LaunchRequestKind::Launch));
        assert_eq!(cfg0.stop_on_entry, Some(true));
        assert_eq!(cfg0.pre_launch_task.as_deref(), Some("cargo build"));
        assert_eq!(cfg0.extra.get("sourceLanguages"), Some(&serde_json::json!(["rust"])));

        let cfg1 = &parsed.configurations[1];
        assert_eq!(cfg1.request_kind(), Some(LaunchRequestKind::Attach));
        assert_eq!(cfg1.extra.get("port"), Some(&serde_json::json!(9229)));

        // Variable expansion
        let ctx = LaunchVariableContext::new("/workspace/myproj")
            .with_active_file("/workspace/myproj/src/main.rs")
            .with_env("TEST_LAUNCH_TOKEN", "secret-123");

        let resolved = cfg0.resolve(&ctx);
        assert_eq!(
            resolved.program.as_deref(),
            Some("/workspace/myproj/target/debug/app")
        );
        assert_eq!(
            resolved.args,
            vec![
                "--input",
                "/workspace/myproj/src/main.rs",
                "--base",
                "main.rs"
            ]
        );
        assert_eq!(resolved.cwd.as_deref(), Some("/workspace/myproj"));
        assert_eq!(resolved.env.get("TOKEN").map(String::as_str), Some("secret-123"));
        assert_eq!(
            resolved.env.get("ENDPOINT").map(String::as_str),
            Some("https://example.com/v1")
        );
    }

    #[test]
    fn test_parse_launch_json_malformed_returns_structured_error() {
        assert!(matches!(
            parse_launch_json("{ /* unterminated"),
            Err(LaunchConfigError::InvalidJsonc(_))
        ));
        assert!(matches!(
            parse_launch_json("{ \"configurations\": [ { \"name\": 123 } ] }"),
            Err(LaunchConfigError::Json(_))
        ));
        assert!(matches!(
            parse_launch_json(
                r#"{ "configurations": [ { "name": "Bad", "type": "lldb", "request": "restart" } ] }"#
            ),
            Err(LaunchConfigError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn test_load_launch_json_from_workspace() {
        let unique = format!(
            "rustcode_launch_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let vscode_dir = root.join(".vscode");
        std::fs::create_dir_all(&vscode_dir).unwrap();
        std::fs::write(
            vscode_dir.join("launch.json"),
            r#"{ "version": "0.2.0", "configurations": [ { "name": "Run", "type": "node", "request": "launch", "program": "${workspaceFolder}/index.js" } ] }"#,
        )
        .unwrap();

        let loaded = load_launch_json(&root).unwrap();
        assert_eq!(loaded.configurations.len(), 1);
        assert_eq!(loaded.configurations[0].name, "Run");

        let _ = std::fs::remove_dir_all(&root);
    }
}
