use parking_lot::RwLock;
use phase8_e2e::{
    init_git_repo, repo_root, run_rustcode_cli_args, write_unpacked_extension, TempWorkspace,
};
use rustcode_app::runtime::load_extension_themes;
use rustcode_editor::{EditorView, GrammarRegistry, Position, SyntaxHighlighter, SyntaxScope, TextMateGrammar};
use rustcode_ext_host::{
    ExtensionLoader, LanguageFeatureBridge, NodeExtHostConfig, NodeExtHostProcess, RpcWireFormat,
};
use rustcode_platform::configuration::ConfigurationTarget;
use rustcode_platform::fs::path_to_file_uri;
use rustcode_platform::render::{MockRenderSurface, Rgba, WorkbenchThemeStyles};
use rustcode_platform::theme::ThemeService;
use rustcode_protocols::codec::{read_framed_message, write_framed_message};
use rustcode_protocols::git::{GitBackend, GitCliBackend};
use rustcode_protocols::lsp::{
    CodeActionContext, LspClient, LspPosition, LspRange, LspTextEdit, WorkspaceEdit,
};
use rustcode_workbench::scm::ScmAction;
use rustcode_workbench::WorkbenchWindow;
use serde_json::json;
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

/// [M6-06 #1] Real-world extension harness: One Dark Pro (Theme JSON + TextMate `.tmLanguage.json`).
#[test]
fn t4_01_one_dark_pro_theme_and_textmate_grammar() {
    let ws = TempWorkspace::new();
    let exts = ws.mkdir_all("extensions");

    let ext_dir = write_unpacked_extension(
        &exts,
        "zhuangtongfa.material-theme",
        json!({
            "name": "OneDark-Pro",
            "publisher": "zhuangtongfa",
            "version": "3.17.0",
            "activationEvents": ["*"],
            "contributes": {
                "themes": [{
                    "label": "One Dark Pro",
                    "uiTheme": "vs-dark",
                    "path": "./themes/OneDark-Pro.json"
                }],
                "languages": [{
                    "id": "onedarkdemo",
                    "extensions": [".odp"]
                }],
                "grammars": [{
                    "language": "onedarkdemo",
                    "scopeName": "source.odp",
                    "path": "./syntaxes/odp.tmLanguage.json"
                }]
            }
        }),
        &[
            (
                "themes/OneDark-Pro.json",
                r##"{
                    "name": "One Dark Pro",
                    "type": "dark",
                    "colors": {
                        "editor.background": "#282c34",
                        "editor.foreground": "#abb2bf"
                    },
                    "tokenColors": [
                        { "scope": "keyword.control", "settings": { "foreground": "#c678dd" } },
                        { "scope": "string.quoted", "settings": { "foreground": "#98c379" } },
                        { "scope": "comment.line", "settings": { "foreground": "#5c6370", "fontStyle": "italic" } }
                    ]
                }"##,
            ),
            (
                "syntaxes/odp.tmLanguage.json",
                r#"{
                    "name": "OneDarkDemo",
                    "scopeName": "source.odp",
                    "patterns": [
                        { "match": "//.*$", "name": "comment.line.double-slash.odp" },
                        { "match": "\\b(return|if|else)\\b", "name": "keyword.control.odp" },
                        { "begin": "\"", "end": "\"", "name": "string.quoted.double.odp" }
                    ]
                }"#,
            ),
        ],
    );

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).unwrap();
    let mut themes = ThemeService::new();
    load_extension_themes(&mut themes, &loader);
    themes.set_theme_by_name("One Dark Pro").unwrap();

    let active_theme = themes.current().clone();
    assert_eq!(
        WorkbenchThemeStyles::from_theme(&active_theme).editor_background,
        Rgba::from_hex("#282c34").unwrap()
    );

    let tm_path = ext_dir.join("syntaxes/odp.tmLanguage.json");
    let tm = Arc::new(TextMateGrammar::from_file(&tm_path, "onedarkdemo", &["odp"]).unwrap());
    let mut reg = GrammarRegistry::new();
    reg.register_textmate(tm);

    let mut hl = SyntaxHighlighter::for_language("onedarkdemo", active_theme.clone(), &reg);
    let spans = hl.get_line_tokens(0, "return \"ok\" // note");
    assert!(spans.iter().any(|s| s.scope == SyntaxScope::KeywordControl));
    assert!(spans.iter().any(|s| s.scope == SyntaxScope::StringQuoted));
    assert!(spans.iter().any(|s| s.scope == SyntaxScope::CommentLine));

    let mut ed = EditorView::from_str("return \"ok\" // note\n");
    ed.set_language("onedarkdemo", active_theme, &reg);
    let mut win = WorkbenchWindow::default();
    win.theme = themes;
    win.open_file("file:///demo.odp", "demo.odp", "return \"ok\" // note\n", false);
    let mut surface = MockRenderSurface::new(1024.0, 768.0);
    let scene = win.render(&mut surface);
    assert!(scene.contains_text("return"));
}

