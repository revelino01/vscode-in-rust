use phase7_e2e::{create_bridge_with_root, repo_root, E2eWindow, TempWorkspace};
use rustcode_editor::{
    GrammarRegistry, Position, Selection, SyntaxHighlighter, SyntaxScope, TextDocument,
};
use rustcode_ext_host::{ExtensionLoader, LanguageFeatureBridge};
use rustcode_platform::fs::{file_uri_to_path, path_to_file_uri, paths_equivalent};
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::render::{KeyCode, MockRenderSurface, Point, Rect};
use rustcode_platform::theme::ThemeService;
use rustcode_workbench::workbench_view::FocusedRegion;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

fn dispatch_ctrl_shift_char(win: &mut E2eWindow, ch: char) -> bool {
    win.dispatch_key(
        KeyCode::Char(ch),
        Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        },
        None,
    )
    .is_consumed()
}

fn dispatch_shift_key(win: &mut E2eWindow, key: KeyCode) -> bool {
    win.dispatch_key(
        key,
        Modifiers {
            ctrl: false,
            shift: true,
            alt: false,
            meta: false,
        },
        None,
    )
    .is_consumed()
}

fn trigger_sync(win: &mut E2eWindow) {
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
}

fn setup_treeview_ext(ws: &TempWorkspace) -> ExtensionLoader {
    let fixture_manifest = std::fs::read_to_string(
        repo_root().join("crates/rustcode-ext-host/tests/fixtures/extensions/treeview_ext/package.json"),
    )
    .expect("treeview_ext fixture missing");
    let ext_dir = ws.path().join(".ext/treeview_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("package.json"), fixture_manifest).unwrap();

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).expect("load treeview_ext");
    loader
}

// ============================================================================
// F9: In-Editor Find & Replace — Tier 2 Boundary & Negative
// ============================================================================

