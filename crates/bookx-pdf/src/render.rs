//! Page rendering (CPU, pure Rust) and the pixel helpers around it.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

use crate::doc::Document;

/// Largest side of a rendered page in pixels.
pub const MAX_SIDE: f32 = 8192.0;
/// Largest rendered page in pixels (width times height).
pub const MAX_PIXELS: f32 = 36.0e6;

/// A rendered page: RGBA, 8 bit, rows top to bottom, opaque.
#[derive(Debug, Clone, PartialEq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Bitmap {
    pub fn bytes(&self) -> usize {
        self.rgba.len()
    }

    /// Inverts the colours (night reading, e-ink with a dark theme); alpha is kept.
    pub fn invert(&mut self) {
        for px in self.rgba.chunks_exact_mut(4) {
            px[0] = 255 - px[0];
            px[1] = 255 - px[1];
            px[2] = 255 - px[2];
        }
    }

    /// The box of the content (anything darker than `threshold` or different from the paper) as
    /// fractions `[x0, y0, x1, y1]` of the page, `None` for a blank page.
    pub fn content_box(&self, threshold: u8) -> Option<[f32; 4]> {
        let (w, h) = (self.width as usize, self.height as usize);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for y in 0..h {
            let row = &self.rgba[y * w * 4..(y + 1) * w * 4];
            for (x, px) in row.chunks_exact(4).enumerate() {
                if px[0] < threshold || px[1] < threshold || px[2] < threshold {
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        if x0 == usize::MAX {
            return None;
        }
        Some([
            x0 as f32 / w as f32,
            y0 as f32 / h as f32,
            (x1 + 1) as f32 / w as f32,
            (y1 + 1) as f32 / h as f32,
        ])
    }
}

/// The scale to render a `w` by `h` point page at so that neither side nor the area gets
/// out of hand.
pub fn clamp_scale(w: f32, h: f32, scale: f32) -> f32 {
    let s = if scale.is_finite() { scale } else { 1.0 }.clamp(0.05, 16.0);
    let by_side = MAX_SIDE / w.max(h).max(1.0);
    let by_area = (MAX_PIXELS / (w * h).max(1.0)).sqrt();
    s.min(by_side).min(by_area)
}

/// Renders pages of one document; keeps font and glyph caches between pages.
pub struct Renderer<'d> {
    doc: &'d Document,
    cache: RenderCache<'d>,
}

impl Document {
    pub fn renderer(&self) -> Renderer<'_> {
        Renderer {
            doc: self,
            cache: RenderCache::new(),
        }
    }
}

impl<'d> Renderer<'d> {
    /// Page `page` at `scale` pixels per point.
    pub fn render(&self, page: usize, scale: f32, invert: bool) -> Option<Bitmap> {
        let p = self.doc.pdf.pages().get(page)?;
        let (w, h) = p.render_dimensions();
        let s = clamp_scale(w, h, scale);
        let pix = hayro::render(
            p,
            &self.cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: s,
                y_scale: s,
                bg_color: WHITE,
            },
        );
        let mut bm = Bitmap {
            width: pix.width() as u32,
            height: pix.height() as u32,
            rgba: pix.data_as_u8_slice().to_vec(),
        };
        if invert {
            bm.invert();
        }
        Some(bm)
    }

    /// The page as PNG, `width` pixels wide at most (covers).
    pub fn render_png(&self, page: usize, width: u32) -> Option<Vec<u8>> {
        let (w, _) = self.doc.page_size(page);
        let pix_scale = width as f32 / w;
        let p = self.doc.pdf.pages().get(page)?;
        let (pw, ph) = p.render_dimensions();
        let s = clamp_scale(pw, ph, pix_scale);
        let pix = hayro::render(
            p,
            &self.cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: s,
                y_scale: s,
                bg_color: WHITE,
            },
        );
        pix.into_png().ok()
    }

    /// The margins to crop: the content box of the page found on a small rendering.
    pub fn content_box(&self, page: usize) -> Option<[f32; 4]> {
        let bm = self.render(page, 0.35, false)?;
        bm.content_box(235)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testpdf::{Page, TestPdf};

    #[test]
    fn scale_is_clamped_by_side_and_area() {
        assert_eq!(clamp_scale(612.0, 792.0, 1.0), 1.0);
        assert!(clamp_scale(612.0, 792.0, 100.0) <= 16.0);
        let s = clamp_scale(612.0, 792.0, 16.0);
        assert!(612.0 * s * 792.0 * s <= MAX_PIXELS * 1.01);
        assert!(792.0 * s <= MAX_SIDE + 1.0);
        // a huge poster page
        let s = clamp_scale(20000.0, 20000.0, 1.0);
        assert!(20000.0 * s <= MAX_SIDE + 1.0);
        assert_eq!(clamp_scale(612.0, 792.0, f32::NAN), 1.0);
        assert!(clamp_scale(612.0, 792.0, -3.0) >= 0.05);
    }

    #[test]
    fn invert_flips_colour_not_alpha() {
        let mut b = Bitmap {
            width: 1,
            height: 2,
            rgba: vec![0, 10, 255, 255, 255, 255, 255, 128],
        };
        b.invert();
        assert_eq!(b.rgba, vec![255, 245, 0, 255, 0, 0, 0, 128]);
    }

    #[test]
    fn content_box_finds_the_ink() {
        let mut b = Bitmap {
            width: 10,
            height: 10,
            rgba: vec![255; 400],
        };
        for (x, y) in [(2, 3), (6, 7)] {
            let i = (y * 10 + x) * 4;
            b.rgba[i] = 0;
            b.rgba[i + 1] = 0;
            b.rgba[i + 2] = 0;
        }
        assert_eq!(b.content_box(200), Some([0.2, 0.3, 0.7, 0.8]));
        let blank = Bitmap {
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        };
        assert_eq!(blank.content_box(200), None);
    }

    #[test]
    fn a_page_renders_to_the_asked_size_with_ink_on_paper() {
        let d = Document::open(TestPdf::simple(&[&["Hello world"]]).build(), None).unwrap();
        let r = d.renderer();
        let bm = r.render(0, 0.5, false).unwrap();
        assert_eq!((bm.width, bm.height), (306, 396));
        assert_eq!(bm.rgba.len(), 306 * 396 * 4);
        assert!(bm.rgba.chunks_exact(4).any(|p| p[0] < 100), "no ink");
        assert_eq!(&bm.rgba[..4], &[255, 255, 255, 255], "paper is white");
        let inv = r.render(0, 0.5, true).unwrap();
        assert_eq!(&inv.rgba[..4], &[0, 0, 0, 255]);
        assert!(r.render(5, 1.0, false).is_none());
    }

    #[test]
    fn the_content_box_excludes_the_margins() {
        let page = Page::new(200.0, 200.0).rect([50.0, 50.0, 150.0, 120.0], 0.0);
        let d = Document::open(TestPdf::new().page(page).build(), None).unwrap();
        let b = d.renderer().content_box(0).unwrap();
        assert!((b[0] - 0.25).abs() < 0.03, "{b:?}");
        assert!((b[2] - 0.75).abs() < 0.03, "{b:?}");
        assert!((b[1] - 0.40).abs() < 0.03, "{b:?}");
        assert!((b[3] - 0.75).abs() < 0.03, "{b:?}");
    }

    #[test]
    fn the_cover_png_has_the_png_signature() {
        let d = Document::open(TestPdf::simple(&[&["x"]]).build(), None).unwrap();
        let png = d.renderer().render_png(0, 200).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }
}
