//! Filling the book grid in chunks (pure, no GTK).
//!
//! A `GtkGridView` builds tiles while a model is filled, so putting 2000 books into it in one go blocks the
//! main loop for most of a second. A [`Chunker`] hands the books out in pieces instead: a first piece that
//! covers what is on screen (so the grid is never empty), then pieces sized to a time budget per idle step,
//! learned from how long the previous pieces took. A new requery, search or sort replaces the chunker; a step
//! that was queued for the old one sees its generation is stale and does nothing ([`Chunker::is_for`]).

use std::collections::VecDeque;

/// Books in the first piece when nothing says otherwise: a screenful of tiles at the usual zoom.
pub const FIRST_CHUNK: usize = 48;
/// Smallest and largest piece of the later steps.
pub const MIN_CHUNK: usize = 16;
pub const MAX_CHUNK: usize = 1500;
/// Time one step may take (ms): below a frame at 60 Hz.
pub const BUDGET_MS: f64 = 8.0;
/// Time one step may take on an e-ink screen (ms): fewer, larger refreshes.
pub const BUDGET_EINK_MS: f64 = 40.0;
/// A fill that takes longer than this (ms) shows the busy indicator.
pub const PROGRESS_AFTER_MS: u128 = 200;

#[derive(Debug)]
pub struct Chunker<T> {
    generation: u64,
    rest: VecDeque<T>,
    first: usize,
    taken: usize,
    budget_ms: f64,
    /// Smoothed cost of one book (ms); `None` until a piece was timed.
    per_item_ms: Option<f64>,
}

impl<T> Chunker<T> {
    /// `first` is the size of the first piece (clamped to the books there are).
    pub fn new(generation: u64, items: Vec<T>, first: usize, budget_ms: f64) -> Self {
        Chunker {
            generation,
            rest: items.into(),
            first: first.max(1),
            taken: 0,
            budget_ms: budget_ms.max(1.0),
            per_item_ms: None,
        }
    }

    /// True when a step queued for `generation` still belongs to this fill.
    pub fn is_for(&self, generation: u64) -> bool {
        self.generation == generation
    }

    /// How many books were handed out.
    pub fn loaded(&self) -> usize {
        self.taken
    }

    pub fn is_done(&self) -> bool {
        self.rest.is_empty()
    }

    /// Size of the next piece.
    pub fn next_size(&self) -> usize {
        let n = if self.taken == 0 {
            self.first
        } else {
            match self.per_item_ms {
                Some(c) if c > 0.0 => ((self.budget_ms / c) as usize).clamp(MIN_CHUNK, MAX_CHUNK),
                _ => MIN_CHUNK * 2,
            }
        };
        n.min(self.rest.len())
    }

    /// The next piece (empty when everything was handed out).
    pub fn take(&mut self) -> Vec<T> {
        let n = self.next_size();
        self.taken += n;
        self.rest.drain(..n).collect()
    }

    /// Everything that is left (for "select all", or when the fill must be complete now).
    pub fn take_all(&mut self) -> Vec<T> {
        self.taken += self.rest.len();
        self.rest.drain(..).collect()
    }

    /// Reports how long a piece of `n` books took, so the next ones fit the budget.
    pub fn record(&mut self, n: usize, ms: f64) {
        if n == 0 {
            return;
        }
        let c = ms / n as f64;
        self.per_item_ms = Some(match self.per_item_ms {
            // the newest piece counts most; one slow piece does not collapse the size for good
            Some(p) => p * 0.4 + c * 0.6,
            None => c,
        });
    }
}

/// True when the busy indicator should show: the fill is not done and has run for a while.
pub fn show_progress(done: bool, elapsed_ms: u128) -> bool {
    !done && elapsed_ms >= PROGRESS_AFTER_MS
}

