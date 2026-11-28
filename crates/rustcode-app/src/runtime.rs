//! Application runtime loop, disk configuration/theme/extension discovery,
//! SQLite/in-memory state persistence, `FileSystemWatcher`, `AutoSaveService`,
//! and live `NodeExtHostProcess` lifecycle management.

use crate::cli::{CliOptions, GotoTarget};
use crate::RustCodeContext;
use parking_lot::{Mutex, RwLock};
use rustcode_base::disposable::Disposable;
use rustcode_editor::{AutoSaveConfig, AutoSaveMode, AutoSaveService, Position};
use rustcode_ext_host::manifest::ExtensionLoader;
use rustcode_ext_host::marketplace::WorkspaceExtensionsRecommendations;
use rustcode_ext_host::rpc::{NodeExtHostConfig, NodeExtHostProcess, RpcWireFormat};
use rustcode_ext_host::runtime_health::{ExtHostWatchdog, WatchdogHealth};
use rustcode_ext_host::window::NotificationSeverity;
use rustcode_platform::fs::{
    file_uri_to_path, is_file_uri, path_to_file_uri, strip_windows_verbatim_prefix,
    FileChangeEvent, FileChangeType, FileSystemWatcher,
};
use rustcode_platform::os_ux::{
    AppPaths, ArboardClipboardBackend, ClipboardBackend, FileDialogBackend,
    InMemoryClipboardBackend, MockFileDialogBackend,
};
#[cfg(feature = "desktop")]
use rustcode_platform::os_ux::RfdFileDialogBackend;
#[cfg(feature = "desktop")]
use rustcode_platform::storage::SqliteStorageService;
use rustcode_platform::storage::{
    InMemoryStorageBackend, MementoScope, PersistedTabState, PersistedWorkspaceState,
    StorageBackend, StorageError,
};
use rustcode_platform::theme::ThemeService;
use rustcode_workbench::workbench_view::FocusedRegion;
use rustcode_workbench::WorkbenchWindow;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::time::{Duration, Instant};

const DEFAULT_EMPTY_WORKSPACE_KEY: &str = "__default_empty_workspace__";

/// Configuration for `RuntimeLoop`.
#[derive(Debug, Clone)]
pub struct RuntimeLoopConfig {
    pub disable_extensions: bool,
    pub spawn_ext_host_on_startup: bool,
    pub node_binary: Option<PathBuf>,
    pub ext_host_entry: Option<PathBuf>,
    pub ext_host_wire_format: RpcWireFormat,
    pub max_ext_host_restarts: usize,
    pub ext_host_restart_backoff: Duration,
    pub ext_host_healthy_reset_after: Duration,
    pub tick_interval: Duration,
}

impl Default for RuntimeLoopConfig {
    fn default() -> Self {
        Self {
            disable_extensions: false,
            spawn_ext_host_on_startup: false,
            node_binary: None,
            ext_host_entry: None,
            ext_host_wire_format: RpcWireFormat::ContentLengthJson,
            max_ext_host_restarts: 3,
            ext_host_restart_backoff: Duration::from_millis(150),
            ext_host_healthy_reset_after: Duration::from_secs(60),
            tick_interval: Duration::from_millis(50),
        }
    }
}

/// Bootstrapped application container ready for headless or desktop execution.
pub struct BootstrappedApp {
    pub context: RustCodeContext,
    pub window: WorkbenchWindow,
    pub app_paths: AppPaths,
    pub runtime_loop: RuntimeLoop,
}

enum RuntimeHandle {
    Owned(tokio::runtime::Runtime),
    Borrowed(tokio::runtime::Handle),
}

impl RuntimeHandle {
    fn acquire() -> Result<Self, String> {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            Ok(Self::Borrowed(handle))
        } else {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .thread_name("rustcode-tokio")
                .build()
                .map_err(|e| format!("Failed to build Tokio runtime: {e}"))?;
            Ok(Self::Owned(rt))
        }
    }

    fn handle(&self) -> &tokio::runtime::Handle {
        match self {
            Self::Owned(rt) => rt.handle(),
            Self::Borrowed(h) => h,
        }
    }
}

/// Open the default `StorageBackend` for `AppPaths` (`SqliteStorageService` under `desktop`,
/// `InMemoryStorageBackend` otherwise).
pub fn open_storage_backend(app_paths: &AppPaths) -> Result<Arc<dyn StorageBackend>, StorageError> {
    #[cfg(feature = "desktop")]
    {
        let sqlite = SqliteStorageService::open(&app_paths.state_db_file)?;
        Ok(Arc::new(sqlite))
    }
    #[cfg(not(feature = "desktop"))]
    {
        let _ = app_paths;
        Ok(Arc::new(InMemoryStorageBackend::new()))
    }
}

/// Open default storage backend, logging and falling back to in-memory storage on error.
pub fn open_storage_backend_with_fallback(app_paths: &AppPaths) -> Arc<dyn StorageBackend> {
    match open_storage_backend(app_paths) {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!(
                "Warning: failed to initialize persistent storage at '{}': {}; using in-memory fallback",
                app_paths.state_db_file.display(),
                err
            );
            Arc::new(InMemoryStorageBackend::new())
        }
    }
}

/// Construct default OS backends (`ArboardClipboardBackend` + `RfdFileDialogBackend` under `desktop`)
/// or deterministic in-memory backends for tests.
pub fn default_os_ux_backends(
    use_os_backends: bool,
) -> (Arc<dyn ClipboardBackend>, Arc<dyn FileDialogBackend>) {
    let clipboard: Arc<dyn ClipboardBackend> = if use_os_backends {
        Arc::new(ArboardClipboardBackend::new())
    } else {
        Arc::new(InMemoryClipboardBackend::new())
    };

    #[cfg(feature = "desktop")]
    let file_dialog: Arc<dyn FileDialogBackend> = if use_os_backends {
        Arc::new(RfdFileDialogBackend::new())
    } else {
        Arc::new(MockFileDialogBackend::new())
    };

    #[cfg(not(feature = "desktop"))]
    let file_dialog: Arc<dyn FileDialogBackend> = Arc::new(MockFileDialogBackend::new());

    (clipboard, file_dialog)
}

/// Discover and load extensions from `dir` (either an extensions container directory
/// containing `<ext>/package.json` subdirectories, or a single extension directory).
pub fn scan_extensions_dir(loader: &mut ExtensionLoader, dir: &Path) -> usize {
    if !dir.is_dir() {
        return 0;
    }

    let mut loaded_count = 0usize;
    let mut existing_ids: HashSet<String> = loader
        .loaded_extensions()
        .iter()
        .map(|ext| ext.id().to_ascii_lowercase())
        .collect();

    let mut try_load = |ext_dir: &Path| {
        let manifest_path = ext_dir.join("package.json");
        if !manifest_path.is_file() {
            return;
        }
        let Ok(content) = std::fs::read_to_string(&manifest_path) else {
            return;
        };
        let Ok(manifest) = ExtensionLoader::parse_manifest(&content) else {
            return;
        };
        let ext_id = rustcode_ext_host::marketplace::ExtensionInstaller::extension_id(
            &manifest.publisher,
            &manifest.name,
        )
        .to_ascii_lowercase();
        if existing_ids.contains(&ext_id) {
            return;
        }
        if loader.load_from_dir(ext_dir).is_ok() {
            existing_ids.insert(ext_id);
            loaded_count += 1;
        }
    };

    if dir.join("package.json").is_file() {
        try_load(dir);
        return loaded_count;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut subdirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        subdirs.sort();
        for sub in subdirs {
            try_load(&sub);
        }
    }

    loaded_count
}

fn resolve_contained_extension_path(extension_root: &Path, rel_path: &str) -> Option<PathBuf> {
    let trimmed = rel_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Guard against Windows-style separators or drive prefixes on Unix hosts before canonicalize.
    if trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || trimmed.split(['/', '\\']).any(|seg| seg == "..")
        || trimmed
            .as_bytes()
            .get(1)
            .is_some_and(|&b| b == b':' && trimmed.as_bytes()[0].is_ascii_alphabetic())
    {
        return None;
    }
    let rel = Path::new(trimmed);
    if rel.is_absolute() {
        return None;
    }
    for comp in rel.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let canon_root = std::fs::canonicalize(extension_root).ok()?;
    let canon_target = std::fs::canonicalize(extension_root.join(rel)).ok()?;
    if !canon_target.starts_with(&canon_root) || !canon_target.is_file() {
        return None;
    }
    Some(canon_target)
}

/// Load any theme JSON files contributed by loaded extensions into `ThemeService`.
pub fn load_extension_themes(themes: &mut ThemeService, loader: &ExtensionLoader) {
    for ext in loader.loaded_extensions() {
        if let Some(contribs) = &ext.manifest.contributes {
            if let Some(theme_contribs) = &contribs.themes {
                for contrib in theme_contribs {
                    let Some(theme_path) =
                        resolve_contained_extension_path(&ext.root_dir, &contrib.path)
                    else {
                        continue;
                    };
                    if let Ok(loaded) = themes.load_theme_from_disk(&theme_path) {
                        if !contrib.label.trim().is_empty() {
                            themes.register_alias(&contrib.label, &loaded.name);
                        }
                    }
                }
            }
        }
    }
}

