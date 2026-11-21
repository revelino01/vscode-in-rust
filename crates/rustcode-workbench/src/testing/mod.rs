//! Testing Sidebar View (`workbench.view.testing`) wired to `ExtHostTestManager`.

use parking_lot::RwLock;
use rustcode_ext_host::testing::{
    ExtHostTestManager, ExtTestControllerState, ExtTestEvent, ExtTestItemState,
    ExtTestResultRecord,
};
use rustcode_platform::accessibility::{AccessibleNode, AccessibleRole};
use rustcode_platform::render::{
    EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestingAction {
    RunAll,
    RunController(String),
    OpenUri(String),
    SelectItem(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestingTreeRow {
    pub controller_id: String,
    pub item_id: String,
    pub label: String,
    pub depth: usize,
    pub is_controller: bool,
    pub has_children: bool,
    pub is_expanded: bool,
    pub uri: Option<String>,
    pub state: Option<String>,
    pub duration_ms: Option<u64>,
    pub failure_message: Option<String>,
}

#[derive(Debug, Default)]
struct TestingSidebarState {
    controllers: Vec<ExtTestControllerState>,
    known_run_ids: Vec<String>,
    results_by_controller: HashMap<String, HashMap<String, ExtTestResultRecord>>,
    selected_item_id: Option<String>,
    collapsed_ids: HashSet<String>,
    scroll_top: f32,
    synced_revision: u64,
    failed_count: usize,
}

#[derive(Debug, Clone)]
enum TestingRowHit {
    RunAll,
    Row {
        controller_id: String,
        item_id: String,
        uri: Option<String>,
        is_controller: bool,
    },
}

#[derive(Debug, Clone)]
struct RenderedRowLayout {
    row: TestingTreeRow,
    rect: Rect,
    indent_x: f32,
    detail_rect: Option<Rect>,
}

#[derive(Debug, Clone)]
struct TestingLayout {
    header_rect: Rect,
    run_all_rect: Rect,
    body_rect: Rect,
    rows: Vec<RenderedRowLayout>,
}

fn clamp_rect(rect: Rect, bounds: Rect) -> Rect {
    if !crate::is_finite_rect(bounds) || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return Rect::zero();
    }
    let rx = if rect.origin.x.is_finite() {
        rect.origin.x
    } else {
        bounds.min_x()
    };
    let ry = if rect.origin.y.is_finite() {
        rect.origin.y
    } else {
        bounds.min_y()
    };
    let rw = if rect.size.width.is_finite() && rect.size.width >= 0.0 {
        rect.size.width
    } else {
        0.0
    };
    let rh = if rect.size.height.is_finite() && rect.size.height >= 0.0 {
        rect.size.height
    } else {
        0.0
    };

    let min_x = rx.clamp(bounds.min_x(), bounds.max_x());
    let min_y = ry.clamp(bounds.min_y(), bounds.max_y());
    let max_x = (rx + rw).clamp(min_x, bounds.max_x());
    let max_y = (ry + rh).clamp(min_y, bounds.max_y());
    Rect::from_xywh(
        min_x,
        min_y,
        (max_x - min_x).max(0.0),
        (max_y - min_y).max(0.0),
    )
}

fn extract_test_message(val: &Value) -> Option<String> {
    match val {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Object(obj) => match obj.get("message") {
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
            Some(Value::Object(inner)) => inner
                .get("value")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string),
            _ => None,
        },
        _ => None,
    }
}

/// Decode a `file://` URI (Windows `file:///C:/...` or POSIX `file:///...`) or local path.
pub fn decode_test_uri_to_path(uri: &str) -> Option<PathBuf> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(rustcode_platform::file_uri_to_path(trimmed))
}

pub struct TestingSidebarView {
    state: RwLock<TestingSidebarState>,
    pub last_action: Option<TestingAction>,
    last_font: RwLock<FontMetrics>,
}

impl Default for TestingSidebarView {
    fn default() -> Self {
        Self::new()
    }
}

