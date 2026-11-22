//! Rocket eBook (`.rb`) writer.
//!
//! Little-endian container: a 0x128 byte header (magic `B0 0C B0 0C`, version 2, `NUVO`, table of
//! contents offset at 0x18, file length at 0x1C), the table of contents (page count, then 44 byte
//! entries: 32 byte name, length, offset, flags), and the pages: `info.info` (flag 2, `KEY=VALUE`
//! lines), `index.html` (flag 8: deflated in 4096 byte chunks, each its own zlib stream), an empty
//! `index.hidx` and the images as `N.png` (stored). The HTML is the Rocket dialect: upper-case tags,
//! Windows-1252 text (declared by a `META` tag so readers that sniff the charset agree) with `&#N;` for
//! other characters, `A NAME`/`A HREF` for internal links.

use std::collections::HashMap;
use std::io::Write;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;
use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::charset::{cp1252_byte, to_cp1252_lossy};
use crate::images::to_png;
use crate::model::{Align, Block, Fmt, Inline, Link, Model, flat_inlines};

#[derive(Debug, Clone, Copy, Default)]
pub struct RbWriter;

pub const HEADER_LEN: usize = 0x128;
pub const CHUNK: usize = 4096;
const FLAG_INFO: u32 = 2;
const FLAG_DEFLATED: u32 = 8;

/// Escapes text for the Rocket HTML dialect, appending bytes to `out`.
pub fn escape_into(s: &str, out: &mut Vec<u8>) {
    for ch in s.chars() {
        match ch {
            '&' => out.extend_from_slice(b"&amp;"),
            '<' => out.extend_from_slice(b"&lt;"),
            '>' => out.extend_from_slice(b"&gt;"),
            '\n' | '\r' | '\t' => out.push(b' '),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {}
            c => match cp1252_byte(c) {
                Some(b) => out.push(b),
                None => out.extend_from_slice(format!("&#{};", c as u32).as_bytes()),
            },
        }
    }
}

struct Html<'a> {
    book: &'a OpenBook,
    model: &'a Model,
    out: Vec<u8>,
    images: Vec<(String, Vec<u8>)>,
    image_names: HashMap<String, Option<String>>,
    next_image: usize,
    anchors: HashMap<String, String>,
}

impl<'a> Html<'a> {
    fn push(&mut self, s: &str) {
        self.out.extend_from_slice(s.as_bytes());
    }

    fn anchor(&mut self, key: &str) -> String {
        let n = self.anchors.len() + 1;
        self.anchors
            .entry(key.to_string())
            .or_insert_with(|| format!("bx{n}"))
            .clone()
    }

    fn image(&mut self, path: &str) -> Option<String> {
        if let Some(n) = self.image_names.get(path) {
            return n.clone();
        }
        let name = self
            .book
            .archive
            .get(path)
            .and_then(to_png)
            .map(|(png, _, _)| {
                let name = format!("{}.png", self.next_image);
                self.next_image += 1;
                self.images.push((name.clone(), png));
                name
            });
        self.image_names.insert(path.to_string(), name.clone());
        name
    }

    fn run(&mut self, text: &str, fmt: &Fmt, link: &Option<Link>) {
        let mut close: Vec<&str> = Vec::new();
        if let Some(Link::Internal(k)) = link {
            if self.model.linked.contains(k) {
                let n = self.anchor(k);
                self.push(&format!("<A HREF=\"#{n}\">"));
                close.push("</A>");
            }
        }
        for (on, open, shut) in [
            (fmt.bold, "<B>", "</B>"),
            (fmt.italic, "<I>", "</I>"),
            (fmt.underline, "<U>", "</U>"),
            (fmt.strike, "<STRIKE>", "</STRIKE>"),
            (fmt.sup, "<SUP>", "</SUP>"),
            (fmt.sub, "<SUB>", "</SUB>"),
            (fmt.mono, "<TT>", "</TT>"),
        ] {
            if on {
                self.push(open);
                close.push(shut);
            }
        }
        let mut buf = Vec::new();
        escape_into(text, &mut buf);
        self.out.extend(buf);
        for c in close.into_iter().rev() {
            self.push(c);
        }
    }

    fn inlines(&mut self, inl: &[Inline]) {
        for i in inl {
            match i {
                Inline::Text { text, fmt, link } => self.run(text, fmt, link),
                Inline::Image { path, .. } => {
                    if let Some(n) = self.image(path) {
                        self.push(&format!("<IMG SRC=\"{n}\">"));
                    }
                }
                Inline::Break => self.push("<BR>"),
                Inline::Anchor(k) => {
                    if self.model.linked.contains(k) {
                        let n = self.anchor(k);
                        self.push(&format!("<A NAME=\"{n}\"></A>"));
                    }
                }
            }
        }
    }

