//! How long each part of a book was on screen (for the bars of the book map).
//!
//! The reader's pages change with the font size, so time is not kept per page but per
//! **bucket**: the book is cut into [`BUCKETS`] equal slices of its text and a bucket holds the
//! seconds spent while that slice was on screen. The map turns buckets into pages for whatever the
//! current layout is. Everything is capped so the file cannot grow without limit:
//! a visit counts at most [`MAX_VISIT_SECS`], a bucket at most [`MAX_BUCKET_SECS`], a book has at
//! most [`BUCKETS`] entries and the store keeps the [`MAX_BOOKS`] books read last.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const BUCKETS: usize = 4096;
/// A page left open longer than this (a break) counts as this much.
pub const MAX_VISIT_SECS: f32 = 180.0;
pub const MAX_BUCKET_SECS: f32 = 6.0 * 3600.0;
pub const MAX_BOOKS: usize = 500;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BookDwell {
    /// When the book was last read, a counter of the store (not a date): the oldest is dropped first.
    pub touched: u64,
    /// Bucket → seconds.
    pub buckets: BTreeMap<u16, f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DwellStore {
    clock: u64,
    books: BTreeMap<String, BookDwell>,
}

/// The bucket a position (a fraction of the book) falls in.
pub fn bucket_of(frac: f64) -> u16 {
    let f = if frac.is_finite() {
        frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((f * BUCKETS as f64) as usize).min(BUCKETS - 1) as u16
}

impl DwellStore {
    /// Adds `secs` spent at `frac` of `book`. Returns the seconds that counted.
    pub fn record(&mut self, book: &str, frac: f64, secs: f64) -> f32 {
        if !secs.is_finite() || secs <= 0.0 {
            return 0.0;
        }
        let secs = (secs as f32).min(MAX_VISIT_SECS);
        self.clock += 1;
        let clock = self.clock;
        let b = self.books.entry(book.to_string()).or_default();
        b.touched = clock;
        let slot = b.buckets.entry(bucket_of(frac)).or_insert(0.0);
        let before = *slot;
        *slot = (*slot + secs).min(MAX_BUCKET_SECS);
        let counted = *slot - before;
        self.trim();
        counted
    }

    fn trim(&mut self) {
        while self.books.len() > MAX_BOOKS {
            let Some(oldest) = self
                .books
                .iter()
                .min_by_key(|(_, b)| b.touched)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.books.remove(&oldest);
        }
    }

    pub fn get(&self, book: &str) -> Option<&BTreeMap<u16, f32>> {
        self.books.get(book).map(|b| &b.buckets)
    }

    pub fn forget(&mut self, book: &str) -> bool {
        self.books.remove(book).is_some()
    }

    pub fn book_count(&self) -> usize {
        self.books.len()
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Reads a store; entries outside the caps are clamped or dropped.
    pub fn from_json(s: &str) -> Result<Self> {
        let mut st: DwellStore = serde_json::from_str(s)?;
        for b in st.books.values_mut() {
            b.buckets
                .retain(|k, v| (*k as usize) < BUCKETS && v.is_finite() && *v > 0.0);
            for v in b.buckets.values_mut() {
                *v = v.min(MAX_BUCKET_SECS);
            }
        }
        st.trim();
        Ok(st)
    }

    /// A missing file is an empty store.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(s) => Self::from_json(&s).with_context(|| format!("loading {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, self.to_json()?)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Seconds per page for a book of `pages` pages: each bucket's time is shared between the pages it
/// overlaps in proportion to the overlap, so the total stays the same.
pub fn per_page(buckets: &BTreeMap<u16, f32>, pages: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; pages];
    if pages == 0 {
        return out;
    }
    let (bw, pw) = (1.0 / BUCKETS as f64, 1.0 / pages as f64);
    for (&b, &secs) in buckets {
        let (lo, hi) = (b as f64 * bw, (b as f64 + 1.0) * bw);
        let first = ((lo / pw) as usize).min(pages - 1);
        let last = (((hi / pw) - 1e-9).max(0.0) as usize).min(pages - 1);
        for (p, slot) in out.iter_mut().enumerate().take(last + 1).skip(first) {
            let overlap = (hi.min((p + 1) as f64 * pw) - lo.max(p as f64 * pw)).max(0.0);
            *slot += secs * (overlap / bw) as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_is_recorded_per_slice_of_the_book() {
        let mut s = DwellStore::default();
        assert_eq!(s.record("1", 0.5, 30.0), 30.0);
        s.record("1", 0.5, 10.0);
        s.record("1", 0.0, 5.0);
        let b = s.get("1").unwrap();
        assert_eq!(b[&bucket_of(0.5)], 40.0);
        assert_eq!(b[&0], 5.0);
        assert!(s.get("2").is_none());
    }

    #[test]
    fn bad_values_do_not_count_and_visits_are_capped() {
        let mut s = DwellStore::default();
        assert_eq!(s.record("1", 0.1, -3.0), 0.0);
        assert_eq!(s.record("1", 0.1, f64::NAN), 0.0);
        assert_eq!(s.record("1", f64::NAN, 5.0), 5.0);
        assert_eq!(s.record("1", 0.1, 100_000.0), MAX_VISIT_SECS);
        assert_eq!(bucket_of(2.0), (BUCKETS - 1) as u16);
        assert_eq!(bucket_of(-1.0), 0);
    }

    #[test]
    fn a_bucket_stops_at_the_cap() {
        let mut s = DwellStore::default();
        for _ in 0..1000 {
            s.record("1", 0.3, MAX_VISIT_SECS as f64);
        }
        assert_eq!(s.get("1").unwrap()[&bucket_of(0.3)], MAX_BUCKET_SECS);
        assert_eq!(s.record("1", 0.3, 10.0), 0.0);
    }

    #[test]
    fn only_the_books_read_last_are_kept() {
        let mut s = DwellStore::default();
        for i in 0..(MAX_BOOKS + 25) {
            s.record(&i.to_string(), 0.1, 5.0);
        }
        assert_eq!(s.book_count(), MAX_BOOKS);
        assert!(s.get("0").is_none());
        assert!(s.get(&(MAX_BOOKS + 24).to_string()).is_some());
        // reading an old book again keeps it
        s.record("30", 0.2, 5.0);
        s.record("new", 0.2, 5.0);
        assert!(s.get("30").is_some());
        assert!(s.get("25").is_none());
    }

    #[test]
    fn a_book_has_at_most_one_entry_per_bucket() {
        let mut s = DwellStore::default();
        for i in 0..20_000 {
            s.record("1", i as f64 / 20_000.0, 1.0);
        }
        assert_eq!(s.get("1").unwrap().len(), BUCKETS);
    }

    #[test]
    fn json_round_trip_and_hostile_files() {
        let mut s = DwellStore::default();
        s.record("a", 0.25, 12.0);
        let back = DwellStore::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(back, s);
        let bad = DwellStore::from_json(
            r#"{"books":{"x":{"touched":1,"buckets":{"7":1e9,"9999":3.0,"8":-1.0}}}}"#,
        )
        .unwrap();
        let b = bad.get("x").unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[&7], MAX_BUCKET_SECS);
        assert!(DwellStore::from_json("nonsense").is_err());
        assert_eq!(DwellStore::from_json("{}").unwrap().book_count(), 0);
    }

    #[test]
    fn save_and_load_use_a_file() {
        let dir = std::env::temp_dir().join(format!("bookx-dwell-{}", std::process::id()));
        let path = dir.join("dwell.json");
        assert_eq!(DwellStore::load(&path).unwrap().book_count(), 0);
        let mut s = DwellStore::default();
        s.record("1", 0.5, 9.0);
        s.save(&path).unwrap();
        assert_eq!(DwellStore::load(&path).unwrap(), s);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn buckets_become_pages_without_losing_time() {
        let mut s = DwellStore::default();
        for i in 0..BUCKETS {
            s.record("1", (i as f64 + 0.5) / BUCKETS as f64, 1.0);
        }
        let b = s.get("1").unwrap();
        for pages in [1usize, 7, 300, BUCKETS, 10_000] {
            let v = per_page(b, pages);
            assert_eq!(v.len(), pages);
            let total: f32 = v.iter().sum();
            assert!(
                (total - BUCKETS as f32).abs() < BUCKETS as f32 * 1e-3,
                "{pages}: {total}"
            );
        }
        let v = per_page(b, 4);
        assert!(v.iter().all(|x| (x - 1024.0).abs() < 1.0));
        assert!(per_page(b, 0).is_empty());
    }

    #[test]
    fn unread_pages_have_no_time() {
        let mut s = DwellStore::default();
        s.record("1", 0.52, 20.0);
        let v = per_page(s.get("1").unwrap(), 10);
        assert_eq!(v[5], 20.0);
        assert!(v.iter().enumerate().all(|(i, x)| i == 5 || *x == 0.0));
    }
}
