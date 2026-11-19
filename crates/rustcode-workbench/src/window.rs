//! GPUI WorkbenchWindow host managing rendering surfaces, theme updates, and input routing.

use crate::editor_area::SplitDirection;
use crate::extensions_view::ExtensionsViewAction;
use crate::output_panel::OutputAction;
use crate::quickinput::{CommandPalette, CommandPaletteView, PaletteItem};
use crate::workbench_view::{FocusedRegion, WorkbenchAction, WorkbenchView};
use rustcode_base::context_key::ContextKeyService;
use rustcode_base::disposable::DisposableStore;
use rustcode_editor::WordWrapMode;
use rustcode_ext_host::manifest::CustomEditorContribution;
use rustcode_ext_host::marketplace::ExtensionInstaller;
use rustcode_ext_host::output::OutputChannelEvent;
use rustcode_ext_host::provider::LanguageFeatureBridge;
use rustcode_ext_host::terminal::ExtTerminalEvent;
use rustcode_ext_host::webview::{CustomEditorEvent, WebviewEvent, WebviewOptions};
use rustcode_platform::accessibility::AccessibilityTree;
use rustcode_platform::commands::CommandRegistry;
use rustcode_platform::configuration::ConfigurationService;
use rustcode_platform::keybindings::KeybindingService;
use rustcode_platform::render::{
    EventResult, FontMetrics, InputEvent, Rect, RenderSurface, Scene, View,
    WorkbenchThemeStyles,
};
use rustcode_platform::theme::ThemeService;
use rustcode_protocols::lsp::{
    CodeAction, CodeActionContext, FormattingOptions, LspPosition, LspRange, WorkspaceEdit,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub struct WorkbenchWindow {
    pub view: WorkbenchView,
    pub config: ConfigurationService,
    pub theme: ThemeService,
    pub keybindings: KeybindingService,
    pub commands: CommandRegistry,
    pub context_keys: ContextKeyService,
    pub font_metrics: FontMetrics,
    pub language_bridge: Option<Arc<LanguageFeatureBridge>>,
    pub pending_code_actions: Vec<CodeAction>,
    pub extension_disposables: HashMap<String, DisposableStore>,
    pub custom_editor_contributions: Vec<CustomEditorContribution>,
    synced_output_revision: u64,
    synced_terminal_revision: u64,
    synced_webview_revision: u64,
    synced_custom_editor_revision: u64,
    ext_terminal_map: HashMap<String, String>,
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

        let mut win = Self {
            view,
            config: ConfigurationService::new(),
            theme,
            keybindings,
            commands,
            context_keys,
            font_metrics,
            language_bridge: None,
            pending_code_actions: Vec::new(),
            extension_disposables: HashMap::new(),
            custom_editor_contributions: Vec::new(),
            synced_output_revision: 0,
            synced_terminal_revision: 0,
            synced_webview_revision: 0,
            synced_custom_editor_revision: 0,
            ext_terminal_map: HashMap::new(),
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

    pub fn set_config(&mut self, config: ConfigurationService) {
        self.config = config;
        self.sync_configuration();
    }

    pub fn accessibility_tree(&self, bounds: Rect) -> AccessibilityTree {
        self.view.accessibility_tree(bounds, &self.font_metrics)
    }

    pub fn sync_configuration(&mut self) {
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
                            self.view.layout.panel_visible = false;
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
                        if let Some(term) = self.view.terminal_panel.manager.terminal_mut(&local_id)
                        {
                            term.insert_str(&text);
                            if add_new_line {
                                term.submit_input();
                            }
                        }
                    }
                }
                ExtTerminalEvent::Disposed { id } => {
                    if let Some(local_id) = self.ext_terminal_map.remove(&id) {
                        self.view.terminal_panel.manager.close_terminal(&local_id);
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
            return;
        };
        let rev = bridge.webview_manager.revision();
        if rev == self.synced_webview_revision {
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
                    .unwrap_or_else(|| panel_id.to_string())
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
                    self.view.editor_area.open_webview(
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
                        if !is_custom && self.view.editor_area.any_webview_for_uri(&id).is_none() {
                            let (html, options) = bridge
                                .webview_manager
                                .get_panel(&id)
                                .map(|p| (p.html, p.options))
                                .unwrap_or_default();
                            self.view.editor_area.open_webview(
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
                            .unwrap_or_else(|| id.clone());
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

    pub fn with_language_bridge(mut self, bridge: Arc<LanguageFeatureBridge>) -> Self {
        self.language_bridge = Some(bridge);
        self.sync_output_channels();
        self.sync_ext_terminals();
        self.sync_custom_editors();
        self.sync_webviews();
        self
    }

    pub fn set_language_bridge(&mut self, bridge: Arc<LanguageFeatureBridge>) {
        self.language_bridge = Some(bridge);
        self.sync_output_channels();
        self.sync_ext_terminals();
        self.sync_custom_editors();
        self.sync_webviews();
    }

    /// Renders the workbench window into the given render surface and returns the display list.
    pub fn render(&self, surface: &mut dyn RenderSurface) -> Scene {
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
        // 1. If key down, check keybindings evaluation first (unless modal command palette is open)
        if !self.view.is_command_palette_open {
            if let InputEvent::KeyDown(key_ev) = event {
                let combo = key_ev.to_key_combo();
                if let Some(cmd_id) = self.keybindings.handle_key(&combo, &self.context_keys) {
                    let _ = self.commands.execute(&cmd_id, None);
                    self.handle_builtin_command_effects(&cmd_id);
                    self.sync_output_channels();
                    self.sync_ext_terminals();
                    self.sync_custom_editors();
                    self.sync_webviews();
                    self.sync_focus_context_keys();
                    self.sync_configuration();
                    return EventResult::Consumed;
                }
            }
        }

        // 2. Route event to workbench view hierarchy
        let res = self.view.handle_event(event, bounds);

        // 3. Update context keys from focused region
        self.sync_focus_context_keys();

        // 4. Process any actions emitted by views
        if let Some(action) = self.view.last_action.take() {
            match action {
                WorkbenchAction::ExecuteCommand(cmd) => {
                    let _ = self.commands.execute(&cmd, None);
                    self.handle_builtin_command_effects(&cmd);
                    self.sync_focus_context_keys();
                }
                WorkbenchAction::OpenFile(path) => {
                    let path_str = path.to_string_lossy().to_string();
                    let title = path.file_name().map_or("untitled", |n| n.to_str().unwrap_or(""));
                    let content = std::fs::read_to_string(&path).unwrap_or_default();
                    self.open_file(&path_str, title, &content, false);
                    self.sync_focus_context_keys();
                }
                WorkbenchAction::JumpToDiagnostic { .. } => {}
                WorkbenchAction::SidebarToggled(_) => {}
                WorkbenchAction::PanelToggled(_) => {}
                WorkbenchAction::OpenDiff { .. } => {}
                WorkbenchAction::DebugToolbar(_) => {}
                WorkbenchAction::DebugAction(_) => {}
                WorkbenchAction::ScmAction(_) => {}
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
                WorkbenchAction::TerminalAction(_) => {}
            }
        }

        if !matches!(event, InputEvent::MouseMove(_)) {
            self.sync_output_channels();
            self.sync_ext_terminals();
            self.sync_custom_editors();
            self.sync_webviews();
            self.sync_configuration();
        }

        res
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
                    self.view.layout.panel_visible = false;
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
                    self.view.layout.panel_visible = false;
                    if self.view.focused_region == FocusedRegion::Panel {
                        self.view.focused_region = FocusedRegion::Editor;
                    }
                }
            }
            "workbench.action.output.toggleOutput" => {
                if self.view.layout.panel_visible && self.view.layout.active_panel_tab == "output" {
                    self.view.layout.panel_visible = false;
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
                self.view.layout.panel_visible = true;
                self.view.layout.set_panel_tab("problems");
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
            "workbench.action.findInFiles" => {
                self.view.layout.sidebar_visible = true;
                self.view.layout.set_sidebar_view("search");
                self.view.focused_region = crate::workbench_view::FocusedRegion::Sidebar;
            }
            "workbench.view.extensions" => {
                self.view.layout.active_sidebar_view = "extensions".to_string();
                self.view.layout.sidebar_visible = true;
                self.view.focused_region = crate::workbench_view::FocusedRegion::Sidebar;
                self.view.activity_bar.set_active_index(4);
            }
            "workbench.extensions.action.showInstalledExtensions" => {
                self.view.layout.active_sidebar_view = "extensions".to_string();
                self.view.layout.sidebar_visible = true;
                self.view.focused_region = crate::workbench_view::FocusedRegion::Sidebar;
                self.view.activity_bar.set_active_index(4);
                self.view.extensions_view.apply_search_query("");
                self.view.extensions_view.installed_collapsed = false;
                self.view.extensions_view.refresh_installed();
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
            }
            "workbench.action.splitEditorDown" => {
                self.view
                    .editor_area
                    .split_active_group(SplitDirection::Vertical);
                self.view.focused_region = FocusedRegion::Editor;
                self.view
                    .editor_area
                    .sync_breadcrumbs(Some(&self.view.workspace));
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
            }
            "redo" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.redo();
                }
            }
            "editor.action.triggerSuggest" => {
                if let Some(editor) = self.view.editor_area.active_editor_mut() {
                    editor.trigger_suggest(None);
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
                // Formatting is triggered asynchronously via format_active_document
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
            _ => cmd_id
                .replace("workbench.action.files.", "")
                .replace("workbench.action.", "")
                .replace("editor.action.", ""),
        }
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
                s.location.range.start.character as usize,
            ));
        }

        let mut palette = CommandPalette::new();
        palette.set_items(items);
        palette.open_mode(crate::quickinput::QuickInputMode::WorkspaceSymbol);
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
    }

    pub async fn format_active_document(&mut self) -> bool {
        let (uri, lang_id) = match self.view.editor_area.model.current_group_ref().active_tab() {
            Some(tab) => {
                let uri = tab.uri.clone();
                let lang = self
                    .view
                    .editor_area
                    .active_editor()
                    .and_then(|ed| ed.document.as_ref())
                    .map(|doc| doc.read().language_id.clone())
                    .unwrap_or_else(|| "*".to_string());
                (uri, lang)
            }
            None => return false,
        };

        if let Some(bridge) = self.language_bridge.clone() {
            let options = FormattingOptions::new(4, true);
            let edits = bridge.request_formatting(&uri, &lang_id, options).await;

            if !edits.is_empty() {
                let range_edits: Vec<rustcode_editor::RangeEdit> = edits
                    .into_iter()
                    .map(|e| rustcode_editor::RangeEdit {
                        range: rustcode_editor::Range::new(
                            rustcode_editor::Position::new(
                                e.range.start.line as usize,
                                e.range.start.character as usize,
                            ),
                            rustcode_editor::Position::new(
                                e.range.end.line as usize,
                                e.range.end.character as usize,
                            ),
                        ),
                        new_text: e.new_text,
                    })
                    .collect();
                let applied = self.view.editor_area.apply_text_edits(&uri, &range_edits);
                if applied {
                    self.sync_configuration();
                }
                return applied;
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
            self.sync_configuration();
        }
        executed
    }

    pub fn apply_workspace_edit(&mut self, edit: &WorkspaceEdit) -> usize {
        let count = self.view.editor_area.apply_workspace_edit(edit);
        if count > 0 {
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
}
