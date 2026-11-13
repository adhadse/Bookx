//! Mobipocket / Kindle readers: MOBI, AZW, AZW3 (KF8), PRC and plain PalmDoc.
//!
//! Implemented from the public format descriptions: PalmDB header, PalmDoc and MOBI headers,
//! EXTH metadata, PalmDoc LZ77 decompression, trailing entry stripping, image records, the KF8
//! boundary record of "joint" files and the FDST flow table.
//!
//! Not supported (documented, tested to fail with a clear message where it is a hard error):
//! * HUFF/CDIC compression (error), DRM / encrypted books (error).
//! * MOBI 6 `INDX` / NCX tables: the table of contents is built from the page breaks and
//!   headings of the text instead of the book's own NCX.
//! * KF8 `SKEL` / `FRAG` / `NCX` tables: the text of flow 0 is read in reading order (every
//!   skeleton is followed by its fragments, so wrapper elements of fragments are lost and
//!   internal `kindle:pos:` links are dropped). CSS / SVG flows, fonts and audio are ignored.
//! * Index based metadata such as series, and the thumbnail cover record.

use std::cell::RefCell;

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::{Identifier, Metadata};
use bookx_core::toc::{write_toc, TocEntry};
use bookx_core::util::xml_escape;
use regex::{Captures, Regex};

use crate::common::*;
use crate::mobi_index::{self, TocNode};
use crate::mobi_kf8;

/// Largest decompressed size of one text record.
const MAX_RECORD_OUT: usize = 8 << 20;
/// Largest total text.
const MAX_TEXT: usize = 256 << 20;

/// Reader for MOBI, AZW, AZW3 and PRC files.
pub struct MobiReader;

impl FormatReader for MobiReader {
    fn id(&self) -> &'static str {
        "mobi"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["mobi", "azw", "azw3", "prc"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.len() >= 68 && (&head[60..68] == b"BOOKMOBI" || &head[60..68] == b"TEXtREAd")
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_mobi(bytes)
    }
}

// ---------------------------------------------------------------------------
// Binary helpers
// ---------------------------------------------------------------------------

fn be16(b: &[u8], off: usize) -> Option<u16> {
    b.get(off..off + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
}

fn be32(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn be32_or0(b: &[u8], off: usize) -> u32 {
    be32(b, off).unwrap_or(0)
}

/// The records of a PalmDB file.
struct Pdb<'a> {
    data: &'a [u8],
    offsets: Vec<usize>,
}

impl<'a> Pdb<'a> {
    fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < 78 {
            bail!("file is too small to be a PalmDB / MOBI book");
        }
        let n = be16(data, 76).unwrap_or(0) as usize;
        if n == 0 {
            bail!("the PalmDB file has no records");
        }
        let mut offsets = Vec::with_capacity(n);
        for i in 0..n {
            let off =
                be32(data, 78 + i * 8).context("the PalmDB record list is truncated")? as usize;
            offsets.push(off);
        }
        if offsets.iter().any(|o| *o > data.len()) {
            bail!("a record points beyond the end of the file (truncated file)");
        }
        if offsets.windows(2).any(|w| w[0] > w[1]) {
            bail!("the PalmDB record offsets are not ascending");
        }
        Ok(Pdb { data, offsets })
    }

    fn len(&self) -> usize {
        self.offsets.len()
    }

    fn record(&self, i: usize) -> Result<&'a [u8]> {
        let start = *self
            .offsets
            .get(i)
            .with_context(|| format!("record {i} does not exist"))?;
        let end = self.offsets.get(i + 1).copied().unwrap_or(self.data.len());
        Ok(&self.data[start..end])
    }
}

/// Fields of the PalmDoc + MOBI header found in a book's first record.
#[derive(Debug, Default, Clone)]
struct Header {
    compression: u16,
    text_length: u32,
    record_count: u16,
    encryption: u16,
    has_mobi: bool,
    header_len: u32,
    encoding: u32,
    version: u32,
    full_name_offset: u32,
    full_name_len: u32,
    locale: u32,
    first_image: u32,
    exth_flags: u32,
    drm_offset: u32,
    extra_flags: u32,
    fdst_index: u32,
    ncx_index: u32,
    frag_index: u32,
    skel_index: u32,
}

fn parse_header(rec0: &[u8]) -> Result<Header> {
    if rec0.len() < 16 {
        bail!("the first record is too small");
    }
    let mut h = Header {
        compression: be16(rec0, 0).unwrap_or(0),
        text_length: be32_or0(rec0, 4),
        record_count: be16(rec0, 8).unwrap_or(0),
        encryption: be16(rec0, 12).unwrap_or(0),
        drm_offset: 0xFFFF_FFFF,
        ..Default::default()
    };
    if rec0.get(16..20) == Some(b"MOBI") {
        h.has_mobi = true;
        let m = 16;
        h.header_len = be32_or0(rec0, m + 4);
        h.encoding = be32_or0(rec0, m + 12);
        h.version = be32_or0(rec0, m + 20);
        h.full_name_offset = be32_or0(rec0, m + 0x44);
        h.full_name_len = be32_or0(rec0, m + 0x48);
        h.locale = be32_or0(rec0, m + 0x4C);
        h.first_image = be32(rec0, m + 0x5C).unwrap_or(0xFFFF_FFFF);
        h.exth_flags = be32_or0(rec0, m + 0x70);
        h.drm_offset = be32(rec0, m + 0x98).unwrap_or(0xFFFF_FFFF);
        if h.header_len >= 0xE4 {
            h.extra_flags = be32_or0(rec0, m + 0xE0);
        }
        if h.version >= 8 {
            h.fdst_index = be32(rec0, 0xC0).unwrap_or(0xFFFF_FFFF);
        } else {
            h.fdst_index = 0xFFFF_FFFF;
        }
        // index records (NCX / fragments / skeletons); only present in headers long enough
        let idx = |off: usize| {
            if h.header_len as usize + m >= off + 4 {
                be32(rec0, off).unwrap_or(0xFFFF_FFFF)
            } else {
                0xFFFF_FFFF
            }
        };
        h.ncx_index = idx(0xF4);
        h.frag_index = if h.version >= 8 {
            idx(0xF8)
        } else {
            0xFFFF_FFFF
        };
        h.skel_index = if h.version >= 8 {
            idx(0xFC)
        } else {
            0xFFFF_FFFF
        };
    }
    Ok(h)
}

