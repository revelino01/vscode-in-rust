use phase7_e2e::{
    create_bridge_with_root, repo_root, run_node_script, run_rustcode_cli, E2eWindow,
    TempWorkspace,
};
use rustcode_app::RustCodeContext;
use rustcode_ext_host::{
    ExtensionLoader, LanguageFeatureBridge, NotificationSeverity, ProviderRegistry, ProviderType,
    RpcBridge, RpcMessage,
};
use rustcode_platform::configuration::ConfigurationTarget;
use rustcode_platform::fs::{
    file_uri_to_path, is_file_uri, path_starts_with, path_strip_prefix, path_to_file_uri,
    paths_equivalent, strip_windows_verbatim_prefix, FileSystemService,
};
use rustcode_protocols::lsp::LspClientManager;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ============================================================================
// F1: Live Rust <-> JS RPC Transport (Tier 1)
// ============================================================================

#[test]
fn f1_rpc_message_json_roundtrip_request_response_notification() {
    let req = RpcMessage::Request {
        id: 7,
        method: "workspace.getConfiguration".to_string(),
        params: json!({ "section": "editor" }),
    };
    let req_json = req.to_json().expect("request to_json");
    assert_eq!(RpcMessage::from_json(&req_json).expect("from_json"), req);

    let resp = RpcMessage::Response {
        id: 7,
        result: Some(json!({ "fontSize": 14 })),
        error: None,
    };
    let resp_json = resp.to_json().expect("response to_json");
    assert_eq!(RpcMessage::from_json(&resp_json).expect("from_json"), resp);

    let notif = RpcMessage::Notification {
        channel: "workspace.onDidChangeConfiguration".to_string(),
        data: json!({ "affectedKeys": ["editor.fontSize"] }),
    };
    let notif_json = notif.to_json().expect("notification to_json");
    assert_eq!(
        RpcMessage::from_json(&notif_json).expect("from_json"),
        notif
    );
}

#[test]
fn f1_rpc_message_msgpack_binary_roundtrip_preserves_unicode_and_nested_params() {
    let msg = RpcMessage::Request {
        id: 101,
        method: "window.showInformationMessage".to_string(),
        params: json!({
            "message": "café 🚀 rustcode",
            "items": [{ "title": "Save" }, { "title": "Cancel" }]
        }),
    };
    let bytes = msg.to_msgpack().expect("to_msgpack");
    let decoded = RpcMessage::from_msgpack(&bytes).expect("from_msgpack");
    assert_eq!(decoded, msg);
}

#[tokio::test]
async fn f1_rpc_bridge_concurrent_requests_correlate_out_of_order_responses() {
    let rpc = RpcBridge::new();
    let (req1, rx1) = rpc.create_request("m1", json!({ "n": 1 }));
    let (req2, rx2) = rpc.create_request("m2", json!({ "n": 2 }));
    assert_eq!(rpc.pending_count(), 2);

    let id1 = match req1 {
        RpcMessage::Request { id, .. } => id,
        _ => panic!("expected request"),
    };
    let id2 = match req2 {
        RpcMessage::Request { id, .. } => id,
        _ => panic!("expected request"),
    };

    assert!(rpc.handle_incoming_response(id2, Some(json!("second")), None));
    assert!(rpc.handle_incoming_response(id1, Some(json!("first")), None));
    assert_eq!(rpc.pending_count(), 0);
    assert_eq!(rx1.await.unwrap().unwrap(), json!("first"));
    assert_eq!(rx2.await.unwrap().unwrap(), json!("second"));
}

#[test]
fn f1_rpc_bridge_notification_does_not_increment_pending_count() {
    let rpc = RpcBridge::new();
    let msg = rpc.create_notification("extensions.onDidChange", json!([]));
    assert_eq!(rpc.pending_count(), 0);
    assert!(matches!(msg, RpcMessage::Notification { .. }));
}

