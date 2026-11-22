//! GPUI WorkbenchWindow host managing rendering surfaces, theme updates, and input routing.

use crate::debug::{DebugToolbarAction, DebugViewAction};
use crate::editor_area::SplitDirection;
use crate::explorer::{ContributedViewDescriptor, ExplorerAction};
use crate::extensions_view::ExtensionsViewAction;
use crate::layout::LayoutHandle;
use crate::notifications::{NotificationUiAction, PromptUiAction};
use crate::output_panel::OutputAction;
use crate::quickinput::{CommandPalette, CommandPaletteView, PaletteItem};
use crate::scm::ScmAction;
use crate::search::{SearchAction, SearchInputFocus, WorkspaceReplaceResult};
use crate::terminal_panel::TerminalAction;
use crate::testing::TestingAction;
use crate::workbench_view::{FocusedRegion, WorkbenchAction, WorkbenchView};
use parking_lot::Mutex;
use rustcode_base::context_key::ContextKeyService;
use rustcode_base::disposable::DisposableStore;
use rustcode_editor::{
    CompletionItem, CompletionItemKind as EditorCompletionKind, EditorAction, LspHoverInfo,
    Position, Range, RangeEdit, TextDocument, WordWrapMode,
};
use rustcode_ext_host::manifest::{CustomEditorContribution, ViewContribution};
use rustcode_ext_host::marketplace::ExtensionInstaller;
use rustcode_ext_host::output::OutputChannelEvent;
use rustcode_ext_host::provider::{LanguageFeatureBridge, ProviderType};
use rustcode_ext_host::terminal::{ExtTerminalEvent, ExtTerminalLifecycleEvent};
use rustcode_ext_host::webview::{CustomEditorEvent, WebviewEvent, WebviewOptions};
use rustcode_platform::accessibility::AccessibilityTree;
use rustcode_platform::commands::CommandRegistry;
use rustcode_platform::configuration::ConfigurationService;
use rustcode_platform::fs::FileSystemService;
use rustcode_platform::keybindings::KeybindingService;
use rustcode_platform::os_ux::{
    ClipboardBackend, FileDialogBackend, FileDialogOptions, InMemoryClipboardBackend,
    MockFileDialogBackend,
};
use rustcode_platform::render::{
    EventResult, FontMetrics, InputEvent, Rect, RenderSurface, Scene, View,
    WorkbenchThemeStyles,
};
use rustcode_platform::storage::{InMemoryStorageBackend, StorageBackend};
use rustcode_platform::theme::ThemeService;
use rustcode_protocols::dap::{
    DapClient, DapError, DapMessage, InitializeRequestArguments, LaunchRequestKind,
    LaunchVariableContext, Scope, StackFrame, StoppedEventBody, Thread, Variable,
};
use rustcode_protocols::lsp::{
    CodeAction, CodeActionContext, CompletionItemKind as LspCompletionKind, FormattingOptions,
    Hover, LspCompletionItem, LspDiagnostic, LspLocation, LspPosition, LspRange, LspTextEdit,
    WorkspaceEdit,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct WindowCommandSink {
    pub layout: LayoutHandle,
    pub pending_commands: Mutex<Vec<String>>,
    pub pending_settings_open: Arc<AtomicBool>,
    pub suppress_command: Mutex<Option<String>>,
}

struct SuppressCommandGuard<'a> {
    slot: &'a Mutex<Option<String>>,
}

impl Drop for SuppressCommandGuard<'_> {
    fn drop(&mut self) {
        *self.slot.lock() = None;
    }
}

enum PendingLspResult {
    Hover {
        uri: String,
        position: Position,
        hover: Option<Hover>,
    },
    Definition {
        locations: Vec<LspLocation>,
    },
    Completions {
        uri: String,
        position: Position,
        trigger_char: Option<char>,
        items: Vec<LspCompletionItem>,
    },
    Formatting {
        uri: String,
        edits: Vec<LspTextEdit>,
        save_after: bool,
    },
}

struct PendingDapSnapshot {
    threads: Vec<Thread>,
    frames: Vec<StackFrame>,
    scopes: Vec<Scope>,
    variables: Vec<(u32, Vec<Variable>)>,
    stopped_thread_id: Option<i64>,
}

pub struct WorkbenchWindow {
    pub view: WorkbenchView,
    pub config: ConfigurationService,
    pub theme: ThemeService,
    pub keybindings: KeybindingService,
    pub commands: CommandRegistry,
    pub context_keys: ContextKeyService,
    pub font_metrics: FontMetrics,
    pub file_system: Arc<FileSystemService>,
    pub clipboard_backend: Arc<dyn ClipboardBackend>,
    pub file_dialog_backend: Arc<dyn FileDialogBackend>,
    pub storage_backend: Arc<dyn StorageBackend>,
    pub language_bridge: Option<Arc<LanguageFeatureBridge>>,
    pub dap_client: Option<Arc<DapClient>>,
    pub command_sink: Arc<WindowCommandSink>,
    pub pending_code_actions: Vec<CodeAction>,
    pub extension_disposables: HashMap<String, DisposableStore>,
    pub custom_editor_contributions: Vec<CustomEditorContribution>,
    pub registered_view_contributions: HashMap<String, Vec<ViewContribution>>,
    synced_output_revision: u64,
    synced_terminal_revision: u64,
    synced_webview_revision: u64,
    synced_custom_editor_revision: u64,
    synced_status_bar_revision: AtomicU64,
    render_status_bar_revision: AtomicU64,
    synced_window_revision: AtomicU64,
    render_window_revision: AtomicU64,
    synced_test_revision: AtomicU64,
    render_test_revision: AtomicU64,
    synced_tree_refresh_revision: AtomicU64,
    synced_ext_status_ids: HashSet<String>,
    tracked_documents: HashMap<String, (String, u64)>,
    tracked_active_uri: Option<String>,
    ext_terminal_map: HashMap<String, String>,
    synced_diagnostics: HashMap<String, Vec<LspDiagnostic>>,
    lsp_spawn_attempted: HashSet<(String, PathBuf)>,
    pending_lsp_results: Arc<Mutex<Vec<PendingLspResult>>>,
    pending_dap_events: Arc<Mutex<Vec<DapMessage>>>,
    pending_dap_snapshots: Arc<Mutex<Vec<PendingDapSnapshot>>>,
    last_hover_request: Option<(String, Position)>,
    synced_debug_console_len: usize,
    last_synced_color_theme: Option<String>,
}

impl Default for WorkbenchWindow {
    fn default() -> Self {
        Self::new(
            ThemeService::new(),
            KeybindingService::new(),
            CommandRegistry::new(),
            ContextKeyService::new(),
        )
    }
}

impl WorkbenchWindow {
    pub fn new(
        theme: ThemeService,
        keybindings: KeybindingService,
        commands: CommandRegistry,
        context_keys: ContextKeyService,
    ) -> Self {
        let font_metrics = FontMetrics::default_monospace();
        let view = WorkbenchView::new();
        let command_sink = Arc::new(WindowCommandSink {
            layout: view.layout.handle(),
            pending_commands: Mutex::new(Vec::new()),
            pending_settings_open: Arc::clone(&view.pending_settings_open),
            suppress_command: Mutex::new(None),
        });
        let config = ConfigurationService::new();
        let initial_theme = config.get::<String>("workbench.colorTheme");

        let mut win = Self {
            view,
            config,
            theme,
            keybindings,
            commands,
            context_keys,
            font_metrics,
            file_system: Arc::new(FileSystemService::new()),
            clipboard_backend: Arc::new(InMemoryClipboardBackend::new()),
            file_dialog_backend: Arc::new(MockFileDialogBackend::new()),
            storage_backend: Arc::new(InMemoryStorageBackend::new()),
            language_bridge: None,
            dap_client: None,
            command_sink,
            pending_code_actions: Vec::new(),
            extension_disposables: HashMap::new(),
            custom_editor_contributions: Vec::new(),
            registered_view_contributions: HashMap::new(),
            synced_output_revision: 0,
            synced_terminal_revision: 0,
            synced_webview_revision: 0,
            synced_custom_editor_revision: 0,
            synced_status_bar_revision: AtomicU64::new(0),
            render_status_bar_revision: AtomicU64::new(0),
            synced_window_revision: AtomicU64::new(0),
            render_window_revision: AtomicU64::new(0),
            synced_test_revision: AtomicU64::new(0),
            render_test_revision: AtomicU64::new(0),
            synced_tree_refresh_revision: AtomicU64::new(0),
            synced_ext_status_ids: HashSet::new(),
            tracked_documents: HashMap::new(),
            tracked_active_uri: None,
            ext_terminal_map: HashMap::new(),
            synced_diagnostics: HashMap::new(),
            lsp_spawn_attempted: HashSet::new(),
            pending_lsp_results: Arc::new(Mutex::new(Vec::new())),
            pending_dap_events: Arc::new(Mutex::new(Vec::new())),
            pending_dap_snapshots: Arc::new(Mutex::new(Vec::new())),
            last_hover_request: None,
            synced_debug_console_len: 0,
            last_synced_color_theme: initial_theme,
        };
        win.sync_configuration();
        win.sync_focus_context_keys();
        win
    }

    pub fn with_config(mut self, config: ConfigurationService) -> Self {
        self.config = config;
        self.sync_configuration();
        self
    }

    pub fn with_file_system(mut self, file_system: Arc<FileSystemService>) -> Self {
        self.file_system = file_system;
        self
    }

    pub fn with_clipboard_backend(mut self, backend: Arc<dyn ClipboardBackend>) -> Self {
        self.set_clipboard_backend(backend);
        self
    }

    pub fn set_clipboard_backend(&mut self, backend: Arc<dyn ClipboardBackend>) {
        self.clipboard_backend = backend;
        self.wire_clipboard_on_open_editors();
    }

    pub fn with_file_dialog_backend(mut self, backend: Arc<dyn FileDialogBackend>) -> Self {
        self.set_file_dialog_backend(backend);
        self
    }

    pub fn set_file_dialog_backend(&mut self, backend: Arc<dyn FileDialogBackend>) {
        self.file_dialog_backend = backend;
    }

    pub fn with_storage_backend(mut self, backend: Arc<dyn StorageBackend>) -> Self {
        self.set_storage_backend(backend);
        self
    }

    pub fn set_storage_backend(&mut self, backend: Arc<dyn StorageBackend>) {
        self.storage_backend = backend;
    }

    pub fn with_dap_client(mut self, client: Arc<DapClient>) -> Self {
        self.attach_dap_client(client);
        self
    }

    pub fn attach_dap_client(&mut self, client: Arc<DapClient>) {
        let queue = Arc::clone(&self.pending_dap_events);
        client.add_event_listener(move |msg| {
            queue.lock().push(msg.clone());
        });
        self.dap_client = Some(client);
    }

    pub fn command_sink(&self) -> Arc<WindowCommandSink> {
        Arc::clone(&self.command_sink)
    }

    pub fn set_config(&mut self, config: ConfigurationService) {
        self.config = config;
        self.sync_configuration();
    }

    pub fn accessibility_tree(&self, bounds: Rect) -> AccessibilityTree {
        self.sync_for_render();
        self.view.accessibility_tree(bounds, &self.font_metrics)
    }

    pub fn sync_configuration(&mut self) {
        self.view.cached_config = Some(self.config.clone());
        let configured_theme = self.config.get::<String>("workbench.colorTheme");
        if configured_theme != self.last_synced_color_theme {
            self.last_synced_color_theme = configured_theme;
            if matches!(self.theme.sync_with_configuration(&self.config), Ok(true)) {
                let active_theme = self.theme.current().clone();
                self.view.editor_area.for_each_editor_mut(|_, editor| {
                    editor.set_theme(active_theme.clone());
                });
            }
        }
        let default_minimap = self
            .config
            .get::<bool>("editor.minimap.enabled")
            .unwrap_or(true);
        let default_word_wrap = self
            .config
            .get::<String>("editor.wordWrap")
            .map(|s| WordWrapMode::from_config_str(&s))
            .unwrap_or(WordWrapMode::Off);
        let default_wrap_col = self
            .config
            .get::<usize>("editor.wordWrapColumn")
            .unwrap_or(80)
            .max(1);
        let default_tab_size = self
            .config
            .get::<usize>("editor.tabSize")
            .unwrap_or(4)
            .clamp(1, 16);
        let default_insert_spaces = self
            .config
            .get::<bool>("editor.insertSpaces")
            .unwrap_or(true);
        let active_path: Option<PathBuf> = self
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .and_then(|t| {
                if rustcode_platform::is_file_uri(&t.uri) || !t.uri.contains("://") {
                    Some(rustcode_platform::file_uri_to_path(&t.uri))
                } else {
                    None
                }
            });
        let breadcrumbs_enabled = self
            .config
            .get_for_resource::<bool>("breadcrumbs.enabled", active_path.as_deref())
            .unwrap_or(true);

        self.view.editor_area.set_minimap_enabled(default_minimap);
        self.view.editor_area.set_word_wrap(default_word_wrap);
        self.view.editor_area.set_word_wrap_column(default_wrap_col);
        let config = &self.config;
        self.view.editor_area.for_each_editor_mut(|_, editor| {
            let doc_path = editor
                .document
                .as_ref()
                .and_then(|d| d.read().path.clone());
            let enabled = config
                .get_for_resource::<bool>("editor.minimap.enabled", doc_path.as_deref())
                .unwrap_or(default_minimap);
            editor.set_minimap_enabled(enabled);
            let wrap_mode = config
                .get_for_resource::<String>("editor.wordWrap", doc_path.as_deref())
                .map(|s| WordWrapMode::from_config_str(&s))
                .unwrap_or(default_word_wrap);
            editor.set_word_wrap(wrap_mode);
            let wrap_col = config
                .get_for_resource::<usize>("editor.wordWrapColumn", doc_path.as_deref())
                .unwrap_or(default_wrap_col)
                .max(1);
            editor.set_word_wrap_column(wrap_col);
            let tab_size = config
                .get_for_resource::<usize>("editor.tabSize", doc_path.as_deref())
                .unwrap_or(default_tab_size)
                .clamp(1, 16);
            editor.set_tab_size(tab_size);
            let insert_spaces = config
                .get_for_resource::<bool>("editor.insertSpaces", doc_path.as_deref())
                .unwrap_or(default_insert_spaces);
            editor.set_insert_spaces(insert_spaces);
        });
        self.view
            .editor_area
            .set_breadcrumbs_enabled(breadcrumbs_enabled);
        self.view
            .editor_area
            .sync_breadcrumbs(Some(&self.view.workspace));

        #[cfg(windows)]
        let shell_setting_key = "terminal.integrated.shell.windows";
        #[cfg(not(windows))]
        let shell_setting_key = "terminal.integrated.shell.linux";

        self.view.terminal_panel.default_shell = self
            .config
            .get_for_resource::<String>(shell_setting_key, active_path.as_deref());
    }

    pub fn open_settings(&mut self) {
        self.view.open_settings(Some(self.config.clone()));
        self.sync_focus_context_keys();
        self.sync_configuration();
    }

    pub fn new_untitled_notebook(&mut self) {
        self.view.new_untitled_notebook();
        self.sync_focus_context_keys();
        self.sync_configuration();
    }

    fn sync_focus_context_keys(&mut self) {
        let panel_open = self.view.layout.panel_visible;
        match self.view.focused_region {
            FocusedRegion::Editor => {
                self.context_keys.set_bool("editorTextFocus", true);
                self.context_keys.set_bool("sideBarFocus", false);
                self.context_keys.set_bool("panelFocus", false);
                self.context_keys.set_bool("terminalFocus", false);
                self.context_keys.set_bool("inCommandPalette", false);
                self.context_keys.set_bool(
                    "notebookEditorFocused",
                    self.view.editor_area.active_notebook().is_some(),
                );
            }
            FocusedRegion::Sidebar => {
                self.context_keys.set_bool("editorTextFocus", false);
                self.context_keys.set_bool("sideBarFocus", true);
                self.context_keys.set_bool("panelFocus", false);
                self.context_keys.set_bool("terminalFocus", false);
                self.context_keys.set_bool("inCommandPalette", false);
                self.context_keys.set_bool("notebookEditorFocused", false);
            }
            FocusedRegion::Panel => {
                let in_terminal = panel_open && self.view.layout.active_panel_tab == "terminal";
                self.context_keys.set_bool("editorTextFocus", false);
                self.context_keys.set_bool("sideBarFocus", false);
                self.context_keys.set_bool("panelFocus", panel_open);
                self.context_keys.set_bool("terminalFocus", in_terminal);
                self.context_keys.set_bool("inCommandPalette", false);
                self.context_keys.set_bool("notebookEditorFocused", false);
            }
            FocusedRegion::CommandPalette => {
                self.context_keys.set_bool("editorTextFocus", false);
                self.context_keys.set_bool("sideBarFocus", false);
                self.context_keys.set_bool("panelFocus", false);
                self.context_keys.set_bool("terminalFocus", false);
                self.context_keys.set_bool("inCommandPalette", true);
                self.context_keys.set_bool("notebookEditorFocused", false);
            }
        }
    }

    pub fn sync_output_channels(&mut self) {
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let rev = bridge.output_manager.revision();
        if rev == self.synced_output_revision {
            return;
        }

        let events = bridge.output_manager.take_pending_events();
        if events.is_empty() {
            for ch in bridge.output_manager.list_channels() {
                self.view.output_panel.sync_channel_content(
                    &ch.id,
                    &ch.name,
                    &ch.content,
                    ch.is_visible,
                );
            }
        } else {
            for ev in events {
                match ev {
                    OutputChannelEvent::Created { id, name, .. } => {
                        self.view.output_panel.ensure_channel(&id, &name);
                    }
                    OutputChannelEvent::Appended { id, name, text } => {
                        self.view.output_panel.ensure_channel(&id, &name);
                        self.view.output_panel.append(&id, &text);
                    }
                    OutputChannelEvent::Replaced { id, name, text } => {
                        self.view.output_panel.ensure_channel(&id, &name);
                        self.view.output_panel.replace(&id, &text);
                    }
                    OutputChannelEvent::Cleared { id, .. } => {
                        self.view.output_panel.clear_channel(&id);
                    }
                    OutputChannelEvent::Shown {
                        id,
                        name,
                        preserve_focus,
                    } => {
                        let panel_ch = self.view.output_panel.ensure_channel(&id, &name);
                        panel_ch.visible = true;
                        self.view.output_panel.select_channel(&id);
                        self.view.layout.set_panel_tab("output");
                        if !preserve_focus {
                            self.view.focused_region = FocusedRegion::Panel;
                        }
                    }
                    OutputChannelEvent::Hidden { id, .. } => {
                        if let Some(ch) = self
                            .view
                            .output_panel
                            .channels
                            .iter_mut()
                            .find(|c| c.id == id)
                        {
                            ch.visible = false;
                        }
                        if self.view.layout.panel_visible
                            && self.view.layout.active_panel_tab == "output"
                        {
                            self.view.layout.mutate(|l| l.panel_visible = false);
                            if self.view.focused_region == FocusedRegion::Panel {
                                self.view.focused_region = FocusedRegion::Editor;
                            }
                        }
                    }
                    OutputChannelEvent::Disposed { id, .. } => {
                        self.view.output_panel.remove_channel(&id);
                    }
                }
            }
        }

        if self.view.output_panel.active_channel_id.is_none() {
            if let Some(active_id) = bridge.output_manager.active_channel_id() {
                self.view.output_panel.select_channel(&active_id);
            }
        }

        self.synced_output_revision = rev;
        self.sync_focus_context_keys();
    }

    pub fn sync_ext_terminals(&mut self) {
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let rev = bridge.terminal_manager.revision();
        if rev == self.synced_terminal_revision {
            return;
        }

        for ev in bridge.terminal_manager.take_pending_events() {
            match ev {
                ExtTerminalEvent::Created {
                    id,
                    name,
                    shell_path,
                    cwd,
                    ..
                } => {
                    let local_id = self.view.terminal_panel.create_terminal(
                        Some(&name),
                        shell_path.as_deref(),
                        cwd.map(PathBuf::from),
                    );
                    if let Some(ext_state) = bridge.terminal_manager.get_terminal(&id) {
                        if let (Some(rows), Some(cols)) = (ext_state.rows, ext_state.cols) {
                            if let Some(term) =
                                self.view.terminal_panel.manager.terminal_mut(&local_id)
                            {
                                let _ = term.resize(rows.max(1), cols.max(1));
                            }
                        }
                    }
                    self.ext_terminal_map.insert(id, local_id);
                }
                ExtTerminalEvent::Shown { id, preserve_focus } => {
                    if let Some(local_id) = self.ext_terminal_map.get(&id).cloned() {
                        self.view.terminal_panel.manager.set_active_terminal(&local_id);
                    }
                    self.view.layout.set_panel_tab("terminal");
                    if !preserve_focus {
                        self.view.terminal_panel.is_focused = true;
                        self.view.focused_region = FocusedRegion::Panel;
                    }
                }
                ExtTerminalEvent::TextSent {
                    id,
                    text,
                    add_new_line,
                } => {
                    if let Some(local_id) = self.ext_terminal_map.get(&id).cloned() {
                        let _ = self.view.terminal_panel.send_text_to_terminal(
                            &local_id,
                            &text,
                            add_new_line,
                        );
                    }
                }
                ExtTerminalEvent::Disposed { id } => {
                    if let Some(local_id) = self.ext_terminal_map.remove(&id) {
                        self.view.terminal_panel.manager.close_terminal(&local_id);
                    }
                }
            }
        }

        for lifecycle in bridge.terminal_manager.take_pending_lifecycle_events() {
            match lifecycle {
                ExtTerminalLifecycleEvent::Resized { id, rows, cols } => {
                    if let Some(local_id) = self.ext_terminal_map.get(&id).cloned() {
                        if let Some(term) = self.view.terminal_panel.manager.terminal_mut(&local_id)
                        {
                            let _ = term.resize(rows.max(1), cols.max(1));
                        }
                    }
                }
                ExtTerminalLifecycleEvent::Exited { id, exit_code } => {
                    if let Some(local_id) = self.ext_terminal_map.get(&id).cloned() {
                        if let Some(term) = self.view.terminal_panel.manager.terminal_mut(&local_id)
                        {
                            term.exited = true;
                            term.exit_code = exit_code;
                            term.last_exit_code = exit_code;
                        }
                    }
                }
            }
        }

        self.synced_terminal_revision = rev;
        self.sync_focus_context_keys();
    }

    pub fn register_custom_editor_contribution(&mut self, contrib: CustomEditorContribution) {
        self.custom_editor_contributions
            .retain(|c| c.view_type != contrib.view_type);
        self.custom_editor_contributions.push(contrib);
        self.view
            .editor_area
            .register_custom_editor_contributions(self.custom_editor_contributions.clone());
    }

    pub fn find_matching_custom_editor(&self, uri: &str) -> Option<CustomEditorContribution> {
        let mut candidates: Vec<CustomEditorContribution> = self
            .custom_editor_contributions
            .iter()
            .filter(|c| c.matches_path(uri))
            .cloned()
            .collect();

        if let Some(installer) = &self.view.extensions_view.installer {
            for ext in installer.list_installed() {
                let ext_id =
                    ExtensionInstaller::extension_id(&ext.manifest.publisher, &ext.manifest.name);
                if !installer.is_enabled(&ext_id) {
                    continue;
                }
                if let Some(contribs) = ext.manifest.contributes {
                    for ce in contribs.custom_editors {
                        if ce.matches_path(uri)
                            && !candidates.iter().any(|c| c.view_type == ce.view_type)
                        {
                            candidates.push(ce);
                        }
                    }
                }
            }
        }

        let bridge = self.language_bridge.as_ref();
        candidates.into_iter().min_by_key(|c| {
            let is_registered = bridge
                .and_then(|b| b.custom_editor_manager.get_provider(&c.view_type))
                .is_some();
            let priority_rank = match c.priority.as_deref() {
                Some("default") | None => 0u8,
                _ => 1u8,
            };
            (!is_registered, priority_rank)
        })
    }

    fn close_webview_tab_by_key(&mut self, key: &str) {
        let targets: Vec<(usize, usize)> = self
            .view
            .editor_area
            .model
            .groups
            .iter()
            .filter_map(|g| {
                g.tabs
                    .iter()
                    .position(|t| t.uri == key)
                    .map(|idx| (g.id, idx))
            })
            .collect();
        for (gid, idx) in targets {
            self.view.editor_area.close_tab_in_group(gid, idx);
        }
    }

    fn focus_webview_tab_by_key(&mut self, key: &str) {
        let target = self
            .view
            .editor_area
            .model
            .groups
            .iter()
            .find_map(|g| {
                g.tabs
                    .iter()
                    .position(|t| t.uri == key)
                    .map(|idx| (g.id, idx))
            });
        if let Some((gid, idx)) = target {
            self.view.editor_area.focus_group(gid);
            if let Some(group) = self.view.editor_area.model.group_by_id_mut(gid) {
                group.active_tab_index = Some(idx);
            }
            self.view.editor_area.sync_editor_focus();
            self.view.editor_area.sync_breadcrumbs(Some(&self.view.workspace));
            self.view.focused_region = FocusedRegion::Editor;
        }
    }

    fn sync_closed_webview_panels(&mut self) {
        let closed_ids = std::mem::take(&mut self.view.editor_area.closed_webview_panels);
        if closed_ids.is_empty() {
            return;
        }
        if let Some(bridge) = self.language_bridge.clone() {
            for panel_id in closed_ids {
                if let Some(session) = bridge
                    .custom_editor_manager
                    .find_session_by_panel_id(&panel_id)
                {
                    bridge
                        .custom_editor_manager
                        .close_session(&session.id, Some(&bridge.webview_manager));
                } else {
                    bridge.webview_manager.dispose_panel(&panel_id);
                }
            }
        }
    }

    pub fn sync_custom_editors(&mut self) {
        self.sync_closed_webview_panels();
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let rev = bridge.custom_editor_manager.revision();
        if rev == self.synced_custom_editor_revision {
            return;
        }

        let events = bridge.custom_editor_manager.take_pending_events();
        if events.is_empty() {
            for session in bridge.custom_editor_manager.list_active_sessions() {
                if !self
                    .view
                    .editor_area
                    .custom_editor_panels
                    .contains_key(&session.panel_id)
                {
                    let (html, options) = bridge
                        .webview_manager
                        .get_panel(&session.panel_id)
                        .map(|p| (p.html, p.options))
                        .unwrap_or_default();
                    self.view.editor_area.open_custom_editor(
                        &session.uri,
                        &session.title,
                        &session.view_type,
                        &session.panel_id,
                        &html,
                        options,
                        Some((*bridge.webview_manager).clone()),
                        false,
                    );
                }
            }
        } else {
            for ev in events {
                match ev {
                    CustomEditorEvent::SessionOpened {
                        id,
                        view_type,
                        uri,
                        panel_id,
                    } => {
                        if !self
                            .view
                            .editor_area
                            .custom_editor_panels
                            .contains_key(&panel_id)
                        {
                            let title = bridge
                                .custom_editor_manager
                                .get_session(&id)
                                .map(|s| s.title)
                                .unwrap_or_else(|| view_type.clone());
                            let (html, options) = bridge
                                .webview_manager
                                .get_panel(&panel_id)
                                .map(|p| (p.html, p.options))
                                .unwrap_or_default();
                            self.view.editor_area.open_custom_editor(
                                &uri,
                                &title,
                                &view_type,
                                &panel_id,
                                &html,
                                options,
                                Some((*bridge.webview_manager).clone()),
                                false,
                            );
                        }
                    }
                    CustomEditorEvent::SessionClosed { id } => {
                        if let Some(uri) = self
                            .view
                            .editor_area
                            .custom_editor_panels
                            .get(&id)
                            .cloned()
                        {
                            self.close_webview_tab_by_key(&uri);
                        }
                    }
                    CustomEditorEvent::ProviderRegistered { .. }
                    | CustomEditorEvent::ProviderUnregistered { .. } => {}
                }
            }
        }

        self.synced_custom_editor_revision = rev;
    }

