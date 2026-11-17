//! Webview panel container and pure-Rust visual renderer in GPUI scene graph.
//!
//! Hosts custom extension HTML views (`vscode.window.createWebviewPanel`) and provides
//! a bidirectional MessagePort / RPC bridge (`postMessage`, `onDidReceiveMessage`).

use parking_lot::RwLock;
use rustcode_ext_host::webview::{ExtHostWebviewManager, WebviewOptions};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebviewElementKind {
    Heading1(String),
    Heading2(String),
    Heading3(String),
    Paragraph(String),
    CodeBlock(String),
    Button { label: String, message_payload: Value },
    HorizontalRule,
    ListItem(String),
}

#[derive(Debug, Clone)]
pub struct ParsedWebviewNode {
    pub kind: WebviewElementKind,
}

pub struct WebviewEditorView {
    pub id: String,
    pub view_type: String,
    pub title: String,
    pub html: String,
    pub options: WebviewOptions,
    pub nodes: Vec<ParsedWebviewNode>,
    pub inbound_messages: Vec<Value>,
    pub outbound_messages: Vec<Value>,
    pub scroll_top: f32,
    pub ext_host_manager: Option<ExtHostWebviewManager>,
    pub button_targets: RwLock<Vec<(Rect, Value)>>,
}

impl WebviewEditorView {
    pub fn new(
        id: &str,
        view_type: &str,
        title: &str,
        html: &str,
        options: WebviewOptions,
        ext_host_manager: Option<ExtHostWebviewManager>,
    ) -> Self {
        let nodes = Self::parse_html(html);
        Self {
            id: id.to_string(),
            view_type: view_type.to_string(),
            title: title.to_string(),
            html: html.to_string(),
            options,
            nodes,
            inbound_messages: Vec::new(),
            outbound_messages: Vec::new(),
            scroll_top: 0.0,
            ext_host_manager,
            button_targets: RwLock::new(Vec::new()),
        }
    }

    pub fn set_html(&mut self, html: &str) {
        self.html = html.to_string();
        self.nodes = Self::parse_html(html);
    }

    /// Receive message from extension host into webview.
    pub fn receive_message_from_host(&mut self, msg: Value) {
        self.inbound_messages.push(msg);
    }

    /// Send message from webview out to extension host.
    pub fn send_message_to_host(&mut self, msg: Value) {
        self.outbound_messages.push(msg.clone());
        if let Some(mgr) = &self.ext_host_manager {
            let _ = mgr.receive_message_from_webview(&self.id, msg);
        }
    }

    /// Robust pure-Rust HTML tag scanner converting tags into renderable nodes.
    pub fn parse_html(html: &str) -> Vec<ParsedWebviewNode> {
        let mut cleaned = html.to_string();
        strip_comments(&mut cleaned);
        strip_doctype(&mut cleaned);
        strip_tag_block(&mut cleaned, "head");
        strip_tag_block(&mut cleaned, "style");
        strip_tag_block(&mut cleaned, "script");

        let mut nodes = Vec::new();
        parse_nodes_recursive(&cleaned, &mut nodes);
        nodes
    }
}

fn strip_tag_block(html: &mut String, tag_name: &str) {
    let open_pat = format!("<{}", tag_name);
    let close_pat = format!("</{}>", tag_name);
    while let Some(start) = html.to_lowercase().find(&open_pat) {
        if let Some(end) = html.to_lowercase()[start..].find(&close_pat) {
            let full_end = start + end + close_pat.len();
            html.replace_range(start..full_end, "");
        } else {
            html.truncate(start);
            break;
        }
    }
}

fn strip_comments(html: &mut String) {
    while let Some(start) = html.find("<!--") {
        if let Some(end) = html[start..].find("-->") {
            let full_end = start + end + 3;
            html.replace_range(start..full_end, "");
        } else {
            html.truncate(start);
            break;
        }
    }
}

fn strip_doctype(html: &mut String) {
    while let Some(start) = html.to_lowercase().find("<!doctype") {
        if let Some(end) = html[start..].find('>') {
            let full_end = start + end + 1;
            html.replace_range(start..full_end, "");
        } else {
            html.truncate(start);
            break;
        }
    }
}

