//! The PDF page view without GTK: where pages sit when laid out, zoom steps, the way PDF
//! annotations are stored in the library, search hit snippets and page jumps. The window is
//! `ui/reader_pdf.rs`; the engine is the `bookx-pdf` crate.

use std::ops::Range;

use bookx_pdf::text::Rect;

use crate::core::library::Highlight;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Pixels per point at 100 % (96 dpi screen, 72 points per inch).
pub const PX_PER_PT: f32 = 96.0 / 72.0;
/// Zoom steps of Ctrl+plus / Ctrl+minus, in percent.
pub const ZOOM_STEPS: [u32; 12] = [25, 33, 50, 67, 80, 100, 125, 150, 200, 300, 400, 600];

/// How the PDF page view looks; kept in `pdf.json` next to the settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PdfPrefs {
    /// `width`, `page` or a percentage.
    pub zoom: String,
    /// One page at a time instead of continuous scrolling.
    pub single: bool,
    /// Cut the white margins off the pages.
    pub crop: bool,
    /// Light text on dark paper (night reading, e-ink with a dark theme).
    pub invert: bool,
    /// Open PDFs as pages (`true`) or as reflowed text (`false`).
    pub pages: bool,
}

impl Default for PdfPrefs {
    fn default() -> Self {
        PdfPrefs {
            zoom: "width".into(),
            single: false,
            crop: false,
            invert: false,
            pages: true,
        }
    }
}

fn prefs_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("pdf.json"))
}

