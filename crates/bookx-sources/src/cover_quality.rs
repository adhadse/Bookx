//! How good a cover image is, and whether a candidate is a real improvement (roadmap PP-G-28).
//!
//! Only sizes are judged (read from the image header, no decoding), so scanning a library of
//! thousands of covers is quick. The review table of *Fetch Metadata* uses [`is_upgrade`] to offer a
//! larger cover as a safe change; Library Health lists the books whose cover is [`Grade::Low`] or
//! [`Grade::Tiny`].

use crate::cover;

/// A cover is `Good` from this height on (a screen of an e-reader or a retina grid).
pub const GOOD_HEIGHT: u32 = 600;
/// Below this height a cover looks blurry in the library grid.
pub const TINY_HEIGHT: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Grade {
    Tiny,
    Low,
    Good,
}

impl Grade {
    pub fn label(self) -> &'static str {
        match self {
            Grade::Tiny => "very small",
            Grade::Low => "low resolution",
            Grade::Good => "good",
        }
    }
}

/// Width and height in pixels.
pub type Size = (u32, u32);

pub fn grade(size: Size) -> Grade {
    let (w, h) = size;
    // both sides count, so a 2000×300 banner is not good
    let long = h.max(w);
    let short = h.min(w);
    if long < TINY_HEIGHT || short < TINY_HEIGHT * 2 / 3 {
        Grade::Tiny
    } else if long < GOOD_HEIGHT || short < GOOD_HEIGHT * 2 / 3 {
        Grade::Low
    } else {
        Grade::Good
    }
}

/// Book covers are upright rectangles (about 2:3). Squares, banners and sideways pictures are odd.
pub fn odd_shape(size: Size) -> bool {
    let (w, h) = size;
    if w == 0 || h == 0 {
        return true;
    }
    let r = w as f64 / h as f64;
    !(0.45..=0.85).contains(&r)
}

/// The size of a JPEG or PNG, `None` when it is neither.
pub fn size_of(bytes: &[u8]) -> Option<Size> {
    cover::info(bytes).ok().map(|(w, h, _)| (w, h))
}

/// Is `candidate` worth replacing `current` with? It must not be a worse shape, must be at least
/// half as big again, and must not be tiny itself.
pub fn is_upgrade(current: Size, candidate: Size) -> bool {
    if grade(candidate) == Grade::Tiny {
        return false;
    }
    if odd_shape(candidate) && !odd_shape(current) {
        return false;
    }
    let px = |s: Size| s.0 as u64 * s.1 as u64;
    px(candidate) * 2 >= px(current) * 3
}

pub fn describe(size: Size) -> String {
    format!("{} × {} px", size.0, size.1)
}

/// What to say about a cover in a list: size, grade and shape.
pub fn describe_quality(size: Size) -> String {
    let mut s = format!("{}, {}", describe(size), grade(size).label());
    if odd_shape(size) {
        s.push_str(", unusual shape");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_by_size() {
        assert_eq!(grade((200, 299)), Grade::Tiny);
        assert_eq!(grade((120, 180)), Grade::Tiny);
        assert_eq!(grade((320, 480)), Grade::Low);
        assert_eq!(grade((400, 600)), Grade::Good);
        assert_eq!(grade((800, 1200)), Grade::Good);
        assert_eq!(grade((2000, 150)), Grade::Tiny);
        assert_eq!(grade((1500, 380)), Grade::Low);
    }

    #[test]
    fn shapes() {
        assert!(!odd_shape((400, 600)));
        assert!(odd_shape((600, 600)));
        assert!(odd_shape((1200, 400)));
        assert!(odd_shape((0, 10)));
    }

    #[test]
    fn upgrade_needs_a_real_gain() {
        assert!(is_upgrade((320, 480), (800, 1200)));
        assert!(!is_upgrade((800, 1200), (810, 1215)), "barely bigger");
        assert!(!is_upgrade((800, 1200), (200, 300)), "smaller");
        assert!(!is_upgrade((320, 480), (250, 280)), "tiny candidate");
        assert!(
            !is_upgrade((320, 480), (1200, 1200)),
            "square replaces upright"
        );
        assert!(is_upgrade((1200, 1200), (1200, 1800)));
    }

    #[test]
    fn sizes_come_from_headers() {
        let png = {
            let img = image::RgbImage::from_pixel(40, 60, image::Rgb([1, 2, 3]));
            let mut v = Vec::new();
            image::DynamicImage::ImageRgb8(img)
                .write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png)
                .unwrap();
            v
        };
        assert_eq!(size_of(&png), Some((40, 60)));
        assert_eq!(size_of(b"nope"), None);
    }

    #[test]
    fn descriptions() {
        assert_eq!(describe((10, 20)), "10 × 20 px");
        assert_eq!(
            describe_quality((600, 600)),
            "600 × 600 px, good, unusual shape"
        );
    }
}
