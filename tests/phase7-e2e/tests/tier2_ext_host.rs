use phase7_e2e::{create_bridge_with_root, repo_root, E2eWindow, TempWorkspace};
use rustcode_app::RustCodeContext;
use rustcode_ext_host::{
    ExtHostEnvManager, ExtHostWindowManager, ExtensionLoader, LanguageFeatureBridge,
    ProviderRegistry, RpcBridge, RpcMessage,
};
use rustcode_platform::configuration::ConfigurationTarget;
use rustcode_platform::fs::{
    file_uri_to_path, is_file_uri, path_starts_with, path_strip_prefix, paths_equivalent,
    strip_windows_verbatim_prefix,
};
use rustcode_protocols::lsp::LspClientManager;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

// ============================================================================
// F1: Live Rust <-> JS RPC Transport (Tier 2: Boundary / Error)
// ============================================================================

#[tokio::test]
async fn f1_rpc_bridge_timeout_cleans_up_pending_entry() {
    let ws = TempWorkspace::new();
    let (bridge, rpc) = create_bridge_with_root(ws.path());

    bridge
        .handle_rpc_message(
            "commands.registerCommand",
            &json!({ "command": "ext.slowNeverReplies" }),
        )
        .await
        .expect("registerCommand");

    let res = bridge
        .request_execute_command("ext.slowNeverReplies", vec![])
        .await;
    assert!(res.is_err(), "expected timeout error");
    assert_eq!(
        rpc.pending_count(),
        0,
        "F1 requires timed-out outbound RPC requests to remove their pending entry without leaking"
    );
}

#[test]
fn f1_rpc_bridge_unknown_response_id_returns_false_without_mutating_pending() {
    let rpc = RpcBridge::new();
    let (_req, _rx) = rpc.create_request("m1", json!({}));
    assert_eq!(rpc.pending_count(), 1);
    assert!(!rpc.handle_incoming_response(999_999, Some(json!("nope")), None));
    assert_eq!(rpc.pending_count(), 1);
}

#[tokio::test]
async fn f1_rpc_bridge_duplicate_response_id_returns_false_second_time() {
    let rpc = RpcBridge::new();
    let (req, rx) = rpc.create_request("m1", json!({}));
    let id = match req {
        RpcMessage::Request { id, .. } => id,
        _ => panic!("expected request"),
    };
    assert!(rpc.handle_incoming_response(id, Some(json!(1)), None));
    assert!(!rpc.handle_incoming_response(id, Some(json!(2)), None));
    assert_eq!(rx.await.unwrap().unwrap(), json!(1));
}

#[test]
fn f1_rpc_message_from_json_rejects_malformed_and_truncated_frames() {
    for bad in ["", "{", "{\"type\":\"unknown\"}", "{\"type\":\"request\",\"id\":\"nan\"}"] {
        assert!(
            RpcMessage::from_json(bad).is_err(),
            "expected JSON parse error for {:?}",
            bad
        );
    }
}

#[test]
fn f1_rpc_message_from_msgpack_rejects_corrupt_bytes() {
    for bad in [&[][..], &[0xc1][..], &[0x82, 0xa4, b't', b'y', b'p', b'e'][..]] {
        assert!(
            RpcMessage::from_msgpack(bad).is_err(),
            "expected MsgPack error for {:?}",
            bad
        );
    }
}

// ============================================================================
// F2: RPC Error Propagation (Tier 2: Boundary / Error)
// ============================================================================

#[tokio::test]
async fn f2_handle_rpc_message_unknown_method_returns_structured_error() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("nonexistent.namespace.method", &json!({}))
        .await
        .expect("F2 requires unknown RPC method to return structured { error } instead of None");
    assert!(res.get("error").is_some(), "expected error envelope: {:?}", res);
    assert!(res["error"]["message"]
        .as_str()
        .is_some_and(|m| !m.is_empty()));
}

#[tokio::test]
async fn f2_handle_rpc_message_fs_invalid_encoding_returns_einval() {
    let ws = TempWorkspace::new();
    ws.write_file("a.txt", "hello");
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("fs.readFile", &json!({ "path": "a.txt", "encoding": "latin99" }))
        .await
        .expect("F2 requires structured { error } on invalid encoding");
    assert_eq!(res["error"]["code"], json!("EINVAL"));
}

