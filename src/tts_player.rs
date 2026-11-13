//! The state model of the read-aloud player card (enhancement 11): which phase it is in, which
//! buttons are enabled, what the seek bar shows. No GTK; `ui::tts_player` draws it.

/// The speeds offered as chips.
pub const SPEEDS: [f64; 6] = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Read aloud is off: no card.
    Stopped,
    /// The card is open but nothing is playing: it was restored with the book, or reading was
    /// paused by leaving the chapter. Play continues from the saved sentence.
    Idle,
    /// The default voice is downloading; reading starts when it is ready.
    Preparing(u32),
    Playing,
    Paused,
    /// `espeak-ng` is missing: the card explains and offers the install command.
    NeedsEspeak,
    /// Something failed (offline, damaged download, ...); the message is shown with *Try again*.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub phase: Phase,
    /// Sentence being read and the number of sentences of the chapter.
    pub sentence: usize,
    pub sentences: usize,
    /// Chapter on screen (0-based) and the number of chapters.
    pub chapter: usize,
    pub chapters: usize,
    pub speed: f64,
    pub collapsed: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            phase: Phase::Stopped,
            sentence: 0,
            sentences: 0,
            chapter: 0,
            chapters: 1,
            speed: 1.0,
            collapsed: false,
        }
    }
}

/// What the card shows and enables for a [`Model`].
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub visible: bool,
    /// The big button shows *pause* (reading) instead of *play*.
    pub shows_pause: bool,
    pub play_enabled: bool,
    pub prev_sentence: bool,
    pub next_sentence: bool,
    pub prev_chapter: bool,
    pub next_chapter: bool,
    pub seek_enabled: bool,
    /// 0.0..=1.0 position in the chapter.
    pub progress: f64,
    /// `12 / 340`, empty when there are no sentences.
    pub progress_text: String,
    /// The download is running: show the progress line and *Cancel*.
    pub preparing: Option<u32>,
    /// The speed chip that is selected (index into [`SPEEDS`]).
    pub speed_chip: Option<usize>,
}

pub fn view(m: &Model) -> View {
    let live = matches!(m.phase, Phase::Playing | Phase::Paused);
    let has = live && m.sentences > 0;
    // a card waiting at a saved place still shows where it is
    let shows_place = (live || m.phase == Phase::Idle) && m.sentences > 0;
    View {
        visible: m.phase != Phase::Stopped,
        shows_pause: m.phase == Phase::Playing,
        play_enabled: matches!(
            m.phase,
            Phase::Playing | Phase::Paused | Phase::Idle | Phase::Failed(_)
        ),
        prev_sentence: has && m.sentence > 0,
        next_sentence: has && m.sentence + 1 < m.sentences,
        prev_chapter: live && m.chapter > 0,
        next_chapter: live && m.chapter + 1 < m.chapters,
        seek_enabled: has && m.sentences > 1,
        progress: if shows_place {
            progress_of(m.sentence, m.sentences)
        } else {
            0.0
        },
        progress_text: if shows_place {
            format!("{} / {}", m.sentence + 1, m.sentences)
        } else {
            String::new()
        },
        preparing: match m.phase {
            Phase::Preparing(p) => Some(p),
            _ => None,
        },
        speed_chip: speed_chip(m.speed),
    }
}

/// Position of sentence `idx` of `count` on the seek bar: the first sentence is 0, the last 1.
pub fn progress_of(idx: usize, count: usize) -> f64 {
    if count <= 1 {
        0.0
    } else {
        (idx.min(count - 1) as f64) / (count - 1) as f64
    }
}

/// The sentence a seek bar value (0..=1) stands for; the inverse of [`progress_of`].
pub fn seek_index(fraction: f64, count: usize) -> usize {
    if count <= 1 || !fraction.is_finite() {
        return 0;
    }
    ((fraction.clamp(0.0, 1.0) * (count - 1) as f64).round() as usize).min(count - 1)
}

/// The chip of `speed` when it is one of the offered speeds.
pub fn speed_chip(speed: f64) -> Option<usize> {
    SPEEDS.iter().position(|s| (s - speed).abs() < 0.01)
}

