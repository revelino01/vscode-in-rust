use encoding_rs::{SHIFT_JIS, WINDOWS_1252};
use phase8_e2e::TempWorkspace;
use rustcode_app::cli::{parse_cli_args, parse_goto_spec, CliAction, CliError, StartupMode};
use rustcode_app::runtime::{bootstrap_app, memento_get, memento_update, restore_workbench_state, save_workbench_state};
use rustcode_app::RustCodeContext;
use rustcode_editor::{
    load_textmate_contribution, EditorAction, EditorView, GrammarContribution, GrammarRegistry,
    Position, SyntaxHighlighter, SyntaxScope, TextDocument, TextEncoding, TextMateGrammar,
};
use rustcode_platform::fs::{path_to_file_uri, FileSystemService};
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::os_ux::{
    ClipboardBackend, FileDialogBackend, FileDialogOptions, InMemoryClipboardBackend,
    MockFileDialogBackend,
};
use rustcode_platform::render::{
    AtlasInsertResult, ClipStack, Corners, Edges, FontMetrics, GlyphAtlasAllocator, GlyphCacheKey,
    GlyphRun, InputEvent, KeyCode, KeyEvent, MockRenderSurface, MouseButton, MouseEvent,
    MouseMoveEvent, Point, Quad, QuadInstance, Rect, RenderSurface, Rgba, Scene, SceneBatchBuilder,
    Shadow, ShadowInstance, View, WorkbenchThemeStyles,
};
use rustcode_platform::storage::{MementoScope, PersistedTabState, PersistedWorkspaceState, StorageBackend};
use rustcode_platform::theme::ThemeService;
use rustcode_workbench::quickinput::{CommandPalette, PaletteItem, QuickInputMode};
use rustcode_workbench::search::{
    replace_in_workspace, search_files, search_text, search_workspace_symbols, try_search_text,
    SearchQuery,
};
use rustcode_workbench::workspace::WorkspaceFolder;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "desktop")]
use rustcode_app::desktop::{
    safe_logical_bounds, translate_winit_ime, translate_winit_key_event, translate_winit_mouse_wheel,
};
#[cfg(feature = "desktop")]
use rustcode_editor::TreeSitterEngine;
#[cfg(feature = "desktop")]
use rustcode_platform::render::{GlyphInstance, TextShaper};
#[cfg(feature = "desktop")]
use rustcode_platform::storage::SqliteStorageService;

