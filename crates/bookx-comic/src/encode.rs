//! Writing a page as JPEG, for readers that produce raw pixels (DjVu).

use anyhow::{Result, bail};
use image::ExtendedColorType;
use image::codecs::jpeg::JpegEncoder;

/// Most pixels of a page that is encoded (a 200 megapixel scan is not a page).
pub const MAX_PIXELS: u64 = 120_000_000;

/// Encodes 8-bit pixels (`channels` is 1 for grey, 3 for RGB) as a JPEG.
pub fn jpeg(width: u32, height: u32, channels: u8, pixels: &[u8], quality: u8) -> Result<Vec<u8>> {
    let color = match channels {
        1 => ExtendedColorType::L8,
        3 => ExtendedColorType::Rgb8,
        _ => bail!("unsupported pixel format"),
    };
    let px = width as u64 * height as u64;
    if width == 0 || height == 0 || px > MAX_PIXELS || width > 65_535 || height > 65_535 {
        bail!("the page is empty or too large");
    }
    if pixels.len() as u64 != px * channels as u64 {
        bail!("the pixel data does not match the page size");
    }
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .encode(pixels, width, height, color)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_round_trips_through_jpeg() {
        let px: Vec<u8> = (0..30 * 20 * 3).map(|i| (i % 251) as u8).collect();
        let j = jpeg(30, 20, 3, &px, 85).unwrap();
        assert_eq!(&j[..2], &[0xFF, 0xD8]);
        assert_eq!(crate::decode::dimensions(&j).unwrap(), (30, 20));
        let g = jpeg(8, 8, 1, &[128; 64], 85).unwrap();
        assert_eq!(crate::decode::dimensions(&g).unwrap(), (8, 8));
    }

    #[test]
    fn wrong_sizes_are_errors() {
        assert!(jpeg(0, 5, 3, &[], 85).is_err());
        assert!(jpeg(4, 4, 3, &[0; 10], 85).is_err());
        assert!(jpeg(4, 4, 2, &[0; 32], 85).is_err());
        assert!(jpeg(70_000, 2, 1, &[0; 140_000], 85).is_err());
    }
}
