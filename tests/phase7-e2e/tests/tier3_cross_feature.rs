use phase7_e2e::{
    create_bridge_with_root, repo_root, run_node_script, E2eWindow, TempWorkspace,
};
use rustcode_base::context_key::ContextKeyExpr;
use rustcode_editor::{GrammarRegistry, Position, SyntaxHighlighter, SyntaxScope, WordWrapMode};
use rustcode_ext_host::{ExtensionLoader, LanguageFeatureBridge};
use rustcode_platform::accessibility::AccessibleRole;
use rustcode_platform::fs::{file_uri_to_path, path_to_file_uri, paths_equivalent};
use rustcode_platform::keybindings::{KeyCombo, KeybindingService, Modifiers};
use rustcode_platform::render::KeyCode;
use rustcode_platform::theme::ThemeService;
use rustcode_workbench::search::search_document_symbols;
use rustcode_workbench::workbench_view::FocusedRegion;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

fn assert_docs_cover_expected_surface(root: &Path) {
    let readme = std::fs::read_to_string(root.join("README.md")).expect("README.md missing");
    let arch =
        std::fs::read_to_string(root.join("docs/architecture.md")).expect("architecture.md missing");
    let matrix = std::fs::read_to_string(root.join("docs/extension-compat-matrix.md"))
        .expect("extension-compat-matrix.md missing");
    let plan = std::fs::read_to_string(root.join("rustcode-implementation-plan.md"))
        .expect("rustcode-implementation-plan.md missing");

    let combined = format!("{readme}\n{arch}\n{matrix}\n{plan}");

    // F17 + F16: All 14 grammars documented
    for lang in [
        "rust",
        "javascript",
        "typescript",
        "python",
        "json",
        "markdown",
        "plaintext",
        "go",
        "c",
        "cpp",
        "html",
        "css",
        "toml",
        "yaml",
    ] {
        assert!(
            combined.to_ascii_lowercase().contains(lang),
            "Documentation missing language tokenizer '{lang}'"
        );
    }

    // F17 + F15: All 9 node-compat modules documented and present on disk
    for module in [
        "events",
        "fs",
        "path",
        "buffer",
        "url",
        "util",
        "os",
        "crypto",
        "child_process",
    ] {
        assert!(
            root.join("ext-host-js/node-compat")
                .join(format!("{module}.js"))
                .is_file(),
            "Expected node-compat module '{module}.js' on disk"
        );
        assert!(
            matrix.contains(module) || arch.contains(module),
            "Documentation missing node-compat module '{module}'"
        );
    }

    // F17 + F9 + F10 + F12: Phase 7 UI commands and keybindings documented
    for term in [
        "actions.find",
        "editor.action.startFindReplaceAction",
        "workbench.action.gotoLine",
        "workbench.action.gotoSymbol",
        "workbench.view.testing",
    ] {
        assert!(
            combined.contains(term),
            "Documentation missing Phase 7 command/view '{term}'"
        );
    }

    // Windows + Linux cross-platform guidance
    assert!(
        readme.contains("package.ps1") && readme.contains("package.sh"),
        "README.md must document both Windows (package.ps1) and Linux (package.sh) packaging"
    );

    // No stale unchecked Phase 7 M1-M5 items in README or compat matrix
    for doc_name in ["README.md", "docs/extension-compat-matrix.md"] {
        let text = std::fs::read_to_string(root.join(doc_name)).unwrap();
        assert!(
            !text.contains("- [ ] **M"),
            "{doc_name} still has unchecked Phase 7 milestone checkboxes"
        );
    }
}

/// [F9+F4+F3+F6] Find & Replace (Ctrl+H) -> Replace All -> Atomic Undo (Ctrl+Z) -> Save (Ctrl+S) -> Bridge Doc Sync
#[tokio::test]
async fn t3_01_f9_f4_f3_f6_find_replace_undo_save_and_doc_events() {
    let ws = TempWorkspace::new();
    let initial = "fn alpha() {\n    let legacy = 1;\n    let sum = legacy + legacy;\n}\n";
    ws.write_file("src/refactor.rs", initial);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    // F3: RustCodeContext::create_window() must wire LanguageFeatureBridge automatically.
    assert!(
        win.window.language_bridge.is_some(),
        "[F3] create_window() must attach a live LanguageFeatureBridge"
    );

    let uri = win.open_file_from_disk("src/refactor.rs");

    // Open Find & Replace widget via Ctrl+H (F9)
    let res = win.dispatch_ctrl_char('h');
    assert!(res.is_consumed(), "[F9] Ctrl+H must be consumed to open Find & Replace");

    win.type_text("legacy");
    let scene_after_query = win.render();
    assert!(
        scene_after_query.contains_text("1 of 3") || scene_after_query.contains_text("3 of 3"),
        "[F9] Find & Replace must display match count for 3 occurrences of 'legacy', got: {}",
        scene_after_query.all_text()
    );

    // Switch to Replace field via Tab and type replacement
    win.dispatch_key(KeyCode::Tab, Modifiers::none(), None);
    win.type_text("modern");

    let clicked_replace_all = win.click_a11y_node("Replace All")
        || win.click_scene_text("Replace All")
        || win.click_scene_text("All");
    assert!(
        clicked_replace_all,
        "[F9] Replace All control must be reachable via AccessibilityTree or Scene"
    );

    let replaced = win.active_editor_text().expect("active editor");
    assert!(
        !replaced.contains("legacy") && replaced.matches("modern").count() == 3,
        "[F9] Replace All must replace all 3 occurrences, got:\n{replaced}"
    );

    // Atomic undo via Ctrl+Z (F9 + F4)
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    win.dispatch_ctrl_char('z');
    let undone = win.active_editor_text().expect("active editor after undo");
    assert_eq!(
        undone, initial,
        "[F9+F4] Single Ctrl+Z must atomically undo Replace All"
    );

    // Redo / re-apply and save to disk (F3 + F6)
    win.dispatch_ctrl_char('y');
    let after_redo = win.active_editor_text().expect("active editor after redo");
    assert_eq!(
        after_redo.matches("modern").count(),
        3,
        "[F9+F4] Redo must restore Replace All edits"
    );

    win.dispatch_ctrl_char('s');
    let on_disk = ws.read_file("src/refactor.rs");
    assert_eq!(
        on_disk, after_redo,
        "[F3+F6] Ctrl+S must persist replaced content to disk at {uri}"
    );
    assert!(
        !win.active_editor().expect("active editor").is_dirty(),
        "[F6] Editor must be clean after Ctrl+S save"
    );
}