fn decode_html_entities(s: &str) -> String {
    let mut res = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");

    while let Some(start) = res.find("&#") {
        if let Some(end) = res[start..].find(';') {
            let entity = &res[start + 2..start + end];
            let parsed_char = if entity.starts_with('x') || entity.starts_with('X') {
                u32::from_str_radix(&entity[1..], 16).ok().and_then(char::from_u32)
            } else {
                entity.parse::<u32>().ok().and_then(char::from_u32)
            };
            if let Some(ch) = parsed_char {
                res.replace_range(start..=start + end, &ch.to_string());
            } else {
                break;
            }
        } else {
            break;
        }
    }
    res
}

fn strip_inner_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(c);
        }
    }
    out
}

fn strip_and_decode(html: &str) -> String {
    let stripped = strip_inner_tags(html);
    decode_html_entities(&stripped).trim().to_string()
}

fn extract_attribute(tag_content: &str, attr: &str) -> Option<String> {
    let pattern_double = format!("{}=\"", attr);
    if let Some(pos) = tag_content.to_lowercase().find(&pattern_double) {
        let after = &tag_content[pos + pattern_double.len()..];
        if let Some(end) = after.find('"') {
            return Some(after[..end].to_string());
        }
    }
    let pattern_single = format!("{}='", attr);
    if let Some(pos) = tag_content.to_lowercase().find(&pattern_single) {
        let after = &tag_content[pos + pattern_single.len()..];
        if let Some(end) = after.find('\'') {
            return Some(after[..end].to_string());
        }
    }
    None
}

fn has_child_tags(s: &str) -> bool {
    s.contains('<') && s.contains('>')
}

fn has_block_child_tags(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.contains("<h1")
        || lower.contains("<h2")
        || lower.contains("<h3")
        || lower.contains("<h4")
        || lower.contains("<h5")
        || lower.contains("<h6")
        || lower.contains("<p")
        || lower.contains("<button")
        || lower.contains("<pre")
        || lower.contains("<code")
        || lower.contains("<div")
        || lower.contains("<hr")
        || lower.contains("<ul")
        || lower.contains("<ol")
        || lower.contains("<li")
}

fn find_matching_close_tag<'a>(tag_name: &str, html: &'a str) -> (&'a str, &'a str) {
    let open_prefix = format!("<{}", tag_name);
    let close_tag = format!("</{}>", tag_name);
    let mut depth = 1usize;
    let mut idx = 0;

    while idx < html.len() {
        let remainder = &html[idx..];
        if remainder.to_lowercase().starts_with(&close_tag) {
            depth -= 1;
            if depth == 0 {
                let inner = &html[..idx];
                let next_rest = &html[idx + close_tag.len()..];
                return (inner.trim(), next_rest);
            }
            idx += close_tag.len();
        } else if remainder.to_lowercase().starts_with(&open_prefix) {
            let after_open = &remainder[open_prefix.len()..];
            if after_open.starts_with(' ') || after_open.starts_with('>') || after_open.starts_with('/') {
                depth += 1;
            }
            idx += open_prefix.len();
        } else {
            idx += 1;
        }
    }

    (html.trim(), "")
}

