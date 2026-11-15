use rustcode_app::RustCodeContext;
use rustcode_base::context_key::ContextKeyService;
use rustcode_editor::model::piece_table::PieceTable;
use rustcode_editor::model::undo_redo::{TextEdit, UndoRedoStack};
use rustcode_platform::commands::CommandRegistry;
use rustcode_platform::configuration::ConfigurationService;
use rustcode_platform::keybindings::{KeyCombo, KeybindingService, Modifiers};
use rustcode_platform::render::{
    EventResult, FontMetrics, InputEvent, KeyCode, KeyEvent, MockRenderSurface, MouseButton,
    MouseEvent, Point, Rect, ScrollEvent,
};
use rustcode_platform::theme::ThemeService;
use rustcode_workbench::explorer::FileTreeNode;
use serde_json::json;
use std::path::PathBuf;

#[test]
fn test_end_to_end_editor_and_platform_integration() {
    // 1. Initialize platform configuration
    let config = ConfigurationService::new();
    let font_size: u32 = config.get("editor.fontSize").unwrap();
    assert_eq!(font_size, 14);

    // 2. Initialize text buffer and edit
    let mut buffer = PieceTable::from_str("fn main() {\n    println!(\"RustCode\");\n}\n");
    let mut undo_stack = UndoRedoStack::new();

    assert_eq!(buffer.line_count(), 4);
    assert_eq!(
        buffer.get_line_content(1),
        Some("    println!(\"RustCode\");".to_string())
    );

    // Make an edit: replace println text
    buffer.insert(12, "let vibe = true;\n    ");
    undo_stack.push_edit(TextEdit::insert(12, "let vibe = true;\n    "));
    assert_eq!(buffer.line_count(), 5);

    // Undo edit
    undo_stack.undo(&mut buffer);
    assert_eq!(buffer.line_count(), 4);

    // Redo edit
    undo_stack.redo(&mut buffer);
    assert_eq!(buffer.line_count(), 5);

    // 3. Initialize Keybinding and Context Key Service
    let keybindings = KeybindingService::new();
    let ctx = ContextKeyService::new();
    ctx.set_bool("editorTextFocus", true);

    let keymap_json = r#"[
        {
            "key": "ctrl+b",
            "command": "workbench.action.toggleSidebar"
        },
        {
            "key": "ctrl+k ctrl+c",
            "command": "editor.action.commentLine",
            "when": "editorTextFocus"
        }
    ]"#;
    keybindings.load_keybindings_json(keymap_json).unwrap();

    let b_combo = KeyCombo::parse("ctrl+b").unwrap();
    assert_eq!(
        keybindings.handle_key(&b_combo, &ctx),
        Some("workbench.action.toggleSidebar".to_string())
    );

    // 4. Initialize Command Registry and execute command
    let commands = CommandRegistry::new();
    let _sub = commands.register("workbench.action.toggleSidebar", |_| {
        Ok(json!({ "status": "sidebar_toggled" }))
    });

    let res = commands.execute("workbench.action.toggleSidebar", None).unwrap();
    assert_eq!(res, json!({ "status": "sidebar_toggled" }));

    // 5. Theme loading verification
    let theme_json = include_str!("../../../resources/default-themes/dark_plus.json");
    let theme = ThemeService::parse_theme_json(theme_json).unwrap();
    assert_eq!(theme.name, "Dark+ (default dark)");
    assert_eq!(
        theme.colors.get("editor.background"),
        Some(&"#1e1e1e".to_string())
    );
}

