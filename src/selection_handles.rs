//! The two teardrop handles at the ends of a text selection (the way touch screens do it): where they
//! sit, which one a finger is on, which character a drag means. No GTK: the canvas gives the
//! rectangles of the selection's first and last line, the overlay draws and moves the handles.

/// (x, y, width, height) in widget coordinates.
pub type Rect = (f64, f64, f64, f64);

/// Radius of the round part of a handle.
pub const RADIUS: f64 = 9.0;
/// Distance from the tip (the edge of the selection) to the centre of the ball.
pub const BALL_DIST: f64 = RADIUS * std::f64::consts::SQRT_2;
/// A finger within this distance of a ball (or of its tip) is on the handle.
pub const GRAB: f64 = 24.0;
/// How far the handles reach from the selection: the action popover points beyond them.
pub const REACH: f64 = BALL_DIST + RADIUS + 4.0;

/// The way a line of the page runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Ltr,
    Rtl,
    /// Vertical writing: lines run down the page.
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// At the first character: the ball is before it (above it).
    Start,
    /// After the last character: the ball is below it.
    End,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handle {
    pub kind: Kind,
    /// The point on the selection the handle points at.
    pub tip: (f64, f64),
    /// The centre of the round part.
    pub ball: (f64, f64),
    /// How thick the line is across its direction: a drag aims half of it into the line from the
    /// tip. (In vertical text the tip is at the end of a column, so a few pixels only.)
    pub line_h: f64,
}

/// The point of the selection a handle points at: the start edge of the first line, the end edge of
/// the last one. In vertical text the tip sits in the middle of the column.
pub fn tip(kind: Kind, line: Rect, dir: Dir) -> (f64, f64) {
    let (x, y, w, h) = line;
    match (kind, dir) {
        (Kind::Start, Dir::Ltr) => (x, y),
        (Kind::Start, Dir::Rtl) => (x + w, y),
        (Kind::Start, Dir::Vertical) => (x + w / 2.0, y),
        (Kind::End, Dir::Ltr) => (x + w, y + h),
        (Kind::End, Dir::Rtl) => (x, y + h),
        (Kind::End, Dir::Vertical) => (x + w / 2.0, y + h),
    }
}