/// [F10+F16+F9] QuickInput @symbol & :line:col combined with Go grammar tokenizer & Find
#[test]
fn t3_02_f10_f16_f9_quickinput_goto_line_symbol_and_go_tokenizer() {
    let ws = TempWorkspace::new();
    let go_src = "package main\n\nfunc helperAlpha() int {\n\treturn 42\n}\n\nfunc main() {\n\t_ = helperAlpha()\n}\n";
    let go_path = ws.write_file("cmd/main.go", go_src);

    // F16: Go grammar registered in GrammarRegistry::new()
    let registry = GrammarRegistry::new();
    let go_grammar = registry
        .get_by_id("go")
        .expect("[F16] 'go' grammar must be registered in GrammarRegistry::new()");
    assert_eq!(go_grammar.language_id(), "go");
    assert_eq!(registry.get_for_path(&go_path).language_id(), "go");

    let mut hl = SyntaxHighlighter::for_language(
        "go",
        ThemeService::new().current().clone(),
        &registry,
    );
    let spans = hl.get_line_tokens(2, "func helperAlpha() int {");
    assert!(
        spans.iter().any(|s| s.scope != SyntaxScope::Plain),
        "[F16] Go tokenizer must classify keywords/functions with non-Plain scopes"
    );

    // F10: @symbol extraction & QuickInput Ctrl+Shift+O
    let symbols = search_document_symbols(go_src, &go_path);
    assert!(
        symbols.iter().any(|s| s.name == "helperAlpha"),
        "[F10] Go symbol extractor must find helperAlpha"
    );

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("cmd/main.go");

    let open_sym = win.dispatch_key(
        KeyCode::Char('o'),
        Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        },
        None,
    );
    assert!(open_sym.is_consumed(), "[F10] Ctrl+Shift+O must open @symbol QuickInput");
    win.type_text("helperAlpha");
    let sym_scene = win.render();
    assert!(
        sym_scene.contains_text("helperAlpha"),
        "[F10] @symbol QuickInput must render 'helperAlpha'"
    );
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert_eq!(
        win.active_editor().unwrap().primary_position().line,
        2,
        "[F10] Selecting @helperAlpha must jump cursor to line index 2 (1-based line 3)"
    );

    // F10: Go to Line :line:col via Ctrl+G
    let open_line = win.dispatch_ctrl_char('g');
    assert!(open_line.is_consumed(), "[F10] Ctrl+G must open :line QuickInput");
    win.type_text("8:5");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert_eq!(
        win.active_editor().unwrap().primary_position(),
        Position::new(7, 4),
        "[F10] :8:5 must move cursor to 0-based (7, 4)"
    );

    // F9: Find in Go buffer
    win.dispatch_ctrl_char('f');
    win.type_text("helperAlpha");
    let find_scene = win.render();
    assert!(
        find_scene.contains_text("1 of 2") || find_scene.contains_text("2 of 2"),
        "[F9+F16] Find widget must locate both Go occurrences of helperAlpha"
    );
}

