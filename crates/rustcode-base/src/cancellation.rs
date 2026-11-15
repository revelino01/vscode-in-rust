//! Cancellation token and source, matching VS Code's `CancellationToken`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::event::{Emitter, Event};

#[derive(Clone)]
pub struct CancellationToken {
    is_cancelled: Arc<AtomicBool>,
    on_cancelled: Event<()>,
}

impl CancellationToken {
    pub fn none() -> Self {
        let emitter = Emitter::<()>::new();
        Self {
            is_cancelled: Arc::new(AtomicBool::new(false)),
            on_cancelled: emitter.event(),
        }
    }

    pub fn is_cancellation_requested(&self) -> bool {
        self.is_cancelled.load(Ordering::SeqCst)
    }

    pub fn on_cancellation_requested(&self) -> Event<()> {
        self.on_cancelled.clone()
    }
}

pub struct CancellationTokenSource {
    token: CancellationToken,
    emitter: Emitter<()>,
}

impl Default for CancellationTokenSource {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationTokenSource {
    pub fn new() -> Self {
        let emitter = Emitter::<()>::new();
        let token = CancellationToken {
            is_cancelled: Arc::new(AtomicBool::new(false)),
            on_cancelled: emitter.event(),
        };
        Self { token, emitter }
    }

    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    pub fn cancel(&self) {
        if !self.token.is_cancelled.swap(true, Ordering::SeqCst) {
            self.emitter.fire(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancellation() {
        let source = CancellationTokenSource::new();
        let token = source.token();

        assert!(!token.is_cancellation_requested());

        let cancelled_flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&cancelled_flag);

        let _sub = token.on_cancellation_requested().attach(move |()| {
            flag_clone.store(true, Ordering::SeqCst);
        });

        source.cancel();
        assert!(token.is_cancellation_requested());
        assert!(cancelled_flag.load(Ordering::SeqCst));

        // Subsequent cancel does not re-fire
        source.cancel();
        assert!(token.is_cancellation_requested());
    }
}
