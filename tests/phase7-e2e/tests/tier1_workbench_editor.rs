use phase7_e2e::{create_bridge_with_root, repo_root, E2eWindow, TempWorkspace};
use rustcode_editor::{GrammarRegistry, Position, SyntaxHighlighter, SyntaxScope, TextDocument};
use rustcode_ext_host::{ExtensionLoader, LanguageFeatureBridge};
use rustcode_platform::accessibility::AccessibleRole;
use rustcode_platform::fs::path_to_file_uri;
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::render::KeyCode;
use rustcode_platform::theme::ThemeService;
use rustcode_workbench::quickinput::{
    CommandPalette, CommandPaletteView, PaletteItem, PaletteItemKind, QuickInputMode,
};
use rustcode_workbench::workbench_view::FocusedRegion;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
// F9: In-Editor Find & Replace (Ctrl+F / Ctrl+H) — Tier 1 Happy Path
// ============================================================================

/// [F9] Ctrl+F opens Find widget in Scene & AccessibilityTree and highlights matches
#[test]
fn t1_f9_01_ctrl_f_opens_find_widget_and_highlights_matches() {
    let ws = TempWorkspace::new();
    ws.write_file("src/main.rs", "let foo = 1;\nlet bar = foo + foo;\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("src/main.rs");
    let base_quads = win.render().quads().len();

    let consumed = win.dispatch_ctrl_char('f').is_consumed();
    assert!(consumed, "[F9] Ctrl+F must be consumed to open Find widget");

    win.type_text("foo");
    let scene = win.render();
    assert!(
        scene.contains_text("1 of 3"),
        "[F9] Find widget must render '1 of 3' for 3 matches, scene text: {}",
        scene.all_text()
    );
    assert!(
        scene.quads().len() > base_quads,
        "[F9] Find widget & match highlights must add quads to Scene"
    );

    let tree = win.accessibility_tree();
    let has_find_a11y = tree.to_flat_list().iter().any(|n| {
        let id_l = n.id.to_ascii_lowercase();
        let name_l = n.name.to_ascii_lowercase();
        (id_l.contains("find") || name_l.contains("find"))
            && matches!(
                n.role,
                AccessibleRole::Dialog | AccessibleRole::TextInput | AccessibleRole::Panel
            )
    });
    assert!(
        has_find_a11y,
        "[F9] AccessibilityTree must expose Find widget nodes"
    );
}

/// [F9] Enter/F3 and Shift+Enter/Shift+F3 navigate forward and backward through matches
#[test]
fn t1_f9_02_enter_f3_and_shift_navigation() {
    let ws = TempWorkspace::new();
    ws.write_file("lines.txt", "alpha one\nalpha two\nalpha three\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("lines.txt");
    assert!(win.dispatch_ctrl_char('f').is_consumed(), "[F9] Ctrl+F must open find");
    win.type_text("alpha");

    let s1 = win.render();
    assert!(
        s1.contains_text("1 of 3"),
        "[F9] Initial match must be 1 of 3, got: {}",
        s1.all_text()
    );

    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    let s2 = win.render();
    assert!(
        s2.contains_text("2 of 3"),
        "[F9] Enter must advance to 2 of 3, got: {}",
        s2.all_text()
    );
    assert_eq!(win.active_editor().unwrap().primary_position().line, 1);

    win.dispatch_key(KeyCode::F3, Modifiers::none(), None);
    let s3 = win.render();
    assert!(
        s3.contains_text("3 of 3"),
        "[F9] F3 must advance to 3 of 3, got: {}",
        s3.all_text()
    );
    assert_eq!(win.active_editor().unwrap().primary_position().line, 2);

    assert!(
        dispatch_shift_key(&mut win, KeyCode::F3),
        "[F9] Shift+F3 must be consumed by Find widget"
    );
    let s2_back = win.render();
    assert!(
        s2_back.contains_text("2 of 3"),
        "[F9] Shift+F3 must move back to 2 of 3, got: {}",
        s2_back.all_text()
    );
    assert_eq!(win.active_editor().unwrap().primary_position().line, 1);

    dispatch_shift_key(&mut win, KeyCode::Enter);
    let s1_back = win.render();
    assert!(
        s1_back.contains_text("1 of 3"),
        "[F9] Shift+Enter must move back to 1 of 3, got: {}",
        s1_back.all_text()
    );
    assert_eq!(win.active_editor().unwrap().primary_position().line, 0);
}

/// [F9] Match Case (Aa), Whole Word (ab), and Regex (.*) toggles filter matches accurately
#[test]
fn t1_f9_03_case_whole_word_and_regex_toggles() {
    let ws = TempWorkspace::new();
    ws.write_file("tokens.txt", "Foo foo foobar f123 f99\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("tokens.txt");
    assert!(win.dispatch_ctrl_char('f').is_consumed(), "[F9] Ctrl+F must open find");
    win.type_text("foo");

    let s_default = win.render();
    assert!(
        s_default.contains_text("1 of 3"),
        "[F9] Default case-insensitive find for 'foo' must match 3 tokens, got: {}",
        s_default.all_text()
    );

    let toggled_case = win.click_a11y_node("Match Case") || win.click_scene_text("Aa");
    assert!(toggled_case, "[F9] Match Case (Aa) toggle must be clickable");
    let s_case = win.render();
    assert!(
        s_case.contains_text("1 of 2"),
        "[F9] Match Case must narrow matches to 'foo' & 'foobar' (2), got: {}",
        s_case.all_text()
    );

    let toggled_word = win.click_a11y_node("Whole Word") || win.click_scene_text("ab");
    assert!(toggled_word, "[F9] Whole Word (ab) toggle must be clickable");
    let s_word = win.render();
    assert!(
        s_word.contains_text("1 of 1"),
        "[F9] Match Case + Whole Word must match only 'foo' (1), got: {}",
        s_word.all_text()
    );

    // Disable Aa & ab, enable Regex .* and search f[0-9]+
    assert!(
        win.click_a11y_node("Match Case") || win.click_scene_text("Aa"),
        "[F9] Match Case toggle must be clickable to disable"
    );
    assert!(
        win.click_a11y_node("Whole Word") || win.click_scene_text("ab"),
        "[F9] Whole Word toggle must be clickable to disable"
    );
    let toggled_regex =
        win.click_a11y_node("Use Regular Expression") || win.click_scene_text(".*");
    assert!(toggled_regex, "[F9] Regex (.*) toggle must be clickable");

    for _ in 0..3 {
        win.dispatch_key(KeyCode::Backspace, Modifiers::none(), None);
    }
    win.type_text("f[0-9]+");
    let s_regex = win.render();
    assert!(
        s_regex.contains_text("1 of 2"),
        "[F9] Regex 'f[0-9]+' must match 'f123' and 'f99' (2), got: {}",
        s_regex.all_text()
    );
}

/// [F9] Ctrl+H opens Replace mode and supports Replace single and Replace All
#[test]
fn t1_f9_04_ctrl_h_replace_single_and_replace_all() {
    let ws = TempWorkspace::new();
    ws.write_file("pets.txt", "one cat\ntwo cat\nthree cat\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("pets.txt");
    assert!(
        win.dispatch_ctrl_char('h').is_consumed(),
        "[F9] Ctrl+H must open Find & Replace"
    );

    win.type_text("cat");
    win.dispatch_key(KeyCode::Tab, Modifiers::none(), None);
    win.type_text("dog");

    let replaced_one = win.click_a11y_node("Replace") || win.click_scene_text("Replace");
    assert!(replaced_one, "[F9] Replace button must be clickable");
    let mid = win.active_editor_text().unwrap();
    assert_eq!(
        mid.matches("dog").count(),
        1,
        "[F9] Single Replace must replace 1 occurrence, got:\n{mid}"
    );
    assert_eq!(mid.matches("cat").count(), 2);

    let replaced_all = win.click_a11y_node("Replace All")
        || win.click_scene_text("Replace All")
        || win.click_scene_text("All");
    assert!(replaced_all, "[F9] Replace All button must be clickable");
    let final_text = win.active_editor_text().unwrap();
    assert_eq!(
        final_text, "one dog\ntwo dog\nthree dog\n",
        "[F9] Replace All must replace remaining occurrences"
    );
}

/// [F9] Ctrl+Z performs atomic undo of Replace All and Escape dismisses Find widget
#[test]
fn t1_f9_05_atomic_undo_replace_all_and_escape_dismiss() {
    let ws = TempWorkspace::new();
    let original = "let v = old + old + old;\n";
    ws.write_file("undo.rs", original);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("undo.rs");
    assert!(win.dispatch_ctrl_char('h').is_consumed(), "[F9] Ctrl+H must open replace");
    win.type_text("old");
    win.dispatch_key(KeyCode::Tab, Modifiers::none(), None);
    win.type_text("new");

    let clicked = win.click_a11y_node("Replace All")
        || win.click_scene_text("Replace All")
        || win.click_scene_text("All");
    assert!(clicked, "[F9] Replace All must be clickable");
    assert_eq!(
        win.active_editor_text().unwrap(),
        "let v = new + new + new;\n"
    );

    // Escape closes Find widget and returns focus to editor
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    let after_esc = win.render();
    assert!(
        !after_esc.contains_text("Replace All") && !after_esc.contains_text("No results"),
        "[F9] Escape must dismiss Find & Replace widget"
    );

    // Single Ctrl+Z reverts all 3 replacements atomically
    win.dispatch_ctrl_char('z');
    assert_eq!(
        win.active_editor_text().unwrap(),
        original,
        "[F9] Single Ctrl+Z must atomically undo Replace All"
    );
}

/// [F9] Escape restores focus to editor buffer so typing inserts into buffer and re-find updates
#[test]
fn t1_f9_06_escape_restores_editor_focus_and_refind_updates() {
    let ws = TempWorkspace::new();
    ws.write_file("edit.txt", "target line\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("edit.txt");
    assert!(win.dispatch_ctrl_char('f').is_consumed(), "[F9] Ctrl+F must open find");
    win.type_text("target");
    assert!(win.render().contains_text("1 of 1"));

    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    win.type_text("X");
    let text = win.active_editor_text().unwrap();
    assert!(
        text.contains('X'),
        "[F9] After Escape, typing must edit document buffer rather than find input: {text:?}"
    );
}

// ============================================================================
// F10: QuickInput :line[:col] (Ctrl+G) & @symbol (Ctrl+Shift+O) — Tier 1
// ============================================================================

/// [F10] Ctrl+G opens : mode and jumps cursor to 0-based (line-1, col-1)
#[test]
fn t1_f10_01_ctrl_g_opens_colon_mode_and_jumps_line_col() {
    let ws = TempWorkspace::new();
    ws.write_file("nav.rs", "line1\nline2\nline3_abcdef\nline4\nline5\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("nav.rs");

    assert!(
        win.dispatch_ctrl_char('g').is_consumed(),
        "[F10] Ctrl+G must open QuickInput Go to Line mode"
    );
    let pal_scene = win.render();
    assert!(
        pal_scene.contains_text(":")
            || win
                .accessibility_tree()
                .find_by_id("workbench.commandPalette")
                .is_some(),
        "[F10] Ctrl+G must render QuickInput palette"
    );

    win.type_text("3:5");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    assert_eq!(
        win.active_editor().unwrap().primary_position(),
        Position::new(2, 4),
        "[F10] :3:5 must jump cursor to 0-based (2, 4)"
    );
    assert!(
        !win.window.view.is_command_palette_open,
        "[F10] Pressing Enter must close QuickInput"
    );
}

/// [F10] :line without column jumps to column 0 of that line
#[test]
fn t1_f10_02_colon_line_without_col_jumps_to_col_zero() {
    let ws = TempWorkspace::new();
    ws.write_file("nav2.rs", "alpha\nbeta\ngamma\ndelta\nepsilon\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("nav2.rs");
    win.window
        .view
        .editor_area
        .active_editor_mut()
        .unwrap()
        .set_primary_position(Position::new(0, 3));

    assert!(win.dispatch_ctrl_char('g').is_consumed(), "[F10] Ctrl+G must open");
    win.type_text("4");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    assert_eq!(
        win.active_editor().unwrap().primary_position(),
        Position::new(3, 0),
        "[F10] :4 must jump to line index 3, column 0"
    );
}

/// [F10] Ctrl+Shift+O populates @symbol items from active file and jumps to chosen symbol
#[test]
fn t1_f10_03_ctrl_shift_o_populates_document_symbols_and_jumps() {
    let ws = TempWorkspace::new();
    ws.write_file(
        "src/lib.rs",
        "fn first_fn() {}\n\nstruct MyStruct {\n    val: i32,\n}\n\nfn target_fn() {}\n",
    );

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("src/lib.rs");

    assert!(
        dispatch_ctrl_shift_char(&mut win, 'o'),
        "[F10] Ctrl+Shift+O must open @symbol QuickInput"
    );
    let scene = win.render();
    assert!(
        scene.contains_text("first_fn") && scene.contains_text("target_fn"),
        "[F10] @symbol QuickInput must populate symbols from active .rs file, got: {}",
        scene.all_text()
    );

    win.type_text("target_fn");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert_eq!(
        win.active_editor().unwrap().primary_position().line,
        6,
        "[F10] Selecting @target_fn must jump cursor to line index 6"
    );
}

/// [F10] Ctrl+P allows switching prefix dynamically to '@' and ':'
#[test]
fn t1_f10_04_ctrl_p_prefix_switch_to_symbol_and_line_modes() {
    let ws = TempWorkspace::new();
    ws.write_file(
        "app.py",
        "def alpha():\n    pass\n\ndef beta_helper():\n    return 1\n",
    );

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("app.py");

    assert!(win.dispatch_ctrl_char('p').is_consumed(), "[F10] Ctrl+P must open QuickOpen");
    win.type_text("@beta_helper");
    let _sym_scene = win.render();
    let sym_tree = win.accessibility_tree();
    let sym_results = sym_tree
        .find_by_id("commandPalette.results")
        .expect("[F10] CommandPalette results must be exposed in AccessibilityTree");
    assert!(
        sym_results
            .children
            .iter()
            .any(|c| c.name.contains("beta_helper")),
        "[F10] Typing '@beta_helper' in Ctrl+P must populate document symbol in palette results"
    );
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert_eq!(
        win.active_editor().unwrap().primary_position().line,
        3,
        "[F10] Choosing @beta_helper from Ctrl+P must jump to line 3"
    );

    assert!(win.dispatch_ctrl_char('p').is_consumed(), "[F10] Ctrl+P must open QuickOpen");
    win.type_text(":2:5");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert_eq!(
        win.active_editor().unwrap().primary_position(),
        Position::new(1, 4),
        "[F10] Typing ':2:5' in Ctrl+P must jump to (1, 4)"
    );
}

/// [F10] PaletteAction::OpenFile preserves target line & column without cursor reset
#[test]
fn t1_f10_05_palette_open_file_preserves_line_and_col() {
    let ws = TempWorkspace::new();
    ws.write_file("a.rs", "fn a() {}\n");
    let b_path = ws.write_file("b.rs", "// one\n// two\nfn target_in_b() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("a.rs");

    let mut palette = CommandPalette::new();
    palette.set_items(vec![PaletteItem {
        command_id: "open:b".to_string(),
        label: "b.rs".to_string(),
        category: Some("Go to Line".to_string()),
        keybinding: None,
        path: Some(b_path),
        line: Some(3),
        column: Some(4),
        item_kind: PaletteItemKind::File,
    }]);
    palette.open_mode(QuickInputMode::File);
    win.window
        .view
        .open_command_palette(CommandPaletteView::new(palette));

    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    let pos = win.active_editor().unwrap().primary_position();
    assert_eq!(
        pos.line, 2,
        "[F10] PaletteAction::OpenFile must preserve line=3 (0-based 2) without resetting cursor, got {:?}",
        pos
    );
    assert!(
        pos.column >= 3,
        "[F10] PaletteAction::OpenFile must preserve column on target line, got {:?}",
        pos
    );
}

// ============================================================================
// F11: Notification Toast Overlay & Interactive Prompt UI — Tier 1
// ============================================================================

/// [F11] window.showInformationMessage renders toast in Scene and AccessibilityTree
#[tokio::test]
async fn t1_f11_01_show_information_message_renders_toast_in_scene_and_a11y() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message(
            "window.showInformationMessage",
            &json!({ "message": "Workspace indexed successfully" }),
        )
        .await;

    trigger_sync(&mut win);
    let scene = win.render();
    assert!(
        scene.contains_text("Workspace indexed successfully"),
        "[F11] Information notification must render in Scene, got: {}",
        scene.all_text()
    );

    let tree = win.accessibility_tree();
    let found_in_a11y = tree.to_flat_list().iter().any(|n| {
        n.name.contains("Workspace indexed successfully")
            || n.value
                .as_deref()
                .is_some_and(|v| v.contains("Workspace indexed successfully"))
            || n.description
                .as_deref()
                .is_some_and(|d| d.contains("Workspace indexed successfully"))
    });
    assert!(
        found_in_a11y,
        "[F11] Notification toast must be exposed in AccessibilityTree"
    );
}

/// [F11] Warning/Error toast renders action buttons and clicking resolves ExtHostWindowManager waiter
#[tokio::test]
async fn t1_f11_02_warning_error_toast_action_click_resolves_waiter() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));
    bridge.window_manager.set_await_responses(true);

    let bridge_clone = Arc::clone(&bridge);
    let rpc_task = tokio::spawn(async move {
        bridge_clone
            .handle_rpc_message(
                "window.showWarningMessage",
                &json!({ "message": "Config outdated", "items": ["Reload", "Later"] }),
            )
            .await
    });

    for _ in 0..20 {
        if !bridge.window_manager.active_notifications().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }

    trigger_sync(&mut win);
    let scene = win.render();
    assert!(
        scene.contains_text("Config outdated") && scene.contains_text("Reload"),
        "[F11] Warning toast and action button must render in Scene, got: {}",
        scene.all_text()
    );

    let clicked = win.click_a11y_node("Reload") || win.click_scene_text("Reload");
    assert!(clicked, "[F11] Action button 'Reload' must be clickable");

    let resolved = tokio::time::timeout(std::time::Duration::from_secs(2), rpc_task)
        .await
        .expect("notification task should resolve")
        .expect("join ok");
    assert_eq!(
        resolved,
        Some(json!("Reload")),
        "[F11] Clicking 'Reload' must resolve ExtHostWindowManager waiter with 'Reload'"
    );
    assert!(
        !win.render().contains_text("Config outdated"),
        "[F11] Resolved toast must be removed from Scene"
    );
}

/// [F11] Dismiss button removes toast from Scene and resolves waiter with null/undefined
#[tokio::test]
async fn t1_f11_03_dismiss_button_removes_toast_and_resolves_null() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));
    bridge.window_manager.set_await_responses(true);

    let bridge_clone = Arc::clone(&bridge);
    let rpc_task = tokio::spawn(async move {
        bridge_clone
            .handle_rpc_message(
                "window.showErrorMessage",
                &json!({ "message": "Connection lost", "items": ["Retry"] }),
            )
            .await
    });

    for _ in 0..20 {
        if !bridge.window_manager.active_notifications().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }

    trigger_sync(&mut win);
    let scene = win.render();
    assert!(
        scene.contains_text("Connection lost"),
        "[F11] Error toast must render before dismiss"
    );

    let dismissed = win.click_a11y_node("Dismiss")
        || win.click_a11y_node("Close Notification")
        || win.click_scene_text("✕");
    assert!(dismissed, "[F11] Dismiss [✕] button must be clickable");

    let resolved = tokio::time::timeout(std::time::Duration::from_secs(2), rpc_task)
        .await
        .expect("dismiss task should resolve")
        .expect("join ok");
    assert_eq!(
        resolved,
        Some(Value::Null),
        "[F11] Dismissing toast must resolve waiter with Null"
    );
    assert!(
        !win.render().contains_text("Connection lost"),
        "[F11] Dismissed toast must disappear from Scene"
    );
}

/// [F11] modal: true notification renders modal dialog with detail and resolves action
#[tokio::test]
async fn t1_f11_04_modal_notification_renders_dialog_and_resolves() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));
    bridge.window_manager.set_await_responses(true);

    let bridge_clone = Arc::clone(&bridge);
    let rpc_task = tokio::spawn(async move {
        bridge_clone
            .handle_rpc_message(
                "window.showWarningMessage",
                &json!({
                    "message": "Overwrite file?",
                    "options": { "modal": true, "detail": "Local edits will be lost." },
                    "items": ["Overwrite"]
                }),
            )
            .await
    });

    for _ in 0..20 {
        if !bridge.window_manager.active_notifications().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }

    trigger_sync(&mut win);
    let scene = win.render();
    assert!(
        scene.contains_text("Overwrite file?") && scene.contains_text("Local edits will be lost."),
        "[F11] Modal notification must render message and detail, got: {}",
        scene.all_text()
    );
    let tree = win.accessibility_tree();
    let dialogs = tree.find_by_role(AccessibleRole::Dialog);
    assert!(
        !dialogs.is_empty(),
        "[F11] Modal notification must expose AccessibleRole::Dialog"
    );

    let clicked = win.click_a11y_node("Overwrite") || win.click_scene_text("Overwrite");
    assert!(clicked, "[F11] Modal action button must be clickable");

    let res = tokio::time::timeout(std::time::Duration::from_secs(2), rpc_task)
        .await
        .expect("modal task should resolve")
        .expect("join ok");
    assert_eq!(res, Some(json!("Overwrite")));
}