/// [F9] Empty query renders 'No results' / 0 matches and handles backspace to empty
#[test]
fn t2_f9_01_empty_query_shows_no_results() {
    let ws = TempWorkspace::new();
    ws.write_file("sample.rs", "fn hello() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("sample.rs");
    assert!(win.dispatch_ctrl_char('f').is_consumed(), "[F9] Ctrl+F must open find");

    let s_empty = win.render();
    assert!(
        s_empty.contains_text("No results"),
        "[F9] Empty query must show 'No results', got: {}",
        s_empty.all_text()
    );
    assert!(!s_empty.contains_text("1 of "));

    win.type_text("h");
    assert!(win.render().contains_text("1 of 1"));
    win.dispatch_key(KeyCode::Backspace, Modifiers::none(), None);
    let s_cleared = win.render();
    assert!(
        s_cleared.contains_text("No results"),
        "[F9] Clearing query back to empty must return to 'No results', got: {}",
        s_cleared.all_text()
    );
}

/// [F9] Zero-width regex patterns (^, .*, a*) complete without hanging
#[test]
fn t2_f9_02_zero_width_regex_does_not_hang() {
    let ws = TempWorkspace::new();
    ws.write_file("zw.txt", "abc\ndef\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("zw.txt");
    assert!(win.dispatch_ctrl_char('f').is_consumed(), "[F9] Ctrl+F must open find");

    let toggled_regex =
        win.click_a11y_node("Use Regular Expression") || win.click_scene_text(".*");
    assert!(toggled_regex, "[F9] Regex toggle must be clickable");

    for pat in ["^", ".*", "a*"] {
        // Clear any existing query characters
        for _ in 0..8 {
            win.dispatch_key(KeyCode::Backspace, Modifiers::none(), None);
        }
        win.type_text(pat);
        let scene = win.render();
        assert!(
            scene.contains_text(" of ") || scene.contains_text("No results"),
            "[F9] Zero-width regex '{pat}' must render bounded match status without hanging"
        );
        win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    }
}

/// [F9] Invalid regex ([unclosed) displays error state without panicking or mutating buffer
#[test]
fn t2_f9_03_invalid_regex_shows_error_without_panic() {
    let ws = TempWorkspace::new();
    let content = "hello [world]\n";
    ws.write_file("bad_regex.txt", content);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("bad_regex.txt");
    assert!(win.dispatch_ctrl_char('h').is_consumed(), "[F9] Ctrl+H must open find/replace");

    let toggled_regex =
        win.click_a11y_node("Use Regular Expression") || win.click_scene_text(".*");
    assert!(toggled_regex, "[F9] Regex toggle must be clickable");

    win.type_text("[unclosed");
    let scene = win.render();
    assert!(
        scene.contains_text("No results")
            || scene.all_text().to_ascii_lowercase().contains("invalid")
            || scene.all_text().to_ascii_lowercase().contains("error"),
        "[F9] Invalid regex must show error/No results without panic, got: {}",
        scene.all_text()
    );

    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    let _ = win.click_a11y_node("Replace All") || win.click_scene_text("All");
    assert_eq!(
        win.active_editor_text().as_deref(),
        Some(content),
        "[F9] Invalid regex must not mutate editor buffer"
    );
}

/// [F9] UTF-16 surrogate pairs (emoji) and CRLF line endings compute accurate positions
#[test]
fn t2_f9_04_utf16_surrogate_pairs_and_crlf_positions() {
    let ws = TempWorkspace::new();
    let crlf_text = "🚀 café\r\nhello 🚀 world\r\n";
    ws.write_file("utf16.txt", crlf_text);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("utf16.txt");
    assert!(win.dispatch_ctrl_char('h').is_consumed(), "[F9] Ctrl+H must open find/replace");

    win.type_text("café");
    let scene = win.render();
    assert!(
        scene.contains_text("1 of 1"),
        "[F9] Must find UTF-8 'café' after surrogate-pair emoji, got: {}",
        scene.all_text()
    );

    let pos = win.active_editor().unwrap().primary_position();
    assert_eq!(
        pos.line, 0,
        "[F9] Match 'café' must be on line 0"
    );
    // "🚀 " is 3 UTF-16 code units; "café" ends at UTF-16 col 7 (or cursor at 3 or 7)
    assert!(
        pos.column == 3 || pos.column == 7,
        "[F9] Cursor column after matching 'café' following '🚀 ' must be UTF-16 offset (3 or 7), got {:?}",
        pos
    );

    win.dispatch_key(KeyCode::Tab, Modifiers::none(), None);
    win.type_text("tea");
    let clicked = win.click_a11y_node("Replace All")
        || win.click_scene_text("Replace All")
        || win.click_scene_text("All");
    assert!(clicked, "[F9] Replace All must be clickable");
    assert_eq!(
        win.active_editor_text().as_deref(),
        Some("🚀 tea\r\nhello 🚀 world\r\n"),
        "[F9] Replace with surrogate pairs + CRLF must preserve emoji and \\r\\n"
    );
}

/// [F9] Selection-seeded Ctrl+F and wrap-around at end and start of document
#[test]
fn t2_f9_05_selection_seeded_find_and_wrap_around() {
    let ws = TempWorkspace::new();
    ws.write_file("wrap.txt", "word one\nword two\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("wrap.txt");
    {
        let ed = win.window.view.editor_area.active_editor_mut().unwrap();
        *ed.cursor_state.primary_mut() =
            Selection::new(Position::new(0, 0), Position::new(0, 4));
    }

    assert!(win.dispatch_ctrl_char('f').is_consumed(), "[F9] Ctrl+F must open find");
    let s1 = win.render();
    assert!(
        s1.contains_text("1 of 2"),
        "[F9] Selection-seeded Ctrl+F must prefill 'word' and show '1 of 2', got: {}",
        s1.all_text()
    );

    // Advance to 2 of 2, then wrap around back to 1 of 2
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert!(win.render().contains_text("2 of 2"));
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert!(
        win.render().contains_text("1 of 2"),
        "[F9] Enter at last match must wrap around to '1 of 2'"
    );

    // Shift+Enter at first match must wrap around to 2 of 2
    dispatch_shift_key(&mut win, KeyCode::Enter);
    assert!(
        win.render().contains_text("2 of 2"),
        "[F9] Shift+Enter at first match must wrap around to '2 of 2'"
    );
}

/// [F9] Navigating to a match inside a collapsed fold automatically unfolds it
#[test]
fn t2_f9_06_unfolding_collapsed_fold_containing_match() {
    let ws = TempWorkspace::new();
    ws.write_file(
        "fold.rs",
        "fn outer() {\n    let hidden_secret = 42;\n}\n",
    );

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("fold.rs");
    {
        let ed = win.window.view.editor_area.active_editor_mut().unwrap();
        ed.set_primary_position(Position::new(0, 0));
        assert!(ed.fold_at_cursor(), "fold_at_cursor must fold fn outer");
        assert!(ed.folding_model.is_collapsed(0));
    }

    assert!(win.dispatch_ctrl_char('f').is_consumed(), "[F9] Ctrl+F must open find");
    win.type_text("hidden_secret");
    let _ = win.render();
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    let ed = win.active_editor().unwrap();
    assert!(
        !ed.folding_model.is_collapsed(0),
        "[F9] Jumping to match inside collapsed fold must unfold line 0"
    );
    assert_eq!(ed.primary_position().line, 1);
}

// ============================================================================
// F10: QuickInput :line[:col] & @symbol — Tier 2 Boundary & Negative
// ============================================================================

/// [F10] :0 clamps to 1-based line 1 (0-based line 0, col 0)
#[test]
fn t2_f10_01_colon_zero_clamps_to_line_one() {
    let ws = TempWorkspace::new();
    ws.write_file("clamp0.rs", "l1\nl2\nl3\nl4\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("clamp0.rs");
    win.window
        .view
        .editor_area
        .active_editor_mut()
        .unwrap()
        .set_primary_position(Position::new(2, 1));

    assert!(win.dispatch_ctrl_char('g').is_consumed(), "[F10] Ctrl+G must open");
    win.type_text("0");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    assert_eq!(
        win.active_editor().unwrap().primary_position(),
        Position::new(0, 0),
        "[F10] :0 must clamp to (0, 0)"
    );
}

/// [F10] :999999 clamps to the last line of the buffer
#[test]
fn t2_f10_02_colon_overflow_clamps_to_last_line() {
    let ws = TempWorkspace::new();
    ws.write_file("clamp_end.rs", "a\nb\nc\nd");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("clamp_end.rs");

    assert!(win.dispatch_ctrl_char('g').is_consumed(), "[F10] Ctrl+G must open");
    win.type_text("999999");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    assert_eq!(
        win.active_editor().unwrap().primary_position().line,
        3,
        "[F10] :999999 must clamp to last line index 3"
    );
}

/// [F10] :line:99999 clamps column to UTF-16 line length (not UTF-8 byte length)
#[test]
fn t2_f10_03_colon_line_overflow_col_clamps_to_utf16_length() {
    let ws = TempWorkspace::new();
    // Line 1 ("🚀x") has UTF-8 byte len 5 and UTF-16 code-unit len 3.
    ws.write_file("utf16_col.txt", "line0\n🚀x\nline2\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("utf16_col.txt");

    assert!(win.dispatch_ctrl_char('g').is_consumed(), "[F10] Ctrl+G must open");
    win.type_text("2:99999");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    assert_eq!(
        win.active_editor().unwrap().primary_position(),
        Position::new(1, 3),
        "[F10] :2:99999 on '🚀x' must clamp to UTF-16 length 3 -> (1, 3)"
    );
}

/// [F10] Non-numeric :abc shows prompt without jumping cursor or panicking
#[test]
fn t2_f10_04_non_numeric_colon_shows_prompt_without_jumping() {
    let ws = TempWorkspace::new();
    ws.write_file("nonnum.rs", "line0\nline1\nline2\nline3\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("nonnum.rs");
    win.window
        .view
        .editor_area
        .active_editor_mut()
        .unwrap()
        .set_primary_position(Position::new(2, 1));

    assert!(win.dispatch_ctrl_char('g').is_consumed(), "[F10] Ctrl+G must open");
    win.type_text("abc");
    let scene = win.render();
    assert!(
        scene.contains_text("line") || scene.contains_text("Line"),
        "[F10] Non-numeric :abc must display helpful line prompt, got: {}",
        scene.all_text()
    );

    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert_eq!(
        win.active_editor().unwrap().primary_position(),
        Position::new(2, 1),
        "[F10] Non-numeric :abc must not move cursor"
    );
}

/// [F10] @symbol on empty/unsupported file shows clean empty state & Escape cancels without moving cursor
#[test]
fn t2_f10_05_at_symbol_empty_unsupported_and_escape_cancels() {
    let ws = TempWorkspace::new();
    ws.write_file("empty.txt", "");
    ws.write_file("funcs.rs", "fn one() {}\nfn two() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("empty.txt");
    assert!(
        dispatch_ctrl_shift_char(&mut win, 'o'),
        "[F10] Ctrl+Shift+O must open cleanly on empty .txt file"
    );
    let _ = win.render();
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    assert!(!win.window.view.is_command_palette_open);

    // Open funcs.rs, place cursor at (0, 3), open @symbol, type 'two', press Escape -> cursor unchanged
    win.open_file_from_disk("funcs.rs");
    win.window
        .view
        .editor_area
        .active_editor_mut()
        .unwrap()
        .set_primary_position(Position::new(0, 3));

    assert!(dispatch_ctrl_shift_char(&mut win, 'o'));
    win.type_text("two");
    let _ = win.render();
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);

    assert!(!win.window.view.is_command_palette_open);
    assert_eq!(
        win.active_editor().unwrap().primary_position(),
        Position::new(0, 3),
        "[F10] Escape in @symbol QuickInput must cancel without moving cursor"
    );
}

// ============================================================================
// F11: Notification Toast Overlay & Interactive Prompts — Tier 2 Boundary/Neg
// ============================================================================

/// [F11] Small window bounds (< 320px wide/tall) render finite positive rects (no NaN/negative)
#[tokio::test]
async fn t2_f11_01_small_window_bounds_render_finite_positive_rects() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.bounds = Rect::from_xywh(0.0, 0.0, 240.0, 180.0);
    win.surface = MockRenderSurface::new(240.0, 180.0);

    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message(
            "window.showWarningMessage",
            &json!({ "message": "Compact viewport warning", "items": ["OK", "Cancel"] }),
        )
        .await;

    trigger_sync(&mut win);
    let scene = win.render();
    assert!(
        scene.contains_text("Compact viewport warning"),
        "[F11] Small viewport must still render notification overlay, got: {}",
        scene.all_text()
    );
    for quad in scene.quads() {
        assert!(
            quad.bounds.origin.x.is_finite()
                && quad.bounds.origin.y.is_finite()
                && quad.bounds.width().is_finite()
                && quad.bounds.height().is_finite()
                && quad.bounds.width() >= 0.0
                && quad.bounds.height() >= 0.0,
            "[F11] Quad bounds under small viewport must be finite and non-negative: {:?}",
            quad.bounds
        );
    }
    for run in scene.glyph_runs() {
        assert!(
            run.origin.x.is_finite()
                && run.origin.y.is_finite()
                && run.bounds.width().is_finite()
                && run.bounds.height().is_finite(),
            "[F11] GlyphRun bounds under small viewport must be finite: {:?}",
            run.bounds
        );
    }
}

/// [F11] Duplicate sync/notification ID does not double-stack the same toast
#[tokio::test]
async fn t2_f11_02_duplicate_notification_does_not_double_stack() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message(
            "window.showInformationMessage",
            &json!({ "message": "UniqueToastOnce" }),
        )
        .await;

    trigger_sync(&mut win);
    let scene1 = win.render();
    trigger_sync(&mut win);
    let scene2 = win.render();

    let count1 = scene1
        .glyph_runs()
        .iter()
        .filter(|g| g.text.contains("UniqueToastOnce"))
        .count();
    let count2 = scene2
        .glyph_runs()
        .iter()
        .filter(|g| g.text.contains("UniqueToastOnce"))
        .count();
    assert_eq!(count1, 1, "[F11] Notification must render exactly once, got {count1}");
    assert_eq!(
        count2, 1,
        "[F11] Repeated sync must not duplicate existing toast, got {count2}"
    );
}

/// [F11] Escape cancels QuickPick and InputBox resolving Null (JS undefined)
#[tokio::test]
async fn t2_f11_03_escape_cancels_quickpick_and_inputbox() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));
    bridge.window_manager.set_await_responses(true);

    // QuickPick Escape
    let b1 = Arc::clone(&bridge);
    let qp_task = tokio::spawn(async move {
        b1.handle_rpc_message(
            "window.showQuickPick",
            &json!({ "items": ["Alpha", "Beta"] }),
        )
        .await
    });
    for _ in 0..20 {
        if !bridge.window_manager.list_prompts().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    trigger_sync(&mut win);
    assert!(
        win.render().contains_text("Alpha"),
        "[F11] QuickPick must be visible before Escape"
    );
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    let qp_res = tokio::time::timeout(std::time::Duration::from_secs(2), qp_task)
        .await
        .expect("QuickPick cancel should resolve")
        .unwrap();
    assert_eq!(
        qp_res,
        Some(Value::Null),
        "[F11] Escape on QuickPick must resolve Null/undefined"
    );

    // InputBox Escape
    let b2 = Arc::clone(&bridge);
    let ib_task = tokio::spawn(async move {
        b2.handle_rpc_message(
            "window.showInputBox",
            &json!({ "options": { "prompt": "Input Cancel Test" } }),
        )
        .await
    });
    for _ in 0..20 {
        if bridge.window_manager.list_prompts().iter().any(|p| !p.resolved) {
            break;
        }
        tokio::task::yield_now().await;
    }
    trigger_sync(&mut win);
    assert!(
        win.render().contains_text("Input Cancel Test"),
        "[F11] InputBox must be visible before Escape"
    );
    win.type_text("partial");
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    let ib_res = tokio::time::timeout(std::time::Duration::from_secs(2), ib_task)
        .await
        .expect("InputBox cancel should resolve")
        .unwrap();
    assert_eq!(
        ib_res,
        Some(Value::Null),
        "[F11] Escape on InputBox must resolve Null/undefined"
    );
}

/// [F11] Empty and long multi-byte UTF-8 notification messages wrap/truncate safely
#[tokio::test]
async fn t2_f11_04_empty_and_long_utf8_notification_safe() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message("window.showInformationMessage", &json!({ "message": "" }))
        .await;
    let long_utf8 = "🚀 café 日本語 ".repeat(40);
    bridge
        .handle_rpc_message(
            "window.showErrorMessage",
            &json!({ "message": long_utf8, "items": ["OK"] }),
        )
        .await;

    trigger_sync(&mut win);
    let scene = win.render();
    assert!(
        scene.contains_text("OK") || scene.contains_text("café"),
        "[F11] Long UTF-8 notification must render safely without panic"
    );
    for quad in scene.quads() {
        assert!(
            quad.bounds.width().is_finite() && quad.bounds.height().is_finite(),
            "[F11] Long UTF-8 notification quads must have finite dimensions"
        );
    }
}

/// [F11] Post-resolution click and stale dismiss are safe no-ops
#[tokio::test]
async fn t2_f11_05_post_resolution_click_and_stale_dismiss_noop() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));
    bridge.window_manager.set_await_responses(true);

    let b = Arc::clone(&bridge);
    let task = tokio::spawn(async move {
        b.handle_rpc_message(
            "window.showInformationMessage",
            &json!({ "message": "OneShot", "items": ["Ack"] }),
        )
        .await
    });
    for _ in 0..20 {
        if !bridge.window_manager.active_notifications().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }

    let notif_id = bridge.window_manager.active_notifications()[0].id.clone();
    trigger_sync(&mut win);
    let scene = win.render();
    let ack_pt = scene
        .glyph_runs()
        .into_iter()
        .find(|g| g.text.contains("Ack"))
        .map(|g| Point::new(g.bounds.min_x() + 4.0, g.bounds.min_y() + 4.0))
        .expect("[F11] Ack button must render before click");

    win.click(ack_pt);
    let res = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("OneShot should resolve")
        .unwrap();
    assert_eq!(res, Some(json!("Ack")));

    // Stale programmatic resolve/dismiss and clicking old button position must be safe no-ops
    assert!(!bridge.window_manager.respond_notification(&notif_id, Some(json!("Ack"))));
    assert!(!bridge.window_manager.dismiss_notification(&notif_id));
    let _ = win.click(ack_pt);
    let _ = win.render();
}

