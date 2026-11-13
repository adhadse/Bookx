//! The *Needs attention* list (roadmap PP-G-1, import quality): books with weak metadata (no
//! author, a title that looks like a file name, no cover, no language) and identifiers that could be
//! tidied. Nothing here changes a book by itself; cleaning identifiers is an explicit action that is
//! logged (undoable) and recorded in the provenance.
//!
//! No GTK in here, so it is unit tested; the window is `ui/organize.rs`.

use bookx_sources::idclean::{self, Suggestion};
use bookx_store::SqliteStore;

use crate::core::library::{Book, Library};
use crate::core::quality::{weak_reasons, Weak};

/// The search that shows these books in the library.
pub const QUERY: &str = "weak:";

/// A book that needs a look.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: u64,
    pub title: String,
    pub reasons: Vec<Weak>,
}

impl Item {
    /// `no author, no cover`
    pub fn reasons_text(&self) -> String {
        self.reasons
            .iter()
            .map(|r| r.label())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Books with at least one weak point, in library order.
pub fn items(books: &[Book]) -> Vec<Item> {
    books
        .iter()
        .filter_map(|b| {
            let reasons = weak_reasons(b);
            (!reasons.is_empty()).then(|| Item {
                id: b.id,
                title: if b.meta.title.trim().is_empty() {
                    b.file.clone()
                } else {
                    b.meta.title.clone()
                },
                reasons,
            })
        })
        .collect()
}

/// How many books of each weak point there are.
pub fn counts(books: &[Book]) -> Vec<(Weak, usize)> {
    Weak::ALL
        .iter()
        .map(|w| {
            (
                *w,
                books.iter().filter(|b| weak_reasons(b).contains(w)).count(),
            )
        })
        .collect()
}

/// The identifier suggestions per book (books without any are left out).
pub fn identifier_suggestions(books: &[Book]) -> Vec<(u64, String, Vec<Suggestion>)> {
    books
        .iter()
        .filter_map(|b| {
            let s = idclean::suggest(&b.meta);
            (!s.is_empty()).then(|| (b.id, b.meta.title.clone(), s))
        })
        .collect()
}

/// Applies every fixable identifier suggestion of the given books (all books when `ids` is empty).
/// One logged edit per book changed; returns how many books changed.
pub fn clean_identifiers(lib: &mut Library, store: &SqliteStore, ids: &[u64], now: i64) -> usize {
    let todo: Vec<u64> = if ids.is_empty() {
        lib.books.iter().map(|b| b.id).collect()
    } else {
        ids.to_vec()
    };
    let mut n = 0;
    for id in todo {
        let Some(before) = lib.get(id).map(|b| b.meta.clone()) else {
            continue;
        };
        let sugg = idclean::suggest(&before);
        let after = idclean::apply(&before, &sugg);
        if after == before {
            continue;
        }
        if lib.update_metadata(id, after.clone()).is_ok() {
            crate::library_health::record_edit_as(
                store,
                id,
                &before,
                &after,
                &[],
                "identifier cleanup",
                now,
            );
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::opf::Identifier;

    fn lib_with(
        tag: &str,
        books: &[(&str, &str)],
    ) -> (
        Library,
        std::path::PathBuf,
        crate::library_health::LibraryGuard,
    ) {
        let d = std::env::temp_dir().join(format!("bookx-attn-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let (mut lib, g, _) = crate::library_health::open(&d.join("lib")).unwrap();
        for (i, (t, a)) in books.iter().enumerate() {
            let f = d.join(format!("{i}.md"));
            std::fs::write(
                &f,
                format!("---\ntitle: \"{t}\"\nauthor: \"{a}\"\n---\n# a\n\ntext\n"),
            )
            .unwrap();
            lib.add_path(&f, false).unwrap();
        }
        (lib, d, g)
    }

    #[test]
    fn weak_books_are_listed_with_their_reasons_and_counted() {
        let (lib, d, _g) = lib_with(
            "items",
            &[("Dune", "Frank Herbert"), ("scan_001", "Frank Herbert")],
        );
        let it = items(&lib.books);
        // markdown imports have no cover, so both lack one
        assert_eq!(it.len(), 2);
        assert_eq!(it[0].reasons_text(), "no cover");
        assert_eq!(
            it[1].reasons_text(),
            "title looks like a file name, no cover"
        );
        let c = counts(&lib.books);
        assert_eq!(
            c.iter().find(|(w, _)| *w == Weak::FilenameTitle).unwrap().1,
            1
        );
        assert_eq!(c.iter().find(|(w, _)| *w == Weak::NoCover).unwrap().1, 2);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn cleaning_identifiers_is_logged_recorded_and_idempotent() {
        let (mut lib, d, g) =
            lib_with("ids", &[("Dune", "Frank Herbert"), ("Emma", "Jane Austen")]);
        let mut m = lib.books[0].meta.clone();
        m.identifiers = vec![Identifier {
            scheme: "ISBN".into(),
            value: "978-0-441-17271-9".into(),
            xml_id: None,
        }];
        let id0 = lib.books[0].id;
        lib.update_metadata(id0, m).unwrap();
        let sugg = identifier_suggestions(&lib.books);
        assert_eq!(sugg.len(), 1);
        assert_eq!(sugg[0].0, id0);
        let ops = lib.history().len();
        assert_eq!(clean_identifiers(&mut lib, &g.store, &[], 5), 1);
        assert_eq!(lib.history().len(), ops + 1);
        let ids = &lib.get(id0).unwrap().meta.identifiers;
        assert_eq!(
            (ids[0].scheme.as_str(), ids[0].value.as_str()),
            ("isbn", "9780441172719")
        );
        assert!(g
            .store
            .provenance(id0)
            .iter()
            .any(|f| f.field == "identifiers" && f.source == "identifier cleanup"));
        assert_eq!(
            clean_identifiers(&mut lib, &g.store, &[], 6),
            0,
            "nothing left to do"
        );
        // the operation log can take it back
        lib.undo_last().unwrap();
        assert_eq!(
            lib.get(id0).unwrap().meta.identifiers[0].value,
            "978-0-441-17271-9"
        );
        let _ = std::fs::remove_dir_all(d);
    }
}