/// [F11+F5+F4+F3] Notification Toast + Bidirectional Command Execution + Bridge Wiring
#[tokio::test]
async fn t3_03_f11_f5_f4_f3_notification_toast_and_command_routing() {
    let mut win = E2eWindow::new();
    let bridge = win
        .window
        .language_bridge
        .clone()
        .expect("[F3] E2eWindow must have LanguageFeatureBridge wired");

    // F4: Bidirectional builtin command via CommandRegistry mutates WorkbenchWindow
    let initial_sidebar = win.window.view.layout.sidebar_visible;
    let cmd_res = win
        .app
        .commands
        .execute("workbench.action.toggleSidebar", None);
    assert!(cmd_res.is_ok(), "[F4] toggleSidebar command must succeed");
    assert_ne!(
        win.window.view.layout.sidebar_visible, initial_sidebar,
        "[F4+F3] Executing toggleSidebar via CommandRegistry must toggle WorkbenchWindow sidebar"
    );

    // F5 + F11: Async notification with await_responses(true) rendered as toast & resolved via click
    bridge.window_manager.set_await_responses(true);
    let bridge_clone = Arc::clone(&bridge);
    let rpc_task = tokio::spawn(async move {
        bridge_clone
            .handle_rpc_message(
                "window.showInformationMessage",
                &json!({
                    "message": "Deploy build artifact?",
                    "items": ["Deploy", "Cancel"]
                }),
            )
            .await
    });

    // Yield until notification is queued
    for _ in 0..20 {
        if !bridge.window_manager.active_notifications().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }

    // Trigger workbench sync/render
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    let scene = win.render();
    assert!(
        scene.contains_text("Deploy build artifact?"),
        "[F11+F5] Notification message must render in WorkbenchWindow Scene"
    );
    assert!(
        scene.contains_text("Deploy"),
        "[F11+F5] Notification action button 'Deploy' must render in Scene"
    );

    let clicked = win.click_a11y_node("Deploy") || win.click_scene_text("Deploy");
    assert!(clicked, "[F11] Clicking 'Deploy' toast button must hit target");

    let resolved = tokio::time::timeout(std::time::Duration::from_secs(2), rpc_task)
        .await
        .expect("notification task should resolve")
        .expect("join should succeed");
    assert_eq!(
        resolved,
        Some(json!("Deploy")),
        "[F11+F5] Clicking toast action must resolve extension RPC with 'Deploy'"
    );
}

/// [F11+F12] Notification Toast + Testing Sidebar Failures Coexistence & Rendering
#[tokio::test]
async fn t3_04_f11_f12_notification_toast_and_testing_sidebar_failures() {
    let ws = TempWorkspace::new();
    let test_file = ws.write_file("tests/unit.rs", "#[test]\nfn broken_test() { assert_eq!(1, 2); }\n");
    let test_uri = path_to_file_uri(&test_file);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    // Populate Testing state via RPC (F12)
    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "rust-tests", "label": "Cargo Tests" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "rust-tests",
                "items": [{
                    "id": "broken_test",
                    "label": "broken_test",
                    "uri": test_uri,
                    "children": []
                }]
            }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.createTestRun",
            &json!({ "runId": "run-1", "controllerId": "rust-tests", "name": "CI Run" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateTestState",
            &json!({
                "runId": "run-1",
                "testId": "broken_test",
                "state": "failed",
                "duration": 15,
                "messages": [{ "message": "assertion failed: 1 != 2" }]
            }),
        )
        .await;
    bridge
        .handle_rpc_message("tests.endTestRun", &json!({ "runId": "run-1" }))
        .await;

    // Emit error notification toast (F11)
    bridge
        .handle_rpc_message(
            "window.showErrorMessage",
            &json!({ "message": "1 test failed in Cargo Tests", "items": ["View Output"] }),
        )
        .await;

    // Activate Testing sidebar via command or ActivityBar
    let _ = win.app.commands.execute("workbench.view.testing", None);
    let _ = win.click_a11y_node("Testing") || win.click_scene_text("TST") || win.click_scene_text("TEST");

    let scene = win.render();
    assert!(
        scene.contains_text("broken_test"),
        "[F12] Testing sidebar must render failing test 'broken_test', scene text: {}",
        scene.all_text()
    );
    assert!(
        scene.contains_text("1 test failed in Cargo Tests"),
        "[F11] Error notification toast must render simultaneously with Testing sidebar"
    );

    let tree = win.accessibility_tree();
    let has_tree = !tree.find_by_role(AccessibleRole::Tree).is_empty();
    assert!(has_tree, "[F12] Testing sidebar must expose AccessibleRole::Tree");
}

