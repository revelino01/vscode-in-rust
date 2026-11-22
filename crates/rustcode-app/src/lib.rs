//! RustCode application service container and GPUI window bootstrap.

pub mod cli;
#[cfg(feature = "desktop")]
pub mod desktop;
pub mod runtime;

use parking_lot::Mutex;
use rustcode_base::context_key::ContextKeyService;
use rustcode_base::disposable::DisposableStore;
use rustcode_ext_host::{
    ExtWorkspaceFolder, ExtensionLoader, LanguageFeatureBridge, ProviderRegistry, RpcBridge,
};
use rustcode_platform::{
    CommandRegistry, ConfigurationService, FileSystemService, KeybindingService, ThemeService,
};
use rustcode_protocols::lsp::LspClientManager;
use rustcode_workbench::layout::{LayoutHandle, SharedWorkbenchLayout};
use rustcode_workbench::window::WindowCommandSink;
pub use rustcode_workbench::WorkbenchWindow;
use rustcode_workbench::FileExplorer;
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

pub struct RustCodeContext {
    pub config: ConfigurationService,
    pub commands: CommandRegistry,
    pub keybindings: KeybindingService,
    pub context_keys: ContextKeyService,
    pub theme: ThemeService,
    pub layout: SharedWorkbenchLayout,
    pub explorer: FileExplorer,
    pub ext_loader: ExtensionLoader,
    pub file_system: Arc<FileSystemService>,
    pub language_bridge: Arc<LanguageFeatureBridge>,
    pub disposables: Arc<Mutex<DisposableStore>>,
    window_sinks: Arc<Mutex<Vec<Weak<WindowCommandSink>>>>,
}

impl Default for RustCodeContext {
    fn default() -> Self {
        Self::new()
    }
}

fn dispatch_to_sinks(
    sinks: &Arc<Mutex<Vec<Weak<WindowCommandSink>>>>,
    cmd_id: &str,
    mut f: impl FnMut(&WindowCommandSink),
) {
    let active: Vec<Arc<WindowCommandSink>> = {
        let mut guard = sinks.lock();
        guard.retain(|w| w.strong_count() > 0);
        guard.iter().filter_map(Weak::upgrade).collect()
    };
    for sink in active {
        let suppressed = sink
            .suppress_command
            .lock()
            .as_deref()
            .is_some_and(|s| s == cmd_id);
        if !suppressed {
            f(&sink);
        }
    }
}

impl RustCodeContext {
    pub fn new() -> Self {
        let config = ConfigurationService::new();
        let commands = CommandRegistry::new();
        let keybindings = KeybindingService::new();
        let _ = keybindings.load_keybindings_json(include_str!(
            "../../../resources/default-keymaps/linux.json"
        ));
        let context_keys = ContextKeyService::new();
        let theme = ThemeService::new();
        let layout = SharedWorkbenchLayout::new();
        let explorer = FileExplorer::new();
        let ext_loader = ExtensionLoader::new();
        let file_system = Arc::new(FileSystemService::new());
        let language_bridge = Arc::new(
            LanguageFeatureBridge::new(
                Arc::new(ProviderRegistry::new()),
                Arc::new(RpcBridge::new()),
                Arc::new(LspClientManager::new()),
            )
            .with_configuration_service(config.clone())
            .with_command_registry(commands.clone())
            .with_file_system_service(Arc::clone(&file_system))
            .with_os_env_backends(),
        );
        let disposables = Arc::new(Mutex::new(DisposableStore::new()));
        let window_sinks = Arc::new(Mutex::new(Vec::new()));

        let ctx = Self {
            config,
            commands,
            keybindings,
            context_keys,
            theme,
            layout,
            explorer,
            ext_loader,
            file_system,
            language_bridge,
            disposables,
            window_sinks,
        };
        ctx.register_builtin_commands();
        ctx
    }

    fn register_queue_command(
        &self,
        store: &mut DisposableStore,
        cmd_id: &'static str,
        payload: serde_json::Value,
    ) {
        let sinks = Arc::clone(&self.window_sinks);
        store.add_boxed(self.commands.register(cmd_id, move |_| {
            dispatch_to_sinks(&sinks, cmd_id, |s| {
                s.pending_commands.lock().push(cmd_id.to_string());
            });
            Ok(payload.clone())
        }));
    }