/// The next offered speed above (`1`) or below (`-1`) `speed`; stays at the ends.
pub fn step_speed(speed: f64, dir: i32) -> f64 {
    if dir >= 0 {
        SPEEDS
            .iter()
            .copied()
            .find(|s| *s > speed + 0.01)
            .unwrap_or(SPEEDS[SPEEDS.len() - 1])
    } else {
        SPEEDS
            .iter()
            .rev()
            .copied()
            .find(|s| *s < speed - 0.01)
            .unwrap_or(SPEEDS[0])
    }
}

/// The gap between a card and the edges of the page.
pub const EDGE: f64 = 8.0;

/// The rectangle `(x, y, width, height)` of a card of `size` in an overlay of `area`, for a place
/// given as fractions of the free room (0 = left/top edge, 1 = right/bottom edge). The card is
/// never wider than the area and never leaves it; a window with no room puts it at the edge gap.
#[cfg(test)]
pub fn rect_of(frac: (f64, f64), area: (f64, f64), size: (f64, f64)) -> (i32, i32, i32, i32) {
    let fin = |v: f64, d: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { d };
    let w = size.0.min((area.0 - 2.0 * EDGE).max(1.0)).max(1.0);
    let h = size.1.min((area.1 - 2.0 * EDGE).max(1.0)).max(1.0);
    let room_x = (area.0 - w - 2.0 * EDGE).max(0.0);
    let room_y = (area.1 - h - 2.0 * EDGE).max(0.0);
    (
        (EDGE + fin(frac.0, 0.5) * room_x).round() as i32,
        (EDGE + fin(frac.1, 1.0) * room_y).round() as i32,
        w.round() as i32,
        h.round() as i32,
    )
}

/// The place (fractions) of a card whose top-left corner is at `top_left` in the overlay: the
/// inverse of [`rect_of`], clamped so the card stays inside the window.
pub fn frac_of(top_left: (f64, f64), area: (f64, f64), size: (f64, f64)) -> (f64, f64) {
    let w = size.0.min((area.0 - 2.0 * EDGE).max(1.0));
    let h = size.1.min((area.1 - 2.0 * EDGE).max(1.0));
    let room_x = (area.0 - w - 2.0 * EDGE).max(0.0);
    let room_y = (area.1 - h - 2.0 * EDGE).max(0.0);
    let f = |v: f64, room: f64, d: f64| {
        if room < 1.0 || !v.is_finite() {
            d
        } else {
            ((v - EDGE) / room).clamp(0.0, 1.0)
        }
    };
    (f(top_left.0, room_x, 0.5), f(top_left.1, room_y, 1.0))
}

/// The place for a drag: the pointer is at `pointer` in the overlay's coordinates and the user took
/// hold of the card at `grab` (in the card's coordinates). Always measured in the overlay, never in
/// the card that is moving, so the card does not feed back into the next position.
pub fn drag_frac(top_left: (f64, f64), area: (f64, f64), size: (f64, f64)) -> (f64, f64) {
    frac_of(top_left, area, size)
}

/// The top-left corner a card must have to stay under `pointer` after being grabbed at `grab`.
pub fn card_corner(pointer: (f64, f64), grab: (f64, f64)) -> (f64, f64) {
    (pointer.0 - grab.0, pointer.1 - grab.1)
}