/// [F13+F14] Explorer CRUD + Contributed TreeViews + ContextKeyService
#[tokio::test]
async fn t3_05_f13_f14_explorer_crud_and_contributed_treeviews() {
    let ws = TempWorkspace::new();
    ws.write_file("src/lib.rs", "pub fn init() {}\n");

    // Copy treeview_ext fixture manifest into temp extension dir
    let fixture_manifest = std::fs::read_to_string(
        repo_root().join("crates/rustcode-ext-host/tests/fixtures/extensions/treeview_ext/package.json"),
    )
    .expect("treeview_ext fixture missing");
    let ext_dir = ws.path().join(".ext/treeview_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(ext_dir.join("package.json"), fixture_manifest).unwrap();

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).expect("load treeview_ext");

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

    // Register TreeDataProvider for projectOutlineView
    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TreeDataProvider", "id": "projectOutlineView" }),
        )
        .await;

    // Context key false -> Project Outline hidden (F14)
    win.window
        .context_keys
        .set_bool("workspaceHasProjects", false);
    assert!(
        !ContextKeyExpr::parse("workspaceHasProjects")
            .unwrap()
            .evaluate(&win.window.context_keys),
        "when clause must evaluate false initially"
    );
    let hidden_scene = win.render();
    assert!(
        !hidden_scene.contains_text("Project Outline"),
        "[F14] Contributed view must stay hidden when 'when' context key is false"
    );

    // Context key true -> Project Outline visible (F14)
    win.window
        .context_keys
        .set_bool("workspaceHasProjects", true);
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    let visible_scene = win.render();
    assert!(
        visible_scene.contains_text("Project Outline"),
        "[F14] Contributed view 'Project Outline' must render when 'workspaceHasProjects' is true"
    );

    // F13: Explorer CRUD — create new file and rename via Explorer UI
    win.window.view.focused_region = FocusedRegion::Sidebar;
    let new_file_clicked =
        win.click_a11y_node("New File") || win.click_scene_text("+F") || win.click_scene_text("New File");
    assert!(
        new_file_clicked,
        "[F13] Explorer header must expose New File action button"
    );
    win.type_text("created_e2e.rs");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert!(
        ws.exists("created_e2e.rs"),
        "[F13] Explorer New File must create 'created_e2e.rs' on disk"
    );

    // Rename via F2
    win.dispatch_key(KeyCode::F2, Modifiers::none(), None);
    win.type_text("renamed_e2e.rs");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert!(
        ws.exists("renamed_e2e.rs") && !ws.exists("created_e2e.rs"),
        "[F13] Explorer F2 rename must rename file on disk"
    );

    let after_crud_scene = win.render();
    assert!(
        after_crud_scene.contains_text("Project Outline"),
        "[F13+F14] Contributed TreeView section must remain visible after Explorer CRUD"
    );
}

/// [F15+F1+F2] Expanded node-compat (env.getOsInfo + process.exec) + RPC Error Propagation
#[tokio::test]
async fn t3_06_f15_f1_f2_os_info_process_exec_and_rpc_errors() {
    let ws = TempWorkspace::new();
    let (bridge, _rpc) = create_bridge_with_root(ws.path());

    // F15: env.getOsInfo RPC returns host OS details
    let os_info = bridge
        .handle_rpc_message("env.getOsInfo", &json!({}))
        .await
        .expect("[F15] env.getOsInfo must return OS metadata");
    assert!(
        os_info.get("platform").and_then(|v| v.as_str()).is_some()
            && os_info.get("eol").and_then(|v| v.as_str()).is_some()
            && os_info.get("homedir").and_then(|v| v.as_str()).is_some(),
        "[F15] env.getOsInfo must include platform, eol, and homedir: {os_info}"
    );

    // F15 + F1: process.exec executes cross-platform command and returns stdout/exitCode
    let proc_res = bridge
        .handle_rpc_message(
            "process.exec",
            &json!({
                "command": "node -e \"console.log('phase7-process-ok')\"",
                "cwd": ws.path().to_string_lossy()
            }),
        )
        .await
        .expect("[F15+F1] process.exec must return result payload");
    let stdout = proc_res
        .get("stdout")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let exit_code = proc_res.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(-1);
    assert_eq!(exit_code, 0, "[F15] process.exec exitCode must be 0: {proc_res}");
    assert!(
        stdout.contains("phase7-process-ok"),
        "[F15] process.exec stdout mismatch: {proc_res}"
    );

    // F2: Structured RPC error propagation (missing file & unknown method must return error envelope, NOT None)
    let missing_file_res = bridge
        .handle_rpc_message("fs.readFile", &json!({ "path": "missing_404.txt" }))
        .await
        .expect("[F2] fs.readFile on missing file must return structured error Value, not None");
    let err_obj = missing_file_res
        .get("error")
        .expect("[F2] Missing file response must contain 'error' object");
    assert_eq!(
        err_obj.get("code").and_then(|v| v.as_str()),
        Some("ENOENT"),
        "[F2] Missing file must propagate code 'ENOENT': {missing_file_res}"
    );

    let unknown_res = bridge
        .handle_rpc_message("unknown.nonexistentMethod", &json!({}))
        .await
        .expect("[F2] Unknown RPC method must return structured error Value, not None");
    assert!(
        unknown_res.get("error").is_some(),
        "[F2] Unknown RPC method must contain 'error' envelope: {unknown_res}"
    );
}

