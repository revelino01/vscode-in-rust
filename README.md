# RustCode (vscode-in-rust)

> [!NOTE]
> ⚡ **Vibe Coded Repository**: This repository and its entire codebase are **100% vibe coded** with autonomous AI agent workflows. Pure vibes, modern Rust, and blazing-fast performance.

A ground-up, high-performance Rust rewrite of Visual Studio Code. **RustCode** aims to deliver the look, feel, keybindings, settings schema, and extension ecosystem of VS Code while using dramatically less RAM and launching cleanly with a native GPU-style scene-graph architecture.

---

## 🎯 Vision & Performance

- **Familiar Experience**: VS Code-compatible keybindings (`keybindings.json` + multi-chord + `when` clauses), settings (`settings.json` JSONC + visual Settings UI via `Ctrl+,`), TextMate themes, command palette (`Ctrl+Shift+P` / `Ctrl+P` / `#`), and workbench layout.
- **Extension Compatibility**: Run JS/TS VS Code extensions through an embedded V8 / `deno_core`-oriented extension host architecture, Node.js compatibility polyfills, MessagePack RPC, and `vscode.*` API shims.
- **Lightweight & Fast** (measured in workspace Criterion benchmarks alongside target budgets):

| Metric | Electron VS Code | RustCode Target | Measured Benchmark Baseline |
|---|---|---|---|
| **Cold Start** | 2–5 s | **< 500 ms** | **~134 µs** (`startup_time.rs`) |
| **Idle RAM (empty window)** | 300–400 MB | **< 120 MB** | **~5 MB RSS** (`memory_footprint.rs`) |
| **Keystroke Latency** | 5–20 ms | **< 3 ms** | **~425 µs** (`editor_latency.rs`) |
| **Frame Rate Target** | 60 FPS | **120 FPS** | GPU scene-graph pipeline |

---

## 🏗️ Architecture

| Component | Choice | Description |
|---|---|---|
| **GUI / Presentation** | **GPUI** / Scene Graph (`rustcode-platform::render`) | Hardware-accelerated display lists, mockable render surface, zero Chromium DOM |
| **Extension Host** | **Pure-Rust `RpcBridge`, `NodeExtHostProcess` & `ext-host-js`** | Live framed stdio (`Content-Length` JSON/MsgPack & newline-JSON) Rust ↔ Node subprocess transport (`NodeExtHostProcess` + `ext-host-js/rpc-bridge.js`) with structured `RpcError` propagation, `vscode.*` API shims, and `node-compat` polyfills |
| **IPC Protocol** | **JSON & MessagePack RPC (`rmp-serde`)** | Framed `Content-Length` (JSON/MsgPack) and newline-delimited JSON-RPC between Rust Core and Extension Host |
| **Text Buffer** | **Custom Piece Table** | Red-Black tree piece table with O(log n) inserts/deletes, UTF-8 safety, and atomic undo/redo |
| **Syntax Highlighting** | **Pure-Rust State-Machine / Tree-sitter Pipeline** | Incremental line caching for Rust, JS, TS, Python, JSON, and Markdown + VS Code scope colors |
| **Language & Debug Protocols** | **LSP 3.17, DAP & Myers Diff** | Stdio `Content-Length` JSON-RPC for LSP/DAP, UTF-16 alignment, and Eugene Myers O(ND) diff |
| **Terminal** | **Pure-Rust `rustcode-terminal`** | `TerminalManager` / `TerminalInstance` multi-tab & split sessions, command history, and pure-Rust `std::process::Command` execution wired to `TerminalPanelView` |

---

## 📦 Workspace Crates & Structure

