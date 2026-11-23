//! DOCX (WordprocessingML) writer.
//!
//! Package: `[Content_Types].xml`, `_rels/.rels`, `docProps/core.xml` and `app.xml`,
//! `word/document.xml`, `styles.xml` (Normal, Heading1-6, Title, Quote, Code, ListParagraph,
//! TOC1-3, Hyperlink, TableGrid), `numbering.xml` (bullets and one numbering instance per ordered
//! list so every list restarts at 1), `settings.xml`, `word/_rels/document.xml.rels` and the images
//! under `word/media/`. Hyperlinks are relationships (external) or `w:anchor` references to
//! bookmarks (internal), sections are separated by `w:pageBreakBefore`, and an optional `TOC`
//! field lists the table of contents.

use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;
use bookx_core::util::xml_escape;
use serde::{Deserialize, Serialize};

use crate::images::{Kind, image_size, kind_of, to_png};
use crate::model::{Align, Block, Cell, Fmt, Inline, Link, Model, flat_inlines, split_lines};

use super::flat::zip_files;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageSize {
    #[default]
    Letter,
    A4,
    A5,
    /// Width and height in twips (1/1440 inch).
    Custom(u32, u32),
}

impl PageSize {
    /// `(width, height)` in twips.
    pub fn twips(self) -> (u32, u32) {
        match self {
            PageSize::Letter => (12240, 15840),
            PageSize::A4 => (11906, 16838),
            PageSize::A5 => (8392, 11906),
            PageSize::Custom(w, h) => (w.clamp(2880, 31680), h.clamp(2880, 31680)),
        }
    }
}

/// Output options of the DOCX writer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocxOptions {
    pub page: PageSize,
    /// Page margin on all four sides, in twips.
    pub margin_twips: u32,
    /// Body font family.
    pub font: String,
    /// Body font size in points.
    pub font_size_pt: f32,
    /// Insert a "Contents" page with a `TOC` field built from the book's table of contents.
    pub toc: bool,
}

impl Default for DocxOptions {
    fn default() -> Self {
        DocxOptions {
            page: PageSize::Letter,
            margin_twips: 1440,
            font: "Times New Roman".into(),
            font_size_pt: 12.0,
            toc: false,
        }
    }
}

impl DocxOptions {
    fn half_points(&self) -> u32 {
        ((self.font_size_pt.clamp(4.0, 72.0) * 2.0).round()) as u32
    }

    fn margin(&self) -> u32 {
        let (w, h) = self.page.twips();
        self.margin_twips.min(w.min(h) / 3)
    }
}

#[derive(Debug, Clone, Default)]
pub struct DocxWriter {
    pub options: DocxOptions,
}

const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const EMU_PER_PX: u64 = 9525;
const EMU_PER_TWIP: u64 = 635;

struct Media {
    name: String,
    data: Vec<u8>,
    rel_id: String,
    width: u32,
    height: u32,
}

struct Docx<'a> {
    book: &'a OpenBook,
    model: &'a Model,
    opts: &'a DocxOptions,
    body: String,
    links: Vec<(String, String)>, // rel id, external target
    media: Vec<Media>,
    media_by_path: HashMap<String, Option<usize>>,
    bookmarks: HashMap<String, (u32, String)>,
    pending_bookmarks: Vec<String>,
    /// (num id, ordered)
    nums: Vec<(u32, bool)>,
    next_rel: u32,
    drawing_id: u32,
    page_break: bool,
    text_width_emu: u64,
}

fn clean(s: &str) -> String {
    let filtered: String = s
        .chars()
        .filter(|c| {
            matches!(*c, '\t' | '\n' | '\r')
                || (*c as u32 >= 0x20 && !matches!(*c as u32, 0xfffe | 0xffff))
        })
        .collect();
    xml_escape(&filtered)
}

fn jc(a: Align) -> &'static str {
    match a {
        Align::Left => "",
        Align::Center => "<w:jc w:val=\"center\"/>",
        Align::Right => "<w:jc w:val=\"right\"/>",
    }
}

