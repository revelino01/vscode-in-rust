//! DAP (Debug Adapter Protocol) Workbench Integration: Floating toolbar, Debug Sidebar, and Debug Console.

use parking_lot::RwLock;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Rgba,
    Scene, Shadow, View, WorkbenchThemeStyles,
};
use rustcode_protocols::dap::{Scope, SourceBreakpoint, StackFrame, Thread, Variable};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugToolbarAction {
    Continue,
    Pause,
    StepOver,
    StepInto,
    StepOut,
    Restart,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugViewAction {
    Toolbar(DebugToolbarAction),
    SelectFrame {
        frame_id: u32,
        path: Option<String>,
        line: u32,
    },
    JumpToBreakpoint {
        path: String,
        line: u32,
    },
    ToggleBreakpoint {
        path: String,
        line: u32,
    },
    EvaluateExpression {
        expression: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugOutputEntry {
    pub category: String, // "stdout", "stderr", "console"
    pub text: String,
}

/// Active debug session state shared across workbench views.
#[derive(Debug, Clone, Default)]
pub struct DebugSessionState {
    pub is_active: bool,
    pub is_paused: bool,
    pub active_thread_id: Option<i64>,
    pub active_frame_id: Option<u32>,
    pub threads: Vec<Thread>,
    pub call_stack: Vec<StackFrame>,
    pub scopes: Vec<Scope>,
    pub variables: HashMap<u32, Vec<Variable>>, // keyed by variables_reference
    pub breakpoints: HashMap<String, Vec<SourceBreakpoint>>, // keyed by file path
    pub console_output: Vec<DebugOutputEntry>,
}

impl DebugSessionState {
    pub fn sample_session() -> Self {
        let mut session = Self {
            is_active: true,
            is_paused: true,
            active_thread_id: Some(1),
            active_frame_id: Some(10),
            threads: vec![Thread {
                id: 1,
                name: "main thread".to_string(),
            }],
            call_stack: vec![
                StackFrame {
                    id: 10,
                    name: "calculate_sum(a: 10, b: 20)".to_string(),
                    source: Some(rustcode_protocols::dap::Source {
                        name: Some("main.rs".to_string()),
                        path: Some("src/main.rs".to_string()),
                        source_reference: None,
                    }),
                    line: 14,
                    column: 5,
                },
                StackFrame {
                    id: 11,
                    name: "main()".to_string(),
                    source: Some(rustcode_protocols::dap::Source {
                        name: Some("main.rs".to_string()),
                        path: Some("src/main.rs".to_string()),
                        source_reference: None,
                    }),
                    line: 22,
                    column: 1,
                },
            ],
            scopes: vec![
                Scope {
                    name: "Locals".to_string(),
                    variables_reference: 100,
                    expensive: false,
                    named_variables: Some(2),
                    indexed_variables: None,
                },
                Scope {
                    name: "Globals".to_string(),
                    variables_reference: 200,
                    expensive: true,
                    named_variables: Some(1),
                    indexed_variables: None,
                },
            ],
            variables: HashMap::new(),
            breakpoints: HashMap::new(),
            console_output: vec![
                DebugOutputEntry {
                    category: "stdout".to_string(),
                    text: "[lldb] Launching target process...\n".to_string(),
                },
                DebugOutputEntry {
                    category: "console".to_string(),
                    text: "Process 1234 stopped at breakpoint 1: src/main.rs:14\n".to_string(),
                },
            ],
        };

        session.variables.insert(
            100,
            vec![
                Variable {
                    name: "a".to_string(),
                    value: "10".to_string(),
                    type_name: Some("i32".to_string()),
                    variables_reference: 0,
                    evaluate_name: Some("a".to_string()),
                },
                Variable {
                    name: "b".to_string(),
                    value: "20".to_string(),
                    type_name: Some("i32".to_string()),
                    variables_reference: 0,
                    evaluate_name: Some("b".to_string()),
                },
            ],
        );

        let bp_list = vec![SourceBreakpoint::new(14)];
        session.breakpoints.insert("src/main.rs".to_string(), bp_list);

        session
    }

    pub fn toggle_breakpoint(&mut self, path: &str, line: u32) -> bool {
        let list = self.breakpoints.entry(path.to_string()).or_default();
        if let Some(pos) = list.iter().position(|b| b.line == line) {
            list.remove(pos);
            false
        } else {
            list.push(SourceBreakpoint::new(line));
            true
        }
    }
}

/// Floating Debug Toolbar positioned at top-center of editor.
pub struct DebugToolbarView {
    pub is_visible: bool,
    pub is_paused: bool,
    pub last_action: Option<DebugToolbarAction>,
    button_bounds: RwLock<Vec<(Rect, DebugToolbarAction)>>,
}

impl Default for DebugToolbarView {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugToolbarView {
    pub fn new() -> Self {
        Self {
            is_visible: true,
            is_paused: true,
            last_action: None,
            button_bounds: RwLock::new(Vec::new()),
        }
    }

    pub fn toolbar_bounds(&self, container_bounds: Rect) -> Rect {
        let width = 230.0;
        let height = 32.0;
        let x = container_bounds.min_x() + (container_bounds.width() - width) / 2.0;
        let y = container_bounds.min_y() + 8.0;
        Rect::from_xywh(x, y, width, height)
    }
}

impl View for DebugToolbarView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        if !self.is_visible {
            return;
        }

        // Drop shadow for floating effect
        scene.push_shadow(Shadow {
            bounds,
            offset: Point::new(0.0, 3.0),
            blur: 8.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.35),
        });

        // Background pill
        scene.push_quad(
            Quad::new(bounds)
                .with_background(theme.command_palette_bg)
                .with_border_edges(theme.panel_border, Edges::all(1.0))
                .with_corner_radius(4.0),
        );

        let buttons = [
            (
                if self.is_paused { "▶" } else { "⏸" },
                if self.is_paused {
                    DebugToolbarAction::Continue
                } else {
                    DebugToolbarAction::Pause
                },
                theme.editor_info_foreground,
            ),
            ("⤵", DebugToolbarAction::StepOver, theme.editor_foreground),
            ("⤓", DebugToolbarAction::StepInto, theme.editor_foreground),
            ("⤒", DebugToolbarAction::StepOut, theme.editor_foreground),
            ("↻", DebugToolbarAction::Restart, theme.editor_warning_foreground),
            ("⏹", DebugToolbarAction::Stop, theme.editor_error_foreground),
        ];

        let slot_w = bounds.width() / buttons.len() as f32;
        let mut targets = Vec::with_capacity(buttons.len());

        for (idx, (label, action, color)) in buttons.iter().enumerate() {
            let slot_x = bounds.min_x() + idx as f32 * slot_w;
            let slot_rect = Rect::from_xywh(slot_x, bounds.min_y(), slot_w, bounds.height());
            targets.push((slot_rect, action.clone()));

            let text_w = font.text_width(label);
            let text_x = slot_x + (slot_w - text_w) / 2.0;
            let text_y = bounds.min_y() + (bounds.height() - font.line_height) / 2.0;

            scene.push_text(GlyphRun::new(
                label.to_string(),
                Point::new(text_x, text_y),
                Rect::from_xywh(text_x, text_y, text_w, font.line_height),
                *color,
                font.font_size * 0.9,
            ));
        }

        *self.button_bounds.write() = targets;
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if !self.is_visible {
            return EventResult::Ignored;
        }

        match event {
            InputEvent::MouseDown(mouse) => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }
                let targets = self.button_bounds.read().clone();
                for (rect, action) in targets {
                    if rect.contains(mouse.position) {
                        self.last_action = Some(action);
                        return EventResult::Consumed;
                    }
                }
                EventResult::Ignored
            }
            InputEvent::KeyDown(key) => {
                // Keybinding shortcuts for debug actions
                let action = match key.key {
                    KeyCode::F5 => {
                        if key.modifiers.shift {
                            Some(DebugToolbarAction::Stop)
                        } else if key.modifiers.ctrl {
                            Some(DebugToolbarAction::Restart)
                        } else {
                            Some(if self.is_paused {
                                DebugToolbarAction::Continue
                            } else {
                                DebugToolbarAction::Pause
                            })
                        }
                    }
                    KeyCode::F10 => Some(DebugToolbarAction::StepOver),
                    KeyCode::F11 => {
                        if key.modifiers.shift {
                            Some(DebugToolbarAction::StepOut)
                        } else {
                            Some(DebugToolbarAction::StepInto)
                        }
                    }
                    _ => None,
                };

                if let Some(act) = action {
                    self.last_action = Some(act);
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
        }
    }
}

/// Debug Sidebar view (Variables, Call Stack, Breakpoints).
pub struct DebugSidebarView {
    pub session: DebugSessionState,
    pub variables_collapsed: bool,
    pub callstack_collapsed: bool,
    pub breakpoints_collapsed: bool,
    pub last_action: Option<DebugViewAction>,
    pub click_targets: RwLock<Vec<(Rect, DebugViewAction)>>,
}

impl Default for DebugSidebarView {
    fn default() -> Self {
        Self::new(DebugSessionState::sample_session())
    }
}

impl DebugSidebarView {
    pub fn new(session: DebugSessionState) -> Self {
        Self {
            session,
            variables_collapsed: false,
            callstack_collapsed: false,
            breakpoints_collapsed: false,
            last_action: None,
            click_targets: RwLock::new(Vec::new()),
        }
    }
}

impl View for DebugSidebarView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        let mut targets = Vec::new();

        // 1. Sidebar Background
        scene.push_quad(
            Quad::new(bounds)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.sidebar_border, Edges::new(0.0, 1.0, 0.0, 0.0)),
        );

        // 2. Header
        let header_h = 36.0;
        let header_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), header_h);
        scene.push_quad(
            Quad::new(header_rect)
                .with_background(theme.panel_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        scene.push_text(GlyphRun::new(
            "RUN AND DEBUG".to_string(),
            Point::new(bounds.min_x() + 12.0, bounds.min_y() + 10.0),
            Rect::from_xywh(bounds.min_x() + 12.0, bounds.min_y() + 10.0, 150.0, font.line_height),
            theme.sidebar_header_fg,
            font.font_size * 0.85,
        ));

        let mut cur_y = bounds.min_y() + header_h + 4.0;
        let section_h = 24.0;

        // 3. Section: VARIABLES
        let var_hdr_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), section_h);
        scene.push_quad(Quad::new(var_hdr_rect).with_background(theme.panel_bg));
        let arrow = if self.variables_collapsed { "▶" } else { "▼" };
        scene.push_text(GlyphRun::new(
            format!("{} VARIABLES", arrow),
            Point::new(bounds.min_x() + 8.0, cur_y + 4.0),
            Rect::from_xywh(bounds.min_x() + 8.0, cur_y + 4.0, 150.0, font.line_height),
            theme.sidebar_header_fg,
            font.font_size * 0.8,
        ));
        cur_y += section_h;

        if !self.variables_collapsed {
            for scope in &self.session.scopes {
                // Scope row
                let _scope_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), 20.0);
                scene.push_text(GlyphRun::new(
                    scope.name.clone(),
                    Point::new(bounds.min_x() + 16.0, cur_y + 2.0),
                    Rect::from_xywh(bounds.min_x() + 16.0, cur_y + 2.0, 150.0, font.line_height),
                    theme.sidebar_header_fg,
                    font.font_size * 0.8,
                ));
                cur_y += 20.0;

                // Variables in scope
                if let Some(vars) = self.session.variables.get(&scope.variables_reference) {
                    for var in vars {
                        let _var_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), 20.0);
                        let var_display = format!("{}: {} ({})", var.name, var.value, var.type_name.as_deref().unwrap_or(""));
                        scene.push_text(GlyphRun::new(
                            var_display,
                            Point::new(bounds.min_x() + 28.0, cur_y + 2.0),
                            Rect::from_xywh(bounds.min_x() + 28.0, cur_y + 2.0, bounds.width() - 36.0, font.line_height),
                            theme.editor_foreground,
                            font.font_size * 0.8,
                        ));
                        cur_y += 20.0;
                    }
                }
            }
        }

        cur_y += 6.0;

        // 4. Section: CALL STACK
        let cs_hdr_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), section_h);
        scene.push_quad(Quad::new(cs_hdr_rect).with_background(theme.panel_bg));
        let arrow = if self.callstack_collapsed { "▶" } else { "▼" };
        scene.push_text(GlyphRun::new(
            format!("{} CALL STACK", arrow),
            Point::new(bounds.min_x() + 8.0, cur_y + 4.0),
            Rect::from_xywh(bounds.min_x() + 8.0, cur_y + 4.0, 150.0, font.line_height),
            theme.sidebar_header_fg,
            font.font_size * 0.8,
        ));
        cur_y += section_h;

        if !self.callstack_collapsed {
            for frame in &self.session.call_stack {
                let frame_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), 22.0);
                let is_active = self.session.active_frame_id == Some(frame.id);

                if is_active {
                    scene.push_quad(Quad::new(frame_rect).with_background(theme.command_palette_selected_bg));
                }

                let src_path = frame.source.as_ref().and_then(|s| s.path.clone());
                targets.push((
                    frame_rect,
                    DebugViewAction::SelectFrame {
                        frame_id: frame.id,
                        path: src_path,
                        line: frame.line,
                    },
                ));

                let frame_text = format!("{} {}:{}", frame.name, frame.source.as_ref().and_then(|s| s.name.clone()).unwrap_or_default(), frame.line);
                scene.push_text(GlyphRun::new(
                    frame_text,
                    Point::new(bounds.min_x() + 16.0, cur_y + 3.0),
                    Rect::from_xywh(bounds.min_x() + 16.0, cur_y + 3.0, bounds.width() - 24.0, font.line_height),
                    theme.editor_foreground,
                    font.font_size * 0.8,
                ));
                cur_y += 22.0;
            }
        }

        cur_y += 6.0;

        // 5. Section: BREAKPOINTS
        let bp_hdr_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), section_h);
        scene.push_quad(Quad::new(bp_hdr_rect).with_background(theme.panel_bg));
        let arrow = if self.breakpoints_collapsed { "▶" } else { "▼" };
        scene.push_text(GlyphRun::new(
            format!("{} BREAKPOINTS", arrow),
            Point::new(bounds.min_x() + 8.0, cur_y + 4.0),
            Rect::from_xywh(bounds.min_x() + 8.0, cur_y + 4.0, 150.0, font.line_height),
            theme.sidebar_header_fg,
            font.font_size * 0.8,
        ));
        cur_y += section_h;

        if !self.breakpoints_collapsed {
            for (path, bps) in &self.session.breakpoints {
                for bp in bps {
                    let bp_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), 20.0);
                    targets.push((
                        bp_rect,
                        DebugViewAction::JumpToBreakpoint {
                            path: path.clone(),
                            line: bp.line,
                        },
                    ));

                    // Red dot
                    scene.push_quad(
                        Quad::new(Rect::from_xywh(bounds.min_x() + 16.0, cur_y + 6.0, 8.0, 8.0))
                            .with_background(Rgba::new(0.9, 0.15, 0.15, 1.0))
                            .with_corner_radius(4.0),
                    );

                    let label = format!("{}:{}", PathBuf::from(path).file_name().map_or(path.as_str(), |n| n.to_str().unwrap_or("")), bp.line);
                    scene.push_text(GlyphRun::new(
                        label,
                        Point::new(bounds.min_x() + 30.0, cur_y + 2.0),
                        Rect::from_xywh(bounds.min_x() + 30.0, cur_y + 2.0, bounds.width() - 40.0, font.line_height),
                        theme.editor_foreground,
                        font.font_size * 0.8,
                    ));
                    cur_y += 20.0;
                }
            }
        }

        *self.click_targets.write() = targets;
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        if let InputEvent::MouseDown(mouse) = event {
            if !bounds.contains(mouse.position) {
                return EventResult::Ignored;
            }

            let targets = self.click_targets.read().clone();
            for (rect, action) in targets {
                if rect.contains(mouse.position) {
                    self.last_action = Some(action);
                    return EventResult::Consumed;
                }
            }
        }
        EventResult::Ignored
    }
}

