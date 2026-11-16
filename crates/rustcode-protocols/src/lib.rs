//! `rustcode-protocols` — Layer 5: LSP, DAP, and Git SCM Protocol Definitions.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod lsp;

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
    use git::*;
    use lsp::*;

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
