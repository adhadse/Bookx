//! Image helpers shared by the writers: sizes and format conversion.

use std::io::Cursor;

use image::{DynamicImage, ImageFormat};

/// Pixel size of a PNG, JPEG or GIF.
pub fn image_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.starts_with(b"GIF8") && data.len() >= 10 {
        let w = u16::from_le_bytes([data[6], data[7]]) as u32;
        let h = u16::from_le_bytes([data[8], data[9]]) as u32;
        return (w > 0 && h > 0).then_some((w, h));
    }
    image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
        .filter(|(w, h)| *w > 0 && *h > 0)
}

/// Kinds of image the writers embed natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
}

pub fn kind_of(data: &[u8]) -> Option<Kind> {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(Kind::Png)
    } else if data.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(Kind::Jpeg)
    } else if data.starts_with(b"GIF8") {
        Some(Kind::Gif)
    } else {
        None
    }
}

fn decode(data: &[u8]) -> Option<DynamicImage> {
    image::load_from_memory(data).ok()
}

/// PNG bytes and size of any decodable image (PNG is passed through).
pub fn to_png(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    if kind_of(data) == Some(Kind::Png) {
        let (w, h) = image_size(data)?;
        return Some((data.to_vec(), w, h));
    }
    let img = decode(data)?;
    let (w, h) = (img.width(), img.height());
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).ok()?;
    Some((out.into_inner(), w, h))
}

/// JPEG bytes and size of any decodable image (JPEG is passed through); transparency is flattened
/// onto white.
pub fn to_jpeg(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    if kind_of(data) == Some(Kind::Jpeg) {
        let (w, h) = image_size(data)?;
        return Some((data.to_vec(), w, h));
    }
    let img = decode(data)?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let mut rgb = image::RgbImage::new(w, h);
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = p[3] as u32;
        let blend = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        rgb.put_pixel(x, y, image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
    }
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(rgb)
        .write_to(&mut out, ImageFormat::Jpeg)
        .ok()?;
    Some((out.into_inner(), w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        if alpha {
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                w,
                h,
                image::Rgba([0, 0, 0, 0]),
            ))
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        } else {
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([9, 9, 9])))
                .write_to(&mut out, ImageFormat::Png)
                .unwrap();
        }
        out.into_inner()
    }

    fn gif(w: u32, h: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            w,
            h,
            image::Rgba([1, 2, 3, 255]),
        ))
        .write_to(&mut out, ImageFormat::Gif)
        .unwrap();
        out.into_inner()
    }

    #[test]
    fn sizes_of_png_jpeg_gif() {
        assert_eq!(image_size(&png(7, 5, false)), Some((7, 5)));
        let jpg = to_jpeg(&png(6, 4, false)).unwrap().0;
        assert_eq!(image_size(&jpg), Some((6, 4)));
        assert_eq!(image_size(&gif(11, 3)), Some((11, 3)));
    }

    #[test]
    fn sizes_reject_garbage() {
        assert_eq!(image_size(b""), None);
        assert_eq!(image_size(b"not an image at all"), None);
        assert_eq!(image_size(b"GIF8"), None);
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of(&png(1, 1, false)), Some(Kind::Png));
        assert_eq!(kind_of(&gif(1, 1)), Some(Kind::Gif));
        assert_eq!(kind_of(&[0xff, 0xd8, 0xff, 0xe0]), Some(Kind::Jpeg));
        assert_eq!(kind_of(b"RIFF"), None);
    }

    #[test]
    fn png_passthrough_and_conversion() {
        let p = png(3, 2, false);
        let (out, w, h) = to_png(&p).unwrap();
        assert_eq!((out.clone(), w, h), (p, 3, 2));
        let (conv, w, h) = to_png(&gif(5, 4)).unwrap();
        assert_eq!((w, h), (5, 4));
        assert_eq!(kind_of(&conv), Some(Kind::Png));
        assert!(to_png(b"junk").is_none());
    }

    #[test]
    fn jpeg_conversion_flattens_alpha_onto_white() {
        let (j, w, h) = to_jpeg(&png(8, 8, true)).unwrap();
        assert_eq!((w, h), (8, 8));
        assert_eq!(kind_of(&j), Some(Kind::Jpeg));
        let px = image::load_from_memory(&j).unwrap().to_rgb8();
        assert!(px.get_pixel(4, 4)[0] > 240, "transparent became white");
        assert!(to_jpeg(b"junk").is_none());
        let jj = j.clone();
        assert_eq!(to_jpeg(&jj).unwrap().0, j, "jpeg passes through");
    }
}