impl<'a> Docx<'a> {
    fn new_rel(&mut self) -> String {
        self.next_rel += 1;
        format!("rId{}", self.next_rel)
    }

    fn bookmark(&mut self, key: &str) -> (u32, String) {
        let n = self.bookmarks.len() as u32 + 1;
        self.bookmarks
            .entry(key.to_string())
            .or_insert_with(|| (n, format!("bx{n}")))
            .clone()
    }

    fn bookmark_xml(&mut self, key: &str) -> String {
        let (id, name) = self.bookmark(key);
        format!("<w:bookmarkStart w:id=\"{id}\" w:name=\"{name}\"/><w:bookmarkEnd w:id=\"{id}\"/>")
    }

    fn media_for(&mut self, path: &str) -> Option<usize> {
        if let Some(m) = self.media_by_path.get(path) {
            return *m;
        }
        let result = (|| {
            let data = self.book.archive.get(path)?;
            let (bytes, ext) = match kind_of(data) {
                Some(Kind::Png) => (data.to_vec(), "png"),
                Some(Kind::Jpeg) => (data.to_vec(), "jpeg"),
                Some(Kind::Gif) => (data.to_vec(), "gif"),
                None => (to_png(data)?.0, "png"),
            };
            let (width, height) = image_size(&bytes)?;
            Some((bytes, ext, width, height))
        })();
        let idx = result.map(|(data, ext, width, height)| {
            let n = self.media.len() + 1;
            let rel_id = self.new_rel();
            self.media.push(Media {
                name: format!("image{n}.{ext}"),
                data,
                rel_id,
                width,
                height,
            });
            self.media.len() - 1
        });
        self.media_by_path.insert(path.to_string(), idx);
        idx
    }

