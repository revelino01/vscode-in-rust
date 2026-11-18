//! GPUI WorkbenchWindow host managing rendering surfaces, theme updates, and input routing.

use crate::quickinput::{CommandPalette, CommandPaletteView, PaletteItem};
use crate::workbench_view::{WorkbenchAction, WorkbenchView};
use rustcode_base::context_key::ContextKeyService;
use rustcode_ext_host::provider::LanguageFeatureBridge;
use rustcode_platform::commands::CommandRegistry;
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
            theme,
            keybindings,
            commands,
            context_keys,
            font_metrics,
            language_bridge: None,
            pending_code_actions: Vec::new(),
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
                    return EventResult::Consumed;
                }
            }
        }

        // 2. Route event to workbench view hierarchy
        let res = self.view.handle_event(event, bounds);

        // 3. Update context keys from focused region
        match self.view.focused_region {
            crate::workbench_view::FocusedRegion::Editor => {
                self.context_keys.set_bool("editorTextFocus", true);
                self.context_keys.set_bool("sideBarFocus", false);
            }
            crate::workbench_view::FocusedRegion::Sidebar => {
                self.context_keys.set_bool("editorTextFocus", false);
                self.context_keys.set_bool("sideBarFocus", true);
            }
            crate::workbench_view::FocusedRegion::CommandPalette => {
                self.context_keys.set_bool("inCommandPalette", true);
            }
            _ => {}
        }

        // 4. Process any actions emitted by views
        if let Some(action) = self.view.last_action.take() {
            match action {
                WorkbenchAction::ExecuteCommand(cmd) => {
                    let _ = self.commands.execute(&cmd, None);
                    self.handle_builtin_command_effects(&cmd);
                }
                WorkbenchAction::OpenFile(path) => {
                    let path_str = path.to_string_lossy().to_string();
                    let title = path.file_name().map_or("untitled", |n| n.to_str().unwrap_or(""));
                    let content = std::fs::read_to_string(&path).unwrap_or_default();
                    self.open_file(&path_str, title, &content, false);
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
            "workbench.action.closeActiveEditor" => {
                if let Some(active_tab) = self.view.editor_area.model.current_group_ref().active_tab_index {
                    self.view.editor_area.model.current_group().close_tab(active_tab);
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
                cmd_id.replace("workbench.action.", "").replace("editor.action.", ""),
                Some("Command".to_string()),
                keybinding,
            ));
        }

        let mut palette = CommandPalette::new();
        palette.set_items(items);
        palette.open_mode(crate::quickinput::QuickInputMode::Command);
        self.view.open_command_palette(CommandPaletteView::new(palette));
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
                cmd_id.replace("workbench.action.", "").replace("editor.action.", ""),
                Some("Command".to_string()),
                keybinding,
            ));
        }

        let mut palette = CommandPalette::new();
        palette.set_items(items);
        palette.open_mode(crate::quickinput::QuickInputMode::File);
        self.view.open_command_palette(CommandPaletteView::new(palette));
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
    }

    pub fn open_file(&mut self, uri: &str, title: &str, content: &str, is_preview: bool) {
        self.view
            .editor_area
            .open_file(uri, title, content, is_preview);
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
}
