//! XHTML content documents, the EPUB 3 nav document, the EPUB 2 NCX and SVG documents.

use std::collections::{HashMap, HashSet};

use roxmltree::{Document, Node};

use super::package::{node_pos, parse_prefixes, parse_xml, prepare_xml, RESERVED_PREFIXES};
use super::{add_ref, css, Cx, Features, Item, Pos, RefKind};

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const MATHML_NS: &str = "http://www.w3.org/1998/Math/MathML";
const OPS_NS: &str = "http://www.idpf.org/2007/ops";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// Elements that EPUB 2 (XHTML 1.1 based OPS) does not know.
const FORBIDDEN_IN_EPUB2: &[&str] = &[
    "script",
    "iframe",
    "canvas",
    "audio",
    "video",
    "embed",
    "source",
    "track",
    "section",
    "article",
    "aside",
    "nav",
    "header",
    "footer",
    "figure",
    "figcaption",
    "main",
    "mark",
    "details",
    "summary",
    "template",
    "dialog",
    "picture",
    "ruby",
    "rt",
    "rp",
    "bdi",
    "data",
    "time",
    "wbr",
];

/// The structural semantics vocabulary of EPUB 3 (plus the common DPUB-ARIA terms).
const EPUB_TYPES: &[&str] = &[
    "abstract",
    "acknowledgments",
    "afterword",
    "answer",
    "answers",
    "appendix",
    "assessment",
    "assessments",
    "backlink",
    "backmatter",
    "balloon",
    "biblioentry",
    "bibliography",
    "biblioref",
    "bodymatter",
    "bridgehead",
    "case-study",
    "chapter",
    "colophon",
    "concluding-sentence",
    "conclusion",
    "contributors",
    "copyright-page",
    "cover",
    "covertitle",
    "credit",
    "credits",
    "dedication",
    "endnote",
    "endnotes",
    "epigraph",
    "epilogue",
    "errata",
    "example",
    "figure",
    "fill-in-the-blank-problem",
    "footnote",
    "footnotes",
    "foreword",
    "frontmatter",
    "fulltitle",
    "general-problem",
    "glossary",
    "glossdef",
    "glossref",
    "glossterm",
    "halftitle",
    "help",
    "imprint",
    "imprimatur",
    "index",
    "index-editor-note",
    "index-entry",
    "index-entry-list",
    "index-group",
    "index-headnotes",
    "index-legend",
    "index-locator",
    "index-locator-list",
    "index-locator-range",
    "index-term",
    "index-term-categories",
    "index-term-category",
    "index-xref-preferred",
    "index-xref-related",
    "keyword",
    "keywords",
    "label",
    "landmarks",
    "learning-objective",
    "learning-objectives",
    "learning-outcome",
    "learning-outcomes",
    "learning-resource",
    "learning-resources",
    "learning-standard",
    "learning-standards",
    "list",
    "list-item",
    "loa",
    "loi",
    "lot",
    "lov",
    "marginalia",
    "match-problem",
    "multiple-choice-problem",
    "noteref",
    "notice",
    "ordinal",
    "other-credits",
    "pagebreak",
    "page-list",
    "panel",
    "panel-group",
    "part",
    "practice",
    "practices",
    "preamble",
    "preface",
    "prologue",
    "pullquote",
    "qna",
    "question",
    "rearnote",
    "rearnotes",
    "referrer",
    "revision-history",
    "seriespage",
    "sidebar",
    "subchapter",
    "subtitle",
    "table",
    "table-cell",
    "table-row",
    "text-area",
    "tip",
    "title",
    "titlepage",
    "toc",
    "toc-brief",
    "topic-sentence",
    "true-false-problem",
    "volume",
    "warning",
];

fn name<'a>(n: Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn is_html(n: Node) -> bool {
    n.tag_name().namespace() == Some(XHTML_NS)
}

fn epub_type<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    n.attribute((OPS_NS, "type"))
}

fn text_of(n: Node) -> String {
    n.descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect::<String>()
}

fn has_text(n: Node) -> bool {
    !text_of(n).trim().is_empty()
}

fn is_js_type(t: Option<&str>) -> bool {
    match t.map(|s| s.trim().to_ascii_lowercase()) {
        None => true,
        Some(t) => {
            t.is_empty() || t == "module" || t.contains("javascript") || t.contains("ecmascript")
        }
    }
}

