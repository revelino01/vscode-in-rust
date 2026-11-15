//! RustCode application entry point and service bootstrap.

use parking_lot::Mutex;
use rustcode_base::context_key::ContextKeyService;
use rustcode_base::disposable::DisposableStore;
use rustcode_ext_host::ExtensionLoader;
use rustcode_platform::{CommandRegistry, ConfigurationService, KeybindingService, ThemeService};
use rustcode_workbench::{EditorArea, FileExplorer, WorkbenchLayout};
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
            println!("[RustCode] Toggling sidebar visibility");
            Ok(json!({ "toggled": true }))
        }));

        store.add_boxed(self.commands.register("workbench.action.openSettings", |_| {
            println!("[RustCode] Opening settings");
            Ok(json!({ "opened": "settings.json" }))
        }));

        store.add_boxed(self.commands.register("workbench.action.showCommands", |_| {
            println!("[RustCode] Opening Command Palette");
            Ok(json!({ "palette": "opened" }))
        }));
    }
}

fn main() {
    println!("=======================================================");
    println!(" RustCode (vscode-in-rust) — 100% Vibe Coded in Rust ");
    println!(" High-performance, lightweight VS Code rewrite        ");
    println!("=======================================================");

    let app = RustCodeContext::new();

    let font_size: u32 = app.config.get("editor.fontSize").unwrap_or(14);
    println!("Theme: {} ({:?})", app.theme.current().name, app.theme.current().kind);
    println!("Editor Font Size: {}", font_size);
    println!("Active Sidebar: {}", app.layout.active_sidebar_view);

    // Test dispatching a built-in command
    let res = app.commands.execute("workbench.action.toggleSidebar", None).unwrap();
    println!("Executed builtin command: {}", res);

    println!("Bootstrap completed successfully.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rustcode_bootstrap() {
        let app = RustCodeContext::new();
        assert!(app.commands.has_command("workbench.action.toggleSidebar"));
        assert_eq!(app.config.get::<u32>("editor.fontSize"), Some(14));
    }
}
