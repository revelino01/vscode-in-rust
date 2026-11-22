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
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use uuid::Uuid;

/// Returns the repository root (`c:\...\vscode-in-rust` or `/.../vscode-in-rust`).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("tests/phase8-e2e must sit two levels below repo root")
        .to_path_buf()
}

/// RAII temporary directory workspace cleaned up automatically on `Drop`.
pub struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    pub fn new() -> Self {
        let dir_name = format!(
            "rustcode_phase8_e2e_{}_{}",
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

    pub fn mkdir_all(&self, rel: impl AsRef<Path>) -> PathBuf {
        let target = self.root.join(rel.as_ref());
        std::fs::create_dir_all(&target).expect("failed to create temp directory");
        target
    }

    pub fn write_file(&self, rel: impl AsRef<Path>, content: &str) -> PathBuf {
        let target = self.root.join(rel.as_ref());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent directories");
        }
        std::fs::write(&target, content).expect("failed to write temp workspace file");
        target
    }

    pub fn write_bytes(&self, rel: impl AsRef<Path>, bytes: &[u8]) -> PathBuf {
        let target = self.root.join(rel.as_ref());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent directories");
        }
        std::fs::write(&target, bytes).expect("failed to write temp workspace bytes");
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

/// Opaque-box Phase 8 test driver wrapping `RustCodeContext`, `WorkbenchWindow`,
/// `MockRenderSurface`, and viewport `Rect` bounds (default `1200.0 x 800.0`).
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
        let surface = MockRenderSurface::new(1200.0, 800.0);
        let bounds = Rect::from_xywh(0.0, 0.0, 1200.0, 800.0);
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

    pub fn right_click(&mut self, point: Point) -> EventResult {
        self.window.dispatch_event(
            &InputEvent::MouseDown(MouseEvent::new(point, MouseButton::Right)),
            self.bounds,
        )
    }

    pub fn render(&mut self) -> Scene {
        self.window.render(&mut self.surface)
    }

    pub fn accessibility_tree(&self) -> AccessibilityTree {
        self.window.accessibility_tree(self.bounds)
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

/// Executes the `rustcode` CLI binary with arbitrary arguments.
pub fn run_rustcode_cli_args(args: &[&str]) -> Output {
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
            .args(args)
            .current_dir(&root)
            .output()
            .expect("failed to execute rustcode binary")
    } else {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.args(["run", "--quiet", "-j", "2", "-p", "rustcode-app", "--bin", "rustcode", "--"]);
        cmd.args(args);
        cmd.current_dir(&root)
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

/// Initializes a real `git` repository on branch `main` with local user identity configured.
pub fn init_git_repo(dir: &Path) {
    let init = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir)
        .output();
    match init {
        Ok(out) if out.status.success() => {}
        _ => {
            let fallback = Command::new("git")
                .args(["init"])
                .current_dir(dir)
                .output()
                .expect("git init failed");
            assert!(
                fallback.status.success(),
                "git init failed: {}",
                String::from_utf8_lossy(&fallback.stderr)
            );
            let _ = Command::new("git")
                .args(["checkout", "-b", "main"])
                .current_dir(dir)
                .output();
        }
    }

    for args in [
        ["config", "user.name", "RustCode E2E"],
        ["config", "user.email", "e2e@rustcode.dev"],
    ] {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git config failed");
        assert!(
            out.status.success(),
            "git config failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Writes an unpacked VS Code extension bundle (`package.json` + arbitrary assets/JS files).
pub fn write_unpacked_extension(
    extensions_root: &Path,
    dir_name: &str,
    manifest: Value,
    files: &[(&str, &str)],
) -> PathBuf {
    let ext_dir = extensions_root.join(dir_name);
    std::fs::create_dir_all(&ext_dir).expect("failed to create extension dir");
    std::fs::write(
        ext_dir.join("package.json"),
        serde_json::to_string_pretty(&manifest).expect("manifest json"),
    )
    .expect("failed to write extension package.json");
    for (rel, content) in files {
        let target = ext_dir.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("failed to create extension subdir");
        }
        std::fs::write(&target, content).expect("failed to write extension file");
    }
    ext_dir
}

/// Writes a real Node.js stdio JSON-RPC LSP server script supporting `Content-Length` framing,
/// `initialize`, `didOpen`/`didChange` diagnostics (triggering on `"ERROR_TOKEN"`),
/// `hover`, `definition` (cross-file to `def_target_uri`), `completion`, and `formatting`.
pub fn write_mock_lsp_server(
    dir: &Path,
    filename: &str,
    def_target_uri: &str,
    formatted_text: &str,
) -> PathBuf {
    let script_path = dir.join(filename);
    let def_uri_json = serde_json::to_string(def_target_uri).unwrap();
    let fmt_text_json = serde_json::to_string(formatted_text).unwrap();

    let script = format!(
        r#"'use strict';
const DEF_TARGET_URI = {def_uri_json};
const FORMATTED_TEXT = {fmt_text_json};
const docs = new Map();
let buf = Buffer.alloc(0);

function send(msg) {{
  const body = Buffer.from(JSON.stringify(msg), 'utf8');
  const header = Buffer.from('Content-Length: ' + body.length + '\r\n\r\n', 'ascii');
  process.stdout.write(Buffer.concat([header, body]));
}}

function publishDiags(uri, text) {{
  const diagnostics = [];
  const lines = text.split(/\r?\n/);
  for (let lineIdx = 0; lineIdx < lines.length; lineIdx++) {{
    const line = lines[lineIdx];
    const byteIdx = line.indexOf('ERROR_TOKEN');
    if (byteIdx !== -1) {{
      // Compute UTF-16 code unit offset for LSP 3.17 parity
      const utf16Start = line.slice(0, byteIdx).length;
      const utf16End = utf16Start + 'ERROR_TOKEN'.length;
      diagnostics.push({{
        range: {{
          start: {{ line: lineIdx, character: utf16Start }},
          end: {{ line: lineIdx, character: utf16End }}
        }},
        severity: 1,
        source: 'rust-analyzer-mock',
        message: 'unresolved ERROR_TOKEN'
      }});
    }}
  }}
  send({{
    jsonrpc: '2.0',
    method: 'textDocument/publishDiagnostics',
    params: {{ uri, diagnostics }}
  }});
}}

function handleMessage(msg) {{
  const method = msg.method;
  const id = msg.id;
  const params = msg.params || {{}};

  if (method === 'initialize') {{
    send({{
      jsonrpc: '2.0',
      id,
      result: {{
        capabilities: {{
          textDocumentSync: 1,
          hoverProvider: true,
          definitionProvider: true,
          completionProvider: {{ triggerCharacters: ['.', ':'] }},
          documentFormattingProvider: true,
          positionEncoding: 'utf-16'
        }}
      }}
    }});
    return;
  }}

  if (method === 'textDocument/didOpen') {{
    const td = params.textDocument || {{}};
    docs.set(td.uri, td.text || '');
    publishDiags(td.uri, td.text || '');
    return;
  }}

  if (method === 'textDocument/didChange') {{
    const td = params.textDocument || {{}};
    const changes = params.contentChanges || [];
    if (changes.length > 0 && typeof changes[changes.length - 1].text === 'string') {{
      const next = changes[changes.length - 1].text;
      docs.set(td.uri, next);
      publishDiags(td.uri, next);
    }}
    return;
  }}

  if (method === 'textDocument/hover') {{
    const pos = params.position || {{ line: 0, character: 0 }};
    send({{
      jsonrpc: '2.0',
      id,
      result: {{
        contents: {{
          kind: 'markdown',
          value: '```rust\npub fn target_symbol() // at ' + pos.line + ':' + pos.character + '\n```'
        }}
      }}
    }});
    return;
  }}

  if (method === 'textDocument/definition') {{
    send({{
      jsonrpc: '2.0',
      id,
      result: {{
        uri: DEF_TARGET_URI,
        range: {{
          start: {{ line: 1, character: 7 }},
          end: {{ line: 1, character: 20 }}
        }}
      }}
    }});
    return;
  }}

  if (method === 'textDocument/completion') {{
    send({{
      jsonrpc: '2.0',
      id,
      result: [
        {{ label: 'target_symbol', kind: 3, detail: 'pub fn target_symbol()' }},
        {{ label: 'lsp_helper_fn', kind: 3, detail: 'pub fn lsp_helper_fn()' }}
      ]
    }});
    return;
  }}

  if (method === 'textDocument/formatting') {{
    const uri = params.textDocument && params.textDocument.uri;
    const current = (uri && docs.get(uri)) || '';
    const lines = current.split('\n');
    const endLine = Math.max(0, lines.length - 1);
    const endChar = lines[endLine] ? lines[endLine].length : 0;
    send({{
      jsonrpc: '2.0',
      id,
      result: [
        {{
          range: {{
            start: {{ line: 0, character: 0 }},
            end: {{ line: endLine, character: endChar }}
          }},
          newText: FORMATTED_TEXT
        }}
      ]
    }});
    return;
  }}

  if (method === 'shutdown') {{
    send({{ jsonrpc: '2.0', id, result: null }});
    return;
  }}

  if (method === 'exit') {{
    process.exit(0);
  }}

  if (id !== undefined) {{
    send({{ jsonrpc: '2.0', id, result: null }});
  }}
}}

process.stdin.on('data', (chunk) => {{
  buf = Buffer.concat([buf, chunk]);
  while (true) {{
    const headerEnd = buf.indexOf('\r\n\r\n');
    if (headerEnd === -1) break;
    const header = buf.slice(0, headerEnd).toString('ascii');
    const m = /content-length:\s*(\d+)/i.exec(header);
    if (!m) {{
      buf = buf.slice(headerEnd + 4);
      continue;
    }}
    const len = parseInt(m[1], 10);
    const bodyStart = headerEnd + 4;
    if (buf.length < bodyStart + len) break;
    const body = buf.slice(bodyStart, bodyStart + len).toString('utf8');
    buf = buf.slice(bodyStart + len);
    try {{
      handleMessage(JSON.parse(body));
    }} catch (_) {{}}
  }}
}});
"#
    );

    std::fs::write(&script_path, script).expect("failed to write mock LSP server");
    script_path
}

/// Writes a real Node.js stdio DAP adapter script supporting `Content-Length` framing,
/// `initialize`, `launch`, `setBreakpoints`, `configurationDone` -> `stopped`,
/// `threads`, `stackTrace`, `scopes`, `variables`, `evaluate`, `next`, `continue`, `disconnect`.
pub fn write_mock_dap_adapter(dir: &Path, filename: &str, source_path: &Path) -> PathBuf {
    let script_path = dir.join(filename);
    let source_path_json =
        serde_json::to_string(&source_path.to_string_lossy().into_owned()).unwrap();
    let source_name_json = serde_json::to_string(
        &source_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "main.rs".to_string()),
    )
    .unwrap();

    let script = format!(
        r#"'use strict';
const SOURCE_PATH = {source_path_json};
const SOURCE_NAME = {source_name_json};
let seq = 1;
let counterVal = 42;
let currentLine = 2;
let buf = Buffer.alloc(0);

function send(msg) {{
  const payload = Object.assign({{ seq: seq++ }}, msg);
  const body = Buffer.from(JSON.stringify(payload), 'utf8');
  const header = Buffer.from('Content-Length: ' + body.length + '\r\n\r\n', 'ascii');
  process.stdout.write(Buffer.concat([header, body]));
}}

function sendResp(req, body) {{
  send({{
    type: 'response',
    request_seq: req.seq,
    success: true,
    command: req.command,
    body: body || {{}}
  }});
}}

function handleRequest(req) {{
  const cmd = req.command;
  if (cmd === 'initialize') {{
    sendResp(req, {{
      supportsConfigurationDoneRequest: true,
      supportsEvaluateForHovers: true
    }});
    send({{ type: 'event', event: 'initialized' }});
    return;
  }}
  if (cmd === 'setBreakpoints') {{
    const bps = (req.arguments && req.arguments.breakpoints) || [];
    sendResp(req, {{
      breakpoints: bps.map((bp, i) => ({{ id: i + 1, verified: true, line: bp.line }}))
    }});
    return;
  }}
  if (cmd === 'configurationDone') {{
    sendResp(req, {{}});
    send({{
      type: 'event',
      event: 'stopped',
      body: {{ reason: 'breakpoint', threadId: 1 }}
    }});
    return;
  }}
  if (cmd === 'threads') {{
    sendResp(req, {{ threads: [{{ id: 1, name: 'main' }}] }});
    return;
  }}
  if (cmd === 'stackTrace') {{
    sendResp(req, {{
      stackFrames: [{{
        id: 100,
        name: 'main',
        source: {{ name: SOURCE_NAME, path: SOURCE_PATH }},
        line: currentLine,
        column: 5
      }}],
      totalFrames: 1
    }});
    return;
  }}
  if (cmd === 'scopes') {{
    sendResp(req, {{
      scopes: [{{ name: 'Locals', variablesReference: 200, expensive: false }}]
    }});
    return;
  }}
  if (cmd === 'variables') {{
    sendResp(req, {{
      variables: [{{
        name: 'counter',
        value: String(counterVal),
        type: 'i32',
        variablesReference: 0
      }}]
    }});
    return;
  }}
  if (cmd === 'evaluate') {{
    const expr = (req.arguments && req.arguments.expression) || '';
    const result = expr === 'counter + 1' ? String(counterVal + 1) : String(counterVal);
    sendResp(req, {{ result, type: 'i32', variablesReference: 0 }});
    return;
  }}
  if (cmd === 'next' || cmd === 'stepIn' || cmd === 'stepOut') {{
    counterVal += 1;
    currentLine += 1;
    sendResp(req, {{}});
    send({{
      type: 'event',
      event: 'stopped',
      body: {{ reason: 'step', threadId: 1 }}
    }});
    return;
  }}
  if (cmd === 'continue') {{
    sendResp(req, {{ allThreadsContinued: true }});
    send({{ type: 'event', event: 'terminated' }});
    return;
  }}
  if (cmd === 'disconnect') {{
    sendResp(req, {{}});
    setTimeout(() => process.exit(0), 20);
    return;
  }}
  sendResp(req, {{}});
}}

process.stdin.on('data', (chunk) => {{
  buf = Buffer.concat([buf, chunk]);
  while (true) {{
    const headerEnd = buf.indexOf('\r\n\r\n');
    if (headerEnd === -1) break;
    const header = buf.slice(0, headerEnd).toString('ascii');
    const m = /content-length:\s*(\d+)/i.exec(header);
    if (!m) {{
      buf = buf.slice(headerEnd + 4);
      continue;
    }}
    const len = parseInt(m[1], 10);
    const bodyStart = headerEnd + 4;
    if (buf.length < bodyStart + len) break;
    const body = buf.slice(bodyStart, bodyStart + len).toString('utf8');
    buf = buf.slice(bodyStart + len);
    try {{
      handleRequest(JSON.parse(body));
    }} catch (_) {{}}
  }}
}});
"#
    );

    std::fs::write(&script_path, script).expect("failed to write mock DAP adapter");
    script_path
}
