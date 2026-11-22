//! LRF (Sony BBeB "Broadband eBook") input, best effort.
//!
//! The file is a little-endian binary container: a 0x58 byte header (`L\0R\0F\0`, version, the
//! scrambling key, the object count and the offset of the object index), a zlib compressed XML
//! document with the book information, an optional thumbnail and the objects. Every object starts
//! with a `0xF500` tag (id and type) and is a sequence of tags: `0xF5xx` where `xx` selects the
//! payload size (see [`tag_size`]). A few objects own a stream (flags `0x100` zlib, `0x200`
//! scrambled).
//!
//! What is read: the page tree (reading order), the blocks of each page, the text objects (UTF-16
//! text with paragraph, line break, italic, superscript and subscript tags), image blocks and inline
//! images (JPEG, PNG, GIF, BMP), the table of contents and the book information. Page layout,
//! fonts, colours, buttons and sound are ignored. Scrambled (`0x200`) streams are descrambled. LRX
//! (encrypted) files are rejected. Real files made by Calibre's LRF writer are read in the tests
//! when `ebook-convert` is installed; everything else is tested on synthetic files.

use std::collections::HashMap;
use std::io::Read;

use anyhow::{Context, Result, bail};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::Metadata;
use bookx_core::util::xml_escape;
use flate2::read::ZlibDecoder;

use crate::common::*;

const HEADER_LEN: usize = 0x58;
const MAX_OBJECTS: usize = 500_000;
const MAX_STREAM: u64 = 128 << 20;

/// Reader for `.lrf` files.
pub struct LrfReader;

/// `L\0R\0F\0`, "LRF" in UTF-16LE.
pub fn has_magic(head: &[u8]) -> bool {
    head.starts_with(b"L\0R\0F\0")
}

impl FormatReader for LrfReader {
    fn id(&self) -> &'static str {
        "lrf"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["lrf"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        has_magic(head)
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_lrf(bytes)
    }
}

/// How the payload of a tag is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Fixed(usize),
    /// `u16` byte length followed by UTF-16 text.
    Str,
    /// `u16` count followed by that many `u32`.
    List,
    /// `u32`, a nested `0xF516` string tag, `u16`.
    Nested,
}

/// Payload layout of the tag `0xF5xx` (`None`: unknown tag).
pub fn tag_size(code: u8) -> Option<Size> {
    const Z: &[u8] = &[
        0x01, 0x05, 0x06, 0x4D, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x6B, 0x6E,
        0x71, 0x72, 0x81, 0x82, 0xA2, 0xA6, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xB1, 0xB2,
        0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD, 0xBE, 0xC1, 0xC2, 0xC4,
        0xC7, 0xC9, 0xD2, 0xD6,
    ];
    const W: &[u8] = &[
        0x0D, 0x0E, 0x11, 0x12, 0x13, 0x14, 0x15, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x21, 0x22,
        0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2A, 0x2B, 0x2C, 0x2E, 0x31, 0x32, 0x33, 0x35, 0x36,
        0x38, 0x39, 0x3A, 0x3C, 0x3D, 0x3E, 0x41, 0x42, 0x46, 0x47, 0x48, 0x51, 0x52, 0x54, 0x56,
        0x57, 0x58, 0x5E, 0x61, 0x6D, 0x75, 0x76, 0x77, 0x79, 0x7A, 0xC3, 0xC5, 0xC6, 0xC8, 0xCA,
        0xCC, 0xD4, 0xDA, 0xDB, 0xDC, 0xDD, 0xF1, 0xF4,
    ];
    const D: &[u8] = &[
        0x02, 0x03, 0x04, 0x07, 0x08, 0x09, 0x0A, 0x17, 0x18, 0x2D, 0x34, 0x37, 0x44, 0x45, 0x4B,
        0x4C, 0x53, 0x5B, 0x7B, 0x7C, 0xA1, 0xA7, 0xD8, 0xF2, 0xF3, 0xF5, 0xF6, 0xF7, 0xF8,
    ];
    Some(match code {
        c if Z.contains(&c) => Size::Fixed(0),
        c if W.contains(&c) => Size::Fixed(2),
        c if D.contains(&c) => Size::Fixed(4),
        0x00 | 0x29 | 0xF9 => Size::Fixed(6),
        0x49 | 0x4A | 0x6C | 0xD9 => Size::Fixed(8),
        0x73 => Size::Fixed(10),
        0x4E | 0xD1 => Size::Fixed(12),
        0xD7 => Size::Fixed(14),
        0x16 | 0x55 | 0x59 | 0x5A | 0x5D => Size::Str,
        0x0B | 0x5C => Size::List,
        0x78 => Size::Nested,
        _ => return None,
    })
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn le32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn le64(b: &[u8], at: usize) -> Option<u64> {
    let s = b.get(at..at + 8)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(u64::from_le_bytes(a))
}

/// Reads the tag at `*pos`: its id (`0xF5xx`) and payload (for strings and lists without the
/// count). `None` at the end of the data, at a non-tag or unknown tag, or when it is truncated.
pub fn read_tag<'a>(data: &'a [u8], pos: &mut usize) -> Option<(u16, &'a [u8])> {
    let head = data.get(*pos..*pos + 2)?;
    if head[1] != 0xF5 {
        return None;
    }
    let id = 0xF500 + head[0] as u16;
    let start = *pos + 2;
    let (payload, end) = match tag_size(head[0])? {
        Size::Fixed(n) => (data.get(start..start + n)?, start + n),
        Size::Str => {
            let n = le16(data, start)? as usize;
            (data.get(start + 2..start + 2 + n)?, start + 2 + n)
        }
        Size::List => {
            let n = le16(data, start)? as usize * 4;
            (data.get(start + 2..start + 2 + n)?, start + 2 + n)
        }
        Size::Nested => {
            // u32, then the tag F516 (string), then u16
            let inner = start + 4;
            if data.get(inner..inner + 2)? != [0x16, 0xF5] {
                return None;
            }
            let n = le16(data, inner + 2)? as usize;
            let end = inner + 4 + n + 2;
            (data.get(start..end)?, end)
        }
    };
    *pos = end;
    Some((id, payload))
}

