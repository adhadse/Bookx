//! RB (NuvoMedia Rocket eBook) input.
//!
//! Little-endian container: magic `B0 0C` + (`B0 0C` | `F0 0D` | `FE FE`), version, `NUVO`,
//! creation date, the table-of-contents offset at 0x18 and the file length at 0x1C. The table
//! of contents is an `int32` count followed by 44 byte entries (32 byte zero-padded name, length,
//! offset, flags). Flag 1 = encrypted (rejected), 2 = info page (`NAME=VALUE` lines), 8 =
//! deflated (4096 byte chunks: chunk count, total length, chunk sizes, then deflate data).
//! `.html` pages are converted in table order; `.png` pages are the images; the `.hidx` and
//! `.hkey` index pages are ignored.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::{Identifier, Metadata};
use flate2::read::{DeflateDecoder, ZlibDecoder};

use crate::common::*;

const FLAG_ENCRYPTED: u32 = 1;
const FLAG_INFO: u32 = 2;
const FLAG_DEFLATED: u32 = 8;
const CHUNK: usize = 4096;
/// Largest decompressed page.
const MAX_PAGE: usize = 64 << 20;
const MAX_PAGES: usize = 50_000;

/// Reader for `.rb` files.
pub struct RbReader;

fn has_magic(head: &[u8]) -> bool {
    head.len() >= 4
        && head[0] == 0xB0
        && head[1] == 0x0C
        && matches!(
            [head[2], head[3]],
            [0xB0, 0x0C] | [0xF0, 0x0D] | [0xFE, 0xFE]
        )
}

impl FormatReader for RbReader {
    fn id(&self) -> &'static str {
        "rb"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["rb"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        has_magic(head)
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_rb(bytes)
    }
}

fn le32(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// One table-of-contents entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RbEntry {
    pub name: String,
    pub length: u32,
    pub offset: u32,
    pub flags: u32,
}

/// Reads the table of contents.
pub fn parse_toc(bytes: &[u8]) -> Result<Vec<RbEntry>> {
    if !has_magic(bytes) {
        bail!("not a Rocket eBook file: the magic number is wrong");
    }
    let toc = le32(bytes, 0x18).context("the file is too short for a header")? as usize;
    let count = le32(bytes, toc).context("the table of contents lies outside the file")? as usize;
    if count == 0 || count > MAX_PAGES {
        bail!("the table of contents lists {count} pages");
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let at = toc + 4 + i * 44;
        let raw = bytes
            .get(at..at + 44)
            .context("the table of contents is truncated")?;
        let end = raw[..32].iter().position(|b| *b == 0).unwrap_or(32);
        out.push(RbEntry {
            name: String::from_utf8_lossy(&raw[..end]).into_owned(),
            length: le32(raw, 32).unwrap_or(0),
            offset: le32(raw, 36).unwrap_or(0),
            flags: le32(raw, 40).unwrap_or(0),
        });
    }
    Ok(out)
}

fn inflate_chunk(data: &[u8], expect: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let z = ZlibDecoder::new(data)
        .take(expect as u64 + 1)
        .read_to_end(&mut out);
    if z.is_err() {
        out.clear();
        DeflateDecoder::new(data)
            .take(expect as u64 + 1)
            .read_to_end(&mut out)
            .context("corrupt deflate data in a Rocket eBook page")?;
    }
    Ok(out)
}

/// Expands the data of a page flagged as deflated.
pub fn decompress_page(data: &[u8]) -> Result<Vec<u8>> {
    let count = le32(data, 0).context("compressed page header is truncated")? as usize;
    let total = le32(data, 4).context("compressed page header is truncated")? as usize;
    if total > MAX_PAGE {
        bail!("a page of {total} bytes is larger than the limit of {MAX_PAGE}");
    }
    if count > total.div_ceil(CHUNK) + 1 || (total > 0 && count == 0) {
        bail!("the chunk count {count} does not fit a page of {total} bytes");
    }
    let mut pos = 8 + count * 4;
    let mut out = Vec::with_capacity(total);
    for i in 0..count {
        let size = le32(data, 8 + i * 4).context("chunk size table is truncated")? as usize;
        let chunk = data
            .get(pos..pos.checked_add(size).context("chunk size overflows")?)
            .context("a compressed chunk lies beyond the end of the page")?;
        pos += size;
        let want = CHUNK.min(total.saturating_sub(out.len()));
        let part = inflate_chunk(chunk, want)?;
        out.extend_from_slice(&part);
    }
    if out.len() != total {
        bail!("page decompressed to {} bytes, expected {total}", out.len());
    }
    Ok(out)
}

