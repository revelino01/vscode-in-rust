use parking_lot::RwLock;
use phase10_ext_compat::{repo_root, TempTestDir};
use rustcode_app::runtime::{RuntimeLoop, RuntimeLoopConfig};
use rustcode_app::RustCodeContext;
use rustcode_ext_host::manifest::ExtensionLoader;
use rustcode_ext_host::profiler::ExtHostProfiler;
use rustcode_ext_host::provider::LanguageFeatureBridge;
use rustcode_ext_host::runtime_health::{
    inspect_node_runtime, parse_node_version, ExtHostWatchdog, NodeVersion, WatchdogHealth,
    MINIMUM_NODE_MAJOR,
};
use rustcode_ext_host::{NodeExtHostConfig, NodeExtHostProcess, RpcWireFormat};
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn test_tier2_profiler_watchdog_and_node_runtime_gate() {
    assert_eq!(MINIMUM_NODE_MAJOR, 18);
    assert_eq!(
        parse_node_version("v20.11.1"),
        Some(NodeVersion {
            major: 20,
            minor: 11,
            patch: 1
        })
    );
    assert!(parse_node_version("v16.20.2")
        .is_some_and(|v| v.major < MINIMUM_NODE_MAJOR));

    let node_status = inspect_node_runtime(None, None);
    if node_status.binary_path.is_some() {
        assert!(
            node_status.meets_minimum,
            "Host Node.js runtime must satisfy >=v{}: {:?}",
            MINIMUM_NODE_MAJOR, node_status
        );
    }

    let profiler = ExtHostProfiler::new();
    profiler.begin_activation("esbenp.prettier-vscode", "*");
    std::thread::sleep(Duration::from_millis(5));
    let rec = profiler.finish_activation("esbenp.prettier-vscode", true, None);
    assert!(rec.succeeded);

    profiler.record_rpc_call("languages.provideDocumentFormattingEdits", Duration::from_micros(450), true);
    profiler.record_rpc_call("vfs.readFile", Duration::from_micros(900), false);
    profiler.update_memory(rustcode_ext_host::profiler::ExtHostMemoryStats {
        rss_bytes: 64 * 1024 * 1024,
        heap_total_bytes: 24 * 1024 * 1024,
        heap_used_bytes: 16 * 1024 * 1024,
        external_bytes: 1024 * 1024,
    });

    let snap = profiler.snapshot();
    assert_eq!(snap.activations.len(), 1);
    assert_eq!(snap.total_rpc_calls, 2);
    assert_eq!(snap.total_rpc_errors, 1);
    let report = profiler.format_running_extensions_report();
    assert!(report.contains("esbenp.prettier-vscode"));
    assert!(report.contains("RSS 64 MB"));
    assert!(report.contains("vfs.readFile"));

    let t0 = Instant::now();
    let mut wd = ExtHostWatchdog::new(Duration::from_millis(20), Duration::from_millis(80), 1);
    assert!(!wd.should_send_heartbeat(t0));
    let t1 = t0 + Duration::from_millis(30);
    assert!(wd.should_send_heartbeat(t1));
    wd.mark_ping_sent(t1);
    wd.record_ack(t1 + Duration::from_millis(10), 10);
    assert!(matches!(
        wd.check_health(t1 + Duration::from_millis(20)),
        WatchdogHealth::Healthy {
            last_latency_ms: Some(10),
            restarts: 0
        }
    ));

    let t2 = t1 + Duration::from_millis(50);
    wd.mark_ping_sent(t2);
    assert!(matches!(
        wd.check_health(t2 + Duration::from_millis(120)),
        WatchdogHealth::Unresponsive {
            can_auto_restart: true,
            restarts: 0,
            ..
        }
    ));
    wd.record_restart(t2 + Duration::from_millis(120));
    wd.mark_ping_sent(t2 + Duration::from_millis(130));
    assert!(matches!(
        wd.check_health(t2 + Duration::from_millis(250)),
        WatchdogHealth::RestartLimitReached { restarts: 1, .. }
    ));

    // Reject missing node binary cleanly
    let bad = inspect_node_runtime(Some(Path::new("/definitely/missing/node-bin")), None);
    assert!(!bad.meets_minimum);
    assert!(bad.error.is_some());
}

