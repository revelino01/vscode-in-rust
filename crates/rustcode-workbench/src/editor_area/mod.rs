//! Editor group and tab model mirroring VS Code's editor grid, and GPUI EditorAreaView component.

use parking_lot::RwLock;
use rustcode_editor::{
    DiffEditorView, EditorView, GrammarRegistry, LanguageConfigurationRegistry, Position, Range,
    RangeEdit, SnippetRegistry, TextDocument, WordWrapMode,
};
use rustcode_ext_host::manifest::CustomEditorContribution;
use rustcode_ext_host::webview::{ExtHostWebviewManager, WebviewOptions};
use rustcode_platform::fs::{FileMetadata, FileSystemError, FileSystemService};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, MouseButton, Point, Quad, Rect, Rgba,
    Scene, View, WorkbenchThemeStyles,
};
use rustcode_platform::theme::ThemeService;
use rustcode_protocols::lsp::{DocumentChange, WorkspaceEdit};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use crate::breadcrumbs::{BreadcrumbItem, BreadcrumbKind, BreadcrumbsAction, BreadcrumbsBar};
use crate::notebook::NotebookEditorView;
use crate::refactor_preview::{FileUndoSnapshot, WorkspaceEditUndoEntry};
use crate::settings_ui::SettingsUiView;
use crate::webview::WebviewEditorView;
use crate::workspace::Workspace;
use rustcode_platform::configuration::ConfigurationService;


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
    pub id: usize,
    pub tabs: Vec<EditorTab>,
    pub active_tab_index: Option<usize>,
}

impl EditorGroup {
    pub fn new() -> Self {
        Self::with_id(0)
    }

