//! Extensions Marketplace Sidebar View (`workbench.view.extensions`).
//!
//! Renders a search bar, collapsible `INSTALLED` and `MARKETPLACE` sections,
//! and interactive Install / Enable / Disable / Uninstall controls wired to
//! `rustcode_ext_host::marketplace`.

use parking_lot::RwLock;
use rustcode_ext_host::manifest::LoadedExtension;
use rustcode_ext_host::marketplace::{
    curated_catalog, search_curated_catalog, ExtensionInstaller, ExtensionMetadata,
    MarketplaceClient, MarketplaceError, SearchResponse,
};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, KeyEvent, MouseButton, Point,
    Quad, Rect, Scene, View, WorkbenchThemeStyles,
};
use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

/// Return the curated offline Open VSX catalog.
pub fn curated_marketplace_extensions() -> Vec<ExtensionMetadata> {
    curated_catalog()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledExtensionItem {
    pub id: String,
    pub publisher: String,
    pub name: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub enabled: bool,
    pub root_dir: PathBuf,
}

impl InstalledExtensionItem {
    pub fn from_loaded(ext: &LoadedExtension, enabled: bool) -> Self {
        let id = ExtensionInstaller::extension_id(&ext.manifest.publisher, &ext.manifest.name);
        let display_name = ext
            .manifest
            .display_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| ext.manifest.name.clone());
        let description = ext.manifest.description.clone().unwrap_or_default();
        Self {
            id,
            publisher: ext.manifest.publisher.clone(),
            name: ext.manifest.name.clone(),
            version: ext.manifest.version.clone(),
            display_name,
            description,
            enabled,
            root_dir: ext.root_dir.clone(),
        }
    }
}

