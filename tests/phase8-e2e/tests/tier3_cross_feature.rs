use parking_lot::RwLock;
use phase8_e2e::{
    init_git_repo, repo_root, write_mock_lsp_server, write_unpacked_extension, TempWorkspace,
};
use rustcode_app::cli::{parse_cli_args, CliAction};
use rustcode_app::runtime::{
    bootstrap_app, restore_workbench_state, save_workbench_state, RuntimeLoop, RuntimeLoopConfig,
};
use rustcode_app::RustCodeContext;
use rustcode_editor::Position;
use rustcode_platform::configuration::ConfigurationTarget;
use rustcode_platform::fs::path_to_file_uri;
use rustcode_platform::storage::StorageBackend;
use rustcode_protocols::git::{GitBackend, GitCliBackend};
use rustcode_protocols::lsp::{ClientCapabilities, InitializeParams, LspClient};
use rustcode_workbench::scm::ScmAction;
use rustcode_workbench::statusbar::STATUS_GIT_BRANCH_ID;
use serde_json::json;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "desktop")]
use rustcode_platform::storage::SqliteStorageService;

/// [E2E-02] Full developer workflow:
/// CLI `--user-data-dir` + `--goto` -> `bootstrap_app` -> syntax highlighting ->
/// live stdio LSP diagnostics + `formatOnSave` -> `GitCliBackend` stage/commit ->
/// SQLite state persistence & fresh restart restore.
#[tokio::test]
async fn t3_01_full_developer_workflow_cli_goto_syntax_lsp_git_sqlite_restart() {
    let ws = TempWorkspace::new();
    let user_data = ws.mkdir_all("user-data");
    let proj = ws.mkdir_all("project");

    init_git_repo(&proj);
    let main_rs = ws.write_file("project/src/main.rs", "fn main() {\n    ERROR_TOKEN();\n}\n");
    let lib_rs = ws.write_file("project/src/lib.rs", "pub fn clean() {}\npub fn helper() {}\n");
    ws.write_file(
        "project/.vscode/settings.json",
        r#"{
            "editor.formatOnSave": true,
            "editor.tabSize": 2
        }"#,
    );

    assert!(Command::new("git")
        .args(["add", "-A"])
        .current_dir(&proj)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args(["commit", "-m", "baseline"])
        .current_dir(&proj)
        .output()
        .unwrap()
        .status
        .success());

    // 1. CLI --user-data-dir + --goto opens project workspace & positions cursor
    let goto_spec = format!("{}:2:5", main_rs.display());
    let opts = match parse_cli_args([
        "--headless",
        "--user-data-dir",
        user_data.to_str().unwrap(),
        "--goto",
        &goto_spec,
        proj.to_str().unwrap(),
    ])
    .unwrap()
    {
        CliAction::Run(o) => o,
        other => panic!("unexpected cli action: {other:?}"),
    };

    let mut app = bootstrap_app(&opts, false).unwrap();
    assert_eq!(
        app.window
            .view
            .editor_area
            .active_editor()
            .unwrap()
            .primary_position(),
        Position::new(1, 4)
    );
    assert_eq!(app.window.view.scm_view.repo.current_branch.name, "main");
    assert!(app
        .window
        .view
        .status_bar
        .model
        .item(STATUS_GIT_BRANCH_ID)
        .is_some_and(|it| it.text.contains("main")));

    // 2. Attach live stdio mock LSP server
    let lib_uri = path_to_file_uri(&lib_rs);
    let mock_lsp = write_mock_lsp_server(
        ws.path(),
        "workflow_lsp.js",
        &lib_uri,
        "fn main() {\n    helper();\n}\n",
    );
    let client = Arc::new(
        LspClient::spawn_process("node", &[mock_lsp.to_str().unwrap()], Some(proj.clone()))
            .expect("spawn workflow mock LSP"),
    );
    client
        .initialize(InitializeParams {
            process_id: None,
            root_uri: Some(path_to_file_uri(&proj)),
            workspace_folders: None,
            capabilities: ClientCapabilities::default(),
        })
        .await
        .unwrap();
    let lsp_mgr = Arc::clone(&app.window.language_bridge.as_ref().unwrap().lsp_manager);
    lsp_mgr.attach_client("rust", client);

    // Send didOpen for main.rs against the newly attached LSP client
    // (`bootstrap_app` opened main.rs before the mock LSP client was attached).
    let main_uri = app
        .window
        .view
        .editor_area
        .find_open_tab_uri_for_path(&main_rs)
        .unwrap_or_else(|| path_to_file_uri(&main_rs));
    let main_text = app
        .window
        .view
        .editor_area
        .any_editor_for_uri(&main_uri)
        .unwrap()
        .text();
    app.window
        .language_bridge
        .as_ref()
        .unwrap()
        .did_open_document(&main_uri, "rust", 1, &main_text)
        .await;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        app.window.poll_runtime_events();
        if app.window.view.problems_panel.total_errors() > 0 {
            break;
        }
    }
    assert_eq!(app.window.view.problems_panel.total_errors(), 1);

    // 3. Save with formatOnSave = true -> formats document, clears diagnostic, updates git working tree
    assert!(app.window.save_active_file_async().await);
    assert_eq!(
        std::fs::read_to_string(&main_rs).unwrap(),
        "fn main() {\n    helper();\n}\n"
    );
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        app.window.poll_runtime_events();
        if app.window.view.problems_panel.total_errors() == 0 {
            break;
        }
    }
    assert_eq!(app.window.view.problems_panel.total_errors(), 0);

    // 4. GitCliBackend stage & commit formatted changes
    app.window.view.refresh_scm().unwrap();
    assert_eq!(app.window.view.scm_view.repo.changes.len(), 1);
    app.window.handle_scm_action(&ScmAction::StageAll).unwrap();
    app.window
        .handle_scm_action(&ScmAction::Commit {
            message: "format main.rs via LSP".to_string(),
        })
        .unwrap();
    let git_backend = GitCliBackend::new(proj.clone());
    assert!(git_backend
        .log(5)
        .unwrap()
        .iter()
        .any(|c| c.message.contains("format main.rs via LSP")));

    // 5. Persist layout/tabs/theme/cursor into SQLite & restore in a fresh window
    app.window.open_path(&lib_rs, false);
    if let Some(ed) = app.window.view.editor_area.active_editor_mut() {
        ed.set_primary_position(Position::new(1, 2));
    }
    app.window.view.layout.mutate(|l| {
        l.sidebar_visible = false;
        l.panel_visible = true;
    });
    save_workbench_state(&app.window).unwrap();
    app.runtime_loop.shutdown_async(&mut app.window).await;

    #[cfg(feature = "desktop")]
    {
        let db_file = app.app_paths.state_db_file.clone();
        assert!(db_file.is_file(), "SQLite DB file must exist on disk");
        let reopened = Arc::new(SqliteStorageService::open(&db_file).unwrap());
        let mut ctx2 = RustCodeContext::new();
        let _ = ctx2.theme.register_builtin_themes(None);
        let mut win2 = ctx2
            .create_window()
            .with_storage_backend(reopened as Arc<dyn StorageBackend>);
        win2.open_workspace(&proj);
        assert!(restore_workbench_state(&mut win2, Some(&proj)).unwrap());
        assert!(!win2.view.layout.sidebar_visible);
        assert!(win2.view.layout.panel_visible);
        assert_eq!(
            win2.view.editor_area.model.current_group_ref().tabs.len(),
            2
        );
        assert_eq!(
            win2.view
                .editor_area
                .active_editor()
                .unwrap()
                .primary_position(),
            Position::new(1, 2)
        );
    }
}

