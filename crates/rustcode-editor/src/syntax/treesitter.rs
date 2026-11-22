//! Incremental `tree-sitter` syntax highlighting engine (desktop feature).
//!
//! Layers over `GrammarRegistry` / `SyntaxHighlighter` for Rust, JavaScript,
//! TypeScript/TSX, Python, and JSON with incremental `tree.edit(...)` re-parsing.

use crate::syntax::scope::{SyntaxScope, TokenSpan};
use tree_sitter::{InputEdit, Language, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

/// Map a tree-sitter `highlights.scm` capture name to `SyntaxScope`.
pub fn map_capture_name_to_scope(name: &str) -> SyntaxScope {
    let n = name.trim();
    if n.starts_with("comment.documentation") || n.starts_with("comment.block.documentation") {
        return SyntaxScope::CommentDoc;
    }
    if n.starts_with("comment.line") {
        return SyntaxScope::CommentLine;
    }
    if n.starts_with("comment.block") {
        return SyntaxScope::CommentBlock;
    }
    if n.starts_with("comment") {
        return SyntaxScope::Comment;
    }
    if n.starts_with("keyword.control") || n.starts_with("keyword.conditional") || n.starts_with("keyword.repeat") || n.starts_with("keyword.return") {
        return SyntaxScope::KeywordControl;
    }
    if n.starts_with("keyword.operator") {
        return SyntaxScope::KeywordOperator;
    }
    if n.starts_with("keyword") {
        return SyntaxScope::Keyword;
    }
    if n.starts_with("string.special") || n.starts_with("string.template") {
        return SyntaxScope::StringTemplate;
    }
    if n.starts_with("string") || n.starts_with("character") {
        return SyntaxScope::String;
    }
    if n.starts_with("number") || n.starts_with("float") || n.starts_with("constant.numeric") {
        return SyntaxScope::Number;
    }
    if n.starts_with("constant.builtin") || n.starts_with("boolean") {
        return SyntaxScope::ConstantLanguage;
    }
    if n.starts_with("constant") {
        return SyntaxScope::ConstantLanguage;
    }
    if n.starts_with("type.builtin") {
        return SyntaxScope::StorageType;
    }
    if n.starts_with("type") || n.starts_with("constructor") {
        return SyntaxScope::Type;
    }
    if n.starts_with("function.builtin") || n.starts_with("function.macro") {
        return SyntaxScope::SupportFunction;
    }
    if n.starts_with("function") {
        return SyntaxScope::Function;
    }
    if n.starts_with("variable.parameter") {
        return SyntaxScope::VariableParameter;
    }
    if n.starts_with("property") || n.starts_with("attribute") || n.starts_with("variable.member") {
        return SyntaxScope::Property;
    }
    if n.starts_with("variable.builtin") {
        return SyntaxScope::KeywordOther;
    }
    if n.starts_with("variable") {
        return SyntaxScope::Variable;
    }
    if n.starts_with("operator") {
        return SyntaxScope::Operator;
    }
    if n.starts_with("punctuation.bracket") {
        return SyntaxScope::PunctuationBracket;
    }
    if n.starts_with("punctuation.delimiter") || n.starts_with("punctuation.special") {
        return SyntaxScope::PunctuationDelimiter;
    }
    if n.starts_with("punctuation") {
        return SyntaxScope::Punctuation;
    }
    if n.starts_with("escape") {
        return SyntaxScope::ConstantLanguage;
    }
    SyntaxScope::Plain
}

fn capture_specificity(name: &str) -> u8 {
    let base = if name.starts_with("comment") || name.starts_with("string") {
        30
    } else if name.starts_with("keyword") || name.starts_with("function") || name.starts_with("type") {
        25
    } else if name.starts_with("number")
        || name.starts_with("float")
        || name.starts_with("constant.numeric")
    {
        22
    } else if name.starts_with("property") || name.starts_with("constant") {
        20
    } else if name.starts_with("variable") {
        15
    } else if name.starts_with("operator") || name.starts_with("punctuation") {
        10
    } else {
        5
    };
    base + (name.matches('.').count() as u8).min(5)
}

pub struct TreeSitterEngine;

impl TreeSitterEngine {
    pub fn supports_language(lang_id: &str) -> bool {
        matches!(
            lang_id.to_ascii_lowercase().as_str(),
            "rust" | "rs" | "javascript" | "js" | "jsx" | "typescript" | "ts" | "tsx" | "python" | "py" | "json" | "jsonc"
        )
    }

    pub fn language_and_query(lang_id: &str) -> Option<(Language, Query, u8)> {
        let lower = lang_id.to_ascii_lowercase();
        let (language, query_src, numeric_id): (Language, String, u8) = match lower.as_str() {
            "rust" | "rs" => (
                tree_sitter_rust::LANGUAGE.into(),
                format!(
                    "{}\n(integer_literal) @number\n(float_literal) @number\n",
                    tree_sitter_rust::HIGHLIGHTS_QUERY
                ),
                1,
            ),
            "javascript" | "js" | "jsx" => (
                tree_sitter_javascript::LANGUAGE.into(),
                format!(
                    "{}\n{}",
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
                ),
                2,
            ),
            "typescript" | "ts" => {
                let lang: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
                let combined = format!(
                    "{}\n{}",
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY
                );
                let query = Query::new(&lang, &combined)
                    .or_else(|_| Query::new(&lang, tree_sitter_typescript::HIGHLIGHTS_QUERY))
                    .ok()?;
                return Some((lang, query, 3));
            }
            "tsx" => {
                let lang: Language = tree_sitter_typescript::LANGUAGE_TSX.into();
                let combined = format!(
                    "{}\n{}\n{}",
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY
                );
                let query = Query::new(&lang, &combined)
                    .or_else(|_| Query::new(&lang, tree_sitter_typescript::HIGHLIGHTS_QUERY))
                    .ok()?;
                return Some((lang, query, 3));
            }
            "python" | "py" => (
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::HIGHLIGHTS_QUERY.to_string(),
                4,
            ),
            "json" | "jsonc" => (
                tree_sitter_json::LANGUAGE.into(),
                tree_sitter_json::HIGHLIGHTS_QUERY.to_string(),
                5,
            ),
            _ => return None,
        };

        let query = Query::new(&language, &query_src).ok()?;
        Some((language, query, numeric_id))
    }
}

/// Incremental `tree-sitter` document state maintaining a live CST and highlight query.
pub struct TreeSitterDocument {
    pub language_id: String,
    numeric_id: u8,
    parser: Parser,
    query: Query,
    tree: Option<Tree>,
    last_text: String,
    line_byte_starts: Vec<usize>,
}

impl std::fmt::Debug for TreeSitterDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeSitterDocument")
            .field("language_id", &self.language_id)
            .field("has_tree", &self.tree.is_some())
            .field("text_len", &self.last_text.len())
            .finish()
    }
}

