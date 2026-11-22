//! Pure-Rust symbol extractor for workspace-wide and document-level symbol indexing.

use crate::search::engine::{parse_nucleo_pattern, score_nucleo_match};
use crate::workspace::WorkspaceFolder;
use ignore::WalkBuilder;
use nucleo_matcher::{Config, Matcher as NucleoMatcher};
use regex::Regex;
use rustcode_protocols::lsp::types::{LspLocation, LspPosition, LspRange, SymbolInformation, SymbolKind};
use std::cmp::Reverse;
use std::fs;
use std::path::Path;

/// Extract symbols from source document content based on file extension and token patterns.
pub fn search_document_symbols(content: &str, file_path: &Path) -> Vec<SymbolInformation> {
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let uri = rustcode_platform::path_to_file_uri(file_path);

    match ext.as_str() {
        "rs" => extract_rust_symbols(content, &uri),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => extract_js_ts_symbols(content, &uri),
        "py" => extract_python_symbols(content, &uri),
        "go" => extract_go_symbols(content, &uri),
        _ => Vec::new(),
    }
}

/// Search for symbols matching `query` across all workspace folders, ranked by `nucleo`.
pub fn search_workspace_symbols(
    folders: &[WorkspaceFolder],
    query: &str,
) -> Vec<SymbolInformation> {
    let trimmed = query.trim();
    let mut extracted = Vec::new();
    let collect_limit = if trimmed.is_empty() { 500 } else { 5000 };

    for folder in folders {
        if extracted.len() >= collect_limit {
            break;
        }
        walk_and_extract_symbols(&folder.path, collect_limit, &mut extracted);
    }

    if trimmed.is_empty() {
        extracted.truncate(500);
        return extracted;
    }

    let mut ranked = rank_symbols(extracted, trimmed);
    ranked.truncate(500);
    ranked
}

/// Filter and rank symbols using `nucleo-matcher`. Empty queries preserve original order.
pub fn rank_symbols(symbols: Vec<SymbolInformation>, query: &str) -> Vec<SymbolInformation> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return symbols;
    }

    let mut matcher = NucleoMatcher::new(Config::DEFAULT);
    let pattern = parse_nucleo_pattern(trimmed);
    let mut buf = Vec::new();
    let mut indices = Vec::new();
    let mut scored: Vec<(u32, usize, SymbolInformation)> = Vec::new();

    for (idx, sym) in symbols.into_iter().enumerate() {
        let name_score = score_nucleo_match(
            &pattern,
            trimmed,
            &sym.name,
            &mut matcher,
            &mut buf,
            &mut indices,
        );
        let full_score = sym.container_name.as_ref().and_then(|c| {
            let qualified = format!("{c}::{}", sym.name);
            score_nucleo_match(
                &pattern,
                trimmed,
                &qualified,
                &mut matcher,
                &mut buf,
                &mut indices,
            )
        });
        let combined = match (name_score, full_score) {
            (Some(ns), Some(fs)) => Some(ns.saturating_add(16).max(fs)),
            (Some(ns), None) => Some(ns.saturating_add(16)),
            (None, Some(fs)) => Some(fs),
            (None, None) => None,
        };
        if let Some(score) = combined {
            scored.push((score, idx, sym));
        }
    }

    scored.sort_by(|a, b| {
        Reverse(a.0)
            .cmp(&Reverse(b.0))
            .then_with(|| a.2.name.len().cmp(&b.2.name.len()))
            .then_with(|| a.1.cmp(&b.1))
    });
    scored.into_iter().map(|(_, _, sym)| sym).collect()
}

fn walk_and_extract_symbols(
    dir: &Path,
    limit: usize,
    symbols: &mut Vec<SymbolInformation>,
) {
    let walker = WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .require_git(false)
        .parents(false)
        .build();

    for entry in walker.flatten() {
        if symbols.len() >= limit {
            break;
        }
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(
            ext.as_str(),
            "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "go"
        ) {
            if let Ok(content) = fs::read_to_string(path) {
                for s in search_document_symbols(&content, path) {
                    symbols.push(s);
                    if symbols.len() >= limit {
                        break;
                    }
                }
            }
        }
    }
}

