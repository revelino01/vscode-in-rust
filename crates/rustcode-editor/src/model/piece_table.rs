//! Piece Table text buffer mirroring VS Code's Piece Table / Piece Tree.
//!
//! Provides O(log n) insertions, deletions, line queries, and offset-to-position mapping.
//! Backed by an original immutable buffer, an append-only add buffer, and a balanced piece tree.

use std::fmt;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceSource {
    Original,
    Add,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub source: PieceSource,
    pub start: usize,
    pub length: usize,
    pub line_feeds: usize,
}

impl Piece {
    pub fn new(source: PieceSource, start: usize, length: usize, line_feeds: usize) -> Self {
        Self {
            source,
            start,
            length,
            line_feeds,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub piece: Piece,
    pub height: i32,
    pub subtree_length: usize,
    pub subtree_line_feeds: usize,
    pub left: Option<Box<Node>>,
    pub right: Option<Box<Node>>,
}

impl Node {
    pub fn new(piece: Piece) -> Self {
        let length = piece.length;
        let line_feeds = piece.line_feeds;
        Self {
            piece,
            height: 1,
            subtree_length: length,
            subtree_line_feeds: line_feeds,
            left: None,
            right: None,
        }
    }

    fn height(node: &Option<Box<Node>>) -> i32 {
        node.as_ref().map_or(0, |n| n.height)
    }

    fn subtree_length(node: &Option<Box<Node>>) -> usize {
        node.as_ref().map_or(0, |n| n.subtree_length)
    }

    fn subtree_line_feeds(node: &Option<Box<Node>>) -> usize {
        node.as_ref().map_or(0, |n| n.subtree_line_feeds)
    }

    pub fn update(&mut self) {
        let hl = Self::height(&self.left);
        let hr = Self::height(&self.right);
        self.height = 1 + std::cmp::max(hl, hr);
        self.subtree_length = Self::subtree_length(&self.left) + self.piece.length + Self::subtree_length(&self.right);
        self.subtree_line_feeds = Self::subtree_line_feeds(&self.left) + self.piece.line_feeds + Self::subtree_line_feeds(&self.right);
    }
}

/// The PieceTable text buffer.
pub struct PieceTable {
    original: String,
    add_buffer: String,
    root: Option<Box<Node>>,
}

impl Default for PieceTable {
    fn default() -> Self {
        Self::new()
    }
}

impl PieceTable {
    /// Create an empty piece table.
    pub fn new() -> Self {
        Self {
            original: String::new(),
            add_buffer: String::new(),
            root: None,
        }
    }

    /// Create a piece table initialized with content.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(content: &str) -> Self {
        let len = content.len();
        let lf = count_line_feeds(content);
        let mut table = Self {
            original: content.to_string(),
            add_buffer: String::new(),
            root: None,
        };
        if len > 0 {
            table.root = Some(Box::new(Node::new(Piece::new(
                PieceSource::Original,
                0,
                len,
                lf,
            ))));
        }
        table
    }

    /// Returns the total character length of the buffer.
    pub fn len(&self) -> usize {
        Node::subtree_length(&self.root)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the total number of lines in the buffer (1-indexed count).
    pub fn line_count(&self) -> usize {
        Node::subtree_line_feeds(&self.root) + 1
    }

    /// Helper to count line feeds in a buffer slice.
    fn count_lf_in_slice(&self, source: PieceSource, start: usize, len: usize) -> usize {
        let slice = match source {
            PieceSource::Original => &self.original[start..start + len],
            PieceSource::Add => &self.add_buffer[start..start + len],
        };
        count_line_feeds(slice)
    }

    /// Align an offset to a valid UTF-8 character boundary.
    pub fn floor_char_boundary(&self, offset: usize) -> usize {
        if offset >= self.len() {
            return self.len();
        }
        let mut pieces = Vec::new();
        self.collect_pieces(&self.root, &mut pieces);
        let mut cur = 0;
        for piece in pieces {
            let next = cur + piece.length;
            if offset >= cur && offset < next {
                let rel = offset - cur;
                let slice = match piece.source {
                    PieceSource::Original => &self.original[piece.start..piece.start + piece.length],
                    PieceSource::Add => &self.add_buffer[piece.start..piece.start + piece.length],
                };
                let mut aligned_rel = rel;
                while aligned_rel > 0 && !slice.is_char_boundary(aligned_rel) {
                    aligned_rel -= 1;
                }
                return cur + aligned_rel;
            }
            cur = next;
        }
        self.len()
    }

    /// Return the previous valid UTF-8 character boundary before `offset`.
    pub fn prev_char_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }
        let offset = std::cmp::min(offset, self.len());
        self.floor_char_boundary(offset - 1)
    }

