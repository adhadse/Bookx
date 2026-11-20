//! Which pages share a screen, where they sit, and which way the pages turn.

/// A page wider than this (width / height) is a double page already.
pub const WIDE_RATIO: f64 = 1.15;

pub fn is_wide(w: u32, h: u32) -> bool {
    h > 0 && (w as f64) > h as f64 * WIDE_RATIO
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpreadOpts {
    /// Two pages per screen (otherwise every page is a screen of its own).
    pub two_page: bool,
    /// The first page (the cover) is a screen of its own.
    pub cover_alone: bool,
    /// A wide page is a screen of its own.
    pub wide_alone: bool,
}

/// The pages of every screen in reading order. `double` marks pages that ComicInfo says are double
/// pages (checked besides the shape of the picture).
pub fn plan_spreads(sizes: &[(u32, u32)], double: &[bool], o: SpreadOpts) -> Vec<Vec<usize>> {
    let n = sizes.len();
    let wide = |i: usize| {
        i < n && (is_wide(sizes[i].0, sizes[i].1) || double.get(i).copied().unwrap_or(false))
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let alone = !o.two_page
            || (i == 0 && o.cover_alone)
            || (o.wide_alone && wide(i))
            || i + 1 >= n
            || (o.wide_alone && wide(i + 1));
        if alone {
            out.push(vec![i]);
            i += 1;
        } else {
            out.push(vec![i, i + 1]);
            i += 2;
        }
    }
    out
}

/// The screen a page is on.
pub fn spread_of(plan: &[Vec<usize>], page: usize) -> usize {
    plan.iter()
        .position(|s| s.contains(&page))
        .unwrap_or_else(|| plan.len().saturating_sub(1))
}

/// A page placed in the content of a screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub page: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Places the pages of one screen side by side at the same height (the tallest page sets it).
/// Returns the placements and the size of the whole. In `rtl` the first page is on the right.
pub fn place_spread(sizes: &[(u32, u32)], pages: &[usize], rtl: bool) -> (Vec<Placed>, (f64, f64)) {
    let h = pages
        .iter()
        .filter_map(|p| sizes.get(*p))
        .map(|s| s.1 as f64)
        .fold(0.0, f64::max)
        .max(1.0);
    let widths: Vec<f64> = pages
        .iter()
        .map(|p| {
            let (pw, ph) = sizes.get(*p).copied().unwrap_or((1, 1));
            pw as f64 * h / ph.max(1) as f64
        })
        .collect();
    let total: f64 = widths.iter().sum();
    let mut out: Vec<Placed> = Vec::new();
    let mut x = if rtl { total } else { 0.0 };
    for (p, w) in pages.iter().zip(&widths) {
        if rtl {
            x -= w;
        }
        out.push(Placed {
            page: *p,
            x,
            y: 0.0,
            w: *w,
            h,
        });
        if !rtl {
            x += w;
        }
    }
    (out, (total, h))
}

/// The strip of a webtoon: every page as wide as the widest, one under the other.
/// Returns the placements and the size of the whole.
pub fn strip(sizes: &[(u32, u32)]) -> (Vec<Placed>, (f64, f64)) {
    let w = sizes.iter().map(|s| s.0 as f64).fold(1.0, f64::max);
    let mut y = 0.0;
    let mut out = Vec::with_capacity(sizes.len());
    for (i, (pw, ph)) in sizes.iter().enumerate() {
        let h = *ph as f64 * w / (*pw).max(1) as f64;
        out.push(Placed {
            page: i,
            x: 0.0,
            y,
            w,
            h,
        });
        y += h;
    }
    (out, (w, y))
}

/// The pages of a strip that show between two heights of the content.
pub fn visible_in_strip(placed: &[Placed], top: f64, bottom: f64) -> std::ops::Range<usize> {
    let first = placed.partition_point(|p| p.y + p.h <= top);
    let last = placed.partition_point(|p| p.y < bottom);
    first..last.max(first)
}

/// The page that fills most of the screen between two heights of the strip.
pub fn strip_current(placed: &[Placed], top: f64, bottom: f64) -> usize {
    let mid = top + (bottom - top) * 0.35;
    let i = placed.partition_point(|p| p.y + p.h <= mid);
    i.min(placed.len().saturating_sub(1))
}

/// A turn of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    Next,
    Prev,
}