/// [F11] showQuickPick and showInputBox render interactive prompts and resolve on Enter
#[tokio::test]
async fn t1_f11_05_show_quick_pick_and_input_box_render_and_resolve() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));
    bridge.window_manager.set_await_responses(true);

    // 1. QuickPick
    let b1 = Arc::clone(&bridge);
    let qp_task = tokio::spawn(async move {
        b1.handle_rpc_message(
            "window.showQuickPick",
            &json!({
                "items": ["Debug", "Release"],
                "options": { "placeHolder": "Select build profile" }
            }),
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
    let qp_scene = win.render();
    assert!(
        qp_scene.contains_text("Debug") && qp_scene.contains_text("Release"),
        "[F11] QuickPick must render items in Scene, got: {}",
        qp_scene.all_text()
    );
    win.dispatch_key(KeyCode::ArrowDown, Modifiers::none(), None);
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    let qp_res = tokio::time::timeout(std::time::Duration::from_secs(2), qp_task)
        .await
        .expect("quickPick should resolve")
        .expect("join ok");
    assert_eq!(
        qp_res,
        Some(json!({ "label": "Release" })),
        "[F11] QuickPick Enter on second item must resolve {{ \"label\": \"Release\" }}"
    );

    // 2. InputBox
    let b2 = Arc::clone(&bridge);
    let ib_task = tokio::spawn(async move {
        b2.handle_rpc_message(
            "window.showInputBox",
            &json!({
                "options": { "prompt": "Enter module name", "placeHolder": "my_mod" }
            }),
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
    let ib_scene = win.render();
    assert!(
        ib_scene.contains_text("Enter module name") || ib_scene.contains_text("my_mod"),
        "[F11] InputBox must render prompt/placeholder in Scene, got: {}",
        ib_scene.all_text()
    );
    win.type_text("parser");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    let ib_res = tokio::time::timeout(std::time::Duration::from_secs(2), ib_task)
        .await
        .expect("inputBox should resolve")
        .expect("join ok");
    assert_eq!(
        ib_res,
        Some(json!("parser")),
        "[F11] InputBox Enter must resolve typed text 'parser'"
    );
}

// ============================================================================
// F12: Testing Sidebar View (workbench.view.testing) — Tier 1
// ============================================================================

/// [F12] ActivityBar exposes workbench.view.testing (keeping extensions at index 4) and switches sidebar
#[test]
fn t1_f12_01_activity_bar_exposes_testing_and_switches_sidebar() {
    let mut win = E2eWindow::new();
    let items = &win.window.view.activity_bar.model.items;
    assert_eq!(
        items.get(4).map(|i| i.id.as_str()),
        Some("workbench.view.extensions"),
        "[F12] Extensions must remain at ActivityBar index 4"
    );
    assert!(
        items.iter().any(|i| i.id == "workbench.view.testing"),
        "[F12] ActivityBar must include 'workbench.view.testing'"
    );

    let clicked = win.click_a11y_node("Testing")
        || win.click_scene_text("TST")
        || win.click_scene_text("TEST");
    assert!(clicked, "[F12] Testing ActivityBar icon must be clickable");

    let active_sidebar = win.window.view.layout.active_sidebar_view.as_str();
    assert!(
        active_sidebar == "testing" || active_sidebar == "workbench.view.testing",
        "[F12] Clicking Testing icon must activate testing sidebar, got '{active_sidebar}'"
    );
    let scene = win.render();
    assert!(
        scene.contains_text("TESTING"),
        "[F12] Testing sidebar header must render 'TESTING'"
    );
}

/// [F12] createController + updateItems renders hierarchical controller & test items in Scene & AccessibilityTree
#[tokio::test]
async fn t1_f12_02_controller_and_items_render_hierarchy_in_scene_and_a11y() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "cargo-suite", "label": "Cargo Suite" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "cargo-suite",
                "items": [{
                    "id": "suite_math",
                    "label": "suite_math",
                    "children": [
                        { "id": "test_add", "label": "test_add", "children": [] }
                    ]
                }]
            }),
        )
        .await;

    let _ = win.app.commands.execute("workbench.view.testing", None);
    let _ = win.click_a11y_node("Testing") || win.click_scene_text("TST") || win.click_scene_text("TEST");
    trigger_sync(&mut win);

    let scene = win.render();
    assert!(
        scene.contains_text("Cargo Suite")
            && scene.contains_text("suite_math")
            && scene.contains_text("test_add"),
        "[F12] Testing sidebar must render controller and nested test items, got: {}",
        scene.all_text()
    );

    let tree = win.accessibility_tree();
    assert!(
        !tree.find_by_role(AccessibleRole::Tree).is_empty(),
        "[F12] Testing sidebar must expose AccessibleRole::Tree"
    );
    let tree_items = tree.find_by_role(AccessibleRole::TreeItem);
    assert!(
        tree_items.iter().any(|n| n.name.contains("suite_math"))
            && tree_items.iter().any(|n| n.name.contains("test_add")),
        "[F12] AccessibilityTree must expose TreeItems for test hierarchy"
    );
}

/// [F12] Test run updates render passed/failed/skipped, durations, failure detail, and ActivityBar badge
#[tokio::test]
async fn t1_f12_03_test_run_states_durations_and_failure_badge() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "unit", "label": "Unit Tests" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "unit",
                "items": [
                    { "id": "t_pass", "label": "t_pass", "children": [] },
                    { "id": "t_fail", "label": "t_fail", "children": [] },
                    { "id": "t_skip", "label": "t_skip", "children": [] }
                ]
            }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.createTestRun",
            &json!({ "runId": "r1", "controllerId": "unit", "name": "Run 1" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateTestState",
            &json!({ "runId": "r1", "testId": "t_pass", "state": "passed", "duration": 12 }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateTestState",
            &json!({
                "runId": "r1",
                "testId": "t_fail",
                "state": "failed",
                "duration": 45,
                "messages": [{ "message": "expected 4 got 5" }]
            }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateTestState",
            &json!({ "runId": "r1", "testId": "t_skip", "state": "skipped" }),
        )
        .await;
    bridge
        .handle_rpc_message("tests.endTestRun", &json!({ "runId": "r1" }))
        .await;

    let _ = win.click_a11y_node("Testing") || win.click_scene_text("TST") || win.click_scene_text("TEST");
    trigger_sync(&mut win);
    let scene = win.render();

    assert!(
        scene.contains_text("12ms") && scene.contains_text("45ms"),
        "[F12] Testing sidebar must render item durations (12ms, 45ms), got: {}",
        scene.all_text()
    );
    assert!(
        scene.contains_text("expected 4 got 5"),
        "[F12] Testing sidebar must render failure detail message, got: {}",
        scene.all_text()
    );
    let testing_item = win
        .window
        .view
        .activity_bar
        .model
        .items
        .iter()
        .find(|i| i.id == "workbench.view.testing");
    assert_eq!(
        testing_item.and_then(|i| i.badge),
        Some(1),
        "[F12] ActivityBar testing item must show badge=1 for 1 failed test"
    );
}

/// [F12] Clicking Run All triggers outbound tests.runTests RPC
#[tokio::test]
async fn t1_f12_04_clicking_run_triggers_outbound_run_tests_rpc() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "cargo", "label": "Cargo" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "cargo",
                "items": [{ "id": "t1", "label": "t1", "children": [] }]
            }),
        )
        .await;

    let _ = win.click_a11y_node("Testing") || win.click_scene_text("TST") || win.click_scene_text("TEST");
    trigger_sync(&mut win);
    let _ = win.render();

    let clicked_run = win.click_a11y_node("Run All")
        || win.click_scene_text("Run All")
        || win.click_scene_text("Run");
    assert!(clicked_run, "[F12] Run All button must be clickable");

    let mut saw_pending = false;
    for _ in 0..20 {
        if rpc.pending_count() > 0 {
            saw_pending = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        saw_pending,
        "[F12] Clicking Run must dispatch outbound tests.runTests RPC on RpcBridge"
    );
    let _ = rpc.handle_incoming_response(1, Some(json!({ "ran": true })), None);
}

