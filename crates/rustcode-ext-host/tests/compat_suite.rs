//! Extension Compatibility Test Suite
//!
//! Comprehensive integration test suite validating real-world VS Code extension scenarios
//! using synthetic fixtures under `crates/rustcode-ext-host/tests/fixtures/extensions/`:
//! - Manifest parsing for all contribution points (commands, languages, grammars, themes, keybindings, configuration, views)
//! - Activation events lifecycle (`*`, `onLanguage`, `onCommand`, `onView`)
//! - Command registration and bidirectional RPC execution
//! - Workspace edits applied to PieceTable buffer with atomic undo
//! - Providers (DocumentFormatting, CodeAction, TreeData, status bar, diagnostics)

use rustcode_editor::{EditorView, Range, RangeEdit};
use rustcode_ext_host::manifest::ExtensionLoader;
use rustcode_ext_host::provider::{
    DiagnosticCollectionManager, LanguageFeatureBridge, ProviderRegistration, ProviderType,
    TreeItem, TreeItemCollapsibleState,
};
use rustcode_ext_host::rpc::{RpcBridge, RpcMessage};
use rustcode_platform::theme::{ThemeKind, ThemeService};
use rustcode_protocols::lsp::{
    CodeAction, CodeActionContext, CodeActionKind, DiagnosticSeverity, FormattingOptions,
    LspClientManager, LspCommand, LspDiagnostic, LspPosition, LspRange, LspTextEdit, WorkspaceEdit,
};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extensions")
}

#[test]
fn test_compat_manifest_parsing_all_contributions() {
    let dir = fixtures_dir();

    // 1. theme_ext
    let theme_pkg = std::fs::read_to_string(dir.join("theme_ext/package.json")).unwrap();
    let theme_manifest = ExtensionLoader::parse_manifest(&theme_pkg).unwrap();
    assert_eq!(theme_manifest.name, "theme-oceanic");
    assert_eq!(theme_manifest.publisher, "rustcode");
    assert_eq!(theme_manifest.activation_events, vec!["*"]);
    let themes = theme_manifest.contributes.as_ref().unwrap().themes.as_ref().unwrap();
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0].label, "Oceanic Next");
    assert_eq!(themes[0].ui_theme, "vs-dark");

    // 2. formatter_ext
    let fmt_pkg = std::fs::read_to_string(dir.join("formatter_ext/package.json")).unwrap();
    let fmt_manifest = ExtensionLoader::parse_manifest(&fmt_pkg).unwrap();
    assert_eq!(fmt_manifest.name, "rust-formatter-ext");
    assert_eq!(fmt_manifest.activation_events, vec!["onLanguage:rust"]);
    let languages = fmt_manifest.contributes.as_ref().unwrap().languages.as_ref().unwrap();
    assert_eq!(languages[0].id, "rust");
    assert_eq!(languages[0].extensions, vec![".rs"]);
    let commands = fmt_manifest.contributes.as_ref().unwrap().commands.as_ref().unwrap();
    assert_eq!(commands[0].command, "rustFormatter.formatDocument");

    // 3. codeactions_ext
    let ca_pkg = std::fs::read_to_string(dir.join("codeactions_ext/package.json")).unwrap();
    let ca_manifest = ExtensionLoader::parse_manifest(&ca_pkg).unwrap();
    assert_eq!(ca_manifest.name, "rust-quickfix-ext");
    assert_eq!(ca_manifest.activation_events, vec!["onLanguage:rust"]);

    // 4. treeview_ext
    let tree_pkg = std::fs::read_to_string(dir.join("treeview_ext/package.json")).unwrap();
    let tree_manifest = ExtensionLoader::parse_manifest(&tree_pkg).unwrap();
    assert_eq!(tree_manifest.name, "project-outline-ext");
    assert_eq!(tree_manifest.activation_events, vec!["onView:projectOutlineView"]);
    let views = tree_manifest.contributes.as_ref().unwrap().views.as_ref().unwrap();
    let explorer_views = views.get("explorer").unwrap();
    assert_eq!(explorer_views[0].id, "projectOutlineView");
    assert_eq!(explorer_views[0].name, "Project Outline");

    // 5. statusbar_ext
    let sb_pkg = std::fs::read_to_string(dir.join("statusbar_ext/package.json")).unwrap();
    let sb_manifest = ExtensionLoader::parse_manifest(&sb_pkg).unwrap();
    assert_eq!(sb_manifest.name, "git-status-ext");
    assert_eq!(sb_manifest.activation_events, vec!["*"]);
    let sb_commands = sb_manifest.contributes.as_ref().unwrap().commands.as_ref().unwrap();
    assert_eq!(sb_commands.len(), 2);

    // 6. linter_ext
    let linter_pkg = std::fs::read_to_string(dir.join("linter_ext/package.json")).unwrap();
    let linter_manifest = ExtensionLoader::parse_manifest(&linter_pkg).unwrap();
    assert_eq!(linter_manifest.name, "eslint-diagnostics-ext");
    assert_eq!(
        linter_manifest.activation_events,
        vec!["onLanguage:javascript", "onLanguage:typescript"]
    );
}

