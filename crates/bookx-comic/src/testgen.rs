//! Synthetic comic pages and archives for tests and the UI scenarios (no real comics are
//! needed, and nothing here is copyrighted).

use std::io::{Cursor, Write};

use crate::panels::Gray;

/// How a generated page is divided.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Layout {
    /// `(columns, rows)` of equal panels.
    Grid(usize, usize),
    /// A wide panel, two panels, then a tall one next to two stacked ones (6 panels).
    Mixed,
    /// One picture over the whole page.
    Splash,
    /// Noise over the whole page.
    Noise,
}

const MARGIN: f64 = 0.03;
const GUTTER: f64 = 0.035;

/// The panels of a layout as `(x, y, w, h)` fractions, in left-to-right reading order.
pub fn rects(l: Layout) -> Vec<[f64; 4]> {
    match l {
        Layout::Splash | Layout::Noise => vec![[0.0, 0.0, 1.0, 1.0]],
        Layout::Grid(c, r) => {
            let cw = (1.0 - 2.0 * MARGIN - GUTTER * (c as f64 - 1.0)) / c as f64;
            let rh = (1.0 - 2.0 * MARGIN - GUTTER * (r as f64 - 1.0)) / r as f64;
            let mut v = Vec::new();
            for j in 0..r {
                for i in 0..c {
                    v.push([
                        MARGIN + i as f64 * (cw + GUTTER),
                        MARGIN + j as f64 * (rh + GUTTER),
                        cw,
                        rh,
                    ]);
                }
            }
            v
        }
        Layout::Mixed => {
            let inner = 1.0 - 2.0 * MARGIN;
            let w = inner;
            let half = (w - GUTTER) / 2.0;
            let avail = inner - 2.0 * GUTTER;
            let (h1, h2, h3) = (avail * 0.25, avail * 0.3, avail * 0.45);
            let y2 = MARGIN + h1 + GUTTER;
            let y3 = y2 + h2 + GUTTER;
            let sh = (h3 - GUTTER) / 2.0;
            vec![
                [MARGIN, MARGIN, w, h1],
                [MARGIN, y2, half, h2],
                [MARGIN + half + GUTTER, y2, half, h2],
                [MARGIN, y3, half, h3],
                [MARGIN + half + GUTTER, y3, half, sh],
                [MARGIN + half + GUTTER, y3 + sh + GUTTER, half, sh],
            ]
        }
    }
}

/// Pixel value of the picture inside a panel (never close to black or white).
fn art(x: usize, y: usize, seed: usize) -> u8 {
    (70 + ((x * 3 + y * 5 + seed * 17) % 120)) as u8
}

fn noise(x: usize, y: usize) -> u8 {
    let mut h = (x as u32).wrapping_mul(374761393) ^ (y as u32).wrapping_mul(668265263);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    (40 + (h >> 24) % 180) as u8
}

/// A grey page: gutters of colour `bg` and the picture inside the panels.
pub fn page_gray(w: usize, h: usize, l: Layout, bg: u8) -> Gray {
    let mut px = vec![bg; w * h];
    if l == Layout::Noise {
        for y in 0..h {
            for x in 0..w {
                px[y * w + x] = noise(x, y);
            }
        }
        return Gray::new(w, h, px);
    }
    for (n, r) in rects(l).iter().enumerate() {
        let (x0, y0) = (
            (r[0] * w as f64).round() as usize,
            (r[1] * h as f64).round() as usize,
        );
        let (x1, y1) = (
            (((r[0] + r[2]) * w as f64).round() as usize).min(w),
            (((r[1] + r[3]) * h as f64).round() as usize).min(h),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                px[y * w + x] = art(x, y, n);
            }
        }
    }
    Gray::new(w, h, px)
}

/// The page as RGBA, with a tint per panel so that pictures can be told apart on screen.
pub fn page_rgba(w: usize, h: usize, l: Layout, bg: u8) -> Vec<u8> {
    let g = page_gray(w, h, l, bg);
    let panels = rects(l);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let v = g.px[y * w + x];
            let (fx, fy) = (x as f64 / w as f64, y as f64 / h as f64);
            let tint = panels
                .iter()
                .position(|r| fx >= r[0] && fx < r[0] + r[2] && fy >= r[1] && fy < r[1] + r[3]);
            let [r, gr, b] = match tint {
                Some(n) if l != Layout::Noise => {
                    let t = [
                        [1.0, 0.55, 0.45],
                        [0.5, 0.9, 0.5],
                        [0.5, 0.6, 1.0],
                        [1.0, 0.9, 0.4],
                    ][n % 4];
                    [
                        (v as f64 * t[0]).min(255.0) as u8,
                        (v as f64 * t[1]).min(255.0) as u8,
                        (v as f64 * t[2]).min(255.0) as u8,
                    ]
                }
                _ => [v, v, v],
            };
            out.extend_from_slice(&[r, gr, b, 255]);
        }
    }
    out
}

pub fn png(w: usize, h: usize, l: Layout, bg: u8) -> Vec<u8> {
    let rgba = page_rgba(w, h, l, bg);
    let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba).expect("size");
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .expect("encode");
    out.into_inner()
}

pub fn jpeg(w: usize, h: usize, l: Layout, bg: u8) -> Vec<u8> {
    let rgba = page_rgba(w, h, l, bg);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let img = image::RgbImage::from_raw(w as u32, h as u32, rgb).expect("size");
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Jpeg)
        .expect("encode");
    out.into_inner()
}

/// A zip archive from `(name, bytes)` entries.
pub fn zip_of(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (n, d) in files {
        z.start_file(n.as_str(), o).expect("start");
        z.write_all(d).expect("write");
    }
    z.finish().expect("finish").into_inner()
}

/// A small comic: a cover, pages with different layouts, a double-page spread and a webtoon-ish
/// tall page. `comic_info` is the text of `ComicInfo.xml` when given.
pub fn small_comic(comic_info: Option<&str>) -> Vec<u8> {
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("p001.png".into(), png(400, 600, Layout::Splash, 255)),
        ("p002.png".into(), png(400, 600, Layout::Grid(2, 2), 255)),
        ("p003.png".into(), png(400, 600, Layout::Mixed, 255)),
        ("p004.png".into(), png(800, 600, Layout::Grid(3, 1), 255)),
        ("p005.png".into(), png(400, 600, Layout::Grid(1, 3), 255)),
        ("p006.png".into(), png(400, 600, Layout::Grid(2, 3), 0)),
    ];
    if let Some(ci) = comic_info {
        files.push(("ComicInfo.xml".into(), ci.as_bytes().to_vec()));
    }
    zip_of(&files)
}

/// A PNG with a header that claims a huge size (an image bomb): the dimensions are real, the
/// pixel data is a few bytes of zlib.
pub fn bomb_png(w: u32, h: u32) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for b in data {
            c ^= *b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    (c >> 1) ^ 0xEDB8_8320
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    fn chunk(t: &[u8; 4], d: &[u8]) -> Vec<u8> {
        let mut v = (d.len() as u32).to_be_bytes().to_vec();
        let mut body = t.to_vec();
        body.extend_from_slice(d);
        v.extend_from_slice(&body);
        v.extend_from_slice(&crc(&body).to_be_bytes());
        v
    }
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(chunk(b"IHDR", &ihdr));
    // an empty deflate stream: the decoder would run out of data long before the claimed size
    out.extend(chunk(
        b"IDAT",
        &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    ));
    out.extend(chunk(b"IEND", &[]));
    out
}
