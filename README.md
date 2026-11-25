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
| **Keystroke Latency** | 5–20 ms | **< 3 ms** | **36 ns** typing (`editor_latency.rs`) · **111 ns** 100 MiB edit (`large_file.rs`) |
| **Large File (100 MiB / 1.23M lines)** | 1–4 s+ | **< 3 ms edit** | **43.75 ms** load (2285.8 MiB/s) · **1.49 µs** pos query · **4.59 ms** frame (`large_file.rs`) |
| **Frame Rate Target** | 60 FPS | **120 FPS** | GPU scene-graph pipeline |

---

## 🏗️ Architecture

| Component | Choice | Description |
|---|---|---|
| **GUI / Presentation** | **`winit` + `wgpu` + `cosmic-text`** (`RenderSurface`, `desktop` feature) | Hardware GPU display-list rendering (rounded quads, shadows, clip stacks, ligatures, CJK/emoji/Codicons, HiDPI) with `MockRenderSurface` preserved for default headless CI/tests |
| **Extension Host** | **Live `NodeExtHostProcess`, `RpcBridge` & `ext-host-js`** | Live framed stdio (`Content-Length` JSON/MsgPack & newline-JSON) Rust ↔ Node subprocess transport with auto-activation/crash recovery, structured `RpcError`, `vscode.*` API shims, and 9 `node-compat` polyfills |
| **IPC Protocol** | **Shared `rustcode_protocols::codec` (JSON & MessagePack)** | Bounded O(N) `Content-Length` framing (`64 KiB` header / `64 MiB` frame limits) shared across LSP, DAP, and Extension Host RPC |
| **Text Buffer & Unicode** | **Piece Table + `unicode-segmentation` / `unicode-width` / `encoding_rs`** | Balanced Piece Table with atomic undo/redo, grapheme-aware cursoring, display column widths, and UTF-8/UTF-16/Windows-1252/Shift-JIS roundtrips |
| **Syntax Highlighting** | **TextMate `.tmLanguage` + `tree-sitter` + 14 Built-in Grammars** | Extension TextMate `.tmLanguage`/`.tmLanguage.json` loader + `desktop` `tree-sitter` incremental parsing layered over **14** pure-Rust grammars (`plaintext`, `rust`, `javascript`, `typescript`, `python`, `json`, `markdown`, `go`, `c`, `cpp` / C++, `html`, `css`, `toml`, `yaml`) |
| **Search & Fuzzy** | **`ripgrep` (`ignore` + `grep-regex`) + `nucleo-matcher`** | `.gitignore`-aware workspace search/replace (`Ctrl+Shift+F`/`Ctrl+Shift+H`) and VS Code-grade fuzzy matching in QuickInput |
| **Language, Debug & Git** | **Auto-LSP (`gopls`+4), DAP (`launch.json`) & `GitCliBackend`** | Auto-attached LSP servers (`rust-analyzer`, `typescript-language-server`, `pyright`, `gopls`) + `formatOnSave`, `.vscode/launch.json` DAP sessions, real `git` CLI SCM, and Myers O(ND) diff |
| **Terminal & WebViews** | **`portable-pty` + `alacritty_terminal` & `wry` (`desktop`)** | Real PTY + VT100/xterm grid emulation (`PtyBackend`) and OS system WebViews (`WebviewBackend` via `wry` + `postMessage`), both with deterministic headless test fallbacks |
| **Persistence & OS UX** | **`rusqlite` + `rfd` + `arboard` + `dirs`** | SQLite workspace/tab/`Memento` persistence (`SqliteStorageService`), native open/save dialogs, OS clipboard, and XDG/AppData path resolution |

---

## 📦 Workspace Crates & Structure

