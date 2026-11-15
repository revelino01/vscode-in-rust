//! `rustcode-workbench` — Layer 3: Workbench UI Shell.
//!
//! Provides the core shell state mirroring VS Code's workbench:
//! - Layout regions (`WorkbenchLayout`)
//! - Activity bar (`ActivityBar`)
//! - File explorer model (`FileExplorer`, `FileTreeNode`)
//! - Editor tab & group management (`EditorArea`, `EditorGroup`, `EditorTab`)
//! - Command palette with fuzzy search (`CommandPalette`, `PaletteItem`)
//! - Status bar management (`StatusBar`, `StatusBarItem`, `StatusBarAlignment`)

pub mod activitybar;
pub mod editor_area;
pub mod explorer;
pub mod layout;
pub mod quickinput;
pub mod statusbar;

pub use activitybar::{ActivityBar, ActivityBarItem};
pub use editor_area::{EditorArea, EditorGroup, EditorTab};
pub use explorer::{FileExplorer, FileTreeNode};
pub use layout::WorkbenchLayout;
pub use quickinput::{CommandPalette, PaletteItem};
pub use statusbar::{StatusBar, StatusBarAlignment, StatusBarItem};
