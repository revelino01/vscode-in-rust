use rustcode_app::{RustCodeContext, WorkbenchWindow};
use rustcode_editor::EditorView;
use rustcode_ext_host::{
    ExtensionLoader, LanguageFeatureBridge, ProviderRegistry, RpcBridge,
};
use rustcode_platform::accessibility::AccessibilityTree;
use rustcode_platform::commands::CommandRegistry;
use rustcode_platform::configuration::ConfigurationService;
use rustcode_platform::fs::{
    file_uri_to_path, is_file_uri, path_to_file_uri, paths_equivalent, FileSystemService,
};
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::render::{
    EventResult, InputEvent, KeyCode, KeyEvent, MockRenderSurface, MouseButton, MouseEvent, Point,
    Rect, Scene,
};
use rustcode_protocols::lsp::LspClientManager;
use rustcode_workbench::workspace::Workspace;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use uuid::Uuid;

/// Returns the repository root (`c:\...\vscode-in-rust` or `/.../vscode-in-rust`).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("tests/phase7-e2e must sit two levels below repo root")
        .to_path_buf()
}

/// RAII temporary directory workspace cleaned up automatically on `Drop`.
pub struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    pub fn new() -> Self {
        let dir_name = format!(
            "rustcode_phase7_e2e_{}_{}",
            std::process::id(),
            Uuid::new_v4().simple()
        );
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
            std::fs::create_dir_all(parent).expect("failed to create parent directories");
        }
        std::fs::write(&target, content).expect("failed to write temp workspace file");
        target
    }

    pub fn read_file(&self, rel: impl AsRef<Path>) -> String {
        let target = self.root.join(rel.as_ref());
        std::fs::read_to_string(&target).expect("failed to read temp workspace file")
    }

    pub fn exists(&self, rel: impl AsRef<Path>) -> bool {
        self.root.join(rel.as_ref()).exists()
    }

    pub fn file_uri(&self, rel: impl AsRef<Path>) -> String {
        path_to_file_uri(&self.root.join(rel.as_ref()))
    }
}

impl Default for TempWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Opaque-box test driver wrapping `RustCodeContext`, `WorkbenchWindow`, `MockRenderSurface`,
/// and viewport `Rect` bounds (default `1024.0 x 768.0`).
pub struct E2eWindow {
    pub app: RustCodeContext,
    pub window: WorkbenchWindow,
    pub surface: MockRenderSurface,
    pub bounds: Rect,
}

impl E2eWindow {
    pub fn new() -> Self {
        let app = RustCodeContext::new();
        let window = app.create_window();
        let surface = MockRenderSurface::new(1024.0, 768.0);
        let bounds = Rect::from_xywh(0.0, 0.0, 1024.0, 768.0);
        Self {
            app,
            window,
            surface,
            bounds,
        }
    }

    pub fn with_temp_workspace(ws: &TempWorkspace) -> Self {
        let mut win = Self::new();
        win.window.view.workspace = Workspace::from_folder(ws.path().to_path_buf());
        win.window
            .view
            .explorer
            .model
            .sync_from_workspace(&win.window.view.workspace);
        win.window
            .view
            .explorer
            .model
            .expanded_paths
            .insert(ws.path().to_path_buf());
        win.window.sync_configuration();
        win
    }

    fn resolve_path(&self, rel_path: impl AsRef<Path>) -> PathBuf {
        let p = rel_path.as_ref();
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(folder) = self.window.view.workspace.folders().first() {
            folder.path.join(p)
        } else {
            p.to_path_buf()
        }
    }

