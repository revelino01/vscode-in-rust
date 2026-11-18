# Extension Compatibility Matrix

This document tracks compatibility of the RustCode Extension Host (`crates/rustcode-ext-host` + `ext-host-js/`) against the VS Code Extension API (`vscode.version = '1.95.0'`).

> **Runtime & Parity Note:** RustCode coordinates extension host state via a pure-Rust JSON/MessagePack `RpcBridge` (`crates/rustcode-ext-host/src/rpc/mod.rs`) and `LanguageFeatureBridge` (`crates/rustcode-ext-host/src/provider/bridge.rs`), paired with CommonJS shims in `ext-host-js/`. On environments without Node.js or `deno_core` (such as Termux), `ext-host-js/` is validated via static syntax/delimiter/export checks in `compat_suite.rs` alongside full Rust `RpcBridge` roundtrip tests. The tables below explicitly distinguish **full JS shim + Rust `LanguageFeatureBridge`/manager parity** from **JS-only `rpc.call(...)` stubs** whose RPC methods do not yet have match arms in `LanguageFeatureBridge::handle_rpc_message`.

---

## Core API Namespaces & Types (`vscode.*`)

| Namespace / Module | Parity Level | JS API Surface (`ext-host-js/vscode-api/`) | Rust Host Wiring (`crates/rustcode-ext-host/`) & Honest Notes |
|---|---|---|---|
| `vscode.EventEmitter` & Core Types (`types.js`, `index.js`) | ✅ Full JS types + Rust core parity | `vscode.version = '1.95.0'`; `Position`, `Range`, `Selection`, `Uri` (`file`, `parse`, `fsPath`), `Disposable`, `CancellationTokenSource`, `CancellationToken`, `EventEmitter` (`event`, `fire`, `dispose`), `DiagnosticSeverity`, `Diagnostic`, `CompletionItemKind`, `CompletionItem`, `Hover`, `StatusBarAlignment`, `ThemeColor`, `ThemeIcon`, `SnippetString`, `TextEdit`, `WorkspaceEdit`, `CodeActionKind`, `CodeActionTriggerKind`, `CodeAction` | Shared DTOs aligned with `rustcode-base` (`Emitter<T>`, `CancellationToken`, `Disposable`), `rustcode-editor` (`Position`, `Range`, `Selection`, `TextEdit`, `EditorDiagnostic`), and `rustcode-protocols` (`rustcode_protocols::lsp::WorkspaceEdit`). |
| `vscode.window` (`window.js`) | 🟢 Mixed (Managers wired / UI prompts JS-only) | **Wired:** `createOutputChannel` (`OutputChannelProxy`: `append`, `appendLine`, `replace`, `clear`, `show`, `hide`, `dispose`, `trace`/`debug`/`info`/`warn`/`error`), `createTerminal` (`name`, `show`, `sendText`, `dispose`), `createStatusBarItem` (`StatusBarItemProxy`), `createWebviewPanel`, `registerCustomEditorProvider`, `registerTreeDataProvider`, events (`onDidChangeActiveTextEditor`, `onDidChangeVisibleTextEditors`, `onDidChangeTextEditorSelection`).<br>**JS-only `rpc.call`:** `showInformationMessage`, `showWarningMessage`, `showErrorMessage`, `showQuickPick`, `showInputBox`. | **Wired managers:** `ExtHostOutputManager`, `ExtHostTerminalManager`, `ExtStatusBarManager`, `ExtHostWebviewManager`, `ExtHostCustomEditorManager` (plus `WorkbenchWindow` sync & `EditorAreaView` `WebviewEditorView` dispatch).<br>**Known deltas:** UI prompt calls (`show*Message`, `showQuickPick`, `showInputBox`) emit `rpc.call` without match arms in `LanguageFeatureBridge::handle_rpc_message`. Also, `window.js` registers proxy `'TreeDataProvider'` whereas `ProviderType::from_str_name` maps `'TreeData'`, and `bridge.rs` sends outbound `"treeView.getChildren"` which is not yet handled in `rpc-bridge.js:handleIncomingRequest`. |
| `vscode.languages` (`languages.js`) | ✅ Full JS + Rust Bridge | `createDiagnosticCollection` (`DiagnosticCollection`: `set`, `delete`, `clear`, `dispose`), `registerCompletionItemProvider`, `registerHoverProvider`, `registerDefinitionProvider`, `registerDocumentFormattingEditProvider`, `registerCodeActionsProvider`, `format`, `codeAction` | Wired via `ProviderRegistry` (`Completion`, `Hover`, `Definition`, `References`, `DocumentFormatting`, `CodeAction`), `DiagnosticCollectionManager`, and `LanguageFeatureBridge` (aggregating extension providers with `LspClientManager`). |
| `vscode.notebooks` (`notebooks.js`) | ✅ Full JS + Rust Manager | `registerNotebookSerializer`, `createNotebookController`, `createNotebookCellExecution`; events (`onDid{Open,Close,Change,Save}NotebookDocument`, `onDidChangeActiveNotebookEditor`, `onDidChangeVisibleNotebookEditors`, `onDidChangeNotebookEditorSelection`); classes (`NotebookCellKind`, `NotebookCellOutputItem`, `NotebookCellOutput`, `NotebookCellData`, `NotebookData`, `NotebookRange`, `NotebookEdit`, `NotebookCellExecution`, `NotebookController`) | Wired to `ExtHostNotebookManager` (`notebook/mod.rs`), `ProviderType::{NotebookSerializer, NotebookController}`, and `NotebookEditorView` / `NotebookKernel` in `rustcode-workbench`. |
| `vscode.scm` (`scm.js`) | ✅ Full JS + Rust Manager | `createSourceControl`, `inputBox` getter, `onDidChangeActiveSourceControl`; `SourceControlInputBoxValidationType`, `SourceControlInputBox`, `SourceControlResourceGroup`, `SourceControl` | Wired to `ExtHostScmManager` (`scm/mod.rs`), `ProviderType::SourceControl`, inbound `scm.*` RPC handlers, and outbound `scm.provideOriginalResource` / `scm.acceptInput` requests; converts to `rustcode-protocols::git::{ScmRepository, GitStatusEntry}`. |
| `vscode.debug` (`debug.js`) | ✅ Full JS + Rust Manager | `activeDebugSession`, `activeDebugConsole`, `breakpoints`, `startDebugging`, `stopDebugging`, `registerDebugConfigurationProvider`, `registerDebugAdapterDescriptorFactory`, `registerDebugAdapterTrackerFactory`, `addBreakpoints`, `removeBreakpoints`, `asDebugSourceUri`, 5 debug events, `DebugConsoleMode`, `DebugConfigurationProviderTriggerKind`, `Breakpoint`/`SourceBreakpoint`/`FunctionBreakpoint`, `DebugAdapter{Executable,Server,NamedPipeServer,InlineImplementation}` | Wired to `ExtHostDebugManager` (`debug/mod.rs`), `ProviderType::{DebugConfiguration, DebugAdapterFactory}`, all `debug.*` RPC handlers, outbound `debug.{provideDebugConfigurations,resolveDebugConfiguration,createDebugAdapterDescriptor}`, and DAP type helpers. |
| `vscode.tasks` (`tasks.js`) | ✅ Full JS + Rust Manager | `taskExecutions`, `registerTaskProvider`, `fetchTasks`, `executeTask`, 4 task events, `TaskScope`, `TaskRevealKind`, `TaskPanelKind`, `TaskGroup`, `ProcessExecution`, `ShellExecution`, `CustomExecution`, `Task` | Wired to `ExtHostTaskManager` (`tasks/mod.rs`), `ProviderType::Task`, inbound `tasks.*` RPC handlers, and outbound `tasks.provideTasks` / `tasks.resolveTask`. |
| `vscode.tests` (`tests.js`) | ✅ Full JS + Rust Manager | `createTestController`, `TestRunProfileKind`, `TestMessage`, `TestTag`, `TestRunRequest`, `TestItem`, `TestItemCollection`, `TestRun` (`onDidDispose`, `enqueued`, `started`, `skipped`, `passed`, `failed`, `errored`, `appendOutput`, `end`), `TestRunProfile`, `TestController` | Wired to `ExtHostTestManager` (`testing/mod.rs`), `ProviderType::TestController`, inbound `tests.*` RPC handlers, and outbound `tests.{resolveChildren,refreshTests,runTests}`. |
| `vscode.authentication` (`authentication.js`) | ✅ Full JS + Rust Manager | `getSession`, `getAccounts`, `registerAuthenticationProvider`, `onDidChangeSessions` | Wired to `ExtHostAuthManager` (`auth/mod.rs`), `ProviderType::Authentication`, inbound `authentication.*` RPC handlers, and outbound `authentication.{getSessions,createSession,removeSession}`. |
| `vscode.workspace` (`workspace.js`) | 🟡 Partial (Events wired / RPC calls JS-only) | `rootPath`, `workspaceFolders`, `getConfiguration` (`WorkspaceConfiguration`: `get`, `has`, `update`), `openTextDocument`; text document events (`onDid{Open,Close,Change,Save}TextDocument`, `onDidChangeConfiguration`) + 4 notebook document events (`onDid{Open,Close,Change,Save}NotebookDocument`) | Document lifecycle notifications (`did_open_document`, `did_change_document`, `did_close_document`) and `Workspace` / `ConfigurationService` exist in Rust; however `workspace.{getConfiguration,updateConfiguration,openTextDocument}` JS `rpc.call` methods do not yet have match arms in `LanguageFeatureBridge::handle_rpc_message`. |
| `vscode.commands` (`commands.js`) | 🟡 JS Shim + Rust Registry (RPC stub) | `registerCommand`, `executeCommand`, `getCommands(filterInternal = false)` | JS proxy invokes `commands.{registerCommand,unregisterCommand,executeCommand,getCommands}` via `rpc.call`; `CommandRegistry` exists in `rustcode-platform`, but `LanguageFeatureBridge::handle_rpc_message` does not yet route `commands.*` RPC calls directly (local command callback registration works inside `commands.js` + `rpc-bridge.js`). |
| `vscode.env` (`env.js`) | 🟡 JS Properties + RPC Stub | `appName` (`'RustCode'`), `appRoot` (`'/rustcode'`), `language` (`'en'`), `clipboard.readText`, `clipboard.writeText`, `openExternal` | Static properties are populated in JS; `env.{clipboardRead,clipboardWrite,openExternal}` emit `rpc.call` without Rust `LanguageFeatureBridge::handle_rpc_message` branches yet. |
| `vscode.extensions` (`env.js`) | 🟡 JS Shim Stub | `all` (`[]`), `getExtension(extensionId)` | Discovery & manifest loading live in Rust (`ExtensionLoader` / `ExtensionInstaller`), while `env.js` currently exposes an empty `all` array unless populated over RPC. |

