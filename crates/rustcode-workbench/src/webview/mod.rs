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

    /// Simple pure-Rust HTML tag scanner converting tags into renderable nodes.
    pub fn parse_html(html: &str) -> Vec<ParsedWebviewNode> {
        let mut nodes = Vec::new();
        let mut rest = html.trim();

        while !rest.is_empty() {
            if let Some(start_idx) = rest.find('<') {
                if let Some(end_idx) = rest[start_idx..].find('>') {
                    let full_tag_end = start_idx + end_idx;
                    let tag_content = &rest[start_idx + 1..full_tag_end].trim();
                    let tag_name = tag_content.split_whitespace().next().unwrap_or("").to_lowercase();
                    let after_tag = &rest[full_tag_end + 1..];

                    if tag_name.starts_with('/') {
                        rest = after_tag;
                        continue;
                    }

                    let is_void = matches!(tag_name.as_str(), "hr" | "br" | "img" | "input") || tag_content.ends_with('/');
                    let (clean_inner, next_rest) = if is_void {
                        ("", after_tag)
                    } else {
                        let close_tag = format!("</{}>", tag_name);
                        if let Some(close_idx) = after_tag.find(&close_tag) {
                            (after_tag[..close_idx].trim(), &after_tag[close_idx + close_tag.len()..])
                        } else {
                            (after_tag.trim(), "")
                        }
                    };

                    match tag_name.as_str() {
                        "h1" => nodes.push(ParsedWebviewNode {
                            kind: WebviewElementKind::Heading1(clean_inner.to_string()),
                        }),
                        "h2" => nodes.push(ParsedWebviewNode {
                            kind: WebviewElementKind::Heading2(clean_inner.to_string()),
                        }),
                        "h3" => nodes.push(ParsedWebviewNode {
                            kind: WebviewElementKind::Heading3(clean_inner.to_string()),
                        }),
                        "p" | "div" | "span" => {
                            if !clean_inner.is_empty() {
                                nodes.push(ParsedWebviewNode {
                                    kind: WebviewElementKind::Paragraph(clean_inner.to_string()),
                                });
                            }
                        }
                        "pre" | "code" => nodes.push(ParsedWebviewNode {
                            kind: WebviewElementKind::CodeBlock(clean_inner.to_string()),
                        }),
                        "button" => {
                            // Extract data-payload or onclick if present, else send text
                            let payload = serde_json::json!({
                                "command": "click",
                                "text": clean_inner,
                            });
                            nodes.push(ParsedWebviewNode {
                                kind: WebviewElementKind::Button {
                                    label: clean_inner.to_string(),
                                    message_payload: payload,
                                },
                            });
                        }
                        "hr" => nodes.push(ParsedWebviewNode {
                            kind: WebviewElementKind::HorizontalRule,
                        }),
                        "li" => nodes.push(ParsedWebviewNode {
                            kind: WebviewElementKind::ListItem(clean_inner.to_string()),
                        }),
                        _ => {
                            if !clean_inner.is_empty() && !clean_inner.starts_with('<') {
                                nodes.push(ParsedWebviewNode {
                                    kind: WebviewElementKind::Paragraph(clean_inner.to_string()),
                                });
                            }
                        }
                    }

                    rest = next_rest;
                } else {
                    break;
                }
            } else {
                if !rest.trim().is_empty() {
                    nodes.push(ParsedWebviewNode {
                        kind: WebviewElementKind::Paragraph(rest.trim().to_string()),
                    });
                }
                break;
            }
        }

        nodes
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
}
