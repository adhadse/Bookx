//! Container, encryption and package document rules, and the driver that hands items to the content checkers.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use roxmltree::{Document, Node};

use super::{content, css, resolve, resolve_refs, Cx, Item, Pos, Target};

pub(super) const DC_NS: &str = "http://purl.org/dc/elements/1.1/";

/// Prefixes EPUB 3 reserves for package metadata and `epub:type`.
pub(super) const RESERVED_PREFIXES: &[&str] = &[
    "a11y",
    "dcterms",
    "epubsc",
    "marc",
    "media",
    "msv",
    "onix",
    "prism",
    "rendition",
    "schema",
    "xsd",
    "z3998",
];

const ENC_IDPF: &str = "http://www.idpf.org/2008/embedding";
const ENC_ADOBE: &str = "http://ns.adobe.com/pdf/enc#RC";

const ITEM_PROPERTIES: &[&str] = &[
    "cover-image",
    "mathml",
    "nav",
    "remote-resources",
    "scripted",
    "svg",
    "switch",
];
const ITEMREF_PROPERTIES: &[&str] = &["page-spread-left", "page-spread-right"];

pub(super) const XHTML: &str = "application/xhtml+xml";
pub(super) const NCX: &str = "application/x-dtbncx+xml";
pub(super) const CSS: &str = "text/css";
pub(super) const SVG: &str = "image/svg+xml";

static HTML_ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&([A-Za-z][A-Za-z0-9]{1,8});").expect("static regex"));
static MODIFIED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$").expect("static regex"));
static LANG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z]{2,8}(-[A-Za-z0-9]{1,8})*$").expect("static regex"));

/// Documents with an XHTML doctype may use the named entities its DTD defines; roxmltree does not load DTDs,
/// so the common ones are replaced by numeric references first.
pub(super) fn prepare_xml(text: &str) -> Cow<'_, str> {
    if !(text.contains("<!DOCTYPE") && text.contains('&')) {
        return Cow::Borrowed(text);
    }
    HTML_ENTITY.replace_all(text, |c: &regex::Captures| {
        match super::entities::lookup(&c[1]) {
            Some(n) => format!("&#{n};"),
            None => c[0].to_string(),
        }
    })
}

/// Parses `text`, reporting `RSC-016` with the position when it is not well-formed.
pub(super) fn parse_xml<'t>(cx: &mut Cx, path: &str, text: &'t str) -> Option<Document<'t>> {
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    match Document::parse_with_options(text, opts) {
        Ok(d) => Some(d),
        Err(e) => {
            let p = e.pos();
            cx.emit(
                "RSC-016",
                path,
                Some((p.row, p.col)),
                format!("the document is not well-formed: {e}"),
            );
            None
        }
    }
}

pub(super) fn node_pos(doc: &Document, n: Node) -> Pos {
    let p = doc.text_pos_at(n.range().start);
    Some((p.row, p.col))
}

fn local<'a>(n: Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

