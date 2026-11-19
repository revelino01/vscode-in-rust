//! Extension Host Window UI Management (`vscode.window.show*Message`, `showQuickPick`, `showInputBox`).

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationActionItem {
    pub title: String,
    pub is_close_affordance: bool,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtNotificationState {
    pub id: String,
    pub severity: NotificationSeverity,
    pub message: String,
    pub detail: Option<String>,
    pub modal: bool,
    pub items: Vec<NotificationActionItem>,
    pub selected_action: Option<Value>,
    pub dismissed: bool,
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "promptKind", rename_all = "camelCase")]
pub enum ExtPromptKind {
    QuickPick {
        items: Vec<Value>,
        #[serde(rename = "canPickMany")]
        can_pick_many: bool,
        #[serde(rename = "placeHolder")]
        place_holder: Option<String>,
        title: Option<String>,
        options: Value,
    },
    InputBox {
        prompt: Option<String>,
        #[serde(rename = "placeHolder")]
        place_holder: Option<String>,
        value: Option<String>,
        password: bool,
        title: Option<String>,
        options: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtPromptState {
    pub id: String,
    #[serde(flatten)]
    pub kind: ExtPromptKind,
    pub response: Option<Value>,
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtWindowEvent {
    NotificationShown {
        id: String,
        severity: NotificationSeverity,
        message: String,
        detail: Option<String>,
        modal: bool,
        items: Vec<NotificationActionItem>,
    },
    NotificationResolved {
        id: String,
        selected: Option<Value>,
        dismissed: bool,
    },
    PromptRequested {
        id: String,
        prompt: ExtPromptKind,
    },
    PromptResolved {
        id: String,
        response: Option<Value>,
    },
}

pub type WindowEventListener = Arc<dyn Fn(&ExtWindowEvent) + Send + Sync>;
pub type NotificationResponder = Arc<dyn Fn(&ExtNotificationState) -> Option<Value> + Send + Sync>;
pub type PromptResponder = Arc<dyn Fn(&ExtPromptState) -> Option<Value> + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostWindowManager {
    notifications: Arc<RwLock<HashMap<String, ExtNotificationState>>>,
    notification_order: Arc<RwLock<Vec<String>>>,
    prompts: Arc<RwLock<HashMap<String, ExtPromptState>>>,
    prompt_order: Arc<RwLock<Vec<String>>>,
    pending_events: Arc<RwLock<Vec<ExtWindowEvent>>>,
    listeners: Arc<RwLock<Vec<WindowEventListener>>>,
    revision: Arc<AtomicU64>,
    next_id: Arc<AtomicU64>,
    notification_responder: Arc<RwLock<Option<NotificationResponder>>>,
    prompt_responder: Arc<RwLock<Option<PromptResponder>>>,
    await_responses: Arc<AtomicBool>,
    response_timeout: Arc<RwLock<Option<Duration>>>,
    pending_notification_waiters: Arc<Mutex<HashMap<String, oneshot::Sender<Option<Value>>>>>,
    pending_prompt_waiters: Arc<Mutex<HashMap<String, oneshot::Sender<Option<Value>>>>>,
}

impl ExtHostWindowManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<ExtWindowEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_window_event<F>(&self, listener: F)
    where
        F: Fn(&ExtWindowEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    pub fn set_notification_responder<F>(&self, responder: F)
    where
        F: Fn(&ExtNotificationState) -> Option<Value> + Send + Sync + 'static,
    {
        *self.notification_responder.write() = Some(Arc::new(responder));
    }

    pub fn clear_notification_responder(&self) {
        *self.notification_responder.write() = None;
    }

    pub fn set_prompt_responder<F>(&self, responder: F)
    where
        F: Fn(&ExtPromptState) -> Option<Value> + Send + Sync + 'static,
    {
        *self.prompt_responder.write() = Some(Arc::new(responder));
    }

    pub fn clear_prompt_responder(&self) {
        *self.prompt_responder.write() = None;
    }

    pub fn set_await_responses(&self, enabled: bool) {
        self.await_responses.store(enabled, Ordering::SeqCst);
    }

    pub fn set_response_timeout(&self, timeout: Option<Duration>) {
        *self.response_timeout.write() = timeout;
    }

    fn emit(&self, event: ExtWindowEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.listeners.read().clone();
        for listener in listeners {
            listener(&event);
        }
    }

    fn next_notif_id(&self) -> String {
        let n = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        format!("notif_{}", n)
    }

    fn next_prompt_id(&self) -> String {
        let n = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        format!("prompt_{}", n)
    }

    pub fn list_notifications(&self) -> Vec<ExtNotificationState> {
        let order = self.notification_order.read();
        let map = self.notifications.read();
        order.iter().filter_map(|id| map.get(id).cloned()).collect()
    }

    pub fn active_notifications(&self) -> Vec<ExtNotificationState> {
        self.list_notifications()
            .into_iter()
            .filter(|n| !n.dismissed && !n.resolved)
            .collect()
    }

    pub fn get_notification(&self, id: &str) -> Option<ExtNotificationState> {
        self.notifications.read().get(id).cloned()
    }

    pub fn list_prompts(&self) -> Vec<ExtPromptState> {
        let order = self.prompt_order.read();
        let map = self.prompts.read();
        order.iter().filter_map(|id| map.get(id).cloned()).collect()
    }

    pub fn get_prompt(&self, id: &str) -> Option<ExtPromptState> {
        self.prompts.read().get(id).cloned()
    }

    pub fn respond_notification(&self, id: &str, selection: Option<Value>) -> bool {
        let (normalized, dismissed) = {
            let mut map = self.notifications.write();
            let Some(state) = map.get_mut(id) else {
                return false;
            };
            if state.resolved {
                return false;
            }

            let (sel, is_dismissed) = match selection {
                None | Some(Value::Null) => (None, true),
                Some(Value::String(s)) => {
                    let matched = state
                        .items
                        .iter()
                        .find(|item| item.title == s)
                        .map(|item| item.raw.clone())
                        .unwrap_or(Value::String(s));
                    (Some(matched), false)
                }
                Some(other) => (Some(other), false),
            };

            state.selected_action = sel.clone();
            state.dismissed = is_dismissed;
            state.resolved = true;
            (sel, is_dismissed)
        };

        self.emit(ExtWindowEvent::NotificationResolved {
            id: id.to_string(),
            selected: normalized.clone(),
            dismissed,
        });

        if let Some(tx) = self.pending_notification_waiters.lock().remove(id) {
            let _ = tx.send(normalized);
        }
        true
    }

    pub fn dismiss_notification(&self, id: &str) -> bool {
        self.respond_notification(id, None)
    }

    pub fn respond_prompt(&self, id: &str, value: Option<Value>) -> bool {
        let normalized = {
            let mut map = self.prompts.write();
            let Some(state) = map.get_mut(id) else {
                return false;
            };
            if state.resolved {
                return false;
            }

            let norm = match &state.kind {
                ExtPromptKind::QuickPick {
                    items,
                    can_pick_many,
                    ..
                } => normalize_quick_pick_response(value, items, *can_pick_many),
                ExtPromptKind::InputBox { .. } => match value {
                    None | Some(Value::Null) => None,
                    Some(Value::String(s)) => Some(Value::String(s)),
                    Some(other) => Some(other),
                },
            };

            state.response = norm.clone();
            state.resolved = true;
            norm
        };

        self.emit(ExtWindowEvent::PromptResolved {
            id: id.to_string(),
            response: normalized.clone(),
        });

        if let Some(tx) = self.pending_prompt_waiters.lock().remove(id) {
            let _ = tx.send(normalized);
        }
        true
    }

    pub async fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "window.showInformationMessage" => {
                self.handle_show_message(NotificationSeverity::Info, params)
                    .await
            }
            "window.showWarningMessage" => {
                self.handle_show_message(NotificationSeverity::Warning, params)
                    .await
            }
            "window.showErrorMessage" => {
                self.handle_show_message(NotificationSeverity::Error, params)
                    .await
            }
            "window.showQuickPick" => self.handle_show_quick_pick(params).await,
            "window.showInputBox" => self.handle_show_input_box(params).await,
            _ => Err(format!("Unsupported window RPC method: {}", method)),
        }
    }

    async fn handle_show_message(
        &self,
        severity: NotificationSeverity,
        params: &Value,
    ) -> Result<Value, String> {
        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut modal = params
            .get("options")
            .and_then(|o| o.get("modal"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut detail = params
            .get("options")
            .and_then(|o| o.get("detail"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        let raw_items = params
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut action_slice: &[Value] = &raw_items;
        if let Some(first) = raw_items.first() {
            if is_message_options_object(first) {
                if let Some(m) = first.get("modal").and_then(|v| v.as_bool()) {
                    modal = m;
                }
                if detail.is_none() {
                    detail = first
                        .get("detail")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string);
                }
                action_slice = &raw_items[1..];
            }
        }

        let items: Vec<NotificationActionItem> = action_slice
            .iter()
            .filter_map(parse_notification_action_item)
            .collect();

        let id = self.next_notif_id();
        let state = ExtNotificationState {
            id: id.clone(),
            severity,
            message: message.clone(),
            detail: detail.clone(),
            modal,
            items: items.clone(),
            selected_action: None,
            dismissed: false,
            resolved: false,
        };

        let should_await = self.notification_responder.read().is_none()
            && self.await_responses.load(Ordering::SeqCst);
        let rx_opt = if should_await {
            let (tx, rx) = oneshot::channel();
            self.pending_notification_waiters
                .lock()
                .insert(id.clone(), tx);
            Some(rx)
        } else {
            None
        };

        {
            self.notifications.write().insert(id.clone(), state.clone());
            self.notification_order.write().push(id.clone());
        }

        self.emit(ExtWindowEvent::NotificationShown {
            id: id.clone(),
            severity,
            message,
            detail,
            modal,
            items,
        });

        let responder_opt = self.notification_responder.read().clone();
        if let Some(responder) = responder_opt {
            let result = responder(&state);
            self.respond_notification(&id, result);
            let final_val = self
                .notifications
                .read()
                .get(&id)
                .and_then(|s| s.selected_action.clone())
                .unwrap_or(Value::Null);
            return Ok(final_val);
        }

        if let Some(rx) = rx_opt {
            struct NotificationWaiterGuard<'a> {
                mgr: &'a ExtHostWindowManager,
                id: String,
                active: bool,
            }
            impl Drop for NotificationWaiterGuard<'_> {
                fn drop(&mut self) {
                    if self.active {
                        self.mgr.pending_notification_waiters.lock().remove(&self.id);
                        self.mgr.dismiss_notification(&self.id);
                    }
                }
            }

            let mut guard = NotificationWaiterGuard {
                mgr: self,
                id: id.clone(),
                active: true,
            };
            let timeout_opt = *self.response_timeout.read();
            let resolved = match timeout_opt {
                Some(dur) => match tokio::time::timeout(dur, rx).await {
                    Ok(Ok(val)) => {
                        guard.active = false;
                        val
                    }
                    _ => {
                        drop(guard);
                        None
                    }
                },
                None => match rx.await {
                    Ok(val) => {
                        guard.active = false;
                        val
                    }
                    Err(_) => {
                        drop(guard);
                        None
                    }
                },
            };
            return Ok(resolved.unwrap_or(Value::Null));
        }

        Ok(Value::Null)
    }

    async fn handle_show_quick_pick(&self, params: &Value) -> Result<Value, String> {
        let items: Vec<Value> = params
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|item| match item {
                        Value::String(s) => json!({ "label": s }),
                        other => other.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let options = params.get("options").cloned().unwrap_or_else(|| json!({}));
        let can_pick_many = options
            .get("canPickMany")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let place_holder = options
            .get("placeHolder")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let title = options
            .get("title")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        let kind = ExtPromptKind::QuickPick {
            items,
            can_pick_many,
            place_holder,
            title,
            options,
        };
        self.dispatch_prompt(kind).await
    }

    async fn handle_show_input_box(&self, params: &Value) -> Result<Value, String> {
        let options = params.get("options").cloned().unwrap_or_else(|| json!({}));
        let prompt = options
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let place_holder = options
            .get("placeHolder")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let value = options
            .get("value")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let password = options
            .get("password")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let title = options
            .get("title")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        let kind = ExtPromptKind::InputBox {
            prompt,
            place_holder,
            value,
            password,
            title,
            options,
        };
        self.dispatch_prompt(kind).await
    }

    async fn dispatch_prompt(&self, kind: ExtPromptKind) -> Result<Value, String> {
        let id = self.next_prompt_id();
        let state = ExtPromptState {
            id: id.clone(),
            kind: kind.clone(),
            response: None,
            resolved: false,
        };

        let should_await =
            self.prompt_responder.read().is_none() && self.await_responses.load(Ordering::SeqCst);
        let rx_opt = if should_await {
            let (tx, rx) = oneshot::channel();
            self.pending_prompt_waiters.lock().insert(id.clone(), tx);
            Some(rx)
        } else {
            None
        };

        {
            self.prompts.write().insert(id.clone(), state.clone());
            self.prompt_order.write().push(id.clone());
        }

        self.emit(ExtWindowEvent::PromptRequested {
            id: id.clone(),
            prompt: kind,
        });

        let responder_opt = self.prompt_responder.read().clone();
        if let Some(responder) = responder_opt {
            let result = responder(&state);
            self.respond_prompt(&id, result);
            let final_val = self
                .prompts
                .read()
                .get(&id)
                .and_then(|s| s.response.clone())
                .unwrap_or(Value::Null);
            return Ok(final_val);
        }

        if let Some(rx) = rx_opt {
            struct PromptWaiterGuard<'a> {
                mgr: &'a ExtHostWindowManager,
                id: String,
                active: bool,
            }
            impl Drop for PromptWaiterGuard<'_> {
                fn drop(&mut self) {
                    if self.active {
                        self.mgr.pending_prompt_waiters.lock().remove(&self.id);
                        self.mgr.respond_prompt(&self.id, None);
                    }
                }
            }

            let mut guard = PromptWaiterGuard {
                mgr: self,
                id: id.clone(),
                active: true,
            };
            let timeout_opt = *self.response_timeout.read();
            let resolved = match timeout_opt {
                Some(dur) => match tokio::time::timeout(dur, rx).await {
                    Ok(Ok(val)) => {
                        guard.active = false;
                        val
                    }
                    _ => {
                        drop(guard);
                        None
                    }
                },
                None => match rx.await {
                    Ok(val) => {
                        guard.active = false;
                        val
                    }
                    Err(_) => {
                        drop(guard);
                        None
                    }
                },
            };
            return Ok(resolved.unwrap_or(Value::Null));
        }

        Ok(Value::Null)
    }
}

fn is_message_options_object(val: &Value) -> bool {
    let Some(obj) = val.as_object() else {
        return false;
    };
    !obj.contains_key("title") && (obj.contains_key("modal") || obj.contains_key("detail"))
}

fn parse_notification_action_item(val: &Value) -> Option<NotificationActionItem> {
    match val {
        Value::String(s) => Some(NotificationActionItem {
            title: s.clone(),
            is_close_affordance: false,
            raw: Value::String(s.clone()),
        }),
        Value::Object(obj) => {
            let title = obj.get("title").and_then(|v| v.as_str())?.to_string();
            let is_close_affordance = obj
                .get("isCloseAffordance")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(NotificationActionItem {
                title,
                is_close_affordance,
                raw: val.clone(),
            })
        }
        _ => None,
    }
}

fn resolve_quick_pick_item_by_label(val: Value, items: &[Value]) -> Value {
    match val {
        Value::String(s) => items
            .iter()
            .find(|item| item.get("label").and_then(|v| v.as_str()) == Some(s.as_str()))
            .cloned()
            .unwrap_or(Value::String(s)),
        other => other,
    }
}

fn normalize_quick_pick_response(
    value: Option<Value>,
    items: &[Value],
    can_pick_many: bool,
) -> Option<Value> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::Array(arr)) => {
            if can_pick_many {
                let normalized: Vec<Value> = arr
                    .into_iter()
                    .map(|v| resolve_quick_pick_item_by_label(v, items))
                    .collect();
                Some(Value::Array(normalized))
            } else {
                arr.into_iter()
                    .next()
                    .map(|v| resolve_quick_pick_item_by_label(v, items))
            }
        }
        Some(single) => {
            let resolved = resolve_quick_pick_item_by_label(single, items);
            if can_pick_many {
                Some(Value::Array(vec![resolved]))
            } else {
                Some(resolved)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_window_notifications_headless_options_and_events() {
        let mgr = ExtHostWindowManager::new();
        assert_eq!(mgr.revision(), 0);

        let res_info = mgr
            .handle_rpc_call(
                "window.showInformationMessage",
                &json!({
                    "message": "Build succeeded",
                    "items": [
                        { "modal": true, "detail": "All 42 targets built" },
                        "Open Log",
                        { "title": "Dismiss", "isCloseAffordance": true }
                    ]
                }),
            )
            .await
            .unwrap();
        assert_eq!(res_info, Value::Null);
        assert_eq!(mgr.revision(), 1);

        let notifs = mgr.list_notifications();
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].severity, NotificationSeverity::Info);
        assert_eq!(notifs[0].message, "Build succeeded");
        assert!(notifs[0].modal);
        assert_eq!(notifs[0].detail.as_deref(), Some("All 42 targets built"));
        assert_eq!(notifs[0].items.len(), 2);
        assert_eq!(notifs[0].items[0].title, "Open Log");
        assert!(notifs[0].items[1].is_close_affordance);
        assert_eq!(mgr.active_notifications().len(), 1);

        let events = mgr.take_pending_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            ExtWindowEvent::NotificationShown {
                severity: NotificationSeverity::Info,
                ..
            }
        ));

        // Subsequent manual response resolves and marks inactive
        assert!(mgr.respond_notification(&notifs[0].id, Some(json!("Dismiss"))));
        let updated = mgr.get_notification(&notifs[0].id).unwrap();
        assert!(updated.resolved);
        assert!(!updated.dismissed);
        assert_eq!(
            updated.selected_action,
            Some(json!({ "title": "Dismiss", "isCloseAffordance": true }))
        );
        assert!(mgr.active_notifications().is_empty());

        let warn_res = mgr
            .handle_rpc_call(
                "window.showWarningMessage",
                &json!({ "message": "Watch out", "items": ["Ignore"] }),
            )
            .await
            .unwrap();
        assert_eq!(warn_res, Value::Null);

        let err_res = mgr
            .handle_rpc_call(
                "window.showErrorMessage",
                &json!({ "message": "Boom", "items": [] }),
            )
            .await
            .unwrap();
        assert_eq!(err_res, Value::Null);

        let all = mgr.list_notifications();
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].severity, NotificationSeverity::Warning);
        assert_eq!(all[2].severity, NotificationSeverity::Error);
    }

    #[tokio::test]
    async fn test_window_sync_responders_and_quickpick_inputbox() {
        let mgr = ExtHostWindowManager::new();
        mgr.set_notification_responder(|state| {
            state.items.first().map(|item| item.raw.clone())
        });

        let selected = mgr
            .handle_rpc_call(
                "window.showInformationMessage",
                &json!({ "message": "Choose", "items": ["Retry", "Cancel"] }),
            )
            .await
            .unwrap();
        assert_eq!(selected, json!("Retry"));

        mgr.set_prompt_responder(|prompt| match &prompt.kind {
            ExtPromptKind::QuickPick {
                can_pick_many,
                items,
                ..
            } => {
                if *can_pick_many {
                    Some(json!(["Alpha"]))
                } else {
                    items.last().cloned()
                }
            }
            ExtPromptKind::InputBox { .. } => Some(json!("")),
        });

        let qp_single = mgr
            .handle_rpc_call(
                "window.showQuickPick",
                &json!({
                    "items": ["Alpha", { "label": "Beta", "description": "Second" }],
                    "options": { "placeHolder": "Pick one" }
                }),
            )
            .await
            .unwrap();
        assert_eq!(qp_single, json!({ "label": "Beta", "description": "Second" }));

        let qp_many = mgr
            .handle_rpc_call(
                "window.showQuickPick",
                &json!({
                    "items": ["Alpha", "Beta"],
                    "options": { "canPickMany": true }
                }),
            )
            .await
            .unwrap();
        assert_eq!(qp_many, json!([{ "label": "Alpha" }]));

        // Empty string InputBox submission is preserved (not converted to Null)
        let input_empty = mgr
            .handle_rpc_call(
                "window.showInputBox",
                &json!({ "options": { "prompt": "Name", "password": true } }),
            )
            .await
            .unwrap();
        assert_eq!(input_empty, json!(""));

        // Cancel returns Null
        mgr.set_prompt_responder(|_| None);
        let input_cancel = mgr
            .handle_rpc_call(
                "window.showInputBox",
                &json!({ "options": { "prompt": "Cancelled" } }),
            )
            .await
            .unwrap();
        assert_eq!(input_cancel, Value::Null);
    }

    #[tokio::test]
    async fn test_window_async_oneshot_and_timeout() {
        let mgr = ExtHostWindowManager::new();
        mgr.set_await_responses(true);
        mgr.set_response_timeout(Some(Duration::from_secs(2)));

        let mgr_bg = mgr.clone();
        let waiter = tokio::spawn(async move {
            mgr_bg
                .handle_rpc_call(
                    "window.showQuickPick",
                    &json!({
                        "items": ["One", "Two"],
                        "options": { "canPickMany": true }
                    }),
                )
                .await
                .unwrap()
        });

        tokio::task::yield_now().await;
        let prompts = mgr.list_prompts();
        assert_eq!(prompts.len(), 1);
        // Single item string normalizes to array of item object when canPickMany is true
        assert!(mgr.respond_prompt(&prompts[0].id, Some(json!("Two"))));

        let res = waiter.await.unwrap();
        assert_eq!(res, json!([{ "label": "Two" }]));

        // Timeout resolves cleanly to Null and cleans up state
        mgr.set_response_timeout(Some(Duration::from_millis(20)));
        let timed_out = mgr
            .handle_rpc_call(
                "window.showInformationMessage",
                &json!({ "message": "Timeout test", "items": ["OK"] }),
            )
            .await
            .unwrap();
        assert_eq!(timed_out, Value::Null);
        assert!(mgr.active_notifications().is_empty());
    }

    #[tokio::test]
    async fn test_window_post_resolution_immutable_and_abort_cleans_waiters() {
        let mgr = ExtHostWindowManager::new();
        mgr.set_await_responses(true);
        mgr.set_response_timeout(Some(Duration::from_millis(15)));

        let timed_out = mgr
            .handle_rpc_call(
                "window.showInformationMessage",
                &json!({ "message": "Late test", "items": ["OK"] }),
            )
            .await
            .unwrap();
        assert_eq!(timed_out, Value::Null);

        let rev_before = mgr.revision();
        let _ = mgr.take_pending_events();
        let notif_id = mgr.list_notifications()[0].id.clone();

        // Late respond_notification after timeout must return false and not mutate state
        assert!(!mgr.respond_notification(&notif_id, Some(json!("OK"))));
        assert_eq!(mgr.revision(), rev_before);
        assert!(mgr.take_pending_events().is_empty());
        let final_notif = mgr.get_notification(&notif_id).unwrap();
        assert!(final_notif.resolved);
        assert!(final_notif.dismissed);
        assert_eq!(final_notif.selected_action, None);

        // Same for prompt after timeout
        let prompt_timed_out = mgr
            .handle_rpc_call(
                "window.showInputBox",
                &json!({ "options": { "prompt": "Late prompt" } }),
            )
            .await
            .unwrap();
        assert_eq!(prompt_timed_out, Value::Null);
        let rev_prompt = mgr.revision();
        let _ = mgr.take_pending_events();
        let prompt_id = mgr.list_prompts()[0].id.clone();
        assert!(!mgr.respond_prompt(&prompt_id, Some(json!("late"))));
        assert_eq!(mgr.revision(), rev_prompt);
        assert!(mgr.take_pending_events().is_empty());
        assert_eq!(mgr.get_prompt(&prompt_id).unwrap().response, None);

        // Aborting/dropping a pending future with response_timeout = None cleans up waiters
        mgr.set_response_timeout(None);
        let mgr_notif = mgr.clone();
        let notif_task = tokio::spawn(async move {
            let _ = mgr_notif
                .handle_rpc_call(
                    "window.showInformationMessage",
                    &json!({ "message": "Abort notif", "items": ["A"] }),
                )
                .await;
        });
        let mgr_prompt = mgr.clone();
        let prompt_task = tokio::spawn(async move {
            let _ = mgr_prompt
                .handle_rpc_call(
                    "window.showInputBox",
                    &json!({ "options": { "prompt": "Abort prompt" } }),
                )
                .await;
        });

        tokio::task::yield_now().await;
        assert_eq!(mgr.pending_notification_waiters.lock().len(), 1);
        assert_eq!(mgr.pending_prompt_waiters.lock().len(), 1);

        notif_task.abort();
        prompt_task.abort();
        let _ = notif_task.await;
        let _ = prompt_task.await;

        assert!(mgr.pending_notification_waiters.lock().is_empty());
        assert!(mgr.pending_prompt_waiters.lock().is_empty());
        assert!(mgr.active_notifications().is_empty());
        assert!(mgr.list_prompts().iter().all(|p| p.resolved));
    }
}
