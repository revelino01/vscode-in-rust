use phase10_ext_compat::{build_vsix_bytes, TempTestDir};
use rustcode_ext_host::marketplace::{
    curated_catalog, is_newer_version, search_curated_catalog, ExtensionAutoUpdater,
    ExtensionInstaller, InstalledExtensionMeta, MarketplaceClient, MockHttpTransport,
    SemverVersion, WorkspaceExtensionsRecommendations,
};
use rustcode_platform::configuration::{ConfigurationService, ConfigurationTarget};
use rustcode_platform::keybindings::KeybindingService;
use rustcode_platform::profiles::{ProfileExtensionEntry, ProfileManager, DEFAULT_PROFILE_ID};
use rustcode_workbench::extensions_view::{
    parse_extension_detail_message, render_extension_detail_html, ExtensionDetailContext,
    ExtensionDetailMessage,
};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

#[test]
fn test_tier3_extension_detail_html_and_postmessage_protocol() {
    let catalog = curated_catalog();
    assert_eq!(catalog.len(), 14);
    for id in [
        "esbenp.prettier-vscode",
        "dbaeumer.vscode-eslint",
        "eamodio.gitlens",
        "rust-lang.rust-analyzer",
        "ms-python.python",
        "golang.go",
        "ms-vscode.cpptools",
        "ms-azuretools.vscode-docker",
        "PKief.material-icon-theme",
        "dracula-theme.theme-dracula",
        "enkia.tokyo-night",
        "yzhang.markdown-all-in-one",
        "usernamehw.errorlens",
        "rangav.vscode-thunder-client",
    ] {
        let query_res = search_curated_catalog(id);
        assert!(
            query_res
                .extensions
                .iter()
                .any(|m| format!("{}.{}", m.namespace, m.name).eq_ignore_ascii_case(id)),
            "Curated catalog missing {}",
            id
        );
    }

    let meta = &catalog[0];
    let readme = "# Heading\n\n- Feature **One** and `code`\n\n[Click](javascript:alert(1))\n\n<script>alert('xss')</script>";
    let changelog = "## 1.0.0\n- Initial release";
    let html = render_extension_detail_html(&ExtensionDetailContext {
        extension_id: "rust-lang.rust-analyzer",
        display_name: meta.display_name.as_deref().unwrap_or("rust-analyzer"),
        publisher: &meta.namespace,
        version: &meta.version,
        description: meta.description.as_deref().unwrap_or(""),
        installed: true,
        enabled: true,
        pre_release: false,
        available_update: Some("0.4.0"),
        metadata: Some(meta),
        manifest: None,
        readme_md: Some(readme),
        changelog_md: Some(changelog),
    });

    assert!(html.contains("rust-lang.rust-analyzer"));
    assert!(html.contains("Update to v0.4.0"));
    assert!(html.contains("Switch to Pre-Release Version"));
    assert!(html.contains("<h1>Heading</h1>"));
    assert!(html.contains("&lt;script&gt;alert("));
    assert!(!html.contains("<script>alert"));
    assert!(!html.contains("href=\"javascript:"));

    assert_eq!(
        parse_extension_detail_message(&json!({
            "type": "switchPreRelease",
            "extensionId": "rust-lang.rust-analyzer",
            "preRelease": true
        })),
        Some(ExtensionDetailMessage::SwitchPreRelease {
            extension_id: "rust-lang.rust-analyzer".to_string(),
            pre_release: true,
        })
    );
    assert_eq!(
        parse_extension_detail_message(&json!({
            "type": "update",
            "id": "rust-lang.rust-analyzer"
        })),
        Some(ExtensionDetailMessage::Update {
            extension_id: "rust-lang.rust-analyzer".to_string(),
        })
    );
}