/// [F12] Clicking failed test item with file:// URI opens file in editor
#[tokio::test]
async fn t1_f12_05_clicking_failed_test_with_uri_opens_file() {
    let ws = TempWorkspace::new();
    let test_file = ws.write_file(
        "tests/calc.rs",
        "fn setup() {}\nfn helper() {}\n#[test]\nfn fail_math() { panic!(); }\n",
    );
    let test_uri = path_to_file_uri(&test_file);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "calc-ctrl", "label": "Calc" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "calc-ctrl",
                "items": [{
                    "id": "fail_math",
                    "label": "fail_math",
                    "uri": test_uri,
                    "children": []
                }]
            }),
        )
        .await;

    let _ = win.click_a11y_node("Testing") || win.click_scene_text("TST") || win.click_scene_text("TEST");
    trigger_sync(&mut win);
    let _ = win.render();

    let clicked_item = win.click_a11y_node("fail_math") || win.click_scene_text("fail_math");
    assert!(clicked_item, "[F12] Test item 'fail_math' must be clickable");

    assert!(
        win.read_doc(&test_file)
            .is_some_and(|txt| txt.contains("fail_math")),
        "[F12] Clicking test item with file:// URI must open file in EditorAreaView"
    );
}

// ============================================================================
// F13: Interactive File Explorer CRUD — Tier 1
// ============================================================================

