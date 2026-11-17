//! Activity Bar icon strip, view containers, and GPUI ActivityBarView component.

use rustcode_platform::render::{
    EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityBarItem {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub view_container_id: String,
    pub badge: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ActivityBar {
    pub items: Vec<ActivityBarItem>,
    pub active_item_index: usize,
}

impl Default for ActivityBar {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivityBar {
    pub fn new() -> Self {
        Self {
            items: vec![
                ActivityBarItem {
                    id: "workbench.view.explorer".to_string(),
                    title: "Explorer".to_string(),
                    icon: "files".to_string(),
                    view_container_id: "explorer".to_string(),
                    badge: None,
                },
                ActivityBarItem {
                    id: "workbench.view.search".to_string(),
                    title: "Search".to_string(),
                    icon: "search".to_string(),
                    view_container_id: "search".to_string(),
                    badge: None,
                },
                ActivityBarItem {
                    id: "workbench.view.scm".to_string(),
                    title: "Source Control".to_string(),
                    icon: "source-control".to_string(),
                    view_container_id: "scm".to_string(),
                    badge: None,
                },
                ActivityBarItem {
                    id: "workbench.view.debug".to_string(),
                    title: "Run and Debug".to_string(),
                    icon: "debug".to_string(),
                    view_container_id: "debug".to_string(),
                    badge: None,
                },
                ActivityBarItem {
                    id: "workbench.view.extensions".to_string(),
                    title: "Extensions".to_string(),
                    icon: "extensions".to_string(),
                    view_container_id: "extensions".to_string(),
                    badge: None,
                },
            ],
            active_item_index: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityBarAction {
    SelectView(String),
    ToggleSidebar,
}

pub struct ActivityBarView {
    pub model: ActivityBar,
    pub last_action: Option<ActivityBarAction>,
}

impl Default for ActivityBarView {
    fn default() -> Self {
        Self::new(ActivityBar::new())
    }
}

impl ActivityBarView {
    pub fn new(model: ActivityBar) -> Self {
        Self {
            model,
            last_action: None,
        }
    }

    pub fn active_item(&self) -> Option<&ActivityBarItem> {
        self.model.items.get(self.model.active_item_index)
    }

    pub fn set_active_index(&mut self, index: usize) {
        if index < self.model.items.len() {
            self.model.active_item_index = index;
        }
    }
}

impl View for ActivityBarView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        // Background
        scene.push_quad(Quad::new(bounds).with_background(theme.activity_bar_bg));

        let slot_h = 48.0;
        for (idx, item) in self.model.items.iter().enumerate() {
            let slot_y = bounds.min_y() + idx as f32 * slot_h;
            if slot_y + slot_h > bounds.max_y() {
                break;
            }

            let is_active = idx == self.model.active_item_index;

            if is_active {
                // Active indicator on the left
                let bar_rect = Rect::from_xywh(bounds.min_x(), slot_y, 2.0, slot_h);
                scene.push_quad(Quad::new(bar_rect).with_background(theme.activity_bar_active_border));
            }

            // Short abbreviation / icon label
            let icon_str = match item.view_container_id.as_str() {
                "explorer" => "EXP",
                "search" => "SRC",
                "scm" => "SCM",
                "debug" => "DBG",
                "extensions" => "EXT",
                _ => &item.title[..item.title.len().min(3)],
            };

            let text_color = if is_active {
                theme.activity_bar_fg
            } else {
                theme.activity_bar_inactive_fg
            };

            let text_w = font.text_width(icon_str);
            let text_x = bounds.min_x() + (bounds.width() - text_w) / 2.0;
            let text_y = slot_y + (slot_h - font.line_height) / 2.0;

            scene.push_text(GlyphRun::new(
                icon_str.to_string(),
                Point::new(text_x, text_y),
                Rect::from_xywh(text_x, text_y, text_w, font.line_height),
                text_color,
                font.font_size,
            ));

            // Badge indicator if present
            if let Some(badge) = item.badge {
                let badge_str = badge.to_string();
                let b_w = (font.text_width(&badge_str) + 8.0).max(16.0);
                let b_h = 14.0;
                let b_x = bounds.max_x() - b_w - 4.0;
                let b_y = slot_y + 4.0;

                scene.push_quad(
                    Quad::new(Rect::from_xywh(b_x, b_y, b_w, b_h))
                        .with_background(theme.activity_bar_badge_bg)
                        .with_corner_radius(4.0),
                );

                scene.push_text(GlyphRun::new(
                    badge_str,
                    Point::new(b_x + 4.0, b_y),
                    Rect::from_xywh(b_x + 4.0, b_y, b_w - 8.0, b_h),
                    theme.activity_bar_badge_fg,
                    font.font_size * 0.8,
                ));
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if let InputEvent::MouseDown(mouse) = event {
            if !bounds.contains(mouse.position) {
                return EventResult::Ignored;
            }
            let slot_h = 48.0;
            let idx = ((mouse.position.y - bounds.min_y()) / slot_h).floor() as usize;
            if idx < self.model.items.len() {
                if idx == self.model.active_item_index {
                    self.last_action = Some(ActivityBarAction::ToggleSidebar);
                } else {
                    self.model.active_item_index = idx;
                    let v_id = self.model.items[idx].view_container_id.clone();
                    self.last_action = Some(ActivityBarAction::SelectView(v_id));
                }
                EventResult::Consumed
            } else {
                EventResult::Ignored
            }
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
    fn test_activity_bar_view_render_and_click() {
        let mut view = ActivityBarView::default();
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        let bounds = Rect::from_xywh(0.0, 0.0, 48.0, 600.0);
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        assert!(!scene.quads().is_empty());
        assert!(scene.contains_text("EXP"));

        // Click search icon (slot 1)
        let click_ev = InputEvent::MouseDown(MouseEvent::new(
            Point::new(24.0, 48.0 + 20.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = view.handle_event(&click_ev, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(view.model.active_item_index, 1);
        assert_eq!(
            view.last_action,
            Some(ActivityBarAction::SelectView("search".to_string()))
        );

        // Click active search icon again -> ToggleSidebar
        let res2 = view.handle_event(&click_ev, bounds);
        assert_eq!(res2, EventResult::Consumed);
        assert_eq!(view.last_action, Some(ActivityBarAction::ToggleSidebar));
    }
}
