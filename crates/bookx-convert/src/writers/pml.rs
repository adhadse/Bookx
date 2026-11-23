//! Palm Markup Language: the text generator shared by the eReader PDB writer and the PMLZ writer.
//!
//! Output is pure ASCII: Latin-1 range characters become `\aNNN` (Windows-1252 code), other
//! characters in the BMP `\UXXXX`, characters outside the BMP `\Ufffd` (hex digits are lower case, which Calibre requires). Chapters (`\x`, `\X0`..`\X4`)
//! come from headings, links become `\q="#id"…\q` with `\Q="id"` anchors, images `\m="name.png"`.
//! External links keep only their text (PML has no URL links).

use std::collections::HashMap;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;

use crate::charset::cp1252_byte;
use crate::images::to_png;
use crate::model::{Align, Block, Fmt, Inline, Link, Model, flat_inlines, inline_text};

use super::flat::{flat_opf, zip_files};

/// An image of a PML book: always PNG.
#[derive(Debug, Clone)]
pub struct PmlImage {
    pub name: String,
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A chapter heading of the PML text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmlChapter {
    /// Byte offset of the chapter code in `text`.
    pub offset: usize,
    /// The heading level, 1 (`\x`) to 6 (`\X4`).
    pub level: u8,
    pub title: String,
}

#[derive(Debug, Clone, Default)]
pub struct PmlText {
    pub text: String,
    pub images: Vec<PmlImage>,
    pub chapters: Vec<PmlChapter>,
    /// Anchors (`\Q` codes): byte offset in `text` and name; they become the link index records of
    /// eReader files.
    pub links: Vec<(usize, String)>,
    /// Number of visible characters (no markup), stored by the eReader header.
    pub visible_chars: usize,
}

/// Appends `s` to `out` with PML escapes.
pub fn escape_into(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {}
            c if c.is_ascii() => out.push(c),
            c => match cp1252_byte(c) {
                Some(b) => out.push_str(&format!("\\a{b:03}")),
                None if (c as u32) <= 0xffff => out.push_str(&format!("\\U{:04x}", c as u32)),
                None => out.push_str("\\Ufffd"),
            },
        }
    }
}

struct Renderer<'a> {
    book: &'a OpenBook,
    model: &'a Model,
    out: String,
    images: Vec<PmlImage>,
    image_names: HashMap<String, Option<String>>,
    next_image: usize,
    anchor_names: HashMap<String, String>,
    chapters: Vec<PmlChapter>,
    links: Vec<(usize, String)>,
    fmt: Fmt,
    link: Option<String>,
}

fn heading_code(level: u8) -> String {
    match level {
        0 | 1 => "\\x".to_string(),
        n => format!("\\X{}", (n - 2).min(4)),
    }
}

impl<'a> Renderer<'a> {
    fn anchor_name(&mut self, key: &str) -> String {
        let n = self.anchor_names.len() + 1;
        self.anchor_names
            .entry(key.to_string())
            .or_insert_with(|| format!("bx{n}"))
            .clone()
    }

    fn emit_anchor(&mut self, key: &str) {
        let n = self.anchor_name(key);
        self.links.push((self.out.len(), n.clone()));
        self.out.push_str(&format!("\\Q=\"{n}\""));
    }

    fn image_name(&mut self, path: &str) -> Option<String> {
        if let Some(n) = self.image_names.get(path) {
            return n.clone();
        }
        let name = self
            .book
            .archive
            .get(path)
            .and_then(to_png)
            .map(|(png, width, height)| {
                self.next_image += 1;
                let name = format!("img{}.png", self.next_image);
                self.images.push(PmlImage {
                    name: name.clone(),
                    png,
                    width,
                    height,
                });
                name
            });
        self.image_names.insert(path.to_string(), name.clone());
        name
    }

    fn toggle(&mut self, code: &str) {
        self.out.push_str(code);
    }

    fn set_fmt(&mut self, new: Fmt) {
        let old = self.fmt;
        let pairs = [
            (old.bold, new.bold, "\\B"),
            (old.italic, new.italic, "\\i"),
            (old.underline, new.underline, "\\u"),
            (old.strike, new.strike, "\\o"),
            (old.sup, new.sup, "\\Sp"),
            (old.sub, new.sub, "\\Sb"),
            (old.small, new.small, "\\k"),
        ];
        for (a, b, code) in pairs {
            if a != b {
                self.toggle(code);
            }
        }
        self.fmt = Fmt {
            mono: new.mono,
            ..new
        };
    }

    fn set_link(&mut self, new: Option<String>) {
        if self.link == new {
            return;
        }
        if self.link.is_some() {
            self.out.push_str("\\q");
        }
        if let Some(n) = &new {
            self.out.push_str(&format!("\\q=\"#{n}\""));
        }
        self.link = new;
    }

