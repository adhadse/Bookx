//! High-level handle on an EPUB: archive + parsed package.

use std::path::Path;

use anyhow::{Context, Result};

use super::archive::Archive;
use super::doc::Doc;
use super::html::parse_html;
use super::opf::{Metadata, Package};
use super::toc::{self, TocEntry};
use super::util::*;

pub struct OpenBook {
    pub archive: Archive,
    pub package: Package,
}

impl OpenBook {
    pub fn open(path: &Path) -> Result<Self> {
        let archive = Archive::open(path)?;
        Self::from_archive(archive)
    }

    pub fn from_archive(archive: Archive) -> Result<Self> {
        let package = Package::parse(&archive).context("could not read the EPUB package")?;
        Ok(Self { archive, package })
    }

    pub fn reload_package(&mut self) -> Result<()> {
        self.package = Package::parse(&self.archive)?;
        Ok(())
    }

    pub fn metadata(&self) -> &Metadata {
        &self.package.metadata
    }

    /// The book is fixed layout (pre-paginated): one page per spine item.
    pub fn is_fixed_layout(&self) -> bool {
        self.archive
            .get_string(&self.package.opf_path)
            .is_some_and(|x| crate::doc::is_fixed_layout(&x))
    }

    /// The spine says pages turn right to left (Arabic, Hebrew, manga).
    pub fn is_rtl(&self) -> bool {
        self.package.rtl
    }

    pub fn spine_paths(&self) -> Vec<String> {
        self.package.spine_paths()
    }

    pub fn toc(&self) -> Vec<TocEntry> {
        toc::read_toc(&self.archive, &self.package)
    }

    pub fn cover_bytes(&self) -> Option<&[u8]> {
        self.package.cover_bytes(&self.archive)
    }

    pub fn chapter_source(&self, path: &str) -> Option<String> {
        self.archive.get_string(path)
    }

    pub fn chapter_doc(&self, path: &str) -> Option<Doc> {
        self.chapter_source(path).map(|s| parse_html(&s, path))
    }

    /// The chapter with the book's own styling applied (linked style sheets and `<style>` blocks).
    pub fn chapter_doc_styled(&self, path: &str) -> Option<Doc> {
        let source = self.chapter_source(path)?;
        Some(self.doc_from_source_styled(&source, path))
    }

    /// Like [`chapter_doc_styled`](Self::chapter_doc_styled) for text that is not (yet) in the
    /// archive, e.g. what the editor has in its buffer.
    pub fn doc_from_source_styled(&self, source: &str, path: &str) -> Doc {
        let sheets = self.chapter_stylesheets(source, path);
        if sheets.is_empty() {
            return parse_html(source, path);
        }
        let styling = crate::html::Styling::new(&sheets);
        crate::html::parse_html_styled(source, path, Some(&styling))
    }

    /// The chapter with the reader's style layer: the book's own CSS when `publisher` is set and
    /// the user's style sheet `user_css` on top of it (R21).
    pub fn chapter_doc_layered(&self, path: &str, publisher: bool, user_css: &str) -> Option<Doc> {
        let source = self.chapter_source(path)?;
        let all_sheets = self.chapter_stylesheets(&source, path);
        let sheets = if publisher {
            all_sheets.clone()
        } else {
            Vec::new()
        };
        if sheets.is_empty() && user_css.trim().is_empty() {
            let mut doc = parse_html(&source, path);
            // the direction of the lines is not a matter of style: a vertical book stays vertical
            // when the reader turns the publisher's styles off
            if all_sheets.iter().any(|s| s.contains("writing-mode"))
                || source.contains("writing-mode")
            {
                let probe = crate::html::parse_html_styled(
                    &source,
                    path,
                    Some(&crate::html::Styling::new(&all_sheets)),
                );
                doc.writing_mode = probe.writing_mode;
                doc.orientation = probe.orientation;
            }
            return Some(doc);
        }
        let styling = crate::html::Styling::layered(&sheets, user_css, publisher);
        Some(crate::html::parse_html_styled(
            &source,
            path,
            Some(&styling),
        ))
    }

    /// CSS that applies to a chapter, in document order.
    pub fn chapter_stylesheets(&self, source: &str, path: &str) -> Vec<String> {
        use scraper::Selector;
        let html = crate::util::parse_html_document(source);
        let mut out = Vec::new();
        let (Ok(link), Ok(style), Ok(any)) = (
            Selector::parse("link[rel~=stylesheet], link[rel~=Stylesheet], style"),
            Selector::parse("style"),
            Selector::parse("*"),
        ) else {
            return out;
        };
        let _ = (&style, &any);
        let base = crate::util::parent_dir(path).to_string();
        for el in html.select(&link) {
            if el.value().name() == "style" {
                let css: String = el.text().collect();
                if !css.trim().is_empty() {
                    out.push(css);
                }
            } else if let Some(href) = el.value().attr("href") {
                if href.starts_with("http:")
                    || href.starts_with("https:")
                    || href.starts_with("data:")
                {
                    continue;
                }
                let (p, _) = crate::util::resolve_href(&base, href);
                if let Some(css) = self.archive.get_string(&p) {
                    out.push(css);
                }
            }
        }
        out
    }

