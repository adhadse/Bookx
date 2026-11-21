//! Limits how many conversions run at the same time (C10). Converting a long list of books on
//! every core at once makes the machine unusable and uses a lot of memory (each book is held
//! several times while it is converted), so the app runs at most N at a time.

use std::sync::{Arc, Condvar, Mutex};

/// How many conversions run at a time when the user has not chosen: half the processors, at
/// least one.
pub fn default_parallel(cpus: usize) -> usize {
    (cpus / 2).max(1)
}

/// The largest number the setting accepts.
pub const MAX_PARALLEL: usize = 64;

/// Keeps a setting inside its range.
pub fn clamp_parallel(n: usize) -> usize {
    n.clamp(1, MAX_PARALLEL)
}

#[derive(Debug)]
struct Inner {
    limit: usize,
    running: usize,
}

/// A counting gate: [`Limiter::acquire`] waits until fewer than `limit` permits are out. The
/// limit can change while jobs wait.
#[derive(Debug, Clone)]
pub struct Limiter {
    shared: Arc<(Mutex<Inner>, Condvar)>,
}

/// A running conversion; the slot is free again when this is dropped.
#[derive(Debug)]
pub struct Permit {
    shared: Arc<(Mutex<Inner>, Condvar)>,
}

impl Limiter {
    pub fn new(limit: usize) -> Limiter {
        Limiter {
            shared: Arc::new((
                Mutex::new(Inner {
                    limit: clamp_parallel(limit),
                    running: 0,
                }),
                Condvar::new(),
            )),
        }
    }

    /// Changes the limit; waiting jobs are woken to check it.
    pub fn set_limit(&self, limit: usize) {
        let (m, cv) = &*self.shared;
        m.lock().unwrap_or_else(|e| e.into_inner()).limit = clamp_parallel(limit);
        cv.notify_all();
    }

    pub fn limit(&self) -> usize {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .limit
    }

    /// How many permits are out.
    pub fn running(&self) -> usize {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .running
    }

    /// Waits for a free slot.
    pub fn acquire(&self) -> Permit {
        let (m, cv) = &*self.shared;
        let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
        while g.running >= g.limit {
            g = cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        g.running += 1;
        Permit {
            shared: self.shared.clone(),
        }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let (m, cv) = &*self.shared;
        let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
        g.running = g.running.saturating_sub(1);
        cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn the_default_is_half_the_processors_and_never_zero() {
        assert_eq!(default_parallel(16), 8);
        assert_eq!(default_parallel(2), 1);
        assert_eq!(default_parallel(1), 1);
        assert_eq!(default_parallel(0), 1);
        assert_eq!(clamp_parallel(0), 1);
        assert_eq!(clamp_parallel(10_000), MAX_PARALLEL);
    }

    #[test]
    fn never_more_than_the_limit_run_at_once() {
        let l = Limiter::new(3);
        let now = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..20)
            .map(|_| {
                let (l, now, peak) = (l.clone(), now.clone(), peak.clone());
                std::thread::spawn(move || {
                    let _p = l.acquire();
                    let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(n, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(5));
                    now.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= 3);
        assert!(peak.load(Ordering::SeqCst) >= 2, "they do run in parallel");
        assert_eq!(l.running(), 0);
    }

    #[test]
    fn a_waiting_job_goes_on_when_the_limit_is_raised() {
        let l = Limiter::new(1);
        let first = l.acquire();
        let l2 = l.clone();
        let t = std::thread::spawn(move || {
            let _p = l2.acquire();
        });
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(l.running(), 1);
        l.set_limit(2);
        t.join().unwrap();
        drop(first);
        assert_eq!(l.running(), 0);
        assert_eq!(l.limit(), 2);
    }
}
