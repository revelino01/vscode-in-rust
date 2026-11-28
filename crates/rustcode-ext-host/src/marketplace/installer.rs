//! Extension installer orchestrating download, extraction, manifest discovery, and contribution registration.

use super::client::MarketplaceClient;
use super::downloader::ExtensionDownloader;
use super::types::MarketplaceError;
use crate::manifest::{ExtensionLoader, LoadedExtension};
use rustcode_base::disposable::DisposableStore;
use rustcode_platform::commands::CommandRegistry;
use rustcode_platform::theme::ThemeService;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledExtensionMeta {
    pub pre_release: bool,
    pub installed_from_vsix: bool,
    pub installed_version: String,
    #[serde(default)]
    pub installed_at_epoch_secs: u64,
}

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

    pub fn client(&self) -> &Arc<MarketplaceClient> {
        &self.client
    }

    /// Canonical extension ID (`<publisher>.<name>`, or `<name>` if publisher is empty).
    pub fn extension_id(publisher: &str, name: &str) -> String {
        if publisher.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", publisher, name)
        }
    }

    fn disabled_file_path(&self) -> PathBuf {
        self.extensions_dir.join(".disabled.json")
    }

    fn validate_path_segment(
        segment: &str,
        allow_empty: bool,
        field: &str,
    ) -> Result<(), MarketplaceError> {
        if segment.is_empty() {
            if allow_empty {
                return Ok(());
            }
            return Err(MarketplaceError::InvalidManifest(format!(
                "Empty {} path segment",
                field
            )));
        }

        if segment == "."
            || segment == ".."
            || segment.contains('/')
            || segment.contains('\\')
            || segment.contains('\0')
            || segment.contains("..")
        {
            return Err(MarketplaceError::InvalidManifest(format!(
                "Invalid {} path segment '{}': path traversal not allowed",
                field, segment
            )));
        }

        let mut comps = Path::new(segment).components();
        match (comps.next(), comps.next()) {
            (Some(Component::Normal(_)), None) => Ok(()),
            _ => Err(MarketplaceError::InvalidManifest(format!(
                "Invalid {} path segment '{}': must be a single normal component",
                field, segment
            ))),
        }
    }

    fn validate_target_dir(&self, folder_name: &str) -> Result<PathBuf, MarketplaceError> {
        Self::validate_path_segment(folder_name, false, "folder_name")?;
        if folder_name.starts_with('.') {
            return Err(MarketplaceError::InvalidManifest(format!(
                "Invalid folder_name '{}': dot-prefixed folder names are not allowed",
                folder_name
            )));
        }
        let target_dir = self.extensions_dir.join(folder_name);
        if target_dir.parent() != Some(self.extensions_dir.as_path()) {
            return Err(MarketplaceError::InvalidManifest(format!(
                "Target directory '{}' escapes extensions directory",
                folder_name
            )));
        }
        if let Ok(meta) = std::fs::symlink_metadata(&target_dir) {
            let is_link = meta.file_type().is_symlink() || {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
                    (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
                        && std::fs::read_link(&target_dir).is_ok()
                }
                #[cfg(not(windows))]
                {
                    false
                }
            };
            if is_link {
                return Err(MarketplaceError::InvalidManifest(format!(
                    "Target directory '{}' cannot be a symlink",
                    folder_name
                )));
            }
        }
        Ok(target_dir)
    }

    fn resolve_contribution_path(
        root_dir: &Path,
        contrib_path: &str,
        kind: &str,
    ) -> Result<PathBuf, MarketplaceError> {
        if contrib_path.is_empty() || contrib_path.contains('\0') || contrib_path.contains('\\') {
            return Err(MarketplaceError::InvalidManifest(format!(
                "Invalid {} contribution path '{}': unsafe path characters",
                kind, contrib_path
            )));
        }

        let mut rel_buf = PathBuf::new();
        for comp in Path::new(contrib_path).components() {
            match comp {
                Component::CurDir => {}
                Component::Normal(part) => rel_buf.push(part),
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(MarketplaceError::InvalidManifest(format!(
                        "Invalid {} contribution path '{}': path escapes extension root",
                        kind, contrib_path
                    )));
                }
            }
        }

        if rel_buf.as_os_str().is_empty() {
            return Err(MarketplaceError::InvalidManifest(format!(
                "Invalid {} contribution path '{}': empty relative path",
                kind, contrib_path
            )));
        }

        let resolved = root_dir.join(&rel_buf);
        if !resolved.starts_with(root_dir) {
            return Err(MarketplaceError::InvalidManifest(format!(
                "Invalid {} contribution path '{}': resolved path outside extension root",
                kind, contrib_path
            )));
        }

        if root_dir.exists() {
            let canon_root = std::fs::canonicalize(root_dir).map_err(|_| {
                MarketplaceError::InvalidManifest(format!(
                    "Invalid {} contribution path '{}': failed to resolve extension root",
                    kind, contrib_path
                ))
            })?;
            // Walk up to the deepest existing component so escaping symlink/junction ancestors
            // are caught even when the leaf file does not exist yet.
            let mut probe = resolved.as_path();
            while std::fs::symlink_metadata(probe).is_err() {
                match probe.parent() {
                    Some(parent) if parent != root_dir && parent.starts_with(root_dir) => {
                        probe = parent;
                    }
                    _ => {
                        probe = root_dir;
                        break;
                    }
                }
            }
            if probe != root_dir {
                let canon_probe = std::fs::canonicalize(probe).map_err(|_| {
                    MarketplaceError::InvalidManifest(format!(
                        "Invalid {} contribution path '{}': invalid or broken symlink target",
                        kind, contrib_path
                    ))
                })?;
                if !rustcode_platform::path_starts_with(&canon_probe, &canon_root) {
                    return Err(MarketplaceError::InvalidManifest(format!(
                        "Invalid {} contribution path '{}': symlink escapes extension root",
                        kind, contrib_path
                    )));
                }
            }
        }

        Ok(resolved)
    }

    /// Install an extension directly from raw `.vsix` archive bytes.
    pub fn install_from_vsix(
        &self,
        vsix_bytes: &[u8],
        custom_folder_name: Option<&str>,
    ) -> Result<LoadedExtension, MarketplaceError> {
        let manifest = ExtensionDownloader::read_manifest_from_vsix(vsix_bytes)?;

        let folder_name = if let Some(custom) = custom_folder_name {
            custom.to_string()
        } else {
            Self::validate_path_segment(&manifest.publisher, true, "publisher")?;
            Self::validate_path_segment(&manifest.name, false, "name")?;
            Self::validate_path_segment(&manifest.version, true, "version")?;
            if manifest.publisher.is_empty() {
                if manifest.version.is_empty() {
                    manifest.name.clone()
                } else {
                    format!("{}-{}", manifest.name, manifest.version)
                }
            } else if manifest.version.is_empty() {
                format!("{}.{}", manifest.publisher, manifest.name)
            } else {
                format!("{}.{}-{}", manifest.publisher, manifest.name, manifest.version)
            }
        };

        let target_dir = self.validate_target_dir(&folder_name)?;
        let unpacked_manifest = ExtensionDownloader::unpack_vsix(vsix_bytes, &target_dir)?;
        let _ = self.write_installed_meta(
            &target_dir,
            &InstalledExtensionMeta {
                pre_release: false,
                installed_from_vsix: true,
                installed_version: unpacked_manifest.version.clone(),
                installed_at_epoch_secs: now_epoch_secs(),
            },
        );

        Ok(LoadedExtension {
            root_dir: target_dir,
            manifest: unpacked_manifest,
            is_activated: false,
        })
    }

    /// Install an extension directly from a `.vsix` file path on disk.
    pub fn install_from_vsix_file(
        &self,
        vsix_path: &Path,
    ) -> Result<LoadedExtension, MarketplaceError> {
        if !vsix_path.is_file() {
            return Err(MarketplaceError::NotFound(format!(
                "VSIX file not found: {}",
                vsix_path.display()
            )));
        }
        let is_vsix = vsix_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("vsix"));
        if !is_vsix {
            return Err(MarketplaceError::InvalidManifest(format!(
                "Expected a .vsix package file, got '{}'",
                vsix_path.display()
            )));
        }
        let bytes = std::fs::read(vsix_path)?;
        self.install_from_vsix(&bytes, None)
    }

    pub fn read_installed_meta(&self, root_dir: &Path) -> Option<InstalledExtensionMeta> {
        let raw = std::fs::read_to_string(root_dir.join(".metadata.json")).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn write_installed_meta(
        &self,
        root_dir: &Path,
        meta: &InstalledExtensionMeta,
    ) -> Result<(), MarketplaceError> {
        let content = serde_json::to_string_pretty(meta)
            .map_err(|e| MarketplaceError::Serialization(e.to_string()))?;
        std::fs::write(root_dir.join(".metadata.json"), content)?;
        Ok(())
    }

    /// Download and install an extension from Open VSX by namespace and extension name.
    pub async fn install_from_marketplace(
        &self,
        namespace: &str,
        name: &str,
        version: Option<&str>,
    ) -> Result<LoadedExtension, MarketplaceError> {
        self.install_from_marketplace_with_options(namespace, name, version, false)
            .await
    }

    /// Download and install an extension from Open VSX with explicit pre-release option.
    pub async fn install_from_marketplace_with_options(
        &self,
        namespace: &str,
        name: &str,
        version: Option<&str>,
        pre_release: bool,
    ) -> Result<LoadedExtension, MarketplaceError> {
        Self::validate_path_segment(namespace, false, "namespace")?;
        Self::validate_path_segment(name, false, "name")?;

        let meta = if let Some(v) = version {
            Self::validate_path_segment(v, false, "version")?;
            self.client.get_version_metadata(namespace, name, v).await?
        } else if pre_release {
            self.client.get_prerelease_metadata(namespace, name).await?
        } else {
            self.client.get_extension_metadata(namespace, name).await?
        };

        Self::validate_path_segment(&meta.version, false, "version")?;

        let download_url = meta.resolved_download_url().ok_or_else(|| {
            MarketplaceError::MissingDownloadUrl(format!("{}.{}", namespace, name))
        })?;

        let vsix_bytes = self.client.download_vsix(&download_url).await?;
        let folder_name = format!("{}.{}-{}", namespace, name, meta.version);
        let target_dir = self.validate_target_dir(&folder_name)?;

        let manifest = ExtensionDownloader::unpack_vsix(&vsix_bytes, &target_dir)?;
        let _ = self.write_installed_meta(
            &target_dir,
            &InstalledExtensionMeta {
                pre_release: pre_release || meta.pre_release,
                installed_from_vsix: false,
                installed_version: manifest.version.clone(),
                installed_at_epoch_secs: now_epoch_secs(),
            },
        );

        Ok(LoadedExtension {
            root_dir: target_dir,
            manifest,
            is_activated: false,
        })
    }

    /// Install an extension along with any transitive `extensionPack` and `extensionDependencies`,
    /// protecting against dependency cycles and skipping already-installed extensions.
    pub async fn install_with_pack(
        &self,
        namespace: &str,
        name: &str,
        pre_release: bool,
    ) -> Result<Vec<LoadedExtension>, MarketplaceError> {
        let mut visited: HashSet<String> = HashSet::new();
        for existing in self.list_installed() {
            visited.insert(existing.id().to_ascii_lowercase());
        }

        let mut queue: VecDeque<(String, String, bool)> = VecDeque::new();
        let root_id = Self::extension_id(namespace, name).to_ascii_lowercase();
        visited.remove(&root_id);
        queue.push_back((namespace.to_string(), name.to_string(), pre_release));

        let mut installed = Vec::new();
        while let Some((ns, ext_name, is_pre)) = queue.pop_front() {
            let id_lower = Self::extension_id(&ns, &ext_name).to_ascii_lowercase();
            if !visited.insert(id_lower) {
                continue;
            }
            let ext = self
                .install_from_marketplace_with_options(&ns, &ext_name, None, is_pre)
                .await?;

            for dep_id in ext
                .manifest
                .extension_pack
                .iter()
                .chain(ext.manifest.extension_dependencies.iter())
            {
                if let Some((dep_ns, dep_name)) = dep_id.trim().split_once('.') {
                    let dep_lower = dep_id.trim().to_ascii_lowercase();
                    if !visited.contains(&dep_lower) {
                        queue.push_back((dep_ns.to_string(), dep_name.to_string(), false));
                    }
                }
            }
            installed.push(ext);
        }

        Ok(installed)
    }

    /// List installed extensions by scanning `self.extensions_dir` subdirectories for `package.json`.
    pub fn list_installed(&self) -> Vec<LoadedExtension> {
        if !self.extensions_dir.is_dir() {
            return Vec::new();
        }

        let Ok(entries) = std::fs::read_dir(&self.extensions_dir) else {
            return Vec::new();
        };

        let mut installed = Vec::new();
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let dir_path = entry.path();
            let manifest_path = dir_path.join("package.json");
            if !manifest_path.is_file() {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            if let Ok(manifest) = ExtensionLoader::parse_manifest(&raw) {
                installed.push(LoadedExtension {
                    root_dir: dir_path,
                    manifest,
                    is_activated: false,
                });
            }
        }

        installed.sort_by(|a, b| {
            (
                a.manifest.publisher.to_ascii_lowercase(),
                a.manifest.name.to_ascii_lowercase(),
                &a.manifest.version,
                &a.root_dir,
            )
                .cmp(&(
                    b.manifest.publisher.to_ascii_lowercase(),
                    b.manifest.name.to_ascii_lowercase(),
                    &b.manifest.version,
                    &b.root_dir,
                ))
        });

        installed
    }

    /// Load the set of disabled extension IDs from `.disabled.json` (read-only when absent).
    pub fn load_disabled_set(&self) -> HashSet<String> {
        let path = self.disabled_file_path();
        let Ok(raw) = std::fs::read_to_string(path) else {
            return HashSet::new();
        };
        serde_json::from_str::<Vec<String>>(&raw)
            .map(|list| list.into_iter().collect())
            .unwrap_or_default()
    }

    /// Check whether an extension ID is enabled (true by default when not in `.disabled.json`).
    pub fn is_enabled(&self, id: &str) -> bool {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return true;
        }
        !self
            .load_disabled_set()
            .iter()
            .any(|entry| entry.trim().eq_ignore_ascii_case(trimmed))
    }

    /// Persist enabled/disabled state for an extension ID in `<extensions_dir>/.disabled.json`.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), MarketplaceError> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(MarketplaceError::InvalidManifest(
                "Extension ID cannot be empty".to_string(),
            ));
        }

        std::fs::create_dir_all(&self.extensions_dir)?;
        let mut disabled = self.load_disabled_set();
        disabled.retain(|entry| !entry.trim().eq_ignore_ascii_case(trimmed));
        if !enabled {
            disabled.insert(trimmed.to_string());
        }

        let mut sorted: Vec<String> = disabled.into_iter().collect();
        sorted.sort_by_key(|a| a.to_ascii_lowercase());
        let json_str = serde_json::to_string_pretty(&sorted)
            .map_err(|e| MarketplaceError::Serialization(e.to_string()))?;
        std::fs::write(self.disabled_file_path(), json_str)?;
        Ok(())
    }

    /// Uninstall an extension by publisher and name, cleaning matching directories and `.disabled.json`.
    /// Returns `Ok(true)` if one or more directories were removed, `Ok(false)` otherwise.
    pub fn uninstall(&self, publisher: &str, name: &str) -> Result<bool, MarketplaceError> {
        Self::validate_path_segment(publisher, true, "publisher")?;
        Self::validate_path_segment(name, false, "name")?;

        if !self.extensions_dir.is_dir() {
            return Ok(false);
        }

        let exact_id = if publisher.is_empty() {
            name.to_ascii_lowercase()
        } else {
            format!("{}.{}", publisher, name).to_ascii_lowercase()
        };
        let versioned_prefix = format!("{}-", exact_id);

        let mut removed = false;
        for entry in std::fs::read_dir(&self.extensions_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path();
            if path.parent() != Some(self.extensions_dir.as_path()) {
                continue;
            }

            let manifest_path = path.join("package.json");
            let parsed_manifest = if manifest_path.is_file() {
                std::fs::read_to_string(&manifest_path)
                    .ok()
                    .and_then(|raw| ExtensionLoader::parse_manifest(&raw).ok())
            } else {
                None
            };

            let matches = if let Some(manifest) = parsed_manifest {
                manifest.publisher.eq_ignore_ascii_case(publisher)
                    && manifest.name.eq_ignore_ascii_case(name)
            } else {
                let folder_lower = entry.file_name().to_string_lossy().to_ascii_lowercase();
                !folder_lower.starts_with('.')
                    && (folder_lower == exact_id
                        || folder_lower.strip_prefix(&versioned_prefix).is_some_and(|rest| {
                            rest.chars().next().is_some_and(|c| c.is_ascii_digit())
                        }))
            };

            if matches {
                std::fs::remove_dir_all(&path)?;
                removed = true;
            }
        }

        if removed && self.disabled_file_path().exists() {
            let ext_id = Self::extension_id(publisher, name);
            if !self.is_enabled(&ext_id) {
                self.set_enabled(&ext_id, true)?;
            }
        }

        Ok(removed)
    }

    /// Convenience helper to uninstall an extension using its canonical `<publisher>.<name>` ID.
    pub fn uninstall_by_id(&self, id: &str) -> Result<bool, MarketplaceError> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(MarketplaceError::InvalidManifest(
                "Extension ID cannot be empty".to_string(),
            ));
        }

        let mut matched_pairs: Vec<(String, String)> = Vec::new();
        for ext in self.list_installed() {
            if Self::extension_id(&ext.manifest.publisher, &ext.manifest.name)
                .eq_ignore_ascii_case(trimmed)
            {
                let pair = (ext.manifest.publisher, ext.manifest.name);
                if !matched_pairs.iter().any(|(p, n)| {
                    p.eq_ignore_ascii_case(&pair.0) && n.eq_ignore_ascii_case(&pair.1)
                }) {
                    matched_pairs.push(pair);
                }
            }
        }

        if !matched_pairs.is_empty() {
            let mut removed = false;
            for (publisher, name) in matched_pairs {
                if self.uninstall(&publisher, &name)? {
                    removed = true;
                }
            }
            return Ok(removed);
        }

        if let Some((publisher, name)) = trimmed.split_once('.') {
            self.uninstall(publisher, name)
        } else {
            self.uninstall("", trimmed)
        }
    }

    /// Register contributed commands, themes, and grammars from the unpacked extension into runtime services.
    /// Returns the number of registered items and a `DisposableStore` managing lifetime of registered commands.
    pub fn register_contributions<F>(
        &self,
        ext: &LoadedExtension,
        command_registry: &CommandRegistry,
        mut theme_service: Option<&mut ThemeService>,
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
                for theme_contrib in themes {
                    let theme_path =
                        Self::resolve_contribution_path(&ext.root_dir, &theme_contrib.path, "theme")?;
                    if let Some(ts) = theme_service.as_deref_mut() {
                        if theme_path.exists() {
                            if let Ok(theme_content) = std::fs::read_to_string(&theme_path) {
                                if let Ok(parsed_theme) =
                                    ThemeService::parse_theme_json(&theme_content)
                                {
                                    ts.set_theme(parsed_theme);
                                    registered_count += 1;
                                }
                            }
                        }
                    }
                }
            }

            // 3. Register contributed icon themes & product icon themes
            if let Some(icon_themes) = &contributes.icon_themes {
                for it in icon_themes {
                    let icon_path =
                        Self::resolve_contribution_path(&ext.root_dir, &it.path, "iconTheme")?;
                    if let Some(ts) = theme_service.as_deref_mut() {
                        if icon_path.exists()
                            && ts
                                .load_icon_theme_from_disk(&it.id, &it.label, &icon_path)
                                .is_ok()
                        {
                            registered_count += 1;
                        }
                    }
                }
            }
            if let Some(prod_themes) = &contributes.product_icon_themes {
                for pt in prod_themes {
                    let pt_path = Self::resolve_contribution_path(
                        &ext.root_dir,
                        &pt.path,
                        "productIconTheme",
                    )?;
                    if let Some(ts) = theme_service.as_deref_mut() {
                        if pt_path.exists()
                            && ts
                                .load_product_icon_theme_from_disk(&pt.id, &pt.label, &pt_path)
                                .is_ok()
                        {
                            registered_count += 1;
                        }
                    }
                }
            }

            // 4. Register contributed grammars
            if let Some(grammars) = &contributes.grammars {
                for g in grammars {
                    let grammar_path =
                        Self::resolve_contribution_path(&ext.root_dir, &g.path, "grammar")?;
                    if grammar_path.exists() && grammar_registrar(&g.language, &grammar_path).is_ok()
                    {
                        registered_count += 1;
                    }
                }
            }
        }

        Ok((registered_count, store))
    }
}

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
