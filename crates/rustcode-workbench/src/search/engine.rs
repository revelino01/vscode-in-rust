//! Fast in-process cross-folder search and replace engine powered by `ripgrep`
//! (`ignore` + `grep-regex` + `grep-searcher`) and `nucleo-matcher` file ranking.

use crate::workspace::WorkspaceFolder;
use grep_matcher::Matcher as GrepMatcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{sinks::UTF8, BinaryDetection, SearcherBuilder};
use ignore::overrides::{Override, OverrideBuilder};
use ignore::WalkBuilder;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher as NucleoMatcher, Utf32Str};
use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub query: String,
    pub is_regex: bool,
    pub match_case: bool,
    pub match_whole_word: bool,
    pub files_to_include: Vec<String>,
    pub files_to_exclude: Vec<String>,
    pub max_results: usize,
    pub smart_case: bool,
    pub include_hidden: bool,
    pub use_gitignore: bool,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            is_regex: false,
            match_case: false,
            match_whole_word: false,
            files_to_include: Vec::new(),
            files_to_exclude: Vec::new(),
            max_results: 2000,
            smart_case: false,
            include_hidden: false,
            use_gitignore: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSearchMatch {
    pub line_number: usize, // 1-based
    pub start_col: usize,   // 0-based character offset in line
    pub end_col: usize,     // 0-based character offset in line
    pub line_content: String,
}

impl TextSearchMatch {
    /// Returns the 0-based `(start, end)` UTF-8 byte offsets within `line_content`.
    pub fn byte_col_range(&self) -> (usize, usize) {
        let start_byte = char_index_to_byte_offset(&self.line_content, self.start_col);
        let end_byte = char_index_to_byte_offset(&self.line_content, self.end_col);
        (start_byte, end_byte)
    }

    /// Returns the 0-based `(start, end)` UTF-16 code-unit offsets within `line_content`.
    pub fn utf16_col_range(&self) -> (usize, usize) {
        let start_utf16 = char_index_to_utf16_offset(&self.line_content, self.start_col);
        let end_utf16 = char_index_to_utf16_offset(&self.line_content, self.end_col);
        (start_utf16, end_utf16)
    }
}

fn char_index_to_byte_offset(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map_or(s.len(), |(byte_idx, _)| byte_idx)
}