fn extract_rust_symbols(content: &str, uri: &str) -> Vec<SymbolInformation> {
    let mut symbols = Vec::new();
    let fn_re = Regex::new(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?(?:unsafe\s+)?fn\s+(\w+)").unwrap();
    let struct_re = Regex::new(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?struct\s+(\w+)").unwrap();
    let enum_re = Regex::new(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?enum\s+(\w+)").unwrap();
    let trait_re = Regex::new(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?trait\s+(\w+)").unwrap();
    let type_re = Regex::new(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?type\s+(\w+)").unwrap();
    let const_re = Regex::new(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?const\s+(\w+)").unwrap();
    let mod_re = Regex::new(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(\w+)").unwrap();
    let impl_re = Regex::new(r"^\s*impl(?:<[^>]*>)?\s+(?:[\w:]+\s+for\s+)?(\w+)").unwrap();

    let mut current_container: Option<String> = None;

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx as u32;

        if let Some(caps) = impl_re.captures(line) {
            current_container = Some(caps[1].to_string());
        }

        if let Some(caps) = fn_re.captures(line) {
            let name = &caps[1];
            let kind = if current_container.is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            symbols.push(make_symbol(name, kind, uri, line_num, line, current_container.clone()));
        } else if let Some(caps) = struct_re.captures(line) {
            let name = &caps[1];
            current_container = Some(name.to_string());
            symbols.push(make_symbol(name, SymbolKind::Struct, uri, line_num, line, None));
        } else if let Some(caps) = enum_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::Enum, uri, line_num, line, None));
        } else if let Some(caps) = trait_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::Interface, uri, line_num, line, None));
        } else if let Some(caps) = type_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::TypeParameter, uri, line_num, line, None));
        } else if let Some(caps) = const_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::Constant, uri, line_num, line, None));
        } else if let Some(caps) = mod_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::Module, uri, line_num, line, None));
        }

        if line.contains('}') && current_container.is_some() && !line.contains('{') {
            current_container = None;
        }
    }

    symbols
}

fn extract_js_ts_symbols(content: &str, uri: &str) -> Vec<SymbolInformation> {
    let mut symbols = Vec::new();
    let fn_re = Regex::new(r"^\s*(?:export\s+)?(?:async\s+)?function\s+([\w$]+)").unwrap();
    let class_re = Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?class\s+([\w$]+)").unwrap();
    let interface_re = Regex::new(r"^\s*(?:export\s+)?interface\s+([\w$]+)").unwrap();
    let type_re = Regex::new(r"^\s*(?:export\s+)?type\s+([\w$]+)").unwrap();
    let enum_re = Regex::new(r"^\s*(?:export\s+)?enum\s+([\w$]+)").unwrap();
    let const_fn_re = Regex::new(r"^\s*(?:export\s+)?(?:const|let|var)\s+([\w$]+)\s*=\s*(?:async\s*)?(?:\([^)]*\)|[\w$]+)\s*=>").unwrap();

    let mut current_class: Option<String> = None;

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx as u32;

        if let Some(caps) = class_re.captures(line) {
            let name = &caps[1];
            current_class = Some(name.to_string());
            symbols.push(make_symbol(name, SymbolKind::Class, uri, line_num, line, None));
        } else if let Some(caps) = interface_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::Interface, uri, line_num, line, None));
        } else if let Some(caps) = type_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::TypeParameter, uri, line_num, line, None));
        } else if let Some(caps) = enum_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::Enum, uri, line_num, line, None));
        } else if let Some(caps) = fn_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::Function, uri, line_num, line, current_class.clone()));
        } else if let Some(caps) = const_fn_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::Function, uri, line_num, line, current_class.clone()));
        }

        if line.contains('}') && current_class.is_some() && !line.contains('{') {
            current_class = None;
        }
    }

    symbols
}

fn extract_python_symbols(content: &str, uri: &str) -> Vec<SymbolInformation> {
    let mut symbols = Vec::new();
    let def_re = Regex::new(r"^\s*(?:async\s+)?def\s+(\w+)").unwrap();
    let class_re = Regex::new(r"^\s*class\s+(\w+)").unwrap();

    let mut current_class: Option<String> = None;

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx as u32;

        if let Some(caps) = class_re.captures(line) {
            let name = &caps[1];
            current_class = Some(name.to_string());
            symbols.push(make_symbol(name, SymbolKind::Class, uri, line_num, line, None));
        } else if let Some(caps) = def_re.captures(line) {
            let name = &caps[1];
            let kind = if line.starts_with("    ") && current_class.is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            symbols.push(make_symbol(name, kind, uri, line_num, line, current_class.clone()));
        }

        if !line.starts_with(' ') && !line.is_empty() && !line.starts_with("class ") {
            current_class = None;
        }
    }

    symbols
}

fn extract_go_symbols(content: &str, uri: &str) -> Vec<SymbolInformation> {
    let mut symbols = Vec::new();
    let fn_re = Regex::new(r"^\s*func\s+(?:\([^)]*\)\s+)?(\w+)").unwrap();
    let type_re = Regex::new(r"^\s*type\s+(\w+)\s+(struct|interface)").unwrap();

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx as u32;
        if let Some(caps) = fn_re.captures(line) {
            let name = &caps[1];
            symbols.push(make_symbol(name, SymbolKind::Function, uri, line_num, line, None));
        } else if let Some(caps) = type_re.captures(line) {
            let name = &caps[1];
            let kind = if &caps[2] == "struct" {
                SymbolKind::Struct
            } else {
                SymbolKind::Interface
            };
            symbols.push(make_symbol(name, kind, uri, line_num, line, None));
        }
    }

    symbols
}

