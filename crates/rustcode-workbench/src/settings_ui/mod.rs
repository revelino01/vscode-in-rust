//! Visual Settings UI — mirrors VS Code's `Ctrl+,` Settings editor.
//!
//! Provides a searchable, categorised settings editor with support for
//! bool toggles, string inputs, number fields, enum selects, and JSON objects.

use parking_lot::RwLock;
use rustcode_platform::configuration::{ConfigurationService, ConfigurationTarget};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, KeyEvent, MouseButton, Point,
    Quad, Rect, Scene, View, WorkbenchThemeStyles,
};
use serde_json::Value;
use std::collections::HashMap;

// ── Value type ──────────────────────────────────────────────────────────────

/// The kind of value a setting holds.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsValueType {
    Bool,
    String,
    Number,
    Enum(Vec<String>),
    Object,
}

// ── Category tree ────────────────────────────────────────────────────────────

/// A category node in the left-hand navigation tree.
#[derive(Debug, Clone)]
pub struct SettingsCategory {
    pub id: String,
    pub label: String,
    pub children: Vec<SettingsCategory>,
}

impl SettingsCategory {
    pub fn leaf(id: &str, label: &str) -> Self {
        Self { id: id.to_owned(), label: label.to_owned(), children: vec![] }
    }
    pub fn group(id: &str, label: &str, children: Vec<Self>) -> Self {
        Self { id: id.to_owned(), label: label.to_owned(), children }
    }
}

// ── Settings item ────────────────────────────────────────────────────────────

/// A single setting with its metadata and current value.
#[derive(Debug, Clone)]
pub struct SettingsItem {
    pub key: String,
    pub category: String,
    pub title: String,
    pub description: String,
    pub value_type: SettingsValueType,
    pub current_value: Value,
    pub default_value: Value,
}

impl SettingsItem {
    fn new(
        key: &str,
        category: &str,
        title: &str,
        description: &str,
        vtype: SettingsValueType,
        default: Value,
    ) -> Self {
        Self {
            key: key.to_owned(),
            category: category.to_owned(),
            title: title.to_owned(),
            description: description.to_owned(),
            value_type: vtype,
            current_value: default.clone(),
            default_value: default,
        }
    }
}

// ── Default schema ───────────────────────────────────────────────────────────

/// Returns the built-in settings schema (≥ 20 well-known VS Code settings).
pub fn default_settings_schema() -> Vec<SettingsItem> {
    vec![
        SettingsItem::new(
            "editor.fontSize",
            "Editor",
            "Font Size",
            "Controls the font size in pixels.",
            SettingsValueType::Number,
            Value::from(14),
        ),
        SettingsItem::new(
            "editor.tabSize",
            "Editor",
            "Tab Size",
            "The number of spaces a tab is equal to.",
            SettingsValueType::Number,
            Value::from(4),
        ),
        SettingsItem::new(
            "editor.wordWrap",
            "Editor",
            "Word Wrap",
            "Controls how lines should wrap.",
            SettingsValueType::Enum(vec![
                "off".into(),
                "on".into(),
                "wordWrapColumn".into(),
                "bounded".into(),
            ]),
            Value::from("off"),
        ),
        SettingsItem::new(
            "editor.minimap.enabled",
            "Editor",
            "Minimap: Enabled",
            "Controls whether the minimap is shown.",
            SettingsValueType::Bool,
            Value::from(true),
        ),
        SettingsItem::new(
            "editor.lineNumbers",
            "Editor",
            "Line Numbers",
            "Controls the display of line numbers.",
            SettingsValueType::Enum(vec![
                "off".into(),
                "on".into(),
                "relative".into(),
                "interval".into(),
            ]),
            Value::from("on"),
        ),
        SettingsItem::new(
            "editor.formatOnSave",
            "Editor",
            "Format On Save",
            "Format a file on save.",
            SettingsValueType::Bool,
            Value::from(false),
        ),
        SettingsItem::new(
            "editor.formatOnPaste",
            "Editor",
            "Format On Paste",
            "Controls whether the editor should automatically format pasted content.",
            SettingsValueType::Bool,
            Value::from(false),
        ),
        SettingsItem::new(
            "editor.autoSave",
            "Editor",
            "Auto Save",
            "Controls auto save of editors that have unsaved changes.",
            SettingsValueType::Enum(vec![
                "off".into(),
                "afterDelay".into(),
                "onFocusChange".into(),
                "onWindowChange".into(),
            ]),
            Value::from("off"),
        ),
        SettingsItem::new(
            "editor.cursorStyle",
            "Editor",
            "Cursor Style",
            "Controls the cursor style.",
            SettingsValueType::Enum(vec![
                "block".into(),
                "underline".into(),
                "line".into(),
                "block-outline".into(),
                "underline-thin".into(),
                "line-thin".into(),
            ]),
            Value::from("line"),
        ),
        SettingsItem::new(
            "editor.renderWhitespace",
            "Editor",
            "Render Whitespace",
            "Controls how the editor should render whitespace characters.",
            SettingsValueType::Enum(vec![
                "none".into(),
                "boundary".into(),
                "selection".into(),
                "trailing".into(),
                "all".into(),
            ]),
            Value::from("selection"),
        ),
        SettingsItem::new(
            "workbench.colorTheme",
            "Workbench",
            "Color Theme",
            "Specifies the color theme used in the workbench.",
            SettingsValueType::String,
            Value::from("Default Dark+"),
        ),
        SettingsItem::new(
            "workbench.iconTheme",
            "Workbench",
            "Icon Theme",
            "Specifies the file icon theme used in the workbench.",
            SettingsValueType::String,
            Value::from("vs-seti"),
        ),
        SettingsItem::new(
            "workbench.sideBar.location",
            "Workbench",
            "Sidebar Location",
            "Controls the location of the primary side bar.",
            SettingsValueType::Enum(vec!["left".into(), "right".into()]),
            Value::from("left"),
        ),
        SettingsItem::new(
            "files.autoSave",
            "Files",
            "Auto Save",
            "Controls auto-save of dirty editors.",
            SettingsValueType::Enum(vec![
                "off".into(),
                "afterDelay".into(),
                "onFocusChange".into(),
                "onWindowChange".into(),
            ]),
            Value::from("off"),
        ),
        SettingsItem::new(
            "files.encoding",
            "Files",
            "Encoding",
            "The default character set encoding to use when reading and writing files.",
            SettingsValueType::String,
            Value::from("utf8"),
        ),
        SettingsItem::new(
            "files.eol",
            "Files",
            "EOL",
            "The default end of line character.",
            SettingsValueType::Enum(vec!["\n".into(), "\r\n".into(), "auto".into()]),
            Value::from("auto"),
        ),
        SettingsItem::new(
            "terminal.integrated.fontSize",
            "Terminal",
            "Font Size",
            "Controls the font size in pixels of the terminal.",
            SettingsValueType::Number,
            Value::from(14),
        ),
        SettingsItem::new(
            "terminal.integrated.shell.linux",
            "Terminal",
            "Shell: Linux",
            "The path of the shell that the terminal uses on Linux.",
            SettingsValueType::String,
            Value::from("/bin/bash"),
        ),
        SettingsItem::new(
            "git.enableSmartCommit",
            "Git",
            "Enable Smart Commit",
            "Commit all changes when there are no staged changes.",
            SettingsValueType::Bool,
            Value::from(false),
        ),
        SettingsItem::new(
            "extensions.autoUpdate",
            "Extensions",
            "Auto Update",
            "Controls the automatic update behavior of extensions.",
            SettingsValueType::Enum(vec!["true".into(), "onlyEnabledExtensions".into(), "false".into()]),
            Value::from("true"),
        ),
        SettingsItem::new(
            "window.zoomLevel",
            "Window",
            "Zoom Level",
            "Adjust the zoom level of the window. Positive values zoom in, negative values zoom out.",
            SettingsValueType::Number,
            Value::from(0),
        ),
        SettingsItem::new(
            "breadcrumbs.enabled",
            "Breadcrumbs",
            "Enabled",
            "Enable/disable navigation breadcrumbs.",
            SettingsValueType::Bool,
            Value::from(true),
        ),
        SettingsItem::new(
            "search.exclude",
            "Search",
            "Exclude",
            "Configure glob patterns for excluding files and folders in searches.",
            SettingsValueType::Object,
            serde_json::json!({ "**/node_modules": true, "**/target": true }),
        ),
    ]
}

