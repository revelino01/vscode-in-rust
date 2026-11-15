//! Presentation layer primitives: geometry, colors, scene graph, theme styles, font metrics, and input events.

pub mod color;
pub mod event;
pub mod font;
pub mod geometry;
pub mod scene;
pub mod surface;
pub mod theme_style;
pub mod view;

pub use color::{Hsla, Rgba};
pub use event::{
    EventResult, InputEvent, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseMoveEvent,
    ScrollEvent,
};
pub use font::FontMetrics;
pub use geometry::{Corners, Edges, Point, Rect, Size};
pub use scene::{GlyphRun, Quad, Scene, SceneNode, Shadow};
pub use surface::{MockRenderSurface, RenderSurface};
pub use theme_style::WorkbenchThemeStyles;
pub use view::{GpuiElement, View, ViewContext};