impl TestingSidebarView {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(TestingSidebarState::default()),
            last_action: None,
            last_font: RwLock::new(FontMetrics::default_monospace()),
        }
    }

    pub fn failed_count(&self) -> usize {
        self.state.read().failed_count
    }

    pub fn selected_item_id(&self) -> Option<String> {
        self.state.read().selected_item_id.clone()
    }

    /// Synchronize active controllers and test run results from `ExtHostTestManager`.
    /// Returns the total count of currently failed/errored tests across non-disposed controllers.
    pub fn sync_from_manager(&self, mgr: &ExtHostTestManager) -> usize {
        let rev = mgr.revision();
        {
            let guard = self.state.read();
            if guard.synced_revision == rev {
                return guard.failed_count;
            }
        }

        let events = mgr.take_pending_events();
        let mut guard = self.state.write();

        for ev in events {
            match ev {
                ExtTestEvent::RunCreated { run_id, .. }
                | ExtTestEvent::TestStateUpdated { run_id, .. }
                | ExtTestEvent::RunOutputAppended { run_id, .. }
                | ExtTestEvent::RunEnded { run_id } => {
                    if !guard.known_run_ids.contains(&run_id) {
                        guard.known_run_ids.push(run_id);
                    }
                }
                ExtTestEvent::ResultsInvalidated { controller_id } => {
                    guard.results_by_controller.remove(&controller_id);
                    guard.known_run_ids.retain(|rid| {
                        mgr.get_run(rid)
                            .is_some_and(|r| r.controller_id != controller_id)
                    });
                }
                _ => {}
            }
        }

        let mut controllers: Vec<ExtTestControllerState> = mgr
            .list_controllers()
            .into_iter()
            .filter(|c| !c.is_disposed)
            .collect();
        controllers.sort_by(|a, b| {
            a.label
                .to_ascii_lowercase()
                .cmp(&b.label.to_ascii_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });

        let active_ctrl_ids: HashSet<String> = controllers.iter().map(|c| c.id.clone()).collect();
        let mut results_by_ctrl: HashMap<String, HashMap<String, ExtTestResultRecord>> =
            HashMap::new();

        let run_ids = guard.known_run_ids.clone();
        for run_id in run_ids {
            if let Some(run) = mgr.get_run(&run_id) {
                if !active_ctrl_ids.contains(&run.controller_id) {
                    continue;
                }
                let entry = results_by_ctrl.entry(run.controller_id.clone()).or_default();
                for (test_id, record) in run.results {
                    entry.insert(test_id, record);
                }
            }
        }

        // Compute failed count across items belonging to active controllers
        let mut failed_count = 0usize;
        for ctrl in &controllers {
            let Some(ctrl_results) = results_by_ctrl.get(&ctrl.id) else {
                continue;
            };
            let mut seen_ids = HashSet::new();
            let mut stack: Vec<&ExtTestItemState> = ctrl.items.iter().collect();
            while let Some(item) = stack.pop() {
                if seen_ids.insert(item.id.as_str()) {
                    if let Some(rec) = ctrl_results.get(&item.id) {
                        if rec.state == "failed" || rec.state == "errored" {
                            failed_count += 1;
                        }
                    }
                }
                for child in &item.children {
                    stack.push(child);
                }
            }
        }

        guard.controllers = controllers;
        guard.results_by_controller = results_by_ctrl;
        guard.failed_count = failed_count;
        guard.synced_revision = rev;
        failed_count
    }

    /// Flatten active controllers and their test hierarchies iteratively (stack-safe for deep trees).
    pub fn flattened_rows(&self) -> Vec<TestingTreeRow> {
        let guard = self.state.read();
        let mut rows = Vec::new();

        for ctrl in &guard.controllers {
            if ctrl.is_disposed {
                continue;
            }
            let ctrl_collapsed = guard.collapsed_ids.contains(&format!("ctrl:{}", ctrl.id));
            rows.push(TestingTreeRow {
                controller_id: ctrl.id.clone(),
                item_id: ctrl.id.clone(),
                label: ctrl.label.clone(),
                depth: 0,
                is_controller: true,
                has_children: !ctrl.items.is_empty(),
                is_expanded: !ctrl_collapsed,
                uri: None,
                state: None,
                duration_ms: None,
                failure_message: None,
            });
            if ctrl_collapsed {
                continue;
            }

            let ctrl_results = guard.results_by_controller.get(&ctrl.id);
            let mut stack: Vec<(&ExtTestItemState, usize)> = Vec::new();
            for item in ctrl.items.iter().rev() {
                stack.push((item, 1));
            }
            while let Some((item, depth)) = stack.pop() {
                let rec = ctrl_results.and_then(|m| m.get(&item.id));
                let state = rec.map(|r| r.state.clone());
                let duration_ms = rec.and_then(|r| r.duration);
                let failure_message = rec.and_then(|r| {
                    r.messages
                        .iter()
                        .find_map(extract_test_message)
                });
                let item_key = format!("item:{}:{}", ctrl.id, item.id);
                let collapsed = guard.collapsed_ids.contains(&item_key);
                rows.push(TestingTreeRow {
                    controller_id: ctrl.id.clone(),
                    item_id: item.id.clone(),
                    label: item.label.clone(),
                    depth,
                    is_controller: false,
                    has_children: !item.children.is_empty(),
                    is_expanded: !collapsed,
                    uri: item.uri.clone(),
                    state,
                    duration_ms,
                    failure_message,
                });
                if !collapsed {
                    for child in item.children.iter().rev() {
                        stack.push((child, depth + 1));
                    }
                }
            }
        }
        rows
    }

    fn compute_layout(&self, bounds: Rect, font: &FontMetrics) -> TestingLayout {
        let header_h = 28.0f32.min(bounds.height().max(0.0));
        let header_rect = clamp_rect(
            Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), header_h),
            bounds,
        );

        let run_label = "Run All";
        let run_w = (font.text_width(run_label) + 12.0)
            .max(52.0)
            .min((bounds.width() * 0.45).max(0.0));
        let run_h = 20.0f32.min(header_h);
        let run_all_rect = clamp_rect(
            Rect::from_xywh(
                (header_rect.max_x() - run_w - 8.0).max(header_rect.min_x()),
                header_rect.min_y() + ((header_h - run_h) * 0.5).max(0.0),
                run_w,
                run_h,
            ),
            header_rect,
        );

        let body_rect = clamp_rect(
            Rect::from_xywh(
                bounds.min_x(),
                bounds.min_y() + header_h,
                bounds.width(),
                (bounds.height() - header_h).max(0.0),
            ),
            bounds,
        );

        let rows_data = self.flattened_rows();
        let scroll_top = self.state.read().scroll_top;
        let base_row_h = (font.line_height + 6.0).max(22.0);
        let detail_h = (font.line_height + 2.0).max(16.0);

        let mut layout_rows = Vec::with_capacity(rows_data.len());
        let mut cur_y = body_rect.min_y() - scroll_top;

        for row in rows_data {
            let has_detail = row.failure_message.is_some();
            let total_h = base_row_h + if has_detail { detail_h } else { 0.0 };
            if cur_y + total_h <= body_rect.min_y() {
                cur_y += total_h;
                continue;
            }
            if cur_y >= body_rect.max_y() {
                break;
            }

            let max_indent = (body_rect.width() - 72.0).max(0.0);
            let indent = (8.0 + row.depth as f32 * 12.0).min(max_indent);
            let row_rect = clamp_rect(
                Rect::from_xywh(
                    body_rect.min_x(),
                    cur_y,
                    body_rect.width(),
                    base_row_h.min((body_rect.max_y() - cur_y).max(0.0)),
                ),
                body_rect,
            );
            let detail_rect = if has_detail && cur_y + base_row_h < body_rect.max_y() {
                Some(clamp_rect(
                    Rect::from_xywh(
                        body_rect.min_x(),
                        cur_y + base_row_h,
                        body_rect.width(),
                        detail_h.min((body_rect.max_y() - (cur_y + base_row_h)).max(0.0)),
                    ),
                    body_rect,
                ))
            } else {
                None
            };

            layout_rows.push(RenderedRowLayout {
                row,
                rect: row_rect,
                indent_x: (body_rect.min_x() + indent).min(body_rect.max_x()),
                detail_rect,
            });
            cur_y += total_h;
        }

        TestingLayout {
            header_rect,
            run_all_rect,
            body_rect,
            rows: layout_rows,
        }
    }

    pub fn accessibility_nodes(&self, bounds: Rect, font: &FontMetrics) -> Vec<AccessibleNode> {
        let layout = self.compute_layout(bounds, font);
        let selected_id = self.selected_item_id();

        let run_btn = AccessibleNode::new(
            "testing.runAll",
            AccessibleRole::Button,
            "Run All",
            layout.run_all_rect,
        )
        .with_action("press");

        let mut tree = AccessibleNode::new(
            "workbench.sidebar.testing.tree",
            AccessibleRole::Tree,
            "Testing",
            layout.body_rect,
        );

        if layout.rows.is_empty() {
            tree.add_child(AccessibleNode::new(
                "testing.empty",
                AccessibleRole::StaticText,
                "No tests found",
                layout.body_rect,
            ));
        } else {
            for item_layout in &layout.rows {
                let row = &item_layout.row;
                let mut desc_parts = Vec::new();
                if let Some(st) = &row.state {
                    desc_parts.push(st.clone());
                }
                if let Some(ms) = row.duration_ms {
                    desc_parts.push(format!("{}ms", ms));
                }
                if let Some(msg) = &row.failure_message {
                    desc_parts.push(msg.clone());
                }
                let mut node = AccessibleNode::new(
                    format!("testing.item.{}.{}", row.controller_id, row.item_id),
                    AccessibleRole::TreeItem,
                    row.label.clone(),
                    item_layout.rect,
                )
                .with_selected(selected_id.as_deref() == Some(row.item_id.as_str()));
                if row.has_children {
                    node = node.with_expanded(row.is_expanded);
                }
                if !desc_parts.is_empty() {
                    node = node.with_description(desc_parts.join(" "));
                }
                tree.add_child(node);
            }
        }

        vec![run_btn, tree]
    }
}