/// Load any snippet JSON files contributed by loaded extensions into `SnippetRegistry`.
pub fn load_extension_snippets(
    registry: &mut rustcode_editor::SnippetRegistry,
    loader: &ExtensionLoader,
) {
    for ext in loader.loaded_extensions() {
        if let Some(contribs) = &ext.manifest.contributes {
            if let Some(snippet_contribs) = &contribs.snippets {
                for contrib in snippet_contribs {
                    let Some(snippet_path) =
                        resolve_contained_extension_path(&ext.root_dir, &contrib.path)
                    else {
                        continue;
                    };
                    if let Ok(raw) = std::fs::read_to_string(&snippet_path) {
                        let _ = registry.load_vscode_json(&contrib.language, &raw);
                    }
                }
            }
        }
    }
}

/// Load any `language-configuration.json` files contributed by loaded extensions.
pub fn load_extension_language_configs(
    registry: &mut rustcode_editor::LanguageConfigurationRegistry,
    loader: &ExtensionLoader,
) {
    for ext in loader.loaded_extensions() {
        if let Some(contribs) = &ext.manifest.contributes {
            if let Some(langs) = &contribs.languages {
                for lang in langs {
                    let Some(cfg_rel) = &lang.configuration else {
                        continue;
                    };
                    let Some(cfg_path) =
                        resolve_contained_extension_path(&ext.root_dir, cfg_rel)
                    else {
                        continue;
                    };
                    if let Ok(raw) = std::fs::read_to_string(&cfg_path) {
                        let _ = registry.load_from_json_str(&lang.id, &raw);
                    }
                }
            }
        }
    }
}

/// Register extension `contributes.menus` contributions into `ContextMenuRegistry`.
pub fn load_extension_menus(
    registry: &mut rustcode_workbench::context_menu::ContextMenuRegistry,
    loader: &ExtensionLoader,
) {
    for ext in loader.loaded_extensions() {
        let Some(contribs) = &ext.manifest.contributes else {
            continue;
        };
        let Some(menus) = &contribs.menus else {
            continue;
        };
        let mut command_titles = HashMap::new();
        if let Some(cmds) = &contribs.commands {
            for cmd in cmds {
                command_titles.insert(cmd.command.clone(), cmd.title.clone());
            }
        }
        registry.register_contributions(menus, &command_titles);
    }
}

/// Register `contributes.{iconThemes,productIconThemes,jsonValidation,problemMatchers,submenus}` from loaded extensions.
pub fn load_extension_phase10_contribs(ctx: &mut RustCodeContext) {
    for ext in ctx.ext_loader.loaded_extensions() {
        let ext_id = ext.id();
        let Some(contribs) = &ext.manifest.contributes else {
            continue;
        };
        if let Some(icon_themes) = &contribs.icon_themes {
            for it in icon_themes {
                if let Some(path) = resolve_contained_extension_path(&ext.root_dir, &it.path) {
                    let _ = ctx
                        .theme
                        .load_icon_theme_from_disk(&it.id, &it.label, &path);
                }
            }
        }
        if let Some(prod_themes) = &contribs.product_icon_themes {
            for pt in prod_themes {
                if let Some(path) = resolve_contained_extension_path(&ext.root_dir, &pt.path) {
                    let _ = ctx
                        .theme
                        .load_product_icon_theme_from_disk(&pt.id, &pt.label, &path);
                }
            }
        }
        if let Some(json_vals) = &contribs.json_validation {
            for jv in json_vals {
                let _ = ctx.json_validation.register(&ext_id, &ext.root_dir, jv);
            }
        }
        if let Some(matchers) = &contribs.problem_matchers {
            for pm in matchers {
                let _ = ctx.problem_matchers.register(pm.clone());
            }
        }
        if let Some(submenus) = &contribs.submenus {
            ctx.context_menus.register_submenus(submenus);
        }
    }
}

/// Load user settings, user keybindings, workspace settings, built-in themes, and extension discovery.
pub fn load_disk_resources(
    ctx: &mut RustCodeContext,
    app_paths: &AppPaths,
    workspace_roots: &[PathBuf],
    disable_extensions: bool,
) {
    {
        let mut pm = rustcode_platform::profiles::ProfileManager::new(app_paths.profiles_dir());
        let _ = pm.load_from_disk();
        *ctx.profile_manager.lock() = pm;
    }
    if let Err(err) = ctx
        .config
        .load_user_settings_from_disk(&app_paths.settings_file)
    {
        eprintln!("Warning: {err}");
    }

    if let Err(err) = ctx
        .keybindings
        .load_user_keybindings_from_disk(&app_paths.keybindings_file)
    {
        eprintln!("Warning: {err}");
    }

    let _ = ctx.theme.register_builtin_themes(None);

    if !disable_extensions {
        scan_extensions_dir(&mut ctx.ext_loader, &app_paths.extensions_dir);
    }

    for root in workspace_roots {
        let ws_paths = AppPaths::workspace_paths(root);
        if let Err(err) = ctx
            .config
            .load_workspace_settings_from_disk(&ws_paths.settings_file)
        {
            eprintln!("Warning: {err}");
        }
        if !disable_extensions {
            scan_extensions_dir(&mut ctx.ext_loader, &ws_paths.extensions_dir);
        }
        if let Ok(recs) = WorkspaceExtensionsRecommendations::load_from_workspace(root) {
            for rec in recs.recommendations {
                if !ctx
                    .recommended_extensions
                    .iter()
                    .any(|r| r.eq_ignore_ascii_case(&rec))
                {
                    ctx.recommended_extensions.push(rec);
                }
            }
        }
    }

    load_extension_themes(&mut ctx.theme, &ctx.ext_loader);
    load_extension_phase10_contribs(ctx);
    load_extension_snippets(&mut ctx.snippets, &ctx.ext_loader);
    load_extension_language_configs(&mut ctx.language_configs, &ctx.ext_loader);
    load_extension_menus(&mut ctx.context_menus, &ctx.ext_loader);
    let _ = ctx.theme.sync_with_configuration(&ctx.config);
}

/// Derive a stable storage key for workspace persistence.
pub fn workspace_storage_key(workspace_root: Option<&Path>) -> String {
    match workspace_root {
        Some(path) if !path.as_os_str().is_empty() => {
            let cleaned = strip_windows_verbatim_prefix(path.to_path_buf());
            let canonical = std::fs::canonicalize(&cleaned)
                .map(strip_windows_verbatim_prefix)
                .unwrap_or(cleaned);
            canonical.to_string_lossy().into_owned()
        }
        _ => DEFAULT_EMPTY_WORKSPACE_KEY.to_string(),
    }
}

/// Snapshot open tabs, cursor/scroll positions, layout visibility, and active theme.
pub fn capture_workspace_state(window: &WorkbenchWindow) -> PersistedWorkspaceState {
    let current_group = window.view.editor_area.model.current_group_ref();
    let mut open_files = Vec::new();
    let mut tab_states = Vec::new();
    let mut seen_uris = HashSet::new();

    for group in &window.view.editor_area.model.groups {
        for tab in &group.tabs {
            if !seen_uris.insert(tab.uri.clone()) {
                continue;
            }
            if tab.uri.starts_with("settings://") || tab.uri.starts_with("untitled://") {
                continue;
            }
            open_files.push(tab.uri.clone());
            if let Some(editor) = window.view.editor_area.any_editor_for_uri(&tab.uri) {
                let pos = editor.primary_position();
                tab_states.push(PersistedTabState {
                    uri: tab.uri.clone(),
                    cursor_line: pos.line,
                    cursor_col: pos.column,
                    scroll_top: editor.scroll_top,
                    scroll_left: editor.scroll_left,
                });
            }
        }
    }

    let (sb_vis, sec_sb_vis, pan_vis, tb_vis) =
        if let Some(snap) = &window.view.layout.zen_snapshot {
            (
                snap.sidebar_visible,
                snap.secondary_sidebar_visible,
                snap.panel_visible,
                snap.title_bar_visible,
            )
        } else {
            (
                window.view.layout.sidebar_visible,
                window.view.layout.secondary_sidebar_visible,
                window.view.layout.panel_visible,
                window.view.layout.title_bar_visible,
            )
        };

    PersistedWorkspaceState {
        open_files,
        active_tab_index: current_group.active_tab_index,
        tab_states,
        sidebar_visible: sb_vis,
        sidebar_width: window.view.layout.sidebar_width,
        panel_visible: pan_vis,
        panel_height: window.view.layout.panel_height,
        active_theme_id: Some(window.theme.current().name.clone()),
        secondary_sidebar_visible: sec_sb_vis,
        secondary_sidebar_width: window.view.layout.secondary_sidebar_width,
        panel_position: window.view.layout.panel_position.as_str().to_string(),
        panel_width: window.view.layout.panel_width,
        title_bar_visible: tb_vis,
        zoom_level: window.view.layout.zoom_level,
        zen_mode: window.view.layout.zen_mode,
    }
}

