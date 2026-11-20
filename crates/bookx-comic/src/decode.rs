//! Decoding one page: size limits first (image bombs), then the pixels, reduced so that a huge scan
//! does not fill the memory, optionally grey, and the panels found on the reduced image.

use std::io::Cursor;

use anyhow::{bail, Context, Result};
use image::{imageops::FilterType, ImageReader, Limits};

use crate::panels::{self, Gray, Rect};

/// A page claiming more pixels than this is refused (a 20000 x 20000 PNG is 400 megapixels).
pub const MAX_SOURCE_PIXELS: u64 = 160_000_000;
/// A page claiming a longer side than this is refused.
pub const MAX_SOURCE_SIDE: u32 = 40_000;
/// Memory the decoder itself may allocate for one page.
pub const MAX_DECODER_ALLOC: u64 = 1 << 30;

/// What to decode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opts {
    /// Longest side of the result.
    pub max_edge: u32,
    /// Pixels of the result.
    pub max_pixels: u64,
    /// Grey instead of colour (e-ink).
    pub grey: bool,
    /// Find the panels, reading right to left.
    pub panels: Option<bool>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            max_edge: 4096,
            max_pixels: 12_000_000,
            grey: false,
            panels: Some(false),
        }
    }
}

/// A decoded page.
#[derive(Debug, Clone)]
pub struct Page {
    /// Size of the pixels below.
    pub width: u32,
    pub height: u32,
    /// Size of the picture in the file.
    pub orig_width: u32,
    pub orig_height: u32,
    /// RGBA, opaque.
    pub rgba: Vec<u8>,
    /// Panels in reading order, as fractions of the page.
    pub panels: Vec<Rect>,
}

impl Page {
    /// The pixels the way cairo wants them (B, G, R, A in memory; alpha is 255, so premultiplying
    /// changes nothing). Done on the worker thread so the main thread only wraps the bytes.
    pub fn into_bgra(mut self) -> Page {
        for p in self.rgba.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        self
    }

    /// Memory the pixels take.
    pub fn bytes(&self) -> usize {
        self.rgba.len() + self.panels.len() * 32
    }
}

fn reader(bytes: &[u8]) -> Result<ImageReader<Cursor<&[u8]>>> {
    let mut r = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("unknown image format")?;
    let mut l = Limits::default();
    l.max_image_width = Some(MAX_SOURCE_SIDE);
    l.max_image_height = Some(MAX_SOURCE_SIDE);
    l.max_alloc = Some(MAX_DECODER_ALLOC);
    r.limits(l);
    Ok(r)
}

/// The size of the picture, read from the header only, and checked against the limits.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    let (w, h) = reader(bytes)?
        .into_dimensions()
        .context("cannot read the image header")?;
    check_size(w, h)?;
    Ok((w, h))
}

/// Whether the header of the picture claims a size beyond the limits (an image bomb). Pictures
/// whose header cannot be read at all are not bombs, they are just broken.
pub fn is_bomb(bytes: &[u8]) -> bool {
    let Ok(r) = ImageReader::new(Cursor::new(bytes)).with_guessed_format() else {
        return false;
    };
    match r.into_dimensions() {
        Ok((w, h)) => {
            w > MAX_SOURCE_SIDE
                || h > MAX_SOURCE_SIDE
                || (w as u64) * (h as u64) > MAX_SOURCE_PIXELS
        }
        Err(_) => false,
    }
}

/// Refuses sizes that are empty or too big.
pub fn check_size(w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 {
        bail!("the image is empty");
    }
    if w > MAX_SOURCE_SIDE || h > MAX_SOURCE_SIDE || (w as u64) * (h as u64) > MAX_SOURCE_PIXELS {
        bail!("the image is too large ({w} x {h} pixels)");
    }
    Ok(())
}

/// The size a `w` x `h` picture is reduced to (never enlarged, aspect kept).
pub fn target_size(w: u32, h: u32, o: &Opts) -> (u32, u32) {
    let by_edge = o.max_edge as f64 / w.max(h) as f64;
    let by_pixels = (o.max_pixels as f64 / (w as f64 * h as f64)).sqrt();
    let f = by_edge.min(by_pixels).min(1.0);
    (
        ((w as f64 * f).floor() as u32).max(1),
        ((h as f64 * f).floor() as u32).max(1),
    )
}