#[test]
fn f1_js_ext_host_rpc_roundtrip_via_node_subprocess() {
    let root = repo_root();
    let script = r#"
const path = require('path');
const assert = require('assert/strict');
const calls = [];
globalThis.Deno = {
  core: {
    ops: {
      op_rpc_call: async (method, raw) => {
        calls.push({ method, params: JSON.parse(raw) });
        if (method === 'env.clipboardRead') return JSON.stringify('ok-clip');
        return JSON.stringify(null);
      },
      op_rpc_call_sync: (method) => {
        if (method === 'extensions.getAll') return JSON.stringify([]);
        return JSON.stringify({});
      }
    }
  }
};
const rpc = require('./ext-host-js/rpc-bridge.js');
const vscode = require('./ext-host-js/vscode-api/index.js');
(async () => {
  const disp = vscode.commands.registerCommand('node.ping', (x) => `pong:${x}`);
  const out = await rpc.handleIncomingRequest('commands.executeCommand', { command: 'node.ping', args: [7] });
  assert.equal(out, 'pong:7');
  assert.equal(await vscode.env.clipboard.readText(), 'ok-clip');
  disp.dispose();
  console.log('NODE_RPC_OK');
})();
"#;
    let output = run_node_script(script, &root);
    assert!(
        output.status.success(),
        "node script failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("NODE_RPC_OK"));
}

// ============================================================================
// F2: RPC Error Propagation (Tier 1)
// ============================================================================

#[tokio::test]
async fn f2_handle_rpc_message_fs_read_file_returns_content_on_success() {
    let ws = TempWorkspace::new();
    ws.write_file("hello.txt", "hello phase7");
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("fs.readFile", &json!({ "path": "hello.txt", "encoding": "utf8" }))
        .await;
    assert_eq!(res, Some(json!("hello phase7")));
}

#[tokio::test]
async fn f2_handle_rpc_message_fs_missing_file_returns_structured_enoent_error() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("fs.readFile", &json!({ "path": "does_not_exist.txt" }))
        .await
        .expect("F2 requires structured { error } instead of swallowing Err via .ok()");
    assert_eq!(res["error"]["code"], json!("ENOENT"));
    assert!(
        res["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "expected non-empty error.message: {:?}",
        res
    );
}

#[tokio::test]
async fn f2_handle_rpc_message_commands_unknown_returns_structured_error() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message(
            "commands.executeCommand",
            &json!({ "command": "no.such.command", "args": [] }),
        )
        .await
        .expect("F2 requires structured { error } for unknown command");
    assert!(res.get("error").is_some(), "expected error envelope: {:?}", res);
    assert!(res["error"]["message"].as_str().is_some_and(|s| !s.is_empty()));
}

#[tokio::test]
async fn f2_handle_rpc_message_env_clipboard_write_missing_value_returns_structured_error() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("env.clipboardWrite", &json!({}))
        .await
        .expect("F2 requires structured { error } on missing clipboard value");
    assert!(res.get("error").is_some(), "expected error envelope: {:?}", res);
    assert!(res["error"]["code"].as_str().is_some());
}

#[tokio::test]
async fn f2_handle_rpc_message_extensions_missing_id_returns_structured_error() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("extensions.getExtension", &json!({}))
        .await
        .expect("F2 requires structured { error } on missing extension id");
    assert!(res.get("error").is_some(), "expected error envelope: {:?}", res);
}

// ============================================================================
// F3: RustCodeContext & WorkbenchWindow Bridge Wiring (Tier 1)
// ============================================================================

#[test]
fn f3_create_window_attaches_language_bridge_with_shared_services() {
    let app = RustCodeContext::new();
    let window = app.create_window();
    let bridge = window
        .language_bridge
        .as_ref()
        .expect("F3 requires RustCodeContext::create_window() to attach language_bridge");

    app.config
        .update("editor.fontSize", json!(20), ConfigurationTarget::User)
        .expect("config update");
    assert_eq!(
        bridge.configuration_service().get::<i64>("editor.fontSize"),
        Some(20)
    );

    let mut disp = app
        .commands
        .register("e2e.sharedCmd", |_| Ok(json!({ "shared": true })));
    assert!(bridge.command_registry().has_command("e2e.sharedCmd"));
    disp.dispose();
}

#[test]
fn f3_create_window_hydrates_extensions_from_context_loader() {
    let mut app = RustCodeContext::new();
    let sample_dir = repo_root().join("resources/sample-extensions/hello-world");
    app.ext_loader
        .load_from_dir(&sample_dir)
        .expect("load sample hello-world extension");

    let window = app.create_window();
    let bridge = window
        .language_bridge
        .as_ref()
        .expect("create_window must attach language_bridge");
    let loaded = bridge.env_manager().list_extensions();
    assert!(
        loaded
            .iter()
            .any(|e| e.id.eq_ignore_ascii_case("rustcode.sample-hello-world")),
        "expected sample extension hydrated into env_manager, got {:?}",
        loaded
    );
}