/// Persist current workspace state and recent workspace entry to `window.storage_backend`.
pub fn save_workbench_state(window: &WorkbenchWindow) -> Result<(), StorageError> {
    let ws_root = window
        .view
        .workspace
        .folders()
        .first()
        .map(|f| f.path.as_path());
    if let Some(root) = ws_root {
        window.storage_backend.add_recent_workspace(root)?;
    }
    let key = workspace_storage_key(ws_root);
    let state = capture_workspace_state(window);
    window.storage_backend.save_workspace_state(&key, &state)
}

/// Restore persisted workspace state (layout, theme, open tabs, active tab, cursor/scroll).
pub fn restore_workbench_state(
    window: &mut WorkbenchWindow,
    workspace_root: Option<&Path>,
) -> Result<bool, StorageError> {
    let key = workspace_storage_key(workspace_root);
    let Some(state) = window.storage_backend.load_workspace_state(&key)? else {
        return Ok(false);
    };

    window.view.layout.mutate(|l| {
        l.sidebar_visible = state.sidebar_visible;
        if state.sidebar_width.is_finite() && state.sidebar_width > 50.0 {
            l.sidebar_width = state.sidebar_width;
        }
        l.panel_visible = state.panel_visible;
        if state.panel_height.is_finite() && state.panel_height > 40.0 {
            l.panel_height = state.panel_height;
        }
        l.secondary_sidebar_visible = state.secondary_sidebar_visible;
        if state.secondary_sidebar_width.is_finite() && state.secondary_sidebar_width > 50.0 {
            l.secondary_sidebar_width = state.secondary_sidebar_width;
        }
        if !state.panel_position.is_empty() {
            l.panel_position =
                rustcode_workbench::layout::PanelPosition::from_str_lossy(&state.panel_position);
        }
        if state.panel_width.is_finite() && state.panel_width > 50.0 {
            l.panel_width = state.panel_width;
        }
        l.title_bar_visible = state.title_bar_visible;
        if state.zoom_level.is_finite() {
            l.zoom_level = state.zoom_level.clamp(-5.0, 5.0);
        }
        l.zen_mode = false;
        l.zen_snapshot = None;
        if state.zen_mode {
            l.toggle_zen_mode();
        }
    });

    if let Some(theme_name) = &state.active_theme_id {
        if window.theme.set_theme_by_name(theme_name).is_ok() {
            let active_theme = window.theme.current().clone();
            window.view.editor_area.for_each_editor_mut(|_, ed| {
                ed.set_theme(active_theme.clone());
            });
        }
    }

    for uri in &state.open_files {
        let path = if is_file_uri(uri) {
            file_uri_to_path(uri)
        } else {
            PathBuf::from(uri)
        };
        if path.is_file() {
            window.open_path(&path, false);
        }
    }

    for tab_state in &state.tab_states {
        window
            .view
            .editor_area
            .for_each_editor_for_uri_mut(&tab_state.uri, |ed| {
                let line_count = ed.buffer.read().line_count().max(1);
                let line = tab_state.cursor_line.min(line_count - 1);
                let line_len = ed
                    .buffer
                    .read()
                    .get_line_content(line)
                    .map_or(0, |l| l.chars().map(|c| c.len_utf16()).sum());
                let col = tab_state.cursor_col.min(line_len);
                ed.set_primary_position(Position::new(line, col));
                if tab_state.scroll_top.is_finite() && tab_state.scroll_top >= 0.0 {
                    ed.scroll_top = tab_state.scroll_top;
                }
                if tab_state.scroll_left.is_finite() && tab_state.scroll_left >= 0.0 {
                    ed.scroll_left = tab_state.scroll_left;
                }
            });
    }

    if let Some(idx) = state.active_tab_index {
        let tab_count = window.view.editor_area.model.current_group_ref().tabs.len();
        if idx < tab_count {
            window
                .view
                .editor_area
                .model
                .current_group()
                .active_tab_index = Some(idx);
            window.view.editor_area.sync_editor_focus();
            window
                .view
                .editor_area
                .sync_breadcrumbs(Some(&window.view.workspace));
        }
    }

    window.sync_all();
    Ok(true)
}

/// Persist or query extension `Memento` (`globalState` / `workspaceState`) via `StorageBackend`.
pub fn memento_get(
    storage: &dyn StorageBackend,
    scope: MementoScope,
    ext_id: &str,
    key: &str,
) -> Result<Option<Value>, StorageError> {
    storage.memento_get(scope, ext_id, key)
}

pub fn memento_update(
    storage: &dyn StorageBackend,
    scope: MementoScope,
    ext_id: &str,
    key: &str,
    value: Option<Value>,
) -> Result<(), StorageError> {
    storage.memento_update(scope, ext_id, key, value)
}

fn apply_paths_and_goto(
    window: &mut WorkbenchWindow,
    paths: &[PathBuf],
    goto: &[GotoTarget],
    restore_saved: bool,
    allow_empty_restore: bool,
) {
    let mut workspace_dirs = Vec::new();
    let mut file_paths = Vec::new();

    for path in paths {
        if path.is_dir() {
            workspace_dirs.push(path.clone());
        } else {
            file_paths.push(path.clone());
        }
    }

    if let Some(first_dir) = workspace_dirs.first() {
        if window.view.workspace.folders().is_empty() {
            window.open_workspace(first_dir);
            for extra_dir in workspace_dirs.iter().skip(1) {
                let cleaned = strip_windows_verbatim_prefix(extra_dir.clone());
                window.view.workspace.add_folder(cleaned, None);
            }
        } else {
            for dir in &workspace_dirs {
                let cleaned = strip_windows_verbatim_prefix(dir.clone());
                window.view.workspace.add_folder(cleaned, None);
            }
        }
        if workspace_dirs.len() > 1 {
            window
                .view
                .explorer
                .model
                .sync_from_workspace(&window.view.workspace);
        }
        if restore_saved && file_paths.is_empty() && goto.is_empty() {
            let _ = restore_workbench_state(window, Some(first_dir));
        }
    } else if restore_saved && file_paths.is_empty() && goto.is_empty() && allow_empty_restore {
        let _ = restore_workbench_state(window, None);
    }

    for file in &file_paths {
        window.open_path(file, false);
    }

    for target in goto {
        open_goto_target(window, target);
    }
}

/// Apply CLI positional `[path...]` and `--goto <file:line[:col]>` arguments to `window`.
pub fn apply_cli_targets(window: &mut WorkbenchWindow, opts: &CliOptions, restore_saved: bool) {
    apply_paths_and_goto(window, &opts.paths, &opts.goto, restore_saved, opts.gui);
}

/// Apply an IPC [`crate::multi_window::OpenRequest`] to an existing or newly opened window.
pub fn apply_open_request(
    window: &mut WorkbenchWindow,
    req: &crate::multi_window::OpenRequest,
) {
    let had_folders = !window.view.workspace.folders().is_empty();
    apply_paths_and_goto(window, &req.paths, &req.goto, !had_folders, false);
    window.sync_all();
}

/// Open a `--goto` file and jump cursor to its clamped 0-based line & column.
pub fn open_goto_target(window: &mut WorkbenchWindow, target: &GotoTarget) {
    window.open_path(&target.path, false);
    if let Some(editor) = window.view.editor_area.active_editor_mut() {
        let line_count = editor.buffer.read().line_count().max(1);
        let line = target.line_zero_based().min(line_count - 1);
        let line_utf16_len = editor
            .buffer
            .read()
            .get_line_content(line)
            .map_or(0, |l| l.chars().map(|c| c.len_utf16()).sum());
        let col = target.col_zero_based().min(line_utf16_len);
        editor.jump_to_position_and_unfold(Position::new(line, col));
    }
}

/// Bootstrap a full `RustCodeContext`, `WorkbenchWindow`, and `RuntimeLoop` from `CliOptions`.
pub fn bootstrap_app(
    opts: &CliOptions,
    use_os_backends: bool,
) -> Result<BootstrappedApp, String> {
    let app_paths = AppPaths::resolve(
        opts.user_data_dir.as_deref(),
        opts.extensions_dir.as_deref(),
    );

    let storage_backend = open_storage_backend_with_fallback(&app_paths);
    let (clipboard_backend, file_dialog_backend) = default_os_ux_backends(use_os_backends);

    let workspace_dirs: Vec<PathBuf> = opts
        .paths
        .iter()
        .filter(|p| p.is_dir())
        .cloned()
        .collect();

    let mut context = RustCodeContext::new();
    context
        .language_bridge
        .env_manager
        .set_platform_clipboard(Arc::clone(&clipboard_backend));
    if use_os_backends {
        context.language_bridge.secret_manager.set_backend(Arc::new(
            rustcode_platform::OsKeyringSecretStorageBackend::new(
                "rustcode",
                app_paths.secrets_dir(),
            ),
        ));
    }

    load_disk_resources(
        &mut context,
        &app_paths,
        &workspace_dirs,
        opts.disable_extensions,
    );

    let shared_ext_loader = Arc::new(RwLock::new(context.ext_loader.clone()));
    context
        .language_bridge
        .set_extension_loader(Arc::clone(&shared_ext_loader));

    let mut window = context
        .create_window()
        .with_storage_backend(storage_backend)
        .with_clipboard_backend(clipboard_backend)
        .with_file_dialog_backend(file_dialog_backend);

    apply_cli_targets(&mut window, opts, true);

    let runtime_cfg = RuntimeLoopConfig {
        disable_extensions: opts.disable_extensions,
        ..RuntimeLoopConfig::default()
    };
    let mut runtime_loop =
        RuntimeLoop::new_with_shared_loader(runtime_cfg, shared_ext_loader)?;
    runtime_loop.register_contributed_command_stubs(&mut window);

    Ok(BootstrappedApp {
        context,
        window,
        app_paths,
        runtime_loop,
    })
}

