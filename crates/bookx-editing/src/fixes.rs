//! Automatic fixes for the problems `check_book` reports (E17/E18). Every fix says exactly what it changed and a
//! second run changes nothing.

use std::collections::HashSet;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::check::{Severity, check_book};
use bookx_core::util::{
    extension, file_name, is_external, media_type_for, parse_xml, relative_href, xml_escape,
};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::markup::*;
use crate::opf_edit::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FixKind {
    Mimetype,
    Container,
    ManifestAdded,
    ManifestRemoved,
    DuplicateManifestId,
    DuplicateElementId,
    MissingAlt,
    WellFormedness,
    Metadata,
    UniqueIdentifier,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fix {
    pub kind: FixKind,
    pub file: String,
    pub description: String,
}

fn fix(kind: FixKind, file: &str, description: impl Into<String>) -> Fix {
    Fix {
        kind,
        file: file.to_string(),
        description: description.into(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixReport {
    pub fixes: Vec<Fix>,
    /// Files that could not be repaired, with the reason.
    pub failures: Vec<String>,
    /// Error-level problems `check_book` still reports after fixing.
    pub remaining_errors: Vec<String>,
    /// Validator rule ids (see [`crate::validate`]) that were reported before fixing and no longer are.
    #[serde(default)]
    pub resolved_validator_ids: Vec<String>,
    /// Validator rule ids with severity `ERROR` that are still reported after fixing.
    #[serde(default)]
    pub remaining_validator_ids: Vec<String>,
}

impl FixReport {
    pub fn is_empty(&self) -> bool {
        self.fixes.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct FixOptions {
    pub default_language: String,
    pub default_title: String,
    /// Identifier value for books without one (default: a random `urn:uuid:`).
    pub identifier: Option<String>,
}

impl Default for FixOptions {
    fn default() -> Self {
        Self {
            default_language: "en".into(),
            default_title: "Untitled".into(),
            identifier: None,
        }
    }
}

// ---------------------------------------------------------------------------
// archive level
// ---------------------------------------------------------------------------

/// The `mimetype` file must contain exactly `application/epub+zip`.
pub fn fix_mimetype(archive: &mut Archive) -> Option<Fix> {
    if archive.get("mimetype") == Some(b"application/epub+zip".as_slice()) {
        return None;
    }
    let had = archive.contains("mimetype");
    archive.set("mimetype", b"application/epub+zip".to_vec());
    Some(fix(
        FixKind::Mimetype,
        "mimetype",
        if had {
            "Rewrote mimetype with the exact content application/epub+zip"
        } else {
            "Added the missing mimetype file"
        },
    ))
}

const CONTAINER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n  <rootfiles>\n    <rootfile full-path=\"{PATH}\" media-type=\"application/oebps-package+xml\"/>\n  </rootfiles>\n</container>\n";

/// Recreates `META-INF/container.xml` when it is missing, broken or points at a file that does not exist.
pub fn fix_container(archive: &mut Archive) -> Result<Option<Fix>> {
    const FILE: &str = "META-INF/container.xml";
    let current = archive.get_string(FILE);
    let valid = current.as_deref().is_some_and(|t| {
        parse_xml(t).is_ok_and(|d| {
            d.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "rootfile")
                .and_then(|n| n.attribute("full-path"))
                .is_some_and(|p| archive.contains(p))
        })
    });
    if valid {
        return Ok(None);
    }
    let mut opfs: Vec<&String> = archive
        .names()
        .iter()
        .filter(|n| extension(n) == "opf")
        .collect();
    opfs.sort_by_key(|n| (n.matches('/').count(), n.to_string()));
    let Some(opf) = opfs.first() else {
        bail!("cannot repair container.xml: the archive contains no .opf package file");
    };
    let text = CONTAINER.replace("{PATH}", &xml_escape(opf));
    let desc = format!(
        "{} META-INF/container.xml pointing at {opf}",
        if current.is_some() {
            "Rewrote"
        } else {
            "Created"
        }
    );
    archive.set_string(FILE, &text);
    Ok(Some(fix(FixKind::Container, FILE, desc)))
}

// ---------------------------------------------------------------------------
// package level
// ---------------------------------------------------------------------------

fn sanitize_id(stem: &str) -> String {
    let mut id: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if id.is_empty() || !id.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        id.insert(0, 'f');
    }
    id
}

fn is_junk(name: &str) -> bool {
    let f = file_name(name);
    f.starts_with('.') || name.starts_with("__MACOSX/") || f == "Thumbs.db"
}

/// Adds every file that is in the archive but not in the manifest.
pub fn add_missing_manifest_entries(book: &mut OpenBook) -> Result<Vec<Fix>> {
    let pkg = book.package.clone();
    let mut ids: HashSet<String> = pkg.manifest.iter().map(|i| i.id.clone()).collect();
    let mut lines = String::new();
    let mut fixes = Vec::new();
    for name in book.archive.names().to_vec() {
        if name == "mimetype"
            || name.starts_with("META-INF/")
            || name == pkg.opf_path
            || is_junk(&name)
            || pkg.item_by_path(&name).is_some()
        {
            continue;
        }
        let stem = file_name(&name)
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(file_name(&name));
        let base = sanitize_id(stem);
        let mut id = base.clone();
        let mut n = 1;
        while ids.contains(&id) {
            n += 1;
            id = format!("{base}-{n}");
        }
        ids.insert(id.clone());
        let media = media_type_for(&name);
        lines.push_str(&format!(
            "<item id=\"{}\" href=\"{}\" media-type=\"{}\"/>\n",
            xml_escape(&id),
            xml_escape(&relative_href(&pkg.opf_path, &name)),
            media
        ));
        fixes.push((name, id, media));
    }
    if fixes.is_empty() {
        return Ok(Vec::new());
    }
    edit_opf(book, |t| {
        let mut t = t.to_string();
        for line in lines.lines() {
            t = append_to_section(&t, "manifest", line)?;
        }
        Ok(t)
    })?;
    Ok(fixes
        .into_iter()
        .map(|(name, id, media)| {
            fix(
                FixKind::ManifestAdded,
                &name,
                format!("Added to the manifest as \"{id}\" ({media})"),
            )
        })
        .collect())
}

/// Removes manifest items (and their spine entries) whose file does not exist.
pub fn remove_missing_manifest_items(book: &mut OpenBook) -> Result<Vec<Fix>> {
    let missing: Vec<_> = book
        .package
        .manifest
        .iter()
        .filter(|i| !is_external(&i.href) && !book.archive.contains(&book.package.item_path(i)))
        .cloned()
        .collect();
    let mut fixes = Vec::new();
    for item in missing {
        let pkg = book.package.clone();
        pkg.remove_manifest_item(&mut book.archive, &item.id)?;
        book.reload_package()?;
        if pkg.toc_id.as_deref() == Some(item.id.as_str()) {
            edit_opf(book, |t| {
                let mut out = String::new();
                let mut last = 0;
                for ev in scan(t) {
                    if let Event::Open { name, range, .. } = ev {
                        if name == "spine" {
                            out.push_str(&t[last..range.start]);
                            out.push_str(&remove_tag_attr(&t[range.clone()], "toc"));
                            last = range.end;
                            break;
                        }
                    }
                }
                out.push_str(&t[last..]);
                Ok(out)
            })?;
        }
        fixes.push(fix(
            FixKind::ManifestRemoved,
            &pkg.opf_path,
            format!(
                "Removed manifest item \"{}\" ({}) because the file does not exist",
                item.id, item.href
            ),
        ));
    }
    Ok(fixes)
}

/// Gives later manifest items with a repeated id a new unique id (the first keeps it).
pub fn fix_duplicate_manifest_ids(book: &mut OpenBook) -> Result<Vec<Fix>> {
    let mut renamed: Vec<(String, String)> = Vec::new();
    let opf = book.package.opf_path.clone();
    edit_opf(book, |t| {
        let all: HashSet<String> = scan(t)
            .into_iter()
            .filter_map(|e| match e {
                Event::Open { range, .. } => tag_attr(&t[range], "id"),
                _ => None,
            })
            .collect();
        let mut taken = all.clone();
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = String::with_capacity(t.len());
        let mut last = 0;
        for ev in scan(t) {
            if let Event::Open { name, range, .. } = ev {
                if name != "item" {
                    continue;
                }
                let tag = &t[range.clone()];
                let Some(id) = tag_attr(tag, "id") else {
                    continue;
                };
                if seen.insert(id.clone()) {
                    continue;
                }
                let mut n = 2;
                let mut cand = format!("{id}-{n}");
                while taken.contains(&cand) {
                    n += 1;
                    cand = format!("{id}-{n}");
                }
                taken.insert(cand.clone());
                seen.insert(cand.clone());
                out.push_str(&t[last..range.start]);
                out.push_str(&set_tag_attr(tag, "id", &cand));
                last = range.end;
                renamed.push((id, cand));
            }
        }
        out.push_str(&t[last..]);
        Ok(out)
    })?;
    Ok(renamed
        .into_iter()
        .map(|(a, b)| {
            fix(
                FixKind::DuplicateManifestId,
                &opf,
                format!("Renamed duplicate manifest id \"{a}\" to \"{b}\""),
            )
        })
        .collect())
}

/// Renames repeated `id` attributes inside XHTML files (the first occurrence keeps its id, so existing links still
/// resolve).
pub fn fix_duplicate_element_ids(book: &mut OpenBook) -> Vec<Fix> {
    let mut fixes = Vec::new();
    for file in markup_files(&book.archive) {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        let all: HashSet<String> = defined_ids(&src).into_iter().collect();
        let mut taken = all.clone();
        let mut seen: HashSet<String> = HashSet::new();
        let mut log: Vec<(String, String)> = Vec::new();
        let new = rewrite_all_tags(&src, &mut |_, attr, val| {
            if attr != "id" {
                return None;
            }
            if seen.insert(val.to_string()) {
                return None;
            }
            let mut n = 2;
            let mut cand = format!("{val}-{n}");
            while taken.contains(&cand) {
                n += 1;
                cand = format!("{val}-{n}");
            }
            taken.insert(cand.clone());
            seen.insert(cand.clone());
            log.push((val.to_string(), cand.clone()));
            Some(cand)
        });
        if new != src {
            book.archive.set_string(&file, &new);
            for (a, b) in log {
                fixes.push(fix(
                    FixKind::DuplicateElementId,
                    &file,
                    format!("Renamed duplicate id \"{a}\" to \"{b}\""),
                ));
            }
        }
    }
    fixes
}

/// Adds `alt=""` to images without an `alt` attribute (they are then treated as decorative).
pub fn fix_missing_alt(book: &mut OpenBook) -> Vec<Fix> {
    let mut fixes = Vec::new();
    for file in markup_files(&book.archive) {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        let mut count = 0;
        let mut out = String::with_capacity(src.len());
        let mut last = 0;
        for ev in scan(&src) {
            if let Event::Open { name, range, .. } = ev {
                if name == "img" && tag_attr(&src[range.clone()], "alt").is_none() {
                    out.push_str(&src[last..range.start]);
                    out.push_str(&set_tag_attr(&src[range.clone()], "alt", ""));
                    last = range.end;
                    count += 1;
                }
            }
        }
        if count > 0 {
            out.push_str(&src[last..]);
            book.archive.set_string(&file, &out);
            fixes.push(fix(
                FixKind::MissingAlt,
                &file,
                format!("Added an empty alt attribute to {count} image(s)"),
            ));
        }
    }
    fixes
}

// ---------------------------------------------------------------------------
// well-formedness
// ---------------------------------------------------------------------------

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

const HTML_ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "bdi",
    "bdo",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "meta",
    "nav",
    "object",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "section",
    "select",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
    "center",
    "font",
    "tt",
    "big",
    "strike",
    "acronym",
    "dir",
    "basefont",
];

fn fix_name(name: &str) -> String {
    if !name.contains(':') {
        let lower = name.to_ascii_lowercase();
        if lower != name && HTML_ELEMENTS.contains(&lower.as_str()) {
            return lower;
        }
    }
    name.to_string()
}

fn amp_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"&(#[0-9]+;|#[xX][0-9A-Fa-f]+;|[A-Za-z][A-Za-z0-9]*;)?").unwrap())
}