    pub fn open_file_from_disk(&mut self, rel_path: impl AsRef<Path>) -> String {
        let full_path = self.resolve_path(rel_path);
        let content =
            std::fs::read_to_string(&full_path).expect("failed to read file from disk");
        let uri = path_to_file_uri(&full_path);
        let title = full_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string());
        self.window.open_file(&uri, &title, &content, false);
        uri
    }

    pub fn dispatch_key(
        &mut self,
        key: KeyCode,
        modifiers: Modifiers,
        text: Option<&str>,
    ) -> EventResult {
        let mut event = KeyEvent::new(key, modifiers);
        if let Some(t) = text {
            event = event.with_text(t.to_string());
        }
        self.window
            .dispatch_event(&InputEvent::KeyDown(event), self.bounds)
    }

    pub fn dispatch_ctrl_char(&mut self, ch: char) -> EventResult {
        self.dispatch_key(
            KeyCode::Char(ch),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
            None,
        )
    }

    pub fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' => {
                    self.dispatch_key(KeyCode::Enter, Modifiers::none(), Some("\n"));
                }
                '\t' => {
                    self.dispatch_key(KeyCode::Tab, Modifiers::none(), Some("\t"));
                }
                _ => {
                    let mut buf = [0u8; 4];
                    let s = ch.encode_utf8(&mut buf);
                    self.dispatch_key(KeyCode::Char(ch), Modifiers::none(), Some(s));
                }
            }
        }
    }

    pub fn click(&mut self, point: Point) -> EventResult {
        let down = self.window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(point, MouseButton::Left)),
            self.bounds,
        );
        let up = self.window.dispatch_event(
            &InputEvent::MouseUp(MouseEvent::new(point, MouseButton::Left)),
            self.bounds,
        );
        if down.is_consumed() || up.is_consumed() {
            EventResult::Consumed
        } else {
            EventResult::Ignored
        }
    }

    pub fn render(&mut self) -> Scene {
        self.window.render(&mut self.surface)
    }

    pub fn accessibility_tree(&self) -> AccessibilityTree {
        self.window.accessibility_tree(self.bounds)
    }

    pub fn click_scene_text(&mut self, needle: &str) -> bool {
        let scene = self.render();
        let target_center = scene
            .glyph_runs()
            .into_iter()
            .rev()
            .find(|g| g.text.contains(needle))
            .map(|g| {
                Point::new(
                    g.bounds.min_x() + g.bounds.width() * 0.5,
                    g.bounds.min_y() + g.bounds.height() * 0.5,
                )
            });
        if let Some(center) = target_center {
            self.click(center);
            true
        } else {
            false
        }
    }

    pub fn click_a11y_node(&mut self, id_or_name: &str) -> bool {
        let tree = self.accessibility_tree();
        let found_bounds = if let Some(node) = tree.find_by_id(id_or_name) {
            Some(node.bounds)
        } else {
            let flat = tree.to_flat_list();
            let is_overlay = |n: &&&rustcode_platform::accessibility::AccessibleNode| {
                n.id.starts_with("notifications.")
                    || n.id.starts_with("prompt.")
                    || n.id.starts_with("workbench.commandPalette")
                    || n.id.starts_with("commandPalette.")
            };
            let overlay_match = flat
                .iter()
                .rev()
                .filter(is_overlay)
                .find(|n| n.name == id_or_name)
                .or_else(|| {
                    flat.iter()
                        .rev()
                        .filter(is_overlay)
                        .find(|n| n.name.contains(id_or_name))
                })
                .map(|n| n.bounds);
            let sidebar_match = if overlay_match.is_none()
                && self.window.view.focused_region
                    == rustcode_workbench::workbench_view::FocusedRegion::Sidebar
            {
                tree.find_by_id("workbench.sidebar").and_then(|sidebar| {
                    fn collect<'a>(
                        n: &'a rustcode_platform::accessibility::AccessibleNode,
                        out: &mut Vec<&'a rustcode_platform::accessibility::AccessibleNode>,
                    ) {
                        out.push(n);
                        for c in &n.children {
                            collect(c, out);
                        }
                    }
                    let mut s_flat = Vec::new();
                    collect(sidebar, &mut s_flat);
                    s_flat
                        .iter()
                        .rev()
                        .find(|n| n.name == id_or_name)
                        .or_else(|| s_flat.iter().rev().find(|n| n.name.contains(id_or_name)))
                        .map(|n| n.bounds)
                })
            } else {
                None
            };
            overlay_match.or(sidebar_match).or_else(|| {
                flat.iter()
                    .rev()
                    .find(|n| n.name == id_or_name)
                    .or_else(|| flat.iter().rev().find(|n| n.name.contains(id_or_name)))
                    .map(|n| n.bounds)
            })
        };
        if let Some(b) = found_bounds {
            let center = Point::new(b.min_x() + b.width() * 0.5, b.min_y() + b.height() * 0.5);
            self.click(center);
            true
        } else {
            false
        }
    }

    pub fn active_editor(&self) -> Option<&EditorView> {
        let gid = self.window.view.editor_area.model.active_group_id();
        let uri = self
            .window
            .view
            .editor_area
            .model
            .current_group_ref()
            .active_tab()
            .map(|t| t.uri.as_str())?;
        self.window.view.editor_area.editor_for_group(gid, uri)
    }

    pub fn active_editor_text(&self) -> Option<String> {
        self.active_editor().map(EditorView::text)
    }

    pub fn read_doc(&self, path: impl AsRef<Path>) -> Option<String> {
        let raw = path.as_ref().to_string_lossy();
        if let Some(ed) = self.window.view.editor_area.any_editor_for_uri(&raw) {
            return Some(ed.text());
        }

        let target_path = if is_file_uri(&raw) {
            file_uri_to_path(&raw)
        } else {
            self.resolve_path(path.as_ref())
        };

        let uri = path_to_file_uri(&target_path);
        if let Some(ed) = self.window.view.editor_area.any_editor_for_uri(&uri) {
            return Some(ed.text());
        }

        for (open_uri, ed) in &self.window.view.editor_area.editors {
            let open_path = if is_file_uri(open_uri) {
                file_uri_to_path(open_uri)
            } else {
                PathBuf::from(open_uri)
            };
            if paths_equivalent(&open_path, &target_path) {
                return Some(ed.text());
            }
        }
        None
    }
}