pub(super) fn check_xhtml(cx: &mut Cx, it: &Item) {
    let path = it.path.clone();
    let text = cx.text(&path).unwrap_or_default();
    let t = prepare_xml(&text);
    let Some(doc) = parse_xml(cx, &path, &t) else {
        return;
    };
    let root = doc.root_element();
    let mut ids: HashSet<String> = HashSet::new();
    if name(root) != "html" || !is_html(root) {
        cx.emit(
            "HTM-901",
            &path,
            node_pos(&doc, root),
            format!(
                "the root element is <{}> in the namespace {:?}, expected <html> in {XHTML_NS}",
                name(root),
                root.tag_name().namespace().unwrap_or("")
            ),
        );
        for n in doc.descendants().filter(|n| n.is_element()) {
            if let Some(id) = n.attribute("id") {
                ids.insert(id.to_string());
            }
        }
        cx.ids.insert(path, ids);
        return;
    }
    let head = root
        .children()
        .find(|n| n.is_element() && name(*n) == "head");
    match head {
        None => cx.emit(
            "HTM-903",
            &path,
            node_pos(&doc, root),
            "the document has no <head>",
        ),
        Some(h) => {
            if !h.children().any(|n| n.is_element() && name(n) == "title") {
                cx.emit(
                    "HTM-902",
                    &path,
                    node_pos(&doc, h),
                    "the <head> has no <title>",
                );
            }
        }
    }
    if root.attribute("lang").is_none() && root.attribute((XML_NS, "lang")).is_none() {
        cx.emit(
            "ACC-007",
            &path,
            node_pos(&doc, root),
            "the document declares no language (lang / xml:lang)",
        );
    }
    let mut prefixes = parse_prefixes(
        root.attribute((OPS_NS, "prefix"))
            .or(root.attribute("prefix"))
            .unwrap_or(""),
    );
    prefixes.extend(RESERVED_PREFIXES.iter().map(|s| s.to_string()));

    let mut feat = Features::default();
    let mut idrefs: Vec<(String, Pos)> = Vec::new();
    let mut styles: Vec<(String, u32)> = Vec::new();
    let epub2 = cx.major == 2;

    for n in doc.descendants().filter(|n| n.is_element()) {
        let ns = n.tag_name().namespace();
        let nm = name(n);
        let pos = node_pos(&doc, n);
        for key in ["id"] {
            if let Some(id) = n.attribute(key) {
                if !ids.insert(id.to_string()) {
                    cx.emit(
                        "HTM-905",
                        &path,
                        pos,
                        format!("the id \"{id}\" is used more than once"),
                    );
                }
            }
        }
        if let Some(id) = n.attribute((XML_NS, "id")) {
            ids.insert(id.to_string());
        }
        if ns == Some(XHTML_NS) && nm == "a" {
            if let Some(nmv) = n.attribute("name") {
                ids.insert(nmv.to_string());
            }
        }

        // ---- features
        if ns == Some(SVG_NS)
            && nm == "svg"
            && n.parent()
                .is_none_or(|p| p.tag_name().namespace() != Some(SVG_NS))
        {
            feat.svg = true;
            if epub2 {
                cx.emit(
                    "HTM-906",
                    &path,
                    pos,
                    "inline <svg> is not allowed in EPUB 2",
                );
            }
        }
        if ns == Some(MATHML_NS) && nm == "math" {
            feat.mathml = true;
            if epub2 {
                cx.emit("HTM-906", &path, pos, "<math> is not allowed in EPUB 2");
            }
        }
        if ns == Some(OPS_NS) && nm == "switch" {
            feat.switch = true;
        }
        if (ns == Some(XHTML_NS) || ns == Some(SVG_NS))
            && nm == "script"
            && is_js_type(n.attribute("type"))
        {
            feat.scripted = true;
        }
        if ns == Some(XHTML_NS)
            && n.attributes().any(|a| {
                a.namespace().is_none() && a.name().starts_with("on") && a.name().len() > 2
            })
        {
            feat.scripted = true;
        }
        if epub2 && ns == Some(XHTML_NS) && FORBIDDEN_IN_EPUB2.contains(&nm) {
            cx.emit(
                "HTM-906",
                &path,
                pos,
                format!("the element <{nm}> is not allowed in EPUB 2"),
            );
        }

        // ---- epub:type
        if let Some(ty) = epub_type(n) {
            for tok in ty.split_whitespace() {
                if let Some((prefix, _)) = tok.split_once(':') {
                    if !prefixes.contains(prefix) {
                        cx.emit(
                            "OPF-028",
                            &path,
                            pos,
                            format!("the prefix \"{prefix}\" of the epub:type value \"{tok}\" is not declared"),
                        );
                    }
                } else if !EPUB_TYPES.contains(&tok) {
                    cx.emit(
                        "OPF-088",
                        &path,
                        pos,
                        format!("the epub:type value \"{tok}\" is not in the structural semantics vocabulary"),
                    );
                }
            }
        }

        // ---- images
        if ns == Some(XHTML_NS) && nm == "img" && n.attribute("alt").is_none() {
            let in_figure = n.ancestors().any(|a| {
                is_html(a)
                    && name(a) == "figure"
                    && a.children()
                        .any(|c| c.is_element() && name(c) == "figcaption")
            });
            if !in_figure || epub2 {
                cx.emit("HTM-907", &path, pos, "<img> has no alt attribute");
            }
        }
        if ns == Some(XHTML_NS)
            && nm == "a"
            && n.attribute("href").is_some()
            && !has_text(n)
            && n.attribute("aria-label").is_none()
            && n.attribute("title").is_none()
            && !n.descendants().any(|d| {
                d.is_element()
                    && name(d) == "img"
                    && d.attribute("alt").is_some_and(|a| !a.trim().is_empty())
            })
        {
            cx.emit("ACC-004", &path, pos, "the link has no text");
        }

        // ---- id references
        for attr in [
            "aria-labelledby",
            "aria-describedby",
            "aria-controls",
            "aria-owns",
            "aria-flowto",
            "headers",
        ] {
            if let Some(v) = n.attribute(attr) {
                for id in v.split_whitespace() {
                    idrefs.push((id.to_string(), pos));
                }
            }
        }
        if nm == "label" && ns == Some(XHTML_NS) {
            if let Some(v) = n.attribute("for") {
                idrefs.push((v.trim().to_string(), pos));
            }
        }

        // ---- references
        let reference = |attr: &str, kind: RefKind, cx: &mut Cx, feat: &mut Features| {
            let v = if let Some(v) = n.attribute(attr) {
                Some(v)
            } else if attr == "href" {
                n.attribute((XLINK_NS, "href"))
            } else {
                None
            };
            let Some(v) = v else { return };
            let remote = add_ref(cx, &path, pos, v, kind);
            if remote {
                match kind {
                    RefKind::Script => cx.emit(
                        "RSC-006",
                        &path,
                        pos,
                        format!("the remote script \"{v}\" is not allowed"),
                    ),
                    RefKind::Resource | RefKind::Style => feat.remote = true,
                    RefKind::Link => {}
                }
            }
        };
        match (ns, nm) {
            (Some(XHTML_NS), "a") | (Some(XHTML_NS), "area") => {
                reference("href", RefKind::Link, cx, &mut feat)
            }
            (Some(XHTML_NS), "link") => {
                let kind = if n
                    .attribute("rel")
                    .is_some_and(|r| r.to_ascii_lowercase().contains("stylesheet"))
                {
                    RefKind::Style
                } else {
                    RefKind::Resource
                };
                reference("href", kind, cx, &mut feat)
            }
            (Some(XHTML_NS), "img")
            | (Some(XHTML_NS), "audio")
            | (Some(XHTML_NS), "video")
            | (Some(XHTML_NS), "source")
            | (Some(XHTML_NS), "track")
            | (Some(XHTML_NS), "embed")
            | (Some(XHTML_NS), "iframe") => {
                reference("src", RefKind::Resource, cx, &mut feat);
                if nm == "video" {
                    reference("poster", RefKind::Resource, cx, &mut feat);
                }
            }
            (Some(XHTML_NS), "object") => reference("data", RefKind::Resource, cx, &mut feat),
            (Some(XHTML_NS), "script") => reference("src", RefKind::Script, cx, &mut feat),
            (Some(SVG_NS), "image") => reference("href", RefKind::Resource, cx, &mut feat),
            (Some(SVG_NS), "use") => reference("href", RefKind::Link, cx, &mut feat),
            (Some(SVG_NS), "a") => reference("href", RefKind::Link, cx, &mut feat),
            _ => {}
        }
        if ns == Some(XHTML_NS) && nm == "style" {
            styles.push((text_of(n), pos.map(|p| p.0).unwrap_or(1)));
        }
    }
    for (id, pos) in idrefs {
        if !ids.contains(&id) {
            cx.emit(
                "HTM-909",
                &path,
                pos,
                format!("the id \"{id}\" referenced here does not exist in the document"),
            );
        }
    }
    cx.ids.insert(path.clone(), ids);
    if cx.opts.extended {
        super::extra::check_doc(cx, &path, &doc);
    }
    for (css_text, line) in styles {
        let mut f = Features::default();
        css::scan(cx, &path, &css_text, line.saturating_sub(1), &mut f);
        feat.remote |= f.remote;
    }
    cx.features.insert(path.clone(), feat);
    if it.has_prop("nav") {
        check_nav(cx, &doc, &path);
    }
}

