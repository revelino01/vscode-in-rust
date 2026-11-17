//! Keybinding service supporting multi-chords and VS Code `when` clauses.

use parking_lot::RwLock;
use rustcode_base::context_key::{ContextKeyExpr, ContextKeyService};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    pub fn none() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyCombo {
    pub modifiers: Modifiers,
    pub key: String, // lowercase normalized: "p", "f1", "enter", "backspace"
}

impl KeyCombo {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        // Special case for "+" or keys ending with "++" (e.g. "ctrl++")
        if s == "+" {
            return Some(Self {
                modifiers: Modifiers::none(),
                key: "+".to_string(),
            });
        }

        let (mod_part, key_part) = if let Some(prefix) = s.strip_suffix("++") {
            (prefix, "+")
        } else {
            let parts: Vec<&str> = s.rsplitn(2, '+').collect();
            if parts.len() == 2 {
                (parts[1], parts[0])
            } else {
                ("", parts[0])
            }
        };

        let mut modifiers = Modifiers::none();
        if !mod_part.is_empty() {
            for part in mod_part.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()) {
                let lower = part.to_ascii_lowercase();
                match lower.as_str() {
                    "ctrl" | "control" => modifiers.ctrl = true,
                    "shift" => modifiers.shift = true,
                    "alt" | "opt" | "option" => modifiers.alt = true,
                    "cmd" | "command" | "meta" | "win" => modifiers.meta = true,
                    _ => {}
                }
            }
        }

        let key = key_part.trim().to_ascii_lowercase();
        if key.is_empty() {
            None
        } else {
            Some(Self { modifiers, key })
        }
    }

    pub fn to_display_string(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.ctrl {
            parts.push("Ctrl");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }
        if self.modifiers.meta {
            parts.push("Cmd");
        }
        let key_display = match self.key.as_str() {
            " " => "Space".to_string(),
            k if k.len() == 1 => k.to_ascii_uppercase(),
            k => {
                let mut c = k.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            }
        };
        parts.push(&key_display);
        parts.join("+")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub first: KeyCombo,
    pub second: Option<KeyCombo>,
}

impl KeyChord {
    pub fn parse(s: &str) -> Option<Self> {
        let chords: Vec<&str> = s.split_whitespace().collect();
        if chords.is_empty() {
            return None;
        }

        let first = KeyCombo::parse(chords[0])?;
        let second = if chords.len() > 1 {
            Some(KeyCombo::parse(chords[1])?)
        } else {
            None
        };

        Some(Self { first, second })
    }