impl View for TestingSidebarView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !crate::is_finite_positive_rect(bounds) || !crate::is_finite_font(font) {
            return;
        }
        *self.last_font.write() = font.clone();

        crate::push_finite_quad(scene, Quad::new(bounds).with_background(theme.sidebar_bg));

        let layout = self.compute_layout(bounds, font);
        let selected_id = self.selected_item_id();

        // Header ("TESTING" + "Run All" button)
        if layout.header_rect.height() > 0.0 && layout.header_rect.width() > 0.0 {
            let title_x = (layout.header_rect.min_x() + 12.0).min(layout.header_rect.max_x());
            let title_y = layout.header_rect.min_y()
                + ((layout.header_rect.height() - font.line_height) * 0.5).max(0.0);
            let title_w = (layout.run_all_rect.min_x() - title_x - 6.0).max(0.0);
            if title_w > 0.0 {
                let tr = clamp_rect(
                    Rect::from_xywh(
                        title_x,
                        title_y,
                        title_w,
                        font.line_height.min(layout.header_rect.height()),
                    ),
                    layout.header_rect,
                );
                crate::push_finite_text(
                    scene,
                    GlyphRun::new(
                        "TESTING".to_string(),
                        Point::new(title_x, title_y),
                        tr,
                        theme.sidebar_header_fg,
                        font.font_size * 0.85,
                    ),
                );
            }

            if layout.run_all_rect.width() > 4.0 && layout.run_all_rect.height() > 4.0 {
                crate::push_finite_quad(
                    scene,
                    Quad::new(layout.run_all_rect)
                        .with_background(theme.command_palette_selected_bg)
                        .with_corner_radius(3.0),
                );
                let btn_x = (layout.run_all_rect.min_x() + 6.0).min(layout.run_all_rect.max_x());
                let btn_y = layout.run_all_rect.min_y()
                    + ((layout.run_all_rect.height() - font.line_height) * 0.5).max(0.0);
                let br = clamp_rect(
                    Rect::from_xywh(
                        btn_x,
                        btn_y,
                        (layout.run_all_rect.max_x() - btn_x).max(0.0),
                        font.line_height.min(layout.run_all_rect.height()),
                    ),
                    layout.run_all_rect,
                );
                crate::push_finite_text(
                    scene,
                    GlyphRun::new(
                        "Run All".to_string(),
                        Point::new(btn_x, btn_y),
                        br,
                        theme.activity_bar_fg,
                        font.font_size * 0.85,
                    ),
                );
            }
        }

        if layout.rows.is_empty() {
            if layout.body_rect.height() > 0.0 && layout.body_rect.width() > 0.0 {
                let empty_x = (layout.body_rect.min_x() + 12.0).min(layout.body_rect.max_x());
                let empty_y = (layout.body_rect.min_y() + 12.0).min(layout.body_rect.max_y());
                let er = clamp_rect(
                    Rect::from_xywh(
                        empty_x,
                        empty_y,
                        (layout.body_rect.max_x() - empty_x - 8.0).max(0.0),
                        font.line_height.min((layout.body_rect.max_y() - empty_y).max(0.0)),
                    ),
                    layout.body_rect,
                );
                crate::push_finite_text(
                    scene,
                    GlyphRun::new(
                        "No tests found".to_string(),
                        Point::new(empty_x, empty_y),
                        er,
                        theme.tab_inactive_fg,
                        font.font_size * 0.9,
                    ),
                );
            }
            return;
        }

        for item_layout in &layout.rows {
            let row = &item_layout.row;
            if item_layout.rect.height() <= 0.0 || item_layout.rect.width() <= 0.0 {
                continue;
            }
            if selected_id.as_deref() == Some(row.item_id.as_str()) {
                crate::push_finite_quad(
                    scene,
                    Quad::new(item_layout.rect).with_background(theme.list_active_selection_bg),
                );
            }

            let text_y = item_layout.rect.min_y()
                + ((item_layout.rect.height() - font.line_height) * 0.5).max(0.0);
            let mut cur_x = item_layout.indent_x;

            if let Some(st) = &row.state {
                let (badge, badge_color) = match st.as_str() {
                    "passed" => ("[PASS]", theme.editor_info_foreground),
                    "failed" | "errored" => ("[FAIL]", theme.editor_error_foreground),
                    "skipped" => ("[SKIP]", theme.editor_warning_foreground),
                    "running" | "enqueued" => ("[RUN]", theme.activity_bar_active_border),
                    _ => ("[TEST]", theme.tab_inactive_fg),
                };
                let bw = font.text_width(badge).min((item_layout.rect.max_x() - cur_x).max(0.0));
                if bw > 0.0 {
                    let br = clamp_rect(
                        Rect::from_xywh(
                            cur_x,
                            text_y,
                            bw,
                            font.line_height.min(item_layout.rect.height()),
                        ),
                        item_layout.rect,
                    );
                    crate::push_finite_text(
                        scene,
                        GlyphRun::new(
                            badge.to_string(),
                            Point::new(cur_x, text_y),
                            br,
                            badge_color,
                            font.font_size * 0.85,
                        ),
                    );
                    cur_x = (cur_x + bw + 6.0).min(item_layout.rect.max_x());
                }
            }

            let dur_str = row.duration_ms.map(|ms| format!("{}ms", ms));
            let dur_w = dur_str
                .as_ref()
                .map(|s| font.text_width(s) + 8.0)
                .unwrap_or(0.0);

            let label_w = (item_layout.rect.max_x() - cur_x - dur_w - 4.0).max(0.0);
            if label_w > 0.0 {
                let lr = clamp_rect(
                    Rect::from_xywh(
                        cur_x,
                        text_y,
                        label_w,
                        font.line_height.min(item_layout.rect.height()),
                    ),
                    item_layout.rect,
                );
                let fg = if row.is_controller {
                    theme.sidebar_header_fg
                } else {
                    theme.sidebar_fg
                };
                crate::push_finite_text(
                    scene,
                    GlyphRun::new(
                        row.label.clone(),
                        Point::new(cur_x, text_y),
                        lr,
                        fg,
                        font.font_size * 0.9,
                    ),
                );
            }

            if let Some(d_text) = dur_str {
                let raw_dw = font.text_width(&d_text);
                let dx = (item_layout.rect.max_x() - raw_dw - 6.0).max(cur_x);
                let dw = raw_dw.min((item_layout.rect.max_x() - dx).max(0.0));
                if dw > 0.0 {
                    let dr = clamp_rect(
                        Rect::from_xywh(
                            dx,
                            text_y,
                            dw,
                            font.line_height.min(item_layout.rect.height()),
                        ),
                        item_layout.rect,
                    );
                    crate::push_finite_text(
                        scene,
                        GlyphRun::new(
                            d_text,
                            Point::new(dx, text_y),
                            dr,
                            theme.tab_inactive_fg,
                            font.font_size * 0.8,
                        ),
                    );
                }
            }

            if let (Some(msg), Some(d_rect)) = (&row.failure_message, item_layout.detail_rect) {
                if d_rect.width() > 0.0 && d_rect.height() > 0.0 {
                    let mx = (item_layout.indent_x + 12.0).min(d_rect.max_x());
                    let my = d_rect.min_y() + ((d_rect.height() - font.line_height) * 0.5).max(0.0);
                    let mr = clamp_rect(
                        Rect::from_xywh(
                            mx,
                            my,
                            (d_rect.max_x() - mx - 4.0).max(0.0),
                            font.line_height.min(d_rect.height()),
                        ),
                        d_rect,
                    );
                    crate::push_finite_text(
                        scene,
                        GlyphRun::new(
                            msg.clone(),
                            Point::new(mx, my),
                            mr,
                            theme.editor_error_foreground,
                            font.font_size * 0.85,
                        ),
                    );
                }
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let font = self.last_font.read().clone();
        match event {
            InputEvent::MouseDown(mouse) => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }
                let layout = self.compute_layout(bounds, &font);
                let mut hit: Option<TestingRowHit> = None;
                if layout.run_all_rect.contains(mouse.position) {
                    hit = Some(TestingRowHit::RunAll);
                } else {
                    for item_layout in &layout.rows {
                        let in_row = item_layout.rect.contains(mouse.position)
                            || item_layout
                                .detail_rect
                                .is_some_and(|dr| dr.contains(mouse.position));
                        if in_row {
                            hit = Some(TestingRowHit::Row {
                                controller_id: item_layout.row.controller_id.clone(),
                                item_id: item_layout.row.item_id.clone(),
                                uri: item_layout.row.uri.clone(),
                                is_controller: item_layout.row.is_controller,
                            });
                            break;
                        }
                    }
                }

                match hit {
                    Some(TestingRowHit::RunAll) => {
                        self.last_action = Some(TestingAction::RunAll);
                        EventResult::Consumed
                    }
                    Some(TestingRowHit::Row {
                        controller_id,
                        item_id,
                        uri,
                        is_controller,
                    }) => {
                        self.state.write().selected_item_id = Some(item_id.clone());
                        if let Some(u) = uri.filter(|s| !s.trim().is_empty()) {
                            self.last_action = Some(TestingAction::OpenUri(u));
                        } else if is_controller {
                            self.last_action = Some(TestingAction::RunController(controller_id));
                        } else {
                            self.last_action = Some(TestingAction::SelectItem(item_id));
                        }
                        EventResult::Consumed
                    }
                    None => EventResult::Consumed,
                }
            }
            InputEvent::ScrollWheel(scroll) => {
                if !bounds.contains(scroll.position) {
                    return EventResult::Ignored;
                }
                let mut guard = self.state.write();
                guard.scroll_top = (guard.scroll_top - scroll.delta_y).max(0.0);
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => {
                let rows = self.flattened_rows();
                if rows.is_empty() {
                    return EventResult::Ignored;
                }
                match key.key {
                    KeyCode::ArrowDown => {
                        let mut guard = self.state.write();
                        let next_idx = guard
                            .selected_item_id
                            .as_deref()
                            .and_then(|sel| rows.iter().position(|r| r.item_id == sel))
                            .map(|i| (i + 1).min(rows.len() - 1))
                            .unwrap_or(0);
                        guard.selected_item_id = Some(rows[next_idx].item_id.clone());
                        EventResult::Consumed
                    }
                    KeyCode::ArrowUp => {
                        let mut guard = self.state.write();
                        let prev_idx = guard
                            .selected_item_id
                            .as_deref()
                            .and_then(|sel| rows.iter().position(|r| r.item_id == sel))
                            .map(|i| i.saturating_sub(1))
                            .unwrap_or(0);
                        guard.selected_item_id = Some(rows[prev_idx].item_id.clone());
                        EventResult::Consumed
                    }
                    KeyCode::Enter => {
                        let sel = self.selected_item_id();
                        if let Some(row) =
                            sel.and_then(|id| rows.into_iter().find(|r| r.item_id == id))
                        {
                            if let Some(u) = row.uri.filter(|s| !s.trim().is_empty()) {
                                self.last_action = Some(TestingAction::OpenUri(u));
                            } else {
                                self.last_action = Some(TestingAction::SelectItem(row.item_id));
                            }
                            return EventResult::Consumed;
                        }
                        EventResult::Ignored
                    }
                    _ => EventResult::Ignored,
                }
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::render::{MouseButton, MouseEvent};
    use serde_json::json;

    #[test]
    fn test_testing_sidebar_sync_disposed_hiding_run_all_and_uri_decode() {
        let mgr = ExtHostTestManager::new();
        let mut view = TestingSidebarView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 280.0, 600.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        // 1. Empty state renders "No tests found"
        let mut empty_scene = Scene::new();
        view.render(bounds, &theme, &font, &mut empty_scene);
        assert!(empty_scene.contains_text("TESTING"));
        assert!(empty_scene.contains_text("No tests found"));

        // 2. Create two controllers, dispose one, record results on active one
        mgr.handle_rpc_call(
            "tests.createController",
            &json!({ "id": "active", "label": "Cargo Suite" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.createController",
            &json!({ "id": "disposed", "label": "Old Suite" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.updateItems",
            &json!({
                "controllerId": "active",
                "items": [
                    { "id": "t_pass", "label": "t_pass", "children": [] },
                    { "id": "t_fail", "label": "t_fail", "uri": "file:///C:/repo/tests/my%20test.rs", "children": [] }
                ]
            }),
        )
        .unwrap();
        mgr.handle_rpc_call("tests.disposeController", &json!({ "id": "disposed" }))
            .unwrap();
        mgr.handle_rpc_call(
            "tests.createTestRun",
            &json!({ "runId": "r1", "controllerId": "active" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.updateTestState",
            &json!({ "runId": "r1", "testId": "t_pass", "state": "passed", "duration": 12 }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.updateTestState",
            &json!({
                "runId": "r1",
                "testId": "t_fail",
                "state": "failed",
                "duration": 45,
                "messages": [{ "message": "expected 4 got 5" }]
            }),
        )
        .unwrap();
        mgr.handle_rpc_call("tests.endTestRun", &json!({ "runId": "r1" }))
            .unwrap();

        let failed = view.sync_from_manager(&mgr);
        assert_eq!(failed, 1);

        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("Cargo Suite"));
        assert!(!scene.contains_text("Old Suite"));
        assert!(scene.contains_text("12ms"));
        assert!(scene.contains_text("45ms"));
        assert!(scene.contains_text("expected 4 got 5"));

        // 3. Click "Run All" button
        let a11y = view.accessibility_nodes(bounds, &font);
        let run_bounds = a11y.iter().find(|n| n.name == "Run All").unwrap().bounds;
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(
                    run_bounds.min_x() + run_bounds.width() * 0.5,
                    run_bounds.min_y() + run_bounds.height() * 0.5,
                ),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(view.last_action, Some(TestingAction::RunAll));

        // 4. Click item without URI -> SelectItem (no OpenUri)
        let tree_node = a11y
            .iter()
            .find(|n| n.role == AccessibleRole::Tree)
            .unwrap();
        let pass_bounds = tree_node
            .children
            .iter()
            .find(|c| c.name == "t_pass")
            .unwrap()
            .bounds;
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(
                    pass_bounds.min_x() + 10.0,
                    pass_bounds.min_y() + pass_bounds.height() * 0.5,
                ),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(
            view.last_action,
            Some(TestingAction::SelectItem("t_pass".to_string()))
        );

        // 5. Click item with URI -> OpenUri, and verify Windows & POSIX URI decoding
        let fail_bounds = tree_node
            .children
            .iter()
            .find(|c| c.name == "t_fail")
            .unwrap()
            .bounds;
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(
                    fail_bounds.min_x() + 10.0,
                    fail_bounds.min_y() + fail_bounds.height() * 0.5,
                ),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(
            view.last_action,
            Some(TestingAction::OpenUri(
                "file:///C:/repo/tests/my%20test.rs".to_string()
            ))
        );

        let posix_path = decode_test_uri_to_path("file:///home/dev/my%20spec.rs").unwrap();
        assert_eq!(
            posix_path.to_string_lossy().replace('\\', "/"),
            "/home/dev/my spec.rs"
        );
        assert!(decode_test_uri_to_path("   ").is_none());

        // Tiny viewport (240x180 and 120x80) clamping safety
        for (w, h) in [(240.0, 180.0), (120.0, 80.0)] {
            let tiny = Rect::from_xywh(0.0, 0.0, w, h);
            let mut tiny_scene = Scene::new();
            view.render(tiny, &theme, &font, &mut tiny_scene);
            for q in tiny_scene.quads() {
                assert!(
                    q.bounds.min_x().is_finite()
                        && q.bounds.min_y().is_finite()
                        && q.bounds.width().is_finite()
                        && q.bounds.height().is_finite()
                        && q.bounds.width() >= 0.0
                        && q.bounds.height() >= 0.0
                        && q.bounds.max_x() <= w + 0.01
                        && q.bounds.max_y() <= h + 0.01
                );
            }
        }
    }
}
