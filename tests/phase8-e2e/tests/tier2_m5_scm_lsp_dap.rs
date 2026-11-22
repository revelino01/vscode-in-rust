use phase8_e2e::{
    init_git_repo, write_mock_dap_adapter, write_mock_lsp_server, TempWorkspace,
};
use rustcode_editor::Position;
use rustcode_ext_host::LanguageFeatureBridge;
use rustcode_platform::configuration::ConfigurationTarget;
use rustcode_platform::fs::path_to_file_uri;
use rustcode_platform::render::{InputEvent, KeyCode, KeyEvent, MockRenderSurface, Rect};
use rustcode_protocols::dap::{
    load_launch_json, parse_launch_json, DapClient, LaunchVariableContext,
};
use rustcode_protocols::git::{GitBackend, GitCliBackend, GitStatusKind};
use rustcode_protocols::lsp::{ClientCapabilities, InitializeParams, LspClientManager, LspServerConfig};
use rustcode_workbench::debug::{DebugToolbarAction, DebugToolbarView, DebugViewAction};
use rustcode_workbench::scm::ScmAction;
use rustcode_workbench::statusbar::STATUS_GIT_BRANCH_ID;
use rustcode_workbench::workbench_view::WorkbenchView;
use rustcode_workbench::WorkbenchWindow;
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

/// [M5-01, M5-02, M5-07, AUD-02] Real temp git repo lifecycle via GitCliBackend & Status Bar branch readout.
#[test]
fn t2_m5_git_cli_backend_real_repo_and_status_bar() {
    // AUD-02 & M5-07: Default WorkbenchView has NO sample SCM/Debug data and hidden DebugToolbarView
    let default_view = WorkbenchView::new();
    assert!(default_view.scm_view.repo.changes.is_empty());
    assert!(default_view.scm_view.repo.staged.is_empty());
    assert!(default_view.scm_view.repo.untracked.is_empty());
    assert!(default_view.debug_sidebar.session.call_stack.is_empty());
    assert!(!default_view.debug_toolbar.is_visible);
    assert!(!DebugToolbarView::new().is_visible);

    let ws = TempWorkspace::new();
    init_git_repo(ws.path());

    // Initial commit
    ws.write_file("README.md", "# Phase 8\n");
    ws.write_file("src/lib.rs", "pub fn v1() {}\n");
    let add_out = Command::new("git")
        .args(["add", "-A"])
        .current_dir(ws.path())
        .output()
        .unwrap();
    assert!(add_out.status.success());
    let commit_out = Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(ws.path())
        .output()
        .unwrap();
    assert!(commit_out.status.success());

    let mut window = WorkbenchWindow::default();
    window.open_workspace(ws.path());

    assert_eq!(window.view.scm_view.repo.current_branch.name, "main");
    assert!(window.view.scm_view.repo.changes.is_empty());
    assert!(window
        .view
        .status_bar
        .model
        .item(STATUS_GIT_BRANCH_ID)
        .is_some_and(|it| it.text.contains("main")));

    // Modify tracked file + add untracked file with spaces
    ws.write_file("src/lib.rs", "pub fn v2() {}\n");
    ws.write_file("notes with spaces.txt", "hello git\n");
    window.view.refresh_scm().unwrap();

    assert_eq!(window.view.scm_view.repo.changes.len(), 1);
    assert_eq!(window.view.scm_view.repo.changes[0].status, GitStatusKind::Modified);
    assert_eq!(window.view.scm_view.repo.untracked.len(), 1);

    // Stage -> Unstage -> Diff -> Show HEAD -> StageAll -> Commit
    let backend = GitCliBackend::new(ws.path().to_path_buf());
    backend.stage(PathBuf::from("src/lib.rs").as_path()).unwrap();
    assert_eq!(backend.status().unwrap().staged.len(), 1);
    backend.unstage(PathBuf::from("src/lib.rs").as_path()).unwrap();
    assert!(backend.status().unwrap().staged.is_empty());

    let diff = backend.diff(PathBuf::from("src/lib.rs").as_path(), false).unwrap();
    assert!(diff.contains("-pub fn v1()"));
    assert!(diff.contains("+pub fn v2()"));
    assert_eq!(
        backend.show_head(PathBuf::from("src/lib.rs").as_path()).unwrap().as_deref(),
        Some("pub fn v1() {}\n")
    );

    window.handle_scm_action(&ScmAction::StageAll).unwrap();
    assert_eq!(window.view.scm_view.repo.staged.len(), 2);
    window
        .handle_scm_action(&ScmAction::Commit {
            message: "Phase 8 SCM commit".to_string(),
        })
        .unwrap();
    assert!(window.view.scm_view.repo.staged.is_empty());
    assert!(window.view.scm_view.repo.changes.is_empty());
    assert!(window.view.scm_view.repo.untracked.is_empty());
    assert!(backend
        .log(5)
        .unwrap()
        .iter()
        .any(|c| c.message.contains("Phase 8 SCM commit")));

    // Non-git temp dir handled cleanly
    let non_git = TempWorkspace::new();
    assert!(!GitCliBackend::new(non_git.path().to_path_buf()).is_git_repo());
}

