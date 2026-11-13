//! Real page numbers of an EPUB (R24, R42): reads the `page-list` of the navigation document and
//! turns its targets into a `bookx_text::pages::PageMap`. Offsets are byte positions in the
//! chapter's source, the same unit as the spine weights (file sizes).

use bookx_text::pages::{PageMap, PageSource, PageTarget, SpineItem};

use crate::core::book::OpenBook;
use crate::core::util::{parent_dir, resolve_href};

/// One entry of the page-list: the printed label and where it points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    /// Archive path of the chapter.
    pub path: String,
    pub fragment: Option<String>,
}

/// The entries of `<nav epub:type="page-list">` in a navigation document at `nav_path`.
pub fn parse_nav(source: &str, nav_path: &str) -> Vec<Entry> {
    let Ok(doc) = roxmltree::Document::parse(source) else {
        return Vec::new();
    };
    let base = parent_dir(nav_path).to_string();
    let is_page_list = |n: &roxmltree::Node| {
        n.is_element()
            && n.tag_name().name() == "nav"
            && n.attributes().any(|a| {
                a.name() == "type" && a.value().split_whitespace().any(|v| v == "page-list")
            })
    };
    let Some(nav) = doc.descendants().find(is_page_list) else {
        return Vec::new();
    };
    nav.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "a")
        .filter_map(|a| {
            let href = a.attribute("href")?;
            let label = a
                .descendants()
                .filter(|d| d.is_text())
                .filter_map(|d| d.text())
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if label.is_empty() {
                return None;
            }
            let (path, fragment) = resolve_href(&base, href);
            Some(Entry {
                label,
                path,
                fragment,
            })
        })
        .collect()
}

/// Byte offset of the element with this `id` (or `name`) in a chapter's source; 0 when it is
/// not found, so a broken target still lands in the right chapter.
pub fn fragment_offset(source: &str, fragment: &str) -> usize {
    for attr in ["id", "name"] {
        for q in ['"', '\''] {
            let needle = format!("{attr}={q}{fragment}{q}");
            let mut from = 0;
            while let Some(i) = source[from..].find(&needle) {
                let at = from + i;
                // must be a real attribute, not the tail of another one (`data-id=`)
                let before = source[..at].chars().next_back();
                if before.is_some_and(|c| c.is_whitespace()) {
                    return source[..at].rfind('<').unwrap_or(at);
                }
                from = at + needle.len();
            }
        }
    }
    0
}

/// A printed page break of the book: its label and the chapter (spine index) and anchor it is at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub label: String,
    pub spine: usize,
    pub fragment: Option<String>,
}

/// The page-list entries that point into the spine, in book order.
pub fn marks(book: &OpenBook) -> Vec<Mark> {
    let Some(nav) = book.package.nav_item() else {
        return Vec::new();
    };
    let nav_path = book.package.item_path(nav);
    let Some(src) = book.archive.get_string(&nav_path) else {
        return Vec::new();
    };
    let spine = book.spine_paths();
    let mut out: Vec<Mark> = parse_nav(&src, &nav_path)
        .into_iter()
        .filter_map(|e| {
            Some(Mark {
                label: e.label,
                spine: spine.iter().position(|p| *p == e.path)?,
                fragment: e.fragment,
            })
        })
        .collect();
    out.sort_by_key(|m| m.spine);
    out
}

/// The page map of a book: the page-list when it has one, else `None` (the reader keeps its own
/// layout-based numbers).
pub fn page_map(book: &OpenBook) -> Option<PageMap> {
    let marks = marks(book);
    if marks.is_empty() {
        return None;
    }
    let weights = book.spine_weights();
    let mut targets: Vec<Vec<PageTarget>> = vec![Vec::new(); weights.len()];
    let mut sources: std::collections::HashMap<usize, String> = Default::default();
    let spine = book.spine_paths();
    for m in &marks {
        let off = match &m.fragment {
            Some(f) => {
                let src = sources
                    .entry(m.spine)
                    .or_insert_with(|| book.chapter_source(&spine[m.spine]).unwrap_or_default());
                fragment_offset(src, f) as f64
            }
            None => 0.0,
        };
        targets[m.spine].push(PageTarget::new(m.label.clone(), off));
    }
    let items: Vec<SpineItem> = weights
        .iter()
        .zip(targets)
        .map(|(w, t)| SpineItem::new(*w).with_targets(t))
        .collect();
    let map = PageMap::new(&items, 0.0, PageSource::PageList);
    (map.total_pages() > 0).then_some(map)
}