impl TreeSitterDocument {
    pub fn new(lang_id: &str, initial_text: &str) -> Result<Self, String> {
        let (language, query, numeric_id) = TreeSitterEngine::language_and_query(lang_id)
            .ok_or_else(|| format!("Unsupported tree-sitter language: {lang_id}"))?;
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| format!("Failed to set tree-sitter language {lang_id}: {e}"))?;
        let mut doc = Self {
            language_id: lang_id.to_string(),
            numeric_id,
            parser,
            query,
            tree: None,
            last_text: String::new(),
            line_byte_starts: vec![0],
        };
        doc.parse_full(initial_text);
        Ok(doc)
    }

    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    pub fn text(&self) -> &str {
        &self.last_text
    }

    pub fn parse_full(&mut self, text: &str) {
        self.last_text = text.to_string();
        self.line_byte_starts = compute_line_starts(&self.last_text);
        self.tree = self.parser.parse(&self.last_text, None);
    }

    /// Incrementally notify `tree-sitter` via `tree.edit(...)` and re-parse using the previous tree.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_edit(
        &mut self,
        start_byte: usize,
        old_end_byte: usize,
        new_end_byte: usize,
        start_position: Point,
        old_end_position: Point,
        new_end_position: Point,
        new_text: &str,
    ) {
        if let Some(tree) = self.tree.as_mut() {
            let edit = InputEdit {
                start_byte,
                old_end_byte,
                new_end_byte,
                start_position,
                old_end_position,
                new_end_position,
            };
            tree.edit(&edit);
        }
        self.last_text = new_text.to_string();
        self.line_byte_starts = compute_line_starts(&self.last_text);
        self.tree = self.parser.parse(&self.last_text, self.tree.as_ref());
    }

    /// Apply a byte-offset edit `(start_byte, deleted_len, inserted_text)` incrementally,
    /// computing `tree_sitter::Point` coordinates automatically.
    pub fn apply_byte_edit(
        &mut self,
        start_byte: usize,
        deleted_len: usize,
        inserted_text: &str,
        new_full_text: &str,
    ) {
        let clamped_start = start_byte.min(self.last_text.len());
        let old_end_byte = (clamped_start + deleted_len).min(self.last_text.len());
        let new_end_byte = (clamped_start + inserted_text.len()).min(new_full_text.len());

        let start_position = byte_to_point(&self.last_text, clamped_start);
        let old_end_position = byte_to_point(&self.last_text, old_end_byte);
        let new_end_position = advance_point(start_position, inserted_text);

        self.apply_edit(
            clamped_start,
            old_end_byte,
            new_end_byte,
            start_position,
            old_end_position,
            new_end_position,
            new_full_text,
        );
    }

    /// Synchronize document text incrementally using minimal prefix/suffix byte diff + `tree.edit(...)`.
    pub fn sync_text(&mut self, new_text: &str) {
        if self.last_text == new_text {
            return;
        }
        if self.tree.is_none() {
            self.parse_full(new_text);
            return;
        }

        let old = self.last_text.as_bytes();
        let new_b = new_text.as_bytes();
        let min_len = old.len().min(new_b.len());

        let mut prefix = 0usize;
        while prefix < min_len && old[prefix] == new_b[prefix] {
            prefix += 1;
        }
        while prefix > 0
            && (!self.last_text.is_char_boundary(prefix) || !new_text.is_char_boundary(prefix))
        {
            prefix -= 1;
        }

        let mut suffix = 0usize;
        while suffix < (old.len() - prefix).min(new_b.len() - prefix)
            && old[old.len() - 1 - suffix] == new_b[new_b.len() - 1 - suffix]
        {
            suffix += 1;
        }
        while suffix > 0
            && (!self.last_text.is_char_boundary(old.len() - suffix)
                || !new_text.is_char_boundary(new_b.len() - suffix))
        {
            suffix -= 1;
        }

        let old_end_byte = old.len() - suffix;
        let new_end_byte = new_b.len() - suffix;
        let inserted = &new_text[prefix..new_end_byte];
        let deleted_len = old_end_byte.saturating_sub(prefix);
        self.apply_byte_edit(prefix, deleted_len, inserted, new_text);
    }

    /// Query highlight tokens for a 0-based `line_idx` (returning byte offsets relative to `line_text`).
    pub fn line_tokens(&self, line_idx: usize, line_text: &str) -> Vec<TokenSpan> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let Some(&line_start) = self.line_byte_starts.get(line_idx) else {
            return Vec::new();
        };
        let clean_len = line_text.trim_end_matches('\r').len();
        if clean_len == 0 {
            return Vec::new();
        }
        let line_end = (line_start + clean_len).min(self.last_text.len());
        if line_start >= line_end {
            return Vec::new();
        }

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(line_start..line_end);

        let capture_names = self.query.capture_names();
        let text_bytes = self.last_text.as_bytes();

        let mut raw_spans: Vec<(usize, usize, SyntaxScope, u8)> = Vec::new();
        let mut captures = cursor.captures(&self.query, tree.root_node(), text_bytes);
        while let Some((mat, cap_idx)) = captures.next() {
            let cap = mat.captures[*cap_idx];
            let node = cap.node;
            let s = node.start_byte().max(line_start);
            let e = node.end_byte().min(line_end);
            if s >= e {
                continue;
            }
            let rel_s = s - line_start;
            let rel_e = e - line_start;
            if rel_e <= line_text.len()
                && line_text.is_char_boundary(rel_s)
                && line_text.is_char_boundary(rel_e)
            {
                let cap_name = capture_names[cap.index as usize];
                let scope = map_capture_name_to_scope(cap_name);
                if scope != SyntaxScope::Plain {
                    let spec = capture_specificity(cap_name);
                    raw_spans.push((rel_s, rel_e, scope, spec));
                }
            }
        }

        resolve_non_overlapping_line_spans(line_text, raw_spans, self.numeric_id)
    }
}

