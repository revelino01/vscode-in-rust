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
            resource_uri: None,
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
            resource_uri: None,
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
            resource_uri: None,
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
            resource_uri: None,
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
    let err_out = bridge
        .handle_rpc_message(
            "window.appendOutputChannel",
            &serde_json::json!({ "id": "out_1", "value": "ignored" }),
        )
        .await
        .unwrap();
    assert_eq!(err_out["error"]["code"], "INVALID_STATE");
    assert_eq!(
        bridge
            .handle_rpc_message_result(
                "window.appendOutputChannel",
                &serde_json::json!({ "id": "out_1", "value": "ignored" }),
            )
            .await
            .unwrap_err()
            .code,
        "INVALID_STATE"
    );
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

#[tokio::test]
async fn test_compat_scm_rpc_lifecycle() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg.clone(), rpc, lsp);

    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let events_clone = events.clone();
    bridge.scm_manager().on_scm_event(move |ev| {
        events_clone.lock().push(ev.clone());
    });

    let reg_res = bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({
                "type": "SourceControl",
                "id": "git",
                "selector": "git"
            }),
        )
        .await
        .unwrap();
    assert_eq!(reg_res["registered"], true);
    assert_eq!(
        reg.get_providers_for_language(ProviderType::SourceControl, "git")
            .len(),
        1
    );

    let rev0 = bridge.scm_manager().revision();
    bridge
        .handle_rpc_message(
            "scm.createSourceControl",
            &json!({ "id": "git", "label": "Git", "rootUri": "file:///workspace/demo" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "scm.updateInputBox",
            &json!({ "sourceControlId": "git", "value": "chore: phase 6 m2" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "scm.createResourceGroup",
            &json!({ "sourceControlId": "git", "id": "index", "label": "Staged Changes" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "scm.createResourceGroup",
            &json!({ "sourceControlId": "git", "id": "workingTree", "label": "Changes" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "scm.updateResourceGroup",
            &json!({
                "sourceControlId": "git",
                "id": "index",
                "resourceStates": [{
                    "resourceUri": "file:///workspace/demo/Cargo.toml",
                    "decorations": { "tooltip": "Added" }
                }]
            }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "scm.updateResourceGroup",
            &json!({
                "sourceControlId": "git",
                "id": "workingTree",
                "resourceStates": [{
                    "resourceUri": "file:///workspace/demo/src/main.rs",
                    "decorations": { "tooltip": "Modified" }
                }]
            }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "scm.updateSourceControl",
            &json!({ "id": "git", "count": 2, "hasQuickDiffProvider": true }),
        )
        .await
        .unwrap();

    let repo = bridge.scm_manager().to_scm_repository("git").unwrap();
    assert_eq!(repo.staged.len(), 1);
    assert_eq!(repo.changes.len(), 1);
    assert!(bridge.scm_manager().revision() > rev0);

    bridge
        .handle_rpc_message(
            "scm.disposeResourceGroup",
            &json!({ "sourceControlId": "git", "id": "workingTree" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message("scm.disposeSourceControl", &json!({ "id": "git" }))
        .await
        .unwrap();
    assert!(bridge.scm_manager().list_source_controls().is_empty());
    let scm_err = bridge
        .handle_rpc_message(
            "scm.updateInputBox",
            &json!({ "sourceControlId": "git", "value": "after dispose" }),
        )
        .await
        .unwrap();
    assert_eq!(scm_err["error"]["code"], "INVALID_STATE");
    assert_eq!(
        bridge
            .handle_rpc_message_result(
                "scm.updateInputBox",
                &json!({ "sourceControlId": "git", "value": "after dispose" }),
            )
            .await
            .unwrap_err()
            .code,
        "INVALID_STATE"
    );

    bridge
        .handle_rpc_message("extHost.unregisterProvider", &json!({ "id": "git" }))
        .await
        .unwrap();
    assert!(reg
        .get_providers_for_language(ProviderType::SourceControl, "git")
        .is_empty());
    assert_eq!(events.lock().len(), 9);
    assert_eq!(bridge.scm_manager().take_pending_events().len(), 9);
}

#[tokio::test]
async fn test_compat_debug_rpc_lifecycle() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg.clone(), rpc, lsp);

    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let events_clone = events.clone();
    bridge.debug_manager().on_debug_event(move |ev| {
        events_clone.lock().push(ev.clone());
    });

    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "DebugConfiguration", "id": "cfg_lldb", "selector": "lldb" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "DebugAdapterFactory", "id": "adp_lldb", "selector": "lldb" }),
        )
        .await
        .unwrap();
    assert_eq!(
        reg.get_providers_for_language(ProviderType::DebugConfiguration, "lldb")
            .len(),
        1
    );
    assert_eq!(
        reg.get_providers_for_language(ProviderType::DebugAdapterFactory, "lldb")
            .len(),
        1
    );

    bridge
        .handle_rpc_message(
            "debug.registerConfigurationProvider",
            &json!({ "id": "cfg_lldb", "debugType": "lldb", "triggerKind": 1 }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "debug.registerAdapterDescriptorFactory",
            &json!({ "id": "adp_lldb", "debugType": "lldb" }),
        )
        .await
        .unwrap();

    let started = bridge
        .handle_rpc_message(
            "debug.startDebugging",
            &json!({
                "folder": { "uri": "file:///workspace" },
                "nameOrConfiguration": { "type": "lldb", "name": "Debug Tests" }
            }),
        )
        .await
        .unwrap();
    let session_id = started["session"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        bridge.debug_manager().active_session().unwrap().name,
        "Debug Tests"
    );

    bridge
        .handle_rpc_message(
            "debug.addBreakpoints",
            &json!({
                "breakpoints": [{
                    "id": "bp_1",
                    "kind": "source",
                    "enabled": true,
                    "condition": "i == 42",
                    "location": {
                        "uri": "file:///workspace/src/lib.rs",
                        "range": { "start": { "line": 4, "character": 0 } }
                    }
                }]
            }),
        )
        .await
        .unwrap();
    let dap_bps = bridge
        .debug_manager()
        .to_dap_source_breakpoints("file:///workspace/src/lib.rs");
    assert_eq!(dap_bps.len(), 1);
    assert_eq!(dap_bps[0].line, 5);

    bridge
        .handle_rpc_message(
            "debug.appendConsole",
            &json!({ "value": "ready", "newLine": true }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "debug.sendCustomEvent",
            &json!({ "sessionId": session_id, "event": "outputEx", "body": { "ok": true } }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message("debug.removeBreakpoints", &json!({ "ids": ["bp_1"] }))
        .await
        .unwrap();
    assert!(bridge.debug_manager().list_breakpoints().is_empty());

    bridge
        .handle_rpc_message("debug.stopDebugging", &json!({ "sessionId": session_id }))
        .await
        .unwrap();
    assert!(bridge.debug_manager().active_session().is_none());

    bridge
        .handle_rpc_message("extHost.unregisterProvider", &json!({ "id": "cfg_lldb" }))
        .await
        .unwrap();
    bridge
        .handle_rpc_message("extHost.unregisterProvider", &json!({ "id": "adp_lldb" }))
        .await
        .unwrap();
    assert_eq!(events.lock().len(), 8);
}

#[tokio::test]
async fn test_compat_tasks_rpc_lifecycle() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg.clone(), rpc, lsp);

    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let events_clone = events.clone();
    bridge.task_manager().on_task_event(move |ev| {
        events_clone.lock().push(ev.clone());
    });

    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "Task", "id": "task_cargo", "selector": "cargo" }),
        )
        .await
        .unwrap();
    assert_eq!(
        reg.get_providers_for_language(ProviderType::Task, "cargo")
            .len(),
        1
    );

    bridge
        .handle_rpc_message(
            "tasks.registerProvider",
            &json!({ "id": "task_cargo", "type": "cargo" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "tasks.executeTask",
            &json!({
                "id": "exec_1",
                "task": {
                    "name": "cargo test",
                    "source": "cargo",
                    "definition": { "type": "cargo", "command": "test" }
                }
            }),
        )
        .await
        .unwrap();
    assert_eq!(bridge.task_manager().list_active_executions().len(), 1);

    bridge
        .handle_rpc_message(
            "tasks.reportTaskProcessStart",
            &json!({ "id": "exec_1", "processId": 9001 }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "tasks.reportTaskProcessEnd",
            &json!({ "id": "exec_1", "exitCode": 0 }),
        )
        .await
        .unwrap();
    assert_eq!(
        bridge.task_manager().get_execution("exec_1").unwrap().exit_code,
        Some(0)
    );
    assert!(bridge.task_manager().list_active_executions().is_empty());

    bridge
        .handle_rpc_message(
            "tasks.executeTask",
            &json!({ "id": "exec_2", "task": { "name": "watch", "source": "cargo" } }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message("tasks.terminateTask", &json!({ "id": "exec_2" }))
        .await
        .unwrap();
    assert_eq!(
        bridge.task_manager().get_execution("exec_2").unwrap().status,
        "terminated"
    );

    bridge
        .handle_rpc_message("tasks.unregisterProvider", &json!({ "id": "task_cargo" }))
        .await
        .unwrap();
    assert!(bridge.task_manager().list_providers().is_empty());
    assert_eq!(events.lock().len(), 7);
}

#[tokio::test]
async fn test_compat_tests_rpc_lifecycle() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg.clone(), rpc, lsp);

    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let events_clone = events.clone();
    bridge.test_manager().on_test_event(move |ev| {
        events_clone.lock().push(ev.clone());
    });

    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TestController", "id": "rust-tests", "selector": "rust-tests" }),
        )
        .await
        .unwrap();
    assert_eq!(
        reg.get_providers_for_language(ProviderType::TestController, "rust-tests")
            .len(),
        1
    );

    bridge
        .handle_rpc_message(
            "tests.createController",
            &json!({ "id": "rust-tests", "label": "Rust Tests" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({
                "controllerId": "rust-tests",
                "items": [{ "id": "suite_1", "label": "Suite 1", "children": [{ "id": "t1", "label": "test_1" }] }]
            }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "tests.createRunProfile",
            &json!({
                "controllerId": "rust-tests",
                "id": "profile_1",
                "label": "Run",
                "kind": 1,
                "isDefault": true
            }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "tests.createTestRun",
            &json!({ "runId": "run_1", "controllerId": "rust-tests", "name": "CI Run" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "tests.updateTestState",
            &json!({ "runId": "run_1", "testId": "t1", "state": "passed", "duration": 22 }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "tests.appendRunOutput",
            &json!({ "runId": "run_1", "output": "1 passed\n" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message("tests.endTestRun", &json!({ "runId": "run_1" }))
        .await
        .unwrap();

    let run = bridge.test_manager().get_run("run_1").unwrap();
    assert!(run.is_ended);
    assert_eq!(run.results["t1"].state, "passed");
    assert_eq!(run.output, "1 passed\n");

    bridge
        .handle_rpc_message("tests.disposeController", &json!({ "id": "rust-tests" }))
        .await
        .unwrap();
    assert!(bridge.test_manager().list_controllers().is_empty());
    let test_err = bridge
        .handle_rpc_message(
            "tests.updateItems",
            &json!({ "controllerId": "rust-tests", "items": [] }),
        )
        .await
        .unwrap();
    assert_eq!(test_err["error"]["code"], "INVALID_STATE");
    assert_eq!(
        bridge
            .handle_rpc_message_result(
                "tests.updateItems",
                &json!({ "controllerId": "rust-tests", "items": [] }),
            )
            .await
            .unwrap_err()
            .code,
        "INVALID_STATE"
    );
    assert_eq!(events.lock().len(), 8);
}

#[tokio::test]
async fn test_compat_authentication_rpc_lifecycle() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(reg.clone(), rpc, lsp);

    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let events_clone = events.clone();
    bridge.auth_manager().on_auth_event(move |ev| {
        events_clone.lock().push(ev.clone());
    });

    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "Authentication", "id": "github", "selector": "github" }),
        )
        .await
        .unwrap();
    assert_eq!(
        reg.get_providers_for_language(ProviderType::Authentication, "github")
            .len(),
        1
    );

    bridge
        .handle_rpc_message(
            "authentication.registerProvider",
            &json!({ "id": "github", "label": "GitHub", "supportsMultipleAccounts": false }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "authentication.didChangeSessions",
            &json!({
                "providerId": "github",
                "added": [{
                    "id": "sess_1",
                    "accessToken": "tok_xyz",
                    "account": { "id": "u1", "label": "dev" },
                    "scopes": ["repo", "user"]
                }],
                "removed": [],
                "changed": []
            }),
        )
        .await
        .unwrap();

    let sess = bridge
        .handle_rpc_message(
            "authentication.getSession",
            &json!({ "providerId": "github", "scopes": ["repo"] }),
        )
        .await
        .unwrap();
    assert_eq!(sess["id"], "sess_1");

    let accounts = bridge
        .handle_rpc_message(
            "authentication.getAccounts",
            &json!({ "providerId": "github" }),
        )
        .await
        .unwrap();
    assert_eq!(accounts.as_array().unwrap().len(), 1);

    bridge
        .handle_rpc_message("authentication.unregisterProvider", &json!({ "id": "github" }))
        .await
        .unwrap();
    assert!(bridge.auth_manager().list_providers().is_empty());
    assert_eq!(events.lock().len(), 3);
}

#[tokio::test]
async fn test_compat_m2_providers_rpc_roundtrip() {
    let reg = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = Arc::new(LanguageFeatureBridge::new(reg, rpc.clone(), lsp));

    // 1. SCM provideOriginalResource (id = 1)
    let rpc1 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc1.handle_incoming_response(1, Some(json!("git:///workspace/main.rs")), None);
    });
    let orig = bridge
        .request_scm_original_resource("git", "file:///workspace/main.rs")
        .await
        .unwrap();
    assert_eq!(orig.as_deref(), Some("git:///workspace/main.rs"));

    // 2. SCM acceptInput (id = 2)
    let rpc2 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc2.handle_incoming_response(2, Some(json!({ "accepted": true })), None);
    });
    let accepted = bridge
        .request_scm_accept_input("git", Some("commit msg"))
        .await
        .unwrap();
    assert_eq!(accepted["accepted"], true);

    // 3. Debug provideDebugConfigurations (id = 3)
    let rpc3 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc3.handle_incoming_response(
            3,
            Some(json!([{ "type": "lldb", "name": "Launch", "request": "launch" }])),
            None,
        );
    });
    let cfgs = bridge
        .request_debug_configurations("lldb", None)
        .await
        .unwrap();
    assert_eq!(cfgs.len(), 1);

    // 4. Debug resolveDebugConfiguration (id = 4)
    let rpc4 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc4.handle_incoming_response(
            4,
            Some(json!({ "type": "lldb", "name": "Launch", "stopOnEntry": true })),
            None,
        );
    });
    let resolved_cfg = bridge
        .request_resolve_debug_configuration("lldb", None, json!({ "type": "lldb" }))
        .await
        .unwrap();
    assert_eq!(resolved_cfg["stopOnEntry"], true);

    // 5. Debug createDebugAdapterDescriptor (id = 5)
    let rpc5 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc5.handle_incoming_response(
            5,
            Some(json!({ "type": "executable", "command": "lldb-vscode" })),
            None,
        );
    });
    let desc = bridge
        .request_debug_adapter_descriptor("lldb", json!({ "id": "s1" }))
        .await
        .unwrap();
    assert_eq!(desc["command"], "lldb-vscode");

    // 6. Tasks provideTasks (id = 6)
    let rpc6 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc6.handle_incoming_response(6, Some(json!([{ "name": "cargo build" }])), None);
    });
    let tasks = bridge.request_provide_tasks("cargo").await.unwrap();
    assert_eq!(tasks.len(), 1);

    // 7. Tasks resolveTask (id = 7)
    let rpc7 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc7.handle_incoming_response(7, Some(json!({ "name": "cargo build", "resolved": true })), None);
    });
    let resolved_task = bridge
        .request_resolve_task("cargo", json!({ "name": "cargo build" }))
        .await
        .unwrap();
    assert_eq!(resolved_task["resolved"], true);

    // 8. Tests resolveChildren (id = 8)
    let rpc8 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc8.handle_incoming_response(8, Some(json!({ "resolved": true })), None);
    });
    let resolved_children = bridge
        .request_resolve_test_children("rust-tests", Some("suite_1"))
        .await
        .unwrap();
    assert_eq!(resolved_children["resolved"], true);

    // 9. Tests runTests (id = 9)
    let rpc9 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc9.handle_incoming_response(9, Some(json!({ "ran": 3 })), None);
    });
    let run_res = bridge
        .request_run_tests("rust-tests", json!({ "profileId": "profile_1" }))
        .await
        .unwrap();
    assert_eq!(run_res["ran"], 3);

    // 10. Authentication getSessions (id = 10)
    let rpc10 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc10.handle_incoming_response(
            10,
            Some(json!([{
                "id": "s1",
                "providerId": "github",
                "accessToken": "tok_1",
                "account": { "id": "u1", "label": "dev" },
                "scopes": ["repo"]
            }])),
            None,
        );
    });
    let sessions = bridge
        .request_auth_sessions("github", &["repo".to_string()])
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "s1");

    // 11. Authentication createSession (id = 11)
    let rpc11 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc11.handle_incoming_response(
            11,
            Some(json!({
                "id": "s2",
                "providerId": "github",
                "accessToken": "tok_2",
                "account": { "id": "u1", "label": "dev" },
                "scopes": ["repo"]
            })),
            None,
        );
    });
    let created_sess = bridge
        .request_create_auth_session("github", &["repo".to_string()])
        .await
        .unwrap();
    assert_eq!(created_sess.id, "s2");

    // 12. Authentication removeSession (id = 12)
    let rpc12 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc12.handle_incoming_response(12, Some(json!(true)), None);
    });
    let removed = bridge
        .request_remove_auth_session("github", "s2")
        .await
        .unwrap();
    assert!(removed);
}