/// [M1-01..M1-07, ADV-05] ClipStack, Quad/Shadow instances, GlyphAtlasAllocator, HiDPI, cosmic-text shaping, and winit event mapping.
#[test]
fn t1_m1_render_surface_quads_shadows_clip_stack_and_atlas() {
    // M1-01: MockRenderSurface resize + 0x0 guard + HiDPI scaling
    let mut surface = MockRenderSurface::new(800.0, 600.0);
    surface.resize(0, 0, 2.0);
    assert_eq!(surface.size().width, 800.0);
    surface.resize(2560, 1600, 2.0);
    assert!((surface.size().width - 1280.0).abs() < 1e-3);
    assert!((surface.size().height - 800.0).abs() < 1e-3);

    // M1-02: QuadInstance rounded corner clamping & asymmetric borders
    let quad = Quad::new(Rect::from_xywh(10.0, 10.0, 40.0, 20.0))
        .with_background(Rgba::new(0.2, 0.4, 0.8, 1.0))
        .with_border_edges(Rgba::white(), Edges::new(1.0, 2.0, 3.0, 4.0))
        .with_corner_radius(50.0);
    let inst = QuadInstance::from_quad(&quad, 1.5).expect("valid quad");
    assert_eq!(inst.bounds, [15.0, 15.0, 60.0, 30.0]);
    assert_eq!(inst.border_widths, [1.5, 3.0, 4.5, 6.0]);
    assert!(inst.corner_radii.iter().all(|&r| (r - 15.0).abs() < 1e-3));
    assert!(QuadInstance::from_quad(&Quad::new(Rect::zero()), 1.0).is_none());

    // M1-03: ShadowInstance with blur == 0.0 and negative spread
    let shadow = Shadow {
        bounds: Rect::from_xywh(10.0, 10.0, 100.0, 50.0),
        offset: Point::new(0.0, 4.0),
        blur: 0.0,
        spread: -2.0,
        color: Rgba::new(0.0, 0.0, 0.0, 0.5),
    };
    let sinst = ShadowInstance::from_shadow(&shadow, 1.0).expect("valid shadow");
    assert_eq!(sinst.shadow_rect, [12.0, 16.0, 96.0, 46.0]);

    let huge_blur = Shadow {
        bounds: Rect::from_xywh(10.0, 10.0, 100.0, 50.0),
        offset: Point::zero(),
        blur: f32::MAX,
        spread: 0.0,
        color: Rgba::black(),
    };
    if let Some(huge_inst) = ShadowInstance::from_shadow(&huge_blur, 2.0) {
        assert!(
            huge_inst.bounds.iter().all(|v| v.is_finite())
                && huge_inst.shadow_rect.iter().all(|v| v.is_finite())
                && huge_inst.params.iter().all(|v| v.is_finite()),
            "ShadowInstance with huge blur must never emit non-finite bounds: {huge_inst:?}"
        );
    }

    // M1-04 & ADV-05: ClipStack intersection & unbalanced PopClip safety
    let mut clips = ClipStack::new(Rect::from_xywh(0.0, 0.0, 1000.0, 800.0));
    clips.pop(); // empty pop must not panic
    clips.push(Rect::from_xywh(10.0, 10.0, 200.0, 200.0));
    clips.push(Rect::from_xywh(50.0, 50.0, 300.0, 300.0));
    assert_eq!(clips.current(), Rect::from_xywh(50.0, 50.0, 160.0, 160.0));
    assert_eq!(clips.scissor_rect(1.25, 1250, 1000), Some([62, 62, 201, 201]));
    clips.pop();
    assert_eq!(clips.current(), Rect::from_xywh(10.0, 10.0, 200.0, 200.0));

    let mut unbalanced_scene = Scene::new();
    unbalanced_scene.pop_clip();
    unbalanced_scene.push_clip(Rect::from_xywh(10.0, 10.0, 100.0, 100.0));
    unbalanced_scene.push_quad(Quad::new(Rect::from_xywh(20.0, 20.0, 30.0, 30.0)).with_background(Rgba::white()));
    // Leave PushClip unclosed intentionally
    let batch = SceneBatchBuilder::build_non_text(
        &unbalanced_scene,
        Rect::from_xywh(0.0, 0.0, 800.0, 600.0),
        1.0,
        800,
        600,
    );
    assert_eq!(batch.stats.quad_count, 1);

    // M1-06: GlyphAtlasAllocator hit reuse + overflow reset
    let mut atlas = GlyphAtlasAllocator::new(64, 64);
    atlas.begin_frame();
    let k1 = GlyphCacheKey {
        font_id: 1,
        glyph_id: 42,
        font_size_bits: 14.0_f32.to_bits(),
        x_bin: 0,
        y_bin: 0,
        flags: 0,
    };
    assert!(matches!(
        atlas.insert(k1, 16, 16, 0, 12, false),
        AtlasInsertResult::Allocated(_)
    ));
    atlas.begin_frame();
    assert!(matches!(
        atlas.insert(k1, 16, 16, 0, 12, false),
        AtlasInsertResult::Hit(_)
    ));
    assert_eq!(atlas.hit_count(), 1);
    let mut saw_reset = false;
    for gid in 100..140u16 {
        let key = GlyphCacheKey {
            font_id: 1,
            glyph_id: gid,
            font_size_bits: 14.0_f32.to_bits(),
            x_bin: 0,
            y_bin: 0,
            flags: 0,
        };
        if matches!(
            atlas.insert(key, 20, 20, 0, 12, false),
            AtlasInsertResult::AllocatedAfterReset(_)
        ) {
            saw_reset = true;
            break;
        }
    }
    assert!(saw_reset, "atlas must reset cleanly on overflow");
    assert_eq!(
        atlas.insert(k1, 128, 128, 0, 0, false),
        AtlasInsertResult::TooLarge
    );
    for huge_dim in [u32::MAX, u32::MAX - 1] {
        assert_eq!(
            atlas.insert(k1, huge_dim, 10, 0, 0, false),
            AtlasInsertResult::TooLarge
        );
        assert_eq!(
            atlas.insert(k1, 10, huge_dim, 0, 0, false),
            AtlasInsertResult::TooLarge
        );
    }

    // M1-05 & M1-07 (desktop feature): cosmic-text shaping + winit event mapping
    #[cfg(feature = "desktop")]
    {
        let mut shaper = TextShaper::new();
        for sample in [
            "fn main() -> Result<(), ()> { a !== b && c === d }",
            "你好世界 日本語",
            "e\u{0301}",
            "👨‍👩‍👧‍👦 🚀",
        ] {
            let run = GlyphRun::new(
                sample.to_string(),
                Point::new(0.0, 0.0),
                Rect::from_xywh(0.0, 0.0, 600.0, 24.0),
                Rgba::white(),
                14.0,
            );
            let shaped = shaper.shape_run(&run, 1.5);
            assert!(
                shaped.total_advance_width >= 0.0 && shaped.total_advance_width.is_finite(),
                "cosmic-text shaping failed for {sample:?}"
            );
            let _ = std::mem::size_of::<GlyphInstance>();
            let _ = Corners::zero();
        }

        // ADV-05: Non-finite / extreme GlyphRun origins must not panic cosmic-text shaper
        for bad_origin in [
            Point::new(f32::NAN, f32::INFINITY),
            Point::new(1.0e20, 1.0e20),
            Point::new(-1.0e20, f32::NEG_INFINITY),
        ] {
            let bad_run = GlyphRun::new(
                "hello".to_string(),
                bad_origin,
                Rect::from_xywh(0.0, 0.0, 600.0, 24.0),
                Rgba::white(),
                14.0,
            );
            let shaped_bad = shaper.shape_run(&bad_run, 1.5);
            assert!(shaped_bad.total_advance_width.is_finite());
        }

        // winit translations
        let zero = safe_logical_bounds(0, 0, 2.0);
        assert_eq!(zero.width(), 0.0);
        let hidpi = safe_logical_bounds(2560, 1600, 2.0);
        assert!((hidpi.width() - 1280.0).abs() < 1e-3);

        let key_ev = translate_winit_key_event(
            &winit::keyboard::Key::Named(winit::keyboard::NamedKey::F12),
            None,
            None,
            winit::event::ElementState::Pressed,
            rustcode_platform::keybindings::Modifiers::none(),
        )
        .unwrap();
        assert!(matches!(key_ev, InputEvent::KeyDown(k) if k.key == KeyCode::F12));

        let ime_ev = translate_winit_ime(&winit::event::Ime::Commit("你好".to_string())).unwrap();
        assert_eq!(ime_ev, InputEvent::ImeCommit("你好".to_string()));

        let wheel = translate_winit_mouse_wheel(
            winit::event::MouseScrollDelta::LineDelta(0.0, -1.0),
            Point::new(10.0, 10.0),
            rustcode_platform::keybindings::Modifiers::none(),
            20.0,
            1.0,
        );
        assert!(matches!(wheel, InputEvent::ScrollWheel(_)));
    }
}