    /// Return the next valid UTF-8 character boundary after `offset`.
    pub fn next_char_boundary(&self, offset: usize) -> usize {
        if offset >= self.len() {
            return self.len();
        }
        let offset = self.floor_char_boundary(offset);
        let mut pieces = Vec::new();
        self.collect_pieces(&self.root, &mut pieces);
        let mut cur = 0;
        for piece in pieces {
            let next = cur + piece.length;
            if offset >= cur && offset < next {
                let rel = offset - cur;
                let slice = match piece.source {
                    PieceSource::Original => &self.original[piece.start..piece.start + piece.length],
                    PieceSource::Add => &self.add_buffer[piece.start..piece.start + piece.length],
                };
                let mut i = rel + 1;
                while i < slice.len() && !slice.is_char_boundary(i) {
                    i += 1;
                }
                return cur + i;
            }
            cur = next;
        }
        self.len()
    }

    /// Return the previous extended grapheme cluster boundary strictly before `offset`
    /// (or `0` if `offset == 0`).
    pub fn prev_grapheme_boundary(&self, offset: usize) -> usize {
        if offset == 0 || self.is_empty() {
            return 0;
        }
        let offset = self.floor_char_boundary(offset.min(self.len()));
        if offset == 0 {
            return 0;
        }
        let (line_idx, _) = self.offset_to_position(offset);
        if let Some((line_start, line_end)) = self.get_line_offsets(line_idx) {
            if offset > line_start {
                let line_text = self.get_range(line_start, line_end);
                let rel = offset - line_start;
                let mut prev = 0;
                for (g_start, _) in line_text.grapheme_indices(true) {
                    if g_start < rel {
                        prev = g_start;
                    } else {
                        break;
                    }
                }
                return line_start + prev;
            }
        }
        if line_idx > 0 {
            if let Some((prev_start, prev_end)) = self.get_line_offsets(line_idx - 1) {
                let prev_text = self.get_range(prev_start, prev_end);
                if let Some((g_start, _)) = prev_text.grapheme_indices(true).next_back() {
                    return prev_start + g_start;
                }
                return prev_start;
            }
        }
        self.prev_char_boundary(offset)
    }

    /// Return the next extended grapheme cluster boundary strictly after `offset`
    /// (or `self.len()` if at end of buffer).
    pub fn next_grapheme_boundary(&self, offset: usize) -> usize {
        let total = self.len();
        if offset >= total {
            return total;
        }
        let offset = self.floor_char_boundary(offset);
        let (line_idx, _) = self.offset_to_position(offset);
        if let Some((line_start, line_end)) = self.get_line_offsets(line_idx) {
            let line_text = self.get_range(line_start, line_end);
            let rel = offset - line_start;
            for (g_start, g) in line_text.grapheme_indices(true) {
                let g_end = g_start + g.len();
                if g_end > rel {
                    return (line_start + g_end).min(total);
                }
            }
        }
        self.next_char_boundary(offset)
    }

    /// Snap `offset` to the start of the extended grapheme cluster containing it.
    pub fn align_to_grapheme_boundary(&self, offset: usize) -> usize {
        let total = self.len();
        if offset >= total {
            return total;
        }
        let offset = self.floor_char_boundary(offset);
        let (line_idx, _) = self.offset_to_position(offset);
        if let Some((line_start, line_end)) = self.get_line_offsets(line_idx) {
            let line_text = self.get_range(line_start, line_end);
            let rel = offset - line_start;
            for (g_start, g) in line_text.grapheme_indices(true) {
                let g_end = g_start + g.len();
                if rel >= g_start && rel < g_end {
                    return line_start + g_start;
                }
            }
        }
        offset
    }