#[tokio::test]
async fn f3_window_document_lifecycle_open_edit_close_syncs_bridge() {
    let ws = TempWorkspace::new();
    let file_path = ws.write_file("main.rs", "fn main() {}\n");
    let uri = path_to_file_uri(&file_path);

    let mut win = E2eWindow::with_temp_workspace(&ws);
    assert!(
        win.window.language_bridge.is_some(),
        "F3 requires create_window() to attach language_bridge"
    );

    win.open_file_from_disk("main.rs");
    win.type_text("// edited\n");
    let _ = win.render();

    let bridge = win.window.language_bridge.as_ref().unwrap().clone();
    let opened = bridge
        .handle_rpc_message("workspace.openTextDocument", &json!({ "uri": uri }))
        .await
        .expect("openTextDocument should return document snapshot");
    let text = opened["text"].as_str().unwrap_or("");
    assert!(
        text.contains("// edited"),
        "expected open_documents in bridge to reflect live editor edit, got: {:?}",
        opened
    );

    win.dispatch_ctrl_char('w');
    let _ = win.render();
}

#[test]
fn f3_rustcode_context_sync_helpers_do_not_panic_and_render_scene() {
    let mut win = E2eWindow::new();
    win.window.sync_output_channels();
    win.window.sync_ext_terminals();
    win.window.sync_custom_editors();
    win.window.sync_webviews();
    let scene = win.render();
    assert!(
        scene.contains_text("EXPLORER") && !scene.glyph_runs().is_empty(),
        "expected Explorer UI in rendered scene after sync helpers, got: {}",
        scene.all_text()
    );
}

#[test]
fn f3_rustcode_cli_binary_bootstraps_and_exits_zero() {
    let output = run_rustcode_cli();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "rustcode CLI exited with {:?}, stderr={}",
        output.status.code(),
        stderr
    );
    assert!(stdout.contains("Rendered GPUI Scene:"), "stdout={}", stdout);
    assert!(
        stdout.contains("Bootstrap completed successfully."),
        "stdout={}",
        stdout
    );
}

// ============================================================================
// F4: Bidirectional commands.* Routing (Tier 1)
// ============================================================================

#[test]
fn f4_builtin_toggle_sidebar_via_command_registry_mutates_window_layout() {
    let mut win = E2eWindow::new();
    let initial = win.window.view.layout.sidebar_visible;

    let res = win
        .app
        .commands
        .execute("workbench.action.toggleSidebar", None)
        .expect("toggleSidebar command registered");
    assert!(res.is_object());
    let _ = win.render();

    assert_ne!(
        win.window.view.layout.sidebar_visible, initial,
        "F4 requires CommandRegistry::execute('workbench.action.toggleSidebar') to mutate WorkbenchWindow layout"
    );
}

#[test]
fn f4_builtin_toggle_panel_and_open_settings_via_command_registry_mutate_window() {
    let mut win = E2eWindow::new();
    let initial_panel = win.window.view.layout.panel_visible;

    win.app
        .commands
        .execute("workbench.action.togglePanel", None)
        .expect("togglePanel");
    win.app
        .commands
        .execute("workbench.action.openSettings", None)
        .expect("openSettings");
    let scene = win.render();

    assert_ne!(win.window.view.layout.panel_visible, initial_panel);
    assert!(
        scene.contains_text("Settings"),
        "expected openSettings via CommandRegistry to open Settings view"
    );
}

#[tokio::test]
async fn f4_js_command_registration_and_request_execute_command_roundtrip() {
    let ws = TempWorkspace::new();
    let (bridge, rpc) = create_bridge_with_root(ws.path());

    let reg = bridge
        .handle_rpc_message(
            "commands.registerCommand",
            &json!({ "command": "ext.greet" }),
        )
        .await
        .expect("registerCommand");
    assert_eq!(reg["registered"], json!(true));

    let bridge_clone = Arc::clone(&bridge);
    let task = tokio::spawn(async move {
        bridge_clone
            .request_execute_command("ext.greet", vec![json!("RustCode")])
            .await
    });

    tokio::task::yield_now().await;
    assert_eq!(rpc.pending_count(), 1);
    assert!(rpc.handle_incoming_response(1, Some(json!("Hello, RustCode!")), None));

    let result = task.await.unwrap().expect("request_execute_command");
    assert_eq!(result, json!("Hello, RustCode!"));
}

#[tokio::test]
async fn f4_commands_execute_command_rpc_invokes_rust_command_and_returns_result() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());
    let _disp = bridge.command_registry().register("rust.sum", |args| {
        let arr = args
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let sum: i64 = arr.iter().filter_map(|x| x.as_i64()).sum();
        Ok(json!(sum))
    });

    let res = bridge
        .handle_rpc_message(
            "commands.executeCommand",
            &json!({ "command": "rust.sum", "args": [10, 15, 17] }),
        )
        .await;
    assert_eq!(res, Some(json!(42)));
}