    /// Weight of each spine item (bytes), used to estimate overall progress.
    pub fn spine_weights(&self) -> Vec<f64> {
        self.spine_paths()
            .iter()
            .map(|p| self.archive.size_of(p).max(1) as f64)
            .collect()
    }

    /// Writes new metadata into the package document.
    pub fn set_metadata(&mut self, md: Metadata) -> Result<()> {
        self.package.metadata = md;
        self.package.write_metadata(&mut self.archive)?;
        self.reload_package()
    }

    pub fn set_cover(&mut self, data: Vec<u8>, ext: &str) -> Result<()> {
        let ext = ext.to_ascii_lowercase();
        let mut pkg = self.package.clone();
        pkg.set_cover(&mut self.archive, data, &ext)?;
        self.package = pkg;
        Ok(())
    }

    pub fn remove_cover(&mut self) -> Result<()> {
        let mut pkg = self.package.clone();
        pkg.remove_cover(&mut self.archive)?;
        self.package = pkg;
        Ok(())
    }

    /// Resolve an internal link into (spine index, fragment).
    pub fn locate(&self, href: &str) -> Option<(usize, Option<String>)> {
        let (path, frag) = match href.split_once('#') {
            Some((p, f)) => (p.to_string(), Some(f.to_string())),
            None => (href.to_string(), None),
        };
        let spine = self.spine_paths();
        spine.iter().position(|p| *p == path).map(|i| (i, frag))
    }

    pub fn file_list(&self) -> Vec<String> {
        let mut v: Vec<String> = self.archive.names().to_vec();
        v.sort_by_key(|a| (file_rank(a), a.to_ascii_lowercase()));
        v
    }
}

fn file_rank(name: &str) -> u8 {
    match extension(name).as_str() {
        "xhtml" | "html" | "htm" | "xht" => 0,
        "css" => 1,
        "opf" | "ncx" | "xml" => 2,
        e if is_image_path(&format!("x.{e}")) => 3,
        "ttf" | "otf" | "woff" | "woff2" => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod styled_tests {
    use super::*;
    use crate::convert::{Chapter, create_epub};
    use crate::doc::Span;
    use crate::opf::Metadata;

    fn book_with(css: &str, body: &str) -> OpenBook {
        let md = Metadata {
            title: "T".into(),
            authors: vec!["A".into()],
            ..Default::default()
        };
        let ch = Chapter {
            title: "One".into(),
            body: body.into(),
        };
        let mut a = create_epub(&md, &[ch], None);
        a.set_string("OEBPS/style.css", css);
        OpenBook::from_archive(a).unwrap()
    }

    fn italic_words(d: &crate::doc::Doc) -> Vec<String> {
        d.blocks
            .iter()
            .flat_map(|b| b.spans.iter())
            .filter_map(|s| match s {
                Span::Text { text, style, .. } if style.italic => Some(text.trim().to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn linked_stylesheets_are_applied() {
        let b = book_with(
            ".it { font-style: italic }",
            r#"<p>a <span class="it">slanted</span></p>"#,
        );
        let path = b.spine_paths()[0].clone();
        let styled = b.chapter_doc_styled(&path).unwrap();
        assert_eq!(italic_words(&styled), vec!["slanted"]);
        // the plain version ignores the book's CSS
        assert!(italic_words(&b.chapter_doc(&path).unwrap()).is_empty());
    }

    #[test]
    fn style_blocks_in_the_chapter_are_applied() {
        let b = book_with("", "<p class=\"k\">kursiv</p>");
        let path = b.spine_paths()[0].clone();
        let src = b
            .chapter_source(&path)
            .unwrap()
            .replace("</head>", "<style>.k { font-style: italic }</style></head>");
        let sheets = b.chapter_stylesheets(&src, &path);
        assert!(sheets.iter().any(|s| s.contains(".k")), "{sheets:?}");
    }

    #[test]
    fn external_and_missing_stylesheets_are_skipped() {
        let b = book_with("", "<p>x</p>");
        let path = b.spine_paths()[0].clone();
        let src = "<html><head>\
            <link rel=\"stylesheet\" href=\"https://example.com/x.css\"/>\
            <link rel=\"stylesheet\" href=\"missing.css\"/>\
            <link rel=\"stylesheet\" href=\"style.css\"/></head><body><p>x</p></body></html>";
        let sheets = b.chapter_stylesheets(src, &path);
        assert_eq!(sheets.len(), 1, "{sheets:?}");
    }

    #[test]
    fn a_chapter_without_css_uses_the_plain_parser() {
        let b = book_with("", "<p>plain text</p>");
        let path = b.spine_paths()[0].clone();
        let a = b.chapter_doc(&path).unwrap();
        let s = b.chapter_doc_styled(&path).unwrap();
        assert_eq!(a.blocks, s.blocks);
        assert!(b.chapter_doc_styled("no/such.xhtml").is_none());
    }
}
