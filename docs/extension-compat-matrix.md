# Extension Compatibility Matrix

This document tracks compatibility of the RustCode Extension Host (`crates/rustcode-ext-host` + `ext-host-js/`) against the VS Code Extension API (`vscode.version = '1.95.0'`).

> **Runtime & Parity Note:** RustCode coordinates extension host state via a pure-Rust JSON/MessagePack `RpcBridge` and live framed stdio subprocess transport `NodeExtHostProcess` (`ContentLengthJson`, `ContentLengthMsgPack`, `NewlineJson`, `from_io`, `JoinSet` half-close EOF draining, `IncomingNotifWork::Barrier` causal fence) paired with `LanguageFeatureBridge::handle_rpc_message_result` (structured `RpcError` `{ error: { code, message } }`) and CommonJS shims in `ext-host-js/`. Phase 7 **M1.1 + M1.2** achieved Rust ↔ JS RPC parity across `commands`, `window`, `workspace`, `env`, `extensions`, `fs`, `treeView`, `languages`, `notebooks`, `scm`, `debug`, `tasks`, `tests`, and `authentication`. The tables below explicitly distinguish **`✅ JS + Rust RPC Parity`** (verified in `compat_suite.rs` and `ext-host-js/test.js`), **`🟡 Pending M1.3 Wiring / M3–M4 UI`** (desktop `RustCodeContext` / `WorkbenchWindow` end-to-end attachment, toast/prompt/testing/contributed tree-view UI), and **`⏳ Planned (M5)`**.

---

## Core API Namespaces & Types (`vscode.*`)

