//! Advanced LSP editor widgets: Rich Markdown Hover, Signature Help,
//! Peek References/Implementations, Inline Rename, Inlay Hints, CodeLens, and Document Links.

use crate::cursor::{Position, Range};
use crate::model::piece_table::utf16_col_to_display_col;
use crate::syntax::{GrammarRegistry, SyntaxHighlighter};
use parking_lot::Mutex;
use rustcode_platform::render::{
    Edges, FontMetrics, GlyphRun, InputEvent, KeyCode, MouseButton, Point, Quad,
    Rect, Rgba, Scene, Shadow, WorkbenchThemeStyles,
};
use rustcode_platform::theme::ColorTheme;

// ---------------------------------------------------------------------------
// Rich Markdown Hover Popover
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkdownSpan {
    Plain(String),
    Bold(String),
    Italic(String),
    InlineCode(String),
    Link { label: String, url: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkdownBlock {
    Heading { level: u8, spans: Vec<MarkdownSpan> },
    Paragraph(Vec<MarkdownSpan>),
    CodeBlock { lang: Option<String>, code: String },
    ListItem(Vec<MarkdownSpan>),
    HorizontalRule,
}

fn parse_inline_markdown(text: &str) -> Vec<MarkdownSpan> {
    let mut spans = Vec::new();
    let mut rest = text;
    let mut plain_buf = String::new();

    let flush_plain = |buf: &mut String, out: &mut Vec<MarkdownSpan>| {
        if !buf.is_empty() {
            out.push(MarkdownSpan::Plain(std::mem::take(buf)));
        }
    };

    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('`') {
            if let Some(end_idx) = after.find('`') {
                flush_plain(&mut plain_buf, &mut spans);
                spans.push(MarkdownSpan::InlineCode(after[..end_idx].to_string()));
                rest = &after[end_idx + 1..];
                continue;
            }
        }
        if let Some(after) = rest.strip_prefix("**") {
            if let Some(end_idx) = after.find("**") {
                flush_plain(&mut plain_buf, &mut spans);
                spans.push(MarkdownSpan::Bold(after[..end_idx].to_string()));
                rest = &after[end_idx + 2..];
                continue;
            }
        }
        if let Some(after) = rest.strip_prefix('*') {
            if let Some(end_idx) = after.find('*') {
                if end_idx > 0 {
                    flush_plain(&mut plain_buf, &mut spans);
                    spans.push(MarkdownSpan::Italic(after[..end_idx].to_string()));
                    rest = &after[end_idx + 1..];
                    continue;
                }
            }
        }
        if let Some(after) = rest.strip_prefix('[') {
            if let Some(close_bracket) = after.find("](") {
                let label = &after[..close_bracket];
                let after_paren = &after[close_bracket + 2..];
                if let Some(close_paren) = after_paren.find(')') {
                    let url = &after_paren[..close_paren];
                    flush_plain(&mut plain_buf, &mut spans);
                    spans.push(MarkdownSpan::Link {
                        label: label.to_string(),
                        url: url.to_string(),
                    });
                    rest = &after_paren[close_paren + 1..];
                    continue;
                }
            }
        }
        if let Some(ch) = rest.chars().next() {
            plain_buf.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    flush_plain(&mut plain_buf, &mut spans);
    spans
}

pub fn parse_markdown_hover(markdown: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut in_code = false;
    let mut code_lang: Option<String> = None;
    let mut code_lines: Vec<String> = Vec::new();

    for raw_line in markdown.lines() {
        let trimmed = raw_line.trim();
        if let Some(fence) = trimmed.strip_prefix("```") {
            if in_code {
                blocks.push(MarkdownBlock::CodeBlock {
                    lang: code_lang.take(),
                    code: code_lines.join("\n"),
                });
                code_lines.clear();
                in_code = false;
            } else {
                let lang_trimmed = fence.trim();
                code_lang = (!lang_trimmed.is_empty()).then(|| lang_trimmed.to_string());
                in_code = true;
            }
            continue;
        }
        if in_code {
            code_lines.push(raw_line.to_string());
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            blocks.push(MarkdownBlock::HorizontalRule);
            continue;
        }
        if let Some(h) = trimmed.strip_prefix("### ") {
            blocks.push(MarkdownBlock::Heading {
                level: 3,
                spans: parse_inline_markdown(h),
            });
            continue;
        }
        if let Some(h) = trimmed.strip_prefix("## ") {
            blocks.push(MarkdownBlock::Heading {
                level: 2,
                spans: parse_inline_markdown(h),
            });
            continue;
        }
        if let Some(h) = trimmed.strip_prefix("# ") {
            blocks.push(MarkdownBlock::Heading {
                level: 1,
                spans: parse_inline_markdown(h),
            });
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            blocks.push(MarkdownBlock::ListItem(parse_inline_markdown(item)));
            continue;
        }
        blocks.push(MarkdownBlock::Paragraph(parse_inline_markdown(trimmed)));
    }

    if in_code && !code_lines.is_empty() {
        blocks.push(MarkdownBlock::CodeBlock {
            lang: code_lang,
            code: code_lines.join("\n"),
        });
    }

    blocks
}

fn spans_plain_len(spans: &[MarkdownSpan]) -> usize {
    spans
        .iter()
        .map(|s| match s {
            MarkdownSpan::Plain(t)
            | MarkdownSpan::Bold(t)
            | MarkdownSpan::Italic(t)
            | MarkdownSpan::InlineCode(t) => t.chars().count(),
            MarkdownSpan::Link { label, .. } => label.chars().count(),
        })
        .sum()
}

#[allow(clippy::too_many_arguments)]
pub fn render_markdown_hover_popover(
    markdown: &str,
    anchor_pos: Position,
    scroll_left: f32,
    scroll_top: f32,
    gutter_w: f32,
    bounds: Rect,
    theme: &WorkbenchThemeStyles,
    font: &FontMetrics,
    registry: &GrammarRegistry,
    color_theme: &ColorTheme,
    scene: &mut Scene,
) {
    if !bounds.is_finite_positive() || !font.is_finite() {
        return;
    }
    let blocks = parse_markdown_hover(markdown);
    if blocks.is_empty() {
        return;
    }

    let mut max_chars = 20usize;
    let mut est_height = 12.0_f32;
    for b in &blocks {
        match b {
            MarkdownBlock::Heading { spans, .. } | MarkdownBlock::Paragraph(spans) => {
                max_chars = max_chars.max(spans_plain_len(spans));
                est_height += font.line_height + 4.0;
            }
            MarkdownBlock::ListItem(spans) => {
                max_chars = max_chars.max(spans_plain_len(spans) + 2);
                est_height += font.line_height + 2.0;
            }
            MarkdownBlock::HorizontalRule => {
                est_height += 8.0;
            }
            MarkdownBlock::CodeBlock { code, .. } => {
                for l in code.lines() {
                    max_chars = max_chars.max(l.chars().count() + 2);
                    est_height += font.line_height;
                }
                est_height += 10.0;
            }
        }
    }

    let max_card_w = (bounds.width() - 24.0).max(160.0);
    let card_w = (max_chars as f32 * font.char_width + 24.0).clamp(180.0, max_card_w);
    let max_card_h = (bounds.height() - 16.0).max(font.line_height + 16.0);
    let card_h = est_height.min(max_card_h);

    let anchor_x = bounds.min_x()
        + gutter_w
        + 8.0
        + (anchor_pos.column as f32 - scroll_left) * font.char_width;
    let anchor_y = bounds.min_y() + (anchor_pos.line as f32 - scroll_top + 1.0) * font.line_height;
    let card_x = anchor_x.clamp(
        bounds.min_x() + 4.0,
        (bounds.max_x() - card_w - 4.0).max(bounds.min_x() + 4.0),
    );
    let card_y = if anchor_y + card_h <= bounds.max_y() - 4.0 {
        anchor_y
    } else {
        (anchor_y - card_h - font.line_height).max(bounds.min_y() + 4.0)
    };
    let card_rect = Rect::from_xywh(card_x, card_y, card_w, card_h);
    if !card_rect.is_finite_positive() {
        return;
    }

    scene.push_finite_shadow(Shadow {
        bounds: card_rect,
        offset: Point::new(0.0, 4.0),
        blur: 10.0,
        spread: 0.0,
        color: Rgba::new(0.0, 0.0, 0.0, 0.35),
    });
    scene.push_finite_quad(
        Quad::new(card_rect)
            .with_background(theme.command_palette_bg)
            .with_border_edges(theme.panel_border, Edges::all(1.0))
            .with_corner_radius(4.0),
    );

    let mut cur_y = card_y + 6.0;
    let max_y = card_rect.max_y() - 4.0;
    let inner_x = card_x + 10.0;
    let inner_w = (card_w - 20.0).max(20.0);

    for block in blocks {
        if cur_y >= max_y {
            break;
        }
        match block {
            MarkdownBlock::HorizontalRule => {
                scene.push_finite_quad(
                    Quad::new(Rect::from_xywh(inner_x, cur_y + 3.0, inner_w, 1.0))
                        .with_background(theme.panel_border),
                );
                cur_y += 8.0;
            }
            MarkdownBlock::Heading { spans, .. } => {
                render_markdown_spans(
                    &spans,
                    inner_x,
                    cur_y,
                    inner_w,
                    Rgba::new(0.35, 0.72, 0.98, 1.0),
                    true,
                    font,
                    scene,
                );
                cur_y += font.line_height + 4.0;
            }
            MarkdownBlock::Paragraph(spans) => {
                render_markdown_spans(
                    &spans,
                    inner_x,
                    cur_y,
                    inner_w,
                    theme.editor_foreground,
                    false,
                    font,
                    scene,
                );
                cur_y += font.line_height + 3.0;
            }
            MarkdownBlock::ListItem(spans) => {
                scene.push_finite_text(GlyphRun::new(
                    "•".to_string(),
                    Point::new(inner_x, cur_y),
                    Rect::from_xywh(inner_x, cur_y, font.char_width, font.line_height),
                    theme.editor_line_number,
                    font.font_size,
                ));
                render_markdown_spans(
                    &spans,
                    inner_x + font.char_width * 2.0,
                    cur_y,
                    (inner_w - font.char_width * 2.0).max(10.0),
                    theme.editor_foreground,
                    false,
                    font,
                    scene,
                );
                cur_y += font.line_height + 2.0;
            }
            MarkdownBlock::CodeBlock { lang, code } => {
                let code_lines: Vec<&str> = code.lines().collect();
                let block_h = (code_lines.len().max(1) as f32 * font.line_height + 8.0)
                    .min((max_y - cur_y).max(0.0));
                if block_h > 4.0 {
                    scene.push_finite_quad(
                        Quad::new(Rect::from_xywh(inner_x - 2.0, cur_y, inner_w + 4.0, block_h))
                            .with_background(Rgba::new(0.08, 0.09, 0.11, 0.85))
                            .with_corner_radius(3.0),
                    );
                }
                let mut highlighter = SyntaxHighlighter::for_language(
                    lang.as_deref().unwrap_or("plaintext"),
                    color_theme.clone(),
                    registry,
                );
                cur_y += 4.0;
                for (line_idx, line_str) in code_lines.iter().enumerate() {
                    if cur_y + font.line_height > max_y {
                        break;
                    }
                    let tokens = highlighter.get_line_tokens(line_idx, line_str);
                    if tokens.is_empty() {
                        scene.push_finite_text(GlyphRun::new(
                            (*line_str).to_string(),
                            Point::new(inner_x + 4.0, cur_y),
                            Rect::from_xywh(inner_x + 4.0, cur_y, inner_w - 8.0, font.line_height),
                            theme.editor_foreground,
                            font.font_size * 0.92,
                        ));
                    } else {
                        let mut x = inner_x + 4.0;
                        for tok in tokens {
                            if tok.start < tok.end && tok.end <= line_str.len() {
                                let txt = &line_str[tok.start..tok.end];
                                let w = font.text_width(txt);
                                scene.push_finite_text(
                                    GlyphRun::new(
                                        txt.to_string(),
                                        Point::new(x, cur_y),
                                        Rect::from_xywh(x, cur_y, w, font.line_height),
                                        tok.color.unwrap_or(theme.editor_foreground),
                                        font.font_size * 0.92,
                                    )
                                    .with_style(
                                        tok.metadata.is_bold(),
                                        tok.metadata.is_italic(),
                                        tok.metadata.is_underline(),
                                    ),
                                );
                                x += w;
                            }
                        }
                    }
                    cur_y += font.line_height;
                }
                cur_y += 6.0;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_markdown_spans(
    spans: &[MarkdownSpan],
    start_x: f32,
    y: f32,
    max_w: f32,
    default_fg: Rgba,
    force_bold: bool,
    font: &FontMetrics,
    scene: &mut Scene,
) {
    let mut x = start_x;
    let right_limit = start_x + max_w;
    for span in spans {
        if x >= right_limit {
            break;
        }
        let (text, fg, bold, italic, underline) = match span {
            MarkdownSpan::Plain(t) => (t.as_str(), default_fg, force_bold, false, false),
            MarkdownSpan::Bold(t) => (t.as_str(), default_fg, true, false, false),
            MarkdownSpan::Italic(t) => (t.as_str(), default_fg, force_bold, true, false),
            MarkdownSpan::InlineCode(t) => (
                t.as_str(),
                Rgba::new(0.84, 0.72, 0.52, 1.0),
                force_bold,
                false,
                false,
            ),
            MarkdownSpan::Link { label, .. } => (
                label.as_str(),
                Rgba::new(0.28, 0.65, 0.98, 1.0),
                force_bold,
                false,
                true,
            ),
        };
        let w = font.text_width(text).min((right_limit - x).max(0.0));
        if w > 0.0 {
            scene.push_finite_text(
                GlyphRun::new(
                    text.to_string(),
                    Point::new(x, y),
                    Rect::from_xywh(x, y, w, font.line_height),
                    fg,
                    font.font_size,
                )
                .with_style(bold, italic, underline),
            );
            x += w;
        }
    }
}

// ---------------------------------------------------------------------------
// Signature Help Popover (`Ctrl+Shift+Space`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureParamInfo {
    pub label: String,
    pub label_offsets: Option<(usize, usize)>,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureEntry {
    pub label: String,
    pub documentation: Option<String>,
    pub parameters: Vec<SignatureParamInfo>,
    pub active_parameter: Option<usize>,
}

pub struct SignatureHelpWidget {
    pub is_visible: bool,
    pub signatures: Vec<SignatureEntry>,
    pub active_signature: usize,
    pub active_parameter: usize,
    pub trigger_position: Position,
    pub last_bounds: Mutex<Option<Rect>>,
}

impl Default for SignatureHelpWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl SignatureHelpWidget {
    pub fn new() -> Self {
        Self {
            is_visible: false,
            signatures: Vec::new(),
            active_signature: 0,
            active_parameter: 0,
            trigger_position: Position::new(0, 0),
            last_bounds: Mutex::new(None),
        }
    }

    pub fn show(
        &mut self,
        signatures: Vec<SignatureEntry>,
        active_signature: usize,
        active_parameter: usize,
        pos: Position,
    ) {
        if signatures.is_empty() {
            self.hide();
            return;
        }
        self.signatures = signatures;
        self.active_signature = active_signature.min(self.signatures.len() - 1);
        self.active_parameter = active_parameter;
        self.trigger_position = pos;
        self.is_visible = true;
    }

    pub fn hide(&mut self) {
        self.is_visible = false;
        self.signatures.clear();
        self.active_signature = 0;
        self.active_parameter = 0;
        *self.last_bounds.lock() = None;
    }

    pub fn next_signature(&mut self) {
        if !self.signatures.is_empty() {
            self.active_signature = (self.active_signature + 1) % self.signatures.len();
        }
    }

    pub fn prev_signature(&mut self) {
        if !self.signatures.is_empty() {
            self.active_signature = if self.active_signature == 0 {
                self.signatures.len() - 1
            } else {
                self.active_signature - 1
            };
        }
    }

    pub fn set_active_parameter(&mut self, param_idx: usize) {
        self.active_parameter = param_idx;
    }

    pub fn active_entry(&self) -> Option<&SignatureEntry> {
        self.signatures.get(self.active_signature)
    }

    pub fn render(
        &self,
        cursor_anchor: Point,
        editor_bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !self.is_visible
            || !editor_bounds.is_finite_positive()
            || !font.is_finite()
            || !cursor_anchor.x.is_finite()
            || !cursor_anchor.y.is_finite()
        {
            return;
        }
        let Some(entry) = self.active_entry() else {
            return;
        };

        let active_param_idx = entry.active_parameter.unwrap_or(self.active_parameter);
        let active_param = entry.parameters.get(active_param_idx);

        let count_prefix = if self.signatures.len() > 1 {
            format!("▲ {}/{} ▼ ", self.active_signature + 1, self.signatures.len())
        } else {
            String::new()
        };

        let sig_line_len = count_prefix.chars().count() + entry.label.chars().count();
        let doc_text = active_param
            .and_then(|p| p.documentation.as_deref())
            .or(entry.documentation.as_deref())
            .unwrap_or("");

        let max_w = (editor_bounds.width() - 24.0).max(160.0);
        let box_w = ((sig_line_len.max(doc_text.chars().count()) as f32 * font.char_width) + 24.0)
            .clamp(180.0, max_w);
        let box_h = if doc_text.is_empty() {
            font.line_height + 12.0
        } else {
            font.line_height * 2.0 + 18.0
        };

        let box_x = cursor_anchor.x.clamp(
            editor_bounds.min_x() + 4.0,
            (editor_bounds.max_x() - box_w - 4.0).max(editor_bounds.min_x() + 4.0),
        );
        let above_y = cursor_anchor.y - box_h - 4.0;
        let box_y = if above_y >= editor_bounds.min_y() + 4.0 {
            above_y
        } else {
            (cursor_anchor.y + font.line_height + 4.0)
                .min((editor_bounds.max_y() - box_h - 4.0).max(editor_bounds.min_y()))
        };
        let rect = Rect::from_xywh(box_x, box_y, box_w, box_h);
        if !rect.is_finite_positive() {
            return;
        }
        *self.last_bounds.lock() = Some(rect);

        scene.push_finite_shadow(Shadow {
            bounds: rect,
            offset: Point::new(0.0, 3.0),
            blur: 8.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.35),
        });
        scene.push_finite_quad(
            Quad::new(rect)
                .with_background(theme.command_palette_bg)
                .with_border_edges(theme.panel_border, Edges::all(1.0))
                .with_corner_radius(4.0),
        );

        let mut x = box_x + 8.0;
        let y = box_y + 6.0;
        if !count_prefix.is_empty() {
            let pw = font.text_width(&count_prefix);
            scene.push_finite_text(GlyphRun::new(
                count_prefix,
                Point::new(x, y),
                Rect::from_xywh(x, y, pw, font.line_height),
                theme.editor_line_number,
                font.font_size * 0.9,
            ));
            x += pw;
        }

        // Highlight active parameter substring within signature label if resolvable
        let highlight_span: Option<(usize, usize)> = active_param.and_then(|p| {
            if let Some((s, e)) = p.label_offsets {
                let b_start = entry
                    .label
                    .char_indices()
                    .nth(s)
                    .map(|(i, _)| i)?;
                let b_end = entry
                    .label
                    .char_indices()
                    .nth(e)
                    .map_or(entry.label.len(), |(i, _)| i);
                (b_start < b_end).then_some((b_start, b_end))
            } else {
                entry
                    .label
                    .find(&p.label)
                    .map(|start| (start, start + p.label.len()))
            }
        });

        if let Some((hs, he)) = highlight_span {
            let pre = &entry.label[..hs];
            let mid = &entry.label[hs..he];
            let post = &entry.label[he..];
            if !pre.is_empty() {
                let w = font.text_width(pre);
                scene.push_finite_text(GlyphRun::new(
                    pre.to_string(),
                    Point::new(x, y),
                    Rect::from_xywh(x, y, w, font.line_height),
                    theme.editor_foreground,
                    font.font_size,
                ));
                x += w;
            }
            let mw = font.text_width(mid);
            scene.push_finite_text(
                GlyphRun::new(
                    mid.to_string(),
                    Point::new(x, y),
                    Rect::from_xywh(x, y, mw, font.line_height),
                    Rgba::new(0.32, 0.76, 1.0, 1.0),
                    font.font_size,
                )
                .with_style(true, false, true),
            );
            x += mw;
            if !post.is_empty() {
                let w = font.text_width(post);
                scene.push_finite_text(GlyphRun::new(
                    post.to_string(),
                    Point::new(x, y),
                    Rect::from_xywh(x, y, w, font.line_height),
                    theme.editor_foreground,
                    font.font_size,
                ));
            }
        } else {
            let w = font.text_width(&entry.label);
            scene.push_finite_text(GlyphRun::new(
                entry.label.clone(),
                Point::new(x, y),
                Rect::from_xywh(x, y, w, font.line_height),
                theme.editor_foreground,
                font.font_size,
            ));
        }

        if !doc_text.is_empty() {
            let doc_y = y + font.line_height + 4.0;
            let dw = font.text_width(doc_text).min((box_w - 16.0).max(20.0));
            scene.push_finite_text(GlyphRun::new(
                doc_text.to_string(),
                Point::new(box_x + 8.0, doc_y),
                Rect::from_xywh(box_x + 8.0, doc_y, dw, font.line_height),
                Rgba::new(0.75, 0.78, 0.82, 1.0),
                font.font_size * 0.9,
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Inline Annotations: Inlay Hints, CodeLens, Document Links
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorInlayHint {
    pub position: Position,
    pub label: String,
    pub is_type: bool,
    pub padding_left: bool,
    pub padding_right: bool,
    pub tooltip: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditorCodeLens {
    pub line: usize,
    pub title: String,
    pub command: Option<String>,
    pub arguments: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorDocumentLink {
    pub range: Range,
    pub target: String,
    pub tooltip: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn render_inline_annotations(
    line_idx: usize,
    line_content: &str,
    line_y: f32,
    text_area_x: f32,
    scroll_left: f32,
    code_clip_rect: Rect,
    inlay_hints: &[EditorInlayHint],
    code_lenses: &[EditorCodeLens],
    document_links: &[EditorDocumentLink],
    font: &FontMetrics,
    scene: &mut Scene,
) {
    // 1. Inlay hints on this line
    for hint in inlay_hints {
        if hint.position.line != line_idx || hint.label.is_empty() {
            continue;
        }
        let vis_col = utf16_col_to_display_col(line_content, hint.position.column);
        let pad_l = if hint.padding_left { 4.0 } else { 0.0 };
        let hx = text_area_x + (vis_col as f32 - scroll_left) * font.char_width + pad_l;
        let hw = font.text_width(&hint.label) + 6.0;
        let rect = Rect::from_xywh(hx, line_y + 1.0, hw, (font.line_height - 2.0).max(4.0));
        if rect.max_x() >= code_clip_rect.min_x() && rect.min_x() <= code_clip_rect.max_x() && rect.is_finite_positive() {
            scene.push_finite_quad(
                Quad::new(rect)
                    .with_background(Rgba::new(0.22, 0.25, 0.30, 0.55))
                    .with_corner_radius(3.0),
            );
            scene.push_finite_text(GlyphRun::new(
                hint.label.clone(),
                Point::new(hx + 3.0, line_y + 1.0),
                rect,
                Rgba::new(0.60, 0.66, 0.74, 0.95),
                font.font_size * 0.85,
            ));
        }
    }

    // 2. CodeLens badges on this line (rendered inline right of line end)
    let lenses: Vec<&EditorCodeLens> = code_lenses.iter().filter(|l| l.line == line_idx).collect();
    if !lenses.is_empty() {
        let line_vis = utf16_col_to_display_col(line_content, line_content.chars().map(|c| c.len_utf16()).sum());
        let mut lx = text_area_x + (line_vis as f32 - scroll_left + 2.0) * font.char_width;
        for lens in lenses {
            let lw = font.text_width(&lens.title) + 8.0;
            let rect = Rect::from_xywh(lx, line_y + 1.0, lw, (font.line_height - 2.0).max(4.0));
            if rect.is_finite_positive() {
                scene.push_finite_text(GlyphRun::new(
                    lens.title.clone(),
                    Point::new(lx + 4.0, line_y + 1.0),
                    rect,
                    Rgba::new(0.55, 0.65, 0.82, 0.92),
                    font.font_size * 0.85,
                ));
            }
            lx += lw + 8.0;
        }
    }

    // 3. Document Links underline on this line
    for link in document_links {
        if link.range.start.line != line_idx {
            continue;
        }
        let s_vis = utf16_col_to_display_col(line_content, link.range.start.column);
        let e_vis = utf16_col_to_display_col(line_content, link.range.end.column).max(s_vis + 1);
        let x = text_area_x + (s_vis as f32 - scroll_left) * font.char_width;
        let w = ((e_vis - s_vis) as f32 * font.char_width).max(4.0);
        let ul_rect = Rect::from_xywh(x, line_y + font.line_height - 2.0, w, 1.0);
        if ul_rect.is_finite_positive() {
            scene.push_finite_quad(
                Quad::new(ul_rect).with_background(Rgba::new(0.28, 0.66, 0.98, 0.9)),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn hit_test_codelens<'a>(
    point: Point,
    line_idx: usize,
    line_content: &str,
    line_y: f32,
    text_area_x: f32,
    scroll_left: f32,
    code_lenses: &'a [EditorCodeLens],
    font: &FontMetrics,
) -> Option<&'a EditorCodeLens> {
    if point.y < line_y || point.y > line_y + font.line_height {
        return None;
    }
    let line_vis = utf16_col_to_display_col(line_content, line_content.chars().map(|c| c.len_utf16()).sum());
    let mut lx = text_area_x + (line_vis as f32 - scroll_left + 2.0) * font.char_width;
    for lens in code_lenses.iter().filter(|l| l.line == line_idx) {
        let lw = font.text_width(&lens.title) + 8.0;
        let rect = Rect::from_xywh(lx, line_y, lw, font.line_height);
        if rect.contains(point) {
            return Some(lens);
        }
        lx += lw + 8.0;
    }
    None
}

// ---------------------------------------------------------------------------
// Peek References / Implementations / Type Definitions Widget (`Shift+F12`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeekKind {
    References,
    Implementations,
    TypeDefinitions,
}

impl PeekKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::References => "References",
            Self::Implementations => "Implementations",
            Self::TypeDefinitions => "Type Definitions",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeekLocationItem {
    pub uri: String,
    pub display_path: String,
    pub range: Range,
    pub preview_lines: Vec<(usize, String)>,
    pub preview_match_line: usize,
}

pub struct PeekViewWidget {
    pub is_visible: bool,
    pub kind: PeekKind,
    pub title: String,
    pub anchor_line: usize,
    pub items: Vec<PeekLocationItem>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub last_bounds: Mutex<Option<Rect>>,
}

impl Default for PeekViewWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl PeekViewWidget {
    pub const HEADER_HEIGHT: f32 = 26.0;
    pub const ROW_HEIGHT: f32 = 22.0;

    pub fn new() -> Self {
        Self {
            is_visible: false,
            kind: PeekKind::References,
            title: String::new(),
            anchor_line: 0,
            items: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            last_bounds: Mutex::new(None),
        }
    }

    pub fn open(
        &mut self,
        kind: PeekKind,
        title: impl Into<String>,
        anchor_line: usize,
        items: Vec<PeekLocationItem>,
    ) {
        self.kind = kind;
        self.title = title.into();
        self.anchor_line = anchor_line;
        self.items = items;
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.is_visible = !self.items.is_empty();
    }

    pub fn close(&mut self) {
        self.is_visible = false;
        self.items.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
        *self.last_bounds.lock() = None;
    }

    pub fn select_next(&mut self) {
        if !self.items.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.items.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.items.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                self.items.len() - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    pub fn selected_item(&self) -> Option<&PeekLocationItem> {
        self.items.get(self.selected_index)
    }

    pub fn compute_bounds(&self, editor_bounds: Rect, font: &FontMetrics) -> Option<Rect> {
        if !self.is_visible || !editor_bounds.is_finite_positive() || !font.is_finite() {
            return None;
        }
        let h = (editor_bounds.height() * 0.42).clamp(130.0, 260.0).min(editor_bounds.height() - 16.0);
        let w = (editor_bounds.width() - 24.0).max(160.0);
        let x = editor_bounds.min_x() + 12.0;
        let y = (editor_bounds.min_y() + 48.0).min((editor_bounds.max_y() - h - 8.0).max(editor_bounds.min_y() + 4.0));
        let rect = Rect::from_xywh(x, y, w, h);
        rect.is_finite_positive().then_some(rect)
    }

    pub fn handle_event(&mut self, event: &InputEvent, editor_bounds: Rect, font: &FontMetrics) -> Option<Option<PeekLocationItem>> {
        if !self.is_visible {
            return None;
        }
        match event {
            InputEvent::KeyDown(key) => match key.key {
                KeyCode::Escape => {
                    self.close();
                    Some(None)
                }
                KeyCode::ArrowDown => {
                    self.select_next();
                    Some(None)
                }
                KeyCode::ArrowUp => {
                    self.select_prev();
                    Some(None)
                }
                KeyCode::Enter => {
                    let chosen = self.selected_item().cloned();
                    self.close();
                    Some(chosen)
                }
                _ => None,
            },
            InputEvent::MouseDown(mouse) if mouse.button == MouseButton::Left => {
                let rect = self.compute_bounds(editor_bounds, font)?;
                if !rect.contains(mouse.position) {
                    return None;
                }
                // Close button in top-right of header
                let close_rect = Rect::from_xywh(rect.max_x() - 26.0, rect.min_y(), 26.0, Self::HEADER_HEIGHT);
                if close_rect.contains(mouse.position) {
                    self.close();
                    return Some(None);
                }
                // Right list pane
                let list_x = rect.min_x() + rect.width() * 0.62;
                if mouse.position.x >= list_x && mouse.position.y >= rect.min_y() + Self::HEADER_HEIGHT {
                    let idx = self.scroll_offset
                        + ((mouse.position.y - rect.min_y() - Self::HEADER_HEIGHT) / Self::ROW_HEIGHT)
                            .floor()
                            .max(0.0) as usize;
                    if idx < self.items.len() {
                        self.selected_index = idx;
                        if mouse.click_count >= 2 {
                            let chosen = self.items.get(idx).cloned();
                            self.close();
                            return Some(chosen);
                        }
                    }
                }
                Some(None)
            }
            _ => None,
        }
    }

    pub fn render(
        &self,
        editor_bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        let Some(rect) = self.compute_bounds(editor_bounds, font) else {
            return;
        };
        *self.last_bounds.lock() = Some(rect);

        scene.push_finite_shadow(Shadow {
            bounds: rect,
            offset: Point::new(0.0, 4.0),
            blur: 10.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.4),
        });
        scene.push_finite_quad(
            Quad::new(rect)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.activity_bar_active_border, Edges::all(1.5))
                .with_corner_radius(3.0),
        );

        // Header bar
        let header_rect = Rect::from_xywh(rect.min_x(), rect.min_y(), rect.width(), Self::HEADER_HEIGHT);
        scene.push_finite_quad(
            Quad::new(header_rect)
                .with_background(theme.panel_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );
        let header_title = format!(
            "{} ({}) — {}",
            self.kind.label(),
            self.items.len(),
            self.title
        );
        scene.push_finite_text(GlyphRun::new(
            header_title,
            Point::new(rect.min_x() + 10.0, rect.min_y() + 4.0),
            Rect::from_xywh(rect.min_x() + 10.0, rect.min_y() + 4.0, (rect.width() - 42.0).max(20.0), font.line_height),
            theme.editor_foreground,
            font.font_size * 0.9,
        ));
        scene.push_finite_text(GlyphRun::new(
            "✕".to_string(),
            Point::new(rect.max_x() - 20.0, rect.min_y() + 4.0),
            Rect::from_xywh(rect.max_x() - 24.0, rect.min_y() + 4.0, 20.0, font.line_height),
            theme.editor_line_number,
            font.font_size * 0.9,
        ));

        let body_y = rect.min_y() + Self::HEADER_HEIGHT;
        let body_h = (rect.height() - Self::HEADER_HEIGHT).max(0.0);
        let preview_w = (rect.width() * 0.62).max(40.0);
        let list_x = rect.min_x() + preview_w;
        let list_w = (rect.width() - preview_w).max(40.0);

        // Divider between preview & location list
        scene.push_finite_quad(
            Quad::new(Rect::from_xywh(list_x, body_y, 1.0, body_h))
                .with_background(theme.panel_border),
        );

        // Left preview snippet
        if let Some(sel) = self.selected_item() {
            let mut py = body_y + 4.0;
            for (line_num, line_text) in &sel.preview_lines {
                if py + font.line_height > rect.max_y() - 2.0 {
                    break;
                }
                if *line_num == sel.preview_match_line {
                    scene.push_finite_quad(
                        Quad::new(Rect::from_xywh(rect.min_x() + 2.0, py, preview_w - 4.0, font.line_height))
                            .with_background(Rgba::new(0.28, 0.45, 0.72, 0.25)),
                    );
                }
                let formatted = format!("{:>4} | {}", line_num + 1, line_text);
                scene.push_finite_text(GlyphRun::new(
                    formatted,
                    Point::new(rect.min_x() + 6.0, py),
                    Rect::from_xywh(rect.min_x() + 6.0, py, (preview_w - 12.0).max(20.0), font.line_height),
                    theme.editor_foreground,
                    font.font_size * 0.9,
                ));
                py += font.line_height;
            }
        }

        // Right clickable location list
        for (idx, item) in self.items.iter().enumerate().skip(self.scroll_offset) {
            let row_y = body_y + (idx - self.scroll_offset) as f32 * Self::ROW_HEIGHT;
            if row_y + Self::ROW_HEIGHT > rect.max_y() {
                break;
            }
            let row_rect = Rect::from_xywh(list_x + 2.0, row_y, (list_w - 4.0).max(10.0), Self::ROW_HEIGHT);
            let selected = idx == self.selected_index;
            if selected {
                scene.push_finite_quad(
                    Quad::new(row_rect).with_background(theme.command_palette_selected_bg),
                );
            }
            let label = format!("{}:{}", item.display_path, item.range.start.line + 1);
            scene.push_finite_text(GlyphRun::new(
                label,
                Point::new(list_x + 8.0, row_y + 2.0),
                row_rect,
                if selected {
                    theme.command_palette_fg
                } else {
                    theme.editor_foreground
                },
                font.font_size * 0.88,
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Inline Rename Input Widget (`F2`)
// ---------------------------------------------------------------------------

pub struct RenameInputWidget {
    pub is_visible: bool,
    pub position: Position,
    pub buffer: String,
    pub error: Option<String>,
    pub last_bounds: Mutex<Option<Rect>>,
}

impl Default for RenameInputWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl RenameInputWidget {
    pub fn new() -> Self {
        Self {
            is_visible: false,
            position: Position::new(0, 0),
            buffer: String::new(),
            error: None,
            last_bounds: Mutex::new(None),
        }
    }

    pub fn open(&mut self, position: Position, initial_name: impl Into<String>) {
        self.is_visible = true;
        self.position = position;
        self.buffer = initial_name.into();
        self.error = None;
    }

    pub fn close(&mut self) {
        self.is_visible = false;
        self.buffer.clear();
        self.error = None;
        *self.last_bounds.lock() = None;
    }

    /// Returns `Some(Some((pos, new_name, preview)))` when Enter/Shift+Enter confirms,
    /// `Some(None)` when cancelled, and `None` when not open.
    pub fn handle_key(
        &mut self,
        key: &rustcode_platform::render::KeyEvent,
    ) -> Option<Option<(Position, String, bool)>> {
        if !self.is_visible {
            return None;
        }
        match &key.key {
            KeyCode::Escape => {
                self.close();
                Some(None)
            }
            KeyCode::Enter => {
                let trimmed = self.buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.error = Some("Identifier cannot be empty".to_string());
                    return Some(None);
                }
                let pos = self.position;
                let preview = key.modifiers.shift;
                self.close();
                Some(Some((pos, trimmed, preview)))
            }
            KeyCode::Backspace => {
                self.buffer.pop();
                self.error = None;
                Some(None)
            }
            KeyCode::Char(c) if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                let s = match &key.text {
                    Some(t) if !t.is_empty() => t.clone(),
                    _ => {
                        if key.modifiers.shift {
                            c.to_ascii_uppercase().to_string()
                        } else {
                            c.to_string()
                        }
                    }
                };
                self.buffer.push_str(&s);
                self.error = None;
                Some(None)
            }
            _ => Some(None),
        }
    }

    pub fn render(
        &self,
        cursor_anchor: Point,
        editor_bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !self.is_visible
            || !editor_bounds.is_finite_positive()
            || !font.is_finite()
            || !cursor_anchor.x.is_finite()
            || !cursor_anchor.y.is_finite()
        {
            return;
        }
        let w = 240.0_f32.min((editor_bounds.width() - 16.0).max(80.0));
        let h = font.line_height + 10.0;
        let x = cursor_anchor.x.clamp(
            editor_bounds.min_x() + 4.0,
            (editor_bounds.max_x() - w - 4.0).max(editor_bounds.min_x() + 4.0),
        );
        let y = (cursor_anchor.y + font.line_height + 2.0)
            .min((editor_bounds.max_y() - h - 4.0).max(editor_bounds.min_y() + 4.0));
        let rect = Rect::from_xywh(x, y, w, h);
        if !rect.is_finite_positive() {
            return;
        }
        *self.last_bounds.lock() = Some(rect);

        scene.push_finite_shadow(Shadow {
            bounds: rect,
            offset: Point::new(0.0, 2.0),
            blur: 6.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.35),
        });
        scene.push_finite_quad(
            Quad::new(rect)
                .with_background(theme.command_palette_bg)
                .with_border_edges(theme.activity_bar_active_border, Edges::all(1.0))
                .with_corner_radius(3.0),
        );
        scene.push_finite_text(GlyphRun::new(
            self.buffer.clone(),
            Point::new(x + 6.0, y + 4.0),
            Rect::from_xywh(x + 6.0, y + 4.0, (w - 12.0).max(16.0), font.line_height),
            theme.editor_foreground,
            font.font_size,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::render::KeyEvent;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::theme::ThemeService;

    #[test]
    fn test_markdown_hover_parser_and_render() {
        let md = "### Function `calc`\n\nComputes **sum** of [values](https://example.com).\n\n- fast\n- safe\n\n```rust\npub fn calc(a: i32) -> i32 { a }\n```";
        let blocks = parse_markdown_hover(md);
        assert_eq!(blocks.len(), 5);
        assert!(matches!(blocks[0], MarkdownBlock::Heading { level: 3, .. }));
        assert!(matches!(blocks[4], MarkdownBlock::CodeBlock { lang: Some(ref l), .. } if l == "rust"));

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let font = FontMetrics::default_monospace();
        let color_theme = ThemeService::default_dark_theme();
        let theme = WorkbenchThemeStyles::from_theme(&color_theme);
        let reg = GrammarRegistry::new();
        let mut scene = Scene::new();
        render_markdown_hover_popover(
            md,
            Position::new(2, 4),
            0.0,
            0.0,
            40.0,
            bounds,
            &theme,
            &font,
            &reg,
            &color_theme,
            &mut scene,
        );
        assert!(scene.glyph_runs().iter().any(|r| r.text.contains("calc")));
        assert!(scene.glyph_runs().iter().any(|r| r.text.contains("sum") && r.bold));
    }

    #[test]
    fn test_signature_help_peek_and_rename_widgets() {
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        let mut sig = SignatureHelpWidget::new();
        sig.show(
            vec![SignatureEntry {
                label: "fn add(a: i32, b: i32) -> i32".to_string(),
                documentation: Some("Adds integers".to_string()),
                parameters: vec![
                    SignatureParamInfo {
                        label: "a: i32".to_string(),
                        label_offsets: None,
                        documentation: Some("First operand".to_string()),
                    },
                    SignatureParamInfo {
                        label: "b: i32".to_string(),
                        label_offsets: None,
                        documentation: None,
                    },
                ],
                active_parameter: None,
            }],
            0,
            0,
            Position::new(1, 10),
        );
        let mut scene = Scene::new();
        sig.render(Point::new(100.0, 80.0), bounds, &theme, &font, &mut scene);
        assert!(scene.glyph_runs().iter().any(|r| r.text.contains("a: i32") && r.bold));

        let mut peek = PeekViewWidget::new();
        peek.open(
            PeekKind::References,
            "add",
            5,
            vec![
                PeekLocationItem {
                    uri: "file:///a.rs".to_string(),
                    display_path: "a.rs".to_string(),
                    range: Range::new(Position::new(2, 4), Position::new(2, 7)),
                    preview_lines: vec![(2, "let x = add(1, 2);".to_string())],
                    preview_match_line: 2,
                },
                PeekLocationItem {
                    uri: "file:///b.rs".to_string(),
                    display_path: "b.rs".to_string(),
                    range: Range::new(Position::new(8, 0), Position::new(8, 3)),
                    preview_lines: vec![(8, "add(3, 4)".to_string())],
                    preview_match_line: 8,
                },
            ],
        );
        peek.render(bounds, &theme, &font, &mut scene);
        let down = InputEvent::KeyDown(KeyEvent::new(KeyCode::ArrowDown, Modifiers::none()));
        peek.handle_event(&down, bounds, &font);
        assert_eq!(peek.selected_index, 1);
        let enter = InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none()));
        let nav = peek.handle_event(&enter, bounds, &font).unwrap().unwrap();
        assert_eq!(nav.uri, "file:///b.rs");
        assert!(!peek.is_visible);

        let mut rename = RenameInputWidget::new();
        rename.open(Position::new(1, 4), "add");
        let back = KeyEvent::new(KeyCode::Backspace, Modifiers::none());
        rename.handle_key(&back);
        rename.handle_key(&KeyEvent::new(KeyCode::Char('s'), Modifiers::none()));
        let confirmed = rename
            .handle_key(&KeyEvent::new(KeyCode::Enter, Modifiers::none()))
            .unwrap()
            .unwrap();
        assert_eq!(confirmed.1, "ads");
        assert!(!confirmed.2);
    }
}
