//! Hardcover reading status in Bookx (read only): the lists on hardcover.app (Want to Read, Currently
//! Reading, Read, Did Not Finish) and the ratings become reading status, rating and the `to-read` tag
//! of the matching books. Nothing is ever sent to Hardcover. The client and the matching are in
//! `bookx_sources::hardcover`, the writing reuses `tracker_sync::apply` (one logged step for ratings
//! and tags); this file decides what changes and keeps what *Undo* needs.
//!
//! No GTK in here, so it is unit tested; the screen is `ui/hardcover.rs`.

use anyhow::Result;
use bookx_sources::hardcover::{self as hc, Item, Local, Status};
use bookx_sources::isbn;
use bookx_sources::trackers::{TrackerStatus, TrackerUpdate};
use bookx_store::{ReadingStatus, SqliteStore};

use crate::core::library::{Book, Library};
use crate::tracker_sync::{self, to_status, Applied};

/// The tag a Want to Read book gets, the same word the tracker import uses.
pub const TO_READ_TAG: &str = "to-read";

/// What Hardcover says, in the words of the tracker import.
pub fn tracker_status(s: Status) -> TrackerStatus {
    match s {
        Status::WantToRead => TrackerStatus::ToRead,
        Status::Reading => TrackerStatus::Reading,
        Status::Read => TrackerStatus::Read,
        Status::DidNotFinish => TrackerStatus::OnHold,
    }
}

/// The books of the library as the matcher needs them.
pub fn locals(books: &[Book]) -> Vec<Local> {
    books
        .iter()
        .map(|b| Local {
            id: b.id,
            title: b.meta.title.clone(),
            authors: b.meta.authors.clone(),
            isbns: b
                .meta
                .identifiers
                .iter()
                .filter(|i| i.scheme.to_lowercase().starts_with("isbn") || i.scheme.is_empty())
                .filter_map(|i| isbn::canonical(&i.value))
                .collect(),
        })
        .collect()
}

/// Whether Hardcover's `target` should replace what the book has now. A book is never taken back
/// from Finished, and a "Want to Read" never moves a book that was started.
fn moves_status(current: ReadingStatus, target: ReadingStatus) -> bool {
    use ReadingStatus::*;
    if current == target {
        return false;
    }
    match target {
        Finished => true,
        Reading => matches!(current, New | OnHold),
        OnHold => matches!(current, New | Reading),
        New => false,
    }
}

/// What importing would change. `current` gives the reading status a book has now;
/// `overwrite_ratings` replaces a rating the book already has.
pub fn plan(
    items: &[Item],
    books: &[Book],
    current: &dyn Fn(u64) -> ReadingStatus,
    overwrite_ratings: bool,
) -> Vec<TrackerUpdate> {
    let mut out = Vec::new();
    for (id, index) in hc::match_items(items, &locals(books)) {
        let (Some(book), Some(item)) = (books.iter().find(|b| b.id == id), items.get(index)) else {
            continue;
        };
        let target = tracker_status(item.status);
        let status = moves_status(current(id), to_status(target)).then_some(target);
        let rating = item
            .rating
            .filter(|r| book.meta.rating == 0 || (overwrite_ratings && book.meta.rating != *r));
        let add_tags: Vec<String> = if item.status == Status::WantToRead
            && !book
                .meta
                .tags
                .iter()
                .any(|t| t.eq_ignore_ascii_case(TO_READ_TAG))
        {
            vec![TO_READ_TAG.to_string()]
        } else {
            Vec::new()
        };
        let u = TrackerUpdate {
            book: id,
            title: book.meta.title.clone(),
            rating,
            status,
            date_read: None,
            add_tags,
        };
        if !u.is_empty() {
            out.push(u);
        }
    }
    out
}

/// What the preview says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Counts {
    /// Books on the Hardcover lists.
    pub items: usize,
    /// Of them found in the library.
    pub matched: usize,
    pub changes: usize,
    pub statuses: usize,
    pub ratings: usize,
}

pub fn counts(items: &[Item], books: &[Book], updates: &[TrackerUpdate]) -> Counts {
    Counts {
        items: items.len(),
        matched: hc::match_items(items, &locals(books)).len(),
        changes: updates.len(),
        statuses: updates.iter().filter(|u| u.status.is_some()).count(),
        ratings: updates.iter().filter(|u| u.rating.is_some()).count(),
    }
}

