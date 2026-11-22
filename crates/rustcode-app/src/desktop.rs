//! Native desktop window & `winit` OS event loop (`#[cfg(feature = "desktop")]`).

use crate::runtime::{save_workbench_state, BootstrappedApp, RuntimeLoop};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::render::{
    sanitize_scale_factor, ClickTracker, InputEvent, KeyCode, KeyEvent, MouseButton, MouseEvent,
    MouseMoveEvent, Point, Rect, RenderSurface, ScrollEvent, WgpuRenderSurface,
};
use rustcode_workbench::WorkbenchWindow;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{
    ElementState, Ime, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode as WinitPhysicalKeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopWakeEvent {
    RuntimeTick,
}

/// Translate `winit::keyboard::ModifiersState` into RustCode `Modifiers`.
pub fn translate_winit_modifiers(state: ModifiersState) -> Modifiers {
    Modifiers {
        ctrl: state.control_key(),
        shift: state.shift_key(),
        alt: state.alt_key(),
        meta: state.super_key(),
    }
}

/// Compute safe logical viewport bounds from physical framebuffer pixels and scale factor.
pub fn safe_logical_bounds(phys_width: u32, phys_height: u32, scale_factor: f32) -> Rect {
    let scale = sanitize_scale_factor(scale_factor);
    if phys_width == 0 || phys_height == 0 {
        return Rect::from_xywh(0.0, 0.0, 0.0, 0.0);
    }
    Rect::from_xywh(
        0.0,
        0.0,
        (phys_width as f32 / scale).max(1.0),
        (phys_height as f32 / scale).max(1.0),
    )
}

fn physical_key_to_char(code: WinitPhysicalKeyCode) -> Option<char> {
    match code {
        WinitPhysicalKeyCode::KeyA => Some('a'),
        WinitPhysicalKeyCode::KeyB => Some('b'),
        WinitPhysicalKeyCode::KeyC => Some('c'),
        WinitPhysicalKeyCode::KeyD => Some('d'),
        WinitPhysicalKeyCode::KeyE => Some('e'),
        WinitPhysicalKeyCode::KeyF => Some('f'),
        WinitPhysicalKeyCode::KeyG => Some('g'),
        WinitPhysicalKeyCode::KeyH => Some('h'),
        WinitPhysicalKeyCode::KeyI => Some('i'),
        WinitPhysicalKeyCode::KeyJ => Some('j'),
        WinitPhysicalKeyCode::KeyK => Some('k'),
        WinitPhysicalKeyCode::KeyL => Some('l'),
        WinitPhysicalKeyCode::KeyM => Some('m'),
        WinitPhysicalKeyCode::KeyN => Some('n'),
        WinitPhysicalKeyCode::KeyO => Some('o'),
        WinitPhysicalKeyCode::KeyP => Some('p'),
        WinitPhysicalKeyCode::KeyQ => Some('q'),
        WinitPhysicalKeyCode::KeyR => Some('r'),
        WinitPhysicalKeyCode::KeyS => Some('s'),
        WinitPhysicalKeyCode::KeyT => Some('t'),
        WinitPhysicalKeyCode::KeyU => Some('u'),
        WinitPhysicalKeyCode::KeyV => Some('v'),
        WinitPhysicalKeyCode::KeyW => Some('w'),
        WinitPhysicalKeyCode::KeyX => Some('x'),
        WinitPhysicalKeyCode::KeyY => Some('y'),
        WinitPhysicalKeyCode::KeyZ => Some('z'),
        WinitPhysicalKeyCode::Digit0 => Some('0'),
        WinitPhysicalKeyCode::Digit1 => Some('1'),
        WinitPhysicalKeyCode::Digit2 => Some('2'),
        WinitPhysicalKeyCode::Digit3 => Some('3'),
        WinitPhysicalKeyCode::Digit4 => Some('4'),
        WinitPhysicalKeyCode::Digit5 => Some('5'),
        WinitPhysicalKeyCode::Digit6 => Some('6'),
        WinitPhysicalKeyCode::Digit7 => Some('7'),
        WinitPhysicalKeyCode::Digit8 => Some('8'),
        WinitPhysicalKeyCode::Digit9 => Some('9'),
        WinitPhysicalKeyCode::Backquote => Some('`'),
        WinitPhysicalKeyCode::Slash => Some('/'),
        WinitPhysicalKeyCode::Backslash => Some('\\'),
        WinitPhysicalKeyCode::Minus => Some('-'),
        WinitPhysicalKeyCode::Equal => Some('='),
        WinitPhysicalKeyCode::BracketLeft => Some('['),
        WinitPhysicalKeyCode::BracketRight => Some(']'),
        WinitPhysicalKeyCode::Semicolon => Some(';'),
        WinitPhysicalKeyCode::Quote => Some('\''),
        WinitPhysicalKeyCode::Comma => Some(','),
        WinitPhysicalKeyCode::Period => Some('.'),
        _ => None,
    }
}

/// Translate `winit` logical + physical key into RustCode `KeyCode`.
pub fn winit_key_to_keycode(
    logical_key: &Key,
    physical_key: Option<PhysicalKey>,
    modifiers: Modifiers,
) -> Option<KeyCode> {
    match logical_key {
        Key::Named(named) => match named {
            NamedKey::Enter => Some(KeyCode::Enter),
            NamedKey::Escape => Some(KeyCode::Escape),
            NamedKey::Backspace => Some(KeyCode::Backspace),
            NamedKey::Tab => Some(KeyCode::Tab),
            NamedKey::Space => Some(KeyCode::Space),
            NamedKey::Delete => Some(KeyCode::Delete),
            NamedKey::ArrowUp => Some(KeyCode::ArrowUp),
            NamedKey::ArrowDown => Some(KeyCode::ArrowDown),
            NamedKey::ArrowLeft => Some(KeyCode::ArrowLeft),
            NamedKey::ArrowRight => Some(KeyCode::ArrowRight),
            NamedKey::Home => Some(KeyCode::Home),
            NamedKey::End => Some(KeyCode::End),
            NamedKey::PageUp => Some(KeyCode::PageUp),
            NamedKey::PageDown => Some(KeyCode::PageDown),
            NamedKey::F1 => Some(KeyCode::F1),
            NamedKey::F2 => Some(KeyCode::F2),
            NamedKey::F3 => Some(KeyCode::F3),
            NamedKey::F4 => Some(KeyCode::F4),
            NamedKey::F5 => Some(KeyCode::F5),
            NamedKey::F6 => Some(KeyCode::F6),
            NamedKey::F7 => Some(KeyCode::F7),
            NamedKey::F8 => Some(KeyCode::F8),
            NamedKey::F9 => Some(KeyCode::F9),
            NamedKey::F10 => Some(KeyCode::F10),
            NamedKey::F11 => Some(KeyCode::F11),
            NamedKey::F12 => Some(KeyCode::F12),
            _ => None,
        },
        Key::Character(s) => {
            if s.as_str() == " " {
                return Some(KeyCode::Space);
            }
            if let Some(ch) = s.chars().next() {
                if ch.is_ascii_control() && modifiers.ctrl {
                    let b = ch as u8;
                    if (1..=26).contains(&b) {
                        return Some(KeyCode::Char((b'a' + (b - 1)) as char));
                    }
                    if let Some(PhysicalKey::Code(code)) = physical_key {
                        if let Some(mapped) = physical_key_to_char(code) {
                            return Some(KeyCode::Char(mapped));
                        }
                    }
                }
                return Some(KeyCode::Char(ch));
            }
            None
        }
        _ => {
            if let Some(PhysicalKey::Code(code)) = physical_key {
                physical_key_to_char(code).map(KeyCode::Char)
            } else {
                None
            }
        }
    }
}

/// Translate a `winit` keyboard event into `InputEvent::KeyDown` or `InputEvent::KeyUp`.
pub fn translate_winit_key_event(
    logical_key: &Key,
    physical_key: Option<PhysicalKey>,
    text: Option<&str>,
    state: ElementState,
    modifiers: Modifiers,
) -> Option<InputEvent> {
    let key = winit_key_to_keycode(logical_key, physical_key, modifiers)?;
    let mut key_ev = KeyEvent::new(key, modifiers);
    if !modifiers.ctrl && !modifiers.alt && !modifiers.meta {
        if let Some(t) = text {
            if !t.is_empty() && !t.chars().all(char::is_control) {
                key_ev.text = Some(t.to_string());
            }
        }
    } else {
        key_ev.text = None;
    }

    Some(match state {
        ElementState::Pressed => InputEvent::KeyDown(key_ev),
        ElementState::Released => InputEvent::KeyUp(key_ev),
    })
}

/// Translate `winit::event::Ime` into `InputEvent::ImePreedit` / `InputEvent::ImeCommit`.
pub fn translate_winit_ime(ime: &Ime) -> Option<InputEvent> {
    match ime {
        Ime::Preedit(text, cursor) => Some(InputEvent::ImePreedit {
            text: text.clone(),
            cursor: *cursor,
        }),
        Ime::Commit(text) => Some(InputEvent::ImeCommit(text.clone())),
        Ime::Enabled | Ime::Disabled => None,
    }
}

/// Translate `winit::event::MouseButton` into RustCode `MouseButton`.
pub fn translate_winit_mouse_button(button: WinitMouseButton) -> Option<MouseButton> {
    match button {
        WinitMouseButton::Left => Some(MouseButton::Left),
        WinitMouseButton::Right => Some(MouseButton::Right),
        WinitMouseButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

/// Translate `winit::event::MouseScrollDelta` into `InputEvent::ScrollWheel`.
pub fn translate_winit_mouse_wheel(
    delta: MouseScrollDelta,
    cursor_pos: Point,
    modifiers: Modifiers,
    line_height: f32,
    scale_factor: f32,
) -> InputEvent {
    let safe_line_h = if line_height.is_finite() && line_height > 0.0 {
        line_height
    } else {
        20.0
    };
    let safe_scale = sanitize_scale_factor(scale_factor);
    let (dx, dy) = match delta {
        MouseScrollDelta::LineDelta(x, y) => {
            let sx = if x.is_finite() { x * safe_line_h * 3.0 } else { 0.0 };
            let sy = if y.is_finite() { y * safe_line_h * 3.0 } else { 0.0 };
            (sx, sy)
        }
        MouseScrollDelta::PixelDelta(pos) => {
            let sx = if pos.x.is_finite() {
                (pos.x as f32) / safe_scale
            } else {
                0.0
            };
            let sy = if pos.y.is_finite() {
                (pos.y as f32) / safe_scale
            } else {
                0.0
            };
            (sx, sy)
        }
    };
    InputEvent::ScrollWheel(ScrollEvent {
        position: cursor_pos,
        delta_x: dx,
        delta_y: dy,
        modifiers,
    })
}

fn attach_webviews_to_window<W>(
    window: &mut WorkbenchWindow,
    os_window: &Arc<W>,
    attached: &mut HashSet<String>,
) where
    W: HasWindowHandle + HasDisplayHandle,
{
    let mut live_ids = HashSet::new();
    for (id, wv) in &mut window.view.editor_area.webviews {
        live_ids.insert(id.clone());
        if !attached.contains(id) {
            if let Err(err) = wv.upgrade_to_wry(Some(os_window)) {
                eprintln!("Warning: failed to attach WryWebviewBackend '{id}': {err}");
            }
            attached.insert(id.clone());
        }
    }
    for ((group_id, id), wv) in &mut window.view.editor_area.split_webviews {
        let key = format!("split:{group_id}:{id}");
        live_ids.insert(key.clone());
        if !attached.contains(&key) {
            if let Err(err) = wv.upgrade_to_wry(Some(os_window)) {
                eprintln!("Warning: failed to attach split WryWebviewBackend '{key}': {err}");
            }
            attached.insert(key);
        }
    }
    attached.retain(|k| live_ids.contains(k));
}

struct DesktopApp {
    window: WorkbenchWindow,
    runtime_loop: RuntimeLoop,
    os_window: Option<Arc<Window>>,
    surface: Option<WgpuRenderSurface>,
    modifiers: Modifiers,
    cursor_pos: Point,
    click_tracker: ClickTracker,
    start_instant: Instant,
    scale_factor: f32,
    bounds: Rect,
    attached_webviews: HashSet<String>,
    proxy: EventLoopProxy<DesktopWakeEvent>,
    ticker_started: bool,
    shutdown_done: bool,
}

impl DesktopApp {
    fn new(bootstrapped: BootstrappedApp, proxy: EventLoopProxy<DesktopWakeEvent>) -> Self {
        Self {
            window: bootstrapped.window,
            runtime_loop: bootstrapped.runtime_loop,
            os_window: None,
            surface: None,
            modifiers: Modifiers::none(),
            cursor_pos: Point::zero(),
            click_tracker: ClickTracker::default(),
            start_instant: Instant::now(),
            scale_factor: 1.0,
            bounds: Rect::from_xywh(0.0, 0.0, 1280.0, 800.0),
            attached_webviews: HashSet::new(),
            proxy,
            ticker_started: false,
            shutdown_done: false,
        }
    }

    fn perform_shutdown(&mut self) {
        if self.shutdown_done {
            return;
        }
        self.shutdown_done = true;
        let _ = save_workbench_state(&self.window);
        self.runtime_loop.shutdown(&mut self.window);
    }

    fn request_redraw(&self) {
        if let Some(win) = &self.os_window {
            win.request_redraw();
        }
    }
}

impl ApplicationHandler<DesktopWakeEvent> for DesktopApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.os_window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("RustCode")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));

        let created = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(err) => {
                eprintln!("Failed to create desktop window: {err}");
                self.perform_shutdown();
                event_loop.exit();
                return;
            }
        };

        created.set_ime_allowed(true);
        let scale = sanitize_scale_factor(created.scale_factor() as f32);
        let phys = created.inner_size();
        self.scale_factor = scale;
        self.bounds = safe_logical_bounds(phys.width, phys.height, scale);

        match WgpuRenderSurface::new_for_window(Arc::clone(&created), phys.width, phys.height, scale)
        {
            Ok(surface) => {
                self.surface = Some(surface);
            }
            Err(err) => {
                eprintln!("Failed to initialize WgpuRenderSurface: {err}");
                self.perform_shutdown();
                event_loop.exit();
                return;
            }
        }

        attach_webviews_to_window(&mut self.window, &created, &mut self.attached_webviews);
        created.request_redraw();
        self.os_window = Some(created);

        if !self.ticker_started {
            self.ticker_started = true;
            let proxy = self.proxy.clone();
            self.runtime_loop.tokio_handle().spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(50));
                loop {
                    interval.tick().await;
                    if proxy.send_event(DesktopWakeEvent::RuntimeTick).is_err() {
                        break;
                    }
                }
            });
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: DesktopWakeEvent) {
        match event {
            DesktopWakeEvent::RuntimeTick => {
                self.runtime_loop.tick(&mut self.window);
                if let Some(os_win) = &self.os_window {
                    attach_webviews_to_window(
                        &mut self.window,
                        os_win,
                        &mut self.attached_webviews,
                    );
                    os_win.request_redraw();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.perform_shutdown();
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                if let Some(surface) = &mut self.surface {
                    surface.resize(new_size.width, new_size.height, self.scale_factor);
                }
                self.bounds =
                    safe_logical_bounds(new_size.width, new_size.height, self.scale_factor);
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale_factor = sanitize_scale_factor(scale_factor as f32);
                if let Some(os_win) = &self.os_window {
                    let phys = os_win.inner_size();
                    if let Some(surface) = &mut self.surface {
                        surface.resize(phys.width, phys.height, self.scale_factor);
                    }
                    self.bounds =
                        safe_logical_bounds(phys.width, phys.height, self.scale_factor);
                }
                self.request_redraw();
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = translate_winit_modifiers(mods.state());
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(input_ev) = translate_winit_key_event(
                    &event.logical_key,
                    Some(event.physical_key),
                    event.text.as_deref(),
                    event.state,
                    self.modifiers,
                ) {
                    self.window.dispatch_event(&input_ev, self.bounds);
                    self.request_redraw();
                }
            }
            WindowEvent::Ime(ime) => {
                if let Some(input_ev) = translate_winit_ime(&ime) {
                    self.window.dispatch_event(&input_ev, self.bounds);
                    self.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.scale_factor as f64;
                self.cursor_pos = Point::new(
                    (position.x / scale) as f32,
                    (position.y / scale) as f32,
                );
                let move_ev = InputEvent::MouseMove(MouseMoveEvent {
                    position: self.cursor_pos,
                });
                self.window.dispatch_event(&move_ev, self.bounds);
                self.request_redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(btn) = translate_winit_mouse_button(button) {
                    let input_ev = match state {
                        ElementState::Pressed => {
                            let ts_ms = self.start_instant.elapsed().as_millis() as u64;
                            let count =
                                self.click_tracker
                                    .register_click(btn, self.cursor_pos, ts_ms);
                            InputEvent::MouseDown(
                                MouseEvent::new(self.cursor_pos, btn)
                                    .with_click_count(count)
                                    .with_modifiers(self.modifiers),
                            )
                        }
                        ElementState::Released => InputEvent::MouseUp(
                            MouseEvent::new(self.cursor_pos, btn).with_modifiers(self.modifiers),
                        ),
                    };
                    self.window.dispatch_event(&input_ev, self.bounds);
                    self.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let line_h = self.window.font_metrics.line_height;
                let scroll_ev = translate_winit_mouse_wheel(
                    delta,
                    self.cursor_pos,
                    self.modifiers,
                    line_h,
                    self.scale_factor,
                );
                self.window.dispatch_event(&scroll_ev, self.bounds);
                self.request_redraw();
            }
            WindowEvent::Focused(false) => {
                self.runtime_loop.on_window_blur(&mut self.window);
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.runtime_loop.tick(&mut self.window);
                if let Some(os_win) = &self.os_window {
                    attach_webviews_to_window(
                        &mut self.window,
                        os_win,
                        &mut self.attached_webviews,
                    );
                }
                if let Some(surface) = &mut self.surface {
                    let sz = surface.size();
                    if sz.width > 0.0 && sz.height > 0.0 {
                        let _ = self.window.render(surface);
                    }
                }
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.perform_shutdown();
    }
}

/// Run the native `winit` + `WgpuRenderSurface` desktop event loop.
pub fn run_desktop_event_loop(bootstrapped: BootstrappedApp) -> Result<(), String> {
    let event_loop = EventLoop::<DesktopWakeEvent>::with_user_event()
        .build()
        .map_err(|e| format!("Failed to create winit event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut app = DesktopApp::new(bootstrapped, proxy);
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("Desktop event loop error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;

    #[test]
    fn test_winit_key_and_modifiers_translation() {
        let mods = translate_winit_modifiers(
            ModifiersState::CONTROL | ModifiersState::SHIFT,
        );
        assert!(mods.ctrl);
        assert!(mods.shift);
        assert!(!mods.alt);
        assert!(!mods.meta);

        // Named keys (arrows, F1..F12, Enter, Escape, Backspace, Delete, Tab)
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::Enter), None, Modifiers::none()),
            Some(KeyCode::Enter)
        );
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::F12), None, Modifiers::none()),
            Some(KeyCode::F12)
        );
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::ArrowUp), None, Modifiers::none()),
            Some(KeyCode::ArrowUp)
        );
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::Delete), None, Modifiers::none()),
            Some(KeyCode::Delete)
        );

        // Ctrl+B control char fallback
        let ctrl_only = Modifiers {
            ctrl: true,
            ..Modifiers::none()
        };
        assert_eq!(
            winit_key_to_keycode(
                &Key::Character("\u{2}".into()),
                Some(PhysicalKey::Code(WinitPhysicalKeyCode::KeyB)),
                ctrl_only
            ),
            Some(KeyCode::Char('b'))
        );

        // KeyDown / KeyUp with IME-safe text suppression on Ctrl shortcuts
        let down = translate_winit_key_event(
            &Key::Character("a".into()),
            Some(PhysicalKey::Code(WinitPhysicalKeyCode::KeyA)),
            Some("a"),
            ElementState::Pressed,
            Modifiers::none(),
        )
        .unwrap();
        assert!(matches!(
            down,
            InputEvent::KeyDown(KeyEvent { key: KeyCode::Char('a'), text: Some(ref t), .. }) if t == "a"
        ));

        let ctrl_down = translate_winit_key_event(
            &Key::Character("a".into()),
            Some(PhysicalKey::Code(WinitPhysicalKeyCode::KeyA)),
            Some("\u{1}"),
            ElementState::Pressed,
            ctrl_only,
        )
        .unwrap();
        assert!(matches!(
            ctrl_down,
            InputEvent::KeyDown(KeyEvent { key: KeyCode::Char('a'), text: None, .. })
        ));

        let up = translate_winit_key_event(
            &Key::Named(NamedKey::Escape),
            None,
            None,
            ElementState::Released,
            Modifiers::none(),
        )
        .unwrap();
        assert!(matches!(
            up,
            InputEvent::KeyUp(KeyEvent { key: KeyCode::Escape, .. })
        ));
    }

    #[test]
    fn test_winit_ime_mouse_wheel_and_zero_resize_safety() {
        // IME Preedit & Commit
        assert_eq!(
            translate_winit_ime(&Ime::Preedit("nihao".to_string(), Some((0, 5)))),
            Some(InputEvent::ImePreedit {
                text: "nihao".to_string(),
                cursor: Some((0, 5)),
            })
        );
        assert_eq!(
            translate_winit_ime(&Ime::Commit("你好".to_string())),
            Some(InputEvent::ImeCommit("你好".to_string()))
        );
        assert_eq!(translate_winit_ime(&Ime::Enabled), None);

        // Mouse buttons + ClickTracker 1/2/3 detection
        assert_eq!(
            translate_winit_mouse_button(WinitMouseButton::Left),
            Some(MouseButton::Left)
        );
        assert_eq!(
            translate_winit_mouse_button(WinitMouseButton::Right),
            Some(MouseButton::Right)
        );
        assert_eq!(
            translate_winit_mouse_button(WinitMouseButton::Middle),
            Some(MouseButton::Middle)
        );
        assert_eq!(translate_winit_mouse_button(WinitMouseButton::Back), None);

        let mut tracker = ClickTracker::default();
        let pt = Point::new(40.0, 50.0);
        assert_eq!(tracker.register_click(MouseButton::Left, pt, 100), 1);
        assert_eq!(tracker.register_click(MouseButton::Left, pt, 250), 2);
        assert_eq!(tracker.register_click(MouseButton::Left, pt, 400), 3);

        // MouseWheel line delta & pixel delta (with scale_factor = 2.0)
        let line_scroll = translate_winit_mouse_wheel(
            MouseScrollDelta::LineDelta(0.0, -1.0),
            pt,
            Modifiers::none(),
            20.0,
            1.0,
        );
        assert!(matches!(
            line_scroll,
            InputEvent::ScrollWheel(ScrollEvent { delta_y, .. }) if (delta_y + 60.0).abs() < 1e-4
        ));

        let px_scroll = translate_winit_mouse_wheel(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(10.0, 20.0)),
            pt,
            Modifiers::none(),
            20.0,
            2.0,
        );
        assert!(matches!(
            px_scroll,
            InputEvent::ScrollWheel(ScrollEvent { delta_x, delta_y, .. })
                if (delta_x - 5.0).abs() < 1e-4 && (delta_y - 10.0).abs() < 1e-4
        ));

        // 0x0 minimized window size safety
        let zero_bounds = safe_logical_bounds(0, 0, 1.5);
        assert_eq!(zero_bounds.width(), 0.0);
        assert_eq!(zero_bounds.height(), 0.0);

        let hidpi_bounds = safe_logical_bounds(2560, 1600, 2.0);
        assert!((hidpi_bounds.width() - 1280.0).abs() < 1e-4);
        assert!((hidpi_bounds.height() - 800.0).abs() < 1e-4);
    }
}
