//! SNB (Shanda Bambook) writer.
//!
//! Big-endian container: a 44 byte header (`SNBP000B`, 0x8000, 0x00A3A3A3, 0, file count, sizes of
//! the file table / its zlib stream / the binary stream / the plain stream, 0), the zlib compressed
//! file table (12 byte entries `attr, name offset, size`, then NUL separated names), the binary
//! stream (images, stored), the plain stream (`.snbc` / `.snbf` XML) as independent bzip2 blocks of
//! 32 KiB, a zlib compressed tail (absolute block offsets, then `block, offset` per file) and the
//! trailer (tail size, tail offset, magic). Chapters (`snbc/chapter_N.snbc`) hold one `<text>` per
//! paragraph (indented with two ideographic spaces like Calibre does) and `<img>` elements; images
//! are JPEG. Formatting and links are not representable in SNB and are dropped.

use std::io::Write;

use anyhow::{Context, Result};
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;
use bookx_core::util::xml_escape;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::images::to_jpeg;
use crate::model::{flat_inlines, inline_text, Block, Inline, Model};

#[derive(Debug, Clone, Copy, Default)]
pub struct SnbWriter;

pub const MAGIC: &[u8; 8] = b"SNBP000B";
pub const BLOCK: usize = 0x8000;
const ATTR_PLAIN: u32 = 0x4100_0000;
const ATTR_BINARY: u32 = 0x0100_0000;
const INDENT: &str = "\u{3000}\u{3000}";

/// A file of the container.
#[derive(Debug, Clone)]
pub struct SnbEntry {
    pub name: String,
    pub data: Vec<u8>,
    /// Binary files are stored, plain files go through bzip2.
    pub binary: bool,
}

fn zlib(data: &[u8]) -> Result<Vec<u8>> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data)?;
    Ok(e.finish()?)
}

fn bzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    e.write_all(data)?;
    Ok(e.finish()?)
}

/// Assembles an SNB container.
pub fn build_container(files: &[SnbEntry]) -> Result<Vec<u8>> {
    let mut vfat = Vec::new();
    let mut names = Vec::new();
    for f in files {
        let attr = if f.binary { ATTR_BINARY } else { ATTR_PLAIN };
        vfat.extend_from_slice(&attr.to_be_bytes());
        vfat.extend_from_slice(&(names.len() as u32).to_be_bytes());
        vfat.extend_from_slice(&(f.data.len() as u32).to_be_bytes());
        names.extend_from_slice(f.name.as_bytes());
        names.push(0);
    }
    vfat.extend_from_slice(&names);
    let vfat_c = zlib(&vfat)?;

    let bin: Vec<u8> = files
        .iter()
        .filter(|f| f.binary)
        .flat_map(|f| f.data.iter().copied())
        .collect();
    let plain: Vec<u8> = files
        .iter()
        .filter(|f| !f.binary)
        .flat_map(|f| f.data.iter().copied())
        .collect();
    let bin_blocks = bin.len().div_ceil(BLOCK);
    let plain_blocks = plain.len().div_ceil(BLOCK);

    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    for v in [
        BLOCK as u32,
        0x00a3_a3a3,
        0,
        files.len() as u32,
        vfat.len() as u32,
        vfat_c.len() as u32,
        bin.len() as u32,
        plain.len() as u32,
        0,
    ] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&vfat_c);
    let bin_start = out.len();
    out.extend_from_slice(&bin);

    let mut tail = Vec::new();
    for k in 0..bin_blocks {
        tail.extend_from_slice(&((bin_start + k * BLOCK) as u32).to_be_bytes());
    }
    for chunk in plain.chunks(BLOCK) {
        tail.extend_from_slice(&(out.len() as u32).to_be_bytes());
        out.extend_from_slice(&bzip(chunk)?);
    }
    let (mut bin_pos, mut plain_pos) = (0usize, 0usize);
    for f in files {
        let (block, off) = if f.binary {
            let p = bin_pos;
            bin_pos += f.data.len();
            (p / BLOCK, p % BLOCK)
        } else {
            let p = plain_pos;
            plain_pos += f.data.len();
            (bin_blocks + p / BLOCK, p % BLOCK)
        };
        tail.extend_from_slice(&(block as u32).to_be_bytes());
        tail.extend_from_slice(&(off as u32).to_be_bytes());
    }
    debug_assert_eq!(
        tail.len(),
        (bin_blocks + plain_blocks) * 4 + files.len() * 8
    );
    let tail_c = zlib(&tail)?;
    let tail_offset = out.len();
    out.extend_from_slice(&tail_c);
    out.extend_from_slice(&(tail_c.len() as u32).to_be_bytes());
    out.extend_from_slice(&(tail_offset as u32).to_be_bytes());
    out.extend_from_slice(MAGIC);
    Ok(out)
}

