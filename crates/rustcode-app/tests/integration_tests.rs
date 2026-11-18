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
    let content_rect = window.view.editor_area.content_rect(editor_rect);

    // Mouse click at line 1, column 4 inside editor
    let font = FontMetrics::default_monospace();
    let gutter_w = window
        .view
        .editor_area
        .active_editor()
        .unwrap()
        .gutter_width(&font);
    let click_x = content_rect.min_x() + gutter_w + 8.0 + 8.0 * font.char_width;
    let click_y = content_rect.min_y() + 1.0 * font.line_height + 5.0;

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
    assert_eq!(editor.primary_position(), rustcode_editor::cursor::Position::new(0, 2));

    // Delete backwards removes emoji completely
    editor.delete_backwards();
    assert!(editor.text().starts_with("fn main()"));
    assert_eq!(editor.primary_position(), rustcode_editor::cursor::Position::new(0, 0));

    // Undo restores emoji and cursor position
    assert!(editor.undo());
    assert!(editor.text().starts_with("🚀fn main()"));
    assert_eq!(editor.primary_position(), rustcode_editor::cursor::Position::new(0, 2));

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
    let content_rect = window.view.editor_area.content_rect(editor_regions.editor_area);
    let target_x = content_rect.min_x() + gutter_w + 8.0 + 25.0 * font.char_width;
    let target_y = content_rect.min_y() + 1.5 * font.line_height;

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

    // Test Ctrl+Shift+F5 (Restart) keyboard shortcut
    let restart_key = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::F5,
        Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        },
    ));
    let res_restart = wb.handle_event(&restart_key, bounds);
    assert_eq!(res_restart, EventResult::Consumed);
    assert_eq!(
        wb.last_action,
        Some(WorkbenchAction::DebugToolbar(DebugToolbarAction::Restart))
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

    // 5. Test full real-world document HTML with doctype, head, and nested body
    let full_doc = r#"
        <!DOCTYPE html>
        <html>
        <head><title>Full Doc</title><style>body { margin: 0; }</style></head>
        <body>
            <h1>Real Webview</h1>
            <div>
                <p>Nested paragraph with &amp; entity.</p>
                <button id="real_btn">Click</button>
            </div>
        </body>
        </html>
    "#;
    webview_view.set_html(full_doc);
    assert_eq!(webview_view.nodes.len(), 3);
    assert_eq!(
        webview_view.nodes[0].kind,
        WebviewElementKind::Heading1("Real Webview".to_string())
    );
    assert_eq!(
        webview_view.nodes[1].kind,
        WebviewElementKind::Paragraph("Nested paragraph with & entity.".to_string())
    );
}

