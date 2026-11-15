//! Activity Bar icon strip and view containers.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityBarItem {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub view_container_id: String,
    pub badge: Option<u32>,
}

pub struct ActivityBar {
    pub items: Vec<ActivityBarItem>,
    pub active_item_index: usize,
}

impl Default for ActivityBar {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivityBar {
    pub fn new() -> Self {
        Self {
            items: vec![
                ActivityBarItem {
                    id: "workbench.view.explorer".to_string(),
                    title: "Explorer".to_string(),
                    icon: "files".to_string(),
                    view_container_id: "explorer".to_string(),
                    badge: None,
                },
                ActivityBarItem {
                    id: "workbench.view.search".to_string(),
                    title: "Search".to_string(),
                    icon: "search".to_string(),
                    view_container_id: "search".to_string(),
                    badge: None,
                },
                ActivityBarItem {
                    id: "workbench.view.scm".to_string(),
                    title: "Source Control".to_string(),
                    icon: "source-control".to_string(),
                    view_container_id: "scm".to_string(),
                    badge: None,
                },
                ActivityBarItem {
                    id: "workbench.view.extensions".to_string(),
                    title: "Extensions".to_string(),
                    icon: "extensions".to_string(),
                    view_container_id: "extensions".to_string(),
                    badge: None,
                },
            ],
            active_item_index: 0,
        }
    }
}