#[test]
fn test_compat_m2_js_api_export_surface() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let types_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/types.js")).unwrap();
    let scm_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/scm.js")).unwrap();
    let debug_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/debug.js")).unwrap();
    let tasks_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/tasks.js")).unwrap();
    let tests_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/tests.js")).unwrap();
    let auth_js = std::fs::read_to_string(
        repo_root.join("ext-host-js/vscode-api/authentication.js"),
    )
    .unwrap();
    let index_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/index.js")).unwrap();
    let rpc_bridge_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/rpc-bridge.js")).unwrap();
    let test_js = std::fs::read_to_string(repo_root.join("ext-host-js/test.js")).unwrap();

    for sym in ["class EventEmitter", "get event()", "fire(", "dispose(", "EventEmitter,"] {
        assert!(types_js.contains(sym), "types.js missing {}", sym);
    }

    for sym in [
        "SourceControlInputBoxValidationType",
        "SourceControlInputBox",
        "SourceControlResourceGroup",
        "SourceControl",
        "createSourceControl",
        "onDidChangeActiveSourceControl",
        "scm.createSourceControl",
        "scm.createResourceGroup",
    ] {
        assert!(scm_js.contains(sym), "scm.js missing {}", sym);
    }

    for sym in [
        "DebugConsoleMode",
        "DebugConfigurationProviderTriggerKind",
        "Breakpoint",
        "SourceBreakpoint",
        "FunctionBreakpoint",
        "DebugAdapterExecutable",
        "DebugAdapterServer",
        "DebugAdapterNamedPipeServer",
        "DebugAdapterInlineImplementation",
        "activeDebugSession",
        "activeDebugConsole",
        "startDebugging",
        "stopDebugging",
        "registerDebugConfigurationProvider",
        "registerDebugAdapterDescriptorFactory",
        "registerDebugAdapterTrackerFactory",
        "addBreakpoints",
        "removeBreakpoints",
        "asDebugSourceUri",
        "onDidStartDebugSession",
        "onDidTerminateDebugSession",
        "onDidChangeActiveDebugSession",
        "onDidReceiveDebugSessionCustomEvent",
        "onDidChangeBreakpoints",
    ] {
        assert!(debug_js.contains(sym), "debug.js missing {}", sym);
    }

    for sym in [
        "TaskScope",
        "TaskRevealKind",
        "TaskPanelKind",
        "TaskGroup",
        "ProcessExecution",
        "ShellExecution",
        "CustomExecution",
        "Task",
        "registerTaskProvider",
        "fetchTasks",
        "executeTask",
        "taskExecutions",
        "onDidStartTask",
        "onDidEndTask",
        "onDidStartTaskProcess",
        "onDidEndTaskProcess",
    ] {
        assert!(tasks_js.contains(sym), "tasks.js missing {}", sym);
    }

    for sym in [
        "TestRunProfileKind",
        "TestMessage",
        "TestTag",
        "TestRunRequest",
        "TestItem",
        "TestItemCollection",
        "TestRun",
        "TestRunProfile",
        "TestController",
        "createTestController",
        "createRunProfile",
        "createTestRun",
        "createTestItem",
    ] {
        assert!(tests_js.contains(sym), "tests.js missing {}", sym);
    }

    for sym in [
        "getSession",
        "getAccounts",
        "registerAuthenticationProvider",
        "onDidChangeSessions",
    ] {
        assert!(auth_js.contains(sym), "authentication.js missing {}", sym);
    }

    for exported in [
        "require('./scm')",
        "require('./debug')",
        "require('./tasks')",
        "require('./tests')",
        "require('./authentication')",
        "scm,",
        "debug,",
        "tasks,",
        "tests,",
        "authentication,",
        "SourceControl,",
        "DebugConfigurationProviderTriggerKind,",
        "TaskGroup,",
        "TestController,",
    ] {
        assert!(index_js.contains(exported), "index.js missing {}", exported);
    }

    for handler in [
        "scm.provideOriginalResource",
        "scm.acceptInput",
        "debug.provideDebugConfigurations",
        "debug.resolveDebugConfiguration",
        "debug.resolveDebugConfigurationWithSubstitutedVariables",
        "debug.createDebugAdapterDescriptor",
        "tasks.provideTasks",
        "tasks.resolveTask",
        "tests.resolveChildren",
        "tests.refreshTests",
        "tests.runTests",
        "authentication.getSessions",
        "authentication.createSession",
        "authentication.removeSession",
    ] {
        assert!(
            rpc_bridge_js.contains(handler),
            "rpc-bridge.js missing handler {}",
            handler
        );
    }

    for marker in [
        "vscode.EventEmitter",
        "vscode.scm",
        "vscode.debug",
        "vscode.tasks",
        "vscode.tests",
        "vscode.authentication",
    ] {
        assert!(test_js.contains(marker), "test.js missing {}", marker);
    }

    for (name, src) in [
        ("types.js", &types_js),
        ("scm.js", &scm_js),
        ("debug.js", &debug_js),
        ("tasks.js", &tasks_js),
        ("tests.js", &tests_js),
        ("authentication.js", &auth_js),
        ("index.js", &index_js),
        ("rpc-bridge.js", &rpc_bridge_js),
        ("test.js", &test_js),
    ] {
        assert_js_delimiters_balanced(name, src);
    }
}

#[tokio::test]
async fn test_compat_custom_editor_and_webview_rpc_lifecycle() {
    let manifest_json = r#"{
        "name": "cat-scratch-custom-editor",
        "publisher": "rustcode",
        "version": "1.0.0",
        "contributes": {
            "customEditors": [
                {
                    "viewType": "catEdit.catScratch",
                    "displayName": "Cat Scratch",
                    "selector": [
                        { "filenamePattern": "*.cscratch" }
                    ],
                    "priority": "default"
                }
            ]
        }
    }"#;
    let manifest = ExtensionLoader::parse_manifest(manifest_json).unwrap();
    let ce_contrib = &manifest.contributes.unwrap().custom_editors[0];
    assert_eq!(ce_contrib.view_type, "catEdit.catScratch");
    assert!(ce_contrib.matches_path("file:///workspace/demo.cscratch"));
    assert!(!ce_contrib.matches_path("file:///workspace/main.rs"));

    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(registry.clone(), rpc, lsp);

    let reg_res = bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &serde_json::json!({
                "type": "CustomEditor",
                "id": "catEdit.catScratch",
                "selector": "catEdit.catScratch"
            }),
        )
        .await
        .unwrap();
    assert_eq!(reg_res["registered"], true);
    assert_eq!(
        registry
            .get_providers_for_language(
                rustcode_ext_host::ProviderType::CustomEditor,
                "catEdit.catScratch"
            )
            .len(),
        1
    );

    let provider_res = bridge
        .handle_rpc_message(
            "window.registerCustomEditorProvider",
            &serde_json::json!({
                "viewType": "catEdit.catScratch",
                "options": {
                    "webviewOptions": { "enableScripts": true },
                    "supportsMultipleEditorsPerDocument": true
                }
            }),
        )
        .await
        .unwrap();
    assert_eq!(provider_res["viewType"], "catEdit.catScratch");
    assert_eq!(bridge.custom_editor_manager.list_providers().len(), 1);

    let session_res = bridge
        .handle_rpc_message(
            "window.openCustomEditor",
            &serde_json::json!({
                "id": "ce_panel_1",
                "viewType": "catEdit.catScratch",
                "uri": "file:///workspace/demo.cscratch",
                "title": "demo.cscratch"
            }),
        )
        .await
        .unwrap();
    assert_eq!(session_res["panelId"], "ce_panel_1");
    assert!(bridge.webview_manager.get_panel("ce_panel_1").is_some());

    assert!(bridge
        .handle_rpc_message(
            "window.createWebviewPanel",
            &serde_json::json!({
                "id": "wv_direct_1",
                "viewType": "catCoding",
                "title": "Cat Coding",
                "options": { "enableScripts": true }
            })
        )
        .await
        .is_some());

    assert_eq!(
        bridge
            .handle_rpc_message(
                "window.setWebviewHtml",
                &serde_json::json!({ "id": "ce_panel_1", "html": "<h1>Scratch</h1>" })
            )
            .await
            .unwrap(),
        serde_json::json!(true)
    );
    assert_eq!(
        bridge
            .webview_manager
            .get_panel("ce_panel_1")
            .unwrap()
            .html,
        "<h1>Scratch</h1>"
    );

    assert_eq!(
        bridge
            .handle_rpc_message(
                "window.postMessageToWebview",
                &serde_json::json!({ "id": "ce_panel_1", "message": { "type": "sync" } })
            )
            .await
            .unwrap(),
        serde_json::json!(true)
    );

    let revealed = bridge
        .handle_rpc_message(
            "window.revealWebviewPanel",
            &serde_json::json!({ "id": "ce_panel_1", "viewColumn": 2 }),
        )
        .await
        .unwrap();
    assert_eq!(revealed["revealed"], true);

    assert_eq!(
        bridge
            .handle_rpc_message(
                "window.disposeWebviewPanel",
                &serde_json::json!({ "id": "wv_direct_1" })
            )
            .await
            .unwrap(),
        serde_json::json!(true)
    );
    assert!(bridge
        .webview_manager
        .get_panel("wv_direct_1")
        .unwrap()
        .is_disposed);

    let unreg_ce = bridge
        .handle_rpc_message(
            "window.unregisterCustomEditorProvider",
            &serde_json::json!({ "viewType": "catEdit.catScratch" }),
        )
        .await
        .unwrap();
    assert_eq!(unreg_ce["unregistered"], true);

    let unreg_provider = bridge
        .handle_rpc_message(
            "extHost.unregisterProvider",
            &serde_json::json!({ "id": "catEdit.catScratch" }),
        )
        .await
        .unwrap();
    assert_eq!(unreg_provider["unregistered"], true);
}

#[tokio::test]
async fn test_compat_custom_editor_rpc_roundtrip() {
    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(registry, rpc.clone(), lsp);

    bridge.webview_manager.create_panel(
        "ce_panel_rt",
        "catEdit.catScratch",
        "demo.cscratch",
        rustcode_ext_host::WebviewOptions {
            enable_scripts: true,
            local_resource_roots: vec![],
        },
    );

    let rpc1 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc1.handle_incoming_response(
            1,
            Some(serde_json::json!({ "uri": "file:///demo.cscratch", "opened": true })),
            None,
        );
    });
    let opened = bridge
        .request_open_custom_document("catEdit.catScratch", "file:///demo.cscratch", None)
        .await
        .unwrap();
    assert_eq!(opened["opened"], true);

    let rpc2 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc2.handle_incoming_response(
            2,
            Some(serde_json::json!({
                "resolved": true,
                "panelId": "ce_panel_rt",
                "viewType": "catEdit.catScratch",
                "html": "<div id=\"custom\">Resolved Body</div>"
            })),
            None,
        );
    });
    let resolved = bridge
        .request_resolve_custom_editor(
            "catEdit.catScratch",
            "file:///demo.cscratch",
            "ce_panel_rt",
            "demo.cscratch",
            Some("raw text"),
        )
        .await
        .unwrap();
    assert_eq!(resolved["resolved"], true);
    assert_eq!(
        bridge
            .webview_manager
            .get_panel("ce_panel_rt")
            .unwrap()
            .html,
        "<div id=\"custom\">Resolved Body</div>"
    );

    let rpc3 = rpc.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        rpc3.handle_incoming_response(3, Some(serde_json::json!({ "saved": true })), None);
    });
    let saved = bridge
        .request_save_custom_document("catEdit.catScratch", "file:///demo.cscratch")
        .await
        .unwrap();
    assert_eq!(saved["saved"], true);
}

#[test]
fn test_compat_custom_editor_js_api_export_surface() {
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
    let rpc_bridge_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/rpc-bridge.js")).unwrap();
    let test_js = std::fs::read_to_string(repo_root.join("ext-host-js/test.js")).unwrap();

    for sym in [
        "registerCustomEditorProvider(",
        "window.registerCustomEditorProvider",
        "window.unregisterCustomEditorProvider",
        "Array.isArray(provider)",
        "viewType.trim()",
        "resolveCustomTextEditor",
        "resolveCustomEditor",
        "openCustomDocument",
        "saveCustomDocument",
        "saveCustomDocumentAs",
        "revertCustomDocument",
        "backupCustomDocument",
        "createWebviewPanelProxy",
    ] {
        assert!(
            window_js.contains(sym),
            "window.js missing custom editor symbol {}",
            sym
        );
    }

    for handler in [
        "createWebviewPanelProxy(",
        "let _disposed = false;",
        "window.openCustomDocument",
        "window.resolveCustomEditor",
        "CustomEditor.resolveCustomEditor",
        "CustomEditor.resolveCustomTextEditor",
        "window.saveCustomDocument",
        "window.saveCustomDocumentAs",
        "window.revertCustomDocument",
        "window.backupCustomDocument",
        "window.closeCustomDocument",
        "CustomEditor.closeCustomDocument",
    ] {
        assert!(
            rpc_bridge_js.contains(handler),
            "rpc-bridge.js missing handler {}",
            handler
        );
    }

    assert!(test_js.contains("registerCustomEditorProvider"));
    assert!(test_js.contains("window.resolveCustomEditor"));

    for (name, src) in [
        ("window.js", &window_js),
        ("index.js", &index_js),
        ("rpc-bridge.js", &rpc_bridge_js),
        ("test.js", &test_js),
    ] {
        assert_js_delimiters_balanced(name, src);
    }
}

