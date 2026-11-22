//! The camera: where the content (a page, a spread or a webtoon strip) is on screen.
//!
//! A point of the content `(cx, cy)` is at `(cx * s + x, cy * s + y)` on screen. Content is in
//! the units of [`crate::layout`] (picture pixels), the screen in logical pixels.

use crate::settings::Fit;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cam {
    pub s: f64,
    pub x: f64,
    pub y: f64,
}

/// Size of the content and of the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub cw: f64,
    pub ch: f64,
    pub vw: f64,
    pub vh: f64,
}

/// The most the page is enlarged beyond its picture pixels.
pub const MAX_PIXEL_ZOOM: f64 = 6.0;

impl Cam {
    pub fn to_view(&self, cx: f64, cy: f64) -> (f64, f64) {
        (cx * self.s + self.x, cy * self.s + self.y)
    }
    pub fn to_content(&self, vx: f64, vy: f64) -> (f64, f64) {
        ((vx - self.x) / self.s, (vy - self.y) / self.s)
    }
}

/// The scale that fits the content to the window.
pub fn fit_scale(f: Frame, fit: Fit) -> f64 {
    let (cw, ch) = (f.cw.max(1.0), f.ch.max(1.0));
    let s = match fit {
        Fit::Width => f.vw / cw,
        Fit::Height => f.vh / ch,
        Fit::Page => (f.vw / cw).min(f.vh / ch),
        Fit::Original => 1.0,
    };
    s.max(1e-4)
}

/// The scales the user may zoom between: from half of the whole page to many times the picture.
pub fn zoom_limits(f: Frame) -> (f64, f64) {
    let page = fit_scale(f, Fit::Page);
    let min = page * 0.5;
    let max = MAX_PIXEL_ZOOM.max(page * 2.0);
    (min, max)
}

/// Keeps the content on screen: centred on an axis where it is smaller than the window, and with
/// no empty edge where it is larger.
pub fn clamp(c: Cam, f: Frame) -> Cam {
    let fix = |off: f64, content: f64, view: f64| {
        let size = content * c.s;
        if size <= view {
            (view - size) / 2.0
        } else {
            off.clamp(view - size, 0.0)
        }
    };
    Cam {
        s: c.s,
        x: fix(c.x, f.cw, f.vw),
        y: fix(c.y, f.ch, f.vh),
    }
}

/// The camera that shows a page at a fit, at the start of the page (top, and the side the
/// reading starts from: left, or right when `rtl`).
pub fn place(f: Frame, fit: Fit, rtl: bool) -> Cam {
    let s = fit_scale(f, fit);
    let x = if rtl { f.vw - f.cw * s } else { 0.0 };
    clamp(Cam { s, x, y: 0.0 }, f)
}

/// Zooms by a factor keeping the content point under `(ax, ay)` where it is.
pub fn zoom_at(c: Cam, factor: f64, ax: f64, ay: f64, f: Frame) -> Cam {
    let (lo, hi) = zoom_limits(f);
    let s = (c.s * factor).clamp(lo, hi.max(lo));
    let (cx, cy) = c.to_content(ax, ay);
    clamp(
        Cam {
            s,
            x: ax - cx * s,
            y: ay - cy * s,
        },
        f,
    )
}

/// Moves the picture by `(dx, dy)` screen pixels (positive moves the content right/down).
pub fn pan(c: Cam, dx: f64, dy: f64, f: Frame) -> Cam {
    clamp(
        Cam {
            x: c.x + dx,
            y: c.y + dy,
            ..c
        },
        f,
    )
}

/// How far the picture can still be moved down (`down`) or up before its edge: used to turn the
/// page when the end is reached. Negative `dy` scrolls towards the bottom of the page.
pub fn scroll_room(c: Cam, f: Frame, towards_end: bool) -> f64 {
    let size = f.ch * c.s;
    if size <= f.vh {
        return 0.0;
    }
    if towards_end {
        (c.y - (f.vh - size)).max(0.0)
    } else {
        (-c.y).max(0.0)
    }
}