/// [F13] New File creates real file on disk and updates explorer and open editor
#[test]
fn t1_f13_01_new_file_creates_on_disk_and_updates_explorer_editor() {
    let ws = TempWorkspace::new();
    ws.write_file("existing.rs", "fn old() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.window.view.focused_region = FocusedRegion::Sidebar;
    let _ = win.render();

    let clicked = win.click_a11y_node("New File")
        || win.click_scene_text("+F")
        || win.click_scene_text("New File");
    assert!(clicked, "[F13] Explorer header must expose New File button");

    win.type_text("created.rs");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    assert!(
        ws.exists("created.rs"),
        "[F13] New File must create real file on disk"
    );
    assert!(
        win.window
            .view
            .explorer
            .flattened_rows()
            .iter()
            .any(|r| r.name == "created.rs"),
        "[F13] Explorer tree must refresh to include 'created.rs'"
    );
    assert!(
        win.read_doc("created.rs").is_some(),
        "[F13] Newly created file must open in editor"
    );
}

/// [F13] New Folder creates real directory on disk and updates explorer
#[test]
fn t1_f13_02_new_folder_creates_real_dir_on_disk() {
    let ws = TempWorkspace::new();
    ws.write_file("README.md", "# Demo\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.window.view.focused_region = FocusedRegion::Sidebar;
    let _ = win.render();

    let clicked = win.click_a11y_node("New Folder")
        || win.click_scene_text("+D")
        || win.click_scene_text("New Folder");
    assert!(clicked, "[F13] Explorer header must expose New Folder button");

    win.type_text("models");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    assert!(
        ws.path().join("models").is_dir(),
        "[F13] New Folder must create real directory on disk"
    );
    assert!(
        win.window
            .view
            .explorer
            .flattened_rows()
            .iter()
            .any(|r| r.name == "models" && r.is_dir),
        "[F13] Explorer must include newly created directory 'models'"
    );
}

/// [F13] F2 Rename renames file on disk and updates open editor tab URI & title
#[test]
fn t1_f13_03_f2_rename_updates_disk_and_open_editor_tab() {
    let ws = TempWorkspace::new();
    let old_path = ws.write_file("before.rs", "pub fn stay() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("before.rs");
    win.window.view.focused_region = FocusedRegion::Sidebar;
    win.window.view.explorer.model.selected_path = Some(old_path);
    let _ = win.render();

    win.dispatch_key(KeyCode::F2, Modifiers::none(), None);
    win.type_text("after.rs");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    assert!(
        !ws.exists("before.rs") && ws.exists("after.rs"),
        "[F13] F2 rename must move 'before.rs' to 'after.rs' on disk"
    );
    assert!(
        win.read_doc("after.rs").is_some(),
        "[F13] Open editor URI must update to renamed file 'after.rs'"
    );
    let has_new_tab = win
        .window
        .view
        .editor_area
        .model
        .current_group_ref()
        .tabs
        .iter()
        .any(|t| t.title == "after.rs");
    assert!(
        has_new_tab,
        "[F13] Open editor tab title must update to 'after.rs'"
    );
}

/// [F13] Delete + confirmation removes file from disk and closes open editor tab
#[test]
fn t1_f13_04_delete_with_confirmation_removes_file_and_closes_tab() {
    let ws = TempWorkspace::new();
    let target = ws.write_file("doomed.rs", "fn bye() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("doomed.rs");
    win.window.view.focused_region = FocusedRegion::Sidebar;
    win.window.view.explorer.model.selected_path = Some(target);
    let _ = win.render();

    win.dispatch_key(KeyCode::Delete, Modifiers::none(), None);
    assert!(
        ws.exists("doomed.rs"),
        "[F13] Pressing Delete must prompt for confirmation before removing file"
    );
    let confirm_scene = win.render();
    assert!(
        confirm_scene.contains_text("Delete") || confirm_scene.contains_text("Confirm"),
        "[F13] Delete confirmation UI must render in Scene, got: {}",
        confirm_scene.all_text()
    );

    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert!(
        !ws.exists("doomed.rs"),
        "[F13] Confirming Delete must remove file from disk"
    );
    assert!(
        win.read_doc("doomed.rs").is_none(),
        "[F13] Deleting open file must close its editor tab"
    );
}

/// [F13] Keyboard navigation (ArrowUp/ArrowDown/Enter) selects and opens file in Explorer
#[test]
fn t1_f13_05_keyboard_navigation_opens_selected_file() {
    let ws = TempWorkspace::new();
    ws.write_file("alpha.rs", "// alpha\n");
    ws.write_file("beta.rs", "// beta\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.window.view.focused_region = FocusedRegion::Sidebar;
    win.window.view.explorer.model.selected_path = Some(ws.path().to_path_buf());
    let _ = win.render();

    // Navigate down from root -> alpha.rs -> beta.rs
    win.dispatch_key(KeyCode::ArrowDown, Modifiers::none(), None);
    win.dispatch_key(KeyCode::ArrowDown, Modifiers::none(), None);
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    let opened = win.active_editor_text();
    assert_eq!(
        opened.as_deref(),
        Some("// beta\n"),
        "[F13] ArrowDown + Enter in Explorer must open selected file 'beta.rs', got: {:?}",
        opened
    );
}

// ============================================================================
// F14: Contributed TreeViews — Tier 1
// ============================================================================

/// [F14] Loads treeview_ext and renders contributed section when when-clause is true
#[tokio::test]
async fn t1_f14_01_loads_treeview_ext_and_renders_section_when_true() {
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
    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TreeDataProvider", "id": "projectOutlineView" }),
        )
        .await;

    win.window
        .context_keys
        .set_bool("workspaceHasProjects", true);
    trigger_sync(&mut win);
    let scene = win.render();
    assert!(
        scene.contains_text("Project Outline"),
        "[F14] Contributed view 'Project Outline' must render when 'workspaceHasProjects' is true, got: {}",
        scene.all_text()
    );
}

/// [F14] Lazy root & child expansion invokes treeView.getChildren RPC
#[tokio::test]
async fn t1_f14_02_lazy_root_and_child_expansion_invokes_get_children() {
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

    // Fulfill root getChildren RPC if queued
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
            { "id": "root_pkg", "label": "root_pkg", "collapsibleState": 1 }
        ])),
        None,
    );
    tokio::task::yield_now().await;
    trigger_sync(&mut win);

    let root_scene = win.render();
    assert!(
        root_scene.contains_text("root_pkg"),
        "[F14] Root TreeItem 'root_pkg' from treeView.getChildren must render in Explorer, got: {}",
        root_scene.all_text()
    );

    // Click collapsible root item to trigger child getChildren RPC
    let clicked = win.click_a11y_node("root_pkg") || win.click_scene_text("root_pkg");
    assert!(clicked, "[F14] Collapsible TreeItem 'root_pkg' must be clickable");

    for _ in 0..20 {
        if rpc.pending_count() > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    let _ = rpc.handle_incoming_response(
        2,
        Some(json!([
            { "id": "child_mod", "label": "child_mod.rs", "collapsibleState": 0 }
        ])),
        None,
    );
    tokio::task::yield_now().await;
    trigger_sync(&mut win);

    let child_scene = win.render();
    assert!(
        child_scene.contains_text("child_mod.rs"),
        "[F14] Expanding 'root_pkg' must render child 'child_mod.rs' via treeView.getChildren, got: {}",
        child_scene.all_text()
    );
}

/// [F14] Clicking TreeItem with command executes registered command
#[tokio::test]
async fn t1_f14_03_clicking_tree_item_with_command_executes_command() {
    let ws = TempWorkspace::new();
    let loader = setup_treeview_ext(&ws);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let ran = Arc::new(AtomicBool::new(false));
    let ran_flag = Arc::clone(&ran);
    let _disp = win.app.commands.register("projectOutline.refresh", move |_| {
        ran_flag.store(true, Ordering::SeqCst);
        Ok(json!({ "ok": true }))
    });

    let (bridge, rpc) = create_bridge_with_root(ws.path());
    let bridge = Arc::new(
        LanguageFeatureBridge::new(
            Arc::clone(&bridge.provider_registry),
            Arc::clone(&rpc),
            Arc::clone(&bridge.lsp_manager),
        )
        .with_extension_loader(&loader)
        .with_command_registry(win.app.commands.clone())
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
            {
                "id": "cmd_item",
                "label": "Run Outline Refresh",
                "collapsibleState": 0,
                "command": { "title": "Refresh", "command": "projectOutline.refresh" }
            }
        ])),
        None,
    );
    tokio::task::yield_now().await;
    trigger_sync(&mut win);
    let _ = win.render();

    let clicked = win.click_a11y_node("Run Outline Refresh")
        || win.click_scene_text("Run Outline Refresh");
    assert!(clicked, "[F14] TreeItem with command must be clickable");
    assert!(
        ran.load(Ordering::SeqCst),
        "[F14] Clicking TreeItem with command must execute 'projectOutline.refresh'"
    );
}

