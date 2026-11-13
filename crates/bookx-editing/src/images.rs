//! Image optimisation (E7): lossless PNG recompression, JPEG re-encoding, downscaling and conversion of formats
//! that are not EPUB core media types, with every reference updated.

use std::collections::HashMap;
use std::io::Cursor;

use anyhow::{anyhow, bail, Context, Result};
use bookx_core::book::OpenBook;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{DynamicImage, ImageEncoder, ImageFormat};
use serde::{Deserialize, Serialize};

use crate::markup::rename_paths;
use crate::opf_edit::edit_opf;

#[derive(Debug, Clone)]
pub struct ImageOptions {
    /// Re-encode PNG with the best compression and drop unused alpha / colour channels.
    pub optimize_png: bool,
    /// Re-encode JPEG at this quality (1..=100); `None` leaves JPEG data alone unless it has to be resized.
    pub jpeg_quality: Option<u8>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    /// Convert BMP and WebP to PNG (they are not EPUB core media types).
    pub convert_unsupported: bool,
    /// Convert non-animated GIF to PNG when that is smaller.
    pub gif_to_png: bool,
}

impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            optimize_png: true,
            jpeg_quality: None,
            max_width: None,
            max_height: None,
            convert_unsupported: true,
            gif_to_png: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageChange {
    pub path: String,
    /// Set when the extension changed.
    pub new_path: Option<String>,
    pub before: usize,
    pub after: usize,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizeReport {
    pub changes: Vec<ImageChange>,
    pub skipped: Vec<(String, String)>,
}

impl OptimizeReport {
    pub fn bytes_before(&self) -> usize {
        self.changes.iter().map(|c| c.before).sum()
    }

    pub fn bytes_after(&self) -> usize {
        self.changes.iter().map(|c| c.after).sum()
    }

    /// Positive when the book got smaller (conversions may make single files larger).
    pub fn bytes_saved(&self) -> i64 {
        self.bytes_before() as i64 - self.bytes_after() as i64
    }
}

pub fn guess_format(data: &[u8]) -> Option<ImageFormat> {
    image::guess_format(data).ok()
}

pub fn decode(data: &[u8]) -> Result<DynamicImage> {
    image::load_from_memory(data).context("cannot decode the image")
}

fn encode_png(img: &DynamicImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let enc = PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive);
    enc.write_image(
        img.as_bytes(),
        img.width(),
        img.height(),
        img.color().into(),
    )
    .context("PNG encoding failed")?;
    Ok(out)
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
    JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .write_image(
            rgb.as_bytes(),
            rgb.width(),
            rgb.height(),
            rgb.color().into(),
        )
        .context("JPEG encoding failed")?;
    Ok(out)
}

/// Drops an alpha channel that is fully opaque and colour channels that are all grey.
pub fn simplify_color(img: DynamicImage) -> DynamicImage {
    match img {
        DynamicImage::ImageRgba8(rgba) => {
            if rgba.pixels().all(|p| p[3] == 255) {
                simplify_color(DynamicImage::ImageRgb8(
                    DynamicImage::ImageRgba8(rgba).to_rgb8(),
                ))
            } else if rgba.pixels().all(|p| p[0] == p[1] && p[1] == p[2]) {
                DynamicImage::ImageLumaA8(DynamicImage::ImageRgba8(rgba).to_luma_alpha8())
            } else {
                DynamicImage::ImageRgba8(rgba)
            }
        }
        DynamicImage::ImageRgb8(rgb) => {
            if rgb.pixels().all(|p| p[0] == p[1] && p[1] == p[2]) {
                DynamicImage::ImageLuma8(DynamicImage::ImageRgb8(rgb).to_luma8())
            } else {
                DynamicImage::ImageRgb8(rgb)
            }
        }
        DynamicImage::ImageLumaA8(la) => {
            if la.pixels().all(|p| p[1] == 255) {
                DynamicImage::ImageLuma8(DynamicImage::ImageLumaA8(la).to_luma8())
            } else {
                DynamicImage::ImageLumaA8(la)
            }
        }
        other => other,
    }
}