    fn drawing(&mut self, path: &str, alt: &str, max_width: u64) -> String {
        let Some(i) = self.media_for(path) else {
            return String::new();
        };
        let m = &self.media[i];
        let mut cx = m.width as u64 * EMU_PER_PX;
        let mut cy = m.height as u64 * EMU_PER_PX;
        if cx > max_width {
            cy = cy * max_width / cx;
            cx = max_width;
        }
        let max_h = (self.opts.page.twips().1 as u64).saturating_sub(2 * self.opts.margin() as u64)
            * EMU_PER_TWIP;
        if cy > max_h && max_h > 0 {
            cx = cx * max_h / cy;
            cy = max_h;
        }
        let (cx, cy) = (cx.max(1), cy.max(1));
        self.drawing_id += 1;
        let id = self.drawing_id;
        let rel = m.rel_id.clone();
        let name = m.name.clone();
        format!(
            "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{id}\" name=\"Picture {id}\" descr=\"{}\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"{name}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rel}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",
            clean(alt)
        )
    }

    fn run(&self, text: &str, fmt: &Fmt, hyperlink: bool) -> String {
        if text.is_empty() {
            return String::new();
        }
        let mut rpr = String::new();
        if hyperlink {
            rpr.push_str("<w:rStyle w:val=\"Hyperlink\"/>");
        }
        if fmt.mono {
            rpr.push_str(
                "<w:rFonts w:ascii=\"Courier New\" w:hAnsi=\"Courier New\" w:cs=\"Courier New\"/>",
            );
        }
        if fmt.bold {
            rpr.push_str("<w:b/>");
        }
        if fmt.italic {
            rpr.push_str("<w:i/>");
        }
        if fmt.strike {
            rpr.push_str("<w:strike/>");
        }
        if fmt.small {
            rpr.push_str("<w:smallCaps/>");
        }
        if fmt.underline && !hyperlink {
            rpr.push_str("<w:u w:val=\"single\"/>");
        }
        if fmt.sup {
            rpr.push_str("<w:vertAlign w:val=\"superscript\"/>");
        } else if fmt.sub {
            rpr.push_str("<w:vertAlign w:val=\"subscript\"/>");
        }
        let rpr = if rpr.is_empty() {
            String::new()
        } else {
            format!("<w:rPr>{rpr}</w:rPr>")
        };
        let mut out = String::new();
        let mut first = true;
        for line in text.split('\n') {
            if !first {
                let _ = write!(out, "<w:r>{rpr}<w:br/></w:r>");
            }
            first = false;
            let mut tab_first = true;
            for piece in line.split('\t') {
                if !tab_first {
                    let _ = write!(out, "<w:r>{rpr}<w:tab/></w:r>");
                }
                tab_first = false;
                if !piece.is_empty() {
                    let _ = write!(
                        out,
                        "<w:r>{rpr}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
                        clean(piece)
                    );
                }
            }
        }
        out
    }

    fn inlines(&mut self, inl: &[Inline], max_width: u64) -> String {
        let mut out = String::new();
        let mut i = 0;
        while i < inl.len() {
            match &inl[i] {
                Inline::Text { link, .. } if link.is_some() => {
                    let mut j = i;
                    while j < inl.len() {
                        match &inl[j] {
                            Inline::Text { link: l, .. } if l == link => j += 1,
                            _ => break,
                        }
                    }
                    let attr = match link {
                        Some(Link::External(u)) => {
                            let id = self.new_rel();
                            self.links.push((id.clone(), u.clone()));
                            Some(format!("r:id=\"{id}\""))
                        }
                        Some(Link::Internal(k)) if self.model.linked.contains(k) => {
                            let (_, name) = self.bookmark(k);
                            Some(format!("w:anchor=\"{name}\""))
                        }
                        _ => None,
                    };
                    let mut runs = String::new();
                    for r in &inl[i..j] {
                        if let Inline::Text { text, fmt, .. } = r {
                            runs.push_str(&self.run(text, fmt, attr.is_some()));
                        }
                    }
                    match attr {
                        Some(a) => {
                            let _ = write!(
                                out,
                                "<w:hyperlink {a} w:history=\"1\">{runs}</w:hyperlink>"
                            );
                        }
                        None => out.push_str(&runs),
                    }
                    i = j;
                    continue;
                }
                Inline::Text { text, fmt, .. } => out.push_str(&self.run(text, fmt, false)),
                Inline::Image { path, alt } => out.push_str(&self.drawing(path, alt, max_width)),
                Inline::Break => out.push_str("<w:r><w:br/></w:r>"),
                Inline::Anchor(k) => {
                    if self.model.linked.contains(k) {
                        out.push_str(&self.bookmark_xml(k));
                    }
                }
            }
            i += 1;
        }
        out
    }

    /// Opens a paragraph with the given properties (already in schema order, without the
    /// `pStyle`, `pageBreakBefore` and bookmarks which are added here).
    fn paragraph(&mut self, style: &str, before_num: &str, after_num: &str, content: &str) {
        let mut ppr = format!("<w:pStyle w:val=\"{style}\"/>");
        ppr.push_str(before_num);
        if self.page_break {
            ppr.push_str("<w:pageBreakBefore/>");
            self.page_break = false;
        }
        ppr.push_str(after_num);
        let mut marks = String::new();
        for k in std::mem::take(&mut self.pending_bookmarks) {
            marks.push_str(&self.bookmark_xml(&k));
        }
        let _ = write!(self.body, "<w:p><w:pPr>{ppr}</w:pPr>{marks}{content}</w:p>");
    }

    fn blocks(&mut self, blocks: &[Block], indent: u32, depth_list: Option<u32>, max_width: u64) {
        for b in blocks {
            match b {
                Block::Heading {
                    level,
                    align,
                    inlines,
                } => {
                    let content = self.inlines(inlines, max_width);
                    let ind = if indent > 0 {
                        format!("<w:ind w:left=\"{indent}\"/>")
                    } else {
                        String::new()
                    };
                    self.paragraph(
                        &format!("Heading{}", (*level).clamp(1, 6)),
                        "",
                        &format!("{ind}{}", jc(*align)),
                        &content,
                    );
                }
                Block::Para { align, inlines } => {
                    let content = self.inlines(inlines, max_width);
                    let ind = if indent > 0 {
                        format!("<w:ind w:left=\"{indent}\"/>")
                    } else {
                        String::new()
                    };
                    let style = if indent > 0 { "Quote" } else { "Normal" };
                    self.paragraph(style, "", &format!("{ind}{}", jc(*align)), &content);
                }
                Block::Pre(inlines) => {
                    let content = self.inlines(&split_lines(inlines), max_width);
                    let ind = if indent > 0 {
                        format!("<w:ind w:left=\"{indent}\"/>")
                    } else {
                        String::new()
                    };
                    self.paragraph("Code", "", &ind, &content);
                }
                Block::Quote(inner) => self.blocks(inner, indent + 720, depth_list, max_width),
                Block::Rule => {
                    self.paragraph(
                        "Normal",
                        "",
                        "<w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"808080\"/></w:pBdr>",
                        "",
                    );
                }
                Block::List { ordered, items } => {
                    let depth = depth_list.map(|d| d + 1).unwrap_or(0).min(8);
                    self.list(*ordered, items, depth, max_width);
                }
                Block::Table { rows } => self.table(rows, max_width),
            }
        }
    }

    fn list(&mut self, ordered: bool, items: &[Vec<Block>], depth: u32, max_width: u64) {
        let num_id = if ordered {
            let id = self.nums.len() as u32 + 2;
            self.nums.push((id, true));
            id
        } else {
            1
        };
        for item in items {
            let mut first = true;
            for b in item {
                match b {
                    Block::Para { align, inlines } if first => {
                        let content = self.inlines(inlines, max_width);
                        self.paragraph(
                            "ListParagraph",
                            "",
                            &format!(
                                "<w:numPr><w:ilvl w:val=\"{depth}\"/><w:numId w:val=\"{num_id}\"/></w:numPr>{}",
                                jc(*align)
                            ),
                            &content,
                        );
                    }
                    Block::List { ordered, items } => {
                        self.list(*ordered, items, (depth + 1).min(8), max_width)
                    }
                    other => {
                        if first {
                            self.paragraph(
                                "ListParagraph",
                                "",
                                &format!(
                                    "<w:numPr><w:ilvl w:val=\"{depth}\"/><w:numId w:val=\"{num_id}\"/></w:numPr>"
                                ),
                                "",
                            );
                        }
                        self.blocks(
                            std::slice::from_ref(other),
                            720 * (depth + 1),
                            Some(depth),
                            max_width,
                        );
                    }
                }
                first = false;
            }
        }
    }

    fn table(&mut self, rows: &[Vec<Cell>], max_width: u64) {
        let cols = rows
            .iter()
            .map(|r| r.iter().map(|c| c.colspan).sum::<usize>())
            .max()
            .unwrap_or(1)
            .max(1);
        let (pw, _) = self.opts.page.twips();
        let text_w = pw.saturating_sub(2 * self.opts.margin()).max(1440);
        let col_w = (text_w / cols as u32).max(360);
        if self.page_break || !self.pending_bookmarks.is_empty() {
            self.paragraph("Normal", "", "", "");
        }
        self.body.push_str("<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblLook w:val=\"04A0\"/></w:tblPr><w:tblGrid>");
        for _ in 0..cols {
            let _ = write!(self.body, "<w:gridCol w:w=\"{col_w}\"/>");
        }
        self.body.push_str("</w:tblGrid>");
        for (ri, row) in rows.iter().enumerate() {
            self.body.push_str("<w:tr>");
            if ri == 0 && row.iter().all(|c| c.header) {
                self.body.push_str("<w:trPr><w:tblHeader/></w:trPr>");
            }
            let mut used = 0;
            for c in row {
                used += c.colspan;
                let span = if c.colspan > 1 {
                    format!("<w:gridSpan w:val=\"{}\"/>", c.colspan)
                } else {
                    String::new()
                };
                let _ = write!(
                    self.body,
                    "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/>{span}</w:tcPr>",
                    col_w * c.colspan as u32
                );
                let before = self.body.len();
                let inl = flat_inlines(&c.blocks);
                let nested: Vec<&Block> = c
                    .blocks
                    .iter()
                    .filter(|b| matches!(b, Block::Table { .. } | Block::List { .. }))
                    .collect();
                if nested.is_empty() {
                    let content =
                        self.inlines(&inl, (col_w as u64).saturating_sub(216) * EMU_PER_TWIP);
                    let _ = write!(
                        self.body,
                        "<w:p><w:pPr><w:pStyle w:val=\"Normal\"/></w:pPr>{content}</w:p>"
                    );
                } else {
                    self.blocks(&c.blocks, 0, None, max_width / cols as u64);
                    if !self.body.ends_with("</w:p>") {
                        self.body.push_str("<w:p/>");
                    }
                }
                debug_assert!(self.body.len() > before);
                self.body.push_str("</w:tc>");
            }
            while used < cols {
                used += 1;
                let _ = write!(
                    self.body,
                    "<w:tc><w:tcPr><w:tcW w:w=\"{col_w}\" w:type=\"dxa\"/></w:tcPr><w:p/></w:tc>"
                );
            }
            self.body.push_str("</w:tr>");
        }
        self.body.push_str("</w:tbl><w:p/>");
    }

    fn toc(&mut self) {
        if self.model.toc.is_empty() {
            return;
        }
        self.paragraph("TOCHeading", "", "", "<w:r><w:t>Contents</w:t></w:r>");
        let entries = self.model.toc.clone();
        let n = entries.len();
        for (i, e) in entries.iter().enumerate() {
            let (_, name) = self.bookmark(&e.key);
            let mut content = String::new();
            if i == 0 {
                content.push_str("<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> TOC \\o \"1-3\" \\h \\z \\u </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>");
            }
            let _ = write!(
                content,
                "<w:hyperlink w:anchor=\"{name}\" w:history=\"1\">{}</w:hyperlink>",
                self.run(&e.title, &Fmt::default(), true)
            );
            if i + 1 == n {
                content.push_str("<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>");
            }
            self.paragraph(&format!("TOC{}", (e.depth + 1).min(3)), "", "", &content);
        }
        self.page_break = true;
    }
}

