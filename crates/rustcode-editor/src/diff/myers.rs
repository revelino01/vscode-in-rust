//! Pure-Rust Myers Diff Algorithm.
//!
//! Implements Eugene W. Myers' O(ND) difference algorithm ("An O(ND) Difference Algorithm
//! and Its Variations", 1986). Calculates shortest edit scripts, line additions, deletions,
//! and modifications, producing inline and side-by-side display lists.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffOpKind {
    Equal,
    Insert,
    Delete,
    Modify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffChange {
    pub kind: DiffOpKind,
    pub old_line: Option<usize>, // 1-based line number in original
    pub new_line: Option<usize>, // 1-based line number in modified
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffViewMode {
    SideBySide,
    Inline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineDiffRow {
    pub kind: DiffOpKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
    pub prefix: char,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffCell {
    pub line_num: Option<usize>,
    pub text: String,
    pub kind: DiffOpKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideBySideDiffRow {
    pub left: Option<DiffCell>,
    pub right: Option<DiffCell>,
    pub kind: DiffOpKind,
}

/// Computes the raw edit script between two slices of strings using Myers O(ND) algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOp<T> {
    Keep(T),
    Delete(T),
    Insert(T),
}

pub fn myers_diff<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<EditOp<&'a str>> {
    let n = a.len();
    let m = b.len();

    if n == 0 && m == 0 {
        return Vec::new();
    }

    if n == 0 {
        return b.iter().map(|&s| EditOp::Insert(s)).collect();
    }

    if m == 0 {
        return a.iter().map(|&s| EditOp::Delete(s)).collect();
    }

    let max_d = n + m;
    let offset = max_d as isize;
    let v_size = 2 * max_d + 1;

    let mut v = vec![0isize; v_size];
    let mut history: Vec<Vec<isize>> = Vec::with_capacity(max_d + 1);

    // Initial state: D = 0
    let mut d_found = None;

    'outer: for d in 0..=max_d {
        history.push(v.clone());
        let d_isize = d as isize;

        for k in (-d_isize..=d_isize).step_by(2) {
            let k_idx = (k + offset) as usize;

            let mut x = if k == -d_isize || (k != d_isize && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize]) {
                v[(k + 1 + offset) as usize]
            } else {
                v[(k - 1 + offset) as usize] + 1
            };

            let mut y = x - k;

            while (x as usize) < n && (y as usize) < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }

            v[k_idx] = x;

            if (x as usize) >= n && (y as usize) >= m {
                d_found = Some(d);
                break 'outer;
            }
        }
    }

    // Backtrack to build the edit script
    let mut edits = Vec::new();
    let mut x = n as isize;
    let mut y = m as isize;

    let d_end = d_found.unwrap_or(max_d);

    for d in (1..=d_end).rev() {
        let v_prev = &history[d];
        let k = x - y;
        let d_isize = d as isize;

        let prev_k = if k == -d_isize || (k != d_isize && v_prev[(k - 1 + offset) as usize] < v_prev[(k + 1 + offset) as usize]) {
            k + 1
        } else {
            k - 1
        };

        let prev_x = v_prev[(prev_k + offset) as usize];
        let prev_y = prev_x - prev_k;

        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            edits.push(EditOp::Keep(a[x as usize]));
        }

        if d > 0 {
            if x == prev_x {
                y -= 1;
                edits.push(EditOp::Insert(b[y as usize]));
            } else {
                x -= 1;
                edits.push(EditOp::Delete(a[x as usize]));
            }
        }
    }

    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        edits.push(EditOp::Keep(a[x as usize]));
    }
    while x > 0 {
        x -= 1;
        edits.push(EditOp::Delete(a[x as usize]));
    }
    while y > 0 {
        y -= 1;
        edits.push(EditOp::Insert(b[y as usize]));
    }

    edits.reverse();
    edits
}

/// Diff model representing the complete diff between original and modified text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffModel {
    pub changes: Vec<DiffChange>,
    pub inline_rows: Vec<InlineDiffRow>,
    pub side_by_side_rows: Vec<SideBySideDiffRow>,
    pub additions: usize,
    pub deletions: usize,
    pub modifications: usize,
}

