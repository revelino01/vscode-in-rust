use phase7_e2e::{
    create_bridge_with_root, repo_root, run_node_script, run_rustcode_cli, E2eWindow,
    TempWorkspace,
};
use rustcode_base::context_key::ContextKeyExpr;
use rustcode_editor::{GrammarRegistry, Position, SyntaxHighlighter, SyntaxScope, WordWrapMode};
use rustcode_ext_host::{ExtensionLoader, LanguageFeatureBridge};
use rustcode_platform::accessibility::AccessibleRole;
use rustcode_platform::configuration::ConfigurationTarget;
use rustcode_platform::fs::{file_uri_to_path, path_to_file_uri, paths_equivalent};
use rustcode_platform::keybindings::{KeybindingService, Modifiers};
use rustcode_platform::render::KeyCode;
use rustcode_platform::theme::ThemeService;
use rustcode_workbench::search::search_document_symbols;
use rustcode_workbench::workbench_view::FocusedRegion;
use serde_json::json;
use std::sync::Arc;

/// Scenario 1: Extension Activation & Interactive Command Workflow
/// (`resources/sample-extensions/hello-world` + F1 + F3 + F4 + F5 + F7 + F11)
#[tokio::test]
async fn scenario_01_extension_activation_and_interactive_command_workflow() {
    let ext_src = repo_root().join("resources/sample-extensions/hello-world");
    let mut loader = ExtensionLoader::new();
    let loaded = loader
        .load_from_dir(&ext_src)
        .expect("sample hello-world extension must load");
    let ext_id = format!("{}.{}", loaded.manifest.publisher, loaded.manifest.name);
    assert_eq!(ext_id, "rustcode.sample-hello-world");

    let mut win = E2eWindow::new();
    // F3: RustCodeContext::create_window() wires LanguageFeatureBridge
    assert!(
        win.window.language_bridge.is_some(),
        "[F3] create_window() must wire LanguageFeatureBridge"
    );
    let bridge = win.window.language_bridge.clone().unwrap();

    // F7: Hydrate ExtHostEnvManager from ExtensionLoader and activate hello-world
    bridge.env_manager.hydrate_from_loader(&loader);
    let ext_info = bridge
        .handle_rpc_message("extensions.getExtension", &json!({ "id": ext_id }))
        .await
        .expect("[F7] extensions.getExtension must return snapshot");
    assert_eq!(
        ext_info.get("id").and_then(|v| v.as_str()),
        Some("rustcode.sample-hello-world")
    );
    let act_res = bridge
        .handle_rpc_message("extensions.activate", &json!({ "id": ext_id }))
        .await
        .expect("[F7] extensions.activate must succeed");
    assert_eq!(act_res.get("activated").and_then(|v| v.as_bool()), Some(true));

    // F1 + F4 + F5: Execute sample-hello-world in Node ext-host, trigger showInformationMessage
    let node_out = run_node_script(
        "const path = require('path'); \
         const { loadExtension, repoRoot } = require('./tests/phase7-e2e/js/helpers'); \
         const calls = []; \
         const ext = loadExtension(path.join(repoRoot(), 'resources/sample-extensions/hello-world'), { \
           mockRpc: (method, params) => { calls.push({ method, params }); return method === 'extensions.getAll' ? [] : 'OK'; } \
         }); \
         const ctx = { subscriptions: [] }; \
         ext.exports.activate(ctx); \
         ext.vscode.commands.executeCommand('sample.helloWorld').then(() => { \
           console.log(JSON.stringify(calls)); \
         });",
        &repo_root(),
    );
    assert!(
        node_out.status.success(),
        "[F1+F4] Node hello-world execution failed: {}",
        String::from_utf8_lossy(&node_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&node_out.stdout);
    assert!(
        stdout.contains("Hello World from RustCode Extension Host!"),
        "[F4+F5] Expected hello-world notification call in Node output, got: {stdout}"
    );

    // F11: Render notification toast in WorkbenchWindow and dismiss
    bridge
        .handle_rpc_message(
            "window.showInformationMessage",
            &json!({
                "message": "Hello World from RustCode Extension Host!",
                "items": ["Acknowledge"]
            }),
        )
        .await;
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    let scene = win.render();
    assert!(
        scene.contains_text("Hello World from RustCode Extension Host!"),
        "[F11] Notification toast must render in WorkbenchWindow Scene"
    );
    assert!(
        win.click_a11y_node("Acknowledge") || win.click_scene_text("Acknowledge"),
        "[F11] Must click 'Acknowledge' on notification toast"
    );
    assert!(
        !win.render()
            .contains_text("Hello World from RustCode Extension Host!"),
        "[F11] Dismissed toast must disappear from rendered scene"
    );
}

/// Scenario 2: Multi-File Refactoring Session
/// (Explorer CRUD create/rename + Find/Replace + Atomic Undo + Save + Doc Events: F3 + F6 + F9 + F13)
#[tokio::test]
async fn scenario_02_multi_file_refactoring_session() {
    let ws = TempWorkspace::new();
    ws.write_file(
        "src/legacy.rs",
        "pub fn old_compute(v: i32) -> i32 { v * 2 }\npub fn caller() -> i32 { old_compute(21) }\n",
    );

    let mut win = E2eWindow::with_temp_workspace(&ws);
    assert!(
        win.window.language_bridge.is_some(),
        "[F3] create_window() must wire bridge"
    );

    // F13: Rename src/legacy.rs -> modern.rs via Explorer F2, and create notes.md
    win.window.view.focused_region = FocusedRegion::Sidebar;
    assert!(
        win.click_a11y_node("src") || win.click_scene_text("src"),
        "[F13] Explorer must allow expanding 'src' directory"
    );
    let selected = win.click_a11y_node("legacy.rs") || win.click_scene_text("legacy.rs");
    assert!(selected, "[F13] Explorer must display legacy.rs");
    win.dispatch_key(KeyCode::F2, Modifiers::none(), None);
    win.type_text("modern.rs");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert!(
        ws.exists("src/modern.rs") && !ws.exists("src/legacy.rs"),
        "[F13] Explorer F2 rename must move src/legacy.rs -> src/modern.rs on disk"
    );

    // Open renamed file and perform Find & Replace (Ctrl+H)
    win.open_file_from_disk("src/modern.rs");
    win.dispatch_ctrl_char('h');
    win.type_text("old_compute");
    let find_scene = win.render();
    assert!(
        find_scene.contains_text("1 of 2") || find_scene.contains_text("2 of 2"),
        "[F9] Find & Replace must find 2 occurrences of old_compute"
    );
    win.dispatch_key(KeyCode::Tab, Modifiers::none(), None);
    win.type_text("new_compute");
    let clicked = win.click_a11y_node("Replace All")
        || win.click_scene_text("Replace All")
        || win.click_scene_text("All");
    assert!(clicked, "[F9] Replace All button must activate");
    assert_eq!(
        win.active_editor_text().unwrap().matches("new_compute").count(),
        2
    );

    // Atomic undo (Ctrl+Z), redo (Ctrl+Y), and Save (Ctrl+S) (F9 + F6)
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    win.dispatch_ctrl_char('z');
    assert_eq!(
        win.active_editor_text().unwrap().matches("old_compute").count(),
        2,
        "[F9] Single Ctrl+Z must atomically revert Replace All"
    );
    win.dispatch_ctrl_char('y');
    win.dispatch_ctrl_char('s');

    let saved = ws.read_file("src/modern.rs");
    assert!(
        saved.contains("new_compute") && !saved.contains("old_compute"),
        "[F6] Ctrl+S must persist refactored file to disk"
    );
}

/// Scenario 3: Polyglot Code Navigation & Syntax Highlighting
/// (Go/C++/HTML/TOML/YAML + @symbol + :line:col: F10 + F16)
#[test]
fn scenario_03_polyglot_code_navigation_and_syntax_highlighting() {
    let ws = TempWorkspace::new();
    let files = [
        (
            "main.go",
            "go",
            "package main\n\nfunc ComputeSum(a int, b int) int {\n\treturn a + b\n}\n\nfunc main() {\n\t_ = ComputeSum(2, 3)\n}\n",
        ),
        ("engine.cpp", "cpp", "#include <vector>\nint main() { return 0; }\n"),
        ("index.html", "html", "<html><body><h1>RustCode</h1></body></html>\n"),
        ("Cargo.toml", "toml", "[package]\nname = \"polyglot\"\nversion = \"0.1.0\"\n"),
        ("deploy.yaml", "yaml", "replicas: 3\nimage: \"rustcode:latest\"\n"),
    ];

    let reg = GrammarRegistry::new();
    let theme = ThemeService::new().current().clone();
    for (rel, expected_lang, body) in files {
        let p = ws.write_file(rel, body);
        let grammar = reg.get_for_path(&p);
        assert_eq!(
            grammar.language_id(),
            expected_lang,
            "[F16] Expected grammar '{expected_lang}' for {rel}"
        );
        let mut hl = SyntaxHighlighter::new(grammar, theme.clone());
        let first_line = body.lines().next().unwrap_or("");
        let spans = hl.get_line_tokens(0, first_line);
        assert!(
            spans.iter().any(|s| s.scope != SyntaxScope::Plain),
            "[F16] Polyglot line '{first_line}' ({expected_lang}) must highlight"
        );
    }

    let go_path = ws.path().join("main.go");
    let go_syms = search_document_symbols(&ws.read_file("main.go"), &go_path);
    assert!(
        go_syms.iter().any(|s| s.name == "ComputeSum"),
        "[F10] search_document_symbols must extract Go function ComputeSum"
    );

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("main.go");

    // Ctrl+Shift+O -> @ComputeSum -> Enter
    win.dispatch_key(
        KeyCode::Char('o'),
        Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        },
        None,
    );
    win.type_text("ComputeSum");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert_eq!(
        win.active_editor().unwrap().primary_position().line,
        2,
        "[F10] @ComputeSum must jump to line index 2"
    );

    // Ctrl+G -> 8:6 -> Enter
    win.dispatch_ctrl_char('g');
    win.type_text("8:6");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert_eq!(
        win.active_editor().unwrap().primary_position(),
        Position::new(7, 5),
        "[F10] :8:6 must jump to 0-based (7, 5)"
    );
}

