//! Workbench layout models matching VS Code's shell regions.

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

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

#[derive(Debug, Clone)]
pub struct LayoutHandle(Arc<RwLock<(u64, Arc<WorkbenchLayout>)>>);

impl Default for LayoutHandle {
    fn default() -> Self {
        Self(Arc::new(RwLock::new((0, Arc::new(WorkbenchLayout::new())))))
    }
}

impl LayoutHandle {
    pub fn update<R>(&self, f: impl FnOnce(&mut WorkbenchLayout) -> R) -> R {
        let mut guard = self.0.write();
        let mut next = (*guard.1).clone();
        let res = f(&mut next);
        guard.0 = guard.0.wrapping_add(1);
        guard.1 = Arc::new(next);
        res
    }

    pub fn snapshot(&self) -> WorkbenchLayout {
        (*self.0.read().1).clone()
    }
}

/// Shared layout container supporting direct field access (`Deref<Target = WorkbenchLayout>`)
/// while allowing external command handlers to mutate layout immediately via [`LayoutHandle`].
#[derive(Debug)]
pub struct SharedWorkbenchLayout {
    shared: Arc<RwLock<(u64, Arc<WorkbenchLayout>)>>,
    local: WorkbenchLayout,
    synced_rev: u64,
    pin_pool: Mutex<Vec<Arc<WorkbenchLayout>>>,
}

impl Default for SharedWorkbenchLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SharedWorkbenchLayout {
    fn clone(&self) -> Self {
        Self::from_layout((**self).clone())
    }
}

impl PartialEq for SharedWorkbenchLayout {
    fn eq(&self, other: &Self) -> bool {
        (**self) == (**other)
    }
}

impl From<WorkbenchLayout> for SharedWorkbenchLayout {
    fn from(layout: WorkbenchLayout) -> Self {
        Self::from_layout(layout)
    }
}

impl SharedWorkbenchLayout {
    pub fn new() -> Self {
        Self::from_layout(WorkbenchLayout::new())
    }

    pub fn from_layout(layout: WorkbenchLayout) -> Self {
        let arc = Arc::new(layout.clone());
        Self {
            shared: Arc::new(RwLock::new((0, arc))),
            local: layout,
            synced_rev: 0,
            pin_pool: Mutex::new(Vec::new()),
        }
    }

    pub fn handle(&self) -> LayoutHandle {
        LayoutHandle(Arc::clone(&self.shared))
    }

    pub fn mutate<R>(&mut self, f: impl FnOnce(&mut WorkbenchLayout) -> R) -> R {
        self.pin_pool.get_mut().clear();
        let mut guard = self.shared.write();
        if guard.0 != self.synced_rev {
            self.local = (*guard.1).clone();
        }
        let res = f(&mut self.local);
        guard.0 = guard.0.wrapping_add(1);
        guard.1 = Arc::new(self.local.clone());
        self.synced_rev = guard.0;
        res
    }

    pub fn toggle_sidebar(&mut self) {
        self.mutate(|l| l.toggle_sidebar());
    }

    pub fn toggle_panel(&mut self) {
        self.mutate(|l| l.toggle_panel());
    }

    pub fn set_sidebar_view(&mut self, view_id: &str) {
        self.mutate(|l| l.set_sidebar_view(view_id));
    }

    pub fn set_panel_tab(&mut self, tab_id: &str) {
        self.mutate(|l| l.set_panel_tab(tab_id));
    }
}

impl Deref for SharedWorkbenchLayout {
    type Target = WorkbenchLayout;

    fn deref(&self) -> &Self::Target {
        let guard = self.shared.read();
        if guard.0 == self.synced_rev {
            return &self.local;
        }
        let snap = Arc::clone(&guard.1);
        drop(guard);
        let ptr = Arc::as_ptr(&snap);
        let mut pool = self.pin_pool.lock();
        if !pool.last().is_some_and(|last| Arc::ptr_eq(last, &snap)) {
            pool.push(snap);
        }
        // SAFETY: `snap` remains stored in `self.pin_pool` (and `Arc<WorkbenchLayout>` is
        // heap-allocated and never mutated in place) until `&mut self` is taken, at which
        // point no `&'a Self::Target` references borrowed from `&'a self` can exist.
        unsafe { &*ptr }
    }
}

impl DerefMut for SharedWorkbenchLayout {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.pin_pool.get_mut().clear();
        let guard = self.shared.read();
        if guard.0 != self.synced_rev {
            self.local = (*guard.1).clone();
            self.synced_rev = guard.0;
        }
        &mut self.local
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

    #[test]
    fn test_shared_workbench_layout_immediate_handle_visibility() {
        let mut shared = SharedWorkbenchLayout::new();
        let handle = shared.handle();
        assert!(shared.sidebar_visible);
        assert!(!shared.panel_visible);

        handle.update(|l| {
            l.toggle_sidebar();
            l.toggle_panel();
        });

        // Visible immediately via Deref without any explicit sync method
        assert!(!shared.sidebar_visible);
        assert!(shared.panel_visible);

        handle.update(|l| {
            l.set_sidebar_view("workbench.view.testing");
        });
        assert!(shared.sidebar_visible);
        assert_eq!(shared.active_sidebar_view, "workbench.view.testing");

        // Local mutations via mutate() are also visible to handle
        shared.mutate(|l| l.sidebar_visible = true);
        assert!(handle.snapshot().sidebar_visible);
    }
}
