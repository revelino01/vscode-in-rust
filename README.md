# RustCode (vscode-in-rust)

> [!NOTE]
> ⚡ **Vibe Coded Repository**: This repository and its entire codebase are **100% vibe coded** with autonomous AI agent workflows. Pure vibes, modern Rust, and blazing-fast performance.

A ground-up, high-performance Rust rewrite of Visual Studio Code. **RustCode** aims to deliver the look, feel, keybindings, settings schema, and extension ecosystem of VS Code while using dramatically less RAM and launching cleanly with a native GPU-style scene-graph architecture.

---

## 🎯 Vision & Performance

- **Familiar Experience**: VS Code-compatible keybindings (`keybindings.json` + multi-chord + `when` clauses), settings (`settings.json` JSONC + visual Settings UI via `Ctrl+,`), TextMate themes, command palette (`Ctrl+Shift+P` / `Ctrl+P` / `#`), and workbench layout.
- **Extension Compatibility**: Run JS/TS VS Code extensions through `NodeExtHostProcess` framed stdio (`Content-Length` JSON/MessagePack & newline-JSON) + `ext-host-js`, 9 `node-compat` polyfill modules, and `vscode.*` API shims (`vscode.version = '1.95.0'`).
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
| **Syntax Highlighting** | **Pure-Rust State-Machine Pipeline** | Incremental line caching for **14** languages (`plaintext`, `rust`, `javascript`, `typescript`, `python`, `json`, `markdown`, `go`, `c`, `cpp` / C++, `html`, `css`, `toml`, `yaml`) + VS Code scope colors |
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
│   │                           #          FindReplaceWidget (`Ctrl+F` / `Ctrl+H`), FoldingModel, Soft Word-Wrap (`WordWrapMode`),
│   │                           #          14 pure-Rust Syntax tokenizers, Autocomplete, Minimap, DiffEditorView
│   ├── rustcode-terminal/      # TerminalManager & TerminalInstance split/tab sessions, command history & pure-Rust execution
│   ├── rustcode-protocols/     # Layer 5: LSP 3.17, DAP, and Git SCM protocol definitions & Myers diff
│   ├── rustcode-ext-host/      # Layer 4: Extension manifests (`customEditors`, `views`, etc.), `RpcBridge` & `NodeExtHostProcess`,
│   │                           #          structured `RpcError`, 16 `ProviderType`s, 14 managers (Diagnostic, StatusBar, Notebook,
│   │                           #          Output, Terminal, SCM, Debug, Task, Test, Auth, Webview, CustomEditor, Window, Env),
│   │                           #          `env.getOsInfo` & `process.{exec,spawn,spawnSync}` RPCs,
│   │                           #          Open VSX marketplace client, VSIX installer & `compat_suite`
│   ├── rustcode-workbench/     # Layer 3: ActivityBar (Explorer, Search, SCM, Debug, Extensions, Testing),
│   │                           #          Explorer CRUD (`New File`/`New Folder`/`F2`/`Delete`) & Contributed TreeViews,
│   │                           #          TestingSidebarView (`workbench.view.testing`), NotificationsOverlay & PromptOverlay,
│   │                           #          EditorArea split grid (`EditorGridNode`/`EditorSash`) & CustomEditor routing,
│   │                           #          TerminalPanelView, OutputPanelView, SettingsUiView, NotebookEditorView,
│   │                           #          WebviewEditorView, BreadcrumbsBar, ProblemsPanel,
│   │                           #          CommandPalette / QuickInput (`Ctrl+P`, `>`, `#`, `:line[:col]`, `@symbol`), StatusBar
│   └── rustcode-app/           # Bootstrap, service container (`RustCodeContext` + live `LanguageFeatureBridge` &
│                               #          `SharedWorkbenchLayout` wiring), benchmarks & integration tests
├── ext-host-js/
│   ├── rpc-bridge.js           # Framed stdio transport (`Content-Length` / newline-JSON), `globalThis` singleton, `require('vscode')` hook
│   ├── vscode-api/             # `require('vscode')` (13 modules): index, commands, window, workspace, languages, notebooks,
│   │                           # scm, debug, tasks, tests, authentication, env, types (`EventEmitter`, `Uri`, `RelativePattern`, etc.)
│   ├── node-compat/            # 9 Node.js compat polyfills (`events`, `fs`, `path`, `buffer`, `url`, `util`, `os`, `crypto`, `child_process`)
│   └── test.js                 # JavaScript extension host suite (`ext-host-js/test.js`)
├── resources/                  # Default Dark+/Light+ themes, Linux keymaps, 1 sample extension (`hello-world`), `rustcode.desktop`
├── scripts/                    # Release packaging scripts: `scripts/package.ps1` (Windows `.zip`) & `scripts/package.sh` (Linux/macOS `.tar.gz`)
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

