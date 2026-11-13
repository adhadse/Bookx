//! The zoom and pan arithmetic of the reader's image viewer (R44, F7), without GTK.

/// Smallest and largest zoom, as a multiple of the picture's own size.
pub const MIN_ZOOM: f64 = 0.05;
pub const MAX_ZOOM: f64 = 16.0;
/// One press of + or -.
pub const STEP: f64 = 1.25;

/// How the picture fits the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// The whole picture is visible.
    Fit,
    /// A fixed multiple of its size.
    Scale(f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub mode: Mode,
    /// Picture size in pixels.
    pub image: (f64, f64),
    /// Visible area in pixels.
    pub window: (f64, f64),
}

impl View {
    pub fn new(image: (f64, f64), window: (f64, f64)) -> Self {
        View {
            mode: Mode::Fit,
            image,
            window,
        }
    }

    /// The scale that shows the whole picture (never above 1:1 for a small picture it would blur; it is
    /// still allowed to fill the window up to 4x so small figures are readable).
    pub fn fit_scale(&self) -> f64 {
        let (iw, ih) = self.image;
        let (ww, wh) = self.window;
        if iw <= 0.0 || ih <= 0.0 || ww <= 0.0 || wh <= 0.0 {
            return 1.0;
        }
        (ww / iw).min(wh / ih).clamp(MIN_ZOOM, 4.0)
    }

    pub fn scale(&self) -> f64 {
        match self.mode {
            Mode::Fit => self.fit_scale(),
            Mode::Scale(s) => s.clamp(MIN_ZOOM, MAX_ZOOM),
        }
    }

    /// The picture's size on screen.
    pub fn size(&self) -> (f64, f64) {
        let s = self.scale();
        (
            (self.image.0 * s).round().max(1.0),
            (self.image.1 * s).round().max(1.0),
        )
    }

