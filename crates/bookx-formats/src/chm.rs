//! CHM (Microsoft Compiled HTML Help) input.
//!
//! * `ITSF` header (versions 2 and 3): offsets of the directory and of the content section.
//! * `ITSP` directory header and `PMGL` listing chunks (`PMGI` index chunks are skipped; the
//!   listing chunks alone contain every entry): path, section, offset, length.
//! * Section 0 is stored as is; section 1 (`MSCompressed`) is LZX: window and reset interval
//!   from `ControlData`, frame offsets from the `ResetTable`, decoded by [`crate::lzx`].
//! * `#SYSTEM` supplies the title, the table of contents file and the default topic. The `.hhc`
//!   table of contents (a sitemap in HTML) gives the chapter order; without one the HTML files
//!   are taken in directory order. `#TOPICS`/`#URLTBL` string tables are not needed.

use std::cell::RefCell;
use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::Metadata;
use regex::Regex;

use crate::common::*;
use crate::lzx;

const CONTENT: &str = "::DataSpace/Storage/MSCompressed/Content";
const CONTROL: &str = "::DataSpace/Storage/MSCompressed/ControlData";
const RESET: &str = "::DataSpace/Storage/MSCompressed/Transform/{7FC28940-9D31-11D0-9B27-00A0C91E9C7C}/InstanceData/ResetTable";
const MAX_ENTRIES: usize = 200_000;
const MAX_CHUNKS: usize = 1_000_000;
/// Largest decompressed LZX content.
const MAX_CONTENT: usize = 256 << 20;

/// Reader for `.chm` files.
pub struct ChmReader;

impl FormatReader for ChmReader {
    fn id(&self) -> &'static str {
        "chm"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["chm"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(b"ITSF")
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_chm(bytes)
    }
}

