//! Cover tools (L13): generate a cover from title/author/series, crop to an aspect ratio, resize,
//! find the dominant colour and detect blank images. JPEG and PNG only.
//!
//! Generated covers are deterministic: the same text and size give byte-identical PNGs. Text is
//! drawn with a built-in 5×7 bitmap font (written for this project, no external font files);
//! characters it lacks are drawn as an empty box.

use std::io::Cursor;

use anyhow::{bail, Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageEncoder, ImageFormat, ImageReader, Limits, Rgb, RgbImage};

use crate::text::fold;

/// Pixel size limit for decoded images (guards against decompression bombs).
pub const MAX_DIMENSION: u32 = 20_000;

// ---------------------------------------------------------------------------------------------
// 5x7 font: each glyph is 7 rows, the low 5 bits of a row are the pixels (bit 4 = left).
// ---------------------------------------------------------------------------------------------

const TOFU: [u8; 7] = [
    0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111,
];

/// Glyph rows for an upper-case ASCII character, digit or punctuation mark.
pub fn glyph(c: char) -> [u8; 7] {
    match c {
        ' ' => [0; 7],
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        ',' => [0, 0, 0, 0, 0b01100, 0b00100, 0b01000],
        ':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
        ';' => [0, 0b01100, 0b01100, 0, 0b01100, 0b00100, 0b01000],
        '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100],
        '?' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        '\'' => [0b00100, 0b00100, 0b01000, 0, 0, 0, 0],
        '"' => [0b01010, 0b01010, 0, 0, 0, 0, 0],
        '(' => [
            0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
        ],
        ')' => [
            0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
        ],
        '/' => [
            0b00001, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b10000,
        ],
        '&' => [
            0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101,
        ],
        '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        '#' => [
            0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
        ],
        '*' => [0, 0b10101, 0b01110, 0b11111, 0b01110, 0b10101, 0],
        _ => TOFU,
    }
}

/// Characters the font can draw (after upper-casing), for tests and callers that want to check.
pub const FONT_CHARS: &str = " ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,:;!?-_'\"()/&+#*";

