//! Interactive Output Panel View supporting structured channels, dropdown selection, and scrollback.

use parking_lot::RwLock;
use rustcode_platform::accessibility::{AccessibleNode, AccessibleRole};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Scene,
    View, WorkbenchThemeStyles,
};

pub const DEFAULT_MAX_SCROLLBACK: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputAction {
    SelectChannel(String),
    ClearChannel(String),
    ToggleAutoScroll(bool),
    ToggleChannelDropdown(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputClickTarget {
    ChannelDropdownToggle,
    ChannelOption(String),
    ClearButton,
    AutoScrollToggle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputChannelData {
    pub id: String,
    pub name: String,
    pub lines: Vec<String>,
    pub partial_line: String,
    pub visible: bool,
    pub ends_with_cr: bool,
}

#[derive(Debug, Clone)]
struct OutputPanelLayout {
    toolbar_rect: Rect,
    selector_rect: Rect,
    clear_rect: Rect,
    auto_scroll_rect: Rect,
    log_rect: Rect,
    dropdown_rect: Rect,
    option_rects: Vec<(String, Rect)>,
}

pub struct OutputPanelView {
    pub channels: Vec<OutputChannelData>,
    pub active_channel_id: Option<String>,
    pub auto_scroll: bool,
    pub dropdown_open: bool,
    pub scroll_top: f32,
    pub max_scrollback: usize,
    pub last_action: Option<OutputAction>,
    pub click_targets: RwLock<Vec<(Rect, OutputClickTarget)>>,
    last_line_height: RwLock<f32>,
    last_content_height: RwLock<f32>,
    last_font: RwLock<FontMetrics>,
}

impl Default for OutputPanelView {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputPanelView {
    pub fn new() -> Self {
        Self {
            channels: Vec::new(),
            active_channel_id: None,
            auto_scroll: true,
            dropdown_open: false,
            scroll_top: 0.0,
            max_scrollback: DEFAULT_MAX_SCROLLBACK,
            last_action: None,
            click_targets: RwLock::new(Vec::new()),
            last_line_height: RwLock::new(18.0),
            last_content_height: RwLock::new(160.0),
            last_font: RwLock::new(FontMetrics::default_monospace()),
        }
    }

    pub fn ensure_channel(&mut self, id: &str, name: &str) -> &mut OutputChannelData {
        if let Some(pos) = self.channels.iter().position(|c| c.id == id) {
            if self.channels[pos].name != name {
                self.channels[pos].name = name.to_string();
            }
            return &mut self.channels[pos];
        }
        self.channels.push(OutputChannelData {
            id: id.to_string(),
            name: name.to_string(),
            lines: Vec::new(),
            partial_line: String::new(),
            visible: false,
            ends_with_cr: false,
        });
        if self.active_channel_id.is_none() {
            self.active_channel_id = Some(id.to_string());
        }
        self.channels.last_mut().unwrap()
    }

    pub fn select_channel(&mut self, id_or_name: &str) -> bool {
        if let Some(ch) = self
            .channels
            .iter()
            .find(|c| c.id == id_or_name || c.name == id_or_name)
        {
            let changed = self.active_channel_id.as_deref() != Some(ch.id.as_str());
            self.active_channel_id = Some(ch.id.clone());
            self.dropdown_open = false;
            if changed {
                self.scroll_top = 0.0;
            }
            true
        } else {
            false
        }
    }

    fn find_channel_index_mut(&mut self, id_or_name: &str) -> Option<usize> {
        self.channels
            .iter()
            .position(|c| c.id == id_or_name || c.name == id_or_name)
    }

    fn append_into_channel(ch: &mut OutputChannelData, text: &str, max_scrollback: usize) {
        if text.is_empty() {
            return;
        }
        let input = if ch.ends_with_cr {
            ch.ends_with_cr = false;
            text.strip_prefix('\n').unwrap_or(text)
        } else {
            text
        };
        if input.is_empty() {
            return;
        }
        ch.ends_with_cr = input.ends_with('\r');
        let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
        let combined = format!("{}{}", ch.partial_line, normalized);
        let mut parts = combined.split('\n').peekable();
        ch.partial_line.clear();
        while let Some(part) = parts.next() {
            if parts.peek().is_some() {
                ch.lines.push(part.to_string());
            } else {
                ch.partial_line = part.to_string();
            }
        }
        if max_scrollback > 0 && ch.lines.len() > max_scrollback {
            let excess = ch.lines.len() - max_scrollback;
            ch.lines.drain(0..excess);
        }
    }

    pub fn append(&mut self, id_or_name: &str, text: &str) {
        let max_scrollback = self.max_scrollback;
        if let Some(idx) = self.find_channel_index_mut(id_or_name) {
            Self::append_into_channel(&mut self.channels[idx], text, max_scrollback);
        } else {
            let ch = self.ensure_channel(id_or_name, id_or_name);
            Self::append_into_channel(ch, text, max_scrollback);
        }
    }

    pub fn append_line(&mut self, id_or_name: &str, line: &str) {
        self.append(id_or_name, &format!("{}\n", line));
    }

    pub fn replace(&mut self, id_or_name: &str, text: &str) {
        let max_scrollback = self.max_scrollback;
        if let Some(idx) = self.find_channel_index_mut(id_or_name) {
            self.channels[idx].lines.clear();
            self.channels[idx].partial_line.clear();
            self.channels[idx].ends_with_cr = false;
            Self::append_into_channel(&mut self.channels[idx], text, max_scrollback);
        } else {
            let ch = self.ensure_channel(id_or_name, id_or_name);
            Self::append_into_channel(ch, text, max_scrollback);
        }
        if self.active_channel_id.as_deref() == Some(id_or_name) {
            self.scroll_top = 0.0;
        }
    }

    pub fn sync_channel_content(&mut self, id: &str, name: &str, content: &str, visible: bool) {
        let max_scrollback = self.max_scrollback;
        let idx = if let Some(idx) = self.find_channel_index_mut(id) {
            if self.channels[idx].name != name {
                self.channels[idx].name = name.to_string();
            }
            idx
        } else {
            self.ensure_channel(id, name);
            self.channels.len() - 1
        };
        let ch = &mut self.channels[idx];
        ch.visible = visible;
        ch.lines.clear();
        ch.partial_line.clear();
        ch.ends_with_cr = false;
        Self::append_into_channel(ch, content, max_scrollback);
        if self.active_channel_id.as_deref() == Some(id) && self.auto_scroll {
            let row_h = *self.last_line_height.read();
            let content_h = *self.last_content_height.read();
            self.scroll_top = self.compute_max_scroll(row_h, content_h);
        }
    }

    pub fn clear_channel(&mut self, id_or_name: &str) -> bool {
        let Some(idx) = self.find_channel_index_mut(id_or_name) else {
            return false;
        };
        let id = self.channels[idx].id.clone();
        self.channels[idx].lines.clear();
        self.channels[idx].partial_line.clear();
        self.channels[idx].ends_with_cr = false;
        if self.active_channel_id.as_deref() == Some(id.as_str()) {
            self.scroll_top = 0.0;
        }
        true
    }

    pub fn clear_active(&mut self) -> Option<String> {
        let id = self.active_channel_id.clone()?;
        if self.clear_channel(&id) {
            Some(id)
        } else {
            None
        }
    }

    pub fn remove_channel(&mut self, id_or_name: &str) -> bool {
        let Some(idx) = self.find_channel_index_mut(id_or_name) else {
            return false;
        };
        let removed_id = self.channels.remove(idx).id;
        if self.active_channel_id.as_deref() == Some(removed_id.as_str()) {
            self.active_channel_id = self.channels.first().map(|c| c.id.clone());
            self.scroll_top = 0.0;
        }
        true
    }

    pub fn active_channel(&self) -> Option<&OutputChannelData> {
        let id = self.active_channel_id.as_deref()?;
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn display_lines(&self) -> Vec<&str> {
        let Some(ch) = self.active_channel() else {
            return Vec::new();
        };
        let mut lines: Vec<&str> = ch.lines.iter().map(|s| s.as_str()).collect();
        if !ch.partial_line.is_empty() {
            lines.push(ch.partial_line.as_str());
        }
        lines
    }

    fn compute_max_scroll(&self, line_h: f32, content_h: f32) -> f32 {
        let total_lines = self.display_lines().len();
        let total_h = total_lines as f32 * line_h.max(1.0);
        (total_h - content_h.max(0.0)).max(0.0)
    }

    fn compute_layout(&self, bounds: Rect, font: &FontMetrics) -> OutputPanelLayout {
        let toolbar_h = 26.0f32.min(bounds.height().max(0.0));
        let toolbar_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), toolbar_h);

        let pad_l = 8.0f32.min(bounds.width() * 0.1);
        let pad_r = 8.0f32.min(bounds.width() * 0.1);
        let gap = 4.0f32.min(bounds.width() * 0.05);
        let avail_w = (toolbar_rect.width() - pad_l - pad_r).max(0.0);

        let active_label = self
            .active_channel()
            .map(|c| format!("{} ▾", c.name))
            .unwrap_or_else(|| "No Channels ▾".to_string());
        let desired_selector_w = (font.text_width(&active_label) + 20.0)
            .clamp(110.0, (bounds.width() * 0.5).max(110.0));
        let btn_h = (toolbar_h - 4.0).max(0.0);
        let btn_y = toolbar_rect.min_y() + 2.0;

        let auto_label = if self.auto_scroll {
            "Auto-Scroll: ON"
        } else {
            "Auto-Scroll: OFF"
        };
        let desired_auto_w = (font.text_width(auto_label) + 14.0).max(96.0);
        let desired_clear_w = 50.0;
        let desired_sum = desired_selector_w + desired_clear_w + desired_auto_w;

        let (selector_w, clear_w, auto_w, selector_x, clear_x, auto_x) =
            if avail_w >= desired_sum + 12.0 {
                let sx = toolbar_rect.min_x() + 8.0;
                let ax = toolbar_rect.max_x() - 8.0 - desired_auto_w;
                let cx = ax - 6.0 - desired_clear_w;
                (desired_selector_w, desired_clear_w, desired_auto_w, sx, cx, ax)
            } else {
                let usable = (avail_w - 2.0 * gap).max(0.0);
                let scale = if desired_sum > 0.0 {
                    (usable / desired_sum).min(1.0)
                } else {
                    0.0
                };
                let sw = (desired_selector_w * scale).max(0.0);
                let cw = (desired_clear_w * scale).max(0.0);
                let aw = (desired_auto_w * scale).max(0.0);
                let sx = toolbar_rect.min_x() + pad_l;
                let cx = sx + sw + gap;
                let ax = cx + cw + gap;
                (sw, cw, aw, sx, cx, ax)
            };

        let selector_rect = Rect::from_xywh(selector_x, btn_y, selector_w, btn_h);
        let clear_rect = Rect::from_xywh(clear_x, btn_y, clear_w, btn_h);
        let auto_scroll_rect = Rect::from_xywh(auto_x, btn_y, auto_w, btn_h);

        let log_rect = Rect::from_xywh(
            bounds.min_x(),
            bounds.min_y() + toolbar_h,
            bounds.width(),
            (bounds.height() - toolbar_h).max(0.0),
        );

        let opt_h = 22.0f32;
        let dropdown_w = selector_w.max(160.0).min(bounds.width().max(0.0));
        let dropdown_h = (self.channels.len() as f32 * opt_h).min(log_rect.height().max(0.0));
        let dropdown_rect = Rect::from_xywh(
            selector_rect.min_x(),
            toolbar_rect.max_y(),
            dropdown_w,
            dropdown_h,
        );
        let mut option_rects = Vec::new();
        if self.dropdown_open && !self.channels.is_empty() {
            for (idx, ch) in self.channels.iter().enumerate() {
                let oy = dropdown_rect.min_y() + idx as f32 * opt_h;
                if oy + opt_h > bounds.max_y() {
                    break;
                }
                option_rects.push((
                    ch.id.clone(),
                    Rect::from_xywh(dropdown_rect.min_x(), oy, dropdown_w, opt_h),
                ));
            }
        }

        OutputPanelLayout {
            toolbar_rect,
            selector_rect,
            clear_rect,
            auto_scroll_rect,
            log_rect,
            dropdown_rect,
            option_rects,
        }
    }

    fn build_click_targets(&self, layout: &OutputPanelLayout) -> Vec<(Rect, OutputClickTarget)> {
        let mut targets = vec![
            (layout.selector_rect, OutputClickTarget::ChannelDropdownToggle),
            (layout.clear_rect, OutputClickTarget::ClearButton),
            (layout.auto_scroll_rect, OutputClickTarget::AutoScrollToggle),
        ];
        // Push dropdown options last so reverse scan hits overlay options first.
        for (id, rect) in &layout.option_rects {
            targets.push((*rect, OutputClickTarget::ChannelOption(id.clone())));
        }
        targets
    }

    pub fn accessibility_node(&self, bounds: Rect, font: &FontMetrics) -> AccessibleNode {
        if !bounds.is_finite_positive() || !font.is_finite() {
            return AccessibleNode::new(
                "workbench.panel.output",
                AccessibleRole::Panel,
                "Output",
                Rect::zero(),
            );
        }
        let layout = self.compute_layout(bounds, font);
        let mut root = AccessibleNode::new(
            "workbench.panel.output",
            AccessibleRole::Panel,
            "Output",
            bounds,
        );

        let active_name = self
            .active_channel()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "No Channels".to_string());
        root.add_child(
            AccessibleNode::new(
                "output.channelSelector",
                AccessibleRole::Button,
                active_name,
                layout.selector_rect,
            )
            .with_expanded(self.dropdown_open)
            .with_action("press"),
        );

        if self.dropdown_open {
            let mut list = AccessibleNode::new(
                "output.channelList",
                AccessibleRole::List,
                "Output Channels",
                layout.dropdown_rect,
            );
            for (id, rect) in &layout.option_rects {
                if let Some(ch) = self.channels.iter().find(|c| &c.id == id) {
                    list.add_child(
                        AccessibleNode::new(
                            format!("output.channel.{}", ch.id),
                            AccessibleRole::ListItem,
                            ch.name.clone(),
                            *rect,
                        )
                        .with_selected(self.active_channel_id.as_deref() == Some(ch.id.as_str())),
                    );
                }
            }
            root.add_child(list);
        }

        root.add_child(
            AccessibleNode::new(
                "output.autoScroll",
                AccessibleRole::Button,
                "Toggle Auto-Scroll",
                layout.auto_scroll_rect,
            )
            .with_value(if self.auto_scroll { "on" } else { "off" })
            .with_action("press"),
        );
        root.add_child(
            AccessibleNode::new(
                "output.clear",
                AccessibleRole::Button,
                "Clear Output",
                layout.clear_rect,
            )
            .with_action("press"),
        );

        let mut lines_node = AccessibleNode::new(
            "output.lines",
            AccessibleRole::List,
            "Output Lines",
            layout.log_rect,
        );
        let row_h = font.line_height.max(1.0);
        let lines = self.display_lines();
        let max_scroll = self.compute_max_scroll(row_h, layout.log_rect.height());
        let safe_scroll = if self.scroll_top.is_finite() {
            self.scroll_top.max(0.0)
        } else {
            0.0
        };
        let effective_scroll = if self.auto_scroll {
            max_scroll
        } else {
            safe_scroll.clamp(0.0, max_scroll)
        };
        let first_idx = ((effective_scroll / row_h).floor().max(0.0) as usize).min(lines.len());
        let visible_count = (layout.log_rect.height() / row_h).ceil().max(0.0) as usize + 1;
        let end_idx = (first_idx + visible_count).min(lines.len());
        for (idx, line) in lines[first_idx..end_idx].iter().enumerate() {
            let line_idx = first_idx + idx;
            let y = layout.log_rect.min_y() + line_idx as f32 * row_h - effective_scroll;
            lines_node.add_child(AccessibleNode::new(
                format!("output.line.{}", line_idx),
                AccessibleRole::ListItem,
                (*line).to_string(),
                Rect::from_xywh(
                    layout.log_rect.min_x() + 10.0,
                    y,
                    (layout.log_rect.width() - 20.0).max(0.0),
                    row_h,
                ),
            ));
        }
        root.add_child(lines_node);

        root
    }
}

impl View for OutputPanelView {
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
        *self.last_font.write() = font.clone();

        let layout = self.compute_layout(bounds, font);
        let row_h = font.line_height.max(1.0);
        *self.last_line_height.write() = row_h;
        *self.last_content_height.write() = layout.log_rect.height();

        scene.push_finite_quad(Quad::new(bounds).with_background(theme.panel_bg));

        // Toolbar
        scene.push_finite_quad(
            Quad::new(layout.toolbar_rect)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        let selector_label = self
            .active_channel()
            .map(|c| format!("{} ▾", c.name))
            .unwrap_or_else(|| "No Channels ▾".to_string());
        scene.push_finite_quad(
            Quad::new(layout.selector_rect)
                .with_background(theme.tab_inactive_bg)
                .with_border(theme.panel_border, 1.0)
                .with_corner_radius(3.0),
        );
        scene.push_finite_text(
            GlyphRun::new(
                selector_label,
                Point::new(layout.selector_rect.min_x() + 6.0, layout.selector_rect.min_y() + 3.0),
                layout.selector_rect,
                theme.tab_active_fg,
                font.font_size * 0.85,
            ),
        );

        scene.push_finite_quad(
            Quad::new(layout.clear_rect)
                .with_background(theme.tab_inactive_bg)
                .with_border(theme.panel_border, 1.0)
                .with_corner_radius(3.0),
        );
        scene.push_finite_text(
            GlyphRun::new(
                "Clear".to_string(),
                Point::new(layout.clear_rect.min_x() + 8.0, layout.clear_rect.min_y() + 3.0),
                layout.clear_rect,
                theme.tab_active_fg,
                font.font_size * 0.8,
            ),
        );

        let auto_label = if self.auto_scroll {
            "Auto-Scroll: ON"
        } else {
            "Auto-Scroll: OFF"
        };
        scene.push_finite_quad(
            Quad::new(layout.auto_scroll_rect)
                .with_background(if self.auto_scroll {
                    theme.list_active_selection_bg
                } else {
                    theme.tab_inactive_bg
                })
                .with_border(theme.panel_border, 1.0)
                .with_corner_radius(3.0),
        );
        scene.push_finite_text(
            GlyphRun::new(
                auto_label.to_string(),
                Point::new(
                    layout.auto_scroll_rect.min_x() + 6.0,
                    layout.auto_scroll_rect.min_y() + 3.0,
                ),
                layout.auto_scroll_rect,
                theme.tab_active_fg,
                font.font_size * 0.8,
            ),
        );

        // Log area
        scene.push_clip(layout.log_rect);
        let lines = self.display_lines();
        if self.channels.is_empty() {
            scene.push_finite_text(
                GlyphRun::new(
                    "No output channels.".to_string(),
                    Point::new(layout.log_rect.min_x() + 12.0, layout.log_rect.min_y() + 10.0),
                    Rect::from_xywh(
                        layout.log_rect.min_x() + 12.0,
                        layout.log_rect.min_y() + 10.0,
                        (layout.log_rect.width() - 24.0).max(0.0),
                        row_h,
                    ),
                    theme.sidebar_header_fg,
                    font.font_size * 0.9,
                ),
            );
        } else if lines.is_empty() {
            scene.push_finite_text(
                GlyphRun::new(
                    "Output channel is empty.".to_string(),
                    Point::new(layout.log_rect.min_x() + 12.0, layout.log_rect.min_y() + 10.0),
                    Rect::from_xywh(
                        layout.log_rect.min_x() + 12.0,
                        layout.log_rect.min_y() + 10.0,
                        (layout.log_rect.width() - 24.0).max(0.0),
                        row_h,
                    ),
                    theme.sidebar_header_fg,
                    font.font_size * 0.9,
                ),
            );
        } else {
            let max_scroll = self.compute_max_scroll(row_h, layout.log_rect.height());
            let safe_scroll = if self.scroll_top.is_finite() {
                self.scroll_top.max(0.0)
            } else {
                0.0
            };
            let effective_scroll = if self.auto_scroll {
                max_scroll
            } else {
                safe_scroll.clamp(0.0, max_scroll)
            };

            let first_idx = ((effective_scroll / row_h).floor().max(0.0) as usize).min(lines.len());
            let visible_count = (layout.log_rect.height() / row_h).ceil().max(0.0) as usize + 2;
            let end_idx = (first_idx + visible_count).min(lines.len());

            for (offset, line) in lines[first_idx..end_idx].iter().enumerate() {
                let idx = first_idx + offset;
                let y = layout.log_rect.min_y() + 4.0 + idx as f32 * row_h - effective_scroll;
                if y + row_h < layout.log_rect.min_y() || y > layout.log_rect.max_y() {
                    continue;
                }
                let color = if line.contains("[error]") || line.contains("ERROR") {
                    theme.editor_error_foreground
                } else if line.contains("[warn]") || line.contains("WARN") {
                    theme.editor_warning_foreground
                } else {
                    theme.editor_foreground
                };
                scene.push_finite_text(
                    GlyphRun::new(
                        (*line).to_string(),
                        Point::new(layout.log_rect.min_x() + 10.0, y),
                        Rect::from_xywh(
                            layout.log_rect.min_x() + 10.0,
                            y,
                            (layout.log_rect.width() - 20.0).max(0.0),
                            row_h,
                        ),
                        color,
                        font.font_size * 0.9,
                    ),
                );
            }
        }
        scene.pop_clip();

        // Dropdown overlay rendered after log area
        if self.dropdown_open && !layout.option_rects.is_empty() {
            scene.push_finite_quad(
                Quad::new(layout.dropdown_rect)
                    .with_background(theme.command_palette_bg)
                    .with_border(theme.command_palette_border, 1.0),
            );
            for (id, rect) in &layout.option_rects {
                let is_active = self.active_channel_id.as_deref() == Some(id.as_str());
                if is_active {
                    scene.push_finite_quad(
                        Quad::new(*rect).with_background(theme.list_active_selection_bg),
                    );
                }
                if let Some(ch) = self.channels.iter().find(|c| &c.id == id) {
                    scene.push_finite_text(
                        GlyphRun::new(
                            ch.name.clone(),
                            Point::new(rect.min_x() + 8.0, rect.min_y() + 3.0),
                            *rect,
                            theme.editor_foreground,
                            font.font_size * 0.85,
                        ),
                    );
                }
            }
        }

        *self.click_targets.write() = self.build_click_targets(&layout);
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

                let font = self.last_font.read().clone();
                let layout = self.compute_layout(bounds, &font);
                *self.last_content_height.write() = layout.log_rect.height();
                let targets = self.build_click_targets(&layout);
                *self.click_targets.write() = targets.clone();

                for (rect, target) in targets.into_iter().rev() {
                    if rect.contains(mouse.position) {
                        match target {
                            OutputClickTarget::ChannelDropdownToggle => {
                                self.dropdown_open = !self.dropdown_open;
                                self.last_action = Some(OutputAction::ToggleChannelDropdown(
                                    self.dropdown_open,
                                ));
                            }
                            OutputClickTarget::ChannelOption(id) => {
                                if self.select_channel(&id) {
                                    self.last_action = Some(OutputAction::SelectChannel(id));
                                }
                            }
                            OutputClickTarget::ClearButton => {
                                if let Some(id) = self.clear_active() {
                                    self.last_action = Some(OutputAction::ClearChannel(id));
                                }
                            }
                            OutputClickTarget::AutoScrollToggle => {
                                self.auto_scroll = !self.auto_scroll;
                                if self.auto_scroll {
                                    let row_h = *self.last_line_height.read();
                                    let content_h = *self.last_content_height.read();
                                    self.scroll_top = self.compute_max_scroll(row_h, content_h);
                                }
                                self.last_action =
                                    Some(OutputAction::ToggleAutoScroll(self.auto_scroll));
                            }
                        }
                        let updated_layout = self.compute_layout(bounds, &font);
                        *self.click_targets.write() = self.build_click_targets(&updated_layout);
                        return EventResult::Consumed;
                    }
                }

                if self.dropdown_open {
                    self.dropdown_open = false;
                    let updated_layout = self.compute_layout(bounds, &font);
                    *self.click_targets.write() = self.build_click_targets(&updated_layout);
                }
                EventResult::Consumed
            }
            InputEvent::ScrollWheel(scroll) => {
                if !scroll.position.x.is_finite()
                    || !scroll.position.y.is_finite()
                    || !bounds.contains(scroll.position)
                    || !scroll.delta_y.is_finite()
                {
                    return EventResult::Ignored;
                }

                let font = self.last_font.read().clone();
                let layout = self.compute_layout(bounds, &font);
                let row_h = font.line_height.max(1.0);
                let content_h = layout.log_rect.height();
                *self.last_line_height.write() = row_h;
                *self.last_content_height.write() = content_h;
                let max_scroll = self.compute_max_scroll(row_h, content_h);
                let safe_scroll = if self.scroll_top.is_finite() {
                    self.scroll_top.clamp(0.0, max_scroll)
                } else {
                    0.0
                };
                let base_scroll = if self.auto_scroll {
                    max_scroll
                } else {
                    safe_scroll
                };
                let next = (base_scroll - scroll.delta_y).clamp(0.0, max_scroll);
                self.scroll_top = next;

                if scroll.delta_y > 0.0 {
                    self.auto_scroll = false;
                } else if scroll.delta_y < 0.0 && (max_scroll == 0.0 || next >= max_scroll - 0.5) {
                    self.auto_scroll = true;
                }
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => {
                let font = self.last_font.read().clone();
                let layout = self.compute_layout(bounds, &font);
                let row_h = font.line_height.max(1.0);
                let content_h = layout.log_rect.height();
                *self.last_line_height.write() = row_h;
                *self.last_content_height.write() = content_h;
                let max_scroll = self.compute_max_scroll(row_h, content_h);
                let current = if self.auto_scroll {
                    max_scroll
                } else if self.scroll_top.is_finite() {
                    self.scroll_top.clamp(0.0, max_scroll)
                } else {
                    0.0
                };

                match key.key {
                    KeyCode::PageUp => {
                        self.auto_scroll = false;
                        self.scroll_top = (current - content_h.max(row_h)).clamp(0.0, max_scroll);
                        EventResult::Consumed
                    }
                    KeyCode::PageDown => {
                        let next = (current + content_h.max(row_h)).clamp(0.0, max_scroll);
                        self.scroll_top = next;
                        if max_scroll == 0.0 || next >= max_scroll - 0.5 {
                            self.auto_scroll = true;
                        }
                        EventResult::Consumed
                    }
                    KeyCode::ArrowUp => {
                        self.auto_scroll = false;
                        self.scroll_top = (current - row_h).clamp(0.0, max_scroll);
                        EventResult::Consumed
                    }
                    KeyCode::ArrowDown => {
                        let next = (current + row_h).clamp(0.0, max_scroll);
                        self.scroll_top = next;
                        if max_scroll == 0.0 || next >= max_scroll - 0.5 {
                            self.auto_scroll = true;
                        }
                        EventResult::Consumed
                    }
                    KeyCode::Home => {
                        self.auto_scroll = false;
                        self.scroll_top = 0.0;
                        EventResult::Consumed
                    }
                    KeyCode::End => {
                        self.auto_scroll = true;
                        self.scroll_top = max_scroll;
                        EventResult::Consumed
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
    use rustcode_platform::render::{MouseButton, MouseEvent, SceneNode, ScrollEvent, Size};

    #[test]
    fn test_output_panel_append_replace_clear_and_channels() {
        let mut panel = OutputPanelView::new();
        panel.ensure_channel("out_1", "Build");
        panel.ensure_channel("out_2", "Extension Host");

        panel.append("out_1", "hello ");
        panel.append_line("out_1", "world");
        assert_eq!(panel.display_lines(), vec!["hello world"]);

        panel.replace("out_1", "fresh\nlines\n");
        assert_eq!(panel.display_lines(), vec!["fresh", "lines"]);

        assert!(panel.select_channel("out_2"));
        assert!(panel.display_lines().is_empty());
        panel.append_line("out_2", "ext ready");
        assert_eq!(panel.display_lines(), vec!["ext ready"]);

        assert_eq!(panel.clear_active(), Some("out_2".to_string()));
        assert!(panel.display_lines().is_empty());

        assert!(panel.remove_channel("out_2"));
        assert_eq!(panel.active_channel_id.as_deref(), Some("out_1"));
    }

    #[test]
    fn test_output_panel_dropdown_mouse_scroll_and_accessibility() {
        let mut panel = OutputPanelView::new();
        panel.ensure_channel("out_1", "Build");
        panel.ensure_channel("out_2", "Tests");
        for i in 0..40 {
            panel.append_line("out_1", &format!("log_line_{:02}", i));
        }

        let bounds = Rect::from_xywh(0.0, 0.0, 600.0, 160.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let mut scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("Build"));
        assert!(scene.contains_text("log_line_39"));
        assert!(!scene.contains_text("log_line_00"));

        // Scroll up disables auto_scroll and shows earlier lines
        assert_eq!(
            panel.handle_event(
                &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(100.0, 100.0), 0.0, 2000.0)),
                bounds,
            ),
            EventResult::Consumed
        );
        assert!(!panel.auto_scroll);
        let mut scrolled_scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scrolled_scene);
        assert!(scrolled_scene.contains_text("log_line_00"));

        // Open channel dropdown and select "Tests"
        let selector_rect = panel
            .click_targets
            .read()
            .iter()
            .find(|(_, t)| *t == OutputClickTarget::ChannelDropdownToggle)
            .unwrap()
            .0;
        let sel_center = Point::new(
            selector_rect.min_x() + selector_rect.width() / 2.0,
            selector_rect.min_y() + selector_rect.height() / 2.0,
        );
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(sel_center, MouseButton::Left)),
            bounds,
        );
        assert!(panel.dropdown_open);

        let mut dd_scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut dd_scene);
        let opt_rect = panel
            .click_targets
            .read()
            .iter()
            .find(|(_, t)| *t == OutputClickTarget::ChannelOption("out_2".to_string()))
            .unwrap()
            .0;
        let opt_center = Point::new(
            opt_rect.min_x() + opt_rect.width() / 2.0,
            opt_rect.min_y() + opt_rect.height() / 2.0,
        );
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(opt_center, MouseButton::Left)),
            bounds,
        );
        assert_eq!(panel.active_channel_id.as_deref(), Some("out_2"));
        assert!(!panel.dropdown_open);

        let a11y = panel.accessibility_node(bounds, &font);
        assert_eq!(a11y.id, "workbench.panel.output");
        assert!(a11y.find_by_id("output.channelSelector").is_some());
        assert!(a11y.find_by_id("output.clear").is_some());
        assert!(a11y.find_by_id("output.autoScroll").is_some());
        assert!(a11y.find_by_id("output.lines").is_some());

        // Non-finite bounds & balanced clip check
        let mut bad_scene = Scene::new();
        panel.render(
            Rect::new(Point::new(f32::NAN, 0.0), Size::new(500.0, 200.0)),
            &theme,
            &font,
            &mut bad_scene,
        );
        assert!(bad_scene.nodes().is_empty());

        let mut clip_depth = 0isize;
        for node in dd_scene.nodes() {
            match node {
                SceneNode::PushClip(_) => clip_depth += 1,
                SceneNode::PopClip => clip_depth -= 1,
                _ => {}
            }
            assert!(clip_depth >= 0);
        }
        assert_eq!(clip_depth, 0);
    }

    #[test]
    fn test_output_panel_consecutive_mousedown_without_render_split_crlf_and_narrow_toolbar() {
        let mut panel = OutputPanelView::new();
        panel.ensure_channel("ch1", "Channel 1");
        panel.ensure_channel("ch2", "Channel 2");

        // Split \r + \n across consecutive append calls must NOT emit a spurious blank line
        panel.append("ch1", "first\r");
        panel.append("ch1", "\nsecond\r\n");
        assert_eq!(panel.display_lines(), vec!["first", "second"]);

        let bounds = Rect::from_xywh(0.0, 0.0, 600.0, 200.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        // Render once with dropdown closed
        let mut scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);
        assert!(!panel.dropdown_open);

        // Case A: Click selector to open dropdown, then immediately click ch2 option WITHOUT intermediate render()
        let layout_closed = panel.compute_layout(bounds, &font);
        let sel_center = Point::new(
            layout_closed.selector_rect.min_x() + layout_closed.selector_rect.width() / 2.0,
            layout_closed.selector_rect.min_y() + layout_closed.selector_rect.height() / 2.0,
        );
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(sel_center, MouseButton::Left)),
            bounds,
        );
        assert!(panel.dropdown_open);

        let layout_open = panel.compute_layout(bounds, &font);
        let ch2_rect = layout_open
            .option_rects
            .iter()
            .find(|(id, _)| id == "ch2")
            .unwrap()
            .1;
        let ch2_center = Point::new(
            ch2_rect.min_x() + ch2_rect.width() / 2.0,
            ch2_rect.min_y() + ch2_rect.height() / 2.0,
        );
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(ch2_center, MouseButton::Left)),
            bounds,
        );
        assert_eq!(panel.active_channel_id.as_deref(), Some("ch2"));
        assert!(!panel.dropdown_open);

        // Case B: Render with dropdown open, click outside to close, then click where ch2 option used to be WITHOUT intermediate render()
        panel.dropdown_open = true;
        let mut open_scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut open_scene);
        let outside_point = Point::new(500.0, 150.0);
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(outside_point, MouseButton::Left)),
            bounds,
        );
        assert!(!panel.dropdown_open);

        // Select ch1 programmatically, then click old ch2 coordinate -> must NOT ghost-select ch2
        assert!(panel.select_channel("ch1"));
        panel.last_action = None;
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(ch2_center, MouseButton::Left)),
            bounds,
        );
        assert_eq!(panel.active_channel_id.as_deref(), Some("ch1"));
        assert!(panel.last_action.is_none());

        // Narrow-width toolbar buttons must not overlap or exceed bounds.max_x()
        for narrow_w in [240.0, 180.0, 100.0] {
            let narrow_bounds = Rect::from_xywh(10.0, 20.0, narrow_w, 160.0);
            let layout = panel.compute_layout(narrow_bounds, &font);
            assert!(layout.selector_rect.max_x() <= layout.clear_rect.min_x());
            assert!(layout.clear_rect.max_x() <= layout.auto_scroll_rect.min_x());
            assert!(layout.auto_scroll_rect.max_x() <= narrow_bounds.max_x() + 0.01);
        }

        // Non-finite / overflow accessibility_node returns safe zero-bounds Panel node
        let bad_a11y = panel.accessibility_node(
            Rect::new(Point::new(f32::MAX, 0.0), Size::new(100.0, 200.0)),
            &font,
        );
        assert_eq!(bad_a11y.bounds, Rect::zero());
        assert!(bad_a11y.children.is_empty());
    }
}