#[tokio::test]
async fn test_compat_m1_window_notifications_quickpick_and_inputbox() {
    use rustcode_ext_host::{ExtPromptKind, NotificationSeverity};

    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(registry, rpc, lsp);

    // 1. Headless non-blocking default + MessageOptions normalization
    let res = bridge
        .handle_rpc_message(
            "window.showInformationMessage",
            &json!({
                "message": "Workspace loaded",
                "items": [{ "modal": true, "detail": "Ready" }, "Configure"]
            }),
        )
        .await
        .unwrap();
    assert_eq!(res, serde_json::Value::Null);
    assert_eq!(bridge.window_manager().revision(), 1);
    let notifs = bridge.window_manager().list_notifications();
    assert_eq!(notifs.len(), 1);
    assert_eq!(notifs[0].severity, NotificationSeverity::Info);
    assert!(notifs[0].modal);
    assert_eq!(notifs[0].items.len(), 1);

    // 2. Sync responders for Warning/Error/QuickPick/InputBox
    bridge.window_manager().set_notification_responder(|n| {
        n.items.first().map(|item| item.raw.clone())
    });
    let warn_res = bridge
        .handle_rpc_message(
            "window.showWarningMessage",
            &json!({ "message": "Slow task", "items": ["Retry"] }),
        )
        .await
        .unwrap();
    assert_eq!(warn_res, json!("Retry"));

    let err_res = bridge
        .handle_rpc_message(
            "window.showErrorMessage",
            &json!({ "message": "Disk full", "items": [{ "title": "Cleanup" }] }),
        )
        .await
        .unwrap();
    assert_eq!(err_res, json!({ "title": "Cleanup" }));

    bridge.window_manager().set_prompt_responder(|p| match &p.kind {
        ExtPromptKind::QuickPick { can_pick_many, .. } => {
            if *can_pick_many {
                Some(json!(["One", "Two"]))
            } else {
                Some(json!("Two"))
            }
        }
        ExtPromptKind::InputBox { .. } => Some(json!("my-input")),
    });

    let qp_single = bridge
        .handle_rpc_message(
            "window.showQuickPick",
            &json!({ "items": ["One", "Two"], "options": {} }),
        )
        .await
        .unwrap();
    assert_eq!(qp_single, json!({ "label": "Two" }));

    let qp_many = bridge
        .handle_rpc_message(
            "window.showQuickPick",
            &json!({ "items": ["One", "Two"], "options": { "canPickMany": true } }),
        )
        .await
        .unwrap();
    assert_eq!(qp_many, json!([{ "label": "One" }, { "label": "Two" }]));

    let ib = bridge
        .handle_rpc_message(
            "window.showInputBox",
            &json!({ "options": { "prompt": "Enter name" } }),
        )
        .await
        .unwrap();
    assert_eq!(ib, json!("my-input"));
}

#[tokio::test]
async fn test_compat_m1_tree_data_provider_alias_and_numeric_collapsible_state() {
    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = Arc::new(LanguageFeatureBridge::new(registry.clone(), rpc.clone(), lsp));

    let reg_res = bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({
                "type": "TreeDataProvider",
                "id": "outlineView",
                "selector": "*"
            }),
        )
        .await
        .unwrap();
    assert_eq!(reg_res["registered"], true);
    assert!(registry.has_provider(ProviderType::TreeData));
    assert!(registry.has_provider_with_id(ProviderType::TreeData, "outlineView"));
    assert_eq!(registry.get_providers(ProviderType::TreeData).len(), 1);

    let b = bridge.clone();
    let task = tokio::spawn(async move { b.request_tree_children("outlineView", None).await });
    tokio::task::yield_now().await;

    rpc.handle_incoming_response(
        1,
        Some(json!([
            {
                "id": "src",
                "label": "src",
                "collapsibleState": 1,
                "contextValue": "folder"
            },
            {
                "id": "lib",
                "label": "lib.rs",
                "collapsibleState": 0,
                "contextValue": "file"
            }
        ])),
        None,
    );

    let items = task.await.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].collapsible_state, TreeItemCollapsibleState::Collapsed);
    assert_eq!(items[0].context_value.as_deref(), Some("folder"));
    assert_eq!(items[1].collapsible_state, TreeItemCollapsibleState::None);
}