    pub fn to_display_string(&self) -> String {
        match &self.second {
            Some(sec) => format!("{} {}", self.first.to_display_string(), sec.to_display_string()),
            None => self.first.to_display_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedKeybinding {
    pub chord: KeyChord,
    pub command: String,
    pub args: Option<Value>,
    pub when: Option<ContextKeyExpr>,
}

#[derive(Debug, Deserialize)]
struct KeybindingEntryRaw {
    key: String,
    command: String,
    args: Option<Value>,
    when: Option<String>,
}

#[derive(Clone)]
pub struct KeybindingService {
    bindings: Arc<RwLock<Vec<ResolvedKeybinding>>>,
    pending_chord: Arc<RwLock<Option<KeyCombo>>>,
}

impl Default for KeybindingService {
    fn default() -> Self {
        Self::new()
    }
}

impl KeybindingService {
    pub fn new() -> Self {
        Self {
            bindings: Arc::new(RwLock::new(Vec::new())),
            pending_chord: Arc::new(RwLock::new(None)),
        }
    }

    /// Register a single keybinding.
    pub fn register(&self, binding: ResolvedKeybinding) {
        self.bindings.write().push(binding);
    }

    /// Lookup formatted keybinding display string for a command ID.
    pub fn lookup_keybinding(&self, command: &str) -> Option<String> {
        let bindings = self.bindings.read();
        for b in bindings.iter() {
            if b.command == command {
                return Some(b.chord.to_display_string());
            }
        }
        None
    }

    /// Load keybindings from a JSON string matching VS Code's `keybindings.json` format (supports JSONC).
    pub fn load_keybindings_json(&self, json_str: &str) -> Result<(), String> {
        let stripped = crate::configuration::strip_json_comments(json_str);
        let entries: Vec<KeybindingEntryRaw> = serde_json::from_str(&stripped).map_err(|e| e.to_string())?;
        let mut list = self.bindings.write();

        for raw in entries {
            if let Some(chord) = KeyChord::parse(&raw.key) {
                let when = raw.when.and_then(|w| ContextKeyExpr::parse(&w).ok());
                list.push(ResolvedKeybinding {
                    chord,
                    command: raw.command,
                    args: raw.args,
                    when,
                });
            }
        }
        Ok(())
    }

    /// Process a key press event. Returns Some(command_id) if a binding triggered,
    /// or None if no match or if awaiting the second key of a chord.
    pub fn handle_key(&self, key: &KeyCombo, context: &ContextKeyService) -> Option<String> {
        let mut pending = self.pending_chord.write();

        if let Some(first_key) = pending.take() {
            // We were in a chord, looking for chord match
            let current_chord = KeyChord {
                first: first_key,
                second: Some(key.clone()),
            };

            let bindings = self.bindings.read();
            for binding in bindings.iter().rev() {
                if binding.chord == current_chord {
                    if let Some(ref when) = binding.when {
                        if !when.evaluate(context) {
                            continue;
                        }
                    }
                    return Some(binding.command.clone());
                }
            }
            return None;
        }

        // Check if this key could be the first part of a two-key chord
        let bindings = self.bindings.read();
        let is_chord_prefix = bindings.iter().any(|b| b.chord.second.is_some() && b.chord.first == *key);
        if is_chord_prefix {
            *pending = Some(key.clone());
            return None;
        }

        // Match single key combo
        let single_chord = KeyChord {
            first: key.clone(),
            second: None,
        };

        for binding in bindings.iter().rev() {
            if binding.chord == single_chord {
                if let Some(ref when) = binding.when {
                    if !when.evaluate(context) {
                        continue;
                    }
                }
                return Some(binding.command.clone());
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keybinding_parsing_and_resolution() {
        let service = KeybindingService::new();
        let context = ContextKeyService::new();

        let json = r#"[
            {
                "key": "ctrl+shift+p",
                "command": "workbench.action.showCommands"
            },
            {
                "key": "ctrl+k ctrl+c",
                "command": "editor.action.addCommentLine",
                "when": "editorTextFocus"
            }
        ]"#;

        service.load_keybindings_json(json).unwrap();

        // Test single keybinding
        let p_combo = KeyCombo::parse("ctrl+shift+p").unwrap();
        let cmd = service.handle_key(&p_combo, &context);
        assert_eq!(cmd, Some("workbench.action.showCommands".to_string()));

        // Test chord: first key should return None
        let k_combo = KeyCombo::parse("ctrl+k").unwrap();
        assert_eq!(service.handle_key(&k_combo, &context), None);

        // Second key without context: when condition fails
        let c_combo = KeyCombo::parse("ctrl+c").unwrap();
        assert_eq!(service.handle_key(&c_combo, &context), None);

        // Second attempt with context active
        assert_eq!(service.handle_key(&k_combo, &context), None);
        context.set_bool("editorTextFocus", true);
        let chord_cmd = service.handle_key(&c_combo, &context);
        assert_eq!(chord_cmd, Some("editor.action.addCommentLine".to_string()));
    }

    #[test]
    fn test_keybinding_plus_and_jsonc() {
        let zoom_in = KeyCombo::parse("ctrl++").unwrap();
        assert!(zoom_in.modifiers.ctrl);
        assert_eq!(zoom_in.key, "+");

        let plus_only = KeyCombo::parse("+").unwrap();
        assert_eq!(plus_only.key, "+");
        assert!(!plus_only.modifiers.ctrl);

        // JSONC with comments & trailing comma
        let service = KeybindingService::new();
        let jsonc = r#"[
            // Zoom in
            {
                "key": "ctrl++",
                "command": "workbench.action.zoomIn",
            },
        ]"#;
        service.load_keybindings_json(jsonc).unwrap();
        let ctx = ContextKeyService::new();
        assert_eq!(service.handle_key(&zoom_in, &ctx), Some("workbench.action.zoomIn".to_string()));
    }

    #[test]
    fn test_keybinding_display_string_and_lookup() {
        let service = KeybindingService::new();
        let json = r#"[
            { "key": "ctrl+shift+p", "command": "workbench.action.showCommands" },
            { "key": "ctrl+k ctrl+c", "command": "editor.action.commentLine" }
        ]"#;
        service.load_keybindings_json(json).unwrap();

        assert_eq!(
            service.lookup_keybinding("workbench.action.showCommands"),
            Some("Ctrl+Shift+P".to_string())
        );
        assert_eq!(
            service.lookup_keybinding("editor.action.commentLine"),
            Some("Ctrl+K Ctrl+C".to_string())
        );
        assert_eq!(service.lookup_keybinding("unknown.command"), None);
    }
}