/// [F14] onDidChangeTreeData notification refreshes contributed TreeView items
#[tokio::test]
async fn t1_f14_04_on_did_change_tree_data_refreshes_items() {
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
        Some(json!([{ "id": "v1", "label": "Outline_V1", "collapsibleState": 0 }])),
        None,
    );
    tokio::task::yield_now().await;
    trigger_sync(&mut win);
    assert!(
        win.render().contains_text("Outline_V1"),
        "[F14] Initial item Outline_V1 must render"
    );

    // Fire onDidChangeTreeData refresh
    bridge
        .handle_rpc_message(
            "treeView.onDidChangeTreeData",
            &json!({ "viewId": "projectOutlineView" }),
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
        2,
        Some(json!([{ "id": "v2", "label": "Outline_V2", "collapsibleState": 0 }])),
        None,
    );
    tokio::task::yield_now().await;
    trigger_sync(&mut win);

    let refreshed = win.render();
    assert!(
        refreshed.contains_text("Outline_V2"),
        "[F14] onDidChangeTreeData must refresh contributed items to 'Outline_V2', got: {}",
        refreshed.all_text()
    );
}

/// [F14] AccessibilityTree exposes contributed TreeView container and TreeItem nodes
#[tokio::test]
async fn t1_f14_05_accessibility_tree_exposes_contributed_tree_and_items() {
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
        Some(json!([{ "id": "a11y_item", "label": "AccessibleNodeItem", "collapsibleState": 0 }])),
        None,
    );
    tokio::task::yield_now().await;
    trigger_sync(&mut win);

    let tree = win.accessibility_tree();
    let trees = tree.find_by_role(AccessibleRole::Tree);
    assert!(
        trees.len() >= 2 || trees.iter().any(|t| t.name.contains("Project Outline")),
        "[F14] AccessibilityTree must expose contributed Tree node for Project Outline"
    );
    let items = tree.find_by_role(AccessibleRole::TreeItem);
    assert!(
        items.iter().any(|i| i.name.contains("AccessibleNodeItem")),
        "[F14] AccessibilityTree must expose TreeItem for contributed node"
    );
}