/// One object of the file.
#[derive(Debug, Default, Clone)]
pub struct Object {
    pub id: u32,
    pub ty: u16,
    pub tags: Vec<(u16, Vec<u8>)>,
    pub stream: Option<Vec<u8>>,
    pub stream_flags: u16,
}

impl Object {
    fn tag(&self, id: u16) -> Option<&[u8]> {
        self.tags
            .iter()
            .find(|(t, _)| *t == id)
            .map(|(_, p)| p.as_slice())
    }
    fn dword(&self, id: u16) -> Option<u32> {
        self.tag(id).and_then(|p| le32(p, 0))
    }
    fn list(&self, id: u16) -> Vec<u32> {
        self.tag(id)
            .map(|p| {
                p.chunks_exact(4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect()
            })
            .unwrap_or_default()
    }
}

const T_OBJECT_START: u16 = 0xF500;
const T_OBJECT_END: u16 = 0xF501;
const T_LINK: u16 = 0xF503;
const T_STREAM_SIZE: u16 = 0xF504;
const T_STREAM_START: u16 = 0xF505;
const T_STREAM_FLAGS: u16 = 0xF554;
const T_CONTENTS: u16 = 0xF55C;
const T_IMAGE_STREAM: u16 = 0xF54C;

const TY_PAGE_TREE: u16 = 0x01;
const TY_PAGE: u16 = 0x02;
const TY_BLOCK: u16 = 0x06;
const TY_TEXT: u16 = 0x0A;
const TY_IMAGE: u16 = 0x0C;
const TY_IMAGE_STREAM: u16 = 0x11;
const TY_SIMPLE_TEXT: u16 = 0x1D;
const TY_TOC: u16 = 0x1E;

fn descramble(buf: &mut [u8], key: u16, limited: bool) {
    let l = buf.len();
    let k = (key & 0xFF) as usize;
    let key = if k != 0 && k <= 0xF0 { l % k + 0xF } else { 0 } as u8;
    let n = if limited && l > 0x400 { 0x400 } else { l };
    for b in &mut buf[..n] {
        *b ^= key;
    }
}

fn parse_object(data: &[u8], offset: usize, size: usize, key: u16) -> Result<Object> {
    let end = offset
        .checked_add(size)
        .filter(|e| *e <= data.len())
        .context("an object lies beyond the end of the file")?;
    let body = &data[offset..end];
    let mut pos = 0;
    let (t, payload) = read_tag(body, &mut pos).context("bad object start")?;
    if t != T_OBJECT_START {
        bail!("object does not start with the object tag");
    }
    let mut obj = Object {
        id: le32(payload, 0).context("short object tag")?,
        ty: le16(payload, 4).context("short object tag")?,
        ..Default::default()
    };
    let mut stream_size = 0usize;
    while pos < body.len() {
        let (t, p) = read_tag(body, &mut pos).context("unreadable tag in an object")?;
        match t {
            T_OBJECT_END => break,
            T_STREAM_SIZE => stream_size = le32(p, 0).unwrap_or(0) as usize,
            T_STREAM_FLAGS => {
                obj.stream_flags = le16(p, 0).unwrap_or(0);
                obj.tags.push((t, p.to_vec()));
            }
            T_STREAM_START => {
                let raw = body
                    .get(pos..pos.checked_add(stream_size).context("stream size")?)
                    .context("a stream is truncated")?;
                pos += stream_size;
                let mut buf = raw.to_vec();
                if obj.stream_flags & 0x200 != 0 {
                    let media = matches!(obj.ty, TY_IMAGE_STREAM | 0x17 | 0x19);
                    descramble(&mut buf, key, media);
                }
                if obj.stream_flags & 0x100 != 0 {
                    let want = le32(&buf, 0).context("compressed stream without size")? as u64;
                    if want > MAX_STREAM {
                        bail!("a stream of {want} bytes is larger than the limit");
                    }
                    let mut out = Vec::new();
                    ZlibDecoder::new(&buf[4..])
                        .take(MAX_STREAM)
                        .read_to_end(&mut out)
                        .context("corrupt compressed stream")?;
                    buf = out;
                }
                obj.stream = Some(buf);
                // the end-of-stream tag follows; the loop reads it as an ordinary tag
            }
            _ => obj.tags.push((t, p.to_vec())),
        }
    }
    Ok(obj)
}

/// The parsed file.
pub struct Lrf {
    pub version: u16,
    pub objects: HashMap<u32, Object>,
    /// Object ids in index order.
    pub order: Vec<u32>,
    pub info_xml: Option<String>,
    pub thumbnail: Option<Vec<u8>>,
    pub toc_id: u32,
}

/// Parses the container.
pub fn parse(bytes: &[u8]) -> Result<Lrf> {
    if !has_magic(bytes) {
        bail!("not a Sony LRF file: the magic number is wrong");
    }
    if bytes.len() < HEADER_LEN {
        bail!("the LRF header is truncated");
    }
    let version = le16(bytes, 8).unwrap_or(0);
    let key = le16(bytes, 0xA).unwrap_or(0);
    let count = le64(bytes, 0x10).unwrap_or(0) as usize;
    let index = le64(bytes, 0x18).unwrap_or(0) as usize;
    let toc_id = le32(bytes, 0x44).unwrap_or(0);
    let info_len = le16(bytes, 0x4C).unwrap_or(0) as usize;
    let thumb_len = le32(bytes, 0x50).unwrap_or(0) as usize;
    let info_raw = le32(bytes, 0x54).unwrap_or(0) as usize;
    if count == 0 || count > MAX_OBJECTS {
        bail!("the file claims {count} objects");
    }
    let table = bytes
        .get(index..index.saturating_add(count * 16))
        .context("the object index lies outside the file (truncated file)")?;
    let info_xml = if info_len > 4 {
        bytes
            .get(HEADER_LEN..HEADER_LEN + info_len - 4)
            .and_then(|z| {
                let mut out = Vec::new();
                ZlibDecoder::new(z)
                    .take(info_raw.max(1) as u64 * 8 + 4096)
                    .read_to_end(&mut out)
                    .ok()?;
                Some(decode_xml(&out))
            })
    } else {
        None
    };
    let thumbnail = (thumb_len > 0)
        .then(|| {
            let at = HEADER_LEN + info_len.saturating_sub(4);
            bytes.get(at..at + thumb_len).map(|t| t.to_vec())
        })
        .flatten();
    let mut objects = HashMap::new();
    let mut order = Vec::new();
    for e in table.chunks_exact(16) {
        let id = le32(e, 0).unwrap_or(0);
        let off = le32(e, 4).unwrap_or(0) as usize;
        let size = le32(e, 8).unwrap_or(0) as usize;
        let obj = parse_object(bytes, off, size, key)
            .with_context(|| format!("object {id} at offset {off}"))?;
        order.push(obj.id);
        objects.insert(obj.id, obj);
    }
    Ok(Lrf {
        version,
        objects,
        order,
        info_xml,
        thumbnail,
        toc_id,
    })
}

/// Book information from the `<Info>` document.
fn metadata(xml: &str) -> Metadata {
    let mut md = Metadata::default();
    let Ok(doc) = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')) else {
        return md;
    };
    let find = |name: &str| {
        doc.descendants()
            .find(|n| n.has_tag_name(name))
            .map(node_text)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if let Some(t) = find("Title") {
        md.title = t;
    }
    if let Some(a) = find("Author") {
        md.authors = a
            .split(" & ")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(p) = find("Publisher") {
        md.publisher = p;
    }
    if let Some(l) = find("Language") {
        md.language = l;
    }
    if let Some(c) = find("FreeText") {
        md.description = c;
    }
    md
}

fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Inline images are collected while the text is rendered.
struct Ctx<'a> {
    lrf: &'a Lrf,
    media: MediaSet,
}

impl Ctx<'_> {
    fn image_xhtml(&mut self, image_obj: u32) -> Option<String> {
        let img = self.lrf.objects.get(&image_obj)?;
        let sid = img.dword(T_IMAGE_STREAM)?;
        let stream = self.lrf.objects.get(&sid)?;
        let data = stream.stream.clone()?;
        let path = self.media.add(&format!("lrf-{sid}"), data)?;
        Some(format!(
            "<div class=\"image\"><img src=\"{}\" alt=\"\"/></div>",
            xml_escape(&path)
        ))
    }
}