#[tokio::test]
async fn test_tier3_vsix_file_pack_cycles_prerelease_and_autoupdater() {
    let dir = TempTestDir::new("phase10_tier3_marketplace");
    let ext_dir = dir.mkdir_all("extensions");

    // 1. Direct .vsix file install writes InstalledExtensionMeta (.metadata.json)
    let vsix_path = dir.path().join("sample-tool.vsix");
    let vsix_bytes = build_vsix_bytes(
        &json!({
            "name": "sample-tool",
            "publisher": "acme",
            "version": "1.2.3",
            "displayName": "Sample Tool",
            "categories": ["Other"]
        }),
        &[],
    );
    std::fs::write(&vsix_path, &vsix_bytes).unwrap();

    let transport = Arc::new(MockHttpTransport::new());
    let base = "https://mock-openvsx.local/api";

    // Extension pack with a mutual cycle A <-> B and dependency C -> must terminate cleanly!
    let pack_a_vsix = build_vsix_bytes(
        &json!({
            "name": "pack-a",
            "publisher": "acme",
            "version": "1.0.0",
            "extensionPack": ["acme.pack-b"],
            "extensionDependencies": ["acme.dep-c"]
        }),
        &[],
    );
    let pack_b_vsix = build_vsix_bytes(
        &json!({
            "name": "pack-b",
            "publisher": "acme",
            "version": "1.0.0",
            "extensionPack": ["acme.pack-a"]
        }),
        &[],
    );
    let dep_c_vsix = build_vsix_bytes(
        &json!({
            "name": "dep-c",
            "publisher": "acme",
            "version": "1.0.0"
        }),
        &[],
    );

    transport.set_response(
        format!("{base}/acme/pack-a"),
        serde_json::to_vec(&json!({
            "namespace": "acme",
            "name": "pack-a",
            "version": "1.0.0",
            "downloadUrl": format!("{base}/files/pack-a.vsix")
        }))
        .unwrap(),
    );
    transport.set_response(format!("{base}/files/pack-a.vsix"), pack_a_vsix);
    transport.set_response(
        format!("{base}/acme/pack-b"),
        serde_json::to_vec(&json!({
            "namespace": "acme",
            "name": "pack-b",
            "version": "1.0.0",
            "downloadUrl": format!("{base}/files/pack-b.vsix")
        }))
        .unwrap(),
    );
    transport.set_response(format!("{base}/files/pack-b.vsix"), pack_b_vsix);
    transport.set_response(
        format!("{base}/acme/dep-c"),
        serde_json::to_vec(&json!({
            "namespace": "acme",
            "name": "dep-c",
            "version": "1.0.0",
            "downloadUrl": format!("{base}/files/dep-c.vsix")
        }))
        .unwrap(),
    );
    transport.set_response(format!("{base}/files/dep-c.vsix"), dep_c_vsix);

    // Pre-release + auto-update fixtures for `acme.sample-tool`
    let pre_vsix = build_vsix_bytes(
        &json!({
            "name": "sample-tool",
            "publisher": "acme",
            "version": "2.0.0-pre.1"
        }),
        &[],
    );
    transport.set_response(
        format!("{base}/acme/sample-tool/pre-release"),
        serde_json::to_vec(&json!({
            "namespace": "acme",
            "name": "sample-tool",
            "version": "2.0.0-pre.1",
            "preRelease": true,
            "downloadUrl": format!("{base}/files/sample-tool-pre.vsix")
        }))
        .unwrap(),
    );
    transport.set_response(format!("{base}/files/sample-tool-pre.vsix"), pre_vsix);

    let stable_v2_vsix = build_vsix_bytes(
        &json!({
            "name": "sample-tool",
            "publisher": "acme",
            "version": "2.0.0"
        }),
        &[],
    );
    let stable_v2_meta = serde_json::to_vec(&json!({
        "namespace": "acme",
        "name": "sample-tool",
        "version": "2.0.0",
        "downloadUrl": format!("{base}/files/sample-tool-2.0.0.vsix")
    }))
    .unwrap();
    transport.set_response(format!("{base}/acme/sample-tool"), stable_v2_meta.clone());
    transport.set_response(format!("{base}/acme/sample-tool/2.0.0"), stable_v2_meta);
    transport.set_response(format!("{base}/files/sample-tool-2.0.0.vsix"), stable_v2_vsix);

    let client = Arc::new(MarketplaceClient::new(base, transport));
    let installer = ExtensionInstaller::new(client, &ext_dir);

    let loaded_vsix = installer.install_from_vsix_file(&vsix_path).unwrap();
    let meta = installer
        .read_installed_meta(&loaded_vsix.root_dir)
        .expect(".metadata.json must be written for VSIX install");
    assert_eq!(
        meta,
        InstalledExtensionMeta {
            pre_release: false,
            installed_from_vsix: true,
            installed_version: "1.2.3".to_string(),
            installed_at_epoch_secs: meta.installed_at_epoch_secs,
        }
    );

    // Extension pack + dependency installation breaks A <-> B cycle cleanly
    let pack_installed = installer
        .install_with_pack("acme", "pack-a", false)
        .await
        .unwrap();
    assert_eq!(pack_installed.len(), 3);

    // AutoUpdater detects 1.2.3 -> 2.0.0 upgrade and respects pre-release channel
    assert!(SemverVersion::parse("2.0.0") > SemverVersion::parse("2.0.0-pre.1"));
    assert!(is_newer_version("1.2.3", "2.0.0"));

    let cfg = ConfigurationService::new();
    cfg.update(
        "extensions.autoCheckUpdates",
        json!(true),
        ConfigurationTarget::User,
    )
    .unwrap();
    cfg.update("extensions.autoUpdate", json!(true), ConfigurationTarget::User)
        .unwrap();
    let updates = ExtensionAutoUpdater::check_for_updates(&installer, Some(&cfg))
        .await
        .unwrap();
    assert!(updates
        .iter()
        .any(|u| u.id == "acme.sample-tool" && u.available_version == "2.0.0"));

    let applied = ExtensionAutoUpdater::check_and_apply_auto_updates(&installer, &cfg)
        .await
        .unwrap();
    assert!(applied.iter().any(|u| u.id == "acme.sample-tool"));

    let pre_loaded = installer
        .install_from_marketplace_with_options("acme", "sample-tool", None, true)
        .await
        .unwrap();
    assert_eq!(pre_loaded.manifest.version, "2.0.0-pre.1");
    assert_eq!(
        installer
            .read_installed_meta(&pre_loaded.root_dir)
            .map(|m| m.pre_release),
        Some(true)
    );
}

