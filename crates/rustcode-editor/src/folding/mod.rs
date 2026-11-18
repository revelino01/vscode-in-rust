//! Code folding range detection, collapse state, and visible-line navigation.

use crate::model::piece_table::PieceTable;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum FoldingRangeKind {
    #[default]
    Region,
    Comment,
    Imports,
    Brace,
    Indentation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldingRange {
    /// 0-based inclusive header line (remains visible when collapsed).
    pub start_line: usize,
    /// 0-based inclusive last line of the folded block (hidden when collapsed).
    /// Invariant: `end_line > start_line`.
    pub end_line: usize,
    pub kind: FoldingRangeKind,
}

impl FoldingRange {
    pub fn new(start_line: usize, end_line: usize, kind: FoldingRangeKind) -> Self {
        Self {
            start_line,
            end_line,
            kind,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FoldingModel {
    ranges: Vec<FoldingRange>,
    manual_ranges: Vec<FoldingRange>,
    pub collapsed: BTreeSet<usize>,
}

impl FoldingModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ranges(&self) -> &[FoldingRange] {
        &self.ranges
    }

    pub fn range_at(&self, start_line: usize) -> Option<&FoldingRange> {
        self.ranges
            .iter()
            .filter(|r| r.start_line == start_line)
            .max_by_key(|r| r.end_line)
    }

    pub fn is_foldable(&self, line: usize) -> bool {
        self.ranges.iter().any(|r| r.start_line == line)
    }

    pub fn is_collapsed(&self, line: usize) -> bool {
        self.collapsed.contains(&line)
    }

    pub fn has_collapsed(&self) -> bool {
        !self.collapsed.is_empty()
    }

    pub fn fold(&mut self, line: usize) -> bool {
        if self.is_foldable(line) {
            self.collapsed.insert(line)
        } else {
            false
        }
    }

    pub fn unfold(&mut self, line: usize) -> bool {
        self.collapsed.remove(&line)
    }

    pub fn toggle_fold(&mut self, line: usize) -> Option<bool> {
        if !self.is_foldable(line) {
            return None;
        }
        if self.collapsed.contains(&line) {
            self.collapsed.remove(&line);
            Some(false)
        } else {
            self.collapsed.insert(line);
            Some(true)
        }
    }

    pub fn fold_all(&mut self) {
        for r in &self.ranges {
            self.collapsed.insert(r.start_line);
        }
    }

    pub fn unfold_all(&mut self) {
        self.collapsed.clear();
    }

    pub fn add_range(&mut self, range: FoldingRange) {
        if range.end_line <= range.start_line {
            return;
        }
        self.manual_ranges.push(range.clone());
        self.ranges.push(range);
        self.normalize_ranges();
    }

    pub fn clear_manual_ranges(&mut self) {
        self.manual_ranges.clear();
        // Retain only ranges not exclusively from manual_ranges? Without re-reading buffer,
        // callers typically recompute afterwards; remove exact manual matches if needed.
        // Keeping simple: clear manual_ranges; next recompute won't include them.
    }

    pub fn is_line_hidden(&self, line: usize) -> bool {
        if self.collapsed.is_empty() {
            return false;
        }
        for &start in &self.collapsed {
            if start >= line {
                break;
            }
            if let Some(range) = self.range_at(start) {
                if line <= range.end_line {
                    return true;
                }
            }
        }
        false
    }

    /// If `line` is hidden inside a collapsed range, returns the outermost collapsed
    /// header line that hides `line`.
    pub fn enclosing_collapsed_start(&self, line: usize) -> Option<usize> {
        for &start in &self.collapsed {
            if start >= line {
                break;
            }
            if let Some(range) = self.range_at(start) {
                if line <= range.end_line {
                    return Some(start);
                }
            }
        }
        None
    }

    pub fn next_visible_line(&self, line: usize, line_count: usize) -> Option<usize> {
        let start = line.saturating_add(1);
        (start..line_count).find(|&l| !self.is_line_hidden(l))
    }

    pub fn prev_visible_line(&self, line: usize) -> Option<usize> {
        (0..line).rev().find(|&l| !self.is_line_hidden(l))
    }

    pub fn visible_lines(&self, line_count: usize) -> Vec<usize> {
        if self.collapsed.is_empty() {
            return (0..line_count).collect();
        }
        (0..line_count)
            .filter(|&l| !self.is_line_hidden(l))
            .collect()
    }

    pub fn visible_line_count(&self, line_count: usize) -> usize {
        if self.collapsed.is_empty() {
            return line_count;
        }
        (0..line_count)
            .filter(|&l| !self.is_line_hidden(l))
            .count()
    }

    pub fn buffer_line_to_visible_index(
        &self,
        buffer_line: usize,
        line_count: usize,
    ) -> Option<usize> {
        if buffer_line >= line_count || self.is_line_hidden(buffer_line) {
            return None;
        }
        if self.collapsed.is_empty() {
            return Some(buffer_line);
        }
        let mut vis = 0usize;
        for l in 0..buffer_line {
            if !self.is_line_hidden(l) {
                vis += 1;
            }
        }
        Some(vis)
    }

    pub fn visible_index_to_buffer_line(
        &self,
        visible_idx: usize,
        line_count: usize,
    ) -> Option<usize> {
        if self.collapsed.is_empty() {
            return (visible_idx < line_count).then_some(visible_idx);
        }
        let mut vis = 0usize;
        for l in 0..line_count {
            if !self.is_line_hidden(l) {
                if vis == visible_idx {
                    return Some(l);
                }
                vis += 1;
            }
        }
        None
    }

    /// Returns the innermost foldable range start_line containing `line` (`start_line <= line <= end_line`).
    pub fn find_enclosing_fold(&self, line: usize) -> Option<usize> {
        self.ranges
            .iter()
            .filter(|r| r.start_line <= line && line <= r.end_line)
            .max_by_key(|r| r.start_line)
            .map(|r| r.start_line)
    }

    /// Returns the innermost collapsed range start_line containing `line` (`start_line <= line <= end_line`).
    pub fn find_enclosing_collapsed_fold(&self, line: usize) -> Option<usize> {
        self.collapsed
            .iter()
            .copied()
            .filter(|&start| {
                start <= line
                    && self
                        .range_at(start)
                        .is_some_and(|r| line <= r.end_line)
            })
            .max()
    }

    /// Unfolds any collapsed range strictly containing `line` (`start_line < line <= end_line`).
    /// Returns `true` if at least one range was unfolded.
    pub fn unfold_containing(&mut self, line: usize) -> bool {
        let to_unfold: Vec<usize> = self
            .collapsed
            .iter()
            .copied()
            .filter(|&start| {
                start < line
                    && self
                        .range_at(start)
                        .is_some_and(|r| line <= r.end_line)
            })
            .collect();
        if to_unfold.is_empty() {
            return false;
        }
        for start in to_unfold {
            self.collapsed.remove(&start);
        }
        true
    }

    pub fn compute_from_buffer(&mut self, buf: &PieceTable, language_id: Option<&str>) {
        self.collapsed.clear();
        self.recompute_internal(buf, language_id);
    }

    pub fn recompute(&mut self, buf: &PieceTable, language_id: Option<&str>) {
        self.recompute_internal(buf, language_id);
        let valid_starts: BTreeSet<usize> = self.ranges.iter().map(|r| r.start_line).collect();
        self.collapsed.retain(|s| valid_starts.contains(s));
    }

    pub fn shift_lines_for_edit(&mut self, edit_start_line: usize, lines_delta: isize) {
        if lines_delta == 0 {
            return;
        }
        let mut next_collapsed = BTreeSet::new();
        for &start in &self.collapsed {
            if start <= edit_start_line {
                next_collapsed.insert(start);
            } else if lines_delta > 0 {
                next_collapsed.insert(start.saturating_add(lines_delta as usize));
            } else {
                let del = (-lines_delta) as usize;
                if start > edit_start_line.saturating_add(del) {
                    next_collapsed.insert(start - del);
                }
            }
        }
        self.collapsed = next_collapsed;

        let mut next_manual = Vec::with_capacity(self.manual_ranges.len());
        for r in &self.manual_ranges {
            if r.end_line <= edit_start_line {
                next_manual.push(r.clone());
            } else if lines_delta > 0 {
                let add = lines_delta as usize;
                let start_line = if r.start_line > edit_start_line {
                    r.start_line.saturating_add(add)
                } else {
                    r.start_line
                };
                let end_line = r.end_line.saturating_add(add);
                if end_line > start_line {
                    next_manual.push(FoldingRange::new(start_line, end_line, r.kind));
                }
            } else {
                let del = (-lines_delta) as usize;
                let del_end = edit_start_line.saturating_add(del);
                if r.start_line > edit_start_line && r.start_line <= del_end {
                    continue;
                }
                let start_line = if r.start_line > del_end {
                    r.start_line - del
                } else {
                    r.start_line
                };
                let end_line = if r.end_line > del_end {
                    r.end_line - del
                } else {
                    edit_start_line.max(start_line)
                };
                if end_line > start_line {
                    next_manual.push(FoldingRange::new(start_line, end_line, r.kind));
                }
            }
        }
        self.manual_ranges = next_manual;
    }

    pub fn adjust_for_edit(
        &mut self,
        edit_start_line: usize,
        lines_delta: isize,
        buf: &PieceTable,
        language_id: Option<&str>,
    ) {
        self.shift_lines_for_edit(edit_start_line, lines_delta);
        self.recompute(buf, language_id);
    }

    fn recompute_internal(&mut self, buf: &PieceTable, language_id: Option<&str>) {
        let mut detected = detect_ranges(buf, language_id);
        let line_count = buf.line_count();
        for mr in &self.manual_ranges {
            if mr.end_line < line_count && mr.end_line > mr.start_line {
                detected.push(mr.clone());
            }
        }
        self.ranges = detected;
        self.normalize_ranges();
    }

    fn normalize_ranges(&mut self) {
        self.ranges.retain(|r| r.end_line > r.start_line);
        self.ranges
            .sort_by(|a, b| a.start_line.cmp(&b.start_line).then_with(|| a.end_line.cmp(&b.end_line)));
        self.ranges
            .dedup_by(|a, b| a.start_line == b.start_line && a.end_line == b.end_line);
    }
}

fn detect_ranges(buf: &PieceTable, language_id: Option<&str>) -> Vec<FoldingRange> {
    let lang = language_id.unwrap_or("plaintext").to_ascii_lowercase();
    let line_count = buf.line_count();
    if line_count <= 1 {
        return Vec::new();
    }

    match lang.as_str() {
        "markdown" | "md" => detect_markdown_ranges(buf, line_count),
        "python" | "py" => detect_indentation_ranges(buf, line_count),
        "plaintext" | "text" | "" => detect_indentation_ranges(buf, line_count),
        _ => {
            let mut ranges = detect_brace_and_comment_ranges(buf, line_count, &lang);
            if ranges.is_empty() {
                ranges = detect_indentation_ranges(buf, line_count);
            }
            ranges
        }
    }
}

fn detect_markdown_ranges(buf: &PieceTable, line_count: usize) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();
    let mut fence_start: Option<(usize, char, usize)> = None;
    let mut heading_stack: Vec<(usize, usize)> = Vec::new();
    let mut last_non_blank_line = 0usize;

    for line_idx in 0..line_count {
        let line = buf.get_line_content(line_idx).unwrap_or_default();
        let trimmed = line.trim_start();
        if !trimmed.is_empty() {
            // Check fenced code block
            let first = trimmed.chars().next();
            if let Some(fc) = first.filter(|&c| c == '`' || c == '~') {
                let flen = trimmed.chars().take_while(|&c| c == fc).count();
                if flen >= 3 {
                    if let Some((start, open_ch, open_len)) = fence_start {
                        if fc == open_ch && flen >= open_len && trimmed[flen * fc.len_utf8()..].trim().is_empty() {
                            if line_idx > start {
                                ranges.push(FoldingRange::new(start, line_idx, FoldingRangeKind::Region));
                            }
                            fence_start = None;
                            last_non_blank_line = line_idx;
                            continue;
                        }
                    } else {
                        fence_start = Some((line_idx, fc, flen));
                        last_non_blank_line = line_idx;
                        continue;
                    }
                }
            }

            if fence_start.is_none() && trimmed.starts_with('#') {
                let hashes = trimmed.chars().take_while(|&c| c == '#').count();
                let after = trimmed[hashes..].chars().next();
                if (1..=6).contains(&hashes) && (after.is_none() || matches!(after, Some(' ' | '\t'))) {
                    while let Some(&(top_line, top_level)) = heading_stack.last() {
                        if top_level >= hashes {
                            heading_stack.pop();
                            let end_line = last_non_blank_line.min(line_idx.saturating_sub(1));
                            if end_line > top_line {
                                ranges.push(FoldingRange::new(top_line, end_line, FoldingRangeKind::Region));
                            }
                        } else {
                            break;
                        }
                    }
                    heading_stack.push((line_idx, hashes));
                }
            }

            last_non_blank_line = line_idx;
        }
    }

    while let Some((top_line, _)) = heading_stack.pop() {
        if last_non_blank_line > top_line {
            ranges.push(FoldingRange::new(top_line, last_non_blank_line, FoldingRangeKind::Region));
        }
    }

    ranges
}

fn leading_indent(line: &str) -> Option<usize> {
    let mut indent = 0usize;
    for ch in line.chars() {
        match ch {
            ' ' => indent += 1,
            '\t' => indent += 4,
            _ => return Some(indent),
        }
    }
    None
}

fn detect_indentation_ranges(buf: &PieceTable, line_count: usize) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut last_non_blank_line = 0usize;

    for line_idx in 0..line_count {
        let line = buf.get_line_content(line_idx).unwrap_or_default();
        if let Some(cur_indent) = leading_indent(&line) {
            while let Some(&(top_line, top_indent)) = stack.last() {
                if cur_indent <= top_indent {
                    stack.pop();
                    if last_non_blank_line > top_line {
                        ranges.push(FoldingRange::new(
                            top_line,
                            last_non_blank_line,
                            FoldingRangeKind::Indentation,
                        ));
                    }
                } else {
                    break;
                }
            }
            stack.push((line_idx, cur_indent));
            last_non_blank_line = line_idx;
        }
    }

    while let Some((top_line, _)) = stack.pop() {
        if last_non_blank_line > top_line {
            ranges.push(FoldingRange::new(
                top_line,
                last_non_blank_line,
                FoldingRangeKind::Indentation,
            ));
        }
    }

    ranges
}

fn is_single_line_char_literal(bytes: &[u8], quote_idx: usize) -> Option<usize> {
    let n = bytes.len();
    if quote_idx + 2 >= n {
        return None;
    }
    if bytes[quote_idx + 1] == b'\\' {
        // Escaped char literal like '\n', '\'', '\\', '\x7f', '\u{1234}'
        let mut i = quote_idx + 2;
        if i < n {
            i += 1;
        }
        while i < n && i <= quote_idx + 12 {
            if bytes[i] == b'\'' {
                return Some(i + 1);
            }
            if matches!(bytes[i], b';' | b'{' | b'}' | b'(' | b')') {
                return None;
            }
            i += 1;
        }
        None
    } else {
        // Simple 'x' char literal (or multi-byte UTF-8 single char)
        let rest = std::str::from_utf8(&bytes[quote_idx + 1..]).ok()?;
        let mut chars = rest.chars();
        let ch = chars.next()?;
        if ch == '\'' {
            return None;
        }
        let next_ch = chars.next()?;
        if next_ch == '\'' {
            Some(quote_idx + 1 + ch.len_utf8() + 1)
        } else {
            None
        }
    }
}

fn is_rust_ident_boundary(prev_byte: u8) -> bool {
    !prev_byte.is_ascii_alphanumeric() && prev_byte != b'_' && prev_byte < 0x80
}

fn detect_brace_and_comment_ranges(
    buf: &PieceTable,
    line_count: usize,
    lang: &str,
) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();
    let is_rust = matches!(lang, "rust" | "rs");
    let track_brackets = matches!(lang, "json" | "jsonc");
    let allow_backtick_template = matches!(lang, "javascript" | "typescript" | "js" | "ts");
    let allow_single_quote_string = matches!(lang, "javascript" | "typescript" | "js" | "ts");
    let allow_multiline_double_quote = is_rust;

    let mut in_block_comment_start: Option<usize> = None;
    let mut in_string: Option<char> = None;
    let mut in_raw_string: Option<usize> = None;
    let mut brace_stack: Vec<(usize, char)> = Vec::new();
    let mut region_stack: Vec<usize> = Vec::new();

    for line_idx in 0..line_count {
        let line = buf.get_line_content(line_idx).unwrap_or_default();
        let trimmed = line.trim_start();
        if in_block_comment_start.is_none()
            && in_string.is_none()
            && in_raw_string.is_none()
            && trimmed.starts_with("//")
        {
            let comment_body = trimmed[2..].trim_start();
            if comment_body.starts_with("#region") || comment_body.starts_with("region ") || comment_body == "region" {
                region_stack.push(line_idx);
            } else if comment_body.starts_with("#endregion") || comment_body.starts_with("endregion") {
                if let Some(start) = region_stack.pop() {
                    if line_idx > start {
                        ranges.push(FoldingRange::new(start, line_idx, FoldingRangeKind::Region));
                    }
                }
            }
        }

        // Non-multiline strings reset at end of line.
        if let Some(q) = in_string {
            let keep_multiline = q == '`' || (q == '"' && allow_multiline_double_quote);
            if !keep_multiline {
                in_string = None;
            }
        }

        let bytes = line.as_bytes();
        let mut i = 0usize;
        let mut escaped = false;
        let mut pending_closes: Vec<(usize, usize)> = Vec::new();

        while i < bytes.len() {
            if let Some(comment_start) = in_block_comment_start {
                if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    if line_idx > comment_start {
                        ranges.push(FoldingRange::new(
                            comment_start,
                            line_idx,
                            FoldingRangeKind::Comment,
                        ));
                    }
                    in_block_comment_start = None;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }

            if let Some(hashes) = in_raw_string {
                if bytes[i] == b'"'
                    && i + 1 + hashes <= bytes.len()
                    && bytes[i + 1..i + 1 + hashes].iter().all(|&b| b == b'#')
                {
                    in_raw_string = None;
                    i += 1 + hashes;
                } else {
                    i += 1;
                }
                continue;
            }

            if let Some(q) = in_string {
                if escaped {
                    escaped = false;
                    i += 1;
                    continue;
                }
                if bytes[i] == b'\\' {
                    escaped = true;
                    i += 1;
                    continue;
                }
                if bytes[i] == q as u8 {
                    in_string = None;
                }
                i += 1;
                continue;
            }

            // Normal code state
            if bytes[i] == b'/' && i + 1 < bytes.len() {
                if bytes[i + 1] == b'/' {
                    break;
                }
                if bytes[i + 1] == b'*' {
                    in_block_comment_start = Some(line_idx);
                    i += 2;
                    continue;
                }
            }

            if is_rust {
                let r_pos = if bytes[i] == b'r'
                    && (i == 0 || is_rust_ident_boundary(bytes[i - 1]))
                {
                    Some(i)
                } else if (bytes[i] == b'b' || bytes[i] == b'c')
                    && i + 1 < bytes.len()
                    && bytes[i + 1] == b'r'
                    && (i == 0 || is_rust_ident_boundary(bytes[i - 1]))
                {
                    Some(i + 1)
                } else {
                    None
                };
                if let Some(rp) = r_pos {
                    let mut j = rp + 1;
                    while j < bytes.len() && bytes[j] == b'#' {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'"' {
                        let hashes = j - (rp + 1);
                        in_raw_string = Some(hashes);
                        i = j + 1;
                        continue;
                    }
                }
            }

            if bytes[i] == b'"' {
                in_string = Some('"');
                i += 1;
                continue;
            }

            if bytes[i] == b'`' && allow_backtick_template {
                in_string = Some('`');
                i += 1;
                continue;
            }

            if bytes[i] == b'\'' {
                if allow_single_quote_string {
                    in_string = Some('\'');
                    i += 1;
                    continue;
                } else if let Some(next_i) = is_single_line_char_literal(bytes, i) {
                    i = next_i;
                    continue;
                } else {
                    // Rust lifetime (e.g. 'a, 'static) — do not enter string state
                    i += 1;
                    continue;
                }
            }

            if bytes[i] == b'{' || (track_brackets && bytes[i] == b'[') {
                brace_stack.push((line_idx, bytes[i] as char));
            } else if bytes[i] == b'}' || (track_brackets && bytes[i] == b']') {
                let expected = if bytes[i] == b'}' { '{' } else { '[' };
                if let Some(pos) = brace_stack.iter().rposition(|&(_, c)| c == expected) {
                    let (open_line, _) = brace_stack.remove(pos);
                    if line_idx > open_line {
                        pending_closes.push((open_line, line_idx));
                    }
                }
            }

            i += 1;
        }

        // If a line closed a prior block AND opened a new block that stays open at EOL (e.g. `} else {`),
        // stop the prior fold at `line_idx - 1` so `} else {` stays visible as its own foldable header.
        // When `close_line == open_line + 1` (zero-body adjacent `} else {`), `end_line == open_line`
        // so no fold is emitted that would hide `} else {`.
        let line_opened_new_block = brace_stack.last().is_some_and(|&(l, _)| l == line_idx);
        for (open_line, close_line) in pending_closes {
            let end_line = if line_opened_new_block {
                close_line - 1
            } else {
                close_line
            };
            if end_line > open_line {
                ranges.push(FoldingRange::new(open_line, end_line, FoldingRangeKind::Brace));
            }
        }
    }

    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_folding_brace_and_else_and_block_comments() {
        let src = "\
fn demo<'a>(x: &'a str) {
    let s = \"not { a } fold\";
    // neither { this }
    if x.is_empty() {
        println!(\"empty\");
    } else {
        println!(\"{}\", x);
    }
    /* multi
       line
       comment */
}";
        let buf = PieceTable::from_str(src);
        let mut model = FoldingModel::new();
        model.compute_from_buffer(&buf, Some("rust"));

        // fn demo: lines 0..=11
        assert!(model.is_foldable(0));
        assert_eq!(model.range_at(0).unwrap().end_line, 11);
        // if branch stops at line 4 so `} else {` on line 5 stays visible
        assert!(model.is_foldable(3));
        assert_eq!(model.range_at(3).unwrap().end_line, 4);
        // else branch: lines 5..=7
        assert!(model.is_foldable(5));
        assert_eq!(model.range_at(5).unwrap().end_line, 7);
        // block comment: lines 8..=10
        assert!(model.is_foldable(8));
        let comment_range = model.range_at(8).unwrap();
        assert_eq!(comment_range.end_line, 10);
        assert_eq!(comment_range.kind, FoldingRangeKind::Comment);

        // Single-line strings/comments must not create folds
        assert!(!model.is_foldable(1));
        assert!(!model.is_foldable(2));
    }

    #[test]
    fn test_folding_python_indentation_and_markdown() {
        let py = "\
def outer():
    if True:
        x = 1

    return x

def second():
    pass";
        let py_buf = PieceTable::from_str(py);
        let mut py_model = FoldingModel::new();
        py_model.compute_from_buffer(&py_buf, Some("python"));
        assert!(py_model.is_foldable(0));
        assert_eq!(py_model.range_at(0).unwrap().end_line, 4);
        assert!(py_model.is_foldable(1));
        assert_eq!(py_model.range_at(1).unwrap().end_line, 2);
        assert!(py_model.is_foldable(6));
        assert_eq!(py_model.range_at(6).unwrap().end_line, 7);

        let md = "\
# Title
intro
## Sub
```rust
fn a() {}
```
# Next
end";
        let md_buf = PieceTable::from_str(md);
        let mut md_model = FoldingModel::new();
        md_model.compute_from_buffer(&md_buf, Some("markdown"));
        assert!(md_model.is_foldable(0));
        assert_eq!(md_model.range_at(0).unwrap().end_line, 5);
        assert!(md_model.is_foldable(2));
        assert_eq!(md_model.range_at(2).unwrap().end_line, 5);
        assert!(md_model.is_foldable(3));
        assert_eq!(md_model.range_at(3).unwrap().end_line, 5);
        assert!(md_model.is_foldable(6));
        assert_eq!(md_model.range_at(6).unwrap().end_line, 7);
    }

    #[test]
    fn test_folding_collapse_visibility_and_navigation() {
        let src = "fn a() {\n    let x = 1;\n    if true {\n        let y = 2;\n    }\n}\nfn b() {}";
        let buf = PieceTable::from_str(src);
        let line_count = buf.line_count();
        let mut model = FoldingModel::new();
        model.compute_from_buffer(&buf, Some("rust"));

        assert_eq!(model.visible_line_count(line_count), 7);
        assert_eq!(model.toggle_fold(2), Some(true));
        // Lines 3 and 4 hidden
        assert!(model.is_line_hidden(3));
        assert!(model.is_line_hidden(4));
        assert!(!model.is_line_hidden(2));
        assert!(!model.is_line_hidden(5));
        assert_eq!(model.visible_lines(line_count), vec![0, 1, 2, 5, 6]);
        assert_eq!(model.next_visible_line(2, line_count), Some(5));
        assert_eq!(model.prev_visible_line(5), Some(2));
        assert_eq!(model.buffer_line_to_visible_index(5, line_count), Some(3));
        assert_eq!(model.buffer_line_to_visible_index(3, line_count), None);
        assert_eq!(model.visible_index_to_buffer_line(3, line_count), Some(5));

        // Collapse outer preserves inner state in `collapsed`
        assert!(model.fold(0));
        assert!(model.is_collapsed(2));
        assert_eq!(model.visible_lines(line_count), vec![0, 6]);

        // Unfolding outer leaves inner collapsed
        assert!(model.unfold(0));
        assert_eq!(model.visible_lines(line_count), vec![0, 1, 2, 5, 6]);

        // unfold_containing(3) unfolds line 2
        assert!(model.unfold_containing(3));
        assert_eq!(model.visible_line_count(line_count), 7);
    }

    #[test]
    fn test_folding_manual_ranges_and_adjust_for_edit() {
        let mut buf = PieceTable::from_str("line 0\nline 1\nline 2\nline 3\n");
        let mut model = FoldingModel::new();
        model.compute_from_buffer(&buf, Some("plaintext"));
        model.add_range(FoldingRange::new(1, 3, FoldingRangeKind::Region));
        assert!(model.fold(1));

        // Insert a newline at line 0 -> range & collapsed line shift from 1..=3 to 2..=4
        buf.insert(0, "inserted\n");
        model.adjust_for_edit(0, 1, &buf, Some("plaintext"));
        assert!(!model.is_collapsed(1));
        assert!(model.is_collapsed(2));
        assert!(model.is_line_hidden(3));
        assert!(model.is_line_hidden(4));

        model.clear_manual_ranges();
        model.recompute(&buf, Some("plaintext"));
        assert!(!model.is_foldable(2));
    }

    #[test]
    fn test_folding_rust_raw_strings_multiline_strings_and_adjacent_else() {
        // 1. Rust raw strings (single-line & multi-line, multiple hashes, plus identifier boundary checks)
        let raw_src = "\
fn f() {
    let a = r\"hello { \\\";
    let b = r#\" { \" } \"#;
    let c = r##\"
        { \"# }
    \"##;
    let r#match = 1;
    for x in [1] {
        let _ = x;
    }
    return;
}";
        let raw_buf = PieceTable::from_str(raw_src);
        let mut raw_model = FoldingModel::new();
        raw_model.compute_from_buffer(&raw_buf, Some("rust"));
        assert_eq!(raw_model.range_at(0).unwrap().end_line, 11);
        assert!(!raw_model.is_foldable(1));
        assert!(!raw_model.is_foldable(2));
        assert!(!raw_model.is_foldable(3));
        assert_eq!(raw_model.range_at(7).unwrap().end_line, 9);

        // 2. Rust multi-line regular "..." string containing braces
        let ml_src = "\
fn g() {
    let s = \"hello {
}\";
    let y = 1;
}";
        let ml_buf = PieceTable::from_str(ml_src);
        let mut ml_model = FoldingModel::new();
        ml_model.compute_from_buffer(&ml_buf, Some("rust"));
        assert_eq!(ml_model.range_at(0).unwrap().end_line, 4);
        assert!(!ml_model.is_foldable(1));

        // 3. Adjacent `} else {` with zero body lines keeps `} else {` visible
        let else_src = "\
if a {
} else {
    b();
}";
        let else_buf = PieceTable::from_str(else_src);
        let mut else_model = FoldingModel::new();
        else_model.compute_from_buffer(&else_buf, Some("rust"));
        assert!(!else_model.is_foldable(0));
        assert!(else_model.is_foldable(1));
        assert_eq!(else_model.range_at(1).unwrap().end_line, 3);
        else_model.fold_all();
        assert!(!else_model.is_line_hidden(0));
        assert!(!else_model.is_line_hidden(1));
        assert!(else_model.is_line_hidden(2));
        assert!(else_model.is_line_hidden(3));
    }
}
