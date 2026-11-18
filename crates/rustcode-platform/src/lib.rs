//! `rustcode-platform` — Layer 1: Platform Services.
//!
//! Provides configuration management (`ConfigurationService`),
//! keybinding resolution (`KeybindingService`), command registration (`CommandRegistry`),
//! and theme styling (`ThemeService`).

pub mod accessibility;
pub mod commands;
pub mod configuration;
pub mod fs;
pub mod keybindings;
pub mod render;
pub mod theme;

pub use accessibility::{AccessibilityTree, AccessibleNode, AccessibleRole, AccessibleState};
pub use commands::{CommandCallback, CommandRegistry};
pub use configuration::{ConfigurationChangeEvent, ConfigurationService, ConfigurationTarget};
pub use fs::{
    hash_content, FileChangeEvent, FileChangeType, FileConflict, FileMetadata, FileSystemError,
    FileSystemService, FileSystemWatcher,
};

pub use keybindings::{KeyChord, KeyCombo, KeybindingService, Modifiers, ResolvedKeybinding};
pub use render::{
    Corners, Edges, EventResult, FontMetrics, GlyphRun, GpuiElement, Hsla, InputEvent, KeyCode,
    KeyEvent, MockRenderSurface, MouseButton, MouseEvent, MouseMoveEvent, Point, Quad, Rect,
    RenderSurface, Rgba, Scene, SceneNode, ScrollEvent, Shadow, Size, View, ViewContext,
    WorkbenchThemeStyles,
};
pub use theme::{ColorTheme, ThemeKind, ThemeService, TokenColorRule};
