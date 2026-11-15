//! RustCode application binary entry point.

use rustcode_app::RustCodeContext;
use rustcode_platform::render::MockRenderSurface;

fn main() {
    println!("=======================================================");
    println!(" RustCode (vscode-in-rust) — 100% Vibe Coded in Rust ");
    println!(" High-performance, lightweight VS Code rewrite        ");
    println!("=======================================================");

    let app = RustCodeContext::new();

    let font_size: u32 = app.config.get("editor.fontSize").unwrap_or(14);
    println!("Theme: {} ({:?})", app.theme.current().name, app.theme.current().kind);
    println!("Editor Font Size: {}", font_size);
    println!("Active Sidebar: {}", app.layout.active_sidebar_view);

    // Test dispatching a built-in command
    let res = app.commands.execute("workbench.action.toggleSidebar", None).unwrap();
    println!("Executed builtin command: {}", res);

    // Bootstrap GPUI WorkbenchWindow host
    let window = app.create_window();
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let scene = window.render(&mut surface);
    println!(
        "Rendered GPUI Scene: {} quads, {} text runs",
        scene.quads().len(),
        scene.glyph_runs().len()
    );

    println!("Bootstrap completed successfully.");
}