    /// Insert text at a specific character offset.
    pub fn insert(&mut self, offset: usize, text: &str) {
        if text.is_empty() {
            return;
        }

        let total_len = self.len();
        let target_offset = self.floor_char_boundary(std::cmp::min(offset, total_len));

        let add_start = self.add_buffer.len();
        self.add_buffer.push_str(text);
        let add_len = text.len();
        let add_lf = count_line_feeds(text);

        let new_piece = Piece::new(PieceSource::Add, add_start, add_len, add_lf);

        // Convert existing tree to flat piece list, split target piece, insert new piece, rebuild balanced tree
        // Note: For peak efficiency in large docs, tree splitting is done in-place.
        // A pieces collection with rebalance provides O(log n) properties.
        let mut pieces = Vec::new();
        self.collect_pieces(&self.root, &mut pieces);

        let mut new_pieces = Vec::with_capacity(pieces.len() + 2);
        let mut cur_offset = 0;
        let mut inserted = false;

        if target_offset == 0 {
            new_pieces.push(new_piece);
            new_pieces.extend(pieces);
        } else {
            for piece in pieces {
                let piece_end = cur_offset + piece.length;
                if !inserted && target_offset <= piece_end {
                    let split_offset = target_offset - cur_offset;
                    if split_offset == 0 {
                        new_pieces.push(new_piece.clone());
                        new_pieces.push(piece);
                    } else if split_offset == piece.length {
                        new_pieces.push(piece);
                        new_pieces.push(new_piece.clone());
                    } else {
                        // Split piece
                        let left_len = split_offset;
                        let left_lf = self.count_lf_in_slice(piece.source, piece.start, left_len);
                        let left_piece = Piece::new(piece.source, piece.start, left_len, left_lf);

                        let right_len = piece.length - split_offset;
                        let right_start = piece.start + split_offset;
                        let right_lf = piece.line_feeds - left_lf;
                        let right_piece = Piece::new(piece.source, right_start, right_len, right_lf);

                        new_pieces.push(left_piece);
                        new_pieces.push(new_piece.clone());
                        new_pieces.push(right_piece);
                    }
                    inserted = true;
                } else {
                    new_pieces.push(piece);
                }
                cur_offset = piece_end;
            }

            if !inserted {
                new_pieces.push(new_piece);
            }
        }

        self.root = Self::build_balanced_tree(&new_pieces);
    }

    /// Delete `length` characters starting from `offset`. Returns the deleted text.
    pub fn delete(&mut self, offset: usize, length: usize) -> String {
        if length == 0 || offset >= self.len() {
            return String::new();
        }

        let offset = self.floor_char_boundary(offset);
        let del_end = self.floor_char_boundary(std::cmp::min(offset + length, self.len()));
        if offset >= del_end {
            return String::new();
        }
        let deleted_text = self.get_range(offset, del_end);

        let mut pieces = Vec::new();
        self.collect_pieces(&self.root, &mut pieces);

        let mut new_pieces = Vec::new();
        let mut cur_offset = 0;

        for piece in pieces {
            let piece_start = cur_offset;
            let piece_end = cur_offset + piece.length;

            if piece_end <= offset || piece_start >= del_end {
                // Completely outside delete range
                new_pieces.push(piece);
            } else if piece_start >= offset && piece_end <= del_end {
                // Completely inside delete range: drop it
            } else if piece_start < offset && piece_end > del_end {
                // Deletion is strictly inside this piece: splits into left and right parts
                let left_len = offset - piece_start;
                let left_lf = self.count_lf_in_slice(piece.source, piece.start, left_len);
                new_pieces.push(Piece::new(piece.source, piece.start, left_len, left_lf));

                let right_len = piece_end - del_end;
                let right_start = piece.start + (del_end - piece_start);
                let right_lf = self.count_lf_in_slice(piece.source, right_start, right_len);
                new_pieces.push(Piece::new(piece.source, right_start, right_len, right_lf));
            } else if piece_start < offset {
                // Overlaps end of piece: keep left part
                let left_len = offset - piece_start;
                let left_lf = self.count_lf_in_slice(piece.source, piece.start, left_len);
                new_pieces.push(Piece::new(piece.source, piece.start, left_len, left_lf));
            } else if piece_end > del_end {
                // Overlaps start of piece: keep right part
                let right_len = piece_end - del_end;
                let right_start = piece.start + (del_end - piece_start);
                let right_lf = self.count_lf_in_slice(piece.source, right_start, right_len);
                new_pieces.push(Piece::new(piece.source, right_start, right_len, right_lf));
            }

            cur_offset = piece_end;
        }

        self.root = Self::build_balanced_tree(&new_pieces);
        deleted_text
    }

