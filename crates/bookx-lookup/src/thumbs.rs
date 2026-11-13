//! The logic behind the book map's page thumbnails, without GTK: which chapter a page belongs to,
//! a small LRU cache, the "latest request wins" token that cancels outdated work, a debounce and
//! the thumbnail size.

use std::collections::HashMap;
use std::hash::Hash;

/// What the reader adds to a place it jumps to from the map.
pub const JUMP_EPSILON: f64 = 1e-6;

/// The chapter and the place inside it that a page of the book falls in. `weights` are the sizes
/// of the chapters (the same unit as the reader's progress), `page` is 0-based among `total`.
pub fn locate(weights: &[f64], total: usize, page: usize) -> (usize, f64) {
    let sum: f64 = weights.iter().filter(|w| w.is_finite() && **w > 0.0).sum();
    if weights.is_empty() || total == 0 || sum <= 0.0 {
        return (0, 0.0);
    }
    // the same place the reader's jump goes to (it adds a hair so that a chapter's first page belongs
    // to that chapter, not to the end of the one before)
    let f = (page.min(total - 1) as f64 / total as f64 + JUMP_EPSILON).min(1.0) * sum;
    let mut acc = 0.0;
    for (i, w) in weights.iter().enumerate() {
        let w = if w.is_finite() && *w > 0.0 { *w } else { 0.0 };
        if acc + w >= f || i + 1 == weights.len() {
            let local = if w > 0.0 { (f - acc) / w } else { 0.0 };
            return (i, local.clamp(0.0, 1.0));
        }
        acc += w;
    }
    (0, 0.0)
}

/// The size of a thumbnail of a view `view_w` x `view_h`, `target_w` wide, at least 1 x 1.
pub fn thumb_size(view_w: f64, view_h: f64, target_w: f64) -> (u32, u32) {
    if view_w <= 0.0 || view_h <= 0.0 || target_w <= 0.0 {
        return (1, 1);
    }
    let w = target_w.round().max(1.0);
    (w as u32, ((w * view_h / view_w).round() as u32).max(1))
}

/// The pages to make ahead, nearest to `centre` first (`centre` itself first), at most `limit`,
/// without those that `have` says are done. `pages` are the pages in view.
pub fn prefetch_order(
    pages: &[usize],
    centre: usize,
    limit: usize,
    have: impl Fn(usize) -> bool,
) -> Vec<usize> {
    let mut v: Vec<usize> = pages.iter().copied().filter(|p| !have(*p)).collect();
    v.sort_by_key(|p| (p.abs_diff(centre), *p));
    v.dedup();
    v.truncate(limit);
    v
}

/// Orders pages for making ahead so that a chapter's pages come together (its layout is used for
/// all of them before the next chapter is laid out): chapters in the order their nearest page to
/// `centre` comes in `pages`, at most `max_chapters` of them, the pages of a chapter nearest first.
pub fn group_by_chapter(
    pages: &[usize],
    centre: usize,
    max_chapters: usize,
    chapter_of: impl Fn(usize) -> usize,
) -> Vec<usize> {
    let mut chapters: Vec<usize> = Vec::new();
    let mut keyed: Vec<(usize, usize)> = Vec::new();
    for &p in pages {
        let ch = chapter_of(p);
        let rank = match chapters.iter().position(|c| *c == ch) {
            Some(r) => r,
            None if chapters.len() < max_chapters => {
                chapters.push(ch);
                chapters.len() - 1
            }
            None => continue,
        };
        keyed.push((rank, p));
    }
    keyed.sort_by_key(|(rank, p)| (*rank, p.abs_diff(centre), *p));
    keyed.into_iter().map(|(_, p)| p).collect()
}

/// The page among `have` that is closest to `page` (for a stand-in while the real one is made).
pub fn nearest(have: &[usize], page: usize) -> Option<usize> {
    have.iter().copied().min_by_key(|p| (p.abs_diff(page), *p))
}