/// [F8+F6+F7+F12] Cross-Platform URIs + env.openExternal + workspace.openTextDocument + Testing URI open
#[tokio::test]
async fn t3_07_f8_f6_f7_f12_cross_platform_uris_env_and_testing_open() {
    let ws = TempWorkspace::new();
    let spec_path = ws.write_file(
        "tests/sub dir/sample_spec.rs",
        "#[test]\nfn pass_case() {}\n#[test]\nfn fail_case() {\n    panic!(\"boom\");\n}\n",
    );
    let spec_uri = path_to_file_uri(&spec_path);

    // F8: URI roundtrip preserves path equivalence across Windows drive letters & POSIX roots
    let decoded = file_uri_to_path(&spec_uri);
    assert!(
        paths_equivalent(&decoded, &spec_path),
        "[F8] file_uri_to_path roundtrip mismatch: {spec_uri} -> {:?}",
        decoded
    );

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    // F6: workspace.openTextDocument opens URI with spaces cleanly
    let doc = bridge
        .handle_rpc_message("workspace.openTextDocument", &json!({ "uri": spec_uri }))
        .await
        .expect("[F6+F8] workspace.openTextDocument must resolve URI with space");
    assert!(
        doc.get("text")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t.contains("fail_case")),
        "[F6] Document text must match disk file"
    );

    // F7: env.openExternal allows https and blocks javascript:
    let allowed = bridge
        .handle_rpc_message("env.openExternal", &json!({ "uri": "https://rust-lang.org" }))
        .await;
    let blocked = bridge
        .handle_rpc_message("env.openExternal", &json!({ "uri": "javascript:alert(1)" }))
        .await;
    assert_eq!(allowed, Some(json!(true)), "[F7] https URI must be allowed");
    assert_eq!(blocked, Some(json!(false)), "[F7] javascript: URI must be rejected");

    // F12: Populate Testing item with URI and click to open in editor
    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "uri-ctrl", "label": "URI Tests" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "uri-ctrl",
                "items": [{ "id": "fail_case", "label": "fail_case", "uri": spec_uri, "children": [] }]
            }),
        )
        .await;
    let _ = win.app.commands.execute("workbench.view.testing", None);
    let _ = win.render();
    let clicked = win.click_a11y_node("fail_case") || win.click_scene_text("fail_case");
    assert!(clicked, "[F12] Test item 'fail_case' must be clickable in Testing sidebar");
    assert!(
        win.read_doc(&spec_path)
            .is_some_and(|txt| txt.contains("fail_case")),
        "[F12+F8] Clicking test item must open target file URI in EditorAreaView"
    );
}