/// [M6-06 #2] Real-world extension harness: Prettier (DocumentFormattingEditProvider via live NodeExtHostProcess + formatOnSave).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t4_02_prettier_live_node_ext_host_formatting() {
    let ws = TempWorkspace::new();
    let exts = ws.mkdir_all("extensions");
    let ts_file = ws.write_file("app.ts", "const   x=1+2");

    let ext_dir = write_unpacked_extension(
        &exts,
        "esbenp.prettier-vscode",
        json!({
            "name": "prettier-vscode",
            "publisher": "esbenp",
            "version": "11.0.0",
            "main": "./extension.js",
            "activationEvents": ["onLanguage:typescript"],
            "contributes": {
                "commands": [{
                    "command": "prettier.forceFormatDocument",
                    "title": "Prettier: Format Document"
                }]
            }
        }),
        &[(
            "extension.js",
            r#"const vscode = require('vscode');
exports.activate = (context) => {
  const disp = vscode.languages.registerDocumentFormattingEditProvider('typescript', {
    async provideDocumentFormattingEdits(documentOrUri) {
      const uriStr = typeof documentOrUri === 'string'
        ? documentOrUri
        : (documentOrUri.uri ? documentOrUri.uri.toString() : String(documentOrUri));
      const doc = await vscode.workspace.openTextDocument(uriStr);
      const text = doc.getText();
      const formatted = text.replace(/\s+/g, ' ').replace('=', ' = ').replace('+', ' + ').trim() + ';\n';
      const lastLine = Math.max(0, doc.lineCount - 1);
      const lastChar = doc.lineAt(lastLine).text.length;
      return [{
        range: { start: { line: 0, character: 0 }, end: { line: lastLine, character: lastChar } },
        newText: formatted
      }];
    }
  });
  context.subscriptions.push(disp);
};
"#,
        )],
    );

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).unwrap();
    let shared_loader = Arc::new(RwLock::new(loader));
    let bridge = Arc::new(
        LanguageFeatureBridge::with_defaults()
            .with_shared_extension_loader(Arc::clone(&shared_loader))
            .with_fs_root(ws.path()),
    );

    let mut proc = NodeExtHostProcess::spawn(
        Arc::clone(&bridge),
        NodeExtHostConfig {
            cwd: Some(repo_root()),
            wire_format: RpcWireFormat::ContentLengthJson,
            ..Default::default()
        },
    )
    .expect("spawn NodeExtHostProcess");

    bridge
        .activate_extension("esbenp.prettier-vscode")
        .await
        .unwrap();

    let mut win = WorkbenchWindow::default().with_language_bridge(Arc::clone(&bridge));
    win.open_workspace(ws.path());
    win.config
        .update("editor.formatOnSave", json!(true), ConfigurationTarget::Workspace)
        .unwrap();
    win.open_path(&ts_file, true);

    for _ in 0..40 {
        if !bridge
            .provider_registry
            .get_providers_for_language(
                rustcode_ext_host::ProviderType::DocumentFormatting,
                "typescript",
            )
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert!(win.save_active_file_async().await);
    let saved = std::fs::read_to_string(&ts_file).unwrap();
    assert_eq!(saved, "const x = 1 + 2;\n");
    assert_eq!(
        win.view.editor_area.active_editor().unwrap().text(),
        "const x = 1 + 2;\n"
    );

    let _ = proc.shutdown().await;
}