/// The label of the page shown, given where each break of the current chapter is on the page
/// (`positions`: fraction of the chapter, with the label) and the page's start and end. The
/// break on the page wins; otherwise the last one before it (in an earlier chapter, `before`).
pub fn label_for(
    before: Option<&str>,
    positions: &[(f64, String)],
    page_end: f64,
) -> Option<String> {
    positions
        .iter()
        .filter(|(f, _)| *f <= page_end)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, l)| l.clone())
        .or_else(|| before.map(String::from))
        .or_else(|| positions.first().map(|(_, l)| l.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAV: &str = r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body>
<nav epub:type="toc"><ol><li><a href="c1.xhtml">One</a></li></ol></nav>
<nav epub:type="page-list" hidden=""><ol>
<li><a href="text/c1.xhtml#p1">i</a></li>
<li><a href="text/c1.xhtml#p2"> 2 </a></li>
<li><a href="text/c2.xhtml">3</a></li>
<li><a href="text/c2.xhtml#x"></a></li>
</ol></nav></body></html>"#;

    #[test]
    fn the_page_list_is_read_with_resolved_paths() {
        let e = parse_nav(NAV, "OEBPS/nav.xhtml");
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].label, "i");
        assert_eq!(e[0].path, "OEBPS/text/c1.xhtml");
        assert_eq!(e[0].fragment.as_deref(), Some("p1"));
        assert_eq!(e[1].label, "2");
        assert_eq!(e[2].fragment, None);
    }

    #[test]
    fn a_nav_without_a_page_list_or_broken_xml_gives_nothing() {
        assert!(parse_nav(
            "<html><body><nav epub:type=\"toc\"/></body></html>",
            "n.xhtml"
        )
        .is_empty());
        assert!(parse_nav("not xml at all", "n.xhtml").is_empty());
    }

    #[test]
    fn fragments_are_found_as_byte_offsets() {
        let src = "<html><body><p>one</p><span id=\"p1\"/><p>two</p><a name='x'/></body></html>";
        assert_eq!(fragment_offset(src, "p1"), src.find("<span").unwrap());
        assert_eq!(fragment_offset(src, "x"), src.find("<a name").unwrap());
        assert_eq!(fragment_offset(src, "missing"), 0);
        // data-id is not id
        assert_eq!(fragment_offset("<p data-id=\"z\"/>", "z"), 0);
    }

    #[test]
    fn the_label_is_the_last_break_on_or_before_the_end_of_the_page() {
        let pos = vec![(0.0, "i".to_string()), (0.6, "ii".to_string())];
        assert_eq!(label_for(None, &pos, 0.3).as_deref(), Some("i"));
        assert_eq!(label_for(None, &pos, 0.7).as_deref(), Some("ii"));
        assert_eq!(
            label_for(Some("9"), &[(0.5, "10".to_string())], 0.2).as_deref(),
            Some("9")
        );
        assert_eq!(
            label_for(None, &[(0.5, "10".to_string())], 0.2).as_deref(),
            Some("10")
        );
        assert_eq!(label_for(None, &[], 0.2), None);
    }

    #[test]
    fn targets_make_a_page_map_with_printed_labels() {
        let items = vec![
            SpineItem::new(1000.0).with_targets(vec![
                PageTarget::new("i", 0.0),
                PageTarget::new("ii", 400.0),
            ]),
            SpineItem::new(1000.0).with_targets(vec![PageTarget::new("1", 0.0)]),
        ];
        let map = PageMap::new(&items, 0.0, PageSource::PageList);
        assert_eq!(map.total_pages(), 3);
        assert_eq!(map.page_at(0, 500.0).unwrap().label, "ii");
        assert_eq!(map.page_at(1, 10.0).unwrap().label, "1");
        assert_eq!(map.position_of_label("1"), Some((1, 0.0)));
        assert_eq!(map.position_of_label("nope"), None);
    }
}