impl DiffModel {
    pub fn compute(original: &str, modified: &str) -> Self {
        let orig_lines: Vec<&str> = if original.is_empty() {
            Vec::new()
        } else {
            original.lines().collect()
        };

        let mod_lines: Vec<&str> = if modified.is_empty() {
            Vec::new()
        } else {
            modified.lines().collect()
        };

        let raw_edits = myers_diff(&orig_lines, &mod_lines);

        let mut changes = Vec::new();
        let mut old_line = 1;
        let mut new_line = 1;

        let mut additions = 0;
        let mut deletions = 0;
        let mut modifications = 0;

        // Convert raw edits to DiffChanges with line numbers
        for op in raw_edits {
            match op {
                EditOp::Keep(text) => {
                    changes.push(DiffChange {
                        kind: DiffOpKind::Equal,
                        old_line: Some(old_line),
                        new_line: Some(new_line),
                        text: text.to_string(),
                    });
                    old_line += 1;
                    new_line += 1;
                }
                EditOp::Delete(text) => {
                    changes.push(DiffChange {
                        kind: DiffOpKind::Delete,
                        old_line: Some(old_line),
                        new_line: None,
                        text: text.to_string(),
                    });
                    old_line += 1;
                    deletions += 1;
                }
                EditOp::Insert(text) => {
                    changes.push(DiffChange {
                        kind: DiffOpKind::Insert,
                        old_line: None,
                        new_line: Some(new_line),
                        text: text.to_string(),
                    });
                    new_line += 1;
                    additions += 1;
                }
            }
        }

        // Build inline display rows
        let mut inline_rows = Vec::with_capacity(changes.len());
        for c in &changes {
            let (prefix, kind) = match c.kind {
                DiffOpKind::Equal => (' ', DiffOpKind::Equal),
                DiffOpKind::Insert => ('+', DiffOpKind::Insert),
                DiffOpKind::Delete => ('-', DiffOpKind::Delete),
                DiffOpKind::Modify => ('~', DiffOpKind::Modify),
            };
            inline_rows.push(InlineDiffRow {
                kind,
                old_line: c.old_line,
                new_line: c.new_line,
                text: c.text.clone(),
                prefix,
            });
        }

        // Build side-by-side display rows with modification pairing
        let mut side_by_side_rows = Vec::new();
        let mut i = 0;
        while i < changes.len() {
            match changes[i].kind {
                DiffOpKind::Equal => {
                    side_by_side_rows.push(SideBySideDiffRow {
                        left: Some(DiffCell {
                            line_num: changes[i].old_line,
                            text: changes[i].text.clone(),
                            kind: DiffOpKind::Equal,
                        }),
                        right: Some(DiffCell {
                            line_num: changes[i].new_line,
                            text: changes[i].text.clone(),
                            kind: DiffOpKind::Equal,
                        }),
                        kind: DiffOpKind::Equal,
                    });
                    i += 1;
                }
                DiffOpKind::Delete | DiffOpKind::Insert => {
                    // Group contiguous deletions and insertions into paired modifications
                    let mut del_chunk = Vec::new();
                    let mut ins_chunk = Vec::new();
                    while i < changes.len() && (changes[i].kind == DiffOpKind::Delete || changes[i].kind == DiffOpKind::Insert) {
                        if changes[i].kind == DiffOpKind::Delete {
                            del_chunk.push(&changes[i]);
                        } else {
                            ins_chunk.push(&changes[i]);
                        }
                        i += 1;
                    }

                    let max_chunk = del_chunk.len().max(ins_chunk.len());
                    for j in 0..max_chunk {
                        let left = del_chunk.get(j).map(|d| DiffCell {
                            line_num: d.old_line,
                            text: d.text.clone(),
                            kind: if ins_chunk.get(j).is_some() {
                                DiffOpKind::Modify
                            } else {
                                DiffOpKind::Delete
                            },
                        });
                        let right = ins_chunk.get(j).map(|ins| DiffCell {
                            line_num: ins.new_line,
                            text: ins.text.clone(),
                            kind: if del_chunk.get(j).is_some() {
                                DiffOpKind::Modify
                            } else {
                                DiffOpKind::Insert
                            },
                        });

                        let row_kind = match (left.as_ref(), right.as_ref()) {
                            (Some(_), Some(_)) => {
                                modifications += 1;
                                DiffOpKind::Modify
                            }
                            (Some(_), None) => DiffOpKind::Delete,
                            (None, Some(_)) => DiffOpKind::Insert,
                            (None, None) => DiffOpKind::Equal,
                        };

                        side_by_side_rows.push(SideBySideDiffRow {
                            left,
                            right,
                            kind: row_kind,
                        });
                    }
                }
                _ => {
                    i += 1;
                }
            }
        }

        Self {
            changes,
            inline_rows,
            side_by_side_rows,
            additions,
            deletions,
            modifications,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_myers_diff_empty() {
        let diff = DiffModel::compute("", "");
        assert!(diff.changes.is_empty());
        assert_eq!(diff.additions, 0);
        assert_eq!(diff.deletions, 0);
    }

    #[test]
    fn test_myers_diff_pure_insertions() {
        let diff = DiffModel::compute("", "alpha\nbeta\ngamma");
        assert_eq!(diff.additions, 3);
        assert_eq!(diff.deletions, 0);
        assert_eq!(diff.inline_rows.len(), 3);
        assert_eq!(diff.inline_rows[0].prefix, '+');
        assert_eq!(diff.inline_rows[0].text, "alpha");
        assert_eq!(diff.inline_rows[0].new_line, Some(1));
    }

    #[test]
    fn test_myers_diff_pure_deletions() {
        let diff = DiffModel::compute("line 1\nline 2", "");
        assert_eq!(diff.deletions, 2);
        assert_eq!(diff.additions, 0);
        assert_eq!(diff.inline_rows.len(), 2);
        assert_eq!(diff.inline_rows[0].prefix, '-');
        assert_eq!(diff.inline_rows[0].text, "line 1");
        assert_eq!(diff.inline_rows[0].old_line, Some(1));
    }

    #[test]
    fn test_myers_diff_identical() {
        let diff = DiffModel::compute("hello\nworld", "hello\nworld");
        assert_eq!(diff.additions, 0);
        assert_eq!(diff.deletions, 0);
        assert_eq!(diff.inline_rows.len(), 2);
        assert_eq!(diff.inline_rows[0].prefix, ' ');
        assert_eq!(diff.inline_rows[1].prefix, ' ');
        assert_eq!(diff.side_by_side_rows.len(), 2);
        assert_eq!(diff.side_by_side_rows[0].left.as_ref().unwrap().text, "hello");
        assert_eq!(diff.side_by_side_rows[0].right.as_ref().unwrap().text, "hello");
    }

    #[test]
    fn test_myers_diff_modifications_and_pairing() {
        let old_text = "fn main() {\n    let x = 1;\n    println!(\"{}\", x);\n}";
        let new_text = "fn main() {\n    let x = 42;\n    let y = 100;\n    println!(\"{}\", x + y);\n}";

        let diff = DiffModel::compute(old_text, new_text);
        assert!(diff.additions > 0);
        assert!(diff.deletions > 0);

        // Verify side-by-side pairing has modifications
        let has_modify = diff.side_by_side_rows.iter().any(|r| r.kind == DiffOpKind::Modify);
        assert!(has_modify);

        // Verify inline rows have both + and -
        let has_add = diff.inline_rows.iter().any(|r| r.prefix == '+');
        let has_del = diff.inline_rows.iter().any(|r| r.prefix == '-');
        assert!(has_add);
        assert!(has_del);
    }

    #[test]
    fn test_myers_diff_crlf_and_trailing_newline_handling() {
        let old_text = "line1\r\nline2\r\nline3";
        let new_text = "line1\nline2_modified\nline3\n";

        let diff = DiffModel::compute(old_text, new_text);
        assert_eq!(diff.additions, 1);
        assert_eq!(diff.deletions, 1);
        assert_eq!(diff.modifications, 1);
    }

    #[test]
    fn test_myers_diff_roundtrip_transformation_property() {
        let a = ["Apple", "Banana", "Cherry", "Date", "Elderberry"];
        let b = ["Banana", "Cherry Pie", "Date", "Elderberry", "Fig", "Grape"];

        let edits = myers_diff(&a, &b);
        let mut reconstructed = Vec::new();
        let mut a_idx = 0;

        for op in edits {
            match op {
                EditOp::Keep(t) => {
                    assert_eq!(a[a_idx], t);
                    reconstructed.push(t);
                    a_idx += 1;
                }
                EditOp::Delete(t) => {
                    assert_eq!(a[a_idx], t);
                    a_idx += 1;
                }
                EditOp::Insert(t) => {
                    reconstructed.push(t);
                }
            }
        }
        assert_eq!(a_idx, a.len());
        assert_eq!(reconstructed, b.to_vec());
    }
}