#[test]
fn test_compat_activation_events_lifecycle() {
    let dir = fixtures_dir();
    let mut loader = ExtensionLoader::new();

    let exts = [
        "theme_ext",
        "formatter_ext",
        "codeactions_ext",
        "treeview_ext",
        "statusbar_ext",
        "linter_ext",
    ];

    for ext in &exts {
        let loaded = loader.load_from_dir(dir.join(ext)).unwrap();
        assert!(!loaded.is_activated);
    }
    assert_eq!(loader.loaded_extensions().len(), 6);

    // 1. Wildcard '*' activation event matches theme_ext and statusbar_ext
    let wildcards = loader.extensions_for_event("*");
    let wildcard_names: Vec<&str> = wildcards.iter().map(|e| e.manifest.name.as_str()).collect();
    assert!(wildcard_names.contains(&"theme-oceanic"));
    assert!(wildcard_names.contains(&"git-status-ext"));

    // 2. 'onLanguage:rust' matches formatter_ext and codeactions_ext
    let rust_exts = loader.extensions_for_event("onLanguage:rust");
    let rust_names: Vec<&str> = rust_exts.iter().map(|e| e.manifest.name.as_str()).collect();
    assert!(rust_names.contains(&"rust-formatter-ext"));
    assert!(rust_names.contains(&"rust-quickfix-ext"));
    assert!(!rust_names.contains(&"eslint-diagnostics-ext"));

    // 3. 'onLanguage:javascript' matches linter_ext
    let js_exts = loader.extensions_for_event("onLanguage:javascript");
    let js_names: Vec<&str> = js_exts.iter().map(|e| e.manifest.name.as_str()).collect();
    assert!(js_names.contains(&"eslint-diagnostics-ext"));

    // 4. 'onView:projectOutlineView' matches treeview_ext
    let view_exts = loader.extensions_for_event("onView:projectOutlineView");
    let view_names: Vec<&str> = view_exts.iter().map(|e| e.manifest.name.as_str()).collect();
    assert!(view_names.contains(&"project-outline-ext"));

    // 5. Activation state tracking
    assert!(!loader.is_activated("git-status-ext"));
    assert!(loader.activate("git-status-ext"));
    assert!(loader.is_activated("git-status-ext"));
    assert!(loader.deactivate("git-status-ext"));
    assert!(!loader.is_activated("git-status-ext"));
}

#[test]
fn test_compat_theme_loading_and_palette_validation() {
    let theme_path = fixtures_dir().join("theme_ext/themes/oceanic-next.json");
    let theme_content = std::fs::read_to_string(&theme_path).unwrap();

    let theme = ThemeService::parse_theme_json(&theme_content).expect("valid theme");
    assert_eq!(theme.name, "Oceanic Next");
    assert_eq!(theme.kind, ThemeKind::Dark);

    // Validate color palette
    assert_eq!(theme.colors.get("editor.background"), Some(&"#1b2b34".to_string()));
    assert_eq!(theme.colors.get("editor.foreground"), Some(&"#d8dee9".to_string()));
    assert_eq!(theme.colors.get("statusBar.background"), Some(&"#1b2b34".to_string()));

    // Validate token syntax colors
    let keyword_rule = theme.token_colors.iter().find(|r| r.name.as_deref() == Some("Keywords")).unwrap();
    assert_eq!(keyword_rule.foreground.as_deref(), Some("#c594c5"));

    let string_rule = theme.token_colors.iter().find(|r| r.name.as_deref() == Some("Strings")).unwrap();
    assert_eq!(string_rule.foreground.as_deref(), Some("#99c794"));

    let fn_rule = theme.token_colors.iter().find(|r| r.name.as_deref() == Some("Functions")).unwrap();
    assert_eq!(fn_rule.foreground.as_deref(), Some("#6699cc"));
}

#[tokio::test]
async fn test_compat_commands_registration_and_bidirectional_rpc() {
    let bridge = RpcBridge::new();

    // 1. Outbound request from Rust core to extension host
    let (req, rx) = bridge.create_request(
        "git.checkoutBranch",
        json!({
            "branch": "feat/extension-host",
            "createIfMissing": true
        }),
    );

    let req_id = match req {
        RpcMessage::Request { id, method, params } => {
            assert_eq!(method, "git.checkoutBranch");
            assert_eq!(params["branch"], "feat/extension-host");
            id
        }
        _ => panic!("Expected RpcMessage::Request"),
    };

    // 2. Extension host fulfills request
    let handled = bridge.handle_incoming_response(
        req_id,
        Some(json!({ "checkedOut": true, "branch": "feat/extension-host" })),
        None,
    );
    assert!(handled);

    let res = rx.await.unwrap().unwrap();
    assert_eq!(res["checkedOut"], true);
    assert_eq!(res["branch"], "feat/extension-host");

    // 3. MessagePack serialization roundtrip for binary IPC
    let msg = RpcMessage::Notification {
        channel: "workbench.onDidActiveEditorChange".to_string(),
        data: json!({ "uri": "file:///workspace/src/lib.rs" }),
    };
    let encoded = msg.to_msgpack().unwrap();
    let decoded = RpcMessage::from_msgpack(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[tokio::test]
async fn test_compat_document_formatting_provider_and_piecetable() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp_mgr = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg.clone(), rpc.clone(), lsp_mgr);

    // 1. Register formatting provider for rust
    reg.register(ProviderRegistration {
        id: "rust-formatter-ext".to_string(),
        provider_type: ProviderType::DocumentFormatting,
        language_selector: Some("rust".to_string()),
    });

    let uri = "file:///workspace/src/calc.rs";
    let initial_code = "fn add(a:i32,b:i32)->i32{a+b}";
    let mut editor = EditorView::from_str(initial_code);

    // 2. Request formatting asynchronously
    let bridge_clone = Arc::new(bridge);
    let bridge_task = {
        let b = bridge_clone.clone();
        tokio::spawn(async move {
            b.request_formatting(uri, "rust", FormattingOptions::new(4, true)).await
        })
    };

    tokio::task::yield_now().await;

    // 3. Extension host returns formatted edits
    let formatted_code = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
    rpc.handle_incoming_response(
        1,
        Some(json!([
            {
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": initial_code.len() as u32 }
                },
                "newText": formatted_code
            }
        ])),
        None,
    );

    let edits = bridge_task.await.unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].new_text, formatted_code);

    // 4. Apply text edits to PieceTable buffer
    let range_edits: Vec<RangeEdit> = edits
        .into_iter()
        .map(|e| RangeEdit {
            range: Range::new(
                rustcode_editor::Position::new(e.range.start.line as usize, e.range.start.character as usize),
                rustcode_editor::Position::new(e.range.end.line as usize, e.range.end.character as usize),
            ),
            new_text: e.new_text,
        })
        .collect();

    let applied = editor.apply_text_edits(&range_edits);
    assert!(applied);
    assert_eq!(editor.text(), formatted_code);
}

