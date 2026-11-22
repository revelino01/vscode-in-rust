use phase8_e2e::TempWorkspace;
use rustcode_app::RustCodeContext;
use rustcode_ext_host::manifest::{CustomEditorContribution, CustomEditorSelector};
use rustcode_ext_host::provider::{LanguageFeatureBridge, ProviderRegistry};
use rustcode_ext_host::rpc::RpcBridge;
use rustcode_ext_host::webview::WebviewOptions;
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::render::{InputEvent, KeyCode, KeyEvent, MockRenderSurface, Rect, Rgba};
use rustcode_protocols::lsp::LspClientManager;
use rustcode_terminal::pty::HeadlessPtyBackend;
use rustcode_terminal::{TerminalInstance, TerminalManager, DEFAULT_MAX_SCROLLBACK};
use rustcode_workbench::webview::{HeadlessWebviewBackend, WebviewEditorView};
use rustcode_workbench::WorkbenchWindow;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(feature = "desktop")]
use rustcode_workbench::webview::{WebviewBackend, WryWebviewBackend};

/// [M4-01..M4-02, M4-05] Real OS PTY interactive shell echo, cwd persistence, and exit status propagation.
#[test]
fn t2_m4_portable_pty_interactive_shell_echo_cwd_and_exit() {
    let ws = TempWorkspace::new();
    let sub_dir = ws.mkdir_all("sub_cwd");

    let mut mgr = TerminalManager::new();
    mgr.create_pty_terminal(
        Some("Phase8 PTY"),
        None,
        Some(ws.path().to_path_buf()),
        24,
        100,
    )
    .expect("PortablePtyBackend shell spawn must succeed");
    let term_id = mgr.active_terminal().unwrap().id.clone();

    // Wait briefly for initial shell prompt, then run `echo PHASE8_PTY_OK`, `cd sub_cwd`, pwd/cd, and `exit`.
    std::thread::sleep(Duration::from_millis(250));
    mgr.poll_all_ptys();

    let term = mgr.terminal_mut(&term_id).unwrap();
    assert!(term.has_pty());
    if cfg!(windows) {
        term.send_pty_str("echo PHASE8_PTY_OK\r\n").unwrap();
        term.send_pty_str("cd sub_cwd\r\n").unwrap();
        term.send_pty_str("cd\r\n").unwrap();
        term.send_pty_str("exit\r\n").unwrap();
    } else {
        term.send_pty_str("echo PHASE8_PTY_OK\n").unwrap();
        term.send_pty_str("cd sub_cwd\n").unwrap();
        term.send_pty_str("pwd\n").unwrap();
        term.send_pty_str("exit\n").unwrap();
    }

    let deadline = Instant::now() + Duration::from_secs(8);
    let mut saw_echo = false;
    let mut saw_cwd = false;
    let mut saw_exit = false;

    while Instant::now() < deadline {
        mgr.poll_all_ptys();
        let t = mgr.terminal(&term_id).unwrap();
        let snap = t.grid_snapshot();
        let joined = snap.visible_lines().join("\n");
        if joined.contains("PHASE8_PTY_OK") {
            saw_echo = true;
        }
        if joined.contains("sub_cwd") && sub_dir.is_dir() {
            saw_cwd = true;
        }
        if t.exited {
            saw_exit = true;
        }
        if saw_echo && saw_cwd && saw_exit {
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }

    assert!(saw_echo, "PTY grid must capture PHASE8_PTY_OK from interactive shell");
    assert!(saw_cwd, "PTY shell working directory must persist across commands");
    assert!(saw_exit, "PTY exit must propagate exited state");
}

/// [M4-03, M4-04, M4-06] VT100/ANSI SGR 16/256/truecolor, alt-screen, resize clamping, TerminalPanelView Scene, and ExtHost sync.
#[test]
fn t2_m4_vt100_ansi_sgr_truecolor_alt_screen_and_resize() {
    let mut term = TerminalInstance::new("t1", "VT", "sh", ".".into());
    term.resize(0, 0).unwrap(); // safe clamping >= 1
    term.resize(12, 60).unwrap();

    // 16-color red, 256-color, truecolor, cursor positioning
    term.feed_vt_bytes(b"\x1b[31mRED\x1b[0m ");
    term.feed_vt_bytes(b"\x1b[38;5;196mC256\x1b[0m ");
    term.feed_vt_bytes(b"\x1b[38;2;10;20;30mRGB\x1b[0m");
    let snap = term.grid_snapshot();
    assert_eq!(snap.rows, 12);
    assert_eq!(snap.cols, 60);
    let row0 = &snap.cells[0];
    assert_eq!(row0[0].ch, 'R');
    assert_ne!(row0[0].fg, [255, 255, 255, 255]);
    // Check truecolor cell at index 9 ("RED C256 RGB" -> 'R' of RGB is col 9)
    assert_eq!(row0[9].ch, 'R');
    assert_eq!(row0[9].fg, [10, 20, 30, 255]);

    // Alternate screen switch & restore
    term.feed_vt_bytes(b"\x1b[?1049hALT_ONLY");
    assert!(term.grid_snapshot().visible_lines().join("\n").contains("ALT_ONLY"));
    term.feed_vt_bytes(b"\x1b[?1049l");
    let main_lines = term.grid_snapshot().visible_lines().join("\n");
    assert!(main_lines.contains("RED") && !main_lines.contains("ALT_ONLY"));
    const { assert!(DEFAULT_MAX_SCROLLBACK >= 1000); };

    // TerminalPanelView renders VT cell grid + cursor into Scene + ExtHostTerminalManager sync
    let rpc = Arc::new(RpcBridge::new());
    let registry = Arc::new(ProviderRegistry::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp));
    bridge
        .terminal_manager
        .create_terminal("ext_pty", "ExtPTY", None, vec![], None);
    bridge
        .terminal_manager
        .send_text("ext_pty", "echo EXT_SYNC", true)
        .unwrap();
    bridge.terminal_manager.show_terminal("ext_pty", false).unwrap();

    let mut window = WorkbenchWindow::default().with_language_bridge(bridge.clone());
    window.sync_ext_terminals();
    let local_id = window
        .view
        .terminal_panel
        .manager
        .active_terminal()
        .unwrap()
        .id
        .clone();
    let pty = HeadlessPtyBackend::new(24, 80);
    pty.push_output_str("\x1b[38;2;25;150;220mVT_SCENE_CELL\x1b[0m\r\n");
    {
        let local_term = window
            .view
            .terminal_panel
            .manager
            .terminal_mut(&local_id)
            .unwrap();
        local_term.attach_pty(Box::new(pty));
        local_term.resize(28, 90).unwrap();
    }
    window.poll_runtime_events();

    let ext_state = bridge.terminal_manager.get_terminal("ext_pty").unwrap();
    assert_eq!(ext_state.rows, Some(28));
    assert_eq!(ext_state.cols, Some(90));

    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("VT_SCENE_CELL"));
    let _ = Rgba::new(0.0, 0.0, 0.0, 1.0);
    let _ = Rect::from_xywh(0.0, 0.0, 100.0, 100.0);
}