```text
vscode-in-rust/
├── crates/
│   ├── rustcode-base/          # Layer 0: Disposables, typed Emitter/Event, CancellationToken, `when`-clause parser
│   ├── rustcode-platform/      # Layer 1: ConfigurationService (JSONC + per-folder), KeybindingService, CommandRegistry,
│   │                           #          ThemeService, FS/AutoSave/Watcher, Scene Graph renderer, AccessibilityTree
│   ├── rustcode-editor/        # Layer 2: Piece Table, Undo/Redo, Multi-cursor editing & commands (`Ctrl+D`, `Ctrl+Alt+Up/Down`),
│   │                           #          FoldingModel (code folding), Soft Word-Wrap (`WordWrapMode`), Syntax tokenizers,
│   │                           #          Autocomplete, Minimap, DiffEditorView
│   ├── rustcode-terminal/      # TerminalManager & TerminalInstance split/tab sessions, command history & pure-Rust execution
│   ├── rustcode-protocols/     # Layer 5: LSP 3.17, DAP, and Git SCM protocol definitions & Myers diff
│   ├── rustcode-ext-host/      # Layer 4: Extension manifests (`customEditors`, etc.), `RpcBridge` & `NodeExtHostProcess`,
│   │                           #          structured `RpcError`, 16 `ProviderType`s, 14 managers (Diagnostic, StatusBar, Notebook,
│   │                           #          Output, Terminal, SCM, Debug, Task, Test, Auth, Webview, CustomEditor, Window, Env),
│   │                           #          Open VSX marketplace client, VSIX installer & `compat_suite` (41 tests)
│   ├── rustcode-workbench/     # Layer 3: ActivityBar, Explorer, Search, SCM, Debug, ExtensionsSidebarView,
│   │                           #          EditorArea split grid (`EditorGridNode`/`EditorSash`) & CustomEditor routing,
│   │                           #          TerminalPanelView, OutputPanelView, SettingsUiView, NotebookEditorView,
│   │                           #          WebviewEditorView, BreadcrumbsBar, ProblemsPanel, CommandPalette/QuickInput, StatusBar
│   └── rustcode-app/           # Bootstrap, service container (`RustCodeContext`), benchmarks & integration tests (25 tests)
├── ext-host-js/
│   ├── rpc-bridge.js           # Framed stdio transport (`Content-Length` / newline-JSON), `globalThis` singleton, `require('vscode')` hook
│   ├── vscode-api/             # `require('vscode')` (13 modules): index, commands, window, workspace, languages, notebooks,
│   │                           # scm, debug, tasks, tests, authentication, env, types (`EventEmitter`, `Uri`, `RelativePattern`, etc.)
│   ├── node-compat/            # Node.js compat polyfills (`events`, OS-aware `path` `win32`/`posix`, `fs` with `Dirent`/encodings)
│   └── test.js                 # JavaScript extension host suite (25 `[Test]` sections / 271 `assert(...)` call sites)
├── resources/                  # Default Dark+/Light+ themes, Linux keymaps, 1 sample extension (`hello-world`), `rustcode.desktop`
├── scripts/package.sh          # Release packaging script
├── tests/phase7-e2e/           # Standalone opaque-box E2E harness (Tiers 1–4, excluded from root Cargo `[workspace]`)
├── docs/                       # Architecture, contributing guide, and extension compatibility matrix
└── rustcode-implementation-plan.md # Master implementation plan & progress tracker
```

---

## 🚦 Progress & Roadmap

See [rustcode-implementation-plan.md](rustcode-implementation-plan.md) for the complete module-by-module tracker.