#[tokio::test]
async fn test_compat_code_actions_quickfix_and_workspace_edit_transaction() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp_mgr = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg.clone(), rpc.clone(), lsp_mgr);

    reg.register(ProviderRegistration {
        id: "rust-quickfix-ext".to_string(),
        provider_type: ProviderType::CodeAction,
        language_selector: Some("rust".to_string()),
    });

    let uri = "file:///workspace/src/main.rs";
    let initial_code = "fn main() {\n    let val = Arc::new(42);\n}\n";
    let mut editor = EditorView::from_str(initial_code);

    let bridge_task = {
        let b = Arc::new(bridge);
        tokio::spawn(async move {
            b.request_code_actions(
                uri,
                "rust",
                LspRange::new(LspPosition::new(1, 14), LspPosition::new(1, 17)),
                CodeActionContext::default(),
            )
            .await
        })
    };

    tokio::task::yield_now().await;

    // Extension responds with QuickFix containing WorkspaceEdit
    let import_edit = "use std::sync::Arc;\n\n";
    let mut changes = HashMap::new();
    changes.insert(
        uri.to_string(),
        vec![LspTextEdit {
            range: LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 0)),
            new_text: import_edit.to_string(),
        }],
    );

    let quickfix = CodeAction {
        title: "Import std::sync::Arc".to_string(),
        kind: Some(CodeActionKind::QUICKFIX.to_string()),
        diagnostics: None,
        is_preferred: Some(true),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
        }),
        command: None,
    };

    rpc.handle_incoming_response(1, Some(json!([quickfix])), None);

    let actions = bridge_task.await.unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].title, "Import std::sync::Arc");
    assert_eq!(actions[0].kind.as_deref(), Some("quickfix"));

    // Apply WorkspaceEdit to EditorView with transactional undo/redo
    let ws_edit = actions[0].edit.as_ref().unwrap();
    let file_edits = ws_edit.changes.as_ref().unwrap().get(uri).unwrap();
    let range_edits: Vec<RangeEdit> = file_edits
        .iter()
        .map(|e| RangeEdit {
            range: Range::new(
                rustcode_editor::Position::new(e.range.start.line as usize, e.range.start.character as usize),
                rustcode_editor::Position::new(e.range.end.line as usize, e.range.end.character as usize),
            ),
            new_text: e.new_text.clone(),
        })
        .collect();

    assert!(editor.apply_text_edits(&range_edits));
    assert!(editor.text().starts_with("use std::sync::Arc;"));

    // Verify atomic undo reverts entire QuickFix
    assert!(editor.undo());
    assert_eq!(editor.text(), initial_code);

    // Verify redo restores the QuickFix
    assert!(editor.redo());
    assert!(editor.text().starts_with("use std::sync::Arc;"));
}

#[tokio::test]
async fn test_compat_tree_view_provider_hierarchical_items() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp_mgr = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg.clone(), rpc.clone(), lsp_mgr);

    reg.register(ProviderRegistration {
        id: "projectOutlineView".to_string(),
        provider_type: ProviderType::TreeData,
        language_selector: None,
    });

    // 1. Request root tree items
    let bridge_arc = Arc::new(bridge);
    let root_task = {
        let b = bridge_arc.clone();
        tokio::spawn(async move {
            b.request_tree_children("projectOutlineView", None).await
        })
    };

    tokio::task::yield_now().await;

    let root_items = vec![
        TreeItem {
            id: Some("node_src".to_string()),
            label: "src".to_string(),
            description: Some("Source folder".to_string()),
            tooltip: Some("Project source code".to_string()),
            collapsible_state: TreeItemCollapsibleState::Collapsed,
            icon_path: Some("folder".to_string()),
            context_value: Some("folder".to_string()),
            command: None,
        },
        TreeItem {
            id: Some("node_cargo".to_string()),
            label: "Cargo.toml".to_string(),
            description: None,
            tooltip: Some("Manifest".to_string()),
            collapsible_state: TreeItemCollapsibleState::None,
            icon_path: Some("file".to_string()),
            context_value: Some("file".to_string()),
            command: Some(LspCommand {
                title: "Open Cargo.toml".to_string(),
                command: "workbench.action.openFile".to_string(),
                arguments: Some(vec![json!("Cargo.toml")]),
            }),
        },
    ];

    rpc.handle_incoming_response(1, Some(json!(root_items)), None);
    let roots = root_task.await.unwrap();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].label, "src");
    assert_eq!(roots[0].collapsible_state, TreeItemCollapsibleState::Collapsed);
    assert_eq!(roots[1].label, "Cargo.toml");
    assert_eq!(roots[1].collapsible_state, TreeItemCollapsibleState::None);

    // 2. Request children of "node_src"
    let child_task = {
        let b = bridge_arc.clone();
        tokio::spawn(async move {
            b.request_tree_children("projectOutlineView", Some("node_src")).await
        })
    };

    tokio::task::yield_now().await;

    let child_items = vec![
        TreeItem {
            id: Some("node_main".to_string()),
            label: "main.rs".to_string(),
            description: None,
            tooltip: None,
            collapsible_state: TreeItemCollapsibleState::None,
            icon_path: Some("file-code".to_string()),
            context_value: Some("file".to_string()),
            command: None,
        },
        TreeItem {
            id: Some("node_lib".to_string()),
            label: "lib.rs".to_string(),
            description: None,
            tooltip: None,
            collapsible_state: TreeItemCollapsibleState::None,
            icon_path: Some("file-code".to_string()),
            context_value: Some("file".to_string()),
            command: None,
        },
    ];

    rpc.handle_incoming_response(2, Some(json!(child_items)), None);
    let children = child_task.await.unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].label, "main.rs");
    assert_eq!(children[1].label, "lib.rs");
}

