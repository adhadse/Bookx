//! Scroll sync between the code view and the preview (enhancement 26): pure mapping and timing logic.
//!
//! The preview blocks that could be paired with the source (see [`crate::preview_map`]) give *anchors*:
//! `(source offset, preview y)`. Between two anchors the two scales are interpolated linearly, so a position
//! inside a block, and the stretches of generated or unpaired blocks, map smoothly. Both directions are a
//! binary search, so one sync step does not depend on the size of the file.

/// Anchors of one laid out page, strictly increasing in offset and non-decreasing in y.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScrollMap {
    off: Vec<f64>,
    y: Vec<f64>,
}

impl ScrollMap {
    /// Builds the map from `(source offset, y)` pairs (any order, may be inconsistent), the length of the
    /// source and the height of the preview. Pairs that would make the map go backwards are dropped.
    /// The ends of the text and of the page are added as anchors, so every position has a neighbour.
    pub fn new(anchors: &[(usize, f64)], text_len: usize, flow_h: f64) -> ScrollMap {
        let mut a: Vec<(usize, f64)> = anchors
            .iter()
            .copied()
            .filter(|(_, y)| y.is_finite() && *y >= 0.0 && *y <= flow_h)
            .collect();
        a.sort_by(|p, q| p.0.cmp(&q.0).then(p.1.total_cmp(&q.1)));
        let mut off: Vec<f64> = Vec::with_capacity(a.len() + 2);
        let mut y: Vec<f64> = Vec::with_capacity(a.len() + 2);
        if a.first().is_none_or(|f| f.0 > 0) {
            off.push(0.0);
            y.push(0.0);
        }
        for (o, yy) in a {
            let o = o.min(text_len) as f64;
            let last_o = off.last().copied();
            let last_y = y.last().copied().unwrap_or(0.0);
            if last_o.is_some_and(|l| o <= l) || yy < last_y {
                continue;
            }
            off.push(o);
            y.push(yy);
        }
        if off.last().is_none_or(|l| *l < text_len as f64) {
            off.push(text_len as f64);
            y.push(flow_h.max(y.last().copied().unwrap_or(0.0)));
        }
        if off.len() < 2 {
            return ScrollMap::default();
        }
        ScrollMap { off, y }
    }

    pub fn is_empty(&self) -> bool {
        self.off.len() < 2
    }

    pub fn len(&self) -> usize {
        self.off.len()
    }

    fn lerp(xs: &[f64], ys: &[f64], x: f64) -> f64 {
        let i = xs.partition_point(|v| *v <= x);
        if i == 0 {
            return ys[0];
        }
        if i >= xs.len() {
            return ys[ys.len() - 1];
        }
        let (x0, x1) = (xs[i - 1], xs[i]);
        if x1 <= x0 {
            return ys[i - 1];
        }
        ys[i - 1] + (ys[i] - ys[i - 1]) * (x - x0) / (x1 - x0)
    }

    /// The preview y that shows source offset `offset` (fractions of a character count).
    pub fn y_of(&self, offset: f64) -> Option<f64> {
        (!self.is_empty()).then(|| Self::lerp(&self.off, &self.y, offset))
    }

    /// The source offset shown at preview y. Where several anchors share a y (blocks of no height) the first wins.
    pub fn offset_of(&self, y: f64) -> Option<f64> {
        if self.is_empty() {
            return None;
        }
        // `y` is non-decreasing but may have plateaus: look for the first anchor above y
        let i = self.y.partition_point(|v| *v <= y);
        if i == 0 {
            return Some(self.off[0]);
        }
        if i >= self.y.len() {
            return Some(self.off[self.off.len() - 1]);
        }
        let (y0, y1) = (self.y[i - 1], self.y[i]);
        if y1 <= y0 {
            return Some(self.off[i - 1]);
        }
        Some(self.off[i - 1] + (self.off[i] - self.off[i - 1]) * (y - y0) / (y1 - y0))
    }
}

/// Decides when a scroll may be passed on: a cursor move (typing, arrow keys, a click) wins for a short time,
/// because it scrolls both panes by itself and the sync must not move them away again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gate {
    quiet_until_ms: u64,
}

impl Gate {
    /// How long a cursor move keeps the scroll sync quiet.
    pub const CURSOR_QUIET_MS: u64 = 300;

    pub fn cursor_moved(&mut self, now_ms: u64) {
        self.quiet_until_ms = now_ms + Self::CURSOR_QUIET_MS;
    }

    /// Milliseconds until a scroll may be passed on (0 = now).
    pub fn wait_ms(&self, now_ms: u64) -> u64 {
        self.quiet_until_ms.saturating_sub(now_ms)
    }
}