/// Escapes stray `&` and replaces HTML-only named entities by numeric references.
fn fix_ampersands(s: &str) -> String {
    amp_re()
        .replace_all(s, |c: &regex::Captures| match c.get(1) {
            None => "&amp;".to_string(),
            Some(m) => {
                let body = m.as_str();
                if body.starts_with('#')
                    || matches!(body, "amp;" | "lt;" | "gt;" | "quot;" | "apos;")
                {
                    format!("&{body}")
                } else {
                    let decoded = decode_entities(&format!("&{body}"));
                    match decoded.chars().next() {
                        Some(ch) if decoded.chars().count() == 1 && ch != '&' => {
                            format!("&#{};", ch as u32)
                        }
                        _ => format!("&amp;{body}"),
                    }
                }
            }
        })
        .into_owned()
}

fn fix_text(s: &str) -> String {
    fix_ampersands(s).replace('<', "&lt;")
}

fn attr_tok_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"([^\s=/>"']+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?"#).unwrap()
    })
}

/// Rebuilds a start tag with quoted attributes, no duplicates and escaped values.
fn normalize_start_tag(qname: &str, inner_attrs: &str, void: bool, self_closing: bool) -> String {
    let mut out = format!("<{}", fix_name(qname));
    let mut seen: HashSet<String> = HashSet::new();
    for c in attr_tok_re().captures_iter(inner_attrs) {
        let mut name = c[1].to_string();
        if name == name.to_ascii_uppercase() && !name.contains(':') {
            name = name.to_ascii_lowercase();
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        let val = c
            .get(2)
            .or_else(|| c.get(3))
            .or_else(|| c.get(4))
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| name.clone());
        let val = fix_ampersands(&val)
            .replace('<', "&lt;")
            .replace('"', "&quot;");
        out.push_str(&format!(" {name}=\"{val}\""));
    }
    out.push_str(if void || self_closing { "/>" } else { ">" });
    out
}