fn check_nav(cx: &mut Cx, doc: &Document, path: &str) {
    let toc = doc.descendants().find(|n| {
        n.is_element()
            && is_html(*n)
            && name(*n) == "nav"
            && epub_type(*n).is_some_and(|t| t.split_whitespace().any(|x| x == "toc"))
    });
    let Some(toc) = toc else {
        cx.emit(
            "NAV-901",
            path,
            node_pos(doc, doc.root_element()),
            "the nav document has no <nav epub:type=\"toc\">",
        );
        return;
    };
    let mut hrefs: Vec<String> = Vec::new();
    let mut lists = 0;
    for c in toc.children().filter(|c| c.is_element()) {
        match (is_html(c), name(c)) {
            (true, "ol") => {
                lists += 1;
                check_list(cx, doc, path, c, &mut hrefs, 0);
            }
            (true, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {}
            _ => cx.emit(
                "NAV-001",
                path,
                node_pos(doc, c),
                format!("<{}> is not allowed directly inside the toc nav", name(c)),
            ),
        }
    }
    if lists != 1 {
        cx.emit(
            "NAV-001",
            path,
            node_pos(doc, toc),
            format!("the toc nav must contain exactly one <ol>, found {lists}"),
        );
    }
    // reading order
    let index: HashMap<&str, usize> = cx
        .spine_paths
        .iter()
        .enumerate()
        .map(|(i, p)| (p.as_str(), i))
        .collect();
    let mut last = 0usize;
    for h in &hrefs {
        if let super::Target::Local(p, _) = super::resolve(path, h) {
            if let Some(&i) = index.get(p.as_str()) {
                if i < last {
                    cx.emit(
                        "NAV-011",
                        path,
                        node_pos(doc, toc),
                        format!("\"{h}\" comes after a link to a later spine item: the table of contents is not in reading order"),
                    );
                    break;
                }
                last = i;
            }
        }
    }
}

fn check_list(
    cx: &mut Cx,
    doc: &Document,
    path: &str,
    ol: Node,
    hrefs: &mut Vec<String>,
    depth: usize,
) {
    if depth > 100 {
        cx.emit(
            "NAV-001",
            path,
            node_pos(doc, ol),
            "the navigation list is nested too deeply",
        );
        return;
    }
    let lis: Vec<Node> = ol.children().filter(|c| c.is_element()).collect();
    if lis.is_empty() {
        cx.emit("NAV-001", path, node_pos(doc, ol), "the <ol> has no <li>");
    }
    for li in lis {
        if !(is_html(li) && name(li) == "li") {
            cx.emit(
                "NAV-001",
                path,
                node_pos(doc, li),
                format!("<{}> is not allowed inside <ol>, only <li>", name(li)),
            );
            continue;
        }
        let kids: Vec<Node> = li.children().filter(|c| c.is_element()).collect();
        let Some(first) = kids.first() else {
            cx.emit("NAV-001", path, node_pos(doc, li), "the <li> is empty");
            continue;
        };
        match (is_html(*first), name(*first)) {
            (true, "a") => {
                if !has_text(*first) {
                    cx.emit(
                        "NAV-001",
                        path,
                        node_pos(doc, *first),
                        "the link in the navigation list has no text",
                    );
                }
                match first.attribute("href") {
                    Some(h) => hrefs.push(h.to_string()),
                    None => cx.emit(
                        "NAV-001",
                        path,
                        node_pos(doc, *first),
                        "the link in the navigation list has no href",
                    ),
                }
            }
            (true, "span") => {
                if !has_text(*first) {
                    cx.emit(
                        "NAV-001",
                        path,
                        node_pos(doc, *first),
                        "the heading <span> in the navigation list has no text",
                    );
                }
                if !kids.iter().skip(1).any(|k| is_html(*k) && name(*k) == "ol") {
                    cx.emit(
                        "NAV-001",
                        path,
                        node_pos(doc, li),
                        "a <span> entry must be followed by a nested <ol>",
                    );
                }
            }
            _ => cx.emit(
                "NAV-001",
                path,
                node_pos(doc, *first),
                format!(
                    "an <li> must start with <a> or <span>, found <{}>",
                    name(*first)
                ),
            ),
        }
        for k in kids.iter().skip(1) {
            if is_html(*k) && name(*k) == "ol" {
                check_list(cx, doc, path, *k, hrefs, depth + 1);
            } else {
                cx.emit(
                    "NAV-001",
                    path,
                    node_pos(doc, *k),
                    format!("<{}> is not allowed here, only a nested <ol>", name(*k)),
                );
            }
        }
    }
}

pub(super) fn check_ncx(cx: &mut Cx, it: &Item) {
    let path = it.path.clone();
    let text = cx.text(&path).unwrap_or_default();
    let Some(doc) = parse_xml(cx, &path, &text) else {
        return;
    };
    let root = doc.root_element();
    // dtb:uid
    let uid = doc
        .descendants()
        .find(|n| n.is_element() && name(*n) == "meta" && n.attribute("name") == Some("dtb:uid"))
        .and_then(|n| n.attribute("content"));
    match (uid, cx.identifier.clone()) {
        (None, _) => cx.emit(
            "NCX-002",
            &path,
            node_pos(&doc, root),
            "the NCX has no dtb:uid meta element",
        ),
        (Some(u), Some(id)) if u.trim() != id.trim() => cx.emit(
            "NCX-001",
            &path,
            node_pos(&doc, root),
            format!(
                "dtb:uid \"{}\" differs from the package identifier \"{}\"",
                u.trim(),
                id.trim()
            ),
        ),
        _ => {}
    }
    let nav_map = root
        .children()
        .find(|n| n.is_element() && name(*n) == "navMap");
    let Some(nav_map) = nav_map else {
        cx.emit(
            "NCX-901",
            &path,
            node_pos(&doc, root),
            "the NCX has no <navMap>",
        );
        return;
    };
    let mut orders: HashMap<String, String> = HashMap::new();
    for np in nav_map
        .descendants()
        .filter(|n| n.is_element() && name(*n) == "navPoint")
    {
        let pos = node_pos(&doc, np);
        let src = np
            .children()
            .find(|c| c.is_element() && name(*c) == "content")
            .and_then(|c| c.attribute("src"))
            .unwrap_or("")
            .to_string();
        let label_ok = np
            .children()
            .find(|c| c.is_element() && name(*c) == "navLabel")
            .is_some_and(|l| {
                l.children()
                    .any(|t| t.is_element() && name(t) == "text" && has_text(t))
            });
        if src.is_empty() || !label_ok {
            cx.emit(
                "NCX-904",
                &path,
                pos,
                "a navPoint needs a navLabel with text and a content element with a src",
            );
        }
        match np.attribute("playOrder") {
            None => cx.emit("NCX-902", &path, pos, "the navPoint has no playOrder"),
            Some(po) => {
                if let Some(prev) = orders.get(po.trim()) {
                    if *prev != src {
                        cx.emit(
                            "NCX-903",
                            &path,
                            pos,
                            format!("playOrder {po} is used for \"{prev}\" and \"{src}\""),
                        );
                    }
                } else {
                    orders.insert(po.trim().to_string(), src.clone());
                }
            }
        }
    }
    for c in doc
        .descendants()
        .filter(|n| n.is_element() && name(*n) == "content")
    {
        if let Some(src) = c.attribute("src") {
            if !src.is_empty() {
                add_ref(cx, &path, node_pos(&doc, c), src, RefKind::Link);
            }
        }
    }
}

pub(super) fn check_svg(cx: &mut Cx, it: &Item) {
    let path = it.path.clone();
    let text = cx.text(&path).unwrap_or_default();
    let Some(doc) = parse_xml(cx, &path, &text) else {
        return;
    };
    let mut ids = HashSet::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        if let Some(id) = n.attribute("id") {
            ids.insert(id.to_string());
        }
        if n.tag_name().namespace() == Some(SVG_NS) && name(n) == "image" {
            if let Some(h) = n.attribute("href").or(n.attribute((XLINK_NS, "href"))) {
                add_ref(cx, &path, node_pos(&doc, n), h, RefKind::Resource);
            }
        }
    }
    cx.ids.insert(path, ids);
}