#[tokio::test]
async fn test_end_to_end_formatting_and_code_actions_integration() {
    use rustcode_app::RustCodeContext;
    use rustcode_ext_host::provider::{ProviderRegistration, ProviderRegistry, ProviderType};
    use rustcode_ext_host::rpc::RpcBridge;
    use rustcode_protocols::lsp::LspClientManager;
    use std::sync::Arc;

    let app = RustCodeContext::new();
    assert!(app.commands.has_command("editor.action.formatDocument"));
    assert!(app.commands.has_command("editor.action.quickFix"));

    let reg = Arc::new(ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp_mgr = Arc::new(LspClientManager::new());

    reg.register(ProviderRegistration {
        id: "rust-fmt".to_string(),
        provider_type: ProviderType::DocumentFormatting,
        language_selector: Some("rust".to_string()),
    });

    reg.register(ProviderRegistration {
        id: "rust-actions".to_string(),
        provider_type: ProviderType::CodeAction,
        language_selector: Some("rust".to_string()),
    });

    let bridge = Arc::new(rustcode_ext_host::LanguageFeatureBridge::new(
        reg,
        rpc.clone(),
        lsp_mgr,
    ));

    let mut window = app.create_window().with_language_bridge(bridge);
    window.open_file(
        "file:///test_e2e.rs",
        "test_e2e.rs",
        "fn main(){\nlet a=1;\nlet b=2;\n}\n",
        false,
    );

    // 1. Formatting
    let rpc_fmt = rpc.clone();
    tokio::spawn(async move {
        rpc_fmt.handle_incoming_response(
            1,
            Some(serde_json::json!([
                {
                    "range": {
                        "start": { "line": 0, "character": 9 },
                        "end": { "line": 0, "character": 10 }
                    },
                    "newText": " {"
                },
                {
                    "range": {
                        "start": { "line": 1, "character": 0 },
                        "end": { "line": 1, "character": 8 }
                    },
                    "newText": "    let a = 1;"
                },
                {
                    "range": {
                        "start": { "line": 2, "character": 0 },
                        "end": { "line": 2, "character": 8 }
                    },
                    "newText": "    let b = 2;"
                }
            ])),
            None,
        );
    });

    assert!(window.format_active_document().await);
    assert_eq!(
        window.view.editor_area.editors.get("file:///test_e2e.rs").unwrap().text(),
        "fn main() {\n    let a = 1;\n    let b = 2;\n}\n"
    );

    // Verify undo restores original
    assert!(window.view.editor_area.editors.get_mut("file:///test_e2e.rs").unwrap().undo());
    assert_eq!(
        window.view.editor_area.editors.get("file:///test_e2e.rs").unwrap().text(),
        "fn main(){\nlet a=1;\nlet b=2;\n}\n"
    );

    // Redo restores formatted
    assert!(window.view.editor_area.editors.get_mut("file:///test_e2e.rs").unwrap().redo());
    assert_eq!(
        window.view.editor_area.editors.get("file:///test_e2e.rs").unwrap().text(),
        "fn main() {\n    let a = 1;\n    let b = 2;\n}\n"
    );

    // 2. Code Action
    let rpc_action = rpc.clone();
    tokio::spawn(async move {
        rpc_action.handle_incoming_response(
            2,
            Some(serde_json::json!([
                {
                    "title": "Rename variable a to val_a",
                    "kind": "quickfix",
                    "edit": {
                        "changes": {
                            "file:///test_e2e.rs": [
                                {
                                    "range": {
                                        "start": { "line": 1, "character": 8 },
                                        "end": { "line": 1, "character": 9 }
                                    },
                                    "newText": "val_a"
                                }
                            ]
                        }
                    }
                }
            ])),
            None,
        );
    });

    assert!(window.show_quick_fixes().await);
    assert!(window.view.is_command_palette_open);
    assert_eq!(window.pending_code_actions.len(), 1);

    assert!(window.apply_code_action_by_index(0));
    assert_eq!(
        window.view.editor_area.editors.get("file:///test_e2e.rs").unwrap().text(),
        "fn main() {\n    let val_a = 1;\n    let b = 2;\n}\n"
    );
}

#[test]
fn test_end_to_end_visual_settings_ui_and_config_sync() {
    use rustcode_workbench::SettingsHitTarget;

    // 1. Bootstrap RustCodeContext (preloads linux.json with Ctrl+,) and create window
    let app = RustCodeContext::new();
    let mut window = app.create_window();
    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
    let mut surface = MockRenderSurface::new(1024.0, 768.0);

    // 2. Dispatch Ctrl+, to open Visual Settings UI tab
    let ctrl_comma = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char(','),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    let res = window.dispatch_event(&ctrl_comma, bounds);
    assert_eq!(res, EventResult::Consumed);
    assert_eq!(
        window
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .unwrap()
            .uri,
        "rustcode://settings"
    );

    // 3. Render window and search for "formatOnSave"
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("Settings"));
    assert!(scene.contains_text("Font Size"));

    let search_rect = window
        .view
        .editor_area
        .settings_views
        .get("rustcode://settings")
        .unwrap()
        .hit_targets
        .read()
        .iter()
        .find_map(|t| match t {
            SettingsHitTarget::SearchBar(r) => Some(*r),
            _ => None,
        })
        .unwrap();

    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(search_rect.min_x() + 6.0, search_rect.min_y() + 6.0),
            MouseButton::Left,
        )),
        bounds,
    );
    for ch in "formatOnSave".chars() {
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
            bounds,
        );
    }

    // 4. Render filtered list and click bool toggle -> updates shared app.config
    window.render(&mut surface);
    let toggle_rect = window
        .view
        .editor_area
        .settings_views
        .get("rustcode://settings")
        .unwrap()
        .hit_targets
        .read()
        .iter()
        .find_map(|t| match t {
            SettingsHitTarget::ToggleBool { key, rect } if key == "editor.formatOnSave" => {
                Some(*rect)
            }
            _ => None,
        })
        .unwrap();

    assert_eq!(app.config.get::<bool>("editor.formatOnSave"), Some(false));
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(toggle_rect.min_x() + 5.0, toggle_rect.min_y() + 5.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert_eq!(app.config.get::<bool>("editor.formatOnSave"), Some(true));

    // 5. Render modified item -> click Reset -> restores default in app.config
    let scene_mod = window.render(&mut surface);
    assert!(scene_mod.contains_text("Reset"));

    let reset_rect = window
        .view
        .editor_area
        .settings_views
        .get("rustcode://settings")
        .unwrap()
        .hit_targets
        .read()
        .iter()
        .find_map(|t| match t {
            SettingsHitTarget::ResetButton { key, rect } if key == "editor.formatOnSave" => {
                Some(*rect)
            }
            _ => None,
        })
        .unwrap();

    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(reset_rect.min_x() + 5.0, reset_rect.min_y() + 5.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert_eq!(app.config.get::<bool>("editor.formatOnSave"), Some(false));
}

#[test]
fn test_end_to_end_notebook_editor_kernel_and_save() {
    use rustcode_platform::fs::FileSystemService;
    use rustcode_workbench::{NotebookClickTarget, NotebookDocument};

    let tmp_dir = std::env::temp_dir().join(format!(
        "rustcode_e2e_nb_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let nb_path = tmp_dir.join("e2e_notebook.ipynb");
    let nb_uri = format!("file://{}", nb_path.display());
    let nb_content = r#"{
        "cells": [
            {
                "id": "cell-1",
                "cell_type": "code",
                "execution_count": null,
                "metadata": {},
                "outputs": [],
                "source": ["let base = 20\n", "print(\"ok\")"]
            }
        ],
        "metadata": { "language_info": { "name": "python" } },
        "nbformat": 4,
        "nbformat_minor": 5
    }"#;
    std::fs::write(&nb_path, nb_content).unwrap();

    // 1. Bootstrap app and window
    let app = RustCodeContext::new();
    assert!(app
        .commands
        .has_command("workbench.action.files.newUntitledNotebook"));
    assert!(app.commands.has_command("notebook.execute"));
    assert!(app.commands.has_command("notebook.cell.execute"));

    let mut window = app.create_window();
    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
    let mut surface = MockRenderSurface::new(1024.0, 768.0);

    // 2. Open .ipynb file via window.open_file
    window.open_file(&nb_uri, "e2e_notebook.ipynb", nb_content, false);
    assert!(window.view.editor_area.notebooks.contains_key(&nb_uri));
    assert!(!window.view.editor_area.editors.contains_key(&nb_uri));

    // 3. Render and click [+ Code] button to add a second code cell
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("e2e_notebook.ipynb"));
    assert!(scene.contains_text("let base = 20"));

    let add_code_rect = window
        .view
        .editor_area
        .active_notebook()
        .unwrap()
        .hit_targets
        .read()
        .iter()
        .find_map(|(r, t)| match t {
            NotebookClickTarget::AddCode => Some(*r),
            _ => None,
        })
        .unwrap();

    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(add_code_rect.min_x() + 5.0, add_code_rect.min_y() + 5.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert_eq!(
        window
            .view
            .editor_area
            .active_notebook()
            .unwrap()
            .document
            .cells
            .len(),
        2
    );

    // Type `base + 22` into the newly added cell and run all via [Run All]
    for (k, txt) in [
        (KeyCode::Char('b'), Some("b")),
        (KeyCode::Char('a'), Some("a")),
        (KeyCode::Char('s'), Some("s")),
        (KeyCode::Char('e'), Some("e")),
        (KeyCode::Space, None),
        (KeyCode::Char('+'), Some("+")),
        (KeyCode::Space, None),
        (KeyCode::Char('2'), Some("2")),
        (KeyCode::Char('2'), Some("2")),
    ] {
        let mut ev = KeyEvent::new(k, Modifiers::none());
        ev.text = txt.map(|s| s.to_string());
        window.dispatch_event(&InputEvent::KeyDown(ev), bounds);
    }

    let run_all_rect = window
        .view
        .editor_area
        .active_notebook()
        .unwrap()
        .hit_targets
        .read()
        .iter()
        .find_map(|(r, t)| match t {
            NotebookClickTarget::RunAll => Some(*r),
            _ => None,
        })
        .unwrap();

    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(run_all_rect.min_x() + 5.0, run_all_rect.min_y() + 5.0),
            MouseButton::Left,
        )),
        bounds,
    );

    let scene2 = window.render(&mut surface);
    assert!(scene2.contains_text("e2e_notebook.ipynb \u{25cf}"));
    assert!(scene2.contains_text("ok"));
    assert!(scene2.contains_text("42"));

    // 4. Save active notebook and reload from disk to verify persistence
    let fs = FileSystemService::new();
    window.view.editor_area.save_active_file(&fs).unwrap();
    assert!(
        !window
            .view
            .editor_area
            .active_notebook()
            .unwrap()
            .document
            .is_dirty
    );

    let reloaded_raw = std::fs::read_to_string(&nb_path).unwrap();
    let reloaded = NotebookDocument::from_ipynb_json(&nb_uri, &reloaded_raw).unwrap();
    assert_eq!(reloaded.cells.len(), 2);
    assert_eq!(reloaded.cells[0].outputs[0].plain_text(), "ok\n");
    assert_eq!(reloaded.cells[1].execution_count, Some(2));
    assert_eq!(reloaded.cells[1].outputs[0].plain_text(), "42");

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[tokio::test]
async fn test_end_to_end_extensions_marketplace_sidebar_ui() {
    use rustcode_ext_host::marketplace::{
        ExtensionInstaller, MarketplaceClient, MockHttpTransport,
    };
    use rustcode_workbench::ExtensionsClickTarget;
    use std::io::Write;
    use std::sync::Arc;
    use zip::write::SimpleFileOptions;

    let tmp_dir = std::env::temp_dir().join(format!(
        "rustcode_e2e_ext_ui_{}",
        uuid::Uuid::new_v4().simple()
    ));

    // Build synthetic .vsix with a contributed command & theme
    let manifest_json = r#"{
        "name": "e2e-ext",
        "publisher": "e2epub",
        "version": "1.0.0",
        "displayName": "E2E Ext",
        "description": "End-to-end marketplace extension",
        "contributes": {
            "commands": [
                {
                    "command": "e2eExt.hello",
                    "title": "E2E Hello",
                    "category": "E2E"
                }
            ],
            "themes": [
                {
                    "label": "E2E Dark",
                    "uiTheme": "vs-dark",
                    "path": "./themes/e2e-dark.json"
                }
            ]
        }
    }"#;
    let theme_json = r##"{
        "name": "E2E Dark",
        "type": "dark",
        "colors": { "editor.background": "#121314" }
    }"##;
    let mut vsix_buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut vsix_buf);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("extension/package.json", opts).unwrap();
        zip.write_all(manifest_json.as_bytes()).unwrap();
        zip.start_file("extension/themes/e2e-dark.json", opts)
            .unwrap();
        zip.write_all(theme_json.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    let vsix_bytes = vsix_buf.into_inner();

    let transport = Arc::new(MockHttpTransport::new());
    let dl_url = "https://open-vsx.org/api/e2epub/e2e-ext/1.0.0/file/e2e-ext.vsix";
    transport.set_response(
        "https://open-vsx.org/api/-/search?query=e2e&size=20&offset=0",
        serde_json::to_vec(&json!({
            "totalSize": 1,
            "extensions": [{
                "namespace": "e2epub",
                "name": "e2e-ext",
                "version": "1.0.0",
                "displayName": "E2E Ext",
                "description": "End-to-end marketplace extension",
                "downloadUrl": dl_url
            }]
        }))
        .unwrap(),
    );
    transport.set_response(
        "https://open-vsx.org/api/e2epub/e2e-ext/1.0.0",
        serde_json::to_vec(&json!({
            "namespace": "e2epub",
            "name": "e2e-ext",
            "version": "1.0.0",
            "displayName": "E2E Ext",
            "downloadUrl": dl_url
        }))
        .unwrap(),
    );
    transport.set_response(dl_url, vsix_bytes);

    let client = Arc::new(MarketplaceClient::new(
        "https://open-vsx.org/api",
        transport,
    ));
    let installer = Arc::new(ExtensionInstaller::new(client, &tmp_dir));

    // 1. Bootstrap RustCodeContext & WorkbenchWindow
    let app = RustCodeContext::new();
    assert!(app.commands.has_command("workbench.view.extensions"));
    assert!(app
        .commands
        .has_command("workbench.extensions.action.showInstalledExtensions"));

    let mut window = app.create_window();
    window.view.extensions_view.set_installer(installer.clone());

    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
    let mut surface = MockRenderSurface::new(1024.0, 768.0);

    // 2. Dispatch Ctrl+Shift+X -> opens Extensions sidebar
    let ctrl_shift_x = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('x'),
        Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(
        window.dispatch_event(&ctrl_shift_x, bounds),
        EventResult::Consumed
    );
    assert_eq!(window.view.layout.active_sidebar_view, "extensions");
    assert_eq!(window.view.activity_bar.model.active_item_index, 4);

    // 3. Render -> verifies EXTENSIONS rendered, NOT EXPLORER
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("EXTENSIONS"));
    assert!(!scene.contains_text("EXPLORER"));

    // Verify "workbench.view.extensions" alias also renders EXTENSIONS, NOT EXPLORER
    window.view.layout.active_sidebar_view = "workbench.view.extensions".to_string();
    let scene_alias = window.render(&mut surface);
    assert!(scene_alias.contains_text("EXTENSIONS"));
    assert!(!scene_alias.contains_text("EXPLORER"));

    // 4. Focus search input, type "e2e", press Enter
    let search_rect = window
        .view
        .extensions_view
        .click_targets
        .read()
        .iter()
        .find_map(|(r, t)| match t {
            ExtensionsClickTarget::SearchInput => Some(*r),
            _ => None,
        })
        .unwrap();
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(search_rect.min_x() + 6.0, search_rect.min_y() + 6.0),
            MouseButton::Left,
        )),
        bounds,
    );
    for ch in "e2e".chars() {
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
            bounds,
        );
    }
    window.dispatch_event(
        &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
        bounds,
    );

    let scene_search = window.render(&mut surface);
    assert!(scene_search.contains_text("E2E Ext"));
    assert!(scene_search.contains_text("Install"));

    // 5. Click [Install] -> unpacks into tmp_dir and registers contributed command/theme
    let install_rect = window
        .view
        .extensions_view
        .click_targets
        .read()
        .iter()
        .find_map(|(r, t)| match t {
            ExtensionsClickTarget::InstallBtn { name, .. } if name == "e2e-ext" => Some(*r),
            _ => None,
        })
        .unwrap();
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(install_rect.min_x() + 6.0, install_rect.min_y() + 6.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert_eq!(installer.list_installed().len(), 1);
    assert!(window.commands.has_command("e2eExt.hello"));
    assert_eq!(window.theme.current().name, "E2E Dark");

    let scene_inst = window.render(&mut surface);
    assert!(scene_inst.contains_text("INSTALLED (1)"));
    assert!(scene_inst.contains_text("Disable"));

    // 6. Click [Disable] -> persists .disabled.json & disposes contributed command
    let disable_rect = window
        .view
        .extensions_view
        .click_targets
        .read()
        .iter()
        .find_map(|(r, t)| match t {
            ExtensionsClickTarget::DisableBtn(id) if id == "e2epub.e2e-ext" => Some(*r),
            _ => None,
        })
        .unwrap();
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(disable_rect.min_x() + 6.0, disable_rect.min_y() + 6.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert!(!installer.is_enabled("e2epub.e2e-ext"));
    assert!(tmp_dir.join(".disabled.json").exists());
    assert!(!window.commands.has_command("e2eExt.hello"));

    // 7. Re-render and click [Enable] -> re-enables and re-registers contributed command
    let scene_dis = window.render(&mut surface);
    assert!(scene_dis.contains_text("Enable"));
    let enable_rect = window
        .view
        .extensions_view
        .click_targets
        .read()
        .iter()
        .find_map(|(r, t)| match t {
            ExtensionsClickTarget::EnableBtn(id) if id == "e2epub.e2e-ext" => Some(*r),
            _ => None,
        })
        .unwrap();
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(enable_rect.min_x() + 6.0, enable_rect.min_y() + 6.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert!(installer.is_enabled("e2epub.e2e-ext"));
    assert!(window.commands.has_command("e2eExt.hello"));

    // 8. Re-render and click [Uninstall] -> removes folder & disposes command
    window.render(&mut surface);
    let uninstall_rect = window
        .view
        .extensions_view
        .click_targets
        .read()
        .iter()
        .find_map(|(r, t)| match t {
            ExtensionsClickTarget::UninstallBtn(id) if id == "e2epub.e2e-ext" => Some(*r),
            _ => None,
        })
        .unwrap();
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(uninstall_rect.min_x() + 6.0, uninstall_rect.min_y() + 6.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert!(installer.list_installed().is_empty());
    assert!(!window.commands.has_command("e2eExt.hello"));

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_end_to_end_breadcrumbs_minimap_and_settings_live_toggles() {
    use rustcode_editor::Position;
    use rustcode_platform::render::MouseMoveEvent;
    use rustcode_workbench::{SettingsHitTarget, Workspace};

    let app = RustCodeContext::new();
    let mut window = app.create_window();
    window.view.workspace = Workspace::from_folder(PathBuf::from("/workspace"));

    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let font = FontMetrics::default_monospace();

    let mut src = String::from(
        "pub struct Engine {\n    pub state: usize,\n}\n\nimpl Engine {\n    pub fn tick(&mut self) {\n        let v = 1;\n    }\n}\n",
    );
    for i in 0..160 {
        src.push_str(&format!("// extra line {}\n", i));
    }

    window.open_file("file:///workspace/src/lib.rs", "lib.rs", &src, false);
    window
        .view
        .editor_area
        .active_editor_mut()
        .unwrap()
        .set_primary_position(Position::new(6, 12));
    window.sync_configuration();

    let scene = window.render(&mut surface);
    assert!(scene.contains_text("workspace"));
    assert!(scene.contains_text("src"));
    assert!(scene.contains_text("lib.rs"));
    assert!(scene.contains_text("Engine"));
    assert!(scene.contains_text("tick"));

    let regions = window.view.compute_regions(bounds);
    let bc_rect = window
        .view
        .editor_area
        .breadcrumbs_rect(regions.editor_area)
        .unwrap();
    let content_rect = window
        .view
        .editor_area
        .content_rect(regions.editor_area);
    let mm_rect = window
        .view
        .editor_area
        .active_editor()
        .unwrap()
        .minimap_rect(content_rect)
        .unwrap();
    assert!(
        scene.quads().iter().any(|q| q.bounds.min_x() >= mm_rect.min_x() - 0.1),
        "Expected minimap quads in rendered scene"
    );

    // Click `Engine` symbol breadcrumb -> jumps cursor to line 0
    let item_rects = window
        .view
        .editor_area
        .breadcrumbs
        .compute_item_rects(bc_rect, &font);
    let (_, engine_rect) = item_rects
        .into_iter()
        .find(|(idx, _)| window.view.editor_area.breadcrumbs.items[*idx].label == "Engine")
        .unwrap();
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(engine_rect.min_x() + 4.0, engine_rect.min_y() + 10.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert_eq!(
        window
            .view
            .editor_area
            .active_editor()
            .unwrap()
            .primary_position()
            .line,
        0
    );

    // Click & drag minimap, then release outside editor_area
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(mm_rect.min_x() + 20.0, mm_rect.max_y() - 30.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert!(
        window
            .view
            .editor_area
            .active_editor()
            .unwrap()
            .minimap_dragging
    );
    assert!(
        window
            .view
            .editor_area
            .active_editor()
            .unwrap()
            .scroll_top
            > 20.0
    );
    window.dispatch_event(
        &InputEvent::MouseMove(MouseMoveEvent {
            position: Point::new(mm_rect.min_x() + 20.0, mm_rect.min_y() + 40.0),
        }),
        bounds,
    );
    window.dispatch_event(
        &InputEvent::MouseUp(MouseEvent::new(
            Point::new(50.0, 50.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert!(
        !window
            .view
            .editor_area
            .active_editor()
            .unwrap()
            .minimap_dragging
    );

    // Press Ctrl+, to open Settings UI and toggle editor.minimap.enabled and breadcrumbs.enabled
    let ctrl_comma = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char(','),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(window.dispatch_event(&ctrl_comma, bounds), EventResult::Consumed);
    assert_eq!(
        window
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .unwrap()
            .uri,
        "rustcode://settings"
    );

    window.render(&mut surface);
    let search_rect = window
        .view
        .editor_area
        .settings_views
        .get("rustcode://settings")
        .unwrap()
        .hit_targets
        .read()
        .iter()
        .find_map(|t| match t {
            SettingsHitTarget::SearchBar(r) => Some(*r),
            _ => None,
        })
        .unwrap();
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(search_rect.min_x() + 6.0, search_rect.min_y() + 6.0),
            MouseButton::Left,
        )),
        bounds,
    );
    for ch in "minimap".chars() {
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
            bounds,
        );
    }
    window.render(&mut surface);
    let minimap_toggle = window
        .view
        .editor_area
        .settings_views
        .get("rustcode://settings")
        .unwrap()
        .hit_targets
        .read()
        .iter()
        .find_map(|t| match t {
            SettingsHitTarget::ToggleBool { key, rect } if key == "editor.minimap.enabled" => {
                Some(*rect)
            }
            _ => None,
        })
        .unwrap();
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(minimap_toggle.min_x() + 5.0, minimap_toggle.min_y() + 5.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert_eq!(
        app.config.get::<bool>("editor.minimap.enabled"),
        Some(false)
    );
    assert!(!window.view.editor_area.minimap_enabled);

    // Search for "breadcrumbs" and toggle breadcrumbs.enabled -> false
    {
        let settings_view = window
            .view
            .editor_area
            .settings_views
            .get_mut("rustcode://settings")
            .unwrap();
        settings_view.state.apply_search("breadcrumbs");
        settings_view.scroll_top = 0.0;
    }
    window.render(&mut surface);
    let bc_toggle = window
        .view
        .editor_area
        .settings_views
        .get("rustcode://settings")
        .unwrap()
        .hit_targets
        .read()
        .iter()
        .find_map(|t| match t {
            SettingsHitTarget::ToggleBool { key, rect } if key == "breadcrumbs.enabled" => {
                Some(*rect)
            }
            _ => None,
        })
        .unwrap();
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(bc_toggle.min_x() + 5.0, bc_toggle.min_y() + 5.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert_eq!(app.config.get::<bool>("breadcrumbs.enabled"), Some(false));
    assert!(!window.view.editor_area.breadcrumbs_enabled);

    // Switch back to lib.rs tab and verify content_rect & minimap_rect reflect disabled toggles
    window.dispatch_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(regions.editor_area.min_x() + 20.0, regions.editor_area.min_y() + 15.0),
            MouseButton::Left,
        )),
        bounds,
    );
    assert_eq!(
        window
            .view
            .editor_area
            .content_rect(regions.editor_area)
            .min_y(),
        regions.editor_area.min_y() + 35.0
    );
    assert!(
        window
            .view
            .editor_area
            .active_editor()
            .unwrap()
            .minimap_rect(window.view.editor_area.content_rect(regions.editor_area))
            .is_none()
    );
    let scene_after = window.render(&mut surface);
    assert!(!scene_after.contains_text("Engine > tick"));
}

#[test]
fn test_end_to_end_accessibility_tree_workbench_and_overlays() {
    use rustcode_platform::accessibility::AccessibleRole;
    use rustcode_workbench::Workspace;

    let app = RustCodeContext::new();
    let mut window = app.create_window();
    window.view.workspace = Workspace::from_folder(PathBuf::from("/workspace"));
    window.open_file(
        "file:///workspace/src/main.rs",
        "main.rs",
        "pub fn main() {\n    let x = 1;\n}\n",
        false,
    );

    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
    let tree = window.accessibility_tree(bounds);
    assert_eq!(tree.root.role, AccessibleRole::Window);
    assert!(!tree.find_by_role(AccessibleRole::Toolbar).is_empty());
    assert!(!tree.find_by_role(AccessibleRole::Sidebar).is_empty());
    assert!(!tree.find_by_role(AccessibleRole::Tree).is_empty());
    assert!(!tree.find_by_role(AccessibleRole::TabList).is_empty());
    assert!(
        tree.find_by_role(AccessibleRole::Tab)
            .iter()
            .any(|n| n.state.selected && n.name == "main.rs")
    );
    assert!(!tree.find_by_role(AccessibleRole::Breadcrumb).is_empty());
    assert!(!tree.find_by_role(AccessibleRole::BreadcrumbItem).is_empty());
    assert!(!tree.find_by_role(AccessibleRole::TextEditor).is_empty());
    assert!(!tree.find_by_role(AccessibleRole::ScrollBar).is_empty());
    assert!(!tree.find_by_role(AccessibleRole::StatusBar).is_empty());
    assert_eq!(tree.focused_node().unwrap().role, AccessibleRole::TextEditor);

    // Hit-test inside breadcrumbs & editor
    let regions = window.view.compute_regions(bounds);
    let bc_hit = tree
        .hit_test(Point::new(
            regions.editor_area.min_x() + 18.0,
            regions.editor_area.min_y() + 45.0,
        ))
        .unwrap();
    assert!(matches!(
        bc_hit.role,
        AccessibleRole::Breadcrumb | AccessibleRole::BreadcrumbItem
    ));

    let content_rect = window.view.editor_area.content_rect(regions.editor_area);
    let ed_hit = tree
        .hit_test(Point::new(
            content_rect.min_x() + 100.0,
            content_rect.min_y() + 20.0,
        ))
        .unwrap();
    assert_eq!(ed_hit.role, AccessibleRole::TextEditor);

    // Open Command Palette via Ctrl+Shift+P -> Dialog + TextInput + List/ListItem present and focused
    let ctrl_shift_p = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('p'),
        Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(
        window.dispatch_event(&ctrl_shift_p, bounds),
        EventResult::Consumed
    );
    let pal_tree = window.accessibility_tree(bounds);
    assert!(!pal_tree.find_by_role(AccessibleRole::Dialog).is_empty());
    assert!(!pal_tree.find_by_role(AccessibleRole::List).is_empty());
    assert!(!pal_tree.find_by_role(AccessibleRole::ListItem).is_empty());
    assert_eq!(
        pal_tree.focused_node().unwrap().id,
        "commandPalette.input"
    );
}

#[test]
fn test_end_to_end_notebook_breadcrumbs_and_accessibility() {
    use rustcode_platform::accessibility::AccessibleRole;
    use rustcode_workbench::Workspace;

    let app = RustCodeContext::new();
    let mut window = app.create_window();
    window.view.workspace = Workspace::from_folder(PathBuf::from("/workspace"));

    let ipynb_json = r#"{
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {},
        "cells": [
            {
                "cell_type": "code",
                "metadata": {},
                "execution_count": null,
                "source": ["x = 10 + 5"],
                "outputs": []
            }
        ]
    }"#;
    window.open_file(
        "file:///workspace/notebooks/demo.ipynb",
        "demo.ipynb",
        ipynb_json,
        false,
    );

    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("notebooks"));
    assert!(scene.contains_text("demo.ipynb"));

    let tree = window.accessibility_tree(Rect::from_xywh(0.0, 0.0, 1024.0, 768.0));
    assert!(
        tree.find_by_role(AccessibleRole::Tab)
            .iter()
            .any(|n| n.state.selected && n.name == "demo.ipynb")
    );
    let bc_labels: Vec<&str> = tree
        .find_by_role(AccessibleRole::BreadcrumbItem)
        .iter()
        .map(|n| n.name.as_str())
        .collect();
    assert_eq!(bc_labels, vec!["workspace", "notebooks", "demo.ipynb"]);
    assert!(tree.find_by_id("notebook.cells").is_some());
    assert!(!tree.find_by_role(AccessibleRole::StatusBar).is_empty());
}

#[test]
fn test_end_to_end_extensions_sidebar_in_accessibility_tree() {
    use rustcode_platform::accessibility::AccessibleRole;

    let app = RustCodeContext::new();
    let mut window = app.create_window();
    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
    let mut surface = MockRenderSurface::new(1024.0, 768.0);

    let ctrl_shift_x = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('x'),
        Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(
        window.dispatch_event(&ctrl_shift_x, bounds),
        EventResult::Consumed
    );
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("EXTENSIONS"));

    let tree = window.accessibility_tree(bounds);
    let sidebar = tree
        .find_by_role(AccessibleRole::Sidebar)
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(sidebar.name, "Extensions");
    assert!(sidebar.state.focused);
    assert!(tree.find_by_id("workbench.sidebar.extensions.search").is_some());
    assert!(tree.find_by_id("workbench.sidebar.extensions.marketplace").is_some());
    assert_eq!(tree.focused_node().unwrap().id, "workbench.sidebar");
}

#[test]
fn test_end_to_end_interactive_terminal_panel_and_commands() {
    use rustcode_platform::accessibility::AccessibleRole;
    use rustcode_platform::render::{MouseButton, MouseEvent, ScrollEvent};
    use rustcode_workbench::FocusedRegion;

    let app = RustCodeContext::new();
    assert!(app.commands.has_command("workbench.action.problems.focus"));
    assert!(app
        .commands
        .has_command("workbench.action.terminal.toggleTerminal"));
    assert!(app.commands.has_command("workbench.action.terminal.new"));
    assert!(app.commands.has_command("workbench.action.terminal.split"));
    assert!(app.commands.has_command("workbench.action.terminal.kill"));
    assert!(app.commands.has_command("workbench.action.terminal.focus"));
    assert!(app
        .commands
        .has_command("workbench.action.output.toggleOutput"));
    assert!(app
        .commands
        .has_command("workbench.action.output.clearOutput"));

    let mut window = app.create_window();
    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
    let mut surface = MockRenderSurface::new(1024.0, 768.0);

    // Toggle terminal via Ctrl+`
    let ctrl_backtick = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('`'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(
        window.dispatch_event(&ctrl_backtick, bounds),
        EventResult::Consumed
    );
    assert!(window.view.layout.panel_visible);
    assert_eq!(window.view.layout.active_panel_tab, "terminal");
    assert_eq!(window.view.focused_region, FocusedRegion::Panel);
    assert_eq!(
        window.context_keys.get("panelFocus"),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        window.context_keys.get("terminalFocus"),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        window.context_keys.get("editorTextFocus"),
        Some(serde_json::json!(false))
    );

    // Type and execute shell command interactively
    for ch in "echo rustcode_m1".chars() {
        assert_eq!(
            window.dispatch_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            ),
            EventResult::Consumed
        );
    }
    assert_eq!(
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        ),
        EventResult::Consumed
    );
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("rustcode_m1"));

    // Create new terminal tab (Ctrl+Shift+`) and split pane (Ctrl+Shift+5, gated by terminalFocus)
    let ctrl_shift_backtick = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('`'),
        Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(
        window.dispatch_event(&ctrl_shift_backtick, bounds),
        EventResult::Consumed
    );
    assert_eq!(window.view.terminal_panel.manager.tabs().len(), 2);

    let ctrl_shift_5 = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('5'),
        Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(
        window.dispatch_event(&ctrl_shift_5, bounds),
        EventResult::Consumed
    );
    assert_eq!(
        window
            .view
            .terminal_panel
            .manager
            .active_tab_terminals()
            .len(),
        2
    );

    // Switch back to first tab by clicking its tab rect
    let _ = window.render(&mut surface);
    let first_tab_rect = window
        .view
        .terminal_panel
        .click_targets
        .read()
        .iter()
        .find(|(_, t)| *t == rustcode_workbench::TerminalClickTarget::SelectTab(0))
        .unwrap()
        .0;
    let tab0_pt = Point::new(
        first_tab_rect.min_x() + 10.0,
        first_tab_rect.min_y() + first_tab_rect.height() / 2.0,
    );
    assert_eq!(
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(tab0_pt, MouseButton::Left)),
            bounds,
        ),
        EventResult::Consumed
    );
    assert_eq!(window.view.terminal_panel.manager.active_tab_index(), Some(0));

    // Verify accessibility_tree focused node is terminal.pane.*
    let tree = window.accessibility_tree(bounds);
    assert!(tree.find_by_id("workbench.panel.terminal").is_some());
    let focused = tree.focused_node().unwrap();
    assert!(focused.id.starts_with("terminal.pane."));
    assert_eq!(focused.role, AccessibleRole::TextInput);

    // ScrollWheel inside regions.panel is consumed
    let regions = window.view.compute_regions(bounds);
    let scroll_pt = Point::new(regions.panel.min_x() + 80.0, regions.panel.min_y() + 60.0);
    assert_eq!(
        window.dispatch_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(scroll_pt, 0.0, 200.0)),
            bounds,
        ),
        EventResult::Consumed
    );

    // Toggle terminal closed -> restores Editor focus
    assert_eq!(
        window.dispatch_event(&ctrl_backtick, bounds),
        EventResult::Consumed
    );
    assert!(!window.view.layout.panel_visible);
    assert_eq!(window.view.focused_region, FocusedRegion::Editor);
    assert_eq!(
        window.context_keys.get("panelFocus"),
        Some(serde_json::json!(false))
    );
    assert_eq!(
        window.context_keys.get("terminalFocus"),
        Some(serde_json::json!(false))
    );
}