fn content_types(media_exts: &[&str]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>",
    );
    for e in media_exts {
        let ct = match *e {
            "png" => "image/png",
            "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            _ => continue,
        };
        let _ = write!(s, "<Default Extension=\"{e}\" ContentType=\"{ct}\"/>");
    }
    for (part, ct) in [
        (
            "/word/document.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
        ),
        (
            "/word/styles.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml",
        ),
        (
            "/word/numbering.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
        ),
        (
            "/word/settings.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
        ),
        (
            "/docProps/core.xml",
            "application/vnd.openxmlformats-package.core-properties+xml",
        ),
        (
            "/docProps/app.xml",
            "application/vnd.openxmlformats-officedocument.extended-properties+xml",
        ),
    ] {
        let _ = write!(s, "<Override PartName=\"{part}\" ContentType=\"{ct}\"/>");
    }
    s.push_str("</Types>");
    s
}

fn root_rels() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"{REL}/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>"
    )
}

fn core_xml(book: &OpenBook) -> String {
    let md = book.metadata();
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">",
    );
    let _ = write!(s, "<dc:title>{}</dc:title>", clean(&md.title));
    if !md.authors.is_empty() {
        let _ = write!(
            s,
            "<dc:creator>{}</dc:creator>",
            clean(&md.authors.join("; "))
        );
    }
    if !md.tags.is_empty() {
        let _ = write!(
            s,
            "<cp:keywords>{}</cp:keywords>",
            clean(&md.tags.join(", "))
        );
    }
    if !md.description.is_empty() {
        let _ = write!(
            s,
            "<dc:description>{}</dc:description>",
            clean(&crate::markup::plain_text(&md.description))
        );
    }
    if !md.language.is_empty() {
        let _ = write!(s, "<dc:language>{}</dc:language>", clean(&md.language));
    }
    let _ = write!(
        s,
        "<dcterms:created xsi:type=\"dcterms:W3CDTF\">{now}</dcterms:created><dcterms:modified xsi:type=\"dcterms:W3CDTF\">{now}</dcterms:modified></cp:coreProperties>"
    );
    s
}

