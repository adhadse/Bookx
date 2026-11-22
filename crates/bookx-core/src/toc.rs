//! Table of contents reading (EPUB 3 nav + EPUB 2 NCX) and writing.

use anyhow::{Result, bail};
use roxmltree::Node;

use super::archive::Archive;
use super::opf::Package;
use super::util::*;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TocEntry {
    pub title: String,
    /// Absolute archive path, optionally followed by `#fragment`.
    pub href: String,
    pub children: Vec<TocEntry>,
}

#[derive(Debug, Clone)]
pub struct FlatToc {
    pub depth: usize,
    pub title: String,
    pub href: String,
}

pub fn flatten(entries: &[TocEntry]) -> Vec<FlatToc> {
    fn walk(out: &mut Vec<FlatToc>, e: &[TocEntry], depth: usize) {
        for t in e {
            out.push(FlatToc {
                depth,
                title: t.title.clone(),
                href: t.href.clone(),
            });
            walk(out, &t.children, depth + 1);
        }
    }
    let mut out = Vec::new();
    walk(&mut out, entries, 0);
    out
}

pub fn unflatten(flat: &[FlatToc]) -> Vec<TocEntry> {
    fn build(flat: &[FlatToc], i: &mut usize, depth: usize) -> Vec<TocEntry> {
        let mut out = Vec::new();
        while *i < flat.len() {
            let f = &flat[*i];
            if f.depth < depth {
                break;
            }
            if f.depth > depth {
                // Orphaned deeper item: attach to the previous entry or promote.
                if let Some(last) = out.last_mut() {
                    let kids = build(flat, i, depth + 1);
                    let l: &mut TocEntry = last;
                    l.children.extend(kids);
                    continue;
                }
            }
            *i += 1;
            let mut e = TocEntry {
                title: f.title.clone(),
                href: f.href.clone(),
                children: Vec::new(),
            };
            e.children = build(flat, i, depth + 1);
            out.push(e);
        }
        out
    }
    let mut i = 0;
    build(flat, &mut i, 0)
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_of(n: Node) -> String {
    collapse(
        &n.descendants()
            .filter(|d| d.is_text())
            .filter_map(|d| d.text())
            .collect::<String>(),
    )
}

pub fn read_toc(archive: &Archive, pkg: &Package) -> Vec<TocEntry> {
    if let Some(nav) = pkg.nav_item() {
        let path = pkg.item_path(nav);
        if let Some(t) = archive.get_string(&path) {
            let entries = parse_nav(&path, &t);
            if !entries.is_empty() {
                return entries;
            }
        }
    }
    if let Some(ncx) = pkg.ncx_item() {
        let path = pkg.item_path(ncx);
        if let Some(t) = archive.get_string(&path) {
            return parse_ncx(&path, &t);
        }
    }
    Vec::new()
}

fn parse_nav(nav_path: &str, text: &str) -> Vec<TocEntry> {
    let Ok(doc) = parse_xml(text) else {
        return Vec::new();
    };
    let base = parent_dir(nav_path).to_string();
    let navs: Vec<Node> = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "nav")
        .collect();
    let nav = navs
        .iter()
        .find(|n| {
            n.attributes()
                .any(|a| a.name() == "type" && a.value().split_whitespace().any(|v| v == "toc"))
        })
        .or(navs.first());
    let Some(nav) = nav else { return Vec::new() };
    let Some(ol) = nav
        .children()
        .find(|c| c.is_element() && c.tag_name().name() == "ol")
    else {
        return Vec::new();
    };
    fn walk(ol: Node, base: &str) -> Vec<TocEntry> {
        let mut out = Vec::new();
        for li in ol
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "li")
        {
            let label = li
                .children()
                .find(|c| c.is_element() && matches!(c.tag_name().name(), "a" | "span"));
            let (title, href) = match label {
                Some(l) => {
                    let href = l
                        .attribute("href")
                        .map(|h| {
                            let (p, f) = resolve_href(base, h);
                            match f {
                                Some(f) => format!("{p}#{f}"),
                                None => p,
                            }
                        })
                        .unwrap_or_default();
                    (text_of(l), href)
                }
                None => (String::new(), String::new()),
            };
            let children = li
                .children()
                .find(|c| c.is_element() && c.tag_name().name() == "ol")
                .map(|o| walk(o, base))
                .unwrap_or_default();
            out.push(TocEntry {
                title,
                href,
                children,
            });
        }
        out
    }
    walk(ol, &base)
}

fn parse_ncx(ncx_path: &str, text: &str) -> Vec<TocEntry> {
    let Ok(doc) = parse_xml(text) else {
        return Vec::new();
    };
    let base = parent_dir(ncx_path).to_string();
    let Some(map) = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "navMap")
    else {
        return Vec::new();
    };
    fn walk(parent: Node, base: &str) -> Vec<TocEntry> {
        parent
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "navPoint")
            .map(|p| {
                let title = p
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "navLabel")
                    .map(text_of)
                    .unwrap_or_default();
                let href = p
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "content")
                    .and_then(|c| c.attribute("src"))
                    .map(|s| {
                        let (path, frag) = resolve_href(base, s);
                        match frag {
                            Some(f) => format!("{path}#{f}"),
                            None => path,
                        }
                    })
                    .unwrap_or_default();
                TocEntry {
                    title,
                    href,
                    children: walk(p, base),
                }
            })
            .collect()
    }
    walk(map, &base)
}