/// Debug Console view rendered in the bottom panel.
pub struct DebugConsoleView {
    pub output_lines: Vec<DebugOutputEntry>,
    pub input_text: String,
    pub input_focused: bool,
    pub scroll_top: f32,
    pub last_action: Option<DebugViewAction>,
}

impl Default for DebugConsoleView {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugConsoleView {
    pub fn new() -> Self {
        Self {
            output_lines: Vec::new(),
            input_text: String::new(),
            input_focused: false,
            scroll_top: 0.0,
            last_action: None,
        }
    }

    pub fn append_output(&mut self, category: &str, text: &str) {
        self.output_lines.push(DebugOutputEntry {
            category: category.to_string(),
            text: text.to_string(),
        });
    }
}

impl View for DebugConsoleView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        // Panel background
        scene.push_quad(Quad::new(bounds).with_background(theme.panel_bg));

        let input_h = 28.0;
        let output_h = (bounds.height() - input_h).max(0.0);
        let _output_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), output_h);

        // Output lines
        let visible_lines = (output_h / font.line_height).ceil() as usize + 1;
        let start_idx = (self.scroll_top.floor() as usize).min(self.output_lines.len().saturating_sub(1));
        let end_idx = (start_idx + visible_lines).min(self.output_lines.len());

        for (i, entry) in self.output_lines[start_idx..end_idx].iter().enumerate() {
            let line_y = bounds.min_y() + (i as f32) * font.line_height + 4.0;
            let color = match entry.category.as_str() {
                "stderr" => theme.editor_error_foreground,
                "console" => theme.editor_info_foreground,
                _ => theme.editor_foreground,
            };

            scene.push_text(GlyphRun::new(
                entry.text.clone(),
                Point::new(bounds.min_x() + 10.0, line_y),
                Rect::from_xywh(bounds.min_x() + 10.0, line_y, bounds.width() - 20.0, font.line_height),
                color,
                font.font_size * 0.85,
            ));
        }

        // Bottom expression input prompt
        let input_rect = Rect::from_xywh(bounds.min_x(), bounds.max_y() - input_h, bounds.width(), input_h);
        scene.push_quad(
            Quad::new(input_rect)
                .with_background(theme.command_palette_bg)
                .with_border_edges(theme.panel_border, Edges::new(1.0, 0.0, 0.0, 0.0)),
        );

        // Prompt symbol
        scene.push_text(GlyphRun::new(
            ">".to_string(),
            Point::new(input_rect.min_x() + 8.0, input_rect.min_y() + 6.0),
            Rect::from_xywh(input_rect.min_x() + 8.0, input_rect.min_y() + 6.0, 16.0, font.line_height),
            theme.command_palette_border,
            font.font_size * 0.9,
        ));

        // Prompt input text
        let placeholder = if self.input_text.is_empty() {
            "Evaluate expression in current stack frame..."
        } else {
            &self.input_text
        };
        let text_color = if self.input_text.is_empty() {
            theme.sidebar_header_fg
        } else {
            theme.editor_foreground
        };
        scene.push_text(GlyphRun::new(
            placeholder.to_string(),
            Point::new(input_rect.min_x() + 24.0, input_rect.min_y() + 6.0),
            Rect::from_xywh(input_rect.min_x() + 24.0, input_rect.min_y() + 6.0, input_rect.width() - 32.0, font.line_height),
            text_color,
            font.font_size * 0.85,
        ));
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let input_h = 28.0;
        let input_rect = Rect::from_xywh(bounds.min_x(), bounds.max_y() - input_h, bounds.width(), input_h);

        match event {
            InputEvent::MouseDown(mouse) => {
                if input_rect.contains(mouse.position) {
                    self.input_focused = true;
                    EventResult::Consumed
                } else if bounds.contains(mouse.position) {
                    self.input_focused = false;
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            InputEvent::KeyDown(key) => {
                if !self.input_focused {
                    return EventResult::Ignored;
                }

                if key.key == KeyCode::Enter {
                    let expr = std::mem::take(&mut self.input_text);
                    if !expr.trim().is_empty() {
                        self.append_output("console", &format!("> {}\n", expr));
                        self.last_action = Some(DebugViewAction::EvaluateExpression {
                            expression: expr,
                        });
                    }
                    EventResult::Consumed
                } else if key.key == KeyCode::Backspace {
                    self.input_text.pop();
                    EventResult::Consumed
                } else if let KeyCode::Char(c) = key.key {
                    if !key.modifiers.ctrl && !key.modifiers.alt {
                        self.input_text.push(c);
                        EventResult::Consumed
                    } else {
                        EventResult::Ignored
                    }
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_toolbar_clicks_and_keybindings() {
        let mut toolbar = DebugToolbarView::new();
        let bounds = Rect::from_xywh(200.0, 10.0, 230.0, 32.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        let mut scene = Scene::new();
        toolbar.render(bounds, &theme, &font, &mut scene);

        // Click continue button (first slot)
        let click = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(bounds.min_x() + 10.0, bounds.min_y() + 10.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = toolbar.handle_event(&click, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert_eq!(toolbar.last_action, Some(DebugToolbarAction::Continue));

        // Press F10 (Step Over)
        let key_f10 = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::F10,
            rustcode_platform::keybindings::Modifiers::default(),
        ));
        let res_f10 = toolbar.handle_event(&key_f10, bounds);
        assert_eq!(res_f10, EventResult::Consumed);
        assert_eq!(toolbar.last_action, Some(DebugToolbarAction::StepOver));

        // Press Shift+F11 (Step Out)
        let key_shift_f11 = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::F11,
            rustcode_platform::keybindings::Modifiers {
                shift: true,
                ..Default::default()
            },
        ));
        let res_s_f11 = toolbar.handle_event(&key_shift_f11, bounds);
        assert_eq!(res_s_f11, EventResult::Consumed);
        assert_eq!(toolbar.last_action, Some(DebugToolbarAction::StepOut));
    }

    #[test]
    fn test_debug_sidebar_sections_and_jump() {
        let mut sidebar = DebugSidebarView::default();
        let bounds = Rect::from_xywh(0.0, 0.0, 300.0, 600.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        let mut scene = Scene::new();
        sidebar.render(bounds, &theme, &font, &mut scene);
        assert!(!scene.quads().is_empty());

        // Click call stack frame
        let targets = sidebar.click_targets.read().clone();
        let frame_target = targets
            .iter()
            .find(|(_, a)| matches!(a, DebugViewAction::SelectFrame { .. }))
            .unwrap();

        let click = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(frame_target.0.min_x() + 5.0, frame_target.0.min_y() + 5.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        let res = sidebar.handle_event(&click, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert!(matches!(sidebar.last_action, Some(DebugViewAction::SelectFrame { frame_id: 10, .. })));
    }
}