type PendingHeartbeatSlot = Arc<Mutex<Option<(Instant, Result<u64, String>)>>>;

/// Live Tokio runtime loop driving `FileSystemWatcher`, `AutoSaveService`,
/// `NodeExtHostProcess` auto-activation/crash recovery, and `WorkbenchWindow::poll_runtime_events()`.
pub struct RuntimeLoop {
    runtime: RuntimeHandle,
    config: RuntimeLoopConfig,
    watcher: Option<FileSystemWatcher>,
    watched_roots: HashSet<PathBuf>,
    fs_events: Arc<Mutex<Vec<FileChangeEvent>>>,
    _fs_sub: Option<Box<dyn Disposable>>,
    auto_save: AutoSaveService,
    tracked_doc_versions: HashMap<PathBuf, u64>,
    last_focused_doc_path: Option<PathBuf>,
    last_focused_region: FocusedRegion,
    ext_loader: Arc<RwLock<ExtensionLoader>>,
    ext_host_process: Option<NodeExtHostProcess>,
    ext_host_shutting_down: bool,
    ext_host_spawn_failed_permanently: bool,
    ext_host_restart_count: usize,
    ext_host_next_restart_at: Option<Instant>,
    ext_host_last_healthy_at: Option<Instant>,
    activated_extension_ids: HashSet<String>,
    activating_extension_ids: Arc<Mutex<HashSet<String>>>,
    dispatched_activation_events: HashSet<String>,
    pending_command_activations: Arc<Mutex<Vec<String>>>,
    conflict_notified_paths: HashSet<PathBuf>,
    last_auto_save_setting: Option<(String, u64)>,
    watchdog: ExtHostWatchdog,
    pending_heartbeat_result: PendingHeartbeatSlot,
    watchdog_warning_shown: bool,
}

impl RuntimeLoop {
    pub fn new(config: RuntimeLoopConfig) -> Result<Self, String> {
        Self::new_with_shared_loader(config, Arc::new(RwLock::new(ExtensionLoader::new())))
    }

    pub fn new_with_shared_loader(
        config: RuntimeLoopConfig,
        ext_loader: Arc<RwLock<ExtensionLoader>>,
    ) -> Result<Self, String> {
        let runtime = RuntimeHandle::acquire()?;
        let fs_events: Arc<Mutex<Vec<FileChangeEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let (watcher, fs_sub) = match FileSystemWatcher::new() {
            Ok(w) => {
                let q = Arc::clone(&fs_events);
                let sub = w.on_any_event.event().attach(move |ev| {
                    q.lock().push(ev.clone());
                });
                (Some(w), Some(sub))
            }
            Err(err) => {
                eprintln!("Warning: failed to initialize FileSystemWatcher: {err}");
                (None, None)
            }
        };

        Ok(Self {
            runtime,
            config,
            watcher,
            watched_roots: HashSet::new(),
            fs_events,
            _fs_sub: fs_sub,
            auto_save: AutoSaveService::new(AutoSaveConfig::default()),
            tracked_doc_versions: HashMap::new(),
            last_focused_doc_path: None,
            last_focused_region: FocusedRegion::Editor,
            ext_loader,
            ext_host_process: None,
            ext_host_shutting_down: false,
            ext_host_spawn_failed_permanently: false,
            ext_host_restart_count: 0,
            ext_host_next_restart_at: None,
            ext_host_last_healthy_at: None,
            activated_extension_ids: HashSet::new(),
            activating_extension_ids: Arc::new(Mutex::new(HashSet::new())),
            dispatched_activation_events: HashSet::new(),
            pending_command_activations: Arc::new(Mutex::new(Vec::new())),
            conflict_notified_paths: HashSet::new(),
            last_auto_save_setting: None,
            watchdog: ExtHostWatchdog::default(),
            pending_heartbeat_result: Arc::new(Mutex::new(None)),
            watchdog_warning_shown: false,
        })
    }

    pub fn watchdog_health(&self) -> WatchdogHealth {
        self.watchdog.check_health(Instant::now())
    }

    pub fn restart_ext_host(&mut self, window: &mut WorkbenchWindow) -> bool {
        if let Some(mut proc) = self.ext_host_process.take() {
            if let RuntimeHandle::Owned(ref rt) = self.runtime {
                let _ = rt.block_on(proc.shutdown());
            } else {
                drop(proc);
            }
        }
        {
            let mut loader = self.ext_loader.write();
            let ids: Vec<String> = loader
                .loaded_extensions()
                .iter()
                .map(|e| e.id())
                .collect();
            for id in ids {
                loader.deactivate(&id);
            }
        }
        self.activating_extension_ids.lock().clear();
        self.ext_host_spawn_failed_permanently = false;
        self.ext_host_next_restart_at = None;
        self.watchdog.reset_restart_budget(Instant::now());
        self.watchdog_warning_shown = false;
        if self.spawn_ext_host(window) {
            self.ext_host_restart_count += 1;
            let to_reactivate: Vec<String> = self.activated_extension_ids.iter().cloned().collect();
            for ext_id in to_reactivate {
                self.spawn_activate_extension(window, &ext_id);
            }
            true
        } else {
            false
        }
    }

    pub fn tokio_handle(&self) -> &tokio::runtime::Handle {
        self.runtime.handle()
    }

    pub fn ext_loader(&self) -> &Arc<RwLock<ExtensionLoader>> {
        &self.ext_loader
    }

    pub fn is_ext_host_running(&self) -> bool {
        self.ext_host_process
            .as_ref()
            .is_some_and(NodeExtHostProcess::is_running)
    }

    pub fn ext_host_restart_count(&self) -> usize {
        self.ext_host_restart_count
    }

    /// Manually attach a `NodeExtHostProcess` (useful in tests or custom host wiring).
    pub fn attach_ext_host_process(&mut self, process: NodeExtHostProcess) {
        self.ext_host_process = Some(process);
        self.ext_host_last_healthy_at = Some(Instant::now());
        self.ext_host_shutting_down = false;
    }

    /// Watch a directory recursively via `FileSystemWatcher`.
    pub fn watch_directory(&mut self, dir: &Path) -> Result<(), String> {
        let cleaned = strip_windows_verbatim_prefix(dir.to_path_buf());
        if !cleaned.is_dir() || self.watched_roots.contains(&cleaned) {
            return Ok(());
        }
        if let Some(watcher) = &mut self.watcher {
            watcher
                .watch(&cleaned, true)
                .map_err(|e| format!("Failed to watch '{}': {e}", cleaned.display()))?;
            self.watched_roots.insert(cleaned);
        }
        Ok(())
    }

    /// Register non-blocking stub handlers for extension-contributed commands that trigger
    /// `onCommand:<cmd>` activation when executed before the extension activates.
    pub fn register_contributed_command_stubs(&mut self, window: &mut WorkbenchWindow) {
        if self.config.disable_extensions {
            return;
        }
        let mut contributed_cmds = Vec::new();
        {
            let loader = self.ext_loader.read();
            for ext in loader.loaded_extensions() {
                if let Some(contribs) = &ext.manifest.contributes {
                    if let Some(cmds) = &contribs.commands {
                        for c in cmds {
                            contributed_cmds.push(c.command.clone());
                        }
                    }
                }
                for ev in &ext.manifest.activation_events {
                    if let Some(cmd) = ev.strip_prefix("onCommand:") {
                        if !cmd.is_empty() {
                            contributed_cmds.push(cmd.to_string());
                        }
                    }
                }
            }
        }

        for cmd_id in contributed_cmds {
            if window.commands.has_command(&cmd_id) {
                continue;
            }
            let queue = Arc::clone(&self.pending_command_activations);
            let cmd_clone = cmd_id.clone();
            let disp = window.commands.register(&cmd_id, move |_| {
                queue.lock().push(cmd_clone.clone());
                Ok(serde_json::json!({ "activating": cmd_clone }))
            });
            window
                .extension_disposables
                .entry("__rustcode_activation_stubs__".to_string())
                .or_default()
                .add_boxed(disp);
        }
    }

    /// Queue or trigger an `onCommand:<cmd_id>` activation event.
    pub fn notify_command_invoked(&mut self, window: &mut WorkbenchWindow, cmd_id: &str) {
        if self.config.disable_extensions {
            return;
        }
        let event = format!("onCommand:{cmd_id}");
        self.trigger_activation_event(window, &event);
    }

    /// Notify `AutoSaveService` that the application window lost OS focus (`onWindowChange`).
    pub fn on_window_blur(&mut self, window: &mut WorkbenchWindow) {
        self.sync_auto_save_config(window);
        self.register_open_documents(window);
        let results = self.auto_save.on_window_blur(&window.file_system);
        self.handle_auto_save_results(window, results);
    }