#[tokio::test]
async fn test_compat_status_bar_item_lifecycle_and_priority_sorting() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp_mgr = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg, rpc, lsp_mgr);

    // 1. Extension registers status bar item: branch indicator (priority 100)
    let branch_params = json!({
        "id": "git_branch",
        "text": "$(git-branch) main",
        "tooltip": "Git Branch: main",
        "command": "git.checkoutBranch",
        "alignment": 1,
        "priority": 100
    });
    let res1 = bridge.handle_rpc_message("window.setStatusBarItem", &branch_params).await;
    assert!(res1.is_some());

    // 2. Extension registers sync indicator (priority 200)
    let sync_params = json!({
        "id": "git_sync",
        "text": "$(sync) Up to date",
        "tooltip": "Repository synchronized",
        "command": "git.syncRepository",
        "alignment": 1,
        "priority": 200
    });
    let res2 = bridge.handle_rpc_message("window.setStatusBarItem", &sync_params).await;
    assert!(res2.is_some());

    // 3. Verify priority ordering (higher priority comes first)
    let items = bridge.status_bar_manager.get_visible_items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, "git_sync"); // priority 200
    assert_eq!(items[1].id, "git_branch"); // priority 100

    // 4. Update status bar item text dynamically
    let update_params = json!({
        "id": "git_branch",
        "text": "$(git-branch) feat/compat-suite",
        "tooltip": "Branch: feat/compat-suite",
        "command": "git.checkoutBranch",
        "alignment": 1,
        "priority": 100
    });
    bridge.handle_rpc_message("window.setStatusBarItem", &update_params).await;
    let updated = bridge.status_bar_manager.get_item("git_branch").unwrap();
    assert_eq!(updated.text, "$(git-branch) feat/compat-suite");

    // 5. Hide status bar item
    let hide_params = json!({ "id": "git_sync" });
    bridge.handle_rpc_message("window.hideStatusBarItem", &hide_params).await;
    let visible = bridge.status_bar_manager.get_visible_items();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, "git_branch");
}

#[tokio::test]
async fn test_compat_linter_diagnostics_collection_and_severity() {
    let diag_manager = Arc::new(DiagnosticCollectionManager::new());
    let notifications = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let notify_clone = notifications.clone();

    diag_manager.on_diagnostics(move |uri, diags| {
        notify_clone.lock().push((uri.to_string(), diags.len()));
    });

    let uri = "file:///workspace/src/app.js";

    // 1. Publish diagnostics with all 4 VS Code severity levels
    let diags = vec![
        LspDiagnostic {
            range: LspRange::new(LspPosition::new(1, 0), LspPosition::new(1, 10)),
            severity: Some(DiagnosticSeverity::Error),
            code: Some(rustcode_protocols::lsp::DiagnosticCode::String("no-undef".to_string())),
            source: Some("eslint".to_string()),
            message: "'foo' is not defined".to_string(),
        },
        LspDiagnostic {
            range: LspRange::new(LspPosition::new(5, 4), LspPosition::new(5, 12)),
            severity: Some(DiagnosticSeverity::Warning),
            code: Some(rustcode_protocols::lsp::DiagnosticCode::String("no-unused-vars".to_string())),
            source: Some("eslint".to_string()),
            message: "'bar' is defined but never used".to_string(),
        },
        LspDiagnostic {
            range: LspRange::new(LspPosition::new(10, 0), LspPosition::new(10, 5)),
            severity: Some(DiagnosticSeverity::Information),
            code: None,
            source: Some("eslint".to_string()),
            message: "Consider using const instead of let".to_string(),
        },
        LspDiagnostic {
            range: LspRange::new(LspPosition::new(12, 0), LspPosition::new(12, 2)),
            severity: Some(DiagnosticSeverity::Hint),
            code: None,
            source: Some("eslint".to_string()),
            message: "Unnecessary semicolon".to_string(),
        },
    ];

    let publish_params = json!({
        "name": "eslint",
        "uri": uri,
        "diagnostics": diags
    });

    let handled = diag_manager.handle_rpc_message("languages.setDiagnostics", &publish_params);
    assert!(handled);

    // Verify diagnostics stored and retrieved
    let retrieved = diag_manager.get_diagnostics_for_uri(uri);
    assert_eq!(retrieved.len(), 4);
    assert_eq!(retrieved[0].severity, Some(DiagnosticSeverity::Error));
    assert_eq!(retrieved[1].severity, Some(DiagnosticSeverity::Warning));
    assert_eq!(retrieved[2].severity, Some(DiagnosticSeverity::Information));
    assert_eq!(retrieved[3].severity, Some(DiagnosticSeverity::Hint));

    // Verify notification fired
    {
        let events = notifications.lock();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], (uri.to_string(), 4));
    }

    // 2. Clear diagnostics for file
    let clear_params = json!({
        "name": "eslint",
        "uri": uri
    });
    assert!(diag_manager.handle_rpc_message("languages.clearDiagnostics", &clear_params));
    assert!(diag_manager.get_diagnostics_for_uri(uri).is_empty());

    {
        let events = notifications.lock();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1], (uri.to_string(), 0));
    }
}