/// `1.25×` style label of a speed.
pub fn speed_label(speed: f64) -> String {
    let s = format!("{speed:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s}×")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing() -> Model {
        Model {
            phase: Phase::Playing,
            sentence: 3,
            sentences: 10,
            chapter: 1,
            chapters: 4,
            ..Default::default()
        }
    }

    #[test]
    fn nothing_shows_while_read_aloud_is_off() {
        let v = view(&Model::default());
        assert!(!v.visible && !v.play_enabled && !v.seek_enabled);
        assert_eq!(v.progress_text, "");
    }

    #[test]
    fn playing_enables_everything_it_can() {
        let v = view(&playing());
        assert!(v.visible && v.shows_pause && v.play_enabled);
        assert!(v.prev_sentence && v.next_sentence && v.prev_chapter && v.next_chapter);
        assert!(v.seek_enabled);
        assert_eq!(v.progress_text, "4 / 10");
        assert!((v.progress - 3.0 / 9.0).abs() < 1e-9);
        let paused = view(&Model {
            phase: Phase::Paused,
            ..playing()
        });
        assert!(!paused.shows_pause && paused.play_enabled && paused.seek_enabled);
    }

    #[test]
    fn the_ends_of_the_chapter_and_the_book_disable_their_buttons() {
        let first = view(&Model {
            sentence: 0,
            chapter: 0,
            ..playing()
        });
        assert!(!first.prev_sentence && !first.prev_chapter);
        assert!(first.next_sentence && first.next_chapter);
        let last = view(&Model {
            sentence: 9,
            chapter: 3,
            ..playing()
        });
        assert!(!last.next_sentence && !last.next_chapter);
        assert_eq!(last.progress, 1.0);
        let one = view(&Model {
            sentences: 1,
            sentence: 0,
            ..playing()
        });
        assert!(!one.seek_enabled && !one.next_sentence);
    }

    #[test]
    fn a_restored_card_waits_at_its_place_and_can_be_played() {
        let v = view(&Model {
            phase: Phase::Idle,
            sentence: 11,
            sentences: 40,
            chapter: 2,
            chapters: 5,
            ..Default::default()
        });
        assert!(v.visible && v.play_enabled && !v.shows_pause);
        assert_eq!(v.progress_text, "12 / 40", "the saved place is shown");
        assert!((v.progress - 11.0 / 39.0).abs() < 1e-9);
        assert!(
            !v.seek_enabled && !v.prev_sentence && !v.next_sentence,
            "nothing moves while it waits"
        );
    }

    #[test]
    fn preparing_shows_the_percent_and_cannot_play_yet() {
        let v = view(&Model {
            phase: Phase::Preparing(42),
            ..Default::default()
        });
        assert!(v.visible && !v.play_enabled && !v.seek_enabled && !v.shows_pause);
        assert_eq!(v.preparing, Some(42));
        assert!(!v.next_chapter && !v.prev_sentence);
    }

    #[test]
    fn a_failure_can_be_retried_with_the_play_button() {
        let v = view(&Model {
            phase: Phase::Failed("offline".into()),
            ..Default::default()
        });
        assert!(v.visible && v.play_enabled && !v.shows_pause);
        let e = view(&Model {
            phase: Phase::NeedsEspeak,
            ..Default::default()
        });
        assert!(e.visible && !e.play_enabled);
    }

    #[test]
    fn the_seek_bar_and_the_sentence_agree() {
        for count in [2usize, 3, 10, 341] {
            for i in 0..count {
                assert_eq!(seek_index(progress_of(i, count), count), i, "{i}/{count}");
            }
        }
        assert_eq!(seek_index(0.5, 11), 5);
        assert_eq!(seek_index(2.0, 5), 4);
        assert_eq!(seek_index(-1.0, 5), 0);
        assert_eq!(seek_index(f64::NAN, 5), 0);
        assert_eq!(seek_index(0.7, 0), 0);
        assert_eq!(progress_of(0, 1), 0.0);
    }

    #[test]
    fn speed_chips_step_and_stop_at_the_ends() {
        assert_eq!(speed_chip(1.0), Some(1));
        assert_eq!(speed_chip(1.1), None);
        assert_eq!(step_speed(1.0, 1), 1.25);
        assert_eq!(step_speed(1.1, 1), 1.25);
        assert_eq!(step_speed(2.0, 1), 2.0);
        assert_eq!(step_speed(1.0, -1), 0.75);
        assert_eq!(step_speed(0.5, -1), 0.75);
        assert_eq!(step_speed(1.1, -1), 1.0);
        assert_eq!(speed_label(1.0), "1×");
        assert_eq!(speed_label(1.25), "1.25×");
        assert_eq!(speed_label(0.75), "0.75×");
        assert_eq!(speed_label(1.5), "1.5×");
    }

    #[test]
    fn a_place_becomes_a_rectangle_inside_the_window() {
        let (area, size) = ((800.0, 600.0), (400.0, 100.0));
        // centred at the bottom: the default
        assert_eq!(rect_of((0.5, 1.0), area, size), (200, 492, 400, 100));
        assert_eq!(rect_of((0.0, 0.0), area, size), (8, 8, 400, 100));
        assert_eq!(rect_of((1.0, 1.0), area, size), (392, 492, 400, 100));
        // out-of-range and broken fractions are made sane
        assert_eq!(
            rect_of((9.0, -3.0), area, size),
            rect_of((1.0, 0.0), area, size)
        );
        assert_eq!(
            rect_of((f64::NAN, f64::NAN), area, size),
            rect_of((0.5, 1.0), area, size)
        );
        // a window narrower than the card shrinks it and keeps it inside
        let r = rect_of((0.5, 1.0), (300.0, 600.0), size);
        assert_eq!((r.0, r.2), (8, 284));
        // no room at all: the edge gap, never a negative position
        let r = rect_of((0.5, 0.5), (10.0, 10.0), size);
        assert!(r.0 >= 0 && r.1 >= 0 && r.2 >= 1 && r.3 >= 1);
    }

    #[test]
    fn frac_of_is_the_inverse_of_rect_of() {
        let (area, size) = ((1280.0, 720.0), (360.0, 96.0));
        for f in [
            (0.0, 0.0),
            (0.5, 1.0),
            (0.25, 0.8),
            (1.0, 0.0),
            (0.123, 0.456),
        ] {
            let (x, y, ..) = rect_of(f, area, size);
            let back = frac_of((x as f64, y as f64), area, size);
            assert!(
                (back.0 - f.0).abs() < 0.002 && (back.1 - f.1).abs() < 0.002,
                "{f:?} -> {back:?}"
            );
        }
        // dragged past every edge: clamped to the window
        assert_eq!(frac_of((-500.0, -500.0), area, size), (0.0, 0.0));
        assert_eq!(frac_of((9999.0, 9999.0), area, size), (1.0, 1.0));
        // a window with no room keeps the defaults instead of dividing by zero
        assert_eq!(
            frac_of((50.0, 50.0), (200.0, 100.0), (400.0, 200.0)),
            (0.5, 1.0)
        );
        assert_eq!(frac_of((f64::NAN, 3.0), area, size).0, 0.5);
    }

    #[test]
    fn the_same_fraction_keeps_its_place_in_a_resized_window() {
        let size = (360.0, 96.0);
        let f = (0.3, 0.6);
        let (x1, y1, ..) = rect_of(f, (1000.0, 700.0), size);
        let (x2, y2, ..) = rect_of(f, (1400.0, 900.0), size);
        assert!(x2 > x1 && y2 > y1, "grows with the window");
        // the fraction itself is untouched by the resize: re-applying it is idempotent
        assert_eq!(
            rect_of(f, (1400.0, 900.0), size),
            rect_of(f, (1400.0, 900.0), size)
        );
        let back = frac_of((x2 as f64, y2 as f64), (1400.0, 900.0), size);
        assert!((back.0 - f.0).abs() < 0.002 && (back.1 - f.1).abs() < 0.002);
    }

    #[test]
    fn a_drag_follows_the_pointer_one_to_one_whatever_the_card_does() {
        // The pointer moves 5 px per step; the card is grabbed 40 px from its left edge and 10 from
        // its top. After every step the card's corner is the pointer minus the grab point, and the
        // place computed from it puts the card exactly there: no feedback, no lag.
        let (area, size, grab) = ((1280.0, 720.0), (360.0, 96.0), (40.0, 10.0));
        let mut pointer = (700.0, 500.0);
        for _ in 0..60 {
            pointer = (pointer.0 - 5.0, pointer.1 - 3.0);
            let corner = card_corner(pointer, grab);
            let f = drag_frac(corner, area, size);
            let (x, y, ..) = rect_of(f, area, size);
            assert!((x as f64 - corner.0).abs() <= 1.0, "x {x} vs {}", corner.0);
            assert!((y as f64 - corner.1).abs() <= 1.0, "y {y} vs {}", corner.1);
        }
        // the card stays under the pointer even when it is pushed against an edge and back
        let f = drag_frac(card_corner((2.0, 700.0), grab), area, size);
        assert_eq!(f, (0.0, 1.0));
        let back = drag_frac(card_corner((300.0, 600.0), grab), area, size);
        let (x, y, ..) = rect_of(back, area, size);
        assert_eq!((x, y), (260, 590));
    }
}