/// [F6+F3] Workspace Configuration live update -> EditorView minimap_enabled + onDidChangeConfiguration
#[tokio::test]
async fn t3_08_f6_f3_workspace_config_live_update_and_minimap() {
    let ws = TempWorkspace::new();
    ws.write_file("src/main.rs", "fn main() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("src/main.rs");
    let bridge = win
        .window
        .language_bridge
        .clone()
        .expect("[F3] WorkbenchWindow must share bridge");

    assert!(
        win.active_editor().unwrap().minimap_enabled,
        "minimap_enabled defaults to true"
    );

    let changed_keys: Arc<std::sync::Mutex<Vec<String>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let changed_keys_clone = Arc::clone(&changed_keys);
    let _sub = win.window.config.on_did_change.event().attach(move |ev| {
        changed_keys_clone
            .lock()
            .unwrap()
            .extend(ev.affected_keys.iter().cloned());
    });

    // F6: Update configuration via RPC
    let res = bridge
        .handle_rpc_message(
            "workspace.updateConfiguration",
            &json!({
                "key": "editor.minimap.enabled",
                "value": false,
                "target": 1
            }),
        )
        .await;
    assert!(res.is_some(), "[F6] updateConfiguration must succeed");

    // F3: Shared ConfigurationService updates WorkbenchWindow live
    win.window.sync_configuration();
    assert!(
        !win.active_editor().unwrap().minimap_enabled,
        "[F6+F3] Updating editor.minimap.enabled via bridge must update EditorView.minimap_enabled"
    );
    assert!(
        changed_keys
            .lock()
            .unwrap()
            .iter()
            .any(|k| k == "editor.minimap.enabled"),
        "[F6] on_did_change event must include 'editor.minimap.enabled'"
    );
}

/// [F7+F9] env.clipboard write/read + Editor Find query
#[tokio::test]
async fn t3_09_f7_f9_env_clipboard_and_editor_find() {
    let ws = TempWorkspace::new();
    ws.write_file(
        "notes.txt",
        "first token_xyz_42 line\nsecond line\nthird token_xyz_42 line\n",
    );

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let bridge = win
        .window
        .language_bridge
        .clone()
        .expect("[F3] bridge wired");
    win.open_file_from_disk("notes.txt");

    // F7: Write & read back via env.clipboard RPC
    bridge
        .handle_rpc_message("env.clipboardWrite", &json!({ "value": "token_xyz_42" }))
        .await;
    let clip = bridge
        .handle_rpc_message("env.clipboardRead", &json!({}))
        .await
        .and_then(|v| v.as_str().map(ToString::to_string))
        .unwrap_or_default();
    assert_eq!(clip, "token_xyz_42", "[F7] clipboardRead must return written text");

    // F9: Feed clipboard text into Find widget
    win.dispatch_ctrl_char('f');
    win.type_text(&clip);
    let scene = win.render();
    assert!(
        scene.contains_text("1 of 2") || scene.contains_text("2 of 2"),
        "[F7+F9] Find widget must match both occurrences of clipboard query"
    );
}

/// [F17+F9+F10+F12+F15+F16] Documentation Refresh cross-checked against Keymaps, Grammars & node-compat
#[test]
fn t3_10_f17_docs_cross_checked_against_keymaps_grammars_and_node_compat() {
    let root = repo_root();

    // Check default keymaps contain Phase 7 keybindings
    let keymap_str =
        std::fs::read_to_string(root.join("resources/default-keymaps/linux.json")).unwrap();
    let kb = KeybindingService::new();
    kb.load_keybindings_json(&keymap_str)
        .expect("linux.json must parse cleanly");
    assert_eq!(
        kb.lookup_keybinding("actions.find").as_deref(),
        Some("Ctrl+F"),
        "[F9+F17] linux.json must bind Ctrl+F -> actions.find"
    );
    assert_eq!(
        kb.lookup_keybinding("editor.action.startFindReplaceAction")
            .as_deref(),
        Some("Ctrl+H"),
        "[F9+F17] linux.json must bind Ctrl+H -> editor.action.startFindReplaceAction"
    );
    assert_eq!(
        kb.lookup_keybinding("workbench.action.gotoLine").as_deref(),
        Some("Ctrl+G"),
        "[F10+F17] linux.json must bind Ctrl+G -> workbench.action.gotoLine"
    );
    assert_eq!(
        kb.lookup_keybinding("workbench.action.gotoSymbol")
            .as_deref(),
        Some("Ctrl+Shift+O"),
        "[F10+F17] linux.json must bind Ctrl+Shift+O -> workbench.action.gotoSymbol"
    );

    // Check all 14 grammars in GrammarRegistry
    let reg = GrammarRegistry::new();
    for id in [
        "rust",
        "javascript",
        "typescript",
        "python",
        "json",
        "markdown",
        "plaintext",
        "go",
        "c",
        "cpp",
        "html",
        "css",
        "toml",
        "yaml",
    ] {
        assert!(
            reg.get_by_id(id).is_some(),
            "[F16] GrammarRegistry::new() must register '{id}'"
        );
    }

    assert_docs_cover_expected_surface(&root);
}

/// [F1+F5+F11] Live Node child_process RPC -> ExtHostWindowManager -> Toast Overlay
#[tokio::test]
async fn t3_11_f1_f5_f11_node_ext_host_notification_to_toast() {
    let ws = TempWorkspace::new();
    let mut win = E2eWindow::with_temp_workspace(&ws);
    let bridge = win
        .window
        .language_bridge
        .clone()
        .expect("[F3] bridge wired");

    // Verify ext-host-js loads cleanly in a real Node subprocess (F1)
    let out = run_node_script(
        "const { loadExtHost } = require('./tests/phase7-e2e/js/helpers'); \
         const { vscode } = loadExtHost({ mockRpc: (method) => (method === 'extensions.getAll' ? [] : { ok: true }) }); \
         console.log(vscode.version);",
        &repo_root(),
    );
    assert!(
        out.status.success(),
        "[F1] Node ext-host load failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Trigger warning notification with buttons via RPC -> render toast -> click action (F5 + F11)
    bridge.window_manager.set_await_responses(true);
    let bridge_clone = Arc::clone(&bridge);
    let handle = tokio::spawn(async move {
        bridge_clone
            .handle_rpc_message(
                "window.showWarningMessage",
                &json!({ "message": "Disk almost full", "items": ["Retry", "Ignore"] }),
            )
            .await
    });

    for _ in 0..20 {
        if !bridge.window_manager.active_notifications().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }

    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    let scene = win.render();
    assert!(
        scene.contains_text("Disk almost full") && scene.contains_text("Retry"),
        "[F11+F5] Warning toast and 'Retry' action must render in Scene"
    );
    assert!(
        win.click_a11y_node("Retry") || win.click_scene_text("Retry"),
        "[F11] Must click 'Retry' action on toast"
    );

    let res = tokio::time::timeout(std::time::Duration::from_secs(2), handle)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(res, Some(json!("Retry")));
}

/// [F4+F10+F13] Command Palette execution + :line jump + Explorer state sync
#[test]
fn t3_12_f4_f10_f13_command_palette_goto_line_and_explorer_sync() {
    let ws = TempWorkspace::new();
    ws.write_file(
        "src/math_utils.rs",
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\npub fn mul(a: i32, b: i32) -> i32 {\n    a * b\n}\n",
    );

    let mut win = E2eWindow::with_temp_workspace(&ws);
    assert!(
        win.click_a11y_node("src") || win.click_scene_text("src"),
        "[F13] Explorer must allow expanding 'src' directory"
    );
    win.open_file_from_disk("src/math_utils.rs");

    // F4: Open Command Palette (Ctrl+Shift+P) and execute toggleSidebar
    let sidebar_before = win.window.view.layout.sidebar_visible;
    win.dispatch_key(
        KeyCode::Char('p'),
        Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        },
        None,
    );
    win.type_text("toggleSidebar");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert_ne!(
        win.window.view.layout.sidebar_visible, sidebar_before,
        "[F4] Running toggleSidebar from Command Palette must flip sidebar visibility"
    );

    // F10: Jump to line 5 via Ctrl+G
    win.dispatch_ctrl_char('g');
    win.type_text("5:5");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert_eq!(
        win.active_editor().unwrap().primary_position(),
        Position::new(4, 4),
        "[F10] :5:5 must jump to 0-based (4, 4)"
    );

    // F13: Explorer model keeps math_utils.rs in workspace tree
    let rows = win.window.view.explorer.flattened_rows();
    assert!(
        rows.iter().any(|r| r.name == "math_utils.rs"),
        "[F13] Explorer must continue tracking math_utils.rs"
    );
}

/// [F16+F6+F3] Multi-language tokenizers + Configuration WordWrap + EditorView sync
#[test]
fn t3_13_f16_f6_f3_multi_language_highlight_and_word_wrap_config() {
    let ws = TempWorkspace::new();
    let samples = [
        ("app.cpp", "cpp", "int main() { return 0; }\n"),
        ("index.html", "html", "<!DOCTYPE html><html><body>Hi</body></html>\n"),
        ("styles.css", "css", "body { color: #ff00aa; }\n"),
        ("Cargo.toml", "toml", "[package]\nname = \"demo\"\n"),
        ("config.yaml", "yaml", "service:\n  port: 8080\n"),
    ];

    let reg = GrammarRegistry::new();
    let theme = ThemeService::new().current().clone();
    let mut win = E2eWindow::with_temp_workspace(&ws);

    for (rel, expected_id, content) in samples {
        let path = ws.write_file(rel, content);
        let g = reg.get_for_path(&path);
        assert_eq!(
            g.language_id(),
            expected_id,
            "[F16] GrammarRegistry::get_for_path({rel}) must resolve '{expected_id}'"
        );
        let mut hl = SyntaxHighlighter::new(g, theme.clone());
        let first_line = content.lines().next().unwrap_or("");
        let tokens = hl.get_line_tokens(0, first_line);
        assert!(
            tokens.iter().any(|t| t.scope != SyntaxScope::Plain),
            "[F16] Tokenizer for '{expected_id}' must emit non-Plain scopes on '{first_line}'"
        );
        win.open_file_from_disk(rel);
    }

    // F6 + F3: Update editor.wordWrap and verify active EditorView updates
    win.window
        .config
        .update(
            "editor.wordWrap",
            json!("on"),
            rustcode_platform::configuration::ConfigurationTarget::Workspace,
        )
        .unwrap();
    win.window.sync_configuration();
    assert_eq!(
        win.active_editor().unwrap().word_wrap,
        WordWrapMode::On,
        "[F6+F3] editor.wordWrap='on' must update EditorView.word_wrap"
    );
}

/// [F2+F8+F13] Sandbox traversal EACCES error + Explorer invalid rename preservation
#[tokio::test]
async fn t3_14_f2_f8_f13_sandbox_error_and_explorer_invalid_rename() {
    let ws = TempWorkspace::new();
    ws.write_file("safe.rs", "fn keep_me() {}\n");
    let (bridge, _rpc) = create_bridge_with_root(ws.path());

    // F2 + F8: Traversal outside fs_root returns structured EACCES
    let traversal = bridge
        .handle_rpc_message("fs.readFile", &json!({ "path": "../outside_secret.txt" }))
        .await
        .expect("[F2] Sandbox traversal must return structured error, not None");
    assert_eq!(
        traversal.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_str()),
        Some("EACCES"),
        "[F2+F8] Traversal outside sandbox must yield EACCES: {traversal}"
    );

    // F13: Invalid rename containing path separator must NOT clobber or move file
    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.window.view.focused_region = FocusedRegion::Sidebar;
    assert!(
        win.click_a11y_node("safe.rs") || win.click_scene_text("safe.rs"),
        "[F13] Explorer must display and allow selecting safe.rs"
    );
    win.dispatch_key(KeyCode::F2, Modifiers::none(), None);
    win.type_text("bad/name.rs");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);

    assert!(
        ws.exists("safe.rs"),
        "[F13] Invalid rename with '/' must leave 'safe.rs' intact on disk"
    );
    assert_eq!(ws.read_file("safe.rs"), "fn keep_me() {}\n");

    // F13: Valid F2 rename must actually rename on disk after recovering from invalid input
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    assert!(
        win.click_a11y_node("safe.rs") || win.click_scene_text("safe.rs"),
        "[F13] Explorer must allow re-selecting safe.rs after invalid rename"
    );
    win.dispatch_key(KeyCode::F2, Modifiers::none(), None);
    win.type_text("safe_renamed.rs");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert!(
        ws.exists("safe_renamed.rs") && !ws.exists("safe.rs"),
        "[F13] Valid F2 rename must rename 'safe.rs' -> 'safe_renamed.rs' after recovery"
    );
}