impl Default for E2eWindow {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds a `LanguageFeatureBridge` + `RpcBridge` wired with `ConfigurationService`,
/// `CommandRegistry`, `FileSystemService`, `ExtensionLoader`, and `fs_root`.
pub fn create_bridge_with_root(root: &Path) -> (Arc<LanguageFeatureBridge>, Arc<RpcBridge>) {
    let provider_registry = Arc::new(ProviderRegistry::new());
    let rpc_bridge = Arc::new(RpcBridge::new());
    let lsp_manager = Arc::new(LspClientManager::new());
    let config = ConfigurationService::new();
    let commands = CommandRegistry::new();
    let fs_service = Arc::new(FileSystemService::new());
    let loader = ExtensionLoader::new();

    let bridge = Arc::new(
        LanguageFeatureBridge::new(provider_registry, Arc::clone(&rpc_bridge), lsp_manager)
            .with_configuration_service(config)
            .with_command_registry(commands)
            .with_file_system_service(fs_service)
            .with_extension_loader(&loader)
            .with_fs_root(root),
    );
    (bridge, rpc_bridge)
}

/// Executes the `rustcode` CLI binary (from `target/debug/rustcode(.exe)` if present,
/// otherwise via `cargo run --quiet -p rustcode-app --bin rustcode`).
pub fn run_rustcode_cli() -> Output {
    let root = repo_root();
    let exe_name = if cfg!(windows) {
        "rustcode.exe"
    } else {
        "rustcode"
    };
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let bin_path = target_dir.join("debug").join(exe_name);

    if bin_path.is_file() {
        Command::new(&bin_path)
            .current_dir(&root)
            .output()
            .expect("failed to execute rustcode binary")
    } else {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        Command::new(cargo)
            .args(["run", "--quiet", "-p", "rustcode-app", "--bin", "rustcode"])
            .current_dir(&root)
            .output()
            .expect("failed to run rustcode via cargo")
    }
}

/// Runs a Node.js script (inline JS via `-e`, or a `.js` file path if it exists under `cwd`).
pub fn run_node_script(script: &str, cwd: &Path) -> Output {
    let mut cmd = Command::new("node");
    let is_script_file =
        !script.contains('\n') && script.ends_with(".js") && cwd.join(script).is_file();
    if is_script_file {
        cmd.arg(script);
    } else {
        cmd.arg("-e").arg(script);
    }
    cmd.current_dir(cwd)
        .output()
        .expect("failed to execute node script")
}