#[tokio::test]
async fn f4_commands_get_commands_filters_internal_underscore_commands() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());
    let _d1 = bridge
        .command_registry()
        .register("public.action", |_| Ok(json!(true)));
    let _d2 = bridge
        .command_registry()
        .register("_internal.hidden", |_| Ok(json!(true)));

    let all = bridge
        .handle_rpc_message("commands.getCommands", &json!({ "filterInternal": false }))
        .await
        .unwrap();
    let filtered = bridge
        .handle_rpc_message("commands.getCommands", &json!({ "filterInternal": true }))
        .await
        .unwrap();

    let all_arr = all.as_array().unwrap();
    let filt_arr = filtered.as_array().unwrap();
    assert!(all_arr.iter().any(|v| v == "_internal.hidden"));
    assert!(!filt_arr.iter().any(|v| v == "_internal.hidden"));
    assert!(filt_arr.iter().any(|v| v == "public.action"));
}

// ============================================================================
// F5: window.* Prompts, Editors & TreeDataProvider (Tier 1)
// ============================================================================

#[tokio::test]
async fn f5_window_show_messages_record_severity_and_resolve_chosen_action() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());
    bridge
        .window_manager()
        .set_notification_responder(|notif| Some(json!(notif.items[0].title)));

    let info = bridge
        .handle_rpc_message(
            "window.showInformationMessage",
            &json!({ "message": "Info msg", "items": ["Ack"] }),
        )
        .await;
    assert_eq!(info, Some(json!("Ack")));

    let warn = bridge
        .handle_rpc_message(
            "window.showWarningMessage",
            &json!({ "message": "Warn msg", "items": ["Fix"] }),
        )
        .await;
    assert_eq!(warn, Some(json!("Fix")));

    let err = bridge
        .handle_rpc_message(
            "window.showErrorMessage",
            &json!({ "message": "Err msg", "items": ["Retry"] }),
        )
        .await;
    assert_eq!(err, Some(json!("Retry")));

    let all = bridge.window_manager().list_notifications();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].severity, NotificationSeverity::Info);
    assert_eq!(all[1].severity, NotificationSeverity::Warning);
    assert_eq!(all[2].severity, NotificationSeverity::Error);
}

#[tokio::test]
async fn f5_window_show_quick_pick_single_and_multi_select() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());
    bridge
        .window_manager()
        .set_prompt_responder(|prompt| match &prompt.kind {
            rustcode_ext_host::ExtPromptKind::QuickPick { can_pick_many, .. } => {
                if *can_pick_many {
                    Some(json!(["A", "C"]))
                } else {
                    Some(json!("B"))
                }
            }
            _ => None,
        });

    let single = bridge
        .handle_rpc_message(
            "window.showQuickPick",
            &json!({ "items": ["A", "B", "C"], "options": {} }),
        )
        .await
        .unwrap();
    assert_eq!(single["label"], json!("B"));

    let multi = bridge
        .handle_rpc_message(
            "window.showQuickPick",
            &json!({ "items": ["A", "B", "C"], "options": { "canPickMany": true } }),
        )
        .await
        .unwrap();
    let arr = multi.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["label"], json!("A"));
    assert_eq!(arr[1]["label"], json!("C"));
}

#[tokio::test]
async fn f5_window_show_input_box_distinguishes_empty_string_from_cancel() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    bridge
        .window_manager()
        .set_prompt_responder(|_| Some(json!("")));
    let empty_ok = bridge
        .handle_rpc_message(
            "window.showInputBox",
            &json!({ "options": { "prompt": "Filter" } }),
        )
        .await;
    assert_eq!(empty_ok, Some(json!("")));

    bridge.window_manager().set_prompt_responder(|_| None);
    let cancelled = bridge
        .handle_rpc_message(
            "window.showInputBox",
            &json!({ "options": { "prompt": "Filter" } }),
        )
        .await;
    assert_eq!(cancelled, Some(Value::Null));
}

#[tokio::test]
async fn f5_tree_data_provider_and_alias_register_and_fetch_children() {
    let ws = TempWorkspace::new();
    let (bridge, rpc) = create_bridge_with_root(ws.path());

    let r1 = bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TreeDataProvider", "id": "view.one" }),
        )
        .await;
    assert_eq!(r1.unwrap()["registered"], json!(true));

    let r2 = bridge
        .handle_rpc_message(
            "extHost.registerProvider",
            &json!({ "type": "TreeData", "id": "view.two" }),
        )
        .await;
    assert_eq!(r2.unwrap()["registered"], json!(true));
    assert!(bridge
        .provider_registry
        .has_provider_with_id(ProviderType::TreeData, "view.one"));
    assert!(bridge
        .provider_registry
        .has_provider_with_id(ProviderType::TreeData, "view.two"));

    let b = Arc::clone(&bridge);
    let task = tokio::spawn(async move { b.request_tree_children("view.one", None).await });
    tokio::task::yield_now().await;
    assert!(rpc.handle_incoming_response(
        1,
        Some(json!([{ "id": "root", "label": "src", "collapsibleState": 1 }])),
        None
    ));
    let items = task.await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, "src");
}

