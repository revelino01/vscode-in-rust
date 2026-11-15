//! Disposable pattern mirroring VS Code's `IDisposable`.
//!
//! Resources that allocate handles, subscriptions, or watchers implement `Disposable`.
//! `DisposableStore` aggregates multiple disposables and cleans them up in reverse order.

use parking_lot::Mutex;
use std::sync::Arc;

pub trait Disposable: Send + Sync {
    fn dispose(&mut self);
}

/// Collects multiple disposables and disposes all on drop or explicit call.
pub struct DisposableStore {
    items: Vec<Box<dyn Disposable>>,
    is_disposed: bool,
}

impl Default for DisposableStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DisposableStore {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            is_disposed: false,
        }
    }

    /// Add a disposable item into this store.
    pub fn add<D: Disposable + 'static>(&mut self, item: D) {
        if self.is_disposed {
            let mut item = item;
            item.dispose();
        } else {
            self.items.push(Box::new(item));
        }
    }

    /// Add a boxed disposable into this store.
    pub fn add_boxed(&mut self, mut item: Box<dyn Disposable>) {
        if self.is_disposed {
            item.dispose();
        } else {
            self.items.push(item);
        }
    }

    /// Dispose all managed resources in reverse insertion order.
    pub fn clear_and_dispose(&mut self) {
        for mut item in self.items.drain(..).rev() {
            item.dispose();
        }
    }

    pub fn is_disposed(&self) -> bool {
        self.is_disposed
    }
}

impl Disposable for DisposableStore {
    fn dispose(&mut self) {
        if !self.is_disposed {
            self.clear_and_dispose();
            self.is_disposed = true;
        }
    }
}

impl Drop for DisposableStore {
    fn drop(&mut self) {
        self.dispose();
    }
}

/// A disposable backed by a closure action.
pub struct ActionDisposable<F: FnMut() + Send + Sync + 'static> {
    action: Option<F>,
}

impl<F: FnMut() + Send + Sync + 'static> ActionDisposable<F> {
    pub fn new(action: F) -> Self {
        Self {
            action: Some(action),
        }
    }
}

impl<F: FnMut() + Send + Sync + 'static> Disposable for ActionDisposable<F> {
    fn dispose(&mut self) {
        if let Some(mut action) = self.action.take() {
            action();
        }
    }
}

impl<F: FnMut() + Send + Sync + 'static> Drop for ActionDisposable<F> {
    fn drop(&mut self) {
        self.dispose();
    }
}

/// Helper function to create an `ActionDisposable` boxed.
pub fn to_disposable<F: FnMut() + Send + Sync + 'static>(action: F) -> Box<dyn Disposable> {
    Box::new(ActionDisposable::new(action))
}

/// Thread-safe reference-counted disposable handle.
#[derive(Clone)]
pub struct ArcDisposable {
    inner: Arc<Mutex<Option<Box<dyn Disposable>>>>,
}

impl ArcDisposable {
    pub fn new<D: Disposable + 'static>(item: D) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(Box::new(item)))),
        }
    }

    pub fn from_boxed(item: Box<dyn Disposable>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(item))),
        }
    }
}

impl Disposable for ArcDisposable {
    fn dispose(&mut self) {
        let mut guard = self.inner.lock();
        if let Some(mut item) = guard.take() {
            item.dispose();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_disposable_store_disposes_in_reverse() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c1 = Arc::clone(&counter);
        let c2 = Arc::clone(&counter);

        let mut store = DisposableStore::new();
        store.add(ActionDisposable::new(move || {
            c1.store(1, Ordering::SeqCst);
        }));
        store.add(ActionDisposable::new(move || {
            c2.store(2, Ordering::SeqCst);
        }));

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        store.dispose();
        // Since c2 was added second, c2 disposes first (sets to 2), then c1 disposes (sets to 1)
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(store.is_disposed());
    }

    #[test]
    fn test_store_drops_on_scope_exit() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&counter);
        {
            let mut store = DisposableStore::new();
            store.add(ActionDisposable::new(move || {
                c.store(42, Ordering::SeqCst);
            }));
        }
        assert_eq!(counter.load(Ordering::SeqCst), 42);
    }
}