/// [M6-06 #3] Real-world extension harness: ESLint (DiagnosticCollection + CodeActionProvider quick-fix via live NodeExtHostProcess).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t4_03_eslint_live_node_ext_host_diagnostics_and_code_actions() {
    let ws = TempWorkspace::new();
    let exts = ws.mkdir_all("extensions");
    let js_file = ws.write_file("index.js", "var count = 1;\n");
    let js_uri = path_to_file_uri(&js_file);

    let ext_dir = write_unpacked_extension(
        &exts,
        "dbaeumer.vscode-eslint",
        json!({
            "name": "vscode-eslint",
            "publisher": "dbaeumer",
            "version": "3.0.10",
            "main": "./extension.js",
            "activationEvents": ["onLanguage:javascript"]
        }),
        &[(
            "extension.js",
            r#"const vscode = require('vscode');
exports.activate = (context) => {
  const collection = vscode.languages.createDiagnosticCollection('eslint');
  function lint(uriStr, text) {
    const idx = text.indexOf('var ');
    if (idx !== -1) {
      collection.set(vscode.Uri.parse(uriStr), [{
        range: { start: { line: 0, character: idx }, end: { line: 0, character: idx + 3 } },
        severity: vscode.DiagnosticSeverity.Error,
        source: 'eslint',
        code: 'no-var',
        message: 'Unexpected var, use let or const instead.'
      }]);
    } else {
      collection.delete(vscode.Uri.parse(uriStr));
    }
  }
  context.subscriptions.push(
    collection,
    vscode.commands.registerCommand('eslint.lintUri', async (uriStr) => {
      const doc = await vscode.workspace.openTextDocument(uriStr);
      lint(uriStr, doc.getText());
      return true;
    }),
    vscode.languages.registerCodeActionsProvider('javascript', {
      provideCodeActions(uriStr) {
        return [{
          title: 'Replace var with let',
          kind: 'quickfix',
          edit: {
            changes: {
              [uriStr]: [{
                range: { start: { line: 0, character: 0 }, end: { line: 0, character: 3 } },
                newText: 'let'
              }]
            }
          }
        }];
      }
    })
  );
};
"#,
        )],
    );

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).unwrap();
    let shared_loader = Arc::new(RwLock::new(loader));
    let bridge = Arc::new(
        LanguageFeatureBridge::with_defaults()
            .with_shared_extension_loader(Arc::clone(&shared_loader))
            .with_fs_root(ws.path()),
    );

    let mut proc = NodeExtHostProcess::spawn(
        Arc::clone(&bridge),
        NodeExtHostConfig {
            cwd: Some(repo_root()),
            wire_format: RpcWireFormat::ContentLengthJson,
            ..Default::default()
        },
    )
    .expect("spawn NodeExtHostProcess");

    bridge
        .activate_extension("dbaeumer.vscode-eslint")
        .await
        .unwrap();

    let mut win = WorkbenchWindow::default().with_language_bridge(Arc::clone(&bridge));
    win.open_workspace(ws.path());
    win.open_path(&js_file, true);

    for _ in 0..40 {
        if bridge.command_registry().has_command("eslint.lintUri") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    bridge
        .request_execute_command("eslint.lintUri", vec![json!(js_uri)])
        .await
        .unwrap();
    win.sync_diagnostics();
    assert_eq!(win.view.problems_panel.total_errors(), 1);

    let actions = bridge
        .request_code_actions(
            &js_uri,
            "javascript",
            LspRange {
                start: LspPosition {
                    line: 0,
                    character: 0,
                },
                end: LspPosition {
                    line: 0,
                    character: 3,
                },
            },
            CodeActionContext {
                diagnostics: vec![],
                only: None,
                trigger_kind: None,
            },
        )
        .await;
    assert_eq!(actions.len(), 1);
    let edit: WorkspaceEdit = actions[0].edit.clone().expect("quickfix workspace edit");
    assert!(win.apply_workspace_edit(&edit) > 0);
    assert_eq!(
        win.view.editor_area.active_editor().unwrap().text(),
        "let count = 1;\n"
    );

    let _ = proc.shutdown().await;
}