| Namespace / Module | Parity Level | JS API Surface (`ext-host-js/vscode-api/`) | Rust Host Wiring (`crates/rustcode-ext-host/`) & Honest Notes |
|---|---|---|---|
| `vscode.EventEmitter` & Core Types (`types.js`, `index.js`) | ✅ Full JS types + Rust core parity | `vscode.version = '1.95.0'`; `Position`, `Range`, `Selection`, `Uri` (`file`, `parse`, `from`, `fsPath`, `toString`, `toJSON`, Windows drive-letter/UNC/non-authority roundtrip), `Disposable`, `CancellationTokenSource`, `CancellationToken`, `EventEmitter` (`event`, `fire`, `dispose`), `DiagnosticSeverity`, `Diagnostic`, `CompletionItemKind`, `CompletionItem`, `Hover`, `StatusBarAlignment`, `ThemeColor`, `ThemeIcon`, `SnippetString`, `TextEdit`, `WorkspaceEdit`, `CodeActionKind`, `CodeActionTriggerKind`, `CodeAction`, `TreeItem`, `TreeItemCollapsibleState`, `ConfigurationTarget`, `EndOfLine`, `UIKind`, `ViewColumn`, `TextEditorSelectionChangeKind`, `TextEditorRevealType`, `FileType`, `FileSystemError`, `RelativePattern` | Shared DTOs aligned with `rustcode-base` (`Emitter<T>`, `CancellationToken`, `Disposable`), `rustcode-editor` (`Position`, `Range`, `Selection`, `TextEdit`, `EditorDiagnostic`), `rustcode-platform` (`file_uri_to_path`, `path_to_file_uri`, `paths_equivalent`), and `rustcode-protocols` (`rustcode_protocols::lsp::WorkspaceEdit`). |
| `vscode.window` (`window.js`) | ✅ JS + Rust RPC Parity / 🟡 Pending M1.3 & M3–M4 UI | `showInformationMessage`, `showWarningMessage`, `showErrorMessage`, `showQuickPick` (cancellation + `onDidSelectItem`), `showInputBox` (cancellation + `validateInput`), `activeTextEditor`, `visibleTextEditors`, `showTextDocument`, `TextEditorProxy`, `createOutputChannel` (`OutputChannelProxy`: `append`, `appendLine`, `replace`, `clear`, `show`, `hide`, `dispose`, `trace`/`debug`/`info`/`warn`/`error`), `createTerminal` (`name`, `show`, `sendText`, `dispose`), `createStatusBarItem` (`StatusBarItemProxy`), `createWebviewPanel`, `registerCustomEditorProvider`, `registerTreeDataProvider`, `createTreeView`, events (`onDidChangeActiveTextEditor`, `onDidChangeVisibleTextEditors`, `onDidChangeTextEditorSelection`). | **Wired managers (14 total on `LanguageFeatureBridge`):** `ExtHostWindowManager` (`window/mod.rs`: prompts, `cancelPrompt` TOCTOU guard, `active_editor`, `visible_editors`, `showTextDocument`), `ExtHostOutputManager`, `ExtHostTerminalManager`, `ExtStatusBarManager`, `ExtHostWebviewManager`, `ExtHostCustomEditorManager`, plus `ProviderType::TreeData` (aliases `'TreeDataProvider'`), `treeView.{getChildren, getTreeItem, onDidChangeTreeData}` with synthetic handles (`__rustcode_tree_<id>_<seq>`) (inbound `treeView.resolveTreeItem` is currently only a JS-side handler in `ext-host-js/rpc-bridge.js`).<br>**Pending UI / App wiring:** `RustCodeContext::create_window()` bridge attachment & `WorkbenchWindow` `window_manager`/`status_bar` sync (**M1.3**), `NotificationsOverlay` + interactive QuickPick/InputBox UI (**M3**), and contributed TreeView sidebar UI (**M4**). |
| `vscode.languages` (`languages.js`) | ✅ Full JS + Rust Bridge | `createDiagnosticCollection` (`DiagnosticCollection`: `set`, `delete`, `clear`, `dispose`), `registerCompletionItemProvider`, `registerHoverProvider`, `registerDefinitionProvider`, `registerDocumentFormattingEditProvider`, `registerCodeActionsProvider`, `format`, `codeAction` | Wired via `ProviderRegistry` (`Completion`, `Hover`, `Definition`, `References`, `DocumentFormatting`, `CodeAction`), `DiagnosticCollectionManager`, and `LanguageFeatureBridge` (aggregating extension providers with `LspClientManager`). |
| `vscode.notebooks` (`notebooks.js`) | ✅ Full JS + Rust Manager | `registerNotebookSerializer`, `createNotebookController`, `createNotebookCellExecution`; events (`onDid{Open,Close,Change,Save}NotebookDocument`, `onDidChangeActiveNotebookEditor`, `onDidChangeVisibleNotebookEditors`, `onDidChangeNotebookEditorSelection`); classes (`NotebookCellKind`, `NotebookCellOutputItem`, `NotebookCellOutput`, `NotebookCellData`, `NotebookData`, `NotebookRange`, `NotebookEdit`, `NotebookCellExecution`, `NotebookController`) | Wired to `ExtHostNotebookManager` (`notebook/mod.rs`), `ProviderType::{NotebookSerializer, NotebookController}`, and `NotebookEditorView` / `NotebookKernel` in `rustcode-workbench`. |
| `vscode.scm` (`scm.js`) | ✅ Full JS + Rust Manager | `createSourceControl`, `inputBox` getter, `onDidChangeActiveSourceControl`; `SourceControlInputBoxValidationType`, `SourceControlInputBox`, `SourceControlResourceGroup`, `SourceControl` | Wired to `ExtHostScmManager` (`scm/mod.rs`), `ProviderType::SourceControl`, inbound `scm.*` RPC handlers, and outbound `scm.provideOriginalResource` / `scm.acceptInput` requests; converts to `rustcode-protocols::git::{ScmRepository, GitStatusEntry}`. |
| `vscode.debug` (`debug.js`) | ✅ Full JS + Rust Manager | `activeDebugSession`, `activeDebugConsole`, `breakpoints`, `startDebugging`, `stopDebugging`, `registerDebugConfigurationProvider`, `registerDebugAdapterDescriptorFactory`, `registerDebugAdapterTrackerFactory`, `addBreakpoints`, `removeBreakpoints`, `asDebugSourceUri`, 5 debug events, `DebugConsoleMode`, `DebugConfigurationProviderTriggerKind`, `Breakpoint`/`SourceBreakpoint`/`FunctionBreakpoint`, `DebugAdapter{Executable,Server,NamedPipeServer,InlineImplementation}` | Wired to `ExtHostDebugManager` (`debug/mod.rs`), `ProviderType::{DebugConfiguration, DebugAdapterFactory}`, all `debug.*` RPC handlers, outbound `debug.{provideDebugConfigurations,resolveDebugConfiguration,createDebugAdapterDescriptor}`, and DAP type helpers. |
| `vscode.tasks` (`tasks.js`) | ✅ Full JS + Rust Manager | `taskExecutions`, `registerTaskProvider`, `fetchTasks`, `executeTask`, 4 task events, `TaskScope`, `TaskRevealKind`, `TaskPanelKind`, `TaskGroup`, `ProcessExecution`, `ShellExecution`, `CustomExecution`, `Task` | Wired to `ExtHostTaskManager` (`tasks/mod.rs`), `ProviderType::Task`, inbound `tasks.*` RPC handlers, and outbound `tasks.provideTasks` / `tasks.resolveTask`. |
| `vscode.tests` (`tests.js`) | ✅ Full JS + Rust Manager / 🟡 Sidebar UI Pending M3 | `createTestController`, `TestRunProfileKind`, `TestMessage`, `TestTag`, `TestRunRequest`, `TestItem`, `TestItemCollection`, `TestRun` (`onDidDispose`, `enqueued`, `started`, `skipped`, `passed`, `failed`, `errored`, `appendOutput`, `end`), `TestRunProfile`, `TestController` | Wired to `ExtHostTestManager` (`testing/mod.rs`), `ProviderType::TestController`, inbound `tests.*` RPC handlers, and outbound `tests.{resolveChildren,refreshTests,runTests}`. Interactive `TestingSidebarView` (`workbench.view.testing`) awaits **M3**. |
| `vscode.authentication` (`authentication.js`) | ✅ Full JS + Rust Manager | `getSession`, `getAccounts`, `registerAuthenticationProvider`, `onDidChangeSessions` | Wired to `ExtHostAuthManager` (`auth/mod.rs`), `ProviderType::Authentication`, inbound `authentication.*` RPC handlers, and outbound `authentication.{getSessions,createSession,removeSession}`. |
| `vscode.workspace` (`workspace.js`) | ✅ JS + Rust RPC Parity / 🟡 Pending M1.3 Wiring | `rootPath`, `workspaceFolders`, `getWorkspaceFolder`, `asRelativePath`, `getConfiguration` (`WorkspaceConfiguration`: `get`, `has`, `inspect`, `update`), `openTextDocument`, `saveAll`, `applyEdit`, `findFiles`, `textDocuments`, `fs` (`stat`, `readDirectory`, `createDirectory`, `readFile`, `writeFile`, `delete`, `rename`, `copy`, `isWritableFileSystem`), `TextDocumentProxy` (CRLF/UTF-16 `offsetAt`/`positionAt`/`lineAt`/`getText`/`validateRange`/`validatePosition`/`save`); text document events (`onDid{Open,Close,Change,Save}TextDocument`, `onDidChangeConfiguration` with `affectsConfiguration(section, scope)`) + 4 notebook document events | Wired in `LanguageFeatureBridge`: `workspace.{getConfiguration, inspectConfiguration, updateConfiguration, getWorkspaceFolders, getTextDocuments, openTextDocument, saveTextDocument, saveAll, applyEdit, findFiles}`, Windows `paths_equivalent` folder-config keys & `extract_scope_path` `Uri` normalization, `attach_configuration_listener()`, and `did_{open,change,save,close}_document`.<br>**Pending M1.3:** Automatic `WorkbenchWindow` document lifecycle forwarding & `RustCodeContext::create_window()` wiring. |
| `vscode.commands` (`commands.js`) | ✅ JS + Rust RPC Parity / 🟡 Builtin Effects Pending M1.3 | `registerCommand`, `executeCommand`, `getCommands(filterInternal = false)` | Bidirectional routing via `commands.{registerCommand, unregisterCommand, executeCommand, getCommands}` with `command_mutation_lock`, token-checked `CommandRegistry` disposables, `MAX_JS_COMMAND_DEPTH = 8` recursion guard, and outbound `commands.executeCommand` (`request_execute_command`).<br>**Pending M1.3:** Replace static JSON stubs in `RustCodeContext::register_builtin_commands()` with real `WorkbenchWindow` command effects. |
| `vscode.env` (`env.js`) | ✅ JS + Rust RPC Parity / 🟡 App Wiring & `getOsInfo` Pending M1.3/M5 | `appName` (`'RustCode'`), `appRoot`, `appHost`, `language`, `machineId`, `sessionId`, `shell`, `uriScheme`, `uiKind`, `clipboard.readText`, `clipboard.writeText`, `openExternal`, `refresh()`, plus `ExtensionContext` (`Memento`, `EnvironmentVariableCollection`, `asAbsolutePath`) | Wired to `ExtHostEnvManager` (`env/mod.rs`) handling `env.{getEnvironment, clipboardRead, clipboardWrite, openExternal}` with pluggable `ClipboardBackend` (`OsClipboardBackend` on Win/Linux/macOS + `InMemoryClipboardBackend`) and `ExternalUriOpener` (`OsExternalUriOpener` + `RecordingExternalUriOpener`, `is_valid_external_uri` allowlist).<br>**Pending:** `RustCodeContext::create_window()` OS backend wiring (**M1.3**) and `env.getOsInfo` (**M5**). |
| `vscode.extensions` (`env.js`) | ✅ JS + Rust RPC Parity / 🟡 App Wiring Pending M1.3 | `all`, `getExtension(extensionId)`, `onDidChange`, `Extension.activate()` (CJS `main` loading with `isPathInsideBase` traversal guard and `installVscodeModuleHook`) | Wired to `extensions.{getAll, getExtension, activate}` backed by shared `Arc<RwLock<ExtensionLoader>>` with short-name case-insensitive matching and `activate_extension`.<br>**Pending M1.3:** `RustCodeContext::create_window()` sharing of `ExtensionLoader` into `LanguageFeatureBridge`. |

