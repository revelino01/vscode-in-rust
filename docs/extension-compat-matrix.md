# Extension Compatibility Matrix

This document tracks compatibility of the RustCode Extension Host against major VS Code extensions.

## Core API Namespaces

| Namespace | Status | Coverage | Notes |
|---|---|---|---|
| `vscode.commands` | ✅ Supported | 100% | `registerCommand`, `executeCommand`, `getCommands` |
| `vscode.window` | ✅ Supported | 85% | Messages, QuickPick, InputBox, StatusBarItem, Terminal |
| `vscode.workspace` | ✅ Supported | 80% | `getConfiguration`, document events, openTextDocument |
| `vscode.languages` | ✅ Supported | 75% | Completions, Hover, Definition, Formatting, Diagnostics |
| `vscode.env` | ✅ Supported | 90% | Clipboard, app properties, openExternal |
| `vscode.extensions` | ✅ Supported | 70% | Extension discovery and query |
| `vscode.scm` | 🟡 In Progress | 40% | SCM provider integration |
| `vscode.debug` | 🟡 In Progress | 35% | DAP bridge underway |

## Tested Extensions

| Extension | Category | Status | Notes |
|---|---|---|---|
| One Dark Pro | Theme | ✅ Compatible | JSON theme loader fully functional |
| Monokai Pro | Theme | ✅ Compatible | Color and token scopes load |
| Prettier | Formatter | 🟡 Ready for host | Uses `languages.registerDocumentFormattingEditProvider` |
| ESLint | Linter | 🟡 Ready for host | Uses diagnostics and code actions |
| Rust-Analyzer | Language Server| 🟡 In Progress | LSP JSON-RPC client integration |