#[tokio::test]
async fn f5_window_show_text_document_rpc_opens_and_returns_editor_state() {
    let ws = TempWorkspace::new();
    let file_path = ws.write_file("hello.rs", "fn hello() {}\n");
    let uri = path_to_file_uri(&file_path);
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("window.showTextDocument", &json!({ "uri": uri }))
        .await
        .expect("F5 requires window.showTextDocument RPC handler");
    assert_eq!(res["document"]["languageId"], json!("rust"));
    assert!(res["document"]["text"]
        .as_str()
        .is_some_and(|t| t.contains("fn hello")));
}

// ============================================================================
// F6: workspace.* Hydration, Config & Events (Tier 1)
// ============================================================================

#[tokio::test]
async fn f6_workspace_get_configuration_and_inspect_across_scopes() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());
    bridge
        .configuration_service()
        .update("editor.tabSize", json!(2), ConfigurationTarget::Workspace)
        .unwrap();

    let cfg = bridge
        .handle_rpc_message("workspace.getConfiguration", &json!({ "section": "editor" }))
        .await
        .unwrap();
    assert_eq!(cfg["tabSize"], json!(2));

    let inspect = bridge
        .handle_rpc_message(
            "workspace.inspectConfiguration",
            &json!({ "key": "editor.tabSize" }),
        )
        .await
        .unwrap();
    assert_eq!(inspect["defaultValue"], json!(4));
    assert_eq!(inspect["workspaceValue"], json!(2));
}

#[tokio::test]
async fn f6_workspace_update_configuration_mutates_service_and_fires_event() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen_clone = Arc::clone(&seen);
    let mut disp = bridge
        .configuration_service()
        .on_did_change
        .event()
        .attach(move |ev| {
            seen_clone.lock().unwrap().extend(ev.affected_keys.clone());
        });

    let res = bridge
        .handle_rpc_message(
            "workspace.updateConfiguration",
            &json!({ "key": "editor.minimap.enabled", "value": false, "target": 1 }),
        )
        .await
        .unwrap();
    assert_eq!(res["updated"], json!(true));
    assert_eq!(
        bridge
            .configuration_service()
            .get::<bool>("editor.minimap.enabled"),
        Some(false)
    );
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .any(|k| k == "editor.minimap.enabled"));
    disp.dispose();
}

#[tokio::test]
async fn f6_workspace_open_text_document_disk_crlf_and_untitled() {
    let ws = TempWorkspace::new();
    let file_path = ws.write_file("crlf.rs", "line1\r\nline2\r\n");
    let uri = path_to_file_uri(&file_path);
    let (bridge, _) = create_bridge_with_root(ws.path());

    let disk_doc = bridge
        .handle_rpc_message("workspace.openTextDocument", &json!({ "uri": uri }))
        .await
        .unwrap();
    assert_eq!(disk_doc["languageId"], json!("rust"));
    assert_eq!(disk_doc["eol"], json!(2));
    assert_eq!(disk_doc["lineCount"], json!(3));

    let untitled = bridge
        .handle_rpc_message(
            "workspace.openTextDocument",
            &json!({ "language": "json", "content": "{}" }),
        )
        .await
        .unwrap();
    assert_eq!(untitled["isUntitled"], json!(true));
    assert_eq!(untitled["languageId"], json!("json"));
    assert_eq!(untitled["text"], json!("{}"));
}

#[tokio::test]
async fn f6_workspace_open_text_document_registers_doc_in_open_documents() {
    let ws = TempWorkspace::new();
    let file_path = ws.write_file("tracked.txt", "v1");
    let uri = path_to_file_uri(&file_path);
    let (bridge, _) = create_bridge_with_root(ws.path());

    let first = bridge
        .handle_rpc_message("workspace.openTextDocument", &json!({ "uri": uri }))
        .await
        .unwrap();
    assert_eq!(first["text"], json!("v1"));

    // Overwrite disk file directly: if openTextDocument registered the document in
    // open_documents, reopening it returns the live open buffer ("v1") rather than disk.
    std::fs::write(&file_path, "disk-overwrite").unwrap();
    let second = bridge
        .handle_rpc_message("workspace.openTextDocument", &json!({ "uri": uri }))
        .await
        .unwrap();
    assert_eq!(
        second["text"],
        json!("v1"),
        "F6 requires workspace.openTextDocument to register opened docs into open_documents"
    );
}