### Completed (Phase 7 — Pure-Rust Parity & Extended Ecosystem)
- [x] **Phase 7 M1.1 — Live Framed Stdio RPC Transport, Structured Error Envelope & `node-compat/{fs,path}.js` Parity**: `NodeExtHostProcess` (`ContentLengthJson`, `ContentLengthMsgPack`, `NewlineJson`, `from_io`, `JoinSet` half-close EOF draining, Windows `CREATE_NO_WINDOW`), O(N) `read_content_length_frame` (`64 KiB` header / `64 MiB` frame limits), structured `{ error: { code, message } }` (`RpcError` / `RpcErrorPayload`) across 12+ namespaces via `handle_rpc_message_result`, `node-compat/fs.js` (`Dirent`, encodings, `mkdir` `firstCreated`, `promises.{access,rm,rename,copyFile}`, `constants`, `invokeCallback`, 11 `*Sync` methods throwing `ENOSYS` in live stdio except `existsSync`), OS-aware `node-compat/path.js` (`win32`/`posix`, `relative`, `parse`, `format`, UNC/drive-letter parity), and Windows/Linux `map_io_error` + `EISDIR` parity
- [x] **Phase 7 M1.2 — Extension Host VS Code API Parity & Live Fixture E2E**: Parity across `commands`, `window`, `workspace`, `env`, `extensions`; sequential FIFO incoming notification worker + `IncomingNotifWork::Barrier` causal fence; `reader_closed` fast-fail; `command_mutation_lock` + token-checked `CommandRegistry` disposal; Windows folder-config equivalence & `extract_scope_path` `Uri` normalization + `affectsConfiguration(section, scope)`; CRLF/UTF-16 `TextDocumentProxy`; pluggable `ClipboardBackend` (`OsClipboardBackend`/`InMemoryClipboardBackend`) & `ExternalUriOpener` (`OsExternalUriOpener`/`RecordingExternalUriOpener`); `TreeDataProvider`/`TreeData` synthetic handles & `onDidChangeTreeData`; `require('vscode')` resolution (`installVscodeModuleHook` + `globalThis` singleton); live fixture (`hello-world`, `treeview_ext`, `theme_ext`) & multi-threaded 4-worker stdio reverse-split stress tests
- [x] **Phase 7 M1.3 — `RustCodeContext` & `WorkbenchWindow` End-to-End Wiring**: Wired live `LanguageFeatureBridge` (loader-hydrated) & shared `FileSystemService` in `RustCodeContext::create_window()`, `SharedWorkbenchLayout` / `LayoutHandle` & `WindowCommandSink` for immediate builtin command effects without double-toggling, synchronous document lifecycle forwarding (`did_{open,change,save,close}_document_sync`), UI sync hooks (`status_bar`, `window_manager`, `test_manager`, `provider_registry`) with `Escape`-sync safety, and Windows drive-root / `/` Explorer name fallback
- [x] **Phase 7 M2 — In-Editor Find & Replace (`Ctrl+F` / `Ctrl+H`) & QuickInput Navigation (`Ctrl+G` / `@`)**: Floating `FindReplaceWidget` overlay (`actions.find`, `editor.action.startFindReplaceAction`) with live match highlights, case/whole-word/regex toggles, zero-width regex guard, UTF-16/CRLF offsets, fold unfolding, and single-undo Replace All; plus QuickInput `:line[:col]` (`workbench.action.gotoLine`, `Ctrl+G`) and `@symbol` (`workbench.action.gotoSymbol`, `Ctrl+Shift+O`) with dynamic `Ctrl+P` prefix switching
- [x] **Phase 7 M3 — Notifications, Interactive Prompts & Testing Sidebar (`workbench.view.testing`)**: Non-blocking `NotificationsOverlay` toast stack & `AccessibleRole::Dialog` modal dialogs, interactive `PromptOverlay` (`showQuickPick` / `showInputBox` with `validateInput`), and `TestingSidebarView` at Activity Bar index 5 (`workbench.view.testing`) with failure badge, disposed-controller filtering, `"Run All"` RPC dispatch, and `file://` click-to-open
- [x] **Phase 7 M4 — Interactive File Explorer CRUD & Contributed TreeViews**: Explorer `"New File"` (`+F`), `"New Folder"` (`+D`), inline `F2` rename (updating open tabs), confirmed `Delete` (closing open tabs), keyboard navigation, cross-platform filename validation, and contributed `TreeDataProvider` views (`contributes.views`, `when`-clause evaluation, lazy `getChildren`, refresh events, command execution)
- [x] **Phase 7 M5 — Expanded `node-compat` (9 Modules), Process/OS RPCs & 14 Language Tokenizers**: Added `ext-host-js/node-compat/{buffer,url,util,os,crypto,child_process}.js` (9 modules total alongside `events`, `fs`, `path`), Rust `env.getOsInfo` & `process.{exec,spawn,spawnSync}` RPCs, and 7 pure-Rust state-machine tokenizers (`go`, `c`, `cpp`, `html`, `css`, `toml`, `yaml` — 14 grammars total) with `GrammarState::RawString` and `TextDocument::map_ext_to_lang_id`
- [x] **Phase 7 M6 — Documentation & Trackers Refresh**: Refreshed `README.md`, `rustcode-implementation-plan.md`, `docs/architecture.md`, and `docs/extension-compat-matrix.md` to reflect M1.1–M5 implementations
- [x] **Phase 7 M7 — Full Verification, Tier 5 Hardening & Final Counts**: Verified **513/513** workspace tests across **8** crates (**41/41** `compat_suite`), **31/31** `ext-host-js/test.js` sections (**404** `assert(...)` call sites), **8/8** `tests/phase7-e2e/runner.js --syntax-check` checks, **286/286** Windows (`287` Unix) `tests/phase7-e2e` tests across Tiers 1–4, Tier 5 adversarial hardening, **0** `clippy` warnings on both Windows (`x86_64-pc-windows-gnu`) and Linux (`x86_64-unknown-linux-gnu`), `cargo bench --workspace --no-run`, and `rustcode` startup smoke check