```text
vscode-in-rust/
├── crates/
│   ├── rustcode-base/          # Layer 0: Disposables, typed Emitter/Event, CancellationToken, `when`-clause parser, canonical `map_ext_to_lang_id`
│   ├── rustcode-platform/      # Layer 1: ConfigurationService (JSONC + disk loading), KeybindingService, CommandRegistry, ThemeService,
│   │                           #          FS/AutoSave/Watcher, Scene + `WgpuRenderSurface` (`desktop`), `SqliteStorageService` (`desktop`),
│   │                           #          `AppPaths` (`dirs`), `ArboardClipboardBackend`, `RfdFileDialogBackend` (`desktop`), AccessibilityTree
│   ├── rustcode-editor/        # Layer 2: AVL-balanced Piece Table (100 MB+ ready), `unicode-*`/`encoding_rs`, daily-driver EditorView,
│   │                           #          Snippets Engine, `language-configuration.json`, Sticky Scroll, Rainbow Brackets, LSP Hover/
│   │                           #          SignatureHelp/Peek/SemanticTokens/InlayHints/CodeLens/DocumentLinks, FindReplaceWidget, Folding,
│   │                           #          WordWrapMode, TextMate `.tmLanguage` + `tree-sitter` (`desktop`) + 14 built-in grammars
│   ├── rustcode-terminal/      # Terminal: `PtyBackend` (`HeadlessPtyBackend` / `PortablePtyBackend`), `alacritty_terminal` VT100/xterm grid, split/tab sessions
│   ├── rustcode-protocols/     # Layer 5: Shared bounded `codec`, auto-LSP 3.17 (incl. SignatureHelp/SemanticTokens/Rename/Hierarchy + `gopls`), DAP + `launch.json`, `GitBackend` / `GitCliBackend`
│   ├── rustcode-ext-host/      # Layer 4: Extension manifests (13 `contributes.*` incl. `snippets`/`menus`), `RpcBridge` & `NodeExtHostProcess`, Unix `setsid`/`killpg` isolation,
│   │                           #          26 `ProviderType`s, 14 managers, Open VSX marketplace, VSIX installer & `compat_suite`
│   ├── rustcode-workbench/     # Layer 3: ActivityBar (6 views), Explorer CRUD/D&D & TreeViews, `ripgrep`/`nucleo` Search, TestingSidebarView,
│   │                           #          Title Bar & `ContextMenuRegistry` (`when` clauses), Secondary Sidebar, Panel Bottom/Left/Right + Sashes,
│   │                           #          Split EditorGrid + Tab D&D, Call/Type Hierarchy & Refactor Preview (atomic undo), Zoom/Zen, WebViews
│   └── rustcode-app/           # Layer 6: CLI (`-n`/`-r`/`-g`), `WindowManager` & `SingleInstanceIpc` multi-window host, Tokio `RuntimeLoop`,
│                               #          `winit` desktop loop (`desktop`), `RustCodeContext`, binary (`src/main.rs`), benches & tests
├── ext-host-js/
│   ├── rpc-bridge.js           # Framed stdio transport (`Content-Length` / newline-JSON), `globalThis` singleton, `require('vscode')` hook
│   ├── vscode-api/             # `require('vscode')` (13 modules): index, commands, window, workspace, languages, notebooks,
│   │                           # scm, debug, tasks, tests, authentication, env, types (`EventEmitter`, `Uri`, `RelativePattern`, etc.)
│   ├── node-compat/            # 9 Node.js compat polyfills (`events`, `fs`, `path`, `buffer`, `url`, `util`, `os`, `crypto`, `child_process`)
│   └── test.js                 # JavaScript extension host suite (`ext-host-js/test.js`)
├── .github/workflows/ci.yml    # Windows (`windows-latest`) & Linux (`ubuntu-latest`) CI: tests, E2E suites, clippy, benches & packaging
├── resources/                  # Default Dark+/Light+ themes, Linux keymaps, 1 sample extension (`hello-world`), `rustcode.desktop`
├── scripts/                    # Release packaging scripts: `scripts/package.ps1` (Windows `.zip`) & `scripts/package.sh` (Linux/macOS `.tar.gz`)
├── tests/phase7-e2e/           # Standalone Phase 7 opaque-box E2E harness (Tiers 1–4, excluded from root Cargo `[workspace]`)
├── tests/phase8-e2e/           # Standalone Phase 8 opaque-box E2E harness (Tiers 1–4 + JS, excluded from root Cargo `[workspace]`)
├── tests/phase9-e2e/           # Standalone Phase 9 E2E harness (LSP refactoring, snippets, 100 MB virtualization, chrome D&D, multi-window IPC)
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

### Completed (Pre-Phase 8 Cleanup)
- [x] **Consolidate stdio `Content-Length` codecs**: Removed `crates/rustcode-protocols/src/lsp/codec.rs`; unified `LspClient`, `DapClient`, and `NodeExtHostProcess` onto bounded O(N) `rustcode_protocols::codec` (`64 KiB` header / `64 MiB` frame limits)
- [x] **Unify `map_ext_to_lang_id`**: Centralized canonical extension-to-language mapping in `rustcode_base::map_ext_to_lang_id`, shared by `rustcode-editor::document` and `rustcode-ext-host::provider::bridge`
- [x] **Deduplicate `ext-host-js/node-compat/` Base64 & UTF-8 codecs**: Shared `buffer.js` Base64 and UTF-8 primitives across `fs.js` and `util.js` (`TextEncoder`/`TextDecoder`)
- [x] **Extract shared geometry, scene, fuzzy-match & bench helpers**: Centralized `Rect`/`FontMetrics`/`Scene` finite/clamping helpers in `rustcode-platform::render`, `rustcode_workbench::fuzzy_match`, and `rustcode_platform::bench_support::BenchmarkStats`
- [x] **Remove dead code, legacy dual state & unused Cargo items**: Deleted unused AVL rotations in `piece_table.rs`, removed unused `RustCodeContext::editor_area`, replaced no-op `gpui` features with `desktop`, simplified `FileExplorer` onto `roots: Vec<FileTreeNode>`, and pruned unused `[dependencies]` (`smallvec`, `thiserror`, `serde`, moving `rustcode-terminal` in `rustcode-app` to `[dev-dependencies]`)
- [x] **Tidy dual sync/RPC wrappers & align blueprint notes**: Extracted shared `WorkbenchWindow` sync projection helpers (`sync_window_overlays`, `sync_testing_view_from_bridge`), retained `LanguageFeatureBridge::handle_rpc_message` cleanly for `tests/phase7-e2e/`, and marked historical pre-pivot notes in `rustcode-implementation-plan.md`

### Completed (Phase 8 — 1:1 VS Code Native Desktop & Runtime Parity, Stage 3 Verified)
> Locked-in stack: **`winit` + `wgpu` + `cosmic-text`** (`desktop` feature; `MockRenderSurface` preserved for headless CI/tests), live **Node.js `NodeExtHostProcess`**, **`alacritty_terminal` + `portable-pty`**, **`wry`** (`desktop`), **TextMate `.tmLanguage` + `tree-sitter`**, **real `git` CLI (`GitCliBackend`)**, **`ripgrep` (`ignore`/`grep-regex`) + `nucleo-matcher`**, and **`rusqlite` / `arboard` / `rfd` / `dirs`**.

- [x] **Phase 8 M1 — Hardware GPU Desktop Window & Rich Text Rasterizer (`winit` + `wgpu` + `cosmic-text`)**: Implemented `#[cfg(feature = "desktop")] WgpuRenderSurface` (`wgpu` + `cosmic-text` shaping, ligatures, CJK/emoji/fallback fonts, Codicons, rounded corners/borders/shadows, intersection `ClipStack`, `GlyphAtlasAllocator`, HiDPI scaling) and `rustcode-app::desktop` `winit` OS keyboard/IME/mouse/click/scroll/resize/close event loop (preserving `MockRenderSurface` as default headless backend)
- [x] **Phase 8 M2 — CLI, SQLite State Persistence (`rusqlite`), Native OS Integration (`rfd`/`arboard`/`dirs`) & Live Runtime Loop**: Added `rustcode-app::cli` (`[PATH...]`, `--headless`, `--gui`, `--goto file:line[:col]`, `--user-data-dir`, `--extensions-dir`, `--disable-extensions`, `--version`), `AppPaths` (`dirs`), disk config/keymap/theme/extension loading (`~/.rustcode/` & `<workspace>/.vscode/`), `SqliteStorageService` (`rusqlite`, `desktop`), `RfdFileDialogBackend` (`rfd`, `desktop`), `ArboardClipboardBackend` (`arboard`), and `RuntimeLoop` Tokio runtime driving `FileSystemWatcher`, `AutoSaveService`, and live `NodeExtHostProcess` auto-activation/crash recovery
- [x] **Phase 8 M3 — Daily-Driver Editor, `ripgrep`/`nucleo` Search & TextMate + `tree-sitter` Syntax Engine**: Added editor clipboard shortcuts (`Ctrl+C`/`X`/`V` incl. empty-selection line copy/cut), mouse drag & double/triple-click selection, scrollbar/Minimap dragging, `Ctrl+/` comments across languages, `Alt+Up/Down` move & `Shift+Alt+Up/Down` duplicate lines, multi-line indent/outdent, bracket auto-close/overtype/matching, right-click `EditorContextMenu`; `unicode-segmentation`/`unicode-width`/`encoding_rs` in `rustcode-editor`; `ignore`/`grep-regex` (`ripgrep`) workspace search/replace (`Ctrl+Shift+F`/`Ctrl+Shift+H`) + `nucleo-matcher` ranking in `CommandPalette`; and TextMate `.tmLanguage`/`.tmLanguage.json` loader + `desktop` `tree-sitter` incremental highlighting layered over the 14 built-in grammars
- [x] **Phase 8 M4 — Real PTY Terminal (`alacritty_terminal`) & System WebViews (`wry`)**: Added `PtyBackend` (`PortablePtyBackend` via `portable-pty` + `HeadlessPtyBackend`) and `TerminalCellGrid` (`alacritty_terminal` VT100/xterm emulation with ANSI/256/truecolor, alternate screen, resize, signals, scrollback) wired to `TerminalPanelView`/`TerminalAction`/`ExtHostTerminalManager`; added `WebviewBackend` (`WryWebviewBackend` via `wry` behind `desktop` + `HeadlessWebviewBackend` fallback) wired to `ExtHostWebviewManager` & `ExtHostCustomEditorManager` `postMessage` IPC channels
- [x] **Phase 8 M5 — Real `git` CLI SCM, Auto-Attached LSP Servers & Live DAP (`launch.json`)**: Implemented `GitBackend` / `GitCliBackend` (`git status --porcelain=v2 -b`, `diff`, `show HEAD:<path>`, `add`, `restore --staged`, `commit`, `log`, Status Bar branch readout) replacing `ScmRepository::sample_repository()` in default `WorkbenchView`; added auto-attached `LspClientManager` (incl. `gopls`, hover, `F12`/`Ctrl+Click` definition, completion, diagnostics, `editor.formatOnSave`); and added `.vscode/launch.json` JSONC parser + live `DapClient` sessions with `DebugToolbarView` hidden when idle
- [x] **Phase 8 M6 — Cross-Platform Hardening, CI & Compatibility Refresh**: Added Unix `setsid`/`setpgid`/`killpg` process-group isolation in `crates/rustcode-ext-host/src/process/mod.rs`, hardened `util.isDeepStrictEqual` edge cases, fixed `ContextKeyExpr` `[^]/]` regex char-class parsing, added `.github/workflows/ci.yml` (`ubuntu-latest` + `windows-latest`), updated `scripts/package.ps1` / `scripts/package.sh` (`-Desktop`/`--desktop`, `-Jobs`/`--jobs`), added `tests/phase8-e2e/`, and refreshed all 4 docs/trackers