#[tokio::test]
async fn f2_handle_rpc_message_fs_mkdir_existing_returns_eexist() {
    let ws = TempWorkspace::new();
    ws.write_file("existing/file.txt", "x");
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("fs.mkdir", &json!({ "path": "existing", "options": { "recursive": false } }))
        .await
        .expect("F2 requires structured { error } on EEXIST");
    assert_eq!(res["error"]["code"], json!("EEXIST"));
}

#[tokio::test]
async fn f2_handle_rpc_message_fs_unlink_missing_returns_enoent() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("fs.unlink", &json!({ "path": "ghost.txt" }))
        .await
        .expect("F2 requires structured { error } on fs.unlink ENOENT");
    assert_eq!(res["error"]["code"], json!("ENOENT"));
}

#[tokio::test]
async fn f2_handle_rpc_message_commands_register_empty_name_returns_error() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("commands.registerCommand", &json!({ "command": "   " }))
        .await
        .expect("F2 requires structured { error } for empty command name");
    assert!(res.get("error").is_some(), "expected error envelope: {:?}", res);
}

// ============================================================================
// F3: RustCodeContext & WorkbenchWindow Bridge Wiring (Tier 2: Boundary / Error)
// ============================================================================

#[test]
fn f3_create_window_with_empty_loader_still_attaches_functional_bridge() {
    let app = RustCodeContext::new();
    let window = app.create_window();
    let bridge = window
        .language_bridge
        .as_ref()
        .expect("F3 requires create_window() to attach language_bridge even with zero extensions");
    assert!(bridge.env_manager().list_extensions().is_empty());
}

#[test]
fn f3_two_windows_from_same_context_share_configuration_and_commands() {
    let app = RustCodeContext::new();
    let mut win1 = app.create_window();
    let mut win2 = app.create_window();

    app.config
        .update("editor.minimap.enabled", json!(false), ConfigurationTarget::User)
        .unwrap();
    win1.sync_configuration();
    win2.sync_configuration();

    assert_eq!(win1.config.get::<bool>("editor.minimap.enabled"), Some(false));
    assert_eq!(win2.config.get::<bool>("editor.minimap.enabled"), Some(false));

    let mut disp = app
        .commands
        .register("e2e.sharedTwo", |_| Ok(json!({ "ok": 2 })));
    assert!(win1.commands.has_command("e2e.sharedTwo"));
    assert!(win2.commands.has_command("e2e.sharedTwo"));
    disp.dispose();
}

#[test]
fn f3_opening_unknown_extension_and_empty_uri_does_not_panic() {
    let mut win = E2eWindow::new();
    win.window.open_file("untitled:Empty", "Empty", "", false);
    win.window.open_file("file:///tmp/data.unknownext", "data.unknownext", "hello", true);
    let scene = win.render();
    assert_eq!(win.read_doc("untitled:Empty").as_deref(), Some(""));
    assert_eq!(
        win.read_doc("file:///tmp/data.unknownext").as_deref(),
        Some("hello")
    );
    assert!(
        scene.contains_text("data.unknownext"),
        "expected 'data.unknownext' tab in rendered scene, got: {}",
        scene.all_text()
    );
}

#[test]
fn f3_closing_active_editor_when_no_editors_open_is_safe_noop() {
    let mut win = E2eWindow::new();
    win.dispatch_ctrl_char('w');
    let scene = win.render();
    assert!(win.active_editor().is_none());
    assert!(win.window.view.editor_area.model.current_group_ref().tabs.is_empty());
    assert!(
        scene.contains_text("EXPLORER"),
        "expected Explorer UI to remain rendered after closing empty editor, got: {}",
        scene.all_text()
    );
}

#[test]
fn f3_rustcode_context_loads_default_keybindings_on_startup() {
    let app = RustCodeContext::new();
    assert!(app
        .keybindings
        .lookup_keybinding("workbench.action.toggleSidebar")
        .is_some());
    assert!(app
        .keybindings
        .lookup_keybinding("workbench.action.showCommands")
        .is_some());
}

