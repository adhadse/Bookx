//! OpenDocument Text (`.odt`) reader.
//!
//! Supported: headings (`text:h`), paragraphs, character styles from automatic and named styles
//! (bold, italic, underline, strike, super/subscript, resolved through parent styles), centred
//! and right aligned paragraphs, nested bulleted / numbered lists, tables, hyperlinks and
//! bookmarks, embedded raster images (`Pictures/`), footnotes and endnotes, and `meta.xml`
//! metadata. Ignored: headers/footers, text boxes, tracked changes, tables of contents.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::Metadata;
use bookx_core::util::{parse_xml, resolve_href, xml_escape};
use roxmltree::Node;

use crate::common::*;

const MAX_DEPTH: usize = 128;
const MIME: &[u8] = b"application/vnd.oasis.opendocument.text";

/// Reader for `.odt` files.
pub struct OdtReader;

impl FormatReader for OdtReader {
    fn id(&self) -> &'static str {
        "odt"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["odt", "ott"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(b"PK\x03\x04")
            && head[..head.len().min(256)]
                .windows(MIME.len())
                .any(|w| w == MIME)
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_odt(bytes)
    }
}

#[derive(Clone, Default, PartialEq, Debug)]
struct Fmt {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    sup: bool,
    sub: bool,
    /// `center`, `end` or empty
    align: String,
}

#[derive(Default)]
struct StyleDef {
    parent: Option<String>,
    own: Fmt,
    /// Which properties the style itself sets (so a child can switch them off).
    set: Vec<&'static str>,
}

#[derive(Default)]
struct Styles {
    defs: HashMap<String, StyleDef>,
    /// list style name -> ordered?
    lists: HashMap<String, bool>,
}

fn collect_styles(doc: &roxmltree::Document, s: &mut Styles) {
    for st in doc.descendants().filter(|n| n.is_element()) {
        match st.tag_name().name() {
            "style" => {
                let Some(name) = attr(st, "name") else {
                    continue;
                };
                let mut def = StyleDef {
                    parent: attr(st, "parent-style-name").map(str::to_string),
                    ..Default::default()
                };
                for p in st.children().filter(|c| c.is_element()) {
                    match p.tag_name().name() {
                        "text-properties" => {
                            if let Some(w) = attr(p, "font-weight") {
                                def.own.bold =
                                    w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600);
                                def.set.push("bold");
                            }
                            if let Some(i) = attr(p, "font-style") {
                                def.own.italic = i == "italic" || i == "oblique";
                                def.set.push("italic");
                            }
                            if let Some(u) = attr(p, "text-underline-style") {
                                def.own.underline = u != "none";
                                def.set.push("underline");
                            }
                            if let Some(l) = attr(p, "text-line-through-style") {
                                def.own.strike = l != "none";
                                def.set.push("strike");
                            }
                            if let Some(pos) = attr(p, "text-position") {
                                let first = pos.split_whitespace().next().unwrap_or("");
                                def.own.sup = first.starts_with("super")
                                    || (!first.starts_with('-')
                                        && first
                                            .chars()
                                            .next()
                                            .is_some_and(|c| c.is_ascii_digit())
                                        && first != "0%");
                                def.own.sub = first.starts_with("sub") || first.starts_with('-');
                                def.set.push("pos");
                            }
                        }
                        "paragraph-properties" => {
                            if let Some(a) = attr(p, "text-align") {
                                def.own.align = match a {
                                    "center" => "center",
                                    "end" | "right" => "end",
                                    _ => "",
                                }
                                .to_string();
                                def.set.push("align");
                            }
                        }
                        _ => {}
                    }
                }
                s.defs.insert(name.to_string(), def);
            }
            "list-style" => {
                if let Some(name) = attr(st, "name") {
                    let first = st.children().find(|c| c.is_element());
                    let ordered =
                        first.is_some_and(|f| f.tag_name().name() == "list-level-style-number");
                    s.lists.insert(name.to_string(), ordered);
                }
            }
            _ => {}
        }
    }
}

