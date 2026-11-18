//! Open VSX Marketplace Client & Extension Downloader.

pub mod catalog;
pub mod client;
pub mod downloader;
pub mod installer;
pub mod transport;
pub mod types;

pub use catalog::{curated_catalog, search_curated_catalog};
pub use client::MarketplaceClient;
pub use downloader::ExtensionDownloader;
pub use installer::ExtensionInstaller;
pub use transport::{CurlHttpTransport, HttpTransport, MockHttpTransport};
pub use types::{ExtensionFiles, ExtensionMetadata, MarketplaceError, SearchResponse};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::LoadedExtension;
    use rustcode_platform::commands::CommandRegistry;
    use rustcode_platform::theme::ThemeService;
    use serde_json::json;
    use std::io::Write;
    use std::sync::Arc;
    use zip::write::SimpleFileOptions;

    fn create_test_vsix(manifest_json: &str, theme_json: Option<&str>) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

            zip.start_file("extension/package.json", options).unwrap();
            zip.write_all(manifest_json.as_bytes()).unwrap();

            if let Some(t_json) = theme_json {
                zip.start_file("extension/themes/test-theme.json", options).unwrap();
                zip.write_all(t_json.as_bytes()).unwrap();
            }

            zip.start_file("extension/syntaxes/test.tmGrammar.json", options).unwrap();
            zip.write_all(b"{\"scopeName\": \"source.test\"}").unwrap();

            zip.finish().unwrap();
        }
        buffer.into_inner()
    }

    #[tokio::test]
    async fn test_open_vsx_marketplace_search_and_metadata() {
        let transport = Arc::new(MockHttpTransport::new());
        let client = MarketplaceClient::new("https://open-vsx.org/api", transport.clone());

        // Mock search response
        transport.set_response(
            "https://open-vsx.org/api/-/search?query=rust-analyzer&size=10&offset=0",
            serde_json::to_vec(&json!({
                "totalSize": 1,
                "extensions": [
                    {
                        "namespace": "rust-lang",
                        "name": "rust-analyzer",
                        "version": "0.3.1800",
                        "displayName": "rust-analyzer",
                        "description": "Rust language support",
                        "downloadUrl": "https://open-vsx.org/api/rust-lang/rust-analyzer/0.3.1800/file/rust-analyzer.vsix"
                    }
                ]
            })).unwrap(),
        );

        let search_res = client.search("rust-analyzer", Some(10), Some(0)).await.unwrap();
        assert_eq!(search_res.total_size, 1);
        assert_eq!(search_res.extensions.len(), 1);
        assert_eq!(search_res.extensions[0].name, "rust-analyzer");
        assert_eq!(
            search_res.extensions[0].resolved_download_url().as_deref(),
            Some("https://open-vsx.org/api/rust-lang/rust-analyzer/0.3.1800/file/rust-analyzer.vsix")
        );

        // Mock extension version metadata
        transport.set_response(
            "https://open-vsx.org/api/rust-lang/rust-analyzer/0.3.1800",
            serde_json::to_vec(&json!({
                "namespace": "rust-lang",
                "name": "rust-analyzer",
                "version": "0.3.1800",
                "displayName": "rust-analyzer",
                "files": {
                    "download": "https://open-vsx.org/api/rust-lang/rust-analyzer/0.3.1800/file/rust-analyzer.vsix"
                }
            })).unwrap(),
        );

        let meta = client.get_version_metadata("rust-lang", "rust-analyzer", "0.3.1800").await.unwrap();
        assert_eq!(meta.version, "0.3.1800");
        assert_eq!(
            meta.resolved_download_url().as_deref(),
            Some("https://open-vsx.org/api/rust-lang/rust-analyzer/0.3.1800/file/rust-analyzer.vsix")
        );
    }

    #[test]
    fn test_vsix_unpack_and_manifest_parsing() {
        let manifest_json = r#"{
            "name": "my-theme-pack",
            "publisher": "designer",
            "version": "1.2.0",
            "displayName": "My Theme Pack",
            "contributes": {
                "themes": [
                    {
                        "label": "Neon Dark",
                        "uiTheme": "vs-dark",
                        "path": "./themes/test-theme.json"
                    }
                ]
            }
        }"#;

        let theme_json = r##"{
            "name": "Neon Dark",
            "type": "dark",
            "colors": {
                "editor.background": "#001122"
            }
        }"##;

        let vsix_bytes = create_test_vsix(manifest_json, Some(theme_json));

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("rustcode_vsix_test_{}", now));
        let manifest = ExtensionDownloader::unpack_vsix(&vsix_bytes, &temp_dir).unwrap();

        assert_eq!(manifest.name, "my-theme-pack");
        assert_eq!(manifest.publisher, "designer");
        assert_eq!(manifest.version, "1.2.0");

        assert!(temp_dir.join("package.json").exists());
        assert!(temp_dir.join("themes/test-theme.json").exists());
        assert!(temp_dir.join("syntaxes/test.tmGrammar.json").exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_extension_installer_and_runtime_registration() {
        let manifest_json = r#"{
            "name": "super-tool",
            "publisher": "devteam",
            "version": "2.0.0",
            "displayName": "Super Tool",
            "contributes": {
                "commands": [
                    {
                        "command": "superTool.run",
                        "title": "Run Super Tool",
                        "category": "Tools"
                    }
                ],
                "themes": [
                    {
                        "label": "Super Dark",
                        "uiTheme": "vs-dark",
                        "path": "./themes/test-theme.json"
                    }
                ],
                "grammars": [
                    {
                        "language": "superlang",
                        "scopeName": "source.super",
                        "path": "./syntaxes/test.tmGrammar.json"
                    }
                ]
            }
        }"#;

        let theme_json = r##"{
            "name": "Super Dark",
            "type": "dark",
            "colors": {
                "editor.background": "#101010"
            }
        }"##;

        let vsix_bytes = create_test_vsix(manifest_json, Some(theme_json));

        let transport = Arc::new(MockHttpTransport::new());
        let dl_url = "https://open-vsx.org/api/devteam/super-tool/2.0.0/file/super-tool-2.0.0.vsix";

        transport.set_response(
            "https://open-vsx.org/api/devteam/super-tool",
            serde_json::to_vec(&json!({
                "namespace": "devteam",
                "name": "super-tool",
                "version": "2.0.0",
                "downloadUrl": dl_url
            })).unwrap(),
        );
        transport.set_response(dl_url, vsix_bytes);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_ext_dir = std::env::temp_dir().join(format!("rustcode_install_test_{}", now));
        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = ExtensionInstaller::new(client, &temp_ext_dir);

        let installed: LoadedExtension = installer
            .install_from_marketplace("devteam", "super-tool", None)
            .await
            .unwrap();

        assert_eq!(installed.manifest.name, "super-tool");
        assert_eq!(installed.manifest.publisher, "devteam");

        // Test registering contributions into CommandRegistry, ThemeService, and grammar registrar
        let commands = CommandRegistry::new();
        let mut theme_service = ThemeService::new();
        let mut registered_grammar = None;

        let (count, _subs) = installer
            .register_contributions(
                &installed,
                &commands,
                Some(&mut theme_service),
                |lang, path| {
                    registered_grammar = Some((lang.to_string(), path.to_path_buf()));
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(count, 3, "Should have registered 1 command, 1 theme, and 1 grammar");

        // Verify command execution
        assert!(commands.has_command("superTool.run"));
        let cmd_res = commands.execute("superTool.run", None).unwrap();
        assert_eq!(cmd_res.get("command").and_then(|c| c.as_str()), Some("superTool.run"));

        // Verify theme application
        assert_eq!(theme_service.current().name, "Super Dark");
        assert_eq!(
            theme_service.current().colors.get("editor.background").map(|s| s.as_str()),
            Some("#101010")
        );

        // Verify grammar registration
        assert!(registered_grammar.is_some());
        let (lang, path) = registered_grammar.unwrap();
        assert_eq!(lang, "superlang");
        assert!(path.ends_with("test.tmGrammar.json"));

        let _ = std::fs::remove_dir_all(&temp_ext_dir);
    }

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), now))
    }

    #[tokio::test]
    async fn test_list_installed_and_folder_layouts() {
        let temp_ext_dir = unique_temp_dir("rustcode_list_installed");
        let transport = Arc::new(MockHttpTransport::new());

        let vsix_a = create_test_vsix(
            r#"{"name":"ext-a","publisher":"pubone","version":"1.0.0","displayName":"Ext A"}"#,
            None,
        );
        let vsix_b = create_test_vsix(
            r#"{"name":"ext-b","publisher":"pubtwo","version":"2.1.0","displayName":"Ext B"}"#,
            None,
        );

        let dl_url_a = "https://open-vsx.org/api/pubone/ext-a/1.0.0/file/ext-a.vsix";
        transport.set_response(
            "https://open-vsx.org/api/pubone/ext-a",
            serde_json::to_vec(&json!({
                "namespace": "pubone",
                "name": "ext-a",
                "version": "1.0.0",
                "downloadUrl": dl_url_a
            }))
            .unwrap(),
        );
        transport.set_response(dl_url_a, vsix_a);

        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = ExtensionInstaller::new(client, &temp_ext_dir);

        installer
            .install_from_marketplace("pubone", "ext-a", None)
            .await
            .unwrap();
        installer
            .install_from_vsix(&vsix_b, Some("pubtwo.ext-b"))
            .unwrap();

        // Write a non-extension file (e.g. .disabled.json) to ensure list_installed ignores files
        std::fs::write(temp_ext_dir.join(".disabled.json"), "[]").unwrap();

        let installed = installer.list_installed();
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].manifest.publisher, "pubone");
        assert_eq!(installed[0].manifest.name, "ext-a");
        assert_eq!(installed[1].manifest.publisher, "pubtwo");
        assert_eq!(installed[1].manifest.name, "ext-b");

        // Verify LoadedExtension implements Debug + Clone
        let cloned = installed[0].clone();
        assert_eq!(cloned.manifest.name, "ext-a");
        assert!(!format!("{:?}", cloned).is_empty());

        let _ = std::fs::remove_dir_all(&temp_ext_dir);
    }

    #[test]
    fn test_disabled_persistence_roundtrip() {
        let temp_ext_dir = unique_temp_dir("rustcode_disabled_roundtrip");
        let transport = Arc::new(MockHttpTransport::new());
        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = ExtensionInstaller::new(client.clone(), &temp_ext_dir);

        // Read-only when absent
        assert!(installer.is_enabled("pubone.ext-a"));
        assert!(installer.load_disabled_set().is_empty());
        assert!(!temp_ext_dir.exists());

        // Disable persists .disabled.json
        installer.set_enabled("pubone.ext-a", false).unwrap();
        assert!(temp_ext_dir.join(".disabled.json").exists());
        assert!(!installer.is_enabled("pubone.ext-a"));
        assert!(!installer.is_enabled("PubOne.Ext-A")); // case-insensitive
        assert!(installer.load_disabled_set().contains("pubone.ext-a"));

        // Re-create installer pointing to same temp_dir to verify disk persistence
        let installer2 = ExtensionInstaller::new(client, &temp_ext_dir);
        assert!(!installer2.is_enabled("pubone.ext-a"));

        // Re-enable removes it
        installer2.set_enabled("PubOne.Ext-A", true).unwrap();
        assert!(installer2.is_enabled("pubone.ext-a"));
        assert!(installer2.load_disabled_set().is_empty());

        let _ = std::fs::remove_dir_all(&temp_ext_dir);
    }

    #[test]
    fn test_uninstall_cleans_folders_and_disabled_state() {
        let temp_ext_dir = unique_temp_dir("rustcode_uninstall");
        let transport = Arc::new(MockHttpTransport::new());
        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = ExtensionInstaller::new(client, &temp_ext_dir);

        let vsix_foo = create_test_vsix(
            r#"{"name":"foo","publisher":"pub","version":"1.0.0"}"#,
            None,
        );
        let vsix_foobar = create_test_vsix(
            r#"{"name":"foobar","publisher":"pub","version":"1.0.0"}"#,
            None,
        );

        installer.install_from_vsix(&vsix_foo, None).unwrap();
        installer.install_from_vsix(&vsix_foobar, None).unwrap();
        installer.set_enabled("pub.foo", false).unwrap();
        assert!(!installer.is_enabled("pub.foo"));

        assert!(installer.uninstall("pub", "foo").unwrap());
        assert!(!temp_ext_dir.join("pub.foo-1.0.0").exists());
        // Prefix sibling must remain untouched
        assert!(temp_ext_dir.join("pub.foobar-1.0.0").exists());
        // Disabled entry cleaned on uninstall
        assert!(installer.is_enabled("pub.foo"));
        // Uninstalling again returns Ok(false)
        assert!(!installer.uninstall("pub", "foo").unwrap());

        let _ = std::fs::remove_dir_all(&temp_ext_dir);
    }

    #[test]
    fn test_path_traversal_rejected() {
        let temp_ext_dir = unique_temp_dir("rustcode_traversal");
        let transport = Arc::new(MockHttpTransport::new());
        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = ExtensionInstaller::new(client, &temp_ext_dir);

        let benign_vsix = create_test_vsix(
            r#"{"name":"good","publisher":"pub","version":"1.0.0"}"#,
            None,
        );
        assert!(matches!(
            installer.install_from_vsix(&benign_vsix, Some("../escape")),
            Err(MarketplaceError::InvalidManifest(_))
        ));
        assert!(matches!(
            installer.install_from_vsix(&benign_vsix, Some("sub/dir")),
            Err(MarketplaceError::InvalidManifest(_))
        ));

        let evil_vsix = create_test_vsix(
            r#"{"name":"evil","publisher":"../../outside","version":"1.0.0"}"#,
            None,
        );
        assert!(matches!(
            installer.install_from_vsix(&evil_vsix, None),
            Err(MarketplaceError::InvalidManifest(_))
        ));

        assert!(matches!(
            installer.uninstall("../etc", "passwd"),
            Err(MarketplaceError::InvalidManifest(_))
        ));
        assert!(matches!(
            installer.uninstall("pub", "../evil"),
            Err(MarketplaceError::InvalidManifest(_))
        ));

        let _ = std::fs::remove_dir_all(&temp_ext_dir);
    }

    #[test]
    fn test_curated_catalog_search() {
        let all = curated_catalog();
        assert!(!all.is_empty());

        let empty_q = search_curated_catalog("");
        assert_eq!(empty_q.total_size, all.len());

        let rust_q = search_curated_catalog("rust");
        assert!(rust_q
            .extensions
            .iter()
            .any(|e| e.namespace == "rust-lang" && e.name == "rust-analyzer"));
        assert_eq!(rust_q.total_size, rust_q.extensions.len());
    }

    #[test]
    fn test_register_contributions_rejects_path_escape() {
        let temp_root = unique_temp_dir("rustcode_contrib_escape");
        let ext_dir = temp_root.join("extensions");
        let outside_secret = temp_root.join("secret_grammar.json");
        let outside_theme = temp_root.join("secret_theme.json");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(&outside_secret, r#"{"scopeName": "source.secret"}"#).unwrap();
        std::fs::write(&outside_theme, r#"{"name": "Secret", "type": "dark", "colors": {}}"#).unwrap();

        let transport = Arc::new(MockHttpTransport::new());
        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = ExtensionInstaller::new(client, &ext_dir);

        let commands = CommandRegistry::new();
        let mut theme_service = ThemeService::new();

        // Grammar path traversal via ./../../
        let vsix_grammar_escape = create_test_vsix(
            r#"{
                "name": "grammar-escape",
                "publisher": "pub",
                "version": "1.0.0",
                "contributes": {
                    "grammars": [{"language": "evil", "scopeName": "source.evil", "path": "./../../secret_grammar.json"}]
                }
            }"#,
            None,
        );
        let ext_g = installer.install_from_vsix(&vsix_grammar_escape, None).unwrap();
        let mut called = false;
        let res_g = installer.register_contributions(&ext_g, &commands, Some(&mut theme_service), |_, _| {
            called = true;
            Ok(())
        });
        assert!(matches!(res_g, Err(MarketplaceError::InvalidManifest(_))));
        assert!(!called);

        // Theme path traversal via ./../../
        let vsix_theme_escape = create_test_vsix(
            r#"{
                "name": "theme-escape",
                "publisher": "pub",
                "version": "1.0.0",
                "contributes": {
                    "themes": [{"label": "Evil", "uiTheme": "vs-dark", "path": "./../../secret_theme.json"}]
                }
            }"#,
            None,
        );
        let ext_t = installer.install_from_vsix(&vsix_theme_escape, None).unwrap();
        let res_t = installer.register_contributions(&ext_t, &commands, Some(&mut theme_service), |_, _| Ok(()));
        assert!(matches!(res_t, Err(MarketplaceError::InvalidManifest(_))));

        // Absolute, backslash, nul-byte, and curdir-only paths rejected
        for bad_path in ["/etc/passwd", "sub\\..\\..\\secret.json", "foo\0bar.json", ".", "./"] {
            let mut bad_ext = ext_g.clone();
            bad_ext.manifest.contributes.as_mut().unwrap().grammars.as_mut().unwrap()[0].path =
                bad_path.to_string();
            assert!(
                matches!(
                    installer.register_contributions(&bad_ext, &commands, None, |_, _| Ok(())),
                    Err(MarketplaceError::InvalidManifest(_))
                ),
                "expected InvalidManifest for path {}",
                bad_path
            );
        }

        // Symlink inside ext root pointing outside is rejected
        #[cfg(unix)]
        {
            let link_path = ext_g.root_dir.join("linked_grammar.json");
            std::os::unix::fs::symlink(&outside_secret, &link_path).unwrap();
            let mut sym_ext = ext_g.clone();
            sym_ext.manifest.contributes.as_mut().unwrap().grammars.as_mut().unwrap()[0].path =
                "./linked_grammar.json".to_string();
            let mut sym_called = false;
            let res_sym = installer.register_contributions(&sym_ext, &commands, None, |_, _| {
                sym_called = true;
                Ok(())
            });
            assert!(matches!(res_sym, Err(MarketplaceError::InvalidManifest(_))));
            assert!(!sym_called);
        }

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn test_uninstall_hyphen_prefix_and_corrupt_manifest_fallback() {
        let temp_ext_dir = unique_temp_dir("rustcode_uninstall_hyphen");
        let transport = Arc::new(MockHttpTransport::new());
        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = ExtensionInstaller::new(client, &temp_ext_dir);

        // 1. Damaged sibling without package.json: pub.foo-bar-1.0.0 must NOT be deleted by uninstall("pub", "foo")
        let foo_dir = temp_ext_dir.join("pub.foo-1.0.0");
        let foo_bar_dir = temp_ext_dir.join("pub.foo-bar-1.0.0");
        std::fs::create_dir_all(&foo_dir).unwrap();
        std::fs::create_dir_all(&foo_bar_dir).unwrap();
        std::fs::write(
            foo_dir.join("package.json"),
            r#"{"name":"foo","publisher":"pub","version":"1.0.0"}"#,
        )
        .unwrap();

        assert!(installer.uninstall("pub", "foo").unwrap());
        assert!(!foo_dir.exists());
        assert!(foo_bar_dir.exists(), "pub.foo-bar-1.0.0 without package.json must remain intact");

        // 2. Corrupt package.json in pub.foo-1.0.0 and pub.foo-bar-1.0.0 falls back to folder-name match
        std::fs::create_dir_all(&foo_dir).unwrap();
        std::fs::write(foo_dir.join("package.json"), "{corrupt json").unwrap();
        std::fs::write(foo_bar_dir.join("package.json"), "{corrupt json").unwrap();

        assert!(installer.uninstall("pub", "foo").unwrap());
        assert!(!foo_dir.exists(), "corrupt pub.foo-1.0.0 should uninstall via folder fallback");
        assert!(foo_bar_dir.exists(), "corrupt pub.foo-bar-1.0.0 must remain intact");

        let _ = std::fs::remove_dir_all(&temp_ext_dir);
    }

    #[test]
    fn test_install_rejects_reserved_dotfiles() {
        let temp_ext_dir = unique_temp_dir("rustcode_dotfiles");
        let transport = Arc::new(MockHttpTransport::new());
        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = ExtensionInstaller::new(client, &temp_ext_dir);

        let benign_vsix = create_test_vsix(
            r#"{"name":"good","publisher":"pub","version":"1.0.0"}"#,
            None,
        );
        assert!(matches!(
            installer.install_from_vsix(&benign_vsix, Some(".disabled.json")),
            Err(MarketplaceError::InvalidManifest(_))
        ));
        assert!(matches!(
            installer.install_from_vsix(&benign_vsix, Some(".hidden")),
            Err(MarketplaceError::InvalidManifest(_))
        ));
        assert!(!temp_ext_dir.join(".disabled.json").is_dir());

        let dot_manifest_vsix = create_test_vsix(
            r#"{"name":".disabled.json","publisher":"","version":""}"#,
            None,
        );
        assert!(matches!(
            installer.install_from_vsix(&dot_manifest_vsix, None),
            Err(MarketplaceError::InvalidManifest(_))
        ));

        installer.set_enabled("pub.good", false).unwrap();
        assert!(!installer.is_enabled("pub.good"));

        let _ = std::fs::remove_dir_all(&temp_ext_dir);
    }

    #[test]
    fn test_uninstall_by_id_dotted_name_and_publisher() {
        let temp_ext_dir = unique_temp_dir("rustcode_dotted_uninstall");
        let transport = Arc::new(MockHttpTransport::new());
        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = ExtensionInstaller::new(client, &temp_ext_dir);

        let vsix_dotted_name = create_test_vsix(
            r#"{"name":"my.ext","publisher":"","version":"1.0.0"}"#,
            None,
        );
        let vsix_dotted_pub = create_test_vsix(
            r#"{"name":"ext","publisher":"pub.org","version":"1.0.0"}"#,
            None,
        );

        installer.install_from_vsix(&vsix_dotted_name, None).unwrap();
        installer.install_from_vsix(&vsix_dotted_pub, None).unwrap();
        assert_eq!(installer.list_installed().len(), 2);

        assert!(installer.uninstall_by_id("my.ext").unwrap());
        assert!(installer.uninstall_by_id("PUB.ORG.EXT").unwrap());
        assert!(installer.list_installed().is_empty());

        let _ = std::fs::remove_dir_all(&temp_ext_dir);
    }

    #[test]
    fn test_disabled_whitespace_trimming_symmetry() {
        let temp_ext_dir = unique_temp_dir("rustcode_disabled_ws");
        let transport = Arc::new(MockHttpTransport::new());
        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = ExtensionInstaller::new(client, &temp_ext_dir);

        installer.set_enabled(" pub.space ", false).unwrap();
        assert!(!installer.is_enabled(" pub.space "));
        assert!(!installer.is_enabled("pub.space"));

        installer.set_enabled("  pub.space  ", true).unwrap();
        assert!(installer.is_enabled(" pub.space "));
        assert!(installer.is_enabled("pub.space"));

        let _ = std::fs::remove_dir_all(&temp_ext_dir);
    }

    #[test]
    fn test_install_rejects_symlink_target_dir() {
        let temp_root = unique_temp_dir("rustcode_symlink_target");
        let ext_dir = temp_root.join("extensions");
        let outside_dir = temp_root.join("outside_target");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside_dir, ext_dir.join("pub.symtarget-1.0.0")).unwrap();
            let transport = Arc::new(MockHttpTransport::new());
            let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
            let installer = ExtensionInstaller::new(client, &ext_dir);

            let vsix = create_test_vsix(
                r#"{"name":"symtarget","publisher":"pub","version":"1.0.0"}"#,
                None,
            );
            let res = installer.install_from_vsix(&vsix, None);
            assert!(matches!(res, Err(MarketplaceError::InvalidManifest(_))));
            assert!(!outside_dir.join("package.json").exists());
        }

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