#[tokio::test]
async fn test_compat_full_extension_end_to_end_workflow() {
    let dir = fixtures_dir();
    let mut loader = ExtensionLoader::new();
    let loaded = loader.load_from_dir(dir.join("codeactions_ext")).unwrap();
    assert_eq!(loaded.manifest.name, "rust-quickfix-ext");

    // 1. Activation lifecycle
    assert!(loader.activate("rust-quickfix-ext"));
    assert!(loader.is_activated("rust-quickfix-ext"));

    // 2. Setup RPC and LanguageFeatureBridge
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp_mgr = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg.clone(), rpc.clone(), lsp_mgr);

    reg.register(ProviderRegistration {
        id: "rust-quickfix-ext".to_string(),
        provider_type: ProviderType::CodeAction,
        language_selector: Some("rust".to_string()),
    });

    // 3. Document has missing import error
    let uri = "file:///workspace/src/lib.rs";
    let initial_source = "pub fn allocate() -> Box<u32> {\n    Box::new(100)\n}\n";
    let mut editor = EditorView::from_str(initial_source);

    // 4. Request code actions
    let bridge_task = {
        let b = Arc::new(bridge);
        tokio::spawn(async move {
            b.request_code_actions(
                uri,
                "rust",
                LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 5)),
                CodeActionContext::default(),
            )
            .await
        })
    };

    tokio::task::yield_now().await;

    // Fulfill with a code action that adds a doc comment
    let doc_comment = "/// Allocates a heap boxed integer.\n";
    let mut changes = HashMap::new();
    changes.insert(
        uri.to_string(),
        vec![LspTextEdit {
            range: LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 0)),
            new_text: doc_comment.to_string(),
        }],
    );

    rpc.handle_incoming_response(
        1,
        Some(json!([
            CodeAction {
                title: "Generate doc comment".to_string(),
                kind: Some(CodeActionKind::QUICKFIX.to_string()),
                diagnostics: None,
                is_preferred: Some(true),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                }),
                command: None,
            }
        ])),
        None,
    );

    let actions = bridge_task.await.unwrap();
    assert_eq!(actions.len(), 1);

    // 5. Apply edit with transactional undo
    let ws_edit = actions[0].edit.as_ref().unwrap();
    let edits = ws_edit.changes.as_ref().unwrap().get(uri).unwrap();
    let range_edits: Vec<RangeEdit> = edits
        .iter()
        .map(|e| RangeEdit {
            range: Range::new(
                rustcode_editor::Position::new(e.range.start.line as usize, e.range.start.character as usize),
                rustcode_editor::Position::new(e.range.end.line as usize, e.range.end.character as usize),
            ),
            new_text: e.new_text.clone(),
        })
        .collect();

    assert!(editor.apply_text_edits(&range_edits));
    assert!(editor.text().starts_with("/// Allocates"));

    // 6. Undo restores original
    assert!(editor.undo());
    assert_eq!(editor.text(), initial_source);
}