### Completed (Phase 9 — Deep Editing, Navigation, Chrome & 100 MiB Buffer, Stage 3 Verified)
- [x] **Phase 9 M1 — Advanced LSP & Multi-File Refactoring**: Rich Markdown hover popovers, Signature Help (`Ctrl+Shift+Space`), Peek References / Go to Implementation (`Ctrl+F12`) / Type Definition (`Shift+F12`), Rename Symbol (`F2`) with multi-file `RefactorPreviewOverlay` & atomic `WorkspaceEdit` undo (`WorkspaceEditUndoStack`), LSP Semantic Tokens layered over TextMate/`tree-sitter`, Inlay Hints, CodeLens, Document Links, and Call/Type Hierarchy views (`HierarchySidebarView`, `Shift+Alt+H`)
- [x] **Phase 9 M2 — Snippets Engine, Language Config & Large-File PieceTable**: Full VS Code TextMate snippet engine (nested tabstops, `${1|a,b,c|}` choices, regex transforms, linked mirrors, `$TM_*` variables), `contributes.snippets` & `language-configuration.json` loader (auto-indent, on-enter rules, surrounding pairs), Sticky Scroll, rainbow bracket-pair colorization & guides, and true O(log n) AVL-balanced `PieceTable` rebalancing + virtualized rendering & `benches/large_file.rs` for 100 MB+ files
- [x] **Phase 9 M3 — Full Workbench Chrome, Context Menus & Multi-Window**: Drag-and-drop tabs across split groups & Explorer files, Secondary Sidebar (`workbench.action.toggleAuxiliaryBar`), Panel position toggle (bottom/left/right), draggable Sash resizing, custom Title Bar & Menu Bar, full `contributes.menus` right-click Context Menus with `when` clauses, Window Zoom (`Ctrl+=`/`-`/`0`), Zen Mode (`Ctrl+K Z`), and Multi-Window orchestration (`Ctrl+Shift+N`, `-n`/`-r` CLI flags, `WindowManager`, and cross-platform `SingleInstanceIpc` socket forwarding)
- [x] **Phase 9 M4 — Phase 9 E2E & Large-File Benchmarks (`tests/phase9-e2e/`)**: Automated Tier 1–4 Rust + JS E2E verification (`5/5` targets, `4/4` syntax/clippy checks) and `benches/large_file.rs` (`111 ns` mid-file edit latency on 100.00 MiB, `4.59 ms` virtualized viewport frame)