/// [M5-03..M5-05] Default LSP configs incl. `gopls`, missing-binary safety, and live mock stdio LSP server.
#[tokio::test]
async fn t2_m5_live_mock_stdio_lsp_server_and_missing_binary_safety() {
    let ws = TempWorkspace::new();
    let main_rs = ws.write_file("src/main.rs", "fn main() { ERROR_TOKEN(); }\n");
    let lib_rs = ws.write_file("src/lib.rs", "pub fn zero() {}\npub fn target_symbol() {}\n");
    let lib_uri = path_to_file_uri(&lib_rs);
    let main_uri = path_to_file_uri(&main_rs);

    // M5-03 & M5-04: Default server configs include `gopls`; missing binary fails non-fatally
    let mgr_check = LspClientManager::new();
    for (lang, bin) in [
        ("rust", "rust-analyzer"),
        ("typescript", "typescript-language-server"),
        ("javascript", "typescript-language-server"),
        ("python", "pyright-langserver"),
        ("json", "vscode-json-language-server"),
        ("go", "gopls"),
    ] {
        assert_eq!(
            mgr_check.get_config(lang).unwrap().command,
            bin,
            "unexpected LSP binary for {lang}"
        );
    }
    let missing_mgr = LspClientManager::new();
    missing_mgr.register_config(LspServerConfig::new(
        "nonexistent-lang",
        "definitely_missing_lsp_binary_xyz_12345",
        vec![],
        vec![],
    ));
    assert!(matches!(
        missing_mgr
            .ensure_server_for_language("nonexistent-lang", ws.path())
            .await,
        Ok(None)
    ));
    assert!(missing_mgr.is_language_failed("nonexistent-lang"));

    // Spawn real Node.js stdio mock LSP server
    let mock_lsp = write_mock_lsp_server(
        ws.path(),
        "mock_lsp.js",
        &lib_uri,
        "fn main() {\n    target_symbol();\n}\n",
    );

    let mut window = WorkbenchWindow::default()
        .with_language_bridge(Arc::new(LanguageFeatureBridge::with_defaults()));
    window.open_workspace(ws.path());
    window
        .config
        .update("editor.formatOnSave", json!(true), ConfigurationTarget::Workspace)
        .unwrap();

    let lsp_mgr = Arc::clone(&window.language_bridge.as_ref().unwrap().lsp_manager);
    let client = Arc::new(
        rustcode_protocols::lsp::LspClient::spawn_process(
            "node",
            &[mock_lsp.to_str().unwrap()],
            Some(ws.path().to_path_buf()),
        )
        .expect("spawn mock stdio LSP server"),
    );
    client
        .initialize(InitializeParams {
            process_id: None,
            root_uri: Some(path_to_file_uri(ws.path())),
            workspace_folders: None,
            capabilities: ClientCapabilities::default(),
        })
        .await
        .expect("LSP initialize handshake");
    lsp_mgr.attach_client("rust", client);

    window.open_path(&main_rs, true);

    // Wait for publishDiagnostics from mock stdio LSP
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        window.poll_runtime_events();
        if window.view.problems_panel.total_errors() > 0 {
            break;
        }
    }
    assert_eq!(window.view.problems_panel.total_errors(), 1);

    // Hover + Completion
    assert!(window.request_hover_at(Position::new(0, 14)).await);
    assert!(window.request_completions_at(Position::new(0, 14), Some('.')).await >= 2);
    let mut surface = MockRenderSurface::new(1200.0, 800.0);
    let scene = window.render(&mut surface);
    assert!(scene.contains_text("target_symbol"));

    // GoToDefinition via F12 keybinding & cross-file jump to lib.rs
    let bounds = Rect::from_xywh(0.0, 0.0, 1200.0, 800.0);
    window.dispatch_event(
        &InputEvent::KeyDown(KeyEvent::new(
            KeyCode::F12,
            rustcode_platform::keybindings::Modifiers::none(),
        )),
        bounds,
    );
    assert!(window.go_to_definition_at(Position::new(0, 14)).await);
    assert_eq!(
        window
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .unwrap()
            .uri,
        lib_uri
    );
    assert_eq!(
        window.view.editor_area.active_editor().unwrap().primary_position(),
        Position::new(1, 7)
    );

    // Return to main.rs, save with formatOnSave = true -> formats & clears ERROR_TOKEN diagnostics!
    window.open_path(&main_rs, true);
    assert!(window.save_active_file_async().await);
    assert_eq!(
        std::fs::read_to_string(&main_rs).unwrap(),
        "fn main() {\n    target_symbol();\n}\n"
    );
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        window.poll_runtime_events();
        if window.view.problems_panel.total_errors() == 0 {
            break;
        }
    }
    assert_eq!(window.view.problems_panel.total_errors(), 0);
    let _ = main_uri;
}

