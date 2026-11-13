//! FictionBook 2 (`.fb2`) and zipped FictionBook (`.fbz` / `.fb2.zip`).

use std::collections::{HashMap, HashSet};

use anyhow::{bail, Context, Result};
use base64::Engine;
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::{Identifier, Metadata};
use bookx_core::toc::{write_toc, TocEntry};
use bookx_core::util::{parse_xml, xml_escape};
use roxmltree::Node;

use crate::common::*;

const MAX_DEPTH: usize = 256;
const EPUB_NS: &str = "xmlns:epub=\"http://www.idpf.org/2007/ops\"";

const CSS: &str = "\
.epigraph { margin: 1em 2em; font-style: italic; }\n\
.cite { margin: 1em 2em; }\n\
.annotation { margin: 1em 2em; font-size: .9em; }\n\
.poem { margin: 1em 2em; }\n\
.stanza { margin: 0 0 1em 0; }\n\
.stanza p, p.v { margin: 0; text-align: left; }\n\
.text-author { text-align: right; font-style: italic; }\n\
.subtitle { text-align: center; font-weight: bold; }\n\
.empty-line { margin: 0; }\n\
.image { text-align: center; }\n\
img { max-width: 100%; }\n";

/// Reader for `.fb2` files.
pub struct Fb2Reader;
/// Reader for `.fbz` / `.fb2.zip` files.
pub struct FbzReader;

impl FormatReader for Fb2Reader {
    fn id(&self) -> &'static str {
        "fb2"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["fb2"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        let n = head.len().min(4096);
        String::from_utf8_lossy(&head[..n]).contains("<FictionBook")
            || decode_xml(&head[..n]).contains("<FictionBook")
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_fb2(bytes)
    }
}

impl FormatReader for FbzReader {
    fn id(&self) -> &'static str {
        "fbz"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["fbz"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        zip_names(head)
            .map(|n| n.iter().any(|f| f.to_ascii_lowercase().ends_with(".fb2")))
            .unwrap_or(false)
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        let zip = read_zip(bytes)?;
        let Some((_, data)) = zip
            .files
            .iter()
            .find(|(n, _)| n.to_ascii_lowercase().ends_with(".fb2"))
        else {
            bail!("the zip contains no .fb2 file");
        };
        convert_fb2(data)
    }
}

/// Converts FB2 bytes (any encoding declared in the XML header) to an EPUB archive.
pub fn convert_fb2(bytes: &[u8]) -> Result<Archive> {
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        bail!("empty FB2 file");
    }
    let text = decode_xml(bytes);
    let doc = parse_xml(&text).context("invalid FB2 XML")?;
    let root = doc.root_element();
    if root.tag_name().name() != "FictionBook" {
        bail!("not a FictionBook document");
    }

    let mut ctx = Ctx::default();
    for b in children_named(root, "binary") {
        ctx.add_binary(b);
    }
    let mut parts = Parts {
        css: CSS.into(),
        ..Default::default()
    };
    if let Some(desc) = child(root, "description") {
        read_description(desc, &ctx, &mut parts);
    }

    // Notes bodies first, so links can be recognised as note references.
    let mut note_bodies = Vec::new();
    let mut main_bodies = Vec::new();
    for b in children_named(root, "body") {
        if matches!(b.attribute("name"), Some("notes") | Some("comments")) {
            note_bodies.push(b);
        } else {
            main_bodies.push(b);
        }
    }
    for b in &note_bodies {
        collect_ids(*b, &mut ctx.note_ids);
    }
    for b in main_bodies {
        ctx.body(b, &mut parts.chapters);
    }
    if !note_bodies.is_empty() {
        let mut body = String::from("<h1>Notes</h1>\n");
        for b in note_bodies {
            for s in children_named(b, "section") {
                ctx.note_section(s, &mut body, 0);
            }
        }
        ctx.marks.push((1, "Notes".into(), parts.chapters.len()));
        parts.chapters.push(Chapter {
            title: "Notes".into(),
            body,
        });
    }
    if parts.chapters.is_empty() {
        bail!("the FB2 file has no text");
    }
    parts.media = ctx.used_media();
    let mut arc = build(parts)?;
    // contents nested like the sections
    if ctx.marks.iter().any(|m| m.0 > 1) {
        let mut roots: Vec<TocEntry> = Vec::new();
        let mut path: Vec<usize> = Vec::new(); // index of the open entry at each depth
        for (depth, title, ch) in &ctx.marks {
            let depth = (*depth).max(1);
            path.truncate(depth - 1);
            let entry = TocEntry {
                title: title.clone(),
                href: format!("OEBPS/chapter-{}.xhtml", ch + 1),
                children: Vec::new(),
            };
            let mut list = &mut roots;
            for ix in &path {
                list = &mut list[*ix].children;
            }
            list.push(entry);
            path.push(list.len() - 1);
        }
        if let Ok(book) = bookx_core::book::OpenBook::from_archive(arc.clone()) {
            let pkg = book.package.clone();
            let mut a = book.archive;
            if write_toc(&mut a, &pkg, &roots).is_ok() {
                arc = a;
            }
        }
    }
    Ok(arc)
}