    fn close_inline(&mut self) {
        self.set_link(None);
        self.set_fmt(Fmt::default());
    }

    fn inlines(&mut self, inl: &[Inline]) {
        for i in inl {
            match i {
                Inline::Text { text, fmt, link } => {
                    let target = match link {
                        Some(Link::Internal(k)) if self.model.linked.contains(k) => {
                            Some(self.anchor_name(k))
                        }
                        _ => None,
                    };
                    self.set_fmt(*fmt);
                    self.set_link(target);
                    escape_into(text, &mut self.out);
                }
                Inline::Image { path, .. } => {
                    if let Some(n) = self.image_name(path) {
                        self.close_inline();
                        self.out.push_str(&format!("\\m=\"{n}\""));
                    }
                }
                Inline::Break => {
                    self.close_inline();
                    self.out.push('\n');
                }
                Inline::Anchor(k) => {
                    if self.model.linked.contains(k) {
                        self.emit_anchor(k);
                    }
                }
            }
        }
        self.close_inline();
    }

    fn paragraph(&mut self, inl: &[Inline], align: Align, indent: bool) {
        let marks = match (align, indent) {
            (Align::Center, _) => "\\c",
            (Align::Right, _) => "\\r",
            (_, true) => "\\t",
            _ => "",
        };
        self.out.push_str(marks);
        self.inlines(inl);
        self.out.push_str(marks);
        self.out.push_str("\n\n");
    }

    fn blocks(&mut self, blocks: &[Block], indent: bool) {
        for b in blocks {
            match b {
                Block::Heading { level, inlines, .. } => {
                    let (anchors, rest): (Vec<_>, Vec<_>) = inlines
                        .iter()
                        .cloned()
                        .partition(|i| matches!(i, Inline::Anchor(_)));
                    self.inlines(&anchors);
                    let title = inline_text(&rest).trim().to_string();
                    let code = heading_code(*level);
                    self.chapters.push(PmlChapter {
                        offset: self.out.len(),
                        level: *level,
                        title,
                    });
                    self.out.push_str(&code);
                    self.out.push_str("\\B");
                    self.fmt.bold = true;
                    let mut rest = rest;
                    for i in &mut rest {
                        if let Inline::Text { fmt, .. } = i {
                            fmt.bold = true;
                        }
                    }
                    self.inlines(&rest);
                    self.out.push_str(&code);
                    self.out.push_str("\n\n");
                }
                Block::Para { align, inlines } => self.paragraph(inlines, *align, indent),
                Block::Pre(inlines) => {
                    self.out.push_str(if indent { "\\t" } else { "" });
                    // one PML line per source line
                    let mut line: Vec<Inline> = Vec::new();
                    let mut lines: Vec<Vec<Inline>> = Vec::new();
                    for i in inlines {
                        match i {
                            Inline::Text { text, fmt, link } => {
                                let mut parts = text.split('\n').peekable();
                                while let Some(p) = parts.next() {
                                    if !p.is_empty() {
                                        line.push(Inline::Text {
                                            text: p.to_string(),
                                            fmt: *fmt,
                                            link: link.clone(),
                                        });
                                    }
                                    if parts.peek().is_some() {
                                        lines.push(std::mem::take(&mut line));
                                    }
                                }
                            }
                            other => line.push(other.clone()),
                        }
                    }
                    lines.push(line);
                    for (n, l) in lines.iter().enumerate() {
                        if n > 0 {
                            self.out.push('\n');
                        }
                        self.inlines(l);
                    }
                    self.out.push_str(if indent { "\\t" } else { "" });
                    self.out.push_str("\n\n");
                }
                Block::Quote(inner) => self.blocks(inner, true),
                Block::Rule => self.out.push_str("\\w=\"50%\"\n\n"),
                Block::List { ordered, items } => self.list(*ordered, items, indent),
                Block::Table { rows } => {
                    for row in rows {
                        let mut line: Vec<Inline> = Vec::new();
                        for (n, c) in row.iter().enumerate() {
                            if n > 0 {
                                line.push(Inline::Text {
                                    text: " | ".into(),
                                    fmt: Fmt::default(),
                                    link: None,
                                });
                            }
                            line.extend(flat_inlines(&c.blocks));
                        }
                        self.paragraph(&line, Align::Left, indent);
                    }
                }
            }
        }
    }