/// Returns the default category tree for the settings sidebar.
pub fn default_categories() -> Vec<SettingsCategory> {
    vec![
        SettingsCategory::group(
            "editor",
            "Editor",
            vec![
                SettingsCategory::leaf("editor.font", "Font"),
                SettingsCategory::leaf("editor.cursor", "Cursor"),
                SettingsCategory::leaf("editor.minimap", "Minimap"),
                SettingsCategory::leaf("editor.formatting", "Formatting"),
            ],
        ),
        SettingsCategory::group(
            "workbench",
            "Workbench",
            vec![
                SettingsCategory::leaf("workbench.appearance", "Appearance"),
                SettingsCategory::leaf("workbench.layout", "Layout"),
            ],
        ),
        SettingsCategory::leaf("files", "Files"),
        SettingsCategory::leaf("terminal", "Terminal"),
        SettingsCategory::leaf("git", "Git"),
        SettingsCategory::leaf("extensions", "Extensions"),
        SettingsCategory::leaf("window", "Window"),
        SettingsCategory::leaf("search", "Search"),
        SettingsCategory::leaf("breadcrumbs", "Breadcrumbs"),
    ]
}

// ── Action ───────────────────────────────────────────────────────────────────

/// Actions dispatched to the settings UI.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsUiAction {
    Open,
    Close,
    Search(String),
    SelectCategory(String),
    ToggleBool(String, bool),
    SetString(String, String),
    SetNumber(String, f64),
    SetObject(String, Value),
    ResetToDefault(String),
}

/// Active keyboard focus inside the Settings UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsFocus {
    Search,
    ItemInput(String),
    None,
}

/// Hit-test regions recorded during `View::render`.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsHitTarget {
    SearchBar(Rect),
    Category { id: String, rect: Rect },
    ToggleBool { key: String, rect: Rect },
    EnumOption { key: String, value: String, rect: Rect },
    TextInput { key: String, rect: Rect },
    ResetButton { key: String, rect: Rect },
}

// ── State ────────────────────────────────────────────────────────────────────

/// Full UI state for the visual settings editor.
pub struct SettingsUiState {
    pub search_query: String,
    pub selected_category: String,
    pub categories: Vec<SettingsCategory>,
    pub all_items: Vec<SettingsItem>,
    /// Indices into `all_items` that pass the current filter.
    pub filtered_items: Vec<usize>,
    pub is_open: bool,
    /// Pending user edits not yet written to disk (key → value).
    pub pending_writes: HashMap<String, Value>,
}