// ============================================================================
// F12: Testing Sidebar View — Tier 2 Boundary & Negative
// ============================================================================

/// [F12] Empty Testing view renders 'No tests found' hint
#[test]
fn t2_f12_01_empty_testing_view_shows_no_tests_found() {
    let mut win = E2eWindow::new();
    let clicked = win.click_a11y_node("Testing")
        || win.click_scene_text("TST")
        || win.click_scene_text("TEST");
    assert!(clicked, "[F12] Testing ActivityBar item must be clickable");

    let scene = win.render();
    assert!(
        scene.contains_text("No tests found"),
        "[F12] Empty Testing sidebar must show 'No tests found', got: {}",
        scene.all_text()
    );
}

/// [F12] Disposed test controller is hidden from Testing sidebar
#[tokio::test]
async fn t2_f12_02_disposed_controller_is_hidden() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "ephemeral", "label": "Ephemeral Suite" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "ephemeral",
                "items": [{ "id": "ephemeral_test", "label": "ephemeral_test", "children": [] }]
            }),
        )
        .await;

    let _ = win.click_a11y_node("Testing") || win.click_scene_text("TST") || win.click_scene_text("TEST");
    trigger_sync(&mut win);
    assert!(
        win.render().contains_text("ephemeral_test"),
        "[F12] Active controller item must appear before disposal"
    );

    bridge
        .handle_rpc_message("tests.disposeController", &json!({ "id": "ephemeral" }))
        .await;
    trigger_sync(&mut win);
    let after_dispose = win.render();
    assert!(
        !after_dispose.contains_text("Ephemeral Suite")
            && !after_dispose.contains_text("ephemeral_test"),
        "[F12] Disposed controller and its tests must be hidden, got: {}",
        after_dispose.all_text()
    );
}