fn local<'a>(n: Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && local(*c) == name)
}

fn children_named<'a, 'i>(
    n: Node<'a, 'i>,
    name: &'static str,
) -> impl Iterator<Item = Node<'a, 'i>> {
    n.children()
        .filter(move |c| c.is_element() && local(*c) == name)
}

fn href<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.name() == "href")
        .map(|a| a.value())
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn plain(n: Node) -> String {
    fn go(n: Node, out: &mut String) {
        for c in n.children() {
            if c.is_text() {
                out.push_str(c.text().unwrap_or(""));
            } else if c.is_element() {
                go(c, out);
                if matches!(
                    local(c),
                    "p" | "v" | "subtitle" | "text-author" | "empty-line" | "title"
                ) {
                    out.push(' ');
                }
            }
        }
    }
    let mut s = String::new();
    go(n, &mut s);
    collapse(&s)
}

fn collect_ids(n: Node, out: &mut HashSet<String>) {
    for d in n.descendants().filter(|d| d.is_element()) {
        if let Some(id) = d.attribute("id") {
            out.insert(id.to_string());
        }
    }
}

fn person(n: Node) -> String {
    let get = |t: &str| child(n, t).map(plain).unwrap_or_default();
    let parts: Vec<String> = [get("first-name"), get("middle-name"), get("last-name")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        get("nickname")
    } else {
        parts.join(" ")
    }
}

fn read_description(desc: Node, ctx: &Ctx, parts: &mut Parts) {
    let md: &mut Metadata = &mut parts.md;
    if let Some(ti) = child(desc, "title-info") {
        md.title = child(ti, "book-title").map(plain).unwrap_or_default();
        md.authors = children_named(ti, "author")
            .map(person)
            .filter(|a| !a.is_empty())
            .collect();
        md.tags = children_named(ti, "genre")
            .map(plain)
            .filter(|g| !g.is_empty())
            .collect();
        md.language = child(ti, "lang").map(plain).unwrap_or_default();
        if let Some(a) = child(ti, "annotation") {
            let paras: Vec<String> = a
                .descendants()
                .filter(|d| d.is_element() && matches!(local(*d), "p" | "v"))
                .map(plain)
                .filter(|p| !p.is_empty())
                .collect();
            md.description = if paras.is_empty() {
                plain(a)
            } else {
                paras.join("\n\n")
            };
        }
        if let Some(d) = child(ti, "date") {
            md.published = d
                .attribute("value")
                .map(str::to_string)
                .unwrap_or_else(|| plain(d));
        }
        if let Some(s) = child(ti, "sequence") {
            md.series = s.attribute("name").unwrap_or("").trim().to_string();
            if let Some(n) = s
                .attribute("number")
                .and_then(|n| n.trim().parse::<f64>().ok())
            {
                md.series_index = n;
            }
        }
        if let Some(img) = child(ti, "coverpage").and_then(|c| child(c, "image")) {
            if let Some(id) = href(img).map(|h| h.trim_start_matches('#')) {
                if let Some(b) = ctx.binaries.get(id) {
                    parts.cover = Some((b.data.clone(), b.ext.to_string()));
                }
            }
        }
    }
    if let Some(pi) = child(desc, "publish-info") {
        md.publisher = child(pi, "publisher").map(plain).unwrap_or_default();
        if let Some(isbn) = child(pi, "isbn").map(plain).filter(|i| !i.is_empty()) {
            md.identifiers.push(Identifier {
                scheme: "isbn".into(),
                value: isbn,
                xml_id: None,
            });
        }
    }
}