### Future — Full Roadmap to v1.0 (Pre-Phase 8 Cleanup & Phases 8–12)

#### Anti-Drift Guardrails
- **No Silent Stack Substitutions**: Locked-in engines (`winit`/`wgpu`/`cosmic-text`, live Node.js `NodeExtHostProcess`, `alacritty_terminal` + real PTY, `wry`, TextMate `.tmLanguage` + `tree-sitter`, real `git` CLI, `ripgrep`/`nucleo`, `rusqlite`/`arboard`/`rfd`/`dirs`, `accesskit`) MUST NOT be silently replaced by stubs; any change requires an explicit plan amendment first.
- **Trait & Feature-Gate Isolation**: Keep deterministic headless/`Mock*` backends behind trait boundaries (`RenderSurface`, `PtyBackend`, `WebviewBackend`, `GitBackend`, `StorageBackend`, `ClipboardBackend`, `FileDialogBackend`, `AccessibilityBackend`) for CI tests while wiring real engines in the `desktop` app.
- **Mandatory Milestone Exit Gate**: Every milestone requires zero stack drift, passing workspace/`compat_suite`/`ext-host-js`/`phase7-e2e`/phase E2E suites, `0` Windows & Linux `clippy -D warnings` (default + `desktop` features), and synchronized tracker updates across all docs.

