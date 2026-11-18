//! Cold Startup Time Performance Benchmark
//!
//! Measures cold startup phases:
//! - ConfigurationService initialization + settings resolution
//! - 100 builtin commands registration in CommandRegistry
//! - ThemeService loading and Dark+ JSON theme parsing
//! - KeybindingService loading and keymap resolution
//! - Complete RustCodeContext service container bootstrap
//! - First GPUI scene rendering into a MockRenderSurface
//! - End-to-end cold start pipeline (verifies < 500 ms target)

use rustcode_app::RustCodeContext;
use rustcode_platform::commands::CommandRegistry;
use rustcode_platform::configuration::ConfigurationService;
use rustcode_platform::keybindings::{KeyChord, KeybindingService, ResolvedKeybinding};
use rustcode_platform::render::MockRenderSurface;
use rustcode_platform::theme::ThemeService;
use serde_json::json;
use std::time::{Duration, Instant};

struct BenchmarkStats {
    name: &'static str,
    samples: Vec<Duration>,
}

impl BenchmarkStats {
    fn new(name: &'static str, mut samples: Vec<Duration>) -> Self {
        samples.sort();
        Self { name, samples }
    }

    fn min(&self) -> Duration {
        *self.samples.first().unwrap_or(&Duration::ZERO)
    }

    fn max(&self) -> Duration {
        *self.samples.last().unwrap_or(&Duration::ZERO)
    }

    fn mean(&self) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        let total: Duration = self.samples.iter().sum();
        total / self.samples.len() as u32
    }

    fn percentile(&self, p: f64) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        let idx = ((self.samples.len() as f64 - 1.0) * (p / 100.0)).round() as usize;
        self.samples[idx]
    }

    fn print(&self) {
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
}

fn generate_100_command_ids() -> Vec<String> {
    let categories = [
        "workbench.action",
        "workbench.view",
        "editor.action",
        "terminal",
        "git",
        "debug",
        "search",
        "explorer",
        "settings",
        "extensions",
    ];

    let mut commands = Vec::with_capacity(100);
    for cat in &categories {
        for i in 1..=10 {
            commands.push(format!("{}.command_{:02}", cat, i));
        }
    }
    commands
}

fn sample_theme_json() -> &'static str {
    r##"{
        "name": "RustCode Dark+",
        "type": "dark",
        "colors": {
            "editor.background": "#1e1e1e",
            "editor.foreground": "#d4d4d4",
            "activityBar.background": "#333333",
            "activityBar.foreground": "#ffffff",
            "sideBar.background": "#252526",
            "sideBar.foreground": "#cccccc",
            "statusBar.background": "#007acc",
            "statusBar.foreground": "#ffffff",
            "tab.activeBackground": "#1e1e1e",
            "tab.inactiveBackground": "#2d2d2d"
        },
        "tokenColors": [
            {
                "name": "Keyword",
                "scope": "keyword, storage.type",
                "settings": { "foreground": "#c586c0" }
            },
            {
                "name": "String",
                "scope": "string",
                "settings": { "foreground": "#ce9178" }
            },
            {
                "name": "Function",
                "scope": "entity.name.function",
                "settings": { "foreground": "#dcdcaa" }
            },
            {
                "name": "Comment",
                "scope": "comment",
                "settings": { "foreground": "#6a9955" }
            }
        ]
    }"##
}

