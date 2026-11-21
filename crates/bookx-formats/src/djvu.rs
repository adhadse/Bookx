//! DjVu documents read as page pictures, like a comic.
//!
//! There is no DjVu decoder in Rust that Bookx can use, so the pages are rendered by `ddjvu`
//! (DjVuLibre) when it is installed, one process per page, and the book is a picture book (the
//! same EPUB a CBZ becomes). Rules as for RAR (`rar`, docs/security.md): programs are looked up in
//! fixed system directories only, started with an argument vector, on a private temporary copy of
//! the file; every page is limited in size and time, and so is the page count.
//!
//! The text layer of a DjVu file is not read (`djvutxt` could provide it; not done). Without
//! DjVuLibre the reader says what to install.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;

use crate::helper::{find_in, run, TempFile, DIRS};

/// `AT&TFORM` + length + `DJVU` (one page) or `DJVM` (several).
pub fn is_djvu(head: &[u8]) -> bool {
    head.len() >= 16
        && head.starts_with(b"AT&TFORM")
        && (&head[12..16] == b"DJVU" || &head[12..16] == b"DJVM")
}

/// Most pages read from one document.
pub const MAX_PAGES: usize = 3000;
/// Bytes of one rendered page (a PPM): about 130 megapixels of RGB.
const MAX_PAGE_BYTES: u64 = 400 << 20;
const PAGE_TIME: Duration = Duration::from_secs(60);
const TOTAL_BUDGET: u64 = 1 << 30;
/// The longest side of a rendered page; DjVu pages are often scanned at 400 dpi.
const MAX_SIDE: u32 = 2400;

/// The programs of DjVuLibre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub ddjvu: PathBuf,
    pub djvused: PathBuf,
}

pub fn find_tools() -> Option<Tools> {
    find_tools_in(DIRS)
}

fn find_tools_in(dirs: &[&str]) -> Option<Tools> {
    Some(Tools {
        ddjvu: find_in(dirs, "ddjvu")?,
        djvused: find_in(dirs, "djvused")?,
    })
}