/// Best-effort conversion of sloppy HTML-in-XHTML into well-formed XML: closes unclosed and void elements, drops
/// stray end tags, quotes attributes, escapes `&` and `<`, replaces HTML entities by numeric references, lowercases
/// HTML element names and declares missing `xhtml`/`epub`/`xlink` namespaces on the root. Returns `None` when the
/// result still does not parse.
pub fn repair_xhtml(src: &str) -> Option<String> {
    let mut out = String::with_capacity(src.len() + 64);
    let mut stack: Vec<String> = Vec::new();
    let mut raw_text_of: Option<String> = None;
    for ev in scan(src) {
        match ev {
            Event::Text(r) => {
                let t = &src[r];
                if raw_text_of.is_some() {
                    if t.contains(['<', '&']) && !t.contains("CDATA") {
                        out.push_str(&format!("/*<![CDATA[*/{t}/*]]>*/"));
                    } else {
                        out.push_str(t);
                    }
                } else {
                    out.push_str(&fix_text(t));
                }
            }
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                let tag = &src[range];
                let qname = tag_qname(tag);
                let attrs_start = 1 + qname.len();
                let inner = tag[attrs_start..]
                    .trim_end_matches('>')
                    .trim_end_matches('/');
                let local = name.as_str();
                let void = VOID.contains(&local);
                let mut s = normalize_start_tag(qname, inner, void, self_closing);
                if stack.is_empty() && local == "html" {
                    s = add_root_namespaces(&s, src);
                }
                out.push_str(&s);
                if !void && !self_closing {
                    stack.push(fix_name(qname));
                    if local == "script" || local == "style" {
                        raw_text_of = Some(local.to_string());
                    }
                }
            }
            Event::Close { name, range } => {
                let qname = fix_name(tag_qname(&src[range]));
                if VOID.contains(&name.as_str()) {
                    continue;
                }
                if raw_text_of.as_deref() == Some(name.as_str()) {
                    raw_text_of = None;
                }
                if let Some(pos) = stack.iter().rposition(|n| *n == qname) {
                    while stack.len() > pos + 1 {
                        let n = stack.pop().unwrap();
                        out.push_str(&format!("</{n}>"));
                    }
                    stack.pop();
                    out.push_str(&format!("</{qname}>"));
                }
            }
            Event::Other(r) => {
                let t = &src[r];
                if let Some(body) = t.strip_prefix("<!--") {
                    let body = body.strip_suffix("-->").unwrap_or(body);
                    let mut b = body.replace("--", "- -");
                    if b.ends_with('-') {
                        b.push(' ');
                    }
                    out.push_str(&format!("<!--{b}-->"));
                } else {
                    out.push_str(t);
                }
            }
        }
    }
    while let Some(n) = stack.pop() {
        out.push_str(&format!("</{n}>"));
    }
    parse_xml(&out).is_ok().then_some(out)
}

fn add_root_namespaces(tag: &str, src: &str) -> String {
    let mut t = tag.to_string();
    if tag_attr(&t, "xmlns").is_none() {
        t = set_tag_attr(&t, "xmlns", "http://www.w3.org/1999/xhtml");
    }
    if src.contains("epub:") && tag_attr(&t, "xmlns:epub").is_none() {
        t = set_tag_attr(&t, "xmlns:epub", "http://www.idpf.org/2007/ops");
    }
    if src.contains("xlink:") && tag_attr(&t, "xmlns:xlink").is_none() {
        t = set_tag_attr(&t, "xmlns:xlink", "http://www.w3.org/1999/xlink");
    }
    t
}

/// Adds the XHTML namespace to the root `<html>` element when it is missing (well-formed files only).
pub fn fix_missing_namespace(src: &str) -> Option<String> {
    for ev in scan(src) {
        if let Event::Open { name, range, .. } = ev {
            if name == "html" {
                let tag = &src[range.clone()];
                if tag_attr(tag, "xmlns").is_some() {
                    return None;
                }
                let new = set_tag_attr(tag, "xmlns", "http://www.w3.org/1999/xhtml");
                return Some(format!(
                    "{}{}{}",
                    &src[..range.start],
                    new,
                    &src[range.end..]
                ));
            }
        }
    }
    None
}

const HTML_NS: &str = "http://www.w3.org/1999/xhtml";

fn html5_escape(s: &str, attr: bool) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' if attr => o.push_str("&quot;"),
            '\u{0}' => {}
            c => o.push(c),
        }
    }
    o
}

fn html5_write(node: ego_tree::NodeRef<'_, scraper::Node>, parent_ns: &str, out: &mut String) {
    use scraper::Node;
    for child in node.children() {
        match child.value() {
            Node::Text(t) => out.push_str(&html5_escape(t, false)),
            Node::Comment(c) => {
                let body = c.replace("--", "- -");
                out.push_str("<!--");
                out.push_str(&body);
                if body.ends_with('-') {
                    out.push(' ');
                }
                out.push_str("-->");
            }
            Node::Element(el) => {
                let name = el.name();
                let ns: &str = &el.name.ns;
                out.push('<');
                out.push_str(name);
                if ns != parent_ns && !ns.is_empty() {
                    out.push_str(&format!(" xmlns=\"{}\"", html5_escape(ns, true)));
                }
                for (qn, v) in el.attrs.iter() {
                    let local: &str = &qn.local;
                    let key = match &qn.prefix {
                        Some(p) if !p.is_empty() && local != "xmlns" => format!("{p}:{local}"),
                        _ => local.to_string(),
                    };
                    if key == "xmlns" {
                        continue;
                    }
                    out.push_str(&format!(" {key}=\"{}\"", html5_escape(v, true)));
                }
                let void = ns == HTML_NS && VOID.contains(&name);
                if void {
                    out.push_str("/>");
                    continue;
                }
                out.push('>');
                let raw = ns == HTML_NS && (name == "script" || name == "style");
                if raw {
                    let text: String = child
                        .children()
                        .filter_map(|c| c.value().as_text().map(|t| t.to_string()))
                        .collect();
                    if text.contains(['<', '&']) {
                        out.push_str(&format!("/*<![CDATA[*/{text}/*]]>*/"));
                    } else {
                        out.push_str(&text);
                    }
                } else {
                    html5_write(child, ns, out);
                }
                out.push_str(&format!("</{name}>"));
            }
            _ => {}
        }
    }
}