enum Line {
    Text(String),
    Image(String),
}

struct Chapter {
    lines: Vec<Line>,
}

struct Collector<'a> {
    book: &'a OpenBook,
    images: Vec<(String, Vec<u8>)>,
    names: std::collections::HashMap<String, Option<String>>,
    next_image: usize,
}

impl Collector<'_> {
    fn image(&mut self, path: &str) -> Option<String> {
        if let Some(n) = self.names.get(path) {
            return n.clone();
        }
        let name = self
            .book
            .archive
            .get(path)
            .and_then(to_jpeg)
            .map(|(jpg, _, _)| {
                self.next_image += 1;
                let name = format!("img{}.jpg", self.next_image);
                self.images.push((name.clone(), jpg));
                name
            });
        self.names.insert(path.to_string(), name.clone());
        name
    }

    fn paragraph(&mut self, inl: &[Inline], marker: &str, out: &mut Vec<Line>) {
        let mut cur = marker.to_string();
        let flush = |cur: &mut String, out: &mut Vec<Line>| {
            let t = cur.trim().to_string();
            if !t.is_empty() {
                out.push(Line::Text(t));
            }
            cur.clear();
        };
        for i in inl {
            match i {
                Inline::Text { text, .. } => cur.push_str(&text.replace(['\n', '\r', '\t'], " ")),
                Inline::Break => {
                    flush(&mut cur, out);
                }
                Inline::Image { path, .. } => {
                    flush(&mut cur, out);
                    if let Some(n) = self.image(path) {
                        out.push(Line::Image(n));
                    }
                }
                Inline::Anchor(_) => {}
            }
        }
        flush(&mut cur, out);
    }

    fn blocks(&mut self, blocks: &[Block], out: &mut Vec<Line>) {
        for b in blocks {
            match b {
                Block::Heading { inlines, .. } | Block::Para { inlines, .. } => {
                    self.paragraph(inlines, "", out)
                }
                Block::Pre(inlines) => {
                    for part in crate::model::split_lines(inlines).split(|i| *i == Inline::Break) {
                        let t = inline_text(part);
                        if !t.trim().is_empty() {
                            out.push(Line::Text(t.trim_end().to_string()));
                        }
                    }
                }
                Block::Quote(inner) => self.blocks(inner, out),
                Block::Rule => {}
                Block::List { ordered, items } => {
                    for (n, item) in items.iter().enumerate() {
                        let marker = if *ordered {
                            format!("{}. ", n + 1)
                        } else {
                            "\u{2022} ".to_string()
                        };
                        let mut first = true;
                        for b in item {
                            match b {
                                Block::Para { inlines, .. } if first => {
                                    self.paragraph(inlines, &marker, out)
                                }
                                other => self.blocks(std::slice::from_ref(other), out),
                            }
                            first = false;
                        }
                    }
                }
                Block::Table { rows } => {
                    for row in rows {
                        let mut line: Vec<Inline> = Vec::new();
                        for (n, c) in row.iter().enumerate() {
                            if n > 0 {
                                line.push(Inline::Text {
                                    text: " | ".into(),
                                    fmt: Default::default(),
                                    link: None,
                                });
                            }
                            line.extend(flat_inlines(&c.blocks));
                        }
                        self.paragraph(&line, "", out);
                    }
                }
            }
        }
    }
}