#[tokio::test]
async fn test_compat_notebook_provider_and_execution_lifecycle() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp_mgr = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg.clone(), rpc.clone(), lsp_mgr);

    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let events_clone = events.clone();
    bridge.notebook_manager.on_notebook_event(move |name, _| {
        events_clone.lock().push(name);
    });

    // 1. Register NotebookSerializer and NotebookController proxies
    let ser_reg = bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({
                "type": "NotebookSerializer",
                "id": "jupyter-notebook",
                "selector": "jupyter-notebook"
            }),
        )
        .await
        .unwrap();
    assert_eq!(ser_reg["registered"], true);

    let ctrl_reg = bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({
                "type": "NotebookController",
                "id": "rust-kernel",
                "selector": "jupyter-notebook"
            }),
        )
        .await
        .unwrap();
    assert_eq!(ctrl_reg["registered"], true);

    assert_eq!(
        reg.get_providers_for_language(ProviderType::NotebookSerializer, "jupyter-notebook")
            .len(),
        1
    );
    assert_eq!(
        reg.get_providers_for_language(ProviderType::NotebookController, "jupyter-notebook")
            .len(),
        1
    );

    // 2. Register serializer, create controller, run execution lifecycle
    bridge
        .handle_rpc_message(
            "notebooks.registerSerializer",
            &json!({ "id": "jupyter-notebook", "notebookType": "jupyter-notebook" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "notebooks.createController",
            &json!({
                "id": "rust-kernel",
                "notebookType": "jupyter-notebook",
                "label": "Pure Rust Kernel",
                "supportedLanguages": ["python"],
                "supportsExecutionOrder": true
            }),
        )
        .await
        .unwrap();

    bridge
        .handle_rpc_message(
            "notebooks.updateExecution",
            &json!({
                "controllerId": "rust-kernel",
                "notebookUri": "file:///demo.ipynb",
                "cellIndex": 0,
                "state": "start",
                "executionOrder": 1
            }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "notebooks.updateCellOutputs",
            &json!({
                "notebookUri": "file:///demo.ipynb",
                "cellIndex": 0,
                "outputs": [{ "items": [{ "mime": "text/plain", "data": [52, 50] }] }]
            }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "notebooks.appendCellOutputs",
            &json!({
                "notebookUri": "file:///demo.ipynb",
                "cellIndex": 0,
                "outputs": [{ "items": [{ "mime": "application/vnd.code.notebook.stdout", "data": [111, 107] }] }]
            }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "notebooks.updateExecution",
            &json!({
                "controllerId": "rust-kernel",
                "notebookUri": "file:///demo.ipynb",
                "cellIndex": 0,
                "state": "end",
                "success": true
            }),
        )
        .await
        .unwrap();

    let exec_state = bridge
        .notebook_manager
        .get_execution("file:///demo.ipynb", 0)
        .unwrap();
    assert_eq!(exec_state.state, "succeeded");
    assert_eq!(exec_state.outputs.len(), 2);

    // 3. Cleanup controller and serializer
    bridge
        .handle_rpc_message("notebooks.disposeController", &json!({ "id": "rust-kernel" }))
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "notebooks.unregisterSerializer",
            &json!({ "id": "jupyter-notebook" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message("extHost.unregisterProvider", &json!({ "id": "rust-kernel" }))
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "extHost.unregisterProvider",
            &json!({ "id": "jupyter-notebook" }),
        )
        .await
        .unwrap();

    assert!(bridge.notebook_manager.list_controllers().is_empty());
    assert!(bridge.notebook_manager.list_serializers().is_empty());
    assert_eq!(events.lock().len(), 8);
}

#[tokio::test]
async fn test_compat_notebook_serializer_and_controller_rpc_roundtrip() {
    use rustcode_ext_host::notebook::{
        ExtNotebookCellData, ExtNotebookCellKind, ExtNotebookCellOutput,
        ExtNotebookCellOutputItem, ExtNotebookData,
    };

    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp_mgr = Arc::new(LspClientManager::new());
    let bridge = Arc::new(LanguageFeatureBridge::new(reg, rpc.clone(), lsp_mgr));

    // 1. Deserialize RPC roundtrip (id = 1)
    let rpc1 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc1.handle_incoming_response(
            1,
            Some(json!({
                "cells": [{
                    "kind": ExtNotebookCellKind::CODE,
                    "value": "let x = 21\nx * 2",
                    "languageId": "python",
                    "outputs": []
                }],
                "metadata": { "nbformat": 4 }
            })),
            None,
        );
    });

    let nb_data = bridge
        .request_notebook_deserialize("jupyter-notebook", b"{\"cells\":[]}")
        .await
        .unwrap();
    assert_eq!(nb_data.cells.len(), 1);
    assert_eq!(nb_data.cells[0].kind, ExtNotebookCellKind::CODE);
    assert_eq!(nb_data.cells[0].value, "let x = 21\nx * 2");

    // 2. Serialize RPC roundtrip (id = 2)
    let expected_bytes = b"{\"nbformat\":4}".to_vec();
    let rpc2 = rpc.clone();
    let resp_bytes = expected_bytes.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc2.handle_incoming_response(2, Some(json!(resp_bytes)), None);
    });

    let ser_bytes = bridge
        .request_notebook_serialize(
            "jupyter-notebook",
            &ExtNotebookData {
                cells: vec![ExtNotebookCellData {
                    kind: ExtNotebookCellKind::CODE,
                    value: "1 + 1".to_string(),
                    language_id: "python".to_string(),
                    outputs: vec![ExtNotebookCellOutput {
                        items: vec![ExtNotebookCellOutputItem::text("2", "text/plain")],
                        metadata: None,
                    }],
                    metadata: None,
                    execution_summary: None,
                }],
                metadata: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(ser_bytes, expected_bytes);

    // 3. Execute cells RPC roundtrip (id = 3)
    let rpc3 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc3.handle_incoming_response(3, Some(json!({ "executed": 1 })), None);
    });

    let exec_res = bridge
        .request_notebook_execute_cells(
            "rust-kernel",
            "file:///demo.ipynb",
            &[0],
            &nb_data.cells,
        )
        .await
        .unwrap();
    assert_eq!(exec_res["executed"], 1);
}

#[test]
fn test_compat_notebook_js_api_export_surface() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let notebooks_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/notebooks.js")).unwrap();
    let index_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/index.js")).unwrap();
    let rpc_bridge_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/rpc-bridge.js")).unwrap();
    let test_js = std::fs::read_to_string(repo_root.join("ext-host-js/test.js")).unwrap();

    for symbol in [
        "NotebookCellKind",
        "NotebookCellData",
        "NotebookData",
        "NotebookCellOutput",
        "NotebookCellOutputItem",
        "NotebookRange",
        "NotebookEdit",
        "NotebookController",
        "NotebookCellExecution",
        "registerNotebookSerializer",
        "createNotebookController",
        "createNotebookCellExecution",
        "onDidOpenNotebookDocument",
        "onDidCloseNotebookDocument",
        "onDidChangeNotebookDocument",
        "onDidSaveNotebookDocument",
        "onDidChangeActiveNotebookEditor",
        "toJSON",
    ] {
        assert!(
            notebooks_js.contains(symbol),
            "notebooks.js missing symbol {}",
            symbol
        );
    }

    for exported in [
        "require('./notebooks')",
        "notebooks,",
        "NotebookCellKind,",
        "NotebookCellData,",
        "NotebookData,",
        "NotebookCellOutput,",
        "NotebookCellOutputItem,",
        "NotebookRange,",
        "NotebookEdit,",
    ] {
        assert!(
            index_js.contains(exported),
            "index.js missing export {}",
            exported
        );
    }

    for rpc_method in [
        "notebooks.deserializeNotebook",
        "notebooks.serializeNotebook",
        "notebooks.executeCells",
        "notebooks.interruptExecution",
    ] {
        assert!(
            rpc_bridge_js.contains(rpc_method),
            "rpc-bridge.js missing handler {}",
            rpc_method
        );
    }

    assert!(test_js.contains("vscode.notebooks"));

    // Verify balanced (), {}, [] delimiters in notebooks.js and index.js
    for (name, src) in [("notebooks.js", &notebooks_js), ("index.js", &index_js)] {
        let mut parens = 0i32;
        let mut braces = 0i32;
        let mut brackets = 0i32;
        for ch in src.chars() {
            match ch {
                '(' => parens += 1,
                ')' => parens -= 1,
                '{' => braces += 1,
                '}' => braces -= 1,
                '[' => brackets += 1,
                ']' => brackets -= 1,
                _ => {}
            }
            assert!(parens >= 0 && braces >= 0 && brackets >= 0, "Unbalanced closer in {}", name);
        }
        assert_eq!(parens, 0, "Unbalanced () in {}", name);
        assert_eq!(braces, 0, "Unbalanced {{}} in {}", name);
        assert_eq!(brackets, 0, "Unbalanced [] in {}", name);
    }
}

