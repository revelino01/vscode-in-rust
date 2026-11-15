//! Editor group and tab model mirroring VS Code's editor grid.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorTab {
    pub id: String,
    pub uri: String,
    pub title: String,
    pub is_dirty: bool,
    pub is_preview: bool,
}

impl EditorTab {
    pub fn new(uri: &str, title: &str, is_preview: bool) -> Self {
        Self {
            id: uri.to_string(),
            uri: uri.to_string(),
            title: title.to_string(),
            is_dirty: false,
            is_preview,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EditorGroup {
    pub tabs: Vec<EditorTab>,
    pub active_tab_index: Option<usize>,
}

impl EditorGroup {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab_index: None,
        }
    }

    /// Open or activate a tab. Replaces current preview tab if not dirty.
    pub fn open_tab(&mut self, uri: &str, title: &str, is_preview: bool) -> usize {
        // Check if already open
        if let Some(idx) = self.tabs.iter().position(|t| t.uri == uri) {
            self.active_tab_index = Some(idx);
            if !is_preview && self.tabs[idx].is_preview {
                self.tabs[idx].is_preview = false;
            }
            return idx;
        }

        // If current active tab is preview and not dirty, replace it
        if let Some(idx) = self.active_tab_index {
            if self.tabs[idx].is_preview && !self.tabs[idx].is_dirty {
                self.tabs[idx] = EditorTab::new(uri, title, is_preview);
                return idx;
            }
        }

        let new_tab = EditorTab::new(uri, title, is_preview);
        self.tabs.push(new_tab);
        let new_idx = self.tabs.len() - 1;
        self.active_tab_index = Some(new_idx);
        new_idx
    }

    /// Close tab by index. Returns false if index was out of bounds.
    pub fn close_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }

        self.tabs.remove(index);

        if self.tabs.is_empty() {
            self.active_tab_index = None;
        } else if let Some(active) = self.active_tab_index {
            if active >= self.tabs.len() {
                self.active_tab_index = Some(self.tabs.len() - 1);
            } else if active > index {
                self.active_tab_index = Some(active - 1);
            }
        }
        true
    }

    pub fn set_dirty(&mut self, index: usize, dirty: bool) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.is_dirty = dirty;
            if dirty {
                tab.is_preview = false;
            }
        }
    }

    pub fn active_tab(&self) -> Option<&EditorTab> {
        self.active_tab_index.and_then(|idx| self.tabs.get(idx))
    }
}

pub struct EditorArea {
    pub groups: Vec<EditorGroup>,
    pub active_group: usize,
}

impl Default for EditorArea {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorArea {
    pub fn new() -> Self {
        Self {
            groups: vec![EditorGroup::new()],
            active_group: 0,
        }
    }

    pub fn current_group(&mut self) -> &mut EditorGroup {
        &mut self.groups[self.active_group]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_editor_tabs_and_groups() {
        let mut group = EditorGroup::new();
        assert_eq!(group.tabs.len(), 0);

        // Open preview tab 1
        group.open_tab("file:///a.rs", "a.rs", true);
        assert_eq!(group.tabs.len(), 1);
        assert!(group.active_tab().unwrap().is_preview);

        // Open preview tab 2 replaces preview tab 1
        group.open_tab("file:///b.rs", "b.rs", true);
        assert_eq!(group.tabs.len(), 1);
        assert_eq!(group.active_tab().unwrap().title, "b.rs");

        // Make it dirty -> becomes permanent
        group.set_dirty(0, true);
        assert!(!group.active_tab().unwrap().is_preview);

        // Now opening c.rs adds a new tab instead of replacing
        group.open_tab("file:///c.rs", "c.rs", true);
        assert_eq!(group.tabs.len(), 2);

        // Close tab 0
        group.close_tab(0);
        assert_eq!(group.tabs.len(), 1);
        assert_eq!(group.active_tab().unwrap().title, "c.rs");
    }
}
