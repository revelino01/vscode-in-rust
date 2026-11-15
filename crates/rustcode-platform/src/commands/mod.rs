//! Central Command Registry mirroring VS Code's `ICommandService`.

use parking_lot::RwLock;
use rustcode_base::disposable::{to_disposable, Disposable};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub type CommandCallback = Box<dyn Fn(Option<Value>) -> Result<Value, String> + Send + Sync + 'static>;

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
        self.commands.write().insert(id_str.clone(), Box::new(handler));

        let commands = Arc::clone(&self.commands);
        to_disposable(move || {
            commands.write().remove(&id_str);
        })
    }

    /// Execute a registered command by ID.
    pub fn execute(&self, id: &str, args: Option<Value>) -> Result<Value, String> {
        let guard = self.commands.read();
        let handler = guard.get(id).ok_or_else(|| format!("Command '{}' not found", id))?;
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
}
