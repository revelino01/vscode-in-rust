use phase9_e2e::{create_test_window, dispatch, exec_cmd, key_down, mouse_down, mouse_move, mouse_up, render_window_scene, TempWorkspace};
use rustcode_app::multi_window::{OpenRequest, SingleInstanceBind, SingleInstanceIpc};
use rustcode_editor::{
    expand_snippet_with_context, EditorView, LanguageConfigurationRegistry, PieceTable,
    SnippetContext, SnippetRegistry, SnippetSession,
};
use rustcode_ext_host::ExtensionLoader;
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::render::{KeyCode, MouseButton, Point, Rect, View};
use std::path::PathBuf;

#[test]
fn tier4_adversarial_malformed_snippets_and_path_traversal_guards() {
    // 1. Malformed snippets & broken regex transforms never panic
    let ctx = SnippetContext::from_editor(None, 0, "", "", "", "");
    let weird = expand_snippet_with_context("${unclosed ${1/[broken/repl/g} $0 ${TM_YEAR}", &ctx);
    assert!(!weird.text.is_empty());

    let mut ed = EditorView::from_str("");
    assert!(ed.insert_snippet("fn ${1:name}() { $0 }", Some(ctx)));
    let bounds = Rect::from_xywh(0.0, 0.0, 600.0, 400.0);
    for _ in 0..15 {
        ed.handle_event(&key_down(KeyCode::Backspace, Modifiers::none()), bounds);
    }
    ed.handle_event(&key_down(KeyCode::Escape, Modifiers::none()), bounds);
    assert!(ed.snippet_session.is_none());

    // 2. Empty expanded snippet creates no session
    let empty_exp = expand_snippet_with_context("plain text", &SnippetContext::default());
    assert!(SnippetSession::from_expanded(0, empty_exp).is_none());

    // 3. Extension path traversal guard rejects `../` escapes
    let ws = TempWorkspace::new("phase9_t4_traversal");
    ws.write_file("secret.json", r#"{ "Bad": { "prefix": "bad", "body": "bad" } }"#);
    ws.write_file("secret-lc.json", r#"{ "comments": { "lineComment": "!!" } }"#);
    let ext_dir = ws.mkdir_all("ext");
    ws.write_file(
        "ext/package.json",
        r#"{
            "name": "evil-ext",
            "publisher": "BadPub",
            "version": "1.0.0",
            "contributes": {
                "languages": [{ "id": "evillang", "configuration": "../secret-lc.json" }],
                "snippets": [{ "language": "evillang", "path": "../secret.json" }]
            }
        }"#,
    );
    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).unwrap();

    let mut snip_reg = SnippetRegistry::new();
    rustcode_app::runtime::load_extension_snippets(&mut snip_reg, &loader);
    assert!(snip_reg.snippets_for_language("evillang").is_empty());

    let mut lc_reg = LanguageConfigurationRegistry::new();
    rustcode_app::runtime::load_extension_language_configs(&mut lc_reg, &loader);
    assert!(lc_reg.get("evillang").is_none());
}

#[test]
fn tier4_adversarial_explorer_cycle_guard_nan_bounds_and_zen_zoom_churn() {
    let ws = TempWorkspace::new("phase9_t4_guards");
    let folder = ws.mkdir_all("parent");
    let nested = ws.mkdir_all("parent/child/grandchild");
    let file_a = ws.write_file("parent/a.txt", "hello");

    let (ctx, mut win) = create_test_window();
    win.open_workspace(ws.path());

    // 1. Cycle & self-move guards
    assert!(win.view.explorer.move_entry(&folder, &folder).is_err());
    assert!(win.view.explorer.move_entry(&folder, &nested).is_err());
    assert!(win.view.explorer.move_entry(&file_a, &folder).is_err()); // same directory no-op rejected

    // 2. Non-finite / zero bounds never panic across sashes, menus, or TitleBar
    let zero_rect = Rect::zero();
    let nan_rect = Rect::from_xywh(f32::NAN, 0.0, f32::INFINITY, 600.0);
    dispatch(
        &mut win,
        &mouse_down(Point::new(f32::NAN, f32::NEG_INFINITY), MouseButton::Left),
        zero_rect,
    );
    dispatch(
        &mut win,
        &mouse_move(Point::new(f32::INFINITY, 200.0)),
        nan_rect,
    );
    dispatch(&mut win, &mouse_up(Point::new(10.0, 10.0), MouseButton::Left), zero_rect);

    // 3. Rapid Zen + Zoom + Panel position churn restores exact state
    let initial_sb = win.view.layout.sidebar_visible;
    exec_cmd(&ctx, &mut win, "workbench.action.toggleAuxiliaryBar");
    exec_cmd(&ctx, &mut win, "workbench.action.toggleMenuBar");
    for _ in 0..20 {
        exec_cmd(&ctx, &mut win, "workbench.action.zoomIn");
        exec_cmd(&ctx, &mut win, "workbench.action.positionPanelLeft");
        exec_cmd(&ctx, &mut win, "workbench.action.toggleZenMode");
        exec_cmd(&ctx, &mut win, "workbench.action.toggleZenMode");
        exec_cmd(&ctx, &mut win, "workbench.action.positionPanelRight");
    }
    exec_cmd(&ctx, &mut win, "workbench.action.zoomReset");
    assert_eq!(win.view.layout.sidebar_visible, initial_sb);
    assert!(win.view.layout.secondary_sidebar_visible);
    assert!(win.view.layout.title_bar_visible);
    assert_eq!(win.view.layout.zoom_level, 0.0);
    let _ = render_window_scene(&win, 1280.0, 800.0);
}

#[test]
fn tier4_adversarial_100mb_utf8_edits_and_stale_ipc_recovery() {
    // 1. 100 MB multi-byte UTF-8 file mid-file edit boundary safety
    let utf8_line = "fn テスト() { let msg = \"🚀\"; }\n";
    let target_bytes = 100 * 1024 * 1024;
    let payload = utf8_line.repeat(target_bytes / utf8_line.len() + 1);
    let mut pt = PieceTable::from_str(&payload);
    drop(payload);

    let mid = pt.len() / 2;
    let safe_mid = pt.floor_char_boundary(mid);
    let (line, col) = pt.offset_to_position(safe_mid);
    assert_eq!(pt.position_to_offset(line, col), safe_mid);
    pt.insert(safe_mid, "α");
    assert_eq!(pt.get_range(safe_mid, safe_mid + "α".len()), "α");
    assert_eq!(pt.delete(safe_mid, "α".len()), "α");

    // 2. Stale IPC recovery on dead lockfile/socket
    let ws = TempWorkspace::new("phase9_t4_ipc");
    let ipc_dir = ws.mkdir_all("stale_ipc");
    #[cfg(windows)]
    std::fs::write(
        ipc_dir.join("rustcode-ipc.lock"),
        r#"{"port":1,"token":"stale","pid":999999}"#,
    )
    .unwrap();

    let req = OpenRequest {
        paths: vec![PathBuf::from("fresh.rs")],
        goto: Vec::new(),
        new_window: false,
        reuse_window: false,
    };
    let bound = SingleInstanceIpc::try_bind_or_forward(&ipc_dir, &req, None).unwrap();
    assert!(matches!(bound, SingleInstanceBind::Leader(_)));
    drop(bound);
}
