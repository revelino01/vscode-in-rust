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
}
