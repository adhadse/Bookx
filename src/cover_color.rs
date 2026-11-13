//! The colours of the Continue Reading banner, without GTK: the dominant colour of a cover,
//! softened so that a saturated cover does not give a harsh slab, and a text colour with a
//! guaranteed contrast (WCAG 4.5:1) on both ends of the gradient, in light and in dark mode.

/// An sRGB colour.
pub type Rgb = [u8; 3];

/// The colours of the card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BannerColors {
    pub bg1: Rgb,
    pub bg2: Rgb,
    pub fg: Rgb,
}

fn lin(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The relative luminance of a colour (WCAG).
pub fn luminance(c: Rgb) -> f64 {
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// The contrast ratio of two colours (1 to 21).
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (
        c[0] as f64 / 255.0,
        c[1] as f64 / 255.0,
        c[2] as f64 / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-9 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < 1e-9 {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if (max - g).abs() < 1e-9 {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}

/// The dominant colour of some pixels: the average, with the more colourful pixels counting more
/// (a cover is mostly a paper-coloured background; the colour people see is the colourful part).
/// Black, white and grey pixels count a little so that a grey cover stays grey.
pub fn dominant(pixels: &[Rgb]) -> Option<Rgb> {
    if pixels.is_empty() {
        return None;
    }
    let (mut r, mut g, mut b, mut w) = (0.0, 0.0, 0.0, 0.0);
    for p in pixels {
        let (_, s, l) = to_hsl(*p);
        // very dark and very light pixels (text, borders, paper) say little about the cover
        let weight = 0.05 + s * (1.0 - (2.0 * l - 1.0).abs());
        r += p[0] as f64 * weight;
        g += p[1] as f64 * weight;
        b += p[2] as f64 * weight;
        w += weight;
    }
    Some([
        (r / w).round() as u8,
        (g / w).round() as u8,
        (b / w).round() as u8,
    ])
}

/// Black or white text, whichever reads better on `bg` (at least 4.5:1 for the colours this module
/// makes).
pub fn readable_fg(bg: Rgb) -> Rgb {
    let dark: Rgb = [0x1d, 0x1d, 0x20];
    let light: Rgb = [0xff, 0xff, 0xff];
    if contrast(dark, bg) >= contrast(light, bg) {
        dark
    } else {
        light
    }
}

/// The card colours for a cover colour: saturation is capped (no harsh yellow), light mode gets a
/// light pastel, dark mode a deep shade; the second colour of the gradient is a little different in
/// lightness. The text colour has at least 4.5:1 on both.
pub fn banner_colors(cover: Rgb, dark_mode: bool) -> BannerColors {
    let (h, s, _) = to_hsl(cover);
    let s = s.min(if dark_mode { 0.45 } else { 0.55 });
    let (l1, l2) = if dark_mode {
        (0.24, 0.17)
    } else {
        (0.80, 0.72)
    };
    let (bg1, bg2) = (from_hsl(h, s, l1), from_hsl(h, s, l2));
    // the text must read on the worse of the two
    let fg = {
        let cand = readable_fg(bg1);
        if contrast(cand, bg1) >= 4.5 && contrast(cand, bg2) >= 4.5 {
            cand
        } else {
            // fall back to the extreme that works on both ends
            let black: Rgb = [0, 0, 0];
            let white: Rgb = [255, 255, 255];
            let worst = |c: Rgb| contrast(c, bg1).min(contrast(c, bg2));
            if worst(black) >= worst(white) {
                black
            } else {
                white
            }
        }
    };
    BannerColors { bg1, bg2, fg }
}

/// `#rrggbb`.
pub fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_is_the_wcag_ratio() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 0.01);
        assert!((contrast([255, 255, 255], [255, 255, 255]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_dominant_colour_is_the_colourful_part() {
        // mostly white paper with some red
        let mut px = vec![[250, 250, 250]; 90];
        px.extend(vec![[200, 30, 30]; 10]);
        let d = dominant(&px).unwrap();
        assert!(d[0] > d[1] + 40 && d[0] > d[2] + 40, "{d:?}");
        assert_eq!(dominant(&[]), None);
        // a grey cover stays grey
        let g = dominant(&[[120, 120, 120], [130, 130, 130]]).unwrap();
        assert!((g[0] as i32 - g[1] as i32).abs() <= 1);
    }

    #[test]
    fn the_text_is_readable_on_every_cover_colour_in_both_modes() {
        let mut n = 0;
        for r in (0..=255).step_by(51) {
            for g in (0..=255).step_by(51) {
                for b in (0..=255).step_by(51) {
                    for dark in [false, true] {
                        let c = banner_colors([r, g, b], dark);
                        assert!(
                            contrast(c.fg, c.bg1) >= 4.5,
                            "{r},{g},{b} dark={dark}: {c:?}"
                        );
                        assert!(
                            contrast(c.fg, c.bg2) >= 4.5,
                            "{r},{g},{b} dark={dark}: {c:?}"
                        );
                        n += 1;
                    }
                }
            }
        }
        assert_eq!(n, 6 * 6 * 6 * 2);
    }

    #[test]
    fn a_saturated_cover_is_softened_and_dark_mode_is_dark() {
        let yellow = banner_colors([255, 215, 0], false);
        let (_, s, l) = to_hsl(yellow.bg1);
        assert!(
            s <= 0.56 && l > 0.7,
            "soft pastel, not a harsh slab: {s} {l}"
        );
        let dark = banner_colors([255, 215, 0], true);
        assert!(luminance(dark.bg1) < 0.1, "{dark:?}");
        assert!(luminance(dark.fg) > 0.5, "light text on dark");
        assert!(luminance(yellow.fg) < 0.2, "dark text on light");
    }
}