// ============================================================================
// F4: Bidirectional commands.* Routing (Tier 2: Boundary / Error)
// ============================================================================

#[tokio::test]
async fn f4_reregistering_same_command_disposes_old_without_removing_new() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    bridge
        .handle_rpc_message("commands.registerCommand", &json!({ "command": "ext.dup" }))
        .await
        .unwrap();
    bridge
        .handle_rpc_message("commands.registerCommand", &json!({ "command": "ext.dup" }))
        .await
        .unwrap();
    assert!(bridge.command_registry().has_command("ext.dup"));

    let unreg = bridge
        .handle_rpc_message(
            "commands.unregisterCommand",
            &json!({ "command": "ext.dup" }),
        )
        .await
        .unwrap();
    assert_eq!(unreg["unregistered"], json!(true));
    assert!(!bridge.command_registry().has_command("ext.dup"));
}

#[tokio::test]
async fn f4_recursion_guard_prevents_loop_when_js_command_executed_via_rpc() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    bridge
        .handle_rpc_message("commands.registerCommand", &json!({ "command": "ext.jsOnly" }))
        .await
        .unwrap();

    let res = bridge
        .handle_rpc_message(
            "commands.executeCommand",
            &json!({ "command": "ext.jsOnly", "args": [] }),
        )
        .await;
    // Must not bounce back into JS infinitely; must return structured COMMAND_RECURSION error.
    let val = res.expect("F2/F4 requires structured { error } from recursion guard, not None");
    assert_eq!(
        val["error"]["code"],
        json!("COMMAND_RECURSION"),
        "expected COMMAND_RECURSION error envelope, got {:?}",
        val
    );
}

#[tokio::test]
async fn f4_request_execute_command_unknown_command_returns_err() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let err = bridge
        .request_execute_command("does.not.exist", vec![])
        .await
        .expect_err("expected unknown command error");
    assert!(!err.is_empty());
}

#[tokio::test]
async fn f4_unregister_unknown_command_returns_false_cleanly() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message(
            "commands.unregisterCommand",
            &json!({ "command": "never.registered" }),
        )
        .await
        .unwrap();
    assert_eq!(res["unregistered"], json!(false));
}

#[test]
fn f4_command_registry_execute_builtin_without_window_does_not_panic_and_unknown_errs() {
    let app = RustCodeContext::new();
    assert!(app.commands.execute("definitely.missing", None).is_err());
    assert!(app
        .commands
        .execute("workbench.action.toggleSidebar", None)
        .is_ok());
}

// ============================================================================
// F5: window.* Prompts, Editors & TreeDataProvider (Tier 2: Boundary / Error)
// ============================================================================

#[tokio::test]
async fn f5_window_manager_timeout_cleans_up_pending_waiter_and_resolves_null() {
    let mgr = ExtHostWindowManager::new();
    mgr.set_await_responses(true);
    mgr.set_response_timeout(Some(Duration::from_millis(25)));

    let res = mgr
        .handle_rpc_call(
            "window.showInformationMessage",
            &json!({ "message": "Timed out", "items": ["OK"] }),
        )
        .await
        .unwrap();
    assert_eq!(res, Value::Null);
    assert!(mgr.active_notifications().is_empty());
}

#[tokio::test]
async fn f5_window_manager_post_resolution_respond_returns_false() {
    let mgr = ExtHostWindowManager::new();
    let res = mgr
        .handle_rpc_call(
            "window.showInformationMessage",
            &json!({ "message": "Once", "items": ["A", "B"] }),
        )
        .await
        .unwrap();
    assert_eq!(res, Value::Null);
    let notif = mgr.list_notifications().pop().unwrap();
    assert!(mgr.respond_notification(&notif.id, Some(json!("A"))));
    assert!(!mgr.respond_notification(&notif.id, Some(json!("B"))));
    assert!(!mgr.respond_notification("nonexistent_notif", Some(json!("A"))));
}

#[tokio::test]
async fn f5_window_manager_unsupported_method_returns_err() {
    let mgr = ExtHostWindowManager::new();
    assert!(mgr
        .handle_rpc_call("window.unknownMethod", &json!({}))
        .await
        .is_err());
}

