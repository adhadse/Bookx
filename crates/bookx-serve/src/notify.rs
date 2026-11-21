//! A change notice from the sync handlers to whoever owns the server (the app). The handlers run
//! on worker threads; the listener is called on that thread after the data is stored and the
//! store's lock is released, so it must be quick and must not block (the app only queues the
//! notice and wakes its main loop).

use std::sync::{Arc, Mutex};

/// What changed on the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Note {
    /// A device sent annotations that changed the store (never sent for Bookx's own publishing).
    Annotations {
        user: String,
        document: String,
        device: String,
        /// Server change counter after this request.
        seq: u64,
        highlights: usize,
        bookmarks: usize,
        notes: usize,
        removed: usize,
    },
    /// A device stored a reading position.
    Progress {
        user: String,
        document: String,
        device: String,
        device_id: String,
        percentage: f64,
        progress: String,
    },
}

pub type Listener = Arc<dyn Fn(&Note) + Send + Sync>;

/// Holds the one listener of a handler; cheap to leave unset.
#[derive(Default)]
pub struct Notifier {
    listener: Mutex<Option<Listener>>,
}

impl Notifier {
    /// Replaces the listener (`None` removes it).
    pub fn set(&self, listener: Option<Listener>) {
        *self.listener.lock().unwrap_or_else(|p| p.into_inner()) = listener;
    }

    /// Calls the listener, if any. A panic in it is not allowed to break the request.
    pub fn emit(&self, note: &Note) {
        let l = self
            .listener
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(l) = l {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| l(note)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn note() -> Note {
        Note::Progress {
            user: "u".into(),
            document: "d".into(),
            device: "k".into(),
            device_id: "i".into(),
            percentage: 0.5,
            progress: String::new(),
        }
    }

    #[test]
    fn no_listener_is_fine_and_a_listener_hears_every_note() {
        let n = Notifier::default();
        n.emit(&note());
        let c = Arc::new(AtomicUsize::new(0));
        let c2 = c.clone();
        n.set(Some(Arc::new(move |_| {
            c2.fetch_add(1, Ordering::SeqCst);
        })));
        n.emit(&note());
        n.emit(&note());
        assert_eq!(c.load(Ordering::SeqCst), 2);
        n.set(None);
        n.emit(&note());
        assert_eq!(c.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_panicking_listener_does_not_break_the_request() {
        let n = Notifier::default();
        n.set(Some(Arc::new(|_| panic!("boom"))));
        n.emit(&note());
    }
}