### Node.js Compatibility Layer (`ext-host-js/node-compat/`)

| Module | Exports & Notes |
|---|---|
| `node-compat/events.js` | CommonJS `EventEmitter`: `on`, `addListener`, `once`, `emit`, `removeListener`, `off`, `removeAllListeners` |
| `node-compat/fs.js` | CommonJS `fs` shim: `promises.{readFile, writeFile, readdir, stat, mkdir, unlink}` and callback `readFile`/`writeFile` (delegates via `fs.*` `rpc.call`, currently JS-side stub without `LanguageFeatureBridge` branches) |
| `node-compat/path.js` | Pure-JS POSIX/path utilities: `sep`, `delimiter`, `normalize`, `join`, `resolve`, `dirname`, `basename`, `extname`, `isAbsolute` |

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
| 7 | `TreeData` | `'TreeDataProvider'` in `window.js:207` *(naming delta vs `'TreeData'` in Rust)* | `LanguageFeatureBridge::request_tree_children` |
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

All 12 manager structs are wired into `LanguageFeatureBridge` (`crates/rustcode-ext-host/src/provider/bridge.rs`) as `Arc<...>`: the 10 `ExtHost*Manager` structs manage state internally via `Arc<RwLock<...>>`, while `DiagnosticCollectionManager` (`RwLock<...>` + `Arc<Mutex<...>>`) and `ExtStatusBarManager` (`RwLock<...>`) are wrapped in `Arc<...>` on `LanguageFeatureBridge`:

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