/// What *Undo* needs: the status each book had, and the logged step of ratings and tags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Undo {
    statuses: Vec<(u64, ReadingStatus)>,
    op_seq: Option<u64>,
}

/// Writes the changes. Ratings and tags are one logged step of the library, the reading statuses
/// are kept in the returned [`Undo`]; together they are the one step *Undo* takes back.
pub fn apply(
    lib: &mut Library,
    store: &SqliteStore,
    updates: &[TrackerUpdate],
) -> Result<(Applied, Undo)> {
    let statuses: Vec<(u64, ReadingStatus)> = updates
        .iter()
        .filter(|u| u.status.is_some())
        .filter_map(|u| store.get_status(u.book).map(|s| (u.book, s)))
        .collect();
    let before = lib.history().last().map(|o| o.seq);
    let applied = tracker_sync::apply(lib, store, updates)?;
    let after = lib.history().last().map(|o| o.seq);
    Ok((
        applied,
        Undo {
            statuses,
            op_seq: after.filter(|a| Some(*a) != before),
        },
    ))
}

/// Takes the import back: the statuses return, and the ratings and tags too when nothing else was
/// logged since. Returns whether the logged step was undone as well.
pub fn undo(lib: &mut Library, store: &SqliteStore, u: &Undo) -> Result<bool> {
    for (id, s) in &u.statuses {
        store.set_status(*id, *s)?;
    }
    let newest = lib
        .history()
        .iter()
        .rev()
        .find(|o| !o.undone)
        .map(|o| o.seq);
    match u.op_seq {
        Some(seq) if newest == Some(seq) => {
            lib.undo_last()?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::AddOutcome;

    fn item(title: &str, isbn: &str, status: Status, rating: Option<u8>) -> Item {
        Item {
            status,
            title: title.into(),
            authors: vec![],
            isbns: if isbn.is_empty() {
                vec![]
            } else {
                vec![isbn.into()]
            },
            rating,
        }
    }

    fn setup() -> (Library, SqliteStore, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("bookx-hc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        for t in ["Dune", "Emma", "Ulysses", "Persuasion"] {
            let p = d.join(format!("{t}.md"));
            std::fs::write(
                &p,
                format!("---\ntitle: \"{t}\"\nauthor: \"A B\"\n---\n# One\n\nHi.\n"),
            )
            .unwrap();
            let AddOutcome::Added(_) = lib.add_path(&p, true).unwrap() else {
                panic!()
            };
        }
        let store = SqliteStore::open_in_memory().unwrap();
        store.put_many(&lib.books).unwrap();
        (lib, store, d)
    }

    fn id_of(lib: &Library, t: &str) -> u64 {
        lib.books.iter().find(|b| b.meta.title == t).unwrap().id
    }

    #[test]
    fn hardcover_words_map_to_tracker_words() {
        assert_eq!(tracker_status(Status::WantToRead), TrackerStatus::ToRead);
        assert_eq!(tracker_status(Status::Reading), TrackerStatus::Reading);
        assert_eq!(tracker_status(Status::Read), TrackerStatus::Read);
        assert_eq!(tracker_status(Status::DidNotFinish), TrackerStatus::OnHold);
    }

    #[test]
    fn a_finished_book_is_never_taken_back_and_a_started_one_is_not_made_new() {
        use ReadingStatus::*;
        assert!(moves_status(New, Finished) && moves_status(Reading, Finished));
        assert!(!moves_status(Finished, Reading) && !moves_status(Finished, OnHold));
        assert!(moves_status(New, Reading) && moves_status(OnHold, Reading));
        assert!(!moves_status(Reading, New) && !moves_status(Reading, Reading));
        assert!(moves_status(Reading, OnHold) && !moves_status(OnHold, OnHold));
    }

    #[test]
    fn the_plan_counts_only_real_changes() {
        let (lib, store, _d) = setup();
        let items = vec![
            item("Dune", "", Status::Read, Some(9)),
            item("Emma", "", Status::WantToRead, None),
            item("Ulysses", "", Status::Reading, None),
            item("Unknown book", "", Status::Read, Some(10)),
        ];
        let cur = |id| store.get_status(id).unwrap_or(ReadingStatus::New);
        let u = plan(&items, &lib.books, &cur, false);
        assert_eq!(u.len(), 3, "the unknown book is not in the library");
        let dune = u.iter().find(|u| u.title == "Dune").unwrap();
        assert_eq!(
            (dune.status, dune.rating),
            (Some(TrackerStatus::Read), Some(9))
        );
        let emma = u.iter().find(|u| u.title == "Emma").unwrap();
        assert_eq!(emma.add_tags, vec!["to-read"]);
        assert_eq!(emma.status, None, "a new book stays new");
        let c = counts(&items, &lib.books, &u);
        assert_eq!(
            c,
            Counts {
                items: 4,
                matched: 3,
                changes: 3,
                statuses: 2,
                ratings: 1
            }
        );
        // the second run, after applying, has nothing left to do
        store
            .set_status(id_of(&lib, "Dune"), ReadingStatus::Finished)
            .unwrap();
        store
            .set_status(id_of(&lib, "Ulysses"), ReadingStatus::Reading)
            .unwrap();
        let cur = |id| store.get_status(id).unwrap_or(ReadingStatus::New);
        let again = plan(&items[2..3], &lib.books, &cur, false);
        assert!(again.is_empty());
    }

    #[test]
    fn existing_ratings_stay_unless_asked() {
        let (mut lib, store, _d) = setup();
        let id = id_of(&lib, "Dune");
        let mut m = lib.get(id).unwrap().meta.clone();
        m.rating = 4;
        lib.update_metadata(id, m).unwrap();
        let items = vec![item("Dune", "", Status::Read, Some(9))];
        let cur = |id| store.get_status(id).unwrap_or(ReadingStatus::New);
        assert_eq!(plan(&items, &lib.books, &cur, false)[0].rating, None);
        assert_eq!(plan(&items, &lib.books, &cur, true)[0].rating, Some(9));
    }

    #[test]
    fn apply_writes_and_undo_takes_everything_back() {
        let (mut lib, store, _d) = setup();
        let items = vec![
            item("Dune", "", Status::Read, Some(10)),
            item("Emma", "", Status::WantToRead, None),
        ];
        let cur = |id| store.get_status(id).unwrap_or(ReadingStatus::New);
        let updates = plan(&items, &lib.books, &cur, false);
        let (a, undo_token) = apply(&mut lib, &store, &updates).unwrap();
        assert_eq!((a.ratings, a.statuses), (1, 1));
        let (dune, emma) = (id_of(&lib, "Dune"), id_of(&lib, "Emma"));
        assert_eq!(store.get_status(dune), Some(ReadingStatus::Finished));
        assert_eq!(lib.get(dune).unwrap().meta.rating, 10);
        assert!(lib
            .get(emma)
            .unwrap()
            .meta
            .tags
            .iter()
            .any(|t| t == "to-read"));
        assert!(undo(&mut lib, &store, &undo_token).unwrap());
        assert_eq!(store.get_status(dune), Some(ReadingStatus::New));
        assert_eq!(lib.get(dune).unwrap().meta.rating, 0);
        assert!(lib.get(emma).unwrap().meta.tags.is_empty());
    }

    #[test]
    fn undo_leaves_the_log_alone_when_something_else_happened_since() {
        let (mut lib, store, _d) = setup();
        let items = vec![item("Dune", "", Status::Read, Some(10))];
        let cur = |id| store.get_status(id).unwrap_or(ReadingStatus::New);
        let updates = plan(&items, &lib.books, &cur, false);
        let (_, token) = apply(&mut lib, &store, &updates).unwrap();
        let emma = id_of(&lib, "Emma");
        let mut m = lib.get(emma).unwrap().meta.clone();
        m.title = "Emma 2".into();
        lib.update_metadata_many("rename".into(), vec![(emma, m)])
            .unwrap();
        assert!(!undo(&mut lib, &store, &token).unwrap());
        assert_eq!(lib.get(emma).unwrap().meta.title, "Emma 2");
        // the statuses came back anyway
        assert_eq!(
            store.get_status(id_of(&lib, "Dune")),
            Some(ReadingStatus::New)
        );
    }
}
