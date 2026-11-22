//! Webview panel container, OS WebView backend (`wry`), and pure-Rust fallback renderer.
//!
//! Hosts custom extension HTML views (`vscode.window.createWebviewPanel`) and provides
//! a bidirectional MessagePort / RPC bridge (`postMessage`, `onDidReceiveMessage`).

#[cfg(feature = "desktop")]
use parking_lot::Mutex;
use parking_lot::RwLock;
use rustcode_ext_host::webview::{ExtHostWebviewManager, WebviewOptions};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use serde_json::Value;
#[cfg(feature = "desktop")]
use std::sync::Arc;

/// Backend abstraction for hosting WebView HTML and bidirectional `postMessage` IPC.
pub trait WebviewBackend: Send + Sync {
    fn load_html(&mut self, html: &str);
    fn post_message(&mut self, payload: &Value) -> Result<(), String>;
    fn drain_outbound_messages(&mut self) -> Vec<Value>;
    fn set_bounds(&mut self, bounds: Rect);
    fn set_visible(&mut self, visible: bool);
    fn current_html(&self) -> &str;
}

/// Deterministic in-memory WebView backend for headless CI and unit tests.
#[derive(Debug, Clone)]
pub struct HeadlessWebviewBackend {
    html: String,
    posted_messages: Vec<Value>,
    outbound_queue: Vec<Value>,
    bounds: Rect,
    visible: bool,
}

impl Default for HeadlessWebviewBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessWebviewBackend {
    pub fn new() -> Self {
        Self {
            html: String::new(),
            posted_messages: Vec::new(),
            outbound_queue: Vec::new(),
            bounds: Rect::from_xywh(0.0, 0.0, 200.0, 200.0),
            visible: true,
        }
    }

    pub fn with_html(html: &str) -> Self {
        let mut backend = Self::new();
        backend.load_html(html);
        backend
    }

    pub fn inject_outbound_message(&mut self, payload: Value) {
        self.outbound_queue.push(payload);
    }

    pub fn posted_messages(&self) -> &[Value] {
        &self.posted_messages
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }
}

impl WebviewBackend for HeadlessWebviewBackend {
    fn load_html(&mut self, html: &str) {
        self.html = html.to_string();
    }

    fn post_message(&mut self, payload: &Value) -> Result<(), String> {
        self.posted_messages.push(payload.clone());
        Ok(())
    }

    fn drain_outbound_messages(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.outbound_queue)
    }

    fn set_bounds(&mut self, bounds: Rect) {
        if bounds.is_finite() {
            self.bounds = bounds;
        }
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    fn current_html(&self) -> &str {
        &self.html
    }
}

#[cfg(feature = "desktop")]
const VSCODE_WEBVIEW_IPC_SHIM_JS: &str = r#"
(function() {
    if (typeof window.acquireVsCodeApi !== 'function') {
        var __state = undefined;
        window.acquireVsCodeApi = function() {
            return {
                postMessage: function(msg) {
                    if (window.ipc && typeof window.ipc.postMessage === 'function') {
                        window.ipc.postMessage(JSON.stringify(msg));
                    }
                },
                getState: function() { return __state; },
                setState: function(next) { __state = next; return next; }
            };
        };
    }
})();
"#;

#[cfg(feature = "desktop")]
fn to_wry_rect(bounds: Rect) -> wry::Rect {
    let x = if bounds.min_x().is_finite() {
        bounds.min_x().max(0.0) as f64
    } else {
        0.0
    };
    let y = if bounds.min_y().is_finite() {
        bounds.min_y().max(0.0) as f64
    } else {
        0.0
    };
    let w = if bounds.width().is_finite() {
        bounds.width().max(1.0) as f64
    } else {
        200.0
    };
    let h = if bounds.height().is_finite() {
        bounds.height().max(1.0) as f64
    } else {
        200.0
    };
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(x, y).into(),
        size: wry::dpi::LogicalSize::new(w, h).into(),
    }
}

#[cfg(feature = "desktop")]
fn format_post_message_script(payload: &Value) -> Result<String, String> {
    let serialized = serde_json::to_string(payload)
        .map_err(|e| format!("Failed to serialize webview postMessage payload: {e}"))?
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    Ok(format!(
        "window.dispatchEvent(new MessageEvent('message', {{ data: {} }}));",
        serialized
    ))
}