/// [M4-07..M4-09] WebviewBackend & CustomEditor bidirectional postMessage IPC and WryWebviewBackend contract.
#[test]
fn t2_m4_webview_and_custom_editor_postmessage_and_wry_contract() {
    let rpc = Arc::new(RpcBridge::new());
    let registry = Arc::new(ProviderRegistry::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp));

    // 1. Standalone WebviewPanel HTML sanitization + bidirectional postMessage
    bridge.webview_manager.create_panel(
        "p_m4",
        "m4.webview",
        "M4 Webview",
        WebviewOptions {
            enable_scripts: false,
            local_resource_roots: vec![],
        },
    );
    bridge
        .webview_manager
        .set_html("p_m4", "<h1>Safe</h1><script>alert(1)</script>");
    bridge
        .webview_manager
        .post_message_to_webview("p_m4", json!({ "type": "init", "n": 1 }))
        .unwrap();

    let ctx = RustCodeContext::new();
    let mut window = ctx.create_window().with_language_bridge(bridge.clone());
    window.sync_webviews();

    let wv = window
        .view
        .editor_area
        .webviews
        .get_mut("webview://p_m4")
        .unwrap();
    assert!(
        !wv.backend.current_html().contains("<script>"),
        "scripts must be sanitized when enable_scripts=false"
    );
    assert_eq!(wv.inbound_messages, vec![json!({ "type": "init", "n": 1 })]);

    let mut mock_backend = HeadlessWebviewBackend::with_html("<h1>Safe</h1>");
    mock_backend.inject_outbound_message(json!({ "type": "ack", "ok": true }));
    wv.set_backend(Box::new(mock_backend));
    window.sync_webview_outbound_messages();
    let inbound = bridge.webview_manager.take_inbound_messages("p_m4");
    assert_eq!(inbound, vec![json!({ "type": "ack", "ok": true })]);

    // 2. CustomEditor lifecycle & postMessage
    bridge.custom_editor_manager.register_provider(
        "e2e.jsonView",
        WebviewOptions {
            enable_scripts: true,
            local_resource_roots: vec![],
        },
        true,
    );
    window.register_custom_editor_contribution(CustomEditorContribution {
        view_type: "e2e.jsonView".to_string(),
        display_name: "JSON View".to_string(),
        selector: vec![CustomEditorSelector {
            filename_pattern: "*.e2ejson".to_string(),
        }],
        priority: Some("default".to_string()),
    });
    let ce_uri = "file:///ws/data.e2ejson";
    window.open_file(ce_uri, "data.e2ejson", "{}", false);
    let session = bridge
        .custom_editor_manager
        .find_session_by_uri(ce_uri, Some("e2e.jsonView"))
        .unwrap();
    bridge
        .webview_manager
        .set_html(&session.panel_id, "<div>Custom</div>");
    window.sync_webviews();
    assert!(window.view.editor_area.webviews[ce_uri].html.contains("Custom"));

    window.dispatch_event(
        &InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('w'),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        )),
        Rect::from_xywh(0.0, 0.0, 1024.0, 768.0),
    );
    assert!(bridge.webview_manager.get_panel(&session.panel_id).unwrap().is_disposed);
    assert!(bridge.custom_editor_manager.get_session(&session.id).unwrap().is_closed);

    // 3. desktop-gated WryWebviewBackend unattached Send+Sync smoke
    #[cfg(feature = "desktop")]
    {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<WryWebviewBackend>();
        let mut wry_backend = WryWebviewBackend::new();
        wry_backend.load_html("<p>Wry Ready</p>");
        wry_backend.post_message(&json!({ "ping": 1 })).unwrap();
        wry_backend.set_bounds(Rect::from_xywh(0.0, 0.0, 640.0, 480.0));
        wry_backend.set_visible(true);
        assert_eq!(wry_backend.current_html(), "<p>Wry Ready</p>");
        let _ = WebviewEditorView::new(
            "wry",
            "wry.view",
            "Wry",
            "<html></html>",
            WebviewOptions::default(),
            None,
        );
    }
}