// ============================================================================
// F16: Additional Language Tokenizers (Go, C, C++, HTML, CSS, TOML, YAML) — Tier 1
// ============================================================================

/// [F16] GrammarRegistry::new() resolves go, c, cpp, html, css, toml, yaml by ID and file path
#[test]
fn t1_f16_01_grammar_registry_resolves_seven_languages_by_id_and_path() {
    let reg = GrammarRegistry::new();

    for (id, paths) in [
        ("go", vec!["main.go"]),
        ("c", vec!["lib.c", "include/header.h"]),
        ("cpp", vec!["engine.cpp", "mod.cc", "vec.cxx", "api.hpp"]),
        ("html", vec!["index.html", "about.htm"]),
        ("css", vec!["styles.css", "theme.scss", "base.less"]),
        ("toml", vec!["Cargo.toml"]),
        ("yaml", vec!["ci.yaml", "config.yml"]),
    ] {
        let by_id = reg
            .get_by_id(id)
            .unwrap_or_else(|| panic!("[F16] GrammarRegistry must register '{id}'"));
        assert_eq!(by_id.language_id(), id);
        for p in paths {
            assert_eq!(
                reg.get_for_path(std::path::Path::new(p)).language_id(),
                id,
                "[F16] get_for_path({p}) must resolve '{id}'"
            );
        }
    }
}