/// [F12] Deeply nested test items (depth >= 12) render cleanly without stack overflow
#[tokio::test]
async fn t2_f12_03_deeply_nested_test_items_render_safely() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    // Build depth 14 nested ExtTestItemState JSON
    let mut node = json!({
        "id": "deep_13",
        "label": "deep_leaf_13",
        "children": []
    });
    for d in (0..13).rev() {
        node = json!({
            "id": format!("deep_{d}"),
            "label": format!("deep_node_{d}"),
            "children": [node]
        });
    }

    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "deep-ctrl", "label": "Deep Tests" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({ "controllerId": "deep-ctrl", "items": [node] }),
        )
        .await;

    let _ = win.click_a11y_node("Testing") || win.click_scene_text("TST") || win.click_scene_text("TEST");
    trigger_sync(&mut win);
    let scene = win.render();
    let _tree = win.accessibility_tree();
    assert!(
        scene.contains_text("deep_node_0"),
        "[F12] Deeply nested test hierarchy must render without stack overflow, got: {}",
        scene.all_text()
    );
}

/// [F12] Clicking a test item without a URI selects it safely without panic or opening a tab
#[tokio::test]
async fn t2_f12_04_clicking_test_item_without_uri_safe() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "no-uri", "label": "NoURI" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "no-uri",
                "items": [{ "id": "virtual_case", "label": "virtual_case", "children": [] }]
            }),
        )
        .await;

    let _ = win.click_a11y_node("Testing") || win.click_scene_text("TST") || win.click_scene_text("TEST");
    trigger_sync(&mut win);
    let _ = win.render();

    let clicked = win.click_a11y_node("virtual_case") || win.click_scene_text("virtual_case");
    assert!(clicked, "[F12] Virtual test item without URI must be selectable");
    assert!(
        win.window
            .view
            .editor_area
            .model
            .current_group_ref()
            .tabs
            .is_empty(),
        "[F12] Clicking item without URI must not open spurious editor tabs"
    );
}