fn le16(b: &[u8], off: usize) -> Option<u16> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn le32(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn le64(b: &[u8], off: usize) -> Option<u64> {
    b.get(off..off + 8).map(|s| {
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        u64::from_le_bytes(a)
    })
}

/// Reads a CHM "ENCINT": 7 bits per byte, big endian groups, high bit = more bytes follow.
fn encint(b: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v: u64 = 0;
    for _ in 0..10 {
        let byte = *b.get(*pos)?;
        *pos += 1;
        v = (v << 7) | (byte & 0x7F) as u64;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// A directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChmEntry {
    pub path: String,
    pub section: u64,
    pub offset: u64,
    pub length: u64,
}

/// A parsed CHM container.
pub struct Chm<'a> {
    bytes: &'a [u8],
    content_offset: usize,
    pub entries: Vec<ChmEntry>,
    index: HashMap<String, usize>,
    lzx_stream: RefCell<Option<Vec<u8>>>,
}

impl<'a> Chm<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if !bytes.starts_with(b"ITSF") {
            bail!("not a CHM file: the ITSF header is missing");
        }
        let version = le32(bytes, 4).context("the ITSF header is truncated")?;
        let header_len = le32(bytes, 8).context("the ITSF header is truncated")? as usize;
        if !(2..=3).contains(&version) || header_len < 0x58 {
            bail!("unsupported CHM version {version} (header length {header_len})");
        }
        let dir_off = le64(bytes, 0x48).context("the ITSF header is truncated")? as usize;
        let dir_len = le64(bytes, 0x50).context("the ITSF header is truncated")? as usize;
        let content_offset = if version >= 3 {
            le64(bytes, 0x58).context("the ITSF header is truncated")? as usize
        } else {
            dir_off.saturating_add(dir_len)
        };
        let dir = bytes
            .get(dir_off..dir_off.saturating_add(dir_len))
            .context("the CHM directory lies beyond the end of the file")?;
        if !dir.starts_with(b"ITSP") {
            bail!("the CHM directory header (ITSP) is missing");
        }
        let hdr = le32(dir, 8).context("the ITSP header is truncated")? as usize;
        let chunk_size = le32(dir, 0x10).context("the ITSP header is truncated")? as usize;
        let n_chunks = le32(dir, 0x2C).context("the ITSP header is truncated")? as usize;
        if !(32..=(1 << 24)).contains(&chunk_size) || n_chunks > MAX_CHUNKS {
            bail!("the CHM directory declares a chunk size of {chunk_size} and {n_chunks} chunks");
        }
        let mut entries = Vec::new();
        for i in 0..n_chunks {
            let start = hdr.saturating_add(i.saturating_mul(chunk_size));
            let Some(chunk) = dir.get(start..start.saturating_add(chunk_size)) else {
                break;
            };
            if !chunk.starts_with(b"PMGL") {
                continue;
            }
            let free = le32(chunk, 4).unwrap_or(0) as usize;
            let end = chunk_size.saturating_sub(free).max(20);
            let mut pos = 20usize;
            while pos < end {
                let Some(name_len) = encint(chunk, &mut pos) else {
                    break;
                };
                let Some(name) = chunk.get(pos..pos.saturating_add(name_len as usize)) else {
                    break;
                };
                pos += name_len as usize;
                let (Some(section), Some(offset), Some(length)) = (
                    encint(chunk, &mut pos),
                    encint(chunk, &mut pos),
                    encint(chunk, &mut pos),
                ) else {
                    break;
                };
                entries.push(ChmEntry {
                    path: String::from_utf8_lossy(name).into_owned(),
                    section,
                    offset,
                    length,
                });
                if entries.len() > MAX_ENTRIES {
                    bail!("the CHM directory has too many entries");
                }
            }
        }
        if entries.is_empty() {
            bail!("the CHM directory has no entries");
        }
        let index = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.path.to_ascii_lowercase(), i))
            .rev()
            .collect();
        Ok(Chm {
            bytes,
            content_offset,
            entries,
            index,
            lzx_stream: RefCell::new(None),
        })
    }

    pub fn entry(&self, path: &str) -> Option<&ChmEntry> {
        self.index
            .get(&path.to_ascii_lowercase())
            .map(|i| &self.entries[*i])
    }

    fn raw_section0(&self, e: &ChmEntry) -> Result<&'a [u8]> {
        let start = self
            .content_offset
            .checked_add(e.offset as usize)
            .context("entry offset overflows")?;
        let end = start
            .checked_add(e.length as usize)
            .context("entry length overflows")?;
        self.bytes.get(start..end).with_context(|| {
            format!(
                "`{}` lies beyond the end of the file (truncated file)",
                e.path
            )
        })
    }

    fn decompress_content(&self) -> Result<Vec<u8>> {
        let content = self
            .entry(CONTENT)
            .context("the CHM has no compressed content section")?;
        let control = self
            .entry(CONTROL)
            .context("the LZX control data is missing")?;
        let reset = self
            .entry(RESET)
            .context("the LZX reset table is missing")?;
        let compressed = self.raw_section0(content)?;
        let cd = self.raw_section0(control)?;
        if cd.get(4..8) != Some(b"LZXC") {
            bail!("the LZX control data has no LZXC tag");
        }
        let version = le32(cd, 8).context("the LZX control data is truncated")?;
        let interval = le32(cd, 12).context("the LZX control data is truncated")? as u64;
        let window = le32(cd, 16).context("the LZX control data is truncated")? as u64;
        let (window_bytes, reset_frames) = if version == 2 {
            (window * lzx::FRAME as u64, interval)
        } else {
            (window, interval / lzx::FRAME as u64)
        };
        let window_bytes = u32::try_from(window_bytes).context("LZX window size overflows")?;
        let ws = lzx::window_size(window_bytes)?;

        let rt = self.raw_section0(reset)?;
        let n = le32(rt, 4).context("the LZX reset table is truncated")? as usize;
        let table_at = le32(rt, 12).context("the LZX reset table is truncated")? as usize;
        let total = le64(rt, 16).context("the LZX reset table is truncated")?;
        let comp_total = le64(rt, 24).unwrap_or(compressed.len() as u64) as usize;
        if total > MAX_CONTENT as u64 {
            bail!("the compressed content of {total} bytes is larger than the limit");
        }
        let total = total as usize;
        if n > total.div_ceil(lzx::FRAME) + 2 || (n == 0 && total > 0) {
            bail!("the LZX reset table lists {n} frames for {total} bytes");
        }
        let comp_end = comp_total.min(compressed.len());
        let mut offsets = Vec::with_capacity(n + 1);
        for i in 0..n {
            let o = le64(rt, table_at.saturating_add(i * 8))
                .context("the LZX reset table is truncated")? as usize;
            offsets.push(o);
        }
        offsets.push(comp_end);
        let mut frames: Vec<&[u8]> = Vec::with_capacity(n);
        for i in 0..n {
            let (s, e) = (offsets[i], offsets[i + 1]);
            if s > e || e > compressed.len() {
                bail!("LZX frame {i} has invalid bounds {s}..{e}");
            }
            frames.push(&compressed[s..e]);
        }
        lzx::decompress_frames(ws, &frames, total, reset_frames as usize, MAX_CONTENT)
    }

    /// The bytes of a file, whichever section it lives in.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let e = self
            .entry(path)
            .with_context(|| format!("`{path}` is not in the CHM"))?;
        match e.section {
            0 => Ok(self.raw_section0(e)?.to_vec()),
            1 => {
                if self.lzx_stream.borrow().is_none() {
                    let data = self.decompress_content()?;
                    *self.lzx_stream.borrow_mut() = Some(data);
                }
                let guard = self.lzx_stream.borrow();
                let stream = guard.as_ref().context("no LZX data")?;
                let start = e.offset as usize;
                let end = start
                    .checked_add(e.length as usize)
                    .context("entry length overflows")?;
                stream
                    .get(start..end)
                    .map(|s| s.to_vec())
                    .with_context(|| format!("`{}` lies beyond the decompressed content", e.path))
            }
            s => bail!("`{}` is in unknown section {s}", e.path),
        }
    }
}

