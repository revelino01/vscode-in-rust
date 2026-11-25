#[allow(dead_code)]
use rustcode_app::{RustCodeContext, WorkbenchWindow};
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::render::{
    EventResult, InputEvent, KeyCode, KeyEvent, MockRenderSurface, MouseButton, MouseEvent,
    MouseMoveEvent, Point, Rect, Scene,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Returns the repository root (`.../vscode-in-rust`).
#[allow(dead_code)]
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("tests/phase9-e2e must sit two levels below repo root")
        .to_path_buf()
}

/// RAII temporary workspace directory cleaned automatically on drop.
#[allow(dead_code)]
pub struct TempWorkspace {
    root: PathBuf,
}

#[allow(dead_code)]
impl TempWorkspace {
    pub fn new(prefix: &str) -> Self {
        let dir_name = format!("{prefix}_{}_{}", std::process::id(), Uuid::new_v4().simple());
        let root = std::env::temp_dir().join(dir_name);
        std::fs::create_dir_all(&root).expect("failed to create temp workspace directory");
        Self { root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn write_file(&self, rel: impl AsRef<Path>, content: &str) -> PathBuf {
        let target = self.root.join(rel.as_ref());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("failed creating parent dirs");
        }
        std::fs::write(&target, content).expect("failed writing temp file");
        target
    }

    pub fn mkdir_all(&self, rel: impl AsRef<Path>) -> PathBuf {
        let target = self.root.join(rel.as_ref());
        std::fs::create_dir_all(&target).expect("failed creating dir");
        target
    }

    pub fn read_file(&self, rel: impl AsRef<Path>) -> String {
        std::fs::read_to_string(self.root.join(rel.as_ref())).expect("failed reading temp file")
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[allow(dead_code)]
pub fn create_test_window() -> (RustCodeContext, WorkbenchWindow) {
    let ctx = RustCodeContext::new();
    let win = ctx.create_window();
    (ctx, win)
}

#[allow(dead_code)]
pub fn render_window_scene(win: &WorkbenchWindow, width: f32, height: f32) -> Scene {
    let mut surface = MockRenderSurface::new(width, height);
    win.render(&mut surface)
}

#[allow(dead_code)]
pub fn key_down(key: KeyCode, modifiers: Modifiers) -> InputEvent {
    InputEvent::KeyDown(KeyEvent::new(key, modifiers))
}

#[allow(dead_code)]
pub fn mouse_down(pos: Point, button: MouseButton) -> InputEvent {
    InputEvent::MouseDown(MouseEvent::new(pos, button))
}

#[allow(dead_code)]
pub fn mouse_move(pos: Point) -> InputEvent {
    InputEvent::MouseMove(MouseMoveEvent { position: pos })
}

#[allow(dead_code)]
pub fn mouse_up(pos: Point, button: MouseButton) -> InputEvent {
    InputEvent::MouseUp(MouseEvent::new(pos, button))
}

#[allow(dead_code)]
pub fn dispatch(win: &mut WorkbenchWindow, event: &InputEvent, bounds: Rect) -> EventResult {
    win.dispatch_event(event, bounds)
}

#[allow(dead_code)]
pub fn exec_cmd(ctx: &RustCodeContext, win: &mut WorkbenchWindow, cmd_id: &str) {
    ctx.commands
        .execute(cmd_id, None)
        .unwrap_or_else(|e| panic!("command {cmd_id} failed: {e}"));
    win.sync_all();
}
