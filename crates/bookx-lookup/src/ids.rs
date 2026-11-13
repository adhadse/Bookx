//! Stable identities for highlights.
//!
//! A highlight made in Bookx has a random id (a UUID), which is good inside the library but means
//! nothing outside it: exporting a book twice, or exporting and importing on another machine,
//! cannot tell that two entries are the same quote. [`content_id`] derives an id from what the
//! highlight *is* (book title, author and the quote), so it is the same on every export, on every
//! machine and after an import. Position and colour are left out on purpose: moving a highlight
//! or recolouring it must not make it a new one.
//!
//! [`migrate`] gives every highlight of a book a usable library id (non-empty and unique inside the
//! book); existing ids are kept because devices and sync state refer to them.

use sha2::{Digest, Sha256};

use crate::export::AnnotationItem;

fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `bx-` and 24 hex digits, from the book and the quote.
pub fn content_id(title: &str, author: Option<&str>, text: &str) -> String {
    let mut h = Sha256::new();
    h.update(norm(title).as_bytes());
    h.update([0]);
    h.update(norm(author.unwrap_or("")).as_bytes());
    h.update([0]);
    h.update(norm(text).as_bytes());
    let d = h.finalize();
    let hex: String = d.iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!("bx-{hex}")
}

/// The stable id of an export item.
pub fn item_id(i: &AnnotationItem) -> String {
    content_id(&i.book_title, i.book_author.as_deref(), &i.text)
}

/// What [`migrate`] changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Migrated {
    /// Highlights that had no id.
    pub filled: usize,
    /// Highlights whose id clashed with an earlier one in the same book.
    pub renamed: usize,
}

/// Makes the ids in `ids` non-empty and unique. `fresh` makes a new id for a position (called
/// with the index). Ids that are fine stay as they are.
pub fn migrate(ids: &mut [&mut String], mut fresh: impl FnMut(usize) -> String) -> Migrated {
    let mut seen = std::collections::HashSet::new();
    let mut out = Migrated::default();
    for (n, id) in ids.iter_mut().enumerate() {
        if id.trim().is_empty() {
            **id = fresh(n);
            out.filled += 1;
        } else if seen.contains(&**id) {
            **id = fresh(n);
            out.renamed += 1;
        }
        seen.insert((**id).clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_follows_the_quote_not_the_spelling_of_spaces_or_case() {
        let a = content_id("Dune", Some("Frank Herbert"), "Fear is  the mind-killer.");
        let b = content_id(" dune ", Some("FRANK HERBERT"), "fear is the\nmind-killer.");
        assert_eq!(a, b);
        assert!(a.starts_with("bx-") && a.len() == 27);
    }

    #[test]
    fn another_quote_book_or_author_is_another_id() {
        let a = content_id("Dune", Some("Frank Herbert"), "Fear.");
        assert_ne!(a, content_id("Dune", Some("Frank Herbert"), "Fear!"));
        assert_ne!(
            a,
            content_id("Dune Messiah", Some("Frank Herbert"), "Fear.")
        );
        assert_ne!(a, content_id("Dune", None, "Fear."));
    }

    #[test]
    fn migration_fills_empty_ids_and_splits_clashes_but_keeps_good_ones() {
        let (mut a, mut b, mut c, mut d) = (
            "keep".to_string(),
            String::new(),
            "keep".to_string(),
            "other".to_string(),
        );
        let r = migrate(&mut [&mut a, &mut b, &mut c, &mut d], |n| {
            format!("new-{n}")
        });
        assert_eq!(
            r,
            Migrated {
                filled: 1,
                renamed: 1
            }
        );
        assert_eq!((a.as_str(), b.as_str()), ("keep", "new-1"));
        assert_eq!((c.as_str(), d.as_str()), ("new-2", "other"));
        let r = migrate(&mut [&mut a, &mut b, &mut c, &mut d], |n| format!("x-{n}"));
        assert_eq!(r, Migrated::default(), "a second pass changes nothing");
    }
}