/// EXTH records: `(type, data)`.
fn parse_exth(rec0: &[u8], h: &Header) -> Vec<(u32, Vec<u8>)> {
    let mut out = Vec::new();
    if !h.has_mobi || h.exth_flags & 0x40 == 0 {
        return out;
    }
    let start = 16 + h.header_len as usize;
    if rec0.get(start..start + 4) != Some(b"EXTH") {
        return out;
    }
    let count = be32_or0(rec0, start + 8) as usize;
    let mut pos = start + 12;
    for _ in 0..count.min(2000) {
        let (Some(t), Some(len)) = (be32(rec0, pos), be32(rec0, pos + 4)) else {
            break;
        };
        let len = len as usize;
        if len < 8 {
            break;
        }
        let Some(data) = rec0.get(pos + 8..pos + len) else {
            break;
        };
        out.push((t, data.to_vec()));
        pos += len;
    }
    out
}

fn decode_with(enc: u32, bytes: &[u8]) -> String {
    if enc == 65001 {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0
            .into_owned()
    }
}

// ---------------------------------------------------------------------------
// PalmDoc compression and trailing entries
// ---------------------------------------------------------------------------

/// PalmDoc LZ77 decompression.
///
/// Byte `0x00` and `0x09..=0x7F` are literals; `0x01..=0x08` introduces that many literal
/// bytes; `0x80..=0xBF` starts a two byte (distance, length) pair; `0xC0..=0xFF` is a space
/// followed by the byte with the high bit cleared.
pub fn palmdoc_decompress(data: &[u8], max_out: usize) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(data.len() * 2);
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        i += 1;
        match c {
            0x00 | 0x09..=0x7F => out.push(c),
            0x01..=0x08 => {
                let n = c as usize;
                let Some(lit) = data.get(i..i + n) else {
                    bail!("PalmDoc data ends inside a literal run");
                };
                out.extend_from_slice(lit);
                i += n;
            }
            0x80..=0xBF => {
                let Some(&d) = data.get(i) else {
                    bail!("PalmDoc data ends inside a length/distance pair");
                };
                i += 1;
                let pair = ((c as u16) << 8) | d as u16;
                let dist = ((pair >> 3) & 0x7FF) as usize;
                let len = (pair & 7) as usize + 3;
                if dist == 0 || dist > out.len() {
                    bail!("PalmDoc back reference points before the start of the text");
                }
                for _ in 0..len {
                    out.push(out[out.len() - dist]);
                }
            }
            0xC0..=0xFF => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
        }
        if out.len() > max_out {
            bail!("a PalmDoc record decompresses to more than {max_out} bytes");
        }
    }
    Ok(out)
}

/// Size of one trailing entry whose last byte is at `data[end - 1]` (backward varint).
fn trailing_entry_size(data: &[u8], end: usize) -> usize {
    let mut size = end;
    let mut result = 0usize;
    let mut shift = 0;
    loop {
        if size == 0 {
            return result;
        }
        let v = data[size - 1];
        result |= ((v & 0x7F) as usize) << shift;
        shift += 7;
        size -= 1;
        if v & 0x80 != 0 || shift >= 28 || size == 0 {
            return result;
        }
    }
}

/// Removes the trailing entries announced by the MOBI "extra data flags".
fn strip_trailing(rec: &[u8], flags: u32) -> &[u8] {
    let mut num = 0usize;
    let mut f = flags >> 1;
    while f != 0 {
        if f & 1 != 0 {
            let end = rec.len().saturating_sub(num);
            num += trailing_entry_size(rec, end);
        }
        f >>= 1;
    }
    if flags & 1 != 0 {
        let idx = rec.len().saturating_sub(num);
        if idx > 0 {
            num += (rec[idx - 1] & 3) as usize + 1;
        }
    }
    &rec[..rec.len().saturating_sub(num)]
}

fn extract_text(pdb: &Pdb, first: usize, h: &Header) -> Result<Vec<u8>> {
    match h.compression {
        1 | 2 => {}
        17480 => bail!("this MOBI uses HUFF/CDIC compression, which is not supported"),
        c => bail!("unknown MOBI compression type {c}"),
    }
    if h.encryption != 0 {
        bail!("this book is DRM protected (encrypted) and cannot be read");
    }
    if h.drm_offset != 0xFFFF_FFFF {
        bail!("this book is DRM protected and cannot be read");
    }
    let mut text = Vec::new();
    for i in 1..=h.record_count as usize {
        let rec = pdb
            .record(first + i)
            .with_context(|| format!("text record {i} is missing (truncated file)"))?;
        let rec = strip_trailing(rec, h.extra_flags);
        if h.compression == 2 {
            text.extend(palmdoc_decompress(rec, MAX_RECORD_OUT)?);
        } else {
            text.extend_from_slice(rec);
        }
        if text.len() > MAX_TEXT {
            bail!("the book text is too large");
        }
    }
    if h.text_length > 0 && text.len() > h.text_length as usize {
        text.truncate(h.text_length as usize);
    }
    Ok(text)
}

// ---------------------------------------------------------------------------
// Text to chapters
// ---------------------------------------------------------------------------

fn base32_index(s: &str) -> Option<usize> {
    let mut v = 0usize;
    if s.is_empty() || s.len() > 6 {
        return None;
    }
    for c in s.chars() {
        let d = c.to_digit(32)? as usize;
        v = v * 32 + d;
    }
    Some(v)
}

/// Inserts `<a id="fileposN">` anchors at the targets of `filepos` links and rewrites those
/// links to `href="#fileposN"`.
fn resolve_filepos(text: Vec<u8>, extra: &[usize]) -> Vec<u8> {
    let Ok(re) = regex::bytes::Regex::new(r#"(?i)filepos\s*=\s*["']?0*(\d+)["']?"#) else {
        return text;
    };
    let mut targets: Vec<usize> = re
        .captures_iter(&text)
        .filter_map(|c| std::str::from_utf8(&c[1]).ok()?.parse::<usize>().ok())
        .chain(extra.iter().copied())
        .filter(|&t| t < text.len() && text[t] == b'<')
        .collect();
    targets.sort_unstable();
    targets.dedup();
    let mut out = Vec::with_capacity(text.len() + targets.len() * 24);
    let mut last = 0;
    for t in targets {
        out.extend_from_slice(&text[last..t]);
        out.extend_from_slice(format!("<a id=\"filepos{t}\"></a>").as_bytes());
        last = t;
    }
    out.extend_from_slice(&text[last..]);
    // Anchors shifted the offsets only for the *output*; links name the original offsets.
    re.replace_all(&out, |c: &regex::bytes::Captures| {
        let n = std::str::from_utf8(&c[1])
            .ok()
            .and_then(|s| s.parse::<usize>().ok());
        match n {
            Some(n) => format!("href=\"#filepos{n}\"").into_bytes(),
            None => Vec::new(),
        }
    })
    .into_owned()
}

