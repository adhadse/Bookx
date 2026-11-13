//! EPUB package (OPF) parsing and in-place editing.
//!
//! Edits are performed as *text splices* on the original document so that
//! everything we do not model (custom elements, comments, formatting) survives.

use std::ops::Range;

use anyhow::{anyhow, Context, Result};
use roxmltree::Node;
use serde::{Deserialize, Serialize};

use super::archive::Archive;
use super::util::*;

pub const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
pub const NS_OPF: &str = "http://www.idpf.org/2007/opf";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identifier {
    pub scheme: String,
    pub value: String,
    #[serde(default)]
    pub xml_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub title: String,
    pub title_sort: String,
    pub authors: Vec<String>,
    pub author_sort: String,
    pub publisher: String,
    pub language: String,
    pub description: String,
    pub tags: Vec<String>,
    pub series: String,
    pub series_index: f64,
    pub identifiers: Vec<Identifier>,
    pub published: String,
    pub rights: String,
    /// 0..=10 (Calibre convention, 2 per star)
    pub rating: u8,
}

impl Metadata {
    #[allow(dead_code)]
    pub fn identifier(&self, scheme: &str) -> Option<&str> {
        self.identifiers
            .iter()
            .find(|i| i.scheme.eq_ignore_ascii_case(scheme))
            .map(|i| i.value.as_str())
    }

    pub fn authors_display(&self) -> String {
        if self.authors.is_empty() {
            "Unknown".into()
        } else {
            self.authors.join(" & ")
        }
    }