fn resolve_non_overlapping_line_spans(
    line_text: &str,
    mut raw: Vec<(usize, usize, SyntaxScope, u8)>,
    lang_id: u8,
) -> Vec<TokenSpan> {
    if raw.is_empty() {
        return Vec::new();
    }
    // Sort by start asc, then narrower/more-specific first
    raw.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.3.cmp(&a.3))
            .then_with(|| (a.1 - a.0).cmp(&(b.1 - b.0)))
    });

    // Collect boundary points to split overlapping intervals cleanly, choosing highest specificity
    let mut points: Vec<usize> = Vec::with_capacity(raw.len() * 2);
    for &(s, e, _, _) in &raw {
        points.push(s);
        points.push(e);
    }
    points.sort_unstable();
    points.dedup();

    let mut out = Vec::new();
    for win in points.windows(2) {
        let seg_s = win[0];
        let seg_e = win[1];
        if seg_s >= seg_e || !line_text.is_char_boundary(seg_s) || !line_text.is_char_boundary(seg_e)
        {
            continue;
        }
        // Pick best matching span covering [seg_s, seg_e): highest specificity, then smallest span
        let best = raw
            .iter()
            .filter(|&&(rs, re, _, _)| rs <= seg_s && re >= seg_e)
            .max_by(|a, b| a.3.cmp(&b.3).then_with(|| (b.1 - b.0).cmp(&(a.1 - a.0))));
        if let Some(&(_, _, scope, _)) = best {
            if let Some(last) = out.last_mut() {
                let last_span: &mut TokenSpan = last;
                if last_span.end == seg_s && last_span.scope == scope {
                    last_span.end = seg_e;
                    continue;
                }
            }
            out.push(TokenSpan::new(seg_s, seg_e, scope, lang_id));
        }
    }
    out
}

fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn byte_to_point(text: &str, byte_offset: usize) -> Point {
    let clamped = byte_offset.min(text.len());
    let mut row = 0usize;
    let mut last_nl = 0usize;
    for (i, b) in text.as_bytes()[..clamped].iter().enumerate() {
        if *b == b'\n' {
            row += 1;
            last_nl = i + 1;
        }
    }
    Point::new(row, clamped - last_nl)
}

fn advance_point(start: Point, inserted: &str) -> Point {
    let mut row = start.row;
    let mut col = start.column;
    for b in inserted.bytes() {
        if b == b'\n' {
            row += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    Point::new(row, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_treesitter_all_bundled_languages_and_incremental_edit() {
        // Rust incremental parse + mid-file multi-line string/comment edit
        let src_v1 = "fn main() {\n    let x = 42;\n    println!(\"hi\");\n}\n";
        let mut ts_doc = TreeSitterDocument::new("rust", src_v1).unwrap();
        let line1_v1 = ts_doc.line_tokens(1, "    let x = 42;");
        assert!(line1_v1.iter().any(|t| t.scope == SyntaxScope::Keyword));
        assert!(line1_v1.iter().any(|t| t.scope == SyntaxScope::Number));

        // Insert an unclosed block comment start on line 1 -> line 2 becomes Comment!
        let src_v2 = "fn main() {\n    /* comment\n    let x = 42;\n    println!(\"hi\");\n*/\n}\n";
        ts_doc.sync_text(src_v2);
        let line2_v2 = ts_doc.line_tokens(2, "    let x = 42;");
        assert!(
            line2_v2
                .iter()
                .all(|t| matches!(t.scope, SyntaxScope::Comment | SyntaxScope::CommentBlock)),
            "expected line 2 inside block comment to be comment, got {:?}",
            line2_v2
        );

        // Close block comment right on line 1 -> line 2 becomes code again
        let src_v3 = "fn main() {\n    /* comment */\n    let x = 42;\n    println!(\"hi\");\n}\n";
        ts_doc.sync_text(src_v3);
        let line2_v3 = ts_doc.line_tokens(2, "    let x = 42;");
        assert!(line2_v3.iter().any(|t| t.scope == SyntaxScope::Keyword));

        // JS, TS, TSX, Python, JSON
        for (lang, sample, expected_scope) in [
            ("javascript", "const a = 'hello';", SyntaxScope::String),
            ("typescript", "interface User { id: number; }", SyntaxScope::Keyword),
            ("tsx", "const el = <div className=\"a\" />;", SyntaxScope::String),
            ("python", "def foo(x: int):\n    return x", SyntaxScope::Keyword),
            ("json", "{ \"key\": true }", SyntaxScope::ConstantLanguage),
        ] {
            let doc = TreeSitterDocument::new(lang, sample).unwrap();
            let first_line = sample.lines().next().unwrap();
            let toks = doc.line_tokens(0, first_line);
            assert!(
                toks.iter().any(|t| t.scope == expected_scope),
                "{lang} missing expected scope {:?} in {:?}",
                expected_scope,
                toks
            );
        }
    }
}
