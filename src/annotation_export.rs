//! Exporting highlights, notes and bookmarks: Markdown, HTML, JSON, plain text, Kindle
//! `My Clippings.txt`, an Obsidian note and the body Readwise's highlights API takes, for one book
//! or the whole library. The renderers are those of `bookx-lookup`; this file turns library books
//! into its items and names the files.

use bookx_lookup::export::{
    self as ex, AnnotationItem, ExportFormat, ExportOptions, HighlightStyle,
};
use chrono::{DateTime, Utc};

use crate::core::library::Book;

/// One entry of the *Export Annotations* menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    /// The value of the menu action.
    pub id: &'static str,
    pub label: &'static str,
    pub extension: &'static str,
}

pub const FORMATS: &[Format] = &[
    Format {
        id: "md",
        label: "Markdown",
        extension: "md",
    },
    Format {
        id: "html",
        label: "HTML",
        extension: "html",
    },
    Format {
        id: "json",
        label: "JSON",
        extension: "json",
    },
    Format {
        id: "txt",
        label: "Plain text",
        extension: "txt",
    },
    Format {
        id: "obsidian",
        label: "Obsidian note",
        extension: "md",
    },
    Format {
        id: "joplin",
        label: "Joplin note",
        extension: "md",
    },
    Format {
        id: "kindle",
        label: "Kindle clippings (My Clippings.txt)",
        extension: "txt",
    },
    Format {
        id: "readwise",
        label: "Readwise (JSON for the highlights API)",
        extension: "json",
    },
];

pub fn format(id: &str) -> Option<Format> {
    FORMATS.iter().copied().find(|f| f.id == id)
}

pub fn style_of(s: &str) -> HighlightStyle {
    // a PDF annotation keeps its style inside `pdf|style|rects`
    let s = crate::pdf::parse_pdf_style(s)
        .map(|x| x.0)
        .unwrap_or_else(|| s.to_string());
    // the device words too (`underscore`, `strikeout`): one table for every path (`devstyle`)
    match crate::core::devstyle::style_from_any(&s) {
        Some("underline") => HighlightStyle::Underline,
        Some("strikethrough") => HighlightStyle::Strikeout,
        Some("squiggly") => HighlightStyle::Squiggly,
        Some("invert") => HighlightStyle::Invert,
        _ => HighlightStyle::Highlight,
    }
}

/// The annotations of one book (highlights, then bookmarks) as export items. `chapters` are the
/// titles by spine position, for the chapter of each entry.
pub fn items_of(book: &Book, chapters: &[String]) -> Vec<AnnotationItem> {
    let chapter = |i: usize| chapters.get(i).cloned();
    let author = Some(book.meta.authors_display()).filter(|a| !a.trim().is_empty());
    let mut hs: Vec<_> = book.highlights.iter().collect();
    hs.sort_by_key(|h| (h.spine, h.start));
    let mut items: Vec<AnnotationItem> = hs
        .into_iter()
        .map(|h| AnnotationItem {
            book_title: book.meta.title.clone(),
            book_author: author.clone(),
            chapter: if crate::pdf::is_pdf(h) {
                None
            } else {
                chapter(h.spine)
            },
            text: h.text.clone(),
            note: Some(h.note.clone()).filter(|n| !n.trim().is_empty()),
            style: style_of(&h.style),
            color: Some(h.color.clone()).filter(|c| !c.is_empty()),
            created: h.created,
            location: crate::pdf::is_pdf(h).then(|| format!("page {}", h.spine + 1)),
        })
        .collect();
    items.extend(book.bookmarks.iter().map(|b| AnnotationItem {
        book_title: book.meta.title.clone(),
        book_author: author.clone(),
        chapter: chapter(b.spine),
        text: b.label.clone(),
        note: None,
        style: HighlightStyle::Bookmark,
        color: None,
        created: b.created,
        location: Some(format!("{:.0}%", b.frac * 100.0)),
    }));
    items
}

/// The text of an export in the format `id`; `None` for an unknown id.
pub fn render(items: &[AnnotationItem], id: &str, now: DateTime<Utc>) -> Option<String> {
    let opts = ExportOptions {
        group_by_chapter: true,
        ..Default::default()
    };
    let f = match id {
        "md" => ExportFormat::Markdown,
        "html" => ExportFormat::Html,
        "json" => ExportFormat::Json,
        "txt" => ExportFormat::Text,
        "obsidian" => ExportFormat::Obsidian,
        "joplin" => ExportFormat::Joplin,
        "kindle" => ExportFormat::KindleClippings,
        "readwise" => {
            return serde_json::to_string_pretty(&ex::readwise_payload(items, &opts)).ok();
        }
        _ => return None,
    };
    Some(ex::export(items, f, &opts, now))
}

/// The file name for one book's export: `<title> - annotations.<ext>`.
pub fn file_name(title: &str, id: &str) -> String {
    let ext = format(id).map_or("txt", |f| f.extension);
    format!("{} - annotations.{ext}", ex::sanitize_filename(title))
}

/// One file per book that has annotations, in the format `id`. A book without any is left out.
/// Name clashes get a number.
pub fn library_files(
    books: &[(&Book, Vec<String>)],
    id: &str,
    now: DateTime<Utc>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (book, chapters) in books {
        let items = items_of(book, chapters);
        if items.is_empty() {
            continue;
        }
        let Some(text) = render(&items, id, now) else {
            continue;
        };
        let mut name = file_name(&book.meta.title, id);
        let mut n = 2;
        while out.iter().any(|(f, _)| *f == name) {
            let ext = format_ext(id);
            name = format!(
                "{} - annotations ({n}).{ext}",
                ex::sanitize_filename(&book.meta.title)
            );
            n += 1;
        }
        out.push((name, text));
    }
    out
}