#[cfg(feature = "desktop")]
struct SendSyncWryWebView {
    inner: wry::WebView,
    owner_thread: std::thread::ThreadId,
}

#[cfg(feature = "desktop")]
// SAFETY: All calls touching `wry::WebView` are guarded by `owner_thread` matching
// the UI thread that constructed it and synchronized through `parking_lot::Mutex`.
unsafe impl Send for SendSyncWryWebView {}
#[cfg(feature = "desktop")]
unsafe impl Sync for SendSyncWryWebView {}

/// Native OS WebView backend powered by `wry` (WebView2 / WebKitGTK / WKWebView).
#[cfg(feature = "desktop")]
pub struct WryWebviewBackend {
    html: String,
    enable_scripts: bool,
    bounds: Rect,
    visible: bool,
    pending_inbound: Vec<Value>,
    outbound_queue: Arc<Mutex<Vec<Value>>>,
    webview: Mutex<Option<SendSyncWryWebView>>,
}

#[cfg(feature = "desktop")]
impl Default for WryWebviewBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "desktop")]
impl WryWebviewBackend {
    pub fn new() -> Self {
        Self::with_options(true)
    }

    pub fn with_options(enable_scripts: bool) -> Self {
        Self {
            html: String::new(),
            enable_scripts,
            bounds: Rect::from_xywh(0.0, 0.0, 200.0, 200.0),
            visible: true,
            pending_inbound: Vec::new(),
            outbound_queue: Arc::new(Mutex::new(Vec::new())),
            webview: Mutex::new(None),
        }
    }

    pub fn is_attached(&self) -> bool {
        self.webview.lock().is_some()
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn pending_inbound_messages(&self) -> &[Value] {
        &self.pending_inbound
    }

    pub fn attach_to_window<W>(&mut self, window: &Arc<W>) -> Result<(), String>
    where
        W: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    {
        self.attach_to_wry_parent(window.as_ref())
    }

    pub fn attach_to_wry_parent<W>(&mut self, parent: &W) -> Result<(), String>
    where
        W: raw_window_handle::HasWindowHandle,
    {
        let outbound = Arc::clone(&self.outbound_queue);
        let initial_html = if self.html.is_empty() {
            "<!DOCTYPE html><html><body></body></html>".to_string()
        } else {
            self.html.clone()
        };

        let mut builder = wry::WebViewBuilder::new()
            .with_bounds(to_wry_rect(self.bounds))
            .with_visible(self.visible)
            .with_initialization_script(VSCODE_WEBVIEW_IPC_SHIM_JS)
            .with_ipc_handler(move |request| {
                let body = request.body();
                let payload = serde_json::from_str::<Value>(body)
                    .unwrap_or_else(|_| Value::String(body.clone()));
                outbound.lock().push(payload);
            })
            .with_html(initial_html);

        if !self.enable_scripts {
            builder = builder.with_javascript_disabled();
        }

        let built = builder
            .build_as_child(parent)
            .map_err(|e| format!("Failed to create wry child WebView: {e}"))?;

        let pending = std::mem::take(&mut self.pending_inbound);
        for msg in pending {
            let script = format_post_message_script(&msg)?;
            let _ = built.evaluate_script(&script);
        }

        *self.webview.lock() = Some(SendSyncWryWebView {
            inner: built,
            owner_thread: std::thread::current().id(),
        });
        Ok(())
    }
}

#[cfg(feature = "desktop")]
impl WebviewBackend for WryWebviewBackend {
    fn load_html(&mut self, html: &str) {
        self.html = html.to_string();
        let guard = self.webview.lock();
        if let Some(wv) = guard.as_ref() {
            if wv.owner_thread == std::thread::current().id() {
                let _ = wv.inner.load_html(html);
            }
        }
    }

    fn post_message(&mut self, payload: &Value) -> Result<(), String> {
        let script = format_post_message_script(payload)?;
        let guard = self.webview.lock();
        if let Some(wv) = guard.as_ref() {
            if wv.owner_thread != std::thread::current().id() {
                return Err("WryWebviewBackend::post_message called from non-owner thread".to_string());
            }
            wv.inner
                .evaluate_script(&script)
                .map_err(|e| format!("wry evaluate_script failed: {e}"))
        } else {
            drop(guard);
            self.pending_inbound.push(payload.clone());
            Ok(())
        }
    }

    fn drain_outbound_messages(&mut self) -> Vec<Value> {
        std::mem::take(&mut *self.outbound_queue.lock())
    }