fn app_xml(book: &OpenBook) -> String {
    let md = book.metadata();
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Bookx</Application>",
    );
    if !md.publisher.is_empty() {
        let _ = write!(s, "<Company>{}</Company>", clean(&md.publisher));
    }
    s.push_str("</Properties>");
    s
}

fn styles_xml(o: &DocxOptions) -> String {
    let font = clean(&o.font);
    let sz = o.half_points();
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"{NS_W}\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:eastAsia=\"{font}\" w:cs=\"{font}\"/><w:sz w:val=\"{sz}\"/><w:szCs w:val=\"{sz}\"/><w:lang w:val=\"en-US\" w:eastAsia=\"en-US\" w:bidi=\"ar-SA\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"120\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>"
    );
    s.push_str("<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>");
    s.push_str("<w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\"><w:name w:val=\"Default Paragraph Font\"/><w:uiPriority w:val=\"1\"/><w:semiHidden/></w:style>");
    s.push_str("<w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\"><w:name w:val=\"Normal Table\"/><w:uiPriority w:val=\"99\"/><w:semiHidden/><w:tblPr><w:tblInd w:w=\"0\" w:type=\"dxa\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"108\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>");
    s.push_str("<w:style w:type=\"numbering\" w:default=\"1\" w:styleId=\"NoList\"><w:name w:val=\"No List\"/><w:uiPriority w:val=\"99\"/><w:semiHidden/></w:style>");
    let sizes = [36u32, 32, 28, 26, 24, 22];
    for (i, size) in sizes.iter().enumerate() {
        let l = i + 1;
        let _ = write!(
            s,
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{l}\"><w:name w:val=\"heading {l}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before=\"{}\" w:after=\"120\"/><w:outlineLvl w:val=\"{i}\"/></w:pPr><w:rPr><w:b/><w:bCs/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr></w:style>",
            if l == 1 { 360 } else { 240 }
        );
    }
    s.push_str("<w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"48\"/></w:rPr></w:style>");
    s.push_str("<w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:rPr><w:i/><w:iCs/></w:rPr></w:style>");
    s.push_str("<w:style w:type=\"paragraph\" w:styleId=\"Code\"><w:name w:val=\"Code\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:after=\"120\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Courier New\" w:hAnsi=\"Courier New\" w:cs=\"Courier New\"/><w:sz w:val=\"20\"/></w:rPr></w:style>");
    s.push_str("<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:uiPriority w:val=\"34\"/><w:qFormat/><w:pPr><w:spacing w:after=\"60\"/><w:ind w:left=\"720\"/></w:pPr></w:style>");
    s.push_str("<w:style w:type=\"paragraph\" w:styleId=\"TOCHeading\"><w:name w:val=\"TOC Heading\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"39\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"240\" w:after=\"120\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"32\"/></w:rPr></w:style>");
    for l in 1..=3u32 {
        let _ = write!(
            s,
            "<w:style w:type=\"paragraph\" w:styleId=\"TOC{l}\"><w:name w:val=\"toc {l}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"39\"/><w:pPr><w:spacing w:after=\"60\"/><w:ind w:left=\"{}\"/></w:pPr></w:style>",
            (l - 1) * 240
        );
    }
    s.push_str("<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:basedOn w:val=\"DefaultParagraphFont\"/><w:uiPriority w:val=\"99\"/><w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr></w:style>");
    s.push_str("<w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:basedOn w:val=\"TableNormal\"/><w:uiPriority w:val=\"59\"/><w:tblPr><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/></w:tblBorders></w:tblPr></w:style>");
    s.push_str("</w:styles>");
    s
}