/// [F12] Windows file:///C:/... and POSIX file:///... URIs decode cleanly in test item click
#[tokio::test]
async fn t2_f12_05_windows_and_posix_uri_decoding() {
    let win_decoded = file_uri_to_path("file:///C:/Users/dev/my%20spec.rs");
    let win_str = win_decoded.to_string_lossy().replace('\\', "/");
    assert!(
        win_str.ends_with("C:/Users/dev/my spec.rs"),
        "[F12] Windows file URI must decode cleanly: {win_str}"
    );
    let posix_decoded = file_uri_to_path("file:///home/dev/my%20spec.rs");
    assert_eq!(
        posix_decoded.to_string_lossy().replace('\\', "/"),
        "/home/dev/my spec.rs",
        "[F12] POSIX file URI must decode cleanly"
    );

    let ws = TempWorkspace::new();
    let spec = ws.write_file("tests/uri case.rs", "#[test]\nfn uri_case() {}\n");
    let uri = path_to_file_uri(&spec);
    assert!(paths_equivalent(&file_uri_to_path(&uri), &spec));

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message("tests.createController", &json!({ "id": "u", "label": "U" }))
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "u",
                "items": [{ "id": "uri_case", "label": "uri_case", "uri": uri, "children": [] }]
            }),
        )
        .await;

    let _ = win.click_a11y_node("Testing") || win.click_scene_text("TST") || win.click_scene_text("TEST");
    trigger_sync(&mut win);
    let _ = win.render();
    let clicked = win.click_a11y_node("uri_case") || win.click_scene_text("uri_case");
    assert!(clicked, "[F12] Test item with encoded URI must be clickable");
    assert!(
        win.read_doc(&spec).is_some(),
        "[F12] Clicking test item with encoded file:/// URI must open file"
    );
}

// ============================================================================
// F13: Interactive File Explorer CRUD — Tier 2 Boundary & Negative
// ============================================================================

