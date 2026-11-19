//! The book jacket (CE-426): a page near the front of the book that lists the metadata (title, authors, series,
//! tags, rating, comments, cover), generated from the package metadata. The page carries a marker so Bookx
//! finds it again to refresh it when the metadata changes, or to remove it without a trace.

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::opf::Metadata;
use bookx_core::util::{relative_href, xml_escape};
use serde::{Deserialize, Serialize};

use crate::files::{add_file, delete_files};
use crate::markup::is_markup_path;

/// The start of the marker in the page's `<head>`; its content lists the options the page was made with.
pub const MARKER: &str = "<meta name=\"bookx:jacket\"";
pub const FILE_NAME: &str = "bookx-jacket.xhtml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JacketOptions {
    pub cover: bool,
    pub series: bool,
    pub tags: bool,
    pub rating: bool,
    pub comments: bool,
    pub publisher: bool,
}

impl Default for JacketOptions {
    fn default() -> Self {
        Self {
            cover: true,
            series: true,
            tags: true,
            rating: true,
            comments: true,
            publisher: false,
        }
    }
}

impl JacketOptions {
    /// The text stored in the marker: `cover=1 series=1 ...`.
    pub fn to_marker_text(&self) -> String {
        let b = |v: bool| u8::from(v);
        format!(
            "cover={} series={} tags={} rating={} comments={} publisher={}",
            b(self.cover),
            b(self.series),
            b(self.tags),
            b(self.rating),
            b(self.comments),
            b(self.publisher)
        )
    }

    pub fn from_marker_text(t: &str) -> JacketOptions {
        let mut o = JacketOptions::default();
        for kv in t.split_whitespace() {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            let on = v == "1";
            match k {
                "cover" => o.cover = on,
                "series" => o.series = on,
                "tags" => o.tags = on,
                "rating" => o.rating = on,
                "comments" => o.comments = on,
                "publisher" => o.publisher = on,
                _ => {}
            }
        }
        o
    }
}

/// The options a jacket page was made with.
pub fn options_of(book: &OpenBook) -> Option<JacketOptions> {
    let p = find_jacket(book)?;
    let src = book.archive.get_string(&p)?;
    let at = src.find(MARKER)?;
    let tag = &src[at..src[at..].find('>').map(|e| at + e)?];
    Some(JacketOptions::from_marker_text(
        &crate::markup::tag_attr(tag, "content").unwrap_or_default(),
    ))
}

/// Stars for a rating on the 0..=10 scale (two per star): `★★★½` style, as full stars and a half.
pub fn stars(rating: u8) -> String {
    let r = rating.min(10);
    let mut s = "★".repeat((r / 2) as usize);
    if r % 2 == 1 {
        s.push('½');
    }
    s
}

fn paragraphs(text: &str) -> Vec<String> {
    let plain = crate::markup::plain_text(text);
    plain
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect()
}