    /// "isbn:123, doi:abc"
    pub fn identifiers_string(&self) -> String {
        self.identifiers
            .iter()
            .map(|i| {
                if i.scheme.is_empty() {
                    i.value.clone()
                } else {
                    format!("{}:{}", i.scheme, i.value)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn set_identifiers_from_string(&mut self, s: &str) {
        let old = self.identifiers.clone();
        self.identifiers = s
            .split(',')
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (scheme, value) = match p.split_once(':') {
                    Some((s, v)) => (s.trim().to_string(), v.trim().to_string()),
                    None => (String::new(), p.to_string()),
                };
                let xml_id = old
                    .iter()
                    .find(|o| o.scheme == scheme)
                    .and_then(|o| o.xml_id.clone());
                Identifier {
                    scheme,
                    value,
                    xml_id,
                }
            })
            .collect();
        // The identifier referenced by `unique-identifier` must survive.
        for o in old.into_iter().filter(|o| o.xml_id.is_some()) {
            if !self.identifiers.iter().any(|i| i.xml_id == o.xml_id) {
                self.identifiers.insert(0, o);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ManifestItem {
    pub id: String,
    pub href: String,
    pub media_type: String,
    pub properties: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpineItem {
    pub idref: String,
    pub linear: bool,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub opf_path: String,
    pub version: String,
    pub unique_id: String,
    pub metadata: Metadata,
    pub manifest: Vec<ManifestItem>,
    pub spine: Vec<SpineItem>,
    pub toc_id: Option<String>,
    pub cover_id: Option<String>,
    /// `page-progression-direction="rtl"` on the spine: the book turns pages right to left.
    pub rtl: bool,
}

fn local_attr<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name).map(|a| a.value())
}

fn node_text(n: Node) -> String {
    n.descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect::<String>()
        .trim()
        .to_string()
}

fn child_named<'a, 'b>(n: Node<'a, 'b>, name: &str) -> Option<Node<'a, 'b>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

pub fn find_opf_path(archive: &Archive) -> Result<String> {
    let container = archive
        .get_string("META-INF/container.xml")
        .context("missing META-INF/container.xml")?;
    let doc = parse_xml(&container)?;
    doc.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "rootfile")
        .and_then(|n| n.attribute("full-path"))
        .map(|s| s.to_string())
        .context("container.xml has no rootfile")
}

impl Package {
    pub fn parse(archive: &Archive) -> Result<Package> {
        let opf_path = find_opf_path(archive)?;
        let text = archive
            .get_string(&opf_path)
            .with_context(|| format!("missing package document {opf_path}"))?;
        Self::parse_text(&opf_path, &text)
    }

    pub fn parse_text(opf_path: &str, text: &str) -> Result<Package> {
        let doc = parse_xml(text)?;
        let root = doc.root_element();
        let version = root.attribute("version").unwrap_or("2.0").to_string();
        let unique_id = root
            .attribute("unique-identifier")
            .unwrap_or("")
            .to_string();

        let md_node = child_named(root, "metadata").context("package has no <metadata>")?;
        let mut md = Metadata::default();
        let mut cover_id = None;
        let mut collection: Option<(String, Option<String>)> = None;
        let mut group_positions: Vec<(String, String)> = Vec::new();

        for c in md_node.children().filter(|c| c.is_element()) {
            let name = c.tag_name().name();
            let ns = c.tag_name().namespace();
            if ns == Some(NS_DC) {
                let t = node_text(c);
                match name {
                    "title" if md.title.is_empty() => {
                        md.title = t;
                        if let Some(s) = local_attr(c, "file-as") {
                            md.title_sort = s.to_string();
                        }
                    }
                    "creator" => {
                        let role = local_attr(c, "role").unwrap_or("aut");
                        if role == "aut" && !t.is_empty() {
                            if md.author_sort.is_empty() {
                                if let Some(s) = local_attr(c, "file-as") {
                                    md.author_sort = s.to_string();
                                }
                            }
                            // "A; B" is a common (non-standard) way to list several authors
                            for a in t.split(';').map(|a| a.trim()).filter(|a| !a.is_empty()) {
                                md.authors.push(a.to_string());
                            }
                        }
                    }
                    "publisher" if md.publisher.is_empty() => md.publisher = t,
                    "language" if md.language.is_empty() => md.language = t,
                    "description" if md.description.is_empty() => md.description = t,
                    "subject" if !t.is_empty() => md.tags.push(t),
                    "date" if md.published.is_empty() => {
                        // some publishers ship a placeholder "0101-01-01" date
                        if !t.starts_with("0101") && !t.starts_with("0001") {
                            md.published = t
                        }
                    }
                    "rights" if md.rights.is_empty() => md.rights = t,
                    "identifier" if !t.is_empty() => {
                        let xml_id = c.attribute("id").map(|s| s.to_string());
                        let (scheme, value) = if let Some(s) = local_attr(c, "scheme") {
                            (s.to_ascii_lowercase(), t)
                        } else if let Some(rest) = t.strip_prefix("urn:") {
                            match rest.split_once(':') {
                                Some((s, v)) => (s.to_ascii_lowercase(), v.to_string()),
                                None => (String::new(), t.clone()),
                            }
                        } else {
                            let digits: String =
                                t.chars().filter(|c| *c != '-' && *c != ' ').collect();
                            let looks_isbn = (digits.len() == 13 || digits.len() == 10)
                                && digits[..digits.len() - 1]
                                    .chars()
                                    .all(|c| c.is_ascii_digit());
                            if looks_isbn {
                                ("isbn".to_string(), t)
                            } else {
                                (String::new(), t)
                            }
                        };
                        md.identifiers.push(Identifier {
                            scheme,
                            value,
                            xml_id,
                        });
                    }
                    _ => {}
                }
            } else if name == "meta" {
                let content = c.attribute("content");
                match (c.attribute("name"), c.attribute("property")) {
                    (Some("calibre:series"), _) => md.series = content.unwrap_or("").to_string(),
                    (Some("calibre:series_index"), _) => {
                        md.series_index = content.and_then(|s| s.parse().ok()).unwrap_or(1.0)
                    }
                    (Some("calibre:title_sort"), _) => {
                        md.title_sort = content.unwrap_or("").to_string()
                    }
                    (Some("calibre:rating"), _) => {
                        md.rating = content
                            .and_then(|s| s.parse::<f64>().ok())
                            .map(|r| r.round().clamp(0.0, 10.0) as u8)
                            .unwrap_or(0)
                    }
                    (Some("cover"), _) => cover_id = content.map(|s| s.to_string()),
                    (_, Some("belongs-to-collection")) => {
                        collection = Some((node_text(c), c.attribute("id").map(String::from)));
                    }
                    (_, Some("group-position")) => {
                        if let Some(r) = c.attribute("refines") {
                            group_positions
                                .push((r.trim_start_matches('#').to_string(), node_text(c)));
                        }
                    }
                    _ => {}
                }
            }
        }
        if md.series.is_empty() {
            if let Some((name, id)) = collection {
                md.series = name;
                if let Some(id) = id {
                    if let Some((_, pos)) = group_positions.iter().find(|(r, _)| *r == id) {
                        md.series_index = pos.parse().unwrap_or(1.0);
                    }
                }
            }
        }
        if md.series_index == 0.0 {
            md.series_index = 1.0;
        }
        if md.title_sort.is_empty() {
            md.title_sort = title_sort(&md.title);
        }
        if md.author_sort.is_empty() {
            md.author_sort = md
                .authors
                .iter()
                .map(|a| author_sort(a))
                .collect::<Vec<_>>()
                .join(" & ");
        }

        let mut manifest = Vec::new();
        if let Some(m) = child_named(root, "manifest") {
            for it in m
                .children()
                .filter(|c| c.is_element() && c.tag_name().name() == "item")
            {
                manifest.push(ManifestItem {
                    id: it.attribute("id").unwrap_or("").to_string(),
                    href: it.attribute("href").unwrap_or("").to_string(),
                    media_type: it.attribute("media-type").unwrap_or("").to_string(),
                    properties: it.attribute("properties").unwrap_or("").to_string(),
                });
            }
        }
        let mut spine = Vec::new();
        let mut toc_id = None;
        let mut rtl = false;
        if let Some(s) = child_named(root, "spine") {
            rtl = s
                .attribute("page-progression-direction")
                .is_some_and(|d| d.trim().eq_ignore_ascii_case("rtl"));
            toc_id = s.attribute("toc").map(|s| s.to_string());
            for it in s
                .children()
                .filter(|c| c.is_element() && c.tag_name().name() == "itemref")
            {
                spine.push(SpineItem {
                    idref: it.attribute("idref").unwrap_or("").to_string(),
                    linear: it.attribute("linear") != Some("no"),
                });
            }
        }

        let mut pkg = Package {
            opf_path: opf_path.to_string(),
            version,
            unique_id,
            metadata: md,
            manifest,
            spine,
            toc_id,
            cover_id,
            rtl,
        };
        if pkg.cover_item().is_none() {
            pkg.cover_id = pkg.guess_cover_id();
        }
        Ok(pkg)
    }

    pub fn base_dir(&self) -> &str {
        parent_dir(&self.opf_path)
    }

    pub fn is_epub3(&self) -> bool {
        self.version.starts_with('3')
    }

    pub fn item_path(&self, item: &ManifestItem) -> String {
        resolve_href(self.base_dir(), &item.href).0
    }

    pub fn item_by_id(&self, id: &str) -> Option<&ManifestItem> {
        self.manifest.iter().find(|i| i.id == id)
    }

    pub fn item_by_path(&self, path: &str) -> Option<&ManifestItem> {
        self.manifest.iter().find(|i| self.item_path(i) == path)
    }

    pub fn spine_paths(&self) -> Vec<String> {
        self.spine
            .iter()
            .filter_map(|s| self.item_by_id(&s.idref))
            .map(|i| self.item_path(i))
            .collect()
    }

    pub fn nav_item(&self) -> Option<&ManifestItem> {
        self.manifest
            .iter()
            .find(|i| i.properties.split_whitespace().any(|p| p == "nav"))
    }

    pub fn ncx_item(&self) -> Option<&ManifestItem> {
        self.toc_id
            .as_deref()
            .and_then(|id| self.item_by_id(id))
            .or_else(|| {
                self.manifest
                    .iter()
                    .find(|i| i.media_type == "application/x-dtbncx+xml")
            })
    }

    pub fn cover_item(&self) -> Option<&ManifestItem> {
        self.cover_id
            .as_deref()
            .and_then(|id| self.item_by_id(id))
            .filter(|i| i.media_type.starts_with("image/"))
    }

    pub fn cover_path(&self) -> Option<String> {
        self.cover_item().map(|i| self.item_path(i))
    }

    fn guess_cover_id(&self) -> Option<String> {
        let imgs = || {
            self.manifest
                .iter()
                .filter(|i| i.media_type.starts_with("image/"))
        };
        imgs()
            .find(|i| i.properties.split_whitespace().any(|p| p == "cover-image"))
            .or_else(|| imgs().find(|i| i.id.to_ascii_lowercase().contains("cover")))
            .or_else(|| {
                imgs().find(|i| file_name(&i.href).to_ascii_lowercase().starts_with("cover"))
            })
            .map(|i| i.id.clone())
    }

    pub fn cover_bytes<'a>(&self, archive: &'a Archive) -> Option<&'a [u8]> {
        self.cover_path().and_then(|p| archive.get(&p))
    }
}

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

fn line_indent(text: &str, pos: usize) -> String {
    let line_start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let s = &text[line_start..pos];
    if s.chars().all(|c| c == ' ' || c == '\t') {
        s.to_string()
    } else {
        String::new()
    }
}

/// Range to delete so that an element on its own line disappears together with its line break.
fn removal_range(text: &str, r: Range<usize>) -> Range<usize> {
    let mut start = r.start;
    let bytes = text.as_bytes();
    while start > 0 && (bytes[start - 1] == b' ' || bytes[start - 1] == b'\t') {
        start -= 1;
    }
    if start > 0 && bytes[start - 1] == b'\n' {
        start -= 1;
        if start > 0 && bytes[start - 1] == b'\r' {
            start -= 1;
        }
    }
    start..r.end
}

fn ns_prefix(node: Node, uri: &str) -> Option<String> {
    node.namespaces()
        .find(|n| n.uri() == uri && n.name().is_some())
        .and_then(|n| n.name().map(|s| s.to_string()))
}

/// Applies `edit` to the OPF text and stores the result.
fn splice_opf(
    archive: &mut Archive,
    opf_path: &str,
    edit: impl FnOnce(&str, &roxmltree::Document) -> Result<String>,
) -> Result<()> {
    let text = archive
        .get_string(opf_path)
        .with_context(|| format!("missing {opf_path}"))?;
    let new = {
        let doc = parse_xml(&text)?;
        edit(&text, &doc)?
    };
    // Safety net: never store a document that no longer parses.
    parse_xml(&new).context("edited package document is not well-formed")?;
    archive.set_string(opf_path, &new);
    Ok(())
}

fn element_start_tag(text: &str, node: Node) -> (usize, String) {
    let r = node.range();
    let gt = text[r.start..]
        .find('>')
        .map(|i| r.start + i)
        .unwrap_or(r.start);
    (gt, text[r.start..=gt].to_string())
}

fn is_modeled(n: Node, seen: &mut Vec<String>, removed_ids: &mut Vec<String>) -> bool {
    let name = n.tag_name().name();
    let ns = n.tag_name().namespace();
    let remember = |removed_ids: &mut Vec<String>| {
        if let Some(id) = n.attribute("id") {
            removed_ids.push(id.to_string());
        }
    };
    if ns == Some(NS_DC) {
        let multi = matches!(name, "creator" | "subject" | "identifier");
        let single = matches!(
            name,
            "title" | "publisher" | "language" | "description" | "date" | "rights"
        );
        if multi || (single && !seen.iter().any(|s| s == name)) {
            if single {
                seen.push(name.to_string());
            }
            if name != "identifier" {
                remember(removed_ids);
            }
            return true;
        }
        return false;
    }
    if name == "meta" {
        let nm = n.attribute("name");
        let prop = n.attribute("property");
        let modeled = matches!(
            nm,
            Some(
                "calibre:series"
                    | "calibre:series_index"
                    | "calibre:title_sort"
                    | "calibre:rating"
                    | "cover"
            )
        ) || matches!(prop, Some("dcterms:modified" | "belongs-to-collection"));
        if modeled {
            remember(removed_ids);
        }
        return modeled;
    }
    false
}

impl Package {
    /// Writes `self.metadata` and the cover reference back into the OPF stored in `archive`.
    pub fn write_metadata(&self, archive: &mut Archive) -> Result<()> {
        let md = &self.metadata;
        let cover_id = self.cover_id.clone();
        let epub3 = self.is_epub3();
        let unique_id = self.unique_id.clone();
        splice_opf(archive, &self.opf_path, move |text, doc| {
            let root = doc.root_element();
            let node = child_named(root, "metadata").context("package has no <metadata>")?;
            let range = node.range();
            let (_gt, start_tag) = element_start_tag(text, node);

            let dc_prefix = ns_prefix(node, NS_DC);
            let opf_prefix = ns_prefix(node, NS_OPF);
            let dcp = dc_prefix.clone().unwrap_or_else(|| "dc".into());
            let dc_decl = if dc_prefix.is_none() {
                format!(" xmlns:{dcp}=\"{NS_DC}\"")
            } else {
                String::new()
            };
            let opp = opf_prefix.clone().unwrap_or_else(|| "opf".into());
            let opf_decl = if opf_prefix.is_none() {
                format!(" xmlns:{opp}=\"{NS_OPF}\"")
            } else {
                String::new()
            };

            let first_child = node.children().find(|c| c.is_element());
            let indent = first_child
                .map(|c| line_indent(text, c.range().start))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "    ".to_string());
            let closing_start = text[..range.end].rfind("</").unwrap_or(range.end);
            let close_indent = line_indent(text, closing_start);
            let close_tag = &text[closing_start..range.end];

            let mut lines: Vec<String> = Vec::new();
            let dcel = |name: &str, attrs: String, value: &str| -> String {
                format!(
                    "<{dcp}:{name}{dc_decl}{attrs}>{}</{dcp}:{name}>",
                    xml_escape(value)
                )
            };
            let meta_el = |name: &str, content: &str| -> String {
                format!(
                    "<meta name=\"{name}\" content=\"{}\"/>",
                    xml_escape(content)
                )
            };

            if !md.title.is_empty() {
                let attrs = if md.title_sort.is_empty() || md.title_sort == title_sort(&md.title) {
                    String::new()
                } else {
                    format!(
                        "{opf_decl} {opp}:file-as=\"{}\"",
                        xml_escape(&md.title_sort)
                    )
                };
                lines.push(dcel("title", attrs, &md.title));
            }
            for (i, a) in md.authors.iter().enumerate() {
                let mut attrs = format!("{opf_decl} {opp}:role=\"aut\"");
                if i == 0 && !md.author_sort.is_empty() {
                    attrs.push_str(&format!(
                        " {opp}:file-as=\"{}\"",
                        xml_escape(&md.author_sort)
                    ));
                }
                lines.push(dcel("creator", attrs, a));
            }
            if !md.publisher.is_empty() {
                lines.push(dcel("publisher", String::new(), &md.publisher));
            }
            if !md.language.is_empty() {
                lines.push(dcel("language", String::new(), &md.language));
            }
            if !md.description.is_empty() {
                lines.push(dcel("description", String::new(), &md.description));
            }
            for t in &md.tags {
                lines.push(dcel("subject", String::new(), t));
            }
            if !md.published.is_empty() {
                lines.push(dcel("date", String::new(), &md.published));
            }
            if !md.rights.is_empty() {
                lines.push(dcel("rights", String::new(), &md.rights));
            }
            for id in &md.identifiers {
                let mut attrs = String::new();
                if let Some(x) = &id.xml_id {
                    attrs.push_str(&format!(" id=\"{}\"", xml_escape(x)));
                }
                let is_unique = id.xml_id.as_deref() == Some(unique_id.as_str()) && epub3;
                let value = if id.scheme.is_empty() {
                    id.value.clone()
                } else if is_unique {
                    format!("urn:{}:{}", id.scheme, id.value)
                } else {
                    attrs.push_str(&format!(
                        "{opf_decl} {opp}:scheme=\"{}\"",
                        xml_escape(&id.scheme)
                    ));
                    id.value.clone()
                };
                lines.push(dcel("identifier", attrs, &value));
            }
            if !md.series.is_empty() {
                lines.push(meta_el("calibre:series", &md.series));
                lines.push(meta_el(
                    "calibre:series_index",
                    &format_index(md.series_index),
                ));
            }
            if md.rating > 0 {
                lines.push(meta_el("calibre:rating", &md.rating.to_string()));
            }
            if let Some(c) = &cover_id {
                lines.push(meta_el("cover", c));
            }
            if epub3 {
                lines.push(format!(
                    "<meta property=\"dcterms:modified\">{}</meta>",
                    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
                ));
            }

            // Preserve everything we do not model.
            let mut seen = Vec::new();
            let mut removed_ids = Vec::new();
            let children: Vec<Node> = node.children().filter(|c| c.is_element()).collect();
            let modeled: Vec<bool> = children
                .iter()
                .map(|c| is_modeled(*c, &mut seen, &mut removed_ids))
                .collect();
            for (c, m) in children.iter().zip(modeled) {
                if m {
                    continue;
                }
                if c.tag_name().name() == "meta" {
                    if let Some(r) = c.attribute("refines") {
                        if removed_ids.iter().any(|i| i == r.trim_start_matches('#')) {
                            continue;
                        }
                    }
                }
                lines.push(text[c.range()].to_string());
            }

            let body = lines
                .iter()
                .map(|l| format!("{indent}{l}"))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(format!(
                "{}{}\n{}\n{}{}{}",
                &text[..range.start],
                start_tag,
                body,
                close_indent,
                close_tag,
                &text[range.end..]
            ))
        })
    }

    /// Replaces (or adds) the cover image. `ext` is the lowercase file extension, e.g. `jpg`.
    pub fn set_cover(&mut self, archive: &mut Archive, data: Vec<u8>, ext: &str) -> Result<()> {
        let ext = if ext == "jpeg" { "jpg" } else { ext };
        if let Some(item) = self.cover_item().cloned() {
            if extension(&item.href) == ext || (extension(&item.href) == "jpeg" && ext == "jpg") {
                let path = self.item_path(&item);
                archive.set(&path, data);
                return Ok(());
            }
        }
        let href = format!("images/bookx_cover.{ext}");
        let abs = join_path(self.base_dir(), &href);
        archive.set(&abs, data);
        let mut id = "bookx-cover".to_string();
        let mut n = 1;
        while self.item_by_id(&id).is_some() {
            n += 1;
            id = format!("bookx-cover-{n}");
        }
        // Demote the previous cover-image item, if any.
        let old_cover = self.cover_item().map(|i| i.id.clone());
        let props = if self.is_epub3() { "cover-image" } else { "" };
        self.add_manifest_item(archive, &id, &abs, media_type_for(&abs), props)?;
        if let Some(old) = old_cover {
            self.edit_item_properties(archive, &old, |p| {
                p.split_whitespace()
                    .filter(|t| *t != "cover-image")
                    .collect::<Vec<_>>()
                    .join(" ")
            })?;
        }
        *self = Package::parse(archive)?;
        self.cover_id = Some(id);
        self.write_metadata(archive)?;
        Ok(())
    }

    pub fn remove_cover(&mut self, archive: &mut Archive) -> Result<()> {
        if let Some(item) = self.cover_item().cloned() {
            self.edit_item_properties(archive, &item.id, |p| {
                p.split_whitespace()
                    .filter(|t| *t != "cover-image")
                    .collect::<Vec<_>>()
                    .join(" ")
            })?;
        }
        *self = Package::parse(archive)?;
        self.cover_id = None;
        self.write_metadata(archive)?;
        Ok(())
    }

    pub fn add_manifest_item(
        &self,
        archive: &mut Archive,
        id: &str,
        abs_path: &str,
        media: &str,
        properties: &str,
    ) -> Result<()> {
        let href = relative_href(&self.opf_path, abs_path);
        splice_opf(archive, &self.opf_path, |text, doc| {
            let manifest = child_named(doc.root_element(), "manifest").context("no <manifest>")?;
            let r = manifest.range();
            let closing = text[..r.end].rfind("</").unwrap_or(r.end);
            let item_indent = manifest
                .children()
                .find(|c| c.is_element())
                .map(|c| line_indent(text, c.range().start))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "    ".into());
            let close_indent = line_indent(text, closing);
            let props = if properties.is_empty() {
                String::new()
            } else {
                format!(" properties=\"{}\"", xml_escape(properties))
            };
            let item = format!(
                "{item_indent}<item id=\"{}\" href=\"{}\" media-type=\"{}\"{props}/>\n",
                xml_escape(id),
                xml_escape(&href),
                xml_escape(media)
            );
            // Insert before the whitespace that precedes </manifest>.
            let insert_at = closing - close_indent.len();
            Ok(format!(
                "{}{}{}",
                &text[..insert_at],
                item,
                &text[insert_at..]
            ))
        })
    }

