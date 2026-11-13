//! An offline queue for KOSync pushes. A position that could not be sent (no network, server
//! down) is kept on disk, one entry per book (the newest position wins), and retried with growing
//! pauses: 30 s, 1 min, 2 min, … up to 1 h. The queue is flushed when the application starts, on
//! a timer and when the network comes back. A rejected login or another answer of the server
//! that retrying cannot fix drops the entry instead of retrying for ever.
//!
//! This file is the queue and the rules; sending is done by the caller through [`Pusher`].

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use bookx_serve::kosync_client::Failure;
use serde::{Deserialize, Serialize};

/// Serialises read-modify-write cycles on the queue file inside the process.
static FILE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// The book file; one entry per file.
    pub path: String,
    pub document: String,
    pub progress: String,
    pub percentage: f64,
    pub device: String,
    pub device_id: String,
    pub queued_at: i64,
    pub attempts: u32,
    pub next_try: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Queue {
    pub entries: Vec<Entry>,
}

/// Seconds to wait after the `attempts`-th failure: 30 s doubling up to an hour.
pub fn backoff(attempts: u32) -> i64 {
    (30i64 << attempts.saturating_sub(1).min(7)).min(3600)
}

/// Whether trying again later can help: not for a refused login or another HTTP answer.
pub fn is_transient(e: &anyhow::Error) -> bool {
    match e.downcast_ref::<Failure>() {
        Some(Failure::Status(s)) => *s >= 500 || *s == 429 || *s == 408,
        Some(_) => false,
        None => true,
    }
}

impl Queue {
    pub fn load(path: &Path) -> Queue {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if self.entries.is_empty() {
            let _ = std::fs::remove_file(path);
            return Ok(());
        }
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Queues a position; replaces the earlier one of the same book (and tries soon again).
    pub fn push(&mut self, mut e: Entry) {
        match self.entries.iter_mut().find(|x| x.path == e.path) {
            Some(old) => {
                e.attempts = old.attempts;
                e.next_try = old.next_try.max(e.queued_at);
                *old = e;
            }
            None => self.entries.push(e),
        }
    }

    /// Entries whose turn has come.
    pub fn due(&self, now: i64) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|e| e.next_try <= now)
            .cloned()
            .collect()
    }

    /// After a failed try: wait longer next time.
    pub fn failed(&mut self, path: &str, now: i64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.path == path) {
            e.attempts += 1;
            e.next_try = now + backoff(e.attempts);
        }
    }

    /// After a successful (or given-up) try. Only removes the entry if its position is still the
    /// one that was sent, so a newer position queued meanwhile stays.
    pub fn done(&mut self, path: &str, percentage: f64) {
        self.entries
            .retain(|e| !(e.path == path && (e.percentage - percentage).abs() < 1e-9));
    }

    /// Drops the entry of a book (its position was sent by a normal push).
    pub fn forget(&mut self, path: &str) {
        self.entries.retain(|e| e.path != path);
    }
}

/// Where the queue lives (`$BOOKX_KOSYNC_QUEUE` for tests).
pub fn queue_path() -> Option<PathBuf> {
    std::env::var_os("BOOKX_KOSYNC_QUEUE")
        .map(PathBuf::from)
        .or_else(|| {
            directories::ProjectDirs::from("com", "adhadse", "Bookx")
                .map(|d| d.config_dir().join("kosync-queue.json"))
        })
}

/// Read-modify-write of the queue file under the process lock.
pub fn update<R>(path: &Path, f: impl FnOnce(&mut Queue) -> R) -> R {
    let _g = FILE.lock().unwrap_or_else(|p| p.into_inner());
    let mut q = Queue::load(path);
    let r = f(&mut q);
    let _ = q.save(path);
    r
}