fn char_index_to_utf16_offset(s: &str, char_idx: usize) -> usize {
    s.chars().take(char_idx).map(char::len_utf16).sum()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSearchResult {
    pub folder_name: String,
    pub folder_path: PathBuf,
    pub file_path: PathBuf,
    pub relative_path: PathBuf,
    pub matches: Vec<TextSearchMatch>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchResults {
    pub file_results: Vec<FileSearchResult>,
    pub total_matches: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplaceFileOutcome {
    pub file_path: PathBuf,
    pub replacements: usize,
    pub updated_content: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceReplaceResult {
    pub files_modified: usize,
    pub total_replacements: usize,
    pub modified_files: Vec<ReplaceFileOutcome>,
}

/// Perform in-process text search across all workspace folders.
/// Returns `SearchResults::default()` on empty or invalid regex queries without panicking.
pub fn search_text(folders: &[WorkspaceFolder], query: &SearchQuery) -> SearchResults {
    try_search_text(folders, query).unwrap_or_default()
}

/// Perform in-process text search across all workspace folders, surfacing regex or glob errors.
pub fn try_search_text(
    folders: &[WorkspaceFolder],
    query: &SearchQuery,
) -> Result<SearchResults, String> {
    if query.query.trim().is_empty() || query.max_results == 0 {
        return Ok(SearchResults::default());
    }

    let matcher = build_grep_matcher(query)?;
    let mut results = SearchResults::default();

    for folder in folders {
        if results.total_matches >= query.max_results {
            break;
        }
        walk_and_search_folder(folder, query, &matcher, &mut results)?;
    }

    Ok(results)
}

/// Replace matches across in-memory text (supporting literal mode & regex `$1` capture groups).
pub fn replace_in_text(
    content: &str,
    query: &SearchQuery,
    replacement: &str,
) -> Result<(String, usize), String> {
    if query.query.trim().is_empty() {
        return Ok((content.to_string(), 0));
    }

    let matcher = build_grep_matcher(query)?;
    let capture_regex = if query.is_regex {
        Some(build_capture_regex(query)?)
    } else {
        None
    };

    let mut out = String::with_capacity(content.len());
    let mut total_replacements = 0usize;

    for raw_line in content.split_inclusive('\n') {
        let (body, ending) = if let Some(stripped) = raw_line.strip_suffix("\r\n") {
            (stripped, "\r\n")
        } else if let Some(stripped) = raw_line.strip_suffix('\n') {
            (stripped, "\n")
        } else {
            (raw_line, "")
        };

        if let Some(re) = &capture_regex {
            let mut last_end = 0usize;
            let mut replaced_any = false;
            for caps in re.captures_iter(body) {
                if let Some(whole) = caps.get(0) {
                    replaced_any = true;
                    out.push_str(&body[last_end..whole.start()]);
                    caps.expand(replacement, &mut out);
                    last_end = whole.end();
                    total_replacements += 1;
                }
            }
            if replaced_any {
                out.push_str(&body[last_end..]);
            } else {
                out.push_str(body);
            }
        } else {
            let mut ranges = Vec::new();
            let _ = matcher.find_iter(body.as_bytes(), |m| {
                ranges.push((m.start(), m.end()));
                true
            });
            if ranges.is_empty() {
                out.push_str(body);
            } else {
                let mut last_end = 0usize;
                for (start, end) in ranges {
                    out.push_str(&body[last_end..start]);
                    out.push_str(replacement);
                    last_end = end;
                    total_replacements += 1;
                }
                out.push_str(&body[last_end..]);
            }
        }

        out.push_str(ending);
    }

    Ok((out, total_replacements))
}

/// Replace matches across workspace files on disk (supporting literal & regex `$1` capture groups).
pub fn replace_in_workspace(
    folders: &[WorkspaceFolder],
    query: &SearchQuery,
    replacement: &str,
) -> Result<WorkspaceReplaceResult, String> {
    if query.query.trim().is_empty() {
        return Ok(WorkspaceReplaceResult::default());
    }

    // Validate pattern up front before touching any files on disk.
    let _ = build_grep_matcher(query)?;
    if query.is_regex {
        let _ = build_capture_regex(query)?;
    }

    let mut result = WorkspaceReplaceResult::default();

    for folder in folders {
        let files = collect_matching_workspace_files(folder, query)?;
        for path in files {
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            if bytes.contains(&0) {
                continue;
            }
            let Ok(content) = String::from_utf8(bytes) else {
                continue;
            };

            let (updated_content, replacements) =
                replace_in_text(&content, query, replacement)?;
            if replacements > 0 {
                fs::write(&path, updated_content.as_bytes())
                    .map_err(|e| format!("Failed to write '{}': {e}", path.display()))?;
                result.files_modified += 1;
                result.total_replacements += replacements;
                result.modified_files.push(ReplaceFileOutcome {
                    file_path: path,
                    replacements,
                    updated_content,
                });
            }
        }
    }

    Ok(result)
}

/// Perform file search across all workspace folders matching a fuzzy query ranked by `nucleo`.
pub fn search_files(
    folders: &[WorkspaceFolder],
    pattern: &str,
    max_results: usize,
) -> Vec<PathBuf> {
    if max_results == 0 {
        return Vec::new();
    }

    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        let mut files = Vec::new();
        for folder in folders {
            if files.len() >= max_results {
                break;
            }
            for entry in build_workspace_walker(&folder.path, false, true, None)
                .build()
                .flatten()
            {
                if files.len() >= max_results {
                    break;
                }
                if entry.file_type().is_some_and(|ft| ft.is_file()) {
                    files.push(entry.into_path());
                }
            }
        }
        return files;
    }

    let mut nucleo = NucleoMatcher::new(Config::DEFAULT.match_paths());
    let pat = parse_nucleo_pattern(trimmed);
    let mut char_buf = Vec::new();
    let mut indices = Vec::new();
    let mut scored: Vec<(u32, String, PathBuf)> = Vec::new();

    for folder in folders {
        for entry in build_workspace_walker(&folder.path, false, true, None)
            .build()
            .flatten()
        {
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let path = entry.into_path();
            let rel_str = path
                .strip_prefix(&folder.path)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let name_score = score_nucleo_match(
                &pat,
                trimmed,
                &file_name,
                &mut nucleo,
                &mut char_buf,
                &mut indices,
            );
            let rel_score = score_nucleo_match(
                &pat,
                trimmed,
                &rel_str,
                &mut nucleo,
                &mut char_buf,
                &mut indices,
            );
            let best_score = match (name_score, rel_score) {
                (Some(ns), Some(rs)) => Some(ns.saturating_add(16).max(rs)),
                (Some(ns), None) => Some(ns.saturating_add(16)),
                (None, Some(rs)) => Some(rs),
                (None, None) => None,
            };

            if let Some(score) = best_score {
                scored.push((score, rel_str, path));
            }
        }
    }

    scored.sort_by(|a, b| {
        Reverse(a.0)
            .cmp(&Reverse(b.0))
            .then_with(|| a.1.len().cmp(&b.1.len()))
            .then_with(|| a.1.cmp(&b.1))
    });
    scored
        .into_iter()
        .take(max_results)
        .map(|(_, _, p)| p)
        .collect()
}

pub(crate) fn parse_nucleo_pattern(query: &str) -> Pattern {
    let parsed = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    if parsed.atoms.is_empty() {
        Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        )
    } else {
        parsed
    }
}

/// Computes a `nucleo-matcher` score adjusted with VS Code-style contiguity,
/// boundary, and prefix weighting so contiguous / camelCase matches outrank
/// scattered underscore-separated matches.
pub(crate) fn score_nucleo_match(
    pattern: &Pattern,
    query: &str,
    haystack: &str,
    matcher: &mut NucleoMatcher,
    char_buf: &mut Vec<char>,
    indices: &mut Vec<u32>,
) -> Option<u32> {
    indices.clear();
    let base_score = pattern.indices(Utf32Str::new(haystack, char_buf), matcher, indices)?;
    let mut score = base_score as i64;

    let q_trimmed = query.trim();
    if !q_trimmed.is_empty() {
        let hay_lower = haystack.to_lowercase();
        let q_lower = q_trimmed.to_lowercase();
        if hay_lower == q_lower {
            score += 240;
        } else if hay_lower.starts_with(&q_lower) {
            score += 140;
        } else if let Some(byte_pos) = hay_lower.find(&q_lower) {
            score += 80;
            if byte_pos > 0 {
                let prev_is_sep = hay_lower[..byte_pos]
                    .chars()
                    .next_back()
                    .is_some_and(|c| matches!(c, '_' | '-' | '/' | '\\' | '.' | ' ' | ':'));
                let char_pos = hay_lower[..byte_pos].chars().count();
                let is_camel = haystack
                    .chars()
                    .nth(char_pos)
                    .is_some_and(char::is_uppercase);
                if prev_is_sep || is_camel {
                    score += 24;
                }
            }
        }
    }

    if indices.len() >= 2 {
        indices.sort_unstable();
        indices.dedup();
        let mut gaps = 0i64;
        let mut skipped = 0i64;
        for w in indices.windows(2) {
            let diff = w[1].saturating_sub(w[0]);
            if diff > 1 {
                gaps += 1;
                skipped += (diff - 1) as i64;
            }
        }
        if gaps == 0 {
            score += 30;
        } else {
            score -= gaps * 20 + skipped.min(64) * 2;
        }
    } else if indices.len() == 1 {
        score += 10;
    }

    Some(score.max(1) as u32)
}

fn build_grep_matcher(query: &SearchQuery) -> Result<grep_regex::RegexMatcher, String> {
    let mut builder = RegexMatcherBuilder::new();
    builder
        .fixed_strings(!query.is_regex)
        .word(query.match_whole_word)
        .multi_line(false);
    if query.smart_case && !query.match_case {
        builder.case_smart(true);
    } else {
        builder.case_insensitive(!query.match_case);
    }
    builder
        .build(&query.query)
        .map_err(|e| format!("Invalid search pattern: {e}"))
}

fn build_capture_regex(query: &SearchQuery) -> Result<regex::Regex, String> {
    let pattern_body = if query.is_regex {
        query.query.clone()
    } else {
        regex::escape(&query.query)
    };
    let full_pattern = if query.match_whole_word {
        format!(r"\b(?:{pattern_body})\b")
    } else {
        pattern_body
    };
    let case_insensitive = if query.smart_case && !query.match_case {
        !query.query.chars().any(char::is_uppercase)
    } else {
        !query.match_case
    };

    regex::RegexBuilder::new(&full_pattern)
        .case_insensitive(case_insensitive)
        .multi_line(false)
        .build()
        .map_err(|e| format!("Invalid replace regex: {e}"))
}

pub fn is_ignored_directory(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "target"
            | ".cargo"
            | "dist"
            | "build"
            | ".svn"
            | ".hg"
            | ".idea"
            | ".vscode"
    )
}

fn normalize_override_glob(raw: &str) -> Option<String> {
    let p = raw.trim().replace('\\', "/");
    if p.is_empty() {
        return None;
    }
    let body = p.strip_prefix('!').unwrap_or(&p);
    if body.is_empty() {
        return None;
    }
    if body.starts_with('/') || body.starts_with("**/") || !body.contains('/') {
        Some(body.to_string())
    } else {
        Some(format!("**/{body}"))
    }
}

fn build_exclude_overrides(
    root: &Path,
    excludes: &[String],
) -> Result<(Option<Override>, Option<Override>), String> {
    let mut has_walk_prune = false;
    let mut walk_builder = OverrideBuilder::new(root);
    let mut has_file_exclude = false;
    let mut file_builder = OverrideBuilder::new(root);

    for raw in excludes {
        if let Some(norm) = normalize_override_glob(raw) {
            let negated = format!("!{norm}");
            file_builder
                .add(&negated)
                .map_err(|e| format!("Invalid exclude glob '{raw}': {e}"))?;
            has_file_exclude = true;

            // Only push whole-subtree excludes (`/**` or `/`) into WalkBuilder overrides so
            // single-level excludes like `handlers/*` do not prune subdirectories (`handlers/sub/`).
            if norm.ends_with("/**") || norm.ends_with('/') {
                walk_builder
                    .add(&negated)
                    .map_err(|e| format!("Invalid exclude glob '{raw}': {e}"))?;
                has_walk_prune = true;
            }
        }
    }

    let walk_override = if has_walk_prune {
        Some(
            walk_builder
                .build()
                .map_err(|e| format!("Failed to build walk exclude overrides: {e}"))?,
        )
    } else {
        None
    };
    let file_override = if has_file_exclude {
        Some(
            file_builder
                .build()
                .map_err(|e| format!("Failed to build file exclude overrides: {e}"))?,
        )
    } else {
        None
    };

    Ok((walk_override, file_override))
}

fn build_include_overrides(root: &Path, includes: &[String]) -> Result<Option<Override>, String> {
    let mut patterns = Vec::new();
    for raw in includes {
        let trimmed = raw.trim().replace('\\', "/");
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "*" || trimmed == "**" {
            return Ok(None);
        }
        if let Some(norm) = normalize_override_glob(&trimmed) {
            patterns.push((raw.as_str(), norm));
        }
    }
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = OverrideBuilder::new(root);
    for (raw, norm) in patterns {
        builder
            .add(&norm)
            .map_err(|e| format!("Invalid include glob '{raw}': {e}"))?;
    }
    builder
        .build()
        .map(Some)
        .map_err(|e| format!("Failed to build include overrides: {e}"))
}

fn build_workspace_walker(
    root: &Path,
    include_hidden: bool,
    use_gitignore: bool,
    exclude_overrides: Option<Override>,
) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!include_hidden)
        .git_ignore(use_gitignore)
        .git_global(false)
        .git_exclude(use_gitignore)
        .require_git(false)
        .parents(false);
    if let Some(overrides) = exclude_overrides {
        builder.overrides(overrides);
    }
    builder
}