impl Default for SettingsUiState {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsUiState {
    pub fn new() -> Self {
        let all_items = default_settings_schema();
        let filtered: Vec<usize> = (0..all_items.len()).collect();
        Self {
            search_query: String::new(),
            selected_category: String::new(),
            categories: default_categories(),
            all_items,
            filtered_items: filtered,
            is_open: false,
            pending_writes: HashMap::new(),
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    /// Returns whether `item` belongs to `category_id` (empty / `"all"` matches all items).
    pub fn matches_category(item: &SettingsItem, category_id: &str) -> bool {
        if category_id.is_empty() || category_id.eq_ignore_ascii_case("all") {
            return true;
        }
        match category_id {
            "editor.font" => item.key.starts_with("editor.font"),
            "editor.cursor" => item.key.starts_with("editor.cursor"),
            "editor.minimap" => item.key.starts_with("editor.minimap"),
            "editor.formatting" => item.key.starts_with("editor.format"),
            "workbench.appearance" => {
                matches!(
                    item.key.as_str(),
                    "workbench.colorTheme" | "workbench.iconTheme"
                )
            }
            "workbench.layout" => item.key == "workbench.sideBar.location",
            other => {
                let cat_lower = other.to_lowercase();
                item.category.eq_ignore_ascii_case(&cat_lower)
                    || item
                        .key
                        .to_lowercase()
                        .starts_with(&format!("{}.", cat_lower))
            }
        }
    }

    /// Recomputes `filtered_items` by intersecting `selected_category` and `search_query`.
    pub fn apply_filters(&mut self) {
        let q = self.search_query.to_lowercase();
        let cat = self.selected_category.clone();
        self.filtered_items = self
            .all_items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                if !Self::matches_category(item, &cat) {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                item.key.to_lowercase().contains(&q)
                    || item.title.to_lowercase().contains(&q)
                    || item.description.to_lowercase().contains(&q)
                    || item.category.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();
    }

    /// Fuzzy-filter settings by `query` across key, title, and description.
    pub fn apply_search(&mut self, query: &str) {
        self.search_query = query.to_owned();
        self.apply_filters();
    }

    /// Select a category and refresh `filtered_items`.
    pub fn select_category(&mut self, category: &str) {
        self.selected_category = category.to_owned();
        self.apply_filters();
    }

    /// Returns references to the currently visible settings items.
    pub fn get_filtered(&self) -> Vec<&SettingsItem> {
        self.filtered_items.iter().map(|&i| &self.all_items[i]).collect()
    }

    /// Update the current value of a setting (does not write to disk).
    pub fn update_value(&mut self, key: &str, val: Value) {
        if let Some(item) = self.all_items.iter_mut().find(|i| i.key == key) {
            item.current_value = val.clone();
            self.pending_writes.insert(key.to_owned(), val);
        }
    }

    /// Reset a setting back to its default value.
    pub fn reset_to_default(&mut self, key: &str) {
        if let Some(item) = self.all_items.iter_mut().find(|i| i.key == key) {
            item.current_value = item.default_value.clone();
            self.pending_writes.remove(key);
        }
    }
}

// ── View ─────────────────────────────────────────────────────────────────────

/// GPUI-compatible presenter for the settings editor.
pub struct SettingsUiView {
    pub state: SettingsUiState,
    pub config: Option<ConfigurationService>,
    pub scroll_top: f32,
    pub focus: SettingsFocus,
    pub editing_buffer: String,
    pub last_action: Option<SettingsUiAction>,
    pub hit_targets: RwLock<Vec<SettingsHitTarget>>,
    pub last_content_height: RwLock<f32>,
    last_font: RwLock<FontMetrics>,
}

impl Default for SettingsUiView {
    fn default() -> Self {
        Self::new(SettingsUiState::new())
    }
}

impl SettingsUiView {
    pub fn new(state: SettingsUiState) -> Self {
        Self {
            state,
            config: None,
            scroll_top: 0.0,
            focus: SettingsFocus::None,
            editing_buffer: String::new(),
            last_action: None,
            hit_targets: RwLock::new(Vec::new()),
            last_content_height: RwLock::new(0.0),
            last_font: RwLock::new(FontMetrics::default_monospace()),
        }
    }

    pub fn with_config(mut self, config: ConfigurationService) -> Self {
        self.set_config(config);
        self
    }

    pub fn set_config(&mut self, config: ConfigurationService) {
        self.sync_from_config(&config);
        self.config = Some(config);
    }

    pub fn sync_from_config(&mut self, config: &ConfigurationService) {
        for item in &mut self.state.all_items {
            item.current_value = config
                .get_value(&item.key)
                .filter(|v| !v.is_null())
                .unwrap_or_else(|| item.default_value.clone());
        }
        self.state.pending_writes.clear();
    }

    fn update_and_sync_value(&mut self, key: &str, val: Value) {
        self.state.update_value(key, val.clone());
        if let Some(cfg) = &self.config {
            if cfg.update(key, val, ConfigurationTarget::User).is_ok() {
                self.state.pending_writes.remove(key);
                if let Some(effective) = cfg.get_value(key).filter(|v| !v.is_null()) {
                    if let Some(item) = self.state.all_items.iter_mut().find(|i| i.key == key) {
                        item.current_value = effective;
                    }
                }
            }
        }
    }

    fn compute_total_content_height(&self, bounds: Rect, font: &FontMetrics) -> f32 {
        let (_, list_rect) = compute_list_layout(bounds);
        let list_w = list_rect.width();
        self.state
            .get_filtered()
            .into_iter()
            .map(|item| item_height(item, list_w, font))
            .sum()
    }

    fn commit_editing_buffer(&mut self, key: &str) {
        let vtype = self
            .state
            .all_items
            .iter()
            .find(|i| i.key == key)
            .map(|i| i.value_type.clone());
        let Some(vtype) = vtype else {
            return;
        };

        match vtype {
            SettingsValueType::String => {
                self.handle_action(SettingsUiAction::SetString(
                    key.to_string(),
                    self.editing_buffer.clone(),
                ));
            }
            SettingsValueType::Number => {
                if let Ok(num) = self.editing_buffer.trim().parse::<f64>() {
                    if num.is_finite() {
                        self.handle_action(SettingsUiAction::SetNumber(key.to_string(), num));
                    }
                }
            }
            SettingsValueType::Object => {
                if let Ok(val) = serde_json::from_str::<Value>(&self.editing_buffer) {
                    if val.is_object() {
                        self.handle_action(SettingsUiAction::SetObject(key.to_string(), val));
                    }
                }
            }
            _ => {}
        }
    }

    pub fn handle_action(&mut self, action: SettingsUiAction) {
        self.last_action = Some(action.clone());
        match action {
            SettingsUiAction::Open => {
                self.state.open();
                if let Some(cfg) = self.config.clone() {
                    self.sync_from_config(&cfg);
                }
            }
            SettingsUiAction::Close => self.state.close(),
            SettingsUiAction::Search(q) => {
                self.state.apply_search(&q);
                self.scroll_top = 0.0;
            }
            SettingsUiAction::SelectCategory(cat) => {
                self.state.select_category(&cat);
                self.scroll_top = 0.0;
            }
            SettingsUiAction::ToggleBool(key, val) => {
                self.update_and_sync_value(&key, Value::Bool(val));
            }
            SettingsUiAction::SetString(key, val) => {
                self.update_and_sync_value(&key, Value::String(val));
            }
            SettingsUiAction::SetNumber(key, val) => {
                // Use integer representation when possible for cleaner JSON
                let json_val = if val.fract() == 0.0 && val.abs() < 1e15 {
                    serde_json::json!(val as i64)
                } else {
                    serde_json::json!(val)
                };
                self.update_and_sync_value(&key, json_val);
            }
            SettingsUiAction::SetObject(key, val) => {
                self.update_and_sync_value(&key, val);
            }
            SettingsUiAction::ResetToDefault(key) => {
                self.state.reset_to_default(&key);
                if let Some(cfg) = &self.config {
                    cfg.reset(&key, ConfigurationTarget::User);
                    if let Some(item) = self.state.all_items.iter_mut().find(|i| i.key == key) {
                        item.current_value = cfg
                            .get_value(&key)
                            .filter(|v| !v.is_null())
                            .unwrap_or_else(|| item.default_value.clone());
                    }
                }
            }
        }
    }

    /// Produce a flat list of display lines suitable for diagnostics/text inspection.
    ///
    /// Format:
    /// - `"category:<id>:<label>"` — category header
    /// - `"item:<key>:<title>:<value>:<type>"` — settings row
    /// - `"item:<key>:<title>:<value>:enum:<opt1>,<opt2>..."` — enum row
    pub fn render_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.state.is_open {
            return lines;
        }

        // Header
        lines.push(format!("search:{}", self.state.search_query));

        // Category sidebar entries
        for cat in &self.state.categories {
            lines.push(format!("category:{}:{}", cat.id, cat.label));
        }

        // Settings items
        for item in self.state.get_filtered() {
            let val_str = format_value_for_input(&item.current_value);
            let type_str = match &item.value_type {
                SettingsValueType::Bool => "bool".to_owned(),
                SettingsValueType::String => "string".to_owned(),
                SettingsValueType::Number => "number".to_owned(),
                SettingsValueType::Object => "object".to_owned(),
                SettingsValueType::Enum(opts) => format!("enum:{}", opts.join(",")),
            };
            lines.push(format!("item:{}:{}:{}:{}", item.key, item.title, val_str, type_str));
        }
        lines
    }
}

fn format_value_for_input(val: &Value) -> String {
    match val {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn format_enum_label(opt: &str) -> String {
    match opt {
        "\n" => "\\n".to_string(),
        "\r\n" => "\\r\\n".to_string(),
        other => other.to_string(),
    }
}

fn key_event_to_char(key: &KeyEvent) -> Option<char> {
    if key.modifiers.ctrl || key.modifiers.alt || key.modifiers.meta {
        return None;
    }
    if let Some(ref txt) = key.text {
        let mut chars = txt.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            if !ch.is_control() {
                return Some(ch);
            }
        }
    }
    match key.key {
        KeyCode::Char(c) => Some(if key.modifiers.shift {
            c.to_ascii_uppercase()
        } else {
            c
        }),
        KeyCode::Space => Some(' '),
        _ => None,
    }
}

fn compute_list_layout(bounds: Rect) -> (Rect, Rect) {
    let search_bar_h = 40.0f32;
    let body_y = bounds.min_y() + search_bar_h;
    let body_h = (bounds.height() - search_bar_h).max(0.0);
    let sidebar_w = (bounds.width() * 0.25)
        .clamp(150.0, 200.0)
        .min(bounds.width() * 0.45);
    let sidebar_rect = Rect::from_xywh(bounds.min_x(), body_y, sidebar_w, body_h);
    let list_rect = Rect::from_xywh(
        bounds.min_x() + sidebar_w,
        body_y,
        (bounds.width() - sidebar_w).max(0.0),
        body_h,
    );
    (sidebar_rect, list_rect)
}

fn layout_enum_pills(
    opts: &[String],
    content_x: f32,
    ctrl_y: f32,
    row_max_x: f32,
    ctrl_h: f32,
    font: &FontMetrics,
) -> (Vec<(String, String, Rect)>, usize) {
    let mut placements = Vec::with_capacity(opts.len());
    let mut opt_x = content_x;
    let mut opt_y = ctrl_y;
    let mut rows = 1usize;
    let row_step = ctrl_h + 4.0;
    for opt in opts {
        let disp = format_enum_label(opt);
        let opt_w = (font.text_width(&disp) + 14.0).max(36.0);
        if opt_x > content_x && opt_x + opt_w > row_max_x {
            opt_x = content_x;
            opt_y += row_step;
            rows += 1;
        }
        let opt_rect = Rect::from_xywh(opt_x, opt_y, opt_w, ctrl_h);
        placements.push((opt.clone(), disp, opt_rect));
        opt_x += opt_w + 6.0;
    }
    (placements, rows)
}

fn item_height(item: &SettingsItem, list_width: f32, font: &FontMetrics) -> f32 {
    let base_h = 80.0f32;
    if let SettingsValueType::Enum(opts) = &item.value_type {
        let content_offset = 14.0f32;
        let row_max_x = (list_width - 14.0).max(content_offset);
        let (_, rows) = layout_enum_pills(opts, content_offset, 46.0, row_max_x, 24.0, font);
        base_h + (rows.saturating_sub(1) as f32) * 28.0
    } else {
        base_h
    }
}

impl View for SettingsUiView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !crate::is_finite_positive_rect(bounds)
            || !crate::is_finite_font(font)
            || !(bounds.min_y() + 40.0).is_finite()
        {
            return;
        }
        *self.last_font.write() = font.clone();
        scene.push_quad(Quad::new(bounds).with_background(theme.editor_background));

        let mut targets = Vec::new();

        // 1. Top Search Bar
        let search_bar_h = 40.0f32;
        let search_bar_area =
            Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), search_bar_h);
        scene.push_quad(
            Quad::new(search_bar_area)
                .with_background(theme.editor_background)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        let search_rect = Rect::from_xywh(
            bounds.min_x() + 12.0,
            bounds.min_y() + 6.0,
            (bounds.width() - 24.0).max(0.0),
            28.0,
        );
        let search_focused = self.focus == SettingsFocus::Search;
        let search_border = if search_focused {
            theme.activity_bar_active_border
        } else {
            theme.command_palette_border
        };
        scene.push_quad(
            Quad::new(search_rect)
                .with_background(theme.command_palette_bg)
                .with_border(search_border, 1.0)
                .with_corner_radius(4.0),
        );
        if let Some(clipped_search) = search_rect.intersect(&bounds) {
            targets.push(SettingsHitTarget::SearchBar(clipped_search));
        }

        let search_text_x = search_rect.min_x() + 8.0;
        let search_text_y = search_rect.min_y() + (search_rect.height() - font.line_height) / 2.0;
        if self.state.search_query.is_empty() {
            scene.push_text(GlyphRun::new(
                "Search settings...".to_string(),
                Point::new(search_text_x, search_text_y),
                search_rect,
                theme.tab_inactive_fg,
                font.font_size * 0.9,
            ));
        } else {
            scene.push_text(GlyphRun::new(
                self.state.search_query.clone(),
                Point::new(search_text_x, search_text_y),
                search_rect,
                theme.command_palette_fg,
                font.font_size * 0.9,
            ));
        }

        if search_focused {
            let cursor_x = search_text_x + font.text_width(&self.state.search_query);
            scene.push_quad(
                Quad::new(Rect::from_xywh(
                    cursor_x,
                    search_rect.min_y() + 5.0,
                    1.5,
                    search_rect.height() - 10.0,
                ))
                .with_background(theme.editor_cursor),
            );
        }

        // 2. Body: Left Category Sidebar + Right Settings List
        let (sidebar_rect, list_rect) = compute_list_layout(bounds);
        let body_y = sidebar_rect.min_y();
        let sidebar_w = sidebar_rect.width();

        scene.push_quad(
            Quad::new(sidebar_rect)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.sidebar_border, Edges::new(0.0, 1.0, 0.0, 0.0)),
        );

