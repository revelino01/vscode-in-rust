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

    // 7. Multi-byte UTF-8 editing, cursor placement, and undo/redo round-trip
    let editor = window.view.editor_area.active_editor_mut().unwrap();
    editor.set_primary_position(rustcode_editor::cursor::Position::new(0, 0));
    editor.insert_text("🚀");
    assert!(editor.text().starts_with("🚀fn main()"));
    assert_eq!(editor.primary_position(), rustcode_editor::cursor::Position::new(0, 4));

    // Delete backwards removes emoji completely
    editor.delete_backwards();
    assert!(editor.text().starts_with("fn main()"));
    assert_eq!(editor.primary_position(), rustcode_editor::cursor::Position::new(0, 0));

    // Undo restores emoji and cursor position
    assert!(editor.undo());
    assert!(editor.text().starts_with("🚀fn main()"));
    assert_eq!(editor.primary_position(), rustcode_editor::cursor::Position::new(0, 4));

    // Redo reverts to deleted state
    assert!(editor.redo());
    assert!(editor.text().starts_with("fn main()"));
    assert_eq!(editor.primary_position(), rustcode_editor::cursor::Position::new(0, 0));
}

#[tokio::test]
async fn test_end_to_end_syntax_highlighting_and_file_io() {
    use rustcode_editor::{AutoSaveConfig, AutoSaveMode, AutoSaveService, TextDocument};
    use rustcode_platform::fs::{FileConflict, FileSystemService};
    use rustcode_platform::render::Rgba;
    use std::sync::Arc;
    use parking_lot::RwLock;


    let tmp_dir = std::env::temp_dir().join(format!("rustcode_e2e_{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let file_path = tmp_dir.join("main.rs");
    let uri = format!("file://{}", file_path.display());

    let fs = FileSystemService::new();
    let initial_code = "/* multi-line\n   comment */\npub fn add(a: i32, b: i32) -> i32 {\n    let sum_π = a + b;\n    sum_π\n}\n";

    // 1. Write file atomically
    let meta = fs.write_atomic(&file_path, initial_code).await.unwrap();
    assert_eq!(meta.size, initial_code.len() as u64);

    // 2. Open file in document model
    let doc = Arc::new(RwLock::new(TextDocument::open_async(&file_path).await.unwrap()));
    assert!(!doc.read().is_dirty);
    assert_eq!(doc.read().language_id, "rust");

    // 3. Set up auto-save service
    let mut auto_save = AutoSaveService::new(AutoSaveConfig {
        mode: AutoSaveMode::OnFocusChange,
        delay: std::time::Duration::from_millis(50),
    });
    auto_save.register(doc.clone());

    // 4. Create workbench window and open file with syntax highlighting
    let app = RustCodeContext::new();
    let mut window = app.create_window();
    window.open_file(&uri, "main.rs", initial_code, false);


    // 5. Render window to mock surface
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let scene = window.render(&mut surface);

    // Verify syntax colors present: keyword purple (#C586C0) and comment green (#6A9955)
    let keyword_color = Rgba::from_hex("#C586C0").unwrap();
    let comment_color = Rgba::from_hex("#6A9955").unwrap();
    assert!(scene.glyph_runs().iter().any(|r| r.color == keyword_color));
    assert!(scene.glyph_runs().iter().any(|r| r.color == comment_color));
    assert!(scene.contains_text("pub fn add"));
    assert!(scene.contains_text("sum_π"));

    // 6. Edit file in editor
    let editor = window.view.editor_area.active_editor_mut().unwrap();
    editor.insert_text("// comment\n");
    assert!(editor.is_dirty());

    // 7. Save active file
    let saved_meta = window.view.editor_area.save_active_file(&fs).unwrap();
    assert_eq!(saved_meta.path, file_path);
    assert!(!window.view.editor_area.active_editor().unwrap().is_dirty());

    // Verify external conflict check
    std::fs::write(&file_path, "externally modified!").unwrap();
    let conflict = fs.check_conflict(&file_path, &saved_meta).unwrap();
    assert!(matches!(conflict, Some(FileConflict::ModifiedOnDisk { .. })));

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[tokio::test]
async fn test_end_to_end_lsp_and_autocomplete_integration() {
    use rustcode_editor::{CompletionItem, CompletionItemKind};
    use rustcode_ext_host::{LanguageFeatureBridge, ProviderRegistry};
    use rustcode_protocols::lsp::{
        read_framed_message, write_framed_message, InitializeParams, LspClient, LspClientManager,
        LspPosition,
    };
    use std::sync::Arc;
    use tokio::io::duplex;

    // 1. Set up LSP Client with Mock Language Server over duplex
    let (client_read, mut server_write) = duplex(4096);
    let (mut server_read, client_write) = duplex(4096);
    let lsp_client = Arc::new(LspClient::from_io(client_read, client_write));

    tokio::spawn(async move {
        let mut buf = Vec::new();

        // Expect initialize
        let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
        let req: serde_json::Value = serde_json::from_slice(&msg).unwrap();
        assert_eq!(req["method"], "initialize");
        let id = req["id"].as_u64().unwrap();
        let resp = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "capabilities": {
                    "textDocumentSync": 1,
                    "completionProvider": { "triggerCharacters": [".", ":"] }
                }
            }
        });
        write_framed_message(&mut server_write, &serde_json::to_vec(&resp).unwrap()).await.unwrap();

        // Expect initialized
        let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
        let notif: serde_json::Value = serde_json::from_slice(&msg).unwrap();
        assert_eq!(notif["method"], "initialized");

        // Expect didOpen
        let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
        let open: serde_json::Value = serde_json::from_slice(&msg).unwrap();
        assert_eq!(open["method"], "textDocument/didOpen");

        // Expect completion request
        let msg = read_framed_message(&mut server_read, &mut buf).await.unwrap().unwrap();
        let comp: serde_json::Value = serde_json::from_slice(&msg).unwrap();
        assert_eq!(comp["method"], "textDocument/completion");
        let comp_id = comp["id"].as_u64().unwrap();
        let comp_resp = json!({
            "jsonrpc": "2.0",
            "id": comp_id,
            "result": [
                {
                    "label": "to_uppercase",
                    "kind": 2,
                    "detail": "fn(&self) -> String",
                    "documentation": "Returns a new uppercase copy of the string.",
                    "insertText": "to_uppercase()"
                }
            ]
        });
        write_framed_message(&mut server_write, &serde_json::to_vec(&comp_resp).unwrap()).await.unwrap();
    });

    let lsp_mgr = Arc::new(LspClientManager::new());
    lsp_client.initialize(InitializeParams::default()).await.unwrap();
    lsp_client.initialized().await.unwrap();
    lsp_mgr.attach_client("rust", lsp_client.clone());

    // 2. Set up LanguageFeatureBridge
    let reg = Arc::new(ProviderRegistry::new());
    let rpc = Arc::new(rustcode_ext_host::RpcBridge::new());
    let bridge = Arc::new(LanguageFeatureBridge::new(reg, rpc, lsp_mgr.clone()));

    // 3. Set up RustCode window and open document
    let app = RustCodeContext::new();
    let mut window = app.create_window();
    let initial_text = "fn main() {\n    let text = \"rust\";\n}\n";
    window.open_file("file:///workspace/main.rs", "main.rs", initial_text, false);

    // Sync didOpen to LSP
    lsp_mgr
        .did_open_document("file:///workspace/main.rs", "rust", 1, initial_text)
        .await
        .unwrap();

    // Query completions from LSP via bridge
    let lsp_items = bridge
        .request_completions(
            "file:///workspace/main.rs",
            "rust",
            LspPosition::new(1, 20),
            Some('.'),
        )
        .await;
    assert_eq!(lsp_items.len(), 1);
    assert_eq!(lsp_items[0].label, "to_uppercase");

    // 4. Attach completion provider to active editor view
    let editor = window.view.editor_area.active_editor_mut().unwrap();
    editor.completion_provider = Some(Arc::new(move |_lang, _pos, _trigger| {
        vec![CompletionItem::new("to_uppercase")
            .with_kind(CompletionItemKind::Method)
            .with_detail("fn(&self) -> String")
            .with_doc("Returns a new uppercase copy of the string.")
            .with_insert_text("to_uppercase()")]
    }));

    // Move cursor right after "text" in `let text = "rust";` (line 1, column 12)
    editor.set_primary_position(rustcode_editor::cursor::Position::new(1, 12));

    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);

    // 5. Type '.' -> Triggers autocomplete
    let dot_ev = InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('.'), Modifiers::none()));
    let res = window.dispatch_event(&dot_ev, bounds);
    assert_eq!(res, EventResult::Consumed);

    assert!(window
        .view
        .editor_area
        .active_editor()
        .unwrap()
        .autocomplete
        .is_visible);

    // Type "to_u" to live-filter completion candidates specifically to "to_uppercase"
    for ch in "to_u".chars() {
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
            bounds,
        );
    }

    // 6. Render surface: verify autocomplete popover & documentation preview present in scene graph
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("to_uppercase"));
    assert!(scene.contains_text("fn(&self) -> String"));

    // 7. Commit completion item with Tab key
    let tab_ev = InputEvent::KeyDown(KeyEvent::new(KeyCode::Tab, Modifiers::none()));
    let res = window.dispatch_event(&tab_ev, bounds);
    assert_eq!(res, EventResult::Consumed);

    let editor_mut = window.view.editor_area.active_editor_mut().unwrap();
    assert!(!editor_mut.autocomplete.is_visible);
    assert!(editor_mut.text().contains("let text.to_uppercase() = \"rust\";"));

    // 8. Undo reverts completion
    assert!(editor_mut.undo());
    assert!(editor_mut.text().contains("let text.to_u = \"rust\";"));
}