/// Scenario 4: Extension-Driven Test Discovery, Run, Failure Inspection & Jump-to-Source
/// (F1 + F3 + F5 + F12)
#[tokio::test]
async fn scenario_04_extension_driven_test_discovery_run_and_jump_to_source() {
    let ws = TempWorkspace::new();
    let test_path = ws.write_file(
        "tests/math_test.rs",
        "#[test]\nfn test_add() { assert_eq!(1 + 1, 2); }\n\n#[test]\nfn test_div() {\n    assert_eq!(4 / 2, 3);\n}\n",
    );
    let test_uri = path_to_file_uri(&test_path);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));

    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "suite-ctrl", "label": "Workspace Suite" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "suite-ctrl",
                "items": [
                    { "id": "test_add", "label": "test_add", "uri": test_uri, "children": [] },
                    { "id": "test_div", "label": "test_div", "uri": test_uri, "children": [] }
                ]
            }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.createTestRun",
            &json!({ "runId": "r1", "controllerId": "suite-ctrl", "name": "Run 1" }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateTestState",
            &json!({ "runId": "r1", "testId": "test_add", "state": "passed", "duration": 4 }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "tests.updateTestState",
            &json!({
                "runId": "r1",
                "testId": "test_div",
                "state": "failed",
                "duration": 9,
                "messages": [{ "message": "expected 2, got 3" }]
            }),
        )
        .await;
    bridge
        .handle_rpc_message("tests.endTestRun", &json!({ "runId": "r1" }))
        .await;

    let _ = win.app.commands.execute("workbench.view.testing", None);
    let _ = win.click_a11y_node("Testing") || win.click_scene_text("TST") || win.click_scene_text("TEST");
    let scene = win.render();
    assert!(
        scene.contains_text("test_add") && scene.contains_text("test_div"),
        "[F12] Testing sidebar must display discovered tests"
    );
    assert!(
        scene.contains_text("expected 2, got 3") || scene.contains_text("failed"),
        "[F12] Testing sidebar must show failure detail"
    );

    // Click failing test to jump to source file
    let clicked = win.click_a11y_node("test_div") || win.click_scene_text("test_div");
    assert!(clicked, "[F12] Failing test item must be clickable");
    assert!(
        win.read_doc(&test_path)
            .is_some_and(|txt| txt.contains("fn test_div")),
        "[F12+F3] Clicking test item must open tests/math_test.rs in editor"
    );
}

