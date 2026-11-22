//! RustCode application binary entry point.

use rustcode_app::cli::{parse_env_args, usage_text, version_text, CliAction, CliOptions, StartupMode};
use rustcode_app::runtime::bootstrap_app;
use rustcode_platform::render::MockRenderSurface;
use std::process::ExitCode;

fn run_headless(opts: &CliOptions) -> ExitCode {
    println!("=======================================================");
    println!(" RustCode (vscode-in-rust) — 100% Vibe Coded in Rust ");
    println!(" High-performance, lightweight VS Code rewrite        ");
    println!("=======================================================");

    let mut app = match bootstrap_app(opts, false) {
        Ok(app) => app,
        Err(err) => {
            eprintln!("Bootstrap error: {err}");
            return ExitCode::from(1);
        }
    };

    let font_size: u32 = app.context.config.get("editor.fontSize").unwrap_or(14);
    println!(
        "Theme: {} ({:?})",
        app.window.theme.current().name,
        app.window.theme.current().kind
    );
    println!("Editor Font Size: {}", font_size);
    println!("Active Sidebar: {}", app.window.view.layout.active_sidebar_view);

    if let Ok(res) = app
        .context
        .commands
        .execute("workbench.action.toggleSidebar", None)
    {
        println!("Executed builtin command: {}", res);
    }

    app.runtime_loop.tick(&mut app.window);
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let scene = app.window.render(&mut surface);
    println!(
        "Rendered GPUI Scene: {} quads, {} text runs",
        scene.quads().len(),
        scene.glyph_runs().len()
    );

    app.runtime_loop.shutdown(&mut app.window);
    println!("Bootstrap completed successfully.");
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let action = match parse_env_args() {
        Ok(action) => action,
        Err(err) => {
            eprintln!("{err}\n\n{}", usage_text());
            return ExitCode::from(2);
        }
    };

    match action {
        CliAction::ShowHelp => {
            println!("{}", usage_text());
            ExitCode::SUCCESS
        }
        CliAction::ShowVersion => {
            println!("{}", version_text());
            ExitCode::SUCCESS
        }
        CliAction::Run(opts) => match opts.startup_mode(cfg!(feature = "desktop")) {
            StartupMode::Headless => run_headless(&opts),
            StartupMode::Desktop => {
                #[cfg(feature = "desktop")]
                {
                    match bootstrap_app(&opts, true) {
                        Ok(app) => {
                            if let Err(err) = rustcode_app::desktop::run_desktop_event_loop(app) {
                                eprintln!("Desktop error: {err}");
                                return ExitCode::from(1);
                            }
                            ExitCode::SUCCESS
                        }
                        Err(err) => {
                            eprintln!("Bootstrap error: {err}");
                            ExitCode::from(1)
                        }
                    }
                }
                #[cfg(not(feature = "desktop"))]
                {
                    run_headless(&opts)
                }
            }
        },
    }
}