/// [F13] Rejects empty and whitespace-only names without touching disk
#[test]
fn t2_f13_01_rejects_empty_and_whitespace_names() {
    let ws = TempWorkspace::new();
    ws.write_file("seed.rs", "fn seed() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.window.view.focused_region = FocusedRegion::Sidebar;
    let _ = win.render();

    for bad_input in ["", "   "] {
        let clicked = win.click_a11y_node("New File")
            || win.click_scene_text("+F")
            || win.click_scene_text("New File");
        assert!(clicked, "[F13] New File button must be clickable");
        if !bad_input.is_empty() {
            win.type_text(bad_input);
        }
        win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
        win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    }

    let entries: Vec<_> = std::fs::read_dir(ws.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["seed.rs".to_string()],
        "[F13] Empty/whitespace names must not create any files on disk"
    );
}

/// [F13] Rejects path separators (/ and \) and NUL byte (\0)
#[test]
fn t2_f13_02_rejects_path_separators_and_nul() {
    let ws = TempWorkspace::new();
    ws.write_file("anchor.rs", "fn anchor() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.window.view.focused_region = FocusedRegion::Sidebar;
    let _ = win.render();

    for bad_name in ["sub/evil.rs", "sub\\evil.rs", "bad\0name.rs"] {
        let clicked = win.click_a11y_node("New File")
            || win.click_scene_text("+F")
            || win.click_scene_text("New File");
        assert!(clicked, "[F13] New File button must be clickable");
        win.type_text(bad_name);
        win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
        win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    }

    assert!(!ws.exists("sub"));
    assert!(!ws.exists("sub/evil.rs"));
    assert!(!ws.exists("bad"));
    let entries: Vec<_> = std::fs::read_dir(ws.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["anchor.rs".to_string()],
        "[F13] Path separators and NUL names must not create any files on disk"
    );
}

/// [F13] Rejects '.' / '..' and Windows invalid filename characters (< > : " | ? *)
#[test]
fn t2_f13_03_rejects_dot_dotdot_and_windows_invalid_chars() {
    let ws = TempWorkspace::new();
    let keep = ws.write_file("valid.rs", "fn valid() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.window.view.focused_region = FocusedRegion::Sidebar;
    win.window.view.explorer.model.selected_path = Some(keep);
    let _ = win.render();

    for bad_name in [
        ".",
        "..",
        "bad<name>.rs",
        "a:b.rs",
        "a\"b.rs",
        "a|b?.rs",
        "a*b.rs",
    ] {
        assert!(
            win.dispatch_key(KeyCode::F2, Modifiers::none(), None)
                .is_consumed(),
            "[F13] F2 must enter inline rename mode"
        );
        win.type_text(bad_name);
        win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
        win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
        assert!(
            ws.exists("valid.rs"),
            "[F13] Invalid rename '{bad_name}' must leave 'valid.rs' untouched"
        );
    }
}

/// [F13] Prevents clobbering an existing file on Create and Rename
#[test]
fn t2_f13_04_prevents_clobbering_existing_file_on_create_and_rename() {
    let ws = TempWorkspace::new();
    ws.write_file("existing.rs", "// preserve me\n");
    let other = ws.write_file("other.rs", "// other\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.window.view.focused_region = FocusedRegion::Sidebar;
    let _ = win.render();

    // Attempt Create with existing name
    let clicked = win.click_a11y_node("New File")
        || win.click_scene_text("+F")
        || win.click_scene_text("New File");
    assert!(clicked, "[F13] New File button must be clickable");
    win.type_text("existing.rs");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    assert_eq!(
        ws.read_file("existing.rs"),
        "// preserve me\n",
        "[F13] Create colliding with existing.rs must not clobber contents"
    );

    // Attempt Rename other.rs -> existing.rs
    win.window.view.explorer.model.selected_path = Some(other);
    win.dispatch_key(KeyCode::F2, Modifiers::none(), None);
    win.type_text("existing.rs");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);

    assert_eq!(
        ws.read_file("existing.rs"),
        "// preserve me\n",
        "[F13] Rename colliding with existing.rs must not clobber existing.rs"
    );
    assert!(
        ws.exists("other.rs"),
        "[F13] Colliding rename must keep 'other.rs' intact"
    );
}

/// [F13] Escape cancels inline New File, Rename, and ConfirmDelete without mutating disk
#[test]
fn t2_f13_05_escape_cancels_new_rename_and_delete() {
    let ws = TempWorkspace::new();
    let keep = ws.write_file("keep.rs", "// intact\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.window.view.focused_region = FocusedRegion::Sidebar;
    let _ = win.render();

    // Cancel New File
    let clicked = win.click_a11y_node("New File")
        || win.click_scene_text("+F")
        || win.click_scene_text("New File");
    assert!(clicked, "[F13] New File button must be clickable");
    win.type_text("ghost.rs");
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    assert!(!ws.exists("ghost.rs"), "[F13] Escape must cancel New File");

    // Cancel Rename
    win.window.view.explorer.model.selected_path = Some(keep.clone());
    win.dispatch_key(KeyCode::F2, Modifiers::none(), None);
    win.type_text("changed.rs");
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    assert!(
        ws.exists("keep.rs") && !ws.exists("changed.rs"),
        "[F13] Escape must cancel Rename"
    );

    // Cancel Delete
    win.window.view.explorer.model.selected_path = Some(keep);
    win.dispatch_key(KeyCode::Delete, Modifiers::none(), None);
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    assert!(ws.exists("keep.rs"), "[F13] Escape must cancel Delete");
}

/// [F13] Directory deletion requires explicit confirmation before remove_dir_all
#[test]
fn t2_f13_06_directory_delete_requires_confirmation() {
    let ws = TempWorkspace::new();
    ws.write_file("pkg/mod.rs", "pub fn x() {}\n");
    let pkg_dir = ws.path().join("pkg");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.window.view.focused_region = FocusedRegion::Sidebar;
    win.window.view.explorer.model.selected_path = Some(pkg_dir.clone());
    let _ = win.render();

    win.dispatch_key(KeyCode::Delete, Modifiers::none(), None);
    assert!(
        pkg_dir.is_dir(),
        "[F13] Directory must NOT be deleted before confirmation"
    );
    let scene = win.render();
    assert!(
        scene.contains_text("Delete") || scene.contains_text("Confirm"),
        "[F13] Directory Delete must show confirmation prompt, got: {}",
        scene.all_text()
    );

    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert!(
        !pkg_dir.exists(),
        "[F13] Confirming Delete must remove directory and contents"
    );
}

// ============================================================================
// F14: Contributed TreeViews — Tier 2 Boundary & Negative
// ============================================================================

/// [F14] 'when' clause evaluating false hides contributed TreeView section
#[tokio::test]
async fn t2_f14_01_when_false_hides_contributed_section() {
    let ws = TempWorkspace::new();
    let loader = setup_treeview_ext(&ws);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, rpc) = create_bridge_with_root(ws.path());
    let bridge = Arc::new(
        LanguageFeatureBridge::new(
            Arc::clone(&bridge.provider_registry),
            Arc::clone(&rpc),
            Arc::clone(&bridge.lsp_manager),
        )
        .with_extension_loader(&loader)
        .with_fs_root(ws.path()),
    );
    win.window.set_language_bridge(Arc::clone(&bridge));
    win.window
        .context_keys
        .set_bool("workspaceHasProjects", true);

    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TreeDataProvider", "id": "projectOutlineView" }),
        )
        .await;

    trigger_sync(&mut win);
    assert!(
        win.render().contains_text("Project Outline"),
        "[F14] Contributed section must render when 'when' is true before testing false"
    );

    win.window
        .context_keys
        .set_bool("workspaceHasProjects", false);
    trigger_sync(&mut win);
    let scene = win.render();
    assert!(
        !scene.contains_text("Project Outline"),
        "[F14] Contributed section must stay hidden when 'when' flips to false, got: {}",
        scene.all_text()
    );
}

/// [F14] Malformed 'when' clause hides contributed view safely without panicking
#[test]
fn t2_f14_02_malformed_when_clause_hides_safely() {
    let ws = TempWorkspace::new();
    let ext_dir = ws.path().join(".ext/bad_when_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(
        ext_dir.join("package.json"),
        serde_json::to_string_pretty(&json!({
            "name": "bad-when-ext",
            "publisher": "test",
            "version": "0.0.1",
            "contributes": {
                "views": {
                    "explorer": [
                        {
                            "id": "validWhenView",
                            "name": "Valid When View",
                            "when": "workspaceHasProjects"
                        },
                        {
                            "id": "brokenWhenView",
                            "name": "Broken When View",
                            "when": "(((invalid && || !!"
                        }
                    ]
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).unwrap();

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, rpc) = create_bridge_with_root(ws.path());
    let bridge = Arc::new(
        LanguageFeatureBridge::new(
            Arc::clone(&bridge.provider_registry),
            Arc::clone(&rpc),
            Arc::clone(&bridge.lsp_manager),
        )
        .with_extension_loader(&loader)
        .with_fs_root(ws.path()),
    );
    win.window.set_language_bridge(bridge);
    win.window
        .context_keys
        .set_bool("workspaceHasProjects", true);

    trigger_sync(&mut win);
    let scene = win.render();
    let _tree = win.accessibility_tree();
    assert!(
        scene.contains_text("Valid When View"),
        "[F14] Valid sibling contributed view must render, got: {}",
        scene.all_text()
    );
    assert!(
        !scene.contains_text("Broken When View"),
        "[F14] Malformed 'when' clause must safely hide view without panicking"
    );
}

/// [F14] TreeItem without explicit 'id' works via synthesized fallback ID
#[tokio::test]
async fn t2_f14_03_tree_item_without_explicit_id_works() {
    let ws = TempWorkspace::new();
    let loader = setup_treeview_ext(&ws);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, rpc) = create_bridge_with_root(ws.path());
    let bridge = Arc::new(
        LanguageFeatureBridge::new(
            Arc::clone(&bridge.provider_registry),
            Arc::clone(&rpc),
            Arc::clone(&bridge.lsp_manager),
        )
        .with_extension_loader(&loader)
        .with_fs_root(ws.path()),
    );
    win.window.set_language_bridge(Arc::clone(&bridge));
    win.window
        .context_keys
        .set_bool("workspaceHasProjects", true);

    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TreeDataProvider", "id": "projectOutlineView" }),
        )
        .await;

    trigger_sync(&mut win);
    for _ in 0..20 {
        if rpc.pending_count() > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    let _ = rpc.handle_incoming_response(
        1,
        Some(json!([
            { "label": "FallbackIdItem", "collapsibleState": 0 }
        ])),
        None,
    );
    tokio::task::yield_now().await;
    trigger_sync(&mut win);

    let scene = win.render();
    assert!(
        scene.contains_text("FallbackIdItem"),
        "[F14] TreeItem without explicit 'id' must render via fallback ID, got: {}",
        scene.all_text()
    );
    assert!(
        win.click_a11y_node("FallbackIdItem") || win.click_scene_text("FallbackIdItem"),
        "[F14] FallbackIdItem must be clickable"
    );
}

/// [F14] 'TreeData' provider alias & Windows backslash resourceUri basename render cleanly
#[tokio::test]
async fn t2_f14_04_treedata_alias_and_windows_backslash_resource_uri() {
    let ws = TempWorkspace::new();
    let loader = setup_treeview_ext(&ws);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, rpc) = create_bridge_with_root(ws.path());
    let bridge = Arc::new(
        LanguageFeatureBridge::new(
            Arc::clone(&bridge.provider_registry),
            Arc::clone(&rpc),
            Arc::clone(&bridge.lsp_manager),
        )
        .with_extension_loader(&loader)
        .with_fs_root(ws.path()),
    );
    win.window.set_language_bridge(Arc::clone(&bridge));
    win.window
        .context_keys
        .set_bool("workspaceHasProjects", true);

    let reg_res = bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TreeData", "id": "projectOutlineView" }),
        )
        .await;
    assert!(
        reg_res.is_some(),
        "[F14] 'TreeData' alias must register successfully"
    );

    trigger_sync(&mut win);
    for _ in 0..20 {
        if rpc.pending_count() > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    let _ = rpc.handle_incoming_response(
        1,
        Some(json!([
            {
                "id": "win_uri_item",
                "resourceUri": "C:\\Users\\dev\\repo\\src\\win_entry.rs",
                "collapsibleState": 0
            }
        ])),
        None,
    );
    tokio::task::yield_now().await;
    trigger_sync(&mut win);

    let scene = win.render();
    assert!(
        scene.contains_text("win_entry.rs"),
        "[F14] TreeItem with Windows backslash resourceUri must display basename 'win_entry.rs', got: {}",
        scene.all_text()
    );
}

/// [F14] Empty getChildren response renders clean empty state without crashing
#[tokio::test]
async fn t2_f14_05_empty_get_children_renders_clean_empty_state() {
    let ws = TempWorkspace::new();
    let loader = setup_treeview_ext(&ws);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, rpc) = create_bridge_with_root(ws.path());
    let bridge = Arc::new(
        LanguageFeatureBridge::new(
            Arc::clone(&bridge.provider_registry),
            Arc::clone(&rpc),
            Arc::clone(&bridge.lsp_manager),
        )
        .with_extension_loader(&loader)
        .with_fs_root(ws.path()),
    );
    win.window.set_language_bridge(Arc::clone(&bridge));
    win.window
        .context_keys
        .set_bool("workspaceHasProjects", true);

    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TreeDataProvider", "id": "projectOutlineView" }),
        )
        .await;

    trigger_sync(&mut win);
    for _ in 0..20 {
        if rpc.pending_count() > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    let _ = rpc.handle_incoming_response(1, Some(json!([])), None);
    tokio::task::yield_now().await;
    trigger_sync(&mut win);

    let scene = win.render();
    let _tree = win.accessibility_tree();
    assert!(
        scene.contains_text("Project Outline"),
        "[F14] Empty getChildren must still render 'Project Outline' header cleanly"
    );
}

// ============================================================================
// F16: Additional Language Tokenizers — Tier 2 Boundary & Negative
// ============================================================================

/// [F16] Empty and whitespace-only lines yield 0 tokens across all 7 languages
#[test]
fn t2_f16_01_empty_and_whitespace_lines_yield_zero_tokens() {
    let reg = GrammarRegistry::new();
    let theme = ThemeService::default_dark_theme();

    for lang in ["go", "c", "cpp", "html", "css", "toml", "yaml"] {
        let grammar = reg
            .get_by_id(lang)
            .unwrap_or_else(|| panic!("[F16] Missing grammar '{lang}'"));
        let mut hl = SyntaxHighlighter::new(grammar, theme.clone());
        assert!(
            hl.get_line_tokens(0, "").is_empty(),
            "[F16] {lang} empty line must yield 0 tokens"
        );
        assert!(
            hl.get_line_tokens(1, "   \t  ").is_empty(),
            "[F16] {lang} whitespace-only line must yield 0 tokens"
        );
    }
}

/// [F16] Unclosed block comments, HTML comments, Go backticks, TOML """, and C++ R"(...)" propagate state
#[test]
fn t2_f16_02_unclosed_multiline_constructs_propagate_grammar_state() {
    let reg = GrammarRegistry::new();
    let theme = ThemeService::default_dark_theme();

    // Block comments /* ... */ in Go, C, C++, CSS
    for lang in ["go", "c", "cpp", "css"] {
        let g = reg
            .get_by_id(lang)
            .unwrap_or_else(|| panic!("[F16] Missing grammar '{lang}'"));
        let mut hl = SyntaxHighlighter::new(g, theme.clone());
        let _ = hl.get_line_tokens(0, "/* start multiline comment");
        let mid = hl.get_line_tokens(1, "still inside block comment");
        assert!(
            mid.iter()
                .any(|s| matches!(s.scope, SyntaxScope::Comment | SyntaxScope::CommentBlock)),
            "[F16] {lang} must propagate block comment state to line 1"
        );
    }

    // HTML <!-- ... -->
    let mut html_hl =
        SyntaxHighlighter::new(reg.get_by_id("html").expect("[F16] Missing html"), theme.clone());
    let _ = html_hl.get_line_tokens(0, "<!-- open comment");
    let html_mid = html_hl.get_line_tokens(1, "middle of html comment");
    assert!(
        html_mid
            .iter()
            .any(|s| matches!(s.scope, SyntaxScope::Comment | SyntaxScope::CommentBlock)),
        "[F16] HTML must propagate <!-- comment state across lines"
    );

    // Go raw backtick string `...`
    let mut go_hl =
        SyntaxHighlighter::new(reg.get_by_id("go").expect("[F16] Missing go"), theme.clone());
    let _ = go_hl.get_line_tokens(0, "s := `raw line 1");
    let go_mid = go_hl.get_line_tokens(1, "raw line 2");
    assert!(
        go_mid.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::String | SyntaxScope::StringQuoted | SyntaxScope::StringTemplate
        )),
        "[F16] Go must propagate backtick raw string state across lines"
    );

    // TOML multiline string """..."""
    let mut toml_hl =
        SyntaxHighlighter::new(reg.get_by_id("toml").expect("[F16] Missing toml"), theme.clone());
    let _ = toml_hl.get_line_tokens(0, "desc = \"\"\"first line");
    let toml_mid = toml_hl.get_line_tokens(1, "second line");
    assert!(
        toml_mid.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::String | SyntaxScope::StringQuoted | SyntaxScope::StringTemplate
        )),
        "[F16] TOML must propagate triple-quoted string state across lines"
    );

    // C++ raw string R"(...)"
    let mut cpp_hl =
        SyntaxHighlighter::new(reg.get_by_id("cpp").expect("[F16] Missing cpp"), theme);
    let _ = cpp_hl.get_line_tokens(0, "const char* s = R\"(line1");
    let cpp_mid = cpp_hl.get_line_tokens(1, "line2");
    assert!(
        cpp_mid.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::String | SyntaxScope::StringQuoted | SyntaxScope::StringTemplate
        )),
        "[F16] C++ must propagate R\"(...)\" raw string state across lines"
    );
}