        scene.push_clip(sidebar_rect);
        let cat_row_h = 24.0f32;
        let mut cat_y = body_y + 6.0;

        let mut flat_cats: Vec<(String, String, f32)> =
            vec![(String::new(), "All Settings".to_string(), 12.0)];
        for cat in &self.state.categories {
            flat_cats.push((cat.id.clone(), cat.label.clone(), 12.0));
            for child in &cat.children {
                flat_cats.push((child.id.clone(), child.label.clone(), 26.0));
            }
        }

        for (id, label, indent) in flat_cats {
            let row_rect = Rect::from_xywh(sidebar_rect.min_x(), cat_y, sidebar_w, cat_row_h);
            if cat_y + cat_row_h <= sidebar_rect.max_y() {
                let is_selected = self.state.selected_category == id;
                if is_selected {
                    scene.push_quad(
                        Quad::new(row_rect).with_background(theme.list_active_selection_bg),
                    );
                }
                let text_color = if is_selected {
                    theme.tab_active_fg
                } else {
                    theme.sidebar_fg
                };
                scene.push_text(GlyphRun::new(
                    label,
                    Point::new(
                        sidebar_rect.min_x() + indent,
                        cat_y + (cat_row_h - font.line_height) / 2.0,
                    ),
                    row_rect,
                    text_color,
                    font.font_size * 0.9,
                ));
                if let Some(clipped_cat) = row_rect.intersect(&sidebar_rect) {
                    targets.push(SettingsHitTarget::Category {
                        id,
                        rect: clipped_cat,
                    });
                }
            }
            cat_y += cat_row_h;
        }
        scene.pop_clip();

        // 3. Right Settings List
        scene.push_clip(list_rect);

        let filtered = self.state.get_filtered();
        let item_heights: Vec<f32> = filtered
            .iter()
            .map(|item| item_height(item, list_rect.width(), font))
            .collect();
        let total_h: f32 = item_heights.iter().sum();
        *self.last_content_height.write() = total_h;

        let max_scroll = (total_h - list_rect.height()).max(0.0);
        let max_scroll = if max_scroll.is_finite() { max_scroll } else { 0.0 };
        let safe_scroll = if self.scroll_top.is_finite() {
            self.scroll_top
        } else {
            0.0
        };
        let clamped_scroll = safe_scroll.clamp(0.0, max_scroll);
        let mut cur_y = list_rect.min_y() - clamped_scroll;

        if filtered.is_empty() {
            scene.push_text(GlyphRun::new(
                "No settings found matching the current filter.".to_string(),
                Point::new(list_rect.min_x() + 16.0, list_rect.min_y() + 16.0),
                list_rect,
                theme.editor_line_number,
                font.font_size * 0.95,
            ));
        } else {
            for (item, item_h) in filtered.into_iter().zip(item_heights) {
                let row_rect = Rect::from_xywh(list_rect.min_x(), cur_y, list_rect.width(), item_h);
                if cur_y + item_h > list_rect.min_y() && cur_y < list_rect.max_y() {
                    let is_modified = item.current_value != item.default_value;
                    if is_modified {
                        scene.push_quad(
                            Quad::new(Rect::from_xywh(
                                row_rect.min_x() + 4.0,
                                cur_y + 6.0,
                                2.0,
                                item_h - 12.0,
                            ))
                            .with_background(theme.activity_bar_active_border),
                        );

                        let reset_w = (font.text_width("Reset") + 14.0).max(48.0);
                        let reset_rect = Rect::from_xywh(
                            row_rect.max_x() - reset_w - 14.0,
                            cur_y + 6.0,
                            reset_w,
                            20.0,
                        );
                        scene.push_quad(
                            Quad::new(reset_rect)
                                .with_background(theme.tab_inactive_bg)
                                .with_border(theme.panel_border, 1.0)
                                .with_corner_radius(3.0),
                        );
                        scene.push_text(GlyphRun::new(
                            "Reset".to_string(),
                            Point::new(reset_rect.min_x() + 7.0, reset_rect.min_y() + 2.0),
                            reset_rect,
                            theme.editor_foreground,
                            font.font_size * 0.85,
                        ));
                        if let Some(clipped_reset) = reset_rect.intersect(&list_rect) {
                            targets.push(SettingsHitTarget::ResetButton {
                                key: item.key.clone(),
                                rect: clipped_reset,
                            });
                        }
                    }

                    let content_x = row_rect.min_x() + 14.0;
                    let title_w = font.text_width(&item.title);
                    scene.push_text(
                        GlyphRun::new(
                            item.title.clone(),
                            Point::new(content_x, cur_y + 6.0),
                            Rect::from_xywh(content_x, cur_y + 6.0, title_w + 4.0, font.line_height),
                            theme.editor_foreground,
                            font.font_size * 0.95,
                        )
                        .with_style(true, false, false),
                    );

                    let key_label = format!("  ({})", item.key);
                    scene.push_text(GlyphRun::new(
                        key_label,
                        Point::new(content_x + title_w + 4.0, cur_y + 6.0),
                        row_rect,
                        theme.editor_line_number,
                        font.font_size * 0.85,
                    ));

                    scene.push_text(GlyphRun::new(
                        item.description.clone(),
                        Point::new(content_x, cur_y + 24.0),
                        row_rect,
                        theme.sidebar_fg,
                        font.font_size * 0.85,
                    ));

                    let ctrl_y = cur_y + 46.0;
                    let ctrl_h = 24.0f32;
                    match &item.value_type {
                        SettingsValueType::Bool => {
                            let checked = item.current_value.as_bool().unwrap_or(false);
                            let label = if checked {
                                "[x] Enabled"
                            } else {
                                "[ ] Disabled"
                            };
                            let pill_w = (font.text_width(label) + 16.0).max(96.0);
                            let pill_rect = Rect::from_xywh(content_x, ctrl_y, pill_w, ctrl_h);
                            let bg = if checked {
                                theme.list_active_selection_bg
                            } else {
                                theme.command_palette_bg
                            };
                            scene.push_quad(
                                Quad::new(pill_rect)
                                    .with_background(bg)
                                    .with_border(theme.command_palette_border, 1.0)
                                    .with_corner_radius(3.0),
                            );
                            scene.push_text(GlyphRun::new(
                                label.to_string(),
                                Point::new(pill_rect.min_x() + 8.0, ctrl_y + 4.0),
                                pill_rect,
                                if checked {
                                    theme.tab_active_fg
                                } else {
                                    theme.editor_foreground
                                },
                                font.font_size * 0.85,
                            ));
                            if let Some(clipped_pill) = pill_rect.intersect(&list_rect) {
                                targets.push(SettingsHitTarget::ToggleBool {
                                    key: item.key.clone(),
                                    rect: clipped_pill,
                                });
                            }
                        }
                        SettingsValueType::Enum(opts) => {
                            let cur_str = item.current_value.as_str().unwrap_or("");
                            let row_max_x = (list_rect.max_x() - 14.0).max(content_x);
                            let (placements, _) = layout_enum_pills(
                                opts, content_x, ctrl_y, row_max_x, ctrl_h, font,
                            );
                            for (opt, disp, opt_rect) in placements {
                                let is_sel = cur_str == opt;
                                let bg = if is_sel {
                                    theme.list_active_selection_bg
                                } else {
                                    theme.command_palette_bg
                                };
                                scene.push_quad(
                                    Quad::new(opt_rect)
                                        .with_background(bg)
                                        .with_border(theme.command_palette_border, 1.0)
                                        .with_corner_radius(3.0),
                                );
                                scene.push_text(GlyphRun::new(
                                    disp,
                                    Point::new(opt_rect.min_x() + 7.0, opt_rect.min_y() + 4.0),
                                    opt_rect,
                                    if is_sel {
                                        theme.tab_active_fg
                                    } else {
                                        theme.editor_foreground
                                    },
                                    font.font_size * 0.85,
                                ));
                                if let Some(clipped_opt) = opt_rect.intersect(&list_rect) {
                                    targets.push(SettingsHitTarget::EnumOption {
                                        key: item.key.clone(),
                                        value: opt,
                                        rect: clipped_opt,
                                    });
                                }
                            }
                        }
                        SettingsValueType::String
                        | SettingsValueType::Number
                        | SettingsValueType::Object => {
                            let is_focused = matches!(
                                &self.focus,
                                SettingsFocus::ItemInput(k) if k == &item.key
                            );
                            let display_val = if is_focused {
                                self.editing_buffer.clone()
                            } else {
                                format_value_for_input(&item.current_value)
                            };
                            let input_w = (row_rect.width() - 28.0).clamp(0.0, 320.0);
                            let input_rect = Rect::from_xywh(content_x, ctrl_y, input_w, ctrl_h);
                            let border = if is_focused {
                                theme.activity_bar_active_border
                            } else {
                                theme.command_palette_border
                            };
                            scene.push_quad(
                                Quad::new(input_rect)
                                    .with_background(theme.command_palette_bg)
                                    .with_border(border, 1.0)
                                    .with_corner_radius(3.0),
                            );
                            scene.push_text(GlyphRun::new(
                                display_val.clone(),
                                Point::new(input_rect.min_x() + 6.0, ctrl_y + 4.0),
                                input_rect,
                                theme.command_palette_fg,
                                font.font_size * 0.9,
                            ));
                            if is_focused {
                                let cursor_x =
                                    input_rect.min_x() + 6.0 + font.text_width(&display_val);
                                scene.push_quad(
                                    Quad::new(Rect::from_xywh(
                                        cursor_x,
                                        input_rect.min_y() + 4.0,
                                        1.5,
                                        ctrl_h - 8.0,
                                    ))
                                    .with_background(theme.editor_cursor),
                                );
                            }
                            if let Some(clipped_input) = input_rect.intersect(&list_rect) {
                                targets.push(SettingsHitTarget::TextInput {
                                    key: item.key.clone(),
                                    rect: clipped_input,
                                });
                            }
                        }
                    }
                }
                cur_y += item_h;
            }
        }

