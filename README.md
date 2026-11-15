# RustCode (vscode-in-rust)

> [!NOTE]
> ⚡ **Vibe Coded Repository**: This repository and its entire codebase are **100% vibe coded** with autonomous AI agent workflows. Pure vibes, modern Rust, and blazing-fast performance.

A ground-up, high-performance Rust rewrite of Visual Studio Code. **RustCode** aims to deliver the exact look, feel, and extension ecosystem of VS Code while consuming ~3× less RAM and launching in milliseconds with a native GPU-accelerated interface.

---

## 🎯 Vision & Goals

- **Familiar Experience**: Exact keybindings (`keybindings.json`), settings (`settings.json`), themes, command palette (`Ctrl+Shift+P`), and workbench layout.
- **Extension Compatibility**: Run real JS/TS VS Code extensions unmodified through an embedded V8 extension host with Node.js compatibility ops and a `vscode.*` API shim.
- **Lightweight & Blazing Fast**:
  - **Cold Start**: < 500 ms (vs. 2–5s in Electron)
  - **Idle RAM**: < 120 MB (vs. 300–400 MB in Electron)
  - **Frame Rate**: Smooth 120 FPS via native GPU rendering
  - **Keystroke Latency**: < 3 ms

---

## 🏗️ Architecture

| Component | Choice | Description |
|---|---|---|
| **GUI Framework** | **GPUI** / GPU Scene Graph | 120 FPS hardware-accelerated text and UI rendering |
| **Extension Host** | **Embedded V8 (`deno_core`)** | Runs JavaScript extensions with full JIT speed and isolated memory |
| **IPC Protocol** | **MessagePack RPC (`rmp-serde`)** | Fast binary RPC between the Rust Core and Extension Host |
| **Text Buffer** | **Custom Piece Table** | Red-Black tree piece table with O(log n) inserts, deletes, and offset-to-position mapping |
| **Syntax Highlighting**| **Tree-sitter** + TextMate | Incremental syntax trees with VS Code theme scope mapping |
| **Terminal** | **`alacritty_terminal`** | Industry-standard VT100 / xterm terminal emulation |

---

## 📦 Workspace Crates

```
vscode-in-rust/
├── crates/
│   ├── rustcode-base/          # Zero-dependency foundation (disposables, events, cancellation, when-clauses)
│   ├── rustcode-platform/      # Platform services (configuration, keybindings, commands, themes)
│   ├── rustcode-editor/        # Text editing engine (Piece Table, undo/redo, cursors, selections)
│   ├── rustcode-terminal/      # Integrated terminal manager and ANSI buffer
│   ├── rustcode-protocols/     # LSP, DAP, and Git SCM protocol definitions
│   ├── rustcode-ext-host/      # Extension host manager, manifest loader, and RPC bridge
│   ├── rustcode-workbench/     # Workbench layout shell (activity bar, sidebars, tabs, command palette, status bar)
│   └── rustcode-app/           # Main application bootstrap and service container
├── ext-host-js/                # JavaScript vscode.* API shim and Node.js compat polyfills
├── resources/                  # Default themes (Dark+, Light+), keymaps, and sample extensions
├── tests/                      # Integration and stress test suites
└── rustcode-implementation-plan.md # Master implementation plan & progress tracker
```

---

## 🚦 Progress & Roadmap

Track detailed milestone checklists in [rustcode-implementation-plan.md](rustcode-implementation-plan.md).

- [x] **Repository Scaffolding & Architecture Plan**
- [x] **`rustcode-base`**: Disposables, typed event emitter, cancellation tokens, VS Code `when` clause evaluator
- [x] **`rustcode-editor`**: Piece Table buffer with Red-Black tree, transactional undo/redo stack, multi-cursor navigation
- [x] **`rustcode-platform`**: JSONC configuration service, multi-chord keybinding resolver, command registry, VS Code theme parser
- [x] **`rustcode-ext-host`**: Extension manifest parser (`package.json`), MessagePack RPC bridge, provider callback registry
- [x] **`rustcode-workbench`**: Shell layout models (activity bar, explorer tree, editor groups & tabs, fuzzy command palette, status bar)
- [x] **`rustcode-protocols` & `terminal`**: LSP client types, Git SCM state model, terminal PTY and escape buffer
- [x] **`ext-host-js`**: `vscode.*` API JavaScript shim (`commands`, `window`, `workspace`, `languages`, `types`)
- [ ] **Full GPUI Window Rendering & Vulkan/Metal Surface Integration**
- [ ] **V8 JIT Runtime Binding with `deno_core`**
- [ ] **Full Open VSX Marketplace Integration**

---

## 🧪 Building & Testing

```bash
# Clone the repository
git clone https://github.com/revelino01/vscode-in-rust.git
cd vscode-in-rust

# Run all crate unit tests
cargo test --workspace

# Run Piece Table stress tests
cargo test -p rustcode-editor -- piece_table
```

---

## 📄 License

MIT OR Apache-2.0