/// Parses the `NAME=VALUE` lines of the info page.
pub fn parse_info(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().to_string()))
        .collect()
}

/// Converts a Rocket eBook file.
pub fn convert_rb(bytes: &[u8]) -> Result<Archive> {
    let toc = parse_toc(bytes)?;
    let mut info = HashMap::new();
    let mut pages: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total = 0usize;
    for e in &toc {
        if e.flags & FLAG_ENCRYPTED != 0 {
            bail!("this Rocket eBook is encrypted (DRM) and cannot be read");
        }
        let start = e.offset as usize;
        let Some(raw) = start
            .checked_add(e.length as usize)
            .and_then(|end| bytes.get(start..end))
        else {
            bail!(
                "page `{}` lies beyond the end of the file (truncated file)",
                e.name
            );
        };
        let data = if e.flags & FLAG_DEFLATED != 0 {
            decompress_page(raw).with_context(|| format!("page `{}`", e.name))?
        } else {
            raw.to_vec()
        };
        total += data.len();
        if total > MAX_PAGE * 4 {
            bail!("the book is too large");
        }
        if e.flags & FLAG_INFO != 0 || e.name.to_ascii_lowercase().ends_with(".info") {
            info = parse_info(&decode_text(&data));
        } else {
            pages.push((e.name.clone(), data));
        }
    }

    let mut md = Metadata {
        title: info.get("TITLE").cloned().unwrap_or_default(),
        ..Default::default()
    };
    if let Some(a) = info.get("AUTHOR").filter(|a| !a.is_empty()) {
        md.authors = vec![a.clone()];
    }
    if let Some(p) = info.get("PUB_NAME") {
        md.publisher = p.clone();
    }
    if let Some(c) = info.get("COPYRIGHT") {
        md.rights = c.clone();
    }
    if let Some(l) = info.get("TITLE_LANGUAGE") {
        md.language = l.clone();
    }
    if let Some(g) = info.get("GENRE").filter(|g| !g.is_empty()) {
        md.tags = vec![g.clone()];
    }
    if let Some(i) = info.get("ISBN").filter(|i| !i.is_empty()) {
        md.identifiers.push(Identifier {
            scheme: "isbn".into(),
            value: i.clone(),
            xml_id: None,
        });
    }

    let media = RefCell::new(MediaSet::default());
    for (name, data) in &pages {
        if name.to_ascii_lowercase().ends_with(".png") && image_ext(data).is_some() {
            media
                .borrow_mut()
                .add(&name.to_ascii_lowercase(), data.clone());
        }
    }
    let rewrite = |src: &str| -> Option<String> {
        media
            .borrow()
            .path_of(&src.to_ascii_lowercase())
            .map(String::from)
    };
    let default_title = if md.title.is_empty() {
        "Beginning".to_string()
    } else {
        md.title.clone()
    };
    let body_name = info.get("BODY").map(|b| b.to_ascii_lowercase());
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut html_pages: Vec<&(String, Vec<u8>)> = pages
        .iter()
        .filter(|(n, _)| {
            let l = n.to_ascii_lowercase();
            l.ends_with(".html") || l.ends_with(".htm")
        })
        .collect();
    // The root page named by BODY comes first when it is not already.
    if let Some(b) = &body_name {
        if let Some(pos) = html_pages
            .iter()
            .position(|(n, _)| n.to_ascii_lowercase() == *b)
        {
            let root = html_pages.remove(pos);
            html_pages.insert(0, root);
        }
    }
    for (n, (_, data)) in html_pages.iter().enumerate() {
        let html = decode_xml(data);
        let blocks = html_blocks(&html, &rewrite);
        let title = if n == 0 {
            default_title.clone()
        } else {
            format!("{default_title} ({})", n + 1)
        };
        chapters.extend(chapters_from_blocks(blocks, &title));
    }
    if chapters.is_empty() {
        bail!("the Rocket eBook has no readable text pages");
    }
    if md.title.is_empty() {
        md.title = "Untitled".into();
    }
    let cover = media
        .borrow()
        .items
        .iter()
        .find(|m| m.path.to_ascii_lowercase().contains("cover"))
        .map(|m| (m.data.clone(), "png".to_string()));
    build(Parts {
        md,
        chapters,
        cover,
        media: media.into_inner().items,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fmt_writers::*;
    use crate::testutil::*;

    fn sample() -> Vec<u8> {
        write_rb(&RbSpec {
            info: "COMMENT=x\nTYPE=2\nTITLE=Rocket Book\nAUTHOR=Rick Et\nBODY=000001-main.html\nISBN=0-1\nPUB_NAME=Nuvo\nGENRE=Reference\nTITLE_LANGUAGE=en-us\nCOPYRIGHT=(c)\n".into(),
            pages: vec![
                ("000001-main.html".into(), b"<HTML><BODY><H1>First</H1><P>Hello rocket.</P><IMG SRC=\"000002-pic.png\"><H1>Second</H1><P>More.</P></BODY></HTML>".to_vec(), true),
                ("000002-pic.png".into(), PNG.to_vec(), false),
                ("000003-extra.html".into(), b"<HTML><BODY><P>Extra page text.</P></BODY></HTML>".to_vec(), false),
            ],
            ..Default::default()
        })
    }

    #[test]
    fn header_and_toc_parse() {
        let f = sample();
        let toc = parse_toc(&f).unwrap();
        assert_eq!(toc.len(), 4);
        assert_eq!(toc[0].name, "000000.info");
        assert_eq!(toc[0].flags & FLAG_INFO, FLAG_INFO);
        assert_eq!(toc[1].flags & FLAG_DEFLATED, FLAG_DEFLATED);
        assert!(RbReader.sniff(&f));
    }

    #[test]
    fn converts_text_images_and_metadata() {
        let arc = RbReader.read(&sample()).unwrap();
        let book = validate(&arc);
        let md = book.metadata();
        assert_eq!(md.title, "Rocket Book");
        assert_eq!(md.authors, ["Rick Et"]);
        assert_eq!(md.publisher, "Nuvo");
        assert_eq!(md.language, "en-us");
        assert_eq!(md.identifier("isbn"), Some("0-1"));
        assert_eq!(toc_titles(&book), ["First", "Second", "Rocket Book (2)"]);
        let t = chapter_texts(&book);
        assert!(t[0].contains("Hello rocket."));
        assert!(t[2].contains("Extra page text."));
        assert!(chapter_xml(&book, 0).contains("<img src=\"images/"));
    }

    #[test]
    fn root_page_named_by_body_comes_first() {
        let f = write_rb(&RbSpec {
            info: "TITLE=T\nBODY=b.html\n".into(),
            pages: vec![
                ("a.html".into(), b"<p>AAA</p>".to_vec(), false),
                ("b.html".into(), b"<p>BBB</p>".to_vec(), false),
            ],
            ..Default::default()
        });
        let t = chapter_texts(&validate(&RbReader.read(&f).unwrap()));
        assert!(t[0].contains("BBB") && t[1].contains("AAA"));
    }

    #[test]
    fn alternate_magics_are_accepted_and_others_rejected() {
        for m in [
            [0xB0, 0x0C, 0xB0, 0x0C],
            [0xB0, 0x0C, 0xF0, 0x0D],
            [0xB0, 0x0C, 0xFE, 0xFE],
        ] {
            let mut f = sample();
            f[..4].copy_from_slice(&m);
            assert!(RbReader.sniff(&f));
            assert!(RbReader.read(&f).is_ok());
        }
        let mut f = sample();
        f[2] = 0x11;
        assert!(!RbReader.sniff(&f));
        assert!(format!("{}", RbReader.read(&f).unwrap_err()).contains("magic number"));
    }

    #[test]
    fn encrypted_titles_have_a_clear_error() {
        let mut f = sample();
        let toc = le32(&f, 0x18).unwrap() as usize;
        // flags of the second entry
        let at = toc + 4 + 44 + 40;
        f[at..at + 4].copy_from_slice(&(FLAG_ENCRYPTED | FLAG_DEFLATED).to_le_bytes());
        assert!(format!("{}", RbReader.read(&f).unwrap_err()).contains("encrypted (DRM)"));
    }

    #[test]
    fn page_beyond_file_and_bad_toc_are_errors() {
        let f = sample();
        let cut = &f[..f.len() - 25];
        let m = format!("{:#}", RbReader.read(cut).unwrap_err());
        assert!(m.contains("beyond the end"), "{m}");
        let mut f = sample();
        let toc = le32(&f, 0x18).unwrap() as usize;
        f[toc..toc + 4].copy_from_slice(&0u32.to_le_bytes());
        assert!(format!("{}", RbReader.read(&f).unwrap_err()).contains("lists 0 pages"));
        let mut f = sample();
        f[0x18..0x1C].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(RbReader.read(&f).is_err());
        assert!(parse_toc(&[0xB0, 0x0C, 0xB0, 0x0C]).is_err());
        let mut f = sample();
        f[toc..toc + 4].copy_from_slice(&9999u32.to_le_bytes());
        assert!(format!("{}", RbReader.read(&f).unwrap_err()).contains("truncated"));
    }

    #[test]
    fn decompress_hand_assembled_page() {
        // count=2, total=4097, sizes, zlib of 4096 x 'a' and of "b" (computed with zlib).
        let c1 = zlib_compress(&vec![b'a'; 4096]);
        let c2 = zlib_compress(b"b");
        let mut d = Vec::new();
        d.extend(2u32.to_le_bytes());
        d.extend(4097u32.to_le_bytes());
        d.extend((c1.len() as u32).to_le_bytes());
        d.extend((c2.len() as u32).to_le_bytes());
        d.extend(&c1);
        d.extend(&c2);
        let out = decompress_page(&d).unwrap();
        assert_eq!(out.len(), 4097);
        assert_eq!(out[4096], b'b');
        // The same with raw deflate chunks (no zlib wrapper) is accepted too.
        let raw = |z: &[u8]| z[2..z.len() - 4].to_vec();
        let (r1, r2) = (raw(&c1), raw(&c2));
        let mut d = Vec::new();
        d.extend(2u32.to_le_bytes());
        d.extend(4097u32.to_le_bytes());
        d.extend((r1.len() as u32).to_le_bytes());
        d.extend((r2.len() as u32).to_le_bytes());
        d.extend(&r1);
        d.extend(&r2);
        assert_eq!(decompress_page(&d).unwrap(), out);
    }

    #[test]
    fn decompress_errors() {
        assert!(decompress_page(&[1, 0]).is_err());
        let mut d = Vec::new();
        d.extend(1u32.to_le_bytes());
        d.extend(((MAX_PAGE + 1) as u32).to_le_bytes());
        assert!(format!("{}", decompress_page(&d).unwrap_err()).contains("larger than the limit"));
        let mut d = Vec::new();
        d.extend(9u32.to_le_bytes());
        d.extend(10u32.to_le_bytes());
        assert!(format!("{}", decompress_page(&d).unwrap_err()).contains("does not fit"));
        // length mismatch
        let c = zlib_compress(b"abc");
        let mut d = Vec::new();
        d.extend(1u32.to_le_bytes());
        d.extend(5u32.to_le_bytes());
        d.extend((c.len() as u32).to_le_bytes());
        d.extend(&c);
        assert!(format!("{}", decompress_page(&d).unwrap_err()).contains("expected 5"));
        // chunk beyond the page
        let mut d = Vec::new();
        d.extend(1u32.to_le_bytes());
        d.extend(5u32.to_le_bytes());
        d.extend(999u32.to_le_bytes());
        assert!(format!("{}", decompress_page(&d).unwrap_err()).contains("beyond the end"));
        // garbage chunk
        let mut d = Vec::new();
        d.extend(1u32.to_le_bytes());
        d.extend(5u32.to_le_bytes());
        d.extend(3u32.to_le_bytes());
        d.extend([0xFF, 0xFF, 0xFF]);
        assert!(decompress_page(&d).is_err());
    }

    #[test]
    fn info_parser() {
        let m = parse_info("title=Hello World\nAUTHOR= Me \nnoequals\nURL=ebook:a=b\n");
        assert_eq!(m["TITLE"], "Hello World");
        assert_eq!(m["AUTHOR"], "Me");
        assert_eq!(m["URL"], "ebook:a=b");
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn no_text_pages_is_an_error_and_missing_info_is_fine() {
        let f = write_rb(&RbSpec {
            info: String::new(),
            pages: vec![("a.png".into(), PNG.to_vec(), false)],
            no_info_page: true,
        });
        assert!(format!("{}", RbReader.read(&f).unwrap_err()).contains("no readable text"));
        let f = write_rb(&RbSpec {
            pages: vec![("a.html".into(), b"<p>just text</p>".to_vec(), false)],
            no_info_page: true,
            ..Default::default()
        });
        let book = validate(&RbReader.read(&f).unwrap());
        assert_eq!(book.metadata().title, "Untitled");
    }

    #[test]
    fn ids_extensions_and_fuzz() {
        assert_eq!(RbReader.id(), "rb");
        assert_eq!(RbReader.extensions(), &["rb"]);
        assert!(!RbReader.sniff(b"\xB0\x0C"));
        fuzz_no_panic(&RbReader, &sample());
        assert_rejects_garbage(&RbReader);
    }
}
