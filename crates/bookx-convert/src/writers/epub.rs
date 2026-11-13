//! EPUB writer: a clean re-pack of an [`OpenBook`] as EPUB 2 or EPUB 3.

use std::collections::HashSet;

use anyhow::Result;
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;
use bookx_core::toc::TocEntry;
use bookx_core::util::{join_path, media_type_for, relative_href, xml_escape};
use serde::{Deserialize, Serialize};

use crate::markup::{is_wellformed_xml, plain_text, repair_to_xhtml, tag_info, tokenize, Tok};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EpubVersion {
    #[serde(rename = "2")]
    V2,
    #[serde(rename = "3")]
    #[default]
    V3,
}

impl EpubVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V2 => "2.0",
            Self::V3 => "3.0",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EpubWriter {
    pub version: EpubVersion,
}

impl FormatWriter for EpubWriter {
    fn id(&self) -> &'static str {
        match self.version {
            EpubVersion::V2 => "epub2",
            EpubVersion::V3 => "epub",
        }
    }
    fn extension(&self) -> &'static str {
        "epub"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        repack(book, self.version)?.to_bytes()
    }
}

const V3_ONLY_PROPERTIES: &[&str] = &[
    "nav",
    "cover-image",
    "scripted",
    "svg",
    "mathml",
    "remote-resources",
    "switch",
];

/// Adds the EPUB 3 content properties (`svg`, `mathml`, `scripted`, `remote-resources`, `switch`)
/// that the document needs but the manifest does not declare.
fn content_properties(data: &[u8], props: &mut Vec<String>) {
    let text = String::from_utf8_lossy(data);
    let mut add = |p: &str, yes: bool| {
        if yes && !props.iter().any(|x| x == p) {
            props.push(p.to_string());
        }
    };
    add("svg", text.contains("<svg") || text.contains(":svg"));
    add("mathml", text.contains("<math") || text.contains(":math"));
    add(
        "scripted",
        text.contains("<script") && !text.contains("type=\"application/ld+json\""),
    );
    add("switch", text.contains("<epub:switch"));
}

fn ident_text(scheme: &str, value: &str) -> String {
    if scheme.is_empty() || value.starts_with("urn:") {
        value.to_string()
    } else {
        format!("urn:{scheme}:{value}")
    }
}

fn nav_list(entries: &[TocEntry], nav_path: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = format!("{pad}<ol>\n");
    for e in entries {
        out.push_str(&format!(
            "{pad}  <li><a href=\"{}\">{}</a>",
            xml_escape(&relative_href(nav_path, &e.href)),
            xml_escape(&e.title)
        ));
        if e.children.is_empty() {
            out.push_str("</li>\n");
        } else {
            out.push('\n');
            out.push_str(&nav_list(&e.children, nav_path, indent + 4));
            out.push_str(&format!("{pad}  </li>\n"));
        }
    }
    out.push_str(&format!("{pad}</ol>\n"));
    out
}

pub(crate) fn nav_document(title: &str, entries: &[TocEntry], nav_path: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">\n<head>\n  <meta charset=\"utf-8\"/>\n  <title>{}</title>\n</head>\n<body>\n  <nav epub:type=\"toc\" id=\"toc\">\n    <h1>Contents</h1>\n{}  </nav>\n</body>\n</html>\n",
        xml_escape(title),
        nav_list(entries, nav_path, 4)
    )
}

fn depth(entries: &[TocEntry]) -> usize {
    entries
        .iter()
        .map(|e| 1 + depth(&e.children))
        .max()
        .unwrap_or(0)
}

fn ncx_points(entries: &[TocEntry], ncx_path: &str, counter: &mut usize, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for e in entries {
        *counter += 1;
        let n = *counter;
        out.push_str(&format!(
            "{pad}<navPoint id=\"navpoint-{n}\" playOrder=\"{n}\">\n{pad}  <navLabel><text>{}</text></navLabel>\n{pad}  <content src=\"{}\"/>\n",
            xml_escape(&e.title),
            xml_escape(&relative_href(ncx_path, &e.href))
        ));
        out.push_str(&ncx_points(&e.children, ncx_path, counter, indent + 2));
        out.push_str(&format!("{pad}</navPoint>\n"));
    }
    out
}

pub(crate) fn ncx_document(title: &str, uid: &str, entries: &[TocEntry], ncx_path: &str) -> String {
    let mut counter = 0;
    bookx_core::toc::fix_play_order(&format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n  <head>\n    <meta name=\"dtb:uid\" content=\"{}\"/>\n    <meta name=\"dtb:depth\" content=\"{}\"/>\n    <meta name=\"dtb:totalPageCount\" content=\"0\"/>\n    <meta name=\"dtb:maxPageNumber\" content=\"0\"/>\n  </head>\n  <docTitle><text>{}</text></docTitle>\n  <navMap>\n{}  </navMap>\n</ncx>\n",
        xml_escape(uid),
        depth(entries).max(1),
        xml_escape(title),
        ncx_points(entries, ncx_path, &mut counter, 4)
    ))
}