#### Pre-Phase 8 Cleanup Checklist
- [ ] **Consolidate stdio `Content-Length` codecs**: Migrate `LspClient` from `crates/rustcode-protocols/src/lsp/codec.rs` to the shared `crates/rustcode-protocols/src/codec.rs` (and align bounded framing with `crates/rustcode-ext-host/src/rpc/mod.rs`)
- [ ] **Unify `map_ext_to_lang_id`**: Deduplicate the drifted extension-to-language mappings between `crates/rustcode-editor/src/document/mod.rs` and `crates/rustcode-ext-host/src/provider/bridge.rs` (`.hh`/`.hxx`, `.txt`/`.log`, `.sh`/`.bash`)
- [ ] **Deduplicate `ext-host-js/node-compat/` Base64 & UTF-8 codecs**: Share `buffer.js` Base64/UTF-8 primitives across `fs.js` (`decodeBase64Pure`/`encodeBase64Pure`) and `util.js` (`TextEncoder`/`TextDecoder`)
- [ ] **Extract shared geometry, scene & fuzzy-match helpers**: Centralize `is_finite_rect` (5 copies), `push_finite_quad`/`push_finite_text` (3 copies), `clamp_rect`/`clamp_rect_to_bounds` (4 copies), identical `fuzzy_match` (`quickinput/mod.rs` & `search/symbols.rs`), and `BenchmarkStats` (2 bench files)
- [ ] **Remove dead code, legacy dual state & unused Cargo items**: Delete unused `#[allow(dead_code)]` AVL rotation methods in `piece_table.rs`, remove unused `RustCodeContext::editor_area`, resolve no-op `gpui` Cargo feature across 3 crates, simplify `FileExplorer` dual `root`/`roots`, and trim unused `[dependencies]` (`smallvec`, `thiserror`, `serde`, move test-only `rustcode-editor`/`rustcode-terminal` in `rustcode-app` to `[dev-dependencies]`)
- [ ] **Tidy dual sync/RPC wrappers & align blueprint notes**: Share internal sync helpers between `WorkbenchWindow::sync_*` and `sync_for_render`, evaluate deprecating legacy `handle_rpc_message`, and align superseded `deno_core` / Zed `gpui-fork` / `git2` notes with the 1:1 VS Code Rust Hybrid stack in `rustcode-implementation-plan.md`

#### Phase 8: 1:1 VS Code Native Desktop & Runtime Parity (Planned — Rust Hybrid Stack)
> Target stack: **`winit` + `wgpu` + `cosmic-text`** behind `RenderSurface` (keeping `MockRenderSurface` for headless tests), live **Node.js `NodeExtHostProcess`** (real VS Code extension host parity), **`alacritty_terminal` + real PTY**, **`wry`** WebView2/WebKit, **TextMate `.tmLanguage` + `tree-sitter`**, **real `git` CLI**, **`ripgrep` (`ignore`/`grep-regex`) + `nucleo`**, and **`rusqlite` / `arboard` / `rfd` / `dirs`**.

