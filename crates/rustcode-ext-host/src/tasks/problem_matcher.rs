//! Task Problem Matcher registry and parser (`contributes.problemMatchers`, `$tsc`, `$rustc`, `$gcc`, `$eslint-compact`).

use crate::provider::DiagnosticCollectionManager;
use parking_lot::RwLock;
use regex::{Regex, RegexBuilder};
use rustcode_protocols::lsp::{
    DiagnosticCode, DiagnosticSeverity, LspDiagnostic, LspPosition, LspRange,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

const REGEX_SIZE_LIMIT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProblemPattern {
    pub regexp: String,
    #[serde(default)]
    pub file: Option<usize>,
    #[serde(default)]
    pub location: Option<usize>,
    #[serde(default)]
    pub line: Option<usize>,
    #[serde(default)]
    pub column: Option<usize>,
    #[serde(default)]
    pub end_line: Option<usize>,
    #[serde(default)]
    pub end_column: Option<usize>,
    #[serde(default)]
    pub severity: Option<usize>,
    #[serde(default)]
    pub code: Option<usize>,
    #[serde(default)]
    pub message: Option<usize>,
    #[serde(default)]
    pub r#loop: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ProblemPatternSpec {
    Name(String),
    Single(ProblemPattern),
    MultiLine(Vec<ProblemPattern>),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProblemMatcherContribution {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub file_location: Option<serde_json::Value>,
    #[serde(default)]
    pub pattern: Option<ProblemPatternSpec>,
}

#[derive(Clone)]
struct CompiledMatcher {
    name: String,
    owner: String,
    source: String,
    default_severity: DiagnosticSeverity,
    patterns: Vec<(ProblemPattern, Regex)>,
}

#[derive(Clone, Default)]
pub struct ProblemMatcherRegistry {
    matchers: Arc<RwLock<HashMap<String, ProblemMatcherContribution>>>,
}

impl ProblemMatcherRegistry {
    pub fn new() -> Self {
        let reg = Self::default();
        reg.register_builtin_matchers();
        reg
    }

    fn register_builtin_matchers(&self) {
        let builtins = [
            ProblemMatcherContribution {
                name: Some("tsc".to_string()),
                owner: Some("typescript".to_string()),
                source: Some("ts".to_string()),
                severity: Some("error".to_string()),
                pattern: Some(ProblemPatternSpec::Single(ProblemPattern {
                    regexp: r"^([^\s].*)\((\d+),(\d+)\):\s+(error|warning)\s+(TS\d+):\s+(.*)$"
                        .to_string(),
                    file: Some(1),
                    line: Some(2),
                    column: Some(3),
                    severity: Some(4),
                    code: Some(5),
                    message: Some(6),
                    ..Default::default()
                })),
                ..Default::default()
            },
            ProblemMatcherContribution {
                name: Some("tsc-watch".to_string()),
                base: Some("$tsc".to_string()),
                owner: Some("typescript".to_string()),
                source: Some("ts".to_string()),
                severity: Some("error".to_string()),
                ..Default::default()
            },
            ProblemMatcherContribution {
                name: Some("gcc".to_string()),
                owner: Some("cpp".to_string()),
                source: Some("gcc".to_string()),
                severity: Some("error".to_string()),
                pattern: Some(ProblemPatternSpec::Single(ProblemPattern {
                    regexp: r"^([^:\s][^:]*):(\d+):(\d+):\s+(fatal error|error|warning|note):\s+(.*)$"
                        .to_string(),
                    file: Some(1),
                    line: Some(2),
                    column: Some(3),
                    severity: Some(4),
                    message: Some(5),
                    ..Default::default()
                })),
                ..Default::default()
            },
            ProblemMatcherContribution {
                name: Some("eslint-compact".to_string()),
                owner: Some("eslint".to_string()),
                source: Some("eslint".to_string()),
                severity: Some("warning".to_string()),
                pattern: Some(ProblemPatternSpec::Single(ProblemPattern {
                    regexp: r"^(.+):\s+line\s+(\d+),\s+col\s+(\d+),\s+(Error|Warning)\s+-\s+(.*)$"
                        .to_string(),
                    file: Some(1),
                    line: Some(2),
                    column: Some(3),
                    severity: Some(4),
                    message: Some(5),
                    ..Default::default()
                })),
                ..Default::default()
            },
            ProblemMatcherContribution {
                name: Some("rustc".to_string()),
                owner: Some("rustc".to_string()),
                source: Some("rustc".to_string()),
                severity: Some("error".to_string()),
                pattern: Some(ProblemPatternSpec::MultiLine(vec![
                    ProblemPattern {
                        regexp: r"^(error|warning|note)(?:\[([A-Z]\d+)\])?:\s+(.*)$".to_string(),
                        severity: Some(1),
                        code: Some(2),
                        message: Some(3),
                        ..Default::default()
                    },
                    ProblemPattern {
                        regexp: r"^\s*-->\s+(.+):(\d+):(\d+)$".to_string(),
                        file: Some(1),
                        line: Some(2),
                        column: Some(3),
                        ..Default::default()
                    },
                ])),
                ..Default::default()
            },
        ];

        for m in builtins {
            let _ = self.register(m);
        }
    }

    pub fn normalize_name(name: &str) -> String {
        name.trim().trim_start_matches('$').to_string()
    }

    pub fn register(&self, matcher: ProblemMatcherContribution) -> Result<String, String> {
        let name = matcher
            .name
            .as_deref()
            .or(matcher.base.as_deref())
            .map(Self::normalize_name)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| "ProblemMatcher must specify a non-empty name".to_string())?;
        // Validate regex compilation eagerly with bounded compiled size.
        let _ = self.compile_contribution(&name, &matcher)?;
        self.matchers.write().insert(name.clone(), matcher);
        Ok(name)
    }

    pub fn list_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.matchers.read().keys().cloned().collect();
        names.sort();
        names
    }

    pub fn get(&self, name: &str) -> Option<ProblemMatcherContribution> {
        self.matchers.read().get(&Self::normalize_name(name)).cloned()
    }

    fn resolve_contribution(&self, name: &str) -> Result<ProblemMatcherContribution, String> {
        let key = Self::normalize_name(name);
        let map = self.matchers.read();
        let entry = map
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("Unknown problem matcher '{}'", name))?;
        if let Some(base_name) = &entry.base {
            let base_key = Self::normalize_name(base_name);
            if let Some(base) = map.get(&base_key) {
                return Ok(ProblemMatcherContribution {
                    name: entry.name.or_else(|| base.name.clone()),
                    base: entry.base,
                    owner: entry.owner.or_else(|| base.owner.clone()),
                    source: entry.source.or_else(|| base.source.clone()),
                    severity: entry.severity.or_else(|| base.severity.clone()),
                    file_location: entry.file_location.or_else(|| base.file_location.clone()),
                    pattern: entry.pattern.or_else(|| base.pattern.clone()),
                });
            }
        }
        Ok(entry)
    }

    fn compile_contribution(
        &self,
        name: &str,
        contrib: &ProblemMatcherContribution,
    ) -> Result<CompiledMatcher, String> {
        let raw_patterns = match &contrib.pattern {
            Some(ProblemPatternSpec::Single(p)) => vec![p.clone()],
            Some(ProblemPatternSpec::MultiLine(list)) => list.clone(),
            Some(ProblemPatternSpec::Name(ref_name)) => {
                let ref_contrib = self.resolve_contribution(ref_name)?;
                match ref_contrib.pattern {
                    Some(ProblemPatternSpec::Single(p)) => vec![p],
                    Some(ProblemPatternSpec::MultiLine(list)) => list,
                    _ => return Err(format!("Referenced problem pattern '{}' missing", ref_name)),
                }
            }
            None => {
                if let Some(base) = &contrib.base {
                    let base_contrib = self.resolve_contribution(base)?;
                    match base_contrib.pattern {
                        Some(ProblemPatternSpec::Single(p)) => vec![p],
                        Some(ProblemPatternSpec::MultiLine(list)) => list,
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                }
            }
        };

        let mut compiled = Vec::with_capacity(raw_patterns.len());
        for pat in raw_patterns {
            let re = RegexBuilder::new(&pat.regexp)
                .size_limit(REGEX_SIZE_LIMIT_BYTES)
                .build()
                .map_err(|e| format!("Invalid problem matcher regex in '{}': {}", name, e))?;
            compiled.push((pat, re));
        }

        Ok(CompiledMatcher {
            name: Self::normalize_name(name),
            owner: contrib
                .owner
                .clone()
                .unwrap_or_else(|| Self::normalize_name(name)),
            source: contrib
                .source
                .clone()
                .unwrap_or_else(|| Self::normalize_name(name)),
            default_severity: parse_severity_str(contrib.severity.as_deref()),
            patterns: compiled,
        })
    }

    pub fn parse_output(
        &self,
        matcher_name: &str,
        workspace_root: Option<&Path>,
        output: &str,
    ) -> Result<(String, HashMap<String, Vec<LspDiagnostic>>), String> {
        let contrib = self.resolve_contribution(matcher_name)?;
        let compiled = self.compile_contribution(matcher_name, &contrib)?;
        let mut by_uri: HashMap<String, Vec<LspDiagnostic>> = HashMap::new();
        if compiled.patterns.is_empty() {
            return Ok((compiled.owner, by_uri));
        }

        let lines: Vec<&str> = output.lines().collect();
        let pat_count = compiled.patterns.len();
        let mut idx = 0usize;

        while idx < lines.len() {
            let mut acc_file: Option<String> = None;
            let mut acc_line: u32 = 1;
            let mut acc_col: u32 = 1;
            let mut acc_end_line: Option<u32> = None;
            let mut acc_end_col: Option<u32> = None;
            let mut acc_severity: DiagnosticSeverity = compiled.default_severity;
            let mut acc_code: Option<String> = None;
            let mut acc_msg: Option<String> = None;

            let mut matched_all = true;
            for offset in 0..pat_count {
                if idx + offset >= lines.len() {
                    matched_all = false;
                    break;
                }
                let (pat, re) = &compiled.patterns[offset];
                let line_str = lines[idx + offset].trim_end_matches('\r');
                let Some(caps) = re.captures(line_str) else {
                    matched_all = false;
                    break;
                };
                if let Some(g) = pat.file.and_then(|i| caps.get(i)) {
                    acc_file = Some(g.as_str().trim().to_string());
                }
                if let Some(g) = pat.line.and_then(|i| caps.get(i)) {
                    acc_line = g.as_str().parse::<u32>().unwrap_or(1).max(1);
                }
                if let Some(g) = pat.column.and_then(|i| caps.get(i)) {
                    acc_col = g.as_str().parse::<u32>().unwrap_or(1).max(1);
                }
                if let Some(g) = pat.end_line.and_then(|i| caps.get(i)) {
                    acc_end_line = g.as_str().parse::<u32>().ok().map(|v| v.max(1));
                }
                if let Some(g) = pat.end_column.and_then(|i| caps.get(i)) {
                    acc_end_col = g.as_str().parse::<u32>().ok().map(|v| v.max(1));
                }
                if let Some(g) = pat.severity.and_then(|i| caps.get(i)) {
                    acc_severity = parse_severity_str(Some(g.as_str()));
                }
                if let Some(g) = pat.code.and_then(|i| caps.get(i)) {
                    let c = g.as_str().trim();
                    if !c.is_empty() {
                        acc_code = Some(c.to_string());
                    }
                }
                if let Some(g) = pat.message.and_then(|i| caps.get(i)) {
                    acc_msg = Some(g.as_str().trim().to_string());
                }
            }

            if matched_all {
                if let (Some(file_str), Some(message)) = (acc_file, acc_msg) {
                    let uri = resolve_problem_file_uri(&file_str, workspace_root);
                    let start = LspPosition::new(acc_line.saturating_sub(1), acc_col.saturating_sub(1));
                    let end = LspPosition::new(
                        acc_end_line.unwrap_or(acc_line).saturating_sub(1),
                        acc_end_col.unwrap_or(acc_col).saturating_sub(1),
                    );
                    by_uri.entry(uri).or_default().push(LspDiagnostic {
                        range: LspRange::new(start, end),
                        severity: Some(acc_severity),
                        message: message.clone(),
                        source: Some(compiled.source.clone()),
                        code: acc_code.map(DiagnosticCode::String),
                    });
                }
                idx += pat_count;
            } else {
                idx += 1;
            }
        }

        let _ = &compiled.name;
        Ok((compiled.owner, by_uri))
    }

    pub fn apply_task_output_to_diagnostics(
        &self,
        matcher_name: &str,
        workspace_root: Option<&Path>,
        output: &str,
        diagnostics: &DiagnosticCollectionManager,
    ) -> Result<usize, String> {
        let (owner, by_uri) = self.parse_output(matcher_name, workspace_root, output)?;
        let mut total = 0usize;
        for (uri, diags) in by_uri {
            total += diags.len();
            diagnostics.set_diagnostics(&owner, &uri, diags);
        }
        Ok(total)
    }
}