/// A table of contents derived from the spine when the book has none.
fn fallback_toc(book: &OpenBook) -> Vec<TocEntry> {
    book.spine_paths()
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            let title = book
                .archive
                .get_string(&path)
                .and_then(|t| {
                    let mut in_title = false;
                    let mut s = String::new();
                    for tok in tokenize(&t) {
                        match tok {
                            Tok::Tag(g) => {
                                if let Some(info) = tag_info(g) {
                                    if info.name == "title" {
                                        in_title = !info.closing;
                                    }
                                }
                            }
                            Tok::Text(x) if in_title => s.push_str(x),
                            Tok::Text(_) => {}
                        }
                    }
                    let s = plain_text(&s).trim().to_string();
                    (!s.is_empty()).then_some(s)
                })
                .unwrap_or_else(|| format!("Section {}", i + 1));
            TocEntry {
                title,
                href: path,
                children: vec![],
            }
        })
        .collect()
}

fn unique_path(archive: &Archive, dir: &str, name: &str) -> String {
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let mut n = 0;
    loop {
        let file = match (n, ext.is_empty()) {
            (0, true) => stem.to_string(),
            (0, false) => format!("{stem}.{ext}"),
            (n, true) => format!("{stem}-{n}"),
            (n, false) => format!("{stem}-{n}.{ext}"),
        };
        let p = join_path(dir, &file);
        if !archive.contains(&p) {
            return p;
        }
        n += 1;
    }
}

struct Item {
    id: String,
    path: String,
    media: String,
    props: Vec<String>,
}