/// [M2-18..M2-20, ADV-04] Live Tokio RuntimeLoop: FileSystemWatcher reload/conflict, AutoSaveService, and NodeExtHostProcess crash recovery.
#[tokio::test]
async fn t3_02_runtime_loop_fs_watcher_auto_save_and_ext_host_crash_recovery() {
    let ws = TempWorkspace::new();
    let proj = ws.mkdir_all("ws");
    let exts = ws.mkdir_all("exts");
    let clean_rs = ws.write_file("ws/clean.rs", "fn v1() {}\n");
    let dirty_rs = ws.write_file("ws/dirty.rs", "fn d1() {}\n");

    let ext_dir = write_unpacked_extension(
        &exts,
        "crash-recovery-ext",
        json!({
            "name": "crash-recovery-ext",
            "publisher": "Phase8",
            "version": "1.0.0",
            "main": "./extension.js",
            "activationEvents": ["onLanguage:rust"]
        }),
        &[(
            "extension.js",
            r#"const vscode = require('vscode');
exports.activate = (context) => {
  context.subscriptions.push(
    vscode.commands.registerCommand('phase8.crashPing', () => ({ alive: true }))
  );
  return { ok: true };
};
"#,
        )],
    );

    let mut ctx = RustCodeContext::new();
    ctx.config
        .update("files.autoSave", json!("afterDelay"), ConfigurationTarget::Workspace)
        .unwrap();
    ctx.config
        .update("files.autoSaveDelay", json!(30), ConfigurationTarget::Workspace)
        .unwrap();
    ctx.ext_loader.load_from_dir(&ext_dir).unwrap();
    let shared_loader = Arc::new(RwLock::new(ctx.ext_loader.clone()));
    ctx.language_bridge
        .set_extension_loader(Arc::clone(&shared_loader));

    let mut win = ctx.create_window();
    win.open_workspace(&proj);
    win.open_path(&clean_rs, false);
    win.open_path(&dirty_rs, false);

    let mut rt = RuntimeLoop::new_with_shared_loader(
        RuntimeLoopConfig {
            disable_extensions: false,
            ext_host_restart_backoff: Duration::from_millis(50),
            ..Default::default()
        },
        Arc::clone(&shared_loader),
    )
    .unwrap();
    rt.tick(&mut win);

    // 1. Watcher reloads clean file & AutoSave flushes dirty buffer after delay
    std::fs::write(&clean_rs, "fn v2_disk() {}\n").unwrap();
    if let Some(ed) = win.view.editor_area.active_editor_mut() {
        ed.insert_text("// autosaved\n");
    }

    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(30)).await;
        rt.tick(&mut win);
        let clean_uri = win
            .view
            .editor_area
            .find_open_tab_uri_for_path(&clean_rs)
            .unwrap();
        let reloaded = win
            .view
            .editor_area
            .any_editor_for_uri(&clean_uri)
            .unwrap()
            .text()
            == "fn v2_disk() {}\n";
        let autosaved = std::fs::read_to_string(&dirty_rs)
            .unwrap()
            .contains("// autosaved");
        if reloaded && autosaved && shared_loader.read().is_activated("Phase8.crash-recovery-ext")
        {
            break;
        }
    }

    let clean_uri = win
        .view
        .editor_area
        .find_open_tab_uri_for_path(&clean_rs)
        .unwrap();
    assert_eq!(
        win.view
            .editor_area
            .any_editor_for_uri(&clean_uri)
            .unwrap()
            .text(),
        "fn v2_disk() {}\n"
    );
    assert!(std::fs::read_to_string(&dirty_rs)
        .unwrap()
        .contains("// autosaved"));

    // 2. Live NodeExtHostProcess activation + crash detection & automatic restart
    assert!(rt.is_ext_host_running());
    assert!(shared_loader.read().is_activated("Phase8.crash-recovery-ext"));

    let pong = win
        .language_bridge
        .as_ref()
        .unwrap()
        .request_execute_command("phase8.crashPing", vec![])
        .await
        .unwrap();
    assert_eq!(pong["alive"], json!(true));

    rt.shutdown_async(&mut win).await;
}