fn make_symbol(
    name: &str,
    kind: SymbolKind,
    uri: &str,
    line_num: u32,
    line: &str,
    container_name: Option<String>,
) -> SymbolInformation {
    let start_char = line.find(name).unwrap_or(0) as u32;
    let end_char = start_char + name.len() as u32;

    SymbolInformation {
        name: name.to_string(),
        kind,
        tags: None,
        deprecated: None,
        location: LspLocation {
            uri: uri.to_string(),
            range: LspRange::new(
                LspPosition::new(line_num, start_char),
                LspPosition::new(line_num, end_char),
            ),
        },
        container_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_symbol_extraction() {
        let src = r#"
            pub mod parser;
            pub const MAX_SIZE: usize = 1024;
            pub struct Engine {
                id: u32,
            }
            pub enum State {
                Active,
                Idle,
            }
            pub trait Service {
                fn run(&self);
            }
            impl Engine {
                pub fn start(&self) {}
            }
            fn helper() {}
        "#;

        let symbols = extract_rust_symbols(src, "file:///workspace/engine.rs");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"parser"));
        assert!(names.contains(&"MAX_SIZE"));
        assert!(names.contains(&"Engine"));
        assert!(names.contains(&"State"));
        assert!(names.contains(&"Service"));
        assert!(names.contains(&"start"));
        assert!(names.contains(&"helper"));

        let start_sym = symbols.iter().find(|s| s.name == "start").unwrap();
        assert_eq!(start_sym.kind, SymbolKind::Method);
        assert_eq!(start_sym.container_name.as_deref(), Some("Engine"));
    }

    #[test]
    fn test_js_ts_and_python_symbol_extraction() {
        let ts_src = r#"
            export interface User { id: string; }
            export class AuthManager {
                login() {}
            }
            export const logout = () => {};
            function checkStatus() {}
        "#;
        let ts_syms = extract_js_ts_symbols(ts_src, "file:///workspace/auth.ts");
        let ts_names: Vec<&str> = ts_syms.iter().map(|s| s.name.as_str()).collect();
        assert!(ts_names.contains(&"User"));
        assert!(ts_names.contains(&"AuthManager"));
        assert!(ts_names.contains(&"logout"));
        assert!(ts_names.contains(&"checkStatus"));

        let py_src = r#"
class DataProcessor:
    def process(self):
        pass

def top_level_func():
    pass
        "#;
        let py_syms = extract_python_symbols(py_src, "file:///workspace/proc.py");
        let py_names: Vec<&str> = py_syms.iter().map(|s| s.name.as_str()).collect();
        assert!(py_names.contains(&"DataProcessor"));
        assert!(py_names.contains(&"process"));
        assert!(py_names.contains(&"top_level_func"));
    }

    #[test]
    fn test_unicode_symbol_extraction() {
        // 1. Rust Unicode identifiers (German umlauts, accented characters)
        let rs_src = r#"
            pub struct ÜberManager;
            pub trait Prüfer {
                fn prüfen(&self);
            }
            impl Prüfer for ÜberManager {
                fn prüfen(&self) {}
            }
            pub fn berechne_größe() -> usize { 42 }
        "#;
        let rs_syms = search_document_symbols(rs_src, Path::new("test.rs"));
        assert!(
            rs_syms.iter().any(|s| s.name == "ÜberManager" && s.kind == SymbolKind::Struct),
            "ÜberManager not found in Rust symbols"
        );
        assert!(
            rs_syms.iter().any(|s| s.name == "Prüfer" && s.kind == SymbolKind::Interface),
            "Prüfer not found in Rust symbols"
        );
        assert!(
            rs_syms.iter().any(|s| s.name == "berechne_größe" && s.kind == SymbolKind::Function),
            "berechne_größe not found or truncated in Rust symbols"
        );
        let prufen_method = rs_syms
            .iter()
            .find(|s| s.name == "prüfen" && s.kind == SymbolKind::Method)
            .expect("prüfen method with ÜberManager container");
        assert_eq!(prufen_method.container_name.as_deref(), Some("ÜberManager"));

        // 2. JavaScript / TypeScript Unicode & $ identifiers
        let ts_src = r#"
            export class CaféRésumé {}
            export function créerSession() {}
            export const $zahlen_über = () => {};
        "#;
        let ts_syms = search_document_symbols(ts_src, Path::new("service.ts"));
        assert!(
            ts_syms.iter().any(|s| s.name == "CaféRésumé" && s.kind == SymbolKind::Class),
            "CaféRésumé not found or truncated"
        );
        assert!(
            ts_syms.iter().any(|s| s.name == "créerSession" && s.kind == SymbolKind::Function),
            "créerSession not found or truncated"
        );
        assert!(
            ts_syms.iter().any(|s| s.name == "$zahlen_über" && s.kind == SymbolKind::Function),
            "$zahlen_über not found or truncated"
        );

        // 3. Python CJK & accented identifiers
        let py_src = r#"
class ユーザーマネージャー:
    def 認証する(self):
        pass

def 用户登录():
    pass
"#;
        let py_syms = search_document_symbols(py_src, Path::new("auth.py"));
        assert!(
            py_syms.iter().any(|s| s.name == "ユーザーマネージャー" && s.kind == SymbolKind::Class),
            "ユーザーマネージャー class not found"
        );
        assert!(
            py_syms.iter().any(|s| s.name == "認証する" && s.kind == SymbolKind::Method),
            "認証する method not found"
        );
        assert!(
            py_syms.iter().any(|s| s.name == "用户登录" && s.kind == SymbolKind::Function),
            "用户登录 function not found"
        );

        // 4. Go Unicode symbols
        let go_src = r#"
func PrüfenAktion() {}
type ÜberService struct {}
"#;
        let go_syms = search_document_symbols(go_src, Path::new("service.go"));
        assert!(
            go_syms.iter().any(|s| s.name == "PrüfenAktion" && s.kind == SymbolKind::Function),
            "PrüfenAktion not found in Go symbols"
        );
        assert!(
            go_syms.iter().any(|s| s.name == "ÜberService" && s.kind == SymbolKind::Struct),
            "ÜberService not found in Go symbols"
        );

        // 5. Workspace symbol search across folders
        let temp_dir = std::env::temp_dir().join(format!("rustcode_sym_test_{}", uuid::Uuid::new_v4()));
        let folder_a = temp_dir.join("folder_a");
        let folder_b = temp_dir.join("folder_b");
        fs::create_dir_all(&folder_a).unwrap();
        fs::create_dir_all(&folder_b).unwrap();

        fs::write(folder_a.join("service.rs"), rs_src).unwrap();
        fs::write(folder_a.join("service.ts"), ts_src).unwrap();
        fs::write(folder_b.join("auth.py"), py_src).unwrap();
        fs::write(folder_b.join("service.go"), go_src).unwrap();

        let folders = vec![
            WorkspaceFolder::new(folder_a, Some("FolderA".to_string()), 0),
            WorkspaceFolder::new(folder_b, Some("FolderB".to_string()), 1),
        ];

        let test_queries = [
            ("größe", "berechne_größe"),
            ("Über", "ÜberManager"),
            ("prüfen", "prüfen"),
            ("Café", "CaféRésumé"),
            ("créer", "créerSession"),
            ("用户", "用户登录"),
            ("ユーザー", "ユーザーマネージャー"),
        ];

        for (query, expected_name) in test_queries {
            let results = search_workspace_symbols(&folders, query);
            assert!(
                results.iter().any(|s| s.name == expected_name),
                "Expected search_workspace_symbols to find '{}' for query '{}', found: {:?}",
                expected_name,
                query,
                results.iter().map(|s| &s.name).collect::<Vec<_>>()
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_workspace_symbols_nucleo_ranking_and_gitignore() {
        let temp_dir =
            std::env::temp_dir().join(format!("rustcode_sym_nucleo_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(temp_dir.join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(
            temp_dir.join("ignored.rs"),
            "pub fn parse_ignored() {}\n",
        )
        .unwrap();
        fs::write(
            temp_dir.join("lib.rs"),
            r#"
pub fn p_a_r_s_e_scattered() {}
pub fn runParseQuery() {}
pub fn parse_query() {}
"#,
        )
        .unwrap();

        let folders = vec![WorkspaceFolder::new(
            temp_dir.clone(),
            Some("Root".to_string()),
            0,
        )];

        // Ignored file symbols are excluded; empty query preserves document order
        let all = search_workspace_symbols(&folders, "");
        let all_names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            all_names,
            vec!["p_a_r_s_e_scattered", "runParseQuery", "parse_query"]
        );

        // Nucleo ranks exact prefix > camelCase boundary > scattered
        let ranked = search_workspace_symbols(&folders, "parse");
        let ranked_names: Vec<&str> = ranked.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            ranked_names,
            vec!["parse_query", "runParseQuery", "p_a_r_s_e_scattered"]
        );

        // Adversarial Unicode/symbol queries never panic
        for q in ["!", "^", "$", "'", "🚀", "std::foo()", "   "] {
            let _ = search_workspace_symbols(&folders, q);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