struct Binary {
    data: Vec<u8>,
    ext: &'static str,
    path: String,
}

#[derive(Default)]
struct Ctx {
    binaries: HashMap<String, Binary>,
    used: Vec<String>,
    note_ids: HashSet<String>,
    /// `(section depth, title, chapter index)` of every titled section, for the nested contents.
    marks: Vec<(usize, String, usize)>,
}

impl Ctx {
    fn add_binary(&mut self, n: Node) {
        let Some(id) = n.attribute("id") else { return };
        let raw: String = n
            .text()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let engine = base64::engine::GeneralPurpose::new(
            &base64::alphabet::STANDARD,
            base64::engine::GeneralPurposeConfig::new()
                .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
                .with_decode_allow_trailing_bits(true),
        );
        let Ok(data) = engine.decode(raw.as_bytes()) else {
            return;
        };
        let ext = image_ext(&data).or(match n.attribute("content-type") {
            Some("image/jpeg") | Some("image/jpg") => Some("jpg"),
            Some("image/png") => Some("png"),
            Some("image/gif") => Some("gif"),
            _ => None,
        });
        let Some(ext) = ext else { return };
        let stem = id
            .rsplit_once('.')
            .filter(|(_, e)| {
                matches!(
                    e.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "gif"
                )
            })
            .map(|(s, _)| s)
            .unwrap_or(id);
        let mut path = format!("images/{}.{ext}", sanitize_stem(stem));
        let mut k = 1;
        while self.binaries.values().any(|b| b.path == path) {
            k += 1;
            path = format!("images/{}-{k}.{ext}", sanitize_stem(stem));
        }
        self.binaries
            .insert(id.to_string(), Binary { data, ext, path });
    }

    fn used_media(&self) -> Vec<Media> {
        self.used
            .iter()
            .filter_map(|id| self.binaries.get(id))
            .map(|b| Media {
                path: b.path.clone(),
                data: b.data.clone(),
            })
            .collect()
    }

    fn image(&mut self, n: Node, block: bool) -> String {
        let Some(id) = href(n).map(|h| h.trim_start_matches('#').to_string()) else {
            return String::new();
        };
        let Some(b) = self.binaries.get(&id) else {
            return String::new();
        };
        let path = b.path.clone();
        if !self.used.contains(&id) {
            self.used.push(id);
        }
        let alt = n.attribute("alt").unwrap_or("");
        let title = n
            .attribute("title")
            .map(|t| format!(" title=\"{}\"", xml_escape(t)))
            .unwrap_or_default();
        let tag = format!("<img src=\"{path}\" alt=\"{}\"{title}/>", xml_escape(alt));
        if block {
            format!("<div class=\"image\">{tag}</div>\n")
        } else {
            tag
        }
    }

    fn inline(&mut self, n: Node, out: &mut String, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for c in n.children() {
            if c.is_text() {
                out.push_str(&xml_escape(c.text().unwrap_or("")));
                continue;
            }
            if !c.is_element() {
                continue;
            }
            let wrap = |ctx: &mut Ctx, tag: &str, attrs: &str, out: &mut String| {
                out.push_str(&format!("<{tag}{attrs}>"));
                ctx.inline(c, out, depth + 1);
                out.push_str(&format!("</{tag}>"));
            };
            match local(c) {
                "strong" => wrap(self, "strong", "", out),
                "emphasis" => wrap(self, "em", "", out),
                "strikethrough" => wrap(self, "del", "", out),
                "sub" => wrap(self, "sub", "", out),
                "sup" => wrap(self, "sup", "", out),
                "code" => wrap(self, "code", "", out),
                "style" => {
                    let class = c.attribute("name").map(sanitize_stem).unwrap_or_default();
                    wrap(self, "span", &format!(" class=\"{class}\""), out)
                }
                "a" => {
                    let target = href(c).unwrap_or("");
                    let attrs = if let Some(frag) = target.strip_prefix('#') {
                        if c.attribute("type") == Some("note") || self.note_ids.contains(frag) {
                            format!(
                                " {EPUB_NS} epub:type=\"noteref\" href=\"#{}\"",
                                xml_escape(frag)
                            )
                        } else {
                            format!(" href=\"#{}\"", xml_escape(frag))
                        }
                    } else if target.is_empty() {
                        String::new()
                    } else {
                        format!(" href=\"{}\"", xml_escape(target))
                    };
                    wrap(self, "a", &attrs, out)
                }
                "image" => out.push_str(&self.image(c, false)),
                _ => self.inline(c, out, depth + 1),
            }
        }
    }