    pub fn zoom_by(&mut self, factor: f64) {
        let s = (self.scale() * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.mode = if (s - self.fit_scale()).abs() < 1e-3 {
            Mode::Fit
        } else {
            Mode::Scale(s)
        };
    }

    pub fn zoom_in(&mut self) {
        self.zoom_by(STEP);
    }

    pub fn zoom_out(&mut self) {
        self.zoom_by(1.0 / STEP);
    }

    pub fn fit(&mut self) {
        self.mode = Mode::Fit;
    }

    pub fn actual_size(&mut self) {
        self.mode = Mode::Scale(1.0);
    }

    /// Toggles between fitting and 100% (a double click).
    pub fn toggle(&mut self) {
        if self.mode == Mode::Fit && (self.fit_scale() - 1.0).abs() > 1e-3 {
            self.actual_size();
        } else if self.mode == Mode::Fit {
            self.mode = Mode::Scale(2.0);
        } else {
            self.fit();
        }
    }

    pub fn percent(&self) -> u32 {
        (self.scale() * 100.0).round() as u32
    }

    /// Scroll offsets after a zoom, keeping the picture point under `focus` (in window pixels)
    /// where it was. `scroll` is the offset before, `old` the scale before.
    pub fn scroll_after_zoom(&self, scroll: (f64, f64), old: f64, focus: (f64, f64)) -> (f64, f64) {
        let new = self.scale();
        if old <= 0.0 {
            return scroll;
        }
        let r = new / old;
        let x = (scroll.0 + focus.0) * r - focus.0;
        let y = (scroll.1 + focus.1) * r - focus.1;
        self.clamp_scroll((x, y))
    }

    /// Keeps the offsets inside the picture (a picture smaller than the window has none).
    pub fn clamp_scroll(&self, s: (f64, f64)) -> (f64, f64) {
        let (w, h) = self.size();
        (
            s.0.clamp(0.0, (w - self.window.0).max(0.0)),
            s.1.clamp(0.0, (h - self.window.1).max(0.0)),
        )
    }

    /// Pinch: the scale at the start of the gesture times the gesture's factor.
    pub fn pinch(&mut self, start_scale: f64, factor: f64) {
        let s = (start_scale * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.mode = Mode::Scale(s);
    }
}

/// A file name for *Save Image As…* from the picture's path in the book.
pub fn suggest_name(src: &str) -> String {
    let name = src.rsplit('/').next().unwrap_or("image");
    let name = name.split(['?', '#']).next().unwrap_or(name);
    if name.is_empty() || !name.contains('.') {
        "image.png".into()
    } else {
        name.to_string()
    }
}

/// The name for *Save Image As…*: the picture's own name, with the extension its bytes say when
/// the name has none.
pub fn name_for(src: &str, bytes: &[u8]) -> String {
    let name = suggest_name(src);
    if src.rsplit('/').next().is_some_and(|n| n.contains('.')) {
        return name;
    }
    format!("image.{}", sniff(bytes).unwrap_or("png"))
}

/// The picture's format from its first bytes: `png`, `jpg`, `gif`, `webp`, `svg`.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.windows(4).take(256).any(|w| w == b"<svg") {
        Some("svg")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v() -> View {
        View::new((2000.0, 1000.0), (800.0, 600.0))
    }

    #[test]
    fn fit_shows_the_whole_picture() {
        let v = v();
        assert!((v.fit_scale() - 0.4).abs() < 1e-9);
        assert_eq!(v.size(), (800.0, 400.0));
        assert_eq!(v.percent(), 40);
    }

    #[test]
    fn a_small_picture_is_enlarged_only_up_to_four_times() {
        let v = View::new((100.0, 100.0), (1000.0, 1000.0));
        assert_eq!(v.fit_scale(), 4.0);
        let v = View::new((0.0, 0.0), (1000.0, 1000.0));
        assert_eq!(v.fit_scale(), 1.0);
    }

    #[test]
    fn zooming_steps_and_clamps() {
        let mut v = v();
        v.zoom_in();
        assert!((v.scale() - 0.5).abs() < 1e-9);
        assert_eq!(v.mode, Mode::Scale(0.5));
        for _ in 0..50 {
            v.zoom_in();
        }
        assert_eq!(v.scale(), MAX_ZOOM);
        for _ in 0..80 {
            v.zoom_out();
        }
        assert_eq!(v.scale(), MIN_ZOOM);
    }

    #[test]
    fn zooming_back_to_the_fit_scale_is_fit_again() {
        let mut v = v();
        v.zoom_in();
        v.zoom_out();
        assert_eq!(v.mode, Mode::Fit);
    }

    #[test]
    fn actual_size_and_toggle() {
        let mut v = v();
        v.actual_size();
        assert_eq!(v.percent(), 100);
        v.toggle();
        assert_eq!(v.mode, Mode::Fit);
        v.toggle();
        assert_eq!(v.mode, Mode::Scale(1.0));
        // a picture that fits at 100% toggles to 200%
        let mut small = View::new((400.0, 300.0), (400.0, 300.0));
        small.toggle();
        assert_eq!(small.mode, Mode::Scale(2.0));
    }

    #[test]
    fn the_point_under_the_pointer_stays_put() {
        let mut v = v();
        v.actual_size();
        let old = v.scale();
        // looking at picture point (1000, 500) with the pointer at window (400, 300)
        let scroll = (600.0, 200.0);
        v.zoom_by(2.0);
        let after = v.scroll_after_zoom(scroll, old, (400.0, 300.0));
        // in the new scale that point is at (2000, 1000): offset is that minus the pointer
        assert_eq!(after, (1600.0, 700.0));
    }

    #[test]
    fn scroll_stays_inside_the_picture() {
        let mut v = v();
        assert_eq!(v.clamp_scroll((50.0, 50.0)), (0.0, 0.0)); // fits: nothing to scroll
        v.actual_size();
        assert_eq!(v.clamp_scroll((5000.0, -4.0)), (1200.0, 0.0));
    }

    #[test]
    fn pinch_scales_from_where_it_started() {
        let mut v = v();
        v.pinch(1.0, 1.5);
        assert_eq!(v.scale(), 1.5);
        v.pinch(1.0, 1000.0);
        assert_eq!(v.scale(), MAX_ZOOM);
    }

    #[test]
    fn names_and_formats() {
        assert_eq!(suggest_name("OEBPS/images/fig 1.jpg"), "fig 1.jpg");
        assert_eq!(suggest_name("OEBPS/images/"), "image.png");
        assert_eq!(suggest_name("cover"), "image.png");
        assert_eq!(sniff(&[0x89, b'P', b'N', b'G', 13]), Some("png"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniff(b"GIF89a"), Some("gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff(b"<?xml?><svg xmlns=''/>"), Some("svg"));
        assert_eq!(sniff(b"plain"), None);
        assert_eq!(name_for("a/b/fig.png", b"x"), "fig.png");
        assert_eq!(name_for("a/b/cover", &[0xFF, 0xD8, 0xFF]), "image.jpg");
        assert_eq!(name_for("a/b/cover", b"unknown"), "image.png");
    }
}
