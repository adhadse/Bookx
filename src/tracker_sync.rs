//! Reading-tracker exchange (roadmap PP-G-30): import a Goodreads, StoryGraph or Hardcover CSV
//! into the library (rating, tags from shelves, reading status, date read) and export the library
//! in the layouts those services import. The CSV and matching logic is `bookx_sources::trackers`;
//! this is the glue with the library and the catalogue store.
//!
//! No GTK in here, so it is unit tested; the window is `ui/trackers.rs`.

use crate::i18n::trf;
use anyhow::Result;
use bookx_sources::trackers::{
    self as tr, Matched, Service, TrackerRow, TrackerStatus, TrackerUpdate,
};
use bookx_store::{ReadingStatus, SqliteStore};

use crate::core::library::{Book, Library};

/// A parsed file with its matches and the planned changes, for the preview.
#[derive(Debug, Clone)]
pub struct Import {
    pub rows: Vec<TrackerRow>,
    pub matches: Vec<Matched>,
    pub updates: Vec<TrackerUpdate>,
}

impl Import {
    pub fn matched(&self) -> usize {
        self.matches.iter().filter(|m| m.book.is_some()).count()
    }

    pub fn unmatched(&self) -> Vec<&TrackerRow> {
        self.matches
            .iter()
            .filter(|m| m.book.is_none())
            .filter_map(|m| self.rows.get(m.row))
            .collect()
    }

    pub fn summary(&self) -> String {
        format!(
            "{} rows, {} matched with your books, {} would change, {} not in the library",
            self.rows.len(),
            self.matched(),
            self.updates.len(),
            self.rows.len() - self.matched()
        )
    }
}

/// What the Reading Trackers window shows before anything is written (WIN-L6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub rows: usize,
    pub matched: usize,
    pub changes: usize,
    pub not_found: Vec<String>,
    /// Titles not listed because the list is capped.
    pub not_found_more: usize,
}

/// The longest "Not found" list the window builds.
pub const NOT_FOUND_CAP: usize = 50;

impl Import {
    /// Counts and the titles that match no book (at most `NOT_FOUND_CAP`).
    pub fn preview(&self) -> Preview {
        let all = self.unmatched();
        Preview {
            rows: self.rows.len(),
            matched: self.matched(),
            changes: self.updates.len(),
            not_found: all
                .iter()
                .take(NOT_FOUND_CAP)
                .map(|r| r.title.clone())
                .collect(),
            not_found_more: all.len().saturating_sub(NOT_FOUND_CAP),
        }
    }
}

/// Reads CSV text and plans the changes. Nothing is written.
pub fn plan_import(text: &str, books: &[Book], overwrite_ratings: bool) -> Result<Import, String> {
    let rows = tr::parse(text)?;
    let matches = tr::match_rows(&rows, books);
    let updates = tr::plan(&rows, &matches, books, overwrite_ratings);
    Ok(Import {
        rows,
        matches,
        updates,
    })
}

pub fn to_status(s: TrackerStatus) -> ReadingStatus {
    match s {
        TrackerStatus::Read => ReadingStatus::Finished,
        TrackerStatus::Reading => ReadingStatus::Reading,
        TrackerStatus::OnHold => ReadingStatus::OnHold,
        TrackerStatus::ToRead => ReadingStatus::New,
    }
}

fn from_status(s: ReadingStatus, b: &Book) -> TrackerStatus {
    match s {
        ReadingStatus::Finished => TrackerStatus::Read,
        ReadingStatus::Reading => TrackerStatus::Reading,
        ReadingStatus::OnHold => TrackerStatus::OnHold,
        ReadingStatus::New => {
            if b.progress >= 0.98 {
                TrackerStatus::Read
            } else if b.progress > 0.0 {
                TrackerStatus::Reading
            } else {
                TrackerStatus::ToRead
            }
        }
    }
}

/// What applying did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    pub books: usize,
    pub ratings: usize,
    pub tags: usize,
    pub statuses: usize,
    pub dates: usize,
}

/// Writes the planned changes. Ratings and tags are one logged step (`update_metadata_many`, so one
/// undo); the reading status and the last-read date live in the catalogue only.
pub fn apply(lib: &mut Library, store: &SqliteStore, updates: &[TrackerUpdate]) -> Result<Applied> {
    let mut out = Applied::default();
    let mut edits = Vec::new();
    for u in updates {
        let Some(b) = lib.get(u.book) else { continue };
        if u.rating.is_some() || !u.add_tags.is_empty() {
            let mut m = b.meta.clone();
            if let Some(r) = u.rating {
                m.rating = r;
                out.ratings += 1;
            }
            for t in &u.add_tags {
                if !m.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                    m.tags.push(t.clone());
                    out.tags += 1;
                }
            }
            edits.push((u.book, m));
        }
    }
    lib.update_metadata_many(
        trf("Imported reading data for {} books", &[&updates.len()]),
        edits,
    )?;
    for u in updates {
        if let Some(s) = u.status {
            store.set_status(u.book, to_status(s))?;
            out.statuses += 1;
        }
        if let (Some(d), Some(b)) = (u.date_read, lib.get_mut(u.book)) {
            b.last_read = d.and_hms_opt(12, 0, 0).map(|t| t.and_utc());
            out.dates += 1;
        }
        out.books += 1;
    }
    lib.save()?;
    Ok(out)
}

