//! Microsoft Word `.docx` reader.
//!
//! Supported: paragraphs, headings (`Heading N` / `Title` styles or outline levels), bold,
//! italic, underline, strike, super/subscript, centred / right aligned paragraphs, bulleted and
//! numbered lists (flat), tables, hyperlinks (external and bookmarks), embedded raster images,
//! footnotes, and `docProps/core.xml` metadata. Ignored: headers and footers, text boxes,
//! comments, tracked deletions, EMF/WMF pictures, and list nesting.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::Metadata;
use bookx_core::util::{parse_xml, resolve_href, xml_escape};
use roxmltree::Node;

use crate::common::*;

const MAX_DEPTH: usize = 128;

/// Reader for `.docx` files.
pub struct DocxReader;

impl FormatReader for DocxReader {
    fn id(&self) -> &'static str {
        "docx"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["docx"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        zip_names(head).is_some_and(|n| {
            n.iter().any(|f| f == "word/document.xml")
                || (n.len() == 1 && n[0].starts_with("word/"))
        })
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_docx(bytes)
    }
}

struct Rels {
    map: HashMap<String, (String, bool)>,
}

fn parse_rels(zip: &ZipFiles, name: &str) -> Rels {
    let mut map = HashMap::new();
    if let Some(data) = zip.get(name) {
        if let Ok(doc) = parse_xml(&decode_xml(data)) {
            for r in doc
                .descendants()
                .filter(|n| n.tag_name().name() == "Relationship")
            {
                if let (Some(id), Some(t)) = (r.attribute("Id"), r.attribute("Target")) {
                    let ext = r.attribute("TargetMode") == Some("External");
                    map.insert(id.to_string(), (t.to_string(), ext));
                }
            }
        }
    }
    Rels { map }
}

#[derive(Default)]
struct Styles {
    /// styleId -> lower-case style name
    names: HashMap<String, String>,
}

impl Styles {
    fn heading_level(&self, style_id: &str) -> Option<usize> {
        let name = self
            .names
            .get(style_id)
            .cloned()
            .unwrap_or_else(|| style_id.to_ascii_lowercase());
        if name == "title" {
            return Some(1);
        }
        let rest = name
            .strip_prefix("heading")
            .map(|r| r.trim())
            .and_then(|r| r.parse::<usize>().ok())?;
        (1..=9).contains(&rest).then_some(rest.min(6))
    }
}

#[derive(Default)]
struct Numbering {
    /// numId -> (ordered)
    ordered: HashMap<String, bool>,
}

fn parse_numbering(zip: &ZipFiles) -> Numbering {
    let mut n = Numbering::default();
    let Some(data) = zip.get("word/numbering.xml") else {
        return n;
    };
    let text = decode_xml(data);
    let Ok(doc) = parse_xml(&text) else {
        return n;
    };
    let mut abstracts: HashMap<String, bool> = HashMap::new();
    for a in doc
        .descendants()
        .filter(|d| d.tag_name().name() == "abstractNum")
    {
        let id = attr(a, "abstractNumId").unwrap_or("").to_string();
        let fmt = a
            .descendants()
            .find(|d| d.tag_name().name() == "numFmt")
            .and_then(|d| attr(d, "val"))
            .unwrap_or("bullet");
        abstracts.insert(id, !matches!(fmt, "bullet" | "none"));
    }
    for num in doc.descendants().filter(|d| d.tag_name().name() == "num") {
        let id = attr(num, "numId").unwrap_or("").to_string();
        let abs = child_el(num, "abstractNumId")
            .and_then(|a| attr(a, "val"))
            .unwrap_or("");
        n.ordered
            .insert(id, abstracts.get(abs).copied().unwrap_or(false));
    }
    n
}

fn parse_styles(zip: &ZipFiles) -> Styles {
    let mut s = Styles::default();
    if let Some(data) = zip.get("word/styles.xml") {
        if let Ok(doc) = parse_xml(&decode_xml(data)) {
            for st in doc.descendants().filter(|d| d.tag_name().name() == "style") {
                let (Some(id), Some(name)) = (
                    attr(st, "styleId"),
                    child_el(st, "name").and_then(|n| attr(n, "val")),
                ) else {
                    continue;
                };
                s.names.insert(id.to_string(), name.to_ascii_lowercase());
            }
        }
    }
    s
}

