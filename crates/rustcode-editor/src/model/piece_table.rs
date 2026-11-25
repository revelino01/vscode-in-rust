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

/// The PieceTable text buffer backed by an O(log n) AVL Piece Tree and sorted LF indices.
pub struct PieceTable {
    original: String,
    add_buffer: String,
    orig_lf_offsets: Vec<usize>,
    add_lf_offsets: Vec<usize>,
    root: Option<Box<Node>>,
}

impl Default for PieceTable {
    fn default() -> Self {
        Self::new()
    }
}

fn collect_lf_byte_offsets(s: &str, base: usize, out: &mut Vec<usize>) {
    for (idx, b) in s.bytes().enumerate() {
        if b == b'\n' {
            out.push(base + idx);
        }
    }
}

impl Node {
    fn balance_factor(&self) -> i32 {
        Self::height(&self.left) - Self::height(&self.right)
    }

    fn rotate_left(mut node: Box<Node>) -> Box<Node> {
        let mut right = node.right.take().expect("rotate_left requires right child");
        node.right = right.left.take();
        node.update();
        right.left = Some(node);
        right.update();
        right
    }

    fn rotate_right(mut node: Box<Node>) -> Box<Node> {
        let mut left = node.left.take().expect("rotate_right requires left child");
        node.left = left.right.take();
        node.update();
        left.right = Some(node);
        left.update();
        left
    }

    fn rebalance(mut node: Box<Node>) -> Box<Node> {
        node.update();
        let bf = node.balance_factor();
        if bf > 1 {
            if let Some(left) = node.left.take() {
                if left.balance_factor() < 0 {
                    node.left = Some(Self::rotate_left(left));
                } else {
                    node.left = Some(left);
                }
            }
            return Self::rotate_right(node);
        }
        if bf < -1 {
            if let Some(right) = node.right.take() {
                if right.balance_factor() > 0 {
                    node.right = Some(Self::rotate_right(right));
                } else {
                    node.right = Some(right);
                }
            }
            return Self::rotate_left(node);
        }
        node
    }

    fn join_avl(
        left: Option<Box<Node>>,
        mut mid: Box<Node>,
        right: Option<Box<Node>>,
    ) -> Box<Node> {
        let hl = Self::height(&left);
        let hr = Self::height(&right);
        if hl > hr + 1 {
            let mut l = left.expect("left height > right height + 1");
            let l_right = l.right.take();
            l.right = Some(Self::join_avl(l_right, mid, right));
            Self::rebalance(l)
        } else if hr > hl + 1 {
            let mut r = right.expect("right height > left height + 1");
            let r_left = r.left.take();
            r.left = Some(Self::join_avl(left, mid, r_left));
            Self::rebalance(r)
        } else {
            mid.left = left;
            mid.right = right;
            mid.update();
            mid
        }
    }

    fn pop_max(mut node: Box<Node>) -> (Option<Box<Node>>, Box<Node>) {
        if let Some(right) = node.right.take() {
            let (new_right, max_node) = Self::pop_max(right);
            node.right = new_right;
            (Some(Self::rebalance(node)), max_node)
        } else {
            let left = node.left.take();
            node.update();
            (left, node)
        }
    }

    fn concat_avl(left: Option<Box<Node>>, right: Option<Box<Node>>) -> Option<Box<Node>> {
        match (left, right) {
            (None, r) => r,
            (l, None) => l,
            (Some(l), Some(r)) => {
                let (rest_left, mid) = Self::pop_max(l);
                Some(Self::join_avl(rest_left, mid, Some(r)))
            }
        }
    }

    fn try_append_rightmost_add(
        node: &mut Box<Node>,
        expected_start: usize,
        extra_len: usize,
        extra_lf: usize,
    ) -> bool {
        let appended = if let Some(right) = node.right.as_mut() {
            Self::try_append_rightmost_add(right, expected_start, extra_len, extra_lf)
        } else if node.piece.source == PieceSource::Add
            && node.piece.start + node.piece.length == expected_start
        {
            node.piece.length += extra_len;
            node.piece.line_feeds += extra_lf;
            true
        } else {
            false
        };
        if appended {
            node.update();
        }
        appended
    }
}

