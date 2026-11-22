//! Problems panel view rendering grouped diagnostics, file hierarchies, and jump-to-location.

use parking_lot::RwLock;
use rustcode_editor::cursor::Position;
use rustcode_editor::viewmodel::{DiagnosticSeverity, EditorDiagnostic};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, Point, Quad, Rect, Rgba, Scene, View,
    WorkbenchThemeStyles,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProblemEntry {
    pub uri: String,
    pub diagnostic: EditorDiagnostic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProblemsAction {
    JumpToLocation {
        uri: String,
        position: Position,
    },
    ToggleFile {
        uri: String,
        collapsed: bool,
    },
    FilterChanged(String),
}

pub type ProblemRowBound = (String, Option<usize>, Rect);

pub struct ProblemsPanel {
    /// Diagnostics grouped by file/URI
    pub files: BTreeMap<String, Vec<EditorDiagnostic>>,
    /// Files whose group rows are collapsed
    pub collapsed_files: HashSet<String>,
    /// Selected problem: (file, index)
    pub selected: Option<(String, usize)>,
    /// Filter query text
    pub filter: String,
    pub last_action: Option<ProblemsAction>,
    pub scroll_top: f32,
    /// Cached layout rows for mouse hit testing: (file, maybe_diag_index, row_rect)
    pub row_bounds: Arc<RwLock<Vec<ProblemRowBound>>>,
}

impl Default for ProblemsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ProblemsPanel {
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            collapsed_files: HashSet::new(),
            selected: None,
            filter: String::new(),
            last_action: None,
            scroll_top: 0.0,
            row_bounds: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn set_file_diagnostics(&mut self, uri: &str, diags: Vec<EditorDiagnostic>) {
        if diags.is_empty() {
            self.files.remove(uri);
            self.collapsed_files.remove(uri);
        } else {
            self.files.insert(uri.to_string(), diags);
        }
    }

    pub fn set_lsp_diagnostics(&mut self, uri: &str, diags: &[rustcode_protocols::lsp::types::LspDiagnostic]) {
        let editor_diags: Vec<EditorDiagnostic> = diags.iter().map(lsp_diagnostic_to_editor).collect();
        self.set_file_diagnostics(uri, editor_diags);
    }

    pub fn clear_file_diagnostics(&mut self, uri: &str) {
        self.files.remove(uri);
        self.collapsed_files.remove(uri);
    }

    pub fn clear_all(&mut self) {
        self.files.clear();
        self.collapsed_files.clear();
        self.selected = None;
    }

    pub fn total_errors(&self) -> usize {
        self.files
            .values()
            .flat_map(|list| list.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count()
    }

    pub fn total_warnings(&self) -> usize {
        self.files
            .values()
            .flat_map(|list| list.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .count()
    }

    pub fn total_infos(&self) -> usize {
        self.files
            .values()
            .flat_map(|list| list.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Information)
            .count()
    }

    pub fn total_count(&self) -> usize {
        self.files.values().map(|list| list.len()).sum()
    }

    pub fn status_badge_text(&self) -> String {
        let errs = self.total_errors();
        let warns = self.total_warnings();
        format!("$(error) {}  $(warning) {}", errs, warns)
    }

    pub fn toggle_file_collapsed(&mut self, uri: &str) {
        if self.collapsed_files.contains(uri) {
            self.collapsed_files.remove(uri);
        } else {
            self.collapsed_files.insert(uri.to_string());
        }
    }

    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_lowercase();
    }

    /// Return diagnostics matching current filter, grouped by file, retaining original file index
    pub fn filtered_files(&self) -> BTreeMap<String, Vec<(usize, &EditorDiagnostic)>> {
        let mut map = BTreeMap::new();
        let f = self.filter.trim();

        for (uri, list) in &self.files {
            let matched: Vec<(usize, &EditorDiagnostic)> = list
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    if f.is_empty() {
                        return true;
                    }
                    uri.to_lowercase().contains(f)
                        || d.message.to_lowercase().contains(f)
                        || d.source.as_deref().unwrap_or("").to_lowercase().contains(f)
                        || d.code.as_deref().unwrap_or("").to_lowercase().contains(f)
                })
                .collect();

            if !matched.is_empty() {
                map.insert(uri.clone(), matched);
            }
        }
        map
    }
}

