//! Workbench layout models matching VS Code's shell regions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkbenchLayout {
    pub sidebar_visible: bool,
    pub panel_visible: bool,
    pub status_bar_visible: bool,
    pub activity_bar_visible: bool,
    pub sidebar_width: f32,
    pub panel_height: f32,
    pub active_sidebar_view: String,
    pub active_panel_tab: String,
}

impl Default for WorkbenchLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkbenchLayout {
    pub fn new() -> Self {
        Self {
            sidebar_visible: true,
            panel_visible: false,
            status_bar_visible: true,
            activity_bar_visible: true,
            sidebar_width: 280.0,
            panel_height: 220.0,
            active_sidebar_view: "workbench.view.explorer".to_string(),
            active_panel_tab: "terminal".to_string(),
        }
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
    }

    pub fn toggle_panel(&mut self) {
        self.panel_visible = !self.panel_visible;
    }

    pub fn set_sidebar_view(&mut self, view_id: &str) {
        if self.active_sidebar_view == view_id && self.sidebar_visible {
            self.sidebar_visible = false;
        } else {
            self.active_sidebar_view = view_id.to_string();
            self.sidebar_visible = true;
        }
    }

    pub fn set_panel_tab(&mut self, tab_id: &str) {
        self.active_panel_tab = tab_id.to_string();
        self.panel_visible = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workbench_layout_toggles() {
        let mut layout = WorkbenchLayout::new();
        assert!(layout.sidebar_visible);

        layout.toggle_sidebar();
        assert!(!layout.sidebar_visible);

        // Clicking active view icon toggles sidebar visibility
        layout.set_sidebar_view("workbench.view.explorer");
        assert!(layout.sidebar_visible);
        layout.set_sidebar_view("workbench.view.explorer");
        assert!(!layout.sidebar_visible);
    }
}