    /// Perform one non-blocking tick of all runtime subsystems.
    pub fn tick(&mut self, window: &mut WorkbenchWindow) {
        // 1. Ensure workspace folders are watched
        let ws_folders: Vec<PathBuf> = window
            .view
            .workspace
            .folders()
            .iter()
            .map(|f| f.path.clone())
            .collect();
        for folder in ws_folders {
            let _ = self.watch_directory(&folder);
        }

        // 2. Drain FileSystemWatcher events -> Explorer refresh + clean reload / dirty conflict
        self.process_fs_events(window);

        // 3. Sync AutoSaveService config, document edits, focus change, and timer ticks
        self.process_auto_save(window);

        // 4. Poll WorkbenchWindow runtime events (PTY, LSP, DAP, Webview IPC, commands)
        window.poll_runtime_events();

        // 5. Drive NodeExtHostProcess auto-activation & bounded crash recovery
        self.process_ext_host(window);
    }

    fn process_fs_events(&mut self, window: &mut WorkbenchWindow) {
        let raw_events = {
            let mut guard = self.fs_events.lock();
            if guard.is_empty() {
                return;
            }
            std::mem::take(&mut *guard)
        };

        let mut latest_by_path: HashMap<PathBuf, FileChangeType> = HashMap::new();
        for ev in raw_events {
            let cleaned = strip_windows_verbatim_prefix(ev.path);
            latest_by_path.insert(cleaned, ev.change_type);
        }

        window.view.explorer.model.refresh_from_disk();
        let _ = window.view.refresh_scm();

        for (path, change_type) in latest_by_path {
            let Some(uri) = window.view.editor_area.find_open_tab_uri_for_path(&path) else {
                continue;
            };
            let is_dirty = window
                .view
                .editor_area
                .any_editor_for_uri(&uri)
                .is_some_and(|ed| ed.is_dirty());

            if is_dirty {
                if self.conflict_notified_paths.insert(path.clone()) {
                    push_warning_notification(
                        window,
                        format!(
                            "File '{}' changed on disk while buffer has unsaved changes.",
                            path.display()
                        ),
                    );
                }
                continue;
            }

            match change_type {
                FileChangeType::Created | FileChangeType::Changed => {
                    if window.refresh_open_file_from_disk(&path) {
                        self.conflict_notified_paths.remove(&path);
                        if let Some(doc_lock) = window
                            .view
                            .editor_area
                            .any_editor_for_uri(&uri)
                            .and_then(|ed| ed.document.clone())
                        {
                            self.tracked_doc_versions
                                .insert(path.clone(), doc_lock.read().version_id);
                        }
                    }
                }
                FileChangeType::Deleted => {
                    if self.conflict_notified_paths.insert(path.clone()) {
                        push_warning_notification(
                            window,
                            format!("File '{}' was deleted on disk.", path.display()),
                        );
                    }
                }
            }
        }
    }

    fn sync_auto_save_config(&mut self, window: &WorkbenchWindow) {
        let mode_str = window
            .config
            .get::<String>("files.autoSave")
            .or_else(|| window.config.get::<String>("editor.autoSave"))
            .unwrap_or_else(|| "off".to_string());
        let delay_ms = window
            .config
            .get::<u64>("files.autoSaveDelay")
            .unwrap_or(1000)
            .max(10);

        if self
            .last_auto_save_setting
            .as_ref()
            .is_some_and(|(m, d)| m == &mode_str && *d == delay_ms)
        {
            return;
        }

        let mode = match mode_str.as_str() {
            "afterDelay" => AutoSaveMode::AfterDelay,
            "onFocusChange" => AutoSaveMode::OnFocusChange,
            "onWindowChange" => AutoSaveMode::OnWindowChange,
            _ => AutoSaveMode::Off,
        };

        self.auto_save.set_config(AutoSaveConfig {
            mode,
            delay: Duration::from_millis(delay_ms),
        });
        self.last_auto_save_setting = Some((mode_str, delay_ms));
    }

    fn register_open_documents(&mut self, window: &WorkbenchWindow) {
        for editor in window.view.editor_area.editors.values() {
            let Some(doc_lock) = &editor.document else {
                continue;
            };
            let (path_opt, version_id, is_dirty) = {
                let d = doc_lock.read();
                (d.path.clone(), d.version_id, d.is_dirty)
            };
            let Some(path) = path_opt else {
                continue;
            };
            let cleaned = strip_windows_verbatim_prefix(path);
            if !self.tracked_doc_versions.contains_key(&cleaned) {
                self.auto_save.register(Arc::clone(doc_lock));
                self.tracked_doc_versions.insert(cleaned.clone(), version_id);
                if is_dirty {
                    self.auto_save.on_document_content_changed(&cleaned);
                }
            } else if self.tracked_doc_versions.get(&cleaned).copied() != Some(version_id) {
                self.tracked_doc_versions.insert(cleaned.clone(), version_id);
                if is_dirty {
                    self.auto_save.on_document_content_changed(&cleaned);
                } else {
                    self.conflict_notified_paths.remove(&cleaned);
                }
            } else if !is_dirty {
                self.conflict_notified_paths.remove(&cleaned);
            }
        }
    }

    fn handle_auto_save_results(
        &mut self,
        window: &mut WorkbenchWindow,
        results: Vec<Result<rustcode_platform::fs::FileMetadata, rustcode_platform::fs::FileSystemError>>,
    ) {
        let mut any_saved = false;
        let mut saved_paths = Vec::new();
        for meta in results.into_iter().flatten() {
            any_saved = true;
            saved_paths.push(meta.path);
        }
        if any_saved {
            window.view.editor_area.sync_dirty_states();
            window.sync_document_lifecycle();
            for p in saved_paths {
                self.conflict_notified_paths.remove(&p);
                if let Some(bridge) = &window.language_bridge {
                    let uri = window
                        .view
                        .editor_area
                        .find_open_tab_uri_for_path(&p)
                        .unwrap_or_else(|| path_to_file_uri(&p));
                    bridge.did_save_document_sync(&uri);
                }
            }
        }
    }

    fn process_auto_save(&mut self, window: &mut WorkbenchWindow) {
        self.sync_auto_save_config(window);
        self.register_open_documents(window);

        let current_active_path = window
            .view
            .editor_area
            .active_editor()
            .and_then(|ed| ed.document.as_ref())
            .and_then(|d| d.read().path.clone())
            .map(strip_windows_verbatim_prefix);
        let current_focus = window.view.focused_region;

        let lost_editor_focus = self.last_focused_doc_path.as_ref().is_some_and(|prev| {
            current_active_path.as_ref() != Some(prev)
                || (self.last_focused_region == FocusedRegion::Editor
                    && current_focus != FocusedRegion::Editor)
        });

        if lost_editor_focus {
            if let Some(prev) = self.last_focused_doc_path.clone() {
                if let Some(res) = self.auto_save.on_focus_lost(&prev, &window.file_system) {
                    self.handle_auto_save_results(window, vec![res]);
                }
            }
        }

        self.last_focused_doc_path = current_active_path;
        self.last_focused_region = current_focus;

        let tick_results = self.auto_save.tick(&window.file_system);
        if !tick_results.is_empty() {
            self.handle_auto_save_results(window, tick_results);
        }
    }