fn numbering_xml(nums: &[(u32, bool)]) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:numbering xmlns:w=\"{NS_W}\">"
    );
    // abstractNum 0: bullets, abstractNum 1: decimal / letters / roman by level
    for (abs, ordered) in [(0u32, false), (1u32, true)] {
        let _ = write!(
            s,
            "<w:abstractNum w:abstractNumId=\"{abs}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>"
        );
        for lvl in 0..9u32 {
            let (fmt, text, font) = if ordered {
                let f = ["decimal", "lowerLetter", "lowerRoman"][(lvl % 3) as usize];
                (f, format!("%{}.", lvl + 1), "")
            } else {
                let b = ["\u{2022}", "o", "\u{25aa}"][(lvl % 3) as usize];
                let font = if lvl % 3 == 1 {
                    "<w:rFonts w:ascii=\"Courier New\" w:hAnsi=\"Courier New\" w:cs=\"Courier New\"/>"
                } else {
                    ""
                };
                ("bullet", b.to_string(), font)
            };
            let _ = write!(
                s,
                "<w:lvl w:ilvl=\"{lvl}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr>{}</w:lvl>",
                720 * (lvl + 1),
                if font.is_empty() {
                    String::new()
                } else {
                    format!("<w:rPr>{font}</w:rPr>")
                }
            );
        }
        s.push_str("</w:abstractNum>");
    }
    s.push_str("<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>");
    for (id, _) in nums {
        let _ = write!(
            s,
            "<w:num w:numId=\"{id}\"><w:abstractNumId w:val=\"1\"/>{}</w:num>",
            (0..9)
                .map(|l| format!(
                    "<w:lvlOverride w:ilvl=\"{l}\"><w:startOverride w:val=\"1\"/></w:lvlOverride>"
                ))
                .collect::<String>()
        );
    }
    s.push_str("</w:numbering>");
    s
}