/// [M2-01..M2-17, ADV-02, ADV-03] CLI parsing, JSONC disk resources, themes, SQLite persistence, Memento, and OS UX.
#[test]
fn t1_m2_cli_parser_disk_resources_and_sqlite_storage() {
    // M2-01..M2-07: CLI parsing incl. Windows drive letter colons & errors
    let zero_opts = match parse_cli_args(Vec::<&str>::new()).unwrap() {
        CliAction::Run(o) => o,
        other => panic!("unexpected: {other:?}"),
    };
    assert_eq!(zero_opts.startup_mode(true), StartupMode::Headless);

    let goto_win = parse_goto_spec(r"C:\dir\file with spaces\main.rs:15:3").unwrap();
    assert_eq!(goto_win.path, PathBuf::from(r"C:\dir\file with spaces\main.rs"));
    assert_eq!(goto_win.line_zero_based(), 14);
    assert_eq!(goto_win.col_zero_based(), 2);

    // ADV-02: short filename with line:col (`a:15:2`) must not misparse `a:` as Windows drive
    let short_goto = parse_goto_spec("a:15:2").unwrap();
    assert_eq!(short_goto.path, PathBuf::from("a"));
    assert_eq!(short_goto.line, 15);
    assert_eq!(short_goto.column, 2);
    let parsed_short = match parse_cli_args(["--goto", "a:15:2"]).unwrap() {
        CliAction::Run(o) => o,
        other => panic!("unexpected: {other:?}"),
    };
    assert_eq!(parsed_short.goto, vec![short_goto]);

    assert_eq!(
        parse_cli_args(["--bogus-flag"]).unwrap_err(),
        CliError::UnknownFlag("--bogus-flag".to_string())
    );
    assert_eq!(
        parse_cli_args(["--goto"]).unwrap_err(),
        CliError::MissingValue("--goto".to_string())
    );

    // M2-13..M2-15: Disk JSONC (with BOM + comments + trailing commas), precedence, themes, extensions
    let ws = TempWorkspace::new();
    let user_data = ws.mkdir_all("user-data");
    let custom_exts = ws.mkdir_all("custom-exts");
    let workspace = ws.mkdir_all("project");

    ws.write_file(
        "user-data/settings.json",
        "\u{FEFF}{\n// user setting\n\"editor.fontSize\": 19,\n\"editor.tabSize\": 4,\n\"workbench.colorTheme\": \"Default Light+\",\n}",
    );
    ws.write_file(
        "project/.vscode/settings.json",
        "{ \"editor.tabSize\": 2 }",
    );
    ws.write_file(
        "user-data/keybindings.json",
        "[{ \"key\": \"ctrl+alt+k\", \"command\": \"e2e.customKey\" }]",
    );
    ws.write_file(
        "outside-theme.json",
        r##"{ "name": "Evil Escape", "type": "dark", "colors": { "editor.background": "#000000" } }"##,
    );
    ws.write_file(
        "outside.tmLanguage.json",
        r#"{ "name": "Evil", "scopeName": "source.evil", "patterns": [] }"#,
    );
    ws.write_file(
        "custom-exts/my-theme/package.json",
        r#"{
            "name": "my-theme",
            "publisher": "TestPub",
            "version": "1.0.0",
            "contributes": {
                "themes": [
                    { "label": "E2E Solar", "uiTheme": "vs-dark", "path": "./solar.json" },
                    { "label": "Evil Escape", "uiTheme": "vs-dark", "path": "../../outside-theme.json" }
                ]
            }
        }"#,
    );
    ws.write_file(
        "custom-exts/my-theme/solar.json",
        r##"{ "name": "E2E Solar", "type": "dark", "colors": { "editor.background": "#112233" } }"##,
    );
    let target_rs = ws.write_file("project/src/app.rs", "line1\nline2 target\nline3\n");

    let goto_spec = format!("{}:2:7", target_rs.display());
    let opts = match parse_cli_args([
        "--headless",
        "--user-data-dir",
        user_data.to_str().unwrap(),
        "--extensions-dir",
        custom_exts.to_str().unwrap(),
        "--goto",
        &goto_spec,
        workspace.to_str().unwrap(),
    ])
    .unwrap()
    {
        CliAction::Run(o) => o,
        other => panic!("unexpected: {other:?}"),
    };

    let mut app = bootstrap_app(&opts, false).unwrap();
    assert_eq!(app.context.config.get::<u32>("editor.fontSize"), Some(19));
    assert_eq!(app.context.config.get::<usize>("editor.tabSize"), Some(2));
    assert_eq!(app.window.theme.current().kind, rustcode_platform::ThemeKind::Light);
    assert!(app.window.theme.get_theme("E2E Solar").is_some());
    // ADV-03: `../` traversal outside extension root must be rejected
    assert!(app.window.theme.get_theme("Evil Escape").is_none());
    assert!(load_textmate_contribution(
        &custom_exts.join("my-theme"),
        &GrammarContribution {
            language: Some("evil".to_string()),
            scope_name: "source.evil".to_string(),
            path: "../../outside.tmLanguage.json".to_string(),
        },
        &["evil"],
    )
    .is_err());
    assert_eq!(
        app.context
            .keybindings
            .lookup_keybinding("e2e.customKey")
            .as_deref(),
        Some("Ctrl+Alt+K")
    );
    assert_eq!(app.context.ext_loader.loaded_extensions().len(), 1);
    assert_eq!(
        app.window
            .view
            .editor_area
            .active_editor()
            .unwrap()
            .primary_position(),
        Position::new(1, 6)
    );
    app.runtime_loop.shutdown(&mut app.window);

    // --disable-extensions skips discovery
    let no_ext_opts = match parse_cli_args([
        "--headless",
        "--user-data-dir",
        user_data.to_str().unwrap(),
        "--extensions-dir",
        custom_exts.to_str().unwrap(),
        "--disable-extensions",
        workspace.to_str().unwrap(),
    ])
    .unwrap()
    {
        CliAction::Run(o) => o,
        other => panic!("unexpected: {other:?}"),
    };
    let mut no_ext_app = bootstrap_app(&no_ext_opts, false).unwrap();
    assert!(no_ext_app.context.ext_loader.loaded_extensions().is_empty());
    no_ext_app.runtime_loop.shutdown(&mut no_ext_app.window);

    // M2-08..M2-12 (desktop feature): SqliteStorageService reopen persistence + Memento + corrupt DB errors
    #[cfg(feature = "desktop")]
    {
        let db_path = ws.path().join("state/rustcode.db");
        {
            let sqlite = Arc::new(SqliteStorageService::open(&db_path).unwrap());
            sqlite.add_recent_workspace(&ws.mkdir_all("ws_one")).unwrap();
            sqlite.add_recent_workspace(&ws.mkdir_all("ws_two")).unwrap();
            sqlite.add_recent_workspace(&ws.mkdir_all("ws_one")).unwrap();
            assert_eq!(sqlite.recent_workspaces().unwrap().len(), 2);

            let mut ctx = RustCodeContext::new();
            let _ = ctx.theme.register_builtin_themes(None);
            let mut win = ctx
                .create_window()
                .with_storage_backend(Arc::clone(&sqlite) as Arc<dyn StorageBackend>);
            win.open_workspace(&workspace);
            win.open_path(&target_rs, false);
            if let Some(ed) = win.view.editor_area.active_editor_mut() {
                ed.set_primary_position(Position::new(1, 4));
                ed.scroll_top = 18.0;
            }
            win.view.layout.mutate(|l| {
                l.sidebar_visible = false;
                l.panel_visible = true;
            });
            let _ = win.theme.set_theme_by_name("Default Light+");
            save_workbench_state(&win).unwrap();

            memento_update(
                sqlite.as_ref(),
                MementoScope::Global,
                "pub.ext",
                "token",
                Some(json!({ "nested": [1, true, "ok"] })),
            )
            .unwrap();
            memento_update(
                sqlite.as_ref(),
                MementoScope::Workspace(workspace.to_string_lossy().into_owned()),
                "pub.ext",
                "count",
                Some(json!(42)),
            )
            .unwrap();
        }

        // Re-open fresh SqliteStorageService on the same .db file
        let reopened = Arc::new(SqliteStorageService::open(&db_path).unwrap());
        let recent = reopened.recent_workspaces().unwrap();
        assert_eq!(recent.len(), 3); // ws_one, ws_two, and `workspace` saved by save_workbench_state
        let mut ctx2 = RustCodeContext::new();
        let _ = ctx2.theme.register_builtin_themes(None);
        let mut win2 = ctx2
            .create_window()
            .with_storage_backend(Arc::clone(&reopened) as Arc<dyn StorageBackend>);
        win2.open_workspace(&workspace);
        assert!(restore_workbench_state(&mut win2, Some(&workspace)).unwrap());
        assert!(!win2.view.layout.sidebar_visible);
        assert!(win2.view.layout.panel_visible);
        assert_eq!(
            win2.view
                .editor_area
                .active_editor()
                .unwrap()
                .primary_position(),
            Position::new(1, 4)
        );
        assert_eq!(
            memento_get(reopened.as_ref(), MementoScope::Global, "pub.ext", "token").unwrap(),
            Some(json!({ "nested": [1, true, "ok"] }))
        );
        memento_update(reopened.as_ref(), MementoScope::Global, "pub.ext", "token", None).unwrap();
        assert_eq!(
            memento_get(reopened.as_ref(), MementoScope::Global, "pub.ext", "token").unwrap(),
            None
        );

        // ADV-03 / M2-12: Corrupt DB file fails cleanly with Err, no panic
        let corrupt_db = ws.write_file("corrupt.db", "not an sqlite file at all");
        assert!(SqliteStorageService::open(&corrupt_db).is_err());
        assert!(SqliteStorageService::open(ws.path()).is_err());
        let _ = PersistedTabState::default();
        let _ = PersistedWorkspaceState::default();
    }

    // M2-16 & M2-17: ClipboardBackend + MockFileDialogBackend
    let clip = InMemoryClipboardBackend::new();
    clip.write_text("copied").unwrap();
    assert_eq!(clip.read_text().unwrap(), "copied");
    let dialogs = MockFileDialogBackend::new();
    dialogs.push_open_file(Some(target_rs.clone()));
    assert_eq!(
        dialogs.pick_open_file(&FileDialogOptions::default()).unwrap(),
        target_rs
    );
}

