//! Extension installer orchestrating download, extraction, manifest discovery, and contribution registration.

use super::client::MarketplaceClient;
use super::downloader::ExtensionDownloader;
use super::types::MarketplaceError;
use crate::manifest::LoadedExtension;
use rustcode_base::disposable::DisposableStore;
use rustcode_platform::commands::CommandRegistry;
use rustcode_platform::theme::ThemeService;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct ExtensionInstaller {
    client: Arc<MarketplaceClient>,
    extensions_dir: PathBuf,
}

impl ExtensionInstaller {
    pub fn new(client: Arc<MarketplaceClient>, extensions_dir: impl Into<PathBuf>) -> Self {
        Self {
            client,
            extensions_dir: extensions_dir.into(),
        }
    }

    pub fn with_default_dir(client: Arc<MarketplaceClient>) -> Self {
        Self::new(client, ExtensionDownloader::default_extensions_dir())
    }

    pub fn extensions_dir(&self) -> &Path {
        &self.extensions_dir
    }

    /// Install an extension directly from raw `.vsix` archive bytes.
    pub fn install_from_vsix(
        &self,
        vsix_bytes: &[u8],
        custom_folder_name: Option<&str>,
    ) -> Result<LoadedExtension, MarketplaceError> {
        let manifest = ExtensionDownloader::read_manifest_from_vsix(vsix_bytes)?;

        let folder_name = custom_folder_name
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                format!(
                    "{}.{}-{}",
                    manifest.publisher, manifest.name, manifest.version
                )
            });

        let target_dir = self.extensions_dir.join(folder_name);
        let unpacked_manifest = ExtensionDownloader::unpack_vsix(vsix_bytes, &target_dir)?;

        Ok(LoadedExtension {
            root_dir: target_dir,
            manifest: unpacked_manifest,
            is_activated: false,
        })
    }

    /// Download and install an extension from Open VSX by namespace and extension name.
    pub async fn install_from_marketplace(
        &self,
        namespace: &str,
        name: &str,
        version: Option<&str>,
    ) -> Result<LoadedExtension, MarketplaceError> {
        let meta = if let Some(v) = version {
            self.client.get_version_metadata(namespace, name, v).await?
        } else {
            self.client.get_extension_metadata(namespace, name).await?
        };

        let download_url = meta.resolved_download_url().ok_or_else(|| {
            MarketplaceError::MissingDownloadUrl(format!("{}.{}", namespace, name))
        })?;

        let vsix_bytes = self.client.download_vsix(&download_url).await?;
        let folder_name = format!("{}.{}-{}", namespace, name, meta.version);
        let target_dir = self.extensions_dir.join(folder_name);

        let manifest = ExtensionDownloader::unpack_vsix(&vsix_bytes, &target_dir)?;

        Ok(LoadedExtension {
            root_dir: target_dir,
            manifest,
            is_activated: false,
        })
    }

    /// Register contributed commands, themes, and grammars from the unpacked extension into runtime services.
    /// Returns the number of registered items and a `DisposableStore` managing lifetime of registered commands.
    pub fn register_contributions<F>(
        &self,
        ext: &LoadedExtension,
        command_registry: &CommandRegistry,
        theme_service: Option<&mut ThemeService>,
        mut grammar_registrar: F,
    ) -> Result<(usize, DisposableStore), MarketplaceError>
    where
        F: FnMut(&str, &Path) -> Result<(), String>,
    {
        let mut registered_count = 0;
        let mut store = DisposableStore::new();

        if let Some(contributes) = &ext.manifest.contributes {
            // 1. Register contributed commands
            if let Some(commands) = &contributes.commands {
                for cmd in commands {
                    let cmd_id = cmd.command.clone();
                    let title = cmd.title.clone();
                    let cat = cmd.category.clone();
                    let id_for_fn = cmd_id.clone();
                    let disp = command_registry.register(&cmd_id, move |_args| {
                        Ok(json!({
                            "command": id_for_fn,
                            "title": title,
                            "category": cat
                        }))
                    });
                    store.add_boxed(disp);
                    registered_count += 1;
                }
            }

            // 2. Register contributed themes
            if let Some(themes) = &contributes.themes {
                if let Some(ts) = theme_service {
                    for theme_contrib in themes {
                        let rel_path = theme_contrib.path.trim_start_matches('.').trim_start_matches('/');
                        let theme_path = ext.root_dir.join(rel_path);
                        if theme_path.exists() {
                            if let Ok(theme_content) = std::fs::read_to_string(&theme_path) {
                                if let Ok(parsed_theme) = ThemeService::parse_theme_json(&theme_content) {
                                    ts.set_theme(parsed_theme);
                                    registered_count += 1;
                                }
                            }
                        }
                    }
                }
            }

            // 3. Register contributed grammars
            if let Some(grammars) = &contributes.grammars {
                for g in grammars {
                    let rel_path = g.path.trim_start_matches('.').trim_start_matches('/');
                    let grammar_path = ext.root_dir.join(rel_path);
                    if grammar_path.exists() && grammar_registrar(&g.language, &grammar_path).is_ok() {
                        registered_count += 1;
                    }
                }
            }
        }

        Ok((registered_count, store))
    }
}
