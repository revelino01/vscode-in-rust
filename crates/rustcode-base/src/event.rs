//! Typed event emitter mirroring VS Code's `Emitter<T>` and `Event<T>`.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::disposable::{Disposable, ActionDisposable};

type Listener<T> = Arc<dyn Fn(&T) + Send + Sync + 'static>;

struct SharedEmitterState<T> {
    listeners: HashMap<u64, Listener<T>>,
}

/// An emitter that produces events of type `T`.
pub struct Emitter<T: Clone + Send + 'static> {
    state: Arc<Mutex<SharedEmitterState<T>>>,
    next_id: Arc<AtomicU64>,
}

impl<T: Clone + Send + 'static> Default for Emitter<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Send + 'static> Emitter<T> {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(SharedEmitterState {
                listeners: HashMap::new(),
            })),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Obtain an `Event<T>` subscribable view of this emitter.
    pub fn event(&self) -> Event<T> {
        Event {
            state: Arc::clone(&self.state),
            next_id: Arc::clone(&self.next_id),
        }
    }

    /// Fire the event to all registered listeners without holding the lock during invocation.
    pub fn fire(&self, value: T) {
        let listeners: Vec<Listener<T>> = {
            let guard = self.state.lock();
            guard.listeners.values().cloned().collect()
        };
        for listener in listeners {
            listener(&value);
        }
    }

    /// Check if there are active listeners.
    pub fn has_listeners(&self) -> bool {
        !self.state.lock().listeners.is_empty()
    }
}

/// Subscribable event handle (mirrors VS Code's `Event<T>`).
#[derive(Clone)]
pub struct Event<T: Clone + Send + 'static> {
    state: Arc<Mutex<SharedEmitterState<T>>>,
    next_id: Arc<AtomicU64>,
}

impl<T: Clone + Send + 'static> Event<T> {
    /// Attach a listener callback. Returns a `Disposable` that unsubscribes when disposed.
    pub fn attach<F>(&self, listener: F) -> Box<dyn Disposable>
    where
        F: Fn(&T) + Send + Sync + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut guard = self.state.lock();
            guard.listeners.insert(id, Arc::new(listener));
        }

        let state = Arc::clone(&self.state);
        Box::new(ActionDisposable::new(move || {
            let mut guard = state.lock();
            guard.listeners.remove(&id);
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emitter_and_event() {
        let emitter = Emitter::<String>::new();
        let event = emitter.event();

        let received = Arc::new(Mutex::new(Vec::new()));
        let r_clone = Arc::clone(&received);

        let mut sub = event.attach(move |val| {
            r_clone.lock().push(val.clone());
        });

        emitter.fire("first".to_string());
        emitter.fire("second".to_string());

        assert_eq!(*received.lock(), vec!["first", "second"]);

        // Dispose subscription
        sub.dispose();
        emitter.fire("third".to_string());

        assert_eq!(*received.lock(), vec!["first", "second"]);
    }

    #[test]
    fn test_emitter_reentrant_dispose_during_fire() {
        let emitter = Emitter::<i32>::new();
        let event = emitter.event();

        let sub_holder: Arc<Mutex<Option<Box<dyn Disposable>>>> = Arc::new(Mutex::new(None));
        let sub_clone = Arc::clone(&sub_holder);
        let count = Arc::new(Mutex::new(0));
        let count_clone = Arc::clone(&count);

        let sub = event.attach(move |val| {
            *count_clone.lock() += val;
            // Dispose self inside the callback — must not deadlock!
            if let Some(mut s) = sub_clone.lock().take() {
                s.dispose();
            }
        });

        *sub_holder.lock() = Some(sub);

        // Firing should succeed cleanly and unregister the listener
        emitter.fire(10);
        assert_eq!(*count.lock(), 10);

        // Subsequent fires should not reach the listener
        emitter.fire(20);
        assert_eq!(*count.lock(), 10);
    }
}