    pub fn with_id(id: usize) -> Self {
        Self {
            id,
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
            if let Some(tab) = self.tabs.get_mut(idx) {
                if tab.is_preview && !tab.is_dirty {
                    *tab = EditorTab::new(uri, title, is_preview);
                    return idx;
                }
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

pub const SASH_THICKNESS: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EditorGridNode {
    Leaf(usize),
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<EditorGridNode>,
        second: Box<EditorGridNode>,
    },
}

impl Default for EditorGridNode {
    fn default() -> Self {
        Self::Leaf(0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditorSash {
    pub split_path: Vec<bool>,
    pub direction: SplitDirection,
    pub rect: Rect,
    pub parent_bounds: Rect,
}

pub struct EditorArea {
    pub groups: Vec<EditorGroup>,
    pub active_group: usize,
    pub grid: EditorGridNode,
    next_group_id: usize,
}

impl Default for EditorArea {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorArea {
    pub fn new() -> Self {
        Self {
            groups: vec![EditorGroup::with_id(0)],
            active_group: 0,
            grid: EditorGridNode::Leaf(0),
            next_group_id: 1,
        }
    }

    pub fn current_group(&mut self) -> &mut EditorGroup {
        &mut self.groups[self.active_group]
    }

    pub fn current_group_ref(&self) -> &EditorGroup {
        &self.groups[self.active_group]
    }

    pub fn active_group_id(&self) -> usize {
        self.groups.get(self.active_group).map_or(0, |g| g.id)
    }

    pub fn group_index_by_id(&self, group_id: usize) -> Option<usize> {
        self.groups.iter().position(|g| g.id == group_id)
    }

    pub fn group_by_id(&self, group_id: usize) -> Option<&EditorGroup> {
        self.groups.iter().find(|g| g.id == group_id)
    }

    pub fn group_by_id_mut(&mut self, group_id: usize) -> Option<&mut EditorGroup> {
        self.groups.iter_mut().find(|g| g.id == group_id)
    }

    pub fn split_active_group(&mut self, direction: SplitDirection) -> usize {
        let active_id = self.active_group_id();
        let new_id = self.next_group_id;
        self.next_group_id += 1;

        let mut new_group = EditorGroup::with_id(new_id);
        if let Some(mut cloned_tab) = self.current_group_ref().active_tab().cloned() {
            cloned_tab.is_preview = false;
            new_group.tabs.push(cloned_tab);
            new_group.active_tab_index = Some(0);
        }

        Self::split_leaf(&mut self.grid, active_id, new_id, direction);
        self.groups.push(new_group);
        self.active_group = self.groups.len() - 1;
        new_id
    }

    pub fn close_group(&mut self, group_id: usize) -> bool {
        if self.groups.len() <= 1 || self.group_index_by_id(group_id).is_none() {
            return false;
        }
        let (next_grid, surviving_id) = Self::remove_leaf(&self.grid, group_id);
        self.grid = next_grid;
        self.groups.retain(|g| g.id != group_id);

        if let Some(target_id) = surviving_id {
            if let Some(idx) = self.group_index_by_id(target_id) {
                self.active_group = idx;
                return true;
            }
        }
        self.active_group = self.active_group.min(self.groups.len().saturating_sub(1));
        true
    }

    pub fn reorder_tab_in_group(&mut self, group_id: usize, from_idx: usize, to_idx: usize) -> bool {
        let Some(group) = self.group_by_id_mut(group_id) else {
            return false;
        };
        if from_idx >= group.tabs.len() {
            return false;
        }
        let target = to_idx.min(group.tabs.len() - 1);
        if from_idx == target {
            group.active_tab_index = Some(from_idx);
            return false;
        }
        let tab = group.tabs.remove(from_idx);
        group.tabs.insert(target, tab);
        group.active_tab_index = Some(target);
        true
    }

    pub fn split_group_empty(
        &mut self,
        target_group_id: usize,
        direction: SplitDirection,
        insert_before: bool,
    ) -> Option<usize> {
        self.group_by_id(target_group_id)?;
        let new_id = self.next_group_id;
        self.next_group_id += 1;
        let new_group = EditorGroup::with_id(new_id);
        if !Self::split_leaf_with_order(
            &mut self.grid,
            target_group_id,
            new_id,
            direction,
            insert_before,
        ) {
            return None;
        }
        self.groups.push(new_group);
        Some(new_id)
    }

    pub fn focus_group(&mut self, group_id: usize) -> bool {
        if let Some(idx) = self.group_index_by_id(group_id) {
            self.active_group = idx;
            true
        } else {
            false
        }
    }

    pub fn focus_group_index(&mut self, idx: usize) -> bool {
        if idx < self.groups.len() {
            self.active_group = idx;
            true
        } else {
            false
        }
    }

    pub fn focus_next_group(&mut self) {
        if !self.groups.is_empty() {
            self.active_group = (self.active_group + 1) % self.groups.len();
        }
    }

    pub fn focus_previous_group(&mut self) {
        if !self.groups.is_empty() {
            self.active_group = if self.active_group == 0 {
                self.groups.len() - 1
            } else {
                self.active_group - 1
            };
        }
    }

    pub fn focus_first_editor_group(&mut self) -> bool {
        self.focus_group_index(0)
    }

    pub fn focus_second_editor_group(&mut self) -> bool {
        self.focus_group_index(1)
    }

    pub fn compute_group_rects(&self, bounds: Rect) -> Vec<(usize, Rect)> {
        if !bounds.is_finite_positive() {
            return Vec::new();
        }
        let mut rects = Vec::new();
        Self::collect_group_rects(&self.grid, bounds, &mut rects);
        rects
    }

    pub fn compute_sashes(&self, bounds: Rect) -> Vec<EditorSash> {
        if !bounds.is_finite_positive() {
            return Vec::new();
        }
        let mut sashes = Vec::new();
        let mut path = Vec::new();
        Self::collect_sashes(&self.grid, bounds, &mut path, &mut sashes);
        sashes
    }

    pub fn update_sash_ratio(
        &mut self,
        split_path: &[bool],
        pointer: Point,
        parent_bounds: Rect,
    ) -> bool {
        if !pointer.x.is_finite()
            || !pointer.y.is_finite()
            || !parent_bounds.is_finite_positive()
        {
            return false;
        }
        Self::set_sash_ratio(&mut self.grid, split_path, pointer, parent_bounds)
    }

    fn split_leaf(
        node: &mut EditorGridNode,
        target_id: usize,
        new_id: usize,
        direction: SplitDirection,
    ) -> bool {
        Self::split_leaf_with_order(node, target_id, new_id, direction, false)
    }

    fn split_leaf_with_order(
        node: &mut EditorGridNode,
        target_id: usize,
        new_id: usize,
        direction: SplitDirection,
        insert_before: bool,
    ) -> bool {
        match node {
            EditorGridNode::Leaf(id) if *id == target_id => {
                let (first_id, second_id) = if insert_before {
                    (new_id, target_id)
                } else {
                    (target_id, new_id)
                };
                *node = EditorGridNode::Split {
                    direction,
                    ratio: 0.5,
                    first: Box::new(EditorGridNode::Leaf(first_id)),
                    second: Box::new(EditorGridNode::Leaf(second_id)),
                };
                true
            }
            EditorGridNode::Leaf(_) => false,
            EditorGridNode::Split { first, second, .. } => {
                Self::split_leaf_with_order(first, target_id, new_id, direction, insert_before)
                    || Self::split_leaf_with_order(second, target_id, new_id, direction, insert_before)
            }
        }
    }

    fn first_leaf_id(node: &EditorGridNode) -> usize {
        match node {
            EditorGridNode::Leaf(id) => *id,
            EditorGridNode::Split { first, .. } => Self::first_leaf_id(first),
        }
    }

    fn contains_leaf(node: &EditorGridNode, target_id: usize) -> bool {
        match node {
            EditorGridNode::Leaf(id) => *id == target_id,
            EditorGridNode::Split { first, second, .. } => {
                Self::contains_leaf(first, target_id) || Self::contains_leaf(second, target_id)
            }
        }
    }

    fn remove_leaf(node: &EditorGridNode, target_id: usize) -> (EditorGridNode, Option<usize>) {
        match node {
            EditorGridNode::Leaf(id) => (EditorGridNode::Leaf(*id), None),
            EditorGridNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if matches!(first.as_ref(), EditorGridNode::Leaf(id) if *id == target_id) {
                    let surviving = second.as_ref().clone();
                    let next_focus = Some(Self::first_leaf_id(&surviving));
                    return (surviving, next_focus);
                }
                if matches!(second.as_ref(), EditorGridNode::Leaf(id) if *id == target_id) {
                    let surviving = first.as_ref().clone();
                    let next_focus = Some(Self::first_leaf_id(&surviving));
                    return (surviving, next_focus);
                }
                if Self::contains_leaf(first, target_id) {
                    let (new_first, focus) = Self::remove_leaf(first, target_id);
                    (
                        EditorGridNode::Split {
                            direction: *direction,
                            ratio: *ratio,
                            first: Box::new(new_first),
                            second: second.clone(),
                        },
                        focus,
                    )
                } else if Self::contains_leaf(second, target_id) {
                    let (new_second, focus) = Self::remove_leaf(second, target_id);
                    (
                        EditorGridNode::Split {
                            direction: *direction,
                            ratio: *ratio,
                            first: first.clone(),
                            second: Box::new(new_second),
                        },
                        focus,
                    )
                } else {
                    (node.clone(), None)
                }
            }
        }
    }

    fn split_child_rects(
        direction: SplitDirection,
        ratio: f32,
        bounds: Rect,
    ) -> (Rect, Rect, Rect) {
        let r = if ratio.is_finite() {
            ratio.clamp(0.15, 0.85)
        } else {
            0.5
        };
        match direction {
            SplitDirection::Horizontal => {
                let sash = SASH_THICKNESS.min(bounds.width());
                let avail_w = (bounds.width() - sash).max(0.0);
                let first_w = avail_w * r;
                let second_w = (avail_w - first_w).max(0.0);
                let first_rect =
                    Rect::from_xywh(bounds.min_x(), bounds.min_y(), first_w, bounds.height());
                let sash_rect = Rect::from_xywh(
                    bounds.min_x() + first_w,
                    bounds.min_y(),
                    sash,
                    bounds.height(),
                );
                let second_rect = Rect::from_xywh(
                    bounds.min_x() + first_w + sash,
                    bounds.min_y(),
                    second_w,
                    bounds.height(),
                );
                (first_rect, sash_rect, second_rect)
            }
            SplitDirection::Vertical => {
                let sash = SASH_THICKNESS.min(bounds.height());
                let avail_h = (bounds.height() - sash).max(0.0);
                let first_h = avail_h * r;
                let second_h = (avail_h - first_h).max(0.0);
                let first_rect =
                    Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), first_h);
                let sash_rect = Rect::from_xywh(
                    bounds.min_x(),
                    bounds.min_y() + first_h,
                    bounds.width(),
                    sash,
                );
                let second_rect = Rect::from_xywh(
                    bounds.min_x(),
                    bounds.min_y() + first_h + sash,
                    bounds.width(),
                    second_h,
                );
                (first_rect, sash_rect, second_rect)
            }
        }
    }

    fn collect_group_rects(
        node: &EditorGridNode,
        bounds: Rect,
        out: &mut Vec<(usize, Rect)>,
    ) {
        match node {
            EditorGridNode::Leaf(id) => out.push((*id, bounds)),
            EditorGridNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_rect, _, second_rect) =
                    Self::split_child_rects(*direction, *ratio, bounds);
                Self::collect_group_rects(first, first_rect, out);
                Self::collect_group_rects(second, second_rect, out);
            }
        }
    }

    fn collect_sashes(
        node: &EditorGridNode,
        bounds: Rect,
        path: &mut Vec<bool>,
        out: &mut Vec<EditorSash>,
    ) {
        if let EditorGridNode::Split {
            direction,
            ratio,
            first,
            second,
        } = node
        {
            let (first_rect, sash_rect, second_rect) =
                Self::split_child_rects(*direction, *ratio, bounds);
            out.push(EditorSash {
                split_path: path.clone(),
                direction: *direction,
                rect: sash_rect,
                parent_bounds: bounds,
            });
            path.push(false);
            Self::collect_sashes(first, first_rect, path, out);
            path.pop();
            path.push(true);
            Self::collect_sashes(second, second_rect, path, out);
            path.pop();
        }
    }

    fn set_sash_ratio(
        node: &mut EditorGridNode,
        split_path: &[bool],
        pointer: Point,
        parent_bounds: Rect,
    ) -> bool {
        let EditorGridNode::Split {
            direction,
            ratio,
            first,
            second,
        } = node
        else {
            return false;
        };

        if split_path.is_empty() {
            let raw = match direction {
                SplitDirection::Horizontal => {
                    let sash = SASH_THICKNESS.min(parent_bounds.width());
                    let avail_w = parent_bounds.width() - sash;
                    if avail_w > 0.0 {
                        (pointer.x - parent_bounds.min_x() - sash * 0.5) / avail_w
                    } else {
                        (pointer.x - parent_bounds.min_x()) / parent_bounds.width()
                    }
                }
                SplitDirection::Vertical => {
                    let sash = SASH_THICKNESS.min(parent_bounds.height());
                    let avail_h = parent_bounds.height() - sash;
                    if avail_h > 0.0 {
                        (pointer.y - parent_bounds.min_y() - sash * 0.5) / avail_h
                    } else {
                        (pointer.y - parent_bounds.min_y()) / parent_bounds.height()
                    }
                }
            };
            if raw.is_finite() {
                *ratio = raw.clamp(0.15, 0.85);
                return true;
            }
            return false;
        }

        if split_path[0] {
            Self::set_sash_ratio(second, &split_path[1..], pointer, parent_bounds)
        } else {
            Self::set_sash_ratio(first, &split_path[1..], pointer, parent_bounds)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAreaAction {
    TabSelected(usize),
    TabClosed(usize),
    BreadcrumbSelected(BreadcrumbItem),
    GroupFocused(usize),
    GroupSplit {
        group_id: usize,
        direction: SplitDirection,
    },
    GroupClosed(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabDragState {
    pub source_group_id: usize,
    pub source_tab_index: usize,
    pub uri: String,
    pub start_pos: Point,
    pub current_pos: Point,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabDropZone {
    ReorderTab {
        group_id: usize,
        target_index: usize,
    },
    MoveToGroup(usize),
    SplitGroup {
        group_id: usize,
        direction: SplitDirection,
        insert_before: bool,
    },
}

pub struct EditorAreaView {
    pub model: EditorArea,
    pub editors: HashMap<String, EditorView>,
    pub split_editors: HashMap<(usize, String), EditorView>,
    primary_editor_group: HashMap<String, usize>,
    pub diff_editors: HashMap<String, DiffEditorView>,
    pub webviews: HashMap<String, WebviewEditorView>,
    pub split_webviews: HashMap<(usize, String), WebviewEditorView>,
    primary_webview_group: HashMap<String, usize>,
    pub closed_webview_panels: Vec<String>,
    pub settings_views: HashMap<String, SettingsUiView>,
    pub notebooks: HashMap<String, NotebookEditorView>,
    pub grammar_registry: Arc<GrammarRegistry>,
    pub language_configs: LanguageConfigurationRegistry,
    pub snippet_registry: SnippetRegistry,
    pub last_action: Option<EditorAreaAction>,
    pub breadcrumbs_enabled: bool,
    pub minimap_enabled: bool,
    pub word_wrap_mode: WordWrapMode,
    pub word_wrap_column: usize,
    pub breadcrumbs: BreadcrumbsBar,
    pub group_breadcrumbs: HashMap<usize, BreadcrumbsBar>,
    pub custom_editor_panels: HashMap<String, String>,
    pub custom_editor_contributions: Vec<CustomEditorContribution>,
    pub active_sash_drag: Option<(Vec<bool>, Rect)>,
    pub active_tab_drag: Option<TabDragState>,
    known_workspace: Option<Workspace>,
}

impl Default for EditorAreaView {
    fn default() -> Self {
        Self::new(EditorArea::new())
    }
}

impl EditorAreaView {
    pub fn new(model: EditorArea) -> Self {
        let mut group_breadcrumbs = HashMap::new();
        for g in &model.groups {
            group_breadcrumbs.insert(g.id, BreadcrumbsBar::new());
        }
        Self {
            model,
            editors: HashMap::new(),
            split_editors: HashMap::new(),
            primary_editor_group: HashMap::new(),
            diff_editors: HashMap::new(),
            webviews: HashMap::new(),
            split_webviews: HashMap::new(),
            primary_webview_group: HashMap::new(),
            closed_webview_panels: Vec::new(),
            settings_views: HashMap::new(),
            notebooks: HashMap::new(),
            grammar_registry: Arc::new(GrammarRegistry::new()),
            language_configs: LanguageConfigurationRegistry::new(),
            snippet_registry: SnippetRegistry::new(),
            last_action: None,
            breadcrumbs_enabled: true,
            minimap_enabled: true,
            word_wrap_mode: WordWrapMode::Off,
            word_wrap_column: 80,
            breadcrumbs: BreadcrumbsBar::new(),
            group_breadcrumbs,
            custom_editor_panels: HashMap::new(),
            custom_editor_contributions: Vec::new(),
            active_sash_drag: None,
            active_tab_drag: None,
            known_workspace: None,
        }
    }

    pub fn editor_for_group(&self, group_id: usize, uri: &str) -> Option<&EditorView> {
        if let Some(&primary_gid) = self.primary_editor_group.get(uri) {
            if primary_gid == group_id {
                self.editors.get(uri)
            } else {
                self.split_editors.get(&(group_id, uri.to_string()))
            }
        } else {
            self.split_editors
                .get(&(group_id, uri.to_string()))
                .or_else(|| self.editors.get(uri))
        }
    }

    pub fn editor_for_group_mut(&mut self, group_id: usize, uri: &str) -> Option<&mut EditorView> {
        if let Some(&primary_gid) = self.primary_editor_group.get(uri) {
            if primary_gid == group_id {
                self.editors.get_mut(uri)
            } else {
                self.split_editors.get_mut(&(group_id, uri.to_string()))
            }
        } else if self.split_editors.contains_key(&(group_id, uri.to_string())) {
            self.split_editors.get_mut(&(group_id, uri.to_string()))
        } else {
            self.editors.get_mut(uri)
        }
    }

    pub fn any_editor_for_uri(&self, uri: &str) -> Option<&EditorView> {
        self.editors.get(uri).or_else(|| {
            self.split_editors
                .iter()
                .find(|((_, u), _)| u == uri)
                .map(|(_, ed)| ed)
        })
    }

    pub fn find_open_tab_uri_for_path(&self, target_path: &std::path::Path) -> Option<String> {
        let active_gid = self.model.active_group_id();
        let matches_target = |uri: &str| -> bool {
            if rustcode_platform::is_file_uri(uri) || !uri.contains("://") {
                let open_path = rustcode_platform::file_uri_to_path(uri);
                rustcode_platform::paths_equivalent(&open_path, target_path)
            } else {
                false
            }
        };
        if let Some(active_group) = self.model.group_by_id(active_gid) {
            if let Some(tab) = active_group.tabs.iter().find(|t| matches_target(&t.uri)) {
                return Some(tab.uri.clone());
            }
        }
        for group in &self.model.groups {
            if let Some(tab) = group.tabs.iter().find(|t| matches_target(&t.uri)) {
                return Some(tab.uri.clone());
            }
        }
        None
    }

    pub fn rename_tab(
        &mut self,
        old_path: &std::path::Path,
        new_path: &std::path::Path,
    ) -> bool {
        let mut renamed_entries: Vec<(String, String, PathBuf)> = Vec::new();
        for group in &mut self.model.groups {
            for tab in &mut group.tabs {
                if !(rustcode_platform::is_file_uri(&tab.uri) || !tab.uri.contains("://")) {
                    continue;
                }
                let open_path = rustcode_platform::file_uri_to_path(&tab.uri);
                let updated_path = if rustcode_platform::paths_equivalent(&open_path, old_path) {
                    Some(new_path.to_path_buf())
                } else {
                    rustcode_platform::path_strip_prefix(&open_path, old_path)
                        .filter(|rel| !rel.as_os_str().is_empty())
                        .map(|rel| new_path.join(rel))
                };
                let Some(updated_path) = updated_path else {
                    continue;
                };
                let old_uri = tab.uri.clone();
                let new_uri = if rustcode_platform::is_file_uri(&old_uri) {
                    rustcode_platform::path_to_file_uri(&updated_path)
                } else {
                    updated_path.to_string_lossy().into_owned()
                };
                let new_title = updated_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("untitled")
                    .to_string();

                tab.id = new_uri.clone();
                tab.uri = new_uri.clone();
                tab.title = new_title;
                if !renamed_entries.iter().any(|(o, _, _)| o == &old_uri) {
                    renamed_entries.push((old_uri, new_uri, updated_path));
                }
            }
        }

        if renamed_entries.is_empty() {
            return false;
        }

        for (old_uri, new_uri, updated_path) in renamed_entries {
            let new_lang = self.grammar_registry.get_for_path(&updated_path).language_id();

            if let Some(mut editor) = self.editors.remove(&old_uri) {
                if let Some(doc) = &editor.document {
                    let mut d = doc.write();
                    d.path = Some(updated_path.clone());
                    d.language_id = new_lang.to_string();
                }
                let theme = ThemeService::default_dark_theme();
                editor.set_language(new_lang, theme, &self.grammar_registry);
                self.editors.insert(new_uri.clone(), editor);
            }

            let split_keys: Vec<(usize, String)> = self
                .split_editors
                .keys()
                .filter(|(_, u)| u == &old_uri)
                .cloned()
                .collect();
            for (gid, u) in split_keys {
                if let Some(mut editor) = self.split_editors.remove(&(gid, u)) {
                    if let Some(doc) = &editor.document {
                        let mut d = doc.write();
                        d.path = Some(updated_path.clone());
                        d.language_id = new_lang.to_string();
                    }
                    let theme = ThemeService::default_dark_theme();
                    editor.set_language(new_lang, theme, &self.grammar_registry);
                    self.split_editors.insert((gid, new_uri.clone()), editor);
                }
            }

            if let Some(gid) = self.primary_editor_group.remove(&old_uri) {
                self.primary_editor_group.insert(new_uri.clone(), gid);
            }
            if let Some(diff) = self.diff_editors.remove(&old_uri) {
                self.diff_editors.insert(new_uri.clone(), diff);
            }
            if let Some(wv) = self.webviews.remove(&old_uri) {
                self.webviews.insert(new_uri.clone(), wv);
            }
            let wv_split_keys: Vec<(usize, String)> = self
                .split_webviews
                .keys()
                .filter(|(_, u)| u == &old_uri)
                .cloned()
                .collect();
            for (gid, u) in wv_split_keys {
                if let Some(wv) = self.split_webviews.remove(&(gid, u)) {
                    self.split_webviews.insert((gid, new_uri.clone()), wv);
                }
            }
            if let Some(gid) = self.primary_webview_group.remove(&old_uri) {
                self.primary_webview_group.insert(new_uri.clone(), gid);
            }
            if let Some(mut nb) = self.notebooks.remove(&old_uri) {
                nb.document.uri = new_uri.clone();
                nb.document.path = Some(updated_path.clone());
                self.notebooks.insert(new_uri.clone(), nb);
            }
            for uri_val in self.custom_editor_panels.values_mut() {
                if *uri_val == old_uri {
                    *uri_val = new_uri.clone();
                }
            }
        }

        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
        true
    }

    pub fn close_tabs_for_path(&mut self, target_path: &std::path::Path) -> usize {
        let mut to_close: Vec<(usize, usize)> = Vec::new();
        for group in &self.model.groups {
            for (idx, tab) in group.tabs.iter().enumerate() {
                if !(rustcode_platform::is_file_uri(&tab.uri) || !tab.uri.contains("://")) {
                    continue;
                }
                let open_path = rustcode_platform::file_uri_to_path(&tab.uri);
                if rustcode_platform::paths_equivalent(&open_path, target_path)
                    || rustcode_platform::path_starts_with(&open_path, target_path)
                {
                    to_close.push((group.id, idx));
                }
            }
        }

        let mut closed_count = 0;
        for (gid, idx) in to_close.into_iter().rev() {
            if self.close_tab_in_group(gid, idx) {
                closed_count += 1;
            }
        }
        closed_count
    }

    pub fn for_each_editor_mut(&mut self, mut f: impl FnMut(&str, &mut EditorView)) {
        for (uri, ed) in &mut self.editors {
            f(uri, ed);
        }
        for ((_, uri), ed) in &mut self.split_editors {
            f(uri, ed);
        }
    }

    pub fn for_each_editor_for_uri_mut(&mut self, uri: &str, mut f: impl FnMut(&mut EditorView)) {
        if let Some(ed) = self.editors.get_mut(uri) {
            f(ed);
        }
        for ((_, u), ed) in &mut self.split_editors {
            if u == uri {
                f(ed);
            }
        }
    }

    pub fn is_minimap_or_sash_dragging(&self) -> bool {
        self.active_sash_drag.is_some()
            || self.active_tab_drag.is_some()
            || self.editors.values().any(|e| e.minimap_dragging)
            || self.split_editors.values().any(|e| e.minimap_dragging)
    }

    pub fn clear_all_minimap_dragging(&mut self) {
        self.active_sash_drag = None;
        self.for_each_editor_mut(|_, ed| {
            ed.minimap_dragging = false;
        });
    }

    pub fn clear_all_drag_state(&mut self) {
        self.active_tab_drag = None;
        self.clear_all_minimap_dragging();
    }

    pub fn group_id_at_point(&self, bounds: Rect, pos: Point) -> Option<usize> {
        self.model
            .compute_group_rects(bounds)
            .into_iter()
            .find(|(_, rect)| rect.contains(pos))
            .map(|(gid, _)| gid)
    }

    pub fn tab_rects_for_group(
        &self,
        group_id: usize,
        group_rect: Rect,
        font: &FontMetrics,
    ) -> Vec<(usize, Rect, f32)> {
        let tab_bar_h = 35.0;
        let Some(group) = self.model.group_by_id(group_id) else {
            return Vec::new();
        };
        let mut cur_x = group_rect.min_x();
        let mut list = Vec::with_capacity(group.tabs.len());
        for (idx, tab) in group.tabs.iter().enumerate() {
            let tab_w = (font.text_width(&tab.title) + 40.0).clamp(80.0, 220.0);
            let tab_rect = Rect::from_xywh(cur_x, group_rect.min_y(), tab_w, tab_bar_h);
            list.push((idx, tab_rect, cur_x + tab_w - 22.0));
            cur_x += tab_w;
        }
        list
    }

    pub fn tab_bar_contains(&self, bounds: Rect, pos: Point) -> bool {
        self.model
            .compute_group_rects(bounds)
            .into_iter()
            .any(|(_, rect)| rect.contains(pos) && pos.y < rect.min_y() + 35.0)
    }

    pub fn move_tab_to_group(
        &mut self,
        from_group: usize,
        from_index: usize,
        to_group: usize,
    ) -> bool {
        if from_group == to_group {
            return false;
        }
        if self.model.group_by_id(to_group).is_none() {
            return false;
        }
        let Some(tab) = (if let Some(src) = self.model.group_by_id_mut(from_group) {
            if from_index < src.tabs.len() {
                let removed = src.tabs.remove(from_index);
                if src.tabs.is_empty() {
                    src.active_tab_index = None;
                } else if let Some(active) = src.active_tab_index {
                    if active >= src.tabs.len() {
                        src.active_tab_index = Some(src.tabs.len() - 1);
                    } else if active > from_index {
                        src.active_tab_index = Some(active - 1);
                    }
                }
                Some(removed)
            } else {
                None
            }
        } else {
            None
        }) else {
            return false;
        };

        let target_already_has_uri = self
            .model
            .group_by_id(to_group)
            .is_some_and(|g| g.tabs.iter().any(|t| t.uri == tab.uri));

        if !target_already_has_uri {
            if self.primary_editor_group.get(&tab.uri) == Some(&from_group) {
                self.primary_editor_group.insert(tab.uri.clone(), to_group);
            } else if let Some(ed) = self.split_editors.remove(&(from_group, tab.uri.clone())) {
                self.split_editors.entry((to_group, tab.uri.clone())).or_insert(ed);
            }
            if self.primary_webview_group.get(&tab.uri) == Some(&from_group) {
                self.primary_webview_group.insert(tab.uri.clone(), to_group);
            } else if let Some(wv) = self.split_webviews.remove(&(from_group, tab.uri.clone())) {
                self.split_webviews.entry((to_group, tab.uri.clone())).or_insert(wv);
            }
        }

        if let Some(dst) = self.model.group_by_id_mut(to_group) {
            if let Some(existing_idx) = dst.tabs.iter().position(|t| t.uri == tab.uri) {
                dst.tabs[existing_idx].is_preview = false;
                dst.active_tab_index = Some(existing_idx);
            } else {
                dst.tabs.push(tab);
                dst.active_tab_index = Some(dst.tabs.len() - 1);
            }
        }

        let src_empty = self
            .model
            .group_by_id(from_group)
            .is_some_and(|g| g.tabs.is_empty());
        if src_empty && self.model.groups.len() > 1 && self.model.close_group(from_group) {
            self.active_sash_drag = None;
            self.last_action = Some(EditorAreaAction::GroupClosed(from_group));
        }

        self.focus_group(to_group);
        self.retain_open_resources();
        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
        true
    }

    pub fn resolve_tab_drop_target(&self, bounds: Rect, pos: Point) -> Option<TabDropZone> {
        let font = FontMetrics::default_monospace();
        let src_gid = self.active_tab_drag.as_ref().map(|d| d.source_group_id);
        for (gid, rect) in self.model.compute_group_rects(bounds) {
            if !rect.contains(pos) {
                continue;
            }
            if pos.y < rect.min_y() + 35.0 {
                let tabs_len = self.model.group_by_id(gid).map_or(0, |g| g.tabs.len());
                if tabs_len == 0 {
                    return Some(TabDropZone::MoveToGroup(gid));
                }
                let tab_rects = self.tab_rects_for_group(gid, rect, &font);
                for (idx, tr, _) in &tab_rects {
                    if pos.x < tr.min_x() + tr.width() * 0.5 || tr.contains(pos) {
                        return Some(TabDropZone::ReorderTab {
                            group_id: gid,
                            target_index: *idx,
                        });
                    }
                }
                if src_gid == Some(gid) {
                    return Some(TabDropZone::ReorderTab {
                        group_id: gid,
                        target_index: tabs_len.saturating_sub(1),
                    });
                }
                return Some(TabDropZone::MoveToGroup(gid));
            }
            let rel_x = (pos.x - rect.min_x()) / rect.width().max(1.0);
            let rel_y = (pos.y - rect.min_y()) / rect.height().max(1.0);
            let can_split = src_gid != Some(gid)
                || self.model.group_by_id(gid).is_some_and(|g| g.tabs.len() > 1);
            if can_split {
                if rel_x < 0.25 {
                    return Some(TabDropZone::SplitGroup {
                        group_id: gid,
                        direction: SplitDirection::Horizontal,
                        insert_before: true,
                    });
                } else if rel_x > 0.75 {
                    return Some(TabDropZone::SplitGroup {
                        group_id: gid,
                        direction: SplitDirection::Horizontal,
                        insert_before: false,
                    });
                } else if rel_y < 0.25 {
                    return Some(TabDropZone::SplitGroup {
                        group_id: gid,
                        direction: SplitDirection::Vertical,
                        insert_before: true,
                    });
                } else if rel_y > 0.75 {
                    return Some(TabDropZone::SplitGroup {
                        group_id: gid,
                        direction: SplitDirection::Vertical,
                        insert_before: false,
                    });
                }
            }
            return Some(TabDropZone::MoveToGroup(gid));
        }
        None
    }

    pub fn webview_for_group(&self, group_id: usize, uri: &str) -> Option<&WebviewEditorView> {
        if let Some(&primary_gid) = self.primary_webview_group.get(uri) {
            if primary_gid == group_id {
                self.webviews.get(uri)
            } else {
                self.split_webviews.get(&(group_id, uri.to_string()))
            }
        } else {
            self.split_webviews
                .get(&(group_id, uri.to_string()))
                .or_else(|| self.webviews.get(uri))
        }
    }

    pub fn webview_for_group_mut(
        &mut self,
        group_id: usize,
        uri: &str,
    ) -> Option<&mut WebviewEditorView> {
        if let Some(&primary_gid) = self.primary_webview_group.get(uri) {
            if primary_gid == group_id {
                self.webviews.get_mut(uri)
            } else {
                self.split_webviews.get_mut(&(group_id, uri.to_string()))
            }
        } else if self.split_webviews.contains_key(&(group_id, uri.to_string())) {
            self.split_webviews.get_mut(&(group_id, uri.to_string()))
        } else {
            self.webviews.get_mut(uri)
        }
    }

    pub fn any_webview_for_uri(&self, uri: &str) -> Option<&WebviewEditorView> {
        self.webviews.get(uri).or_else(|| {
            self.split_webviews
                .iter()
                .find(|((_, u), _)| u == uri)
                .map(|(_, wv)| wv)
        })
    }

    pub fn for_each_webview_for_uri_mut(
        &mut self,
        uri: &str,
        mut f: impl FnMut(&mut WebviewEditorView),
    ) {
        if let Some(wv) = self.webviews.get_mut(uri) {
            f(wv);
        }
        for ((_, u), wv) in &mut self.split_webviews {
            if u == uri {
                f(wv);
            }
        }
    }

    pub fn sync_editor_focus(&mut self) {
        let active_gid = self.model.active_group_id();
        let active_uri = self
            .model
            .current_group_ref()
            .active_tab()
            .map(|t| t.uri.clone());

        for (uri, ed) in &mut self.editors {
            let primary_gid = self.primary_editor_group.get(uri).copied().unwrap_or(active_gid);
            ed.is_focused = primary_gid == active_gid && active_uri.as_deref() == Some(uri.as_str());
        }
        for ((gid, uri), ed) in &mut self.split_editors {
            ed.is_focused = *gid == active_gid && active_uri.as_deref() == Some(uri.as_str());
        }
    }

    pub fn breadcrumbs_rect_for_group(&self, group_id: usize, bounds: Rect) -> Option<Rect> {
        let tab_bar_h = 35.0;
        let has_active_tab = self
            .model
            .group_by_id(group_id)
            .and_then(|g| g.active_tab())
            .is_some();
        if !bounds.is_finite_positive()
            || !(bounds.min_y() + tab_bar_h).is_finite()
            || !self.breadcrumbs_enabled
            || !has_active_tab
            || bounds.height() <= tab_bar_h
        {
            return None;
        }
        let bc_h = (bounds.height() - tab_bar_h).clamp(0.0, 22.0);
        if bc_h <= 0.0 {
            return None;
        }
        let rect = Rect::from_xywh(
            bounds.min_x(),
            bounds.min_y() + tab_bar_h,
            bounds.width(),
            bc_h,
        );
        if rect.is_finite_positive() {
            Some(rect)
        } else {
            None
        }
    }

    pub fn content_rect_for_group(&self, group_id: usize, bounds: Rect) -> Rect {
        if !bounds.is_finite() {
            return Rect::zero();
        }
        let tab_bar_h = 35.0;
        let bc_h = self
            .breadcrumbs_rect_for_group(group_id, bounds)
            .map_or(0.0, |r| r.height());
        let top_offset = tab_bar_h + bc_h;
        let rect = Rect::from_xywh(
            bounds.min_x(),
            bounds.min_y() + top_offset,
            bounds.width(),
            (bounds.height() - top_offset).max(0.0),
        );
        if rect.is_finite() {
            rect
        } else {
            Rect::zero()
        }
    }

    pub fn breadcrumbs_rect(&self, bounds: Rect) -> Option<Rect> {
        self.breadcrumbs_rect_for_group(self.model.active_group_id(), bounds)
    }

    pub fn content_rect(&self, bounds: Rect) -> Rect {
        self.content_rect_for_group(self.model.active_group_id(), bounds)
    }

    pub fn set_minimap_enabled(&mut self, enabled: bool) {
        self.minimap_enabled = enabled;
        self.for_each_editor_mut(|_, editor| {
            editor.set_minimap_enabled(enabled);
        });
    }

    pub fn set_word_wrap(&mut self, mode: WordWrapMode) {
        self.word_wrap_mode = mode;
        self.for_each_editor_mut(|_, editor| {
            editor.set_word_wrap(mode);
        });
    }

    pub fn set_word_wrap_column(&mut self, col: usize) {
        let clamped = col.max(1);
        self.word_wrap_column = clamped;
        self.for_each_editor_mut(|_, editor| {
            editor.set_word_wrap_column(clamped);
        });
    }

    pub fn set_breadcrumbs_enabled(&mut self, enabled: bool) {
        self.breadcrumbs_enabled = enabled;
    }

    pub fn sync_breadcrumbs(&mut self, workspace: Option<&Workspace>) {
        if let Some(ws) = workspace {
            self.known_workspace = Some(ws.clone());
        }
        let ws_ref = self.known_workspace.as_ref();
        let active_gid = self.model.active_group_id();

        let group_snapshots: Vec<(usize, Option<(String, String)>)> = self
            .model
            .groups
            .iter()
            .map(|g| {
                (
                    g.id,
                    g.active_tab().map(|t| (t.uri.clone(), t.title.clone())),
                )
            })
            .collect();

        for (gid, tab_info) in group_snapshots {
            let mut bar = self
                .group_breadcrumbs
                .remove(&gid)
                .unwrap_or_default();
            if let Some((uri, title)) = tab_info {
                let editor_ctx = self
                    .editor_for_group(gid, &uri)
                    .map(|ed| (ed.text(), ed.primary_position().line));
                if let Some((ref content, cursor_line)) = editor_ctx {
                    bar.update(&uri, &title, ws_ref, Some((content, cursor_line)));
                    if gid == active_gid {
                        self.breadcrumbs
                            .update(&uri, &title, ws_ref, Some((content, cursor_line)));
                    }
                } else {
                    bar.update(&uri, &title, ws_ref, None);
                    if gid == active_gid {
                        self.breadcrumbs.update(&uri, &title, ws_ref, None);
                    }
                }
            } else {
                bar.clear();
                if gid == active_gid {
                    self.breadcrumbs.clear();
                }
            }
            self.group_breadcrumbs.insert(gid, bar);
        }
    }

    pub fn is_notebook_uri(uri: &str) -> bool {
        uri.split(['?', '#'])
            .next()
            .unwrap_or(uri)
            .to_ascii_lowercase()
            .ends_with(".ipynb")
    }

    pub fn open_notebook(&mut self, uri: &str, title: &str, content: &str, is_preview: bool) {
        let group = self.model.current_group();
        group.open_tab(uri, title, is_preview);

        if !self.notebooks.contains_key(uri) {
            let nb_view = NotebookEditorView::from_ipynb_str(uri, content)
                .with_grammar_registry(self.grammar_registry.clone());
            self.notebooks.insert(uri.to_string(), nb_view);
        }
        self.retain_open_resources();
        self.sync_breadcrumbs(None);
    }

    pub fn sync_dirty_states(&mut self) {
        let mut dirty_uris = HashSet::new();
        for (uri, ed) in &self.editors {
            if ed.is_dirty() {
                dirty_uris.insert(uri.clone());
            }
        }
        for ((_, uri), ed) in &self.split_editors {
            if ed.is_dirty() {
                dirty_uris.insert(uri.clone());
            }
        }
        for group in &mut self.model.groups {
            for tab in &mut group.tabs {
                if dirty_uris.contains(&tab.uri) {
                    tab.is_dirty = true;
                    tab.is_preview = false;
                } else if let Some(notebook) = self.notebooks.get(&tab.uri) {
                    tab.is_dirty = notebook.document.is_dirty;
                    if tab.is_dirty {
                        tab.is_preview = false;
                    }
                }
            }
        }
    }

    pub fn active_notebook(&self) -> Option<&NotebookEditorView> {
        let uri = self.model.current_group_ref().active_tab()?.uri.clone();
        self.notebooks.get(&uri)
    }

    pub fn active_notebook_mut(&mut self) -> Option<&mut NotebookEditorView> {
        let uri = self.model.current_group().active_tab()?.uri.clone();
        self.notebooks.get_mut(&uri)
    }

    pub fn open_settings(&mut self, config: Option<ConfigurationService>) {
        let uri = "rustcode://settings";
        let group = self.model.current_group();
        group.open_tab(uri, "Settings", false);
        let view = self
            .settings_views
            .entry(uri.to_string())
            .or_default();
        view.state.open();
        if let Some(cfg) = config {
            view.set_config(cfg);
        }
        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
    }

    pub fn split_active_group(&mut self, direction: SplitDirection) -> usize {
        let prev_gid = self.model.active_group_id();
        let active_uri = self
            .model
            .current_group_ref()
            .active_tab()
            .map(|t| t.uri.clone());
        let new_gid = self.model.split_active_group(direction);
        self.group_breadcrumbs.entry(new_gid).or_default();

        if let Some(uri) = active_uri {
            if let Some(primary) = self.editor_for_group(prev_gid, &uri) {
                let doc_opt = primary.document.clone();
                let diags = primary.diagnostics.clone();
                let mut cloned_view = if let Some(doc) = doc_opt {
                    let (buf, lang_id) = {
                        let d = doc.read();
                        (d.buffer.clone(), d.language_id.clone())
                    };
                    let mut v = EditorView::new(buf)
                        .with_document(doc)
                        .with_minimap(self.minimap_enabled)
                        .with_word_wrap(self.word_wrap_mode)
                        .with_word_wrap_column(self.word_wrap_column);
                    let theme = ThemeService::default_dark_theme();
                    v.set_language(&lang_id, theme, &self.grammar_registry);
                    v
                } else {
                    let content = primary.text();
                    EditorView::from_str(&content)
                        .with_minimap(self.minimap_enabled)
                        .with_word_wrap(self.word_wrap_mode)
                        .with_word_wrap_column(self.word_wrap_column)
                };
                if !diags.is_empty() {
                    cloned_view.set_diagnostics(diags);
                }
                self.split_editors.insert((new_gid, uri), cloned_view);
            } else if let Some(primary_wv) = self.webview_for_group(prev_gid, &uri) {
                let mut cloned_wv = WebviewEditorView::new(
                    &primary_wv.id,
                    &primary_wv.view_type,
                    &primary_wv.title,
                    &primary_wv.html,
                    primary_wv.options.clone(),
                    primary_wv.ext_host_manager.clone(),
                );
                cloned_wv.inbound_messages = primary_wv.inbound_messages.clone();
                cloned_wv.outbound_messages = primary_wv.outbound_messages.clone();
                self.primary_webview_group
                    .entry(uri.clone())
                    .or_insert(prev_gid);
                self.split_webviews.insert((new_gid, uri), cloned_wv);
            }
        }

        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
        self.last_action = Some(EditorAreaAction::GroupSplit {
            group_id: new_gid,
            direction,
        });
        new_gid
    }

    pub fn focus_group(&mut self, group_id: usize) -> bool {
        if self.model.focus_group(group_id) {
            self.sync_editor_focus();
            self.sync_breadcrumbs(None);
            self.last_action = Some(EditorAreaAction::GroupFocused(group_id));
            true
        } else {
            false
        }
    }

    pub fn focus_next_group(&mut self) {
        self.model.focus_next_group();
        let gid = self.model.active_group_id();
        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
        self.last_action = Some(EditorAreaAction::GroupFocused(gid));
    }

    pub fn focus_previous_group(&mut self) {
        self.model.focus_previous_group();
        let gid = self.model.active_group_id();
        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
        self.last_action = Some(EditorAreaAction::GroupFocused(gid));
    }

    pub fn focus_first_editor_group(&mut self) -> bool {
        if self.model.focus_first_editor_group() {
            let gid = self.model.active_group_id();
            self.sync_editor_focus();
            self.sync_breadcrumbs(None);
            self.last_action = Some(EditorAreaAction::GroupFocused(gid));
            true
        } else {
            false
        }
    }

    pub fn focus_second_editor_group(&mut self) -> bool {
        if self.model.focus_second_editor_group() {
            let gid = self.model.active_group_id();
            self.sync_editor_focus();
            self.sync_breadcrumbs(None);
            self.last_action = Some(EditorAreaAction::GroupFocused(gid));
            true
        } else {
            false
        }
    }

    fn retain_open_resources(&mut self) {
        let mut open_pairs: HashSet<(usize, String)> = HashSet::new();
        let mut open_uris: HashSet<String> = HashSet::new();
        let mut open_gids: HashSet<usize> = HashSet::new();
        for g in &self.model.groups {
            open_gids.insert(g.id);
            for t in &g.tabs {
                open_pairs.insert((g.id, t.uri.clone()));
                open_uris.insert(t.uri.clone());
            }
        }

        self.split_editors.retain(|k, _| open_pairs.contains(k));
        self.group_breadcrumbs.retain(|gid, _| open_gids.contains(gid));

        // If a primary group dropped its URI while a split group still has it open, promote one.
        let uris_to_check: Vec<String> = self.editors.keys().cloned().collect();
        for uri in uris_to_check {
            if !open_uris.contains(&uri) {
                self.editors.remove(&uri);
                self.primary_editor_group.remove(&uri);
                continue;
            }
            if let Some(&primary_gid) = self.primary_editor_group.get(&uri) {
                if !open_pairs.contains(&(primary_gid, uri.clone())) {
                    if let Some((&(_, _), _)) = self
                        .split_editors
                        .iter()
                        .find(|((_, u), _)| u == &uri)
                    {
                        let promote_key = self
                            .split_editors
                            .keys()
                            .find(|(_, u)| u == &uri)
                            .cloned()
                            .unwrap();
                        if let Some(promoted) = self.split_editors.remove(&promote_key) {
                            self.editors.insert(uri.clone(), promoted);
                            self.primary_editor_group.insert(uri, promote_key.0);
                        }
                    }
                }
            }
        }

        let mut closed_panels: Vec<String> = Vec::new();
        for (panel_id, uri) in &self.custom_editor_panels {
            if !open_uris.contains(uri) && !closed_panels.contains(panel_id) {
                closed_panels.push(panel_id.clone());
            }
        }

        let wv_uris_to_check: Vec<String> = self.webviews.keys().cloned().collect();
        for uri in wv_uris_to_check {
            if !open_uris.contains(&uri) {
                if let Some(wv) = self.webviews.remove(&uri) {
                    if let Some(mgr) = &wv.ext_host_manager {
                        mgr.dispose_panel(&wv.id);
                    }
                    if !closed_panels.contains(&wv.id) {
                        closed_panels.push(wv.id);
                    }
                }
                self.primary_webview_group.remove(&uri);
                continue;
            }
            if let Some(&primary_gid) = self.primary_webview_group.get(&uri) {
                if !open_pairs.contains(&(primary_gid, uri.clone())) {
                    if let Some(promote_key) = self
                        .split_webviews
                        .keys()
                        .find(|(_, u)| u == &uri)
                        .cloned()
                    {
                        if let Some(promoted) = self.split_webviews.remove(&promote_key) {
                            self.webviews.insert(uri.clone(), promoted);
                            self.primary_webview_group.insert(uri, promote_key.0);
                        }
                    }
                }
            }
        }

        self.split_webviews.retain(|k, wv| {
            if open_pairs.contains(k) {
                true
            } else {
                if !open_uris.contains(&k.1) {
                    if let Some(mgr) = &wv.ext_host_manager {
                        mgr.dispose_panel(&wv.id);
                    }
                    if !closed_panels.contains(&wv.id) {
                        closed_panels.push(wv.id.clone());
                    }
                }
                false
            }
        });

        for panel_id in closed_panels {
            if !self.closed_webview_panels.contains(&panel_id) {
                self.closed_webview_panels.push(panel_id);
            }
        }

        self.diff_editors.retain(|u, _| open_uris.contains(u));
        self.settings_views.retain(|u, _| open_uris.contains(u));
        self.notebooks.retain(|u, _| open_uris.contains(u));
        self.custom_editor_panels
            .retain(|_, u| open_uris.contains(u));
    }

    pub fn close_tab_in_group(&mut self, group_id: usize, index: usize) -> bool {
        let Some(group) = self.model.group_by_id_mut(group_id) else {
            return false;
        };
        let closed = group.close_tab(index);
        if closed {
            let group_empty = self
                .model
                .group_by_id(group_id)
                .is_some_and(|g| g.tabs.is_empty());
            if group_empty && self.model.groups.len() > 1 {
                if self.model.close_group(group_id) {
                    self.active_sash_drag = None;
                }
                self.last_action = Some(EditorAreaAction::GroupClosed(group_id));
            } else {
                self.last_action = Some(EditorAreaAction::TabClosed(index));
            }
            self.retain_open_resources();
            self.sync_editor_focus();
            self.sync_breadcrumbs(None);
        }
        closed
    }

    pub fn close_tab(&mut self, index: usize) -> bool {
        let gid = self.model.active_group_id();
        self.close_tab_in_group(gid, index)
    }

    pub fn close_editors_in_active_group(&mut self) -> bool {
        let gid = self.model.active_group_id();
        let had_tabs = self
            .model
            .group_by_id(gid)
            .is_some_and(|g| !g.tabs.is_empty());
        if let Some(group) = self.model.group_by_id_mut(gid) {
            group.tabs.clear();
            group.active_tab_index = None;
        }
        let collapsed = if self.model.groups.len() > 1 {
            self.model.close_group(gid)
        } else {
            false
        };
        if collapsed {
            self.active_sash_drag = None;
            self.last_action = Some(EditorAreaAction::GroupClosed(gid));
        }
        self.retain_open_resources();
        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
        had_tabs || collapsed
    }

    pub fn open_diff(&mut self, uri: &str, title: &str, original: &str, modified: &str) {
        let group = self.model.current_group();
        group.open_tab(uri, title, false);
        self.diff_editors
            .insert(uri.to_string(), DiffEditorView::new(title, original, modified));
        self.retain_open_resources();
        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
    }

    pub fn open_webview(
        &mut self,
        id: &str,
        title: &str,
        view_type: &str,
        html: &str,
        options: WebviewOptions,
        ext_host_manager: Option<ExtHostWebviewManager>,
    ) {
        self.open_webview_with_key(id, id, title, view_type, html, options, ext_host_manager);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn open_webview_with_key(
        &mut self,
        uri: &str,
        panel_id: &str,
        title: &str,
        view_type: &str,
        html: &str,
        options: WebviewOptions,
        ext_host_manager: Option<ExtHostWebviewManager>,
    ) {
        let gid = self.model.active_group_id();
        let group = self.model.current_group();
        group.open_tab(uri, title, false);
        let view = WebviewEditorView::new(panel_id, view_type, title, html, options, ext_host_manager);
        let primary_gid = self.primary_webview_group.get(uri).copied();
        if !self.webviews.contains_key(uri) || primary_gid.is_none_or(|pg| pg == gid) {
            self.webviews.insert(uri.to_string(), view);
            self.primary_webview_group.insert(uri.to_string(), gid);
        } else {
            self.split_webviews.insert((gid, uri.to_string()), view);
        }
        self.retain_open_resources();
        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
    }

    pub fn register_custom_editor_contributions(
        &mut self,
        contributions: Vec<CustomEditorContribution>,
    ) {
        self.custom_editor_contributions = contributions;
    }

    pub fn find_matching_custom_editor(&self, uri: &str) -> Option<&CustomEditorContribution> {
        self.custom_editor_contributions
            .iter()
            .filter(|c| c.matches_path(uri))
            .min_by_key(|c| match c.priority.as_deref() {
                Some("default") | None => 0u8,
                _ => 1u8,
            })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn open_custom_editor(
        &mut self,
        uri: &str,
        title: &str,
        view_type: &str,
        panel_id: &str,
        html: &str,
        options: WebviewOptions,
        ext_host_manager: Option<ExtHostWebviewManager>,
        is_preview: bool,
    ) {
        let gid = self.model.active_group_id();
        let group = self.model.current_group();
        group.open_tab(uri, title, is_preview);

        // Remove any stale text editor instances if transitioning URI to custom editor
        self.editors.remove(uri);
        self.primary_editor_group.remove(uri);
        self.split_editors.retain(|(_, u), _| u != uri);

        let view = WebviewEditorView::new(panel_id, view_type, title, html, options, ext_host_manager);
        let primary_gid = self.primary_webview_group.get(uri).copied();
        if !self.webviews.contains_key(uri) || primary_gid.is_none_or(|pg| pg == gid) {
            self.webviews.insert(uri.to_string(), view);
            self.primary_webview_group.insert(uri.to_string(), gid);
        } else {
            self.split_webviews.insert((gid, uri.to_string()), view);
        }
        self.custom_editor_panels
            .retain(|pid, u| u != uri || pid == panel_id);
        self.custom_editor_panels
            .insert(panel_id.to_string(), uri.to_string());
        self.retain_open_resources();
        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
    }

    pub fn open_file(&mut self, uri: &str, title: &str, content: &str, is_preview: bool) {
        if Self::is_notebook_uri(uri) {
            self.open_notebook(uri, title, content, is_preview);
            return;
        }
        if let Some(contrib) = self.find_matching_custom_editor(uri).cloned() {
            let panel_id = format!("custom_editor:{}", uri);
            let display = if contrib.display_name.is_empty() {
                contrib.view_type.as_str()
            } else {
                contrib.display_name.as_str()
            };
            let html = format!("<h1>{}</h1><p>Custom Editor: {}</p>", title, display);
            self.open_custom_editor(
                uri,
                title,
                &contrib.view_type,
                &panel_id,
                &html,
                WebviewOptions::default(),
                None,
                is_preview,
            );
            return;
        }

        let gid = self.model.active_group_id();
        self.model.current_group().open_tab(uri, title, is_preview);

        if !self.editors.contains_key(uri) {
            let path_buf = Some(rustcode_platform::file_uri_to_path(uri));

            let grammar = if let Some(p) = &path_buf {
                self.grammar_registry.get_for_path(p)
            } else {
                self.grammar_registry.get_by_id("plaintext").unwrap()
            };

            let doc = Arc::new(RwLock::new(TextDocument::new(
                content,
                path_buf,
                Some(grammar.language_id()),
            )));

            let mut editor = EditorView::from_str(content)
                .with_document(doc)
                .with_minimap(self.minimap_enabled)
                .with_word_wrap(self.word_wrap_mode)
                .with_word_wrap_column(self.word_wrap_column);
            let theme = ThemeService::default_dark_theme();
            editor.set_language(grammar.language_id(), theme, &self.grammar_registry);
            if let Some(cfg) = self.language_configs.get(grammar.language_id()).cloned() {
                editor.set_language_config(Some(cfg));
            }

            self.editors.insert(uri.to_string(), editor);
            self.primary_editor_group.insert(uri.to_string(), gid);
        } else if self.editor_for_group(gid, uri).is_none() {
            let (doc_opt, diags, text_fallback) = {
                let primary = &self.editors[uri];
                (
                    primary.document.clone(),
                    primary.diagnostics.clone(),
                    primary.text(),
                )
            };
            let mut editor = if let Some(doc) = doc_opt {
                let (buf, lang_id) = {
                    let d = doc.read();
                    (d.buffer.clone(), d.language_id.clone())
                };
                let mut v = EditorView::new(buf)
                    .with_document(doc)
                    .with_minimap(self.minimap_enabled)
                    .with_word_wrap(self.word_wrap_mode)
                    .with_word_wrap_column(self.word_wrap_column);
                let theme = ThemeService::default_dark_theme();
                v.set_language(&lang_id, theme, &self.grammar_registry);
                v
            } else {
                EditorView::from_str(&text_fallback)
                    .with_minimap(self.minimap_enabled)
                    .with_word_wrap(self.word_wrap_mode)
                    .with_word_wrap_column(self.word_wrap_column)
            };
            if !diags.is_empty() {
                editor.set_diagnostics(diags);
            }
            self.split_editors.insert((gid, uri.to_string()), editor);
        }

        self.retain_open_resources();
        self.sync_editor_focus();
        self.sync_breadcrumbs(None);
    }

    pub fn save_active_file(
        &mut self,
        fs: &FileSystemService,
    ) -> Result<FileMetadata, FileSystemError> {
        let active_gid = self.model.active_group_id();
        let uri = self
            .model
            .current_group()
            .active_tab()
            .map(|t| t.uri.clone())
            .ok_or_else(|| {
                FileSystemError::NotFound(PathBuf::from("no_active_tab"))
            })?;

        if let Some(editor) = self.editor_for_group_mut(active_gid, &uri) {
            let meta = editor.save_sync(fs)?;
            for group in &mut self.model.groups {
                for tab in &mut group.tabs {
                    if tab.uri == uri {
                        tab.is_dirty = false;
                    }
                }
            }
            Ok(meta)
        } else if let Some(notebook) = self.notebooks.get_mut(&uri) {
            let path = notebook
                .document
                .path
                .clone()
                .or_else(|| {
                    if uri.starts_with("untitled:") {
                        None
                    } else if rustcode_platform::is_file_uri(&uri) {
                        Some(rustcode_platform::file_uri_to_path(&uri))
                    } else {
                        None
                    }
                })
                .ok_or_else(|| FileSystemError::NotFound(PathBuf::from(&uri)))?;

            let json = notebook.document.to_ipynb_json().map_err(|e| {
                FileSystemError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            })?;
            let meta = fs.write_atomic_sync(&path, &json)?;
            notebook.document.path = Some(path);
            notebook.document.is_dirty = false;
            if let Some(idx) = self.model.current_group().active_tab_index {
                self.model.current_group().set_dirty(idx, false);
            }
            Ok(meta)
        } else {
            Err(FileSystemError::NotFound(PathBuf::from(&uri)))
        }
    }

    pub fn active_editor_mut(&mut self) -> Option<&mut EditorView> {
        let gid = self.model.active_group_id();
        let uri = self.model.current_group_ref().active_tab()?.uri.clone();
        self.editor_for_group_mut(gid, &uri)
    }

    pub fn active_editor(&self) -> Option<&EditorView> {
        let gid = self.model.active_group_id();
        let uri = self.model.current_group_ref().active_tab()?.uri.clone();
        self.editor_for_group(gid, &uri)
    }

    pub fn apply_text_edits(&mut self, uri: &str, edits: &[RangeEdit]) -> bool {
        let active_gid = self.model.active_group_id();
        let changed = if let Some(editor) = self.editor_for_group_mut(active_gid, uri) {
            editor.apply_text_edits(edits)
        } else if let Some(editor) = self.editors.get_mut(uri) {
            editor.apply_text_edits(edits)
        } else {
            return false;
        };

        if changed {
            // Recompute folding ranges on any sibling split views sharing this document.
            if let Some(doc) = self.any_editor_for_uri(uri).and_then(|e| e.document.clone()) {
                let (buf_arc, lang_id) = {
                    let d = doc.read();
                    (d.buffer.clone(), d.language_id.clone())
                };
                let buf = buf_arc.read();
                self.for_each_editor_for_uri_mut(uri, |ed| {
                    ed.folding_model.compute_from_buffer(&buf, Some(&lang_id));
                });
            }

            for group in &mut self.model.groups {
                for tab in &mut group.tabs {
                    if tab.uri == uri {
                        tab.is_dirty = true;
                        tab.is_preview = false;
                    }
                }
            }
            self.sync_breadcrumbs(None);
        }
        changed
    }

    pub fn apply_workspace_edit(&mut self, edit: &WorkspaceEdit) -> usize {
        let mut total_applied = 0;

        if let Some(doc_changes) = &edit.document_changes {
            for change in doc_changes {
                match change {
                    DocumentChange::CreateFile { uri } => {
                        let path = rustcode_platform::file_uri_to_path(uri);
                        if !path.exists() {
                            if let Some(parent) = path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            if std::fs::write(&path, "").is_ok() {
                                total_applied += 1;
                            }
                        }
                    }
                    DocumentChange::RenameFile { old_uri, new_uri } => {
                        let old_path = rustcode_platform::file_uri_to_path(old_uri);
                        let new_path = rustcode_platform::file_uri_to_path(new_uri);
                        if old_path.exists() {
                            if let Some(parent) = new_path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            if std::fs::rename(&old_path, &new_path).is_ok() {
                                total_applied += 1;
                            }
                        }
                    }
                    DocumentChange::DeleteFile { uri } => {
                        let path = rustcode_platform::file_uri_to_path(uri);
                        if path.exists() {
                            let ok = if path.is_dir() {
                                std::fs::remove_dir_all(&path).is_ok()
                            } else {
                                std::fs::remove_file(&path).is_ok()
                            };
                            if ok {
                                total_applied += 1;
                            }
                        }
                    }
                    DocumentChange::Edit(_) => {}
                }
            }
        }

        for (uri, lsp_edits) in edit.all_text_edits() {
            let range_edits: Vec<RangeEdit> = lsp_edits
                .into_iter()
                .map(|e| RangeEdit {
                    range: Range::new(
                        Position::new(e.range.start.line as usize, e.range.start.character as usize),
                        Position::new(e.range.end.line as usize, e.range.end.character as usize),
                    ),
                    new_text: e.new_text,
                })
                .collect();

            let path = rustcode_platform::file_uri_to_path(&uri);
            let target_uri = self
                .find_open_tab_uri_for_path(&path)
                .unwrap_or_else(|| uri.clone());

            if self.apply_text_edits(&target_uri, &range_edits) {
                total_applied += range_edits.len();
            } else if let Ok(content) = std::fs::read_to_string(&path) {
                let fallback_title = path.to_string_lossy().into_owned();
                let title = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&fallback_title);
                self.open_file(&uri, title, &content, false);
                if self.apply_text_edits(&uri, &range_edits) {
                    total_applied += range_edits.len();
                }
            }
        }

        total_applied
    }

    pub fn snapshot_files_for_workspace_edit(
        &self,
        edit: &WorkspaceEdit,
        label: impl Into<String>,
    ) -> WorkspaceEditUndoEntry {
        let mut uris: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (uri, _) in edit.all_text_edits() {
            if seen.insert(uri.clone()) {
                uris.push(uri);
            }
        }
        if let Some(doc_changes) = &edit.document_changes {
            for c in doc_changes {
                match c {
                    DocumentChange::Edit(e) => {
                        let u = &e.text_document.uri;
                        if seen.insert(u.clone()) {
                            uris.push(u.clone());
                        }
                    }
                    DocumentChange::CreateFile { uri } | DocumentChange::DeleteFile { uri } => {
                        if seen.insert(uri.clone()) {
                            uris.push(uri.clone());
                        }
                    }
                    DocumentChange::RenameFile { old_uri, new_uri } => {
                        for u in [old_uri, new_uri] {
                            if seen.insert(u.clone()) {
                                uris.push(u.clone());
                            }
                        }
                    }
                }
            }
        }

        let snapshots = uris
            .into_iter()
            .map(|uri| {
                let path = rustcode_platform::file_uri_to_path(&uri);
                let resolved_uri = self
                    .find_open_tab_uri_for_path(&path)
                    .unwrap_or_else(|| uri.clone());
                let open_editor = self.any_editor_for_uri(&resolved_uri);
                let existed_before = path.exists();
                let previous_content = if let Some(ed) = open_editor {
                    Some(ed.text())
                } else {
                    std::fs::read_to_string(&path).ok()
                };
                FileUndoSnapshot {
                    uri: resolved_uri,
                    path: (!uri.is_empty()).then_some(path),
                    existed_before,
                    previous_content,
                    was_open: open_editor.is_some(),
                }
            })
            .collect();

        WorkspaceEditUndoEntry {
            label: label.into(),
            snapshots,
        }
    }

    pub fn restore_workspace_edit_snapshot(&mut self, entry: &WorkspaceEditUndoEntry) -> usize {
        let mut restored = 0;
        for snap in &entry.snapshots {
            if !snap.existed_before && !snap.was_open {
                if let Some(path) = &snap.path {
                    let closed = self.close_tabs_for_path(path) > 0;
                    let removed = path.exists() && std::fs::remove_file(path).is_ok();
                    if closed || removed {
                        restored += 1;
                    }
                }
                continue;
            }
            if let Some(prev) = &snap.previous_content {
                let is_open = self.any_editor_for_uri(&snap.uri).is_some();
                if is_open {
                    self.for_each_editor_for_uri_mut(&snap.uri, |ed| {
                        *ed.buffer.write() = rustcode_editor::PieceTable::from_str(prev);
                        if let Some(doc) = &ed.document {
                            let mut d = doc.write();
                            d.version_id = d.version_id.saturating_add(1);
                        }
                        ed.recompute_folding();
                    });
                    restored += 1;
                } else if let Some(path) = &snap.path {
                    if snap.existed_before {
                        if std::fs::write(path, prev).is_ok() {
                            restored += 1;
                        }
                    } else if path.exists() && std::fs::remove_file(path).is_ok() {
                        restored += 1;
                    }
                }
            }
        }
        if restored > 0 {
            self.sync_dirty_states();
            self.sync_breadcrumbs(None);
        }
        restored
    }

    fn route_event_to_group_content(
        &mut self,
        group_id: usize,
        event: &InputEvent,
        content_rect: Rect,
    ) -> EventResult {
        let Some(uri) = self
            .model
            .group_by_id(group_id)
            .and_then(|g| g.active_tab())
            .map(|t| t.uri.clone())
        else {
            return EventResult::Ignored;
        };

        if let Some(diff_view) = self.diff_editors.get_mut(&uri) {
            diff_view.handle_event(event, content_rect)
        } else if let Some(webview) = self.webview_for_group_mut(group_id, &uri) {
            webview.handle_event(event, content_rect)
        } else if let Some(settings_view) = self.settings_views.get_mut(&uri) {
            settings_view.handle_event(event, content_rect)
        } else if let Some(notebook) = self.notebooks.get_mut(&uri) {
            let res = notebook.handle_event(event, content_rect);
            self.sync_dirty_states();
            res
        } else if let Some(editor) = self.editor_for_group_mut(group_id, &uri) {
            let track_edit = matches!(event, InputEvent::KeyDown(_) | InputEvent::MouseDown(_));
            let before_state = if track_edit {
                Some((
                    editor.buffer.read().len(),
                    editor.buffer.read().line_count(),
                    editor.document.as_ref().map(|d| d.read().version_id),
                ))
            } else {
                None
            };
            let res = editor.handle_event(event, content_rect);
            let after_state = if track_edit && res == EventResult::Consumed {
                Some((
                    editor.buffer.read().len(),
                    editor.buffer.read().line_count(),
                    editor.document.as_ref().map(|d| d.read().version_id),
                ))
            } else {
                None
            };
            if track_edit && res == EventResult::Consumed {
                self.sync_dirty_states();
                if before_state != after_state {
                    self.for_each_editor_for_uri_mut(&uri, |ed| {
                        ed.recompute_folding();
                    });
                }
                self.sync_breadcrumbs(None);
            }
            res
        } else {
            EventResult::Ignored
        }
    }
}


impl View for EditorAreaView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !bounds.is_finite_positive()
            || !font.is_finite()
            || !(bounds.min_y() + 35.0).is_finite()
        {
            return;
        }

        let tab_bar_h = 35.0;
        let group_rects = self.model.compute_group_rects(bounds);
        let sashes = self.model.compute_sashes(bounds);
        let active_gid = self.model.active_group_id();
        let multi_group = self.model.groups.len() > 1;

        for (group_id, group_rect) in group_rects {
            if !group_rect.is_finite_positive()
                || !(group_rect.min_y() + tab_bar_h).is_finite()
            {
                continue;
            }
            let Some(group) = self.model.group_by_id(group_id) else {
                continue;
            };
            let is_active_group = group_id == active_gid;

            scene.push_clip(group_rect);

            let tab_bar_rect =
                Rect::from_xywh(group_rect.min_x(), group_rect.min_y(), group_rect.width(), tab_bar_h);
            scene.push_finite_quad(
                Quad::new(tab_bar_rect)
                    .with_background(theme.tab_inactive_bg)
                    .with_border_edges(theme.tab_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
            );

            let mut cur_x = group_rect.min_x();
            for (idx, tab) in group.tabs.iter().enumerate() {
                let is_active = group.active_tab_index == Some(idx);
                let tab_w = (font.text_width(&tab.title) + 40.0).clamp(80.0, 220.0);
                let tab_rect = Rect::from_xywh(cur_x, group_rect.min_y(), tab_w, tab_bar_h);
                if !tab_rect.is_finite() {
                    break;
                }

                let bg_color = if is_active {
                    theme.tab_active_bg
                } else {
                    theme.tab_inactive_bg
                };

                let mut tab_quad = Quad::new(tab_rect).with_background(bg_color);
                if is_active && (!multi_group || is_active_group) {
                    tab_quad = tab_quad.with_border_edges(
                        theme.activity_bar_active_border,
                        Edges::new(2.0, 0.0, 0.0, 0.0),
                    );
                } else {
                    tab_quad = tab_quad.with_border_edges(
                        theme.tab_border,
                        Edges::new(0.0, 1.0, 0.0, 0.0),
                    );
                }
                scene.push_quad(tab_quad);

                let fg_color = if is_active {
                    theme.tab_active_fg
                } else {
                    theme.tab_inactive_fg
                };

                let is_dirty = tab.is_dirty
                    || self
                        .any_editor_for_uri(&tab.uri)
                        .is_some_and(|e| e.is_dirty())
                    || self.notebooks.get(&tab.uri).is_some_and(|n| n.document.is_dirty);

                let title_text = if is_dirty {
                    format!("{} \u{25cf}", tab.title)
                } else {
                    tab.title.clone()
                };

                let title_y = group_rect.min_y() + (tab_bar_h - font.line_height) / 2.0;
                let title_x = cur_x + 10.0;
                let mut run = GlyphRun::new(
                    title_text,
                    Point::new(title_x, title_y),
                    Rect::from_xywh(title_x, title_y, tab_w - 24.0, font.line_height),
                    fg_color,
                    font.font_size,
                );
                if tab.is_preview {
                    run = run.with_style(false, true, false);
                }
                scene.push_text(run);

                let close_x = cur_x + tab_w - 18.0;
                scene.push_text(GlyphRun::new(
                    "x".to_string(),
                    Point::new(close_x, title_y),
                    Rect::from_xywh(close_x, title_y, 14.0, font.line_height),
                    fg_color,
                    font.font_size * 0.85,
                ));

                cur_x += tab_w;
            }

            if let Some(bc_rect) = self.breadcrumbs_rect_for_group(group_id, group_rect) {
                let bc_bar = self
                    .group_breadcrumbs
                    .get(&group_id)
                    .unwrap_or(&self.breadcrumbs);
                bc_bar.render(bc_rect, theme, font, scene);
            }

            let content_rect = self.content_rect_for_group(group_id, group_rect);
            if let Some(tab) = group.active_tab() {
                if let Some(diff_view) = self.diff_editors.get(&tab.uri) {
                    diff_view.render(content_rect, theme, font, scene);
                } else if let Some(webview) = self.webview_for_group(group_id, &tab.uri) {
                    webview.render(content_rect, theme, font, scene);
                } else if let Some(settings_view) = self.settings_views.get(&tab.uri) {
                    settings_view.render(content_rect, theme, font, scene);
                } else if let Some(notebook) = self.notebooks.get(&tab.uri) {
                    notebook.render(content_rect, theme, font, scene);
                } else if let Some(editor) = self.editor_for_group(group_id, &tab.uri) {
                    editor.render(content_rect, theme, font, scene);
                } else {
                    scene.push_finite_quad(
                        Quad::new(content_rect).with_background(theme.editor_background),
                    );
                }
            } else {
                scene.push_finite_quad(
                    Quad::new(content_rect).with_background(theme.editor_background),
                );
            }

            if multi_group {
                let border_color = if is_active_group {
                    theme.activity_bar_active_border
                } else {
                    theme.tab_border
                };
                scene.push_finite_quad(
                    Quad::new(group_rect).with_border_edges(border_color, Edges::all(1.0)),
                );
            }

            scene.pop_clip();
        }

        for sash in sashes {
            let is_active = self
                .active_sash_drag
                .as_ref()
                .is_some_and(|(path, _)| path == &sash.split_path);
            let color = if is_active {
                theme.sash_hover
            } else {
                theme.panel_border
            };
            scene.push_finite_quad(Quad::new(sash.rect).with_background(color));
        }

        if let Some(drag) = &self.active_tab_drag {
            if drag.active {
                if let Some(drop_zone) = self.resolve_tab_drop_target(bounds, drag.current_pos) {
                    let group_rects = self.model.compute_group_rects(bounds);
                    match drop_zone {
                        TabDropZone::ReorderTab { group_id, target_index } => {
                            if let Some((_, grect)) =
                                group_rects.into_iter().find(|(id, _)| *id == group_id)
                            {
                                let rects = self.tab_rects_for_group(group_id, grect, font);
                                if let Some((_, tr, _)) = rects.get(target_index) {
                                    scene.push_finite_quad(
                                        Quad::new(Rect::from_xywh(tr.min_x(), tr.min_y(), 3.0, tr.height()))
                                            .with_background(theme.activity_bar_active_border),
                                    );
                                }
                            }
                        }
                        TabDropZone::MoveToGroup(gid) => {
                            if let Some((_, grect)) = group_rects.into_iter().find(|(id, _)| *id == gid) {
                                scene.push_finite_quad(
                                    Quad::new(grect)
                                        .with_background(Rgba::new(0.0, 0.48, 0.8, 0.16))
                                        .with_border_edges(theme.activity_bar_active_border, Edges::all(2.0)),
                                );
                            }
                        }
                        TabDropZone::SplitGroup { group_id, direction, insert_before } => {
                            if let Some((_, grect)) =
                                group_rects.into_iter().find(|(id, _)| *id == group_id)
                            {
                                let preview = match (direction, insert_before) {
                                    (SplitDirection::Horizontal, true) => {
                                        Rect::from_xywh(grect.min_x(), grect.min_y(), grect.width() * 0.5, grect.height())
                                    }
                                    (SplitDirection::Horizontal, false) => {
                                        Rect::from_xywh(grect.min_x() + grect.width() * 0.5, grect.min_y(), grect.width() * 0.5, grect.height())
                                    }
                                    (SplitDirection::Vertical, true) => {
                                        Rect::from_xywh(grect.min_x(), grect.min_y(), grect.width(), grect.height() * 0.5)
                                    }
                                    (SplitDirection::Vertical, false) => {
                                        Rect::from_xywh(grect.min_x(), grect.min_y() + grect.height() * 0.5, grect.width(), grect.height() * 0.5)
                                    }
                                };
                                scene.push_finite_quad(
                                    Quad::new(preview)
                                        .with_background(Rgba::new(0.0, 0.48, 0.8, 0.22))
                                        .with_border_edges(theme.activity_bar_active_border, Edges::all(2.0)),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let tab_bar_h = 35.0;

        match event {
            InputEvent::MouseDown(mouse) => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }

                // 1. Check split sashes first (with 2px hit tolerance)
                for sash in self.model.compute_sashes(bounds) {
                    let hit_rect = Rect::from_xywh(
                        sash.rect.min_x() - 2.0,
                        sash.rect.min_y() - 2.0,
                        sash.rect.width() + 4.0,
                        sash.rect.height() + 4.0,
                    );
                    if hit_rect.contains(mouse.position) {
                        self.active_sash_drag = Some((sash.split_path, sash.parent_bounds));
                        return EventResult::Consumed;
                    }
                }

                // 2. Hit-test target group
                let Some((group_id, group_rect)) = self
                    .model
                    .compute_group_rects(bounds)
                    .into_iter()
                    .find(|(_, rect)| rect.contains(mouse.position))
                else {
                    return EventResult::Ignored;
                };

                let group_changed = group_id != self.model.active_group_id();
                if group_changed {
                    self.focus_group(group_id);
                }

                if mouse.position.y < group_rect.min_y() + tab_bar_h {
                    let font = FontMetrics::default_monospace();
                    let tab_rects = self.tab_rects_for_group(group_id, group_rect, &font);

                    for (idx, tab_rect, close_min_x) in tab_rects {
                        if tab_rect.contains(mouse.position) {
                            if mouse.position.x >= close_min_x && mouse.button == MouseButton::Left {
                                self.close_tab_in_group(group_id, idx);
                            } else {
                                if let Some(group) = self.model.group_by_id_mut(group_id) {
                                    group.active_tab_index = Some(idx);
                                }
                                if mouse.button == MouseButton::Left {
                                    if let Some(tab) = self
                                        .model
                                        .group_by_id(group_id)
                                        .and_then(|g| g.tabs.get(idx))
                                    {
                                        self.active_tab_drag = Some(TabDragState {
                                            source_group_id: group_id,
                                            source_tab_index: idx,
                                            uri: tab.uri.clone(),
                                            start_pos: mouse.position,
                                            current_pos: mouse.position,
                                            active: false,
                                        });
                                    }
                                }
                                self.last_action = Some(EditorAreaAction::TabSelected(idx));
                                self.sync_editor_focus();
                                self.sync_breadcrumbs(None);
                            }
                            return EventResult::Consumed;
                        }
                    }
                    if group_changed {
                        EventResult::Consumed
                    } else {
                        EventResult::Ignored
                    }
                } else if let Some(bc_rect) =
                    self.breadcrumbs_rect_for_group(group_id, group_rect)
                {
                    if bc_rect.contains(mouse.position) {
                        let active_gid = self.model.active_group_id();
                        let (res, action) = {
                            let bc_bar = self.group_breadcrumbs.entry(group_id).or_default();
                            let r = bc_bar.handle_event(event, bc_rect);
                            let act = bc_bar.last_action.take();
                            if group_id == active_gid {
                                self.breadcrumbs.items = bc_bar.items.clone();
                                self.breadcrumbs.selected_index = bc_bar.selected_index;
                            }
                            (r, act)
                        };
                        if let Some(BreadcrumbsAction::SelectItem { item, .. }) = action {
                            if matches!(item.kind, BreadcrumbKind::Symbol(_)) {
                                if let Some(line) = item.line {
                                    let col = item.column.unwrap_or(0);
                                    let content_rect =
                                        self.content_rect_for_group(group_id, group_rect);
                                    let font = FontMetrics::default_monospace();
                                    let visible_lines =
                                        (content_rect.height() / font.line_height).floor().max(1.0);
                                    let uri_opt = self
                                        .model
                                        .group_by_id(group_id)
                                        .and_then(|g| g.active_tab())
                                        .map(|t| t.uri.clone());
                                    if let Some(uri) = uri_opt {
                                        if let Some(editor) =
                                            self.editor_for_group_mut(group_id, &uri)
                                        {
                                            editor.set_primary_position(Position::new(line, col));
                                            let line_f = line as f32;
                                            if line_f < editor.scroll_top
                                                || line_f >= editor.scroll_top + visible_lines
                                            {
                                                editor.scroll_top = (line_f - 2.0).max(0.0);
                                            }
                                        }
                                    }
                                    self.sync_breadcrumbs(None);
                                }
                            }
                            self.last_action = Some(EditorAreaAction::BreadcrumbSelected(item));
                        }
                        return res;
                    }
                    let content_rect = self.content_rect_for_group(group_id, group_rect);
                    let res = self.route_event_to_group_content(group_id, event, content_rect);
                    if group_changed && res == EventResult::Ignored {
                        EventResult::Consumed
                    } else {
                        res
                    }
                } else {
                    let content_rect = self.content_rect_for_group(group_id, group_rect);
                    let res = self.route_event_to_group_content(group_id, event, content_rect);
                    if group_changed && res == EventResult::Ignored {
                        EventResult::Consumed
                    } else {
                        res
                    }
                }
            }
            InputEvent::MouseMove(mouse) => {
                if let Some(drag) = &mut self.active_tab_drag {
                    drag.current_pos = mouse.position;
                    let dx = mouse.position.x - drag.start_pos.x;
                    let dy = mouse.position.y - drag.start_pos.y;
                    if !drag.active && (dx * dx + dy * dy).sqrt() >= 6.0 {
                        drag.active = true;
                    }
                    if drag.active {
                        return EventResult::Consumed;
                    }
                }

                if let Some((path, parent_bounds)) = self.active_sash_drag.clone() {
                    self.model
                        .update_sash_ratio(&path, mouse.position, parent_bounds);
                    return EventResult::Consumed;
                }

                // Route to any editor currently dragging minimap
                let dragging_target = self
                    .model
                    .groups
                    .iter()
                    .find_map(|g| {
                        g.active_tab().and_then(|t| {
                            self.editor_for_group(g.id, &t.uri)
                                .filter(|e| e.minimap_dragging)
                                .map(|_| g.id)
                        })
                    });

                let target_group = if let Some(gid) = dragging_target {
                    self.model
                        .compute_group_rects(bounds)
                        .into_iter()
                        .find(|(id, _)| *id == gid)
                } else {
                    self.model
                        .compute_group_rects(bounds)
                        .into_iter()
                        .find(|(_, rect)| rect.contains(mouse.position))
                };

                if let Some((gid, rect)) = target_group {
                    let content_rect = self.content_rect_for_group(gid, rect);
                    self.route_event_to_group_content(gid, event, content_rect)
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::MouseUp(mouse) => {
                if let Some(drag) = self.active_tab_drag.take() {
                    if drag.active {
                        self.active_tab_drag = Some(drag.clone());
                        let drop_target = self.resolve_tab_drop_target(bounds, mouse.position);
                        self.active_tab_drag = None;
                        if let Some(zone) = drop_target {
                            match zone {
                                TabDropZone::ReorderTab {
                                    group_id,
                                    target_index,
                                } => {
                                    if group_id == drag.source_group_id {
                                        self.model.reorder_tab_in_group(
                                            group_id,
                                            drag.source_tab_index,
                                            target_index,
                                        );
                                        self.last_action =
                                            Some(EditorAreaAction::TabSelected(target_index));
                                        self.sync_editor_focus();
                                        self.sync_breadcrumbs(None);
                                    } else {
                                        self.move_tab_to_group(
                                            drag.source_group_id,
                                            drag.source_tab_index,
                                            group_id,
                                        );
                                    }
                                }
                                TabDropZone::MoveToGroup(gid) => {
                                    self.move_tab_to_group(
                                        drag.source_group_id,
                                        drag.source_tab_index,
                                        gid,
                                    );
                                }
                                TabDropZone::SplitGroup {
                                    group_id,
                                    direction,
                                    insert_before,
                                } => {
                                    if let Some(new_gid) = self.model.split_group_empty(
                                        group_id,
                                        direction,
                                        insert_before,
                                    ) {
                                        self.group_breadcrumbs.entry(new_gid).or_default();
                                        self.move_tab_to_group(
                                            drag.source_group_id,
                                            drag.source_tab_index,
                                            new_gid,
                                        );
                                    }
                                }
                            }
                        }
                        return EventResult::Consumed;
                    }
                }
                if self.active_sash_drag.take().is_some() {
                    return EventResult::Consumed;
                }
                let gid = self.model.active_group_id();
                let rect = self
                    .model
                    .compute_group_rects(bounds)
                    .into_iter()
                    .find(|(id, _)| *id == gid)
                    .map(|(_, r)| r)
                    .unwrap_or(bounds);
                let content_rect = self.content_rect_for_group(gid, rect);
                let res = self.route_event_to_group_content(gid, event, content_rect);
                self.clear_all_minimap_dragging();
                res
            }
            InputEvent::ScrollWheel(scroll) => {
                if let Some((gid, rect)) = self
                    .model
                    .compute_group_rects(bounds)
                    .into_iter()
                    .find(|(_, rect)| rect.contains(scroll.position))
                {
                    let content_rect = self.content_rect_for_group(gid, rect);
                    self.route_event_to_group_content(gid, event, content_rect)
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::KeyDown(_) | InputEvent::KeyUp(_) => {
                let gid = self.model.active_group_id();
                let rect = self
                    .model
                    .compute_group_rects(bounds)
                    .into_iter()
                    .find(|(id, _)| *id == gid)
                    .map(|(_, r)| r)
                    .unwrap_or(bounds);
                let content_rect = self.content_rect_for_group(gid, rect);
                self.route_event_to_group_content(gid, event, content_rect)
            }
            InputEvent::ImePreedit { .. } => EventResult::Ignored,
            InputEvent::ImeCommit(text) => {
                if text.is_empty() {
                    return EventResult::Ignored;
                }
                let gid = self.model.active_group_id();
                let rect = self
                    .model
                    .compute_group_rects(bounds)
                    .into_iter()
                    .find(|(id, _)| *id == gid)
                    .map(|(_, r)| r)
                    .unwrap_or(bounds);
                let content_rect = self.content_rect_for_group(gid, rect);
                let res = if let Some(editor) = self.active_editor_mut() {
                    editor.handle_event(event, content_rect)
                } else {
                    EventResult::Ignored
                };
                if res == EventResult::Consumed {
                    self.sync_dirty_states();
                    if let Some(uri) = self
                        .model
                        .current_group_ref()
                        .active_tab()
                        .map(|t| t.uri.clone())
                    {
                        self.for_each_editor_for_uri_mut(&uri, |ed| {
                            ed.recompute_folding();
                        });
                    }
                    self.sync_breadcrumbs(None);
                }
                res
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::render::MouseEvent;

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

    #[test]
    fn test_editor_area_view_tabs_and_rendering() {
        let mut area_view = EditorAreaView::default();
        area_view.open_file(
            "file:///src/main.rs",
            "main.rs",
            "fn main() {\n    println!(\"Rust\");\n}\n",
            false,
        );
        area_view.open_file(
            "file:///src/lib.rs",
            "lib.rs",
            "pub fn hello() {}\n",
            true,
        );

        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let mut scene = Scene::new();
        area_view.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("main.rs"));
        assert!(scene.contains_text("lib.rs"));
        assert!(scene.contains_text("pub fn hello()"));


        // Click first tab to switch back to main.rs

        let click_ev = InputEvent::MouseDown(MouseEvent::new(
            Point::new(30.0, 15.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = area_view.handle_event(&click_ev, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(
            area_view.model.current_group_ref().active_tab().unwrap().title,
            "main.rs"
        );
    }

    #[test]
    fn test_editor_area_save_active_file() {
        let tmp_dir = std::env::temp_dir().join(format!("rustcode_area_save_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let path = tmp_dir.join("saved_app.rs");
        let uri = format!("file://{}", path.display());

        let fs = FileSystemService::new();
        let mut area_view = EditorAreaView::default();
        area_view.open_file(&uri, "saved_app.rs", "fn initial() {}", false);

        // Edit active editor
        area_view.active_editor_mut().unwrap().insert_text(" ");
        assert!(area_view.active_editor().unwrap().is_dirty());

        // Save active file
        let meta = area_view.save_active_file(&fs).unwrap();
        assert_eq!(meta.path, path);
        assert!(!area_view.active_editor().unwrap().is_dirty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), " fn initial() {}");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_editor_area_apply_text_edits() {
        let mut area_view = EditorAreaView::default();
        area_view.open_file(
            "file:///src/calc.rs",
            "calc.rs",
            "fn add(a: i32, b: i32) -> i32 {\na + b\n}\n",
            false,
        );

        let edits = vec![RangeEdit::new(
            Range::new(Position::new(1, 0), Position::new(1, 5)),
            "    a + b",
        )];

        let ok = area_view.apply_text_edits("file:///src/calc.rs", &edits);
        assert!(ok);
        assert_eq!(
            area_view.editors.get("file:///src/calc.rs").unwrap().text(),
            "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n"
        );
        assert!(area_view.model.current_group_ref().tabs[0].is_dirty);
    }

    #[test]
    fn test_editor_area_apply_workspace_edit() {
        use rustcode_protocols::lsp::{LspPosition, LspRange, LspTextEdit};
        use std::collections::HashMap;

        let mut area_view = EditorAreaView::default();
        area_view.open_file(
            "file:///src/file1.rs",
            "file1.rs",
            "let x = 1;\n",
            false,
        );

        let mut changes = HashMap::new();
        changes.insert(
            "file:///src/file1.rs".to_string(),
            vec![LspTextEdit::new(
                LspRange::new(LspPosition::new(0, 8), LspPosition::new(0, 9)),
                "100",
            )],
        );

        let ws_edit = WorkspaceEdit::with_changes(changes);
        let applied = area_view.apply_workspace_edit(&ws_edit);
        assert_eq!(applied, 1);
        assert_eq!(
            area_view.editors.get("file:///src/file1.rs").unwrap().text(),
            "let x = 100;\n"
        );
        assert!(area_view.model.current_group_ref().tabs[0].is_dirty);
    }

    #[test]
    fn test_editor_area_apply_workspace_edit_multibyte_unicode() {
        use rustcode_protocols::lsp::{LspPosition, LspRange, LspTextEdit};
        use std::collections::HashMap;

        let mut area_view = EditorAreaView::default();
        area_view.open_file(
            "file:///src/unicode.rs",
            "unicode.rs",
            "let s = \"🦀abc\";\n",
            false,
        );

        let mut changes = HashMap::new();
        // Replace "abc" at UTF-16 col 11..14 with "xyz"
        changes.insert(
            "file:///src/unicode.rs".to_string(),
            vec![LspTextEdit::new(
                LspRange::new(LspPosition::new(0, 11), LspPosition::new(0, 14)),
                "xyz",
            )],
        );

        let ws_edit = WorkspaceEdit::with_changes(changes);
        let applied = area_view.apply_workspace_edit(&ws_edit);
        assert_eq!(applied, 1);
        assert_eq!(
            area_view.editors.get("file:///src/unicode.rs").unwrap().text(),
            "let s = \"🦀xyz\";\n"
        );
        assert!(area_view.model.current_group_ref().tabs[0].is_dirty);
    }

    #[test]
    fn test_editor_area_open_settings_render_events_and_close() {
        use crate::settings_ui::{SettingsFocus, SettingsHitTarget};
        use rustcode_platform::configuration::ConfigurationService;
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{KeyCode, KeyEvent, MouseButton, ScrollEvent};

        let config = ConfigurationService::new();
        let mut area_view = EditorAreaView::default();

        area_view.open_settings(Some(config.clone()));
        assert_eq!(area_view.model.current_group_ref().tabs.len(), 1);
        let tab = area_view.model.current_group_ref().active_tab().unwrap();
        assert_eq!(tab.uri, "rustcode://settings");
        assert_eq!(tab.title, "Settings");
        assert!(!tab.is_preview);
        assert!(area_view.settings_views.contains_key("rustcode://settings"));

        // Re-opening does not create a duplicate tab
        area_view.open_settings(Some(config.clone()));
        assert_eq!(area_view.model.current_group_ref().tabs.len(), 1);

        let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        area_view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("Settings"));
        assert!(scene.contains_text("Search settings..."));
        assert!(scene.contains_text("Font Size"));

        // Click SearchBar inside settings view
        let search_rect = area_view
            .settings_views
            .get("rustcode://settings")
            .unwrap()
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::SearchBar(r) => Some(*r),
                _ => None,
            })
            .unwrap();

        let res = area_view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(search_rect.min_x() + 5.0, search_rect.min_y() + 5.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(
            area_view
                .settings_views
                .get("rustcode://settings")
                .unwrap()
                .focus,
            SettingsFocus::Search
        );

        // Type into SearchBar via EditorAreaView::handle_event
        for ch in "minimap".chars() {
            let res = area_view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
            assert_eq!(res, EventResult::Consumed);
        }
        assert_eq!(
            area_view
                .settings_views
                .get("rustcode://settings")
                .unwrap()
                .state
                .search_query,
            "minimap"
        );

        // ScrollWheel routes to settings view
        let scroll_res = area_view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(400.0, 200.0), 0.0, -40.0)),
            bounds,
        );
        assert_eq!(scroll_res, EventResult::Consumed);

        // Closing tab cleans up settings_views
        assert!(area_view.close_tab(0));
        assert!(area_view.model.current_group_ref().tabs.is_empty());
        assert!(!area_view.settings_views.contains_key("rustcode://settings"));
    }

    #[test]
    fn test_editor_area_notebook_routing_render_save_and_close() {
        use crate::notebook::NotebookClickTarget;
        use rustcode_platform::render::MouseButton;

        let tmp_dir = std::env::temp_dir().join(format!(
            "rustcode_nb_area_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let nb_path = tmp_dir.join("analysis.IPYNB");
        let nb_uri = format!("file://{}", nb_path.display());

        let initial_ipynb = r#"{
            "cells": [
                {
                    "id": "c1",
                    "cell_type": "code",
                    "execution_count": null,
                    "metadata": {},
                    "outputs": [],
                    "source": ["a = 7\n", "a * 6"]
                }
            ],
            "metadata": { "language_info": { "name": "python" } },
            "nbformat": 4,
            "nbformat_minor": 5
        }"#;

        let mut area_view = EditorAreaView::default();
        area_view.open_file(&nb_uri, "analysis.IPYNB", initial_ipynb, false);

        // Routed to notebooks, NOT editors
        assert!(area_view.notebooks.contains_key(&nb_uri));
        assert!(!area_view.editors.contains_key(&nb_uri));
        assert!(area_view.active_notebook().is_some());

        let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        area_view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("analysis.IPYNB"));
        assert!(scene.contains_text("a = 7"));

        // Click Run cell 0 -> becomes dirty and renders ● indicator
        let run_rect = area_view
            .active_notebook()
            .unwrap()
            .hit_targets
            .read()
            .iter()
            .find_map(|(r, t)| match t {
                NotebookClickTarget::RunCell(0) => Some(*r),
                _ => None,
            })
            .unwrap();
        let res = area_view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(run_rect.min_x() + 4.0, run_rect.min_y() + 4.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(res, EventResult::Consumed);
        assert!(area_view.active_notebook().unwrap().document.is_dirty);
        assert!(area_view.model.current_group_ref().tabs[0].is_dirty);

        let mut dirty_scene = Scene::new();
        area_view.render(bounds, &theme, &font, &mut dirty_scene);
        assert!(dirty_scene.contains_text("analysis.IPYNB \u{25cf}"));
        assert!(dirty_scene.contains_text("42"));

        // Save active file writes valid .ipynb JSON and clears dirty flag
        let fs = FileSystemService::new();
        let meta = area_view.save_active_file(&fs).unwrap();
        assert_eq!(meta.path, nb_path);
        assert!(!area_view.active_notebook().unwrap().document.is_dirty);
        assert!(!area_view.model.current_group_ref().tabs[0].is_dirty);

        let saved_text = std::fs::read_to_string(&nb_path).unwrap();
        let saved_doc =
            crate::notebook::NotebookDocument::from_ipynb_json(&nb_uri, &saved_text).unwrap();
        assert_eq!(saved_doc.cells[0].execution_count, Some(1));
        assert_eq!(saved_doc.cells[0].outputs[0].plain_text(), "42");

        // Closing tab removes from notebooks map
        assert!(area_view.close_tab(0));
        assert!(!area_view.notebooks.contains_key(&nb_uri));

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_editor_area_breadcrumbs_layout_click_jump_and_minimap_propagation() {
        use rustcode_platform::render::MouseButton;

        let mut area_view = EditorAreaView::new(EditorArea::new());
        let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 600.0);

        // Empty editor area has no breadcrumbs rect -> content_rect starts at +35.0
        assert!(area_view.breadcrumbs_rect(bounds).is_none());
        assert_eq!(area_view.content_rect(bounds).min_y(), 35.0);

        // Disable minimap before opening file -> newly opened editor inherits minimap_enabled = false
        area_view.set_minimap_enabled(false);
        let ws = Workspace::from_folder(PathBuf::from("/demo"));
        let src = "pub struct Engine {\n    x: usize,\n}\n\nimpl Engine {\n    pub fn run(&mut self) {\n        let y = 1;\n    }\n}\n";
        area_view.open_file("file:///demo/src/lib.rs", "lib.rs", src, false);
        area_view.sync_breadcrumbs(Some(&ws));

        assert!(!area_view.active_editor().unwrap().minimap_enabled);
        let bc_rect = area_view.breadcrumbs_rect(bounds).unwrap();
        assert_eq!(bc_rect.min_y(), 35.0);
        assert_eq!(bc_rect.height(), 22.0);
        assert_eq!(area_view.content_rect(bounds).min_y(), 57.0);

        // Move cursor into `run` body (line 6) and sync breadcrumbs
        area_view
            .active_editor_mut()
            .unwrap()
            .set_primary_position(Position::new(6, 8));
        area_view.sync_breadcrumbs(Some(&ws));

        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        area_view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("demo"));
        assert!(scene.contains_text("Engine"));
        assert!(scene.contains_text("run"));

        // Click `Engine` breadcrumb item -> jumps cursor to line 0 and emits BreadcrumbSelected
        let item_rects = area_view.breadcrumbs.compute_item_rects(bc_rect, &font);
        let (engine_idx, engine_rect) = item_rects
            .into_iter()
            .find(|(idx, _)| area_view.breadcrumbs.items[*idx].label == "Engine")
            .unwrap();
        assert_eq!(area_view.breadcrumbs.items[engine_idx].line, Some(0));

        let click = InputEvent::MouseDown(MouseEvent::new(
            Point::new(engine_rect.min_x() + 4.0, engine_rect.min_y() + 10.0),
            MouseButton::Left,
        ));
        assert_eq!(area_view.handle_event(&click, bounds), EventResult::Consumed);
        assert_eq!(
            area_view.active_editor().unwrap().primary_position().line,
            0
        );
        match area_view.last_action.as_ref().unwrap() {
            EditorAreaAction::BreadcrumbSelected(item) => assert_eq!(item.label, "Engine"),
            other => panic!("Unexpected action: {:?}", other),
        }

        // Disabling breadcrumbs restores content_rect to +35.0
        area_view.set_breadcrumbs_enabled(false);
        assert!(area_view.breadcrumbs_rect(bounds).is_none());
        assert_eq!(area_view.content_rect(bounds).min_y(), 35.0);
    }

    #[test]
    fn test_editor_area_breadcrumb_click_after_update_and_bounds_shift_without_render() {
        use rustcode_platform::render::MouseButton;

        let mut area_view = EditorAreaView::new(EditorArea::new());
        let ws = Workspace::from_folder(PathBuf::from("/demo"));
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        // Render initial tab with sidebar open (x = 298.0)
        let bounds_open = Rect::from_xywh(298.0, 0.0, 700.0, 600.0);
        area_view.open_file("file:///demo/a.rs", "a.rs", "pub fn first() {}\n", false);
        area_view.sync_breadcrumbs(Some(&ws));
        area_view.render(bounds_open, &theme, &font, &mut Scene::new());

        // Switch to another file with symbol at line 0 and move cursor to line 3 WITHOUT render()
        let src_b = "pub fn target_sym() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n";
        area_view.open_file("file:///demo/sub/longer_file.rs", "longer_file.rs", src_b, false);
        area_view
            .active_editor_mut()
            .unwrap()
            .set_primary_position(Position::new(3, 4));
        area_view.sync_breadcrumbs(Some(&ws));

        // Also shift bounds (simulating sidebar closed: x = 48.0) WITHOUT render()
        let bounds_closed = Rect::from_xywh(48.0, 0.0, 950.0, 600.0);
        let bc_rect = area_view.breadcrumbs_rect(bounds_closed).unwrap();
        let item_rects = area_view.breadcrumbs.compute_item_rects(bc_rect, &font);
        let (_, sym_rect) = item_rects
            .into_iter()
            .find(|(idx, _)| area_view.breadcrumbs.items[*idx].label == "target_sym")
            .unwrap();

        let click = InputEvent::MouseDown(MouseEvent::new(
            Point::new(sym_rect.min_x() + 4.0, sym_rect.min_y() + 10.0),
            MouseButton::Left,
        ));
        assert_eq!(area_view.handle_event(&click, bounds_closed), EventResult::Consumed);
        assert_eq!(area_view.active_editor().unwrap().primary_position().line, 0);
        assert_eq!(
            area_view.last_action.as_ref().map(|a| matches!(a, EditorAreaAction::BreadcrumbSelected(item) if item.label == "target_sym")),
            Some(true)
        );
    }

    #[test]
    fn test_editor_area_render_non_finite_bounds_emits_nothing() {
        use rustcode_platform::render::Size;

        let mut area_view = EditorAreaView::new(EditorArea::new());
        area_view.open_file("file:///demo/a.rs", "a.rs", "fn main() {}", false);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for bad_bounds in [
            Rect::new(Point::zero(), Size::new(f32::NAN, 600.0)),
            Rect::new(Point::zero(), Size::new(800.0, f32::NAN)),
            Rect::from_xywh(f32::INFINITY, 0.0, 800.0, 600.0),
        ] {
            let mut scene = Scene::new();
            area_view.render(bad_bounds, &theme, &font, &mut scene);
            assert!(scene.is_empty());
            assert!(area_view.breadcrumbs_rect(bad_bounds).is_none());
        }
    }

    #[test]
    fn test_editor_area_word_wrap_propagation() {
        let mut area_view = EditorAreaView::new(EditorArea::new());
        area_view.open_file("file:///demo/first.rs", "first.rs", "let x = 1;", false);
        area_view.set_word_wrap(WordWrapMode::Bounded);
        area_view.set_word_wrap_column(64);

        assert_eq!(
            area_view.editors["file:///demo/first.rs"].word_wrap,
            WordWrapMode::Bounded
        );
        assert_eq!(
            area_view.editors["file:///demo/first.rs"].word_wrap_column,
            64
        );

        // Newly opened files inherit current word_wrap_mode and word_wrap_column
        area_view.open_file("file:///demo/second.rs", "second.rs", "let y = 2;", false);
        assert_eq!(
            area_view.editors["file:///demo/second.rs"].word_wrap,
            WordWrapMode::Bounded
        );
        assert_eq!(
            area_view.editors["file:///demo/second.rs"].word_wrap_column,
            64
        );
    }

    #[test]
    fn test_split_editor_grid_horizontal_vertical_sash_drag_and_collapse() {
        use rustcode_platform::render::{MouseButton, MouseMoveEvent, ScrollEvent};

        let mut area_view = EditorAreaView::default();
        area_view.open_file(
            "file:///demo/shared.rs",
            "shared.rs",
            "fn one() {}\nfn two() {}\nfn three() {}\n",
            false,
        );

        let bounds = Rect::from_xywh(0.0, 0.0, 1000.0, 600.0);

        // 1. Horizontal split (side-by-side)
        let gid1 = area_view.split_active_group(SplitDirection::Horizontal);
        assert_eq!(area_view.model.groups.len(), 2);
        assert_eq!(area_view.model.active_group_id(), gid1);

        let rects = area_view.model.compute_group_rects(bounds);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].0, 0);
        assert_eq!(rects[1].0, gid1);
        assert!(rects[1].1.min_x() >= rects[0].1.max_x() + SASH_THICKNESS - 0.1);

        // Shared TextDocument across group 0 and group 1 with isolated cursor & scroll
        area_view
            .editor_for_group_mut(gid1, "file:///demo/shared.rs")
            .unwrap()
            .set_primary_position(Position::new(2, 0));
        area_view
            .editor_for_group_mut(gid1, "file:///demo/shared.rs")
            .unwrap()
            .insert_text("// split\n");

        assert_eq!(
            area_view.editor_for_group(0, "file:///demo/shared.rs").unwrap().text(),
            area_view.editor_for_group(gid1, "file:///demo/shared.rs").unwrap().text()
        );
        assert_eq!(
            area_view.editor_for_group(0, "file:///demo/shared.rs").unwrap().primary_position().line,
            0
        );
        assert_eq!(
            area_view.editor_for_group(gid1, "file:///demo/shared.rs").unwrap().primary_position().line,
            3
        );

        // Scroll group 0 without changing active group focus
        let scroll_res = area_view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(100.0, 120.0), 0.0, -30.0)),
            bounds,
        );
        assert_eq!(scroll_res, EventResult::Consumed);
        assert_eq!(area_view.model.active_group_id(), gid1);

        // 2. Drag sash to resize horizontal split
        let sashes = area_view.model.compute_sashes(bounds);
        assert_eq!(sashes.len(), 1);
        let sash_center = Point::new(
            sashes[0].rect.min_x() + sashes[0].rect.width() * 0.5,
            sashes[0].rect.min_y() + 50.0,
        );
        assert_eq!(
            area_view.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(sash_center, MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        assert_eq!(
            area_view.handle_event(
                &InputEvent::MouseMove(MouseMoveEvent {
                    position: Point::new(700.0, 50.0)
                }),
                bounds
            ),
            EventResult::Consumed
        );
        assert_eq!(
            area_view.handle_event(
                &InputEvent::MouseUp(MouseEvent::new(Point::new(700.0, 50.0), MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        let resized = area_view.model.compute_group_rects(bounds);
        assert!(resized[0].1.width() > resized[1].1.width());

        // 3. Vertical split on active group -> 3 groups
        let gid2 = area_view.split_active_group(SplitDirection::Vertical);
        assert_eq!(area_view.model.groups.len(), 3);
        let rects3 = area_view.model.compute_group_rects(bounds);
        assert_eq!(rects3.len(), 3);
        let r1 = rects3.iter().find(|(id, _)| *id == gid1).unwrap().1;
        let r2 = rects3.iter().find(|(id, _)| *id == gid2).unwrap().1;
        assert!(r2.min_y() > r1.max_y());

        // 4. Close last tab in gid2 -> collapses split back to 2 groups
        assert!(area_view.close_tab_in_group(gid2, 0));
        assert_eq!(area_view.model.groups.len(), 2);
        assert!(area_view.model.group_by_id(gid2).is_none());

        // 5. Close primary group 0's tab -> promotes gid1 into `editors`
        assert!(area_view.close_tab_in_group(0, 0));
        assert_eq!(area_view.model.groups.len(), 1);
        assert!(area_view.editors.contains_key("file:///demo/shared.rs"));
    }

    #[test]
    fn test_custom_editor_selector_matching_and_webview_dispatch() {
        use rustcode_ext_host::manifest::CustomEditorSelector;

        let mut area_view = EditorAreaView::default();
        area_view.register_custom_editor_contributions(vec![CustomEditorContribution {
            view_type: "catEdit.catScratch".to_string(),
            display_name: "Cat Scratch".to_string(),
            selector: vec![CustomEditorSelector {
                filename_pattern: "*.cscratch".to_string(),
            }],
            priority: Some("default".to_string()),
        }]);

        area_view.open_file(
            "file:///workspace/sketch.cscratch",
            "sketch.cscratch",
            "ignored raw",
            false,
        );
        assert!(area_view.webviews.contains_key("file:///workspace/sketch.cscratch"));
        assert!(!area_view.editors.contains_key("file:///workspace/sketch.cscratch"));

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 500.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        area_view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("sketch.cscratch"));
        assert!(scene.contains_text("Custom Editor: Cat Scratch"));
    }

    #[test]
    fn test_nested_sash_drag_horizontal_vertical_and_deep() {
        use rustcode_platform::render::{MouseButton, MouseMoveEvent};

        let bounds = Rect::from_xywh(0.0, 0.0, 1000.0, 600.0);

        // 1. 2-level Horizontal -> Horizontal nested sash drag via EditorAreaView::handle_event
        let mut h_view = EditorAreaView::default();
        h_view.open_file("file:///a.rs", "a.rs", "let a = 1;\n", false);
        h_view.split_active_group(SplitDirection::Horizontal);
        h_view.split_active_group(SplitDirection::Horizontal);

        let sashes_h = h_view.model.compute_sashes(bounds);
        let nested_h = sashes_h
            .iter()
            .find(|s| s.split_path == vec![true])
            .unwrap()
            .clone();
        let h_start = Point::new(
            nested_h.rect.min_x() + nested_h.rect.width() * 0.5,
            nested_h.rect.min_y() + 50.0,
        );
        let h_target = Point::new(
            nested_h.parent_bounds.min_x() + nested_h.parent_bounds.width() * 0.25,
            100.0,
        );
        assert_eq!(
            h_view.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(h_start, MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        assert_eq!(
            h_view.handle_event(
                &InputEvent::MouseMove(MouseMoveEvent { position: h_target }),
                bounds
            ),
            EventResult::Consumed
        );
        assert_eq!(
            h_view.handle_event(
                &InputEvent::MouseUp(MouseEvent::new(h_target, MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        let moved_h = h_view
            .model
            .compute_sashes(bounds)
            .into_iter()
            .find(|s| s.split_path == vec![true])
            .unwrap();
        let moved_h_center_x = moved_h.rect.min_x() + moved_h.rect.width() * 0.5;
        assert!(
            (moved_h_center_x - h_target.x).abs() <= 4.0,
            "H->H sash center {} should be within 4px of {}",
            moved_h_center_x,
            h_target.x
        );

        // 2. 2-level Vertical -> Vertical nested sash drag via EditorAreaView::handle_event
        let mut v_view = EditorAreaView::default();
        v_view.open_file("file:///b.rs", "b.rs", "let b = 2;\n", false);
        v_view.split_active_group(SplitDirection::Vertical);
        v_view.split_active_group(SplitDirection::Vertical);

        let sashes_v = v_view.model.compute_sashes(bounds);
        let nested_v = sashes_v
            .iter()
            .find(|s| s.split_path == vec![true])
            .unwrap()
            .clone();
        let v_start = Point::new(
            nested_v.rect.min_x() + 50.0,
            nested_v.rect.min_y() + nested_v.rect.height() * 0.5,
        );
        let v_target = Point::new(
            100.0,
            nested_v.parent_bounds.min_y() + nested_v.parent_bounds.height() * 0.25,
        );
        assert_eq!(
            v_view.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(v_start, MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        assert_eq!(
            v_view.handle_event(
                &InputEvent::MouseMove(MouseMoveEvent { position: v_target }),
                bounds
            ),
            EventResult::Consumed
        );
        assert_eq!(
            v_view.handle_event(
                &InputEvent::MouseUp(MouseEvent::new(v_target, MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        let moved_v = v_view
            .model
            .compute_sashes(bounds)
            .into_iter()
            .find(|s| s.split_path == vec![true])
            .unwrap();
        let moved_v_center_y = moved_v.rect.min_y() + moved_v.rect.height() * 0.5;
        assert!(
            (moved_v_center_y - v_target.y).abs() <= 4.0,
            "V->V sash center {} should be within 4px of {}",
            moved_v_center_y,
            v_target.y
        );

        // 3. Deeper [true, true] nested sash drag via EditorArea::update_sash_ratio & handle_event
        h_view.split_active_group(SplitDirection::Horizontal);
        let deep_sash = h_view
            .model
            .compute_sashes(bounds)
            .into_iter()
            .find(|s| s.split_path == vec![true, true])
            .unwrap();
        let deep_target = Point::new(
            deep_sash.parent_bounds.min_x() + deep_sash.parent_bounds.width() * 0.7,
            100.0,
        );
        assert!(h_view.model.update_sash_ratio(
            &deep_sash.split_path,
            deep_target,
            deep_sash.parent_bounds
        ));
        let moved_deep = h_view
            .model
            .compute_sashes(bounds)
            .into_iter()
            .find(|s| s.split_path == vec![true, true])
            .unwrap();
        let moved_deep_center_x = moved_deep.rect.min_x() + moved_deep.rect.width() * 0.5;
        assert!(
            (moved_deep_center_x - deep_target.x).abs() <= 4.0,
            "[true, true] sash center {} should be within 4px of {}",
            moved_deep_center_x,
            deep_target.x
        );
    }

    #[test]
    fn test_preview_keydown_edit_prevents_overwrite_and_cleans_replaced_preview() {
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{KeyCode, KeyEvent};

        let mut area_view = EditorAreaView::default();
        let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 600.0);

        // Unedited preview p0 replaced by unedited preview p1 -> p0 cleaned up from editors
        area_view.open_file("file:///p0.rs", "p0.rs", "let p0 = 0;\n", true);
        assert!(area_view.editors.contains_key("file:///p0.rs"));
        area_view.open_file("file:///p1.rs", "p1.rs", "let p1 = 1;\n", true);
        assert!(!area_view.editors.contains_key("file:///p0.rs"));
        assert_eq!(area_view.model.current_group_ref().tabs.len(), 1);
        assert!(area_view.model.current_group_ref().tabs[0].is_preview);

        // Edit p1 via KeyDown -> becomes dirty and non-preview
        let res = area_view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('X'), Modifiers::none())),
            bounds,
        );
        assert_eq!(res, EventResult::Consumed);
        let p1_tab = &area_view.model.current_group_ref().tabs[0];
        assert!(p1_tab.is_dirty);
        assert!(!p1_tab.is_preview);

        // Opening another preview file p2 must preserve p1
        area_view.open_file("file:///p2.rs", "p2.rs", "let p2 = 2;\n", true);
        assert_eq!(area_view.model.current_group_ref().tabs.len(), 2);
        assert_eq!(
            area_view.model.current_group_ref().tabs[0].uri,
            "file:///p1.rs"
        );
        assert!(area_view.model.current_group_ref().tabs[0].is_dirty);
        assert!(!area_view.model.current_group_ref().tabs[0].is_preview);
        assert_eq!(
            area_view.model.current_group_ref().tabs[1].uri,
            "file:///p2.rs"
        );
    }

    #[test]
    fn test_same_file_split_keydown_syncs_dirty_tabs_and_sibling_folding() {
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{KeyCode, KeyEvent};

        let mut area_view = EditorAreaView::default();
        let bounds = Rect::from_xywh(0.0, 0.0, 1000.0, 600.0);
        let uri = "file:///demo/fold_sync.rs";
        area_view.open_file(
            uri,
            "fold_sync.rs",
            "fn one() {\n    let a = 1;\n}\n",
            false,
        );

        let gid1 = area_view.split_active_group(SplitDirection::Horizontal);
        let fold_count_before = area_view
            .editor_for_group(0, uri)
            .unwrap()
            .folding_model
            .ranges()
            .len();
        assert_eq!(fold_count_before, 1);

        // Position cursor at end of file in group 1 and type a second foldable function via KeyDown
        area_view
            .editor_for_group_mut(gid1, uri)
            .unwrap()
            .set_primary_position(Position::new(3, 0));
        for ch in "fn two() {\n    let b = 2;\n}\n".chars() {
            let key = if ch == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(ch)
            };
            let res = area_view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(key, Modifiers::none())),
                bounds,
            );
            assert_eq!(res, EventResult::Consumed);
        }

        // All groups must mark tab dirty
        for g in &area_view.model.groups {
            let tab = g.tabs.iter().find(|t| t.uri == uri).unwrap();
            assert!(tab.is_dirty, "group {} tab must be dirty", g.id);
            assert!(!tab.is_preview);
        }

        // Sibling group 0 folding ranges must match group 1
        let ranges0 = area_view
            .editor_for_group(0, uri)
            .unwrap()
            .folding_model
            .ranges()
            .to_vec();
        let ranges1 = area_view
            .editor_for_group(gid1, uri)
            .unwrap()
            .folding_model
            .ranges()
            .to_vec();
        assert_eq!(ranges1.len(), 2);
        assert_eq!(ranges0, ranges1);
    }

    #[test]
    fn test_closing_split_mid_sash_drag_clears_active_sash_drag() {
        use rustcode_platform::render::{MouseButton, MouseMoveEvent};

        let mut area_view = EditorAreaView::default();
        area_view.open_file("file:///drag.rs", "drag.rs", "let x = 1;\n", false);
        let gid1 = area_view.split_active_group(SplitDirection::Horizontal);
        let bounds = Rect::from_xywh(0.0, 0.0, 1000.0, 600.0);

        let sash = area_view.model.compute_sashes(bounds).pop().unwrap();
        let sash_pt = Point::new(
            sash.rect.min_x() + sash.rect.width() * 0.5,
            sash.rect.min_y() + 40.0,
        );
        assert_eq!(
            area_view.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(sash_pt, MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        assert!(area_view.active_sash_drag.is_some());

        // Collapse via close_tab_in_group
        assert!(area_view.close_tab_in_group(gid1, 0));
        assert!(area_view.active_sash_drag.is_none());
        assert_eq!(
            area_view.handle_event(
                &InputEvent::MouseMove(MouseMoveEvent {
                    position: Point::new(300.0, 40.0)
                }),
                bounds
            ),
            EventResult::Ignored
        );

        // Repeat and collapse via close_editors_in_active_group
        let _gid2 = area_view.split_active_group(SplitDirection::Horizontal);
        let sash2 = area_view.model.compute_sashes(bounds).pop().unwrap();
        let sash_pt2 = Point::new(
            sash2.rect.min_x() + sash2.rect.width() * 0.5,
            sash2.rect.min_y() + 40.0,
        );
        assert_eq!(
            area_view.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(sash_pt2, MouseButton::Left)),
                bounds
            ),
            EventResult::Consumed
        );
        assert!(area_view.active_sash_drag.is_some());
        assert!(area_view.close_editors_in_active_group());
        assert!(area_view.active_sash_drag.is_none());
        assert_eq!(
            area_view.handle_event(
                &InputEvent::MouseMove(MouseMoveEvent {
                    position: Point::new(300.0, 40.0)
                }),
                bounds
            ),
            EventResult::Ignored
        );
    }

    #[test]
    fn test_split_custom_editor_independent_button_hit_testing() {
        use rustcode_platform::render::MouseButton;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let wv_mgr = ExtHostWebviewManager::new();
        wv_mgr.create_panel(
            "ce_split_1",
            "catEdit.catScratch",
            "split.cscratch",
            WebviewOptions::default(),
        );
        let html = "<h1>Split Cat</h1><button id=\"feed\">Feed</button>";
        wv_mgr.set_html("ce_split_1", html);

        let msgs_received = Arc::new(AtomicUsize::new(0));
        let msgs_clone = msgs_received.clone();
        wv_mgr.on_did_receive_message("ce_split_1", move |msg| {
            if msg["id"] == "feed" {
                msgs_clone.fetch_add(1, Ordering::SeqCst);
            }
        });

        let mut area_view = EditorAreaView::default();
        area_view.open_custom_editor(
            "file:///w/split.cscratch",
            "split.cscratch",
            "catEdit.catScratch",
            "ce_split_1",
            html,
            WebviewOptions::default(),
            Some(wv_mgr.clone()),
            false,
        );
        let gid1 = area_view.split_active_group(SplitDirection::Horizontal);
        assert!(area_view
            .webview_for_group(0, "file:///w/split.cscratch")
            .is_some());
        assert!(area_view
            .webview_for_group(gid1, "file:///w/split.cscratch")
            .is_some());

        let bounds = Rect::from_xywh(0.0, 0.0, 1000.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        // Renders Group 0, then Group 1
        area_view.render(bounds, &theme, &font, &mut scene);

        let g0_btn = area_view
            .webview_for_group(0, "file:///w/split.cscratch")
            .unwrap()
            .button_targets
            .read()[0]
            .0;
        let g1_btn = area_view
            .webview_for_group(gid1, "file:///w/split.cscratch")
            .unwrap()
            .button_targets
            .read()[0]
            .0;
        assert!(g0_btn.max_x() < g1_btn.min_x());

        // Clicking Group 0's button must still hit Group 0's WebviewEditorView even after Group 1 rendered
        let click_g0 = InputEvent::MouseDown(MouseEvent::new(
            Point::new(g0_btn.min_x() + 5.0, g0_btn.min_y() + 5.0),
            MouseButton::Left,
        ));
        assert_eq!(area_view.handle_event(&click_g0, bounds), EventResult::Consumed);
        assert_eq!(msgs_received.load(Ordering::SeqCst), 1);

        // Clicking Group 1's button also works
        let click_g1 = InputEvent::MouseDown(MouseEvent::new(
            Point::new(g1_btn.min_x() + 5.0, g1_btn.min_y() + 5.0),
            MouseButton::Left,
        ));
        assert_eq!(area_view.handle_event(&click_g1, bounds), EventResult::Consumed);
        assert_eq!(msgs_received.load(Ordering::SeqCst), 2);

        // Closing Group 0 promotes Group 1's webview and keeps panel alive
        assert!(area_view.close_tab_in_group(0, 0));
        assert!(!wv_mgr.get_panel("ce_split_1").unwrap().is_disposed);
        assert!(area_view.webviews.contains_key("file:///w/split.cscratch"));

        // Closing the last group disposes the panel
        assert!(area_view.close_tab_in_group(gid1, 0));
        assert!(wv_mgr.get_panel("ce_split_1").unwrap().is_disposed);
    }

    #[test]
    fn test_editor_area_rename_tab_file_and_directory_descendants() {
        let tmp = std::env::temp_dir().join(format!(
            "rustcode_rename_tab_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let src_dir = tmp.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let old_file = src_dir.join("before.rs");
        let other_file = src_dir.join("nested.rs");
        std::fs::write(&old_file, "fn before() {}\n").unwrap();
        std::fs::write(&other_file, "fn nested() {}\n").unwrap();

        let old_uri = rustcode_platform::path_to_file_uri(&old_file);
        let other_uri = rustcode_platform::path_to_file_uri(&other_file);

        let mut area_view = EditorAreaView::default();
        area_view.open_file(&old_uri, "before.rs", "fn before() {}\n", false);
        area_view.open_file(&other_uri, "nested.rs", "fn nested() {}\n", false);
        let gid1 = area_view.split_active_group(SplitDirection::Horizontal);
        let _ = gid1;

        let new_file = src_dir.join("after.py");
        let new_uri = rustcode_platform::path_to_file_uri(&new_file);
        assert!(area_view.rename_tab(&old_file, &new_file));

        assert!(area_view.editors.contains_key(&new_uri));
        assert!(!area_view.editors.contains_key(&old_uri));
        assert!(area_view.find_open_tab_uri_for_path(&new_file).is_some());
        assert!(area_view.find_open_tab_uri_for_path(&old_file).is_none());
        let ed = area_view.editors.get(&new_uri).unwrap();
        assert_eq!(ed.document.as_ref().unwrap().read().language_id, "python");
        assert!(area_view
            .model
            .groups
            .iter()
            .any(|g| g.tabs.iter().any(|t| t.title == "after.py" && t.uri == new_uri)));

        // Rename parent directory src -> lib updates both open files
        let lib_dir = tmp.join("lib");
        assert!(area_view.rename_tab(&src_dir, &lib_dir));
        assert!(area_view
            .find_open_tab_uri_for_path(&lib_dir.join("after.py"))
            .is_some());
        assert!(area_view
            .find_open_tab_uri_for_path(&lib_dir.join("nested.rs"))
            .is_some());
        assert!(area_view.find_open_tab_uri_for_path(&other_file).is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_editor_area_close_tabs_for_path_file_and_directory() {
        let tmp = std::env::temp_dir().join(format!(
            "rustcode_close_tab_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let pkg_dir = tmp.join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        let f1 = pkg_dir.join("a.rs");
        let f2 = pkg_dir.join("b.rs");
        let f3 = tmp.join("keep.rs");

        let mut area_view = EditorAreaView::default();
        area_view.open_file(&rustcode_platform::path_to_file_uri(&f1), "a.rs", "a", false);
        area_view.open_file(&rustcode_platform::path_to_file_uri(&f2), "b.rs", "b", false);
        area_view.open_file(&rustcode_platform::path_to_file_uri(&f3), "keep.rs", "k", false);

        assert_eq!(area_view.close_tabs_for_path(&f1), 1);
        assert!(area_view.find_open_tab_uri_for_path(&f1).is_none());
        assert!(area_view.find_open_tab_uri_for_path(&f2).is_some());

        // Deleting directory closes remaining descendant tab f2 but leaves f3 open
        assert_eq!(area_view.close_tabs_for_path(&pkg_dir), 1);
        assert!(area_view.find_open_tab_uri_for_path(&f2).is_none());
        assert!(area_view.find_open_tab_uri_for_path(&f3).is_some());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_editor_area_tab_drag_reorder_and_split_drop() {
        use rustcode_platform::render::{MouseButton, MouseEvent, MouseMoveEvent};

        let mut area_view = EditorAreaView::default();
        area_view.open_file("file:///a.rs", "a.rs", "let a = 1;", false);
        area_view.open_file("file:///b.rs", "b.rs", "let b = 2;", false);
        area_view.open_file("file:///c.rs", "c.rs", "let c = 3;", false);

        let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 500.0);
        let font = FontMetrics::default_monospace();
        let rects = area_view.tab_rects_for_group(0, bounds, &font);
        assert_eq!(rects.len(), 3);

        // Drag tab 0 (a.rs) past tab 2 (c.rs) -> reorders tabs within group 0
        let start = Point::new(rects[0].1.min_x() + 15.0, rects[0].1.min_y() + 15.0);
        let target = Point::new(rects[2].1.min_x() + 20.0, rects[2].1.min_y() + 15.0);
        area_view.handle_event(&InputEvent::MouseDown(MouseEvent::new(start, MouseButton::Left)), bounds);
        area_view.handle_event(&InputEvent::MouseMove(MouseMoveEvent { position: target }), bounds);
        area_view.handle_event(&InputEvent::MouseUp(MouseEvent::new(target, MouseButton::Left)), bounds);

        let titles: Vec<&str> = area_view.model.groups[0]
            .tabs
            .iter()
            .map(|t| t.title.as_str())
            .collect();
        assert_eq!(titles, vec!["b.rs", "c.rs", "a.rs"]);

        // Drag active tab (a.rs, index 2) to right 25% edge zone -> splits & moves to new group
        let rects_after = area_view.tab_rects_for_group(0, bounds, &font);
        let a_start = Point::new(rects_after[2].1.min_x() + 15.0, 15.0);
        let right_edge = Point::new(bounds.max_x() - 30.0, 250.0);
        area_view.handle_event(&InputEvent::MouseDown(MouseEvent::new(a_start, MouseButton::Left)), bounds);
        area_view.handle_event(&InputEvent::MouseMove(MouseMoveEvent { position: right_edge }), bounds);
        area_view.handle_event(&InputEvent::MouseUp(MouseEvent::new(right_edge, MouseButton::Left)), bounds);

        assert_eq!(area_view.model.groups.len(), 2);
        assert_eq!(area_view.model.groups[0].tabs.len(), 2);
        assert_eq!(area_view.model.groups[1].tabs.len(), 1);
        assert_eq!(area_view.model.groups[1].tabs[0].uri, "file:///a.rs");
        assert!(area_view.editor_for_group(area_view.model.groups[1].id, "file:///a.rs").is_some());
    }
}

