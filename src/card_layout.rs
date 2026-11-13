//! Where the floating cards of the reader (read aloud, speed reading) end up so that they never sit on
//! top of each other or of a popover: pure geometry, no GTK.
//!
//! The user's own positions (fractions of the free room in the window, as stored) are *wishes*; this
//! turns the wishes into final rectangles. A card that was never moved has the default place; when both
//! are open and none was moved they stand side by side, centred as a group at the bottom of the page
//! area (stacked when the group does not fit the width). A card the user moved keeps its wish; the other
//! one goes to the nearest free spot (beside, then above or below). The card being dragged, then the
//! one moved last, come first, so the other yields. Popovers and other obstacles are avoided the same
//! way, and a card returns to its wish when the obstacle is gone, because the final places are always
//! computed again from the wishes (same input, same output).

/// Gap kept between cards and between a card and an obstacle.
pub const GAP: f64 = 12.0;
/// Gap to the edges of the page area.
pub const EDGE: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    /// The rectangle grown by `m` on every side.
    pub fn grown(&self, m: f64) -> Rect {
        Rect::new(self.x - m, self.y - m, self.w + 2.0 * m, self.h + 2.0 * m)
    }

    /// Do the interiors overlap? Rectangles that only touch do not.
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() - 1e-6
            && o.x < self.right() - 1e-6
            && self.y < o.bottom() - 1e-6
            && o.y < self.bottom() - 1e-6
    }

    #[cfg(test)]
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(
            x,
            y,
            self.right().max(o.right()) - x,
            self.bottom().max(o.bottom()) - y,
        )
    }
}

/// A card to place.
#[derive(Debug, Clone, PartialEq)]
pub struct CardIn {
    pub id: u8,
    pub size: (f64, f64),
    /// Where the user put it, as fractions of the free room (None: never moved).
    pub wish: Option<(f64, f64)>,
    /// Where it goes when never moved and alone.
    pub default_frac: (f64, f64),
    /// 0 is placed first: the card being dragged, then the one moved last, then the rest.
    pub rank: u8,
}

/// The rectangle for a card of `size` at the place `frac` of the free room (what the stored positions
/// mean): never wider than the area, never outside it.
pub fn rect_at(frac: (f64, f64), area: (f64, f64), size: (f64, f64)) -> Rect {
    let fin = |v: f64, d: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { d };
    let w = size.0.min((area.0 - 2.0 * EDGE).max(1.0)).max(1.0);
    let h = size.1.min((area.1 - 2.0 * EDGE).max(1.0)).max(1.0);
    let room_x = (area.0 - w - 2.0 * EDGE).max(0.0);
    let room_y = (area.1 - h - 2.0 * EDGE).max(0.0);
    Rect::new(
        EDGE + fin(frac.0, 0.5) * room_x,
        EDGE + fin(frac.1, 1.0) * room_y,
        w,
        h,
    )
}