### Completed (Phases 0–6 / Milestones M1–M5)
- [x] **Phase 0 — Scaffold & Core Foundation**: 8-crate workspace, `rustcode-base`, Piece Table & Undo/Redo, platform services, MessagePack RPC bridge, GPUI presentation layer & scene graph, UTF-8 hardening
- [x] **Phase 1 — Minimal Editor & File Workflows**: `settings.json` (JSONC) & `keybindings.json` resolution, JSON theme loader, syntax highlighting pipeline (Rust, JS, TS, Python, JSON, Markdown), atomic save + auto-save + `notify` file watcher
- [x] **Phase 2 — Language Intelligence & Marketplace**: LSP client lifecycle, interactive Autocomplete widget (snippets + doc preview), Diagnostics squiggles + Problems panel + status badges, Open VSX client & `.vsix` unpacker
- [x] **Phase 3 — Full Workbench Parity**: DAP debug adapter + floating debug toolbar + Call Stack/Variables/Breakpoints/Console, Git SCM sidebar + Myers O(ND) inline & side-by-side Diff viewer, HTML parser + Webview host bridge
- [x] **Phase 4 — Polish & Ecosystem**: Multi-root workspaces & hierarchical per-folder config, cross-folder regex/glob Search + SymbolExtractor + multi-mode QuickInput (`Ctrl+P` / `>` / `#`), Formatting & Code Actions (`WorkspaceEdit`), UTF-16 LSP alignment, `compat_suite`, Criterion benchmarks, release profile (`lto="thin"`), `scripts/package.sh` & `.desktop`
- [x] **M1 (R1) — Visual Settings UI**: Searchable/categorised `SettingsUiView` (`Ctrl+,` / `workbench.action.openSettings`), two-way syncing & reset with `ConfigurationService`
- [x] **M2 (R2) — Notebook Support**: `.ipynb` (nbformat v4) `NotebookDocument` / `NotebookEditorView`, pure-Rust `NotebookKernel`, `vscode.notebooks.*` JS shim (`notebooks.js`) & `ExtHostNotebookManager`
- [x] **M3 (R3) — Extension Marketplace Sidebar UI**: `ExtensionsSidebarView` (`workbench.view.extensions`) with search, Installed/Marketplace sections, install/enable/disable/uninstall wired to `rustcode-ext-host::marketplace`
- [x] **M4 (R4) — Breadcrumbs, Minimap & Accessibility Tree**: `BreadcrumbsBar` (`breadcrumbs.enabled`), `Minimap` (`editor.minimap.enabled`), semantic `AccessibilityTree` across editor/workbench views, live `ConfigurationService` sync
- [x] **Phase 6 M1 — Interactive Terminal & Output Panels**: Interactive `TerminalPanelView` & `OutputPanelView`, `rustcode-terminal` split/tab & pure-Rust process execution, `ExtHostOutputManager` & `vscode.window.createOutputChannel`
- [x] **Phase 6 M2 — Full `vscode.*` Extension Host APIs**: `vscode.EventEmitter`, `scm`, `debug`, `tasks`, `tests`, `authentication` JS shims + `ExtHost{Scm,Debug,Task,Test,Auth}Manager` RPC wiring
- [x] **Phase 6 M3 — Code Folding, Word-Wrap & Multi-Cursor Commands**: `FoldingModel` & gutter fold chevrons, soft word-wrap (`editor.wordWrap`, `editor.wordWrapColumn`) & `VisualLayout`, multi-cursor editing & commands (`Ctrl+D`, `Ctrl+Alt+Up/Down`, `Escape`)
- [x] **Phase 6 M4 — Split Editor Grid & Custom Editors**: Recursive `EditorGridNode` & `EditorSash` split grid (`workbench.action.splitEditor`), `vscode.window.registerCustomEditorProvider`, `ExtHostCustomEditorManager`, `CustomEditorContribution`, and `WebviewEditorView` dispatch
- [x] **Phase 6 M5 — Documentation & Compatibility Matrix Refresh**: Refreshed `docs/extension-compat-matrix.md`, `docs/architecture.md`, `README.md`, and `rustcode-implementation-plan.md`

### In Progress (Phase 7 — Pure-Rust Parity & Extended Ecosystem)
- [x] **Phase 7 M1.1 — Live Framed Stdio RPC Transport, Structured Error Envelope & `node-compat/{fs,path}.js` Parity**: `NodeExtHostProcess` (`ContentLengthJson`, `ContentLengthMsgPack`, `NewlineJson`, `from_io`, `JoinSet` half-close EOF draining, Windows `CREATE_NO_WINDOW`), O(N) `read_content_length_frame` (`64 KiB` header / `64 MiB` frame limits), structured `{ error: { code, message } }` (`RpcError` / `RpcErrorPayload`) across 12+ namespaces via `handle_rpc_message_result`, `node-compat/fs.js` (`Dirent`, encodings, `mkdir` `firstCreated`, `promises.{access,rm,rename,copyFile}`, `constants`, `invokeCallback`, 11 `*Sync` methods throwing `ENOSYS` in live stdio except `existsSync`), OS-aware `node-compat/path.js` (`win32`/`posix`, `relative`, `parse`, `format`, UNC/drive-letter parity), and Windows/Linux `map_io_error` + `EISDIR` parity
- [x] **Phase 7 M1.2 — Extension Host VS Code API Parity & Live Fixture E2E**: Parity across `commands`, `window`, `workspace`, `env`, `extensions`; sequential FIFO incoming notification worker + `IncomingNotifWork::Barrier` causal fence; `reader_closed` fast-fail; `command_mutation_lock` + token-checked `CommandRegistry` disposal; Windows folder-config equivalence & `extract_scope_path` `Uri` normalization + `affectsConfiguration(section, scope)`; CRLF/UTF-16 `TextDocumentProxy`; pluggable `ClipboardBackend` (`OsClipboardBackend`/`InMemoryClipboardBackend`) & `ExternalUriOpener` (`OsExternalUriOpener`/`RecordingExternalUriOpener`); `TreeDataProvider`/`TreeData` synthetic handles & `onDidChangeTreeData`; `require('vscode')` resolution (`installVscodeModuleHook` + `globalThis` singleton); live fixture (`hello-world`, `treeview_ext`, `theme_ext`) & multi-threaded 4-worker stdio reverse-split stress tests
- [ ] **Phase 7 M1.3 — `RustCodeContext` & `WorkbenchWindow` End-to-End Wiring (Deferred / Next)**: Construct/attach shared `LanguageFeatureBridge` + `FileSystemService` in `RustCodeContext::create_window()`, replace dummy builtin command JSON stubs with real `WorkbenchWindow` command effects, forward document open/change/save/close & active-editor events, sync `ExtHostWindowManager` / `ExtHostEnvManager` / `ExtStatusBarManager`, and add Windows Explorer drive-root fallback
- [ ] **Phase 7 M2 — In-Editor Find & Replace (`Ctrl+F` / `Ctrl+H`) & QuickInput Navigation (`Ctrl+G` / `@`) (Not Started)**: Floating `FindReplaceWidget` overlay with live match highlights & replace, plus Go to Line (`:line[:col]`) and Go to Symbol in Editor (`@symbol`)
- [ ] **Phase 7 M3 — Notification Toast Overlay & Testing Sidebar (`workbench.view.testing`) (Not Started)**: Bottom-right `NotificationsOverlay` toast stack (`Info`/`Warning`/`Error` + actions) + interactive QuickPick/InputBox sync, and `TestingSidebarView` wired to `ExtHostTestManager` (`vscode.tests`)
- [ ] **Phase 7 M4 — Interactive File Explorer CRUD & Contributed TreeViews (Not Started)**: Inline new file/folder, inline rename (`F2`), delete confirmation, and extension-contributed `views` (`registerTreeDataProvider`) in `FileExplorerView`
- [ ] **Phase 7 M5 — Expanded `node-compat` & Additional Language Tokenizers (Not Started)**: Add `ext-host-js/node-compat/{buffer,url,util,os,crypto,child_process}.js` + `env.getOsInfo` / `process.*` RPC, and pure-Rust state-machine tokenizers for 7 additional languages (`Go`, `C`, `C++`, `HTML`, `CSS`, `TOML`, `YAML`)
- [~] **Phase 7 M6 — Documentation & Trackers Wrap-Up**: M1.1 + M1.2 wrap-up complete across `README.md`, `rustcode-implementation-plan.md`, `docs/extension-compat-matrix.md`, and `docs/architecture.md`; final refresh follows M5
- [ ] **Phase 7 M7 — Standalone E2E Suite Pass (`tests/phase7-e2e/`) & Tier 5 Hardening (Not Started)**: Pass all standalone Tiers 1–4 tests (`286` Win / `287` Unix tests via `node tests/phase7-e2e/runner.js`, excluded from root Cargo `[workspace]`) + Tier 5 adversarial hardening