struct Ctx<'z> {
    zip: &'z ZipFiles,
    rels: Rels,
    styles: Styles,
    numbering: Numbering,
    media: MediaSet,
    footnotes: HashMap<String, Vec<String>>,
    used_notes: Vec<String>,
}

fn is_on(n: Option<Node>) -> bool {
    match n {
        None => false,
        Some(n) => !matches!(attr(n, "val"), Some("0" | "false" | "off" | "none")),
    }
}

impl Ctx<'_> {
    fn image(&mut self, rid: &str) -> String {
        let Some((target, false)) = self.rels.map.get(rid).cloned() else {
            return String::new();
        };
        let (path, _) = resolve_href("word", &target);
        let Some(data) = self.zip.get(&path) else {
            return String::new();
        };
        match self.media.add(&path, data.to_vec()) {
            Some(p) => format!("<img src=\"{p}\" alt=\"\"/>"),
            None => String::new(),
        }
    }

    fn run(&mut self, r: Node, out: &mut String, depth: usize) {
        let rpr = child_el(r, "rPr");
        let has = |name: &str| rpr.and_then(|p| child_el(p, name));
        let (bold, italic, strike) = (
            is_on(has("b")),
            is_on(has("i")),
            is_on(has("strike")) || is_on(has("dstrike")),
        );
        let underline = is_on(has("u"));
        let vert = has("vertAlign").and_then(|v| attr(v, "val"));
        let mut inner = String::new();
        for c in r.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "t" | "delText" if c.tag_name().name() == "t" => {
                    inner.push_str(&xml_escape(c.text().unwrap_or("")))
                }
                "tab" => inner.push(' '),
                "br" | "cr" => {
                    if attr(c, "type") != Some("page") {
                        inner.push_str("<br/>")
                    }
                }
                "noBreakHyphen" => inner.push('\u{2011}'),
                // Word and Calibre put bookmarks inside runs as well
                "bookmarkStart" => {
                    if let Some(n) = attr(c, "name") {
                        inner.push_str(&format!("<span id=\"{}\"></span>", xml_escape(n)));
                    }
                }
                "sym" => {}
                "drawing" | "pict" | "object" => {
                    for b in c.descendants().filter(|d| d.is_element()) {
                        let rid = match b.tag_name().name() {
                            "blip" => attr(b, "embed"),
                            "imagedata" => attr(b, "id"),
                            _ => None,
                        };
                        if let Some(rid) = rid {
                            inner.push_str(&self.image(rid));
                        }
                    }
                }
                "footnoteReference" => {
                    if let Some(id) = attr(c, "id") {
                        if self.footnotes.contains_key(id) {
                            if !self.used_notes.iter().any(|u| u == id) {
                                self.used_notes.push(id.to_string());
                            }
                            let n = self.used_notes.iter().position(|u| u == id).unwrap_or(0) + 1;
                            inner.push_str(&noteref(&format!("fn{id}"), &format!("[{n}]")));
                        }
                    }
                }
                _ => {}
            }
        }
        if inner.is_empty() {
            return;
        }
        for (on, tag) in [
            (vert == Some("subscript"), "sub"),
            (vert == Some("superscript"), "sup"),
            (strike, "del"),
            (underline, "u"),
            (italic, "em"),
            (bold, "strong"),
        ] {
            if on && !inner.starts_with("<img") {
                inner = format!("<{tag}>{inner}</{tag}>");
            }
        }
        let _ = depth;
        out.push_str(&inner);
    }

    fn inline(&mut self, p: Node, out: &mut String, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for c in p.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "r" => self.run(c, out, depth),
                "hyperlink" => {
                    let mut inner = String::new();
                    self.inline(c, &mut inner, depth + 1);
                    let href = if let Some(a) = attr(c, "anchor") {
                        Some(format!("#{a}"))
                    } else {
                        attr(c, "id").and_then(|id| match self.rels.map.get(id) {
                            Some((t, true)) => Some(t.clone()),
                            _ => None,
                        })
                    };
                    match href {
                        Some(h) if !inner.is_empty() => {
                            out.push_str(&format!("<a href=\"{}\">{inner}</a>", xml_escape(&h)))
                        }
                        _ => out.push_str(&inner),
                    }
                }
                "bookmarkStart" => {
                    if let Some(n) = attr(c, "name") {
                        out.push_str(&format!("<span id=\"{}\"></span>", xml_escape(n)));
                    }
                }
                "ins" | "smartTag" | "sdt" | "sdtContent" | "fldSimple" | "customXml" => {
                    self.inline(c, out, depth + 1)
                }
                _ => {}
            }
        }
    }

    /// Returns `(tag, attrs, inner)` for a paragraph plus its list info.
    fn paragraph(&mut self, p: Node) -> Option<Para> {
        let ppr = child_el(p, "pPr");
        let style = ppr
            .and_then(|x| child_el(x, "pStyle"))
            .and_then(|s| attr(s, "val"))
            .unwrap_or("");
        let mut level = if style.is_empty() {
            None
        } else {
            self.styles.heading_level(style)
        };
        if let Some(o) = ppr
            .and_then(|x| child_el(x, "outlineLvl"))
            .and_then(|o| attr(o, "val"))
            .and_then(|v| v.parse::<usize>().ok())
        {
            if o < 9 {
                level = Some((o + 1).min(6));
            }
        }
        let num = ppr.and_then(|x| child_el(x, "numPr"));
        let list = num
            .and_then(|n| child_el(n, "numId"))
            .and_then(|n| attr(n, "val"))
            .filter(|v| *v != "0")
            .map(|id| self.numbering.ordered.get(id).copied().unwrap_or(false));
        let align = ppr
            .and_then(|x| child_el(x, "jc"))
            .and_then(|j| attr(j, "val"))
            .map(|a| match a {
                "center" => " style=\"text-align:center\"",
                "right" | "end" => " style=\"text-align:right\"",
                _ => "",
            })
            .unwrap_or("");
        let mut inner = String::new();
        self.inline(p, &mut inner, 0);
        let blank =
            visible_text_len(&inner) == 0 && !inner.contains("<img") && !inner.contains(" id=\"");
        if blank {
            return None;
        }
        Some(Para {
            level,
            list,
            align,
            inner,
        })
    }

    fn blocks(&mut self, parent: Node, out: &mut Vec<String>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut list: Option<(bool, String)> = None;
        let flush = |list: &mut Option<(bool, String)>, out: &mut Vec<String>| {
            if let Some((ordered, items)) = list.take() {
                let tag = if ordered { "ol" } else { "ul" };
                out.push(format!("<{tag}>\n{items}</{tag}>\n"));
            }
        };
        for c in parent.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "p" => {
                    if let Some(para) = self.paragraph(c) {
                        if let Some(ordered) = para.list {
                            if list.as_ref().is_some_and(|(o, _)| *o != ordered) {
                                flush(&mut list, out);
                            }
                            list.get_or_insert((ordered, String::new()))
                                .1
                                .push_str(&format!("<li>{}</li>\n", para.inner));
                            continue;
                        }
                        flush(&mut list, out);
                        out.push(match para.level {
                            Some(l) => format!("<h{l}{0}>{1}</h{l}>\n", para.align, para.inner),
                            None => format!("<p{}>{}</p>\n", para.align, para.inner),
                        });
                    }
                }
                "tbl" => {
                    flush(&mut list, out);
                    let mut t = String::from("<table>\n");
                    for tr in c
                        .children()
                        .filter(|r| r.is_element() && r.tag_name().name() == "tr")
                    {
                        t.push_str("<tr>");
                        for tc in tr
                            .children()
                            .filter(|r| r.is_element() && r.tag_name().name() == "tc")
                        {
                            let mut cell = Vec::new();
                            self.blocks(tc, &mut cell, depth + 1);
                            t.push_str(&format!("<td>{}</td>", cell.concat()));
                        }
                        t.push_str("</tr>\n");
                    }
                    t.push_str("</table>\n");
                    out.push(t);
                }
                "sdt" | "sdtContent" | "customXml" | "ins" => {
                    flush(&mut list, out);
                    self.blocks(c, out, depth + 1)
                }
                _ => {}
            }
        }
        flush(&mut list, out);
    }
}

