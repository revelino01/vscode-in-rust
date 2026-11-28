use parking_lot::RwLock;
use phase10_ext_compat::{build_top14_vsix_fixtures, repo_root, TempTestDir};
use rustcode_app::runtime::load_extension_phase10_contribs;
use rustcode_app::RustCodeContext;
use rustcode_ext_host::manifest::{ExtensionLoader, JsonValidationRegistry};
use rustcode_ext_host::marketplace::{
    ExtensionInstaller, MarketplaceClient, MockHttpTransport,
};
use rustcode_ext_host::output::ExtLogLevel;
use rustcode_ext_host::provider::{LanguageFeatureBridge, ProviderType};
use rustcode_ext_host::tasks::{
    ProblemMatcherContribution, ProblemMatcherRegistry, ProblemPattern, ProblemPatternSpec,
};
use rustcode_ext_host::{
    ExtHostSecretManager, ExtHostVfsManager, NodeExtHostConfig, NodeExtHostProcess,
    RpcWireFormat,
};
use rustcode_platform::configuration::ConfigurationService;
use rustcode_platform::profiles::{ProfileManager, DEFAULT_PROFILE_ID};
use rustcode_platform::theme::ThemeService;
use rustcode_workbench::extensions_view::{render_extension_detail_html, ExtensionDetailContext};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_tier4_top14_vsix_install_parse_register_and_live_activation() {
    let dir = TempTestDir::new("phase10_tier4_top14");
    let ext_root = dir.mkdir_all("extensions");

    let fixtures = build_top14_vsix_fixtures();
    assert_eq!(fixtures.len(), 14);

    let client = Arc::new(MarketplaceClient::new(
        "https://mock-openvsx.local/api",
        Arc::new(MockHttpTransport::new()),
    ));
    let installer = ExtensionInstaller::new(client, &ext_root);

    let mut ctx = RustCodeContext::new();
    let mut win = ctx.create_window();

    // Install all 14 .vsix packages and load manifests into ExtensionLoader
    let mut loaded_extensions = Vec::new();
    for fx in &fixtures {
        let loaded = installer
            .install_from_vsix(&fx.vsix_bytes, None)
            .unwrap_or_else(|e| panic!("install_from_vsix failed for {}: {:?}", fx.id, e));
        assert_eq!(loaded.id().to_ascii_lowercase(), fx.id.to_ascii_lowercase());
        ctx.ext_loader
            .load_from_dir(&loaded.root_dir)
            .unwrap_or_else(|e| panic!("ExtensionLoader failed for {}: {:?}", fx.id, e));

        let (count, store) = installer
            .register_contributions(&loaded, &ctx.commands, Some(&mut ctx.theme), |_, _| Ok(()))
            .unwrap_or_else(|e| panic!("register_contributions failed for {}: {:?}", fx.id, e));
        // Each of our 14 extensions declares at least one contribution (command, theme, or iconTheme)
        assert!(count >= 1, "Expected >=1 contribution for {}", fx.id);
        std::mem::forget(store);

        // Register view containers, views, welcome views, submenus into workbench
        if let Some(contribs) = &loaded.manifest.contributes {
            if let Some(containers) = &contribs.views_containers {
                for c in containers.values().flatten() {
                    win.view.register_view_container(c.clone());
                }
            }
            if let Some(submenus) = &contribs.submenus {
                win.view.context_menu_registry.register_submenus(submenus);
            }
        }

        loaded_extensions.push(loaded);
    }

    assert_eq!(ctx.ext_loader.loaded_extensions().len(), 14);

    // Load Phase 10 contributions (iconThemes, productIconThemes, jsonValidation, problemMatchers)
    load_extension_phase10_contribs(&mut ctx);

    // Verify icon theme & product icon theme from PKief.material-icon-theme
    ctx.theme.set_icon_theme("material-icon-theme").unwrap();
    let (rs_icon_id, rs_icon_def) = ctx.theme.resolve_file_icon("main.rs", Some("rust")).unwrap();
    assert_eq!(rs_icon_id, "_rust");
    assert_eq!(rs_icon_def.unwrap().font_color.as_deref(), Some("#dea584"));
    ctx.theme
        .set_product_icon_theme("material-product-icons")
        .unwrap();
    assert!(ctx.theme.resolve_product_icon("explorer-view-icon").is_some());

    // Verify Dracula & Tokyo Night themes registered
    let theme_ids = ctx.theme.registered_theme_names();
    assert!(theme_ids.iter().any(|t| t.eq_ignore_ascii_case("dracula")));
    assert!(theme_ids
        .iter()
        .any(|t| t.eq_ignore_ascii_case("tokyo night")));

    // Verify jsonValidation rule from ms-azuretools.vscode-docker
    assert_eq!(
        ctx.json_validation
            .schemas_for_filename("docker-compose.override.json")
            .len(),
        1
    );

    // Verify problemMatchers from dbaeumer.vscode-eslint, rust-analyzer, cpptools
    let eslint_out = ctx
        .problem_matchers
        .parse_output("eslint-custom", None, "src/index.ts:12:4: Unused variable")
        .unwrap();
    assert_eq!(eslint_out.0, "eslint");
    assert_eq!(eslint_out.1.len(), 1);
    assert!(ctx.problem_matchers.get("ra-cargo").is_some());
    assert!(ctx.problem_matchers.get("cpptools-gcc").is_some());

    // Verify GitLens submenu registered in ContextMenuRegistry
    assert_eq!(
        win.view
            .context_menu_registry
            .get_submenu("gitlens.submenu.commit")
            .map(|s| s.label.as_str()),
        Some("GitLens Actions")
    );

    // Activate all JS-backed extensions live inside NodeExtHostProcess
    let shared_loader = Arc::new(RwLock::new(ctx.ext_loader.clone()));
    let bridge = Arc::new(
        LanguageFeatureBridge::with_defaults()
            .with_shared_extension_loader(shared_loader.clone())
            .with_command_registry(ctx.commands.clone())
            .with_configuration_service(ConfigurationService::new()),
    );

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

    for fx in fixtures.iter().filter(|f| f.has_main_js) {
        bridge
            .activate_extension(fx.id)
            .await
            .unwrap_or_else(|e| panic!("activate_extension failed for {}: {}", fx.id, e));
        if let Some(cmd) = fx.test_command {
            for _ in 0..50 {
                if bridge.command_registry().has_command(cmd) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let out = bridge
                .request_execute_command(cmd, vec![])
                .await
                .unwrap_or_else(|e| panic!("command '{}' failed for {}: {}", cmd, fx.id, e));
            assert!(!out.is_null(), "command '{}' returned null", cmd);
        }
    }

    // Verify GitLens VFS + WebviewViewProvider and Thunder Client custom FS + SecretStorage
    let gitlens_doc = bridge
        .vfs_manager()
        .request_provide_text_document_content(bridge.rpc_bridge(), "gitlens-rev:///Cargo.toml")
        .await
        .expect("gitlens TextDocumentContentProvider failed");
    assert_eq!(gitlens_doc, "commit rev for /Cargo.toml");

    let gitlens_wv = bridge
        .webview_view_manager()
        .request_resolve_webview_view(
            bridge.rpc_bridge(),
            "gitlens.views.home",
            Some("GitLens Home"),
        )
        .await
        .expect("gitlens resolveWebviewView failed");
    assert_eq!(gitlens_wv["resolved"], json!(true));
    assert!(bridge
        .provider_registry()
        .has_provider_with_id(ProviderType::WebviewView, "gitlens.views.home")
        || bridge.webview_view_manager().has_provider("gitlens.views.home"));

    // Verify SecretStorage namespace isolation across activated extensions
    assert_eq!(
        bridge
            .secret_manager()
            .get("rangav.vscode-thunder-client", "apiKey")
            .unwrap()
            .as_deref(),
        Some("tc_secret_123")
    );
    assert!(bridge
        .secret_manager()
        .get("esbenp.prettier-vscode", "apiKey")
        .unwrap()
        .is_none());

    // Verify LogOutputChannel created by activated JS extensions
    let prettier_log = bridge
        .output_manager()
        .get_channel_by_name("prettier-vscode")
        .expect("prettier-vscode output channel missing");
    assert!(prettier_log.log);
    assert_eq!(prettier_log.log_level, ExtLogLevel::Info);
    assert!(prettier_log.content.contains("prettier-vscode activated"));

    let status = proc.shutdown().await.expect("clean Node shutdown");
    assert!(status.is_some_and(|s| s.success()));
    assert!(
        proc.stderr_tail().is_empty(),
        "Unexpected Node stderr: {:?}",
        proc.stderr_tail()
    );
    drop(proc);
}

#[test]
fn test_tier4_adversarial_security_boundaries() {
    let dir = TempTestDir::new("phase10_tier4_security");
    let ext_root = dir.mkdir_all("extensions");

    // 1. VFS scheme reservation & unknown scheme boundary
    let vfs = ExtHostVfsManager::new();
    assert!(vfs.register_file_system_provider("file", true, false).is_err());
    assert!(vfs.register_file_system_provider("", true, false).is_err());
    assert!(vfs
        .register_text_document_content_provider("file")
        .is_err());
    assert!(vfs
        .register_text_document_content_provider("untitled")
        .is_err());
    assert!(!vfs.has_file_system_scheme("C:\\Windows\\System32\\drivers\\etc\\hosts"));

    // 2. SecretStorage namespace isolation & malformed key/id rejection
    let secrets = ExtHostSecretManager::new();
    assert!(secrets.store("", "k", "v").is_err());
    assert!(secrets.store("bad ext id", "k", "v").is_err());
    assert!(secrets.store("pub.valid", "", "v").is_err());
    secrets.store("pub.alpha", "token", "alpha-only").unwrap();
    assert!(secrets.get("pub.beta", "token").unwrap().is_none());

    // 3. Path traversal rejection in iconThemes / productIconThemes / jsonValidation
    let client = Arc::new(MarketplaceClient::new(
        "https://mock-openvsx.local/api",
        Arc::new(MockHttpTransport::new()),
    ));
    let installer = ExtensionInstaller::new(client, &ext_root);
    let vsix_evil_icon = phase10_ext_compat::build_vsix_bytes(
        &json!({
            "name": "evil-icons",
            "publisher": "attacker",
            "version": "1.0.0",
            "contributes": {
                "iconThemes": [{ "id": "evil", "label": "Evil", "path": "./../../etc/passwd" }]
            }
        }),
        &[],
    );
    let evil_ext = installer.install_from_vsix(&vsix_evil_icon, None).unwrap();
    let cmds = rustcode_platform::commands::CommandRegistry::new();
    let mut theme = ThemeService::new();
    assert!(installer
        .register_contributions(&evil_ext, &cmds, Some(&mut theme), |_, _| Ok(()))
        .is_err());

    let mut json_val = JsonValidationRegistry::new();
    assert!(json_val
        .register(
            "attacker.evil",
            &evil_ext.root_dir,
            &rustcode_ext_host::manifest::JsonValidationContribution {
                file_match: rustcode_ext_host::manifest::JsonValidationFileMatch::Single(
                    "*.json".to_string(),
                ),
                url: "../../secrets.json".to_string(),
            },
        )
        .is_err());

    // 4. ProfileManager ID traversal & Default deletion protection
    let mut pm = ProfileManager::new(dir.mkdir_all("profiles"));
    assert!(pm.delete_profile(DEFAULT_PROFILE_ID).is_err());
    assert!(pm.delete_profile("../outside").is_err());
    assert!(ProfileManager::validate_profile_id("bad/slash").is_err());
    assert!(ProfileManager::validate_profile_id("..").is_err());

    // 5. ProblemMatcherRegistry ReDoS / compiled regex size limit rejection
    let matchers = ProblemMatcherRegistry::new();
    let oversized_pattern = "a".repeat(20_000) + "(?:b|c){1,200}(.*)";
    assert!(matchers
        .register(ProblemMatcherContribution {
            name: Some("huge".to_string()),
            pattern: Some(ProblemPatternSpec::Single(ProblemPattern {
                regexp: oversized_pattern,
                file: Some(1),
                ..Default::default()
            })),
            ..Default::default()
        })
        .is_err());

    // 6. Extension Detail HTML XSS / javascript: URL escaping
    let html = render_extension_detail_html(&ExtensionDetailContext {
        extension_id: "<script>bad</script>",
        display_name: "<img onerror=alert(1) src=x>",
        publisher: "pub",
        version: "1.0.0",
        description: "desc",
        installed: false,
        enabled: true,
        pre_release: false,
        available_update: None,
        metadata: None,
        manifest: None,
        readme_md: Some("[xss](javascript:alert(1)) <script>steal()</script>"),
        changelog_md: None,
    });
    assert!(!html.contains("<script>steal()</script>"));
    assert!(!html.contains("href=\"javascript:"));
    assert!(html.contains("&lt;script&gt;bad&lt;/script&gt;"));

    // ExtensionLoader sanity
    assert_eq!(ExtensionLoader::new().loaded_extensions().len(), 0);
}
