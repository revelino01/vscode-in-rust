//! `rustcode-base` — Layer 0: Core Foundation Utilities.
//!
//! Provides the core architectural building blocks mirroring VS Code's runtime model:
//! - Disposables (`Disposable`, `DisposableStore`)
//! - Typed Event Emitter (`Emitter`, `Event`)
//! - Cancellation Tokens (`CancellationTokenSource`, `CancellationToken`)
//! - VS Code `when` Clause Context Key Evaluator (`ContextKeyExpr`, `ContextKeyService`)

pub mod cancellation;
pub mod context_key;
pub mod disposable;
pub mod event;

pub use cancellation::{CancellationToken, CancellationTokenSource};
pub use context_key::{ContextKeyError, ContextKeyExpr, ContextKeyService};
pub use disposable::{ActionDisposable, ArcDisposable, Disposable, DisposableStore, to_disposable};
pub use event::{Emitter, Event};
