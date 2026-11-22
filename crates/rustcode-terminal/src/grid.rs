//! VT100/xterm grid emulation powered by `alacritty_terminal`.

use alacritty_terminal::event::{Event as AlacrittyEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{
    point_to_viewport, viewport_to_point, Config as TermConfig, Term, TermMode,
};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCell {
    pub ch: char,
    pub fg: [u8; 4],
    pub bg: [u8; 4],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Default for TerminalCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: DEFAULT_FG_RGBA,
            bg: DEFAULT_BG_RGBA,
            bold: false,
            italic: false,
            underline: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalGridSnapshot {
    pub rows: u16,
    pub cols: u16,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub cursor_visible: bool,
    pub alt_screen: bool,
    pub display_offset: usize,
    pub history_size: usize,
    pub cells: Vec<Vec<TerminalCell>>,
}

impl TerminalGridSnapshot {
    pub fn cell(&self, row: usize, col: usize) -> Option<&TerminalCell> {
        self.cells.get(row).and_then(|r| r.get(col))
    }

    pub fn row_text(&self, row: usize) -> Option<String> {
        self.cells.get(row).map(|cells| {
            let s: String = cells.iter().map(|c| c.ch).collect();
            s.trim_end_matches(' ').to_string()
        })
    }

    pub fn visible_lines(&self) -> Vec<String> {
        self.cells
            .iter()
            .map(|row| {
                let s: String = row.iter().map(|c| c.ch).collect();
                s.trim_end_matches(' ').to_string()
            })
            .collect()
    }
}

const DEFAULT_FG_RGBA: [u8; 4] = [212, 212, 212, 255];
const DEFAULT_BG_RGBA: [u8; 4] = [30, 30, 30, 255];

fn named_color_rgb(named: NamedColor) -> [u8; 3] {
    match named {
        NamedColor::Black => [0, 0, 0],
        NamedColor::Red => [205, 49, 49],
        NamedColor::Green => [13, 188, 121],
        NamedColor::Yellow => [229, 229, 16],
        NamedColor::Blue => [36, 114, 200],
        NamedColor::Magenta => [188, 63, 188],
        NamedColor::Cyan => [17, 168, 205],
        NamedColor::White => [229, 229, 229],
        NamedColor::BrightBlack => [102, 102, 102],
        NamedColor::BrightRed => [241, 76, 76],
        NamedColor::BrightGreen => [35, 209, 139],
        NamedColor::BrightYellow => [245, 245, 67],
        NamedColor::BrightBlue => [59, 142, 234],
        NamedColor::BrightMagenta => [214, 112, 214],
        NamedColor::BrightCyan => [41, 184, 219],
        NamedColor::BrightWhite => [255, 255, 255],
        NamedColor::Foreground | NamedColor::Cursor => [212, 212, 212],
        NamedColor::Background => [30, 30, 30],
        NamedColor::BrightForeground => [255, 255, 255],
        NamedColor::DimForeground => [141, 141, 141],
        NamedColor::DimBlack => [0, 0, 0],
        NamedColor::DimRed => [136, 32, 32],
        NamedColor::DimGreen => [8, 125, 80],
        NamedColor::DimYellow => [152, 152, 10],
        NamedColor::DimBlue => [24, 76, 133],
        NamedColor::DimMagenta => [125, 42, 125],
        NamedColor::DimCyan => [11, 112, 136],
        NamedColor::DimWhite => [152, 152, 152],
    }
}

fn indexed_color_rgb(idx: u8, colors: &Colors) -> [u8; 3] {
    if let Some(Rgb { r, g, b }) = colors[idx as usize] {
        return [r, g, b];
    }
    match idx {
        0 => named_color_rgb(NamedColor::Black),
        1 => named_color_rgb(NamedColor::Red),
        2 => named_color_rgb(NamedColor::Green),
        3 => named_color_rgb(NamedColor::Yellow),
        4 => named_color_rgb(NamedColor::Blue),
        5 => named_color_rgb(NamedColor::Magenta),
        6 => named_color_rgb(NamedColor::Cyan),
        7 => named_color_rgb(NamedColor::White),
        8 => named_color_rgb(NamedColor::BrightBlack),
        9 => named_color_rgb(NamedColor::BrightRed),
        10 => named_color_rgb(NamedColor::BrightGreen),
        11 => named_color_rgb(NamedColor::BrightYellow),
        12 => named_color_rgb(NamedColor::BrightBlue),
        13 => named_color_rgb(NamedColor::BrightMagenta),
        14 => named_color_rgb(NamedColor::BrightCyan),
        15 => named_color_rgb(NamedColor::BrightWhite),
        16..=231 => {
            let v = idx - 16;
            let r_idx = v / 36;
            let g_idx = (v % 36) / 6;
            let b_idx = v % 6;
            let to_level = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
            [to_level(r_idx), to_level(g_idx), to_level(b_idx)]
        }
        232..=255 => {
            let gray = 8 + (idx - 232) * 10;
            [gray, gray, gray]
        }
    }
}

pub fn resolve_alacritty_color(color: Color, colors: &Colors) -> [u8; 4] {
    let [r, g, b] = match color {
        Color::Spec(Rgb { r, g, b }) => [r, g, b],
        Color::Indexed(idx) => indexed_color_rgb(idx, colors),
        Color::Named(named) => {
            if let Some(Rgb { r, g, b }) = colors[named] {
                [r, g, b]
            } else {
                named_color_rgb(named)
            }
        }
    };
    [r, g, b, 255]
}

#[derive(Clone, Default)]
struct TermEventProxy {
    pty_writes: Arc<Mutex<Vec<String>>>,
    title: Arc<Mutex<Option<String>>>,
}

impl EventListener for TermEventProxy {
    fn send_event(&self, event: AlacrittyEvent) {
        match event {
            AlacrittyEvent::PtyWrite(text) => {
                self.pty_writes.lock().push(text);
            }
            AlacrittyEvent::Title(title) => {
                *self.title.lock() = Some(title);
            }
            AlacrittyEvent::ResetTitle => {
                *self.title.lock() = None;
            }
            _ => {}
        }
    }
}

struct GridDimensions {
    rows: usize,
    cols: usize,
}

impl Dimensions for GridDimensions {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

/// VT100/xterm terminal emulator grid backed by `alacritty_terminal::Term` and `vte::ansi::Processor`.
pub struct TerminalCellGrid {
    term: Term<TermEventProxy>,
    processor: Processor,
    proxy: TermEventProxy,
    rows: u16,
    cols: u16,
    max_scrollback: usize,
}

impl std::fmt::Debug for TerminalCellGrid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalCellGrid")
            .field("rows", &self.rows)
            .field("cols", &self.cols)
            .field("max_scrollback", &self.max_scrollback)
            .field("history_size", &self.history_size())
            .field("alt_screen", &self.is_alt_screen())
            .finish()
    }
}

impl Default for TerminalCellGrid {
    fn default() -> Self {
        Self::new(
            crate::DEFAULT_TERMINAL_ROWS,
            crate::DEFAULT_TERMINAL_COLS,
            crate::DEFAULT_MAX_SCROLLBACK,
        )
    }
}

impl TerminalCellGrid {
    pub fn new(rows: u16, cols: u16, max_scrollback: usize) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let proxy = TermEventProxy::default();
        let config = TermConfig {
            scrolling_history: max_scrollback,
            ..TermConfig::default()
        };
        let dims = GridDimensions {
            rows: rows as usize,
            cols: cols as usize,
        };
        let term = Term::new(config, &dims, proxy.clone());
        Self {
            term,
            processor: Processor::new(),
            proxy,
            rows,
            cols,
            max_scrollback,
        }
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn max_scrollback(&self) -> usize {
        self.max_scrollback
    }

    pub fn set_max_scrollback(&mut self, max_scrollback: usize) {
        self.max_scrollback = max_scrollback;
        let config = TermConfig {
            scrolling_history: max_scrollback,
            ..TermConfig::default()
        };
        self.term.set_options(config);
    }

    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.processor.advance(&mut self.term, bytes);
    }

    pub fn drain_pty_writes(&mut self) -> Vec<String> {
        let mut guard = self.proxy.pty_writes.lock();
        std::mem::take(&mut *guard)
    }

    pub fn title(&self) -> Option<String> {
        self.proxy.title.lock().clone()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        self.rows = rows;
        self.cols = cols;
        self.term.resize(GridDimensions {
            rows: rows as usize,
            cols: cols as usize,
        });
    }

    pub fn clear(&mut self) {
        self.processor.advance(&mut self.term, b"\x1b[2J\x1b[3J\x1b[H");
    }

    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn is_alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    pub fn scroll_display(&mut self, delta: i32) {
        self.term.scroll_display(Scroll::Delta(delta));
    }

    pub fn scroll_to_top(&mut self) {
        self.term.scroll_display(Scroll::Top);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    fn extract_line(&self, line: Line) -> String {
        let grid = self.term.grid();
        let row = &grid[line];
        let mut s = String::new();
        for col_idx in 0..self.cols as usize {
            let cell = &row[Column(col_idx)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            s.push(cell.c);
            if let Some(zw) = cell.zerowidth() {
                for &ch in zw {
                    s.push(ch);
                }
            }
        }
        s.trim_end_matches(' ').to_string()
    }

    pub fn visible_lines(&self) -> Vec<String> {
        let display_offset = self.term.grid().display_offset();
        (0..self.rows as usize)
            .map(|r| {
                let grid_pt = viewport_to_point(display_offset, Point::new(r, Column(0)));
                self.extract_line(grid_pt.line)
            })
            .collect()
    }

    pub fn all_lines(&self) -> Vec<String> {
        let history = self.history_size() as i32;
        let screen_lines = self.rows as i32;
        ((-history)..screen_lines)
            .map(|line_idx| self.extract_line(Line(line_idx)))
            .collect()
    }

    pub fn all_non_empty_tail_trimmed_lines(&self) -> Vec<String> {
        let mut lines = self.all_lines();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines
    }

    pub fn snapshot(&self) -> TerminalGridSnapshot {
        let renderable = self.term.renderable_content();
        let display_offset = renderable.display_offset;
        let colors = renderable.colors;
        let cursor = renderable.cursor;
        let grid = self.term.grid();
        let history_size = grid.history_size();
        let alt_screen = renderable.mode.contains(TermMode::ALT_SCREEN);

        let mut cells = Vec::with_capacity(self.rows as usize);
        for r in 0..self.rows as usize {
            let mut row_cells = Vec::with_capacity(self.cols as usize);
            let grid_line = viewport_to_point(display_offset, Point::new(r, Column(0))).line;
            let row = &grid[grid_line];
            for c in 0..self.cols as usize {
                let cell = &row[Column(c)];
                let mut fg = resolve_alacritty_color(cell.fg, colors);
                let mut bg = resolve_alacritty_color(cell.bg, colors);

                if cell.flags.contains(Flags::DIM) {
                    fg[0] = ((fg[0] as u16 * 2) / 3) as u8;
                    fg[1] = ((fg[1] as u16 * 2) / 3) as u8;
                    fg[2] = ((fg[2] as u16 * 2) / 3) as u8;
                }
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if cell.flags.contains(Flags::HIDDEN) {
                    fg = bg;
                }

                let ch = if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    ' '
                } else {
                    cell.c
                };

                row_cells.push(TerminalCell {
                    ch,
                    fg,
                    bg,
                    bold: cell.flags.contains(Flags::BOLD),
                    italic: cell.flags.contains(Flags::ITALIC),
                    underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                });
            }
            cells.push(row_cells);
        }

        let cursor_vp = point_to_viewport(display_offset, cursor.point);
        let (cursor_row, cursor_col, cursor_visible) = match cursor_vp {
            Some(vp) if vp.line < self.rows as usize && vp.column.0 < self.cols as usize => (
                vp.line as u16,
                vp.column.0 as u16,
                cursor.shape != CursorShape::Hidden,
            ),
            _ => (0, 0, false),
        };

        TerminalGridSnapshot {
            rows: self.rows,
            cols: self.cols,
            cursor_row,
            cursor_col,
            cursor_visible,
            alt_screen,
            display_offset,
            history_size,
            cells,
        }
    }
}