`LanguageFeatureBridge` also handles `extHost.{registerProvider, unregisterProvider}`, `languages.{format, codeAction}`, outbound language queries (`languages.{provideCompletionItems, provideHover, provideDefinition, provideDocumentFormattingEdits, provideCodeActions}`), and document lifecycle notifications (`did_open_document`, `did_change_document`, `did_close_document`).

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

### Synthetic Fixture Extensions (`crates/rustcode-ext-host/tests/fixtures/extensions/`)

1. `theme_ext` — Theme & manifest contributions
2. `formatter_ext` — Document formatting edits & `PieceTable` application
3. `codeactions_ext` — Quick-fix `CodeAction` + `WorkspaceEdit` transaction
4. `treeview_ext` — Hierarchical tree items
5. `statusbar_ext` — Status bar item lifecycle & priority sorting
6. `linter_ext` — Diagnostic collection & severity mapping

### Verified Test Suites

- **`crates/rustcode-ext-host/tests/compat_suite.rs` (`26/26` integration tests passing):** Covers manifest contributions, activation lifecycle, themes, commands & bidirectional RPC, document formatting & `PieceTable`, code actions & `WorkspaceEdit`, tree view items, status bar lifecycle, linter diagnostics, full extension E2E workflow, notebooks (lifecycle, RPC roundtrip, JS surface), output channels (RPC lifecycle & JS surface), terminals (RPC lifecycle), M2 APIs (`scm`, `debug`, `tasks`, `tests`, `authentication`, M2 RPC roundtrip, M2 JS surface), and M4 custom editors & webviews (lifecycle, RPC roundtrip, JS surface).
- **`crates/rustcode-ext-host` unit tests (`48/48` passing):** Covers `auth` (3), `debug` (3), `manifest` (4), `marketplace` (14), `notebook` (2), `output` (1), `provider` (1), `provider::bridge` (5), `rpc` (2), `scm` (3), `tasks` (3), `terminal` (1), `testing` (3), and `webview` (3).
- **`ext-host-js/test.js` (`68` assertions across `15` `[Test]` sections):** Covers `Position`/`Range`, `Uri`, `node-compat/{events,path}`, `vscode` namespaces, `TextEdit`/`WorkspaceEdit`/`CodeAction`, `OutputChannel` & `Terminal`, `vscode.notebooks`, `vscode.EventEmitter`, `vscode.scm`, `vscode.debug`, `vscode.tasks`, `vscode.tests`, `vscode.authentication`, and `vscode.window` Webview & Custom Editors. In environments without `node`/`deno` (such as Termux), `compat_suite.rs` verifies balanced JS delimiters (`assert_js_delimiters_balanced`) and required exports/handlers across `ext-host-js/`.