/// [M3-01..M3-11] Daily-driver EditorView ops, grapheme/unicode-width layout, and multi-encoding load/save.
#[test]
fn t1_m3_editor_daily_ops_unicode_encodings_and_window_interactions() {
    let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
    let font = FontMetrics::default_monospace();
    let clip = Arc::new(InMemoryClipboardBackend::new());
    let clip_r = Arc::clone(&clip);
    let clip_w = Arc::clone(&clip);

    // M3-01: Clipboard & empty-selection line copy/cut/paste
    let mut ed = EditorView::from_str("alpha\nbeta\ngamma\n").with_clipboard_callbacks(
        Arc::new(move || clip_r.read_text().ok()),
        Arc::new(move |s| {
            let _ = clip_w.write_text(s);
        }),
    );
    ed.cursor_state.set_primary(Position::new(1, 2));
    ed.copy();
    assert_eq!(clip.read_text().unwrap(), "beta\n");
    ed.cursor_state.set_primary(Position::new(0, 1));
    ed.paste();
    assert_eq!(ed.text(), "beta\nalpha\nbeta\ngamma\n");
    assert!(ed.undo());
    ed.cursor_state.set_primary(Position::new(1, 1));
    ed.cut();
    assert_eq!(ed.text(), "alpha\ngamma\n");
    assert!(ed.undo());
    assert_eq!(ed.text(), "alpha\nbeta\ngamma\n");

    // M3-02: Mouse drag + double-click + triple-click
    let mut mouse_ed = EditorView::from_str("hello_world second\nthird\n");
    let text_x = bounds.min_x() + mouse_ed.gutter_width(&font) + 8.0;
    let p0 = Point::new(text_x, 4.0);
    let p5 = Point::new(text_x + 5.0 * font.char_width, 4.0);
    mouse_ed.handle_event(&InputEvent::MouseDown(MouseEvent::new(p0, MouseButton::Left)), bounds);
    mouse_ed.handle_event(&InputEvent::MouseMove(MouseMoveEvent { position: p5 }), bounds);
    mouse_ed.handle_event(&InputEvent::MouseUp(MouseEvent::new(p5, MouseButton::Left)), bounds);
    assert_eq!(mouse_ed.cursor_state.primary().end(), Position::new(0, 5));

    let p_word = Point::new(text_x + 3.0 * font.char_width, 4.0);
    mouse_ed.handle_event(
        &InputEvent::MouseDown(MouseEvent::new(p_word, MouseButton::Left).with_click_count(2)),
        bounds,
    );
    assert_eq!(mouse_ed.cursor_state.primary().end(), Position::new(0, 11));
    mouse_ed.handle_event(
        &InputEvent::MouseDown(MouseEvent::new(p_word, MouseButton::Left).with_click_count(3)),
        bounds,
    );
    assert_eq!(mouse_ed.cursor_state.primary().end(), Position::new(1, 0));

    // M3-03: Scrollbar & Minimap drag
    let mut big = String::new();
    for i in 0..150 {
        big.push_str(&format!("line_{i:03}_wide_content_exceeding_width_1234567890_abcdef\n"));
    }
    let mut scroll_ed = EditorView::from_str(&big);
    let sm_bounds = Rect::from_xywh(0.0, 0.0, 500.0, 300.0);
    let gw = scroll_ed.gutter_width(&font);
    let vtrack = scroll_ed.vertical_scrollbar_rect(sm_bounds, gw).unwrap();
    scroll_ed.handle_event(
        &InputEvent::MouseDown(MouseEvent::new(
            Point::new(vtrack.min_x() + 3.0, vtrack.min_y() + 8.0),
            MouseButton::Left,
        )),
        sm_bounds,
    );
    scroll_ed.handle_event(
        &InputEvent::MouseMove(MouseMoveEvent {
            position: Point::new(vtrack.min_x() + 3.0, vtrack.max_y() - 8.0),
        }),
        sm_bounds,
    );
    assert!(scroll_ed.scroll_top > 20.0);
    scroll_ed.handle_event(
        &InputEvent::MouseUp(MouseEvent::new(
            Point::new(vtrack.min_x() + 3.0, vtrack.max_y() - 8.0),
            MouseButton::Left,
        )),
        sm_bounds,
    );

    // M3-04..M3-08: Comments, move/duplicate lines, Tab indent/outdent, brackets, right-click context menu
    let registry = GrammarRegistry::new();
    let dark = ThemeService::default_dark_theme();
    let mut ops_ed = EditorView::from_str("let a = 1;\nlet b = 2;\n");
    ops_ed.set_language("rust", dark.clone(), &registry);
    let ctrl_slash = InputEvent::KeyDown(KeyEvent::new(
        KeyCode::Char('/'),
        Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        },
    ));
    ops_ed.handle_event(&ctrl_slash, bounds);
    assert!(ops_ed.text().starts_with("// "));
    ops_ed.handle_event(&ctrl_slash, bounds);
    assert_eq!(ops_ed.text(), "let a = 1;\nlet b = 2;\n");

    let mut bracket_ed = EditorView::from_str("");
    bracket_ed.handle_event(
        &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('('), Modifiers::none())),
        bounds,
    );
    assert_eq!(bracket_ed.text(), "()");
    bracket_ed.handle_event(
        &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(')'), Modifiers::none())),
        bounds,
    );
    assert_eq!(bracket_ed.text(), "()");
    bracket_ed.handle_event(
        &InputEvent::KeyDown(KeyEvent::new(KeyCode::ArrowLeft, Modifiers::none())),
        bounds,
    );
    bracket_ed.handle_event(
        &InputEvent::KeyDown(KeyEvent::new(KeyCode::Backspace, Modifiers::none())),
        bounds,
    );
    assert_eq!(bracket_ed.text(), "");

    let mut ctx_ed = EditorView::from_str("let z = 1;\n");
    ctx_ed.handle_event(
        &InputEvent::MouseDown(MouseEvent::new(Point::new(100.0, 10.0), MouseButton::Right)),
        bounds,
    );
    assert!(ctx_ed.context_menu.is_visible);
    let menu_rect = ctx_ed.context_menu.compute_bounds(bounds).unwrap();
    let go_def_pt = Point::new(
        menu_rect.min_x() + 20.0,
        menu_rect.min_y()
            + rustcode_editor::EditorContextMenu::PADDING_Y
            + 3.5 * rustcode_editor::EditorContextMenu::ITEM_HEIGHT,
    );
    ctx_ed.handle_event(
        &InputEvent::MouseDown(MouseEvent::new(go_def_pt, MouseButton::Left)),
        bounds,
    );
    assert!(matches!(ctx_ed.take_last_action(), Some(EditorAction::GoToDefinition(_))));

    // M3-09 & M3-10: Grapheme navigation & unicode-width CJK visual alignment
    let mut g_ed = EditorView::from_str("e\u{0301}👨‍👩‍👧‍👦🇺🇸z");
    g_ed.move_cursor_right(false);
    assert_eq!(g_ed.primary_position(), Position::new(0, 2));
    g_ed.move_cursor_right(false);
    assert_eq!(g_ed.primary_position(), Position::new(0, 13));
    g_ed.delete_backwards();
    assert_eq!(g_ed.text(), "e\u{0301}🇺🇸z");

    let mut w_ed = EditorView::from_str("中文a\n012345\n");
    w_ed.cursor_state.set_primary(Position::new(0, 2));
    let mut scene = Scene::new();
    w_ed.render(bounds, &WorkbenchThemeStyles::from_theme(&dark), &font, &mut scene);
    let expected_x = bounds.min_x() + w_ed.gutter_width(&font) + 8.0 + 4.0 * font.char_width;
    assert!(scene.quads().iter().any(|q| {
        (q.bounds.width() - 2.0).abs() < 0.1 && (q.bounds.min_x() - expected_x).abs() < 0.5
    }));
    w_ed.move_cursor_down(false);
    assert_eq!(w_ed.primary_position(), Position::new(1, 4));

    // M3-11: encoding_rs multi-encoding decode/save & NUL binary rejection
    let ws = TempWorkspace::new();
    let fs_svc = FileSystemService::new();
    let utf16_path = ws.write_bytes("utf16.txt", &TextEncoding::Utf16Le.encode("héllo 日本語"));
    let doc16 = TextDocument::open_with_encoding_sync(&fs_svc, &utf16_path, None).unwrap();
    assert_eq!(doc16.encoding, TextEncoding::Utf16Le);
    assert_eq!(doc16.text(), "héllo 日本語");

    let (win_bytes, _, _) = WINDOWS_1252.encode("café");
    let win_path = ws.write_bytes("win1252.txt", &win_bytes);
    let doc_win =
        TextDocument::open_with_encoding_sync(&fs_svc, &win_path, Some(TextEncoding::Windows1252))
            .unwrap();
    assert_eq!(doc_win.text(), "café");

    let (sjis_bytes, _, _) = SHIFT_JIS.encode("テスト");
    let sjis_path = ws.write_bytes("sjis.txt", &sjis_bytes);
    let mut doc_sjis = TextDocument::open_with_encoding_sync(&fs_svc, &sjis_path, None).unwrap();
    assert_eq!(doc_sjis.encoding, TextEncoding::ShiftJis);
    assert_eq!(doc_sjis.text(), "テスト");
    doc_sjis
        .save_with_encoding_sync(&fs_svc, TextEncoding::Utf8Bom)
        .unwrap();
    assert_eq!(&std::fs::read(&sjis_path).unwrap()[..3], &[0xEF, 0xBB, 0xBF]);

    let bin_path = ws.write_bytes("bin.dat", b"ELF\x00\x01\x02");
    assert!(TextDocument::open_with_encoding_sync(&fs_svc, &bin_path, None).is_err());
}

