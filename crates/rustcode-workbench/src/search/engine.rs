//! Fast in-process cross-folder search engine supporting regex, globs, case, and whole-word matching.

use crate::workspace::WorkspaceFolder;
use regex::{Regex, RegexBuilder};
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

/// Perform in-process text search across all workspace folders.
pub fn search_text(folders: &[WorkspaceFolder], query: &SearchQuery) -> SearchResults {
    if query.query.trim().is_empty() {
        return SearchResults::default();
    }

    let regex = match build_search_regex(query) {
        Ok(r) => r,
        Err(_) => return SearchResults::default(),
    };

    let mut results = SearchResults::default();

    for folder in folders {
        if results.total_matches >= query.max_results {
            break;
        }

        walk_and_search_folder(folder, query, &regex, &mut results);
    }

    results
}

/// Perform file search across all workspace folders matching a pattern or fuzzy query.
pub fn search_files(
    folders: &[WorkspaceFolder],
    pattern: &str,
    max_results: usize,
) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let pat = pattern.trim().to_lowercase();

    for folder in folders {
        if files.len() >= max_results {
            break;
        }
        walk_folder_for_files(&folder.path, &folder.path, &pat, max_results, &mut files);
    }

    files
}

fn build_search_regex(query: &SearchQuery) -> Result<Regex, regex::Error> {
    let pattern_body = if query.is_regex {
        query.query.clone()
    } else {
        regex::escape(&query.query)
    };

    let full_pattern = if query.match_whole_word {
        format!(r"\b(?:{})\b", pattern_body)
    } else {
        pattern_body
    };

    RegexBuilder::new(&full_pattern)
        .case_insensitive(!query.match_case)
        .multi_line(false)
        .build()
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

fn matches_glob(path_str: &str, pattern: &str) -> bool {
    let p = pattern.trim().replace('\\', "/");
    let path_str = path_str.replace('\\', "/");
    if p.is_empty() || p == "*" || p == "**" {
        return true;
    }

    if let Some(ext) = p.strip_prefix("*.") {
        return path_str.ends_with(&format!(".{}", ext));
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

        let regex_pat = format!("{}{}$", prefix, body);
        if let Ok(re) = Regex::new(&regex_pat) {
            return re.is_match(&path_str);
        }
    }

    path_str.contains(&p)
}

fn should_include_file(file_path: &Path, query: &SearchQuery) -> bool {
    let path_str = file_path.to_string_lossy();

    // Check excludes
    for exc in &query.files_to_exclude {
        if matches_glob(&path_str, exc) {
            return false;
        }
    }

    // Check includes
    if !query.files_to_include.is_empty() {
        let mut included = false;
        for inc in &query.files_to_include {
            if matches_glob(&path_str, inc) {
                included = true;
                break;
            }
        }
        if !included {
            return false;
        }
    }

    true
}

fn walk_and_search_folder(
    folder: &WorkspaceFolder,
    query: &SearchQuery,
    regex: &Regex,
    results: &mut SearchResults,
) {
    let mut dirs_to_visit = vec![folder.path.clone()];

    while let Some(current_dir) = dirs_to_visit.pop() {
        if results.total_matches >= query.max_results {
            break;
        }

        let entries = match fs::read_dir(&current_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            if results.total_matches >= query.max_results {
                break;
            }

            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if path.is_dir() {
                if !is_ignored_directory(&file_name) {
                    dirs_to_visit.push(path);
                }
            } else if path.is_file() {
                if !should_include_file(&path, query) {
                    continue;
                }

                // Read file text and search lines
                if let Ok(content) = fs::read_to_string(&path) {
                    let mut file_matches = Vec::new();

                    for (line_idx, line) in content.lines().enumerate() {
                        if results.total_matches + file_matches.len() >= query.max_results {
                            break;
                        }

                        for m in regex.find_iter(line) {
                            file_matches.push(TextSearchMatch {
                                line_number: line_idx + 1,
                                start_col: m.start(),
                                end_col: m.end(),
                                line_content: line.to_string(),
                            });

                            if results.total_matches + file_matches.len() >= query.max_results {
                                break;
                            }
                        }
                    }

                    if !file_matches.is_empty() {
                        results.total_matches += file_matches.len();
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
            }
        }
    }
}

fn walk_folder_for_files(
    base_dir: &Path,
    dir: &Path,
    pattern: &str,
    max_results: usize,
    files: &mut Vec<PathBuf>,
) {
    let mut dirs_to_visit = vec![dir.to_path_buf()];

    while let Some(current_dir) = dirs_to_visit.pop() {
        if files.len() >= max_results {
            break;
        }

        let entries = match fs::read_dir(&current_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            if files.len() >= max_results {
                break;
            }

            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if path.is_dir() {
                if !is_ignored_directory(&file_name) {
                    dirs_to_visit.push(path);
                }
            } else if path.is_file() {
                if pattern.is_empty() {
                    files.push(path);
                } else {
                    let rel_path = path.strip_prefix(base_dir).unwrap_or(&path);
                    let rel_str = rel_path.to_string_lossy().to_lowercase();
                    if fuzzy_match_path(&rel_str, pattern) || fuzzy_match_path(&file_name.to_lowercase(), pattern) {
                        files.push(path);
                    }
                }
            }
        }
    }
}

fn fuzzy_match_path(target: &str, pattern: &str) -> bool {
    let mut pattern_chars = pattern.chars().peekable();
    for c in target.chars() {
        if let Some(&pc) = pattern_chars.peek() {
            if c == pc {
                pattern_chars.next();
            }
        } else {
            return true;
        }
    }
    pattern_chars.peek().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_engine_basic_and_options() {
        let temp_dir = std::env::temp_dir().join(format!("rustcode_search_test_{}", uuid::Uuid::new_v4()));
        let folder_a = temp_dir.join("folder_a");
        let folder_b = temp_dir.join("folder_b");
        fs::create_dir_all(&folder_a).unwrap();
        fs::create_dir_all(&folder_b).unwrap();

        fs::write(folder_a.join("alpha.rs"), "fn hello_world() {\n    println!(\"Hello World\");\n}\n").unwrap();
        fs::write(folder_b.join("beta.js"), "function helloWorld() {\n  console.log('hello world');\n}\n").unwrap();

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
        assert!(matches_glob("/workspace/project/src/handlers/auth.rs", "src/**/*.rs"));
        assert!(matches_glob("/workspace/project/src/handlers/sub/deep.rs", "src/**/*.rs"));
        assert!(!matches_glob("/workspace/project/tests/test_main.rs", "src/**/*.rs"));
        assert!(!matches_glob("/workspace/project/src/main.js", "src/**/*.rs"));

        assert!(matches_glob("/workspace/project/src/handlers/auth.rs", "handlers/*"));
        assert!(matches_glob("/workspace/project/src/handlers/data.rs", "handlers/*"));
        assert!(!matches_glob("/workspace/project/src/handlers/sub/deep.rs", "handlers/*"));
        assert!(!matches_glob("/workspace/project/src/main.rs", "handlers/*"));

        assert!(matches_glob("/workspace/project/src/main.rs", "src/*"));
        assert!(matches_glob("/workspace/project/src/lib.rs", "src/*"));
        assert!(!matches_glob("/workspace/project/src/handlers/auth.rs", "src/*"));

        // Full search_text integration test across nested directory structure
        let temp_dir = std::env::temp_dir().join(format!("rustcode_glob_test_{}", uuid::Uuid::new_v4()));
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

        let folders = vec![WorkspaceFolder::new(temp_dir.clone(), Some("Root".to_string()), 0)];

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
}