fn max_depth(e: &[TocEntry]) -> usize {
    e.iter()
        .map(|t| 1 + max_depth(&t.children))
        .max()
        .unwrap_or(0)
}

/// Gives navigation points that name the same target the same `playOrder` (the NCX rules ask for
/// it) and numbers the distinct targets 1, 2, 3 in document order.
pub fn fix_play_order(xml: &str) -> String {
    let re = regex::Regex::new(
        r#"(?s)(<navPoint[^>]*?playOrder=")\d+("[^>]*>\s*<navLabel>.*?</navLabel>\s*<content src=")([^"]*)""#,
    )
    .expect("static regex");
    let mut seen: Vec<String> = Vec::new();
    re.replace_all(xml, |c: &regex::Captures| {
        let target = c[3].to_string();
        let n = match seen.iter().position(|t| *t == target) {
            Some(i) => i + 1,
            None => {
                seen.push(target.clone());
                seen.len()
            }
        };
        format!("{}{n}{}{}\"", &c[1], &c[2], target)
    })
    .into_owned()
}

fn ncx_points(entries: &[TocEntry], ncx_path: &str, counter: &mut usize, indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let mut out = String::new();
    for e in entries {
        *counter += 1;
        out.push_str(&format!(
            "{pad}<navPoint id=\"navPoint-{n}\" playOrder=\"{n}\">\n{pad}  <navLabel><text>{t}</text></navLabel>\n{pad}  <content src=\"{s}\"/>\n",
            n = *counter,
            t = xml_escape(&e.title),
            s = xml_escape(&relative_href(ncx_path, &e.href)),
        ));
        out.push_str(&ncx_points(&e.children, ncx_path, counter, indent + 1));
        out.push_str(&format!("{pad}</navPoint>\n"));
    }
    out
}

fn nav_list(entries: &[TocEntry], nav_path: &str, indent: usize) -> String {
    let pad = "  ".repeat(indent);
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
            out.push_str(&nav_list(&e.children, nav_path, indent + 2));
            out.push_str(&format!("{pad}  </li>\n"));
        }
    }
    out.push_str(&format!("{pad}</ol>\n"));
    out
}

pub fn write_toc(archive: &mut Archive, pkg: &Package, entries: &[TocEntry]) -> Result<()> {
    let mut wrote = false;

    if let Some(ncx) = pkg.ncx_item() {
        let path = pkg.item_path(ncx);
        let old = archive.get_string(&path).unwrap_or_default();
        let uid = regex::Regex::new(r#"name="dtb:uid"\s+content="([^"]*)""#)
            .ok()
            .and_then(|re| re.captures(&old).map(|c| c[1].to_string()))
            .or_else(|| {
                pkg.metadata
                    .identifiers
                    .first()
                    .map(|i| xml_escape(&i.value))
            })
            .unwrap_or_else(|| "bookx".into());
        let mut counter = 0;
        let points = ncx_points(entries, &path, &mut counter, 2);
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n  <head>\n    <meta name=\"dtb:uid\" content=\"{uid}\"/>\n    <meta name=\"dtb:depth\" content=\"{depth}\"/>\n    <meta name=\"dtb:totalPageCount\" content=\"0\"/>\n    <meta name=\"dtb:maxPageNumber\" content=\"0\"/>\n  </head>\n  <docTitle><text>{title}</text></docTitle>\n  <navMap>\n{points}  </navMap>\n</ncx>\n",
            depth = max_depth(entries).max(1),
            title = xml_escape(&pkg.metadata.title),
        );
        archive.set_string(&path, &fix_play_order(&xml));
        wrote = true;
    }

    if let Some(nav) = pkg.nav_item() {
        let path = pkg.item_path(nav);
        if let Some(text) = archive.get_string(&path) {
            let new = {
                let doc = parse_xml(&text)?;
                let navs: Vec<Node> = doc
                    .descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "nav")
                    .collect();
                let nav_node = navs
                    .iter()
                    .find(|n| {
                        n.attributes().any(|a| {
                            a.name() == "type" && a.value().split_whitespace().any(|v| v == "toc")
                        })
                    })
                    .or(navs.first());
                nav_node.and_then(|n| {
                    n.children()
                        .find(|c| c.is_element() && c.tag_name().name() == "ol")
                        .map(|ol| {
                            let r = ol.range();
                            let line_start =
                                text[..r.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
                            let ind = r.start - line_start;
                            let list = nav_list(entries, &path, ind / 2);
                            let list = list.trim_start().trim_end_matches('\n').to_string();
                            format!("{}{}{}", &text[..r.start], list, &text[r.end..])
                        })
                })
            };
            if let Some(new) = new {
                parse_xml(&new)?;
                archive.set_string(&path, &new);
                wrote = true;
            }
        }
    }

    if !wrote {
        bail!("this book has neither an NCX nor a navigation document to update");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_roundtrip() {
        let t = vec![
            TocEntry {
                title: "A".into(),
                href: "a.xhtml".into(),
                children: vec![TocEntry {
                    title: "A1".into(),
                    href: "a.xhtml#1".into(),
                    children: vec![],
                }],
            },
            TocEntry {
                title: "B".into(),
                href: "b.xhtml".into(),
                children: vec![],
            },
        ];
        assert_eq!(unflatten(&flatten(&t)), t);
    }
}