/// Fields of the `#SYSTEM` file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SystemInfo {
    pub title: String,
    pub toc_file: String,
    pub default_topic: String,
}

/// Parses `#SYSTEM`: a version, then `code(u16) length(u16) data` records.
pub fn parse_system(data: &[u8]) -> SystemInfo {
    let mut info = SystemInfo::default();
    let mut pos = 4usize;
    while let (Some(code), Some(len)) = (le16(data, pos), le16(data, pos + 2)) {
        let Some(rec) = data.get(pos + 4..pos + 4 + len as usize) else {
            break;
        };
        pos += 4 + len as usize;
        let end = rec.iter().position(|b| *b == 0).unwrap_or(rec.len());
        let s = decode_text(&rec[..end]);
        match code {
            0 => info.toc_file = s,
            2 => info.default_topic = s,
            3 => info.title = s,
            _ => {}
        }
    }
    info
}

/// One node of an `.hhc` table of contents, flattened in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocItem {
    pub title: String,
    pub local: String,
    pub depth: usize,
}

/// Parses a sitemap `.hhc` file.
pub fn parse_hhc(html: &str) -> Vec<TocItem> {
    let doc = bookx_core::util::parse_html_document(html);
    let mut out = Vec::new();
    let Ok(sel) = scraper::Selector::parse("object") else {
        return out;
    };
    for obj in doc.select(&sel) {
        let mut name = String::new();
        let mut local = String::new();
        for child in obj.children() {
            if let Some(el) = scraper::ElementRef::wrap(child) {
                if el.value().name() == "param" {
                    let n = el.value().attr("name").unwrap_or("").to_ascii_lowercase();
                    let v = el.value().attr("value").unwrap_or("");
                    match n.as_str() {
                        "name" if name.is_empty() => name = v.trim().to_string(),
                        "local" if local.is_empty() => local = v.trim().to_string(),
                        _ => {}
                    }
                }
            }
        }
        if name.is_empty() && local.is_empty() {
            continue;
        }
        // depth = number of enclosing <ul> elements
        let depth = obj
            .ancestors()
            .filter(|a| a.value().as_element().is_some_and(|e| e.name() == "ul"))
            .count();
        out.push(TocItem {
            title: name,
            local,
            depth,
        });
    }
    out
}

