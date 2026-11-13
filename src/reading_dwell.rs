//! Time spent on each part of the open book, kept for the book map (`bookx_lookup::dwell`). One
//! store per process, loaded on first use and written at most every half minute and when a book
//! is closed. A damaged file is set aside and replaced by an empty store.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bookx_lookup::dwell::DwellStore;

struct Hub {
    store: DwellStore,
    path: Option<PathBuf>,
    dirty: bool,
    saved: Instant,
}

thread_local! {
    static HUB: RefCell<Option<Hub>> = const { RefCell::new(None) };
}

fn load(path: Option<PathBuf>) -> Hub {
    let store = match path.as_deref().map(DwellStore::load) {
        Some(Ok(s)) => s,
        Some(Err(_)) => {
            if let Some(p) = &path {
                let _ = std::fs::rename(p, p.with_extension("json.corrupt"));
            }
            DwellStore::default()
        }
        None => DwellStore::default(),
    };
    Hub {
        store,
        path,
        dirty: false,
        saved: Instant::now(),
    }
}

fn with<T>(f: impl FnOnce(&mut Hub) -> T) -> T {
    HUB.with(|h| {
        let mut h = h.borrow_mut();
        let hub = h.get_or_insert_with(|| load(crate::config::dwell_path()));
        f(hub)
    })
}

fn save(h: &mut Hub) {
    if let (true, Some(p)) = (h.dirty, &h.path) {
        if h.store.save(p).is_ok() {
            h.dirty = false;
        }
    }
    h.saved = Instant::now();
}

/// `secs` were spent at `frac` of the book.
pub fn record(book: u64, frac: f64, secs: f64) {
    with(|h| {
        if h.store.record(&book.to_string(), frac, secs) > 0.0 {
            h.dirty = true;
        }
        if h.dirty && h.saved.elapsed() > Duration::from_secs(30) {
            save(h);
        }
    });
}

/// Writes the store now if something changed.
pub fn flush() {
    with(save);
}

/// The buckets of a book (bucket → seconds).
pub fn buckets(book: u64) -> BTreeMap<u16, f32> {
    with(|h| h.store.get(&book.to_string()).cloned().unwrap_or_default())
}