    fn list(&mut self, ordered: bool, items: &[Vec<Block>], indent: bool) {
        for (n, item) in items.iter().enumerate() {
            let marker = if ordered {
                format!("{}. ", n + 1)
            } else {
                "\u{2022} ".to_string()
            };
            let mut first = true;
            for b in item {
                match b {
                    Block::Para { align, inlines } if first => {
                        let mut v = vec![Inline::Text {
                            text: marker.clone(),
                            fmt: Fmt::default(),
                            link: None,
                        }];
                        v.extend(inlines.iter().cloned());
                        self.paragraph(&v, *align, indent);
                    }
                    Block::List { ordered, items } => self.list(*ordered, items, true),
                    other => {
                        if first {
                            let m = [Inline::Text {
                                text: marker.clone(),
                                fmt: Fmt::default(),
                                link: None,
                            }];
                            self.paragraph(&m, Align::Left, indent);
                        }
                        self.blocks(std::slice::from_ref(other), true);
                    }
                }
                first = false;
            }
        }
    }
}

/// Renders the whole book as PML.
pub fn render_pml(book: &OpenBook, model: &Model) -> PmlText {
    let mut r = Renderer {
        book,
        model,
        out: String::new(),
        images: Vec::new(),
        image_names: HashMap::new(),
        next_image: 0,
        anchor_names: HashMap::new(),
        chapters: Vec::new(),
        links: Vec::new(),
        fmt: Fmt::default(),
        link: None,
    };
    if let Some(c) = &model.cover {
        if let Some((png, width, height)) = to_png(&c.data) {
            r.images.push(PmlImage {
                name: "cover.png".into(),
                png,
                width,
                height,
            });
            r.out.push_str("\\c\\m=\"cover.png\"\\c\n\n");
        }
    }
    for (i, s) in model.sections.iter().enumerate() {
        if i > 0 || !r.out.is_empty() {
            r.out.push_str("\\p\n");
        }
        if model.linked.contains(&s.path) {
            r.emit_anchor(&s.path);
        }
        r.blocks(&s.blocks, false);
    }
    let visible_chars = visible_len(&r.out);
    PmlText {
        text: r.out,
        images: r.images,
        chapters: r.chapters,
        links: r.links,
        visible_chars,
    }
}

