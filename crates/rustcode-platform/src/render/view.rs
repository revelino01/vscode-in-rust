//! GPUI view abstractions and component traits.

use crate::render::event::{EventResult, InputEvent};
use crate::render::font::FontMetrics;
use crate::render::geometry::Rect;
use crate::render::scene::Scene;
use crate::render::theme_style::WorkbenchThemeStyles;

/// Core presentation trait implemented by all workbench and editor visual components.
pub trait View: Send + Sync {
    /// Render this view into the provided `Scene` display list.
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    );

    /// Handle an input event routed to this view.
    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult;
}

/// Context passed down to GPUI elements during scene graph layout and render.
pub struct ViewContext<'a> {
    pub bounds: Rect,
    pub theme: &'a WorkbenchThemeStyles,
    pub font: &'a FontMetrics,
    pub is_focused: bool,
}

impl<'a> ViewContext<'a> {
    pub fn new(bounds: Rect, theme: &'a WorkbenchThemeStyles, font: &'a FontMetrics, is_focused: bool) -> Self {
        Self {
            bounds,
            theme,
            font,
            is_focused,
        }
    }
}

/// GPUI-compatible element trait mirroring Zed's Element/IntoElement model.
pub trait GpuiElement {
    fn render(&mut self, cx: &mut ViewContext) -> Scene;
}
