//! Central Command Registry mirroring VS Code's `ICommandService`.

use parking_lot::RwLock;
use rustcode_base::disposable::{to_disposable, Disposable};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub type CommandCallback = Arc<dyn Fn(Option<Value>) -> Result<Value, String> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct CommandRegistry {
    commands: Arc<RwLock<HashMap<String, CommandCallback>>>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a command handler. Returns a `Disposable` that unregisters the command.
    pub fn register<F>(&self, id: &str, handler: F) -> Box<dyn Disposable>
    where
        F: Fn(Option<Value>) -> Result<Value, String> + Send + Sync + 'static,
    {
        let id_str = id.to_string();
        self.commands.write().insert(id_str.clone(), Arc::new(handler));

        let commands = Arc::clone(&self.commands);
        to_disposable(move || {
            commands.write().remove(&id_str);
        })
    }

    /// Execute a registered command by ID, releasing the read lock before invocation.
    pub fn execute(&self, id: &str, args: Option<Value>) -> Result<Value, String> {
        let handler = {
            let guard = self.commands.read();
            guard.get(id).cloned().ok_or_else(|| format!("Command '{}' not found", id))?
        };
        handler(args)
    }

    pub fn has_command(&self, id: &str) -> bool {
        self.commands.read().contains_key(id)
    }

    pub fn list_commands(&self) -> Vec<String> {
        self.commands.read().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_command_registry() {
        let registry = CommandRegistry::new();

        let mut sub = registry.register("test.echo", |args| {
            let msg = args.unwrap_or(json!("default"));
            Ok(msg)
        });

        assert!(registry.has_command("test.echo"));
        let res = registry.execute("test.echo", Some(json!("hello"))).unwrap();
        assert_eq!(res, json!("hello"));

        sub.dispose();
        assert!(!registry.has_command("test.echo"));
        assert!(registry.execute("test.echo", None).is_err());
    }

    #[test]
    fn test_command_reentrant_registration() {
        use parking_lot::Mutex;
        let registry = Arc::new(CommandRegistry::new());
        let reg_clone = Arc::clone(&registry);
        let child_sub = Arc::new(Mutex::new(None));
        let child_sub_clone = Arc::clone(&child_sub);

        let _sub = registry.register("parent.command", move |_| {
            // Registering a child command inside a command handler must not deadlock
            let sub = reg_clone.register("child.command", |_| Ok(json!("child")));
            *child_sub_clone.lock() = Some(sub);
            Ok(json!("parent"))
        });

        let res = registry.execute("parent.command", None).unwrap();
        assert_eq!(res, json!("parent"));
        assert!(registry.has_command("child.command"));
    }
}