/// [F16] TextDocument::new assigns expected language_id for all seven new languages
#[test]
fn t1_f16_02_text_document_assigns_expected_language_id() {
    for (filename, expected_id) in [
        ("server.go", "go"),
        ("driver.c", "c"),
        ("driver.h", "c"),
        ("widget.cpp", "cpp"),
        ("widget.hpp", "cpp"),
        ("page.html", "html"),
        ("style.css", "css"),
        ("Cargo.toml", "toml"),
        ("deploy.yaml", "yaml"),
        ("deploy.yml", "yaml"),
    ] {
        let doc = TextDocument::new("", Some(PathBuf::from(filename)), None);
        assert_eq!(
            doc.language_id, expected_id,
            "[F16] TextDocument::new({filename}) must set language_id '{expected_id}'"
        );
    }
}

/// [F16] Go, C, and C++ tokenizers emit keyword, string, comment, and number scopes
#[test]
fn t1_f16_03_go_c_cpp_emit_keyword_string_comment_number_scopes() {
    let reg = GrammarRegistry::new();
    let theme = ThemeService::default_dark_theme();

    for (lang, line) in [
        ("go", "const msg = \"hello\"; var n = 42 // go comment"),
        ("c", "int x = 100; const char *s = \"ok\"; // c comment"),
        ("cpp", "constexpr auto s = \"cpp\"; int n = 7; // cpp comment"),
    ] {
        let mut hl = SyntaxHighlighter::for_language(lang, theme.clone(), &reg);
        let spans = hl.get_line_tokens(0, line);
        assert!(
            spans.iter().any(|s| matches!(
                s.scope,
                SyntaxScope::Keyword | SyntaxScope::KeywordControl | SyntaxScope::StorageType | SyntaxScope::Type
            )),
            "[F16] {lang} must emit keyword/type scope for '{line}'"
        );
        assert!(
            spans.iter().any(|s| matches!(
                s.scope,
                SyntaxScope::String | SyntaxScope::StringQuoted | SyntaxScope::StringTemplate
            )),
            "[F16] {lang} must emit string scope for '{line}'"
        );
        assert!(
            spans
                .iter()
                .any(|s| matches!(s.scope, SyntaxScope::Number | SyntaxScope::ConstantNumeric)),
            "[F16] {lang} must emit number scope for '{line}'"
        );
        assert!(
            spans.iter().any(|s| matches!(
                s.scope,
                SyntaxScope::Comment | SyntaxScope::CommentLine | SyntaxScope::CommentBlock
            )),
            "[F16] {lang} must emit comment scope for '{line}'"
        );
    }
}

