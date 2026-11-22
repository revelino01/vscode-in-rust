//! Interactive Terminal Panel View backed by `rustcode-terminal`.

use parking_lot::RwLock;
use rustcode_platform::accessibility::{AccessibleNode, AccessibleRole};
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, KeyEvent, Point, Quad, Rect,
    Rgba, Scene, View, WorkbenchThemeStyles,
};
use rustcode_terminal::{TerminalGridSnapshot, TerminalManager};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalAction {
    NewTerminal {
        id: String,
    },
    SplitTerminal {
        id: String,
    },
    CloseTerminal {
        id: String,
    },
    SelectTab {
        tab_index: usize,
    },
    FocusPane {
        id: String,
    },
    CommandExecuted {
        terminal_id: String,
        command: String,
        exit_code: Option<i32>,
    },
    ClearTerminal {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalClickTarget {
    NewButton,
    SplitButton,
    ClearButton,
    KillButton,
    SelectTab(usize),
    CloseTab(usize),
    FocusPane(String),
}

#[derive(Debug, Clone)]
struct TerminalPanelLayout {
    header_rect: Rect,
    body_rect: Rect,
    tab_rects: Vec<(usize, Rect, Rect)>,
    new_btn_rect: Rect,
    split_btn_rect: Rect,
    clear_btn_rect: Rect,
    kill_btn_rect: Rect,
    pane_rects: Vec<(String, Rect)>,
}

pub struct TerminalPanelView {
    pub manager: TerminalManager,
    pub scroll_offsets: HashMap<String, f32>,
    pub auto_scroll: HashMap<String, bool>,
    pub is_focused: bool,
    pub default_cwd: Option<PathBuf>,
    pub default_shell: Option<String>,
    pub last_action: Option<TerminalAction>,
    pub click_targets: RwLock<Vec<(Rect, TerminalClickTarget)>>,
    last_font: RwLock<FontMetrics>,
    last_body_height: RwLock<f32>,
    last_layout_bounds: RwLock<Option<Rect>>,
}

impl Default for TerminalPanelView {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalPanelView {
    pub fn new() -> Self {
        let mut view = Self::empty();
        view.ensure_default_terminal(None);
        view
    }

    pub fn empty() -> Self {
        Self {
            manager: TerminalManager::new(),
            scroll_offsets: HashMap::new(),
            auto_scroll: HashMap::new(),
            is_focused: true,
            default_cwd: None,
            default_shell: None,
            last_action: None,
            click_targets: RwLock::new(Vec::new()),
            last_font: RwLock::new(FontMetrics::default_monospace()),
            last_body_height: RwLock::new(160.0),
            last_layout_bounds: RwLock::new(None),
        }
    }

    pub fn with_default_cwd(mut self, cwd: PathBuf) -> Self {
        self.default_cwd = Some(cwd.clone());
        if let Some(term) = self.manager.active_terminal_mut() {
            term.cwd = cwd;
        }
        self
    }

    pub fn has_active_terminal(&self) -> bool {
        self.manager.active_terminal().is_some()
    }

    pub fn ensure_default_terminal(&mut self, cwd: Option<PathBuf>) -> String {
        if let Some(active) = self.manager.active_terminal() {
            return active.id.clone();
        }
        let resolved_cwd = cwd.or_else(|| self.default_cwd.clone());
        let shell = self.default_shell.as_deref();
        let id = self
            .manager
            .create_terminal(Some("sh"), shell, resolved_cwd)
            .id
            .clone();
        self.auto_scroll.insert(id.clone(), true);
        id
    }

    pub fn create_terminal(&mut self, title: Option<&str>, shell: Option<&str>, cwd: Option<PathBuf>) -> String {
        let resolved_cwd = cwd.or_else(|| self.default_cwd.clone());
        let resolved_shell = shell.or(self.default_shell.as_deref());
        let id = self
            .manager
            .create_terminal(title.or(Some("sh")), resolved_shell, resolved_cwd)
            .id
            .clone();
        self.auto_scroll.insert(id.clone(), true);
        self.is_focused = true;
        self.last_action = Some(TerminalAction::NewTerminal { id: id.clone() });
        id
    }

    pub fn split_active_terminal(&mut self) -> String {
        let resolved_cwd = self.default_cwd.clone();
        let resolved_shell = self.default_shell.as_deref();
        let id = self
            .manager
            .split_active_terminal(Some("sh"), resolved_shell, resolved_cwd)
            .id
            .clone();
        self.auto_scroll.insert(id.clone(), true);
        self.is_focused = true;
        self.last_action = Some(TerminalAction::SplitTerminal { id: id.clone() });
        id
    }

    pub fn kill_active_terminal(&mut self) -> Option<String> {
        let closed = self.manager.close_active_terminal()?;
        self.scroll_offsets.remove(&closed);
        self.auto_scroll.remove(&closed);
        self.last_action = Some(TerminalAction::CloseTerminal { id: closed.clone() });
        Some(closed)
    }

    pub fn compute_grid_size(pane_bounds: Rect, font: &FontMetrics) -> (u16, u16) {
        let pad_x = 10.0;
        let pad_y = 4.0;
        let char_w = if font.char_width.is_finite() && font.char_width > 0.0 {
            font.char_width
        } else {
            8.0
        };
        let line_h = if font.line_height.is_finite() && font.line_height > 0.0 {
            font.line_height
        } else {
            18.0
        };
        let usable_w = (pane_bounds.width() - pad_x * 2.0).max(char_w);
        let usable_h = (pane_bounds.height() - pad_y * 2.0).max(line_h);
        let cols = (usable_w / char_w).floor().clamp(1.0, u16::MAX as f32) as u16;
        let rows = (usable_h / line_h).floor().clamp(1.0, u16::MAX as f32) as u16;
        (rows, cols)
    }

    pub fn spawn_pty_shell(
        &mut self,
        title: Option<&str>,
        shell: Option<&str>,
        cwd: Option<PathBuf>,
    ) -> Result<String, String> {
        let resolved_cwd = cwd.or_else(|| self.default_cwd.clone());
        let resolved_shell = shell.or(self.default_shell.as_deref());
        let font = self.last_font.read().clone();
        let (rows, cols) = if let Some(bounds) = *self.last_layout_bounds.read() {
            let layout = self.compute_layout(bounds, &font);
            Self::compute_grid_size(layout.body_rect, &font)
        } else {
            (
                rustcode_terminal::DEFAULT_TERMINAL_ROWS,
                rustcode_terminal::DEFAULT_TERMINAL_COLS,
            )
        };
        let id = self
            .manager
            .create_pty_terminal(
                title.or(Some("sh")),
                resolved_shell,
                resolved_cwd,
                rows,
                cols,
            )?
            .id
            .clone();
        self.auto_scroll.insert(id.clone(), true);
        self.is_focused = true;
        self.last_action = Some(TerminalAction::NewTerminal { id: id.clone() });
        Ok(id)
    }

    pub fn split_pty_terminal(&mut self) -> Result<String, String> {
        let resolved_cwd = self.default_cwd.clone();
        let resolved_shell = self.default_shell.as_deref();
        let id = self
            .manager
            .split_active_terminal(Some("sh"), resolved_shell, resolved_cwd)
            .id
            .clone();
        let font = self.last_font.read().clone();
        let (rows, cols) = if let Some(bounds) = *self.last_layout_bounds.read() {
            let layout = self.compute_layout(bounds, &font);
            let pane_rect = layout
                .pane_rects
                .iter()
                .find(|(pid, _)| pid == &id)
                .map(|(_, r)| *r)
                .unwrap_or(layout.body_rect);
            Self::compute_grid_size(pane_rect, &font)
        } else {
            (
                rustcode_terminal::DEFAULT_TERMINAL_ROWS,
                rustcode_terminal::DEFAULT_TERMINAL_COLS,
            )
        };
        let spawn_res = self
            .manager
            .terminal_mut(&id)
            .ok_or_else(|| format!("Split terminal '{}' missing", id))?
            .spawn_pty_shell(rows, cols);
        if let Err(err) = spawn_res {
            self.manager.close_terminal(&id);
            return Err(err);
        }
        let cached_bounds = *self.last_layout_bounds.read();
        if let Some(bounds) = cached_bounds {
            self.resize_to_bounds(bounds);
        }
        self.auto_scroll.insert(id.clone(), true);
        self.is_focused = true;
        self.last_action = Some(TerminalAction::SplitTerminal { id: id.clone() });
        Ok(id)
    }

    pub fn poll_all_ptys(&mut self) -> Vec<String> {
        let updated = self.manager.poll_all_ptys();
        for id in &updated {
            if self.auto_scroll.get(id).copied().unwrap_or(true) {
                if let Some(term) = self.manager.terminal_mut(id) {
                    term.grid_mut().scroll_to_bottom();
                }
            }
        }
        updated
    }

    pub fn resize_to_bounds(&mut self, bounds: Rect) -> usize {
        if !bounds.is_finite_positive() {
            return 0;
        }
        let font = self.last_font.read().clone();
        if !font.is_finite() {
            return 0;
        }
        let layout = self.compute_layout(bounds, &font);
        *self.last_layout_bounds.write() = Some(bounds);
        *self.last_body_height.write() = layout.body_rect.height();

        let mut pane_sizes: HashMap<String, (u16, u16)> = HashMap::new();
        for (term_id, pane_rect) in &layout.pane_rects {
            pane_sizes.insert(term_id.clone(), Self::compute_grid_size(*pane_rect, &font));
        }
        let fallback_size = Self::compute_grid_size(layout.body_rect, &font);

        let term_ids: Vec<String> = self
            .manager
            .terminals()
            .iter()
            .map(|t| t.id.clone())
            .collect();
        let mut resized_count = 0;
        for term_id in term_ids {
            let (rows, cols) = pane_sizes.get(&term_id).copied().unwrap_or(fallback_size);
            if let Some(term) = self.manager.terminal_mut(&term_id) {
                if term.rows != rows || term.cols != cols {
                    let _ = term.resize(rows, cols);
                    resized_count += 1;
                }
            }
        }
        resized_count
    }

    pub fn new_terminal(&mut self) -> String {
        self.create_terminal(Some("sh"), None, None)
    }

    pub fn close_terminal(&mut self) -> Option<String> {
        self.kill_active_terminal()
    }

    pub fn switch_tab(&mut self, tab_index: usize) -> bool {
        if self.manager.set_active_tab(tab_index) {
            self.is_focused = true;
            self.last_action = Some(TerminalAction::SelectTab { tab_index });
            true
        } else {
            false
        }
    }

    pub fn split_terminal(&mut self) -> String {
        self.split_active_terminal()
    }

    pub fn clear_active(&mut self) -> bool {
        let Some(term) = self.manager.active_terminal_mut() else {
            return false;
        };
        let id = term.id.clone();
        term.clear();
        self.scroll_offsets.insert(id.clone(), 0.0);
        self.auto_scroll.insert(id.clone(), true);
        self.last_action = Some(TerminalAction::ClearTerminal { id });
        true
    }

    pub fn send_text_to_terminal(
        &mut self,
        id: &str,
        text: &str,
        add_new_line: bool,
    ) -> Result<(), String> {
        let Some(term) = self.manager.terminal_mut(id) else {
            return Err(format!("Terminal '{}' not found", id));
        };
        if term.has_pty() {
            if add_new_line {
                let trimmed = text.trim_end_matches(['\r', '\n']);
                term.send_pty_str(&format!("{}\r", trimmed))?;
            } else {
                term.send_pty_str(text)?;
            }
            let _ = term.poll_pty();
        } else {
            term.insert_str(text);
            if add_new_line {
                term.submit_input();
            }
        }
        self.auto_scroll.insert(id.to_string(), true);
        Ok(())
    }

    pub fn handle_terminal_action(&mut self, action: &TerminalAction) -> bool {
        match action {
            TerminalAction::NewTerminal { .. } => {
                self.new_terminal();
                true
            }
            TerminalAction::SplitTerminal { .. } => {
                self.split_terminal();
                true
            }
            TerminalAction::CloseTerminal { id } => {
                if id.is_empty() {
                    self.close_terminal().is_some()
                } else if self.manager.close_terminal(id) {
                    self.scroll_offsets.remove(id);
                    self.auto_scroll.remove(id);
                    self.last_action = Some(TerminalAction::CloseTerminal { id: id.clone() });
                    true
                } else {
                    false
                }
            }
            TerminalAction::SelectTab { tab_index } => self.switch_tab(*tab_index),
            TerminalAction::FocusPane { id } => {
                if self.manager.set_active_terminal(id) {
                    self.is_focused = true;
                    self.last_action = Some(TerminalAction::FocusPane { id: id.clone() });
                    true
                } else {
                    false
                }
            }
            TerminalAction::ClearTerminal { id } => {
                if id.is_empty() {
                    self.clear_active()
                } else if let Some(term) = self.manager.terminal_mut(id) {
                    term.clear();
                    self.scroll_offsets.insert(id.clone(), 0.0);
                    self.auto_scroll.insert(id.clone(), true);
                    self.last_action = Some(TerminalAction::ClearTerminal { id: id.clone() });
                    true
                } else {
                    false
                }
            }
            TerminalAction::CommandExecuted {
                terminal_id,
                command,
                ..
            } => self.send_text_to_terminal(terminal_id, command, true).is_ok(),
        }
    }

    fn tab_label(&self, tab_idx: usize, tab: &rustcode_terminal::TerminalTab) -> String {
        let base_title = tab
            .active_pane_id()
            .and_then(|pid| self.manager.terminal(pid))
            .map(|t| t.title.as_str())
            .unwrap_or("sh");
        if tab.pane_ids.len() > 1 {
            format!("{}: {} [{}]", tab_idx + 1, base_title, tab.pane_ids.len())
        } else {
            format!("{}: {}", tab_idx + 1, base_title)
        }
    }

    fn compute_layout(&self, bounds: Rect, font: &FontMetrics) -> TerminalPanelLayout {
        let header_h = 26.0f32.min(bounds.height().max(0.0));
        let header_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), header_h);
        let body_rect = Rect::from_xywh(
            bounds.min_x(),
            bounds.min_y() + header_h,
            bounds.width(),
            (bounds.height() - header_h).max(0.0),
        );

        let kill_w = 40.0;
        let clear_w = 46.0;
        let split_w = 46.0;
        let new_w = 26.0;
        let gap = 4.0;
        let pad_r = 6.0;

        let kill_x = (header_rect.max_x() - pad_r - kill_w).max(header_rect.min_x());
        let clear_x = (kill_x - gap - clear_w).max(header_rect.min_x());
        let split_x = (clear_x - gap - split_w).max(header_rect.min_x());
        let new_x = (split_x - gap - new_w).max(header_rect.min_x());

        let btn_h = (header_h - 4.0).max(0.0);
        let btn_y = header_rect.min_y() + 2.0;

        let new_btn_rect = Rect::from_xywh(new_x, btn_y, new_w, btn_h);
        let split_btn_rect = Rect::from_xywh(split_x, btn_y, split_w, btn_h);
        let clear_btn_rect = Rect::from_xywh(clear_x, btn_y, clear_w, btn_h);
        let kill_btn_rect = Rect::from_xywh(kill_x, btn_y, kill_w, btn_h);

        let tabs_max_x = (new_x - 6.0).max(header_rect.min_x());
        let mut tab_rects = Vec::new();
        let mut cur_x = header_rect.min_x() + 6.0;
        for (idx, tab) in self.manager.tabs().iter().enumerate() {
            let label = self.tab_label(idx, tab);
            let close_w = 14.0;
            let tab_w = (font.text_width(&label) + 26.0).max(58.0);
            if cur_x + tab_w > tabs_max_x && !tab_rects.is_empty() {
                break;
            }
            let clamped_w = tab_w.min((tabs_max_x - cur_x).max(0.0));
            if clamped_w <= 0.0 {
                break;
            }
            let tab_rect = Rect::from_xywh(cur_x, header_rect.min_y(), clamped_w, header_h);
            let close_rect = Rect::from_xywh(
                (tab_rect.max_x() - close_w - 2.0).max(tab_rect.min_x()),
                header_rect.min_y() + 4.0,
                close_w.min(clamped_w),
                (header_h - 8.0).max(0.0),
            );
            tab_rects.push((idx, tab_rect, close_rect));
            cur_x += clamped_w + 4.0;
        }

        let active_panes = self.manager.active_tab_terminals();
        let mut pane_rects = Vec::new();
        if !active_panes.is_empty() && body_rect.width() > 0.0 && body_rect.height() > 0.0 {
            let count = active_panes.len() as f32;
            let pane_w = body_rect.width() / count;
            for (idx, term) in active_panes.iter().enumerate() {
                let px = body_rect.min_x() + idx as f32 * pane_w;
                let pw = if idx + 1 == active_panes.len() {
                    (body_rect.max_x() - px).max(0.0)
                } else {
                    pane_w
                };
                pane_rects.push((
                    term.id.clone(),
                    Rect::from_xywh(px, body_rect.min_y(), pw, body_rect.height()),
                ));
            }
        }

        TerminalPanelLayout {
            header_rect,
            body_rect,
            tab_rects,
            new_btn_rect,
            split_btn_rect,
            clear_btn_rect,
            kill_btn_rect,
            pane_rects,
        }
    }

    fn build_click_targets(&self, layout: &TerminalPanelLayout) -> Vec<(Rect, TerminalClickTarget)> {
        let mut targets = Vec::new();
        for (idx, tab_rect, close_rect) in &layout.tab_rects {
            targets.push((*tab_rect, TerminalClickTarget::SelectTab(*idx)));
            targets.push((*close_rect, TerminalClickTarget::CloseTab(*idx)));
        }
        targets.push((layout.new_btn_rect, TerminalClickTarget::NewButton));
        targets.push((layout.split_btn_rect, TerminalClickTarget::SplitButton));
        targets.push((layout.clear_btn_rect, TerminalClickTarget::ClearButton));
        targets.push((layout.kill_btn_rect, TerminalClickTarget::KillButton));
        for (id, pane_rect) in &layout.pane_rects {
            targets.push((*pane_rect, TerminalClickTarget::FocusPane(id.clone())));
        }
        targets
    }

    fn compute_max_scroll_lines(&self, term_id: &str, body_h: f32, line_h: f32) -> f32 {
        let Some(term) = self.manager.terminal(term_id) else {
            return 0.0;
        };
        let total_lines = term.lines.len() + if term.exited { 0 } else { 1 };
        let visible_rows = ((body_h - 8.0).max(line_h) / line_h.max(1.0)).floor().max(1.0);
        (total_lines as f32 - visible_rows).max(0.0)
    }

    pub fn accessibility_node(&self, bounds: Rect, panel_focused: bool) -> AccessibleNode {
        if !bounds.is_finite_positive() {
            return AccessibleNode::new(
                "workbench.panel.terminal",
                AccessibleRole::Panel,
                "Terminal",
                Rect::zero(),
            );
        }
        let font = self.last_font.read().clone();
        let layout = self.compute_layout(bounds, &font);

        let mut root = AccessibleNode::new(
            "workbench.panel.terminal",
            AccessibleRole::Panel,
            "Terminal",
            bounds,
        );

        let mut tab_list = AccessibleNode::new(
            "workbench.panel.terminal.tabs",
            AccessibleRole::TabList,
            "Terminal Tabs",
            layout.header_rect,
        );
        let active_tab_idx = self.manager.active_tab_index();
        for (idx, tab_rect, _) in &layout.tab_rects {
            if let Some(tab) = self.manager.tabs().get(*idx) {
                tab_list.add_child(
                    AccessibleNode::new(
                        format!("terminal.tab.{}", idx),
                        AccessibleRole::Tab,
                        self.tab_label(*idx, tab),
                        *tab_rect,
                    )
                    .with_selected(active_tab_idx == Some(*idx)),
                );
            }
        }
        root.add_child(tab_list);

        root.add_child(
            AccessibleNode::new(
                "terminal.action.new",
                AccessibleRole::Button,
                "New Terminal",
                layout.new_btn_rect,
            )
            .with_action("press"),
        );
        root.add_child(
            AccessibleNode::new(
                "terminal.action.split",
                AccessibleRole::Button,
                "Split Terminal",
                layout.split_btn_rect,
            )
            .with_action("press"),
        );
        root.add_child(
            AccessibleNode::new(
                "terminal.action.clear",
                AccessibleRole::Button,
                "Clear Terminal",
                layout.clear_btn_rect,
            )
            .with_action("press"),
        );
        root.add_child(
            AccessibleNode::new(
                "terminal.action.kill",
                AccessibleRole::Button,
                "Kill Terminal",
                layout.kill_btn_rect,
            )
            .with_action("press"),
        );

        let active_id = self.manager.active_terminal().map(|t| t.id.as_str());
        for (term_id, pane_rect) in &layout.pane_rects {
            if let Some(term) = self.manager.terminal(term_id) {
                let is_active_pane = active_id == Some(term.id.as_str());
                root.add_child(
                    AccessibleNode::new(
                        format!("terminal.pane.{}", term.id),
                        AccessibleRole::TextInput,
                        term.title.clone(),
                        *pane_rect,
                    )
                    .with_value(term.input_buffer.clone())
                    .with_description(term.cwd.display().to_string())
                    .with_focused(panel_focused && self.is_focused && is_active_pane)
                    .with_action("input"),
                );
            }
        }

        root
    }
}

fn rgba_from_u8(c: [u8; 4]) -> Rgba {
    Rgba::new(
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        c[3] as f32 / 255.0,
    )
}

fn snapshot_has_vt_formatting(snapshot: &TerminalGridSnapshot) -> bool {
    if snapshot.alt_screen || snapshot.display_offset > 0 {
        return true;
    }
    snapshot.cells.iter().any(|row| {
        row.iter().any(|cell| {
            cell.fg != [212, 212, 212, 255]
                || cell.bg != [30, 30, 30, 255]
                || cell.bold
                || cell.italic
                || cell.underline
        })
    })
}

fn render_grid_snapshot(
    snapshot: &TerminalGridSnapshot,
    pane_rect: Rect,
    show_cursor: bool,
    theme: &WorkbenchThemeStyles,
    font: &FontMetrics,
    scene: &mut Scene,
) {
    let pad_x = 10.0;
    let pad_y = 4.0;
    let line_h = font.line_height.max(1.0);
    let char_w = font.char_width.max(1.0);
    let start_x = pane_rect.min_x() + pad_x;
    let start_y = pane_rect.min_y() + pad_y;
    let default_bg = [30, 30, 30, 255];

    for (row_idx, row_cells) in snapshot.cells.iter().enumerate() {
        let y = start_y + row_idx as f32 * line_h;
        if y > pane_rect.max_y() {
            break;
        }
        let mut col = 0usize;
        while col < row_cells.len() {
            let first = &row_cells[col];
            let style = (first.fg, first.bg, first.bold, first.italic, first.underline);
            let start_col = col;
            let mut text = String::new();
            while col < row_cells.len() {
                let c = &row_cells[col];
                if (c.fg, c.bg, c.bold, c.italic, c.underline) != style {
                    break;
                }
                text.push(c.ch);
                col += 1;
            }
            let run_len = col - start_col;
            let run_x = start_x + start_col as f32 * char_w;
            let run_w = run_len as f32 * char_w;

            if style.1 != default_bg {
                scene.push_finite_quad(
                    Quad::new(Rect::from_xywh(run_x, y, run_w, line_h))
                        .with_background(rgba_from_u8(style.1)),
                );
            }

            let emit_text = if style.4 {
                text
            } else {
                text.trim_end_matches(' ').to_string()
            };
            if !emit_text.is_empty() {
                let text_w = (emit_text.chars().count() as f32 * char_w).max(char_w);
                scene.push_finite_text(
                    GlyphRun::new(
                        emit_text,
                        Point::new(run_x, y),
                        Rect::from_xywh(run_x, y, text_w, line_h),
                        rgba_from_u8(style.0),
                        font.font_size * 0.9,
                    )
                    .with_style(style.2, style.3, style.4),
                );
            }
        }
    }

    if show_cursor && snapshot.cursor_visible {
        let cx = start_x + snapshot.cursor_col as f32 * char_w;
        let cy = start_y + snapshot.cursor_row as f32 * line_h;
        if cy < pane_rect.max_y() {
            scene.push_finite_quad(
                Quad::new(Rect::from_xywh(cx, cy, char_w.max(2.0), line_h))
                    .with_background(theme.editor_cursor),
            );
        }
    }
}

fn key_event_to_char(key: &KeyEvent) -> Option<char> {
    if key.modifiers.ctrl || key.modifiers.alt || key.modifiers.meta {
        return None;
    }
    if let Some(ref text) = key.text {
        let mut chars = text.chars();
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

impl View for TerminalPanelView {
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
        *self.last_layout_bounds.write() = Some(bounds);

        let layout = self.compute_layout(bounds, font);
        *self.last_body_height.write() = layout.body_rect.height();

        scene.push_finite_quad(Quad::new(bounds).with_background(theme.panel_bg));

        // Header toolbar
        scene.push_finite_quad(
            Quad::new(layout.header_rect)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        let active_tab_idx = self.manager.active_tab_index();
        for (idx, tab_rect, close_rect) in &layout.tab_rects {
            let is_active = active_tab_idx == Some(*idx);
            if is_active {
                scene.push_finite_quad(
                    Quad::new(*tab_rect)
                        .with_background(theme.tab_active_bg)
                        .with_border_edges(
                            theme.activity_bar_active_border,
                            Edges::new(0.0, 0.0, 2.0, 0.0),
                        ),
                );
            }
            if let Some(tab) = self.manager.tabs().get(*idx) {
                let label = self.tab_label(*idx, tab);
                let text_color = if is_active {
                    theme.tab_active_fg
                } else {
                    theme.tab_inactive_fg
                };
                scene.push_finite_text(
                    GlyphRun::new(
                        label,
                        Point::new(tab_rect.min_x() + 6.0, tab_rect.min_y() + 5.0),
                        Rect::from_xywh(
                            tab_rect.min_x() + 6.0,
                            tab_rect.min_y() + 5.0,
                            (tab_rect.width() - 22.0).max(0.0),
                            font.line_height,
                        ),
                        text_color,
                        font.font_size * 0.85,
                    ),
                );
            }
            scene.push_finite_text(
                GlyphRun::new(
                    "x".to_string(),
                    Point::new(close_rect.min_x() + 3.0, close_rect.min_y() + 1.0),
                    *close_rect,
                    theme.sidebar_header_fg,
                    font.font_size * 0.8,
                ),
            );
        }

        for (btn_rect, label) in [
            (layout.new_btn_rect, "+"),
            (layout.split_btn_rect, "Split"),
            (layout.clear_btn_rect, "Clear"),
            (layout.kill_btn_rect, "Kill"),
        ] {
            scene.push_finite_quad(
                Quad::new(btn_rect)
                    .with_background(theme.tab_inactive_bg)
                    .with_border(theme.panel_border, 1.0)
                    .with_corner_radius(3.0),
            );
            scene.push_finite_text(
                GlyphRun::new(
                    label.to_string(),
                    Point::new(btn_rect.min_x() + 6.0, btn_rect.min_y() + 3.0),
                    btn_rect,
                    theme.tab_active_fg,
                    font.font_size * 0.8,
                ),
            );
        }

        // Body panes
        if self.manager.active_tab_terminals().is_empty() {
            if layout.body_rect.height() >= font.line_height {
                scene.push_clip(layout.body_rect);
                scene.push_finite_text(
                    GlyphRun::new(
                        "No active terminal session. Click '+' to create a new terminal.".to_string(),
                        Point::new(layout.body_rect.min_x() + 12.0, layout.body_rect.min_y() + 10.0),
                        Rect::from_xywh(
                            layout.body_rect.min_x() + 12.0,
                            layout.body_rect.min_y() + 10.0,
                            (layout.body_rect.width() - 24.0).max(0.0),
                            font.line_height,
                        ),
                        theme.sidebar_header_fg,
                        font.font_size * 0.9,
                    ),
                );
                scene.pop_clip();
            }
        } else {
            let active_id = self.manager.active_terminal().map(|t| t.id.as_str());
            let multi_pane = layout.pane_rects.len() > 1;
            let line_h = font.line_height.max(1.0);
            let pad_x = 10.0;
            let pad_y = 4.0;

            for (pane_idx, (term_id, pane_rect)) in layout.pane_rects.iter().enumerate() {
                if pane_idx > 0 {
                    scene.push_finite_quad(
                        Quad::new(Rect::from_xywh(
                            pane_rect.min_x(),
                            pane_rect.min_y(),
                            1.0,
                            pane_rect.height(),
                        ))
                        .with_background(theme.panel_border),
                    );
                }
                let is_active_pane = active_id == Some(term_id.as_str());
                if multi_pane && is_active_pane {
                    scene.push_finite_quad(
                        Quad::new(Rect::from_xywh(
                            pane_rect.min_x(),
                            pane_rect.min_y(),
                            pane_rect.width(),
                            2.0,
                        ))
                        .with_background(theme.activity_bar_active_border),
                    );
                }

                let Some(term) = self.manager.terminal(term_id) else {
                    continue;
                };

                scene.push_clip(*pane_rect);

                let use_vt_grid = if term.has_pty() {
                    true
                } else if term.input_buffer.is_empty() {
                    let snap = term.grid_snapshot();
                    snapshot_has_vt_formatting(&snap)
                } else {
                    false
                };

                if use_vt_grid {
                    let snapshot = term.grid_snapshot();
                    render_grid_snapshot(
                        &snapshot,
                        *pane_rect,
                        is_active_pane && self.is_focused && !term.exited,
                        theme,
                        font,
                        scene,
                    );
                } else {
                    let total_lines = term.lines.len() + if term.exited { 0 } else { 1 };
                    let visible_rows = ((pane_rect.height() - pad_y * 2.0).max(line_h) / line_h)
                        .ceil()
                        .max(1.0) as usize;
                    let max_scroll = self.compute_max_scroll_lines(term_id, pane_rect.height(), line_h);
                    let auto = self.auto_scroll.get(term_id).copied().unwrap_or(true);
                    let scroll_top = if auto {
                        max_scroll
                    } else {
                        self.scroll_offsets
                            .get(term_id)
                            .copied()
                            .unwrap_or(max_scroll)
                            .clamp(0.0, max_scroll)
                    };
                    let start_row = (scroll_top.floor().max(0.0) as usize).min(total_lines);
                    let end_row = (start_row + visible_rows + 1).min(total_lines);

                    for row in start_row..end_row {
                        let visual_idx = row - start_row;
                        let y = pane_rect.min_y() + pad_y + visual_idx as f32 * line_h;
                        if y > pane_rect.max_y() {
                            break;
                        }
                        if row < term.lines.len() {
                            let line_text = &term.lines[row];
                            let color = if line_text.contains("timed out") || line_text.starts_with("cd: ") {
                                theme.editor_error_foreground
                            } else {
                                theme.editor_foreground
                            };
                            scene.push_finite_text(
                                GlyphRun::new(
                                    line_text.clone(),
                                    Point::new(pane_rect.min_x() + pad_x, y),
                                    Rect::from_xywh(
                                        pane_rect.min_x() + pad_x,
                                        y,
                                        (pane_rect.width() - pad_x * 2.0).max(0.0),
                                        line_h,
                                    ),
                                    color,
                                    font.font_size * 0.9,
                                ),
                            );
                        } else if !term.exited {
                            let prompt = term.prompt_prefix();
                            let prompt_w = font.text_width(&prompt);
                            scene.push_finite_text(
                                GlyphRun::new(
                                    prompt.clone(),
                                    Point::new(pane_rect.min_x() + pad_x, y),
                                    Rect::from_xywh(
                                        pane_rect.min_x() + pad_x,
                                        y,
                                        prompt_w,
                                        line_h,
                                    ),
                                    theme.editor_info_foreground,
                                    font.font_size * 0.9,
                                ),
                            );
                            if !term.input_buffer.is_empty() {
                                let input_w = font.text_width(&term.input_buffer);
                                scene.push_finite_text(
                                    GlyphRun::new(
                                        term.input_buffer.clone(),
                                        Point::new(pane_rect.min_x() + pad_x + prompt_w, y),
                                        Rect::from_xywh(
                                            pane_rect.min_x() + pad_x + prompt_w,
                                            y,
                                            input_w,
                                            line_h,
                                        ),
                                        theme.editor_foreground,
                                        font.font_size * 0.9,
                                    ),
                                );
                            }
                            if is_active_pane && self.is_focused {
                                let cursor_x = pane_rect.min_x()
                                    + pad_x
                                    + prompt_w
                                    + font.text_width(term.input_before_cursor());
                                scene.push_finite_quad(
                                    Quad::new(Rect::from_xywh(cursor_x, y, 2.0, line_h))
                                        .with_background(theme.editor_cursor),
                                );
                            }
                        }
                    }
                }

                scene.pop_clip();
            }
        }

        *self.click_targets.write() = self.build_click_targets(&layout);
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if !bounds.is_finite_positive() {
            return EventResult::Ignored;
        }

        match event {
            InputEvent::MouseDown(mouse) => {
                if !mouse.position.x.is_finite()
                    || !mouse.position.y.is_finite()
                    || !bounds.contains(mouse.position)
                {
                    return EventResult::Ignored;
                }

                let font = self.last_font.read().clone();
                let layout = self.compute_layout(bounds, &font);
                let targets = self.build_click_targets(&layout);
                *self.click_targets.write() = targets.clone();
                *self.last_layout_bounds.write() = Some(bounds);

                for (rect, target) in targets.into_iter().rev() {
                    if rect.contains(mouse.position) {
                        match target {
                            TerminalClickTarget::NewButton => {
                                self.create_terminal(Some("sh"), None, None);
                            }
                            TerminalClickTarget::SplitButton => {
                                self.split_active_terminal();
                            }
                            TerminalClickTarget::ClearButton => {
                                if let Some(term) = self.manager.active_terminal_mut() {
                                    let id = term.id.clone();
                                    term.clear();
                                    self.scroll_offsets.insert(id.clone(), 0.0);
                                    self.auto_scroll.insert(id.clone(), true);
                                    self.last_action = Some(TerminalAction::ClearTerminal { id });
                                }
                            }
                            TerminalClickTarget::KillButton => {
                                self.kill_active_terminal();
                            }
                            TerminalClickTarget::CloseTab(idx) => {
                                let (pane_ids, closed_id) = self
                                    .manager
                                    .tabs()
                                    .get(idx)
                                    .map(|t| {
                                        (
                                            t.pane_ids.clone(),
                                            t.active_pane_id().map(|s| s.to_string()),
                                        )
                                    })
                                    .unwrap_or_default();
                                if self.manager.close_tab(idx) {
                                    for pid in &pane_ids {
                                        self.scroll_offsets.remove(pid);
                                        self.auto_scroll.remove(pid);
                                    }
                                    if let Some(id) = closed_id {
                                        self.last_action = Some(TerminalAction::CloseTerminal { id });
                                    }
                                }
                            }
                            TerminalClickTarget::SelectTab(idx) => {
                                if self.manager.set_active_tab(idx) {
                                    self.is_focused = true;
                                    self.last_action =
                                        Some(TerminalAction::SelectTab { tab_index: idx });
                                }
                            }
                            TerminalClickTarget::FocusPane(id) => {
                                if self.manager.set_active_terminal(&id) {
                                    self.is_focused = true;
                                    self.last_action = Some(TerminalAction::FocusPane { id });
                                }
                            }
                        }
                        let updated_layout = self.compute_layout(bounds, &font);
                        *self.click_targets.write() = self.build_click_targets(&updated_layout);
                        return EventResult::Consumed;
                    }
                }

                self.is_focused = true;
                EventResult::Consumed
            }
            InputEvent::ScrollWheel(scroll) => {
                if !scroll.position.x.is_finite()
                    || !scroll.position.y.is_finite()
                    || !bounds.contains(scroll.position)
                    || !scroll.delta_y.is_finite()
                {
                    return EventResult::Ignored;
                }

                let font = self.last_font.read().clone();
                let line_h = font.line_height.max(1.0);
                let layout = self.compute_layout(bounds, &font);
                let target_id = layout
                    .pane_rects
                    .iter()
                    .find(|(_, r)| r.contains(scroll.position))
                    .map(|(id, _)| id.clone())
                    .or_else(|| self.manager.active_terminal().map(|t| t.id.clone()));

                let Some(id) = target_id else {
                    return EventResult::Ignored;
                };

                if self.manager.terminal(&id).is_some_and(|t| t.has_pty()) {
                    let lines = (scroll.delta_y.abs() / line_h).ceil().max(1.0) as i32;
                    let delta = if scroll.delta_y > 0.0 {
                        lines
                    } else if scroll.delta_y < 0.0 {
                        -lines
                    } else {
                        0
                    };
                    if delta != 0 {
                        if let Some(term) = self.manager.terminal_mut(&id) {
                            term.scroll_display(delta);
                            let at_bottom = term.grid().display_offset() == 0;
                            self.auto_scroll.insert(id, at_bottom);
                        }
                    }
                    return EventResult::Consumed;
                }

                let max_scroll =
                    self.compute_max_scroll_lines(&id, layout.body_rect.height(), line_h);
                let auto = self.auto_scroll.get(&id).copied().unwrap_or(true);
                let base_scroll = if auto {
                    max_scroll
                } else {
                    self.scroll_offsets
                        .get(&id)
                        .copied()
                        .unwrap_or(max_scroll)
                        .clamp(0.0, max_scroll)
                };
                let line_delta = -scroll.delta_y / line_h;
                let next_scroll = (base_scroll + line_delta).clamp(0.0, max_scroll);
                self.scroll_offsets.insert(id.clone(), next_scroll);

                if scroll.delta_y > 0.0 {
                    self.auto_scroll.insert(id, false);
                } else if scroll.delta_y < 0.0 && (max_scroll == 0.0 || next_scroll >= max_scroll - 0.1) {
                    self.auto_scroll.insert(id, true);
                }

                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => {
                if !self.is_focused {
                    return EventResult::Ignored;
                }
                let Some(term) = self.manager.active_terminal_mut() else {
                    return EventResult::Ignored;
                };
                let term_id = term.id.clone();

                if term.has_pty() {
                    if term.exited {
                        return EventResult::Ignored;
                    }
                    if key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta {
                        match key.key {
                            KeyCode::Char('c') | KeyCode::Char('C') => {
                                let _ = term.send_ctrl_c();
                                self.auto_scroll.insert(term_id, true);
                                return EventResult::Consumed;
                            }
                            KeyCode::Char('l') | KeyCode::Char('L') => {
                                let _ = term.send_pty_input(&[0x0c]);
                                term.clear();
                                self.scroll_offsets.insert(term_id.clone(), 0.0);
                                self.auto_scroll.insert(term_id.clone(), true);
                                self.last_action = Some(TerminalAction::ClearTerminal { id: term_id });
                                return EventResult::Consumed;
                            }
                            KeyCode::Char(c) if c.is_ascii_alphabetic() => {
                                let ctrl_byte = (c.to_ascii_lowercase() as u8) - b'a' + 1;
                                let _ = term.send_pty_input(&[ctrl_byte]);
                                self.auto_scroll.insert(term_id, true);
                                return EventResult::Consumed;
                            }
                            _ => return EventResult::Ignored,
                        }
                    }
                    if key.modifiers.alt || key.modifiers.meta {
                        return EventResult::Ignored;
                    }
                    let seq: Option<&[u8]> = match key.key {
                        KeyCode::Enter => Some(b"\r"),
                        KeyCode::Backspace => Some(b"\x7f"),
                        KeyCode::Delete => Some(b"\x1b[3~"),
                        KeyCode::Tab => Some(b"\t"),
                        KeyCode::Escape => Some(b"\x1b"),
                        KeyCode::ArrowUp => Some(b"\x1b[A"),
                        KeyCode::ArrowDown => Some(b"\x1b[B"),
                        KeyCode::ArrowRight => Some(b"\x1b[C"),
                        KeyCode::ArrowLeft => Some(b"\x1b[D"),
                        KeyCode::Home => Some(b"\x1b[H"),
                        KeyCode::End => Some(b"\x1b[F"),
                        KeyCode::PageUp => Some(b"\x1b[5~"),
                        KeyCode::PageDown => Some(b"\x1b[6~"),
                        _ => None,
                    };
                    if let Some(bytes) = seq {
                        let _ = term.send_pty_input(bytes);
                        term.grid_mut().scroll_to_bottom();
                        let _ = term.poll_pty();
                        self.auto_scroll.insert(term_id, true);
                        return EventResult::Consumed;
                    }
                    if let Some(ref text) = key.text {
                        let filtered: String = text.chars().filter(|&c| c != '\0').collect();
                        if !filtered.is_empty() {
                            let _ = term.send_pty_str(&filtered);
                            term.grid_mut().scroll_to_bottom();
                            self.auto_scroll.insert(term_id, true);
                            return EventResult::Consumed;
                        }
                    }
                    if let Some(ch) = key_event_to_char(key) {
                        let mut buf = [0u8; 4];
                        let s = ch.encode_utf8(&mut buf);
                        let _ = term.send_pty_str(s);
                        term.grid_mut().scroll_to_bottom();
                        self.auto_scroll.insert(term_id, true);
                        return EventResult::Consumed;
                    }
                    return EventResult::Ignored;
                }

                if key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta {
                    match key.key {
                        KeyCode::Char('l') | KeyCode::Char('L') => {
                            if term.exited {
                                return EventResult::Ignored;
                            }
                            term.clear();
                            self.scroll_offsets.insert(term_id.clone(), 0.0);
                            self.auto_scroll.insert(term_id.clone(), true);
                            self.last_action = Some(TerminalAction::ClearTerminal { id: term_id });
                            return EventResult::Consumed;
                        }
                        KeyCode::Char('c') | KeyCode::Char('C') => {
                            if term.exited {
                                return EventResult::Ignored;
                            }
                            let line = format!("{}{}^C", term.prompt_prefix(), term.input_buffer);
                            term.lines.push(line);
                            term.trim_scrollback();
                            term.clear_input();
                            self.auto_scroll.insert(term_id, true);
                            return EventResult::Consumed;
                        }
                        KeyCode::Char('u') | KeyCode::Char('U') => {
                            term.clear_input();
                            self.auto_scroll.insert(term_id, true);
                            return EventResult::Consumed;
                        }
                        _ => return EventResult::Ignored,
                    }
                }

                if key.modifiers.alt || key.modifiers.meta {
                    return EventResult::Ignored;
                }

                match key.key {
                    KeyCode::Enter => {
                        let cmd = term.input_buffer.clone();
                        let res = term.submit_input();
                        self.auto_scroll.insert(term_id.clone(), true);
                        self.last_action = Some(TerminalAction::CommandExecuted {
                            terminal_id: term_id,
                            command: cmd,
                            exit_code: res.exit_code,
                        });
                        EventResult::Consumed
                    }
                    KeyCode::Backspace => {
                        term.backspace();
                        self.auto_scroll.insert(term_id, true);
                        EventResult::Consumed
                    }
                    KeyCode::Delete => {
                        term.delete();
                        self.auto_scroll.insert(term_id, true);
                        EventResult::Consumed
                    }
                    KeyCode::ArrowLeft => {
                        term.move_cursor_left();
                        EventResult::Consumed
                    }
                    KeyCode::ArrowRight => {
                        term.move_cursor_right();
                        EventResult::Consumed
                    }
                    KeyCode::Home => {
                        term.move_cursor_home();
                        EventResult::Consumed
                    }
                    KeyCode::End => {
                        term.move_cursor_end();
                        EventResult::Consumed
                    }
                    KeyCode::ArrowUp => {
                        term.history_up();
                        self.auto_scroll.insert(term_id, true);
                        EventResult::Consumed
                    }
                    KeyCode::ArrowDown => {
                        term.history_down();
                        self.auto_scroll.insert(term_id, true);
                        EventResult::Consumed
                    }
                    _ => {
                        if !key.modifiers.ctrl && !key.modifiers.alt && !key.modifiers.meta {
                            if let Some(ref text) = key.text {
                                let filtered: String = text
                                    .chars()
                                    .filter(|&c| c != '\0' && c != '\r' && c != '\n' && !c.is_control())
                                    .collect();
                                if !filtered.is_empty() {
                                    term.insert_str(&filtered);
                                    self.auto_scroll.insert(term_id, true);
                                    return EventResult::Consumed;
                                }
                            }
                        }
                        if let Some(ch) = key_event_to_char(key) {
                            term.insert_char(ch);
                            self.auto_scroll.insert(term_id, true);
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                }
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_platform::keybindings::Modifiers;
    use rustcode_platform::render::{MouseButton, MouseEvent, SceneNode, ScrollEvent, Size};

    #[test]
    fn test_terminal_panel_render_and_key_input() {
        let mut panel = TerminalPanelView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 600.0, 220.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for ch in "echo term_ok".chars() {
            let res = panel.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char(ch), Modifiers::none())),
                bounds,
            );
            assert_eq!(res, EventResult::Consumed);
        }
        assert_eq!(
            panel.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
                bounds,
            ),
            EventResult::Consumed
        );
        assert!(matches!(
            panel.last_action,
            Some(TerminalAction::CommandExecuted { .. })
        ));

        let mut scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("term_ok"));

        // History up, left, backspace, delete, Ctrl+L
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::ArrowUp, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            panel.manager.active_terminal().unwrap().input_buffer,
            "echo term_ok"
        );
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::ArrowLeft, Modifiers::none())),
            bounds,
        );
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Backspace, Modifiers::none())),
            bounds,
        );
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Delete, Modifiers::none())),
            bounds,
        );
        assert_eq!(
            panel.manager.active_terminal().unwrap().input_buffer,
            "echo term_"
        );

        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(
                KeyCode::Char('l'),
                Modifiers {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    meta: false,
                },
            )),
            bounds,
        );
        assert!(panel.manager.active_terminal().unwrap().lines.is_empty());
    }

    #[test]
    fn test_terminal_panel_tabs_split_and_mouse_hit_testing() {
        let mut panel = TerminalPanelView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 240.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let mut scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);
        let targets = panel.click_targets.read().clone();

        let new_rect = targets
            .iter()
            .find(|(_, t)| *t == TerminalClickTarget::NewButton)
            .unwrap()
            .0;
        let new_center = Point::new(
            new_rect.min_x() + new_rect.width() / 2.0,
            new_rect.min_y() + new_rect.height() / 2.0,
        );
        assert_eq!(
            panel.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(new_center, MouseButton::Left)),
                bounds,
            ),
            EventResult::Consumed
        );
        assert_eq!(panel.manager.tabs().len(), 2);

        scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);
        let targets = panel.click_targets.read().clone();
        let split_rect = targets
            .iter()
            .find(|(_, t)| *t == TerminalClickTarget::SplitButton)
            .unwrap()
            .0;
        let split_center = Point::new(
            split_rect.min_x() + split_rect.width() / 2.0,
            split_rect.min_y() + split_rect.height() / 2.0,
        );
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(split_center, MouseButton::Left)),
            bounds,
        );
        assert_eq!(panel.manager.active_tab_terminals().len(), 2);

        scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);
        let targets = panel.click_targets.read().clone();
        let pane_targets: Vec<(Rect, String)> = targets
            .iter()
            .filter_map(|(r, t)| match t {
                TerminalClickTarget::FocusPane(id) => Some((*r, id.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(pane_targets.len(), 2);

        // Click left pane and type
        let left_center = Point::new(
            pane_targets[0].0.min_x() + 10.0,
            pane_targets[0].0.min_y() + 10.0,
        );
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(left_center, MouseButton::Left)),
            bounds,
        );
        assert_eq!(
            panel.manager.active_terminal().unwrap().id,
            pane_targets[0].1
        );
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('a'), Modifiers::none())),
            bounds,
        );

        // Click right pane and type
        let right_center = Point::new(
            pane_targets[1].0.min_x() + 10.0,
            pane_targets[1].0.min_y() + 10.0,
        );
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(right_center, MouseButton::Left)),
            bounds,
        );
        assert_eq!(
            panel.manager.active_terminal().unwrap().id,
            pane_targets[1].1
        );
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('b'), Modifiers::none())),
            bounds,
        );
        assert_eq!(
            panel.manager.terminal(&pane_targets[0].1).unwrap().input_buffer,
            "a"
        );
        assert_eq!(
            panel.manager.terminal(&pane_targets[1].1).unwrap().input_buffer,
            "b"
        );

        // Kill active pane, then close tab 0
        let kill_rect = targets
            .iter()
            .find(|(_, t)| *t == TerminalClickTarget::KillButton)
            .unwrap()
            .0;
        let kill_center = Point::new(
            kill_rect.min_x() + kill_rect.width() / 2.0,
            kill_rect.min_y() + kill_rect.height() / 2.0,
        );
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(kill_center, MouseButton::Left)),
            bounds,
        );
        assert_eq!(panel.manager.active_tab_terminals().len(), 1);

        scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);
        let close_tab_rect = panel
            .click_targets
            .read()
            .iter()
            .find(|(_, t)| *t == TerminalClickTarget::CloseTab(0))
            .unwrap()
            .0;
        let close_center = Point::new(
            close_tab_rect.min_x() + close_tab_rect.width() / 2.0,
            close_tab_rect.min_y() + close_tab_rect.height() / 2.0,
        );
        panel.handle_event(
            &InputEvent::MouseDown(MouseEvent::new(close_center, MouseButton::Left)),
            bounds,
        );
        assert_eq!(panel.manager.tabs().len(), 1);
    }

    #[test]
    fn test_terminal_panel_scroll_wheel_and_auto_scroll() {
        let mut panel = TerminalPanelView::new();
        let bounds = Rect::from_xywh(0.0, 0.0, 600.0, 140.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let term_id = {
            let term = panel.manager.active_terminal_mut().unwrap();
            for i in 0..50 {
                term.write_output(&format!("line_{:02}", i));
            }
            term.id.clone()
        };

        let mut scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);
        assert!(scene.contains_text("line_49"));
        assert!(!scene.contains_text("line_00"));

        // Scroll up (delta_y > 0) pauses auto_scroll
        let res = panel.handle_event(
            &InputEvent::ScrollWheel(ScrollEvent::new(Point::new(100.0, 80.0), 0.0, 2000.0)),
            bounds,
        );
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(panel.auto_scroll.get(&term_id), Some(&false));

        let mut scrolled_scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scrolled_scene);
        assert!(scrolled_scene.contains_text("line_00"));

        // Press Enter snaps back to bottom
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        assert_eq!(panel.auto_scroll.get(&term_id), Some(&true));
    }

    #[test]
    fn test_terminal_panel_non_finite_bounds_and_accessibility() {
        let panel = TerminalPanelView::new();
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        for bad_bounds in [
            Rect::new(Point::new(f32::NAN, 0.0), Size::new(400.0, 200.0)),
            Rect::from_xywh(0.0, 0.0, f32::INFINITY, 200.0),
        ] {
            let mut scene = Scene::new();
            panel.render(bad_bounds, &theme, &font, &mut scene);
            assert!(scene.nodes().is_empty());
        }

        let max_font = FontMetrics {
            font_family: "mono".to_string(),
            font_size: f32::MAX,
            line_height: f32::MAX,
            char_width: f32::MAX,
            ascent: f32::MAX,
            descent: 0.0,
        };
        let valid_bounds = Rect::from_xywh(0.0, 0.0, 600.0, 200.0);
        let mut scene = Scene::new();
        panel.render(valid_bounds, &theme, &max_font, &mut scene);
        assert!(scene.nodes().is_empty());

        let mut ok_scene = Scene::new();
        panel.render(valid_bounds, &theme, &font, &mut ok_scene);
        let mut clip_depth = 0isize;
        for node in ok_scene.nodes() {
            match node {
                SceneNode::PushClip(_) => clip_depth += 1,
                SceneNode::PopClip => clip_depth -= 1,
                _ => {}
            }
            assert!(clip_depth >= 0);
        }
        assert_eq!(clip_depth, 0);

        let a11y = panel.accessibility_node(valid_bounds, true);
        assert_eq!(a11y.id, "workbench.panel.terminal");
        let focused = a11y.find_focused().unwrap();
        assert!(focused.id.starts_with("terminal.pane."));
        assert_eq!(focused.role, AccessibleRole::TextInput);
    }

    #[test]
    fn test_terminal_panel_resize_before_render_mousedown_and_close_tab_cleanup() {
        let mut panel = TerminalPanelView::new();
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        // Render at 800px width
        let mut scene = Scene::new();
        panel.render(Rect::from_xywh(0.0, 0.0, 800.0, 240.0), &theme, &font, &mut scene);

        // Immediately resize to 400px and click NewButton at the 400px layout position without re-rendering
        let narrow_bounds = Rect::from_xywh(0.0, 0.0, 400.0, 240.0);
        let narrow_layout = panel.compute_layout(narrow_bounds, &font);
        let new_center = Point::new(
            narrow_layout.new_btn_rect.min_x() + narrow_layout.new_btn_rect.width() / 2.0,
            narrow_layout.new_btn_rect.min_y() + narrow_layout.new_btn_rect.height() / 2.0,
        );
        assert_eq!(
            panel.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(new_center, MouseButton::Left)),
                narrow_bounds,
            ),
            EventResult::Consumed
        );
        assert_eq!(panel.manager.tabs().len(), 2);

        // Split tab 1 so it has 2 panes, track their scroll state, then close tab 1 via CloseTab click
        panel.split_active_terminal();
        let tab1_pane_ids = panel.manager.tabs()[1].pane_ids.clone();
        assert_eq!(tab1_pane_ids.len(), 2);
        for pid in &tab1_pane_ids {
            panel.scroll_offsets.insert(pid.clone(), 4.0);
            panel.auto_scroll.insert(pid.clone(), false);
        }

        let mut scene2 = Scene::new();
        panel.render(narrow_bounds, &theme, &font, &mut scene2);
        let close_tab1_rect = panel
            .click_targets
            .read()
            .iter()
            .find(|(_, t)| *t == TerminalClickTarget::CloseTab(1))
            .unwrap()
            .0;
        let close_center = Point::new(
            close_tab1_rect.min_x() + close_tab1_rect.width() / 2.0,
            close_tab1_rect.min_y() + close_tab1_rect.height() / 2.0,
        );
        assert_eq!(
            panel.handle_event(
                &InputEvent::MouseDown(MouseEvent::new(close_center, MouseButton::Left)),
                narrow_bounds,
            ),
            EventResult::Consumed
        );
        assert_eq!(panel.manager.tabs().len(), 1);
        for pid in &tab1_pane_ids {
            assert!(!panel.scroll_offsets.contains_key(pid));
            assert!(!panel.auto_scroll.contains_key(pid));
        }
    }

    #[test]
    fn test_terminal_panel_tiny_height_ime_multichar_post_exit_and_a11y_overflow() {
        let mut panel = TerminalPanelView::new();
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        // Active terminal at tiny height (<= 26.0) must NOT emit "No active terminal session"
        let mut tiny_scene = Scene::new();
        panel.render(Rect::from_xywh(0.0, 0.0, 600.0, 26.0), &theme, &font, &mut tiny_scene);
        assert!(!tiny_scene.contains_text("No active terminal session"));

        // Empty manager at tiny height (< header + line_height) must also stay clipped/suppressed
        let empty_panel = TerminalPanelView::empty();
        let mut empty_tiny_scene = Scene::new();
        empty_panel.render(
            Rect::from_xywh(0.0, 0.0, 600.0, 30.0),
            &theme,
            &font,
            &mut empty_tiny_scene,
        );
        assert!(!empty_tiny_scene.contains_text("No active terminal session"));

        // Multi-char IME KeyEvent.text inserts full UTF-8 string instead of 1-char fallback
        let bounds = Rect::from_xywh(0.0, 0.0, 600.0, 200.0);
        let ime_key =
            KeyEvent::new(KeyCode::Char('a'), Modifiers::none()).with_text("é🚀中".to_string());
        assert_eq!(
            panel.handle_event(&InputEvent::KeyDown(ime_key), bounds),
            EventResult::Consumed
        );
        assert_eq!(panel.manager.active_terminal().unwrap().input_buffer, "é🚀中");

        // Post-exit Ctrl+C and Ctrl+L do not mutate lines or clear "[Process exited]"
        panel.manager.active_terminal_mut().unwrap().clear_input();
        let exit_res = panel.manager.active_terminal_mut().unwrap().execute_command("exit 7");
        assert!(exit_res.exited);
        let lines_after_exit = panel.manager.active_terminal().unwrap().lines.clone();

        let ctrl_mod = Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        };
        assert_eq!(
            panel.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('c'), ctrl_mod)),
                bounds,
            ),
            EventResult::Ignored
        );
        assert_eq!(
            panel.handle_event(
                &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('l'), ctrl_mod)),
                bounds,
            ),
            EventResult::Ignored
        );
        assert_eq!(panel.manager.active_terminal().unwrap().lines, lines_after_exit);

        // Non-finite and f32::MAX overflow bounds must produce safe zero-bounds accessibility & empty scene
        for bad_bounds in [
            Rect::new(Point::new(f32::NAN, 0.0), Size::new(400.0, 200.0)),
            Rect::new(Point::new(f32::MAX, 0.0), Size::new(100.0, 200.0)),
        ] {
            let mut bad_scene = Scene::new();
            panel.render(bad_bounds, &theme, &font, &mut bad_scene);
            assert!(bad_scene.nodes().is_empty());

            let bad_a11y = panel.accessibility_node(bad_bounds, true);
            assert_eq!(bad_a11y.bounds, Rect::zero());
            assert!(bad_a11y.children.is_empty());
        }
    }

    #[test]
    fn test_terminal_panel_vt_grid_rendering_colors_styles_and_cursor() {
        use rustcode_terminal::HeadlessPtyBackend;

        let mut panel = TerminalPanelView::new();
        let backend = HeadlessPtyBackend::new(24, 80);
        backend.push_output(b"\x1b[1;4;31;44mALERT\x1b[0m \x1b[38;2;10;20;30mRGB\x1b[0m\r\n");

        {
            let term = panel.manager.active_terminal_mut().unwrap();
            term.attach_pty(Box::new(backend));
        }
        let updated = panel.poll_all_ptys();
        assert_eq!(updated.len(), 1);

        let bounds = Rect::from_xywh(0.0, 0.0, 700.0, 260.0);
        let theme = WorkbenchThemeStyles::dark_default();
        let font = FontMetrics::default_monospace();

        let mut scene = Scene::new();
        panel.render(bounds, &theme, &font, &mut scene);

        let alert_run = scene
            .glyph_runs()
            .into_iter()
            .find(|g| g.text == "ALERT")
            .expect("ALERT glyph run should be rendered from VT grid");
        assert!(alert_run.bold);
        assert!(alert_run.underline);
        assert!(alert_run.color.r > 0.7 && alert_run.color.g < 0.3);

        let rgb_run = scene
            .glyph_runs()
            .into_iter()
            .find(|g| g.text == "RGB")
            .expect("RGB glyph run should be rendered from VT grid");
        assert!((rgb_run.color.r - 10.0 / 255.0).abs() < 0.02);
        assert!((rgb_run.color.g - 20.0 / 255.0).abs() < 0.02);
        assert!((rgb_run.color.b - 30.0 / 255.0).abs() < 0.02);

        // Background quad for SGR 44 (blue) should be emitted
        assert!(scene.quads().iter().any(|q| {
            q.background
                .is_some_and(|bg| bg.b > 0.7 && bg.r < 0.25 && bg.g > 0.35 && bg.g < 0.55)
        }));

        // Cursor quad on row 1, col 0 should be emitted
        assert!(scene
            .quads()
            .iter()
            .any(|q| q.background == Some(theme.editor_cursor)));
    }

    #[test]
    fn test_terminal_panel_pty_key_forwarding_ctrl_c_resize_and_actions() {
        use rustcode_terminal::{HeadlessPtyBackend, TerminalSignal};

        let mut panel = TerminalPanelView::new();
        let backend = HeadlessPtyBackend::new(24, 80);
        let backend_handle = backend.clone();
        {
            let term = panel.manager.active_terminal_mut().unwrap();
            term.attach_pty(Box::new(backend));
        }

        let bounds = Rect::from_xywh(0.0, 0.0, 660.0, 240.0);
        let font = FontMetrics::default_monospace();
        let mut scene = Scene::new();
        panel.render(bounds, &WorkbenchThemeStyles::dark_default(), &font, &mut scene);

        // Resize computes rows/cols from FontMetrics & pane Rect and propagates to PTY backend
        let resized = panel.resize_to_bounds(bounds);
        assert_eq!(resized, 1);
        let (expected_rows, expected_cols) =
            TerminalPanelView::compute_grid_size(Rect::from_xywh(0.0, 26.0, 660.0, 214.0), &font);
        assert_eq!(backend_handle.size(), (expected_rows, expected_cols));

        // Forward typed chars, ArrowUp, Enter, and Ctrl+C to PTY backend
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('h'), Modifiers::none())),
            bounds,
        );
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Char('i'), Modifiers::none())),
            bounds,
        );
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::ArrowUp, Modifiers::none())),
            bounds,
        );
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(KeyCode::Enter, Modifiers::none())),
            bounds,
        );
        panel.handle_event(
            &InputEvent::KeyDown(KeyEvent::new(
                KeyCode::Char('c'),
                Modifiers {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    meta: false,
                },
            )),
            bounds,
        );

        assert_eq!(backend_handle.written_input(), b"hi\x1b[A\r\x03");
        assert_eq!(backend_handle.signals(), vec![TerminalSignal::Interrupt]);

        // send_text_to_terminal routes to send_pty_str when PTY is attached
        let first_id = panel.manager.active_terminal().unwrap().id.clone();
        panel.send_text_to_terminal(&first_id, "pwd", true).unwrap();
        assert!(backend_handle.written_input().ends_with(b"pwd\r"));

        // Action helpers: new_terminal, split_terminal, switch_tab, clear_active, close_terminal
        let second_id = panel.new_terminal();
        assert_eq!(panel.manager.tabs().len(), 2);
        assert!(panel.switch_tab(0));
        assert_eq!(panel.manager.active_terminal().unwrap().id, first_id);
        assert!(panel.clear_active());
        let split_id = panel.split_terminal();
        assert_eq!(panel.manager.active_tab_terminals().len(), 2);
        assert_eq!(panel.close_terminal(), Some(split_id));
        assert!(panel.switch_tab(1));
        assert_eq!(panel.close_terminal(), Some(second_id));
    }
}