/// Scenario 5: Contributed TreeView Workspace Outline
/// (`treeview_ext` + ContextKeyService `when` + Command + Editor: F4 + F5 + F14)
#[tokio::test]
async fn scenario_05_contributed_treeview_workspace_outline() {
    let ws = TempWorkspace::new();
    ws.write_file("src/app.rs", "fn main() {}\n");

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

    // ContextKey false -> hidden
    win.window
        .context_keys
        .set_bool("workspaceHasProjects", false);
    assert!(
        !ContextKeyExpr::parse("workspaceHasProjects")
            .unwrap()
            .evaluate(&win.window.context_keys)
    );
    assert!(
        !win.render().contains_text("Project Outline"),
        "[F14] Contributed view must be hidden when when-clause is false"
    );

    // ContextKey true + TreeDataProvider registered -> visible
    win.window
        .context_keys
        .set_bool("workspaceHasProjects", true);
    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TreeDataProvider", "id": "projectOutlineView" }),
        )
        .await;

    let scene = win.render();
    assert!(
        scene.contains_text("Project Outline"),
        "[F14+F5] Contributed TreeView 'Project Outline' must render when context key is true"
    );

    let refreshed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let refreshed_clone = Arc::clone(&refreshed);
    let _disp = win.app.commands.register("projectOutline.refresh", move |_| {
        refreshed_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(json!({ "ok": true }))
    });
    let _ = win.app.commands.execute("projectOutline.refresh", None);
    assert!(
        refreshed.load(std::sync::atomic::Ordering::SeqCst),
        "[F4] Outline refresh command must execute"
    );
}

