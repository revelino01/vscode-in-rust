//! Input events and event routing types matching GPUI.

use crate::keybindings::{KeyCombo, Modifiers};
use crate::render::geometry::Point;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyCode {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Tab,
    Space,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl KeyCode {
    pub fn as_key_str(&self) -> String {
        match self {
            KeyCode::Char(c) => c.to_ascii_lowercase().to_string(),
            KeyCode::Enter => "enter".to_string(),
            KeyCode::Escape => "escape".to_string(),
            KeyCode::Backspace => "backspace".to_string(),
            KeyCode::Tab => "tab".to_string(),
            KeyCode::Space => "space".to_string(),
            KeyCode::Delete => "delete".to_string(),
            KeyCode::ArrowUp => "up".to_string(),
            KeyCode::ArrowDown => "down".to_string(),
            KeyCode::ArrowLeft => "left".to_string(),
            KeyCode::ArrowRight => "right".to_string(),
            KeyCode::Home => "home".to_string(),
            KeyCode::End => "end".to_string(),
            KeyCode::PageUp => "pageup".to_string(),
            KeyCode::PageDown => "pagedown".to_string(),
            KeyCode::F1 => "f1".to_string(),
            KeyCode::F2 => "f2".to_string(),
            KeyCode::F3 => "f3".to_string(),
            KeyCode::F4 => "f4".to_string(),
            KeyCode::F5 => "f5".to_string(),
            KeyCode::F6 => "f6".to_string(),
            KeyCode::F7 => "f7".to_string(),
            KeyCode::F8 => "f8".to_string(),
            KeyCode::F9 => "f9".to_string(),
            KeyCode::F10 => "f10".to_string(),
            KeyCode::F11 => "f11".to_string(),
            KeyCode::F12 => "f12".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key: KeyCode,
    pub modifiers: Modifiers,
    pub text: Option<String>,
}

impl KeyEvent {
    pub fn new(key: KeyCode, modifiers: Modifiers) -> Self {
        let text = match key {
            KeyCode::Char(c) if !modifiers.ctrl && !modifiers.alt && !modifiers.meta => {
                if modifiers.shift {
                    Some(c.to_uppercase().to_string())
                } else {
                    Some(c.to_string())
                }
            }
            KeyCode::Space if !modifiers.ctrl && !modifiers.alt && !modifiers.meta => {
                Some(" ".to_string())
            }
            _ => None,
        };
        Self {
            key,
            modifiers,
            text,
        }
    }

    pub fn with_text(mut self, text: String) -> Self {
        self.text = Some(text);
        self
    }

    /// Map into platform `KeyCombo` for evaluation against `KeybindingService`.
    pub fn to_key_combo(&self) -> KeyCombo {
        KeyCombo {
            modifiers: self.modifiers,
            key: self.key.as_key_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MouseEvent {
    pub position: Point,
    pub button: MouseButton,
    pub click_count: u32,
    pub modifiers: Modifiers,
}

impl MouseEvent {
    pub fn new(position: Point, button: MouseButton) -> Self {
        Self {
            position,
            button,
            click_count: 1,
            modifiers: Modifiers::none(),
        }
    }

    pub fn with_click_count(mut self, count: u32) -> Self {
        self.click_count = count;
        self
    }

    pub fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    pub fn is_double_click(&self) -> bool {
        self.click_count == 2
    }

    pub fn is_triple_click(&self) -> bool {
        self.click_count >= 3
    }
}

/// Stateful multi-click detector for OS mouse down events (single/double/triple click).
#[derive(Debug, Clone)]
pub struct ClickTracker {
    max_interval: std::time::Duration,
    max_distance_sq: f32,
    last_button: Option<MouseButton>,
    last_position: Option<Point>,
    last_time_ms: Option<u64>,
    count: u32,
}

impl Default for ClickTracker {
    fn default() -> Self {
        Self::new(std::time::Duration::from_millis(500), 4.0)
    }
}

impl ClickTracker {
    pub fn new(max_interval: std::time::Duration, max_distance_px: f32) -> Self {
        let d = if max_distance_px.is_finite() && max_distance_px >= 0.0 {
            max_distance_px
        } else {
            4.0
        };
        Self {
            max_interval,
            max_distance_sq: d * d,
            last_button: None,
            last_position: None,
            last_time_ms: None,
            count: 0,
        }
    }

    /// Record a mouse press at `timestamp_ms` and return the resulting `click_count` (1, 2, 3, ...).
    pub fn register_click(&mut self, button: MouseButton, position: Point, timestamp_ms: u64) -> u32 {
        let within_time = self.last_time_ms.is_some_and(|prev| {
            timestamp_ms >= prev
                && (timestamp_ms - prev) <= self.max_interval.as_millis() as u64
        });
        let within_dist = self.last_position.is_some_and(|prev| {
            let dx = position.x - prev.x;
            let dy = position.y - prev.y;
            dx.is_finite() && dy.is_finite() && (dx * dx + dy * dy) <= self.max_distance_sq
        });
        let same_button = self.last_button == Some(button);

        if same_button && within_time && within_dist {
            self.count = self.count.saturating_add(1).min(3);
            if self.count == 0 {
                self.count = 1;
            }
        } else {
            self.count = 1;
        }

        self.last_button = Some(button);
        self.last_position = Some(position);
        self.last_time_ms = Some(timestamp_ms);
        self.count
    }

    pub fn reset(&mut self) {
        self.last_button = None;
        self.last_position = None;
        self.last_time_ms = None;
        self.count = 0;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MouseMoveEvent {
    pub position: Point,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollEvent {
    pub position: Point,
    pub delta_x: f32,
    pub delta_y: f32,
    pub modifiers: Modifiers,
}

impl ScrollEvent {
    pub fn new(position: Point, delta_x: f32, delta_y: f32) -> Self {
        Self {
            position,
            delta_x,
            delta_y,
            modifiers: Modifiers::none(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    KeyDown(KeyEvent),
    KeyUp(KeyEvent),
    MouseDown(MouseEvent),
    MouseUp(MouseEvent),
    MouseMove(MouseMoveEvent),
    ScrollWheel(ScrollEvent),
    ImePreedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    ImeCommit(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventResult {
    Consumed,
    Ignored,
}

impl EventResult {
    pub fn is_consumed(&self) -> bool {
        matches!(self, EventResult::Consumed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_event_to_key_combo() {
        let key_ev = KeyEvent::new(
            KeyCode::Char('b'),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        );
        let combo = key_ev.to_key_combo();
        assert_eq!(combo.key, "b");
        assert!(combo.modifiers.ctrl);
    }

    #[test]
    fn test_click_tracker_multi_click() {
        let mut tracker = ClickTracker::default();
        let p = Point::new(100.0, 200.0);
        assert_eq!(tracker.register_click(MouseButton::Left, p, 1000), 1);
        assert_eq!(tracker.register_click(MouseButton::Left, Point::new(101.0, 201.0), 1200), 2);
        assert_eq!(tracker.register_click(MouseButton::Left, Point::new(101.0, 201.0), 1400), 3);
        // Caps at 3 (triple-click)
        assert_eq!(tracker.register_click(MouseButton::Left, Point::new(101.0, 201.0), 1500), 3);
        // Too slow resets to 1
        assert_eq!(tracker.register_click(MouseButton::Left, p, 2500), 1);
        // Different button resets to 1
        assert_eq!(tracker.register_click(MouseButton::Right, p, 2600), 1);
        // Moved too far resets to 1
        assert_eq!(tracker.register_click(MouseButton::Right, Point::new(150.0, 200.0), 2700), 1);
    }

    #[test]
    fn test_ime_events_and_mouse_click_helpers() {
        let preedit = InputEvent::ImePreedit {
            text: "nihao".to_string(),
            cursor: Some((0, 5)),
        };
        let commit = InputEvent::ImeCommit("你好".to_string());
        assert!(matches!(preedit, InputEvent::ImePreedit { cursor: Some((0, 5)), .. }));
        assert!(matches!(commit, InputEvent::ImeCommit(ref s) if s == "你好"));

        let dbl = MouseEvent::new(Point::zero(), MouseButton::Left).with_click_count(2);
        assert!(dbl.is_double_click());
        assert!(!dbl.is_triple_click());

        let tpl = MouseEvent::new(Point::zero(), MouseButton::Left).with_click_count(3);
        assert!(!tpl.is_double_click());
        assert!(tpl.is_triple_click());
    }
}