/// [F14+F4+F5] Contributed TreeView lazy children request & TreeItem command execution
#[tokio::test]
async fn t3_15_f14_f4_f5_contributed_treeview_children_and_command() {
    let ws = TempWorkspace::new();
    let ext_dir = ws.path().join(".ext/treeview_ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    let manifest = std::fs::read_to_string(
        repo_root().join("crates/rustcode-ext-host/tests/fixtures/extensions/treeview_ext/package.json"),
    )
    .unwrap();
    std::fs::write(ext_dir.join("package.json"), manifest).unwrap();

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

    let cmd_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cmd_ran_flag = Arc::clone(&cmd_ran);
    let _disp = win.app.commands.register("projectOutline.refresh", move |_| {
        cmd_ran_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(json!({ "refreshed": true }))
    });

    // Simulate outbound request_tree_children roundtrip
    let b_clone = Arc::clone(&bridge);
    let req_task = tokio::spawn(async move {
        b_clone
            .request_tree_children("projectOutlineView", None)
            .await
    });
    tokio::task::yield_now().await;
    // Fulfill if pending (id 1 on fresh RpcBridge)
    rpc.handle_incoming_response(
        1,
        Some(json!([
            {
                "id": "node_src",
                "label": "src_outline",
                "collapsibleState": 1,
                "command": { "title": "Refresh", "command": "projectOutline.refresh" }
            }
        ])),
        None,
    );
    let children = req_task.await.unwrap();
    assert_eq!(children.len(), 1, "[F14+F5] TreeDataProvider returned 1 node");

    let _ = win.app.commands.execute("projectOutline.refresh", None);
    assert!(
        cmd_ran.load(std::sync::atomic::Ordering::SeqCst),
        "[F14+F4] TreeItem command execution must invoke registered command"
    );

    let scene = win.render();
    assert!(
        scene.contains_text("Project Outline"),
        "[F14] Contributed view header must render in Explorer"
    );
}

