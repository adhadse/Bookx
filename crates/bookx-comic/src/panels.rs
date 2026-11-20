//! Finding the panels of a comic page.
//!
//! The page is reduced to a small grey image. The gutter colour is the colour most of the border
//! (or of the whole page) has; a line of the image is a gutter when nearly all its pixels have
//! that colour. The page is cut along gutters, first into rows, then each row into columns, and so
//! on (an XY cut), until a part has no gutter inside. What is left are the panels, in reading
//! order. Pages with no gutters (full-bleed art) give no panels and the reader falls back to the
//! page.

/// A rectangle in fractions of the page (0 to 1).
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
    pub fn area(&self) -> f64 {
        self.w * self.h
    }
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    /// Squared distance of a point to the rectangle (0 inside).
    pub fn dist2(&self, px: f64, py: f64) -> f64 {
        let dx = (self.x - px).max(0.0).max(px - (self.x + self.w));
        let dy = (self.y - py).max(0.0).max(py - (self.y + self.h));
        dx * dx + dy * dy
    }
    /// Scaled to a page of the given size.
    pub fn scaled(&self, w: f64, h: f64) -> [f64; 4] {
        [self.x * w, self.y * h, self.w * w, self.h * h]
    }
}

/// A grey image, one byte per pixel.
#[derive(Debug, Clone)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Gray {
    pub fn new(w: usize, h: usize, px: Vec<u8>) -> Gray {
        assert_eq!(px.len(), w * h);
        Gray { w, h, px }
    }

    /// From RGBA bytes.
    pub fn from_rgba(rgba: &[u8], w: usize, h: usize) -> Gray {
        let px = rgba
            .chunks_exact(4)
            .take(w * h)
            .map(|p| ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8)
            .collect();
        Gray { w, h, px }
    }

    /// Reduced by whole-pixel box averaging so that the longest side is at most `edge`.
    pub fn reduced(&self, edge: usize) -> Gray {
        let longest = self.w.max(self.h);
        if longest <= edge || edge == 0 {
            return self.clone();
        }
        let f = longest.div_ceil(edge);
        let (nw, nh) = (self.w.div_ceil(f), self.h.div_ceil(f));
        let mut out = vec![0u8; nw * nh];
        for ny in 0..nh {
            for nx in 0..nw {
                let (mut sum, mut n) = (0u32, 0u32);
                for y in ny * f..((ny + 1) * f).min(self.h) {
                    let row = &self.px[y * self.w..(y + 1) * self.w];
                    for v in &row[nx * f..((nx + 1) * f).min(self.w)] {
                        sum += *v as u32;
                        n += 1;
                    }
                }
                out[ny * nw + nx] = (sum / n.max(1)) as u8;
            }
        }
        Gray {
            w: nw,
            h: nh,
            px: out,
        }
    }
}

/// Tuning of the detection.
#[derive(Debug, Clone, Copy)]
pub struct Opts {
    /// The page is reduced to this many pixels on its longest side first.
    pub edge: usize,
    /// How far from the gutter colour a pixel may be and still count as gutter.
    pub tolerance: u8,
    /// Share of a line that may be non-gutter (specks, scan noise) and still be a gutter line.
    pub noise: f64,
    /// The narrowest gutter, as a share of the page side.
    pub min_gutter: f64,
    /// Panels narrower or lower than this share of the page are dropped.
    pub min_panel: f64,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            edge: 400,
            tolerance: 28,
            noise: 0.004,
            min_gutter: 0.009,
            min_panel: 0.06,
        }
    }
}

struct Ctx<'a> {
    g: &'a Gray,
    bg: u8,
    o: Opts,
    gutter_x: usize,
    gutter_y: usize,
    rtl: bool,
}

#[derive(Clone, Copy)]
struct Area {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Ctx<'_> {
    fn is_bg(&self, v: u8) -> bool {
        v.abs_diff(self.bg) <= self.o.tolerance
    }

    /// Whether the row/column `i` of the area (rows when `rows`) is all gutter.
    fn empty_line(&self, a: Area, rows: bool, i: usize) -> bool {
        let (len, bad) = if rows {
            let row = &self.g.px[i * self.g.w + a.x0..i * self.g.w + a.x1];
            (row.len(), row.iter().filter(|v| !self.is_bg(**v)).count())
        } else {
            let bad = (a.y0..a.y1)
                .filter(|y| !self.is_bg(self.g.px[y * self.g.w + i]))
                .count();
            (a.y1 - a.y0, bad)
        };
        (bad as f64) <= self.o.noise * len as f64
    }

    /// The area without the empty lines at its four sides.
    fn trim(&self, mut a: Area) -> Option<Area> {
        while a.y0 < a.y1 && self.empty_line(a, true, a.y0) {
            a.y0 += 1;
        }
        while a.y1 > a.y0 && self.empty_line(a, true, a.y1 - 1) {
            a.y1 -= 1;
        }
        while a.x0 < a.x1 && self.empty_line(a, false, a.x0) {
            a.x0 += 1;
        }
        while a.x1 > a.x0 && self.empty_line(a, false, a.x1 - 1) {
            a.x1 -= 1;
        }
        (a.x1 > a.x0 && a.y1 > a.y0).then_some(a)
    }