impl From<(&LoadedExtension, bool)> for InstalledExtensionItem {
    fn from((ext, enabled): (&LoadedExtension, bool)) -> Self {
        Self::from_loaded(ext, enabled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketplaceExtensionItem {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub download_url: Option<String>,
    pub is_installed: bool,
}

impl MarketplaceExtensionItem {
    pub fn from_metadata(meta: &ExtensionMetadata, installed_ids: &HashSet<String>) -> Self {
        let id = ExtensionInstaller::extension_id(&meta.namespace, &meta.name);
        let is_installed = installed_ids
            .iter()
            .any(|inst| inst.eq_ignore_ascii_case(&id));
        let display_name = meta
            .display_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| meta.name.clone());
        let description = meta.description.clone().unwrap_or_default();
        Self {
            id,
            namespace: meta.namespace.clone(),
            name: meta.name.clone(),
            version: meta.version.clone(),
            display_name,
            description,
            download_url: meta.resolved_download_url(),
            is_installed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionsViewAction {
    Search(String),
    Install {
        namespace: String,
        name: String,
        version: Option<String>,
    },
    Uninstall {
        id: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    SelectExtension(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionsClickTarget {
    SearchInput,
    ClearSearch,
    ToggleInstalledSection,
    ToggleMarketplaceSection,
    SelectRow(String),
    EnableBtn(String),
    DisableBtn(String),
    UninstallBtn(String),
    InstallBtn {
        namespace: String,
        name: String,
        version: String,
    },
}

pub struct ExtensionsSidebarView {
    pub search_query: String,
    pub search_focused: bool,
    pub installed_collapsed: bool,
    pub marketplace_collapsed: bool,
    pub installed: Vec<InstalledExtensionItem>,
    pub marketplace_results: Vec<MarketplaceExtensionItem>,
    pub catalog: Vec<ExtensionMetadata>,
    pub selected_id: Option<String>,
    pub status_message: Option<String>,
    pub is_loading: bool,
    pub scroll_top: f32,
    pub installer: Option<Arc<ExtensionInstaller>>,
    pub client: Option<Arc<MarketplaceClient>>,
    pub last_action: Option<ExtensionsViewAction>,
    pub click_targets: RwLock<Vec<(Rect, ExtensionsClickTarget)>>,
    live_search_query: Option<String>,
    last_font: RwLock<FontMetrics>,
}

impl Default for ExtensionsSidebarView {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtensionsSidebarView {
    /// Create a non-mutating `ExtensionsSidebarView` backed by the pure-Rust curated catalog
    /// without touching `$HOME/.rustcode` or spawning `curl`.
    pub fn new() -> Self {
        let catalog = curated_marketplace_extensions();
        let empty_ids = HashSet::new();
        let marketplace_results = catalog
            .iter()
            .map(|meta| MarketplaceExtensionItem::from_metadata(meta, &empty_ids))
            .collect();

        Self {
            search_query: String::new(),
            search_focused: false,
            installed_collapsed: false,
            marketplace_collapsed: false,
            installed: Vec::new(),
            marketplace_results,
            catalog,
            selected_id: None,
            status_message: None,
            is_loading: false,
            scroll_top: 0.0,
            installer: None,
            client: None,
            last_action: None,
            click_targets: RwLock::new(Vec::new()),
            live_search_query: None,
            last_font: RwLock::new(FontMetrics::default_monospace()),
        }
    }

    pub fn with_installer(mut self, installer: Arc<ExtensionInstaller>) -> Self {
        self.set_installer(installer);
        self
    }

    pub fn set_installer(&mut self, installer: Arc<ExtensionInstaller>) {
        if self.client.is_none() {
            self.client = Some(Arc::clone(installer.client()));
        }
        self.installer = Some(installer);
        self.refresh_installed();
    }

    pub fn with_client(mut self, client: Arc<MarketplaceClient>) -> Self {
        self.set_client(client);
        self
    }

    pub fn set_client(&mut self, client: Arc<MarketplaceClient>) {
        self.client = Some(client);
    }

    fn installed_id_set(&self) -> HashSet<String> {
        self.installed
            .iter()
            .map(|ext| ext.id.to_ascii_lowercase())
            .collect()
    }

    fn sync_installed_flags(&mut self) {
        let installed_ids = self.installed_id_set();
        for item in &mut self.marketplace_results {
            item.is_installed = installed_ids.contains(&item.id.to_ascii_lowercase());
        }
    }

    /// Reload installed extensions from `self.installer` (if present) and sync `is_installed` badges.
    pub fn refresh_installed(&mut self) {
        if let Some(installer) = &self.installer {
            let loaded = installer.list_installed();
            let mut items: Vec<InstalledExtensionItem> = loaded
                .iter()
                .map(|ext| {
                    let id = ExtensionInstaller::extension_id(
                        &ext.manifest.publisher,
                        &ext.manifest.name,
                    );
                    let enabled = installer.is_enabled(&id);
                    InstalledExtensionItem::from_loaded(ext, enabled)
                })
                .collect();
            items.sort_by(|a, b| {
                a.display_name
                    .to_ascii_lowercase()
                    .cmp(&b.display_name.to_ascii_lowercase())
                    .then_with(|| a.id.to_ascii_lowercase().cmp(&b.id.to_ascii_lowercase()))
            });
            self.installed = items;
        }
        self.sync_installed_flags();
    }

    pub fn filtered_installed(&self) -> Vec<&InstalledExtensionItem> {
        let q = self.search_query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return self.installed.iter().collect();
        }
        self.installed
            .iter()
            .filter(|ext| {
                ext.id.to_ascii_lowercase().contains(&q)
                    || ext.display_name.to_ascii_lowercase().contains(&q)
                    || ext.publisher.to_ascii_lowercase().contains(&q)
                    || ext.name.to_ascii_lowercase().contains(&q)
                    || ext.description.to_ascii_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn filtered_marketplace(&self) -> Vec<&MarketplaceExtensionItem> {
        let q = self.search_query.trim().to_ascii_lowercase();
        if q.is_empty() || self.live_search_query.as_deref() == Some(self.search_query.as_str()) {
            return self.marketplace_results.iter().collect();
        }
        self.marketplace_results
            .iter()
            .filter(|ext| {
                ext.id.to_ascii_lowercase().contains(&q)
                    || ext.display_name.to_ascii_lowercase().contains(&q)
                    || ext.namespace.to_ascii_lowercase().contains(&q)
                    || ext.name.to_ascii_lowercase().contains(&q)
                    || ext.description.to_ascii_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn set_marketplace_results(&mut self, response: SearchResponse) {
        let installed_ids = self.installed_id_set();
        self.marketplace_results = response
            .extensions
            .iter()
            .map(|meta| MarketplaceExtensionItem::from_metadata(meta, &installed_ids))
            .collect();
        self.live_search_query = Some(self.search_query.clone());
    }

    /// Filter the local curated catalog using `query` and emit `ExtensionsViewAction::Search`.
    pub fn apply_search_query(&mut self, query: &str) {
        self.search_query = query.to_string();
        self.scroll_top = 0.0;
        self.live_search_query = None;

        let trimmed = query.trim().to_ascii_lowercase();
        let installed_ids = self.installed_id_set();
        let matching: Vec<MarketplaceExtensionItem> = if trimmed.is_empty() {
            self.catalog
                .iter()
                .map(|m| MarketplaceExtensionItem::from_metadata(m, &installed_ids))
                .collect()
        } else {
            self.catalog
                .iter()
                .filter(|m| {
                    let id = ExtensionInstaller::extension_id(&m.namespace, &m.name)
                        .to_ascii_lowercase();
                    let disp = m
                        .display_name
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    let desc = m.description.as_deref().unwrap_or("").to_ascii_lowercase();
                    id.contains(&trimmed)
                        || m.namespace.to_ascii_lowercase().contains(&trimmed)
                        || m.name.to_ascii_lowercase().contains(&trimmed)
                        || disp.contains(&trimmed)
                        || desc.contains(&trimmed)
                })
                .map(|m| MarketplaceExtensionItem::from_metadata(m, &installed_ids))
                .collect()
        };
        self.marketplace_results = matching;
        self.last_action = Some(ExtensionsViewAction::Search(query.to_string()));
    }

    /// Enable or disable an installed extension, persisting `.disabled.json` if an installer is attached.
    pub fn set_extension_enabled(
        &mut self,
        id: &str,
        enabled: bool,
    ) -> Result<bool, MarketplaceError> {
        if let Some(installer) = &self.installer {
            installer.set_enabled(id, enabled)?;
        }

        let mut updated = false;
        for item in &mut self.installed {
            if item.id.eq_ignore_ascii_case(id) {
                item.enabled = enabled;
                updated = true;
            }
        }

        if updated {
            self.status_message = Some(if enabled {
                format!("Enabled {}", id)
            } else {
                format!("Disabled {}", id)
            });
            self.last_action = Some(if enabled {
                ExtensionsViewAction::Enable { id: id.to_string() }
            } else {
                ExtensionsViewAction::Disable { id: id.to_string() }
            });
        }

        Ok(updated)
    }

    /// Uninstall an extension by ID (`<publisher>.<name>` or `<name>`).
    pub fn uninstall_extension(&mut self, id: &str) -> Result<bool, MarketplaceError> {
        let removed = if let Some(installer) = &self.installer {
            let (publisher, name) = if let Some(existing) = self
                .installed
                .iter()
                .find(|item| item.id.eq_ignore_ascii_case(id))
            {
                (existing.publisher.clone(), existing.name.clone())
            } else if let Some((p, n)) = id.split_once('.') {
                (p.to_string(), n.to_string())
            } else {
                (String::new(), id.to_string())
            };
            let ok = installer.uninstall(&publisher, &name)?;
            self.refresh_installed();
            ok
        } else {
            let before = self.installed.len();
            self.installed
                .retain(|item| !item.id.eq_ignore_ascii_case(id));
            self.sync_installed_flags();
            self.installed.len() != before
        };

        if removed {
            if self
                .selected_id
                .as_deref()
                .is_some_and(|sel| sel.eq_ignore_ascii_case(id))
            {
                self.selected_id = None;
            }
            self.status_message = Some(format!("Uninstalled {}", id));
            self.last_action = Some(ExtensionsViewAction::Uninstall { id: id.to_string() });
        }

        Ok(removed)
    }

    /// Install an extension directly from `.vsix` bytes via the attached `ExtensionInstaller`.
    pub fn install_from_vsix_bytes(
        &mut self,
        vsix_bytes: &[u8],
    ) -> Result<InstalledExtensionItem, MarketplaceError> {
        let installer = self.installer.as_ref().ok_or_else(|| {
            MarketplaceError::NotFound("No ExtensionInstaller configured".to_string())
        })?;
        let loaded = installer.install_from_vsix(vsix_bytes, None)?;
        let id = ExtensionInstaller::extension_id(
            &loaded.manifest.publisher,
            &loaded.manifest.name,
        );
        let enabled = installer.is_enabled(&id);
        let item = InstalledExtensionItem::from_loaded(&loaded, enabled);
        self.refresh_installed();
        self.status_message = Some(format!("Installed {}", item.id));
        Ok(item)
    }

    fn effective_client(&self) -> Option<Arc<MarketplaceClient>> {
        self.client
            .clone()
            .or_else(|| self.installer.as_ref().map(|i| Arc::clone(i.client())))
    }

    /// Execute a marketplace search asynchronously via `client` (or fall back to the curated catalog).
    pub async fn execute_search(&mut self) -> Result<SearchResponse, MarketplaceError> {
        self.is_loading = true;
        self.scroll_top = 0.0;
        let query = self.search_query.clone();

        let res = if let Some(client) = self.effective_client() {
            client.search(&query, Some(20), Some(0)).await
        } else {
            Ok(search_curated_catalog(&query))
        };

        self.is_loading = false;
        match res {
            Ok(response) => {
                let count = response.extensions.len();
                self.set_marketplace_results(response.clone());
                self.status_message = Some(format!("Found {} extension(s)", count));
                self.last_action = Some(ExtensionsViewAction::Search(query));
                Ok(response)
            }
            Err(err) => {
                // Graceful fallback to curated catalog when offline/unmapped, while still returning Err
                // if caller explicitly inspects the Result.
                let fallback = search_curated_catalog(&query);
                self.set_marketplace_results(fallback);
                self.live_search_query = None;
                self.status_message = Some(format!("Offline search fallback ({})", err));
                self.last_action = Some(ExtensionsViewAction::Search(query));
                Err(err)
            }
        }
    }

    /// Execute a marketplace install asynchronously via `self.installer`.
    pub async fn execute_install(
        &mut self,
        namespace: &str,
        name: &str,
        version: Option<&str>,
    ) -> Result<InstalledExtensionItem, MarketplaceError> {
        let installer = self.installer.as_ref().ok_or_else(|| {
            MarketplaceError::NotFound("No ExtensionInstaller configured".to_string())
        })?;

        self.is_loading = true;
        let loaded = installer
            .install_from_marketplace(namespace, name, version)
            .await;
        self.is_loading = false;

        let loaded = loaded?;
        let id = ExtensionInstaller::extension_id(
            &loaded.manifest.publisher,
            &loaded.manifest.name,
        );
        let enabled = installer.is_enabled(&id);
        let item = InstalledExtensionItem::from_loaded(&loaded, enabled);
        self.refresh_installed();
        self.selected_id = Some(item.id.clone());
        self.status_message = Some(format!("Installed {}", item.id));
        self.last_action = Some(ExtensionsViewAction::Install {
            namespace: namespace.to_string(),
            name: name.to_string(),
            version: version.map(|v| v.to_string()),
        });
        Ok(item)
    }

    /// Execute search synchronously (panic-safe inside an active `#[tokio::test]` runtime).
    pub fn execute_search_sync(&mut self) -> Result<SearchResponse, MarketplaceError> {
        if let Some(client) = self.effective_client() {
            self.is_loading = true;
            self.scroll_top = 0.0;
            let query = self.search_query.clone();
            let res = block_on_future(async move { client.search(&query, Some(20), Some(0)).await });
            self.is_loading = false;

            match res {
                Ok(response) => {
                    let count = response.extensions.len();
                    self.set_marketplace_results(response.clone());
                    self.status_message = Some(format!("Found {} extension(s)", count));
                    self.last_action = Some(ExtensionsViewAction::Search(self.search_query.clone()));
                    Ok(response)
                }
                Err(err) => {
                    let fallback = search_curated_catalog(&self.search_query);
                    self.set_marketplace_results(fallback);
                    self.live_search_query = None;
                    self.status_message = Some(format!("Offline search fallback ({})", err));
                    self.last_action = Some(ExtensionsViewAction::Search(self.search_query.clone()));
                    Err(err)
                }
            }
        } else {
            let query = self.search_query.clone();
            self.apply_search_query(&query);
            let resp = search_curated_catalog(&query);
            self.status_message = Some(format!("Found {} extension(s)", resp.extensions.len()));
            Ok(resp)
        }
    }

    /// Execute install synchronously (panic-safe inside an active `#[tokio::test]` runtime).
    pub fn execute_install_sync(
        &mut self,
        namespace: &str,
        name: &str,
        version: Option<&str>,
    ) -> Result<InstalledExtensionItem, MarketplaceError> {
        let installer = self.installer.as_ref().ok_or_else(|| {
            MarketplaceError::NotFound("No ExtensionInstaller configured".to_string())
        })?;
        let installer_arc = Arc::clone(installer);
        let ns = namespace.to_string();
        let nm = name.to_string();
        let ver = version.map(|v| v.to_string());

        self.is_loading = true;
        let res = block_on_future(async move {
            installer_arc
                .install_from_marketplace(&ns, &nm, ver.as_deref())
                .await
        });
        self.is_loading = false;

        let loaded = res?;
        let id = ExtensionInstaller::extension_id(
            &loaded.manifest.publisher,
            &loaded.manifest.name,
        );
        let enabled = installer.is_enabled(&id);
        let item = InstalledExtensionItem::from_loaded(&loaded, enabled);
        self.refresh_installed();
        self.selected_id = Some(item.id.clone());
        self.status_message = Some(format!("Installed {}", item.id));
        self.last_action = Some(ExtensionsViewAction::Install {
            namespace: namespace.to_string(),
            name: name.to_string(),
            version: version.map(|v| v.to_string()),
        });
        Ok(item)
    }

    fn compute_total_content_height(&self) -> f32 {
        let section_h = 24.0;
        let row_h = 54.0;
        let installed_count = self.filtered_installed().len();
        let marketplace_count = self.filtered_marketplace().len();

        let mut total = section_h;
        if !self.installed_collapsed {
            total += if installed_count == 0 {
                24.0
            } else {
                installed_count as f32 * row_h
            };
        }
        total += section_h;
        if !self.marketplace_collapsed {
            total += if marketplace_count == 0 {
                24.0
            } else {
                marketplace_count as f32 * row_h
            };
        }
        total
    }

    fn push_clipped_target(
        targets: &mut Vec<(Rect, ExtensionsClickTarget)>,
        rect: Rect,
        clip: Rect,
        target: ExtensionsClickTarget,
    ) {
        if let Some(clipped) = rect.intersect(&clip) {
            if clipped.width() > 0.0 && clipped.height() > 0.0 {
                targets.push((clipped, target));
            }
        }
    }

    pub fn compute_layout(&self, bounds: Rect) -> ExtensionsSidebarLayout {
        let header_h = 28.0;
        let input_h = 26.0;
        let safe_bounds = if crate::is_finite_rect(bounds)
            && (bounds.min_y() + header_h + input_h + 6.0).is_finite()
        {
            Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), bounds.height())
        } else {
            Rect::zero()
        };

        let mut cur_y = safe_bounds.min_y() + header_h;

        let search_rect = Rect::from_xywh(
            safe_bounds.min_x() + 8.0,
            cur_y,
            (safe_bounds.width() - 16.0).max(0.0),
            input_h,
        );
        cur_y += input_h + 6.0;

        let footer_h = if self.status_message.is_some() { 20.0 } else { 0.0 };
        let body_h = (safe_bounds.max_y() - cur_y - footer_h).max(0.0);
        let body_rect = Rect::from_xywh(safe_bounds.min_x(), cur_y, safe_bounds.width(), body_h);

        let installed_items = self.filtered_installed();
        let marketplace_items = self.filtered_marketplace();
        let total_h = self.compute_total_content_height();

        let max_scroll = (total_h - body_h).max(0.0);
        let max_scroll = if max_scroll.is_finite() { max_scroll } else { 0.0 };
        let safe_scroll_top = if self.scroll_top.is_finite() {
            self.scroll_top
        } else {
            0.0
        };
        let scroll = safe_scroll_top.clamp(0.0, max_scroll);

        let section_h = 24.0;
        let row_h = 54.0;
        let mut y = body_rect.min_y() - scroll;

        let installed_header_rect =
            Rect::from_xywh(safe_bounds.min_x(), y, safe_bounds.width(), section_h);
        y += section_h;

        let inst_body_start = y;
        let mut installed_rows = Vec::new();
        if !self.installed_collapsed {
            if installed_items.is_empty() {
                y += 24.0;
            } else {
                for item in &installed_items {
                    installed_rows.push((
                        item.id.clone(),
                        Rect::from_xywh(safe_bounds.min_x(), y, safe_bounds.width(), row_h),
                    ));
                    y += row_h;
                }
            }
        }
        let clamp_body_y = |val: f32| -> f32 {
            let min_y = body_rect.min_y();
            let max_y = body_rect.max_y();
            if val.is_finite() && min_y.is_finite() && max_y.is_finite() && min_y <= max_y {
                val.clamp(min_y, max_y)
            } else if min_y.is_finite() {
                min_y
            } else {
                0.0
            }
        };
        let inst_raw_rect = Rect::from_xywh(
            safe_bounds.min_x(),
            inst_body_start,
            safe_bounds.width(),
            (y - inst_body_start).max(0.0),
        );
        let installed_list_rect = inst_raw_rect.intersect(&body_rect).unwrap_or_else(|| {
            Rect::from_xywh(
                safe_bounds.min_x(),
                clamp_body_y(inst_body_start),
                safe_bounds.width(),
                0.0,
            )
        });

        let marketplace_header_rect =
            Rect::from_xywh(safe_bounds.min_x(), y, safe_bounds.width(), section_h);
        y += section_h;

        let mkt_body_start = y;
        let mut marketplace_rows = Vec::new();
        if !self.marketplace_collapsed {
            if marketplace_items.is_empty() {
                y += 24.0;
            } else {
                for item in &marketplace_items {
                    marketplace_rows.push((
                        item.id.clone(),
                        Rect::from_xywh(safe_bounds.min_x(), y, safe_bounds.width(), row_h),
                    ));
                    y += row_h;
                }
            }
        }
        let mkt_raw_rect = Rect::from_xywh(
            safe_bounds.min_x(),
            mkt_body_start,
            safe_bounds.width(),
            (y - mkt_body_start).max(0.0),
        );
        let marketplace_list_rect = mkt_raw_rect.intersect(&body_rect).unwrap_or_else(|| {
            Rect::from_xywh(
                safe_bounds.min_x(),
                clamp_body_y(mkt_body_start),
                safe_bounds.width(),
                0.0,
            )
        });

        ExtensionsSidebarLayout {
            search_rect,
            body_rect,
            installed_header_rect,
            installed_list_rect,
            installed_rows,
            marketplace_header_rect,
            marketplace_list_rect,
            marketplace_rows,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionsSidebarLayout {
    pub search_rect: Rect,
    pub body_rect: Rect,
    pub installed_header_rect: Rect,
    pub installed_list_rect: Rect,
    pub installed_rows: Vec<(String, Rect)>,
    pub marketplace_header_rect: Rect,
    pub marketplace_list_rect: Rect,
    pub marketplace_rows: Vec<(String, Rect)>,
}

fn block_on_future<F>(fut: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build current_thread tokio runtime");
                rt.block_on(fut)
            })
            .join()
            .expect("extensions_view async helper thread panicked")
        })
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build current_thread tokio runtime");
        rt.block_on(fut)
    }
}

fn key_event_to_char(key: &KeyEvent) -> Option<char> {
    if key.modifiers.ctrl || key.modifiers.alt || key.modifiers.meta {
        return None;
    }
    if let Some(text) = &key.text {
        let mut chars = text.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if !c.is_control() {
                return Some(c);
            }
        }
    }
    match key.key {
        KeyCode::Space => Some(' '),
        KeyCode::Char(c) => {
            if key.modifiers.shift {
                c.to_uppercase().next()
            } else {
                Some(c)
            }
        }
        _ => None,
    }
}

fn truncate_text_to_width(text: &str, max_width: f32, font: &FontMetrics, scale: f32) -> String {
    let eff_char_w = font.char_width * scale.max(0.1);
    if max_width <= 0.0 || eff_char_w <= 0.0 {
        return String::new();
    }
    let max_chars = (max_width / eff_char_w).floor() as usize;
    if max_chars == 0 {
        return String::new();
    }
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }
    let mut out: String = text.chars().take(max_chars - 3).collect();
    out.push_str("...");
    out
}

impl View for ExtensionsSidebarView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !crate::is_finite_positive_rect(bounds)
            || !crate::is_finite_font(font)
            || !(bounds.min_x() + 12.0).is_finite()
            || !(bounds.min_y() + 60.0).is_finite()
        {
            return;
        }
        *self.last_font.write() = font.clone();
        let mut targets = Vec::new();

        // 1. Sidebar background + right border
        scene.push_quad(
            Quad::new(bounds)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.sidebar_border, Edges::new(0.0, 1.0, 0.0, 0.0)),
        );

        let layout = self.compute_layout(bounds);
        let mut cur_y = bounds.min_y();

        // 2. Header (28.0 px)
        let header_h = 28.0;
        scene.push_text(GlyphRun::new(
            "EXTENSIONS".to_string(),
            Point::new(bounds.min_x() + 12.0, cur_y + 6.0),
            Rect::from_xywh(
                bounds.min_x() + 12.0,
                cur_y + 6.0,
                (bounds.width() - 24.0).max(0.0),
                font.line_height,
            ),
            theme.sidebar_header_fg,
            font.font_size * 0.85,
        ));
        cur_y += header_h;

        // 3. Search input box (26.0 px + 6.0 bottom padding)
        let input_h = 26.0;
        let input_rect = layout.search_rect;
        let input_border = if self.search_focused {
            theme.activity_bar_active_border
        } else {
            theme.command_palette_border
        };
        scene.push_quad(
            Quad::new(input_rect)
                .with_background(theme.editor_background)
                .with_border(input_border, 1.0)
                .with_corner_radius(2.0),
        );
        Self::push_clipped_target(
            &mut targets,
            input_rect,
            bounds,
            ExtensionsClickTarget::SearchInput,
        );

        let clear_w = if !self.search_query.is_empty() { 20.0 } else { 0.0 };
        let text_x = input_rect.min_x() + 6.0;
        let text_y = input_rect.min_y() + ((input_h - font.line_height) * 0.5).max(2.0);
        let text_max_w = (input_rect.width() - 12.0 - clear_w).max(0.0);

        if self.search_query.is_empty() {
            let placeholder = truncate_text_to_width(
                "Search Extensions in Marketplace",
                text_max_w,
                font,
                0.88,
            );
            scene.push_text(GlyphRun::new(
                placeholder,
                Point::new(text_x, text_y),
                Rect::from_xywh(text_x, text_y, text_max_w, font.line_height),
                theme.tab_inactive_fg,
                font.font_size * 0.88,
            ));
        } else {
            let display_q = truncate_text_to_width(&self.search_query, text_max_w, font, 0.9);
            scene.push_text(GlyphRun::new(
                display_q,
                Point::new(text_x, text_y),
                Rect::from_xywh(text_x, text_y, text_max_w, font.line_height),
                theme.sidebar_fg,
                font.font_size * 0.9,
            ));
        }

        if self.search_focused {
            let q_w = font.text_width(&self.search_query).min(text_max_w);
            scene.push_quad(
                Quad::new(Rect::from_xywh(
                    text_x + q_w,
                    input_rect.min_y() + 4.0,
                    1.5,
                    (input_h - 8.0).max(8.0),
                ))
                .with_background(theme.editor_cursor),
            );
        }

        if !self.search_query.is_empty() {
            let clear_rect = Rect::from_xywh(
                input_rect.max_x() - 20.0,
                input_rect.min_y() + 3.0,
                18.0,
                input_h - 6.0,
            );
            Self::push_clipped_target(
                &mut targets,
                clear_rect,
                bounds,
                ExtensionsClickTarget::ClearSearch,
            );
            scene.push_text(GlyphRun::new(
                "x".to_string(),
                Point::new(clear_rect.min_x() + 5.0, text_y),
                clear_rect,
                theme.tab_inactive_fg,
                font.font_size * 0.85,
            ));
        }

        let _ = cur_y + input_h + 6.0;

        // 4. Status footer (optional 20.0 px)
        let footer_h = if self.status_message.is_some() { 20.0 } else { 0.0 };
        let body_rect = layout.body_rect;

        let installed_items = self.filtered_installed();
        let marketplace_items = self.filtered_marketplace();

        // 5. Scrollable body clipped to `body_rect`
        scene.push_clip(body_rect);

        // --- INSTALLED SECTION ---
        let inst_arrow = if self.installed_collapsed { "▶" } else { "▼" };
        let inst_header_text = format!("{} INSTALLED ({})", inst_arrow, installed_items.len());
        let inst_header_rect = layout.installed_header_rect;
        let y_inst_header = inst_header_rect.min_y();
        if inst_header_rect.max_y() > body_rect.min_y()
            && inst_header_rect.min_y() < body_rect.max_y()
        {
            scene.push_quad(
                Quad::new(inst_header_rect)
                    .with_background(theme.panel_bg)
                    .with_border_edges(theme.panel_border, Edges::new(1.0, 0.0, 1.0, 0.0)),
            );
            scene.push_text(
                GlyphRun::new(
                    inst_header_text,
                    Point::new(bounds.min_x() + 8.0, y_inst_header + 4.0),
                    inst_header_rect,
                    theme.sidebar_header_fg,
                    font.font_size * 0.82,
                )
                .with_style(true, false, false),
            );
            Self::push_clipped_target(
                &mut targets,
                inst_header_rect,
                body_rect,
                ExtensionsClickTarget::ToggleInstalledSection,
            );
        }

        if !self.installed_collapsed {
            if installed_items.is_empty() {
                let empty_y = inst_header_rect.max_y();
                let empty_rect = Rect::from_xywh(bounds.min_x(), empty_y, bounds.width(), 24.0);
                if empty_rect.max_y() > body_rect.min_y()
                    && empty_rect.min_y() < body_rect.max_y()
                {
                    scene.push_text(GlyphRun::new(
                        "No installed extensions".to_string(),
                        Point::new(bounds.min_x() + 12.0, empty_y + 4.0),
                        empty_rect,
                        theme.tab_inactive_fg,
                        font.font_size * 0.85,
                    ));
                }
            } else {
                for (item, (_, row_rect_ref)) in
                    installed_items.iter().zip(layout.installed_rows.iter())
                {
                    let row_rect = *row_rect_ref;
                    let y = row_rect.min_y();
                    if row_rect.max_y() > body_rect.min_y()
                        && row_rect.min_y() < body_rect.max_y()
                    {
                        let is_selected = self
                            .selected_id
                            .as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case(&item.id));
                        if is_selected {
                            scene.push_quad(
                                Quad::new(row_rect)
                                    .with_background(theme.list_active_selection_bg),
                            );
                        }
                        // Push row target BEFORE buttons so reverse scan gives buttons priority
                        Self::push_clipped_target(
                            &mut targets,
                            row_rect,
                            body_rect,
                            ExtensionsClickTarget::SelectRow(item.id.clone()),
                        );

                        // Line 1: Display Name + Version + Disabled tag
                        let title_avail = (bounds.width() - 90.0).max(40.0);
                        let title_text =
                            truncate_text_to_width(&item.display_name, title_avail, font, 0.9);
                        scene.push_text(
                            GlyphRun::new(
                                title_text,
                                Point::new(bounds.min_x() + 10.0, y + 4.0),
                                Rect::from_xywh(bounds.min_x() + 10.0, y + 4.0, title_avail, font.line_height),
                                if item.enabled {
                                    theme.sidebar_fg
                                } else {
                                    theme.tab_inactive_fg
                                },
                                font.font_size * 0.9,
                            )
                            .with_style(true, false, false),
                        );

                        let ver_label = if item.enabled {
                            format!("v{}", item.version)
                        } else {
                            format!("v{} (Disabled)", item.version)
                        };
                        let ver_color = if item.enabled {
                            theme.tab_inactive_fg
                        } else {
                            theme.editor_warning_foreground
                        };
                        let ver_w = font.text_width(&ver_label);
                        scene.push_text(GlyphRun::new(
                            ver_label,
                            Point::new((bounds.max_x() - ver_w - 8.0).max(bounds.min_x() + 10.0), y + 4.0),
                            row_rect,
                            ver_color,
                            font.font_size * 0.78,
                        ));

                        // Line 2: publisher - description
                        let sub_raw = if item.description.is_empty() {
                            item.publisher.clone()
                        } else {
                            format!("{} - {}", item.publisher, item.description)
                        };
                        let sub_text = truncate_text_to_width(
                            &sub_raw,
                            (bounds.width() - 20.0).max(20.0),
                            font,
                            0.8,
                        );
                        scene.push_text(GlyphRun::new(
                            sub_text,
                            Point::new(bounds.min_x() + 10.0, y + 19.0),
                            row_rect,
                            theme.tab_inactive_fg,
                            font.font_size * 0.8,
                        ));

                        // Line 3: [Disable]/[Enable] and [Uninstall] buttons
                        let btn_y = y + 34.0;
                        let btn_h = 16.0;
                        let toggle_label = if item.enabled { "Disable" } else { "Enable" };
                        let toggle_w = (font.text_width(toggle_label) + 12.0).max(54.0);
                        let toggle_rect = Rect::from_xywh(bounds.min_x() + 10.0, btn_y, toggle_w, btn_h);
                        scene.push_quad(
                            Quad::new(toggle_rect)
                                .with_background(theme.panel_bg)
                                .with_border(theme.command_palette_border, 1.0)
                                .with_corner_radius(2.0),
                        );
                        scene.push_text(GlyphRun::new(
                            toggle_label.to_string(),
                            Point::new(toggle_rect.min_x() + 6.0, btn_y + 1.0),
                            toggle_rect,
                            theme.sidebar_fg,
                            font.font_size * 0.78,
                        ));
                        let toggle_target = if item.enabled {
                            ExtensionsClickTarget::DisableBtn(item.id.clone())
                        } else {
                            ExtensionsClickTarget::EnableBtn(item.id.clone())
                        };
                        Self::push_clipped_target(&mut targets, toggle_rect, body_rect, toggle_target);

                        let uninst_label = "Uninstall";
                        let uninst_w = (font.text_width(uninst_label) + 12.0).max(62.0);
                        let uninst_rect = Rect::from_xywh(
                            toggle_rect.max_x() + 6.0,
                            btn_y,
                            uninst_w,
                            btn_h,
                        );
                        scene.push_quad(
                            Quad::new(uninst_rect)
                                .with_background(theme.panel_bg)
                                .with_border(theme.editor_error_foreground, 1.0)
                                .with_corner_radius(2.0),
                        );
                        scene.push_text(GlyphRun::new(
                            uninst_label.to_string(),
                            Point::new(uninst_rect.min_x() + 6.0, btn_y + 1.0),
                            uninst_rect,
                            theme.editor_error_foreground,
                            font.font_size * 0.78,
                        ));
                        Self::push_clipped_target(
                            &mut targets,
                            uninst_rect,
                            body_rect,
                            ExtensionsClickTarget::UninstallBtn(item.id.clone()),
                        );
                    }
                }
            }
        }

        // --- MARKETPLACE SECTION ---
        let mkt_arrow = if self.marketplace_collapsed { "▶" } else { "▼" };
        let mkt_header_text = format!("{} MARKETPLACE ({})", mkt_arrow, marketplace_items.len());
        let mkt_header_rect = layout.marketplace_header_rect;
        let y_mkt_header = mkt_header_rect.min_y();
        if mkt_header_rect.max_y() > body_rect.min_y()
            && mkt_header_rect.min_y() < body_rect.max_y()
        {
            scene.push_quad(
                Quad::new(mkt_header_rect)
                    .with_background(theme.panel_bg)
                    .with_border_edges(theme.panel_border, Edges::new(1.0, 0.0, 1.0, 0.0)),
            );
            scene.push_text(
                GlyphRun::new(
                    mkt_header_text,
                    Point::new(bounds.min_x() + 8.0, y_mkt_header + 4.0),
                    mkt_header_rect,
                    theme.sidebar_header_fg,
                    font.font_size * 0.82,
                )
                .with_style(true, false, false),
            );
            Self::push_clipped_target(
                &mut targets,
                mkt_header_rect,
                body_rect,
                ExtensionsClickTarget::ToggleMarketplaceSection,
            );
        }

        if !self.marketplace_collapsed {
            if marketplace_items.is_empty() {
                let empty_y = mkt_header_rect.max_y();
                let empty_rect = Rect::from_xywh(bounds.min_x(), empty_y, bounds.width(), 24.0);
                if empty_rect.max_y() > body_rect.min_y()
                    && empty_rect.min_y() < body_rect.max_y()
                {
                    scene.push_text(GlyphRun::new(
                        "No matching marketplace extensions".to_string(),
                        Point::new(bounds.min_x() + 12.0, empty_y + 4.0),
                        empty_rect,
                        theme.tab_inactive_fg,
                        font.font_size * 0.85,
                    ));
                }
            } else {
                for (item, (_, row_rect_ref)) in
                    marketplace_items.iter().zip(layout.marketplace_rows.iter())
                {
                    let row_rect = *row_rect_ref;
                    let y = row_rect.min_y();
                    if row_rect.max_y() > body_rect.min_y()
                        && row_rect.min_y() < body_rect.max_y()
                    {
                        let is_selected = self
                            .selected_id
                            .as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case(&item.id));
                        if is_selected {
                            scene.push_quad(
                                Quad::new(row_rect)
                                    .with_background(theme.list_active_selection_bg),
                            );
                        }
                        Self::push_clipped_target(
                            &mut targets,
                            row_rect,
                            body_rect,
                            ExtensionsClickTarget::SelectRow(item.id.clone()),
                        );

                        // Line 1: Display Name + version
                        let title_avail = (bounds.width() - 74.0).max(40.0);
                        let title_text =
                            truncate_text_to_width(&item.display_name, title_avail, font, 0.9);
                        scene.push_text(
                            GlyphRun::new(
                                title_text,
                                Point::new(bounds.min_x() + 10.0, y + 4.0),
                                Rect::from_xywh(bounds.min_x() + 10.0, y + 4.0, title_avail, font.line_height),
                                theme.sidebar_fg,
                                font.font_size * 0.9,
                            )
                            .with_style(true, false, false),
                        );

                        let ver_label = format!("v{}", item.version);
                        let ver_w = font.text_width(&ver_label);
                        scene.push_text(GlyphRun::new(
                            ver_label,
                            Point::new((bounds.max_x() - ver_w - 8.0).max(bounds.min_x() + 10.0), y + 4.0),
                            row_rect,
                            theme.tab_inactive_fg,
                            font.font_size * 0.78,
                        ));

                        // Line 2: namespace - description
                        let sub_raw = if item.description.is_empty() {
                            item.namespace.clone()
                        } else {
                            format!("{} - {}", item.namespace, item.description)
                        };
                        let sub_text = truncate_text_to_width(
                            &sub_raw,
                            (bounds.width() - 20.0).max(20.0),
                            font,
                            0.8,
                        );
                        scene.push_text(GlyphRun::new(
                            sub_text,
                            Point::new(bounds.min_x() + 10.0, y + 19.0),
                            row_rect,
                            theme.tab_inactive_fg,
                            font.font_size * 0.8,
                        ));

                        // Line 3: "Installed" badge or [Install] button
                        let btn_y = y + 34.0;
                        let btn_h = 16.0;
                        if item.is_installed {
                            scene.push_text(GlyphRun::new(
                                "Installed".to_string(),
                                Point::new(bounds.min_x() + 10.0, btn_y + 1.0),
                                Rect::from_xywh(bounds.min_x() + 10.0, btn_y, 80.0, btn_h),
                                theme.editor_info_foreground,
                                font.font_size * 0.78,
                            ));
                        } else {
                            let inst_label = "Install";
                            let inst_w = (font.text_width(inst_label) + 14.0).max(52.0);
                            let inst_rect =
                                Rect::from_xywh(bounds.min_x() + 10.0, btn_y, inst_w, btn_h);
                            scene.push_quad(
                                Quad::new(inst_rect)
                                    .with_background(theme.activity_bar_badge_bg)
                                    .with_corner_radius(2.0),
                            );
                            scene.push_text(GlyphRun::new(
                                inst_label.to_string(),
                                Point::new(inst_rect.min_x() + 7.0, btn_y + 1.0),
                                inst_rect,
                                theme.activity_bar_badge_fg,
                                font.font_size * 0.78,
                            ));
                            Self::push_clipped_target(
                                &mut targets,
                                inst_rect,
                                body_rect,
                                ExtensionsClickTarget::InstallBtn {
                                    namespace: item.namespace.clone(),
                                    name: item.name.clone(),
                                    version: item.version.clone(),
                                },
                            );
                        }
                    }
                }
            }
        }

        scene.pop_clip();

        // 6. Optional Status Footer
        if let Some(status) = &self.status_message {
            let footer_rect = Rect::from_xywh(
                bounds.min_x(),
                bounds.max_y() - footer_h,
                bounds.width(),
                footer_h,
            );
            scene.push_quad(
                Quad::new(footer_rect)
                    .with_background(theme.panel_bg)
                    .with_border_edges(theme.panel_border, Edges::new(1.0, 0.0, 0.0, 0.0)),
            );
            let msg =
                truncate_text_to_width(status, (bounds.width() - 16.0).max(20.0), font, 0.78);
            scene.push_text(GlyphRun::new(
                msg,
                Point::new(bounds.min_x() + 8.0, footer_rect.min_y() + 3.0),
                footer_rect,
                theme.tab_inactive_fg,
                font.font_size * 0.78,
            ));
        }

        *self.click_targets.write() = targets;
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::MouseDown(mouse) => {
                if !crate::is_finite_positive_rect(bounds)
                    || !mouse.position.x.is_finite()
                    || !mouse.position.y.is_finite()
                    || mouse.button != MouseButton::Left
                    || !bounds.contains(mouse.position)
                {
                    return EventResult::Ignored;
                }

                let matched = self
                    .click_targets
                    .read()
                    .iter()
                    .rev()
                    .find(|(rect, _)| rect.contains(mouse.position))
                    .map(|(_, target)| target.clone());

                match matched {
                    Some(ExtensionsClickTarget::SearchInput) => {
                        self.search_focused = true;
                    }
                    Some(ExtensionsClickTarget::ClearSearch) => {
                        self.search_focused = true;
                        self.apply_search_query("");
                    }
                    Some(ExtensionsClickTarget::ToggleInstalledSection) => {
                        self.installed_collapsed = !self.installed_collapsed;
                        self.search_focused = false;
                    }
                    Some(ExtensionsClickTarget::ToggleMarketplaceSection) => {
                        self.marketplace_collapsed = !self.marketplace_collapsed;
                        self.search_focused = false;
                    }
                    Some(ExtensionsClickTarget::SelectRow(id)) => {
                        self.selected_id = Some(id.clone());
                        self.search_focused = false;
                        self.last_action = Some(ExtensionsViewAction::SelectExtension(id));
                    }
                    Some(ExtensionsClickTarget::EnableBtn(id)) => {
                        self.search_focused = false;
                        if let Err(err) = self.set_extension_enabled(&id, true) {
                            self.status_message = Some(format!("Enable failed: {}", err));
                        }
                    }
                    Some(ExtensionsClickTarget::DisableBtn(id)) => {
                        self.search_focused = false;
                        if let Err(err) = self.set_extension_enabled(&id, false) {
                            self.status_message = Some(format!("Disable failed: {}", err));
                        }
                    }
                    Some(ExtensionsClickTarget::UninstallBtn(id)) => {
                        self.search_focused = false;
                        if let Err(err) = self.uninstall_extension(&id) {
                            self.status_message = Some(format!("Uninstall failed: {}", err));
                        }
                    }
                    Some(ExtensionsClickTarget::InstallBtn {
                        namespace,
                        name,
                        version,
                    }) => {
                        self.selected_id = Some(ExtensionInstaller::extension_id(&namespace, &name));
                        self.search_focused = false;
                        let ver_opt = if version.is_empty() {
                            None
                        } else {
                            Some(version.as_str())
                        };
                        if self.installer.is_some() {
                            if let Err(err) = self.execute_install_sync(&namespace, &name, ver_opt) {
                                self.status_message = Some(format!("Install failed: {}", err));
                                self.last_action = None;
                            }
                        } else {
                            self.status_message =
                                Some(format!("Installing {}.{}...", namespace, name));
                            self.last_action = Some(ExtensionsViewAction::Install {
                                namespace,
                                name,
                                version: ver_opt.map(|v| v.to_string()),
                            });
                        }
                    }
                    None => {
                        self.search_focused = false;
                    }
                }
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => {
                if !self.search_focused {
                    return EventResult::Ignored;
                }
                match key.key {
                    KeyCode::Enter => {
                        let _ = self.execute_search_sync();
                        EventResult::Consumed
                    }
                    KeyCode::Escape => {
                        if !self.search_query.is_empty() {
                            self.apply_search_query("");
                        } else {
                            self.search_focused = false;
                        }
                        EventResult::Consumed
                    }
                    KeyCode::Backspace => {
                        let mut next = self.search_query.clone();
                        next.pop();
                        self.apply_search_query(&next);
                        EventResult::Consumed
                    }
                    _ => {
                        if let Some(ch) = key_event_to_char(key) {
                            let mut next = self.search_query.clone();
                            next.push(ch);
                            self.apply_search_query(&next);
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                }
            }
            InputEvent::ScrollWheel(scroll) => {
                if !crate::is_finite_positive_rect(bounds)
                    || !scroll.position.x.is_finite()
                    || !scroll.position.y.is_finite()
                    || !bounds.contains(scroll.position)
                {
                    return EventResult::Ignored;
                }
                let footer_h = if self.status_message.is_some() { 20.0 } else { 0.0 };
                let viewport_h = (bounds.height() - 28.0 - 32.0 - footer_h).max(0.0);
                let content_h = self.compute_total_content_height();
                let max_scroll = (content_h - viewport_h).max(0.0);
                let max_scroll = if max_scroll.is_finite() { max_scroll } else { 0.0 };
                let safe_scroll = if self.scroll_top.is_finite() {
                    self.scroll_top
                } else {
                    0.0
                };
                if !scroll.delta_y.is_finite() {
                    self.scroll_top = safe_scroll.clamp(0.0, max_scroll);
                    return EventResult::Ignored;
                }
                let raw = safe_scroll - scroll.delta_y;
                self.scroll_top = if raw.is_nan() {
                    0.0
                } else {
                    raw.clamp(0.0, max_scroll)
                };
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_ext_host::marketplace::MockHttpTransport;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{MouseEvent, ScrollEvent};
    use serde_json::json;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), now))
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320u32 & mask);
            }
        }
        !crc
    }

    fn create_test_vsix(manifest_json: &str) -> Vec<u8> {
        let name = b"extension/package.json";
        let data = manifest_json.as_bytes();
        let crc = crc32(data);
        let len = data.len() as u32;
        let name_len = name.len() as u16;

        let mut out = Vec::new();
        // Local file header
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // compression: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name);
        out.extend_from_slice(data);

        let cd_offset = out.len() as u32;
        // Central directory header
        out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version made by
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // compression: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        out.extend_from_slice(&0u16.to_le_bytes()); // disk start
        out.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        out.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        out.extend_from_slice(&0u32.to_le_bytes()); // local header offset
        out.extend_from_slice(name);

        let cd_size = (out.len() as u32) - cd_offset;
        // End of central directory
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn center(rect: Rect) -> Point {
        Point::new(rect.min_x() + rect.width() * 0.5, rect.min_y() + rect.height() * 0.5)
    }

    #[test]
    fn test_extensions_view_default_render_and_search_filtering() {
        let mut view = ExtensionsSidebarView::new();
        assert!(view.installer.is_none());
        assert!(!view.marketplace_results.is_empty());

        let bounds = Rect::from_xywh(0.0, 0.0, 280.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("EXTENSIONS"));
        assert!(scene.contains_text("Search Extensions in Marketplace"));
        assert!(scene.contains_text("INSTALLED (0)"));
        assert!(scene.contains_text("MARKETPLACE ("));
        assert!(scene.contains_text("rust-analyzer"));

        // Click search input and type "python"
        let search_rect = view
            .click_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                ExtensionsClickTarget::SearchInput => Some(*r),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            view.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(center(search_rect), MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        assert!(view.search_focused);

        for ch in "python".chars() {
            view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        assert_eq!(view.search_query, "python");
        assert_eq!(view.filtered_marketplace().len(), 1);
        assert_eq!(view.filtered_marketplace()[0].name, "python");

        // Escape clears query, second Escape unfocuses
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bounds,
        );
        assert_eq!(view.search_query, "");
        assert!(view.search_focused);

        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bounds,
        );
        assert!(!view.search_focused);
    }

    #[tokio::test]
    async fn test_extensions_view_install_enable_disable_uninstall_with_mock_transport() {
        let temp_dir = unique_temp_dir("rustcode_ext_view_lifecycle");
        let transport = Arc::new(MockHttpTransport::new());

        let vsix_bytes = create_test_vsix(
            r#"{
                "name": "mock-tool",
                "publisher": "mockpub",
                "version": "1.2.3",
                "displayName": "Mock Tool",
                "description": "A mocked extension"
            }"#,
        );
        let dl_url = "https://open-vsx.org/api/mockpub/mock-tool/1.2.3/file/mock-tool.vsix";
        transport.set_response(
            "https://open-vsx.org/api/-/search?query=mock&size=20&offset=0",
            serde_json::to_vec(&json!({
                "totalSize": 1,
                "extensions": [{
                    "namespace": "mockpub",
                    "name": "mock-tool",
                    "version": "1.2.3",
                    "displayName": "Mock Tool",
                    "description": "A mocked extension",
                    "downloadUrl": dl_url
                }]
            }))
            .unwrap(),
        );
        transport.set_response(
            "https://open-vsx.org/api/mockpub/mock-tool/1.2.3",
            serde_json::to_vec(&json!({
                "namespace": "mockpub",
                "name": "mock-tool",
                "version": "1.2.3",
                "displayName": "Mock Tool",
                "downloadUrl": dl_url
            }))
            .unwrap(),
        );
        transport.set_response(dl_url, vsix_bytes);

        let client = Arc::new(MarketplaceClient::new(
            "https://open-vsx.org/api",
            transport,
        ));
        let installer = Arc::new(ExtensionInstaller::new(client.clone(), &temp_dir));

        let mut view = ExtensionsSidebarView::new().with_installer(installer.clone());
        let bounds = Rect::from_xywh(0.0, 0.0, 280.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        // 1. Focus search input, type "mock", press Enter inside #[tokio::test] (verifies panic-free sync bridge)
        view.render(bounds, &theme, &font, &mut Scene::new());
        let search_rect = view
            .click_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                ExtensionsClickTarget::SearchInput => Some(*r),
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(center(search_rect), MouseButton::Left)),
            bounds,
        );
        for ch in "mock".chars() {
            view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            view.last_action,
            Some(ExtensionsViewAction::Search("mock".to_string()))
        );

        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("Mock Tool"));
        assert!(scene.contains_text("Install"));

