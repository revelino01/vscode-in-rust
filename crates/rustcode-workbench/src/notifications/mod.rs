//! Notification toasts, modal message dialogs, and interactive QuickPick / InputBox prompt UI.

use parking_lot::RwLock;
use rustcode_ext_host::window::{
    ExtNotificationState, ExtPromptKind, ExtPromptState, NotificationSeverity,
};
use rustcode_platform::accessibility::{AccessibleNode, AccessibleRole};
use rustcode_platform::render::{
    EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Rgba, Scene,
    Shadow, View, WorkbenchThemeStyles,
};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Weak};

fn clamp_rect_to_bounds(rect: Rect, bounds: Rect) -> Rect {
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

fn truncate_utf8_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let end_byte = text
        .char_indices()
        .nth(keep)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    format!("{}...", &text[..end_byte])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationUiAction {
    SelectAction { id: String, action_title: String },
    Dismiss { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NotificationHitTarget {
    Action { id: String, title: String },
    Dismiss { id: String },
    Backdrop,
}

#[derive(Debug, Clone)]
struct NotificationCardLayout {
    id: String,
    severity: NotificationSeverity,
    message: String,
    detail: Option<String>,
    modal: bool,
    card_rect: Rect,
    message_rect: Rect,
    detail_rect: Option<Rect>,
    dismiss_rect: Rect,
    action_buttons: Vec<(String, Rect)>,
}

/// Renders non-blocking notification toast stack and modal notification dialogs.
pub struct NotificationsOverlay {
    notifications: RwLock<Vec<ExtNotificationState>>,
    pub last_action: Option<NotificationUiAction>,
    last_font: RwLock<FontMetrics>,
}

impl Default for NotificationsOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationsOverlay {
    pub fn new() -> Self {
        Self {
            notifications: RwLock::new(Vec::new()),
            last_action: None,
            last_font: RwLock::new(FontMetrics::default_monospace()),
        }
    }

    /// Synchronize active notifications from `ExtHostWindowManager`, deduplicating by ID
    /// and removing any notifications resolved or dismissed externally.
    pub fn sync_notifications(&self, active: &[ExtNotificationState]) {
        let mut seen = HashSet::with_capacity(active.len());
        let mut deduped = Vec::with_capacity(active.len());
        for notif in active {
            if !notif.dismissed && !notif.resolved && seen.insert(notif.id.clone()) {
                deduped.push(notif.clone());
            }
        }
        *self.notifications.write() = deduped;
    }

    pub fn notifications(&self) -> Vec<ExtNotificationState> {
        self.notifications.read().clone()
    }

    pub fn is_empty(&self) -> bool {
        self.notifications.read().is_empty()
    }

    pub fn has_modal(&self) -> bool {
        self.notifications.read().iter().any(|n| n.modal)
    }

    fn remove_local(&self, id: &str) {
        self.notifications.write().retain(|n| n.id != id);
    }

    fn compute_layouts(&self, bounds: Rect, font: &FontMetrics) -> Vec<NotificationCardLayout> {
        if !crate::is_finite_positive_rect(bounds) || !crate::is_finite_font(font) {
            return Vec::new();
        }
        let list = self.notifications.read();
        if list.is_empty() {
            return Vec::new();
        }

        let mut layouts = Vec::with_capacity(list.len());
        let pad = 10.0f32.min(bounds.width() * 0.1).max(2.0);
        let btn_h = 22.0f32.min(bounds.height().max(0.0));
        let line_h = font.line_height.max(12.0);

        // Non-modal toast stack from bottom-right upward
        let margin_r = 10.0f32.min((bounds.width() * 0.05).max(2.0));
        let margin_b = if bounds.height() > 80.0 { 28.0 } else { 4.0 };
        let toast_w = (360.0f32)
            .min((bounds.width() - margin_r * 2.0).max(0.0))
            .max(bounds.width().min(120.0));
        let toast_x = (bounds.max_x() - toast_w - margin_r).max(bounds.min_x());
        let mut cursor_bottom = (bounds.max_y() - margin_b).max(bounds.min_y());

        for notif in list.iter().filter(|n| !n.modal).rev() {
            let has_buttons = !notif.items.is_empty();
            let raw_h = pad + line_h + if has_buttons { 8.0 + btn_h } else { 0.0 } + pad;
            let toast_h = raw_h.min((cursor_bottom - bounds.min_y()).max(0.0));
            if toast_h <= 2.0 {
                break;
            }
            let toast_y = (cursor_bottom - toast_h).max(bounds.min_y());
            let card_rect = clamp_rect_to_bounds(
                Rect::from_xywh(toast_x, toast_y, toast_w, toast_h),
                bounds,
            );
            if card_rect.width() <= 0.0 || card_rect.height() <= 0.0 {
                continue;
            }

            let dismiss_size = 20.0f32.min(card_rect.width()).min(card_rect.height());
            let dismiss_rect = clamp_rect_to_bounds(
                Rect::from_xywh(
                    (card_rect.max_x() - dismiss_size - 4.0).max(card_rect.min_x()),
                    card_rect.min_y() + 4.0f32.min(card_rect.height() * 0.25),
                    dismiss_size,
                    dismiss_size,
                ),
                card_rect,
            );

            let msg_x = (card_rect.min_x() + pad).min(dismiss_rect.min_x());
            let msg_w = (dismiss_rect.min_x() - msg_x - 4.0).max(0.0);
            let msg_rect = clamp_rect_to_bounds(
                Rect::from_xywh(
                    msg_x,
                    card_rect.min_y() + pad.min(card_rect.height() * 0.25),
                    msg_w,
                    line_h.min(card_rect.height()),
                ),
                card_rect,
            );

            let mut action_buttons = Vec::new();
            if has_buttons {
                let btn_y = (card_rect.max_y() - pad.min(6.0) - btn_h).max(msg_rect.max_y() + 2.0);
                let actual_btn_h = btn_h.min((card_rect.max_y() - btn_y).max(0.0));
                if actual_btn_h > 0.0 {
                    let mut cur_bx = card_rect.min_x() + pad.min(card_rect.width() * 0.1);
                    for item in &notif.items {
                        let bw = (font.text_width(&item.title) + 14.0)
                            .max(36.0)
                            .min((card_rect.max_x() - cur_bx - 4.0).max(0.0));
                        if bw <= 4.0 {
                            break;
                        }
                        let br = clamp_rect_to_bounds(
                            Rect::from_xywh(cur_bx, btn_y, bw, actual_btn_h),
                            card_rect,
                        );
                        action_buttons.push((item.title.clone(), br));
                        cur_bx = br.max_x() + 6.0;
                    }
                }
            }

            layouts.push(NotificationCardLayout {
                id: notif.id.clone(),
                severity: notif.severity,
                message: truncate_utf8_chars(&notif.message, 200),
                detail: notif.detail.as_ref().map(|d| truncate_utf8_chars(d, 200)),
                modal: false,
                card_rect,
                message_rect: msg_rect,
                detail_rect: None,
                dismiss_rect,
                action_buttons,
            });

            cursor_bottom = (card_rect.min_y() - 8.0).max(bounds.min_y());
        }

        // Modal dialog (latest modal notification rendered centered on top)
        if let Some(modal) = list.iter().rev().find(|n| n.modal) {
            let modal_w = (440.0f32).min((bounds.width() - 24.0).max(bounds.width().min(160.0)));
            let has_detail = modal.detail.as_deref().is_some_and(|d| !d.is_empty());
            let raw_modal_h =
                pad + line_h + if has_detail { line_h + 8.0 } else { 0.0 } + 14.0 + btn_h + pad;
            let modal_h = raw_modal_h.min((bounds.height() - 16.0).max(bounds.height().min(90.0)));
            let modal_x = bounds.min_x() + ((bounds.width() - modal_w) * 0.5).max(0.0);
            let modal_y = bounds.min_y() + ((bounds.height() - modal_h) * 0.5).max(0.0);
            let card_rect = clamp_rect_to_bounds(
                Rect::from_xywh(modal_x, modal_y, modal_w, modal_h),
                bounds,
            );

            let dismiss_size = 20.0f32.min(card_rect.width()).min(card_rect.height());
            let dismiss_rect = clamp_rect_to_bounds(
                Rect::from_xywh(
                    (card_rect.max_x() - dismiss_size - 6.0).max(card_rect.min_x()),
                    card_rect.min_y() + 6.0f32.min(card_rect.height() * 0.2),
                    dismiss_size,
                    dismiss_size,
                ),
                card_rect,
            );

            let msg_x = (card_rect.min_x() + pad).min(dismiss_rect.min_x());
            let msg_w = (dismiss_rect.min_x() - msg_x - 6.0).max(0.0);
            let msg_rect = clamp_rect_to_bounds(
                Rect::from_xywh(
                    msg_x,
                    card_rect.min_y() + pad.min(card_rect.height() * 0.2),
                    msg_w,
                    line_h.min(card_rect.height()),
                ),
                card_rect,
            );

            let detail_rect = if has_detail {
                let dy = (msg_rect.max_y() + 6.0).min(card_rect.max_y());
                Some(clamp_rect_to_bounds(
                    Rect::from_xywh(
                        msg_x,
                        dy,
                        (card_rect.max_x() - msg_x - pad).max(0.0),
                        line_h.min((card_rect.max_y() - dy).max(0.0)),
                    ),
                    card_rect,
                ))
            } else {
                None
            };

            let mut action_buttons = Vec::new();
            let btn_y = (card_rect.max_y() - pad.min(8.0) - btn_h)
                .max(detail_rect.map_or(msg_rect.max_y(), |r| r.max_y()) + 4.0);
            let actual_btn_h = btn_h.min((card_rect.max_y() - btn_y).max(0.0));
            if actual_btn_h > 0.0 {
                let mut cur_bx = card_rect.min_x() + pad;
                for item in &modal.items {
                    let bw = (font.text_width(&item.title) + 18.0)
                        .max(44.0)
                        .min((card_rect.max_x() - cur_bx - 4.0).max(0.0));
                    if bw <= 4.0 {
                        break;
                    }
                    let br = clamp_rect_to_bounds(
                        Rect::from_xywh(cur_bx, btn_y, bw, actual_btn_h),
                        card_rect,
                    );
                    action_buttons.push((item.title.clone(), br));
                    cur_bx = br.max_x() + 8.0;
                }
            }

            layouts.push(NotificationCardLayout {
                id: modal.id.clone(),
                severity: modal.severity,
                message: truncate_utf8_chars(&modal.message, 200),
                detail: modal.detail.as_ref().map(|d| truncate_utf8_chars(d, 200)),
                modal: true,
                card_rect,
                message_rect: msg_rect,
                detail_rect,
                dismiss_rect,
                action_buttons,
            });
        }

        layouts
    }

    fn hit_test_target(&self, pos: Point, bounds: Rect) -> Option<NotificationHitTarget> {
        let font = self.last_font.read().clone();
        let layouts = self.compute_layouts(bounds, &font);
        let has_modal = layouts.iter().any(|l| l.modal);

        for card in layouts.iter().rev() {
            if has_modal && !card.modal {
                continue;
            }
            for (title, rect) in &card.action_buttons {
                if rect.contains(pos) {
                    return Some(NotificationHitTarget::Action {
                        id: card.id.clone(),
                        title: title.clone(),
                    });
                }
            }
            if card.dismiss_rect.contains(pos) {
                return Some(NotificationHitTarget::Dismiss {
                    id: card.id.clone(),
                });
            }
        }
        if has_modal {
            Some(NotificationHitTarget::Backdrop)
        } else {
            None
        }
    }

    pub fn accessibility_nodes(&self, bounds: Rect, font: &FontMetrics) -> Vec<AccessibleNode> {
        let layouts = self.compute_layouts(bounds, font);
        let mut nodes = Vec::new();
        for card in layouts {
            let sev_label = match card.severity {
                NotificationSeverity::Info => "Information",
                NotificationSeverity::Warning => "Warning",
                NotificationSeverity::Error => "Error",
            };
            let (node_id, role) = if card.modal {
                (
                    format!("notifications.modal.{}", card.id),
                    AccessibleRole::Dialog,
                )
            } else {
                (
                    format!("notifications.toast.{}", card.id),
                    AccessibleRole::Panel,
                )
            };

            let mut container =
                AccessibleNode::new(node_id.clone(), role, card.message.clone(), card.card_rect)
                    .with_description(sev_label);

            if let Some(detail) = &card.detail {
                if let Some(d_rect) = card.detail_rect {
                    container.add_child(AccessibleNode::new(
                        format!("{}.detail", node_id),
                        AccessibleRole::StaticText,
                        detail.clone(),
                        d_rect,
                    ));
                }
            }

            for (idx, (title, rect)) in card.action_buttons.iter().enumerate() {
                container.add_child(
                    AccessibleNode::new(
                        format!("{}.action.{}", node_id, idx),
                        AccessibleRole::Button,
                        title.clone(),
                        *rect,
                    )
                    .with_action("press"),
                );
            }

            container.add_child(
                AccessibleNode::new(
                    format!("{}.dismiss", node_id),
                    AccessibleRole::Button,
                    "Close Notification",
                    card.dismiss_rect,
                )
                .with_action("press"),
            );

            nodes.push(container);
        }
        nodes
    }
}

impl View for NotificationsOverlay {
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
        let layouts = self.compute_layouts(bounds, font);
        if layouts.is_empty() {
            return;
        }

        if layouts.iter().any(|l| l.modal) {
            crate::push_finite_quad(
                scene,
                Quad::new(bounds).with_background(Rgba::new(0.0, 0.0, 0.0, 0.45)),
            );
        }

        for card in layouts {
            let accent = match card.severity {
                NotificationSeverity::Info => theme.editor_info_foreground,
                NotificationSeverity::Warning => theme.editor_warning_foreground,
                NotificationSeverity::Error => theme.editor_error_foreground,
            };

            scene.push_shadow(Shadow {
                bounds: card.card_rect,
                offset: Point::new(0.0, 4.0),
                blur: 12.0,
                spread: 0.0,
                color: Rgba::new(0.0, 0.0, 0.0, 0.35),
            });

            crate::push_finite_quad(
                scene,
                Quad::new(card.card_rect)
                    .with_background(theme.command_palette_bg)
                    .with_border(theme.command_palette_border, 1.0)
                    .with_corner_radius(4.0),
            );

            // Severity left accent stripe
            let stripe_rect = clamp_rect_to_bounds(
                Rect::from_xywh(
                    card.card_rect.min_x(),
                    card.card_rect.min_y(),
                    3.0f32.min(card.card_rect.width()),
                    card.card_rect.height(),
                ),
                card.card_rect,
            );
            crate::push_finite_quad(scene, Quad::new(stripe_rect).with_background(accent));

            // Message
            if card.message_rect.width() > 0.0 && card.message_rect.height() > 0.0 {
                crate::push_finite_text(
                    scene,
                    GlyphRun::new(
                        card.message.clone(),
                        card.message_rect.origin,
                        card.message_rect,
                        theme.command_palette_fg,
                        font.font_size,
                    ),
                );
            }

            // Modal detail
            if let (Some(detail), Some(d_rect)) = (&card.detail, card.detail_rect) {
                if d_rect.width() > 0.0 && d_rect.height() > 0.0 {
                    crate::push_finite_text(
                        scene,
                        GlyphRun::new(
                            detail.clone(),
                            d_rect.origin,
                            d_rect,
                            theme.tab_inactive_fg,
                            font.font_size * 0.9,
                        ),
                    );
                }
            }

            // Dismiss [✕] button
            if card.dismiss_rect.width() > 0.0 && card.dismiss_rect.height() > 0.0 {
                let tx = (card.dismiss_rect.min_x() + 4.0).min(card.dismiss_rect.max_x());
                let ty = (card.dismiss_rect.min_y() + 2.0).min(card.dismiss_rect.max_y());
                crate::push_finite_text(
                    scene,
                    GlyphRun::new(
                        "✕".to_string(),
                        Point::new(tx, ty),
                        card.dismiss_rect,
                        theme.tab_inactive_fg,
                        font.font_size * 0.9,
                    ),
                );
            }

            // Action buttons
            for (title, btn_rect) in &card.action_buttons {
                if btn_rect.width() <= 0.0 || btn_rect.height() <= 0.0 {
                    continue;
                }
                crate::push_finite_quad(
                    scene,
                    Quad::new(*btn_rect)
                        .with_background(theme.command_palette_selected_bg)
                        .with_corner_radius(3.0),
                );
                let tx = (btn_rect.min_x() + 7.0).min(btn_rect.max_x());
                let ty = btn_rect.min_y() + ((btn_rect.height() - font.line_height) * 0.5).max(0.0);
                let text_rect = clamp_rect_to_bounds(
                    Rect::from_xywh(
                        tx,
                        ty,
                        (btn_rect.max_x() - tx).max(0.0),
                        font.line_height.min(btn_rect.height()),
                    ),
                    *btn_rect,
                );
                crate::push_finite_text(
                    scene,
                    GlyphRun::new(
                        title.clone(),
                        Point::new(tx, ty),
                        text_rect,
                        theme.activity_bar_fg,
                        font.font_size * 0.9,
                    ),
                );
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::MouseDown(mouse) => {
                let Some(target) = self.hit_test_target(mouse.position, bounds) else {
                    return EventResult::Ignored;
                };
                match target {
                    NotificationHitTarget::Action { id, title } => {
                        self.remove_local(&id);
                        self.last_action = Some(NotificationUiAction::SelectAction {
                            id,
                            action_title: title,
                        });
                        EventResult::Consumed
                    }
                    NotificationHitTarget::Dismiss { id } => {
                        self.remove_local(&id);
                        self.last_action = Some(NotificationUiAction::Dismiss { id });
                        EventResult::Consumed
                    }
                    NotificationHitTarget::Backdrop => EventResult::Consumed,
                }
            }
            InputEvent::KeyDown(key) => {
                if self.has_modal() {
                    if key.key == KeyCode::Escape {
                        let modal_id = self
                            .notifications
                            .read()
                            .iter()
                            .rev()
                            .find(|n| n.modal)
                            .map(|n| n.id.clone());
                        if let Some(id) = modal_id {
                            self.remove_local(&id);
                            self.last_action = Some(NotificationUiAction::Dismiss { id });
                        }
                    }
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            _ => {
                if self.has_modal() {
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PromptUiAction {
    SubmitQuickPick {
        id: String,
        value: Value,
    },
    SelectQuickPickItem {
        id: String,
        client_id: Option<String>,
        item: Value,
    },
    SubmitInputBox {
        id: String,
        value: String,
    },
    ValidateInput {
        id: String,
        client_id: Option<String>,
        value: String,
    },
    Cancel {
        id: String,
    },
}

#[derive(Debug, Clone)]
pub enum ActivePromptKind {
    QuickPick {
        items: Vec<Value>,
        filtered_indices: Vec<usize>,
        selected_index: usize,
        picked_indices: HashSet<usize>,
        can_pick_many: bool,
        filter_query: String,
        place_holder: Option<String>,
        title: Option<String>,
        has_on_did_select_item: bool,
    },
    InputBox {
        prompt: Option<String>,
        place_holder: Option<String>,
        input_text: String,
        password: bool,
        title: Option<String>,
        has_validate_input: bool,
        validation_message: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ActivePromptUi {
    pub id: String,
    pub client_id: Option<String>,
    pub kind: ActivePromptKind,
}

impl ActivePromptUi {
    fn from_ext(ext: &ExtPromptState) -> Self {
        let kind = match &ext.kind {
            ExtPromptKind::QuickPick {
                items,
                can_pick_many,
                place_holder,
                title,
                has_on_did_select_item,
                ..
            } => {
                let filtered_indices = (0..items.len()).collect();
                ActivePromptKind::QuickPick {
                    items: items.clone(),
                    filtered_indices,
                    selected_index: 0,
                    picked_indices: HashSet::new(),
                    can_pick_many: *can_pick_many,
                    filter_query: String::new(),
                    place_holder: place_holder.clone(),
                    title: title.clone(),
                    has_on_did_select_item: *has_on_did_select_item,
                }
            }
            ExtPromptKind::InputBox {
                prompt,
                place_holder,
                value,
                password,
                title,
                has_validate_input,
                ..
            } => ActivePromptKind::InputBox {
                prompt: prompt.clone(),
                place_holder: place_holder.clone(),
                input_text: value.clone().unwrap_or_default(),
                password: *password,
                title: title.clone(),
                has_validate_input: *has_validate_input,
                validation_message: None,
            },
        };
        Self {
            id: ext.id.clone(),
            client_id: ext.client_id.clone(),
            kind,
        }
    }

    pub fn set_validation_message(&mut self, msg: Option<String>) {
        if let ActivePromptKind::InputBox {
            validation_message, ..
        } = &mut self.kind
        {
            *validation_message = msg;
        }
    }
}

fn quick_pick_item_label(item: &Value) -> String {
    item.get("label")
        .and_then(|v| v.as_str())
        .or_else(|| item.as_str())
        .unwrap_or("")
        .to_string()
}

fn quick_pick_item_description(item: &Value) -> Option<String> {
    item.get("description")
        .or_else(|| item.get("detail"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn refilter_quick_pick(
    items: &[Value],
    query: &str,
    filtered_indices: &mut Vec<usize>,
    selected_index: &mut usize,
) {
    let q = query.trim().to_ascii_lowercase();
    filtered_indices.clear();
    for (idx, item) in items.iter().enumerate() {
        if q.is_empty() {
            filtered_indices.push(idx);
            continue;
        }
        let label = quick_pick_item_label(item).to_ascii_lowercase();
        let desc = quick_pick_item_description(item)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if label.contains(&q) || desc.contains(&q) {
            filtered_indices.push(idx);
        }
    }
    if filtered_indices.is_empty() || *selected_index >= filtered_indices.len() {
        *selected_index = 0;
    }
}

/// Interactive overlay for `vscode.window.showQuickPick` and `vscode.window.showInputBox`.
pub struct PromptOverlay {
    active: Arc<RwLock<Option<ActivePromptUi>>>,
    pub last_action: Option<PromptUiAction>,
    last_font: RwLock<FontMetrics>,
}

impl Default for PromptOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptOverlay {
    pub fn new() -> Self {
        Self {
            active: Arc::new(RwLock::new(None)),
            last_action: None,
            last_font: RwLock::new(FontMetrics::default_monospace()),
        }
    }

    pub fn weak_state(&self) -> Weak<RwLock<Option<ActivePromptUi>>> {
        Arc::downgrade(&self.active)
    }

    pub fn is_open(&self) -> bool {
        self.active.read().is_some()
    }

    pub fn active_prompt(&self) -> Option<ActivePromptUi> {
        self.active.read().clone()
    }

    pub fn set_validation_message_for(&self, prompt_id: &str, msg: Option<String>) {
        let mut guard = self.active.write();
        if let Some(active) = guard.as_mut() {
            if active.id == prompt_id {
                active.set_validation_message(msg);
            }
        }
    }

    /// Synchronize with active prompts from `ExtHostWindowManager`.
    /// Preserves local interactive typing/selection state when the current prompt ID remains active.
    pub fn sync_prompts(&self, active_prompts: &[ExtPromptState]) {
        let next = active_prompts.iter().find(|p| !p.resolved);
        let mut guard = self.active.write();
        match next {
            None => {
                *guard = None;
            }
            Some(ext) => {
                let same_id = guard.as_ref().is_some_and(|cur| cur.id == ext.id);
                if !same_id {
                    *guard = Some(ActivePromptUi::from_ext(ext));
                }
            }
        }
    }

    pub fn compute_dialog_bounds(bounds: Rect) -> Rect {
        if !crate::is_finite_positive_rect(bounds) {
            return Rect::zero();
        }
        let margin_x = 40.0f32.min(bounds.width() * 0.15);
        let w = (560.0f32).min((bounds.width() - margin_x).max(bounds.width().min(180.0)));
        let h = (300.0f32).min((bounds.height() - 48.0).max(bounds.height().min(120.0)));
        let x = bounds.min_x() + ((bounds.width() - w) * 0.5).max(0.0);
        let y = bounds.min_y() + (36.0f32).min((bounds.height() - h).max(0.0));
        clamp_rect_to_bounds(Rect::from_xywh(x, y, w, h), bounds)
    }

    pub fn accessibility_node(&self, bounds: Rect, font: &FontMetrics) -> Option<AccessibleNode> {
        let guard = self.active.read();
        let prompt = guard.as_ref()?;
        let dialog_rect = Self::compute_dialog_bounds(bounds);

        match &prompt.kind {
            ActivePromptKind::QuickPick {
                items,
                filtered_indices,
                selected_index,
                filter_query,
                place_holder,
                title,
                ..
            } => {
                let header_h = if title.as_deref().is_some_and(|s| !s.is_empty()) {
                    font.line_height + 6.0
                } else {
                    0.0
                };
                let input_y = dialog_rect.min_y() + 8.0 + header_h;
                let input_h = 30.0f32.min((dialog_rect.max_y() - input_y).max(0.0));
                let input_rect = clamp_rect_to_bounds(
                    Rect::from_xywh(
                        dialog_rect.min_x() + 10.0,
                        input_y,
                        (dialog_rect.width() - 20.0).max(0.0),
                        input_h,
                    ),
                    dialog_rect,
                );
                let dialog_name = title
                    .as_deref()
                    .or(place_holder.as_deref())
                    .unwrap_or("Quick Pick");
                let mut dialog = AccessibleNode::new(
                    "workbench.prompt.quickPick",
                    AccessibleRole::Dialog,
                    dialog_name,
                    dialog_rect,
                );
                dialog.add_child(
                    AccessibleNode::new(
                        "prompt.quickPick.input",
                        AccessibleRole::TextInput,
                        place_holder.as_deref().unwrap_or("Filter items"),
                        input_rect,
                    )
                    .with_value(filter_query.clone())
                    .with_focused(true)
                    .with_action("input"),
                );

                let list_y = (input_rect.max_y() + 6.0).min(dialog_rect.max_y());
                let list_rect = clamp_rect_to_bounds(
                    Rect::from_xywh(
                        dialog_rect.min_x(),
                        list_y,
                        dialog_rect.width(),
                        (dialog_rect.max_y() - list_y).max(0.0),
                    ),
                    dialog_rect,
                );
                let mut list_node = AccessibleNode::new(
                    "prompt.quickPick.list",
                    AccessibleRole::List,
                    "Quick Pick Items",
                    list_rect,
                );
                let row_h = (font.line_height + 10.0).max(24.0);
                for (vis_idx, &item_idx) in filtered_indices.iter().enumerate() {
                    let row_y = list_y + vis_idx as f32 * row_h;
                    if row_y >= dialog_rect.max_y() {
                        break;
                    }
                    let row_rect = clamp_rect_to_bounds(
                        Rect::from_xywh(
                            dialog_rect.min_x() + 4.0,
                            row_y,
                            (dialog_rect.width() - 8.0).max(0.0),
                            row_h,
                        ),
                        dialog_rect,
                    );
                    if let Some(item) = items.get(item_idx) {
                        let label = quick_pick_item_label(item);
                        let mut item_node = AccessibleNode::new(
                            format!("prompt.quickPick.item.{}", vis_idx),
                            AccessibleRole::ListItem,
                            label,
                            row_rect,
                        )
                        .with_selected(vis_idx == *selected_index)
                        .with_action("select");
                        if let Some(desc) = quick_pick_item_description(item) {
                            item_node = item_node.with_description(desc);
                        }
                        list_node.add_child(item_node);
                    }
                }
                dialog.add_child(list_node);
                Some(dialog)
            }
            ActivePromptKind::InputBox {
                prompt: prompt_text,
                place_holder,
                input_text,
                password,
                title,
                validation_message,
                ..
            } => {
                let header_h = if title.as_deref().is_some_and(|s| !s.is_empty()) {
                    font.line_height + 6.0
                } else {
                    0.0
                };
                let input_y = dialog_rect.min_y() + 10.0 + header_h;
                let input_h = 30.0f32.min((dialog_rect.max_y() - input_y).max(0.0));
                let input_rect = clamp_rect_to_bounds(
                    Rect::from_xywh(
                        dialog_rect.min_x() + 12.0,
                        input_y,
                        (dialog_rect.width() - 24.0).max(0.0),
                        input_h,
                    ),
                    dialog_rect,
                );
                let dialog_name = title
                    .as_deref()
                    .or(prompt_text.as_deref())
                    .or(place_holder.as_deref())
                    .unwrap_or("Input Box");
                let mut dialog = AccessibleNode::new(
                    "workbench.prompt.inputBox",
                    AccessibleRole::Dialog,
                    dialog_name,
                    dialog_rect,
                );
                let val = if *password {
                    "*".repeat(input_text.chars().count())
                } else {
                    input_text.clone()
                };
                dialog.add_child(
                    AccessibleNode::new(
                        "prompt.inputBox.input",
                        AccessibleRole::TextInput,
                        prompt_text.as_deref().unwrap_or("Input"),
                        input_rect,
                    )
                    .with_value(val)
                    .with_focused(true)
                    .with_action("input"),
                );
                if let Some(p_txt) = prompt_text {
                    let p_rect = clamp_rect_to_bounds(
                        Rect::from_xywh(
                            dialog_rect.min_x() + 10.0,
                            (input_rect.max_y() + 6.0).min(dialog_rect.max_y()),
                            (dialog_rect.width() - 20.0).max(0.0),
                            font.line_height,
                        ),
                        dialog_rect,
                    );
                    dialog.add_child(AccessibleNode::new(
                        "prompt.inputBox.prompt",
                        AccessibleRole::StaticText,
                        p_txt.clone(),
                        p_rect,
                    ));
                }
                if let Some(err) = validation_message {
                    let e_rect = clamp_rect_to_bounds(
                        Rect::from_xywh(
                            dialog_rect.min_x() + 10.0,
                            (input_rect.max_y() + font.line_height + 10.0).min(dialog_rect.max_y()),
                            (dialog_rect.width() - 20.0).max(0.0),
                            font.line_height,
                        ),
                        dialog_rect,
                    );
                    dialog.add_child(AccessibleNode::new(
                        "prompt.inputBox.validation",
                        AccessibleRole::StaticText,
                        err.clone(),
                        e_rect,
                    ));
                }
                Some(dialog)
            }
        }
    }
}

impl View for PromptOverlay {
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
        let guard = self.active.read();
        let Some(prompt) = guard.as_ref() else {
            return;
        };

        let dialog_rect = Self::compute_dialog_bounds(bounds);
        if dialog_rect.width() <= 0.0 || dialog_rect.height() <= 0.0 {
            return;
        }

        // Dim backdrop
        crate::push_finite_quad(
            scene,
            Quad::new(bounds).with_background(Rgba::new(0.0, 0.0, 0.0, 0.35)),
        );

        scene.push_shadow(Shadow {
            bounds: dialog_rect,
            offset: Point::new(0.0, 6.0),
            blur: 16.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.4),
        });

        crate::push_finite_quad(
            scene,
            Quad::new(dialog_rect)
                .with_background(theme.command_palette_bg)
                .with_border(theme.command_palette_border, 1.0)
                .with_corner_radius(6.0),
        );

        match &prompt.kind {
            ActivePromptKind::QuickPick {
                items,
                filtered_indices,
                selected_index,
                picked_indices,
                can_pick_many,
                filter_query,
                place_holder,
                title,
                ..
            } => {
                let mut cur_y = dialog_rect.min_y() + 8.0;
                if let Some(t) = title.as_deref().filter(|s| !s.is_empty()) {
                    let t_rect = clamp_rect_to_bounds(
                        Rect::from_xywh(
                            dialog_rect.min_x() + 12.0,
                            cur_y,
                            (dialog_rect.width() - 24.0).max(0.0),
                            font.line_height,
                        ),
                        dialog_rect,
                    );
                    if t_rect.width() > 0.0 && t_rect.height() > 0.0 {
                        crate::push_finite_text(
                            scene,
                            GlyphRun::new(
                                t.to_string(),
                                t_rect.origin,
                                t_rect,
                                theme.sidebar_header_fg,
                                font.font_size * 0.9,
                            ),
                        );
                    }
                    cur_y = (t_rect.max_y() + 6.0).min(dialog_rect.max_y());
                }

                let input_h = 30.0f32.min((dialog_rect.max_y() - cur_y).max(0.0));
                let input_rect = clamp_rect_to_bounds(
                    Rect::from_xywh(
                        dialog_rect.min_x() + 10.0,
                        cur_y,
                        (dialog_rect.width() - 20.0).max(0.0),
                        input_h,
                    ),
                    dialog_rect,
                );
                crate::push_finite_quad(
                    scene,
                    Quad::new(input_rect)
                        .with_background(theme.editor_background)
                        .with_border(theme.activity_bar_active_border, 1.0)
                        .with_corner_radius(3.0),
                );

                let (display_str, display_color) = if !filter_query.is_empty() {
                    (filter_query.clone(), theme.command_palette_fg)
                } else if let Some(ph) = place_holder {
                    (ph.clone(), theme.tab_inactive_fg)
                } else {
                    (String::new(), theme.command_palette_fg)
                };

                if !display_str.is_empty() {
                    let tx = (input_rect.min_x() + 8.0).min(input_rect.max_x());
                    let ty =
                        input_rect.min_y() + ((input_rect.height() - font.line_height) * 0.5).max(0.0);
                    let tr = clamp_rect_to_bounds(
                        Rect::from_xywh(
                            tx,
                            ty,
                            (input_rect.max_x() - tx - 6.0).max(0.0),
                            font.line_height.min(input_rect.height()),
                        ),
                        input_rect,
                    );
                    crate::push_finite_text(
                        scene,
                        GlyphRun::new(display_str, Point::new(tx, ty), tr, display_color, font.font_size),
                    );
                }

                let list_y = (input_rect.max_y() + 6.0).min(dialog_rect.max_y());
                let row_h = (font.line_height + 10.0).max(24.0);
                for (vis_idx, &item_idx) in filtered_indices.iter().enumerate() {
                    let row_y = list_y + vis_idx as f32 * row_h;
                    if row_y >= dialog_rect.max_y() {
                        break;
                    }
                    let row_rect = clamp_rect_to_bounds(
                        Rect::from_xywh(
                            dialog_rect.min_x() + 6.0,
                            row_y,
                            (dialog_rect.width() - 12.0).max(0.0),
                            row_h.min((dialog_rect.max_y() - row_y).max(0.0)),
                        ),
                        dialog_rect,
                    );
                    if row_rect.height() <= 0.0 || row_rect.width() <= 0.0 {
                        break;
                    }
                    let is_sel = vis_idx == *selected_index;
                    if is_sel {
                        crate::push_finite_quad(
                            scene,
                            Quad::new(row_rect)
                                .with_background(theme.command_palette_selected_bg)
                                .with_corner_radius(3.0),
                        );
                    }
                    if let Some(item) = items.get(item_idx) {
                        let label = quick_pick_item_label(item);
                        let text = if *can_pick_many {
                            let mark = if picked_indices.contains(&item_idx) {
                                "[x] "
                            } else {
                                "[ ] "
                            };
                            format!("{}{}", mark, label)
                        } else {
                            label
                        };
                        let tx = (row_rect.min_x() + 8.0).min(row_rect.max_x());
                        let ty = row_rect.min_y()
                            + ((row_rect.height() - font.line_height) * 0.5).max(0.0);
                        let tr = clamp_rect_to_bounds(
                            Rect::from_xywh(
                                tx,
                                ty,
                                (row_rect.max_x() - tx - 6.0).max(0.0),
                                font.line_height.min(row_rect.height()),
                            ),
                            row_rect,
                        );
                        crate::push_finite_text(
                            scene,
                            GlyphRun::new(
                                text,
                                Point::new(tx, ty),
                                tr,
                                theme.command_palette_fg,
                                font.font_size,
                            ),
                        );
                    }
                }
            }
            ActivePromptKind::InputBox {
                prompt: prompt_text,
                place_holder,
                input_text,
                password,
                title,
                validation_message,
                ..
            } => {
                let mut cur_y = dialog_rect.min_y() + 10.0;
                if let Some(t) = title.as_deref().filter(|s| !s.is_empty()) {
                    let t_rect = clamp_rect_to_bounds(
                        Rect::from_xywh(
                            dialog_rect.min_x() + 12.0,
                            cur_y,
                            (dialog_rect.width() - 24.0).max(0.0),
                            font.line_height,
                        ),
                        dialog_rect,
                    );
                    crate::push_finite_text(
                        scene,
                        GlyphRun::new(
                            t.to_string(),
                            t_rect.origin,
                            t_rect,
                            theme.sidebar_header_fg,
                            font.font_size * 0.9,
                        ),
                    );
                    cur_y = (t_rect.max_y() + 6.0).min(dialog_rect.max_y());
                }

                let input_h = 30.0f32.min((dialog_rect.max_y() - cur_y).max(0.0));
                let input_rect = clamp_rect_to_bounds(
                    Rect::from_xywh(
                        dialog_rect.min_x() + 12.0,
                        cur_y,
                        (dialog_rect.width() - 24.0).max(0.0),
                        input_h,
                    ),
                    dialog_rect,
                );
                let border_color = if validation_message.is_some() {
                    theme.editor_error_foreground
                } else {
                    theme.activity_bar_active_border
                };
                crate::push_finite_quad(
                    scene,
                    Quad::new(input_rect)
                        .with_background(theme.editor_background)
                        .with_border(border_color, 1.0)
                        .with_corner_radius(3.0),
                );

                let (shown, color) = if !input_text.is_empty() {
                    let s = if *password {
                        "*".repeat(input_text.chars().count())
                    } else {
                        input_text.clone()
                    };
                    (s, theme.command_palette_fg)
                } else if let Some(ph) = place_holder {
                    (ph.clone(), theme.tab_inactive_fg)
                } else {
                    (String::new(), theme.command_palette_fg)
                };

                if !shown.is_empty() && input_rect.width() > 8.0 && input_rect.height() > 0.0 {
                    let tx = (input_rect.min_x() + 8.0).min(input_rect.max_x());
                    let ty = input_rect.min_y()
                        + ((input_rect.height() - font.line_height) * 0.5).max(0.0);
                    let tr = clamp_rect_to_bounds(
                        Rect::from_xywh(
                            tx,
                            ty,
                            (input_rect.max_x() - tx - 6.0).max(0.0),
                            font.line_height.min(input_rect.height()),
                        ),
                        input_rect,
                    );
                    crate::push_finite_text(
                        scene,
                        GlyphRun::new(shown, Point::new(tx, ty), tr, color, font.font_size),
                    );
                }

                cur_y = (input_rect.max_y() + 8.0).min(dialog_rect.max_y());
                if let Some(p_txt) = prompt_text.as_deref().filter(|s| !s.is_empty()) {
                    let pr = clamp_rect_to_bounds(
                        Rect::from_xywh(
                            dialog_rect.min_x() + 12.0,
                            cur_y,
                            (dialog_rect.width() - 24.0).max(0.0),
                            font.line_height.min((dialog_rect.max_y() - cur_y).max(0.0)),
                        ),
                        dialog_rect,
                    );
                    if pr.width() > 0.0 && pr.height() > 0.0 {
                        crate::push_finite_text(
                            scene,
                            GlyphRun::new(
                                p_txt.to_string(),
                                pr.origin,
                                pr,
                                theme.tab_inactive_fg,
                                font.font_size * 0.9,
                            ),
                        );
                    }
                    cur_y = (pr.max_y() + 6.0).min(dialog_rect.max_y());
                }

                if let Some(err) = validation_message.as_deref().filter(|s| !s.is_empty()) {
                    let er = clamp_rect_to_bounds(
                        Rect::from_xywh(
                            dialog_rect.min_x() + 12.0,
                            cur_y,
                            (dialog_rect.width() - 24.0).max(0.0),
                            font.line_height.min((dialog_rect.max_y() - cur_y).max(0.0)),
                        ),
                        dialog_rect,
                    );
                    if er.width() > 0.0 && er.height() > 0.0 {
                        crate::push_finite_text(
                            scene,
                            GlyphRun::new(
                                err.to_string(),
                                er.origin,
                                er,
                                theme.editor_error_foreground,
                                font.font_size * 0.9,
                            ),
                        );
                    }
                }
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if !self.is_open() {
            return EventResult::Ignored;
        }
        let dialog_rect = Self::compute_dialog_bounds(bounds);
        let font = self.last_font.read().clone();

        match event {
            InputEvent::KeyDown(key) => {
                let mut guard = self.active.write();
                let Some(prompt) = guard.as_mut() else {
                    return EventResult::Ignored;
                };
                if key.key == KeyCode::Escape {
                    let id = prompt.id.clone();
                    *guard = None;
                    self.last_action = Some(PromptUiAction::Cancel { id });
                    return EventResult::Consumed;
                }

                match &mut prompt.kind {
                    ActivePromptKind::QuickPick {
                        items,
                        filtered_indices,
                        selected_index,
                        picked_indices,
                        can_pick_many,
                        filter_query,
                        has_on_did_select_item,
                        ..
                    } => match &key.key {
                        KeyCode::ArrowDown => {
                            if !filtered_indices.is_empty() {
                                *selected_index = (*selected_index + 1) % filtered_indices.len();
                                if *has_on_did_select_item {
                                    if let Some(item) =
                                        filtered_indices.get(*selected_index).and_then(|&i| items.get(i))
                                    {
                                        self.last_action = Some(PromptUiAction::SelectQuickPickItem {
                                            id: prompt.id.clone(),
                                            client_id: prompt.client_id.clone(),
                                            item: item.clone(),
                                        });
                                    }
                                }
                            }
                            EventResult::Consumed
                        }
                        KeyCode::ArrowUp => {
                            if !filtered_indices.is_empty() {
                                if *selected_index == 0 {
                                    *selected_index = filtered_indices.len() - 1;
                                } else {
                                    *selected_index -= 1;
                                }
                                if *has_on_did_select_item {
                                    if let Some(item) =
                                        filtered_indices.get(*selected_index).and_then(|&i| items.get(i))
                                    {
                                        self.last_action = Some(PromptUiAction::SelectQuickPickItem {
                                            id: prompt.id.clone(),
                                            client_id: prompt.client_id.clone(),
                                            item: item.clone(),
                                        });
                                    }
                                }
                            }
                            EventResult::Consumed
                        }
                        KeyCode::Space
                            if *can_pick_many
                                && !key.modifiers.ctrl
                                && !key.modifiers.alt
                                && !key.modifiers.meta =>
                        {
                            if let Some(&item_idx) = filtered_indices.get(*selected_index) {
                                if !picked_indices.insert(item_idx) {
                                    picked_indices.remove(&item_idx);
                                }
                            }
                            EventResult::Consumed
                        }
                        KeyCode::Enter => {
                            let response_opt = if *can_pick_many {
                                let selected: Vec<Value> = items
                                    .iter()
                                    .enumerate()
                                    .filter(|(idx, _)| picked_indices.contains(idx))
                                    .map(|(_, item)| item.clone())
                                    .collect();
                                if selected.is_empty() {
                                    filtered_indices
                                        .get(*selected_index)
                                        .and_then(|&idx| items.get(idx))
                                        .map(|item| Value::Array(vec![item.clone()]))
                                } else {
                                    Some(Value::Array(selected))
                                }
                            } else {
                                filtered_indices
                                    .get(*selected_index)
                                    .and_then(|&idx| items.get(idx))
                                    .cloned()
                            };
                            if let Some(value) = response_opt {
                                let id = prompt.id.clone();
                                *guard = None;
                                self.last_action = Some(PromptUiAction::SubmitQuickPick { id, value });
                            }
                            EventResult::Consumed
                        }
                        KeyCode::Backspace => {
                            if !filter_query.is_empty() {
                                filter_query.pop();
                                refilter_quick_pick(
                                    items,
                                    filter_query,
                                    filtered_indices,
                                    selected_index,
                                );
                            }
                            EventResult::Consumed
                        }
                        KeyCode::Space if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                            filter_query.push(' ');
                            refilter_quick_pick(
                                items,
                                filter_query,
                                filtered_indices,
                                selected_index,
                            );
                            EventResult::Consumed
                        }
                        KeyCode::Char(c) if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                            match &key.text {
                                Some(txt) if !txt.is_empty() => filter_query.push_str(txt),
                                _ => {
                                    let ch = if key.modifiers.shift {
                                        c.to_ascii_uppercase()
                                    } else {
                                        *c
                                    };
                                    filter_query.push(ch);
                                }
                            }
                            refilter_quick_pick(
                                items,
                                filter_query,
                                filtered_indices,
                                selected_index,
                            );
                            EventResult::Consumed
                        }
                        _ => EventResult::Consumed,
                    },
                    ActivePromptKind::InputBox {
                        input_text,
                        has_validate_input,
                        validation_message,
                        ..
                    } => match &key.key {
                        KeyCode::Enter => {
                            if validation_message.as_deref().is_some_and(|m| !m.is_empty()) {
                                return EventResult::Consumed;
                            }
                            let id = prompt.id.clone();
                            let value = input_text.clone();
                            *guard = None;
                            self.last_action = Some(PromptUiAction::SubmitInputBox { id, value });
                            EventResult::Consumed
                        }
                        KeyCode::Backspace => {
                            input_text.pop();
                            if *has_validate_input {
                                self.last_action = Some(PromptUiAction::ValidateInput {
                                    id: prompt.id.clone(),
                                    client_id: prompt.client_id.clone(),
                                    value: input_text.clone(),
                                });
                            }
                            EventResult::Consumed
                        }
                        KeyCode::Space if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                            input_text.push(' ');
                            if *has_validate_input {
                                self.last_action = Some(PromptUiAction::ValidateInput {
                                    id: prompt.id.clone(),
                                    client_id: prompt.client_id.clone(),
                                    value: input_text.clone(),
                                });
                            }
                            EventResult::Consumed
                        }
                        KeyCode::Char(c) if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                            match &key.text {
                                Some(txt) if !txt.is_empty() => input_text.push_str(txt),
                                _ => {
                                    let ch = if key.modifiers.shift {
                                        c.to_ascii_uppercase()
                                    } else {
                                        *c
                                    };
                                    input_text.push(ch);
                                }
                            }
                            if *has_validate_input {
                                self.last_action = Some(PromptUiAction::ValidateInput {
                                    id: prompt.id.clone(),
                                    client_id: prompt.client_id.clone(),
                                    value: input_text.clone(),
                                });
                            }
                            EventResult::Consumed
                        }
                        _ => EventResult::Consumed,
                    },
                }
            }
            InputEvent::MouseDown(mouse) => {
                let mut guard = self.active.write();
                let Some(prompt) = guard.as_mut() else {
                    return EventResult::Ignored;
                };
                if !dialog_rect.contains(mouse.position) {
                    let id = prompt.id.clone();
                    *guard = None;
                    self.last_action = Some(PromptUiAction::Cancel { id });
                    return EventResult::Consumed;
                }
                if let ActivePromptKind::QuickPick {
                    items,
                    filtered_indices,
                    selected_index,
                    can_pick_many,
                    picked_indices,
                    title,
                    ..
                } = &mut prompt.kind
                {
                    let header_h = if title.as_deref().is_some_and(|s| !s.is_empty()) {
                        font.line_height + 6.0
                    } else {
                        0.0
                    };
                    let list_y = dialog_rect.min_y() + 8.0 + header_h + 30.0 + 6.0;
                    let row_h = (font.line_height + 10.0).max(24.0);
                    if mouse.position.y >= list_y {
                        let vis_idx = ((mouse.position.y - list_y) / row_h).floor().max(0.0) as usize;
                        if let Some(&item_idx) = filtered_indices.get(vis_idx) {
                            *selected_index = vis_idx;
                            if *can_pick_many {
                                if !picked_indices.insert(item_idx) {
                                    picked_indices.remove(&item_idx);
                                }
                            } else if let Some(item) = items.get(item_idx).cloned() {
                                let id = prompt.id.clone();
                                *guard = None;
                                self.last_action = Some(PromptUiAction::SubmitQuickPick {
                                    id,
                                    value: item,
                                });
                            }
                        }
                    }
                }
                EventResult::Consumed
            }
            _ => EventResult::Consumed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_ext_host::window::NotificationActionItem;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{KeyEvent, MouseButton, MouseEvent};
    use serde_json::json;

    fn make_notif(id: &str, message: &str, modal: bool, items: &[&str]) -> ExtNotificationState {
        ExtNotificationState {
            id: id.to_string(),
            severity: NotificationSeverity::Warning,
            message: message.to_string(),
            detail: if modal {
                Some("Detailed context".to_string())
            } else {
                None
            },
            modal,
            items: items
                .iter()
                .map(|t| NotificationActionItem {
                    title: (*t).to_string(),
                    is_close_affordance: false,
                    raw: json!(*t),
                })
                .collect(),
            selected_action: None,
            dismissed: false,
            resolved: false,
        }
    }

    #[test]
    fn test_notifications_dedup_tiny_viewport_and_modal_dialog() {
        let mut overlay = NotificationsOverlay::new();
        let n1 = make_notif("n1", "UniqueToastOnce", false, &["Reload"]);
        overlay.sync_notifications(&[n1.clone(), n1]);
        assert_eq!(overlay.notifications().len(), 1);

        let tiny = Rect::from_xywh(0.0, 0.0, 240.0, 180.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let mut scene = Scene::new();
        overlay.render(tiny, &theme, &font, &mut scene);

        assert_eq!(
            scene
                .glyph_runs()
                .iter()
                .filter(|g| g.text.contains("UniqueToastOnce"))
                .count(),
            1
        );
        for q in scene.quads() {
            assert!(
                q.bounds.width().is_finite()
                    && q.bounds.height().is_finite()
                    && q.bounds.width() >= 0.0
                    && q.bounds.height() >= 0.0
            );
        }

        // Modal notification renders AccessibleRole::Dialog with detail and handles click
        let modal = make_notif("m1", "Overwrite?", true, &["Overwrite"]);
        overlay.sync_notifications(&[modal]);
        let nodes = overlay.accessibility_nodes(tiny, &font);
        assert!(nodes.iter().any(|n| n.role == AccessibleRole::Dialog));

        let mut modal_scene = Scene::new();
        overlay.render(tiny, &theme, &font, &mut modal_scene);
        assert!(modal_scene.contains_text("Overwrite?"));
        assert!(modal_scene.contains_text("Detailed context"));

        let btn_bounds = nodes[0]
            .children
            .iter()
            .find(|c| c.name == "Overwrite")
            .unwrap()
            .bounds;
        let click = InputEvent::MouseDown(MouseEvent::new(
            Point::new(
                btn_bounds.min_x() + btn_bounds.width() * 0.5,
                btn_bounds.min_y() + btn_bounds.height() * 0.5,
            ),
            MouseButton::Left,
        ));
        assert_eq!(overlay.handle_event(&click, tiny), EventResult::Consumed);
        assert_eq!(
            overlay.last_action,
            Some(NotificationUiAction::SelectAction {
                id: "m1".to_string(),
                action_title: "Overwrite".to_string(),
            })
        );
        assert!(overlay.is_empty());
    }

    #[test]
    fn test_prompt_overlay_quickpick_and_inputbox_navigation() {
        let mut overlay = PromptOverlay::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        let qp = ExtPromptState {
            id: "p1".to_string(),
            client_id: Some("c1".to_string()),
            kind: ExtPromptKind::QuickPick {
                items: vec![json!({ "label": "Debug" }), json!({ "label": "Release" })],
                can_pick_many: false,
                place_holder: Some("Select profile".to_string()),
                title: None,
                has_on_did_select_item: false,
                options: json!({}),
            },
            response: None,
            resolved: false,
        };
        overlay.sync_prompts(&[qp]);
        assert!(overlay.is_open());

        let mut scene = Scene::new();
        overlay.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("Debug"));
        assert!(scene.contains_text("Release"));

        overlay.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::ArrowDown, Modifiers::none())),
            bounds,
        );
        overlay.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            overlay.last_action,
            Some(PromptUiAction::SubmitQuickPick {
                id: "p1".to_string(),
                value: json!({ "label": "Release" }),
            })
        );
        assert!(!overlay.is_open());

        // InputBox typing + Escape cancel
        let ib = ExtPromptState {
            id: "p2".to_string(),
            client_id: None,
            kind: ExtPromptKind::InputBox {
                prompt: Some("Module name".to_string()),
                place_holder: Some("mod".to_string()),
                value: None,
                password: false,
                title: None,
                has_validate_input: false,
                options: json!({}),
            },
            response: None,
            resolved: false,
        };
        overlay.sync_prompts(&[ib]);
        overlay.handle_event(
            &InputEvent::KeyDown(
                KeyEvent::new(KeyCode::Char('a'), Modifiers::none()).with_text("a".to_string()),
            ),
            bounds,
        );
        overlay.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            overlay.last_action,
            Some(PromptUiAction::Cancel {
                id: "p2".to_string(),
            })
        );
        assert!(!overlay.is_open());

        // Tiny viewport (240x180 and 120x80) and stale click safety
        for (w, h) in [(240.0, 180.0), (120.0, 80.0)] {
            let tiny = Rect::from_xywh(0.0, 0.0, w, h);
            let qp_tiny = ExtPromptState {
                id: "p_tiny".to_string(),
                client_id: None,
                kind: ExtPromptKind::QuickPick {
                    items: vec![json!({ "label": "One" }), json!({ "label": "Two" })],
                    can_pick_many: false,
                    place_holder: Some("Filter".to_string()),
                    title: Some("Title".to_string()),
                    has_on_did_select_item: false,
                    options: json!({}),
                },
                response: None,
                resolved: false,
            };
            overlay.sync_prompts(&[qp_tiny]);
            let mut tiny_scene = Scene::new();
            overlay.render(tiny, &theme, &font, &mut tiny_scene);
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
            overlay.sync_prompts(&[]);
        }

        // Stale click after prompt dismissal is a safe no-op
        overlay.last_action = None;
        let stale_click = InputEvent::MouseDown(MouseEvent::new(
            Point::new(50.0, 50.0),
            MouseButton::Left,
        ));
        assert_eq!(overlay.handle_event(&stale_click, bounds), EventResult::Ignored);
        assert!(overlay.last_action.is_none());
    }
}
