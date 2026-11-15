//! Quick Input and Command Palette with fuzzy search.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteItem {
    pub command_id: String,
    pub label: String,
    pub category: Option<String>,
    pub keybinding: Option<String>,
}

pub struct CommandPalette {
    items: Vec<PaletteItem>,
    query: String,
    filtered: Vec<usize>,
    selected_index: usize,
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandPalette {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            query: String::new(),
            filtered: Vec::new(),
            selected_index: 0,
        }
    }

    pub fn set_items(&mut self, items: Vec<PaletteItem>) {
        self.items = items;
        self.refilter();
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.refilter();
    }

    pub fn results(&self) -> Vec<&PaletteItem> {
        self.filtered.iter().map(|&idx| &self.items[idx]).collect()
    }

    pub fn selected_item(&self) -> Option<&PaletteItem> {
        self.filtered.get(self.selected_index).map(|&idx| &self.items[idx])
    }

    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.filtered.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.filtered.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    fn refilter(&mut self) {
        let q = self.query.trim().to_ascii_lowercase();
        self.filtered.clear();
        self.selected_index = 0;

        if q.is_empty() {
            self.filtered.extend(0..self.items.len());
            return;
        }

        // Fuzzy match against label and category
        for (idx, item) in self.items.iter().enumerate() {
            let full_text = match &item.category {
                Some(cat) => format!("{}: {}", cat, item.label).to_ascii_lowercase(),
                None => item.label.to_ascii_lowercase(),
            };

            if fuzzy_match(&full_text, &q) {
                self.filtered.push(idx);
            }
        }
    }
}

/// Simple subsequence fuzzy matcher.
fn fuzzy_match(target: &str, pattern: &str) -> bool {
    let mut pattern_chars = pattern.chars().peekable();
    for c in target.chars() {
        if let Some(&pc) = pattern_chars.peek() {
            if c == pc {
                pattern_chars.next();
            }
        } else {
            return true;
        }
    }
    pattern_chars.peek().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_palette_fuzzy_search() {
        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem {
                command_id: "workbench.action.reloadWindow".to_string(),
                label: "Reload Window".to_string(),
                category: Some("Developer".to_string()),
                keybinding: None,
            },
            PaletteItem {
                command_id: "editor.action.formatDocument".to_string(),
                label: "Format Document".to_string(),
                category: None,
                keybinding: Some("Shift+Alt+F".to_string()),
            },
            PaletteItem {
                command_id: "git.commit".to_string(),
                label: "Commit".to_string(),
                category: Some("Git".to_string()),
                keybinding: None,
            },
        ]);

        assert_eq!(palette.results().len(), 3);

        palette.set_query("format");
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.selected_item().unwrap().command_id, "editor.action.formatDocument");

        palette.set_query("dev rel");
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.selected_item().unwrap().command_id, "workbench.action.reloadWindow");
    }
}