        // 2. Click [Install]
        let install_rect = view
            .click_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                ExtensionsClickTarget::InstallBtn { name, .. } if name == "mock-tool" => Some(*r),
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(center(install_rect), MouseButton::Left)),
            bounds,
        );
        assert_eq!(view.installed.len(), 1);
        assert_eq!(view.installed[0].id, "mockpub.mock-tool");
        assert!(view.installed[0].enabled);
        assert!(matches!(
            view.last_action,
            Some(ExtensionsViewAction::Install { .. })
        ));

        let mut scene_inst = Scene::new();
        view.render(bounds, &theme, &font, &mut scene_inst);
        assert!(scene_inst.contains_text("INSTALLED (1)"));
        assert!(scene_inst.contains_text("Installed"));
        assert!(scene_inst.contains_text("Disable"));

        // 3. Click [Disable] -> persists .disabled.json
        let disable_rect = view
            .click_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                ExtensionsClickTarget::DisableBtn(id) if id == "mockpub.mock-tool" => Some(*r),
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(center(disable_rect), MouseButton::Left)),
            bounds,
        );
        assert!(!view.installed[0].enabled);
        assert!(!installer.is_enabled("mockpub.mock-tool"));
        assert_eq!(
            view.last_action,
            Some(ExtensionsViewAction::Disable {
                id: "mockpub.mock-tool".to_string()
            })
        );

        // 4. Render & click [Enable]
        let mut scene_dis = Scene::new();
        view.render(bounds, &theme, &font, &mut scene_dis);
        assert!(scene_dis.contains_text("Enable"));
        let enable_rect = view
            .click_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                ExtensionsClickTarget::EnableBtn(id) if id == "mockpub.mock-tool" => Some(*r),
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(center(enable_rect), MouseButton::Left)),
            bounds,
        );
        assert!(view.installed[0].enabled);
        assert!(installer.is_enabled("mockpub.mock-tool"));
        assert_eq!(
            view.last_action,
            Some(ExtensionsViewAction::Enable {
                id: "mockpub.mock-tool".to_string()
            })
        );

        // 5. Render & click [Uninstall]
        view.render(bounds, &theme, &font, &mut Scene::new());
        let uninstall_rect = view
            .click_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                ExtensionsClickTarget::UninstallBtn(id) if id == "mockpub.mock-tool" => Some(*r),
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(center(uninstall_rect), MouseButton::Left)),
            bounds,
        );
        assert!(view.installed.is_empty());
        assert!(installer.list_installed().is_empty());
        assert!(!view.marketplace_results[0].is_installed);
        assert_eq!(
            view.last_action,
            Some(ExtensionsViewAction::Uninstall {
                id: "mockpub.mock-tool".to_string()
            })
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extensions_view_collapse_and_scroll_clamping() {
        let mut view = ExtensionsSidebarView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 280.0, 220.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        view.render(bounds, &theme, &font, &mut Scene::new());
        assert!(view.scroll_top == 0.0);

        // Scroll down
        let res = view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(100.0, 120.0), 0.0, -120.0)),
            bounds,
        );
        assert_eq!(res, EventResult::Consumed);
        assert!(view.scroll_top > 0.0);

        // Scroll past top clamps to 0.0
        view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(100.0, 120.0), 0.0, 5000.0)),
            bounds,
        );
        assert_eq!(view.scroll_top, 0.0);

        // Collapse Marketplace section
        view.render(bounds, &theme, &font, &mut Scene::new());
        let mkt_toggle = view
            .click_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                ExtensionsClickTarget::ToggleMarketplaceSection => Some(*r),
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(center(mkt_toggle), MouseButton::Left)),
            bounds,
        );
        assert!(view.marketplace_collapsed);

        // ScrollWheel immediately after collapsing (before re-render) uses current content height
        view.scroll_top = 80.0;
        view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(100.0, 120.0), 0.0, -20.0)),
            bounds,
        );
        assert_eq!(view.scroll_top, 0.0);
    }

    #[tokio::test]
    async fn test_failed_install_does_not_emit_install_action() {
        let temp_dir = unique_temp_dir("rustcode_ext_failed_install");
        let transport = Arc::new(MockHttpTransport::new());
        let client = Arc::new(MarketplaceClient::new(
            "https://open-vsx.org/api",
            transport,
        ));
        let installer = Arc::new(ExtensionInstaller::new(client, &temp_dir));

        let mut view = ExtensionsSidebarView::new().with_installer(installer);
        let bounds = Rect::from_xywh(0.0, 0.0, 280.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        view.render(bounds, &theme, &font, &mut Scene::new());
        let install_rect = view
            .click_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                ExtensionsClickTarget::InstallBtn { .. } => Some(*r),
                _ => None,
            })
            .unwrap();

        view.last_action = Some(ExtensionsViewAction::Search("prior".to_string()));
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(center(install_rect), MouseButton::Left)),
            bounds,
        );

        assert!(view
            .status_message
            .as_deref()
            .is_some_and(|s| s.starts_with("Install failed:")));
        assert_eq!(view.last_action, None);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extensions_view_non_finite_bounds_and_nan_scrollwheel_no_panic_no_poison() {
        use rustcode_platform::render::{SceneNode, Size};

        let mut view = ExtensionsSidebarView::new();
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let bad_bounds = [
            Rect::new(Point::new(f32::NAN, f32::NAN), Size::new(1024.0, 768.0)),
            Rect::new(Point::zero(), Size::new(280.0, f32::NAN)),
            Rect::from_xywh(f32::INFINITY, 0.0, 280.0, 600.0),
            Rect::from_xywh(0.0, 0.0, f32::INFINITY, 600.0),
        ];

        for &b in &bad_bounds {
            let layout = view.compute_layout(b);
            assert!(crate::is_finite_rect(layout.search_rect));
            assert!(crate::is_finite_rect(layout.body_rect));
            assert!(crate::is_finite_rect(layout.installed_list_rect));
            assert!(crate::is_finite_rect(layout.marketplace_list_rect));

            let mut scene = Scene::new();
            view.render(b, &theme, &font, &mut scene);
            assert!(scene.nodes().is_empty());
        }

        // ScrollWheel with delta_y = f32::NAN must not poison scroll_top
        let valid_bounds = Rect::from_xywh(0.0, 0.0, 280.0, 200.0);
        view.scroll_top = 10.0;
        view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(100.0, 120.0), 0.0, f32::NAN)),
            valid_bounds,
        );
        assert!(view.scroll_top.is_finite());

        let mut scene = Scene::new();
        view.render(valid_bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("MARKETPLACE"));
        for node in scene.nodes() {
            if let SceneNode::Quad(q) = node {
                assert!(crate::is_finite_rect(q.bounds));
            }
        }
    }
}
