//! Cross-folder search view, result tree projection, and GPUI SearchView component.

pub mod engine;
pub mod symbols;

pub use engine::{
    replace_in_text, replace_in_workspace, search_files, search_text, try_search_text,
    FileSearchResult, ReplaceFileOutcome, SearchQuery, SearchResults, TextSearchMatch,
    WorkspaceReplaceResult,
};
pub use symbols::{search_document_symbols, search_workspace_symbols};

use crate::workspace::WorkspaceFolder;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Scene, View,
    WorkbenchThemeStyles,
};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchAction {
    OpenMatch {
        path: PathBuf,
        line: usize,
        col: usize,
    },
    ExecuteSearch,
    ExecuteReplace,
    ClearSearch,
    ToggleMatchCase,
    ToggleWholeWord,
    ToggleRegex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchInputFocus {
    Query,
    Replace,
    Results,
}

pub struct SearchView {
    pub query: SearchQuery,
    pub replace_text: String,
    pub is_replace_expanded: bool,
    pub results: SearchResults,
    pub collapsed_files: HashSet<PathBuf>,
    pub selected_item: Option<usize>, // flat index across visible result rows
    pub focused_input: SearchInputFocus,
    pub last_action: Option<SearchAction>,
    pub last_error: Option<String>,
    pub last_replace_result: Option<WorkspaceReplaceResult>,
}

impl Default for SearchView {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchView {
    pub fn new() -> Self {
        Self {
            query: SearchQuery::default(),
            replace_text: String::new(),
            is_replace_expanded: false,
            results: SearchResults::default(),
            collapsed_files: HashSet::new(),
            selected_item: None,
            focused_input: SearchInputFocus::Query,
            last_action: None,
            last_error: None,
            last_replace_result: None,
        }
    }

    /// Execute the current search across the given workspace folders.
    pub fn execute_search(&mut self, folders: &[WorkspaceFolder]) {
        match try_search_text(folders, &self.query) {
            Ok(results) => {
                self.results = results;
                self.last_error = None;
                self.selected_item = if self.results.total_matches > 0 {
                    Some(1) // first match line
                } else {
                    None
                };
            }
            Err(err) => {
                self.results = SearchResults::default();
                self.last_error = Some(err);
                self.selected_item = None;
            }
        }
    }

    /// Replace all matches across the workspace and refresh results.
    pub fn execute_replace(
        &mut self,
        folders: &[WorkspaceFolder],
    ) -> Result<WorkspaceReplaceResult, String> {
        match replace_in_workspace(folders, &self.query, &self.replace_text) {
            Ok(outcome) => {
                self.last_error = None;
                self.last_replace_result = Some(outcome.clone());
                self.execute_search(folders);
                Ok(outcome)
            }
            Err(err) => {
                self.last_error = Some(err.clone());
                Err(err)
            }
        }
    }

    /// Clear results and query.
    pub fn clear(&mut self) {
        self.query.query.clear();
        self.results = SearchResults::default();
        self.selected_item = None;
        self.last_error = None;
        self.last_replace_result = None;
    }

    pub fn set_query(&mut self, q: &str) {
        self.query.query = q.to_string();
    }

    /// Generate flat list of visible rows (file headers and line matches) for rendering and click mapping.
    pub fn visible_rows(&self) -> Vec<SearchRow> {
        let mut rows = Vec::new();

        for file_res in &self.results.file_results {
            let is_collapsed = self.collapsed_files.contains(&file_res.file_path);
            rows.push(SearchRow::FileHeader {
                file_path: file_res.file_path.clone(),
                relative_path: file_res.relative_path.clone(),
                folder_name: file_res.folder_name.clone(),
                match_count: file_res.matches.len(),
                is_collapsed,
            });

            if !is_collapsed {
                for m in &file_res.matches {
                    rows.push(SearchRow::MatchLine {
                        file_path: file_res.file_path.clone(),
                        line_number: m.line_number,
                        start_col: m.start_col,
                        end_col: m.end_col,
                        line_content: m.line_content.clone(),
                    });
                }
            }
        }

        rows
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchRow {
    FileHeader {
        file_path: PathBuf,
        relative_path: PathBuf,
        folder_name: String,
        match_count: usize,
        is_collapsed: bool,
    },
    MatchLine {
        file_path: PathBuf,
        line_number: usize,
        start_col: usize,
        end_col: usize,
        line_content: String,
    },
}

impl View for SearchView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        // Sidebar background
        scene.push_quad(
            Quad::new(bounds)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.sidebar_border, Edges::new(0.0, 1.0, 0.0, 0.0)),
        );

        let mut cur_y = bounds.min_y();

        // 1. Header (height 28.0)
        let header_h = 28.0;
        let header_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), header_h);
        scene.push_text(GlyphRun::new(
            "SEARCH".to_string(),
            Point::new(bounds.min_x() + 12.0, cur_y + (header_h - font.line_height) / 2.0),
            header_rect,
            theme.sidebar_header_fg,
            font.font_size * 0.85,
        ));
        cur_y += header_h;

        // 2. Query input box (height 26.0)
        let pad_x = 8.0;
        let input_h = 26.0;
        let input_w = bounds.width() - pad_x * 2.0;
        let input_rect = Rect::from_xywh(bounds.min_x() + pad_x, cur_y, input_w, input_h);

        scene.push_quad(
            Quad::new(input_rect)
                .with_background(theme.editor_background)
                .with_border(theme.command_palette_border, 1.0)
                .with_corner_radius(3.0),
        );

        // Query text or placeholder
        let text_y = cur_y + (input_h - font.line_height) / 2.0;
        let text_x = input_rect.min_x() + 6.0;

        if self.query.query.is_empty() {
            scene.push_text(GlyphRun::new(
                "Search...".to_string(),
                Point::new(text_x, text_y),
                input_rect,
                theme.tab_inactive_fg,
                font.font_size * 0.9,
            ));
        } else {
            let q_w = font.text_width(&self.query.query);
            scene.push_text(GlyphRun::new(
                self.query.query.clone(),
                Point::new(text_x, text_y),
                input_rect,
                theme.sidebar_fg,
                font.font_size * 0.9,
            ));

            if self.focused_input == SearchInputFocus::Query {
                let cursor_rect = Rect::from_xywh(text_x + q_w, text_y, 1.5, font.line_height);
                scene.push_quad(Quad::new(cursor_rect).with_background(theme.editor_cursor));
            }
        }

        // Search options chips on the right edge of input: [Aa] [\b] [.*]
        let chip_w = 16.0;
        let chip_h = 16.0;
        let chip_y = cur_y + (input_h - chip_h) / 2.0;

        let regex_x = input_rect.max_x() - 4.0 - chip_w;
        let word_x = regex_x - chip_w - 2.0;
        let case_x = word_x - chip_w - 2.0;

        // Case chip
        render_chip(scene, Rect::from_xywh(case_x, chip_y, chip_w, chip_h), "Aa", self.query.match_case, theme, font);
        // Whole word chip
        render_chip(scene, Rect::from_xywh(word_x, chip_y, chip_w, chip_h), r"\b", self.query.match_whole_word, theme, font);
        // Regex chip
        render_chip(scene, Rect::from_xywh(regex_x, chip_y, chip_w, chip_h), ".*", self.query.is_regex, theme, font);

        cur_y += input_h + 6.0;

        // 3. Results Summary / Error (height 20.0)
        let (summary_text, summary_color) = if let Some(err) = &self.last_error {
            (err.clone(), theme.editor_error_foreground)
        } else if self.results.total_matches > 0 {
            (
                format!(
                    "{} results in {} files",
                    self.results.total_matches,
                    self.results.file_results.len()
                ),
                theme.sidebar_header_fg,
            )
        } else if !self.query.query.is_empty() {
            ("No results found".to_string(), theme.sidebar_header_fg)
        } else {
            (String::new(), theme.sidebar_header_fg)
        };

        if !summary_text.is_empty() {
            let summary_rect = Rect::from_xywh(bounds.min_x() + pad_x, cur_y, input_w, 18.0);
            scene.push_text(GlyphRun::new(
                summary_text,
                Point::new(bounds.min_x() + pad_x, cur_y),
                summary_rect,
                summary_color,
                font.font_size * 0.8,
            ));
            cur_y += 20.0;
        }

        // 4. Render Result Rows
        let row_h = 20.0;
        let rows = self.visible_rows();

        for (idx, row) in rows.iter().enumerate() {
            if cur_y + row_h > bounds.max_y() {
                break;
            }

            let row_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), row_h);

            let is_selected = self.selected_item == Some(idx);
            if is_selected {
                scene.push_quad(Quad::new(row_rect).with_background(theme.list_active_selection_bg));
            }

            match row {
                SearchRow::FileHeader {
                    relative_path,
                    match_count,
                    is_collapsed,
                    ..
                } => {
                    let chevron = if *is_collapsed { "> " } else { "v " };
                    let header_label = format!(
                        "{}{} ({})",
                        chevron,
                        relative_path.to_string_lossy(),
                        match_count
                    );
                    let label_w = font.text_width(&header_label);

                    scene.push_text(GlyphRun::new(
                        header_label,
                        Point::new(bounds.min_x() + 8.0, cur_y),
                        Rect::from_xywh(bounds.min_x() + 8.0, cur_y, label_w, row_h),
                        theme.sidebar_fg,
                        font.font_size * 0.9,
                    ));
                }
                SearchRow::MatchLine {
                    line_number,
                    line_content,
                    ..
                } => {
                    let line_label = format!("  {}: {}", line_number, line_content.trim());
                    let label_w = font.text_width(&line_label);

                    scene.push_text(GlyphRun::new(
                        line_label,
                        Point::new(bounds.min_x() + 16.0, cur_y),
                        Rect::from_xywh(bounds.min_x() + 16.0, cur_y, label_w, row_h),
                        theme.tab_inactive_fg,
                        font.font_size * 0.85,
                    ));
                }
            }

            cur_y += row_h;
        }
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::MouseDown(mouse) => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }

                let cur_y = bounds.min_y() + 28.0; // below header
                let input_h = 26.0;
                let pad_x = 8.0;
                let input_w = bounds.width() - pad_x * 2.0;
                let input_rect = Rect::from_xywh(bounds.min_x() + pad_x, cur_y, input_w, input_h);

                // Check toggle chips clicks
                let chip_w = 16.0;
                let chip_h = 16.0;
                let chip_y = cur_y + (input_h - chip_h) / 2.0;
                let regex_rect = Rect::from_xywh(input_rect.max_x() - 4.0 - chip_w, chip_y, chip_w, chip_h);
                let word_rect = Rect::from_xywh(regex_rect.min_x() - 2.0 - chip_w, chip_y, chip_w, chip_h);
                let case_rect = Rect::from_xywh(word_rect.min_x() - 2.0 - chip_w, chip_y, chip_w, chip_h);

                if case_rect.contains(mouse.position) {
                    self.query.match_case = !self.query.match_case;
                    self.last_action = Some(SearchAction::ToggleMatchCase);
                    return EventResult::Consumed;
                } else if word_rect.contains(mouse.position) {
                    self.query.match_whole_word = !self.query.match_whole_word;
                    self.last_action = Some(SearchAction::ToggleWholeWord);
                    return EventResult::Consumed;
                } else if regex_rect.contains(mouse.position) {
                    self.query.is_regex = !self.query.is_regex;
                    self.last_action = Some(SearchAction::ToggleRegex);
                    return EventResult::Consumed;
                }

                // Check query input click
                if input_rect.contains(mouse.position) {
                    self.focused_input = SearchInputFocus::Query;
                    return EventResult::Consumed;
                }

                // Check result rows click
                let results_start_y = cur_y + input_h + 6.0 + 20.0;
                if mouse.position.y >= results_start_y {
                    let row_h = 20.0;
                    let row_idx = ((mouse.position.y - results_start_y) / row_h).floor() as usize;
                    let rows = self.visible_rows();

                    if let Some(row) = rows.get(row_idx) {
                        self.selected_item = Some(row_idx);
                        match row {
                            SearchRow::FileHeader { file_path, is_collapsed, .. } => {
                                if *is_collapsed {
                                    self.collapsed_files.remove(file_path);
                                } else {
                                    self.collapsed_files.insert(file_path.clone());
                                }
                            }
                            SearchRow::MatchLine { file_path, line_number, start_col, .. } => {
                                self.last_action = Some(SearchAction::OpenMatch {
                                    path: file_path.clone(),
                                    line: *line_number,
                                    col: *start_col,
                                });
                            }
                        }
                        return EventResult::Consumed;
                    }
                }

                EventResult::Ignored
            }
            InputEvent::KeyDown(key) => {
                match &key.key {
                    KeyCode::Enter => {
                        if self.focused_input == SearchInputFocus::Query {
                            self.last_action = Some(SearchAction::ExecuteSearch);
                            EventResult::Consumed
                        } else if let Some(idx) = self.selected_item {
                            let rows = self.visible_rows();
                            if let Some(SearchRow::MatchLine { file_path, line_number, start_col, .. }) = rows.get(idx) {
                                self.last_action = Some(SearchAction::OpenMatch {
                                    path: file_path.clone(),
                                    line: *line_number,
                                    col: *start_col,
                                });
                                EventResult::Consumed
                            } else {
                                EventResult::Ignored
                            }
                        } else {
                            EventResult::Ignored
                        }
                    }
                    KeyCode::ArrowDown => {
                        let rows = self.visible_rows();
                        if !rows.is_empty() {
                            let next = self.selected_item.map_or(0, |i| (i + 1).min(rows.len() - 1));
                            self.selected_item = Some(next);
                            self.focused_input = SearchInputFocus::Results;
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                    KeyCode::ArrowUp => {
                        let rows = self.visible_rows();
                        if !rows.is_empty() {
                            let prev = self.selected_item.map_or(0, |i| i.saturating_sub(1));
                            self.selected_item = Some(prev);
                            if prev == 0 {
                                self.focused_input = SearchInputFocus::Query;
                            }
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                    KeyCode::Escape => {
                        self.clear();
                        self.last_action = Some(SearchAction::ClearSearch);
                        EventResult::Consumed
                    }
                    KeyCode::Backspace if self.focused_input == SearchInputFocus::Query => {
                        if !self.query.query.is_empty() {
                            self.query.query.pop();
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                    KeyCode::Space if self.focused_input == SearchInputFocus::Query && !key.modifiers.ctrl && !key.modifiers.alt => {
                        self.query.query.push(' ');
                        EventResult::Consumed
                    }
                    KeyCode::Char(c) if self.focused_input == SearchInputFocus::Query && !key.modifiers.ctrl && !key.modifiers.alt => {
                        let ch = if key.modifiers.shift { c.to_ascii_uppercase() } else { *c };
                        self.query.query.push(ch);
                        EventResult::Consumed
                    }
                    _ => EventResult::Ignored,
                }
            }
            _ => EventResult::Ignored,
        }
    }
}

fn render_chip(
    scene: &mut Scene,
    bounds: Rect,
    label: &str,
    is_active: bool,
    theme: &WorkbenchThemeStyles,
    font: &FontMetrics,
) {
    let bg = if is_active {
        theme.list_active_selection_bg
    } else {
        rustcode_platform::render::Rgba::new(0.0, 0.0, 0.0, 0.0)
    };

    let fg = if is_active {
        theme.command_palette_fg
    } else {
        theme.tab_inactive_fg
    };

    scene.push_quad(Quad::new(bounds).with_background(bg).with_corner_radius(2.0));

    let text_w = font.text_width(label);
    let text_x = bounds.min_x() + (bounds.width() - text_w) / 2.0;
    let text_y = bounds.min_y() + (bounds.height() - font.line_height) / 2.0;

    scene.push_text(GlyphRun::new(
        label.to_string(),
        Point::new(text_x, text_y),
        bounds,
        fg,
        font.font_size * 0.75,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::render::MouseEvent;

    #[test]
    fn test_search_view_rendering_and_interaction() {
        let mut view = SearchView::new();
        view.set_query("calculate");
        view.query.match_case = true;

        // Mock search results
        view.results = SearchResults {
            file_results: vec![FileSearchResult {
                folder_name: "Core".to_string(),
                folder_path: PathBuf::from("/workspace/core"),
                file_path: PathBuf::from("/workspace/core/math.rs"),
                relative_path: PathBuf::from("math.rs"),
                matches: vec![TextSearchMatch {
                    line_number: 15,
                    start_col: 4,
                    end_col: 13,
                    line_content: "    calculate_sum();".to_string(),
                }],
            }],
            total_matches: 1,
        };

        let rows = view.visible_rows();
        assert_eq!(rows.len(), 2); // 1 header + 1 match line

        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();
        let bounds = Rect::from_xywh(0.0, 0.0, 250.0, 600.0);
        let mut scene = Scene::new();
        view.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("SEARCH"));
        assert!(scene.contains_text("calculate"));
        assert!(scene.contains_text("1 results in 1 files"));

        // Click match line (row 1)
        let click_y = bounds.min_y() + 28.0 + 26.0 + 6.0 + 20.0 + 20.0 + 5.0;
        let click_ev = InputEvent::MouseDown(MouseEvent::new(
            Point::new(50.0, click_y),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = view.handle_event(&click_ev, bounds);
        assert_eq!(res, EventResult::Consumed);

        assert_eq!(
            view.last_action,
            Some(SearchAction::OpenMatch {
                path: PathBuf::from("/workspace/core/math.rs"),
                line: 15,
                col: 4,
            })
        );
    }

    #[test]
    fn test_search_view_invalid_regex_error_and_replace() {
        let temp_dir =
            std::env::temp_dir().join(format!("rustcode_sview_m3_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file = temp_dir.join("main.rs");
        std::fs::write(&file, "let value_1 = 10;\n").unwrap();
        let folders = vec![WorkspaceFolder::new(temp_dir.clone(), None, 0)];

        let mut view = SearchView::new();
        view.set_query("[bad_regex");
        view.query.is_regex = true;
        view.execute_search(&folders);
        assert!(view.last_error.is_some());
        assert_eq!(view.results.total_matches, 0);

        let mut scene = Scene::new();
        view.render(
            Rect::from_xywh(0.0, 0.0, 260.0, 400.0),
            &WorkbenchThemeStyles::dark_default(),
            &FontMetrics::default_monospace(),
            &mut scene,
        );
        assert!(scene.contains_text("Invalid search pattern"));

        view.set_query(r"value_(\d+)");
        view.replace_text = "item_$1".to_string();
        let rep = view.execute_replace(&folders).unwrap();
        assert_eq!(rep.files_modified, 1);
        assert_eq!(rep.total_replacements, 1);
        assert!(view.last_error.is_none());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "let item_1 = 10;\n");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