/// How many books the first piece needs so that the part of the list the user is looking at is there:
/// the scroll position as a share of the old content (`value + page` of `upper`) times the books, plus a
/// margin of a screen. Without a scroll position it is the usual first piece.
pub fn first_for_scroll(value: f64, page: f64, upper: f64, books: usize) -> usize {
    if value <= 0.5 || upper <= 0.0 || books == 0 {
        return FIRST_CHUNK.min(books.max(1));
    }
    let share = ((value + page) / upper).clamp(0.0, 1.0);
    let need = (share * books as f64).ceil() as usize + FIRST_CHUNK;
    need.clamp(FIRST_CHUNK, books.max(FIRST_CHUNK))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: &mut Chunker<u32>) -> Vec<Vec<u32>> {
        let mut out = Vec::new();
        while !c.is_done() {
            let p = c.take();
            c.record(p.len(), 1.0);
            out.push(p);
        }
        out
    }

    #[test]
    fn hands_out_every_book_once_in_order() {
        let mut c = Chunker::new(1, (0..2000).collect(), FIRST_CHUNK, BUDGET_MS);
        let all: Vec<u32> = run(&mut c).into_iter().flatten().collect();
        assert_eq!(all, (0..2000).collect::<Vec<_>>());
        assert!(c.is_done());
        assert_eq!(c.loaded(), 2000);
    }

    #[test]
    fn first_piece_is_the_first_size_and_later_ones_follow_the_budget() {
        let mut c = Chunker::new(1, (0..5000).collect(), 48, 8.0);
        assert_eq!(c.take().len(), 48);
        // 0.1 ms per book and 8 ms of budget: 80 books
        c.record(100, 10.0);
        assert_eq!(c.next_size(), 80);
        // a slower machine gets smaller pieces, never below the minimum
        let mut slow = Chunker::new(1, (0..5000).collect(), 48, 8.0);
        slow.take();
        slow.record(10, 1000.0);
        assert_eq!(slow.next_size(), MIN_CHUNK);
        // a very fast one never above the maximum
        let mut fast = Chunker::new(1, (0..5000).collect(), 48, 8.0);
        fast.take();
        fast.record(1000, 0.001);
        assert_eq!(fast.next_size(), MAX_CHUNK);
    }

    #[test]
    fn the_e_ink_budget_gives_larger_pieces() {
        let mut a = Chunker::new(1, (0..5000).collect(), 48, BUDGET_MS);
        let mut b = Chunker::new(1, (0..5000).collect(), 48, BUDGET_EINK_MS);
        for c in [&mut a, &mut b] {
            c.take();
            c.record(100, 20.0);
        }
        assert!(b.next_size() > a.next_size() * 3);
    }

    #[test]
    fn small_lists_come_in_one_piece_and_empty_ones_are_done() {
        let mut c = Chunker::new(1, (0..30).collect(), FIRST_CHUNK, BUDGET_MS);
        assert_eq!(c.take().len(), 30);
        assert!(c.is_done());
        let mut e: Chunker<u32> = Chunker::new(1, Vec::new(), FIRST_CHUNK, BUDGET_MS);
        assert!(e.is_done());
        assert!(e.take().is_empty());
    }

    #[test]
    fn a_stale_generation_is_recognised() {
        let c: Chunker<u32> = Chunker::new(7, vec![1, 2, 3], 2, BUDGET_MS);
        assert!(c.is_for(7));
        assert!(!c.is_for(6));
    }

    #[test]
    fn take_all_finishes_the_fill() {
        let mut c = Chunker::new(1, (0..500).collect(), 48, BUDGET_MS);
        let first = c.take();
        let rest = c.take_all();
        assert_eq!(first.len() + rest.len(), 500);
        assert_eq!(rest[0], 48);
        assert!(c.is_done());
        assert!(c.take().is_empty());
    }

    #[test]
    fn progress_shows_only_for_a_long_unfinished_fill() {
        assert!(!show_progress(false, 150));
        assert!(show_progress(false, 200));
        assert!(!show_progress(true, 5000));
    }

    #[test]
    fn the_first_piece_covers_the_scroll_position() {
        assert_eq!(first_for_scroll(0.0, 600.0, 10_000.0, 2000), FIRST_CHUNK);
        // half way down: about half the books plus a margin
        let n = first_for_scroll(4700.0, 600.0, 10_600.0, 2000);
        assert!((1000..=1100).contains(&n), "{n}");
        // never more than the books there are
        assert_eq!(first_for_scroll(9000.0, 600.0, 9000.0, 100), 100);
        assert_eq!(first_for_scroll(100.0, 600.0, 0.0, 100), 48);
    }

    #[test]
    fn a_search_that_finds_little_stays_in_one_piece() {
        let mut c = Chunker::new(
            2,
            (0..12).collect(),
            first_for_scroll(900.0, 600.0, 10_000.0, 12),
            BUDGET_MS,
        );
        assert_eq!(c.take().len(), 12);
        assert!(c.is_done());
    }

    /// Perf budget (release numbers, debug gets 30x, `BOOKX_PERF_SCALE` scales): planning and handing out
    /// 100 000 books in pieces must cost next to nothing against the tiles themselves.
    #[test]
    fn planning_100k_books_stays_within_budget() {
        let scale: u128 = std::env::var("BOOKX_PERF_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let debug = if cfg!(debug_assertions) { 30 } else { 1 };
        let t = std::time::Instant::now();
        let mut c = Chunker::new(1, (0..100_000u32).collect(), FIRST_CHUNK, BUDGET_MS);
        let mut pieces = 0;
        let mut seen = 0;
        while !c.is_done() {
            let p = c.take();
            seen += p.len();
            // 0.1 ms a book, a typical debug-build tile
            c.record(p.len(), p.len() as f64 * 0.1);
            pieces += 1;
        }
        let ms = t.elapsed().as_millis();
        assert_eq!(seen, 100_000);
        assert!(ms <= 50 * scale * debug, "{ms} ms for {pieces} pieces");
        // 8 ms pieces of 0.1 ms books are 80 books each: the list arrives in well under 2000 steps
        assert!(pieces < 1500, "{pieces}");
    }
}