/// The camera that shows a rectangle of the content (`[x, y, w, h]`) as large as the window
/// allows, with a margin (share of the window), never beyond the zoom limits.
pub fn focus(rect: [f64; 4], margin: f64, f: Frame) -> Cam {
    let (lo, hi) = zoom_limits(f);
    let aw = f.vw * (1.0 - 2.0 * margin);
    let ah = f.vh * (1.0 - 2.0 * margin);
    let s = (aw / rect[2].max(1.0))
        .min(ah / rect[3].max(1.0))
        .clamp(lo, hi.max(lo));
    let (mx, my) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    clamp(
        Cam {
            s,
            x: f.vw / 2.0 - mx * s,
            y: f.vh / 2.0 - my * s,
        },
        f,
    )
}

/// The content point in the middle of the window.
pub fn center_of(c: Cam, f: Frame) -> (f64, f64) {
    c.to_content(f.vw / 2.0, f.vh / 2.0)
}

/// The camera at time `t` (0..1) of a move: the centre moves in a line and the scale changes
/// evenly in its logarithm, so zooming in and out feel the same.
pub fn tween(a: Cam, b: Cam, t: f64, f: Frame) -> Cam {
    let t = t.clamp(0.0, 1.0);
    if t >= 1.0 {
        return b;
    }
    let (ax, ay) = center_of(a, f);
    let (bx, by) = center_of(b, f);
    let s = (a.s.ln() + (b.s.ln() - a.s.ln()) * t).exp();
    let (mx, my) = (ax + (bx - ax) * t, ay + (by - ay) * t);
    Cam {
        s,
        x: f.vw / 2.0 - mx * s,
        y: f.vh / 2.0 - my * s,
    }
}

pub fn ease_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Length of a move in milliseconds; 0 when animations are off (reduced motion, e-ink).
pub fn duration_ms(animate: bool) -> u32 {
    if animate { 220 } else { 0 }
}

