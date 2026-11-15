//! `rustcode-protocols` — Layer 5: LSP, DAP, and Git SCM Protocol Definitions.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod lsp {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum DiagnosticSeverity {
        Error = 1,
        Warning = 2,
        Information = 3,
        Hint = 4,
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
    }
}
