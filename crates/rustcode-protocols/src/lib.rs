//! `rustcode-protocols` — Layer 5: LSP, DAP, and Git SCM Protocol Definitions.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod lsp {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
    pub enum DiagnosticSeverity {
        Error = 1,
        Warning = 2,
        Information = 3,
        Hint = 4,
    }

    impl<'de> serde::Deserialize<'de> for DiagnosticSeverity {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            struct SeverityVisitor;
            impl<'de> serde::de::Visitor<'de> for SeverityVisitor {
                type Value = DiagnosticSeverity;

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("an integer severity (0-4) or severity name")
                }

                fn visit_u64<E>(self, value: u64) -> Result<DiagnosticSeverity, E>
                where
                    E: serde::de::Error,
                {
                    match value {
                        0 | 1 => Ok(DiagnosticSeverity::Error),
                        2 => Ok(DiagnosticSeverity::Warning),
                        3 => Ok(DiagnosticSeverity::Information),
                        4 => Ok(DiagnosticSeverity::Hint),
                        _ => Err(E::custom(format!("invalid diagnostic severity: {}", value))),
                    }
                }

                fn visit_i64<E>(self, value: i64) -> Result<DiagnosticSeverity, E>
                where
                    E: serde::de::Error,
                {
                    self.visit_u64(value as u64)
                }

                fn visit_str<E>(self, value: &str) -> Result<DiagnosticSeverity, E>
                where
                    E: serde::de::Error,
                {
                    match value.to_ascii_lowercase().as_str() {
                        "error" => Ok(DiagnosticSeverity::Error),
                        "warning" => Ok(DiagnosticSeverity::Warning),
                        "information" | "info" => Ok(DiagnosticSeverity::Information),
                        "hint" => Ok(DiagnosticSeverity::Hint),
                        _ => Err(E::custom(format!("invalid diagnostic severity string: {}", value))),
                    }
                }
            }

            deserializer.deserialize_any(SeverityVisitor)
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct LspPosition {
        pub line: u32,
        pub character: u32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct LspRange {
        pub start: LspPosition,
        pub end: LspPosition,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct LspDiagnostic {
        pub range: LspRange,
        pub severity: Option<DiagnosticSeverity>,
        pub message: String,
        pub source: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct LspCompletionItem {
        pub label: String,
        pub detail: Option<String>,
        pub documentation: Option<String>,
        pub insert_text: Option<String>,
    }
}

pub mod git {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum GitStatusKind {
        Untracked,
        Modified,
        Added,
        Deleted,
        Renamed,
        Conflicted,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GitStatusEntry {
        pub path: PathBuf,
        pub status: GitStatusKind,
        pub is_staged: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BranchInfo {
        pub name: String,
        pub is_current: bool,
        pub upstream: Option<String>,
    }
}

pub mod dap {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SourceBreakpoint {
        pub line: u32,
        pub column: Option<u32>,
        pub condition: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct StackFrame {
        pub id: u32,
        pub name: String,
        pub line: u32,
        pub column: u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp::*;
    use git::*;

    #[test]
    fn test_lsp_and_git_protocols() {
        let diag = LspDiagnostic {
            range: LspRange {
                start: LspPosition { line: 10, character: 4 },
                end: LspPosition { line: 10, character: 12 },
            },
            severity: Some(DiagnosticSeverity::Error),
            message: "mismatched types".to_string(),
            source: Some("rust-analyzer".to_string()),
        };

        let json = serde_json::to_string(&diag).unwrap();
        let decoded: LspDiagnostic = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.message, "mismatched types");

        let entry = GitStatusEntry {
            path: PathBuf::from("src/main.rs"),
            status: GitStatusKind::Modified,
            is_staged: true,
        };
        assert!(entry.is_staged);

        // Test VS Code API 0-indexed severity and string severity deserialization
        let vscode_sev_json = r#"{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"severity":0,"message":"err"}"#;
        let d0: LspDiagnostic = serde_json::from_str(vscode_sev_json).unwrap();
        assert_eq!(d0.severity, Some(DiagnosticSeverity::Error));

        let str_sev_json = r#"{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"severity":"warning","message":"warn"}"#;
        let d_str: LspDiagnostic = serde_json::from_str(str_sev_json).unwrap();
        assert_eq!(d_str.severity, Some(DiagnosticSeverity::Warning));
    }
}