#[tokio::test]
async fn test_tier2_live_node_heartbeat_and_profiler_metrics() {
    let bridge = Arc::new(LanguageFeatureBridge::with_defaults());
    let mut proc = NodeExtHostProcess::spawn(
        bridge.clone(),
        NodeExtHostConfig {
            args: vec![],
            cwd: Some(repo_root()),
            wire_format: RpcWireFormat::ContentLengthJson,
            ..Default::default()
        },
    )
    .expect("spawn Node extension host");

    let ping_val = bridge
        .rpc_bridge()
        .request("extHost.ping", json!({ "probe": "tier2" }), Duration::from_secs(5))
        .await
        .expect("extHost.ping RPC failed");
    assert_eq!(ping_val["pong"], json!(true));
    assert!(ping_val["pid"].is_u64());
    assert!(ping_val["uptimeMs"].is_number());
    assert!(ping_val["memory"]["rssBytes"].as_u64().unwrap_or(0) > 0);

    let latency_ms = bridge.ping_ext_host().await.expect("ping_ext_host failed");
    assert!(latency_ms < 5_000);
    let snap = bridge.profiler().snapshot();
    assert!(snap.memory.as_ref().is_some_and(|m| m.rss_bytes > 0));

    let status = proc.shutdown().await.expect("clean shutdown");
    assert!(status.is_some_and(|s| s.success()));
    assert!(
        proc.stderr_tail().is_empty(),
        "Unexpected Node stderr: {:?}",
        proc.stderr_tail()
    );
    drop(proc);
}

#[tokio::test]
async fn test_tier2_runtime_loop_manual_ext_host_restart_and_watchdog() {
    let dir = TempTestDir::new("phase10_tier2_rt_restart");
    let ext_dir = dir.mkdir_all("ext");
    std::fs::write(
        ext_dir.join("package.json"),
        json!({
            "name": "rt-probe",
            "publisher": "TestPub",
            "version": "1.0.0",
            "main": "./extension.js",
            "activationEvents": ["*"]
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        ext_dir.join("extension.js"),
        r#"
const vscode = require('vscode');
exports.activate = (context) => {
    context.subscriptions.push(
        vscode.commands.registerCommand('rtProbe.echo', (v) => ({ ok: true, v }))
    );
};
"#,
    )
    .unwrap();

    let mut ctx = RustCodeContext::new();
    ctx.ext_loader.load_from_dir(&ext_dir).unwrap();
    let shared_loader = Arc::new(RwLock::new(ctx.ext_loader.clone()));
    ctx.language_bridge
        .set_extension_loader(Arc::clone(&shared_loader));

    let mut win = ctx.create_window();
    let mut rt_loop = RuntimeLoop::new_with_shared_loader(
        RuntimeLoopConfig {
            disable_extensions: false,
            ext_host_restart_backoff: Duration::from_millis(50),
            ..Default::default()
        },
        Arc::clone(&shared_loader),
    )
    .unwrap();

    rt_loop.tick(&mut win);
    if rt_loop.is_ext_host_running() {
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(40)).await;
            rt_loop.tick(&mut win);
            if shared_loader.read().is_activated("TestPub.rt-probe") {
                break;
            }
        }
        assert!(shared_loader.read().is_activated("TestPub.rt-probe"));
        assert!(matches!(
            rt_loop.watchdog_health(),
            WatchdogHealth::Healthy { .. }
        ));

        let restarted = rt_loop.restart_ext_host(&mut win);
        assert!(restarted);
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(40)).await;
            rt_loop.tick(&mut win);
            if shared_loader.read().is_activated("TestPub.rt-probe") {
                break;
            }
        }
        assert!(rt_loop.ext_host_restart_count() >= 1);
        assert!(shared_loader.read().is_activated("TestPub.rt-probe"));
    } else {
        // Explicit ExtensionLoader contract sanity check even if node missing
        assert_eq!(ExtensionLoader::new().loaded_extensions().len(), 0);
    }

    rt_loop.shutdown_async(&mut win).await;
}
