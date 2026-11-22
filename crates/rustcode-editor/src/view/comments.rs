//! Language-aware line and block comment toggling for `EditorView`.

use crate::model::piece_table::PieceTable;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentConfig {
    pub line_comment: Option<String>,
    pub block_comment: Option<(String, String)>,
}

impl CommentConfig {
    pub fn line(prefix: &str) -> Self {
        Self {
            line_comment: Some(prefix.to_string()),
            block_comment: None,
        }
    }

    pub fn line_and_block(prefix: &str, open: &str, close: &str) -> Self {
        Self {
            line_comment: Some(prefix.to_string()),
            block_comment: Some((open.to_string(), close.to_string())),
        }
    }

    pub fn block_only(open: &str, close: &str) -> Self {
        Self {
            line_comment: None,
            block_comment: Some((open.to_string(), close.to_string())),
        }
    }
}

pub fn comment_config_for_language(lang_id: &str) -> CommentConfig {
    match lang_id.trim().to_ascii_lowercase().as_str() {
        "rust" | "rs" | "javascript" | "js" | "jsx" | "javascriptreact" | "typescript" | "ts"
        | "tsx" | "typescriptreact" | "go" | "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "json"
        | "jsonc" | "java" | "csharp" | "cs" | "swift" | "kotlin" | "scala" | "dart" | "scss"
        | "less" => CommentConfig::line_and_block("//", "/*", "*/"),
        "python" | "py" | "toml" | "yaml" | "yml" | "shellscript" | "sh" | "bash" | "zsh"
        | "ruby" | "rb" | "perl" | "r" | "dockerfile" | "makefile" => CommentConfig::line("#"),
        "sql" | "lua" | "haskell" | "hs" => CommentConfig::line("--"),
        "html" | "htm" | "xml" | "svg" | "markdown" | "md" => {
            CommentConfig::block_only("<!--", "-->")
        }
        "css" => CommentConfig::block_only("/*", "*/"),
        _ => CommentConfig::line("//"),
    }
}

/// Compute atomic byte-offset edits `(start, end, replacement)` to toggle comments
/// across `start_line..=end_line`.
pub fn compute_toggle_line_comment_edits(
    buf: &PieceTable,
    start_line: usize,
    end_line: usize,
    config: &CommentConfig,
) -> Vec<(usize, usize, String)> {
    let line_count = buf.line_count();
    if line_count == 0 || start_line >= line_count {
        return Vec::new();
    }
    let clamped_end = end_line.min(line_count - 1).max(start_line);

    let mut lines = Vec::with_capacity(clamped_end - start_line + 1);
    for line_idx in start_line..=clamped_end {
        let Some((line_start, _)) = buf.get_line_offsets(line_idx) else {
            continue;
        };
        let content = buf.get_line_content(line_idx).unwrap_or_default();
        lines.push((line_idx, line_start, content));
    }

    let non_empty: Vec<&(usize, usize, String)> =
        lines.iter().filter(|(_, _, s)| !s.trim().is_empty()).collect();

    if let Some(prefix) = &config.line_comment {
        if non_empty.is_empty() {
            if let Some((_, line_start, content)) = lines.first() {
                let off = line_start + content.len();
                return vec![(off, off, format!("{prefix} "))];
            }
            return Vec::new();
        }

        let all_commented = non_empty
            .iter()
            .all(|(_, _, s)| s.trim_start().starts_with(prefix.as_str()));

        if all_commented {
            let mut edits = Vec::with_capacity(non_empty.len());
            for &(_, line_start, s) in &non_empty {
                let indent = s.len() - s.trim_start().len();
                let after_prefix = indent + prefix.len();
                let del_end = if s[after_prefix..].starts_with(' ') {
                    after_prefix + 1
                } else {
                    after_prefix
                };
                edits.push((line_start + indent, line_start + del_end, String::new()));
            }
            edits
        } else {
            let common_indent = non_empty
                .iter()
                .map(|(_, _, s)| s.len() - s.trim_start().len())
                .min()
                .unwrap_or(0);
            non_empty
                .iter()
                .map(|&(_, line_start, _)| {
                    let off = line_start + common_indent;
                    (off, off, format!("{prefix} "))
                })
                .collect()
        }
    } else if let Some((open, close)) = &config.block_comment {
        if non_empty.is_empty() {
            if let Some((_, line_start, content)) = lines.first() {
                let off = line_start + content.len();
                return vec![(off, off, format!("{open}  {close}"))];
            }
            return Vec::new();
        }

        let all_commented = non_empty.iter().all(|(_, _, s)| {
            let trimmed = s.trim();
            trimmed.starts_with(open.as_str())
                && trimmed.ends_with(close.as_str())
                && trimmed.len() >= open.len() + close.len()
        });

        let mut edits = Vec::new();
        if all_commented {
            for &(_, line_start, s) in &non_empty {
                let indent = s.len() - s.trim_start().len();
                let open_end = indent + open.len();
                let open_del_end = if s[open_end..].starts_with(' ') {
                    open_end + 1
                } else {
                    open_end
                };
                let trimmed_end = s.trim_end().len();
                let close_start = trimmed_end - close.len();
                let close_del_start = if close_start > open_del_end && s[..close_start].ends_with(' ')
                {
                    close_start - 1
                } else {
                    close_start
                };
                edits.push((line_start + indent, line_start + open_del_end, String::new()));
                if close_del_start >= open_del_end {
                    edits.push((
                        line_start + close_del_start,
                        line_start + trimmed_end,
                        String::new(),
                    ));
                }
            }
        } else {
            let common_indent = non_empty
                .iter()
                .map(|(_, _, s)| s.len() - s.trim_start().len())
                .min()
                .unwrap_or(0);
            for &(_, line_start, s) in &non_empty {
                let trimmed_end = s.trim_end().len();
                edits.push((
                    line_start + common_indent,
                    line_start + common_indent,
                    format!("{open} "),
                ));
                edits.push((
                    line_start + trimmed_end,
                    line_start + trimmed_end,
                    format!(" {close}"),
                ));
            }
        }
        edits
    } else {
        Vec::new()
    }
}