/// [M6-06 #4] Real-world extension harness: rust-analyzer LSP 3.17 UTF-16 <-> UTF-8 alignment & full feature set.
#[tokio::test]
async fn t4_04_rust_analyzer_lsp_317_utf16_utf8_and_features() {
    let ws = TempWorkspace::new();
    // Emoji 🚀 is 4 UTF-8 bytes but 2 UTF-16 code units!
    let main_rs = ws.write_file("src/main.rs", "let s = \"🚀\"; let x = target_call();\n");
    let lib_rs = ws.write_file("src/lib.rs", "pub fn a() {}\npub fn target_call() {}\n");
    let main_uri = path_to_file_uri(&main_rs);
    let lib_uri = path_to_file_uri(&lib_rs);

    let (client_r, server_w) = tokio::io::duplex(8192);
    let (server_r, client_w) = tokio::io::duplex(8192);
    let lib_uri_srv = lib_uri.clone();
    let main_uri_srv = main_uri.clone();

    let server_task = tokio::spawn(async move {
        let mut r = server_r;
        let mut w = server_w;
        let mut buf = Vec::new();
        while let Ok(Some(bytes)) = read_framed_message(&mut r, &mut buf).await {
            let msg: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
            if let Some(id) = msg.get("id").cloned() {
                let result = match method {
                    "initialize" => json!({
                        "capabilities": {
                            "textDocumentSync": 1,
                            "hoverProvider": true,
                            "definitionProvider": true,
                            "completionProvider": { "triggerCharacters": ["."] },
                            "documentFormattingProvider": true,
                            "positionEncoding": "utf-16"
                        }
                    }),
                    "textDocument/hover" => json!({
                        "contents": { "kind": "markdown", "value": "fn target_call()" }
                    }),
                    "textDocument/definition" => json!({
                        "uri": lib_uri_srv,
                        "range": { "start": { "line": 1, "character": 7 }, "end": { "line": 1, "character": 18 } }
                    }),
                    "textDocument/completion" => json!([
                        { "label": "target_call", "kind": 3, "detail": "pub fn target_call()" }
                    ]),
                    "textDocument/formatting" => json!([{
                        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 37 } },
                        "newText": "let s = \"🚀\";\nlet x = target_call();"
                    }]),
                    _ => json!(null),
                };
                let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result });
                let _ = write_framed_message(&mut w, resp.to_string().as_bytes()).await;

                if method == "textDocument/hover" {
                    // Publish diagnostic at UTF-16 column 21..32 (`target_call` after surrogate pair 🚀)
                    let diag = json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/publishDiagnostics",
                        "params": {
                            "uri": main_uri_srv,
                            "diagnostics": [{
                                "range": { "start": { "line": 0, "character": 21 }, "end": { "line": 0, "character": 32 } },
                                "severity": 2,
                                "source": "rust-analyzer",
                                "message": "utf16 aligned warning"
                            }]
                        }
                    });
                    let _ = write_framed_message(&mut w, diag.to_string().as_bytes()).await;
                }
            }
        }
    });

    let client = Arc::new(LspClient::from_io(client_r, client_w));
    let mut win = WorkbenchWindow::default()
        .with_language_bridge(Arc::new(LanguageFeatureBridge::with_defaults()));
    win.language_bridge
        .as_ref()
        .unwrap()
        .lsp_manager
        .attach_client("rust", client);
    win.open_workspace(ws.path());
    win.open_path(&main_rs, true);

    assert!(win.request_hover_at(Position::new(0, 22)).await);
    assert!(win.request_completions_at(Position::new(0, 22), None).await > 0);
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(15)).await;
        win.poll_runtime_events();
        if win.view.problems_panel.total_warnings() > 0 {
            break;
        }
    }
    assert_eq!(win.view.problems_panel.total_warnings(), 1);

    assert!(win.format_active_document().await);
    assert!(win
        .view
        .editor_area
        .active_editor()
        .unwrap()
        .text()
        .contains("let s = \"🚀\";\nlet x = target_call();"));

    assert!(win.go_to_definition_at(Position::new(0, 22)).await);
    assert_eq!(
        win.view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .unwrap()
            .uri,
        lib_uri
    );
    let _ = LspTextEdit {
        range: LspRange {
            start: LspPosition {
                line: 0,
                character: 0,
            },
            end: LspPosition {
                line: 0,
                character: 0,
            },
        },
        new_text: String::new(),
    };
    let _ = HashMap::<String, String>::new();
    server_task.abort();
}