### Node.js Compatibility Layer (`ext-host-js/node-compat/`)

| Module | Parity Level | Exports & Notes |
|---|---|---|
| `node-compat/events.js` | ✅ Full | CommonJS `EventEmitter`: `on`, `addListener`, `once`, `emit`, `removeListener`, `off`, `removeAllListeners` |
| `node-compat/fs.js` | ✅ Full Async + Callback / 🟡 Sync `ENOSYS` in Live Stdio | `constants`, `Dirent`, `promises.{readFile, writeFile, readdir, stat, mkdir, unlink, access, rm, rename, copyFile}`, 10 callback wrappers (`invokeCallback`), encodings (`utf8`, `ascii`, `latin1`, `base64`, `base64url`, `hex`, `utf16le`, `buffer`/`null`), `mkdir` `firstCreated` return value, backed by `fs.*` RPC handlers in `LanguageFeatureBridge` (`fs_root` sandbox, symlink limit 40, Win/Unix `map_io_error`, `EISDIR` checks). 11 `*Sync` methods exist (`existsSync` returns `false`, 10 others throw `ENOSYS` in live stdio without a sync host op). |
| `node-compat/path.js` | ✅ Full OS-Aware (Win32 + POSIX) | OS-aware `sep`/`delimiter`, `path.posix` & `path.win32`, `normalize`, `join`, `resolve` (including Windows drive-letter `resolve('C:foo')` & UNC `\\server\share`), `relative`, `dirname`, `basename`, `extname`, `isAbsolute`, `parse`, `format` |
| `node-compat/{buffer,url,util,os,crypto,child_process}.js` | ⏳ Planned (M5) | Not yet implemented; planned for Phase 7 **M5** alongside `env.getOsInfo` and `process.{spawn,exec,spawnSync}` RPC. |