/// The handle for a tip: the ball sits above a start tip and below an end tip, and stays inside
/// `bounds` (width, height of the page area) so it can always be seen and grabbed.
pub fn place(kind: Kind, line: Rect, dir: Dir, bounds: (f64, f64)) -> Handle {
    let t = tip(kind, line, dir);
    let by = match kind {
        Kind::Start => t.1 - BALL_DIST,
        Kind::End => t.1 + BALL_DIST,
    };
    let ball = (
        t.0.clamp(RADIUS, (bounds.0 - RADIUS).max(RADIUS)),
        by.clamp(RADIUS, (bounds.1 - RADIUS).max(RADIUS)),
    );
    Handle {
        kind,
        tip: t,
        ball,
        line_h: if dir == Dir::Vertical { 4.0 } else { line.3 },
    }
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Which handle (index into `handles`) a press at `p` is on: the nearest within [`GRAB`] of its
/// ball or its tip.
pub fn hit(handles: &[Handle], p: (f64, f64)) -> Option<usize> {
    handles
        .iter()
        .enumerate()
        .map(|(i, h)| (i, dist(h.ball, p).min(dist(h.tip, p) + 4.0)))
        .filter(|(_, d)| *d <= GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The point of the text a dragged handle means: the ball is at `ball`, the tip is `BALL_DIST`
/// from it on the side of the text, and the point is half a line into the text from the tip.
pub fn probe(kind: Kind, ball: (f64, f64), line_h: f64) -> (f64, f64) {
    match kind {
        Kind::Start => (ball.0, ball.1 + BALL_DIST + line_h / 2.0),
        Kind::End => (ball.0, ball.1 - BALL_DIST - line_h / 2.0),
    }
}

/// The new selection when the free end is moved to `moving` while the other end `anchor` stays:
/// `(start, end, the moving end is now the start)`. Never empty: when both are the same the
/// selection keeps `keep`.
pub fn reorder(anchor: i32, moving: i32, keep: (i32, i32)) -> ((i32, i32), bool) {
    if moving == anchor {
        return (keep, moving <= keep.0);
    }
    ((anchor.min(moving), anchor.max(moving)), moving < anchor)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '’' || c == '-'
}

/// Moves a boundary `pos` (a gap between characters) to the edge of the word it is in: down to the
/// start of the word when `to_start`, else up to its end. A boundary between words stays.
pub fn snap_word(chars: &[char], pos: usize, to_start: bool) -> usize {
    let pos = pos.min(chars.len());
    if to_start {
        let mut a = pos;
        while a > 0 && is_word_char(chars[a - 1]) && chars.get(a).is_some_and(|c| is_word_char(*c))
        {
            a -= 1;
        }
        a
    } else {
        let mut b = pos;
        while b < chars.len() && b > 0 && is_word_char(chars[b - 1]) && is_word_char(chars[b]) {
            b += 1;
        }
        b
    }
}

/// The area the action popover points at: the selection grown by what the handles reach, and kept
/// inside the page area, so the popover stays near the selection without covering a handle.
pub fn popover_rect(sel: (i32, i32, i32, i32), bounds: (i32, i32)) -> (i32, i32, i32, i32) {
    let reach = REACH.ceil() as i32;
    let (x, y, w, h) = sel;
    let top = (y - reach).max(0);
    let bottom = (y + h + reach).min(bounds.1.max(1));
    (x, top, w.max(1), (bottom - top).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: Rect = (100.0, 200.0, 80.0, 20.0);
    const PAGE: (f64, f64) = (800.0, 600.0);

    #[test]
    fn tips_follow_the_way_the_text_runs() {
        assert_eq!(tip(Kind::Start, LINE, Dir::Ltr), (100.0, 200.0));
        assert_eq!(tip(Kind::End, LINE, Dir::Ltr), (180.0, 220.0));
        assert_eq!(tip(Kind::Start, LINE, Dir::Rtl), (180.0, 200.0));
        assert_eq!(tip(Kind::End, LINE, Dir::Rtl), (100.0, 220.0));
        // a vertical column: the tip is in the middle of it, at the top (start) and bottom (end)
        let col: Rect = (300.0, 50.0, 24.0, 160.0);
        assert_eq!(tip(Kind::Start, col, Dir::Vertical), (312.0, 50.0));
        assert_eq!(tip(Kind::End, col, Dir::Vertical), (312.0, 210.0));
    }

    #[test]
    fn the_ball_is_before_a_start_and_after_an_end() {
        let s = place(Kind::Start, LINE, Dir::Ltr, PAGE);
        let e = place(Kind::End, LINE, Dir::Ltr, PAGE);
        assert!(s.ball.1 < s.tip.1 && (s.tip.1 - s.ball.1 - BALL_DIST).abs() < 1e-9);
        assert!(e.ball.1 > e.tip.1 && (e.ball.1 - e.tip.1 - BALL_DIST).abs() < 1e-9);
        assert_eq!(s.ball.0, 100.0);
        assert_eq!(e.ball.0, 180.0);
        assert_eq!(s.line_h, 20.0);
    }

    #[test]
    fn handles_stay_inside_the_page_area() {
        // a selection on the first line of the page: the start ball cannot go above the page
        let top: Rect = (0.0, 2.0, 50.0, 20.0);
        let h = place(Kind::Start, top, Dir::Ltr, PAGE);
        assert_eq!(h.ball, (RADIUS, RADIUS));
        // on the last line, at the right edge
        let bottom: Rect = (700.0, 585.0, 100.0, 15.0);
        let h = place(Kind::End, bottom, Dir::Ltr, PAGE);
        assert_eq!(h.ball, (PAGE.0 - RADIUS, PAGE.1 - RADIUS));
        // a page area smaller than a handle does not panic
        let h = place(Kind::End, LINE, Dir::Ltr, (5.0, 5.0));
        assert_eq!(h.ball, (RADIUS, RADIUS));
    }

    #[test]
    fn a_finger_picks_the_nearest_handle() {
        let hs = [
            place(Kind::Start, LINE, Dir::Ltr, PAGE),
            place(Kind::End, LINE, Dir::Ltr, PAGE),
        ];
        assert_eq!(hit(&hs, hs[0].ball), Some(0));
        assert_eq!(hit(&hs, (hs[1].ball.0 + 10.0, hs[1].ball.1 + 5.0)), Some(1));
        assert_eq!(hit(&hs, (140.0, 300.0)), None, "far from both");
        // two handles close together (a one-word selection): the nearer ball wins
        let close = [
            place(Kind::Start, (100.0, 200.0, 4.0, 20.0), Dir::Ltr, PAGE),
            place(Kind::End, (100.0, 200.0, 4.0, 20.0), Dir::Ltr, PAGE),
        ];
        assert_eq!(hit(&close, close[0].ball), Some(0));
        assert_eq!(hit(&close, close[1].ball), Some(1));
    }

    #[test]
    fn a_drag_aims_into_the_line_it_points_at() {
        let s = place(Kind::Start, LINE, Dir::Ltr, PAGE);
        let p = probe(Kind::Start, s.ball, 20.0);
        assert_eq!(p, (100.0, 210.0), "half a line below the start tip");
        let e = place(Kind::End, LINE, Dir::Ltr, PAGE);
        let p = probe(Kind::End, e.ball, 20.0);
        assert_eq!(p, (180.0, 210.0), "half a line above the end tip");
    }

    #[test]
    fn a_drag_in_vertical_text_aims_just_into_the_column() {
        // a column 160 px long: the probe must not move half the column length into it
        let col: Rect = (300.0, 50.0, 24.0, 160.0);
        let e = place(Kind::End, col, Dir::Vertical, PAGE);
        let p = probe(Kind::End, e.ball, e.line_h);
        assert!(
            (e.tip.1 - p.1 - 2.0).abs() < 1e-9,
            "{p:?} vs tip {:?}",
            e.tip
        );
        assert_eq!(p.0, 312.0);
        let s = place(Kind::Start, col, Dir::Vertical, PAGE);
        let p = probe(Kind::Start, s.ball, s.line_h);
        assert!((p.1 - s.tip.1 - 2.0).abs() < 1e-9);
    }

    #[test]
    fn the_ends_swap_when_a_handle_crosses_the_other() {
        assert_eq!(reorder(50, 30, (50, 80)), ((30, 50), true));
        assert_eq!(reorder(50, 70, (50, 80)), ((50, 70), false));
        // dragging the start handle past the end: the other end is the anchor and stays the end
        assert_eq!(reorder(80, 90, (50, 80)), ((80, 90), false));
        // on the anchor itself nothing changes, the selection never becomes empty
        assert_eq!(reorder(50, 50, (50, 80)), ((50, 80), true));
    }

    #[test]
    fn a_boundary_snaps_to_the_edge_of_its_word() {
        let t: Vec<char> = "one two-part three".chars().collect();
        // inside "two-part" (chars 4..12)
        assert_eq!(snap_word(&t, 6, true), 4);
        assert_eq!(snap_word(&t, 6, false), 12);
        // between words stays
        assert_eq!(snap_word(&t, 3, true), 3);
        assert_eq!(
            snap_word(&t, 4, false),
            4,
            "the boundary before a word is already an edge"
        );
        assert_eq!(snap_word(&t, 12, true), 12, "so is the one after it");
        assert_eq!(snap_word(&t, 0, true), 0);
        assert_eq!(snap_word(&t, 99, false), t.len());
        assert_eq!(snap_word(&[], 0, true), 0);
    }

    #[test]
    fn the_popover_points_beyond_the_handles() {
        let r = popover_rect((100, 200, 80, 20), (800, 600));
        assert!(r.1 <= 200 - 22 && r.1 + r.3 >= 220 + 22);
        assert_eq!((r.0, r.2), (100, 80));
        // near the edges it stays inside the area
        let r = popover_rect((0, 3, 10, 10), (800, 600));
        assert_eq!(r.1, 0);
        let r = popover_rect((0, 590, 10, 10), (800, 600));
        assert_eq!(r.1 + r.3, 600);
    }
}
