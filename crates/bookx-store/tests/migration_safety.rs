//! A schema update never starts without a copy of the library next to it, keeps only the newest copies, checks the result,
//! and a database that is new, current or too new gets no copy.

mod common;

use std::path::{Path, PathBuf};

use bookx_store::{SCHEMA_VERSION, SqliteStore};
use common::book;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bookx-migsafe-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A database at `version`: a current one with its version number turned back (the later migrations only add tables
/// with IF NOT EXISTS, so opening it again runs them for real).
fn database_at(path: &Path, version: i32, books: u64) {
    {
        let s = SqliteStore::open(path).unwrap();
        let list: Vec<_> = (1..=books)
            .map(|i| book(i, &format!("Title {i}"), "An Author"))
            .collect();
        s.put_many(&list).unwrap();
    }
    let c = rusqlite::Connection::open(path).unwrap();
    c.execute_batch(&format!("PRAGMA user_version = {version}"))
        .unwrap();
}

fn copies(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().contains(".pre-v"))
        .collect();
    v.sort();
    v
}

fn version_of(path: &Path) -> i32 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn an_update_keeps_a_complete_copy_of_the_old_library() {
    let d = dir("copy");
    let db = d.join("lib.db");
    database_at(&db, SCHEMA_VERSION - 1, 25);
    let s = SqliteStore::open(&db).unwrap();
    assert_eq!(s.schema_version(), SCHEMA_VERSION);
    assert_eq!(s.len(), 25);
    let c = copies(&d);
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(
        version_of(&c[0]),
        SCHEMA_VERSION - 1,
        "the copy is the state before the update"
    );
    let n: i64 = rusqlite::Connection::open(&c[0])
        .unwrap()
        .query_row("SELECT count(*) FROM books", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 25, "every book is in the copy");
}

#[test]
fn only_the_newest_copies_are_kept() {
    let d = dir("prune");
    let db = d.join("lib.db");
    for _ in 0..4 {
        database_at(&db, SCHEMA_VERSION - 1, 3);
        SqliteStore::open(&db).unwrap();
        // the time in the name has a resolution of one second
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    assert_eq!(copies(&d).len(), 2);
}

#[test]
fn a_new_a_current_and_a_too_new_database_get_no_copy() {
    let d = dir("none");
    let db = d.join("lib.db");
    SqliteStore::open(&db).unwrap();
    SqliteStore::open(&db).unwrap();
    assert!(copies(&d).is_empty(), "new and current");

    let newer = d.join("newer.db");
    database_at(&newer, SCHEMA_VERSION + 5, 2);
    let err = SqliteStore::open(&newer)
        .err()
        .expect("a database from a newer version is refused");
    assert!(format!("{err:#}").contains("newer"), "{err:#}");
    assert!(
        copies(&d).is_empty(),
        "nothing is copied for a refused database"
    );
    assert_eq!(
        version_of(&newer),
        SCHEMA_VERSION + 5,
        "and it is left alone"
    );
}