/// What an arrow key (or a tap on the right or left side) does: in a right to left book the left
/// side goes forward.
pub fn horizontal_turn(rtl: bool, right: bool) -> Turn {
    if right != rtl {
        Turn::Next
    } else {
        Turn::Prev
    }
}

/// A click at `x` of `width`: the outer thirds turn the page, the middle shows the bars.
pub fn click_zone(x: f64, width: f64, rtl: bool) -> Option<Turn> {
    if width <= 0.0 {
        return None;
    }
    let f = x / width;
    if f < 0.28 {
        Some(horizontal_turn(rtl, false))
    } else if f > 0.72 {
        Some(horizontal_turn(rtl, true))
    } else {
        None
    }
}

/// The screen after a turn, stopping at both ends.
pub fn turned(cur: usize, count: usize, t: Turn) -> Option<usize> {
    match t {
        Turn::Next if cur + 1 < count => Some(cur + 1),
        Turn::Prev if cur > 0 => Some(cur - 1),
        _ => None,
    }
}

/// Reading progress 0..1 for a page (the last page is 1).
pub fn progress(page: usize, count: usize) -> f64 {
    if count <= 1 {
        return if count == 1 { 1.0 } else { 0.0 };
    }
    (page as f64 / (count - 1) as f64).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const O: SpreadOpts = SpreadOpts {
        two_page: true,
        cover_alone: true,
        wide_alone: true,
    };

    fn sizes(n: usize) -> Vec<(u32, u32)> {
        vec![(400, 600); n]
    }

    #[test]
    fn single_page_mode_has_one_page_per_screen() {
        let p = plan_spreads(
            &sizes(3),
            &[],
            SpreadOpts {
                two_page: false,
                ..O
            },
        );
        assert_eq!(p, vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn cover_alone_then_pairs() {
        let p = plan_spreads(&sizes(6), &[], O);
        assert_eq!(p, vec![vec![0], vec![1, 2], vec![3, 4], vec![5]]);
        let p = plan_spreads(
            &sizes(4),
            &[],
            SpreadOpts {
                cover_alone: false,
                ..O
            },
        );
        assert_eq!(p, vec![vec![0, 1], vec![2, 3]]);
    }

    #[test]
    fn wide_pages_stand_alone_and_shift_the_pairs() {
        let mut s = sizes(7);
        s[3] = (900, 600);
        let p = plan_spreads(&s, &[], O);
        assert_eq!(
            p,
            vec![vec![0], vec![1, 2], vec![3], vec![4, 5], vec![6]],
            "{p:?}"
        );
        // not alone: it pairs up like any page
        let p = plan_spreads(
            &s,
            &[],
            SpreadOpts {
                wide_alone: false,
                ..O
            },
        );
        assert_eq!(p[2], vec![3, 4]);
    }

    #[test]
    fn a_page_before_a_wide_page_is_alone() {
        let mut s = sizes(5);
        s[2] = (1000, 600);
        let p = plan_spreads(&s, &[], O);
        assert_eq!(p, vec![vec![0], vec![1], vec![2], vec![3, 4]]);
    }

    #[test]
    fn comic_info_double_page_flag_counts_as_wide() {
        let p = plan_spreads(&sizes(5), &[false, false, true], O);
        assert_eq!(p, vec![vec![0], vec![1], vec![2], vec![3, 4]]);
    }

    #[test]
    fn every_page_is_on_exactly_one_screen() {
        for n in 0..20 {
            let mut s = sizes(n);
            if n > 5 {
                s[4] = (900, 600);
            }
            let p = plan_spreads(&s, &[], O);
            let flat: Vec<usize> = p.iter().flatten().copied().collect();
            assert_eq!(flat, (0..n).collect::<Vec<_>>());
            for page in 0..n {
                assert!(p[spread_of(&p, page)].contains(&page));
            }
        }
    }

    #[test]
    fn spread_placement_follows_the_direction() {
        let s = vec![(400, 600), (400, 600)];
        let (a, (w, h)) = place_spread(&s, &[0, 1], false);
        assert_eq!((w, h), (800.0, 600.0));
        assert_eq!((a[0].page, a[0].x), (0, 0.0));
        assert_eq!((a[1].page, a[1].x), (1, 400.0));
        let (a, _) = place_spread(&s, &[0, 1], true);
        assert_eq!(
            (a[0].page, a[0].x),
            (0, 400.0),
            "manga: first page on the right"
        );
        assert_eq!((a[1].page, a[1].x), (1, 0.0));
    }

    #[test]
    fn pages_of_other_sizes_are_scaled_to_the_same_height() {
        let s = vec![(400, 600), (1000, 1500)];
        let (a, (w, h)) = place_spread(&s, &[0, 1], false);
        assert_eq!(h, 1500.0);
        assert!((a[0].w - 1000.0).abs() < 1e-9 && (a[1].w - 1000.0).abs() < 1e-9);
        assert!((w - 2000.0).abs() < 1e-9);
        let (one, (w1, _)) = place_spread(&s, &[0], false);
        assert_eq!((one.len(), w1), (1, 400.0));
    }

    #[test]
    fn strip_stacks_pages_at_the_widest_width() {
        let s = vec![(800, 1600), (400, 400), (800, 800)];
        let (p, (w, h)) = strip(&s);
        assert_eq!(w, 800.0);
        assert_eq!((p[1].y, p[1].h), (1600.0, 800.0));
        assert_eq!(h, 1600.0 + 800.0 + 800.0);
        assert_eq!(visible_in_strip(&p, 0.0, 100.0), 0..1);
        assert_eq!(visible_in_strip(&p, 1500.0, 1700.0), 0..2);
        assert_eq!(visible_in_strip(&p, 1600.0, 2400.0), 1..2);
        assert_eq!(visible_in_strip(&p, 2000.0, 9999.0), 1..3);
        assert!(visible_in_strip(&p, 5000.0, 6000.0).is_empty());
        assert_eq!(strip_current(&p, 0.0, 1000.0), 0);
        assert_eq!(strip_current(&p, 1500.0, 2500.0), 1);
        assert_eq!(strip_current(&p, 3100.0, 4000.0), 2);
        assert_eq!(strip(&[]).1, (1.0, 0.0));
    }

    #[test]
    fn arrows_follow_the_reading_direction() {
        assert_eq!(horizontal_turn(false, true), Turn::Next);
        assert_eq!(horizontal_turn(false, false), Turn::Prev);
        assert_eq!(horizontal_turn(true, true), Turn::Prev);
        assert_eq!(horizontal_turn(true, false), Turn::Next);
    }

    #[test]
    fn click_zones() {
        assert_eq!(click_zone(10.0, 1000.0, false), Some(Turn::Prev));
        assert_eq!(click_zone(990.0, 1000.0, false), Some(Turn::Next));
        assert_eq!(click_zone(500.0, 1000.0, false), None);
        assert_eq!(click_zone(10.0, 1000.0, true), Some(Turn::Next));
        assert_eq!(click_zone(990.0, 1000.0, true), Some(Turn::Prev));
        assert_eq!(click_zone(1.0, 0.0, false), None);
    }

    #[test]
    fn turning_stops_at_the_ends_and_progress_is_bounded() {
        assert_eq!(turned(0, 3, Turn::Prev), None);
        assert_eq!(turned(2, 3, Turn::Next), None);
        assert_eq!(turned(1, 3, Turn::Next), Some(2));
        assert_eq!(turned(1, 3, Turn::Prev), Some(0));
        assert_eq!(progress(0, 10), 0.0);
        assert_eq!(progress(9, 10), 1.0);
        assert_eq!(progress(0, 1), 1.0);
        assert_eq!(progress(0, 0), 0.0);
    }
}
