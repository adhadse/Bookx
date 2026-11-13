//! SNB (Shanda Bambook) input.
//!
//! Container (big endian): a 44 byte header (`SNBP000B`, 0x8000, 0x00A3A3A3, 0, file count,
//! size of the file table, size of its zlib stream, size of the binary stream, uncompressed size
//! of the plain stream, 0), the zlib compressed file table (12 byte entries `attr, name offset,
//! size`, then the NUL separated names), the binary stream (images, stored), the plain stream
//! (`.snbf` / `.snbc` XML) as bzip2 blocks of 32 KiB, a zlib compressed tail (block offsets and
//! per-file positions) and a 16 byte trailer (tail size, tail offset, `SNBP000B`).
//!
//! `book.snbf` holds the metadata, `toc.snbf` lists the chapters (`<chapter src="x.snbc">`),
//! every `.snbc` has `<text>` paragraphs and `<img>` references.

use std::cell::RefCell;
use std::io::Read;

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::Metadata;
use bookx_core::util::xml_escape;
use flate2::read::ZlibDecoder;

use crate::common::*;

/// Magic at the start and the end of the file.
pub const MAGIC: &[u8; 8] = b"SNBP000B";
const BLOCK: usize = 0x8000;
const MAX_FILES: usize = 100_000;
const MAX_STREAM: usize = 256 << 20;
const ATTR_PLAIN: u32 = 0x4100_0000;

/// Reader for `.snb` files.
pub struct SnbReader;

impl FormatReader for SnbReader {
    fn id(&self) -> &'static str {
        "snb"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["snb"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(MAGIC)
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_snb(bytes)
    }
}

fn be32(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn inflate(data: &[u8], max: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ZlibDecoder::new(data)
        .take(max as u64 + 1)
        .read_to_end(&mut out)
        .context("corrupt zlib data")?;
    if out.len() > max {
        bail!("zlib data expands to more than {max} bytes");
    }
    Ok(out)
}

/// Decompresses one bzip2 stream with an output limit.
pub fn bunzip(data: &[u8], max: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    bzip2_rs::DecoderReader::new(data)
        .take(max as u64 + 1)
        .read_to_end(&mut out)
        .context("corrupt bzip2 data")?;
    if out.len() > max {
        bail!("bzip2 data expands to more than {max} bytes");
    }
    Ok(out)
}

/// A file inside the container.
#[derive(Debug, Clone)]
pub struct SnbFile {
    pub name: String,
    pub data: Vec<u8>,
}

/// Reads every file of the container.
pub fn read_container(bytes: &[u8]) -> Result<Vec<SnbFile>> {
    if !bytes.starts_with(MAGIC) {
        bail!("not an SNB file: the `SNBP000B` magic is missing");
    }
    if bytes.len() < 44 + 16 {
        bail!("the SNB file is too short");
    }
    let field = |i: usize| be32(bytes, 8 + i * 4).unwrap_or(0) as usize;
    let file_count = field(3);
    let vfat_size = field(4);
    let vfat_compressed = field(5);
    let bin_size = field(6);
    let plain_size = field(7);
    if file_count == 0 || file_count > MAX_FILES {
        bail!("the SNB file table lists {file_count} files");
    }
    if vfat_size > MAX_STREAM || bin_size > MAX_STREAM || plain_size > MAX_STREAM {
        bail!("the SNB file declares streams that are too large");
    }
    let vfat_raw = bytes
        .get(44..44usize.saturating_add(vfat_compressed))
        .context("the SNB file table lies beyond the end of the file")?;
    let vfat = inflate(vfat_raw, vfat_size.max(1))?;
    if vfat.len() != vfat_size {
        bail!(
            "the SNB file table has {} bytes, expected {vfat_size}",
            vfat.len()
        );
    }
    if vfat.len() < file_count * 12 {
        bail!("the SNB file table is too short for {file_count} files");
    }
    let names = &vfat[file_count * 12..];

    let trailer = &bytes[bytes.len() - 16..];
    if &trailer[8..] != MAGIC {
        bail!("the SNB trailer magic is missing (truncated file)");
    }
    let tail_size = be32(trailer, 0).unwrap_or(0) as usize;
    let tail_offset = be32(trailer, 4).unwrap_or(0) as usize;
    let tail_raw = bytes
        .get(tail_offset..tail_offset.saturating_add(tail_size))
        .context("the SNB tail lies beyond the end of the file")?;
    let bin_blocks = bin_size.div_ceil(BLOCK);
    let plain_blocks = plain_size.div_ceil(BLOCK);
    let tail = inflate(
        tail_raw,
        (bin_blocks + plain_blocks) * 4 + file_count * 8 + 16,
    )?;
    if tail.len() < (bin_blocks + plain_blocks) * 4 {
        bail!("the SNB tail is too short");
    }
    let block_offset = |i: usize| be32(&tail, i * 4).unwrap_or(0) as usize;

    // Plain stream: independent bzip2 blocks.
    let mut plain = Vec::with_capacity(plain_size);
    for i in 0..plain_blocks {
        let start = block_offset(bin_blocks + i);
        let end = if i + 1 < plain_blocks {
            block_offset(bin_blocks + i + 1)
        } else {
            tail_offset
        };
        let raw = bytes
            .get(start..end)
            .filter(|_| start <= end)
            .with_context(|| format!("plain block {i} lies outside the file"))?;
        let part = if raw.starts_with(b"BZh") {
            bunzip(raw, BLOCK * 4)?
        } else {
            raw.to_vec()
        };
        plain.extend_from_slice(&part);
        if plain.len() > plain_size {
            bail!("the SNB plain stream is longer than declared");
        }
    }
    if plain.len() != plain_size {
        bail!(
            "the SNB plain stream has {} bytes, expected {plain_size}",
            plain.len()
        );
    }

    let bin_start = 44 + vfat_compressed;
    let mut bin_pos = 0usize;
    let mut plain_pos = 0usize;
    let mut files = Vec::with_capacity(file_count);
    for i in 0..file_count {
        let attr = be32(&vfat, i * 12).unwrap_or(0);
        let name_off = be32(&vfat, i * 12 + 4).unwrap_or(0) as usize;
        let size = be32(&vfat, i * 12 + 8).unwrap_or(0) as usize;
        let name_bytes = names.get(name_off..).unwrap_or(&[]);
        let end = name_bytes
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(name_bytes.len());
        let name = String::from_utf8_lossy(&name_bytes[..end]).replace('\\', "/");
        let data = if attr & ATTR_PLAIN == ATTR_PLAIN {
            let d = plain
                .get(plain_pos..plain_pos.saturating_add(size))
                .with_context(|| format!("file `{name}` exceeds the plain stream"))?;
            plain_pos += size;
            d.to_vec()
        } else if attr & 0x0100_0000 != 0 {
            let at = bin_start.saturating_add(bin_pos);
            let d = bytes
                .get(at..at.saturating_add(size))
                .with_context(|| format!("file `{name}` exceeds the binary stream"))?;
            bin_pos += size;
            d.to_vec()
        } else {
            bail!("file `{name}` has an unknown attribute {attr:#x}");
        };
        files.push(SnbFile { name, data });
    }
    Ok(files)
}

fn find<'a>(files: &'a [SnbFile], base: &str) -> Option<&'a SnbFile> {
    files.iter().find(|f| {
        f.name
            .rsplit('/')
            .next()
            .is_some_and(|b| b.eq_ignore_ascii_case(base))
    })
}

