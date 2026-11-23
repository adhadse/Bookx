//! Schema v10 (the annotation history table) on a real v9 library: the update keeps a copy first, loses no book and no
//! annotation, adds an empty history, and a second open does nothing. Pattern of `migration_safety.rs`.

mod common;

use std::path::{Path, PathBuf};

use bookx_core::contracts::Store;
use bookx_core::library::{Bookmark, Highlight};
use bookx_store::{SCHEMA_VERSION, SqliteStore};
use chrono::{TimeZone, Utc};
use common::book;

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("bookx-histmig-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn copies(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.to_string_lossy();
            n.contains(".pre-v") && !n.ends_with("-wal") && !n.ends_with("-shm")
        })
        .collect();
    v.sort();
    v
}

fn annotations(s: &SqliteStore) -> (usize, usize, usize) {
    let books = s.all_books().unwrap();
    (
        books.len(),
        books.iter().map(|b| b.highlights.len()).sum(),
        books.iter().map(|b| b.bookmarks.len()).sum(),
    )
}

/// 12 books, each with 3 highlights (one with a note) and 2 bookmarks, in a database that looks like it came
/// from the previous release: no history table, version 9.
fn v9_library(db: &Path) {
    {
        let s = SqliteStore::open(db).unwrap();
        let books: Vec<_> = (1..=12u64)
            .map(|i| {
                let mut b = book(i, &format!("Title {i}"), "An Author");
                b.highlights = (0..3)
                    .map(|n| Highlight {
                        id: format!("h{i}-{n}"),
                        spine: n,
                        start: 5,
                        end: 20,
                        color: "yellow".into(),
                        style: "background".into(),
                        note: if n == 1 {
                            "a note".into()
                        } else {
                            String::new()
                        },
                        text: format!("quote {n} of book {i}"),
                        created: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
                    })
                    .collect();
                b.bookmarks = (0..2)
                    .map(|n| Bookmark {
                        id: format!("m{i}-{n}"),
                        spine: n,
                        frac: 0.25 * (n + 1) as f64,
                        label: format!("Page {n}"),
                        created: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
                    })
                    .collect();
                b
            })
            .collect();
        s.put_many(&books).unwrap();
    }
    let c = rusqlite::Connection::open(db).unwrap();
    c.execute_batch("DROP TABLE annotation_history; PRAGMA user_version = 9;")
        .unwrap();
}

#[test]
fn opening_a_v9_library_adds_an_empty_history_and_loses_nothing() {
    let d = dir();
    let db = d.join("lib.db");
    v9_library(&db);
    let before = {
        // read it as the old release would have: the tables only, no migration
        let c = rusqlite::Connection::open(&db).unwrap();
        let n: i64 = c
            .query_row("SELECT count(*) FROM books", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 12);
        assert!(
            c.query_row(
                "SELECT 1 FROM sqlite_master WHERE name = 'annotation_history'",
                [],
                |_| Ok(())
            )
            .is_err()
        );
        (12, 36, 24)
    };

    let s = SqliteStore::open(&db).unwrap();
    assert_eq!(s.schema_version(), SCHEMA_VERSION);
    assert_eq!(annotations(&s), before, "every book and every annotation");
    assert_eq!(s.history_count(), 0, "old data has no history");
    let c = copies(&d);
    assert_eq!(c.len(), 1, "{c:?}");
    assert!(c[0].to_string_lossy().contains(".pre-v9-"), "{c:?}");
    let old = rusqlite::Connection::open(&c[0]).unwrap();
    let v: i32 = old
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 9, "the copy is the state before the update");
    let n: i64 = old
        .query_row("SELECT count(*) FROM books", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 12);
    drop(s);

    // a second open does nothing: no new copy, same data, still no history
    let s = SqliteStore::open(&db).unwrap();
    assert_eq!(copies(&d), c);
    assert_eq!(annotations(&s), before);
    assert_eq!(s.history_count(), 0);

    // and the table works from here on: a deletion is journalled with its record
    let mut b = s.get(1).unwrap();
    let gone = b.highlights.remove(0);
    s.put(b).unwrap();
    let es = s
        .history(&bookx_core::annotation_journal::Filter::default(), 10)
        .unwrap();
    assert_eq!(es.len(), 1);
    assert_eq!(es[0].record.id(), gone.id);
    let _ = std::fs::remove_dir_all(d);
}