---

## Extension Host Provider & Manager Architecture

### All 16 `ProviderType` Variants (`crates/rustcode-ext-host/src/provider/mod.rs`)

| # | `ProviderType` Variant | JS Proxy Name (`window.js` / `languages.js` / API modules) | Rust Manager / Handler |
|---|---|---|---|
| 1 | `Completion` | `'Completion'` (`languages.registerCompletionItemProvider`) | `LanguageFeatureBridge` + `ProviderRegistry` |
| 2 | `Hover` | `'Hover'` (`languages.registerHoverProvider`) | `LanguageFeatureBridge` + `ProviderRegistry` |
| 3 | `Definition` | `'Definition'` (`languages.registerDefinitionProvider`) | `LanguageFeatureBridge` + `ProviderRegistry` |
| 4 | `References` | `'References'` (registry type) | `ProviderRegistry` |
| 5 | `DocumentFormatting` | `'DocumentFormatting'` (`languages.registerDocumentFormattingEditProvider`) | `LanguageFeatureBridge` + `ProviderRegistry` |
| 6 | `CodeAction` | `'CodeAction'` (`languages.registerCodeActionsProvider`) | `LanguageFeatureBridge` + `ProviderRegistry` |
| 7 | `TreeData` | `'TreeDataProvider'` / `'TreeData'` (`window.registerTreeDataProvider`, `window.createTreeView`) | `ProviderType::from_str_name` aliases `'TreeDataProvider'` -> `TreeData`; `LanguageFeatureBridge::{request_tree_children_result, request_tree_item}` + `ProviderRegistry::notify_tree_data_changed` |
| 8 | `NotebookSerializer` | `'NotebookSerializer'` (`notebooks.registerNotebookSerializer`) | `ExtHostNotebookManager` |
| 9 | `NotebookController` | `'NotebookController'` (`notebooks.createNotebookController`) | `ExtHostNotebookManager` |
| 10 | `SourceControl` | `'SourceControl'` (`scm.createSourceControl`) | `ExtHostScmManager` |
| 11 | `DebugConfiguration` | `'DebugConfiguration'` (`debug.registerDebugConfigurationProvider`) | `ExtHostDebugManager` |
| 12 | `DebugAdapterFactory` | `'DebugAdapterFactory'` (`debug.registerDebugAdapterDescriptorFactory`) | `ExtHostDebugManager` |
| 13 | `Task` | `'Task'` (`tasks.registerTaskProvider`) | `ExtHostTaskManager` |
| 14 | `TestController` | `'TestController'` (`tests.createTestController`) | `ExtHostTestManager` |
| 15 | `Authentication` | `'Authentication'` (`authentication.registerAuthenticationProvider`) | `ExtHostAuthManager` |
| 16 | `CustomEditor` | `'CustomEditor'` (`window.registerCustomEditorProvider`) | `ExtHostCustomEditorManager` + `ExtHostWebviewManager` |