        scene.pop_clip();
        *self.hit_targets.write() = targets;
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::MouseDown(mouse) => {
                if mouse.button != MouseButton::Left || !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }

                let targets = self.hit_targets.read().clone();
                let mut matched_idx = None;
                for (idx, target) in targets.iter().enumerate() {
                    let rect = match target {
                        SettingsHitTarget::SearchBar(r)
                        | SettingsHitTarget::Category { rect: r, .. }
                        | SettingsHitTarget::ToggleBool { rect: r, .. }
                        | SettingsHitTarget::EnumOption { rect: r, .. }
                        | SettingsHitTarget::TextInput { rect: r, .. }
                        | SettingsHitTarget::ResetButton { rect: r, .. } => r,
                    };
                    if rect.contains(mouse.position) {
                        matched_idx = Some(idx);
                        break;
                    }
                }

                // Commit active item input on blur (unless clicking the same input or its Reset button)
                if let SettingsFocus::ItemInput(active_key) = self.focus.clone() {
                    let same_or_reset = matched_idx.is_some_and(|i| match &targets[i] {
                        SettingsHitTarget::TextInput { key, .. } => key == &active_key,
                        SettingsHitTarget::ResetButton { key, .. } => key == &active_key,
                        _ => false,
                    });
                    if !same_or_reset {
                        self.commit_editing_buffer(&active_key);
                    }
                }

                let Some(idx) = matched_idx else {
                    self.focus = SettingsFocus::None;
                    return EventResult::Consumed;
                };