#[tokio::test]
async fn f5_tree_data_provider_timeout_returns_empty_vec_and_unregister_removes() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TreeData", "id": "temp.tree" }),
        )
        .await
        .unwrap();
    assert!(bridge
        .provider_registry
        .has_provider_with_id(rustcode_ext_host::ProviderType::TreeData, "temp.tree"));

    bridge
        .handle_rpc_message("extHost.unregisterProvider", &json!({ "id": "temp.tree" }))
        .await
        .unwrap();
    assert!(!bridge
        .provider_registry
        .has_provider_with_id(rustcode_ext_host::ProviderType::TreeData, "temp.tree"));

    // Request with no responder times out gracefully returning empty Vec
    let items = bridge.request_tree_children("temp.tree", None).await;
    assert!(items.is_empty());
}

#[tokio::test]
async fn f5_window_show_text_document_missing_file_does_not_succeed() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());
    let missing_uri = ws.file_uri("missing.rs");

    let res = bridge
        .handle_rpc_message("window.showTextDocument", &json!({ "uri": missing_uri }))
        .await;
    // Must not return a fake opened document; requires structured error (not METHOD_NOT_FOUND).
    let val = res.expect("F2/F5 requires structured { error } for missing file, not None");
    assert!(
        val.get("error").is_some(),
        "expected error for missing file: {:?}",
        val
    );
    let code = val["error"]["code"].as_str().unwrap_or("");
    assert_ne!(
        code, "METHOD_NOT_FOUND",
        "window.showTextDocument must be implemented, not METHOD_NOT_FOUND: {:?}",
        val
    );
    assert!(
        matches!(code, "ENOENT" | "FS_ERROR" | "INVALID_PARAMS" | "RPC_ERROR"),
        "expected file-not-found error code, got {:?}",
        val
    );
}

// ============================================================================
// F6: workspace.* Hydration, Config & Events (Tier 2: Boundary / Error)
// ============================================================================

#[tokio::test]
async fn f6_workspace_update_configuration_invalid_target_and_empty_key_fail() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    for (key, target) in [("", json!(1)), ("editor.fontSize", json!(99))] {
        let res = bridge
            .handle_rpc_message(
                "workspace.updateConfiguration",
                &json!({ "key": key, "value": 12, "target": target }),
            )
            .await;
        let v = res.expect("F2/F6 requires structured { error } for invalid key/target, not None");
        assert!(v.get("error").is_some(), "expected error envelope, got {:?}", v);
        assert_ne!(
            v["error"]["code"],
            json!("METHOD_NOT_FOUND"),
            "workspace.updateConfiguration must be implemented, got {:?}",
            v
        );
    }
}

#[tokio::test]
async fn f6_workspace_mixed_nested_and_dotted_keys_resolve_consistently() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());
    bridge
        .configuration_service()
        .load_user_settings(r#"{ "myExt.a.b": 10, "myExt": { "a": { "other": 20 } } }"#)
        .unwrap();

    let cfg = bridge
        .handle_rpc_message("workspace.getConfiguration", &json!({ "section": "myExt" }))
        .await
        .unwrap();
    assert_eq!(cfg["a"]["other"], json!(20));

    let inspect = bridge
        .handle_rpc_message(
            "workspace.inspectConfiguration",
            &json!({ "key": "myExt.a.b" }),
        )
        .await
        .unwrap();
    assert_eq!(inspect["globalValue"], json!(10));
}

#[tokio::test]
async fn f6_workspace_folder_target_without_scope_falls_back_to_workspace() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message(
            "workspace.updateConfiguration",
            &json!({ "key": "editor.tabSize", "value": 8, "target": 3 }),
        )
        .await
        .unwrap();
    assert_eq!(res["updated"], json!(true));
    assert_eq!(
        bridge.configuration_service().get::<i64>("editor.tabSize"),
        Some(8)
    );
}