/// Text prepared for the bitmap font: accents folded, upper-cased; characters outside the font
/// that cannot be folded become `\u{FFFD}` (drawn as a box).
pub fn font_text(s: &str) -> String {
    fold(s)
        .chars()
        .map(|c| {
            let u = c.to_ascii_uppercase();
            if FONT_CHARS.contains(u) {
                u
            } else if c.is_whitespace() || c.is_control() {
                ' '
            } else {
                '\u{FFFD}'
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// colours
// ---------------------------------------------------------------------------------------------

fn lin(c: u8) -> f64 {
    let v = c as f64 / 255.0;
    if v <= 0.03928 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance, 0..=1.
pub fn luminance(c: [u8; 3]) -> f64 {
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG contrast ratio, 1..=21.
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// White or black, whichever reads better on `bg`.
pub fn pick_text_color(bg: [u8; 3]) -> [u8; 3] {
    const LIGHT: [u8; 3] = [0xFF, 0xFF, 0xFF];
    const DARK: [u8; 3] = [0x00, 0x00, 0x00];
    if contrast_ratio(bg, LIGHT) >= contrast_ratio(bg, DARK) {
        LIGHT
    } else {
        DARK
    }
}

fn hsl(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    ]
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The cover palette derived from the book's text: `(background, accent)`. Deterministic.
pub fn palette(title: &str, author: &str, series: &str) -> ([u8; 3], [u8; 3]) {
    let h = fnv1a(&format!("{title}\u{0}{author}\u{0}{series}"));
    let hue = (h % 360) as f64;
    let sat = 0.40 + ((h >> 16) % 25) as f64 / 100.0;
    let light = 0.20 + ((h >> 32) % 12) as f64 / 100.0;
    let bg = hsl(hue, sat, light);
    let mut l = 0.62;
    let mut accent = hsl(hue + 35.0, (sat + 0.2).min(0.9), l);
    while contrast_ratio(bg, accent) < 3.5 && l < 0.95 {
        l += 0.04;
        accent = hsl(hue + 35.0, (sat + 0.2).min(0.9), l);
    }
    (bg, accent)
}

// ---------------------------------------------------------------------------------------------
// text layout
// ---------------------------------------------------------------------------------------------

/// Greedy word wrap for `max_chars` columns; words longer than a line are split.
pub fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let max = max_chars.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur: Vec<char> = Vec::new();
    for word in text.split_whitespace() {
        let w: Vec<char> = word.chars().collect();
        let sep = usize::from(!cur.is_empty());
        if cur.len() + sep + w.len() <= max {
            if sep == 1 {
                cur.push(' ');
            }
            cur.extend(&w);
            continue;
        }
        if !cur.is_empty() {
            lines.push(cur.iter().collect());
            cur.clear();
        }
        for chunk in w.chunks(max) {
            if chunk.len() == max {
                lines.push(chunk.iter().collect());
            } else {
                cur.extend(chunk);
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur.iter().collect());
    }
    lines
}

fn draw_glyph(img: &mut RgbImage, x: i64, y: i64, ch: char, scale: u32, color: Rgb<u8>) {
    let rows = if ch == '\u{FFFD}' { TOFU } else { glyph(ch) };
    for (ry, bits) in rows.iter().enumerate() {
        for rx in 0..5u32 {
            if bits & (1 << (4 - rx)) != 0 {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = x + (rx * scale + dx) as i64;
                        let py = y + (ry as u32 * scale + dy) as i64;
                        if px >= 0
                            && py >= 0
                            && (px as u32) < img.width()
                            && (py as u32) < img.height()
                        {
                            img.put_pixel(px as u32, py as u32, color);
                        }
                    }
                }
            }
        }
    }
}

/// Draws centred lines starting at `y`; returns the y below the block.
fn draw_lines(img: &mut RgbImage, lines: &[String], y: u32, scale: u32, color: Rgb<u8>) -> u32 {
    let cell = 6 * scale;
    let mut cy = y;
    for line in lines {
        let w = line.chars().count() as u32 * cell - scale;
        let x0 = (img.width().saturating_sub(w)) / 2;
        for (i, ch) in line.chars().enumerate() {
            draw_glyph(
                img,
                (x0 + i as u32 * cell) as i64,
                cy as i64,
                ch,
                scale,
                color,
            );
        }
        cy += 9 * scale;
    }
    cy
}

/// Largest scale (≥1) at which `text` wraps into at most `max_lines` lines of `width` pixels and
/// fits `max_height`; returns the scale and the lines (truncated with `...` as a last resort).
fn fit_text(
    text: &str,
    width: u32,
    max_height: u32,
    max_lines: usize,
    start_scale: u32,
) -> (u32, Vec<String>) {
    for scale in (1..=start_scale.max(1)).rev() {
        let cols = (width / (6 * scale)).max(1) as usize;
        let lines = wrap_text(text, cols);
        if lines.len() <= max_lines && lines.len() as u32 * 9 * scale <= max_height {
            return (scale, lines);
        }
    }
    let cols = (width / 6).max(4) as usize;
    let mut lines = wrap_text(text, cols);
    lines.truncate(max_lines.max(1));
    if let Some(last) = lines.last_mut() {
        let keep = last.chars().count().saturating_sub(3);
        let mut s: String = last.chars().take(keep).collect();
        s.push_str("...");
        *last = s;
    }
    (1, lines)
}

fn encode_png(img: &RgbImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .context("encoding PNG")?;
    Ok(out)
}

/// Draws a cover as PNG. `author` and `series` may be empty. Size must be at least 120×160.
pub fn generate_cover(
    title: &str,
    author: &str,
    series: &str,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    if !(120..=MAX_DIMENSION).contains(&width) || !(160..=MAX_DIMENSION).contains(&height) {
        bail!("cover size {width}x{height} is out of range (min 120x160)");
    }
    let (bg, accent) = palette(title, author, series);
    let text_color = Rgb(pick_text_color(bg));
    let mut img = RgbImage::from_pixel(width, height, Rgb(bg));

    // accent bars
    let bar = (height / 40).max(2);
    for y in 0..bar {
        for x in 0..width {
            img.put_pixel(x, y, Rgb(accent));
            img.put_pixel(x, height - 1 - y, Rgb(accent));
        }
    }
    let margin = width / 10;
    let inner = width - 2 * margin;
    let unit = (width / 100).max(1);

    let mut y = height / 14;
    let series = font_text(series);
    if !series.trim().is_empty() {
        let (s, lines) = fit_text(series.trim(), inner, height / 10, 2, (2 * unit).max(1));
        y = draw_lines(&mut img, &lines, y, s, Rgb(accent)) + 3 * s;
    }

    let title_txt = font_text(if title.trim().is_empty() {
        "Untitled"
    } else {
        title.trim()
    });
    let title_top = y.max(height / 6);
    let (ts, tlines) = fit_text(&title_txt, inner, height * 55 / 100, 8, (5 * unit).max(1));
    let title_end = draw_lines(&mut img, &tlines, title_top, ts, text_color);

    // divider under the title
    let div_y = title_end + 3 * ts;
    if div_y + 2 < height {
        let w = inner / 3;
        for x in (width - w) / 2..(width + w) / 2 {
            for dy in 0..(unit.max(2)) {
                if div_y + dy < height {
                    img.put_pixel(x, div_y + dy, Rgb(accent));
                }
            }
        }
    }

    let author_txt = font_text(author);
    if !author_txt.trim().is_empty() {
        let (s, lines) = fit_text(author_txt.trim(), inner, height / 6, 3, (3 * unit).max(1));
        let block = lines.len() as u32 * 9 * s;
        let top = (height - bar - height / 12)
            .saturating_sub(block)
            .max(div_y + 6 * ts);
        draw_lines(&mut img, &lines, top, s, text_color);
    }
    encode_png(&img)
}

// ---------------------------------------------------------------------------------------------
// image tools
// ---------------------------------------------------------------------------------------------

/// Decodes a JPEG or PNG (other formats are refused) with size limits.
pub fn decode(bytes: &[u8]) -> Result<(DynamicImage, ImageFormat)> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("reading image header")?;
    let format = match reader.format() {
        Some(f @ (ImageFormat::Png | ImageFormat::Jpeg)) => f,
        Some(other) => bail!("unsupported image format {other:?} (only JPEG and PNG)"),
        None => bail!("not a recognised image"),
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().context("decoding image")?;
    Ok((img, format))
}

/// Encodes in the given format (PNG, or JPEG at quality 90).
pub fn encode(img: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    match format {
        ImageFormat::Jpeg => {
            let rgb = img.to_rgb8();
            JpegEncoder::new_with_quality(&mut out, 90)
                .encode_image(&rgb)
                .context("encoding JPEG")?;
        }
        _ => {
            let rgba = img.to_rgba8();
            PngEncoder::new(&mut out)
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .context("encoding PNG")?;
        }
    }
    Ok(out)
}

/// Width, height and format of a JPEG/PNG without fully decoding it.
pub fn info(bytes: &[u8]) -> Result<(u32, u32, ImageFormat)> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("reading image header")?;
    let format = match reader.format() {
        Some(f @ (ImageFormat::Png | ImageFormat::Jpeg)) => f,
        _ => bail!("not a JPEG or PNG image"),
    };
    let (w, h) = reader.into_dimensions().context("reading image size")?;
    Ok((w, h, format))
}

/// Centre-crops to the aspect ratio `aw:ah` (e.g. 2:3 for book covers). The format is kept.
pub fn crop_to_aspect(bytes: &[u8], aw: u32, ah: u32) -> Result<Vec<u8>> {
    if aw == 0 || ah == 0 {
        bail!("aspect ratio must be positive");
    }
    let (img, format) = decode(bytes)?;
    let (w, h) = (img.width() as u64, img.height() as u64);
    // compare w/h with aw/ah without floating point
    let (nw, nh) = if w * ah as u64 > h * aw as u64 {
        (((h * aw as u64) / ah as u64).max(1), h)
    } else {
        (w, ((w * ah as u64) / aw as u64).max(1))
    };
    if nw == w && nh == h {
        return Ok(bytes.to_vec());
    }
    let x = (w - nw) / 2;
    let y = (h - nh) / 2;
    encode(
        &img.crop_imm(x as u32, y as u32, nw as u32, nh as u32),
        format,
    )
}

/// Shrinks to fit `max_w × max_h` keeping the aspect ratio; images that already fit are returned
/// unchanged (never upscaled).
pub fn resize_max(bytes: &[u8], max_w: u32, max_h: u32) -> Result<Vec<u8>> {
    if max_w == 0 || max_h == 0 {
        bail!("maximum size must be positive");
    }
    let (img, format) = decode(bytes)?;
    if img.width() <= max_w && img.height() <= max_h {
        return Ok(bytes.to_vec());
    }
    encode(&img.resize(max_w, max_h, FilterType::Lanczos3), format)
}

/// The most common colour (4 bits per channel buckets, mean of the winning bucket). Transparent
/// pixels are ignored; a fully transparent image gives an error.
pub fn dominant_color(bytes: &[u8]) -> Result<[u8; 3]> {
    let (img, _) = decode(bytes)?;
    let small = if img.width() > 96 || img.height() > 96 {
        img.thumbnail(96, 96)
    } else {
        img
    };
    let rgba = small.to_rgba8();
    let mut buckets: std::collections::HashMap<(u8, u8, u8), (u64, [u64; 3])> =
        std::collections::HashMap::new();
    for p in rgba.pixels() {
        if p[3] < 128 {
            continue;
        }
        let key = (p[0] >> 4, p[1] >> 4, p[2] >> 4);
        let e = buckets.entry(key).or_insert((0, [0; 3]));
        e.0 += 1;
        for i in 0..3 {
            e.1[i] += p[i] as u64;
        }
    }
    let Some((_, (n, sum))) = buckets
        .into_iter()
        .max_by(|a, b| a.1 .0.cmp(&b.1 .0).then_with(|| b.0.cmp(&a.0)))
    else {
        bail!("image has no opaque pixels");
    };
    Ok([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8])
}

/// An image is blank when (almost) every pixel has the same brightness: fewer than 1% of the
/// pixels differ from the median by more than 20 levels. Fully transparent images are blank.
pub fn is_blank(bytes: &[u8]) -> Result<bool> {
    let (img, _) = decode(bytes)?;
    let small = img.thumbnail(128, 128).to_rgba8();
    let mut lum: Vec<u8> = small
        .pixels()
        .filter(|p| p[3] >= 128)
        .map(|p| ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as u8)
        .collect();
    if lum.is_empty() {
        return Ok(true);
    }
    lum.sort_unstable();
    let median = lum[lum.len() / 2] as i32;
    let off = lum
        .iter()
        .filter(|l| (**l as i32 - median).abs() > 20)
        .count();
    Ok((off as f64) / (lum.len() as f64) < 0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 3], fmt: ImageFormat) -> Vec<u8> {
        encode(
            &DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, Rgb(c))),
            fmt,
        )
        .unwrap()
    }

    fn halves(w: u32, h: u32) -> Vec<u8> {
        // left half red, right half blue
        let mut img = RgbImage::new(w, h);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = if x < w / 2 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            };
        }
        encode(&DynamicImage::ImageRgb8(img), ImageFormat::Png).unwrap()
    }

    fn dims(bytes: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory(bytes).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn font_has_distinct_glyphs_and_all_rows_fit_five_bits() {
        let mut seen = std::collections::HashSet::new();
        for c in FONT_CHARS.chars() {
            let g = glyph(c);
            assert!(g.iter().all(|r| *r < 32), "{c:?}");
            if c != ' ' {
                assert!(g.iter().any(|r| *r != 0), "{c:?} is blank");
                assert!(seen.insert(g), "{c:?} duplicates another glyph");
                assert_ne!(g, TOFU, "{c:?} falls back to tofu");
            }
        }
        assert_eq!(glyph('~'), TOFU);
        assert_eq!(glyph('a'), TOFU, "lower case must be folded before drawing");
        assert_eq!(glyph(' '), [0; 7]);
    }

    #[test]
    fn font_text_folds_and_marks_unknowns() {
        assert_eq!(font_text("Café Ñandú"), "CAFE NANDU");
        assert_eq!(font_text("a\tb\nc"), "A B C");
        assert_eq!(font_text("日本"), "\u{FFFD}\u{FFFD}");
        assert_eq!(font_text("Q&A: 1+1"), "Q&A: 1+1");
        assert_eq!(font_text(""), "");
    }

    #[test]
    fn contrast_math() {
        assert!((contrast_ratio([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio([10, 20, 30], [10, 20, 30]) - 1.0).abs() < 1e-12);
        assert_eq!(
            contrast_ratio([1, 2, 3], [200, 100, 50]),
            contrast_ratio([200, 100, 50], [1, 2, 3])
        );
        assert_eq!(luminance([0, 0, 0]), 0.0);
        assert!((luminance([255, 255, 255]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn text_colour_is_always_readable() {
        for r in (0..=255).step_by(15) {
            for g in (0..=255).step_by(15) {
                for b in (0..=255).step_by(15) {
                    let bg = [r as u8, g as u8, b as u8];
                    let t = pick_text_color(bg);
                    assert!(contrast_ratio(bg, t) >= 4.5, "{bg:?} -> {t:?}");
                }
            }
        }
        assert_eq!(pick_text_color([0, 0, 0]), [255, 255, 255]);
        assert_eq!(pick_text_color([255, 255, 255]), [0, 0, 0]);
    }

    #[test]
    fn palette_is_deterministic_dark_and_readable() {
        assert_eq!(palette("A", "B", "C"), palette("A", "B", "C"));
        assert_ne!(palette("A", "B", "C"), palette("A", "B", "D"));
        for i in 0..300 {
            let (bg, accent) = palette(&format!("Title {i}"), "Author", "");
            assert!(contrast_ratio(bg, pick_text_color(bg)) >= 4.5);
            assert!(
                contrast_ratio(bg, accent) >= 3.0,
                "accent must stand out: {bg:?} {accent:?}"
            );
        }
    }

    #[test]
    fn wrapping() {
        assert_eq!(
            wrap_text("THE LORD OF THE RINGS", 10),
            vec!["THE LORD", "OF THE", "RINGS"]
        );
        assert_eq!(wrap_text("ABCDEFGHIJKL", 5), vec!["ABCDE", "FGHIJ", "KL"]);
        assert_eq!(wrap_text("AB CDEFGHIJKL", 5), vec!["AB", "CDEFG", "HIJKL"]);
        assert_eq!(wrap_text("   ", 5), Vec::<String>::new());
        assert_eq!(
            wrap_text("A B", 0),
            vec!["A", "B"],
            "zero width is treated as one column"
        );
        assert_eq!(wrap_text("EXACT", 5), vec!["EXACT"]);
    }

    #[test]
    fn fit_text_shrinks_then_truncates() {
        let (s, l) = fit_text("SHORT", 300, 300, 4, 6);
        assert_eq!((s, l.len()), (6, 1), "{l:?}");
        let (s2, l2) = fit_text(&"WORD ".repeat(40), 300, 400, 6, 6);
        assert!(s2 < 6 && l2.len() <= 6);
        let (s3, l3) = fit_text(&"X".repeat(5000), 100, 50, 2, 6);
        assert_eq!(s3, 1);
        assert!(l3.len() <= 2 && l3.last().unwrap().ends_with("..."));
    }

    #[test]
    fn generated_cover_decodes_with_requested_size() {
        let png =
            generate_cover("The Hobbit", "J. R. R. Tolkien", "Middle-earth", 600, 900).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (600, 900));
        assert_eq!(info(&png).unwrap(), (600, 900, ImageFormat::Png));
    }

    #[test]
    fn generated_cover_has_background_text_and_accent_colours() {
        let png = generate_cover("MMMM", "Author", "", 400, 600).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgb8();
        let (bg, accent) = palette("MMMM", "Author", "");
        let text = pick_text_color(bg);
        let count = |c: [u8; 3]| img.pixels().filter(|p| p.0 == c).count();
        assert!(count(bg) > 400 * 600 / 2, "background dominates");
        assert!(
            count(text) > 500,
            "title/author pixels present: {}",
            count(text)
        );
        assert!(count(accent) > 400, "accent bars present");
        assert_eq!(img.get_pixel(0, 0).0, accent, "top bar");
        assert_eq!(img.get_pixel(399, 599).0, accent, "bottom bar");
        // everything drawn is one of the three palette colours
        assert!(img.pixels().all(|p| [bg, accent, text].contains(&p.0)));
    }

    #[test]
    fn title_pixels_are_in_the_title_area_and_author_below() {
        let png = generate_cover("X", "Y", "", 400, 600).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgb8();
        let (bg, _) = palette("X", "Y", "");
        let text = pick_text_color(bg);
        let rows_with_text: Vec<u32> = (0..600)
            .filter(|y| (0..400).any(|x| img.get_pixel(x, *y).0 == text))
            .collect();
        assert!(
            rows_with_text.first().copied().unwrap() >= 600 / 6,
            "title starts below the top region"
        );
        assert!(
            rows_with_text.last().copied().unwrap() > 600 * 3 / 4,
            "author sits near the bottom"
        );
    }

    #[test]
    fn covers_are_deterministic_and_input_sensitive() {
        let a = generate_cover("Dune", "Frank Herbert", "Dune", 300, 450).unwrap();
        let b = generate_cover("Dune", "Frank Herbert", "Dune", 300, 450).unwrap();
        assert_eq!(a, b);
        assert_ne!(
            a,
            generate_cover("Dune", "Frank Herbert", "", 300, 450).unwrap()
        );
        assert_ne!(
            a,
            generate_cover("Dune!", "Frank Herbert", "Dune", 300, 450).unwrap()
        );
        assert_ne!(
            a,
            generate_cover("Dune", "Frank Herbert", "Dune", 301, 450).unwrap()
        );
    }

    #[test]
    fn unicode_and_extreme_text_do_not_panic() {
        let long = "VERY ".repeat(400);
        let cases = [
            ("Ünïcödé Tïtle – Ñandú", "Gabriel García Márquez"),
            ("日本語のタイトル", "村上春樹"),
            ("😀🎉", "👨\u{200D}👩"),
            ("", ""),
            ("   ", "   "),
            (long.as_str(), long.as_str()),
            ("a\u{0}b\tc\nd", "\u{202E}rtl"),
            ("\u{0301}\u{0301}", "x"),
        ];
        for (t, a) in cases {
            let png = generate_cover(t, a, t, 300, 450).unwrap_or_else(|e| panic!("{t:?}: {e}"));
            assert_eq!(dims(&png), (300, 450));
        }
    }

    #[test]
    fn long_text_is_not_drawn_outside_the_margins() {
        let png =
            generate_cover(&"WIDE ".repeat(100), &"AUTHOR ".repeat(50), "", 300, 450).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgb8();
        let (bg, accent) = palette(&"WIDE ".repeat(100), &"AUTHOR ".repeat(50), "");
        let margin = 300 / 10;
        for y in 20..430 {
            for x in (0..margin - 1).chain(300 - margin + 1..300) {
                let p = img.get_pixel(x, y).0;
                assert!(
                    p == bg || p == accent,
                    "text bled into the margin at ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn cover_size_validation() {
        assert!(generate_cover("a", "b", "", 100, 400)
            .unwrap_err()
            .to_string()
            .contains("out of range"));
        assert!(generate_cover("a", "b", "", 400, 100).is_err());
        assert!(generate_cover("a", "b", "", 100_000, 400).is_err());
        assert!(generate_cover("a", "b", "", 120, 160).is_ok());
    }

    #[test]
    fn crop_to_aspect_centres_and_keeps_format() {
        let out = crop_to_aspect(&halves(400, 200), 1, 1).unwrap();
        assert_eq!(dims(&out), (200, 200));
        let img = image::load_from_memory(&out).unwrap().to_rgb8();
        assert_eq!(img.get_pixel(0, 100).0, [255, 0, 0], "left half is red");
        assert_eq!(
            img.get_pixel(199, 100).0,
            [0, 0, 255],
            "right half is blue: crop was centred"
        );
        let tall = crop_to_aspect(&solid(300, 900, [9, 9, 9], ImageFormat::Png), 2, 3).unwrap();
        assert_eq!(dims(&tall), (300, 450));
        let jpg = solid(400, 200, [30, 60, 90], ImageFormat::Jpeg);
        let out = crop_to_aspect(&jpg, 1, 1).unwrap();
        assert_eq!(&out[..3], &[0xFF, 0xD8, 0xFF], "JPEG stays JPEG");
        assert_eq!(dims(&out), (200, 200));
    }

    #[test]
    fn crop_to_aspect_noop_and_errors() {
        let img = solid(200, 300, [1, 2, 3], ImageFormat::Png);
        assert_eq!(
            crop_to_aspect(&img, 2, 3).unwrap(),
            img,
            "already right: untouched bytes"
        );
        assert!(crop_to_aspect(&img, 0, 3).is_err());
        assert!(crop_to_aspect(&img, 3, 0).is_err());
        assert!(crop_to_aspect(b"junk", 1, 1).is_err());
        let tiny = solid(1, 1, [1, 2, 3], ImageFormat::Png);
        assert_eq!(dims(&crop_to_aspect(&tiny, 1, 5).unwrap()), (1, 1));
        let thin = solid(1000, 3, [1, 2, 3], ImageFormat::Png);
        assert_eq!(dims(&crop_to_aspect(&thin, 1, 1).unwrap()), (3, 3));
    }

    #[test]
    fn resize_never_upscales_and_keeps_aspect() {
        let img = solid(1000, 500, [10, 20, 30], ImageFormat::Png);
        assert_eq!(dims(&resize_max(&img, 400, 400).unwrap()), (400, 200));
        assert_eq!(dims(&resize_max(&img, 5000, 5000).unwrap()), (1000, 500));
        assert_eq!(
            resize_max(&img, 1000, 500).unwrap(),
            img,
            "fits: bytes untouched"
        );
        assert_eq!(dims(&resize_max(&img, 100, 1000).unwrap()), (100, 50));
        let jpg = solid(800, 1200, [10, 20, 30], ImageFormat::Jpeg);
        let out = resize_max(&jpg, 300, 300).unwrap();
        assert_eq!(&out[..3], &[0xFF, 0xD8, 0xFF]);
        assert_eq!(dims(&out), (200, 300));
        assert!(resize_max(&img, 0, 10).is_err());
        assert!(resize_max(&img, 10, 0).is_err());
        assert!(resize_max(b"nope", 10, 10).is_err());
    }

    #[test]
    fn dominant_colour() {
        let c = dominant_color(&solid(50, 50, [200, 30, 40], ImageFormat::Png)).unwrap();
        assert_eq!(c, [200, 30, 40]);
        // 70% green, 30% red
        let mut img = RgbImage::from_pixel(100, 100, Rgb([0, 200, 0]));
        for y in 0..30 {
            for x in 0..100 {
                img.put_pixel(x, y, Rgb([220, 0, 0]));
            }
        }
        let bytes = encode(&DynamicImage::ImageRgb8(img), ImageFormat::Png).unwrap();
        assert_eq!(dominant_color(&bytes).unwrap(), [0, 200, 0]);
        let jpg = dominant_color(&solid(64, 64, [10, 100, 200], ImageFormat::Jpeg)).unwrap();
        for (a, b) in jpg.iter().zip([10i32, 100, 200]) {
            assert!((*a as i32 - b).abs() <= 6, "{jpg:?}");
        }
        assert!(dominant_color(b"x").is_err());
    }

    #[test]
    fn dominant_colour_ignores_transparency() {
        let mut img = image::RgbaImage::from_pixel(40, 40, image::Rgba([255, 255, 255, 0]));
        for y in 0..10 {
            for x in 0..10 {
                img.put_pixel(x, y, image::Rgba([0, 0, 255, 255]));
            }
        }
        let bytes = encode(&DynamicImage::ImageRgba8(img), ImageFormat::Png).unwrap();
        assert_eq!(dominant_color(&bytes).unwrap(), [0, 0, 255]);
        let clear = encode(
            &DynamicImage::ImageRgba8(image::RgbaImage::new(8, 8)),
            ImageFormat::Png,
        )
        .unwrap();
        assert!(dominant_color(&clear)
            .unwrap_err()
            .to_string()
            .contains("no opaque"));
    }

    #[test]
    fn blank_detection() {
        assert!(is_blank(&solid(200, 300, [255, 255, 255], ImageFormat::Png)).unwrap());
        assert!(is_blank(&solid(200, 300, [0, 0, 0], ImageFormat::Jpeg)).unwrap());
        assert!(
            is_blank(&solid(200, 300, [128, 90, 40], ImageFormat::Png)).unwrap(),
            "solid colour counts as blank"
        );
        let mut noisy = RgbImage::from_pixel(200, 300, Rgb([250, 250, 250]));
        for y in 100..140 {
            for x in 20..180 {
                noisy.put_pixel(x, y, Rgb([10, 10, 10]));
            }
        }
        assert!(
            !is_blank(&encode(&DynamicImage::ImageRgb8(noisy), ImageFormat::Png).unwrap()).unwrap()
        );
        let cover = generate_cover("Real Title", "Someone", "", 300, 450).unwrap();
        assert!(!is_blank(&cover).unwrap());
        let nearly = {
            let mut i = RgbImage::from_pixel(300, 300, Rgb([255, 255, 255]));
            i.put_pixel(5, 5, Rgb([0, 0, 0]));
            encode(&DynamicImage::ImageRgb8(i), ImageFormat::Png).unwrap()
        };
        assert!(is_blank(&nearly).unwrap(), "a single speck is still blank");
        let clear = encode(
            &DynamicImage::ImageRgba8(image::RgbaImage::new(8, 8)),
            ImageFormat::Png,
        )
        .unwrap();
        assert!(is_blank(&clear).unwrap());
        assert!(is_blank(b"junk").is_err());
    }

    #[test]
    fn decode_rejects_other_formats_and_garbage() {
        let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec();
        assert!(decode(&gif)
            .unwrap_err()
            .to_string()
            .contains("unsupported image format"));
        assert!(decode(b"").is_err());
        assert!(decode(b"definitely not an image")
            .unwrap_err()
            .to_string()
            .contains("not a recognised"));
        let mut truncated = solid(50, 50, [1, 2, 3], ImageFormat::Png);
        truncated.truncate(30);
        assert!(decode(&truncated).is_err());
        assert!(info(&gif).is_err());
        assert!(info(b"").is_err());
    }

    #[test]
    fn info_reports_format() {
        assert_eq!(
            info(&solid(7, 9, [0; 3], ImageFormat::Jpeg)).unwrap(),
            (7, 9, ImageFormat::Jpeg)
        );
    }

    #[test]
    fn encode_roundtrip_both_formats() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(10, 10, Rgb([1, 2, 3])));
        for f in [ImageFormat::Png, ImageFormat::Jpeg] {
            let bytes = encode(&img, f).unwrap();
            let (back, fmt) = decode(&bytes).unwrap();
            assert_eq!(fmt, f);
            assert_eq!((back.width(), back.height()), (10, 10));
        }
        let rgba = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            4,
            4,
            image::Rgba([1, 2, 3, 4]),
        ));
        assert!(
            encode(&rgba, ImageFormat::Jpeg).is_ok(),
            "alpha is dropped for JPEG"
        );
    }
}