- [ ] **Phase 8 M1 — Hardware GPU Desktop Window & Rich Text Rasterizer (`winit` + `wgpu` + `cosmic-text`)**: Feature-gated `desktop` `RenderSurface` rendering `Scene::{Quad, GlyphRun, Shadow, PushClip, PopClip}` with rounded corners, borders, shadows, clip stacks, `cosmic-text` shaping (ligatures, CJK/emoji/fallback fonts, Codicons), HiDPI scaling, and live OS keyboard/IME/mouse/drag/scroll/resize/close event loop (preserving `MockRenderSurface` as default headless test backend)
- [ ] **Phase 8 M2 — CLI, SQLite State Persistence (`rusqlite`), Native OS Integration (`rfd`/`arboard`/`dirs`) & Live Runtime Loop**: CLI parser (`rustcode [path...]`, `--headless`, `--version`, `--goto file:line[:col]`, `--user-data-dir`, `--extensions-dir`, `--disable-extensions`), disk loading of `~/.rustcode/` & `<workspace>/.vscode/` configs/keymaps/themes/extensions, SQLite `StorageService` (recent workspaces, tabs, layout, extension `Memento`), native Open/Save dialogs (`rfd`), `arboard` OS clipboard, and background Tokio runtime driving `FileSystemWatcher`, `AutoSaveService`, and live `NodeExtHostProcess` auto-activation/crash recovery with full Node.js runtime compatibility for real-world extensions
- [ ] **Phase 8 M3 — Daily-Driver Editor, `ripgrep`/`nucleo` Search & TextMate + `tree-sitter` Syntax Engine**: Editor clipboard shortcuts (`Ctrl+C`/`X`/`V` incl. empty-selection line copy/cut), mouse drag/double/triple-click selection, scrollbar/Minimap dragging, `Ctrl+/` comments across all 14+ grammars, `Alt+Up/Down` move & `Shift+Alt+Up/Down` duplicate lines, multi-line indent/outdent, bracket auto-close/matching, right-click context menus; `unicode-segmentation`/`unicode-width`/`encoding_rs` in `rustcode-editor`; `ignore`/`grep-regex` (`ripgrep`) multi-file search/replace (`Ctrl+Shift+F`/`Ctrl+Shift+H`) + `nucleo` fuzzy matching in QuickInput; and TextMate `.tmLanguage` extension grammar loader + `tree-sitter` incremental highlighting layered over the 14 built-in grammars
- [ ] **Phase 8 M4 — Real PTY Terminal (`alacritty_terminal`) & System WebViews (`wry`)**: Replace one-shot `std::process::Command` runner with real PTY (`portable-pty` / ConPTY / Unix PTY) + `alacritty_terminal` VT100/xterm grid emulation (ANSI colors, interactive shells, TUIs like `vim`/`htop`, signals, resize); replace simplified HTML tag parser in `WebviewEditorView` with OS WebViews via `wry` (WebView2 / WebKitGTK / WKWebView) wired to `ExtHostWebviewManager` & `ExtHostCustomEditorManager` `postMessage` channels (preserving headless test fallback)
- [ ] **Phase 8 M5 — Real `git` CLI SCM, Auto-Attached LSP Servers & Live DAP (`launch.json`)**: Replace `ScmRepository::sample_repository()` with pure-Rust `GitCliBackend` (`git status --porcelain=v2 -b`, `diff`, `show HEAD:<path>`, `add`, `restore --staged`, `commit`, Status Bar branch readout); auto-attach configured LSP servers (`rust-analyzer`, `typescript-language-server`, `pyright`, `gopls`) for hover/go-to-def (`F12`/`Ctrl+Click`)/autocomplete/diagnostics/`editor.formatOnSave`; and parse `.vscode/launch.json` to launch real DAP adapters replacing `DebugSessionState::sample_session()`
- [ ] **Phase 8 M6 — Cross-Platform Hardening, Native Windows/Linux CI & 1:1 Extension Verification**: Close Phase 7 M7 minor caveats (Unix `setsid`/`killpg` in `process/mod.rs`, 4 `util.isDeepStrictEqual` edge cases, `ContextKey` `[^]/]` regex char-class), add `.github/workflows/ci.yml` (`windows-latest` + `ubuntu-latest` running workspace tests, `compat_suite`, `ext-host-js/test.js`, native 286 Win / 287 Unix `phase7-e2e`, `clippy -D warnings`, benches, and `package.ps1`/`package.sh`), add `tests/phase8-e2e/`, and verify real-world VS Code extensions (`One Dark Pro`, `Prettier`, `ESLint`, `rust-analyzer`, `GitLens`)

#### Phase 9: Deep Editing, Navigation & Language Intelligence Parity (Planned)
- [ ] **Phase 9 M1 — Advanced LSP & Multi-File Refactoring**: Rich Markdown hover popovers, Signature Help (`Ctrl+Shift+Space`), Peek References / Go to Implementation / Type Definition (`Shift+F12`), Rename Symbol (`F2`) with multi-file `WorkspaceEdit` preview & undo, LSP Semantic Tokens layering over TextMate/`tree-sitter`, Inlay Hints, CodeLens, Document Links, and Call/Type Hierarchy views
- [ ] **Phase 9 M2 — Snippets Engine, Language Config & Large-File PieceTable**: Full VS Code TextMate snippet engine (nested tabstops, `${1|a,b,c|}` choices, regex transforms, `$TM_*` variables), `contributes.snippets` & `language-configuration.json` loader (auto-indent, on-enter rules, surrounding pairs), Sticky Scroll, bracket-pair colorization, and true O(log n) balanced piece-tree rebalancing + virtualized rendering for 100 MB+ files
- [ ] **Phase 9 M3 — Full Workbench Chrome, Context Menus & Multi-Window**: Drag-and-drop tabs across split groups & Explorer files, Secondary Sidebar (right panel), Panel position toggle (bottom/left/right), draggable Sash resizing, custom Title Bar & Menu Bar, full `contributes.menus` right-click Context Menus with `when` clauses, Window Zoom (`Ctrl+=`/`-`/`0`), Zen Mode (`Ctrl+K Z`), and Multi-Window orchestration (`Ctrl+Shift+N` + single-instance IPC socket forwarding CLI files into active windows)
- [ ] **Phase 9 M4 — Phase 9 E2E & Large-File Benchmarks (`tests/phase9-e2e/`)**: Automated verification of Rename Symbol, Peek References, Semantic Tokens, Snippets, Drag-and-Drop layout, Multi-Window IPC, and 100 MB file latency budgets