/// Scenario 6: Cross-Platform Extension Sandbox & Node Polyfills
/// (`node-compat` + `process.exec` + `fs` + `url`/`path`: F1 + F2 + F8 + F15)
#[tokio::test]
async fn scenario_06_cross_platform_extension_sandbox_and_node_polyfills() {
    let ws = TempWorkspace::new();
    let (bridge, _rpc) = create_bridge_with_root(ws.path());

    // F8: Sandbox FS writeFile / readFile / mkdir / readdir / stat / unlink
    bridge
        .handle_rpc_message("fs.mkdir", &json!({ "path": "sandbox_dir", "options": { "recursive": true } }))
        .await;
    bridge
        .handle_rpc_message(
            "fs.writeFile",
            &json!({ "path": "sandbox_dir/hello.txt", "data": "sandbox-payload" }),
        )
        .await;
    let read_res = bridge
        .handle_rpc_message(
            "fs.readFile",
            &json!({ "path": "sandbox_dir/hello.txt", "encoding": "utf8" }),
        )
        .await;
    assert_eq!(read_res, Some(json!("sandbox-payload")));

    // F2: Escape attempt returns structured EACCES
    let esc = bridge
        .handle_rpc_message("fs.readFile", &json!({ "path": "../../etc/passwd" }))
        .await
        .expect("[F2] Escape attempt must return structured error");
    assert_eq!(
        esc.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_str()),
        Some("EACCES")
    );

    // F15 + F1: env.getOsInfo + process.exec executing Node script using node-compat polyfills
    let os_info = bridge
        .handle_rpc_message("env.getOsInfo", &json!({}))
        .await
        .expect("[F15] env.getOsInfo must return host OS info");
    assert!(os_info.get("platform").is_some());

    let root = repo_root();
    let root_norm = root.to_string_lossy().replace('\\', "/");
    let target_norm = ws
        .path()
        .join("sandbox_dir/hello.txt")
        .to_string_lossy()
        .replace('\\', "/");
    let script = format!(
        "const buf = require('{root_norm}/ext-host-js/node-compat/buffer');\n\
         const crypto = require('{root_norm}/ext-host-js/node-compat/crypto');\n\
         const url = require('{root_norm}/ext-host-js/node-compat/url');\n\
         const hash = crypto.createHash('sha256').update((buf.Buffer || buf).from('abc')).digest('hex');\n\
         const u = url.pathToFileURL({target_norm:?});\n\
         console.log(JSON.stringify({{ hash, href: u.href }}));\n"
    );
    ws.write_file("sandbox_dir/probe.js", &script);
    let proc_res = bridge
        .handle_rpc_message(
            "process.exec",
            &json!({ "command": "node sandbox_dir/probe.js", "cwd": ws.path() }),
        )
        .await
        .expect("[F15] process.exec must return result");
    assert_eq!(proc_res.get("exitCode").and_then(|v| v.as_i64()), Some(0));
    let out = proc_res.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        out.contains("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        "[F15] SHA-256('abc') NIST vector must match in node-compat/crypto.js, got: {out}"
    );
    assert!(
        out.contains("file:///"),
        "[F15] url.pathToFileURL must produce file:/// URL, got: {out}"
    );
}