/// [M5-06..M5-08] `.vscode/launch.json` JSONC parser, variable expansion, and live mock stdio DAP session.
#[tokio::test]
async fn t2_m5_launch_json_and_live_mock_stdio_dap_session() {
    let ws = TempWorkspace::new();
    let main_rs = ws.write_file("src/main.rs", "fn main() {\n    let counter = 42;\n}\n");
    ws.write_file(
        ".vscode/launch.json",
        r#"{
            // JSONC comment + trailing comma
            "version": "0.2.0",
            "configurations": [
                {
                    "name": "Debug Mock App",
                    "type": "lldb",
                    "request": "launch",
                    "program": "${workspaceFolder}/target/debug/app",
                    "args": ["--file", "${file}"],
                },
            ],
        }"#,
    );

    // M5-06: launch.json parser & variable expansion + malformed error
    let parsed = load_launch_json(ws.path()).unwrap();
    assert_eq!(parsed.configurations.len(), 1);
    let var_ctx = LaunchVariableContext::new(ws.path()).with_active_file(&main_rs);
    let expanded = parsed.configurations[0].resolve(&var_ctx);
    assert!(expanded.program.as_deref().unwrap().ends_with("/target/debug/app"));
    assert_eq!(expanded.args[1], main_rs.to_string_lossy());
    assert!(parse_launch_json("{ malformed json").is_err());

    // Spawn real Node.js stdio mock DAP adapter
    let mock_dap = write_mock_dap_adapter(ws.path(), "mock_dap.js", &main_rs);
    let dap = Arc::new(
        DapClient::spawn_process(
            "node",
            &[mock_dap.to_str().unwrap()],
            Some(ws.path().to_path_buf()),
        )
        .expect("spawn mock stdio DAP adapter"),
    );

    let mut window = WorkbenchWindow::default().with_dap_client(dap);
    window.open_workspace(ws.path());
    assert!(!window.view.debug_toolbar.is_visible);
    assert_eq!(window.view.debug_sidebar.launch_configurations.len(), 1);

    window.start_debug_session(Some(0)).await.unwrap();
    assert!(window.view.debug_toolbar.is_visible);
    assert!(window.view.debug_sidebar.session.is_active);

    // Wait for stopped event -> threads, stackTrace, scopes, variables
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        window.poll_runtime_events();
        if window.view.debug_sidebar.session.is_paused
            && !window.view.debug_sidebar.session.call_stack.is_empty()
        {
            break;
        }
    }
    assert!(window.view.debug_sidebar.session.is_paused);
    assert_eq!(window.view.debug_sidebar.session.call_stack[0].name, "main");
    assert_eq!(
        window.view.debug_sidebar.session.variables.get(&200).unwrap()[0].value,
        "42"
    );

    // Evaluate expression in Debug Console
    window
        .handle_debug_view_action_async(&DebugViewAction::EvaluateExpression {
            expression: "counter + 1".to_string(),
        })
        .await
        .unwrap();
    window.poll_runtime_events();
    assert!(window
        .view
        .debug_sidebar
        .session
        .console_output
        .iter()
        .any(|e| e.text.contains("43")));

    // StepOver updates variable state on next stopped event
    window
        .handle_debug_toolbar_action_async(&DebugToolbarAction::StepOver)
        .await
        .unwrap();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        window.poll_runtime_events();
        if window
            .view
            .debug_sidebar
            .session
            .variables
            .get(&200)
            .is_some_and(|v| v[0].value == "43")
        {
            break;
        }
    }
    assert_eq!(
        window.view.debug_sidebar.session.variables.get(&200).unwrap()[0].value,
        "43"
    );

    // Stop terminates session and hides DebugToolbarView
    window
        .handle_debug_toolbar_action_async(&DebugToolbarAction::Stop)
        .await
        .unwrap();
    assert!(!window.view.debug_sidebar.session.is_active);
    assert!(!window.view.debug_toolbar.is_visible);
}
