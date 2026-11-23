//! The stall detector: a main-loop timer that notices when the interface did not run for a while.
//!
//! [`StallMeter`] is the pure part: it is told the time of every tick and says how long the loop was
//! late. The GTK timer that feeds it is `ui::stall_watch`. A stall is written to the flight recorder
//! (`ui/stall`) with the area the user was in and the operation that started last, so a complaint like
//! "it hangs when I open X" comes with data.

/// How often the timer runs.
pub const TICK_MS: u64 = 100;
/// A stall shorter than this (milliseconds late) is normal.
pub const STALL_MS: u64 = 250;
/// A gap longer than this is a suspended computer, not a stall.
pub const SUSPEND_MS: u64 = 60_000;

#[derive(Debug, Default)]
pub struct StallMeter {
    last: Option<u64>,
}

impl StallMeter {
    /// The loop ran at `now_ms`: how many milliseconds late it was, when that counts as a stall.
    pub fn tick(&mut self, now_ms: u64) -> Option<u64> {
        let prev = self.last.replace(now_ms)?;
        let gap = now_ms.saturating_sub(prev);
        let late = gap.saturating_sub(TICK_MS);
        (late >= STALL_MS && gap < SUSPEND_MS).then_some(late)
    }
}

/// Writes the stall to the flight recorder, naming where the user was and what started last.
pub fn record(late_ms: u64) {
    // read before the stall's own entry is started: that would become "the last operation"
    let last = crate::trace::last_started();
    let mut t = crate::trace::event(crate::trace::Area::Ui, "stall");
    t.took(late_ms);
    if let Some(a) = crate::trace::active() {
        t.word("in", a.word());
    }
    if let Some((a, name)) = last {
        t.word("area", a.word()).word("last_op", name);
    }
    t.fail_class(crate::trace::ErrorClass::Timeout);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_that_keeps_up_is_never_a_stall() {
        let mut m = StallMeter::default();
        for i in 0..50 {
            assert_eq!(m.tick(i * TICK_MS + 3), None);
        }
    }

    #[test]
    fn a_late_tick_says_how_late() {
        let mut m = StallMeter::default();
        assert_eq!(m.tick(1000), None);
        assert_eq!(m.tick(1100), None);
        assert_eq!(m.tick(1100 + 100 + 249), None);
        let t = 1100 + 100 + 249;
        assert_eq!(m.tick(t + TICK_MS + 250), Some(250));
        assert_eq!(m.tick(t + 2 * TICK_MS + 250 + 4000), Some(4000));
    }

    #[test]
    fn a_suspended_computer_is_not_a_stall() {
        let mut m = StallMeter::default();
        m.tick(0);
        assert_eq!(m.tick(3_600_000), None);
        assert_eq!(m.tick(3_600_100), None);
    }

    #[test]
    fn a_clock_that_goes_back_is_harmless() {
        let mut m = StallMeter::default();
        m.tick(5000);
        assert_eq!(m.tick(100), None);
    }

    #[test]
    fn a_recorded_stall_names_the_area_and_the_last_operation() {
        crate::trace::set_active(crate::trace::Area::Library);
        let _ = crate::trace::op(crate::trace::Area::Import, "zz_stall_probe").ok_dropped();
        record(1234);
        let e = crate::trace::snapshot()
            .into_iter()
            .rev()
            .find(|e| e.name == "stall")
            .unwrap();
        assert_eq!(e.ms, Some(1234));
        assert_eq!(e.field("in"), Some("library"));
        assert!(e.field("last_op").is_some() && e.field("area").is_some());
    }
}