### Rust `ExtHost*Manager` & Bridge Managers (`crates/rustcode-ext-host/src/`)

All **14** manager structs are wired into `LanguageFeatureBridge` (`crates/rustcode-ext-host/src/provider/bridge.rs`) as `Arc<...>`: the 12 `ExtHost*Manager` structs manage state internally via `Arc<RwLock<...>>` (or internal synchronization), while `DiagnosticCollectionManager` (`RwLock<...>` + `Arc<Mutex<...>>`) and `ExtStatusBarManager` (`RwLock<...>`) are wrapped in `Arc<...>` on `LanguageFeatureBridge`:

1. **`DiagnosticCollectionManager`** (`provider/bridge.rs`) — `languages.{setDiagnostics, clearDiagnostics, clearAllDiagnostics}`
2. **`ExtStatusBarManager`** (`provider/bridge.rs`) — `window.{setStatusBarItem, hideStatusBarItem}`
3. **`ExtHostNotebookManager`** (`notebook/mod.rs`) — `notebooks.{registerSerializer, unregisterSerializer, createController, disposeController, updateExecution, updateCellOutputs, appendCellOutputs, updateControllerAffinity}` + outbound `notebooks.{deserializeNotebook, serializeNotebook, executeCells}`
4. **`ExtHostOutputManager`** (`output/mod.rs`) — `window.{createOutputChannel, appendOutputChannel, replaceOutputChannel, clearOutputChannel, showOutputChannel, hideOutputChannel, disposeOutputChannel}` (synced to `OutputPanelView`)
5. **`ExtHostTerminalManager`** (`terminal/mod.rs`) — `window.{createTerminal, showTerminal, sendTerminalText, disposeTerminal}` (synced to `TerminalManager` / `TerminalPanelView`)
6. **`ExtHostScmManager`** (`scm/mod.rs`) — `scm.{createSourceControl, updateSourceControl, disposeSourceControl, createResourceGroup, updateResourceGroup, disposeResourceGroup, updateInputBox}` + outbound `scm.{provideOriginalResource, acceptInput}`
7. **`ExtHostDebugManager`** (`debug/mod.rs`) — `debug.{registerConfigurationProvider, unregisterConfigurationProvider, registerAdapterDescriptorFactory, unregisterAdapterDescriptorFactory, registerAdapterTrackerFactory, unregisterAdapterTrackerFactory, startDebugging, stopDebugging, addBreakpoints, removeBreakpoints, appendConsole, sendCustomEvent}` + outbound `debug.{provideDebugConfigurations, resolveDebugConfiguration, createDebugAdapterDescriptor}`
8. **`ExtHostTaskManager`** (`tasks/mod.rs`) — `tasks.{registerProvider, unregisterProvider, fetchTasks, executeTask, terminateTask, reportTaskProcessStart, reportTaskProcessEnd}` + outbound `tasks.{provideTasks, resolveTask}`
9. **`ExtHostTestManager`** (`testing/mod.rs`) — `tests.{createController, disposeController, updateItems, createRunProfile, disposeRunProfile, createTestRun, updateTestState, appendRunOutput, endTestRun, invalidateTestResults}` + outbound `tests.{resolveChildren, refreshTests, runTests}`
10. **`ExtHostAuthManager`** (`auth/mod.rs`) — `authentication.{registerProvider, unregisterProvider, didChangeSessions, getSession, getAccounts}` + outbound `authentication.{getSessions, createSession, removeSession}`
11. **`ExtHostWebviewManager`** (`webview/mod.rs`) — `window.{createWebviewPanel, setWebviewHtml, postMessageToWebview, disposeWebviewPanel, revealWebviewPanel}` (synced to `WebviewEditorView`)
12. **`ExtHostCustomEditorManager`** (`webview/mod.rs`) — `window.{registerCustomEditorProvider, unregisterCustomEditorProvider, openCustomEditor}` + outbound `window.resolveCustomEditor` and `CustomEditor.{openCustomDocument, saveCustomDocument}` (plus inbound JS handlers in `rpc-bridge.js` for `window.{openCustomDocument, resolveCustomEditor, saveCustomDocument, saveCustomDocumentAs, revertCustomDocument, backupCustomDocument, closeCustomDocument}`)
13. **`ExtHostWindowManager`** (`window/mod.rs`) — `window.{showInformationMessage, showWarningMessage, showErrorMessage, showQuickPick, showInputBox, cancelPrompt, getActiveTextEditor, getVisibleTextEditors, showTextDocument}`, `NotificationResponder` / `PromptResponder` oneshot waiters + `cancelPrompt` TOCTOU guard, and `active_editor` / `visible_editors` state
14. **`ExtHostEnvManager`** (`env/mod.rs`) — `env.{getEnvironment, clipboardRead, clipboardWrite, openExternal}`, pluggable `ClipboardBackend` (`OsClipboardBackend` + `InMemoryClipboardBackend`) & `ExternalUriOpener` (`OsExternalUriOpener` + `RecordingExternalUriOpener` with `is_valid_external_uri` scheme allowlist)

