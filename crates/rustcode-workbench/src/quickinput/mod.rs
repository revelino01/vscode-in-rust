//! Quick Input, Command Palette with fuzzy search, and GPUI CommandPaletteView component.

use crate::search::engine::{parse_nucleo_pattern, score_nucleo_match};
use nucleo_matcher::{Config, Matcher as NucleoMatcher};
use rustcode_platform::render::{
    EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Scene, Shadow,
    View, WorkbenchThemeStyles,
};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaletteItemKind {
    Command,
    File,
    WorkspaceSymbol,
    DocumentSymbol,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickInputMode {
    Command,
    File,
    WorkspaceSymbol,
    DocumentSymbol,
    Line,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GotoLineParseResult {
    Empty,
    Valid {
        line_1based: usize,
        column_1based: Option<usize>,
    },
    Invalid,
}

pub fn parse_goto_line_query(query: &str) -> GotoLineParseResult {
    let stripped = query.strip_prefix(':').unwrap_or(query).trim();
    if stripped.is_empty() {
        return GotoLineParseResult::Empty;
    }
    let mut parts = stripped.split(':');
    let Some(line_str) = parts.next() else {
        return GotoLineParseResult::Empty;
    };
    let col_str_opt = parts.next();
    if parts.next().is_some() {
        return GotoLineParseResult::Invalid;
    }

    let parse_digits_saturating = |s: &str| -> Option<usize> {
        let trimmed = s.trim();
        if trimmed.is_empty() || !trimmed.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(trimmed.parse::<usize>().unwrap_or(usize::MAX))
    };

    let Some(line_1based) = parse_digits_saturating(line_str) else {
        return GotoLineParseResult::Invalid;
    };

    let column_1based = match col_str_opt {
        None => None,
        Some(col_s) if col_s.trim().is_empty() => None,
        Some(col_s) => match parse_digits_saturating(col_s) {
            Some(c) => Some(c),
            None => return GotoLineParseResult::Invalid,
        },
    };

    GotoLineParseResult::Valid {
        line_1based,
        column_1based,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteItem {
    pub command_id: String,
    pub label: String,
    pub category: Option<String>,
    pub keybinding: Option<String>,
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub line: Option<usize>,
    #[serde(default)]
    pub column: Option<usize>,
    #[serde(default = "default_item_kind")]
    pub item_kind: PaletteItemKind,
}

fn default_item_kind() -> PaletteItemKind {
    PaletteItemKind::Command
}

impl Default for PaletteItem {
    fn default() -> Self {
        Self {
            command_id: String::new(),
            label: String::new(),
            category: None,
            keybinding: None,
            path: None,
            line: None,
            column: None,
            item_kind: PaletteItemKind::Command,
        }
    }
}

impl PaletteItem {
    pub fn command(
        command_id: impl Into<String>,
        label: impl Into<String>,
        category: Option<String>,
        keybinding: Option<String>,
    ) -> Self {
        Self {
            command_id: command_id.into(),
            label: label.into(),
            category,
            keybinding,
            path: None,
            line: None,
            column: None,
            item_kind: PaletteItemKind::Command,
        }
    }

    pub fn file(path: PathBuf, relative_label: impl Into<String>) -> Self {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        Self {
            command_id: format!("file:{}", path.to_string_lossy()),
            label: file_name,
            category: Some(relative_label.into()),
            keybinding: None,
            path: Some(path),
            line: None,
            column: None,
            item_kind: PaletteItemKind::File,
        }
    }

    pub fn symbol(
        name: impl Into<String>,
        container: Option<String>,
        path: PathBuf,
        line: usize,
        column: usize,
    ) -> Self {
        let name_str = name.into();
        let cat = match container {
            Some(c) => format!("{} ({}:{})", c, path.to_string_lossy(), line),
            None => format!("{}:{}", path.to_string_lossy(), line),
        };
        Self {
            command_id: format!("symbol:{}:{}:{}", path.to_string_lossy(), line, column),
            label: name_str,
            category: Some(cat),
            keybinding: None,
            path: Some(path),
            line: Some(line),
            column: Some(column),
            item_kind: PaletteItemKind::WorkspaceSymbol,
        }
    }

    pub fn document_symbol(
        name: impl Into<String>,
        container: Option<String>,
        line_1based: usize,
        column_1based: usize,
    ) -> Self {
        let name_str = name.into();
        let cat = match container {
            Some(c) if !c.is_empty() => format!("{} (Line {})", c, line_1based),
            _ => format!("Line {}", line_1based),
        };
        Self {
            command_id: format!("doc_symbol:{}:{}:{}", name_str, line_1based, column_1based),
            label: name_str,
            category: Some(cat),
            keybinding: None,
            path: None,
            line: Some(line_1based),
            column: Some(column_1based),
            item_kind: PaletteItemKind::DocumentSymbol,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandPalette {
    pub items: Vec<PaletteItem>,
    pub query: String,
    pub filtered: Vec<usize>,
    pub selected_index: usize,
    pub mode: QuickInputMode,
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
            mode: QuickInputMode::Command,
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

    pub fn open_mode(&mut self, mode: QuickInputMode) {
        self.mode = mode;
        self.query = match mode {
            QuickInputMode::Command => ">".to_string(),
            QuickInputMode::WorkspaceSymbol => "#".to_string(),
            QuickInputMode::DocumentSymbol => "@".to_string(),
            QuickInputMode::Line => ":".to_string(),
            QuickInputMode::File => String::new(),
        };
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

    pub fn line_prompt_text(&self) -> String {
        match parse_goto_line_query(&self.query) {
            GotoLineParseResult::Empty => {
                "Go to Line: Type a line number (e.g. :line or :line:column)".to_string()
            }
            GotoLineParseResult::Valid {
                line_1based,
                column_1based,
            } => match column_1based {
                Some(col) => format!("Go to Line {}, Column {}", line_1based.max(1), col.max(1)),
                None => format!("Go to Line {}", line_1based.max(1)),
            },
            GotoLineParseResult::Invalid => {
                "Invalid line number — enter a positive line number (e.g. :10 or :10:5)"
                    .to_string()
            }
        }
    }

    pub fn refilter(&mut self) {
        let (detected_mode, target_query) = if let Some(stripped) = self.query.strip_prefix('>') {
            (QuickInputMode::Command, stripped.trim())
        } else if let Some(stripped) = self.query.strip_prefix('#') {
            (QuickInputMode::WorkspaceSymbol, stripped.trim())
        } else if let Some(stripped) = self.query.strip_prefix('@') {
            (QuickInputMode::DocumentSymbol, stripped.trim())
        } else if let Some(stripped) = self.query.strip_prefix(':') {
            (QuickInputMode::Line, stripped.trim())
        } else {
            (QuickInputMode::File, self.query.trim())
        };

        self.mode = detected_mode;
        self.filtered.clear();
        self.selected_index = 0;

        let has_files = self.items.iter().any(|i| i.item_kind == PaletteItemKind::File);
        let matching_indices: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                let kind_matches = match detected_mode {
                    QuickInputMode::Command => item.item_kind == PaletteItemKind::Command,
                    QuickInputMode::WorkspaceSymbol => {
                        item.item_kind == PaletteItemKind::WorkspaceSymbol
                    }
                    QuickInputMode::DocumentSymbol => {
                        item.item_kind == PaletteItemKind::DocumentSymbol
                    }
                    QuickInputMode::Line => item.item_kind == PaletteItemKind::Line,
                    QuickInputMode::File => {
                        if has_files {
                            item.item_kind == PaletteItemKind::File
                        } else {
                            // Fallback for command-only tests; never leak document/workspace symbols
                            item.item_kind == PaletteItemKind::Command
                        }
                    }
                };
                kind_matches.then_some(idx)
            })
            .collect();

        if target_query.is_empty() {
            self.filtered = matching_indices;
            return;
        }

        let config = if detected_mode == QuickInputMode::File {
            Config::DEFAULT.match_paths()
        } else {
            Config::DEFAULT
        };
        let mut matcher = NucleoMatcher::new(config);
        let pattern = parse_nucleo_pattern(target_query);
        let mut char_buf = Vec::new();
        let mut indices = Vec::new();
        let mut scored: Vec<(u32, usize)> = Vec::new();

        for idx in matching_indices {
            let item = &self.items[idx];
            let label_score = score_nucleo_match(
                &pattern,
                target_query,
                &item.label,
                &mut matcher,
                &mut char_buf,
                &mut indices,
            );
            let cat_score = item.category.as_ref().and_then(|cat| {
                let full_text = format!("{cat}: {}", item.label);
                score_nucleo_match(
                    &pattern,
                    target_query,
                    &full_text,
                    &mut matcher,
                    &mut char_buf,
                    &mut indices,
                )
            });

            let combined = match (label_score, cat_score) {
                (Some(ls), Some(cs)) => Some(ls.saturating_add(16).max(cs)),
                (Some(ls), None) => Some(ls.saturating_add(16)),
                (None, Some(cs)) => Some(cs),
                (None, None) => None,
            };

            if let Some(score) = combined {
                scored.push((score, idx));
            }
        }

        scored.sort_by(|a, b| Reverse(a.0).cmp(&Reverse(b.0)).then_with(|| a.1.cmp(&b.1)));
        self.filtered = scored.into_iter().map(|(_, idx)| idx).collect();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    Execute(String),
    OpenFile {
        path: PathBuf,
        line: Option<usize>,
        column: Option<usize>,
    },
    JumpToLine {
        line: usize,
        column: Option<usize>,
    },
    Close,
}

pub struct CommandPaletteView {
    pub model: CommandPalette,
    pub last_action: Option<PaletteAction>,
}

impl Default for CommandPaletteView {
    fn default() -> Self {
        Self::new(CommandPalette::new())
    }
}

impl CommandPaletteView {
    pub fn new(model: CommandPalette) -> Self {
        Self {
            model,
            last_action: None,
        }
    }
}

impl View for CommandPaletteView {
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
        // Shadow behind popup
        scene.push_shadow(Shadow {
            bounds,
            offset: Point::new(0.0, 6.0),
            blur: 16.0,
            spread: 0.0,
            color: rustcode_platform::render::Rgba::new(0.0, 0.0, 0.0, 0.4),
        });

        // Popup background with rounded border
        scene.push_quad(
            Quad::new(bounds)
                .with_background(theme.command_palette_bg)
                .with_border(theme.command_palette_border, 1.0)
                .with_corner_radius(6.0),
        );

        // Input container (clamped to bounds height)
        let input_h = 36.0f32.min(bounds.height());
        let prompt_x = (bounds.min_x() + 12.0).min(bounds.max_x());
        let text_y = bounds.min_y() + ((input_h - font.line_height) / 2.0).max(0.0);
        let text_h = font.line_height.min((bounds.max_y() - text_y).max(0.0));
        let max_input_x = (bounds.max_x() - 6.0).max(prompt_x);

        // Prompt string & query
        let (prompt_str, display_query) = if let Some(stripped) = self.model.query.strip_prefix('>') {
            ("> ", stripped)
        } else if let Some(stripped) = self.model.query.strip_prefix('#') {
            ("# ", stripped)
        } else if let Some(stripped) = self.model.query.strip_prefix('@') {
            ("@ ", stripped)
        } else if let Some(stripped) = self.model.query.strip_prefix(':') {
            (": ", stripped)
        } else {
            ("> ", self.model.query.as_str())
        };

        let prompt_w = font
            .text_width(prompt_str)
            .min((max_input_x - prompt_x).max(0.0));

        // Prompt
        if prompt_w > 0.0 && text_h > 0.0 {
            scene.push_text(GlyphRun::new(
                prompt_str.to_string(),
                Point::new(prompt_x, text_y),
                Rect::from_xywh(prompt_x, text_y, prompt_w, text_h),
                theme.command_palette_fg,
                font.font_size,
            ));
        }

        // Query text
        let query_x = (prompt_x + prompt_w).min(max_input_x);
        let query_w = font
            .text_width(display_query)
            .min((max_input_x - query_x).max(0.0));
        if !display_query.is_empty() && query_w > 0.0 && text_h > 0.0 {
            scene.push_text(GlyphRun::new(
                display_query.to_string(),
                Point::new(query_x, text_y),
                Rect::from_xywh(query_x, text_y, query_w, text_h),
                theme.command_palette_fg,
                font.font_size,
            ));
        }

        // Blinking cursor after query
        let cursor_x = (query_x + query_w).min((bounds.max_x() - 4.0).max(bounds.min_x()));
        let cursor_w = 2.0f32.min((bounds.max_x() - cursor_x).max(0.0));
        if cursor_w > 0.0 && text_h > 0.0 {
            let cursor_rect = Rect::from_xywh(cursor_x, text_y, cursor_w, text_h);
            scene.push_quad(Quad::new(cursor_rect).with_background(theme.editor_cursor));
        }

        // Divider line below input
        if bounds.height() > input_h {
            let div_h = 1.0f32.min((bounds.max_y() - (bounds.min_y() + input_h)).max(0.0));
            let div_rect =
                Rect::from_xywh(bounds.min_x(), bounds.min_y() + input_h, bounds.width(), div_h);
            scene.push_quad(Quad::new(div_rect).with_background(theme.command_palette_border));
        }

        // Result rows
        let row_h = 28.0;
        let max_visible = ((bounds.height() - input_h - 1.0) / row_h)
            .floor()
            .max(0.0) as usize;
        if max_visible == 0 {
            return;
        }

        if self.model.mode == QuickInputMode::Line {
            let row_y = bounds.min_y() + input_h + 1.0;
            let clamped_row_h = row_h.min((bounds.max_y() - row_y).max(0.0));
            let row_rect = Rect::from_xywh(
                bounds.min_x() + 4.0f32.min(bounds.width() * 0.5),
                row_y,
                (bounds.width() - 8.0).max(0.0),
                clamped_row_h,
            );
            scene.push_quad(
                Quad::new(row_rect)
                    .with_background(theme.command_palette_selected_bg)
                    .with_corner_radius(4.0),
            );
            let prompt_msg = self.model.line_prompt_text();
            let label_x = (bounds.min_x() + 14.0).min(bounds.max_x());
            let item_y = row_y + ((clamped_row_h - font.line_height) / 2.0).max(0.0);
            let avail_w = (bounds.max_x() - label_x - 8.0).max(0.0);
            let label_w = font.text_width(&prompt_msg).min(avail_w);
            let item_h = font.line_height.min((bounds.max_y() - item_y).max(0.0));
            if label_w > 0.0 && item_h > 0.0 {
                let color = if matches!(
                    parse_goto_line_query(&self.model.query),
                    GotoLineParseResult::Invalid
                ) {
                    theme.editor_error_foreground
                } else {
                    theme.command_palette_fg
                };
                scene.push_text(GlyphRun::new(
                    prompt_msg,
                    Point::new(label_x, item_y),
                    Rect::from_xywh(label_x, item_y, label_w, item_h),
                    color,
                    font.font_size,
                ));
            }
            return;
        }

        let results = self.model.results();
        if self.model.mode == QuickInputMode::DocumentSymbol && results.is_empty() {
            let row_y = bounds.min_y() + input_h + 1.0;
            let clamped_row_h = row_h.min((bounds.max_y() - row_y).max(0.0));
            let label_x = (bounds.min_x() + 14.0).min(bounds.max_x());
            let item_y = row_y + ((clamped_row_h - font.line_height) / 2.0).max(0.0);
            let avail_w = (bounds.max_x() - label_x - 8.0).max(0.0);
            let msg = "No symbols found";
            let label_w = font.text_width(msg).min(avail_w);
            let item_h = font.line_height.min((bounds.max_y() - item_y).max(0.0));
            if label_w > 0.0 && item_h > 0.0 {
                scene.push_text(GlyphRun::new(
                    msg.to_string(),
                    Point::new(label_x, item_y),
                    Rect::from_xywh(label_x, item_y, label_w, item_h),
                    theme.tab_inactive_fg,
                    font.font_size,
                ));
            }
            return;
        }

        for (idx, item) in results.iter().take(max_visible).enumerate() {
            let row_y = bounds.min_y() + input_h + 1.0 + idx as f32 * row_h;
            let clamped_row_h = row_h.min((bounds.max_y() - row_y).max(0.0));
            if clamped_row_h <= 0.0 {
                break;
            }
            let row_rect = Rect::from_xywh(
                bounds.min_x() + 4.0f32.min(bounds.width() * 0.5),
                row_y,
                (bounds.width() - 8.0).max(0.0),
                clamped_row_h,
            );

            let is_selected = idx == self.model.selected_index;
            if is_selected {
                scene.push_quad(
                    Quad::new(row_rect)
                        .with_background(theme.command_palette_selected_bg)
                        .with_corner_radius(4.0),
                );
            }

            // Category + label
            let display_str = match &item.category {
                Some(cat) => format!("{}: {}", cat, item.label),
                None => item.label.clone(),
            };
            let label_x = (bounds.min_x() + 14.0).min(bounds.max_x());
            let item_y = row_y + ((clamped_row_h - font.line_height) / 2.0).max(0.0);
            let item_h = font.line_height.min((bounds.max_y() - item_y).max(0.0));
            let avail_label_w = (bounds.max_x() - label_x - 8.0).max(0.0);
            let label_w = font.text_width(&display_str).min(avail_label_w);

            if label_w > 0.0 && item_h > 0.0 {
                scene.push_text(GlyphRun::new(
                    display_str,
                    Point::new(label_x, item_y),
                    Rect::from_xywh(label_x, item_y, label_w, item_h),
                    theme.command_palette_fg,
                    font.font_size,
                ));
            }

            // Keybinding badge if present
            if let Some(kb) = &item.keybinding {
                let raw_kb_w = font.text_width(kb);
                let kb_x = (bounds.max_x() - 14.0 - raw_kb_w).max(label_x);
                let kb_w = raw_kb_w.min((bounds.max_x() - kb_x - 4.0).max(0.0));
                if kb_w > 0.0 && item_h > 0.0 {
                    scene.push_text(GlyphRun::new(
                        kb.clone(),
                        Point::new(kb_x, item_y),
                        Rect::from_xywh(kb_x, item_y, kb_w, item_h),
                        theme.tab_inactive_fg,
                        font.font_size * 0.9,
                    ));
                }
            }
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::KeyDown(key) => match &key.key {
                KeyCode::ArrowDown => {
                    self.model.select_next();
                    EventResult::Consumed
                }
                KeyCode::ArrowUp => {
                    self.model.select_prev();
                    EventResult::Consumed
                }
                KeyCode::Enter => {
                    if self.model.mode == QuickInputMode::Line {
                        if let GotoLineParseResult::Valid {
                            line_1based,
                            column_1based,
                        } = parse_goto_line_query(&self.model.query)
                        {
                            self.last_action = Some(PaletteAction::JumpToLine {
                                line: line_1based,
                                column: column_1based,
                            });
                        }
                        return EventResult::Consumed;
                    }
                    if let Some(item) = self.model.selected_item() {
                        if item.item_kind == PaletteItemKind::DocumentSymbol && item.path.is_none()
                        {
                            self.last_action = Some(PaletteAction::JumpToLine {
                                line: item.line.unwrap_or(1),
                                column: item.column,
                            });
                        } else if let Some(path) = &item.path {
                            self.last_action = Some(PaletteAction::OpenFile {
                                path: path.clone(),
                                line: item.line,
                                column: item.column,
                            });
                        } else {
                            self.last_action = Some(PaletteAction::Execute(item.command_id.clone()));
                        }
                        EventResult::Consumed
                    } else {
                        EventResult::Ignored
                    }
                }
                KeyCode::Escape => {
                    self.last_action = Some(PaletteAction::Close);
                    EventResult::Consumed
                }
                KeyCode::Backspace => {
                    if !self.model.query.is_empty() {
                        self.model.query.pop();
                        self.model.refilter();
                        EventResult::Consumed
                    } else {
                        EventResult::Ignored
                    }
                }
                KeyCode::Space if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                    self.model.query.push(' ');
                    self.model.refilter();
                    EventResult::Consumed
                }
                KeyCode::Char(c) if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta => {
                    match &key.text {
                        Some(txt) if !txt.is_empty() => self.model.query.push_str(txt),
                        _ => {
                            let ch = if key.modifiers.shift {
                                c.to_ascii_uppercase()
                            } else {
                                *c
                            };
                            self.model.query.push(ch);
                        }
                    }
                    self.model.refilter();
                    EventResult::Consumed
                }
                _ => EventResult::Ignored,
            },
            InputEvent::MouseDown(mouse) => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }
                let input_h = 36.0f32.min(bounds.height());
                let row_h = 28.0;
                if mouse.position.y > bounds.min_y() + input_h {
                    let idx = ((mouse.position.y - (bounds.min_y() + input_h + 1.0)) / row_h)
                        .floor()
                        .max(0.0) as usize;
                    if self.model.mode == QuickInputMode::Line && idx == 0 {
                        if let GotoLineParseResult::Valid {
                            line_1based,
                            column_1based,
                        } = parse_goto_line_query(&self.model.query)
                        {
                            self.last_action = Some(PaletteAction::JumpToLine {
                                line: line_1based,
                                column: column_1based,
                            });
                        }
                        return EventResult::Consumed;
                    }
                    let maybe_action = if let Some(item) = self.model.results().get(idx) {
                        if item.item_kind == PaletteItemKind::DocumentSymbol && item.path.is_none()
                        {
                            Some(PaletteAction::JumpToLine {
                                line: item.line.unwrap_or(1),
                                column: item.column,
                            })
                        } else if let Some(path) = &item.path {
                            Some(PaletteAction::OpenFile {
                                path: path.clone(),
                                line: item.line,
                                column: item.column,
                            })
                        } else {
                            Some(PaletteAction::Execute(item.command_id.clone()))
                        }
                    } else {
                        None
                    };

                    if let Some(act) = maybe_action {
                        self.model.selected_index = idx;
                        self.last_action = Some(act);
                        return EventResult::Consumed;
                    }
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::KeyEvent;

    #[test]
    fn test_command_palette_fuzzy_search() {
        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem::command(
                "workbench.action.reloadWindow",
                "Reload Window",
                Some("Developer".to_string()),
                None,
            ),
            PaletteItem::command(
                "editor.action.formatDocument",
                "Format Document",
                None,
                Some("Shift+Alt+F".to_string()),
            ),
            PaletteItem::command(
                "git.commit",
                "Commit",
                Some("Git".to_string()),
                None,
            ),
        ]);

        assert_eq!(palette.results().len(), 3);

        palette.set_query("format");
        assert_eq!(palette.results().len(), 1);
        assert_eq!(
            palette.selected_item().unwrap().command_id,
            "editor.action.formatDocument"
        );

        palette.set_query("dev rel");
        assert_eq!(palette.results().len(), 1);
        assert_eq!(
            palette.selected_item().unwrap().command_id,
            "workbench.action.reloadWindow"
        );
    }

    #[test]
    fn test_command_palette_unicode_case_folding() {
        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem::command(
                "workbench.action.openFile",
                "Öffnen",
                Some("Datei".to_string()),
                None,
            ),
            PaletteItem::command(
                "workbench.action.closeFile",
                "Schließen",
                Some("Datei".to_string()),
                None,
            ),
        ]);

        // Query with lowercase umlaut matching uppercase umlaut
        palette.set_query(">öffnen");
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].label, "Öffnen");

        // Query category with lowercase
        palette.set_query(">datei");
        assert_eq!(palette.results().len(), 2);

        // Query lowercase matching German eszett
        palette.set_query(">schließen");
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].label, "Schließen");
    }

    #[test]
    fn test_command_palette_view_interaction() {
        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem::command(
                "workbench.action.toggleSidebar",
                "Toggle Sidebar",
                Some("View".to_string()),
                Some("Ctrl+B".to_string()),
            ),
            PaletteItem::command(
                "workbench.action.openSettings",
                "Open Settings",
                Some("Preferences".to_string()),
                Some("Ctrl+,".to_string()),
            ),
        ]);

        let mut view = CommandPaletteView::new(palette);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let bounds = Rect::from_xywh(100.0, 50.0, 600.0, 300.0);
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("Toggle Sidebar"));
        assert!(scene.contains_text("Open Settings"));

        // Type "set"
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('s'), Modifiers::none())),
            bounds,
        );
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('e'), Modifiers::none())),
            bounds,
        );
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('t'), Modifiers::none())),
            bounds,
        );

        assert_eq!(view.model.results().len(), 1);
        assert_eq!(
            view.model.selected_item().unwrap().command_id,
            "workbench.action.openSettings"
        );

        // Press Enter to execute
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            view.last_action,
            Some(PaletteAction::Execute("workbench.action.openSettings".to_string()))
        );

        // Test space typing in command palette
        let mut space_palette = CommandPalette::new();
        space_palette.set_items(vec![
            PaletteItem::command(
                "workbench.action.toggleSidebar",
                "Toggle Sidebar",
                Some("View".to_string()),
                None,
            ),
        ]);
        let mut space_view = CommandPaletteView::new(space_palette);
        for key in [KeyCode::Char('v'), KeyCode::Space, KeyCode::Char('t')] {
            let res = space_view.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(key, Modifiers::none())),
                bounds,
            );
            assert_eq!(res, EventResult::Consumed);
        }
        assert_eq!(space_view.model.query, "v t");
        assert_eq!(space_view.model.results().len(), 1);

        // Click outside bounds should be ignored
        let outside_ev = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(50.0, 20.0), // outside bounds
            rustcode_platform::render::MouseButton::Left,
        ));
        assert_eq!(space_view.handle_event(&outside_ev, bounds), EventResult::Ignored);
    }

    #[test]
    fn test_quick_open_multi_mode_and_open_file() {
        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem::command(
                "workbench.action.reloadWindow",
                "Reload Window",
                Some("Developer".to_string()),
                None,
            ),
            PaletteItem::file(
                PathBuf::from("/project/backend/src/main.rs"),
                "backend/src/main.rs",
            ),
            PaletteItem::file(
                PathBuf::from("/project/frontend/src/app.tsx"),
                "frontend/src/app.tsx",
            ),
            PaletteItem::symbol(
                "process_order",
                Some("OrderService".to_string()),
                PathBuf::from("/project/backend/src/order.rs"),
                42,
                8,
            ),
        ]);

        // 1. Default / prefix-less: file search
        palette.set_query("main");
        assert_eq!(palette.mode, QuickInputMode::File);
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].label, "main.rs");

        // 2. Command mode with '>'
        palette.set_query(">reload");
        assert_eq!(palette.mode, QuickInputMode::Command);
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].command_id, "workbench.action.reloadWindow");

        // 3. Workspace symbol mode with '#'
        palette.set_query("#order");
        assert_eq!(palette.mode, QuickInputMode::WorkspaceSymbol);
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].label, "process_order");

        // 4. Test Enter emits OpenFile action
        let mut view = CommandPaletteView::new(palette);
        view.model.set_query("app");
        let bounds = Rect::from_xywh(100.0, 50.0, 600.0, 300.0);
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            view.last_action,
            Some(PaletteAction::OpenFile {
                path: PathBuf::from("/project/frontend/src/app.tsx"),
                line: None,
                column: None,
            })
        );
    }

    #[test]
    fn test_command_palette_render_non_finite_bounds_emits_nothing() {
        use rustcode_platform::render::Size;

        let mut palette = CommandPalette::new();
        palette.set_items(vec![PaletteItem::command(
            "workbench.action.openSettings",
            "Open Settings",
            None,
            None,
        )]);
        let view = CommandPaletteView::new(palette);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for bad_bounds in [
            Rect::new(Point::zero(), Size::new(f32::NAN, 320.0)),
            Rect::new(Point::new(0.0, f32::NAN), Size::new(600.0, 320.0)),
            Rect::from_xywh(0.0, 0.0, 600.0, f32::INFINITY),
        ] {
            let mut scene = Scene::new();
            view.render(bad_bounds, &theme, &font, &mut scene);
            assert!(scene.is_empty());
        }
    }

    #[test]
    fn test_quickinput_goto_line_parsing_and_dynamic_prefix_switching() {
        assert_eq!(parse_goto_line_query(":"), GotoLineParseResult::Empty);
        assert_eq!(
            parse_goto_line_query(":0"),
            GotoLineParseResult::Valid {
                line_1based: 0,
                column_1based: None
            }
        );
        assert_eq!(
            parse_goto_line_query(":999999"),
            GotoLineParseResult::Valid {
                line_1based: 999999,
                column_1based: None
            }
        );
        assert_eq!(
            parse_goto_line_query(":2:99999"),
            GotoLineParseResult::Valid {
                line_1based: 2,
                column_1based: Some(99999)
            }
        );
        assert_eq!(parse_goto_line_query(":abc"), GotoLineParseResult::Invalid);
        assert_eq!(parse_goto_line_query(":1:2:3"), GotoLineParseResult::Invalid);

        let mut palette = CommandPalette::new();
        palette.set_items(vec![
            PaletteItem::file(PathBuf::from("/w/main.rs"), "main.rs"),
            PaletteItem::document_symbol("alpha_fn", None, 4, 1),
            PaletteItem::command("workbench.action.toggleSidebar", "Toggle Sidebar", None, None),
        ]);
        palette.open_mode(QuickInputMode::File);
        assert_eq!(palette.mode, QuickInputMode::File);
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].label, "main.rs");

        // Prefix '@' switches to DocumentSymbol
        palette.set_query("@alpha");
        assert_eq!(palette.mode, QuickInputMode::DocumentSymbol);
        assert_eq!(palette.results().len(), 1);
        assert_eq!(palette.results()[0].label, "alpha_fn");

        // Prefix ':' switches to Line; non-numeric does not emit JumpToLine
        let mut view = CommandPaletteView::new(palette);
        view.model.set_query(":abc");
        assert_eq!(view.model.mode, QuickInputMode::Line);
        let bounds = Rect::from_xywh(20.0, 40.0, 200.0, 100.0);
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert!(view.last_action.is_none());

        // Valid ':3:5' emits JumpToLine
        view.model.set_query(":3:5");
        view.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            view.last_action,
            Some(PaletteAction::JumpToLine {
                line: 3,
                column: Some(5)
            })
        );

        // Tiny viewport renders finite non-negative bounds
        let mut scene = Scene::new();
        view.render(bounds, &WorkbenchThemeStyles::dark_default(), &FontMetrics::default_monospace(), &mut scene);
        for q in scene.quads() {
            assert!(
                q.bounds.origin.x.is_finite()
                    && q.bounds.origin.y.is_finite()
                    && q.bounds.width().is_finite()
                    && q.bounds.height().is_finite()
                    && q.bounds.width() >= 0.0
                    && q.bounds.height() >= 0.0
            );
        }
    }

    #[test]
    fn test_quickinput_nucleo_ranking_mru_and_unicode_safety() {
        // 1. Command Palette (Ctrl+Shift+P): exact prefix > word/camelCase boundary > scattered
        let mut cmd_palette = CommandPalette::new();
        cmd_palette.set_items(vec![
            PaletteItem::command("c.scattered", "f_x_o_x_o_bar", None, None),
            PaletteItem::command("c.camel", "openFooBarPanel", None, None),
            PaletteItem::command("c.prefix", "foobar_action", None, None),
        ]);

        // Empty query preserves original/MRU ordering
        cmd_palette.set_query(">");
        let empty_order: Vec<&str> = cmd_palette
            .results()
            .into_iter()
            .map(|i| i.command_id.as_str())
            .collect();
        assert_eq!(empty_order, vec!["c.scattered", "c.camel", "c.prefix"]);

        // Ranked order: prefix > camelCase boundary > scattered
        cmd_palette.set_query(">foobar");
        let ranked_cmds: Vec<&str> = cmd_palette
            .results()
            .into_iter()
            .map(|i| i.command_id.as_str())
            .collect();
        assert_eq!(ranked_cmds, vec!["c.prefix", "c.camel", "c.scattered"]);

        // 2. Quick Open Files (Ctrl+P): prefix > boundary > scattered
        let mut file_palette = CommandPalette::new();
        file_palette.set_items(vec![
            PaletteItem::file(PathBuf::from("/w/src/m_a_i_n_x.rs"), "src/m_a_i_n_x.rs"),
            PaletteItem::file(PathBuf::from("/w/src/appMain.rs"), "src/appMain.rs"),
            PaletteItem::file(PathBuf::from("/w/src/main.rs"), "src/main.rs"),
        ]);
        file_palette.set_query("main");
        let ranked_files: Vec<&str> = file_palette
            .results()
            .into_iter()
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(ranked_files, vec!["main.rs", "appMain.rs", "m_a_i_n_x.rs"]);

        // 3. Document Symbols (Ctrl+Shift+O): prefix > camelCase > scattered
        let mut sym_palette = CommandPalette::new();
        sym_palette.set_items(vec![
            PaletteItem::document_symbol("r_e_n_d_e_r", None, 1, 1),
            PaletteItem::document_symbol("onRenderFrame", None, 10, 1),
            PaletteItem::document_symbol("render_scene", None, 20, 1),
        ]);
        sym_palette.set_query("@render");
        let ranked_syms: Vec<&str> = sym_palette
            .results()
            .into_iter()
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(
            ranked_syms,
            vec!["render_scene", "onRenderFrame", "r_e_n_d_e_r"]
        );

        // 4. Unicode & symbol queries never panic
        for q in [">🚀", ">!", ">^", ">$", ">'", ">äöü", ">(foo)", "@::", "#$"] {
            cmd_palette.set_query(q);
            file_palette.set_query(q);
            sym_palette.set_query(q);
        }
    }
}
