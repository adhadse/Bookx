//! The reading ruler (roadmap R64, PP-G-19): a band of the page stays clear while the rest is
//! dimmed, so the eyes keep their line. Dyslexic readers and people with low vision use it; so do
//! tired eyes. This file is the logic without GTK (settings, size of the band, where it sits); the
//! overlay is `ui/reading_ruler.rs`.

use serde::{Deserialize, Serialize};

pub const MIN_LINES: u8 = 1;
pub const MAX_LINES: u8 = 5;
pub const MIN_DIM: u8 = 20;
pub const MAX_DIM: u8 = 90;

/// What the user chose (kept in the settings file; off by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RulerPrefs {
    pub enabled: bool,
    /// How many lines of text stay clear.
    pub lines: u8,
    /// How strongly the rest is dimmed, in percent.
    pub dim: u8,
}

impl Default for RulerPrefs {
    fn default() -> Self {
        RulerPrefs {
            enabled: false,
            lines: 2,
            dim: 60,
        }
    }
}

impl RulerPrefs {
    pub fn clamped(self) -> RulerPrefs {
        RulerPrefs {
            enabled: self.enabled,
            lines: self.lines.clamp(MIN_LINES, MAX_LINES),
            dim: self.dim.clamp(MIN_DIM, MAX_DIM),
        }
    }

    /// Opacity of the dimmed part, 0..1.
    pub fn alpha(self) -> f64 {
        self.clamped().dim as f64 / 100.0
    }
}

/// The height of one line of text in pixels, from the reader's font size (points, at 96 dpi) and
/// line spacing (a multiple of the font size).
pub fn line_height_px(font_size_pt: f64, line_spacing: f64) -> f64 {
    (font_size_pt.max(1.0) * 96.0 / 72.0 * line_spacing.max(1.0)).max(8.0)
}

/// Top and bottom of the clear band for a pointer at `y` in a view `view_h` high. The band keeps
/// to the view, so at the edges it stops instead of shrinking.
pub fn band(y: f64, view_h: f64, line_h: f64, lines: u8) -> (f64, f64) {
    let h = (line_h * lines.clamp(MIN_LINES, MAX_LINES) as f64).min(view_h.max(0.0));
    let top = (y - h / 2.0).clamp(0.0, (view_h - h).max(0.0));
    (top, top + h)
}

/// Moves the ruler by whole lines (`dir` -1 up, +1 down), staying inside the view.
pub fn step(y: f64, dir: i32, line_h: f64, view_h: f64) -> f64 {
    (y + dir.signum() as f64 * line_h).clamp(0.0, view_h.max(0.0))
}

/// Whether a touch at `y` is on the clear band `top..bottom` (a little room around it counts, so a
/// finger does not have to hit the band exactly).
pub fn hit_band(y: f64, top: f64, bottom: f64, slop: f64) -> bool {
    y >= top - slop && y <= bottom + slop
}

/// Where the middle of the band is after the finger moved `dy` from where the drag began (`from` is the
/// middle when it began), kept inside the view.
pub fn drag_to(from: f64, dy: f64, view_h: f64) -> f64 {
    (from + dy).clamp(0.0, view_h.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_and_moderate() {
        let p = RulerPrefs::default();
        assert!(!p.enabled);
        assert_eq!((p.lines, p.dim), (2, 60));
        assert!((p.alpha() - 0.6).abs() < 1e-9);
    }

    #[test]
    fn old_settings_files_still_load() {
        let p: RulerPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p, RulerPrefs::default());
        let p: RulerPrefs = serde_json::from_str(r#"{"enabled":true,"lines":3}"#).unwrap();
        assert!(p.enabled && p.lines == 3 && p.dim == 60);
    }

    #[test]
    fn values_are_clamped() {
        let p = RulerPrefs {
            enabled: true,
            lines: 99,
            dim: 5,
        }
        .clamped();
        assert_eq!((p.lines, p.dim), (MAX_LINES, MIN_DIM));
        assert!((RulerPrefs { dim: 255, ..p }.alpha() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn line_height_follows_font_and_spacing() {
        let a = line_height_px(14.0, 1.25);
        assert!((a - 14.0 * 96.0 / 72.0 * 1.25).abs() < 1e-9);
        assert!(line_height_px(28.0, 1.25) > a);
        assert!(line_height_px(0.0, 0.0) >= 8.0);
    }

    #[test]
    fn the_band_is_centred_and_stays_in_view() {
        assert_eq!(band(300.0, 600.0, 20.0, 2), (280.0, 320.0));
        assert_eq!(band(0.0, 600.0, 20.0, 2), (0.0, 40.0), "top edge");
        assert_eq!(band(599.0, 600.0, 20.0, 2), (560.0, 600.0), "bottom edge");
        // a tiny view: the band is the whole view
        assert_eq!(band(10.0, 30.0, 20.0, 5), (0.0, 30.0));
        assert_eq!(band(10.0, 0.0, 20.0, 1), (0.0, 0.0));
    }

    #[test]
    fn a_finger_near_the_band_grabs_it() {
        assert!(hit_band(300.0, 280.0, 320.0, 24.0));
        assert!(hit_band(260.0, 280.0, 320.0, 24.0));
        assert!(hit_band(340.0, 280.0, 320.0, 24.0));
        assert!(!hit_band(200.0, 280.0, 320.0, 24.0));
        assert!(!hit_band(400.0, 280.0, 320.0, 24.0));
    }

    #[test]
    fn dragging_moves_the_band_and_stops_at_the_edges() {
        assert_eq!(drag_to(300.0, 50.0, 600.0), 350.0);
        assert_eq!(drag_to(300.0, -400.0, 600.0), 0.0);
        assert_eq!(drag_to(300.0, 900.0, 600.0), 600.0);
    }

    #[test]
    fn stepping_moves_a_line_and_stops_at_the_edges() {
        assert_eq!(step(100.0, 1, 20.0, 600.0), 120.0);
        assert_eq!(step(100.0, -1, 20.0, 600.0), 80.0);
        assert_eq!(step(5.0, -1, 20.0, 600.0), 0.0);
        assert_eq!(step(595.0, 1, 20.0, 600.0), 600.0);
        assert_eq!(step(100.0, 0, 20.0, 600.0), 100.0);
    }
}
