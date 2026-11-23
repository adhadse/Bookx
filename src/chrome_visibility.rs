//! When the reader's chrome (header, page slider, footer statistics) fades away: no GTK, the clock
//! is passed in. This is the "Reader focus" of the design mockup: the bars are away while reading and
//! come back when the user interacts.
//!
//! The reader already hid its bars 4 s after the last interaction (and never while they are being used,
//! see `reader::chrome_busy`). While read aloud or speed reading is **playing** the delay is short,
//! 1.5 s, so letting go of the mouse is enough to be left with the page and the open cards; any
//! interaction (pointer move, press, key, a popover) brings the bars back exactly as before. Nothing
//! changes while nothing plays. The bars keep their room (they fade, they are not removed), so the
//! page never moves.

/// Idle time before the bars fade, normally.
pub const IDLE_MS: u64 = 4000;
/// Idle time before the bars fade while read aloud or speed reading plays.
pub const IDLE_PLAYING_MS: u64 = 1500;

pub fn idle_delay_ms(playing: bool) -> u64 {
    if playing { IDLE_PLAYING_MS } else { IDLE_MS }
}

/// Should the bars be faded now? `busy` is the reader's own rule (a popover open, the pointer on a bar
/// or at an edge, keyboard focus in a bar, a button held down): never while busy.
pub fn fade_now(playing: bool, idle_ms: u64, busy: bool) -> bool {
    !busy && idle_ms >= idle_delay_ms(playing)
}

/// The time of the last interaction, with the clock passed in (so tests and the dev hook can move time).
#[derive(Debug, Clone, Copy, Default)]
pub struct Interaction {
    last_ms: u64,
}

impl Interaction {
    pub fn touch(&mut self, now_ms: u64) {
        self.last_ms = now_ms;
    }

    pub fn idle_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.last_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normally_the_delay_is_the_old_four_seconds() {
        assert!(!fade_now(false, IDLE_MS - 1, false));
        assert!(fade_now(false, IDLE_MS, false));
    }

    #[test]
    fn while_playing_the_bars_fade_after_one_and_a_half_seconds() {
        assert!(!fade_now(true, 0, false));
        assert!(!fade_now(true, IDLE_PLAYING_MS - 1, false));
        assert!(fade_now(true, IDLE_PLAYING_MS, false));
        assert_eq!(idle_delay_ms(true), 1500);
        assert_eq!(idle_delay_ms(false), 4000);
    }

    #[test]
    fn real_use_of_the_bars_always_keeps_them() {
        for playing in [false, true] {
            assert!(!fade_now(playing, 1_000_000, true));
        }
    }

    #[test]
    fn a_whole_session_with_an_injected_clock() {
        // press play at t=1000 (an interaction), let go of the mouse, move the pointer, pause, play again
        let mut it = Interaction::default();
        let faded = |it: &Interaction, now: u64, playing: bool, busy: bool| {
            fade_now(playing, it.idle_ms(now), busy)
        };
        it.touch(1000);
        assert!(
            !faded(&it, 1000, true, false),
            "the bars are there when play was pressed"
        );
        assert!(!faded(&it, 2499, true, false));
        assert!(
            faded(&it, 2500, true, false),
            "1.5 s later only the page and the cards are left"
        );
        // the pointer moves: the bars come back (the reader touches on every motion) and fade again
        it.touch(5000);
        assert!(!faded(&it, 5000, true, false));
        assert!(!faded(&it, 6000, true, false));
        assert!(faded(&it, 6500, true, false));
        // pause: the old four seconds apply again, counted from the last interaction
        it.touch(7000);
        assert!(!faded(&it, 10_999, false, false));
        assert!(faded(&it, 11_000, false, false));
        // play again: short delay again
        it.touch(12_000);
        assert!(!faded(&it, 13_000, true, false));
        assert!(faded(&it, 13_500, true, false));
        // while the pointer is on a bar (busy) nothing fades, however long
        assert!(!faded(&it, 99_000, true, true));
    }

    #[test]
    fn the_idle_time_never_underflows() {
        let mut it = Interaction::default();
        it.touch(500);
        assert_eq!(it.idle_ms(100), 0);
        assert_eq!(it.idle_ms(2500), 2000);
    }
}