/// The jacket page for `md`. `cover_src` is the cover image's path relative to the page.
pub fn jacket_xhtml(md: &Metadata, cover_src: Option<&str>, o: &JacketOptions) -> String {
    let lang = if md.language.is_empty() {
        "en"
    } else {
        &md.language
    };
    let mut body = String::new();
    if let (true, Some(src)) = (o.cover, cover_src) {
        body.push_str(&format!(
            "  <p class=\"jacket-cover\"><img src=\"{}\" alt=\"{}\"/></p>\n",
            xml_escape(src),
            xml_escape(&md.title)
        ));
    }
    body.push_str(&format!("  <h1>{}</h1>\n", xml_escape(&md.title)));
    if !md.authors.is_empty() {
        body.push_str(&format!(
            "  <p class=\"jacket-authors\">{}</p>\n",
            xml_escape(&md.authors.join(" & "))
        ));
    }
    if o.series && !md.series.trim().is_empty() {
        let idx = if md.series_index > 0.0 {
            if md.series_index.fract() == 0.0 {
                format!(" #{}", md.series_index as i64)
            } else {
                format!(" #{}", md.series_index)
            }
        } else {
            String::new()
        };
        body.push_str(&format!(
            "  <p class=\"jacket-series\">{}{}</p>\n",
            xml_escape(md.series.trim()),
            idx
        ));
    }
    if o.rating && md.rating > 0 {
        body.push_str(&format!(
            "  <p class=\"jacket-rating\" aria-label=\"{} of 5 stars\">{}</p>\n",
            f64::from(md.rating) / 2.0,
            stars(md.rating)
        ));
    }
    if o.publisher && !md.publisher.trim().is_empty() {
        body.push_str(&format!(
            "  <p class=\"jacket-publisher\">{}</p>\n",
            xml_escape(md.publisher.trim())
        ));
    }
    if o.tags && !md.tags.is_empty() {
        body.push_str(&format!(
            "  <p class=\"jacket-tags\">{}</p>\n",
            xml_escape(&md.tags.join(", "))
        ));
    }
    if o.comments {
        for p in paragraphs(&md.description) {
            body.push_str(&format!(
                "  <p class=\"jacket-comments\">{}</p>\n",
                xml_escape(&p)
            ));
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"{lang}\" lang=\"{lang}\">\n<head>\n  <title>{}</title>\n  {MARKER} content=\"{marker}\"/>\n  <style type=\"text/css\">\n    .jacket-cover img {{ max-width: 60%; height: auto; }}\n    .jacket-cover, .jacket-authors, .jacket-series, .jacket-rating, .jacket-tags, .jacket-publisher, h1 {{ text-align: center; }}\n    .jacket-series, .jacket-tags, .jacket-publisher {{ font-style: italic; }}\n  </style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        xml_escape(&md.title),
        marker = o.to_marker_text()
    )
}

/// The jacket page of the book, found by its marker.
pub fn find_jacket(book: &OpenBook) -> Option<String> {
    book.package
        .manifest
        .iter()
        .map(|i| book.package.item_path(i))
        .filter(|p| is_markup_path(p))
        .find(|p| {
            book.archive
                .get_string(p)
                .is_some_and(|s| s.contains("name=\"bookx:jacket\""))
        })
}

fn build(book: &OpenBook, page: &str, o: &JacketOptions) -> String {
    let cover = book.package.cover_path().map(|c| relative_href(page, &c));
    jacket_xhtml(book.metadata(), cover.as_deref(), o)
}

/// Adds the jacket, or refreshes the existing one in place. It goes right after the cover page (or first).
/// Returns the path and whether it was new.
pub fn add_or_update(book: &mut OpenBook, o: &JacketOptions) -> Result<(String, bool)> {
    if let Some(p) = find_jacket(book) {
        let new = build(book, &p, o);
        book.archive.set_string(&p, &new);
        return Ok((p, false));
    }
    let after = crate::cover::find_cover_page(book);
    let probe = crate::files::unique_path(book, book.package.base_dir(), FILE_NAME);
    let first = build(book, &probe, o);
    let path = add_file(book, FILE_NAME, first.into_bytes(), after.as_deref())?;
    if after.is_none() {
        crate::book_tools::spine_move_to(book, &path, 0)?;
    }
    let fixed = build(book, &path, o);
    book.archive.set_string(&path, &fixed);
    Ok((path, true))
}

/// Refreshes the jacket if there is one. Returns its path when the page changed.
pub fn refresh(book: &mut OpenBook, o: &JacketOptions) -> Result<Option<String>> {
    let Some(p) = find_jacket(book) else {
        return Ok(None);
    };
    let new = build(book, &p, o);
    if book.archive.get_string(&p).as_deref() == Some(new.as_str()) {
        return Ok(None);
    }
    book.archive.set_string(&p, &new);
    Ok(Some(p))
}

/// True when the jacket exists and no longer shows the current metadata.
pub fn is_stale(book: &OpenBook, o: &JacketOptions) -> bool {
    find_jacket(book).is_some_and(|p| {
        book.archive.get_string(&p).as_deref() != Some(build(book, &p, o).as_str())
    })
}

/// Removes the jacket page from the spine, manifest and archive. Returns the removed path.
pub fn remove(book: &mut OpenBook) -> Result<Option<String>> {
    let Some(p) = find_jacket(book) else {
        return Ok(None);
    };
    delete_files(book, std::slice::from_ref(&p))?;
    Ok(Some(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{make_book, png};

    fn book() -> OpenBook {
        let mut b = make_book(&["<p>one</p>", "<p>two</p>"]);
        let mut md = b.metadata().clone();
        md.title = "Tom & Jerry <1>".into();
        md.authors = vec!["A. Writer".into(), "B. Other".into()];
        md.series = "Cats".into();
        md.series_index = 2.0;
        md.tags = vec!["fun".into(), "cartoon".into()];
        md.rating = 7;
        md.description = "<p>First &amp; second.</p><p>Third</p>".into();
        b.set_metadata(md).unwrap();
        b
    }

    #[test]
    fn stars_use_halves() {
        assert_eq!(stars(0), "");
        assert_eq!(stars(7), "★★★½");
        assert_eq!(stars(10), "★★★★★");
        assert_eq!(stars(200), "★★★★★");
    }

    #[test]
    fn page_lists_the_metadata_and_is_well_formed() {
        let b = book();
        let x = jacket_xhtml(
            b.metadata(),
            Some("../images/c.png"),
            &JacketOptions::default(),
        );
        assert!(bookx_core::util::parse_xml(&x).is_ok(), "{x}");
        for want in [
            "Tom &amp; Jerry &lt;1&gt;",
            "A. Writer &amp; B. Other",
            "Cats #2",
            "★★★½",
            "fun, cartoon",
            "First &amp; second.",
            "../images/c.png",
            MARKER,
        ] {
            assert!(x.contains(want), "{want} missing in {x}");
        }
        let off = JacketOptions {
            cover: false,
            series: false,
            tags: false,
            rating: false,
            comments: false,
            publisher: true,
        };
        let x = jacket_xhtml(b.metadata(), Some("c.png"), &off);
        for gone in ["Cats", "★", "fun", "First", "c.png"] {
            assert!(!x.contains(gone), "{gone} should be off");
        }
    }

    #[test]
    fn add_places_it_first_marks_it_and_a_second_add_updates() {
        let mut b = book();
        let o = JacketOptions::default();
        let (p, new) = add_or_update(&mut b, &o).unwrap();
        assert!(new);
        assert_eq!(b.spine_paths()[0], p);
        assert_eq!(find_jacket(&b), Some(p.clone()));
        let n = b.archive.names().len();
        let (p2, new2) = add_or_update(&mut b, &o).unwrap();
        assert_eq!((p2, new2), (p.clone(), false));
        assert_eq!(b.archive.names().len(), n);
        assert!(!is_stale(&b, &o));
        assert!(OpenBook::from_archive(b.archive.clone()).is_ok());
    }

    #[test]
    fn goes_after_the_cover_page_and_shows_the_cover() {
        let mut b = book();
        crate::cover::set_cover(
            &mut b,
            &crate::cover::CoverOptions::new(crate::cover::CoverSource::New {
                data: png(120, 180),
                ext: "png".into(),
            }),
        )
        .unwrap();
        let cover_page = b.spine_paths()[0].clone();
        let (p, _) = add_or_update(&mut b, &JacketOptions::default()).unwrap();
        assert_eq!(b.spine_paths()[..2], [cover_page, p.clone()]);
        let src = b.archive.get_string(&p).unwrap();
        assert!(
            src.contains("jacket-cover") && src.contains("cover.png"),
            "{src}"
        );
    }

    #[test]
    fn metadata_changes_make_it_stale_and_refresh_updates_it() {
        let mut b = book();
        let o = JacketOptions::default();
        assert_eq!(
            refresh(&mut b, &o).unwrap(),
            None,
            "no jacket, nothing to do"
        );
        let (p, _) = add_or_update(&mut b, &o).unwrap();
        let mut md = b.metadata().clone();
        md.title = "New title".into();
        b.set_metadata(md).unwrap();
        assert!(is_stale(&b, &o));
        assert_eq!(refresh(&mut b, &o).unwrap(), Some(p.clone()));
        assert!(b.archive.get_string(&p).unwrap().contains("New title"));
        assert!(!is_stale(&b, &o));
        assert_eq!(refresh(&mut b, &o).unwrap(), None);
    }

    #[test]
    fn remove_leaves_no_trace() {
        let mut b = book();
        let before = (b.archive.names().to_vec(), b.spine_paths());
        add_or_update(&mut b, &JacketOptions::default()).unwrap();
        assert!(remove(&mut b).unwrap().is_some());
        assert_eq!((b.archive.names().to_vec(), b.spine_paths()), before);
        assert!(find_jacket(&b).is_none());
        assert_eq!(remove(&mut b).unwrap(), None);
        let opf = b.archive.get_string(&b.package.opf_path).unwrap();
        assert!(!opf.contains("jacket"));
    }

    #[test]
    fn the_options_are_stored_in_the_page_and_used_again() {
        let mut b = book();
        let o = JacketOptions {
            cover: false,
            series: true,
            tags: false,
            rating: true,
            comments: false,
            publisher: true,
        };
        add_or_update(&mut b, &o).unwrap();
        assert_eq!(options_of(&b), Some(o.clone()));
        assert_eq!(JacketOptions::from_marker_text(&o.to_marker_text()), o);
        assert!(options_of(&make_book(&["<p>x</p>"])).is_none());
    }
}