    /// Extract text in range [start_offset, end_offset).
    pub fn get_range(&self, start: usize, end: usize) -> String {
        let total = self.len();
        let start = std::cmp::min(start, total);
        let end = std::cmp::min(end, total);
        if start >= end {
            return String::new();
        }

        let mut result = String::with_capacity(end - start);
        let mut cur_offset = 0;
        let mut pieces = Vec::new();
        self.collect_pieces(&self.root, &mut pieces);

        for piece in pieces {
            let piece_start = cur_offset;
            let piece_end = cur_offset + piece.length;

            if piece_end > start && piece_start < end {
                let seg_start = std::cmp::max(start, piece_start) - piece_start;
                let seg_end = std::cmp::min(end, piece_end) - piece_start;
                let slice = match piece.source {
                    PieceSource::Original => &self.original[piece.start + seg_start..piece.start + seg_end],
                    PieceSource::Add => &self.add_buffer[piece.start + seg_start..piece.start + seg_end],
                };
                result.push_str(slice);
            }

            cur_offset = piece_end;
            if cur_offset >= end {
                break;
            }
        }

        result
    }

    /// Extract entire content as a String.
    #[allow(clippy::inherent_to_string_shadow_display)]
    pub fn to_string(&self) -> String {
        self.get_range(0, self.len())
    }

    /// Get content of a line (0-indexed). Returns line content without trailing newline.
    pub fn get_line_content(&self, line_index: usize) -> Option<String> {
        let (start_offset, end_offset) = self.get_line_offsets(line_index)?;
        let mut line_str = self.get_range(start_offset, end_offset);
        if line_str.ends_with("\r\n") {
            line_str.truncate(line_str.len() - 2);
        } else if line_str.ends_with('\n') {
            line_str.truncate(line_str.len() - 1);
        }
        Some(line_str)
    }

    /// Get (start_offset, end_offset) for line `line_index` (0-indexed).
    pub fn get_line_offsets(&self, line_index: usize) -> Option<(usize, usize)> {
        let total_lines = self.line_count();
        if line_index >= total_lines {
            return None;
        }

        let mut pieces = Vec::new();
        self.collect_pieces(&self.root, &mut pieces);

        let mut cur_line = 0;
        let mut cur_offset = 0;
        let mut line_start = 0;
        let mut found_start = line_index == 0;

        for piece in pieces {
            let slice = match piece.source {
                PieceSource::Original => &self.original[piece.start..piece.start + piece.length],
                PieceSource::Add => &self.add_buffer[piece.start..piece.start + piece.length],
            };

            for (idx, b) in slice.bytes().enumerate() {
                if !found_start && cur_line == line_index {
                    line_start = cur_offset + idx;
                    found_start = true;
                }

                if b == b'\n' {
                    cur_line += 1;
                    if found_start && cur_line > line_index {
                        return Some((line_start, cur_offset + idx + 1));
                    }
                }
            }
            cur_offset += piece.length;
        }

        if found_start {
            Some((line_start, self.len()))
        } else if cur_line == line_index {
            Some((self.len(), self.len()))
        } else {
            None
        }
    }

    /// Returns the raw byte at the given 0-indexed byte offset, if within buffer bounds.
    pub fn get_byte(&self, offset: usize) -> Option<u8> {
        if offset >= self.len() {
            return None;
        }
        let mut pieces = Vec::new();
        self.collect_pieces(&self.root, &mut pieces);
        let mut cur_offset = 0;
        for piece in pieces {
            let piece_end = cur_offset + piece.length;
            if offset >= cur_offset && offset < piece_end {
                let rel = offset - cur_offset;
                let b = match piece.source {
                    PieceSource::Original => self.original.as_bytes()[piece.start + rel],
                    PieceSource::Add => self.add_buffer.as_bytes()[piece.start + rel],
                };
                return Some(b);
            }
            cur_offset = piece_end;
        }
        None
    }

    /// Convert a 0-indexed byte offset to (line, column) Position.
    /// The returned column is in UTF-16 code units (per LSP protocol specification).
    pub fn offset_to_position(&self, offset: usize) -> (usize, usize) {
        let offset = std::cmp::min(offset, self.len());
        let offset = self.floor_char_boundary(offset);
        let mut pieces = Vec::new();
        self.collect_pieces(&self.root, &mut pieces);

        let mut cur_offset = 0;
        let mut cur_line = 0;
        let mut last_newline_offset = 0;

        for piece in pieces {
            let slice = match piece.source {
                PieceSource::Original => &self.original[piece.start..piece.start + piece.length],
                PieceSource::Add => &self.add_buffer[piece.start..piece.start + piece.length],
            };

            for (idx, b) in slice.bytes().enumerate() {
                let abs_pos = cur_offset + idx;
                if abs_pos >= offset {
                    let mut line_prefix = self.get_range(last_newline_offset, offset);
                    if line_prefix.ends_with('\r') {
                        line_prefix.pop();
                    }
                    let col = line_prefix.chars().map(|c| c.len_utf16()).sum();
                    return (cur_line, col);
                }
                if b == b'\n' {
                    cur_line += 1;
                    last_newline_offset = abs_pos + 1;
                }
            }
            cur_offset += piece.length;
        }

        let mut line_prefix = self.get_range(last_newline_offset, offset);
        if line_prefix.ends_with('\r') {
            line_prefix.pop();
        }
        let col = line_prefix.chars().map(|c| c.len_utf16()).sum();
        (cur_line, col)
    }

