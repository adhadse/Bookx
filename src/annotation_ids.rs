//! Highlight ids: every highlight of the library has a non-empty id that is unique inside its
//! book (the sync, the Notes window and the readwise ledger refer to highlights by it), and a
//! highlight that arrives from outside (device sync, import) gets an id derived from its content
//! (`bookx_lookup::ids::content_id`) so that importing the same export twice, here or on another
//! machine, produces the same id instead of a new random one.

use crate::core::library::{Book, Library};

/// The id for a highlight that comes from outside: the content id when that is free in the
/// book, else a random one.
pub fn incoming_id(book: &Book, text: &str) -> String {
    let id = bookx_lookup::ids::content_id(
        &book.meta.title,
        Some(book.meta.authors_display().as_str()).filter(|a| !a.is_empty()),
        text,
    );
    if book.highlights.iter().any(|h| h.id == id) {
        format!("device-{}", uuid::Uuid::new_v4())
    } else {
        id
    }
}

/// Fills empty ids and renames clashing ones in every book. Returns how many were changed.
pub fn migrate_books(books: &mut [Book]) -> usize {
    let mut changed = 0;
    for b in books.iter_mut() {
        let mut ids: Vec<&mut String> = b.highlights.iter_mut().map(|h| &mut h.id).collect();
        let m = bookx_lookup::ids::migrate(&mut ids, |_| format!("hl-{}", uuid::Uuid::new_v4()));
        changed += m.filled + m.renamed;
        let mut ids: Vec<&mut String> = b.bookmarks.iter_mut().map(|h| &mut h.id).collect();
        let m = bookx_lookup::ids::migrate(&mut ids, |_| format!("bm-{}", uuid::Uuid::new_v4()));
        changed += m.filled + m.renamed;
    }
    changed
}

/// [`migrate_books`] for the library; saves it when something changed.
pub fn migrate_library(lib: &mut Library) -> usize {
    let n = migrate_books(&mut lib.books);
    if n > 0 {
        let _ = lib.save();
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::Highlight;
    use chrono::Utc;

    fn book(ids: &[&str]) -> Book {
        let mut b: Book = serde_json::from_value(serde_json::json!({
            "id": 1, "file": "a.epub", "meta": {"title": "Dune", "title_sort": "", "authors": ["Frank Herbert"], "author_sort": "", "publisher": "", "language": "", "description": "", "tags": [], "series": "", "series_index": 0.0, "identifiers": [], "published": "", "rights": "", "rating": 0},
            "added": Utc::now(), "modified": Utc::now(), "size": 1
        }))
        .unwrap();
        for (i, id) in ids.iter().enumerate() {
            b.highlights.push(Highlight {
                id: id.to_string(),
                spine: 0,
                start: i as i32,
                end: i as i32 + 1,
                color: "yellow".into(),
                style: String::new(),
                note: String::new(),
                text: format!("t{i}"),
                created: Utc::now(),
            });
        }
        b
    }

    #[test]
    fn old_highlights_get_unique_ids_and_good_ones_are_untouched() {
        let mut books = vec![book(&["a", "", "a", "b"])];
        assert_eq!(migrate_books(&mut books), 2);
        let ids: Vec<_> = books[0].highlights.iter().map(|h| h.id.clone()).collect();
        assert_eq!(ids[0], "a");
        assert_eq!(ids[3], "b");
        assert!(ids[1].starts_with("hl-") && ids[2].starts_with("hl-") && ids[1] != ids[2]);
        assert_eq!(migrate_books(&mut books), 0);
    }

    #[test]
    fn an_incoming_highlight_has_the_same_id_on_every_machine() {
        let a = book(&[]);
        let b = book(&["x"]);
        assert_eq!(incoming_id(&a, "A quote"), incoming_id(&b, "a  quote"));
        assert!(incoming_id(&a, "A quote").starts_with("bx-"));
    }

    #[test]
    fn a_taken_content_id_falls_back_to_a_random_one() {
        let mut b = book(&[]);
        let id = incoming_id(&b, "A quote");
        let mut h = book(&["x"]).highlights.remove(0);
        h.id = id.clone();
        b.highlights.push(h);
        assert!(incoming_id(&b, "A quote").starts_with("device-"));
    }
}