/// Whether a camera shows the whole content (nothing is zoomed in beyond the page fit).
pub fn is_fitted(c: Cam, f: Frame, fit: Fit) -> bool {
    (c.s - fit_scale(f, fit)).abs() < fit_scale(f, fit) * 0.02
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: Frame = Frame {
        cw: 800.0,
        ch: 600.0,
        vw: 400.0,
        vh: 400.0,
    };

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn fit_scales() {
        assert!(close(fit_scale(F, Fit::Width), 0.5));
        assert!(close(fit_scale(F, Fit::Height), 400.0 / 600.0));
        assert!(close(fit_scale(F, Fit::Page), 0.5));
        assert!(close(fit_scale(F, Fit::Original), 1.0));
    }

    #[test]
    fn fitted_content_is_centred_and_overflow_stays_covered() {
        let c = place(F, Fit::Page, false);
        assert!(close(c.x, 0.0) && close(c.y, 50.0), "{c:?}");
        // fit height: the page is wider than the window: starts at the left (or right in rtl)
        let c = place(F, Fit::Height, false);
        assert!(close(c.x, 0.0) && close(c.y, 0.0));
        let c = place(F, Fit::Height, true);
        assert!(close(c.x, 400.0 - 800.0 * c.s), "{c:?}");
        // never an empty edge
        let c = pan(c, 9999.0, 9999.0, F);
        assert!(c.x <= 0.0 && c.y <= 0.0);
        let c = pan(c, -99999.0, -99999.0, F);
        assert!(close(c.x, 400.0 - 800.0 * c.s));
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let c = place(F, Fit::Page, false);
        let before = c.to_content(120.0, 200.0);
        let z = zoom_at(c, 3.0, 120.0, 200.0, F);
        assert!(z.s > c.s);
        let after = z.to_content(120.0, 200.0);
        assert!(
            close(before.0, after.0) && close(before.1, after.1),
            "{before:?} {after:?}"
        );
    }

    #[test]
    fn zoom_is_limited() {
        let c = place(F, Fit::Page, false);
        let (lo, hi) = zoom_limits(F);
        assert!(close(zoom_at(c, 1e-6, 0.0, 0.0, F).s, lo));
        assert!(close(zoom_at(c, 1e6, 0.0, 0.0, F).s, hi));
        assert!(lo < c.s && hi > c.s);
    }

    #[test]
    fn focus_fits_the_rectangle_and_centres_it() {
        let r = [100.0, 100.0, 200.0, 150.0];
        let c = focus(r, 0.04, F);
        // the rectangle is inside the window with its margin
        let (x0, y0) = c.to_view(r[0], r[1]);
        let (x1, y1) = c.to_view(r[0] + r[2], r[1] + r[3]);
        assert!(
            x0 >= 0.0 && y0 >= 0.0 && x1 <= 400.0 && y1 <= 400.0,
            "{c:?}"
        );
        assert!(close(c.s, (400.0 * 0.92) / 200.0));
        let mid = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        assert!(close(mid.0, 200.0) && close(mid.1, 200.0));
    }

    #[test]
    fn focus_near_the_edge_is_clamped_to_the_page() {
        let r = [0.0, 0.0, 200.0, 150.0];
        let c = focus(r, 0.04, F);
        assert!(
            c.x <= 0.0 && c.y <= 0.0,
            "no empty space left of or above the page: {c:?}"
        );
        let (x0, y0) = c.to_view(0.0, 0.0);
        assert!(x0 <= 0.0 && y0 <= 0.0);
    }

    #[test]
    fn tween_ends_and_middle() {
        let a = place(F, Fit::Page, false);
        let b = focus([100.0, 100.0, 200.0, 150.0], 0.04, F);
        assert_eq!(tween(a, b, 1.0, F), b);
        let s0 = tween(a, b, 0.0, F);
        assert!(close(s0.s, a.s) && close(s0.x, a.x) && close(s0.y, a.y));
        let m = tween(a, b, 0.5, F);
        assert!(m.s > a.s && m.s < b.s);
        assert!(close(m.s, (a.s * b.s).sqrt()), "geometric middle");
    }

    #[test]
    fn easing_is_monotonic_and_bounded() {
        let mut last = 0.0;
        for i in 0..=20 {
            let e = ease_out(i as f64 / 20.0);
            assert!(e >= last && (0.0..=1.0).contains(&e));
            last = e;
        }
        assert_eq!(ease_out(2.0), 1.0);
        assert_eq!(ease_out(-1.0), 0.0);
        assert_eq!(duration_ms(false), 0);
        assert!(duration_ms(true) > 0);
    }

    #[test]
    fn scroll_room_tells_when_the_page_end_is_reached() {
        let f = Frame {
            cw: 400.0,
            ch: 1600.0,
            vw: 400.0,
            vh: 400.0,
        };
        let top = place(f, Fit::Width, false);
        assert!(close(scroll_room(top, f, false), 0.0));
        assert!(close(scroll_room(top, f, true), 1200.0));
        let bottom = pan(top, 0.0, -5000.0, f);
        assert!(close(scroll_room(bottom, f, true), 0.0));
        assert!(close(scroll_room(bottom, f, false), 1200.0));
        // a page that fits has no room either way
        let c = place(F, Fit::Page, false);
        assert_eq!(scroll_room(c, F, true), 0.0);
    }

    #[test]
    fn fitted_detection() {
        let c = place(F, Fit::Page, false);
        assert!(is_fitted(c, F, Fit::Page));
        assert!(!is_fitted(zoom_at(c, 2.0, 100.0, 100.0, F), F, Fit::Page));
    }

    #[test]
    fn degenerate_frames_do_not_blow_up() {
        let f = Frame {
            cw: 0.0,
            ch: 0.0,
            vw: 0.0,
            vh: 0.0,
        };
        let c = place(f, Fit::Page, false);
        assert!(c.s.is_finite() && c.x.is_finite() && c.y.is_finite());
        let z = zoom_at(c, 2.0, 1.0, 1.0, f);
        assert!(z.s.is_finite());
    }
}