### Future — Full Roadmap to v1.0 (Phases 10–12)

#### Anti-Drift Guardrails
- **No Silent Stack Substitutions**: Locked-in engines (`winit`/`wgpu`/`cosmic-text`, live Node.js `NodeExtHostProcess`, `alacritty_terminal` + real PTY, `wry`, TextMate `.tmLanguage` + `tree-sitter`, real `git` CLI, `ripgrep`/`nucleo`, `rusqlite`/`arboard`/`rfd`/`dirs`, `accesskit`) MUST NOT be silently replaced by stubs; any change requires an explicit plan amendment first.
- **Trait & Feature-Gate Isolation**: Keep deterministic headless/`Mock*` backends behind trait boundaries (`RenderSurface`, `PtyBackend`, `WebviewBackend`, `GitBackend`, `StorageBackend`, `ClipboardBackend`, `FileDialogBackend`, `AccessibilityBackend`) for CI tests while wiring real engines in the `desktop` app.
- **Mandatory Milestone Exit Gate**: Every milestone requires zero stack drift, passing workspace/`compat_suite`/`ext-host-js`/`phase7-e2e`/phase E2E suites, `0` Windows & Linux `clippy -D warnings` (default + `desktop` features), and synchronized tracker updates across all docs.

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

## 💻 CLI Usage & Running RustCode