    pub fn sync_webviews(&mut self) {
        self.sync_closed_webview_panels();
        self.sync_custom_editors();
        let Some(bridge) = self.language_bridge.clone() else {
            self.sync_webview_outbound_messages();
            return;
        };
        let rev = bridge.webview_manager.revision();
        if rev == self.synced_webview_revision {
            self.sync_webview_outbound_messages();
            return;
        }

        let resolve_key =
            |panels: &std::collections::HashMap<String, String>, panel_id: &str| -> String {
                panels
                    .get(panel_id)
                    .cloned()
                    .or_else(|| {
                        bridge
                            .custom_editor_manager
                            .find_session_by_panel_id(panel_id)
                            .map(|s| s.uri)
                    })
                    .unwrap_or_else(|| {
                        if panel_id.starts_with("webview://") {
                            panel_id.to_string()
                        } else {
                            format!("webview://{panel_id}")
                        }
                    })
            };

        let events = bridge.webview_manager.take_pending_events();
        if events.is_empty() {
            for panel in bridge.webview_manager.list_active_panels() {
                let key = resolve_key(&self.view.editor_area.custom_editor_panels, &panel.id);
                if self.view.editor_area.any_webview_for_uri(&key).is_some() {
                    if !panel.html.is_empty() {
                        self.view
                            .editor_area
                            .for_each_webview_for_uri_mut(&key, |wv| wv.set_html(&panel.html));
                    }
                } else if bridge.custom_editor_manager.get_session(&panel.id).is_none()
                    && bridge
                        .custom_editor_manager
                        .find_session_by_panel_id(&panel.id)
                        .is_none()
                {
                    self.view.editor_area.open_webview_with_key(
                        &key,
                        &panel.id,
                        &panel.title,
                        &panel.view_type,
                        &panel.html,
                        panel.options,
                        Some((*bridge.webview_manager).clone()),
                    );
                }
            }
        } else {
            for ev in events {
                match ev {
                    WebviewEvent::PanelCreated {
                        id,
                        view_type,
                        title,
                    } => {
                        let is_custom = self
                            .view
                            .editor_area
                            .custom_editor_panels
                            .contains_key(&id)
                            || bridge.custom_editor_manager.get_session(&id).is_some()
                            || bridge
                                .custom_editor_manager
                                .find_session_by_panel_id(&id)
                                .is_some();
                        let key = resolve_key(&self.view.editor_area.custom_editor_panels, &id);
                        if !is_custom && self.view.editor_area.any_webview_for_uri(&key).is_none() {
                            let (html, options) = bridge
                                .webview_manager
                                .get_panel(&id)
                                .map(|p| (p.html, p.options))
                                .unwrap_or_default();
                            self.view.editor_area.open_webview_with_key(
                                &key,
                                &id,
                                &title,
                                &view_type,
                                &html,
                                options,
                                Some((*bridge.webview_manager).clone()),
                            );
                        }
                    }
                    WebviewEvent::HtmlUpdated { id, html } => {
                        let key = resolve_key(&self.view.editor_area.custom_editor_panels, &id);
                        self.view
                            .editor_area
                            .for_each_webview_for_uri_mut(&key, |wv| wv.set_html(&html));
                    }
                    WebviewEvent::MessagePosted { id, message } => {
                        let key = resolve_key(&self.view.editor_area.custom_editor_panels, &id);
                        self.view
                            .editor_area
                            .for_each_webview_for_uri_mut(&key, |wv| {
                                wv.receive_message_from_host(message.clone());
                            });
                    }
                    WebviewEvent::PanelRevealed { id, .. } => {
                        let key = resolve_key(&self.view.editor_area.custom_editor_panels, &id);
                        self.focus_webview_tab_by_key(&key);
                    }
                    WebviewEvent::PanelDisposed { id } => {
                        let key = self
                            .view
                            .editor_area
                            .custom_editor_panels
                            .remove(&id)
                            .or_else(|| {
                                bridge
                                    .custom_editor_manager
                                    .find_session_by_panel_id(&id)
                                    .map(|s| s.uri)
                            })
                            .unwrap_or_else(|| {
                                if id.starts_with("webview://") {
                                    id.clone()
                                } else {
                                    format!("webview://{id}")
                                }
                            });
                        self.close_webview_tab_by_key(&key);
                        if let Some(session) =
                            bridge.custom_editor_manager.find_session_by_panel_id(&id)
                        {
                            bridge
                                .custom_editor_manager
                                .close_session(&session.id, None);
                        }
                    }
                }
            }
        }

        self.synced_webview_revision = rev;
        self.sync_webview_outbound_messages();
    }

    pub async fn resolve_custom_editor(
        &mut self,
        uri: &str,
        content: Option<&str>,
    ) -> Result<String, String> {
        let bridge = self
            .language_bridge
            .clone()
            .ok_or_else(|| "LanguageFeatureBridge not attached".to_string())?;

        let (view_type, title) = if let Some(wv) = self.view.editor_area.any_webview_for_uri(uri) {
            (wv.view_type.clone(), wv.title.clone())
        } else if let Some(contrib) = self.find_matching_custom_editor(uri) {
            let t = uri
                .rsplit(['/', '\\'])
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(&contrib.view_type)
                .to_string();
            (contrib.view_type.clone(), t)
        } else {
            return Err(format!("No custom editor contribution matches '{}'", uri));
        };

        let existing_session = bridge
            .custom_editor_manager
            .find_session_by_uri(uri, Some(&view_type))
            .filter(|session| {
                let valid = bridge
                    .webview_manager
                    .get_panel(&session.panel_id)
                    .is_some_and(|p| !p.is_disposed);
                if !valid {
                    bridge
                        .custom_editor_manager
                        .close_session(&session.id, Some(&bridge.webview_manager));
                }
                valid
            });
        let session = existing_session.unwrap_or_else(|| {
            bridge.custom_editor_manager.open_session(
                None,
                &view_type,
                uri,
                Some(&title),
                Some(&bridge.webview_manager),
            )
        });

        let _ = bridge
            .request_open_custom_document(&view_type, uri, None)
            .await;
        bridge
            .request_resolve_custom_editor(&view_type, uri, &session.panel_id, &title, content)
            .await?;

        self.sync_custom_editors();
        self.sync_webviews();
        Ok(session.panel_id)
    }

    fn project_ext_status_items(bridge: &LanguageFeatureBridge) -> Vec<crate::statusbar::StatusBarItem> {
        bridge
            .status_bar_manager
            .get_visible_items()
            .into_iter()
            .filter(|i| i.id != "workbench.status.problems")
            .map(|item| crate::statusbar::StatusBarItem {
                id: item.id,
                text: item.text,
                tooltip: item.tooltip,
                command: item.command,
                alignment: if item.alignment == 2 {
                    crate::statusbar::StatusBarAlignment::Right
                } else {
                    crate::statusbar::StatusBarAlignment::Left
                },
                priority: item.priority,
            })
            .collect()
    }

    fn sync_window_overlays(&self, bridge: &LanguageFeatureBridge) -> u64 {
        let active_notifs = bridge.window_manager.active_notifications();
        self.view.notifications.sync_notifications(&active_notifs);
        let active_prompts = bridge.window_manager.active_prompts();
        self.view.prompt_overlay.sync_prompts(&active_prompts);
        bridge.window_manager.revision()
    }

    fn sync_testing_view_from_bridge(&self, bridge: &LanguageFeatureBridge) -> (u64, usize) {
        let rev = bridge.test_manager.revision();
        let failed = self.view.testing_view.sync_from_manager(&bridge.test_manager);
        (rev, failed)
    }

    pub fn sync_status_bar(&mut self) {
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let rev = bridge.status_bar_manager.revision();
        if rev == self.synced_status_bar_revision.load(Ordering::SeqCst) {
            return;
        }
        let projected = Self::project_ext_status_items(&bridge);
        let active_ids: HashSet<String> = projected.iter().map(|i| i.id.clone()).collect();
        for old_id in self.synced_ext_status_ids.drain() {
            if !active_ids.contains(&old_id) && old_id != "workbench.status.problems" {
                self.view.status_bar.model.remove_item(&old_id);
            }
        }
        for item in projected {
            let id = item.id.clone();
            self.view.status_bar.model.set_item(item);
            self.synced_ext_status_ids.insert(id);
        }
        *self.view.pending_ext_status_bar.write() = None;
        self.synced_status_bar_revision.store(rev, Ordering::SeqCst);
        self.render_status_bar_revision.store(rev, Ordering::SeqCst);
    }