    fn inline_str(&mut self, n: Node) -> String {
        let mut s = String::new();
        self.inline(n, &mut s, 0);
        s
    }

    /// Block level children shared by sections, epigraphs, cites and annotations.
    fn block(&mut self, c: Node, out: &mut String, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let id = c
            .attribute("id")
            .map(|i| format!(" id=\"{}\"", xml_escape(i)))
            .unwrap_or_default();
        match local(c) {
            "p" => {
                let inner = self.inline_str(c);
                out.push_str(&format!("<p{id}>{inner}</p>\n"));
            }
            "empty-line" => out.push_str("<p class=\"empty-line\">&#160;</p>\n"),
            "subtitle" => {
                let inner = self.inline_str(c);
                out.push_str(&format!("<p class=\"subtitle\"{id}>{inner}</p>\n"));
            }
            "text-author" => {
                let inner = self.inline_str(c);
                out.push_str(&format!("<p class=\"text-author\">{inner}</p>\n"));
            }
            "image" => out.push_str(&self.image(c, true)),
            "epigraph" => {
                out.push_str(&format!("<blockquote class=\"epigraph\"{id}>\n"));
                for k in c.children().filter(|k| k.is_element()) {
                    self.block(k, out, depth + 1);
                }
                out.push_str("</blockquote>\n");
            }
            "cite" => {
                out.push_str(&format!("<blockquote class=\"cite\"{id}>\n"));
                for k in c.children().filter(|k| k.is_element()) {
                    self.block(k, out, depth + 1);
                }
                out.push_str("</blockquote>\n");
            }
            "annotation" => {
                out.push_str(&format!("<div class=\"annotation\"{id}>\n"));
                for k in c.children().filter(|k| k.is_element()) {
                    self.block(k, out, depth + 1);
                }
                out.push_str("</div>\n");
            }
            "poem" => {
                out.push_str(&format!("<div class=\"poem\"{id}>\n"));
                for k in c.children().filter(|k| k.is_element()) {
                    match local(k) {
                        "title" => {
                            let t = self.title_html(k, 3);
                            out.push_str(&t);
                        }
                        "stanza" => {
                            out.push_str("<div class=\"stanza\">\n");
                            for s in k.children().filter(|s| s.is_element()) {
                                match local(s) {
                                    "v" => {
                                        let inner = self.inline_str(s);
                                        out.push_str(&format!("<p class=\"v\">{inner}</p>\n"));
                                    }
                                    "title" => {
                                        let t = self.title_html(s, 4);
                                        out.push_str(&t);
                                    }
                                    _ => self.block(s, out, depth + 1),
                                }
                            }
                            out.push_str("</div>\n");
                        }
                        "date" => {
                            let inner = self.inline_str(k);
                            out.push_str(&format!("<p class=\"text-author\">{inner}</p>\n"));
                        }
                        _ => self.block(k, out, depth + 1),
                    }
                }
                out.push_str("</div>\n");
            }
            "table" => {
                out.push_str("<table>\n");
                for tr in children_named(c, "tr") {
                    out.push_str("<tr>");
                    for cell in tr
                        .children()
                        .filter(|k| k.is_element() && matches!(local(*k), "td" | "th"))
                    {
                        let tag = local(cell);
                        let mut attrs = String::new();
                        for a in ["colspan", "rowspan"] {
                            if let Some(v) = cell.attribute(a) {
                                attrs.push_str(&format!(" {a}=\"{}\"", xml_escape(v)));
                            }
                        }
                        let inner = self.inline_str(cell);
                        out.push_str(&format!("<{tag}{attrs}>{inner}</{tag}>"));
                    }
                    out.push_str("</tr>\n");
                }
                out.push_str("</table>\n");
            }
            "section" => {}
            _ => {}
        }
    }