/// Converts the stream of a text object to XHTML blocks.
fn render_text(ctx: &mut Ctx, data: &[u8]) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut open: Vec<Option<&'static str>> = Vec::new();
    let flush =
        |cur: &mut String, open: &mut Vec<Option<&'static str>>, blocks: &mut Vec<String>| {
            while let Some(o) = open.pop() {
                if let Some(t) = o {
                    cur.push_str(&format!("</{t}>"));
                }
            }
            if !cur.trim().is_empty() {
                blocks.push(format!("<p>{}</p>", cur.trim()));
            }
            cur.clear();
        };
    let mut pos = 0;
    let mut text_start = 0;
    let push_text = |cur: &mut String, from: usize, to: usize| {
        if to > from {
            cur.push_str(&xml_escape(&utf16(&data[from..to])));
        }
    };
    while pos + 1 < data.len() {
        let known = data[pos + 1] == 0xF5 && tag_size(data[pos]).is_some();
        if !known {
            pos += 2;
            continue;
        }
        push_text(&mut cur, text_start, pos);
        let at = pos;
        let Some((id, payload)) = read_tag(data, &mut pos) else {
            // a truncated tag: the rest is dropped
            text_start = data.len();
            break;
        };
        text_start = pos;
        match id {
            0xF5CC => {
                // explicit text: the word is a byte count of UTF-16 text that follows
                let n = le16(payload, 0).unwrap_or(0) as usize;
                let end = (pos + n).min(data.len());
                cur.push_str(&xml_escape(&utf16(&data[pos..end])));
                pos = end;
                text_start = pos;
            }
            0xF5A1 | 0xF5A2 | 0xF5D2 => flush(&mut cur, &mut open, &mut blocks),
            0xF581 => {
                cur.push_str("<i>");
                open.push(Some("i"));
            }
            0xF5B7 => {
                cur.push_str("<sup>");
                open.push(Some("sup"));
            }
            0xF5B9 => {
                cur.push_str("<sub>");
                open.push(Some("sub"));
            }
            // other containers (vertical text, ruby, no-break, emphasis dots, boxes, buttons)
            0xF5B1 | 0xF5B3 | 0xF5B5 | 0xF5A9 | 0xF5AB | 0xF5AD | 0xF5BB | 0xF5BD | 0xF5C1
            | 0xF5C3 | 0xF5C6 | 0xF5A7 => open.push(None),
            0xF582 | 0xF5B2 | 0xF5B4 | 0xF5B6 | 0xF5A8 | 0xF5AA | 0xF5AC | 0xF5AE | 0xF5B8
            | 0xF5BA | 0xF5BC | 0xF5BE | 0xF5C2 | 0xF5C4 | 0xF5C7 => {
                if let Some(Some(t)) = open.pop() {
                    cur.push_str(&format!("</{t}>"));
                }
            }
            0xF5D1 => {
                // plot: xsize u16, ysize u16, image object u32, adjustment u32
                if let Some(obj) = le32(payload, 4) {
                    flush(&mut cur, &mut open, &mut blocks);
                    if let Some(x) = ctx.image_xhtml(obj) {
                        blocks.push(x);
                    }
                }
            }
            0xF5CA => cur.push(' '),
            _ => {}
        }
        debug_assert!(pos > at);
    }
    if text_start < data.len() {
        push_text(&mut cur, text_start, data.len() - data.len() % 2);
    }
    flush(&mut cur, &mut open, &mut blocks);
    blocks
}