impl Styles {
    fn resolve(&self, name: &str) -> Fmt {
        let mut chain = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            if chain.len() > 10 {
                break;
            }
            let Some(def) = self.defs.get(&n) else { break };
            cur = def.parent.clone();
            chain.push(def);
        }
        let mut f = Fmt::default();
        for def in chain.into_iter().rev() {
            for k in &def.set {
                match *k {
                    "bold" => f.bold = def.own.bold,
                    "italic" => f.italic = def.own.italic,
                    "underline" => f.underline = def.own.underline,
                    "strike" => f.strike = def.own.strike,
                    "pos" => {
                        f.sup = def.own.sup;
                        f.sub = def.own.sub;
                    }
                    "align" => f.align = def.own.align.clone(),
                    _ => {}
                }
            }
        }
        f
    }
}

struct Ctx<'z> {
    zip: &'z ZipFiles,
    styles: Styles,
    media: MediaSet,
    notes: Vec<(String, String)>,
}

fn collapse_ws(s: &str, out: &mut String) {
    let mut last_space = out.ends_with(' ');
    for c in s.chars() {
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
}

fn wrap(f: &Fmt, inner: String) -> String {
    if inner.is_empty() || inner.starts_with("<img") {
        return inner;
    }
    let mut s = inner;
    for (on, tag) in [
        (f.sub, "sub"),
        (f.sup, "sup"),
        (f.strike, "del"),
        (f.underline, "u"),
        (f.italic, "em"),
        (f.bold, "strong"),
    ] {
        if on {
            s = format!("<{tag}>{s}</{tag}>");
        }
    }
    s
}

impl Ctx<'_> {
    fn inline(&mut self, n: Node, out: &mut String, base: &Fmt, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for c in n.children() {
            if c.is_text() {
                let mut t = String::new();
                collapse_ws(c.text().unwrap_or(""), &mut t);
                out.push_str(&xml_escape(&t));
                continue;
            }
            if !c.is_element() {
                continue;
            }
            match c.tag_name().name() {
                "span" => {
                    let own = attr(c, "style-name")
                        .map(|s| self.styles.resolve(s))
                        .unwrap_or_default();
                    let f = Fmt {
                        bold: own.bold && !base.bold,
                        italic: own.italic && !base.italic,
                        underline: own.underline && !base.underline,
                        strike: own.strike && !base.strike,
                        sup: own.sup && !base.sup,
                        sub: own.sub && !base.sub,
                        align: String::new(),
                    };
                    let mut inner = String::new();
                    self.inline(c, &mut inner, base, depth + 1);
                    out.push_str(&wrap(&f, inner));
                }
                "a" => {
                    let mut inner = String::new();
                    self.inline(c, &mut inner, base, depth + 1);
                    match attr(c, "href") {
                        Some(h) if !inner.is_empty() => {
                            out.push_str(&format!("<a href=\"{}\">{inner}</a>", xml_escape(h)))
                        }
                        _ => out.push_str(&inner),
                    }
                }
                "s" => {
                    let n = attr(c, "c")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(100);
                    out.push_str(&"\u{a0}".repeat(n));
                }
                "tab" => out.push_str("\u{a0}\u{a0}\u{a0}\u{a0}"),
                "line-break" => out.push_str("<br/>"),
                "bookmark" | "bookmark-start" => {
                    if let Some(name) = attr(c, "name") {
                        out.push_str(&format!("<a id=\"{}\"></a>", xml_escape(name)));
                    }
                }
                "frame" => {
                    for img in c
                        .descendants()
                        .filter(|d| d.is_element() && d.tag_name().name() == "image")
                    {
                        if let Some(h) = attr(img, "href") {
                            out.push_str(&self.image(h));
                        }
                    }
                }
                "note" => {
                    let id = attr(c, "id").unwrap_or("").to_string();
                    let id = format!("note{}-{}", self.notes.len() + 1, sanitize_stem(&id));
                    let label = child_el(c, "note-citation")
                        .map(node_text)
                        .unwrap_or_default();
                    let label = if label.trim().is_empty() {
                        format!("{}", self.notes.len() + 1)
                    } else {
                        label.trim().to_string()
                    };
                    let mut body = String::new();
                    if let Some(nb) = child_el(c, "note-body") {
                        let mut blocks = Vec::new();
                        self.blocks(nb, &mut blocks, depth + 1);
                        body = blocks.concat();
                    }
                    out.push_str(&noteref(&id, &format!("[{label}]")));
                    self.notes.push((id, body));
                }
                "soft-page-break" | "annotation" | "annotation-end" | "change-start"
                | "change-end" | "change" => {}
                _ => self.inline(c, out, base, depth + 1),
            }
        }
    }

    fn image(&mut self, href: &str) -> String {
        if bookx_core::util::is_external(href) {
            return String::new();
        }
        let (path, _) = resolve_href("", href);
        let Some(data) = self.zip.get(&path) else {
            return String::new();
        };
        match self.media.add(&path, data.to_vec()) {
            Some(p) => format!("<img src=\"{p}\" alt=\"\"/>"),
            None => String::new(),
        }
    }

    fn para_fmt(&self, n: Node) -> Fmt {
        attr(n, "style-name")
            .map(|s| self.styles.resolve(s))
            .unwrap_or_default()
    }

    fn blocks(&mut self, parent: Node, out: &mut Vec<String>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for c in parent.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "h" | "p" => {
                    let f = self.para_fmt(c);
                    let mut inner = String::new();
                    self.inline(c, &mut inner, &Fmt::default(), 0);
                    if visible_text_len(&inner) == 0 && !inner.contains("<img") {
                        continue;
                    }
                    // Formatting that applies to the whole paragraph.
                    let whole = Fmt {
                        align: String::new(),
                        ..f.clone()
                    };
                    let inner = wrap(&whole, inner);
                    let style = match f.align.as_str() {
                        "center" => " style=\"text-align:center\"",
                        "end" => " style=\"text-align:right\"",
                        _ => "",
                    };
                    let level = if c.tag_name().name() == "h" {
                        Some(
                            attr(c, "outline-level")
                                .and_then(|l| l.parse::<usize>().ok())
                                .unwrap_or(1)
                                .clamp(1, 6),
                        )
                    } else if attr(c, "style-name") == Some("Title") {
                        Some(1)
                    } else {
                        None
                    };
                    out.push(match level {
                        Some(l) => format!("<h{l}{style}>{inner}</h{l}>\n"),
                        None => format!("<p{style}>{inner}</p>\n"),
                    });
                }
                "list" => out.push(self.list(c, depth + 1)),
                "table" => {
                    let mut t = String::from("<table>\n");
                    for row in c
                        .descendants()
                        .filter(|d| d.is_element() && d.tag_name().name() == "table-row")
                    {
                        t.push_str("<tr>");
                        for cell in row
                            .children()
                            .filter(|d| d.is_element() && d.tag_name().name() == "table-cell")
                        {
                            let mut cell_blocks = Vec::new();
                            self.blocks(cell, &mut cell_blocks, depth + 1);
                            t.push_str(&format!("<td>{}</td>", cell_blocks.concat()));
                        }
                        t.push_str("</tr>\n");
                    }
                    t.push_str("</table>\n");
                    out.push(t);
                }
                "section" | "index-body" | "text" | "body" => self.blocks(c, out, depth + 1),
                "frame" => {
                    for img in c
                        .descendants()
                        .filter(|d| d.is_element() && d.tag_name().name() == "image")
                    {
                        if let Some(h) = attr(img, "href") {
                            let tag = self.image(h);
                            if !tag.is_empty() {
                                out.push(format!("<p>{tag}</p>\n"));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn list(&mut self, l: Node, depth: usize) -> String {
        let ordered = attr(l, "style-name")
            .and_then(|s| self.styles.lists.get(s))
            .copied()
            .unwrap_or(false);
        let tag = if ordered { "ol" } else { "ul" };
        let mut s = format!("<{tag}>\n");
        if depth <= MAX_DEPTH {
            for item in l.children().filter(|c| {
                c.is_element() && matches!(c.tag_name().name(), "list-item" | "list-header")
            }) {
                let mut inner = Vec::new();
                self.blocks(item, &mut inner, depth + 1);
                // Single paragraphs read better as bare list item text.
                let joined = inner.concat();
                let simple =
                    inner.len() == 1 && joined.starts_with("<p>") && joined.ends_with("</p>\n");
                let body = if simple {
                    joined[3..joined.len() - 5].to_string()
                } else {
                    joined
                };
                s.push_str(&format!("<li>{body}</li>\n"));
            }
        }
        s.push_str(&format!("</{tag}>\n"));
        s
    }
}

fn read_meta(zip: &ZipFiles) -> Metadata {
    let mut md = Metadata::default();
    let Some(data) = zip.get("meta.xml") else {
        return md;
    };
    let text = decode_xml(data);
    let Ok(doc) = parse_xml(&text) else { return md };
    let mut initial = String::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        let t = node_text(n).trim().to_string();
        match n.tag_name().name() {
            "title" => md.title = t,
            "creator" if !t.is_empty() => md.authors.push(t),
            "initial-creator" => initial = t,
            "language" => md.language = t,
            "description" => md.description = t,
            "subject" if md.description.is_empty() => md.description = t,
            "keyword" if !t.is_empty() => md.tags.push(t),
            _ => {}
        }
    }
    if md.authors.is_empty() && !initial.is_empty() {
        md.authors.push(initial);
    }
    md
}

/// Converts ODT bytes to an EPUB archive.
pub fn convert_odt(bytes: &[u8]) -> Result<Archive> {
    let zip = read_zip(bytes)?;
    let Some(content) = zip.get("content.xml") else {
        bail!("not an OpenDocument file: content.xml is missing");
    };
    let text = decode_xml(content);
    let doc = parse_xml(&text).context("invalid content.xml")?;
    let Some(office_text) = doc.descendants().find(|n| {
        n.is_element()
            && n.tag_name().name() == "text"
            && n.parent().is_some_and(|p| p.tag_name().name() == "body")
    }) else {
        bail!("the document is not a text document");
    };

    let mut styles = Styles::default();
    let styles_text = zip.get("styles.xml").map(decode_xml);
    let styles_doc = styles_text.as_deref().and_then(|t| parse_xml(t).ok());
    if let Some(sd) = &styles_doc {
        collect_styles(sd, &mut styles);
    }
    collect_styles(&doc, &mut styles);

    let mut ctx = Ctx {
        zip: &zip,
        styles,
        media: MediaSet::default(),
        notes: Vec::new(),
    };
    let mut blocks = Vec::new();
    ctx.blocks(office_text, &mut blocks, 0);
    if blocks.is_empty() {
        bail!("the document has no text");
    }
    let md = read_meta(&zip);
    let default_title = if md.title.is_empty() {
        "Beginning".to_string()
    } else {
        md.title.clone()
    };
    let mut chapters = chapters_from_blocks(blocks, &default_title);
    if !ctx.notes.is_empty() {
        let mut body = String::from("<h1>Notes</h1>\n");
        for (id, inner) in &ctx.notes {
            body.push_str(&footnote_aside(id, inner));
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

    const NS: &str = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\"";

    pub fn sample() -> Vec<u8> {
        let content = format!(
            r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content {NS}>
<office:automatic-styles>
 <style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
 <style:style style:name="T2" style:family="text"><style:text-properties fo:font-style="italic" style:text-underline-style="solid"/></style:style>
 <style:style style:name="T3" style:family="text"><style:text-properties style:text-line-through-style="solid" style:text-position="super 58%"/></style:style>
 <style:style style:name="T4" style:family="text"><style:text-properties style:text-position="sub 58%"/></style:style>
 <style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:text-align="center"/></style:style>
 <style:style style:name="P2" style:family="paragraph" style:parent-style-name="BoldBase"/>
 <text:list-style style:name="LBullet"><text:list-level-style-bullet text:level="1"/></text:list-style>
 <text:list-style style:name="LNum"><text:list-level-style-number text:level="1"/></text:list-style>
</office:automatic-styles>
<office:body><office:text>
<text:h text:style-name="Heading_20_1" text:outline-level="1"><text:bookmark-start text:name="start"/>Chapter One</text:h>
<text:p>Plain <text:span text:style-name="T1">bold</text:span> <text:span text:style-name="T2">italic under</text:span> <text:span text:style-name="T3">sup</text:span><text:span text:style-name="T4">sub</text:span>
 and   spaced<text:s text:c="2"/>out<text:line-break/>next<text:tab/>tab.</text:p>
<text:p text:style-name="P1">Centered</text:p>
<text:p text:style-name="P2">Whole paragraph bold</text:p>
<text:p>Link <text:a xlink:href="http://example.com/?a=1&amp;b=2">out</text:a> and <text:a xlink:href="#start">in</text:a>.<text:note text:id="ftn1" text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p>Footnote body.</text:p></text:note-body></text:note></text:p>
<text:list text:style-name="LBullet"><text:list-item><text:p>Bullet A</text:p></text:list-item><text:list-item><text:p>Bullet B</text:p>
  <text:list text:style-name="LBullet"><text:list-item><text:p>Nested</text:p></text:list-item></text:list></text:list-item></text:list>
<text:list text:style-name="LNum"><text:list-item><text:p>First</text:p></text:list-item></text:list>
<text:p><draw:frame><draw:image xlink:href="Pictures/img1.png"/></draw:frame><draw:frame><draw:image xlink:href="http://remote/x.png"/></draw:frame></text:p>
<table:table><table:table-row><table:table-cell><text:p>Cell 1</text:p></table:table-cell><table:table-cell><text:p>Cell 2</text:p></table:table-cell></table:table-row></table:table>
<text:h text:outline-level="1">Chapter Two</text:h>
<text:h text:outline-level="2">Sub</text:h>
<text:p>End &amp; text</text:p>
<text:p>   </text:p>
</office:text></office:body></office:document-content>"##
        );
        let styles = format!(
            r#"<?xml version="1.0"?><office:document-styles {NS}><office:styles>
<style:style style:name="Standard" style:family="paragraph"/>
<style:style style:name="BoldBase" style:family="paragraph"><style:text-properties fo:font-weight="700"/></style:style>
</office:styles></office:document-styles>"#
        );
        let meta = format!(
            r#"<?xml version="1.0"?><office:document-meta {NS}><office:meta><dc:title>ODT Title</dc:title><meta:initial-creator>Olga Original</meta:initial-creator><dc:creator>Olga Editor</dc:creator>
<dc:language>fr</dc:language><dc:description>Odt about</dc:description><meta:keyword>k1</meta:keyword><meta:keyword>k2</meta:keyword></office:meta></office:document-meta>"#
        );
        make_zip_stored(&[
            ("mimetype", b"application/vnd.oasis.opendocument.text"),
            ("content.xml", content.as_bytes()),
            ("styles.xml", styles.as_bytes()),
            ("meta.xml", meta.as_bytes()),
            ("Pictures/img1.png", PNG),
        ])
    }

    #[test]
    fn metadata_from_meta_xml() {
        let book = validate(&convert_odt(&sample()).unwrap());
        let md = book.metadata();
        assert_eq!(md.title, "ODT Title");
        assert_eq!(md.authors, vec!["Olga Editor"]);
        assert_eq!(md.language, "fr");
        assert_eq!(md.description, "Odt about");
        assert_eq!(md.tags, vec!["k1", "k2"]);
        assert_eq!(
            toc_titles(&book),
            vec!["Chapter One", "Chapter Two", "Sub", "Notes"]
        );
    }

    #[test]
    fn inline_styles_whitespace_and_alignment() {
        let book = validate(&convert_odt(&sample()).unwrap());
        let x = chapter_xml(&book, 0);
        assert!(x.contains("Plain <strong>bold</strong> <em><u>italic under</u></em> <del><sup>sup</sup></del><sub>sub</sub>"), "{x}");
        assert!(x.contains("and spaced\u{a0}\u{a0}out<br/>next"), "{x}");
        assert!(x.contains("<p style=\"text-align:center\">Centered</p>"));
        assert!(
            x.contains("<p><strong>Whole paragraph bold</strong></p>"),
            "{x}"
        );
        assert!(x.contains("<h1><a id=\"start\"></a>Chapter One</h1>"));
    }

    #[test]
    fn links_lists_tables_images_and_notes() {
        let arc = convert_odt(&sample()).unwrap();
        let book = validate(&arc);
        let x = chapter_xml(&book, 0);
        assert!(
            x.contains("<a href=\"http://example.com/?a=1&amp;b=2\">out</a>"),
            "{x}"
        );
        assert!(x.contains("<a href=\"#start\">in</a>"));
        assert!(x.contains("<ul>\n<li>Bullet A</li>\n<li><p>Bullet B</p>\n<ul>\n<li>Nested</li>\n</ul>\n</li>\n</ul>"), "{x}");
        assert!(x.contains("<ol>\n<li>First</li>\n</ol>"));
        assert!(x.contains("<td><p>Cell 1</p>\n</td><td><p>Cell 2</p>\n</td>"));
        assert!(x.contains("<img src=\"images/img1.png\" alt=\"\"/>"));
        assert_eq!(arc.get("OEBPS/images/img1.png"), Some(PNG));
        assert_eq!(x.matches("<img").count(), 1, "remote images are skipped");
        assert!(x.contains("epub:type=\"noteref\""));
        let notes = chapter_xml(&book, 2);
        assert!(notes.contains("epub:type=\"footnote\"") && notes.contains("Footnote body."));
        assert!(x.contains("href=\"chapter-3.xhtml#note1-ftn1\""), "{x}");
    }

    #[test]
    fn second_chapter_and_subheadings() {
        let book = validate(&convert_odt(&sample()).unwrap());
        let x = chapter_xml(&book, 1);
        assert!(x.contains("<h1>Chapter Two</h1>") && x.contains(">Sub</h2>"));
        assert!(x.contains("End &amp; text"));
    }

    #[test]
    fn style_inheritance() {
        let xml = format!(
            r#"<r {NS}><style:style style:name="A"><style:text-properties fo:font-weight="bold" fo:font-style="italic"/></style:style>
<style:style style:name="B" style:parent-style-name="A"><style:text-properties fo:font-weight="normal"/></style:style>
<style:style style:name="Loop1" style:parent-style-name="Loop2"/><style:style style:name="Loop2" style:parent-style-name="Loop1"/></r>"#
        );
        let doc = parse_xml(&xml).unwrap();
        let mut s = Styles::default();
        collect_styles(&doc, &mut s);
        let b = s.resolve("B");
        assert!(
            !b.bold && b.italic,
            "child overrides bold, inherits italic: {b:?}"
        );
        assert!(s.resolve("A").bold);
        assert_eq!(s.resolve("Loop1"), Fmt::default(), "cycles terminate");
        assert_eq!(s.resolve("missing"), Fmt::default());
    }

    #[test]
    fn errors() {
        assert!(convert_odt(b"junk").is_err());
        let z = make_zip(&[("a.txt", b"x")]);
        assert!(
            convert_odt(&z)
                .unwrap_err()
                .to_string()
                .contains("content.xml")
        );
        let z = make_zip(&[("content.xml", b"<office:document-content")]);
        assert!(convert_odt(&z).is_err());
        let z = make_zip(&[("content.xml", format!("<office:document-content {NS}><office:body><office:spreadsheet/></office:body></office:document-content>").as_bytes())]);
        assert!(
            convert_odt(&z)
                .unwrap_err()
                .to_string()
                .contains("not a text document")
        );
        let z = make_zip(&[("content.xml", format!("<office:document-content {NS}><office:body><office:text><text:p/></office:text></office:body></office:document-content>").as_bytes())]);
        assert!(convert_odt(&z).unwrap_err().to_string().contains("no text"));
    }

    #[test]
    fn initial_creator_is_used_when_there_is_no_creator() {
        let content = format!(
            "<office:document-content {NS}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"
        );
        let meta = format!(
            "<office:document-meta {NS}><office:meta><meta:initial-creator>Only Initial</meta:initial-creator></office:meta></office:document-meta>"
        );
        let z = make_zip(&[
            ("content.xml", content.as_bytes()),
            ("meta.xml", meta.as_bytes()),
        ]);
        let book = validate(&convert_odt(&z).unwrap());
        assert_eq!(book.metadata().authors, vec!["Only Initial"]);
    }

    #[test]
    fn sniffing() {
        assert!(OdtReader.sniff(&sample()));
        assert!(!OdtReader.sniff(&make_zip_stored(&[(
            "mimetype",
            b"application/vnd.oasis.opendocument.spreadsheet"
        )])));
        assert!(!OdtReader.sniff(b"hello"));
        assert!(!OdtReader.sniff(b""));
        assert_eq!(OdtReader.id(), "odt");
        assert!(OdtReader.extensions().contains(&"odt"));
    }

    #[test]
    fn robust_against_garbage_and_mutations() {
        assert_rejects_garbage(&OdtReader);
        fuzz_no_panic(&OdtReader, &sample());
    }
}