/// How [`flush`] reaches the server.
pub trait Pusher {
    /// `Ok(true)`: sent. `Ok(false)`: not sent on purpose (another device is further along).
    fn push(&self, e: &Entry) -> Result<bool>;
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Flushed {
    pub sent: usize,
    pub kept: usize,
    pub dropped: usize,
}

/// Tries every due entry once. Stops at the first transient failure (the network is probably
/// down for all of them) after scheduling that entry's retry.
pub fn flush(path: &Path, now: i64, pusher: &dyn Pusher) -> Flushed {
    let due = update(path, |q| q.due(now));
    let mut out = Flushed::default();
    for e in due {
        match pusher.push(&e) {
            Ok(sent) => {
                update(path, |q| q.done(&e.path, e.percentage));
                if sent {
                    out.sent += 1;
                } else {
                    out.dropped += 1;
                }
            }
            Err(err) if is_transient(&err) => {
                update(path, |q| q.failed(&e.path, now));
                out.kept += 1;
                break;
            }
            Err(_) => {
                update(path, |q| q.done(&e.path, e.percentage));
                out.dropped += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn entry(path: &str, pct: f64, at: i64) -> Entry {
        Entry {
            path: path.into(),
            document: format!("doc-{path}"),
            progress: "/body/DocFragment[3]/body".into(),
            percentage: pct,
            device: "Bookx".into(),
            device_id: "dev".into(),
            queued_at: at,
            attempts: 0,
            next_try: at,
        }
    }

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("bookx-kq-{}/q.json", uuid::Uuid::new_v4()))
    }

    struct Fake(RefCell<Vec<String>>, Box<dyn Fn(&Entry) -> Result<bool>>);
    impl Pusher for Fake {
        fn push(&self, e: &Entry) -> Result<bool> {
            self.0.borrow_mut().push(e.path.clone());
            (self.1)(e)
        }
    }

    #[test]
    fn the_pause_doubles_from_thirty_seconds_to_an_hour() {
        let v: Vec<i64> = (1..=9).map(backoff).collect();
        assert_eq!(v, vec![30, 60, 120, 240, 480, 960, 1920, 3600, 3600]);
    }

    #[test]
    fn the_newest_position_of_a_book_replaces_the_older_one_and_keeps_the_retry_clock() {
        let mut q = Queue::default();
        q.push(entry("a", 0.2, 100));
        q.failed("a", 100);
        q.push(entry("a", 0.4, 110));
        q.push(entry("b", 0.1, 111));
        assert_eq!(q.entries.len(), 2);
        let a = &q.entries[0];
        assert_eq!((a.percentage, a.attempts), (0.4, 1));
        assert!(a.next_try >= 130, "still waiting after the failed try");
        assert_eq!(q.due(120).len(), 1);
        q.done("a", 0.2);
        assert_eq!(
            q.entries.len(),
            2,
            "a newer position than the one sent stays"
        );
        q.done("a", 0.4);
        assert_eq!(q.entries.len(), 1);
    }

    #[test]
    fn the_queue_survives_a_restart_and_the_empty_queue_leaves_no_file() {
        let p = tmp();
        update(&p, |q| q.push(entry("a", 0.5, 1)));
        assert_eq!(Queue::load(&p).entries[0].percentage, 0.5);
        update(&p, |q| q.done("a", 0.5));
        assert!(!p.exists());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn a_flush_sends_what_is_due_and_empties_the_queue() {
        let p = tmp();
        update(&p, |q| {
            q.push(entry("a", 0.5, 1));
            q.push(entry("b", 0.7, 1));
        });
        let f = Fake(RefCell::new(vec![]), Box::new(|_| Ok(true)));
        assert_eq!(
            flush(&p, 10, &f),
            Flushed {
                sent: 2,
                kept: 0,
                dropped: 0
            }
        );
        assert!(Queue::load(&p).entries.is_empty());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn a_network_failure_keeps_the_entry_with_backoff_and_stops_the_run() {
        let p = tmp();
        update(&p, |q| {
            q.push(entry("a", 0.5, 1));
            q.push(entry("b", 0.7, 1));
        });
        let f = Fake(
            RefCell::new(vec![]),
            Box::new(|_| Err(anyhow::anyhow!("connecting to host: refused"))),
        );
        assert_eq!(
            flush(&p, 10, &f),
            Flushed {
                sent: 0,
                kept: 1,
                dropped: 0
            }
        );
        assert_eq!(f.0.borrow().len(), 1, "no point trying the second one");
        let q = Queue::load(&p);
        assert_eq!(q.entries.len(), 2);
        assert_eq!(q.entries[0].attempts, 1);
        assert_eq!(q.entries[0].next_try, 40);
        assert_eq!(flush(&p, 20, &f).kept, 1, "a is waiting, b is tried");
        assert_eq!(f.0.borrow().as_slice(), ["a", "b"]);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn a_refused_login_or_a_held_back_push_is_dropped_not_retried_for_ever() {
        let p = tmp();
        update(&p, |q| {
            q.push(entry("a", 0.5, 1));
            q.push(entry("b", 0.7, 1));
        });
        let f = Fake(
            RefCell::new(vec![]),
            Box::new(|e| {
                if e.path == "a" {
                    Err(Failure::BadCredentials.into())
                } else {
                    Ok(false)
                }
            }),
        );
        assert_eq!(
            flush(&p, 10, &f),
            Flushed {
                sent: 0,
                kept: 0,
                dropped: 2
            }
        );
        assert!(!p.exists());
        assert!(is_transient(&Failure::Status(503).into()));
        assert!(!is_transient(&Failure::Status(404).into()));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