/// A cache that keeps the most recently used `cap` entries.
#[derive(Debug)]
pub struct Lru<K, V> {
    cap: usize,
    tick: u64,
    map: HashMap<K, (u64, V)>,
}

impl<K: Hash + Eq + Clone, V: Clone> Lru<K, V> {
    pub fn new(cap: usize) -> Self {
        Lru {
            cap: cap.max(1),
            tick: 0,
            map: HashMap::new(),
        }
    }

    pub fn get(&mut self, k: &K) -> Option<V> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| {
            e.0 = t;
            e.1.clone()
        })
    }

    pub fn contains(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }

    pub fn insert(&mut self, k: K, v: V) {
        self.tick += 1;
        self.map.insert(k, (self.tick, v));
        while self.map.len() > self.cap {
            let Some(old) = self
                .map
                .iter()
                .min_by_key(|(_, e)| e.0)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.map.remove(&old);
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// The keys that are in the cache (not touched).
    pub fn keys(&self) -> Vec<K> {
        self.map.keys().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

/// "Only the newest request counts": every request gets a number, work for an older number is
/// dropped when it is about to start.
#[derive(Debug, Default)]
pub struct Latest {
    n: u64,
}

impl Latest {
    pub fn issue(&mut self) -> u64 {
        self.n += 1;
        self.n
    }

    pub fn is_current(&self, token: u64) -> bool {
        token == self.n
    }

    /// Makes every request so far outdated.
    pub fn cancel(&mut self) {
        self.n += 1;
    }
}

/// Waits `delay_ms` after the last event before it lets a request through.
#[derive(Debug)]
pub struct Debounce<K> {
    delay_ms: u64,
    pending: Option<(K, u64)>,
}

impl<K: Clone> Debounce<K> {
    pub fn new(delay_ms: u64) -> Self {
        Debounce {
            delay_ms,
            pending: None,
        }
    }

    /// A new event replaces the one that was waiting and restarts the wait.
    pub fn push(&mut self, key: K, now_ms: u64) {
        self.pending = Some((key, now_ms + self.delay_ms));
    }

    /// The request, once its wait is over (it is taken).
    pub fn ready(&mut self, now_ms: u64) -> Option<K> {
        match &self.pending {
            Some((_, due)) if now_ms >= *due => self.pending.take().map(|p| p.0),
            _ => None,
        }
    }

    pub fn cancel(&mut self) {
        self.pending = None;
    }

    pub fn is_waiting(&self) -> bool {
        self.pending.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_belongs_to_the_chapter_that_holds_its_part_of_the_book() {
        let w = [100.0, 300.0, 100.0];
        assert_eq!(locate(&w, 10, 0).0, 0);
        assert!(locate(&w, 10, 0).1 < 1e-4);
        let (c, f) = locate(&w, 10, 3); // 30% of 500 = 150: inside chapter 1
        assert_eq!(c, 1);
        assert!((f - 50.0 / 300.0).abs() < 1e-5);
        // a chapter's first page belongs to that chapter (the hair the reader adds)
        assert_eq!(locate(&w, 5, 1).0, 1);
        assert_eq!(locate(&w, 5, 4).0, 2);
        assert_eq!(locate(&w, 10, 9).0, 2);
        assert_eq!(locate(&w, 10, 99).0, 2);
        // chapters of no weight are passed over, bad input is safe
        assert_eq!(locate(&[0.0, 10.0], 4, 2).0, 1);
        assert_eq!(locate(&[], 4, 2), (0, 0.0));
        assert_eq!(locate(&w, 0, 2), (0, 0.0));
        assert_eq!(locate(&[f64::NAN, 5.0], 2, 1).0, 1);
    }

    #[test]
    fn a_thumbnail_keeps_the_aspect_ratio_of_the_view() {
        assert_eq!(thumb_size(800.0, 1000.0, 200.0), (200, 250));
        assert_eq!(thumb_size(1000.0, 500.0, 180.0), (180, 90));
        assert_eq!(thumb_size(0.0, 500.0, 180.0), (1, 1));
        assert_eq!(thumb_size(10.0, 1.0, 5.0), (5, 1));
    }

    #[test]
    fn the_cache_drops_the_least_recently_used() {
        let mut c: Lru<u32, &str> = Lru::new(3);
        for (k, v) in [(1, "a"), (2, "b"), (3, "c")] {
            c.insert(k, v);
        }
        assert_eq!(c.get(&1), Some("a")); // 1 is fresh again
        c.insert(4, "d"); // drops 2
        assert!(!c.contains(&2));
        assert!(c.contains(&1) && c.contains(&3) && c.contains(&4));
        assert_eq!(c.len(), 3);
        c.insert(4, "e");
        assert_eq!(c.get(&4), Some("e"));
        assert_eq!(c.len(), 3);
        c.clear();
        assert!(c.is_empty());
        assert_eq!(Lru::<u8, u8>::new(0).cap, 1);
        let mut d: Lru<u8, u8> = Lru::new(4);
        d.insert(1, 1);
        d.insert(2, 2);
        let mut k = d.keys();
        k.sort();
        assert_eq!(k, vec![1, 2]);
    }

    #[test]
    fn prefetch_starts_at_the_pointer_and_skips_what_is_done() {
        let pages: Vec<usize> = (0..20).collect();
        let order = prefetch_order(&pages, 10, 5, |p| p == 11);
        assert_eq!(order, vec![10, 9, 8, 12, 7]);
        assert_eq!(prefetch_order(&pages, 0, 3, |_| false), vec![0, 1, 2]);
        assert!(prefetch_order(&[], 3, 5, |_| false).is_empty());
        assert!(prefetch_order(&pages, 3, 0, |_| false).is_empty());
        assert_eq!(prefetch_order(&[5, 5, 6], 5, 9, |_| false), vec![5, 6]);
    }

    #[test]
    fn prefetch_finishes_a_chapter_before_the_next_and_stops_at_the_pool_size() {
        // chapters of 5 pages; the pointer is on page 7 (chapter 1)
        let pages: Vec<usize> = (0..30).collect();
        let near = prefetch_order(&pages, 7, 12, |_| false);
        let out = group_by_chapter(&near, 7, 2, |p| p / 5);
        // nearest chapter first (1), then the next nearest (0 or 2), nothing of the third
        let chapters: Vec<usize> = out.iter().map(|p| p / 5).collect();
        let mut seen = chapters.clone();
        seen.dedup();
        assert_eq!(seen.len(), 2, "{chapters:?}");
        assert_eq!(seen[0], 1);
        assert_eq!(out[0], 7);
        assert!(out
            .windows(2)
            .all(|w| w[0] / 5 == w[1] / 5 || w[0] / 5 == seen[0]));
        assert!(group_by_chapter(&[], 0, 3, |p| p).is_empty());
        assert!(group_by_chapter(&pages, 0, 0, |p| p).is_empty());
    }

    #[test]
    fn a_stand_in_is_the_nearest_page_that_is_ready() {
        assert_eq!(nearest(&[3, 40, 12], 10), Some(12));
        assert_eq!(nearest(&[], 10), None);
        assert_eq!(nearest(&[8, 12], 10), Some(8));
    }

    #[test]
    fn only_the_newest_request_is_current() {
        let mut l = Latest::default();
        let a = l.issue();
        assert!(l.is_current(a));
        let b = l.issue();
        assert!(!l.is_current(a) && l.is_current(b));
        l.cancel();
        assert!(!l.is_current(b));
    }

    #[test]
    fn the_debounce_waits_after_the_last_event() {
        let mut d = Debounce::new(60);
        d.push(1, 1000);
        assert_eq!(d.ready(1030), None);
        d.push(2, 1040); // a new event restarts the wait and replaces the page
        assert_eq!(d.ready(1090), None);
        assert!(d.is_waiting());
        assert_eq!(d.ready(1100), Some(2));
        assert_eq!(d.ready(2000), None);
        d.push(3, 0);
        d.cancel();
        assert_eq!(d.ready(1000), None);
    }
}