fn assert_js_delimiters_balanced(name: &str, src: &str) {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Code,
        LineComment,
        BlockComment,
        SingleQuote,
        DoubleQuote,
        Template,
    }

    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut state = State::Code;
    let mut template_expr_brace_stack: Vec<i32> = Vec::new();
    let mut parens = 0i32;
    let mut braces = 0i32;
    let mut brackets = 0i32;

    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();

        match state {
            State::LineComment => {
                if ch == '\n' {
                    state = State::Code;
                }
            }
            State::BlockComment => {
                if ch == '*' && next == Some('/') {
                    state = State::Code;
                    i += 1;
                }
            }
            State::SingleQuote => {
                if ch == '\\' {
                    i += 1;
                } else if ch == '\'' {
                    state = State::Code;
                }
            }
            State::DoubleQuote => {
                if ch == '\\' {
                    i += 1;
                } else if ch == '"' {
                    state = State::Code;
                }
            }
            State::Template => {
                if ch == '\\' {
                    i += 1;
                } else if ch == '`' {
                    state = State::Code;
                } else if ch == '$' && next == Some('{') {
                    braces += 1;
                    template_expr_brace_stack.push(braces);
                    state = State::Code;
                    i += 1;
                }
            }
            State::Code => {
                if ch == '/' && next == Some('/') {
                    state = State::LineComment;
                    i += 1;
                } else if ch == '/' && next == Some('*') {
                    state = State::BlockComment;
                    i += 1;
                } else if ch == '\'' {
                    state = State::SingleQuote;
                } else if ch == '"' {
                    state = State::DoubleQuote;
                } else if ch == '`' {
                    state = State::Template;
                } else {
                    match ch {
                        '(' => parens += 1,
                        ')' => parens -= 1,
                        '{' => braces += 1,
                        '}' => {
                            if template_expr_brace_stack.last().copied() == Some(braces) {
                                template_expr_brace_stack.pop();
                                braces -= 1;
                                state = State::Template;
                            } else {
                                braces -= 1;
                            }
                        }
                        '[' => brackets += 1,
                        ']' => brackets -= 1,
                        _ => {}
                    }
                    assert!(
                        parens >= 0 && braces >= 0 && brackets >= 0,
                        "Unbalanced closer in {}",
                        name
                    );
                }
            }
        }
        i += 1;
    }

    assert!(
        state == State::Code || state == State::LineComment,
        "Unterminated string or comment in {}",
        name
    );
    assert_eq!(parens, 0, "Unbalanced () in {}", name);
    assert_eq!(braces, 0, "Unbalanced {{}} in {}", name);
    assert_eq!(brackets, 0, "Unbalanced [] in {}", name);
}