struct Para {
    level: Option<usize>,
    list: Option<bool>,
    align: &'static str,
    inner: String,
}

fn read_core(zip: &ZipFiles) -> Metadata {
    let mut md = Metadata::default();
    let Some(data) = zip.get("docProps/core.xml") else {
        return md;
    };
    let text = decode_xml(data);
    let Ok(doc) = parse_xml(&text) else {
        return md;
    };
    for n in doc.root_element().children().filter(|c| c.is_element()) {
        let t = node_text(n).trim().to_string();
        match n.tag_name().name() {
            "title" => md.title = t,
            "creator" if !t.is_empty() => md.authors.push(t),
            "language" => md.language = t,
            "description" => md.description = t,
            "subject" if md.description.is_empty() => md.description = t,
            "keywords" => md.tags = split_keywords(&t),
            _ => {}
        }
    }
    md
}

/// Converts DOCX bytes to an EPUB archive.
pub fn convert_docx(bytes: &[u8]) -> Result<Archive> {
    let zip = read_zip(bytes)?;
    let Some(doc_bytes) = zip.get("word/document.xml") else {
        bail!("not a Word document: word/document.xml is missing");
    };
    let text = decode_xml(doc_bytes);
    let doc = parse_xml(&text).context("invalid word/document.xml")?;
    let Some(body) = child_el(doc.root_element(), "body") else {
        bail!("the document has no body");
    };

    let mut ctx = Ctx {
        zip: &zip,
        rels: parse_rels(&zip, "word/_rels/document.xml.rels"),
        styles: parse_styles(&zip),
        numbering: parse_numbering(&zip),
        media: MediaSet::default(),
        footnotes: HashMap::new(),
        used_notes: Vec::new(),
    };

    // Footnotes are parsed with the same machinery (their images use the same relationships
    // only rarely, which is why the main relationships are reused).
    let notes_text = zip.get("word/footnotes.xml").map(decode_xml);
    let notes_doc = notes_text.as_deref().and_then(|t| parse_xml(t).ok());
    if let Some(nd) = &notes_doc {
        for f in nd
            .descendants()
            .filter(|n| n.tag_name().name() == "footnote")
        {
            let (Some(id), None) = (attr(f, "id"), attr(f, "type")) else {
                continue;
            };
            let mut blocks = Vec::new();
            ctx.blocks(f, &mut blocks, 0);
            ctx.footnotes.insert(id.to_string(), blocks);
        }
    }

    let mut blocks = Vec::new();
    ctx.blocks(body, &mut blocks, 0);
    if blocks.is_empty() {
        bail!("the document has no text");
    }

    let mut md = read_core(&zip);
    if md.title.trim().is_empty() {
        md.title = String::new();
    }
    let default_title = if md.title.is_empty() {
        "Beginning".to_string()
    } else {
        md.title.clone()
    };
    let mut chapters = chapters_from_blocks(blocks, &default_title);
    if !ctx.used_notes.is_empty() {
        let mut body = String::from("<h1>Notes</h1>\n");
        for id in &ctx.used_notes {
            if let Some(b) = ctx.footnotes.get(id) {
                body.push_str(&footnote_aside(&format!("fn{id}"), &b.concat()));
            }
        }
        chapters.push(Chapter {
            title: "Notes".into(),
            body,
        });
    }
    let used: Vec<Media> = ctx
        .media
        .items
        .iter()
        .filter(|m| chapters.iter().any(|c| c.body.contains(&m.path)))
        .cloned()
        .collect();
    build(Parts {
        md,
        chapters,
        media: used,
        ..Default::default()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::*;

    const W: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\"";

    pub fn sample() -> Vec<u8> {
        let document = format!(
            r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document {W}><w:body>
<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>The Title</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:bookmarkStart w:id="0" w:name="intro"/><w:r><w:t>Intro</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Plain </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r><w:r><w:t xml:space="preserve"> </w:t></w:r><w:r><w:rPr><w:i/><w:u w:val="single"/></w:rPr><w:t>itu</w:t></w:r>
<w:r><w:rPr><w:b w:val="0"/></w:rPr><w:t xml:space="preserve"> notbold </w:t></w:r><w:r><w:rPr><w:strike/></w:rPr><w:t>gone</w:t></w:r><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:t>2</w:t></w:r><w:r><w:rPr><w:vertAlign w:val="subscript"/></w:rPr><w:t>i</w:t></w:r>
<w:r><w:t>x</w:t><w:br/><w:t>after break</w:t><w:footnoteReference w:id="1"/></w:r></w:p>
<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>Centered</w:t></w:r></w:p>
<w:p><w:hyperlink r:id="rId5"><w:r><w:t>web</w:t></w:r></w:hyperlink> and <w:hyperlink w:anchor="intro"><w:r><w:t>jump</w:t></w:r></w:hyperlink></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>bullet one</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>bullet two</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>number one</w:t></w:r></w:p>
<w:p><w:r><w:drawing><wp:inline><a:graphic><a:graphicData><a:blip r:embed="rId6"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>cell A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>cell B</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Second</w:t></w:r></w:p>
<w:p><w:pPr><w:outlineLvl w:val="1"/></w:pPr><w:r><w:t>Outlined</w:t></w:r></w:p>
<w:p><w:r><w:t>Final &amp; text</w:t></w:r></w:p>
<w:p/>
<w:sectPr/></w:body></w:document>"##
        );
        let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId5" Type="x" Target="http://example.com/?a=1&amp;b=2" TargetMode="External"/>
<Relationship Id="rId6" Type="x" Target="media/image1.png"/></Relationships>"#;
        let styles = format!(
            r#"<?xml version="1.0"?><w:styles {W}><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/></w:style></w:styles>"#
        );
        let numbering = format!(
            r#"<?xml version="1.0"?><w:numbering {W}>
<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum>
<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
        );
        let footnotes = format!(
            r#"<?xml version="1.0"?><w:footnotes {W}><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>
<w:footnote w:id="1"><w:p><w:r><w:t>The footnote text.</w:t></w:r></w:p></w:footnote></w:footnotes>"#
        );
        let core = r#"<?xml version="1.0"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:title>Doc Title</dc:title><dc:creator>Dora Writer</dc:creator><dc:language>en-GB</dc:language><dc:description>About it</dc:description><cp:keywords>one, two; three</cp:keywords></cp:coreProperties>"#;
        make_zip(&[
            ("[Content_Types].xml", b"<Types/>"),
            ("word/document.xml", document.as_bytes()),
            ("word/_rels/document.xml.rels", rels.as_bytes()),
            ("word/styles.xml", styles.as_bytes()),
            ("word/numbering.xml", numbering.as_bytes()),
            ("word/footnotes.xml", footnotes.as_bytes()),
            ("word/media/image1.png", PNG),
            ("docProps/core.xml", core.as_bytes()),
        ])
    }

    #[test]
    fn metadata_from_core_properties() {
        let book = validate(&convert_docx(&sample()).unwrap());
        let md = book.metadata();
        assert_eq!(md.title, "Doc Title");
        assert_eq!(md.authors, vec!["Dora Writer"]);
        assert_eq!(md.language, "en-GB");
        assert_eq!(md.description, "About it");
        assert_eq!(md.tags, vec!["one", "two", "three"]);
    }

    #[test]
    fn headings_make_chapters() {
        let book = validate(&convert_docx(&sample()).unwrap());
        assert_eq!(
            toc_titles(&book),
            vec!["The Title", "Intro", "Second", "Outlined", "Notes"]
        );
        assert!(chapter_xml(&book, 0).contains("<h1>The Title</h1>"));
        assert!(chapter_xml(&book, 2).contains(">Outlined</h2>"));
        assert!(chapter_texts(&book)[2].contains("Final & text"));
    }

    #[test]
    fn runs_lists_tables_and_alignment() {
        let book = validate(&convert_docx(&sample()).unwrap());
        let x = chapter_xml(&book, 1);
        assert!(
            x.contains("Plain <strong>bold</strong> <em><u>itu</u></em>"),
            "{x}"
        );
        assert!(x.contains(" notbold "), "{x}");
        assert!(!x.contains("<strong> notbold"));
        assert!(x.contains("<del>gone</del><sup>2</sup><sub>i</sub>"));
        assert!(x.contains("x<br/>after break"));
        assert!(x.contains("<p style=\"text-align:center\">Centered</p>"));
        assert!(
            x.contains("<ul>\n<li>bullet one</li>\n<li>bullet two</li>\n</ul>"),
            "{x}"
        );
        assert!(x.contains("<ol>\n<li>number one</li>\n</ol>"));
        assert!(
            x.contains("<td><p>cell A</p>\n</td><td><p>cell B</p>\n</td>"),
            "{x}"
        );
    }

    #[test]
    fn links_images_and_footnotes() {
        let arc = convert_docx(&sample()).unwrap();
        let book = validate(&arc);
        let x = chapter_xml(&book, 1);
        assert!(
            x.contains("<a href=\"http://example.com/?a=1&amp;b=2\">web</a>"),
            "{x}"
        );
        assert!(x.contains("<a href=\"#intro\">jump</a>"));
        assert!(x.contains("<img src=\"images/image1.png\" alt=\"\"/>"));
        assert_eq!(arc.get("OEBPS/images/image1.png"), Some(PNG));
        assert!(x.contains("epub:type=\"noteref\""));
        assert!(x.contains("href=\"chapter-4.xhtml#fn1\""));
        let notes = chapter_xml(&book, 3);
        assert!(notes.contains("id=\"fn1\" epub:type=\"footnote\""));
        assert!(notes.contains("The footnote text."));
        assert!(!notes.contains("separator"));
    }

    #[test]
    fn minimal_document_without_optional_parts() {
        let doc = format!(
            "<w:document {W}><w:body><w:p><w:r><w:t>Hi</w:t></w:r></w:p></w:body></w:document>"
        );
        let z = make_zip(&[("word/document.xml", doc.as_bytes())]);
        let book = validate(&convert_docx(&z).unwrap());
        assert_eq!(book.metadata().title, "Untitled");
        assert!(chapter_texts(&book)[0].contains("Hi"));
    }

    #[test]
    fn errors() {
        assert!(convert_docx(b"junk").is_err());
        let z = make_zip(&[("a.txt", b"x")]);
        assert!(convert_docx(&z)
            .unwrap_err()
            .to_string()
            .contains("word/document.xml"));
        let z = make_zip(&[("word/document.xml", b"<w:document")]);
        assert!(convert_docx(&z).is_err());
        let doc = format!("<w:document {W}><w:body><w:p/></w:body></w:document>");
        let z = make_zip(&[("word/document.xml", doc.as_bytes())]);
        assert!(convert_docx(&z)
            .unwrap_err()
            .to_string()
            .contains("no text"));
        let doc = format!("<w:document {W}/>");
        let z = make_zip(&[("word/document.xml", doc.as_bytes())]);
        assert!(convert_docx(&z)
            .unwrap_err()
            .to_string()
            .contains("no body"));
    }

    #[test]
    fn style_name_rules() {
        let mut s = Styles::default();
        s.names.insert("H2".into(), "heading 2".into());
        assert_eq!(s.heading_level("H2"), Some(2));
        assert_eq!(s.heading_level("Heading3"), Some(3), "falls back to the id");
        assert_eq!(s.heading_level("Normal"), None);
        assert_eq!(s.heading_level("Title"), Some(1));
        s.names.insert("H9".into(), "heading 9".into());
        assert_eq!(s.heading_level("H9"), Some(6));
    }

    #[test]
    fn sniffing() {
        assert!(DocxReader.sniff(&sample()));
        assert!(!DocxReader.sniff(&make_zip(&[("xl/workbook.xml", b"x")])));
        assert!(!DocxReader.sniff(b"hello"));
        assert!(DocxReader.sniff(&make_zip(&[("word/document.xml", b"<x/>")])[..60]));
        assert_eq!(DocxReader.id(), "docx");
        assert_eq!(DocxReader.extensions(), &["docx"]);
    }

    #[test]
    fn robust_against_garbage_and_mutations() {
        assert_rejects_garbage(&DocxReader);
        fuzz_no_panic(&DocxReader, &sample());
    }
}
