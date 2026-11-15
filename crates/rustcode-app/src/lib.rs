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

        store.add_boxed(self.commands.register("workbench.action.openSettings", |_| {
            Ok(json!({ "opened": "settings.json" }))
        }));

        store.add_boxed(self.commands.register("workbench.action.showCommands", |_| {
            Ok(json!({ "palette": "opened" }))
        }));
    }

    /// Instantiate a GPUI WorkbenchWindow host wired to this service container.
    pub fn create_window(&self) -> WorkbenchWindow {
        WorkbenchWindow::new(
            self.theme.clone(),
            self.keybindings.clone(),
            self.commands.clone(),
            self.context_keys.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rustcode_bootstrap() {
        let app = RustCodeContext::new();
        assert!(app.commands.has_command("workbench.action.toggleSidebar"));
        assert_eq!(app.config.get::<u32>("editor.fontSize"), Some(14));

        let window = app.create_window();
        assert!(window.view.layout.sidebar_visible);
    }
}