impl PieceTable {
    /// Create an empty piece table.
    pub fn new() -> Self {
        Self {
            original: String::new(),
            add_buffer: String::new(),
            orig_lf_offsets: Vec::new(),
            add_lf_offsets: Vec::new(),
            root: None,
        }
    }

    /// Create a piece table initialized with content in O(n) single pass.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(content: &str) -> Self {
        let len = content.len();
        let mut orig_lf_offsets = Vec::with_capacity(len / 32 + 1);
        collect_lf_byte_offsets(content, 0, &mut orig_lf_offsets);
        let lf = orig_lf_offsets.len();
        let mut table = Self {
            original: content.to_string(),
            add_buffer: String::new(),
            orig_lf_offsets,
            add_lf_offsets: Vec::new(),
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

    pub fn tree_height(&self) -> usize {
        Node::height(&self.root).max(0) as usize
    }

    pub fn piece_count(&self) -> usize {
        fn count(n: &Option<Box<Node>>) -> usize {
            n.as_ref()
                .map_or(0, |x| 1 + count(&x.left) + count(&x.right))
        }
        count(&self.root)
    }

    /// Returns the total character length (in bytes) of the buffer in O(1).
    pub fn len(&self) -> usize {
        Node::subtree_length(&self.root)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the total number of lines in the buffer (1-indexed count) in O(1).
    pub fn line_count(&self) -> usize {
        Node::subtree_line_feeds(&self.root) + 1
    }

    /// O(log L) line feed count inside a slice using sorted LF byte indices.
    fn count_lf_in_slice(&self, source: PieceSource, start: usize, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        let offs = match source {
            PieceSource::Original => &self.orig_lf_offsets,
            PieceSource::Add => &self.add_lf_offsets,
        };
        let end = start + len;
        let lo = offs.partition_point(|&o| o < start);
        let hi = offs.partition_point(|&o| o < end);
        hi - lo
    }

    /// O(log L) relative offset within `[start..]` of the 0-indexed `k`-th line feed.
    fn nth_lf_in_slice(&self, source: PieceSource, start: usize, k: usize) -> Option<usize> {
        let offs = match source {
            PieceSource::Original => &self.orig_lf_offsets,
            PieceSource::Add => &self.add_lf_offsets,
        };
        let lo = offs.partition_point(|&o| o < start);
        offs.get(lo + k).map(|&o| o - start)
    }

    /// O(log n + log L) split of an AVL subtree into `(left [0..offset), right [offset..])`.
    fn split_by_offset(
        &self,
        node: Option<Box<Node>>,
        offset: usize,
    ) -> (Option<Box<Node>>, Option<Box<Node>>) {
        let Some(mut cur) = node else {
            return (None, None);
        };
        if offset == 0 {
            return (None, Some(cur));
        }
        if offset >= cur.subtree_length {
            return (Some(cur), None);
        }

        let left_len = Node::subtree_length(&cur.left);
        if offset <= left_len {
            let cur_left = cur.left.take();
            let cur_right = cur.right.take();
            let (l1, l2) = self.split_by_offset(cur_left, offset);
            let right_tree = Node::join_avl(l2, cur, cur_right);
            (l1, Some(right_tree))
        } else if offset >= left_len + cur.piece.length {
            let cur_left = cur.left.take();
            let cur_right = cur.right.take();
            let (r1, r2) = self.split_by_offset(cur_right, offset - left_len - cur.piece.length);
            let left_tree = Node::join_avl(cur_left, cur, r1);
            (Some(left_tree), r2)
        } else {
            let split_at = offset - left_len;
            let p = cur.piece;
            let left_lf = self.count_lf_in_slice(p.source, p.start, split_at);
            let right_lf = p.line_feeds - left_lf;
            let left_node = Box::new(Node::new(Piece::new(p.source, p.start, split_at, left_lf)));
            let right_node = Box::new(Node::new(Piece::new(
                p.source,
                p.start + split_at,
                p.length - split_at,
                right_lf,
            )));
            let left_tree = Node::join_avl(cur.left.take(), left_node, None);
            let right_tree = Node::join_avl(None, right_node, cur.right.take());
            (Some(left_tree), Some(right_tree))
        }
    }

    fn find_piece_at_offset(&self, offset: usize) -> Option<(usize, Piece)> {
        let mut cur = &self.root;
        let mut acc = 0usize;
        while let Some(n) = cur {
            let left_len = Node::subtree_length(&n.left);
            if offset < acc + left_len {
                cur = &n.left;
            } else if offset < acc + left_len + n.piece.length {
                return Some((acc + left_len, n.piece.clone()));
            } else {
                acc += left_len + n.piece.length;
                cur = &n.right;
            }
        }
        None
    }

    /// Align an offset to a valid UTF-8 character boundary in O(log n).
    pub fn floor_char_boundary(&self, offset: usize) -> usize {
        if offset >= self.len() {
            return self.len();
        }
        if let Some((piece_doc_start, piece)) = self.find_piece_at_offset(offset) {
            let rel = offset - piece_doc_start;
            let slice = match piece.source {
                PieceSource::Original => &self.original[piece.start..piece.start + piece.length],
                PieceSource::Add => &self.add_buffer[piece.start..piece.start + piece.length],
            };
            let mut aligned_rel = rel;
            while aligned_rel > 0 && !slice.is_char_boundary(aligned_rel) {
                aligned_rel -= 1;
            }
            return piece_doc_start + aligned_rel;
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

    /// Return the next valid UTF-8 character boundary after `offset` in O(log n).
    pub fn next_char_boundary(&self, offset: usize) -> usize {
        if offset >= self.len() {
            return self.len();
        }
        let offset = self.floor_char_boundary(offset);
        if let Some((piece_doc_start, piece)) = self.find_piece_at_offset(offset) {
            let rel = offset - piece_doc_start;
            let slice = match piece.source {
                PieceSource::Original => &self.original[piece.start..piece.start + piece.length],
                PieceSource::Add => &self.add_buffer[piece.start..piece.start + piece.length],
            };
            let mut i = rel + 1;
            while i < slice.len() && !slice.is_char_boundary(i) {
                i += 1;
            }
            return piece_doc_start + i;
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

    /// Insert text at a specific byte offset in O(log n) amortized time.
    pub fn insert(&mut self, offset: usize, text: &str) {
        if text.is_empty() {
            return;
        }

        let total_len = self.len();
        let target_offset = self.floor_char_boundary(std::cmp::min(offset, total_len));

        let add_start = self.add_buffer.len();
        let before_lf_len = self.add_lf_offsets.len();
        collect_lf_byte_offsets(text, add_start, &mut self.add_lf_offsets);
        self.add_buffer.push_str(text);
        let add_len = text.len();
        let add_lf = self.add_lf_offsets.len() - before_lf_len;

        let root = self.root.take();
        let (mut left, right) = self.split_by_offset(root, target_offset);
        let coalesced = left
            .as_mut()
            .is_some_and(|l| Node::try_append_rightmost_add(l, add_start, add_len, add_lf));
        if !coalesced {
            let mid = Box::new(Node::new(Piece::new(
                PieceSource::Add,
                add_start,
                add_len,
                add_lf,
            )));
            left = Some(Node::join_avl(left, mid, None));
        }
        self.root = Node::concat_avl(left, right);
    }

    /// Delete `length` bytes starting from `offset` in O(log n + length) time. Returns the deleted text.
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

        let root = self.root.take();
        let (left, mid_and_right) = self.split_by_offset(root, offset);
        let (_, right) = self.split_by_offset(mid_and_right, del_end - offset);
        self.root = Node::concat_avl(left, right);
        deleted_text
    }

    /// Extract text in range [start_offset, end_offset) in O(log n + output_len).
    pub fn get_range(&self, start: usize, end: usize) -> String {
        let total = self.len();
        let start = std::cmp::min(start, total);
        let end = std::cmp::min(end, total);
        if start >= end {
            return String::new();
        }

        let mut result = String::with_capacity(end - start);
        self.collect_range_recursive(&self.root, 0, start, end, &mut result);
        result
    }

    fn collect_range_recursive(
        &self,
        node: &Option<Box<Node>>,
        node_start: usize,
        req_start: usize,
        req_end: usize,
        out: &mut String,
    ) {
        let Some(n) = node else {
            return;
        };
        if node_start >= req_end || node_start + n.subtree_length <= req_start {
            return;
        }
        let left_len = Node::subtree_length(&n.left);
        let mid_start = node_start + left_len;
        let mid_end = mid_start + n.piece.length;

        if req_start < mid_start {
            self.collect_range_recursive(&n.left, node_start, req_start, req_end, out);
        }
        if mid_end > req_start && mid_start < req_end {
            let seg_start = req_start.max(mid_start) - mid_start;
            let seg_end = req_end.min(mid_end) - mid_start;
            let slice = match n.piece.source {
                PieceSource::Original => {
                    &self.original[n.piece.start + seg_start..n.piece.start + seg_end]
                }
                PieceSource::Add => {
                    &self.add_buffer[n.piece.start + seg_start..n.piece.start + seg_end]
                }
            };
            out.push_str(slice);
        }
        if req_end > mid_end {
            self.collect_range_recursive(&n.right, mid_end, req_start, req_end, out);
        }
    }

    /// Extract entire content as a String.
    #[allow(clippy::inherent_to_string_shadow_display)]
    pub fn to_string(&self) -> String {
        self.get_range(0, self.len())
    }

    /// Get content of a line (0-indexed). Returns line content without trailing newline in O(log n + line_len).
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

    /// Document byte offset of the 0-indexed `lf_idx`-th `\n` character in O(log n + log L).
    fn nth_lf_doc_offset(&self, mut lf_idx: usize) -> Option<usize> {
        let mut cur = &self.root;
        let mut acc_bytes = 0usize;
        while let Some(n) = cur {
            let left_lfs = Node::subtree_line_feeds(&n.left);
            let left_bytes = Node::subtree_length(&n.left);
            if lf_idx < left_lfs {
                cur = &n.left;
            } else if lf_idx < left_lfs + n.piece.line_feeds {
                let rel = self.nth_lf_in_slice(n.piece.source, n.piece.start, lf_idx - left_lfs)?;
                return Some(acc_bytes + left_bytes + rel);
            } else {
                lf_idx -= left_lfs + n.piece.line_feeds;
                acc_bytes += left_bytes + n.piece.length;
                cur = &n.right;
            }
        }
        None
    }

    /// Get `(start_offset, end_offset)` for line `line_index` (0-indexed) in O(log n + log L).
    pub fn get_line_offsets(&self, line_index: usize) -> Option<(usize, usize)> {
        let total_lines = self.line_count();
        if line_index >= total_lines {
            return None;
        }
        let line_start = if line_index == 0 {
            0
        } else {
            self.nth_lf_doc_offset(line_index - 1)? + 1
        };
        let line_end = if line_index + 1 < total_lines {
            self.nth_lf_doc_offset(line_index)? + 1
        } else {
            self.len()
        };
        Some((line_start, line_end))
    }

    /// Returns the raw byte at the given 0-indexed byte offset in O(log n).
    pub fn get_byte(&self, mut offset: usize) -> Option<u8> {
        if offset >= self.len() {
            return None;
        }
        let mut cur = &self.root;
        while let Some(n) = cur {
            let left_len = Node::subtree_length(&n.left);
            if offset < left_len {
                cur = &n.left;
            } else if offset < left_len + n.piece.length {
                let rel = offset - left_len;
                let b = match n.piece.source {
                    PieceSource::Original => self.original.as_bytes()[n.piece.start + rel],
                    PieceSource::Add => self.add_buffer.as_bytes()[n.piece.start + rel],
                };
                return Some(b);
            } else {
                offset -= left_len + n.piece.length;
                cur = &n.right;
            }
        }
        None
    }

    /// Count line feeds strictly before document byte `offset` in O(log n + log L).
    fn count_lfs_before_offset(&self, mut offset: usize) -> usize {
        let mut cur = &self.root;
        let mut lfs = 0usize;
        while let Some(n) = cur {
            if offset == 0 {
                break;
            }
            let left_len = Node::subtree_length(&n.left);
            if offset <= left_len {
                cur = &n.left;
            } else {
                lfs += Node::subtree_line_feeds(&n.left);
                let into_piece = offset - left_len;
                if into_piece <= n.piece.length {
                    lfs += self.count_lf_in_slice(n.piece.source, n.piece.start, into_piece);
                    if into_piece < n.piece.length {
                        break;
                    }
                    offset = 0;
                    cur = &n.right;
                } else {
                    lfs += n.piece.line_feeds;
                    offset = into_piece - n.piece.length;
                    cur = &n.right;
                }
            }
        }
        lfs
    }

    /// Convert a 0-indexed byte offset to (line, column) Position in O(log n + line_len).
    /// The returned column is in UTF-16 code units (per LSP protocol specification).
    pub fn offset_to_position(&self, offset: usize) -> (usize, usize) {
        let offset = std::cmp::min(offset, self.len());
        let offset = self.floor_char_boundary(offset);
        if offset == 0 {
            return (0, 0);
        }
        let cur_line = self.count_lfs_before_offset(offset);
        let line_start = if cur_line == 0 {
            0
        } else {
            self.nth_lf_doc_offset(cur_line - 1).map_or(0, |lf| lf + 1)
        };
        let mut line_prefix = self.get_range(line_start, offset);
        if line_prefix.ends_with('\r') {
            line_prefix.pop();
        }
        let col = line_prefix.chars().map(|c| c.len_utf16()).sum();
        (cur_line, col)
    }

    /// Convert (line, column) Position to 0-indexed byte offset in O(log n + line_len).
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
}

impl fmt::Display for PieceTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
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

    #[test]
    fn test_avl_piece_table_height_invariant_and_random_fuzz() {
        let mut table = PieceTable::from_str("initial seed content\nsecond line\n");
        let mut reference = "initial seed content\nsecond line\n".to_string();

        // Deterministic PRNG (SplitMix64) driving 2,500 scattered inserts & deletes
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let next_rand = |s: &mut u64| -> u64 {
            *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = *s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };

        for step in 0..2_500 {
            let cur_len = table.len();
            if cur_len > 20 && (next_rand(&mut state) % 3 == 0) {
                let off = (next_rand(&mut state) as usize) % cur_len;
                let max_del = (cur_len - off).clamp(1, 12);
                let del_len = ((next_rand(&mut state) as usize) % max_del).max(1);
                let got = table.delete(off, del_len);
                let expected: String = reference.drain(off..off + del_len).collect();
                assert_eq!(got, expected, "delete mismatch at step {}", step);
            } else {
                let off = if cur_len == 0 {
                    0
                } else {
                    (next_rand(&mut state) as usize) % (cur_len + 1)
                };
                let ins = if step % 7 == 0 {
                    "\nln\n"
                } else {
                    "abc_"
                };
                table.insert(off, ins);
                reference.insert_str(off, ins);
            }

            if step % 250 == 0 {
                assert_eq!(table.len(), reference.len());
                assert_eq!(table.to_string(), reference);
                let pcs = table.piece_count().max(1);
                let max_avl_h = (1.45 * ((pcs as f64) + 2.0).log2()).ceil() as usize + 1;
                assert!(
                    table.tree_height() <= max_avl_h,
                    "AVL height {} exceeded bound {} for {} pieces",
                    table.tree_height(),
                    max_avl_h,
                    pcs
                );
            }
        }
        assert_eq!(table.to_string(), reference);
        let sample_line = table.line_count() / 2;
        let expected_line = reference.split('\n').nth(sample_line).unwrap_or("");
        assert_eq!(table.get_line_content(sample_line).as_deref(), Some(expected_line));
    }

    #[test]
    fn test_avl_piece_table_large_buffer_ologn_latency() {
        // ~6 MB synthetic source buffer (100,000 lines)
        let line = "pub fn compute_metric(alpha: usize, beta: usize) -> usize { alpha + beta }\n";
        let big = line.repeat(85_000);
        let mut table = PieceTable::from_str(&big);
        assert_eq!(table.line_count(), 85_001);

        let mid_off = table.position_to_offset(42_500, 10);
        let t0 = std::time::Instant::now();
        table.insert(mid_off, "/* mid-edit */");
        let got_line = table.get_line_content(42_500).unwrap();
        let elapsed = t0.elapsed();

        assert!(got_line.contains("/* mid-edit */"));
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "O(log n) edit + line lookup took {:?} (expected < 50ms)",
            elapsed
        );
    }
}