/// [M6-06 #5] Real-world extension harness: GitLens (vscode.scm + GitCliBackend + WebviewPanel postMessage).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t4_05_gitlens_scm_git_cli_and_webview_postmessage() {
    let ws = TempWorkspace::new();
    let proj = ws.mkdir_all("repo");
    let exts = ws.mkdir_all("extensions");

    init_git_repo(&proj);
    let app_rs = ws.write_file("repo/app.rs", "fn v1() {}\n");
    assert!(Command::new("git")
        .args(["add", "-A"])
        .current_dir(&proj)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args(["commit", "-m", "gitlens baseline"])
        .current_dir(&proj)
        .output()
        .unwrap()
        .status
        .success());
    ws.write_file("repo/app.rs", "fn v2() {}\n");

    let ext_dir = write_unpacked_extension(
        &exts,
        "eamodio.gitlens",
        json!({
            "name": "gitlens",
            "publisher": "eamodio",
            "version": "15.0.0",
            "main": "./extension.js",
            "activationEvents": ["*"]
        }),
        &[(
            "extension.js",
            r#"const vscode = require('vscode');
exports.activate = (context) => {
  const sc = vscode.scm.createSourceControl('gitlens', 'GitLens', vscode.Uri.file('.'));
  const grp = sc.createResourceGroup('changes', 'Modified');
  const receivedFromWebview = [];
  let panel = null;

  context.subscriptions.push(
    sc,
    grp,
    vscode.commands.registerCommand('gitlens.openGraph', (uriStr) => {
      grp.resourceStates = [{
        resourceUri: vscode.Uri.parse(uriStr),
        decorations: { tooltip: 'Modified in working tree', strikeThrough: false }
      }];
      panel = vscode.window.createWebviewPanel('gitlens.graph', 'Commit Graph', 1, { enableScripts: true });
      panel.webview.html = '<h1>GitLens Graph</h1>';
      panel.webview.onDidReceiveMessage((msg) => {
        receivedFromWebview.push(msg);
      });
      panel.webview.postMessage({ type: 'graphReady', branch: 'main' });
      return { panelOpened: true };
    }),
    vscode.commands.registerCommand('gitlens.getWebviewMessages', () => receivedFromWebview)
  );
};
"#,
        )],
    );

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).unwrap();
    let shared_loader = Arc::new(RwLock::new(loader));
    let bridge = Arc::new(
        LanguageFeatureBridge::with_defaults()
            .with_shared_extension_loader(Arc::clone(&shared_loader))
            .with_fs_root(&proj),
    );

    let mut proc = NodeExtHostProcess::spawn(
        Arc::clone(&bridge),
        NodeExtHostConfig {
            cwd: Some(repo_root()),
            wire_format: RpcWireFormat::ContentLengthJson,
            ..Default::default()
        },
    )
    .expect("spawn NodeExtHostProcess");

    bridge.activate_extension("eamodio.gitlens").await.unwrap();
    for _ in 0..40 {
        if bridge.command_registry().has_command("gitlens.openGraph") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let app_uri = path_to_file_uri(&app_rs);
    bridge
        .request_execute_command("gitlens.openGraph", vec![json!(app_uri)])
        .await
        .unwrap();

    let mut win = WorkbenchWindow::default().with_language_bridge(Arc::clone(&bridge));
    win.open_workspace(&proj);
    win.sync_webviews();

    assert!(bridge.scm_manager.get_source_control("gitlens").is_some());
    assert!(win.view.editor_area.webviews.values().any(|w| w.html.contains("GitLens Graph")));

    // Real GitCliBackend status -> stage -> commit
    assert_eq!(win.view.scm_view.repo.changes.len(), 1);
    win.handle_scm_action(&ScmAction::StageAll).unwrap();
    win.handle_scm_action(&ScmAction::Commit {
        message: "GitLens commit".to_string(),
    })
    .unwrap();
    assert!(GitCliBackend::new(proj)
        .log(5)
        .unwrap()
        .iter()
        .any(|c| c.message.contains("GitLens commit")));

    let _ = proc.shutdown().await;
}

/// [M2-01, M2-02, M2-07, INV-03] Real CLI binary smoke checks across zero-arg, --headless, --version, and invalid flags.
#[test]
fn t4_06_cli_binary_smoke_all_flags() {
    let zero = run_rustcode_cli_args(&[]);
    assert!(zero.status.success(), "zero-arg CLI failed: {}", String::from_utf8_lossy(&zero.stderr));
    let zero_out = String::from_utf8_lossy(&zero.stdout);
    assert!(
        zero_out.contains("Rendered GPUI Scene:")
            && zero_out.contains("Bootstrap completed successfully."),
        "F17 zero-arg contract broken: {zero_out}"
    );

    let headless = run_rustcode_cli_args(&["--headless"]);
    assert!(headless.status.success());
    let headless_out = String::from_utf8_lossy(&headless.stdout);
    assert!(headless_out.contains("Rendered GPUI Scene:"));

    let version = run_rustcode_cli_args(&["--version"]);
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("rustcode "));

    let bogus = run_rustcode_cli_args(&["--bogus-flag"]);
    assert!(!bogus.status.success());
    assert!(!String::from_utf8_lossy(&bogus.stderr).trim().is_empty());

    let missing_goto = run_rustcode_cli_args(&["--goto"]);
    assert!(!missing_goto.status.success());
}