/// [F16] Multi-byte UTF-8 emojis preserve valid char boundaries on every TokenSpan
#[test]
fn t2_f16_03_multibyte_utf8_preserves_char_boundaries() {
    let reg = GrammarRegistry::new();
    let theme = ThemeService::default_dark_theme();

    let cases = [
        ("go", "msg := \"🚀 café 日本語\" // comment 🚀"),
        ("c", "const char* s = \"🚀 café\"; /* 🚀 */"),
        ("cpp", "std::string s = \"🚀 café\"; // 日本語"),
        ("html", "<p title=\"🚀 café\">Hello 🚀 日本語</p>"),
        ("css", "/* 🚀 café */ .btn { content: \"🚀\"; }"),
        ("toml", "title = \"🚀 café 日本語\" # 🚀"),
        ("yaml", "greeting: \"🚀 café 日本語\" # 🚀"),
    ];

    for (lang, line) in cases {
        let g = reg
            .get_by_id(lang)
            .unwrap_or_else(|| panic!("[F16] Missing grammar '{lang}'"));
        let mut hl = SyntaxHighlighter::new(g, theme.clone());
        let spans = hl.get_line_tokens(0, line);
        assert!(!spans.is_empty(), "[F16] {lang} must emit tokens for UTF-8 line");
        for span in spans {
            assert!(
                span.start <= span.end
                    && span.end <= line.len()
                    && line.is_char_boundary(span.start)
                    && line.is_char_boundary(span.end),
                "[F16] {lang} span {:?} violates UTF-8 char boundaries on {:?}",
                span,
                line
            );
        }
    }
}