/// One block of a page, with the ids that table of contents entries may point at.
struct Piece {
    ids: Vec<u32>,
    xhtml: Vec<String>,
}

fn block_pieces(ctx: &mut Ctx, page_id: u32, block: &Object) -> Piece {
    let mut ids = vec![page_id, block.id];
    let mut xhtml = Vec::new();
    let target = block.stream.as_deref().and_then(|s| {
        let mut p = 0;
        match read_tag(s, &mut p) {
            Some((T_LINK, pl)) => le32(pl, 0),
            _ => None,
        }
    });
    if let Some(t) = target {
        ids.push(t);
        if let Some(obj) = ctx.lrf.objects.get(&t) {
            match obj.ty {
                TY_TEXT | TY_SIMPLE_TEXT => {
                    if let Some(s) = &obj.stream {
                        xhtml = render_text(ctx, s);
                    }
                }
                TY_IMAGE => {
                    if let Some(x) = ctx.image_xhtml(t) {
                        xhtml.push(x);
                    }
                }
                _ => {}
            }
        }
    }
    Piece { ids, xhtml }
}

/// Block ids of a page, in order (links inside the page stream).
fn page_blocks(page: &Object) -> Vec<u32> {
    let Some(s) = &page.stream else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut p = 0;
    while p < s.len() {
        match read_tag(s, &mut p) {
            Some((T_LINK, pl)) => out.extend(le32(pl, 0)),
            Some(_) => {}
            None => break,
        }
    }
    out
}

