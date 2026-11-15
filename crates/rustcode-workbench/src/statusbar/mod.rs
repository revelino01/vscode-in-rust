//! Status bar item management and sorting.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatusBarAlignment {
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusBarItem {
    pub id: String,
    pub text: String,
    pub tooltip: Option<String>,
    pub command: Option<String>,
    pub alignment: StatusBarAlignment,
    pub priority: i32,
}

#[derive(Default)]
pub struct StatusBar {
    items: Vec<StatusBarItem>,
}

impl StatusBar {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn set_item(&mut self, item: StatusBarItem) {
        if let Some(existing) = self.items.iter_mut().find(|i| i.id == item.id) {
            *existing = item;
        } else {
            self.items.push(item);
        }
    }

    pub fn remove_item(&mut self, id: &str) {
        self.items.retain(|i| i.id != id);
    }

    /// Returns left-aligned items sorted by priority (higher priority first).
    pub fn left_items(&self) -> Vec<&StatusBarItem> {
        let mut left: Vec<_> = self.items.iter().filter(|i| i.alignment == StatusBarAlignment::Left).collect();
        left.sort_by(|a, b| b.priority.cmp(&a.priority));
        left
    }

    /// Returns right-aligned items sorted by priority (higher priority first).
    pub fn right_items(&self) -> Vec<&StatusBarItem> {
        let mut right: Vec<_> = self.items.iter().filter(|i| i.alignment == StatusBarAlignment::Right).collect();
        right.sort_by(|a, b| b.priority.cmp(&a.priority));
        right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_bar_priority_ordering() {
        let mut bar = StatusBar::new();

        bar.set_item(StatusBarItem {
            id: "git".to_string(),
            text: "main*".to_string(),
            tooltip: None,
            command: None,
            alignment: StatusBarAlignment::Left,
            priority: 100,
        });

        bar.set_item(StatusBarItem {
            id: "errors".to_string(),
            text: "0 errors".to_string(),
            tooltip: None,
            command: None,
            alignment: StatusBarAlignment::Left,
            priority: 200,
        });

        let left = bar.left_items();
        assert_eq!(left[0].id, "errors"); // Higher priority comes first
        assert_eq!(left[1].id, "git");
    }
}