#[tokio::test]
async fn f6_workspace_open_text_document_percent_encoded_space_and_missing_file() {
    let ws = TempWorkspace::new();
    let file = ws.write_file("my file.rs", "fn spaced() {}");
    let uri = rustcode_platform::fs::path_to_file_uri(&file).replace(' ', "%20");
    let (bridge, _) = create_bridge_with_root(ws.path());

    let doc = bridge
        .handle_rpc_message("workspace.openTextDocument", &json!({ "uri": uri }))
        .await
        .expect("should decode %20 in file URI");
    assert_eq!(doc["text"], json!("fn spaced() {}"));

    let missing = bridge
        .handle_rpc_message(
            "workspace.openTextDocument",
            &json!({ "uri": ws.file_uri("absent.rs") }),
        )
        .await;
    let v = missing.expect("F2/F6 requires structured { error } for missing file, not None");
    assert_eq!(
        v["error"]["code"],
        json!("ENOENT"),
        "missing file must return ENOENT error envelope, got {:?}",
        v
    );
}

#[tokio::test]
async fn f6_workspace_open_text_document_lf_vs_empty_file_eol() {
    let ws = TempWorkspace::new();
    let empty = ws.write_file("empty.txt", "");
    let lf = ws.write_file("lf.txt", "a\nb\n");
    let (bridge, _) = create_bridge_with_root(ws.path());

    let empty_doc = bridge
        .handle_rpc_message(
            "workspace.openTextDocument",
            &json!({ "uri": rustcode_platform::fs::path_to_file_uri(&empty) }),
        )
        .await
        .unwrap();
    assert_eq!(empty_doc["eol"], json!(1));
    assert_eq!(empty_doc["text"], json!(""));

    let lf_doc = bridge
        .handle_rpc_message(
            "workspace.openTextDocument",
            &json!({ "uri": rustcode_platform::fs::path_to_file_uri(&lf) }),
        )
        .await
        .unwrap();
    assert_eq!(lf_doc["eol"], json!(1));
}

// ============================================================================
// F7: env.* & extensions.* (Tier 2: Boundary / Error)
// ============================================================================

#[test]
fn f7_env_open_external_rejects_unsafe_schemes_and_control_characters() {
    let mgr = ExtHostEnvManager::new();
    for bad in [
        "",
        "   ",
        "javascript:alert(1)",
        "data:text/html,<script>1</script>",
        "vbscript:msgbox",
        "https://example.com/has space",
        "https://example.com\r\n",
        "https://example.com\n",
        "\thttps://example.com",
        "\u{000b}https://example.com",
        "https://",
        "mailto:invalid",
    ] {
        assert!(!mgr.open_external(bad), "expected rejection for {:?}", bad);
    }
    assert!(mgr.opened_external_uris().is_empty());
}

#[test]
fn f7_env_extensions_unknown_lookup_and_activate_return_null_or_none() {
    let mgr = ExtHostEnvManager::new();
    let missing = mgr
        .handle_rpc_call("extensions.getExtension", &json!({ "id": "no.such.ext" }))
        .unwrap();
    assert_eq!(missing, Value::Null);

    let act = mgr
        .handle_rpc_call("extensions.activate", &json!({ "id": "no.such.ext" }))
        .unwrap();
    assert_eq!(act["activated"], json!(false));
}

#[test]
fn f7_env_missing_required_params_return_err() {
    let mgr = ExtHostEnvManager::new();
    assert!(mgr.handle_rpc_call("env.clipboardWrite", &json!({})).is_err());
    assert!(mgr.handle_rpc_call("extensions.getExtension", &json!({})).is_err());
    assert!(mgr.handle_rpc_call("extensions.activate", &json!({})).is_err());
    assert!(mgr.handle_rpc_call("env.unsupported", &json!({})).is_err());
}