    /// The parts of the (trimmed) area separated by gutters along one axis.
    fn bands(&self, a: Area, rows: bool) -> Vec<Area> {
        let (lo, hi, gmin) = if rows {
            (a.y0, a.y1, self.gutter_y)
        } else {
            (a.x0, a.x1, self.gutter_x)
        };
        let mut out = Vec::new();
        let mut start = lo;
        let mut i = lo;
        while i < hi {
            if self.empty_line(a, rows, i) {
                let mut j = i;
                while j < hi && self.empty_line(a, rows, j) {
                    j += 1;
                }
                if j - i >= gmin && i > start {
                    out.push((start, i));
                    start = j;
                }
                i = j;
            } else {
                i += 1;
            }
        }
        out.push((start, hi));
        out.into_iter()
            .map(|(s, e)| {
                if rows {
                    Area { y0: s, y1: e, ..a }
                } else {
                    Area { x0: s, x1: e, ..a }
                }
            })
            .collect()
    }

    fn cut(&self, a: Area, depth: usize, out: &mut Vec<Area>) {
        let Some(a) = self.trim(a) else { return };
        if depth < 8 {
            for rows in [true, false] {
                let mut parts = self.bands(a, rows);
                if parts.len() > 1 {
                    if !rows && self.rtl {
                        parts.reverse();
                    }
                    for p in parts {
                        self.cut(p, depth + 1, out);
                    }
                    return;
                }
            }
        }
        out.push(a);
    }
}

fn mode_of_border(g: &Gray) -> u8 {
    let ring = (g.w.min(g.h) / 60).max(1);
    let mut hist = [0u32; 32];
    for y in 0..g.h {
        for x in 0..g.w {
            if x < ring || y < ring || x >= g.w - ring || y >= g.h - ring {
                hist[(g.px[y * g.w + x] >> 3) as usize] += 1;
            }
        }
    }
    let best = hist
        .iter()
        .enumerate()
        .max_by_key(|(_, n)| **n)
        .map(|(i, _)| i)
        .unwrap_or(31);
    (best * 8 + 4).min(255) as u8
}

fn run(g: &Gray, bg: u8, o: Opts, rtl: bool) -> Vec<Rect> {
    let ctx = Ctx {
        g,
        bg,
        o,
        gutter_x: ((g.w as f64 * o.min_gutter).round() as usize).max(2),
        gutter_y: ((g.h as f64 * o.min_gutter).round() as usize).max(2),
        rtl,
    };
    let mut areas = Vec::new();
    ctx.cut(
        Area {
            x0: 0,
            y0: 0,
            x1: g.w,
            y1: g.h,
        },
        0,
        &mut areas,
    );
    areas
        .into_iter()
        .filter(|a| {
            (a.x1 - a.x0) as f64 >= o.min_panel * g.w as f64
                && (a.y1 - a.y0) as f64 >= o.min_panel * g.h as f64
        })
        .map(|a| {
            Rect::new(
                a.x0 as f64 / g.w as f64,
                a.y0 as f64 / g.h as f64,
                (a.x1 - a.x0) as f64 / g.w as f64,
                (a.y1 - a.y0) as f64 / g.h as f64,
            )
        })
        .collect()
}

/// The panels of a page in reading order (`rtl`: right to left within a row). Empty when the page
/// has no gutters to cut along.
pub fn detect(page: &Gray, rtl: bool, o: Opts) -> Vec<Rect> {
    if page.w < 16 || page.h < 16 {
        return Vec::new();
    }
    let g = page.reduced(o.edge);
    let mut candidates = vec![mode_of_border(&g), 255, 0];
    candidates.dedup();
    let mut best: Vec<Rect> = Vec::new();
    for bg in candidates {
        let found = run(&g, bg, o, rtl);
        if found.len() > best.len() {
            best = found;
        }
    }
    if best.len() == 1 && best[0].area() > 0.95 {
        return Vec::new();
    }
    best
}

/// The panel under a point of the page (fractions): the one that contains it, else the nearest.
pub fn panel_at(panels: &[Rect], x: f64, y: f64) -> Option<usize> {
    panels.iter().position(|p| p.contains(x, y)).or_else(|| {
        panels
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.dist2(x, y).total_cmp(&b.1.dist2(x, y)))
            .map(|(i, _)| i)
    })
}

/// Where guided view goes next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Another panel of this page.
    Panel(usize),
    /// The first (or, going back, the last) panel of the next (previous) page.
    NextPage,
    PrevPage,
}