impl PdfPrefs {
    pub fn load() -> PdfPrefs {
        prefs_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = prefs_path() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(t) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, t);
            }
        }
    }

    pub fn zoom_mode(&self) -> ZoomMode {
        ZoomMode::from_name(&self.zoom).unwrap_or(ZoomMode::FitWidth)
    }

    pub fn view_mode(&self) -> ViewMode {
        if self.single {
            ViewMode::Single
        } else {
            ViewMode::Continuous
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomMode {
    FitWidth,
    FitPage,
    /// Percent of the 96 dpi size.
    Percent(u32),
}

impl ZoomMode {
    /// The name stored in the settings and used by the dev hooks.
    pub fn name(self) -> String {
        match self {
            ZoomMode::FitWidth => "width".into(),
            ZoomMode::FitPage => "page".into(),
            ZoomMode::Percent(p) => p.to_string(),
        }
    }

    pub fn from_name(s: &str) -> Option<ZoomMode> {
        match s {
            "width" => Some(ZoomMode::FitWidth),
            "page" => Some(ZoomMode::FitPage),
            n => n
                .trim_end_matches('%')
                .parse::<u32>()
                .ok()
                .map(|p| ZoomMode::Percent(p.clamp(10, 800))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Continuous,
    Single,
}

/// A page placed in the document: pixels, origin top left of the whole document.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub page: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Device pixels per point of the visible part.
    pub scale: f32,
}

#[derive(Debug, Clone)]
pub struct Layout {
    /// Page sizes in points.
    pub sizes: Vec<(f32, f32)>,
    /// Margins to cut off, as fractions `[x0, y0, x1, y1]` of the page that stay.
    pub crop: Option<[f32; 4]>,
    pub mode: ViewMode,
    pub zoom: ZoomMode,
    /// The window of the view in pixels.
    pub viewport: (f32, f32),
    pub gap: f32,
    pub margin: f32,
}

impl Layout {
    pub fn new(sizes: Vec<(f32, f32)>) -> Layout {
        Layout {
            sizes,
            crop: None,
            mode: ViewMode::Continuous,
            zoom: ZoomMode::FitWidth,
            viewport: (800.0, 600.0),
            gap: 12.0,
            margin: 12.0,
        }
    }

    /// Size of the shown part of page `i` in points.
    pub fn shown_pt(&self, i: usize) -> (f32, f32) {
        let (w, h) = self.sizes.get(i).copied().unwrap_or((612.0, 792.0));
        match self.crop {
            Some(c) => ((w * (c[2] - c[0])).max(1.0), (h * (c[3] - c[1])).max(1.0)),
            None => (w, h),
        }
    }

    /// Pixels per point for page `i`.
    pub fn scale_of(&self, i: usize) -> f32 {
        let (w, h) = self.shown_pt(i);
        let avail_w = (self.viewport.0 - 2.0 * self.margin).max(50.0);
        let avail_h = (self.viewport.1 - 2.0 * self.margin).max(50.0);
        match self.zoom {
            ZoomMode::FitWidth => avail_w / w,
            ZoomMode::FitPage => (avail_w / w).min(avail_h / h),
            ZoomMode::Percent(p) => PX_PER_PT * p as f32 / 100.0,
        }
    }

    fn place(&self, i: usize, x: f32, y: f32, doc_w: f32) -> Place {
        let (w, h) = self.shown_pt(i);
        let s = self.scale_of(i);
        let (pw, ph) = (w * s, h * s);
        let x = x + ((doc_w - pw) / 2.0).max(0.0);
        Place {
            page: i,
            x,
            y,
            w: pw,
            h: ph,
            scale: s,
        }
    }

    /// Where the pages are. In single page mode only page `current`.
    pub fn places(&self, current: usize) -> Vec<Place> {
        let n = self.sizes.len();
        if n == 0 {
            return Vec::new();
        }
        let range: Vec<usize> = match self.mode {
            ViewMode::Continuous => (0..n).collect(),
            ViewMode::Single => vec![current.min(n - 1)],
        };
        let doc_w = range
            .iter()
            .map(|i| {
                let (w, _) = self.shown_pt(*i);
                w * self.scale_of(*i) + 2.0 * self.margin
            })
            .fold(self.viewport.0, f32::max);
        let mut y = self.margin;
        let mut out = Vec::with_capacity(range.len());
        for i in range {
            let p = self.place(i, 0.0, y, doc_w);
            y += p.h + self.gap;
            out.push(p);
        }
        out
    }

    /// Size of the whole document in pixels (what the scroll bars span).
    pub fn content_size(places: &[Place], margin: f32, viewport: (f32, f32)) -> (f32, f32) {
        let w = places
            .iter()
            .map(|p| p.x + p.w + margin)
            .fold(viewport.0, f32::max);
        let h = places
            .last()
            .map(|p| p.y + p.h + margin)
            .unwrap_or(viewport.1);
        (w, h.max(viewport.1))
    }
}

/// The indexes (into `places`) of the pages that intersect `[top, top + height)`.
pub fn visible(places: &[Place], top: f32, height: f32) -> Range<usize> {
    if places.is_empty() {
        return 0..0;
    }
    let bottom = top + height;
    let first = places.partition_point(|p| p.y + p.h < top);
    let last = places.partition_point(|p| p.y <= bottom);
    first.min(places.len())..last.max(first.min(places.len())).min(places.len())
}

/// The index into `places` of the page the reader is on: the one under the middle of the
/// view, or the one closest to it in a gap.
pub fn current_in(places: &[Place], top: f32, height: f32) -> usize {
    if places.is_empty() {
        return 0;
    }
    let mid = top + height * 0.35;
    let i = places.partition_point(|p| p.y + p.h < mid);
    i.min(places.len() - 1)
}

/// The next zoom in percent when stepping in or out from the effective `percent`.
pub fn step_zoom(percent: f32, zoom_in: bool) -> u32 {
    if zoom_in {
        ZOOM_STEPS
            .iter()
            .copied()
            .find(|s| *s as f32 > percent + 0.5)
            .unwrap_or(*ZOOM_STEPS.last().unwrap())
    } else {
        ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|s| (*s as f32) < percent - 0.5)
            .unwrap_or(ZOOM_STEPS[0])
    }
}

/// The scale a page is rendered at: the device pixels per point rounded up to a step of 1/8, so
/// that small zoom changes reuse a rendering.
pub fn render_scale(device_px_per_pt: f32) -> f32 {
    ((device_px_per_pt.clamp(0.05, 16.0) * 8.0).ceil() / 8.0).max(0.125)
}

/// Cache key of a rendering.
pub fn render_key(page: usize, scale: f32, invert: bool) -> (usize, u32, bool) {
    (page, (scale * 1000.0).round() as u32, invert)
}

/// Fraction of the document read when at `frac` (0..1) of page `page`; the last page's end is 1.
pub fn progress(page: usize, frac: f64, pages: usize) -> f64 {
    if pages == 0 {
        return 0.0;
    }
    ((page as f64 + frac.clamp(0.0, 1.0)) / pages as f64).clamp(0.0, 1.0)
}

/// The label of page `i` or a number, for the footer.
pub fn page_caption(labels: &[String], i: usize, pages: usize) -> String {
    let n = i + 1;
    match labels.get(i) {
        Some(l) if *l != n.to_string() && !l.is_empty() => format!("{l} ({n} of {pages})"),
        _ => format!("{n} of {pages}"),
    }
}

/// The page typed in the page box: a page label (`iv`, `A-3`), else a number; zero based.
pub fn resolve_page(input: &str, labels: &[String], pages: usize) -> Option<usize> {
    let t = input.trim();
    if t.is_empty() || pages == 0 {
        return None;
    }
    if let Some(i) = labels.iter().position(|l| l.eq_ignore_ascii_case(t)) {
        // a label that is a plain number wins only when it is not shadowed by another page
        return Some(i);
    }
    let n: usize = t.parse().ok()?;
    if n == 0 {
        return None;
    }
    Some((n - 1).min(pages - 1))
}

/// The marker a PDF annotation carries in [`Highlight::style`].
const PDF_TAG: &str = "pdf";

/// `style` for a PDF highlight: `pdf|background|x0,y0,x1,y1;...` (rectangles in points on the page).
pub fn pdf_style(style: &str, rects: &[Rect]) -> String {
    let r: Vec<String> = rects
        .iter()
        .map(|r| format!("{:.1},{:.1},{:.1},{:.1}", r[0], r[1], r[2], r[3]))
        .collect();
    format!("{PDF_TAG}|{style}|{}", r.join(";"))
}

/// Is this highlight an annotation of a PDF page (its `spine` is the page)?
pub fn is_pdf(h: &Highlight) -> bool {
    h.style == PDF_TAG || h.style.starts_with("pdf|")
}

/// Highlight style name and rectangles of a PDF highlight.
pub fn parse_pdf_style(s: &str) -> Option<(String, Vec<Rect>)> {
    let mut it = s.splitn(3, '|');
    if it.next()? != PDF_TAG {
        return None;
    }
    let style = it.next().unwrap_or("background").to_string();
    let rects = it
        .next()
        .unwrap_or("")
        .split(';')
        .filter_map(|r| {
            let v: Vec<f32> = r.split(',').filter_map(|x| x.parse().ok()).collect();
            (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
        })
        .collect();
    Some((style, rects))
}

/// Position of the page on screen `[x0, y0, x1, y1]` (points) as pixels inside `place`.
pub fn to_px(place: &Place, crop: Option<[f32; 4]>, size_pt: (f32, f32), r: Rect) -> [f32; 4] {
    let (ox, oy) = match crop {
        Some(c) => (c[0] * size_pt.0, c[1] * size_pt.1),
        None => (0.0, 0.0),
    };
    [
        place.x + (r[0] - ox) * place.scale,
        place.y + (r[1] - oy) * place.scale,
        place.x + (r[2] - ox) * place.scale,
        place.y + (r[3] - oy) * place.scale,
    ]
}

/// The point on the page in points for a position in document pixels.
pub fn to_pt(
    place: &Place,
    crop: Option<[f32; 4]>,
    size_pt: (f32, f32),
    x: f32,
    y: f32,
) -> (f32, f32) {
    let (ox, oy) = match crop {
        Some(c) => (c[0] * size_pt.0, c[1] * size_pt.1),
        None => (0.0, 0.0),
    };
    (
        (x - place.x) / place.scale + ox,
        (y - place.y) / place.scale + oy,
    )
}

/// The next/previous hit after `(page, pos)`, wrapping around: indexes into `hits`
/// (sorted by page then position).
pub fn step_hit(hits: &[(usize, usize)], page: usize, pos: usize, forward: bool) -> Option<usize> {
    if hits.is_empty() {
        return None;
    }
    if forward {
        hits.iter().position(|h| *h > (page, pos)).or(Some(0))
    } else {
        hits.iter()
            .rposition(|h| *h < (page, pos))
            .or(Some(hits.len() - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(n: usize) -> Layout {
        let mut l = Layout::new(vec![(612.0, 792.0); n]);
        l.viewport = (824.0, 600.0);
        l
    }

    #[test]
    fn fit_width_fills_the_window_minus_margins() {
        let l = layout(3);
        let s = l.scale_of(0);
        assert!((612.0 * s - 800.0).abs() < 0.01);
        let p = l.places(0);
        assert_eq!(p.len(), 3);
        assert!((p[0].w - 800.0).abs() < 0.01);
        assert_eq!(p[0].y, 12.0);
        assert!((p[1].y - (p[0].y + p[0].h + 12.0)).abs() < 0.01);
        assert_eq!(p[0].x, 12.0);
    }

    #[test]
    fn fit_page_fits_the_height_when_that_is_smaller() {
        let mut l = layout(1);
        l.zoom = ZoomMode::FitPage;
        let s = l.scale_of(0);
        assert!((792.0 * s - 576.0).abs() < 0.01, "{s}");
    }

    #[test]
    fn percent_zoom_uses_96_dpi() {
        let mut l = layout(1);
        l.zoom = ZoomMode::Percent(100);
        assert!((l.scale_of(0) - 96.0 / 72.0).abs() < 1e-6);
        l.zoom = ZoomMode::Percent(200);
        assert!((l.scale_of(0) - 96.0 / 36.0).abs() < 1e-6);
    }

    #[test]
    fn a_zoomed_page_wider_than_the_window_scrolls_horizontally() {
        let mut l = layout(2);
        l.zoom = ZoomMode::Percent(300);
        let p = l.places(0);
        let (w, h) = Layout::content_size(&p, l.margin, l.viewport);
        assert!(w > l.viewport.0);
        assert!(h > l.viewport.1);
        assert_eq!(p[0].x, l.margin);
    }

    #[test]
    fn narrow_pages_are_centred() {
        let mut l = Layout::new(vec![(612.0, 792.0), (306.0, 396.0)]);
        l.zoom = ZoomMode::Percent(100);
        l.viewport = (1200.0, 600.0);
        let p = l.places(0);
        assert!((p[1].x + p[1].w / 2.0 - 600.0).abs() < 0.01);
    }

    #[test]
    fn single_page_mode_places_only_the_current_page() {
        let mut l = layout(5);
        l.mode = ViewMode::Single;
        let p = l.places(3);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].page, 3);
        assert_eq!(l.places(99)[0].page, 4);
        assert!(Layout::new(vec![]).places(0).is_empty());
    }

    #[test]
    fn crop_shrinks_the_page_and_raises_the_scale() {
        let mut l = layout(1);
        let before = l.scale_of(0);
        l.crop = Some([0.1, 0.1, 0.9, 0.9]);
        assert!(l.scale_of(0) > before);
        let (w, h) = l.shown_pt(0);
        assert!((w - 612.0 * 0.8).abs() < 0.01 && (h - 792.0 * 0.8).abs() < 0.01);
    }

    #[test]
    fn visible_range_and_current_page() {
        let l = layout(10);
        let p = l.places(0);
        let h = p[0].h;
        assert_eq!(visible(&p, 0.0, 100.0), 0..1);
        let r = visible(&p, h * 2.0, 2.0 * h);
        assert!(r.start <= 1 && r.end >= 4 && r.end <= 5, "{r:?}");
        assert_eq!(current_in(&p, 0.0, 600.0), 0);
        assert_eq!(current_in(&p, p[4].y, 600.0), 4);
        assert_eq!(current_in(&p, 1.0e9, 600.0), 9);
        assert_eq!(visible(&[], 0.0, 10.0), 0..0);
    }

    #[test]
    fn a_five_hundred_page_layout_is_cheap() {
        let l = layout(500);
        let t = std::time::Instant::now();
        let p = l.places(0);
        assert_eq!(p.len(), 500);
        for k in 0..1000 {
            let _ = visible(&p, k as f32 * 300.0, 600.0);
        }
        assert!(t.elapsed().as_millis() < 500);
    }

    #[test]
    fn zoom_steps_go_through_the_list() {
        assert_eq!(step_zoom(100.0, true), 125);
        assert_eq!(step_zoom(100.0, false), 80);
        assert_eq!(step_zoom(133.0, true), 150);
        assert_eq!(step_zoom(133.0, false), 125);
        assert_eq!(step_zoom(600.0, true), 600);
        assert_eq!(step_zoom(25.0, false), 25);
    }

    #[test]
    fn prefs_survive_json_and_unknown_or_missing_fields() {
        let p = PdfPrefs {
            zoom: "150".into(),
            single: true,
            crop: true,
            invert: true,
            pages: false,
        };
        let t = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<PdfPrefs>(&t).unwrap(), p);
        let old: PdfPrefs = serde_json::from_str(r#"{"invert": true, "future": 1}"#).unwrap();
        assert!(old.invert && old.pages && old.zoom == "width");
        assert_eq!(old.zoom_mode(), ZoomMode::FitWidth);
        assert_eq!(p.zoom_mode(), ZoomMode::Percent(150));
        assert_eq!(p.view_mode(), ViewMode::Single);
        assert_eq!(
            PdfPrefs {
                zoom: "junk".into(),
                ..Default::default()
            }
            .zoom_mode(),
            ZoomMode::FitWidth
        );
    }

    #[test]
    fn zoom_names_round_trip() {
        for z in [
            ZoomMode::FitWidth,
            ZoomMode::FitPage,
            ZoomMode::Percent(150),
        ] {
            assert_eq!(ZoomMode::from_name(&z.name()), Some(z));
        }
        assert_eq!(ZoomMode::from_name("150%"), Some(ZoomMode::Percent(150)));
        assert_eq!(ZoomMode::from_name("nope"), None);
        assert_eq!(ZoomMode::from_name("5"), Some(ZoomMode::Percent(10)));
    }

    #[test]
    fn render_scale_rounds_up_to_eighths_and_keys_are_stable() {
        assert_eq!(render_scale(1.33), 1.375);
        assert_eq!(render_scale(1.0), 1.0);
        assert_eq!(render_scale(0.0), 0.125);
        assert_eq!(render_key(3, 1.375, false), (3, 1375, false));
        assert_ne!(render_key(3, 1.375, true), render_key(3, 1.375, false));
    }

    #[test]
    fn progress_and_captions() {
        assert_eq!(progress(0, 0.0, 10), 0.0);
        assert!((progress(4, 0.5, 10) - 0.45).abs() < 1e-9);
        assert_eq!(progress(9, 1.0, 10), 1.0);
        assert_eq!(progress(0, 0.5, 0), 0.0);
        let labels: Vec<String> = ["i", "ii", "1"].iter().map(|s| s.to_string()).collect();
        assert_eq!(page_caption(&labels, 1, 3), "ii (2 of 3)");
        assert_eq!(page_caption(&["1".into()], 0, 1), "1 of 1");
        assert_eq!(page_caption(&[], 0, 4), "1 of 4");
    }

    #[test]
    fn the_page_box_takes_labels_and_numbers() {
        let labels: Vec<String> = ["i", "ii", "1", "2", "3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(resolve_page("ii", &labels, 5), Some(1));
        assert_eq!(resolve_page("II", &labels, 5), Some(1));
        assert_eq!(
            resolve_page("2", &labels, 5),
            Some(3),
            "a label wins over the index"
        );
        assert_eq!(resolve_page("5", &labels, 5), Some(4));
        assert_eq!(resolve_page("99", &labels, 5), Some(4));
        assert_eq!(resolve_page("0", &labels, 5), None);
        assert_eq!(resolve_page("x", &labels, 5), None);
        assert_eq!(resolve_page(" ", &labels, 5), None);
    }

    #[test]
    fn pdf_annotations_round_trip_through_the_style_field() {
        let s = pdf_style("underline", &[[1.0, 2.0, 30.5, 40.0], [5.0, 6.0, 7.0, 8.0]]);
        assert_eq!(s, "pdf|underline|1.0,2.0,30.5,40.0;5.0,6.0,7.0,8.0");
        let (st, r) = parse_pdf_style(&s).unwrap();
        assert_eq!(st, "underline");
        assert_eq!(r, vec![[1.0, 2.0, 30.5, 40.0], [5.0, 6.0, 7.0, 8.0]]);
        assert_eq!(parse_pdf_style("background"), None);
        assert_eq!(parse_pdf_style("pdf").map(|x| x.1.len()), Some(0));
        assert_eq!(
            parse_pdf_style("pdf|x|1,2,3").unwrap().1.len(),
            0,
            "a broken rectangle is dropped"
        );
    }

    #[test]
    fn epub_highlights_are_not_pdf_ones() {
        let mut h = Highlight {
            id: "a".into(),
            spine: 0,
            start: 0,
            end: 1,
            color: "yellow".into(),
            style: "background".into(),
            note: String::new(),
            text: "t".into(),
            created: chrono::Utc::now(),
        };
        assert!(!is_pdf(&h));
        h.style = pdf_style("background", &[]);
        assert!(is_pdf(&h));
    }

    #[test]
    fn pixel_and_point_conversions_are_inverse() {
        let place = Place {
            page: 0,
            x: 12.0,
            y: 100.0,
            w: 800.0,
            h: 1000.0,
            scale: 2.0,
        };
        let crop = Some([0.1, 0.2, 0.9, 0.9]);
        let size = (400.0, 500.0);
        let px = to_px(&place, crop, size, [60.0, 150.0, 80.0, 170.0]);
        assert_eq!(
            px,
            [
                12.0 + 20.0 * 2.0,
                100.0 + 50.0 * 2.0,
                12.0 + 40.0 * 2.0,
                100.0 + 70.0 * 2.0
            ]
        );
        let (x, y) = to_pt(&place, crop, size, px[0], px[1]);
        assert!((x - 60.0).abs() < 1e-3 && (y - 150.0).abs() < 1e-3);
    }

    #[test]
    fn stepping_through_hits_wraps() {
        let hits = vec![(0, 5), (0, 40), (3, 1)];
        assert_eq!(step_hit(&hits, 0, 0, true), Some(0));
        assert_eq!(step_hit(&hits, 0, 5, true), Some(1));
        assert_eq!(step_hit(&hits, 3, 1, true), Some(0));
        assert_eq!(step_hit(&hits, 0, 5, false), Some(2));
        assert_eq!(step_hit(&hits, 3, 1, false), Some(1));
        assert_eq!(step_hit(&[], 0, 0, true), None);
    }
}
