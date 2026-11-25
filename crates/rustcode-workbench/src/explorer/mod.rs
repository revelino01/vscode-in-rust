//! File tree model, row projection, Explorer CRUD, Contributed TreeViews, and GPUI FileExplorerView.

use parking_lot::RwLock;
use rustcode_base::context_key::ContextKeyService;
use rustcode_ext_host::manifest::ViewContribution;
use rustcode_ext_host::provider::{TreeItem, TreeItemCollapsibleState};
use rustcode_platform::accessibility::{AccessibleNode, AccessibleRole};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, MouseButton, Point, Quad,
    Rect, Scene, View, WorkbenchThemeStyles,
};
use rustcode_protocols::lsp::LspCommand;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct FileTreeNode {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub children: Option<Vec<FileTreeNode>>,
    pub is_expanded: bool,
}

fn node_display_name(path: &Path) -> String {
    let cleaned = rustcode_platform::strip_windows_verbatim_prefix(path.to_path_buf());
    cleaned
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cleaned.to_string_lossy().into_owned())
}

impl FileTreeNode {
    pub fn new_file(path: PathBuf) -> Self {
        let name = node_display_name(&path);
        Self {
            path,
            name,
            is_dir: false,
            children: None,
            is_expanded: false,
        }
    }

    pub fn new_dir(path: PathBuf) -> Self {
        let name = node_display_name(&path);
        Self {
            path,
            name,
            is_dir: true,
            children: Some(Vec::new()),
            is_expanded: false,
        }
    }

    pub fn from_path(path: PathBuf) -> Self {
        Self::from_path_with_depth(path, 0, 15)
    }

    pub fn from_path_with_depth(path: PathBuf, depth: usize, max_depth: usize) -> Self {
        if path.is_dir() {
            let mut node = Self::new_dir(path.clone());
            if depth >= max_depth {
                return node;
            }
            if let Ok(entries) = std::fs::read_dir(&path) {
                let mut children = Vec::new();
                for entry in entries.flatten() {
                    let p = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    if crate::search::engine::is_ignored_directory(&name) {
                        continue;
                    }
                    if p.is_dir() {
                        children.push(Self::from_path_with_depth(p, depth + 1, max_depth));
                    } else {
                        children.push(Self::new_file(p));
                    }
                }
                children.sort_by(|a, b| {
                    if a.is_dir == b.is_dir {
                        a.name.cmp(&b.name)
                    } else if a.is_dir {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    }
                });
                node.children = Some(children);
            }
            node
        } else {
            Self::new_file(path)
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileExplorer {
    pub roots: Vec<FileTreeNode>,
    pub expanded_paths: HashSet<PathBuf>,
    pub selected_path: Option<PathBuf>,
}

impl Default for FileExplorer {
    fn default() -> Self {
        Self::new()
    }
}

impl FileExplorer {
    pub fn new() -> Self {
        Self {
            roots: Vec::new(),
            expanded_paths: HashSet::new(),
            selected_path: None,
        }
    }

    pub fn set_roots(&mut self, roots: Vec<FileTreeNode>) {
        self.roots = roots;
    }

    pub fn add_root(&mut self, root: FileTreeNode) {
        if !self.roots.iter().any(|r| r.path == root.path) {
            self.roots.push(root);
        }
    }

    pub fn remove_root(&mut self, path: &Path) {
        self.roots.retain(|r| r.path != path);
    }

    pub fn sync_from_workspace(&mut self, ws: &crate::workspace::Workspace) {
        let mut new_roots = Vec::new();
        for folder in ws.folders() {
            new_roots.push(FileTreeNode::from_path(folder.path.clone()));
        }
        self.set_roots(new_roots);
    }

    pub fn refresh_from_disk(&mut self) {
        if !self.roots.is_empty() {
            let new_roots: Vec<FileTreeNode> = self
                .roots
                .iter()
                .map(|r| {
                    if r.path.exists() {
                        FileTreeNode::from_path(r.path.clone())
                    } else {
                        r.clone()
                    }
                })
                .collect();
            self.set_roots(new_roots);
        }
    }

    pub fn toggle_expanded(&mut self, path: &PathBuf) {
        if self.expanded_paths.contains(path) {
            self.expanded_paths.remove(path);
        } else {
            self.expanded_paths.insert(path.clone());
        }
    }

    pub fn is_root_path(&self, target: &Path) -> bool {
        self.roots
            .iter()
            .any(|r| rustcode_platform::paths_equivalent(&r.path, target))
    }

    pub fn find_node(&self, target: &Path) -> Option<&FileTreeNode> {
        fn find_in_node<'a>(node: &'a FileTreeNode, target: &Path) -> Option<&'a FileTreeNode> {
            if rustcode_platform::paths_equivalent(&node.path, target) {
                return Some(node);
            }
            if let Some(children) = &node.children {
                for child in children {
                    if let Some(found) = find_in_node(child, target) {
                        return Some(found);
                    }
                }
            }
            None
        }

        for root in &self.roots {
            if let Some(found) = find_in_node(root, target) {
                return Some(found);
            }
        }
        None
    }

    fn update_paths_after_rename(&mut self, old_path: &Path, new_path: &Path) {
        let mut updated_expanded = HashSet::with_capacity(self.expanded_paths.len());
        for p in self.expanded_paths.drain() {
            if rustcode_platform::paths_equivalent(&p, old_path) {
                updated_expanded.insert(new_path.to_path_buf());
            } else if let Some(suffix) = rustcode_platform::path_strip_prefix(&p, old_path) {
                updated_expanded.insert(new_path.join(suffix));
            } else {
                updated_expanded.insert(p);
            }
        }
        self.expanded_paths = updated_expanded;
    }

    fn remove_deleted_paths(&mut self, deleted_path: &Path) {
        self.expanded_paths.retain(|p| {
            !rustcode_platform::paths_equivalent(p, deleted_path)
                && !rustcode_platform::path_starts_with(p, deleted_path)
        });
        if let Some(sel) = &self.selected_path {
            if rustcode_platform::paths_equivalent(sel, deleted_path)
                || rustcode_platform::path_starts_with(sel, deleted_path)
            {
                self.selected_path = deleted_path.parent().map(Path::to_path_buf);
            }
        }
    }
}

/// Cross-platform validation for a single file or folder name entered in Explorer CRUD.
pub fn validate_entry_name(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err("Name cannot be empty");
    }
    if name == "." || name == ".." {
        return Err("Invalid name '.' or '..'");
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err("Name cannot end with a space or dot");
    }
    if name.len() > 255 {
        return Err("Name is too long");
    }
    if name.chars().any(|c| {
        matches!(
            c,
            '/' | '\\' | '\0' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
        ) || c.is_control()
    }) {
        return Err("Name contains invalid characters");
    }

