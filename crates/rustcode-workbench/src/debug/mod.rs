//! DAP (Debug Adapter Protocol) Workbench Integration: Floating toolbar, Debug Sidebar, and Debug Console.

use parking_lot::RwLock;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect, Rgba,
    Scene, Shadow, View, WorkbenchThemeStyles,
};
use rustcode_protocols::dap::{
    load_launch_json, ContinuedEventBody, DapClient, DapError, DapMessage, LaunchConfigError,
    LaunchConfiguration, OutputEventBody, Scope, SourceBreakpoint, StackFrame, StoppedEventBody,
    Thread, Variable,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    StartDebugging {
        config_index: Option<usize>,
    },
    SelectLaunchConfig(usize),
    ToggleVariables,
    ToggleCallStack,
    ToggleBreakpoints,
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
            is_visible: false,
            is_paused: false,
            last_action: None,
            button_bounds: RwLock::new(Vec::new()),
        }
    }

    pub fn is_running(&self) -> bool {
        self.is_visible
    }

    pub fn on_session_started(&mut self, paused: bool) {
        self.is_visible = true;
        self.is_paused = paused;
    }

    pub fn on_session_stopped(&mut self) {
        self.is_visible = false;
        self.is_paused = false;
        self.last_action = None;
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.is_paused = paused;
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
                        if key.modifiers.ctrl {
                            Some(DebugToolbarAction::Restart)
                        } else if key.modifiers.shift {
                            Some(DebugToolbarAction::Stop)
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
    pub launch_configurations: Vec<LaunchConfiguration>,
    pub active_launch_config_index: Option<usize>,
    pub variables_collapsed: bool,
    pub callstack_collapsed: bool,
    pub breakpoints_collapsed: bool,
    pub last_action: Option<DebugViewAction>,
    pub click_targets: RwLock<Vec<(Rect, DebugViewAction)>>,
}

impl Default for DebugSidebarView {
    fn default() -> Self {
        Self::new(DebugSessionState::default())
    }
}

impl DebugSidebarView {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn new(session: DebugSessionState) -> Self {
        Self {
            session,
            launch_configurations: Vec::new(),
            active_launch_config_index: None,
            variables_collapsed: false,
            callstack_collapsed: false,
            breakpoints_collapsed: false,
            last_action: None,
            click_targets: RwLock::new(Vec::new()),
        }
    }

    pub fn set_launch_configurations(&mut self, configs: Vec<LaunchConfiguration>) {
        self.launch_configurations = configs;
        self.active_launch_config_index = if self.launch_configurations.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    pub fn active_launch_configuration(&self) -> Option<&LaunchConfiguration> {
        self.active_launch_config_index
            .and_then(|idx| self.launch_configurations.get(idx))
    }

    pub fn select_launch_configuration(&mut self, index: usize) -> bool {
        if index < self.launch_configurations.len() {
            self.active_launch_config_index = Some(index);
            true
        } else {
            false
        }
    }

    pub fn load_launch_json(&mut self, workspace_root: &Path) -> Result<usize, LaunchConfigError> {
        let parsed = load_launch_json(workspace_root)?;
        let count = parsed.configurations.len();
        self.set_launch_configurations(parsed.configurations);
        Ok(count)
    }

    pub fn on_session_started(&mut self) {
        self.session.is_active = true;
        self.session.is_paused = false;
    }

    pub fn on_session_terminated(&mut self) {
        self.session.is_active = false;
        self.session.is_paused = false;
        self.session.active_thread_id = None;
        self.session.active_frame_id = None;
        self.session.threads.clear();
        self.session.call_stack.clear();
        self.session.scopes.clear();
        self.session.variables.clear();
    }

    pub fn apply_threads(&mut self, threads: Vec<Thread>) {
        self.session.threads = threads;
        let still_valid = self
            .session
            .active_thread_id
            .is_some_and(|tid| self.session.threads.iter().any(|t| t.id == tid));
        if !still_valid {
            self.session.active_thread_id = self.session.threads.first().map(|t| t.id);
        }
    }

    pub fn apply_stack_frames(&mut self, frames: Vec<StackFrame>) {
        self.session.call_stack = frames;
        let still_valid = self
            .session
            .active_frame_id
            .is_some_and(|fid| self.session.call_stack.iter().any(|f| f.id == fid));
        if !still_valid {
            self.session.active_frame_id = self.session.call_stack.first().map(|f| f.id);
        }
    }

    pub fn apply_scopes(&mut self, scopes: Vec<Scope>) {
        self.session.scopes = scopes;
    }

    pub fn apply_variables(&mut self, variables_reference: u32, vars: Vec<Variable>) {
        self.session.variables.insert(variables_reference, vars);
    }

    pub fn append_console_output(&mut self, category: &str, text: &str) {
        self.session.console_output.push(DebugOutputEntry {
            category: category.to_string(),
            text: text.to_string(),
        });
    }

    pub fn apply_dap_event(&mut self, msg: &DapMessage) {
        let DapMessage::Event { event, body, .. } = msg else {
            return;
        };
        match event.as_str() {
            "stopped" => {
                self.session.is_active = true;
                self.session.is_paused = true;
                if let Some(b) = body
                    .as_ref()
                    .and_then(|v| serde_json::from_value::<StoppedEventBody>(v.clone()).ok())
                {
                    if let Some(tid) = b.thread_id {
                        self.session.active_thread_id = Some(tid);
                    }
                    if let Some(desc) = b.description.or(b.text) {
                        self.append_console_output("console", &format!("{}\n", desc));
                    }
                }
            }
            "continued" => {
                if let Some(b) = body
                    .as_ref()
                    .and_then(|v| serde_json::from_value::<ContinuedEventBody>(v.clone()).ok())
                {
                    if b.all_threads_continued.unwrap_or(true)
                        || self.session.active_thread_id == Some(b.thread_id)
                    {
                        self.session.is_paused = false;
                    }
                } else {
                    self.session.is_paused = false;
                }
            }
            "terminated" | "exited" => {
                self.on_session_terminated();
            }
            "output" => {
                if let Some(b) = body
                    .as_ref()
                    .and_then(|v| serde_json::from_value::<OutputEventBody>(v.clone()).ok())
                {
                    let cat = b.category.as_deref().unwrap_or("stdout");
                    self.append_console_output(cat, &b.output);
                }
            }
            _ => {}
        }
    }

    pub async fn sync_from_dap_client(
        &mut self,
        client: &DapClient,
        stopped_thread_id: Option<i64>,
    ) -> Result<(), DapError> {
        let threads = client.threads().await?;
        self.apply_threads(threads);
        self.session.is_active = true;
        self.session.is_paused = true;

        let target_thread = stopped_thread_id
            .or(self.session.active_thread_id)
            .or_else(|| self.session.threads.first().map(|t| t.id));
        self.session.active_thread_id = target_thread;

        if let Some(tid) = target_thread {
            let frames = client.stack_trace(tid).await?;
            self.apply_stack_frames(frames);

            if let Some(frame_id) = self.session.active_frame_id {
                let scopes = client.scopes(frame_id).await?;
                self.apply_scopes(scopes.clone());
                self.session.variables.clear();
                for scope in &scopes {
                    if scope.variables_reference > 0 && !scope.expensive {
                        if let Ok(vars) = client.variables(scope.variables_reference).await {
                            self.apply_variables(scope.variables_reference, vars);
                        }
                    }
                }
            } else {
                self.session.scopes.clear();
                self.session.variables.clear();
            }
        }

        Ok(())
    }

    pub async fn dispatch_toolbar_action(
        &mut self,
        action: &DebugToolbarAction,
        client: &DapClient,
    ) -> Result<(), DapError> {
        let tid = self
            .session
            .active_thread_id
            .or_else(|| self.session.threads.first().map(|t| t.id));
        match action {
            DebugToolbarAction::Continue => {
                if let Some(thread_id) = tid {
                    client.continue_execution(thread_id).await?;
                    self.session.is_paused = false;
                }
            }
            DebugToolbarAction::Pause => {
                if let Some(thread_id) = tid {
                    client
                        .send_request("pause", Some(serde_json::json!({ "threadId": thread_id })))
                        .await?;
                    self.session.is_paused = true;
                }
            }
            DebugToolbarAction::StepOver => {
                if let Some(thread_id) = tid {
                    client.next(thread_id).await?;
                }
            }
            DebugToolbarAction::StepInto => {
                if let Some(thread_id) = tid {
                    client.step_in(thread_id).await?;
                }
            }
            DebugToolbarAction::StepOut => {
                if let Some(thread_id) = tid {
                    client.step_out(thread_id).await?;
                }
            }
            DebugToolbarAction::Restart => {
                client.restart().await?;
            }
            DebugToolbarAction::Stop => {
                client.disconnect(false).await?;
                self.on_session_terminated();
            }
        }
        Ok(())
    }

    pub async fn dispatch_view_action(
        &mut self,
        action: &DebugViewAction,
        client: &DapClient,
    ) -> Result<Option<String>, DapError> {
        match action {
            DebugViewAction::Toolbar(tb) => {
                self.dispatch_toolbar_action(tb, client).await?;
                Ok(None)
            }
            DebugViewAction::SelectFrame { frame_id, .. } => {
                self.session.active_frame_id = Some(*frame_id);
                let scopes = client.scopes(*frame_id).await?;
                self.apply_scopes(scopes.clone());
                self.session.variables.clear();
                for scope in &scopes {
                    if scope.variables_reference > 0 && !scope.expensive {
                        if let Ok(vars) = client.variables(scope.variables_reference).await {
                            self.apply_variables(scope.variables_reference, vars);
                        }
                    }
                }
                Ok(None)
            }
            DebugViewAction::ToggleBreakpoint { path, .. } => {
                let bps = self
                    .session
                    .breakpoints
                    .get(path)
                    .cloned()
                    .unwrap_or_default();
                let _ = client.set_breakpoints(path, &bps).await?;
                Ok(None)
            }
            DebugViewAction::EvaluateExpression { expression } => {
                let body = client
                    .evaluate(expression, self.session.active_frame_id)
                    .await?;
                self.append_console_output("console", &format!("{}\n", body.result));
                Ok(Some(body.result))
            }
            DebugViewAction::SelectLaunchConfig(idx) => {
                self.select_launch_configuration(*idx);
                Ok(None)
            }
            DebugViewAction::JumpToBreakpoint { .. }
            | DebugViewAction::StartDebugging { .. }
            | DebugViewAction::ToggleVariables
            | DebugViewAction::ToggleCallStack
            | DebugViewAction::ToggleBreakpoints => Ok(None),
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

        let header_title = match self.active_launch_configuration() {
            Some(cfg) => format!("RUN AND DEBUG: {}", cfg.name),
            None => "RUN AND DEBUG".to_string(),
        };
        scene.push_text(GlyphRun::new(
            header_title,
            Point::new(bounds.min_x() + 12.0, bounds.min_y() + 10.0),
            Rect::from_xywh(bounds.min_x() + 12.0, bounds.min_y() + 10.0, (bounds.width() - 48.0).max(40.0), font.line_height),
            theme.sidebar_header_fg,
            font.font_size * 0.85,
        ));

        let start_btn_rect = Rect::from_xywh(bounds.max_x() - 28.0, bounds.min_y() + 8.0, 20.0, 20.0);
        targets.push((
            start_btn_rect,
            DebugViewAction::StartDebugging {
                config_index: self.active_launch_config_index,
            },
        ));
        scene.push_text(GlyphRun::new(
            "▶".to_string(),
            Point::new(start_btn_rect.min_x() + 4.0, start_btn_rect.min_y() + 2.0),
            start_btn_rect,
            theme.editor_info_foreground,
            font.font_size * 0.85,
        ));

        let mut cur_y = bounds.min_y() + header_h + 4.0;
        let section_h = 24.0;

        // 3. Section: VARIABLES
        let var_hdr_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), section_h);
        targets.push((var_hdr_rect, DebugViewAction::ToggleVariables));
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
        targets.push((cs_hdr_rect, DebugViewAction::ToggleCallStack));
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
        targets.push((bp_hdr_rect, DebugViewAction::ToggleBreakpoints));
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
                    match &action {
                        DebugViewAction::ToggleVariables => {
                            self.variables_collapsed = !self.variables_collapsed;
                        }
                        DebugViewAction::ToggleCallStack => {
                            self.callstack_collapsed = !self.callstack_collapsed;
                        }
                        DebugViewAction::ToggleBreakpoints => {
                            self.breakpoints_collapsed = !self.breakpoints_collapsed;
                        }
                        DebugViewAction::SelectFrame { frame_id, .. } => {
                            self.session.active_frame_id = Some(*frame_id);
                        }
                        DebugViewAction::SelectLaunchConfig(idx) => {
                            self.select_launch_configuration(*idx);
                        }
                        _ => {}
                    }
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
        assert!(!toolbar.is_visible);
        assert!(!toolbar.is_running());

        let bounds = Rect::from_xywh(200.0, 10.0, 230.0, 32.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        // While idle, toolbar does not render or consume events
        let mut idle_scene = Scene::new();
        toolbar.render(bounds, &theme, &font, &mut idle_scene);
        assert!(idle_scene.quads().is_empty());
        let idle_click = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(bounds.min_x() + 10.0, bounds.min_y() + 10.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        assert_eq!(toolbar.handle_event(&idle_click, bounds), EventResult::Ignored);

        // Start session -> toolbar becomes visible
        toolbar.on_session_started(true);
        assert!(toolbar.is_visible);
        assert!(toolbar.is_running());

        let mut scene = Scene::new();
        toolbar.render(bounds, &theme, &font, &mut scene);
        assert!(!scene.quads().is_empty());

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

        // Press Ctrl+Shift+F5 (Restart)
        let key_ctrl_shift_f5 = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::F5,
            rustcode_platform::keybindings::Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
        ));
        let res_restart = toolbar.handle_event(&key_ctrl_shift_f5, bounds);
        assert_eq!(res_restart, EventResult::Consumed);
        assert_eq!(toolbar.last_action, Some(DebugToolbarAction::Restart));

        // Press Shift+F5 (Stop)
        let key_shift_f5 = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::F5,
            rustcode_platform::keybindings::Modifiers {
                shift: true,
                ..Default::default()
            },
        ));
        let res_stop = toolbar.handle_event(&key_shift_f5, bounds);
        assert_eq!(res_stop, EventResult::Consumed);
        assert_eq!(toolbar.last_action, Some(DebugToolbarAction::Stop));

        // Stop session -> toolbar hides again
        toolbar.on_session_stopped();
        assert!(!toolbar.is_visible);
    }

    #[test]
    fn test_debug_sidebar_sections_and_jump() {
        let mut sidebar = DebugSidebarView::default();
        assert!(!sidebar.session.is_active);
        assert!(sidebar.session.threads.is_empty());
        assert!(sidebar.session.call_stack.is_empty());
        assert!(sidebar.session.scopes.is_empty());
        assert!(sidebar.session.variables.is_empty());
        assert!(sidebar.session.breakpoints.is_empty());

        sidebar.on_session_started();
        sidebar.apply_threads(vec![Thread {
            id: 1,
            name: "main thread".to_string(),
        }]);
        sidebar.apply_stack_frames(vec![StackFrame {
            id: 10,
            name: "main()".to_string(),
            source: Some(rustcode_protocols::dap::Source {
                name: Some("main.rs".to_string()),
                path: Some("src/main.rs".to_string()),
                source_reference: None,
            }),
            line: 14,
            column: 5,
        }]);
        sidebar.apply_scopes(vec![Scope {
            name: "Locals".to_string(),
            variables_reference: 100,
            expensive: false,
            named_variables: Some(1),
            indexed_variables: None,
        }]);
        sidebar.apply_variables(
            100,
            vec![Variable {
                name: "x".to_string(),
                value: "42".to_string(),
                type_name: Some("i32".to_string()),
                variables_reference: 0,
                evaluate_name: Some("x".to_string()),
            }],
        );

        let bounds = Rect::from_xywh(0.0, 0.0, 300.0, 600.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        let mut scene = Scene::new();
        sidebar.render(bounds, &theme, &font, &mut scene);
        assert!(!scene.quads().is_empty());
        assert!(scene.contains_text("x: 42 (i32)"));

        // Click variables header to collapse
        let targets = sidebar.click_targets.read().clone();
        let var_hdr = targets
            .iter()
            .find(|(_, a)| matches!(a, DebugViewAction::ToggleVariables))
            .unwrap();
        let click_var = InputEvent::MouseDown(rustcode_platform::render::MouseEvent::new(
            Point::new(var_hdr.0.min_x() + 5.0, var_hdr.0.min_y() + 5.0),
            rustcode_platform::render::MouseButton::Left,
        ));
        assert!(!sidebar.variables_collapsed);
        sidebar.handle_event(&click_var, bounds);
        assert!(sidebar.variables_collapsed);

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
        assert!(matches!(
            sidebar.last_action,
            Some(DebugViewAction::SelectFrame { frame_id: 10, .. })
        ));
    }

    #[tokio::test]
    async fn test_debug_sidebar_sync_from_dap_client_and_dispatch() {
        use rustcode_protocols::codec::{read_framed_message, write_framed_message};
        use tokio::io::duplex;

        let (client_io, server_io) = duplex(4096);
        let (c_reader, c_writer) = tokio::io::split(client_io);
        let client = DapClient::from_io(c_reader, c_writer);

        tokio::spawn(async move {
            let (mut s_reader, mut s_writer) = tokio::io::split(server_io);
            let mut buf = Vec::new();
            while let Ok(Some(bytes)) = read_framed_message(&mut s_reader, &mut buf).await {
                let msg: DapMessage = serde_json::from_slice(&bytes).unwrap();
                if let DapMessage::Request { seq, command, .. } = msg {
                    let body = match command.as_str() {
                        "threads" => Some(serde_json::json!({
                            "threads": [{ "id": 7, "name": "worker-7" }]
                        })),
                        "stackTrace" => Some(serde_json::json!({
                            "stackFrames": [{
                                "id": 21,
                                "name": "compute()",
                                "source": { "name": "lib.rs", "path": "src/lib.rs" },
                                "line": 9,
                                "column": 3
                            }]
                        })),
                        "scopes" => Some(serde_json::json!({
                            "scopes": [{
                                "name": "Locals",
                                "variablesReference": 300,
                                "expensive": false
                            }]
                        })),
                        "variables" => Some(serde_json::json!({
                            "variables": [{
                                "name": "total",
                                "value": "99",
                                "type": "usize",
                                "variablesReference": 0
                            }]
                        })),
                        "evaluate" => Some(serde_json::json!({
                            "result": "198",
                            "variablesReference": 0
                        })),
                        _ => None,
                    };
                    let resp = DapMessage::Response {
                        seq: seq + 100,
                        request_seq: seq,
                        success: true,
                        command: command.clone(),
                        message: None,
                        body,
                    };
                    let _ = write_framed_message(&mut s_writer, &serde_json::to_vec(&resp).unwrap())
                        .await;
                    if command == "disconnect" {
                        break;
                    }
                }
            }
        });

        let mut sidebar = DebugSidebarView::default();
        sidebar.sync_from_dap_client(&client, None).await.unwrap();
        assert!(sidebar.session.is_active);
        assert!(sidebar.session.is_paused);
        assert_eq!(sidebar.session.active_thread_id, Some(7));
        assert_eq!(sidebar.session.active_frame_id, Some(21));
        assert_eq!(sidebar.session.threads[0].name, "worker-7");
        assert_eq!(sidebar.session.call_stack[0].name, "compute()");
        assert_eq!(sidebar.session.scopes[0].name, "Locals");
        assert_eq!(sidebar.session.variables[&300][0].value, "99");

        let eval_res = sidebar
            .dispatch_view_action(
                &DebugViewAction::EvaluateExpression {
                    expression: "total * 2".to_string(),
                },
                &client,
            )
            .await
            .unwrap();
        assert_eq!(eval_res.as_deref(), Some("198"));
        assert!(sidebar
            .session
            .console_output
            .iter()
            .any(|e| e.text.contains("198")));

        sidebar
            .dispatch_toolbar_action(&DebugToolbarAction::StepOver, &client)
            .await
            .unwrap();
        sidebar
            .dispatch_toolbar_action(&DebugToolbarAction::Stop, &client)
            .await
            .unwrap();
        assert!(!sidebar.session.is_active);
        assert!(sidebar.session.threads.is_empty());
    }
}