fn cdata(s: &str) -> String {
    format!("<![CDATA[{}]]>", s.replace("]]>", "]]]]><![CDATA[>"))
}

fn chapter_xml(title: &str, ch: &Chapter) -> String {
    let mut s = format!(
        "<snbc>\n  <head>\n    <title>{}</title>\n  </head>\n  <body>\n",
        xml_escape(title)
    );
    for l in &ch.lines {
        match l {
            Line::Text(t) => s.push_str(&format!(
                "    <text>{}</text>\n",
                cdata(&format!("{INDENT}{t}"))
            )),
            Line::Image(n) => s.push_str(&format!("    <img>{}</img>\n", xml_escape(n))),
        }
    }
    s.push_str("  </body>\n</snbc>\n");
    s
}

impl FormatWriter for SnbWriter {
    fn id(&self) -> &'static str {
        "snb"
    }
    fn extension(&self) -> &'static str {
        "snb"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let model = Model::build(book);
        let md = book.metadata();
        let mut col = Collector {
            book,
            images: Vec::new(),
            names: Default::default(),
            next_image: 0,
        };
        let mut cover_name = String::new();
        if let Some(c) = &model.cover {
            if let Some((jpg, _, _)) = to_jpeg(&c.data) {
                cover_name = "cover.jpg".to_string();
                col.images.push((cover_name.clone(), jpg));
            }
        }
        let mut chapters: Vec<(String, String)> = Vec::new(); // file name, title
        let mut files: Vec<SnbEntry> = Vec::new();
        for (i, s) in model.sections.iter().enumerate() {
            let mut ch = Chapter { lines: Vec::new() };
            col.blocks(&s.blocks, &mut ch.lines);
            if ch.lines.is_empty() {
                continue;
            }
            let title = model
                .section_title(i)
                .unwrap_or_else(|| format!("Chapter {}", chapters.len() + 1));
            let file = format!("chapter_{}.snbc", chapters.len() + 1);
            files.push(SnbEntry {
                name: format!("snbc/{file}"),
                data: chapter_xml(&title, &ch).into_bytes(),
                binary: false,
            });
            chapters.push((file, title));
        }
        if chapters.is_empty() {
            let ch = Chapter {
                lines: vec![Line::Text(String::new())],
            };
            files.push(SnbEntry {
                name: "snbc/chapter_1.snbc".into(),
                data: chapter_xml(&md.title, &ch).into_bytes(),
                binary: false,
            });
            chapters.push(("chapter_1.snbc".into(), md.title.clone()));
        }
        for (name, data) in &col.images {
            files.push(SnbEntry {
                name: format!("snbc/images/{name}"),
                data: data.clone(),
                binary: true,
            });
        }
        let book_snbf = format!(
            "<book-snbf version=\"1.0\">\n  <head>\n    <name>{}</name>\n    <author>{}</author>\n    <language>{}</language>\n    <rights>{}</rights>\n    <publisher>{}</publisher>\n    <generator>Bookx</generator>\n    <created>{}</created>\n    <abstract>{}</abstract>\n    <cover>{}</cover>\n  </head>\n</book-snbf>\n",
            xml_escape(&md.title),
            xml_escape(&md.authors.join(" & ")),
            xml_escape(&md.language.to_ascii_uppercase()),
            xml_escape(&md.rights),
            xml_escape(&md.publisher),
            xml_escape(&md.published),
            xml_escape(&crate::markup::plain_text(&md.description)),
            xml_escape(&cover_name)
        );
        let mut toc = format!(
            "<toc-snbf>\n  <head>\n    <chapters>{}</chapters>\n  </head>\n  <body>\n",
            chapters.len()
        );
        for (file, title) in &chapters {
            toc.push_str(&format!(
                "    <chapter src=\"{file}\">{}</chapter>\n",
                xml_escape(title)
            ));
        }
        toc.push_str("  </body>\n</toc-snbf>\n");
        files.push(SnbEntry {
            name: "snbf/book.snbf".into(),
            data: book_snbf.into_bytes(),
            binary: false,
        });
        files.push(SnbEntry {
            name: "snbf/toc.snbf".into(),
            data: toc.into_bytes(),
            binary: false,
        });
        build_container(&files).context("building the SNB container")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, data: Vec<u8>, binary: bool) -> SnbEntry {
        SnbEntry {
            name: name.into(),
            data,
            binary,
        }
    }

    #[test]
    fn cdata_splits_the_terminator() {
        assert_eq!(cdata("a"), "<![CDATA[a]]>");
        assert_eq!(cdata("a]]>b"), "<![CDATA[a]]]]><![CDATA[>b]]>");
        let x = format!("<t>{}</t>", cdata("x]]>y"));
        let doc = roxmltree::Document::parse(&x).unwrap();
        assert_eq!(doc.root_element().text(), Some("x]]>y"));
    }

    #[test]
    fn container_header_and_trailer() {
        let f = build_container(&[
            entry("a.snbc", b"<snbc/>".to_vec(), false),
            entry("i.jpg", vec![1, 2, 3], true),
        ])
        .unwrap();
        assert_eq!(&f[..8], MAGIC);
        assert_eq!(&f[f.len() - 8..], MAGIC);
        let be = |o: usize| u32::from_be_bytes(f[o..o + 4].try_into().unwrap());
        assert_eq!(be(8), 0x8000);
        assert_eq!(be(12), 0x00a3_a3a3);
        assert_eq!(be(20), 2, "file count");
        assert_eq!(be(32), 3, "binary size");
        assert_eq!(be(36), 7, "plain size");
        let tail_off = be(f.len() - 12) as usize;
        let tail_len = be(f.len() - 16) as usize;
        assert_eq!(tail_off + tail_len + 16, f.len());
    }

    #[test]
    fn large_plain_streams_use_several_blocks() {
        let big = vec![b'x'; BLOCK * 2 + 5];
        let f = build_container(&[entry("a.snbc", big, false)]).unwrap();
        let be = |o: usize| u32::from_be_bytes(f[o..o + 4].try_into().unwrap());
        assert_eq!(be(36) as usize, BLOCK * 2 + 5);
        assert!(f.windows(3).filter(|w| *w == b"BZh").count() >= 3);
    }

    #[test]
    fn chapter_xml_escapes_and_indents() {
        let ch = Chapter {
            lines: vec![Line::Text("a & b".into()), Line::Image("i.jpg".into())],
        };
        let x = chapter_xml("T<1>", &ch);
        let doc = roxmltree::Document::parse(&x).unwrap();
        let t = doc.descendants().find(|n| n.has_tag_name("text")).unwrap();
        assert_eq!(t.text().unwrap(), "\u{3000}\u{3000}a & b");
        assert!(x.contains("<title>T&lt;1&gt;</title>"));
        assert!(x.contains("<img>i.jpg</img>"));
    }

    use crate::testutil::book_with;

    #[test]
    fn writer_identity_and_chapter_collection() {
        assert_eq!(SnbWriter.id(), "snb");
        assert_eq!(SnbWriter.extension(), "snb");
        let book = book_with(&["<p>a<br/>b <img src=\"pic.png\"/> c</p>"]);
        let model = Model::build(&book);
        let mut col = Collector {
            book: &book,
            images: Vec::new(),
            names: Default::default(),
            next_image: 0,
        };
        let mut lines = Vec::new();
        col.blocks(&model.sections[0].blocks, &mut lines);
        let kinds: Vec<String> = lines
            .iter()
            .map(|l| match l {
                Line::Text(t) => format!("t:{t}"),
                Line::Image(n) => format!("i:{n}"),
            })
            .collect();
        assert_eq!(kinds, ["t:a", "t:b", "i:img1.jpg", "t:c"]);
        assert_eq!(col.images.len(), 1);
        let bytes = SnbWriter.write(&book).unwrap();
        assert!(bytes.starts_with(MAGIC));
    }
}
