//! "Insert inline Table of Contents": a readable contents page inside the spine.

use anyhow::{Result, bail};
use bookx_core::book::OpenBook;
use bookx_core::opf::SpineItem;
use bookx_core::toc::TocEntry;
use bookx_core::util::{file_name, join_path, relative_href, xml_escape};

pub const INLINE_TOC_ID: &str = "inline-toc";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocPosition {
    Start,
    /// After the first spine item when it looks like a cover, else at the start.
    AfterCover,
    End,
}

#[derive(Debug, Clone)]
pub struct InlineTocOptions {
    pub title: String,
    /// File name, created next to the package document.
    pub file_name: String,
    pub position: TocPosition,
    /// 0 = all levels.
    pub max_depth: usize,
    /// Archive path of a stylesheet to link, if any.
    pub stylesheet: Option<String>,
}

impl Default for InlineTocOptions {
    fn default() -> Self {
        Self {
            title: "Table of Contents".into(),
            file_name: "toc-inline.xhtml".into(),
            position: TocPosition::AfterCover,
            max_depth: 0,
            stylesheet: None,
        }
    }
}

fn list(entries: &[TocEntry], from: &str, depth: usize, max: usize, indent: usize) -> String {
    if entries.is_empty() || (max != 0 && depth >= max) {
        return String::new();
    }
    let pad = " ".repeat(indent);
    let mut s = format!("{pad}<ul>\n");
    for e in entries {
        s.push_str(&format!(
            "{pad}  <li><a href=\"{}\">{}</a>",
            xml_escape(&relative_href(from, &e.href)),
            xml_escape(&e.title)
        ));
        let sub = list(&e.children, from, depth + 1, max, indent + 4);
        if sub.is_empty() {
            s.push_str("</li>\n");
        } else {
            s.push('\n');
            s.push_str(&sub);
            s.push_str(&format!("{pad}  </li>\n"));
        }
    }
    s.push_str(&format!("{pad}</ul>\n"));
    s
}

/// Builds the page for `entries` as it would be stored at `path`.
pub fn inline_toc_xhtml(
    entries: &[TocEntry],
    path: &str,
    lang: &str,
    opts: &InlineTocOptions,
) -> String {
    let css = opts
        .stylesheet
        .as_ref()
        .map(|c| {
            format!(
                "  <link rel=\"stylesheet\" type=\"text/css\" href=\"{}\"/>\n",
                xml_escape(&relative_href(path, c))
            )
        })
        .unwrap_or_default();
    let lang_attr = if lang.is_empty() {
        String::new()
    } else {
        format!(" lang=\"{0}\" xml:lang=\"{0}\"", xml_escape(lang))
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"{lang_attr}>\n<head>\n  <meta charset=\"utf-8\"/>\n  <title>{t}</title>\n{css}</head>\n<body>\n<div class=\"inline-toc\" id=\"{INLINE_TOC_ID}\">\n<h2>{t}</h2>\n{l}</div>\n</body>\n</html>\n",
        t = xml_escape(&opts.title),
        l = list(entries, path, 0, opts.max_depth, 0),
    )
}

/// Creates (or regenerates in place) the inline table of contents page and returns its path. The page is added
/// to the manifest and the spine once; running again only refreshes its content.
pub fn insert_inline_toc(book: &mut OpenBook, opts: &InlineTocOptions) -> Result<String> {
    let toc = book.toc();
    if toc.is_empty() {
        bail!("the book has no table of contents to insert");
    }
    let existing = book.package.item_by_id(INLINE_TOC_ID).cloned();
    let path = match &existing {
        Some(i) => book.package.item_path(i),
        None => {
            let mut p = join_path(book.package.base_dir(), &opts.file_name);
            let mut n = 1;
            while book.archive.contains(&p) {
                n += 1;
                let stem = opts
                    .file_name
                    .rsplit_once('.')
                    .map(|(s, e)| (s.to_string(), e.to_string()));
                let name = match stem {
                    Some((s, e)) => format!("{s}-{n}.{e}"),
                    None => format!("{}-{n}", file_name(&opts.file_name)),
                };
                p = join_path(book.package.base_dir(), &name);
            }
            p
        }
    };
    // The inline page itself is never part of the contents it lists.
    let entries: Vec<TocEntry> = filter_self(&toc, &path);
    let lang = book.package.metadata.language.clone();
    let xhtml = inline_toc_xhtml(&entries, &path, &lang, opts);
    book.archive.set_string(&path, &xhtml);
    if existing.is_some() {
        return Ok(path);
    }
    let pkg = book.package.clone();
    pkg.add_manifest_item(
        &mut book.archive,
        INLINE_TOC_ID,
        &path,
        "application/xhtml+xml",
        "",
    )?;
    book.reload_package()?;
    let mut spine = book.package.spine.clone();
    let index = match opts.position {
        TocPosition::Start => 0,
        TocPosition::End => spine.len(),
        TocPosition::AfterCover => {
            let looks_cover = spine.first().is_some_and(|s| {
                book.package.item_by_id(&s.idref).is_some_and(|i| {
                    i.id.to_ascii_lowercase().contains("cover")
                        || i.href.to_ascii_lowercase().contains("cover")
                })
            });
            usize::from(looks_cover)
        }
    };
    spine.insert(
        index,
        SpineItem {
            idref: INLINE_TOC_ID.into(),
            linear: true,
        },
    );
    let pkg = book.package.clone();
    pkg.set_spine(&mut book.archive, &spine)?;
    book.reload_package()?;
    Ok(path)
}

fn filter_self(entries: &[TocEntry], path: &str) -> Vec<TocEntry> {
    entries
        .iter()
        .filter(|e| e.href.split('#').next() != Some(path))
        .map(|e| TocEntry {
            title: e.title.clone(),
            href: e.href.clone(),
            children: filter_self(&e.children, path),
        })
        .collect()
}