fn parse_nodes_recursive(html: &str, nodes: &mut Vec<ParsedWebviewNode>) {
    let mut rest = html.trim();
    while !rest.is_empty() {
        if let Some(start_idx) = rest.find('<') {
            let leading_text = rest[..start_idx].trim();
            if !leading_text.is_empty() {
                let decoded = decode_html_entities(leading_text);
                if !decoded.trim().is_empty() {
                    nodes.push(ParsedWebviewNode {
                        kind: WebviewElementKind::Paragraph(decoded),
                    });
                }
            }

            if let Some(end_idx) = rest[start_idx..].find('>') {
                let full_tag_end = start_idx + end_idx;
                let tag_content = rest[start_idx + 1..full_tag_end].trim();
                let after_tag = &rest[full_tag_end + 1..];

                if tag_content.starts_with('/') {
                    rest = after_tag;
                    continue;
                }

                let tag_name = tag_content
                    .trim_end_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                let is_void = matches!(tag_name.as_str(), "hr" | "br" | "img" | "input" | "meta" | "link") || tag_content.ends_with('/');

                if is_void {
                    if tag_name == "hr" {
                        nodes.push(ParsedWebviewNode {
                            kind: WebviewElementKind::HorizontalRule,
                        });
                    }
                    rest = after_tag;
                    continue;
                }

                let (inner, next_rest) = find_matching_close_tag(&tag_name, after_tag);

                match tag_name.as_str() {
                    "h1" => {
                        let text = strip_and_decode(inner);
                        if !text.is_empty() {
                            nodes.push(ParsedWebviewNode {
                                kind: WebviewElementKind::Heading1(text),
                            });
                        }
                    }
                    "h2" => {
                        let text = strip_and_decode(inner);
                        if !text.is_empty() {
                            nodes.push(ParsedWebviewNode {
                                kind: WebviewElementKind::Heading2(text),
                            });
                        }
                    }
                    "h3" | "h4" | "h5" | "h6" => {
                        let text = strip_and_decode(inner);
                        if !text.is_empty() {
                            nodes.push(ParsedWebviewNode {
                                kind: WebviewElementKind::Heading3(text),
                            });
                        }
                    }
                    "pre" | "code" => {
                        let text = decode_html_entities(inner).trim().to_string();
                        if !text.is_empty() {
                            nodes.push(ParsedWebviewNode {
                                kind: WebviewElementKind::CodeBlock(text),
                            });
                        }
                    }
                    "button" => {
                        let label = strip_and_decode(inner);
                        let mut payload = serde_json::json!({
                            "command": "click",
                            "text": label,
                        });
                        if let Some(id) = extract_attribute(tag_content, "id") {
                            payload["id"] = serde_json::json!(id);
                        }
                        nodes.push(ParsedWebviewNode {
                            kind: WebviewElementKind::Button {
                                label,
                                message_payload: payload,
                            },
                        });
                    }
                    "li" => {
                        let text = strip_and_decode(inner);
                        if !text.is_empty() {
                            nodes.push(ParsedWebviewNode {
                                kind: WebviewElementKind::ListItem(text),
                            });
                        }
                    }
                    "p" => {
                        if has_block_child_tags(inner) {
                            parse_nodes_recursive(inner, nodes);
                        } else {
                            let text = strip_and_decode(inner);
                            if !text.is_empty() {
                                nodes.push(ParsedWebviewNode {
                                    kind: WebviewElementKind::Paragraph(text),
                                });
                            }
                        }
                    }
                    "div" | "span" | "section" | "article" | "main" | "header" | "footer"
                    | "aside" | "nav" | "ul" | "ol" | "html" | "body" | "form" | "table"
                    | "tbody" | "thead" | "tr" | "td" | "th" => {
                        if has_child_tags(inner) {
                            parse_nodes_recursive(inner, nodes);
                        } else {
                            let text = strip_and_decode(inner);
                            if !text.is_empty() {
                                nodes.push(ParsedWebviewNode {
                                    kind: WebviewElementKind::Paragraph(text),
                                });
                            }
                        }
                    }
                    _ => {
                        if has_child_tags(inner) {
                            parse_nodes_recursive(inner, nodes);
                        } else {
                            let text = strip_and_decode(inner);
                            if !text.is_empty() {
                                nodes.push(ParsedWebviewNode {
                                    kind: WebviewElementKind::Paragraph(text),
                                });
                            }
                        }
                    }
                }

                rest = next_rest;
            } else {
                break;
            }
        } else {
            let decoded = decode_html_entities(rest.trim());
            if !decoded.is_empty() {
                nodes.push(ParsedWebviewNode {
                    kind: WebviewElementKind::Paragraph(decoded),
                });
            }
            break;
        }
    }
}