    fn process_ext_host(&mut self, window: &mut WorkbenchWindow) {
        if window.pending_ext_host_restart_request {
            window.pending_ext_host_restart_request = false;
            let _ = self.restart_ext_host(window);
            return;
        }
        if self.config.disable_extensions {
            return;
        }

        if let Some((ack_at, res)) = self.pending_heartbeat_result.lock().take() {
            match res {
                Ok(lat) => {
                    self.watchdog.record_ack(ack_at, lat);
                    self.watchdog_warning_shown = false;
                }
                Err(_) => {
                    self.watchdog.record_miss();
                }
            }
        }

        // Check if running ext host exited unexpectedly (crash detection)
        if let Some(proc) = &self.ext_host_process {
            if !proc.is_running() && !self.ext_host_shutting_down {
                self.ext_host_process = None;
                self.ext_host_last_healthy_at = None;
                {
                    let mut loader = self.ext_loader.write();
                    let ids: Vec<String> = loader
                        .loaded_extensions()
                        .iter()
                        .map(|e| e.id())
                        .collect();
                    for id in ids {
                        loader.deactivate(&id);
                    }
                }
                self.activating_extension_ids.lock().clear();

                if self.ext_host_restart_count < self.config.max_ext_host_restarts {
                    self.ext_host_next_restart_at =
                        Some(Instant::now() + self.config.ext_host_restart_backoff);
                } else if !self.ext_host_spawn_failed_permanently {
                    self.ext_host_spawn_failed_permanently = true;
                    push_warning_notification(
                        window,
                        "Extension host terminated unexpectedly and exceeded automatic restart limit.",
                    );
                }
            } else if proc.is_running() {
                let now = Instant::now();
                if self.watchdog.should_send_heartbeat(now) {
                    if let Some(bridge) = window.language_bridge.clone() {
                        self.watchdog.mark_ping_sent(now);
                        let slot = Arc::clone(&self.pending_heartbeat_result);
                        self.runtime.handle().spawn(async move {
                            let res = bridge.ping_ext_host().await;
                            *slot.lock() = Some((Instant::now(), res));
                        });
                    }
                }
                match self.watchdog.check_health(now) {
                    WatchdogHealth::Healthy { .. } => {}
                    WatchdogHealth::Unresponsive { .. } => {
                        if !self.watchdog_warning_shown {
                            self.watchdog_warning_shown = true;
                            push_warning_notification(
                                window,
                                "Extension Host unresponsive — use 'Developer: Restart Extension Host' to restart.",
                            );
                        }
                    }
                    WatchdogHealth::RestartLimitReached { .. } => {
                        if !self.watchdog_warning_shown {
                            self.watchdog_warning_shown = true;
                            push_warning_notification(
                                window,
                                "Extension Host unresponsive and reached auto-restart limit.",
                            );
                        }
                    }
                }
                if let Some(since) = self.ext_host_last_healthy_at {
                    if since.elapsed() >= self.config.ext_host_healthy_reset_after {
                        self.ext_host_restart_count = 0;
                        self.ext_host_last_healthy_at = Some(now);
                    }
                }
            }
        }

        // Handle scheduled restart
        if let Some(deadline) = self.ext_host_next_restart_at {
            if Instant::now() >= deadline {
                self.ext_host_next_restart_at = None;
                self.ext_host_restart_count += 1;
                if self.spawn_ext_host(window) {
                    let to_reactivate: Vec<String> =
                        self.activated_extension_ids.iter().cloned().collect();
                    for ext_id in to_reactivate {
                        self.spawn_activate_extension(window, &ext_id);
                    }
                }
            }
            return;
        }

        if self.config.spawn_ext_host_on_startup && self.ext_host_process.is_none() {
            let _ = self.spawn_ext_host(window);
        }

        // Startup "*" activation
        if !self.dispatched_activation_events.contains("*") {
            self.dispatched_activation_events.insert("*".to_string());
            self.trigger_activation_event(window, "*");
        }

        // "onLanguage:<lang>" for open editors
        let open_langs: Vec<String> = window
            .view
            .editor_area
            .editors
            .values()
            .filter_map(|ed| ed.document.as_ref().map(|d| d.read().language_id.clone()))
            .collect();
        for lang in open_langs {
            let event = format!("onLanguage:{lang}");
            if self.dispatched_activation_events.insert(event.clone()) {
                self.trigger_activation_event(window, &event);
            }
        }

        // "onCommand:<cmd>" from queued stub invocations
        let pending_cmds: Vec<String> = {
            let mut q = self.pending_command_activations.lock();
            if q.is_empty() {
                Vec::new()
            } else {
                std::mem::take(&mut *q)
            }
        };
        for cmd in pending_cmds {
            let event = format!("onCommand:{cmd}");
            self.dispatched_activation_events.insert(event.clone());
            self.trigger_activation_event(window, &event);
        }
    }

    fn spawn_ext_host(&mut self, window: &WorkbenchWindow) -> bool {
        if self.config.disable_extensions
            || self.ext_host_shutting_down
            || self.ext_host_spawn_failed_permanently
        {
            return false;
        }
        let Some(bridge) = window.language_bridge.as_ref() else {
            return false;
        };

        let node_cfg = NodeExtHostConfig {
            node_binary: self.config.node_binary.clone(),
            entry_script: self.config.ext_host_entry.clone(),
            wire_format: self.config.ext_host_wire_format,
            cwd: window
                .view
                .workspace
                .folders()
                .first()
                .map(|f| f.path.clone()),
            ..Default::default()
        };

        let _guard = self.runtime.handle().enter();
        match NodeExtHostProcess::spawn(Arc::clone(bridge), node_cfg) {
            Ok(proc) => {
                self.ext_host_process = Some(proc);
                self.ext_host_last_healthy_at = Some(Instant::now());
                bridge.push_initial_state();
                true
            }
            Err(err) => {
                eprintln!("Warning: could not start Node.js extension host: {err}");
                self.ext_host_spawn_failed_permanently = true;
                false
            }
        }
    }

    /// Trigger activation for all loaded extensions matching `event` (e.g. `"*"`,
    /// `"onLanguage:rust"`, `"onCommand:ext.hello"`), without blocking the UI thread.
    pub fn trigger_activation_event(&mut self, window: &WorkbenchWindow, event: &str) {
        if self.config.disable_extensions {
            return;
        }
        let matching_ids: Vec<(String, bool)> = {
            let loader = self.ext_loader.read();
            loader
                .extensions_for_event(event)
                .into_iter()
                .filter(|ext| !ext.is_activated)
                .map(|ext| (ext.id(), ext.manifest.main.is_some()))
                .collect()
        };

        if matching_ids.is_empty() {
            return;
        }

        let needs_node = matching_ids.iter().any(|(_, has_main)| *has_main);
        if needs_node && self.ext_host_process.is_none() && self.ext_host_next_restart_at.is_none()
        {
            let _ = self.spawn_ext_host(window);
        }

        for (ext_id, _) in matching_ids {
            self.activated_extension_ids.insert(ext_id.clone());
            self.spawn_activate_extension(window, &ext_id);
        }
    }

    fn spawn_activate_extension(&self, window: &WorkbenchWindow, ext_id: &str) {
        let Some(bridge) = window.language_bridge.as_ref().map(Arc::clone) else {
            return;
        };
        if !self
            .activating_extension_ids
            .lock()
            .insert(ext_id.to_string())
        {
            return;
        }

        let id = ext_id.to_string();
        let activating = Arc::clone(&self.activating_extension_ids);
        self.runtime.handle().spawn(async move {
            let _ = bridge.activate_extension(&id).await;
            activating.lock().remove(&id);
        });
    }

    /// Gracefully shut down watcher and child processes synchronously.
    pub fn shutdown(&mut self, window: &mut WorkbenchWindow) {
        self.ext_host_shutting_down = true;
        let _ = save_workbench_state(window);
        if let Some(mut watcher) = self.watcher.take() {
            watcher.dispose();
        }
        if let Some(mut proc) = self.ext_host_process.take() {
            if let RuntimeHandle::Owned(ref rt) = self.runtime {
                let _ = rt.block_on(proc.shutdown());
            } else {
                drop(proc);
            }
        }
    }

    /// Gracefully shut down watcher and child processes from an async context.
    pub async fn shutdown_async(&mut self, window: &mut WorkbenchWindow) {
        self.ext_host_shutting_down = true;
        let _ = save_workbench_state(window);
        if let Some(mut watcher) = self.watcher.take() {
            watcher.dispose();
        }
        if let Some(mut proc) = self.ext_host_process.take() {
            let _ = proc.shutdown().await;
        }
    }
}

fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |ptr| RawWaker::new(ptr, &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );
    // SAFETY: No-op vtable with null data pointer never dereferences `ptr`.
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

fn push_warning_notification(window: &mut WorkbenchWindow, message: impl Into<String>) {
    let msg = message.into();
    if let Some(bridge) = window.language_bridge.clone() {
        let payload = serde_json::json!({ "message": msg });
        let is_pending = {
            let mut fut = Box::pin(
                bridge
                    .window_manager
                    .handle_rpc_call("window.showWarningMessage", &payload),
            );
            let waker = noop_waker();
            let mut cx = Context::from_waker(&waker);
            matches!(Pin::as_mut(&mut fut).poll(&mut cx), Poll::Pending)
        };
        if is_pending {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let wm = Arc::clone(&bridge.window_manager);
                handle.spawn(async move {
                    let _ = wm
                        .handle_rpc_call("window.showWarningMessage", &payload)
                        .await;
                });
            }
        }
        window.sync_window_manager();
    } else {
        let state = rustcode_ext_host::window::ExtNotificationState {
            id: format!("runtime_warn_{}", uuid_like_counter()),
            severity: NotificationSeverity::Warning,
            message: msg,
            detail: None,
            modal: false,
            items: Vec::new(),
            selected_action: None,
            dismissed: false,
            resolved: false,
        };
        let mut current = window.view.notifications.notifications();
        current.push(state);
        window.view.notifications.sync_notifications(&current);
    }
}

