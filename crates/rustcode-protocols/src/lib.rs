//! `rustcode-protocols` — Layer 5: LSP, DAP, and Git SCM Protocol Definitions.

pub mod codec;
pub mod dap;
pub mod git;
pub mod lsp;

pub use codec::{decode_framed_message, encode_framed_message, CodecError};
pub use dap::{
    Breakpoint, Capabilities, ContinuedEventBody, DapClient, DapError, DapEventListener,
    DapMessage, InitializeRequestArguments, OutputEventBody, Scope, Source, SourceBreakpoint,
    StackFrame, StoppedEventBody, Thread, Variable,
};
pub use git::{
    BranchInfo, CommitInfo, GitStatusEntry, GitStatusKind, ScmRepository,
};

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
            code: None,
        };

        let json = serde_json::to_string(&diag).unwrap();
        let decoded: LspDiagnostic = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.message, "mismatched types");

        let entry = GitStatusEntry {
            path: std::path::PathBuf::from("src/main.rs"),
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
