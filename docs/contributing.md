# Contributing to RustCode

Welcome! RustCode is a community-driven project to build the fastest, lightest VS Code compatible editor in Rust.

## Development Setup

1. **Prerequisites**:
   - Rust toolchain (`rustc`, `cargo` 1.80+)
   - `git`

2. **Clone & Build**:
   ```bash
   git clone https://github.com/revelino01/vscode-in-rust.git
   cd vscode-in-rust
   cargo test --workspace
   cargo run -p rustcode-app
   ```

3. **Crate Guidelines**:
   - `rustcode-base`: Zero external heavy dependencies. Keep primitives clean.
   - `rustcode-editor`: Piece Table algorithms must maintain O(log n) complexity and full unit test coverage.
   - `rustcode-ext-host`: Ensure MessagePack and JSON serialization remain binary-compatible with the JS shim.

## Code Style

- Follow standard `cargo fmt` and `cargo clippy`.
- Keep diffs focused and cleanly documented.