#[test]
fn test_tier3_workspace_recommendations_and_profiles_code_profile() {
    let dir = TempTestDir::new("phase10_tier3_profiles");
    let ws = dir.mkdir_all("ws");
    dir.write_file(
        "ws/.vscode/extensions.json",
        r#"{
            // Team recommended tooling
            "recommendations": ["rust-lang.rust-analyzer", "esbenp.prettier-vscode", "dbaeumer.vscode-eslint"],
            "unwantedRecommendations": ["dbaeumer.vscode-eslint"]
        }"#,
    );

    let recs = WorkspaceExtensionsRecommendations::load_from_workspace(&ws).unwrap();
    let pending = recs.pending_recommendations(
        &["rust-lang.rust-analyzer".to_string()],
        &HashSet::new(),
    );
    assert_eq!(pending, vec!["esbenp.prettier-vscode".to_string()]);

    // Profile lifecycle + .code-profile export/import
    let profiles_dir = dir.mkdir_all("profiles");
    let mut pm = ProfileManager::new(&profiles_dir);
    assert_eq!(pm.active_profile_id(), DEFAULT_PROFILE_ID);

    let cfg = ConfigurationService::new();
    cfg.update("editor.tabSize", json!(4), ConfigurationTarget::User)
        .unwrap();
    let kb = KeybindingService::new();
    kb.load_keybindings_json(r#"[{ "key": "ctrl+k ctrl+d", "command": "default.cmd" }]"#)
        .unwrap();
    pm.capture_current_into_active(
        &cfg,
        Some(&kb),
        vec![ProfileExtensionEntry {
            id: "esbenp.prettier-vscode".to_string(),
            version: Some("11.0.0".to_string()),
            enabled: true,
        }],
        BTreeMap::new(),
    );

    let rust_profile = pm.create_profile("Rust Heavy", Some("crab".into()), None).unwrap();
    pm.switch_profile(&rust_profile.id, &cfg, Some(&kb)).unwrap();
    cfg.update("editor.tabSize", json!(2), ConfigurationTarget::User)
        .unwrap();
    pm.capture_current_into_active(&cfg, Some(&kb), Vec::new(), BTreeMap::new());

    // Switching back to Default restores tabSize=4
    pm.switch_profile(DEFAULT_PROFILE_ID, &cfg, Some(&kb)).unwrap();
    assert_eq!(cfg.get::<u32>("editor.tabSize"), Some(4));

    let exported = pm.export_code_profile(&rust_profile.id).unwrap();
    pm.save_to_disk().unwrap();

    let mut pm2 = ProfileManager::new(&profiles_dir);
    pm2.load_from_disk().unwrap();
    let imported = pm2
        .import_code_profile(&exported, true, Some(&cfg), Some(&kb))
        .unwrap();
    assert_eq!(pm2.active_profile_id(), imported.id);
    assert_eq!(cfg.get::<u32>("editor.tabSize"), Some(2));
}