/// [F16] HTML and CSS tokenizers emit tag/attribute/string/selector/property scopes
#[test]
fn t1_f16_04_html_and_css_emit_tag_selector_property_scopes() {
    let reg = GrammarRegistry::new();
    let theme = ThemeService::default_dark_theme();

    let mut html_hl = SyntaxHighlighter::for_language("html", theme.clone(), &reg);
    let html_spans = html_hl.get_line_tokens(0, "<div class=\"hero\"><!-- note -->Hi</div>");
    assert!(
        html_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::Keyword | SyntaxScope::Type | SyntaxScope::EntityNameType | SyntaxScope::Property
        )),
        "[F16] HTML must emit tag/attribute scopes"
    );
    assert!(
        html_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::String | SyntaxScope::StringQuoted
        )),
        "[F16] HTML must emit string scope for attribute values"
    );
    assert!(
        html_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::Comment | SyntaxScope::CommentBlock
        )),
        "[F16] HTML must emit comment scope for <!-- note -->"
    );

    let mut css_hl = SyntaxHighlighter::for_language("css", theme, &reg);
    let css_spans = css_hl.get_line_tokens(0, ".btn { color: #ff00aa; margin: 12px; } /* box */");
    assert!(
        css_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::Property | SyntaxScope::Variable | SyntaxScope::Type | SyntaxScope::EntityNameType | SyntaxScope::Keyword
        )),
        "[F16] CSS must emit selector/property scopes"
    );
    assert!(
        css_spans
            .iter()
            .any(|s| matches!(s.scope, SyntaxScope::Number | SyntaxScope::ConstantNumeric)),
        "[F16] CSS must emit numeric scope for 12px"
    );
    assert!(
        css_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::Comment | SyntaxScope::CommentBlock
        )),
        "[F16] CSS must emit comment scope for /* box */"
    );
}

/// [F16] TOML and YAML tokenizers emit key/property, string, number, and comment scopes
#[test]
fn t1_f16_05_toml_and_yaml_emit_key_string_comment_scopes() {
    let reg = GrammarRegistry::new();
    let theme = ThemeService::default_dark_theme();

    let mut toml_hl = SyntaxHighlighter::for_language("toml", theme.clone(), &reg);
    let toml_spans = toml_hl.get_line_tokens(0, "name = \"rustcode\" # pkg");
    assert!(
        toml_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::Property | SyntaxScope::Variable | SyntaxScope::Keyword
        )),
        "[F16] TOML must emit key scope"
    );
    assert!(
        toml_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::String | SyntaxScope::StringQuoted
        )),
        "[F16] TOML must emit string scope"
    );
    assert!(
        toml_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::Comment | SyntaxScope::CommentLine
        )),
        "[F16] TOML must emit comment scope"
    );

    let mut yaml_hl = SyntaxHighlighter::for_language("yaml", theme, &reg);
    let yaml_spans = yaml_hl.get_line_tokens(0, "image: \"alpine:3.19\" # base");
    assert!(
        yaml_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::Property | SyntaxScope::Variable | SyntaxScope::Keyword
        )),
        "[F16] YAML must emit key scope"
    );
    assert!(
        yaml_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::String | SyntaxScope::StringQuoted
        )),
        "[F16] YAML must emit string scope"
    );
    assert!(
        yaml_spans.iter().any(|s| matches!(
            s.scope,
            SyntaxScope::Comment | SyntaxScope::CommentLine
        )),
        "[F16] YAML must emit comment scope"
    );
}