impl View for WebviewEditorView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        // Base Webview background
        scene.push_quad(Quad::new(bounds).with_background(theme.editor_background));

        let mut cur_y = bounds.min_y() + 20.0 - self.scroll_top;
        let left_x = bounds.min_x() + 24.0;
        let content_w = (bounds.width() - 48.0).max(100.0);

        let mut targets = Vec::new();

        for node in &self.nodes {
            match &node.kind {
                WebviewElementKind::Heading1(text) => {
                    let h1_size = font.font_size * 1.5;
                    let line_h = font.line_height * 1.5;
                    scene.push_text(GlyphRun::new(
                        text.clone(),
                        Point::new(left_x, cur_y),
                        Rect::from_xywh(left_x, cur_y, content_w, line_h),
                        theme.editor_foreground,
                        h1_size,
                    ));
                    cur_y += line_h + 12.0;
                }
                WebviewElementKind::Heading2(text) => {
                    let h2_size = font.font_size * 1.25;
                    let line_h = font.line_height * 1.25;
                    scene.push_text(GlyphRun::new(
                        text.clone(),
                        Point::new(left_x, cur_y),
                        Rect::from_xywh(left_x, cur_y, content_w, line_h),
                        theme.editor_foreground,
                        h2_size,
                    ));
                    cur_y += line_h + 10.0;
                }
                WebviewElementKind::Heading3(text) => {
                    let h3_size = font.font_size * 1.1;
                    scene.push_text(GlyphRun::new(
                        text.clone(),
                        Point::new(left_x, cur_y),
                        Rect::from_xywh(left_x, cur_y, content_w, font.line_height),
                        theme.editor_foreground,
                        h3_size,
                    ));
                    cur_y += font.line_height + 8.0;
                }
                WebviewElementKind::Paragraph(text) => {
                    scene.push_text(GlyphRun::new(
                        text.clone(),
                        Point::new(left_x, cur_y),
                        Rect::from_xywh(left_x, cur_y, content_w, font.line_height),
                        theme.editor_foreground,
                        font.font_size,
                    ));
                    cur_y += font.line_height + 8.0;
                }
                WebviewElementKind::ListItem(text) => {
                    let bullet_text = format!("•  {}", text);
                    scene.push_text(GlyphRun::new(
                        bullet_text,
                        Point::new(left_x + 8.0, cur_y),
                        Rect::from_xywh(left_x + 8.0, cur_y, content_w - 8.0, font.line_height),
                        theme.editor_foreground,
                        font.font_size,
                    ));
                    cur_y += font.line_height + 4.0;
                }
                WebviewElementKind::CodeBlock(code) => {
                    let block_lines: Vec<&str> = code.lines().collect();
                    let block_h = (block_lines.len() as f32 * font.line_height + 12.0).max(28.0);
                    let block_rect = Rect::from_xywh(left_x, cur_y, content_w, block_h);

                    scene.push_quad(
                        Quad::new(block_rect)
                            .with_background(theme.command_palette_bg)
                            .with_border_edges(theme.panel_border, Edges::all(1.0))
                            .with_corner_radius(4.0),
                    );

                    for (l_idx, line) in block_lines.iter().enumerate() {
                        let line_y = cur_y + 6.0 + l_idx as f32 * font.line_height;
                        scene.push_text(GlyphRun::new(
                            line.to_string(),
                            Point::new(left_x + 10.0, line_y),
                            Rect::from_xywh(left_x + 10.0, line_y, content_w - 20.0, font.line_height),
                            theme.editor_foreground,
                            font.font_size * 0.9,
                        ));
                    }

                    cur_y += block_h + 12.0;
                }
                WebviewElementKind::Button { label, message_payload } => {
                    let btn_w = font.text_width(label) + 24.0;
                    let btn_h = 28.0;
                    let btn_rect = Rect::from_xywh(left_x, cur_y, btn_w, btn_h);
                    targets.push((btn_rect, message_payload.clone()));

                    scene.push_quad(
                        Quad::new(btn_rect)
                            .with_background(theme.command_palette_selected_bg)
                            .with_border_edges(theme.panel_border, Edges::all(1.0))
                            .with_corner_radius(4.0),
                    );

                    scene.push_text(GlyphRun::new(
                        label.clone(),
                        Point::new(left_x + 12.0, cur_y + 4.0),
                        Rect::from_xywh(left_x + 12.0, cur_y + 4.0, btn_w - 24.0, font.line_height),
                        theme.editor_foreground,
                        font.font_size * 0.9,
                    ));

                    cur_y += btn_h + 12.0;
                }
                WebviewElementKind::HorizontalRule => {
                    let hr_rect = Rect::from_xywh(left_x, cur_y + 6.0, content_w, 1.0);
                    scene.push_quad(Quad::new(hr_rect).with_background(theme.panel_border));
                    cur_y += 14.0;
                }
            }
        }

        *self.button_targets.write() = targets;
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::MouseDown(mouse) => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }

                let targets = self.button_targets.read().clone();
                for (rect, payload) in targets {
                    if rect.contains(mouse.position) {
                        self.send_message_to_host(payload);
                        return EventResult::Consumed;
                    }
                }
                EventResult::Ignored
            }
            InputEvent::ScrollWheel(scroll) => {
                if bounds.contains(scroll.position) {
                    self.scroll_top = (self.scroll_top + scroll.delta_y).max(0.0);
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webview_parsing_and_interactive_button() {
        let html = r#"
            <h1>Cat Coding</h1>
            <p>Welcome to our custom webview extension.</p>
            <hr>
            <button>Click Me</button>
        "#;

        let mut webview = WebviewEditorView::new(
            "webview_test",
            "catCoding",
            "Cat Coding",
            html,
            WebviewOptions {
                enable_scripts: true,
                local_resource_roots: vec![],
            },
            None,
        );

        assert_eq!(webview.nodes.len(), 4);
        assert!(matches!(webview.nodes[0].kind, WebviewElementKind::Heading1(_)));
        assert!(matches!(webview.nodes[1].kind, WebviewElementKind::Paragraph(_)));
        assert!(matches!(webview.nodes[2].kind, WebviewElementKind::HorizontalRule));
        assert!(matches!(webview.nodes[3].kind, WebviewElementKind::Button { .. }));

        let bounds = Rect::from_xywh(0.0, 0.0, 600.0, 500.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        let mut scene = Scene::new();
        webview.render(bounds, &theme, &font, &mut scene);
        assert!(!scene.quads().is_empty());

        // Click the button
        let targets = webview.button_targets.read().clone();
        assert_eq!(targets.len(), 1);
        let btn_rect = targets[0].0;

        let click = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(btn_rect.min_x() + 5.0, btn_rect.min_y() + 5.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = webview.handle_event(&click, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(webview.outbound_messages.len(), 1);
        assert_eq!(webview.outbound_messages[0]["text"], "Click Me");
    }

    #[test]
    fn test_webview_full_document_html_parsing() {
        let full_html = r#"
            <!DOCTYPE html>
            <html lang="en">
            <head>
                <meta charset="UTF-8">
                <title>Ignored Head</title>
                <style>body { color: red; }</style>
                <script>console.log("ignore script content");</script>
            </head>
            <body>
                <!-- Test comment -->
                <div class="main-wrapper">
                    <h1>Title &amp; Header</h1>
                    <p>Paragraph with <strong>bold text</strong> and &lt;escaped&gt; entities.</p>
                    <div class="actions">
                        <button id="submit_btn">Submit Action</button>
                    </div>
                    <hr/>
                    <ul>
                        <li>Item &quot;One&quot;</li>
                        <li>Item &#39;Two&#39;</li>
                    </ul>
                </div>
            </body>
            </html>
        "#;

        let nodes = WebviewEditorView::parse_html(full_html);
        assert_eq!(nodes.len(), 6);

        assert_eq!(
            nodes[0].kind,
            WebviewElementKind::Heading1("Title & Header".to_string())
        );
        assert_eq!(
            nodes[1].kind,
            WebviewElementKind::Paragraph("Paragraph with bold text and <escaped> entities.".to_string())
        );
        assert!(matches!(
            &nodes[2].kind,
            WebviewElementKind::Button { label, message_payload }
                if label == "Submit Action" && message_payload["id"] == "submit_btn"
        ));
        assert_eq!(nodes[3].kind, WebviewElementKind::HorizontalRule);
        assert_eq!(
            nodes[4].kind,
            WebviewElementKind::ListItem("Item \"One\"".to_string())
        );
        assert_eq!(
            nodes[5].kind,
            WebviewElementKind::ListItem("Item 'Two'".to_string())
        );
    }
}