fn split_sections(html: &str, kf8: bool) -> Vec<String> {
    let re = if kf8 {
        Regex::new(r"(?i)<html[\s>]").expect("static regex")
    } else {
        Regex::new(r"(?i)<mbp:pagebreak\s*/?>").expect("static regex")
    };
    if kf8 {
        let starts: Vec<usize> = re.find_iter(html).map(|m| m.start()).collect();
        if starts.len() <= 1 {
            return vec![html.to_string()];
        }
        let mut out = Vec::new();
        let mut prev = 0;
        for s in starts.iter().skip(1) {
            out.push(html[prev..*s].to_string());
            prev = *s;
        }
        out.push(html[prev..].to_string());
        out
    } else {
        re.split(html).map(str::to_string).collect()
    }
}

fn section_title(blocks: &[String], fallback: usize) -> String {
    for b in blocks {
        if b.len() > 2 && b.starts_with("<h") && b.as_bytes()[2].is_ascii_digit() {
            let t = strip_tags(b);
            if !t.is_empty() {
                return t;
            }
        }
    }
    for b in blocks.iter().take(3) {
        let t = strip_tags(b);
        if !t.is_empty() && t.chars().count() <= 60 {
            return t;
        }
    }
    format!("Section {fallback}")
}

fn build_chapters(
    html: &str,
    kf8: bool,
    rewrite: &dyn Fn(&str) -> Option<String>,
    default_title: &str,
) -> Vec<Chapter> {
    let sections: Vec<Vec<String>> = split_sections(html, kf8)
        .iter()
        .map(|s| html_blocks(s, rewrite))
        .filter(|b| {
            !b.is_empty()
                && (b.iter().map(|x| visible_text_len(x)).sum::<usize>() > 0
                    || b.iter().any(|x| x.contains("<img")))
        })
        .collect();
    if sections.len() <= 1 {
        return chapters_from_blocks(
            sections.into_iter().next().unwrap_or_default(),
            default_title,
        );
    }
    sections
        .into_iter()
        .enumerate()
        .map(|(i, blocks)| Chapter {
            title: section_title(&blocks, i + 1),
            body: blocks.concat(),
        })
        .collect()
}

const IMG_FALLBACK: &str = r#"<img[^>]*src="(?:mobi-img-\d+|kindle:[^"]*)"[^>]*/>"#;