#[tokio::test]
async fn test_end_to_end_output_channel_ext_host_sync_and_panel_controls() {
    use rustcode_ext_host::provider::{LanguageFeatureBridge, ProviderRegistry};
    use rustcode_ext_host::rpc::RpcBridge;
    use rustcode_platform::render::{MouseButton, MouseEvent, ScrollEvent};
    use rustcode_protocols::lsp::LspClientManager;
    use rustcode_workbench::OutputClickTarget;
    use std::sync::Arc;

    let app = RustCodeContext::new();
    let registry = Arc::new(ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp));

    let mut window = app.create_window().with_language_bridge(bridge.clone());
    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
    let mut surface = MockRenderSurface::new(1024.0, 768.0);

    bridge
        .handle_rpc_message(
            "window.createOutputChannel",
            &serde_json::json!({ "id": "out_build", "name": "RustCode Build", "languageId": "log" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "window.createOutputChannel",
            &serde_json::json!({ "id": "out_ext", "name": "Extension Host" }),
        )
        .await
        .unwrap();

    for i in 0..35 {
        bridge
            .handle_rpc_message(
                "window.appendOutputChannel",
                &serde_json::json!({
                    "id": "out_build",
                    "value": format!("build_step_{:02}\n", i)
                }),
            )
            .await
            .unwrap();
    }
    bridge
        .handle_rpc_message(
            "window.appendOutputChannel",
            &serde_json::json!({ "id": "out_ext", "value": "ext_host_ready\n" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "window.showOutputChannel",
            &serde_json::json!({ "id": "out_build", "preserveFocus": false }),
        )
        .await
        .unwrap();

    window.sync_output_channels();
    assert!(window.view.layout.panel_visible);
    assert_eq!(window.view.layout.active_panel_tab, "output");
    assert_eq!(
        window.context_keys.get("panelFocus"),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        window.context_keys.get("terminalFocus"),
        Some(serde_json::json!(false))
    );

    let scene = window.render(&mut surface);
    assert!(scene.contains_text("RustCode Build"));
    assert!(scene.contains_text("build_step_34"));

    // ScrollWheel up inside regions.panel disables auto_scroll
    let regions = window.view.compute_regions(bounds);
    let scroll_pt = Point::new(regions.panel.min_x() + 100.0, regions.panel.min_y() + 80.0);
    assert_eq!(
        window.dispatch_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(scroll_pt, 0.0, 2000.0)),
            bounds,
        ),
        EventResult::Consumed
    );
    assert!(!window.view.output_panel.auto_scroll);

    // Click channel dropdown toggle then select "Extension Host"
    let selector_rect = window
        .view
        .output_panel
        .click_targets
        .read()
        .iter()
        .find(|(_, t)| *t == OutputClickTarget::ChannelDropdownToggle)
        .unwrap()
        .0;
    let sel_pt = Point::new(
        selector_rect.min_x() + selector_rect.width() / 2.0,
        selector_rect.min_y() + selector_rect.height() / 2.0,
    );
    assert_eq!(
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(sel_pt, MouseButton::Left)),
            bounds,
        ),
        EventResult::Consumed
    );
    assert!(window.view.output_panel.dropdown_open);

    let _ = window.render(&mut surface);
    let opt_rect = window
        .view
        .output_panel
        .click_targets
        .read()
        .iter()
        .find(|(_, t)| *t == OutputClickTarget::ChannelOption("out_ext".to_string()))
        .unwrap()
        .0;
    let opt_pt = Point::new(
        opt_rect.min_x() + opt_rect.width() / 2.0,
        opt_rect.min_y() + opt_rect.height() / 2.0,
    );
    assert_eq!(
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(opt_pt, MouseButton::Left)),
            bounds,
        ),
        EventResult::Consumed
    );
    assert_eq!(
        window.view.output_panel.active_channel_id.as_deref(),
        Some("out_ext")
    );
    let ext_scene = window.render(&mut surface);
    assert!(ext_scene.contains_text("ext_host_ready"));

    // Click Clear button -> clears both OutputPanelView and ExtHostOutputManager
    let clear_rect = window
        .view
        .output_panel
        .click_targets
        .read()
        .iter()
        .find(|(_, t)| *t == OutputClickTarget::ClearButton)
        .unwrap()
        .0;
    let clear_pt = Point::new(
        clear_rect.min_x() + clear_rect.width() / 2.0,
        clear_rect.min_y() + clear_rect.height() / 2.0,
    );
    assert_eq!(
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(clear_pt, MouseButton::Left)),
            bounds,
        ),
        EventResult::Consumed
    );
    assert!(window.view.output_panel.display_lines().is_empty());
    assert!(bridge
        .output_manager
        .get_channel("out_ext")
        .unwrap()
        .lines()
        .is_empty());

    // Re-syncing does not resurrect cleared lines
    window.sync_output_channels();
    assert!(window.view.output_panel.display_lines().is_empty());

    let tree = window.accessibility_tree(bounds);
    assert!(tree.find_by_id("workbench.panel.output").is_some());
    assert!(tree.find_by_id("output.channelSelector").is_some());
    assert!(tree.find_by_id("output.clear").is_some());
    assert!(tree.find_by_id("output.autoScroll").is_some());
    assert!(tree.find_by_id("output.lines").is_some());
}