    fn title_html(&mut self, t: Node, level: usize) -> String {
        let level = level.clamp(1, 6);
        let mut lines = Vec::new();
        for k in t.children().filter(|k| k.is_element()) {
            match local(k) {
                "p" => lines.push(self.inline_str(k)),
                "empty-line" => {}
                _ => {}
            }
        }
        if lines.is_empty() {
            return String::new();
        }
        format!("<h{level}>{}</h{level}>\n", lines.join("<br/>"))
    }

    fn body(&mut self, b: Node, chapters: &mut Vec<Chapter>) {
        let mut head = String::new();
        let mut head_title = String::new();
        for c in b.children().filter(|c| c.is_element()) {
            match local(c) {
                "title" => {
                    head.push_str(&self.title_html(c, 1));
                    head_title = plain(c);
                }
                "section" => {
                    if !head.is_empty() {
                        self.flush_marked(chapters, &mut head, &head_title, 1, &mut false);
                    }
                    self.section(c, 1, chapters);
                }
                _ => self.block(c, &mut head, 0),
            }
        }
        self.flush_marked(chapters, &mut head, &head_title, 1, &mut false);
    }

    /// [`flush`] that also notes the section in the nested table of contents. Only the first
    /// chapter of a section gets an entry; text that continues after a sub-section does not.
    fn flush_marked(
        &mut self,
        chapters: &mut Vec<Chapter>,
        body: &mut String,
        title: &str,
        depth: usize,
        recorded: &mut bool,
    ) {
        let before = chapters.len();
        flush(chapters, body, title);
        if chapters.len() > before && !*recorded && !title.is_empty() {
            self.marks.push((depth, title.to_string(), before));
            *recorded = true;
        }
    }

    fn section(&mut self, s: Node, depth: usize, chapters: &mut Vec<Chapter>) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut body = String::new();
        let mut title = String::new();
        let mut recorded = false;
        if let Some(id) = s.attribute("id") {
            body.push_str(&format!("<a id=\"{}\"></a>\n", xml_escape(id)));
        }
        for c in s.children().filter(|c| c.is_element()) {
            match local(c) {
                "title" => {
                    body.push_str(&self.title_html(c, depth + 1));
                    title = plain(c);
                }
                "section" => {
                    self.flush_marked(chapters, &mut body, &title, depth, &mut recorded);
                    self.section(c, depth + 1, chapters);
                }
                _ => self.block(c, &mut body, 0),
            }
        }
        self.flush_marked(chapters, &mut body, &title, depth, &mut recorded);
    }

    fn note_section(&mut self, s: Node, out: &mut String, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut inner = String::new();
        for c in s.children().filter(|c| c.is_element()) {
            match local(c) {
                "title" => {
                    let t = plain(c);
                    if !t.is_empty() {
                        inner.push_str(&format!("<p><strong>{}</strong></p>\n", xml_escape(&t)));
                    }
                }
                "section" => self.note_section(c, &mut inner, depth + 1),
                _ => self.block(c, &mut inner, 0),
            }
        }
        match s.attribute("id") {
            Some(id) => out.push_str(&format!(
                "<aside {EPUB_NS} id=\"{}\" epub:type=\"footnote\">\n{inner}</aside>\n",
                xml_escape(id)
            )),
            None => out.push_str(&format!("<div>\n{inner}</div>\n")),
        }
    }
}