pub(super) fn parse_prefixes(value: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let toks: Vec<&str> = value.split_whitespace().collect();
    let mut i = 0;
    while i < toks.len() {
        if let Some(p) = toks[i].strip_suffix(':') {
            out.insert(p.to_string());
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

fn sniff(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn is_image(media_type: &str) -> bool {
    media_type.starts_with("image/")
}

/// Media types a spine item may have without a fallback.
fn spine_type_ok(major: u8, t: &str) -> bool {
    t == XHTML
        || t == "text/html"
        || t == "text/x-oeb1-document"
        || t == "application/x-dtbook+xml"
        || (major >= 3 && t == SVG)
}

pub(super) fn run(cx: &mut Cx) {
    if !cx.contains("META-INF/container.xml") {
        cx.emit(
            "RSC-002",
            "META-INF/container.xml",
            None,
            "the required file META-INF/container.xml is missing",
        );
        return;
    }
    let ctext = cx.text("META-INF/container.xml").unwrap_or_default();
    let ct = prepare_xml(&ctext);
    let Some(cdoc) = parse_xml(cx, "META-INF/container.xml", &ct) else {
        return;
    };
    let roots: Vec<Node> = cdoc
        .descendants()
        .filter(|n| n.is_element() && local(*n) == "rootfile")
        .collect();
    let opf_path = roots
        .iter()
        .find(|n| {
            n.attribute("media-type")
                .is_none_or(|m| m == "application/oebps-package+xml")
        })
        .and_then(|n| n.attribute("full-path"))
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    let Some(opf_path) = opf_path else {
        cx.emit(
            "RSC-003",
            "META-INF/container.xml",
            roots.first().and_then(|n| node_pos(&cdoc, *n)),
            "container.xml has no rootfile for a package document (application/oebps-package+xml)",
        );
        return;
    };
    check_encryption(cx);
    cx.opf_path = opf_path.clone();
    if !cx.contains(&opf_path) {
        cx.emit(
            "OPF-002",
            "META-INF/container.xml",
            None,
            format!("the package document \"{opf_path}\" does not exist"),
        );
        return;
    }
    check_package(cx, &opf_path);
}

fn check_encryption(cx: &mut Cx) {
    let Some(text) = cx.text("META-INF/encryption.xml") else {
        return;
    };
    let Some(doc) = parse_xml(cx, "META-INF/encryption.xml", &text) else {
        return;
    };
    for ed in doc
        .descendants()
        .filter(|n| n.is_element() && local(*n) == "EncryptedData")
    {
        let alg = ed
            .descendants()
            .find(|n| n.is_element() && local(*n) == "EncryptionMethod")
            .and_then(|n| n.attribute("Algorithm"))
            .unwrap_or("");
        let uri = ed
            .descendants()
            .find(|n| n.is_element() && local(*n) == "CipherReference")
            .and_then(|n| n.attribute("URI"))
            .unwrap_or("");
        let path = bookx_core::util::percent_decode(uri);
        if alg != ENC_IDPF && alg != ENC_ADOBE {
            cx.emit(
                "RSC-004",
                "META-INF/encryption.xml",
                node_pos(&doc, ed),
                format!("\"{path}\" is encrypted with an unsupported algorithm ({alg:?}) and cannot be checked"),
            );
            cx.encrypted.insert(path);
        }
    }
}

struct Meta {
    property: Option<String>,
    name: Option<String>,
    content: Option<String>,
    refines: bool,
    text: String,
}

#[allow(clippy::too_many_lines)]
fn check_package(cx: &mut Cx, opf_path: &str) {
    let text = cx.text(opf_path).unwrap_or_default();
    let t = prepare_xml(&text);
    let Some(doc) = parse_xml(cx, opf_path, &t) else {
        return;
    };
    let root = doc.root_element();
    if local(root) != "package" {
        cx.emit(
            "OPF-001",
            opf_path,
            node_pos(&doc, root),
            format!("the root element is <{}>, expected <package>", local(root)),
        );
        return;
    }
    let version = root.attribute("version").map(|v| v.trim().to_string());
    match version.as_deref() {
        Some("2.0") => cx.major = 2,
        Some("3.0") | Some("3.1") | Some("3.2") => cx.major = 3,
        other => {
            cx.emit(
                "OPF-001",
                opf_path,
                node_pos(&doc, root),
                match other {
                    Some(v) => format!("the package version \"{v}\" is not supported (2.0 or 3.0)"),
                    None => "the package has no version attribute".to_string(),
                },
            );
            cx.major = if other.is_some_and(|v| v.starts_with('2')) {
                2
            } else {
                3
            };
        }
    }
    cx.version = version;
    let mut prefixes = parse_prefixes(root.attribute("prefix").unwrap_or(""));
    prefixes.extend(RESERVED_PREFIXES.iter().map(|s| s.to_string()));

    let metadata = root
        .children()
        .find(|n| n.is_element() && local(*n) == "metadata");
    let manifest = root
        .children()
        .find(|n| n.is_element() && local(*n) == "manifest");
    let spine = root
        .children()
        .find(|n| n.is_element() && local(*n) == "spine");
    let guide = root
        .children()
        .find(|n| n.is_element() && local(*n) == "guide");

    // ---- metadata
    let mut identifiers: Vec<(Option<String>, String)> = Vec::new();
    let mut titles: Vec<String> = Vec::new();
    let mut languages: Vec<(String, Pos)> = Vec::new();
    let mut metas: Vec<Meta> = Vec::new();
    if let Some(md) = metadata {
        for c in md.children().filter(|n| n.is_element()) {
            let in_dc = c.tag_name().namespace() == Some(DC_NS);
            let txt = c.text().unwrap_or("").trim().to_string();
            match (in_dc, local(c)) {
                (true, "identifier") => {
                    identifiers.push((c.attribute("id").map(String::from), txt));
                }
                (true, "title") => titles.push(txt),
                (true, "language") => languages.push((txt, node_pos(&doc, c))),
                (_, "meta") => metas.push(Meta {
                    property: c.attribute("property").map(|s| s.trim().to_string()),
                    name: c.attribute("name").map(String::from),
                    content: c.attribute("content").map(String::from),
                    refines: c.attribute("refines").is_some(),
                    text: txt,
                }),
                _ => {}
            }
        }
    }
    let md_pos = metadata
        .and_then(|n| node_pos(&doc, n))
        .or(node_pos(&doc, root));
    if identifiers.iter().all(|(_, t)| t.is_empty()) {
        cx.emit(
            "OPF-901",
            opf_path,
            md_pos,
            "the package has no dc:identifier with a value",
        );
    }
    match root.attribute("unique-identifier") {
        None => cx.emit(
            "OPF-048",
            opf_path,
            node_pos(&doc, root),
            "the package has no unique-identifier attribute",
        ),
        Some(uid) => match identifiers
            .iter()
            .find(|(id, _)| id.as_deref() == Some(uid))
        {
            None => cx.emit(
                "OPF-030",
                opf_path,
                node_pos(&doc, root),
                format!("unique-identifier \"{uid}\" does not refer to a dc:identifier"),
            ),
            Some((_, v)) => cx.identifier = Some(v.clone()),
        },
    }
    if titles.is_empty() {
        cx.emit("OPF-902", opf_path, md_pos, "the package has no dc:title");
    } else if titles.iter().all(|t| t.is_empty()) {
        cx.emit("OPF-055", opf_path, md_pos, "dc:title is empty");
    }
    if languages.is_empty() {
        cx.emit(
            "OPF-903",
            opf_path,
            md_pos,
            "the package has no dc:language",
        );
    }
    for (l, p) in &languages {
        if !LANG.is_match(l) {
            cx.emit(
                "OPF-092",
                opf_path,
                *p,
                format!("\"{l}\" is not a valid language tag"),
            );
        }
    }
    if cx.major >= 3 {
        match metas
            .iter()
            .find(|m| m.property.as_deref() == Some("dcterms:modified") && !m.refines)
        {
            None => cx.emit(
                "OPF-904",
                opf_path,
                md_pos,
                "the package has no dcterms:modified meta element",
            ),
            Some(m) if !MODIFIED.is_match(&m.text) => cx.emit(
                "OPF-905",
                opf_path,
                md_pos,
                format!(
                    "dcterms:modified \"{}\" is not of the form CCYY-MM-DDThh:mm:ssZ",
                    m.text
                ),
            ),
            Some(_) => {}
        }
    }

    // ---- manifest
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut by_path: HashMap<String, String> = HashMap::new();
    let mut items: Vec<Item> = Vec::new();
    if let Some(m) = manifest {
        for n in m
            .children()
            .filter(|n| n.is_element() && local(*n) == "item")
        {
            let pos = node_pos(&doc, n);
            let id = n.attribute("id").unwrap_or("").trim().to_string();
            let href = n.attribute("href").unwrap_or("").trim().to_string();
            let media_type = n.attribute("media-type").unwrap_or("").trim().to_string();
            if id.is_empty() || href.is_empty() || media_type.is_empty() {
                cx.emit(
                    "OPF-907",
                    opf_path,
                    pos,
                    format!("the manifest item \"{id}\" lacks id, href or media-type"),
                );
                continue;
            }
            if !seen_ids.insert(id.clone()) {
                cx.emit(
                    "OPF-906",
                    opf_path,
                    pos,
                    format!("the manifest id \"{id}\" is used twice"),
                );
            }
            if href.contains('#') {
                cx.emit(
                    "OPF-091",
                    opf_path,
                    pos,
                    format!("the manifest href \"{href}\" has a fragment"),
                );
            }
            let mut path = String::new();
            match resolve(opf_path, &href) {
                Target::Local(p, _) => path = p,
                Target::Remote | Target::Ignored => {}
                Target::Leaks => cx.emit(
                    "RSC-026",
                    opf_path,
                    pos,
                    format!("the manifest href \"{href}\" points outside the container"),
                ),
            }
            if !path.is_empty() {
                if path == opf_path {
                    cx.emit(
                        "OPF-099",
                        opf_path,
                        pos,
                        "the manifest lists the package document itself",
                    );
                } else if let Some(prev) = by_path.insert(path.clone(), id.clone()) {
                    cx.emit(
                        "OPF-074",
                        opf_path,
                        pos,
                        format!("\"{path}\" is declared by the items \"{prev}\" and \"{id}\""),
                    );
                }
            }
            let properties: Vec<String> = n
                .attribute("properties")
                .unwrap_or("")
                .split_whitespace()
                .map(String::from)
                .collect();
            if cx.major >= 3 {
                check_properties(cx, opf_path, pos, &properties, ITEM_PROPERTIES, &prefixes);
            }
            items.push(Item {
                id,
                href,
                path,
                media_type,
                properties,
                fallback: n.attribute("fallback").map(|s| s.trim().to_string()),
                pos,
            });
        }
    }
    cx.manifest_paths = items
        .iter()
        .filter(|i| !i.path.is_empty())
        .map(|i| i.path.clone())
        .collect();

    // existence, media types, properties versus media type
    for it in &items {
        if it.path.is_empty() {
            continue;
        }
        let Some(data) = cx.get(&it.path) else {
            cx.emit(
                "RSC-001",
                opf_path,
                it.pos,
                format!(
                    "the manifest item \"{}\" refers to \"{}\" ({}), which is not in the container",
                    it.id, it.href, it.path
                ),
            );
            continue;
        };
        if let Some(real) = sniff(data) {
            if real != it.media_type && it.media_type != "application/octet-stream" {
                cx.emit(
                    "OPF-029",
                    opf_path,
                    it.pos,
                    format!(
                        "\"{}\" is declared as {} but its content looks like {real}",
                        it.path, it.media_type
                    ),
                );
            }
        }
        if it.media_type == "text/html"
            && (it.path.ends_with(".xhtml")
                || it.path.ends_with(".html")
                || it.path.ends_with(".htm"))
        {
            cx.emit(
                "OPF-035",
                opf_path,
                it.pos,
                format!("\"{}\" is declared as text/html, XHTML content should use application/xhtml+xml", it.path),
            );
        }
        if cx.major >= 3 {
            if it.has_prop("cover-image") && !is_image(&it.media_type) {
                cx.emit(
                    "OPF-012",
                    opf_path,
                    it.pos,
                    format!(
                        "the cover-image property is used on \"{}\" ({}), which is not an image",
                        it.id, it.media_type
                    ),
                );
            }
            if it.has_prop("nav") && it.media_type != XHTML {
                cx.emit(
                    "OPF-012",
                    opf_path,
                    it.pos,
                    format!(
                        "the nav property is used on \"{}\" ({}), which is not XHTML",
                        it.id, it.media_type
                    ),
                );
            }
        }
    }
    if cx.major >= 3 {
        let navs = items.iter().filter(|i| i.has_prop("nav")).count();
        let mp = manifest.and_then(|n| node_pos(&doc, n));
        if navs == 0 {
            cx.emit(
                "OPF-912",
                opf_path,
                mp,
                "the manifest has no item with the nav property",
            );
        } else if navs > 1 {
            cx.emit(
                "OPF-913",
                opf_path,
                mp,
                format!("{navs} manifest items have the nav property, exactly one is allowed"),
            );
        }
    }

    // fallbacks
    let by_id: HashMap<&str, &Item> = {
        let mut m = HashMap::new();
        for i in &items {
            m.entry(i.id.as_str()).or_insert(i);
        }
        m
    };
    let mut broken: HashSet<String> = HashSet::new();
    let mut looped: HashSet<String> = HashSet::new();
    let mut reported_040: HashSet<String> = HashSet::new();
    for it in &items {
        let mut cur = it;
        let mut seen: HashSet<&str> = HashSet::from([it.id.as_str()]);
        while let Some(fb) = cur.fallback.as_deref() {
            let Some(next) = by_id.get(fb) else {
                broken.insert(it.id.clone());
                if reported_040.insert(cur.id.clone()) {
                    cx.emit(
                        "OPF-040",
                        opf_path,
                        cur.pos,
                        format!(
                            "the fallback \"{fb}\" of \"{}\" is not a manifest item",
                            cur.id
                        ),
                    );
                }
                break;
            };
            if !seen.insert(next.id.as_str()) {
                let first = !looped.contains(&it.id);
                looped.extend(seen.iter().map(|s| s.to_string()));
                if first {
                    cx.emit(
                        "OPF-045",
                        opf_path,
                        it.pos,
                        format!("the fallback chain of \"{}\" is circular", it.id),
                    );
                }
                break;
            }
            cur = next;
        }
    }

    // ---- spine
    let mut spine_items: Vec<&Item> = Vec::new();
    match spine {
        None => cx.emit(
            "OPF-033",
            opf_path,
            node_pos(&doc, root),
            "the package has no spine",
        ),
        Some(sp) => {
            let sp_pos = node_pos(&doc, sp);
            let toc = sp.attribute("toc");
            match toc {
                None if cx.major == 2 => cx.emit(
                    "OPF-910",
                    opf_path,
                    sp_pos,
                    "the EPUB 2 spine has no toc attribute",
                ),
                Some(t) => match by_id.get(t) {
                    Some(i) if i.media_type == NCX => {}
                    Some(i) => cx.emit(
                        "OPF-050",
                        opf_path,
                        sp_pos,
                        format!("the spine toc \"{t}\" is {}, not the NCX", i.media_type),
                    ),
                    None => cx.emit(
                        "OPF-050",
                        opf_path,
                        sp_pos,
                        format!("the spine toc \"{t}\" is not a manifest item"),
                    ),
                },
                None => {}
            }
            let mut seen: HashSet<&str> = HashSet::new();
            let mut any_linear = false;
            let mut count = 0;
            for r in sp
                .children()
                .filter(|n| n.is_element() && local(*n) == "itemref")
            {
                count += 1;
                let pos = node_pos(&doc, r);
                let linear = r.attribute("linear");
                match linear {
                    None | Some("yes") => any_linear = true,
                    Some("no") => cx.emit(
                        "OPF-918",
                        opf_path,
                        pos,
                        format!(
                            "the spine item \"{}\" is non-linear",
                            r.attribute("idref").unwrap_or("")
                        ),
                    ),
                    Some(v) => {
                        any_linear = true;
                        cx.emit(
                            "OPF-917",
                            opf_path,
                            pos,
                            format!("linear=\"{v}\" is not allowed, use yes or no"),
                        );
                    }
                }
                let idref = r.attribute("idref").unwrap_or("");
                if cx.major >= 3 {
                    let props: Vec<String> = r
                        .attribute("properties")
                        .unwrap_or("")
                        .split_whitespace()
                        .map(String::from)
                        .collect();
                    check_properties(cx, opf_path, pos, &props, ITEMREF_PROPERTIES, &prefixes);
                }
                let Some(item) = by_id.get(idref) else {
                    cx.emit(
                        "OPF-049",
                        opf_path,
                        pos,
                        format!("the spine itemref \"{idref}\" is not a manifest item"),
                    );
                    continue;
                };
                if !seen.insert(idref) {
                    cx.emit(
                        "OPF-034",
                        opf_path,
                        pos,
                        format!("the spine item \"{idref}\" appears more than once"),
                    );
                    continue;
                }
                if !broken.contains(&item.id)
                    && !looped.contains(&item.id)
                    && !chain_has_ok_type(cx.major, item, &by_id)
                {
                    cx.emit(
                        "OPF-042",
                        opf_path,
                        pos,
                        format!(
                            "the spine item \"{idref}\" has the media type {}, which is not allowed in the spine, and no fallback leads to a content document",
                            item.media_type
                        ),
                    );
                }
                spine_items.push(item);
            }
            if count == 0 || !any_linear {
                cx.emit(
                    "OPF-033",
                    opf_path,
                    sp_pos,
                    "the spine has no linear itemref",
                );
            }
        }
    }
    cx.spine_paths = spine_items.iter().map(|i| i.path.clone()).collect();

    // ---- cover (EPUB 2 meta) and guide
    if let Some(m) = metas
        .iter()
        .find(|m| m.name.as_deref() == Some("cover") && m.content.is_some())
    {
        let c = m.content.as_deref().unwrap_or("");
        cx.cover_meta = Some(c.to_string());
        match by_id.get(c) {
            Some(i) if is_image(&i.media_type) => {}
            Some(i) => cx.emit(
                "OPF-915",
                opf_path,
                md_pos,
                format!("the cover item \"{c}\" is {}, not an image", i.media_type),
            ),
            None => cx.emit(
                "OPF-915",
                opf_path,
                md_pos,
                format!("the cover meta refers to \"{c}\", which is not a manifest item"),
            ),
        }
    }
    if let Some(g) = guide {
        for r in g
            .children()
            .filter(|n| n.is_element() && local(*n) == "reference")
        {
            let href = r.attribute("href").unwrap_or("");
            if let Target::Local(p, _) = resolve(opf_path, href) {
                if !p.is_empty() && !cx.manifest_paths.contains(&p) {
                    cx.emit(
                        "OPF-031",
                        opf_path,
                        node_pos(&doc, r),
                        format!("the guide reference \"{href}\" is not in the manifest"),
                    );
                }
            }
        }
    }

    cx.items = items;
    check_items(cx);
    resolve_refs(cx);

    // ---- files that are not in the manifest
    let unmanifested: Vec<String> = cx
        .archive
        .map(|a| a.names().to_vec())
        .unwrap_or_default()
        .into_iter()
        .filter(|n| {
            n != "mimetype"
                && !n.starts_with("META-INF/")
                && n != &cx.opf_path
                && !cx.manifest_paths.contains(n)
                && !n.ends_with('/')
        })
        .collect();
    for n in unmanifested {
        cx.emit(
            "OPF-003",
            &n,
            None,
            format!("the file \"{n}\" is in the container but not declared in the manifest"),
        );
    }
}

fn chain_has_ok_type(major: u8, item: &Item, by_id: &HashMap<&str, &Item>) -> bool {
    let mut cur = item;
    for _ in 0..64 {
        if spine_type_ok(major, &cur.media_type) {
            return true;
        }
        match cur.fallback.as_deref().and_then(|f| by_id.get(f)) {
            Some(n) => cur = n,
            None => return false,
        }
    }
    false
}

/// Validates the tokens of a `properties` attribute.
fn check_properties(
    cx: &mut Cx,
    file: &str,
    pos: Pos,
    tokens: &[String],
    allowed: &[&str],
    prefixes: &HashSet<String>,
) {
    for t in tokens {
        if !t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
        {
            cx.emit(
                "OPF-026",
                file,
                pos,
                format!("the property value \"{t}\" is malformed"),
            );
            continue;
        }
        if let Some((prefix, _)) = t.split_once(':') {
            if !prefixes.contains(prefix) {
                cx.emit(
                    "OPF-028",
                    file,
                    pos,
                    format!("the prefix \"{prefix}\" of the property \"{t}\" is not declared"),
                );
            }
        } else if !allowed.contains(&t.as_str()) {
            cx.emit(
                "OPF-027",
                file,
                pos,
                format!("the property \"{t}\" is not defined here"),
            );
        }
    }
}

/// Runs the content checkers over every manifest item and compares the features they found with the declared
/// properties.
fn check_items(cx: &mut Cx) {
    let items = cx.items.clone();
    for it in &items {
        if it.path.is_empty() || !cx.contains(&it.path) || cx.encrypted.contains(&it.path) {
            continue;
        }
        match it.media_type.as_str() {
            XHTML | "text/html" => content::check_xhtml(cx, it),
            NCX => content::check_ncx(cx, it),
            CSS => css::check_css(cx, it),
            SVG => content::check_svg(cx, it),
            _ => {}
        }
    }
    if cx.major < 3 {
        return;
    }
    let opf = cx.opf_path.clone();
    for it in &items {
        let Some(f) = cx.features.get(&it.path).cloned() else {
            continue;
        };
        let is_doc = it.media_type == XHTML;
        for (needed, name) in [
            (f.scripted && is_doc, "scripted"),
            (f.svg && is_doc, "svg"),
            (f.mathml && is_doc, "mathml"),
            (f.switch && is_doc, "switch"),
            (f.remote, "remote-resources"),
        ] {
            if needed && !it.has_prop(name) {
                cx.emit(
                    "OPF-014",
                    &opf,
                    it.pos,
                    format!(
                        "\"{}\" requires the property \"{name}\" in its manifest item",
                        it.path
                    ),
                );
            }
        }
        if it.has_prop("scripted") && !f.scripted {
            cx.emit(
                "OPF-015",
                &opf,
                it.pos,
                format!(
                    "\"{}\" has the scripted property but contains no script",
                    it.path
                ),
            );
        }
        if it.has_prop("remote-resources") && !f.remote {
            cx.emit(
                "OPF-018",
                &opf,
                it.pos,
                format!(
                    "\"{}\" has the remote-resources property but uses no remote resource",
                    it.path
                ),
            );
        }
    }
}