fn parse_severity_str(raw: Option<&str>) -> DiagnosticSeverity {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("warning" | "warn") => DiagnosticSeverity::Warning,
        Some("info" | "note" | "information") => DiagnosticSeverity::Information,
        Some("hint") => DiagnosticSeverity::Hint,
        _ => DiagnosticSeverity::Error,
    }
}

fn resolve_problem_file_uri(file_str: &str, workspace_root: Option<&Path>) -> String {
    if rustcode_platform::is_file_uri(file_str) {
        return file_str.to_string();
    }
    let p = Path::new(file_str);
    let resolved = if p.is_absolute() {
        p.to_path_buf()
    } else if let Some(root) = workspace_root {
        root.join(p)
    } else {
        p.to_path_buf()
    };
    rustcode_platform::path_to_file_uri(&resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_problem_matchers_feed_diagnostics() {
        let reg = ProblemMatcherRegistry::new();
        let diags = DiagnosticCollectionManager::new();
        let ws = Path::new("/repo");

        let tsc_output = "\
src/index.ts(12,5): error TS2322: Type 'string' is not assignable to type 'number'.\n\
src/util.ts(4,1): warning TS6133: 'x' is declared but its value is never read.\n";
        let count_tsc = reg
            .apply_task_output_to_diagnostics("$tsc", Some(ws), tsc_output, &diags)
            .unwrap();
        assert_eq!(count_tsc, 2);
        let idx_diags = diags.get_diagnostics_for_uri(&rustcode_platform::path_to_file_uri(
            &ws.join("src/index.ts"),
        ));
        assert_eq!(idx_diags.len(), 1);
        assert_eq!(idx_diags[0].range.start.line, 11);
        assert_eq!(idx_diags[0].range.start.character, 4);
        assert_eq!(idx_diags[0].severity, Some(DiagnosticSeverity::Error));
        assert_eq!(
            idx_diags[0].code,
            Some(DiagnosticCode::String("TS2322".into()))
        );

        let rustc_output = "\
error[E0308]: mismatched types\n \
 --> src/main.rs:8:12\n";
        let count_rustc = reg
            .apply_task_output_to_diagnostics("$rustc", Some(ws), rustc_output, &diags)
            .unwrap();
        assert_eq!(count_rustc, 1);
        let main_diags = diags.get_diagnostics_for_uri(&rustcode_platform::path_to_file_uri(
            &ws.join("src/main.rs"),
        ));
        assert_eq!(main_diags.len(), 1);
        assert_eq!(main_diags[0].range.start.line, 7);
        assert_eq!(main_diags[0].range.start.character, 11);
    }
}