#[test]
fn test_e2e_multi_cursor_ctrl_d_ctrl_alt_arrows_and_escape() {
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{
        KeyCode, KeyEvent, MockRenderSurface, MouseButton, MouseEvent, Point, Rect,
    };

    let app = RustCodeContext::new();
    assert!(app.commands.has_command("editor.action.addSelectionToNextFindMatch"));
    assert!(app.commands.has_command("editor.action.insertCursorAbove"));
    assert!(app.commands.has_command("editor.action.insertCursorBelow"));

    let mut window = app.create_window();
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);

    window.open_file(
        "file:///workspace/multi.rs",
        "multi.rs",
        "let item = 1;\nlet item = 2;\nlet item = 3;\n",
        false,
    );
    let _ = window.render(&mut surface);

    // Click on "item" on line 0 to ensure Editor focus and editorTextFocus context
    let font = FontMetrics::default_monospace();
    let regions = window.view.compute_regions(bounds);
    let content_rect = window.view.editor_area.content_rect(regions.editor_area);
    let gutter_w = window
        .view
        .editor_area
        .active_editor()
        .unwrap()
        .gutter_width(&font);
    let click = InputEvent::MouseDown(MouseEvent::new(
        Point::new(
            content_rect.min_x() + gutter_w + 8.0 + 5.0 * font.char_width,
            content_rect.min_y() + 0.5 * font.line_height,
        ),
        MouseButton::Left,
    ));
    assert_eq!(window.dispatch_event(&click, bounds), EventResult::Consumed);

    // Ctrl+D x3 via KeybindingService -> selects all three "item" occurrences
    let ctrl_d = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('d'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(window.dispatch_event(&ctrl_d, bounds), EventResult::Consumed);
    assert_eq!(window.dispatch_event(&ctrl_d, bounds), EventResult::Consumed);
    assert_eq!(window.dispatch_event(&ctrl_d, bounds), EventResult::Consumed);
    assert_eq!(
        window
            .view
            .editor_area
            .active_editor()
            .unwrap()
            .cursor_state
            .selections
            .len(),
        3
    );

    // Type "val" to replace all three simultaneously
    for ch in ['v', 'a', 'l'] {
        let ev = InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none()));
        assert_eq!(window.dispatch_event(&ev, bounds), EventResult::Consumed);
    }
    assert_eq!(
        window.view.editor_area.active_editor().unwrap().text(),
        "let val = 1;\nlet val = 2;\nlet val = 3;\n"
    );

    // Escape collapses secondary cursors
    let esc = InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none()));
    assert_eq!(window.dispatch_event(&esc, bounds), EventResult::Consumed);
    assert_eq!(
        window
            .view
            .editor_area
            .active_editor()
            .unwrap()
            .cursor_state
            .selections
            .len(),
        1
    );

    // Ctrl+Alt+Up from line 2 adds cursors upward
    window
        .view
        .editor_area
        .active_editor_mut()
        .unwrap()
        .set_primary_position(rustcode_editor::Position::new(2, 4));
    let ctrl_alt_up = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::ArrowUp,
        Modifiers {
            ctrl: true,
            shift: false,
            alt: true,
            meta: false,
        },
    ));
    assert_eq!(
        window.dispatch_event(&ctrl_alt_up, bounds),
        EventResult::Consumed
    );
    assert_eq!(
        window
            .view
            .editor_area
            .active_editor()
            .unwrap()
            .cursor_state
            .selections
            .len(),
        2
    );
}

