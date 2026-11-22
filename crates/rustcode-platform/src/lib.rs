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
pub mod os_ux;
pub mod render;
pub mod storage;
pub mod theme;

pub use accessibility::{AccessibilityTree, AccessibleNode, AccessibleRole, AccessibleState};
pub use commands::{CommandCallback, CommandRegistry};
pub use configuration::{
    ConfigurationChangeEvent, ConfigurationInspection, ConfigurationService, ConfigurationTarget,
};
pub use fs::{
    file_uri_to_path, hash_content, is_file_uri, path_starts_with, path_strip_prefix,
    path_to_file_uri, paths_equivalent, strip_windows_verbatim_prefix, FileChangeEvent,
    FileChangeType, FileConflict, FileMetadata, FileSystemError, FileSystemService,
    FileSystemWatcher,
};

pub use keybindings::{KeyChord, KeyCombo, KeybindingService, Modifiers, ResolvedKeybinding};
#[cfg(feature = "desktop")]
pub use os_ux::RfdFileDialogBackend;
pub use os_ux::{
    AppPaths, ArboardClipboardBackend, ClipboardBackend, FileDialogBackend, FileDialogCallback,
    FileDialogOptions, FileDialogRequestKind, InMemoryClipboardBackend, MockFileDialogBackend,
    RecordedDialogRequest, WorkspacePaths,
};
pub use render::geometry::MAX_RECT_COORD;
pub use render::{
    Corners, Edges, EventResult, FontMetrics, GlyphRun, GpuiElement, Hsla, InputEvent, KeyCode,
    KeyEvent, MockRenderSurface, MouseButton, MouseEvent, MouseMoveEvent, Point, Quad, Rect,
    RenderSurface, Rgba, Scene, SceneNode, ScrollEvent, Shadow, Size, View, ViewContext,
    WorkbenchThemeStyles,
};
#[cfg(feature = "desktop")]
pub use storage::SqliteStorageService;
pub use storage::{
    InMemoryStorageBackend, MementoScope, PersistedTabState, PersistedWorkspaceState,
    StorageBackend, StorageError, DEFAULT_MAX_RECENT_WORKSPACES,
};
pub use theme::{ColorTheme, ThemeKind, ThemeService, TokenColorRule};

#[doc(hidden)]
pub mod bench_support {
    use std::time::Duration;

    #[derive(Debug, Clone)]
    pub struct BenchmarkStats {
        pub name: &'static str,
        pub samples: Vec<Duration>,
    }

    impl BenchmarkStats {
        pub fn new(name: &'static str, mut samples: Vec<Duration>) -> Self {
            samples.sort();
            Self { name, samples }
        }

        pub fn min(&self) -> Duration {
            *self.samples.first().unwrap_or(&Duration::ZERO)
        }

        pub fn max(&self) -> Duration {
            *self.samples.last().unwrap_or(&Duration::ZERO)
        }

        pub fn mean(&self) -> Duration {
            if self.samples.is_empty() {
                return Duration::ZERO;
            }
            let total: Duration = self.samples.iter().sum();
            total / self.samples.len() as u32
        }

        pub fn percentile(&self, p: f64) -> Duration {
            if self.samples.is_empty() {
                return Duration::ZERO;
            }
            let clamped = p.clamp(0.0, 100.0);
            let idx = ((self.samples.len() as f64 - 1.0) * (clamped / 100.0)).round() as usize;
            self.samples[idx]
        }

        pub fn ops_per_sec(&self) -> f64 {
            let mean_secs = self.mean().as_secs_f64();
            if mean_secs > 0.0 {
                1.0 / mean_secs
            } else {
                0.0
            }
        }

        pub fn print(&self) {
            println!(
                "{:<40} | Min: {:>8.2?} | Mean: {:>8.2?} | P50: {:>8.2?} | P95: {:>8.2?} | P99: {:>8.2?} | Max: {:>8.2?}",
                self.name,
                self.min(),
                self.mean(),
                self.percentile(50.0),
                self.percentile(95.0),
                self.percentile(99.0),
                self.max(),
            );
        }

        pub fn print_with_ops(&self) {
            println!(
                "{:<42} | Min: {:>8.2?} | Mean: {:>8.2?} | P50: {:>8.2?} | P95: {:>8.2?} | P99: {:>8.2?} | Max: {:>8.2?} | {:>10.0} ops/s",
                self.name,
                self.min(),
                self.mean(),
                self.percentile(50.0),
                self.percentile(95.0),
                self.percentile(99.0),
                self.max(),
                self.ops_per_sec(),
            );
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_benchmark_stats_empty_and_ops_per_sec() {
            let empty = BenchmarkStats::new("empty", vec![]);
            assert_eq!(empty.min(), Duration::ZERO);
            assert_eq!(empty.max(), Duration::ZERO);
            assert_eq!(empty.mean(), Duration::ZERO);
            assert_eq!(empty.percentile(95.0), Duration::ZERO);
            assert_eq!(empty.ops_per_sec(), 0.0);

            let stats = BenchmarkStats::new(
                "latency",
                vec![
                    Duration::from_millis(20),
                    Duration::from_millis(10),
                    Duration::from_millis(30),
                ],
            );
            assert_eq!(stats.min(), Duration::from_millis(10));
            assert_eq!(stats.max(), Duration::from_millis(30));
            assert_eq!(stats.mean(), Duration::from_millis(20));
            assert_eq!(stats.percentile(50.0), Duration::from_millis(20));
            assert!((stats.ops_per_sec() - 50.0).abs() < 1e-6);
        }
    }
}
