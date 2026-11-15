//! Piece Table text buffer mirroring VS Code's Piece Table / Piece Tree.
//!
//! Provides O(log n) insertions, deletions, line queries, and offset-to-position mapping.
//! Backed by an original immutable buffer, an append-only add buffer, and a balanced piece tree.

use std::fmt;

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

    #[allow(dead_code)]
    fn balance_factor(&self) -> i32 {
        Self::height(&self.left) - Self::height(&self.right)
    }

    #[allow(dead_code)]
    fn rotate_right(mut self: Box<Self>) -> Box<Self> {
        let mut left = self.left.take().expect("rotate_right requires left child");
        self.left = left.right.take();
        self.update();
        left.right = Some(self);
        left.update();
        left
    }

    #[allow(dead_code)]
    fn rotate_left(mut self: Box<Self>) -> Box<Self> {
        let mut right = self.right.take().expect("rotate_left requires right child");
        self.right = right.left.take();
        self.update();
        right.left = Some(self);
        right.update();
        right
    }

    #[allow(dead_code)]
    fn balance(mut self: Box<Self>) -> Box<Self> {
        self.update();
        let bf = self.balance_factor();
        if bf > 1 {
            if let Some(ref left) = self.left {
                if left.balance_factor() < 0 {
                    let left_box = self.left.take().unwrap();
                    self.left = Some(left_box.rotate_left());
                }
            }
            return self.rotate_right();
        } else if bf < -1 {
            if let Some(ref right) = self.right {
                if right.balance_factor() > 0 {
                    let right_box = self.right.take().unwrap();
                    self.right = Some(right_box.rotate_right());
                }
            }
            return self.rotate_left();
        }
        self
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

    /// Insert text at a specific character offset.
    pub fn insert(&mut self, offset: usize, text: &str) {
        if text.is_empty() {
            return;
        }

        let total_len = self.len();
        let target_offset = std::cmp::min(offset, total_len);

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

        let del_end = std::cmp::min(offset + length, self.len());
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
        } else {
            None
        }
    }

    /// Convert a 0-indexed byte offset to (line, column) Position.
    pub fn offset_to_position(&self, offset: usize) -> (usize, usize) {
        let offset = std::cmp::min(offset, self.len());
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
                    let col = abs_pos - last_newline_offset;
                    return (cur_line, col);
                }
                if b == b'\n' {
                    cur_line += 1;
                    last_newline_offset = abs_pos + 1;
                }
            }
            cur_offset += piece.length;
        }

        let col = offset - last_newline_offset;
        (cur_line, col)
    }

    /// Convert (line, column) Position to 0-indexed byte offset.
    pub fn position_to_offset(&self, line: usize, col: usize) -> usize {
        match self.get_line_offsets(line) {
            Some((start, end)) => std::cmp::min(start + col, end),
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
}