/// Runs `oxipng` at its fast preset (no zopfli, no alpha tinkering, so every pixel stays identical).
fn oxipng_pass(data: &[u8]) -> Option<Vec<u8>> {
    let mut opts = oxipng::Options::from_preset(1);
    opts.strip = oxipng::StripChunks::Safe;
    opts.optimize_alpha = false;
    oxipng::optimize_from_memory(data, &opts).ok()
}

/// Lossless PNG recompression: same pixels, simplest colour type, then `oxipng` (filter and palette search,
/// libdeflate). The smallest of {colour-simplified re-encode, oxipng on the original, oxipng on the
/// re-encode} wins. Returns the original bytes when nothing is smaller.
pub fn optimize_png(data: &[u8]) -> Result<Vec<u8>> {
    if guess_format(data) != Some(ImageFormat::Png) {
        bail!("not a PNG image");
    }
    let img = decode(data)?;
    let reencoded = encode_png(&simplify_color(img))?;
    let mut best = data.to_vec();
    let mut candidates = vec![reencoded.clone()];
    candidates.extend(oxipng_pass(data));
    candidates.extend(oxipng_pass(&reencoded));
    for c in candidates {
        if c.len() < best.len() {
            best = c;
        }
    }
    Ok(best)
}

/// Re-encodes a JPEG at `quality`. Returns the original bytes unless the result is at least 3% smaller.
pub fn reencode_jpeg(data: &[u8], quality: u8) -> Result<Vec<u8>> {
    if guess_format(data) != Some(ImageFormat::Jpeg) {
        bail!("not a JPEG image");
    }
    let new = encode_jpeg(&decode(data)?, quality)?;
    // Lossy: only worth another generation of artefacts when it saves at least 3%.
    Ok(if new.len() * 100 < data.len() * 97 {
        new
    } else {
        data.to_vec()
    })
}

/// Dimensions that fit `w`x`h` within the limits, keeping the aspect ratio; `None` when no resizing is needed.
pub fn fit_within(w: u32, h: u32, max_w: Option<u32>, max_h: Option<u32>) -> Option<(u32, u32)> {
    let mut scale = 1.0f64;
    if let Some(mw) = max_w.filter(|m| *m > 0 && w > *m) {
        scale = scale.min(f64::from(mw) / f64::from(w));
    }
    if let Some(mh) = max_h.filter(|m| *m > 0 && h > *m) {
        scale = scale.min(f64::from(mh) / f64::from(h));
    }
    if scale >= 1.0 {
        return None;
    }
    Some((
        ((f64::from(w) * scale).round() as u32).max(1),
        ((f64::from(h) * scale).round() as u32).max(1),
    ))
}

/// Scales a PNG or JPEG down to fit within the limits and encodes it in the same format. `None` when it
/// already fits.
pub fn downscale(
    data: &[u8],
    max_w: Option<u32>,
    max_h: Option<u32>,
    jpeg_quality: u8,
) -> Result<Option<Vec<u8>>> {
    let fmt = guess_format(data).ok_or_else(|| anyhow!("unknown image format"))?;
    let img = decode(data)?;
    let Some((nw, nh)) = fit_within(img.width(), img.height(), max_w, max_h) else {
        return Ok(None);
    };
    let small = img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3);
    Ok(Some(match fmt {
        ImageFormat::Jpeg => encode_jpeg(&small, jpeg_quality)?,
        ImageFormat::Png => encode_png(&simplify_color(small))?,
        other => bail!("cannot downscale {other:?} images"),
    }))
}

/// Converts any readable still image to PNG.
pub fn convert_to_png(data: &[u8]) -> Result<Vec<u8>> {
    encode_png(&simplify_color(decode(data)?))
}

/// Converts any readable still image to JPEG (alpha is flattened onto black).
pub fn convert_to_jpeg(data: &[u8], quality: u8) -> Result<Vec<u8>> {
    encode_jpeg(&decode(data)?, quality)
}

pub fn is_animated_gif(data: &[u8]) -> bool {
    use image::AnimationDecoder;
    match image::codecs::gif::GifDecoder::new(Cursor::new(data)) {
        Ok(d) => d.into_frames().take(2).count() > 1,
        Err(_) => false,
    }
}