#[test]
fn test_e2e_code_folding_gutter_and_keybinding_chords() {
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{
        KeyCode, KeyEvent, MockRenderSurface, MouseButton, MouseEvent, Point, Rect,
    };

    let app = RustCodeContext::new();
    assert!(app.commands.has_command("editor.fold"));
    assert!(app.commands.has_command("editor.unfold"));
    assert!(app.commands.has_command("editor.foldAll"));
    assert!(app.commands.has_command("editor.unfoldAll"));

    let mut window = app.create_window();
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);

    window.open_file(
        "file:///workspace/fold.rs",
        "fold.rs",
        "fn first() {\n    let hidden_one = 1;\n}\nfn second() {\n    let hidden_two = 2;\n}\n",
        false,
    );
    let scene_open = window.render(&mut surface);
    assert!(scene_open.contains_text("hidden_one"));
    assert!(scene_open.contains_text("hidden_two"));

    // Click fold chevron on line 0
    let font = FontMetrics::default_monospace();
    let regions = window.view.compute_regions(bounds);
    let content_rect = window.view.editor_area.content_rect(regions.editor_area);
    let gutter_w = window
        .view
        .editor_area
        .active_editor()
        .unwrap()
        .gutter_width(&font);
    let fold_click = InputEvent::MouseDown(MouseEvent::new(
        Point::new(
            content_rect.min_x() + gutter_w - 8.0,
            content_rect.min_y() + 0.5 * font.line_height,
        ),
        MouseButton::Left,
    ));
    assert_eq!(
        window.dispatch_event(&fold_click, bounds),
        EventResult::Consumed
    );
    let scene_folded = window.render(&mut surface);
    assert!(!scene_folded.contains_text("hidden_one"));
    assert!(scene_folded.contains_text(" ..."));
    assert!(scene_folded.contains_text("hidden_two"));

    // Chord Ctrl+K Ctrl+0 folds all
    let ctrl_k = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('k'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    let ctrl_0 = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('0'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    window.dispatch_event(&ctrl_k, bounds);
    assert_eq!(window.dispatch_event(&ctrl_0, bounds), EventResult::Consumed);
    let scene_all_folded = window.render(&mut surface);
    assert!(!scene_all_folded.contains_text("hidden_one"));
    assert!(!scene_all_folded.contains_text("hidden_two"));

    // Chord Ctrl+K Ctrl+J unfolds all
    let ctrl_j = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('j'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    window.dispatch_event(&ctrl_k, bounds);
    assert_eq!(window.dispatch_event(&ctrl_j, bounds), EventResult::Consumed);
    let scene_unfolded = window.render(&mut surface);
    assert!(scene_unfolded.contains_text("hidden_one"));
    assert!(scene_unfolded.contains_text("hidden_two"));
}

#[test]
fn test_e2e_word_wrap_config_sync_and_scene_rendering() {
    use rustcode_platform::configuration::ConfigurationTarget;
    use rustcode_platform::render::{MockRenderSurface, Rect};
    use rustcode_workbench::settings_ui::default_settings_schema;
    use serde_json::json;

    // Verify Settings UI schema exposes both editor.wordWrap and editor.wordWrapColumn
    let schema = default_settings_schema();
    assert!(schema.iter().any(|s| s.key == "editor.wordWrap"));
    assert!(schema.iter().any(|s| s.key == "editor.wordWrapColumn"));

    let app = RustCodeContext::new();
    let mut window = app.create_window();
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);

    window.open_file(
        "file:///workspace/wrap.txt",
        "wrap.txt",
        "firstword secondword thirdword fourthword\nnextline",
        false,
    );

    app.config
        .update("editor.wordWrap", json!("wordWrapColumn"), ConfigurationTarget::User)
        .unwrap();
    app.config
        .update("editor.wordWrapColumn", json!(18), ConfigurationTarget::User)
        .unwrap();
    window.sync_configuration();

    let ed = window.view.editor_area.active_editor().unwrap();
    assert_eq!(ed.word_wrap, rustcode_editor::WordWrapMode::WordWrapColumn);
    assert_eq!(ed.word_wrap_column, 18);

    let scene = window.render(&mut surface);
    let font = FontMetrics::default_monospace();
    let regions = window.view.compute_regions(bounds);
    let content_rect = window.view.editor_area.content_rect(regions.editor_area);
    let gutter_w = window
        .view
        .editor_area
        .active_editor()
        .unwrap()
        .gutter_width(&font);

    // "nextline" must render below visual row 1 because line 0 wrapped across multiple rows
    let next_run = scene
        .glyph_runs()
        .into_iter()
        .find(|g| g.text == "nextline" && g.origin.x >= content_rect.min_x() + gutter_w)
        .expect("nextline must render in editor scene");
    assert!(next_run.origin.y >= content_rect.min_y() + font.line_height * 2.0);
}

#[test]
fn test_e2e_split_editor_grid_side_by_side_stacked_focus_and_close() {
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{
        KeyCode, KeyEvent, MockRenderSurface, MouseButton, MouseEvent, Point, Rect,
    };

    let app = RustCodeContext::new();
    let mut window = app.create_window();
    let mut surface = MockRenderSurface::new(1200.0, 800.0);
    let bounds = Rect::from_xywh(0.0, 0.0, 1200.0, 800.0);

    window.open_file("file:///workspace/a.rs", "a.rs", "fn alpha() {}\n", false);
    window.open_file(
        "file:///workspace/b.rs",
        "b.rs",
        "fn beta_one() {}\nfn beta_two() {}\n",
        false,
    );

    // 1. Ctrl+\ splits active editor horizontally (side-by-side)
    let ctrl_backslash = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('\\'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(
        window.dispatch_event(&ctrl_backslash, bounds),
        EventResult::Consumed
    );
    assert_eq!(window.view.editor_area.model.groups.len(), 2);
    let gid1 = window.view.editor_area.model.active_group_id();
    assert_ne!(gid1, 0);

    let regions = window.view.compute_regions(bounds);
    let rects = window
        .view
        .editor_area
        .model
        .compute_group_rects(regions.editor_area);
    assert_eq!(rects.len(), 2);
    assert!(rects[1].1.min_x() > rects[0].1.max_x());

    // 2. Shared TextDocument across group 0 and group 1 with isolated cursor & scroll
    window
        .view
        .editor_area
        .editor_for_group_mut(0, "file:///workspace/b.rs")
        .unwrap()
        .scroll_top = 1.0;
    window
        .view
        .editor_area
        .active_editor_mut()
        .unwrap()
        .set_primary_position(rustcode_editor::Position::new(1, 0));

    let type_z = InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('/'), Modifiers::none()));
    assert_eq!(window.dispatch_event(&type_z, bounds), EventResult::Consumed);

    assert_eq!(
        window
            .view
            .editor_area
            .editor_for_group(0, "file:///workspace/b.rs")
            .unwrap()
            .text(),
        window
            .view
            .editor_area
            .editor_for_group(gid1, "file:///workspace/b.rs")
            .unwrap()
            .text()
    );
    assert_eq!(
        window
            .view
            .editor_area
            .editor_for_group(0, "file:///workspace/b.rs")
            .unwrap()
            .primary_position()
            .line,
        0
    );
    assert_eq!(
        window
            .view
            .editor_area
            .editor_for_group(0, "file:///workspace/b.rs")
            .unwrap()
            .scroll_top,
        1.0
    );
    assert_eq!(
        window
            .view
            .editor_area
            .editor_for_group(gid1, "file:///workspace/b.rs")
            .unwrap()
            .scroll_top,
        0.0
    );

    // 3. Ctrl+K Ctrl+\ creates a vertical (stacked) split -> 3 groups
    let ctrl_k = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('k'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    window.dispatch_event(&ctrl_k, bounds);
    assert_eq!(
        window.dispatch_event(&ctrl_backslash, bounds),
        EventResult::Consumed
    );
    assert_eq!(window.view.editor_area.model.groups.len(), 3);
    let gid2 = window.view.editor_area.model.active_group_id();
    let rects3 = window
        .view
        .editor_area
        .model
        .compute_group_rects(regions.editor_area);
    let r1 = rects3.iter().find(|(id, _)| *id == gid1).unwrap().1;
    let r2 = rects3.iter().find(|(id, _)| *id == gid2).unwrap().1;
    assert!(r2.min_y() > r1.max_y());

    // Render multi-group scene cleanly
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("b.rs"));

    // 4. Focus switching via Ctrl+1, Ctrl+2, Ctrl+K Ctrl+Left, Ctrl+K Ctrl+Right, and mouse click
    let ctrl_1 = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('1'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(window.dispatch_event(&ctrl_1, bounds), EventResult::Consumed);
    assert_eq!(window.view.editor_area.model.active_group_id(), 0);

    let ctrl_2 = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('2'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(window.dispatch_event(&ctrl_2, bounds), EventResult::Consumed);
    assert_eq!(window.view.editor_area.model.active_group_id(), gid1);

    let ctrl_right = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::ArrowRight,
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    window.dispatch_event(&ctrl_k, bounds);
    assert_eq!(
        window.dispatch_event(&ctrl_right, bounds),
        EventResult::Consumed
    );
    assert_eq!(window.view.editor_area.model.active_group_id(), gid2);

    let ctrl_left = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::ArrowLeft,
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    window.dispatch_event(&ctrl_k, bounds);
    assert_eq!(
        window.dispatch_event(&ctrl_left, bounds),
        EventResult::Consumed
    );
    assert_eq!(window.view.editor_area.model.active_group_id(), gid1);

    // Click inside group 0 switches focus back to 0
    let r0 = rects3.iter().find(|(id, _)| *id == 0).unwrap().1;
    let click_g0 = InputEvent::MouseDown(MouseEvent::new(
        Point::new(r0.min_x() + 50.0, r0.min_y() + 80.0),
        MouseButton::Left,
    ));
    assert_eq!(window.dispatch_event(&click_g0, bounds), EventResult::Consumed);
    assert_eq!(window.view.editor_area.model.active_group_id(), 0);

    // Accessibility tree exposes all groups and only marks active group focused
    let tree = window.accessibility_tree(bounds);
    assert!(tree.find_by_id("workbench.editorGroup.0").is_some());
    assert!(tree
        .find_by_id(&format!("workbench.editorGroup.{}", gid1))
        .is_some());
    assert!(tree
        .find_by_id(&format!("workbench.editorGroup.{}", gid2))
        .is_some());
    assert!(tree
        .find_by_id("workbench.editorGroup.0")
        .unwrap()
        .children
        .iter()
        .any(|c| c.state.focused));
    assert!(!tree
        .find_by_id(&format!("workbench.editorGroup.{}", gid1))
        .unwrap()
        .children
        .iter()
        .any(|c| c.state.focused));

    // 5. Focus group 2 and close via Ctrl+K W (workbench.action.closeEditorsInGroup)
    assert_eq!(window.dispatch_event(&ctrl_2, bounds), EventResult::Consumed);
    window.dispatch_event(&ctrl_k, bounds);
    assert_eq!(
        window.dispatch_event(&ctrl_right, bounds),
        EventResult::Consumed
    );
    assert_eq!(window.view.editor_area.model.active_group_id(), gid2);

    let plain_w = InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('w'), Modifiers::none()));
    window.dispatch_event(&ctrl_k, bounds);
    assert_eq!(window.dispatch_event(&plain_w, bounds), EventResult::Consumed);
    assert_eq!(window.view.editor_area.model.groups.len(), 2);
}