    fn register_sidebar_view_command(
        &self,
        store: &mut DisposableStore,
        app_layout: &LayoutHandle,
        cmd_id: &'static str,
        view_id: &'static str,
        payload: serde_json::Value,
    ) {
        let sinks = Arc::clone(&self.window_sinks);
        let layout_handle = app_layout.clone();
        store.add_boxed(self.commands.register(cmd_id, move |_| {
            layout_handle.update(|l| {
                l.active_sidebar_view = view_id.to_string();
                l.sidebar_visible = true;
            });
            dispatch_to_sinks(&sinks, cmd_id, |s| {
                s.layout.update(|l| {
                    l.active_sidebar_view = view_id.to_string();
                    l.sidebar_visible = true;
                });
                s.pending_commands.lock().push(cmd_id.to_string());
            });
            Ok(payload.clone())
        }));
    }

    fn register_builtin_commands(&self) {
        let mut store = self.disposables.lock();
        let app_layout = self.layout.handle();

        {
            let sinks = Arc::clone(&self.window_sinks);
            let layout_handle = app_layout.clone();
            store.add_boxed(self.commands.register("workbench.action.toggleSidebar", move |_| {
                layout_handle.update(|l| l.toggle_sidebar());
                dispatch_to_sinks(&sinks, "workbench.action.toggleSidebar", |s| {
                    s.layout.update(|l| l.toggle_sidebar());
                });
                Ok(json!({ "toggled": true }))
            }));
        }

        {
            let sinks = Arc::clone(&self.window_sinks);
            let layout_handle = app_layout.clone();
            store.add_boxed(self.commands.register("workbench.action.togglePanel", move |_| {
                layout_handle.update(|l| l.toggle_panel());
                dispatch_to_sinks(&sinks, "workbench.action.togglePanel", |s| {
                    s.layout.update(|l| l.toggle_panel());
                });
                Ok(json!({ "panel_toggled": true }))
            }));
        }

        {
            let sinks = Arc::clone(&self.window_sinks);
            let layout_handle = app_layout.clone();
            store.add_boxed(self.commands.register("workbench.action.problems.focus", move |_| {
                layout_handle.update(|l| {
                    l.panel_visible = true;
                    l.set_panel_tab("problems");
                });
                dispatch_to_sinks(&sinks, "workbench.action.problems.focus", |s| {
                    s.layout.update(|l| {
                        l.panel_visible = true;
                        l.set_panel_tab("problems");
                    });
                    s.pending_commands
                        .lock()
                        .push("workbench.action.problems.focus".to_string());
                });
                Ok(json!({ "focused": "problems" }))
            }));
        }

        {
            let sinks = Arc::clone(&self.window_sinks);
            let layout_handle = app_layout.clone();
            store.add_boxed(
                self.commands
                    .register("workbench.action.terminal.toggleTerminal", move |_| {
                        layout_handle.update(|l| {
                            if l.panel_visible && l.active_panel_tab == "terminal" {
                                l.panel_visible = false;
                            } else {
                                l.set_panel_tab("terminal");
                            }
                        });
                        dispatch_to_sinks(
                            &sinks,
                            "workbench.action.terminal.toggleTerminal",
                            |s| {
                                let opened = s.layout.update(|l| {
                                    if l.panel_visible && l.active_panel_tab == "terminal" {
                                        l.panel_visible = false;
                                        false
                                    } else {
                                        l.set_panel_tab("terminal");
                                        true
                                    }
                                });
                                if opened {
                                    s.pending_commands
                                        .lock()
                                        .push("workbench.action.terminal.focus".to_string());
                                }
                            },
                        );
                        Ok(json!({ "terminal_toggled": true }))
                    }),
            );
        }

        self.register_queue_command(
            &mut store,
            "workbench.action.terminal.new",
            json!({ "terminal": "created" }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.terminal.split",
            json!({ "terminal": "split" }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.terminal.kill",
            json!({ "terminal": "killed" }),
        );
        {
            let sinks = Arc::clone(&self.window_sinks);
            let layout_handle = app_layout.clone();
            store.add_boxed(
                self.commands
                    .register("workbench.action.terminal.focus", move |_| {
                        layout_handle.update(|l| l.set_panel_tab("terminal"));
                        dispatch_to_sinks(&sinks, "workbench.action.terminal.focus", |s| {
                            s.layout.update(|l| l.set_panel_tab("terminal"));
                            s.pending_commands
                                .lock()
                                .push("workbench.action.terminal.focus".to_string());
                        });
                        Ok(json!({ "focused": "terminal" }))
                    }),
            );
        }

        {
            let sinks = Arc::clone(&self.window_sinks);
            let layout_handle = app_layout.clone();
            store.add_boxed(
                self.commands
                    .register("workbench.action.output.toggleOutput", move |_| {
                        layout_handle.update(|l| {
                            if l.panel_visible && l.active_panel_tab == "output" {
                                l.panel_visible = false;
                            } else {
                                l.set_panel_tab("output");
                            }
                        });
                        dispatch_to_sinks(&sinks, "workbench.action.output.toggleOutput", |s| {
                            s.layout.update(|l| {
                                if l.panel_visible && l.active_panel_tab == "output" {
                                    l.panel_visible = false;
                                } else {
                                    l.set_panel_tab("output");
                                }
                            });
                        });
                        Ok(json!({ "output_toggled": true }))
                    }),
            );
        }

        self.register_queue_command(
            &mut store,
            "workbench.action.output.clearOutput",
            json!({ "output_cleared": true }),
        );

        {
            let sinks = Arc::clone(&self.window_sinks);
            store.add_boxed(self.commands.register("workbench.action.openSettings", move |_| {
                dispatch_to_sinks(&sinks, "workbench.action.openSettings", |s| {
                    s.pending_settings_open.store(true, Ordering::SeqCst);
                    s.pending_commands
                        .lock()
                        .push("workbench.action.openSettings".to_string());
                });
                Ok(json!({ "opened": "settings.json" }))
            }));
        }

        self.register_queue_command(
            &mut store,
            "workbench.action.files.save",
            json!({ "saved": true }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.closeActiveEditor",
            json!({ "closed": true }),
        );
        self.register_queue_command(&mut store, "undo", json!({ "undone": true }));
        self.register_queue_command(&mut store, "redo", json!({ "redone": true }));
        self.register_queue_command(
            &mut store,
            "workbench.action.showCommands",
            json!({ "palette": "opened" }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.quickOpen",
            json!({ "quickOpen": "opened" }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.showAllSymbols",
            json!({ "symbols": "opened" }),
        );
        self.register_queue_command(&mut store, "actions.find", json!({ "find": true }));
        self.register_queue_command(
            &mut store,
            "editor.action.startFindReplaceAction",
            json!({ "findReplace": true }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.gotoLine",
            json!({ "gotoLine": true }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.gotoSymbol",
            json!({ "gotoSymbol": true }),
        );
        self.register_queue_command(
            &mut store,
            "editor.action.triggerSuggest",
            json!({ "suggest": "triggered" }),
        );
        self.register_queue_command(
            &mut store,
            "editor.action.insertCursorAbove",
            json!({ "cursor": "above" }),
        );
        self.register_queue_command(
            &mut store,
            "editor.action.insertCursorBelow",
            json!({ "cursor": "below" }),
        );
        self.register_queue_command(
            &mut store,
            "editor.action.addSelectionToNextFindMatch",
            json!({ "nextFindMatch": true }),
        );
        self.register_queue_command(&mut store, "editor.fold", json!({ "folded": true }));
        self.register_queue_command(&mut store, "editor.unfold", json!({ "unfolded": true }));
        self.register_queue_command(&mut store, "editor.foldAll", json!({ "foldedAll": true }));
        self.register_queue_command(
            &mut store,
            "editor.unfoldAll",
            json!({ "unfoldedAll": true }),
        );
        self.register_queue_command(
            &mut store,
            "editor.action.formatDocument",
            json!({ "formatted": true }),
        );
        self.register_queue_command(
            &mut store,
            "editor.action.quickFix",
            json!({ "quickFix": true }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.files.newUntitledNotebook",
            json!({ "notebook": "created" }),
        );
        self.register_queue_command(
            &mut store,
            "notebook.execute",
            json!({ "notebookExecuted": true }),
        );
        self.register_queue_command(
            &mut store,
            "notebook.cell.execute",
            json!({ "cellExecuted": true }),
        );

        self.register_sidebar_view_command(
            &mut store,
            &app_layout,
            "workbench.view.explorer",
            "workbench.view.explorer",
            json!({ "view": "explorer" }),
        );
        self.register_sidebar_view_command(
            &mut store,
            &app_layout,
            "workbench.view.search",
            "search",
            json!({ "view": "search" }),
        );
        self.register_sidebar_view_command(
            &mut store,
            &app_layout,
            "workbench.action.findInFiles",
            "search",
            json!({ "view": "search" }),
        );
        self.register_sidebar_view_command(
            &mut store,
            &app_layout,
            "workbench.view.scm",
            "scm",
            json!({ "view": "scm" }),
        );
        self.register_sidebar_view_command(
            &mut store,
            &app_layout,
            "workbench.view.debug",
            "debug",
            json!({ "view": "debug" }),
        );
        self.register_sidebar_view_command(
            &mut store,
            &app_layout,
            "workbench.view.extensions",
            "extensions",
            json!({ "view": "extensions" }),
        );
        self.register_sidebar_view_command(
            &mut store,
            &app_layout,
            "workbench.extensions.action.showInstalledExtensions",
            "extensions",
            json!({ "view": "extensions", "filter": "installed" }),
        );
        self.register_sidebar_view_command(
            &mut store,
            &app_layout,
            "workbench.view.testing",
            "workbench.view.testing",
            json!({ "view": "testing" }),
        );

        self.register_queue_command(
            &mut store,
            "workbench.action.splitEditor",
            json!({ "split": "horizontal" }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.splitEditorDown",
            json!({ "split": "vertical" }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.focusPreviousGroup",
            json!({ "focus": "previousGroup" }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.focusNextGroup",
            json!({ "focus": "nextGroup" }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.focusFirstEditorGroup",
            json!({ "focus": "firstGroup" }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.focusSecondEditorGroup",
            json!({ "focus": "secondGroup" }),
        );
        self.register_queue_command(
            &mut store,
            "workbench.action.closeEditorsInGroup",
            json!({ "closedGroupEditors": true }),
        );
    }

    /// Instantiate a GPUI WorkbenchWindow host wired to this service container.
    pub fn create_window(&self) -> WorkbenchWindow {
        self.language_bridge
            .env_manager
            .hydrate_from_loader(&self.ext_loader);

        let folders: Vec<ExtWorkspaceFolder> = self
            .explorer
            .roots
            .iter()
            .enumerate()
            .map(|(idx, root)| ExtWorkspaceFolder::from_path(idx, &root.path))
            .collect();
        if !folders.is_empty() && self.language_bridge.workspace_folders().is_empty() {
            self.language_bridge.set_workspace_folders(folders);
        }

        let mut window = WorkbenchWindow::new(
            self.theme.clone(),
            self.keybindings.clone(),
            self.commands.clone(),
            self.context_keys.clone(),
        )
        .with_config(self.config.clone())
        .with_file_system(Arc::clone(&self.file_system))
        .with_language_bridge(Arc::clone(&self.language_bridge));

        for ext in self.ext_loader.loaded_extensions() {
            if let Some(contribs) = &ext.manifest.contributes {
                for ce in &contribs.custom_editors {
                    window.register_custom_editor_contribution(ce.clone());
                }
                if let Some(views) = &contribs.views {
                    for (container_id, view_list) in views {
                        window.register_view_contributions(container_id, view_list.clone());
                    }
                }
            }
        }

        self.window_sinks.lock().push(Arc::downgrade(&window.command_sink));
        window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rustcode_bootstrap() {
        let app = RustCodeContext::new();
        assert!(app.commands.has_command("workbench.action.toggleSidebar"));
        assert!(app.commands.has_command("editor.action.triggerSuggest"));
        assert!(app.commands.has_command("editor.action.formatDocument"));
        assert!(app.commands.has_command("editor.action.quickFix"));
        assert!(app.commands.has_command("workbench.action.splitEditor"));
        assert!(app.commands.has_command("workbench.action.splitEditorDown"));
        assert!(app.commands.has_command("workbench.action.focusPreviousGroup"));
        assert!(app.commands.has_command("workbench.action.focusNextGroup"));
        assert!(app.commands.has_command("workbench.action.focusFirstEditorGroup"));
        assert!(app.commands.has_command("workbench.action.focusSecondEditorGroup"));
        assert!(app.commands.has_command("workbench.action.closeEditorsInGroup"));
        assert_eq!(app.config.get::<u32>("editor.fontSize"), Some(14));

        let window = app.create_window();
        assert!(window.view.layout.sidebar_visible);
        assert!(window.language_bridge.is_some());
    }

    #[test]
    fn test_m1_3_commands_mutate_window_immediately_and_no_double_toggle() {
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{InputEvent, KeyCode, KeyEvent, MockRenderSurface, Rect};

        let app = RustCodeContext::new();
        for cmd in [
            "workbench.action.files.save",
            "workbench.view.testing",
            "actions.find",
            "editor.action.startFindReplaceAction",
            "workbench.action.gotoLine",
            "workbench.action.gotoSymbol",
            "undo",
            "redo",
            "workbench.action.quickOpen",
            "workbench.action.showAllSymbols",
            "workbench.action.findInFiles",
            "workbench.action.closeActiveEditor",
        ] {
            assert!(app.commands.has_command(cmd), "missing command: {cmd}");
        }

        let mut window = app.create_window();
        assert!(window.view.layout.sidebar_visible);
        assert!(!window.view.layout.panel_visible);

        // Immediate mutation via CommandRegistry::execute without render/dispatch_event
        app.commands
            .execute("workbench.action.toggleSidebar", None)
            .unwrap();
        assert!(!window.view.layout.sidebar_visible);

        app.commands
            .execute("workbench.action.togglePanel", None)
            .unwrap();
        assert!(window.view.layout.panel_visible);

        app.commands
            .execute("workbench.view.testing", None)
            .unwrap();
        assert!(window.view.layout.sidebar_visible);
        assert_eq!(
            window.view.layout.active_sidebar_view,
            "workbench.view.testing"
        );

        // openSettings via CommandRegistry::execute renders Settings without dispatch_event
        app.commands
            .execute("workbench.action.openSettings", None)
            .unwrap();
        let mut surface = MockRenderSurface::new(1024.0, 768.0);
        let scene = window.render(&mut surface);
        assert!(scene.contains_text("Settings"));

        // Ctrl+B via dispatch_event must toggle exactly once (no double-toggle)
        let before = window.view.layout.sidebar_visible;
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
        window.dispatch_event(&ctrl_b, bounds);
        assert_ne!(window.view.layout.sidebar_visible, before);
    }

    #[test]
    fn test_m4_create_window_wires_contributed_views() {
        use rustcode_platform::render::MockRenderSurface;

        let tmp = std::env::temp_dir().join(format!(
            "rustcode_app_m4_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let ext_dir = tmp.join("ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("package.json"),
            r#"{
                "name": "outline-demo",
                "publisher": "test",
                "version": "1.0.0",
                "contributes": {
                    "views": {
                        "explorer": [
                            { "id": "demoOutline", "name": "Demo Outline", "when": "hasOutline" }
                        ]
                    }
                }
            }"#,
        )
        .unwrap();

        let mut app = RustCodeContext::new();
        app.ext_loader.load_from_dir(&ext_dir).unwrap();
        let window = app.create_window();

        let mut surface = MockRenderSurface::new(1024.0, 768.0);
        assert!(!window.render(&mut surface).contains_text("Demo Outline"));

        app.context_keys.set_bool("hasOutline", true);
        assert!(window.render(&mut surface).contains_text("Demo Outline"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