#[tokio::test]
async fn f6_workspace_get_workspace_folders_rpc_returns_hydrated_folders() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message("workspace.getWorkspaceFolders", &json!({}))
        .await
        .expect("F6 requires workspace.getWorkspaceFolders RPC handler");
    assert!(res.is_array(), "expected array of WorkspaceFolder, got {:?}", res);
}

// ============================================================================
// F7: env.* & extensions.* Real Backends (Tier 1)
// ============================================================================

#[tokio::test]
async fn f7_env_clipboard_write_and_read_roundtrip_unicode() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let wr = bridge
        .handle_rpc_message("env.clipboardWrite", &json!({ "value": "RustCode 📋 café" }))
        .await
        .unwrap();
    assert_eq!(wr["written"], json!(true));

    let rd = bridge
        .handle_rpc_message("env.clipboardRead", &json!({}))
        .await
        .unwrap();
    assert_eq!(rd, json!("RustCode 📋 café"));
}

#[tokio::test]
async fn f7_env_open_external_accepts_allowlisted_schemes() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    for uri in [
        "https://rustcode.dev/docs",
        "http://localhost:3000",
        "mailto:dev@rustcode.dev",
        "file:///workspace/README.md",
        "vscode://settings/editor.fontSize",
        "rustcode://open/sample",
    ] {
        let ok = bridge
            .handle_rpc_message("env.openExternal", &json!({ "uri": uri }))
            .await
            .unwrap();
        assert_eq!(ok, json!(true), "expected valid URI allowed: {}", uri);
    }
    assert_eq!(bridge.env_manager().opened_external_uris().len(), 6);
}

#[tokio::test]
async fn f7_extensions_activate_syncs_extension_loader_and_case_insensitive_lookup() {
    let sample_dir = repo_root().join("resources/sample-extensions/hello-world");
    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&sample_dir).unwrap();

    let registry = Arc::new(ProviderRegistry::new());
    let rpc = Arc::new(RpcBridge::new());
    let lsp = Arc::new(LspClientManager::new());
    let bridge = LanguageFeatureBridge::new(registry, rpc, lsp).with_extension_loader(&loader);

    let ext = bridge
        .handle_rpc_message(
            "extensions.getExtension",
            &json!({ "id": "RUSTCODE.SAMPLE-HELLO-WORLD" }),
        )
        .await
        .unwrap();
    assert_eq!(ext["id"], json!("rustcode.sample-hello-world"));
    assert_eq!(ext["isActive"], json!(false));

    let act = bridge
        .handle_rpc_message(
            "extensions.activate",
            &json!({ "id": "rustcode.sample-hello-world" }),
        )
        .await
        .unwrap();
    assert_eq!(act["activated"], json!(true));

    let all = bridge
        .handle_rpc_message("extensions.getAll", &json!({}))
        .await
        .unwrap();
    assert_eq!(all[0]["isActive"], json!(true));
}

#[tokio::test]
async fn f7_env_get_environment_rpc_returns_cross_platform_metadata() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let env_info = bridge
        .handle_rpc_message("env.getEnvironment", &json!({}))
        .await
        .expect("F7 requires env.getEnvironment RPC handler");
    assert!(env_info["appRoot"]
        .as_str()
        .is_some_and(|s| !s.is_empty() && s != "/rustcode"));
    assert!(env_info["shell"].as_str().is_some_and(|s| !s.is_empty()));
    assert_eq!(env_info["uriScheme"], json!("rustcode"));
}

#[test]
fn f7_env_manager_hydrate_from_loader_populates_snapshots_and_events() {
    let sample_dir = repo_root().join("resources/sample-extensions/hello-world");
    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&sample_dir).unwrap();

    let mgr = rustcode_ext_host::ExtHostEnvManager::new();
    mgr.hydrate_from_loader(&loader);
    assert_eq!(mgr.list_extensions().len(), 1);
    assert!(!mgr.take_pending_events().is_empty());
}

// ============================================================================
// F8: node-compat/fs.js, path.js & M1 Cross-Platform Fixes (Tier 1)
// ============================================================================

#[test]
fn f8_platform_fs_uri_and_path_helpers_cross_platform_roundtrip() {
    let ws = TempWorkspace::new();
    let file = ws.write_file("sub/file with space.rs", "fn x() {}");
    let uri = path_to_file_uri(&file);
    assert!(is_file_uri(&uri));
    let decoded = file_uri_to_path(&uri);
    assert!(paths_equivalent(&decoded, &file));
    assert!(path_starts_with(&file, ws.path()));
    let rel = path_strip_prefix(&file, ws.path()).expect("strip_prefix");
    assert_eq!(rel, PathBuf::from("sub").join("file with space.rs"));
    let _ = strip_windows_verbatim_prefix(file);
}

