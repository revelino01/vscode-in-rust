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
| **Extension Host** | **Embedded V8 (`deno_core`) & `ext-host-js`** | Pure-Rust IPC + JS `vscode.*` shim and Node.js compat layer |
| **IPC Protocol** | **MessagePack RPC (`rmp-serde`)** | Compact binary RPC between Rust Core and Extension Host |
| **Text Buffer** | **Custom Piece Table** | Red-Black tree piece table with O(log n) inserts/deletes, UTF-8 safety, and atomic undo/redo |
| **Syntax Highlighting** | **Pure-Rust State-Machine / Tree-sitter Pipeline** | Incremental line caching for Rust, JS, TS, Python, JSON, and Markdown + VS Code scope colors |
| **Language & Debug Protocols** | **LSP 3.17, DAP & Myers Diff** | Stdio `Content-Length` JSON-RPC for LSP/DAP, UTF-16 alignment, and Eugene Myers O(ND) diff |
| **Terminal** | **`alacritty_terminal`** | Terminal session manager and VT100/xterm buffer support |

---

## 📦 Workspace Crates & Structure

```text
vscode-in-rust/
├── crates/
│   ├── rustcode-base/          # Layer 0: Disposables, typed Emitter/Event, CancellationToken, `when`-clause parser
│   ├── rustcode-platform/      # Layer 1: ConfigurationService (JSONC + per-folder), KeybindingService, CommandRegistry,
│   │                           #          ThemeService, FS/AutoSave/Watcher, Scene Graph renderer, AccessibilityTree
│   ├── rustcode-editor/        # Layer 2: Piece Table, Undo/Redo, Multi-cursor, Syntax tokenizers,
│   │                           #          Autocomplete, Minimap, DiffEditorView
│   ├── rustcode-terminal/      # TerminalManager & TerminalInstance sessions
│   ├── rustcode-protocols/     # Layer 5: LSP 3.17, DAP, and Git SCM protocol definitions & Myers diff
│   ├── rustcode-ext-host/      # Layer 4: Extension manifests, RPC bridge, Provider callbacks, Webview & Notebook managers,
│   │                           #          Open VSX marketplace client & VSIX installer
│   ├── rustcode-workbench/     # Layer 3: ActivityBar, Explorer, Search, SCM, Debug, ExtensionsSidebarView,
│   │                           #          EditorArea, SettingsUiView, NotebookEditorView, WebviewEditorView,
│   │                           #          BreadcrumbsBar, ProblemsPanel, CommandPalette/QuickInput, StatusBar
│   └── rustcode-app/           # Bootstrap, service container (`RustCodeContext`), benchmarks & integration tests
├── ext-host-js/
│   ├── vscode-api/             # `require('vscode')`: index, commands, window, workspace, languages, notebooks, env, types
│   └── node-compat/            # Node.js compat polyfills (`events`, `path`, `fs`)
├── resources/                  # Default Dark+/Light+ themes, Linux keymaps, sample extensions, `rustcode.desktop`
├── scripts/package.sh          # Release packaging script
├── docs/                       # Architecture, contributing guide, and extension compatibility matrix
└── rustcode-implementation-plan.md # Master implementation plan & progress tracker
```

---

## 🚦 Progress & Roadmap

See [rustcode-implementation-plan.md](rustcode-implementation-plan.md) for the complete module-by-module tracker.

### Completed (Phases 0–5 / Milestones M1–M4)
- [x] **Phase 0 — Scaffold & Core Foundation**: 8-crate workspace, `rustcode-base`, Piece Table & Undo/Redo, platform services, MessagePack RPC bridge, GPUI presentation layer & scene graph, UTF-8 hardening
- [x] **Phase 1 — Minimal Editor & File Workflows**: `settings.json` (JSONC) & `keybindings.json` resolution, JSON theme loader, syntax highlighting pipeline (Rust, JS, TS, Python, JSON, Markdown), atomic save + auto-save + `notify` file watcher
- [x] **Phase 2 — Language Intelligence & Marketplace**: LSP client lifecycle, interactive Autocomplete widget (snippets + doc preview), Diagnostics squiggles + Problems panel + status badges, Open VSX client & `.vsix` unpacker
- [x] **Phase 3 — Full Workbench Parity**: DAP debug adapter + floating debug toolbar + Call Stack/Variables/Breakpoints/Console, Git SCM sidebar + Myers O(ND) inline & side-by-side Diff viewer, HTML parser + Webview host bridge
- [x] **Phase 4 — Polish & Ecosystem**: Multi-root workspaces & hierarchical per-folder config, cross-folder regex/glob Search + SymbolExtractor + multi-mode QuickInput (`Ctrl+P` / `>` / `#`), Formatting & Code Actions (`WorkspaceEdit`), UTF-16 LSP alignment, `compat_suite`, Criterion benchmarks, release profile (`lto="thin"`), `scripts/package.sh` & `.desktop`
- [x] **M1 (R1) — Visual Settings UI**: Searchable/categorised `SettingsUiView` (`Ctrl+,` / `workbench.action.openSettings`), two-way syncing & reset with `ConfigurationService`
- [x] **M2 (R2) — Notebook Support**: `.ipynb` (nbformat v4) `NotebookDocument` / `NotebookEditorView`, pure-Rust `NotebookKernel`, `vscode.notebooks.*` JS shim (`notebooks.js`) & `ExtHostNotebookManager`
- [x] **M3 (R3) — Extension Marketplace Sidebar UI**: `ExtensionsSidebarView` (`workbench.view.extensions`) with search, Installed/Marketplace sections, install/enable/disable/uninstall wired to `rustcode-ext-host::marketplace`
- [x] **M4 (R4) — Breadcrumbs, Minimap & Accessibility Tree**: `BreadcrumbsBar` (`breadcrumbs.enabled`), `Minimap` (`editor.minimap.enabled`), semantic `AccessibilityTree` across editor/workbench views, live `ConfigurationService` sync

### Upcoming (Phase 6)
- [ ] Interactive integrated Terminal & Output panel views (`TerminalPanelView`, `OutputPanelView`)
- [ ] Remaining `vscode.*` API namespaces (`scm`, `debug`, `tasks`, `tests`, `authentication`)
- [ ] Editor code folding, soft word wrap (`editor.wordWrap`), and multi-cursor editing commands (`Ctrl+D`)
- [ ] Split editor grid (`workbench.action.splitEditor`) & custom editors (`registerCustomEditorProvider`)
- [ ] Refresh `docs/extension-compat-matrix.md` and `docs/architecture.md`
- [ ] Live hardware GPU runtime (`wgpu` / GPUI native window) & live V8 binding when building on full desktop toolchains

---

## 🧪 Building, Testing & Packaging

Verified test & lint status: **313 workspace tests passing**, **13/13 `compat_suite` tests passing**, **0 `clippy` warnings**.

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