```bash
# Default headless bootstrap (zero-arg & --headless preserve MockRenderSurface smoke output)
cargo run --bin rustcode -j 2
cargo run --bin rustcode -j 2 -- --headless
cargo run --bin rustcode -j 2 -- --version

# Native Desktop GUI build & launch (winit + wgpu + cosmic-text + wry + rusqlite + rfd + tree-sitter)
cargo build --bin rustcode -j 2 --features desktop
cargo run --bin rustcode -j 2 --features desktop -- --gui .
cargo run --bin rustcode -j 2 --features desktop -- --goto src/main.rs:42:10

# Custom user-data/extensions directories & extension isolation
rustcode --user-data-dir /path/to/data --extensions-dir /path/to/exts --disable-extensions ./workspace
```

**Supported CLI Options (`crates/rustcode-app/src/cli.rs`):**
- `[PATH...]`: Open one or more workspace folders or files (launches Desktop GUI when compiled with `--features desktop`, unless `--headless` is set).
- `--headless`: Force headless bootstrap via `MockRenderSurface` and exit.
- `--gui`: Launch the native desktop `winit` + `wgpu` window (requires `--features desktop`).
- `-n, --new-window`: Force opening a new desktop window (bypasses existing `SingleInstanceIpc` leader reuse).
- `-r, --reuse-window`: Force forwarding paths / `--goto` requests into the active desktop window via `SingleInstanceIpc`.
- `-g, --goto <FILE:LINE[:COL]>`: Open a file at a 1-based line and optional column (supports Windows drive-letter paths like `C:\repo\src\lib.rs:10:5`).
- `--user-data-dir <DIR>`: Override the default user data/config directory (`dirs::config_dir()/rustcode`).
- `--extensions-dir <DIR>`: Override the default user extensions directory.
- `--disable-extensions`: Disable scanning and spawning extension host extensions.
- `-V, --version` / `-h, --help`: Print version or CLI usage information.

> **Linux Desktop System Dependencies (Ubuntu/Debian):** Building with `--features desktop` on Linux requires `pkg-config`, `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libxdo-dev`, `libasound2-dev`, `libxkbcommon-dev`, `libxkbcommon-x11-dev`, and `libssl-dev` (see `.github/workflows/ci.yml`).