#[tokio::test]
async fn f8_fs_rpc_write_read_stat_unlink_and_change_event() {
    let ws = TempWorkspace::new();

    let fs_svc = FileSystemService::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_cl = Arc::clone(&events);
    let mut disp = fs_svc.on_did_change.event().attach(move |ev| {
        events_cl.lock().unwrap().push(ev.path.clone());
    });
    let bridge = LanguageFeatureBridge::new(
        Arc::new(ProviderRegistry::new()),
        Arc::new(RpcBridge::new()),
        Arc::new(LspClientManager::new()),
    )
    .with_file_system_service(Arc::new(fs_svc))
    .with_fs_root(ws.path());

    let wr = bridge
        .handle_fs_rpc("fs.writeFile", &json!({ "path": "a.txt", "data": "abc" }))
        .await
        .unwrap();
    assert_eq!(wr["written"], json!(true));
    assert!(!events.lock().unwrap().is_empty());

    let rd = bridge
        .handle_fs_rpc("fs.readFile", &json!({ "path": "a.txt", "encoding": "utf8" }))
        .await
        .unwrap();
    assert_eq!(rd, json!("abc"));

    let st = bridge
        .handle_fs_rpc("fs.stat", &json!({ "path": "a.txt" }))
        .await
        .unwrap();
    assert_eq!(st["isFile"], json!(true));
    assert_eq!(st["size"], json!(3));

    bridge
        .handle_fs_rpc("fs.unlink", &json!({ "path": "a.txt" }))
        .await
        .unwrap();
    assert!(!ws.exists("a.txt"));
    disp.dispose();
}

#[tokio::test]
async fn f8_fs_rpc_readdir_supports_with_file_types_dirents() {
    let ws = TempWorkspace::new();
    ws.write_file("dir/sub/inner.txt", "hi");
    ws.write_file("dir/root.txt", "hello");
    let (bridge, _) = create_bridge_with_root(ws.path());

    let entries = bridge
        .handle_fs_rpc(
            "fs.readdir",
            &json!({ "path": "dir", "withFileTypes": true }),
        )
        .await
        .unwrap();
    let arr = entries.as_array().expect("expected array");
    assert_eq!(arr.len(), 2);
    assert!(
        arr.iter().any(|e| e["name"] == "sub" && e["isDirectory"] == true),
        "F8 requires fs.readdir withFileTypes=true to return Dirent objects, got {:?}",
        arr
    );
}

#[tokio::test]
async fn f8_fs_rpc_mkdir_recursive_creates_nested_directories() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    bridge
        .handle_fs_rpc(
            "fs.mkdir",
            &json!({ "path": "nested/deep/dir", "options": { "recursive": true } }),
        )
        .await
        .unwrap();
    assert!(ws.path().join("nested").join("deep").join("dir").is_dir());
}

#[tokio::test]
async fn f8_fs_rpc_binary_write_and_read_roundtrip() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let payload = [0u8, 1, 2, 127, 255];
    bridge
        .handle_fs_rpc("fs.writeFile", &json!({ "path": "bin.dat", "data": payload }))
        .await
        .unwrap();
    let read_back = bridge
        .handle_fs_rpc("fs.readFile", &json!({ "path": "bin.dat", "encoding": null }))
        .await
        .unwrap();
    assert_eq!(read_back, json!([0, 1, 2, 127, 255]));
}

// ============================================================================
// F15: Expanded node-compat Polyfills & Rust RPCs (Tier 1)
// ============================================================================