/// The step after panel `cur` (`None`: the whole page is shown) in a direction.
pub fn step(count: usize, cur: Option<usize>, forward: bool) -> Step {
    match (cur, forward) {
        (None, true) if count > 0 => Step::Panel(0),
        (None, false) if count > 0 => Step::PrevPage,
        (None, true) => Step::NextPage,
        (None, false) => Step::PrevPage,
        (Some(i), true) if i + 1 < count => Step::Panel(i + 1),
        (Some(_), true) => Step::NextPage,
        (Some(0), false) => Step::PrevPage,
        (Some(i), false) => Step::Panel(i - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::{page_gray, Layout};

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.03
    }

    fn rect_near(r: &Rect, e: [f64; 4]) -> bool {
        near(r.x, e[0]) && near(r.y, e[1]) && near(r.w, e[2]) && near(r.h, e[3])
    }

    #[test]
    fn a_grid_of_four_is_found_in_reading_order() {
        let g = page_gray(600, 900, Layout::Grid(2, 2), 255);
        let p = detect(&g, false, Opts::default());
        assert_eq!(p.len(), 4, "{p:?}");
        assert!(p[0].x < p[1].x && p[0].y < p[2].y);
        assert!(near(p[0].y, p[1].y), "the first two share a row");
        assert!(p[2].y > p[1].y);
        assert!(rect_near(&p[0], [0.04, 0.03, 0.44, 0.45]), "{:?}", p[0]);
    }

    #[test]
    fn right_to_left_reads_each_row_from_the_right() {
        let g = page_gray(600, 900, Layout::Grid(2, 2), 255);
        let p = detect(&g, true, Opts::default());
        assert_eq!(p.len(), 4);
        assert!(p[0].x > p[1].x, "manga: right panel first");
        assert!(p[2].x > p[3].x);
        assert!(p[2].y > p[0].y, "rows still go down");
    }

    #[test]
    fn rows_of_different_shape() {
        // wide panel on top, two below, then a tall one next to two stacked ones
        let g = page_gray(600, 900, Layout::Mixed, 255);
        let p = detect(&g, false, Opts::default());
        assert_eq!(p.len(), 6, "{p:?}");
        assert!(p[0].w > 0.8, "the first panel is the wide one");
        assert!(p[1].x < p[2].x && near(p[1].y, p[2].y));
    }

    #[test]
    fn dark_gutters_work_too() {
        let g = page_gray(600, 900, Layout::Grid(2, 3), 0);
        let p = detect(&g, false, Opts::default());
        assert_eq!(p.len(), 6, "{p:?}");
    }

    #[test]
    fn a_full_bleed_page_has_no_panels() {
        let g = page_gray(600, 900, Layout::Splash, 255);
        assert!(detect(&g, false, Opts::default()).is_empty());
        let g = page_gray(600, 900, Layout::Noise, 255);
        assert!(detect(&g, false, Opts::default()).is_empty());
    }

    #[test]
    fn a_single_framed_picture_is_one_panel() {
        let g = page_gray(600, 900, Layout::Grid(1, 1), 255);
        let p = detect(&g, false, Opts::default());
        assert_eq!(p.len(), 1);
        assert!(p[0].area() < 0.95 && p[0].area() > 0.6);
    }

    #[test]
    fn specks_in_a_gutter_do_not_hide_it() {
        let mut g = page_gray(600, 900, Layout::Grid(2, 2), 255);
        // a few dust pixels across the horizontal gutter
        let y = (0.5 * 900.0) as usize;
        for x in (50..550).step_by(97) {
            g.px[y * 600 + x] = 0;
        }
        assert_eq!(detect(&g, false, Opts::default()).len(), 4);
    }

    #[test]
    fn tiny_and_blank_images_are_safe() {
        assert!(detect(&Gray::new(4, 4, vec![0; 16]), false, Opts::default()).is_empty());
        assert!(detect(&Gray::new(64, 64, vec![255; 4096]), false, Opts::default()).is_empty());
    }

    #[test]
    fn large_pages_are_reduced_first() {
        let g = page_gray(2000, 3000, Layout::Grid(2, 2), 255);
        assert_eq!(detect(&g, false, Opts::default()).len(), 4);
    }

    #[test]
    fn panel_at_picks_containing_then_nearest() {
        let p = [Rect::new(0.0, 0.0, 0.4, 0.4), Rect::new(0.6, 0.0, 0.4, 0.4)];
        assert_eq!(panel_at(&p, 0.1, 0.1), Some(0));
        assert_eq!(panel_at(&p, 0.9, 0.2), Some(1));
        assert_eq!(panel_at(&p, 0.55, 0.2), Some(1), "in the gutter: nearest");
        assert_eq!(panel_at(&p, 0.45, 0.2), Some(0));
        assert_eq!(panel_at(&[], 0.5, 0.5), None);
    }

    #[test]
    fn guided_steps() {
        assert_eq!(step(3, None, true), Step::Panel(0));
        assert_eq!(step(3, Some(0), true), Step::Panel(1));
        assert_eq!(step(3, Some(2), true), Step::NextPage);
        assert_eq!(step(3, Some(1), false), Step::Panel(0));
        assert_eq!(step(3, Some(0), false), Step::PrevPage);
        assert_eq!(step(0, None, true), Step::NextPage);
        assert_eq!(step(0, None, false), Step::PrevPage);
    }
}