    fn blocks(&mut self, blocks: &[Block]) {
        for b in blocks {
            match b {
                Block::Heading { level, inlines, .. } => {
                    let l = (*level).clamp(1, 6);
                    self.push(&format!("<H{l}>"));
                    self.inlines(inlines);
                    self.push(&format!("</H{l}>"));
                }
                Block::Para { align, inlines } => {
                    self.push(match align {
                        Align::Left => "<P>",
                        Align::Center => "<P ALIGN=\"CENTER\">",
                        Align::Right => "<P ALIGN=\"RIGHT\">",
                    });
                    self.inlines(inlines);
                    self.push("</P>");
                }
                Block::Pre(inlines) => {
                    self.push("<PRE>");
                    for i in inlines {
                        if let Inline::Text { text, fmt, link } = i {
                            let mut parts = text.split('\n').peekable();
                            while let Some(p) = parts.next() {
                                self.run(p, fmt, link);
                                if parts.peek().is_some() {
                                    self.push("<BR>");
                                }
                            }
                        } else {
                            self.inlines(std::slice::from_ref(i));
                        }
                    }
                    self.push("</PRE>");
                }
                Block::Quote(inner) => {
                    self.push("<BLOCKQUOTE>");
                    self.blocks(inner);
                    self.push("</BLOCKQUOTE>");
                }
                Block::Rule => self.push("<HR>"),
                Block::List { ordered, items } => {
                    let tag = if *ordered { "OL" } else { "UL" };
                    self.push(&format!("<{tag}>"));
                    for item in items {
                        self.push("<LI>");
                        match item.first() {
                            Some(Block::Para { inlines, .. }) => {
                                self.inlines(inlines);
                                self.blocks(&item[1..]);
                            }
                            _ => self.blocks(item),
                        }
                        self.push("</LI>");
                    }
                    self.push(&format!("</{tag}>"));
                }
                Block::Table { rows } => {
                    for row in rows {
                        self.push("<P>");
                        for (n, c) in row.iter().enumerate() {
                            if n > 0 {
                                self.push(" | ");
                            }
                            self.inlines(&flat_inlines(&c.blocks));
                        }
                        self.push("</P>");
                    }
                }
            }
        }
    }
}