    pub fn remove_manifest_item(&self, archive: &mut Archive, id: &str) -> Result<()> {
        splice_opf(archive, &self.opf_path, |text, doc| {
            let root = doc.root_element();
            let mut ranges: Vec<Range<usize>> = Vec::new();
            for sec in ["manifest", "spine"] {
                if let Some(m) = child_named(root, sec) {
                    for c in m.children().filter(|c| c.is_element()) {
                        let hit = (sec == "manifest" && c.attribute("id") == Some(id))
                            || (sec == "spine" && c.attribute("idref") == Some(id));
                        if hit {
                            ranges.push(removal_range(text, c.range()));
                        }
                    }
                }
            }
            ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
            let mut out = text.to_string();
            for r in ranges {
                out.replace_range(r, "");
            }
            Ok(out)
        })
    }

    pub fn set_item_href(&self, archive: &mut Archive, id: &str, new_abs: &str) -> Result<()> {
        let href = relative_href(&self.opf_path, new_abs);
        splice_opf(archive, &self.opf_path, |text, doc| {
            let manifest = child_named(doc.root_element(), "manifest").context("no <manifest>")?;
            let item = manifest
                .children()
                .find(|c| c.is_element() && c.attribute("id") == Some(id))
                .ok_or_else(|| anyhow!("no manifest item {id}"))?;
            let r = item.range();
            let slice = &text[r.clone()];
            let re = regex::Regex::new(r#"\shref\s*=\s*("[^"]*"|'[^']*')"#)?;
            let new_slice = re
                .replace(slice, format!(" href=\"{}\"", xml_escape(&href)).as_str())
                .into_owned();
            Ok(format!(
                "{}{}{}",
                &text[..r.start],
                new_slice,
                &text[r.end..]
            ))
        })
    }

    fn edit_item_properties(
        &self,
        archive: &mut Archive,
        id: &str,
        f: impl Fn(&str) -> String,
    ) -> Result<()> {
        splice_opf(archive, &self.opf_path, |text, doc| {
            let manifest = child_named(doc.root_element(), "manifest").context("no <manifest>")?;
            let Some(item) = manifest
                .children()
                .find(|c| c.is_element() && c.attribute("id") == Some(id))
            else {
                return Ok(text.to_string());
            };
            let r = item.range();
            let slice = &text[r.clone()];
            let old = item.attribute("properties").unwrap_or("");
            let new = f(old);
            let re = regex::Regex::new(r#"\sproperties\s*=\s*("[^"]*"|'[^']*')"#)?;
            let new_slice = if new.is_empty() {
                re.replace(slice, "").into_owned()
            } else if re.is_match(slice) {
                re.replace(
                    slice,
                    format!(" properties=\"{}\"", xml_escape(&new)).as_str(),
                )
                .into_owned()
            } else {
                let at = slice
                    .rfind("/>")
                    .or_else(|| slice.find('>'))
                    .unwrap_or(slice.len());
                format!(
                    "{} properties=\"{}\"{}",
                    &slice[..at],
                    xml_escape(&new),
                    &slice[at..]
                )
            };
            Ok(format!(
                "{}{}{}",
                &text[..r.start],
                new_slice,
                &text[r.end..]
            ))
        })
    }

    pub fn set_spine(&self, archive: &mut Archive, order: &[SpineItem]) -> Result<()> {
        splice_opf(archive, &self.opf_path, |text, doc| {
            let spine = child_named(doc.root_element(), "spine").context("no <spine>")?;
            let r = spine.range();
            let (_, start_tag) = element_start_tag(text, spine);
            let closing = text[..r.end].rfind("</").unwrap_or(r.end);
            let close_indent = line_indent(text, closing);
            let indent = spine
                .children()
                .find(|c| c.is_element())
                .map(|c| line_indent(text, c.range().start))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "    ".into());
            let body = order
                .iter()
                .map(|s| {
                    let linear = if s.linear { "" } else { " linear=\"no\"" };
                    format!(
                        "{indent}<itemref idref=\"{}\"{linear}/>",
                        xml_escape(&s.idref)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(format!(
                "{}{}\n{}\n{}{}{}",
                &text[..r.start],
                start_tag,
                body,
                close_indent,
                &text[closing..r.end],
                &text[r.end..]
            ))
        })
    }
}

pub fn format_index(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod rtl_tests {
    use super::*;

    fn opf(spine_attr: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="i">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title><dc:identifier id="i">x</dc:identifier></metadata>
<manifest><item id="a" href="a.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine {spine_attr}><itemref idref="a"/></spine></package>"#
        )
    }

    #[test]
    fn page_progression_direction_is_read() {
        assert!(
            Package::parse_text("c.opf", &opf("page-progression-direction=\"rtl\""))
                .unwrap()
                .rtl
        );
        assert!(
            Package::parse_text("c.opf", &opf("page-progression-direction=\" RTL \""))
                .unwrap()
                .rtl
        );
        assert!(
            !Package::parse_text("c.opf", &opf("page-progression-direction=\"ltr\""))
                .unwrap()
                .rtl
        );
        assert!(!Package::parse_text("c.opf", &opf("")).unwrap().rtl);
    }
}