    let stem = name.split('.').next().unwrap_or(name);
    if matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return Err("Reserved device name");
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerInputMode {
    None,
    NewFile {
        target_dir: PathBuf,
        buffer: String,
        error: Option<String>,
    },
    NewFolder {
        target_dir: PathBuf,
        buffer: String,
        error: Option<String>,
    },
    Rename {
        target_path: PathBuf,
        buffer: String,
        select_all: bool,
        error: Option<String>,
    },
    ConfirmDelete {
        target_path: PathBuf,
        is_dir: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplorerRow {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub is_expanded: bool,
    pub is_selected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExplorerAction {
    OpenFile(PathBuf),
    ToggleDir(PathBuf),
    CreateFile(PathBuf),
    CreateFolder(PathBuf),
    Renamed {
        old_path: PathBuf,
        new_path: PathBuf,
    },
    Moved {
        from: PathBuf,
        to: PathBuf,
    },
    Deleted(PathBuf),
    ExpandContributedItem {
        view_id: String,
        element_id: String,
    },
    ExecuteContributedCommand {
        command: String,
        arguments: Vec<Value>,
    },
}

// --- Contributed TreeView Model ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributedViewDescriptor {
    pub id: String,
    pub name: String,
    pub when: Option<String>,
}

impl From<ViewContribution> for ContributedViewDescriptor {
    fn from(v: ViewContribution) -> Self {
        Self {
            id: v.id,
            name: v.name,
            when: v.when,
        }
    }
}

impl From<&ViewContribution> for ContributedViewDescriptor {
    fn from(v: &ViewContribution) -> Self {
        Self {
            id: v.id.clone(),
            name: v.name.clone(),
            when: v.when.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContributedTreeNode {
    pub node_id: String,
    pub rpc_element_id: String,
    pub label: String,
    pub description: Option<String>,
    pub tooltip: Option<String>,
    pub collapsible_state: TreeItemCollapsibleState,
    pub command: Option<LspCommand>,
    pub resource_uri: Option<String>,
}

/// Extract a clean basename from a `resourceUri` (supporting Windows `C:\...`, POSIX `/...`, and `file:///` URIs).
pub fn basename_from_resource_uri(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return String::new();
    }
    let without_qf = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    if let Some(last) = without_qf
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
    {
        return last.replace("%20", " ");
    }
    without_qf.to_string()
}

pub fn tree_item_display_label(item: &TreeItem) -> String {
    if !item.label.trim().is_empty() {
        return item.label.clone();
    }
    if let Some(uri) = &item.resource_uri {
        let base = basename_from_resource_uri(uri);
        if !base.is_empty() {
            return base;
        }
    }
    if let Some(id) = &item.id {
        if !id.trim().is_empty() {
            return id.clone();
        }
    }
    "Item".to_string()
}

/// Safely evaluate a VS Code `when` clause against `ContextKeyService`.
/// Malformed `when` clauses evaluate to `false` without panicking.
pub fn evaluate_when_clause(context_keys: &ContextKeyService, when: Option<&str>) -> bool {
    let Some(raw) = when else {
        return true;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return true;
    }
    context_keys.evaluate_expr(trimmed)
}

#[derive(Debug, Clone)]
pub struct ContributedViewState {
    pub descriptor: ContributedViewDescriptor,
    pub visible: bool,
    pub has_provider: bool,
    pub collapsed: bool,
    pub root_items: Vec<ContributedTreeNode>,
    pub children_by_parent: HashMap<String, Vec<ContributedTreeNode>>,
    pub expanded_ids: HashSet<String>,
    pub user_collapsed_ids: HashSet<String>,
    pub selected_item_id: Option<String>,
    pub root_loaded: bool,
    pub loading_root: bool,
    pub loaded_parents: HashSet<String>,
    pub loading_parents: HashSet<String>,
}

impl ContributedViewState {
    pub fn new(descriptor: ContributedViewDescriptor) -> Self {
        Self {
            descriptor,
            visible: true,
            has_provider: false,
            collapsed: false,
            root_items: Vec::new(),
            children_by_parent: HashMap::new(),
            expanded_ids: HashSet::new(),
            user_collapsed_ids: HashSet::new(),
            selected_item_id: None,
            root_loaded: false,
            loading_root: false,
            loaded_parents: HashSet::new(),
            loading_parents: HashSet::new(),
        }
    }

    fn convert_items(
        view_id: &str,
        parent_key: Option<&str>,
        items: Vec<TreeItem>,
    ) -> Vec<ContributedTreeNode> {
        items
            .into_iter()
            .enumerate()
            .map(|(idx, item)| {
                let label = tree_item_display_label(&item);
                let explicit_id = item.id.clone().filter(|s| !s.trim().is_empty());
                let node_id = explicit_id.clone().unwrap_or_else(|| {
                    format!(
                        "__fallback:{}:{}:{}:{}",
                        view_id,
                        parent_key.unwrap_or("root"),
                        idx,
                        label
                    )
                });
                let rpc_element_id = explicit_id.unwrap_or_else(|| node_id.clone());
                ContributedTreeNode {
                    node_id,
                    rpc_element_id,
                    label,
                    description: item.description,
                    tooltip: item.tooltip,
                    collapsible_state: item.collapsible_state,
                    command: item.command,
                    resource_uri: item.resource_uri,
                }
            })
            .collect()
    }

    pub fn set_root_items(&mut self, items: Vec<TreeItem>) -> Vec<(String, String)> {
        let converted = Self::convert_items(&self.descriptor.id, None, items);
        let mut auto_fetch = Vec::new();
        for node in &converted {
            if node.collapsible_state == TreeItemCollapsibleState::Expanded
                && !self.user_collapsed_ids.contains(&node.node_id)
            {
                self.expanded_ids.insert(node.node_id.clone());
                if !self.loaded_parents.contains(&node.node_id)
                    && !self.loading_parents.contains(&node.node_id)
                {
                    auto_fetch.push((node.node_id.clone(), node.rpc_element_id.clone()));
                }
            }
        }
        self.root_items = converted;
        self.root_loaded = true;
        self.loading_root = false;
        auto_fetch
    }

    pub fn set_child_items(
        &mut self,
        parent_node_id: &str,
        items: Vec<TreeItem>,
    ) -> Vec<(String, String)> {
        let converted = Self::convert_items(&self.descriptor.id, Some(parent_node_id), items);
        let mut auto_fetch = Vec::new();
        for node in &converted {
            if node.collapsible_state == TreeItemCollapsibleState::Expanded
                && !self.user_collapsed_ids.contains(&node.node_id)
            {
                self.expanded_ids.insert(node.node_id.clone());
                if !self.loaded_parents.contains(&node.node_id)
                    && !self.loading_parents.contains(&node.node_id)
                {
                    auto_fetch.push((node.node_id.clone(), node.rpc_element_id.clone()));
                }
            }
        }
        self.children_by_parent
            .insert(parent_node_id.to_string(), converted);
        self.loaded_parents.insert(parent_node_id.to_string());
        self.loading_parents.remove(parent_node_id);
        auto_fetch
    }

    pub fn invalidate_all(&mut self) {
        self.root_loaded = false;
        self.loading_root = false;
        self.children_by_parent.clear();
        self.loaded_parents.clear();
        self.loading_parents.clear();
    }

    pub fn invalidate_elements(&mut self, element_ids: &[String]) {
        if element_ids.is_empty() {
            self.invalidate_all();
            return;
        }
        for eid in element_ids {
            self.loaded_parents.remove(eid);
            self.loading_parents.remove(eid);
            self.children_by_parent.remove(eid);
        }
    }

    pub fn flattened_items(&self) -> Vec<(ContributedTreeNode, usize, bool)> {
        let mut out = Vec::new();
        let mut stack: Vec<(ContributedTreeNode, usize)> = Vec::new();
        let mut visited = HashSet::new();

        for item in self.root_items.iter().rev() {
            stack.push((item.clone(), 0));
        }

        while let Some((node, depth)) = stack.pop() {
            if !visited.insert(node.node_id.clone()) {
                continue;
            }
            let is_expanded = node.collapsible_state != TreeItemCollapsibleState::None
                && self.expanded_ids.contains(&node.node_id);
            out.push((node.clone(), depth, is_expanded));
            if is_expanded {
                if let Some(children) = self.children_by_parent.get(&node.node_id) {
                    for child in children.iter().rev() {
                        stack.push((child.clone(), depth + 1));
                    }
                }
            }
        }

        out
    }
}

#[derive(Debug, Default)]
pub struct ContributedTreesState {
    pub order: Vec<String>,
    pub views: HashMap<String, ContributedViewState>,
}

impl ContributedTreesState {
    pub fn sync_descriptors(
        &mut self,
        descriptors: Vec<ContributedViewDescriptor>,
        context_keys: &ContextKeyService,
        has_provider: impl Fn(&str) -> bool,
    ) {
        let mut new_order = Vec::with_capacity(descriptors.len());
        let mut seen = HashSet::new();
        for desc in descriptors {
            if !seen.insert(desc.id.clone()) {
                continue;
            }
            new_order.push(desc.id.clone());
            let visible = evaluate_when_clause(context_keys, desc.when.as_deref());
            let provider_ok = has_provider(&desc.id);
            let entry = self
                .views
                .entry(desc.id.clone())
                .or_insert_with(|| ContributedViewState::new(desc.clone()));
            entry.descriptor = desc;
            entry.visible = visible;
            entry.has_provider = provider_ok;
        }
        self.views.retain(|id, _| seen.contains(id));
        self.order = new_order;
    }

    pub fn visible_views(&self) -> Vec<&ContributedViewState> {
        self.order
            .iter()
            .filter_map(|id| self.views.get(id))
            .filter(|v| v.visible)
            .collect()
    }
}

pub type SharedContributedTrees = Arc<RwLock<ContributedTreesState>>;

#[derive(Debug, Clone)]
struct ContributedSectionLayout {
    view_id: String,
    view_name: String,
    collapsed: bool,
    selected_item_id: Option<String>,
    section_rect: Rect,
    header_rect: Rect,
    items: Vec<(ContributedTreeNode, usize, bool, Rect)>,
}

#[derive(Debug, Clone)]
struct ExplorerLayout {
    header_rect: Rect,
    new_file_rect: Rect,
    new_folder_rect: Rect,
    inline_input_rect: Option<Rect>,
    inline_error_rect: Option<Rect>,
    confirm_rect: Option<Rect>,
    confirm_delete_btn_rect: Option<Rect>,
    confirm_cancel_btn_rect: Option<Rect>,
    file_tree_rect: Rect,
    file_rows: Vec<(ExplorerRow, Rect)>,
    contrib_sections: Vec<ContributedSectionLayout>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplorerDragState {
    pub source_path: PathBuf,
    pub is_dir: bool,
    pub start_pos: Point,
    pub current_pos: Point,
    pub active: bool,
}

pub struct FileExplorerView {
    pub model: FileExplorer,
    pub last_action: Option<ExplorerAction>,
    pub input_mode: ExplorerInputMode,
    pub scroll_top: f32,
    pub contributed_trees: SharedContributedTrees,
    pub drag_state: Option<ExplorerDragState>,
    pub hovered_drop_dir: Option<PathBuf>,
    last_bounds: RwLock<Rect>,
    last_font: RwLock<FontMetrics>,
}

impl Default for FileExplorerView {
    fn default() -> Self {
        Self::new(FileExplorer::new())
    }
}

impl FileExplorerView {
    pub fn new(model: FileExplorer) -> Self {
        Self {
            model,
            last_action: None,
            input_mode: ExplorerInputMode::None,
            scroll_top: 0.0,
            contributed_trees: Arc::new(RwLock::new(ContributedTreesState::default())),
            drag_state: None,
            hovered_drop_dir: None,
            last_bounds: RwLock::new(Rect::zero()),
            last_font: RwLock::new(FontMetrics::default_monospace()),
        }
    }

    pub fn is_input_active(&self) -> bool {
        !matches!(self.input_mode, ExplorerInputMode::None)
    }

    pub fn flattened_rows(&self) -> Vec<ExplorerRow> {
        let mut rows = Vec::new();
        for root in &self.model.roots {
            Self::flatten_node(
                root,
                0,
                &self.model.expanded_paths,
                &self.model.selected_path,
                &mut rows,
            );
        }
        rows
    }

    fn flatten_node(
        node: &FileTreeNode,
        depth: usize,
        expanded_paths: &HashSet<PathBuf>,
        selected_path: &Option<PathBuf>,
        rows: &mut Vec<ExplorerRow>,
    ) {
        let is_expanded = expanded_paths.contains(&node.path)
            || expanded_paths
                .iter()
                .any(|p| rustcode_platform::paths_equivalent(p, &node.path));
        let is_selected = selected_path
            .as_ref()
            .is_some_and(|sel| sel == &node.path || rustcode_platform::paths_equivalent(sel, &node.path));

        rows.push(ExplorerRow {
            path: node.path.clone(),
            name: node.name.clone(),
            is_dir: node.is_dir,
            depth,
            is_expanded,
            is_selected,
        });

        if node.is_dir && is_expanded {
            if let Some(children) = &node.children {
                for child in children {
                    Self::flatten_node(child, depth + 1, expanded_paths, selected_path, rows);
                }
            }
        }
    }

    fn resolve_target_dir(&self) -> Option<PathBuf> {
        if let Some(sel) = &self.model.selected_path {
            let is_dir = self
                .model
                .find_node(sel)
                .map(|n| n.is_dir)
                .unwrap_or_else(|| sel.is_dir());
            if is_dir {
                return Some(sel.clone());
            }
            if let Some(parent) = sel.parent().filter(|p| !p.as_os_str().is_empty()) {
                return Some(parent.to_path_buf());
            }
        }
        self.model.roots.first().map(|r| r.path.clone())
    }

    pub fn start_new_file(&mut self) -> bool {
        let Some(target_dir) = self.resolve_target_dir() else {
            return false;
        };
        self.input_mode = ExplorerInputMode::NewFile {
            target_dir,
            buffer: String::new(),
            error: None,
        };
        true
    }

    pub fn start_new_folder(&mut self) -> bool {
        let Some(target_dir) = self.resolve_target_dir() else {
            return false;
        };
        self.input_mode = ExplorerInputMode::NewFolder {
            target_dir,
            buffer: String::new(),
            error: None,
        };
        true
    }

    pub fn start_rename(&mut self) -> bool {
        let Some(target_path) = self.model.selected_path.clone() else {
            return false;
        };
        if self.model.is_root_path(&target_path) {
            return false;
        }
        let existing_name = node_display_name(&target_path);
        self.input_mode = ExplorerInputMode::Rename {
            target_path,
            buffer: existing_name,
            select_all: true,
            error: None,
        };
        true
    }

    pub fn start_delete(&mut self) -> bool {
        let Some(target_path) = self.model.selected_path.clone() else {
            return false;
        };
        if self.model.is_root_path(&target_path) {
            return false;
        }
        let is_dir = self
            .model
            .find_node(&target_path)
            .map(|n| n.is_dir)
            .unwrap_or_else(|| target_path.is_dir());
        self.input_mode = ExplorerInputMode::ConfirmDelete {
            target_path,
            is_dir,
        };
        true
    }

    fn execute_confirm_delete(&mut self) -> bool {
        let ExplorerInputMode::ConfirmDelete {
            target_path,
            is_dir,
        } = self.input_mode.clone()
        else {
            return false;
        };

        let res = if is_dir || target_path.is_dir() {
            std::fs::remove_dir_all(&target_path)
        } else {
            std::fs::remove_file(&target_path)
        };

        self.input_mode = ExplorerInputMode::None;
        if res.is_ok() {
            self.model.remove_deleted_paths(&target_path);
            self.model.refresh_from_disk();
            self.last_action = Some(ExplorerAction::Deleted(target_path));
            true
        } else {
            false
        }
    }

    fn submit_inline_input(&mut self) {
        let mode = self.input_mode.clone();
        match mode {
            ExplorerInputMode::NewFile {
                target_dir, buffer, ..
            } => {
                if let Err(msg) = validate_entry_name(&buffer) {
                    self.input_mode = ExplorerInputMode::NewFile {
                        target_dir,
                        buffer,
                        error: Some(msg.to_string()),
                    };
                    return;
                }
                let new_path = target_dir.join(&buffer);
                if std::fs::symlink_metadata(&new_path).is_ok() {
                    self.input_mode = ExplorerInputMode::NewFile {
                        target_dir,
                        buffer,
                        error: Some("Already exists".to_string()),
                    };
                    return;
                }
                if let Err(e) = std::fs::write(&new_path, "") {
                    self.input_mode = ExplorerInputMode::NewFile {
                        target_dir,
                        buffer,
                        error: Some(e.to_string()),
                    };
                    return;
                }
                self.model.expanded_paths.insert(target_dir);
                self.model.refresh_from_disk();
                self.model.selected_path = Some(new_path.clone());
                self.input_mode = ExplorerInputMode::None;
                self.last_action = Some(ExplorerAction::CreateFile(new_path));
            }
            ExplorerInputMode::NewFolder {
                target_dir, buffer, ..
            } => {
                if let Err(msg) = validate_entry_name(&buffer) {
                    self.input_mode = ExplorerInputMode::NewFolder {
                        target_dir,
                        buffer,
                        error: Some(msg.to_string()),
                    };
                    return;
                }
                let new_path = target_dir.join(&buffer);
                if std::fs::symlink_metadata(&new_path).is_ok() {
                    self.input_mode = ExplorerInputMode::NewFolder {
                        target_dir,
                        buffer,
                        error: Some("Already exists".to_string()),
                    };
                    return;
                }
                if let Err(e) = std::fs::create_dir_all(&new_path) {
                    self.input_mode = ExplorerInputMode::NewFolder {
                        target_dir,
                        buffer,
                        error: Some(e.to_string()),
                    };
                    return;
                }
                self.model.expanded_paths.insert(target_dir);
                self.model.expanded_paths.insert(new_path.clone());
                self.model.refresh_from_disk();
                self.model.selected_path = Some(new_path.clone());
                self.input_mode = ExplorerInputMode::None;
                self.last_action = Some(ExplorerAction::CreateFolder(new_path));
            }
            ExplorerInputMode::Rename {
                target_path,
                buffer,
                ..
            } => {
                if let Err(msg) = validate_entry_name(&buffer) {
                    self.input_mode = ExplorerInputMode::Rename {
                        target_path,
                        buffer,
                        select_all: false,
                        error: Some(msg.to_string()),
                    };
                    return;
                }
                let old_name = node_display_name(&target_path);
                if buffer == old_name {
                    self.input_mode = ExplorerInputMode::None;
                    return;
                }
                let Some(parent) = target_path.parent() else {
                    self.input_mode = ExplorerInputMode::None;
                    return;
                };
                let new_path = parent.join(&buffer);
                if !rustcode_platform::paths_equivalent(&target_path, &new_path)
                    && std::fs::symlink_metadata(&new_path).is_ok()
                {
                    self.input_mode = ExplorerInputMode::Rename {
                        target_path,
                        buffer,
                        select_all: false,
                        error: Some("Already exists".to_string()),
                    };
                    return;
                }
                if let Err(e) = std::fs::rename(&target_path, &new_path) {
                    self.input_mode = ExplorerInputMode::Rename {
                        target_path,
                        buffer,
                        select_all: false,
                        error: Some(e.to_string()),
                    };
                    return;
                }
                self.model.update_paths_after_rename(&target_path, &new_path);
                self.model.refresh_from_disk();
                self.model.selected_path = Some(new_path.clone());
                self.input_mode = ExplorerInputMode::None;
                self.last_action = Some(ExplorerAction::Renamed {
                    old_path: target_path,
                    new_path,
                });
            }
            ExplorerInputMode::None | ExplorerInputMode::ConfirmDelete { .. } => {}
        }
    }

    pub fn clear_drag(&mut self) {
        self.drag_state = None;
        self.hovered_drop_dir = None;
    }

    pub fn take_active_drag(&mut self) -> Option<ExplorerDragState> {
        self.hovered_drop_dir = None;
        match self.drag_state.take() {
            Some(d) if d.active => Some(d),
            _ => None,
        }
    }

    /// Move `src` (file or folder) into `target_dir`, guarding against cycles and collisions.
    pub fn move_entry(&mut self, src: &Path, target_dir: &Path) -> Result<PathBuf, String> {
        if !src.exists() {
            return Err("Source path does not exist".to_string());
        }
        if !target_dir.is_dir() {
            return Err("Target path is not a directory".to_string());
        }
        if rustcode_platform::paths_equivalent(src, target_dir)
            || rustcode_platform::path_starts_with(target_dir, src)
        {
            return Err("Cannot move a directory into itself or a descendant".to_string());
        }
        let Some(file_name) = src.file_name() else {
            return Err("Source path has no file name".to_string());
        };
        let dest = target_dir.join(file_name);
        if rustcode_platform::paths_equivalent(src, &dest) {
            return Err("Source is already in the target directory".to_string());
        }
        if dest.exists() {
            return Err("Target destination already exists".to_string());
        }
        std::fs::rename(src, &dest).map_err(|e| format!("Failed moving entry: {e}"))?;
        self.model.update_paths_after_rename(src, &dest);
        self.model.expanded_paths.insert(target_dir.to_path_buf());
        self.model.refresh_from_disk();
        self.model.selected_path = Some(dest.clone());
        self.last_action = Some(ExplorerAction::Moved {
            from: src.to_path_buf(),
            to: dest.clone(),
        });
        Ok(dest)
    }

    pub fn entry_path_at_point(&self, bounds: Rect, pos: Point) -> Option<PathBuf> {
        if !bounds.is_finite_positive() || !bounds.contains(pos) {
            return None;
        }
        let font = self.last_font.read().clone();
        let layout = self.compute_layout(bounds, &font);
        layout
            .file_rows
            .into_iter()
            .find(|(_, r)| r.contains(pos))
            .map(|(row, _)| row.path)
    }

    fn resolve_drop_dir_at(&self, layout: &ExplorerLayout, pos: Point) -> Option<PathBuf> {
        for (row, r) in &layout.file_rows {
            if r.contains(pos) {
                return if row.is_dir {
                    Some(row.path.clone())
                } else {
                    row.path.parent().map(Path::to_path_buf)
                };
            }
        }
        if layout.file_tree_rect.contains(pos) {
            return self.model.roots.first().map(|r| r.path.clone());
        }
        None
    }

    fn compute_layout(&self, bounds: Rect, font: &FontMetrics) -> ExplorerLayout {
        let safe_bounds = if bounds.is_finite_positive() {
            bounds
        } else {
            return ExplorerLayout {
                header_rect: Rect::zero(),
                new_file_rect: Rect::zero(),
                new_folder_rect: Rect::zero(),
                inline_input_rect: None,
                inline_error_rect: None,
                confirm_rect: None,
                confirm_delete_btn_rect: None,
                confirm_cancel_btn_rect: None,
                file_tree_rect: Rect::zero(),
                file_rows: Vec::new(),
                contrib_sections: Vec::new(),
            };
        };

        let header_h = 28.0f32.min(safe_bounds.height());
        let header_rect = Rect::from_xywh(
            safe_bounds.min_x(),
            safe_bounds.min_y(),
            safe_bounds.width(),
            header_h,
        )
        .clamp_to_bounds(safe_bounds);

        let btn_h = 20.0f32.min(header_h);
        let btn_y = header_rect.min_y() + ((header_h - btn_h) * 0.5).max(0.0);
        let btn_w = ((font.text_width("+F") + 8.0).max(22.0))
            .min((header_rect.width() * 0.28).max(0.0));
        let gap = 4.0f32.min((header_rect.width() * 0.05).max(0.0));
        let right_pad = 4.0f32.min((header_rect.width() * 0.05).max(0.0));

        let new_folder_rect = Rect::from_xywh(
            (header_rect.max_x() - right_pad - btn_w).max(header_rect.min_x()),
            btn_y,
            btn_w,
            btn_h,
        )
        .clamp_to_bounds(header_rect);
        let new_file_rect = Rect::from_xywh(
            (new_folder_rect.min_x() - gap - btn_w).max(header_rect.min_x()),
            btn_y,
            btn_w,
            btn_h,
        )
        .clamp_to_bounds(header_rect);

        let mut cur_y = header_rect.max_y();
        let mut inline_input_rect = None;
        let mut inline_error_rect = None;
        let mut confirm_rect = None;
        let mut confirm_delete_btn_rect = None;
        let mut confirm_cancel_btn_rect = None;

        match &self.input_mode {
            ExplorerInputMode::NewFile { error, .. }
            | ExplorerInputMode::NewFolder { error, .. }
            | ExplorerInputMode::Rename { error, .. } => {
                let ih = 24.0f32.min((safe_bounds.max_y() - cur_y).max(0.0));
                let r = Rect::from_xywh(safe_bounds.min_x(), cur_y, safe_bounds.width(), ih)
                    .clamp_to_bounds(safe_bounds);
                inline_input_rect = Some(r);
                cur_y = r.max_y();

                if error.is_some() && cur_y < safe_bounds.max_y() {
                    let eh = 20.0f32.min((safe_bounds.max_y() - cur_y).max(0.0));
                    let er = Rect::from_xywh(safe_bounds.min_x(), cur_y, safe_bounds.width(), eh)
                        .clamp_to_bounds(safe_bounds);
                    inline_error_rect = Some(er);
                    cur_y = er.max_y();
                }
            }
            ExplorerInputMode::ConfirmDelete { .. } => {
                let ch = 44.0f32.min((safe_bounds.max_y() - cur_y).max(0.0));
                let cr = Rect::from_xywh(safe_bounds.min_x(), cur_y, safe_bounds.width(), ch)
                    .clamp_to_bounds(safe_bounds);
                let btn_row_h = 20.0f32.min((ch * 0.5).max(0.0));
                let btn_row_y = cr.min_y() + (ch - btn_row_h - 2.0).max(0.0);
                let bw = ((cr.width() - 18.0) * 0.5).clamp(0.0, 72.0);
                let del_r = Rect::from_xywh(cr.min_x() + 6.0, btn_row_y, bw, btn_row_h)
                    .clamp_to_bounds(cr);
                let cancel_r = Rect::from_xywh(del_r.max_x() + 6.0, btn_row_y, bw, btn_row_h)
                    .clamp_to_bounds(cr);
                confirm_rect = Some(cr);
                confirm_delete_btn_rect = Some(del_r);
                confirm_cancel_btn_rect = Some(cancel_r);
                cur_y = cr.max_y();
            }
            ExplorerInputMode::None => {}
        }

        let body_rect = Rect::from_xywh(
            safe_bounds.min_x(),
            cur_y,
            safe_bounds.width(),
            (safe_bounds.max_y() - cur_y).max(0.0),
        )
        .clamp_to_bounds(safe_bounds);

        let all_file_rows = self.flattened_rows();
        let row_h = 22.0;

        type ContribSnapshot = (
            String,
            String,
            bool,
            Option<String>,
            Vec<(ContributedTreeNode, usize, bool)>,
        );
        let contrib_snapshots: Vec<ContribSnapshot> = {
            let guard = self.contributed_trees.read();
            guard
                .visible_views()
                .into_iter()
                .map(|v| {
                    (
                        v.descriptor.id.clone(),
                        v.descriptor.name.clone(),
                        v.collapsed,
                        v.selected_item_id.clone(),
                        if v.collapsed {
                            Vec::new()
                        } else {
                            v.flattened_items()
                        },
                    )
                })
                .collect()
        };

        let (file_tree_rect, contrib_rects) = if contrib_snapshots.is_empty() {
            (body_rect, Vec::new())
        } else {
            let file_desired = (all_file_rows.len().max(1) as f32) * row_h;
            let contrib_desired: Vec<f32> = contrib_snapshots
                .iter()
                .map(|(_, _, collapsed, _, items)| {
                    if *collapsed {
                        24.0
                    } else {
                        24.0 + (items.len().max(1) as f32) * row_h
                    }
                })
                .collect();
            let contrib_sum: f32 = contrib_desired.iter().sum();
            let total_h = body_rect.height();

            let (file_h, sec_heights) = if file_desired + contrib_sum <= total_h {
                (file_desired, contrib_desired)
            } else {
                let fh = (total_h * 0.45)
                    .min(file_desired)
                    .max((total_h * 0.25).min(total_h));
                let rem = (total_h - fh).max(0.0);
                let per = rem / contrib_snapshots.len() as f32;
                (fh, vec![per; contrib_snapshots.len()])
            };

            let ft_rect = Rect::from_xywh(
                body_rect.min_x(),
                body_rect.min_y(),
                body_rect.width(),
                file_h,
            )
            .clamp_to_bounds(body_rect);
            let mut sec_y = ft_rect.max_y();
            let mut rects = Vec::with_capacity(sec_heights.len());
            for sh in sec_heights {
                let sr = Rect::from_xywh(
                    body_rect.min_x(),
                    sec_y,
                    body_rect.width(),
                    sh.min((body_rect.max_y() - sec_y).max(0.0)),
                )
                .clamp_to_bounds(body_rect);
                sec_y = sr.max_y();
                rects.push(sr);
            }
            (ft_rect, rects)
        };

        let mut file_rows = Vec::new();
        for (idx, row) in all_file_rows.into_iter().enumerate() {
            let row_y = file_tree_rect.min_y() - self.scroll_top + idx as f32 * row_h;
            if row_y + row_h <= file_tree_rect.min_y() {
                continue;
            }
            if row_y >= file_tree_rect.max_y() {
                break;
            }
            let rr = Rect::from_xywh(
                file_tree_rect.min_x(),
                row_y,
                file_tree_rect.width(),
                row_h.min((file_tree_rect.max_y() - row_y).max(0.0)),
            )
            .clamp_to_bounds(file_tree_rect);
            file_rows.push((row, rr));
        }

        let mut contrib_sections = Vec::with_capacity(contrib_snapshots.len());
        for ((view_id, view_name, collapsed, selected_item_id, flat_items), section_rect) in
            contrib_snapshots.into_iter().zip(contrib_rects)
        {
            let sec_header_h = 24.0f32.min(section_rect.height());
            let sec_header_rect = Rect::from_xywh(
                section_rect.min_x(),
                section_rect.min_y(),
                section_rect.width(),
                sec_header_h,
            )
            .clamp_to_bounds(section_rect);
            let sec_body_rect = Rect::from_xywh(
                section_rect.min_x(),
                sec_header_rect.max_y(),
                section_rect.width(),
                (section_rect.max_y() - sec_header_rect.max_y()).max(0.0),
            )
            .clamp_to_bounds(section_rect);

            let mut item_layouts = Vec::new();
            for (idx, (node, depth, is_expanded)) in flat_items.into_iter().enumerate() {
                let iy = sec_body_rect.min_y() + idx as f32 * row_h;
                if iy >= sec_body_rect.max_y() {
                    break;
                }
                let ir = Rect::from_xywh(
                    sec_body_rect.min_x(),
                    iy,
                    sec_body_rect.width(),
                    row_h.min((sec_body_rect.max_y() - iy).max(0.0)),
                )
                .clamp_to_bounds(sec_body_rect);
                item_layouts.push((node, depth, is_expanded, ir));
            }

            contrib_sections.push(ContributedSectionLayout {
                view_id,
                view_name,
                collapsed,
                selected_item_id,
                section_rect,
                header_rect: sec_header_rect,
                items: item_layouts,
            });
        }

        ExplorerLayout {
            header_rect,
            new_file_rect,
            new_folder_rect,
            inline_input_rect,
            inline_error_rect,
            confirm_rect,
            confirm_delete_btn_rect,
            confirm_cancel_btn_rect,
            file_tree_rect,
            file_rows,
            contrib_sections,
        }
    }

    pub fn accessibility_nodes(&self, bounds: Rect, font: &FontMetrics) -> Vec<AccessibleNode> {
        let layout = self.compute_layout(bounds, font);
        let mut nodes = Vec::new();

        nodes.push(
            AccessibleNode::new(
                "explorer.newFile",
                AccessibleRole::Button,
                "New File",
                layout.new_file_rect,
            )
            .with_action("press"),
        );
        nodes.push(
            AccessibleNode::new(
                "explorer.newFolder",
                AccessibleRole::Button,
                "New Folder",
                layout.new_folder_rect,
            )
            .with_action("press"),
        );

        match &self.input_mode {
            ExplorerInputMode::NewFile { buffer, error, .. }
            | ExplorerInputMode::NewFolder { buffer, error, .. }
            | ExplorerInputMode::Rename { buffer, error, .. } => {
                let label = match &self.input_mode {
                    ExplorerInputMode::NewFile { .. } => "New File Name",
                    ExplorerInputMode::NewFolder { .. } => "New Folder Name",
                    ExplorerInputMode::Rename { .. } => "Rename File or Folder",
                    _ => "Input",
                };
                if let Some(ir) = layout.inline_input_rect {
                    nodes.push(
                        AccessibleNode::new(
                            "explorer.inlineInput",
                            AccessibleRole::TextInput,
                            label,
                            ir,
                        )
                        .with_value(buffer.clone())
                        .with_focused(true)
                        .with_action("input"),
                    );
                }
                if let (Some(err), Some(er)) = (error, layout.inline_error_rect) {
                    nodes.push(AccessibleNode::new(
                        "explorer.inlineError",
                        AccessibleRole::StaticText,
                        err.clone(),
                        er,
                    ));
                }
            }
            ExplorerInputMode::ConfirmDelete { target_path, .. } => {
                if let Some(cr) = layout.confirm_rect {
                    let name = node_display_name(target_path);
                    let mut dlg = AccessibleNode::new(
                        "explorer.confirmDelete",
                        AccessibleRole::Dialog,
                        format!("Confirm delete {}", name),
                        cr,
                    );
                    if let Some(dr) = layout.confirm_delete_btn_rect {
                        dlg.add_child(
                            AccessibleNode::new(
                                "explorer.confirmDelete.confirm",
                                AccessibleRole::Button,
                                "Confirm Delete",
                                dr,
                            )
                            .with_action("press"),
                        );
                    }
                    if let Some(cancel_r) = layout.confirm_cancel_btn_rect {
                        dlg.add_child(
                            AccessibleNode::new(
                                "explorer.confirmDelete.cancel",
                                AccessibleRole::Button,
                                "Cancel",
                                cancel_r,
                            )
                            .with_action("press"),
                        );
                    }
                    nodes.push(dlg);
                }
            }
            ExplorerInputMode::None => {}
        }

        let mut tree = AccessibleNode::new(
            "workbench.sidebar.explorer.tree",
            AccessibleRole::Tree,
            "Explorer",
            layout.file_tree_rect,
        );
        for (idx, (row, row_rect)) in layout.file_rows.into_iter().enumerate() {
            let mut item = AccessibleNode::new(
                format!("explorer.row.{}", idx),
                AccessibleRole::TreeItem,
                row.name,
                row_rect,
            )
            .with_selected(row.is_selected);
            if row.is_dir {
                item = item.with_expanded(row.is_expanded);
            }
            tree.add_child(item);
        }
        nodes.push(tree);

        for sec in layout.contrib_sections {
            let mut contrib_tree = AccessibleNode::new(
                format!("workbench.sidebar.explorer.contributed.{}", sec.view_id),
                AccessibleRole::Tree,
                sec.view_name,
                sec.section_rect,
            )
            .with_expanded(!sec.collapsed);

            for (node, _depth, is_expanded, item_rect) in sec.items {
                let mut item_node = AccessibleNode::new(
                    format!("explorer.contrib.{}.{}", sec.view_id, node.node_id),
                    AccessibleRole::TreeItem,
                    node.label,
                    item_rect,
                )
                .with_selected(sec.selected_item_id.as_deref() == Some(node.node_id.as_str()));
                if node.collapsible_state != TreeItemCollapsibleState::None {
                    item_node = item_node.with_expanded(is_expanded);
                }
                if let Some(desc) = node.description {
                    item_node = item_node.with_description(desc);
                }
                contrib_tree.add_child(item_node);
            }
            nodes.push(contrib_tree);
        }

        nodes
    }
}

impl View for FileExplorerView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !bounds.is_finite_positive() || !font.is_finite() {
            return;
        }
        *self.last_font.write() = font.clone();

        scene.push_finite_quad(
            Quad::new(bounds)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.sidebar_border, Edges::new(0.0, 1.0, 0.0, 0.0)),
        );

        let layout = self.compute_layout(bounds, font);

        // Header ("EXPLORER" + "+F" + "+D")
        if layout.header_rect.width() > 0.0 && layout.header_rect.height() > 0.0 {
            let title_x = (layout.header_rect.min_x() + 12.0).min(layout.header_rect.max_x());
            let title_y = layout.header_rect.min_y()
                + ((layout.header_rect.height() - font.line_height) * 0.5).max(0.0);
            let title_w = (layout.new_file_rect.min_x() - title_x - 4.0).max(0.0);
            if title_w > 0.0 {
                let tr = Rect::from_xywh(
                    title_x,
                    title_y,
                    title_w,
                    font.line_height.min(layout.header_rect.height()),
                )
                .clamp_to_bounds(layout.header_rect);
                scene.push_finite_text(
                    GlyphRun::new(
                        "EXPLORER".to_string(),
                        Point::new(title_x, title_y),
                        tr,
                        theme.sidebar_header_fg,
                        font.font_size * 0.85,
                    ),
                );
            }

            for (btn_rect, label) in [(layout.new_file_rect, "+F"), (layout.new_folder_rect, "+D")] {
                if btn_rect.width() > 2.0 && btn_rect.height() > 2.0 {
                    scene.push_finite_quad(
                        Quad::new(btn_rect)
                            .with_background(theme.command_palette_selected_bg)
                            .with_corner_radius(3.0),
                    );
                    let lx = (btn_rect.min_x() + 4.0).min(btn_rect.max_x());
                    let ly = btn_rect.min_y()
                        + ((btn_rect.height() - font.line_height) * 0.5).max(0.0);
                    let lr = Rect::from_xywh(
                        lx,
                        ly,
                        (btn_rect.max_x() - lx).max(0.0),
                        font.line_height.min(btn_rect.height()),
                    )
                    .clamp_to_bounds(btn_rect);
                    scene.push_finite_text(
                        GlyphRun::new(
                            label.to_string(),
                            Point::new(lx, ly),
                            lr,
                            theme.activity_bar_fg,
                            font.font_size * 0.85,
                        ),
                    );
                }
            }
        }

        // Inline input or ConfirmDelete banner
        match &self.input_mode {
            ExplorerInputMode::NewFile { buffer, error, .. }
            | ExplorerInputMode::NewFolder { buffer, error, .. }
            | ExplorerInputMode::Rename { buffer, error, .. } => {
                if let Some(ir) = layout.inline_input_rect {
                    if ir.width() > 0.0 && ir.height() > 0.0 {
                        let prefix = match &self.input_mode {
                            ExplorerInputMode::NewFile { .. } => "New File: ",
                            ExplorerInputMode::NewFolder { .. } => "New Folder: ",
                            ExplorerInputMode::Rename { .. } => "Rename: ",
                            _ => "",
                        };
                        scene.push_finite_quad(
                            Quad::new(ir)
                                .with_background(theme.command_palette_bg)
                                .with_border(theme.activity_bar_active_border, 1.0),
                        );
                        let tx = (ir.min_x() + 8.0).min(ir.max_x());
                        let ty = ir.min_y() + ((ir.height() - font.line_height) * 0.5).max(0.0);
                        let tr = Rect::from_xywh(
                            tx,
                            ty,
                            (ir.max_x() - tx - 4.0).max(0.0),
                            font.line_height.min(ir.height()),
                        )
                        .clamp_to_bounds(ir);
                        scene.push_finite_text(
                            GlyphRun::new(
                                format!("{}{}", prefix, buffer),
                                Point::new(tx, ty),
                                tr,
                                theme.sidebar_fg,
                                font.font_size * 0.9,
                            ),
                        );
                    }
                }
                if let (Some(err), Some(er)) = (error, layout.inline_error_rect) {
                    if er.width() > 0.0 && er.height() > 0.0 {
                        scene.push_finite_quad(
                            Quad::new(er).with_background(theme.editor_background),
                        );
                        let ex = (er.min_x() + 8.0).min(er.max_x());
                        let ey = er.min_y() + ((er.height() - font.line_height) * 0.5).max(0.0);
                        let clamped = Rect::from_xywh(
                            ex,
                            ey,
                            (er.max_x() - ex - 4.0).max(0.0),
                            font.line_height.min(er.height()),
                        )
                        .clamp_to_bounds(er);
                        scene.push_finite_text(
                            GlyphRun::new(
                                err.clone(),
                                Point::new(ex, ey),
                                clamped,
                                theme.editor_error_foreground,
                                font.font_size * 0.85,
                            ),
                        );
                    }
                }
            }
            ExplorerInputMode::ConfirmDelete { target_path, .. } => {
                if let Some(cr) = layout.confirm_rect {
                    if cr.width() > 0.0 && cr.height() > 0.0 {
                        scene.push_finite_quad(
                            Quad::new(cr)
                                .with_background(theme.command_palette_bg)
                                .with_border(theme.editor_error_foreground, 1.0),
                        );
                        let name = node_display_name(target_path);
                        let prompt = format!("Confirm delete {}?", name);
                        let px = (cr.min_x() + 6.0).min(cr.max_x());
                        let py = cr.min_y() + 2.0;
                        let pr = Rect::from_xywh(
                            px,
                            py,
                            (cr.max_x() - px - 4.0).max(0.0),
                            font.line_height.min(cr.height()),
                        )
                        .clamp_to_bounds(cr);
                        scene.push_finite_text(
                            GlyphRun::new(
                                prompt,
                                Point::new(px, py),
                                pr,
                                theme.sidebar_fg,
                                font.font_size * 0.85,
                            ),
                        );
                        if let Some(dr) = layout.confirm_delete_btn_rect {
                            if dr.width() > 2.0 && dr.height() > 2.0 {
                                scene.push_finite_quad(
                                    Quad::new(dr)
                                        .with_background(theme.editor_error_foreground)
                                        .with_corner_radius(3.0),
                                );
                                let lx = (dr.min_x() + 4.0).min(dr.max_x());
                                let ly = dr.min_y()
                                    + ((dr.height() - font.line_height) * 0.5).max(0.0);
                                let lr = Rect::from_xywh(
                                    lx,
                                    ly,
                                    (dr.max_x() - lx).max(0.0),
                                    font.line_height.min(dr.height()),
                                )
                                .clamp_to_bounds(dr);
                                scene.push_finite_text(
                                    GlyphRun::new(
                                        "Delete".to_string(),
                                        Point::new(lx, ly),
                                        lr,
                                        theme.activity_bar_fg,
                                        font.font_size * 0.85,
                                    ),
                                );
                            }
                        }
                        if let Some(cancel_r) = layout.confirm_cancel_btn_rect {
                            if cancel_r.width() > 2.0 && cancel_r.height() > 2.0 {
                                scene.push_finite_quad(
                                    Quad::new(cancel_r)
                                        .with_background(theme.tab_inactive_bg)
                                        .with_corner_radius(3.0),
                                );
                                let lx = (cancel_r.min_x() + 4.0).min(cancel_r.max_x());
                                let ly = cancel_r.min_y()
                                    + ((cancel_r.height() - font.line_height) * 0.5).max(0.0);
                                let lr = Rect::from_xywh(
                                    lx,
                                    ly,
                                    (cancel_r.max_x() - lx).max(0.0),
                                    font.line_height.min(cancel_r.height()),
                                )
                                .clamp_to_bounds(cancel_r);
                                scene.push_finite_text(
                                    GlyphRun::new(
                                        "Cancel".to_string(),
                                        Point::new(lx, ly),
                                        lr,
                                        theme.sidebar_fg,
                                        font.font_size * 0.85,
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            ExplorerInputMode::None => {}
        }

        // Workspace file rows
        for (row, row_rect) in layout.file_rows {
            if row_rect.width() <= 0.0 || row_rect.height() <= 0.0 {
                continue;
            }
            if row.is_selected {
                scene.push_finite_quad(
                    Quad::new(row_rect).with_background(theme.list_active_selection_bg),
                );
            }
            if self
                .hovered_drop_dir
                .as_ref()
                .is_some_and(|d| row.is_dir && rustcode_platform::paths_equivalent(&row.path, d))
            {
                scene.push_finite_quad(
                    Quad::new(row_rect)
                        .with_background(theme.list_hover_bg)
                        .with_border_edges(theme.activity_bar_active_border, Edges::all(1.0)),
                );
            }

            let max_indent = (row_rect.width() - 48.0).max(0.0);
            let indent_offset = (8.0 + (row.depth as f32 * 14.0)).min(max_indent);
            let indent_x = (row_rect.min_x() + indent_offset).min(row_rect.max_x());
            let prefix = if row.is_dir {
                if row.is_expanded { "v " } else { "> " }
            } else {
                "  "
            };
            let icon = if row.is_dir { "[D] " } else { "[F] " };
            let full_label = format!("{}{}{}", prefix, icon, row.name);
            let label_w = font
                .text_width(&full_label)
                .min((row_rect.max_x() - indent_x).max(0.0));
            let text_rect = Rect::from_xywh(
                indent_x,
                row_rect.min_y(),
                label_w,
                font.line_height.min(row_rect.height()),
            )
            .clamp_to_bounds(row_rect);
            scene.push_finite_text(
                GlyphRun::new(
                    full_label,
                    Point::new(indent_x, row_rect.min_y()),
                    text_rect,
                    theme.sidebar_fg,
                    font.font_size,
                ),
            );
        }

        // Contributed TreeView sections
        for sec in layout.contrib_sections {
            if sec.section_rect.width() <= 0.0 || sec.section_rect.height() <= 0.0 {
                continue;
            }
            if sec.header_rect.width() > 0.0 && sec.header_rect.height() > 0.0 {
                scene.push_finite_quad(
                    Quad::new(sec.header_rect)
                        .with_background(theme.tab_inactive_bg)
                        .with_border_edges(theme.sidebar_border, Edges::new(1.0, 0.0, 1.0, 0.0)),
                );
                let chevron = if sec.collapsed { "> " } else { "v " };
                let header_label = format!("{}{}", chevron, sec.view_name);
                let hx = (sec.header_rect.min_x() + 8.0).min(sec.header_rect.max_x());
                let hy = sec.header_rect.min_y()
                    + ((sec.header_rect.height() - font.line_height) * 0.5).max(0.0);
                let hr = Rect::from_xywh(
                    hx,
                    hy,
                    (sec.header_rect.max_x() - hx - 4.0).max(0.0),
                    font.line_height.min(sec.header_rect.height()),
                )
                .clamp_to_bounds(sec.header_rect);
                scene.push_finite_text(
                    GlyphRun::new(
                        header_label,
                        Point::new(hx, hy),
                        hr,
                        theme.sidebar_header_fg,
                        font.font_size * 0.85,
                    ),
                );
            }

            if sec.collapsed {
                continue;
            }
            for (node, depth, is_expanded, item_rect) in sec.items {
                if item_rect.width() <= 0.0 || item_rect.height() <= 0.0 {
                    continue;
                }
                if sec.selected_item_id.as_deref() == Some(node.node_id.as_str()) {
                    scene.push_finite_quad(
                        Quad::new(item_rect).with_background(theme.list_active_selection_bg),
                    );
                }
                let max_indent = (item_rect.width() - 48.0).max(0.0);
                let indent_x = (item_rect.min_x() + (8.0 + depth as f32 * 14.0).min(max_indent))
                    .min(item_rect.max_x());
                let prefix = match node.collapsible_state {
                    TreeItemCollapsibleState::None => "  ",
                    _ if is_expanded => "v ",
                    _ => "> ",
                };
                let full_label = format!("{}{}", prefix, node.label);
                let lw = font
                    .text_width(&full_label)
                    .min((item_rect.max_x() - indent_x).max(0.0));
                let lr = Rect::from_xywh(
                    indent_x,
                    item_rect.min_y(),
                    lw,
                    font.line_height.min(item_rect.height()),
                )
                .clamp_to_bounds(item_rect);
                scene.push_finite_text(
                    GlyphRun::new(
                        full_label,
                        Point::new(indent_x, item_rect.min_y()),
                        lr,
                        theme.sidebar_fg,
                        font.font_size,
                    ),
                );
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let font = self.last_font.read().clone();
        match event {
            InputEvent::MouseDown(mouse) => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }
                let layout = self.compute_layout(bounds, &font);

                if layout.new_file_rect.contains(mouse.position) {
                    self.last_action = None;
                    self.start_new_file();
                    return EventResult::Consumed;
                }
                if layout.new_folder_rect.contains(mouse.position) {
                    self.last_action = None;
                    self.start_new_folder();
                    return EventResult::Consumed;
                }

                if matches!(self.input_mode, ExplorerInputMode::ConfirmDelete { .. }) {
                    if layout
                        .confirm_delete_btn_rect
                        .is_some_and(|r| r.contains(mouse.position))
                    {
                        self.execute_confirm_delete();
                        return EventResult::Consumed;
                    }
                    if layout
                        .confirm_cancel_btn_rect
                        .is_some_and(|r| r.contains(mouse.position))
                    {
                        self.input_mode = ExplorerInputMode::None;
                        return EventResult::Consumed;
                    }
                    if layout
                        .confirm_rect
                        .is_some_and(|r| r.contains(mouse.position))
                    {
                        return EventResult::Consumed;
                    }
                }

                if self.is_input_active() {
                    if layout
                        .inline_input_rect
                        .is_some_and(|r| r.contains(mouse.position))
                    {
                        return EventResult::Consumed;
                    }
                    self.input_mode = ExplorerInputMode::None;
                }

                // Contributed TreeView sections
                for sec in &layout.contrib_sections {
                    if sec.header_rect.contains(mouse.position) {
                        let mut guard = self.contributed_trees.write();
                        if let Some(view) = guard.views.get_mut(&sec.view_id) {
                            view.collapsed = !view.collapsed;
                        }
                        return EventResult::Consumed;
                    }
                    for (node, _depth, _expanded, item_rect) in &sec.items {
                        if item_rect.contains(mouse.position) {
                            let mut pending_expand = None;
                            {
                                let mut guard = self.contributed_trees.write();
                                if let Some(view) = guard.views.get_mut(&sec.view_id) {
                                    view.selected_item_id = Some(node.node_id.clone());
                                    if node.collapsible_state != TreeItemCollapsibleState::None {
                                        if view.expanded_ids.contains(&node.node_id) {
                                            view.expanded_ids.remove(&node.node_id);
                                            view.user_collapsed_ids.insert(node.node_id.clone());
                                        } else {
                                            view.expanded_ids.insert(node.node_id.clone());
                                            view.user_collapsed_ids.remove(&node.node_id);
                                            if !view.loaded_parents.contains(&node.node_id)
                                                && !view.loading_parents.contains(&node.node_id)
                                            {
                                                pending_expand = Some((
                                                    sec.view_id.clone(),
                                                    node.rpc_element_id.clone(),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(cmd) = &node.command {
                                self.last_action = Some(ExplorerAction::ExecuteContributedCommand {
                                    command: cmd.command.clone(),
                                    arguments: cmd.arguments.clone().unwrap_or_default(),
                                });
                            } else if let Some((view_id, element_id)) = pending_expand {
                                self.last_action = Some(ExplorerAction::ExpandContributedItem {
                                    view_id,
                                    element_id,
                                });
                            }
                            return EventResult::Consumed;
                        }
                    }
                }

                // Workspace file tree rows
                for (row, row_rect) in &layout.file_rows {
                    if row_rect.contains(mouse.position) {
                        *self.last_bounds.write() = bounds;
                        self.model.selected_path = Some(row.path.clone());
                        if mouse.button == MouseButton::Left {
                            self.drag_state = Some(ExplorerDragState {
                                source_path: row.path.clone(),
                                is_dir: row.is_dir,
                                start_pos: mouse.position,
                                current_pos: mouse.position,
                                active: false,
                            });
                            if row.is_dir {
                                self.model.toggle_expanded(&row.path);
                                self.last_action = Some(ExplorerAction::ToggleDir(row.path.clone()));
                            } else {
                                self.last_action = Some(ExplorerAction::OpenFile(row.path.clone()));
                            }
                        }
                        return EventResult::Consumed;
                    }
                }

                EventResult::Ignored
            }
            InputEvent::MouseMove(mm) => {
                if let Some(drag) = &mut self.drag_state {
                    drag.current_pos = mm.position;
                    let dx = mm.position.x - drag.start_pos.x;
                    let dy = mm.position.y - drag.start_pos.y;
                    if !drag.active && (dx * dx + dy * dy).sqrt() >= 6.0 {
                        drag.active = true;
                    }
                    if drag.active {
                        let layout = self.compute_layout(bounds, &font);
                        self.hovered_drop_dir = self.resolve_drop_dir_at(&layout, mm.position);
                        return EventResult::Consumed;
                    }
                }
                EventResult::Ignored
            }
            InputEvent::MouseUp(mouse) => {
                if let Some(drag) = self.drag_state.take() {
                    self.hovered_drop_dir = None;
                    if drag.active && mouse.button == MouseButton::Left && bounds.contains(mouse.position)
                    {
                        let layout = self.compute_layout(bounds, &font);
                        if let Some(target_dir) = self.resolve_drop_dir_at(&layout, mouse.position) {
                            let _ = self.move_entry(&drag.source_path, &target_dir);
                        }
                        return EventResult::Consumed;
                    }
                }
                EventResult::Ignored
            }
            InputEvent::KeyDown(key) => {
                // ConfirmDelete modal prompt keys
                if matches!(self.input_mode, ExplorerInputMode::ConfirmDelete { .. }) {
                    match key.key {
                        KeyCode::Enter => {
                            self.execute_confirm_delete();
                        }
                        KeyCode::Escape => {
                            self.input_mode = ExplorerInputMode::None;
                        }
                        _ => {}
                    }
                    return EventResult::Consumed;
                }

                // Inline input keys (NewFile / NewFolder / Rename)
                if !matches!(self.input_mode, ExplorerInputMode::None) {
                    match key.key {
                        KeyCode::Escape => {
                            self.input_mode = ExplorerInputMode::None;
                        }
                        KeyCode::Enter => {
                            self.submit_inline_input();
                        }
                        KeyCode::Backspace => match &mut self.input_mode {
                            ExplorerInputMode::NewFile { buffer, error, .. }
                            | ExplorerInputMode::NewFolder { buffer, error, .. } => {
                                buffer.pop();
                                *error = None;
                            }
                            ExplorerInputMode::Rename {
                                buffer,
                                select_all,
                                error,
                                ..
                            } => {
                                if *select_all {
                                    buffer.clear();
                                    *select_all = false;
                                } else {
                                    buffer.pop();
                                }
                                *error = None;
                            }
                            _ => {}
                        },
                        KeyCode::ArrowLeft
                        | KeyCode::ArrowRight
                        | KeyCode::Home
                        | KeyCode::End => {
                            if let ExplorerInputMode::Rename { select_all, .. } =
                                &mut self.input_mode
                            {
                                *select_all = false;
                            }
                        }
                        KeyCode::Space
                            if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta =>
                        {
                            match &mut self.input_mode {
                                ExplorerInputMode::NewFile { buffer, error, .. }
                                | ExplorerInputMode::NewFolder { buffer, error, .. } => {
                                    buffer.push(' ');
                                    *error = None;
                                }
                                ExplorerInputMode::Rename {
                                    buffer,
                                    select_all,
                                    error,
                                    ..
                                } => {
                                    if *select_all {
                                        buffer.clear();
                                        *select_all = false;
                                    }
                                    buffer.push(' ');
                                    *error = None;
                                }
                                _ => {}
                            }
                        }
                        KeyCode::Char(ch)
                            if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta =>
                        {
                            let append_str = key
                                .text
                                .as_deref()
                                .filter(|s| !s.is_empty())
                                .map(ToString::to_string)
                                .unwrap_or_else(|| ch.to_string());
                            match &mut self.input_mode {
                                ExplorerInputMode::NewFile { buffer, error, .. }
                                | ExplorerInputMode::NewFolder { buffer, error, .. } => {
                                    buffer.push_str(&append_str);
                                    *error = None;
                                }
                                ExplorerInputMode::Rename {
                                    buffer,
                                    select_all,
                                    error,
                                    ..
                                } => {
                                    if *select_all {
                                        buffer.clear();
                                        *select_all = false;
                                    }
                                    buffer.push_str(&append_str);
                                    *error = None;
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                    return EventResult::Consumed;
                }

                // Normal mode Explorer keyboard shortcuts & navigation
                match key.key {
                    KeyCode::F2 => {
                        if self.start_rename() {
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                    KeyCode::Delete => {
                        if self.start_delete() {
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                    KeyCode::ArrowDown => {
                        let rows = self.flattened_rows();
                        if rows.is_empty() {
                            return EventResult::Ignored;
                        }
                        let cur_idx = self.model.selected_path.as_ref().and_then(|sel| {
                            rows.iter()
                                .position(|r| rustcode_platform::paths_equivalent(&r.path, sel))
                        });
                        let next_idx = match cur_idx {
                            Some(i) => (i + 1).min(rows.len() - 1),
                            None => 0,
                        };
                        self.model.selected_path = Some(rows[next_idx].path.clone());
                        EventResult::Consumed
                    }
                    KeyCode::ArrowUp => {
                        let rows = self.flattened_rows();
                        if rows.is_empty() {
                            return EventResult::Ignored;
                        }
                        let cur_idx = self.model.selected_path.as_ref().and_then(|sel| {
                            rows.iter()
                                .position(|r| rustcode_platform::paths_equivalent(&r.path, sel))
                        });
                        let prev_idx = match cur_idx {
                            Some(i) => i.saturating_sub(1),
                            None => 0,
                        };
                        self.model.selected_path = Some(rows[prev_idx].path.clone());
                        EventResult::Consumed
                    }
                    KeyCode::ArrowRight => {
                        let rows = self.flattened_rows();
                        let Some(sel) = &self.model.selected_path else {
                            return EventResult::Ignored;
                        };
                        let Some(idx) = rows
                            .iter()
                            .position(|r| rustcode_platform::paths_equivalent(&r.path, sel))
                        else {
                            return EventResult::Ignored;
                        };
                        if rows[idx].is_dir {
                            if !rows[idx].is_expanded {
                                let p = rows[idx].path.clone();
                                self.model.expanded_paths.insert(p.clone());
                                self.last_action = Some(ExplorerAction::ToggleDir(p));
                            } else if idx + 1 < rows.len() && rows[idx + 1].depth > rows[idx].depth
                            {
                                self.model.selected_path = Some(rows[idx + 1].path.clone());
                            }
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                    KeyCode::ArrowLeft => {
                        let rows = self.flattened_rows();
                        let Some(sel) = &self.model.selected_path else {
                            return EventResult::Ignored;
                        };
                        let Some(idx) = rows
                            .iter()
                            .position(|r| rustcode_platform::paths_equivalent(&r.path, sel))
                        else {
                            return EventResult::Ignored;
                        };
                        if rows[idx].is_dir && rows[idx].is_expanded {
                            let p = rows[idx].path.clone();
                            self.model.expanded_paths.remove(&p);
                            self.last_action = Some(ExplorerAction::ToggleDir(p));
                            EventResult::Consumed
                        } else if rows[idx].depth > 0 {
                            if let Some(parent_row) = rows[..idx]
                                .iter()
                                .rev()
                                .find(|r| r.depth < rows[idx].depth)
                            {
                                self.model.selected_path = Some(parent_row.path.clone());
                            }
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                    KeyCode::Enter => {
                        let rows = self.flattened_rows();
                        let Some(sel) = &self.model.selected_path else {
                            return EventResult::Ignored;
                        };
                        if let Some(row) = rows
                            .iter()
                            .find(|r| rustcode_platform::paths_equivalent(&r.path, sel))
                        {
                            if row.is_dir {
                                let p = row.path.clone();
                                self.model.toggle_expanded(&p);
                                self.last_action = Some(ExplorerAction::ToggleDir(p));
                            } else {
                                self.last_action =
                                    Some(ExplorerAction::OpenFile(row.path.clone()));
                            }
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                    _ => EventResult::Ignored,
                }
            }
            InputEvent::ScrollWheel(scroll) => {
                if !bounds.contains(scroll.position) {
                    return EventResult::Ignored;
                }
                let layout = self.compute_layout(bounds, &font);
                let total_h = (self.flattened_rows().len() as f32) * 22.0;
                let max_scroll = (total_h - layout.file_tree_rect.height()).max(0.0);
                self.scroll_top = (self.scroll_top - scroll.delta_y).clamp(0.0, max_scroll);
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{KeyEvent, MouseButton, MouseEvent};

    #[test]
    fn test_explorer_view_hierarchy_and_click() {
        let mut root = FileTreeNode::new_dir(PathBuf::from("/workspace"));
        let mut src = FileTreeNode::new_dir(PathBuf::from("/workspace/src"));
        src.children = Some(vec![FileTreeNode::new_file(PathBuf::from(
            "/workspace/src/main.rs",
        ))]);
        root.children = Some(vec![
            src,
            FileTreeNode::new_file(PathBuf::from("/workspace/Cargo.toml")),
        ]);

        let mut explorer = FileExplorer::new();
        explorer.set_roots(vec![root]);
        explorer.expanded_paths.insert(PathBuf::from("/workspace"));
        explorer
            .expanded_paths
            .insert(PathBuf::from("/workspace/src"));

        let mut view = FileExplorerView::new(explorer);
        let rows = view.flattened_rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].name, "workspace");
        assert_eq!(rows[1].name, "src");
        assert_eq!(rows[2].name, "main.rs");
        assert_eq!(rows[3].name, "Cargo.toml");

        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let bounds = Rect::from_xywh(0.0, 0.0, 200.0, 600.0);
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("EXPLORER"));
        assert!(scene.contains_text("main.rs"));

        // Click row 2 (main.rs) -> OpenFile action
        let click_y = bounds.min_y() + 28.0 + 2.0 * 22.0 + 5.0;
        let click_ev = InputEvent::MouseDown(MouseEvent::new(
            Point::new(50.0, click_y),
            MouseButton::Left,
        ));
        let res = view.handle_event(&click_ev, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(
            view.last_action,
            Some(ExplorerAction::OpenFile(PathBuf::from(
                "/workspace/src/main.rs"
            )))
        );
    }

    #[test]
    fn test_multi_root_file_explorer() {
        let mut root1 = FileTreeNode::new_dir(PathBuf::from("/backend"));
        root1.children = Some(vec![FileTreeNode::new_file(PathBuf::from(
            "/backend/main.rs",
        ))]);

        let mut root2 = FileTreeNode::new_dir(PathBuf::from("/frontend"));
        root2.children = Some(vec![FileTreeNode::new_file(PathBuf::from(
            "/frontend/index.ts",
        ))]);

        let mut explorer = FileExplorer::new();
        explorer.set_roots(vec![root1, root2]);
        explorer.expanded_paths.insert(PathBuf::from("/backend"));
        explorer.expanded_paths.insert(PathBuf::from("/frontend"));

        let view = FileExplorerView::new(explorer);
        let rows = view.flattened_rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].name, "backend");
        assert_eq!(rows[1].name, "main.rs");
        assert_eq!(rows[2].name, "frontend");
        assert_eq!(rows[3].name, "index.ts");
    }

    #[test]
    fn test_file_explorer_recursive_subdirectory_expansion() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustcode_explorer_test_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let sub_dir = temp_dir.join("src");
        let nested_dir = sub_dir.join("nested");
        std::fs::create_dir_all(&nested_dir).unwrap();
        let main_file = sub_dir.join("main.rs");
        let deep_file = nested_dir.join("deep.rs");
        let root_file = temp_dir.join("Cargo.toml");
        std::fs::write(&main_file, "fn main() {}\n").unwrap();
        std::fs::write(&deep_file, "// deep\n").unwrap();
        std::fs::write(&root_file, "[package]\n").unwrap();

        let root_node = FileTreeNode::from_path(temp_dir.clone());
        assert!(root_node.is_dir);

        let mut explorer = FileExplorer::new();
        explorer.set_roots(vec![root_node]);
        explorer.expanded_paths.insert(temp_dir.clone());
        explorer.expanded_paths.insert(sub_dir.clone());

        let view = FileExplorerView::new(explorer);
        let rows = view.flattened_rows();

        assert!(rows.iter().any(|r| r.name == "src" && r.is_dir));
        assert!(rows.iter().any(|r| r.name == "main.rs" && !r.is_dir));
        assert!(rows.iter().any(|r| r.name == "Cargo.toml" && !r.is_dir));
        assert!(rows.iter().any(|r| r.name == "nested" && r.is_dir));
        assert!(!rows.iter().any(|r| r.name == "deep.rs"));

        let mut explorer2 = view.model.clone();
        explorer2.expanded_paths.insert(nested_dir.clone());
        let view2 = FileExplorerView::new(explorer2);
        let rows2 = view2.flattened_rows();
        assert!(rows2.iter().any(|r| r.name == "deep.rs" && !r.is_dir));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_file_tree_node_root_path_fallback_non_empty() {
        let posix_root_dir = FileTreeNode::new_dir(PathBuf::from("/"));
        assert_eq!(posix_root_dir.name, "/");
        let posix_root_file = FileTreeNode::new_file(PathBuf::from("/"));
        assert_eq!(posix_root_file.name, "/");

        #[cfg(windows)]
        {
            let drive_dir = FileTreeNode::new_dir(PathBuf::from("C:\\"));
            assert_eq!(drive_dir.name, "C:\\");
            let verbatim_drive = FileTreeNode::new_dir(PathBuf::from("\\\\?\\C:\\"));
            assert_eq!(verbatim_drive.name, "C:\\");
            let drive_file = FileTreeNode::new_file(PathBuf::from("D:\\"));
            assert_eq!(drive_file.name, "D:\\");
        }
    }

    #[test]
    fn test_validate_entry_name_rules() {
        assert!(validate_entry_name("main.rs").is_ok());
        assert!(validate_entry_name("my folder").is_ok());
        assert!(validate_entry_name(".gitignore").is_ok());

        for bad in [
            "",
            "   ",
            ".",
            "..",
            "a/b.rs",
            "a\\b.rs",
            "bad\0name",
            "bad<name",
            "a>b",
            "a:b",
            "a\"b",
            "a|b",
            "a?b",
            "a*b",
            "trailing.",
            "trailing ",
            "CON",
            "nul.txt",
        ] {
            assert!(
                validate_entry_name(bad).is_err(),
                "Expected '{}' to be rejected",
                bad
            );
        }
    }

    #[test]
    fn test_explorer_crud_create_rename_delete_cancel_and_keyboard_nav() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustcode_explorer_crud_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(temp_dir.join("alpha.rs"), "// alpha").unwrap();

        let mut explorer = FileExplorer::new();
        explorer.set_roots(vec![FileTreeNode::from_path(temp_dir.clone())]);
        explorer.expanded_paths.insert(temp_dir.clone());
        let mut view = FileExplorerView::new(explorer);
        let bounds = Rect::from_xywh(0.0, 0.0, 280.0, 600.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        // 1. Click New File button, type "beta.rs", press Enter
        let a11y = view.accessibility_nodes(bounds, &font);
        let nf_bounds = a11y
            .iter()
            .find(|n| n.name == "New File")
            .unwrap()
            .bounds;
        assert_eq!(
            view.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(
                    Point::new(
                        nf_bounds.min_x() + nf_bounds.width() * 0.5,
                        nf_bounds.min_y() + nf_bounds.height() * 0.5
                    ),
                    MouseButton::Left
                )),
                bounds
            ),
            EventResult::Consumed
        );
        for ch in "beta.rs".chars() {
            view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert!(temp_dir.join("beta.rs").is_file());
        assert_eq!(
            view.last_action,
            Some(ExplorerAction::CreateFile(temp_dir.join("beta.rs")))
        );

        // 2. Collision check: try creating "alpha.rs", must reject and keep input open
        view.start_new_file();
        for ch in "alpha.rs".chars() {
            view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert!(view.is_input_active());
        assert_eq!(
            std::fs::read_to_string(temp_dir.join("alpha.rs")).unwrap(),
            "// alpha"
        );
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bounds,
        );
        assert!(!view.is_input_active());

        // 3. F2 Rename beta.rs -> gamma.rs (select_all replaces existing name on first char)
        view.model.selected_path = Some(temp_dir.join("beta.rs"));
        assert_eq!(
            view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::F2, Modifiers::none())),
                bounds
            ),
            EventResult::Consumed
        );
        for ch in "gamma.rs".chars() {
            view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert!(!temp_dir.join("beta.rs").exists());
        assert!(temp_dir.join("gamma.rs").exists());
        assert_eq!(
            view.last_action,
            Some(ExplorerAction::Renamed {
                old_path: temp_dir.join("beta.rs"),
                new_path: temp_dir.join("gamma.rs"),
            })
        );

        // 4. Delete confirmation renders Delete/Confirm and deletes on Enter
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Delete, Modifiers::none())),
            bounds,
        );
        assert!(temp_dir.join("gamma.rs").exists());
        let mut confirm_scene = Scene::new();
        view.render(bounds, &theme, &font, &mut confirm_scene);
        assert!(confirm_scene.contains_text("Delete"));
        assert!(confirm_scene.contains_text("Confirm"));

        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert!(!temp_dir.join("gamma.rs").exists());
        assert_eq!(
            view.last_action,
            Some(ExplorerAction::Deleted(temp_dir.join("gamma.rs")))
        );

        // 5. Keyboard navigation ArrowDown + Enter opens alpha.rs
        view.model.selected_path = Some(temp_dir.clone());
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::ArrowDown, Modifiers::none())),
            bounds,
        );
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            view.last_action,
            Some(ExplorerAction::OpenFile(temp_dir.join("alpha.rs")))
        );

        // 6. Tiny viewport clamping (92x158 sidebar slice of 240x180) produces finite non-negative rects
        let tiny_bounds = Rect::from_xywh(48.0, 0.0, 92.0, 158.0);
        let mut tiny_scene = Scene::new();
        view.render(tiny_bounds, &theme, &font, &mut tiny_scene);
        for q in tiny_scene.quads() {
            assert!(q.bounds.min_x().is_finite() && q.bounds.width() >= 0.0);
            assert!(q.bounds.min_y().is_finite() && q.bounds.height() >= 0.0);
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_contributed_treeview_when_clauses_fallback_ids_and_resource_uri_basename() {
        let ctx = ContextKeyService::new();
        ctx.set_bool("workspaceHasProjects", true);

        assert!(evaluate_when_clause(&ctx, None));
        assert!(evaluate_when_clause(&ctx, Some("workspaceHasProjects")));
        assert!(!evaluate_when_clause(&ctx, Some("!workspaceHasProjects")));
        assert!(!evaluate_when_clause(
            &ctx,
            Some("(((invalid && || !!")
        ));
        assert!(!evaluate_when_clause(&ctx, Some("true || (((broken")));

        let no_id_item = TreeItem {
            id: None,
            label: "FallbackItem".to_string(),
            description: None,
            tooltip: None,
            collapsible_state: TreeItemCollapsibleState::None,
            icon_path: None,
            resource_uri: None,
            context_value: None,
            command: None,
        };
        let win_uri_item = TreeItem {
            id: Some("w1".to_string()),
            label: String::new(),
            description: None,
            tooltip: None,
            collapsible_state: TreeItemCollapsibleState::None,
            icon_path: None,
            resource_uri: Some("C:\\Users\\dev\\repo\\src\\win_entry.rs".to_string()),
            context_value: None,
            command: None,
        };

        let mut view_state = ContributedViewState::new(ContributedViewDescriptor {
            id: "outline".to_string(),
            name: "Project Outline".to_string(),
            when: Some("workspaceHasProjects".to_string()),
        });
        view_state.set_root_items(vec![no_id_item, win_uri_item]);
        let flat = view_state.flattened_items();
        assert_eq!(flat.len(), 2);
        assert!(!flat[0].0.node_id.is_empty());
        assert_eq!(flat[0].0.label, "FallbackItem");
        assert_eq!(flat[1].0.label, "win_entry.rs");

        // Windows drive root & POSIX root display-name fallbacks
        let win_root = FileTreeNode::new_dir(PathBuf::from(r"C:\"));
        assert!(!win_root.name.is_empty());
        let verbatim_root = FileTreeNode::new_dir(PathBuf::from(r"\\?\C:\"));
        assert_eq!(verbatim_root.name, r"C:\");
        let posix_root = FileTreeNode::new_dir(PathBuf::from("/"));
        assert_eq!(posix_root.name, "/");
    }

    #[test]
    fn test_evaluate_when_clause_tier5_malformed_and_deep_nesting() {
        let ctx = ContextKeyService::new();
        ctx.set_bool("a", true);
        ctx.set_bool("b", true);

        // Top-level None or empty/whitespace `when` clauses remain unconditional true
        assert!(evaluate_when_clause(&ctx, None));
        assert!(evaluate_when_clause(&ctx, Some("")));
        assert!(evaluate_when_clause(&ctx, Some("   \t ")));

        // Malformed `when` clauses must fail closed (false)
        for bad in [
            "()",
            "(   )",
            "((()))",
            "() && a",
            "nonexistent || ()",
            "a &&|| b",
            "a ||&& b",
            "&&&&",
            "||||",
            "a &&",
            "&& a",
            "a ||",
            "|| a",
            "a && !",
            "!()",
        ] {
            assert!(
                !evaluate_when_clause(&ctx, Some(bad)),
                "malformed when clause {:?} must evaluate to false",
                bad
            );
        }

        // 500 nested groups must fail safely (false) without stack overflow
        let deep_500 = format!("{}a{}", "(a && ".repeat(500), ")".repeat(500));
        assert!(!evaluate_when_clause(&ctx, Some(&deep_500)));

        // Parity between evaluate_when_clause and ContextKeyService::evaluate_expr
        ctx.set_string("slash", "b/c");
        ctx.set_string("op_str", "&& ||");
        ctx.set_string("op_str2", "a &&|| b");
        ctx.set_string("path", "src/foo/bar.rs");
        for expr in [
            "slash == b/c || (a == 'unclosed",
            "op_str == '&& ||'",
            "op_str2 == 'a &&|| b'",
            "path =~ /foo||bar/ && a",
            "a =~ /unclosed",
            "!=b",
            "a & b",
            "a | b",
            "a &&& b",
            "a ||| b",
        ] {
            assert_eq!(
                evaluate_when_clause(&ctx, Some(expr)),
                ctx.evaluate_expr(expr),
                "parity mismatch for {expr:?}"
            );
        }
        assert!(!evaluate_when_clause(&ctx, Some("slash == b/c || (a == 'unclosed")));
        assert!(evaluate_when_clause(&ctx, Some("op_str == '&& ||'")));
        assert!(evaluate_when_clause(&ctx, Some("op_str2 == 'a &&|| b'")));
        assert!(evaluate_when_clause(&ctx, Some("path =~ /foo||bar/ && a")));
    }

    #[test]
    fn test_explorer_drag_and_drop_move_and_cycle_guard() {
        use rustcode_platform::render::{MouseEvent, MouseMoveEvent};
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root_dir = std::env::temp_dir().join(format!("rustcode_explorer_dnd_{suffix}"));
        let sub_a = root_dir.join("dir_a");
        let nested = sub_a.join("nested");
        let sub_b = root_dir.join("dir_b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&sub_b).unwrap();
        let file_x = root_dir.join("x.txt");
        std::fs::write(&file_x, "hello").unwrap();

        let mut explorer = FileExplorer::new();
        explorer.set_roots(vec![FileTreeNode::from_path(root_dir.clone())]);
        explorer.expanded_paths.insert(root_dir.clone());
        explorer.expanded_paths.insert(sub_a.clone());

        let mut view = FileExplorerView::new(explorer);
        let bounds = Rect::from_xywh(0.0, 0.0, 260.0, 500.0);
        let font = FontMetrics::default_monospace();
        let layout = view.compute_layout(bounds, &font);

        let dir_b_rect = layout
            .file_rows
            .iter()
            .find(|(r, _)| rustcode_platform::paths_equivalent(&r.path, &sub_b))
            .unwrap()
            .1;
        let file_x_rect = layout
            .file_rows
            .iter()
            .find(|(r, _)| rustcode_platform::paths_equivalent(&r.path, &file_x))
            .unwrap()
            .1;

        let start = Point::new(file_x_rect.min_x() + 12.0, file_x_rect.min_y() + 8.0);
        let end = Point::new(dir_b_rect.min_x() + 12.0, dir_b_rect.min_y() + 8.0);
        assert_eq!(
            view.handle_event(&InputEvent::MouseDown(MouseEvent::new(start, MouseButton::Left)), bounds),
            EventResult::Consumed
        );
        assert_eq!(
            view.handle_event(&InputEvent::MouseMove(MouseMoveEvent { position: end }), bounds),
            EventResult::Consumed
        );
        assert_eq!(
            view.handle_event(&InputEvent::MouseUp(MouseEvent::new(end, MouseButton::Left)), bounds),
            EventResult::Consumed
        );

        let moved_x = sub_b.join("x.txt");
        assert!(moved_x.exists(), "file should move to dir_b");
        assert!(!file_x.exists());
        assert_eq!(
            view.last_action,
            Some(ExplorerAction::Moved {
                from: file_x.clone(),
                to: moved_x
            })
        );

        // Cycle guard: moving dir_a into its descendant nested must fail
        let err = view.move_entry(&sub_a, &nested).unwrap_err();
        assert!(err.contains("descendant") || err.contains("itself"));
        assert!(sub_a.exists());

        let _ = std::fs::remove_dir_all(&root_dir);
    }
}