#[tokio::test]
async fn test_compat_m1_workspace_configuration_and_open_text_document() {
    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let config = rustcode_platform::ConfigurationService::new();
    let folder = PathBuf::from("/workspace/subpkg");
    let scoped_file = PathBuf::from("/workspace/subpkg/src/main.rs");
    config
        .update(
            "editor.tabSize",
            json!(2),
            rustcode_platform::ConfigurationTarget::WorkspaceFolder(folder.clone()),
        )
        .unwrap();

    let bridge = LanguageFeatureBridge::new(registry, rpc, lsp).with_configuration_service(config);

    // 1. getConfiguration empty section returns full merged config
    let all = bridge
        .handle_rpc_message("workspace.getConfiguration", &json!({}))
        .await
        .unwrap();
    assert_eq!(all["editor"]["fontSize"], json!(14));

    // 2. Scoped getConfiguration merges folder overrides while keeping sibling defaults
    let scoped_editor = bridge
        .handle_rpc_message(
            "workspace.getConfiguration",
            &json!({
                "section": "editor",
                "scope": format!("file://{}", scoped_file.to_string_lossy())
            }),
        )
        .await
        .unwrap();
    assert_eq!(scoped_editor["tabSize"], json!(2));
    assert_eq!(scoped_editor["minimap"]["enabled"], json!(true));

    // 3. updateConfiguration targets 1 (User), 2 (Workspace), bool, and reset (no value)
    bridge
        .handle_rpc_message(
            "workspace.updateConfiguration",
            &json!({ "key": "editor.fontSize", "value": 18, "target": 1 }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message(
            "workspace.updateConfiguration",
            &json!({ "key": "editor.fontSize", "value": 22, "target": 2 }),
        )
        .await
        .unwrap();

    let inspected = bridge
        .handle_rpc_message(
            "workspace.inspectConfiguration",
            &json!({ "key": "editor.fontSize" }),
        )
        .await
        .unwrap();
    assert_eq!(inspected["defaultValue"], json!(14));
    assert_eq!(inspected["globalValue"], json!(18));
    assert_eq!(inspected["workspaceValue"], json!(22));

    // Reset workspace override (missing value) -> falls back to globalValue (18)
    bridge
        .handle_rpc_message(
            "workspace.updateConfiguration",
            &json!({ "key": "editor.fontSize", "target": 2 }),
        )
        .await
        .unwrap();
    let after_reset = bridge
        .handle_rpc_message(
            "workspace.getConfiguration",
            &json!({ "section": "editor.fontSize" }),
        )
        .await
        .unwrap();
    assert_eq!(after_reset, json!(18));

    // 4. openTextDocument with %20 URI decoding, dirty snapshot, untitled, and missing file
    let tmp = std::env::temp_dir().join(format!(
        "rustcode_ws_open_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let spaced_file = tmp.join("my file.rs");
    std::fs::write(&spaced_file, "fn main() {}\n").unwrap();
    let encoded_uri = format!(
        "file://{}",
        spaced_file.to_string_lossy().replace(' ', "%20")
    );

    let doc = bridge
        .handle_rpc_message("workspace.openTextDocument", &json!({ "uri": encoded_uri }))
        .await
        .unwrap();
    assert_eq!(doc["languageId"], "rust");
    assert_eq!(doc["text"], "fn main() {}\n");
    assert_eq!(doc["lineCount"], 2);
    assert_eq!(doc["isDirty"], false);

    // Dirty open snapshot takes precedence over disk
    let raw_file_uri = format!("file://{}", spaced_file.to_string_lossy());
    bridge
        .did_change_document(&raw_file_uri, "rust", 3, "fn main() { println!(\"hi\"); }")
        .await;
    let dirty_doc = bridge
        .handle_rpc_message("workspace.openTextDocument", &json!({ "uri": encoded_uri }))
        .await
        .unwrap();
    assert_eq!(dirty_doc["isDirty"], true);
    assert_eq!(dirty_doc["version"], 3);
    assert_eq!(dirty_doc["text"], "fn main() { println!(\"hi\"); }");

    // Untitled document
    let untitled = bridge
        .handle_rpc_message(
            "workspace.openTextDocument",
            &json!({ "language": "python", "content": "print(1)\nprint(2)" }),
        )
        .await
        .unwrap();
    assert_eq!(untitled["isUntitled"], true);
    assert_eq!(untitled["languageId"], "python");
    assert_eq!(untitled["lineCount"], 2);

    // Missing file returns structured ENOENT error
    let open_err = bridge
        .handle_rpc_message(
            "workspace.openTextDocument",
            &json!({ "uri": format!("file://{}/does_not_exist.rs", tmp.to_string_lossy()) }),
        )
        .await
        .unwrap();
    assert_eq!(open_err["error"]["code"], "ENOENT");
    assert_eq!(
        bridge
            .handle_rpc_message_result(
                "workspace.openTextDocument",
                &json!({ "uri": format!("file://{}/does_not_exist.rs", tmp.to_string_lossy()) }),
            )
            .await
            .unwrap_err()
            .code,
        "ENOENT"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn test_compat_m1_commands_rpc_lifecycle() {
    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let cmd_reg = rustcode_platform::CommandRegistry::new();
    let _rust_disp = cmd_reg.register("rust.echo", |args| {
        Ok(json!({ "echo": args.unwrap_or(serde_json::Value::Null) }))
    });
    let _internal_disp = cmd_reg.register("_internal.secret", |_| Ok(json!(true)));

    let bridge = Arc::new(
        LanguageFeatureBridge::new(registry, rpc.clone(), lsp).with_command_registry(cmd_reg.clone()),
    );

    // Register & re-register JS command safely (old disposable must NOT delete new command)
    bridge
        .handle_rpc_message("commands.registerCommand", &json!({ "command": "ext.sayHi" }))
        .await
        .unwrap();
    bridge
        .handle_rpc_message("commands.registerCommand", &json!({ "command": "ext.sayHi" }))
        .await
        .unwrap();
    assert!(cmd_reg.has_command("ext.sayHi"));

    // getCommands respects filterInternal and sorting
    let filtered = bridge
        .handle_rpc_message("commands.getCommands", &json!({ "filterInternal": true }))
        .await
        .unwrap();
    assert_eq!(filtered, json!(["ext.sayHi", "rust.echo"]));

    let all = bridge
        .handle_rpc_message("commands.getCommands", &json!({ "filterInternal": false }))
        .await
        .unwrap();
    assert_eq!(all, json!(["_internal.secret", "ext.sayHi", "rust.echo"]));

    // Inbound executeCommand runs Rust command
    let rust_res = bridge
        .handle_rpc_message(
            "commands.executeCommand",
            &json!({ "command": "rust.echo", "args": ["abc"] }),
        )
        .await
        .unwrap();
    assert_eq!(rust_res, json!({ "echo": ["abc"] }));

    // Inbound executeCommand recursion guard blocks bouncing JS command back to JS
    let rec_err = bridge
        .handle_rpc_message(
            "commands.executeCommand",
            &json!({ "command": "ext.sayHi", "args": [] }),
        )
        .await
        .unwrap();
    assert_eq!(rec_err["error"]["code"], "COMMAND_RECURSION");
    assert_eq!(
        bridge
            .handle_rpc_message_result(
                "commands.executeCommand",
                &json!({ "command": "ext.sayHi", "args": [] }),
            )
            .await
            .unwrap_err()
            .code,
        "COMMAND_RECURSION"
    );

    // Async outbound request_execute_command dispatches to JS and receives response
    let b = bridge.clone();
    let exec_task =
        tokio::spawn(async move { b.request_execute_command("ext.sayHi", vec![json!("M1")]).await });
    tokio::task::yield_now().await;
    rpc.handle_incoming_response(1, Some(json!("hi M1")), None);
    let exec_res = exec_task.await.unwrap().unwrap();
    assert_eq!(exec_res, json!("hi M1"));

    // Unregister removes from both registry and js_commands
    let unreg = bridge
        .handle_rpc_message("commands.unregisterCommand", &json!({ "command": "ext.sayHi" }))
        .await
        .unwrap();
    assert_eq!(unreg["unregistered"], true);
    assert!(!cmd_reg.has_command("ext.sayHi"));
}

#[tokio::test]
async fn test_compat_m1_env_and_extensions_rpc_lifecycle() {
    let dir = fixtures_dir();
    let mut loader = ExtensionLoader::new();
    for ext in [
        "theme_ext",
        "formatter_ext",
        "codeactions_ext",
        "treeview_ext",
        "statusbar_ext",
        "linter_ext",
    ] {
        loader.load_from_dir(dir.join(ext)).unwrap();
    }
    assert!(loader.activate("git-status-ext"));

    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(registry, rpc, lsp).with_extension_loader(&loader);

    let all_exts = bridge
        .handle_rpc_message("extensions.getAll", &json!({}))
        .await
        .unwrap();
    assert_eq!(all_exts.as_array().unwrap().len(), 6);

    // Case-insensitive lookup
    let git_ext = bridge
        .handle_rpc_message(
            "extensions.getExtension",
            &json!({ "id": "RUSTCODE.git-status-ext" }),
        )
        .await
        .unwrap();
    assert_eq!(git_ext["id"], "rustcode.git-status-ext");
    assert_eq!(git_ext["isActive"], true);

    // Clipboard roundtrip
    let wr = bridge
        .handle_rpc_message("env.clipboardWrite", &json!({ "value": "m1-clipboard" }))
        .await
        .unwrap();
    assert_eq!(wr["written"], true);
    let rd = bridge
        .handle_rpc_message("env.clipboardRead", &json!({}))
        .await
        .unwrap();
    assert_eq!(rd, json!("m1-clipboard"));

    // openExternal allowlist offline validation
    assert_eq!(
        bridge
            .handle_rpc_message("env.openExternal", &json!({ "uri": "https://rustcode.dev" }))
            .await
            .unwrap(),
        json!(true)
    );
    assert_eq!(
        bridge
            .handle_rpc_message("env.openExternal", &json!({ "uri": "javascript:alert(1)" }))
            .await
            .unwrap(),
        json!(false)
    );
}

#[tokio::test]
async fn test_compat_m1_node_fs_rpc_lifecycle() {
    let tmp = std::env::temp_dir().join(format!(
        "rustcode_compat_fs_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let fs_svc = Arc::new(rustcode_platform::FileSystemService::new());
    let changed_paths = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let changed_clone = changed_paths.clone();
    let _sub = fs_svc.on_did_change.event().attach(move |ev| {
        changed_clone.lock().push(ev.path.clone());
    });

    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(registry, rpc, lsp).with_file_system_service(fs_svc);

    let sub_dir = tmp.join("a/b");
    let mk = bridge
        .handle_rpc_message(
            "fs.mkdir",
            &json!({ "path": sub_dir.to_string_lossy(), "options": { "recursive": true } }),
        )
        .await
        .unwrap();
    assert_eq!(mk["created"], true);

    let file_path = sub_dir.join("note.txt");
    let wr = bridge
        .handle_rpc_message(
            "fs.writeFile",
            &json!({ "path": file_path.to_string_lossy(), "data": "hello fs" }),
        )
        .await
        .unwrap();
    assert_eq!(wr["written"], true);
    assert_eq!(changed_paths.lock().len(), 1);

    let content = bridge
        .handle_rpc_message(
            "fs.readFile",
            &json!({ "path": file_path.to_string_lossy(), "encoding": "utf8" }),
        )
        .await
        .unwrap();
    assert_eq!(content, json!("hello fs"));

    let entries = bridge
        .handle_rpc_message("fs.readdir", &json!({ "path": sub_dir.to_string_lossy() }))
        .await
        .unwrap();
    assert_eq!(entries, json!(["note.txt"]));

    let st = bridge
        .handle_rpc_message("fs.stat", &json!({ "path": file_path.to_string_lossy() }))
        .await
        .unwrap();
    assert_eq!(st["isFile"], true);
    assert_eq!(st["size"], 8);

    let ul = bridge
        .handle_rpc_message("fs.unlink", &json!({ "path": file_path.to_string_lossy() }))
        .await
        .unwrap();
    assert_eq!(ul["unlinked"], true);

    // Missing file returns structured ENOENT from both handle_rpc_message and handle_fs_rpc
    let read_err = bridge
        .handle_rpc_message(
            "fs.readFile",
            &json!({ "path": file_path.to_string_lossy() }),
        )
        .await
        .unwrap();
    assert_eq!(read_err["error"]["code"], "ENOENT");
    assert_eq!(
        bridge
            .handle_rpc_message_result(
                "fs.readFile",
                &json!({ "path": file_path.to_string_lossy() }),
            )
            .await
            .unwrap_err()
            .code,
        "ENOENT"
    );
    let err = bridge
        .handle_fs_rpc(
            "fs.readFile",
            &json!({ "path": file_path.to_string_lossy() }),
        )
        .await
        .unwrap_err();
    assert!(err.starts_with("ENOENT:"), "unexpected err: {}", err);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_compat_m1_js_api_export_surface() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let types_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/types.js")).unwrap();
    let index_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/index.js")).unwrap();
    let window_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/window.js")).unwrap();
    let workspace_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/workspace.js")).unwrap();
    let commands_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/commands.js")).unwrap();
    let env_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/vscode-api/env.js")).unwrap();
    let fs_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/node-compat/fs.js")).unwrap();
    let rpc_bridge_js =
        std::fs::read_to_string(repo_root.join("ext-host-js/rpc-bridge.js")).unwrap();
    let test_js = std::fs::read_to_string(repo_root.join("ext-host-js/test.js")).unwrap();

    for sym in [
        "TreeItemCollapsibleState",
        "class TreeItem",
        "ConfigurationTarget",
        "EndOfLine",
    ] {
        assert!(types_js.contains(sym), "types.js missing {}", sym);
    }

    for sym in [
        "createTreeView(",
        "isCancellationRequested",
        "showQuickPick(",
        "showInputBox(",
    ] {
        assert!(window_js.contains(sym), "window.js missing {}", sym);
    }

    for sym in [
        "_resolve(section)",
        "inspect(section)",
        "class TextDocumentProxy",
        "workspace.inspectConfiguration",
    ] {
        assert!(workspace_js.contains(sym), "workspace.js missing {}", sym);
    }

    for sym in [
        "registerCommandHandler",
        "unregisterCommandHandler",
        "Array.isArray(res)",
        "if (localCommands.get(command) === handler)",
    ] {
        assert!(commands_js.contains(sym), "commands.js missing {}", sym);
    }

    for sym in [
        "normalizeExtension(",
        "onDidChange:",
        "_setAll(",
        "async refresh()",
        "toLowerCase()",
    ] {
        assert!(env_js.contains(sym), "env.js missing {}", sym);
    }

    for sym in [
        "unwrapResult(",
        "isFile: () => isFileVal",
        "isDirectory: () => isDirVal",
        "isSymbolicLink: () => isSymVal",
        "options === null",
        "Uint8Array.from(res)",
    ] {
        assert!(fs_js.contains(sym), "node-compat/fs.js missing {}", sym);
    }

    for sym in [
        "treeView.getChildren",
        "TreeDataProvider.getChildren",
        "TreeData.getChildren",
        "treeView.getTreeItem",
        "TreeDataProvider.getTreeItem",
        "TreeData.getTreeItem",
        "commands.executeCommand",
        "normalizeTreeItem(",
        "item.iconPath.dark || item.iconPath.light",
        "Number.isFinite(item.collapsibleState)",
    ] {
        assert!(rpc_bridge_js.contains(sym), "rpc-bridge.js missing {}", sym);
    }

    assert!(workspace_js.contains("(this._section && section)"));

    assert!(test_js.contains("TreeItemCollapsibleState"));
    assert!(test_js.contains("treeView.getChildren"));

    for (name, src) in [
        ("types.js", &types_js),
        ("index.js", &index_js),
        ("window.js", &window_js),
        ("workspace.js", &workspace_js),
        ("commands.js", &commands_js),
        ("env.js", &env_js),
        ("node-compat/fs.js", &fs_js),
        ("rpc-bridge.js", &rpc_bridge_js),
        ("test.js", &test_js),
    ] {
        assert_js_delimiters_balanced(name, src);
    }
}

#[tokio::test]
async fn test_compat_m1_iteration2_regressions() {
    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let config = rustcode_platform::ConfigurationService::new();
    let cmd_reg = rustcode_platform::CommandRegistry::new();

    config
        .load_user_settings(r#"{"myExt": {"a": {"other": 2}, "a.b": 1}}"#)
        .unwrap();
    config
        .update(
            "editor.tabSize",
            json!(2),
            rustcode_platform::ConfigurationTarget::WorkspaceFolder(PathBuf::from(
                "/workspace/subpkg",
            )),
        )
        .unwrap();

    let tmp = std::env::temp_dir().join(format!("rustcode_compat_m1_iter2_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let sandbox = tmp.join("sandbox");
    let outside = tmp.join("outside");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    let bridge = Arc::new(
        LanguageFeatureBridge::new(registry, rpc.clone(), lsp)
            .with_configuration_service(config)
            .with_command_registry(cmd_reg.clone())
            .with_fs_root(&sandbox),
    );

    // 1. Mixed nested + dotted sibling keys match between getConfiguration and inspectConfiguration
    let got = bridge
        .handle_rpc_message(
            "workspace.getConfiguration",
            &json!({ "section": "myExt.a.b" }),
        )
        .await
        .unwrap();
    assert_eq!(got, json!(1));
    let insp = bridge
        .handle_rpc_message(
            "workspace.inspectConfiguration",
            &json!({ "key": "myExt.a.b" }),
        )
        .await
        .unwrap();
    assert_eq!(insp["globalValue"], json!(1));

    // 2. { uri: Uri.toJSON() } scope (with `path` and no `fsPath`) resolves folder settings
    let scoped = bridge
        .handle_rpc_message(
            "workspace.getConfiguration",
            &json!({
                "section": "editor.tabSize",
                "scope": {
                    "uri": {
                        "$mid": 1,
                        "scheme": "file",
                        "path": "/workspace/subpkg/src/main.rs"
                    }
                }
            }),
        )
        .await
        .unwrap();
    assert_eq!(scoped, json!(2));

    // 3. openTextDocument matches %20-encoded did_open_document via raw path & untitled: dirty buffers
    let spaced = sandbox.join("hello world.rs");
    std::fs::write(&spaced, "disk_version").unwrap();
    let encoded_uri = format!("file://{}", spaced.to_string_lossy().replace(' ', "%20"));
    bridge
        .did_open_document(&encoded_uri, "rust", 1, "disk_version")
        .await;
    bridge
        .did_change_document(&encoded_uri, "rust", 2, "DIRTY_BUFFER")
        .await;

    let doc_by_path = bridge
        .handle_rpc_message(
            "workspace.openTextDocument",
            &json!({ "path": spaced.to_string_lossy() }),
        )
        .await
        .unwrap();
    assert_eq!(doc_by_path["isDirty"], true);
    assert_eq!(doc_by_path["text"], "DIRTY_BUFFER");

    bridge
        .did_open_document("untitled:Untitled-1", "plaintext", 1, "")
        .await;
    bridge
        .did_change_document("untitled:Untitled-1", "plaintext", 2, "unsaved untitled text")
        .await;
    let doc_untitled = bridge
        .handle_rpc_message(
            "workspace.openTextDocument",
            &json!({ "uri": "untitled://Untitled-1" }),
        )
        .await
        .unwrap();
    assert_eq!(doc_untitled["isUntitled"], true);
    assert_eq!(doc_untitled["isDirty"], true);
    assert_eq!(doc_untitled["version"], 2);
    assert_eq!(doc_untitled["fileName"], "Untitled-1");
    assert_eq!(doc_untitled["text"], "unsaved untitled text");

    // 4. Sync CommandCallback does NOT leak RpcBridge::pending entries
    bridge
        .handle_rpc_message("commands.registerCommand", &json!({ "command": "ext.syncNoLeak" }))
        .await
        .unwrap();
    let exec_out = cmd_reg.execute("ext.syncNoLeak", None).unwrap();
    assert_eq!(exec_out["dispatched"], true);
    assert_eq!(rpc.pending_count(), 0);

    // 5. env.openExternal rejects leading/trailing control chars & whitespace
    for bad in [
        "https://example.com\r\n",
        "\thttps://example.com",
        "\u{000b}https://example.com",
        " https://example.com ",
    ] {
        assert_eq!(
            bridge
                .handle_rpc_message("env.openExternal", &json!({ "uri": bad }))
                .await
                .unwrap(),
            json!(false)
        );
    }

    // 6. %00 URI bypass rejected with EINVAL & dangling symlink rejected with EACCES
    let nul_err = bridge
        .handle_fs_rpc(
            "fs.readFile",
            &json!({ "path": format!("file://{}/evil%00.txt", sandbox.to_string_lossy()) }),
        )
        .await
        .unwrap_err();
    assert!(nul_err.starts_with("EINVAL:"), "got {}", nul_err);

    #[cfg(unix)]
    {
        let pwned = outside.join("pwned.txt");
        let link = sandbox.join("dangling_link");
        std::os::unix::fs::symlink(&pwned, &link).unwrap();
        let sym_err = bridge
            .handle_fs_rpc(
                "fs.writeFile",
                &json!({ "path": "dangling_link", "data": [112, 119, 110, 101, 100] }),
            )
            .await
            .unwrap_err();
        assert!(sym_err.starts_with("EACCES:"), "got {}", sym_err);
        assert!(!pwned.exists());
    }

    // 7. ExtHostWindowManager post-timeout immutability + abort waiter cleanup
    let wm = bridge.window_manager();
    wm.set_await_responses(true);
    wm.set_response_timeout(Some(std::time::Duration::from_millis(15)));
    let res = bridge
        .handle_rpc_message(
            "window.showInformationMessage",
            &json!({ "message": "Timeout", "items": ["OK"] }),
        )
        .await
        .unwrap();
    assert_eq!(res, serde_json::Value::Null);
    let notif_id = wm.list_notifications()[0].id.clone();
    assert!(!wm.respond_notification(&notif_id, Some(json!("OK"))));

    wm.set_response_timeout(None);
    let bridge_clone = bridge.clone();
    let aborted_task = tokio::spawn(async move {
        let _ = bridge_clone
            .handle_rpc_message(
                "window.showInputBox",
                &json!({ "options": { "prompt": "Aborted" } }),
            )
            .await;
    });
    tokio::task::yield_now().await;
    aborted_task.abort();
    let _ = aborted_task.await;
    assert!(wm.list_prompts().iter().all(|p| p.resolved));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn test_compat_m1_1_rpc_error_propagation_all_namespaces() {
    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(registry, rpc, lsp);

    // Every namespace must return explicit METHOD_NOT_FOUND for unknown methods
    for unknown_method in [
        "fs.unknownOp",
        "commands.unknownOp",
        "env.unknownOp",
        "extensions.unknownOp",
        "window.unknownOp",
        "workspace.unknownOp",
        "notebooks.unknownOp",
        "scm.unknownOp",
        "debug.unknownOp",
        "tasks.unknownOp",
        "tests.unknownOp",
        "authentication.unknownOp",
        "languages.unknownOp",
        "extHost.unknownOp",
        "completely.nonexistent",
    ] {
        let err = bridge
            .handle_rpc_message_result(unknown_method, &json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            err.code, "METHOD_NOT_FOUND",
            "Expected METHOD_NOT_FOUND for {}, got {:?}",
            unknown_method, err
        );

        let wrapped = bridge
            .handle_rpc_message(unknown_method, &json!({}))
            .await
            .unwrap();
        assert_eq!(wrapped["error"]["code"], "METHOD_NOT_FOUND");
    }

    // Invalid params across sub-managers & workspace & commands & extHost
    for (method, params, expected_code) in [
        ("languages.setDiagnostics", json!({}), "INVALID_PARAMS"),
        ("window.setStatusBarItem", json!({}), "INVALID_PARAMS"),
        ("extHost.registerProvider", json!({ "type": "Nope", "id": "x" }), "INVALID_PARAMS"),
        ("extHost.unregisterProvider", json!({}), "INVALID_PARAMS"),
        ("workspace.inspectConfiguration", json!({}), "INVALID_PARAMS"),
        ("workspace.updateConfiguration", json!({ "key": "a.b", "target": 999 }), "INVALID_PARAMS"),
        ("commands.registerCommand", json!({ "command": "  " }), "INVALID_PARAMS"),
        ("commands.executeCommand", json!({ "command": "no.such.cmd" }), "COMMAND_NOT_FOUND"),
    ] {
        let err = bridge
            .handle_rpc_message_result(method, &params)
            .await
            .unwrap_err();
        assert_eq!(
            err.code, expected_code,
            "Unexpected error code for {} ({:?}): {:?}",
            method, params, err
        );
    }
}

#[tokio::test]
async fn test_compat_m1_1_r2_validation_eisdir_and_command_timeout_cleanup() {
    let guard = TempDirGuard::new("rustcode_m1_1_r2");
    let sandbox = guard.path().join("sandbox");
    let subdir = sandbox.join("subdir");
    std::fs::create_dir_all(&subdir).unwrap();

    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(registry, rpc.clone(), lsp).with_fs_root(&sandbox);

    // 1. request_execute_command timeout must clean up BOTH pending and outbound_requests
    bridge
        .handle_rpc_message_result("commands.registerCommand", &json!({ "command": "slow.js.cmd" }))
        .await
        .unwrap();
    let timeout_err = bridge
        .request_execute_command("slow.js.cmd", vec![json!(1)])
        .await
        .unwrap_err();
    assert!(
        timeout_err.contains("Timed out executing JS command"),
        "unexpected timeout error: {}",
        timeout_err
    );
    assert_eq!(rpc.pending_count(), 0);
    assert!(
        rpc.take_outbound_requests().is_empty(),
        "stale outbound_requests leaked after request_execute_command timeout"
    );

    // 2. Remaining F2 validation & silent-swallow fixes
    let cases = [
        (
            "notebooks.updateControllerAffinity",
            json!({ "id": "missing_ctrl", "notebookUri": "file:///a.ipynb", "affinity": 1 }),
            "NOT_FOUND",
        ),
        (
            "notebooks.createController",
            json!({ "id": "nb_bad_langs", "supportedLanguages": "python" }),
            "INVALID_PARAMS",
        ),
        (
            "notebooks.updateCellOutputs",
            json!({ "notebookUri": "file:///a.ipynb", "cellIndex": 0, "outputs": "bad" }),
            "INVALID_PARAMS",
        ),
        (
            "notebooks.appendCellOutputs",
            json!({ "notebookUri": "file:///a.ipynb", "cellIndex": 0, "outputs": 42 }),
            "INVALID_PARAMS",
        ),
        (
            "window.setWebviewHtml",
            json!({ "id": "missing_panel", "html": "<h1>x</h1>" }),
            "NOT_FOUND",
        ),
        (
            "window.disposeWebviewPanel",
            json!({ "id": "missing_panel" }),
            "NOT_FOUND",
        ),
        (
            "window.createWebviewPanel",
            json!({ "id": "wv_bad_opts", "options": "not-an-object" }),
            "INVALID_PARAMS",
        ),
        (
            "window.registerCustomEditorProvider",
            json!({ "viewType": "ce.bad", "options": "not-an-object" }),
            "INVALID_PARAMS",
        ),
        (
            "window.registerCustomEditorProvider",
            json!({ "viewType": "ce.bad2", "options": { "webviewOptions": "bad" } }),
            "INVALID_PARAMS",
        ),
        ("env.openExternal", json!({}), "INVALID_PARAMS"),
        ("env.openExternal", json!({ "uri": 123 }), "INVALID_PARAMS"),
        (
            "workspace.openTextDocument",
            json!({ "uri": 123 }),
            "INVALID_PARAMS",
        ),
        (
            "workspace.getConfiguration",
            json!({ "section": 123 }),
            "INVALID_PARAMS",
        ),
    ];

    for (method, params, expected_code) in cases {
        let err = bridge
            .handle_rpc_message_result(method, &params)
            .await
            .unwrap_err();
        assert_eq!(
            err.code, expected_code,
            "Unexpected error code for {} ({:?}): {:?}",
            method, params, err
        );
    }

    // env.openExternal with invalid URI string still returns Ok(false)
    assert_eq!(
        bridge
            .handle_rpc_message_result("env.openExternal", &json!({ "uri": "" }))
            .await
            .unwrap(),
        json!(false)
    );

    // Disposed webview panel -> INVALID_STATE on setWebviewHtml & double disposeWebviewPanel
    bridge
        .handle_rpc_message_result(
            "window.createWebviewPanel",
            &json!({ "id": "wv_r2", "viewType": "t", "title": "T" }),
        )
        .await
        .unwrap();
    bridge
        .handle_rpc_message_result("window.disposeWebviewPanel", &json!({ "id": "wv_r2" }))
        .await
        .unwrap();
    assert_eq!(
        bridge
            .handle_rpc_message_result(
                "window.setWebviewHtml",
                &json!({ "id": "wv_r2", "html": "<h1>dead</h1>" })
            )
            .await
            .unwrap_err()
            .code,
        "INVALID_STATE"
    );
    assert_eq!(
        bridge
            .handle_rpc_message_result("window.disposeWebviewPanel", &json!({ "id": "wv_r2" }))
            .await
            .unwrap_err()
            .code,
        "INVALID_STATE"
    );

    // 3. Cross-platform EISDIR on fs.readFile and fs.unlink targeting a directory
    let read_dir_err = bridge
        .handle_rpc_message_result("fs.readFile", &json!({ "path": "subdir" }))
        .await
        .unwrap_err();
    assert_eq!(read_dir_err.code, "EISDIR", "got {:?}", read_dir_err);

    let unlink_dir_err = bridge
        .handle_rpc_message_result("fs.unlink", &json!({ "path": "subdir" }))
        .await
        .unwrap_err();
    assert_eq!(unlink_dir_err.code, "EISDIR", "got {:?}", unlink_dir_err);
}

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!(
            "{}_{}_{}",
            prefix,
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn test_compat_m1_1_live_node_subprocess_rpc_fs_and_commands() {
    use rustcode_ext_host::{NodeExtHostConfig, NodeExtHostProcess, RpcWireFormat};
    use std::time::Duration;

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    // Declare TempDirGuard BEFORE NodeExtHostProcess so the child is killed/reaped before cleanup.
    let guard = TempDirGuard::new("rustcode_m1_1_live");
    let sandbox = guard.path().join("sandbox");
    let outside = guard.path().join("outside");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();

    let driver_path = guard.path().join("live_driver.js");
    let ext_host_dir = repo_root.join("ext-host-js");
    let vscode_idx = ext_host_dir.join("vscode-api/index.js");
    let fs_mod = ext_host_dir.join("node-compat/fs.js");
    let path_mod = ext_host_dir.join("node-compat/path.js");
    let rpc_mod = ext_host_dir.join("rpc-bridge.js");

    let driver_src = format!(
        r#"const vscode = require({vscode});
const nodeFs = require({fs});
const nodePath = require({path});
const rpc = require({rpc});
const rpcDriveMismatch = process.platform === 'win32'
  ? require(({rpc}).replace(/^[A-Za-z]:/, (m) => m.toLowerCase()))
  : rpc;
if (!rpcDriveMismatch.isLiveStdioMode()) {{
  throw new Error('Windows drive-letter case mismatch split rpc-bridge singleton');
}}

// Exercise console.log frame isolation while stdio transport is active
console.log('LIVE_DRIVER_BOOT_LOG');

vscode.commands.registerCommand('test.echo', (arg) => {{
  return {{ echoed: arg, sep: nodePath.sep }};
}});

vscode.commands.registerCommand('test.throwErr', () => {{
  const err = new Error('intentional js failure');
  err.code = 'E_JS_BOOM';
  throw err;
}});

vscode.commands.registerCommand('test.slowCommand', async () => {{
  await new Promise((r) => setTimeout(r, 250));
  return 'too-late';
}});

vscode.commands.registerCommand('test.fsRoundtrip', async () => {{
  const assert = (cond, msg) => {{
    if (!cond) throw new Error('JS assert failed: ' + msg);
  }};

  // 1. mkdir recursive firstCreated & existing -> undefined
  const first = await nodeFs.promises.mkdir('deep/sub', {{ recursive: true }});
  assert(typeof first === 'string' && first.endsWith('deep'), 'expected firstCreated ending in deep, got ' + first);
  const second = await nodeFs.promises.mkdir('deep/sub', {{ recursive: true }});
  assert(second === undefined, 'existing recursive mkdir must return undefined, got ' + second);
  const single = await nodeFs.promises.mkdir('single');
  assert(single === undefined, 'non-recursive mkdir must return undefined');

  // 2. writeFile / readFile encodings + Buffer with non-zero byteOffset + Uint8Array
  await nodeFs.promises.writeFile('deep/sub/utf8.txt', 'hello-m1.1', 'utf8');
  assert((await nodeFs.promises.readFile('deep/sub/utf8.txt', 'utf8')) === 'hello-m1.1', 'utf8 read mismatch');

  const b64 = Buffer.from('binary-payload').toString('base64');
  await nodeFs.promises.writeFile('deep/sub/b64.bin', b64, 'base64');
  assert((await nodeFs.promises.readFile('deep/sub/b64.bin', 'base64')) === b64, 'base64 mismatch');
  assert((await nodeFs.promises.readFile('deep/sub/b64.bin', 'hex')) === Buffer.from('binary-payload').toString('hex'), 'hex mismatch');

  const sliced = Buffer.from([10, 20, 30, 40, 50]).subarray(1, 4);
  await nodeFs.promises.writeFile('deep/sub/slice.bin', sliced);
  const readBytes = await nodeFs.promises.readFile('deep/sub/slice.bin', null);
  assert(readBytes instanceof Uint8Array && Array.from(readBytes).join(',') === '20,30,40', 'sliced Buffer roundtrip failed');

  // 3. readdir withFileTypes -> Dirent
  const dirents = await nodeFs.promises.readdir('deep', {{ withFileTypes: true }});
  assert(dirents.length === 1 && dirents[0] instanceof nodeFs.Dirent, 'expected Dirent');
  assert(dirents[0].name === 'sub' && dirents[0].isDirectory() && !dirents[0].isFile(), 'Dirent directory check failed');

  // 4. stat, access, copyFile (+ COPYFILE_EXCL -> EEXIST), rename, unlink
  const st = await nodeFs.promises.stat('deep/sub/utf8.txt');
  assert(st.isFile() && !st.isDirectory() && st.size === 10, 'stat mismatch');
  await nodeFs.promises.access('deep/sub/utf8.txt', nodeFs.constants.R_OK);
  await nodeFs.promises.copyFile('deep/sub/utf8.txt', 'deep/sub/copy.txt', nodeFs.constants.COPYFILE_EXCL);

  let copyExclCode = null;
  try {{
    await nodeFs.promises.copyFile('deep/sub/utf8.txt', 'deep/sub/copy.txt', nodeFs.constants.COPYFILE_EXCL);
  }} catch (e) {{
    copyExclCode = e.code;
  }}
  assert(copyExclCode === 'EEXIST', 'expected EEXIST on COPYFILE_EXCL, got ' + copyExclCode);

  await nodeFs.promises.rename('deep/sub/copy.txt', 'deep/sub/renamed.txt');
  assert((await nodeFs.promises.readFile('deep/sub/renamed.txt', 'utf8')) === 'hello-m1.1', 'renamed file read failed');
  await nodeFs.promises.unlink('deep/sub/renamed.txt');

  // 5. rm EISDIR without recursive, recursive removal, force on missing, sandbox escape -> EACCES
  let rmDirCode = null;
  try {{
    await nodeFs.promises.rm('deep/sub');
  }} catch (e) {{
    rmDirCode = e.code;
  }}
  assert(rmDirCode === 'EISDIR', 'expected EISDIR on non-recursive rm dir, got ' + rmDirCode);

  await nodeFs.promises.rm('deep', {{ recursive: true }});
  await nodeFs.promises.rm('does-not-exist', {{ force: true }});

  let rmEscCode = null;
  try {{
    await nodeFs.promises.rm('../outside/secret.txt', {{ recursive: true, force: true }});
  }} catch (e) {{
    rmEscCode = e.code;
  }}
  assert(rmEscCode === 'EACCES', 'expected EACCES on sandbox escape rm, got ' + rmEscCode);

  // 6. Error propagation over live stdio: ENOENT, EACCES, ENOTDIR, METHOD_NOT_FOUND
  const expectErr = async (fn, expected) => {{
    let got = null;
    try {{
      await fn();
    }} catch (e) {{
      got = e.code;
    }}
    assert(got === expected, 'expected ' + expected + ', got ' + got);
  }};

  await expectErr(() => nodeFs.promises.readFile('missing.txt'), 'ENOENT');
  await expectErr(() => nodeFs.promises.readFile('../outside/secret.txt'), 'EACCES');
  await nodeFs.promises.writeFile('plain.txt', 'x');
  await expectErr(() => nodeFs.promises.readdir('plain.txt'), 'ENOTDIR');
  await expectErr(() => rpc.call('nonexistent.namespaceMethod', {{}}), 'METHOD_NOT_FOUND');

  // 7. JS -> Rust builtin command execution over live stdio
  const rustOut = await vscode.commands.executeCommand('rust.liveSum', 19, 23);
  assert(rustOut && rustOut.sum === 42, 'JS -> Rust command failed: ' + JSON.stringify(rustOut));

  return {{ allPassed: true }};
}});
"#,
        vscode = serde_json::to_string(&vscode_idx.to_string_lossy().into_owned()).unwrap(),
        fs = serde_json::to_string(&fs_mod.to_string_lossy().into_owned()).unwrap(),
        path = serde_json::to_string(&path_mod.to_string_lossy().into_owned()).unwrap(),
        rpc = serde_json::to_string(&rpc_mod.to_string_lossy().into_owned()).unwrap(),
    );
    std::fs::write(&driver_path, driver_src).unwrap();

    let fs_svc = Arc::new(rustcode_platform::FileSystemService::new());
    let fs_events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let fs_events_clone = Arc::clone(&fs_events);
    let _sub = fs_svc.on_did_change.event().attach(move |ev| {
        fs_events_clone.lock().push(ev.clone());
    });

    let cmd_reg = rustcode_platform::CommandRegistry::new();
    let _sum_disp = cmd_reg.register("rust.liveSum", |args| {
        let arr = args
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let a = arr.first().and_then(|v| v.as_i64()).unwrap_or(0);
        let b = arr.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
        Ok(json!({ "sum": a + b }))
    });

    let registry = Arc::new(rustcode_ext_host::ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = Arc::new(
        LanguageFeatureBridge::new(registry, rpc.clone(), lsp)
            .with_file_system_service(fs_svc)
            .with_command_registry(cmd_reg)
            .with_fs_root(&sandbox),
    );

    let mut proc = NodeExtHostProcess::spawn(
        bridge.clone(),
        NodeExtHostConfig {
            args: vec![driver_path.to_string_lossy().into_owned()],
            cwd: Some(repo_root),
            wire_format: RpcWireFormat::ContentLengthJson,
            ..Default::default()
        },
    )
    .expect("Failed to spawn Node.js extension host");

    // 1. Live extHost.ping over stdio
    let ping = rpc
        .request("extHost.ping", json!({ "hello": "world" }), Duration::from_secs(5))
        .await
        .expect("extHost.ping failed");
    assert_eq!(ping["pong"], json!(true));
    assert_eq!(ping["echo"]["hello"], json!("world"));

    // 2. Wait briefly for driver to finish registering commands via live RPC
    let mut registered = false;
    for _ in 0..50 {
        if bridge.command_registry().has_command("test.fsRoundtrip") {
            registered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(registered, "Driver failed to register commands in time; stderr={:?}", proc.stderr_tail());

    // 3. Bidirectional Rust -> JS commands.executeCommand (success & error)
    let echo_res = rpc
        .request(
            "commands.executeCommand",
            json!({ "command": "test.echo", "args": ["live-m1.1"] }),
            Duration::from_secs(5),
        )
        .await
        .expect("test.echo failed");
    assert_eq!(echo_res["echoed"], json!("live-m1.1"));

    let boom_err = rpc
        .request(
            "commands.executeCommand",
            json!({ "command": "test.throwErr", "args": [] }),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
    assert!(boom_err.contains("E_JS_BOOM"), "Expected E_JS_BOOM, got {}", boom_err);

    // 4. Full live FS roundtrip & error propagation suite driven from JS
    let fs_res = rpc
        .request(
            "commands.executeCommand",
            json!({ "command": "test.fsRoundtrip", "args": [] }),
            Duration::from_secs(10),
        )
        .await
        .expect("test.fsRoundtrip failed");
    assert_eq!(fs_res["allPassed"], json!(true));
    assert!(!fs_events.lock().is_empty(), "Expected FileSystemService change events");

    // 5. Timeout cleanup leaves pending_count() == 0
    let pending_before = rpc.pending_count();
    assert_eq!(pending_before, 0);
    let timeout_err = rpc
        .request(
            "commands.executeCommand",
            json!({ "command": "test.slowCommand", "args": [] }),
            Duration::from_millis(15),
        )
        .await
        .unwrap_err();
    assert!(
        timeout_err.contains("timed out"),
        "expected timeout error, got {}",
        timeout_err
    );
    assert_eq!(rpc.pending_count(), 0);

    // 6. Verify console.log was routed to stderr without breaking stdio framing
    assert!(
        proc.stderr_tail().iter().any(|l| l.contains("LIVE_DRIVER_BOOT_LOG")),
        "Expected console.log on stderr_tail, got {:?}",
        proc.stderr_tail()
    );

    // 7. Clean shutdown
    let _status = proc.shutdown().await.expect("shutdown failed");
    assert!(!proc.is_running());
    drop(proc);
}

#[tokio::test]
async fn test_compat_m1_2_live_fixture_extensions_end_to_end() {
    use parking_lot::RwLock;
    use rustcode_ext_host::{
        ExtensionLoader, ExtPromptKind, NodeExtHostConfig, NodeExtHostProcess, RpcWireFormat,
    };
    use std::time::Duration;

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    // Declare TempDirGuard BEFORE NodeExtHostProcess so proc is shut down and dropped first.
    let guard = TempDirGuard::new("rustcode_m1_2_live_fixtures");
    let workspace_dir = guard.path().join("workspace");
    let probe_ext_dir = guard.path().join("probe_ext");
    let app_root_dir = guard.path().join("app_root");
    std::fs::create_dir_all(workspace_dir.join("src")).unwrap();
    std::fs::create_dir_all(&probe_ext_dir).unwrap();
    std::fs::create_dir_all(&app_root_dir).unwrap();

    let crlf_file = workspace_dir.join("src").join("crlf.rs");
    std::fs::write(&crlf_file, "alpha\r\nbeta\r\ngamma").unwrap();

    std::fs::write(
        probe_ext_dir.join("package.json"),
        r#"{
            "name": "probe-ext",
            "publisher": "rustcode",
            "version": "1.0.0",
            "main": "./extension.js",
            "activationEvents": ["*"]
        }"#,
    )
    .unwrap();

    let probe_js = r#"const vscode = require('vscode');

exports.activate = function(context) {
  let lastConfigEvent = null;
  const docEvents = [];

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      lastConfigEvent = {
        affectsTabSize: e.affectsConfiguration('editor.tabSize'),
        affectsEditor: e.affectsConfiguration('editor'),
        affectsFiles: e.affectsConfiguration('files.autoSave'),
        tabSize: vscode.workspace.getConfiguration('editor').get('tabSize'),
      };
    }),
    vscode.workspace.onDidOpenTextDocument((doc) => docEvents.push('open:' + doc.languageId)),
    vscode.workspace.onDidChangeTextDocument((e) => docEvents.push('change:' + e.document.version)),
    vscode.workspace.onDidSaveTextDocument((doc) => docEvents.push('save:' + doc.isDirty)),
    vscode.commands.registerCommand('probe.runAll', async (crlfUri) => {
      const assert = (cond, msg) => {
        if (!cond) throw new Error('Probe assert failed: ' + msg);
      };

      // 1. Env & clipboard & openExternal
      await vscode.env.refresh();
      assert(vscode.env.uriScheme === 'rustcode', 'uriScheme');
      assert(vscode.env.appHost === 'desktop', 'appHost');
      assert(vscode.env.uiKind === vscode.UIKind.Desktop, 'uiKind');
      assert(typeof vscode.env.shell === 'string' && vscode.env.shell.length > 0, 'shell');
      await vscode.env.clipboard.writeText('m12-clip');
      assert((await vscode.env.clipboard.readText()) === 'm12-clip', 'clipboard roundtrip');
      assert((await vscode.env.openExternal('https://rustcode.dev')) === true, 'openExternal valid');
      assert((await vscode.env.openExternal('javascript:alert(1)')) === false, 'openExternal invalid');

      // 2. Bidirectional commands
      const mulRes = await vscode.commands.executeCommand('rust.liveMul', 6, 7);
      assert(mulRes && mulRes.product === 42, 'JS->Rust command failed');

      // 3. Prompts: QuickPick + cancelled -> undefined + InputBox
      const picked = await vscode.window.showQuickPick(['First', 'Second'], { placeHolder: 'pick' });
      assert(picked === 'Second', 'expected Second from responder, got ' + picked);
      const cancelledPick = await vscode.window.showQuickPick(['Only'], { placeHolder: 'cancel-me' });
      assert(cancelledPick === undefined, 'expected undefined on cancelled QuickPick, got ' + cancelledPick);
      const inputVal = await vscode.window.showInputBox({ prompt: 'enter-val' });
      assert(inputVal === 'typed-m12', 'expected typed-m12, got ' + inputVal);

      // 4. Workspace folders, findFiles, CRLF TextDocumentProxy, showTextDocument
      assert(vscode.workspace.workspaceFolders.length === 1, 'expected 1 workspace folder');
      const found = await vscode.workspace.findFiles('**/*.rs');
      assert(found.length === 1, 'findFiles expected 1 rs file, got ' + found.length);

      const doc = await vscode.workspace.openTextDocument(vscode.Uri.parse(crlfUri));
      assert(doc.eol === vscode.EndOfLine.CRLF, 'expected CRLF eol');
      assert(doc.lineCount === 3, 'expected 3 lines');
      assert(doc.getText(new vscode.Range(0, 0, 0, 99)) === 'alpha', 'CRLF line 0 slice leaked CR');
      assert(doc.getText(new vscode.Range(0, 2, 1, 2)) === 'pha\r\nbe', 'CRLF cross-line slice mismatch');
      const editor = await vscode.window.showTextDocument(doc, vscode.ViewColumn.One);
      assert(editor && editor.document.uri.toString() === doc.uri.toString(), 'showTextDocument failed');
      assert(vscode.window.activeTextEditor !== undefined, 'activeTextEditor missing');

      return {
        appRoot: vscode.env.appRoot,
        lastConfigEvent,
        docEvents,
        activeUri: vscode.window.activeTextEditor.document.uri.toString(),
      };
    }),
    vscode.commands.registerCommand('probe.getConfigState', () => lastConfigEvent),
  );

  return { ready: true };
};
"#;
    std::fs::write(probe_ext_dir.join("extension.js"), probe_js).unwrap();

    // Load all 4 extensions into shared ExtensionLoader
    let mut loader = ExtensionLoader::new();
    loader
        .load_from_dir(repo_root.join("resources/sample-extensions/hello-world"))
        .unwrap();
    loader
        .load_from_dir(repo_root.join("crates/rustcode-ext-host/tests/fixtures/extensions/theme_ext"))
        .unwrap();
    loader
        .load_from_dir(repo_root.join("crates/rustcode-ext-host/tests/fixtures/extensions/treeview_ext"))
        .unwrap();
    loader.load_from_dir(&probe_ext_dir).unwrap();
    let shared_loader = Arc::new(RwLock::new(loader));

    let cs = rustcode_platform::ConfigurationService::new();
    let cmd_reg = rustcode_platform::CommandRegistry::new();
    let _mul_disp = cmd_reg.register("rust.liveMul", |args| {
        let arr = args.and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let a = arr.first().and_then(|v| v.as_i64()).unwrap_or(0);
        let b = arr.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
        Ok(json!({ "product": a * b }))
    });

    let bridge = Arc::new(
        LanguageFeatureBridge::with_defaults()
            .with_shared_extension_loader(shared_loader.clone())
            .with_configuration_service(cs.clone())
            .with_command_registry(cmd_reg)
            .with_fs_root(&workspace_dir)
            .with_app_root(&app_root_dir),
    );
    bridge.add_workspace_folder(&workspace_dir, Some("LiveWs"));

    bridge
        .window_manager()
        .set_prompt_responder(|prompt| match &prompt.kind {
            ExtPromptKind::QuickPick { place_holder, .. } => {
                if place_holder.as_deref() == Some("cancel-me") {
                    None
                } else {
                    Some(json!("Second"))
                }
            }
            ExtPromptKind::InputBox { .. } => Some(json!("typed-m12")),
        });

    let mut proc = NodeExtHostProcess::spawn(
        bridge.clone(),
        NodeExtHostConfig {
            args: vec![],
            cwd: Some(repo_root.clone()),
            wire_format: RpcWireFormat::ContentLengthJson,
            ..Default::default()
        },
    )
    .expect("Failed to spawn Node.js extension host");

    // Ping transport
    let ping = bridge
        .rpc_bridge()
        .request("extHost.ping", json!({ "m": "1.2" }), Duration::from_secs(5))
        .await
        .expect("extHost.ping failed");
    assert_eq!(ping["pong"], json!(true));

    // 1. Activate declarative theme_ext (no main)
    let theme_snap = bridge.activate_extension("theme-oceanic").await.unwrap();
    assert_eq!(theme_snap.id, "rustcode.theme-oceanic");
    assert!(shared_loader.read().is_activated("rustcode.theme-oceanic"));
    assert!(shared_loader.read().is_activated("THEME-OCEANIC"));

    // 2. Activate unmodified sample hello-world via short manifest name case-insensitively
    let hello_snap = bridge.activate_extension("SAMPLE-HELLO-WORLD").await.unwrap();
    assert_eq!(hello_snap.id, "rustcode.sample-hello-world");
    assert!(shared_loader.read().is_activated("rustcode.sample-hello-world"));
    for _ in 0..50 {
        if bridge.command_registry().has_command("sample.helloWorld") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(bridge.command_registry().has_command("sample.helloWorld"));
    let _hello_out = bridge
        .request_execute_command("sample.helloWorld", vec![])
        .await
        .expect("sample.helloWorld execution failed");
    let mut notif_seen = false;
    for _ in 0..50 {
        if bridge
            .window_manager()
            .list_notifications()
            .iter()
            .any(|n| n.message == "Hello World from RustCode Extension Host!")
        {
            notif_seen = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(notif_seen, "hello-world notification not observed in ExtHostWindowManager");

    // 3. Activate treeview_ext and test synthetic handles + children + onDidChangeTreeData
    bridge
        .activate_extension("rustcode.project-outline-ext")
        .await
        .unwrap();
    for _ in 0..50 {
        if bridge
            .provider_registry()
            .has_provider_with_id(rustcode_ext_host::ProviderType::TreeData, "projectOutlineView")
            && bridge.command_registry().has_command("projectOutline.refresh")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let root_items = bridge
        .request_tree_children_result("projectOutlineView", None)
        .await
        .expect("request_tree_children_result failed");
    assert_eq!(root_items.len(), 2);
    assert_eq!(root_items[0].label, "src");
    let src_id = root_items[0]
        .id
        .as_deref()
        .expect("TreeItem without explicit id must receive synthetic handle");
    assert!(src_id.starts_with("__rustcode_tree_projectOutlineView_"));
    assert_eq!(root_items[1].id.as_deref(), Some("explicit_cargo_toml"));

    let child_items = bridge
        .request_tree_children_result("projectOutlineView", Some(src_id))
        .await
        .expect("request_tree_children_result for synthetic handle failed");
    assert_eq!(child_items.len(), 2);
    assert_eq!(child_items[0].label, "main.rs");
    assert!(child_items[0].resource_uri.is_some());
    assert_eq!(child_items[1].id.as_deref(), Some("explicit_lib_rs"));

    let rev_before = bridge.provider_registry().tree_refresh_revision();
    let refresh_res = bridge
        .request_execute_command("projectOutline.refresh", vec![])
        .await
        .expect("projectOutline.refresh failed");
    assert_eq!(refresh_res["refreshed"], json!(true));
    let mut tree_refreshed = false;
    for _ in 0..50 {
        if bridge.provider_registry().tree_refresh_revision() > rev_before {
            tree_refreshed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(tree_refreshed, "treeView.onDidChangeTreeData did not increment tree_refresh_revision");

    // 4. Activate probe_ext, push initial state + update config, and run full live probe
    bridge.activate_extension("probe-ext").await.unwrap();
    for _ in 0..50 {
        if bridge.command_registry().has_command("probe.runAll")
            && bridge.command_registry().has_command("probe.getConfigState")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    bridge.push_initial_state();
    cs.update(
        "editor.tabSize",
        json!(6),
        rustcode_platform::ConfigurationTarget::Workspace,
    )
    .unwrap();

    let crlf_uri = rustcode_platform::path_to_file_uri(&crlf_file);
    let probe_res = bridge
        .request_execute_command_with_timeout(
            "probe.runAll",
            vec![json!(crlf_uri)],
            Duration::from_secs(5),
        )
        .await
        .expect("probe.runAll failed");

    let expected_app_root = rustcode_platform::strip_windows_verbatim_prefix(app_root_dir.clone())
        .to_string_lossy()
        .into_owned();
    assert_eq!(probe_res["appRoot"], json!(expected_app_root));
    assert_eq!(
        bridge.env_manager().opened_external_uris(),
        vec!["https://rustcode.dev".to_string()]
    );
    assert_eq!(probe_res["lastConfigEvent"]["affectsTabSize"], json!(true));
    assert_eq!(probe_res["lastConfigEvent"]["affectsEditor"], json!(true));
    assert_eq!(probe_res["lastConfigEvent"]["affectsFiles"], json!(false));
    assert_eq!(probe_res["lastConfigEvent"]["tabSize"], json!(6));
    assert!(!bridge.take_pending_show_document_requests().is_empty());

    // Shutdown & drop NodeExtHostProcess BEFORE TempDirGuard drops
    let _status = proc.shutdown().await.expect("shutdown failed");
    assert!(!proc.is_running());
    drop(proc);
}

#[tokio::test]
async fn test_compat_m1_2_iteration2_live_stdio_and_edge_cases() {
    use parking_lot::RwLock;
    use rustcode_ext_host::{
        ExtensionLoader, NodeExtHostConfig, NodeExtHostProcess, RpcWireFormat,
    };
    use std::time::Duration;

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let guard = TempDirGuard::new("rustcode_m1_2_iter2_live");
    let folder_a = guard.path().join("folder_a");
    let folder_b = guard.path().join("folder_b");
    let ext_dir = guard.path().join("iter2_ext");
    std::fs::create_dir_all(folder_a.join("src")).unwrap();
    std::fs::create_dir_all(folder_b.join("src")).unwrap();
    std::fs::create_dir_all(&ext_dir).unwrap();

    let target_file = folder_a.join("src").join("main.rs");
    std::fs::write(&target_file, "fn init() {}\r\n").unwrap();
    let file_b = folder_b.join("src").join("lib.rs");
    std::fs::write(&file_b, "pub fn b() {}\r\n").unwrap();

    std::fs::write(
        ext_dir.join("package.json"),
        r#"{
            "name": "iter2-ext",
            "publisher": "rustcode",
            "version": "1.0.0",
            "main": "./extension.js",
            "activationEvents": ["*"]
        }"#,
    )
    .unwrap();

    let commands_js_path = repo_root
        .join("ext-host-js")
        .join("vscode-api")
        .join("commands.js")
        .to_string_lossy()
        .replace('\\', "/");

    let ext_js = format!(
        r#"const vscode = require('vscode');

exports.activate = function(context) {{
  const configEvents = [];
  const hybridNode = {{ name: 'hybrid' }};
  const treeEmitter = new vscode.EventEmitter();

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {{
      configEvents.push(e);
    }}),
    vscode.window.registerTreeDataProvider('iter2.hybridTree', {{
      onDidChangeTreeData: treeEmitter.event,
      getChildren(el) {{
        return el ? [] : [hybridNode];
      }},
      getTreeItem(el) {{
        const ti = new vscode.TreeItem(el.name);
        ti.id = 'hybrid_treeitem_id';
        return ti;
      }},
    }}),
    vscode.commands.registerCommand('iter2.fireHybridTreeRefresh', () => {{
      treeEmitter.fire(hybridNode);
      return true;
    }}),
    vscode.commands.registerCommand('iter2.verifyApplyEditSaveAllZeroSleep', async (targetUriStr) => {{
      const uri = vscode.Uri.parse(targetUriStr);
      const doc = await vscode.workspace.openTextDocument(uri);
      for (let i = 1; i <= 10; i++) {{
        const expected = 'fn iter_' + i + '() {{}}\r\n';
        const edit = new vscode.WorkspaceEdit();
        const fullRange = new vscode.Range(0, 0, doc.lineCount, 0);
        edit.replace(uri, fullRange, expected);
        const applied = await vscode.workspace.applyEdit(edit);
        if (!applied) throw new Error('applyEdit returned false at iter ' + i);
        // Zero sleep: doc.getText() and doc.isDirty MUST be immediately updated!
        if (doc.getText() !== expected) {{
          throw new Error('Stale doc.getText() immediately after applyEdit iter ' + i + ': ' + JSON.stringify(doc.getText()));
        }}
        if (doc.isDirty !== true) {{
          throw new Error('Stale doc.isDirty immediately after applyEdit iter ' + i);
        }}
        const saved = await vscode.workspace.saveAll(false);
        if (!saved) throw new Error('saveAll returned false at iter ' + i);
        if (doc.isDirty !== false) {{
          throw new Error('Stale doc.isDirty immediately after saveAll iter ' + i);
        }}
      }}
      return {{ ok: true, finalText: doc.getText(), isDirty: doc.isDirty }};
    }}),
    vscode.commands.registerCommand('iter2.verifyZeroArgOpenAndFindFiles', async () => {{
      const untitledDoc = await vscode.workspace.openTextDocument();
      const zeroFiles = await vscode.workspace.findFiles('**/*.rs', null, 0);
      return {{
        isUntitled: untitledDoc.isUntitled,
        scheme: untitledDoc.uri.scheme,
        zeroFilesLen: zeroFiles.length,
      }};
    }}),
    vscode.commands.registerCommand('iter2.checkConfigEvents', (folderAFileUri, folderBFileUri) => {{
      const aUri = vscode.Uri.parse(folderAFileUri);
      const bUri = vscode.Uri.parse(folderBFileUri);
      return configEvents.map((e) => ({{
        aTabSize: e.affectsConfiguration('editor.tabSize', aUri),
        bTabSize: e.affectsConfiguration('editor.tabSize', bUri),
        aFiles: e.affectsConfiguration('files.autoSave', aUri),
      }}));
    }}),
    vscode.commands.registerCommand('iter2.setupSplitCommandDisposal', () => {{
      const commandsPath = {commands_path_json};
      const resolved = require.resolve(commandsPath);
      const sharedFn = () => 'same-fn-v1';
      const dSame1 = vscode.commands.registerCommand('iter2.sameFnCmd', sharedFn);
      const dSame2 = vscode.commands.registerCommand('iter2.sameFnCmd', sharedFn);
      dSame1.dispose();

      // Simulate split module cache (e.g. Windows drive-letter casing difference)
      const dOld = vscode.commands.registerCommand('iter2.splitCmd', () => 'from-old');
      delete require.cache[resolved];
      const commandsAlt = require(commandsPath);
      commandsAlt.registerCommand('iter2.splitCmd', () => 'from-split-new');
      dOld.dispose();
      return true;
    }})
  );
}};
"#,
        commands_path_json = serde_json::to_string(&commands_js_path).unwrap()
    );
    std::fs::write(ext_dir.join("extension.js"), ext_js).unwrap();

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).unwrap();
    let shared_loader = Arc::new(RwLock::new(loader));
    let cs = rustcode_platform::ConfigurationService::new();

    let bridge = Arc::new(
        LanguageFeatureBridge::with_defaults()
            .with_shared_extension_loader(shared_loader)
            .with_configuration_service(cs.clone())
            .with_fs_root(guard.path()),
    );
    bridge.add_workspace_folder(&folder_a, Some("FolderA"));
    bridge.add_workspace_folder(&folder_b, Some("FolderB"));

    let mut proc = NodeExtHostProcess::spawn(
        bridge.clone(),
        NodeExtHostConfig {
            args: vec![],
            cwd: Some(repo_root),
            wire_format: RpcWireFormat::ContentLengthJson,
            ..Default::default()
        },
    )
    .expect("Failed to spawn Node.js extension host");

    bridge.activate_extension("rustcode.iter2-ext").await.unwrap();
    for _ in 0..50 {
        if bridge
            .command_registry()
            .has_command("iter2.verifyApplyEditSaveAllZeroSleep")
            && bridge
                .provider_registry()
                .has_provider_with_id(rustcode_ext_host::ProviderType::TreeData, "iter2.hybridTree")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 1. Live stdio notification-before-response ordering (zero sleep!)
    let target_uri = rustcode_platform::path_to_file_uri(&target_file);
    let edit_res = bridge
        .request_execute_command_with_timeout(
            "iter2.verifyApplyEditSaveAllZeroSleep",
            vec![json!(target_uri)],
            Duration::from_secs(5),
        )
        .await
        .expect("iter2.verifyApplyEditSaveAllZeroSleep failed");
    assert_eq!(edit_res["ok"], json!(true));
    assert_eq!(edit_res["finalText"], json!("fn iter_10() {}\r\n"));
    assert_eq!(edit_res["isDirty"], json!(false));

    // 2. Zero-arg openTextDocument() + findFiles(maxResults: 0)
    let misc_res = bridge
        .request_execute_command("iter2.verifyZeroArgOpenAndFindFiles", vec![])
        .await
        .expect("iter2.verifyZeroArgOpenAndFindFiles failed");
    assert_eq!(misc_res["isUntitled"], json!(true));
    assert_eq!(misc_res["scheme"], json!("untitled"));
    assert_eq!(misc_res["zeroFilesLen"], json!(0));

    // 3. TreeDataProvider onDidChangeTreeData(el) when .id is on TreeItem
    let root_items = bridge
        .request_tree_children_result("iter2.hybridTree", None)
        .await
        .expect("hybridTree getChildren failed");
    assert_eq!(root_items.len(), 1);
    assert_eq!(root_items[0].id.as_deref(), Some("hybrid_treeitem_id"));
    let _ = bridge.provider_registry().take_pending_tree_refreshes();
    bridge
        .request_execute_command("iter2.fireHybridTreeRefresh", vec![])
        .await
        .unwrap();
    let mut observed_hybrid_refresh = Vec::new();
    for _ in 0..50 {
        let refreshes = bridge.provider_registry().take_pending_tree_refreshes();
        if !refreshes.is_empty() {
            observed_hybrid_refresh = refreshes;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(observed_hybrid_refresh.len(), 1);
    assert_eq!(
        observed_hybrid_refresh[0].element_ids,
        vec!["hybrid_treeitem_id".to_string()]
    );

    // 4. ConfigurationChangeEvent.affectsConfiguration(section, scope)
    cs.update(
        "editor.tabSize",
        json!(2),
        rustcode_platform::ConfigurationTarget::WorkspaceFolder(folder_a.clone()),
    )
    .unwrap();
    cs.update(
        "editor.tabSize",
        json!(8),
        rustcode_platform::ConfigurationTarget::Workspace,
    )
    .unwrap();
    let uri_a = rustcode_platform::path_to_file_uri(&target_file);
    let uri_b = rustcode_platform::path_to_file_uri(&file_b);
    let mut cfg_res = serde_json::Value::Null;
    for _ in 0..50 {
        let r = bridge
            .request_execute_command(
                "iter2.checkConfigEvents",
                vec![json!(uri_a), json!(uri_b)],
            )
            .await
            .unwrap();
        if r.as_array().is_some_and(|arr| arr.len() >= 2) {
            cfg_res = r;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let arr = cfg_res.as_array().expect("expected 2 config events");
    assert_eq!(arr[0]["aTabSize"], json!(true));
    assert_eq!(arr[0]["bTabSize"], json!(false));
    assert_eq!(arr[0]["aFiles"], json!(false));
    assert_eq!(arr[1]["aTabSize"], json!(true));
    assert_eq!(arr[1]["bTabSize"], json!(true));

    // 5. Out-of-order command disposal across same-fn-ref & split module instances
    bridge
        .request_execute_command("iter2.setupSplitCommandDisposal", vec![])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        bridge
            .request_execute_command("iter2.sameFnCmd", vec![])
            .await
            .unwrap(),
        json!("same-fn-v1")
    );
    assert_eq!(
        bridge
            .request_execute_command("iter2.splitCmd", vec![])
            .await
            .unwrap(),
        json!("from-split-new")
    );

    let _status = proc.shutdown().await.expect("shutdown failed");
    assert!(!proc.is_running());
    drop(proc);
}

#[tokio::test]
async fn test_compat_m1_2_iteration3_live_stdio_edge_cases() {
    use parking_lot::RwLock;
    use rustcode_ext_host::{
        ExtensionLoader, NodeExtHostConfig, NodeExtHostProcess, RpcWireFormat,
    };
    use std::time::Duration;

    let guard = TempDirGuard::new("rustcode_m1_2_iter3");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let ext_dir = guard.path().join("ext");
    let root_a = guard.path().join("ws").join("RootA");
    let root_b = guard.path().join("ws").join("RootB");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::create_dir_all(&root_a).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();

    let file_a = root_a.join("a.rs");
    let file_b = root_b.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\r\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\r\n").unwrap();

    std::fs::write(
        ext_dir.join("package.json"),
        r#"{
            "name": "iter3-ext",
            "publisher": "rustcode",
            "version": "1.0.0",
            "main": "./extension.js",
            "activationEvents": ["*"]
        }"#,
    )
    .unwrap();

    let commands_js_path = repo_root
        .join("ext-host-js")
        .join("vscode-api")
        .join("commands.js")
        .to_string_lossy()
        .replace('\\', "/");
    let ext_js = format!(
        r#"const vscode = require('vscode');

exports.activate = function(context) {{
  const cfgEvents = [];
  let splitLowerCmds = null;
  let splitD1 = null;

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {{
      cfgEvents.push(e);
    }}),
    vscode.commands.registerCommand('iter3.updateFolderTabSize', async () => {{
      const wfA = vscode.workspace.workspaceFolders[0];
      const cfg = vscode.workspace.getConfiguration('editor', wfA.uri);
      await cfg.update('tabSize', 6, vscode.ConfigurationTarget.WorkspaceFolder);
      return {{
        wfUriJson: wfA.uri.toJSON(),
        refreshedTabSize: cfg.get('tabSize'),
      }};
    }}),
    vscode.commands.registerCommand('iter3.verifyNonAuthorityUris', async () => {{
      const u1 = vscode.Uri.parse('untitled:Untitled-1').toString();
      const u2 = vscode.Uri.parse('mailto:user@example.com').toString();
      const opened = await vscode.env.openExternal(vscode.Uri.parse('mailto:user@example.com'));
      return {{ u1, u2, opened }};
    }}),
    vscode.commands.registerCommand('iter3.reverseSplitCommands', async () => {{
      const commandsPath = {commands_path_json};
      const resolved = require.resolve(commandsPath);
      splitLowerCmds = require(commandsPath);
      splitD1 = splitLowerCmds.registerCommand('iter3.liveReverseSplit', () => 'stale_lower');
      delete require.cache[resolved];
      const splitUpperCmds = require(commandsPath);
      const d2 = splitUpperCmds.registerCommand('iter3.liveReverseSplit', () => 'active_upper');
      d2.dispose();
      const inGetCommands = (await splitLowerCmds.getCommands(false)).includes('iter3.liveReverseSplit');
      let execCode = null;
      let execVal = null;
      try {{
        execVal = await splitLowerCmds.executeCommand('iter3.liveReverseSplit');
      }} catch (err) {{
        execCode = err && err.code;
      }}
      if (splitD1) splitD1.dispose();
      return {{ inGetCommands, execCode, execVal }};
    }}),
    vscode.commands.registerCommand('iter3.checkUntitledConfigEvent', () => {{
      return cfgEvents.map((e) => ({{
        untitledUri: e.affectsConfiguration('editor.tabSize', vscode.Uri.parse('untitled:Untitled-1')),
        fileAUri: e.affectsConfiguration('editor.tabSize', vscode.workspace.workspaceFolders[0].uri),
      }}));
    }}),
    vscode.commands.registerCommand('iter3.webviewPostMessageUnawaited', () => {{
      const panel = vscode.window.createWebviewPanel('iter3.wv', 'Iter3', vscode.ViewColumn.One);
      panel.webview.postMessage({{ hello: 'world' }});
      return true;
    }})
  );
}};
"#,
        commands_path_json = serde_json::to_string(&commands_js_path).unwrap(),
    );
    std::fs::write(ext_dir.join("extension.js"), ext_js).unwrap();

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).unwrap();
    let shared_loader = Arc::new(RwLock::new(loader));
    let cs = rustcode_platform::ConfigurationService::new();

    let bridge = Arc::new(
        LanguageFeatureBridge::with_defaults()
            .with_shared_extension_loader(shared_loader)
            .with_configuration_service(cs.clone())
            .with_fs_root(guard.path()),
    );
    bridge.add_workspace_folder(&root_a, Some("RootA"));
    bridge.add_workspace_folder(&root_b, Some("RootB"));

    let mut proc = NodeExtHostProcess::spawn(
        bridge.clone(),
        NodeExtHostConfig {
            args: vec![],
            cwd: Some(repo_root),
            wire_format: RpcWireFormat::ContentLengthJson,
            ..Default::default()
        },
    )
    .expect("Failed to spawn Node.js extension host");

    bridge.activate_extension("rustcode.iter3-ext").await.unwrap();
    for _ in 0..50 {
        if bridge
            .command_registry()
            .has_command("iter3.updateFolderTabSize")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Issue #1: JS calls workspace.getConfiguration('editor', wfA.uri).update('tabSize', 6, WorkspaceFolder)
    let upd_res = bridge
        .request_execute_command("iter3.updateFolderTabSize", vec![])
        .await
        .expect("iter3.updateFolderTabSize failed");
    assert_eq!(upd_res["refreshedTabSize"], json!(6));
    assert_eq!(
        cs.get_for_resource::<i64>("editor.tabSize", Some(&file_a)),
        Some(6),
        "ConfigurationService folder entry must match file_a"
    );
    assert_eq!(
        cs.get_for_resource::<i64>("editor.tabSize", Some(&file_b)),
        Some(4),
        "Sibling folder file_b must remain at default tabSize 4"
    );

    // Issue #5: Check untitled URI on folder-scoped config change event
    let cfg_ev_res = bridge
        .request_execute_command("iter3.checkUntitledConfigEvent", vec![])
        .await
        .expect("iter3.checkUntitledConfigEvent failed");
    let cfg_ev_arr = cfg_ev_res.as_array().expect("expected config event array");
    assert!(!cfg_ev_arr.is_empty());
    let last_ev = cfg_ev_arr.last().unwrap();
    assert_eq!(last_ev["untitledUri"], json!(false));
    assert_eq!(last_ev["fileAUri"], json!(true));

    // Issue #2: Non-authority Uri.toString() & env.openExternal(mailto:)
    let uri_res = bridge
        .request_execute_command("iter3.verifyNonAuthorityUris", vec![])
        .await
        .expect("iter3.verifyNonAuthorityUris failed");
    assert_eq!(uri_res["u1"], json!("untitled:Untitled-1"));
    assert_eq!(uri_res["u2"], json!("mailto:user@example.com"));
    assert_eq!(uri_res["opened"], json!(true));
    assert_eq!(
        bridge.env_manager().opened_external_uris(),
        vec!["mailto:user@example.com".to_string()]
    );

    // Issue #3: Reverse split command disposal (d2.dispose() leaves no stale localCommands fallback)
    let split_res = bridge
        .request_execute_command("iter3.reverseSplitCommands", vec![])
        .await
        .expect("iter3.reverseSplitCommands failed");
    assert_eq!(split_res["inGetCommands"], json!(false));
    assert_eq!(split_res["execVal"], serde_json::Value::Null);
    assert_eq!(
        split_res["execCode"],
        json!("COMMAND_NOT_FOUND"),
        "expected COMMAND_NOT_FOUND error from Rust, got {:?}",
        split_res
    );
    assert!(
        !bridge.is_js_command("iter3.liveReverseSplit"),
        "bridge.is_js_command must be false after d2.dispose()"
    );
    assert!(
        !bridge.command_registry().has_command("iter3.liveReverseSplit"),
        "command_registry must not retain iter3.liveReverseSplit after d2.dispose()"
    );
    let rust_exec_err = bridge
        .request_execute_command("iter3.liveReverseSplit", vec![])
        .await
        .unwrap_err();
    assert!(
        rust_exec_err.contains("not found"),
        "expected Rust command not found error, got {}",
        rust_exec_err
    );

    // Issue #4: webview.postMessage() unawaited right before shutdown
    bridge
        .request_execute_command("iter3.webviewPostMessageUnawaited", vec![])
        .await
        .expect("iter3.webviewPostMessageUnawaited failed");
    let status = proc.shutdown().await.expect("shutdown failed");
    assert!(!proc.is_running());
    assert!(
        status.is_some_and(|s| s.success()),
        "Node exited non-zero after unawaited webview.postMessage before shutdown: {:?}",
        status
    );
    assert!(
        proc.stderr_tail().is_empty(),
        "Unexpected stderr during clean shutdown: {:?}",
        proc.stderr_tail()
    );
    drop(proc);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_compat_m1_2_multithread_live_stdio_reverse_split_stress() {
    use parking_lot::RwLock;
    use rustcode_ext_host::{
        ExtensionLoader, NodeExtHostConfig, NodeExtHostProcess, RpcWireFormat,
    };
    use std::time::Duration;

    let guard = TempDirGuard::new("rustcode_m1_2_iter4_mt_stress");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let ext_dir = guard.path().join("ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    std::fs::write(
        ext_dir.join("package.json"),
        r#"{
            "name": "iter4-mt-stress-ext",
            "publisher": "rustcode",
            "version": "1.0.0",
            "main": "./extension.js",
            "activationEvents": ["*"]
        }"#,
    )
    .unwrap();

    let commands_js_path = repo_root
        .join("ext-host-js")
        .join("vscode-api")
        .join("commands.js")
        .to_string_lossy()
        .replace('\\', "/");

    let ext_js = format!(
        r#"const vscode = require('vscode');

function loadSplitCommandModules(commandsPath) {{
  let lowerPath = commandsPath;
  let upperPath = commandsPath;
  if (process.platform === 'win32' && /^[a-zA-Z]:/.test(commandsPath)) {{
    lowerPath = commandsPath[0].toLowerCase() + commandsPath.slice(1);
    upperPath = commandsPath[0].toUpperCase() + commandsPath.slice(1);
  }}
  const lower = require(lowerPath);
  let upper = require(upperPath);
  if (lower === upper) {{
    const resolved = require.resolve(upperPath);
    delete require.cache[resolved];
    upper = require(upperPath);
  }}
  return {{ lower, upper, sameModule: lower === upper }};
}}

exports.activate = function(context) {{
  context.subscriptions.push(
    vscode.commands.registerCommand('stress.revSplitStep', async (cmdName, execFirst) => {{
      const {{ lower, upper, sameModule }} = loadSplitCommandModules({commands_path_json});
      const d1 = lower.registerCommand(cmdName, () => 'stale_lower');
      const d2 = upper.registerCommand(cmdName, () => 'active_upper');
      d2.dispose();

      let lowerErrCode = null;
      let lowerVal = null;
      let upperErrCode = null;
      let upperVal = null;
      let lowerInGet = null;
      let upperInGet = null;

      if (execFirst) {{
        try {{
          lowerVal = await lower.executeCommand(cmdName);
        }} catch (err) {{
          lowerErrCode = err && err.code;
        }}
        try {{
          upperVal = await upper.executeCommand(cmdName);
        }} catch (err) {{
          upperErrCode = err && err.code;
        }}
        lowerInGet = (await lower.getCommands(false)).includes(cmdName);
        upperInGet = (await upper.getCommands(false)).includes(cmdName);
      }} else {{
        lowerInGet = (await lower.getCommands(false)).includes(cmdName);
        upperInGet = (await upper.getCommands(false)).includes(cmdName);
        try {{
          lowerVal = await lower.executeCommand(cmdName);
        }} catch (err) {{
          lowerErrCode = err && err.code;
        }}
        try {{
          upperVal = await upper.executeCommand(cmdName);
        }} catch (err) {{
          upperErrCode = err && err.code;
        }}
      }}
      d1.dispose();
      return {{
        sameModule,
        lowerInGet,
        upperInGet,
        lowerErrCode,
        upperErrCode,
        lowerVal,
        upperVal,
      }};
    }}),
    vscode.commands.registerCommand('stress.singleModuleStep', async (cmdName) => {{
      const d = vscode.commands.registerCommand(cmdName, () => 'single_ok');
      const hasAfterReg = await vscode.commands.executeCommand('rust.hasCommand', cmdName);
      d.dispose();

      let execAfterDisposeCode = null;
      let execAfterDisposeVal = null;
      try {{
        execAfterDisposeVal = await vscode.commands.executeCommand(cmdName);
      }} catch (err) {{
        execAfterDisposeCode = err && err.code;
      }}
      const inGetAfterDispose = (await vscode.commands.getCommands(false)).includes(cmdName);
      return {{
        hasAfterReg,
        execAfterDisposeCode,
        execAfterDisposeVal,
        inGetAfterDispose,
      }};
    }}),
    vscode.commands.registerCommand('stress.staleTokenStep', async (cmdName) => {{
      const {{ lower, upper, sameModule }} = loadSplitCommandModules({commands_path_json});
      const d1 = lower.registerCommand(cmdName, () => 'stale_v1');
      const d2 = upper.registerCommand(cmdName, () => 'active_v2');
      d1.dispose();

      const lowerActiveVal = await lower.executeCommand(cmdName);
      const upperActiveVal = await upper.executeCommand(cmdName);
      const activeInGet = (await lower.getCommands(false)).includes(cmdName);

      d2.dispose();

      let lowerAfterDisposeCode = null;
      try {{
        await lower.executeCommand(cmdName);
      }} catch (err) {{
        lowerAfterDisposeCode = err && err.code;
      }}
      let upperAfterDisposeCode = null;
      try {{
        await upper.executeCommand(cmdName);
      }} catch (err) {{
        upperAfterDisposeCode = err && err.code;
      }}
      const afterDisposeInGet = (await upper.getCommands(false)).includes(cmdName);

      return {{
        sameModule,
        lowerActiveVal,
        upperActiveVal,
        activeInGet,
        lowerAfterDisposeCode,
        upperAfterDisposeCode,
        afterDisposeInGet,
      }};
    }})
  );
}};
"#,
        commands_path_json = serde_json::to_string(&commands_js_path).unwrap(),
    );
    std::fs::write(ext_dir.join("extension.js"), ext_js).unwrap();

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).unwrap();
    let shared_loader = Arc::new(RwLock::new(loader));
    let cmd_reg = rustcode_platform::CommandRegistry::new();
    let reg_for_has = cmd_reg.clone();
    let _has_cmd_disp = cmd_reg.register("rust.hasCommand", move |args| {
        let target = args
            .as_ref()
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .or_else(|| args.as_ref().and_then(|v| v.as_str()))
            .unwrap_or_default();
        Ok(json!(reg_for_has.has_command(target)))
    });
    let bridge = Arc::new(
        LanguageFeatureBridge::with_defaults()
            .with_shared_extension_loader(shared_loader)
            .with_command_registry(cmd_reg)
            .with_fs_root(guard.path()),
    );

    let mut proc = NodeExtHostProcess::spawn(
        bridge.clone(),
        NodeExtHostConfig {
            args: vec![],
            cwd: Some(repo_root),
            wire_format: RpcWireFormat::ContentLengthJson,
            ..Default::default()
        },
    )
    .expect("Failed to spawn Node.js extension host");

    bridge
        .activate_extension("rustcode.iter4-mt-stress-ext")
        .await
        .unwrap();
    for _ in 0..50 {
        if bridge.command_registry().has_command("stress.revSplitStep")
            && bridge.command_registry().has_command("stress.singleModuleStep")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let cmd = "stress.liveRevSplitCmd";
    for iter in 0..40 {
        for exec_first in [true, false] {
            let res = bridge
                .request_execute_command("stress.revSplitStep", vec![json!(cmd), json!(exec_first)])
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "iter {} (exec_first={}) stress.revSplitStep failed: {}",
                        iter, exec_first, e
                    )
                });

            assert_eq!(
                res["sameModule"],
                json!(false),
                "iter {} (exec_first={}): expected split module instances, got {:?}",
                iter,
                exec_first,
                res
            );
            assert_eq!(
                res["lowerInGet"],
                json!(false),
                "iter {} (exec_first={}): lower getCommands must not include disposed cmd: {:?}",
                iter,
                exec_first,
                res
            );
            assert_eq!(
                res["upperInGet"],
                json!(false),
                "iter {} (exec_first={}): upper getCommands must not include disposed cmd: {:?}",
                iter,
                exec_first,
                res
            );
            assert_eq!(
                res["lowerErrCode"],
                json!("COMMAND_NOT_FOUND"),
                "iter {} (exec_first={}): lower executeCommand must return COMMAND_NOT_FOUND: {:?}",
                iter,
                exec_first,
                res
            );
            assert_eq!(
                res["upperErrCode"],
                json!("COMMAND_NOT_FOUND"),
                "iter {} (exec_first={}): upper executeCommand must return COMMAND_NOT_FOUND: {:?}",
                iter,
                exec_first,
                res
            );
            assert!(
                res["lowerVal"].is_null(),
                "iter {} (exec_first={}): lowerVal must be null: {:?}",
                iter,
                exec_first,
                res
            );
            assert!(
                res["upperVal"].is_null(),
                "iter {} (exec_first={}): upperVal must be null: {:?}",
                iter,
                exec_first,
                res
            );
            assert!(
                !bridge.is_js_command(cmd),
                "iter {} (exec_first={}): bridge.is_js_command must be false after d2.dispose()",
                iter,
                exec_first
            );
            assert!(
                !bridge.command_registry().has_command(cmd),
                "iter {} (exec_first={}): command_registry must not contain disposed command",
                iter,
                exec_first
            );
            let rust_err = bridge
                .request_execute_command(cmd, vec![])
                .await
                .unwrap_err();
            assert!(
                rust_err.contains("not found"),
                "iter {} (exec_first={}): expected Rust not found error, got {}",
                iter,
                exec_first,
                rust_err
            );
        }

        // Single-module 0ms immediate registerCommand -> rust.hasCommand -> dispose -> executeCommand
        let single_cmd = format!("stress.singleImmediate.{}", iter);
        let single_res = bridge
            .request_execute_command("stress.singleModuleStep", vec![json!(single_cmd)])
            .await
            .unwrap_or_else(|e| panic!("iter {} stress.singleModuleStep failed: {}", iter, e));
        assert_eq!(
            single_res["hasAfterReg"],
            json!(true),
            "iter {}: rust.hasCommand must observe 0ms-immediate registerCommand: {:?}",
            iter,
            single_res
        );
        assert_eq!(
            single_res["execAfterDisposeCode"],
            json!("COMMAND_NOT_FOUND"),
            "iter {}: 0ms-immediate executeCommand after dispose must return COMMAND_NOT_FOUND: {:?}",
            iter,
            single_res
        );
        assert!(
            single_res["execAfterDisposeVal"].is_null(),
            "iter {}: execAfterDisposeVal must be null: {:?}",
            iter,
            single_res
        );
        assert_eq!(
            single_res["inGetAfterDispose"],
            json!(false),
            "iter {}: 0ms-immediate getCommands after dispose must be false: {:?}",
            iter,
            single_res
        );
        assert!(!bridge.is_js_command(&single_cmd));
        assert!(!bridge.command_registry().has_command(&single_cmd));
    }

    // Verify stale d1.dispose() preserves active d2 registration and d2.dispose() cleans up.
    let stale_cmd = "stress.staleTokenCmd";
    let stale_res = bridge
        .request_execute_command("stress.staleTokenStep", vec![json!(stale_cmd)])
        .await
        .expect("stress.staleTokenStep failed");
    assert_eq!(stale_res["sameModule"], json!(false));
    assert_eq!(stale_res["lowerActiveVal"], json!("active_v2"));
    assert_eq!(stale_res["upperActiveVal"], json!("active_v2"));
    assert_eq!(stale_res["activeInGet"], json!(true));
    assert_eq!(
        stale_res["lowerAfterDisposeCode"],
        json!("COMMAND_NOT_FOUND")
    );
    assert_eq!(
        stale_res["upperAfterDisposeCode"],
        json!("COMMAND_NOT_FOUND")
    );
    assert_eq!(stale_res["afterDisposeInGet"], json!(false));
    assert!(!bridge.is_js_command(stale_cmd));
    assert!(!bridge.command_registry().has_command(stale_cmd));

    let status = proc.shutdown().await.expect("shutdown failed");
    assert!(!proc.is_running());
    assert!(
        status.is_some_and(|s| s.success()),
        "Node exited non-zero: {:?}",
        status
    );
    assert!(
        proc.stderr_tail().is_empty(),
        "Unexpected stderr during clean shutdown: {:?}",
        proc.stderr_tail()
    );
    drop(proc);
}