fn collect_matching_workspace_files(
    folder: &WorkspaceFolder,
    query: &SearchQuery,
) -> Result<Vec<PathBuf>, String> {
    let (walk_exclude_overrides, file_exclude_overrides) =
        build_exclude_overrides(&folder.path, &query.files_to_exclude)?;
    let include_overrides = build_include_overrides(&folder.path, &query.files_to_include)?;

    let walker = build_workspace_walker(
        &folder.path,
        query.include_hidden,
        query.use_gitignore,
        walk_exclude_overrides,
    )
    .build();

    let mut files = Vec::new();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.into_path();
        let rel = path.strip_prefix(&folder.path).unwrap_or(&path);
        let rel_norm = rel.to_string_lossy().replace('\\', "/");
        if let Some(exc) = &file_exclude_overrides {
            if exc.matched(Path::new(&rel_norm), false).is_ignore() {
                continue;
            }
        }
        if let Some(inc) = &include_overrides {
            if !inc.matched(Path::new(&rel_norm), false).is_whitelist() {
                continue;
            }
        }
        files.push(path);
    }
    Ok(files)
}

fn walk_and_search_folder(
    folder: &WorkspaceFolder,
    query: &SearchQuery,
    matcher: &grep_regex::RegexMatcher,
    results: &mut SearchResults,
) -> Result<(), String> {
    let files = collect_matching_workspace_files(folder, query)?;
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .line_number(true)
        .build();

    for path in files {
        if results.total_matches >= query.max_results {
            break;
        }

        let mut file_matches = Vec::new();
        let current_total = results.total_matches;
        let max_results = query.max_results;

        let search_res = searcher.search_path(
            matcher,
            &path,
            UTF8(|line_number, line| {
                let clean_line = line.trim_end_matches(&['\r', '\n'][..]);
                let mut stop = false;
                let _ = matcher.find_iter(clean_line.as_bytes(), |m| {
                    let prefix = clean_line.get(..m.start()).unwrap_or("");
                    let matched = clean_line.get(m.start()..m.end()).unwrap_or("");
                    let start_col = prefix.chars().count();
                    let end_col = start_col + matched.chars().count();

                    file_matches.push(TextSearchMatch {
                        line_number: line_number as usize,
                        start_col,
                        end_col,
                        line_content: clean_line.to_string(),
                    });

                    if current_total + file_matches.len() >= max_results {
                        stop = true;
                        false
                    } else {
                        true
                    }
                });
                Ok(!stop)
            }),
        );

        // Skip unreadable or invalid-UTF-8 files cleanly without aborting the workspace search.
        if search_res.is_err() {
            continue;
        }

        if !file_matches.is_empty() {
            results.total_matches += file_matches.len();
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let relative_path = folder
                .relative_path(&path)
                .unwrap_or_else(|| PathBuf::from(&file_name));

            results.file_results.push(FileSearchResult {
                folder_name: folder.name.clone(),
                folder_path: folder.path.clone(),
                file_path: path,
                relative_path,
                matches: file_matches,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
fn matches_glob(path_str: &str, pattern: &str) -> bool {
    let p = pattern.trim().replace('\\', "/");
    let path_str = path_str.replace('\\', "/");
    if p.is_empty() || p == "*" || p == "**" {
        return true;
    }

    if let Some(ext) = p.strip_prefix("*.") {
        return path_str.ends_with(&format!(".{ext}"));
    }

    if p.contains('*') {
        let (prefix, pattern_body) = if p.starts_with('/') {
            ("^", p.as_str())
        } else if let Some(rest) = p.strip_prefix("**/") {
            (r"^(?:^|.*/)", rest)
        } else {
            (r"^(?:^|.*/)", p.as_str())
        };

        let body = regex::escape(pattern_body)
            .replace(r"/\*\*/", r"/(?:.*/)?")
            .replace(r"\*\*", ".*")
            .replace(r"\*", "[^/]*");

        let regex_pat = format!("{prefix}{body}$");
        if let Ok(re) = regex::Regex::new(&regex_pat) {
            return re.is_match(&path_str);
        }
    }

    path_str.contains(&p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_engine_basic_and_options() {
        let temp_dir =
            std::env::temp_dir().join(format!("rustcode_search_test_{}", uuid::Uuid::new_v4()));
        let folder_a = temp_dir.join("folder_a");
        let folder_b = temp_dir.join("folder_b");
        fs::create_dir_all(&folder_a).unwrap();
        fs::create_dir_all(&folder_b).unwrap();

        fs::write(
            folder_a.join("alpha.rs"),
            "fn hello_world() {\n    println!(\"Hello World\");\n}\n",
        )
        .unwrap();
        fs::write(
            folder_b.join("beta.js"),
            "function helloWorld() {\n  console.log('hello world');\n}\n",
        )
        .unwrap();

        let folders = vec![
            WorkspaceFolder::new(folder_a.clone(), Some("FolderA".to_string()), 0),
            WorkspaceFolder::new(folder_b.clone(), Some("FolderB".to_string()), 1),
        ];

        // 1. Case-insensitive search across folders
        let query = SearchQuery {
            query: "hello".to_string(),
            match_case: false,
            ..Default::default()
        };
        let results = search_text(&folders, &query);
        assert_eq!(results.file_results.len(), 2);
        assert!(results.total_matches >= 3);

        // 2. Case-sensitive search
        let query_case = SearchQuery {
            query: "Hello".to_string(),
            match_case: true,
            ..Default::default()
        };
        let results_case = search_text(&folders, &query_case);
        assert_eq!(results_case.file_results.len(), 1);
        assert_eq!(results_case.file_results[0].folder_name, "FolderA");

        // 3. Whole word search
        let query_word = SearchQuery {
            query: "hello_world".to_string(),
            match_whole_word: true,
            ..Default::default()
        };
        let results_word = search_text(&folders, &query_word);
        assert_eq!(results_word.file_results.len(), 1);

        // 4. File search
        let matched_files = search_files(&folders, "alpha", 10);
        assert_eq!(matched_files.len(), 1);
        assert!(matched_files[0].ends_with("alpha.rs"));

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_engine_relative_directory_globs() {
        // Direct matches_glob unit tests for relative patterns across directory depths
        assert!(matches_glob("/workspace/project/src/main.rs", "src/**/*.rs"));
        assert!(matches_glob(
            "/workspace/project/src/handlers/auth.rs",
            "src/**/*.rs"
        ));
        assert!(matches_glob(
            "/workspace/project/src/handlers/sub/deep.rs",
            "src/**/*.rs"
        ));
        assert!(!matches_glob(
            "/workspace/project/tests/test_main.rs",
            "src/**/*.rs"
        ));
        assert!(!matches_glob("/workspace/project/src/main.js", "src/**/*.rs"));

        assert!(matches_glob(
            "/workspace/project/src/handlers/auth.rs",
            "handlers/*"
        ));
        assert!(matches_glob(
            "/workspace/project/src/handlers/data.rs",
            "handlers/*"
        ));
        assert!(!matches_glob(
            "/workspace/project/src/handlers/sub/deep.rs",
            "handlers/*"
        ));
        assert!(!matches_glob(
            "/workspace/project/src/main.rs",
            "handlers/*"
        ));

        assert!(matches_glob("/workspace/project/src/main.rs", "src/*"));
        assert!(matches_glob("/workspace/project/src/lib.rs", "src/*"));
        assert!(!matches_glob(
            "/workspace/project/src/handlers/auth.rs",
            "src/*"
        ));

        // Full search_text integration test across nested directory structure
        let temp_dir =
            std::env::temp_dir().join(format!("rustcode_glob_test_{}", uuid::Uuid::new_v4()));
        let src_dir = temp_dir.join("src");
        let handlers_dir = src_dir.join("handlers");
        let sub_dir = handlers_dir.join("sub");
        let tests_dir = temp_dir.join("tests");
        let docs_dir = temp_dir.join("docs");

        fs::create_dir_all(&sub_dir).unwrap();
        fs::create_dir_all(&tests_dir).unwrap();
        fs::create_dir_all(&docs_dir).unwrap();

        let code = "fn target_fn() {\n    println!(\"found\");\n}\n";
        fs::write(src_dir.join("main.rs"), code).unwrap();
        fs::write(src_dir.join("lib.rs"), code).unwrap();
        fs::write(handlers_dir.join("auth.rs"), code).unwrap();
        fs::write(handlers_dir.join("data.rs"), code).unwrap();
        fs::write(sub_dir.join("deep.rs"), code).unwrap();
        fs::write(tests_dir.join("test_main.rs"), code).unwrap();
        fs::write(docs_dir.join("readme.txt"), "target_fn doc reference\n").unwrap();

        let folders = vec![WorkspaceFolder::new(
            temp_dir.clone(),
            Some("Root".to_string()),
            0,
        )];

        // 1. Relative recursive glob: src/**/*.rs -> matches all 5 files in src tree
        let query_recursive = SearchQuery {
            query: "target_fn".to_string(),
            files_to_include: vec!["src/**/*.rs".to_string()],
            ..Default::default()
        };
        let res_recursive = search_text(&folders, &query_recursive);
        assert_eq!(res_recursive.file_results.len(), 5);
        assert_eq!(res_recursive.total_matches, 5);

        // 2. Relative single-directory glob: handlers/* -> matches auth.rs and data.rs only (not sub/deep.rs)
        let query_handlers = SearchQuery {
            query: "target_fn".to_string(),
            files_to_include: vec!["handlers/*".to_string()],
            ..Default::default()
        };
        let res_handlers = search_text(&folders, &query_handlers);
        assert_eq!(res_handlers.file_results.len(), 2);
        assert_eq!(res_handlers.total_matches, 2);

        // 3. Relative direct child glob: src/* -> matches main.rs and lib.rs only
        let query_src_direct = SearchQuery {
            query: "target_fn".to_string(),
            files_to_include: vec!["src/*".to_string()],
            ..Default::default()
        };
        let res_src_direct = search_text(&folders, &query_src_direct);
        assert_eq!(res_src_direct.file_results.len(), 2);
        assert_eq!(res_src_direct.total_matches, 2);

        // 4. Relative directory exclude: exclude handlers/*
        let query_exclude = SearchQuery {
            query: "target_fn".to_string(),
            files_to_include: vec!["src/**/*.rs".to_string()],
            files_to_exclude: vec!["handlers/*".to_string()],
            ..Default::default()
        };
        let res_exclude = search_text(&folders, &query_exclude);
        assert_eq!(res_exclude.file_results.len(), 3); // main.rs, lib.rs, deep.rs
        assert_eq!(res_exclude.total_matches, 3);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_ripgrep_gitignore_hidden_regex_smart_case_and_errors() {
        let temp_dir =
            std::env::temp_dir().join(format!("rustcode_rg_m3_12_{}", uuid::Uuid::new_v4()));
        let src_dir = temp_dir.join("src");
        let target_dir = temp_dir.join("target");
        let hidden_dir = temp_dir.join(".hidden_cfg");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&target_dir).unwrap();
        fs::create_dir_all(&hidden_dir).unwrap();

        fs::write(temp_dir.join(".gitignore"), "ignored.rs\nignored_dir/\n").unwrap();
        fs::create_dir_all(temp_dir.join("ignored_dir")).unwrap();
        fs::write(temp_dir.join("ignored_dir/skip.rs"), "needle_token\n").unwrap();
        fs::write(src_dir.join("ignored.rs"), "needle_token\n").unwrap();
        fs::write(src_dir.join("visible.rs"), "needle_token foo[0].bar() 🚀needle\n").unwrap();
        fs::write(src_dir.join("notes.md"), "Needle_Token\n").unwrap();
        fs::write(target_dir.join("artifact.rs"), "needle_token\n").unwrap();
        fs::write(temp_dir.join(".env"), "needle_token\n").unwrap();
        fs::write(hidden_dir.join("cfg.rs"), "needle_token\n").unwrap();

        let folders = vec![WorkspaceFolder::new(
            temp_dir.clone(),
            Some("Workspace".to_string()),
            0,
        )];

        // 1. Honors .gitignore & hidden files by default, plus files_to_exclude = ["target/**"]
        let q_default = SearchQuery {
            query: "needle_token".to_string(),
            files_to_exclude: vec!["target/**".to_string()],
            ..Default::default()
        };
        let r_default = try_search_text(&folders, &q_default).unwrap();
        assert_eq!(r_default.file_results.len(), 2); // visible.rs and notes.md only
        assert!(r_default
            .file_results
            .iter()
            .all(|f| !f.file_path.ends_with("ignored.rs")
                && !f.file_path.ends_with("skip.rs")
                && !f.file_path.ends_with(".env")));

        // 2. files_to_include = ["*.rs"] narrows to visible.rs while still honoring .gitignore
        let q_include = SearchQuery {
            query: "needle_token".to_string(),
            files_to_include: vec!["*.rs".to_string()],
            files_to_exclude: vec!["target/**".to_string()],
            ..Default::default()
        };
        let r_include = try_search_text(&folders, &q_include).unwrap();
        assert_eq!(r_include.file_results.len(), 1);
        assert!(r_include.file_results[0].file_path.ends_with("visible.rs"));

        // 3. include_hidden = true surfaces .env and .hidden_cfg/cfg.rs
        let q_hidden = SearchQuery {
            query: "needle_token".to_string(),
            include_hidden: true,
            files_to_exclude: vec!["target/**".to_string()],
            ..Default::default()
        };
        let r_hidden = try_search_text(&folders, &q_hidden).unwrap();
        assert_eq!(r_hidden.file_results.len(), 4);

        // 4. Literal mode escapes regex metacharacters like foo[0].bar()
        let q_literal = SearchQuery {
            query: "foo[0].bar()".to_string(),
            is_regex: false,
            files_to_include: vec!["*.rs".to_string()],
            files_to_exclude: vec!["target/**".to_string()],
            ..Default::default()
        };
        let r_literal = try_search_text(&folders, &q_literal).unwrap();
        assert_eq!(r_literal.total_matches, 1);

        // 5. Smart-case: lowercase matches both, uppercase is case-sensitive
        let q_smart_lower = SearchQuery {
            query: "needle_token".to_string(),
            smart_case: true,
            files_to_exclude: vec!["target/**".to_string()],
            ..Default::default()
        };
        assert_eq!(try_search_text(&folders, &q_smart_lower).unwrap().file_results.len(), 2);

        let q_smart_upper = SearchQuery {
            query: "Needle_Token".to_string(),
            smart_case: true,
            files_to_exclude: vec!["target/**".to_string()],
            ..Default::default()
        };
        let r_smart_upper = try_search_text(&folders, &q_smart_upper).unwrap();
        assert_eq!(r_smart_upper.file_results.len(), 1);
        assert!(r_smart_upper.file_results[0].file_path.ends_with("notes.md"));

        // 6. UTF-16 & byte offsets for Unicode matches
        let q_emoji = SearchQuery {
            query: "needle".to_string(),
            match_case: true,
            files_to_include: vec!["*.rs".to_string()],
            files_to_exclude: vec!["target/**".to_string()],
            ..Default::default()
        };
        let r_emoji = try_search_text(&folders, &q_emoji).unwrap();
        let last_match = r_emoji.file_results[0].matches.last().unwrap();
        let (b_start, b_end) = last_match.byte_col_range();
        let (u16_start, u16_end) = last_match.utf16_col_range();
        assert_eq!(&last_match.line_content[b_start..b_end], "needle");
        assert!(u16_start > last_match.start_col); // 🚀 is 1 char (4 UTF-8 bytes, 2 UTF-16 units)
        assert_eq!(u16_end - u16_start, "needle".len());

        // 7. Invalid regex returns Err gracefully without panicking
        let q_bad = SearchQuery {
            query: "[unclosed(".to_string(),
            is_regex: true,
            ..Default::default()
        };
        assert!(try_search_text(&folders, &q_bad).is_err());
        assert_eq!(search_text(&folders, &q_bad).total_matches, 0);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_replace_in_workspace_literal_and_regex_capture_groups() {
        let temp_dir =
            std::env::temp_dir().join(format!("rustcode_replace_m3_13_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        fs::write(temp_dir.join(".gitignore"), "ignored.rs\n").unwrap();
        let file_a = temp_dir.join("a.rs");
        let file_b = temp_dir.join("b.rs");
        let file_ignored = temp_dir.join("ignored.rs");

        fs::write(&file_a, "fn old_alpha() {}\r\nfn old_beta() {}\r\n").unwrap();
        fs::write(&file_b, "fn old_gamma() {}\n").unwrap();
        fs::write(&file_ignored, "fn old_secret() {}\n").unwrap();

        let folders = vec![WorkspaceFolder::new(
            temp_dir.clone(),
            Some("ReplaceRoot".to_string()),
            0,
        )];

        // 1. Regex capture group $1 replacement across multiple files (preserving CRLF & .gitignore)
        let regex_query = SearchQuery {
            query: r"fn old_(\w+)".to_string(),
            is_regex: true,
            ..Default::default()
        };
        let res = replace_in_workspace(&folders, &regex_query, "fn new_$1").unwrap();
        assert_eq!(res.files_modified, 2);
        assert_eq!(res.total_replacements, 3);
        assert_eq!(
            fs::read_to_string(&file_a).unwrap(),
            "fn new_alpha() {}\r\nfn new_beta() {}\r\n"
        );
        assert_eq!(
            fs::read_to_string(&file_b).unwrap(),
            "fn new_gamma() {}\n"
        );
        assert_eq!(
            fs::read_to_string(&file_ignored).unwrap(),
            "fn old_secret() {}\n"
        );

        // 2. Literal replacement treats $1 literally
        let lit_query = SearchQuery {
            query: "new_alpha".to_string(),
            is_regex: false,
            ..Default::default()
        };
        let res_lit = replace_in_workspace(&folders, &lit_query, "$1_alpha").unwrap();
        assert_eq!(res_lit.files_modified, 1);
        assert_eq!(res_lit.total_replacements, 1);
        assert_eq!(
            fs::read_to_string(&file_a).unwrap(),
            "fn $1_alpha() {}\r\nfn new_beta() {}\r\n"
        );

        // 3. Invalid regex returns Err and leaves disk untouched
        let bad_query = SearchQuery {
            query: "(unclosed".to_string(),
            is_regex: true,
            ..Default::default()
        };
        assert!(replace_in_workspace(&folders, &bad_query, "x").is_err());

        // 4. Open buffer helper parity
        let (buf_out, count) = replace_in_text("let old_v = 1;", &regex_query, "let $1").unwrap();
        assert_eq!(count, 0);
        assert_eq!(buf_out, "let old_v = 1;");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_files_nucleo_ranking() {
        let temp_dir =
            std::env::temp_dir().join(format!("rustcode_files_nucleo_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(temp_dir.join("src")).unwrap();
        fs::write(temp_dir.join("src/s_e_a_r_c_h.rs"), "").unwrap();
        fs::write(temp_dir.join("src/globalSearchEngine.rs"), "").unwrap();
        fs::write(temp_dir.join("src/search_files.rs"), "").unwrap();

        let folders = vec![WorkspaceFolder::new(
            temp_dir.clone(),
            Some("Root".to_string()),
            0,
        )];

        let ranked = search_files(&folders, "search", 10);
        assert_eq!(ranked.len(), 3);
        assert!(ranked[0].ends_with("search_files.rs"));
        assert!(ranked[1].ends_with("globalSearchEngine.rs"));
        assert!(ranked[2].ends_with("s_e_a_r_c_h.rs"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