/// Moves `body` into a new chapter when it has content.
fn flush(chapters: &mut Vec<Chapter>, body: &mut String, title: &str) {
    let has_content = visible_text_len(body) > 0 || body.contains("<img");
    if has_content {
        let t = if title.is_empty() {
            format!("Section {}", chapters.len() + 1)
        } else {
            title.to_string()
        };
        chapters.push(Chapter {
            title: t,
            body: std::mem::take(body),
        });
    } else {
        body.clear();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::*;

    pub fn sample() -> Vec<u8> {
        let png = base64::engine::general_purpose::STANDARD.encode(PNG);
        let jpg = base64::engine::general_purpose::STANDARD.encode(JPG);
        format!(
            r##"<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink">
<description>
 <title-info>
  <genre>sf_space</genre><genre>adventure</genre>
  <author><first-name>Ursula</first-name><middle-name>K.</middle-name><last-name>Le Guin</last-name></author>
  <author><nickname>Zed</nickname></author>
  <book-title>The Dispossessed &amp; Co</book-title>
  <annotation><p>An ambiguous utopia.</p><p>Second <emphasis>para</emphasis>.</p></annotation>
  <date value="1974-05-01">May 1974</date>
  <coverpage><image l:href="#cover.png"/></coverpage>
  <lang>en</lang>
  <sequence name="Hainish Cycle" number="5"/>
 </title-info>
 <publish-info><publisher>Harper &amp; Row</publisher><isbn>978-0-06-051275-0</isbn></publish-info>
</description>
<body>
 <title><p>The Dispossessed</p></title>
 <epigraph><p>Body epigraph</p></epigraph>
 <section id="s1">
  <title><p>Part One</p></title>
  <epigraph><p>To go is to return.</p><text-author>Odo</text-author></epigraph>
  <p>Plain <strong>bold</strong> <emphasis>italic</emphasis> <strikethrough>gone</strikethrough> H<sub>2</sub>O x<sup>2</sup> <code>code()</code> <style name="fancy">styled</style>.</p>
  <p>See note<a l:href="#n1" type="note">[1]</a> and <a l:href="#s2">chapter two</a> and <a l:href="http://example.com/x?a=1&amp;b=2">web</a>.</p>
  <empty-line/>
  <subtitle>A subtitle</subtitle>
  <poem><title><p>Ode</p></title><stanza><v>Line one</v><v>Line two</v></stanza><text-author>Poet</text-author></poem>
  <cite><p>Quoted text</p><text-author>Someone</text-author></cite>
  <image l:href="#pic.jpg" alt="A picture"/>
  <table><tr><th>H</th><td colspan="2">C</td></tr></table>
 </section>
 <section id="s2">
  <title><p>Part Two</p></title>
  <section><title><p>Two A</p></title><p>Nested A text.</p></section>
  <section><title><p>Two B</p></title><p>Nested B text.</p></section>
 </section>
</body>
<body name="notes">
 <section id="n1"><title><p>1</p></title><p>The first footnote.</p></section>
</body>
<binary id="cover.png" content-type="image/png">{png}</binary>
<binary id="pic.jpg" content-type="image/jpeg">{jpg}</binary>
</FictionBook>"##
        )
        .into_bytes()
    }

    #[test]
    fn metadata_is_extracted() {
        let arc = convert_fb2(&sample()).unwrap();
        let book = validate(&arc);
        let md = book.metadata();
        assert_eq!(md.title, "The Dispossessed & Co");
        assert_eq!(md.authors, vec!["Ursula K. Le Guin", "Zed"]);
        assert_eq!(md.tags, vec!["sf_space", "adventure"]);
        assert_eq!(md.language, "en");
        assert_eq!(md.publisher, "Harper & Row");
        assert_eq!(md.series, "Hainish Cycle");
        assert_eq!(md.series_index, 5.0);
        assert_eq!(md.published, "1974-05-01");
        assert_eq!(md.description, "An ambiguous utopia.\n\nSecond para.");
        assert_eq!(md.identifier("isbn"), Some("978-0-06-051275-0"));
    }

    #[test]
    fn cover_comes_from_the_coverpage_binary() {
        let book = validate(&convert_fb2(&sample()).unwrap());
        assert_eq!(book.cover_bytes(), Some(PNG));
    }

    #[test]
    fn sections_become_chapters_with_nested_ones_flattened() {
        let book = validate(&convert_fb2(&sample()).unwrap());
        assert_eq!(
            toc_titles(&book),
            vec![
                "The Dispossessed",
                "Part One",
                "Part Two",
                "Two A",
                "Two B",
                "Notes"
            ]
        );
        let texts = chapter_texts(&book);
        assert!(texts[0].contains("Body epigraph"));
        assert!(texts[3].contains("Nested A text."));
        assert!(!texts[3].contains("Nested B"));
        assert!(texts[4].contains("Nested B text."));
        assert_eq!(book.spine_paths().len(), 6);
    }

    #[test]
    fn contents_nest_like_the_sections() {
        let book = validate(&convert_fb2(&sample()).unwrap());
        let depths: Vec<(usize, String)> = bookx_core::toc::flatten(&book.toc())
            .into_iter()
            .map(|f| (f.depth, f.title))
            .collect();
        assert!(depths.contains(&(1, "Two A".to_string())), "{depths:?}");
        assert!(depths.contains(&(1, "Two B".to_string())), "{depths:?}");
        assert!(depths.contains(&(0, "Part Two".to_string())), "{depths:?}");
    }

    #[test]
    fn inline_styles_poems_epigraphs_and_tables_are_kept() {
        let book = validate(&convert_fb2(&sample()).unwrap());
        let x = chapter_xml(&book, 1);
        assert!(x.contains("<strong>bold</strong>"));
        assert!(x.contains("<em>italic</em>"));
        assert!(x.contains("<del>gone</del>"));
        assert!(x.contains("H<sub>2</sub>O"));
        assert!(x.contains("x<sup>2</sup>"));
        assert!(x.contains("<code>code()</code>"));
        assert!(x.contains("<span class=\"fancy\">styled</span>"));
        assert!(x.contains("<blockquote class=\"epigraph\">"));
        assert!(x.contains("class=\"text-author\">Odo<"));
        assert!(x.contains("<p class=\"subtitle\">A subtitle</p>"));
        assert!(x.contains("<div class=\"poem\">"));
        assert!(x.contains("<p class=\"v\">Line one</p>"));
        assert!(x.contains("<blockquote class=\"cite\">"));
        assert!(x.contains("<td colspan=\"2\">C</td>"));
        assert!(x.contains("<h2>Part One</h2>"));
        assert!(x.contains(">Ode</h3>"));
        assert!(x.contains("empty-line"));
    }

    #[test]
    fn images_are_embedded_as_media() {
        let arc = convert_fb2(&sample()).unwrap();
        let book = validate(&arc);
        assert_eq!(arc.get("OEBPS/images/pic.jpg"), Some(JPG));
        assert!(chapter_xml(&book, 1).contains("<img src=\"images/pic.jpg\" alt=\"A picture\"/>"));
        // the cover binary is only used as the cover, not duplicated as media
        assert!(book
            .package
            .manifest
            .iter()
            .any(|i| i.href == "images/pic.jpg"));
        assert!(!arc.contains("OEBPS/images/cover_png.png"));
    }

    #[test]
    fn notes_become_footnotes_and_links_cross_chapters() {
        let book = validate(&convert_fb2(&sample()).unwrap());
        let x = chapter_xml(&book, 1);
        assert!(
            x.contains("epub:type=\"noteref\" href=\"chapter-6.xhtml#n1\""),
            "{x}"
        );
        assert!(x.contains("href=\"chapter-3.xhtml#s2\""), "{x}");
        assert!(x.contains("href=\"http://example.com/x?a=1&amp;b=2\""));
        let notes = chapter_xml(&book, 5);
        assert!(notes.contains(
            "<aside xmlns:epub=\"http://www.idpf.org/2007/ops\" id=\"n1\" epub:type=\"footnote\">"
        ));
        assert!(notes.contains("The first footnote."));
        // links resolve inside the book
        assert_eq!(book.locate("OEBPS/chapter-6.xhtml#n1").unwrap().0, 5);
    }

    #[test]
    fn windows_1251_encoding_is_honoured() {
        let xml = "<?xml version=\"1.0\" encoding=\"windows-1251\"?><FictionBook><description><title-info><book-title>Война и мир</book-title><lang>ru</lang></title-info></description><body><section><title><p>Глава</p></title><p>Привет</p></section></body></FictionBook>";
        let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(xml);
        let book = validate(&convert_fb2(&bytes).unwrap());
        assert_eq!(book.metadata().title, "Война и мир");
        assert!(chapter_texts(&book)[0].contains("Привет"));
    }

    #[test]
    fn missing_title_and_author_use_defaults() {
        let xml = b"<FictionBook><description/><body><section><p>Only text</p></section></body></FictionBook>";
        let book = validate(&convert_fb2(xml).unwrap());
        assert_eq!(book.metadata().title, "Untitled");
        assert!(book.metadata().authors.is_empty());
        assert_eq!(toc_titles(&book), vec!["Section 1"]);
    }

    #[test]
    fn unknown_binaries_and_broken_base64_are_ignored() {
        let xml = b"<FictionBook xmlns:l='http://www.w3.org/1999/xlink'><body><section><title><p>T</p></title><p>x</p><image l:href='#bad'/><image l:href='#nothing'/></section></body><binary id='bad' content-type='image/png'>!!!notbase64***</binary></FictionBook>";
        let arc = convert_fb2(xml).unwrap();
        let book = validate(&arc);
        assert!(!chapter_xml(&book, 0).contains("<img"));
    }

    #[test]
    fn errors_are_reported_for_bad_input() {
        assert!(convert_fb2(b"").unwrap_err().to_string().contains("empty"));
        assert!(convert_fb2(b"<html/>")
            .unwrap_err()
            .to_string()
            .contains("not a FictionBook"));
        assert!(convert_fb2(b"<FictionBook><body></body></FictionBook>")
            .unwrap_err()
            .to_string()
            .contains("no text"));
        assert!(convert_fb2(b"<FictionBook><body>").is_err());
    }

    #[test]
    fn deeply_nested_sections_do_not_overflow_the_stack() {
        let mut xml = String::from("<FictionBook><body>");
        for _ in 0..3000 {
            xml.push_str("<section>");
        }
        xml.push_str("<p>deep</p>");
        for _ in 0..3000 {
            xml.push_str("</section>");
        }
        xml.push_str("</body></FictionBook>");
        let _ = convert_fb2(xml.as_bytes());
    }

    #[test]
    fn fbz_reads_the_zipped_fb2() {
        let z = make_zip(&[("book.fb2", &sample()), ("readme.txt", b"x")]);
        let book = validate(&FbzReader.read(&z).unwrap());
        assert_eq!(book.metadata().title, "The Dispossessed & Co");
        assert!(FbzReader.sniff(&z));
        assert!(
            FbzReader.sniff(&z[..60]),
            "sniff works from the first local header"
        );
        assert!(!FbzReader.sniff(&make_zip(&[("a.txt", b"x")])));
        let empty = make_zip(&[("a.txt", b"x")]);
        assert!(FbzReader
            .read(&empty)
            .unwrap_err()
            .to_string()
            .contains("no .fb2"));
        assert!(FbzReader.read(b"not a zip").is_err());
    }

    #[test]
    fn sniffing() {
        assert!(Fb2Reader.sniff(&sample()));
        assert!(Fb2Reader.sniff(b"\xEF\xBB\xBF<?xml version='1.0'?>\n<FictionBook>"));
        assert!(!Fb2Reader.sniff(b"<html><body>"));
        assert!(!Fb2Reader.sniff(b""));
        assert_eq!(Fb2Reader.id(), "fb2");
        assert_eq!(Fb2Reader.extensions(), &["fb2"]);
        assert_eq!(FbzReader.id(), "fbz");
        assert_eq!(FbzReader.extensions(), &["fbz"]);
    }

    #[test]
    fn robust_against_garbage_and_mutations() {
        assert_rejects_garbage(&Fb2Reader);
        assert_rejects_garbage(&FbzReader);
        let errors = fuzz_no_panic(&Fb2Reader, &sample());
        assert!(errors > 20);
        let z = make_zip(&[("book.fb2", &sample())]);
        fuzz_no_panic(&FbzReader, &z);
    }
}