#[test]
fn test_e2e_custom_editor_in_split_pane_with_phase6_features() {
    use rustcode_ext_host::manifest::{CustomEditorContribution, CustomEditorSelector};
    use rustcode_ext_host::provider::ProviderRegistry;
    use rustcode_ext_host::rpc::RpcBridge;
    use rustcode_ext_host::webview::WebviewOptions;
    use rustcode_ext_host::LanguageFeatureBridge;
    use rustcode_platform::configuration::ConfigurationTarget;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{
        KeyCode, KeyEvent, MockRenderSurface, MouseButton, MouseEvent, Point, Rect,
    };
    use rustcode_protocols::lsp::LspClientManager;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let app = RustCodeContext::new();
    app.config
        .update("editor.wordWrap", json!("on"), ConfigurationTarget::User)
        .unwrap();

    let registry = Arc::new(ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp));
    bridge.custom_editor_manager.register_provider(
        "custom.hexViewer",
        WebviewOptions {
            enable_scripts: true,
            local_resource_roots: vec![],
        },
        true,
    );

    let mut window = app.create_window().with_language_bridge(bridge.clone());
    window.register_custom_editor_contribution(CustomEditorContribution {
        view_type: "custom.hexViewer".to_string(),
        display_name: "Hex Viewer".to_string(),
        selector: vec![CustomEditorSelector {
            filename_pattern: "*.bin".to_string(),
        }],
        priority: Some("default".to_string()),
    });

    let mut surface = MockRenderSurface::new(1200.0, 800.0);
    let bounds = Rect::from_xywh(0.0, 0.0, 1200.0, 800.0);

    // Group 0: regular .rs file with M3 folding & word-wrap
    window.open_file(
        "file:///workspace/main.rs",
        "main.rs",
        "fn folded_fn() {\n    let inside = 42;\n}\n",
        false,
    );
    window
        .view
        .editor_area
        .active_editor_mut()
        .unwrap()
        .fold_at_cursor();

    // Split editor horizontally -> Group 1
    let ctrl_backslash = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('\\'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    assert_eq!(
        window.dispatch_event(&ctrl_backslash, bounds),
        EventResult::Consumed
    );
    assert_eq!(window.view.editor_area.model.groups.len(), 2);

    // Open .bin custom editor in Group 1
    let bin_uri = "file:///workspace/firmware.bin";
    window.open_file(bin_uri, "firmware.bin", "\x00\x01", false);
    let session = bridge
        .custom_editor_manager
        .find_session_by_uri(bin_uri, Some("custom.hexViewer"))
        .unwrap();

    let clicks_received = Arc::new(AtomicUsize::new(0));
    let clicks_clone = clicks_received.clone();
    bridge
        .webview_manager
        .on_did_receive_message(&session.panel_id, move |msg| {
            if msg["command"] == "click" && msg["text"] == "Inspect Hex" {
                clicks_clone.fetch_add(1, Ordering::SeqCst);
            }
        });
    bridge.webview_manager.set_html(
        &session.panel_id,
        "<h1>Hex Dump</h1><p>0000: 7f 45 4c 46</p><button>Inspect Hex</button>",
    );
    window.sync_webviews();

    // Render concurrently in split grid
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("folded_fn"));
    assert!(scene.contains_text("Hex Dump"));
    assert!(scene.contains_text("0000: 7f 45 4c 46"));
    assert!(scene.contains_text("Inspect Hex"));

    // Click button inside split custom editor pane -> delivers outbound message to ExtHostWebviewManager
    let btn_rect = window
        .view
        .editor_area
        .webviews
        .get(bin_uri)
        .unwrap()
        .button_targets
        .read()[0]
        .0;
    let click_btn = InputEvent::MouseDown(MouseEvent::new(
        Point::new(btn_rect.min_x() + 6.0, btn_rect.min_y() + 6.0),
        MouseButton::Left,
    ));
    assert_eq!(
        window.dispatch_event(&click_btn, bounds),
        EventResult::Consumed
    );
    assert_eq!(clicks_received.load(Ordering::SeqCst), 1);

    let tree = window.accessibility_tree(bounds);
    assert!(tree.find_by_id("workbench.editor.text.file:///workspace/main.rs").is_some());
    assert!(tree
        .find_by_id("workbench.editor.webview.file:///workspace/firmware.bin")
        .is_some());
}




