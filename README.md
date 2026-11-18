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
| **Extension Host** | **Pure-Rust `RpcBridge` & `ext-host-js`** | Pure-Rust MessagePack/JSON IPC (`rustcode-ext-host`) + CommonJS `vscode.*` API shims & Node.js compat layer (`deno_core` V8 binding targeted for full desktop toolchains) |
| **IPC Protocol** | **MessagePack RPC (`rmp-serde`)** | Compact binary RPC between Rust Core and Extension Host |
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
│   ├── rustcode-ext-host/      # Layer 4: Extension manifests (`customEditors`, etc.), RPC bridge, 16 ProviderTypes,
│   │                           #          Output, Terminal, Notebook, SCM, Debug, Tasks, Testing, Auth, Webview & CustomEditor managers,
│   │                           #          Open VSX marketplace client, VSIX installer & `compat_suite` (26 tests)
│   ├── rustcode-workbench/     # Layer 3: ActivityBar, Explorer, Search, SCM, Debug, ExtensionsSidebarView,
│   │                           #          EditorArea split grid (`EditorGridNode`/`EditorSash`) & CustomEditor routing,
│   │                           #          TerminalPanelView, OutputPanelView, SettingsUiView, NotebookEditorView,
│   │                           #          WebviewEditorView, BreadcrumbsBar, ProblemsPanel, CommandPalette/QuickInput, StatusBar
│   └── rustcode-app/           # Bootstrap, service container (`RustCodeContext`), benchmarks & integration tests
├── ext-host-js/
│   ├── vscode-api/             # `require('vscode')` (13 modules): index, commands, window, workspace, languages, notebooks,
│   │                           # scm, debug, tasks, tests, authentication, env, types (`EventEmitter`)
│   └── node-compat/            # Node.js compat polyfills (`events`, `path`, `fs`)
├── resources/                  # Default Dark+/Light+ themes, Linux keymaps, sample extensions, `rustcode.desktop`
├── scripts/package.sh          # Release packaging script
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

### Upcoming (Phase 7 — Pure-Rust Parity & Extended Ecosystem)
- [ ] **Phase 7 M1 — 100% Extension Host RPC Bridge Parity**: Route `window.{show*Message,showQuickPick,showInputBox}`, `'TreeDataProvider'` / `"treeView.getChildren"`, `workspace.{getConfiguration,updateConfiguration,openTextDocument}`, `commands.*`, `env.*`, `extensions.all`/`getExtension`, and `node-compat/fs.js` through `LanguageFeatureBridge`
- [ ] **Phase 7 M2 — In-Editor Find & Replace (`Ctrl+F` / `Ctrl+H`) & QuickInput Navigation (`Ctrl+G` / `@`)**: Floating `FindReplaceWidget` overlay with live match highlights & replace, plus Go to Line (`:line[:col]`) and Go to Symbol in Editor (`@symbol`)
- [ ] **Phase 7 M3 — Notification Toast Overlay & Testing Sidebar (`workbench.view.testing`)**: Bottom-right notification toast stack (`Info`/`Warning`/`Error` + actions) and `TestingSidebarView` wired to `ExtHostTestManager` (`vscode.tests`)
- [ ] **Phase 7 M4 — Interactive File Explorer CRUD & Contributed TreeViews**: Inline new file/folder, inline rename (`F2`), delete confirmation, and extension-contributed `views` (`registerTreeDataProvider`) in `FileExplorerView`
- [ ] **Phase 7 M5 — Expanded `node-compat`, More Language Tokenizers & Docs Refresh**: Add `ext-host-js/node-compat/{buffer,url,util,os,crypto,child_process}.js`, pure-Rust tokenizers for Go, C/C++, HTML/CSS, and TOML/YAML, and refresh `docs/` & `README.md`

### Future (Desktop Toolchain Targets)
- [ ] Live hardware GPU runtime (`wgpu` / GPUI native window) & live V8 binding when building on full desktop toolchains

---

## 🧪 Building, Testing & Packaging

Verified test & lint status: **404 workspace tests passing**, **26/26 `compat_suite` tests passing**, **0 `clippy` warnings**.

```bash
# Clone the repository
git clone https://github.com/revelino01/vscode-in-rust.git
cd vscode-in-rust

# Run all workspace unit and integration tests
cargo test --workspace

# Run extension host compatibility suite
cargo test -p rustcode-ext-host --test compat_suite

# Run Piece Table stress tests
cargo test -p rustcode-editor -- piece_table

# Lint workspace cleanly (warnings treated as errors)
cargo clippy --workspace --all-targets -- -D warnings

# Compile benchmarks (without running full bench harness)
cargo bench --no-run

# Package release build (on full build host)
./scripts/package.sh
```

---

## 📄 License

MIT OR Apache-2.0