#[test]
fn f7_extensions_activate_syncs_extension_loader_activation_flag() {
    let sample_dir = repo_root().join("resources/sample-extensions/hello-world");
    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&sample_dir).unwrap();
    assert!(!loader.is_activated("sample-hello-world"));

    let bridge = LanguageFeatureBridge::new(
        Arc::new(ProviderRegistry::new()),
        Arc::new(RpcBridge::new()),
        Arc::new(LspClientManager::new()),
    )
    .with_extension_loader(&loader);

    let _ = bridge.env_manager().take_pending_events();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let res = rt
        .block_on(bridge.handle_rpc_message(
            "extensions.activate",
            &json!({ "id": "rustcode.sample-hello-world" }),
        ))
        .unwrap();
    assert_eq!(res["activated"], json!(true));
    assert!(
        !bridge.env_manager().take_pending_events().is_empty(),
        "F7 requires extensions.activate to emit ExtEnvEvent / extensions.onDidChange"
    );
    let ext = rt
        .block_on(bridge.handle_rpc_message(
            "extensions.getExtension",
            &json!({ "id": "rustcode.sample-hello-world" }),
        ))
        .unwrap();
    assert_eq!(
        ext["isActive"],
        json!(true),
        "F7 requires extensions.getExtension to reflect isActive == true after activation, got {:?}",
        ext
    );
}

#[test]
fn f7_env_open_external_rejects_relative_path_without_scheme() {
    let mgr = ExtHostEnvManager::new();
    assert!(!mgr.open_external("relative/path/index.html"));
    assert!(!mgr.open_external("C:\\Windows\\System32\\calc.exe"));
}

// ============================================================================
// F8: node-compat/fs.js, path.js & M1 Cross-Platform (Tier 2: Boundary / Error)
// ============================================================================

#[tokio::test]
async fn f8_fs_sandbox_blocks_parent_traversal_and_nul_bytes() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    for bad in ["../outside.txt", "sub/../../outside.txt", "bad\0name.txt", "   "] {
        let err = bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": bad }))
            .await
            .expect_err("expected sandbox/path rejection");
        assert!(
            err.starts_with("EACCES") || err.starts_with("EINVAL"),
            "unexpected error for {:?}: {}",
            bad,
            err
        );
    }
}

#[tokio::test]
async fn f8_fs_sandbox_blocks_drive_relative_paths_on_windows_and_root_escapes() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let outside = std::env::temp_dir().join("rustcode_outside_e2e_probe.txt");
    let err = bridge
        .handle_fs_rpc(
            "fs.readFile",
            &json!({ "path": outside.to_string_lossy().into_owned() }),
        )
        .await
        .expect_err("expected EACCES outside fs_root");
    assert!(err.starts_with("EACCES"), "unexpected err: {}", err);

    #[cfg(windows)]
    {
        let drive_rel = bridge
            .handle_fs_rpc("fs.readFile", &json!({ "path": "C:relative.txt" }))
            .await
            .expect_err("expected drive-relative EACCES on Windows");
        assert!(drive_rel.starts_with("EACCES"), "unexpected err: {}", drive_rel);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn f8_fs_sandbox_blocks_symlink_escape_outside_root() {
    let ws = TempWorkspace::new();
    let outside_dir = TempWorkspace::new();
    let outside_file = outside_dir.write_file("secret.txt", "secret");
    let link_path = ws.path().join("escape_link.txt");
    std::os::unix::fs::symlink(&outside_file, &link_path).unwrap();

    let (bridge, _) = create_bridge_with_root(ws.path());
    let err = bridge
        .handle_fs_rpc("fs.readFile", &json!({ "path": "escape_link.txt" }))
        .await
        .expect_err("expected symlink escape EACCES");
    assert!(err.starts_with("EACCES"), "unexpected err: {}", err);
}

#[tokio::test]
async fn f8_fs_rpc_rejects_missing_path_and_invalid_byte_array_data() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let e1 = bridge
        .handle_fs_rpc("fs.readFile", &json!({}))
        .await
        .expect_err("expected EINVAL");
    assert!(e1.starts_with("EINVAL"));

    let e2 = bridge
        .handle_fs_rpc("fs.writeFile", &json!({ "path": "bad.bin", "data": [999] }))
        .await
        .expect_err("expected EINVAL for out-of-range u8");
    assert!(e2.starts_with("EINVAL"));
}