/// Removes a previously inserted inline table of contents. Returns whether there was one.
pub fn remove_inline_toc(book: &mut OpenBook) -> Result<bool> {
    let Some(item) = book.package.item_by_id(INLINE_TOC_ID).cloned() else {
        return Ok(false);
    };
    let path = book.package.item_path(&item);
    let pkg = book.package.clone();
    pkg.remove_manifest_item(&mut book.archive, INLINE_TOC_ID)?;
    book.archive.remove(&path);
    book.reload_package()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;
    use bookx_core::check::{Severity, check_book};

    #[test]
    fn page_is_nested_and_relative() {
        let toc = vec![
            TocEntry {
                title: "A & B".into(),
                href: "OEBPS/a.xhtml".into(),
                children: vec![TocEntry {
                    title: "A1".into(),
                    href: "OEBPS/a.xhtml#one".into(),
                    children: vec![TocEntry {
                        title: "A1x".into(),
                        href: "OEBPS/x/z.xhtml".into(),
                        children: vec![],
                    }],
                }],
            },
            TocEntry {
                title: "C".into(),
                href: "OEBPS/c.xhtml".into(),
                children: vec![],
            },
        ];
        let x = inline_toc_xhtml(
            &toc,
            "OEBPS/toc-inline.xhtml",
            "en",
            &InlineTocOptions::default(),
        );
        let doc = roxmltree::Document::parse_with_options(
            &x,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .unwrap();
        let hrefs: Vec<&str> = doc
            .descendants()
            .filter(|n| n.has_tag_name("a"))
            .map(|n| n.attribute("href").unwrap())
            .collect();
        assert_eq!(
            hrefs,
            vec!["a.xhtml", "a.xhtml#one", "x/z.xhtml", "c.xhtml"]
        );
        assert_eq!(
            doc.descendants().filter(|n| n.has_tag_name("ul")).count(),
            3
        );
        assert!(x.contains("A &amp; B"));
        assert!(x.contains("lang=\"en\""));
        let shallow = inline_toc_xhtml(
            &toc,
            "OEBPS/t.xhtml",
            "",
            &InlineTocOptions {
                max_depth: 1,
                stylesheet: Some("OEBPS/css/s.css".into()),
                ..Default::default()
            },
        );
        assert_eq!(shallow.matches("<a ").count(), 2);
        assert!(shallow.contains("href=\"css/s.css\""));
        assert!(!shallow.contains("lang="));
    }

    #[test]
    fn insert_position_and_idempotence() {
        let mut b = make_book(&["<p>cover</p>", "<p>b</p>", "<p>c</p>"]);
        let p = insert_inline_toc(&mut b, &InlineTocOptions::default()).unwrap();
        assert_eq!(p, "OEBPS/toc-inline.xhtml");
        assert_eq!(
            b.spine_paths()[0],
            p,
            "first item is not a cover: inserted at the start"
        );
        assert_eq!(b.spine_paths().len(), 4);
        assert_eq!(
            b.package.item_by_id(INLINE_TOC_ID).unwrap().media_type,
            "application/xhtml+xml"
        );
        let page = b.archive.get_string(&p).unwrap();
        assert_eq!(page.matches("<a ").count(), 3);
        // second run refreshes in place
        let before_spine = b.spine_paths();
        let before_manifest = b.package.manifest.len();
        insert_inline_toc(
            &mut b,
            &InlineTocOptions {
                title: "Contents".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(b.spine_paths(), before_spine);
        assert_eq!(b.package.manifest.len(), before_manifest);
        assert!(
            b.archive
                .get_string(&p)
                .unwrap()
                .contains("<h2>Contents</h2>")
        );
        assert!(
            check_book(&b.archive)
                .iter()
                .all(|i| i.severity != Severity::Error)
        );
        assert!(remove_inline_toc(&mut b).unwrap());
        assert!(!b.archive.contains(&p));
        assert!(!remove_inline_toc(&mut b).unwrap());
        assert_eq!(b.spine_paths().len(), 3);
    }

    #[test]
    fn positions_end_and_after_cover() {
        let mut b = make_book(&["<p>a</p>", "<p>b</p>"]);
        insert_inline_toc(
            &mut b,
            &InlineTocOptions {
                position: TocPosition::End,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(b.spine_paths().last().unwrap(), "OEBPS/toc-inline.xhtml");
        let mut c = make_book(&["<p>a</p>", "<p>b</p>"]);
        let opf = c.package.opf_path.clone();
        let t = c
            .archive
            .get_string(&opf)
            .unwrap()
            .replace("id=\"ch1\"", "id=\"cover-page\"")
            .replace("idref=\"ch1\"", "idref=\"cover-page\"");
        c.archive.set_string(&opf, &t);
        c.reload_package().unwrap();
        insert_inline_toc(&mut c, &InlineTocOptions::default()).unwrap();
        assert_eq!(c.spine_paths()[1], "OEBPS/toc-inline.xhtml");
    }

    #[test]
    fn name_collision_and_empty_toc() {
        let mut b = make_book(&["<p>a</p>"]);
        b.archive.set_string("OEBPS/toc-inline.xhtml", "<html/>");
        let p = insert_inline_toc(&mut b, &InlineTocOptions::default()).unwrap();
        assert_eq!(p, "OEBPS/toc-inline-2.xhtml");
        let mut e = make_book(&["<p>a</p>"]);
        bookx_core::toc::write_toc(&mut e.archive, &e.package, &[]).unwrap();
        assert_eq!(
            insert_inline_toc(&mut e, &InlineTocOptions::default())
                .unwrap_err()
                .to_string(),
            "the book has no table of contents to insert"
        );
    }
}
