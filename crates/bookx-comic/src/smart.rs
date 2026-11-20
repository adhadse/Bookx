//! Smart zoom and guided view on top of the panels: what a double click zooms to, and the list of
//! stops of a screen for the next/previous panel keys.

use crate::layout::Placed;
use crate::panels::{panel_at, Rect};

/// One stop of guided view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    pub page: usize,
    /// The panel on that page (`None`: the whole page, it has no panels).
    pub panel: Option<usize>,
    /// In content units of the screen.
    pub rect: [f64; 4],
}

fn in_content(p: &Placed, r: &Rect) -> [f64; 4] {
    [p.x + r.x * p.w, p.y + r.y * p.h, r.w * p.w, r.h * p.h]
}

/// The stops of a screen in reading order (the pages of `placed` in the order given: first page
/// first). A page without panels is one stop.
pub fn stops(placed: &[Placed], panels: &dyn Fn(usize) -> Vec<Rect>) -> Vec<Stop> {
    let mut out = Vec::new();
    for p in placed {
        let list = panels(p.page);
        if list.is_empty() {
            out.push(Stop {
                page: p.page,
                panel: None,
                rect: [p.x, p.y, p.w, p.h],
            });
        } else {
            for (i, r) in list.iter().enumerate() {
                out.push(Stop {
                    page: p.page,
                    panel: Some(i),
                    rect: in_content(p, r),
                });
            }
        }
    }
    out
}

/// What a double click at a content point zooms to: the panel under it, else the nearest panel of
/// the page, else (no panels found) a region of the page around the point.
pub fn zoom_target(
    placed: &[Placed],
    panels: &dyn Fn(usize) -> Vec<Rect>,
    cx: f64,
    cy: f64,
) -> Option<Stop> {
    let p = placed
        .iter()
        .find(|p| cx >= p.x && cx <= p.x + p.w && cy >= p.y && cy <= p.y + p.h)
        .or_else(|| {
            placed.iter().min_by(|a, b| {
                let d = |p: &&Placed| {
                    let dx = (p.x - cx).max(0.0).max(cx - (p.x + p.w));
                    let dy = (p.y - cy).max(0.0).max(cy - (p.y + p.h));
                    dx * dx + dy * dy
                };
                d(a).total_cmp(&d(b))
            })
        })?;
    let list = panels(p.page);
    let (fx, fy) = ((cx - p.x) / p.w.max(1.0), (cy - p.y) / p.h.max(1.0));
    if let Some(i) = panel_at(&list, fx, fy) {
        return Some(Stop {
            page: p.page,
            panel: Some(i),
            rect: in_content(p, &list[i]),
        });
    }
    // no panels: a region of the page around the point, 55% of its width and 40% of its height
    let (w, h) = (p.w * 0.55, p.h * 0.4);
    let x = (cx - w / 2.0).clamp(p.x, p.x + p.w - w);
    let y = (cy - h / 2.0).clamp(p.y, p.y + p.h - h);
    Some(Stop {
        page: p.page,
        panel: None,
        rect: [x, y, w, h],
    })
}

/// The index of the stop for a page and panel.
pub fn index_of(stops: &[Stop], page: usize, panel: Option<usize>) -> Option<usize> {
    stops
        .iter()
        .position(|s| s.page == page && s.panel == panel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::place_spread;

    fn two_pages(rtl: bool) -> Vec<Placed> {
        place_spread(&[(400, 600), (400, 600)], &[0, 1], rtl).0
    }

    fn panels(page: usize) -> Vec<Rect> {
        match page {
            0 => vec![
                Rect::new(0.05, 0.05, 0.9, 0.4),
                Rect::new(0.05, 0.55, 0.9, 0.4),
            ],
            _ => Vec::new(),
        }
    }

    #[test]
    fn stops_follow_the_pages_in_reading_order() {
        let s = stops(&two_pages(false), &panels);
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].page, s[0].panel), (0, Some(0)));
        assert_eq!((s[1].page, s[1].panel), (0, Some(1)));
        assert_eq!(
            (s[2].page, s[2].panel),
            (1, None),
            "no panels: the whole page"
        );
        assert_eq!(s[2].rect, [400.0, 0.0, 400.0, 600.0]);
        // rtl: page 0 is on the right
        let s = stops(&two_pages(true), &panels);
        assert!(s[0].rect[0] > 400.0 && s[2].rect[0] < 400.0);
        assert_eq!(index_of(&s, 0, Some(1)), Some(1));
        assert_eq!(index_of(&s, 1, None), Some(2));
        assert_eq!(index_of(&s, 5, None), None);
    }

    #[test]
    fn double_click_zooms_to_the_panel_under_the_pointer() {
        let pl = two_pages(false);
        // in the lower panel of page 0
        let t = zoom_target(&pl, &panels, 200.0, 450.0).unwrap();
        assert_eq!((t.page, t.panel), (0, Some(1)));
        assert_eq!(t.rect, [20.0, 330.0, 360.0, 240.0]);
        // in the gutter between the panels: the nearest one
        let t = zoom_target(&pl, &panels, 200.0, 290.0).unwrap();
        assert_eq!(
            t.panel,
            Some(0),
            "20 away from the first, 40 from the second"
        );
    }

    #[test]
    fn a_page_without_panels_zooms_to_a_region_around_the_pointer() {
        let pl = two_pages(false);
        let t = zoom_target(&pl, &panels, 600.0, 300.0).unwrap();
        assert_eq!((t.page, t.panel), (1, None));
        assert!((t.rect[2] - 220.0).abs() < 1e-9 && (t.rect[3] - 240.0).abs() < 1e-9);
        let c = (t.rect[0] + t.rect[2] / 2.0, t.rect[1] + t.rect[3] / 2.0);
        assert!((c.0 - 600.0).abs() < 1e-9 && (c.1 - 300.0).abs() < 1e-9);
        // near the corner the region stays on the page
        let t = zoom_target(&pl, &panels, 799.0, 599.0).unwrap();
        assert!(t.rect[0] + t.rect[2] <= 800.0 + 1e-9 && t.rect[1] + t.rect[3] <= 600.0 + 1e-9);
        assert!(t.rect[0] >= 400.0);
    }

    #[test]
    fn outside_the_pages_the_nearest_page_is_used_and_nothing_is_none() {
        let pl = two_pages(false);
        let t = zoom_target(&pl, &panels, -50.0, 100.0).unwrap();
        assert_eq!(t.page, 0);
        assert!(zoom_target(&[], &panels, 0.0, 0.0).is_none());
    }
}