#[tokio::test]
async fn test_end_to_end_diagnostics_and_problems_panel() {
    use rustcode_editor::cursor::Position;
    use rustcode_protocols::lsp::types::{
        DiagnosticCode, DiagnosticSeverity as LspSeverity, LspDiagnostic, LspPosition, LspRange,
    };

    let app = RustCodeContext::new();
    let mut window = app.create_window();
    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);

    // 1. Open a document in the editor
    let code = "fn compute() {\n    let answer: i32 = \"forty-two\";\n}\n";
    window.view.editor_area.open_file("src/calc.rs", "calc.rs", code, true);

    // 2. Ingest diagnostics from LSP publishDiagnostics notifications
    let lsp_diags = vec![
        LspDiagnostic {
            range: LspRange::new(LspPosition::new(1, 22), LspPosition::new(1, 33)),
            severity: Some(LspSeverity::Error),
            message: "mismatched types: expected `i32`, found `&str`".to_string(),
            source: Some("rust-analyzer".to_string()),
            code: Some(DiagnosticCode::String("E0308".to_string())),
        },
        LspDiagnostic {
            range: LspRange::new(LspPosition::new(1, 8), LspPosition::new(1, 14)),
            severity: Some(LspSeverity::Warning),
            message: "unused variable: `answer`".to_string(),
            source: Some("rustc".to_string()),
            code: Some(DiagnosticCode::String("unused_variables".to_string())),
        },
    ];

    // Single call ingests LSP diagnostics, updating workbench problems AND editor diagnostics
    window.view.update_lsp_problems("src/calc.rs", &lsp_diags);
    assert_eq!(
        window.view.editor_area.active_editor().unwrap().diagnostics.len(),
        2
    );

    // 3. Render: verify status bar item reflects error & warning count badges
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("$(error) 1  $(warning) 1"));

    // Verify squiggly waves rendered in editor
    let editor_theme = window.theme.current();
    let theme_styles = rustcode_platform::render::WorkbenchThemeStyles::from_theme(editor_theme);
    let has_error_wave = scene.quads().iter().any(|q| q.background == Some(theme_styles.editor_error_foreground));
    let has_warning_wave = scene.quads().iter().any(|q| q.background == Some(theme_styles.editor_warning_foreground));
    assert!(has_error_wave, "Red squiggly underline must be rendered in editor");
    assert!(has_warning_wave, "Yellow squiggly underline must be rendered in editor");

    // 4. Hover over the error diagnostic span: verify hover tooltip in scene
    let font = FontMetrics::default_monospace();
    let gutter_w = window.view.editor_area.active_editor().unwrap().gutter_width(&font);
    let editor_regions = window.view.compute_regions(bounds);
    let target_x = editor_regions.editor_area.min_x() + gutter_w + 8.0 + 25.0 * font.char_width;
    let target_y = editor_regions.editor_area.min_y() + 35.0 + 1.5 * font.line_height;

    let hover_ev = InputEvent::MouseMove(rustcode_platform::render::MouseMoveEvent {
        position: Point::new(target_x, target_y),
    });
    window.dispatch_event(&hover_ev, bounds);

    let hover_scene = window.render(&mut surface);
    assert!(hover_scene.contains_text("rust-analyzer (E0308)"));
    assert!(hover_scene.contains_text("mismatched types"));

    // 5. Verify clicking status bar badge opens bottom panel to problems tab
    assert!(!window.view.layout.panel_visible);
    let status_regions = window.view.compute_regions(bounds);
    let badge_click = InputEvent::MouseDown(MouseEvent::new(
        Point::new(status_regions.status_bar.min_x() + 30.0, status_regions.status_bar.min_y() + 10.0),
        MouseButton::Left,
    ));
    window.dispatch_event(&badge_click, bounds);
    assert!(window.view.layout.panel_visible, "Status bar badge click must open bottom panel");
    assert_eq!(window.view.layout.active_panel_tab, "problems");

    let panel_scene = window.render(&mut surface);
    assert!(panel_scene.contains_text("PROBLEMS 2"));
    assert!(panel_scene.contains_text("src/calc.rs"));
    assert!(panel_scene.contains_text("[Error]"));
    assert!(panel_scene.contains_text("[Warning]"));

    // 6. Click on problem row in Problems panel: jumps to location in editor
    let panel_regions = window.view.compute_regions(bounds);
    let click_y = panel_regions.panel.min_y() + 26.0 + 24.0 + 22.0 + 5.0;
    let click_x = panel_regions.panel.min_x() + 40.0;
    let click_ev = InputEvent::MouseDown(MouseEvent::new(Point::new(click_x, click_y), MouseButton::Left));
    window.dispatch_event(&click_ev, bounds);

    assert_eq!(window.view.focused_region, rustcode_workbench::FocusedRegion::Editor);
    let active_pos = window.view.editor_area.active_editor().unwrap().primary_position();
    assert_eq!(active_pos, Position::new(1, 22));
}