/// [F16] CRLF (\r\n / trailing \r) line endings do not corrupt tokens
#[test]
fn t2_f16_04_crlf_line_endings_do_not_corrupt_tokens() {
    let reg = GrammarRegistry::new();
    let theme = ThemeService::default_dark_theme();

    let cases = [
        ("go", "return 42\r"),
        ("c", "return 0;\r"),
        ("cpp", "constexpr int x = 9;\r"),
        ("html", "<div>ok</div>\r"),
        ("css", "color: red;\r"),
        ("toml", "port = 8080\r"),
        ("yaml", "port: 8080\r"),
    ];

    for (lang, line) in cases {
        let g = reg
            .get_by_id(lang)
            .unwrap_or_else(|| panic!("[F16] Missing grammar '{lang}'"));
        let mut hl = SyntaxHighlighter::new(g, theme.clone());
        let spans = hl.get_line_tokens(0, line);
        assert!(
            spans.iter().any(|s| s.scope != SyntaxScope::Plain),
            "[F16] {lang} must classify non-Plain tokens even with trailing \\r"
        );
        for span in spans {
            assert!(
                span.end <= line.len()
                    && line.is_char_boundary(span.start)
                    && line.is_char_boundary(span.end),
                "[F16] {lang} CRLF span out of bounds: {:?}",
                span
            );
        }
    }
}

/// [F16] Unknown file extension falls back cleanly to plaintext
#[test]
fn t2_f16_05_unknown_extension_falls_back_to_plaintext() {
    let reg = GrammarRegistry::new();
    assert_eq!(
        reg.get_for_path(Path::new("archive.unknownext99"))
            .language_id(),
        "plaintext",
        "[F16] Unknown extension must fall back to 'plaintext'"
    );
    assert_eq!(
        reg.get_for_path(Path::new("Makefile_no_ext")).language_id(),
        "plaintext",
        "[F16] Extensionless unknown file must fall back to 'plaintext'"
    );
    assert_eq!(
        TextDocument::new("hello", None, None).language_id,
        "plaintext",
        "[F16] TextDocument without path or language_id must default to 'plaintext'"
    );
}