/// Parses `src` the way a browser does (HTML5 tree building: missing `html`/`head`/`body` are made, misnested and
/// unclosed tags are resolved) and writes the tree as XHTML. Used when [`repair_xhtml`] cannot cope. `None` when the
/// result is not well-formed XML.
pub fn html5_to_xhtml(src: &str) -> Option<String> {
    // an XML declaration is not HTML: a browser would turn it into a comment
    let trimmed = src.trim_start_matches('\u{feff}').trim_start();
    let src = match trimmed.strip_prefix("<?xml").and_then(|r| r.find("?>")) {
        Some(end) => &trimmed[5 + end + 2..],
        None => src,
    };
    let doc = bookx_core::util::parse_html_document(src.trim_end());
    let mut body = String::new();
    html5_write(doc.tree.root(), "", &mut body);
    // the root needs the prefixes the document uses
    let mut decl = String::new();
    if body.contains(" epub:") && !body.contains("xmlns:epub") {
        decl.push_str(" xmlns:epub=\"http://www.idpf.org/2007/ops\"");
    }
    if body.contains(" xlink:") && !body.contains("xmlns:xlink") {
        decl.push_str(" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
    }
    {
        let at = body.find("<html")?;
        let end = at + body[at..].find('>')?;
        let tag = &body[at..end];
        let mut tag = tag.to_string();
        if !tag.contains("xmlns=\"") {
            tag.push_str(&format!(" xmlns=\"{HTML_NS}\""));
        }
        tag.push_str(&decl);
        body = format!("{}{}{}", &body[..at], tag, &body[end..]);
    }
    let out = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n{body}\n");
    parse_xml(&out).is_ok().then_some(out)
}

/// Fixes the markup of the given files (or of every page when `paths` is empty): a file that already parses is only
/// given a missing namespace; the others go through [`repair_xhtml`], and through the HTML5 parser when that fails or
/// when `html5_only` is set. Returns what changed and what could not be repaired.
pub fn fix_html_files(
    book: &mut OpenBook,
    paths: &[String],
    html5_only: bool,
) -> (Vec<Fix>, Vec<String>) {
    let mut fixes = Vec::new();
    let mut failures = Vec::new();
    let files: Vec<String> = markup_files(&book.archive)
        .into_iter()
        .filter(|f| matches!(extension(f).as_str(), "xhtml" | "xht" | "html" | "htm"))
        .filter(|f| paths.is_empty() || paths.contains(f))
        .collect();
    for file in files {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        match parse_xml(&src) {
            Ok(_) => {
                if let Some(new) = fix_missing_namespace(&src) {
                    if parse_xml(&new).is_ok() {
                        book.archive.set_string(&file, &new);
                        fixes.push(fix(
                            FixKind::WellFormedness,
                            &file,
                            "Added the missing XHTML namespace to <html>",
                        ));
                    }
                }
            }
            Err(e) => {
                let repaired = if html5_only {
                    None
                } else {
                    repair_xhtml(&src).map(|n| (n, "Repaired markup"))
                };
                let repaired = repaired
                    .or_else(|| html5_to_xhtml(&src).map(|n| (n, "Rebuilt with the HTML5 parser")));
                match repaired {
                    Some((new, how)) => {
                        book.archive.set_string(&file, &new);
                        fixes.push(fix(FixKind::WellFormedness, &file, format!("{how} ({e})")));
                    }
                    None => failures.push(format!("{file}: cannot repair automatically ({e})")),
                }
            }
        }
    }
    (fixes, failures)
}

/// Repairs XHTML files that do not parse, and adds a missing XHTML namespace to those that do.
pub fn fix_wellformedness(book: &mut OpenBook) -> (Vec<Fix>, Vec<String>) {
    let mut fixes = Vec::new();
    let mut failures = Vec::new();
    for file in markup_files(&book.archive) {
        if !matches!(extension(&file).as_str(), "xhtml" | "xht" | "html" | "htm") {
            continue;
        }
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        match parse_xml(&src) {
            Ok(_) => {
                if let Some(new) = fix_missing_namespace(&src) {
                    if parse_xml(&new).is_ok() {
                        book.archive.set_string(&file, &new);
                        fixes.push(fix(
                            FixKind::WellFormedness,
                            &file,
                            "Added the missing XHTML namespace to <html>",
                        ));
                    }
                }
            }
            Err(e) => match repair_xhtml(&src) {
                Some(new) => {
                    book.archive.set_string(&file, &new);
                    fixes.push(fix(
                        FixKind::WellFormedness,
                        &file,
                        format!("Repaired markup ({e})"),
                    ));
                }
                None => match html5_to_xhtml(&src) {
                    Some(new) => {
                        book.archive.set_string(&file, &new);
                        fixes.push(fix(
                            FixKind::WellFormedness,
                            &file,
                            format!("Rebuilt with the HTML5 parser ({e})"),
                        ));
                    }
                    None => failures.push(format!("{file}: cannot repair automatically ({e})")),
                },
            },
        }
    }
    (fixes, failures)
}

// ---------------------------------------------------------------------------
// metadata and package attributes
// ---------------------------------------------------------------------------

fn remove_empty_element(text: &str, local: &str) -> String {
    let re = Regex::new(&format!(
        r"[ \t]*<(?:[A-Za-z0-9_]+:)?{l}(?:\s[^>]*)?(?:/>|>\s*</(?:[A-Za-z0-9_]+:)?{l}\s*>)[ \t]*\r?\n?",
        l = regex::escape(local)
    ))
    .unwrap();
    re.replace_all(text, "").into_owned()
}

/// Adds a missing title, language or identifier (and repairs `unique-identifier`).
pub fn fix_metadata(book: &mut OpenBook, opts: &FixOptions) -> Result<Vec<Fix>> {
    let opf = book.package.opf_path.clone();
    let md = book.package.metadata.clone();
    let mut fixes = Vec::new();
    if md.title.trim().is_empty() {
        let t = opts.default_title.clone();
        edit_opf(book, |text| {
            add_dc_element(&remove_empty_element(text, "title"), "title", "", &t)
        })?;
        fixes.push(fix(
            FixKind::Metadata,
            &opf,
            format!("Added dc:title \"{t}\""),
        ));
    }
    if md.language.trim().is_empty() {
        let l = opts.default_language.clone();
        edit_opf(book, |text| {
            add_dc_element(&remove_empty_element(text, "language"), "language", "", &l)
        })?;
        fixes.push(fix(
            FixKind::Metadata,
            &opf,
            format!("Added dc:language \"{l}\""),
        ));
    }
    if md.identifiers.is_empty() {
        let value = opts
            .identifier
            .clone()
            .unwrap_or_else(|| format!("urn:uuid:{}", uuid::Uuid::new_v4()));
        let v = value.clone();
        edit_opf(book, |text| {
            let id = unused_id(text, "bookid");
            add_dc_element(
                &remove_empty_element(text, "identifier"),
                "identifier",
                &format!(" id=\"{id}\""),
                &v,
            )
        })?;
        fixes.push(fix(
            FixKind::Metadata,
            &opf,
            format!("Added dc:identifier \"{value}\""),
        ));
    }
    fixes.extend(fix_unique_identifier(book)?);
    Ok(fixes)
}

fn unused_id(text: &str, base: &str) -> String {
    let ids: HashSet<String> = scan(text)
        .into_iter()
        .filter_map(|e| match e {
            Event::Open { range, .. } => tag_attr(&text[range], "id"),
            _ => None,
        })
        .collect();
    let mut id = base.to_string();
    let mut n = 1;
    while ids.contains(&id) {
        n += 1;
        id = format!("{base}-{n}");
    }
    id
}

/// Makes `<package unique-identifier>` refer to an existing `dc:identifier`.
pub fn fix_unique_identifier(book: &mut OpenBook) -> Result<Vec<Fix>> {
    let pkg = &book.package;
    if pkg.metadata.identifiers.is_empty() {
        return Ok(Vec::new());
    }
    let ok = !pkg.unique_id.is_empty()
        && pkg
            .metadata
            .identifiers
            .iter()
            .any(|i| i.xml_id.as_deref() == Some(pkg.unique_id.as_str()));
    if ok {
        return Ok(Vec::new());
    }
    let opf = pkg.opf_path.clone();
    let existing = pkg
        .metadata
        .identifiers
        .iter()
        .find_map(|i| i.xml_id.clone());
    let mut chosen = existing.clone().unwrap_or_default();
    edit_opf(book, |text| {
        let mut t = text.to_string();
        if existing.is_none() {
            chosen = unused_id(text, "bookid");
            let mut out = String::new();
            let mut last = 0;
            for ev in scan(text) {
                if let Event::Open { name, range, .. } = ev {
                    if name == "identifier" {
                        out.push_str(&text[last..range.start]);
                        out.push_str(&set_tag_attr(&text[range.clone()], "id", &chosen));
                        last = range.end;
                        break;
                    }
                }
            }
            out.push_str(&text[last..]);
            t = out;
        }
        Ok(set_element_attr(
            &t,
            "package",
            "unique-identifier",
            &chosen,
        ))
    })?;
    Ok(vec![fix(
        FixKind::UniqueIdentifier,
        &opf,
        format!("Set unique-identifier to \"{chosen}\""),
    )])
}

/// Makes sure `<package>` has a valid `version` and, for EPUB 2, that `<spine toc>` points at the NCX.
pub fn fix_versions(book: &mut OpenBook) -> Result<Vec<Fix>> {
    let opf = book.package.opf_path.clone();
    let mut fixes = Vec::new();
    let text = book.archive.get_string(&opf).unwrap_or_default();
    let declared = scan(&text)
        .into_iter()
        .find_map(|e| match e {
            Event::Open { name, range, .. } if name == "package" => Some((
                tag_attr(&text[range.clone()], "version"),
                tag_attr(&text[range], "xmlns"),
            )),
            _ => None,
        })
        .unwrap_or((None, None));
    let valid = |v: &str| matches!(v, "2.0" | "3.0" | "3.1" | "3.2");
    if declared.0.as_deref().is_none_or(|v| !valid(v)) {
        let v = if book.package.nav_item().is_some() {
            "3.0"
        } else {
            "2.0"
        };
        edit_opf(book, |t| Ok(set_element_attr(t, "package", "version", v)))?;
        fixes.push(fix(
            FixKind::Version,
            &opf,
            format!(
                "Set package version to {v} (was {})",
                declared
                    .0
                    .as_deref()
                    .map(|s| format!("\"{s}\""))
                    .unwrap_or_else(|| "missing".into())
            ),
        ));
    }
    if declared.1.is_none() {
        edit_opf(book, |t| {
            Ok(set_element_attr(
                t,
                "package",
                "xmlns",
                bookx_core::opf::NS_OPF,
            ))
        })?;
        fixes.push(fix(
            FixKind::Version,
            &opf,
            "Declared the OPF namespace on <package>",
        ));
    }
    if !book.package.is_epub3() && book.package.toc_id.is_none() {
        if let Some(ncx) = book
            .package
            .manifest
            .iter()
            .find(|i| i.media_type == "application/x-dtbncx+xml")
            .cloned()
        {
            edit_opf(book, |t| Ok(set_element_attr(t, "spine", "toc", &ncx.id)))?;
            fixes.push(fix(
                FixKind::Version,
                &opf,
                format!("Set <spine toc> to the NCX item \"{}\"", ncx.id),
            ));
        }
    }
    Ok(fixes)
}

// ---------------------------------------------------------------------------
// everything
// ---------------------------------------------------------------------------

/// Fixes the archive-level problems that prevent the package from opening, then everything else.
/// Returns the opened book and what was done.
pub fn fix_archive(mut archive: Archive, opts: &FixOptions) -> Result<(OpenBook, FixReport)> {
    let mut report = FixReport::default();
    let before = crate::validate::validate(&archive).ids();
    report.fixes.extend(fix_mimetype(&mut archive));
    report.fixes.extend(fix_container(&mut archive)?);
    let mut book =
        OpenBook::from_archive(archive).context("the package document is too damaged to open")?;
    fix_book_into(&mut book, opts, &mut report)?;
    record_validator_ids(&before, &book.archive, &mut report);
    Ok((book, report))
}

/// Runs every fixer on an open book.
pub fn fix_book(book: &mut OpenBook, opts: &FixOptions) -> Result<FixReport> {
    let mut report = FixReport::default();
    let before = crate::validate::validate(&book.archive).ids();
    report.fixes.extend(fix_mimetype(&mut book.archive));
    report.fixes.extend(fix_container(&mut book.archive)?);
    fix_book_into(book, opts, &mut report)?;
    record_validator_ids(&before, &book.archive, &mut report);
    Ok(report)
}

/// Compares the validator ids found before fixing with those found now.
fn record_validator_ids(before: &[String], archive: &Archive, report: &mut FixReport) {
    let after = crate::validate::validate(archive);
    let still: Vec<String> = after.ids();
    report.resolved_validator_ids = before
        .iter()
        .filter(|id| !still.contains(id))
        .cloned()
        .collect();
    report.remaining_validator_ids = after.ids_with(crate::validate::Severity::Error);
}

fn fix_book_into(book: &mut OpenBook, opts: &FixOptions, report: &mut FixReport) -> Result<()> {
    report.fixes.extend(fix_versions(book)?);
    report.fixes.extend(remove_missing_manifest_items(book)?);
    report.fixes.extend(fix_duplicate_manifest_ids(book)?);
    report.fixes.extend(add_missing_manifest_entries(book)?);
    report.fixes.extend(fix_metadata(book, opts)?);
    let (f, failures) = fix_wellformedness(book);
    report.fixes.extend(f);
    report.failures.extend(failures);
    report.fixes.extend(fix_duplicate_element_ids(book));
    report.fixes.extend(fix_missing_alt(book));
    report.remaining_errors = check_book(&book.archive)
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| format!("{}: {}", i.file, i.message))
        .collect();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;

    #[test]
    fn mimetype_fixed_once() {
        let mut a = Archive::default();
        let f = fix_mimetype(&mut a).unwrap();
        assert_eq!(f.description, "Added the missing mimetype file");
        assert_eq!(a.get("mimetype").unwrap(), b"application/epub+zip");
        assert!(fix_mimetype(&mut a).is_none());
        a.set("mimetype", b"application/epub+zip\n".to_vec());
        assert!(
            fix_mimetype(&mut a)
                .unwrap()
                .description
                .starts_with("Rewrote")
        );
        assert!(fix_mimetype(&mut a).is_none());
    }

    #[test]
    fn container_recreated_and_repointed() {
        let mut a = Archive::default();
        assert!(
            fix_container(&mut a)
                .unwrap_err()
                .to_string()
                .contains("no .opf")
        );
        a.set_string("x/deep/b.opf", "<package/>");
        a.set_string("content.opf", "<package/>");
        let f = fix_container(&mut a).unwrap().unwrap();
        assert!(f.description.starts_with("Created"));
        assert_eq!(bookx_core::opf::find_opf_path(&a).unwrap(), "content.opf");
        assert!(fix_container(&mut a).unwrap().is_none());
        a.remove("content.opf");
        let f = fix_container(&mut a).unwrap().unwrap();
        assert!(f.description.starts_with("Rewrote"));
        assert_eq!(bookx_core::opf::find_opf_path(&a).unwrap(), "x/deep/b.opf");
        a.set_string("META-INF/container.xml", "<container><oops");
        assert!(fix_container(&mut a).unwrap().is_some());
    }

    #[test]
    fn unmanifested_files_added_with_ids_and_types() {
        let mut b = make_book(&["<p>x</p>"]);
        b.archive.set("OEBPS/images/pic one.png", vec![1]);
        b.archive.set("OEBPS/images/pic_one.png", vec![1]);
        b.archive.set_string("OEBPS/extra.xhtml", "<html/>");
        b.archive.set("OEBPS/.DS_Store", vec![0]);
        b.archive.set("OEBPS/9lives.css", vec![0]);
        b.archive.set("OEBPS/blob.xyz", vec![0]);
        let fixes = add_missing_manifest_entries(&mut b).unwrap();
        assert_eq!(fixes.len(), 5);
        assert!(b.package.item_by_path("OEBPS/images/pic one.png").is_some());
        assert_eq!(
            b.package
                .item_by_path("OEBPS/images/pic one.png")
                .unwrap()
                .id,
            "pic_one"
        );
        assert_eq!(
            b.package
                .item_by_path("OEBPS/images/pic_one.png")
                .unwrap()
                .id,
            "pic_one-2"
        );
        assert_eq!(
            b.package.item_by_path("OEBPS/9lives.css").unwrap().id,
            "f9lives"
        );
        assert_eq!(
            b.package.item_by_path("OEBPS/blob.xyz").unwrap().media_type,
            "application/octet-stream"
        );
        assert!(b.package.item_by_path("OEBPS/.DS_Store").is_none());
        assert!(add_missing_manifest_entries(&mut b).unwrap().is_empty());
        assert!(b.package.item_by_path("OEBPS/extra.xhtml").is_some());
    }

    #[test]
    fn missing_files_removed_from_manifest_and_spine() {
        let mut b = make_book(&["<p>x</p>", "<p>y</p>"]);
        b.archive.remove("OEBPS/chapter-2.xhtml");
        b.archive.remove("OEBPS/toc.ncx");
        let fixes = remove_missing_manifest_items(&mut b).unwrap();
        assert_eq!(fixes.len(), 2);
        assert!(fixes.iter().any(|f| f.description.contains("\"ch2\"")));
        assert!(b.package.item_by_id("ch2").is_none());
        assert_eq!(b.package.spine.len(), 1);
        assert!(
            b.package.toc_id.is_none(),
            "dangling spine toc attribute removed"
        );
        let opf = b.archive.get_string(&b.package.opf_path).unwrap();
        assert!(!opf.contains("toc=\"ncx\""));
        assert!(remove_missing_manifest_items(&mut b).unwrap().is_empty());
    }

    #[test]
    fn duplicate_manifest_ids() {
        let mut b = make_book(&["<p>x</p>"]);
        let opf = b.package.opf_path.clone();
        let t = b
            .archive
            .get_string(&opf)
            .unwrap()
            .replace("id=\"css\"", "id=\"ch1\"");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        let f = fix_duplicate_manifest_ids(&mut b).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(
            f[0].description,
            "Renamed duplicate manifest id \"ch1\" to \"ch1-2\""
        );
        assert!(b.package.item_by_id("ch1-2").is_some());
        assert_eq!(b.package.item_by_id("ch1").unwrap().href, "style.css");
        assert_eq!(
            b.package.item_by_id("ch1-2").unwrap().href,
            "chapter-1.xhtml"
        );
        assert!(fix_duplicate_manifest_ids(&mut b).unwrap().is_empty());
    }

    #[test]
    fn duplicate_element_ids_keep_first() {
        let mut b =
            make_book(&["<p id=\"a\">x</p><p id=\"a\">y</p><p id=\"a-2\">z</p><p id=\"a\">w</p>"]);
        let f = fix_duplicate_element_ids(&mut b);
        assert_eq!(f.len(), 2);
        let t = b.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(
            t.contains(
                "<p id=\"a\">x</p><p id=\"a-3\">y</p><p id=\"a-2\">z</p><p id=\"a-4\">w</p>"
            ),
            "{t}"
        );
        assert!(fix_duplicate_element_ids(&mut b).is_empty());
    }

    #[test]
    fn missing_alt_added_only_where_absent() {
        let mut b = make_book(&[
            "<p><img src=\"a.png\"/><img src=\"b.png\" alt=\"B\"/><img src=\"c.png\" alt=\"\" /><IMG src=\"d.png\"></IMG></p>",
        ]);
        let f = fix_missing_alt(&mut b);
        assert_eq!(f.len(), 1);
        assert!(f[0].description.contains("2 image"));
        let t = b.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
        assert!(t.contains("<img src=\"a.png\" alt=\"\"/>"));
        assert!(t.contains("alt=\"B\""));
        assert!(fix_missing_alt(&mut b).is_empty());
    }

    #[test]
    fn repair_unclosed_void_entities_and_attributes() {
        let src = "<!DOCTYPE html>\n<HTML><head><title>T &amp; U</title><meta charset=utf-8><link rel=stylesheet href=s.css></head>\n<body><P CLASS=a>One &nbsp;&mdash; AT&T <br>two<img src=a.png alt=x>\n<p>Next 1 < 2 <b>bold <i>it</b> end\n<input disabled name=x name=y></body></HTML>";
        let fixed = repair_xhtml(src).unwrap();
        let doc = roxmltree::Document::parse_with_options(
            &fixed,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(fixed.contains("xmlns=\"http://www.w3.org/1999/xhtml\""));
        assert!(fixed.contains("<meta charset=\"utf-8\"/>"));
        assert!(fixed.contains("<br/>"));
        assert!(fixed.contains("&#160;&#8212; AT&amp;T"), "{fixed}");
        assert!(fixed.contains("1 &lt; 2"));
        assert!(fixed.contains("<p class=\"a\">"));
        assert!(fixed.contains("<input disabled=\"disabled\" name=\"x\"/>"));
        let text: String = doc
            .descendants()
            .filter(|n| n.is_text())
            .filter_map(|n| n.text())
            .collect();
        for w in ["One", "AT&T", "two", "Next", "bold", "it", "end"] {
            assert!(text.contains(w), "{w} in {text}");
        }
        // idempotent on well-formed output
        assert_eq!(repair_xhtml(&fixed).unwrap(), fixed);
    }

    #[test]
    fn repair_namespaces_comments_scripts_and_strays() {
        let src = "<html><body epub:type=\"x\"><!-- a -- b --></p><svg><use xlink:href=\"#a\"/></svg><script>if (a < b && c) {}</script><p>ok</p></body></html>";
        let fixed = repair_xhtml(src).unwrap();
        assert!(fixed.contains("xmlns:epub=\"http://www.idpf.org/2007/ops\""));
        assert!(fixed.contains("xmlns:xlink="));
        assert!(fixed.contains("<!-- a - - b -->"));
        assert!(fixed.contains("/*<![CDATA[*/if (a < b && c) {}/*]]>*/"));
        assert!(!fixed.contains("</p><svg>"), "stray end tag dropped");
        assert!(
            repair_xhtml("<html><body/></html><html><body/></html>").is_none(),
            "two root elements cannot be repaired"
        );
    }

    #[test]
    fn wellformedness_fix_and_namespace() {
        let mut b = make_book(&["<p>fine</p>", "<p>x</p>"]);
        b.archive.set_string(
            "OEBPS/chapter-2.xhtml",
            "<html><body><p>Tom & Jerry<br></body></html>",
        );
        b.archive.set_string(
            "OEBPS/chapter-1.xhtml",
            "<html><head><title>t</title></head><body><p>fine</p></body></html>",
        );
        let (f, fail) = fix_wellformedness(&mut b);
        assert!(fail.is_empty());
        assert_eq!(f.len(), 2);
        assert!(f
            .iter()
            .any(|x| x.file.ends_with("chapter-1.xhtml") && x.description.contains("namespace")));
        assert!(
            f.iter()
                .any(|x| x.file.ends_with("chapter-2.xhtml")
                    && x.description.starts_with("Repaired"))
        );
        assert!(
            b.archive
                .get_string("OEBPS/chapter-2.xhtml")
                .unwrap()
                .contains("Tom &amp; Jerry<br/>")
        );
        let (f2, _) = fix_wellformedness(&mut b);
        assert!(f2.is_empty());
        assert_eq!(fix_missing_namespace("<p>no html</p>"), None);
    }

    #[test]
    fn metadata_added_when_missing() {
        let mut b = make_book(&["<p>x</p>"]);
        let opf = b.package.opf_path.clone();
        let t = b.archive.get_string(&opf).unwrap();
        let t = Regex::new(
            r"<dc:(title|language|identifier)[^>]*>[^<]*</dc:(title|language|identifier)>\s*",
        )
        .unwrap()
        .replace_all(&t, "")
        .into_owned();
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        assert!(b.package.metadata.title.is_empty() && b.package.metadata.identifiers.is_empty());
        let opts = FixOptions {
            identifier: Some("urn:isbn:9781234567897".into()),
            ..Default::default()
        };
        let f = fix_metadata(&mut b, &opts).unwrap();
        assert_eq!(f.len(), 3, "{f:?}");
        assert_eq!(b.package.metadata.title, "Untitled");
        assert_eq!(b.package.metadata.language, "en");
        assert_eq!(b.package.unique_id, "bookid");
        assert!(
            b.package
                .metadata
                .identifiers
                .iter()
                .any(|i| i.value.contains("9781234567897"))
        );
        assert!(
            check_book(&b.archive)
                .iter()
                .all(|i| i.severity != Severity::Error)
        );
        assert!(fix_metadata(&mut b, &opts).unwrap().is_empty());
    }

    #[test]
    fn empty_metadata_elements_replaced_and_random_uuid() {
        let mut b = make_book(&["<p>x</p>"]);
        let opf = b.package.opf_path.clone();
        let t = b.archive.get_string(&opf).unwrap();
        let t = Regex::new(r"<dc:title>[^<]*</dc:title>")
            .unwrap()
            .replace(&t, "<dc:title/>")
            .into_owned();
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        let t = b.archive.get_string(&opf).unwrap();
        let t = t.replace(
            "<dc:identifier id=\"bookid\">",
            "<dc:identifier id=\"other\">",
        );
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        let f = fix_metadata(
            &mut b,
            &FixOptions {
                default_title: "My Book".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(f.len(), 2, "{f:?}");
        assert_eq!(b.package.metadata.title, "My Book");
        let opf_text = b.archive.get_string(&opf).unwrap();
        assert_eq!(opf_text.matches("<dc:title").count(), 1);
        assert_eq!(b.package.unique_id, "other");
        let mut c = make_book(&["<p>x</p>"]);
        let t = c.archive.get_string(&c.package.opf_path.clone()).unwrap();
        let t = Regex::new(r"<dc:identifier[^>]*>[^<]*</dc:identifier>")
            .unwrap()
            .replace(&t, "")
            .into_owned();
        c.archive.set_string(&c.package.opf_path.clone(), &t);
        c.reload_package().unwrap();
        fix_metadata(&mut c, &FixOptions::default()).unwrap();
        let id = &c.package.metadata.identifiers[0];
        assert!(
            id.value.contains("-") && (id.scheme == "uuid" || id.value.starts_with("urn:uuid:")),
            "{id:?}"
        );
    }

    #[test]
    fn unique_identifier_without_ids() {
        let mut b = make_book(&["<p>x</p>"]);
        let opf = b.package.opf_path.clone();
        let t = b
            .archive
            .get_string(&opf)
            .unwrap()
            .replace(" id=\"bookid\"", "")
            .replace("unique-identifier=\"bookid\"", "unique-identifier=\"gone\"");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        assert!(!b.package.metadata.identifiers.is_empty());
        let f = fix_unique_identifier(&mut b).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(b.package.unique_id, "bookid");
        assert!(fix_unique_identifier(&mut b).unwrap().is_empty());
    }

    #[test]
    fn version_attributes() {
        let mut b = make_book(&["<p>x</p>"]);
        let opf = b.package.opf_path.clone();
        let t = b.archive.get_string(&opf).unwrap();
        let t = t
            .replace("version=\"3.0\"", "version=\"9\"")
            .replace(" xmlns=\"http://www.idpf.org/2007/opf\"", "");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        let f = fix_versions(&mut b).unwrap();
        assert_eq!(f.len(), 2);
        assert!(f[0].description.contains("3.0 (was \"9\")"));
        assert!(b.package.is_epub3());
        assert!(fix_versions(&mut b).unwrap().is_empty());
        // EPUB2 without nav and without spine toc
        let t = b.archive.get_string(&opf).unwrap();
        let t = t
            .replace("version=\"3.0\"", "version=\"2.0\"")
            .replace(" properties=\"nav\"", "")
            .replace(" toc=\"ncx\"", "");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        let f = fix_versions(&mut b).unwrap();
        assert_eq!(f.len(), 1);
        assert!(f[0].description.contains("spine toc"));
        assert_eq!(b.package.toc_id.as_deref(), Some("ncx"));
        // missing version, no nav
        let t = b
            .archive
            .get_string(&opf)
            .unwrap()
            .replace(" version=\"2.0\"", "");
        b.archive.set_string(&opf, &t);
        b.reload_package().unwrap();
        let f = fix_versions(&mut b).unwrap();
        assert!(f[0].description.contains("was missing"));
        assert!(!b.package.is_epub3());
    }

    #[test]
    fn fix_book_end_to_end_and_idempotent() {
        let mut b = make_book(&["<p>One<img src=\"i.png\"></p>", "<p>Two</p>"]);
        b.archive.set_string("OEBPS/chapter-1.xhtml", "<html><head><title>x</title></head><body><p id=\"d\">One<img src=\"i.png\"><br></p><p id=\"d\">&nbsp;</p></body></html>");
        b.archive.set("OEBPS/i.png", crate::testutil::png(4, 4));
        b.archive.set("OEBPS/orphan.css", b"p{}".to_vec());
        b.archive.remove("OEBPS/chapter-2.xhtml");
        b.archive
            .set("mimetype", b"application/epub+zip\n".to_vec());
        let before = check_book(&b.archive).len();
        assert!(before >= 3);
        let report = fix_book(&mut b, &FixOptions::default()).unwrap();
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        // links to the removed chapter (here: in the nav document) are beyond automatic repair and reported
        assert_eq!(
            report.remaining_errors,
            vec!["OEBPS/nav.xhtml: Broken link: \"chapter-2.xhtml\" does not exist"]
        );
        let kinds: HashSet<FixKind> = report.fixes.iter().map(|f| f.kind).collect();
        for k in [
            FixKind::Mimetype,
            FixKind::ManifestRemoved,
            FixKind::ManifestAdded,
            FixKind::WellFormedness,
            FixKind::DuplicateElementId,
            FixKind::MissingAlt,
        ] {
            assert!(kinds.contains(&k), "{k:?} missing from {:?}", report.fixes);
        }
        let bytes = b.archive.to_bytes().unwrap();
        let reopened = OpenBook::from_archive(Archive::from_bytes(bytes).unwrap()).unwrap();
        assert_eq!(
            check_book(&reopened.archive)
                .iter()
                .filter(|i| i.severity == Severity::Error)
                .count(),
            1
        );
        let again = fix_book(&mut b, &FixOptions::default()).unwrap();
        assert!(
            again.is_empty(),
            "second run must change nothing: {:?}",
            again.fixes
        );
    }

    #[test]
    fn fix_archive_recovers_missing_container() {
        let b = make_book(&["<p>x</p>"]);
        let mut a = b.archive.clone();
        a.remove("META-INF/container.xml");
        a.remove("mimetype");
        assert!(OpenBook::from_archive(a.clone()).is_err());
        let (book, report) = fix_archive(a, &FixOptions::default()).unwrap();
        assert!(report.fixes.iter().any(|f| f.kind == FixKind::Container));
        assert!(report.fixes.iter().any(|f| f.kind == FixKind::Mimetype));
        assert_eq!(book.spine_paths().len(), 1);
        let mut broken = Archive::default();
        broken.set_string("a.opf", "<package");
        broken.set_string("c.xml", "x");
        let e = fix_archive(broken, &FixOptions::default()).err().unwrap();
        assert!(e.to_string().contains("too damaged"), "{e}");
    }

    #[test]
    fn helper_functions() {
        assert_eq!(sanitize_id("a b"), "a_b");
        assert_eq!(sanitize_id("1x"), "f1x");
        assert_eq!(sanitize_id(""), "f");
        assert!(is_junk("a/.DS_Store") && is_junk("__MACOSX/x") && !is_junk("a/b.png"));
        assert_eq!(
            fix_ampersands("a & b &copy; &#65; &zzz; &amp;"),
            "a &amp; b &#169; &#65; &amp;zzz; &amp;"
        );
        assert_eq!(fix_name("DIV"), "div");
        assert_eq!(fix_name("linearGradient"), "linearGradient");
        assert_eq!(fix_name("svg:Rect"), "svg:Rect");
        assert_eq!(xml_escape("x"), "x");
    }

    #[test]
    fn fix_report_lists_the_validator_ids_it_resolved() {
        use crate::testepub::*;
        let mut files = clean_epub3();
        remove(&mut files, "mimetype"); // PKG-006
        set(&mut files, "OEBPS/extra.txt", "loose"); // OPF-003
        set(
            &mut files,
            "OEBPS/ch2.xhtml",
            xhtml3(
                "<title>Two</title>",
                "<section id=\"sec\"><p>x <a href=\"ch1.xhtml\">back</a></p><img src=\"images/cover.png\"/></section>",
            ),
        ); // HTM-907
        let archive = to_archive(&files);
        let before = crate::validate::validate(&archive).ids();
        assert_eq!(before, vec!["HTM-907", "OPF-003", "PKG-006"]);
        let (book, report) = fix_archive(archive, &FixOptions::default()).unwrap();
        assert_eq!(
            report.resolved_validator_ids,
            vec!["HTM-907", "OPF-003", "PKG-006"]
        );
        assert!(report.remaining_validator_ids.is_empty());
        let after = crate::validate::validate(&book.archive);
        assert!(after.passed, "{}", crate::validate::render_text(&after));

        // fixing a fixed book resolves nothing
        let mut book = book;
        let again = fix_book(&mut book, &FixOptions::default()).unwrap();
        assert!(again.resolved_validator_ids.is_empty());
    }

    #[test]
    fn fix_report_keeps_unfixable_errors_as_remaining() {
        use crate::testepub::*;
        let mut files = clean_epub3();
        rep(
            &mut files,
            "OEBPS/ch2.xhtml",
            "href=\"ch1.xhtml\"",
            "href=\"../../outside.xhtml\"",
        ); // RSC-026, no fixer
        set(&mut files, "OEBPS/extra.txt", "loose"); // OPF-003, fixed
        let mut book = OpenBook::from_archive(to_archive(&files)).unwrap();
        let report = fix_book(&mut book, &FixOptions::default()).unwrap();
        assert_eq!(report.resolved_validator_ids, vec!["OPF-003"]);
        assert_eq!(report.remaining_validator_ids, vec!["RSC-026"]);
    }

    #[test]
    fn html5_parser_rebuilds_tag_soup_as_xhtml() {
        let soup = "<p>One <b>bold <i>both</b> italic</i><br><img src=a.png alt=x><p>Two &nbsp;&copy; <span epub:type=pagebreak>3</span>";
        let out = html5_to_xhtml(soup).expect("rebuilt");
        assert!(out.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>"));
        assert!(out.contains("<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">"), "{out}");
        assert!(out.contains("<br/>"));
        assert!(out.contains("alt=\"x\""));
        assert!(out.contains("epub:type=\"pagebreak\""));
        assert!(out.contains("\u{a0}\u{a9}"));
        assert!(parse_xml(&out).is_ok());
        // a second pass changes nothing
        assert_eq!(html5_to_xhtml(&out).unwrap(), out);
    }

    #[test]
    fn html5_output_keeps_svg_and_script_text_valid() {
        let src = "<html><body><svg viewBox=\"0 0 1 1\"><circle r=\"1\"/></svg><script>if (a < b && c) {}</script></body></html>";
        let out = html5_to_xhtml(src).unwrap();
        assert!(
            out.contains("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\">"),
            "{out}"
        );
        assert!(out.contains("/*<![CDATA[*/if (a < b && c) {}/*]]>*/"));
        assert!(parse_xml(&out).is_ok());
    }

    #[test]
    fn fix_html_files_scope_and_fallback() {
        let mut book = crate::testutil::make_book(&["<p>fine</p>", "<p>fine two</p>"]);
        let spine = book.spine_paths();
        let broken = "<html><body><p>Broken <b>text<p>more</body></html>";
        book.archive.set_string(&spine[0], broken);
        book.archive.set_string(&spine[1], broken);
        let (fixes, failures) = fix_html_files(&mut book, &[spine[0].clone()], false);
        assert_eq!(fixes.len(), 1);
        assert!(failures.is_empty());
        assert!(parse_xml(&book.archive.get_string(&spine[0]).unwrap()).is_ok());
        assert_eq!(
            book.archive.get_string(&spine[1]).unwrap(),
            broken,
            "other files stay as they were"
        );
        let (all, _) = fix_html_files(&mut book, &[], true);
        assert_eq!(all.len(), 1);
        assert!(
            all[0]
                .description
                .starts_with("Rebuilt with the HTML5 parser")
        );
        let (none, _) = fix_html_files(&mut book, &[], false);
        assert!(none.is_empty());
    }
}