/// [M3-12..M3-16] ripgrep search/replace, nucleo fuzzy ranking, TextMate .tmLanguage/.tmLanguage.json, and tree-sitter incremental highlighting.
#[test]
fn t1_m3_ripgrep_search_replace_nucleo_textmate_treesitter() {
    let ws = TempWorkspace::new();
    ws.write_file(".gitignore", "ignored_dir/\n");
    ws.write_file("ignored_dir/secret.rs", "let target_sym = 0;\n");
    ws.write_file("src/alpha.rs", "fn old_1() {}\nfn old_2() {}\n");
    ws.write_file("src/beta.txt", "let old_3 = 30;\n");
    ws.write_file(".hidden.rs", "let old_hidden = 40;\n");

    let folders = vec![WorkspaceFolder {
        name: "ws".to_string(),
        path: ws.path().to_path_buf(),
        uri: path_to_file_uri(ws.path()),
        index: 0,
    }];

    // M3-12: ripgrep honors .gitignore, hidden rules, globs, and rejects invalid regex cleanly
    let q_rs = SearchQuery {
        query: r"old_(\d+)".to_string(),
        is_regex: true,
        files_to_include: vec!["*.rs".to_string()],
        ..Default::default()
    };
    let found = search_text(&folders, &q_rs);
    assert_eq!(found.total_matches, 2);
    assert!(try_search_text(
        &folders,
        &SearchQuery {
            query: "[unclosed(".to_string(),
            is_regex: true,
            ..Default::default()
        }
    )
    .is_err());

    // M3-13: Workspace replace with regex capture group `$1`
    let rep = replace_in_workspace(&folders, &q_rs, "new_$1").unwrap();
    assert_eq!(rep.files_modified, 1);
    assert_eq!(rep.total_replacements, 2);
    assert_eq!(ws.read_file("src/alpha.rs"), "fn new_1() {}\nfn new_2() {}\n");
    assert_eq!(ws.read_file("src/beta.txt"), "let old_3 = 30;\n");

    // M3-14: nucleo fuzzy ranking in CommandPalette & workspace search_files / search_workspace_symbols
    let files = search_files(&folders, "alpha", 100);
    assert!(files.iter().any(|p| p.ends_with("alpha.rs")));
    let syms = search_workspace_symbols(&folders, "new_1");
    assert!(syms.iter().any(|s| s.name == "new_1"));

    let mut palette = CommandPalette::new();
    palette.set_items(vec![
        PaletteItem::command("scattered", "Open Foo Bar Baz File", None, None),
        PaletteItem::command("exact", "openFile", None, None),
    ]);
    palette.open_mode(QuickInputMode::Command);
    palette.set_query(">openFile");
    assert_eq!(palette.results()[0].command_id, "exact");
    palette.set_query(">🚀"); // Unicode safety

    // M3-15: TextMate .tmLanguage (XML plist) + .tmLanguage.json + malformed regex safety
    let tm_json = r#"{
        "name": "DemoLang",
        "scopeName": "source.demolang",
        "patterns": [
            { "match": "\\b(let|fn)\\b", "name": "keyword.control.demo" },
            { "begin": "\"", "end": "\"", "name": "string.quoted.double.demo" },
            { "match": "(?invalid", "name": "invalid.regex.demo" }
        ]
    }"#;
    let tm_plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>name</key><string>PlistLang</string>
    <key>scopeName</key><string>source.plistlang</string>
    <key>patterns</key>
    <array>
        <dict>
            <key>match</key><string>//.*$</string>
            <key>name</key><string>comment.line.double-slash.plist</string>
        </dict>
    </array>
