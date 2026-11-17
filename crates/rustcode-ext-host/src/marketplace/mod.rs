//! Open VSX Marketplace Client & Extension Downloader.

pub mod client;
pub mod downloader;
pub mod installer;
pub mod transport;
pub mod types;

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
}