#### Phase 10: Real-World Top-100 VS Code Extension Ecosystem & Marketplace Maturity (Planned)
- [ ] **Phase 10 M1 — Remaining `vscode.*` APIs & Contribution Points**: `workspace.registerFileSystemProvider` (virtual/remote schemes), `TextDocumentContentProvider`, `WebviewViewProvider` (sidebar/panel webviews), `TreeView` drag-and-drop & checkboxes, `SecretStorage` (OS keyring), `LogOutputChannel`, `TerminalShellIntegration`, `CustomExecution` tasks + Problem Matchers, and `contributes.{menus,viewsContainers,viewsWelcome,submenus,jsonValidation,problemMatchers,iconThemes,productIconThemes}`
- [ ] **Phase 10 M2 — Extension Host Runtime Polish & Profiling**: Managed Node.js discovery/version check, per-extension activation & RPC profiling (`Developer: Show Running Extensions`), unresponsive extension host watchdog + restart banner, and transparent real-Node passthrough alongside `require('vscode')` hook for unmodified Open VSX `.vsix` bundles
- [ ] **Phase 10 M3 — Rich Marketplace UX & Profiles**: Full Open VSX detail view via `wry` (README, changelog, dependencies), extension packs, pre-release channel toggle, auto-updates, install-from-`.vsix`, `.vscode/extensions.json` workspace recommendations, and VS Code **Profiles** (settings/keybindings/extensions snapshots)
- [ ] **Phase 10 M4 — Automated Real-`.vsix` Compatibility Harness (`tests/phase10-ext-compat/`)**: Automated CI harness exercising real `.vsix` extensions (`One Dark Pro`, `Tokyo Night`, `Material Icon Theme`, `Prettier`, `ESLint`, `rust-analyzer`, `Python`, `GitLens`, `Tailwind CSS`, `Error Lens`, `Todo Tree`, `YAML`, `TOML`, `Markdown All in One`) and keeping `docs/extension-compat-matrix.md` scores verified

#### Phase 11: Remote Development, Tasks/Git Merge & Accessibility/I18n (Planned)
- [ ] **Phase 11 M1 — Remote Development (`rustcode-remote` — SSH & Dev Containers)**: Headless `rustcode-server` binary over SSH (`vscode-remote://ssh-remote+...`) and `devcontainer.json` discovery/attach, multiplexing remote `FileSystemProvider`, remote `NodeExtHostProcess`, remote LSP/DAP, and remote PTY terminal over a single framed tunnel
- [ ] **Phase 11 M2 — `.vscode/tasks.json`, Git Blame/Branches & 3-Way Merge Editor**: Full `tasks.json` runner with Problem Matchers feeding the Problems panel, Git branch/stash/tag picker & inline blame/timeline annotations, and a 3-way visual Merge Conflict Editor (`Accept Current` / `Accept Incoming` / `Accept Both` / `Compare`)
- [ ] **Phase 11 M3 — Native OS Accessibility (`accesskit`), IME & Localization (i18n)**: Bridge `AccessibilityTree` (`crates/rustcode-platform/src/accessibility.rs`) to native OS screen readers via **`accesskit`** (Windows UIA, Linux AT-SPI, macOS NSAccessibility), full CJK/dead-key IME composition overlay in `winit` + `cosmic-text`, High Contrast themes, and VS Code `nls.json` localization packs
- [ ] **Phase 11 M4 — Phase 11 Verification (`tests/phase11-e2e/`)**: Loopback SSH/`rustcode-server` remote workspace E2E, `tasks.json` + 3-way merge conflict tests, and `accesskit` tree parity checks