/// [M6-04, M6-05, INV-04] CI workflow, packaging scripts, and documentation surface verification.
#[test]
fn t3_03_ci_packaging_and_phase8_docs_invariants() {
    let root = repo_root();

    let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    for required in [
        "windows-latest",
        "ubuntu-latest",
        "tests/phase7-e2e/runner.js",
        "tests/phase8-e2e/runner.js",
        "clippy",
        "-D warnings",
        "scripts/package.ps1",
        "scripts/package.sh",
    ] {
        assert!(ci.contains(required), "ci.yml missing '{required}'");
    }

    let pkg_ps1 = std::fs::read_to_string(root.join("scripts/package.ps1")).unwrap();
    let pkg_sh = std::fs::read_to_string(root.join("scripts/package.sh")).unwrap();
    assert!(pkg_ps1.contains("rustcode.exe") && pkg_ps1.contains("ext-host-js"));
    assert!(pkg_sh.contains("rustcode.desktop") && pkg_sh.contains("ext-host-js"));

    // F17 doc invariants preserved
    let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
    let matrix = std::fs::read_to_string(root.join("docs/extension-compat-matrix.md")).unwrap();
    let arch = std::fs::read_to_string(root.join("docs/architecture.md")).unwrap();
    let plan = std::fs::read_to_string(root.join("rustcode-implementation-plan.md")).unwrap();
    assert!(!readme.contains("- [ ] **M"));
    assert!(!matrix.contains("- [ ] **M"));
    assert!(readme.contains("package.ps1") && readme.contains("package.sh"));
    let combined = format!("{readme}\n{arch}\n{matrix}\n{plan}");
    for cmd in [
        "actions.find",
        "editor.action.startFindReplaceAction",
        "workbench.action.gotoLine",
        "workbench.action.gotoSymbol",
        "workbench.view.testing",
    ] {
        assert!(combined.contains(cmd), "Docs missing '{cmd}'");
    }
}