#[tokio::test]
async fn f8_fs_rpc_missing_dir_and_unsupported_op_errors() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let dir_err = bridge
        .handle_fs_rpc("fs.readdir", &json!({ "path": "missing_dir" }))
        .await
        .expect_err("expected ENOENT for missing dir");
    assert!(dir_err.starts_with("ENOENT"), "unexpected err: {}", dir_err);

    let stat_err = bridge
        .handle_fs_rpc("fs.stat", &json!({ "path": "missing_file.txt" }))
        .await
        .expect_err("expected ENOENT for missing file");
    assert!(stat_err.starts_with("ENOENT"), "unexpected err: {}", stat_err);

    let op_err = bridge
        .handle_fs_rpc("fs.unknownOp", &json!({ "path": "anything.txt" }))
        .await
        .expect_err("expected METHOD_NOT_FOUND or EINVAL for unsupported fs op");
    assert!(
        op_err.starts_with("METHOD_NOT_FOUND") || op_err.starts_with("EINVAL"),
        "unexpected err: {}",
        op_err
    );
}

#[test]
fn f8_platform_fs_helpers_handle_windows_verbatim_and_non_file_uris() {
    assert!(!is_file_uri("https://example.com"));
    assert!(!is_file_uri("C:\\local\\path"));
    let p = PathBuf::from(if cfg!(windows) {
        r"\\?\C:\Users\Dev\Project"
    } else {
        "/tmp/project"
    });
    let stripped = strip_windows_verbatim_prefix(p.clone());
    if cfg!(windows) {
        assert_eq!(stripped, PathBuf::from(r"C:\Users\Dev\Project"));
        assert!(paths_equivalent(&stripped, PathBuf::from(r"c:\users\dev\project").as_path()));
        assert!(path_starts_with(
            PathBuf::from(r"c:\users\dev\project\src").as_path(),
            &stripped
        ));
        assert!(path_strip_prefix(
            PathBuf::from(r"c:\users\dev\project\src\lib.rs").as_path(),
            &stripped
        )
        .is_some());
    } else {
        assert_eq!(file_uri_to_path("file:///tmp/a%20b.rs"), PathBuf::from("/tmp/a b.rs"));
    }
}

// ============================================================================
// F15: Expanded node-compat Polyfills & Rust RPCs (Tier 2: Boundary / Error)
// ============================================================================

#[tokio::test]
async fn f15_process_exec_non_zero_exit_captures_exit_code_and_stderr() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message(
            "process.exec",
            &json!({
                "command": "node -e \"console.error('err-line'); process.exit(3);\"",
                "cwd": ws.path()
            }),
        )
        .await
        .expect("F15 requires process.exec RPC handler");
    assert_eq!(res["exitCode"], json!(3));
    assert!(res["stderr"]
        .as_str()
        .is_some_and(|s| s.contains("err-line")));
}

#[tokio::test]
async fn f15_process_spawn_missing_executable_returns_error() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message(
            "process.spawn",
            &json!({ "command": "rustcode_nonexistent_binary_xyz_12345", "args": [] }),
        )
        .await
        .expect("F15 requires process.spawn RPC handler to return structured error");
    assert!(res.get("error").is_some(), "expected error envelope: {:?}", res);
    assert_ne!(
        res["error"]["code"],
        json!("METHOD_NOT_FOUND"),
        "process.spawn must be implemented, not METHOD_NOT_FOUND: {:?}",
        res
    );
    assert!(
        matches!(
            res["error"]["code"].as_str(),
            Some("ENOENT" | "SPAWN_ERROR" | "EIO" | "RPC_ERROR")
        ),
        "expected spawn error code, got {:?}",
        res
    );
}

#[tokio::test]
async fn f15_process_exec_enforces_timeout() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message(
            "process.exec",
            &json!({
                "command": "node -e \"setTimeout(() => {}, 5000)\"",
                "timeout": 100
            }),
        )
        .await
        .expect("F15 requires process.exec timeout handling");
    if res.get("error").is_some() {
        assert_ne!(
            res["error"]["code"],
            json!("METHOD_NOT_FOUND"),
            "process.exec must be implemented, not METHOD_NOT_FOUND: {:?}",
            res
        );
    }
    assert!(
        res["killed"] == json!(true)
            || res["timedOut"] == json!(true)
            || (res.get("error").is_some() && res["error"]["code"] != json!("METHOD_NOT_FOUND")),
        "expected timeout error or killed/timedOut=true, got {:?}",
        res
    );
}