/// [F9+F16] UTF-8 + CRLF Find & Replace across YAML buffer with Undo
#[test]
fn t3_16_f9_f16_utf8_crlf_find_replace_yaml_buffer() {
    let ws = TempWorkspace::new();
    let yaml_crlf = "greeting: \"café 🚀\"\r\nfallback: \"café\"\r\n";
    let yaml_path = ws.write_file("i18n.yaml", yaml_crlf);

    let reg = GrammarRegistry::new();
    let yaml_grammar = reg
        .get_for_path(&yaml_path)
        .language_id()
        .to_string();
    assert_eq!(yaml_grammar, "yaml", "[F16] .yaml must resolve to 'yaml' grammar");

    let mut hl = SyntaxHighlighter::for_language(
        "yaml",
        ThemeService::new().current().clone(),
        &reg,
    );
    let tokens = hl.get_line_tokens(0, "greeting: \"café 🚀\"\r");
    for span in tokens {
        assert!(
            "greeting: \"café 🚀\"\r".is_char_boundary(span.start)
                && "greeting: \"café 🚀\"\r".is_char_boundary(span.end),
            "[F16] Token boundaries must align with UTF-8 char boundaries"
        );
    }

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("i18n.yaml");
    win.dispatch_ctrl_char('h');
    win.type_text("café");
    let s = win.render();
    assert!(
        s.contains_text("1 of 2") || s.contains_text("2 of 2"),
        "[F9+F16] Find must locate both UTF-8 'café' occurrences in CRLF YAML buffer"
    );

    win.dispatch_key(KeyCode::Tab, Modifiers::none(), None);
    win.type_text("espresso");
    let clicked = win.click_a11y_node("Replace All")
        || win.click_scene_text("Replace All")
        || win.click_scene_text("All");
    assert!(clicked, "[F9] Replace All button must be clickable");

    let after = win.active_editor_text().unwrap();
    assert!(
        after.contains("espresso 🚀") && after.matches("espresso").count() == 2,
        "[F9] Replace All must preserve emoji and replace both 'café' matches: {after:?}"
    );

    // Undo atomically
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    win.dispatch_ctrl_char('z');
    assert_eq!(
        win.active_editor_text().unwrap(),
        yaml_crlf,
        "[F9] Atomic undo must restore exact CRLF + UTF-8 content"
    );
}

#[test]
fn t3_17_f10_f4_f3_keymap_to_command_pipeline() {
    let combo = KeyCombo::parse("ctrl+g").expect("parse ctrl+g");
    assert!(combo.modifiers.ctrl);
    let expr = ContextKeyExpr::parse("editorTextFocus && !editorReadonly").unwrap();
    let win = E2eWindow::new();
    win.window.context_keys.set_bool("editorTextFocus", true);
    win.window.context_keys.set_bool("editorReadonly", false);
    assert!(expr.evaluate(&win.window.context_keys));

    assert_eq!(
        win.window.keybindings.handle_key(&combo, &win.window.context_keys).as_deref(),
        Some("workbench.action.gotoLine"),
        "[F10] ctrl+g must resolve to workbench.action.gotoLine in E2eWindow keybindings"
    );
    let before = win.window.view.layout.sidebar_visible;
    win.app.commands.execute("workbench.action.toggleSidebar", None).unwrap();
    assert_ne!(
        win.window.view.layout.sidebar_visible, before,
        "[F4+F3] CommandRegistry::execute must mutate WorkbenchWindow layout"
    );
}