/// Builds a clean EPUB archive from the book: regenerated container and package documents,
/// consistent manifest/spine/ids, and a navigation document appropriate for `version`.
pub fn repack(book: &OpenBook, version: EpubVersion) -> Result<Archive> {
    let pkg = &book.package;
    let md = book.metadata();
    let v3 = version == EpubVersion::V3;
    let opf_path = pkg.opf_path.clone();
    let opf_dir = pkg.base_dir().to_string();

    let mut out = Archive::default();
    out.set("mimetype", b"application/epub+zip".to_vec());

    // --- manifest: unique ids, existing files only -------------------------------------
    let original_toc = book.toc();
    let toc_was_empty = original_toc.is_empty();
    let toc = if toc_was_empty {
        fallback_toc(book)
    } else {
        original_toc
    };
    let nav_old = pkg.nav_item().map(|i| i.id.clone());
    let mut items: Vec<Item> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_paths: HashSet<String> = HashSet::new();
    let mut id_map: Vec<(String, String)> = Vec::new(); // original id -> final id
    let mut counter = 0;
    let mut fresh_id = |seen: &HashSet<String>, base: &str| -> String {
        let mut id = base.to_string();
        while id.is_empty() || seen.contains(&id) {
            counter += 1;
            id = format!("{}-{counter}", if base.is_empty() { "item" } else { base });
        }
        id
    };
    for it in &pkg.manifest {
        let path = pkg.item_path(it);
        if !book.archive.contains(&path) || !seen_paths.insert(path.clone()) {
            continue;
        }
        if !v3 && nav_old.as_deref() == Some(it.id.as_str()) {
            continue; // EPUB 2 has no navigation document
        }
        let id = fresh_id(&seen_ids, &it.id);
        seen_ids.insert(id.clone());
        id_map.push((it.id.clone(), id.clone()));
        let props: Vec<String> = it
            .properties
            .split_whitespace()
            .filter(|p| v3 || !V3_ONLY_PROPERTIES.contains(p))
            .map(String::from)
            .collect();
        let media = if it.media_type.is_empty() {
            media_type_for(&path).to_string()
        } else {
            it.media_type.clone()
        };
        let mut data = book.archive.get(&path).unwrap_or_default().to_vec();
        let mut props = props;
        if media == "application/xhtml+xml" {
            if let Ok(text) = std::str::from_utf8(&data) {
                if !is_wellformed_xml(text) {
                    if let Some(fixed) = repair_to_xhtml(text) {
                        data = fixed.into_bytes();
                    }
                }
            }
            if v3 {
                content_properties(&data, &mut props);
            }
        }
        out.set(&path, data);
        items.push(Item {
            id,
            path,
            media,
            props,
        });
    }
    let final_id = |orig: &str| -> Option<String> {
        id_map
            .iter()
            .find(|(o, _)| o == orig)
            .map(|(_, f)| f.clone())
    };
    // files the manifest forgot
    for name in book.archive.names() {
        if name == "mimetype"
            || name.starts_with("META-INF/")
            || *name == opf_path
            || seen_paths.contains(name)
            || media_type_for(name) == "application/octet-stream"
            || name.ends_with('/')
        {
            continue;
        }
        let id = fresh_id(&seen_ids, "bookx-extra");
        seen_ids.insert(id.clone());
        seen_paths.insert(name.clone());
        out.set(name, book.archive.get(name).unwrap_or_default().to_vec());
        items.push(Item {
            id,
            path: name.clone(),
            media: media_type_for(name).to_string(),
            props: vec![],
        });
    }

    // --- cover ---------------------------------------------------------------------------
    let cover_path = pkg.cover_path();
    let cover_id = cover_path
        .as_ref()
        .and_then(|p| items.iter().find(|i| i.path == *p))
        .map(|i| i.id.clone());
    if v3 {
        if let Some(cid) = &cover_id {
            if let Some(it) = items.iter_mut().find(|i| i.id == *cid) {
                if !it.props.iter().any(|p| p == "cover-image") {
                    it.props.push("cover-image".into());
                }
            }
        }
    }

    // --- navigation documents ----------------------------------------------------------------
    let uid_text = {
        let chosen = md
            .identifiers
            .iter()
            .find(|i| {
                !pkg.unique_id.is_empty() && i.xml_id.as_deref() == Some(pkg.unique_id.as_str())
            })
            .or_else(|| md.identifiers.first());
        match chosen {
            Some(i) => ident_text(&i.scheme, &i.value),
            None => format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        }
    };
    let existing_ncx = pkg
        .ncx_item()
        .and_then(|n| items.iter().find(|i| i.path == pkg.item_path(n)))
        .map(|i| (i.id.clone(), i.path.clone()));
    let ncx = match existing_ncx {
        Some(x) => x,
        None => {
            let path = unique_path(&out, &opf_dir, "toc.ncx");
            let id = fresh_id(&seen_ids, "ncx");
            seen_ids.insert(id.clone());
            out.set_string(&path, &ncx_document(&md.title, &uid_text, &toc, &path));
            items.push(Item {
                id: id.clone(),
                path: path.clone(),
                media: "application/x-dtbncx+xml".into(),
                props: vec![],
            });
            (id, path)
        }
    };
    if toc_was_empty && !toc.is_empty() {
        // existing navigation documents are empty: fill them from the spine
        out.set_string(&ncx.1, &ncx_document(&md.title, &uid_text, &toc, &ncx.1));
        if let Some(nav) = items.iter().find(|i| i.props.iter().any(|p| p == "nav")) {
            out.set_string(&nav.path, &nav_document(&md.title, &toc, &nav.path));
        }
    }
    if v3 && !items.iter().any(|i| i.props.iter().any(|p| p == "nav")) {
        let path = unique_path(&out, &opf_dir, "nav.xhtml");
        let id = fresh_id(&seen_ids, "nav");
        seen_ids.insert(id.clone());
        out.set_string(&path, &nav_document(&md.title, &toc, &path));
        items.push(Item {
            id,
            path,
            media: "application/xhtml+xml".into(),
            props: vec!["nav".into()],
        });
    }

    // --- spine -----------------------------------------------------------------------------
    let mut spine: Vec<(String, bool)> = Vec::new();
    for s in &pkg.spine {
        if let Some(id) = final_id(&s.idref) {
            spine.push((id, s.linear));
        }
    }

    // --- package document ----------------------------------------------------------------------
    let lang = if md.language.is_empty() {
        "en"
    } else {
        &md.language
    };
    let title = if md.title.is_empty() {
        "Untitled"
    } else {
        &md.title
    };
    let mut m = String::new();
    m.push_str(&format!(
        "    <dc:identifier id=\"bookid\">{}</dc:identifier>\n",
        xml_escape(&uid_text)
    ));
    let mut seen_ident = HashSet::new();
    seen_ident.insert(uid_text.clone());
    for i in &md.identifiers {
        let t = ident_text(&i.scheme, &i.value);
        if seen_ident.insert(t.clone()) {
            m.push_str(&format!(
                "    <dc:identifier>{}</dc:identifier>\n",
                xml_escape(&t)
            ));
        }
    }
    m.push_str(&format!("    <dc:title>{}</dc:title>\n", xml_escape(title)));
    for (n, a) in md.authors.iter().enumerate() {
        let file_as = (n == 0 && !md.author_sort.is_empty()).then_some(md.author_sort.as_str());
        if v3 {
            m.push_str(&format!(
                "    <dc:creator id=\"creator{}\">{}</dc:creator>\n    <meta refines=\"#creator{}\" property=\"role\" scheme=\"marc:relators\">aut</meta>\n",
                n + 1,
                xml_escape(a),
                n + 1
            ));
            if let Some(f) = file_as {
                m.push_str(&format!(
                    "    <meta refines=\"#creator{}\" property=\"file-as\">{}</meta>\n",
                    n + 1,
                    xml_escape(f)
                ));
            }
        } else {
            let file_as = file_as
                .map(|f| format!(" opf:file-as=\"{}\"", xml_escape(f)))
                .unwrap_or_default();
            m.push_str(&format!(
                "    <dc:creator opf:role=\"aut\"{file_as}>{}</dc:creator>\n",
                xml_escape(a)
            ));
        }
    }
    m.push_str(&format!(
        "    <dc:language>{}</dc:language>\n",
        xml_escape(lang)
    ));
    if !md.publisher.is_empty() {
        m.push_str(&format!(
            "    <dc:publisher>{}</dc:publisher>\n",
            xml_escape(&md.publisher)
        ));
    }
    if !md.description.is_empty() {
        m.push_str(&format!(
            "    <dc:description>{}</dc:description>\n",
            xml_escape(&md.description)
        ));
    }
    for t in &md.tags {
        m.push_str(&format!("    <dc:subject>{}</dc:subject>\n", xml_escape(t)));
    }
    if !md.published.is_empty() {
        m.push_str(&format!(
            "    <dc:date>{}</dc:date>\n",
            xml_escape(&md.published)
        ));
    }
    if !md.rights.is_empty() {
        m.push_str(&format!(
            "    <dc:rights>{}</dc:rights>\n",
            xml_escape(&md.rights)
        ));
    }
    if !md.series.is_empty() {
        m.push_str(&format!(
            "    <meta name=\"calibre:series\" content=\"{}\"/>\n    <meta name=\"calibre:series_index\" content=\"{}\"/>\n",
            xml_escape(&md.series),
            bookx_core::opf::format_index(md.series_index)
        ));
        if v3 {
            m.push_str(&format!(
                "    <meta property=\"belongs-to-collection\" id=\"series1\">{}</meta>\n    <meta refines=\"#series1\" property=\"collection-type\">series</meta>\n    <meta refines=\"#series1\" property=\"group-position\">{}</meta>\n",
                xml_escape(&md.series),
                bookx_core::opf::format_index(md.series_index)
            ));
        }
    }
    if !md.title_sort.is_empty() {
        m.push_str(&format!(
            "    <meta name=\"calibre:title_sort\" content=\"{}\"/>\n",
            xml_escape(&md.title_sort)
        ));
    }
    if md.rating > 0 {
        m.push_str(&format!(
            "    <meta name=\"calibre:rating\" content=\"{}\"/>\n",
            md.rating
        ));
    }
    if let Some(cid) = &cover_id {
        m.push_str(&format!(
            "    <meta name=\"cover\" content=\"{}\"/>\n",
            xml_escape(cid)
        ));
    }
    if v3 {
        m.push_str(&format!(
            "    <meta property=\"dcterms:modified\">{}</meta>\n",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
        ));
    }

    let mut man = String::new();
    for it in &items {
        let props = if it.props.is_empty() {
            String::new()
        } else {
            format!(" properties=\"{}\"", xml_escape(&it.props.join(" ")))
        };
        man.push_str(&format!(
            "    <item id=\"{}\" href=\"{}\" media-type=\"{}\"{props}/>\n",
            xml_escape(&it.id),
            xml_escape(&relative_href(&opf_path, &it.path)),
            xml_escape(&it.media)
        ));
    }
    let mut sp = String::new();
    for (id, linear) in &spine {
        let lin = if *linear { "" } else { " linear=\"no\"" };
        sp.push_str(&format!(
            "    <itemref idref=\"{}\"{lin}/>\n",
            xml_escape(id)
        ));
    }
    let opf = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"{}\" unique-identifier=\"bookid\">\n  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:opf=\"http://www.idpf.org/2007/opf\">\n{m}  </metadata>\n  <manifest>\n{man}  </manifest>\n  <spine toc=\"{}\">\n{sp}  </spine>\n</package>\n",
        version.as_str(),
        xml_escape(&ncx.0)
    );
    out.set_string(&opf_path, &opf);
    out.set_string(
        "META-INF/container.xml",
        &format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n  <rootfiles>\n    <rootfile full-path=\"{}\" media-type=\"application/oebps-package+xml\"/>\n  </rootfiles>\n</container>\n",
            xml_escape(&opf_path)
        ),
    );
    Ok(out)
}