fn locale_language(locale: u32) -> &'static str {
    match locale & 0xFF {
        9 => "en",
        4 => "zh",
        5 => "cs",
        6 => "da",
        7 => "de",
        8 => "el",
        10 => "es",
        11 => "fi",
        12 => "fr",
        14 => "hu",
        16 => "it",
        17 => "ja",
        18 => "ko",
        19 => "nl",
        20 => "no",
        21 => "pl",
        22 => "pt",
        25 => "ru",
        29 => "sv",
        31 => "tr",
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

fn image_record_ok(pdb: &Pdb, idx: usize) -> bool {
    pdb.record(idx).is_ok_and(|r| image_ext(r).is_some())
}

/// Finds the record number of resource 1 (images are numbered from there).
fn resource_base(pdb: &Pdb, h: &Header, kf8_start: usize, text_end: usize) -> Option<usize> {
    let first = h.first_image as usize;
    for cand in [first, first + kf8_start] {
        if h.first_image != 0xFFFF_FFFF && cand < pdb.len() && image_record_ok(pdb, cand) {
            return Some(cand);
        }
    }
    (text_end..pdb.len()).find(|i| image_record_ok(pdb, *i))
}

/// The `(start, end)` text ranges of the flows listed in the FDST record.
fn read_fdst_flows(
    pdb: &Pdb,
    h: &Header,
    kf8_start: usize,
    text_len: usize,
) -> Option<Vec<(usize, usize)>> {
    if h.fdst_index == 0xFFFF_FFFF {
        return None;
    }
    let idx = h.fdst_index as usize;
    let rec = [idx.checked_add(kf8_start)?, idx]
        .into_iter()
        .filter_map(|i| pdb.record(i).ok())
        .find(|r| r.get(0..4) == Some(b"FDST"))?;
    let off = be32(rec, 4)? as usize;
    let count = (be32(rec, 8)? as usize).min(4096);
    let mut flows = Vec::new();
    for i in 0..count {
        let s = be32(rec, off.checked_add(i * 8)?)? as usize;
        let e = be32(rec, off.checked_add(i * 8 + 4)?)? as usize;
        if s <= e && e <= text_len {
            flows.push((s, e));
        } else {
            break;
        }
    }
    (!flows.is_empty()).then_some(flows)
}

fn plain_palmdoc(pdb: &Pdb, h: &Header) -> Result<Archive> {
    let text = extract_text(pdb, 0, h)?;
    let s = decode_text(&text);
    let mut body = String::new();
    for line in s.lines().map(str::trim).filter(|l| !l.is_empty()) {
        body.push_str(&format!("<p>{}</p>\n", xml_escape(line)));
    }
    if body.is_empty() {
        bail!("the PalmDoc file has no text");
    }
    let name = String::from_utf8_lossy(pdb.data.get(0..32).unwrap_or(&[]))
        .split('\0')
        .next()
        .unwrap_or("")
        .replace('_', " ")
        .trim()
        .to_string();
    build(Parts {
        md: Metadata {
            title: name.clone(),
            ..Default::default()
        },
        chapters: vec![Chapter {
            title: if name.is_empty() { "Text".into() } else { name },
            body,
        }],
        ..Default::default()
    })
}

/// Converts MOBI / AZW / AZW3 / PRC bytes to an EPUB archive.
/// Title, authors and publisher of a MOBI-family file, without converting its text.
pub fn peek_metadata(bytes: &[u8]) -> Result<Metadata> {
    let pdb = Pdb::parse(bytes)?;
    let rec0 = pdb.record(0)?;
    let h = parse_header(rec0)?;
    if !h.has_mobi {
        bail!("not a MOBI book: the MOBI header is missing");
    }
    let exth = parse_exth(rec0, &h);
    let enc = h.encoding;
    let all = |t: u32| -> Vec<String> {
        exth.iter()
            .filter(|(k, _)| *k == t)
            .map(|(_, d)| decode_with(enc, d).trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let title = all(503).into_iter().next().unwrap_or_else(|| {
        let off = h.full_name_offset as usize;
        let len = (h.full_name_len as usize).min(1024);
        rec0.get(off..off.saturating_add(len))
            .map(|b| decode_with(enc, b).trim().to_string())
            .unwrap_or_default()
    });
    Ok(Metadata {
        title,
        authors: all(100),
        publisher: all(101).into_iter().next().unwrap_or_default(),
        ..Default::default()
    })
}

pub fn convert_mobi(bytes: &[u8]) -> Result<Archive> {
    let pdb = Pdb::parse(bytes)?;
    let mut rec0 = pdb.record(0)?;
    let mut h = parse_header(rec0)?;
    let mut exth = parse_exth(rec0, &h);
    let mut kf8_start = 0usize;
    let mut kf8 = false;

    if !h.has_mobi {
        if &bytes[60..68] == b"TEXtREAd" {
            return plain_palmdoc(&pdb, &h);
        }
        bail!("not a MOBI book: the MOBI header is missing");
    }

    // Joint MOBI 6 + KF8 files: prefer the KF8 half when it can be read.
    let boundary = exth
        .iter()
        .find(|(t, _)| *t == 121)
        .and_then(|(_, d)| be32(d, 0))
        .map(|v| v as usize);
    if let Some(b) = boundary.filter(|b| *b > 0 && *b < pdb.len()) {
        if let Ok(r) = pdb.record(b) {
            if let Ok(h8) = parse_header(r) {
                if h8.has_mobi && h8.version >= 8 {
                    let mut merged = parse_exth(r, &h8);
                    // The KF8 half may omit fields only the MOBI 6 half carries.
                    for e in exth.drain(..) {
                        if !merged.iter().any(|(t, _)| *t == e.0) {
                            merged.push(e);
                        }
                    }
                    exth = merged;
                    kf8_start = b;
                    h = h8;
                    rec0 = r;
                    kf8 = true;
                }
            }
        }
    } else if h.version >= 8 {
        kf8 = true;
    }
    let enc = h.encoding;

    let mut text = extract_text(&pdb, kf8_start, &h)?;
    let record = |i: usize| pdb.record(kf8_start + i).ok().map(|r| r.to_vec());
    let has = |v: u32| v != 0xFFFF_FFFF && v != 0;
    let ncx: Vec<mobi_index::NcxEntry> = if has(h.ncx_index) {
        mobi_index::read_index(&record, h.ncx_index as usize)
            .map(|ix| mobi_index::ncx_entries(&ix))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let flows = if kf8 {
        read_fdst_flows(&pdb, &h, kf8_start, text.len())
    } else {
        None
    };
    let mut css_text = String::new();
    let mut assembled = None;
    if kf8 {
        let flow0_end = flows.as_ref().map_or(text.len(), |f| f[0].1);
        if has(h.skel_index) && has(h.frag_index) {
            let skel = mobi_index::read_index(&record, h.skel_index as usize)
                .map(|ix| mobi_index::skeletons(&ix));
            let frag = mobi_index::read_index(&record, h.frag_index as usize)
                .map(|ix| mobi_index::fragments(&ix));
            if let (Some(skel), Some(frag)) = (skel, frag) {
                assembled = mobi_kf8::assemble(&text[..flow0_end], &skel, &frag);
            }
        }
        if let Some(flows) = &flows {
            for (s, e) in flows.iter().skip(1) {
                let f = decode_with(enc, &text[*s..*e]);
                if !f.trim_start().starts_with('<') {
                    css_text.push_str(&f);
                    css_text.push('\n');
                }
            }
        }
        text.truncate(flow0_end);
    } else {
        let extra: Vec<usize> = ncx
            .iter()
            .map(|e| e.pos)
            .filter(|p| *p != usize::MAX)
            .collect();
        text = resolve_filepos(text, &extra);
    }
    let html = decode_with(enc, &text);

    // Metadata.
    let get_all = |t: u32| -> Vec<String> {
        exth.iter()
            .filter(|(k, _)| *k == t)
            .map(|(_, d)| decode_with(enc, d).trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let first = |t: u32| get_all(t).into_iter().next();
    let title = first(503).unwrap_or_else(|| {
        let off = h.full_name_offset as usize;
        let len = (h.full_name_len as usize).min(1024);
        rec0.get(off..off.saturating_add(len))
            .map(|b| decode_with(enc, b).trim().to_string())
            .unwrap_or_default()
    });
    let mut md = Metadata {
        title,
        ..Default::default()
    };
    md.authors = get_all(100);
    md.publisher = first(101).unwrap_or_default();
    md.description = first(103)
        .map(|d| if d.contains('<') { strip_tags(&d) } else { d })
        .unwrap_or_default();
    md.tags = get_all(105)
        .into_iter()
        .flat_map(|s| {
            s.split(';')
                .map(|t| t.trim().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|t| !t.is_empty())
        .collect();
    md.published = first(106).unwrap_or_default();
    md.rights = first(109).unwrap_or_default();
    md.language = first(524).unwrap_or_else(|| locale_language(h.locale).to_string());
    if let Some(isbn) = first(104) {
        md.identifiers.push(Identifier {
            scheme: "isbn".into(),
            value: isbn,
            xml_id: None,
        });
    }
    if let Some(asin) = first(113) {
        md.identifiers.push(Identifier {
            scheme: "mobi-asin".into(),
            value: asin,
            xml_id: None,
        });
    }

    // Images.
    let text_end = kf8_start + 1 + h.record_count as usize;
    let base = resource_base(&pdb, &h, kf8_start, text_end);
    let media = RefCell::new(MediaSet::default());
    let rewrite = |src: &str| -> Option<String> {
        let base = base?;
        let n = if let Some(n) = src.strip_prefix("mobi-img-") {
            n.parse::<usize>().ok()?
        } else if let Some(rest) = src.strip_prefix("kindle:embed:") {
            base32_index(rest.split('?').next()?)?
        } else {
            return None;
        };
        let idx = base.checked_add(n.checked_sub(1)?)?;
        let rec = pdb.record(idx).ok()?;
        media
            .borrow_mut()
            .add(&format!("image-{n:04}"), rec.to_vec())
    };

    // Link and image attributes of MOBI 6 markup.
    let html = {
        let re = Regex::new(r#"(?i)recindex\s*=\s*["']?0*(\d+)["']?"#).expect("static regex");
        re.replace_all(&html, |c: &Captures| format!("src=\"mobi-img-{}\"", &c[1]))
            .into_owned()
    };
    let default_title = if md.title.is_empty() {
        "Beginning".to_string()
    } else {
        md.title.clone()
    };
    let mut toc_anchors: Vec<Option<String>> = vec![None; ncx.len()];
    let mut chapters = if let Some(mut asm) = assembled {
        let extra: Vec<(usize, usize)> = ncx.iter().filter_map(|e| e.pos_fid).collect();
        let names = mobi_kf8::resolve_links(&mut asm, &extra);
        for (i, e) in ncx.iter().enumerate() {
            toc_anchors[i] = e.pos_fid.and_then(|t| names.get(&t).cloned());
        }
        let mut out = Vec::new();
        for (i, part) in asm.parts.iter().enumerate() {
            let html = decode_with(enc, part);
            let blocks = html_blocks(&html, &rewrite);
            if blocks.is_empty()
                || (blocks.iter().map(|x| visible_text_len(x)).sum::<usize>() == 0
                    && !blocks.iter().any(|x| x.contains("<img")))
            {
                continue;
            }
            out.push(Chapter {
                title: section_title(&blocks, i + 1),
                body: blocks.concat(),
            });
        }
        out
    } else {
        for (i, e) in ncx.iter().enumerate() {
            if e.pos != usize::MAX {
                toc_anchors[i] = Some(format!("filepos{}", e.pos));
            }
        }
        build_chapters(&html, kf8, &rewrite, &default_title)
    };
    if let Ok(re) = Regex::new(IMG_FALLBACK) {
        for ch in &mut chapters {
            ch.body = re.replace_all(&ch.body, "").into_owned();
        }
    }
    chapters.retain(|c| visible_text_len(&c.body) > 0 || c.body.contains("<img"));
    if chapters.is_empty() {
        bail!("the book has no readable text");
    }

    let cover = exth
        .iter()
        .find(|(t, _)| *t == 201)
        .and_then(|(_, d)| be32(d, 0))
        .and_then(|n| base.and_then(|b| b.checked_add(n as usize)))
        .and_then(|idx| pdb.record(idx).ok())
        .and_then(|r| image_ext(r).map(|e| (r.to_vec(), e.to_string())));

    let css = if css_text.is_empty() {
        String::new()
    } else {
        rewrite_css_urls(&css_text, &rewrite)
    };
    let media = media.into_inner();
    let used: Vec<Media> = media
        .items
        .iter()
        .filter(|m| chapters.iter().any(|c| c.body.contains(&m.path)) || css.contains(&m.path))
        .cloned()
        .collect();
    let tree = mobi_index::toc_tree(&ncx);
    // the book's own (nested) table of contents replaces the one made from chapter titles
    let toc: Vec<TocEntry> = tree
        .iter()
        .flat_map(|n| toc_entry(n, &toc_anchors, &chapters))
        .collect();
    let mut arc = build(Parts {
        md,
        chapters,
        cover,
        media: used,
        css,
        ..Default::default()
    })?;
    if !toc.is_empty() {
        if let Ok(book) = OpenBook::from_archive(arc.clone()) {
            let pkg = book.package.clone();
            if write_toc(&mut arc, &pkg, &toc).is_err() {
                arc = book.archive;
            }
        }
    }
    Ok(arc)
}

/// Rewrites `url(kindle:embed:XXXX?mime=...)` references of a style sheet to extracted images.
fn rewrite_css_urls(css: &str, rewrite: &dyn Fn(&str) -> Option<String>) -> String {
    let Ok(re) = Regex::new(r#"url\(\s*["']?(kindle:embed:[^)"']+)["']?\s*\)"#) else {
        return css.to_string();
    };
    re.replace_all(css, |c: &Captures| match rewrite(&c[1]) {
        Some(p) => format!("url({p})"),
        None => "none".to_string(),
    })
    .into_owned()
}

fn toc_entry(node: &TocNode, anchors: &[Option<String>], chapters: &[Chapter]) -> Vec<TocEntry> {
    let children: Vec<TocEntry> = node
        .children
        .iter()
        .flat_map(|c| toc_entry(c, anchors, chapters))
        .collect();
    let target = anchors.get(node.entry).cloned().flatten().and_then(|a| {
        let needle = format!("id=\"{a}\"");
        chapters
            .iter()
            .position(|c| c.body.contains(&needle))
            .map(|i| format!("OEBPS/chapter-{}.xhtml#{a}", i + 1))
    });
    match target {
        Some(href) if !node.title.is_empty() => vec![TocEntry {
            title: node.title.clone(),
            href,
            children,
        }],
        _ => children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::mobi_writer::*;
    use crate::testutil::*;

    // --- PalmDoc decompression, expectations computed by hand ---------------------------------

    #[test]
    fn palmdoc_literals_and_runs() {
        // 0x09..0x7F and 0x00 are copied as they are.
        assert_eq!(palmdoc_decompress(&[0x61, 0x62, 0x63], 99).unwrap(), b"abc");
        assert_eq!(
            palmdoc_decompress(&[0x00, 0x09, 0x7F], 99).unwrap(),
            vec![0x00, 0x09, 0x7F]
        );
        // 0x01..0x08: copy the next n bytes verbatim, even high bytes.
        assert_eq!(palmdoc_decompress(&[0x01, 0xFF], 99).unwrap(), vec![0xFF]);
        assert_eq!(
            palmdoc_decompress(&[0x03, 0x80, 0x81, 0x82], 99).unwrap(),
            vec![0x80, 0x81, 0x82]
        );
        assert_eq!(
            palmdoc_decompress(&[0x08, 1, 2, 3, 4, 5, 6, 7, 8, 0x41], 99).unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 0x41]
        );
        assert_eq!(palmdoc_decompress(&[], 99).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn palmdoc_space_plus_char() {
        // 0xC1 = space + (0xC1 ^ 0x80 = 0x41 'A'); 0xFF = space + 0x7F.
        assert_eq!(palmdoc_decompress(&[0xC1], 99).unwrap(), b" A");
        assert_eq!(
            palmdoc_decompress(&[0x61, 0xE2, 0xC0], 99).unwrap(),
            b"a b @"
        );
        assert_eq!(palmdoc_decompress(&[0xFF], 99).unwrap(), vec![b' ', 0x7F]);
    }

    #[test]
    fn palmdoc_length_distance_pairs() {
        // "hello " + pair(distance 6, length 5): 0x8000 | (6 << 3) | (5 - 3) = 0x8032.
        let mut d = b"hello ".to_vec();
        d.extend([0x80, 0x32]);
        assert_eq!(palmdoc_decompress(&d, 99).unwrap(), b"hello hello");
        // overlapping copy: "abc" + pair(distance 3, length 6) = 0x8000 | 0x18 | 3 = 0x801B.
        assert_eq!(
            palmdoc_decompress(&[0x61, 0x62, 0x63, 0x80, 0x1B], 99).unwrap(),
            b"abcabcabc"
        );
        // minimum: distance 1 length 3 repeats the last byte: "x" + 0x8000 | 8 | 0 = 0x8008.
        assert_eq!(
            palmdoc_decompress(&[0x78, 0x80, 0x08], 99).unwrap(),
            b"xxxx"
        );
        // maximum distance field: 0xBFFF = distance 0x7FF, length 10.
        let mut d = vec![b'a'; 0x7FF];
        d.extend([0xBF, 0xFF]);
        assert_eq!(palmdoc_decompress(&d, 9999).unwrap().len(), 0x7FF + 10);
    }

    #[test]
    fn palmdoc_errors() {
        // pair before any output / distance zero
        assert!(palmdoc_decompress(&[0x80, 0x08], 99)
            .unwrap_err()
            .to_string()
            .contains("back reference"));
        assert!(palmdoc_decompress(&[0x61, 0x80, 0x00], 99).is_err());
        // pair too far back
        assert!(palmdoc_decompress(&[0x61, 0x80, 0x10], 99).is_err());
        // truncated pair and truncated literal run
        assert!(palmdoc_decompress(&[0x61, 0x80], 99)
            .unwrap_err()
            .to_string()
            .contains("pair"));
        assert!(palmdoc_decompress(&[0x05, 0x41], 99)
            .unwrap_err()
            .to_string()
            .contains("literal"));
        // output limit
        assert!(palmdoc_decompress(&[0x61, 0x80, 0x0B], 4)
            .unwrap_err()
            .to_string()
            .contains("more than"));
    }

    #[test]
    fn writer_and_reader_compressors_agree() {
        let text = b"the quick brown fox jumps over the lazy dog. the quick brown fox again \x01\x02 \xe9 \xff!";
        let packed = palmdoc_compress(text);
        assert_eq!(palmdoc_decompress(&packed, 9999).unwrap(), text);
        let repeated = b"abcabcabcabcabcabc".repeat(50);
        let packed = palmdoc_compress(&repeated);
        assert!(packed.len() < repeated.len() / 4);
        assert_eq!(palmdoc_decompress(&packed, 99999).unwrap(), repeated);
    }

    // --- trailing entries ---------------------------------------------------------------------

    #[test]
    fn trailing_entries_are_stripped() {
        // No flags: untouched.
        assert_eq!(strip_trailing(b"abcdef", 0), b"abcdef");
        // flag bit 1: one backward varint entry. Entry bytes "xy" + size byte 0x83 (size 3 incl. itself).
        assert_eq!(strip_trailing(b"abcdexy\x83", 0b10), b"abcde");
        // flag bit 0 (multibyte overlap): last byte & 3 = 2 -> 3 bytes removed.
        assert_eq!(strip_trailing(b"abcdef\x02", 0b1), b"abcd");
        // both: first the varint entry, then the overlap byte before it.
        // layout: "abc" + overlap(0x01 -> 2 bytes: 'Z', 0x01) + entry("k", 0x82)
        assert_eq!(strip_trailing(b"abcZ\x01k\x82", 0b11), b"abc");
        // two varint bits (bits 1 and 2): the second entry is located before the first.
        assert_eq!(strip_trailing(b"abcyz\x83x\x82", 0b110), b"abc");
        // multi byte varint, read backwards: 0x2C then 0x82 (high bit ends it) = 0x2C | (2 << 7) = 300
        assert_eq!(trailing_entry_size(b"..\x82\x2C", 4), 300);
        // never underflows on tiny records
        assert_eq!(strip_trailing(b"", 0xFFFF), b"");
        assert_eq!(strip_trailing(b"\xff", 0b11), b"");
    }

    #[test]
    fn base32_and_locale_helpers() {
        assert_eq!(base32_index("0001"), Some(1));
        assert_eq!(base32_index("000A"), Some(10));
        assert_eq!(base32_index("0010"), Some(32));
        assert_eq!(base32_index("00v0"), Some(31 * 32));
        assert_eq!(base32_index("00W0"), None);
        assert_eq!(base32_index(""), None);
        assert_eq!(base32_index("1234567"), None);
        assert_eq!(locale_language(9), "en");
        assert_eq!(locale_language(0x0407), "de");
        assert_eq!(locale_language(0x0419), "ru");
        assert_eq!(locale_language(999), "");
    }

    // --- whole books ----------------------------------------------------------------------------

    fn spec() -> MobiSpec {
        MobiSpec {
            title: "Zürich Tales".into(),
            authors: vec!["Ann Author".into(), "Bob Buch".into()],
            publisher: "Pub House".into(),
            description: "<p>About &amp; more</p>".into(),
            isbn: Some("9781234567897".into()),
            subjects: vec!["Fiction".into(), "Travel; Swiss".into()],
            language: Some("de".into()),
            html: concat!(
                "<html><head><guide><reference type=\"toc\" title=\"Contents\" filepos=0000000000 /></guide></head><body>",
                "<h1>Chapter One</h1><p>First paragraph with <b>bold</b> &amp; <i>italic</i> text. Caf\u{e9}.</p>",
                "<p><a filepos=0000000000>skip</a></p><img recindex=\"00002\" />",
                "<mbp:pagebreak/>",
                "<h2>Chapter Two</h2><p>Second chapter <a href=\"#x\">link</a>.</p><p>Jump to <a filepos=XXXXXXXXXX>chapter three</a>.</p>",
                "<mbp:pagebreak/>",
                "<p>Third chapter has no heading, but a first line.</p><img recindex=\"00009\" />",
                "</body></html>"
            )
            .into(),
            images: vec![PNG.to_vec(), JPG.to_vec()],
            cover_index: Some(1),
            ..Default::default()
        }
    }

    #[test]
    fn palmdoc_compressed_mobi_roundtrip() {
        let bytes = write_mobi(&spec());
        assert!(MobiReader.sniff(&bytes));
        let arc = convert_mobi(&bytes).unwrap();
        let book = validate(&arc);
        let md = book.metadata();
        assert_eq!(md.title, "Zürich Tales");
        assert_eq!(md.authors, vec!["Ann Author", "Bob Buch"]);
        assert_eq!(md.publisher, "Pub House");
        assert_eq!(md.description, "About & more");
        assert_eq!(md.tags, vec!["Fiction", "Travel", "Swiss"]);
        assert_eq!(md.language, "de");
        assert_eq!(md.identifier("isbn"), Some("9781234567897"));
        assert_eq!(book.cover_bytes(), Some(JPG));
        assert_eq!(
            toc_titles(&book),
            vec![
                "Chapter One",
                "Chapter Two",
                "Third chapter has no heading, but a first line."
            ]
        );
        let t: Vec<String> = (0..3).map(|i| strip_tags(&chapter_xml(&book, i))).collect();
        assert!(
            t[0].contains("First paragraph with bold & italic text. Café."),
            "{}",
            t[0]
        );
        assert!(t[1].contains("Second chapter link"));
    }

    #[test]
    fn images_links_and_missing_images() {
        let bytes = write_mobi(&spec());
        let arc = convert_mobi(&bytes).unwrap();
        let book = validate(&arc);
        let x = chapter_xml(&book, 0);
        assert!(
            x.contains("<img src=\"images/image-0002.jpg\" alt=\"\"/>"),
            "{x}"
        );
        assert_eq!(arc.get("OEBPS/images/image-0002.jpg"), Some(JPG));
        // recindex 9 does not exist: the image is dropped, not left dangling
        let x3 = chapter_xml(&book, 2);
        assert!(!x3.contains("<img"), "{x3}");
        // filepos links point at an inserted anchor, resolved across chapters
        let x2 = chapter_xml(&book, 1);
        assert!(x2.contains("href=\"chapter-3.xhtml#filepos"), "{x2}");
        assert!(x2.contains("href=\"#x\""));
        let target = x2
            .split("chapter-3.xhtml#")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(chapter_xml(&book, 2).contains(&format!("id=\"{target}\"")));
    }

    #[test]
    fn uncompressed_utf8_mobi_with_trailing_entries() {
        let mut s = spec();
        s.compress = false;
        s.encoding = 65001;
        s.extra_flags = 0b11; // multibyte overlap + one varint entry on every text record
        s.title = "日本語の本".into();
        s.html = "<html><body><h1>見出し</h1><p>本文です。</p></body></html>".into();
        let bytes = write_mobi(&s);
        let book = validate(&convert_mobi(&bytes).unwrap());
        assert_eq!(book.metadata().title, "日本語の本");
        assert_eq!(toc_titles(&book), vec!["見出し"]);
        assert!(chapter_texts(&book)[0].contains("本文です。"));
    }

    #[test]
    fn text_spanning_many_records() {
        let mut s = spec();
        let para = "<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit. Caf\u{e9} \u{201c}quoted\u{201d}.</p>";
        s.html = format!(
            "<html><body><h1>Long</h1>{}</body></html>",
            para.repeat(400)
        );
        s.images.clear();
        s.cover_index = None;
        let bytes = write_mobi(&s);
        let book = validate(&convert_mobi(&bytes).unwrap());
        assert_eq!(chapter_xml(&book, 0).matches("Lorem ipsum").count(), 400);
        assert!(book.cover_bytes().is_none());
    }

    #[test]
    fn exth_title_wins_and_windows_1252_is_decoded() {
        let mut s = spec();
        s.exth_updated_title = Some("Updated Title".into());
        let book = validate(&convert_mobi(&write_mobi(&s)).unwrap());
        assert_eq!(book.metadata().title, "Updated Title");
    }

    #[test]
    fn palmdoc_plain_text_books() {
        let bytes = write_palmdoc("My_Notes", "Line one\nLine two & more\n\nLine three");
        assert!(MobiReader.sniff(&bytes));
        let book = validate(&convert_mobi(&bytes).unwrap());
        assert_eq!(book.metadata().title, "My Notes");
        assert!(chapter_xml(&book, 0).contains("<p>Line two &amp; more</p>"));
        assert!(chapter_xml(&book, 0).contains("<p>Line three</p>"));
    }

    #[test]
    fn huffcdic_and_drm_are_clear_errors() {
        let mut s = spec();
        s.compression_override = Some(17480);
        let err = convert_mobi(&write_mobi(&s)).unwrap_err().to_string();
        assert!(err.contains("HUFF/CDIC"), "{err}");
        let mut s = spec();
        s.encryption = 2;
        assert!(convert_mobi(&write_mobi(&s))
            .unwrap_err()
            .to_string()
            .contains("DRM"));
        let mut s = spec();
        s.drm_offset = Some(0x1000);
        assert!(convert_mobi(&write_mobi(&s))
            .unwrap_err()
            .to_string()
            .contains("DRM"));
        let mut s = spec();
        s.compression_override = Some(7);
        assert!(convert_mobi(&write_mobi(&s))
            .unwrap_err()
            .to_string()
            .contains("compression type 7"));
    }

    #[test]
    fn malformed_containers_are_errors() {
        let good = write_mobi(&spec());
        assert!(convert_mobi(b"").is_err());
        assert!(convert_mobi(&good[..70])
            .unwrap_err()
            .to_string()
            .contains("too small"));
        // truncated in the middle of the records
        let cut = convert_mobi(&good[..good.len() / 2]);
        assert!(cut.is_err());
        // zero records
        let mut z = good.clone();
        z[76] = 0;
        z[77] = 0;
        assert!(convert_mobi(&z)
            .unwrap_err()
            .to_string()
            .contains("no records"));
        // missing MOBI magic
        let mut m = good.clone();
        let rec0 = u32::from_be_bytes([m[78], m[79], m[80], m[81]]) as usize;
        m[rec0 + 16..rec0 + 20].copy_from_slice(b"XXXX");
        assert!(convert_mobi(&m)
            .unwrap_err()
            .to_string()
            .contains("MOBI header is missing"));
        // descending record offsets
        let mut d = good.clone();
        d[78 + 8..78 + 12].copy_from_slice(&0u32.to_be_bytes());
        assert!(convert_mobi(&d).is_err());
        // offsets past the end
        let mut e = good.clone();
        e[78 + 16..78 + 20].copy_from_slice(&0xFFFF_FF00u32.to_be_bytes());
        assert!(convert_mobi(&e)
            .unwrap_err()
            .to_string()
            .contains("truncated"));
        // empty text
        let mut s = spec();
        s.html = "<html><body></body></html>".into();
        assert!(convert_mobi(&write_mobi(&s))
            .unwrap_err()
            .to_string()
            .contains("no readable text"));
    }

    // --- KF8 -------------------------------------------------------------------------------------

    fn kf8_spec(joint: bool) -> MobiSpec {
        let mut s = spec();
        s.kf8 = Some(Kf8Spec {
            joint,
            files: vec![
                (
                    "<?xml version=\"1.0\" encoding=\"utf-8\"?><!DOCTYPE html><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title><link rel=\"stylesheet\" href=\"kindle:flow:0001?mime=text/css\"/></head><body aid=\"0\"></body></html>".into(),
                    vec!["<h1>KF8 One</h1>".into(), "<p>Alpha <a href=\"kindle:pos:fid:0001:off:0000000000\">link</a> text.</p><img src=\"kindle:embed:0001?mime=image/png\"/>".into()],
                ),
                (
                    "<?xml version=\"1.0\" encoding=\"utf-8\"?><!DOCTYPE html><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t2</title></head><body aid=\"1\"></body></html>".into(),
                    vec!["<h2>KF8 Two</h2><p>Beta text.</p>".into()],
                ),
            ],
            css: "p { margin: 0; } .visible-css-junk { color: red; }".into(),
        });
        s
    }

    #[test]
    fn kf8_joint_file_reads_the_kf8_half() {
        let bytes = write_mobi(&kf8_spec(true));
        let arc = convert_mobi(&bytes).unwrap();
        let book = validate(&arc);
        assert_eq!(toc_titles(&book), vec!["KF8 One", "KF8 Two"]);
        let t: Vec<String> = (0..2).map(|i| strip_tags(&chapter_xml(&book, i))).collect();
        assert!(t[0].contains("Alpha link text."), "{}", t[0]);
        assert!(t[1].contains("Beta text."));
        let all = t.join(" ");
        assert!(
            !all.contains("visible-css-junk") && !all.contains("margin"),
            "CSS flow must be cut off: {all}"
        );
        assert!(!all.contains("Chapter One"), "the MOBI 6 half is not used");
        let x = chapter_xml(&book, 0);
        assert!(x.contains("<img src=\"images/image-0001.png\""), "{x}");
        assert_eq!(arc.get("OEBPS/images/image-0001.png"), Some(PNG));
        assert!(!x.contains("kindle:"), "{x}");
        assert_eq!(book.metadata().title, "Zürich Tales");
        assert_eq!(book.cover_bytes(), Some(JPG));
    }

    #[test]
    fn kf8_only_file() {
        let bytes = write_mobi(&kf8_spec(false));
        let book = validate(&convert_mobi(&bytes).unwrap());
        assert_eq!(toc_titles(&book), vec!["KF8 One", "KF8 Two"]);
        assert!(chapter_texts(&book)[1].contains("Beta text."));
        assert_eq!(book.metadata().authors, vec!["Ann Author", "Bob Buch"]);
    }

    #[test]
    fn kf8_without_fdst_still_reads_in_order() {
        let mut s = kf8_spec(false);
        s.kf8.as_mut().unwrap().css = String::new();
        s.no_fdst = true;
        let book = validate(&convert_mobi(&write_mobi(&s)).unwrap());
        assert_eq!(toc_titles(&book), vec!["KF8 One", "KF8 Two"]);
    }

    // --- misc --------------------------------------------------------------------------------------

    #[test]
    fn sniffing_and_registration() {
        let b = write_mobi(&spec());
        assert!(MobiReader.sniff(&b));
        assert!(MobiReader.sniff(&b[..68]));
        assert!(!MobiReader.sniff(&b[..60]));
        assert!(!MobiReader.sniff(b"BOOKMOBI"));
        assert!(!MobiReader.sniff(&[0u8; 200]));
        assert_eq!(MobiReader.id(), "mobi");
        for e in ["mobi", "azw", "azw3", "prc"] {
            assert!(MobiReader.extensions().contains(&e));
        }
    }

    #[test]
    fn filepos_resolution() {
        let src = b"<p>x</p><a filepos=0000000011>l</a><p id=y>t</p>".to_vec();
        // offset 11 is the 'a' of "<a ..." -> not '<', so no anchor is inserted, link survives
        let out = String::from_utf8(resolve_filepos(src.clone(), &[])).unwrap();
        assert!(out.contains("href=\"#filepos11\""));
        assert!(!out.contains("id=\"filepos"));
        // offset 8 is the '<' of the <a> tag
        let src = b"<p>x</p><a filepos=8>l</a>".to_vec();
        let out = String::from_utf8(resolve_filepos(src, &[])).unwrap();
        assert_eq!(
            out,
            "<p>x</p><a id=\"filepos8\"></a><a href=\"#filepos8\">l</a>"
        );
        // out of range targets are ignored
        let out = resolve_filepos(b"<a filepos=99999>l</a>".to_vec(), &[]);
        assert_eq!(out, b"<a href=\"#filepos99999\">l</a>");
    }

    #[test]
    fn robust_against_garbage_and_mutations() {
        assert_rejects_garbage(&MobiReader);
        let errors = fuzz_no_panic(&MobiReader, &write_mobi(&spec()));
        assert!(errors > 20);
        fuzz_no_panic(&MobiReader, &write_mobi(&kf8_spec(true)));
        let mut u = spec();
        u.compress = false;
        fuzz_no_panic(&MobiReader, &write_mobi(&u));
        fuzz_no_panic(&MobiReader, &write_palmdoc("T", "some text\nmore"));
    }

    #[test]
    fn bomb_like_records_are_rejected() {
        // a record that expands past the cap through repeated maximal back references
        let mut d = vec![0x61];
        for _ in 0..2_000_000 {
            d.extend([0x80, 0x0F]);
        }
        assert!(palmdoc_decompress(&d, MAX_RECORD_OUT).is_err());
    }

    #[test]
    fn xml_escape_is_used_for_palmdoc_text() {
        let b = write_palmdoc("T", "<script>alert(1)</script>");
        let book = validate(&convert_mobi(&b).unwrap());
        assert!(chapter_xml(&book, 0).contains("&lt;script&gt;"));
    }
}