/// `(page, object, label)` of the table of contents.
fn toc_entries(lrf: &Lrf) -> Vec<(u32, u32, String)> {
    let obj = lrf
        .objects
        .get(&lrf.toc_id)
        .filter(|o| o.ty == TY_TOC)
        .or_else(|| {
            lrf.order
                .iter()
                .filter_map(|i| lrf.objects.get(i))
                .find(|o| o.ty == TY_TOC)
        });
    let Some(s) = obj.and_then(|o| o.stream.as_ref()) else {
        return Vec::new();
    };
    let count = le16(s, 0).unwrap_or(0) as usize;
    let mut pos = 4 * (count + 1);
    let mut out = Vec::new();
    for _ in 0..count {
        let (Some(page), Some(target), Some(n)) =
            (le32(s, pos), le32(s, pos + 4), le16(s, pos + 8))
        else {
            break;
        };
        let Some(label) = s.get(pos + 10..pos + 10 + n as usize) else {
            break;
        };
        out.push((page, target, utf16(label)));
        pos += 10 + n as usize;
    }
    out
}

/// Converts an LRF file.
pub fn convert_lrf(bytes: &[u8]) -> Result<Archive> {
    let lrf = parse(bytes)?;
    let mut md = lrf.info_xml.as_deref().map(metadata).unwrap_or_default();
    let mut ctx = Ctx {
        lrf: &lrf,
        media: MediaSet::default(),
    };

    let mut pages: Vec<u32> = lrf
        .order
        .iter()
        .filter_map(|i| lrf.objects.get(i))
        .filter(|o| o.ty == TY_PAGE_TREE)
        .flat_map(|o| o.list(T_CONTENTS))
        .collect();
    if pages.is_empty() {
        pages = lrf
            .order
            .iter()
            .filter(|i| lrf.objects.get(i).is_some_and(|o| o.ty == TY_PAGE))
            .copied()
            .collect();
    }
    let mut pieces: Vec<Piece> = Vec::new();
    for pid in pages {
        let Some(page) = lrf.objects.get(&pid).filter(|o| o.ty == TY_PAGE) else {
            continue;
        };
        for bid in page_blocks(page) {
            if let Some(b) = lrf.objects.get(&bid).filter(|o| o.ty == TY_BLOCK) {
                pieces.push(block_pieces(&mut ctx, pid, b));
            }
        }
    }

    // chapters: one per table of contents entry, else a single one
    let toc = toc_entries(&lrf);
    let mut starts: Vec<(usize, String)> = Vec::new();
    for (page, target, label) in &toc {
        let at = pieces
            .iter()
            .position(|p| p.ids[1..].contains(target))
            .or_else(|| pieces.iter().position(|p| p.ids[0] == *page));
        if let Some(at) = at {
            if starts.last().is_none_or(|(s, _)| *s < at) {
                starts.push((at, label.trim().to_string()));
            }
        }
    }
    let default_title = if md.title.is_empty() {
        "Text".to_string()
    } else {
        md.title.clone()
    };
    let mut chapters: Vec<Chapter> = Vec::new();
    let join = |range: &[Piece]| -> String {
        range
            .iter()
            .flat_map(|p| p.xhtml.iter())
            .map(|b| format!("{b}\n"))
            .collect()
    };
    let first = starts.first().map(|(s, _)| *s).unwrap_or(pieces.len());
    let head = join(&pieces[..first]);
    if visible_text_len(&head) > 0 || head.contains("<img") {
        chapters.push(Chapter {
            title: default_title.clone(),
            body: head,
        });
    }
    for (i, (s, label)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map(|(e, _)| *e).unwrap_or(pieces.len());
        let body = join(&pieces[*s..end]);
        let title = if label.is_empty() {
            format!("Part {}", i + 1)
        } else {
            label.clone()
        };
        // keep the chapter even if it only holds a heading-less image
        chapters.push(Chapter {
            title: title.clone(),
            body: format!("<h1>{}</h1>\n{body}", xml_escape(&title)),
        });
    }
    if chapters.is_empty() {
        bail!("the LRF file has no readable text");
    }
    let cover = lrf
        .thumbnail
        .as_ref()
        .and_then(|t| image_ext(t).map(|e| (t.clone(), e.to_string())));
    if md.title.is_empty() {
        md.title = "Untitled".into();
    }
    build(Parts {
        md,
        chapters,
        cover,
        media: ctx.media.items,
        ..Default::default()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::*;
    use std::io::Write;

    fn tag(code: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![code, 0xF5];
        v.extend_from_slice(payload);
        v
    }
    fn tag_str(code: u8, s: &str) -> Vec<u8> {
        let u: Vec<u8> = s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        let mut p = (u.len() as u16).to_le_bytes().to_vec();
        p.extend(u);
        tag(code, &p)
    }
    fn u16s(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect()
    }
    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// An object with optional stream.
    pub(crate) fn object(
        id: u32,
        ty: u16,
        tags: &[Vec<u8>],
        stream: Option<(u16, &[u8])>,
    ) -> Vec<u8> {
        let mut v = tag(
            0x00,
            &[id.to_le_bytes().as_slice(), &ty.to_le_bytes()].concat(),
        );
        for t in tags {
            v.extend_from_slice(t);
        }
        if let Some((flags, data)) = stream {
            v.extend(tag(0x54, &flags.to_le_bytes()));
            v.extend(tag(0x04, &(data.len() as u32).to_le_bytes()));
            v.extend(tag(0x05, &[]));
            v.extend_from_slice(data);
            v.extend(tag(0x06, &[]));
        }
        v.extend(tag(0x01, &[]));
        v
    }

    pub(crate) struct Build {
        pub objects: Vec<Vec<u8>>,
        pub ids: Vec<u32>,
        pub info: String,
        pub toc: u32,
        pub thumb: Vec<u8>,
        pub key: u16,
    }

    impl Build {
        pub(crate) fn new() -> Self {
            Build {
                objects: vec![],
                ids: vec![],
                info: "<Info version=\"1.1\"><BookInfo><Title>Synthetic</Title><Author>Ann Author</Author><Publisher>Pub</Publisher></BookInfo><DocInfo><Language>en</Language></DocInfo></Info>".into(),
                toc: 0,
                thumb: vec![],
                key: 0,
            }
        }
        pub(crate) fn add(&mut self, id: u32, bytes: Vec<u8>) {
            self.ids.push(id);
            self.objects.push(bytes);
        }
        pub(crate) fn bytes(&self) -> Vec<u8> {
            let info = zlib(self.info.as_bytes());
            let mut out = vec![0u8; HEADER_LEN];
            out[..6].copy_from_slice(b"L\0R\0F\0");
            out[8..10].copy_from_slice(&1000u16.to_le_bytes());
            out[0xA..0xC].copy_from_slice(&self.key.to_le_bytes());
            out[0xC..0x10].copy_from_slice(&1u32.to_le_bytes());
            out[0x10..0x18].copy_from_slice(&(self.objects.len() as u64).to_le_bytes());
            out[0x44..0x48].copy_from_slice(&self.toc.to_le_bytes());
            out[0x4C..0x4E].copy_from_slice(&((info.len() + 4) as u16).to_le_bytes());
            out[0x50..0x54].copy_from_slice(&(self.thumb.len() as u32).to_le_bytes());
            out[0x54..0x58].copy_from_slice(&(self.info.len() as u32).to_le_bytes());
            out.extend(info);
            out.extend(&self.thumb);
            let mut index = Vec::new();
            for (id, o) in self.ids.iter().zip(&self.objects) {
                index.extend(id.to_le_bytes());
                index.extend((out.len() as u32).to_le_bytes());
                index.extend((o.len() as u32).to_le_bytes());
                index.extend(0u32.to_le_bytes());
                out.extend(o);
            }
            let at = out.len() as u64;
            out[0x18..0x20].copy_from_slice(&at.to_le_bytes());
            out.extend(index);
            out
        }
    }

    fn link(id: u32) -> Vec<u8> {
        tag(0x03, &id.to_le_bytes())
    }

    fn text_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(tag(0xA1, &[0; 4]));
        s.extend(u16s("Hello "));
        s.extend(tag(0x81, &[]));
        s.extend(u16s("italic"));
        s.extend(tag(0x82, &[]));
        s.extend(u16s(" & <world>"));
        s.extend(tag(0xA2, &[]));
        s.extend(tag(0xA1, &[0; 4]));
        s.extend(tag(0x11, &[24, 0])); // a span attribute is skipped
        s.extend(u16s("Second "));
        s.extend(tag(0xCC, &(10u16).to_le_bytes()));
        s.extend(u16s("para!"));
        s.extend(tag(0xA2, &[]));
        s
    }

    /// page tree 2 -> pages 3, 4 -> blocks 5, 6, 7 -> text 8, 9 and image 10 -> stream 11.
    fn book() -> Build {
        let mut b = Build::new();
        b.add(
            2,
            object(
                2,
                0x01,
                &[tag(
                    0x5C,
                    &[
                        &2u16.to_le_bytes()[..],
                        &3u32.to_le_bytes(),
                        &4u32.to_le_bytes(),
                    ]
                    .concat(),
                )],
                None,
            ),
        );
        let mut p1 = link(5);
        p1.extend(tag(0x4E, &[0; 12])); // a page element that is skipped
        p1.extend(link(6));
        b.add(3, object(3, 0x02, &[], Some((0, &p1))));
        b.add(4, object(4, 0x02, &[], Some((0, &link(7)))));
        b.add(5, object(5, 0x06, &[], Some((0, &link(8)))));
        b.add(6, object(6, 0x06, &[], Some((0, &link(9)))));
        b.add(7, object(7, 0x06, &[], Some((0, &link(10)))));
        b.add(
            8,
            object(
                8,
                0x0A,
                &[],
                Some((
                    0x100,
                    &[
                        (text_stream().len() as u32).to_le_bytes().to_vec(),
                        zlib(&text_stream()),
                    ]
                    .concat(),
                )),
            ),
        );
        let mut t2 = tag(0xA1, &[0; 4]);
        t2.extend(u16s("Chapter two text"));
        t2.extend(tag(0xA2, &[]));
        b.add(9, object(9, 0x0A, &[], Some((0, &t2))));
        b.add(
            10,
            object(10, 0x0C, &[tag(0x4C, &11u32.to_le_bytes())], None),
        );
        b.add(11, object(11, 0x11, &[], Some((0x12, PNG))));
        let toc_label = |page: u32, obj: u32, l: &str| {
            let mut v = page.to_le_bytes().to_vec();
            v.extend(obj.to_le_bytes());
            v.extend((l.encode_utf16().count() as u16 * 2).to_le_bytes());
            v.extend(u16s(l));
            v
        };
        let mut toc = 2u16.to_le_bytes().to_vec();
        toc.extend([0, 0]);
        toc.extend([0u8; 8]);
        toc.extend(toc_label(3, 5, "First"));
        toc.extend(toc_label(3, 6, "Second"));
        b.add(12, object(12, 0x1E, &[], Some((0, &toc))));
        b.toc = 12;
        b
    }

    fn text_of(arc: &Archive) -> (String, Vec<String>) {
        let book = validate(arc);
        let mut all = String::new();
        for p in book.spine_paths() {
            all.push_str(&book.chapter_source(&p).unwrap());
        }
        let toc = bookx_core::toc::flatten(&book.toc())
            .into_iter()
            .map(|t| t.title)
            .collect();
        (all, toc)
    }

    #[test]
    fn tag_table_sizes() {
        assert_eq!(tag_size(0x00), Some(Size::Fixed(6)));
        assert_eq!(tag_size(0x01), Some(Size::Fixed(0)));
        assert_eq!(tag_size(0xD7), Some(Size::Fixed(14)));
        assert_eq!(tag_size(0x16), Some(Size::Str));
        assert_eq!(tag_size(0x5C), Some(Size::List));
        assert_eq!(tag_size(0x78), Some(Size::Nested));
        assert_eq!(tag_size(0xA5), None);
        assert_eq!(tag_size(0xFF), None);
        let mut p = 0;
        let d = tag_str(0x16, "ab");
        assert_eq!(read_tag(&d, &mut p), Some((0xF516, &u16s("ab")[..])));
        assert_eq!(p, d.len());
        let mut p = 0;
        assert_eq!(read_tag(&[1, 2], &mut p), None);
        let mut p = 0;
        assert_eq!(read_tag(&[0x00, 0xF5, 1], &mut p), None, "truncated");
    }

    #[test]
    fn reads_metadata_text_images_and_toc() {
        let b = book().bytes();
        assert!(LrfReader.sniff(&b));
        let arc = LrfReader.read(&b).unwrap();
        let (text, toc) = text_of(&arc);
        assert_eq!(toc, vec!["First", "Second"]);
        assert!(
            text.contains("<p>Hello <i>italic</i> &amp; &lt;world&gt;</p>"),
            "{text}"
        );
        assert!(text.contains("<p>Second para!</p>"), "{text}");
        assert!(text.contains("Chapter two text"));
        assert!(text.contains("<img src=\"images/lrf-11.png\""), "{text}");
        let book = validate(&arc);
        let md = book.metadata();
        assert_eq!(md.title, "Synthetic");
        assert_eq!(md.authors, vec!["Ann Author"]);
        assert_eq!(md.publisher, "Pub");
        assert_eq!(md.language, "en");
        assert!(arc.names().iter().any(|n| n.ends_with("images/lrf-11.png")));
    }

    #[test]
    fn without_a_toc_the_book_is_one_chapter() {
        let mut b = book();
        b.toc = 0;
        b.objects.pop();
        b.ids.pop();
        let arc = LrfReader.read(&b.bytes()).unwrap();
        let (text, toc) = text_of(&arc);
        assert_eq!(toc, vec!["Synthetic"]);
        assert!(text.contains("Chapter two text") && text.contains("Hello"));
    }

    #[test]
    fn scrambled_streams_are_descrambled() {
        let mut b = book();
        let mut t2 = tag(0xA1, &[0; 4]);
        t2.extend(u16s("Scrambled words here"));
        t2.extend(tag(0xA2, &[]));
        let mut s = t2.clone();
        descramble(&mut s, 0x0007, false);
        assert_ne!(s, t2);
        b.key = 0x0007;
        let i = b.ids.iter().position(|i| *i == 9).unwrap();
        b.objects[i] = object(9, 0x0A, &[], Some((0x200, &s)));
        let arc = LrfReader.read(&b.bytes()).unwrap();
        assert!(text_of(&arc).0.contains("Scrambled words here"));
    }

    #[test]
    fn plot_tags_place_inline_images() {
        let mut b = book();
        let mut t = tag(0xA1, &[0; 4]);
        t.extend(u16s("Before"));
        t.extend(tag(
            0xD1,
            &[&[1, 0, 1, 0][..], &10u32.to_le_bytes(), &[0; 4]].concat(),
        ));
        t.extend(u16s("After"));
        t.extend(tag(0xA2, &[]));
        let i = b.ids.iter().position(|i| *i == 9).unwrap();
        b.objects[i] = object(9, 0x0A, &[], Some((0, &t)));
        let (text, _) = text_of(&LrfReader.read(&b.bytes()).unwrap());
        let before = text.find("Before").unwrap();
        let img = text.find("lrf-11.png").unwrap();
        let after = text.find("After").unwrap();
        assert!(before < img && img < after);
    }

    #[test]
    fn thumbnail_becomes_the_cover() {
        let mut b = book();
        b.thumb = PNG.to_vec();
        let arc = LrfReader.read(&b.bytes()).unwrap();
        assert!(validate(&arc).cover_bytes().is_some());
    }

    #[test]
    fn errors_are_clear() {
        let e = |b: &[u8]| format!("{:#}", LrfReader.read(b).unwrap_err());
        assert!(e(b"nope").contains("magic"));
        assert!(e(b"L\0R\0F\0\0\0").contains("truncated"));
        let good = book().bytes();
        assert!(e(&good[..good.len() - 40]).contains("outside the file"));
        let mut zero = good.clone();
        zero[0x10..0x18].fill(0);
        assert!(e(&zero).contains("0 objects"));
        // an LRX (encrypted) file has a different magic
        assert!(e(b"LRX\0\0\0\0\0").contains("magic"));
        // a file with pages but no text
        let mut b = Build::new();
        b.add(2, object(2, 0x01, &[], None));
        assert!(e(&b.bytes()).contains("no readable text"));
    }

    #[test]
    fn corrupt_objects_are_errors_not_panics() {
        let good = book().bytes();
        fuzz_no_panic(&LrfReader, &good);
        assert_rejects_garbage(&LrfReader);
        // truncated stream, bad zlib
        let mut b = book();
        let i = b.ids.iter().position(|i| *i == 8).unwrap();
        b.objects[i] = object(8, 0x0A, &[], Some((0x100, &[9, 0, 0, 0, 1, 2, 3])));
        assert!(format!("{:#}", LrfReader.read(&b.bytes()).unwrap_err()).contains("compressed"));
    }

    #[test]
    fn ebook_convert_files_are_read() {
        let Some(bytes) = real_lrf() else { return };
        let arc = LrfReader.read(&bytes).unwrap();
        let (text, toc) = text_of(&arc);
        assert!(text.contains("The first paragraph with"), "{text}");
        assert!(text.contains("italic words"));
        assert!(text.contains("Another chapter text here."));
        assert_eq!(toc, vec!["Chapter One", "Chapter Two"]);
        let md = validate(&arc).metadata().clone();
        assert_eq!(md.title, "LRF Test");
        assert_eq!(md.authors, vec!["Zed Writer"]);
    }

    /// An LRF made by Calibre, or `None` when `ebook-convert` is missing.
    fn real_lrf() -> Option<Vec<u8>> {
        let dir = std::env::temp_dir().join(format!("bookx-lrf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).ok()?;
        let html = dir.join("b.html");
        std::fs::write(
            &html,
            "<html><head><title>LRF Test</title></head><body><h1>Chapter One</h1><p>The first paragraph with <i>italic words</i> and more.</p><p>Second paragraph.</p><h1>Chapter Two</h1><p>Another chapter text here.</p></body></html>",
        )
        .ok()?;
        let out = dir.join("b.lrf");
        let ok = std::process::Command::new("ebook-convert")
            .arg(&html)
            .arg(&out)
            .args(["--authors", "Zed Writer", "--title", "LRF Test"])
            .args(["--chapter", "//h:h1", "--level1-toc", "//h:h1"])
            .output()
            .ok()?
            .status
            .success();
        let bytes = ok.then(|| std::fs::read(&out).ok()).flatten();
        let _ = std::fs::remove_dir_all(&dir);
        bytes
    }
}