pub fn lsp_diagnostic_to_editor(diag: &rustcode_protocols::lsp::types::LspDiagnostic) -> EditorDiagnostic {
    let sev = match diag.severity {
        Some(rustcode_protocols::lsp::types::DiagnosticSeverity::Error) => DiagnosticSeverity::Error,
        Some(rustcode_protocols::lsp::types::DiagnosticSeverity::Warning) => DiagnosticSeverity::Warning,
        Some(rustcode_protocols::lsp::types::DiagnosticSeverity::Information) => DiagnosticSeverity::Information,
        Some(rustcode_protocols::lsp::types::DiagnosticSeverity::Hint) => DiagnosticSeverity::Hint,
        None => DiagnosticSeverity::Error,
    };
    let mut ed = EditorDiagnostic::new(
        rustcode_editor::cursor::Range::new(
            rustcode_editor::cursor::Position::new(diag.range.start.line as usize, diag.range.start.character as usize),
            rustcode_editor::cursor::Position::new(diag.range.end.line as usize, diag.range.end.character as usize),
        ),
        sev,
        &diag.message,
    );
    if let Some(src) = &diag.source {
        ed = ed.with_source(src);
    }
    if let Some(code) = &diag.code {
        ed = ed.with_code(code.to_string());
    }
    ed
}

impl View for ProblemsPanel {
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

        // 1. Panel base background
        scene.push_finite_quad(Quad::new(bounds).with_background(theme.panel_bg));