                match &targets[idx] {
                    SettingsHitTarget::SearchBar(_) => {
                        self.focus = SettingsFocus::Search;
                    }
                    SettingsHitTarget::Category { id, .. } => {
                        self.focus = SettingsFocus::None;
                        self.handle_action(SettingsUiAction::SelectCategory(id.clone()));
                    }
                    SettingsHitTarget::ToggleBool { key, .. } => {
                        self.focus = SettingsFocus::None;
                        let cur = self
                            .state
                            .all_items
                            .iter()
                            .find(|i| i.key == *key)
                            .and_then(|i| i.current_value.as_bool())
                            .unwrap_or(false);
                        self.handle_action(SettingsUiAction::ToggleBool(key.clone(), !cur));
                    }
                    SettingsHitTarget::EnumOption { key, value, .. } => {
                        self.focus = SettingsFocus::None;
                        self.handle_action(SettingsUiAction::SetString(
                            key.clone(),
                            value.clone(),
                        ));
                    }
                    SettingsHitTarget::TextInput { key, .. } => {
                        let initial = self
                            .state
                            .all_items
                            .iter()
                            .find(|i| i.key == *key)
                            .map(|i| format_value_for_input(&i.current_value))
                            .unwrap_or_default();
                        self.editing_buffer = initial;
                        self.focus = SettingsFocus::ItemInput(key.clone());
                    }
                    SettingsHitTarget::ResetButton { key, .. } => {
                        if matches!(&self.focus, SettingsFocus::ItemInput(k) if k == key) {
                            self.focus = SettingsFocus::None;
                            self.editing_buffer.clear();
                        }
                        self.handle_action(SettingsUiAction::ResetToDefault(key.clone()));
                    }
                }
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => match self.focus.clone() {
                SettingsFocus::Search => match key.key {
                    KeyCode::Escape => {
                        self.handle_action(SettingsUiAction::Search(String::new()));
                        EventResult::Consumed
                    }
                    KeyCode::Backspace => {
                        let mut q = self.state.search_query.clone();
                        q.pop();
                        self.handle_action(SettingsUiAction::Search(q));
                        EventResult::Consumed
                    }
                    _ => {
                        if let Some(ch) = key_event_to_char(key) {
                            let mut q = self.state.search_query.clone();
                            q.push(ch);
                            self.handle_action(SettingsUiAction::Search(q));
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                },
                SettingsFocus::ItemInput(active_key) => match key.key {
                    KeyCode::Enter => {
                        self.commit_editing_buffer(&active_key);
                        self.focus = SettingsFocus::None;
                        EventResult::Consumed
                    }
                    KeyCode::Escape => {
                        self.editing_buffer.clear();
                        self.focus = SettingsFocus::None;
                        EventResult::Consumed
                    }
                    KeyCode::Backspace => {
                        self.editing_buffer.pop();
                        EventResult::Consumed
                    }
                    _ => {
                        if let Some(ch) = key_event_to_char(key) {
                            self.editing_buffer.push(ch);
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                },
                SettingsFocus::None => EventResult::Ignored,
            },
            InputEvent::ScrollWheel(scroll) => {
                if !crate::is_finite_positive_rect(bounds)
                    || !scroll.position.x.is_finite()
                    || !scroll.position.y.is_finite()
                    || !bounds.contains(scroll.position)
                {
                    return EventResult::Ignored;
                }
                let (_, list_rect) = compute_list_layout(bounds);
                let font = self.last_font.read().clone();
                let total_h = self.compute_total_content_height(bounds, &font);
                let safe_total_h = if total_h.is_finite() { total_h } else { 0.0 };
                *self.last_content_height.write() = safe_total_h;
                let max_scroll = (safe_total_h - list_rect.height()).max(0.0);
                let max_scroll = if max_scroll.is_finite() { max_scroll } else { 0.0 };
                let safe_scroll = if self.scroll_top.is_finite() {
                    self.scroll_top
                } else {
                    0.0
                };
                if !scroll.delta_y.is_finite() {
                    self.scroll_top = safe_scroll.clamp(0.0, max_scroll);
                    return EventResult::Ignored;
                }
                let raw = safe_scroll - scroll.delta_y;
                self.scroll_top = if raw.is_nan() {
                    0.0
                } else {
                    raw.clamp(0.0, max_scroll)
                };
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_ui_open_close() {
        let mut state = SettingsUiState::new();
        assert!(!state.is_open);
        state.open();
        assert!(state.is_open);
        state.close();
        assert!(!state.is_open);
    }

    #[test]
    fn test_settings_ui_search_font() {
        let mut state = SettingsUiState::new();
        state.apply_search("font");
        let filtered = state.get_filtered();
        assert!(!filtered.is_empty(), "should find font-related settings");
        for item in &filtered {
            let combined = format!("{} {} {}", item.key, item.title, item.description).to_lowercase();
            assert!(combined.contains("font"), "item '{}' should match 'font'", item.key);
        }
    }

    #[test]
    fn test_settings_ui_search_empty_shows_all() {
        let mut state = SettingsUiState::new();
        state.apply_search("font");
        state.apply_search("");
        assert_eq!(state.filtered_items.len(), state.all_items.len());
    }

    #[test]
    fn test_settings_ui_update_value() {
        let mut state = SettingsUiState::new();
        state.update_value("editor.fontSize", Value::from(16));
        let item = state.all_items.iter().find(|i| i.key == "editor.fontSize").unwrap();
        assert_eq!(item.current_value, Value::from(16));
    }

    #[test]
    fn test_settings_ui_reset_to_default() {
        let mut state = SettingsUiState::new();
        // Change font size
        state.update_value("editor.fontSize", Value::from(20));
        // Reset it
        state.reset_to_default("editor.fontSize");
        let item = state.all_items.iter().find(|i| i.key == "editor.fontSize").unwrap();
        assert_eq!(item.current_value, item.default_value);
        assert!(!state.pending_writes.contains_key("editor.fontSize"));
    }

    #[test]
    fn test_settings_ui_default_schema_count() {
        let schema = default_settings_schema();
        assert!(schema.len() >= 20, "expected ≥ 20 settings, got {}", schema.len());
    }

    #[test]
    fn test_settings_ui_categories_not_empty() {
        let cats = default_categories();
        assert!(!cats.is_empty());
        let labels: Vec<&str> = cats.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"Editor"), "Editor category missing");
        assert!(labels.contains(&"Workbench"), "Workbench category missing");
    }

    #[test]
    fn test_settings_ui_render_when_open() {
        let mut view = SettingsUiView::new(SettingsUiState::new());
        // Closed → empty
        assert!(view.render_lines().is_empty());
        view.handle_action(SettingsUiAction::Open);
        let lines = view.render_lines();
        assert!(!lines.is_empty(), "render should produce lines when open");
        assert!(lines.iter().any(|l| l.starts_with("item:")));
        assert!(lines.iter().any(|l| l.starts_with("category:")));
    }

    #[test]
    fn test_settings_ui_enum_type_word_wrap() {
        let schema = default_settings_schema();
        let wrap = schema.iter().find(|i| i.key == "editor.wordWrap").unwrap();
        match &wrap.value_type {
            SettingsValueType::Enum(opts) => {
                assert!(opts.contains(&"on".to_owned()));
                assert!(opts.contains(&"off".to_owned()));
                assert!(opts.contains(&"wordWrapColumn".to_owned()));
            }
            other => panic!("Expected Enum, got {:?}", other),
        }
    }

    #[test]
    fn test_settings_ui_toggle_bool_action() {
        let mut view = SettingsUiView::new(SettingsUiState::new());
        view.handle_action(SettingsUiAction::ToggleBool("editor.formatOnSave".into(), true));
        let item = view.state.all_items.iter().find(|i| i.key == "editor.formatOnSave").unwrap();
        assert_eq!(item.current_value, Value::Bool(true));
    }

    #[test]
    fn test_settings_ui_set_string_action() {
        let mut view = SettingsUiView::new(SettingsUiState::new());
        view.handle_action(SettingsUiAction::SetString(
            "workbench.colorTheme".into(),
            "Monokai".into(),
        ));
        let item = view.state.all_items.iter().find(|i| i.key == "workbench.colorTheme").unwrap();
        assert_eq!(item.current_value, Value::String("Monokai".into()));
    }

    #[test]
    fn test_settings_ui_set_number_action() {
        let mut view = SettingsUiView::new(SettingsUiState::new());
        view.handle_action(SettingsUiAction::SetNumber("window.zoomLevel".into(), 2.0));
        let item = view.state.all_items.iter().find(|i| i.key == "window.zoomLevel").unwrap();
        assert_eq!(item.current_value, Value::from(2i64));
    }

    #[test]
    fn test_settings_ui_pending_writes_tracking() {
        let mut state = SettingsUiState::new();
        state.update_value("editor.fontSize", Value::from(18));
        assert!(state.pending_writes.contains_key("editor.fontSize"));
        state.reset_to_default("editor.fontSize");
        assert!(!state.pending_writes.contains_key("editor.fontSize"));
    }

    #[test]
    fn test_settings_ui_category_and_search_filtering() {
        let mut view = SettingsUiView::default();

        // Top-level category: editor (10 items)
        view.handle_action(SettingsUiAction::SelectCategory("editor".into()));
        let editor_items: Vec<_> = view
            .state
            .get_filtered()
            .into_iter()
            .map(|i| i.key.as_str())
            .collect();
        assert_eq!(editor_items.len(), 10);
        assert!(editor_items.contains(&"editor.fontSize"));

        // Combined category + search
        view.handle_action(SettingsUiAction::Search("format".into()));
        let editor_format: Vec<_> = view
            .state
            .get_filtered()
            .into_iter()
            .map(|i| i.key.as_str())
            .collect();
        assert_eq!(
            editor_format,
            vec!["editor.formatOnSave", "editor.formatOnPaste"]
        );

        // Clear search while category remains selected
        view.handle_action(SettingsUiAction::Search(String::new()));
        assert_eq!(view.state.get_filtered().len(), 10);

        // Nested categories
        view.handle_action(SettingsUiAction::SelectCategory("editor.font".into()));
        assert_eq!(
            view.state
                .get_filtered()
                .iter()
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>(),
            vec!["editor.fontSize"]
        );

        view.handle_action(SettingsUiAction::SelectCategory("editor.cursor".into()));
        assert_eq!(
            view.state
                .get_filtered()
                .iter()
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>(),
            vec!["editor.cursorStyle"]
        );

        view.handle_action(SettingsUiAction::SelectCategory("editor.minimap".into()));
        assert_eq!(
            view.state
                .get_filtered()
                .iter()
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>(),
            vec!["editor.minimap.enabled"]
        );

        view.handle_action(SettingsUiAction::SelectCategory("editor.formatting".into()));
        assert_eq!(
            view.state
                .get_filtered()
                .iter()
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>(),
            vec!["editor.formatOnSave", "editor.formatOnPaste"]
        );

        view.handle_action(SettingsUiAction::SelectCategory(
            "workbench.appearance".into(),
        ));
        assert_eq!(
            view.state
                .get_filtered()
                .iter()
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>(),
            vec!["workbench.colorTheme", "workbench.iconTheme"]
        );

        view.handle_action(SettingsUiAction::SelectCategory("workbench.layout".into()));
        assert_eq!(
            view.state
                .get_filtered()
                .iter()
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>(),
            vec!["workbench.sideBar.location"]
        );

        // "all" / "" returns all items
        view.handle_action(SettingsUiAction::SelectCategory("all".into()));
        assert_eq!(view.state.get_filtered().len(), view.state.all_items.len());
    }

    #[test]
    fn test_settings_ui_gpui_view_render_and_interactions() {
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::{MouseEvent, ScrollEvent};

        let mut view = SettingsUiView::default();
        view.handle_action(SettingsUiAction::Open);

        let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 600.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("Search settings..."));
        assert!(scene.contains_text("All Settings"));
        assert!(scene.contains_text("Font Size"));
        assert!(!scene.contains_text("Reset"));

        // 1. Click Search bar and type "minimap"
        let search_rect = view
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::SearchBar(r) => Some(*r),
                _ => None,
            })
            .unwrap();
        let res = view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(search_rect.min_x() + 10.0, search_rect.min_y() + 10.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(view.focus, SettingsFocus::Search);

        for ch in "minimap".chars() {
            let res = view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
            assert_eq!(res, EventResult::Consumed);
        }
        assert_eq!(view.state.search_query, "minimap");
        assert_eq!(view.state.get_filtered().len(), 1);

        // Render filtered view -> click bool toggle for editor.minimap.enabled
        let mut scene2 = Scene::new();
        view.render(bounds, &theme, &font, &mut scene2);
        assert!(scene2.contains_text("[x] Enabled"));

        let toggle_rect = view
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::ToggleBool { key, rect } if key == "editor.minimap.enabled" => {
                    Some(*rect)
                }
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(toggle_rect.min_x() + 5.0, toggle_rect.min_y() + 5.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(
            view.state
                .all_items
                .iter()
                .find(|i| i.key == "editor.minimap.enabled")
                .unwrap()
                .current_value,
            Value::Bool(false)
        );

        // Render modified view -> Reset button appears; click Reset
        let mut scene3 = Scene::new();
        view.render(bounds, &theme, &font, &mut scene3);
        assert!(scene3.contains_text("Reset"));
        assert!(scene3.contains_text("[ ] Disabled"));

        let reset_rect = view
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::ResetButton { key, rect }
                    if key == "editor.minimap.enabled" =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(reset_rect.min_x() + 5.0, reset_rect.min_y() + 5.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(
            view.state
                .all_items
                .iter()
                .find(|i| i.key == "editor.minimap.enabled")
                .unwrap()
                .current_value,
            Value::Bool(true)
        );

        // Focus search & Escape clears search
        view.focus = SettingsFocus::Search;
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bounds,
        );
        assert!(view.state.search_query.is_empty());

        // Click category "workbench.layout" and click enum option "right"
        let mut scene4 = Scene::new();
        view.render(bounds, &theme, &font, &mut scene4);
        let layout_cat_rect = view
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::Category { id, rect } if id == "workbench.layout" => {
                    Some(*rect)
                }
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(layout_cat_rect.min_x() + 5.0, layout_cat_rect.min_y() + 5.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(view.state.selected_category, "workbench.layout");

        let mut scene5 = Scene::new();
        view.render(bounds, &theme, &font, &mut scene5);
        let right_enum_rect = view
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::EnumOption { key, value, rect }
                    if key == "workbench.sideBar.location" && value == "right" =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(right_enum_rect.min_x() + 4.0, right_enum_rect.min_y() + 4.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(
            view.state
                .all_items
                .iter()
                .find(|i| i.key == "workbench.sideBar.location")
                .unwrap()
                .current_value,
            Value::String("right".into())
        );

        // Reset to All Settings and test ScrollWheel clamping (-delta_y scrolls down)
        view.handle_action(SettingsUiAction::SelectCategory(String::new()));
        let mut scene6 = Scene::new();
        view.render(bounds, &theme, &font, &mut scene6);
        let total_h = *view.last_content_height.read();
        let max_scroll = (total_h - (bounds.height() - 40.0)).max(0.0);
        assert!(max_scroll > 0.0);

        view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(400.0, 200.0), 0.0, -99999.0)),
            bounds,
        );
        assert_eq!(view.scroll_top, max_scroll);

        view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(400.0, 200.0), 0.0, 99999.0)),
            bounds,
        );
        assert_eq!(view.scroll_top, 0.0);
    }

    #[test]
    fn test_settings_ui_text_number_object_editing_and_config_sync() {
        use rustcode_platform::keybindings::Modifiers;
        use rustcode_platform::render::MouseEvent;

        let config = ConfigurationService::new();
        config
            .update("editor.fontSize", Value::from(18), ConfigurationTarget::User)
            .unwrap();

        let mut view = SettingsUiView::default().with_config(config.clone());
        assert_eq!(
            view.state
                .all_items
                .iter()
                .find(|i| i.key == "editor.fontSize")
                .unwrap()
                .current_value,
            Value::from(18)
        );

        let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 2200.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        // 1. Edit Number ("editor.fontSize") -> backspace twice, type "24", press Enter
        let font_input = view
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::TextInput { key, rect } if key == "editor.fontSize" => {
                    Some(*rect)
                }
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(font_input.min_x() + 5.0, font_input.min_y() + 5.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(
            view.focus,
            SettingsFocus::ItemInput("editor.fontSize".into())
        );
        assert_eq!(view.editing_buffer, "18");

        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Backspace, Modifiers::none())),
            bounds,
        );
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Backspace, Modifiers::none())),
            bounds,
        );
        for ch in "24".chars() {
            view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(view.focus, SettingsFocus::None);
        assert_eq!(config.get::<u32>("editor.fontSize"), Some(24));
        assert!(!view.state.pending_writes.contains_key("editor.fontSize"));

        // 2. Edit String ("workbench.colorTheme") and commit on blur (clicking SearchBar)
        let theme_input = view
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::TextInput { key, rect } if key == "workbench.colorTheme" => {
                    Some(*rect)
                }
                _ => None,
            })
            .unwrap();
        let search_rect = view
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::SearchBar(r) => Some(*r),
                _ => None,
            })
            .unwrap();

        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(theme_input.min_x() + 5.0, theme_input.min_y() + 5.0),
                MouseButton::Left,
            )),
            bounds,
        );
        while !view.editing_buffer.is_empty() {
            view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Backspace, Modifiers::none())),
                bounds,
            );
        }
        for ch in "Light+".chars() {
            view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
        }
        // Blur commit by clicking search bar
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(search_rect.min_x() + 5.0, search_rect.min_y() + 5.0),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(
            config.get::<String>("workbench.colorTheme"),
            Some("Light+".into())
        );

        // 3. Edit Object ("search.exclude") via keyboard + Enter
        let search_excl_input = view
            .hit_targets
            .read()
            .iter()
            .find_map(|t| match t {
                SettingsHitTarget::TextInput { key, rect } if key == "search.exclude" => {
                    Some(*rect)
                }
                _ => None,
            })
            .unwrap();
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(
                    search_excl_input.min_x() + 5.0,
                    search_excl_input.min_y() + 5.0,
                ),
                MouseButton::Left,
            )),
            bounds,
        );
        view.editing_buffer = r#"{"**/dist": true}"#.to_string();
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        let excl = config.get_value("search.exclude").unwrap();
        assert_eq!(excl, serde_json::json!({ "**/dist": true }));
        let item_excl = view
            .state
            .all_items
            .iter()
            .find(|i| i.key == "search.exclude")
            .unwrap()
            .current_value
            .clone();
        assert_eq!(item_excl, excl);

        // 4. Escape cancels edit without changing config
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(font_input.min_x() + 5.0, font_input.min_y() + 5.0),
                MouseButton::Left,
            )),
            bounds,
        );
        view.editing_buffer = "99".to_string();
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Escape, Modifiers::none())),
            bounds,
        );
        assert_eq!(config.get::<u32>("editor.fontSize"), Some(24));

        // 5. ResetToDefault syncs reset back to ConfigurationService
        view.handle_action(SettingsUiAction::ResetToDefault("editor.fontSize".into()));
        assert_eq!(config.get::<u32>("editor.fontSize"), Some(14));
    }

    #[test]
    fn test_settings_ui_object_replacement_survives_reopen_and_reset() {
        use rustcode_platform::keybindings::Modifiers;

        let config = ConfigurationService::new();
        let mut view = SettingsUiView::default().with_config(config.clone());
        view.handle_action(SettingsUiAction::Open);

        view.focus = SettingsFocus::ItemInput("search.exclude".into());
        view.editing_buffer = r#"{"**/dist": true}"#.to_string();
        let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 600.0);
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );

        let expected = serde_json::json!({ "**/dist": true });
        assert_eq!(config.get_value("search.exclude"), Some(expected.clone()));
        assert_eq!(
            view.state
                .all_items
                .iter()
                .find(|i| i.key == "search.exclude")
                .unwrap()
                .current_value,
            expected
        );

        // Reopening a fresh SettingsUiView (or dispatching Open) preserves the exact replacement
        let mut reopened = SettingsUiView::default().with_config(config.clone());
        reopened.handle_action(SettingsUiAction::Open);
        assert_eq!(
            reopened
                .state
                .all_items
                .iter()
                .find(|i| i.key == "search.exclude")
                .unwrap()
                .current_value,
            expected
        );

        // Resetting restores the defaults cleanly
        reopened.handle_action(SettingsUiAction::ResetToDefault("search.exclude".into()));
        let defaults = serde_json::json!({ "**/node_modules": true, "**/target": true });
        assert_eq!(config.get_value("search.exclude"), Some(defaults.clone()));
        assert_eq!(
            reopened
                .state
                .all_items
                .iter()
                .find(|i| i.key == "search.exclude")
                .unwrap()
                .current_value,
            defaults
        );
    }

    #[test]
    fn test_settings_ui_clipped_hit_targets_when_scrolled_and_offset() {
        use rustcode_platform::render::MouseEvent;

        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for origin in [(0.0, 0.0), (50.0, 30.0)] {
            let bounds = Rect::from_xywh(origin.0, origin.1, 1000.0, 700.0);
            let (sidebar_rect, list_rect) = compute_list_layout(bounds);

            let mut view = SettingsUiView::default();
            view.handle_action(SettingsUiAction::Open);
            view.scroll_top = 55.0;

            let mut scene = Scene::new();
            view.render(bounds, &theme, &font, &mut scene);

            for target in view.hit_targets.read().iter() {
                match target {
                    SettingsHitTarget::SearchBar(r) => {
                        assert!(r.min_x() >= bounds.min_x() && r.max_x() <= bounds.max_x());
                        assert!(r.min_y() >= bounds.min_y() && r.max_y() <= bounds.max_y());
                    }
                    SettingsHitTarget::Category { rect: r, .. } => {
                        assert!(r.min_x() >= sidebar_rect.min_x() && r.max_x() <= sidebar_rect.max_x());
                        assert!(r.min_y() >= sidebar_rect.min_y() && r.max_y() <= sidebar_rect.max_y());
                    }
                    SettingsHitTarget::ToggleBool { rect: r, .. }
                    | SettingsHitTarget::EnumOption { rect: r, .. }
                    | SettingsHitTarget::TextInput { rect: r, .. }
                    | SettingsHitTarget::ResetButton { rect: r, .. } => {
                        assert!(
                            r.min_x() >= list_rect.min_x() && r.max_x() <= list_rect.max_x(),
                            "target {:?} overflowed horizontally: list_rect={:?}",
                            target,
                            list_rect
                        );
                        assert!(
                            r.min_y() >= list_rect.min_y() && r.max_y() <= list_rect.max_y(),
                            "target {:?} leaked vertically: list_rect={:?}",
                            target,
                            list_rect
                        );
                    }
                }
            }

            // Clicking in search bar padding (y = origin.1 + 36.0) must NOT hit scrolled-off editor.fontSize
            view.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(
                    Point::new(origin.0 + 260.0, origin.1 + 36.0),
                    MouseButton::Left,
                )),
                bounds,
            );
            assert_eq!(view.focus, SettingsFocus::None);
        }
    }

    #[test]
    fn test_settings_ui_narrow_bounds_wraps_enum_pills_within_bounds() {
        use rustcode_platform::render::MouseEvent;

        let bounds = Rect::from_xywh(0.0, 0.0, 320.0, 200.0);
        let (_, list_rect) = compute_list_layout(bounds);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let mut view = SettingsUiView::default();
        view.handle_action(SettingsUiAction::Open);
        view.handle_action(SettingsUiAction::SelectCategory("editor.cursor".into()));

        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        let enum_targets: Vec<(String, Rect)> = view
            .hit_targets
            .read()
            .iter()
            .filter_map(|t| match t {
                SettingsHitTarget::EnumOption { key, value, rect } if key == "editor.cursorStyle" => {
                    Some((value.clone(), *rect))
                }
                _ => None,
            })
            .collect();

        assert_eq!(enum_targets.len(), 6);
        let mut distinct_y = Vec::new();
        for (val, rect) in &enum_targets {
            assert!(
                rect.min_x() >= list_rect.min_x() && rect.max_x() <= bounds.max_x(),
                "enum pill '{}' overflowed narrow bounds: {:?}",
                val,
                rect
            );
            assert!(
                rect.min_y() >= list_rect.min_y() && rect.max_y() <= bounds.max_y(),
                "enum pill '{}' overflowed vertically: {:?}",
                val,
                rect
            );
            if !distinct_y.iter().any(|&y: &f32| (y - rect.min_y()).abs() < 0.5) {
                distinct_y.push(rect.min_y());
            }
        }
        assert!(distinct_y.len() > 1, "expected enum pills to wrap across multiple rows");

        // Clicking a wrapped pill ("block-outline" on row 3, and "line-thin" at clipped center) works accurately
        let outline_rect = enum_targets
            .iter()
            .find(|(v, _)| v == "block-outline")
            .unwrap()
            .1;
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(
                    (outline_rect.min_x() + outline_rect.max_x()) * 0.5,
                    (outline_rect.min_y() + outline_rect.max_y()) * 0.5,
                ),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(
            view.state
                .all_items
                .iter()
                .find(|i| i.key == "editor.cursorStyle")
                .unwrap()
                .current_value,
            Value::String("block-outline".into())
        );

        let line_thin_rect = enum_targets
            .iter()
            .find(|(v, _)| v == "line-thin")
            .unwrap()
            .1;
        view.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(
                Point::new(
                    (line_thin_rect.min_x() + line_thin_rect.max_x()) * 0.5,
                    (line_thin_rect.min_y() + line_thin_rect.max_y()) * 0.5,
                ),
                MouseButton::Left,
            )),
            bounds,
        );
        assert_eq!(
            view.state
                .all_items
                .iter()
                .find(|i| i.key == "editor.cursorStyle")
                .unwrap()
                .current_value,
            Value::String("line-thin".into())
        );
    }

    #[test]
    fn test_settings_ui_scroll_wheel_clamps_after_filter_before_render() {
        use rustcode_platform::render::ScrollEvent;

        let bounds = Rect::from_xywh(0.0, 0.0, 1000.0, 700.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let mut view = SettingsUiView::default();
        view.handle_action(SettingsUiAction::Open);

        // Initial render populates last_content_height for all 23 items (1840.0)
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);
        assert_eq!(*view.last_content_height.read(), 23.0 * 80.0);

        // Filter down to 1 item and scroll BEFORE calling render() again
        view.handle_action(SettingsUiAction::Search("minimap".into()));
        assert_eq!(view.state.get_filtered().len(), 1);

        let res = view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(500.0, 300.0), 0.0, -10000.0)),
            bounds,
        );
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(view.scroll_top, 0.0);

        // Also verify SelectCategory("editor") clamps to 10 items (800 - 660 = 140) before render()
        view.handle_action(SettingsUiAction::Search(String::new()));
        view.handle_action(SettingsUiAction::SelectCategory("editor".into()));
        view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(500.0, 300.0), 0.0, -10000.0)),
            bounds,
        );
        assert_eq!(view.scroll_top, 10.0 * 80.0 - 660.0);
    }

    #[test]
    fn test_settings_ui_non_finite_bounds_and_nan_scrollwheel_no_poison() {
        use rustcode_platform::render::{ScrollEvent, Size};

        let mut view = SettingsUiView::default();
        view.handle_action(SettingsUiAction::Open);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for bad_bounds in [
            Rect::new(Point::zero(), Size::new(f32::NAN, 600.0)),
            Rect::new(Point::new(f32::NAN, f32::NAN), Size::new(800.0, 600.0)),
            Rect::from_xywh(0.0, 0.0, 800.0, f32::INFINITY),
        ] {
            let mut scene = Scene::new();
            view.render(bad_bounds, &theme, &font, &mut scene);
            assert!(scene.nodes().is_empty());
        }

        let valid_bounds = Rect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let mut init_scene = Scene::new();
        view.render(valid_bounds, &theme, &font, &mut init_scene);
        assert!(init_scene.contains_text("Font Size"));

        view.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(400.0, 300.0), 0.0, f32::NAN)),
            valid_bounds,
        );
        assert!(view.scroll_top.is_finite());

        let mut after_scene = Scene::new();
        view.render(valid_bounds, &theme, &font, &mut after_scene);
        assert!(after_scene.contains_text("Font Size"));
    }
}