`LanguageFeatureBridge` also coordinates shared `ConfigurationService`, `CommandRegistry` (with `command_mutation_lock`), `FileSystemService` (with `fs_root` sandbox & Win/Unix `map_io_error`), `ExtensionLoader`, `NodeExtHostProcess` live stdio RPC (`handle_rpc_message_result` via `RpcRequestHandler`), `extHost.{registerProvider, unregisterProvider}`, `languages.{format, codeAction}`, outbound language/tree/prompt requests, document lifecycle notifications (`did_{open,change,save,close}_document`, `did_change_active_editor`), `push_initial_state`, `activate_extension`, and `attach_configuration_listener`.

---

## Manifest Contribution Points (`crates/rustcode-ext-host/src/manifest/mod.rs`)

`ExtensionManifest::contributes` (`Contributions`) parses all **11** VS Code contribution fields:

| Contribution Field (`package.json`) | Rust Type | Purpose / Runtime Target |
|---|---|---|
| `commands` | `Option<Vec<ContributedCommand>>` | Registers contributed commands into `CommandRegistry` |
| `languages` | `Option<Vec<LanguageContribution>>` | Language IDs, file extensions, aliases |
| `grammars` | `Option<Vec<GrammarContribution>>` | TextMate grammar paths & scope names |
| `themes` | `Option<Vec<ThemeContribution>>` | Color themes loaded into `ThemeService` |
| `keybindings` | `Option<Vec<KeybindingContribution>>` | Keybindings & `when` clauses registered into `KeybindingService` |
| `configuration` | `Option<Value>` | Contributed settings schema |
| `views` | `Option<HashMap<String, Vec<ViewContribution>>>` | Sidebar / panel tree views |
| `debuggers` | `Option<Vec<DebuggerContribution>>` | Debug adapter types & labels |
| `taskDefinitions` | `Option<Vec<TaskDefinitionContribution>>` | Contributed task schemas |
| `authentication` | `Option<Vec<AuthenticationContribution>>` | Authentication provider declarations |
| `customEditors` | `Vec<CustomEditorContribution>` | `viewType`, `displayName`, `selector` (`filenamePattern`), `priority`; `CustomEditorContribution::matches_path(&self, path)` implements pure-Rust glob matching (`*`, `?`, `**/`, `/` & `\` normalization) wired to `WorkbenchWindow` & `EditorAreaView::open_file` |

---

## Tested Extensions & Compatibility Suite

### Real-World & Open VSX Curated Catalog (`crates/rustcode-ext-host/src/marketplace/catalog.rs`)

The pure-Rust Open VSX marketplace client (`MarketplaceClient`) and VSIX unpacker (`ExtensionDownloader` / `ExtensionInstaller` using `zip` + `flate2`) include an 8-extension offline `curated_catalog()`:
- `rust-lang.rust-analyzer` (`0.3.2100`), `ms-python.python` (`2024.16.1`), `esbenp.prettier-vscode` (`11.0.0`), `dbaeumer.vscode-eslint` (`3.0.10`), `PKief.material-icon-theme` (`5.14.1`), `enkia.tokyo-night` (`1.0.9`), `eamodio.gitlens` (`15.6.2`), `dracula-theme.theme-dracula` (`2.25.1`).

| Extension | Category | Status | Notes |
|---|---|---|---|
| One Dark Pro | Theme | ✅ Compatible | VS Code Color Theme JSON & TextMate scope format supported by `ThemeService::parse_theme_json` (verified via `theme_ext` fixture's `Oceanic Next`; not in `curated_catalog()`) |
| Monokai Pro | Theme | ✅ Compatible | Color and token scope JSON rules load via `ThemeService::parse_theme_json` (verified with `Monokai Classic` in `rustcode-platform::theme` tests) |
| Prettier (`prettier-vscode`) | Formatter | 🟡 Host APIs Ready | `languages.registerDocumentFormattingEditProvider` + `TextEdit` / `PieceTable` verified via `formatter_ext` fixture |
| ESLint (`vscode-eslint`) | Linter | 🟡 Host APIs Ready | `DiagnosticCollection` + `languages.registerCodeActionsProvider` + `WorkspaceEdit` verified via `linter_ext` & `codeactions_ext` fixtures |
| Rust-Analyzer (`rust-analyzer`) | Language Server | 🟡 LSP + Bridge Ready | Stdio JSON-RPC `LspClientManager` + `LanguageFeatureBridge` + UTF-16 alignment |

### Sample & Synthetic Fixture Extensions

- **Sample extension (`resources/sample-extensions/`):** `hello-world` (`sample-hello-world` v0.0.1 — live Node subprocess activation verified in `compat_suite.rs`)
- **Synthetic fixture extensions (`crates/rustcode-ext-host/tests/fixtures/extensions/` — 6 total):**
  1. `theme_ext` — Theme & manifest contributions
  2. `formatter_ext` — Document formatting edits & `PieceTable` application
  3. `codeactions_ext` — Quick-fix `CodeAction` + `WorkspaceEdit` transaction
  4. `treeview_ext` — Hierarchical tree items (`TreeDataProvider` / `TreeData` + `onDidChangeTreeData`)
  5. `statusbar_ext` — Status bar item lifecycle & priority sorting
  6. `linter_ext` — Diagnostic collection & severity mapping

### Verified Test Suites

- **Full Rust Workspace (`cargo test -j 2 --workspace`):** **457/457** tests passing across **8** crates (`rustcode-app` 25 [1 unit + 24 integration], `rustcode-base` 10, `rustcode-editor` 100, `rustcode-ext-host` 120 [79 unit + 41 `compat_suite`], `rustcode-platform` 40, `rustcode-protocols` 24, `rustcode-terminal` 9, `rustcode-workbench` 129), plus **0 `clippy` warnings** (`-D warnings`) on both **Windows (`x86_64-pc-windows-gnu`)** and **Linux (`x86_64-unknown-linux-gnu`)** cross-target clippy *(runtime tests executed on Windows 11 x64; Linux verified via cross-target `cargo clippy`)*.
- **`crates/rustcode-ext-host/tests/compat_suite.rs` (`41/41` integration tests passing):** Covers manifest contributions, activation lifecycle, themes, commands & bidirectional RPC, document formatting & `PieceTable`, code actions & `WorkspaceEdit`, tree view items, status bar lifecycle, linter diagnostics, full extension E2E workflow, notebooks, output channels, terminals, M2 APIs (`scm`, `debug`, `tasks`, `tests`, `authentication`), M4 custom editors & webviews, M1.1 live framed stdio subprocess & `fs.*`/`path` parity, and M1.2 live fixture E2E (`hello-world`, `treeview_ext`, `theme_ext`), `Uri`/config scoping, `ClipboardBackend`/`ExternalUriOpener`, `IncomingNotifWork::Barrier` causal ordering, and 40-iteration 4-worker stdio reverse-split stress tests.
- **`crates/rustcode-ext-host` unit tests (`79/79` passing):** Covers `auth` (3), `debug` (3), `env` (4), `manifest` (5), `marketplace` (14), `notebook` (2), `output` (1), `provider` / `provider::bridge` (14), `rpc` (14), `scm` (3), `tasks` (3), `terminal` (1), `testing` (3), `webview` (3), and `window` (6).
- **`ext-host-js/test.js` (`25/25` `[Test]` sections / `271` `assert(...)` call sites passing):** Covers `Position`/`Range`, `Uri` (including Windows/UNC), `node-compat/{events,path,fs}` (encodings, `Dirent`, `mkdir` `firstCreated`, `access`/`rm`/`rename`/`copyFile`, `*Sync` `ENOSYS` in live stdio), all `vscode.*` namespaces, `TextDocumentProxy` CRLF/UTF-16 helpers, `affectsConfiguration`, `require('vscode')` hook & `globalThis` singleton, and live stdio transport rounds. In environments without `node`, `compat_suite.rs` also verifies balanced JS delimiters (`assert_js_delimiters_balanced`) and required exports/handlers across `ext-host-js/`.
- **Standalone E2E Suite (`tests/phase7-e2e/`):** Isolated standalone Cargo `[workspace]` outside root `Cargo.toml` containing **286** Windows / **287** Unix opaque-box tests across Tiers 1–4 (`node tests/phase7-e2e/runner.js`; `--syntax-check` passes **8/8**), awaiting remaining Phase 7 milestones (`M1.3`, `M2–M5`).

---

## Phase 7 Parity Roadmap

### Completed in Phase 7 (`M1.1` + `M1.2`)
- [x] **M1.1 — Live Framed Stdio RPC Transport, Structured `RpcError` Envelope & `node-compat/{fs,path}.js` Parity**: `NodeExtHostProcess` (`ContentLengthJson`, `ContentLengthMsgPack`, `NewlineJson`, `from_io`, `JoinSet` half-close EOF draining, Windows `CREATE_NO_WINDOW`), O(N) `read_content_length_frame` (`64 KiB` / `64 MiB` bounds), structured `{ error: { code, message } }` (`RpcError`/`RpcErrorPayload`) across 12+ namespaces via `handle_rpc_message_result`, `node-compat/fs.js` (`Dirent`, encodings, `mkdir` `firstCreated`, `promises.{access,rm,rename,copyFile}`, `constants`, `invokeCallback`, 11 `*Sync` methods), OS-aware `node-compat/path.js` (`win32`/`posix`), and Win/Linux `map_io_error` + `EISDIR` parity.
- [x] **M1.2 — VS Code API Parity (`commands`, `window`, `workspace`, `env`, `extensions`) & Live Fixture E2E**: Sequential FIFO incoming notification worker + `IncomingNotifWork::Barrier`, `reader_closed` fast-fail, `command_mutation_lock` + token-checked `CommandRegistry` disposal, Windows folder-config equivalence & `extract_scope_path` `Uri` normalization + `affectsConfiguration(section, scope)`, CRLF/UTF-16 `TextDocumentProxy`, pluggable `ClipboardBackend`/`ExternalUriOpener`, `TreeDataProvider`/`TreeData` synthetic handles & `onDidChangeTreeData`, `require('vscode')` module hook + `globalThis` singleton, and live fixture + multi-threaded stress tests (`457` workspace tests, `41/41` `compat_suite`, `25/25` JS sections, `0` Win/Linux clippy warnings).

### Remaining Roadmap (`M1.3`, `M2–M5`, `M7`)
- [ ] **M1.3 — `RustCodeContext` & `WorkbenchWindow` End-to-End Wiring (Deferred / Next)**: Construct/attach shared `LanguageFeatureBridge` + `FileSystemService` in `RustCodeContext::create_window()`, route builtin commands to real `WorkbenchWindow` effects, forward document open/change/save/close & active-editor events, sync `ExtHostWindowManager`/`ExtHostEnvManager`/`ExtStatusBarManager`, and add Windows Explorer drive-root fallback.
- [ ] **M2 — In-Editor Find & Replace + QuickInput `:line[:col]` / `@symbol` (Not Started)**: Floating `FindReplaceWidget` in `EditorView` and interactive `:line[:col]` / `@symbol` QuickInput modes.
- [ ] **M3 — Notification Toast Overlay, Interactive Prompt UI & Testing Sidebar (Not Started)**: `NotificationsOverlay` + QuickPick/InputBox UI sync and `TestingSidebarView` (`workbench.view.testing`).
- [ ] **M4 — Interactive File Explorer CRUD & Contributed TreeViews (Not Started)**: Inline Explorer CRUD (`F2`/`Delete`/New File/Folder) and sidebar rendering for `contributes.views` + `ProviderType::TreeData`.
- [ ] **M5 — Expanded `node-compat` Polyfills & Language Grammars (Not Started)**: Add `ext-host-js/node-compat/{buffer,url,util,os,crypto,child_process}.js` + Rust `env.getOsInfo` / `process.{spawn,exec,spawnSync}` RPC, and 7 pure-Rust state-machine tokenizers (`Go`, `C`, `C++`, `HTML`, `CSS`, `TOML`, `YAML`).
- [ ] **M7 — Standalone E2E Suite Pass (`tests/phase7-e2e/`) & Tier 5 Hardening (Not Started)**: Pass all standalone Tiers 1–4 tests (`286` Win / `287` Unix) + Tier 5 adversarial coverage hardening.
