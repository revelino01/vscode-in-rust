//! RustCode application service container and GPUI window bootstrap.

use parking_lot::Mutex;
use rustcode_base::context_key::ContextKeyService;
use rustcode_base::disposable::DisposableStore;
use rustcode_ext_host::ExtensionLoader;
use rustcode_platform::{CommandRegistry, ConfigurationService, KeybindingService, ThemeService};
use rustcode_workbench::{EditorArea, FileExplorer, WorkbenchLayout};
pub use rustcode_workbench::WorkbenchWindow;
use serde_json::json;
use std::sync::Arc;

pub struct RustCodeContext {
    pub config: ConfigurationService,
    pub commands: CommandRegistry,
    pub keybindings: KeybindingService,
    pub context_keys: ContextKeyService,
    pub theme: ThemeService,
    pub layout: WorkbenchLayout,
    pub editor_area: EditorArea,
    pub explorer: FileExplorer,
    pub ext_loader: ExtensionLoader,
    pub disposables: Arc<Mutex<DisposableStore>>,
}

impl Default for RustCodeContext {
    fn default() -> Self {
        Self::new()
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
        let layout = WorkbenchLayout::new();
        let editor_area = EditorArea::new();
        let explorer = FileExplorer::new();
        let ext_loader = ExtensionLoader::new();
        let disposables = Arc::new(Mutex::new(DisposableStore::new()));

        let ctx = Self {
            config,
            commands,
            keybindings,
            context_keys,
            theme,
            layout,
            editor_area,
            explorer,
            ext_loader,
            disposables,
        };
        ctx.register_builtin_commands();
        ctx
    }

    fn register_builtin_commands(&self) {
        let mut store = self.disposables.lock();

        store.add_boxed(self.commands.register("workbench.action.toggleSidebar", |_| {
            Ok(json!({ "toggled": true }))
        }));

        store.add_boxed(self.commands.register("workbench.action.togglePanel", |_| {
            Ok(json!({ "panel_toggled": true }))
        }));

        store.add_boxed(self.commands.register("workbench.action.problems.focus", |_| {
            Ok(json!({ "focused": "problems" }))
        }));

        store.add_boxed(
            self.commands
                .register("workbench.action.terminal.toggleTerminal", |_| {
                    Ok(json!({ "terminal_toggled": true }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.terminal.new", |_| {
                    Ok(json!({ "terminal": "created" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.terminal.split", |_| {
                    Ok(json!({ "terminal": "split" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.terminal.kill", |_| {
                    Ok(json!({ "terminal": "killed" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.terminal.focus", |_| {
                    Ok(json!({ "focused": "terminal" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.output.toggleOutput", |_| {
                    Ok(json!({ "output_toggled": true }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.output.clearOutput", |_| {
                    Ok(json!({ "output_cleared": true }))
                }),
        );

        store.add_boxed(self.commands.register("workbench.action.openSettings", |_| {
            Ok(json!({ "opened": "settings.json" }))
        }));

        store.add_boxed(self.commands.register("workbench.action.showCommands", |_| {
            Ok(json!({ "palette": "opened" }))
        }));

        store.add_boxed(self.commands.register("editor.action.triggerSuggest", |_| {
            Ok(json!({ "suggest": "triggered" }))
        }));

        store.add_boxed(
            self.commands
                .register("editor.action.insertCursorAbove", |_| {
                    Ok(json!({ "cursor": "above" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("editor.action.insertCursorBelow", |_| {
                    Ok(json!({ "cursor": "below" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("editor.action.addSelectionToNextFindMatch", |_| {
                    Ok(json!({ "nextFindMatch": true }))
                }),
        );

        store.add_boxed(self.commands.register("editor.fold", |_| {
            Ok(json!({ "folded": true }))
        }));

        store.add_boxed(self.commands.register("editor.unfold", |_| {
            Ok(json!({ "unfolded": true }))
        }));

        store.add_boxed(self.commands.register("editor.foldAll", |_| {
            Ok(json!({ "foldedAll": true }))
        }));

        store.add_boxed(self.commands.register("editor.unfoldAll", |_| {
            Ok(json!({ "unfoldedAll": true }))
        }));

        store.add_boxed(self.commands.register("editor.action.formatDocument", |_| {
            Ok(json!({ "formatted": true }))
        }));

        store.add_boxed(self.commands.register("editor.action.quickFix", |_| {
            Ok(json!({ "quickFix": true }))
        }));

        store.add_boxed(
            self.commands
                .register("workbench.action.files.newUntitledNotebook", |_| {
                    Ok(json!({ "notebook": "created" }))
                }),
        );

        store.add_boxed(self.commands.register("notebook.execute", |_| {
            Ok(json!({ "notebookExecuted": true }))
        }));

        store.add_boxed(self.commands.register("notebook.cell.execute", |_| {
            Ok(json!({ "cellExecuted": true }))
        }));

        store.add_boxed(self.commands.register("workbench.view.extensions", |_| {
            Ok(json!({ "view": "extensions" }))
        }));

        store.add_boxed(
            self.commands
                .register("workbench.extensions.action.showInstalledExtensions", |_| {
                    Ok(json!({ "view": "extensions", "filter": "installed" }))
                }),
        );

        store.add_boxed(self.commands.register("workbench.action.splitEditor", |_| {
            Ok(json!({ "split": "horizontal" }))
        }));

        store.add_boxed(
            self.commands
                .register("workbench.action.splitEditorDown", |_| {
                    Ok(json!({ "split": "vertical" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.focusPreviousGroup", |_| {
                    Ok(json!({ "focus": "previousGroup" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.focusNextGroup", |_| {
                    Ok(json!({ "focus": "nextGroup" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.focusFirstEditorGroup", |_| {
                    Ok(json!({ "focus": "firstGroup" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.focusSecondEditorGroup", |_| {
                    Ok(json!({ "focus": "secondGroup" }))
                }),
        );

        store.add_boxed(
            self.commands
                .register("workbench.action.closeEditorsInGroup", |_| {
                    Ok(json!({ "closedGroupEditors": true }))
                }),
        );
    }

    /// Instantiate a GPUI WorkbenchWindow host wired to this service container.
    pub fn create_window(&self) -> WorkbenchWindow {
        let mut window = WorkbenchWindow::new(
            self.theme.clone(),
            self.keybindings.clone(),
            self.commands.clone(),
            self.context_keys.clone(),
        )
        .with_config(self.config.clone());
        for ext in self.ext_loader.loaded_extensions() {
            if let Some(contribs) = &ext.manifest.contributes {
                for ce in &contribs.custom_editors {
                    window.register_custom_editor_contribution(ce.clone());
                }
            }
        }
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
    }
}