/// Deflates a page in 4096 byte chunks: chunk count, total length, chunk sizes, chunks.
pub fn compress_page(data: &[u8]) -> Result<Vec<u8>> {
    let mut chunks = Vec::new();
    for c in data.chunks(CHUNK) {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(c)?;
        chunks.push(e.finish()?);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&(chunks.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    for c in &chunks {
        out.extend_from_slice(&(c.len() as u32).to_le_bytes());
    }
    for c in &chunks {
        out.extend_from_slice(c);
    }
    Ok(out)
}

/// A page of the container: name (at most 31 bytes), content as stored, flags.
pub struct Page {
    pub name: String,
    pub data: Vec<u8>,
    pub flags: u32,
}

/// Assembles the container around ready-made pages.
pub fn build_container(pages: &[Page]) -> Vec<u8> {
    let toc_len = 4 + pages.len() * 44;
    let mut offset = HEADER_LEN + toc_len;
    let total = offset + pages.iter().map(|p| p.data.len()).sum::<usize>();
    let mut out = vec![0u8; HEADER_LEN];
    out[..4].copy_from_slice(&[0xb0, 0x0c, 0xb0, 0x0c]);
    out[4] = 2;
    out[6..10].copy_from_slice(b"NUVO");
    out[0x18..0x1c].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes());
    out[0x1c..0x20].copy_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(pages.len() as u32).to_le_bytes());
    for p in pages {
        let mut name = [0u8; 32];
        let n = p.name.len().min(31);
        name[..n].copy_from_slice(&p.name.as_bytes()[..n]);
        out.extend_from_slice(&name);
        out.extend_from_slice(&(p.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        out.extend_from_slice(&p.flags.to_le_bytes());
        offset += p.data.len();
    }
    for p in pages {
        out.extend_from_slice(&p.data);
    }
    out
}

/// The `KEY=VALUE` info page.
pub fn info_page(book: &OpenBook) -> Vec<u8> {
    let md = book.metadata();
    let mut lines = vec!["TYPE=2".to_string()];
    let mut add = |k: &str, v: &str| {
        let v = v.replace(['\n', '\r'], " ");
        if !v.trim().is_empty() {
            lines.push(format!("{k}={}", v.trim()));
        }
    };
    add("TITLE", &md.title);
    add("AUTHOR", &md.authors.join(" & "));
    add("PUB_NAME", &md.publisher);
    add("COPYRIGHT", &md.rights);
    add("TITLE_LANGUAGE", &md.language);
    add("GENRE", &md.tags.join(", "));
    add("ISBN", md.identifier("isbn").unwrap_or(""));
    lines.push("GENERATOR=Bookx".into());
    lines.push("PARSE=1".into());
    lines.push("OUTPUT=1".into());
    lines.push("BODY=index.html".into());
    let mut bytes = Vec::new();
    for l in lines {
        bytes.extend(to_cp1252_lossy(&l));
        bytes.push(b'\n');
    }
    bytes
}

impl FormatWriter for RbWriter {
    fn id(&self) -> &'static str {
        "rb"
    }
    fn extension(&self) -> &'static str {
        "rb"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let model = Model::build(book);
        let mut h = Html {
            book,
            model: &model,
            out: Vec::new(),
            images: Vec::new(),
            image_names: HashMap::new(),
            next_image: 0,
            anchors: HashMap::new(),
        };
        h.push("<HTML><HEAD><TITLE>");
        let mut t = Vec::new();
        escape_into(&book.metadata().title, &mut t);
        h.out.extend(t);
        h.push("</TITLE><META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=windows-1252\"></HEAD><BODY>");
        if let Some(c) = &model.cover {
            if let Some((png, _, _)) = to_png(&c.data) {
                h.images.push(("cover.png".into(), png));
                h.push("<P ALIGN=\"CENTER\"><IMG SRC=\"cover.png\"></P>");
            }
        }
        for s in &model.sections {
            if model.linked.contains(&s.path) {
                let n = h.anchor(&s.path);
                h.push(&format!("<A NAME=\"{n}\"></A>"));
            }
            h.blocks(&s.blocks);
        }
        h.push("</BODY></HTML>");

        let mut pages = vec![
            Page {
                name: "info.info".into(),
                data: info_page(book),
                flags: FLAG_INFO,
            },
            Page {
                name: "index.html".into(),
                data: compress_page(&h.out)?,
                flags: FLAG_DEFLATED,
            },
            Page {
                name: "index.hidx".into(),
                data: b" ".to_vec(),
                flags: 0,
            },
        ];
        for (name, png) in h.images {
            pages.push(Page {
                name,
                data: png,
                flags: 0,
            });
        }
        Ok(build_container(&pages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esc(s: &str) -> Vec<u8> {
        let mut o = Vec::new();
        escape_into(s, &mut o);
        o
    }

    #[test]
    fn escaping() {
        assert_eq!(esc("a&b<c>"), b"a&amp;b&lt;c&gt;");
        assert_eq!(esc("é“"), vec![0xe9, 0x93]);
        assert_eq!(esc("日"), b"&#26085;");
        assert_eq!(esc("😀"), b"&#128512;");
        assert_eq!(esc("a\nb\u{1}"), b"a b");
    }

    #[test]
    fn container_layout() {
        let f = build_container(&[
            Page {
                name: "a".into(),
                data: vec![1, 2, 3],
                flags: 0,
            },
            Page {
                name: "x".repeat(40),
                data: vec![9],
                flags: 8,
            },
        ]);
        assert_eq!(&f[..4], &[0xb0, 0x0c, 0xb0, 0x0c]);
        assert_eq!(&f[6..10], b"NUVO");
        let le = |o: usize| u32::from_le_bytes(f[o..o + 4].try_into().unwrap());
        assert_eq!(le(0x18), 0x128);
        assert_eq!(le(0x1c) as usize, f.len());
        assert_eq!(le(0x128), 2);
        let e1 = 0x128 + 4 + 44;
        assert_eq!(&f[e1..e1 + 31], "x".repeat(31).as_bytes());
        assert_eq!(f[e1 + 31], 0, "names are cut to 31 bytes");
        assert_eq!(le(e1 + 40), 8);
        let off0 = le(0x128 + 4 + 36) as usize;
        assert_eq!(&f[off0..off0 + 3], &[1, 2, 3]);
    }

    #[test]
    fn page_compression_layout() {
        let data = vec![b'a'; CHUNK * 2 + 10];
        let p = compress_page(&data).unwrap();
        let le = |o: usize| u32::from_le_bytes(p[o..o + 4].try_into().unwrap());
        assert_eq!(le(0), 3);
        assert_eq!(le(4) as usize, data.len());
        let sizes: usize = (0..3).map(|i| le(8 + i * 4) as usize).sum();
        assert_eq!(p.len(), 8 + 12 + sizes);
        let empty = compress_page(b"").unwrap();
        assert_eq!(empty, vec![0u8; 8]);
    }

    use crate::testutil::book_with;

    #[test]
    fn info_page_lists_known_fields_only() {
        let book = book_with(&["<p>x</p>"]);
        let info = String::from_utf8(info_page(&book)).unwrap();
        assert_eq!(
            info,
            "TYPE=2\nTITLE=Unit Book\nAUTHOR=A. Writer\nTITLE_LANGUAGE=en\nGENERATOR=Bookx\nPARSE=1\nOUTPUT=1\nBODY=index.html\n"
        );
    }

    #[test]
    fn writer_identity_and_html_for_a_tiny_book() {
        assert_eq!(RbWriter.id(), "rb");
        assert_eq!(RbWriter.extension(), "rb");
        let book = book_with(&["<h2>Sub</h2><p>a<b>b</b></p>"]);
        let f = RbWriter.write(&book).unwrap();
        assert_eq!(&f[..4], &[0xb0, 0x0c, 0xb0, 0x0c]);
        assert_eq!(u32::from_le_bytes(f[0x128..0x12c].try_into().unwrap()), 3);
    }
}