#[tokio::test]
async fn test_compat_output_channel_rpc_lifecycle() {
    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(registry, rpc, lsp);

    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let events_clone = events.clone();
    bridge.output_manager.on_output_event(move |ev| {
        events_clone.lock().push(ev.clone());
    });

    let created = bridge
        .handle_rpc_message(
            "window.createOutputChannel",
            &serde_json::json!({
                "id": "out_1",
                "name": "RustCode Build",
                "languageId": "log",
                "log": false
            }),
        )
        .await
        .unwrap();
    assert_eq!(created["created"], true);
    assert_eq!(
        bridge.output_manager.active_channel_id().as_deref(),
        Some("out_1")
    );

    assert!(bridge
        .handle_rpc_message(
            "window.appendOutputChannel",
            &serde_json::json!({ "id": "out_1", "value": "step 1: " })
        )
        .await
        .is_some());
    assert!(bridge
        .handle_rpc_message(
            "window.appendOutputChannel",
            &serde_json::json!({ "id": "out_1", "value": "ok\nstep 2\n" })
        )
        .await
        .is_some());

    let state = bridge.output_manager.get_channel("out_1").unwrap();
    assert_eq!(state.lines(), vec!["step 1: ok", "step 2"]);

    assert!(bridge
        .handle_rpc_message(
            "window.replaceOutputChannel",
            &serde_json::json!({ "id": "out_1", "value": "fresh line\n" })
        )
        .await
        .is_some());
    assert_eq!(
        bridge.output_manager.get_channel("out_1").unwrap().lines(),
        vec!["fresh line"]
    );

    assert!(bridge
        .handle_rpc_message(
            "window.showOutputChannel",
            &serde_json::json!({ "id": "out_1", "preserveFocus": true })
        )
        .await
        .is_some());
    let shown = bridge.output_manager.get_channel("out_1").unwrap();
    assert!(shown.is_visible);
    assert!(shown.preserve_focus);

    assert!(bridge
        .handle_rpc_message("window.hideOutputChannel", &serde_json::json!({ "id": "out_1" }))
        .await
        .is_some());
    assert!(!bridge.output_manager.get_channel("out_1").unwrap().is_visible);

    assert!(bridge
        .handle_rpc_message("window.clearOutputChannel", &serde_json::json!({ "id": "out_1" }))
        .await
        .is_some());
    assert!(bridge
        .output_manager
        .get_channel("out_1")
        .unwrap()
        .lines()
        .is_empty());

    assert!(bridge
        .handle_rpc_message("window.disposeOutputChannel", &serde_json::json!({ "id": "out_1" }))
        .await
        .is_some());
    assert!(bridge.output_manager.get_channel("out_1").unwrap().is_disposed);
    assert!(bridge.output_manager.list_channels().is_empty());
    assert!(bridge
        .handle_rpc_message(
            "window.appendOutputChannel",
            &serde_json::json!({ "id": "out_1", "value": "ignored" })
        )
        .await
        .is_none());
    assert_eq!(events.lock().len(), 8);
}

#[test]
fn test_compat_output_channel_js_api_export_surface() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let window_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/window.js")).unwrap();
    let index_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/index.js")).unwrap();
    let test_js = std::fs::read_to_string(repo_root.join("ext-host-js/test.js")).unwrap();

    for symbol in [
        "OutputChannelProxy",
        "createOutputChannel",
        "append(",
        "appendLine(",
        "replace(",
        "clear(",
        "show(",
        "hide(",
        "dispose(",
        "window.createOutputChannel",
        "window.appendOutputChannel",
        "window.replaceOutputChannel",
        "window.clearOutputChannel",
        "window.showOutputChannel",
        "window.hideOutputChannel",
        "window.disposeOutputChannel",
        "createTerminal",
        "window.createTerminal",
        "window.showTerminal",
        "window.sendTerminalText",
        "window.disposeTerminal",
    ] {
        assert!(
            window_js.contains(symbol),
            "window.js missing symbol {}",
            symbol
        );
    }

    assert!(index_js.contains("require('./window')"));
    assert!(index_js.contains("window,"));
    assert!(test_js.contains("createOutputChannel"));
    assert!(test_js.contains("createTerminal"));

    for (name, src) in [
        ("window.js", &window_js),
        ("index.js", &index_js),
        ("test.js", &test_js),
    ] {
        assert_js_delimiters_balanced(name, src);
    }
}

#[tokio::test]
async fn test_compat_terminal_rpc_lifecycle() {
    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(registry, rpc, lsp);

    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let events_clone = events.clone();
    bridge.terminal_manager.on_terminal_event(move |ev| {
        events_clone.lock().push(ev.clone());
    });

    let created = bridge
        .handle_rpc_message(
            "window.createTerminal",
            &serde_json::json!({
                "id": "term_1",
                "name": "Ext Shell",
                "shellPath": "/bin/sh",
                "cwd": "/tmp"
            }),
        )
        .await
        .unwrap();
    assert_eq!(created["created"], true);
    assert_eq!(bridge.terminal_manager.list_terminals().len(), 1);

    assert!(bridge
        .handle_rpc_message(
            "window.showTerminal",
            &serde_json::json!({ "id": "term_1", "preserveFocus": false })
        )
        .await
        .is_some());
    assert!(bridge.terminal_manager.get_terminal("term_1").unwrap().is_visible);

    assert!(bridge
        .handle_rpc_message(
            "window.sendTerminalText",
            &serde_json::json!({ "id": "term_1", "text": "echo hi", "addNewLine": true })
        )
        .await
        .is_some());
    assert!(bridge
        .handle_rpc_message(
            "window.sendTerminalText",
            &serde_json::json!({ "id": "term_1", "text": "partial", "addNewLine": false })
        )
        .await
        .is_some());
    assert_eq!(
        bridge.terminal_manager.get_terminal("term_1").unwrap().sent_texts,
        vec!["echo hi\n", "partial"]
    );

    assert!(bridge
        .handle_rpc_message(
            "window.disposeTerminal",
            &serde_json::json!({ "id": "term_1" })
        )
        .await
        .is_some());
    assert!(bridge.terminal_manager.get_terminal("term_1").unwrap().is_disposed);
    assert!(bridge.terminal_manager.list_terminals().is_empty());
    assert_eq!(events.lock().len(), 5);
}