impl FormatWriter for DocxWriter {
    fn id(&self) -> &'static str {
        "docx"
    }
    fn extension(&self) -> &'static str {
        "docx"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let model = Model::build(book);
        let o = &self.options;
        let (pw, ph) = o.page.twips();
        let margin = o.margin();
        let text_width_emu = (pw.saturating_sub(2 * margin).max(1440)) as u64 * EMU_PER_TWIP;
        let mut d = Docx {
            book,
            model: &model,
            opts: o,
            body: String::new(),
            links: Vec::new(),
            media: Vec::new(),
            media_by_path: HashMap::new(),
            bookmarks: HashMap::new(),
            pending_bookmarks: Vec::new(),
            nums: Vec::new(),
            next_rel: 3,
            drawing_id: 0,
            page_break: false,
            text_width_emu,
        };
        if let Some(c) = &model.cover {
            if d.media_for(&c.path).is_some() {
                let content = d.drawing(&c.path, "Cover", text_width_emu);
                d.paragraph("Normal", "", "<w:jc w:val=\"center\"/>", &content);
                d.page_break = true;
            }
        }
        if o.toc {
            d.toc();
        }
        let width = d.text_width_emu;
        for s in &model.sections {
            if model.linked.contains(&s.path) {
                d.pending_bookmarks.push(s.path.clone());
            }
            d.blocks(&s.blocks, 0, None, width);
            d.page_break = true;
        }
        if d.body.is_empty() {
            d.paragraph("Normal", "", "", "");
        }

        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"{NS_W}\" xmlns:r=\"{NS_R}\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{}<w:sectPr><w:pgSz w:w=\"{pw}\" w:h=\"{ph}\"/><w:pgMar w:top=\"{margin}\" w:right=\"{margin}\" w:bottom=\"{margin}\" w:left=\"{margin}\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>",
            d.body
        );

        let mut rels = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{REL}/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId2\" Type=\"{REL}/numbering\" Target=\"numbering.xml\"/><Relationship Id=\"rId3\" Type=\"{REL}/settings\" Target=\"settings.xml\"/>"
        );
        for m in &d.media {
            let _ = write!(
                rels,
                "<Relationship Id=\"{}\" Type=\"{REL}/image\" Target=\"media/{}\"/>",
                m.rel_id, m.name
            );
        }
        for (id, target) in &d.links {
            let _ = write!(
                rels,
                "<Relationship Id=\"{id}\" Type=\"{REL}/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
                clean(target)
            );
        }
        rels.push_str("</Relationships>");

        let mut exts: Vec<&str> = Vec::new();
        for m in &d.media {
            let e = m.name.rsplit('.').next().unwrap_or("png");
            if !exts.contains(&e) {
                exts.push(e);
            }
        }
        let settings = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:settings xmlns:w=\"{NS_W}\"><w:defaultTabStop w:val=\"720\"/><w:characterSpacingControl w:val=\"doNotCompress\"/></w:settings>"
        );

        let mut files: Vec<(String, Vec<u8>)> = vec![
            (
                "[Content_Types].xml".into(),
                content_types(&exts).into_bytes(),
            ),
            ("_rels/.rels".into(), root_rels().into_bytes()),
            ("docProps/core.xml".into(), core_xml(book).into_bytes()),
            ("docProps/app.xml".into(), app_xml(book).into_bytes()),
            ("word/document.xml".into(), document.into_bytes()),
            ("word/styles.xml".into(), styles_xml(o).into_bytes()),
            (
                "word/numbering.xml".into(),
                numbering_xml(&d.nums).into_bytes(),
            ),
            ("word/settings.xml".into(), settings.into_bytes()),
            ("word/_rels/document.xml.rels".into(), rels.into_bytes()),
        ];
        for m in d.media {
            files.push((format!("word/media/{}", m.name), m.data));
        }
        zip_files(&files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_sizes() {
        assert_eq!(PageSize::Letter.twips(), (12240, 15840));
        assert_eq!(PageSize::A4.twips(), (11906, 16838));
        assert_eq!(PageSize::A5.twips(), (8392, 11906));
        assert_eq!(PageSize::Custom(100, 99999).twips(), (2880, 31680));
    }

    #[test]
    fn option_helpers() {
        let o = DocxOptions {
            font_size_pt: 10.5,
            margin_twips: 99999,
            ..Default::default()
        };
        assert_eq!(o.half_points(), 21);
        assert_eq!(o.margin(), 12240 / 3);
        let tiny = DocxOptions {
            font_size_pt: 0.0,
            ..Default::default()
        };
        assert_eq!(tiny.half_points(), 8);
    }

    #[test]
    fn clean_drops_control_characters_and_escapes() {
        assert_eq!(clean("a\u{1}b<&>"), "ab&lt;&amp;&gt;");
        assert_eq!(clean("tab\there"), "tab\there");
    }

    #[test]
    fn styles_are_well_formed_for_odd_fonts() {
        let o = DocxOptions {
            font: "A&B \"Font\"".into(),
            ..Default::default()
        };
        assert!(roxmltree::Document::parse(&styles_xml(&o)).is_ok());
    }

    #[test]
    fn numbering_has_one_instance_per_ordered_list() {
        let x = numbering_xml(&[(2, true), (3, true)]);
        let doc = roxmltree::Document::parse(&x).unwrap();
        let nums = doc
            .descendants()
            .filter(|n| n.tag_name().name() == "num")
            .count();
        assert_eq!(nums, 3);
    }

    #[test]
    fn content_types_list_only_known_image_types() {
        let ct = content_types(&["png", "weird"]);
        assert!(ct.contains("Extension=\"png\""));
        assert!(!ct.contains("weird"));
        assert!(roxmltree::Document::parse(&ct).is_ok());
    }

    use crate::testutil::book_with;

    #[test]
    fn writer_identity_and_a_minimal_package() {
        let w = DocxWriter::default();
        assert_eq!((w.id(), w.extension()), ("docx", "docx"));
        assert_eq!(w.options, DocxOptions::default());
        let bytes = w.write(&book_with(&["<h1>H</h1><p>p</p>"])).unwrap();
        let z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut names: Vec<_> = z
            .file_names()
            .map(|n| n.expect("name").into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "_rels/.rels",
                "docProps/app.xml",
                "docProps/core.xml",
                "word/_rels/document.xml.rels",
                "word/document.xml",
                "word/numbering.xml",
                "word/settings.xml",
                "word/styles.xml"
            ]
        );
    }

    #[test]
    fn jc_values() {
        assert_eq!(jc(Align::Left), "");
        assert!(jc(Align::Center).contains("center"));
        assert!(jc(Align::Right).contains("right"));
    }
}