/// Delay before a sync step: the debounce, or the rest of the quiet time when that is longer.
pub fn step_delay_ms(gate: &Gate, now_ms: u64, debounce_ms: u64) -> u64 {
    gate.wait_ms(now_ms).saturating_add(1).max(debounce_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> ScrollMap {
        // three paragraphs: source 100..200..400, preview y 40..90..300
        ScrollMap::new(&[(100, 40.0), (200, 90.0), (400, 300.0)], 500, 400.0)
    }

    #[test]
    fn anchors_map_exactly_and_the_ends_are_added() {
        let m = map();
        assert_eq!(m.y_of(100.0), Some(40.0));
        assert_eq!(m.y_of(400.0), Some(300.0));
        assert_eq!(m.offset_of(90.0), Some(200.0));
        assert_eq!(m.y_of(0.0), Some(0.0));
        assert_eq!(m.y_of(500.0), Some(400.0));
        assert_eq!(m.len(), 5);
    }

    #[test]
    fn positions_inside_a_block_are_interpolated() {
        let m = map();
        assert_eq!(m.y_of(150.0), Some(65.0));
        assert_eq!(m.offset_of(195.0), Some(300.0));
        // the stretch before the first anchor runs from the top
        assert_eq!(m.y_of(50.0), Some(20.0));
    }

    #[test]
    fn out_of_range_positions_are_clamped() {
        let m = map();
        assert_eq!(m.y_of(-5.0), Some(0.0));
        assert_eq!(m.y_of(1e9), Some(400.0));
        assert_eq!(m.offset_of(-1.0), Some(0.0));
        assert_eq!(m.offset_of(1e9), Some(500.0));
    }

    #[test]
    fn the_mapping_is_monotonic_and_round_trips() {
        let m = map();
        let mut last = -1.0;
        for o in 0..=500 {
            let y = m.y_of(o as f64).unwrap();
            assert!(y >= last, "y went back at {o}");
            last = y;
            let back = m.offset_of(y).unwrap();
            assert!((back - o as f64).abs() < 1e-6, "{o} -> {y} -> {back}");
        }
    }

    #[test]
    fn inconsistent_anchors_are_dropped_not_followed() {
        // the second anchor goes back in y, the third repeats an offset
        let m = ScrollMap::new(
            &[(100, 50.0), (200, 20.0), (100, 60.0), (300, 80.0)],
            400,
            100.0,
        );
        let mut last = -1.0;
        for o in (0..=400).step_by(7) {
            let y = m.y_of(o as f64).unwrap();
            assert!(y >= last);
            last = y;
        }
        assert_eq!(m.y_of(300.0), Some(80.0));
    }

    #[test]
    fn plateaus_and_empty_inputs_are_survivable() {
        // two blocks with no height between them
        let m = ScrollMap::new(&[(10, 5.0), (20, 5.0), (30, 9.0)], 40, 10.0);
        assert_eq!(m.offset_of(5.0), Some(20.0));
        assert!(m.y_of(15.0).is_some());
        let none = ScrollMap::new(&[], 0, 0.0);
        assert!(none.is_empty());
        assert_eq!(none.y_of(3.0), None);
        assert_eq!(none.offset_of(3.0), None);
        // no anchors at all still gives a straight line over the whole text
        let line = ScrollMap::new(&[], 100, 50.0);
        assert_eq!(line.y_of(50.0), Some(25.0));
        assert_eq!(line.offset_of(25.0), Some(50.0));
        // anchors outside the page are ignored
        let off = ScrollMap::new(&[(10, 999.0), (20, f64::NAN)], 100, 50.0);
        assert_eq!(off.len(), 2);
    }

    #[test]
    fn a_large_map_is_fast() {
        let n = 200_000usize;
        let anchors: Vec<(usize, f64)> = (0..n).map(|i| (i * 60, i as f64 * 22.0)).collect();
        let m = ScrollMap::new(&anchors, n * 60, n as f64 * 22.0);
        let t = std::time::Instant::now();
        let mut acc = 0.0;
        for k in 0..10_000 {
            let o = (k * 1_187) % (n * 60);
            let y = m.y_of(o as f64).unwrap();
            acc += m.offset_of(y).unwrap();
        }
        let per_step = t.elapsed().as_secs_f64() * 1000.0 / 10_000.0;
        assert!(acc > 0.0);
        // a step is far below 2 ms even in a debug build
        assert!(per_step < 0.2, "{per_step} ms per step");
    }

    #[test]
    fn a_cursor_move_keeps_the_sync_quiet_for_a_short_time() {
        let mut g = Gate::default();
        assert_eq!(g.wait_ms(1000), 0);
        g.cursor_moved(1000);
        assert_eq!(g.wait_ms(1100), Gate::CURSOR_QUIET_MS - 100);
        assert_eq!(g.wait_ms(1400), 0);
        // the step waits for the debounce, or for the end of the quiet time
        assert_eq!(step_delay_ms(&g, 1390, 40), 40);
        assert_eq!(step_delay_ms(&g, 1100, 40), 201);
        assert_eq!(step_delay_ms(&Gate::default(), 5, 40), 40);
    }
}