/// Number of characters a reader shows: markup codes and their arguments do not count, `\aNNN`
/// and `\UXXXX` count as one.
pub fn visible_len(pml: &str) -> usize {
    let c: Vec<char> = pml.chars().collect();
    let (mut i, mut n) = (0, 0);
    while i < c.len() {
        if c[i] != '\\' {
            n += 1;
            i += 1;
            continue;
        }
        i += 1;
        match c.get(i) {
            Some('\\') => {
                n += 1;
                i += 1;
            }
            Some('a') => {
                n += 1;
                i += 4;
            }
            Some('U') => {
                n += 1;
                i += 5;
            }
            Some('X' | 'S') => i += 2,
            Some('q' | 'Q' | 'm' | 'w' | 'v') => {
                i += 1;
                if c.get(i) == Some(&'=') {
                    i += 2;
                    while i < c.len() && c[i] != '"' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            Some(_) => i += 1,
            None => {}
        }
    }
    n
}

/// PMLZ: a zip with `index.pml`, `index_img/*.png` and a `metadata.opf`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PmlzWriter;

fn invisible_metadata(book: &OpenBook) -> String {
    let md = book.metadata();
    let clean = |s: &str| {
        let mut o = String::new();
        escape_into(&s.replace('"', "'"), &mut o);
        o
    };
    let mut pairs = Vec::new();
    if !md.title.is_empty() {
        pairs.push(format!("TITLE=\"{}\"", clean(&md.title)));
    }
    if !md.authors.is_empty() {
        pairs.push(format!("AUTHOR=\"{}\"", clean(&md.authors.join(" & "))));
    }
    if !md.publisher.is_empty() {
        pairs.push(format!("PUBLISHER=\"{}\"", clean(&md.publisher)));
    }
    if !md.rights.is_empty() {
        pairs.push(format!("COPYRIGHT=\"{}\"", clean(&md.rights)));
    }
    if let Some(isbn) = md.identifier("isbn") {
        pairs.push(format!("EISBN=\"{}\"", clean(isbn)));
    }
    if pairs.is_empty() {
        String::new()
    } else {
        format!("\\v{}\\v\n\n", pairs.join(" "))
    }
}

impl FormatWriter for PmlzWriter {
    fn id(&self) -> &'static str {
        "pmlz"
    }
    fn extension(&self) -> &'static str {
        "pmlz"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let model = Model::build(book);
        let pml = render_pml(book, &model);
        let text = format!("{}{}", invisible_metadata(book), pml.text);
        let mut files: Vec<(String, Vec<u8>)> = vec![("index.pml".into(), text.into_bytes())];
        for img in &pml.images {
            let name = format!("index_img/{}", img.name);
            files.push((name, img.png.clone()));
        }
        let cover_id = pml.images.iter().any(|i| i.name == "cover.png");
        let mut items: Vec<(&str, &str, &str)> = vec![("text", "index.pml", "text/plain")];
        if cover_id {
            items.push(("cover", "index_img/cover.png", "image/png"));
        }
        files.push((
            "metadata.opf".into(),
            flat_opf(book, &items, cover_id.then_some("cover")).into_bytes(),
        ));
        zip_files(&files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esc(s: &str) -> String {
        let mut o = String::new();
        escape_into(s, &mut o);
        o
    }

    #[test]
    fn escapes() {
        assert_eq!(esc("plain"), "plain");
        assert_eq!(esc("a\\b"), "a\\\\b");
        assert_eq!(esc("“q”"), "\\a147q\\a148");
        assert_eq!(esc("日"), "\\U65e5");
        assert_eq!(esc("😀"), "\\Ufffd");
        assert_eq!(esc("a\nb\tc\u{1}d"), "a b cd");
        assert_eq!(esc("é"), "\\a233");
    }

    #[test]
    fn visible_length_ignores_markup() {
        assert_eq!(visible_len("abc"), 3);
        assert_eq!(visible_len("\\x\\BHi\\B\\x\n"), 3);
        assert_eq!(visible_len("a\\\\b"), 3);
        assert_eq!(visible_len("\\a233\\U65e5"), 2);
        assert_eq!(visible_len("\\q=\"#x\"link\\q\\Q=\"y\"\\m=\"i.png\""), 4);
        assert_eq!(visible_len("\\X0t\\X0\\Sptwo\\Sp"), 4);
        assert_eq!(visible_len(""), 0);
        assert_eq!(visible_len("trailing\\"), 8);
    }

    #[test]
    fn heading_codes() {
        assert_eq!(heading_code(1), "\\x");
        assert_eq!(heading_code(2), "\\X0");
        assert_eq!(heading_code(6), "\\X4");
        assert_eq!(heading_code(0), "\\x");
    }

    use crate::testutil::book_with;

    #[test]
    fn renders_headings_styles_links_and_images() {
        let book = book_with(&[
            "<h1 id=\"a\">Top</h1><p>Some <b>bold <i>both</i></b> <a href=\"#a\">back</a> <img src=\"pic.png\"/></p><h3>Deep</h3><blockquote><p>q</p></blockquote><ul><li>x</li></ul><hr/>",
        ]);
        let model = Model::build(&book);
        let pml = render_pml(&book, &model);
        assert!(pml.text.contains("\\x\\BTop\\B\\x"));
        assert!(pml.text.contains("\\Bbold \\i"), "{}", pml.text);
        assert!(pml.text.contains("\\q=\"#bx"));
        assert!(pml.text.contains("\\m=\"img1.png\""));
        assert!(pml.text.contains("\\X1\\BDeep\\B\\X1"));
        assert!(pml.text.contains("\\tq\\t"), "quotes are indented blocks");
        assert!(!pml.text.contains('\u{2022}') && pml.text.contains("\\a149 x"));
        assert!(pml.text.contains("\\w=\"50%\""));
        assert_eq!(pml.images.len(), 1);
        assert_eq!((pml.images[0].width, pml.images[0].height), (8, 6));
        let titles: Vec<_> = pml
            .chapters
            .iter()
            .map(|c| (c.level, c.title.as_str()))
            .collect();
        assert_eq!(titles, [(1, "Top"), (3, "Deep")]);
        for c in &pml.chapters {
            assert!(
                pml.text[c.offset..].starts_with("\\x") || pml.text[c.offset..].starts_with("\\X")
            );
        }
        assert_eq!(pml.visible_chars, visible_len(&pml.text));
        for (off, name) in &pml.links {
            assert!(pml.text[*off..].starts_with(&format!("\\Q=\"{name}\"")));
        }
    }

    #[test]
    fn external_links_and_broken_images_degrade_to_text() {
        let book = book_with(&[
            "<p><a href=\"https://x.org\">site</a> <img src=\"missing.png\" alt=\"alt text\"/></p>",
        ]);
        let pml = render_pml(&book, &Model::build(&book));
        assert!(pml.text.contains("site alt text"), "{}", pml.text);
        assert!(!pml.text.contains("x.org") && pml.images.is_empty());
    }

    #[test]
    fn pmlz_writer_identity() {
        assert_eq!(PmlzWriter.id(), "pmlz");
        assert_eq!(PmlzWriter.extension(), "pmlz");
        let book = book_with(&["<p>x</p>"]);
        let bytes = PmlzWriter.write(&book).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert!(
            invisible_metadata(&book).starts_with("\\vTITLE=\"Unit Book\" AUTHOR=\"A. Writer\"\\v")
        );
    }
}