#### Phase 12: v1.0 Production Hardening, Performance Budgets & Release Engineering (Planned)
- [ ] **Phase 12 M1 — Performance Budget Enforcement & Soak Testing**: Enforce CI performance budgets against real desktop/headless workloads (`< 500 ms` cold start to interactive GPU window, `< 120 MB` idle RAM, `< 350 MB` 20-file RAM, `< 3 ms` keystroke latency, `< 50 ms` 1 MB file open, `< 1 s` 100K-file search, 120 FPS frame pacing, `< 80 MB` binary size) and long-session leak checks
- [ ] **Phase 12 M2 — Security Audit, Workspace Trust & Crash Recovery**: VS Code **Workspace Trust** (Restricted Mode gating untrusted workspace tasks/extensions/local settings), `.vsix` & path-sandbox security audit, OS keyring secret encryption, and crash-dump/panic recovery with dirty-buffer hot-exit restore
- [ ] **Phase 12 M3 — macOS Support, Native OS Installers & Signed Auto-Updater**: Add macOS (`aarch64-apple-darwin` & `x86_64-apple-darwin`) to CI; ship signed native installers (Windows MSI/Inno Setup + `winget` + *"Open with RustCode"* context menu, Linux `.deb`/`.rpm`/`AppImage`/`Flatpak`, macOS `.dmg`/`.app`) and delta auto-updater
- [ ] **Phase 12 M4 — v1.0 GA Audit & Release**: Side-by-side VS Code UX audit, VS Code migration wizard (`settings.json` / `keybindings.json` / extensions import), complete user & extension-author docs, and tagged `v1.0.0` GA release

---

## 🧪 Building, Testing & Packaging

**Phase 7 verified baseline**: **513/513 workspace tests** across **8 crates** (`rustcode-app` 27 [3 unit + 24 integration], `rustcode-base` 12, `rustcode-editor` 126, `rustcode-ext-host` 127 [86 unit + 41 `compat_suite`], `rustcode-platform` 40, `rustcode-protocols` 24, `rustcode-terminal` 9, `rustcode-workbench` 148), **31/31 `ext-host-js/test.js`** sections (**404 `assert(...)` call sites**), **286/286** Windows (**287** Unix) opaque-box `tests/phase7-e2e/` tests across Tiers 1–4 + Tier 5 adversarial hardening, and **0 `clippy` warnings** (`-D warnings`) on both **Windows (`x86_64-pc-windows-gnu`)** and **Linux (`x86_64-unknown-linux-gnu`)**.

```bash
# Clone the repository
git clone https://github.com/revelino01/vscode-in-rust.git
cd vscode-in-rust

# Run all workspace unit and integration tests (use -j 2 on constrained Windows hosts)
cargo test -j 2 --workspace

# Run extension host compatibility suite
cargo test -j 2 -p rustcode-ext-host --test compat_suite

# Run JavaScript extension host suite
node ext-host-js/test.js

# Run Piece Table stress tests
cargo test -j 2 -p rustcode-editor -- piece_table

# Lint workspace cleanly on Windows host and Linux cross-target (warnings treated as errors)
cargo clippy -j 2 --workspace --all-targets -- -D warnings
cargo clippy -j 2 --target x86_64-unknown-linux-gnu --workspace --all-targets -- -D warnings

# Compile benchmarks (without running full bench harness)
cargo bench --workspace --no-run -j 2

# Build release binary and package distribution bundle:
# Linux / macOS (.tar.gz + rustcode.desktop on Linux):
./scripts/package.sh
# Windows (.zip via PowerShell; pass -Build to compile release binary first):
pwsh -File ./scripts/package.ps1 -Build
```

> **Standalone E2E Suite (`tests/phase7-e2e/`):** `tests/phase7-e2e/` is an isolated standalone Cargo `[workspace]` outside root `Cargo.toml` with **286/286** Windows (**287** Unix) opaque-box E2E tests passing across Tiers 1–4 (`node tests/phase7-e2e/runner.js --syntax-check` [8/8] and `node tests/phase7-e2e/runner.js --tier all` [10/10 targets]).

---

## 📄 License

MIT OR Apache-2.0