fn norm_path(base_dir: &str, href: &str) -> String {
    let href = href.replace('\\', "/");
    let href = href.split(['#', '?']).next().unwrap_or("");
    let mut parts: Vec<&str> = if href.starts_with('/') {
        Vec::new()
    } else {
        base_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn is_html(path: &str) -> bool {
    let l = path.to_ascii_lowercase();
    l.ends_with(".htm") || l.ends_with(".html") || l.ends_with(".xhtml")
}

/// Converts a CHM file.
pub fn convert_chm(bytes: &[u8]) -> Result<Archive> {
    let chm = Chm::parse(bytes)?;
    let system = chm
        .read("/#SYSTEM")
        .map(|d| parse_system(&d))
        .unwrap_or_default();

    // Order of pages and their titles.
    let mut order: Vec<(String, String)> = Vec::new(); // (path, title)
    let toc_candidates: Vec<String> = if system.toc_file.is_empty() {
        chm.entries
            .iter()
            .filter(|e| e.path.to_ascii_lowercase().ends_with(".hhc"))
            .map(|e| e.path.clone())
            .collect()
    } else {
        vec![norm_path("", &system.toc_file)]
    };
    for cand in toc_candidates {
        if let Ok(data) = chm.read(&cand) {
            for item in parse_hhc(&decode_xml(&data)) {
                if item.local.is_empty() {
                    continue;
                }
                let p = norm_path("", &item.local);
                if chm.entry(&p).is_some() && !order.iter().any(|(q, _)| q.eq_ignore_ascii_case(&p))
                {
                    order.push((p, item.title));
                }
            }
            if !order.is_empty() {
                break;
            }
        }
    }
    if order.is_empty() {
        let mut files: Vec<&ChmEntry> = chm
            .entries
            .iter()
            .filter(|e| is_html(&e.path) && !e.path.contains("/#") && e.length > 0)
            .collect();
        if !system.default_topic.is_empty() {
            let d = norm_path("", &system.default_topic);
            if let Some(pos) = files.iter().position(|e| e.path.eq_ignore_ascii_case(&d)) {
                let root = files.remove(pos);
                files.insert(0, root);
            }
        }
        order = files
            .into_iter()
            .map(|e| (e.path.clone(), String::new()))
            .collect();
    }
    if order.is_empty() {
        bail!("the CHM has no HTML pages");
    }

    let media = RefCell::new(MediaSet::default());
    let mut kept: Vec<(usize, String, String, String)> = Vec::new(); // (order index, title, body, dir)
    for (i, (path, title)) in order.iter().enumerate() {
        let data = chm
            .read(path)
            .with_context(|| format!("reading `{path}`"))?;
        let html = decode_xml(&data);
        let dir = dir_of(path).to_string();
        let rewrite = |src: &str| -> Option<String> {
            let p = norm_path(&dir, src);
            let img = chm.read(&p).ok()?;
            media.borrow_mut().add(&p, img)
        };
        let body = html_blocks(&html, &rewrite).join("\n");
        if visible_text_len(&body) == 0 && !body.contains("<img") {
            continue;
        }
        let t = if !title.is_empty() {
            title.clone()
        } else {
            html_title(&html).unwrap_or_else(|| format!("Page {}", kept.len() + 1))
        };
        kept.push((i, t, body, dir));
    }
    if kept.is_empty() {
        bail!("the CHM pages contain no readable text");
    }
    let chapter_of: HashMap<String, usize> = kept
        .iter()
        .enumerate()
        .map(|(n, (i, ..))| (order[*i].0.to_ascii_lowercase(), n + 1))
        .collect();
    let link_re = Regex::new(r##"href="([^"#]*)(#[^"]*)?""##).expect("static regex");
    let chapters: Vec<Chapter> = kept
        .into_iter()
        .map(|(_, title, body, dir)| {
            let body = link_re
                .replace_all(&body, |c: &regex::Captures| {
                    let target = norm_path(&dir, &xml_unescape(&c[1]));
                    match chapter_of.get(&target.to_ascii_lowercase()) {
                        Some(n) if !c[1].is_empty() => format!(
                            "href=\"chapter-{n}.xhtml{}\"",
                            c.get(2).map_or("", |m| m.as_str())
                        ),
                        _ => c[0].to_string(),
                    }
                })
                .into_owned();
            Chapter { title, body }
        })
        .collect();

    let md = Metadata {
        title: if system.title.trim().is_empty() {
            "Untitled".into()
        } else {
            system.title.trim().to_string()
        },
        ..Default::default()
    };
    build(Parts {
        md,
        chapters,
        media: media.into_inner().items,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fmt_writers::*;
    use crate::testutil::*;

    fn pages() -> Vec<(String, Vec<u8>)> {
        vec![
            ("/index.htm".into(), b"<html><head><title>Home</title></head><body><h1>Welcome</h1><p>Start here. <a href=\"sub/two.htm#frag\">to two</a> <img src=\"img/pic.png\"></p></body></html>".to_vec()),
            ("/sub/two.htm".into(), b"<html><body><h1>Second</h1><p>Page two text.</p><a href=\"../index.htm\">back</a></body></html>".to_vec()),
            ("/sub/three.htm".into(), b"<html><head><title>Third Title</title></head><body><p>Third page.</p></body></html>".to_vec()),
            ("/img/pic.png".into(), PNG.to_vec()),
        ]
    }

    const HHC: &str = "<HTML><BODY><UL><LI><OBJECT type=\"text/sitemap\"><param name=\"Name\" value=\"Intro\"><param name=\"Local\" value=\"index.htm\"></OBJECT><UL><LI><OBJECT type=\"text/sitemap\"><param name=\"Name\" value=\"Two\"><param name=\"Local\" value=\"sub\\two.htm#frag\"></OBJECT></UL><LI><OBJECT type=\"text/sitemap\"><param name=\"Name\" value=\"Three\"><param name=\"Local\" value=\"sub/three.htm\"></OBJECT><LI><OBJECT type=\"text/sitemap\"><param name=\"Name\" value=\"Dup\"><param name=\"Local\" value=\"index.htm\"></OBJECT><LI><OBJECT type=\"text/sitemap\"><param name=\"Name\" value=\"Missing\"><param name=\"Local\" value=\"nofile.htm\"></OBJECT><LI><OBJECT type=\"text/sitemap\"><param name=\"Name\" value=\"Folder only\"></OBJECT></UL></BODY></HTML>";

    fn sample(compressed: bool) -> Vec<u8> {
        let mut files = pages();
        files.push(("/toc.hhc".into(), HHC.as_bytes().to_vec()));
        write_chm(&ChmSpec {
            title: "Help Book".into(),
            toc_file: Some("toc.hhc".into()),
            default_topic: Some("index.htm".into()),
            files,
            compressed,
            ..Default::default()
        })
    }

    #[test]
    fn encint_values() {
        let mut p = 0;
        assert_eq!(encint(&[0x05], &mut p), Some(5));
        let mut p = 0;
        assert_eq!(encint(&[0x81, 0x00], &mut p), Some(128));
        let mut p = 0;
        assert_eq!(
            encint(&[0x82, 0xC1, 0x05], &mut p),
            Some((2 << 14) | (0x41 << 7) | 5)
        );
        let mut p = 0;
        assert_eq!(encint(&[0x80], &mut p), None, "ends inside a number");
        let mut p = 0;
        assert_eq!(encint(&[0xFF; 12], &mut p), None, "too long");
    }

    #[test]
    fn directory_lists_all_entries_with_sections() {
        let f = sample(true);
        let chm = Chm::parse(&f).unwrap();
        let e = chm.entry("/INDEX.HTM").unwrap();
        assert_eq!(e.section, 1, "files are in the compressed section");
        assert_eq!(chm.entry(CONTENT).unwrap().section, 0);
        assert!(chm.entry("/#SYSTEM").is_some());
        assert!(chm.entry("/nope").is_none());
        let f2 = sample(false);
        assert_eq!(
            Chm::parse(&f2)
                .unwrap()
                .entry("/index.htm")
                .unwrap()
                .section,
            0
        );
    }

    #[test]
    fn reads_files_from_both_sections() {
        for compressed in [true, false] {
            let f = sample(compressed);
            let chm = Chm::parse(&f).unwrap();
            assert_eq!(
                chm.read("/sub/two.htm").unwrap(),
                pages()[1].1,
                "compressed={compressed}"
            );
            assert_eq!(chm.read("/img/pic.png").unwrap(), PNG);
            assert!(format!("{}", chm.read("/missing").unwrap_err()).contains("not in the CHM"));
        }
    }

    #[test]
    fn multi_frame_content_roundtrips_through_lzx() {
        let big: Vec<u8> = (0..150_000u32)
            .map(|i| b"lorem ipsum dolor sit amet "[(i % 27) as usize] ^ ((i / 5000) as u8 & 1))
            .collect();
        let f = write_chm(&ChmSpec {
            files: vec![
                ("/big.htm".into(), big.clone()),
                ("/small.htm".into(), b"<p>s</p>".to_vec()),
            ],
            compressed: true,
            reset_frames: 2,
            ..Default::default()
        });
        let chm = Chm::parse(&f).unwrap();
        assert_eq!(chm.read("/big.htm").unwrap(), big);
        assert_eq!(chm.read("/small.htm").unwrap(), b"<p>s</p>");
    }

    #[test]
    fn converts_in_toc_order_with_titles_links_and_images() {
        for compressed in [true, false] {
            let arc = ChmReader.read(&sample(compressed)).unwrap();
            let book = validate(&arc);
            assert_eq!(book.metadata().title, "Help Book");
            assert_eq!(
                toc_titles(&book),
                ["Intro", "Two", "Three"],
                "compressed={compressed}"
            );
            let t = chapter_texts(&book);
            assert!(t[0].contains("Start here."));
            assert!(t[1].contains("Page two text."));
            let c0 = chapter_xml(&book, 0);
            assert!(c0.contains("href=\"chapter-2.xhtml#frag\""), "{c0}");
            assert!(c0.contains("<img src=\"images/pic.png\""));
            assert!(chapter_xml(&book, 1).contains("href=\"chapter-1.xhtml\""));
        }
    }

    #[test]
    fn without_hhc_html_files_are_taken_in_directory_order_default_topic_first() {
        let f = write_chm(&ChmSpec {
            title: String::new(),
            default_topic: Some("sub/three.htm".into()),
            files: pages(),
            compressed: false,
            ..Default::default()
        });
        let book = validate(&ChmReader.read(&f).unwrap());
        assert_eq!(book.metadata().title, "Untitled");
        let titles = toc_titles(&book);
        assert_eq!(titles[0], "Third Title");
        assert_eq!(titles.len(), 3);
    }

    #[test]
    fn system_file_parsing() {
        let mut d = 3u32.to_le_bytes().to_vec();
        for (code, s) in [(0u16, "a.hhc"), (2, "start.htm"), (3, "My Title"), (9, "x")] {
            d.extend(code.to_le_bytes());
            d.extend(((s.len() + 1) as u16).to_le_bytes());
            d.extend(s.as_bytes());
            d.push(0);
        }
        let i = parse_system(&d);
        assert_eq!(
            i,
            SystemInfo {
                title: "My Title".into(),
                toc_file: "a.hhc".into(),
                default_topic: "start.htm".into()
            }
        );
        assert_eq!(parse_system(&[]), SystemInfo::default());
        d.truncate(d.len() - 2);
        let _ = parse_system(&d);
    }

    #[test]
    fn hhc_parsing_depth_and_missing_params() {
        let items = parse_hhc(HHC);
        assert_eq!(
            items[0],
            TocItem {
                title: "Intro".into(),
                local: "index.htm".into(),
                depth: 1
            }
        );
        assert_eq!(items[1].depth, 2);
        assert_eq!(items[1].local, "sub\\two.htm#frag");
        assert_eq!(items.last().unwrap().local, "", "folder without a page");
        assert!(parse_hhc("").is_empty());
    }

    #[test]
    fn path_normalisation() {
        assert_eq!(norm_path("/a/b", "../c.htm"), "/a/c.htm");
        assert_eq!(norm_path("/a", "x\\y.htm#f"), "/a/x/y.htm");
        assert_eq!(norm_path("/a", "/root.htm"), "/root.htm");
        assert_eq!(norm_path("", "./i.htm?x=1"), "/i.htm");
        assert_eq!(norm_path("/a", "../../../up.htm"), "/up.htm");
        assert_eq!(dir_of("/a/b.htm"), "/a");
        assert_eq!(dir_of("b.htm"), "");
    }

    #[test]
    fn structural_errors() {
        assert!(
            format!("{}", Chm::parse(b"nope").err().unwrap()).contains("ITSF header is missing")
        );
        let mut f = sample(true);
        f[4..8].copy_from_slice(&9u32.to_le_bytes());
        assert!(format!("{}", Chm::parse(&f).err().unwrap()).contains("unsupported CHM version 9"));
        let mut f = sample(true);
        f[0x48..0x50].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(Chm::parse(&f).is_err());
        let f = sample(true);
        let dir = le64(&f, 0x48).unwrap() as usize;
        let mut g = f.clone();
        g[dir..dir + 4].copy_from_slice(b"XXXX");
        assert!(format!("{}", Chm::parse(&g).err().unwrap()).contains("ITSP"));
        let mut g = f.clone();
        g[dir + 0x10..dir + 0x14].copy_from_slice(&4u32.to_le_bytes());
        assert!(Chm::parse(&g).is_err(), "chunk size too small");
        let mut g = f.clone();
        for b in &mut g[dir + 0x54..dir + 0x54 + 8] {
            *b = 0;
        }
        let cut = &f[..f.len() / 3];
        assert!(ChmReader.read(cut).is_err());
        let _ = g;
    }

    #[test]
    fn corrupt_lzx_control_data_is_reported() {
        let f = sample(true);
        let chm = Chm::parse(&f).unwrap();
        let ctl = chm.entry(CONTROL).unwrap().clone();
        let mut g = f.clone();
        let at = le64(&g, 0x58).unwrap() as usize + ctl.offset as usize;
        g[at + 4..at + 8].copy_from_slice(b"XXXX");
        let e = ChmReader.read(&g).unwrap_err();
        assert!(format!("{e:#}").contains("LZXC"), "{e:#}");
        let mut g = f.clone();
        g[at + 16..at + 20].copy_from_slice(&3u32.to_le_bytes()); // 3 * 0x8000 is no valid window
        assert!(format!("{:#}", ChmReader.read(&g).unwrap_err()).contains("window size"));
    }

    #[test]
    fn no_html_is_an_error() {
        let f = write_chm(&ChmSpec {
            files: vec![("/a.txt".into(), b"hi".to_vec())],
            compressed: false,
            ..Default::default()
        });
        assert!(format!("{}", ChmReader.read(&f).unwrap_err()).contains("no HTML pages"));
        let f = write_chm(&ChmSpec {
            files: vec![("/a.htm".into(), b"<html><body></body></html>".to_vec())],
            compressed: false,
            ..Default::default()
        });
        assert!(format!("{}", ChmReader.read(&f).unwrap_err()).contains("no readable text"));
    }

    #[test]
    fn version_2_header_is_supported() {
        let f = write_chm(&ChmSpec {
            files: pages(),
            compressed: false,
            version2: true,
            ..Default::default()
        });
        let book = validate(&ChmReader.read(&f).unwrap());
        assert_eq!(chapter_texts(&book).len(), 3);
    }

    #[test]
    fn ids_sniff_and_fuzz() {
        assert_eq!(ChmReader.id(), "chm");
        assert_eq!(ChmReader.extensions(), &["chm"]);
        assert!(ChmReader.sniff(b"ITSF\x03"));
        assert!(!ChmReader.sniff(b"ITOLITLS"));
        fuzz_no_panic(&ChmReader, &sample(true));
        fuzz_no_panic(&ChmReader, &sample(false));
        assert_rejects_garbage(&ChmReader);
    }
}