fn uuid_like_counter() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::parse_cli_args;
    use crate::cli::CliAction;
    use rustcode_platform::configuration::ConfigurationTarget;
    use serde_json::json;

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let unique = format!(
                "{}_{}_{}",
                prefix,
                std::process::id(),
                uuid_like_counter()
            );
            let path = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_disk_config_keybindings_themes_and_extension_discovery() {
        let tmp = TempDirGuard::new("rustcode_runtime_disk");
        let user_data = tmp.path.join("user-data");
        let custom_exts = tmp.path.join("custom-exts");
        let workspace = tmp.path.join("workspace");
        let ws_vscode = workspace.join(".vscode");
        let ws_exts = ws_vscode.join("extensions");
        std::fs::create_dir_all(&user_data).unwrap();
        std::fs::create_dir_all(custom_exts.join("theme-ext")).unwrap();
        std::fs::create_dir_all(ws_exts.join("ws-ext")).unwrap();

        // User settings.json (JSONC with comment & trailing comma)
        std::fs::write(
            user_data.join("settings.json"),
            r#"{
                // User preference
                "editor.fontSize": 18,
                "workbench.colorTheme": "Default Light+",
            }"#,
        )
        .unwrap();

        // Workspace .vscode/settings.json overrides user
        std::fs::write(
            ws_vscode.join("settings.json"),
            r#"{ "editor.tabSize": 2 }"#,
        )
        .unwrap();

        // User keybindings.json
        std::fs::write(
            user_data.join("keybindings.json"),
            r#"[{ "key": "ctrl+alt+u", "command": "demo.userCmd" }]"#,
        )
        .unwrap();

        // User extension with custom theme
        std::fs::write(
            custom_exts.join("theme-ext").join("package.json"),
            r#"{
                "name": "theme-ext",
                "publisher": "TestPub",
                "version": "1.0.0",
                "contributes": {
                    "themes": [
                        { "label": "Custom Dusk", "uiTheme": "vs-dark", "path": "./dusk.json" }
                    ]
                }
            }"#,
        )
        .unwrap();
        std::fs::write(
            custom_exts.join("theme-ext").join("dusk.json"),
            r##"{ "name": "Custom Dusk", "type": "dark", "colors": { "editor.background": "#121314" } }"##,
        )
        .unwrap();

        // Workspace extension
        std::fs::write(
            ws_exts.join("ws-ext").join("package.json"),
            r#"{ "name": "ws-ext", "publisher": "WsPub", "version": "0.1.0" }"#,
        )
        .unwrap();

        let opts = match parse_cli_args([
            "--user-data-dir",
            user_data.to_str().unwrap(),
            "--extensions-dir",
            custom_exts.to_str().unwrap(),
            workspace.to_str().unwrap(),
        ])
        .unwrap()
        {
            CliAction::Run(o) => o,
            other => panic!("unexpected cli action: {other:?}"),
        };

        let app = bootstrap_app(&opts, false).unwrap();
        assert_eq!(app.app_paths.user_data_dir, strip_windows_verbatim_prefix(user_data.clone()));
        assert_eq!(app.app_paths.extensions_dir, strip_windows_verbatim_prefix(custom_exts.clone()));
        assert_eq!(app.context.config.get::<u32>("editor.fontSize"), Some(18));
        assert_eq!(app.context.config.get::<usize>("editor.tabSize"), Some(2));
        assert_eq!(app.window.theme.current().kind, rustcode_platform::ThemeKind::Light);
        assert!(app.window.theme.get_theme("Custom Dusk").is_some());
        assert_eq!(
            app.context
                .keybindings
                .lookup_keybinding("demo.userCmd")
                .as_deref(),
            Some("Ctrl+Alt+U")
        );
        assert_eq!(app.context.ext_loader.loaded_extensions().len(), 2);

        // --disable-extensions skips scanning both user and workspace extension dirs
        let disabled_opts = match parse_cli_args([
            "--user-data-dir",
            user_data.to_str().unwrap(),
            "--extensions-dir",
            custom_exts.to_str().unwrap(),
            "--disable-extensions",
            workspace.to_str().unwrap(),
        ])
        .unwrap()
        {
            CliAction::Run(o) => o,
            other => panic!("unexpected cli action: {other:?}"),
        };
        let disabled_app = bootstrap_app(&disabled_opts, false).unwrap();
        assert!(disabled_app.context.ext_loader.loaded_extensions().is_empty());
    }

    #[test]
    fn test_cli_open_paths_and_goto_cursor_positioning() {
        let tmp = TempDirGuard::new("rustcode_runtime_goto");
        let file_path = tmp.path.join("sample.rs");
        std::fs::write(&file_path, "line one\nline two here\nline three\n").unwrap();

        let goto_spec = format!("{}:2:6", file_path.display());
        let opts = match parse_cli_args(["--headless", "--goto", &goto_spec]).unwrap() {
            CliAction::Run(o) => o,
            other => panic!("unexpected cli action: {other:?}"),
        };

        let mut app = bootstrap_app(&opts, false).unwrap();
        let active_editor = app
            .window
            .view
            .editor_area
            .active_editor()
            .expect("goto should open file tab");
        assert_eq!(active_editor.primary_position(), Position::new(1, 5));

        // Non-existent file opens gracefully as empty buffer without panicking
        let missing = tmp.path.join("does_not_exist.rs");
        let missing_opts = match parse_cli_args(["--headless", missing.to_str().unwrap()]).unwrap()
        {
            CliAction::Run(o) => o,
            other => panic!("unexpected cli action: {other:?}"),
        };
        let app_missing = bootstrap_app(&missing_opts, false).unwrap();
        assert_eq!(
            app_missing
                .window
                .view
                .editor_area
                .model
                .current_group_ref()
                .tabs
                .len(),
            1
        );
        app.runtime_loop.shutdown(&mut app.window);
    }

    #[test]
    fn test_workspace_state_persistence_and_restore_roundtrip() {
        let tmp = TempDirGuard::new("rustcode_runtime_persist");
        let ws = tmp.path.join("proj");
        std::fs::create_dir_all(&ws).unwrap();
        let f1 = ws.join("a.rs");
        let f2 = ws.join("b.rs");
        std::fs::write(&f1, "fn a() {\n    let x = 1;\n}\n").unwrap();
        std::fs::write(&f2, "fn b() {\n    let y = 2;\n}\n").unwrap();

        let storage: Arc<dyn StorageBackend> = Arc::new(InMemoryStorageBackend::new());
        let mut ctx = RustCodeContext::new();
        let _ = ctx.theme.register_builtin_themes(None);

        {
            let mut win = ctx.create_window().with_storage_backend(Arc::clone(&storage));
            win.open_workspace(&ws);
            win.open_path(&f1, false);
            win.open_path(&f2, false);
            if let Some(ed) = win.view.editor_area.active_editor_mut() {
                ed.set_primary_position(Position::new(1, 4));
                ed.scroll_top = 24.0;
            }
            win.view.layout.mutate(|l| {
                l.sidebar_visible = false;
                l.panel_visible = true;
                l.secondary_sidebar_visible = true;
                l.secondary_sidebar_width = 275.0;
                l.set_panel_position(rustcode_workbench::layout::PanelPosition::Right);
                l.panel_width = 340.0;
                l.title_bar_visible = true;
                l.zoom_in();
            });
            let _ = win.theme.set_theme_by_name("Default Light+");
            save_workbench_state(&win).unwrap();
        }

        // Restore into a fresh window backed by the same storage
        let mut restored_win = ctx.create_window().with_storage_backend(Arc::clone(&storage));
        restored_win.open_workspace(&ws);
        assert!(restore_workbench_state(&mut restored_win, Some(&ws)).unwrap());
        assert!(!restored_win.view.layout.sidebar_visible);
        assert!(restored_win.view.layout.panel_visible);
        assert!(restored_win.view.layout.secondary_sidebar_visible);
        assert_eq!(restored_win.view.layout.secondary_sidebar_width, 275.0);
        assert_eq!(
            restored_win.view.layout.panel_position,
            rustcode_workbench::layout::PanelPosition::Right
        );
        assert_eq!(restored_win.view.layout.panel_width, 340.0);
        assert!(restored_win.view.layout.title_bar_visible);
        assert_eq!(restored_win.view.layout.zoom_level, 1.0);
        assert_eq!(
            restored_win.theme.current().kind,
            rustcode_platform::ThemeKind::Light
        );
        assert_eq!(
            restored_win
                .view
                .editor_area
                .model
                .current_group_ref()
                .tabs
                .len(),
            2
        );
        let active = restored_win.view.editor_area.active_editor().unwrap();
        assert_eq!(active.primary_position(), Position::new(1, 4));
        assert_eq!(active.scroll_top, 24.0);
        assert_eq!(storage.recent_workspaces().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_runtime_loop_fs_watcher_and_auto_save() {
        let tmp = TempDirGuard::new("rustcode_runtime_loop");
        let ws = tmp.path.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let clean_file = ws.join("clean.rs");
        let dirty_file = ws.join("dirty.rs");
        std::fs::write(&clean_file, "v1\n").unwrap();
        std::fs::write(&dirty_file, "initial dirty\n").unwrap();

        let ctx = RustCodeContext::new();
        ctx.config
            .update("files.autoSave", json!("afterDelay"), ConfigurationTarget::Workspace)
            .unwrap();
        ctx.config
            .update("files.autoSaveDelay", json!(30), ConfigurationTarget::Workspace)
            .unwrap();

        let mut win = ctx.create_window();
        win.open_workspace(&ws);
        win.open_path(&clean_file, false);
        win.open_path(&dirty_file, false);

        let mut rt_loop = RuntimeLoop::new(RuntimeLoopConfig {
            disable_extensions: true,
            ..Default::default()
        })
        .unwrap();
        rt_loop.tick(&mut win);

        // Modify clean file on disk and wait briefly for OS watcher event
        std::fs::write(&clean_file, "v2 from disk\n").unwrap();
        // Also edit dirty_file in editor
        if let Some(ed) = win.view.editor_area.active_editor_mut() {
            ed.insert_text("// unsaved\n");
        }

        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            rt_loop.tick(&mut win);
            let clean_uri = win
                .view
                .editor_area
                .find_open_tab_uri_for_path(&clean_file)
                .unwrap();
            let clean_text = win
                .view
                .editor_area
                .any_editor_for_uri(&clean_uri)
                .unwrap()
                .text();
            let dirty_flushed = std::fs::read_to_string(&dirty_file)
                .is_ok_and(|s| s.contains("// unsaved"));
            if clean_text == "v2 from disk\n" && dirty_flushed {
                break;
            }
        }

        let clean_uri = win
            .view
            .editor_area
            .find_open_tab_uri_for_path(&clean_file)
            .unwrap();
        assert_eq!(
            win.view
                .editor_area
                .any_editor_for_uri(&clean_uri)
                .unwrap()
                .text(),
            "v2 from disk\n"
        );

        // Auto-save should have flushed dirty_file to disk after 30ms delay
        let disk_dirty = std::fs::read_to_string(&dirty_file).unwrap();
        assert!(
            disk_dirty.contains("// unsaved"),
            "expected auto-save to flush dirty edits to disk, got: {disk_dirty:?}"
        );

        // Now mark dirty_file dirty again without waiting for auto-save, modify externally -> conflict notification
        win.config
            .update("files.autoSave", json!("off"), ConfigurationTarget::Workspace)
            .unwrap();
        rt_loop.tick(&mut win);
        if let Some(ed) = win.view.editor_area.active_editor_mut() {
            ed.insert_text("// local unsaved");
        }
        std::fs::write(&dirty_file, "external conflict\n").unwrap();
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            rt_loop.tick(&mut win);
            if !win.view.notifications.is_empty() {
                break;
            }
        }
        assert!(
            !win.view.notifications.is_empty(),
            "expected dirty file disk modification to surface conflict notification"
        );
        let dirty_uri = win
            .view
            .editor_area
            .find_open_tab_uri_for_path(&dirty_file)
            .unwrap();
        assert!(
            win.view
                .editor_area
                .any_editor_for_uri(&dirty_uri)
                .unwrap()
                .text()
                .contains("// local unsaved"),
            "dirty buffer must be preserved on conflict"
        );

        rt_loop.shutdown_async(&mut win).await;
    }

    #[tokio::test]
    async fn test_runtime_loop_ext_host_auto_activation_and_crash_recovery() {
        let tmp = TempDirGuard::new("rustcode_runtime_ext_host");
        let ext_dir = tmp.path.join("lang-ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("package.json"),
            r#"{
                "name": "lang-ext",
                "publisher": "TestPub",
                "version": "1.0.0",
                "main": "./ext.js",
                "activationEvents": ["onLanguage:rust", "onCommand:langExt.ping"]
            }"#,
        )
        .unwrap();
        std::fs::write(
            ext_dir.join("ext.js"),
            r#"
const vscode = require('vscode');
exports.activate = (context) => {
    context.subscriptions.push(
        vscode.commands.registerCommand('langExt.ping', () => ({ pong: true }))
    );
    return { ready: true };
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
        let rs_file = tmp.path.join("demo.rs");
        std::fs::write(&rs_file, "fn main() {}\n").unwrap();

        let mut rt_loop = RuntimeLoop::new_with_shared_loader(
            RuntimeLoopConfig {
                disable_extensions: false,
                ext_host_restart_backoff: Duration::from_millis(50),
                ..Default::default()
            },
            Arc::clone(&shared_loader),
        )
        .unwrap();

        // Opening a .rs file triggers onLanguage:rust activation
        win.open_path(&rs_file, false);
        rt_loop.tick(&mut win);

        // Wait up to 3s for Node ext host spawn & activation (if node is available)
        if rt_loop.is_ext_host_running() {
            for _ in 0..60 {
                tokio::time::sleep(Duration::from_millis(50)).await;
                rt_loop.tick(&mut win);
                if shared_loader.read().is_activated("TestPub.lang-ext") {
                    break;
                }
            }
            assert!(shared_loader.read().is_activated("TestPub.lang-ext"));

            // Simulate crash by replacing process with a short-lived exited process or shutting it down unexpectedly
            if let Some(mut proc) = rt_loop.ext_host_process.take() {
                let _ = proc.shutdown().await;
                // Re-attach the already-stopped process so RuntimeLoop observes `!proc.is_running()`
                rt_loop.ext_host_process = Some(proc);
            }
            for _ in 0..60 {
                tokio::time::sleep(Duration::from_millis(50)).await;
                rt_loop.tick(&mut win);
                if rt_loop.ext_host_restart_count() >= 1
                    && rt_loop.is_ext_host_running()
                    && shared_loader.read().is_activated("TestPub.lang-ext")
                {
                    break;
                }
            }
            assert!(rt_loop.ext_host_restart_count() >= 1);
            assert!(shared_loader.read().is_activated("TestPub.lang-ext"));
        }

        rt_loop.shutdown_async(&mut win).await;
    }

    #[test]
    fn test_load_extension_themes_rejects_path_traversal() {
        let tmp = TempDirGuard::new("rustcode_theme_traversal");
        let ext_dir = tmp.path.join("ext");
        let themes_dir = ext_dir.join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(
            tmp.path.join("outside-theme.json"),
            r#"{ "name": "Escaped Theme", "type": "dark", "colors": {} }"#,
        )
        .unwrap();
        std::fs::write(
            themes_dir.join("inside-theme.json"),
            r#"{ "name": "Contained Theme", "type": "dark", "colors": {} }"#,
        )
        .unwrap();
        std::fs::write(
            ext_dir.join("package.json"),
            r#"{
                "name": "theme-traversal",
                "publisher": "TestPub",
                "version": "1.0.0",
                "contributes": {
                    "themes": [
                        { "label": "Escaped Label", "uiTheme": "vs-dark", "path": "../outside-theme.json" },
                        { "label": "Contained Label", "uiTheme": "vs-dark", "path": "./themes/inside-theme.json" }
                    ]
                }
            }"#,
        )
        .unwrap();

        let mut loader = ExtensionLoader::new();
        loader.load_from_dir(&ext_dir).unwrap();
        let mut themes = ThemeService::new();
        load_extension_themes(&mut themes, &loader);

        assert!(themes.get_theme("Escaped Theme").is_none());
        assert!(themes.get_theme("Escaped Label").is_none());
        assert!(themes.get_theme("Contained Theme").is_some());
        assert!(themes.get_theme("Contained Label").is_some());
        assert!(resolve_contained_extension_path(&ext_dir, "..\\outside-theme.json").is_none());
        assert!(resolve_contained_extension_path(&ext_dir, "themes\\..\\..\\outside-theme.json").is_none());
        assert!(resolve_contained_extension_path(&ext_dir, "C:\\outside-theme.json").is_none());
        assert!(resolve_contained_extension_path(&ext_dir, "/etc/passwd").is_none());
    }

    #[test]
    fn test_load_extension_snippets_and_language_configs_with_traversal_guard() {
        let tmp = TempDirGuard::new("rustcode_m2_ext_contribs");
        let ext_dir = tmp.path.join("ext");
        std::fs::create_dir_all(ext_dir.join("snippets")).unwrap();
        std::fs::write(
            tmp.path.join("escaped.json"),
            r#"{ "Bad": { "prefix": "bad", "body": "bad" } }"#,
        )
        .unwrap();
        std::fs::write(
            ext_dir.join("snippets/rust.json"),
            r#"{ "Print": { "prefix": "pr", "body": "println!(\"$1\");$0" } }"#,
        )
        .unwrap();
        std::fs::write(
            ext_dir.join("language-configuration.json"),
            r###"{ "comments": { "lineComment": "##" }, "surroundingPairs": [["(", ")"]] }"###,
        )
        .unwrap();
        std::fs::write(
            ext_dir.join("package.json"),
            r#"{
                "name": "m2-lang-ext",
                "publisher": "TestPub",
                "version": "1.0.0",
                "contributes": {
                    "languages": [{ "id": "customlang", "configuration": "./language-configuration.json" }],
                    "snippets": [
                        { "language": "customlang", "path": "./snippets/rust.json" },
                        { "language": "customlang", "path": "../escaped.json" }
                    ]
                }
            }"#,
        )
        .unwrap();

        let mut loader = ExtensionLoader::new();
        loader.load_from_dir(&ext_dir).unwrap();

        let mut snip_reg = rustcode_editor::SnippetRegistry::new();
        load_extension_snippets(&mut snip_reg, &loader);
        assert_eq!(snip_reg.snippets_for_language("customlang").len(), 1);
        assert_eq!(snip_reg.snippets_for_language("customlang")[0].prefixes, vec!["pr"]);

        let mut lc_reg = rustcode_editor::LanguageConfigurationRegistry::new();
        load_extension_language_configs(&mut lc_reg, &loader);
        let loaded_lc = lc_reg.get("customlang").unwrap();
        assert_eq!(loaded_lc.comments.line_comment.as_deref(), Some("##"));
    }
}
