use rustcode_base::context_key::ContextKeyService;
use rustcode_editor::model::piece_table::PieceTable;
use rustcode_editor::model::undo_redo::{TextEdit, UndoRedoStack};
use rustcode_platform::commands::CommandRegistry;
use rustcode_platform::configuration::ConfigurationService;
use rustcode_platform::keybindings::{KeyCombo, KeybindingService};
use rustcode_platform::theme::ThemeService;
use serde_json::json;

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
    assert_eq!(buffer.get_line_content(1), Some("    println!(\"RustCode\");".to_string()));

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
    let theme_json = include_str!("../resources/default-themes/dark_plus.json");
    let theme = ThemeService::parse_theme_json(theme_json).unwrap();
    assert_eq!(theme.name, "Dark+ (default dark)");
    assert_eq!(theme.colors.get("editor.background"), Some(&"#1e1e1e".to_string()));
}
