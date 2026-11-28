use phase10_ext_compat::TempTestDir;
use rustcode_base::context_key::ContextKeyService;
use rustcode_ext_host::manifest::{
    ExtensionLoader, JsonValidationRegistry, MenuContribution, SubmenuContribution,
    ViewWelcomeContribution,
};
use rustcode_ext_host::output::{ExtHostOutputManager, ExtLogLevel};
use rustcode_ext_host::provider::{
    DiagnosticCollectionManager, ProviderType, TreeItemCheckbox, TreeItemCollapsibleState,
    TreeViewOptionsMeta,
};
use rustcode_ext_host::tasks::{
    ExtHostTaskManager, ExtTaskEvent, ProblemMatcherContribution, ProblemMatcherRegistry,
    ProblemPattern, ProblemPatternSpec,
};
use rustcode_ext_host::webview::ExtHostWebviewViewManager;
use rustcode_ext_host::{ExtHostSecretManager, ExtHostVfsManager};
use rustcode_platform::theme::ThemeService;
use rustcode_protocols::lsp::DiagnosticSeverity;
use rustcode_terminal::shell_integration::{Osc633Parser, ShellIntegrationEvent};
use rustcode_terminal::TerminalInstance;
use rustcode_workbench::context_menu::ContextMenuRegistry;
use rustcode_workbench::explorer::{
    ContributedTreeNode, ContributedViewDescriptor, ContributedViewState,
};
use serde_json::json;
use std::path::PathBuf;