#[test]
fn test_end_to_end_gpui_presentation_layer() {
    // 1. Initialize RustCode application context
    let app = RustCodeContext::new();

    // Load custom keybindings
    let keymap_json = r#"[
        {
            "key": "ctrl+b",
            "command": "workbench.action.toggleSidebar"
        },
        {
            "key": "ctrl+shift+p",
            "command": "workbench.action.showCommands"
        }
    ]"#;
    app.keybindings.load_keybindings_json(keymap_json).unwrap();

    // 2. Create GPUI WorkbenchWindow host and render surface
    let mut window = app.create_window();

    // Set up file explorer workspace
    let mut root = FileTreeNode::new_dir(PathBuf::from("/workspace"));
    root.children = Some(vec![
        FileTreeNode::new_file(PathBuf::from("/workspace/main.rs")),
        FileTreeNode::new_file(PathBuf::from("/workspace/Cargo.toml")),
    ]);
    window.view.explorer.model.root = Some(root);
    window.view.explorer.model.expanded_paths.insert(PathBuf::from("/workspace"));

    // Open file in editor area
    window.open_file(
        "file:///workspace/main.rs",
        "main.rs",
        "fn main() {\n    let answer = 42;\n    println!(\"{}\", answer);\n}\n",
        false,
    );

    // 3. Render scene to mock surface (1024x768)
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let scene = window.render(&mut surface);

    // Verify scene graph display list primitives
    assert!(!scene.quads().is_empty());
    assert!(!scene.glyph_runs().is_empty());

    // Verify workbench regions rendered:
    // - ActivityBar items
    assert!(scene.contains_text("EXP"));
    // - Explorer tree items
    assert!(scene.contains_text("workspace"));
    assert!(scene.contains_text("main.rs"));
    // - Editor buffer text & gutter
    assert!(scene.contains_text("fn main() {"));
    assert!(scene.contains_text("let answer = 42;"));

    // 4. Input Routing: Keybinding chord Ctrl+B toggles sidebar
    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
    assert!(window.view.layout.sidebar_visible);

    let ctrl_b = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('b'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    let res = window.dispatch_event(&ctrl_b, bounds);
    assert_eq!(res, EventResult::Consumed);
    assert!(!window.view.layout.sidebar_visible);

    // Press Ctrl+B again to show sidebar
    let res = window.dispatch_event(&ctrl_b, bounds);
    assert_eq!(res, EventResult::Consumed);
    assert!(window.view.layout.sidebar_visible);

    // 5. Input Routing: Keybinding chord Ctrl+Shift+P opens Command Palette
    let ctrl_shift_p = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('p'),
        Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        },
    ));
    let res = window.dispatch_event(&ctrl_shift_p, bounds);
    assert_eq!(res, EventResult::Consumed);
    assert!(window.view.is_command_palette_open);

    // Re-render scene with Command Palette overlay
    let mut overlay_surface = MockRenderSurface::new(1024.0, 768.0);
    let overlay_scene = window.render(&mut overlay_surface);
    assert!(overlay_scene.contains_text("> "));

    // Type query "side" into Command Palette
    for c in "side".chars() {
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(c), Modifiers::none())),
            bounds,
        );
    }
    // Filtered results should contain toggleSidebar
    let palette = window.view.command_palette.as_ref().unwrap();
    assert_eq!(palette.model.results().len(), 1);
    assert_eq!(
        palette.model.selected_item().unwrap().command_id,
        "workbench.action.toggleSidebar"
    );

    // Press Enter to execute command and close palette
    let enter_ev = InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none()));
    let res = window.dispatch_event(&enter_ev, bounds);
    assert_eq!(res, EventResult::Consumed);
    assert!(!window.view.is_command_palette_open);

    // 6. Input Routing: Mouse click and text editing in EditorView
    let regions = window.view.compute_regions(bounds);
    let editor_rect = regions.editor_area;

    // Mouse click at line 1, column 4 inside editor
    let font = FontMetrics::default_monospace();
    let gutter_w = window
        .view
        .editor_area
        .active_editor()
        .unwrap()
        .gutter_width(&font);
    let click_x = editor_rect.min_x() + gutter_w + 8.0 + 8.0 * font.char_width;
    let click_y = editor_rect.min_y() + 35.0 + 1.0 * font.line_height + 5.0; // 35px tab bar offset

    let click_ev = InputEvent::MouseDown(MouseEvent::new(
        Point::new(click_x, click_y),
        MouseButton::Left,
    ));
    let res = window.dispatch_event(&click_ev, bounds);
    assert_eq!(res, EventResult::Consumed);

    let active_editor = window.view.editor_area.active_editor().unwrap();
    assert_eq!(active_editor.primary_position().line, 1);
    assert_eq!(active_editor.primary_position().column, 8);

    // Type character '!' at cursor
    let char_ev = InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('!'), Modifiers::none()));
    window.dispatch_event(&char_ev, bounds);

    let updated_text = window.view.editor_area.active_editor().unwrap().text();
    assert!(updated_text.contains("let !answer = 42;"));

    // Mouse scroll inside editor
    let scroll_ev = InputEvent::ScrollWheel(ScrollEvent::new(
        Point::new(click_x, click_y),
        0.0,
        -10.0,
    ));
    let res = window.dispatch_event(&scroll_ev, bounds);
    assert_eq!(res, EventResult::Consumed);
}