#[tokio::test]
async fn f15_process_spawn_missing_command_param_returns_einval() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("process.spawn", &json!({}))
        .await
        .expect("F15 requires structured EINVAL error on missing command");
    assert!(res.get("error").is_some(), "expected error envelope: {:?}", res);
    assert!(
        res["error"]["code"] == json!("EINVAL")
            || res["error"]["code"] == json!("INVALID_PARAMS"),
        "expected EINVAL or INVALID_PARAMS error code, got {:?}",
        res
    );
}

#[tokio::test]
async fn f15_process_spawn_sync_non_zero_exit_returns_status() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message(
            "process.spawnSync",
            &json!({
                "command": "node",
                "args": ["-e", "process.exit(7)"]
            }),
        )
        .await
        .expect("F15 requires process.spawnSync RPC handler");
    assert_eq!(res["status"], json!(7));
}

// ============================================================================
// F17: Documentation Refresh Consistency Checks (Tier 2: Negative / Contradiction)
// ============================================================================

#[test]
fn f17_readme_has_zero_unchecked_phase7_milestones() {
    let readme = std::fs::read_to_string(repo_root().join("README.md")).unwrap();
    let phase7_idx = readme
        .find("Phase 7")
        .expect("README.md must mention Phase 7");
    let after_phase7 = &readme[phase7_idx..];
    let phase7_section = after_phase7
        .split("### Future")
        .next()
        .unwrap_or(after_phase7);
    assert!(
        !phase7_section.contains("- [ ]"),
        "README.md still has unchecked Phase 7 items"
    );
}

#[test]
fn f17_implementation_plan_has_zero_unchecked_phase7_milestones_and_no_stale_phase6_counts() {
    let plan = std::fs::read_to_string(repo_root().join("rustcode-implementation-plan.md")).unwrap();
    let tracker_end = plan.find("## ").unwrap_or(plan.len().min(5000));
    let tracker = &plan[..tracker_end];
    assert!(
        !tracker.contains("- [ ]"),
        "rustcode-implementation-plan.md tracker still has unchecked items"
    );
    assert!(
        !plan.contains("404 workspace tests"),
        "rustcode-implementation-plan.md still contains stale '404 workspace tests' count"
    );
}

#[test]
fn f17_architecture_doc_has_no_stale_phases_0_to_6_only_heading() {
    let arch = std::fs::read_to_string(repo_root().join("docs/architecture.md")).unwrap();
    assert!(
        !arch.contains("Subsystem Breakdown (Phases 0–6)")
            && !arch.contains("Subsystem Breakdown (Phases 0-6)"),
        "docs/architecture.md still has stale 'Phases 0-6' subsystem heading"
    );
    assert!(
        arch.contains("NotificationsOverlay") || arch.contains("notifications"),
        "docs/architecture.md must document M3 NotificationsOverlay"
    );
    assert!(
        arch.contains("TestingSidebarView") || arch.contains("testing"),
        "docs/architecture.md must document M3 TestingSidebarView"
    );
}

#[test]
fn f17_extension_compat_matrix_has_no_stale_js_only_stub_contradictions() {
    let matrix =
        std::fs::read_to_string(repo_root().join("docs/extension-compat-matrix.md")).unwrap();
    let lower = matrix.to_ascii_lowercase();
    assert!(
        !lower.contains("js-only stub") && !lower.contains("in-memory stub only"),
        "docs/extension-compat-matrix.md still claims completed namespaces are JS-only/in-memory stubs"
    );
    assert!(
        !matrix.contains("- [ ]"),
        "docs/extension-compat-matrix.md still has unchecked roadmap items"
    );
    assert!(
        !matrix.contains("26/26"),
        "docs/extension-compat-matrix.md still contains stale 26/26 compat_suite count"
    );
}

#[test]
fn f17_docs_referenced_node_compat_files_exist_on_disk() {
    let root = repo_root();
    for name in [
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
        let p = root.join("ext-host-js/node-compat").join(format!("{name}.js"));
        assert!(
            p.is_file(),
            "F17 requires documented node-compat/{name}.js to exist on disk"
        );
    }
}