fn main() {
    println!("==========================================================================================");
    println!(" RustCode Benchmark: Cold Startup Time (Target: < 500 ms)");
    println!("==========================================================================================");

    const ITERATIONS: usize = 50;
    let command_ids = generate_100_command_ids();
    assert_eq!(command_ids.len(), 100);

    // 1. Benchmark ConfigurationService init
    let mut config_samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let config = ConfigurationService::new();
        let _ = config.get::<u32>("editor.fontSize");
        let _ = config.get::<bool>("editor.lineNumbers");
        config_samples.push(start.elapsed());
    }
    let config_stats = BenchmarkStats::new("1. ConfigurationService Init", config_samples);
    config_stats.print();

    // 2. Benchmark 100 Builtin Commands Registration
    let mut command_samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let registry = CommandRegistry::new();
        for cmd_id in &command_ids {
            let _ = registry.register(cmd_id, |_| Ok(json!({ "executed": true })));
        }
        command_samples.push(start.elapsed());
    }
    let command_stats = BenchmarkStats::new("2. 100 Builtin Commands Registration", command_samples);
    command_stats.print();

    // 3. Benchmark Theme Loading and JSON parsing
    let theme_json = sample_theme_json();
    let mut theme_samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let theme = ThemeService::parse_theme_json(theme_json).expect("valid theme");
        let mut service = ThemeService::new();
        service.set_theme(theme);
        theme_samples.push(start.elapsed());
    }
    let theme_stats = BenchmarkStats::new("3. Theme Parsing & Loading", theme_samples);
    theme_stats.print();

    // 4. Benchmark Keybindings Loading
    let mut keybinding_samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let keybindings = KeybindingService::new();
        for (i, cmd_id) in command_ids.iter().enumerate().take(50) {
            let key = format!("ctrl+{}", (b'a' + (i % 26) as u8) as char);
            if let Some(chord) = KeyChord::parse(&key) {
                keybindings.register(ResolvedKeybinding {
                    chord,
                    command: cmd_id.clone(),
                    args: None,
                    when: None,
                });
            }
        }
        keybinding_samples.push(start.elapsed());
    }
    let keybinding_stats = BenchmarkStats::new("4. Keybindings Registration", keybinding_samples);
    keybinding_stats.print();

    // 5. Benchmark Complete RustCodeContext Bootstrap
    let mut bootstrap_samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let ctx = RustCodeContext::new();
        for cmd_id in &command_ids {
            let _ = ctx.commands.register(cmd_id, |_| Ok(json!({ "executed": true })));
        }
        bootstrap_samples.push(start.elapsed());
    }
    let bootstrap_stats = BenchmarkStats::new("5. RustCodeContext Complete Bootstrap", bootstrap_samples);
    bootstrap_stats.print();

    // 6. Benchmark First GPUI Scene Rendering
    let mut render_samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let ctx = RustCodeContext::new();
        let window = ctx.create_window();
        let mut surface = MockRenderSurface::new(1920.0, 1080.0);
        let start = Instant::now();
        let scene = window.render(&mut surface);
        assert!(!scene.quads().is_empty(), "Scene should contain rendered quads");
        render_samples.push(start.elapsed());
    }
    let render_stats = BenchmarkStats::new("6. First GPUI Scene Rendering", render_samples);
    render_stats.print();

    // 7. End-to-End Cold Start Pipeline
    let mut end_to_end_samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();

        // Step A: Init context and platform services
        let ctx = RustCodeContext::new();

        // Step B: Register 100 commands
        for cmd_id in &command_ids {
            let _ = ctx.commands.register(cmd_id, |_| Ok(json!({ "ok": true })));
        }

        // Step C: Parse and set theme
        let theme = ThemeService::parse_theme_json(theme_json).expect("valid theme");
        let mut theme_svc = ctx.theme.clone();
        theme_svc.set_theme(theme);

        // Step D: Create workbench window
        let window = ctx.create_window();

        // Step E: Render initial GPUI scene to frame buffer
        let mut surface = MockRenderSurface::new(1920.0, 1080.0);
        let scene = window.render(&mut surface);
        assert!(!scene.quads().is_empty());

        end_to_end_samples.push(start.elapsed());
    }

    let end_to_end_stats = BenchmarkStats::new(">>> Total End-to-End Cold Startup", end_to_end_samples);
    println!("------------------------------------------------------------------------------------------");
    end_to_end_stats.print();
    println!("==========================================================================================");

    let mean_time = end_to_end_stats.mean();
    let target = Duration::from_millis(500);
    println!(
        "Cold start mean: {:?} (Target: < {:?}) => {}",
        mean_time,
        target,
        if mean_time < target { "PASSED" } else { "FAILED" }
    );
    assert!(
        mean_time < target,
        "Cold startup time {:?} exceeded target of {:?}",
        mean_time,
        target
    );
}
