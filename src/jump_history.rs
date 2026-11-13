//! Where you were before a jump, and where you jumped back from (R26, F5), without GTK.
//!
//! A jump is a link, a table-of-contents entry, a search result, a bookmark, the slider or the
//! go-to dialog. Each one records the place you left; Alt+Left goes back, Alt+Right forward.

/// A place in the book.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub spine: usize,
    pub frac: f64,
}

impl Place {
    pub fn new(spine: usize, frac: f64) -> Self {
        Place {
            spine,
            frac: if frac.is_finite() {
                frac.clamp(0.0, 1.0)
            } else {
                0.0
            },
        }
    }

    /// The same place, give or take a few lines of a long chapter.
    fn near(&self, other: &Place) -> bool {
        self.spine == other.spine && (self.frac - other.frac).abs() < 0.002
    }
}

/// How many places are remembered.
const LIMIT: usize = 100;
/// Jumps this close together (the slider being dragged) count as one.
const BURST_MS: i64 = 1500;

#[derive(Debug, Default, Clone)]
pub struct JumpHistory {
    back: Vec<Place>,
    forward: Vec<Place>,
    last_burst_ms: Option<i64>,
}

impl JumpHistory {
    /// Remembers `from` before a jump; a new jump drops the forward list.
    pub fn record(&mut self, from: Place) {
        self.last_burst_ms = None;
        self.push(from);
    }

    /// Like [`JumpHistory::record`] for a stream of jumps (dragging the slider): only the first of
    /// a burst is remembered.
    pub fn record_burst(&mut self, from: Place, now_ms: i64) {
        let in_burst = self
            .last_burst_ms
            .is_some_and(|t| (0..BURST_MS).contains(&(now_ms - t)));
        self.last_burst_ms = Some(now_ms);
        if !in_burst {
            self.push(from);
        }
    }

    fn push(&mut self, from: Place) {
        self.forward.clear();
        if self.back.last().is_some_and(|l| l.near(&from)) {
            return;
        }
        self.back.push(from);
        if self.back.len() > LIMIT {
            self.back.remove(0);
        }
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Goes back from `current`: the place to show, and `current` becomes the forward place.
    pub fn back(&mut self, current: Place) -> Option<Place> {
        let to = self.back.pop()?;
        self.forward.push(current);
        Some(to)
    }

    pub fn forward(&mut self, current: Place) -> Option<Place> {
        let to = self.forward.pop()?;
        self.back.push(current);
        Some(to)
    }

    /// The places "back" returns to, oldest first (the last one is where Back goes next).
    pub fn back_places(&self) -> &[Place] {
        &self.back
    }

    pub fn depth(&self) -> (usize, usize) {
        (self.back.len(), self.forward.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: usize, f: f64) -> Place {
        Place::new(s, f)
    }

    #[test]
    fn back_and_forward_walk_the_jumps() {
        let mut h = JumpHistory::default();
        // at A, jump to B, then jump to C
        h.record(p(0, 0.1));
        h.record(p(3, 0.5));
        assert!(h.can_back() && !h.can_forward());
        // back from C goes to B, back again to A
        assert_eq!(h.back(p(7, 0.0)), Some(p(3, 0.5)));
        assert_eq!(h.back(p(3, 0.5)), Some(p(0, 0.1)));
        assert_eq!(h.back(p(0, 0.1)), None);
        // forward retraces
        assert_eq!(h.forward(p(0, 0.1)), Some(p(3, 0.5)));
        assert_eq!(h.forward(p(3, 0.5)), Some(p(7, 0.0)));
        assert_eq!(h.forward(p(7, 0.0)), None);
    }

    #[test]
    fn the_back_places_are_listed_in_order() {
        let mut h = JumpHistory::default();
        h.record(p(0, 0.1));
        h.record(p(3, 0.5));
        assert_eq!(h.back_places(), &[p(0, 0.1), p(3, 0.5)]);
        h.back(p(7, 0.0));
        assert_eq!(h.back_places(), &[p(0, 0.1)]);
    }

    #[test]
    fn a_new_jump_drops_the_forward_list() {
        let mut h = JumpHistory::default();
        h.record(p(0, 0.0));
        h.back(p(5, 0.0));
        assert!(h.can_forward());
        h.record(p(0, 0.0));
        assert!(!h.can_forward());
    }

    #[test]
    fn the_same_place_is_not_recorded_twice() {
        let mut h = JumpHistory::default();
        h.record(p(2, 0.30));
        h.record(p(2, 0.301));
        assert_eq!(h.depth(), (1, 0));
        h.record(p(2, 0.5));
        assert_eq!(h.depth(), (2, 0));
    }

    #[test]
    fn dragging_the_slider_is_one_jump() {
        let mut h = JumpHistory::default();
        h.record_burst(p(0, 0.0), 1_000);
        h.record_burst(p(1, 0.2), 1_100);
        h.record_burst(p(2, 0.4), 1_300);
        assert_eq!(h.depth(), (1, 0));
        assert_eq!(h.back(p(3, 0.0)), Some(p(0, 0.0)));
        // a pause starts a new burst
        h.record_burst(p(3, 0.0), 10_000);
        h.record_burst(p(4, 0.0), 10_100);
        assert_eq!(h.depth(), (1, 0));
    }

    #[test]
    fn the_history_is_bounded() {
        let mut h = JumpHistory::default();
        for i in 0..(LIMIT + 20) {
            h.record(p(i, 0.5));
        }
        assert_eq!(h.depth().0, LIMIT);
        // the oldest went
        let mut oldest = None;
        while let Some(x) = h.back(p(0, 0.0)) {
            oldest = Some(x);
        }
        assert_eq!(oldest, Some(p(20, 0.5)));
    }

    #[test]
    fn places_are_made_safe() {
        assert_eq!(Place::new(1, f64::NAN).frac, 0.0);
        assert_eq!(Place::new(1, 7.0).frac, 1.0);
    }
}