/// Decodes a page. Checks the header before it allocates anything.
pub fn decode_page(bytes: &[u8], o: &Opts) -> Result<Page> {
    let (ow, oh) = dimensions(bytes)?;
    let img = reader(bytes)?.decode().context("cannot decode the image")?;
    let (tw, th) = target_size(ow, oh, o);
    let mut rgba = if (tw, th) != (ow, oh) {
        image::imageops::resize(&img.to_rgba8(), tw, th, FilterType::Triangle).into_raw()
    } else {
        img.to_rgba8().into_raw()
    };
    // pictures with transparency are shown on white paper
    for p in rgba.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 255 * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
    }
    let (w, h) = (tw as usize, th as usize);
    let gray = Gray::from_rgba(&rgba, w, h);
    let found = o
        .panels
        .map(|rtl| panels::detect(&gray, rtl, panels::Opts::default()))
        .unwrap_or_default();
    if o.grey {
        for (p, g) in rgba.chunks_exact_mut(4).zip(&gray.px) {
            p[0] = *g;
            p[1] = *g;
            p[2] = *g;
        }
    }
    Ok(Page {
        width: tw,
        height: th,
        orig_width: ow,
        orig_height: oh,
        rgba,
        panels: found,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::{bomb_png, jpeg, png, Layout};

    #[test]
    fn decodes_png_and_jpeg_and_finds_panels() {
        for bytes in [
            png(300, 450, Layout::Grid(2, 2), 255),
            jpeg(300, 450, Layout::Grid(2, 2), 255),
        ] {
            let p = decode_page(&bytes, &Opts::default()).unwrap();
            assert_eq!((p.width, p.height), (300, 450));
            assert_eq!(p.rgba.len(), 300 * 450 * 4);
            assert_eq!(p.panels.len(), 4, "{:?}", p.panels);
        }
    }

    #[test]
    fn big_pages_are_reduced_and_small_ones_are_not_enlarged() {
        let o = Opts {
            max_edge: 500,
            ..Opts::default()
        };
        let p = decode_page(&png(1000, 1500, Layout::Splash, 255), &o).unwrap();
        assert_eq!((p.width, p.height), (333, 500));
        assert_eq!((p.orig_width, p.orig_height), (1000, 1500));
        let p = decode_page(&png(200, 300, Layout::Splash, 255), &o).unwrap();
        assert_eq!((p.width, p.height), (200, 300));
        assert_eq!(target_size(4000, 6000, &Opts::default()), (2730, 4096));
        assert_eq!(target_size(800, 30000, &Opts::default()), (109, 4096));
    }

    #[test]
    fn pixel_budget_limits_wide_scans() {
        let o = Opts {
            max_edge: 100_000,
            max_pixels: 1_000_000,
            ..Opts::default()
        };
        let (w, h) = target_size(4000, 3000, &o);
        assert!((w as u64) * (h as u64) <= 1_000_000);
    }

    #[test]
    fn grey_removes_colour() {
        let o = Opts {
            grey: true,
            panels: None,
            ..Opts::default()
        };
        let p = decode_page(&png(100, 150, Layout::Grid(2, 2), 255), &o).unwrap();
        assert!(p.rgba.chunks_exact(4).all(|c| c[0] == c[1] && c[1] == c[2]));
        assert!(p.panels.is_empty());
    }

    #[test]
    fn image_bombs_are_refused_from_the_header() {
        // 30000 x 30000 = 900 megapixels in a 70 byte file
        let b = bomb_png(30_000, 30_000);
        assert!(b.len() < 100);
        let t = std::time::Instant::now();
        let e = decode_page(&b, &Opts::default()).unwrap_err().to_string();
        assert!(e.contains("too large"), "{e}");
        assert!(t.elapsed().as_secs() < 2);
        let e = dimensions(&bomb_png(50_000, 10)).unwrap_err().to_string();
        assert!(!e.is_empty());
        assert!(is_bomb(&b));
        assert!(!is_bomb(&png(10, 10, Layout::Splash, 255)));
        assert!(!is_bomb(b"junk"));
        assert!(check_size(0, 5).is_err());
        assert!(check_size(20_000, 20_000).is_err());
        assert!(check_size(4000, 6000).is_ok());
    }

    #[test]
    fn a_small_header_with_missing_data_fails_cleanly() {
        let b = bomb_png(2000, 2000);
        assert!(decode_page(&b, &Opts::default()).is_err());
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_page(b"", &Opts::default()).is_err());
        assert!(decode_page(b"not an image at all", &Opts::default()).is_err());
        let mut p = png(50, 50, Layout::Splash, 255);
        p.truncate(p.len() / 2);
        assert!(decode_page(&p, &Opts::default()).is_err());
    }

    #[test]
    fn bgra_swaps_red_and_blue() {
        let p = Page {
            width: 1,
            height: 1,
            orig_width: 1,
            orig_height: 1,
            rgba: vec![1, 2, 3, 255],
            panels: vec![],
        };
        assert_eq!(p.into_bgra().rgba, vec![3, 2, 1, 255]);
    }

    #[test]
    fn transparent_pixels_become_white() {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        let p = decode_page(
            &out.into_inner(),
            &Opts {
                panels: None,
                ..Opts::default()
            },
        )
        .unwrap();
        assert_eq!(&p.rgba[..4], &[255, 255, 255, 255]);
    }
}
