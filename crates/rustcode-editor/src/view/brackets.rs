//! Rainbow Bracket-Pair Colorization and Vertical Indent Guides.

use crate::model::piece_table::PieceTable;
use rustcode_platform::render::Rgba;

#[derive(Debug, Clone, PartialEq)]
pub struct BracketColorConfig {
    pub enabled: bool,
    pub guides_enabled: bool,
    pub palette: Vec<Rgba>,
    pub unexpected_color: Rgba,
}

impl Default for BracketColorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            guides_enabled: true,
            palette: vec![
                Rgba::new(0.98, 0.84, 0.25, 1.0), // gold
                Rgba::new(0.85, 0.44, 0.84, 1.0), // orchid
                Rgba::new(0.24, 0.72, 0.98, 1.0), // sky blue
                Rgba::new(0.38, 0.84, 0.48, 1.0), // green
                Rgba::new(0.95, 0.58, 0.24, 1.0), // orange
                Rgba::new(0.35, 0.84, 0.84, 1.0), // cyan
            ],
            unexpected_color: Rgba::new(0.95, 0.25, 0.25, 1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BracketSpan {
    pub line: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub depth: usize,
    pub color: Rgba,
    pub is_unmatched: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BracketGuide {
    pub line: usize,
    pub column: usize,
    pub color: Rgba,
}

fn is_open_bracket(b: u8) -> bool {
    matches!(b, b'(' | b'[' | b'{')
}

fn matching_open(close: u8) -> Option<u8> {
    match close {
        b')' => Some(b'('),
        b']' => Some(b'['),
        b'}' => Some(b'{'),
        _ => None,
    }
}

/// Scan a bounded window `[start_line.saturating_sub(256)..=end_line]` to compute
/// rainbow bracket colors & scope guides for lines in `[start_line..=end_line]`.
pub fn compute_visible_brackets(
    buf: &PieceTable,
    start_line: usize,
    end_line: usize,
    cfg: &BracketColorConfig,
) -> (Vec<BracketSpan>, Vec<BracketGuide>) {
    if (!cfg.enabled && !cfg.guides_enabled) || cfg.palette.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let line_count = buf.line_count();
    if line_count == 0 || start_line >= line_count {
        return (Vec::new(), Vec::new());
    }
    let last_line = end_line.min(line_count - 1);
    let prefix_start = start_line.saturating_sub(256);

    let mut stack: Vec<(u8, usize, usize)> = Vec::new(); // (open_byte, depth, open_col)
    let mut in_block_comment = false;
    let mut spans = Vec::new();
    let mut guides = Vec::new();

    for line_idx in prefix_start..=last_line {
        let Some(line) = buf.get_line_content(line_idx) else {
            continue;
        };
        let in_viewport = line_idx >= start_line;
        if in_viewport && cfg.guides_enabled {
            for &(_, depth, col) in &stack {
                guides.push(BracketGuide {
                    line: line_idx,
                    column: col,
                    color: cfg.palette[depth % cfg.palette.len()].with_alpha(0.35),
                });
            }
        }

        let bytes = line.as_bytes();
        let mut i = 0;
        let mut in_string: Option<u8> = None;
        let mut escaped = false;
        while i < bytes.len() {
            let b = bytes[i];
            if in_block_comment {
                if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    in_block_comment = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if let Some(q) = in_string {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == q {
                    in_string = None;
                }
                i += 1;
                continue;
            }
            if b == b'/' && i + 1 < bytes.len() {
                if bytes[i + 1] == b'/' {
                    break;
                }
                if bytes[i + 1] == b'*' {
                    in_block_comment = true;
                    i += 2;
                    continue;
                }
            }
            if b == b'"' || b == b'\'' {
                in_string = Some(b);
                i += 1;
                continue;
            }

            if is_open_bracket(b) {
                let depth = stack.len();
                let indent_col = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
                stack.push((b, depth, indent_col));
                if in_viewport && cfg.enabled {
                    spans.push(BracketSpan {
                        line: line_idx,
                        byte_start: i,
                        byte_end: i + 1,
                        depth,
                        color: cfg.palette[depth % cfg.palette.len()],
                        is_unmatched: false,
                    });
                }
            } else if let Some(expected_open) = matching_open(b) {
                if stack.last().is_some_and(|&(op, _, _)| op == expected_open) {
                    let (_, depth, _) = stack.pop().unwrap();
                    if in_viewport && cfg.enabled {
                        spans.push(BracketSpan {
                            line: line_idx,
                            byte_start: i,
                            byte_end: i + 1,
                            depth,
                            color: cfg.palette[depth % cfg.palette.len()],
                            is_unmatched: false,
                        });
                    }
                } else if in_viewport && cfg.enabled {
                    spans.push(BracketSpan {
                        line: line_idx,
                        byte_start: i,
                        byte_end: i + 1,
                        depth: 0,
                        color: cfg.unexpected_color,
                        is_unmatched: true,
                    });
                }
            }
            i += 1;
        }
    }

    (spans, guides)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bracket_pair_colorization_skips_strings_and_detects_unmatched() {
        let src = "fn main() {\n    let s = \"([{\"; // )\n    let x = [1];\n}\n]\n";
        let buf = PieceTable::from_str(src);
        let cfg = BracketColorConfig::default();
        let (spans, guides) = compute_visible_brackets(&buf, 0, 4, &cfg);

        // No brackets inside `"([{"` string or `// )` comment
        assert!(!spans.iter().any(|s| s.line == 1 && s.byte_start > 10));
        // Matched nesting colors cycle by depth
        let open_curly = spans.iter().find(|s| s.line == 0 && s.byte_start == 10).unwrap();
        let close_curly = spans.iter().find(|s| s.line == 3 && s.byte_start == 0).unwrap();
        assert_eq!(open_curly.depth, close_curly.depth);
        assert_eq!(open_curly.color, close_curly.color);
        // Trailing unmatched `]`
        let bad = spans.iter().find(|s| s.line == 4).unwrap();
        assert!(bad.is_unmatched);
        assert_eq!(bad.color, cfg.unexpected_color);
        assert!(!guides.is_empty());
    }
}