---

## 🧪 Building, Testing, CI & Packaging

**Stage 3 Verified Results (Phases 0–9):** **637/637** default (**646/646** `--features desktop`) workspace tests across **8 crates** (`rustcode-app` 41 [17 lib + 24 integration] / 43 desktop, `rustcode-base` 14, `rustcode-editor` 156 / 158 desktop, `rustcode-ext-host` 130 [89 lib + 41 `compat_suite`], `rustcode-platform` 60 / 64 desktop, `rustcode-protocols` 39, `rustcode-terminal` 14, `rustcode-workbench` 183 / 184 desktop), **32/32 `ext-host-js/test.js`** `[Test]` sections, **286/286** (`10/10` targets) opaque-box `tests/phase7-e2e/` regression tests, **30/30** (`8/8` targets, `5/5` syntax checks) `tests/phase8-e2e/` tests, **12** (`5/5` targets [1 JS + 11 Rust], `4/4` syntax checks) `tests/phase9-e2e/` tests, **0 `clippy` warnings** (`-D warnings`) on **Windows (`x86_64-pc-windows-gnu`, default + `--features desktop`)** and **Linux (`x86_64-unknown-linux-gnu`)**, passing `editor_latency` (`36 ns` typing mean) & `large_file` (`43.75 ms` 100.00 MiB load, `111 ns` mid-file edit mean, `4.59 ms` viewport frame) benchmarks, and standalone `rustcode` CLI smoke checks (`--help`, `--version`, `--headless`, zero-arg).

```bash
# Clone the repository
git clone https://github.com/revelino01/vscode-in-rust.git
cd vscode-in-rust

# Run all workspace unit and integration tests (headless default & desktop feature; use -j 2 on Windows)
cargo test -j 2 --workspace
cargo test -j 2 --workspace --features desktop

# Run extension host compatibility suite & JavaScript suite
cargo test -j 2 -p rustcode-ext-host --test compat_suite
node ext-host-js/test.js

# Run standalone E2E suites (Phase 7 regression, Phase 8 native/runtime parity, Phase 9 deep editing & multi-window)
node tests/phase7-e2e/runner.js --syntax-check
node tests/phase7-e2e/runner.js --tier all
node tests/phase8-e2e/runner.js --syntax-check
node tests/phase8-e2e/runner.js --tier all
node tests/phase9-e2e/runner.js --syntax-check
node tests/phase9-e2e/runner.js --tier all

# Lint workspace cleanly on Windows host (default + desktop) and Linux cross-target (warnings treated as errors)
cargo clippy -j 2 --workspace --all-targets -- -D warnings
cargo clippy -j 2 --workspace --all-targets --features desktop -- -D warnings
cargo clippy -j 2 --target x86_64-unknown-linux-gnu --workspace --all-targets -- -D warnings

# Run editor keystroke & 100 MB large-file benchmarks (or compile all workspace benchmarks with --no-run)
cargo bench -j 2 -p rustcode-editor --bench editor_latency
cargo bench -j 2 -p rustcode-editor --bench large_file
cargo bench --workspace --no-run -j 2

# Build release binary and package distribution bundle (CI runs both via .github/workflows/ci.yml):
# Linux / macOS (.tar.gz + rustcode.desktop on Linux; pass --desktop to include desktop GUI features):
./scripts/package.sh --build --jobs 2
./scripts/package.sh --build --desktop --jobs 2
# Windows (.zip via PowerShell; pass -Desktop for desktop GUI features):
pwsh -File ./scripts/package.ps1 -Build -Jobs 2
pwsh -File ./scripts/package.ps1 -Build -Desktop -Jobs 2
```

> **Standalone E2E Suites (`tests/phase7-e2e/`, `tests/phase8-e2e/`, `tests/phase9-e2e/`):** All three suites live in isolated standalone Cargo `[workspace]` manifests outside root `Cargo.toml` and expose unified Node runners (`runner.js --syntax-check`, `--tier 1|2|3|4|all`, `--filter`, `--json`). `.github/workflows/ci.yml` executes the full workspace, JS, Phase 7/8/9 E2E, `clippy`, benchmark, and `package.ps1` / `package.sh` matrix on both `windows-latest` and `ubuntu-latest`.

---

## 📄 License

MIT OR Apache-2.0