### Future (Desktop Toolchain Targets)
- [ ] Live hardware GPU runtime (`wgpu` / GPUI native window) & live V8 binding when building on full desktop toolchains

---

## 🧪 Building, Testing & Packaging

Verified test & lint status (Phase 7 M1.2): **457 workspace tests passing** across **8 crates** (**41/41 `compat_suite` tests passing**), **25/25 `ext-host-js/test.js` sections passing** (**271 `assert(...)` call sites**), and **0 `clippy` warnings** (`-D warnings`) on both **Windows (`x86_64-pc-windows-gnu`)** and **Linux (`x86_64-unknown-linux-gnu`)** cross-target clippy *(runtime tests executed on Windows 11 x64; Linux verified via cross-target `cargo clippy`)*.

```bash
# Clone the repository
git clone https://github.com/revelino01/vscode-in-rust.git
cd vscode-in-rust

# Run all workspace unit and integration tests (use -j 2 on constrained Windows hosts)
cargo test -j 2 --workspace

# Run extension host compatibility suite (41 tests)
cargo test -j 2 -p rustcode-ext-host --test compat_suite

# Run JavaScript extension host suite (25 [Test] sections / 271 asserts)
node ext-host-js/test.js

# Run Piece Table stress tests
cargo test -j 2 -p rustcode-editor -- piece_table

# Lint workspace cleanly on host and Linux cross-target (warnings treated as errors)
cargo clippy -j 2 --workspace --all-targets -- -D warnings
cargo clippy -j 2 --target x86_64-unknown-linux-gnu --workspace --all-targets -- -D warnings

# Compile benchmarks (without running full bench harness)
cargo bench --no-run

# Package release build (on full build host)
./scripts/package.sh
```

> **Standalone E2E Suite (`tests/phase7-e2e/`):** `tests/phase7-e2e/` is a standalone Cargo `[workspace]` outside root `Cargo.toml` containing **286** Windows / **287** Unix opaque-box E2E tests across Tiers 1–4 (`node tests/phase7-e2e/runner.js`; `--syntax-check` passes **8/8**). Full runtime E2E pass is deferred until remaining Phase 7 milestones (**M1.3**, **M2–M5**) are implemented.

---

## 📄 License

MIT OR Apache-2.0