/// Scenario 7: Workspace Settings Live Update & QuickInput Command/Prompt Flow
/// (F3 + F4 + F5 + F6 + F11)
#[tokio::test]
async fn scenario_07_workspace_settings_live_update_and_quickinput_prompt_flow() {
    let ws = TempWorkspace::new();
    ws.write_file("src/lib.rs", "pub fn demo() {}\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    win.open_file_from_disk("src/lib.rs");
    let bridge = win.window.language_bridge.clone().expect("[F3] bridge wired");

    // F6 + F3: Update configuration via RPC and verify live EditorView update
    bridge
        .handle_rpc_message(
            "workspace.updateConfiguration",
            &json!({ "key": "editor.minimap.enabled", "value": false, "target": 2 }),
        )
        .await;
    bridge
        .handle_rpc_message(
            "workspace.updateConfiguration",
            &json!({ "key": "editor.wordWrap", "value": "on", "target": 2 }),
        )
        .await;
    win.window.sync_configuration();
    let ed = win.active_editor().unwrap();
    assert!(!ed.minimap_enabled);
    assert_eq!(ed.word_wrap, WordWrapMode::On);

    // F4: Execute builtin command to toggle bottom panel
    let panel_before = win.window.view.layout.panel_visible;
    win.app
        .commands
        .execute("workbench.action.togglePanel", None)
        .unwrap();
    assert_ne!(
        win.window.view.layout.panel_visible, panel_before,
        "[F4] togglePanel must mutate WorkbenchWindow panel visibility"
    );

    // F5 + F11: QuickPick prompt resolved via WorkbenchWindow UI
    bridge.window_manager.set_await_responses(true);
    let b_clone = Arc::clone(&bridge);
    let qp_task = tokio::spawn(async move {
        b_clone
            .handle_rpc_message(
                "window.showQuickPick",
                &json!({
                    "items": ["Dark+", "Light+"],
                    "options": { "placeHolder": "Select theme" }
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

    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    let scene = win.render();
    assert!(
        scene.contains_text("Dark+") || scene.contains_text("Select theme"),
        "[F11+F5] QuickPick prompt must render in WorkbenchWindow"
    );
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    let picked = tokio::time::timeout(std::time::Duration::from_secs(2), qp_task)
        .await
        .expect("quickPick resolves")
        .unwrap();
    assert!(
        picked == Some(json!("Dark+")) || picked == Some(json!({ "label": "Dark+" })),
        "[F11+F5] Pressing Enter on QuickPick must resolve first item, got: {picked:?}"
    );
}

/// Scenario 8: Error Recovery & Diagnostics Workflow
/// (Bad FS/RPC calls + Invalid Regex Find + Invalid Explorer Rename + Notification: F2 + F8 + F9 + F11 + F13)
#[tokio::test]
async fn scenario_08_error_recovery_and_diagnostics_workflow() {
    let ws = TempWorkspace::new();
    ws.write_file("src/main.rs", "fn alpha() { let x = 1; }\n");

    let mut win = E2eWindow::with_temp_workspace(&ws);
    let (bridge, _rpc) = create_bridge_with_root(ws.path());
    win.window.set_language_bridge(Arc::clone(&bridge));
    win.open_file_from_disk("src/main.rs");

    // F2 + F8: Missing file & traversal return structured error without crashing
    let enoent = bridge
        .handle_rpc_message("fs.readFile", &json!({ "path": "no_such_file.rs" }))
        .await
        .expect("[F2] Missing file must return structured error");
    assert_eq!(
        enoent.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_str()),
        Some("ENOENT")
    );

    // F9: Invalid regex in Find widget does not panic, then recovers on valid query
    win.dispatch_ctrl_char('f');
    assert!(
        win.click_a11y_node("Use Regular Expression") || win.click_scene_text(".*"),
        "[F9] Regex toggle must be clickable"
    );
    win.type_text("[unclosed(");
    let err_scene = win.render();
    let err_lower = err_scene.all_text().to_ascii_lowercase();
    assert!(
        (err_scene.contains_text("No results")
            || err_lower.contains("invalid")
            || err_lower.contains("error"))
            && !err_scene.contains_text("1 of "),
        "[F9] Invalid regex must render safely without matching, got: {}",
        err_scene.all_text()
    );
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    win.dispatch_ctrl_char('f');
    assert!(
        win.click_a11y_node("Use Regular Expression") || win.click_scene_text(".*"),
        "[F9] Regex toggle must remain clickable after Escape"
    );
    win.type_text("alpha");
    assert!(
        win.render().contains_text("1 of 1"),
        "[F9] Find must recover cleanly after invalid regex"
    );

    // F13: Invalid Explorer rename leaves disk intact
    win.window.view.focused_region = FocusedRegion::Sidebar;
    assert!(
        win.click_a11y_node("src") || win.click_scene_text("src"),
        "[F13] Explorer must allow expanding 'src' directory"
    );
    assert!(
        win.click_a11y_node("main.rs") || win.click_scene_text("main.rs"),
        "[F13] Explorer must allow selecting 'main.rs'"
    );
    assert!(
        win.dispatch_key(KeyCode::F2, Modifiers::none(), None)
            .is_consumed(),
        "[F13] F2 must enter rename mode on selected Explorer file"
    );
    win.type_text("..");
    win.dispatch_key(KeyCode::Enter, Modifiers::none(), None);
    assert!(ws.exists("src/main.rs"), "[F13] Invalid rename '..' must not delete/move file");

    // F11: Error notification toast displays and dismisses cleanly
    bridge
        .handle_rpc_message(
            "window.showErrorMessage",
            &json!({ "message": "Recovered from invalid input" }),
        )
        .await;
    win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
    assert!(
        win.render().contains_text("Recovered from invalid input"),
        "[F11] Error toast must render"
    );
}

/// Scenario 9: Full Project CLI & Documentation Consistency Verification
/// (`rustcode` CLI + F17 Docs + Keymaps + Grammars: F4 + F10 + F16 + F17)
#[test]
fn scenario_09_full_project_cli_and_documentation_consistency() {
    // CLI binary runs cleanly end-to-end
    let out = run_rustcode_cli();
    assert!(
        out.status.success(),
        "rustcode CLI failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Rendered GPUI Scene:") && stdout.contains("Bootstrap completed successfully."),
        "Unexpected CLI stdout: {stdout}"
    );

    let root = repo_root();
    let keymap_json =
        std::fs::read_to_string(root.join("resources/default-keymaps/linux.json")).unwrap();
    let kb = KeybindingService::new();
    kb.load_keybindings_json(&keymap_json).unwrap();
    assert!(kb.lookup_keybinding("actions.find").is_some());
    assert!(kb.lookup_keybinding("workbench.action.gotoLine").is_some());
    assert!(kb.lookup_keybinding("workbench.action.gotoSymbol").is_some());

    let reg = GrammarRegistry::new();
    for lang in ["go", "c", "cpp", "html", "css", "toml", "yaml"] {
        assert!(reg.get_by_id(lang).is_some(), "[F16] missing grammar {lang}");
    }

    let matrix = std::fs::read_to_string(root.join("docs/extension-compat-matrix.md")).unwrap();
    let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
    let arch = std::fs::read_to_string(root.join("docs/architecture.md")).unwrap();
    assert!(
        !readme.contains("- [ ] **M") && !matrix.contains("- [ ] **M"),
        "[F17] Docs must not retain stale unchecked Phase 7 milestones"
    );
    assert!(
        arch.contains("workbench.view.testing") || readme.contains("workbench.view.testing"),
        "[F17] Docs must reference workbench.view.testing"
    );
    for module in ["buffer", "url", "util", "os", "crypto", "child_process"] {
        assert!(
            root.join(format!("ext-host-js/node-compat/{module}.js")).is_file(),
            "[F15] Missing node-compat/{module}.js"
        );
        assert!(
            matrix.contains(module) || arch.contains(module),
            "[F17] Docs missing node-compat module {module}"
        );
    }
}

/// Scenario 10: Multi-Language Web + Systems Workspace Editing
/// (HTML/CSS/TOML/YAML/Go + Find/Replace + Explorer + Settings: F6 + F9 + F13 + F16)
#[test]
fn scenario_10_multi_language_web_and_systems_workspace_editing() {
    let ws = TempWorkspace::new();
    ws.write_file(
        "web/index.html",
        "<!DOCTYPE html>\n<html><body class=\"btn-old\">Hello</body></html>\n",
    );
    ws.write_file("web/styles.css", ".btn-old { color: #ffffff; }\n");
    ws.write_file("server.go", "package main\nfunc serve() {}\n");
    ws.write_file("Cargo.toml", "[package]\nname = \"web-sys\"\n");
    ws.write_file("deploy.yaml", "env: production\n");

    let reg = GrammarRegistry::new();
    for (rel, expected) in [
        ("web/index.html", "html"),
        ("web/styles.css", "css"),
        ("server.go", "go"),
        ("Cargo.toml", "toml"),
        ("deploy.yaml", "yaml"),
    ] {
        assert_eq!(
            reg.get_for_path(&ws.path().join(rel)).language_id(),
            expected,
            "[F16] Wrong grammar for {rel}"
        );
    }

    let mut win = E2eWindow::with_temp_workspace(&ws);
    // F6: Apply workspace wordWrap configuration
    win.window
        .config
        .update("editor.wordWrap", json!("on"), ConfigurationTarget::Workspace)
        .unwrap();
    win.window.sync_configuration();

    // F13: Verify Explorer tracks workspace files
    let rows = win.window.view.explorer.flattened_rows();
    assert!(rows.iter().any(|r| r.name == "Cargo.toml"));
    assert!(rows.iter().any(|r| r.name == "server.go"));

    // F9: Find & Replace btn-old -> btn-primary in index.html & styles.css and save both
    for rel in ["web/index.html", "web/styles.css"] {
        let uri = win.open_file_from_disk(rel);
        let decoded = file_uri_to_path(&uri);
        assert!(paths_equivalent(&decoded, &ws.path().join(rel)));

        win.dispatch_ctrl_char('h');
        win.type_text("btn-old");
        win.dispatch_key(KeyCode::Tab, Modifiers::none(), None);
        win.type_text("btn-primary");
        let clicked = win.click_a11y_node("Replace All")
            || win.click_scene_text("Replace All")
            || win.click_scene_text("All");
        assert!(clicked, "[F9] Replace All must work on {rel}");
        win.dispatch_key(KeyCode::Escape, Modifiers::none(), None);
        win.dispatch_ctrl_char('s');

        let updated_disk = ws.read_file(rel);
        assert!(
            updated_disk.contains("btn-primary") && !updated_disk.contains("btn-old"),
            "[F9+F6] Disk file {rel} must reflect saved Find/Replace edit"
        );
    }

    let a11y = win.accessibility_tree();
    assert!(
        !a11y.find_by_role(AccessibleRole::TextEditor).is_empty(),
        "Active TextEditor must remain accessible"
    );
}