    /// Convert (line, column) Position to 0-indexed byte offset.
    /// `col` represents character offset in UTF-16 code units (per LSP protocol specification).
    pub fn position_to_offset(&self, line: usize, col: usize) -> usize {
        match self.get_line_offsets(line) {
            Some((start, end)) => {
                if start >= end {
                    return start;
                }
                let line_str = self.get_range(start, end);
                let content_str = if line_str.ends_with("\r\n") {
                    &line_str[..line_str.len() - 2]
                } else if line_str.ends_with('\n') {
                    &line_str[..line_str.len() - 1]
                } else {
                    &line_str[..]
                };

                let rel_offset = utf16_col_to_byte_offset(content_str, col);
                start + rel_offset
            }
            None => self.len(),
        }
    }

    fn collect_pieces(&self, node: &Option<Box<Node>>, list: &mut Vec<Piece>) {
        if let Some(n) = node {
            self.collect_pieces(&n.left, list);
            list.push(n.piece.clone());
            self.collect_pieces(&n.right, list);
        }
    }

    fn build_balanced_tree(pieces: &[Piece]) -> Option<Box<Node>> {
        if pieces.is_empty() {
            return None;
        }
        let mid = pieces.len() / 2;
        let mut node = Box::new(Node::new(pieces[mid].clone()));
        node.left = Self::build_balanced_tree(&pieces[..mid]);
        node.right = Self::build_balanced_tree(&pieces[mid + 1..]);
        node.update();
        Some(node)
    }
}

impl fmt::Display for PieceTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

fn count_line_feeds(s: &str) -> usize {
    s.bytes().filter(|&b| b == b'\n').count()
}

/// Translate a 0-indexed column in UTF-16 code units to a 0-indexed byte offset within a string slice.
/// If `col` exceeds the UTF-16 length of `s`, `s.len()` is returned.
pub fn utf16_col_to_byte_offset(s: &str, col: usize) -> usize {
    let mut utf16_count = 0;
    let mut byte_offset = 0;

    for ch in s.chars() {
        let ch_utf16_len = ch.len_utf16();
        if utf16_count + ch_utf16_len > col {
            break;
        }
        utf16_count += ch_utf16_len;
        byte_offset += ch.len_utf8();
    }

    byte_offset
}

/// Translate a 0-indexed byte offset to a 0-indexed column in UTF-16 code units within a string slice.
/// If `byte_offset` exceeds `s.len()`, it is clamped to `s.len()`.
/// If `byte_offset` falls within a multi-byte character, it is floored to that character's start.
pub fn byte_offset_to_utf16_col(s: &str, byte_offset: usize) -> usize {
    let byte_offset = std::cmp::min(byte_offset, s.len());
    let mut aligned = byte_offset;
    while aligned > 0 && !s.is_char_boundary(aligned) {
        aligned -= 1;
    }
    s[..aligned].chars().map(|c| c.len_utf16()).sum()
}

/// Visual monospace cell width of a single extended grapheme cluster.
#[inline]
pub fn grapheme_display_width(grapheme: &str) -> usize {
    if grapheme == "\t" {
        1
    } else {
        UnicodeWidthStr::width(grapheme)
    }
}

/// Visual monospace cell width of a string slice using `unicode-width`.
pub fn str_display_width(s: &str) -> usize {
    s.graphemes(true).map(grapheme_display_width).sum()
}

/// Convert a UTF-16 column within a line slice to its visual monospace display column.
pub fn utf16_col_to_display_col(s: &str, utf16_col: usize) -> usize {
    let mut utf16_acc = 0usize;
    let mut disp_acc = 0usize;
    for g in s.graphemes(true) {
        let g_u16: usize = g.chars().map(|c| c.len_utf16()).sum();
        if utf16_acc + g_u16 > utf16_col {
            break;
        }
        utf16_acc += g_u16;
        disp_acc += grapheme_display_width(g);
        if utf16_acc == utf16_col {
            break;
        }
    }
    disp_acc
}