fn format_ext(id: &str) -> &'static str {
    format(id).map_or("txt", |f| f.extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_style_words_of_every_source_export_as_the_same_style() {
        for (word, want) in [
            ("underline", HighlightStyle::Underline),
            ("underscore", HighlightStyle::Underline),
            ("strikethrough", HighlightStyle::Strikeout),
            ("strikeout", HighlightStyle::Strikeout),
            ("invert", HighlightStyle::Invert),
            ("squiggly", HighlightStyle::Squiggly),
            ("lighten", HighlightStyle::Highlight),
            ("", HighlightStyle::Highlight),
        ] {
            assert_eq!(style_of(word), want, "{word}");
        }
    }

    use crate::core::library::{Bookmark, Highlight};
    use chrono::TimeZone;

    fn book(title: &str, hs: Vec<Highlight>, bm: Vec<Bookmark>) -> Book {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Book {
            id: 1,
            file: "x.epub".into(),
            cover: None,
            meta: crate::core::opf::Metadata {
                title: title.into(),
                authors: vec!["Frank Herbert".into()],
                ..Default::default()
            },
            added: now,
            modified: now,
            last_read: None,
            size: 1,
            progress: 0.0,
            position: None,
            bookmarks: bm,
            highlights: hs,
            formats: vec![],
            resources: vec![],
            drm: String::new(),
            custom: Default::default(),
            marked: false,
        }
    }

    fn hl(id: &str, spine: usize, text: &str, note: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine,
            start: 0,
            end: 1,
            color: "yellow".into(),
            style: String::new(),
            note: note.into(),
            text: text.into(),
            created: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
        }
    }

    fn sample() -> Book {
        book(
            "Dune: Deluxe",
            vec![
                hl("b", 1, "I must not fear.", ""),
                hl("a", 0, "Fear is the mind-killer.", "litany"),
            ],
            vec![Bookmark {
                id: "m".into(),
                spine: 1,
                frac: 0.25,
                label: "here".into(),
                created: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
            }],
        )
    }

    #[test]
    fn pdf_annotations_are_exported_with_their_page_and_style() {
        let mut h = hl("p", 6, "A quote on a page.", "note");
        h.style = crate::pdf::pdf_style("underline", &[[1.0, 2.0, 3.0, 4.0]]);
        let b = book("Paper", vec![h], vec![]);
        let items = items_of(&b, &["Chapter".to_string()]);
        assert_eq!(items[0].location.as_deref(), Some("page 7"));
        assert_eq!(items[0].chapter, None);
        assert_eq!(items[0].style, HighlightStyle::Underline);
    }

    #[test]
    fn every_format_renders_the_quotes_and_the_note() {
        let b = sample();
        let items = items_of(&b, &["One".to_string(), "Two".to_string()]);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].text, "Fear is the mind-killer.", "in book order");
        assert_eq!(items[0].chapter.as_deref(), Some("One"));
        for f in FORMATS {
            let out = render(&items, f.id, Utc::now()).unwrap_or_else(|| panic!("{}", f.id));
            if f.id != "readwise" {
                assert!(
                    out.contains("Dune: Deluxe") || f.id == "kindle",
                    "{} {out}",
                    f.id
                );
            }
            assert!(out.contains("Fear is the mind-killer."), "{}", f.id);
            assert!(out.contains("litany"), "{}", f.id);
        }
        assert!(render(&items, "nope", Utc::now()).is_none());
    }

    #[test]
    fn the_readwise_body_has_the_highlights_and_not_the_bookmark() {
        let items = items_of(&sample(), &[]);
        let j: serde_json::Value =
            serde_json::from_str(&render(&items, "readwise", Utc::now()).unwrap()).unwrap();
        let h = j["highlights"].as_array().unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[0]["title"], "Dune: Deluxe");
        assert_eq!(h[0]["author"], "Frank Herbert");
        assert_eq!(h[0]["note"], "litany");
        assert_eq!(h[0]["source_type"], "bookx");
    }

    #[test]
    fn a_kindle_export_is_read_back_by_the_importer() {
        let items = items_of(&sample(), &[]);
        let txt = render(&items, "kindle", Utc::now()).unwrap();
        let back = bookx_lookup::import::parse_kindle_clippings(&txt);
        let texts: Vec<_> = back
            .iter()
            .filter(|i| !i.text.is_empty())
            .map(|i| i.text.as_str())
            .collect();
        assert_eq!(texts, ["Fear is the mind-killer.", "I must not fear."]);
        assert_eq!(back[0].note, "litany");
    }

    #[test]
    fn file_names_are_safe_and_named_by_format() {
        assert_eq!(
            file_name("Dune: Deluxe", "md"),
            "Dune_ Deluxe - annotations.md"
        );
        assert_eq!(file_name("A/B", "kindle"), "A_B - annotations.txt");
        assert_eq!(file_name("T", "readwise"), "T - annotations.json");
        assert!(format("html").is_some() && format("pdf").is_none());
    }

    #[test]
    fn the_library_export_has_one_file_per_book_with_annotations_and_unique_names() {
        let a = sample();
        let b = sample();
        let empty = book("Empty", vec![], vec![]);
        let files = library_files(
            &[(&a, vec![]), (&b, vec![]), (&empty, vec![])],
            "md",
            Utc::now(),
        );
        assert_eq!(files.len(), 2, "a book without annotations is left out");
        assert_ne!(files[0].0, files[1].0);
        assert!(files[1].0.ends_with("(2).md"), "{}", files[1].0);
    }
}
