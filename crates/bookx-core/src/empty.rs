//! Records without a book file of their own (roadmap L14): "add empty record" and "add by ISBN".
//! The library keeps books as EPUB files, so a record is a tiny EPUB with the title page only; the
//! real file is added later as another format of the same book.

use std::path::Path;

use anyhow::{Result, bail};

use super::convert::{Chapter, create_epub};
use super::library::{AddOutcome, Library, prepare_from_archive};
use super::opf::{Identifier, Metadata};
use super::util::{author_sort, title_sort};

fn esc(s: &str) -> String {
    super::util::xml_escape(s)
}

/// The record's EPUB bytes for `meta`: a title page and nothing else.
pub fn placeholder_epub(meta: &Metadata) -> Result<Vec<u8>> {
    let body = format!(
        "<h1>{}</h1><p>{}</p>",
        esc(&meta.title),
        esc(&meta.authors.join(", "))
    );
    let ch = Chapter {
        title: meta.title.clone(),
        body,
    };
    create_epub(meta, &[ch], None).to_bytes()
}

impl Library {
    /// Adds a record with only metadata. The title is required; an author is filled in as
    /// `Unknown` by the import. Returns the new book's id.
    pub fn add_empty(&mut self, mut meta: Metadata) -> Result<u64> {
        meta.title = meta.title.trim().to_string();
        if meta.title.is_empty() {
            bail!("a record needs a title");
        }
        if meta.title_sort.is_empty() {
            meta.title_sort = title_sort(&meta.title);
        }
        if meta.author_sort.is_empty() && !meta.authors.is_empty() {
            meta.author_sort = meta
                .authors
                .iter()
                .map(|a| author_sort(a))
                .collect::<Vec<_>>()
                .join(" & ");
        }
        let bytes = placeholder_epub(&meta)?;
        let archive = super::archive::Archive::from_bytes(bytes)?;
        let mut p = prepare_from_archive(archive, Path::new(&format!("{}.epub", meta.title)))?;
        p.meta.identifiers = meta.identifiers.clone();
        match self.commit_import(p, true)? {
            AddOutcome::Added(id) => {
                self.save()?;
                Ok(id)
            }
            AddOutcome::Duplicate(id) => Ok(id),
        }
    }
}

/// Metadata for a record made from an ISBN: the title is the ISBN until a lookup fills it in.
pub fn isbn_metadata(isbn13_or_10: &str) -> Metadata {
    Metadata {
        title: format!("ISBN {isbn13_or_10}"),
        identifiers: vec![Identifier {
            scheme: "isbn".into(),
            value: isbn13_or_10.to_string(),
            xml_id: None,
        }],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_record_is_a_real_searchable_book() {
        let dir = std::env::temp_dir().join(format!("bookx-empty-{}", uuid::Uuid::new_v4()));
        let mut lib = Library::in_memory(&dir).unwrap();
        assert!(lib.add_empty(Metadata::default()).is_err(), "needs a title");
        let id = lib
            .add_empty(Metadata {
                title: " The <Missing> Book ".into(),
                authors: vec!["Jane Doe".into()],
                ..Default::default()
            })
            .unwrap();
        let b = lib.get(id).unwrap();
        assert_eq!(b.meta.title, "The <Missing> Book");
        assert_eq!(b.meta.author_sort, "Doe, Jane");
        assert!(lib.book_path(b).is_file());
        // the file itself opens and says the same
        let open = crate::book::OpenBook::open(&lib.book_path(b)).unwrap();
        assert_eq!(open.metadata().title, "The <Missing> Book");
        assert_eq!(
            lib.search("missing", crate::library::SortKey::Title, true),
            vec![id]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn two_empty_records_with_one_title_are_both_kept() {
        let dir = std::env::temp_dir().join(format!("bookx-empty2-{}", uuid::Uuid::new_v4()));
        let mut lib = Library::in_memory(&dir).unwrap();
        let m = isbn_metadata("9780441013593");
        let a = lib.add_empty(m.clone()).unwrap();
        let b = lib.add_empty(m).unwrap();
        assert_ne!(a, b);
        assert_eq!(
            lib.get(a).unwrap().meta.identifier("isbn"),
            Some("9780441013593")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
