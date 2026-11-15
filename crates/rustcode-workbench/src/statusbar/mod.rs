//! Status bar item management, sorting, and GPUI StatusBarView component.

use rustcode_platform::render::{
    EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatusBarAlignment {
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusBarItem {
    pub id: String,
    pub text: String,
    pub tooltip: Option<String>,
    pub command: Option<String>,
    pub alignment: StatusBarAlignment,
    pub priority: i32,
}

#[derive(Default, Clone)]
pub struct StatusBar {
    items: Vec<StatusBarItem>,
}

impl StatusBar {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn set_item(&mut self, item: StatusBarItem) {
        if let Some(existing) = self.items.iter_mut().find(|i| i.id == item.id) {
            *existing = item;
        } else {
            self.items.push(item);
        }
    }

    pub fn remove_item(&mut self, id: &str) {
        self.items.retain(|i| i.id != id);
    }

    /// Returns left-aligned items sorted by priority (higher priority first).
    pub fn left_items(&self) -> Vec<&StatusBarItem> {
        let mut left: Vec<_> = self
            .items
            .iter()
            .filter(|i| i.alignment == StatusBarAlignment::Left)
            .collect();
        left.sort_by_key(|a| std::cmp::Reverse(a.priority));
        left
    }

    /// Returns right-aligned items sorted by priority (higher priority first).
    pub fn right_items(&self) -> Vec<&StatusBarItem> {
        let mut right: Vec<_> = self
            .items
            .iter()
            .filter(|i| i.alignment == StatusBarAlignment::Right)
            .collect();
        right.sort_by_key(|a| std::cmp::Reverse(a.priority));
        right
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusBarAction {
    ExecuteCommand(String),
}

pub struct StatusBarView {
    pub model: StatusBar,
    pub last_action: Option<StatusBarAction>,
}

impl Default for StatusBarView {
    fn default() -> Self {
        Self::new(StatusBar::new())
    }
}

impl StatusBarView {
    pub fn new(model: StatusBar) -> Self {
        Self {
            model,
            last_action: None,
        }
    }

    fn item_rects(&self, bounds: Rect, font: &FontMetrics) -> Vec<(StatusBarItem, Rect)> {
        let mut result = Vec::new();

        // Left items
        let mut cur_x = bounds.min_x() + 8.0;
        for item in self.model.left_items() {
            let w = item.text.len() as f32 * font.char_width;
            let rect = Rect::from_xywh(cur_x, bounds.min_y(), w, bounds.height());
            result.push((item.clone(), rect));
            cur_x += w + 14.0;
        }

        // Right items
        let mut right_x = bounds.max_x() - 8.0;
        for item in self.model.right_items() {
            let w = item.text.len() as f32 * font.char_width;
            right_x -= w;
            let rect = Rect::from_xywh(right_x, bounds.min_y(), w, bounds.height());
            result.push((item.clone(), rect));
            right_x -= 14.0;
        }

        result
    }
}

impl View for StatusBarView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        // Status bar background quad
        scene.push_quad(Quad::new(bounds).with_background(theme.statusbar_bg));

        let rects = self.item_rects(bounds, font);
        for (item, rect) in rects {
            let text_y = rect.min_y() + (bounds.height() - font.line_height) / 2.0;
            scene.push_text(GlyphRun::new(
                item.text,
                Point::new(rect.min_x(), text_y),
                rect,
                theme.statusbar_fg,
                font.font_size * 0.9,
            ));
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if let InputEvent::MouseDown(mouse) = event {
            let font = FontMetrics::default_monospace();
            let rects = self.item_rects(bounds, &font);

            for (item, rect) in rects {
                if rect.contains(mouse.position) {
                    if let Some(cmd) = item.command {
                        self.last_action = Some(StatusBarAction::ExecuteCommand(cmd));
                        return EventResult::Consumed;
                    }
                }
            }
            EventResult::Ignored
        } else {
            EventResult::Ignored
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::render::MouseEvent;

    #[test]
    fn test_status_bar_priority_ordering() {
        let mut bar = StatusBar::new();

        bar.set_item(StatusBarItem {
            id: "git".to_string(),
            text: "main*".to_string(),
            tooltip: None,
            command: None,
            alignment: StatusBarAlignment::Left,
            priority: 100,
        });

        bar.set_item(StatusBarItem {
            id: "errors".to_string(),
            text: "0 errors".to_string(),
            tooltip: None,
            command: None,
            alignment: StatusBarAlignment::Left,
            priority: 200,
        });

        let left = bar.left_items();
        assert_eq!(left[0].id, "errors"); // Higher priority comes first
        assert_eq!(left[1].id, "git");
    }

    #[test]
    fn test_status_bar_view_render_and_click() {
        let mut bar = StatusBar::new();
        bar.set_item(StatusBarItem {
            id: "git".to_string(),
            text: "main*".to_string(),
            tooltip: None,
            command: Some("git.checkout".to_string()),
            alignment: StatusBarAlignment::Left,
            priority: 100,
        });
        bar.set_item(StatusBarItem {
            id: "encoding".to_string(),
            text: "UTF-8".to_string(),
            tooltip: None,
            command: None,
            alignment: StatusBarAlignment::Right,
            priority: 50,
        });

        let mut view = StatusBarView::new(bar);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let bounds = Rect::from_xywh(0.0, 578.0, 800.0, 22.0);
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("main*"));
        assert!(scene.contains_text("UTF-8"));

        // Click git item
        let click_ev = InputEvent::MouseDown(MouseEvent::new(
            Point::new(12.0, 585.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = view.handle_event(&click_ev, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(
            view.last_action,
            Some(StatusBarAction::ExecuteCommand("git.checkout".to_string()))
        );
    }
}