    pub fn sync_window_manager(&mut self) {
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let requests = bridge.take_pending_show_document_requests();
        for req in requests {
            let path = rustcode_platform::file_uri_to_path(&req.uri);
            let title = path
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("untitled")
                .to_string();
            let existing_text = self
                .view
                .editor_area
                .any_editor_for_uri(&req.uri)
                .map(|ed| ed.text());
            let content = existing_text
                .unwrap_or_else(|| std::fs::read_to_string(&path).unwrap_or_default());
            let prev_focus = self.view.focused_region;
            self.open_file(&req.uri, &title, &content, req.preview.unwrap_or(false));
            if let Some(sel) = req.selection {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.set_primary_position(rustcode_editor::Position::new(
                        sel.start.line as usize,
                        sel.start.character as usize,
                    ));
                }
            }
            if req.preserve_focus {
                self.view.focused_region = prev_focus;
                self.sync_focus_context_keys();
            }
        }
        let rev = self.sync_window_overlays(&bridge);
        self.synced_window_revision.store(rev, Ordering::SeqCst);
        self.render_window_revision.store(rev, Ordering::SeqCst);
    }

    pub fn sync_test_manager(&mut self) {
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let (rev, failed) = self.sync_testing_view_from_bridge(&bridge);
        let badge = if failed > 0 {
            Some(failed as u32)
        } else {
            None
        };
        self.view
            .activity_bar
            .set_badge("workbench.view.testing", badge);
        if rev == self.synced_test_revision.load(Ordering::SeqCst) {
            return;
        }
        self.synced_test_revision.store(rev, Ordering::SeqCst);
        self.render_test_revision.store(rev, Ordering::SeqCst);
    }

    pub fn register_view_contributions(
        &mut self,
        container_id: &str,
        views: Vec<ViewContribution>,
    ) {
        let entry = self
            .registered_view_contributions
            .entry(container_id.to_string())
            .or_default();
        for v in views {
            if let Some(existing) = entry.iter_mut().find(|e| e.id == v.id) {
                *existing = v;
            } else {
                entry.push(v);
            }
        }
        self.sync_contributed_views_state();
    }

    fn collect_explorer_view_descriptors(&self) -> Vec<ContributedViewDescriptor> {
        let mut out: Vec<ContributedViewDescriptor> = Vec::new();
        let mut seen = HashSet::new();

        if let Some(views) = self.registered_view_contributions.get("explorer") {
            for v in views {
                if seen.insert(v.id.clone()) {
                    out.push(ContributedViewDescriptor::from(v));
                }
            }
        }

        if let Some(bridge) = &self.language_bridge {
            for snap in bridge.env_manager.list_extensions() {
                let Some(arr) = snap
                    .package_json
                    .get("contributes")
                    .and_then(|c| c.get("views"))
                    .and_then(|v| v.get("explorer"))
                    .and_then(|a| a.as_array())
                else {
                    continue;
                };
                for item in arr {
                    let Some(id) = item
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    else {
                        continue;
                    };
                    if !seen.insert(id.to_string()) {
                        continue;
                    }
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or(id)
                        .to_string();
                    let when = item
                        .get("when")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string);
                    out.push(ContributedViewDescriptor {
                        id: id.to_string(),
                        name,
                        when,
                    });
                }
            }
        }

        out
    }

    fn spawn_tree_children_fetch(
        bridge: Arc<LanguageFeatureBridge>,
        contributed_trees: crate::explorer::SharedContributedTrees,
        view_id: String,
        element_id: Option<String>,
        handle: &tokio::runtime::Handle,
    ) {
        handle.spawn(async move {
            let mut queue: Vec<Option<String>> = vec![element_id];
            while let Some(elem) = queue.pop() {
                let res = bridge
                    .request_tree_children_result(&view_id, elem.as_deref())
                    .await;
                let mut guard = contributed_trees.write();
                let Some(view) = guard.views.get_mut(&view_id) else {
                    break;
                };
                match (elem, res) {
                    (None, Ok(items)) => {
                        let follow_ups = view.set_root_items(items);
                        for (child_id, _rpc_id) in follow_ups {
                            view.loading_parents.insert(child_id.clone());
                            queue.push(Some(child_id));
                        }
                    }
                    (None, Err(_)) => {
                        view.loading_root = false;
                        view.root_loaded = true;
                    }
                    (Some(parent_id), Ok(items)) => {
                        let follow_ups = view.set_child_items(&parent_id, items);
                        for (child_id, _rpc_id) in follow_ups {
                            view.loading_parents.insert(child_id.clone());
                            queue.push(Some(child_id));
                        }
                    }
                    (Some(parent_id), Err(_)) => {
                        view.loading_parents.remove(&parent_id);
                        view.loaded_parents.insert(parent_id);
                    }
                }
            }
        });
    }

    fn fetch_contributed_tree_children(&self, view_id: &str, element_id: Option<String>) {
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        {
            let mut guard = self.view.explorer.contributed_trees.write();
            let Some(view) = guard.views.get_mut(view_id) else {
                return;
            };
            if let Some(eid) = &element_id {
                if view.loaded_parents.contains(eid) || view.loading_parents.contains(eid) {
                    return;
                }
                view.loading_parents.insert(eid.clone());
            } else {
                if view.root_loaded || view.loading_root {
                    return;
                }
                view.loading_root = true;
            }
        }
        Self::spawn_tree_children_fetch(
            bridge,
            Arc::clone(&self.view.explorer.contributed_trees),
            view_id.to_string(),
            element_id,
            &handle,
        );
    }

    fn sync_contributed_views_state(&self) {
        let descriptors = self.collect_explorer_view_descriptors();
        let bridge_opt = self.language_bridge.clone();

        {
            let mut guard = self.view.explorer.contributed_trees.write();
            guard.sync_descriptors(descriptors, &self.context_keys, |view_id| {
                bridge_opt.as_ref().is_some_and(|b| {
                    b.provider_registry
                        .has_provider_with_id(ProviderType::TreeData, view_id)
                })
            });
        }

        let Some(bridge) = bridge_opt else {
            return;
        };

        let rev = bridge.provider_registry.tree_refresh_revision();
        if rev != self.synced_tree_refresh_revision.load(Ordering::SeqCst) {
            let events = bridge.provider_registry.take_pending_tree_refreshes();
            {
                let mut guard = self.view.explorer.contributed_trees.write();
                for ev in events {
                    if let Some(view) = guard.views.get_mut(&ev.provider_id) {
                        view.invalidate_elements(&ev.element_ids);
                    }
                }
            }
            self.synced_tree_refresh_revision
                .store(rev, Ordering::SeqCst);
        }

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };

        let pending_fetches: Vec<(String, Option<String>)> = {
            let mut guard = self.view.explorer.contributed_trees.write();
            let mut fetches = Vec::new();
            let ids = guard.order.clone();
            for vid in ids {
                if let Some(view) = guard.views.get_mut(&vid) {
                    if !view.visible || !view.has_provider {
                        continue;
                    }
                    if !view.root_loaded && !view.loading_root {
                        view.loading_root = true;
                        fetches.push((vid.clone(), None));
                    }
                    let expanded_list: Vec<String> = view.expanded_ids.iter().cloned().collect();
                    for eid in expanded_list {
                        if !view.loaded_parents.contains(&eid)
                            && !view.loading_parents.contains(&eid)
                        {
                            view.loading_parents.insert(eid.clone());
                            fetches.push((vid.clone(), Some(eid)));
                        }
                    }
                }
            }
            fetches
        };

        for (view_id, element_id) in pending_fetches {
            Self::spawn_tree_children_fetch(
                Arc::clone(&bridge),
                Arc::clone(&self.view.explorer.contributed_trees),
                view_id,
                element_id,
                &handle,
            );
        }
    }

    pub fn sync_tree_views(&mut self) {
        self.sync_contributed_views_state();
    }

    pub fn sync_document_lifecycle(&mut self) {
        let mut current: HashMap<String, (String, u64, String)> = HashMap::new();
        for (uri, editor) in &self.view.editor_area.editors {
            let (lang_id, version_id, text) = if let Some(doc) = &editor.document {
                let d = doc.read();
                (d.language_id.clone(), d.version_id, d.text())
            } else {
                ("plaintext".to_string(), 1, editor.text())
            };
            current.insert(uri.clone(), (lang_id, version_id, text));
        }

        let closed: Vec<(String, String)> = self
            .tracked_documents
            .iter()
            .filter(|(uri, _)| !current.contains_key(*uri))
            .map(|(uri, (lang, _))| (uri.clone(), lang.clone()))
            .collect();
        for (uri, lang) in closed {
            self.tracked_documents.remove(&uri);
            if let Some(bridge) = &self.language_bridge {
                bridge.did_close_document_sync(&uri, &lang);
            }
        }

        for (uri, (lang_id, version_id, text)) in current {
            match self.tracked_documents.get(&uri).cloned() {
                None => {
                    self.tracked_documents
                        .insert(uri.clone(), (lang_id.clone(), version_id));
                    if let Some(bridge) = &self.language_bridge {
                        bridge.did_open_document_sync(&uri, &lang_id, version_id as i32, &text);
                    }
                }
                Some((_, prev_ver)) if prev_ver != version_id => {
                    self.tracked_documents
                        .insert(uri.clone(), (lang_id.clone(), version_id));
                    if let Some(bridge) = &self.language_bridge {
                        bridge.did_change_document_sync(&uri, &lang_id, version_id as i32, &text);
                    }
                }
                _ => {}
            }
        }

        let active_uri = self
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .map(|t| t.uri.clone())
            .filter(|u| self.view.editor_area.any_editor_for_uri(u).is_some());

        if active_uri != self.tracked_active_uri {
            self.tracked_active_uri = active_uri.clone();
            if let Some(bridge) = &self.language_bridge {
                bridge.did_change_active_editor(active_uri.as_deref());
            }
        }
    }

    fn sync_for_render(&self) {
        let Some(bridge) = self.language_bridge.as_ref() else {
            return;
        };
        let sb_rev = bridge.status_bar_manager.revision();
        if sb_rev != self.synced_status_bar_revision.load(Ordering::SeqCst)
            && sb_rev != self.render_status_bar_revision.load(Ordering::SeqCst)
        {
            let items = Self::project_ext_status_items(bridge);
            *self.view.pending_ext_status_bar.write() = Some(items);
            self.render_status_bar_revision
                .store(sb_rev, Ordering::SeqCst);
        }
        let win_rev = bridge.window_manager.revision();
        if win_rev != self.synced_window_revision.load(Ordering::SeqCst)
            && win_rev != self.render_window_revision.load(Ordering::SeqCst)
        {
            let synced_rev = self.sync_window_overlays(bridge);
            self.render_window_revision
                .store(synced_rev, Ordering::SeqCst);
        }
        let test_rev = bridge.test_manager.revision();
        if test_rev != self.synced_test_revision.load(Ordering::SeqCst)
            && test_rev != self.render_test_revision.load(Ordering::SeqCst)
        {
            let (synced_rev, _) = self.sync_testing_view_from_bridge(bridge);
            self.render_test_revision
                .store(synced_rev, Ordering::SeqCst);
        }
        self.sync_contributed_views_state();
    }

    fn reset_bridge_sync_state(&mut self) {
        self.synced_output_revision = 0;
        self.synced_terminal_revision = 0;
        self.synced_webview_revision = 0;
        self.synced_custom_editor_revision = 0;
        self.synced_status_bar_revision.store(0, Ordering::SeqCst);
        self.render_status_bar_revision.store(0, Ordering::SeqCst);
        self.synced_window_revision.store(0, Ordering::SeqCst);
        self.render_window_revision.store(0, Ordering::SeqCst);
        self.synced_test_revision.store(0, Ordering::SeqCst);
        self.render_test_revision.store(0, Ordering::SeqCst);
        self.synced_tree_refresh_revision.store(0, Ordering::SeqCst);
        {
            let mut guard = self.view.explorer.contributed_trees.write();
            guard.order.clear();
            guard.views.clear();
        }
        self.tracked_documents.clear();
        self.tracked_active_uri = None;
        self.synced_diagnostics.clear();
        self.lsp_spawn_attempted.clear();
    }

    fn sync_bridge_state(&mut self) {
        self.sync_output_channels();
        self.sync_ext_terminals();
        self.sync_custom_editors();
        self.sync_webviews();
        self.sync_status_bar();
        self.sync_window_manager();
        self.sync_test_manager();
        self.sync_tree_views();
        self.sync_document_lifecycle();
        self.sync_diagnostics();
    }

    pub fn sync_all(&mut self) {
        self.drain_pending_command_effects();
        self.drain_pending_lsp_results();
        self.drain_pending_dap_events();
        self.sync_bridge_state();
        self.sync_focus_context_keys();
        self.sync_configuration();
        self.wire_clipboard_on_open_editors();
    }

    pub fn poll_runtime_events(&mut self) {
        self.poll_terminal_ptys();
        self.sync_webview_outbound_messages();
        self.sync_all();
    }

    pub fn poll_terminal_ptys(&mut self) -> Vec<String> {
        let updated = self.view.terminal_panel.poll_all_ptys();
        if let Some(bridge) = self.language_bridge.clone() {
            let pairs: Vec<(String, String)> = self
                .ext_terminal_map
                .iter()
                .map(|(ext_id, local_id)| (ext_id.clone(), local_id.clone()))
                .collect();
            let mut changed_ext = false;
            for (ext_id, local_id) in pairs {
                let Some(term) = self.view.terminal_panel.manager.terminal(&local_id) else {
                    continue;
                };
                if let Some(ext_state) = bridge.terminal_manager.get_terminal(&ext_id) {
                    if (ext_state.rows != Some(term.rows) || ext_state.cols != Some(term.cols))
                        && bridge
                            .terminal_manager
                            .resize_terminal(&ext_id, term.rows, term.cols)
                            .is_ok()
                    {
                        changed_ext = true;
                    }
                    if term.exited
                        && !ext_state.has_exited
                        && bridge
                            .terminal_manager
                            .notify_terminal_exit(&ext_id, term.exit_code)
                            .is_ok()
                    {
                        changed_ext = true;
                    }
                }
            }
            if changed_ext {
                let _ = bridge.terminal_manager.take_pending_lifecycle_events();
                self.synced_terminal_revision = bridge.terminal_manager.revision();
            }
        }
        updated
    }

    pub fn sync_webview_outbound_messages(&mut self) {
        let bridge = self.language_bridge.clone();
        for wv in self.view.editor_area.webviews.values_mut() {
            let msgs = wv.drain_messages_to_host();
            if wv.ext_host_manager.is_none() && !msgs.is_empty() {
                if let Some(b) = &bridge {
                    for msg in msgs {
                        let _ = b.webview_manager.receive_message_from_webview(&wv.id, msg);
                    }
                }
            }
        }
        for wv in self.view.editor_area.split_webviews.values_mut() {
            let msgs = wv.drain_messages_to_host();
            if wv.ext_host_manager.is_none() && !msgs.is_empty() {
                if let Some(b) = &bridge {
                    for msg in msgs {
                        let _ = b.webview_manager.receive_message_from_webview(&wv.id, msg);
                    }
                }
            }
        }
    }

    pub fn sync_diagnostics(&mut self) {
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let all_diags = bridge.get_all_diagnostics();
        let prev_uris: Vec<String> = self.synced_diagnostics.keys().cloned().collect();
        for uri in prev_uris {
            if !all_diags.contains_key(&uri) {
                self.view.update_lsp_problems(&uri, &[]);
            }
        }
        for (uri, diags) in &all_diags {
            if self.synced_diagnostics.get(uri) != Some(diags) {
                self.view.update_lsp_problems(uri, diags);
            }
        }
        self.synced_diagnostics = all_diags;
    }

    fn wire_clipboard_on_open_editors(&mut self) {
        let read_cb = Arc::clone(&self.clipboard_backend);
        let write_cb = Arc::clone(&self.clipboard_backend);
        let read_fn: rustcode_editor::ClipboardReadFn =
            Arc::new(move || read_cb.read_text().ok());
        let write_fn: rustcode_editor::ClipboardWriteFn = Arc::new(move |text: &str| {
            let _ = write_cb.write_text(text);
        });
        self.view.editor_area.for_each_editor_mut(|_, editor| {
            editor.set_clipboard_callbacks(Arc::clone(&read_fn), Arc::clone(&write_fn));
        });
    }

    fn workspace_root_for_uri(&self, uri: Option<&str>) -> PathBuf {
        if let Some(u) = uri {
            let p = rustcode_platform::file_uri_to_path(u);
            if let Some(folder) = self.view.workspace.get_folder_for_path(&p) {
                return folder.path.clone();
            }
            if let Some(parent) = p.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                return parent.to_path_buf();
            }
        }
        self.view
            .workspace
            .folders()
            .first()
            .map(|f| f.path.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    fn ensure_lsp_for_language_and_open(&mut self, uri: &str, lang_id: &str) {
        if lang_id.is_empty() || lang_id == "plaintext" || lang_id == "*" {
            return;
        }
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        if bridge.lsp_manager.get_config(lang_id).is_none() {
            return;
        }
        if bridge.lsp_manager.get_client(lang_id).is_some()
            || bridge.lsp_manager.is_language_failed(lang_id)
        {
            return;
        }
        let ws_root = self.workspace_root_for_uri(Some(uri));
        let key = (lang_id.to_string(), ws_root.clone());
        if !self.lsp_spawn_attempted.insert(key) {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let docs_for_lang: Vec<(String, String, i32, String)> = self
            .view
            .editor_area
            .editors
            .iter()
            .filter_map(|(u, ed)| {
                let (d_lang, d_ver, d_text) = if let Some(doc) = &ed.document {
                    let d = doc.read();
                    (d.language_id.clone(), d.version_id as i32, d.text())
                } else {
                    ("plaintext".to_string(), 1, ed.text())
                };
                if d_lang == lang_id {
                    Some((u.clone(), d_lang, d_ver, d_text))
                } else {
                    None
                }
            })
            .collect();
        let lang = lang_id.to_string();
        handle.spawn(async move {
            if let Ok(Some(client)) = bridge
                .lsp_manager
                .ensure_server_for_language(&lang, &ws_root)
                .await
            {
                if client.is_running() {
                    for (doc_uri, doc_lang, doc_ver, doc_text) in docs_for_lang {
                        let _ = client.did_open(&doc_uri, &doc_lang, doc_ver, &doc_text).await;
                    }
                }
            }
        });
    }

    fn ensure_lsp_for_open_documents(&mut self) {
        let docs: Vec<(String, String)> = self
            .view
            .editor_area
            .editors
            .iter()
            .filter_map(|(uri, ed)| {
                ed.document
                    .as_ref()
                    .map(|d| (uri.clone(), d.read().language_id.clone()))
            })
            .collect();
        for (uri, lang_id) in docs {
            self.ensure_lsp_for_language_and_open(&uri, &lang_id);
        }
    }

    pub fn open_workspace(&mut self, root: &Path) {
        let root_buf = rustcode_platform::strip_windows_verbatim_prefix(root.to_path_buf());
        self.view.workspace = crate::workspace::Workspace::from_folder(root_buf.clone());
        self.view
            .explorer
            .model
            .sync_from_workspace(&self.view.workspace);
        self.view
            .explorer
            .model
            .expanded_paths
            .insert(root_buf.clone());
        self.view.terminal_panel.default_cwd = Some(root_buf.clone());
        if let Some(bridge) = &self.language_bridge {
            bridge.set_workspace_folders(vec![rustcode_ext_host::ExtWorkspaceFolder::from_path(
                0,
                &root_buf,
            )]);
        }
        let _ = self.view.open_workspace_git(&root_buf);
        let _ = self.view.debug_sidebar.load_launch_json(&root_buf);
        let _ = self.storage_backend.add_recent_workspace(&root_buf);
        self.sync_configuration();
        self.sync_focus_context_keys();
    }

    pub fn open_path(&mut self, path: &Path, is_preview: bool) -> bool {
        let cleaned = rustcode_platform::strip_windows_verbatim_prefix(path.to_path_buf());
        let uri = self
            .view
            .editor_area
            .find_open_tab_uri_for_path(&cleaned)
            .unwrap_or_else(|| rustcode_platform::path_to_file_uri(&cleaned));
        let title = cleaned
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("untitled")
            .to_string();

        if self.view.editor_area.any_editor_for_uri(&uri).is_some() {
            self.open_file(&uri, &title, "", is_preview);
            return true;
        }

        match TextDocument::open_sync(&self.file_system, &cleaned) {
            Ok(decoded_doc) => {
                let content = decoded_doc.text();
                let encoding = decoded_doc.encoding();
                let meta = decoded_doc.metadata.clone();
                self.open_file(&uri, &title, &content, is_preview);
                self.view.editor_area.for_each_editor_for_uri_mut(&uri, |ed| {
                    if let Some(doc) = &ed.document {
                        let mut d = doc.write();
                        d.encoding = encoding;
                        if let Some(m) = meta.clone() {
                            d.metadata = Some(m);
                            d.is_dirty = false;
                            d.saved_version_id = d.version_id;
                        }
                    }
                });
                true
            }
            Err(_) => {
                let content = std::fs::read_to_string(&cleaned).unwrap_or_default();
                self.open_file(&uri, &title, &content, is_preview);
                cleaned.exists()
            }
        }
    }

    pub fn open_file_dialog(&mut self) -> bool {
        let opts = FileDialogOptions {
            title: Some("Open File".to_string()),
            default_directory: self
                .view
                .workspace
                .folders()
                .first()
                .map(|f| f.path.clone()),
            ..Default::default()
        };
        if let Some(path) = self.file_dialog_backend.pick_open_file(&opts) {
            self.open_path(&path, false);
            true
        } else {
            false
        }
    }

    pub fn open_folder_dialog(&mut self) -> bool {
        let opts = FileDialogOptions {
            title: Some("Open Folder".to_string()),
            default_directory: self
                .view
                .workspace
                .folders()
                .first()
                .map(|f| f.path.clone()),
            ..Default::default()
        };
        if let Some(path) = self.file_dialog_backend.pick_open_folder(&opts) {
            self.open_workspace(&path);
            true
        } else {
            false
        }
    }

    pub fn save_active_file_as(&mut self) -> bool {
        let Some(active_tab) = self
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .cloned()
        else {
            return false;
        };
        let active_uri = active_tab.uri.clone();
        let Some(editor) = self.view.editor_area.active_editor() else {
            return false;
        };
        let content = editor.text();
        let encoding = editor
            .document
            .as_ref()
            .map(|d| d.read().encoding())
            .unwrap_or_default();
        let existing_path = editor
            .document
            .as_ref()
            .and_then(|d| d.read().path.clone())
            .or_else(|| {
                if rustcode_platform::is_file_uri(&active_uri) || !active_uri.contains("://") {
                    Some(rustcode_platform::file_uri_to_path(&active_uri))
                } else {
                    None
                }
            });

        let opts = FileDialogOptions {
            title: Some("Save As".to_string()),
            default_directory: existing_path
                .as_ref()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .or_else(|| self.view.workspace.folders().first().map(|f| f.path.clone())),
            default_file_name: existing_path
                .as_ref()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .or_else(|| Some(active_tab.title.clone())),
            ..Default::default()
        };
        let Some(picked_path) = self.file_dialog_backend.pick_save_file(&opts) else {
            return false;
        };
        let target_path = rustcode_platform::strip_windows_verbatim_prefix(picked_path);
        let mut temp_doc = TextDocument::new(&content, Some(target_path.clone()), None);
        if temp_doc.save_with_encoding_sync(&self.file_system, encoding).is_err() {
            return false;
        }
        let saved_meta = temp_doc.metadata.clone();

        let mut renamed = false;
        if let Some(old_path) = &existing_path {
            renamed = self.view.editor_area.rename_tab(old_path, &target_path);
        }
        let new_uri = self
            .view
            .editor_area
            .find_open_tab_uri_for_path(&target_path)
            .unwrap_or_else(|| rustcode_platform::path_to_file_uri(&target_path));

        if renamed {
            self.view
                .editor_area
                .for_each_editor_for_uri_mut(&new_uri, |ed| {
                    if let Some(doc) = &ed.document {
                        let mut d = doc.write();
                        d.path = Some(target_path.clone());
                        d.encoding = encoding;
                        if let Some(meta) = saved_meta.clone() {
                            d.mark_saved(meta);
                        }
                    }
                });
            for group in &mut self.view.editor_area.model.groups {
                for tab in &mut group.tabs {
                    if tab.uri == new_uri {
                        tab.is_dirty = false;
                    }
                }
            }
        } else {
            if let Some(idx) = self
                .view
                .editor_area
                .model
                .current_group_ref()
                .active_tab_index
            {
                self.view.editor_area.close_tab(idx);
            }
            self.open_path(&target_path, false);
        }

        if let Some(bridge) = &self.language_bridge {
            bridge.did_save_document_sync(&new_uri);
        }
        self.view.editor_area.sync_dirty_states();
        self.sync_document_lifecycle();
        self.sync_configuration();
        self.wire_clipboard_on_open_editors();
        true
    }

    pub fn refresh_open_file_from_disk(&mut self, path: &Path) -> bool {
        let Some(uri) = self.view.editor_area.find_open_tab_uri_for_path(path) else {
            return false;
        };
        let mut refreshed = false;
        let doc_opt = self
            .view
            .editor_area
            .any_editor_for_uri(&uri)
            .and_then(|ed| ed.document.clone());
        if let Some(doc) = doc_opt {
            let mut d = doc.write();
            d.mark_edited();
            if d.revert_sync(&self.file_system).is_ok() {
                refreshed = true;
            }
        } else if let Ok(disk_text) = std::fs::read_to_string(path) {
            self.view
                .editor_area
                .for_each_editor_for_uri_mut(&uri, |ed| {
                    *ed.buffer.write() = rustcode_editor::PieceTable::from_str(&disk_text);
                });
            refreshed = true;
        }
        if refreshed {
            let registry = Arc::clone(&self.view.editor_area.grammar_registry);
            let theme = self.theme.current().clone();
            self.view
                .editor_area
                .for_each_editor_for_uri_mut(&uri, |ed| {
                    let pos = ed.primary_position();
                    let line_count = ed.buffer.read().line_count().max(1);
                    let clamped_line = pos.line.min(line_count - 1);
                    let line_utf16_len = ed
                        .buffer
                        .read()
                        .get_line_content(clamped_line)
                        .map_or(0, |l| l.chars().map(|c| c.len_utf16()).sum());
                    ed.set_primary_position(Position::new(
                        clamped_line,
                        pos.column.min(line_utf16_len),
                    ));
                    if let Some(doc) = &ed.document {
                        let lang = doc.read().language_id.clone();
                        ed.set_language(&lang, theme.clone(), &registry);
                    }
                    ed.recompute_folding();
                });
            self.view.editor_area.sync_dirty_states();
            self.view
                .editor_area
                .sync_breadcrumbs(Some(&self.view.workspace));
            self.sync_document_lifecycle();
        }
        refreshed
    }

    pub fn execute_workspace_replace(&mut self) -> Result<WorkspaceReplaceResult, String> {
        let outcome = self
            .view
            .search_view
            .execute_replace(self.view.workspace.folders())?;
        for modified in &outcome.modified_files {
            self.refresh_open_file_from_disk(&modified.file_path);
        }
        let _ = self.view.refresh_scm();
        Ok(outcome)
    }

    pub fn handle_search_action(&mut self, action: &SearchAction) {
        match action {
            SearchAction::OpenMatch { path, line, col } => {
                self.open_path(path, true);
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.jump_to_position_and_unfold(Position::new(
                        line.saturating_sub(1),
                        *col,
                    ));
                }
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
            }
            SearchAction::ExecuteSearch => {
                self.view
                    .search_view
                    .execute_search(self.view.workspace.folders());
            }
            SearchAction::ExecuteReplace => {
                let _ = self.execute_workspace_replace();
            }
            SearchAction::ClearSearch => {
                self.view.search_view.clear();
            }
            SearchAction::ToggleMatchCase
            | SearchAction::ToggleWholeWord
            | SearchAction::ToggleRegex => {}
        }
    }

    pub fn open_scm_diff(&mut self, path: &Path, staged: bool) -> Result<(), String> {
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(backend) = &self.view.scm_view.git_backend {
            backend.root().join(path)
        } else {
            self.view.scm_view.repo.root.join(path)
        };
        let (original, modified) = if let Some(backend) = &self.view.scm_view.git_backend {
            let orig = backend.show_head(path).ok().flatten().unwrap_or_default();
            let modif = if staged {
                backend
                    .diff(path, true)
                    .ok()
                    .filter(|d| !d.is_empty())
                    .or_else(|| std::fs::read_to_string(&full_path).ok())
                    .unwrap_or_default()
            } else {
                std::fs::read_to_string(&full_path).unwrap_or_default()
            };
            (orig, modif)
        } else if let Some((orig, modif)) = self.view.scm_view.repo.get_file_diff(path) {
            (orig.to_string(), modif.to_string())
        } else {
            let modif = std::fs::read_to_string(&full_path).unwrap_or_default();
            (String::new(), modif)
        };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "diff".to_string());
        let title = if staged {
            format!("{} (Index)", file_name)
        } else {
            format!("{} (Working Tree)", file_name)
        };
        let norm_path = path.to_string_lossy().replace('\\', "/");
        let diff_uri = if staged {
            format!("diff://staged/{norm_path}")
        } else {
            format!("diff://{norm_path}")
        };
        self.view
            .editor_area
            .open_diff(&diff_uri, &title, &original, &modified);
        self.view
            .editor_area
            .sync_breadcrumbs(Some(&self.view.workspace));
        self.view.focused_region = FocusedRegion::Editor;
        self.sync_focus_context_keys();
        self.view.sync_scm_status_bar();
        Ok(())
    }

    pub fn handle_scm_action(&mut self, action: &ScmAction) -> Result<(), String> {
        match action {
            ScmAction::StageFile(path) => {
                self.view
                    .scm_view
                    .stage_file(path)
                    .map_err(|e| e.to_string())?;
            }
            ScmAction::UnstageFile(path) => {
                self.view
                    .scm_view
                    .unstage_file(path)
                    .map_err(|e| e.to_string())?;
            }
            ScmAction::StageAll => {
                self.view
                    .scm_view
                    .stage_all()
                    .map_err(|e| e.to_string())?;
            }
            ScmAction::UnstageAll => {
                self.view
                    .scm_view
                    .unstage_all()
                    .map_err(|e| e.to_string())?;
            }
            ScmAction::DiscardChanges(path) => {
                self.view
                    .scm_view
                    .discard_file(path)
                    .map_err(|e| e.to_string())?;
                self.refresh_open_file_from_disk(path);
            }
            ScmAction::Commit { message } => {
                self.view.scm_view.commit_with_message(message)?;
            }
            ScmAction::OpenDiff {
                path,
                title,
                original,
                modified,
            } => {
                let path_str = format!("diff://{}", path.to_string_lossy().replace('\\', "/"));
                self.view
                    .editor_area
                    .open_diff(&path_str, title, original, modified);
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
                self.view.focused_region = FocusedRegion::Editor;
                self.sync_focus_context_keys();
            }
        }
        self.view.scm_view.last_action = None;
        self.view.sync_scm_status_bar();
        Ok(())
    }

    pub fn handle_terminal_action(&mut self, action: &TerminalAction) -> bool {
        let ok = self.view.terminal_panel.handle_terminal_action(action);
        self.view.terminal_panel.last_action = None;
        self.poll_terminal_ptys();
        if matches!(action, TerminalAction::CloseTerminal { .. })
            && !self.view.terminal_panel.has_active_terminal()
        {
            self.view.layout.mutate(|l| l.panel_visible = false);
            if self.view.focused_region == FocusedRegion::Panel {
                self.view.focused_region = FocusedRegion::Editor;
            }
        }
        self.sync_focus_context_keys();
        ok
    }

    fn mirror_debug_console_from_session(&mut self) {
        let session_len = self.view.debug_sidebar.session.console_output.len();
        if session_len > self.synced_debug_console_len {
            let new_entries =
                self.view.debug_sidebar.session.console_output[self.synced_debug_console_len..]
                    .to_vec();
            for entry in new_entries {
                self.view
                    .debug_console
                    .append_output(&entry.category, &entry.text);
            }
            self.synced_debug_console_len = session_len;
        }
    }

    fn sync_active_debug_frame_in_editor(&mut self) {
        if !self.view.debug_sidebar.session.is_active {
            self.view
                .editor_area
                .for_each_editor_mut(|_, ed| ed.set_execution_line(None));
            return;
        }
        let frame_opt = self
            .view
            .debug_sidebar
            .session
            .active_frame_id
            .and_then(|fid| {
                self.view
                    .debug_sidebar
                    .session
                    .call_stack
                    .iter()
                    .find(|f| f.id == fid)
                    .cloned()
            })
            .or_else(|| self.view.debug_sidebar.session.call_stack.first().cloned());

        if let Some(frame) = frame_opt {
            if let Some(path_str) = frame.source.as_ref().and_then(|s| s.path.as_deref()) {
                let path = PathBuf::from(path_str);
                self.open_path(&path, true);
                if let Some(ed) = self.view.editor_area.active_editor_mut() {
                    ed.jump_to_position_and_unfold(Position::new(
                        (frame.line as usize).saturating_sub(1),
                        (frame.column as usize).saturating_sub(1),
                    ));
                    ed.set_execution_line(Some(frame.line as usize));
                }
            }
        }
    }

    fn spawn_dap_state_sync(&self, client: Arc<DapClient>, stopped_thread_id: Option<i64>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let snapshots = Arc::clone(&self.pending_dap_snapshots);
        handle.spawn(async move {
            let Ok(threads) = client.threads().await else {
                return;
            };
            let tid = stopped_thread_id.or_else(|| threads.first().map(|t| t.id));
            let frames = if let Some(id) = tid {
                client.stack_trace(id).await.unwrap_or_default()
            } else {
                Vec::new()
            };
            let first_fid = frames.first().map(|f| f.id);
            let scopes = if let Some(fid) = first_fid {
                client.scopes(fid).await.unwrap_or_default()
            } else {
                Vec::new()
            };
            let mut variables = Vec::new();
            for scope in &scopes {
                if scope.variables_reference > 0 && !scope.expensive {
                    if let Ok(vars) = client.variables(scope.variables_reference).await {
                        variables.push((scope.variables_reference, vars));
                    }
                }
            }
            snapshots.lock().push(PendingDapSnapshot {
                threads,
                frames,
                scopes,
                variables,
                stopped_thread_id: tid,
            });
        });
    }

    pub async fn sync_debug_from_dap(
        &mut self,
        stopped_thread_id: Option<i64>,
    ) -> Result<(), DapError> {
        let Some(client) = self.dap_client.clone() else {
            return Err(DapError::NotRunning);
        };
        self.view
            .debug_sidebar
            .sync_from_dap_client(&client, stopped_thread_id)
            .await?;
        self.view
            .debug_toolbar
            .on_session_started(self.view.debug_sidebar.session.is_paused);
        self.mirror_debug_console_from_session();
        self.sync_active_debug_frame_in_editor();
        Ok(())
    }

    pub async fn start_debug_session(&mut self, config_index: Option<usize>) -> Result<(), String> {
        if let Some(idx) = config_index {
            self.view.debug_sidebar.select_launch_configuration(idx);
        }
        let ws_root = self
            .view
            .workspace
            .folders()
            .first()
            .map(|f| f.path.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        let active_file = self
            .view
            .editor_area
            .active_editor()
            .and_then(|ed| ed.document.as_ref())
            .and_then(|d| d.read().path.clone());
        let mut var_ctx = LaunchVariableContext::new(&ws_root);
        if let Some(af) = active_file {
            var_ctx = var_ctx.with_active_file(af);
        }

        let resolved_cfg = if let Some(cfg) = self.view.debug_sidebar.active_launch_configuration() {
            cfg.resolve(&var_ctx)
        } else if self.dap_client.is_some() {
            rustcode_protocols::dap::LaunchConfiguration {
                name: "Attached Adapter".to_string(),
                adapter_type: "rustcode".to_string(),
                request: "launch".to_string(),
                program: None,
                args: Vec::new(),
                cwd: Some(ws_root.to_string_lossy().into_owned()),
                env: HashMap::new(),
                stop_on_entry: None,
                pre_launch_task: None,
                extra: serde_json::Map::new(),
            }
        } else {
            return Err("No launch configuration available in .vscode/launch.json".to_string());
        };

        let client = if let Some(existing) = self
            .dap_client
            .as_ref()
            .filter(|c| c.is_running())
            .cloned()
        {
            existing
        } else {
            let adapter_cmd = resolved_cfg
                .extra
                .get("adapterCommand")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| match resolved_cfg.adapter_type.as_str() {
                    "lldb" | "codelldb" => "codelldb".to_string(),
                    "cppdbg" | "gdb" => "gdb".to_string(),
                    "python" | "debugpy" => "python".to_string(),
                    "node" | "pwa-node" => "node".to_string(),
                    other => other.to_string(),
                });
            let adapter_args: Vec<String> = resolved_cfg
                .extra
                .get("adapterArgs")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_else(|| match resolved_cfg.adapter_type.as_str() {
                    "cppdbg" | "gdb" => vec!["-i".to_string(), "dap".to_string()],
                    "python" | "debugpy" => vec!["-m".to_string(), "debugpy.adapter".to_string()],
                    _ => Vec::new(),
                });
            let arg_refs: Vec<&str> = adapter_args.iter().map(|s| s.as_str()).collect();
            let cwd = resolved_cfg
                .cwd
                .as_deref()
                .map(PathBuf::from)
                .or_else(|| Some(ws_root.clone()));
            let spawned = DapClient::spawn_process(&adapter_cmd, &arg_refs, cwd).map_err(|e| {
                let msg = format!("Failed to start debug adapter '{}': {}", adapter_cmd, e);
                self.view.debug_console.append_output("stderr", &format!("{}\n", msg));
                msg
            })?;
            let arc_client = Arc::new(spawned);
            self.attach_dap_client(Arc::clone(&arc_client));
            arc_client
        };

        client
            .initialize(InitializeRequestArguments {
                client_id: Some("rustcode".to_string()),
                client_name: Some("RustCode".to_string()),
                adapter_id: resolved_cfg.adapter_type.clone(),
                lines_start_at1: Some(true),
                columns_start_at1: Some(true),
                path_format: Some("path".to_string()),
                supports_variable_type: Some(true),
            })
            .await
            .map_err(|e| format!("DAP initialize failed: {}", e))?;

        let launch_val =
            serde_json::to_value(&resolved_cfg).map_err(|e| format!("DAP config error: {}", e))?;
        match resolved_cfg.request_kind() {
            Some(LaunchRequestKind::Attach) => client
                .attach(launch_val)
                .await
                .map_err(|e| format!("DAP attach failed: {}", e))?,
            _ => client
                .launch(launch_val)
                .await
                .map_err(|e| format!("DAP launch failed: {}", e))?,
        }

        let bp_entries: Vec<(String, Vec<rustcode_protocols::dap::SourceBreakpoint>)> = self
            .view
            .debug_sidebar
            .session
            .breakpoints
            .iter()
            .map(|(p, bps)| (p.clone(), bps.clone()))
            .collect();
        for (bp_path, bps) in bp_entries {
            let resolved_bp_path = if rustcode_platform::is_file_uri(&bp_path) {
                rustcode_platform::file_uri_to_path(&bp_path)
                    .to_string_lossy()
                    .into_owned()
            } else {
                bp_path
            };
            let _ = client.set_breakpoints(&resolved_bp_path, &bps).await;
        }

        client
            .configuration_done()
            .await
            .map_err(|e| format!("DAP configurationDone failed: {}", e))?;

        self.view.debug_sidebar.on_session_started();
        self.view.debug_toolbar.on_session_started(false);
        self.view
            .debug_console
            .append_output("console", &format!("Started debug session: {}\n", resolved_cfg.name));
        self.drain_pending_dap_events();
        Ok(())
    }

    pub async fn handle_debug_toolbar_action_async(
        &mut self,
        action: &DebugToolbarAction,
    ) -> Result<(), DapError> {
        if let Some(client) = self.dap_client.clone() {
            let res = self
                .view
                .debug_sidebar
                .dispatch_toolbar_action(action, &client)
                .await;
            if matches!(action, DebugToolbarAction::Stop) {
                self.view.debug_sidebar.on_session_terminated();
                self.view.debug_toolbar.on_session_stopped();
                self.view
                    .editor_area
                    .for_each_editor_mut(|_, ed| ed.set_execution_line(None));
            } else {
                self.view
                    .debug_toolbar
                    .set_paused(self.view.debug_sidebar.session.is_paused);
                self.drain_pending_dap_events();
            }
            res
        } else {
            if matches!(action, DebugToolbarAction::Stop) {
                self.view.debug_sidebar.on_session_terminated();
                self.view.debug_toolbar.on_session_stopped();
                self.view
                    .editor_area
                    .for_each_editor_mut(|_, ed| ed.set_execution_line(None));
            }
            Ok(())
        }
    }

    pub fn handle_debug_toolbar_action(&mut self, action: &DebugToolbarAction) {
        if matches!(action, DebugToolbarAction::Stop) {
            if let Some(client) = self.dap_client.clone() {
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        let _ = client.disconnect(false).await;
                    });
                }
            }
            self.view.debug_sidebar.on_session_terminated();
            self.view.debug_toolbar.on_session_stopped();
            self.view
                .editor_area
                .for_each_editor_mut(|_, ed| ed.set_execution_line(None));
            return;
        }
        let Some(client) = self.dap_client.clone() else {
            return;
        };
        if matches!(action, DebugToolbarAction::Continue) {
            self.view.debug_sidebar.session.is_paused = false;
            self.view.debug_toolbar.set_paused(false);
        } else if matches!(action, DebugToolbarAction::Pause) {
            self.view.debug_sidebar.session.is_paused = true;
            self.view.debug_toolbar.set_paused(true);
        }
        let tid = self
            .view
            .debug_sidebar
            .session
            .active_thread_id
            .or_else(|| {
                self.view
                    .debug_sidebar
                    .session
                    .threads
                    .first()
                    .map(|t| t.id)
            });
        let act = action.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                match act {
                    DebugToolbarAction::Continue => {
                        if let Some(thread_id) = tid {
                            let _ = client.continue_execution(thread_id).await;
                        }
                    }
                    DebugToolbarAction::Pause => {
                        if let Some(thread_id) = tid {
                            let _ = client
                                .send_request(
                                    "pause",
                                    Some(serde_json::json!({ "threadId": thread_id })),
                                )
                                .await;
                        }
                    }
                    DebugToolbarAction::StepOver => {
                        if let Some(thread_id) = tid {
                            let _ = client.next(thread_id).await;
                        }
                    }
                    DebugToolbarAction::StepInto => {
                        if let Some(thread_id) = tid {
                            let _ = client.step_in(thread_id).await;
                        }
                    }
                    DebugToolbarAction::StepOut => {
                        if let Some(thread_id) = tid {
                            let _ = client.step_out(thread_id).await;
                        }
                    }
                    DebugToolbarAction::Restart => {
                        let _ = client.restart().await;
                    }
                    DebugToolbarAction::Stop => {}
                }
            });
        }
    }

    pub async fn handle_debug_view_action_async(
        &mut self,
        action: &DebugViewAction,
    ) -> Result<(), String> {
        match action {
            DebugViewAction::Toolbar(tb) => self
                .handle_debug_toolbar_action_async(tb)
                .await
                .map_err(|e| e.to_string()),
            DebugViewAction::StartDebugging { config_index } => {
                self.start_debug_session(*config_index).await
            }
            DebugViewAction::SelectLaunchConfig(idx) => {
                self.view.debug_sidebar.select_launch_configuration(*idx);
                Ok(())
            }
            DebugViewAction::SelectFrame { path, line, .. } => {
                if let Some(p) = path {
                    self.open_path(&PathBuf::from(p), true);
                    if let Some(ed) = self.view.editor_area.active_editor_mut() {
                        ed.jump_to_position_and_unfold(Position::new(
                            (*line as usize).saturating_sub(1),
                            0,
                        ));
                        ed.set_execution_line(Some(*line as usize));
                    }
                }
                if let Some(client) = self.dap_client.clone() {
                    self.view
                        .debug_sidebar
                        .dispatch_view_action(action, &client)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            DebugViewAction::JumpToBreakpoint { path, line } => {
                self.open_path(&PathBuf::from(path), true);
                if let Some(ed) = self.view.editor_area.active_editor_mut() {
                    ed.jump_to_position_and_unfold(Position::new(
                        (*line as usize).saturating_sub(1),
                        0,
                    ));
                }
                Ok(())
            }
            DebugViewAction::ToggleBreakpoint { .. }
            | DebugViewAction::EvaluateExpression { .. } => {
                if let Some(client) = self.dap_client.clone() {
                    self.view
                        .debug_sidebar
                        .dispatch_view_action(action, &client)
                        .await
                        .map_err(|e| e.to_string())?;
                    self.mirror_debug_console_from_session();
                }
                Ok(())
            }
            DebugViewAction::ToggleVariables
            | DebugViewAction::ToggleCallStack
            | DebugViewAction::ToggleBreakpoints => Ok(()),
        }
    }

    pub fn handle_debug_view_action(&mut self, action: &DebugViewAction) {
        match action {
            DebugViewAction::Toolbar(tb) => self.handle_debug_toolbar_action(tb),
            DebugViewAction::SelectLaunchConfig(idx) => {
                self.view.debug_sidebar.select_launch_configuration(*idx);
            }
            DebugViewAction::SelectFrame {
                frame_id,
                path,
                line,
            } => {
                if let Some(p) = path {
                    self.open_path(&PathBuf::from(p), true);
                    if let Some(ed) = self.view.editor_area.active_editor_mut() {
                        ed.jump_to_position_and_unfold(Position::new(
                            (*line as usize).saturating_sub(1),
                            0,
                        ));
                        ed.set_execution_line(Some(*line as usize));
                    }
                }
                self.view.debug_sidebar.session.active_frame_id = Some(*frame_id);
                if let Some(client) = self.dap_client.clone() {
                    let fid = *frame_id;
                    let snapshots = Arc::clone(&self.pending_dap_snapshots);
                    let threads = self.view.debug_sidebar.session.threads.clone();
                    let frames = self.view.debug_sidebar.session.call_stack.clone();
                    let tid = self.view.debug_sidebar.session.active_thread_id;
                    if let Ok(handle) = tokio::runtime::Handle::try_current() {
                        handle.spawn(async move {
                            if let Ok(scopes) = client.scopes(fid).await {
                                let mut variables = Vec::new();
                                for scope in &scopes {
                                    if scope.variables_reference > 0 && !scope.expensive {
                                        if let Ok(vars) =
                                            client.variables(scope.variables_reference).await
                                        {
                                            variables.push((scope.variables_reference, vars));
                                        }
                                    }
                                }
                                snapshots.lock().push(PendingDapSnapshot {
                                    threads,
                                    frames,
                                    scopes,
                                    variables,
                                    stopped_thread_id: tid,
                                });
                            }
                        });
                    }
                }
            }
            DebugViewAction::JumpToBreakpoint { path, line } => {
                self.open_path(&PathBuf::from(path), true);
                if let Some(ed) = self.view.editor_area.active_editor_mut() {
                    ed.jump_to_position_and_unfold(Position::new(
                        (*line as usize).saturating_sub(1),
                        0,
                    ));
                }
            }
            DebugViewAction::ToggleBreakpoint { path, .. } => {
                if let Some(client) = self.dap_client.clone() {
                    let bps = self
                        .view
                        .debug_sidebar
                        .session
                        .breakpoints
                        .get(path)
                        .cloned()
                        .unwrap_or_default();
                    let resolved = if rustcode_platform::is_file_uri(path) {
                        rustcode_platform::file_uri_to_path(path)
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        path.clone()
                    };
                    if let Ok(handle) = tokio::runtime::Handle::try_current() {
                        handle.spawn(async move {
                            let _ = client.set_breakpoints(&resolved, &bps).await;
                        });
                    }
                }
            }
            DebugViewAction::EvaluateExpression { expression } => {
                if let Some(client) = self.dap_client.clone() {
                    let expr = expression.clone();
                    let frame_id = self.view.debug_sidebar.session.active_frame_id;
                    let events = Arc::clone(&self.pending_dap_events);
                    if let Ok(handle) = tokio::runtime::Handle::try_current() {
                        handle.spawn(async move {
                            if let Ok(body) = client.evaluate(&expr, frame_id).await {
                                events.lock().push(DapMessage::Event {
                                    seq: 0,
                                    event: "output".to_string(),
                                    body: Some(serde_json::json!({
                                        "category": "console",
                                        "output": format!("{}\n", body.result),
                                    })),
                                });
                            }
                        });
                    }
                }
            }
            DebugViewAction::StartDebugging { config_index } => {
                if let Some(idx) = config_index {
                    self.view.debug_sidebar.select_launch_configuration(*idx);
                }
            }
            DebugViewAction::ToggleVariables
            | DebugViewAction::ToggleCallStack
            | DebugViewAction::ToggleBreakpoints => {}
        }
    }

    fn drain_pending_dap_events(&mut self) {
        let events: Vec<DapMessage> = {
            let mut guard = self.pending_dap_events.lock();
            std::mem::take(&mut *guard)
        };
        for msg in events {
            let event_name = match &msg {
                DapMessage::Event { event, .. } => event.as_str(),
                _ => "",
            };
            self.view.debug_sidebar.apply_dap_event(&msg);
            match event_name {
                "stopped" => {
                    self.view.debug_toolbar.on_session_started(true);
                    let tid = match &msg {
                        DapMessage::Event {
                            body: Some(val), ..
                        } => serde_json::from_value::<StoppedEventBody>(val.clone())
                            .ok()
                            .and_then(|b| b.thread_id),
                        _ => None,
                    };
                    if let Some(client) = self.dap_client.clone() {
                        self.spawn_dap_state_sync(client, tid);
                    }
                }
                "continued" => {
                    self.view
                        .debug_toolbar
                        .set_paused(self.view.debug_sidebar.session.is_paused);
                    if !self.view.debug_sidebar.session.is_paused {
                        self.view
                            .editor_area
                            .for_each_editor_mut(|_, ed| ed.set_execution_line(None));
                    }
                }
                "terminated" | "exited" => {
                    self.view.debug_toolbar.on_session_stopped();
                    self.view
                        .editor_area
                        .for_each_editor_mut(|_, ed| ed.set_execution_line(None));
                }
                _ => {}
            }
        }

        let snapshots: Vec<PendingDapSnapshot> = {
            let mut guard = self.pending_dap_snapshots.lock();
            std::mem::take(&mut *guard)
        };
        for snap in snapshots {
            self.view.debug_sidebar.apply_threads(snap.threads);
            self.view.debug_sidebar.session.is_active = true;
            self.view.debug_sidebar.session.is_paused = true;
            if snap.stopped_thread_id.is_some() {
                self.view.debug_sidebar.session.active_thread_id = snap.stopped_thread_id;
            }
            self.view.debug_sidebar.apply_stack_frames(snap.frames);
            self.view.debug_sidebar.apply_scopes(snap.scopes);
            self.view.debug_sidebar.session.variables.clear();
            for (vref, vars) in snap.variables {
                self.view.debug_sidebar.apply_variables(vref, vars);
            }
            self.view.debug_toolbar.on_session_started(true);
            self.sync_active_debug_frame_in_editor();
        }
        self.mirror_debug_console_from_session();
    }

    fn active_editor_uri_and_lang(&self) -> Option<(String, String)> {
        let tab = self
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()?;
        let uri = tab.uri.clone();
        let lang = self
            .view
            .editor_area
            .active_editor()
            .and_then(|ed| ed.document.as_ref())
            .map(|doc| doc.read().language_id.clone())
            .unwrap_or_else(|| "plaintext".to_string());
        Some((uri, lang))
    }

    fn convert_lsp_completion_item(item: LspCompletionItem) -> CompletionItem {
        let kind = item.kind.map(|k| match k {
            LspCompletionKind::Text => EditorCompletionKind::Text,
            LspCompletionKind::Method => EditorCompletionKind::Method,
            LspCompletionKind::Function => EditorCompletionKind::Function,
            LspCompletionKind::Constructor => EditorCompletionKind::Constructor,
            LspCompletionKind::Field => EditorCompletionKind::Field,
            LspCompletionKind::Variable => EditorCompletionKind::Variable,
            LspCompletionKind::Class => EditorCompletionKind::Class,
            LspCompletionKind::Interface | LspCompletionKind::TypeParameter => {
                EditorCompletionKind::Interface
            }
            LspCompletionKind::Module => EditorCompletionKind::Module,
            LspCompletionKind::Property => EditorCompletionKind::Property,
            LspCompletionKind::Keyword | LspCompletionKind::Operator => EditorCompletionKind::Keyword,
            LspCompletionKind::Snippet => EditorCompletionKind::Snippet,
            LspCompletionKind::Struct => EditorCompletionKind::Struct,
            LspCompletionKind::Enum | LspCompletionKind::EnumMember => EditorCompletionKind::Enum,
            LspCompletionKind::Constant | LspCompletionKind::Value => EditorCompletionKind::Constant,
            LspCompletionKind::File | LspCompletionKind::Folder => EditorCompletionKind::File,
            LspCompletionKind::Reference => EditorCompletionKind::Reference,
            LspCompletionKind::Unit | LspCompletionKind::Color | LspCompletionKind::Event => {
                EditorCompletionKind::Text
            }
        });
        CompletionItem {
            label: item.label,
            kind,
            detail: item.detail,
            documentation: item.documentation,
            insert_text: item.insert_text,
            filter_text: item.filter_text,
            sort_text: item.sort_text,
        }
    }

    fn apply_lsp_hover_to_editor(
        &mut self,
        uri: &str,
        position: Position,
        hover: Option<Hover>,
    ) -> bool {
        let mut applied = false;
        self.view.editor_area.for_each_editor_for_uri_mut(uri, |ed| {
            if let Some(h) = &hover {
                if h.contents.value.trim().is_empty() {
                    ed.clear_lsp_hover();
                } else {
                    let range = h
                        .range
                        .map(|r| {
                            Range::new(
                                Position::new(r.start.line as usize, r.start.character as usize),
                                Position::new(r.end.line as usize, r.end.character as usize),
                            )
                        })
                        .or(Some(Range::new(position, position)));
                    ed.set_lsp_hover(
                        Some(LspHoverInfo {
                            contents: h.contents.value.clone(),
                            range,
                            position,
                        }),
                        None,
                    );
                    applied = true;
                }
            } else {
                ed.clear_lsp_hover();
            }
        });
        applied
    }

    pub fn navigate_to_definition_locations(&mut self, locations: &[LspLocation]) -> bool {
        let Some(loc) = locations.first() else {
            return false;
        };
        let target_pos = Position::new(
            loc.range.start.line as usize,
            loc.range.start.character as usize,
        );
        let target_path = rustcode_platform::file_uri_to_path(&loc.uri);
        let active_uri = self
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .map(|t| t.uri.clone());
        let same_file = active_uri.as_deref().is_some_and(|cur| {
            cur == loc.uri
                || rustcode_platform::paths_equivalent(
                    &rustcode_platform::file_uri_to_path(cur),
                    &target_path,
                )
        });

        if !same_file {
            if let Some(existing_uri) = self
                .view
                .editor_area
                .find_open_tab_uri_for_path(&target_path)
            {
                let title = target_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("untitled")
                    .to_string();
                self.open_file(&existing_uri, &title, "", false);
            } else if target_path.exists() {
                self.open_path(&target_path, false);
            } else {
                let title = target_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("untitled")
                    .to_string();
                self.open_file(&loc.uri, &title, "", false);
            }
        }

        if let Some(editor) = self.view.editor_area.active_editor_mut() {
            editor.jump_to_position_and_unfold(target_pos);
        }
        self.view
            .editor_area
            .sync_breadcrumbs(Some(&self.view.workspace));
        self.view.focused_region = FocusedRegion::Editor;
        self.sync_focus_context_keys();
        true
    }

    pub async fn request_hover_at(&mut self, pos: Position) -> bool {
        let Some((uri, lang_id)) = self.active_editor_uri_and_lang() else {
            return false;
        };
        let Some(bridge) = self.language_bridge.clone() else {
            return false;
        };
        let hover = bridge
            .request_hover(
                &uri,
                &lang_id,
                LspPosition::new(pos.line as u32, pos.column as u32),
            )
            .await;
        self.apply_lsp_hover_to_editor(&uri, pos, hover)
    }

    pub async fn go_to_definition_at(&mut self, pos: Position) -> bool {
        let Some((uri, lang_id)) = self.active_editor_uri_and_lang() else {
            return false;
        };
        let Some(bridge) = self.language_bridge.clone() else {
            return false;
        };
        let locs = bridge
            .request_definition(
                &uri,
                &lang_id,
                LspPosition::new(pos.line as u32, pos.column as u32),
            )
            .await;
        self.navigate_to_definition_locations(&locs)
    }

    pub async fn request_completions_at(
        &mut self,
        pos: Position,
        trigger_char: Option<char>,
    ) -> usize {
        let Some((uri, lang_id)) = self.active_editor_uri_and_lang() else {
            return 0;
        };
        let Some(bridge) = self.language_bridge.clone() else {
            return 0;
        };
        let items = bridge
            .request_completions(
                &uri,
                &lang_id,
                LspPosition::new(pos.line as u32, pos.column as u32),
                trigger_char,
            )
            .await;
        let count = items.len();
        if count > 0 {
            let converted: Vec<CompletionItem> = items
                .into_iter()
                .map(Self::convert_lsp_completion_item)
                .collect();
            self.view.editor_area.for_each_editor_for_uri_mut(&uri, |ed| {
                ed.merge_lsp_completions(converted.clone(), pos, trigger_char);
            });
        }
        count
    }

    fn trigger_hover_at(&self, uri: String, lang_id: String, pos: Position) {
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let queue = Arc::clone(&self.pending_lsp_results);
        handle.spawn(async move {
            let hover = bridge
                .request_hover(
                    &uri,
                    &lang_id,
                    LspPosition::new(pos.line as u32, pos.column as u32),
                )
                .await;
            queue.lock().push(PendingLspResult::Hover {
                uri,
                position: pos,
                hover,
            });
        });
    }

    fn trigger_go_to_definition_at(&self, pos: Position) {
        let Some((uri, lang_id)) = self.active_editor_uri_and_lang() else {
            return;
        };
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let queue = Arc::clone(&self.pending_lsp_results);
        handle.spawn(async move {
            let locations = bridge
                .request_definition(
                    &uri,
                    &lang_id,
                    LspPosition::new(pos.line as u32, pos.column as u32),
                )
                .await;
            if !locations.is_empty() {
                queue
                    .lock()
                    .push(PendingLspResult::Definition { locations });
            }
        });
    }

    fn trigger_completions_at(&self, pos: Position, trigger_char: Option<char>) {
        let Some((uri, lang_id)) = self.active_editor_uri_and_lang() else {
            return;
        };
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let queue = Arc::clone(&self.pending_lsp_results);
        handle.spawn(async move {
            let items = bridge
                .request_completions(
                    &uri,
                    &lang_id,
                    LspPosition::new(pos.line as u32, pos.column as u32),
                    trigger_char,
                )
                .await;
            if !items.is_empty() {
                queue.lock().push(PendingLspResult::Completions {
                    uri,
                    position: pos,
                    trigger_char,
                    items,
                });
            }
        });
    }

    fn formatting_options_for_uri(&self, uri: &str) -> FormattingOptions {
        let path_opt = if rustcode_platform::is_file_uri(uri) || !uri.contains("://") {
            Some(rustcode_platform::file_uri_to_path(uri))
        } else {
            None
        };
        let tab_size = self
            .config
            .get_for_resource::<u32>("editor.tabSize", path_opt.as_deref())
            .unwrap_or(4);
        let insert_spaces = self
            .config
            .get_for_resource::<bool>("editor.insertSpaces", path_opt.as_deref())
            .unwrap_or(true);
        FormattingOptions::new(tab_size, insert_spaces)
    }

    fn trigger_format_active_document(&self, save_after: bool) {
        let Some((uri, lang_id)) = self.active_editor_uri_and_lang() else {
            return;
        };
        let Some(bridge) = self.language_bridge.clone() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let options = self.formatting_options_for_uri(&uri);
        let queue = Arc::clone(&self.pending_lsp_results);
        handle.spawn(async move {
            let edits = bridge.request_formatting(&uri, &lang_id, options).await;
            if !edits.is_empty() || save_after {
                queue.lock().push(PendingLspResult::Formatting {
                    uri,
                    edits,
                    save_after,
                });
            }
        });
    }

    fn apply_lsp_text_edits_to_uri(&mut self, uri: &str, edits: Vec<LspTextEdit>) -> bool {
        if edits.is_empty() {
            return false;
        }
        let range_edits: Vec<RangeEdit> = edits
            .into_iter()
            .map(|e| RangeEdit {
                range: Range::new(
                    Position::new(e.range.start.line as usize, e.range.start.character as usize),
                    Position::new(e.range.end.line as usize, e.range.end.character as usize),
                ),
                new_text: e.new_text,
            })
            .collect();
        let applied = self.view.editor_area.apply_text_edits(uri, &range_edits);
        if applied {
            self.sync_document_lifecycle();
            self.sync_configuration();
        }
        applied
    }

    fn drain_pending_lsp_results(&mut self) {
        let results: Vec<PendingLspResult> = {
            let mut guard = self.pending_lsp_results.lock();
            std::mem::take(&mut *guard)
        };
        for item in results {
            match item {
                PendingLspResult::Hover {
                    uri,
                    position,
                    hover,
                } => {
                    self.apply_lsp_hover_to_editor(&uri, position, hover);
                }
                PendingLspResult::Definition { locations } => {
                    self.navigate_to_definition_locations(&locations);
                }
                PendingLspResult::Completions {
                    uri,
                    position,
                    trigger_char,
                    items,
                } => {
                    let converted: Vec<CompletionItem> = items
                        .into_iter()
                        .map(Self::convert_lsp_completion_item)
                        .collect();
                    self.view.editor_area.for_each_editor_for_uri_mut(&uri, |ed| {
                        ed.merge_lsp_completions(converted.clone(), position, trigger_char);
                    });
                }
                PendingLspResult::Formatting {
                    uri,
                    edits,
                    save_after,
                } => {
                    self.apply_lsp_text_edits_to_uri(&uri, edits);
                    if save_after {
                        let _ = self.save_active_file_without_format();
                    }
                }
            }
        }
    }

    fn save_active_file_without_format(&mut self) -> bool {
        let active_uri = self
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .map(|t| t.uri.clone());
        let saved = self
            .view
            .editor_area
            .save_active_file(&self.file_system)
            .is_ok();
        if saved {
            if let Some(uri) = active_uri {
                let lang_opt = self
                    .view
                    .editor_area
                    .any_editor_for_uri(&uri)
                    .and_then(|ed| ed.document.as_ref())
                    .map(|d| d.read().language_id.clone());
                if let Some(bridge) = &self.language_bridge {
                    bridge.did_save_document_sync(&uri);
                    if let Some(lang) = lang_opt {
                        if let Some(client) = bridge.lsp_manager.get_client(&lang) {
                            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                                let uri_owned = uri.clone();
                                handle.spawn(async move {
                                    let _ = client
                                        .send_notification(
                                            "textDocument/didSave",
                                            serde_json::json!({
                                                "textDocument": { "uri": uri_owned }
                                            }),
                                        )
                                        .await;
                                });
                            }
                        }
                    }
                }
            }
            self.sync_document_lifecycle();
        }
        saved
    }

    pub fn with_language_bridge(mut self, bridge: Arc<LanguageFeatureBridge>) -> Self {
        self.set_language_bridge(bridge);
        self
    }

    pub fn set_language_bridge(&mut self, bridge: Arc<LanguageFeatureBridge>) {
        self.language_bridge = Some(bridge);
        self.reset_bridge_sync_state();
        self.sync_bridge_state();
        self.ensure_lsp_for_open_documents();
    }

    fn execute_command_from_dispatch(&self, cmd_id: &str) {
        *self.command_sink.suppress_command.lock() = Some(cmd_id.to_string());
        let _guard = SuppressCommandGuard {
            slot: &self.command_sink.suppress_command,
        };
        let _ = self.commands.execute(cmd_id, None);
    }

    pub fn drain_pending_command_effects(&mut self) {
        let _ = self.drain_pending_command_effects_returning();
    }

    fn drain_pending_command_effects_returning(&mut self) -> Vec<String> {
        let cmds: Vec<String> = {
            let mut guard = self.command_sink.pending_commands.lock();
            std::mem::take(&mut *guard)
        };
        for cmd in &cmds {
            self.handle_builtin_command_effects(cmd);
        }
        cmds
    }

    /// Renders the workbench window into the given render surface and returns the display list.
    pub fn render(&self, surface: &mut dyn RenderSurface) -> Scene {
        self.sync_for_render();
        let size = surface.size();
        let bounds = Rect::from_xywh(0.0, 0.0, size.width, size.height);
        let theme_styles = WorkbenchThemeStyles::from_theme(self.theme.current());
        let mut scene = Scene::new();

        self.view
            .render(bounds, &theme_styles, &self.font_metrics, &mut scene);
        surface.render_scene(&scene);
        scene
    }

    /// Primary event router mapping keyboard chords and mouse events to editor edits or commands.
    pub fn dispatch_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let drained_cmds = self.drain_pending_command_effects_returning();
        let just_activated_sidebar_cmd = drained_cmds
            .iter()
            .any(|c| c.starts_with("workbench.view."));

        if let Some(pending_action) = self.view.last_action.take() {
            self.handle_workbench_action(pending_action, event, bounds);
        }

        // 1. If key down, check keybindings evaluation first (unless modal overlay or explorer inline input is open)
        if !self.view.is_command_palette_open
            && !self.view.is_prompt_open()
            && !self.view.notifications.has_modal()
            && !self.view.is_explorer_input_active()
        {
            if let InputEvent::KeyDown(key_ev) = event {
                let combo = key_ev.to_key_combo();
                if let Some(cmd_id) = self.keybindings.handle_key(&combo, &self.context_keys) {
                    self.execute_command_from_dispatch(&cmd_id);
                    self.handle_builtin_command_effects(&cmd_id);
                    self.sync_all();
                    return EventResult::Consumed;
                }
            }
        }

        // 2. Route event to workbench view hierarchy
        let res = self.view.handle_event(event, bounds);

        // Preserve sidebar visibility if a sidebar view command was just drained on the same click
        if just_activated_sidebar_cmd
            && matches!(event, InputEvent::MouseDown(_))
            && matches!(
                self.view.activity_bar.last_action,
                Some(crate::activitybar::ActivityBarAction::ToggleSidebar)
            )
            && matches!(self.view.last_action, Some(WorkbenchAction::SidebarToggled(false)))
        {
            self.view.layout.mutate(|l| l.sidebar_visible = true);
            self.view.last_action = None;
        }

        // 3. Update context keys from focused region
        self.sync_focus_context_keys();

        // 3b. Poll EditorView actions (GoToDefinition, FormatDocument, OpenCommandPalette, Hover)
        let editor_action = self
            .view
            .editor_area
            .active_editor_mut()
            .and_then(|ed| ed.take_last_action());
        if let Some(ed_act) = editor_action {
            match ed_act {
                EditorAction::GoToDefinition(pos) => {
                    self.trigger_go_to_definition_at(pos);
                }
                EditorAction::FormatDocument => {
                    self.trigger_format_active_document(false);
                }
                EditorAction::OpenCommandPalette => {
                    self.open_command_palette();
                }
                EditorAction::HoverRequested(pos) => {
                    if let Some((uri, lang)) = self.active_editor_uri_and_lang() {
                        self.trigger_hover_at(uri, lang, pos);
                    }
                }
            }
        }

        if matches!(event, InputEvent::MouseMove(_)) {
            let hover_pos = self
                .view
                .editor_area
                .active_editor()
                .and_then(|ed| ed.hover_request_position());
            match (self.active_editor_uri_and_lang(), hover_pos) {
                (Some((uri, lang)), Some(pos)) => {
                    if self.last_hover_request.as_ref() != Some(&(uri.clone(), pos)) {
                        self.last_hover_request = Some((uri.clone(), pos));
                        self.trigger_hover_at(uri, lang, pos);
                    }
                }
                _ => {
                    if self.last_hover_request.is_some() {
                        self.last_hover_request = None;
                        if let Some(ed) = self.view.editor_area.active_editor_mut() {
                            ed.clear_lsp_hover();
                        }
                    }
                }
            }
        } else if let InputEvent::KeyDown(key_ev) = event {
            if self.view.focused_region == FocusedRegion::Editor
                && !key_ev.modifiers.ctrl
                && !key_ev.modifiers.alt
                && !key_ev.modifiers.meta
            {
                if let crate::workbench_view::FocusedRegion::Editor = self.view.focused_region {
                    if let Some((_uri, lang)) = self.active_editor_uri_and_lang() {
                        let typed_char = match key_ev.key {
                            rustcode_platform::render::KeyCode::Char(c) => Some(c),
                            _ => key_ev
                                .text
                                .as_deref()
                                .and_then(|t| t.chars().next()),
                        };
                        let is_trigger = typed_char.is_some_and(|ch| {
                            self.language_bridge.as_ref().is_some_and(|b| {
                                b.lsp_manager.get_trigger_characters(&lang).contains(&ch)
                            })
                        });
                        if is_trigger {
                            if let Some(pos) = self
                                .view
                                .editor_area
                                .active_editor()
                                .map(|ed| ed.primary_position())
                            {
                                self.trigger_completions_at(pos, typed_char);
                            }
                        }
                    }
                }
            }
        }

        if let Some(search_act) = self.view.search_view.last_action.take() {
            if matches!(search_act, SearchAction::ExecuteReplace) {
                let _ = self.execute_workspace_replace();
            }
        }

        // 4. Process any actions emitted by views
        if let Some(action) = self.view.last_action.take() {
            self.handle_workbench_action(action, event, bounds);
        }

        if !matches!(event, InputEvent::MouseMove(_)) {
            if bounds.is_finite_positive() {
                let regions = self.view.compute_regions(bounds);
                if self.view.layout.panel_visible && self.view.layout.active_panel_tab == "terminal"
                {
                    let panel_content = WorkbenchView::panel_content_rect(regions.panel);
                    self.view.terminal_panel.resize_to_bounds(panel_content);
                }
                if !self.view.editor_area.webviews.is_empty()
                    || !self.view.editor_area.split_webviews.is_empty()
                {
                    let group_rects = self
                        .view
                        .editor_area
                        .model
                        .compute_group_rects(regions.editor_area);
                    for (gid, grect) in group_rects {
                        if let Some(uri) = self
                            .view
                            .editor_area
                            .model
                            .group_by_id(gid)
                            .and_then(|g| g.active_tab())
                            .map(|t| t.uri.clone())
                        {
                            let content_rect =
                                self.view.editor_area.content_rect_for_group(gid, grect);
                            if let Some(wv) =
                                self.view.editor_area.webview_for_group_mut(gid, &uri)
                            {
                                wv.sync_backend_bounds(content_rect);
                            }
                        }
                    }
                }
            }
            self.poll_runtime_events();
        }

        res
    }

    fn handle_workbench_action(
        &mut self,
        action: WorkbenchAction,
        event: &InputEvent,
        bounds: Rect,
    ) {
        match action {
            WorkbenchAction::ExecuteCommand(cmd) => {
                self.execute_command_from_dispatch(&cmd);
                self.handle_builtin_command_effects(&cmd);
                self.sync_focus_context_keys();
            }
            WorkbenchAction::OpenFile(path) => {
                let keep_sidebar_focus = self.view.focused_region
                    == crate::workbench_view::FocusedRegion::Sidebar
                    && matches!(event, InputEvent::MouseDown(_))
                    && !matches!(
                        self.view.layout.active_sidebar_view.as_str(),
                        "search" | "workbench.view.search"
                    );
                let uri = self
                    .view
                    .editor_area
                    .find_open_tab_uri_for_path(&path)
                    .unwrap_or_else(|| rustcode_platform::path_to_file_uri(&path));
                let title = path.file_name().map_or("untitled", |n| n.to_str().unwrap_or(""));
                let content = if self.view.editor_area.any_editor_for_uri(&uri).is_some() {
                    String::new()
                } else {
                    std::fs::read_to_string(&path).unwrap_or_default()
                };
                self.open_file(&uri, title, &content, false);
                if keep_sidebar_focus {
                    self.view.focused_region =
                        crate::workbench_view::FocusedRegion::Sidebar;
                }
                self.sync_focus_context_keys();
            }
            WorkbenchAction::JumpToDiagnostic { uri, line, column } => {
                let path = rustcode_platform::file_uri_to_path(&uri);
                let target_uri = if !uri.contains("://") && !path.is_absolute() {
                    uri.clone()
                } else {
                    self.view
                        .editor_area
                        .find_open_tab_uri_for_path(&path)
                        .unwrap_or_else(|| uri.clone())
                };
                let title = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("untitled");
                let content = if self.view.editor_area.any_editor_for_uri(&target_uri).is_some() {
                    String::new()
                } else {
                    std::fs::read_to_string(&path).unwrap_or_default()
                };
                self.open_file(&target_uri, title, &content, true);
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.jump_to_position_and_unfold(Position::new(line, column));
                    if let Some(diags) = self
                        .view
                        .problems_panel
                        .files
                        .get(&uri)
                        .or_else(|| self.view.problems_panel.files.get(&target_uri))
                    {
                        editor.set_diagnostics(diags.clone());
                    }
                }
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
                self.view.focused_region = FocusedRegion::Editor;
                self.sync_focus_context_keys();
            }
            WorkbenchAction::SidebarToggled(_) => {}
            WorkbenchAction::PanelToggled(_) => {}
            WorkbenchAction::OpenDiff {
                path,
                title,
                original,
                modified,
            } => {
                let path_str = format!("diff://{}", path.to_string_lossy().replace('\\', "/"));
                if !self.view.editor_area.diff_editors.contains_key(&path_str) {
                    self.view
                        .editor_area
                        .open_diff(&path_str, &title, &original, &modified);
                    self.view
                        .editor_area
                        .sync_breadcrumbs(Some(&self.view.workspace));
                }
                self.view.sync_scm_status_bar();
                self.view.focused_region = FocusedRegion::Editor;
                self.sync_focus_context_keys();
            }
            WorkbenchAction::DebugToolbar(tb) => {
                self.handle_debug_toolbar_action(&tb);
            }
            WorkbenchAction::DebugAction(act) => {
                self.handle_debug_view_action(&act);
            }
            WorkbenchAction::ScmAction(act) => {
                if let ScmAction::DiscardChanges(ref p) = act {
                    self.refresh_open_file_from_disk(p);
                }
                self.view.sync_scm_status_bar();
            }
            WorkbenchAction::ExtensionsAction(act) => {
                self.handle_extensions_action(&act);
            }
            WorkbenchAction::OutputAction(act) => {
                if let OutputAction::ClearChannel(id) = act {
                    self.sync_output_channels();
                    self.view.output_panel.clear_channel(&id);
                    if let Some(bridge) = &self.language_bridge {
                        if bridge.output_manager.clear(&id).is_ok() {
                            let _ = bridge.output_manager.take_pending_events();
                            self.synced_output_revision = bridge.output_manager.revision();
                        }
                    }
                }
            }
            WorkbenchAction::TerminalAction(act) => {
                let needs_exec = match &act {
                    TerminalAction::NewTerminal { id } | TerminalAction::SplitTerminal { id } => {
                        id.is_empty() || self.view.terminal_panel.manager.terminal(id).is_none()
                    }
                    TerminalAction::CloseTerminal { id } => {
                        id.is_empty() || self.view.terminal_panel.manager.terminal(id).is_some()
                    }
                    TerminalAction::SelectTab { .. }
                    | TerminalAction::FocusPane { .. }
                    | TerminalAction::ClearTerminal { .. } => true,
                    TerminalAction::CommandExecuted { .. } => false,
                };
                if needs_exec {
                    self.view.terminal_panel.handle_terminal_action(&act);
                    self.view.terminal_panel.last_action = None;
                }
                if let TerminalAction::CloseTerminal { ref id } = act {
                    if let Some(ext_id) = self
                        .ext_terminal_map
                        .iter()
                        .find(|(_, lid)| *lid == id)
                        .map(|(eid, _)| eid.clone())
                    {
                        self.ext_terminal_map.remove(&ext_id);
                        if let Some(bridge) = &self.language_bridge {
                            bridge.terminal_manager.dispose_terminal(&ext_id);
                        }
                    }
                    if !self.view.terminal_panel.has_active_terminal() {
                        self.view.layout.mutate(|l| l.panel_visible = false);
                        if self.view.focused_region == FocusedRegion::Panel {
                            self.view.focused_region = FocusedRegion::Editor;
                        }
                    }
                }
                if self.view.layout.panel_visible
                    && self.view.layout.active_panel_tab == "terminal"
                    && bounds.is_finite_positive()
                {
                    let regions = self.view.compute_regions(bounds);
                    let panel_content = WorkbenchView::panel_content_rect(regions.panel);
                    self.view.terminal_panel.resize_to_bounds(panel_content);
                }
                self.poll_terminal_ptys();
                self.sync_focus_context_keys();
            }
            WorkbenchAction::NotificationAction(act) => {
                if let Some(bridge) = &self.language_bridge {
                    match act {
                        NotificationUiAction::SelectAction { id, action_title } => {
                            let _ = bridge.window_manager.respond_notification(
                                &id,
                                Some(serde_json::Value::String(action_title)),
                            );
                        }
                        NotificationUiAction::Dismiss { id } => {
                            let _ = bridge.window_manager.dismiss_notification(&id);
                        }
                    }
                }
            }
            WorkbenchAction::PromptAction(act) => {
                if let Some(bridge) = &self.language_bridge {
                    match act {
                        PromptUiAction::SubmitQuickPick { id, value } => {
                            let _ = bridge.window_manager.respond_prompt(&id, Some(value));
                        }
                        PromptUiAction::SelectQuickPickItem {
                            client_id, item, ..
                        } => {
                            if let Some(cid) = client_id {
                                bridge.notify_quick_pick_item_selected(&cid, item);
                            }
                        }
                        PromptUiAction::SubmitInputBox { id, value } => {
                            let _ = bridge
                                .window_manager
                                .respond_prompt(&id, Some(serde_json::Value::String(value)));
                        }
                        PromptUiAction::ValidateInput {
                            id,
                            client_id,
                            value,
                        } => {
                            if let Some(cid) = client_id {
                                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                                    let bridge_clone = Arc::clone(bridge);
                                    let overlay_weak = self.view.prompt_overlay.weak_state();
                                    handle.spawn(async move {
                                        if let Ok(val_opt) =
                                            bridge_clone.request_validate_input(&cid, &value).await
                                        {
                                            let msg = val_opt.and_then(|v| match v {
                                                serde_json::Value::String(s)
                                                    if !s.is_empty() =>
                                                {
                                                    Some(s)
                                                }
                                                serde_json::Value::Object(obj) => obj
                                                    .get("message")
                                                    .and_then(|m| m.as_str())
                                                    .filter(|s| !s.is_empty())
                                                    .map(ToString::to_string),
                                                _ => None,
                                            });
                                            if let Some(state_arc) = overlay_weak.upgrade() {
                                                let mut guard = state_arc.write();
                                                if let Some(active) = guard.as_mut() {
                                                    if active.id == id {
                                                        active.set_validation_message(msg);
                                                    }
                                                }
                                            }
                                        }
                                    });
                                }
                            }
                        }
                        PromptUiAction::Cancel { id } => {
                            let _ = bridge.window_manager.cancel_prompt(&id);
                        }
                    }
                }
            }
            WorkbenchAction::TestingAction(act) => match act {
                TestingAction::RunAll => {
                    if let Some(bridge) = &self.language_bridge {
                        let controllers = bridge.test_manager.list_controllers();
                        if let Ok(handle) = tokio::runtime::Handle::try_current() {
                            for ctrl in controllers {
                                let b = Arc::clone(bridge);
                                let cid = ctrl.id;
                                handle.spawn(async move {
                                    let _ = b
                                        .request_run_tests(&cid, serde_json::json!({}))
                                        .await;
                                });
                            }
                        }
                    }
                }
                TestingAction::RunController(controller_id) => {
                    if let Some(bridge) = &self.language_bridge {
                        if let Ok(handle) = tokio::runtime::Handle::try_current() {
                            let b = Arc::clone(bridge);
                            handle.spawn(async move {
                                let _ = b
                                    .request_run_tests(&controller_id, serde_json::json!({}))
                                    .await;
                            });
                        }
                    }
                }
                TestingAction::OpenUri(_) | TestingAction::SelectItem(_) => {}
            },
            WorkbenchAction::ExplorerAction(act) => match act {
                ExplorerAction::ExpandContributedItem {
                    view_id,
                    element_id,
                } => {
                    self.fetch_contributed_tree_children(&view_id, Some(element_id));
                }
                ExplorerAction::ExecuteContributedCommand { command, arguments } => {
                    let arg_val = if arguments.is_empty() {
                        None
                    } else {
                        Some(serde_json::Value::Array(arguments.clone()))
                    };
                    if let Some(bridge) = &self.language_bridge {
                        if bridge.is_js_command(&command) {
                            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                                let b = Arc::clone(bridge);
                                handle.spawn(async move {
                                    let _ = b.request_execute_command(&command, arguments).await;
                                });
                            }
                        } else {
                            let _ = self.commands.execute(&command, arg_val);
                            self.handle_builtin_command_effects(&command);
                        }
                    } else {
                        let _ = self.commands.execute(&command, arg_val);
                        self.handle_builtin_command_effects(&command);
                    }
                }
                ExplorerAction::OpenFile(_)
                | ExplorerAction::ToggleDir(_)
                | ExplorerAction::CreateFile(_)
                | ExplorerAction::CreateFolder(_)
                | ExplorerAction::Renamed { .. }
                | ExplorerAction::Deleted(_) => {}
            },
        }
    }

    fn handle_extensions_action(&mut self, action: &ExtensionsViewAction) {
        match action {
            ExtensionsViewAction::Install {
                namespace, name, ..
            } => {
                let id = ExtensionInstaller::extension_id(namespace, name);
                self.register_extension_contributions_by_id(&id);
            }
            ExtensionsViewAction::Enable { id } => {
                self.register_extension_contributions_by_id(id);
            }
            ExtensionsViewAction::Disable { id } | ExtensionsViewAction::Uninstall { id } => {
                self.extension_disposables.remove(&id.to_ascii_lowercase());
            }
            ExtensionsViewAction::Search(_) | ExtensionsViewAction::SelectExtension(_) => {}
        }
    }

    fn register_extension_contributions_by_id(&mut self, id: &str) {
        let Some(installer) = self.view.extensions_view.installer.clone() else {
            return;
        };
        if !installer.is_enabled(id) {
            return;
        }
        let installed = installer.list_installed();
        if let Some(ext) = installed.into_iter().find(|ext| {
            ExtensionInstaller::extension_id(&ext.manifest.publisher, &ext.manifest.name)
                .eq_ignore_ascii_case(id)
        }) {
            self.extension_disposables.remove(&id.to_ascii_lowercase());
            if let Ok((_, store)) = installer.register_contributions(
                &ext,
                &self.commands,
                Some(&mut self.theme),
                |_lang, _path| Ok(()),
            ) {
                self.extension_disposables
                    .insert(id.to_ascii_lowercase(), store);
            }
            if let Some(contribs) = ext.manifest.contributes {
                for ce in contribs.custom_editors {
                    self.register_custom_editor_contribution(ce);
                }
                if let Some(views) = contribs.views {
                    for (container_id, view_list) in views {
                        self.register_view_contributions(&container_id, view_list);
                    }
                }
            }
        }
    }

    fn handle_builtin_command_effects(&mut self, command_id: &str) {
        match command_id {
            "workbench.action.toggleSidebar" => {
                self.view.toggle_sidebar();
            }
            "workbench.action.togglePanel" => {
                self.view.toggle_panel();
            }
            "workbench.action.terminal.toggleTerminal" => {
                if self.view.layout.panel_visible
                    && self.view.layout.active_panel_tab == "terminal"
                {
                    self.view.layout.mutate(|l| l.panel_visible = false);
                    if self.view.focused_region == FocusedRegion::Panel {
                        self.view.focused_region = FocusedRegion::Editor;
                    }
                    self.view.last_action = Some(WorkbenchAction::PanelToggled(false));
                } else {
                    let default_cwd = self
                        .view
                        .workspace
                        .folders()
                        .first()
                        .map(|f| f.path.clone());
                    self.view.layout.set_panel_tab("terminal");
                    self.view.terminal_panel.ensure_default_terminal(default_cwd);
                    self.view.terminal_panel.is_focused = true;
                    self.view.focused_region = FocusedRegion::Panel;
                    self.view.last_action = Some(WorkbenchAction::PanelToggled(true));
                }
            }
            "workbench.action.terminal.focus" => {
                let default_cwd = self
                    .view
                    .workspace
                    .folders()
                    .first()
                    .map(|f| f.path.clone());
                self.view.layout.set_panel_tab("terminal");
                self.view.terminal_panel.ensure_default_terminal(default_cwd);
                self.view.terminal_panel.is_focused = true;
                self.view.focused_region = FocusedRegion::Panel;
            }
            "workbench.action.terminal.new" => {
                let default_cwd = self
                    .view
                    .workspace
                    .folders()
                    .first()
                    .map(|f| f.path.clone());
                self.view.layout.set_panel_tab("terminal");
                self.view
                    .terminal_panel
                    .create_terminal(Some("sh"), None, default_cwd);
                self.view.focused_region = FocusedRegion::Panel;
            }
            "workbench.action.terminal.split" => {
                self.view.layout.set_panel_tab("terminal");
                self.view.terminal_panel.split_active_terminal();
                self.view.focused_region = FocusedRegion::Panel;
            }
            "workbench.action.terminal.kill" => {
                self.view.terminal_panel.kill_active_terminal();
                if !self.view.terminal_panel.has_active_terminal() {
                    self.view.layout.mutate(|l| l.panel_visible = false);
                    if self.view.focused_region == FocusedRegion::Panel {
                        self.view.focused_region = FocusedRegion::Editor;
                    }
                }
            }
            "workbench.action.output.toggleOutput" => {
                if self.view.layout.panel_visible && self.view.layout.active_panel_tab == "output" {
                    self.view.layout.mutate(|l| l.panel_visible = false);
                    if self.view.focused_region == FocusedRegion::Panel {
                        self.view.focused_region = FocusedRegion::Editor;
                    }
                    self.view.last_action = Some(WorkbenchAction::PanelToggled(false));
                } else {
                    self.view.layout.set_panel_tab("output");
                    self.view.focused_region = FocusedRegion::Panel;
                    self.view.last_action = Some(WorkbenchAction::PanelToggled(true));
                }
            }
            "workbench.action.output.clearOutput" => {
                self.sync_output_channels();
                if let Some(id) = self.view.output_panel.clear_active() {
                    if let Some(bridge) = &self.language_bridge {
                        if bridge.output_manager.clear(&id).is_ok() {
                            let _ = bridge.output_manager.take_pending_events();
                            self.synced_output_revision = bridge.output_manager.revision();
                        }
                    }
                }
            }
            "workbench.action.problems.focus" => {
                self.view.layout.mutate(|l| {
                    l.panel_visible = true;
                    l.set_panel_tab("problems");
                });
                self.view.focused_region = FocusedRegion::Panel;
            }
            "workbench.action.showCommands" => {
                self.open_command_palette();
            }
            "workbench.action.quickOpen" => {
                self.open_quick_open_files();
            }
            "workbench.action.showAllSymbols" => {
                self.open_quick_open_symbols();
            }
            "workbench.action.gotoLine" => {
                self.open_goto_line();
            }
            "workbench.action.gotoSymbol" => {
                self.open_goto_symbol();
            }
            "actions.find" => {
                self.view.focused_region = FocusedRegion::Editor;
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.open_find();
                }
                self.sync_focus_context_keys();
            }
            "editor.action.startFindReplaceAction" => {
                self.view.focused_region = FocusedRegion::Editor;
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.open_replace();
                }
                self.sync_focus_context_keys();
            }
            "workbench.action.findInFiles" => {
                self.view.layout.mutate(|l| {
                    l.active_sidebar_view = "search".to_string();
                    l.sidebar_visible = true;
                });
                self.view.activity_bar.set_active_index(1);
                self.view.search_view.is_replace_expanded = false;
                self.view.search_view.focused_input = SearchInputFocus::Query;
                self.view.focused_region = crate::workbench_view::FocusedRegion::Sidebar;
            }
            "workbench.action.replaceInFiles" => {
                self.view.layout.mutate(|l| {
                    l.active_sidebar_view = "search".to_string();
                    l.sidebar_visible = true;
                });
                self.view.activity_bar.set_active_index(1);
                self.view.search_view.is_replace_expanded = true;
                self.view.search_view.focused_input = if self.view.search_view.query.query.is_empty()
                {
                    SearchInputFocus::Query
                } else {
                    SearchInputFocus::Replace
                };
                self.view.focused_region = crate::workbench_view::FocusedRegion::Sidebar;
            }
            "workbench.view.explorer" => {
                self.view.layout.mutate(|l| {
                    l.active_sidebar_view = "workbench.view.explorer".to_string();
                    l.sidebar_visible = true;
                });
                self.view.focused_region = FocusedRegion::Sidebar;
                self.view.activity_bar.set_active_index(0);
            }
            "workbench.view.search" => {
                self.view.layout.mutate(|l| {
                    l.active_sidebar_view = "search".to_string();
                    l.sidebar_visible = true;
                });
                self.view.focused_region = FocusedRegion::Sidebar;
                self.view.activity_bar.set_active_index(1);
            }
            "workbench.view.scm" => {
                self.view.layout.mutate(|l| {
                    l.active_sidebar_view = "scm".to_string();
                    l.sidebar_visible = true;
                });
                self.view.focused_region = FocusedRegion::Sidebar;
                self.view.activity_bar.set_active_index(2);
            }
            "workbench.view.debug" => {
                self.view.layout.mutate(|l| {
                    l.active_sidebar_view = "debug".to_string();
                    l.sidebar_visible = true;
                });
                self.view.focused_region = FocusedRegion::Sidebar;
                self.view.activity_bar.set_active_index(3);
            }
            "workbench.view.extensions" => {
                self.view.layout.mutate(|l| {
                    l.active_sidebar_view = "extensions".to_string();
                    l.sidebar_visible = true;
                });
                self.view.focused_region = crate::workbench_view::FocusedRegion::Sidebar;
                self.view.activity_bar.set_active_index(4);
            }
            "workbench.extensions.action.showInstalledExtensions" => {
                self.view.layout.mutate(|l| {
                    l.active_sidebar_view = "extensions".to_string();
                    l.sidebar_visible = true;
                });
                self.view.focused_region = crate::workbench_view::FocusedRegion::Sidebar;
                self.view.activity_bar.set_active_index(4);
                self.view.extensions_view.apply_search_query("");
                self.view.extensions_view.installed_collapsed = false;
                self.view.extensions_view.refresh_installed();
            }
            "workbench.view.testing" => {
                self.view.layout.mutate(|l| {
                    l.active_sidebar_view = "workbench.view.testing".to_string();
                    l.sidebar_visible = true;
                });
                self.view.focused_region = FocusedRegion::Sidebar;
                if self.view.activity_bar.model.items.len() > 5 {
                    self.view.activity_bar.set_active_index(5);
                }
            }
            "workbench.action.files.openFile" | "workbench.action.openFile" => {
                self.open_file_dialog();
            }
            "workbench.action.files.openFolder" | "workbench.action.openFolder" => {
                self.open_folder_dialog();
            }
            "workbench.action.files.save" => {
                self.save_active_file();
            }
            "workbench.action.files.saveAs" | "workbench.action.saveAs" => {
                self.save_active_file_as();
            }
            "workbench.action.openSettings" => {
                self.open_settings();
            }
            "workbench.action.files.newUntitledNotebook" => {
                self.new_untitled_notebook();
            }
            "notebook.cell.execute" => {
                if let Some(nb) = self.view.editor_area.active_notebook_mut() {
                    nb.execute_active_cell();
                }
                self.view.editor_area.sync_dirty_states();
            }
            "notebook.execute" => {
                if let Some(nb) = self.view.editor_area.active_notebook_mut() {
                    nb.execute_all_cells();
                }
                self.view.editor_area.sync_dirty_states();
            }
            "workbench.action.closeActiveEditor" => {
                if let Some(active_tab) = self.view.editor_area.model.current_group_ref().active_tab_index {
                    self.view.editor_area.close_tab(active_tab);
                    self.sync_closed_webview_panels();
                    self.sync_document_lifecycle();
                }
            }
            "workbench.action.splitEditor" => {
                self.view
                    .editor_area
                    .split_active_group(SplitDirection::Horizontal);
                self.view.focused_region = FocusedRegion::Editor;
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
                self.wire_clipboard_on_open_editors();
            }
            "workbench.action.splitEditorDown" => {
                self.view
                    .editor_area
                    .split_active_group(SplitDirection::Vertical);
                self.view.focused_region = FocusedRegion::Editor;
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
                self.wire_clipboard_on_open_editors();
            }
            "workbench.action.focusPreviousGroup" => {
                self.view.editor_area.focus_previous_group();
                self.view.focused_region = FocusedRegion::Editor;
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
            }
            "workbench.action.focusNextGroup" => {
                self.view.editor_area.focus_next_group();
                self.view.focused_region = FocusedRegion::Editor;
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
            }
            "workbench.action.focusFirstEditorGroup" => {
                self.view.editor_area.focus_first_editor_group();
                self.view.focused_region = FocusedRegion::Editor;
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
            }
            "workbench.action.focusSecondEditorGroup" => {
                self.view.editor_area.focus_second_editor_group();
                self.view.focused_region = FocusedRegion::Editor;
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
            }
            "workbench.action.closeEditorsInGroup" => {
                self.view.editor_area.close_editors_in_active_group();
                self.sync_closed_webview_panels();
                self.view.focused_region = FocusedRegion::Editor;
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
            }
            "undo" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.undo();
                }
                self.view.editor_area.sync_dirty_states();
                self.sync_document_lifecycle();
            }
            "redo" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.redo();
                }
                self.view.editor_area.sync_dirty_states();
                self.sync_document_lifecycle();
            }
            "editor.action.triggerSuggest" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.trigger_suggest(None);
                }
                if let Some(pos) = self
                    .view
                    .editor_area
                    .active_editor()
                    .map(|ed| ed.primary_position())
                {
                    self.trigger_completions_at(pos, None);
                }
            }
            "editor.action.commentLine" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.toggle_line_comment();
                }
                self.view.editor_area.sync_dirty_states();
                self.sync_document_lifecycle();
            }
            "editor.action.revealDefinition" => {
                if let Some(pos) = self
                    .view
                    .editor_area
                    .active_editor()
                    .map(|ed| ed.primary_position())
                {
                    self.trigger_go_to_definition_at(pos);
                }
            }
            "editor.action.insertCursorAbove" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.insert_cursor_above();
                }
            }
            "editor.action.insertCursorBelow" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.insert_cursor_below();
                }
            }
            "editor.action.addSelectionToNextFindMatch" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.add_selection_to_next_find_match();
                }
            }
            "editor.fold" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.fold_at_cursor();
                }
            }
            "editor.unfold" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.unfold_at_cursor();
                }
            }
            "editor.foldAll" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.fold_all();
                }
            }
            "editor.unfoldAll" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.unfold_all();
                }
            }
            "editor.action.formatDocument" => {
                self.trigger_format_active_document(false);
            }
            "editor.action.quickFix" => {
                self.show_quick_fixes_palette();
            }
            cmd if cmd.starts_with("quickfix:") || cmd.starts_with("codeaction:") => {
                if let Some(idx_str) = cmd.strip_prefix("quickfix:").or_else(|| cmd.strip_prefix("codeaction:")) {
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        self.apply_code_action_by_index(idx);
                    }
                }
            }
            _ => {}
        }
    }

    fn format_command_palette_label(cmd_id: &str) -> String {
        match cmd_id {
            "workbench.view.extensions" => "View: Show Extensions".to_string(),
            "workbench.extensions.action.showInstalledExtensions" => {
                "Extensions: Show Installed".to_string()
            }
            "editor.action.startFindReplaceAction" => "Find and Replace".to_string(),
            _ => cmd_id
                .replace("workbench.action.files.", "")
                .replace("workbench.action.", "")
                .replace("editor.action.", ""),
        }
    }

    fn active_document_symbol_items(&self) -> Vec<PaletteItem> {
        let Some(editor) = self.view.editor_area.active_editor() else {
            return Vec::new();
        };
        let file_path = editor
            .document
            .as_ref()
            .and_then(|d| d.read().path.clone())
            .or_else(|| {
                self.view
                    .editor_area
                    .model
                    .current_group_ref()
                    .active_tab()
                    .map(|t| rustcode_platform::file_uri_to_path(&t.uri))
            });
        let Some(path) = file_path else {
            return Vec::new();
        };
        let content = editor.text();
        let symbols = crate::search::search_document_symbols(&content, &path);
        symbols
            .into_iter()
            .map(|s| {
                PaletteItem::document_symbol(
                    s.name,
                    s.container_name,
                    s.location.range.start.line as usize + 1,
                    s.location.range.start.character as usize + 1,
                )
            })
            .collect()
    }

    pub fn open_command_palette(&mut self) {
        let mut items = Vec::new();
        for cmd_id in self.commands.list_commands() {
            let keybinding = self.keybindings.lookup_keybinding(&cmd_id);
            items.push(PaletteItem::command(
                cmd_id.clone(),
                Self::format_command_palette_label(&cmd_id),
                Some("Command".to_string()),
                keybinding,
            ));
        }
        items.extend(self.active_document_symbol_items());

        let mut palette = CommandPalette::new();
        palette.set_items(items);
        palette.open_mode(crate::quickinput::QuickInputMode::Command);
        self.view.open_command_palette(CommandPaletteView::new(palette));
        self.sync_focus_context_keys();
    }

    pub fn open_quick_open_files(&mut self) {
        let mut items = Vec::new();

        let files = crate::search::search_files(self.view.workspace.folders(), "", 1000);
        for file in files {
            let folder_opt = self.view.workspace.get_folder_for_path(&file);
            let rel_label = folder_opt
                .and_then(|f| f.relative_path(&file))
                .unwrap_or_else(|| file.clone());
            items.push(PaletteItem::file(file, rel_label.to_string_lossy()));
        }

        for cmd_id in self.commands.list_commands() {
            let keybinding = self.keybindings.lookup_keybinding(&cmd_id);
            items.push(PaletteItem::command(
                cmd_id.clone(),
                Self::format_command_palette_label(&cmd_id),
                Some("Command".to_string()),
                keybinding,
            ));
        }

        items.extend(self.active_document_symbol_items());

        let mut palette = CommandPalette::new();
        palette.set_items(items);
        palette.open_mode(crate::quickinput::QuickInputMode::File);
        self.view.open_command_palette(CommandPaletteView::new(palette));
        self.sync_focus_context_keys();
    }

    pub fn open_quick_open_symbols(&mut self) {
        let mut items = Vec::new();
        let symbols = crate::search::search_workspace_symbols(self.view.workspace.folders(), "");
        for s in symbols {
            let path = rustcode_platform::file_uri_to_path(&s.location.uri);
            items.push(PaletteItem::symbol(
                s.name,
                s.container_name,
                path,
                s.location.range.start.line as usize + 1,
                s.location.range.start.character as usize + 1,
            ));
        }
        items.extend(self.active_document_symbol_items());

        let mut palette = CommandPalette::new();
        palette.set_items(items);
        palette.open_mode(crate::quickinput::QuickInputMode::WorkspaceSymbol);
        self.view.open_command_palette(CommandPaletteView::new(palette));
        self.sync_focus_context_keys();
    }

    pub fn open_goto_line(&mut self) {
        let mut items = Vec::new();
        items.extend(self.active_document_symbol_items());

        let mut palette = CommandPalette::new();
        palette.set_items(items);
        palette.open_mode(crate::quickinput::QuickInputMode::Line);
        self.view.open_command_palette(CommandPaletteView::new(palette));
        self.sync_focus_context_keys();
    }

    pub fn open_goto_symbol(&mut self) {
        let items = self.active_document_symbol_items();

        let mut palette = CommandPalette::new();
        palette.set_items(items);
        palette.open_mode(crate::quickinput::QuickInputMode::DocumentSymbol);
        self.view.open_command_palette(CommandPaletteView::new(palette));
        self.sync_focus_context_keys();
    }

    pub fn open_file(&mut self, uri: &str, title: &str, content: &str, is_preview: bool) {
        if let Some(contrib) = self.find_matching_custom_editor(uri) {
            if let Some(bridge) = self.language_bridge.clone() {
                let existing_session = bridge
                    .custom_editor_manager
                    .find_session_by_uri(uri, Some(&contrib.view_type))
                    .filter(|session| {
                        let valid = bridge
                            .webview_manager
                            .get_panel(&session.panel_id)
                            .is_some_and(|p| !p.is_disposed);
                        if !valid {
                            bridge
                                .custom_editor_manager
                                .close_session(&session.id, Some(&bridge.webview_manager));
                        }
                        valid
                    });
                let session = existing_session.unwrap_or_else(|| {
                    bridge.custom_editor_manager.open_session(
                        None,
                        &contrib.view_type,
                        uri,
                        Some(title),
                        Some(&bridge.webview_manager),
                    )
                });
                let (panel_html, options) = bridge
                    .webview_manager
                    .get_panel(&session.panel_id)
                    .map(|p| (p.html, p.options))
                    .unwrap_or_else(|| (String::new(), WebviewOptions::default()));
                let html = if panel_html.is_empty() {
                    let display = if contrib.display_name.is_empty() {
                        contrib.view_type.as_str()
                    } else {
                        contrib.display_name.as_str()
                    };
                    format!("<h1>{}</h1><p>Custom Editor: {}</p>", title, display)
                } else {
                    panel_html
                };
                self.view.editor_area.open_custom_editor(
                    uri,
                    title,
                    &contrib.view_type,
                    &session.panel_id,
                    &html,
                    options,
                    Some((*bridge.webview_manager).clone()),
                    is_preview,
                );
                self.sync_custom_editors();
                self.sync_webviews();
            } else {
                let panel_id = format!("custom_editor:{}", uri);
                let display = if contrib.display_name.is_empty() {
                    contrib.view_type.as_str()
                } else {
                    contrib.display_name.as_str()
                };
                let html = format!("<h1>{}</h1><p>Custom Editor: {}</p>", title, display);
                self.view.editor_area.open_custom_editor(
                    uri,
                    title,
                    &contrib.view_type,
                    &panel_id,
                    &html,
                    WebviewOptions::default(),
                    None,
                    is_preview,
                );
            }
        } else {
            self.view
                .editor_area
                .open_file(uri, title, content, is_preview);
            self.sync_closed_webview_panels();
        }
        self.view.focused_region = crate::workbench_view::FocusedRegion::Editor;
        self.sync_focus_context_keys();
        self.sync_configuration();
        self.wire_clipboard_on_open_editors();
        self.sync_document_lifecycle();
        if let Some((opened_uri, opened_lang)) = self.active_editor_uri_and_lang() {
            self.ensure_lsp_for_language_and_open(&opened_uri, &opened_lang);
        }
        self.sync_diagnostics();
    }

    pub fn save_active_file(&mut self) -> bool {
        let active_path = self
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .and_then(|t| {
                if rustcode_platform::is_file_uri(&t.uri) || !t.uri.contains("://") {
                    Some(rustcode_platform::file_uri_to_path(&t.uri))
                } else {
                    None
                }
            });
        let format_on_save = self
            .config
            .get_for_resource::<bool>("editor.formatOnSave", active_path.as_deref())
            .unwrap_or(false);
        if format_on_save && self.language_bridge.is_some() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
                    let handle_clone = handle.clone();
                    let _ = tokio::task::block_in_place(|| {
                        handle_clone.block_on(tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            self.format_active_document(),
                        ))
                    });
                } else {
                    self.trigger_format_active_document(true);
                }
            }
        }
        self.save_active_file_without_format()
    }

    pub async fn save_active_file_async(&mut self) -> bool {
        let active_path = self
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .and_then(|t| {
                if rustcode_platform::is_file_uri(&t.uri) || !t.uri.contains("://") {
                    Some(rustcode_platform::file_uri_to_path(&t.uri))
                } else {
                    None
                }
            });
        let format_on_save = self
            .config
            .get_for_resource::<bool>("editor.formatOnSave", active_path.as_deref())
            .unwrap_or(false);
        if format_on_save && self.language_bridge.is_some() {
            let _ = self.format_active_document().await;
        }
        self.save_active_file_without_format()
    }

    pub async fn format_active_document(&mut self) -> bool {
        let Some((uri, lang_id)) = self.active_editor_uri_and_lang() else {
            return false;
        };

        if let Some(bridge) = self.language_bridge.clone() {
            let options = self.formatting_options_for_uri(&uri);
            let edits = bridge.request_formatting(&uri, &lang_id, options).await;

            if !edits.is_empty() {
                return self.apply_lsp_text_edits_to_uri(&uri, edits);
            }
        }
        false
    }

    pub fn show_quick_fixes_palette(&mut self) -> bool {
        if self.pending_code_actions.is_empty() {
            return false;
        }

        let mut items = Vec::new();
        for (idx, action) in self.pending_code_actions.iter().enumerate() {
            let category = action
                .kind
                .clone()
                .unwrap_or_else(|| "Quick Fix".to_string());
            items.push(PaletteItem {
                command_id: format!("quickfix:{}", idx),
                label: action.title.clone(),
                category: Some(category),
                keybinding: None,
                path: None,
                line: None,
                column: None,
                item_kind: crate::quickinput::PaletteItemKind::Command,
            });
        }

        let mut palette = CommandPalette::new();
        palette.set_items(items);
        palette.open_mode(crate::quickinput::QuickInputMode::Command);
        self.view.open_command_palette(CommandPaletteView::new(palette));
        self.sync_focus_context_keys();
        true
    }

    pub async fn show_quick_fixes(&mut self) -> bool {
        let (uri, lang_id, range) = match self.view.editor_area.model.current_group_ref().active_tab() {
            Some(tab) => {
                let uri = tab.uri.clone();
                let (lang, range) = if let Some(ed) = self.view.editor_area.active_editor() {
                    let lang = ed
                        .document
                        .as_ref()
                        .map(|doc| doc.read().language_id.clone())
                        .unwrap_or_else(|| "*".to_string());
                    let sel = ed.cursor_state.primary();
                    let start = sel.anchor.min(sel.active);
                    let end = sel.anchor.max(sel.active);
                    let range = LspRange::new(
                        LspPosition::new(start.line as u32, start.column as u32),
                        LspPosition::new(end.line as u32, end.column as u32),
                    );
                    (lang, range)
                } else {
                    (
                        "*".to_string(),
                        LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 0)),
                    )
                };
                (uri, lang, range)
            }
            None => return false,
        };

        if let Some(bridge) = self.language_bridge.clone() {
            let diags = bridge.get_diagnostics(&uri);
            let context = CodeActionContext::new(diags);
            let actions = bridge.request_code_actions(&uri, &lang_id, range, context).await;
            if !actions.is_empty() {
                self.pending_code_actions = actions;
                return self.show_quick_fixes_palette();
            }
        }

        self.show_quick_fixes_palette()
    }

    pub fn apply_code_action_by_index(&mut self, idx: usize) -> bool {
        if let Some(action) = self.pending_code_actions.get(idx).cloned() {
            self.apply_code_action(&action)
        } else {
            false
        }
    }

    pub fn apply_code_action(&mut self, action: &CodeAction) -> bool {
        let mut executed = false;
        if let Some(edit) = &action.edit {
            self.view.editor_area.apply_workspace_edit(edit);
            executed = true;
        }
        if let Some(cmd) = &action.command {
            let args = cmd.arguments.clone().map(serde_json::Value::Array);
            let _ = self.commands.execute(&cmd.command, args);
            self.handle_builtin_command_effects(&cmd.command);
            executed = true;
        }
        if executed {
            self.sync_document_lifecycle();
            self.sync_configuration();
        }
        executed
    }

    pub fn apply_workspace_edit(&mut self, edit: &WorkspaceEdit) -> usize {
        let count = self.view.editor_area.apply_workspace_edit(edit);
        if count > 0 {
            self.sync_document_lifecycle();
            self.sync_configuration();
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{KeyCode, KeyEvent, MockRenderSurface};

    #[test]
    fn test_workbench_window_rendering_and_keybinding_dispatch() {
        let theme = ThemeService::new();
        let keybindings = KeybindingService::new();
        let commands = CommandRegistry::new();
        let context_keys = ContextKeyService::new();

        // Register built-in commands
        let _sub1 = commands.register("workbench.action.toggleSidebar", |_| {
            Ok(serde_json::json!({ "toggled": true }))
        });
        let _sub2 = commands.register("workbench.action.showCommands", |_| {
            Ok(serde_json::json!({ "palette": true }))
        });

        // Load keybinding Ctrl+B -> toggleSidebar
        let keymap_json = r#"[
            { "key": "ctrl+b", "command": "workbench.action.toggleSidebar" },
            { "key": "ctrl+shift+p", "command": "workbench.action.showCommands" }
        ]"#;
        keybindings.load_keybindings_json(keymap_json).unwrap();

        let mut window = WorkbenchWindow::new(theme, keybindings, commands, context_keys);
        let mut surface = MockRenderSurface::new(1024.0, 768.0);

        // Render window
        let scene = window.render(&mut surface);
        assert!(!scene.quads().is_empty());
        assert_eq!(surface.rendered_scenes.len(), 1);

        // Verify initial sidebar is visible
        assert!(window.view.layout.sidebar_visible);

        // Dispatch Ctrl+B key event
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let ctrl_b = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('b'),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        let res = window.dispatch_event(&ctrl_b, bounds);
        assert_eq!(res, EventResult::Consumed);

        // Sidebar should now be hidden!
        assert!(!window.view.layout.sidebar_visible);

        // Dispatch Ctrl+Shift+P key event
        let ctrl_shift_p = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('p'),
            Modifiers {
                ctrl: true,
                shift: true,
                alt: false,
                meta: false,
            },
        ));
        let res2 = window.dispatch_event(&ctrl_shift_p, bounds);
        assert_eq!(res2, EventResult::Consumed);

        // Command Palette should now be open!
        assert!(window.view.is_command_palette_open);

        // Verify keybinding badge was resolved for showCommands
        let palette = window.view.command_palette.as_ref().unwrap();
        let show_cmd_item = palette
            .model
            .items
            .iter()
            .find(|i| i.command_id == "workbench.action.showCommands");
        assert!(show_cmd_item.is_some());
        assert_eq!(
            show_cmd_item.unwrap().keybinding,
            Some("Ctrl+Shift+P".to_string())
        );

        // Close command palette with Escape
        let esc_ev = InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none()));
        window.dispatch_event(&esc_ev, bounds);
        assert!(!window.view.is_command_palette_open);

        // Open a file tab and verify Ctrl+W closes it
        window.open_file("file:///test.rs", "test.rs", "fn test() {}", false);
        assert_eq!(window.view.editor_area.model.current_group_ref().tabs.len(), 1);

        // Register workbench.action.closeActiveEditor
        let keymap2 = r#"[
            { "key": "ctrl+w", "command": "workbench.action.closeActiveEditor" }
        ]"#;
        window.keybindings.load_keybindings_json(keymap2).unwrap();

        let ctrl_w = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('w'),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        let res3 = window.dispatch_event(&ctrl_w, bounds);
        assert_eq!(res3, EventResult::Consumed);
        assert_eq!(window.view.editor_area.model.current_group_ref().tabs.len(), 0);
    }

    #[tokio::test]
    async fn test_workbench_window_formatting_and_quickfix_commands() {
        use rustcode_ext_host::provider::{ProviderRegistration, ProviderRegistry, ProviderType};
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_protocols::lsp::LspClientManager;

        let reg = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp_mgr = Arc::new(LspClientManager::new());

        // Register formatting provider
        reg.register(ProviderRegistration {
            id: "test-formatter".to_string(),
            provider_type: ProviderType::DocumentFormatting,
            language_selector: Some("rust".to_string()),
        });

        // Register code action provider
        reg.register(ProviderRegistration {
            id: "test-codeaction".to_string(),
            provider_type: ProviderType::CodeAction,
            language_selector: Some("rust".to_string()),
        });

        let bridge = Arc::new(LanguageFeatureBridge::new(reg, rpc.clone(), lsp_mgr));

        let mut window = WorkbenchWindow::default().with_language_bridge(bridge);

        window.open_file("file:///test.rs", "test.rs", "fn foo(){\nlet x=1;\n}", false);

        // 1. Test format document command
        let rpc_clone = rpc.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            rpc_clone.handle_incoming_response(
                1,
                Some(serde_json::json!([
                    {
                        "range": {
                            "start": { "line": 0, "character": 8 },
                            "end": { "line": 0, "character": 9 }
                        },
                        "newText": " {"
                    },
                    {
                        "range": {
                            "start": { "line": 1, "character": 0 },
                            "end": { "line": 1, "character": 8 }
                        },
                        "newText": "    let x = 1;"
                    }
                ])),
                None,
            );
        });

        let formatted = window.format_active_document().await;
        assert!(formatted);
        assert_eq!(
            window.view.editor_area.editors.get("file:///test.rs").unwrap().text(),
            "fn foo() {\n    let x = 1;\n}"
        );
        assert!(window.view.editor_area.model.current_group_ref().tabs[0].is_dirty);

        // 2. Test quick fix command
        let rpc_clone2 = rpc.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            rpc_clone2.handle_incoming_response(
                2,
                Some(serde_json::json!([
                    {
                        "title": "Make x mutable",
                        "kind": "quickfix",
                        "edit": {
                            "changes": {
                                "file:///test.rs": [
                                    {
                                        "range": {
                                            "start": { "line": 1, "character": 4 },
                                            "end": { "line": 1, "character": 9 }
                                        },
                                        "newText": "let mut x"
                                    }
                                ]
                            }
                        }
                    }
                ])),
                None,
            );
        });

        let ok = window.show_quick_fixes().await;
        assert!(ok);
        assert!(window.view.is_command_palette_open);
        assert_eq!(window.pending_code_actions.len(), 1);
        assert_eq!(window.pending_code_actions[0].title, "Make x mutable");

        // Apply quick fix by index
        let applied = window.apply_code_action_by_index(0);
        assert!(applied);
        assert_eq!(
            window.view.editor_area.editors.get("file:///test.rs").unwrap().text(),
            "fn foo() {\n    let mut x = 1;\n}"
        );
    }

    #[test]
    fn test_workbench_window_settings_ui_shortcut_palette_sync_and_close() {
        use crate::settings_ui::SettingsHitTarget;
        use rustcode_platform::render::{MouseButton, MouseEvent, Point};

        let mut window = WorkbenchWindow::default();
        let _sub_open = window
            .commands
            .register("workbench.action.openSettings", |_| {
                Ok(serde_json::json!({ "opened": "settings" }))
            });
        let _sub_pal = window
            .commands
            .register("workbench.action.showCommands", |_| {
                Ok(serde_json::json!({ "palette": true }))
            });
        let _sub_close = window
            .commands
            .register("workbench.action.closeActiveEditor", |_| {
                Ok(serde_json::json!({ "closed": true }))
            });

        window
            .keybindings
            .load_keybindings_json(include_str!("../../../resources/default-keymaps/linux.json"))
            .unwrap();

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let mut surface = MockRenderSurface::new(1024.0, 768.0);

        // Start focused on sidebar to ensure Ctrl+, switches focus + context keys
        window.view.focused_region = crate::workbench_view::FocusedRegion::Sidebar;
        window.context_keys.set_bool("sideBarFocus", true);
        window.context_keys.set_bool("editorTextFocus", false);

        // 1. Ctrl+, opens Settings tab
        let ctrl_comma = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char(','),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        let res = window.dispatch_event(&ctrl_comma, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(
            window.view.focused_region,
            crate::workbench_view::FocusedRegion::Editor
        );
        assert_eq!(
            window.context_keys.get("editorTextFocus"),
            Some(serde_json::Value::Bool(true))
        );
        assert_eq!(
            window.context_keys.get("sideBarFocus"),
            Some(serde_json::Value::Bool(false))
        );
        assert_eq!(
            window
                .view
                .editor_area
                .model
                .current_group_ref()
                .active_tab()
                .unwrap()
                .uri,
            "rustcode://settings"
        );

        // Render and verify Settings UI content
        let scene = window.render(&mut surface);
        assert!(scene.contains_text("Settings"));
        assert!(scene.contains_text("Search settings..."));

        // 2. Interactive edit: search "minimap", render, click toggle, check window.config
        let search_rect = window
            .view
            .editor_area
            .settings_views
            .get("rustcode://settings")
            .unwrap()
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::SearchBar(r) => Some(*r),
                _ => None,
            })
            .unwrap();

        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(search_rect.min_x() + 8.0, search_rect.min_y() + 8.0),
                MouseButton::Left,
            )),
            bounds,
        );
        for ch in "minimap".chars() {
            window.dispatch_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        window.render(&mut surface);

        let toggle_rect = window
            .view
            .editor_area
            .settings_views
            .get("rustcode://settings")
            .unwrap()
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::ToggleBool { key, rect } if key == "editor.minimap.enabled" => {
                    Some(*rect)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(window.config.get::<bool>("editor.minimap.enabled"), Some(true));
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(toggle_rect.min_x() + 5.0, toggle_rect.min_y() + 5.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(
            window.config.get::<bool>("editor.minimap.enabled"),
            Some(false)
        );

        // 3. Ctrl+W closes Settings tab and cleans up settings_views
        let ctrl_w = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('w'),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        let res_close = window.dispatch_event(&ctrl_w, bounds);
        assert_eq!(res_close, EventResult::Consumed);
        assert!(window
            .view
            .editor_area
            .model
            .current_group_ref()
            .tabs
            .is_empty());
        assert!(window.view.editor_area.settings_views.is_empty());

        // 4. Open via Command Palette ("openSettings")
        window.open_command_palette();
        assert!(window.view.is_command_palette_open);
        for ch in "openSettings".chars() {
            window.dispatch_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        let enter_ev = InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none()));
        window.dispatch_event(&enter_ev, bounds);
        assert!(!window.view.is_command_palette_open);
        assert_eq!(
            window
                .view
                .editor_area
                .model
                .current_group_ref()
                .active_tab()
                .unwrap()
                .uri,
            "rustcode://settings"
        );
        // Re-opening hydrated the updated config value (false)
        assert_eq!(
            window
                .view
                .editor_area
                .settings_views
                .get("rustcode://settings")
                .unwrap()
                .state
                .all_items
                .iter()
                .find(|i| i.key == "editor.minimap.enabled")
                .unwrap()
                .current_value,
            serde_json::Value::Bool(false)
        );
    }

    #[test]
    fn test_workbench_window_notebook_focus_context_key_on_keybinding_return() {
        let mut window = WorkbenchWindow::default();
        let _sub_close = window
            .commands
            .register("workbench.action.closeActiveEditor", |_| {
                Ok(serde_json::json!({ "closed": true }))
            });
        let _sub_settings = window
            .commands
            .register("workbench.action.openSettings", |_| {
                Ok(serde_json::json!({ "opened": "settings" }))
            });
        window
            .keybindings
            .load_keybindings_json(include_str!("../../../resources/default-keymaps/linux.json"))
            .unwrap();

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);

        window.new_untitled_notebook();
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(true))
        );

        // Ctrl+, opens Settings via keybinding early return -> notebookEditorFocused becomes false
        let ctrl_comma = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char(','),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        assert_eq!(window.dispatch_event(&ctrl_comma, bounds), EventResult::Consumed);
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(false))
        );

        // Re-open notebook, then close via Ctrl+W keybinding early return -> false
        window.new_untitled_notebook();
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(true))
        );
        let ctrl_w = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('w'),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        assert_eq!(window.dispatch_event(&ctrl_w, bounds), EventResult::Consumed);
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(false))
        );
    }

    #[test]
    fn test_workbench_window_open_file_and_palette_syncs_context_keys() {
        use rustcode_base::context_key::ContextKeyExpr;

        let mut window = WorkbenchWindow::default();
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let nb_focused_expr = ContextKeyExpr::parse("notebookEditorFocused").unwrap();
        let in_palette_expr = ContextKeyExpr::parse("inCommandPalette").unwrap();

        // Start focused on Sidebar to verify open_file focuses Editor and syncs context keys
        window.view.focused_region = crate::workbench_view::FocusedRegion::Sidebar;
        window.context_keys.set_bool("sideBarFocus", true);
        window.context_keys.set_bool("editorTextFocus", false);

        // 1. Fresh window open_file with .ipynb sets notebookEditorFocused == true and focuses Editor
        window.open_file("file:///probe.ipynb", "probe.ipynb", "", false);
        assert_eq!(
            window.view.focused_region,
            crate::workbench_view::FocusedRegion::Editor
        );
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(true))
        );
        assert_eq!(
            window.context_keys.get("editorTextFocus"),
            Some(serde_json::Value::Bool(true))
        );
        assert_eq!(
            window.context_keys.get("sideBarFocus"),
            Some(serde_json::Value::Bool(false))
        );
        assert_eq!(
            window.context_keys.get("inCommandPalette"),
            Some(serde_json::Value::Bool(false))
        );
        assert!(nb_focused_expr.evaluate(&window.context_keys));

        // 2. Switching to a regular .rs file clears notebookEditorFocused to false
        window.open_file("file:///probe.rs", "probe.rs", "fn main() {}", false);
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(false))
        );
        assert!(!nb_focused_expr.evaluate(&window.context_keys));

        // 3. Switch back to the notebook tab via open_file -> notebookEditorFocused == true
        window.open_file("file:///probe.ipynb", "probe.ipynb", "", false);
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(true))
        );
        assert!(nb_focused_expr.evaluate(&window.context_keys));

        // 4. open_command_palette sets inCommandPalette == true and notebookEditorFocused == false;
        //    closing via Escape restores inCommandPalette == false and notebookEditorFocused == true.
        let esc_ev = InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none()));

        window.open_command_palette();
        assert!(window.view.is_command_palette_open);
        assert_eq!(
            window.context_keys.get("inCommandPalette"),
            Some(serde_json::Value::Bool(true))
        );
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(false))
        );
        assert!(in_palette_expr.evaluate(&window.context_keys));
        assert!(!nb_focused_expr.evaluate(&window.context_keys));

        window.dispatch_event(&esc_ev, bounds);
        assert!(!window.view.is_command_palette_open);
        assert_eq!(
            window.context_keys.get("inCommandPalette"),
            Some(serde_json::Value::Bool(false))
        );
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(true))
        );
        assert!(!in_palette_expr.evaluate(&window.context_keys));
        assert!(nb_focused_expr.evaluate(&window.context_keys));

        // 5. open_quick_open_files, open_quick_open_symbols, and show_quick_fixes_palette
        //    also sync inCommandPalette/notebookEditorFocused on open and reset on close.
        window.open_quick_open_files();
        assert_eq!(
            window.context_keys.get("inCommandPalette"),
            Some(serde_json::Value::Bool(true))
        );
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(false))
        );
        window.dispatch_event(&esc_ev, bounds);
        assert_eq!(
            window.context_keys.get("inCommandPalette"),
            Some(serde_json::Value::Bool(false))
        );

        window.open_quick_open_symbols();
        assert_eq!(
            window.context_keys.get("inCommandPalette"),
            Some(serde_json::Value::Bool(true))
        );
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(false))
        );
        window.dispatch_event(&esc_ev, bounds);
        assert_eq!(
            window.context_keys.get("inCommandPalette"),
            Some(serde_json::Value::Bool(false))
        );

        window.pending_code_actions.push(CodeAction {
            title: "Dummy Fix".to_string(),
            kind: Some("quickfix".to_string()),
            diagnostics: None,
            is_preferred: None,
            edit: None,
            command: None,
        });
        assert!(window.show_quick_fixes_palette());
        assert_eq!(
            window.context_keys.get("inCommandPalette"),
            Some(serde_json::Value::Bool(true))
        );
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(false))
        );
        window.dispatch_event(&esc_ev, bounds);
        assert_eq!(
            window.context_keys.get("inCommandPalette"),
            Some(serde_json::Value::Bool(false))
        );
        assert_eq!(
            window.context_keys.get("notebookEditorFocused"),
            Some(serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn test_workbench_window_extensions_commands_and_actions() {
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{KeyCode, KeyEvent, MockRenderSurface};

        let mut window = WorkbenchWindow::default();
        let _ = window.keybindings.load_keybindings_json(
            r#"[{"key": "ctrl+shift+x", "command": "workbench.view.extensions"}]"#,
        );
        window.commands.register("workbench.view.extensions", |_| {
            Ok(serde_json::json!({ "view": "extensions" }))
        });
        window
            .commands
            .register("workbench.extensions.action.showInstalledExtensions", |_| {
                Ok(serde_json::json!({ "view": "extensions", "filter": "installed" }))
            });

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let ctrl_shift_x = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('x'),
            Modifiers {
                ctrl: true,
                shift: true,
                alt: false,
                meta: false,
            },
        ));
        assert_eq!(
            window.dispatch_event(&ctrl_shift_x, bounds),
            EventResult::Consumed
        );
        assert_eq!(window.view.layout.active_sidebar_view, "extensions");
        assert!(window.view.layout.sidebar_visible);
        assert_eq!(
            window.view.focused_region,
            crate::workbench_view::FocusedRegion::Sidebar
        );
        assert_eq!(window.view.activity_bar.model.active_item_index, 4);
        assert_eq!(
            window.context_keys.get("sideBarFocus"),
            Some(serde_json::Value::Bool(true))
        );

        // Re-triggering workbench.view.extensions keeps sidebar open (does NOT toggle closed)
        assert_eq!(
            window.dispatch_event(&ctrl_shift_x, bounds),
            EventResult::Consumed
        );
        assert!(window.view.layout.sidebar_visible);
        assert_eq!(window.view.layout.active_sidebar_view, "extensions");

        let mut surface = MockRenderSurface::new(1024.0, 768.0);
        let scene = window.render(&mut surface);
        assert!(scene.contains_text("EXTENSIONS"));
        assert!(!scene.contains_text("EXPLORER"));

        // showInstalledExtensions opens installed section and clears search query + live results
        window.view.extensions_view.search_query = "stale".to_string();
        window.view.extensions_view.set_marketplace_results(
            rustcode_ext_host::marketplace::SearchResponse {
                total_size: 1,
                extensions: vec![rustcode_ext_host::marketplace::ExtensionMetadata {
                    namespace: "only".to_string(),
                    name: "one".to_string(),
                    version: "1.0.0".to_string(),
                    display_name: Some("Only One".to_string()),
                    description: None,
                    download_url: None,
                    files: None,
                    all_versions: None,
                }],
            },
        );
        assert_eq!(window.view.extensions_view.marketplace_results.len(), 1);
        window.view.extensions_view.installed_collapsed = true;
        window.handle_builtin_command_effects("workbench.extensions.action.showInstalledExtensions");
        assert_eq!(window.view.extensions_view.search_query, "");
        assert!(!window.view.extensions_view.installed_collapsed);
        assert_eq!(
            window.view.extensions_view.marketplace_results.len(),
            window.view.extensions_view.catalog.len()
        );
    }

    #[test]
    fn test_reinstall_preserves_registered_extension_commands() {
        use rustcode_ext_host::marketplace::{MarketplaceClient, MockHttpTransport};

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("rustcode_win_reinstall_{}_{}", std::process::id(), now));
        let ext_dir = temp_dir.join("pub.reinstall-1.0.0");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("package.json"),
            r#"{
                "name": "reinstall",
                "publisher": "pub",
                "version": "1.0.0",
                "contributes": {
                    "commands": [
                        { "command": "reinstall.ping", "title": "Ping" }
                    ]
                }
            }"#,
        )
        .unwrap();

        let transport = Arc::new(MockHttpTransport::new());
        let client = Arc::new(MarketplaceClient::new("https://open-vsx.org/api", transport));
        let installer = Arc::new(ExtensionInstaller::new(client, &temp_dir));

        let mut window = WorkbenchWindow::default();
        window.view.extensions_view.set_installer(installer);

        // Register once, then re-register (simulating re-install/upgrade)
        window.register_extension_contributions_by_id("pub.reinstall");
        assert!(window.commands.has_command("reinstall.ping"));

        window.register_extension_contributions_by_id("pub.reinstall");
        assert!(
            window.commands.has_command("reinstall.ping"),
            "old DisposableStore must be dropped before registering new commands"
        );
        assert!(window.commands.execute("reinstall.ping", None).is_ok());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_window_sync_configuration_minimap_breadcrumbs_and_accessibility() {
        use rustcode_platform::accessibility::AccessibleRole;
        use rustcode_platform::configuration::ConfigurationTarget;

        let config = ConfigurationService::new();
        config
            .update("editor.minimap.enabled", serde_json::json!(false), ConfigurationTarget::User)
            .unwrap();
        config
            .update("breadcrumbs.enabled", serde_json::json!(false), ConfigurationTarget::User)
            .unwrap();

        let mut window = WorkbenchWindow::default().with_config(config.clone());
        assert!(!window.view.editor_area.minimap_enabled);
        assert!(!window.view.editor_area.breadcrumbs_enabled);

        window.open_file("file:///workspace/lib.rs", "lib.rs", "pub fn demo() {}\n", false);
        assert!(!window.view.editor_area.active_editor().unwrap().minimap_enabled);

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let tree_disabled = window.accessibility_tree(bounds);
        assert!(tree_disabled.find_by_role(AccessibleRole::Breadcrumb).is_empty());
        assert!(tree_disabled.find_by_role(AccessibleRole::ScrollBar).is_empty());

        // Re-enable both and sync
        config
            .update("editor.minimap.enabled", serde_json::json!(true), ConfigurationTarget::User)
            .unwrap();
        config
            .update("breadcrumbs.enabled", serde_json::json!(true), ConfigurationTarget::User)
            .unwrap();
        window.sync_configuration();

        assert!(window.view.editor_area.active_editor().unwrap().minimap_enabled);
        assert!(window.view.editor_area.breadcrumbs_enabled);

        let tree_enabled = window.accessibility_tree(bounds);
        assert!(!tree_enabled.find_by_role(AccessibleRole::Breadcrumb).is_empty());
        assert!(!tree_enabled.find_by_role(AccessibleRole::ScrollBar).is_empty());
    }

    #[test]
    fn test_window_terminal_output_sync_and_focus_context_keys() {
        use rustcode_ext_host::provider::{LanguageFeatureBridge, ProviderRegistry};
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_protocols::lsp::LspClientManager;

        let registry = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp));

        let mut window = WorkbenchWindow::default().with_language_bridge(bridge.clone());
        assert_eq!(window.context_keys.get("panelFocus"), Some(serde_json::json!(false)));
        assert_eq!(window.context_keys.get("terminalFocus"), Some(serde_json::json!(false)));

        // Toggle terminal open
        window.handle_builtin_command_effects("workbench.action.terminal.toggleTerminal");
        window.sync_focus_context_keys();
        assert!(window.view.layout.panel_visible);
        assert_eq!(window.view.layout.active_panel_tab, "terminal");
        assert_eq!(window.context_keys.get("panelFocus"), Some(serde_json::json!(true)));
        assert_eq!(window.context_keys.get("terminalFocus"), Some(serde_json::json!(true)));
        assert_eq!(window.context_keys.get("editorTextFocus"), Some(serde_json::json!(false)));

        // Split & kill
        window.handle_builtin_command_effects("workbench.action.terminal.split");
        assert_eq!(window.view.terminal_panel.manager.active_tab_terminals().len(), 2);
        window.handle_builtin_command_effects("workbench.action.terminal.kill");
        assert_eq!(window.view.terminal_panel.manager.active_tab_terminals().len(), 1);

        // Output channel sync + bidirectional clear
        bridge
            .output_manager
            .create_channel("out_1", "Build", Some("log".to_string()), false);
        bridge.output_manager.append("out_1", "line 1\n").unwrap();
        bridge.output_manager.show("out_1", false).unwrap();
        window.sync_output_channels();

        assert_eq!(window.view.layout.active_panel_tab, "output");
        assert_eq!(window.context_keys.get("panelFocus"), Some(serde_json::json!(true)));
        assert_eq!(window.context_keys.get("terminalFocus"), Some(serde_json::json!(false)));
        assert_eq!(window.view.output_panel.display_lines(), vec!["line 1"]);

        window.handle_builtin_command_effects("workbench.action.output.clearOutput");
        assert!(window.view.output_panel.display_lines().is_empty());
        assert!(bridge.output_manager.get_channel("out_1").unwrap().lines().is_empty());

        // Re-syncing must not resurrect cleared lines
        window.sync_output_channels();
        assert!(window.view.output_panel.display_lines().is_empty());

        // ExtHostTerminalManager sync
        bridge
            .terminal_manager
            .create_terminal("ext_t1", "Ext", None, Vec::new(), None);
        bridge
            .terminal_manager
            .send_text("ext_t1", "echo synced_term", true)
            .unwrap();
        bridge.terminal_manager.show_terminal("ext_t1", false).unwrap();
        window.sync_ext_terminals();
        assert_eq!(window.view.layout.active_panel_tab, "terminal");
        assert!(window
            .view
            .terminal_panel
            .manager
            .active_terminal()
            .unwrap()
            .lines
            .iter()
            .any(|l| l.contains("synced_term")));
    }

    #[test]
    fn test_window_output_sync_preserves_scroll_top_and_clear_preserves_pending_events() {
        use rustcode_ext_host::provider::{LanguageFeatureBridge, ProviderRegistry};
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_platform::render::{MockRenderSurface, MouseButton, MouseEvent, Point};
        use rustcode_protocols::lsp::LspClientManager;

        let registry = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp));

        bridge
            .output_manager
            .create_channel("out_a", "Alpha", Some("log".to_string()), false);
        for i in 0..60 {
            bridge
                .output_manager
                .append("out_a", &format!("line {}\n", i))
                .unwrap();
        }
        bridge.output_manager.show("out_a", false).unwrap();

        let mut window = WorkbenchWindow::default().with_language_bridge(bridge.clone());
        window.view.output_panel.auto_scroll = false;
        window.view.output_panel.scroll_top = 123.5;

        // Appending more output while auto_scroll == false must NOT reset scroll_top to 0.0
        bridge
            .output_manager
            .append("out_a", "streamed line\n")
            .unwrap();
        window.sync_output_channels();
        assert!(!window.view.output_panel.auto_scroll);
        assert_eq!(window.view.output_panel.scroll_top, 123.5);
        assert!(window
            .view
            .output_panel
            .display_lines()
            .contains(&"streamed line"));

        // Creating & appending to a second channel before clicking ClearButton on out_a must NOT drop out_b
        let mut surface = MockRenderSurface::new(1024.0, 768.0);
        let _ = window.render(&mut surface);
        let clear_rect = window
            .view
            .output_panel
            .click_targets
            .read()
            .iter()
            .find(|(_, t)| *t == crate::output_panel::OutputClickTarget::ClearButton)
            .unwrap()
            .0;
        let clear_center = Point::new(
            clear_rect.min_x() + clear_rect.width() / 2.0,
            clear_rect.min_y() + clear_rect.height() / 2.0,
        );

        bridge
            .output_manager
            .create_channel("out_b", "Beta", None, false);
        bridge
            .output_manager
            .append("out_b", "lint ok\n")
            .unwrap();

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(clear_center, MouseButton::Left)),
            bounds,
        );
        assert!(bridge
            .output_manager
            .get_channel("out_a")
            .unwrap()
            .lines()
            .is_empty());
        let out_b = window
            .view
            .output_panel
            .channels
            .iter()
            .find(|c| c.id == "out_b")
            .expect("out_b must not be dropped by ClearButton");
        assert_eq!(out_b.lines, vec!["lint ok"]);

        // Similarly, workbench.action.output.clearOutput must preserve pending events on out_b
        bridge
            .output_manager
            .append("out_a", "before clearOutput\n")
            .unwrap();
        window.sync_output_channels();
        bridge
            .output_manager
            .append("out_b", "second beta line\n")
            .unwrap();
        window.handle_builtin_command_effects("workbench.action.output.clearOutput");
        let out_b_after = window
            .view
            .output_panel
            .channels
            .iter()
            .find(|c| c.id == "out_b")
            .unwrap();
        assert_eq!(out_b_after.lines, vec!["lint ok", "second beta line"]);
    }

    #[test]
    fn test_window_sync_word_wrap_configuration_and_preserve_editor_state() {
        use rustcode_platform::configuration::ConfigurationTarget;
        use serde_json::json;

        let config = ConfigurationService::new();
        let mut window = WorkbenchWindow::new(
            ThemeService::new(),
            KeybindingService::new(),
            CommandRegistry::new(),
            ContextKeyService::new(),
        )
        .with_config(config.clone());

        window.open_file(
            "file:///workspace/main.rs",
            "main.rs",
            "fn demo() {\n    println!(\"hello world\");\n}\nlet after = 1;",
            false,
        );
        {
            let ed = window.view.editor_area.active_editor_mut().unwrap();
            ed.scroll_top = 1.0;
            assert!(ed.folding_model.fold(0));
        }

        config
            .update("editor.wordWrap", json!("wordWrapColumn"), ConfigurationTarget::User)
            .unwrap();
        config
            .update("editor.wordWrapColumn", json!(40), ConfigurationTarget::User)
            .unwrap();
        window.sync_configuration();

        let ed = window.view.editor_area.active_editor().unwrap();
        assert_eq!(ed.word_wrap, WordWrapMode::WordWrapColumn);
        assert_eq!(ed.word_wrap_column, 40);
        assert_eq!(ed.scroll_top, 1.0);
        assert!(ed.folding_model.is_collapsed(0));
    }

    #[test]
    fn test_window_custom_editor_open_file_and_webview_sync() {
        use rustcode_ext_host::manifest::CustomEditorSelector;
        use rustcode_ext_host::provider::ProviderRegistry;
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_platform::configuration::ConfigurationTarget;
        use rustcode_protocols::lsp::LspClientManager;
        use serde_json::json;

        let registry = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp));

        bridge.custom_editor_manager.register_provider(
            "catEdit.catScratch",
            WebviewOptions {
                enable_scripts: true,
                local_resource_roots: vec![],
            },
            true,
        );

        let config = ConfigurationService::new();
        let mut window = WorkbenchWindow::new(
            ThemeService::new(),
            KeybindingService::new(),
            CommandRegistry::new(),
            ContextKeyService::new(),
        )
        .with_config(config.clone())
        .with_language_bridge(bridge.clone());

        window.register_custom_editor_contribution(CustomEditorContribution {
            view_type: "catEdit.catScratch".to_string(),
            display_name: "Cat Scratch".to_string(),
            selector: vec![CustomEditorSelector {
                filename_pattern: "*.cscratch".to_string(),
            }],
            priority: Some("default".to_string()),
        });

        // Open matching custom editor file
        window.open_file("file:///workspace/demo.cscratch", "demo.cscratch", "{}", false);
        let uri = "file:///workspace/demo.cscratch";
        assert!(window.view.editor_area.webviews.contains_key(uri));

        let session = bridge
            .custom_editor_manager
            .find_session_by_uri(uri, Some("catEdit.catScratch"))
            .unwrap();
        bridge
            .webview_manager
            .set_html(&session.panel_id, "<h1>Synced Cat</h1><button>Purr</button>");
        bridge
            .webview_manager
            .post_message_to_webview(&session.panel_id, json!({ "cmd": "hydrate" }))
            .unwrap();

        window.sync_webviews();
        let wv = window.view.editor_area.webviews.get(uri).unwrap();
        assert!(wv.html.contains("Synced Cat"));
        assert_eq!(wv.inbound_messages.len(), 1);

        // Verify sync_configuration propagates to split editor panes as well
        window.open_file("file:///workspace/code.rs", "code.rs", "let a = 1;", false);
        let gid1 = window
            .view
            .editor_area
            .split_active_group(SplitDirection::Horizontal);
        config
            .update("editor.wordWrap", json!("on"), ConfigurationTarget::User)
            .unwrap();
        window.sync_configuration();

        assert_eq!(
            window
                .view
                .editor_area
                .editor_for_group(0, "file:///workspace/code.rs")
                .unwrap()
                .word_wrap,
            WordWrapMode::On
        );
        assert_eq!(
            window
                .view
                .editor_area
                .editor_for_group(gid1, "file:///workspace/code.rs")
                .unwrap()
                .word_wrap,
            WordWrapMode::On
        );
    }

    #[test]
    fn test_window_dispose_panel_closes_session_reopen_fresh_and_sync_ordering() {
        use rustcode_ext_host::manifest::CustomEditorSelector;
        use rustcode_ext_host::provider::ProviderRegistry;
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_protocols::lsp::LspClientManager;
        use serde_json::json;

        let registry = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp));

        bridge.custom_editor_manager.register_provider(
            "catEdit.catScratch",
            WebviewOptions {
                enable_scripts: true,
                local_resource_roots: vec![],
            },
            true,
        );

        let mut window = WorkbenchWindow::new(
            ThemeService::new(),
            KeybindingService::new(),
            CommandRegistry::new(),
            ContextKeyService::new(),
        )
        .with_language_bridge(bridge.clone());

        window.register_custom_editor_contribution(CustomEditorContribution {
            view_type: "catEdit.catScratch".to_string(),
            display_name: "Cat Scratch".to_string(),
            selector: vec![CustomEditorSelector {
                filename_pattern: "*.cscratch".to_string(),
            }],
            priority: Some("default".to_string()),
        });

        let uri = "file:///workspace/lifecycle.cscratch";
        window.open_file(uri, "lifecycle.cscratch", "{}", false);
        let session1 = bridge
            .custom_editor_manager
            .find_session_by_uri(uri, Some("catEdit.catScratch"))
            .unwrap();
        assert!(!bridge
            .webview_manager
            .get_panel(&session1.panel_id)
            .unwrap()
            .is_disposed);

        // 1. Disposing panel from ext-host + sync_webviews closes CustomEditorSessionState
        assert!(bridge.webview_manager.dispose_panel(&session1.panel_id));
        window.sync_webviews();
        assert!(bridge
            .custom_editor_manager
            .get_session(&session1.id)
            .unwrap()
            .is_closed);
        assert!(!window
            .view
            .editor_area
            .custom_editor_panels
            .contains_key(&session1.panel_id));
        assert!(!window.view.editor_area.webviews.contains_key(uri));

        // 2. Re-opening via open_file creates a fresh non-disposed session & panel
        window.open_file(uri, "lifecycle.cscratch", "{}", false);
        let session2 = bridge
            .custom_editor_manager
            .find_session_by_uri(uri, Some("catEdit.catScratch"))
            .unwrap();
        assert_ne!(session2.panel_id, session1.panel_id);
        assert!(!bridge
            .webview_manager
            .get_panel(&session2.panel_id)
            .unwrap()
            .is_disposed);
        bridge
            .webview_manager
            .post_message_to_webview(&session2.panel_id, json!({ "ok": true }))
            .unwrap();
        window.sync_webviews();
        assert_eq!(
            window
                .view
                .editor_area
                .webviews
                .get(uri)
                .unwrap()
                .inbound_messages
                .len(),
            1
        );

        // 3. Closing from UI closes backing panel & session when no split pane holds it
        window.handle_builtin_command_effects("workbench.action.closeActiveEditor");
        assert!(bridge
            .webview_manager
            .get_panel(&session2.panel_id)
            .unwrap()
            .is_disposed);
        assert!(bridge
            .custom_editor_manager
            .get_session(&session2.id)
            .unwrap()
            .is_closed);

        // 4. Out-of-band openCustomEditor RPC + sync_webviews() called BEFORE sync_custom_editors()
        let rpc_session = bridge
            .custom_editor_manager
            .handle_rpc_call(
                "window.openCustomEditor",
                &json!({
                    "viewType": "catEdit.catScratch",
                    "uri": "file:///workspace/oob.cscratch",
                    "title": "oob.cscratch"
                }),
                Some(&bridge.webview_manager),
            )
            .unwrap();
        let oob_panel_id = rpc_session["panelId"].as_str().unwrap();
        bridge
            .webview_manager
            .set_html(oob_panel_id, "<h1>Order Preserved</h1>");
        bridge
            .webview_manager
            .post_message_to_webview(oob_panel_id, json!({ "hello": "world" }))
            .unwrap();

        // Call sync_webviews BEFORE sync_custom_editors
        window.sync_webviews();
        window.sync_custom_editors();
        let oob_wv = window
            .view
            .editor_area
            .webviews
            .get("file:///workspace/oob.cscratch")
            .unwrap();
        assert!(oob_wv.html.contains("Order Preserved"));
        assert_eq!(oob_wv.inbound_messages, vec![json!({ "hello": "world" })]);
    }

    #[tokio::test]
    async fn test_window_m1_3_document_lifecycle_status_bar_and_show_document() {
        use rustcode_ext_host::provider::{ExtStatusBarItem, LanguageFeatureBridge, ProviderRegistry};
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_platform::render::{KeyCode, KeyEvent, MockRenderSurface};
        use rustcode_protocols::lsp::LspClientManager;
        use serde_json::json;

        let tmp = std::env::temp_dir().join(format!(
            "rustcode_win_m13_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let file_path = tmp.join("demo.rs");
        std::fs::write(&file_path, "fn main() {}\n").unwrap();
        let uri = rustcode_platform::path_to_file_uri(&file_path);

        let registry = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp));

        let mut window = WorkbenchWindow::default().with_language_bridge(bridge.clone());
        let _ = window.keybindings.load_keybindings_json(include_str!(
            "../../../resources/default-keymaps/linux.json"
        ));
        let _sub_save = window
            .commands
            .register("workbench.action.files.save", |_| Ok(json!({ "saved": true })));
        let _sub_close = window
            .commands
            .register("workbench.action.closeActiveEditor", |_| {
                Ok(json!({ "closed": true }))
            });

        // 1. Open -> bridge has document & active editor
        window.open_file(&uri, "demo.rs", "fn main() {}\n", false);
        assert_eq!(
            bridge.active_editor().map(|e| e.document.uri),
            Some(uri.clone())
        );

        // 2. Edit via dispatch_event -> bridge snapshot reflects live edit
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        window.dispatch_event(
            &InputEvent::KeyDown(
                KeyEvent::new(KeyCode::Char('!'), rustcode_platform::keybindings::Modifiers::none())
                    .with_text("!".to_string()),
            ),
            bounds,
        );
        let doc_json = bridge
            .handle_rpc_message("workspace.openTextDocument", &json!({ "uri": uri }))
            .await
            .unwrap();
        assert!(doc_json["text"].as_str().unwrap().contains('!'));

        // 3. Ctrl+S saves to disk and clears dirty state
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(
                KeyCode::Char('s'),
                rustcode_platform::keybindings::Modifiers {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    meta: false,
                },
            )),
            bounds,
        );
        assert!(!window.view.editor_area.active_editor().unwrap().is_dirty());
        assert!(std::fs::read_to_string(&file_path).unwrap().contains('!'));

        // 4. Status bar sync works both via render(&self) and sync_status_bar(&mut self)
        bridge.status_bar_manager.set_item(ExtStatusBarItem {
            id: "ext.rust.status".to_string(),
            text: "Rust Ready".to_string(),
            tooltip: Some("Analyzer ready".to_string()),
            command: None,
            alignment: 2,
            priority: 10,
            visible: true,
        });
        let mut surface = MockRenderSurface::new(1024.0, 768.0);
        let scene = window.render(&mut surface);
        assert!(scene.contains_text("Rust Ready"));
        window.sync_status_bar();
        assert!(window
            .view
            .status_bar
            .model
            .right_items()
            .iter()
            .any(|i| i.id == "ext.rust.status"));

        // 5. showTextDocument request opens file on sync
        let other_path = tmp.join("second.rs");
        std::fs::write(&other_path, "pub fn two() {}\n").unwrap();
        let other_uri = rustcode_platform::path_to_file_uri(&other_path);
        bridge
            .handle_rpc_message(
                "window.showTextDocument",
                &json!({
                    "uri": other_uri,
                    "options": {
                        "selection": {
                            "start": { "line": 0, "character": 4 },
                            "end": { "line": 0, "character": 4 }
                        }
                    }
                }),
            )
            .await;
        window.sync_window_manager();
        assert_eq!(
            window
                .view
                .editor_area
                .model
                .current_group_ref()
                .active_tab()
                .map(|t| t.uri.as_str()),
            Some(other_uri.as_str())
        );
        assert_eq!(
            window
                .view
                .editor_area
                .active_editor()
                .unwrap()
                .primary_position()
                .column,
            4
        );

        // 6. Ctrl+W closes active editor and updates bridge
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(
                KeyCode::Char('w'),
                rustcode_platform::keybindings::Modifiers {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    meta: false,
                },
            )),
            bounds,
        );
        assert_eq!(
            bridge.active_editor().map(|e| e.document.uri),
            Some(uri)
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_window_m2_find_replace_and_quickinput_goto_line_symbol() {
        let theme = ThemeService::new();
        let keybindings = KeybindingService::new();
        keybindings
            .load_keybindings_json(include_str!("../../../resources/default-keymaps/linux.json"))
            .unwrap();
        let commands = CommandRegistry::new();
        let context_keys = ContextKeyService::new();
        let mut window = WorkbenchWindow::new(theme, keybindings, commands, context_keys);
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);

        let tmp = std::env::temp_dir().join(format!("rc_win_m2_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp).unwrap();
        let file_path = tmp.join("sample.rs");
        let content = "fn first() {}\n\nfn target_fn() {\n    let old = 1;\n    let x = old;\n}\n";
        std::fs::write(&file_path, content).unwrap();
        let uri = rustcode_platform::path_to_file_uri(&file_path);
        window.open_file(&uri, "sample.rs", content, false);

        // 1. Ctrl+Shift+O -> @target_fn -> Enter
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(
                KeyCode::Char('o'),
                Modifiers {
                    ctrl: true,
                    shift: true,
                    alt: false,
                    meta: false,
                },
            )),
            bounds,
        );
        assert!(window.view.is_command_palette_open);
        for ch in "target_fn".chars() {
            window.dispatch_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert!(!window.view.is_command_palette_open);
        assert_eq!(
            window
                .view
                .editor_area
                .active_editor()
                .unwrap()
                .primary_position()
                .line,
            2
        );

        // 2. Ctrl+G -> 4:9 -> Enter
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(
                KeyCode::Char('g'),
                Modifiers {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    meta: false,
                },
            )),
            bounds,
        );
        for ch in "4:9".chars() {
            window.dispatch_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            window
                .view
                .editor_area
                .active_editor()
                .unwrap()
                .primary_position(),
            rustcode_editor::Position::new(3, 8)
        );

        // 3. Ctrl+H -> Replace All -> Escape -> Ctrl+Z
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(
                KeyCode::Char('h'),
                Modifiers {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    meta: false,
                },
            )),
            bounds,
        );
        for ch in "old".chars() {
            window.dispatch_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Tab, Modifiers::none())),
            bounds,
        );
        for ch in "new".chars() {
            window.dispatch_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        let replaced = window
            .view
            .editor_area
            .active_editor_mut()
            .unwrap()
            .replace_all();
        assert_eq!(replaced, 2);
        assert_eq!(
            window
                .view
                .editor_area
                .active_editor()
                .unwrap()
                .text()
                .matches("new")
                .count(),
            2
        );

        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bounds,
        );
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(
                KeyCode::Char('z'),
                Modifiers {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    meta: false,
                },
            )),
            bounds,
        );
        assert_eq!(
            window.view.editor_area.active_editor().unwrap().text(),
            content
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_window_m3_notifications_prompts_escape_sync_and_testing_sidebar() {
        use rustcode_ext_host::provider::ProviderRegistry;
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_platform::accessibility::AccessibleRole;
        use rustcode_platform::render::{
            KeyCode, KeyEvent, MockRenderSurface, MouseButton, MouseEvent, Point,
        };
        use rustcode_protocols::lsp::LspClientManager;
        use serde_json::{json, Value};

        let registry = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc.clone(), lsp));
        bridge.window_manager.set_await_responses(true);

        let mut window = WorkbenchWindow::default().with_language_bridge(bridge.clone());
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let mut surface = MockRenderSurface::new(1024.0, 768.0);

        // 1. Escape-sync safety: Escape dispatched BEFORE prompt is visible in WorkbenchView
        // must sync the queued prompt WITHOUT cancelling it on that same keystroke.
        let b1 = Arc::clone(&bridge);
        let qp_task = tokio::spawn(async move {
            b1.handle_rpc_message(
                "window.showQuickPick",
                &json!({ "items": ["Alpha", "Beta"], "options": { "placeHolder": "Pick" } }),
            )
            .await
        });
        for _ in 0..20 {
            if !bridge.window_manager.active_prompts().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!window.view.is_prompt_open());
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bounds,
        );
        assert!(window.view.is_prompt_open());
        assert!(window.render(&mut surface).contains_text("Alpha"));

        // Second Escape cancels the now-visible prompt cleanly
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bounds,
        );
        assert_eq!(qp_task.await.unwrap(), Some(Value::Null));
        assert!(!window.view.is_prompt_open());

        // 2. Notification toast action click resolves waiter & removes toast
        let b2 = Arc::clone(&bridge);
        let notif_task = tokio::spawn(async move {
            b2.handle_rpc_message(
                "window.showWarningMessage",
                &json!({ "message": "Reload config?", "items": ["Reload"] }),
            )
            .await
        });
        for _ in 0..20 {
            if !bridge.window_manager.active_notifications().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        window.sync_window_manager();
        let tree = window.accessibility_tree(bounds);
        let reload_btn = tree
            .to_flat_list()
            .into_iter()
            .find(|n| n.role == AccessibleRole::Button && n.name == "Reload")
            .unwrap();
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(
                    reload_btn.bounds.min_x() + reload_btn.bounds.width() * 0.5,
                    reload_btn.bounds.min_y() + reload_btn.bounds.height() * 0.5,
                ),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(notif_task.await.unwrap(), Some(json!("Reload")));
        assert!(!window.render(&mut surface).contains_text("Reload config?"));

        // 3. Testing sidebar sync, failed badge, Run All RPC
        bridge
            .handle_rpc_message(
                "tests.createController",
                &json!({ "id": "cargo", "label": "Cargo" }),
            )
            .await;
        bridge
            .handle_rpc_message(
                "tests.updateItems",
                &json!({
                    "controllerId": "cargo",
                    "items": [{ "id": "t_fail", "label": "t_fail", "children": [] }]
                }),
            )
            .await;
        bridge
            .handle_rpc_message(
                "tests.createTestRun",
                &json!({ "runId": "r1", "controllerId": "cargo" }),
            )
            .await;
        bridge
            .handle_rpc_message(
                "tests.updateTestState",
                &json!({
                    "runId": "r1",
                    "testId": "t_fail",
                    "state": "failed",
                    "duration": 18,
                    "messages": [{ "message": "boom" }]
                }),
            )
            .await;
        bridge
            .handle_rpc_message("tests.endTestRun", &json!({ "runId": "r1" }))
            .await;

        window.handle_builtin_command_effects("workbench.view.testing");
        window.sync_test_manager();
        assert_eq!(window.view.activity_bar.model.items[5].badge, Some(1));
        let test_scene = window.render(&mut surface);
        assert!(test_scene.contains_text("TESTING"));
        assert!(test_scene.contains_text("18ms"));
        assert!(test_scene.contains_text("boom"));

        let test_tree = window.accessibility_tree(bounds);
        let run_all = test_tree.find_by_id("testing.runAll").unwrap().bounds;
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(
                    run_all.min_x() + run_all.width() * 0.5,
                    run_all.min_y() + run_all.height() * 0.5,
                ),
                MouseButton::Left,
            )),
            bounds,
        );
        for _ in 0..20 {
            if rpc.pending_count() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(rpc.pending_count() > 0);
        let _ = rpc.handle_incoming_response(1, Some(json!({ "ran": true })), None);
    }

    #[tokio::test]
    async fn test_window_m4_explorer_crud_tab_sync_and_contributed_treeviews() {
        use crate::explorer::FileTreeNode;
        use crate::workspace::Workspace;
        use rustcode_ext_host::manifest::ExtensionLoader;
        use rustcode_ext_host::provider::ProviderRegistry;
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_platform::accessibility::AccessibleRole;
        use rustcode_platform::render::{
            KeyCode, KeyEvent, MockRenderSurface, MouseButton, MouseEvent, Point,
        };
        use rustcode_protocols::lsp::LspClientManager;
        use serde_json::json;
        use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

        let tmp = std::env::temp_dir().join(format!(
            "rustcode_win_m4_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let ext_dir = tmp.join(".ext/tree_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("package.json"),
            serde_json::to_string(&json!({
                "name": "tree-ext",
                "publisher": "test",
                "version": "1.0.0",
                "contributes": {
                    "views": {
                        "explorer": [
                            { "id": "outlineView", "name": "Project Outline", "when": "hasProj" },
                            { "id": "badWhenView", "name": "Bad When View", "when": "(((broken && ||" }
                        ]
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let mut loader = ExtensionLoader::new();
        loader.load_from_dir(&ext_dir).unwrap();

        let registry = Arc::new(ProviderRegistry::new());
        let rpc = Arc::new(RpcBridge::new());
        let lsp = Arc::new(LspClientManager::new());
        let commands = CommandRegistry::new();
        let cmd_ran = Arc::new(AtomicBool::new(false));
        let cmd_ran_clone = Arc::clone(&cmd_ran);
        let _cmd_disp = commands.register("outline.refresh", move |_| {
            cmd_ran_clone.store(true, AtomicOrdering::SeqCst);
            Ok(json!({ "ok": true }))
        });

        let bridge = Arc::new(
            LanguageFeatureBridge::new(registry, Arc::clone(&rpc), lsp)
                .with_extension_loader(&loader)
                .with_command_registry(commands.clone())
                .with_fs_root(&tmp),
        );

        let mut window = WorkbenchWindow::new(
            ThemeService::new(),
            KeybindingService::new(),
            commands,
            ContextKeyService::new(),
        )
        .with_language_bridge(Arc::clone(&bridge));
        window.view.workspace = Workspace::from_folder(tmp.clone());
        window
            .view
            .explorer
            .model
            .set_roots(vec![FileTreeNode::from_path(tmp.clone())]);
        window.view.explorer.model.expanded_paths.insert(tmp.clone());

        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        let mut surface = MockRenderSurface::new(1024.0, 768.0);

        // 1. Explorer CRUD: New File -> F2 Rename -> Delete confirm
        window.view.focused_region = FocusedRegion::Sidebar;
        let tree0 = window.accessibility_tree(bounds);
        let nf_bounds = tree0.find_by_id("explorer.newFile").unwrap().bounds;
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(
                    nf_bounds.min_x() + nf_bounds.width() * 0.5,
                    nf_bounds.min_y() + nf_bounds.height() * 0.5,
                ),
                MouseButton::Left,
            )),
            bounds,
        );
        for ch in "hello.rs".chars() {
            window.dispatch_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert!(tmp.join("hello.rs").is_file());
        assert!(window
            .view
            .editor_area
            .find_open_tab_uri_for_path(&tmp.join("hello.rs"))
            .is_some());

        // Rename hello.rs -> renamed.rs via F2 and verify tab updates
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::F2, Modifiers::none())),
            bounds,
        );
        for ch in "renamed.rs".chars() {
            window.dispatch_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert!(!tmp.join("hello.rs").exists() && tmp.join("renamed.rs").is_file());
        assert!(window
            .view
            .editor_area
            .find_open_tab_uri_for_path(&tmp.join("renamed.rs"))
            .is_some());

        // Delete renamed.rs and verify tab closes
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Delete, Modifiers::none())),
            bounds,
        );
        window.dispatch_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert!(!tmp.join("renamed.rs").exists());
        assert!(window
            .view
            .editor_area
            .find_open_tab_uri_for_path(&tmp.join("renamed.rs"))
            .is_none());

        // 2. Contributed TreeViews: when=false hidden, malformed hidden, when=true lazy root & expand & refresh & command
        bridge
            .handle_rpc_message(
                "extHost.registerProvider",
                &json!({ "type": "TreeData", "id": "outlineView" }),
            )
            .await;

        let hidden_scene = window.render(&mut surface);
        assert!(!hidden_scene.contains_text("Project Outline"));
        assert!(!hidden_scene.contains_text("Bad When View"));

        window.context_keys.set_bool("hasProj", true);
        window.sync_tree_views();
        for _ in 0..20 {
            if rpc.pending_count() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        let _ = rpc.handle_incoming_response(
            1,
            Some(json!([
                {
                    "id": "pkg_root",
                    "label": "pkg_root",
                    "collapsibleState": 1,
                    "command": { "title": "Refresh", "command": "outline.refresh" }
                },
                {
                    "resourceUri": "C:\\proj\\src\\win_file.rs",
                    "collapsibleState": 0
                }
            ])),
            None,
        );
        tokio::task::yield_now().await;

        let visible_scene = window.render(&mut surface);
        assert!(visible_scene.contains_text("Project Outline"));
        assert!(visible_scene.contains_text("pkg_root"));
        assert!(visible_scene.contains_text("win_file.rs"));
        assert!(!visible_scene.contains_text("Bad When View"));

        // Click pkg_root -> executes command AND toggles expand -> triggers child fetch (id 2)
        let tree1 = window.accessibility_tree(bounds);
        let trees = tree1.find_by_role(AccessibleRole::Tree);
        assert!(trees.len() >= 2);
        let pkg_bounds = tree1
            .find_by_role(AccessibleRole::TreeItem)
            .into_iter()
            .find(|n| n.name == "pkg_root")
            .unwrap()
            .bounds;
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(
                    pkg_bounds.min_x() + pkg_bounds.width() * 0.5,
                    pkg_bounds.min_y() + pkg_bounds.height() * 0.5,
                ),
                MouseButton::Left,
            )),
            bounds,
        );
        assert!(cmd_ran.load(AtomicOrdering::SeqCst));

        for _ in 0..20 {
            if rpc.pending_count() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        let _ = rpc.handle_incoming_response(
            2,
            Some(json!([{ "id": "child_a", "label": "child_a.rs", "collapsibleState": 0 }])),
            None,
        );
        tokio::task::yield_now().await;
        let child_scene = window.render(&mut surface);
        assert!(child_scene.contains_text("child_a.rs"));

        // onDidChangeTreeData refreshes view (id 3)
        bridge
            .handle_rpc_message(
                "treeView.onDidChangeTreeData",
                &json!({ "viewId": "outlineView" }),
            )
            .await;
        window.sync_tree_views();
        for _ in 0..20 {
            if rpc.pending_count() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        let _ = rpc.handle_incoming_response(
            3,
            Some(json!([{ "id": "v2", "label": "RefreshedNode", "collapsibleState": 0 }])),
            None,
        );
        tokio::task::yield_now().await;
        let refreshed_scene = window.render(&mut surface);
        assert!(refreshed_scene.contains_text("RefreshedNode"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn make_temp_test_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rustcode_w2c_{}_{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_window_os_ux_clipboard_file_dialogs_and_recent_storage() {
        use rustcode_platform::{
            InMemoryClipboardBackend, InMemoryStorageBackend, MockFileDialogBackend, StorageBackend,
        };

        let tmp = make_temp_test_dir("os_ux");
        let file_a = tmp.join("hello.rs");
        let file_save_as = tmp.join("saved_as.rs");
        std::fs::write(&file_a, "fn hello() {}\n").unwrap();

        let clipboard = Arc::new(InMemoryClipboardBackend::new());
        let file_dialog = Arc::new(MockFileDialogBackend::new());
        let storage = Arc::new(InMemoryStorageBackend::new());

        file_dialog.push_open_file(Some(file_a.clone()));
        file_dialog.push_save_file(Some(file_save_as.clone()));
        file_dialog.push_open_folder(Some(tmp.clone()));

        let mut window = WorkbenchWindow::default()
            .with_clipboard_backend(clipboard.clone())
            .with_file_dialog_backend(file_dialog)
            .with_storage_backend(storage.clone());

        // 1. Open File via FileDialogBackend
        window.handle_builtin_command_effects("workbench.action.files.openFile");
        let active_uri = window
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .unwrap()
            .uri
            .clone();
        assert_eq!(active_uri, rustcode_platform::path_to_file_uri(&file_a));

        // 2. EditorView clipboard callbacks route through ClipboardBackend (Ctrl+A + Ctrl+C / Ctrl+V)
        let bounds = Rect::from_xywh(0.0, 0.0, 1200.0, 800.0);
        let ctrl_a = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('a'),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        window.dispatch_event(&ctrl_a, bounds);
        {
            let ed = window.view.editor_area.active_editor_mut().unwrap();
            ed.copy();
        }
        assert_eq!(clipboard.read_text().unwrap(), "fn hello() {}\n");

        clipboard.write_text("fn replaced() {}\n").unwrap();
        {
            let ed = window.view.editor_area.active_editor_mut().unwrap();
            ed.paste();
            assert_eq!(ed.text(), "fn replaced() {}\n");
        }

        // 3. Save As via FileDialogBackend writes disk and updates active tab URI
        window.handle_builtin_command_effects("workbench.action.files.saveAs");
        assert_eq!(
            std::fs::read_to_string(&file_save_as).unwrap(),
            "fn replaced() {}\n"
        );
        let new_uri = window
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .unwrap()
            .uri
            .clone();
        assert_eq!(
            new_uri,
            rustcode_platform::path_to_file_uri(&file_save_as)
        );

        // 4. Open Folder via FileDialogBackend records recent workspace in StorageBackend
        window.handle_builtin_command_effects("workbench.action.files.openFolder");
        assert_eq!(window.view.workspace.folders().len(), 1);
        let recent = storage.recent_workspaces().unwrap();
        assert!(recent.contains(&tmp));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_window_search_replace_and_comment_line_keymap() {
        let tmp = make_temp_test_dir("search_replace");
        let file_a = tmp.join("alpha.rs");
        let file_b = tmp.join("beta.rs");
        std::fs::write(&file_a, "let old_token = 1;\n").unwrap();
        std::fs::write(&file_b, "let old_token = 2;\n").unwrap();

        let mut window = WorkbenchWindow::default();
        let default_linux_keymap = include_str!("../../../resources/default-keymaps/linux.json");
        window
            .keybindings
            .load_keybindings_json(default_linux_keymap)
            .unwrap();
        window.open_workspace(&tmp);
        window.open_path(&file_a, true);

        let bounds = Rect::from_xywh(0.0, 0.0, 1200.0, 800.0);

        // 1. Ctrl+/ comments the active line via default keymap + editor.action.commentLine
        let ctrl_slash = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('/'),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ));
        assert_eq!(window.dispatch_event(&ctrl_slash, bounds), EventResult::Consumed);
        let ed_text_after_comment = window.view.editor_area.active_editor().unwrap().text();
        assert!(
            ed_text_after_comment.contains("// let old_token = 1;"),
            "expected commented line, got: {:?}",
            ed_text_after_comment
        );

        // Toggle back with Ctrl+/ so old_token matches clean text, save to disk
        assert_eq!(window.dispatch_event(&ctrl_slash, bounds), EventResult::Consumed);
        assert!(window.save_active_file());

        // 2. Ctrl+Shift+F opens Search sidebar (find mode)
        let ctrl_shift_f = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('f'),
            Modifiers {
                ctrl: true,
                shift: true,
                alt: false,
                meta: false,
            },
        ));
        assert_eq!(window.dispatch_event(&ctrl_shift_f, bounds), EventResult::Consumed);
        assert!(window.view.layout.sidebar_visible);
        assert_eq!(window.view.layout.active_sidebar_view, "search");

        // 3. Ctrl+Shift+H opens Search sidebar in Replace mode
        let ctrl_shift_h = InputEvent::KeyDown(KeyEvent::new(
            KeyCode::Char('h'),
            Modifiers {
                ctrl: true,
                shift: true,
                alt: false,
                meta: false,
            },
        ));
        assert_eq!(window.dispatch_event(&ctrl_shift_h, bounds), EventResult::Consumed);
        assert!(window.view.layout.sidebar_visible);
        assert_eq!(window.view.layout.active_sidebar_view, "search");
        assert!(window.view.search_view.is_replace_expanded);

        // 4. Execute workspace Search & Replace and verify open EditorArea buffer refreshes
        window.view.search_view.query.query = "old_token".to_string();
        window.view.search_view.replace_text = "new_token".to_string();
        window.handle_search_action(&SearchAction::ExecuteSearch);
        assert!(window.view.search_view.results.total_matches >= 2);

        window.handle_search_action(&SearchAction::ExecuteReplace);
        assert_eq!(
            std::fs::read_to_string(&file_a).unwrap(),
            "let new_token = 1;\n"
        );
        assert_eq!(
            std::fs::read_to_string(&file_b).unwrap(),
            "let new_token = 2;\n"
        );
        assert_eq!(
            window.view.editor_area.active_editor().unwrap().text(),
            "let new_token = 1;\n"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_window_format_on_save_and_format_document_command() {
        use rustcode_ext_host::provider::{ProviderRegistration, ProviderRegistry, ProviderType};
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_protocols::lsp::LspClientManager;
        use serde_json::json;

        let tmp = make_temp_test_dir("format_on_save");
        let file_a = tmp.join("fmt.rs");
        std::fs::write(&file_a, "fn   bad( ) {}\n").unwrap();

        let rpc = Arc::new(RpcBridge::new());
        let registry = Arc::new(ProviderRegistry::new());
        let lsp_mgr = Arc::new(LspClientManager::new());
        registry.register(ProviderRegistration {
            id: "fmt-ext".to_string(),
            provider_type: ProviderType::DocumentFormatting,
            language_selector: Some("rust".to_string()),
        });
        let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc.clone(), lsp_mgr));

        let mut window = WorkbenchWindow::default().with_language_bridge(bridge);
        window.open_workspace(&tmp);
        window
            .config
            .load_workspace_settings(r#"{"editor.formatOnSave": true}"#)
            .unwrap();
        window.open_path(&file_a, true);

        // Save async formats BEFORE writing to disk
        let mut saved = false;
        {
            let save_fut = window.save_active_file_async();
            tokio::pin!(save_fut);

            let mut responded = false;
            for _ in 0..40 {
                tokio::select! {
                    res = &mut save_fut => {
                        saved = res;
                        break;
                    }
                    _ = tokio::task::yield_now() => {
                        if !responded && rpc.pending_count() > 0 {
                            let _ = rpc.handle_incoming_response(
                                1,
                                Some(json!([{
                                    "range": {
                                        "start": { "line": 0, "character": 0 },
                                        "end": { "line": 0, "character": 14 }
                                    },
                                    "newText": "fn good() {}"
                                }])),
                                None,
                            );
                            responded = true;
                        }
                    }
                }
            }
        }
        assert!(saved);
        assert_eq!(
            std::fs::read_to_string(&file_a).unwrap(),
            "fn good() {}\n"
        );
        assert_eq!(
            window.view.editor_area.active_editor().unwrap().text(),
            "fn good() {}\n"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_window_scm_actions_open_diff_and_status_bar() {
        use crate::scm::ScmAction;
        use crate::statusbar::STATUS_GIT_BRANCH_ID;
        use rustcode_protocols::git::{
            GitStatusEntry, GitStatusKind, InMemoryGitBackend, ScmRepository,
        };

        let tmp = make_temp_test_dir("scm_wire");
        let file_foo = tmp.join("foo.rs");
        std::fs::write(&file_foo, "fn v2() {}\n").unwrap();

        let mut repo = ScmRepository::new(tmp.clone());
        repo.current_branch.name = "feature/wave2c".to_string();
        repo.changes.push(GitStatusEntry {
            path: PathBuf::from("foo.rs"),
            status: GitStatusKind::Modified,
            is_staged: false,
        });
        repo.set_file_diff("foo.rs", "fn v1() {}\n", "fn v2() {}\n");

        let backend = InMemoryGitBackend::new(repo);
        let mut window = WorkbenchWindow::default();
        window.view.set_git_backend(Arc::new(backend));

        assert_eq!(window.view.scm_view.repo.changes.len(), 1);
        assert_eq!(window.view.scm_view.repo.current_branch.name, "feature/wave2c");
        assert!(window
            .view
            .status_bar
            .model
            .item(STATUS_GIT_BRANCH_ID)
            .is_some_and(|it| it.text.contains("feature/wave2c")));

        // Stage all via ScmAction handler
        window.handle_scm_action(&ScmAction::StageAll).unwrap();
        assert!(window.view.scm_view.repo.changes.is_empty());
        assert_eq!(window.view.scm_view.repo.staged.len(), 1);

        // OpenDiff opens diff tab in EditorAreaView
        window.open_scm_diff(&PathBuf::from("foo.rs"), false).unwrap();
        let active_tab = window
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .unwrap();
        assert!(active_tab.uri.starts_with("diff://"));

        // Commit staged changes
        window
            .handle_scm_action(&ScmAction::Commit {
                message: "Wave 2C commit".to_string(),
            })
            .unwrap();
        assert!(window.view.scm_view.repo.staged.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_window_terminal_pty_action_and_webview_ipc() {
        use crate::terminal_panel::TerminalAction;
        use crate::webview::HeadlessWebviewBackend;
        use rustcode_ext_host::provider::ProviderRegistry;
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_ext_host::webview::WebviewOptions;
        use rustcode_protocols::lsp::LspClientManager;
        use rustcode_terminal::pty::HeadlessPtyBackend;
        use serde_json::json;

        let rpc = Arc::new(RpcBridge::new());
        let registry = Arc::new(ProviderRegistry::new());
        let lsp_mgr = Arc::new(LspClientManager::new());
        let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp_mgr));

        // 1. ExtHost terminal -> local PTY terminal + lifecycle sync
        bridge
            .terminal_manager
            .create_terminal("ext_term_1", "PTY Task", None, vec![], None);
        bridge
            .terminal_manager
            .send_text("ext_term_1", "echo hi\n", false)
            .unwrap();

        let mut window = WorkbenchWindow::default().with_language_bridge(bridge.clone());
        window.sync_ext_terminals();
        let local_id = window.ext_terminal_map.get("ext_term_1").unwrap().clone();

        // Attach HeadlessPtyBackend and drive TerminalAction + poll_runtime_events
        let pty = HeadlessPtyBackend::new(24, 80);
        pty.push_output_str("Hello PTY\r\n");
        {
            let term = window
                .view
                .terminal_panel
                .manager
                .terminal_mut(&local_id)
                .unwrap();
            term.attach_pty(Box::new(pty.clone()));
            term.resize(30, 100).unwrap();
        }
        window.poll_runtime_events();
        assert_eq!(
            bridge
                .terminal_manager
                .get_terminal("ext_term_1")
                .unwrap()
                .rows,
            Some(30)
        );
        assert_eq!(
            bridge
                .terminal_manager
                .get_terminal("ext_term_1")
                .unwrap()
                .cols,
            Some(100)
        );
        assert!(window
            .view
            .terminal_panel
            .manager
            .terminal(&local_id)
            .unwrap()
            .lines
            .iter()
            .any(|l| l.contains("Hello PTY")));

        // New terminal via TerminalAction
        let before_count = window.view.terminal_panel.manager.terminals().len();
        window.handle_terminal_action(&TerminalAction::NewTerminal { id: String::new() });
        assert_eq!(
            window.view.terminal_panel.manager.terminals().len(),
            before_count + 1
        );

        // 2. Webview IPC outbound drain -> ExtHostWebviewManager
        bridge.webview_manager.create_panel(
            "panel_1",
            "test.view",
            "Test Panel",
            WebviewOptions::default(),
        );
        assert!(bridge.webview_manager.set_html("panel_1", "<p>Loaded</p>"));
        window.sync_webviews();

        let webview_uri = "webview://panel_1";
        let mut backend = HeadlessWebviewBackend::with_html("<p>Loaded</p>");
        backend.inject_outbound_message(json!({ "command": "ping", "value": 7 }));
        let wv = window
            .view
            .editor_area
            .webviews
            .get_mut(webview_uri)
            .unwrap();
        wv.set_backend(Box::new(backend));

        window.sync_webview_outbound_messages();
        let inbound = bridge.webview_manager.take_inbound_messages("panel_1");
        assert_eq!(inbound.len(), 1);
        assert_eq!(inbound[0]["command"], "ping");
        assert_eq!(inbound[0]["value"], 7);
    }

    #[tokio::test]
    async fn test_window_lsp_goto_definition_hover_completions_diagnostics_and_jump() {
        use rustcode_ext_host::provider::ProviderRegistry;
        use rustcode_ext_host::rpc::RpcBridge;
        use rustcode_platform::render::{MouseButton, MouseEvent, Point};
        use rustcode_protocols::codec::{read_framed_message, write_framed_message};
        use rustcode_protocols::lsp::{LspClient, LspClientManager};
        use serde_json::json;

        let tmp = make_temp_test_dir("lsp_wire");
        let main_rs = tmp.join("main.rs");
        let lib_rs = tmp.join("lib.rs");
        std::fs::write(&main_rs, "fn main() { foo(); }\n").unwrap();
        std::fs::write(&lib_rs, "pub fn bar() {}\npub fn foo() {}\n").unwrap();

        let lib_uri = rustcode_platform::path_to_file_uri(&lib_rs);
        let main_uri = rustcode_platform::path_to_file_uri(&main_rs);

        let (client_read, server_write) = tokio::io::duplex(8192);
        let (server_read, client_write) = tokio::io::duplex(8192);

        let lib_uri_srv = lib_uri.clone();
        let main_uri_srv = main_uri.clone();
        let server_task = tokio::spawn(async move {
            let mut reader = server_read;
            let mut writer = server_write;
            let mut msg_buf = Vec::new();
            while let Ok(Some(bytes)) = read_framed_message(&mut reader, &mut msg_buf).await {
                let msg: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let id = msg.get("id").cloned();
                let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
                if let Some(req_id) = id {
                    let result = match method {
                        "textDocument/definition" => json!({
                            "uri": lib_uri_srv,
                            "range": {
                                "start": { "line": 1, "character": 7 },
                                "end": { "line": 1, "character": 10 }
                            }
                        }),
                        "textDocument/hover" => json!({
                            "contents": { "kind": "markdown", "value": "```rust\npub fn foo()\n```" }
                        }),
                        "textDocument/completion" => json!([
                            { "label": "foo_lsp", "kind": 3, "detail": "fn foo_lsp()" }
                        ]),
                        _ => json!(null),
                    };
                    let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                    let _ = write_framed_message(&mut writer, resp.to_string().as_bytes()).await;

                    if method == "textDocument/definition" {
                        let diag_notif = json!({
                            "jsonrpc": "2.0",
                            "method": "textDocument/publishDiagnostics",
                            "params": {
                                "uri": main_uri_srv,
                                "diagnostics": [{
                                    "range": {
                                        "start": { "line": 0, "character": 12 },
                                        "end": { "line": 0, "character": 15 }
                                    },
                                    "severity": 1,
                                    "source": "rustc",
                                    "message": "unresolved foo"
                                }]
                            }
                        });
                        let _ =
                            write_framed_message(&mut writer, diag_notif.to_string().as_bytes())
                                .await;
                    }
                }
            }
        });

        let client = Arc::new(LspClient::from_io(client_read, client_write));
        let rpc = Arc::new(RpcBridge::new());
        let registry = Arc::new(ProviderRegistry::new());
        let lsp_mgr = Arc::new(LspClientManager::new());
        lsp_mgr.attach_client("rust", client);
        let bridge = Arc::new(LanguageFeatureBridge::new(registry, rpc, lsp_mgr));

        let mut window = WorkbenchWindow::default().with_language_bridge(bridge);
        window.open_workspace(&tmp);
        window.open_path(&main_rs, true);

        let mut surface = MockRenderSurface::new(1200.0, 800.0);

        // 1. Hover + Completions on main.rs rendered in Scene
        assert!(window.request_hover_at(Position::new(0, 13)).await);
        assert!(window.request_completions_at(Position::new(0, 13), Some('.')).await > 0);
        let hover_comp_scene = window.render(&mut surface);
        assert!(hover_comp_scene.contains_text("foo_lsp"));

        // 2. GoToDefinition jumps cross-file to lib.rs line 1 col 7
        assert!(window.go_to_definition_at(Position::new(0, 13)).await);

        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            window.poll_runtime_events();
            if window.view.problems_panel.total_errors() > 0 {
                break;
            }
        }

        // Verify cross-file GoToDefinition jumped to lib.rs line 1 col 7
        assert_eq!(
            window
                .view
                .editor_area
                .model
                .current_group_ref()
                .active_tab()
                .unwrap()
                .uri,
            lib_uri
        );
        assert_eq!(
            window.view.editor_area.active_editor().unwrap().primary_position(),
            Position::new(1, 7)
        );

        // Verify diagnostics populated ProblemsPanel & JumpToDiagnostic navigates back to main.rs
        assert_eq!(window.view.problems_panel.total_errors(), 1);
        window.view.last_action = Some(WorkbenchAction::JumpToDiagnostic {
            uri: main_uri.clone(),
            line: 0,
            column: 12,
        });
        window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(Point::new(10.0, 10.0), MouseButton::Left)),
            Rect::from_xywh(0.0, 0.0, 1200.0, 800.0),
        );
        assert_eq!(
            window
                .view
                .editor_area
                .model
                .current_group_ref()
                .active_tab()
                .unwrap()
                .uri,
            main_uri
        );
        assert_eq!(
            window.view.editor_area.active_editor().unwrap().primary_position(),
            Position::new(0, 12)
        );

        server_task.abort();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_window_dap_launch_json_toolbar_and_debug_actions() {
        use crate::debug::{DebugToolbarAction, DebugViewAction};
        use rustcode_protocols::codec::{read_framed_message, write_framed_message};
        use rustcode_protocols::dap::DapClient;
        use serde_json::json;

        let tmp = make_temp_test_dir("dap_wire");
        let vscode_dir = tmp.join(".vscode");
        std::fs::create_dir_all(&vscode_dir).unwrap();
        let main_rs = tmp.join("main.rs");
        std::fs::write(&main_rs, "fn main() {\n    let x = 1;\n}\n").unwrap();
        std::fs::write(
            vscode_dir.join("launch.json"),
            json!({
                "version": "0.2.0",
                "configurations": [{
                    "name": "Debug Main",
                    "type": "lldb",
                    "request": "launch",
                    "program": "${workspaceFolder}/target/debug/app"
                }]
            })
            .to_string(),
        )
        .unwrap();

        let (client_read, server_write) = tokio::io::duplex(8192);
        let (server_read, client_write) = tokio::io::duplex(8192);
        let main_path_str = main_rs.to_string_lossy().into_owned();

        let server_task = tokio::spawn(async move {
            let mut reader = server_read;
            let mut writer = server_write;
            let mut seq = 1i64;
            let mut msg_buf = Vec::new();
            while let Ok(Some(bytes)) = read_framed_message(&mut reader, &mut msg_buf).await {
                let req: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let req_seq = req.get("seq").and_then(|v| v.as_i64()).unwrap_or(1);
                let cmd = req.get("command").and_then(|v| v.as_str()).unwrap_or("");
                let body = match cmd {
                    "initialize" => json!({ "supportsConfigurationDoneRequest": true }),
                    "threads" => json!({ "threads": [{ "id": 1, "name": "main" }] }),
                    "stackTrace" => json!({
                        "stackFrames": [{
                            "id": 100,
                            "name": "main",
                            "source": { "name": "main.rs", "path": main_path_str },
                            "line": 2,
                            "column": 5
                        }],
                        "totalFrames": 1
                    }),
                    "scopes" => json!({
                        "scopes": [{
                            "name": "Locals",
                            "variablesReference": 200,
                            "expensive": false
                        }]
                    }),
                    "variables" => json!({
                        "variables": [{
                            "name": "x",
                            "value": "42",
                            "type": "i32",
                            "variablesReference": 0
                        }]
                    }),
                    "evaluate" => json!({
                        "result": "42",
                        "type": "i32",
                        "variablesReference": 0
                    }),
                    _ => json!({}),
                };
                let resp = json!({
                    "seq": seq,
                    "type": "response",
                    "request_seq": req_seq,
                    "success": true,
                    "command": cmd,
                    "body": body
                });
                seq += 1;
                let _ = write_framed_message(&mut writer, resp.to_string().as_bytes()).await;

                if cmd == "configurationDone" {
                    let stopped = json!({
                        "seq": seq,
                        "type": "event",
                        "event": "stopped",
                        "body": { "reason": "breakpoint", "threadId": 1 }
                    });
                    seq += 1;
                    let _ = write_framed_message(&mut writer, stopped.to_string().as_bytes()).await;
                }
            }
        });

        let dap = Arc::new(DapClient::from_io(client_read, client_write));
        let mut window = WorkbenchWindow::default().with_dap_client(dap);
        window.open_workspace(&tmp);

        assert_eq!(window.view.debug_sidebar.launch_configurations.len(), 1);
        assert_eq!(
            window.view.debug_sidebar.launch_configurations[0].name,
            "Debug Main"
        );

        window.start_debug_session(Some(0)).await.unwrap();
        assert!(window.view.debug_sidebar.session.is_active);
        assert!(window.view.debug_toolbar.is_visible);

        // Wait briefly for stopped event + async snapshot sync
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            window.poll_runtime_events();
            if window.view.debug_sidebar.session.is_paused
                && !window.view.debug_sidebar.session.call_stack.is_empty()
            {
                break;
            }
        }
        assert!(window.view.debug_sidebar.session.is_paused);
        assert_eq!(window.view.debug_sidebar.session.call_stack[0].name, "main");
        assert_eq!(
            window
                .view
                .debug_sidebar
                .session
                .variables
                .get(&200)
                .unwrap()[0]
                .name,
            "x"
        );

        // Evaluate expression in Debug Console
        window
            .handle_debug_view_action_async(&DebugViewAction::EvaluateExpression {
                expression: "x".to_string(),
            })
            .await
            .unwrap();
        window.poll_runtime_events();
        assert!(window
            .view
            .debug_sidebar
            .session
            .console_output
            .iter()
            .any(|entry| entry.text.contains("42")));

        // StepOver & Stop via DebugToolbarAction
        window
            .handle_debug_toolbar_action_async(&DebugToolbarAction::StepOver)
            .await
            .unwrap();
        window
            .handle_debug_toolbar_action_async(&DebugToolbarAction::Stop)
            .await
            .unwrap();
        assert!(!window.view.debug_sidebar.session.is_active);
        assert!(!window.view.debug_toolbar.is_visible);

        server_task.abort();
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