/// The library as CSV for `service`.
pub fn export(lib: &Library, store: &SqliteStore, service: Service) -> String {
    let status = |b: &Book| {
        Some(match store.get_status(b.id) {
            Some(s) => from_status(s, b),
            None => from_status(ReadingStatus::New, b),
        })
    };
    tr::export(&lib.books, &status, service)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::AddOutcome;

    const CSV: &str = "Title,Author,ISBN13,My Rating,Date Read,Bookshelves,Exclusive Shelf\n\
        Dune,Frank Herbert,=\"9780441172719\",5,2021/03/14,\"favorites\",read\n\
        Emma,Jane Austen,,0,,classics,to-read\n\
        Unknown Book,Nobody,,3,,,read\n";

    fn setup() -> (Library, SqliteStore, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("bookx-ts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        for (t, a) in [("Dune", "Frank Herbert"), ("Emma", "Jane Austen")] {
            let p = d.join(format!("{t}.md"));
            std::fs::write(
                &p,
                format!("---\ntitle: \"{t}\"\nauthor: \"{a}\"\n---\n# One\n\nHi.\n"),
            )
            .unwrap();
            let AddOutcome::Added(_) = lib.add_path(&p, true).unwrap() else {
                panic!()
            };
        }
        (lib, SqliteStore::open_in_memory().unwrap(), d)
    }

    #[test]
    fn the_preview_counts_matches_and_changes_without_writing() {
        let (lib, _store, d) = setup();
        let imp = plan_import(CSV, &lib.books, false).unwrap();
        assert_eq!(imp.rows.len(), 3);
        assert_eq!(imp.matched(), 2);
        assert_eq!(imp.unmatched()[0].title, "Unknown Book");
        assert_eq!(imp.updates.len(), 2);
        assert!(imp.summary().contains("2 matched"));
        assert_eq!(lib.books[0].meta.rating, 0);
        assert!(plan_import("nonsense,here\n1,2\n", &lib.books, false).is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_preview_lists_the_titles_that_were_not_found() {
        let (lib, _store, d) = setup();
        let p = plan_import(CSV, &lib.books, false).unwrap().preview();
        assert_eq!((p.rows, p.matched, p.changes), (3, 2, 2));
        assert_eq!(p.not_found, vec!["Unknown Book".to_string()]);
        assert_eq!(p.not_found_more, 0);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn applying_sets_rating_tags_status_and_date_and_one_undo_takes_back_the_metadata() {
        let (mut lib, store, d) = setup();
        store.put_many(&lib.books).unwrap();
        let imp = plan_import(CSV, &lib.books, false).unwrap();
        let history = lib.history().len();
        let a = apply(&mut lib, &store, &imp.updates).unwrap();
        assert_eq!(lib.history().len(), history + 1);
        assert_eq!((a.books, a.ratings, a.statuses, a.dates), (2, 1, 1, 1));
        let dune = &lib.books[0];
        assert_eq!(dune.meta.rating, 10);
        assert_eq!(dune.meta.tags, vec!["favorites"]);
        assert_eq!(
            dune.last_read.unwrap().format("%Y-%m-%d").to_string(),
            "2021-03-14"
        );
        assert_eq!(store.get_status(dune.id), Some(ReadingStatus::Finished));
        let emma = &lib.books[1];
        assert_eq!(emma.meta.tags, vec!["classics", "to-read"]);
        assert_eq!(
            store.get_status(emma.id),
            Some(ReadingStatus::New),
            "to-read sets no status"
        );
        lib.undo_last().unwrap();
        assert_eq!(lib.books[0].meta.rating, 0);
        assert!(lib.books[1].meta.tags.is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_export_reflects_status_and_reads_back() {
        let (mut lib, store, d) = setup();
        store.put_many(&lib.books).unwrap();
        let imp = plan_import(CSV, &lib.books, false).unwrap();
        apply(&mut lib, &store, &imp.updates).unwrap();
        for svc in [Service::Goodreads, Service::StoryGraph] {
            let csv = export(&lib, &store, svc);
            let back = plan_import(&csv, &lib.books, true).unwrap();
            assert_eq!(back.matched(), 2, "{svc:?}");
            let dune = back.rows.iter().find(|r| r.title == "Dune").unwrap();
            assert_eq!(dune.status, Some(TrackerStatus::Read));
            assert_eq!(dune.rating, Some(10));
        }
        let _ = std::fs::remove_dir_all(d);
    }
}