#[test]
fn test_tier1_m1_vfs_secrets_webview_log_tasks_and_tree_dnd() {
    // 1. VFS + TextDocumentContentProvider
    let vfs = ExtHostVfsManager::new();
    vfs.register_file_system_provider("customfs", true, false)
        .unwrap();
    vfs.register_text_document_content_provider("virtualgit")
        .unwrap();
    assert!(vfs.has_file_system_scheme("customfs:///foo.rs"));
    assert!(vfs.has_text_content_scheme("virtualgit:///Cargo.toml"));

    // 2. WebviewViewProvider + message routing
    let wv_views = ExtHostWebviewViewManager::new();
    wv_views.register_provider("sidebar.assistant", true);
    assert!(wv_views.has_provider("sidebar.assistant"));
    assert!(wv_views.set_html("sidebar.assistant", "<h2>Hello</h2>"));
    assert_eq!(
        wv_views.get_view("sidebar.assistant").unwrap().html,
        "<h2>Hello</h2>"
    );
    wv_views
        .post_message_to_view("sidebar.assistant", json!({ "ok": 1 }))
        .unwrap();
    assert_eq!(
        wv_views.drain_outbound_messages("sidebar.assistant"),
        vec![json!({ "ok": 1 })]
    );

    // 3. LogOutputChannel filtering
    let out = ExtHostOutputManager::new();
    out.handle_rpc_call(
        "window.createOutputChannel",
        &json!({ "id": "log_t1", "name": "Tier1Log", "log": true }),
    )
    .unwrap();
    out.handle_rpc_call(
        "window.setOutputChannelLogLevel",
        &json!({ "id": "log_t1", "logLevel": 4 }), // Warning
    )
    .unwrap();
    out.handle_rpc_call(
        "window.logOutputChannelMessage",
        &json!({ "id": "log_t1", "level": 3, "message": "info-skip" }),
    )
    .unwrap();
    out.handle_rpc_call(
        "window.logOutputChannelMessage",
        &json!({ "id": "log_t1", "level": 4, "message": "warn-keep" }),
    )
    .unwrap();
    let st = out.get_channel("log_t1").unwrap();
    assert_eq!(st.log_level, ExtLogLevel::Warning);
    assert!(!st.content.contains("info-skip"));
    assert!(st.content.contains("[WARN] warn-keep"));

    // 4. SecretStorage isolation
    let secrets = ExtHostSecretManager::new();
    secrets.store("ext.one", "k", "v1").unwrap();
    secrets.store("ext.two", "k", "v2").unwrap();
    assert_eq!(secrets.get("ext.one", "k").unwrap().as_deref(), Some("v1"));
    assert_eq!(secrets.get("ext.two", "k").unwrap().as_deref(), Some("v2"));

    // 5. ProblemMatcherRegistry ($tsc, $rustc, $gcc, $eslint-compact, custom) -> Diagnostics
    let matchers = ProblemMatcherRegistry::new();
    let diags = DiagnosticCollectionManager::new();
    let total_rustc = matchers
        .apply_task_output_to_diagnostics(
            "$rustc",
            Some(&PathBuf::from("/ws")),
            "error[E0308]: mismatched types\n  --> src/lib.rs:10:5",
            &diags,
        )
        .unwrap();
    assert_eq!(total_rustc, 1);

    let total_gcc = matchers
        .apply_task_output_to_diagnostics(
            "$gcc",
            None,
            "src/main.c:42:9: warning: unused variable",
            &diags,
        )
        .unwrap();
    assert_eq!(total_gcc, 1);
    let gcc_diags = diags.get_diagnostics_for_uri("file://src/main.c");
    assert_eq!(gcc_diags[0].severity, Some(DiagnosticSeverity::Warning));

    matchers
        .register(ProblemMatcherContribution {
            name: Some("custom-tier1".to_string()),
            owner: Some("tier1".to_string()),
            pattern: Some(ProblemPatternSpec::Single(ProblemPattern {
                regexp: r"^WARN\s+([^:]+):(\d+):\s+(.*)$".to_string(),
                file: Some(1),
                line: Some(2),
                message: Some(3),
                ..Default::default()
            })),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        matchers
            .apply_task_output_to_diagnostics(
                "custom-tier1",
                None,
                "WARN lib/foo.rs:7: check style",
                &diags,
            )
            .unwrap(),
        1
    );

    // 6. CustomExecution Pseudoterminal write/close RPCs
    let tasks = ExtHostTaskManager::new();
    tasks
        .handle_rpc_call(
            "tasks.executeTask",
            &json!({
                "id": "pty_task_1",
                "task": { "name": "PtyBuild", "source": "ext", "definition": { "type": "custom" } }
            }),
        )
        .unwrap();
    tasks
        .handle_rpc_call(
            "tasks.writeCustomExecution",
            &json!({ "id": "pty_task_1", "data": "compiling...\n" }),
        )
        .unwrap();
    tasks
        .handle_rpc_call(
            "tasks.closeCustomExecution",
            &json!({ "id": "pty_task_1", "exitCode": 0 }),
        )
        .unwrap();
    assert_eq!(
        tasks.execution_output("pty_task_1").as_deref(),
        Some("compiling...\n")
    );
    assert!(tasks.take_pending_events().iter().any(|e| matches!(
        e,
        ExtTaskEvent::CustomExecutionClose { id, exit_code: Some(0) } if id == "pty_task_1"
    )));

    // 7. Contributed TreeView checkboxes + welcome views + DragAndDrop meta
    let mut view_state = ContributedViewState::new(ContributedViewDescriptor {
        id: "tier1.tree".to_string(),
        name: "Tier 1 Tree".to_string(),
        when: None,
    });
    view_state.welcome_views.push(ViewWelcomeContribution {
        view: "tier1.tree".to_string(),
        contents: "Click Initialize to begin.".to_string(),
        when: Some("tier1Ready".to_string()),
        group: None,
    });
    let ck = ContextKeyService::new();
    assert!(view_state.active_welcome_contents(&ck).is_empty());
    ck.set_bool("tier1Ready", true);
    assert_eq!(
        view_state.active_welcome_contents(&ck),
        vec!["Click Initialize to begin.".to_string()]
    );
    view_state.root_items.push(ContributedTreeNode {
        node_id: "n1".to_string(),
        rpc_element_id: "n1".to_string(),
        label: "Checkable".to_string(),
        description: None,
        tooltip: None,
        collapsible_state: TreeItemCollapsibleState::None,
        command: None,
        resource_uri: None,
        checkbox_state: Some(TreeItemCheckbox {
            state: 0,
            tooltip: Some("Check".to_string()),
        }),
    });
    assert_eq!(view_state.toggle_node_checkbox("n1"), Some(1));
    assert_eq!(view_state.toggle_node_checkbox("n1"), Some(0));
    view_state.options_meta = Some(TreeViewOptionsMeta {
        view_id: "tier1.tree".to_string(),
        show_collapse_all: true,
        can_select_many: false,
        drag_mime_types: vec!["text/plain".to_string()],
        drop_mime_types: vec!["text/plain".to_string()],
        manage_checkbox_state_manually: false,
    });

    assert_eq!(
        ProviderType::from_str_name("TreeDragAndDrop"),
        Some(ProviderType::TreeDragAndDrop)
    );
    assert_eq!(
        ProviderType::from_str_name("FileSystem"),
        Some(ProviderType::FileSystem)
    );
}

#[test]
fn test_tier1_m1_contribs_themes_submenus_and_osc633_shell_integration() {
    let dir = TempTestDir::new("phase10_tier1_contribs");

    // 1. All 7 new contribution points parse via ExtensionLoader
    let pkg_json = json!({
        "name": "all-contribs",
        "publisher": "acme",
        "version": "1.0.0",
        "activationEvents": ["*"],
        "contributes": {
            "viewsContainers": {
                "activitybar": [{ "id": "acme-container", "title": "Acme", "icon": "icon.svg" }]
            },
            "views": {
                "acme-container": [{ "id": "acme.view", "name": "Acme View", "type": "webview" }]
            },
            "viewsWelcome": [{ "view": "acme.view", "contents": "Welcome Acme!" }],
            "submenus": [{ "id": "acme.submenu", "label": "Acme Submenu" }],
            "menus": {
                "editor/context": [{ "submenu": "acme.submenu", "group": "acme@1" }]
            },
            "jsonValidation": [{ "fileMatch": ["*.acme.json"], "url": "./schemas/acme.json" }],
            "problemMatchers": [{ "name": "acme-lint", "base": "$tsc" }],
            "iconThemes": [{ "id": "acme-icons", "label": "Acme Icons", "path": "./icons.json" }],
            "productIconThemes": [{ "id": "acme-prod-icons", "label": "Acme Product", "path": "./prod.json" }]
        }
    })
    .to_string();
    let manifest = ExtensionLoader::parse_manifest(&pkg_json).unwrap();
    let contribs = manifest.contributes.as_ref().unwrap();
    assert_eq!(
        contribs.views_containers.as_ref().unwrap()["activitybar"][0].id,
        "acme-container"
    );
    assert_eq!(contribs.views_welcome.as_ref().unwrap().len(), 1);
    assert_eq!(contribs.submenus.as_ref().unwrap()[0].id, "acme.submenu");
    assert_eq!(contribs.json_validation.as_ref().unwrap().len(), 1);
    assert_eq!(contribs.problem_matchers.as_ref().unwrap().len(), 1);
    assert_eq!(contribs.icon_themes.as_ref().unwrap().len(), 1);
    assert_eq!(contribs.product_icon_themes.as_ref().unwrap().len(), 1);

    // 2. JsonValidationRegistry + ContextMenuRegistry submenus
    let mut jv = JsonValidationRegistry::new();
    jv.register(
        "acme.all-contribs",
        dir.path(),
        &contribs.json_validation.as_ref().unwrap()[0],
    )
    .unwrap();
    assert_eq!(jv.schemas_for_filename("cfg.acme.json").len(), 1);

    let mut cm = ContextMenuRegistry::new();
    cm.register_submenus(&[SubmenuContribution {
        id: "acme.submenu".to_string(),
        label: "Acme Submenu".to_string(),
        icon: None,
    }]);
    let mut menus_map = std::collections::HashMap::new();
    menus_map.insert(
        "editor/context".to_string(),
        vec![MenuContribution {
            command: String::new(),
            submenu: Some("acme.submenu".to_string()),
            when: None,
            group: Some("acme@1".to_string()),
            alt: None,
        }],
    );
    cm.register_contributions(&menus_map, &std::collections::HashMap::new());
    assert_eq!(cm.get_submenu("acme.submenu").unwrap().label, "Acme Submenu");

    // 3. IconTheme & ProductIconTheme resolution on ThemeService
    let icon_path = dir.write_file(
        "icons.json",
        r##"{
            "iconDefinitions": {
                "_rs": { "iconPath": "./rs.svg" },
                "_folder": { "iconPath": "./folder.svg" }
            },
            "fileExtensions": { "rs": "_rs" },
            "folderNames": { "crates": "_folder" }
        }"##,
    );
    let prod_path = dir.write_file(
        "prod.json",
        r#"{
            "iconDefinitions": {
                "extensions-view-icon": { "fontCharacter": "\\eb01" }
            }
        }"#,
    );
    let mut theme = ThemeService::new();
    theme
        .load_icon_theme_from_disk("acme-icons", "Acme Icons", &icon_path)
        .unwrap();
    theme.set_icon_theme("acme-icons").unwrap();
    assert_eq!(
        theme.resolve_file_icon("foo.rs", Some("rust")).unwrap().0,
        "_rs"
    );
    assert_eq!(theme.resolve_folder_icon("crates", false).unwrap().0, "_folder");
    theme
        .load_product_icon_theme_from_disk("acme-prod-icons", "Acme Product", &prod_path)
        .unwrap();
    theme.set_product_icon_theme("acme-prod-icons").unwrap();
    assert!(theme.resolve_product_icon("extensions-view-icon").is_some());

    // 4. Terminal OSC 633 shell integration chunked across feed_vt_bytes
    let mut term = TerminalInstance::new("t1", "sh", "bash", PathBuf::from("/workspace"));
    let seq = b"\x1b]633;A\x07\x1b]633;P;Cwd=/workspace/src\x07\x1b]633;E;cargo test\x07\x1b]633;C\x07running tests\x1b]633;D;0\x07";
    let (left, right) = seq.split_at(seq.len() / 2);
    term.feed_vt_bytes(left);
    term.feed_vt_bytes(right);

    let sh = term.shell_integration();
    assert!(sh.active);
    assert_eq!(sh.cwd.as_deref(), Some("/workspace/src"));
    assert_eq!(sh.executions.len(), 1);
    assert_eq!(sh.executions[0].command_line, "cargo test");
    assert_eq!(sh.executions[0].exit_code, Some(0));

    let events = term.drain_shell_integration_events();
    assert!(events
        .iter()
        .any(|e| matches!(e, ShellIntegrationEvent::CommandFinished { exit_code: Some(0) })));
    let _ = Osc633Parser::default();
}