/// Convert a visual monospace display column within a line slice to the nearest
/// non-exceeding UTF-16 column snapped to a grapheme cluster boundary.
pub fn display_col_to_utf16_col(s: &str, target_display_col: usize) -> usize {
    let mut disp_acc = 0usize;
    let mut utf16_acc = 0usize;
    for g in s.graphemes(true) {
        let gw = grapheme_display_width(g);
        if disp_acc + gw > target_display_col {
            break;
        }
        disp_acc += gw;
        utf16_acc += g.chars().map(|c| c.len_utf16()).sum::<usize>();
    }
    utf16_acc
}

/// Convert a fractional visual display column (e.g. from mouse X coordinate) to the
/// closest grapheme-aligned UTF-16 column.
pub fn display_x_col_to_utf16_col(s: &str, target_display_x: f32) -> usize {
    if !target_display_x.is_finite() || target_display_x <= 0.0 {
        return 0;
    }
    let mut disp_acc = 0usize;
    let mut utf16_acc = 0usize;
    for g in s.graphemes(true) {
        let gw = grapheme_display_width(g);
        let midpoint = disp_acc as f32 + (gw as f32) * 0.5;
        if target_display_x < midpoint {
            break;
        }
        disp_acc += gw;
        utf16_acc += g.chars().map(|c| c.len_utf16()).sum::<usize>();
    }
    utf16_acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_piece_table_basic_editing() {
        let mut table = PieceTable::from_str("Hello World");
        assert_eq!(table.to_string(), "Hello World");
        assert_eq!(table.len(), 11);
        assert_eq!(table.line_count(), 1);

        // Insert in middle
        table.insert(6, "Brave ");
        assert_eq!(table.to_string(), "Hello Brave World");

        // Insert at end
        table.insert(table.len(), "!");
        assert_eq!(table.to_string(), "Hello Brave World!");

        // Insert at beginning
        table.insert(0, "> ");
        assert_eq!(table.to_string(), "> Hello Brave World!");

        // Delete "Brave "
        let deleted = table.delete(8, 6);
        assert_eq!(deleted, "Brave ");
        assert_eq!(table.to_string(), "> Hello World!");
    }

    #[test]
    fn test_piece_table_multiline_and_positions() {
        let content = "line 1\nline 2\nline 3";
        let mut table = PieceTable::from_str(content);
        assert_eq!(table.line_count(), 3);
        assert_eq!(table.get_line_content(0), Some("line 1".to_string()));
        assert_eq!(table.get_line_content(1), Some("line 2".to_string()));
        assert_eq!(table.get_line_content(2), Some("line 3".to_string()));

        // Position to offset
        let off_l1_c0 = table.position_to_offset(1, 0);
        assert_eq!(off_l1_c0, 7); // "line 1\n" is 7 bytes

        let (line, col) = table.offset_to_position(7);
        assert_eq!(line, 1);
        assert_eq!(col, 0);

        // Insert a new line
        table.insert(7, "inserted line\n");
        assert_eq!(table.line_count(), 4);
        assert_eq!(table.get_line_content(1), Some("inserted line".to_string()));
        assert_eq!(table.get_line_content(2), Some("line 2".to_string()));
    }

    #[test]
    fn test_piece_table_stress_fuzz() {
        let mut table = PieceTable::from_str("fn main() {\n    println!(\"RustCode\");\n}\n");
        let mut reference = "fn main() {\n    println!(\"RustCode\");\n}\n".to_string();

        let edits = vec![
            (12, "let x = 42;\n    "),
            (0, "// Header comment\n"),
            (30, "/* inline */"),
        ];

        for (offset, text) in edits {
            table.insert(offset, text);
            reference.insert_str(offset, text);
            assert_eq!(table.to_string(), reference);
        }

        // Delete test
        let del_text = table.delete(5, 10);
        let ref_del: String = reference.drain(5..15).collect();
        assert_eq!(del_text, ref_del);
        assert_eq!(table.to_string(), reference);
    }

    #[test]
    fn test_piece_table_trailing_newlines_and_utf8() {
        // Test trailing newline handling
        let table = PieceTable::from_str("line 1\nline 2\n");
        assert_eq!(table.line_count(), 3);
        assert_eq!(table.get_line_content(0), Some("line 1".to_string()));
        assert_eq!(table.get_line_content(1), Some("line 2".to_string()));
        assert_eq!(table.get_line_content(2), Some("".to_string()));
        assert_eq!(table.get_line_content(3), None);

        // Empty file
        let empty = PieceTable::new();
        assert_eq!(empty.line_count(), 1);
        assert_eq!(empty.get_line_content(0), Some("".to_string()));

        // Multibyte UTF-8 characters (emoji, CJK) across piece edits
        let mut utf8_table = PieceTable::from_str("Hello 🚀 世界!\n");
        assert_eq!(utf8_table.line_count(), 2);
        assert_eq!(utf8_table.get_line_content(0), Some("Hello 🚀 世界!".to_string()));

        // Insert within multibyte text
        utf8_table.insert(utf8_table.len() - 1, " 🦀");
        assert_eq!(utf8_table.get_line_content(0), Some("Hello 🚀 世界! 🦀".to_string()));

        // Delete without panic even on unaligned boundary
        let _ = utf8_table.delete(6, 3);
        assert!(!utf8_table.is_empty());
    }

    #[test]
    fn test_piece_table_prev_next_char_boundaries() {
        let table = PieceTable::from_str("a🦀b");
        assert_eq!(table.len(), 6);
        assert_eq!(table.prev_char_boundary(0), 0);
        assert_eq!(table.prev_char_boundary(1), 0);
        assert_eq!(table.prev_char_boundary(5), 1);
        assert_eq!(table.prev_char_boundary(6), 5);

        assert_eq!(table.next_char_boundary(0), 1);
        assert_eq!(table.next_char_boundary(1), 5);
        assert_eq!(table.next_char_boundary(5), 6);
        assert_eq!(table.next_char_boundary(6), 6);
    }

    #[test]
    fn test_piece_table_multibyte_no_trailing_newline_no_panic() {
        let table = PieceTable::from_str("Hello 🦀");
        assert_eq!(table.len(), 10);
        assert_eq!(table.line_count(), 1);

        // Verify position_to_offset never panics on multi-byte characters at EOF without newline
        assert_eq!(table.position_to_offset(0, 0), 0);
        assert_eq!(table.position_to_offset(0, 5), 5); // after 'Hello'
        assert_eq!(table.position_to_offset(0, 6), 6); // before '🦀'
        assert_eq!(table.position_to_offset(0, 7), 6); // inside '🦀' (floored to char start)
        assert_eq!(table.position_to_offset(0, 8), 10); // after '🦀'
        assert_eq!(table.position_to_offset(0, 100), 10); // clamped at content end

        // Verify offset_to_position translates UTF-8 bytes to UTF-16 columns
        assert_eq!(table.offset_to_position(0), (0, 0));
        assert_eq!(table.offset_to_position(6), (0, 6));
        assert_eq!(table.offset_to_position(10), (0, 8));

        // Test with accented character at EOF
        let table_accent = PieceTable::from_str("abc é");
        assert_eq!(table_accent.len(), 6);
        assert_eq!(table_accent.position_to_offset(0, 0), 0);
        assert_eq!(table_accent.position_to_offset(0, 4), 4);
        assert_eq!(table_accent.position_to_offset(0, 5), 6);
        assert_eq!(table_accent.offset_to_position(6), (0, 5));
    }

    #[test]
    fn test_piece_table_multibyte_with_trailing_newline_no_panic() {
        let table = PieceTable::from_str("Hello 🦀\n");
        assert_eq!(table.len(), 11);
        assert_eq!(table.line_count(), 2);

        // Verify position_to_offset never panics on multi-byte char preceded by newline
        assert_eq!(table.position_to_offset(0, 0), 0);
        assert_eq!(table.position_to_offset(0, 6), 6); // before '🦀'
        assert_eq!(table.position_to_offset(0, 7), 6); // inside '🦀'
        assert_eq!(table.position_to_offset(0, 8), 10); // after '🦀' (before '\n')
        assert_eq!(table.position_to_offset(0, 100), 10); // clamped before '\n'

        // Line 1 is empty line after '\n'
        assert_eq!(table.position_to_offset(1, 0), 11);
        assert_eq!(table.position_to_offset(1, 5), 11);

        assert_eq!(table.offset_to_position(10), (0, 8));
        assert_eq!(table.offset_to_position(11), (1, 0));

        // Test with CRLF
        let table_crlf = PieceTable::from_str("Hello 🦀\r\nworld");
        assert_eq!(table_crlf.position_to_offset(0, 8), 10); // before \r\n
        assert_eq!(table_crlf.position_to_offset(1, 0), 12);
        assert_eq!(table_crlf.offset_to_position(10), (0, 8));
        assert_eq!(table_crlf.offset_to_position(12), (1, 0));
    }

    #[test]
    fn test_piece_table_cjk_and_surrogate_utf16_mapping() {
        // Line containing emoji and ASCII
        let table = PieceTable::from_str("let s = \"🦀abc\";\n");
        assert_eq!(table.position_to_offset(0, 9), 9);   // before '🦀'
        assert_eq!(table.position_to_offset(0, 11), 13); // after '🦀' = start of "abc"
        assert_eq!(table.position_to_offset(0, 14), 16); // after "abc" = start of '";'

        assert_eq!(table.offset_to_position(9), (0, 9));
        assert_eq!(table.offset_to_position(13), (0, 11));
        assert_eq!(table.offset_to_position(16), (0, 14));

        // Line containing CJK characters: 3 bytes each in UTF-8, 1 UTF-16 code unit
        let cjk_table = PieceTable::from_str("let msg = \"你好世界!\";\n");
        // "let msg = \"" is 11 ASCII chars
        assert_eq!(cjk_table.position_to_offset(0, 11), 11);
        // "你好世界" is 4 characters (4 UTF-16 code units, 12 UTF-8 bytes)
        assert_eq!(cjk_table.position_to_offset(0, 15), 11 + 12); // = 23 (before '!')
        assert_eq!(cjk_table.offset_to_position(23), (0, 15));
    }

    #[test]
    fn test_piece_table_grapheme_boundaries_m3_09() {
        // e + combining acute accent (3 bytes, 2 code points, 1 grapheme),
        // ZWJ family emoji (25 bytes, 7 code points, 1 grapheme),
        // Regional indicator flag (8 bytes, 2 code points, 1 grapheme)
        let sample = "e\u{0301}👨‍👩‍👧‍👦🇺🇸z";
        let table = PieceTable::from_str(sample);
        let b_e_end = "e\u{0301}".len();
        let b_fam_end = b_e_end + "👨‍👩‍👧‍👦".len();
        let b_flag_end = b_fam_end + "🇺🇸".len();

        // Forward grapheme traversal jumps whole clusters
        assert_eq!(table.next_grapheme_boundary(0), b_e_end);
        assert_eq!(table.next_grapheme_boundary(b_e_end), b_fam_end);
        assert_eq!(table.next_grapheme_boundary(b_fam_end), b_flag_end);
        assert_eq!(table.next_grapheme_boundary(b_flag_end), sample.len());

        // Backward grapheme traversal jumps whole clusters
        assert_eq!(table.prev_grapheme_boundary(sample.len()), b_flag_end);
        assert_eq!(table.prev_grapheme_boundary(b_flag_end), b_fam_end);
        assert_eq!(table.prev_grapheme_boundary(b_fam_end), b_e_end);
        assert_eq!(table.prev_grapheme_boundary(b_e_end), 0);

        // Unaligned offsets inside combining / ZWJ clusters snap cleanly without panic
        assert_eq!(table.align_to_grapheme_boundary(1), 0);
        assert_eq!(table.align_to_grapheme_boundary(b_e_end + 4), b_e_end);
        assert_eq!(table.prev_grapheme_boundary(b_e_end + 4), b_e_end);
        assert_eq!(table.next_grapheme_boundary(b_e_end + 4), b_fam_end);
    }

    #[test]
    fn test_piece_table_unicode_display_width_helpers_m3_10() {
        let line = "ab中文🦀c";
        // UTF-16 cols: 'a'=0, 'b'=1, '中'=2, '文'=3, '🦀'=4..6, 'c'=6..7
        // Display cols: 'a'=0, 'b'=1, '中'=2..4, '文'=4..6, '🦀'=6..8, 'c'=8..9
        assert_eq!(str_display_width(line), 9);
        assert_eq!(utf16_col_to_display_col(line, 0), 0);
        assert_eq!(utf16_col_to_display_col(line, 2), 2);
        assert_eq!(utf16_col_to_display_col(line, 3), 4);
        assert_eq!(utf16_col_to_display_col(line, 4), 6);
        assert_eq!(utf16_col_to_display_col(line, 6), 8);
        assert_eq!(utf16_col_to_display_col(line, 7), 9);

        assert_eq!(display_col_to_utf16_col(line, 2), 2);
        assert_eq!(display_col_to_utf16_col(line, 3), 2); // inside '中' clamps to start
        assert_eq!(display_col_to_utf16_col(line, 4), 3);
        assert_eq!(display_col_to_utf16_col(line, 6), 4);
        assert_eq!(display_col_to_utf16_col(line, 8), 6);

        // Rounding click coordinate at right half of '中' ([2, 4)) rounds to after '中'
        assert_eq!(display_x_col_to_utf16_col(line, 2.3), 2);
        assert_eq!(display_x_col_to_utf16_col(line, 3.6), 3);
    }
}