#[tokio::test]
async fn test_end_to_end_open_vsx_marketplace_and_downloader() {
    use rustcode_ext_host::marketplace::{
        ExtensionDownloader, ExtensionInstaller, MarketplaceClient, MockHttpTransport,
    };
    use rustcode_platform::commands::CommandRegistry;
    use rustcode_platform::theme::ThemeService;
    use std::io::Write;
    use std::sync::Arc;
    use zip::write::SimpleFileOptions;

    // 1. Create a mock .vsix archive containing package.json and themes
    let manifest_json = r##"{
        "name": "synthwave-84",
        "publisher": "robbowen",
        "version": "0.1.15",
        "displayName": "SynthWave '84",
        "contributes": {
            "commands": [
                {
                    "command": "synthwave84.enableNeon",
                    "title": "Enable Neon Dreams",
                    "category": "SynthWave '84"
                }
            ],
            "themes": [
                {
                    "label": "SynthWave '84",
                    "uiTheme": "vs-dark",
                    "path": "./themes/synthwave.json"
                }
            ]
        }
    }"##;

    let theme_json = r##"{
        "name": "SynthWave '84",
        "type": "dark",
        "colors": {
            "editor.background": "#262335",
            "editor.foreground": "#ffffff"
        }
    }"##;

    let mut vsix_buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut vsix_buf);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("extension/package.json", options).unwrap();
        zip.write_all(manifest_json.as_bytes()).unwrap();
        zip.start_file("extension/themes/synthwave.json", options).unwrap();
        zip.write_all(theme_json.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    let vsix_bytes = vsix_buf.into_inner();

    // Verify in-memory manifest parsing from VSIX buffer without disk unpack
    let memory_manifest = ExtensionDownloader::read_manifest_from_vsix(&vsix_bytes).unwrap();
    assert_eq!(memory_manifest.name, "synthwave-84");
    assert_eq!(memory_manifest.publisher, "robbowen");

    // 2. Set up Mock Open VSX HTTP transport
    let transport = Arc::new(MockHttpTransport::new());
    let download_url = "https://open-vsx.org/api/robbowen/synthwave-84/0.1.15/file/synthwave-84.vsix";

    // Mock search endpoint
    transport.set_response(
        "https://open-vsx.org/api/-/search?query=synthwave&size=10&offset=0",
        serde_json::to_vec(&serde_json::json!({
            "totalSize": 1,
            "extensions": [
                {
                    "namespace": "robbowen",
                    "name": "synthwave-84",
                    "version": "0.1.15",
                    "displayName": "SynthWave '84",
                    "downloadUrl": download_url
                }
            ]
        })).unwrap(),
    );

    // Mock metadata endpoint
    transport.set_response(
        "https://open-vsx.org/api/robbowen/synthwave-84",
        serde_json::to_vec(&serde_json::json!({
            "namespace": "robbowen",
            "name": "synthwave-84",
            "version": "0.1.15",
            "downloadUrl": download_url
        })).unwrap(),
    );

    // Mock download binary endpoint
    transport.set_response(download_url, vsix_bytes.clone());

    let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));

    // 3. Search extensions via client
    let search_res = client.search("synthwave", Some(10), Some(0)).await.unwrap();
    assert_eq!(search_res.total_size, 1);
    assert_eq!(search_res.extensions[0].name, "synthwave-84");

    // 4. Download and unpack via installer
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("rustcode_e2e_ext_{}", now));
    let installer = ExtensionInstaller::new(client, &temp_dir);

    let loaded = installer
        .install_from_marketplace("robbowen", "synthwave-84", None)
        .await
        .unwrap();

    assert_eq!(loaded.manifest.name, "synthwave-84");
    assert_eq!(loaded.manifest.publisher, "robbowen");
    assert!(loaded.root_dir.join("package.json").exists());
    assert!(loaded.root_dir.join("themes/synthwave.json").exists());

    // Verify direct installation from VSIX bytes (single unpack directly into target folder)
    let direct_loaded = installer.install_from_vsix(&vsix_bytes, None).unwrap();
    assert_eq!(direct_loaded.manifest.name, "synthwave-84");
    assert!(direct_loaded.root_dir.join("package.json").exists());

    // 5. Register runtime contributions
    let commands = CommandRegistry::new();
    let mut theme_service = ThemeService::new();

    let (registered_count, _subs) = installer
        .register_contributions(&loaded, &commands, Some(&mut theme_service), |_lang, _path| Ok(()))
        .unwrap();

    assert_eq!(registered_count, 2);

    // Verify command works
    assert!(commands.has_command("synthwave84.enableNeon"));
    let cmd_res = commands.execute("synthwave84.enableNeon", None).unwrap();
    assert_eq!(cmd_res.get("command").and_then(|c| c.as_str()), Some("synthwave84.enableNeon"));

    // Verify theme applied
    assert_eq!(theme_service.current().name, "SynthWave '84");
    assert_eq!(
        theme_service.current().colors.get("editor.background").map(|s| s.as_str()),
        Some("#262335")
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_end_to_end_git_scm_and_diff_viewer() {
    use rustcode_editor::{DiffModel, DiffViewMode};
    use rustcode_platform::View;
    use rustcode_protocols::git::ScmRepository;
    use rustcode_workbench::{ScmView, WorkbenchView};

    // 1. Initialize SCM repository with staged, modified, and untracked files
    let repo = ScmRepository::sample_repository();
    assert_eq!(repo.staged.len(), 1);
    assert_eq!(repo.changes.len(), 1);
    assert_eq!(repo.untracked.len(), 1);

    // 2. Myers diff verification on sample file
    let (orig, modif) = {
        let (o, m) = repo.get_file_diff("src/main.rs").unwrap();
        (o.to_string(), m.to_string())
    };
    let diff_model = DiffModel::compute(&orig, &modif);
    assert!(diff_model.additions > 0);
    assert!(diff_model.inline_rows.iter().any(|r| r.prefix == '+'));
    assert!(diff_model.side_by_side_rows.iter().any(|r| r.left.is_some() && r.right.is_some()));

    // 3. Initialize WorkbenchView with SCM
    let mut wb = WorkbenchView::new();
    wb.scm_view = ScmView::new(repo);

    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
    let font = FontMetrics::default_monospace();
    let theme_styles = rustcode_platform::render::WorkbenchThemeStyles::dark_default();

    // 4. Switch Activity Bar to SCM view
    wb.layout.set_sidebar_view("scm");
    assert_eq!(wb.layout.active_sidebar_view, "scm");

    // Render SCM sidebar in workbench
    let mut scene = rustcode_platform::render::Scene::new();
    wb.render(bounds, &theme_styles, &font, &mut scene);
    assert!(!scene.quads().is_empty());

    // 5. Open diff in editor area
    let diff_uri = "src/main.rs (Working Tree)";
    wb.editor_area.open_diff(diff_uri, "main.rs (Diff)", &orig, &modif);
    assert!(wb.editor_area.diff_editors.contains_key(diff_uri));

    let diff_view = wb.editor_area.diff_editors.get_mut(diff_uri).unwrap();
    assert_eq!(diff_view.mode, DiffViewMode::SideBySide);
    diff_view.toggle_mode();
    assert_eq!(diff_view.mode, DiffViewMode::Inline);

    // 6. Stage untracked file via SCM view
    assert!(wb.scm_view.repo.stage_file("README.md"));
    assert_eq!(wb.scm_view.repo.staged.len(), 2);
    assert_eq!(wb.scm_view.repo.untracked.len(), 0);

    // 7. Commit changes via commit message input and execute
    wb.scm_view.set_commit_message("feat: update Cargo.toml and README");
    let commit_hash = wb.scm_view.execute_commit().unwrap();
    assert!(!commit_hash.is_empty());
    assert!(wb.scm_view.repo.staged.is_empty());
    assert_eq!(wb.scm_view.repo.commits.len(), 1);
    assert_eq!(wb.scm_view.commit_message, "");
}

#[tokio::test]
async fn test_end_to_end_dap_debug_integration() {
    use rustcode_editor::{EditorGutterAction, EditorView};
    use rustcode_platform::View;
    use rustcode_protocols::codec::{read_framed_message, write_framed_message};
    use rustcode_protocols::dap::{
        DapClient, DapMessage, InitializeRequestArguments, SourceBreakpoint,
    };
    use rustcode_workbench::{
        DebugSessionState, DebugToolbarAction, WorkbenchAction, WorkbenchView,
    };
    use tokio::io::duplex;

    // 1. Breakpoints in EditorView gutter
    let mut editor = EditorView::from_str("fn main() {\n    let mut sum = 0;\n    sum += 42;\n}\n");
    let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
    let font = FontMetrics::default_monospace();
    let theme_styles = rustcode_platform::render::WorkbenchThemeStyles::dark_default();

    // Toggle breakpoint on line 3
    assert!(!editor.has_breakpoint(3));
    let gutter_click_y = bounds.min_y() + font.line_height * 2.5;
    let click_ev = InputEvent::MouseDown(MouseEvent::new(
        Point::new(bounds.min_x() + 8.0, gutter_click_y),
        MouseButton::Left,
    ));
    let res = editor.handle_event(&click_ev, bounds);
    assert_eq!(res, EventResult::Consumed);
    assert!(editor.has_breakpoint(3));
    assert_eq!(
        editor.last_gutter_action,
        Some(EditorGutterAction::BreakpointToggled { line: 3, added: true })
    );

    // Render scene and verify breakpoint red dot
    let mut scene = rustcode_platform::render::Scene::new();
    editor.render(bounds, &theme_styles, &font, &mut scene);
    let has_red_dot = scene.quads().iter().any(|q| {
        q.background
            .is_some_and(|bg| (bg.r - 0.9).abs() < 0.05 && (bg.g - 0.15).abs() < 0.05)
    });
    assert!(has_red_dot);

    // 2. DAP Client communication over stdio duplex stream
    let (client_io, server_io) = duplex(4096);
    let (c_reader, c_writer) = tokio::io::split(client_io);
    let dap_client = DapClient::from_io(c_reader, c_writer);

    tokio::spawn(async move {
        let (mut s_reader, mut s_writer) = tokio::io::split(server_io);
        let mut buf = Vec::new();

        while let Ok(Some(bytes)) = read_framed_message(&mut s_reader, &mut buf).await {
            let msg: DapMessage = serde_json::from_slice(&bytes).unwrap();
            if let DapMessage::Request { seq, command, .. } = msg {
                match command.as_str() {
                    "initialize" => {
                        let resp = DapMessage::Response {
                            seq: 1,
                            request_seq: seq,
                            success: true,
                            command,
                            message: None,
                            body: Some(json!({ "supportsConfigurationDoneRequest": true })),
                        };
                        let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                    }
                    "setBreakpoints" => {
                        let resp = DapMessage::Response {
                            seq: 2,
                            request_seq: seq,
                            success: true,
                            command,
                            message: None,
                            body: Some(json!({
                                "breakpoints": [
                                    { "verified": true, "line": 3 }
                                ]
                            })),
                        };
                        let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                    }
                    "threads" => {
                        let resp = DapMessage::Response {
                            seq: 3,
                            request_seq: seq,
                            success: true,
                            command,
                            message: None,
                            body: Some(json!({
                                "threads": [{ "id": 1, "name": "Worker Thread" }]
                            })),
                        };
                        let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                    }
                    "stackTrace" => {
                        let resp = DapMessage::Response {
                            seq: 4,
                            request_seq: seq,
                            success: true,
                            command,
                            message: None,
                            body: Some(json!({
                                "stackFrames": [
                                    { "id": 100, "name": "main()", "line": 3, "column": 5 }
                                ]
                            })),
                        };
                        let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                    }
                    "next" => {
                        let resp = DapMessage::Response {
                            seq: 5,
                            request_seq: seq,
                            success: true,
                            command,
                            message: None,
                            body: None,
                        };
                        let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                    }
                    "disconnect" => {
                        let resp = DapMessage::Response {
                            seq: 6,
                            request_seq: seq,
                            success: true,
                            command,
                            message: None,
                            body: None,
                        };
                        let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap()).await;
                        break;
                    }
                    _ => {}
                }
            }
        }
    });

    let caps = dap_client
        .initialize(InitializeRequestArguments {
            adapter_id: "lldb".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(caps.supports_configuration_done_request, Some(true));

    let bps = dap_client
        .set_breakpoints("src/main.rs", &[SourceBreakpoint::new(3)])
        .await
        .unwrap();
    assert_eq!(bps.len(), 1);
    assert!(bps[0].verified);

    let threads = dap_client.threads().await.unwrap();
    assert_eq!(threads[0].name, "Worker Thread");

    let frames = dap_client.stack_trace(1).await.unwrap();
    assert_eq!(frames[0].id, 100);
    assert_eq!(frames[0].line, 3);

    assert!(dap_client.next(1).await.is_ok());
    assert!(dap_client.disconnect(false).await.is_ok());

    // 3. Floating Debug Toolbar & Sidebar Panels
    let mut wb = WorkbenchView::new();
    wb.debug_sidebar.session = DebugSessionState::sample_session();
    wb.debug_toolbar.is_visible = true;

    // Test F10 (Step Over) keyboard shortcut
    let f10_key = InputEvent::KeyDown(KeyEvent::new(KeyCode::F10, Modifiers::default()));
    let res = wb.handle_event(&f10_key, bounds);
    assert_eq!(res, EventResult::Consumed);
    assert_eq!(
        wb.last_action,
        Some(WorkbenchAction::DebugToolbar(DebugToolbarAction::StepOver))
    );

    // Test Call Stack navigation in Debug Sidebar
    wb.layout.set_sidebar_view("debug");
    let _ = wb.debug_sidebar.handle_event(
        &InputEvent::MouseDown(MouseEvent::new(Point::new(50.0, 150.0), MouseButton::Left)),
        Rect::from_xywh(48.0, 0.0, 280.0, 768.0),
    );

    // Test Debug Console in bottom panel
    wb.layout.set_panel_tab("debug_console");
    wb.debug_console
        .append_output("stdout", "Process initialized at 0x7fff0000\n");
    assert_eq!(wb.debug_console.output_lines.len(), 1);
}

#[test]
fn test_end_to_end_webview_host_integration() {
    use rustcode_ext_host::webview::{ExtHostWebviewManager, WebviewOptions};
    use rustcode_platform::View;
    use rustcode_workbench::webview::WebviewElementKind;
    use rustcode_workbench::WorkbenchView;

    let ext_host_mgr = ExtHostWebviewManager::new();

    // 1. Extension Host creates webview panel
    let panel_state = ext_host_mgr.create_panel(
        "cat_coding_panel",
        "catCoding",
        "Cat Coding Preview",
        WebviewOptions {
            enable_scripts: true,
            local_resource_roots: vec![],
        },
    );
    assert_eq!(panel_state.id, "cat_coding_panel");

    let sample_html = r#"
        <h1>Cat Coding</h1>
        <p>Realtime collaborative preview powered by RustCode.</p>
        <hr>
        <code>console.log("Hello from inside webview");</code>
        <button>Ping Extension Host</button>
    "#;
    ext_host_mgr.set_html("cat_coding_panel", sample_html);

    // 2. Open Webview in Workbench Editor Area
    let mut wb = WorkbenchView::new();
    wb.editor_area.open_webview(
        "cat_coding_panel",
        "Cat Coding Preview",
        "catCoding",
        sample_html,
        WebviewOptions {
            enable_scripts: true,
            local_resource_roots: vec![],
        },
        Some(ext_host_mgr.clone()),
    );

    assert!(wb.editor_area.webviews.contains_key("cat_coding_panel"));
    let webview_view = wb.editor_area.webviews.get_mut("cat_coding_panel").unwrap();
    assert_eq!(webview_view.nodes.len(), 5);
    assert!(matches!(webview_view.nodes[0].kind, WebviewElementKind::Heading1(_)));
    assert!(matches!(webview_view.nodes[4].kind, WebviewElementKind::Button { .. }));

    // 3. Render Webview into scene graph
    let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
    let font = FontMetrics::default_monospace();
    let theme_styles = rustcode_platform::render::WorkbenchThemeStyles::dark_default();

    let mut scene = rustcode_platform::render::Scene::new();
    webview_view.render(bounds, &theme_styles, &font, &mut scene);
    assert!(!scene.quads().is_empty());

    // 4. Test Bidirectional MessagePort / RPC bridge
    // Ext-host -> Webview
    webview_view.receive_message_from_host(json!({
        "type": "update",
        "cat": "fluffy",
        "energy": 100
    }));
    assert_eq!(webview_view.inbound_messages.len(), 1);
    assert_eq!(webview_view.inbound_messages[0]["cat"], "fluffy");

    // Webview -> Ext-host (clicking the button)
    let targets = webview_view.button_targets.read().clone();
    assert_eq!(targets.len(), 1);
    let btn_rect = targets[0].0;

    let click = InputEvent::MouseDown(MouseEvent::new(
        Point::new(btn_rect.min_x() + 4.0, btn_rect.min_y() + 4.0),
        MouseButton::Left,
    ));
    let res = webview_view.handle_event(&click, bounds);
    assert_eq!(res, EventResult::Consumed);

    // Verify outbound message arrived at Webview
    assert_eq!(webview_view.outbound_messages.len(), 1);
    assert_eq!(webview_view.outbound_messages[0]["text"], "Ping Extension Host");
}



