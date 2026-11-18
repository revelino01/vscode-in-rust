//! GPUI WorkbenchWindow host managing rendering surfaces, theme updates, and input routing.

use crate::quickinput::{CommandPalette, CommandPaletteView, PaletteItem};
use crate::workbench_view::{WorkbenchAction, WorkbenchView};
use rustcode_base::context_key::ContextKeyService;
use rustcode_ext_host::provider::LanguageFeatureBridge;
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

        Self {
            view,
            config: ConfigurationService::new(),
            theme,
            keybindings,
            commands,
            context_keys,
            font_metrics,
            language_bridge: None,
            pending_code_actions: Vec::new(),
        }
    }

    pub fn with_config(mut self, config: ConfigurationService) -> Self {
        self.config = config;
        self
    }

    pub fn set_config(&mut self, config: ConfigurationService) {
        self.config = config;
    }

    pub fn open_settings(&mut self) {
        self.view.open_settings(Some(self.config.clone()));
        self.sync_focus_context_keys();
    }

    pub fn new_untitled_notebook(&mut self) {
        self.view.new_untitled_notebook();
        self.sync_focus_context_keys();
    }

    fn sync_focus_context_keys(&mut self) {
        match self.view.focused_region {
            crate::workbench_view::FocusedRegion::Editor => {
                self.context_keys.set_bool("editorTextFocus", true);
                self.context_keys.set_bool("sideBarFocus", false);
                self.context_keys.set_bool("inCommandPalette", false);
                self.context_keys.set_bool(
                    "notebookEditorFocused",
                    self.view.editor_area.active_notebook().is_some(),
                );
            }
            crate::workbench_view::FocusedRegion::Sidebar => {
                self.context_keys.set_bool("editorTextFocus", false);
                self.context_keys.set_bool("sideBarFocus", true);
                self.context_keys.set_bool("inCommandPalette", false);
                self.context_keys.set_bool("notebookEditorFocused", false);
            }
            crate::workbench_view::FocusedRegion::CommandPalette => {
                self.context_keys.set_bool("inCommandPalette", true);
                self.context_keys.set_bool("notebookEditorFocused", false);
            }
            _ => {
                self.context_keys.set_bool("inCommandPalette", false);
                self.context_keys.set_bool("notebookEditorFocused", false);
            }
        }
    }

    pub fn with_language_bridge(mut self, bridge: Arc<LanguageFeatureBridge>) -> Self {
        self.language_bridge = Some(bridge);
        self
    }

    pub fn set_language_bridge(&mut self, bridge: Arc<LanguageFeatureBridge>) {
        self.language_bridge = Some(bridge);
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
                    self.sync_focus_context_keys();
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
            }
        }

        res
    }

    fn handle_builtin_command_effects(&mut self, command_id: &str) {
        match command_id {
            "workbench.action.toggleSidebar" => {
                self.view.toggle_sidebar();
            }
            "workbench.action.togglePanel" | "workbench.action.terminal.toggleTerminal" => {
                self.view.toggle_panel();
            }
            "workbench.action.problems.focus" => {
                self.view.layout.panel_visible = true;
                self.view.layout.set_panel_tab("problems");
                self.view.focused_region = crate::workbench_view::FocusedRegion::Panel;
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
                }
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

    pub fn open_command_palette(&mut self) {
        let mut items = Vec::new();
        for cmd_id in self.commands.list_commands() {
            let keybinding = self.keybindings.lookup_keybinding(&cmd_id);
            items.push(PaletteItem::command(
                cmd_id.clone(),
                cmd_id
                    .replace("workbench.action.files.", "")
                    .replace("workbench.action.", "")
                    .replace("editor.action.", ""),
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
                cmd_id
                    .replace("workbench.action.files.", "")
                    .replace("workbench.action.", "")
                    .replace("editor.action.", ""),
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
            let path_str = s.location.uri.strip_prefix("file://").unwrap_or(&s.location.uri);
            items.push(PaletteItem::symbol(
                s.name,
                s.container_name,
                std::path::PathBuf::from(path_str),
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
        self.view
            .editor_area
            .open_file(uri, title, content, is_preview);
        self.view.focused_region = crate::workbench_view::FocusedRegion::Editor;
        self.sync_focus_context_keys();
    }

    pub async fn format_active_document(&mut self) -> bool {
        let (uri, lang_id) = match self.view.editor_area.model.current_group_ref().active_tab() {
            Some(tab) => {
                let uri = tab.uri.clone();
                let lang = self
                    .view
                    .editor_area
                    .editors
                    .get(&uri)
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
                return self.view.editor_area.apply_text_edits(&uri, &range_edits);
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
                let (lang, range) = if let Some(ed) = self.view.editor_area.editors.get(&uri) {
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
        executed
    }

    pub fn apply_workspace_edit(&mut self, edit: &WorkspaceEdit) -> usize {
        self.view.editor_area.apply_workspace_edit(edit)
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
}