        let toolbar_h = 24.0f32;
        let toolbar_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), toolbar_h);
        scene.push_finite_quad(
            Quad::new(toolbar_rect)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        // Summary badges in toolbar
        let err_count = self.total_errors();
        let warn_count = self.total_warnings();
        let info_count = self.total_infos();

        let summary_text = format!(
            "{} Errors, {} Warnings, {} Info",
            err_count, warn_count, info_count
        );
        scene.push_finite_text(
            GlyphRun::new(
                summary_text,
                Point::new(bounds.min_x() + 10.0, bounds.min_y() + 4.0),
                Rect::from_xywh(bounds.min_x() + 10.0, bounds.min_y() + 4.0, 300.0, font.line_height),
                theme.sidebar_header_fg,
                font.font_size * 0.85,
            ),
        );

        // Filter notice if filter applied
        if !self.filter.is_empty() {
            let filter_str = format!("Filter: \"{}\"", self.filter);
            let filter_w = font.text_width(&filter_str);
            scene.push_finite_text(
                GlyphRun::new(
                    filter_str,
                    Point::new(bounds.max_x() - filter_w - 12.0, bounds.min_y() + 4.0),
                    Rect::from_xywh(
                        bounds.max_x() - filter_w - 12.0,
                        bounds.min_y() + 4.0,
                        filter_w,
                        font.line_height,
                    ),
                    theme.editor_info_foreground,
                    font.font_size * 0.85,
                ),
            );
        }

        // 2. Diagnostics list
        let mut row_cache = Vec::new();
        let row_h = 22.0f32;
        let content_w = bounds.width();
        let content_top = bounds.min_y() + toolbar_h;
        let content_bottom = bounds.max_y();

        let filtered = self.filtered_files();
        let total_rows: usize = filtered
            .iter()
            .map(|(uri, diags)| {
                1 + if self.collapsed_files.contains(uri) { 0 } else { diags.len() }
            })
            .sum();
        let visible_h = (bounds.height() - toolbar_h).max(0.0);
        let max_scroll = ((total_rows as f32 * row_h) - visible_h).max(0.0);
        let max_scroll = if max_scroll.is_finite() { max_scroll } else { 0.0 };
        let safe_scroll = if self.scroll_top.is_finite() {
            self.scroll_top
        } else {
            0.0
        };
        let clamped_scroll = safe_scroll.clamp(0.0, max_scroll);
        let mut cur_y = content_top + 4.0 - clamped_scroll;
        if !cur_y.is_finite() {
            *self.row_bounds.write() = row_cache;
            return;
        }

        if filtered.is_empty() {
            let empty_text = if self.files.is_empty() {
                "No problems have been detected in the workspace."
            } else {
                "No problems match the current filter."
            };
            scene.push_finite_text(
                GlyphRun::new(
                    empty_text.to_string(),
                    Point::new(bounds.min_x() + 16.0, content_top + 8.0),
                    Rect::from_xywh(bounds.min_x() + 16.0, content_top + 8.0, 400.0, font.line_height),
                    theme.editor_line_number,
                    font.font_size * 0.9,
                ),
            );
        } else {
            for (uri, diags) in filtered {
                if !cur_y.is_finite() || !(cur_y + row_h).is_finite() {
                    break;
                }
                let is_collapsed = self.collapsed_files.contains(&uri);
                let file_row_rect = Rect::from_xywh(bounds.min_x(), cur_y, content_w, row_h);

                if cur_y + row_h >= content_top && cur_y < content_bottom {
                    if file_row_rect.is_finite_positive() {
                        row_cache.push((uri.clone(), None, file_row_rect));
                    }

                    // File header row
                    let arrow = if is_collapsed { "▶ " } else { "▼ " };
                    let file_label = format!("{}{}", arrow, uri);
                    let badge = format!(" ({})", diags.len());

                    let file_w = font.text_width(&file_label);
                    let badge_w = font.text_width(&badge);

                    scene.push_finite_text(
                        GlyphRun::new(
                            file_label,
                            Point::new(bounds.min_x() + 8.0, cur_y + 3.0),
                            Rect::from_xywh(bounds.min_x() + 8.0, cur_y + 3.0, file_w, font.line_height),
                            theme.editor_foreground,
                            font.font_size * 0.9,
                        )
                        .with_style(true, false, false),
                    );

                    scene.push_finite_text(
                        GlyphRun::new(
                            badge,
                            Point::new(bounds.min_x() + 8.0 + file_w, cur_y + 3.0),
                            Rect::from_xywh(bounds.min_x() + 8.0 + file_w, cur_y + 3.0, badge_w, font.line_height),
                            theme.editor_line_number,
                            font.font_size * 0.85,
                        ),
                    );
                }

                cur_y += row_h;

                // Problem items under this file
                if !is_collapsed {
                    for (orig_idx, diag) in diags {
                        if !cur_y.is_finite() || !(cur_y + row_h).is_finite() {
                            break;
                        }
                        let diag_rect = Rect::from_xywh(bounds.min_x(), cur_y, content_w, row_h);

                        if cur_y + row_h >= content_top && cur_y < content_bottom {
                            if diag_rect.is_finite_positive() {
                                row_cache.push((uri.clone(), Some(orig_idx), diag_rect));
                            }

                            let is_selected = self.selected.as_ref().is_some_and(|(u, i)| u == &uri && *i == orig_idx);
                            if is_selected {
                                scene.push_finite_quad(
                                    Quad::new(diag_rect)
                                        .with_background(theme.list_active_selection_bg)
                                        .with_corner_radius(2.0),
                                );
                            }

                            // Severity badge/tag
                            let (sev_str, sev_color) = match diag.severity {
                                DiagnosticSeverity::Error => ("[Error] ", theme.editor_error_foreground),
                                DiagnosticSeverity::Warning => ("[Warning] ", theme.editor_warning_foreground),
                                DiagnosticSeverity::Information => ("[Info] ", theme.editor_info_foreground),
                                DiagnosticSeverity::Hint => ("[Hint] ", theme.editor_hint_foreground),
                            };

                            let pos_str = format!("[{}:{}] ", diag.range.start.line + 1, diag.range.start.column + 1);
                            let source_str = diag.formatted_header();

                            let sev_w = font.text_width(sev_str);
                            let pos_w = font.text_width(&pos_str);
                            let msg_w = font.text_width(&diag.message);
                            let src_w = font.text_width(&source_str);

                            let mut item_x = bounds.min_x() + 24.0;

                            // 1. Severity
                            scene.push_finite_text(
                                GlyphRun::new(
                                    sev_str.to_string(),
                                    Point::new(item_x, cur_y + 3.0),
                                    Rect::from_xywh(item_x, cur_y + 3.0, sev_w, font.line_height),
                                    sev_color,
                                    font.font_size * 0.85,
                                )
                                .with_style(true, false, false),
                            );
                            item_x += sev_w;

                            // 2. Position
                            scene.push_finite_text(
                                GlyphRun::new(
                                    pos_str,
                                    Point::new(item_x, cur_y + 3.0),
                                    Rect::from_xywh(item_x, cur_y + 3.0, pos_w, font.line_height),
                                    theme.editor_line_number,
                                    font.font_size * 0.85,
                                ),
                            );
                            item_x += pos_w;

                            // 3. Message
                            let msg_color = if is_selected {
                                Rgba::from_hex("#ffffff").unwrap()
                            } else {
                                theme.editor_foreground
                            };
                            scene.push_finite_text(
                                GlyphRun::new(
                                    diag.message.clone(),
                                    Point::new(item_x, cur_y + 3.0),
                                    Rect::from_xywh(item_x, cur_y + 3.0, msg_w, font.line_height),
                                    msg_color,
                                    font.font_size * 0.85,
                                ),
                            );
                            item_x += msg_w + 8.0;

                            // 4. Source & Code
                            scene.push_finite_text(
                                GlyphRun::new(
                                    source_str,
                                    Point::new(item_x, cur_y + 3.0),
                                    Rect::from_xywh(item_x, cur_y + 3.0, src_w, font.line_height),
                                    theme.editor_line_number,
                                    font.font_size * 0.8,
                                ),
                            );
                        }

                        cur_y += row_h;
                    }
                }
            }
        }

        *self.row_bounds.write() = row_cache;
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if !bounds.is_finite_positive() {
            return EventResult::Ignored;
        }
        match event {
            InputEvent::ScrollWheel(scroll) => {
                if !scroll.position.x.is_finite()
                    || !scroll.position.y.is_finite()
                    || !bounds.contains(scroll.position)
                {
                    return EventResult::Ignored;
                }
                let safe_scroll = if self.scroll_top.is_finite() {
                    self.scroll_top.max(0.0)
                } else {
                    0.0
                };
                if !scroll.delta_y.is_finite() {
                    self.scroll_top = safe_scroll;
                    return EventResult::Ignored;
                }
                let next = (safe_scroll - scroll.delta_y).max(0.0);
                self.scroll_top = if next.is_finite() {
                    next
                } else {
                    safe_scroll
                };
                EventResult::Consumed
            }
            InputEvent::MouseDown(mouse) => {
                if !mouse.position.x.is_finite()
                    || !mouse.position.y.is_finite()
                    || !bounds.contains(mouse.position)
                {
                    return EventResult::Ignored;
                }

                let rows = self.row_bounds.read().clone();
                for (uri, maybe_idx, row_rect) in rows {
                    if row_rect.is_finite_positive() && row_rect.contains(mouse.position) {
                        match maybe_idx {
                            None => {
                                // Clicked file group row: toggle collapsed
                                self.toggle_file_collapsed(&uri);
                                let is_collapsed = self.collapsed_files.contains(&uri);
                                self.last_action = Some(ProblemsAction::ToggleFile {
                                    uri,
                                    collapsed: is_collapsed,
                                });
                                return EventResult::Consumed;
                            }
                            Some(idx) => {
                                // Clicked diagnostic row: select and jump to location
                                self.selected = Some((uri.clone(), idx));
                                if let Some(diags) = self.files.get(&uri) {
                                    if let Some(diag) = diags.get(idx) {
                                        self.last_action = Some(ProblemsAction::JumpToLocation {
                                            uri: uri.clone(),
                                            position: diag.range.start,
                                        });
                                        return EventResult::Consumed;
                                    }
                                }
                            }
                        }
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
    use rustcode_editor::cursor::Range;

    #[test]
    fn test_problems_panel_grouping_and_counts() {
        let mut panel = ProblemsPanel::new();

        panel.set_file_diagnostics(
            "src/main.rs",
            vec![
                EditorDiagnostic::error(
                    Range::new(Position::new(10, 4), Position::new(10, 15)),
                    "cannot find value `x`",
                )
                .with_source("rust-analyzer")
                .with_code("E0425"),
                EditorDiagnostic::warning(
                    Range::new(Position::new(20, 8), Position::new(20, 12)),
                    "unused variable: `y`",
                )
                .with_source("rustc")
                .with_code("unused_variables"),
            ],
        );

        panel.set_file_diagnostics(
            "src/lib.rs",
            vec![EditorDiagnostic::info(
                Range::new(Position::new(5, 0), Position::new(5, 10)),
                "consider adding doc comment",
            )],
        );

        assert_eq!(panel.total_errors(), 1);
        assert_eq!(panel.total_warnings(), 1);
        assert_eq!(panel.total_infos(), 1);
        assert_eq!(panel.total_count(), 3);
        assert_eq!(panel.status_badge_text(), "$(error) 1  $(warning) 1");
    }

    #[test]
    fn test_problems_panel_toggle_and_filter() {
        let mut panel = ProblemsPanel::new();
        panel.set_file_diagnostics(
            "src/main.rs",
            vec![EditorDiagnostic::error(
                Range::new(Position::new(1, 1), Position::new(1, 5)),
                "type mismatch",
            )],
        );

        assert!(!panel.collapsed_files.contains("src/main.rs"));
        panel.toggle_file_collapsed("src/main.rs");
        assert!(panel.collapsed_files.contains("src/main.rs"));
        panel.toggle_file_collapsed("src/main.rs");
        assert!(!panel.collapsed_files.contains("src/main.rs"));

        // Filter tests
        panel.set_filter("type");
        assert_eq!(panel.filtered_files().len(), 1);

        panel.set_filter("nomatch");
        assert_eq!(panel.filtered_files().len(), 0);
    }

    #[test]
    fn test_problems_panel_render_and_jump_click() {
        let mut panel = ProblemsPanel::new();
        panel.set_file_diagnostics(
            "src/app.rs",
            vec![EditorDiagnostic::error(
                Range::new(Position::new(15, 2), Position::new(15, 8)),
                "unknown identifier `foo`",
            )
            .with_source("rust-analyzer")
            .with_code("E0425")],
        );

        let bounds = Rect::from_xywh(0.0, 300.0, 800.0, 200.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let mut scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);

        assert!(scene.contains_text("src/app.rs"));
        assert!(scene.contains_text("unknown identifier `foo`"));
        assert!(scene.contains_text("[Error]"));
        assert!(scene.contains_text("[16:3]"));

        // Click on the diagnostic row (approx y = 300 + 24 + 4 + 22 + 5 = 355)
        let click_event = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(50.0, 355.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = panel.handle_event(&click_event, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(
            panel.last_action,
            Some(ProblemsAction::JumpToLocation {
                uri: "src/app.rs".to_string(),
                position: Position::new(15, 2),
            })
        );
    }

    #[test]
    fn test_problems_panel_filter_click_jump_accuracy() {
        let mut panel = ProblemsPanel::new();
        // 3 problems in one file: index 0 is warning, index 1 is error, index 2 is info
        panel.set_file_diagnostics(
            "src/service.rs",
            vec![
                EditorDiagnostic::warning(
                    Range::new(Position::new(5, 0), Position::new(5, 10)),
                    "unused import `std::io`",
                ),
                EditorDiagnostic::error(
                    Range::new(Position::new(42, 4), Position::new(42, 20)),
                    "cannot find value `database`",
                )
                .with_source("rust-analyzer")
                .with_code("E0425"),
                EditorDiagnostic::info(
                    Range::new(Position::new(100, 0), Position::new(100, 5)),
                    "todo: implement caching",
                ),
            ],
        );

        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        // Filter for "database" - should match ONLY index 1
        panel.set_filter("database");
        let filtered = panel.filtered_files();
        assert_eq!(filtered["src/service.rs"].len(), 1);
        assert_eq!(filtered["src/service.rs"][0].0, 1); // original index 1

        let mut scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);

        // Click the single displayed diagnostic row under the file header (y approx 0 + 24 + 4 + 22 + 5 = 55)
        let click_event = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(50.0, 55.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = panel.handle_event(&click_event, bounds);
        assert_eq!(res, EventResult::Consumed);

        // Crucial verification: Must jump to Position(42, 4), NOT Position(5, 0) (index 0)
        assert_eq!(
            panel.last_action,
            Some(ProblemsAction::JumpToLocation {
                uri: "src/service.rs".to_string(),
                position: Position::new(42, 4),
            })
        );
        assert_eq!(panel.selected, Some(("src/service.rs".to_string(), 1)));
    }

    #[test]
    fn test_problems_panel_scroll_wheel() {
        let mut panel = ProblemsPanel::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 400.0);

        let scroll_event = InputEvent::ScrollWheel(rustcode_platform::render::ScrollEvent {
            position: Point::new(100.0, 100.0),
            delta_x: 0.0,
            delta_y: -44.0, // scroll down
            modifiers: rustcode_platform::Modifiers::none(),
        });

        let res = panel.handle_event(&scroll_event, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(panel.scroll_top, 44.0);
    }

    #[test]
    fn test_problems_panel_lsp_diagnostics_conversion() {
        use rustcode_protocols::lsp::types::{DiagnosticCode, DiagnosticSeverity as LspSeverity, LspDiagnostic, LspPosition, LspRange};

        let mut panel = ProblemsPanel::new();
        let lsp_diag = LspDiagnostic {
            range: LspRange::new(LspPosition::new(10, 5), LspPosition::new(10, 15)),
            severity: Some(LspSeverity::Error),
            message: "mismatched types".to_string(),
            source: Some("rust-analyzer".to_string()),
            code: Some(DiagnosticCode::String("E0308".to_string())),
        };

        panel.set_lsp_diagnostics("file:///src/main.rs", &[lsp_diag]);

        assert_eq!(panel.total_errors(), 1);
        let diags = panel.files.get("file:///src/main.rs").unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
        assert_eq!(diags[0].range.start, Position::new(10, 5));
        assert_eq!(diags[0].range.end, Position::new(10, 15));
        assert_eq!(diags[0].source.as_deref(), Some("rust-analyzer"));
        assert_eq!(diags[0].code.as_deref(), Some("E0308"));
    }

    #[test]
    fn test_problems_panel_non_finite_scroll_and_bounds_safe() {
        use rustcode_platform::render::{ScrollEvent, Size};

        let mut panel = ProblemsPanel::new();
        panel.set_file_diagnostics(
            "src/lib.rs",
            vec![EditorDiagnostic::error(
                Range::new(Position::new(1, 0), Position::new(1, 4)),
                "err",
            )],
        );
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();
        let valid_bounds = Rect::from_xywh(0.0, 0.0, 800.0, 400.0);

        panel.scroll_top = 12.0;
        for bad_dy in [f32::NEG_INFINITY, f32::INFINITY, f32::NAN] {
            panel.handle_event(
                &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(100.0, 100.0), 0.0, bad_dy)),
                valid_bounds,
            );
            assert!(panel.scroll_top.is_finite());
        }
        assert_eq!(panel.scroll_top, 12.0);

        for bad_bounds in [
            Rect::new(Point::zero(), Size::new(800.0, f32::NAN)),
            Rect::from_xywh(0.0, 0.0, f32::INFINITY, 400.0),
        ] {
            let mut scene = Scene::new();
            panel.render(bad_bounds, &theme, &font, &mut scene);
            assert!(scene.is_empty());
        }
    }
}
