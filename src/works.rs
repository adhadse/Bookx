//! Editions and formats of one work (roadmap PP-G-4, PP-G-6): books with the same normalised title
//! and author are listed together so the user sees them as one work. The catalogue keeps one file
//! per book, so this is a grouping view: nothing is merged or deleted here (Find Duplicates does
//! the merging, with its own confirmation). A book that holds several format files (after
//! *Merge as formats of one book*) is listed too, with all its formats.
//!
//! No GTK in here, so it is unit tested; the window is `ui/organize.rs`.

use std::collections::BTreeMap;

use bookx_sources::authors::to_first_last;
use bookx_sources::dedup::title_key;
use bookx_sources::text::normalize;

use crate::core::library::Book;

/// One book (edition / file) of a work.
#[derive(Debug, Clone, PartialEq)]
pub struct Edition {
    pub id: u64,
    pub format: String,
    /// Further format files of the same book (after a merge).
    pub more: Vec<String>,
    pub isbn: String,
    pub publisher: String,
    pub published: String,
    pub size: u64,
}

/// Books that are the same work.
#[derive(Debug, Clone, PartialEq)]
pub struct Work {
    pub title: String,
    pub authors: String,
    pub editions: Vec<Edition>,
}

impl Work {
    /// The distinct formats, sorted: `epub, kepub`.
    pub fn formats(&self) -> Vec<String> {
        let mut f: Vec<String> = self
            .editions
            .iter()
            .flat_map(|e| std::iter::once(e.format.clone()).chain(e.more.iter().cloned()))
            .collect();
        f.sort();
        f.dedup();
        f
    }

    /// The ids of the books of this work, the one to keep first.
    pub fn ids(&self) -> Vec<u64> {
        self.editions.iter().map(|e| e.id).collect()
    }

    pub fn subtitle(&self) -> String {
        format!(
            "{} · {}",
            crate::library_health::plural(self.editions.len(), "edition"),
            self.formats().join(", ")
        )
    }
}

/// The file format of a library path: `Dune.kepub.epub` → `kepub`, `a.azw3` → `azw3`.
pub fn format_of(file: &str) -> String {
    let name = file.rsplit('/').next().unwrap_or(file).to_lowercase();
    if name.ends_with(".kepub.epub") {
        return "kepub".into();
    }
    name.rsplit_once('.')
        .map(|(_, e)| e.to_string())
        .unwrap_or_default()
}

fn work_key(b: &Book) -> Option<String> {
    let title = title_key(&b.meta.title).main;
    if title.is_empty() {
        return None;
    }
    let author = b
        .meta
        .authors
        .first()
        .map(|a| {
            normalize(&to_first_last(a))
                .split_whitespace()
                .last()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();
    Some(format!("{title}|{author}"))
}

/// Works with more than one edition or format, sorted by title.
pub fn group(books: &[Book]) -> Vec<Work> {
    let mut map: BTreeMap<String, Vec<&Book>> = BTreeMap::new();
    for b in books {
        if let Some(k) = work_key(b) {
            map.entry(k).or_default().push(b);
        }
    }
    let mut out: Vec<Work> = map
        .into_values()
        .filter(|v| v.len() > 1 || !v[0].formats.is_empty())
        .map(|v| Work {
            title: v[0].meta.title.clone(),
            authors: v[0].meta.authors_display(),
            editions: v
                .iter()
                .map(|b| Edition {
                    id: b.id,
                    format: format_of(&b.file),
                    more: b.formats.iter().map(|f| f.format.clone()).collect(),
                    isbn: b
                        .meta
                        .identifiers
                        .iter()
                        .find(|i| i.scheme.eq_ignore_ascii_case("isbn"))
                        .map(|i| i.value.clone())
                        .unwrap_or_default(),
                    publisher: b.meta.publisher.clone(),
                    published: b.meta.published.clone(),
                    size: b.size,
                })
                .collect(),
        })
        .collect();
    out.sort_by_key(|w| normalize(&w.title));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(id: u64, file: &str, title: &str, author: &str) -> Book {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"T","title_sort":"T","authors":["A"],"author_sort":"A","publisher":"","language":"","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        b.id = id;
        b.file = file.into();
        b.meta.title = title.into();
        b.meta.authors = vec![author.into()];
        b
    }

    #[test]
    fn formats_come_from_the_file_name() {
        assert_eq!(format_of("a/b/Dune.kepub.epub"), "kepub");
        assert_eq!(format_of("Dune.EPUB"), "epub");
        assert_eq!(format_of("x/y.azw3"), "azw3");
        assert_eq!(format_of("noext"), "");
    }

    #[test]
    fn editions_and_formats_of_one_work_are_grouped() {
        let books = vec![
            book(1, "a/Dune.epub", "Dune", "Frank Herbert"),
            book(2, "b/Dune.kepub.epub", "The Dune", "Herbert, Frank"),
            book(3, "c/Dune.epub", "Dune: Deluxe Edition", "F. Herbert"),
            book(4, "d/Emma.epub", "Emma", "Jane Austen"),
            book(5, "e/Dune.epub", "Dune", "Someone Else"),
        ];
        let w = group(&books);
        assert_eq!(w.len(), 1, "{w:?}");
        let ids: Vec<u64> = w[0].editions.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(w[0].formats(), vec!["epub", "kepub"]);
        assert_eq!(w[0].subtitle(), "3 editions · epub, kepub");
    }

    #[test]
    fn a_merged_book_is_a_work_of_its_own_and_lists_all_its_formats() {
        let mut b = book(1, "a/Dune.epub", "Dune", "Frank Herbert");
        b.formats.push(crate::core::formats::FormatFile {
            file: "a/Dune.azw3".into(),
            format: "azw3".into(),
            size: 5,
        });
        let w = group(&[b, book(2, "b/Emma.epub", "Emma", "Jane Austen")]);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].subtitle(), "1 edition · azw3, epub");
        assert_eq!(w[0].ids(), vec![1]);
    }

    #[test]
    fn a_book_alone_or_without_a_title_makes_no_work() {
        let books = vec![book(1, "a.epub", "", "A"), book(2, "b.epub", "", "A")];
        assert!(group(&books).is_empty());
        assert!(group(&[]).is_empty());
    }
}