/// A thumbnail that fits `max_dim` x `max_dim`: `(bytes, extension)`. PNG when the image has transparency,
/// JPEG otherwise. Images already smaller are re-encoded unscaled.
pub fn make_thumbnail(data: &[u8], max_dim: u32) -> Result<(Vec<u8>, &'static str)> {
    if max_dim == 0 {
        bail!("thumbnail size must be positive");
    }
    let img = decode(data)?;
    let img = match fit_within(img.width(), img.height(), Some(max_dim), Some(max_dim)) {
        Some((w, h)) => img.resize_exact(w, h, image::imageops::FilterType::Triangle),
        None => img,
    };
    if img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p[3] != 255) {
        Ok((encode_png(&img)?, "png"))
    } else {
        Ok((encode_jpeg(&img, 85)?, "jpg"))
    }
}

/// The cover image of the book as a thumbnail, if it has a readable cover.
pub fn cover_thumbnail(book: &OpenBook, max_dim: u32) -> Option<(Vec<u8>, &'static str)> {
    make_thumbnail(book.cover_bytes()?, max_dim).ok()
}

fn core_media_type(fmt: ImageFormat) -> Option<&'static str> {
    match fmt {
        ImageFormat::Png => Some("image/png"),
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::Gif => Some("image/gif"),
        _ => None,
    }
}

fn unique_path(book: &OpenBook, wanted: String, taken: &HashMap<String, String>) -> String {
    let mut p = wanted.clone();
    let mut n = 1;
    while book.archive.contains(&p) || taken.values().any(|v| *v == p) {
        n += 1;
        let (stem, ext) = wanted.rsplit_once('.').unwrap_or((&wanted, ""));
        p = format!("{stem}-{n}.{ext}");
    }
    p
}