#[tokio::test]
async fn f15_env_get_os_info_rpc_returns_valid_host_metadata() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let info = bridge
        .handle_rpc_message("env.getOsInfo", &json!({}))
        .await
        .expect("F15 requires env.getOsInfo RPC handler");
    assert!(info["platform"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(info["arch"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(info["homedir"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(info["tmpdir"].as_str().is_some_and(|s| !s.is_empty()));
    let expected_eol = if cfg!(windows) { "\r\n" } else { "\n" };
    assert_eq!(info["eol"], json!(expected_eol));
}

#[tokio::test]
async fn f15_process_exec_rpc_runs_cross_platform_command_and_captures_stdout() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message(
            "process.exec",
            &json!({ "command": "node -e \"console.log('e2e-exec-ok')\"", "cwd": ws.path() }),
        )
        .await
        .expect("F15 requires process.exec RPC handler");
    assert_eq!(res["exitCode"], json!(0));
    assert!(res["stdout"]
        .as_str()
        .is_some_and(|s| s.contains("e2e-exec-ok")));
}

#[tokio::test]
async fn f15_process_spawn_rpc_runs_with_args_and_env() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message(
            "process.spawn",
            &json!({
                "command": "node",
                "args": ["-e", "console.log(process.env.E2E_VAR)"],
                "cwd": ws.path(),
                "env": { "E2E_VAR": "spawn-env-42" }
            }),
        )
        .await
        .expect("F15 requires process.spawn RPC handler");
    assert_eq!(res["exitCode"], json!(0));
    assert!(res["stdout"]
        .as_str()
        .is_some_and(|s| s.contains("spawn-env-42")));
}

#[tokio::test]
async fn f15_process_spawn_sync_rpc_returns_status_and_stdout() {
    let ws = TempWorkspace::new();
    let (bridge, _) = create_bridge_with_root(ws.path());

    let res = bridge
        .handle_rpc_message(
            "process.spawnSync",
            &json!({
                "command": "node",
                "args": ["-e", "process.stdout.write('sync-ok')"],
                "cwd": ws.path()
            }),
        )
        .await
        .expect("F15 requires process.spawnSync RPC handler");
    assert_eq!(res["status"], json!(0));
    assert_eq!(res["stdout"], json!("sync-ok"));
}

#[test]
fn f15_all_six_m5_node_compat_modules_exist_and_load_in_node() {
    let root = repo_root();
    for name in ["buffer", "url", "util", "os", "crypto", "child_process"] {
        let p = root.join("ext-host-js/node-compat").join(format!("{name}.js"));
        assert!(p.is_file(), "missing M5 polyfill module: {}", p.display());
    }
    let script = r#"
const path = require('path');
for (const m of ['buffer', 'url', 'util', 'os', 'crypto', 'child_process']) {
  require(path.resolve('ext-host-js/node-compat', m + '.js'));
}
console.log('ALL_6_LOADED');
"#;
    let out = run_node_script(script, &root);
    assert!(
        out.status.success(),
        "loading 6 node-compat modules failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ALL_6_LOADED"));
}

// ============================================================================
// F17: Documentation Refresh (Tier 1)
// ============================================================================

#[test]
fn f17_docs_document_all_9_node_compat_modules() {
    let root = repo_root();
    let arch = std::fs::read_to_string(root.join("docs/architecture.md")).expect("read architecture.md");
    let matrix =
        std::fs::read_to_string(root.join("docs/extension-compat-matrix.md")).expect("read matrix");

    for module_name in [
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
            arch.contains(module_name),
            "docs/architecture.md must document node-compat module '{}'",
            module_name
        );
        assert!(
            matrix.contains(module_name),
            "docs/extension-compat-matrix.md must document node-compat module '{}'",
            module_name
        );
    }
}

#[test]
fn f17_docs_document_all_14_syntax_grammars() {
    let root = repo_root();
    let arch = std::fs::read_to_string(root.join("docs/architecture.md"))
        .unwrap()
        .to_ascii_lowercase();

    for grammar in [
        "rust",
        "javascript",
        "typescript",
        "python",
        "json",
        "markdown",
        "plaintext",
        "go",
        "c++",
        "html",
        "css",
        "toml",
        "yaml",
    ] {
        assert!(
            arch.contains(grammar),
            "docs/architecture.md must document grammar '{}'",
            grammar
        );
    }
    assert!(
        arch.contains(" c,") || arch.contains("`c`") || arch.contains("c/c++"),
        "docs/architecture.md must document C grammar"
    );
}

#[test]
fn f17_docs_document_phase7_commands_and_keybindings() {
    let root = repo_root();
    let arch = std::fs::read_to_string(root.join("docs/architecture.md")).unwrap();
    for cmd in [
        "actions.find",
        "editor.action.startFindReplaceAction",
        "workbench.action.gotoLine",
        "workbench.action.gotoSymbol",
        "workbench.view.testing",
    ] {
        assert!(
            arch.contains(cmd),
            "docs/architecture.md must document Phase 7 command '{}'",
            cmd
        );
    }
}

#[test]
fn f17_docs_document_windows_and_linux_build_and_packaging() {
    let root = repo_root();
    let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
    assert!(readme.contains("package.sh"), "README.md must mention package.sh");
    assert!(
        readme.contains("package.ps1") || readme.to_ascii_lowercase().contains("windows"),
        "README.md must document Windows build/packaging"
    );
    assert!(
        readme.contains("x86_64-unknown-linux-gnu") || readme.to_ascii_lowercase().contains("linux"),
        "README.md must document Linux build/target"
    );
}

#[test]
fn f17_all_four_phase7_docs_exist_and_are_non_empty() {
    let root = repo_root();
    for rel in [
        "README.md",
        "rustcode-implementation-plan.md",
        "docs/architecture.md",
        "docs/extension-compat-matrix.md",
    ] {
        let content = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("failed to read {}: {}", rel, e));
        assert!(content.len() > 500, "{} unexpectedly short", rel);
    }
}