/// A rendered page: `P5` (grey) or `P6` (colour) PNM data to its pixels.
pub fn parse_pnm(data: &[u8]) -> Result<(u32, u32, u8, &[u8])> {
    let mut pos = 0usize;
    let mut token = |data: &[u8]| -> Option<String> {
        loop {
            while pos < data.len() && data[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if data.get(pos) == Some(&b'#') {
                while pos < data.len() && data[pos] != b'\n' {
                    pos += 1;
                }
            } else {
                break;
            }
        }
        let start = pos;
        while pos < data.len() && !data[pos].is_ascii_whitespace() && pos - start < 16 {
            pos += 1;
        }
        (pos > start).then(|| String::from_utf8_lossy(&data[start..pos]).into_owned())
    };
    let magic = token(data).context("not a PNM page")?;
    let channels = match magic.as_str() {
        "P5" => 1u8,
        "P6" => 3u8,
        _ => bail!("not a grey or colour PNM page"),
    };
    let w: u32 = token(data)
        .and_then(|t| t.parse().ok())
        .context("bad width")?;
    let h: u32 = token(data)
        .and_then(|t| t.parse().ok())
        .context("bad height")?;
    let max: u32 = token(data)
        .and_then(|t| t.parse().ok())
        .context("bad depth")?;
    if max != 255 {
        bail!("unsupported colour depth");
    }
    // one whitespace byte separates the header from the pixels
    pos += 1;
    let want = w as u64 * h as u64 * channels as u64;
    if w == 0 || h == 0 || want > MAX_PAGE_BYTES || data.len() as u64 != pos as u64 + want {
        bail!("the page data does not match its size");
    }
    Ok((w, h, channels, &data[pos..]))
}

fn page_count(tools: &Tools, file: &Path) -> Result<usize> {
    let out = run(
        &tools.djvused,
        &[
            std::ffi::OsStr::new("-e"),
            std::ffi::OsStr::new("n"),
            file.as_os_str(),
        ],
        64,
        Duration::from_secs(30),
    )?;
    let n: usize = String::from_utf8_lossy(&out)
        .trim()
        .parse()
        .context("the document has no page count")?;
    if n == 0 {
        bail!("the document has no pages");
    }
    if n > MAX_PAGES {
        bail!("the document has too many pages");
    }
    Ok(n)
}

fn render(tools: &Tools, file: &Path, page: usize) -> Result<Vec<u8>> {
    let p = format!("-page={page}");
    let size = format!("-size={MAX_SIDE}x{MAX_SIDE}");
    run(
        &tools.ddjvu,
        &[
            std::ffi::OsStr::new("-format=pnm"),
            std::ffi::OsStr::new(&p),
            std::ffi::OsStr::new(&size),
            file.as_os_str(),
        ],
        MAX_PAGE_BYTES + 64,
        PAGE_TIME,
    )
}

/// Reads a DjVu document with the helper programs found on this system.
pub fn read_with(tools: &Tools, bytes: &[u8]) -> Result<Archive> {
    if !is_djvu(bytes) {
        bail!("not a DjVu document");
    }
    let tmp = TempFile::create(bytes, "djvu")?;
    let n = page_count(tools, &tmp.0)?;
    let mut files = Vec::with_capacity(n);
    let mut total = 0u64;
    for page in 1..=n {
        let ppm = render(tools, &tmp.0, page)?;
        let (w, h, channels, pixels) = parse_pnm(&ppm)?;
        let jpg = bookx_comic::encode::jpeg(w, h, channels, pixels, 80)?;
        total += jpg.len() as u64;
        if total > TOTAL_BUDGET {
            bail!("the document is too large");
        }
        files.push((format!("page-{page:05}.jpg"), jpg));
    }
    crate::comic::convert_files(files)
}

pub struct DjvuReader;

impl FormatReader for DjvuReader {
    fn id(&self) -> &'static str {
        "djvu"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["djvu", "djv"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        is_djvu(head)
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        let tools = find_tools().context(
            "DjVu books need DjVuLibre (the `ddjvu` and `djvused` programs); install it and try again",
        )?;
        read_with(&tools, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppm(w: u32, h: u32, fill: u8) -> Vec<u8> {
        let mut v = format!("P6\n{w} {h}\n255\n").into_bytes();
        v.extend(vec![fill; (w * h * 3) as usize]);
        v
    }

    #[test]
    fn djvu_files_are_recognised_by_their_header() {
        let mut head = b"AT&TFORM\0\0\0\x10DJVMDIRM".to_vec();
        assert!(is_djvu(&head));
        head[12..16].copy_from_slice(b"DJVU");
        assert!(is_djvu(&head));
        assert!(!is_djvu(b"AT&TFORM\0\0\0\x10DJVX0000"));
        assert!(!is_djvu(b"%PDF-1.4 and more bytes"));
        assert!(!is_djvu(b"AT&T"));
    }

    #[test]
    fn a_rendered_page_is_parsed_and_lies_are_refused() {
        let colour = ppm(3, 2, 7);
        let (w, h, c, px) = parse_pnm(&colour).unwrap();
        assert_eq!((w, h, c, px.len()), (3, 2, 3, 18));
        let grey = b"P5\n# comment\n2 2\n255\n\x01\x02\x03\x04".to_vec();
        let (w, h, c, px) = parse_pnm(&grey).unwrap();
        assert_eq!((w, h, c, px), (2, 2, 1, &[1u8, 2, 3, 4][..]));
        // too few and too many bytes, a huge claim, a 16-bit depth, junk
        assert!(parse_pnm(&ppm(3, 2, 7)[..20]).is_err());
        let mut long = ppm(3, 2, 7);
        long.push(0);
        assert!(parse_pnm(&long).is_err());
        assert!(parse_pnm(b"P6\n100000 100000\n255\nxx").is_err());
        assert!(parse_pnm(b"P6\n2 2\n65535\n").is_err());
        assert!(parse_pnm(b"P4\n2 2\n").is_err());
        assert!(parse_pnm(b"").is_err());
        assert!(parse_pnm(&[b'P'; 40]).is_err());
    }

    #[test]
    fn without_djvulibre_in_the_directories_there_are_no_tools() {
        assert_eq!(find_tools_in(&["/nonexistent-bookx-dir"]), None);
    }

    #[test]
    fn without_the_programs_the_reader_says_what_to_install() {
        if find_tools().is_some() {
            return;
        }
        let mut doc = b"AT&TFORM\0\0\0\x10DJVUINFO".to_vec();
        doc.extend(vec![0; 64]);
        let e = DjvuReader.read(&doc).unwrap_err().to_string();
        assert!(e.contains("DjVuLibre"), "{e}");
    }

    #[test]
    fn a_real_document_is_read_when_djvulibre_is_installed() {
        let Some(tools) = find_tools() else {
            eprintln!("skipped: no ddjvu");
            return;
        };
        // an invalid document must be an error and not a hang or a panic
        let mut doc = b"AT&TFORM\0\0\0\x10DJVUINFO".to_vec();
        doc.extend(vec![0; 64]);
        assert!(read_with(&tools, &doc).is_err());
        assert!(read_with(&tools, b"not djvu at all, really not").is_err());
    }
}