/// The inverse: the fractions for a card whose top-left corner is at `top_left`.
#[cfg(test)]
pub fn frac_for(top_left: (f64, f64), area: (f64, f64), size: (f64, f64)) -> (f64, f64) {
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

fn clamp_into(r: Rect, area: (f64, f64)) -> Rect {
    let maxx = (area.0 - EDGE - r.w).max(EDGE);
    let maxy = (area.1 - EDGE - r.h).max(EDGE);
    Rect::new(r.x.clamp(EDGE, maxx), r.y.clamp(EDGE, maxy), r.w, r.h)
}

fn blocked(r: &Rect, blockers: &[Rect]) -> bool {
    blockers
        .iter()
        .any(|b| r.grown(GAP / 2.0).intersects(&b.grown(GAP / 2.0)))
}

fn overlap_area(r: &Rect, blockers: &[Rect]) -> f64 {
    blockers
        .iter()
        .map(|b| {
            let g = r.grown(GAP / 2.0);
            let b = b.grown(GAP / 2.0);
            let w = (g.right().min(b.right()) - g.x.max(b.x)).max(0.0);
            let h = (g.bottom().min(b.bottom()) - g.y.max(b.y)).max(0.0);
            w * h
        })
        .sum()
}

/// The nearest place to `want` (same size) that is clear of every blocker by the gap.
fn nearest_free(want: Rect, blockers: &[Rect], area: (f64, f64)) -> Rect {
    let want = clamp_into(want, area);
    if !blocked(&want, blockers) {
        return want;
    }
    let mut cands: Vec<Rect> = Vec::new();
    for b in blockers {
        cands.extend([
            Rect::new(b.right() + GAP, want.y, want.w, want.h),
            Rect::new(b.x - GAP - want.w, want.y, want.w, want.h),
            Rect::new(want.x, b.y - GAP - want.h, want.w, want.h),
            Rect::new(want.x, b.bottom() + GAP, want.w, want.h),
            Rect::new(b.right() + GAP, b.y, want.w, want.h),
            Rect::new(b.x - GAP - want.w, b.y, want.w, want.h),
        ]);
    }
    let mut best: Option<(f64, Rect)> = None;
    for c in cands {
        let c = clamp_into(c, area);
        if blocked(&c, blockers) {
            continue;
        }
        let d = (c.x - want.x).powi(2) + (c.y - want.y).powi(2);
        if best.is_none_or(|(bd, _)| d < bd - 1e-9) {
            best = Some((d, c));
        }
    }
    if let Some((_, r)) = best {
        return r;
    }
    // nowhere is free (a tiny window): the place with the least overlap on a coarse grid
    let mut least: Option<(f64, Rect)> = None;
    let steps = 12;
    for i in 0..=steps {
        for j in 0..=steps {
            let x = EDGE + (area.0 - 2.0 * EDGE - want.w).max(0.0) * i as f64 / steps as f64;
            let y = EDGE + (area.1 - 2.0 * EDGE - want.h).max(0.0) * j as f64 / steps as f64;
            let c = Rect::new(x, y, want.w, want.h);
            let o = overlap_area(&c, blockers);
            if least.is_none_or(|(lo, _)| o < lo - 1e-9) {
                least = Some((o, c));
            }
        }
    }
    least.map_or(want, |(_, r)| r)
}

/// The final rectangles, in the order of `cards`. `area` is the page area; `obstacles` are rectangles
/// to stay clear of (popovers, bars that are shown).
pub fn layout(area: (f64, f64), cards: &[CardIn], obstacles: &[Rect]) -> Vec<(u8, Rect)> {
    let unmoved: Vec<&CardIn> = cards.iter().filter(|c| c.wish.is_none()).collect();
    let mut wants: Vec<Rect> = cards
        .iter()
        .map(|c| rect_at(c.wish.unwrap_or(c.default_frac), area, c.size))
        .collect();
    if let [a, b] = unmoved[..] {
        let ra = rect_at(a.default_frac, area, a.size);
        let rb = rect_at(b.default_frac, area, b.size);
        let group_w = ra.w + GAP + rb.w;
        let bottom = area.1 - EDGE;
        let (pa, pb) = if group_w <= area.0 - 2.0 * EDGE {
            let x0 = (area.0 - group_w) / 2.0;
            (
                Rect::new(x0, bottom - ra.h, ra.w, ra.h),
                Rect::new(x0 + ra.w + GAP, bottom - rb.h, rb.w, rb.h),
            )
        } else {
            let pa = Rect::new((area.0 - ra.w) / 2.0, bottom - ra.h, ra.w, ra.h);
            let pb = Rect::new((area.0 - rb.w) / 2.0, pa.y - GAP - rb.h, rb.w, rb.h);
            (pa, pb)
        };
        for (i, c) in cards.iter().enumerate() {
            if c.id == a.id {
                wants[i] = clamp_into(pa, area);
            } else if c.id == b.id {
                wants[i] = clamp_into(pb, area);
            }
        }
    }
    let mut order: Vec<usize> = (0..cards.len()).collect();
    order.sort_by_key(|&i| (cards[i].rank, cards[i].id));
    let mut blockers: Vec<Rect> = obstacles.to_vec();
    let mut placed: Vec<Rect> = wants.clone();
    for i in order {
        let r = nearest_free(wants[i], &blockers, area);
        blockers.push(r);
        placed[i] = r;
    }
    cards.iter().zip(placed).map(|(c, r)| (c.id, r)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: (f64, f64) = (1280.0, 700.0);

    fn tts(wish: Option<(f64, f64)>, rank: u8) -> CardIn {
        CardIn {
            id: 0,
            size: (360.0, 100.0),
            wish,
            default_frac: (0.5, 1.0),
            rank,
        }
    }

    fn rsvp(wish: Option<(f64, f64)>, rank: u8) -> CardIn {
        CardIn {
            id: 1,
            size: (340.0, 110.0),
            wish,
            default_frac: (0.5, 0.8),
            rank,
        }
    }

    fn pair(out: &[(u8, Rect)]) -> (Rect, Rect) {
        (out[0].1, out[1].1)
    }

    fn disjoint(a: &Rect, b: &Rect) -> bool {
        !a.grown(GAP / 2.0 - 0.01)
            .intersects(&b.grown(GAP / 2.0 - 0.01))
    }

    #[test]
    fn two_cards_nobody_moved_stand_side_by_side_centred_as_a_group() {
        let out = layout(AREA, &[tts(None, 3), rsvp(None, 3)], &[]);
        let (a, b) = pair(&out);
        assert!(!a.intersects(&b));
        assert!((b.x - (a.right() + GAP)).abs() < 1e-6);
        let g = a.union(&b);
        assert!(((g.x + g.w / 2.0) - AREA.0 / 2.0).abs() < 1e-6);
        assert!((a.bottom() - (AREA.1 - EDGE)).abs() < 1e-6);
        assert!((b.bottom() - (AREA.1 - EDGE)).abs() < 1e-6);
    }

    #[test]
    fn a_narrow_window_stacks_them_and_never_overlaps() {
        let out = layout((500.0, 700.0), &[tts(None, 3), rsvp(None, 3)], &[]);
        let (a, b) = pair(&out);
        assert!(!a.intersects(&b));
        assert!(b.bottom() <= a.y - GAP + 1e-6);
        assert!((a.x + a.w / 2.0 - 250.0).abs() < 1e-6);
        assert!((b.x + b.w / 2.0 - 250.0).abs() < 1e-6);
    }

    #[test]
    fn one_card_alone_keeps_its_own_place() {
        let only = layout(AREA, &[tts(None, 3)], &[]);
        let r = only[0].1;
        assert!((r.x + r.w / 2.0 - AREA.0 / 2.0).abs() < 1e-6);
        assert!((r.bottom() - (AREA.1 - EDGE)).abs() < 1e-6);
        let moved = layout(AREA, &[rsvp(Some((0.1, 0.2)), 2)], &[]);
        assert_eq!(moved[0].1, rect_at((0.1, 0.2), AREA, (340.0, 110.0)));
    }

    #[test]
    fn a_card_the_user_moved_keeps_its_place_and_the_other_moves_beside_it() {
        let wish = (0.5, 0.97);
        let out = layout(AREA, &[tts(None, 3), rsvp(Some(wish), 2)], &[]);
        let (a, b) = pair(&out);
        assert_eq!(b, rect_at(wish, AREA, (340.0, 110.0)));
        assert!(disjoint(&a, &b));
        assert!((a.y - b.y).abs() < 150.0);
    }

    #[test]
    fn the_card_being_dragged_over_the_other_pushes_it_aside() {
        let on_top = (0.30, 0.55);
        let out = layout(AREA, &[tts(Some(on_top), 2), rsvp(Some(on_top), 0)], &[]);
        let (a, b) = pair(&out);
        assert_eq!(b, rect_at(on_top, AREA, (340.0, 110.0)));
        assert!(disjoint(&a, &b));
        let after = layout(
            AREA,
            &[tts(Some(on_top), 2), rsvp(Some((0.9, 0.1)), 0)],
            &[],
        );
        assert_eq!(after[0].1, rect_at(on_top, AREA, (360.0, 100.0)));
    }

    #[test]
    fn a_popover_is_avoided_and_the_card_returns_when_it_is_gone() {
        let wish = (0.5, 0.5);
        let home = layout(AREA, &[rsvp(Some(wish), 2)], &[])[0].1;
        let popover = Rect::new(home.x + 40.0, home.y - 20.0, 260.0, 160.0);
        let moved = layout(AREA, &[rsvp(Some(wish), 2)], &[popover]);
        assert!(disjoint(&moved[0].1, &popover));
        assert_ne!(moved[0].1, home);
        assert_eq!(layout(AREA, &[rsvp(Some(wish), 2)], &[])[0].1, home);
    }

    #[test]
    fn both_cards_and_a_popover_all_stay_clear_of_each_other() {
        let popover = Rect::new(500.0, 560.0, 280.0, 120.0);
        let out = layout(AREA, &[tts(None, 3), rsvp(None, 3)], &[popover]);
        let (a, b) = pair(&out);
        assert!(disjoint(&a, &b) && disjoint(&a, &popover) && disjoint(&b, &popover));
        for r in [a, b] {
            assert!(r.x >= EDGE - 1e-6 && r.y >= EDGE - 1e-6);
            assert!(r.right() <= AREA.0 - EDGE + 1e-6 && r.bottom() <= AREA.1 - EDGE + 1e-6);
        }
    }

    #[test]
    fn bars_that_are_shown_count_as_obstacles() {
        let header = Rect::new(0.0, 0.0, AREA.0, 46.0);
        let footer = Rect::new(0.0, AREA.1 - 50.0, AREA.0, 50.0);
        let out = layout(AREA, &[tts(None, 3), rsvp(None, 3)], &[header, footer]);
        for (_, r) in &out {
            assert!(disjoint(r, &header) && disjoint(r, &footer), "{r:?}");
        }
    }

    #[test]
    fn cards_stay_inside_the_window_whatever_the_size_and_the_wish() {
        for area in [
            (1280.0, 700.0),
            (400.0, 300.0),
            (300.0, 200.0),
            (50.0, 50.0),
        ] {
            for wish in [(0.0, 0.0), (1.0, 1.0), (f64::NAN, -3.0), (9.0, 9.0)] {
                let out = layout(area, &[tts(Some(wish), 0), rsvp(Some(wish), 1)], &[]);
                for (_, r) in out {
                    assert!(r.w >= 1.0 && r.h >= 1.0);
                    assert!(r.x >= -1e-6 && r.y >= -1e-6, "{area:?} {wish:?} {r:?}");
                    assert!(
                        r.right() <= area.0.max(2.0 * EDGE + 1.0) + 1.0,
                        "{area:?} {r:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_result_is_stable_and_does_not_jitter_when_rects_touch() {
        let a = tts(Some((0.5, 0.5)), 1);
        let near = rect_at((0.5, 0.5), AREA, a.size);
        let b = rsvp(
            Some(frac_for((near.right() + 3.0, near.y), AREA, (340.0, 110.0))),
            2,
        );
        let first = layout(AREA, &[a.clone(), b.clone()], &[]);
        assert_eq!(first, layout(AREA, &[a.clone(), b.clone()], &[]));
        let (ra, rb) = pair(&first);
        assert!(disjoint(&ra, &rb));
        let fb = rsvp(Some(frac_for((rb.x, rb.y), AREA, (340.0, 110.0))), 2);
        let third = layout(AREA, &[a, fb], &[]);
        assert!((third[1].1.x - rb.x).abs() < 0.6 && (third[1].1.y - rb.y).abs() < 0.6);
        let r1 = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(!r1.intersects(&Rect::new(10.0, 0.0, 10.0, 10.0)));
        assert!(r1.intersects(&Rect::new(9.9, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn the_stored_fractions_and_the_rectangles_agree() {
        let size = (360.0, 100.0);
        for f in [
            (0.0, 0.0),
            (0.5, 1.0),
            (0.25, 0.8),
            (1.0, 0.0),
            (0.123, 0.456),
        ] {
            let r = rect_at(f, AREA, size);
            let back = frac_for((r.x, r.y), AREA, size);
            assert!((back.0 - f.0).abs() < 1e-6 && (back.1 - f.1).abs() < 1e-6);
        }
        assert_eq!(
            frac_for((50.0, 50.0), (200.0, 100.0), (400.0, 200.0)),
            (0.5, 1.0)
        );
    }

    #[test]
    fn resizing_keeps_the_wish_and_recomputes_the_places() {
        let wish = Some((0.3, 0.6));
        for area in [(1600.0, 900.0), (900.0, 600.0)] {
            let out = layout(area, &[tts(wish, 2), rsvp(None, 3)], &[]);
            assert!(disjoint(&out[0].1, &out[1].1));
        }
        let small = layout((900.0, 600.0), &[tts(wish, 2), rsvp(None, 3)], &[]);
        let fb = frac_for((small[0].1.x, small[0].1.y), (900.0, 600.0), (360.0, 100.0));
        assert!((fb.0 - 0.3).abs() < 1e-6 && (fb.1 - 0.6).abs() < 1e-6);
    }
}