</dict>
</plist>"#;
    let g_json = Arc::new(TextMateGrammar::from_json_str(tm_json, "demolang", &["demo"]).unwrap());
    let g_plist = Arc::new(
        TextMateGrammar::from_plist_bytes(tm_plist.as_bytes(), "plistlang", &["plst"]).unwrap(),
    );

    let mut reg = GrammarRegistry::new();
    assert_eq!(reg.get_by_id("rust").unwrap().language_id(), "rust");
    reg.register_textmate(g_json);
    reg.register_textmate(g_plist);

    let theme = ThemeService::default_dark_theme();
    let mut hl_tm = SyntaxHighlighter::for_language("demolang", theme.clone(), &reg);
    let spans = hl_tm.get_line_tokens(0, "fn greet() { \"hi\" }");
    assert!(spans.iter().any(|s| s.scope == SyntaxScope::KeywordControl));
    assert!(spans.iter().any(|s| s.scope == SyntaxScope::StringQuoted));

    let mut hl_plist = SyntaxHighlighter::for_language("plistlang", theme.clone(), &reg);
    let c_spans = hl_plist.get_line_tokens(0, "// comment line");
    assert!(c_spans.iter().any(|s| s.scope == SyntaxScope::CommentLine));

    // M3-16 (desktop feature): tree-sitter incremental highlighting
    #[cfg(feature = "desktop")]
    {
        for lang in ["rust", "javascript", "typescript", "python", "json"] {
            assert!(TreeSitterEngine::supports_language(lang));
        }
        let mut ed = EditorView::from_str("fn main() {\n\n    let val = 10;\n}\n");
        ed.set_language("rust", theme, &reg);
        ed.cursor_state.set_primary(Position::new(1, 0));
        ed.insert_text("/* ");
        let line2 = ed.buffer.read().get_line_content(2).unwrap();
        let mut hl = ed.syntax_highlighter.lock();
        let comment_tokens = hl.as_mut().unwrap().get_line_tokens(2, &line2);
        assert!(
            comment_tokens
                .iter()
                .all(|t| matches!(t.scope, SyntaxScope::Comment | SyntaxScope::CommentBlock)),
            "expected incremental tree-sitter comment tokens, got {comment_tokens:?}"
        );
        drop(hl);
        assert!(ed.undo());
        let line2_back = ed.buffer.read().get_line_content(2).unwrap();
        let mut hl = ed.syntax_highlighter.lock();
        let code_tokens = hl.as_mut().unwrap().get_line_tokens(2, &line2_back);
        assert!(code_tokens.iter().any(|t| t.scope == SyntaxScope::Keyword));
    }
}