/// Optimises, resizes and converts the raster images of the book according to `opts`, updating manifest media
/// types and every reference when an extension changes. Files that get no smaller are left untouched (except
/// forced conversions and resizes).
pub fn optimize_images(book: &mut OpenBook, opts: &ImageOptions) -> Result<OptimizeReport> {
    let mut rep = OptimizeReport::default();
    let mut renames: HashMap<String, String> = HashMap::new();
    let mut media_updates: Vec<(String, &'static str)> = Vec::new();
    let items: Vec<(String, String)> = book
        .package
        .manifest
        .iter()
        .filter(|i| i.media_type.starts_with("image/") && i.media_type != "image/svg+xml")
        .map(|i| (i.id.clone(), book.package.item_path(i)))
        .collect();
    for (id, path) in items {
        let Some(data) = book.archive.get(&path).map(|d| d.to_vec()) else {
            continue;
        };
        let Some(fmt) = guess_format(&data) else {
            rep.skipped.push((path, "not a recognised image".into()));
            continue;
        };
        let mut cur = data.clone();
        let mut actions: Vec<String> = Vec::new();
        let mut target_fmt = fmt;
        let quality = opts.jpeg_quality.unwrap_or(85);

        let resize_wanted = opts.max_width.is_some() || opts.max_height.is_some();
        let convertible =
            matches!(fmt, ImageFormat::Bmp | ImageFormat::WebP) && opts.convert_unsupported;
        if matches!(fmt, ImageFormat::Gif) && is_animated_gif(&data) {
            rep.skipped.push((path, "animated GIF".into()));
            continue;
        }
        if core_media_type(fmt).is_none() && !convertible {
            rep.skipped.push((path, format!("{fmt:?} left as it is")));
            continue;
        }
        if convertible {
            cur = convert_to_png(&cur)?;
            target_fmt = ImageFormat::Png;
            actions.push(format!("converted {fmt:?} to PNG"));
        }
        if resize_wanted && matches!(target_fmt, ImageFormat::Png | ImageFormat::Jpeg) {
            if let Some(s) = downscale(&cur, opts.max_width, opts.max_height, quality)? {
                let (w, h) = image_dimensions(&s);
                actions.push(format!("scaled down to {w}x{h}"));
                cur = s;
            }
        } else if resize_wanted && target_fmt == ImageFormat::Gif {
            if let Ok(img) = decode(&cur) {
                if fit_within(img.width(), img.height(), opts.max_width, opts.max_height).is_some()
                {
                    let s = downscale_to_png(&img, opts.max_width, opts.max_height)?;
                    actions.push("scaled down and converted GIF to PNG".into());
                    cur = s;
                    target_fmt = ImageFormat::Png;
                }
            }
        }
        match target_fmt {
            ImageFormat::Png if opts.optimize_png => {
                let n = optimize_png(&cur)?;
                if n.len() < cur.len() {
                    actions.push(format!(
                        "recompressed PNG ({} -> {} bytes)",
                        cur.len(),
                        n.len()
                    ));
                    cur = n;
                }
            }
            ImageFormat::Jpeg if opts.jpeg_quality.is_some() => {
                let n = reencode_jpeg(&cur, quality)?;
                if n != cur {
                    actions.push(format!("re-encoded JPEG at quality {quality}"));
                    cur = n;
                }
            }
            ImageFormat::Gif if opts.gif_to_png => {
                let n = convert_to_png(&cur)?;
                if n.len() < cur.len() {
                    actions.push("converted GIF to PNG".into());
                    cur = n;
                    target_fmt = ImageFormat::Png;
                }
            }
            _ => {}
        }
        if actions.is_empty() {
            continue;
        }
        let mut new_path = None;
        if target_fmt != fmt {
            let ext = if target_fmt == ImageFormat::Png {
                "png"
            } else {
                "jpg"
            };
            let stem = path.rsplit_once('.').map(|(s, _)| s).unwrap_or(&path);
            let np = unique_path(book, format!("{stem}.{ext}"), &renames);
            renames.insert(path.clone(), np.clone());
            media_updates.push((
                id.clone(),
                core_media_type(target_fmt).unwrap_or("image/png"),
            ));
            new_path = Some(np);
        }
        rep.changes.push(ImageChange {
            path: path.clone(),
            new_path,
            before: data.len(),
            after: cur.len(),
            actions,
        });
        book.archive.set(&path, cur);
    }
    if !renames.is_empty() {
        rename_paths(&mut book.archive, &renames);
        book.reload_package()?;
        edit_opf(book, |t| {
            let mut out = t.to_string();
            for (id, media) in &media_updates {
                out = set_item_media_type(&out, id, media);
            }
            Ok(out)
        })?;
    }
    Ok(rep)
}

fn downscale_to_png(img: &DynamicImage, mw: Option<u32>, mh: Option<u32>) -> Result<Vec<u8>> {
    let (w, h) =
        fit_within(img.width(), img.height(), mw, mh).unwrap_or((img.width(), img.height()));
    encode_png(&simplify_color(img.resize_exact(
        w,
        h,
        image::imageops::FilterType::Lanczos3,
    )))
}

fn set_item_media_type(opf: &str, id: &str, media: &str) -> String {
    use crate::markup::*;
    let mut out = String::with_capacity(opf.len());
    let mut last = 0;
    for ev in scan(opf) {
        if let Event::Open { name, range, .. } = ev {
            if name == "item" && tag_attr(&opf[range.clone()], "id").as_deref() == Some(id) {
                out.push_str(&opf[last..range.start]);
                out.push_str(&set_tag_attr(&opf[range.clone()], "media-type", media));
                last = range.end;
            }
        }
    }
    out.push_str(&opf[last..]);
    out
}

/// Width and height of an encoded image (0, 0 if unreadable).
pub fn image_dimensions(data: &[u8]) -> (u32, u32) {
    crate::reports::image_info(data)
        .map(|(_, w, h)| (w, h))
        .unwrap_or((0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;
    use bookx_core::check::{check_book, Severity};
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    fn encode(img: &DynamicImage, fmt: ImageFormat) -> Vec<u8> {
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), fmt).unwrap();
        out
    }

    fn opaque_rgba(w: u32, h: u32) -> Vec<u8> {
        encode(
            &DynamicImage::ImageRgba8(RgbaImage::from_fn(w, h, |x, y| {
                Rgba([(x % 200) as u8, (y % 100) as u8, 80, 255])
            })),
            ImageFormat::Png,
        )
    }

    fn photo(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            Rgb([
                ((x * 3 + y) % 256) as u8,
                ((x ^ y) % 256) as u8,
                ((x * y / 7) % 256) as u8,
            ])
        }))
    }

    #[test]
    fn png_recompression_is_lossless_and_drops_opaque_alpha() {
        let data = opaque_rgba(64, 48);
        let out = optimize_png(&data).unwrap();
        assert!(out.len() < data.len(), "{} -> {}", data.len(), out.len());
        let a = decode(&data).unwrap().to_rgba8();
        let b = decode(&out).unwrap().to_rgba8();
        assert_eq!(a.as_raw(), b.as_raw(), "pixels identical");
        assert_eq!(guess_format(&out), Some(ImageFormat::Png));
        assert_eq!(decode(&out).unwrap().color(), image::ColorType::Rgb8);
        // translucent pixels keep their alpha
        let translucent = encode(
            &DynamicImage::ImageRgba8(RgbaImage::from_fn(8, 8, |x, _| {
                Rgba([x as u8 * 30, 0, 9, if x == 0 { 10 } else { 255 }])
            })),
            ImageFormat::Png,
        );
        let t = optimize_png(&translucent).unwrap();
        assert_eq!(
            decode(&t).unwrap().to_rgba8().as_raw(),
            decode(&translucent).unwrap().to_rgba8().as_raw()
        );
        // grey RGB becomes luma
        let grey = encode(
            &DynamicImage::ImageRgb8(RgbImage::from_fn(32, 32, |x, _| Rgb([x as u8; 3]))),
            ImageFormat::Png,
        );
        assert_eq!(
            decode(&optimize_png(&grey).unwrap()).unwrap().color(),
            image::ColorType::L8
        );
        assert_eq!(
            optimize_png(b"nope").unwrap_err().to_string(),
            "not a PNG image"
        );
    }

    #[test]
    fn oxipng_beats_the_plain_encoder_on_few_colour_images() {
        // 4 colours in a pattern: oxipng turns it into a 2-bit palette image, the plain encoder keeps RGB
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(96, 96, |x, y| {
            let c = ((x / 8 + y / 8) % 4) as u8;
            Rgb([c * 80, 255 - c * 60, 30 + c * 10])
        }));
        let data = encode(&img, ImageFormat::Png);
        let plain = encode_png(&simplify_color(decode(&data).unwrap())).unwrap();
        let out = optimize_png(&data).unwrap();
        assert!(out.len() < plain.len(), "{} !< {}", out.len(), plain.len());
        assert!(out.len() < data.len());
        assert_eq!(
            decode(&out).unwrap().to_rgba8().as_raw(),
            img.to_rgba8().as_raw()
        );
        assert_eq!(guess_format(&out), Some(ImageFormat::Png));
    }

    #[test]
    fn oxipng_keeps_translucent_pixels_exact() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(40, 40, |x, y| {
            Rgba([
                (x * 6) as u8,
                (y * 6) as u8,
                7,
                if (x + y) % 5 == 0 { 0 } else { 200 },
            ])
        }));
        let data = encode(&img, ImageFormat::Png);
        let out = optimize_png(&data).unwrap();
        assert_eq!(
            decode(&out).unwrap().to_rgba8().as_raw(),
            img.to_rgba8().as_raw(),
            "even RGB under alpha 0 is kept"
        );
    }

    #[test]
    fn truncated_png_is_an_error_not_a_panic() {
        let data = opaque_rgba(30, 30);
        assert!(optimize_png(&data[..data.len() / 2]).is_err());
        assert!(optimize_png(&data[..8]).is_err());
    }

    #[test]
    fn colour_simplification_variants() {
        let la = DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_pixel(
            4,
            4,
            image::LumaA([7, 255]),
        ));
        assert_eq!(simplify_color(la).color(), image::ColorType::L8);
        let la_t = DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_pixel(
            4,
            4,
            image::LumaA([7, 100]),
        ));
        assert_eq!(simplify_color(la_t).color(), image::ColorType::La8);
        let grey_t = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 4, Rgba([9, 9, 9, 50])));
        assert_eq!(simplify_color(grey_t).color(), image::ColorType::La8);
        let colour_t = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 4, Rgba([9, 8, 7, 50])));
        assert_eq!(simplify_color(colour_t).color(), image::ColorType::Rgba8);
        let rgb16 =
            DynamicImage::ImageRgb16(image::ImageBuffer::from_pixel(2, 2, Rgb([1u16, 2, 3])));
        assert_eq!(simplify_color(rgb16).color(), image::ColorType::Rgb16);
    }

    #[test]
    fn already_optimal_png_is_returned_unchanged() {
        let once = optimize_png(&opaque_rgba(40, 40)).unwrap();
        assert_eq!(optimize_png(&once).unwrap(), once);
    }

    #[test]
    fn jpeg_quality_reduction() {
        let hi = encode_jpeg(&photo(200, 150), 95).unwrap();
        let lo = reencode_jpeg(&hi, 50).unwrap();
        assert!(lo.len() < hi.len());
        assert_eq!(image_dimensions(&lo), (200, 150));
        // re-encoding at a higher quality cannot shrink: original kept
        assert_eq!(reencode_jpeg(&lo, 98).unwrap(), lo);
        assert_eq!(
            reencode_jpeg(&opaque_rgba(4, 4), 50)
                .unwrap_err()
                .to_string(),
            "not a JPEG image"
        );
    }

    #[test]
    fn fitting_and_downscale() {
        assert_eq!(fit_within(1000, 500, Some(500), None), Some((500, 250)));
        assert_eq!(
            fit_within(1000, 500, Some(800), Some(100)),
            Some((200, 100))
        );
        assert_eq!(fit_within(100, 100, Some(200), Some(200)), None);
        assert_eq!(fit_within(100, 100, None, None), None);
        assert_eq!(fit_within(100, 100, Some(0), Some(0)), None);
        assert_eq!(fit_within(1000, 1, Some(10), None), Some((10, 1)));
        let png = opaque_rgba(120, 80);
        let s = downscale(&png, Some(60), None, 85).unwrap().unwrap();
        assert_eq!(image_dimensions(&s), (60, 40));
        assert_eq!(guess_format(&s), Some(ImageFormat::Png));
        assert!(downscale(&png, Some(500), None, 85).unwrap().is_none());
        let jpg = encode_jpeg(&photo(100, 100), 90).unwrap();
        let s = downscale(&jpg, None, Some(25), 70).unwrap().unwrap();
        assert_eq!(image_dimensions(&s), (25, 25));
        assert_eq!(guess_format(&s), Some(ImageFormat::Jpeg));
        let bmp = encode(&photo(10, 10), ImageFormat::Bmp);
        assert!(downscale(&bmp, Some(5), None, 85)
            .unwrap_err()
            .to_string()
            .contains("cannot downscale"));
        assert!(downscale(b"xx", Some(5), None, 85).is_err());
    }

    #[test]
    fn conversions() {
        let bmp = encode(&photo(16, 16), ImageFormat::Bmp);
        let png = convert_to_png(&bmp).unwrap();
        assert_eq!(guess_format(&png), Some(ImageFormat::Png));
        assert_eq!(
            decode(&png).unwrap().to_rgb8().as_raw(),
            photo(16, 16).to_rgb8().as_raw()
        );
        let jpg = convert_to_jpeg(&png, 80).unwrap();
        assert_eq!(guess_format(&jpg), Some(ImageFormat::Jpeg));
        assert!(convert_to_png(b"junk").is_err());
    }

    #[test]
    fn animated_gif_detection() {
        use image::codecs::gif::GifEncoder;
        use image::Frame;
        let frame = |c: u8| Frame::new(RgbaImage::from_pixel(4, 4, Rgba([c, 0, 0, 255])));
        let mut one = Vec::new();
        GifEncoder::new(&mut one)
            .encode_frames(vec![frame(1)])
            .unwrap();
        let mut two = Vec::new();
        GifEncoder::new(&mut two)
            .encode_frames(vec![frame(1), frame(200)])
            .unwrap();
        assert!(!is_animated_gif(&one));
        assert!(is_animated_gif(&two));
        assert!(!is_animated_gif(b"nope"));
    }

    #[test]
    fn thumbnails() {
        let (t, ext) = make_thumbnail(&opaque_rgba(400, 200), 100).unwrap();
        assert_eq!(ext, "jpg");
        assert_eq!(image_dimensions(&t), (100, 50));
        let translucent = encode(
            &DynamicImage::ImageRgba8(RgbaImage::from_pixel(20, 10, Rgba([1, 2, 3, 100]))),
            ImageFormat::Png,
        );
        let (t, ext) = make_thumbnail(&translucent, 100).unwrap();
        assert_eq!(ext, "png");
        assert_eq!(image_dimensions(&t), (20, 10), "never enlarged");
        assert!(make_thumbnail(&translucent, 0).is_err());
        assert!(make_thumbnail(b"x", 10).is_err());
        let mut b = make_book(&["<p>x</p>"]);
        assert!(cover_thumbnail(&b, 50).is_none());
        b.set_cover(opaque_rgba(300, 300), "png").unwrap();
        let (c, _) = cover_thumbnail(&b, 50).unwrap();
        assert_eq!(image_dimensions(&c), (50, 50));
    }

    fn image_book() -> OpenBook {
        let mut b = make_book(&[
            "<p><img src=\"images/a.png\" alt=\"\"/><img src=\"images/b.jpg\" alt=\"\"/><img src=\"images/c.bmp\" alt=\"\"/><img src=\"images/d.gif\" alt=\"\"/></p>",
        ]);
        b.archive.set("OEBPS/images/a.png", opaque_rgba(120, 90));
        b.archive.set(
            "OEBPS/images/b.jpg",
            encode_jpeg(&photo(300, 200), 95).unwrap(),
        );
        b.archive.set(
            "OEBPS/images/c.bmp",
            encode(&photo(30, 30), ImageFormat::Bmp),
        );
        let mut gif = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut gif)
            .encode_frames(vec![image::Frame::new(RgbaImage::from_pixel(
                8,
                8,
                Rgba([9, 9, 9, 255]),
            ))])
            .unwrap();
        b.archive.set("OEBPS/images/d.gif", gif);
        b.archive.set_string(
            "OEBPS/style.css",
            "body { background: url(images/c.bmp) }\n",
        );
        let pkg = b.package.clone();
        for (id, f, m) in [
            ("a", "a.png", "image/png"),
            ("b", "b.jpg", "image/jpeg"),
            ("c", "c.bmp", "image/bmp"),
            ("d", "d.gif", "image/gif"),
        ] {
            pkg.add_manifest_item(&mut b.archive, id, &format!("OEBPS/images/{f}"), m, "")
                .unwrap();
        }
        b.reload_package().unwrap();
        b
    }

    #[test]
    fn book_optimization_updates_references() {
        let mut b = image_book();
        let rep = optimize_images(
            &mut b,
            &ImageOptions {
                jpeg_quality: Some(50),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rep.changes.len(), 3, "{rep:#?}");
        let c = rep
            .changes
            .iter()
            .find(|c| c.path.ends_with("c.bmp"))
            .unwrap();
        assert_eq!(c.new_path.as_deref(), Some("OEBPS/images/c.png"));
        assert!(rep.bytes_saved() > 0);
        assert!(!b.archive.contains("OEBPS/images/c.bmp"));
        assert_eq!(
            guess_format(b.archive.get("OEBPS/images/c.png").unwrap()),
            Some(ImageFormat::Png)
        );
        assert_eq!(b.package.item_by_id("c").unwrap().media_type, "image/png");
        assert_eq!(b.package.item_by_id("c").unwrap().href, "images/c.png");
        let ch = b.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(ch.contains("src=\"images/c.png\"") && !ch.contains("c.bmp"));
        assert!(b
            .archive
            .get_string("OEBPS/style.css")
            .unwrap()
            .contains("url(images/c.png)"));
        let jpg = b.archive.get("OEBPS/images/b.jpg").unwrap();
        assert_eq!(image_dimensions(jpg), (300, 200));
        assert!(jpg.len() < encode_jpeg(&photo(300, 200), 95).unwrap().len());
        assert!(rep.skipped.is_empty());
        assert!(
            check_book(&b.archive)
                .iter()
                .all(|i| i.severity != Severity::Error),
            "{:?}",
            check_book(&b.archive)
        );
        // second run: nothing left to do
        let again = optimize_images(
            &mut b,
            &ImageOptions {
                jpeg_quality: Some(50),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(again.changes.is_empty(), "{again:#?}");
    }

    #[test]
    fn resize_and_gif_options() {
        let mut b = image_book();
        let rep = optimize_images(
            &mut b,
            &ImageOptions {
                optimize_png: false,
                max_width: Some(100),
                gif_to_png: true,
                convert_unsupported: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            image_dimensions(b.archive.get("OEBPS/images/a.png").unwrap()),
            (100, 75)
        );
        assert_eq!(
            image_dimensions(b.archive.get("OEBPS/images/b.jpg").unwrap()),
            (100, 67)
        );
        assert!(
            b.archive.contains("OEBPS/images/c.bmp"),
            "conversion disabled"
        );
        assert!(rep
            .skipped
            .iter()
            .any(|(p, why)| p.ends_with("c.bmp") && why.contains("left as it is")));
        assert!(rep.changes.iter().any(|c| c
            .actions
            .iter()
            .any(|a| a.starts_with("scaled down to 100x75"))));
        // a tiny GIF is not smaller as PNG, so it stays
        assert!(b.archive.contains("OEBPS/images/d.gif"));
    }

    #[test]
    fn name_collisions_and_garbage() {
        let mut b = image_book();
        b.archive.set("OEBPS/images/c.png", vec![1, 2, 3]);
        b.archive
            .set("OEBPS/images/junk.png", b"not an image".to_vec());
        let pkg = b.package.clone();
        pkg.add_manifest_item(
            &mut b.archive,
            "junk",
            "OEBPS/images/junk.png",
            "image/png",
            "",
        )
        .unwrap();
        b.reload_package().unwrap();
        let rep = optimize_images(&mut b, &ImageOptions::default()).unwrap();
        let c = rep
            .changes
            .iter()
            .find(|c| c.path.ends_with("c.bmp"))
            .unwrap();
        assert_eq!(c.new_path.as_deref(), Some("OEBPS/images/c-2.png"));
        assert_eq!(
            b.archive.get("OEBPS/images/c.png").unwrap(),
            &[1, 2, 3],
            "existing file untouched"
        );
        assert!(rep
            .skipped
            .iter()
            .any(|(p, why)| p.ends_with("junk.png") && why == "not a recognised image"));
    }

    #[test]
    fn animated_gif_skipped_in_book() {
        use image::codecs::gif::GifEncoder;
        let mut b = image_book();
        let mut two = Vec::new();
        GifEncoder::new(&mut two)
            .encode_frames(vec![
                image::Frame::new(RgbaImage::from_pixel(8, 8, Rgba([1, 0, 0, 255]))),
                image::Frame::new(RgbaImage::from_pixel(8, 8, Rgba([200, 0, 0, 255]))),
            ])
            .unwrap();
        b.archive.set("OEBPS/images/d.gif", two.clone());
        let rep = optimize_images(
            &mut b,
            &ImageOptions {
                gif_to_png: true,
                max_width: Some(4),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(rep
            .skipped
            .iter()
            .any(|(p, why)| p.ends_with("d.gif") && why == "animated GIF"));
        assert_eq!(b.archive.get("OEBPS/images/d.gif").unwrap(), two.as_slice());
    }

    #[test]
    fn report_arithmetic_and_helpers() {
        let r = OptimizeReport {
            changes: vec![
                ImageChange {
                    path: "a".into(),
                    new_path: None,
                    before: 100,
                    after: 60,
                    actions: vec![],
                },
                ImageChange {
                    path: "b".into(),
                    new_path: None,
                    before: 50,
                    after: 70,
                    actions: vec![],
                },
            ],
            skipped: vec![],
        };
        assert_eq!(r.bytes_before(), 150);
        assert_eq!(r.bytes_after(), 130);
        assert_eq!(r.bytes_saved(), 20);
        assert_eq!(image_dimensions(b"x"), (0, 0));
        assert_eq!(guess_format(b"zz"), None);
    }
}