fn xml_doc(data: &[u8]) -> Result<String> {
    Ok(decode_xml(data).trim_start_matches('\u{feff}').to_string())
}

fn child_text(parent: roxmltree::Node, name: &str) -> String {
    child_el(parent, name)
        .map(|n| node_text(n).trim().to_string())
        .unwrap_or_default()
}

/// Converts an SNB file.
pub fn convert_snb(bytes: &[u8]) -> Result<Archive> {
    let files = read_container(bytes)?;

    let mut md = Metadata::default();
    let mut cover_name = String::new();
    if let Some(f) = find(&files, "book.snbf") {
        let text = xml_doc(&f.data)?;
        let doc = roxmltree::Document::parse(&text).context("book.snbf is not valid XML")?;
        if let Some(head) = doc.descendants().find(|n| n.has_tag_name("head")) {
            md.title = child_text(head, "name");
            let author = child_text(head, "author");
            if !author.is_empty() {
                md.authors = split_keywords(&author.replace(" & ", ","))
                    .into_iter()
                    .collect();
            }
            md.language = child_text(head, "language").to_ascii_lowercase();
            md.publisher = child_text(head, "publisher");
            md.rights = child_text(head, "rights");
            md.description = child_text(head, "abstract");
            cover_name = child_text(head, "cover");
        }
    }

    let media = RefCell::new(MediaSet::default());
    for f in &files {
        if image_ext(&f.data).is_some() {
            let base = f
                .name
                .rsplit('/')
                .next()
                .unwrap_or(&f.name)
                .to_ascii_lowercase();
            media.borrow_mut().add(&base, f.data.clone());
        }
    }

    // Chapters in table of contents order; without a toc, every .snbc in file order.
    let mut order: Vec<(String, String)> = Vec::new();
    if let Some(f) = find(&files, "toc.snbf") {
        let text = xml_doc(&f.data)?;
        let doc = roxmltree::Document::parse(&text).context("toc.snbf is not valid XML")?;
        for n in doc.descendants().filter(|n| n.has_tag_name("chapter")) {
            if let Some(src) = attr(n, "src") {
                order.push((src.to_string(), node_text(n).trim().to_string()));
            }
        }
    }
    if order.is_empty() {
        for f in files
            .iter()
            .filter(|f| f.name.to_ascii_lowercase().ends_with(".snbc"))
        {
            order.push((f.name.clone(), String::new()));
        }
    }

    let mut chapters = Vec::new();
    for (src, title) in order {
        let base = src.rsplit('/').next().unwrap_or(&src);
        let Some(f) = find(&files, base) else {
            continue;
        };
        let text = xml_doc(&f.data)?;
        let doc = roxmltree::Document::parse(&text)
            .with_context(|| format!("`{}` is not valid XML", f.name))?;
        let mut body = String::new();
        let mut chapter_title = title;
        if let Some(b) = doc.descendants().find(|n| n.has_tag_name("body")) {
            for n in b.children().filter(|n| n.is_element()) {
                match n.tag_name().name() {
                    "text" => {
                        let t = node_text(n);
                        let t = t.trim();
                        if !t.is_empty() {
                            body.push_str(&format!("<p>{}</p>\n", xml_escape(t)));
                        }
                    }
                    "img" => {
                        let name = node_text(n).trim().to_ascii_lowercase();
                        let name = name.rsplit('/').next().unwrap_or("").to_string();
                        if let Some(p) = media.borrow().path_of(&name) {
                            body.push_str(&format!(
                                "<div class=\"image\"><img src=\"{p}\" alt=\"\"/></div>\n"
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        if chapter_title.is_empty() {
            chapter_title = doc
                .descendants()
                .find(|n| n.has_tag_name("head"))
                .map(|h| child_text(h, "name"))
                .unwrap_or_default();
        }
        if chapter_title.is_empty() {
            chapter_title = format!("Chapter {}", chapters.len() + 1);
        }
        if visible_text_len(&body) > 0 || body.contains("<img") {
            chapters.push(Chapter {
                title: chapter_title,
                body,
            });
        }
    }
    if chapters.is_empty() {
        bail!("the SNB file has no readable chapters");
    }
    if md.title.is_empty() {
        md.title = "Untitled".into();
    }
    let cover = (!cover_name.is_empty())
        .then(|| find(&files, &cover_name))
        .flatten()
        .and_then(|f| image_ext(&f.data).map(|e| (f.data.clone(), e.to_string())));
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

    const BOOK: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><book-snbf version=\"1.0\"><head><name>SNB Book</name><author>Sam Ple</author><language>ZH-CN</language><rights>R</rights><publisher>Shanda</publisher><generator>x</generator><created>2010</created><abstract>About</abstract><cover>cover.png</cover></head></book-snbf>";
    const TOC: &str = "<toc-snbf><head><chapters>2</chapters></head><body><chapter src=\"ch2.snbc\">Second</chapter><chapter src=\"ch1.snbc\">First</chapter></body></toc-snbf>";

    fn sample() -> Vec<u8> {
        write_snb(&[
            ("snbf/book.snbf", BOOK.as_bytes().to_vec(), true),
            ("snbf/toc.snbf", TOC.as_bytes().to_vec(), true),
            ("snbc/ch1.snbc", "<snbc><head><name>One</name></head><body><text>\u{3000}Hello snb.</text><img>images/pic.png</img><text>  </text><text>Two &amp; more</text></body></snbc>".as_bytes().to_vec(), true),
            ("snbc/ch2.snbc", "<snbc><head><name>Ch2</name></head><body><text>Chapter two text.</text></body></snbc>".as_bytes().to_vec(), true),
            ("snbc/images/pic.png", PNG.to_vec(), false),
            ("snbc/images/cover.png", PNG.to_vec(), false),
        ])
    }

    #[test]
    fn container_roundtrip_returns_every_file() {
        let files = read_container(&sample()).unwrap();
        assert_eq!(files.len(), 6);
        let n: Vec<_> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(n.contains(&"snbf/book.snbf") && n.contains(&"snbc/images/pic.png"));
        let pic = files.iter().find(|f| f.name.ends_with("pic.png")).unwrap();
        assert_eq!(pic.data, PNG);
        let book = files
            .iter()
            .find(|f| f.name.ends_with("book.snbf"))
            .unwrap();
        assert_eq!(book.data, BOOK.as_bytes());
    }

    #[test]
    fn converts_in_toc_order_with_metadata_images_and_cover() {
        let arc = SnbReader.read(&sample()).unwrap();
        let book = validate(&arc);
        let md = book.metadata();
        assert_eq!(md.title, "SNB Book");
        assert_eq!(md.authors, ["Sam Ple"]);
        assert_eq!(md.language, "zh-cn");
        assert_eq!(md.publisher, "Shanda");
        assert_eq!(md.description, "About");
        assert_eq!(toc_titles(&book), ["Second", "First"]);
        let t = chapter_texts(&book);
        assert!(t[0].contains("Chapter two text."));
        assert!(t[1].contains("Hello snb."));
        assert!(t[1].contains("Two & more"));
        assert!(chapter_xml(&book, 1).contains("<img src=\"images/pic.png\""));
        assert!(arc.to_bytes().unwrap().windows(5).any(|w| w == b"cover"));
    }

    #[test]
    fn large_plain_stream_uses_several_bzip2_blocks() {
        let para = "<text>The quick brown fox jumps over the lazy dog.</text>";
        let body = para.repeat(2000);
        let ch = format!("<snbc><head><name>Big</name></head><body>{body}</body></snbc>");
        assert!(ch.len() > 2 * BLOCK);
        let f = write_snb(&[
            (
                "book.snbf",
                b"<book-snbf><head><name>Big</name></head></book-snbf>".to_vec(),
                true,
            ),
            ("ch.snbc", ch.into_bytes(), true),
        ]);
        let book = validate(&SnbReader.read(&f).unwrap());
        assert_eq!(chapter_xml(&book, 0).matches("<p>").count(), 2000);
    }

    #[test]
    fn without_toc_chapters_come_from_snbc_files() {
        let f = write_snb(&[(
            "a.snbc",
            b"<snbc><head><name>Solo</name></head><body><text>Only text</text></body></snbc>"
                .to_vec(),
            true,
        )]);
        let book = validate(&SnbReader.read(&f).unwrap());
        assert_eq!(toc_titles(&book), ["Solo"]);
        assert_eq!(book.metadata().title, "Untitled");
    }

    #[test]
    fn errors_are_specific() {
        assert!(format!("{}", read_container(b"nope").unwrap_err()).contains("magic is missing"));
        let mut short = MAGIC.to_vec();
        short.extend([0; 20]);
        assert!(format!("{}", read_container(&short).unwrap_err()).contains("too short"));
        let f = sample();
        let cut = &f[..f.len() - 20];
        assert!(read_container(cut).is_err(), "truncated trailer");
        let mut f = sample();
        let n = f.len();
        f[n - 8..].copy_from_slice(b"XXXXXXXX");
        assert!(format!("{}", read_container(&f).unwrap_err()).contains("trailer magic"));
        let mut f = sample();
        f[20..24].copy_from_slice(&0u32.to_be_bytes());
        assert!(format!("{}", read_container(&f).unwrap_err()).contains("lists 0 files"));
        let mut f = sample();
        f[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(read_container(&f).is_err());
        let mut f = sample();
        f[24..28].copy_from_slice(&u32::MAX.to_be_bytes());
        let m = format!("{}", read_container(&f).unwrap_err());
        assert!(m.contains("too large"), "{m}");
    }

    #[test]
    fn invalid_xml_and_empty_books_are_errors() {
        let f = write_snb(&[("book.snbf", b"<book-snbf><head>".to_vec(), true)]);
        assert!(format!("{:#}", SnbReader.read(&f).unwrap_err()).contains("book.snbf"));
        let f = write_snb(&[("book.snbf", BOOK.as_bytes().to_vec(), true)]);
        assert!(format!("{}", SnbReader.read(&f).unwrap_err()).contains("no readable chapters"));
        let f = write_snb(&[
            ("toc.snbf", TOC.as_bytes().to_vec(), true),
            (
                "ch1.snbc",
                b"<snbc><body><text> </text></body></snbc>".to_vec(),
                true,
            ),
        ]);
        assert!(SnbReader.read(&f).is_err());
    }

    #[test]
    fn bunzip_known_vector_and_limits() {
        // Produced by the `bzip2 -9` command line tool from "hello hello hello\n".
        let data = bzip2_cli_fixture();
        assert_eq!(bunzip(&data.0, 1000).unwrap(), data.1);
        assert!(format!("{}", bunzip(&data.0, 3).unwrap_err()).contains("expands to more than"));
        assert!(bunzip(b"BZh9garbage", 100).is_err());
    }

    #[test]
    fn ids_extensions_sniff_and_fuzz() {
        assert_eq!(SnbReader.id(), "snb");
        assert_eq!(SnbReader.extensions(), &["snb"]);
        assert!(SnbReader.sniff(b"SNBP000B...."));
        assert!(!SnbReader.sniff(b"SNBP000"));
        fuzz_no_panic(&SnbReader, &sample());
        assert_rejects_garbage(&SnbReader);
    }
}