    fn set_bounds(&mut self, bounds: Rect) {
        if bounds.is_finite() {
            self.bounds = bounds;
            let guard = self.webview.lock();
            if let Some(wv) = guard.as_ref() {
                if wv.owner_thread == std::thread::current().id() {
                    let _ = wv.inner.set_bounds(to_wry_rect(bounds));
                }
            }
        }
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        let guard = self.webview.lock();
        if let Some(wv) = guard.as_ref() {
            if wv.owner_thread == std::thread::current().id() {
                let _ = wv.inner.set_visible(visible);
            }
        }
    }

    fn current_html(&self) -> &str {
        &self.html
    }
}

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
    pub backend: Box<dyn WebviewBackend>,
    pub last_bounds: RwLock<Option<Rect>>,
}

impl WebviewEditorView {
    pub fn sanitize_html(html: &str, enable_scripts: bool) -> String {
        if enable_scripts {
            html.to_string()
        } else {
            let mut out = html.to_string();
            strip_tag_block(&mut out, "script");
            out
        }
    }

    pub fn new(
        id: &str,
        view_type: &str,
        title: &str,
        html: &str,
        options: WebviewOptions,
        ext_host_manager: Option<ExtHostWebviewManager>,
    ) -> Self {
        let nodes = Self::parse_html(html);
        let prepared = Self::sanitize_html(html, options.enable_scripts);
        let backend: Box<dyn WebviewBackend> = Box::new(HeadlessWebviewBackend::with_html(&prepared));
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
            backend,
            last_bounds: RwLock::new(None),
        }
    }

    pub fn with_backend(mut self, mut backend: Box<dyn WebviewBackend>) -> Self {
        let prepared = Self::sanitize_html(&self.html, self.options.enable_scripts);
        backend.load_html(&prepared);
        if let Some(bounds) = *self.last_bounds.read() {
            backend.set_bounds(bounds);
        }
        self.backend = backend;
        self
    }

    pub fn set_backend(&mut self, mut backend: Box<dyn WebviewBackend>) {
        let prepared = Self::sanitize_html(&self.html, self.options.enable_scripts);
        backend.load_html(&prepared);
        if let Some(bounds) = *self.last_bounds.read() {
            backend.set_bounds(bounds);
        }
        self.backend = backend;
    }

    #[cfg(feature = "desktop")]
    pub fn upgrade_to_wry<W>(&mut self, window: Option<&Arc<W>>) -> Result<(), String>
    where
        W: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    {
        let mut wry_backend = WryWebviewBackend::with_options(self.options.enable_scripts);
        let prepared = Self::sanitize_html(&self.html, self.options.enable_scripts);
        wry_backend.load_html(&prepared);
        if let Some(bounds) = *self.last_bounds.read() {
            wry_backend.set_bounds(bounds);
        }
        if let Some(win) = window {
            wry_backend.attach_to_window(win)?;
        }
        self.backend = Box::new(wry_backend);
        Ok(())
    }

    pub fn set_html(&mut self, html: &str) {
        self.html = html.to_string();
        self.nodes = Self::parse_html(html);
        let prepared = Self::sanitize_html(html, self.options.enable_scripts);
        self.backend.load_html(&prepared);
    }

    /// Post a message from extension host into the webview backend & history.
    pub fn post_message_from_host(&mut self, msg: Value) -> Result<(), String> {
        self.inbound_messages.push(msg.clone());
        self.backend.post_message(&msg)
    }

    /// Receive message from extension host into webview.
    pub fn receive_message_from_host(&mut self, msg: Value) {
        let _ = self.post_message_from_host(msg);
    }

    /// Send message from webview out to extension host.
    pub fn send_message_to_host(&mut self, msg: Value) {
        self.outbound_messages.push(msg.clone());
        if let Some(mgr) = &self.ext_host_manager {
            let _ = mgr.receive_message_from_webview(&self.id, msg);
        }
    }

    /// Drain outbound IPC messages from the active `WebviewBackend` and forward to extension host.
    pub fn drain_messages_to_host(&mut self) -> Vec<Value> {
        let msgs = self.backend.drain_outbound_messages();
        for msg in &msgs {
            self.outbound_messages.push(msg.clone());
            if let Some(mgr) = &self.ext_host_manager {
                let _ = mgr.receive_message_from_webview(&self.id, msg.clone());
            }
        }
        msgs
    }

    pub fn sync_backend_bounds(&mut self, bounds: Rect) {
        if bounds.is_finite() {
            *self.last_bounds.write() = Some(bounds);
            self.backend.set_bounds(bounds);
        }
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.backend.set_visible(visible);
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
        if !bounds.is_finite_positive() || !font.is_finite() {
            return;
        }
        *self.last_bounds.write() = Some(bounds);

        // Base Webview background
        scene.push_finite_quad(Quad::new(bounds).with_background(theme.editor_background));

        let safe_scroll = if self.scroll_top.is_finite() {
            self.scroll_top.max(0.0)
        } else {
            0.0
        };
        let mut cur_y = bounds.min_y() + 20.0 - safe_scroll;
        let left_x = bounds.min_x() + 24.0;
        let content_w = (bounds.width() - 48.0).max(20.0);
        if !cur_y.is_finite() || !left_x.is_finite() || !content_w.is_finite() {
            *self.button_targets.write() = Vec::new();
            return;
        }

        scene.push_clip(bounds);
        let mut targets = Vec::new();

        for node in &self.nodes {
            if !cur_y.is_finite() {
                break;
            }
            match &node.kind {
                WebviewElementKind::Heading1(text) => {
                    let h1_size = font.font_size * 1.5;
                    let line_h = font.line_height * 1.5;
                    scene.push_finite_text(
                        GlyphRun::new(
                            text.clone(),
                            Point::new(left_x, cur_y),
                            Rect::from_xywh(left_x, cur_y, content_w, line_h),
                            theme.editor_foreground,
                            h1_size,
                        ),
                    );
                    cur_y += line_h + 12.0;
                }
                WebviewElementKind::Heading2(text) => {
                    let h2_size = font.font_size * 1.25;
                    let line_h = font.line_height * 1.25;
                    scene.push_finite_text(
                        GlyphRun::new(
                            text.clone(),
                            Point::new(left_x, cur_y),
                            Rect::from_xywh(left_x, cur_y, content_w, line_h),
                            theme.editor_foreground,
                            h2_size,
                        ),
                    );
                    cur_y += line_h + 10.0;
                }
                WebviewElementKind::Heading3(text) => {
                    let h3_size = font.font_size * 1.1;
                    scene.push_finite_text(
                        GlyphRun::new(
                            text.clone(),
                            Point::new(left_x, cur_y),
                            Rect::from_xywh(left_x, cur_y, content_w, font.line_height),
                            theme.editor_foreground,
                            h3_size,
                        ),
                    );
                    cur_y += font.line_height + 8.0;
                }
                WebviewElementKind::Paragraph(text) => {
                    scene.push_finite_text(
                        GlyphRun::new(
                            text.clone(),
                            Point::new(left_x, cur_y),
                            Rect::from_xywh(left_x, cur_y, content_w, font.line_height),
                            theme.editor_foreground,
                            font.font_size,
                        ),
                    );
                    cur_y += font.line_height + 8.0;
                }
                WebviewElementKind::ListItem(text) => {
                    let bullet_text = format!("•  {}", text);
                    scene.push_finite_text(
                        GlyphRun::new(
                            bullet_text,
                            Point::new(left_x + 8.0, cur_y),
                            Rect::from_xywh(left_x + 8.0, cur_y, content_w - 8.0, font.line_height),
                            theme.editor_foreground,
                            font.font_size,
                        ),
                    );
                    cur_y += font.line_height + 4.0;
                }
                WebviewElementKind::CodeBlock(code) => {
                    let block_lines: Vec<&str> = code.lines().collect();
                    let block_h = (block_lines.len() as f32 * font.line_height + 12.0).max(28.0);
                    let block_rect = Rect::from_xywh(left_x, cur_y, content_w, block_h);

                    scene.push_finite_quad(
                        Quad::new(block_rect)
                            .with_background(theme.command_palette_bg)
                            .with_border_edges(theme.panel_border, Edges::all(1.0))
                            .with_corner_radius(4.0),
                    );

                    for (l_idx, line) in block_lines.iter().enumerate() {
                        let line_y = cur_y + 6.0 + l_idx as f32 * font.line_height;
                        if !line_y.is_finite() || !(line_y + font.line_height).is_finite() {
                            break;
                        }
                        scene.push_finite_text(
                            GlyphRun::new(
                                line.to_string(),
                                Point::new(left_x + 10.0, line_y),
                                Rect::from_xywh(
                                    left_x + 10.0,
                                    line_y,
                                    (content_w - 20.0).max(0.0),
                                    font.line_height,
                                ),
                                theme.editor_foreground,
                                font.font_size * 0.9,
                            ),
                        );
                    }

                    cur_y += block_h + 12.0;
                }
                WebviewElementKind::Button { label, message_payload } => {
                    let btn_w = font.text_width(label) + 24.0;
                    let btn_h = 28.0;
                    let btn_rect = Rect::from_xywh(left_x, cur_y, btn_w, btn_h);
                    if btn_rect.is_finite_positive() {
                        targets.push((btn_rect, message_payload.clone()));

                        scene.push_finite_quad(
                            Quad::new(btn_rect)
                                .with_background(theme.command_palette_selected_bg)
                                .with_border_edges(theme.panel_border, Edges::all(1.0))
                                .with_corner_radius(4.0),
                        );

                        scene.push_finite_text(
                            GlyphRun::new(
                                label.clone(),
                                Point::new(left_x + 12.0, cur_y + 4.0),
                                Rect::from_xywh(
                                    left_x + 12.0,
                                    cur_y + 4.0,
                                    (btn_w - 24.0).max(0.0),
                                    font.line_height,
                                ),
                                theme.editor_foreground,
                                font.font_size * 0.9,
                            ),
                        );
                    }

                    cur_y += btn_h + 12.0;
                }
                WebviewElementKind::HorizontalRule => {
                    let hr_rect = Rect::from_xywh(left_x, cur_y + 6.0, content_w, 1.0);
                    scene.push_finite_quad(Quad::new(hr_rect).with_background(theme.panel_border));
                    cur_y += 14.0;
                }
            }
        }

        scene.pop_clip();
        *self.button_targets.write() = targets;
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if !bounds.is_finite_positive() {
            return EventResult::Ignored;
        }
        match event {
            InputEvent::MouseDown(mouse) => {
                if !mouse.position.x.is_finite()
                    || !mouse.position.y.is_finite()
                    || !bounds.contains(mouse.position)
                {
                    return EventResult::Ignored;
                }

                let targets = self.button_targets.read().clone();
                for (rect, payload) in targets {
                    if rect.is_finite_positive() && rect.contains(mouse.position) {
                        self.send_message_to_host(payload);
                        return EventResult::Consumed;
                    }
                }
                EventResult::Ignored
            }
            InputEvent::ScrollWheel(scroll) => {
                if !scroll.position.x.is_finite()
                    || !scroll.position.y.is_finite()
                    || !bounds.contains(scroll.position)
                {
                    return EventResult::Ignored;
                }
                let safe_scroll = if self.scroll_top.is_finite() {
                    self.scroll_top.max(0.0)
                } else {
                    0.0
                };
                if !scroll.delta_y.is_finite() {
                    self.scroll_top = safe_scroll;
                    return EventResult::Ignored;
                }
                let next = (safe_scroll + scroll.delta_y).max(0.0);
                self.scroll_top = if next.is_finite() {
                    next
                } else {
                    safe_scroll
                };
                EventResult::Consumed
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

    #[test]
    fn test_webview_non_finite_bounds_font_and_scroll_no_poison() {
        use rustcode_platform::render::{ScrollEvent, Size};

        let mut webview = WebviewEditorView::new(
            "wv_guard",
            "test",
            "Guard Test",
            "<h1>Hello</h1><p>Body</p><button>Go</button>",
            WebviewOptions::default(),
            None,
        );
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for bad_bounds in [
            Rect::new(Point::zero(), Size::new(800.0, f32::NAN)),
            Rect::from_xywh(0.0, 0.0, f32::INFINITY, 600.0),
            Rect::from_xywh(0.0, 0.0, 0.0, 600.0),
        ] {
            let mut scene = Scene::new();
            webview.render(bad_bounds, &theme, &font, &mut scene);
            assert!(scene.is_empty());
        }

        let valid_bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let extreme_font = FontMetrics {
            font_size: f32::MAX,
            line_height: f32::MAX,
            char_width: f32::MAX,
            ..font.clone()
        };
        let mut extreme_scene = Scene::new();
        webview.render(valid_bounds, &theme, &extreme_font, &mut extreme_scene);
        assert!(extreme_scene.is_empty());

        webview.scroll_top = 40.0;
        for bad_dy in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            webview.handle_event(
                &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(100.0, 100.0), 0.0, bad_dy)),
                valid_bounds,
            );
            assert!(webview.scroll_top.is_finite());
        }
        assert_eq!(webview.scroll_top, 40.0);

        let mut scene = Scene::new();
        webview.render(valid_bounds, &theme, &font, &mut scene);
        assert!(!scene.is_empty());
        for q in scene.quads() {
            assert!(q.bounds.is_finite());
        }
    }

    #[test]
    fn test_webview_backend_headless_post_message_bounds_and_script_sanitization() {
        use rustcode_ext_host::webview::ExtHostCustomEditorManager;
        use std::sync::Arc;

        let ext_wv = ExtHostWebviewManager::new();
        let ext_ce = ExtHostCustomEditorManager::new();
        let received = Arc::new(RwLock::new(Vec::new()));
        let received_clone = Arc::clone(&received);

        ext_ce.register_provider(
            "catEdit.canvas",
            WebviewOptions {
                enable_scripts: false,
                local_resource_roots: vec![],
            },
            false,
        );
        let session = ext_ce.open_session(
            Some("ce_wv_1"),
            "catEdit.canvas",
            "file:///art.canvas",
            Some("art.canvas"),
            Some(&ext_wv),
        );
        ext_wv.on_did_receive_message(&session.panel_id, move |msg| {
            received_clone.write().push(msg);
        });

        let mut backend = HeadlessWebviewBackend::new();
        backend.inject_outbound_message(serde_json::json!({ "command": "ready", "zoom": 1.5 }));

        let mut view = WebviewEditorView::new(
            &session.panel_id,
            &session.view_type,
            &session.title,
            "<script>alert(1)</script><h1>Canvas</h1>",
            WebviewOptions {
                enable_scripts: false,
                local_resource_roots: vec![],
            },
            Some(ext_wv.clone()),
        )
        .with_backend(Box::new(backend));

        // enable_scripts == false strips script blocks from backend HTML while preserving parsed heading
        assert!(!view.backend.current_html().contains("<script>"));
        assert!(view.backend.current_html().contains("<h1>Canvas</h1>"));

        // Re-inject outbound IPC message after with_backend replaced backend, then drain
        let mut custom_backend = HeadlessWebviewBackend::new();
        custom_backend.inject_outbound_message(serde_json::json!({ "command": "save", "stroke": 3 }));
        view.set_backend(Box::new(custom_backend));
        let drained = view.drain_messages_to_host();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0]["command"], "save");
        assert_eq!(received.read().len(), 1);
        assert_eq!(received.read()[0]["stroke"], 3);

        // Host -> Webview post_message_from_host & bounds/visibility forwarding
        view.post_message_from_host(serde_json::json!({ "command": "render", "layer": 2 }))
            .unwrap();
        assert_eq!(view.inbound_messages.len(), 1);
        assert_eq!(view.inbound_messages[0]["layer"], 2);

        let bounds = Rect::from_xywh(12.0, 40.0, 640.0, 480.0);
        view.sync_backend_bounds(bounds);
        assert_eq!(*view.last_bounds.read(), Some(bounds));
        view.set_visible(false);
        view.set_html("<h2>Updated</h2>");
        assert!(view.backend.current_html().contains("<h2>Updated</h2>"));

        // Close custom editor session cleanly
        assert!(ext_ce.close_session(&session.id, Some(&ext_wv)));
        assert!(ext_wv.get_panel(&session.panel_id).unwrap().is_disposed);
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn test_wry_webview_backend_unattached_fallback_and_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<WryWebviewBackend>();

        let mut backend = WryWebviewBackend::new();
        assert!(!backend.is_attached());
        backend.load_html("<h1>Desktop Wry</h1>");
        assert_eq!(backend.current_html(), "<h1>Desktop Wry</h1>");

        let rect = Rect::from_xywh(10.0, 20.0, 500.0, 400.0);
        backend.set_bounds(rect);
        assert_eq!(backend.bounds(), rect);
        backend.set_visible(false);
        assert!(!backend.is_visible());

        // Unattached post_message buffers cleanly instead of panicking
        backend
            .post_message(&serde_json::json!({ "type": "ping" }))
            .unwrap();
        assert_eq!(backend.pending_inbound_messages().len(), 1);
        assert!(backend.drain_outbound_messages().is_empty());
    }
}
